use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, IntoElement, Length, ParentElement, RenderOnce, Window, relative,
};
use rise_ui::animated_tabs_ui::animated_tabs_strip_motion::{AnimatedTabsStripKey, AnimatedTabsStripMotionDriver};
use rise_ui::animated_tabs_ui::animated_tabs_types::AnimatedTabsMetrics;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use smallvec::SmallVec;

use super::animated_tabs_pager_gesture_gate::{
    AnimatedTabsPagerAxisOwner, AnimatedTabsPagerAxisOwnersKey, AnimatedTabsPagerGestureGate,
    AnimatedTabsPagerPanCallbacks, AnimatedTabsPagerPanConfiguration, animated_tabs_pager_axis_owner,
};

/// drag state shared by the pager and the header indicator
/// only the indicator reads drag_progress, a view that builds the pages would rebuild all of them
pub struct AnimatedTabsPagerLiveState {
    drag_progress: f32,
    drag_base_index: Option<i64>,
    drag_release_animation: Option<MotionSpec>,
    suppresses_tab_taps: bool,
    mount_anchor: i64,
    mount_anchor_generation: u64,
    is_mount_anchor_established: bool,
    tap_restore_generation: u64,
}

impl Default for AnimatedTabsPagerLiveState {
    fn default() -> Self {
        Self::new()
    }
}

impl AnimatedTabsPagerLiveState {
    pub fn new() -> Self {
        Self {
            drag_progress: 0.0,
            drag_base_index: None,
            drag_release_animation: None,
            suppresses_tab_taps: false,
            mount_anchor: 0,
            mount_anchor_generation: 0,
            is_mount_anchor_established: false,
            tap_restore_generation: 0,
        }
    }

    pub fn drag_progress(&self) -> f32 {
        self.drag_progress
    }

    pub fn drag_base_index(&self) -> Option<i64> {
        self.drag_base_index
    }

    pub fn drag_release_animation(&self) -> Option<MotionSpec> {
        self.drag_release_animation
    }

    pub fn suppresses_tab_taps(&self) -> bool {
        self.suppresses_tab_taps
    }

    pub fn mount_anchor(&self) -> i64 {
        self.mount_anchor
    }

    fn set_drag_progress_animated(&mut self, value: f32, animation: Option<MotionSpec>, cx: &mut Context<Self>) {
        self.drag_release_animation = animation;
        if self.drag_progress != value {
            self.drag_progress = value;
            cx.notify();
        }
    }

    pub fn publish_external_drag_progress(&mut self, value: f32, base_index: i64, cx: &mut Context<Self>) {
        let mut changed = false;
        self.drag_release_animation = None;
        if self.drag_base_index != Some(base_index) {
            self.drag_base_index = Some(base_index);
            changed = true;
        }
        if self.drag_progress != value {
            self.drag_progress = value;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    pub fn set_mount_anchor(&mut self, index: i64, cx: &mut Context<Self>) {
        self.is_mount_anchor_established = true;
        if index == self.mount_anchor {
            return;
        }
        self.mount_anchor_generation = self.mount_anchor_generation.wrapping_add(1);

        if (index - self.mount_anchor).abs() > 1 {
            self.mount_anchor = index;
            cx.notify();
            return;
        }

        let generation = self.mount_anchor_generation;
        let delay = Duration::from_secs_f64(AnimatedTabsMetrics::RELEASE_DURATION);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |state, cx| {
                if state.mount_anchor_generation != generation {
                    return;
                }
                state.mount_anchor = index;
                cx.notify();
            });
        })
        .detach();
    }

    /// immediate, on pager appearance and gesture start where direction is unknown
    /// so both neighbours must exist already
    pub fn prepare_mount_anchor(&mut self, index: i64, cx: &mut Context<Self>) {
        self.is_mount_anchor_established = true;
        if index == self.mount_anchor {
            return;
        }
        self.mount_anchor_generation = self.mount_anchor_generation.wrapping_add(1);
        self.mount_anchor = index;
        cx.notify();
    }

    /// anchor for a screen that hasnt shown the pager yet
    /// a screen opened on a restored tab must build that tab or its first frame is empty
    pub fn resolved_mount_anchor(&self, active_tab: i64) -> i64 {
        if self.is_mount_anchor_established { self.mount_anchor } else { active_tab }
    }

    /// restore waits so the click that ends a drag lands while taps are still suppressed
    /// the generation stops a late timer of the previous swipe from clearing the new one
    pub fn set_tab_taps_suppressed(&mut self, suppressed: bool, restore_delay: f64, cx: &mut Context<Self>) {
        self.tap_restore_generation = self.tap_restore_generation.wrapping_add(1);

        if suppressed {
            self.suppresses_tab_taps = true;
            return;
        }

        let generation = self.tap_restore_generation;
        let delay = Duration::from_secs_f64(restore_delay.max(0.0));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |state, _| {
                if state.tap_restore_generation == generation {
                    state.suppresses_tab_taps = false;
                }
            });
        })
        .detach();
    }
}

