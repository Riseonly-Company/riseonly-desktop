use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, DispatchPhase, Entity, Hitbox, HitboxBehavior, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, RenderOnce, ScrollDelta, ScrollWheelEvent, Styled, TouchPhase, Window, canvas, point,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::guide_anchor_ui::guide_scroll_host::{GuideScrollHost, WeakGuideScrollHost};
use rise_ui::swiftui::environment::{EnvKey, env};

use crate::modules::chat::pages::chats::embedded_forum_transition::{
    EmbeddedForumSwipeAxisLock, EmbeddedForumTransitionMetrics,
};

#[derive(Clone)]
pub struct EmbeddedForumHorizontalPanCallbacks {
    pub on_changed: Rc<dyn Fn(f32, &mut Window, &mut App)>,
    pub on_ended: Rc<dyn Fn(f32, f32, &mut Window, &mut App)>,
    pub on_cancelled: Rc<dyn Fn(&mut Window, &mut App)>,
}

pub struct EmbeddedForumHorizontalPanCallbacksKey;

impl EnvKey for EmbeddedForumHorizontalPanCallbacksKey {
    type Value = Option<EmbeddedForumHorizontalPanCallbacks>;

    fn default_value() -> Self::Value {
        None
    }
}

pub enum EmbeddedForumHorizontalPanIntentPolicy {}

impl EmbeddedForumHorizontalPanIntentPolicy {
    pub const PREDICTION_DURATION: f64 = 0.18;

    pub fn accepts(translation: Point<f32>, velocity: Point<f32>) -> bool {
        let translation_has_intent = translation.x.abs().max(translation.y.abs()) >= 1.0;
        let vector = if translation_has_intent { translation } else { velocity };
        EmbeddedForumSwipeAxisLock::accepts_rightward_horizontal_intent(
            vector.x,
            vector.y,
            EmbeddedForumSwipeAxisLock::DEFAULT_DOMINANCE_RATIO,
        )
    }

    pub fn activation_offset(translation: f32) -> f32 {
        translation.clamp(0.0, EmbeddedForumTransitionMetrics::SWIPE_MINIMUM_DISTANCE)
    }

    pub fn compensated_translation(translation: f32, activation_offset: f32) -> f32 {
        (translation - activation_offset).max(0.0)
    }

