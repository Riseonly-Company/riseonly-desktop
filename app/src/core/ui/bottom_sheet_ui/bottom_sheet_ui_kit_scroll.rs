use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, DispatchPhase, ElementId, EntityId, Hitbox, HitboxBehavior,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    RenderOnce, ScrollHandle, ScrollWheelEvent, TouchPhase, Window, canvas, point, px,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_axis_ownership::{
    BottomSheetAxisOwnership, BottomSheetGestureRecognizer, BottomSheetGestureState,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_content_pan_gate::{
    BottomSheetNestedScrollKey, BottomSheetNestedScrollRegistry,
};
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::bottom_sheet_ui::bottom_sheet_motion_host::{
    BottomSheetContentHeightHolding, BottomSheetMotionController,
};
use rise_ui::swiftui::prelude::*;

const PAN_HYSTERESIS: f32 = 10.0;
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

pub type BottomSheetUIKitScrollAction = Rc<dyn Fn(&mut Window, &mut App)>;
/// translation y and velocity y of the released pull
pub type BottomSheetUIKitScrollPullEnd = Rc<dyn Fn(Pixels, Pixels, &mut Window, &mut App)>;
pub type BottomSheetUIKitScrollContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// scrollable body of a sheet
/// cloned into its coordinator on every render so it gets the new parent
#[derive(IntoElement, Clone)]
pub struct BottomSheetUIKitScroll {
    pub id: ElementId,
    pub motion_controller: BottomSheetMotionController,
    pub is_sheet_interaction_locked: bool,
    pub on_sheet_drag_began: BottomSheetUIKitScrollAction,
    pub can_expand_from_scroll_top: bool,
    pub sheet_expansion_distance: Pixels,
    pub on_sheet_pull_end: BottomSheetUIKitScrollPullEnd,
    pub content: BottomSheetUIKitScrollContent,
}

impl BottomSheetUIKitScroll {
    /// defaults are is_sheet_interaction_locked false
    /// on_sheet_drag_began empty and sheet_expansion_distance 0
    pub fn new(
        id: impl Into<ElementId>,
        motion_controller: BottomSheetMotionController,
        can_expand_from_scroll_top: bool,
        on_sheet_pull_end: impl Fn(Pixels, Pixels, &mut Window, &mut App) + 'static,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            motion_controller,
            is_sheet_interaction_locked: false,
            on_sheet_drag_began: Rc::new(|_: &mut Window, _: &mut App| {}),
            can_expand_from_scroll_top,
            sheet_expansion_distance: px(0.0),
            on_sheet_pull_end: Rc::new(on_sheet_pull_end),
            content: Rc::new(content),
        }
    }

    pub fn is_sheet_interaction_locked(mut self, locked: bool) -> Self {
        self.is_sheet_interaction_locked = locked;
        self
    }

    pub fn on_sheet_drag_began(mut self, action: BottomSheetUIKitScrollAction) -> Self {
        self.on_sheet_drag_began = action;
        self
    }

    pub fn sheet_expansion_distance(mut self, distance: Pixels) -> Self {
        self.sheet_expansion_distance = distance;
        self
    }

    pub fn make_coordinator(&self) -> Coordinator {
        Coordinator::new(self.clone())
    }
}

impl RenderOnce for BottomSheetUIKitScroll {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let parent = self.clone();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| {
            Rc::new(RefCell::new(parent.make_coordinator()))
        });
        let coordinator = state.read(cx).clone();
        let view = window.current_view();

        let (box_view, registry) = {
            let mut coordinator = coordinator.borrow_mut();
            coordinator.parent = self.clone();
            let box_view = coordinator.host_view.clone();
            box_view.view.set(Some(view));
            (box_view, coordinator.registry.clone())
        };
        let holder: Rc<dyn BottomSheetContentHeightHolding> = box_view.clone();
        self.motion_controller.register_content_height_holder(&holder);

        registry.clear();
        let content = (self.content)(window, cx);
        let hosted = env_scope::<BottomSheetNestedScrollKey>(Some(registry)).child(content);

        let embedded = match box_view.held_content_height.get() {
            Some(held) => div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(held.max(box_view.bounds.get().size.height).max(px(1.0)))
                .child(hosted),
            None => div().absolute().inset_0().child(hosted),
        };

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(embedded)
            .child(Coordinator::recognizer_overlay(&coordinator))
    }
}

