use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, DispatchPhase, ElementId, IntoElement, Pixels, Point, RenderOnce, ScrollHandle,
    ScrollWheelEvent,
    TouchPhase, Window, canvas, point, px,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_axis_ownership::{
    BottomSheetAxisOwnership, BottomSheetGestureRecognizer, BottomSheetGestureState,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_content_pan_gate::{
    BottomSheetNestedScrollKey, BottomSheetNestedScrollRegistry,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::animated_tabs_ui::animated_tabs_pager_gesture_gate::AnimatedTabsPagerPanIntentPolicy;

const PAN_HYSTERESIS: f32 = 10.0;
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BottomSheetPageBackSwipeConfiguration {
    pub is_enabled: bool,
}

pub type BottomSheetPageBackSwipeAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type BottomSheetPageBackSwipeChanged = Rc<dyn Fn(Pixels, &mut Window, &mut App)>;
pub type BottomSheetPageBackSwipeEnded = Rc<dyn Fn(Pixels, Pixels, &mut Window, &mut App)>;

#[derive(Clone)]
pub struct BottomSheetPageBackSwipeCallbacks {
    pub on_began: BottomSheetPageBackSwipeAction,
    pub on_changed: BottomSheetPageBackSwipeChanged,
    pub on_ended: BottomSheetPageBackSwipeEnded,
    pub on_cancelled: BottomSheetPageBackSwipeAction,
}

/// release decision for the back swipe, shared by every page inside the sheet
/// threshold lives here and not in each page owner
/// two copies of the fractions would close one screen at a quarter width and the next at a third
pub enum BottomSheetPageBackSwipePolicy {}

impl BottomSheetPageBackSwipePolicy {
    pub fn commits(translation: Pixels, velocity: Pixels, width: Pixels) -> bool {
        if width <= px(1.0) {
            return false;
        }

        let projected =
            translation + velocity * BottomSheetMetrics::PAGE_FLICK_PROJECTION_SECONDS as f32;
        translation > width * BottomSheetMetrics::PAGE_COMMIT_DISTANCE_FRACTION
            || projected > width * BottomSheetMetrics::PAGE_COMMIT_PROJECTION_FRACTION
    }
}

/// the single recognizer that owns the horizontal axis inside the sheet
/// doesnt touch the vertical axis, should_begin needs horizontal dominance
/// and vertical owners need vertical dominance so the planes never collide
#[derive(IntoElement)]
pub struct BottomSheetPageBackSwipeGate {
    pub id: ElementId,
    pub configuration: BottomSheetPageBackSwipeConfiguration,
    pub callbacks: BottomSheetPageBackSwipeCallbacks,
}

impl BottomSheetPageBackSwipeGate {
    pub fn new(
        id: impl Into<ElementId>,
        configuration: BottomSheetPageBackSwipeConfiguration,
        callbacks: BottomSheetPageBackSwipeCallbacks,
    ) -> Self {
        Self { id: id.into(), configuration, callbacks }
    }

    pub fn make_coordinator(&self) -> Coordinator {
        Coordinator::new(self.configuration, self.callbacks.clone())
    }
}

impl RenderOnce for BottomSheetPageBackSwipeGate {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let initial = self.make_coordinator();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| Rc::new(RefCell::new(initial)));
        let coordinator = state.read(cx).clone();
        let cancelled = coordinator.borrow_mut().update(self.configuration, self.callbacks);
        if let Some(cancelled) = cancelled {
            window.defer(cx, move |window, cx| (cancelled.on_cancelled)(window, cx));
        }
        coordinator.borrow_mut().registry = env::<BottomSheetNestedScrollKey>(cx);

        let probe = coordinator.clone();
        canvas(
            move |bounds, _, _| {
                probe.borrow_mut().probe_bounds = Some(bounds);
            },
            move |_, _, window, _| Coordinator::attach(&coordinator, window),
        )
        .absolute()
        .inset_0()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackingSource {
    Trackpad,
}

struct Tracking {
    source: TrackingSource,
    origin: Point<Pixels>,
    translation: Point<Pixels>,
    samples: Vec<(Instant, Point<Pixels>)>,
}

impl Tracking {
    fn new(source: TrackingSource, location: Point<Pixels>, now: Instant) -> Self {
        Self {
            source,
            origin: location,
            translation: Point::default(),
            samples: vec![(now, Point::default())],
        }
    }

    fn record(&mut self, now: Instant) {
        self.samples.push((now, self.translation));
        self.samples
            .retain(|(time, _)| now.saturating_duration_since(*time) <= VELOCITY_WINDOW);
    }

    /// points per second
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

/// what one input event made the recognizer report, run after the coordinator is released
enum PanEvent {
    Began,
    Changed(Pixels),
    Ended(Pixels, Pixels),
    Cancelled,
}

pub struct Coordinator {
    configuration: BottomSheetPageBackSwipeConfiguration,
    callbacks: BottomSheetPageBackSwipeCallbacks,
    activation_offset: Pixels,
    is_tracking: bool,
    suppress_cancellation_callback: bool,
    pan: BottomSheetGestureRecognizer,
    tracking: Option<Tracking>,
    probe_bounds: Option<Bounds<Pixels>>,
    registry: Option<BottomSheetNestedScrollRegistry>,
}

impl Coordinator {
    const HORIZONTAL_DOMINANCE: f32 = 1.08;

    pub fn new(
        configuration: BottomSheetPageBackSwipeConfiguration,
        callbacks: BottomSheetPageBackSwipeCallbacks,
    ) -> Self {
        Self {
            configuration,
            callbacks,
            activation_offset: px(0.0),
            is_tracking: false,
            suppress_cancellation_callback: false,
            pan: BottomSheetGestureRecognizer::new(),
            tracking: None,
            probe_bounds: None,
            registry: None,
        }
    }

    /// returns the callbacks of a cancellation the update caused, for the caller to run
    pub fn update(
        &mut self,
        configuration: BottomSheetPageBackSwipeConfiguration,
        callbacks: BottomSheetPageBackSwipeCallbacks,
    ) -> Option<BottomSheetPageBackSwipeCallbacks> {
        self.callbacks = callbacks;
        let was_enabled = self.configuration.is_enabled;
        self.configuration = configuration;

        if configuration.is_enabled == was_enabled || configuration.is_enabled {
            return None;
        }
        self.cancel_recognizer_if_active()
    }

    fn attach(this: &Rc<RefCell<Self>>, window: &mut Window) {
        let wheel = Rc::downgrade(this);
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || !event.delta.precise() {
                return;
            }
            let now = cx.background_executor().now();
            let outcome = Self::with(&wheel, |coordinator| match event.touch_phase {
                TouchPhase::Started => {
                    coordinator.touches_began(TrackingSource::Trackpad, event.position, now);
                    None
                }
                TouchPhase::Moved => {
                    let tracking = coordinator.tracking.as_ref()?;
                    if tracking.source != TrackingSource::Trackpad {
                        return None;
                    }
                    let translation =
                        tracking.translation + event.delta.pixel_delta(window.line_height());
                    Some(coordinator.touches_moved(translation, now, cx))
                }
                TouchPhase::Ended => {
                    coordinator.tracks(TrackingSource::Trackpad).then(|| coordinator.touches_ended(now))
                }
                TouchPhase::Cancelled => {
                    coordinator.tracks(TrackingSource::Trackpad).then(|| coordinator.touches_cancelled())
                }
            });
            Self::dispatch(&wheel, outcome.flatten(), window, cx);
        });
    }

    fn with<R>(this: &Weak<RefCell<Self>>, body: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let this = this.upgrade()?;
        let mut coordinator = this.try_borrow_mut().ok()?;
        Some(body(&mut coordinator))
    }

    /// runs the callbacks outside the coordinator borrow
    /// a consumed event is kept from the page so the release cant reach the row under the pointer
    /// and a vertical list wont scroll by the sideways swipe
    fn dispatch(
        this: &Weak<RefCell<Self>>,
        outcome: Option<(Vec<PanEvent>, bool)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some((events, consumed)) = outcome else {
            return;
        };
        if consumed {
            cx.stop_propagation();
        }
        if events.is_empty() {
            return;
        }
        let Some(callbacks) = Self::with(this, |coordinator| coordinator.callbacks.clone()) else {
            return;
        };
        for event in events {
            match event {
                PanEvent::Began => (callbacks.on_began)(window, cx),
                PanEvent::Changed(translation) => (callbacks.on_changed)(translation, window, cx),
                PanEvent::Ended(translation, velocity) => {
                    (callbacks.on_ended)(translation, velocity, window, cx)
                }
                PanEvent::Cancelled => (callbacks.on_cancelled)(window, cx),
            }
        }
    }

    fn tracks(&self, source: TrackingSource) -> bool {
        self.tracking.as_ref().is_some_and(|tracking| tracking.source == source)
    }

    fn touches_began(&mut self, source: TrackingSource, location: Point<Pixels>, now: Instant) {
        if self.is_tracking || self.tracking.is_some() {
            return;
        }
        let Some(probe) = self.probe_bounds else {
            return;
        };
        if probe.size.width <= px(1.0) || probe.size.height <= px(1.0) || !probe.contains(&location) {
            return;
        }
        self.tracking = Some(Tracking::new(source, location, now));
    }

    fn touches_moved(&mut self, translation: Point<Pixels>, now: Instant, cx: &mut App) -> (Vec<PanEvent>, bool) {
        let Some(tracking) = self.tracking.as_mut() else {
            return (Vec::new(), false);
        };
        tracking.translation = translation;
        tracking.record(now);
        let source = tracking.source;
        let origin = tracking.origin;
        let velocity = tracking.velocity();

        if self.is_tracking {
            self.pan.set_state(BottomSheetGestureState::Changed);
            let changed = (translation.x - self.activation_offset).max(px(0.0));
            return (vec![PanEvent::Changed(changed)], true);
        }

        let distance = f32::from(translation.x).hypot(f32::from(translation.y));
        if distance < PAN_HYSTERESIS {
            let undecided_back = source == TrackingSource::Trackpad
                && translation.x > px(0.0)
                && translation.x > translation.y.abs() * Self::HORIZONTAL_DOMINANCE;
            return (Vec::new(), undecided_back);
        }

        if !self.gesture_recognizer_should_begin(origin, translation, velocity, cx) {
            self.tracking = None;
            return (Vec::new(), false);
        }

        self.is_tracking = true;
        self.pan.set_state(BottomSheetGestureState::Began);
        BottomSheetAxisOwnership::claim_horizontal(&self.pan, cx);
        self.activation_offset = translation.x;
        (vec![PanEvent::Began, PanEvent::Changed(px(0.0))], true)
    }

    fn touches_ended(&mut self, now: Instant) -> (Vec<PanEvent>, bool) {
        let Some(mut tracking) = self.tracking.take() else {
            return (Vec::new(), false);
        };
        if !self.is_tracking {
            return (Vec::new(), false);
        }
        tracking.record(now);
        let translation = (tracking.translation.x - self.activation_offset).max(px(0.0));
        let velocity = tracking.velocity().x;
        self.pan.set_state(BottomSheetGestureState::Ended);
        self.finish_tracking();
        (vec![PanEvent::Ended(translation, velocity)], true)
    }

    fn touches_cancelled(&mut self) -> (Vec<PanEvent>, bool) {
        self.tracking = None;
        if !self.is_tracking {
            return (Vec::new(), false);
        }
        self.pan.set_state(BottomSheetGestureState::Cancelled);
        self.finish_tracking();
        if self.suppress_cancellation_callback {
            return (Vec::new(), true);
        }
        (vec![PanEvent::Cancelled], true)
    }

    /// releases the horizontal axis, the arbiter reads liveness from the recognizer state
    /// and a state past Changed no longer counts as driving so the axis is free
    fn finish_tracking(&mut self) {
        self.is_tracking = false;
        self.activation_offset = px(0.0);
    }

    fn gesture_recognizer_should_begin(
        &self,
        start: Point<Pixels>,
        translation: Point<Pixels>,
        velocity: Point<Pixels>,
        cx: &mut App,
    ) -> bool {
        if !self.configuration.is_enabled {
            return false;
        }
        let Some(probe) = self.probe_bounds else {
            return false;
        };

        if translation.x <= px(0.0) || translation.x <= translation.y.abs() * Self::HORIZONTAL_DOMINANCE {
            return false;
        }

        if velocity.x.abs() < velocity.y.abs() {
            return false;
        }

        if BottomSheetAxisOwnership::vertical_is_driving(cx) {
            return false;
        }

        if probe.size.width <= px(1.0) || probe.size.height <= px(1.0) || !probe.contains(&start) {
            return false;
        }

        !self.starts_on_own_axis_owner(start)
    }

    /// a horizontal scroller under the start point owns the sideways gesture
    /// controls that own a drag already stopped the press before it got here
    fn starts_on_own_axis_owner(&self, point: Point<Pixels>) -> bool {
        let Some(registry) = self.registry.as_ref() else {
            return false;
        };
        registry.handles().iter().any(|scroll_view: &ScrollHandle| {
            scroll_view.bounds().contains(&point)
                && AnimatedTabsPagerPanIntentPolicy::scrolls_horizontally(scroll_view)
        })
    }

    /// toggling the recognizer off is the sure way to cancel one that already won
    /// its own callback is suppressed so the deck sees exactly one cancellation
    /// the returned callbacks deliver it
    fn cancel_recognizer_if_active(&mut self) -> Option<BottomSheetPageBackSwipeCallbacks> {
        if !self.is_tracking {
            return None;
        }
        self.suppress_cancellation_callback = true;
        let _ = self.touches_cancelled();
        self.suppress_cancellation_callback = false;
        Some(self.callbacks.clone())
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.pan.set_state(BottomSheetGestureState::Cancelled);
    }
}