    pub fn predicted_translation(translation: f32, velocity: f32, activation_offset: f32) -> f32 {
        Self::compensated_translation(translation + velocity * Self::PREDICTION_DURATION as f32, activation_offset)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanSource {
    Wheel,
    Mouse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanState {
    Idle,
    Possible,
    Failed,
    Tracking,
}

/// pan recognizer for the embedded forum panel, fed by trackpad wheel and mouse drag
pub struct EmbeddedForumHorizontalPanRecognizer {
    is_enabled: bool,
    cancels_touches_in_view: bool,
    view: Option<WeakGuideScrollHost>,
    state: PanState,
    source: Option<PanSource>,
    origin: Point<Pixels>,
    translation: Point<f32>,
    velocity: Point<f32>,
    last_sample: Duration,
}

impl EmbeddedForumHorizontalPanRecognizer {
    const VELOCITY_REST_INTERVAL: Duration = Duration::from_millis(100);

    fn new() -> Self {
        Self {
            is_enabled: false,
            cancels_touches_in_view: true,
            view: None,
            state: PanState::Idle,
            source: None,
            origin: point(Pixels::ZERO, Pixels::ZERO),
            translation: point(0.0, 0.0),
            velocity: point(0.0, 0.0),
            last_sample: Duration::ZERO,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    pub fn cancels_touches_in_view(&self) -> bool {
        self.cancels_touches_in_view
    }

    pub fn view(&self) -> Option<GuideScrollHost> {
        self.view.as_ref().and_then(WeakGuideScrollHost::upgrade)
    }

    fn reset(&mut self) {
        self.state = PanState::Idle;
        self.source = None;
        self.translation = point(0.0, 0.0);
        self.velocity = point(0.0, 0.0);
    }

    fn released_velocity_x(&self, now: Duration) -> f32 {
        if now.saturating_sub(self.last_sample) >= Self::VELOCITY_REST_INTERVAL { 0.0 } else { self.velocity.x }
    }
}

pub enum PanDelivery {
    None,
    Changed(f32),
    Ended(f32, f32),
    Cancelled,
}

/// what one input event did, the callback to run and whether it stops before the rows below
pub(super) struct PanOutcome {
    callbacks: Option<EmbeddedForumHorizontalPanCallbacks>,
    pub(super) delivery: PanDelivery,
    pub(super) consumed: bool,
}

impl PanOutcome {
    fn none() -> Self {
        Self { callbacks: None, delivery: PanDelivery::None, consumed: false }
    }

    fn dispatch(self, window: &mut Window, cx: &mut App) {
        let Some(callbacks) = self.callbacks else { return };
        match self.delivery {
            PanDelivery::None => {}
            PanDelivery::Changed(offset) => (callbacks.on_changed)(offset, window, cx),
            PanDelivery::Ended(translation, predicted) => (callbacks.on_ended)(translation, predicted, window, cx),
            PanDelivery::Cancelled => (callbacks.on_cancelled)(window, cx),
        }
    }
}

pub struct EmbeddedForumHorizontalScrollGateCoordinator {
    scroll_view: Option<WeakGuideScrollHost>,
    callbacks: Option<EmbeddedForumHorizontalPanCallbacks>,
    is_enabled: bool,
    activation_offset: Option<f32>,
    horizontal_pan: EmbeddedForumHorizontalPanRecognizer,
}

impl EmbeddedForumHorizontalScrollGateCoordinator {
    pub fn new(callbacks: Option<EmbeddedForumHorizontalPanCallbacks>, is_enabled: bool) -> Self {
        Self {
            scroll_view: None,
            callbacks,
            is_enabled,
            activation_offset: None,
            horizontal_pan: EmbeddedForumHorizontalPanRecognizer::new(),
        }
    }

    pub fn horizontal_pan(&self) -> &EmbeddedForumHorizontalPanRecognizer {
        &self.horizontal_pan
    }

    /// returns the callbacks whose on_cancelled the caller must run once when a live pan got cut
    pub fn update(
        &mut self,
        callbacks: Option<EmbeddedForumHorizontalPanCallbacks>,
        is_enabled: bool,
    ) -> Option<EmbeddedForumHorizontalPanCallbacks> {
        self.callbacks = callbacks;
        if self.is_enabled == is_enabled {
            return None;
        }

        self.is_enabled = is_enabled;
        self.activation_offset = None;

        if is_enabled {
            self.horizontal_pan.is_enabled = self.scroll_view().is_some();
            None
        } else {
            let cancelled = self.cancel_recognizer_if_active();
            self.horizontal_pan.is_enabled = false;
            cancelled
        }
    }

    pub fn attach(&mut self, scroll_view: &GuideScrollHost) -> Option<EmbeddedForumHorizontalPanCallbacks> {
        if self.scroll_view().is_some_and(|current| current.same_host(scroll_view)) {
            return None;
        }
        let cancelled = self.detach();
        self.activation_offset = None;
        self.scroll_view = Some(scroll_view.downgrade());
        self.horizontal_pan.view = Some(scroll_view.downgrade());
        self.horizontal_pan.is_enabled = self.is_enabled;
        cancelled
    }

    pub fn detach(&mut self) -> Option<EmbeddedForumHorizontalPanCallbacks> {
        let cancelled = self.cancel_recognizer_if_active();
        self.activation_offset = None;
        self.horizontal_pan.is_enabled = false;
        self.horizontal_pan.view = None;
        self.scroll_view = None;
        cancelled
    }

    fn scroll_view(&self) -> Option<GuideScrollHost> {
        self.scroll_view.as_ref().and_then(WeakGuideScrollHost::upgrade)
    }

    fn gesture_recognizer_should_begin(&self) -> bool {
        if !self.is_enabled || self.callbacks.is_none() {
            return false;
        }
        EmbeddedForumHorizontalPanIntentPolicy::accepts(self.horizontal_pan.translation, self.horizontal_pan.velocity)
    }

    pub fn touches_began(&mut self, location: Point<Pixels>, source: PanSource) {
        if !self.horizontal_pan.is_enabled {
            return;
        }
        self.horizontal_pan.reset();
        self.horizontal_pan.state = PanState::Possible;
        self.horizontal_pan.source = Some(source);
        self.horizontal_pan.origin = location;
        self.horizontal_pan.last_sample = RiseClock::monotonic();
    }

    pub fn mouse_moved(&mut self, location: Point<Pixels>) -> PanOutcome {
        if self.horizontal_pan.source != Some(PanSource::Mouse) {
            return PanOutcome::none();
        }
        let origin = self.horizontal_pan.origin;
        self.touches_moved(point(f32::from(location.x - origin.x), f32::from(location.y - origin.y)))
    }

    fn touches_moved(&mut self, translation: Point<f32>) -> PanOutcome {
        let now = RiseClock::monotonic();
        let pan = &mut self.horizontal_pan;
        let elapsed = now.saturating_sub(pan.last_sample).as_secs_f32();
        if elapsed > 0.0 {
            pan.velocity = point((translation.x - pan.translation.x) / elapsed, (translation.y - pan.translation.y) / elapsed);
            pan.last_sample = now;
        }
        pan.translation = translation;

        match pan.state {
            PanState::Idle | PanState::Failed => PanOutcome::none(),
            PanState::Possible => {
                if translation.x.abs().max(translation.y.abs()) < EmbeddedForumTransitionMetrics::SWIPE_MINIMUM_DISTANCE {
                    return PanOutcome::none();
                }
                if !self.gesture_recognizer_should_begin() {
                    self.horizontal_pan.state = PanState::Failed;
                    return PanOutcome::none();
                }
                self.horizontal_pan.state = PanState::Tracking;
                self.activation_offset = Some(EmbeddedForumHorizontalPanIntentPolicy::activation_offset(translation.x));
                self.deliver_changed(translation.x)
            }
            PanState::Tracking => self.deliver_changed(translation.x),
        }
    }

    pub fn touches_ended(&mut self, source: PanSource) -> PanOutcome {
        if self.horizontal_pan.source != Some(source) {
            return PanOutcome::none();
        }
        let was_tracking = self.horizontal_pan.state == PanState::Tracking;
        let translation = self.horizontal_pan.translation.x;
        let velocity = self.horizontal_pan.released_velocity_x(RiseClock::monotonic());
        self.horizontal_pan.reset();
        if !was_tracking {
            return PanOutcome::none();
        }
        let origin = self
            .activation_offset
            .take()
            .unwrap_or_else(|| EmbeddedForumHorizontalPanIntentPolicy::activation_offset(translation));
        let adjusted = EmbeddedForumHorizontalPanIntentPolicy::compensated_translation(translation, origin);
        let predicted = EmbeddedForumHorizontalPanIntentPolicy::predicted_translation(translation, velocity, origin);
        PanOutcome { callbacks: self.callbacks.clone(), delivery: PanDelivery::Ended(adjusted, predicted), consumed: true }
    }

    fn deliver_changed(&mut self, translation: f32) -> PanOutcome {
        let origin = *self
            .activation_offset
            .get_or_insert_with(|| EmbeddedForumHorizontalPanIntentPolicy::activation_offset(translation));
        PanOutcome {
            callbacks: self.callbacks.clone(),
            delivery: PanDelivery::Changed(EmbeddedForumHorizontalPanIntentPolicy::compensated_translation(
                translation,
                origin,
            )),
            consumed: true,
        }
    }

    fn cancel_recognizer_if_active(&mut self) -> Option<EmbeddedForumHorizontalPanCallbacks> {
        let was_active = self.horizontal_pan.state == PanState::Tracking;
        self.activation_offset = None;
        self.horizontal_pan.reset();
        if was_active { self.callbacks.clone() } else { None }
    }

    pub fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, hovered: bool) -> PanOutcome {
        let ScrollDelta::Pixels(delta) = event.delta else {
            return PanOutcome::none();
        };
        match event.touch_phase {
            TouchPhase::Started => {
                if hovered {
                    self.touches_began(event.position, PanSource::Wheel);
                }
                PanOutcome::none()
            }
            TouchPhase::Moved => {
                if self.horizontal_pan.source != Some(PanSource::Wheel) {
                    return PanOutcome::none();
                }
                let translation = point(
                    self.horizontal_pan.translation.x + f32::from(delta.x),
                    self.horizontal_pan.translation.y + f32::from(delta.y),
                );
                let mut outcome = self.touches_moved(translation);
                if self.horizontal_pan.state == PanState::Possible
                    && EmbeddedForumHorizontalPanIntentPolicy::accepts(translation, self.horizontal_pan.velocity)
                {
                    outcome.consumed = true;
                }
                outcome
            }
            TouchPhase::Ended => {
                let mut outcome = self.touches_ended(PanSource::Wheel);
                outcome.consumed = false;
                outcome
            }
            TouchPhase::Cancelled => {
                if self.horizontal_pan.source != Some(PanSource::Wheel) {
                    return PanOutcome::none();
                }
                let callbacks = self.cancel_recognizer_if_active();
                let delivery = if callbacks.is_some() { PanDelivery::Cancelled } else { PanDelivery::None };
                PanOutcome { callbacks, delivery, consumed: false }
            }
        }
    }
}

/// no dismantle here, the coordinator lives in element state and goes with the panel
#[derive(IntoElement)]
pub struct EmbeddedForumHorizontalScrollGate {
    pub is_enabled: bool,
}

impl EmbeddedForumHorizontalScrollGate {
    pub fn new(is_enabled: bool) -> Self {
        Self { is_enabled }
    }

    fn deliver_cancelled(cancelled: Option<EmbeddedForumHorizontalPanCallbacks>, window: &mut Window, cx: &mut App) {
        if let Some(cancelled) = cancelled {
            window.defer(cx, move |window, cx| (cancelled.on_cancelled)(window, cx));
        }
    }
}

impl RenderOnce for EmbeddedForumHorizontalScrollGate {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let callbacks = env::<EmbeddedForumHorizontalPanCallbacksKey>(cx);
        let is_enabled = self.is_enabled;
        let initial = callbacks.clone();
        let coordinator: Entity<EmbeddedForumHorizontalScrollGateCoordinator> =
            window.use_keyed_state("embedded-forum-horizontal-scroll-gate", cx, move |_, _| {
                EmbeddedForumHorizontalScrollGateCoordinator::new(initial, is_enabled)
            });
        let cancelled = coordinator.update(cx, |coordinator, _| coordinator.update(callbacks, is_enabled));
        Self::deliver_cancelled(cancelled, window, cx);

        let probe = coordinator.clone();
        canvas(
            move |bounds, window, cx| {
                let scroll_view = GuideScrollHost::current();
                if let Some(scroll_view) = &scroll_view {
                    let cancelled = probe.update(cx, |coordinator, _| coordinator.attach(scroll_view));
                    Self::deliver_cancelled(cancelled, window, cx);
                }
                let frame = scroll_view.and_then(|scroll_view| scroll_view.frame()).unwrap_or(bounds);
                window.insert_hitbox(frame, HitboxBehavior::Normal)
            },
            move |_, hitbox: Hitbox, window, _| {
                let wheel = coordinator.clone();
                let wheel_hitbox = hitbox.clone();
                let down = coordinator.clone();
                let moved = coordinator.clone();
                let up = coordinator;

                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let hovered = wheel_hitbox.should_handle_scroll(window);
                    let outcome = wheel.update(cx, |coordinator, _| coordinator.handle_scroll_wheel(event, hovered));
                    if outcome.consumed {
                        cx.stop_propagation();
                    }
                    outcome.dispatch(window, cx);
                });
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture
                        || event.button != MouseButton::Left
                        || !hitbox.is_hovered(window)
                    {
                        return;
                    }
                    down.update(cx, |coordinator, _| coordinator.touches_began(event.position, PanSource::Mouse));
                });
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || event.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    let outcome = moved.update(cx, |coordinator, _| coordinator.mouse_moved(event.position));
                    if outcome.consumed {
                        cx.stop_propagation();
                    }
                    outcome.dispatch(window, cx);
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                        return;
                    }
                    let outcome = up.update(cx, |coordinator, _| coordinator.touches_ended(PanSource::Mouse));
                    if outcome.consumed {
                        cx.stop_propagation();
                    }
                    outcome.dispatch(window, cx);
                });
            },
        )
        .absolute()
        .inset_0()
    }
}