/// box the sheet gives its content, content hangs from the top and normally equals the box
/// while the surface moves its held at the height the motion ends at
/// so the box changing every frame lays out nothing inside
pub struct EmbeddedHostView {
    coordinator: RefCell<Weak<RefCell<Coordinator>>>,
    bounds: Cell<Bounds<Pixels>>,
    held_content_height: Cell<Option<Pixels>>,
    view: Cell<Option<EntityId>>,
}

impl EmbeddedHostView {
    fn new() -> Self {
        Self {
            coordinator: RefCell::new(Weak::new()),
            bounds: Cell::new(Bounds::default()),
            held_content_height: Cell::new(None),
            view: Cell::new(None),
        }
    }

    fn set_needs_layout(&self, cx: &mut App) {
        if let Some(view) = self.view.get() {
            cx.notify(view);
        }
    }

    fn layout_subviews(&self, bounds: Bounds<Pixels>) {
        self.bounds.set(bounds);
        let Some(coordinator) = self.coordinator.borrow().upgrade() else {
            return;
        };
        let Ok(mut coordinator) = coordinator.try_borrow_mut() else {
            return;
        };
        if coordinator.is_actively_dragging || self.held_content_height.get().is_some() {
            return;
        }
        coordinator.refresh_inner_scroll();
    }
}