pub type AnimatedTabsPagerAction = Rc<dyn Fn(&mut Window, &mut App)>;

const BACK_BEYOND_FIRST_SETTLE_RESPONSE: f64 = 0.32;
const BACK_BEYOND_FIRST_SETTLE_DAMPING_FRACTION: f64 = 0.9;
pub type AnimatedTabsPagerSuppressedAction = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// rightward drag on the first page that the screen follows itself and not the pages
/// pages stay put and drag is reported live in page widths
/// the feed camera slides in from the left edge this way
#[derive(Clone)]
pub struct AnimatedTabsEdgeDrag {
    pub on_changed: Rc<dyn Fn(f32, &mut Window, &mut App)>,
    pub on_ended: Rc<dyn Fn(f32, f32, &mut Window, &mut App)>,
    pub on_cancelled: Rc<dyn Fn(&mut Window, &mut App)>,
}

/// who owns the current gesture, a plain reference and not view state
/// its written on the first drag frame and a state write there would rebuild pages at touch time
#[derive(Default)]
struct AnimatedTabsEdgeGestureOwner {
    owns_gesture: Cell<Option<bool>>,
}

/// own state of the pager plus the strip motion driver
pub struct AnimatedTabsPagerState {
    did_emit_siblings_tap_suppressed: bool,
    edge_gesture: Rc<AnimatedTabsEdgeGestureOwner>,
    motion: Entity<AnimatedTabsStripMotionDriver>,
    page_width: f32,
}

impl AnimatedTabsPagerState {
    fn new(cx: &mut App) -> Self {
        Self {
            did_emit_siblings_tap_suppressed: false,
            edge_gesture: Rc::default(),
            motion: cx.new(|_| AnimatedTabsStripMotionDriver::new()),
            page_width: 0.0,
        }
    }
}

/// tab index as the strip driver counts pages, negative index is the first page
fn strip_index(index: i64) -> usize {
    usize::try_from(index).unwrap_or(0)
}

#[derive(IntoElement)]
pub struct AnimatedTabsPager {
    id: ElementId,
    tabs_count: i64,
    active_tab: Binding<i64>,
    drag_progress: Binding<f32>,
    page_width: Binding<f32>,
    live_drag_state: Option<Entity<AnimatedTabsPagerLiveState>>,
    on_swipe_back_beyond_first: Option<AnimatedTabsPagerAction>,
    yields_first_tab_back_swipe_to_navigation: bool,
    first_page_edge_drag: Option<AnimatedTabsEdgeDrag>,
    sizes_vertically_to_content: bool,
    drag_locked: bool,
    on_siblings_tap_suppressed: Option<AnimatedTabsPagerSuppressedAction>,
    content: SmallVec<[AnyElement; 4]>,
}

impl AnimatedTabsPager {
    pub fn new(
        id: impl Into<ElementId>,
        tabs_count: i64,
        active_tab: Binding<i64>,
        drag_progress: Binding<f32>,
        page_width: Binding<f32>,
    ) -> Self {
        Self {
            id: id.into(),
            tabs_count,
            active_tab,
            drag_progress,
            page_width,
            live_drag_state: None,
            on_swipe_back_beyond_first: None,
            yields_first_tab_back_swipe_to_navigation: false,
            first_page_edge_drag: None,
            sizes_vertically_to_content: false,
            drag_locked: false,
            on_siblings_tap_suppressed: None,
            content: SmallVec::new(),
        }
    }

