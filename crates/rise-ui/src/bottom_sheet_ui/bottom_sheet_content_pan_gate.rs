use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, Canvas, DispatchPhase, Element, ElementId, Entity, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle, ScrollWheelEvent, TouchPhase,
    WeakEntity, Window, canvas, point, px,
};

use crate::bottom_sheet_ui::bottom_sheet_axis_ownership::{
    BottomSheetAxisOwnership, BottomSheetGestureRecognizer, BottomSheetGestureState,
};
use crate::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use crate::bottom_sheet_ui::bottom_sheet_motion_host::BottomSheetMotionHostingController;
use crate::swiftui::environment::{EnvKey, env};
use crate::swiftui::prelude::*;

const PAN_HYSTERESIS: f32 = 10.0;
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Undetermined,
    Horizontal,
    Vertical,
}

/// scroll areas laid out inside the sheet surface this frame
/// gpui cant hit test for the scroll view under a point, so nested lists declare themselves
/// with bottom_sheet_nested_scroll and the gate looks them up by bounds
#[derive(Clone, Default)]
pub struct BottomSheetNestedScrollRegistry(Rc<RefCell<Vec<ScrollHandle>>>);

impl BottomSheetNestedScrollRegistry {
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub fn register(&self, handle: ScrollHandle) {
        self.0.borrow_mut().push(handle);
    }

    pub fn handles(&self) -> Vec<ScrollHandle> {
        self.0.borrow().clone()
    }
}

/// registry of the sheet surface the subtree lies in, None outside a sheet
pub struct BottomSheetNestedScrollKey;

impl EnvKey for BottomSheetNestedScrollKey {
    type Value = Option<BottomSheetNestedScrollRegistry>;

    fn default_value() -> Self::Value {
        None
    }
}

/// marks child as the vertical scroll area driven by handle
/// so the gate knows if the list under the pointer is at its top, adds no layout node
pub fn bottom_sheet_nested_scroll(
    handle: &ScrollHandle,
    child: impl IntoElement,
) -> BottomSheetNestedScroll {
    BottomSheetNestedScroll {
        handle: handle.clone(),
        child: child.into_any_element(),
    }
}

pub struct BottomSheetNestedScroll {
    handle: ScrollHandle,
    child: AnyElement,
}

impl IntoElement for BottomSheetNestedScroll {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for BottomSheetNestedScroll {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(registry) = env::<BottomSheetNestedScrollKey>(cx) {
            registry.register(self.handle.clone());
        }
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

/// where a press or trackpad session is before it turns into a pan
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackingSource {
    Mouse,
    Trackpad,
}

struct Tracking {
    source: TrackingSource,
    origin: Point<Pixels>,
    location: Point<Pixels>,
    translation: Point<Pixels>,
    samples: Vec<(Instant, Point<Pixels>)>,
    recognized: bool,
    failed: bool,
}

impl Tracking {
    fn new(source: TrackingSource, location: Point<Pixels>, now: Instant) -> Self {
        Self {
            source,
            origin: location,
            location,
            translation: Point::default(),
            samples: vec![(now, Point::default())],
            recognized: false,
            failed: false,
        }
    }

    fn record(&mut self, now: Instant) {
        self.samples.push((now, self.translation));
        self.samples
            .retain(|(time, _)| now.saturating_duration_since(*time) <= VELOCITY_WINDOW);
    }

    /// velocity in points per second
    fn velocity(&self) -> Point<Pixels> {
        let (Some(first), Some(last)) = (self.samples.first(), self.samples.last()) else {
            return Point::default();
        };
        let seconds = last.0.saturating_duration_since(first.0).as_secs_f32();
        if seconds <= f32::EPSILON {
            return Point::default();
        }
        point(
            px(f32::from(last.1.x - first.1.x) / seconds),
            px(f32::from(last.1.y - first.1.y) / seconds),
        )
    }
}

pub struct BottomSheetContentPanGate {
    host: WeakEntity<BottomSheetMotionHostingController>,
    tracked_scroll: Option<ScrollHandle>,
    pan: BottomSheetGestureRecognizer,
    tracking: Option<Tracking>,

