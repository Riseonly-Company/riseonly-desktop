use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, DispatchPhase, ElementId, Hitbox, HitboxBehavior, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    RenderOnce, Size, Styled, Window, canvas, div, point, px, size,
};

const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

const PREDICTION_SECONDS: f32 = 0.25;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DragGestureValue {
    pub start_location: Point<Pixels>,
    pub location: Point<Pixels>,
    pub translation: Size<Pixels>,
    pub predicted_end_translation: Size<Pixels>,
    pub velocity: Size<Pixels>,
}

impl DragGestureValue {
    pub fn predicted_end_location(&self) -> Point<Pixels> {
        point(
            self.start_location.x + self.predicted_end_translation.width,
            self.start_location.y + self.predicted_end_translation.height,
        )
    }
}

/// closures for drag changed and ended
pub type DragGestureHandler = Rc<dyn Fn(&DragGestureValue, &mut Window, &mut App)>;

/// drag gesture with min distance and changed and ended handlers
#[derive(Clone)]
pub struct DragGesture {
    pub minimum_distance: f32,
    on_changed: Option<DragGestureHandler>,
    on_ended: Option<DragGestureHandler>,
}

impl DragGesture {
    pub fn new(minimum_distance: f32) -> Self {
        Self {
            minimum_distance,
            on_changed: None,
            on_ended: None,
        }
    }

    pub fn on_changed(mut self, handler: impl Fn(&DragGestureValue, &mut Window, &mut App) + 'static) -> Self {
        self.on_changed = Some(Rc::new(handler));
        self
    }

    pub fn on_ended(mut self, handler: impl Fn(&DragGestureValue, &mut Window, &mut App) + 'static) -> Self {
        self.on_ended = Some(Rc::new(handler));
        self
    }

    fn changed(&self, value: &DragGestureValue, window: &mut Window, cx: &mut App) {
        if let Some(handler) = &self.on_changed {
            handler(value, window, cx);
        }
    }

    fn ended(&self, value: &DragGestureValue, window: &mut Window, cx: &mut App) {
        if let Some(handler) = &self.on_ended {
            handler(value, window, cx);
        }
    }
}

/// which gestures of the view and its subviews stay enabled
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GestureMask {
    All,
    Subviews,
    Gesture,
    None,
}

impl GestureMask {
    pub const fn includes_gesture(self) -> bool {
        matches!(self, Self::All | Self::Gesture)
    }
}

/// gesture and simultaneous gesture modifiers on any element
pub trait DragGestureModifiers: IntoElement + Sized {
    fn gesture(self, id: impl Into<ElementId>, gesture: DragGesture) -> DragGestureElement {
        DragGestureElement {
            id: id.into(),
            gesture: Some(gesture),
            content: self.into_any_element(),
            fills: false,
        }
    }

    /// attaches a drag gesture next to the others, a mask without the gesture leaves the view as is
    fn simultaneous_gesture(
        self,
        id: impl Into<ElementId>,
        gesture: DragGesture,
        including: GestureMask,
    ) -> DragGestureElement {
        DragGestureElement {
            id: id.into(),
            gesture: including.includes_gesture().then_some(gesture),
            content: self.into_any_element(),
            fills: false,
        }
    }
}

impl<E: IntoElement> DragGestureModifiers for E {}

/// element returned by gesture and simultaneous_gesture
#[derive(IntoElement)]
pub struct DragGestureElement {
    id: ElementId,
    gesture: Option<DragGesture>,
    content: AnyElement,
    fills: bool,
}

impl DragGestureElement {
    pub fn size_full(mut self) -> Self {
        self.fills = true;
        self
    }
}

impl RenderOnce for DragGestureElement {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(gesture) = self.gesture else {
            return self.content;
        };
        let tracking = window
            .use_keyed_state(self.id, cx, |_, _| DragGestureTrackingCell::default())
            .read(cx)
            .clone();
        let area = if self.fills { div().size_full() } else { div() };
        area
            .relative()
            .child(self.content)
            .child(drag_gesture_overlay(gesture, tracking))
            .into_any_element()
    }
}

/// one left button press from down to up
struct DragGestureTracking {
    origin: Point<Pixels>,
    start_location: Point<Pixels>,
    location: Point<Pixels>,
    is_recognized: bool,
    samples: Vec<(Instant, Point<Pixels>)>,
    pressed_in_active_window: bool,
}