    pub fn live_drag_state(mut self, live_drag_state: Option<Entity<AnimatedTabsPagerLiveState>>) -> Self {
        self.live_drag_state = live_drag_state;
        self
    }

    pub fn on_swipe_back_beyond_first(mut self, action: Option<AnimatedTabsPagerAction>) -> Self {
        self.on_swipe_back_beyond_first = action;
        self
    }

    pub fn yields_first_tab_back_swipe_to_navigation(mut self, yields: bool) -> Self {
        self.yields_first_tab_back_swipe_to_navigation = yields;
        self
    }

    pub fn first_page_edge_drag(mut self, edge_drag: Option<AnimatedTabsEdgeDrag>) -> Self {
        self.first_page_edge_drag = edge_drag;
        self
    }

    pub fn sizes_vertically_to_content(mut self, sizes: bool) -> Self {
        self.sizes_vertically_to_content = sizes;
        self
    }

    pub fn drag_locked(mut self, locked: bool) -> Self {
        self.drag_locked = locked;
        self
    }

    pub fn on_siblings_tap_suppressed(mut self, action: Option<AnimatedTabsPagerSuppressedAction>) -> Self {
        self.on_siblings_tap_suppressed = action;
        self
    }
}

impl ParentElement for AnimatedTabsPager {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.content.extend(elements);
    }
}

#[derive(Clone)]
struct AnimatedTabsPagerGesture {
    state: Entity<AnimatedTabsPagerState>,
    tabs_count: i64,
    active_tab: Binding<i64>,
    drag_progress: Binding<f32>,
    live_drag_state: Option<Entity<AnimatedTabsPagerLiveState>>,
    on_swipe_back_beyond_first: Option<AnimatedTabsPagerAction>,
    yields_first_tab_back_swipe_to_navigation: bool,
    first_page_edge_drag: Option<AnimatedTabsEdgeDrag>,
    drag_locked: bool,
    on_siblings_tap_suppressed: Option<AnimatedTabsPagerSuppressedAction>,
}

impl AnimatedTabsPagerGesture {
    fn edge_gesture(&self, cx: &App) -> Rc<AnimatedTabsEdgeGestureOwner> {
        self.state.read(cx).edge_gesture.clone()
    }

    fn page_width(&self, cx: &App) -> f32 {
        self.state.read(cx).page_width
    }

    fn motion(&self, cx: &App) -> Entity<AnimatedTabsStripMotionDriver> {
        self.state.read(cx).motion.clone()
    }

    fn commit_strip(&self, index: i64, cx: &mut App) {
        self.motion(cx).update(cx, |motion, cx| motion.commit(strip_index(index), true, cx));
    }

    fn note_siblings_tap_suppressed(&self, window: &mut Window, cx: &mut App) {
        self.edge_gesture(cx).owns_gesture.set(None);

        let active_tab = self.active_tab.get(cx);
        if let Some(live) = &self.live_drag_state {
            live.update(cx, |live, cx| live.prepare_mount_anchor(active_tab, cx));
        }

        self.motion(cx).update(cx, |motion, cx| motion.begin_interactive_drag(cx));
        let emit = self.state.update(cx, |state, _| {
            let emit = !state.did_emit_siblings_tap_suppressed;
            state.did_emit_siblings_tap_suppressed = true;
            emit
        });
        if emit && let Some(action) = &self.on_siblings_tap_suppressed {
            action(true, window, cx);
        }
    }

    fn restore_siblings_tap(&self, window: &mut Window, cx: &mut App) {
        let emit = self.state.update(cx, |state, _| {
            let emit = state.did_emit_siblings_tap_suppressed;
            state.did_emit_siblings_tap_suppressed = false;
            emit
        });
        if emit && let Some(action) = &self.on_siblings_tap_suppressed {
            action(false, window, cx);
        }
    }