    axis: Axis,
    is_pull_active: bool,
    is_expand_active: bool,
    did_handoff_to_inner: bool,
    began_at_scroll_top: bool,
    activation_translation: Pixels,
    last_translation: Pixels,
    expansion_distance: Pixels,
}

impl BottomSheetContentPanGate {
    pub fn new(host: WeakEntity<BottomSheetMotionHostingController>) -> Self {
        Self {
            host,
            tracked_scroll: None,
            pan: BottomSheetGestureRecognizer::new(),
            tracking: None,
            axis: Axis::Undetermined,
            is_pull_active: false,
            is_expand_active: false,
            did_handoff_to_inner: false,
            began_at_scroll_top: true,
            activation_translation: px(0.0),
            last_translation: px(0.0),
            expansion_distance: px(0.0),
        }
    }

    fn is_driving(&self) -> bool {
        self.is_pull_active || self.is_expand_active
    }

    /// false while the horizontal axis is already driven (a page back swipe)
    /// the vertical owner must skip that gesture entirely
    fn can_drive_sheet(cx: &App) -> bool {
        !BottomSheetAxisOwnership::horizontal_is_driving(cx)
    }

    fn handle_pan(
        &mut self,
        state: BottomSheetGestureState,
        location: Point<Pixels>,
        translation_xy: Point<Pixels>,
        velocity_xy: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let translation = translation_xy.y;
        self.pan.set_state(state);

        match state {
            BottomSheetGestureState::Began => {
                self.reset_state(cx);
                self.pan.set_state(state);
                self.activation_translation = translation;
                self.tracked_scroll = Self::vertically_scrollable_view(location, &host, cx);
                self.began_at_scroll_top = Self::is_at_top(self.tracked_scroll.as_ref());

                let host_state = host.read(cx);
                let locked = host_state.content_pan_is_locked();
                let expansion_room = host_state.expansion_room();
                if !self.began_at_scroll_top
                    || locked
                    || !Self::can_drive_sheet(cx)
                    || velocity_xy.y.abs() <= velocity_xy.x.abs() * 0.9
                {
                    return;
                }

                self.axis = Axis::Vertical;
                if velocity_xy.y > px(0.0) {
                    self.begin_pull(translation, &host, window, cx);
                } else if expansion_room > px(0.5) {
                    self.begin_expand(translation, &host, window, cx);
                }
            }

            BottomSheetGestureState::Changed => {
                if host.read(cx).content_pan_is_locked() {
                    return;
                }

                if self.axis == Axis::Undetermined {
                    let dx = translation_xy.x.abs();
                    let dy = translation.abs();
                    if dx.max(dy) < px(5.0) {
                        return;
                    }
                    self.axis = if dx > dy * 1.08 { Axis::Horizontal } else { Axis::Vertical };
                    self.activation_translation = translation;
                }

                if self.axis != Axis::Vertical || !Self::can_drive_sheet(cx) {
                    return;
                }

                if !self.is_driving() {
                    if !self.began_at_scroll_top {
                        return;
                    }
                    let committed = translation - self.activation_translation;
                    if committed > px(BottomSheetMetrics::SHEET_PULL_COMMIT_THRESHOLD) {
                        self.begin_pull(translation, &host, window, cx);
                    } else if committed < -px(BottomSheetMetrics::SHEET_EXPAND_COMMIT_THRESHOLD)
                        && host.read(cx).expansion_room() > px(0.5)
                    {
                        self.begin_expand(translation, &host, window, cx);
                    } else {
                        return;
                    }
                }

                if self.did_handoff_to_inner {
                    return;
                }

                let adjusted = translation - self.activation_translation;

                if self.is_expand_active
                    && self.expansion_distance > px(0.5)
                    && adjusted <= -self.expansion_distance
                {
                    self.last_translation = -self.expansion_distance;
                    let target = self.last_translation;
                    host.update(cx, |host, cx| host.update_interactive_translation(target, cx));
                    self.did_handoff_to_inner = true;
                    return;
                }

                self.pin_inner_to_top();
                self.last_translation = if adjusted < px(0.0) {
                    adjusted.max(-self.expansion_distance)
                } else {
                    adjusted
                };
                let target = self.last_translation;
                host.update(cx, |host, cx| host.update_interactive_translation(target, cx));
            }

            BottomSheetGestureState::Ended
            | BottomSheetGestureState::Cancelled
            | BottomSheetGestureState::Failed => {
                let was_driving = self.is_driving();
                let velocity = velocity_xy.y;
                let locked = host.read(cx).content_pan_is_locked();
                if was_driving {
                    BottomSheetMotionHostingController::content_pan_did_end(
                        &host,
                        if locked { px(0.0) } else { self.last_translation },
                        if locked { px(0.0) } else { velocity },
                        window,
                        cx,
                    );
                }
                self.reset_state(cx);
            }

            BottomSheetGestureState::Possible => {}
        }
    }