impl BottomSheetContentHeightHolding for EmbeddedHostView {
    fn hold_content_height(&self, extra: Option<Pixels>, cx: &mut App) {
        let Some(extra) = extra else {
            if self.held_content_height.get().is_none() {
                return;
            }
            self.held_content_height.set(None);
            self.set_needs_layout(cx);
            return;
        };
        let next = self.bounds.get().size.height + extra.max(px(0.0));
        if next <= self.held_content_height.get().unwrap_or(px(0.0)) + px(0.5) {
            return;
        }
        self.held_content_height.set(Some(next));
        self.set_needs_layout(cx);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanAxis {
    Undetermined,
    Horizontal,
    Vertical,
}

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

pub struct Coordinator {
    pub parent: BottomSheetUIKitScroll,
    pub host_view: Rc<EmbeddedHostView>,
    pub sheet_pan: BottomSheetGestureRecognizer,
    registry: BottomSheetNestedScrollRegistry,
    tracking: Option<Tracking>,

    inner_scroll: Option<ScrollHandle>,
    gesture_inner_scroll: Option<ScrollHandle>,
    sheet_pull_active: bool,
    sheet_expand_active: bool,
    did_handoff_to_inner: bool,
    pan_began_at_scroll_top: bool,
    pan_axis: PanAxis,
    is_actively_dragging: bool,
    gesture_start_time: Option<Instant>,
    last_pull_translation: Pixels,
    pull_activation_translation: Pixels,
    active_expansion_distance: Pixels,
}

impl Coordinator {
    pub fn new(parent: BottomSheetUIKitScroll) -> Self {
        Self {
            parent,
            host_view: Rc::new(EmbeddedHostView::new()),
            sheet_pan: BottomSheetGestureRecognizer::new(),
            registry: BottomSheetNestedScrollRegistry::default(),
            tracking: None,
            inner_scroll: None,
            gesture_inner_scroll: None,
            sheet_pull_active: false,
            sheet_expand_active: false,
            did_handoff_to_inner: false,
            pan_began_at_scroll_top: false,
            pan_axis: PanAxis::Undetermined,
            is_actively_dragging: false,
            gesture_start_time: None,
            last_pull_translation: px(0.0),
            pull_activation_translation: px(0.0),
            active_expansion_distance: px(0.0),
        }
    }

    pub fn is_actively_dragging(&self) -> bool {
        self.is_actively_dragging
    }

    pub fn refresh_inner_scroll(&mut self) {
        self.inner_scroll = Self::pick_primary_scroll_view(&self.registry);
    }

    pub fn refresh_inner_scroll_at_touch_point(&mut self, point: Point<Pixels>) {
        self.inner_scroll = Self::scroll_view_at_point(point, &self.registry)
            .or_else(|| Self::pick_primary_scroll_view(&self.registry));
    }

    fn scroll_view_at_point(
        point: Point<Pixels>,
        registry: &BottomSheetNestedScrollRegistry,
    ) -> Option<ScrollHandle> {
        registry
            .handles()
            .into_iter()
            .rev()
            .find(|scroll_view| {
                scroll_view.bounds().contains(&point) && Self::scrolls_vertically_by_touch(scroll_view)
            })
    }

    fn scrolls_vertically_by_touch(scroll_view: &ScrollHandle) -> bool {
        let max_offset = scroll_view.max_offset();
        let sideways = max_offset.x > px(0.5) && max_offset.y <= px(0.5);
        !sideways
    }

    fn pick_primary_scroll_view(registry: &BottomSheetNestedScrollRegistry) -> Option<ScrollHandle> {
        registry
            .handles()
            .into_iter()
            .filter(|scroll_view| scroll_view.max_offset().y > px(0.5))
            .max_by(|lhs, rhs| {
                let lhs = lhs.bounds().size;
                let rhs = rhs.bounds().size;
                f32::from(lhs.width * f32::from(lhs.height))
                    .total_cmp(&f32::from(rhs.width * f32::from(rhs.height)))
            })
    }

    /// gpui offsets go negative as the list scrolls down
    fn is_at_scroll_top(&self) -> bool {
        let Some(scroll_view) = self.inner_scroll.as_ref() else {
            return true;
        };
        let slack = -scroll_view.offset().y;
        slack <= px(BottomSheetMetrics::SCROLL_TOP_SLACK)
    }

    fn lock_inner_to_top(&self, scroll_view: Option<&ScrollHandle>) {
        let Some(scroll_view) = scroll_view
            .or(self.gesture_inner_scroll.as_ref())
            .or(self.inner_scroll.as_ref())
        else {
            return;
        };
        let offset = scroll_view.offset();
        if offset.y.abs() > px(0.5) {
            scroll_view.set_offset(point(offset.x, px(0.0)));
        }
    }

    /// pins the offset of the inner list while the sheet moves
    /// gpui lists dont bounce or keep their own pan so pinning is all that is needed
    fn pin_inner_for_sheet_movement(&self) {
        self.lock_inner_to_top(None);
    }

    fn reset_gesture_state(&mut self, cx: &mut App) {
        BottomSheetAxisOwnership::release_vertical(&self.sheet_pan, cx);
        self.sheet_pan.set_state(BottomSheetGestureState::Possible);
        self.gesture_inner_scroll = None;
        self.pan_began_at_scroll_top = false;
        self.pan_axis = PanAxis::Undetermined;
        self.sheet_pull_active = false;
        self.sheet_expand_active = false;
        self.did_handoff_to_inner = false;
        self.pull_activation_translation = px(0.0);
        self.active_expansion_distance = px(0.0);
    }

    /// list kept trackpad momentum the sheet never handed it
    /// pins it back to the top now and on the next two frames unless a new gesture holds it
    fn cancel_unhanded_inner_momentum(
        this: &Rc<RefCell<Self>>,
        scroll_view: ScrollHandle,
        window: &mut Window,
    ) {
        Self::repin(&scroll_view);
        let weak = Rc::downgrade(this);
        window.on_next_frame(move |window, _| {
            let is_tracking = |weak: &Weak<RefCell<Self>>| {
                weak.upgrade()
                    .is_some_and(|this| this.try_borrow().map_or(true, |this| this.tracking.is_some()))
            };
            if !is_tracking(&weak) {
                Self::repin(&scroll_view);
            }
            window.on_next_frame(move |_, _| {
                if !is_tracking(&weak) {
                    Self::repin(&scroll_view);
                }
                Self::log_gesture("sheet_child_repin");
            });
        });
    }

    fn repin(scroll_view: &ScrollHandle) {
        let offset = scroll_view.offset();
        scroll_view.set_offset(point(offset.x, px(0.0)));
    }

    fn handle_sheet_pan(
        this: &Rc<RefCell<Self>>,
        state: BottomSheetGestureState,
        location: Point<Pixels>,
        translation: Point<Pixels>,
        velocity: Point<Pixels>,
        now: Instant,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut coordinator = this.borrow_mut();
        let ty = translation.y;
        coordinator.sheet_pan.set_state(state);

        match state {
            BottomSheetGestureState::Began => {
                coordinator.gesture_start_time = Some(now);
                coordinator.last_pull_translation = px(0.0);
                coordinator.pull_activation_translation = ty;
                coordinator.refresh_inner_scroll_at_touch_point(location);
                coordinator.gesture_inner_scroll = coordinator.inner_scroll.clone();
                coordinator.pan_began_at_scroll_top = coordinator.is_at_scroll_top();
                coordinator.did_handoff_to_inner = false;
                coordinator.pan_axis = PanAxis::Undetermined;

                if velocity.y.abs() > velocity.x.abs() * 0.9 {
                    coordinator.pan_axis = PanAxis::Vertical;
                }

                if coordinator.pan_began_at_scroll_top
                    && coordinator.pan_axis == PanAxis::Vertical
                    && Self::can_drive_sheet(cx)
                    && !coordinator.is_sheet_locked(cx)
                {
                    if velocity.y > px(0.0) {
                        coordinator.sheet_pull_active = true;
                        coordinator.begin_sheet_drive(window, cx);
                    } else if coordinator.live_expansion_room(cx) > px(0.5) {
                        coordinator.sheet_expand_active = true;
                        coordinator.begin_sheet_drive(window, cx);
                        Self::log_gesture(&format!(
                            "sheet_expand_begin distance={}",
                            f32::from(coordinator.active_expansion_distance).round() as i64
                        ));
                    }
                }
            }

            BottomSheetGestureState::Changed => {
                if coordinator.is_sheet_locked(cx) {
                    return;
                }

                if coordinator.pan_axis == PanAxis::Undetermined {
                    let dx = translation.x.abs();
                    let dy = ty.abs();
                    if dx.max(dy) >= px(5.0) {
                        coordinator.pan_axis =
                            if dx > dy * 1.08 { PanAxis::Horizontal } else { PanAxis::Vertical };
                    }
                }

                if coordinator.pan_axis != PanAxis::Vertical || !Self::can_drive_sheet(cx) {
                    return;
                }

                if !coordinator.sheet_pull_active
                    && !coordinator.sheet_expand_active
                    && coordinator.pan_began_at_scroll_top
                    && ty > px(BottomSheetMetrics::SHEET_PULL_COMMIT_THRESHOLD)
                {
                    coordinator.sheet_pull_active = true;
                    coordinator.pull_activation_translation = ty;
                    coordinator.begin_sheet_drive(window, cx);
                }

                if !coordinator.sheet_pull_active
                    && !coordinator.sheet_expand_active
                    && coordinator.pan_began_at_scroll_top
                    && coordinator.live_expansion_room(cx) > px(0.5)
                    && ty < -px(BottomSheetMetrics::SHEET_EXPAND_COMMIT_THRESHOLD)
                {
                    coordinator.sheet_expand_active = true;
                    coordinator.pull_activation_translation = ty;
                    coordinator.begin_sheet_drive(window, cx);
                    Self::log_gesture(&format!(
                        "sheet_expand_begin distance={}",
                        f32::from(coordinator.active_expansion_distance).round() as i64
                    ));
                }

                let adjusted_ty = ty - coordinator.pull_activation_translation;

                if coordinator.sheet_pull_active {
                    coordinator.pin_inner_for_sheet_movement();
                    coordinator.last_pull_translation = adjusted_ty;
                    coordinator.parent.motion_controller.update_interactive_translation(adjusted_ty, cx);
                } else if coordinator.sheet_expand_active {
                    let expansion_distance = coordinator.active_expansion_distance.max(px(0.0));
                    if coordinator.did_handoff_to_inner {
                        return;
                    } else if adjusted_ty > -expansion_distance {
                        coordinator.last_pull_translation = adjusted_ty;
                        coordinator.parent.motion_controller.update_interactive_translation(adjusted_ty, cx);
                        coordinator.pin_inner_for_sheet_movement();
                    } else {
                        coordinator.last_pull_translation = -expansion_distance;
                        coordinator
                            .parent
                            .motion_controller
                            .update_interactive_translation(-expansion_distance, cx);
                        coordinator.did_handoff_to_inner = true;
                        let elapsed_ms = coordinator.elapsed_ms(now);
                        Self::log_gesture(&format!(
                            "sheet_handoff elapsed_ms={} distance={}",
                            elapsed_ms,
                            f32::from(expansion_distance).round() as i64
                        ));
                    }
                }
            }

            BottomSheetGestureState::Ended
            | BottomSheetGestureState::Cancelled
            | BottomSheetGestureState::Failed => {
                coordinator.is_actively_dragging = false;
                let unhanded_inner_scroll =
                    if coordinator.sheet_expand_active && !coordinator.did_handoff_to_inner {
                        coordinator.gesture_inner_scroll.clone()
                    } else {
                        None
                    };

                if coordinator.is_sheet_locked(cx) {
                    let was_driving = coordinator.sheet_pull_active || coordinator.sheet_expand_active;
                    coordinator.reset_gesture_state(cx);
                    let on_sheet_pull_end = coordinator.parent.on_sheet_pull_end.clone();
                    drop(coordinator);
                    if let Some(scroll_view) = unhanded_inner_scroll {
                        Self::cancel_unhanded_inner_momentum(this, scroll_view, window);
                    }
                    if was_driving {
                        on_sheet_pull_end(px(0.0), px(0.0), window, cx);
                    }
                    return;
                }

                let final_vy = velocity.y;
                let was_driving = coordinator.sheet_pull_active || coordinator.sheet_expand_active;
                let last_pull_translation = coordinator.last_pull_translation;
                if was_driving {
                    let elapsed_ms = coordinator.elapsed_ms(now);
                    Self::log_gesture(&format!(
                        "sheet_gesture_end elapsed_ms={} handoff={} velocity={}",
                        elapsed_ms,
                        i64::from(coordinator.did_handoff_to_inner),
                        f32::from(final_vy).round() as i64
                    ));
                }
                coordinator.reset_gesture_state(cx);
                let on_sheet_pull_end = coordinator.parent.on_sheet_pull_end.clone();
                drop(coordinator);
                if let Some(scroll_view) = unhanded_inner_scroll {
                    Self::cancel_unhanded_inner_momentum(this, scroll_view, window);
                }
                if was_driving {
                    on_sheet_pull_end(last_pull_translation, final_vy, window, cx);
                }
            }

            BottomSheetGestureState::Possible => {}
        }
    }

    fn elapsed_ms(&self, now: Instant) -> i64 {
        self.gesture_start_time
            .map_or(0, |start| now.saturating_duration_since(start).as_millis() as i64)
    }

    /// the sheet update is deferred while a snap animation runs so props captured in parent still
    /// describe the old detent
    /// the motion host holds the live value
    fn live_expansion_room(&self, cx: &App) -> Pixels {
        if let Some(room) = self.parent.motion_controller.expansion_room(cx) {
            return room;
        }
        if self.parent.can_expand_from_scroll_top {
            self.parent.sheet_expansion_distance.max(px(0.0))
        } else {
            px(0.0)
        }
    }

    fn begin_expansion_tracking(&mut self, cx: &App) {
        self.active_expansion_distance = self
            .parent
            .motion_controller
            .interaction_expansion_room(cx)
            .unwrap_or(self.parent.sheet_expansion_distance);
    }

    /// parent comes with the hosted tree which the motion host freezes while the surface moves
    /// so its lock is stale for a whole dismissal, the host answers live
    fn is_sheet_locked(&self, cx: &App) -> bool {
        self.parent.is_sheet_interaction_locked || self.parent.motion_controller.refuses_interaction(cx)
    }

    /// horizontal axis inside the sheet is already driven when a page is moving back
    /// vertical owner must skip this gesture entirely
    fn can_drive_sheet(cx: &App) -> bool {
        !BottomSheetAxisOwnership::horizontal_is_driving(cx)
    }

    /// drive starts only once the surface is really moving
    /// the recognizer begins on any pan and claiming the axis at began starved a horizontal page swipe
    /// the page never moved
    fn begin_sheet_drive(&mut self, window: &mut Window, cx: &mut App) {
        self.is_actively_dragging = true;
        BottomSheetAxisOwnership::claim_vertical(&self.sheet_pan, cx);
        (self.parent.on_sheet_drag_began.clone())(window, cx);
        self.begin_expansion_tracking(cx);
        self.pin_inner_for_sheet_movement();
    }

    fn log_gesture(message: &str) {
        if cfg!(debug_assertions) {
            tracing::debug!(target: "MediaPickerPerf", "[MediaPickerPerf] {message}");
        }
    }

    fn gesture_recognizer_should_begin(
        &mut self,
        location: Point<Pixels>,
        velocity: Point<Pixels>,
        cx: &App,
    ) -> bool {
        if self.is_sheet_locked(cx) || !Self::can_drive_sheet(cx) {
            return false;
        }
        self.refresh_inner_scroll_at_touch_point(location);
        if !self.is_at_scroll_top() {
            return false;
        }
        !(velocity.y < px(0.0) && self.live_expansion_room(cx) <= px(0.5))
    }

    fn gesture_recognizer(&self) -> bool {
        !(self.sheet_pull_active || self.sheet_expand_active) || self.did_handoff_to_inner
    }

    fn advance_tracking(this: &Rc<RefCell<Self>>, now: Instant, window: &mut Window, cx: &mut App) {
        let (location, translation, velocity, recognized) = {
            let mut coordinator = this.borrow_mut();
            let Some(tracking) = coordinator.tracking.as_mut() else {
                return;
            };
            if tracking.failed {
                return;
            }
            tracking.record(now);
            let location = tracking.location;
            let translation = tracking.translation;
            let velocity = tracking.velocity();
            if !tracking.recognized
                && tracking.source == TrackingSource::Mouse
                && translation.x.abs().max(translation.y.abs()) < px(PAN_HYSTERESIS)
            {
                return;
            }
            let recognized = tracking.recognized;
            if !recognized {
                if !coordinator.gesture_recognizer_should_begin(location, velocity, cx) {
                    if let Some(tracking) = coordinator.tracking.as_mut() {
                        tracking.failed = true;
                    }
                    return;
                }
                if let Some(tracking) = coordinator.tracking.as_mut() {
                    tracking.recognized = true;
                }
            }
            (location, translation, velocity, recognized)
        };
        if !recognized {
            Self::handle_sheet_pan(
                this,
                BottomSheetGestureState::Began,
                location,
                translation,
                velocity,
                now,
                window,
                cx,
            );
        }
        Self::handle_sheet_pan(
            this,
            BottomSheetGestureState::Changed,
            location,
            translation,
            velocity,
            now,
            window,
            cx,
        );
    }

    fn end_tracking(
        this: &Rc<RefCell<Self>>,
        state: BottomSheetGestureState,
        now: Instant,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(tracking) = this.borrow_mut().tracking.take() else {
            return;
        };
        if !tracking.recognized {
            return;
        }
        Self::handle_sheet_pan(
            this,
            state,
            tracking.location,
            tracking.translation,
            tracking.velocity(),
            now,
            window,
            cx,
        );
    }

    fn tracks(this: &Rc<RefCell<Self>>, source: TrackingSource) -> bool {
        this.borrow().tracking.as_ref().is_some_and(|tracking| tracking.source == source)
    }

    fn on_mouse_down(this: &Rc<RefCell<Self>>, event: &MouseDownEvent, now: Instant) {
        if event.button != MouseButton::Left {
            return;
        }
        this.borrow_mut().tracking = Some(Tracking::new(TrackingSource::Mouse, event.position, now));
    }

    fn on_mouse_move(
        this: &Rc<RefCell<Self>>,
        event: &MouseMoveEvent,
        now: Instant,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !Self::tracks(this, TrackingSource::Mouse) {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            Self::end_tracking(this, BottomSheetGestureState::Cancelled, now, window, cx);
            return;
        }
        {
            let mut coordinator = this.borrow_mut();
            let Some(tracking) = coordinator.tracking.as_mut() else {
                return;
            };
            tracking.location = event.position;
            tracking.translation = event.position - tracking.origin;
        }
        Self::advance_tracking(this, now, window, cx);
    }

    fn on_mouse_up(
        this: &Rc<RefCell<Self>>,
        event: &MouseUpEvent,
        now: Instant,
        window: &mut Window,
        cx: &mut App,
    ) {
        if event.button != MouseButton::Left || !Self::tracks(this, TrackingSource::Mouse) {
            return;
        }
        Self::end_tracking(this, BottomSheetGestureState::Ended, now, window, cx);
    }

    /// true when the event moved the sheet and must not also scroll the list under it
    fn on_scroll_wheel(
        this: &Rc<RefCell<Self>>,
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
                let mut coordinator = this.borrow_mut();
                if hovered && coordinator.tracking.is_none() {
                    coordinator.tracking =
                        Some(Tracking::new(TrackingSource::Trackpad, event.position, now));
                }
                false
            }
            TouchPhase::Moved => {
                {
                    let mut coordinator = this.borrow_mut();
                    let Some(tracking) = coordinator.tracking.as_mut() else {
                        return false;
                    };
                    if tracking.source != TrackingSource::Trackpad {
                        return false;
                    }
                    tracking.translation += event.delta.pixel_delta(window.line_height());
                    tracking.location = event.position;
                }
                Self::advance_tracking(this, now, window, cx);
                !this.borrow().gesture_recognizer()
            }
            TouchPhase::Ended => {
                if !Self::tracks(this, TrackingSource::Trackpad) {
                    return false;
                }
                let consumed = !this.borrow().gesture_recognizer();
                Self::end_tracking(this, BottomSheetGestureState::Ended, now, window, cx);
                consumed
            }
            TouchPhase::Cancelled => {
                if Self::tracks(this, TrackingSource::Trackpad) {
                    Self::end_tracking(this, BottomSheetGestureState::Cancelled, now, window, cx);
                }
                false
            }
        }
    }