    /// pointer travel plus the leftover of a settle the same pointer interrupted
    /// signs are opposite, travel grows toward the previous page and the leftover toward the next
    fn raw_drag_offset(&self, translation_x: f32, page_width: f32, cx: &App) -> f32 {
        translation_x / page_width - self.motion(cx).read(cx).interruption_progress()
    }

    fn handle_change(&self, translation_x: f32, window: &mut Window, cx: &mut App) {
        let page_width = self.page_width(cx);
        if page_width <= 1.0 || self.drag_locked {
            return;
        }

        let raw = self.raw_drag_offset(translation_x, page_width, cx);
        let active_tab = self.active_tab.get(cx);

        if let Some(edge) = &self.first_page_edge_drag {
            let owner = self.edge_gesture(cx);
            if owner.owns_gesture.get().is_none() && raw != 0.0 {
                owner.owns_gesture.set(Some(active_tab == 0 && raw > 0.0));
            }
            if owner.owns_gesture.get() == Some(true) {
                (edge.on_changed)(raw.max(0.0), window, cx);
                return;
            }
        }

        let clamped = if active_tab == 0 && raw > 0.0 {
            if self.consumes_first_tab_back_swipe() { raw.min(1.0) } else { 0.0 }
        } else if active_tab == self.tabs_count - 1 && raw < 0.0 {
            (raw * AnimatedTabsMetrics::EDGE_RESISTANCE).max(-AnimatedTabsMetrics::EDGE_MAXIMUM_OVERSHOOT)
        } else {
            raw.clamp(-1.0, 1.0)
        };

        self.motion(cx).update(cx, |motion, cx| motion.set_drag_progress(-clamped, cx));
        self.publish_progress(-clamped, window, cx);
    }

    fn handle_end(&self, translation_x: f32, velocity_x: f32, window: &mut Window, cx: &mut App) {
        self.restore_siblings_tap(window, cx);

        let page_width = self.page_width(cx);
        let owner = self.edge_gesture(cx);
        if page_width <= 1.0 || self.drag_locked {
            if owner.owns_gesture.get() == Some(true)
                && let Some(edge) = &self.first_page_edge_drag
            {
                (edge.on_cancelled)(window, cx);
            }
            owner.owns_gesture.set(None);
            self.motion(cx).update(cx, |motion, cx| motion.clear_interaction_residue(cx));
            return;
        }

        let raw = self.raw_drag_offset(translation_x, page_width, cx);
        let active_tab = self.active_tab.get(cx);

        if owner.owns_gesture.get() == Some(true)
            && let Some(edge) = &self.first_page_edge_drag
        {
            owner.owns_gesture.set(None);
            self.motion(cx).update(cx, |motion, cx| motion.commit(strip_index(active_tab), false, cx));
            (edge.on_ended)(raw.max(0.0), velocity_x, window, cx);
            return;
        }
        owner.owns_gesture.set(None);

        if active_tab == 0
            && raw > 0.0
            && let Some(action) = &self.on_swipe_back_beyond_first
        {
            let should_pop = raw > AnimatedTabsMetrics::BACK_BEYOND_FIRST_THRESHOLD
                || velocity_x > AnimatedTabsMetrics::BACK_BEYOND_FIRST_VELOCITY;
            if should_pop {
                self.publish_progress_animated(
                    0.0,
                    Some(spring(BACK_BEYOND_FIRST_SETTLE_RESPONSE, BACK_BEYOND_FIRST_SETTLE_DAMPING_FRACTION)),
                    window,
                    cx,
                );
                self.commit_strip(active_tab, cx);
                action(window, cx);
                return;
            }
        }

        let abs_raw = raw.abs();
        let abs_vel = velocity_x.abs();
        let direction: i64 = if raw > 0.0 { -1 } else { 1 };

        let should_commit = abs_raw >= AnimatedTabsMetrics::PAGE_COMMIT_THRESHOLD
            || abs_vel >= AnimatedTabsMetrics::PAGE_VELOCITY_COMMIT_THRESHOLD;

        let mut new_tab = active_tab;
        if should_commit {
            let candidate = active_tab + direction;
            if candidate >= 0 && candidate < self.tabs_count {
                new_tab = candidate;
            }
        }

        self.active_tab.set(new_tab, window, cx);
        self.publish_progress(0.0, window, cx);
        let landed = self.active_tab.get(cx);
        self.commit_strip(landed, cx);
    }