    fn begin_pull(
        &mut self,
        translation: Pixels,
        host: &Entity<BottomSheetMotionHostingController>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.is_pull_active = true;
        self.begin_driving(translation, host, window, cx);
    }

    fn begin_expand(
        &mut self,
        translation: Pixels,
        host: &Entity<BottomSheetMotionHostingController>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.is_expand_active = true;
        self.begin_driving(translation, host, window, cx);
    }

    /// common tail of beginPull and beginExpand
    fn begin_driving(
        &mut self,
        translation: Pixels,
        host: &Entity<BottomSheetMotionHostingController>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.claim_vertical_axis(cx);
        self.activation_translation = translation;
        self.last_translation = px(0.0);
        BottomSheetMotionHostingController::content_pan_did_begin(host, window, cx);
        self.expansion_distance = host.read(cx).interaction_expansion_room();
        self.pin_inner_to_top();
    }

    fn claim_vertical_axis(&self, cx: &mut App) {
        BottomSheetAxisOwnership::claim_vertical(&self.pan, cx);
    }

    /// gpui lists dont bounce so only the offset is pinned
    fn pin_inner_to_top(&self) {
        let Some(scroll) = self.tracked_scroll.as_ref() else {
            return;
        };
        let offset = scroll.offset();
        if offset.y.abs() > px(0.5) {
            scroll.set_offset(point(offset.x, px(0.0)));
        }
    }

    fn reset_state(&mut self, cx: &mut App) {
        BottomSheetAxisOwnership::release_vertical(&self.pan, cx);
        self.pan.set_state(BottomSheetGestureState::Possible);
        self.axis = Axis::Undetermined;
        self.is_pull_active = false;
        self.is_expand_active = false;
        self.did_handoff_to_inner = false;
        self.began_at_scroll_top = true;
        self.activation_translation = px(0.0);
        self.last_translation = px(0.0);
        self.expansion_distance = px(0.0);
        self.tracked_scroll = None;
    }

    pub fn vertically_scrollable_view(
        location: Point<Pixels>,
        host: &Entity<BottomSheetMotionHostingController>,
        cx: &App,
    ) -> Option<ScrollHandle> {
        host.read(cx)
            .nested_scrolls()
            .handles()
            .into_iter()
            .rev()
            .find(|handle| handle.bounds().contains(&location) && Self::is_vertically_scrollable(handle))
    }

    fn is_vertically_scrollable(scroll: &ScrollHandle) -> bool {
        scroll.max_offset().y > px(0.5)
    }

    pub fn is_at_top(scroll: Option<&ScrollHandle>) -> bool {
        let Some(scroll) = scroll else {
            return true;
        };
        let slack = -scroll.offset().y;
        slack <= px(BottomSheetMetrics::SCROLL_TOP_SLACK)
    }

    fn gesture_recognizer_should_begin(
        &self,
        location: Point<Pixels>,
        velocity: Point<Pixels>,
        cx: &App,
    ) -> bool {
        let Some(host) = self.host.upgrade() else {
            return true;
        };
        let host_state = host.read(cx);
        if host_state.content_pan_is_locked() || !Self::can_drive_sheet(cx) {
            return false;
        }
        if velocity.y.abs() <= velocity.x.abs() {
            return false;
        }
        let scroll = Self::vertically_scrollable_view(location, &host, cx);
        if !Self::is_at_top(scroll.as_ref()) {
            return false;
        }
        !(velocity.y < px(0.0) && host_state.expansion_room() <= px(0.5))
    }

    fn gesture_recognizer(&self) -> bool {
        !self.is_driving() || self.did_handoff_to_inner
    }

