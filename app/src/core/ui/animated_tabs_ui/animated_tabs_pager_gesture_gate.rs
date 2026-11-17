use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, DispatchPhase, Div, Element, ElementId, Entity, GlobalElementId, Hitbox,
    HitboxBehavior, InspectorElementId, InteractiveElement, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, ScrollDelta, ScrollHandle, ScrollWheelEvent,
    Stateful, Styled, TouchPhase, Window, canvas, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::animated_tabs_ui::animated_tabs_types::AnimatedTabsMetrics;
use rise_ui::swiftui::environment::{EnvKey, env};

use crate::core::ui::pager_back_swipe_yield::{LeadingPageReporting, PagerBackSwipeYield};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimatedTabsPagerPanConfiguration {
    pub is_locked: bool,
    pub active_tab: i64,
    pub owns_first_tab_back_swipe: bool,
    pub yields_first_tab_back_swipe_to_navigation: bool,
    pub right_to_left: bool,
}

impl AnimatedTabsPagerPanConfiguration {
    pub fn new(
        is_locked: bool,
        active_tab: i64,
        owns_first_tab_back_swipe: bool,
        yields_first_tab_back_swipe_to_navigation: bool,
        right_to_left: bool,
    ) -> Self {
        Self {
            is_locked,
            active_tab,
            owns_first_tab_back_swipe,
            yields_first_tab_back_swipe_to_navigation,
            right_to_left,
        }
    }
}

pub type AnimatedTabsPagerPanAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type AnimatedTabsPagerPanChanged = Rc<dyn Fn(f32, &mut Window, &mut App)>;
pub type AnimatedTabsPagerPanEnded = Rc<dyn Fn(f32, f32, &mut Window, &mut App)>;

#[derive(Clone)]
pub struct AnimatedTabsPagerPanCallbacks {
    pub on_began: AnimatedTabsPagerPanAction,
    pub on_changed: AnimatedTabsPagerPanChanged,
    pub on_ended: AnimatedTabsPagerPanEnded,
    pub on_cancelled: AnimatedTabsPagerPanAction,
    pub on_touch_down: AnimatedTabsPagerPanAction,
}

pub struct AnimatedTabsPagerPanIntentPolicy;

impl AnimatedTabsPagerPanIntentPolicy {
    pub const NAVIGATION_POP_RECOGNIZER_NAME: &str = "riseonly.fullscreenInteractivePop";

    pub const HORIZONTAL_DOMINANCE: f32 = 1.1;
    pub const EARLY_VERTICAL_DISTANCE: f32 = 2.0;
    pub const EARLY_VERTICAL_DOMINANCE: f32 = 2.0;

    pub fn rejects_early_vertical(translation: Point<Pixels>) -> bool {
        let x = f32::from(translation.x).abs();
        let y = f32::from(translation.y).abs();
        y >= Self::EARLY_VERTICAL_DISTANCE && y >= x * Self::EARLY_VERTICAL_DOMINANCE
    }

    pub fn is_horizontal(translation: Point<Pixels>) -> bool {
        f32::from(translation.x).abs() > f32::from(translation.y).abs() * Self::HORIZONTAL_DOMINANCE
    }

    pub fn yields_to_navigation(translation_x: f32, configuration: &AnimatedTabsPagerPanConfiguration) -> bool {
        if !configuration.yields_first_tab_back_swipe_to_navigation
            || configuration.owns_first_tab_back_swipe
            || configuration.active_tab != 0
        {
            return false;
        }
        translation_x > 0.0
    }

    /// true if a scroller owns the horizontal axis, meaning more than a point of horizontal range
    /// gpui scrollers have no bounce or paging switches so range alone decides
    pub fn scrolls_horizontally(scroll_view: &ScrollHandle) -> bool {
        scroll_view.max_offset().x > px(1.0)
    }
}

/// what owns the drag where a pointer lands, so the pager declines it
#[derive(Clone)]
pub enum AnimatedTabsPagerAxisOwner {
    ValueControl,
    ScrollView(ScrollHandle),
    ReportingScrollView(ScrollHandle, Rc<dyn LeadingPageReporting>),
    HorizontalStrip,
}