    fn handle_cancel(&self, window: &mut Window, cx: &mut App) {
        self.restore_siblings_tap(window, cx);
        let owner = self.edge_gesture(cx);
        if owner.owns_gesture.get() == Some(true)
            && let Some(edge) = &self.first_page_edge_drag
        {
            (edge.on_cancelled)(window, cx);
        }
        owner.owns_gesture.set(None);
        self.publish_progress(0.0, window, cx);
        let active_tab = self.active_tab.get(cx);
        self.commit_strip(active_tab, cx);
    }

    fn consumes_first_tab_back_swipe(&self) -> bool {
        self.on_swipe_back_beyond_first.is_some()
            || self.first_page_edge_drag.is_some()
            || !self.yields_first_tab_back_swipe_to_navigation
    }

    /// progress for the header indicator only, never touches the page strip
    fn publish_progress(&self, value: f32, window: &mut Window, cx: &mut App) {
        self.publish_progress_animated(value, None, window, cx);
    }

    fn publish_progress_animated(&self, value: f32, animation: Option<MotionSpec>, window: &mut Window, cx: &mut App) {
        match &self.live_drag_state {
            Some(live) => live.update(cx, |live, cx| live.set_drag_progress_animated(value, animation, cx)),
            None => self.drag_progress.set(value, window, cx),
        }
    }

    fn callbacks(&self) -> AnimatedTabsPagerPanCallbacks {
        let began = self.clone();
        let changed = self.clone();
        let ended = self.clone();
        let cancelled = self.clone();
        AnimatedTabsPagerPanCallbacks {
            on_began: Rc::new(move |window, cx| began.note_siblings_tap_suppressed(window, cx)),
            on_changed: Rc::new(move |translation, window, cx| changed.handle_change(translation, window, cx)),
            on_ended: Rc::new(move |translation, velocity, window, cx| {
                ended.handle_end(translation, velocity, window, cx)
            }),
            on_cancelled: Rc::new(move |window, cx| cancelled.handle_cancel(window, cx)),
            on_touch_down: Rc::new(|_window, _cx| {}),
        }
    }
}

impl AnimatedTabsPager {
    fn is_off_screen(slot: i64, strip_offset: f32, page_width: f32) -> bool {
        let left = slot as f32 * page_width + strip_offset;
        left >= page_width - 0.5 || left + page_width <= 0.5
    }
}

impl RenderOnce for AnimatedTabsPager {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let right_to_left = layout_direction(cx) == LayoutDirection::RightToLeft;
        let active_tab = self.active_tab.get(cx);
        let tabs_count = self.tabs_count;
        let sizes = self.sizes_vertically_to_content;

        let state = window.with_global_id(self.id.clone(), |_, window| {
            window.use_keyed_state("animated-tabs-pager-state", cx, |_, cx| AnimatedTabsPagerState::new(cx))
        });

        let gesture = AnimatedTabsPagerGesture {
            state: state.clone(),
            tabs_count,
            active_tab: self.active_tab.clone(),
            drag_progress: self.drag_progress.clone(),
            live_drag_state: self.live_drag_state.clone(),
            on_swipe_back_beyond_first: self.on_swipe_back_beyond_first.clone(),
            yields_first_tab_back_swipe_to_navigation: self.yields_first_tab_back_swipe_to_navigation,
            first_page_edge_drag: self.first_page_edge_drag.clone(),
            drag_locked: self.drag_locked,
            on_siblings_tap_suppressed: self.on_siblings_tap_suppressed.clone(),
        };
        let gate = AnimatedTabsPagerGestureGate::new(
            AnimatedTabsPagerPanConfiguration::new(
                self.drag_locked,
                active_tab,
                self.on_swipe_back_beyond_first.is_some() || self.first_page_edge_drag.is_some(),
                self.yields_first_tab_back_swipe_to_navigation,
                right_to_left,
            ),
            gesture.callbacks(),
        );
        let coordinator = window.with_global_id(self.id.clone(), |_, window| {
            window.use_keyed_state("animated-tabs-pager-gate", cx, |_, _| gate.make_coordinator())
        });