impl DragGestureTracking {
    fn new(origin: Point<Pixels>, location: Point<Pixels>, now: Instant) -> Self {
        Self {
            origin,
            start_location: location,
            location,
            is_recognized: false,
            samples: vec![(now, location)],
            pressed_in_active_window: false,
        }
    }

    fn record(&mut self, location: Point<Pixels>, now: Instant) {
        self.location = location;
        self.samples.push((now, location));
        self.samples
            .retain(|(time, _)| now.saturating_duration_since(*time) <= VELOCITY_WINDOW);
    }

    /// recognized once the pointer moved minimum_distance from the press
    fn recognize(&mut self, minimum_distance: f32) -> bool {
        if !self.is_recognized {
            let dx = f32::from(self.location.x - self.start_location.x);
            let dy = f32::from(self.location.y - self.start_location.y);
            self.is_recognized = (dx * dx + dy * dy).sqrt() >= minimum_distance;
        }
        self.is_recognized
    }

    fn velocity(&self) -> Size<Pixels> {
        let (Some(first), Some(last)) = (self.samples.first(), self.samples.last()) else {
            return size(px(0.0), px(0.0));
        };
        let seconds = last.0.saturating_duration_since(first.0).as_secs_f32();
        if seconds <= f32::EPSILON {
            return size(px(0.0), px(0.0));
        }
        size(
            px(f32::from(last.1.x - first.1.x) / seconds),
            px(f32::from(last.1.y - first.1.y) / seconds),
        )
    }

    fn value(&self) -> DragGestureValue {
        let translation = size(
            self.location.x - self.start_location.x,
            self.location.y - self.start_location.y,
        );
        let velocity = self.velocity();
        DragGestureValue {
            start_location: self.start_location - self.origin,
            location: self.location - self.origin,
            translation,
            predicted_end_translation: size(
                translation.width + velocity.width * PREDICTION_SECONDS,
                translation.height + velocity.height * PREDICTION_SECONDS,
            ),
            velocity,
        }
    }
}

/// kept per element id across frames, so a drag survives the re-renders it causes
#[derive(Clone, Default)]
struct DragGestureTrackingCell(Rc<RefCell<Option<DragGestureTracking>>>);

fn drag_gesture_overlay(gesture: DragGesture, tracking: DragGestureTrackingCell) -> impl IntoElement {
    canvas(
        |bounds, window, _| (bounds, window.insert_hitbox(bounds, HitboxBehavior::Normal)),
        move |_, (bounds, hitbox): (Bounds<Pixels>, Hitbox), window, cx| {
            let lost_focus = tracking.0.borrow().as_ref().is_some_and(|held| held.pressed_in_active_window);
            if lost_focus && !window.is_window_active() {
                let lost = tracking.clone();
                let lost_gesture = gesture.clone();
                window.defer(cx, move |window, cx| {
                    let Some(ended) = lost.0.borrow_mut().take() else {
                        return;
                    };
                    if ended.is_recognized {
                        lost_gesture.ended(&ended.value(), window, cx);
                    }
                });
            }
            let down = tracking.clone();
            let down_gesture = gesture.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let now = cx.background_executor().now();
                let mut started = DragGestureTracking::new(bounds.origin, event.position, now);
                started.pressed_in_active_window = window.is_window_active();
                let value = started.recognize(down_gesture.minimum_distance).then(|| started.value());
                *down.0.borrow_mut() = Some(started);
                if let Some(value) = value {
                    down_gesture.changed(&value, window, cx);
                }
            });

            let moved = tracking.clone();
            let moved_gesture = gesture.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || moved.0.borrow().is_none() {
                    return;
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    let Some(ended) = moved.0.borrow_mut().take() else {
                        return;
                    };
                    if ended.is_recognized {
                        moved_gesture.ended(&ended.value(), window, cx);
                    }
                    return;
                }
                let now = cx.background_executor().now();
                let value = {
                    let mut tracking = moved.0.borrow_mut();
                    let Some(tracking) = tracking.as_mut() else {
                        return;
                    };
                    tracking.record(event.position, now);
                    tracking.recognize(moved_gesture.minimum_distance).then(|| tracking.value())
                };
                if let Some(value) = value {
                    moved_gesture.changed(&value, window, cx);
                }
            });

            let up = tracking;
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                    return;
                }
                let Some(mut ended) = up.0.borrow_mut().take() else {
                    return;
                };
                let now = cx.background_executor().now();
                ended.record(event.position, now);
                if ended.recognize(gesture.minimum_distance) {
                    gesture.ended(&ended.value(), window, cx);
                }
            });
        },
    )
    .absolute()
    .inset_0()
}