    fn advance_tracking(&mut self, now: Instant, window: &mut Window, cx: &mut App) {
        let Some(tracking) = self.tracking.as_mut() else {
            return;
        };
        if tracking.failed {
            return;
        }
        tracking.record(now);
        let location = tracking.location;
        let translation = tracking.translation;
        let velocity = tracking.velocity();
        let source = tracking.source;
        if tracking.recognized {
            self.handle_pan(
                BottomSheetGestureState::Changed,
                location,
                translation,
                velocity,
                window,
                cx,
            );
            return;
        }
        if source == TrackingSource::Mouse
            && translation.x.abs().max(translation.y.abs()) < px(PAN_HYSTERESIS)
        {
            return;
        }
        if !self.gesture_recognizer_should_begin(location, velocity, cx) {
            if let Some(tracking) = self.tracking.as_mut() {
                tracking.failed = true;
            }
            return;
        }
        if let Some(tracking) = self.tracking.as_mut() {
            tracking.recognized = true;
        }
        self.handle_pan(BottomSheetGestureState::Began, location, translation, velocity, window, cx);
        self.handle_pan(BottomSheetGestureState::Changed, location, translation, velocity, window, cx);
    }

    fn end_tracking(&mut self, state: BottomSheetGestureState, window: &mut Window, cx: &mut App) {
        let Some(tracking) = self.tracking.take() else {
            return;
        };
        if !tracking.recognized {
            return;
        }
        self.handle_pan(
            state,
            tracking.location,
            tracking.translation,
            tracking.velocity(),
            window,
            cx,
        );
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, now: Instant) {
        if event.button != MouseButton::Left {
            return;
        }
        self.tracking = Some(Tracking::new(TrackingSource::Mouse, event.position, now));
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, now: Instant, window: &mut Window, cx: &mut App) {
        if !self.tracking.as_ref().is_some_and(|t| t.source == TrackingSource::Mouse) {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.end_tracking(BottomSheetGestureState::Cancelled, window, cx);
            return;
        }
        let Some(tracking) = self.tracking.as_mut() else {
            return;
        };
        tracking.location = event.position;
        tracking.translation = event.position - tracking.origin;
        self.advance_tracking(now, window, cx);
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut App) {
        if event.button != MouseButton::Left {
            return;
        }
        if self.tracking.as_ref().is_some_and(|t| t.source == TrackingSource::Mouse) {
            self.end_tracking(BottomSheetGestureState::Ended, window, cx);
        }
    }

    /// true when the event moved the sheet and must not also scroll the list under it
    fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        hovered: bool,
        now: Instant,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !event.delta.precise() {
            return false;
        }
        match event.touch_phase {
            TouchPhase::Started => {
                if hovered && self.tracking.is_none() {
                    self.tracking = Some(Tracking::new(TrackingSource::Trackpad, event.position, now));
                }
                false
            }
            TouchPhase::Moved => {
                let Some(tracking) = self.tracking.as_mut() else {
                    return false;
                };
                if tracking.source != TrackingSource::Trackpad {
                    return false;
                }
                tracking.translation += event.delta.pixel_delta(window.line_height());
                tracking.location = event.position;
                self.advance_tracking(now, window, cx);
                !self.gesture_recognizer()
            }
            TouchPhase::Ended => {
                let consumed = self.is_driving();
                if self.tracking.as_ref().is_some_and(|t| t.source == TrackingSource::Trackpad) {
                    self.end_tracking(BottomSheetGestureState::Ended, window, cx);
                }
                consumed
            }
            TouchPhase::Cancelled => {
                if self.tracking.as_ref().is_some_and(|t| t.source == TrackingSource::Trackpad) {
                    self.end_tracking(BottomSheetGestureState::Cancelled, window, cx);
                }
                false
            }
        }
    }

    pub fn recognizer_overlay(gate: &Rc<RefCell<Self>>) -> Canvas<Hitbox> {
        let gate = gate.clone();
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox: Hitbox, window, _| {
                let down_gate = gate.clone();
                let down_hitbox = hitbox.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && down_hitbox.is_hovered(window) {
                        let now = cx.background_executor().now();
                        down_gate.borrow_mut().on_mouse_down(event, now);
                    }
                });
                let move_gate = gate.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let now = cx.background_executor().now();
                        move_gate.borrow_mut().on_mouse_move(event, now, window, cx);
                    }
                });
                let up_gate = gate.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        up_gate.borrow_mut().on_mouse_up(event, window, cx);
                    }
                });
                let wheel_gate = gate;
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let now = cx.background_executor().now();
                    let hovered = hitbox.is_hovered(window);
                    let consumed =
                        wheel_gate.borrow_mut().on_scroll_wheel(event, hovered, now, window, cx);
                    if consumed {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    }
}