    fn recognizer_overlay(this: &Rc<RefCell<Self>>) -> impl IntoElement {
        let this = this.clone();
        let host_view = this.borrow().host_view.clone();
        *host_view.coordinator.borrow_mut() = Rc::downgrade(&this);
        canvas(
            move |bounds, window, _| {
                host_view.layout_subviews(bounds);
                window.insert_hitbox(bounds, HitboxBehavior::Normal)
            },
            move |_, hitbox: Hitbox, window, _| {
                let down = this.clone();
                let down_hitbox = hitbox.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && down_hitbox.is_hovered(window) {
                        let now = cx.background_executor().now();
                        Self::on_mouse_down(&down, event, now);
                    }
                });
                let moved = this.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let now = cx.background_executor().now();
                        Self::on_mouse_move(&moved, event, now, window, cx);
                    }
                });
                let up = this.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture {
                        let now = cx.background_executor().now();
                        Self::on_mouse_up(&up, event, now, window, cx);
                    }
                });
                let wheel = this;
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let now = cx.background_executor().now();
                    let hovered = hitbox.is_hovered(window);
                    if Self::on_scroll_wheel(&wheel, event, hovered, now, window, cx) {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .inset_0()
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        let holder: Rc<dyn BottomSheetContentHeightHolding> = self.host_view.clone();
        self.parent.motion_controller.unregister_content_height_holder(&holder);
    }
}