/// axis owners laid out inside the pager this frame
/// gpui has no hit test for which view is under a point so owners declare themselves
/// and the gate looks them up by bounds
#[derive(Clone, Default)]
pub struct AnimatedTabsPagerAxisOwners(Rc<RefCell<Vec<(Bounds<Pixels>, AnimatedTabsPagerAxisOwner)>>>);

impl AnimatedTabsPagerAxisOwners {
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub fn register(&self, bounds: Bounds<Pixels>, owner: AnimatedTabsPagerAxisOwner) {
        self.0.borrow_mut().push((bounds, owner));
    }

    /// true if any owner is under the point and claims it
    pub fn owns(&self, location: Point<Pixels>) -> bool {
        self.0.borrow().iter().any(|(bounds, owner)| {
            bounds.contains(&location)
                && match owner {
                    AnimatedTabsPagerAxisOwner::ValueControl | AnimatedTabsPagerAxisOwner::HorizontalStrip => true,
                    AnimatedTabsPagerAxisOwner::ScrollView(scroll_view)
                    | AnimatedTabsPagerAxisOwner::ReportingScrollView(scroll_view, _) => {
                        AnimatedTabsPagerPanIntentPolicy::scrolls_horizontally(scroll_view)
                    }
                }
        })
    }

    pub fn owners_at(&self, location: Point<Pixels>) -> Vec<AnimatedTabsPagerAxisOwner> {
        self.0
            .borrow()
            .iter()
            .filter(|(bounds, _)| bounds.contains(&location))
            .map(|(_, owner)| owner.clone())
            .collect()
    }
}

/// registry of the pager the subtree is in, none outside a pager
pub struct AnimatedTabsPagerAxisOwnersKey;

impl EnvKey for AnimatedTabsPagerAxisOwnersKey {
    type Value = Option<AnimatedTabsPagerAxisOwners>;

    fn default_value() -> Self::Value {
        None
    }
}

/// marks child as an axis owner for the enclosing pager, adds no layout node
pub fn animated_tabs_pager_axis_owner(
    owner: AnimatedTabsPagerAxisOwner,
    child: impl IntoElement,
) -> AnimatedTabsPagerAxisOwnerElement {
    AnimatedTabsPagerAxisOwnerElement { owner, child: child.into_any_element() }
}

pub struct AnimatedTabsPagerAxisOwnerElement {
    owner: AnimatedTabsPagerAxisOwner,
    child: AnyElement,
}

impl IntoElement for AnimatedTabsPagerAxisOwnerElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for AnimatedTabsPagerAxisOwnerElement {
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
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(registry) = env::<AnimatedTabsPagerAxisOwnersKey>(cx) {
            let visible = bounds.intersect(&window.content_mask().bounds);
            registry.register(visible, self.owner.clone());
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

/// installer built by the pager each render
/// hands the config and callbacks to the coordinator and puts its listeners on the pager element
pub struct AnimatedTabsPagerGestureGate {
    pub configuration: AnimatedTabsPagerPanConfiguration,
    pub callbacks: AnimatedTabsPagerPanCallbacks,
}

impl AnimatedTabsPagerGestureGate {
    pub fn new(configuration: AnimatedTabsPagerPanConfiguration, callbacks: AnimatedTabsPagerPanCallbacks) -> Self {
        Self { configuration, callbacks }
    }

    pub fn make_coordinator(&self) -> AnimatedTabsPagerGestureGateCoordinator {
        AnimatedTabsPagerGestureGateCoordinator::new(self.configuration, self.callbacks.clone())
    }

    pub fn update_ui_view(
        self,
        host: Stateful<Div>,
        coordinator: &Entity<AnimatedTabsPagerGestureGateCoordinator>,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let configuration = self.configuration;
        let callbacks = self.callbacks;
        let cancelled = coordinator.update(cx, move |coordinator, _| coordinator.update(configuration, callbacks));
        if let Some(cancelled) = cancelled {
            window.defer(cx, move |window, cx| (cancelled.on_cancelled)(window, cx));
        }

        let wheel = coordinator.clone();
        let down = coordinator.clone();
        let moved = coordinator.clone();
        let up_capture = coordinator.clone();
        let up = coordinator.clone();
        let up_out = coordinator.clone();

        let recognizer = canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox: Hitbox, window, _| {
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let hovered = hitbox.should_handle_scroll(window);
                    let outcome = wheel.update(cx, |coordinator, _| coordinator.handle_scroll_wheel(event, hovered));
                    if outcome.consumed {
                        cx.stop_propagation();
                    }
                    outcome.dispatch(window, cx);
                });
            },
        )
        .absolute()
        .inset_0();