        let live_for_change = self.live_drag_state.clone();
        window.with_global_id(self.id.clone(), |_, window| {
            on_change("animated-tabs-pager-active-tab", active_tab, false, window, cx, move |_, new_value, _, cx| {
                if let Some(live) = live_for_change {
                    let new_value = *new_value;
                    live.update(cx, |live, cx| live.set_mount_anchor(new_value, cx));
                }
            });
        });

        let page_width = state.read(cx).page_width;
        let motion = state.read(cx).motion.clone();
        let count = tabs_count.max(1);
        let (strip_left, strip_offset): (Length, Option<f32>) = if page_width > 1.0 {
            let page_count = strip_index(count);
            let resting_index = strip_index(active_tab);
            motion.update(cx, |motion, cx| {
                if motion.is_bound() {
                    motion.sync(px(page_width), page_count, resting_index, right_to_left, cx);
                } else {
                    motion.bind(px(page_width), page_count, resting_index, right_to_left, cx);
                }
            });
            let offset = motion.read(cx).strip_offset_x(window);
            (offset.into(), Some(f32::from(offset)))
        } else {
            let resting = if right_to_left { count - 1 - active_tab } else { active_tab };
            (relative(-(resting as f32)).into(), None)
        };

        let pages = self.content.into_iter().enumerate().map(|(index, page)| {
            let index = index as i64;
            let slot = if right_to_left { count - 1 - index } else { index };
            let is_off_screen = strip_offset.is_some_and(|offset| Self::is_off_screen(slot, offset, page_width));
            div()
                .flex_none()
                .w(relative(1.0 / count as f32))
                .when(!sizes, |page_box| page_box.h_full())
                .when(is_off_screen, |page_box| page_box.invisible())
                .child(page)
        });

        let strip = div()
            .relative()
            .flex()
            .when(right_to_left, |strip| strip.flex_row_reverse())
            .when(!right_to_left, |strip| strip.flex_row())
            .items_start()
            .flex_none()
            .w(relative(count as f32))
            .left(strip_left)
            .when(!sizes, |strip| strip.h_full())
            .children(pages);
        let strip = env_scope::<AnimatedTabsStripKey>(Some(motion.downgrade())).child(strip);
        let axis_owners = coordinator.read(cx).axis_owners();
        axis_owners.clear();
        let strip = env_scope::<AnimatedTabsPagerAxisOwnersKey>(Some(axis_owners)).child(strip);

        let width_state = state.clone();
        let page_width_binding = self.page_width.clone();
        let live_for_appear = self.live_drag_state.clone();
        let host = div()
            .id(self.id.clone())
            .relative()
            .w_full()
            .when(!sizes, |host| host.h_full())
            .overflow_hidden()
            .child(strip)
            .child(measure("animated-tabs-pager-width", move |bounds, window, cx| {
                let width = f32::from(bounds.size.width);
                let first = width_state.update(cx, |state, cx| {
                    let first = state.page_width <= 1.0;
                    if state.page_width != width {
                        state.page_width = width;
                        cx.notify();
                    }
                    first
                });
                page_width_binding.set(width, window, cx);
                if first && let Some(live) = live_for_appear {
                    live.update(cx, |live, cx| live.prepare_mount_anchor(active_tab, cx));
                }
            }));

        let pager = gate.update_ui_view(host, &coordinator, window, cx);
        if tabs_count > 1 {
            animated_tabs_pager_axis_owner(AnimatedTabsPagerAxisOwner::HorizontalStrip, pager).into_any_element()
        } else {
            pager.into_any_element()
        }
    }
}