        host.child(recognizer)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                let outcome = down.update(cx, |coordinator, _| coordinator.touches_began(event.position, PanSource::Mouse));
                outcome.dispatch(window, cx);
            })
            .on_mouse_move(move |event: &MouseMoveEvent, window, cx| {
                if event.pressed_button != Some(MouseButton::Left) || cx.has_active_drag() {
                    return;
                }
                let outcome = moved.update(cx, |coordinator, _| coordinator.mouse_moved(event.position));
                outcome.dispatch(window, cx);
            })
            .capture_any_mouse_up(move |_event: &MouseUpEvent, window, cx| {
                let outcome = up_capture.update(cx, |coordinator, _| coordinator.touches_ended(PanSource::Mouse));
                if outcome.consumed {
                    cx.stop_propagation();
                }
                outcome.dispatch(window, cx);
            })
            .on_mouse_up(MouseButton::Left, move |_event: &MouseUpEvent, window, cx| {
                let outcome = up.update(cx, |coordinator, _| coordinator.touches_ended(PanSource::Mouse));
                outcome.dispatch(window, cx);
            })
            .on_mouse_up_out(MouseButton::Left, move |_event: &MouseUpEvent, window, cx| {
                let outcome = up_out.update(cx, |coordinator, _| coordinator.touches_ended(PanSource::Mouse));
                outcome.dispatch(window, cx);
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanSource {
    Wheel,
    Mouse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecognizerState {
    Idle,
    Possible,
    Failed,
    Tracking,
}

enum PanEvent {
    TouchDown,
    Began,
    Changed(f32),
    Ended(f32, f32),
    Cancelled,
}

/// what one input event produced, callbacks to run outside the coordinator update
/// and whether the gate took the event from the elements behind it
pub struct PanOutcome {
    callbacks: Option<AnimatedTabsPagerPanCallbacks>,
    events: Vec<PanEvent>,
    consumed: bool,
}

impl PanOutcome {
    fn none() -> Self {
        Self { callbacks: None, events: Vec::new(), consumed: false }
    }

    fn dispatch(self, window: &mut Window, cx: &mut App) {
        let Some(callbacks) = self.callbacks else { return };
        for event in self.events {
            match event {
                PanEvent::TouchDown => (callbacks.on_touch_down)(window, cx),
                PanEvent::Began => (callbacks.on_began)(window, cx),
                PanEvent::Changed(translation) => (callbacks.on_changed)(translation, window, cx),
                PanEvent::Ended(translation, velocity) => (callbacks.on_ended)(translation, velocity, window, cx),
                PanEvent::Cancelled => (callbacks.on_cancelled)(window, cx),
            }
        }
    }
}

/// pan the gate owns, fails on early vertical intent before its activation distance
/// so the page own vertical scroll keeps the pointer from the first pixel
struct AnimatedTabsDirectionalPanGestureRecognizer {
    state: RecognizerState,
    source: Option<PanSource>,
    start_location: Point<Pixels>,
    translation: Point<Pixels>,
    velocity_x: f32,
    last_sample: Duration,
}

impl AnimatedTabsDirectionalPanGestureRecognizer {
    fn new() -> Self {
        Self {
            state: RecognizerState::Idle,
            source: None,
            start_location: point(Pixels::ZERO, Pixels::ZERO),
            translation: point(Pixels::ZERO, Pixels::ZERO),
            velocity_x: 0.0,
            last_sample: Duration::ZERO,
        }
    }

    fn reset(&mut self) {
        self.state = RecognizerState::Idle;
        self.source = None;
        self.translation = point(Pixels::ZERO, Pixels::ZERO);
        self.velocity_x = 0.0;
    }
}

/// gate coordinator together with the directional recognizer it owns
pub struct AnimatedTabsPagerGestureGateCoordinator {
    configuration: AnimatedTabsPagerPanConfiguration,
    callbacks: AnimatedTabsPagerPanCallbacks,
    activation_offset: f32,
    is_tracking: bool,
    suppress_recognizer_cancellation_callback: bool,
    pan: AnimatedTabsDirectionalPanGestureRecognizer,
    axis_owners: AnimatedTabsPagerAxisOwners,
    yielded_leading_pager: Option<ScrollHandle>,
}

impl AnimatedTabsPagerGestureGateCoordinator {
    const ACTIVATION_DISTANCE: f32 = 10.0;
    const VELOCITY_LIFETIME: Duration = Duration::from_millis(50);
    const WHEEL_LINE_HEIGHT: f32 = 20.0;

    pub fn new(configuration: AnimatedTabsPagerPanConfiguration, callbacks: AnimatedTabsPagerPanCallbacks) -> Self {
        Self {
            configuration,
            callbacks,
            activation_offset: 0.0,
            is_tracking: false,
            suppress_recognizer_cancellation_callback: false,
            pan: AnimatedTabsDirectionalPanGestureRecognizer::new(),
            axis_owners: AnimatedTabsPagerAxisOwners::default(),
            yielded_leading_pager: None,
        }
    }

    /// registry the pager hands its pages, the pager clears it each render
    pub fn axis_owners(&self) -> AnimatedTabsPagerAxisOwners {
        self.axis_owners.clone()
    }

    /// returns the callbacks of a cancellation the update caused, for the caller to run
    pub fn update(
        &mut self,
        configuration: AnimatedTabsPagerPanConfiguration,
        callbacks: AnimatedTabsPagerPanCallbacks,
    ) -> Option<AnimatedTabsPagerPanCallbacks> {
        self.callbacks = callbacks;
        let was_locked = self.configuration.is_locked;
        self.configuration = configuration;

        if configuration.is_locked == was_locked || !configuration.is_locked {
            return None;
        }
        self.cancel_recognizer_if_active()
    }

    pub fn detach(&mut self) -> Option<AnimatedTabsPagerPanCallbacks> {
        let cancelled = self.cancel_recognizer_if_active();
        self.reset();
        cancelled
    }

    fn gesture_recognizer_should_begin(&mut self, start: Point<Pixels>, translation: Point<Pixels>) -> bool {
        if self.configuration.is_locked {
            return false;
        }
        if !AnimatedTabsPagerPanIntentPolicy::is_horizontal(translation) {
            return false;
        }
        self.yielded_leading_pager = None;
        let is_back_drag = self.reading_x(f32::from(translation.x)) > 0.0;
        if self.starts_on_own_axis_owner(start, is_back_drag) {
            return false;
        }
        !AnimatedTabsPagerPanIntentPolicy::yields_to_navigation(self.reading_x(f32::from(translation.x)), &self.configuration)
    }

    fn starts_on_own_axis_owner(&mut self, location: Point<Pixels>, is_back_drag: bool) -> bool {
        for owner in self.axis_owners.owners_at(location) {
            let (scroll_view, rests_at_leading_page) = match owner {
                AnimatedTabsPagerAxisOwner::ValueControl | AnimatedTabsPagerAxisOwner::HorizontalStrip => return true,
                AnimatedTabsPagerAxisOwner::ScrollView(scroll_view) => {
                    let rests = Self::rests_at_leading_page(&scroll_view, None);
                    (scroll_view, rests)
                }
                AnimatedTabsPagerAxisOwner::ReportingScrollView(scroll_view, pager) => {
                    let rests = Self::rests_at_leading_page(&scroll_view, Some(pager.as_ref()));
                    (scroll_view, rests)
                }
            };
            if !AnimatedTabsPagerPanIntentPolicy::scrolls_horizontally(&scroll_view) {
                continue;
            }
            if !(is_back_drag && rests_at_leading_page) {
                return true;
            }
            self.yielded_leading_pager = Some(scroll_view);
        }
        false
    }

    fn rests_at_leading_page(scroll_view: &ScrollHandle, pager: Option<&dyn LeadingPageReporting>) -> bool {
        if let Some(pager) = pager {
            return pager.rests_at_leading_page();
        }
        PagerBackSwipeYield::rests_at_leading_page(scroll_view).unwrap_or(false)
    }

    fn suspend_yielded_leading_pager(&mut self) {
        self.yielded_leading_pager = None;
    }

    fn reading_x(&self, x: f32) -> f32 {
        if self.configuration.right_to_left { -x } else { x }
    }

    fn outcome(&self, events: Vec<PanEvent>, consumed: bool) -> PanOutcome {
        if events.is_empty() {
            return PanOutcome { callbacks: None, events, consumed };
        }
        PanOutcome { callbacks: Some(self.callbacks.clone()), events, consumed }
    }

    fn touches_began(&mut self, location: Point<Pixels>, source: PanSource) -> PanOutcome {
        if self.pan.state == RecognizerState::Tracking {
            return PanOutcome::none();
        }
        self.pan.state = RecognizerState::Possible;
        self.pan.source = Some(source);
        self.pan.start_location = location;
        self.pan.translation = point(Pixels::ZERO, Pixels::ZERO);
        self.pan.velocity_x = 0.0;
        self.pan.last_sample = RiseClock::monotonic();
        if self.starts_on_own_axis_owner(location, true) {
            self.pan.state = RecognizerState::Failed;
        }
        self.yielded_leading_pager = None;
        self.outcome(vec![PanEvent::TouchDown], false)
    }

    fn mouse_moved(&mut self, location: Point<Pixels>) -> PanOutcome {
        if self.pan.source != Some(PanSource::Mouse) {
            return PanOutcome::none();
        }
        let translation = point(location.x - self.pan.start_location.x, location.y - self.pan.start_location.y);
        self.touches_moved(translation)
    }

    fn touches_moved(&mut self, translation: Point<Pixels>) -> PanOutcome {
        let now = RiseClock::monotonic();
        let previous_x = f32::from(self.pan.translation.x);
        self.pan.translation = translation;
        match self.pan.state {
            RecognizerState::Possible => {
                if AnimatedTabsPagerPanIntentPolicy::rejects_early_vertical(translation) {
                    self.pan.state = RecognizerState::Failed;
                    return PanOutcome::none();
                }
                let distance = f32::from(translation.x).hypot(f32::from(translation.y));
                if distance < Self::ACTIVATION_DISTANCE {
                    return PanOutcome::none();
                }
                if !self.gesture_recognizer_should_begin(self.pan.start_location, translation) {
                    self.pan.state = RecognizerState::Failed;
                    return PanOutcome::none();
                }
                self.pan.state = RecognizerState::Tracking;
                self.is_tracking = true;
                self.activation_offset = self.reading_x(f32::from(translation.x));
                self.suspend_yielded_leading_pager();
                self.pan.last_sample = now;
                self.pan.velocity_x = 0.0;
                self.outcome(vec![PanEvent::Began, PanEvent::Changed(0.0)], true)
            }
            RecognizerState::Tracking => {
                let elapsed = now.saturating_sub(self.pan.last_sample).as_secs_f32();
                if elapsed > 0.0 {
                    self.pan.velocity_x = (f32::from(translation.x) - previous_x) / elapsed;
                }
                self.pan.last_sample = now;
                let resolved = self.reading_x(f32::from(translation.x)) - self.activation_offset;
                self.outcome(vec![PanEvent::Changed(resolved)], true)
            }
            RecognizerState::Idle | RecognizerState::Failed => PanOutcome::none(),
        }
    }

    fn touches_ended(&mut self, source: PanSource) -> PanOutcome {
        if self.pan.source != Some(source) {
            return PanOutcome::none();
        }
        if !self.is_tracking {
            self.reset();
            return PanOutcome::none();
        }
        let resolved = self.reading_x(f32::from(self.pan.translation.x)) - self.activation_offset;
        let rested = RiseClock::monotonic().saturating_sub(self.pan.last_sample) > Self::VELOCITY_LIFETIME;
        let velocity = if rested { 0.0 } else { self.reading_x(self.pan.velocity_x) };
        self.finish_tracking();
        self.reset();
        self.outcome(vec![PanEvent::Ended(resolved, velocity)], true)
    }

    fn touches_cancelled(&mut self) -> PanOutcome {
        let was_tracking = self.is_tracking;
        self.finish_tracking();
        self.reset();
        if !was_tracking || self.suppress_recognizer_cancellation_callback {
            return PanOutcome::none();
        }
        self.outcome(vec![PanEvent::Cancelled], true)
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, hovered: bool) -> PanOutcome {
        match event.delta {
            ScrollDelta::Pixels(delta) => match event.touch_phase {
                TouchPhase::Started => {
                    if !hovered || self.configuration.is_locked {
                        return PanOutcome::none();
                    }
                    self.touches_began(event.position, PanSource::Wheel)
                }
                TouchPhase::Moved => {
                    if self.pan.source != Some(PanSource::Wheel) {
                        return PanOutcome::none();
                    }
                    let delta = Self::wheel_axis(delta, event.modifiers.shift);
                    let translation = point(self.pan.translation.x + delta.x, self.pan.translation.y + delta.y);
                    let mut outcome = self.touches_moved(translation);
                    if self.pan.state == RecognizerState::Possible
                        && AnimatedTabsPagerPanIntentPolicy::is_horizontal(translation)
                    {
                        outcome.consumed = true;
                    }
                    outcome
                }
                TouchPhase::Ended => self.touches_ended(PanSource::Wheel),
                TouchPhase::Cancelled => self.touches_cancelled(),
            },
            ScrollDelta::Lines(lines) => {
                if !hovered {
                    return PanOutcome::none();
                }
                self.wheel_step(lines, event.modifiers.shift, event.position)
            }
        }
    }

    fn wheel_axis(delta: Point<Pixels>, shift: bool) -> Point<Pixels> {
        if shift && delta.x == Pixels::ZERO {
            point(delta.y, Pixels::ZERO)
        } else {
            delta
        }
    }

    fn wheel_step(&mut self, lines: Point<f32>, shift: bool, location: Point<Pixels>) -> PanOutcome {
        if self.pan.state == RecognizerState::Tracking {
            return PanOutcome::none();
        }
        let x = if shift && lines.x == 0.0 { lines.y } else { lines.x };
        let y = if shift && lines.x == 0.0 { 0.0 } else { lines.y };
        let translation = point(Pixels::from(x * Self::WHEEL_LINE_HEIGHT), Pixels::from(y * Self::WHEEL_LINE_HEIGHT));
        if !self.gesture_recognizer_should_begin(location, translation) {
            return PanOutcome::none();
        }
        let direction = self.reading_x(x).signum();
        let velocity = direction * AnimatedTabsMetrics::PAGE_VELOCITY_COMMIT_THRESHOLD;
        self.reset();
        self.outcome(
            vec![PanEvent::TouchDown, PanEvent::Began, PanEvent::Changed(0.0), PanEvent::Ended(0.0, velocity)],
            true,
        )
    }

    fn finish_tracking(&mut self) {
        self.is_tracking = false;
        self.activation_offset = 0.0;
    }

    fn reset(&mut self) {
        self.pan.reset();
    }

    /// exactly one cancellation reaches the pager
    /// the recognizers own one is suppressed and the returned callbacks deliver it
    fn cancel_recognizer_if_active(&mut self) -> Option<AnimatedTabsPagerPanCallbacks> {
        if !self.is_tracking {
            return None;
        }
        self.suppress_recognizer_cancellation_callback = true;
        let cancelled = self.touches_cancelled();
        self.suppress_recognizer_cancellation_callback = false;
        debug_assert!(cancelled.callbacks.is_none());
        Some(self.callbacks.clone())
    }
}
