use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Mutex;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, Div, DispatchPhase, Element, ElementId, Entity, EntityId,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, InteractiveElement, IntoElement, LayoutId,
    ParentElement, Pixels, Point, RenderOnce, ScrollDelta, ScrollHandle, ScrollWheelEvent, Size, Stateful,
    StatefulInteractiveElement, Styled, Subscription, Task, TouchPhase, WeakEntity, Window, canvas, div, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::display_cadence::maximum_frames_per_second;
use rise_ui::animated_tabs_ui::animated_tabs_types::AnimatedTabsMetrics;
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::rise_frame_rate::RiseFrameRateRange;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{CubicBezier, ease_in_out};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription};
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementReturnScrollMath, SharedElementReturnScrollMathBand, SharedElementReturnScrollMathClamp,
    SharedElementReturnScrollMathPosition,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementReturnPlan, SharedElementReturnRequest, SharedElementReturnResolution,
    SharedElementReturnResumableHandler, SharedElementReturnScroller, SharedElementReturnViewport,
};
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::AnimatedTabsPagerLiveState;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager_gesture_gate::AnimatedTabsPagerAxisOwners;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::modules::user::components::profile_content::profile_content::ProfileContentTab;
use crate::modules::user::components::profile_content::profile_performance_trace::{
    ProfilePerformanceGestureDirection, ProfilePerformanceGesturePhase, ProfilePerformanceTrace,
    ProfilePerformanceVerticalGeometry,
};
use crate::modules::user::stores::profile_inner_content_events::PROFILE_INNER_CONTENT_WILL_REORDER;

pub struct ProfileScrollMetrics;

impl ProfileScrollMetrics {
    pub const HEADER_TOP_PADDING: f32 = 0.0;
    pub const PAGE_HEADER_HEIGHT: f32 = PageHeaderMetrics::HEIGHT;
    pub const PINNED_TABS_TOP: f32 = Self::HEADER_TOP_PADDING + Self::PAGE_HEADER_HEIGHT;
    pub const CHAT_PINNED_TABS_TOP: f32 = 52.0 + Self::PAGE_HEADER_HEIGHT;
    pub const TABS_BAR_HEIGHT: f32 = 68.0;
}

/// pinning and its scroll range must be invalidated together or a stale spacer meets the new
/// page height
pub struct ProfileMasterPinnedModifier {
    pub scroll_coordinator: Entity<ProfileScrollCoordinator>,
    pub pinned_tabs_top: f32,
}

impl ProfileMasterPinnedModifier {
    pub fn new(scroll_coordinator: Entity<ProfileScrollCoordinator>) -> Self {
        Self { scroll_coordinator, pinned_tabs_top: ProfileScrollMetrics::PINNED_TABS_TOP }
    }

    pub fn pinned_tabs_top(mut self, pinned_tabs_top: f32) -> Self {
        self.pinned_tabs_top = pinned_tabs_top;
        self
    }

    /// wraps content with the modifier, gpui paints this after the profile top so no z index needed
    pub fn body(self, content: impl IntoElement, window: &mut Window, cx: &mut App) -> AnyElement {
        observe_active_proxy_height(&self.scroll_coordinator, window, cx);
        let (uses_native_engine, proxy_height) = {
            let coordinator = self.scroll_coordinator.read(cx);
            let uses_native_engine = coordinator.uses_native_vertical_engine();
            (uses_native_engine, if uses_native_engine { coordinator.active_proxy_height() } else { 0.0 })
        };
        let pinned = ProfileMasterPinnedContent {
            scroll_coordinator: self.scroll_coordinator.clone(),
            proxy_height,
            uses_native_engine,
            pinned_tabs_top: self.pinned_tabs_top,
            content: content.into_any_element(),
        };
        let mut stack = div().flex().flex_col().w_full().child(pinned);
        if uses_native_engine {
            let coordinator = self.scroll_coordinator;
            stack = stack.child(
                canvas(
                    move |bounds: Bounds<Pixels>, _, cx: &mut App| {
                        coordinator.update(cx, |coordinator, cx| {
                            coordinator.proxy_height_did_layout(f32::from(bounds.size.height), cx);
                        });
                    },
                    |_, _, _, _| {},
                )
                .w_full()
                .h(px(proxy_height)),
            );
        }
        stack.into_any_element()
    }
}

/// keeps the view that drew the spacer subscribed to the proxy height
struct ProfileMasterPinnedObservation {
    _subscription: Subscription,
}

fn observe_active_proxy_height(coordinator: &Entity<ProfileScrollCoordinator>, window: &mut Window, cx: &mut App) {
    let view = window.current_view();
    let key = ("profile_master_pinned_observation", coordinator.entity_id().as_u64());
    let coordinator = coordinator.clone();
    window.use_keyed_state(key, cx, move |_, cx: &mut Context<ProfileMasterPinnedObservation>| {
        ProfileMasterPinnedObservation {
            _subscription: cx.observe(&coordinator, move |_, _, cx| App::notify(cx, view)),
        }
    });
}

struct ProfileMasterPinnedContent {
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    proxy_height: f32,
    uses_native_engine: bool,
    pinned_tabs_top: f32,
    content: AnyElement,
}

impl IntoElement for ProfileMasterPinnedContent {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ProfileMasterPinnedContent {
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
        (self.content.request_layout(window, cx), ())
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
        let viewport_top = self.scroll_coordinator.read(cx).outer_viewport_top().unwrap_or(f32::from(bounds.top()));
        let correction = ProfileMasterPinningPolicy::traced_correction(
            f32::from(bounds.top()) - viewport_top,
            self.proxy_height,
            self.uses_native_engine,
            self.pinned_tabs_top,
        );
        window.with_element_offset(point(px(0.0), px(correction)), |window| self.content.prepaint(window, cx));
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
        self.content.paint(window, cx);
    }
}

pub struct ProfileMasterPinningPolicy;

impl ProfileMasterPinningPolicy {
    pub fn correction(content_min_y: f32, proxy_height: f32, uses_native_engine: bool, pinned_tabs_top: f32) -> f32 {
        if !uses_native_engine {
            return 0.0;
        }
        (pinned_tabs_top - content_min_y).max(0.0).min(proxy_height.max(0.0))
    }

    pub fn traced_correction(
        content_min_y: f32,
        proxy_height: f32,
        uses_native_engine: bool,
        pinned_tabs_top: f32,
    ) -> f32 {
        let value = Self::correction(content_min_y, proxy_height, uses_native_engine, pinned_tabs_top);
        ProfileMasterPinningProbe::report(ProfileMasterPinningProbeSample {
            min_y: content_min_y,
            proxy_height,
            pinned_tabs_top,
            correction: value,
            sequence: 0,
        });
        value
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct ProfileMasterPinningProbeSample {
    pub min_y: f32,
    pub proxy_height: f32,
    pub pinned_tabs_top: f32,
    pub correction: f32,
    pub sequence: u64,
}

/// latest pinning snapshot from the effect
pub struct ProfileMasterPinningProbe;

static PROFILE_MASTER_PINNING_LOCK: Mutex<ProfileMasterPinningProbeSample> =
    Mutex::new(ProfileMasterPinningProbeSample {
        min_y: 0.0,
        proxy_height: 0.0,
        pinned_tabs_top: 0.0,
        correction: 0.0,
        sequence: 0,
    });

impl ProfileMasterPinningProbe {
    pub fn report(sample: ProfileMasterPinningProbeSample) {
        let mut current = PROFILE_MASTER_PINNING_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut next = sample;
        next.sequence = current.sequence.wrapping_add(1);
        *current = next;
    }

    pub fn latest() -> ProfileMasterPinningProbeSample {
        *PROFILE_MASTER_PINNING_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct ProfileContentLayoutPolicy;

impl ProfileContentLayoutPolicy {
    pub fn window(
        visible_tabs: &[ProfileContentTab],
        selected_tab: ProfileContentTab,
        radius: usize,
    ) -> HashSet<ProfileContentTab> {
        let Some(selected_index) = visible_tabs.iter().position(|tab| *tab == selected_tab) else {
            return HashSet::from([selected_tab]);
        };
        let lower_bound = selected_index.saturating_sub(radius);
        let upper_bound = (visible_tabs.len() - 1).min(selected_index + radius);
        visible_tabs[lower_bound..=upper_bound].iter().copied().collect()
    }

    pub fn rendered_tabs(visible_tabs: &[ProfileContentTab], selected_tab: ProfileContentTab) -> HashSet<ProfileContentTab> {
        Self::window(visible_tabs, selected_tab, 1)
    }

    /// mounted tabs in a transition, mounted ones plus the new tab window
    /// only grows, warmed_tabs shrinks it later
    /// else a second swipe opens an empty page
    pub fn grown_tabs_during_transition(
        visible_tabs: &[ProfileContentTab],
        selected_tab: ProfileContentTab,
        mounted_tabs: &HashSet<ProfileContentTab>,
    ) -> HashSet<ProfileContentTab> {
        let reachable = Self::window(visible_tabs, selected_tab, 2);
        let mut tabs: HashSet<ProfileContentTab> = mounted_tabs.intersection(&reachable).copied().collect();
        tabs.extend(Self::window(visible_tabs, selected_tab, 1));
        tabs
    }

    pub fn prepared_horizontal_directions(
        visible_tabs: &[ProfileContentTab],
        selected_tab: ProfileContentTab,
        mounted_tabs: &HashSet<ProfileContentTab>,
    ) -> HashSet<i64> {
        let Some(selected_index) = visible_tabs.iter().position(|tab| *tab == selected_tab) else {
            return HashSet::new();
        };
        let mut directions = HashSet::new();
        for direction in [-1_i64, 1] {
            let index = selected_index as i64 + direction;
            if index >= 0 && (index as usize) < visible_tabs.len() && mounted_tabs.contains(&visible_tabs[index as usize]) {
                directions.insert(direction);
            }
        }
        directions
    }

    pub fn retained_tabs_during_transition(
        visible_tabs: &[ProfileContentTab],
        selected_tab: ProfileContentTab,
        mounted_tabs: &HashSet<ProfileContentTab>,
    ) -> HashSet<ProfileContentTab> {
        let mut tabs: HashSet<ProfileContentTab> =
            visible_tabs.iter().copied().filter(|tab| mounted_tabs.contains(tab)).collect();
        tabs.insert(selected_tab);
        tabs
    }

    pub fn warmed_tabs(
        visible_tabs: &[ProfileContentTab],
        selected_tab: ProfileContentTab,
        _mounted_tabs: &HashSet<ProfileContentTab>,
    ) -> HashSet<ProfileContentTab> {
        Self::rendered_tabs(visible_tabs, selected_tab)
    }
}

pub struct ProfileGeometryRefreshPolicy;

impl ProfileGeometryRefreshPolicy {
    pub const STABLE_IDLE_DELAY: f64 = 0.12;
    pub const CONTENT_SIZE_EPSILON: f32 = 0.5;

    pub fn should_defer(
        vertical_motion_active: bool,
        horizontal_gesture_active: bool,
        horizontal_settlement_active: bool,
    ) -> bool {
        vertical_motion_active || horizontal_gesture_active || horizontal_settlement_active
    }

    pub fn accepts_flush(callback_generation: u64, current_generation: u64) -> bool {
        callback_generation == current_generation
    }

    pub fn content_size_changed(from: Size<f32>, to: Size<f32>, epsilon: f32) -> bool {
        (from.width - to.width).abs() > epsilon || (from.height - to.height).abs() > epsilon
    }
}

pub struct ProfileMotionFrameRatePolicy;

impl ProfileMotionFrameRatePolicy {
    pub fn frame_rate_range(maximum_frames_per_second: i64) -> RiseFrameRateRange {
        let maximum = maximum_frames_per_second.max(30) as f32;
        RiseFrameRateRange { minimum: maximum.min(30.0), maximum, preferred: maximum }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ProfileVerticalMasterScrollPolicyPosition {
    pub collapse_progress: f32,
    pub inner_offset: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ProfileVerticalMasterScrollPolicyMapping {
    pub collapse_progress: f32,
    pub inner_offset: f32,
    pub pin_correction: f32,
}

pub struct ProfileVerticalMasterScrollPolicy;

impl ProfileVerticalMasterScrollPolicy {
    pub const LEGACY_LAUNCH_ARGUMENT: &'static str = "-riseProfileLegacyVerticalScroll";
    pub const EPSILON: f32 = 0.1;

    pub fn uses_native_master_engine(launch_arguments: &[String]) -> bool {
        !launch_arguments.iter().any(|argument| argument == Self::LEGACY_LAUNCH_ARGUMENT)
    }

    pub fn collapse_range(total_outer_range: f32, proxy_height: f32) -> f32 {
        (total_outer_range - proxy_height.max(0.0)).max(0.0)
    }

    pub fn proxy_layout_is_ready(requested_height: f32, laid_out_height: Option<f32>, outer_range: f32) -> bool {
        let Some(laid_out_height) = laid_out_height else {
            return false;
        };
        if !requested_height.is_finite() || !laid_out_height.is_finite() || !outer_range.is_finite() {
            return false;
        }
        (requested_height - laid_out_height).abs() <= 0.5 && outer_range + 0.5 >= laid_out_height
    }

    pub fn mapping(master_offset: f32, collapse_range: f32, inner_range: f32) -> ProfileVerticalMasterScrollPolicyMapping {
        let collapse = collapse_range.max(0.0);
        let inner = inner_range.max(0.0);
        let bounded_master = master_offset.max(0.0);
        let inner_offset = (bounded_master - collapse).max(0.0).min(inner);
        let collapse_progress = if collapse > Self::EPSILON { (bounded_master / collapse).min(1.0) } else { 0.0 };
        ProfileVerticalMasterScrollPolicyMapping { collapse_progress, inner_offset, pin_correction: inner_offset }
    }

    pub fn master_offset(
        position: ProfileVerticalMasterScrollPolicyPosition,
        collapse_range: f32,
        inner_range: f32,
    ) -> f32 {
        let collapse = collapse_range.max(0.0);
        let inner_offset = position.inner_offset.max(0.0).min(inner_range.max(0.0));
        if inner_offset > Self::EPSILON {
            return collapse + inner_offset;
        }
        collapse * position.collapse_progress.max(0.0).min(1.0)
    }

    /// header collapse is the profiles, each pane keeps its own feed depth
    /// partial header keeps its height on swipe
    /// once collapsed the pane restores its own offset
    pub fn restored_tab_position(
        source_position: ProfileVerticalMasterScrollPolicyPosition,
        remembered_target_position: Option<ProfileVerticalMasterScrollPolicyPosition>,
        target_inner_range: f32,
    ) -> ProfileVerticalMasterScrollPolicyPosition {
        let collapse_progress = source_position.collapse_progress.max(0.0).min(1.0);
        if collapse_progress < 1.0 - Self::EPSILON {
            return ProfileVerticalMasterScrollPolicyPosition { collapse_progress, inner_offset: 0.0 };
        }
        let remembered = remembered_target_position.map_or(0.0, |position| position.inner_offset);
        ProfileVerticalMasterScrollPolicyPosition {
            collapse_progress: 1.0,
            inner_offset: remembered.max(0.0).min(target_inner_range.max(0.0)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProfileHorizontalPagerPolicyTabTransition {
    pub source_index: i64,
    pub target_index: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileHorizontalPagerPolicySelectionDisposition {
    Unchanged,
    DeferUntilSettle,
    ReplaceImmediately,
}

pub struct ProfileHorizontalPagerPolicy;

impl ProfileHorizontalPagerPolicy {
    pub const DISTANCE_COMMIT_THRESHOLD: f32 = AnimatedTabsMetrics::PAGE_COMMIT_THRESHOLD;
    pub const VELOCITY_COMMIT_THRESHOLD: f32 = AnimatedTabsMetrics::PAGE_VELOCITY_COMMIT_THRESHOLD;
    pub const SETTLE_DURATION: f64 = AnimatedTabsMetrics::RELEASE_DURATION;
    pub const INITIAL_PAGE_PRELOAD_DELAY: f64 = 0.0;
    pub const PAGE_PRELOAD_BINDING_RETRY_DELAY: f64 = 1.0 / 60.0;
    pub const PAGE_PRELOAD_IDLE_RETRY_DELAY: f64 = 0.10;
    pub const POST_SETTLE_MOUNT_QUIET_DELAY: f64 = 0.22;
    pub const PAGE_PRELOAD_DELAY: f64 = Self::SETTLE_DURATION + Self::POST_SETTLE_MOUNT_QUIET_DELAY;

    /// side comes from the offset on screen, not from velocity
    /// velocity flipped on sharp reversals
    /// velocity decides whether to commit
    pub fn direction(progress: f32, velocity_x: f32, _page_width: f32) -> i64 {
        if progress.abs() > 0.0005 {
            return if progress > 0.0 { 1 } else { -1 };
        }
        if velocity_x < 0.0 { 1 } else { -1 }
    }

    pub fn should_commit(progress: f32, velocity_x: f32) -> bool {
        progress.abs() >= Self::DISTANCE_COMMIT_THRESHOLD || velocity_x.abs() >= Self::VELOCITY_COMMIT_THRESHOLD
    }

    pub fn continuity_progress(progress: f32, direction: i64) -> f32 {
        progress - direction as f32
    }

    pub fn target_index(current_index: i64, direction: i64, page_count: i64) -> Option<i64> {
        if direction.abs() != 1 {
            return None;
        }
        let target = current_index + direction;
        (0..page_count).contains(&target).then_some(target)
    }

    pub fn tab_transition<Tab: PartialEq>(
        current_tab: &Tab,
        direction: i64,
        visible_tabs: &[Tab],
    ) -> Option<ProfileHorizontalPagerPolicyTabTransition> {
        let source_index = visible_tabs.iter().position(|tab| tab == current_tab)? as i64;
        let target_index = Self::target_index(source_index, direction, visible_tabs.len() as i64)?;
        Some(ProfileHorizontalPagerPolicyTabTransition { source_index, target_index })
    }

    pub fn can_commit(direction: i64, prepared_directions: &HashSet<i64>) -> bool {
        direction.abs() == 1 && prepared_directions.contains(&direction)
    }

    pub fn interactive_progress(start_progress: f32, translation_x: f32, page_width: f32) -> f32 {
        if page_width <= 1.0 {
            return start_progress;
        }
        (start_progress - translation_x / page_width).max(-1.0).min(1.0)
    }

    /// clamps drag progress
    /// resisted_directions are sides with no page, dragging there resists
    /// unmounted neighbour still doesnt open
    pub fn constrained_interactive_progress(
        proposed_progress: f32,
        interruption_progress: f32,
        available_directions: &HashSet<i64>,
        resisted_directions: &HashSet<i64>,
    ) -> f32 {
        let proposed_progress = proposed_progress.max(-1.0).min(1.0);
        if proposed_progress.abs() <= 0.0005 {
            return 0.0;
        }

        let direction: i64 = if proposed_progress > 0.0 { 1 } else { -1 };
        if available_directions.contains(&direction) {
            return proposed_progress;
        }

        if resisted_directions.contains(&direction) {
            return direction as f32
                * (proposed_progress.abs() * AnimatedTabsMetrics::EDGE_RESISTANCE)
                    .min(AnimatedTabsMetrics::EDGE_MAXIMUM_OVERSHOOT);
        }

        let interruption_direction: i64 = if interruption_progress > 0.0 { 1 } else { -1 };
        if interruption_progress.abs() <= 0.0005 || direction != interruption_direction {
            return 0.0;
        }

        direction as f32 * proposed_progress.abs().min(interruption_progress.abs())
    }

    pub fn moves_toward_rest(progress: f32, direction: i64) -> bool {
        if progress.abs() <= 0.0005 || direction.abs() != 1 {
            return false;
        }
        (progress > 0.0 && direction == -1) || (progress < 0.0 && direction == 1)
    }

    pub fn settlement_progress(start_progress: f32, elapsed_fraction: f32) -> f32 {
        let linear = elapsed_fraction.max(0.0).min(1.0);
        let remaining = 1.0 - linear;
        let eased = 1.0 - remaining * remaining * remaining;
        start_progress * (1.0 - eased)
    }

    pub fn settlement_position(start_index: f32, target_index: f32, elapsed_fraction: f32) -> f32 {
        let linear = elapsed_fraction.max(0.0).min(1.0);
        let remaining = 1.0 - linear;
        let eased = 1.0 - remaining * remaining * remaining;
        start_index + (target_index - start_index) * eased
    }

    pub fn absolute_index(offset_x: f32, minimum_x: f32, page_width: f32, page_count: i64) -> f32 {
        let index = (offset_x - minimum_x) / page_width.max(1.0);
        index.max(0.0).min((page_count - 1).max(0) as f32)
    }

    pub fn offset_x(absolute_index: f32, minimum_x: f32, page_width: f32, page_count: i64) -> f32 {
        let bounded_index = absolute_index.max(0.0).min((page_count - 1).max(0) as f32);
        minimum_x + bounded_index * page_width.max(1.0)
    }

    pub fn motion_translation_x(visual_absolute_index: f32, resting_absolute_index: f32, page_width: f32) -> f32 {
        -(visual_absolute_index - resting_absolute_index) * page_width.max(1.0)
    }

    pub fn visual_absolute_index(
        resting_absolute_index: f32,
        motion_translation_x: f32,
        page_width: f32,
        page_count: i64,
    ) -> f32 {
        let index = resting_absolute_index - motion_translation_x / page_width.max(1.0);
        let overshoot = AnimatedTabsMetrics::EDGE_MAXIMUM_OVERSHOOT;
        index.max(-overshoot).min((page_count - 1).max(0) as f32 + overshoot)
    }

    /// tells a committed swipe echo from a really new tab choice
    /// the new one must cancel even on the still active page
    #[allow(clippy::too_many_arguments)]
    pub fn selection_disposition(
        active_tab: ProfileContentTab,
        pending_tab: Option<ProfileContentTab>,
        requested_tab: ProfileContentTab,
        requested_index: Option<i64>,
        is_committing: bool,
        settlement_active: bool,
        settlement_target_index: Option<i64>,
    ) -> ProfileHorizontalPagerPolicySelectionDisposition {
        Self::semantic_selection_disposition(
            active_tab,
            pending_tab,
            requested_tab,
            requested_index,
            is_committing,
            settlement_active,
            settlement_target_index,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn semantic_selection_disposition<Tab: PartialEq>(
        active_tab: Tab,
        pending_tab: Option<Tab>,
        requested_tab: Tab,
        requested_index: Option<i64>,
        is_committing: bool,
        settlement_active: bool,
        settlement_target_index: Option<i64>,
    ) -> ProfileHorizontalPagerPolicySelectionDisposition {
        let echoes_commit_target = pending_tab.is_some()
            && requested_index.is_some()
            && requested_index == settlement_target_index
            && (is_committing || settlement_active);
        if echoes_commit_target {
            return ProfileHorizontalPagerPolicySelectionDisposition::DeferUntilSettle;
        }
        if pending_tab.is_some() || settlement_active || requested_tab != active_tab {
            return ProfileHorizontalPagerPolicySelectionDisposition::ReplaceImmediately;
        }
        ProfileHorizontalPagerPolicySelectionDisposition::Unchanged
    }

    pub fn accepts_settlement_callback(callback_generation: u64, current_generation: u64) -> bool {
        callback_generation == current_generation
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfilePagerAxisLockPolicyAxis {
    Horizontal,
    Vertical,
    Undecided,
}

pub struct ProfilePagerAxisLockPolicy;

impl ProfilePagerAxisLockPolicy {
    pub const MINIMUM_INTENT_DISTANCE: f32 = 1.0;
    pub const DOMINANCE_RATIO: f32 = 1.2;
    pub const EARLY_DECISION_DISTANCE: f32 = 2.0;
    pub const EARLY_DOMINANCE_RATIO: f32 = 2.0;
    pub const FORCED_DECISION_DISTANCE: f32 = 10.0;

    pub fn axis(translation: Point<f32>, velocity: Point<f32>) -> ProfilePagerAxisLockPolicyAxis {
        Self::axis_with(translation, velocity, Self::MINIMUM_INTENT_DISTANCE, Self::DOMINANCE_RATIO)
    }

    pub fn axis_with(
        translation: Point<f32>,
        velocity: Point<f32>,
        minimum_intent_distance: f32,
        dominance_ratio: f32,
    ) -> ProfilePagerAxisLockPolicyAxis {
        let translation_has_intent = translation.x.abs().max(translation.y.abs()) >= minimum_intent_distance;
        let vector = if translation_has_intent { translation } else { velocity };
        if vector.x.abs() > vector.y.abs() * dominance_ratio {
            return ProfilePagerAxisLockPolicyAxis::Horizontal;
        }
        if vector.y.abs() > vector.x.abs() * dominance_ratio {
            return ProfilePagerAxisLockPolicyAxis::Vertical;
        }
        ProfilePagerAxisLockPolicyAxis::Undecided
    }

    /// axis from the early gesture,
    /// clearly vertical is rejected after about two points,
    /// diagonals get ten before the dominant axis wins
    pub fn early_axis(translation: Point<f32>) -> ProfilePagerAxisLockPolicyAxis {
        let horizontal = translation.x.abs();
        let vertical = translation.y.abs();

        if vertical >= Self::EARLY_DECISION_DISTANCE && vertical >= horizontal * Self::EARLY_DOMINANCE_RATIO {
            return ProfilePagerAxisLockPolicyAxis::Vertical;
        }
        if horizontal >= Self::EARLY_DECISION_DISTANCE && horizontal >= vertical * Self::EARLY_DOMINANCE_RATIO {
            return ProfilePagerAxisLockPolicyAxis::Horizontal;
        }
        if horizontal.max(vertical) < Self::FORCED_DECISION_DISTANCE {
            return ProfilePagerAxisLockPolicyAxis::Undecided;
        }
        if horizontal > vertical {
            ProfilePagerAxisLockPolicyAxis::Horizontal
        } else {
            ProfilePagerAxisLockPolicyAxis::Vertical
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileScrollToTopOwnershipPolicyRole {
    Outer,
    Inner,
}

impl ProfileScrollToTopOwnershipPolicyRole {
    pub const ALL_CASES: [Self; 2] = [Self::Outer, Self::Inner];
}

pub struct ProfileScrollToTopOwnershipPolicy;

impl ProfileScrollToTopOwnershipPolicy {
    /// outer profile scroll alone owns scroll to top, nested page scrolls stay opted out
    pub fn is_owner(role: ProfileScrollToTopOwnershipPolicyRole) -> bool {
        role == ProfileScrollToTopOwnershipPolicyRole::Outer
    }
}

pub const PROFILE_DECELERATION_RATE_NORMAL: f32 = 0.998;
pub const PROFILE_DECELERATION_RATE_FAST: f32 = 0.99;

const SET_CONTENT_OFFSET_ANIMATION_SECONDS: f64 = 0.3;

struct ProfileScrollViewInner {
    handle: ScrollHandle,
    window: Cell<Option<AnyWindowHandle>>,
    view: Cell<Option<EntityId>>,
    bounds: Cell<Bounds<Pixels>>,
    content_size: Cell<Size<f32>>,
    pan_enabled: Cell<bool>,
    bounces: Cell<bool>,
    scrolls_to_top: Cell<bool>,
    is_directional_lock_enabled: Cell<bool>,
    is_user_interaction_enabled: Cell<bool>,
    deceleration_rate: Cell<f32>,
    is_tracking: Cell<bool>,
    last_wheel: Cell<Option<Duration>>,
    momentum_suppressed: Cell<bool>,
    last_reported_offset_y: Cell<f32>,
    offset_animation: RefCell<Option<Animated<f32>>>,
    prepaint_offset_unlaid: Cell<bool>,
}

/// one profile scroll view, cloning shares it. pan_enabled decides if the element takes the wheel
#[derive(Clone)]
pub struct ProfileScrollView {
    inner: Rc<ProfileScrollViewInner>,
}

impl PartialEq for ProfileScrollView {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

#[derive(Clone)]
pub struct WeakProfileScrollView {
    inner: Weak<ProfileScrollViewInner>,
}

impl WeakProfileScrollView {
    pub fn upgrade(&self) -> Option<ProfileScrollView> {
        self.inner.upgrade().map(|inner| ProfileScrollView { inner })
    }
}

impl Default for ProfileScrollView {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileScrollView {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(ProfileScrollViewInner {
                handle: ScrollHandle::new(),
                window: Cell::new(None),
                view: Cell::new(None),
                bounds: Cell::new(Bounds::default()),
                content_size: Cell::new(size(0.0, 0.0)),
                pan_enabled: Cell::new(true),
                bounces: Cell::new(true),
                scrolls_to_top: Cell::new(true),
                is_directional_lock_enabled: Cell::new(false),
                is_user_interaction_enabled: Cell::new(true),
                deceleration_rate: Cell::new(PROFILE_DECELERATION_RATE_NORMAL),
                is_tracking: Cell::new(false),
                last_wheel: Cell::new(None),
                momentum_suppressed: Cell::new(false),
                last_reported_offset_y: Cell::new(0.0),
                offset_animation: RefCell::new(None),
                prepaint_offset_unlaid: Cell::new(false),
            }),
        }
    }

    pub fn downgrade(&self) -> WeakProfileScrollView {
        WeakProfileScrollView { inner: Rc::downgrade(&self.inner) }
    }

    pub fn handle(&self) -> ScrollHandle {
        self.inner.handle.clone()
    }

    pub fn vertical_scroll_container(&self, id: impl Into<ElementId>) -> Stateful<Div> {
        let container = div().id(id).flex().flex_col().size_full().track_scroll(&self.inner.handle);
        if self.pan_enabled() {
            container.overflow_y_scroll().restrict_scroll_to_axis()
        } else {
            container.overflow_y_hidden()
        }
    }

    /// the page strip, never takes the wheel itself because the coordinator moves it
    pub fn horizontal_pager_container(&self, id: impl Into<ElementId>) -> Stateful<Div> {
        div().id(id).flex().flex_row().size_full().track_scroll(&self.inner.handle).overflow_x_hidden()
    }

    pub fn pan_enabled(&self) -> bool {
        self.inner.pan_enabled.get()
    }

    pub fn set_pan_enabled(&self, enabled: bool) {
        self.inner.pan_enabled.set(enabled);
    }

    pub fn bounces(&self) -> bool {
        self.inner.bounces.get()
    }

    pub fn set_bounces(&self, bounces: bool) {
        self.inner.bounces.set(bounces);
    }

    pub fn scrolls_to_top(&self) -> bool {
        self.inner.scrolls_to_top.get()
    }

    pub fn set_scrolls_to_top(&self, scrolls_to_top: bool) {
        self.inner.scrolls_to_top.set(scrolls_to_top);
    }

    pub fn is_directional_lock_enabled(&self) -> bool {
        self.inner.is_directional_lock_enabled.get()
    }

    pub fn set_is_directional_lock_enabled(&self, enabled: bool) {
        self.inner.is_directional_lock_enabled.set(enabled);
    }

    pub fn is_user_interaction_enabled(&self) -> bool {
        self.inner.is_user_interaction_enabled.get()
    }

    pub fn set_is_user_interaction_enabled(&self, enabled: bool) {
        self.inner.is_user_interaction_enabled.set(enabled);
    }

    pub fn deceleration_rate(&self) -> f32 {
        self.inner.deceleration_rate.get()
    }

    pub fn set_deceleration_rate(&self, rate: f32) {
        self.inner.deceleration_rate.set(rate);
    }

    pub fn is_in_window(&self) -> bool {
        self.inner.window.get().is_some()
    }

    /// frame in window coordinates
    pub fn frame(&self) -> Bounds<Pixels> {
        self.inner.bounds.get()
    }

    pub fn bounds_width(&self) -> f32 {
        f32::from(self.inner.bounds.get().size.width)
    }

    pub fn bounds_height(&self) -> f32 {
        f32::from(self.inner.bounds.get().size.height)
    }

    pub fn content_size(&self) -> Size<f32> {
        self.inner.content_size.get()
    }

    pub fn content_offset_x(&self) -> f32 {
        -f32::from(self.inner.handle.offset().x)
    }

    pub fn content_offset_y(&self) -> f32 {
        -f32::from(self.inner.handle.offset().y)
    }

    /// sets the content offset right away, no animation
    pub fn set_content_offset(&self, x: f32, y: f32, cx: &mut App) {
        self.inner.offset_animation.borrow_mut().take();
        self.inner.handle.set_offset(point(px(-x), px(-y)));
        self.request_redraw(cx);
    }

    pub fn animate_content_offset_y(&self, y: f32, cx: &mut App) {
        let now = RiseClock::monotonic();
        let mut motion = self
            .inner
            .offset_animation
            .borrow_mut()
            .take()
            .unwrap_or_else(|| Animated::new(self.content_offset_y()));
        motion.set(y, ease_in_out(SET_CONTENT_OFFSET_ANIMATION_SECONDS), now, false);
        *self.inner.offset_animation.borrow_mut() = Some(motion);
        self.request_redraw(cx);
    }

    fn advance_offset_animation(&self, window: &Window) -> bool {
        let now = RiseClock::monotonic();
        let frame = {
            let motion = self.inner.offset_animation.borrow();
            motion.as_ref().map(|motion| (motion.frame_value(now, window), motion.is_running(now)))
        };
        let Some((y, running)) = frame else {
            return false;
        };
        self.inner.handle.set_offset(point(self.inner.handle.offset().x, px(-y)));
        if running {
            return false;
        }
        self.inner.offset_animation.borrow_mut().take();
        true
    }

    /// true while the user is touching or dragging
    pub fn is_tracking(&self) -> bool {
        self.inner.is_tracking.get()
    }

    fn set_tracking(&self, tracking: bool) {
        self.inner.is_tracking.set(tracking);
    }

    /// true while the content still rides a fling or wheel run
    pub fn is_decelerating(&self) -> bool {
        !self.inner.is_tracking.get()
            && self.inner.last_wheel.get().is_some_and(|at| {
                RiseClock::monotonic().saturating_sub(at).as_secs_f64() < ProfileGeometryRefreshPolicy::STABLE_IDLE_DELAY
            })
    }

    fn note_wheel(&self) {
        self.inner.last_wheel.set(Some(RiseClock::monotonic()));
    }

    fn stop_scrolling_and_zooming(&self) {
        self.inner.offset_animation.borrow_mut().take();
        self.inner.last_wheel.set(None);
        self.inner.momentum_suppressed.set(true);
    }

    fn request_redraw(&self, cx: &mut App) {
        if let Some(view) = self.inner.view.get() {
            cx.notify(view);
        }
    }

    fn set_content_offset_in_prepaint(&self, x: f32, y: f32) {
        self.inner.offset_animation.borrow_mut().take();
        self.inner.handle.set_offset(point(px(-x), px(-y)));
        self.inner.prepaint_offset_unlaid.set(true);
    }

    fn redraw_if_prepaint_offset_unlaid(&self, cx: &mut App) {
        if self.inner.prepaint_offset_unlaid.replace(false) {
            self.request_redraw(cx);
        }
    }

    fn did_lay_out(&self, window: &Window) {
        self.inner.prepaint_offset_unlaid.set(false);
        let handle = &self.inner.handle;
        let bounds = handle.bounds();
        let max_offset = handle.max_offset();
        self.inner.window.set(Some(window.window_handle()));
        self.inner.view.set(Some(window.current_view()));
        self.inner.bounds.set(bounds);
        self.inner.content_size.set(size(
            f32::from(bounds.size.width) + f32::from(max_offset.x).max(0.0),
            f32::from(bounds.size.height) + f32::from(max_offset.y).max(0.0),
        ));
    }

    fn did_leave_window(&self) {
        self.inner.window.set(None);
        self.inner.is_tracking.set(false);
        self.inner.last_wheel.set(None);
        self.inner.offset_animation.borrow_mut().take();
    }

    fn take_did_scroll(&self) -> bool {
        let offset = self.content_offset_y();
        if (self.inner.last_reported_offset_y.get() - offset).abs() <= 0.01 {
            return false;
        }
        self.inner.last_reported_offset_y.set(offset);
        true
    }

    #[cfg(test)]
    pub(crate) fn with_test_geometry(width: f32, height: f32, content_width: f32, content_height: f32) -> Self {
        let view = Self::new();
        view.set_test_geometry(width, height, content_width, content_height);
        view
    }

    #[cfg(test)]
    pub(crate) fn set_test_geometry(&self, width: f32, height: f32, content_width: f32, content_height: f32) {
        self.inner.bounds.set(Bounds::new(point(px(0.0), px(0.0)), size(px(width), px(height))));
        self.inner.content_size.set(size(content_width, content_height));
    }
}

/// content layer of the page strip, its transform and the settle animation running on it
#[derive(Clone, Copy)]
struct ProfileHorizontalMotionAnimation {
    from_x: f32,
    to_x: f32,
    started_at: Duration,
    duration: f64,
    timing: CubicBezier,
}

struct ProfileHorizontalMotionViewInner {
    transform_x: Cell<f32>,
    animation: Cell<Option<ProfileHorizontalMotionAnimation>>,
    view: Cell<Option<EntityId>>,
}

/// motion view of the pager,
/// cloning shares it. transform_x is the model value and the presentation value follows the
/// animation
#[derive(Clone)]
pub struct ProfileHorizontalMotionView {
    inner: Rc<ProfileHorizontalMotionViewInner>,
}

impl PartialEq for ProfileHorizontalMotionView {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Default for ProfileHorizontalMotionView {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileHorizontalMotionView {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(ProfileHorizontalMotionViewInner {
                transform_x: Cell::new(0.0),
                animation: Cell::new(None),
                view: Cell::new(None),
            }),
        }
    }

    pub fn transform_x(&self) -> f32 {
        self.inner.transform_x.get()
    }

    fn set_transform_x(&self, x: f32, cx: &mut App) {
        if self.inner.transform_x.get() != x {
            self.inner.transform_x.set(x);
            self.request_redraw(cx);
        }
    }

    /// x transform as currently presented, falls back to the model value
    pub fn presentation_transform_x(&self) -> f32 {
        let Some(animation) = self.inner.animation.get() else {
            return self.inner.transform_x.get();
        };
        let elapsed = RiseClock::monotonic().saturating_sub(animation.started_at).as_secs_f64();
        if elapsed >= animation.duration {
            return self.inner.transform_x.get();
        }
        let progress = animation.timing.progress(elapsed / animation.duration) as f32;
        animation.from_x + (animation.to_x - animation.from_x) * progress
    }

    fn is_animating(&self) -> bool {
        self.inner.animation.get().is_some_and(|animation| {
            RiseClock::monotonic().saturating_sub(animation.started_at).as_secs_f64() < animation.duration
        })
    }

    fn remove_animation(&self) {
        self.inner.animation.set(None);
    }

    fn add_animation(&self, from_x: f32, to_x: f32, duration: f64, timing: CubicBezier, cx: &mut App) {
        self.inner.animation.set(Some(ProfileHorizontalMotionAnimation {
            from_x,
            to_x,
            started_at: RiseClock::monotonic(),
            duration,
            timing,
        }));
        self.request_redraw(cx);
    }

    fn request_redraw(&self, cx: &mut App) {
        if let Some(view) = self.inner.view.get() {
            cx.notify(view);
        }
    }
}

/// element of the motion view with the pages as child. holds cached page views so every
/// motion frame stays cheap
pub struct ProfileHorizontalMotionElement {
    motion_view: ProfileHorizontalMotionView,
    child: AnyElement,
}

impl ProfileHorizontalMotionElement {
    pub fn new(motion_view: ProfileHorizontalMotionView, child: impl IntoElement) -> Self {
        Self { motion_view, child: child.into_any_element() }
    }
}

impl IntoElement for ProfileHorizontalMotionElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ProfileHorizontalMotionElement {
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
        self.motion_view.inner.view.set(Some(window.current_view()));
        let scale = window.scale_factor().max(1.0);
        let translation = (self.motion_view.presentation_transform_x() * scale).round() / scale;
        window.with_element_offset(point(px(translation), px(0.0)), |window| self.child.prepaint(window, cx));
        if self.motion_view.is_animating() {
            window.request_animation_frame();
        }
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProfileGestureState {
    Possible,
    Began,
    Changed,
    Ended,
    Cancelled,
    Failed,
}

/// horizontal pan that rejects clear vertical intent early, at about 2 pt
/// outer scroll waits for this so a vertical jump doesnt leak into profile scroll
struct ProfileDirectionalPanGestureRecognizer {
    state: ProfileGestureState,
    has_tracked_touch: bool,
    translation: Point<f32>,
    previous_timestamp: Duration,
    current_velocity: Point<f32>,
}

impl ProfileDirectionalPanGestureRecognizer {
    fn new() -> Self {
        Self {
            state: ProfileGestureState::Possible,
            has_tracked_touch: false,
            translation: point(0.0, 0.0),
            previous_timestamp: Duration::ZERO,
            current_velocity: point(0.0, 0.0),
        }
    }

    fn touches_began(&mut self, timestamp: Duration) {
        self.reset();
        self.has_tracked_touch = true;
        self.previous_timestamp = timestamp;
    }

    fn touches_moved(&mut self, delta: Point<f32>, timestamp: Duration) {
        let elapsed = timestamp.saturating_sub(self.previous_timestamp).as_secs_f32();
        if elapsed > 0.0 {
            self.current_velocity = point(
                self.current_velocity.x * 0.2 + delta.x / elapsed * 0.8,
                self.current_velocity.y * 0.2 + delta.y / elapsed * 0.8,
            );
        }
        self.translation = point(self.translation.x + delta.x, self.translation.y + delta.y);
        self.previous_timestamp = timestamp;
    }

    fn is_active(&self) -> bool {
        self.state == ProfileGestureState::Began || self.state == ProfileGestureState::Changed
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// vertical pan of the old engine, same samples read as one axis
struct ProfileVerticalPanGestureRecognizer {
    state: ProfileGestureState,
}

#[derive(Clone, Copy)]
struct PendingReturn {
    epoch: u64,
    tab_key: i64,
    band: SharedElementReturnScrollMathBand,
    previous: ProfileVerticalMasterScrollPolicyPosition,
    keeps_position: bool,
    bound_outer_by_return: bool,
    bound_inner_by_return: bool,
    awaits_resume: bool,
}

struct InnerRegistration {
    scroll_view: WeakProfileScrollView,
    token: Uuid,
    original_pan_enabled: bool,
    original_bounces: bool,
    original_scrolls_to_top: bool,
    desired_logical_offset: Option<f32>,
}

impl InnerRegistration {
    fn new(scroll_view: &ProfileScrollView, desired_logical_offset: f32) -> Self {
        Self {
            scroll_view: scroll_view.downgrade(),
            token: Uuid::new_v4(),
            original_pan_enabled: scroll_view.pan_enabled(),
            original_bounces: scroll_view.bounces(),
            original_scrolls_to_top: scroll_view.scrolls_to_top(),
            desired_logical_offset: Some(desired_logical_offset),
        }
    }
}

pub type ProfileOnHorizontalSwipe = Rc<dyn Fn(i64, &mut App)>;
/// runs while the coordinator updates so it must not read the coordinator
pub type ProfileCanHorizontalSwipe = Rc<dyn Fn(i64, &App) -> bool>;
pub type ProfileOnContentScrolled = Rc<dyn Fn(f32, &mut App)>;

fn trace_fields<const N: usize>(fields: [(&str, String); N]) -> HashMap<String, String> {
    fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

fn ca_current_media_time() -> f64 {
    RiseClock::monotonic().as_secs_f64()
}

pub struct ProfileScrollCoordinator {
    active_proxy_height: f32,

    horizontal_drag_progress: f32,

    pub tabs_live_state: Entity<AnimatedTabsPagerLiveState>,

    uses_native_vertical_engine: bool,

    outer_scroll_view: Option<ProfileScrollView>,
    outer_token: Uuid,
    outer_original_pan_enabled: bool,
    outer_original_bounces: bool,
    outer_original_scrolls_to_top: bool,
    outer_original_deceleration_rate: f32,
    outer_original_directional_lock_enabled: bool,
    vertical_pan: Option<ProfileVerticalPanGestureRecognizer>,
    horizontal_pan: Option<ProfileDirectionalPanGestureRecognizer>,
    vertical_last_translation: f32,
    last_outer_minimum: Option<f32>,
    last_outer_maximum: Option<f32>,
    pending_outer_progress: Option<f32>,
    outer_refresh_scheduled: bool,
    deferred_outer_refresh: bool,

    inner_registrations: HashMap<i64, InnerRegistration>,
    inner_refresh_scheduled: HashSet<i64>,
    deferred_inner_refresh_tabs: HashSet<i64>,
    local_offset_by_tab: HashMap<i64, f32>,
    held_inner_offset: Option<(i64, f32, f64)>,
    outer_progress_by_tab: HashMap<i64, f32>,
    inner_range_by_tab: HashMap<i64, f32>,
    applied_proxy_height: f32,
    laid_out_proxy_height: Option<f32>,
    native_base_collapse_range: Option<f32>,
    awaiting_proxy_layout: bool,
    proxy_layout_generation: u64,
    awaiting_proxy_layout_generation: Option<u64>,
    expected_proxy_outer_range: Option<f32>,
    pending_master_position: Option<ProfileVerticalMasterScrollPolicyPosition>,
    proxy_height_update_generation: u64,
    pending_proxy_height: Option<f32>,
    is_mirroring_master_offset: bool,
    mirrors_in_prepaint: bool,
    prepaint_mirrors: Vec<ProfileScrollView>,
    last_native_master_offset: f32,
    last_native_scroll_timestamp: f64,
    native_release_velocity_y: f32,
    parked_outer_scroll_view: Option<WeakProfileScrollView>,
    parked_collapse_range: Option<f32>,
    parked_inner_scroll_views: HashMap<i64, WeakProfileScrollView>,
    pending_return: Option<PendingReturn>,
    pending_return_scope: String,
    opened_master: Option<(String, i64, f32)>,
    return_flight_active: bool,
    is_resolving_return: bool,
    return_scroller: Rc<ProfileScrollCoordinatorReturnScroller>,
    pub on_content_scrolled: Option<ProfileOnContentScrolled>,

    active_tab: i64,
    scope: String,
    interaction_locked: bool,
    is_committing_horizontal_swipe: bool,
    horizontal_settlement_generation: u64,
    horizontal_settlement_active: bool,
    horizontal_pan_start_progress: f32,
    horizontal_pan_start_translation_x: f32,
    horizontal_pan_available_directions: HashSet<i64>,
    horizontal_settlement_target_tab_index: Option<i64>,
    prepared_horizontal_directions: HashSet<i64>,
    horizontal_paging_top_inset: Option<f32>,
    visible_horizontal_tabs: Vec<i64>,
    horizontal_pager_scroll_view: Option<ProfileScrollView>,
    horizontal_pager_motion_view: Option<ProfileHorizontalMotionView>,
    horizontal_pager_token: Uuid,
    horizontal_pager_original_pan_enabled: bool,
    horizontal_pager_original_bounces: bool,
    horizontal_pager_original_scrolls_to_top: bool,
    horizontal_pager_original_directional_lock_enabled: bool,
    horizontal_pager_page_width: f32,
    horizontal_pager_page_count: i64,
    horizontal_pager_unreachable_traced: Option<i64>,
    horizontal_pager_selected_index: i64,
    horizontal_pager_original_motion_transform: f32,
    horizontal_settlement_start_index: f32,
    horizontal_settlement_target_index: f32,
    pending_active_tab_after_horizontal_settle: Option<i64>,
    on_horizontal_swipe: Option<ProfileOnHorizontalSwipe>,
    can_horizontal_swipe: Option<ProfileCanHorizontalSwipe>,
    vertical_motion_active: bool,
    vertical_motion_watchdog_generation: u64,
    status_bar_scroll_to_top_active: bool,
    geometry_refresh_generation: u64,
    scheduled_geometry_refresh_generation: Option<u64>,

    horizontal_settlement_completion_work_item: Option<Task<()>>,

    display_link_running: bool,
    deceleration_velocity: f32,
    previous_display_link_timestamp: f64,

    axis_owners: AnimatedTabsPagerAxisOwners,

    outer_host_scroll_view: ProfileScrollView,
    horizontal_pager_host_scroll_view: ProfileScrollView,
    horizontal_pager_host_motion_view: ProfileHorizontalMotionView,
    page_scroll_views: RefCell<HashMap<i64, ProfileScrollView>>,

    _inner_content_will_reorder_observer: RiseSubscription,
    _inner_content_will_reorder_task: Task<()>,
}

impl ProfileScrollCoordinator {
    const INNER_OFFSET_HOLD_DURATION: f64 = 0.4;
    const INNER_OFFSET_HOLD_RESTORE_DELAYS: [f64; 4] = [0.0, 0.05, 0.15, 0.35];

    pub fn new(cx: &mut Context<Self>) -> Self {
        let (reorders, mut inbox) = mpsc::unbounded::<()>();
        let observer = RiseAppEvents::default_center().observe(PROFILE_INNER_CONTENT_WILL_REORDER, move |_| {
            let _ = reorders.unbounded_send(());
        });
        let reorder_task = cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while inbox.next().await.is_some() {
                if this.update(cx, |coordinator, cx| coordinator.hold_active_inner_offset(cx)).is_err() {
                    break;
                }
            }
        });
        let launch_arguments: Vec<String> = rise_platform::process_info::arguments();
        let on_return_resumable = Rc::new(RefCell::new(None));
        Self {
            active_proxy_height: 0.0,
            horizontal_drag_progress: 0.0,
            tabs_live_state: cx.new(|_| AnimatedTabsPagerLiveState::new()),
            uses_native_vertical_engine: ProfileVerticalMasterScrollPolicy::uses_native_master_engine(&launch_arguments),
            outer_scroll_view: None,
            outer_token: Uuid::new_v4(),
            outer_original_pan_enabled: true,
            outer_original_bounces: true,
            outer_original_scrolls_to_top: true,
            outer_original_deceleration_rate: PROFILE_DECELERATION_RATE_NORMAL,
            outer_original_directional_lock_enabled: false,
            vertical_pan: None,
            horizontal_pan: None,
            vertical_last_translation: 0.0,
            last_outer_minimum: None,
            last_outer_maximum: None,
            pending_outer_progress: None,
            outer_refresh_scheduled: false,
            deferred_outer_refresh: false,
            inner_registrations: HashMap::new(),
            inner_refresh_scheduled: HashSet::new(),
            deferred_inner_refresh_tabs: HashSet::new(),
            local_offset_by_tab: HashMap::new(),
            held_inner_offset: None,
            outer_progress_by_tab: HashMap::new(),
            inner_range_by_tab: HashMap::new(),
            applied_proxy_height: 0.0,
            laid_out_proxy_height: None,
            native_base_collapse_range: None,
            awaiting_proxy_layout: false,
            proxy_layout_generation: 0,
            awaiting_proxy_layout_generation: None,
            expected_proxy_outer_range: None,
            pending_master_position: None,
            proxy_height_update_generation: 0,
            pending_proxy_height: None,
            is_mirroring_master_offset: false,
            mirrors_in_prepaint: false,
            prepaint_mirrors: Vec::new(),
            last_native_master_offset: 0.0,
            last_native_scroll_timestamp: 0.0,
            native_release_velocity_y: 0.0,
            parked_outer_scroll_view: None,
            parked_collapse_range: None,
            parked_inner_scroll_views: HashMap::new(),
            pending_return: None,
            pending_return_scope: String::new(),
            opened_master: None,
            return_flight_active: false,
            is_resolving_return: false,
            return_scroller: Rc::new(ProfileScrollCoordinatorReturnScroller {
                coordinator: cx.weak_entity(),
                on_return_resumable,
            }),
            on_content_scrolled: None,
            active_tab: 0,
            scope: String::new(),
            interaction_locked: false,
            is_committing_horizontal_swipe: false,
            horizontal_settlement_generation: 0,
            horizontal_settlement_active: false,
            horizontal_pan_start_progress: 0.0,
            horizontal_pan_start_translation_x: 0.0,
            horizontal_pan_available_directions: HashSet::new(),
            horizontal_settlement_target_tab_index: None,
            prepared_horizontal_directions: HashSet::new(),
            horizontal_paging_top_inset: None,
            visible_horizontal_tabs: vec![0],
            horizontal_pager_scroll_view: None,
            horizontal_pager_motion_view: None,
            horizontal_pager_token: Uuid::new_v4(),
            horizontal_pager_original_pan_enabled: true,
            horizontal_pager_original_bounces: true,
            horizontal_pager_original_scrolls_to_top: true,
            horizontal_pager_original_directional_lock_enabled: false,
            horizontal_pager_page_width: 1.0,
            horizontal_pager_page_count: 0,
            horizontal_pager_unreachable_traced: None,
            horizontal_pager_selected_index: 0,
            horizontal_pager_original_motion_transform: 0.0,
            horizontal_settlement_start_index: 0.0,
            horizontal_settlement_target_index: 0.0,
            pending_active_tab_after_horizontal_settle: None,
            on_horizontal_swipe: None,
            can_horizontal_swipe: None,
            vertical_motion_active: false,
            vertical_motion_watchdog_generation: 0,
            status_bar_scroll_to_top_active: false,
            geometry_refresh_generation: 0,
            scheduled_geometry_refresh_generation: None,
            horizontal_settlement_completion_work_item: None,
            display_link_running: false,
            deceleration_velocity: 0.0,
            previous_display_link_timestamp: 0.0,
            axis_owners: AnimatedTabsPagerAxisOwners::default(),
            outer_host_scroll_view: ProfileScrollView::new(),
            horizontal_pager_host_scroll_view: ProfileScrollView::new(),
            horizontal_pager_host_motion_view: ProfileHorizontalMotionView::new(),
            page_scroll_views: RefCell::new(HashMap::new()),
            _inner_content_will_reorder_observer: observer,
            _inner_content_will_reorder_task: reorder_task,
        }
    }

    pub fn active_proxy_height(&self) -> f32 {
        self.active_proxy_height
    }

    pub fn uses_native_vertical_engine(&self) -> bool {
        self.uses_native_vertical_engine
    }

    /// where a nested horizontal scroll registers so the pan yields to it
    pub fn axis_owners(&self) -> AnimatedTabsPagerAxisOwners {
        self.axis_owners.clone()
    }

    /// the coordinator as a shared element return scroller
    pub fn return_scroller(&self) -> Rc<dyn SharedElementReturnScroller> {
        self.return_scroller.clone()
    }

    /// outer scroll the screen draws, ProfileOuterScrollIntrospector goes first in its content
    pub fn outer_host_scroll_view(&self) -> ProfileScrollView {
        self.outer_host_scroll_view.clone()
    }

    /// strip the screen draws, pages go inside a motion element of the host motion view
    pub fn horizontal_pager_host_scroll_view(&self) -> ProfileScrollView {
        self.horizontal_pager_host_scroll_view.clone()
    }

    pub fn horizontal_pager_host_motion_view(&self) -> ProfileHorizontalMotionView {
        self.horizontal_pager_host_motion_view.clone()
    }

    pub fn horizontal_pager_visual_index(&self) -> Option<f32> {
        if self.horizontal_pager_scroll_view.is_none() || self.horizontal_pager_motion_view.is_none() {
            return None;
        }
        Some(self.horizontal_pager_absolute_index(true))
    }

    /// scroll view of one page, asking again on every frame gives the same view
    pub fn page_scroll_view(&self, tab_key: i64) -> ProfileScrollView {
        self.page_scroll_views.borrow_mut().entry(tab_key).or_default().clone()
    }

    fn outer_viewport_top(&self) -> Option<f32> {
        self.outer_scroll_view.as_ref().map(|outer| f32::from(outer.frame().top()))
    }

    /// runs work later on the main thread
    fn main_async(&self, cx: &mut Context<Self>, work: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let _ = this.update(cx, work);
        })
        .detach();
    }

    /// runs work on the main thread after a delay
    fn main_async_after(
        &self,
        delay: f64,
        cx: &mut Context<Self>,
        work: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let timer = cx.background_executor().timer(Duration::from_secs_f64(delay));
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            timer.await;
            let _ = this.update(cx, work);
        })
        .detach();
    }

    pub fn bind_outer(
        &mut self,
        scroll_view: &ProfileScrollView,
        on_horizontal_swipe: ProfileOnHorizontalSwipe,
        can_horizontal_swipe: ProfileCanHorizontalSwipe,
        cx: &mut Context<Self>,
    ) -> Uuid {
        if self.outer_scroll_view.as_ref() == Some(scroll_view) {
            self.on_horizontal_swipe = Some(on_horizontal_swipe);
            self.can_horizontal_swipe = Some(can_horizontal_swipe);
            self.enforce_outer_configuration(scroll_view);
            return self.outer_token;
        }

        self.unbind_outer_current(cx);
        self.on_horizontal_swipe = Some(on_horizontal_swipe);
        self.can_horizontal_swipe = Some(can_horizontal_swipe);

        self.outer_scroll_view = Some(scroll_view.clone());
        self.outer_token = Uuid::new_v4();
        self.outer_original_pan_enabled = scroll_view.pan_enabled();
        self.outer_original_bounces = scroll_view.bounces();
        self.outer_original_scrolls_to_top = scroll_view.scrolls_to_top();
        self.outer_original_deceleration_rate = scroll_view.deceleration_rate();
        self.outer_original_directional_lock_enabled = scroll_view.is_directional_lock_enabled();
        self.last_outer_minimum = None;
        self.last_outer_maximum = None;
        self.native_base_collapse_range = None;
        self.awaiting_proxy_layout = false;
        self.awaiting_proxy_layout_generation = None;
        self.expected_proxy_outer_range = None;
        let active_progress = self.outer_progress_by_tab.get(&self.active_tab).copied().unwrap_or(0.0);
        if self.uses_native_vertical_engine {
            self.pending_master_position = Some(ProfileVerticalMasterScrollPolicyPosition {
                collapse_progress: active_progress,
                inner_offset: self.local_offset_by_tab.get(&self.active_tab).copied().unwrap_or(0.0),
            });
            self.pending_outer_progress = None;
        } else {
            self.pending_outer_progress = Some(active_progress);
        }

        if !self.uses_native_vertical_engine {
            self.vertical_pan = Some(ProfileVerticalPanGestureRecognizer { state: ProfileGestureState::Possible });
        }

        self.horizontal_pan = Some(ProfileDirectionalPanGestureRecognizer::new());

        self.enforce_outer_configuration(scroll_view);
        self.enforce_all_inner_configurations();
        self.refresh_outer(scroll_view, cx);
        if self.uses_native_vertical_engine {
            if let Some(inner) = self.active_inner_scroll_view() {
                self.refresh_inner(self.active_tab, &inner, cx);
            }
            self.synchronize_native_vertical_position(scroll_view, cx);
        }
        self.outer_token
    }

    pub fn unbind_outer(&mut self, scroll_view: &ProfileScrollView, token: Uuid, cx: &mut Context<Self>) {
        if self.outer_scroll_view.as_ref() != Some(scroll_view) || self.outer_token != token {
            return;
        }
        self.unbind_outer_current(cx);
    }

    pub fn register_inner_for(
        &mut self,
        scroll_view: &ProfileScrollView,
        tab: ProfileContentTab,
        cx: &mut Context<Self>,
    ) -> Uuid {
        self.register_inner(scroll_view, tab.raw_value(), cx)
    }

    pub fn register_inner(&mut self, scroll_view: &ProfileScrollView, for_key: i64, cx: &mut Context<Self>) -> Uuid {
        let tab = for_key;
        if let Some(current) = self.inner_registrations.get(&tab)
            && current.scroll_view.upgrade().as_ref() == Some(scroll_view)
        {
            let token = current.token;
            self.enforce_inner_configuration(tab);
            return token;
        }

        if self.inner_registrations.contains_key(&tab) {
            self.remove_inner_registration(tab, cx);
        }

        let duplicate = self
            .inner_registrations
            .iter()
            .find(|(key, registration)| **key != tab && registration.scroll_view.upgrade().as_ref() == Some(scroll_view))
            .map(|(key, _)| *key);
        if let Some(other_tab) = duplicate {
            self.remove_inner_registration(other_tab, cx);
        }

        let registration =
            InnerRegistration::new(scroll_view, self.local_offset_by_tab.get(&tab).copied().unwrap_or(0.0));
        let token = registration.token;
        self.inner_registrations.insert(tab, registration);

        self.enforce_inner_configuration(tab);
        self.refresh_inner(tab, scroll_view, cx);
        token
    }

    pub fn unregister_inner_for(
        &mut self,
        scroll_view: &ProfileScrollView,
        tab: ProfileContentTab,
        token: Uuid,
        cx: &mut Context<Self>,
    ) {
        self.unregister_inner(scroll_view, tab.raw_value(), token, cx);
    }

    pub fn unregister_inner(&mut self, scroll_view: &ProfileScrollView, for_key: i64, token: Uuid, cx: &mut Context<Self>) {
        let Some(registration) = self.inner_registrations.get(&for_key) else {
            return;
        };
        if registration.scroll_view.upgrade().as_ref() != Some(scroll_view) || registration.token != token {
            return;
        }
        self.remove_inner_registration(for_key, cx);
    }

    pub fn inner_scroll_view(&self, for_key: i64) -> Option<ScrollHandle> {
        self.registered_inner_scroll_view(for_key).map(|scroll_view| scroll_view.handle())
    }

    fn registered_inner_scroll_view(&self, tab: i64) -> Option<ProfileScrollView> {
        self.inner_registrations.get(&tab).and_then(|registration| registration.scroll_view.upgrade())
    }

    pub fn begin_inner_date_scrub(&mut self, for_key: i64, cx: &mut Context<Self>) -> Option<(f32, f32)> {
        let tab = for_key;
        if tab != self.active_tab {
            return None;
        }
        let scroll_view = self.registered_inner_scroll_view(tab)?;
        self.stop_vertical_motion_driver(cx);
        let range = (self.maximum_offset(&scroll_view) - self.minimum_offset(&scroll_view)).max(0.0);
        let offset = if self.outer_scroll_view.is_none() {
            self.logical_offset(&scroll_view)
        } else if self.uses_native_vertical_engine {
            self.current_native_master_position().inner_offset
        } else {
            self.logical_offset(&scroll_view)
        };
        Some((offset.max(0.0).min(range), range))
    }

    pub fn update_inner_date_scrub(&mut self, logical_offset: f32, for_key: i64, cx: &mut Context<Self>) {
        let tab = for_key;
        if tab != self.active_tab {
            return;
        }
        let Some(scroll_view) = self.registered_inner_scroll_view(tab) else {
            return;
        };
        let range = (self.maximum_offset(&scroll_view) - self.minimum_offset(&scroll_view)).max(0.0);
        let target = logical_offset.max(0.0).min(range);

        let outer = self.outer_scroll_view.clone();
        if let (true, Some(outer)) = (self.uses_native_vertical_engine, outer.as_ref()) {
            let master_offset = self.native_collapse_range(outer) + target;
            self.set_offset_y(self.minimum_offset(outer) + master_offset, outer, cx);
            self.synchronize_native_vertical_position(outer, cx);
        } else {
            if let Some(outer) = outer.as_ref() {
                self.set_offset_y(self.maximum_offset(outer), outer, cx);
            }
            self.set_logical_offset(target, &scroll_view, cx);
        }
        self.local_offset_by_tab.insert(tab, target);
        self.outer_progress_by_tab.insert(tab, 1.0);
    }

    pub fn reset_tab_to_content_top(&mut self, tab: i64, cx: &mut Context<Self>) {
        if let Some(registration) = self.inner_registrations.get_mut(&tab) {
            registration.desired_logical_offset = None;
        }
        if tab != self.active_tab {
            self.local_offset_by_tab.insert(tab, 0.0);
            return;
        }
        let depth = match self.outer_scroll_view.clone().filter(|_| self.uses_native_vertical_engine) {
            Some(outer) => outer.content_offset_y() - self.minimum_offset(&outer) - self.native_collapse_range(&outer),
            None => self.active_inner_scroll_view().map_or(0.0, |inner| self.logical_offset(&inner)),
        };
        if depth > ProfileVerticalMasterScrollPolicy::EPSILON {
            self.update_inner_date_scrub(0.0, tab, cx);
        }
    }

    pub fn end_inner_date_scrub(&mut self, for_key: i64, cx: &mut Context<Self>) {
        if for_key != self.active_tab {
            return;
        }
        self.capture_active_position();
        self.finish_vertical_motion(cx);
    }

    pub fn can_prepare_return(&self, tab_key: i64) -> bool {
        self.uses_native_vertical_engine
            && tab_key == self.active_tab
            && self.return_outer_scroll_view().is_some()
            && self.inner_range_by_tab.contains_key(&tab_key)
            && self.pending_return.is_none()
    }

    fn return_outer_scroll_view(&self) -> Option<ProfileScrollView> {
        self.outer_scroll_view
            .clone()
            .or_else(|| self.parked_outer_scroll_view.as_ref().and_then(WeakProfileScrollView::upgrade))
    }

    pub fn return_viewport_snapshot(&self, tab_key: i64) -> Option<SharedElementReturnViewport> {
        if !self.uses_native_vertical_engine || tab_key != self.active_tab {
            return None;
        }
        let outer = self.return_outer_scroll_view()?;
        let inner_range = *self.inner_range_by_tab.get(&tab_key)?;
        let inner = self
            .registered_inner_scroll_view(tab_key)
            .or_else(|| self.parked_inner_scroll_views.get(&tab_key).and_then(WeakProfileScrollView::upgrade));
        Some(SharedElementReturnViewport {
            height: outer.bounds_height(),
            screen_origin: outer.frame().origin,
            inner_width: inner.as_ref().unwrap_or(&outer).bounds_width(),
            collapse_range: self.return_collapse_range().unwrap_or(0.0),
            inner_range: inner_range.max(0.0),
        })
    }

    pub fn note_open_position(&mut self, tab_key: i64) {
        self.opened_master = None;
        if !self.uses_native_vertical_engine || tab_key != self.active_tab || self.awaiting_proxy_layout {
            return;
        }
        let Some(outer) = self.outer_scroll_view.as_ref().filter(|outer| outer.is_in_window()) else {
            return;
        };
        let master = outer.content_offset_y() - self.minimum_offset(outer);
        if master.is_finite() {
            self.opened_master = Some((self.scope.clone(), tab_key, master.max(0.0)));
        }
    }

    fn kept_master_offset(&self, tab_key: i64) -> Option<f32> {
        let (scope, opened_tab, master_offset) = self.opened_master.as_ref()?;
        (*scope == self.scope && *opened_tab == tab_key).then_some(*master_offset)
    }

    pub fn prepare_return(&mut self, request: SharedElementReturnRequest) -> Option<SharedElementReturnPlan> {
        let tab = request.tab_key;
        if !self.can_prepare_return(tab) {
            return None;
        }
        let outer = self.return_outer_scroll_view()?;
        let inner_range = *self.inner_range_by_tab.get(&tab)?;

        let collapse_range = self.return_collapse_range().unwrap_or(0.0);
        let previous = self.current_native_master_position();
        let position = self.return_position(
            request.item_content_rect,
            request.band,
            outer.bounds_height(),
            previous,
            request.keeps_position,
            self.kept_master_offset(tab),
            collapse_range,
            inner_range.max(0.0),
        );

        self.outer_progress_by_tab.insert(tab, position.collapse_progress);
        self.local_offset_by_tab.insert(tab, position.inner_offset);
        self.return_flight_active = true;
        self.pending_return_scope = self.scope.clone();
        self.pending_return = Some(PendingReturn {
            epoch: request.epoch,
            tab_key: tab,
            band: request.band,
            previous,
            keeps_position: request.keeps_position,
            bound_outer_by_return: false,
            bound_inner_by_return: false,
            awaits_resume: false,
        });

        let predicted_frame = SharedElementReturnScrollMath::predicted_screen_frame(
            request.item_content_rect,
            outer.frame().origin,
            0.0,
            collapse_range,
            position.master_offset,
        );
        Some(SharedElementReturnPlan { epoch: request.epoch, predicted_frame, clamp: position.clamp })
    }

    pub fn resolve_return(
        &mut self,
        epoch: u64,
        item_content_rect: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) -> SharedElementReturnResolution {
        let Some(pending) = self.pending_return.filter(|pending| pending.epoch == epoch) else {
            return SharedElementReturnResolution::Unavailable;
        };
        if !self.uses_native_vertical_engine
            || self.pending_return_scope != self.scope
            || pending.tab_key != self.active_tab
        {
            return SharedElementReturnResolution::Unavailable;
        }
        let tab = pending.tab_key;
        let Some(candidate) = self.return_outer_scroll_view() else {
            return SharedElementReturnResolution::Unavailable;
        };
        if !candidate.is_in_window() {
            return SharedElementReturnResolution::Deferred;
        }

        self.is_resolving_return = true;
        let resolution = self.resolve_return_in_window(epoch, tab, pending, &candidate, item_content_rect, cx);
        self.is_resolving_return = false;
        resolution
    }

    fn resolve_return_in_window(
        &mut self,
        epoch: u64,
        tab: i64,
        pending: PendingReturn,
        candidate: &ProfileScrollView,
        item_content_rect: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) -> SharedElementReturnResolution {
        if self.outer_scroll_view.is_none() {
            self.bind_outer(candidate, Rc::new(|_: i64, _: &mut App| {}), Rc::new(|_: i64, _: &App| false), cx);
            if let Some(pending) = self.pending_return.as_mut() {
                pending.bound_outer_by_return = true;
            }
        }
        let Some(outer) = self.outer_scroll_view.clone().filter(|outer| outer == candidate) else {
            return SharedElementReturnResolution::Unavailable;
        };

        if !self.inner_registrations.contains_key(&tab)
            && let Some(inner) = self.parked_inner_scroll_views.get(&tab).and_then(WeakProfileScrollView::upgrade)
            && inner.is_in_window()
            && inner != outer
            && self.horizontal_pager_scroll_view.as_ref() != Some(&inner)
            && !self
                .inner_registrations
                .values()
                .any(|registration| registration.scroll_view.upgrade().as_ref() == Some(&inner))
        {
            self.register_inner(&inner, tab, cx);
            if let Some(pending) = self.pending_return.as_mut() {
                pending.bound_inner_by_return = true;
            }
        }

        if let Some(inner) = self.registered_inner_scroll_view(tab).filter(ProfileScrollView::is_in_window) {
            let inner_content_height = inner.content_size().height;
            if inner_content_height > 1.0 && f32::from(item_content_rect.bottom()) > inner_content_height + 1.0 {
                if let Some(pending) = self.pending_return.as_mut() {
                    pending.awaits_resume = true;
                }
                return SharedElementReturnResolution::Deferred;
            }
            let live_inner_range = (self.maximum_offset(&inner) - self.minimum_offset(&inner)).max(0.0);
            if self.inner_range_by_tab.get(&tab).is_none_or(|stored| (live_inner_range - stored).abs() > 0.5) {
                self.inner_range_by_tab.insert(tab, live_inner_range);
            }
        }

        self.pending_proxy_height = None;
        self.proxy_height_update_generation = self.proxy_height_update_generation.wrapping_add(1);
        if let Some(live_range) = self.inner_range_by_tab.get(&tab).copied()
            && (live_range - self.applied_proxy_height).abs() > 0.5
        {
            self.apply_active_proxy_height(live_range, cx);
        }
        if self.pending_return.map(|pending| pending.epoch) != Some(epoch) {
            return SharedElementReturnResolution::Unavailable;
        }

        if self.awaiting_proxy_layout {
            if let Some(pending) = self.pending_return.as_mut() {
                pending.awaits_resume = true;
            }
            return SharedElementReturnResolution::Deferred;
        }
        if let Some(pending) = self.pending_return.as_mut() {
            pending.awaits_resume = false;
        }

        let collapse_range = self.native_collapse_range(&outer);
        let inner_range =
            self.inner_range_by_tab.get(&tab).copied().unwrap_or_else(|| self.active_inner_logical_range()).max(0.0);
        let position = self.return_position(
            item_content_rect,
            pending.band,
            outer.bounds_height(),
            pending.previous,
            pending.keeps_position,
            self.kept_master_offset(tab),
            collapse_range,
            inner_range,
        );
        let minimum = self.minimum_offset(&outer);
        let maximum = self.maximum_offset(&outer);
        self.set_offset_y((minimum + position.master_offset).max(minimum).min(maximum), &outer, cx);
        self.pending_master_position = None;
        self.synchronize_native_vertical_position(&outer, cx);
        if let Some(registration) = self.inner_registrations.get_mut(&tab) {
            registration.desired_logical_offset = None;
        }
        self.capture_active_position();

        self.reconcile_stale_vertical_motion("return_seed", cx);
        if self.pending_return.map(|pending| pending.epoch) != Some(epoch) {
            return SharedElementReturnResolution::Unavailable;
        }

        SharedElementReturnResolution::Ready {
            predicted_frame: SharedElementReturnScrollMath::predicted_screen_frame(
                item_content_rect,
                outer.frame().origin,
                0.0,
                collapse_range,
                position.master_offset,
            ),
        }
    }

    pub fn cancel_return(&mut self, epoch: u64, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_return.filter(|pending| pending.epoch == epoch) else {
            return;
        };
        let tab = pending.tab_key;
        if pending.bound_outer_by_return
            && self.outer_scroll_view.as_ref().is_some_and(|outer| !outer.is_in_window())
        {
            self.unbind_outer_current(cx);
        }
        if pending.bound_inner_by_return
            && self.inner_registrations.contains_key(&tab)
            && !self.registered_inner_scroll_view(tab).is_some_and(|inner| inner.is_in_window())
        {
            self.remove_inner_registration(tab, cx);
        }

        self.outer_progress_by_tab.insert(tab, pending.previous.collapse_progress);
        self.local_offset_by_tab.insert(tab, pending.previous.inner_offset);
        if self.pending_master_position.is_some() {
            self.pending_master_position = Some(pending.previous);
        }
        self.return_flight_active = false;
        self.pending_return = None;
        if self.pending_proxy_height.is_some() {
            self.schedule_pending_proxy_height_application(cx);
        }
        self.schedule_deferred_geometry_refresh_flush(cx);
    }

    pub fn cancel_live_return(&mut self, epoch: u64, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_return.filter(|pending| pending.epoch == epoch) else {
            return;
        };
        let tab = pending.tab_key;
        let outer = self.outer_scroll_view.clone().filter(ProfileScrollView::is_in_window);
        let Some(outer) = outer.filter(|_| {
            self.uses_native_vertical_engine
                && !pending.bound_outer_by_return
                && !pending.bound_inner_by_return
                && self.pending_return_scope == self.scope
                && tab == self.active_tab
        }) else {
            self.cancel_return(epoch, cx);
            return;
        };
        self.return_flight_active = false;
        self.pending_return = None;
        if self.awaiting_proxy_layout {
            self.outer_progress_by_tab.insert(tab, pending.previous.collapse_progress);
            self.local_offset_by_tab.insert(tab, pending.previous.inner_offset);
            self.pending_master_position = Some(pending.previous);
        } else {
            let inner_range =
                self.inner_range_by_tab.get(&tab).copied().unwrap_or_else(|| self.active_inner_logical_range()).max(0.0);
            let collapse_range = self.native_collapse_range(&outer);
            let master =
                ProfileVerticalMasterScrollPolicy::master_offset(pending.previous, collapse_range, inner_range);
            let minimum = self.minimum_offset(&outer);
            let maximum = self.maximum_offset(&outer);
            self.set_offset_y((minimum + master).max(minimum).min(maximum), &outer, cx);
            self.pending_master_position = None;
            self.synchronize_native_vertical_position(&outer, cx);
            if let Some(registration) = self.inner_registrations.get_mut(&tab) {
                registration.desired_logical_offset = None;
            }
            self.capture_active_position();
        }
        if self.pending_proxy_height.is_some() {
            self.schedule_pending_proxy_height_application(cx);
        }
        self.schedule_deferred_geometry_refresh_flush(cx);
    }

    pub fn finish_return(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if self.pending_return.map(|pending| pending.epoch) != Some(epoch) {
            return;
        }
        self.return_flight_active = false;
        self.pending_return = None;
        if self.pending_proxy_height.is_some() {
            self.schedule_pending_proxy_height_application(cx);
        }
        self.schedule_deferred_geometry_refresh_flush(cx);
    }

    fn resume_pending_return_if_needed(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_return else {
            return;
        };
        if !pending.awaits_resume || self.is_resolving_return {
            return;
        }
        let epoch = pending.epoch;
        self.main_async(cx, move |coordinator, cx| {
            let Some(pending) = coordinator.pending_return else {
                return;
            };
            if pending.epoch != epoch || !pending.awaits_resume {
                return;
            }
            let handler = coordinator.return_scroller.on_return_resumable.borrow().clone();
            if let Some(handler) = handler {
                cx.defer(move |cx| handler(epoch, cx));
            }
        });
    }

    fn expire_stale_return_if_needed(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_return else {
            return;
        };
        ProfilePerformanceTrace::shared()
            .record("return.stale_flight", trace_fields([("epoch", pending.epoch.to_string())]));
        self.finish_return(pending.epoch, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn return_position(
        &self,
        item_rect: Bounds<Pixels>,
        band: SharedElementReturnScrollMathBand,
        viewport_height: f32,
        previous: ProfileVerticalMasterScrollPolicyPosition,
        keeps_position: bool,
        kept_master: Option<f32>,
        collapse_range: f32,
        inner_range: f32,
    ) -> SharedElementReturnScrollMathPosition {
        let collapse = collapse_range.max(0.0);
        let previous_master = kept_master
            .unwrap_or_else(|| ProfileVerticalMasterScrollPolicy::master_offset(previous, collapse, inner_range));
        if keeps_position {
            let master = previous_master.max(0.0).min(collapse + inner_range.max(0.0));
            return SharedElementReturnScrollMathPosition {
                master_offset: master,
                collapse_progress: if collapse > ProfileVerticalMasterScrollPolicy::EPSILON {
                    (master / collapse).min(1.0)
                } else {
                    1.0
                },
                inner_offset: (master - collapse).max(0.0),
                clamp: SharedElementReturnScrollMathClamp::None,
            };
        }
        SharedElementReturnScrollMath::position_item_rect(
            item_rect,
            band,
            viewport_height,
            Some(previous_master),
            0.0,
            collapse,
            inner_range,
            ProfileVerticalMasterScrollPolicy::EPSILON,
        )
    }

    fn return_collapse_range(&self) -> Option<f32> {
        let Some(outer) = self.outer_scroll_view.as_ref() else {
            return self.parked_collapse_range.or(self.native_base_collapse_range);
        };
        Some(self.measured_return_collapse_range(outer))
    }

    /// same value as native_collapse_range but doesnt write its cache
    fn measured_return_collapse_range(&self, scroll_view: &ProfileScrollView) -> f32 {
        if self.awaiting_proxy_layout
            && let Some(native_base_collapse_range) = self.native_base_collapse_range
        {
            return native_base_collapse_range;
        }
        ProfileVerticalMasterScrollPolicy::collapse_range(
            self.maximum_offset(scroll_view) - self.minimum_offset(scroll_view),
            self.laid_out_proxy_height.unwrap_or(0.0),
        )
    }

    pub fn bind_horizontal_pager(
        &mut self,
        scroll_view: &ProfileScrollView,
        motion_view: &ProfileHorizontalMotionView,
        selected_index: i64,
        page_count: i64,
        page_width: f32,
        cx: &mut Context<Self>,
    ) -> Uuid {
        if !self.is_valid_horizontal_pager(scroll_view) {
            return self.horizontal_pager_token;
        }

        if self.horizontal_pager_scroll_view.as_ref() == Some(scroll_view)
            && self.horizontal_pager_motion_view.as_ref() == Some(motion_view)
        {
            self.update_horizontal_pager(scroll_view, motion_view, selected_index, page_count, page_width, cx);
            return self.horizontal_pager_token;
        }

        self.unbind_horizontal_pager_current(cx);
        self.horizontal_pager_scroll_view = Some(scroll_view.clone());
        self.horizontal_pager_motion_view = Some(motion_view.clone());
        self.horizontal_pager_token = Uuid::new_v4();
        self.horizontal_pager_original_pan_enabled = scroll_view.pan_enabled();
        self.horizontal_pager_original_bounces = scroll_view.bounces();
        self.horizontal_pager_original_scrolls_to_top = scroll_view.scrolls_to_top();
        self.horizontal_pager_original_directional_lock_enabled = scroll_view.is_directional_lock_enabled();
        self.horizontal_pager_original_motion_transform = motion_view.transform_x();
        self.horizontal_pager_page_count = page_count.max(0);
        self.horizontal_pager_selected_index = selected_index.max(0).min((self.horizontal_pager_page_count - 1).max(0));
        self.horizontal_pager_page_width = page_width.max(1.0);
        self.enforce_horizontal_pager_configuration(scroll_view);
        self.set_horizontal_pager_absolute_index(self.horizontal_pager_selected_index as f32, true, cx);
        self.horizontal_pager_token
    }

    pub fn is_valid_horizontal_pager(&self, scroll_view: &ProfileScrollView) -> bool {
        self.outer_scroll_view.as_ref() != Some(scroll_view)
            && !self
                .inner_registrations
                .values()
                .any(|registration| registration.scroll_view.upgrade().as_ref() == Some(scroll_view))
    }

    pub fn holds_horizontal_pager(&self, scroll_view: &ProfileScrollView, token: Option<Uuid>) -> bool {
        token == Some(self.horizontal_pager_token) && self.horizontal_pager_scroll_view.as_ref() == Some(scroll_view)
    }

    pub fn unbind_horizontal_pager(&mut self, scroll_view: &ProfileScrollView, token: Uuid, cx: &mut Context<Self>) {
        if self.horizontal_pager_scroll_view.as_ref() != Some(scroll_view) || self.horizontal_pager_token != token {
            return;
        }
        self.unbind_horizontal_pager_current(cx);
    }

    pub fn update_horizontal_pager(
        &mut self,
        scroll_view: &ProfileScrollView,
        motion_view: &ProfileHorizontalMotionView,
        selected_index: i64,
        page_count: i64,
        page_width: f32,
        cx: &mut Context<Self>,
    ) {
        if self.horizontal_pager_scroll_view.as_ref() != Some(scroll_view)
            || self.horizontal_pager_motion_view.as_ref() != Some(motion_view)
        {
            return;
        }

        let old_width = self.horizontal_pager_page_width.max(1.0);
        let old_page_count = self.horizontal_pager_page_count;
        let old_resting_index =
            ProfileHorizontalPagerPolicy::absolute_index(scroll_view.content_offset_x(), 0.0, old_width, old_page_count);
        let old_absolute_index =
            self.horizontal_pager_absolute_index_in(scroll_view, old_width, self.horizontal_settlement_active);
        let normalized_count = page_count.max(0);
        let normalized_selected = selected_index.max(0).min((normalized_count - 1).max(0));

        self.horizontal_pager_page_count = normalized_count;
        self.horizontal_pager_selected_index = normalized_selected;
        self.horizontal_pager_page_width = page_width.max(1.0);
        self.enforce_horizontal_pager_configuration(scroll_view);

        if self.horizontal_settlement_active || self.horizontal_pan_is_active() {
            if old_width != self.horizontal_pager_page_width || old_page_count != normalized_count {
                self.set_horizontal_pager_absolute_index(old_resting_index, true, cx);
                self.set_horizontal_pager_visual_absolute_index(old_absolute_index, true, cx);
            }
        } else {
            self.set_horizontal_pager_absolute_index(normalized_selected as f32, true, cx);
        }
    }

    fn trace_horizontal_pager_selection(&self, tab: i64, target_index: Option<i64>) {
        let (offset_x, content_width, bounds_width) = self
            .horizontal_pager_scroll_view
            .as_ref()
            .map(|view| (view.content_offset_x(), view.content_size().width, view.bounds_width()))
            .unwrap_or((-1.0, -1.0, -1.0));
        tracing::warn!(
            target: "riseonly::X1567",
            tab,
            ?target_index,
            selected_index = self.horizontal_pager_selected_index,
            page_count = self.horizontal_pager_page_count,
            page_width = self.horizontal_pager_page_width,
            visible_tabs = self.visible_horizontal_tabs.len(),
            bound = self.horizontal_pager_scroll_view.is_some() && self.horizontal_pager_motion_view.is_some(),
            offset_x,
            content_width,
            bounds_width,
            "pager selection"
        );
    }

    fn reconcile_horizontal_pager_rest(&mut self, scroll_view: &ProfileScrollView, selected_index: i64, cx: &mut Context<Self>) {
        if self.horizontal_pager_scroll_view.as_ref() != Some(scroll_view)
            || self.horizontal_settlement_active
            || self.horizontal_pan_is_active()
            || self.pending_active_tab_after_horizontal_settle.is_some()
        {
            return;
        }
        let Some(motion_view) = self.horizontal_pager_motion_view.as_ref() else {
            return;
        };
        if motion_view.is_animating()
            || (motion_view.transform_x() - self.horizontal_pager_original_motion_transform).abs() > 0.1
        {
            return;
        }
        let page_width = self.horizontal_pager_page_width.max(1.0);
        let target_x = ProfileHorizontalPagerPolicy::offset_x(
            selected_index as f32,
            0.0,
            page_width,
            self.horizontal_pager_page_count,
        );
        let current_x = scroll_view.content_offset_x();
        let reachable_x = scroll_view.content_size().width - scroll_view.bounds_width();
        if (current_x - target_x).abs() <= 0.5 {
            return;
        }
        if reachable_x + 0.5 < target_x {
            if self.horizontal_pager_unreachable_traced != Some(selected_index) {
                self.horizontal_pager_unreachable_traced = Some(selected_index);
                tracing::warn!(
                    target: "riseonly::X1567",
                    selected_index,
                    page_count = self.horizontal_pager_page_count,
                    page_width,
                    current_x,
                    target_x,
                    reachable_x,
                    "pager rest target beyond the laid out strip"
                );
            }
            return;
        }
        self.horizontal_pager_unreachable_traced = None;
        tracing::warn!(
            target: "riseonly::X954",
            selected_index,
            coordinator_index = self.horizontal_pager_selected_index,
            page_count = self.horizontal_pager_page_count,
            page_width,
            current_x,
            target_x,
            reachable_x,
            "pager at rest off the selected page, re-seated"
        );
        self.horizontal_pager_selected_index = selected_index.max(0).min((self.horizontal_pager_page_count - 1).max(0));
        self.horizontal_drag_progress = 0.0;
        self.set_horizontal_pager_absolute_index(self.horizontal_pager_selected_index as f32, true, cx);
    }

    pub fn set_scope(&mut self, new_scope: &str, active_tab: ProfileContentTab, cx: &mut Context<Self>) {
        self.set_scope_key(new_scope, active_tab.raw_value(), cx);
    }

    pub fn set_scope_key(&mut self, new_scope: &str, active_tab_key: i64, cx: &mut Context<Self>) {
        let active_tab = active_tab_key;
        if self.scope == new_scope {
            self.set_active_tab_key(active_tab, cx);
            return;
        }

        self.pending_return = None;
        self.opened_master = None;
        self.return_flight_active = false;
        self.parked_outer_scroll_view = None;
        self.parked_collapse_range = None;
        self.parked_inner_scroll_views.clear();
        self.stop_vertical_motion_driver(cx);
        self.cancel_horizontal_settlement(cx);
        self.pending_active_tab_after_horizontal_settle = None;
        self.set_horizontal_drag_progress(0.0, false, cx);
        self.reset_deferred_geometry_refreshes();
        self.scope = new_scope.to_owned();
        self.active_tab = active_tab;
        if let Some(index) = self.visible_horizontal_tabs.iter().position(|tab| *tab == active_tab) {
            self.horizontal_pager_selected_index = index as i64;
            self.set_horizontal_pager_absolute_index(index as f32, true, cx);
            self.publish_tabs_indicator(0.0, cx);
        }
        self.prepared_horizontal_directions.clear();
        self.local_offset_by_tab.clear();
        self.outer_progress_by_tab.clear();
        self.inner_range_by_tab.clear();
        self.pending_proxy_height = None;
        self.proxy_height_update_generation = self.proxy_height_update_generation.wrapping_add(1);
        self.proxy_layout_generation = self.proxy_layout_generation.wrapping_add(1);
        self.awaiting_proxy_layout = false;
        self.awaiting_proxy_layout_generation = None;
        self.expected_proxy_outer_range = None;
        self.native_base_collapse_range = None;
        self.outer_progress_by_tab.insert(active_tab, 0.0);
        if self.uses_native_vertical_engine {
            self.pending_master_position =
                Some(ProfileVerticalMasterScrollPolicyPosition { collapse_progress: 0.0, inner_offset: 0.0 });
            self.pending_outer_progress = None;
            self.apply_active_proxy_height(0.0, cx);
        } else {
            self.pending_outer_progress = Some(0.0);
        }

        if let Some(outer) = self.outer_scroll_view.clone() {
            self.set_logical_offset(0.0, &outer, cx);
            self.refresh_outer(&outer, cx);
        }

        let tabs: Vec<i64> = self.inner_registrations.keys().copied().collect();
        for tab in tabs {
            if let Some(registration) = self.inner_registrations.get_mut(&tab) {
                registration.desired_logical_offset = Some(0.0);
            }
            if let Some(scroll_view) = self.registered_inner_scroll_view(tab) {
                self.refresh_inner(tab, &scroll_view, cx);
            }
        }
    }

    pub fn set_active_tab(&mut self, tab: ProfileContentTab, cx: &mut Context<Self>) {
        self.set_active_tab_key(tab.raw_value(), cx);
    }

    pub fn set_active_tab_key(&mut self, tab: i64, cx: &mut Context<Self>) {
        let target_index = self.visible_horizontal_tabs.iter().position(|visible| *visible == tab).map(|index| index as i64);
        match ProfileHorizontalPagerPolicy::semantic_selection_disposition(
            self.active_tab,
            self.pending_active_tab_after_horizontal_settle,
            tab,
            target_index,
            self.is_committing_horizontal_swipe,
            self.horizontal_settlement_active,
            self.horizontal_settlement_target_tab_index,
        ) {
            ProfileHorizontalPagerPolicySelectionDisposition::Unchanged => return,
            ProfileHorizontalPagerPolicySelectionDisposition::DeferUntilSettle => {
                self.pending_active_tab_after_horizontal_settle = Some(tab);
                return;
            }
            ProfileHorizontalPagerPolicySelectionDisposition::ReplaceImmediately => {}
        }

        self.cancel_horizontal_settlement(cx);
        self.pending_active_tab_after_horizontal_settle = None;
        if tab != self.active_tab {
            self.apply_active_tab_change(tab, true, cx);
        }
        let previous_selected_index = self.horizontal_pager_selected_index;
        self.horizontal_pager_selected_index = target_index.unwrap_or_else(|| self.traced_tab_index(tab));

        if self.horizontal_pan_is_active() {
            let shift = (previous_selected_index - self.horizontal_pager_selected_index) as f32;
            self.horizontal_pan_start_progress += shift;
            self.set_horizontal_drag_progress(self.horizontal_drag_progress + shift, true, cx);
            return;
        }

        self.horizontal_drag_progress = 0.0;
        self.set_horizontal_pager_absolute_index(self.horizontal_pager_selected_index as f32, true, cx);
        self.trace_horizontal_pager_selection(tab, target_index);
        self.publish_tabs_indicator(0.0, cx);
    }

    fn apply_active_tab_change(&mut self, tab: i64, preserving_current_semantic_position: bool, cx: &mut Context<Self>) {
        if tab == self.active_tab {
            return;
        }

        self.stop_vertical_motion_driver(cx);
        self.finish_vertical_motion(cx);
        let source_position = self.current_semantic_vertical_position();
        self.capture_active_position();
        self.active_tab = tab;

        let target_range = self
            .inner_range_by_tab
            .get(&tab)
            .copied()
            .or_else(|| {
                self.registered_inner_scroll_view(tab).map(|scroll_view| {
                    (self.maximum_offset(&scroll_view) - self.minimum_offset(&scroll_view)).max(0.0)
                })
            })
            .unwrap_or(if preserving_current_semantic_position { source_position.inner_offset.max(0.0) } else { 0.0 });
        let remembered = ProfileVerticalMasterScrollPolicyPosition {
            collapse_progress: self.outer_progress_by_tab.get(&tab).copied().unwrap_or(0.0),
            inner_offset: self.local_offset_by_tab.get(&tab).copied().unwrap_or(0.0),
        };
        let target_position = if preserving_current_semantic_position {
            ProfileVerticalMasterScrollPolicy::restored_tab_position(source_position, Some(remembered), target_range)
        } else {
            ProfileVerticalMasterScrollPolicyPosition {
                collapse_progress: remembered.collapse_progress,
                inner_offset: remembered.inner_offset.max(0.0).min(target_range),
            }
        };

        self.outer_progress_by_tab.insert(tab, target_position.collapse_progress);
        self.local_offset_by_tab.insert(tab, target_position.inner_offset);
        if let Some(scroll_view) = self.registered_inner_scroll_view(tab) {
            if let Some(registration) = self.inner_registrations.get_mut(&tab) {
                registration.desired_logical_offset = None;
            }
            self.set_logical_offset(target_position.inner_offset, &scroll_view, cx);
        }

        if self.uses_native_vertical_engine {
            self.pending_master_position = Some(target_position);
            self.pending_outer_progress = None;
            self.request_active_proxy_height(target_range, false, cx);
        } else {
            self.pending_outer_progress = Some(target_position.collapse_progress);
            if let Some(outer) = self.outer_scroll_view.clone() {
                self.refresh_outer(&outer, cx);
            }
        }
    }

    pub fn set_interaction_locked(&mut self, locked: bool, cx: &mut Context<Self>) {
        if self.interaction_locked == locked {
            return;
        }
        self.interaction_locked = locked;
        self.stop_vertical_motion_driver(cx);
        self.finish_vertical_motion(cx);
        self.cancel_horizontal_settlement(cx);
        self.finish_pending_horizontal_selection_if_needed(cx);
        self.set_horizontal_drag_progress(0.0, false, cx);
        if locked {
            self.disable_pan_recognizers(cx);
        }
        if let Some(pager) = self.horizontal_pager_scroll_view.as_ref() {
            pager.set_is_user_interaction_enabled(!locked);
        }
        if self.uses_native_vertical_engine
            && let Some(outer) = self.outer_scroll_view.as_ref()
        {
            outer.set_pan_enabled(!locked);
        }
        if !locked {
            if let Some(outer) = self.outer_scroll_view.clone() {
                self.enforce_outer_configuration(&outer);
            }
            if let Some(pager) = self.horizontal_pager_scroll_view.clone() {
                self.enforce_horizontal_pager_configuration(&pager);
            }
            self.enforce_all_inner_configurations();
        }
    }

    fn disable_pan_recognizers(&mut self, cx: &mut Context<Self>) {
        if self.horizontal_pan.as_ref().is_some_and(ProfileDirectionalPanGestureRecognizer::is_active) {
            self.handle_horizontal_pan(ProfileGestureState::Cancelled, 0.0, 0.0, cx);
        }
        if let Some(pan) = self.horizontal_pan.as_mut() {
            pan.reset();
        }
        if self.vertical_pan.as_ref().is_some_and(|pan| matches!(pan.state, ProfileGestureState::Began | ProfileGestureState::Changed)) {
            self.handle_vertical_pan(ProfileGestureState::Cancelled, 0.0, 0.0, cx);
        }
        if let Some(pan) = self.vertical_pan.as_mut() {
            pan.state = ProfileGestureState::Possible;
        }
    }

    pub fn pause_interaction(&mut self, cx: &mut Context<Self>) {
        self.stop_vertical_motion_driver(cx);
        self.finish_vertical_motion(cx);
        self.cancel_horizontal_settlement(cx);
        self.finish_pending_horizontal_selection_if_needed(cx);
        self.set_horizontal_drag_progress(0.0, false, cx);
    }

    pub fn is_idle_for_page_preload(&self) -> bool {
        let vertical_active = self
            .vertical_pan
            .as_ref()
            .is_some_and(|pan| matches!(pan.state, ProfileGestureState::Began | ProfileGestureState::Changed));
        let native_vertical_active = self.uses_native_vertical_engine
            && self.outer_scroll_view.as_ref().is_some_and(|outer| outer.is_tracking() || outer.is_decelerating());
        self.outer_scroll_view.is_some()
            && !self.interaction_locked
            && !vertical_active
            && !native_vertical_active
            && !self.horizontal_pan_is_active()
            && !self.vertical_motion_active
            && !self.display_link_running
            && !self.horizontal_settlement_active
            && !self.return_flight_active
    }

    pub fn has_outer_binding_for_page_preload(&self) -> bool {
        self.outer_scroll_view.is_some()
    }

    pub fn set_prepared_horizontal_directions(&mut self, directions: HashSet<i64>, _cx: &mut Context<Self>) {
        self.prepared_horizontal_directions = directions.into_iter().filter(|direction| direction.abs() == 1).collect();
    }

    pub fn set_horizontal_paging_top_inset(&mut self, inset: f32, _cx: &mut Context<Self>) {
        self.horizontal_paging_top_inset = Some(inset.max(0.0));
    }

    pub fn window_point_is_in_horizontal_paging_region(&self, point: Point<Pixels>) -> bool {
        let Some(pager) = self.horizontal_pager_scroll_view.as_ref() else {
            return true;
        };
        if !pager.is_in_window() {
            return false;
        }
        let frame = pager.frame();
        if !frame.contains(&point) {
            return false;
        }
        let Some(top_inset) = self.horizontal_paging_top_inset else {
            return true;
        };
        if f32::from(frame.size.height) <= 0.0 {
            return true;
        }
        f32::from(point.y) >= f32::from(frame.top()) + top_inset && f32::from(point.y) <= f32::from(frame.bottom())
    }

    pub fn underlap_routing_scroll_views(&self) -> Option<(ProfileScrollView, ProfileScrollView)> {
        Some((self.outer_scroll_view.clone()?, self.horizontal_pager_scroll_view.clone()?))
    }

    pub fn set_visible_horizontal_tabs(&mut self, tabs: &[ProfileContentTab], cx: &mut Context<Self>) {
        let keys: Vec<i64> = tabs.iter().map(|tab| tab.raw_value()).collect();
        self.set_visible_horizontal_tab_keys(keys, cx);
    }

    pub fn set_visible_horizontal_tab_keys(&mut self, tabs: Vec<i64>, cx: &mut Context<Self>) {
        self.visible_horizontal_tabs = if tabs.is_empty() { vec![self.active_tab] } else { tabs };
        let semantic_tab = self.pending_active_tab_after_horizontal_settle.unwrap_or(self.active_tab);
        let semantic_index = self.visible_horizontal_tabs.iter().position(|tab| *tab == semantic_tab);
        if let Some(index) = semantic_index {
            let index = index as i64;
            self.horizontal_pager_selected_index = index;
            if self.horizontal_settlement_active && self.pending_active_tab_after_horizontal_settle.is_some() {
                self.horizontal_settlement_target_tab_index = Some(index);
                self.horizontal_settlement_start_index = self.horizontal_pager_absolute_index(true);
                self.horizontal_settlement_target_index = index as f32;
            }
        } else {
            self.cancel_horizontal_settlement(cx);
            self.pending_active_tab_after_horizontal_settle = None;
            self.horizontal_pager_selected_index = self
                .visible_horizontal_tabs
                .iter()
                .position(|tab| *tab == self.active_tab)
                .map_or(0, |index| index as i64);
        }

        if self.horizontal_pan_is_active() || self.horizontal_settlement_active {
            return;
        }
        self.publish_tabs_indicator(0.0, cx);
    }

    fn outer_content_size_did_change(
        &mut self,
        scroll_view: &ProfileScrollView,
        old_size: Option<Size<f32>>,
        new_size: Option<Size<f32>>,
        cx: &mut Context<Self>,
    ) {
        if self.outer_scroll_view.as_ref() != Some(scroll_view) {
            return;
        }
        if self.uses_native_vertical_engine {
            if self.awaiting_proxy_layout
                && let Some(generation) = self.awaiting_proxy_layout_generation
            {
                let content_height = new_size.map_or_else(|| scroll_view.content_size().height, |size| size.height);
                let measured_range = self.outer_range(scroll_view, content_height);
                if let Some(expected) = self.expected_proxy_outer_range
                    && (measured_range - expected).abs() > 1.5
                {
                    self.schedule_proxy_layout_fallback(scroll_view, generation, 2, cx);
                    return;
                }
                self.complete_proxy_layout(scroll_view, measured_range, generation, cx);
                return;
            }

            if self.pending_master_position.is_none() {
                let old_height = old_size.map_or_else(|| scroll_view.content_size().height, |size| size.height);
                let old_outer_range = self.outer_range(scroll_view, old_height);
                let previous_collapse_range = self.native_base_collapse_range.unwrap_or_else(|| {
                    ProfileVerticalMasterScrollPolicy::collapse_range(
                        old_outer_range,
                        self.laid_out_proxy_height.unwrap_or(0.0),
                    )
                });
                let mapping = ProfileVerticalMasterScrollPolicy::mapping(
                    scroll_view.content_offset_y() - self.minimum_offset(scroll_view),
                    previous_collapse_range,
                    self.active_inner_logical_range(),
                );
                self.pending_master_position = Some(ProfileVerticalMasterScrollPolicyPosition {
                    collapse_progress: mapping.collapse_progress,
                    inner_offset: mapping.inner_offset,
                });
                self.reanchor_master_after_collapse_range_change(scroll_view, previous_collapse_range, cx);
            }
        }
        self.schedule_outer_refresh(scroll_view, cx);
    }

    fn reanchor_master_after_collapse_range_change(
        &mut self,
        scroll_view: &ProfileScrollView,
        previous_collapse_range: f32,
        cx: &mut Context<Self>,
    ) {
        let epsilon = ProfileVerticalMasterScrollPolicy::EPSILON;
        let Some(position) = self.pending_master_position.filter(|position| position.inner_offset > epsilon) else {
            return;
        };
        let collapse_range = self.native_collapse_range(scroll_view);
        if (collapse_range - previous_collapse_range).abs() <= epsilon {
            return;
        }

        let minimum = self.minimum_offset(scroll_view);
        let maximum = self.maximum_offset(scroll_view);
        let target = minimum
            + ProfileVerticalMasterScrollPolicy::master_offset(
                position,
                collapse_range,
                self.active_inner_logical_range(),
            );
        self.set_offset_y(target.max(minimum).min(maximum), scroll_view, cx);
        self.pending_master_position = None;
        self.synchronize_native_vertical_position(scroll_view, cx);
        self.reconcile_stale_vertical_motion("reanchor", cx);
    }

    pub fn proxy_height_did_layout(&mut self, height: f32, cx: &mut Context<Self>) {
        if !self.uses_native_vertical_engine
            || !height.is_finite()
            || height < 0.0
            || self.laid_out_proxy_height == Some(height)
        {
            return;
        }
        self.laid_out_proxy_height = Some(height);
        let Some(outer) = self.outer_scroll_view.clone() else {
            return;
        };
        if let Some(generation) = self.awaiting_proxy_layout_generation {
            self.main_async(cx, move |coordinator, cx| {
                if coordinator.outer_scroll_view.as_ref() != Some(&outer)
                    || coordinator.awaiting_proxy_layout_generation != Some(generation)
                {
                    return;
                }
                let measured_range = coordinator.outer_range(&outer, outer.content_size().height);
                if coordinator.expected_proxy_outer_range.is_some_and(|expected| (measured_range - expected).abs() > 1.5) {
                    coordinator.schedule_proxy_layout_fallback(&outer, generation, 2, cx);
                } else {
                    coordinator.complete_proxy_layout(&outer, measured_range, generation, cx);
                }
            });
        } else {
            self.schedule_outer_refresh(&outer, cx);
        }
    }

    fn complete_proxy_layout(
        &mut self,
        scroll_view: &ProfileScrollView,
        measured_outer_range: f32,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.outer_scroll_view.as_ref() != Some(scroll_view)
            || !self.awaiting_proxy_layout
            || self.awaiting_proxy_layout_generation != Some(generation)
            || !ProfileVerticalMasterScrollPolicy::proxy_layout_is_ready(
                self.applied_proxy_height,
                self.laid_out_proxy_height,
                measured_outer_range,
            )
        {
            return;
        }
        self.awaiting_proxy_layout = false;
        self.awaiting_proxy_layout_generation = None;
        self.expected_proxy_outer_range = None;
        self.native_base_collapse_range =
            Some((measured_outer_range - self.laid_out_proxy_height.unwrap_or(0.0)).max(0.0));

        if self.pending_proxy_height.is_some() {
            self.schedule_pending_proxy_height_application(cx);
        } else {
            self.schedule_outer_refresh(scroll_view, cx);
        }
        self.resume_pending_return_if_needed(cx);
    }

    fn schedule_proxy_layout_fallback(
        &self,
        scroll_view: &ProfileScrollView,
        generation: u64,
        remaining_attempts: i64,
        cx: &mut Context<Self>,
    ) {
        let scroll_view = scroll_view.clone();
        self.main_async_after(0.05, cx, move |coordinator, cx| {
            if coordinator.outer_scroll_view.as_ref() != Some(&scroll_view)
                || coordinator.awaiting_proxy_layout_generation != Some(generation)
            {
                return;
            }
            let measured_range = coordinator.outer_range(&scroll_view, scroll_view.content_size().height);
            if remaining_attempts > 0
                && coordinator.expected_proxy_outer_range.is_some_and(|expected| (measured_range - expected).abs() > 1.5)
            {
                coordinator.schedule_proxy_layout_fallback(&scroll_view, generation, remaining_attempts - 1, cx);
                return;
            }
            coordinator.complete_proxy_layout(&scroll_view, measured_range, generation, cx);
        });
    }

    pub fn schedule_outer_refresh(&mut self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        if self.outer_scroll_view.as_ref() != Some(scroll_view) {
            return;
        }
        if self.motion_blocks_geometry_refresh() {
            self.defer_outer_refresh();
            return;
        }
        if self.deferred_outer_refresh {
            self.schedule_deferred_geometry_refresh_flush(cx);
            return;
        }
        if self.outer_refresh_scheduled {
            return;
        }
        self.outer_refresh_scheduled = true;
        let scroll_view = scroll_view.clone();
        self.main_async(cx, move |coordinator, cx| {
            coordinator.outer_refresh_scheduled = false;
            if coordinator.outer_scroll_view.as_ref() != Some(&scroll_view) {
                return;
            }
            if coordinator.motion_blocks_geometry_refresh() {
                coordinator.defer_outer_refresh();
                return;
            }
            coordinator.refresh_outer(&scroll_view, cx);
        });
    }

    fn defer_outer_refresh(&mut self) {
        if !self.deferred_outer_refresh {
            ProfilePerformanceTrace::shared().record("geometry.outer_deferred", HashMap::new());
        }
        self.deferred_outer_refresh = true;
    }

    pub fn schedule_inner_refresh_for(
        &mut self,
        tab: ProfileContentTab,
        scroll_view: &ProfileScrollView,
        cx: &mut Context<Self>,
    ) {
        self.schedule_inner_refresh(tab.raw_value(), scroll_view, cx);
    }

    pub fn schedule_inner_refresh(&mut self, for_key: i64, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        let tab = for_key;
        if self.registered_inner_scroll_view(tab).as_ref() != Some(scroll_view) {
            return;
        }
        if self.motion_blocks_geometry_refresh() {
            self.defer_inner_refresh(tab);
            return;
        }
        if self.deferred_inner_refresh_tabs.contains(&tab) {
            self.schedule_deferred_geometry_refresh_flush(cx);
            return;
        }
        if !self.inner_refresh_scheduled.insert(tab) {
            return;
        }
        let scroll_view = scroll_view.clone();
        self.main_async(cx, move |coordinator, cx| {
            coordinator.inner_refresh_scheduled.remove(&tab);
            if coordinator.registered_inner_scroll_view(tab).as_ref() != Some(&scroll_view) {
                return;
            }
            if coordinator.motion_blocks_geometry_refresh() {
                coordinator.defer_inner_refresh(tab);
                return;
            }
            coordinator.refresh_inner(tab, &scroll_view, cx);
        });
    }

    fn defer_inner_refresh(&mut self, tab: i64) {
        if self.deferred_inner_refresh_tabs.insert(tab) {
            ProfilePerformanceTrace::shared()
                .record("geometry.inner_deferred", trace_fields([("tab_index", self.traced_tab_index(tab).to_string())]));
        }
    }

    fn can_horizontal_swipe(&self, direction: i64, cx: &App) -> bool {
        self.can_horizontal_swipe.as_ref().is_some_and(|can_swipe| can_swipe(direction, cx))
    }

    fn vertical_pan_should_begin(&self, velocity: Point<f32>, cx: &mut App) -> bool {
        let presenter = HoldContextMenuPresenter::shared(cx);
        if self.interaction_locked || presenter.read(cx).active_request().is_some() || self.outer_scroll_view.is_none() {
            return false;
        }
        if velocity.y.abs() < velocity.x.abs() || velocity.y.abs() <= 8.0 {
            return false;
        }
        self.can_move_vertically(-velocity.y)
    }

    /// decides if the horizontal pan may begin. a wheel step never presses the control under the
    /// pointer so no check for that
    fn horizontal_pan_should_begin(
        &self,
        location: Point<Pixels>,
        translation: Point<f32>,
        velocity: Point<f32>,
        cx: &mut App,
    ) -> bool {
        let presenter = HoldContextMenuPresenter::shared(cx);
        if self.interaction_locked || presenter.read(cx).active_request().is_some() || self.outer_scroll_view.is_none() {
            return false;
        }
        let intent_x = if translation.x.abs() >= ProfilePagerAxisLockPolicy::MINIMUM_INTENT_DISTANCE {
            translation.x
        } else {
            velocity.x
        };
        let direction: i64 = if intent_x < 0.0 { 1 } else { -1 };
        let can_reveal_page = self.can_horizontal_swipe(direction, cx);
        let can_pull_edge = self.horizontal_edge_directions().contains(&direction);
        let can_drive_toward_rest = self.horizontal_settlement_active
            && ProfileHorizontalPagerPolicy::moves_toward_rest(self.horizontal_drag_progress, direction);
        if !can_reveal_page && !can_pull_edge && !can_drive_toward_rest {
            return false;
        }
        if !self.window_point_is_in_horizontal_paging_region(location) {
            return false;
        }
        !self.touches_nested_horizontal_scroll(location)
    }

    fn touches_nested_horizontal_scroll(&self, point: Point<Pixels>) -> bool {
        self.axis_owners.owns(point)
    }

    pub fn outer_scroll_view_wheel(&mut self, event: &ScrollWheelEvent, hovered: bool, cx: &mut Context<Self>) -> bool {
        let Some(outer) = self.outer_scroll_view.clone() else {
            return false;
        };
        match event.delta {
            ScrollDelta::Lines(lines) => {
                if !hovered {
                    return false;
                }
                self.wheel_step(lines, event, &outer, cx)
            }
            ScrollDelta::Pixels(delta) => {
                let delta = point(f32::from(delta.x), f32::from(delta.y));
                match event.touch_phase {
                    TouchPhase::Started => {
                        if !hovered || self.interaction_locked {
                            return false;
                        }
                        outer.inner.momentum_suppressed.set(false);
                        if let Some(pan) = self.horizontal_pan.as_mut() {
                            pan.touches_began(RiseClock::monotonic());
                        }
                        self.stop_momentum_for_touch_down(cx);
                        self.wheel_moved(delta, event.position, &outer, cx)
                    }
                    TouchPhase::Moved => {
                        let tracked = self.horizontal_pan.as_ref().is_some_and(|pan| pan.has_tracked_touch);
                        if !hovered && !tracked {
                            return false;
                        }
                        self.wheel_moved(delta, event.position, &outer, cx)
                    }
                    TouchPhase::Ended => self.wheel_ended(&outer, ProfileGestureState::Ended, cx),
                    TouchPhase::Cancelled => self.wheel_ended(&outer, ProfileGestureState::Cancelled, cx),
                }
            }
        }
    }

    fn wheel_moved(
        &mut self,
        delta: Point<f32>,
        location: Point<Pixels>,
        outer: &ProfileScrollView,
        cx: &mut Context<Self>,
    ) -> bool {
        let tracked = self.horizontal_pan.as_ref().is_some_and(|pan| pan.has_tracked_touch);
        if !tracked {
            return self.wheel_without_gesture(delta, outer, cx);
        }
        let now = RiseClock::monotonic();
        let Some(pan) = self.horizontal_pan.as_mut() else {
            return false;
        };
        pan.touches_moved(delta, now);
        let (state, translation, velocity) = (pan.state, pan.translation, pan.current_velocity);
        match state {
            ProfileGestureState::Possible => match ProfilePagerAxisLockPolicy::early_axis(translation) {
                ProfilePagerAxisLockPolicyAxis::Undecided => true,
                ProfilePagerAxisLockPolicyAxis::Vertical => {
                    self.horizontal_pan_failed();
                    self.vertical_gesture_did_begin(velocity, outer, cx)
                }
                ProfilePagerAxisLockPolicyAxis::Horizontal => {
                    if !self.horizontal_pan_should_begin(location, translation, velocity, cx) {
                        self.horizontal_pan_failed();
                        return self.vertical_gesture_did_begin(velocity, outer, cx);
                    }
                    self.set_horizontal_pan_state(ProfileGestureState::Began);
                    self.handle_horizontal_pan(ProfileGestureState::Began, translation.x, velocity.x, cx);
                    self.set_horizontal_pan_state(ProfileGestureState::Changed);
                    self.handle_horizontal_pan(ProfileGestureState::Changed, translation.x, velocity.x, cx);
                    true
                }
            },
            ProfileGestureState::Began | ProfileGestureState::Changed => {
                self.set_horizontal_pan_state(ProfileGestureState::Changed);
                self.handle_horizontal_pan(ProfileGestureState::Changed, translation.x, velocity.x, cx);
                true
            }
            ProfileGestureState::Failed => self.vertical_gesture_did_move(delta, velocity, outer, cx),
            ProfileGestureState::Ended | ProfileGestureState::Cancelled => false,
        }
    }

    fn set_horizontal_pan_state(&mut self, state: ProfileGestureState) {
        if let Some(pan) = self.horizontal_pan.as_mut() {
            pan.state = state;
        }
    }

    fn horizontal_pan_failed(&mut self) {
        self.set_horizontal_pan_state(ProfileGestureState::Failed);
    }

    /// horizontal recognizer rejected the gesture so the vertical owner takes it
    fn vertical_gesture_did_begin(&mut self, velocity: Point<f32>, outer: &ProfileScrollView, cx: &mut Context<Self>) -> bool {
        if self.uses_native_vertical_engine {
            outer.set_tracking(true);
            outer.note_wheel();
            self.outer_scroll_view_will_begin_dragging(outer, velocity.y, cx);
            return false;
        }
        let may_begin = self.vertical_pan_should_begin(velocity, cx);
        let Some(pan) = self.vertical_pan.as_mut() else {
            return true;
        };
        if !may_begin {
            pan.state = ProfileGestureState::Failed;
            return true;
        }
        pan.state = ProfileGestureState::Began;
        self.vertical_last_translation = 0.0;
        self.handle_vertical_pan(ProfileGestureState::Began, 0.0, velocity.y, cx);
        true
    }

    fn vertical_gesture_did_move(
        &mut self,
        delta: Point<f32>,
        velocity: Point<f32>,
        outer: &ProfileScrollView,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.uses_native_vertical_engine {
            outer.note_wheel();
            return false;
        }
        let Some(state) = self.vertical_pan.as_ref().map(|pan| pan.state) else {
            return true;
        };
        if matches!(state, ProfileGestureState::Began | ProfileGestureState::Changed) {
            if let Some(pan) = self.vertical_pan.as_mut() {
                pan.state = ProfileGestureState::Changed;
            }
            let translation = self.vertical_last_translation + delta.y;
            self.handle_vertical_pan(ProfileGestureState::Changed, translation, velocity.y, cx);
        }
        true
    }

    fn wheel_ended(&mut self, outer: &ProfileScrollView, ended: ProfileGestureState, cx: &mut Context<Self>) -> bool {
        let Some(pan) = self.horizontal_pan.as_ref() else {
            return false;
        };
        if !pan.has_tracked_touch {
            return false;
        }
        let (state, translation, velocity) = (pan.state, pan.translation, pan.current_velocity);
        let consumed = match state {
            ProfileGestureState::Began | ProfileGestureState::Changed => {
                self.set_horizontal_pan_state(ended);
                self.handle_horizontal_pan(ended, translation.x, velocity.x, cx);
                true
            }
            ProfileGestureState::Failed => {
                self.vertical_gesture_did_end(velocity, outer, ended, cx);
                false
            }
            _ => false,
        };
        if let Some(pan) = self.horizontal_pan.as_mut() {
            pan.reset();
        }
        consumed
    }

    fn vertical_gesture_did_end(
        &mut self,
        velocity: Point<f32>,
        outer: &ProfileScrollView,
        ended: ProfileGestureState,
        cx: &mut Context<Self>,
    ) {
        if self.uses_native_vertical_engine {
            if !outer.is_tracking() {
                return;
            }
            outer.set_tracking(false);
            let will_decelerate = ended == ProfileGestureState::Ended && velocity.y.abs() >= Self::MOMENTUM_MINIMUM_VELOCITY;
            self.outer_scroll_view_will_end_dragging(outer, -velocity.y / 1_000.0);
            self.outer_scroll_view_did_end_dragging(outer, will_decelerate, cx);
            if will_decelerate {
                outer.note_wheel();
                self.outer_scroll_view_will_begin_decelerating(outer, cx);
            }
            return;
        }
        let Some(pan) = self.vertical_pan.as_mut() else {
            return;
        };
        let was_active = matches!(pan.state, ProfileGestureState::Began | ProfileGestureState::Changed);
        pan.state = ProfileGestureState::Possible;
        if was_active {
            self.handle_vertical_pan(ended, self.vertical_last_translation, velocity.y, cx);
        }
    }

    fn wheel_without_gesture(&mut self, delta: Point<f32>, outer: &ProfileScrollView, cx: &mut Context<Self>) -> bool {
        if outer.inner.momentum_suppressed.get() {
            return true;
        }
        if self.interaction_locked || delta.y == 0.0 {
            return false;
        }
        if self.uses_native_vertical_engine {
            outer.note_wheel();
            self.begin_vertical_motion(cx);
            return false;
        }
        true
    }

    fn wheel_step(
        &mut self,
        lines: Point<f32>,
        event: &ScrollWheelEvent,
        outer: &ProfileScrollView,
        cx: &mut Context<Self>,
    ) -> bool {
        let shift_turned = event.modifiers.shift && lines.x == 0.0;
        let x = if shift_turned { lines.y } else { lines.x };
        let y = if shift_turned { 0.0 } else { lines.y };
        if x != 0.0 && x.abs() >= y.abs() {
            let direction = x.signum();
            let velocity_x = direction * ProfileHorizontalPagerPolicy::VELOCITY_COMMIT_THRESHOLD;
            let translation = point(direction * ProfilePagerAxisLockPolicy::FORCED_DECISION_DISTANCE, 0.0);
            if !self.horizontal_pan_should_begin(event.position, translation, point(velocity_x, 0.0), cx) {
                return false;
            }
            self.set_horizontal_pan_state(ProfileGestureState::Began);
            self.handle_horizontal_pan(ProfileGestureState::Began, 0.0, velocity_x, cx);
            self.set_horizontal_pan_state(ProfileGestureState::Ended);
            self.handle_horizontal_pan(ProfileGestureState::Ended, 0.0, velocity_x, cx);
            if let Some(pan) = self.horizontal_pan.as_mut() {
                pan.reset();
            }
            return true;
        }
        if self.interaction_locked {
            return false;
        }
        if self.uses_native_vertical_engine {
            outer.inner.momentum_suppressed.set(false);
            outer.note_wheel();
            self.begin_vertical_motion(cx);
            return false;
        }
        let line_height = event.delta.pixel_delta(px(Self::LEGACY_WHEEL_LINE_HEIGHT)).y;
        self.begin_vertical_motion(cx);
        self.apply_vertical_delta_tracing(
            -f32::from(line_height),
            ProfilePerformanceGesturePhase::Changed,
            0.0,
            "wheel",
            cx,
        );
        self.finish_vertical_motion(cx);
        true
    }

    fn handle_vertical_pan(
        &mut self,
        state: ProfileGestureState,
        translation_y: f32,
        velocity_y: f32,
        cx: &mut Context<Self>,
    ) {
        if self.outer_scroll_view.is_none() {
            return;
        }
        match state {
            ProfileGestureState::Began => {
                self.begin_vertical_motion(cx);
                self.stop_momentum();
                self.vertical_last_translation = translation_y;
                self.update_vertical_trace(ProfilePerformanceGesturePhase::Began, velocity_y);
            }
            ProfileGestureState::Changed => {
                if self.interaction_locked {
                    return;
                }
                let delta = self.vertical_last_translation - translation_y;
                self.vertical_last_translation = translation_y;
                self.apply_vertical_delta_tracing(delta, ProfilePerformanceGesturePhase::Changed, velocity_y, "pan", cx);
            }
            ProfileGestureState::Ended => {
                if self.interaction_locked {
                    return;
                }
                if self.start_momentum(-velocity_y, cx) {
                    self.update_vertical_trace(ProfilePerformanceGesturePhase::Decelerating, velocity_y);
                } else {
                    self.update_vertical_trace(ProfilePerformanceGesturePhase::Ended, velocity_y);
                    self.finish_vertical_motion(cx);
                }
            }
            ProfileGestureState::Cancelled | ProfileGestureState::Failed => {
                self.stop_momentum();
                self.update_vertical_trace(
                    if state == ProfileGestureState::Failed {
                        ProfilePerformanceGesturePhase::Failed
                    } else {
                        ProfilePerformanceGesturePhase::Cancelled
                    },
                    0.0,
                );
                self.finish_vertical_motion(cx);
            }
            ProfileGestureState::Possible => {}
        }
    }

    fn handle_horizontal_pan(
        &mut self,
        state: ProfileGestureState,
        translation_x: f32,
        velocity_x: f32,
        cx: &mut Context<Self>,
    ) {
        if self.outer_scroll_view.is_none() {
            return;
        }
        match state {
            ProfileGestureState::Began => {
                self.stop_vertical_motion_driver(cx);
                self.finish_vertical_motion(cx);
                self.interrupt_horizontal_settlement(cx);
                self.synchronize_horizontal_reveal_targets(cx);
                if let Some(pager) = self.horizontal_pager_scroll_view.as_ref() {
                    pager.set_is_user_interaction_enabled(false);
                }
                self.horizontal_pan_start_progress = self.current_horizontal_progress();
                self.horizontal_drag_progress = self.horizontal_pan_start_progress;
                self.publish_tabs_indicator(self.horizontal_pan_start_progress, cx);
                self.horizontal_pan_start_translation_x = translation_x;
                self.horizontal_pan_available_directions = self.available_horizontal_directions(cx);
                self.update_horizontal_trace(ProfilePerformanceGesturePhase::Began, velocity_x, None);
            }
            ProfileGestureState::Changed => {
                if self.interaction_locked {
                    return;
                }
                let width = self.horizontal_pager_page_width.max(1.0);
                let translation = translation_x - self.horizontal_pan_start_translation_x;
                let proposed_progress = ProfileHorizontalPagerPolicy::interactive_progress(
                    self.horizontal_pan_start_progress,
                    translation,
                    width,
                );
                let available = self.available_horizontal_directions(cx);
                self.horizontal_pan_available_directions.extend(available);
                let progress = ProfileHorizontalPagerPolicy::constrained_interactive_progress(
                    proposed_progress,
                    self.horizontal_pan_start_progress,
                    &self.horizontal_pan_available_directions,
                    &self.horizontal_edge_directions(),
                );
                self.set_horizontal_drag_progress(progress, false, cx);
                self.update_horizontal_trace(ProfilePerformanceGesturePhase::Changed, velocity_x, None);
            }
            ProfileGestureState::Ended => {
                if self.interaction_locked {
                    return;
                }
                let width = self.horizontal_pager_page_width.max(1.0);
                let direction =
                    ProfileHorizontalPagerPolicy::direction(self.horizontal_drag_progress, velocity_x, width);
                let should_commit =
                    ProfileHorizontalPagerPolicy::should_commit(self.horizontal_drag_progress, velocity_x);
                let target_tab_index = self.traced_horizontal_transition(direction).map(|transition| transition.target_index);
                self.update_horizontal_trace(
                    ProfilePerformanceGesturePhase::Ended,
                    velocity_x,
                    if should_commit { target_tab_index } else { Some(self.horizontal_pager_selected_index) },
                );
                if should_commit
                    && ProfileHorizontalPagerPolicy::can_commit(direction, &self.horizontal_pan_available_directions)
                    && self.can_horizontal_swipe(direction, cx)
                {
                    self.commit_horizontal_swipe(direction, velocity_x, cx);
                } else {
                    let selected = self.horizontal_pager_selected_index;
                    self.settle_horizontal_drag_progress(false, velocity_x, None, Some(selected), cx);
                }
            }
            ProfileGestureState::Cancelled | ProfileGestureState::Failed => {
                self.update_horizontal_trace(
                    if state == ProfileGestureState::Failed {
                        ProfilePerformanceGesturePhase::Failed
                    } else {
                        ProfilePerformanceGesturePhase::Cancelled
                    },
                    0.0,
                    Some(self.horizontal_pager_selected_index),
                );
                if self.interaction_locked {
                    self.cancel_horizontal_settlement(cx);
                    self.finish_pending_horizontal_selection_if_needed(cx);
                    self.set_horizontal_drag_progress(0.0, true, cx);
                    if let Some(pager) = self.horizontal_pager_scroll_view.as_ref() {
                        pager.set_is_user_interaction_enabled(false);
                    }
                    return;
                }
                let selected = self.horizontal_pager_selected_index;
                self.settle_horizontal_drag_progress(false, 0.0, None, Some(selected), cx);
            }
            ProfileGestureState::Possible => {}
        }
    }

    fn commit_horizontal_swipe(&mut self, direction: i64, velocity_x: f32, cx: &mut Context<Self>) {
        self.cancel_horizontal_settlement(cx);
        let transition = self.traced_horizontal_transition(direction);
        let source_tab_index = transition.map_or_else(|| self.traced_active_tab_index(), |transition| transition.source_index);
        let target_tab_index = transition.map(|transition| transition.target_index).filter(|index| {
            *index >= 0 && (*index as usize) < self.visible_horizontal_tabs.len()
        });
        let Some(target_tab_index) = target_tab_index else {
            let selected = self.horizontal_pager_selected_index;
            self.settle_horizontal_drag_progress(false, velocity_x, None, Some(selected), cx);
            return;
        };
        let target_tab = self.visible_horizontal_tabs[target_tab_index as usize];
        self.horizontal_pager_selected_index = target_tab_index;
        self.pending_active_tab_after_horizontal_settle = (target_tab != self.active_tab).then_some(target_tab);
        self.horizontal_settlement_target_tab_index = Some(target_tab_index);
        self.is_committing_horizontal_swipe = true;
        if let Some(on_horizontal_swipe) = self.on_horizontal_swipe.clone() {
            cx.defer(move |cx| on_horizontal_swipe(direction, cx));
        }
        self.is_committing_horizontal_swipe = false;
        self.settle_horizontal_drag_progress(true, velocity_x, Some(source_tab_index), Some(target_tab_index), cx);
    }

    fn settle_horizontal_drag_progress(
        &mut self,
        after_continuity_frame: bool,
        velocity_x: f32,
        from_tab_index: Option<i64>,
        target_tab_index: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        self.stop_horizontal_settlement_driver();
        self.horizontal_settlement_generation = self.horizontal_settlement_generation.wrapping_add(1);
        let generation = self.horizontal_settlement_generation;
        self.horizontal_settlement_active = true;
        self.publish_tabs_indicator(0.0, cx);
        self.horizontal_settlement_target_tab_index = target_tab_index.or(Some(self.horizontal_pager_selected_index));
        if let Some(pager) = self.horizontal_pager_scroll_view.as_ref() {
            pager.set_is_user_interaction_enabled(false);
        }
        ProfilePerformanceTrace::shared().record_horizontal_settle_began(
            from_tab_index.unwrap_or_else(|| self.traced_active_tab_index()),
            self.horizontal_settlement_target_tab_index.unwrap_or(self.horizontal_pager_selected_index),
            self.horizontal_drag_progress,
            velocity_x,
        );

        if after_continuity_frame {
            self.main_async(cx, move |coordinator, cx| {
                if ProfileHorizontalPagerPolicy::accepts_settlement_callback(
                    generation,
                    coordinator.horizontal_settlement_generation,
                ) {
                    coordinator.start_horizontal_settlement_driver(generation, cx);
                }
            });
        } else {
            self.start_horizontal_settlement_driver(generation, cx);
        }
    }

    fn cancel_horizontal_settlement(&mut self, cx: &mut Context<Self>) {
        let was_active = self.horizontal_settlement_active;
        self.freeze_horizontal_pager_at_presentation_offset(cx);
        self.horizontal_settlement_generation = self.horizontal_settlement_generation.wrapping_add(1);
        self.horizontal_settlement_active = false;
        self.stop_horizontal_settlement_driver();
        self.horizontal_settlement_target_tab_index = None;
        if let Some(pager) = self.horizontal_pager_scroll_view.as_ref() {
            pager.set_is_user_interaction_enabled(!self.interaction_locked);
        }
        if was_active {
            ProfilePerformanceTrace::shared().record_horizontal_settle_ended(self.traced_active_tab_index(), true);
        }
        self.schedule_pending_proxy_height_application(cx);
        self.schedule_deferred_geometry_refresh_flush(cx);
    }

    fn interrupt_horizontal_settlement(&mut self, cx: &mut Context<Self>) {
        if !self.horizontal_settlement_active {
            self.horizontal_drag_progress = self.current_horizontal_progress();
            return;
        }
        self.cancel_horizontal_settlement(cx);
        self.finish_pending_horizontal_selection_if_needed(cx);
        self.horizontal_drag_progress = self.current_horizontal_progress();
    }

    /// sides the gesture may drive pages toward, tab must exist but not be mounted
    fn available_horizontal_directions(&self, cx: &App) -> HashSet<i64> {
        [-1_i64, 1].into_iter().filter(|direction| self.can_horizontal_swipe(*direction, cx)).collect()
    }

    fn synchronize_horizontal_reveal_targets(&mut self, cx: &mut Context<Self>) {
        let source_position = self.current_semantic_vertical_position();
        for direction in [-1_i64, 1] {
            if !self.prepared_horizontal_directions.contains(&direction) {
                continue;
            }
            let Some(transition) = self.traced_horizontal_transition(direction) else {
                continue;
            };
            let Some(target_tab) = self.visible_horizontal_tabs.get(transition.target_index as usize).copied() else {
                continue;
            };
            if target_tab == self.active_tab {
                continue;
            }
            let Some(scroll_view) = self.registered_inner_scroll_view(target_tab) else {
                continue;
            };

            let target_range = self
                .inner_range_by_tab
                .get(&target_tab)
                .copied()
                .unwrap_or_else(|| (self.maximum_offset(&scroll_view) - self.minimum_offset(&scroll_view)).max(0.0));
            let target_position = ProfileVerticalMasterScrollPolicy::restored_tab_position(
                source_position,
                Some(ProfileVerticalMasterScrollPolicyPosition {
                    collapse_progress: self.outer_progress_by_tab.get(&target_tab).copied().unwrap_or(0.0),
                    inner_offset: self.local_offset_by_tab.get(&target_tab).copied().unwrap_or(0.0),
                }),
                target_range,
            );
            if let Some(registration) = self.inner_registrations.get_mut(&target_tab) {
                registration.desired_logical_offset = None;
            }
            self.set_logical_offset(target_position.inner_offset, &scroll_view, cx);
        }
    }

    fn horizontal_pan_is_active(&self) -> bool {
        self.horizontal_pan.as_ref().is_some_and(ProfileDirectionalPanGestureRecognizer::is_active)
    }

    /// sides with no next tab at all. left of the first tab is excluded since the back swipe owns it
    fn horizontal_edge_directions(&self) -> HashSet<i64> {
        if self.traced_horizontal_transition(1).is_some() {
            return HashSet::new();
        }
        HashSet::from([1])
    }

    fn current_horizontal_progress(&self) -> f32 {
        self.horizontal_pager_absolute_index(true) - self.horizontal_pager_selected_index as f32
    }

    fn horizontal_pager_absolute_index(&self, use_presentation_layer: bool) -> f32 {
        let Some(scroll_view) = self.horizontal_pager_scroll_view.as_ref() else {
            return self.horizontal_pager_selected_index as f32 + self.horizontal_drag_progress;
        };
        self.horizontal_pager_absolute_index_in(scroll_view, self.horizontal_pager_page_width, use_presentation_layer)
    }

    fn horizontal_pager_absolute_index_in(
        &self,
        scroll_view: &ProfileScrollView,
        page_width: f32,
        use_presentation_layer: bool,
    ) -> f32 {
        let resting_index = ProfileHorizontalPagerPolicy::absolute_index(
            scroll_view.content_offset_x(),
            0.0,
            page_width,
            self.horizontal_pager_page_count,
        );
        let Some(motion_view) = self.horizontal_pager_motion_view.as_ref() else {
            return resting_index;
        };
        let transform =
            if use_presentation_layer { motion_view.presentation_transform_x() } else { motion_view.transform_x() };
        ProfileHorizontalPagerPolicy::visual_absolute_index(
            resting_index,
            transform - self.horizontal_pager_original_motion_transform,
            page_width,
            self.horizontal_pager_page_count,
        )
    }

    fn set_horizontal_pager_absolute_index(&mut self, absolute_index: f32, force: bool, cx: &mut Context<Self>) {
        let (Some(scroll_view), Some(motion_view)) =
            (self.horizontal_pager_scroll_view.clone(), self.horizontal_pager_motion_view.clone())
        else {
            return;
        };
        let target_x = ProfileHorizontalPagerPolicy::offset_x(
            absolute_index,
            0.0,
            self.horizontal_pager_page_width,
            self.horizontal_pager_page_count,
        );
        let has_motion = (motion_view.transform_x() - self.horizontal_pager_original_motion_transform).abs() > 0.1;
        if !force && !has_motion && (scroll_view.content_offset_x() - target_x).abs() <= 0.1 {
            return;
        }
        motion_view.remove_animation();
        scroll_view.set_content_offset(target_x, scroll_view.content_offset_y(), cx);
        motion_view.set_transform_x(self.horizontal_pager_original_motion_transform, cx);
    }

    fn set_horizontal_pager_visual_absolute_index(&mut self, absolute_index: f32, force: bool, cx: &mut Context<Self>) {
        let (Some(scroll_view), Some(motion_view)) =
            (self.horizontal_pager_scroll_view.clone(), self.horizontal_pager_motion_view.clone())
        else {
            return;
        };
        let current = self.horizontal_pager_absolute_index_in(&scroll_view, self.horizontal_pager_page_width, false);
        if !force && (current - absolute_index).abs() <= 0.0005 {
            return;
        }
        let resting_index = ProfileHorizontalPagerPolicy::absolute_index(
            scroll_view.content_offset_x(),
            0.0,
            self.horizontal_pager_page_width,
            self.horizontal_pager_page_count,
        );
        let translation_x = ProfileHorizontalPagerPolicy::motion_translation_x(
            absolute_index,
            resting_index,
            self.horizontal_pager_page_width,
        );
        motion_view.set_transform_x(self.horizontal_pager_original_motion_transform + translation_x, cx);
    }

    fn freeze_horizontal_pager_at_presentation_offset(&mut self, cx: &mut Context<Self>) {
        let (Some(scroll_view), Some(motion_view)) =
            (self.horizontal_pager_scroll_view.clone(), self.horizontal_pager_motion_view.clone())
        else {
            return;
        };
        let absolute_index = self.horizontal_pager_absolute_index_in(&scroll_view, self.horizontal_pager_page_width, true);
        motion_view.remove_animation();
        self.set_horizontal_pager_visual_absolute_index(absolute_index, true, cx);
        self.horizontal_drag_progress = absolute_index - self.horizontal_pager_selected_index as f32;
    }

    fn enforce_horizontal_pager_configuration(&self, scroll_view: &ProfileScrollView) {
        scroll_view.set_pan_enabled(false);
        scroll_view.set_bounces(false);
        scroll_view.set_scrolls_to_top(false);
        scroll_view.set_is_directional_lock_enabled(true);
    }

    fn unbind_horizontal_pager_current(&mut self, cx: &mut Context<Self>) {
        if self.horizontal_settlement_active {
            self.cancel_horizontal_settlement(cx);
            self.finish_pending_horizontal_selection_if_needed(cx);
        } else {
            self.freeze_horizontal_pager_at_presentation_offset(cx);
        }
        let Some(scroll_view) = self.horizontal_pager_scroll_view.clone() else {
            return;
        };
        let absolute_index = self.horizontal_pager_absolute_index_in(&scroll_view, self.horizontal_pager_page_width, false);
        self.stop_horizontal_settlement_driver();
        self.set_horizontal_pager_absolute_index(absolute_index, true, cx);
        scroll_view.set_pan_enabled(self.horizontal_pager_original_pan_enabled);
        scroll_view.set_bounces(self.horizontal_pager_original_bounces);
        scroll_view.set_scrolls_to_top(self.horizontal_pager_original_scrolls_to_top);
        scroll_view.set_is_directional_lock_enabled(self.horizontal_pager_original_directional_lock_enabled);
        scroll_view.set_is_user_interaction_enabled(true);
        self.horizontal_pager_scroll_view = None;
        self.horizontal_pager_motion_view = None;
        self.horizontal_pager_original_motion_transform = 0.0;
        self.horizontal_pager_page_width = 1.0;
        self.horizontal_pager_page_count = 0;
    }

    fn finish_pending_horizontal_selection_if_needed(&mut self, cx: &mut Context<Self>) {
        let Some(pending_tab) = self.pending_active_tab_after_horizontal_settle.take() else {
            return;
        };
        self.apply_active_tab_change(pending_tab, true, cx);
    }

    fn start_horizontal_settlement_driver(&mut self, generation: u64, cx: &mut Context<Self>) {
        if self.horizontal_settlement_generation != generation {
            return;
        }
        let (Some(scroll_view), Some(motion_view)) =
            (self.horizontal_pager_scroll_view.clone(), self.horizontal_pager_motion_view.clone())
        else {
            self.finish_horizontal_settlement(generation, cx);
            return;
        };
        self.horizontal_settlement_start_index = self.horizontal_pager_absolute_index(true);
        self.horizontal_settlement_target_index =
            self.horizontal_settlement_target_tab_index.unwrap_or(self.horizontal_pager_selected_index) as f32;
        if (self.horizontal_settlement_start_index - self.horizontal_settlement_target_index).abs() <= 0.0005 {
            self.finish_horizontal_settlement(generation, cx);
            return;
        }

        self.horizontal_drag_progress =
            self.horizontal_settlement_start_index - self.horizontal_pager_selected_index as f32;
        self.update_horizontal_trace(
            ProfilePerformanceGesturePhase::Settling,
            0.0,
            self.horizontal_settlement_target_tab_index,
        );
        let resting_index = ProfileHorizontalPagerPolicy::absolute_index(
            scroll_view.content_offset_x(),
            0.0,
            self.horizontal_pager_page_width,
            self.horizontal_pager_page_count,
        );
        let target_translation_x = ProfileHorizontalPagerPolicy::motion_translation_x(
            self.horizontal_settlement_target_index,
            resting_index,
            self.horizontal_pager_page_width,
        );
        let from_transform = motion_view.presentation_transform_x();
        let target_transform = self.horizontal_pager_original_motion_transform + target_translation_x;

        motion_view.remove_animation();
        motion_view.set_transform_x(target_transform, cx);
        motion_view.add_animation(
            from_transform,
            target_transform,
            ProfileHorizontalPagerPolicy::SETTLE_DURATION,
            AnimatedTabsMetrics::release_timing_function(),
            cx,
        );

        let timer = cx.background_executor().timer(Duration::from_secs_f64(ProfileHorizontalPagerPolicy::SETTLE_DURATION));
        self.horizontal_settlement_completion_work_item = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            timer.await;
            let _ = this.update(cx, |coordinator, cx| coordinator.finish_horizontal_settlement(generation, cx));
        }));
    }

    fn finish_horizontal_settlement(&mut self, generation: u64, cx: &mut Context<Self>) {
        if !self.horizontal_settlement_active
            || !ProfileHorizontalPagerPolicy::accepts_settlement_callback(
                generation,
                self.horizontal_settlement_generation,
            )
        {
            return;
        }
        self.horizontal_settlement_completion_work_item = None;
        self.set_horizontal_pager_absolute_index(self.horizontal_settlement_target_index, true, cx);
        self.horizontal_drag_progress = 0.0;
        self.horizontal_settlement_active = false;
        self.horizontal_pager_selected_index = self.horizontal_settlement_target_index.round() as i64;
        self.publish_tabs_indicator(0.0, cx);
        self.horizontal_settlement_target_tab_index = None;
        self.finish_pending_horizontal_selection_if_needed(cx);
        if let Some(pager) = self.horizontal_pager_scroll_view.as_ref() {
            pager.set_is_user_interaction_enabled(!self.interaction_locked);
        }
        ProfilePerformanceTrace::shared().record_horizontal_settle_ended(self.traced_active_tab_index(), false);
        self.schedule_pending_proxy_height_application(cx);
        self.schedule_deferred_geometry_refresh_flush(cx);
    }

    fn stop_horizontal_settlement_driver(&mut self) {
        self.horizontal_settlement_completion_work_item = None;
        if let Some(motion_view) = self.horizontal_pager_motion_view.as_ref() {
            motion_view.remove_animation();
        }
    }

    fn set_horizontal_drag_progress(&mut self, progress: f32, force: bool, cx: &mut Context<Self>) {
        if force {
            if self.horizontal_drag_progress == progress {
                return;
            }
        } else if (self.horizontal_drag_progress - progress).abs() <= 0.0005 {
            return;
        }
        self.horizontal_drag_progress = progress;
        self.publish_tabs_indicator(progress, cx);
        self.set_horizontal_pager_visual_absolute_index(self.horizontal_pager_selected_index as f32 + progress, force, cx);
    }

    /// only place the header pill learns of the pages
    /// base and progress written together, half a pair renders a false state
    /// base is the pager selected index
    fn publish_tabs_indicator(&self, progress: f32, cx: &mut Context<Self>) {
        let base_index = self.horizontal_pager_selected_index;
        self.tabs_live_state.update(cx, |live_state, cx| {
            live_state.publish_external_drag_progress(progress, base_index, cx);
        });
    }

    fn display_link_did_fire(&mut self, timestamp: f64, range: RiseFrameRateRange, cx: &mut Context<Self>) {
        if self.interaction_locked {
            self.stop_momentum();
            self.finish_vertical_motion(cx);
            return;
        }

        if self.previous_display_link_timestamp == 0.0 {
            self.previous_display_link_timestamp = timestamp;
            return;
        }

        let since_previous = timestamp - self.previous_display_link_timestamp;
        if since_previous < 1.0 / f64::from(range.preferred) * Self::DISPLAY_LINK_CADENCE_TOLERANCE {
            return;
        }
        let elapsed = since_previous.max(1.0 / 240.0).min(1.0 / 30.0);
        self.previous_display_link_timestamp = timestamp;
        let proposed = self.deceleration_velocity * elapsed as f32;
        let consumed = self.apply_vertical_delta_tracing(
            proposed,
            ProfilePerformanceGesturePhase::Decelerating,
            -self.deceleration_velocity,
            "deceleration",
            cx,
        );

        if consumed.abs() < 0.1 && proposed.abs() > 0.1 {
            self.stop_momentum();
            self.update_vertical_trace(ProfilePerformanceGesturePhase::Ended, 0.0);
            self.finish_vertical_motion(cx);
            return;
        }

        let decay = f64::from(PROFILE_DECELERATION_RATE_NORMAL).powf(elapsed * 1_000.0);
        self.deceleration_velocity *= decay as f32;
        if self.deceleration_velocity.abs() < 8.0 {
            self.stop_momentum();
            self.update_vertical_trace(ProfilePerformanceGesturePhase::Ended, 0.0);
            self.finish_vertical_motion(cx);
        }
    }

    fn refresh_outer(&mut self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        if self.outer_scroll_view.as_ref() != Some(scroll_view) {
            return;
        }
        let trace = ProfilePerformanceTrace::shared();
        let trace_token = trace.begin_operation("geometry.outer_refresh", HashMap::new);
        self.refresh_outer_traced(scroll_view, cx);
        trace.end_operation(trace_token, Self::TRACE_THRESHOLD_MS, || {
            trace_fields([("offset_y", scroll_view.content_offset_y().to_string())])
        });
    }

    fn refresh_outer_traced(&mut self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        self.enforce_outer_configuration(scroll_view);

        let minimum = self.minimum_offset(scroll_view);
        let maximum = self.maximum_offset(scroll_view);
        if self.uses_native_vertical_engine {
            self.last_outer_minimum = Some(minimum);
            self.last_outer_maximum = Some(maximum);

            if self.awaiting_proxy_layout {
                return;
            }

            self.native_base_collapse_range =
                Some((maximum - minimum - self.laid_out_proxy_height.unwrap_or(0.0)).max(0.0));

            if let Some(pending_master_position) = self.pending_master_position {
                let collapse_range = self.native_collapse_range(scroll_view);
                let target = minimum
                    + ProfileVerticalMasterScrollPolicy::master_offset(
                        pending_master_position,
                        collapse_range,
                        self.active_inner_logical_range(),
                    );
                self.set_offset_y(target.max(minimum).min(maximum), scroll_view, cx);
                self.pending_master_position = None;
            } else if !scroll_view.is_tracking() && !scroll_view.is_decelerating() {
                self.clamp_offset(scroll_view, cx);
            }

            self.synchronize_native_vertical_position(scroll_view, cx);
            return;
        }

        let previous_minimum = self.last_outer_minimum;
        let previous_maximum = self.last_outer_maximum;
        let was_pinned = match (previous_minimum, previous_maximum) {
            (Some(previous_minimum), Some(previous_maximum)) if previous_maximum - previous_minimum > 1.0 => {
                (scroll_view.content_offset_y() - previous_maximum).abs() <= 1.5
            }
            _ => false,
        };

        self.last_outer_minimum = Some(minimum);
        self.last_outer_maximum = Some(maximum);

        if let Some(pending_outer_progress) = self.pending_outer_progress {
            let range = maximum - minimum;
            if pending_outer_progress <= 0.001 {
                self.set_offset_y(minimum, scroll_view, cx);
                self.pending_outer_progress = None;
            } else if range > 1.0 {
                self.set_offset_y(minimum + range * pending_outer_progress.max(0.0).min(1.0), scroll_view, cx);
                self.pending_outer_progress = None;
            }
        } else if was_pinned && previous_maximum.is_some_and(|previous_maximum| (previous_maximum - maximum).abs() > 0.5) {
            self.set_offset_y(maximum, scroll_view, cx);
        } else {
            self.clamp_offset(scroll_view, cx);
        }
    }

    fn refresh_inner(&mut self, tab: i64, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        if self.registered_inner_scroll_view(tab).as_ref() != Some(scroll_view) {
            return;
        }
        let trace = ProfilePerformanceTrace::shared();
        let trace_token = trace.begin_operation("geometry.inner_refresh", || {
            trace_fields([("tab_index", self.traced_tab_index(tab).to_string())])
        });
        self.refresh_inner_traced(tab, scroll_view, cx);
        trace.end_operation(trace_token, Self::TRACE_THRESHOLD_MS, || {
            trace_fields([("offset_y", scroll_view.content_offset_y().to_string())])
        });
    }

    fn refresh_inner_traced(&mut self, tab: i64, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        self.enforce_inner_configuration(tab);

        let minimum = self.minimum_offset(scroll_view);
        let maximum = self.maximum_offset(scroll_view);
        let range = maximum - minimum;
        self.inner_range_by_tab.insert(tab, range.max(0.0));

        let desired =
            self.inner_registrations.get(&tab).and_then(|registration| registration.desired_logical_offset);
        if let Some(desired) = desired.or_else(|| self.held_offset(tab)) {
            let clamped = desired.max(0.0).min(range);
            self.set_offset_y(minimum + clamped, scroll_view, cx);
            if scroll_view.bounds_height() > 1.0
                && scroll_view.content_size().height > 1.0
                && let Some(registration) = self.inner_registrations.get_mut(&tab)
            {
                registration.desired_logical_offset = None;
            }
        } else {
            self.clamp_offset(scroll_view, cx);
        }

        self.local_offset_by_tab.insert(tab, self.logical_offset(scroll_view));
        if self.uses_native_vertical_engine && tab == self.active_tab {
            self.request_active_proxy_height(range, true, cx);
        }
    }

    fn hold_active_inner_offset(&mut self, cx: &mut Context<Self>) {
        let tab = self.active_tab;
        let Some(scroll_view) = self.registered_inner_scroll_view(tab) else {
            return;
        };
        if self.motion_blocks_geometry_refresh() {
            return;
        }
        let offset = if self.uses_native_vertical_engine {
            self.current_native_master_position().inner_offset
        } else {
            self.logical_offset(&scroll_view)
        };
        self.local_offset_by_tab.insert(tab, offset);
        self.held_inner_offset = Some((tab, offset, ca_current_media_time() + Self::INNER_OFFSET_HOLD_DURATION));
        for delay in Self::INNER_OFFSET_HOLD_RESTORE_DELAYS {
            self.main_async_after(delay, cx, |coordinator, cx| coordinator.restore_held_inner_offset(cx));
        }
    }

    fn held_offset(&mut self, tab: i64) -> Option<f32> {
        let (held_tab, offset, until) = self.held_inner_offset?;
        if held_tab != tab {
            return None;
        }
        if tab != self.active_tab || self.vertical_motion_active || ca_current_media_time() > until {
            self.held_inner_offset = None;
            return None;
        }
        Some(offset)
    }

    fn restore_held_inner_offset(&mut self, cx: &mut Context<Self>) {
        let Some((tab, _, _)) = self.held_inner_offset else {
            return;
        };
        if self.held_offset(tab).is_none() {
            return;
        }
        let Some(scroll_view) = self.registered_inner_scroll_view(tab) else {
            return;
        };
        if self.motion_blocks_geometry_refresh() {
            return;
        }
        self.refresh_inner(tab, &scroll_view, cx);
    }

    fn apply_vertical_delta_tracing(
        &mut self,
        delta: f32,
        phase: ProfilePerformanceGesturePhase,
        velocity_y: f32,
        source: &str,
        cx: &mut Context<Self>,
    ) -> f32 {
        let trace = ProfilePerformanceTrace::shared();
        if !trace.is_recording_any_scope() {
            return self.apply_vertical_delta(delta, cx);
        }

        let outer_before = self.outer_scroll_view.as_ref().map_or(0.0, ProfileScrollView::content_offset_y);
        let inner_before = self.active_inner_scroll_view().map_or(0.0, |inner| inner.content_offset_y());
        let token = trace.begin_operation("vertical.apply", || trace_fields([("source", source.to_owned())]));
        let consumed = self.apply_vertical_delta(delta, cx);
        let outer_after = self.outer_scroll_view.as_ref().map_or(0.0, ProfileScrollView::content_offset_y);
        let inner_after = self.active_inner_scroll_view().map_or(0.0, |inner| inner.content_offset_y());

        trace.update_vertical_context(
            phase,
            Self::vertical_trace_direction(velocity_y),
            velocity_y,
            outer_after,
            inner_after,
            self.traced_active_tab_index(),
            ProfilePerformanceVerticalGeometry::default(),
        );
        if (outer_after - outer_before).abs() > 0.01 && (inner_after - inner_before).abs() > 0.01 {
            trace.record_with("vertical.handoff", || {
                trace_fields([
                    ("source", source.to_owned()),
                    ("delta", delta.to_string()),
                    ("consumed", consumed.to_string()),
                ])
            });
        }
        trace.end_operation(token, Self::TRACE_THRESHOLD_MS, || {
            trace_fields([("delta", delta.to_string()), ("consumed", consumed.to_string())])
        });
        consumed
    }

    fn apply_vertical_delta(&mut self, delta: f32, cx: &mut Context<Self>) -> f32 {
        if delta.abs() <= 0.001 {
            return 0.0;
        }

        let mut remaining = delta;
        let mut consumed = 0.0;
        let outer = self.outer_scroll_view.clone();
        let inner = self.active_inner_scroll_view();

        if delta > 0.0 {
            if let Some(outer) = outer.as_ref() {
                let amount = self.consume(remaining, outer, cx);
                remaining -= amount;
                consumed += amount;
            }
            if remaining > 0.001
                && let Some(inner) = inner.as_ref()
            {
                let amount = self.consume(remaining, inner, cx);
                consumed += amount;
            }
        } else {
            if let Some(inner) = inner.as_ref() {
                let amount = self.consume(remaining, inner, cx);
                remaining -= amount;
                consumed += amount;
            }
            if remaining < -0.001
                && let Some(outer) = outer.as_ref()
            {
                let amount = self.consume(remaining, outer, cx);
                consumed += amount;
            }
        }

        if consumed != 0.0
            && let Some(on_content_scrolled) = self.on_content_scrolled.clone()
        {
            on_content_scrolled(consumed, cx);
        }
        consumed
    }

    fn consume(&self, delta: f32, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) -> f32 {
        let minimum = self.minimum_offset(scroll_view);
        let maximum = self.maximum_offset(scroll_view);
        let current = scroll_view.content_offset_y().max(minimum).min(maximum);
        let target = (current + delta).max(minimum).min(maximum);
        self.set_offset_y(target, scroll_view, cx);
        target - current
    }

    fn can_move_vertically(&self, content_delta: f32) -> bool {
        let outer = self.outer_scroll_view.as_ref();
        let inner = self.active_inner_scroll_view();
        if content_delta > 0.0 {
            if outer.is_some_and(|outer| outer.content_offset_y() < self.maximum_offset(outer) - 0.5) {
                return true;
            }
            if inner.as_ref().is_some_and(|inner| inner.content_offset_y() < self.maximum_offset(inner) - 0.5) {
                return true;
            }
        } else if content_delta < 0.0 {
            if inner.as_ref().is_some_and(|inner| inner.content_offset_y() > self.minimum_offset(inner) + 0.5) {
                return true;
            }
            if outer.is_some_and(|outer| outer.content_offset_y() > self.minimum_offset(outer) + 0.5) {
                return true;
            }
        }
        false
    }

    fn start_momentum(&mut self, velocity: f32, cx: &mut Context<Self>) -> bool {
        self.stop_momentum();
        if velocity.abs() < Self::MOMENTUM_MINIMUM_VELOCITY || !self.can_move_vertically(velocity) {
            return false;
        }
        self.deceleration_velocity = velocity;
        self.previous_display_link_timestamp = 0.0;
        self.display_link_running = true;
        if let Some(outer) = self.outer_scroll_view.as_ref() {
            outer.request_redraw(cx);
        }
        true
    }

    fn stop_momentum(&mut self) {
        self.display_link_running = false;
        self.deceleration_velocity = 0.0;
        self.previous_display_link_timestamp = 0.0;
    }

    fn stop_vertical_motion_driver(&mut self, cx: &mut Context<Self>) {
        if !self.uses_native_vertical_engine {
            self.stop_momentum();
            return;
        }
        let Some(scroll_view) = self.outer_scroll_view.clone() else {
            return;
        };
        if scroll_view.is_tracking() || !(scroll_view.is_decelerating() || self.vertical_motion_active) {
            return;
        }

        scroll_view.stop_scrolling_and_zooming();
        self.synchronize_native_vertical_position(&scroll_view, cx);
        self.capture_active_position();
        self.update_vertical_trace(ProfilePerformanceGesturePhase::Cancelled, 0.0);
        self.last_native_scroll_timestamp = 0.0;
        self.native_release_velocity_y = 0.0;
    }

    fn stop_momentum_for_touch_down(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.display_link_running {
            return false;
        }
        self.stop_momentum();
        self.update_vertical_trace(ProfilePerformanceGesturePhase::Cancelled, 0.0);
        self.finish_vertical_motion(cx);
        true
    }

    fn motion_blocks_geometry_refresh(&self) -> bool {
        self.return_flight_active
            || ProfileGeometryRefreshPolicy::should_defer(
                self.vertical_motion_active,
                self.horizontal_pan_is_active(),
                self.horizontal_settlement_active,
            )
    }

    fn begin_vertical_motion(&mut self, cx: &mut Context<Self>) {
        if self.vertical_motion_active {
            return;
        }
        self.vertical_motion_active = true;
        self.geometry_refresh_generation = self.geometry_refresh_generation.wrapping_add(1);
        self.scheduled_geometry_refresh_generation = None;
        self.arm_vertical_motion_watchdog(cx);
    }

    /// nothing announces an aborted fling end, so a motion flag stuck raised
    /// stuck flag deferred the geometry refresh
    fn reconcile_stale_vertical_motion(&mut self, reason: &str, cx: &mut Context<Self>) -> bool {
        if !self.uses_native_vertical_engine || !self.vertical_motion_active || self.status_bar_scroll_to_top_active {
            return false;
        }
        let Some(scroll_view) = self.outer_scroll_view.clone() else {
            return false;
        };
        if scroll_view.is_tracking() || scroll_view.is_decelerating() {
            return false;
        }
        ProfilePerformanceTrace::shared().record_with("vertical.stale_motion", || {
            trace_fields([
                ("reason", reason.to_owned()),
                ("offset_y", scroll_view.content_offset_y().to_string()),
                ("deferred_outer", if self.deferred_outer_refresh { "1" } else { "0" }.to_owned()),
                ("deferred_inner", self.deferred_inner_refresh_tabs.len().to_string()),
                ("pending_proxy", self.pending_proxy_height.map_or_else(|| "none".to_owned(), |height| height.to_string())),
                ("applied_proxy", self.applied_proxy_height.to_string()),
            ])
        });
        self.complete_native_vertical_motion(&scroll_view, cx);
        true
    }

    /// while motion is raised, checks the flag against the scroll view once per idle window
    /// scroll to top animation is excluded by a flag, not by view state
    fn arm_vertical_motion_watchdog(&mut self, cx: &mut Context<Self>) {
        if !self.uses_native_vertical_engine || !self.vertical_motion_active {
            return;
        }
        self.vertical_motion_watchdog_generation = self.vertical_motion_watchdog_generation.wrapping_add(1);
        let generation = self.vertical_motion_watchdog_generation;
        self.main_async_after(ProfileGeometryRefreshPolicy::STABLE_IDLE_DELAY, cx, move |coordinator, cx| {
            if coordinator.vertical_motion_watchdog_generation != generation || !coordinator.vertical_motion_active {
                return;
            }
            if !coordinator.reconcile_stale_vertical_motion("watchdog", cx) {
                coordinator.arm_vertical_motion_watchdog(cx);
            }
        });
    }

    fn finish_vertical_motion(&mut self, cx: &mut Context<Self>) {
        if self.uses_native_vertical_engine
            && self.vertical_motion_active
            && (self.deferred_outer_refresh || self.awaiting_proxy_layout)
        {
            self.pending_master_position = Some(self.current_native_master_position());
        }
        if self.vertical_motion_active {
            self.vertical_motion_active = false;
            self.vertical_motion_watchdog_generation = self.vertical_motion_watchdog_generation.wrapping_add(1);
            self.geometry_refresh_generation = self.geometry_refresh_generation.wrapping_add(1);
            self.scheduled_geometry_refresh_generation = None;
        }
        self.status_bar_scroll_to_top_active = false;
        if self.uses_native_vertical_engine {
            self.schedule_pending_proxy_height_application(cx);
        }
        self.schedule_deferred_geometry_refresh_flush(cx);
    }

    fn schedule_deferred_geometry_refresh_flush(&mut self, cx: &mut Context<Self>) {
        if !self.deferred_outer_refresh && self.deferred_inner_refresh_tabs.is_empty() {
            return;
        }
        if self.motion_blocks_geometry_refresh() || self.scheduled_geometry_refresh_generation.is_some() {
            return;
        }

        let generation = self.geometry_refresh_generation;
        self.scheduled_geometry_refresh_generation = Some(generation);
        self.main_async_after(ProfileGeometryRefreshPolicy::STABLE_IDLE_DELAY, cx, move |coordinator, cx| {
            if coordinator.scheduled_geometry_refresh_generation != Some(generation) {
                return;
            }
            coordinator.scheduled_geometry_refresh_generation = None;
            if !ProfileGeometryRefreshPolicy::accepts_flush(generation, coordinator.geometry_refresh_generation)
                || coordinator.motion_blocks_geometry_refresh()
            {
                return;
            }

            let refresh_outer = coordinator.deferred_outer_refresh;
            let tabs: Vec<i64> = coordinator.deferred_inner_refresh_tabs.drain().collect();
            coordinator.deferred_outer_refresh = false;
            ProfilePerformanceTrace::shared().record(
                "geometry.deferred_flush",
                trace_fields([
                    ("outer", if refresh_outer { "1" } else { "0" }.to_owned()),
                    ("inner_count", tabs.len().to_string()),
                ]),
            );

            if refresh_outer && let Some(outer) = coordinator.outer_scroll_view.clone() {
                coordinator.refresh_outer(&outer, cx);
            }
            for tab in tabs {
                let Some(scroll_view) = coordinator.registered_inner_scroll_view(tab) else {
                    continue;
                };
                coordinator.refresh_inner(tab, &scroll_view, cx);
            }
        });
    }

    fn reset_deferred_geometry_refreshes(&mut self) {
        self.vertical_motion_active = false;
        self.vertical_motion_watchdog_generation = self.vertical_motion_watchdog_generation.wrapping_add(1);
        self.status_bar_scroll_to_top_active = false;
        self.geometry_refresh_generation = self.geometry_refresh_generation.wrapping_add(1);
        self.scheduled_geometry_refresh_generation = None;
        self.deferred_outer_refresh = false;
        self.deferred_inner_refresh_tabs.clear();
    }

    fn update_vertical_trace(&mut self, phase: ProfilePerformanceGesturePhase, velocity_y: f32) {
        let trace = ProfilePerformanceTrace::shared();
        if !trace.is_recording_any_scope() {
            return;
        }
        let geometry = self.traced_vertical_geometry();
        trace.update_vertical_context(
            phase,
            Self::vertical_trace_direction(velocity_y),
            velocity_y,
            self.outer_scroll_view.as_ref().map_or(0.0, ProfileScrollView::content_offset_y),
            self.active_inner_scroll_view().map_or(0.0, |inner| inner.content_offset_y()),
            self.traced_active_tab_index(),
            geometry,
        );
    }

    fn traced_vertical_geometry(&mut self) -> ProfilePerformanceVerticalGeometry {
        let mut geometry = ProfilePerformanceVerticalGeometry {
            applied_proxy_height: self.applied_proxy_height,
            observed_proxy_height: self.active_proxy_height,
            laid_out_proxy_height: self.laid_out_proxy_height,
            inner_range: self.active_inner_logical_range(),
            awaiting_proxy_layout: self.awaiting_proxy_layout,
            vertical_motion_active: self.vertical_motion_active,
            deferred_refreshes: i64::from(self.deferred_outer_refresh) + self.deferred_inner_refresh_tabs.len() as i64,
            inner_registered: self.active_inner_scroll_view().is_some(),
            ..ProfilePerformanceVerticalGeometry::default()
        };
        if self.uses_native_vertical_engine
            && let Some(outer) = self.outer_scroll_view.clone()
        {
            let collapse_range = self.native_collapse_range(&outer);
            let mapping = ProfileVerticalMasterScrollPolicy::mapping(
                outer.content_offset_y() - self.minimum_offset(&outer),
                collapse_range,
                self.active_inner_logical_range(),
            );
            geometry.collapse_range = collapse_range;
            geometry.expected_pin = mapping.pin_correction;
            geometry.outer_range = self.maximum_offset(&outer) - self.minimum_offset(&outer);
        }
        let sample = ProfileMasterPinningProbe::latest();
        geometry.visual_pin = sample.correction;
        geometry.visual_min_y = sample.min_y;
        geometry.visual_proxy_height = sample.proxy_height;
        geometry.visual_pinned_tabs_top = sample.pinned_tabs_top;
        geometry.visual_sequence = sample.sequence;
        geometry
    }

    fn update_horizontal_trace(
        &self,
        phase: ProfilePerformanceGesturePhase,
        velocity_x: f32,
        target_tab_index: Option<i64>,
    ) {
        let trace = ProfilePerformanceTrace::shared();
        if !trace.is_recording_any_scope() {
            return;
        }
        trace.update_horizontal_context(
            phase,
            Self::horizontal_trace_direction(velocity_x),
            velocity_x,
            self.horizontal_drag_progress,
            self.traced_active_tab_index(),
            target_tab_index,
        );
    }

    fn vertical_trace_direction(velocity_y: f32) -> ProfilePerformanceGestureDirection {
        if velocity_y > 0.5 {
            return ProfilePerformanceGestureDirection::Down;
        }
        if velocity_y < -0.5 {
            return ProfilePerformanceGestureDirection::Up;
        }
        ProfilePerformanceGestureDirection::None
    }

    fn horizontal_trace_direction(velocity_x: f32) -> ProfilePerformanceGestureDirection {
        if velocity_x > 0.5 {
            return ProfilePerformanceGestureDirection::Right;
        }
        if velocity_x < -0.5 {
            return ProfilePerformanceGestureDirection::Left;
        }
        ProfilePerformanceGestureDirection::None
    }

    fn traced_active_tab_index(&self) -> i64 {
        self.traced_tab_index(self.active_tab)
    }

    /// tab key is not its position, keys have gaps since the visible set depends on chat type
    fn traced_tab_index(&self, tab: i64) -> i64 {
        if let Some(index) = self.visible_horizontal_tabs.iter().position(|visible| *visible == tab) {
            return index as i64;
        }
        tab.max(0).min((self.visible_horizontal_tabs.len() as i64 - 1).max(0))
    }

    fn traced_horizontal_transition(&self, direction: i64) -> Option<ProfileHorizontalPagerPolicyTabTransition> {
        let current_tab = usize::try_from(self.horizontal_pager_selected_index)
            .ok()
            .and_then(|index| self.visible_horizontal_tabs.get(index))
            .copied()
            .unwrap_or(self.active_tab);
        ProfileHorizontalPagerPolicy::tab_transition(&current_tab, direction, &self.visible_horizontal_tabs)
    }

    fn active_inner_logical_range(&self) -> f32 {
        if let Some(cached) = self.inner_range_by_tab.get(&self.active_tab) {
            return cached.max(0.0);
        }
        let Some(inner) = self.active_inner_scroll_view() else {
            return 0.0;
        };
        (self.maximum_offset(&inner) - self.minimum_offset(&inner)).max(0.0)
    }

    fn native_collapse_range(&mut self, scroll_view: &ProfileScrollView) -> f32 {
        let measured = ProfileVerticalMasterScrollPolicy::collapse_range(
            self.maximum_offset(scroll_view) - self.minimum_offset(scroll_view),
            self.laid_out_proxy_height.unwrap_or(0.0),
        );
        if self.awaiting_proxy_layout
            && let Some(native_base_collapse_range) = self.native_base_collapse_range
        {
            return native_base_collapse_range;
        }
        self.native_base_collapse_range = Some(measured);
        measured
    }

    fn current_native_master_position(&mut self) -> ProfileVerticalMasterScrollPolicyPosition {
        let outer = self.outer_scroll_view.clone().filter(|_| self.uses_native_vertical_engine);
        let Some(outer) = outer else {
            return ProfileVerticalMasterScrollPolicyPosition {
                collapse_progress: self.outer_progress_by_tab.get(&self.active_tab).copied().unwrap_or(0.0),
                inner_offset: self.local_offset_by_tab.get(&self.active_tab).copied().unwrap_or(0.0),
            };
        };
        let collapse_range = self.native_collapse_range(&outer);
        let mapping = ProfileVerticalMasterScrollPolicy::mapping(
            outer.content_offset_y() - self.minimum_offset(&outer),
            collapse_range,
            self.active_inner_logical_range(),
        );
        ProfileVerticalMasterScrollPolicyPosition {
            collapse_progress: mapping.collapse_progress,
            inner_offset: mapping.inner_offset,
        }
    }

    fn current_semantic_vertical_position(&mut self) -> ProfileVerticalMasterScrollPolicyPosition {
        if self.uses_native_vertical_engine {
            return self.current_native_master_position();
        }
        ProfileVerticalMasterScrollPolicyPosition {
            collapse_progress: self.outer_collapse_progress(),
            inner_offset: self
                .active_inner_scroll_view()
                .map(|inner| self.logical_offset(&inner))
                .or_else(|| self.local_offset_by_tab.get(&self.active_tab).copied())
                .unwrap_or(0.0),
        }
    }

    fn request_active_proxy_height(&mut self, raw_height: f32, preserve_position: bool, cx: &mut Context<Self>) {
        if !self.uses_native_vertical_engine {
            return;
        }
        let height = if raw_height.is_finite() { raw_height.max(0.0) } else { 0.0 };
        if preserve_position && self.pending_master_position.is_none() {
            self.pending_master_position = Some(self.current_native_master_position());
        }
        self.pending_proxy_height = Some(height);
        self.schedule_pending_proxy_height_application(cx);
    }

    fn schedule_pending_proxy_height_application(&mut self, cx: &mut Context<Self>) {
        if !self.uses_native_vertical_engine || self.pending_proxy_height.is_none() {
            return;
        }
        self.proxy_height_update_generation = self.proxy_height_update_generation.wrapping_add(1);
        let generation = self.proxy_height_update_generation;
        self.main_async(cx, move |coordinator, cx| {
            if generation != coordinator.proxy_height_update_generation
                || coordinator.motion_blocks_geometry_refresh()
                || coordinator.awaiting_proxy_layout
            {
                return;
            }
            let Some(height) = coordinator.pending_proxy_height.take() else {
                return;
            };
            if (coordinator.applied_proxy_height - height).abs() <= 0.5 {
                if let Some(outer) = coordinator.outer_scroll_view.clone() {
                    coordinator.schedule_outer_refresh(&outer, cx);
                }
                return;
            }
            coordinator.apply_active_proxy_height(height, cx);
        });
    }

    fn apply_active_proxy_height(&mut self, height: f32, cx: &mut Context<Self>) {
        let normalized = if height.is_finite() { height.max(0.0) } else { 0.0 };
        if (self.applied_proxy_height - normalized).abs() <= 0.5 {
            self.applied_proxy_height = normalized;
            if self.active_proxy_height != normalized {
                self.active_proxy_height = normalized;
                cx.notify();
            }
            return;
        }
        let previous_proxy_height = self.applied_proxy_height;
        let outer = self.outer_scroll_view.clone();
        if let Some(outer) = outer.as_ref() {
            self.native_base_collapse_range = Some(self.native_collapse_range(outer));
        }
        self.applied_proxy_height = normalized;
        if let Some(outer) = outer.as_ref() {
            self.proxy_layout_generation = self.proxy_layout_generation.wrapping_add(1);
            self.awaiting_proxy_layout = true;
            self.awaiting_proxy_layout_generation = Some(self.proxy_layout_generation);
            self.expected_proxy_outer_range =
                Some(self.outer_range(outer, outer.content_size().height) + normalized - previous_proxy_height);
        } else {
            self.awaiting_proxy_layout = false;
            self.awaiting_proxy_layout_generation = None;
            self.expected_proxy_outer_range = None;
        }
        self.active_proxy_height = normalized;
        cx.notify();
        if let (Some(outer), Some(generation)) = (outer.as_ref(), self.awaiting_proxy_layout_generation) {
            self.schedule_proxy_layout_fallback(outer, generation, 2, cx);
        }
    }

    fn synchronize_native_vertical_position(&mut self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        if !self.uses_native_vertical_engine
            || self.outer_scroll_view.as_ref() != Some(scroll_view)
            || self.awaiting_proxy_layout
            || self.is_mirroring_master_offset
        {
            return;
        }

        let master_offset = scroll_view.content_offset_y() - self.minimum_offset(scroll_view);
        let collapse_range = self.native_collapse_range(scroll_view);
        let mapping = ProfileVerticalMasterScrollPolicy::mapping(
            master_offset,
            collapse_range,
            self.active_inner_logical_range(),
        );

        if let Some(inner) = self.active_inner_scroll_view() {
            let target_y = self.minimum_offset(&inner) + mapping.inner_offset;
            if (inner.content_offset_y() - target_y).abs() > ProfileVerticalMasterScrollPolicy::EPSILON {
                self.is_mirroring_master_offset = true;
                if !self.mirrors_in_prepaint {
                    self.set_offset_y(target_y, &inner, cx);
                } else if (inner.content_offset_y() - target_y).abs() > 0.1 {
                    inner.set_content_offset_in_prepaint(inner.content_offset_x(), target_y);
                    if !self.prepaint_mirrors.contains(&inner) {
                        self.prepaint_mirrors.push(inner.clone());
                    }
                }
                self.is_mirroring_master_offset = false;
            }
        }

        if !ProfilePerformanceTrace::shared().is_recording_any_scope() {
            return;
        }

        let now = ca_current_media_time();
        let elapsed = (now - self.last_native_scroll_timestamp).max(1.0 / 240.0);
        let velocity_y = if self.last_native_scroll_timestamp > 0.0 {
            -(master_offset - self.last_native_master_offset) / elapsed as f32
        } else {
            0.0
        };
        self.last_native_master_offset = master_offset;
        self.last_native_scroll_timestamp = now;

        let phase = if scroll_view.is_decelerating() {
            ProfilePerformanceGesturePhase::Decelerating
        } else if scroll_view.is_tracking() || self.vertical_motion_active {
            ProfilePerformanceGesturePhase::Changed
        } else {
            ProfilePerformanceGesturePhase::Idle
        };
        self.update_vertical_trace(phase, velocity_y);
    }

    fn capture_active_position(&mut self) {
        if self.uses_native_vertical_engine {
            let position = self.current_native_master_position();
            self.outer_progress_by_tab.insert(self.active_tab, position.collapse_progress);
            self.local_offset_by_tab.insert(self.active_tab, position.inner_offset);
            return;
        }
        let progress = self.outer_collapse_progress();
        self.outer_progress_by_tab.insert(self.active_tab, progress);
        if let Some(inner) = self.active_inner_scroll_view() {
            self.local_offset_by_tab.insert(self.active_tab, self.logical_offset(&inner));
        }
    }

    fn outer_collapse_progress(&mut self) -> f32 {
        let Some(outer) = self.outer_scroll_view.clone() else {
            return self.outer_progress_by_tab.get(&self.active_tab).copied().unwrap_or(0.0);
        };
        if self.uses_native_vertical_engine {
            return self.current_native_master_position().collapse_progress;
        }
        let minimum = self.minimum_offset(&outer);
        let range = self.maximum_offset(&outer) - minimum;
        if range <= 0.5 {
            return 0.0;
        }
        ((outer.content_offset_y() - minimum) / range).max(0.0).min(1.0)
    }

    fn active_inner_scroll_view(&self) -> Option<ProfileScrollView> {
        self.registered_inner_scroll_view(self.active_tab)
    }

    /// minimum offset is 0 here, a desktop scroll view has no safe area
    fn minimum_offset(&self, _scroll_view: &ProfileScrollView) -> f32 {
        0.0
    }

    fn maximum_offset(&self, scroll_view: &ProfileScrollView) -> f32 {
        self.minimum_offset(scroll_view).max(scroll_view.content_size().height - scroll_view.bounds_height())
    }

    fn outer_range(&self, scroll_view: &ProfileScrollView, content_height: f32) -> f32 {
        let minimum = self.minimum_offset(scroll_view);
        let maximum = minimum.max(content_height - scroll_view.bounds_height());
        (maximum - minimum).max(0.0)
    }

    fn logical_offset(&self, scroll_view: &ProfileScrollView) -> f32 {
        let minimum = self.minimum_offset(scroll_view);
        (scroll_view.content_offset_y() - minimum).min(self.maximum_offset(scroll_view) - minimum).max(0.0)
    }

    fn set_logical_offset(&self, logical_offset: f32, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        let minimum = self.minimum_offset(scroll_view);
        let range = self.maximum_offset(scroll_view) - minimum;
        self.set_offset_y(minimum + logical_offset.max(0.0).min(range), scroll_view, cx);
    }

    fn outer_scroll_view_did_scroll(&mut self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        if !self.uses_native_vertical_engine || self.outer_scroll_view.as_ref() != Some(scroll_view) {
            return;
        }
        self.synchronize_native_vertical_position(scroll_view, cx);
    }

    fn outer_scroll_view_will_begin_dragging(
        &mut self,
        scroll_view: &ProfileScrollView,
        velocity_y: f32,
        cx: &mut Context<Self>,
    ) {
        if !self.uses_native_vertical_engine || self.outer_scroll_view.as_ref() != Some(scroll_view) {
            return;
        }
        self.status_bar_scroll_to_top_active = false;
        self.expire_stale_return_if_needed(cx);
        self.begin_vertical_motion(cx);
        self.last_native_scroll_timestamp = 0.0;
        self.native_release_velocity_y = 0.0;
        self.synchronize_native_vertical_position(scroll_view, cx);
        self.update_vertical_trace(ProfilePerformanceGesturePhase::Began, velocity_y);
    }

    fn outer_scroll_view_will_end_dragging(&mut self, scroll_view: &ProfileScrollView, velocity_y: f32) {
        if !self.uses_native_vertical_engine || self.outer_scroll_view.as_ref() != Some(scroll_view) {
            return;
        }
        self.native_release_velocity_y = -velocity_y * 1_000.0;
    }

    fn outer_scroll_view_did_end_dragging(
        &mut self,
        scroll_view: &ProfileScrollView,
        will_decelerate: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.uses_native_vertical_engine || self.outer_scroll_view.as_ref() != Some(scroll_view) {
            return;
        }
        if will_decelerate {
            self.update_vertical_trace(ProfilePerformanceGesturePhase::Decelerating, self.native_release_velocity_y);
            return;
        }
        self.complete_native_vertical_motion(scroll_view, cx);
    }

    fn outer_scroll_view_will_begin_decelerating(&mut self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        if !self.uses_native_vertical_engine || self.outer_scroll_view.as_ref() != Some(scroll_view) {
            return;
        }
        self.begin_vertical_motion(cx);
        self.update_vertical_trace(ProfilePerformanceGesturePhase::Decelerating, self.native_release_velocity_y);
    }

    fn outer_scroll_view_did_end_scrolling_animation(&mut self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        if !self.uses_native_vertical_engine || self.outer_scroll_view.as_ref() != Some(scroll_view) {
            return;
        }
        self.complete_native_vertical_motion(scroll_view, cx);
    }

    fn complete_native_vertical_motion(&mut self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        self.synchronize_native_vertical_position(scroll_view, cx);
        self.capture_active_position();
        self.update_vertical_trace(ProfilePerformanceGesturePhase::Ended, 0.0);
        self.last_native_scroll_timestamp = 0.0;
        self.native_release_velocity_y = 0.0;
        self.finish_vertical_motion(cx);
    }

    pub fn scroll_view_should_scroll_to_top(&mut self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) -> bool {
        if !self.prepare_for_status_bar_scroll_to_top(scroll_view, true, cx) {
            return false;
        }
        scroll_view.animate_content_offset_y(self.minimum_offset(scroll_view), cx);
        true
    }

    /// nested page must start the same trip here, its not a scrolls_to_top owner itself
    pub fn prepare_for_status_bar_scroll_to_top(
        &mut self,
        from: &ProfileScrollView,
        animated: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.outer_scroll_view.as_ref() != Some(from) || self.interaction_locked {
            return false;
        }

        self.stop_vertical_motion_driver(cx);
        self.finish_vertical_motion(cx);
        self.outer_progress_by_tab.insert(self.active_tab, 0.0);
        self.pending_outer_progress = None;
        self.pending_master_position = None;
        self.local_offset_by_tab.insert(self.active_tab, 0.0);

        if self.uses_native_vertical_engine {
            self.begin_vertical_motion(cx);
            self.status_bar_scroll_to_top_active = true;
            return true;
        }

        let Some(inner) = self.active_inner_scroll_view() else {
            return true;
        };
        if let Some(registration) = self.inner_registrations.get_mut(&self.active_tab) {
            registration.desired_logical_offset = None;
        }
        self.set_to_minimum_offset(&inner, animated, cx);
        true
    }

    /// pins the nested page when the outer animation finishes,
    /// no subpixel residue and nothing observed gets published
    pub fn complete_status_bar_scroll_to_top(&mut self, from: &ProfileScrollView, cx: &mut Context<Self>) {
        if self.outer_scroll_view.as_ref() != Some(from) {
            return;
        }
        self.outer_progress_by_tab.insert(self.active_tab, 0.0);
        self.pending_outer_progress = None;
        self.pending_master_position = None;
        self.local_offset_by_tab.insert(self.active_tab, 0.0);

        if let Some(inner) = self.active_inner_scroll_view() {
            if let Some(registration) = self.inner_registrations.get_mut(&self.active_tab) {
                registration.desired_logical_offset = None;
            }
            self.set_to_minimum_offset(&inner, false, cx);
        }
        if self.uses_native_vertical_engine {
            self.finish_vertical_motion(cx);
        }
    }

    fn set_to_minimum_offset(&self, scroll_view: &ProfileScrollView, animated: bool, cx: &mut Context<Self>) {
        let target = self.minimum_offset(scroll_view);
        if (scroll_view.content_offset_y() - target).abs() <= 0.1 {
            return;
        }
        if animated {
            scroll_view.animate_content_offset_y(target, cx);
        } else {
            scroll_view.set_content_offset(scroll_view.content_offset_x(), target, cx);
        }
    }

    fn clamp_offset(&self, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        let clamped =
            scroll_view.content_offset_y().max(self.minimum_offset(scroll_view)).min(self.maximum_offset(scroll_view));
        self.set_offset_y(clamped, scroll_view, cx);
    }

    fn set_offset_y(&self, y: f32, scroll_view: &ProfileScrollView, cx: &mut Context<Self>) {
        if (scroll_view.content_offset_y() - y).abs() <= 0.1 {
            return;
        }
        scroll_view.set_content_offset(scroll_view.content_offset_x(), y, cx);
    }

    fn enforce_outer_configuration(&self, scroll_view: &ProfileScrollView) {
        if self.uses_native_vertical_engine {
            scroll_view.set_pan_enabled(self.outer_original_pan_enabled && !self.interaction_locked);
            scroll_view.set_bounces(self.outer_original_bounces);
            scroll_view.set_deceleration_rate(PROFILE_DECELERATION_RATE_NORMAL);
            scroll_view.set_is_directional_lock_enabled(true);
        } else {
            scroll_view.set_pan_enabled(false);
            scroll_view.set_bounces(false);
        }
        scroll_view.set_scrolls_to_top(ProfileScrollToTopOwnershipPolicy::is_owner(
            ProfileScrollToTopOwnershipPolicyRole::Outer,
        ));
    }

    fn enforce_all_inner_configurations(&self) {
        for tab in self.inner_registrations.keys() {
            self.enforce_inner_configuration(*tab);
        }
    }

    fn enforce_inner_configuration(&self, tab: i64) {
        let Some(registration) = self.inner_registrations.get(&tab) else {
            return;
        };
        let Some(scroll_view) = registration.scroll_view.upgrade() else {
            return;
        };
        let has_outer_owner = self.outer_scroll_view.is_some();
        scroll_view.set_pan_enabled(!has_outer_owner && registration.original_pan_enabled && !self.interaction_locked);
        scroll_view.set_bounces(if has_outer_owner { false } else { registration.original_bounces });
        scroll_view.set_scrolls_to_top(if has_outer_owner {
            ProfileScrollToTopOwnershipPolicy::is_owner(ProfileScrollToTopOwnershipPolicyRole::Inner)
        } else {
            registration.original_scrolls_to_top
        });
    }

    fn remove_inner_registration(&mut self, tab: i64, _cx: &mut Context<Self>) {
        if !self.inner_registrations.contains_key(&tab) {
            return;
        }
        if self.uses_native_vertical_engine && tab == self.active_tab {
            self.pending_master_position = Some(self.current_native_master_position());
        }
        let Some(registration) = self.inner_registrations.remove(&tab) else {
            return;
        };
        if let Some(scroll_view) = registration.scroll_view.upgrade() {
            self.local_offset_by_tab.insert(tab, self.logical_offset(&scroll_view));
            scroll_view.set_pan_enabled(registration.original_pan_enabled);
            scroll_view.set_bounces(registration.original_bounces);
            scroll_view.set_scrolls_to_top(registration.original_scrolls_to_top);
        }
        self.parked_inner_scroll_views.insert(tab, registration.scroll_view);
        self.inner_refresh_scheduled.remove(&tab);
        self.deferred_inner_refresh_tabs.remove(&tab);
    }

    fn unbind_outer_current(&mut self, cx: &mut Context<Self>) {
        self.stop_vertical_motion_driver(cx);
        self.cancel_horizontal_settlement(cx);
        self.finish_pending_horizontal_selection_if_needed(cx);
        self.set_horizontal_drag_progress(0.0, false, cx);
        self.reset_deferred_geometry_refreshes();
        self.capture_active_position();
        self.outer_refresh_scheduled = false;

        if let Some(scroll_view) = self.outer_scroll_view.as_ref() {
            scroll_view.set_pan_enabled(self.outer_original_pan_enabled);
            scroll_view.set_bounces(self.outer_original_bounces);
            scroll_view.set_scrolls_to_top(self.outer_original_scrolls_to_top);
            scroll_view.set_deceleration_rate(self.outer_original_deceleration_rate);
            scroll_view.set_is_directional_lock_enabled(self.outer_original_directional_lock_enabled);
            scroll_view.set_tracking(false);
        }

        self.vertical_pan = None;
        self.horizontal_pan = None;
        if let Some(scroll_view) = self.outer_scroll_view.clone() {
            self.parked_outer_scroll_view = Some(scroll_view.downgrade());
            self.parked_collapse_range =
                self.uses_native_vertical_engine.then(|| self.measured_return_collapse_range(&scroll_view));
        }
        self.on_horizontal_swipe = None;
        self.can_horizontal_swipe = None;
        self.outer_scroll_view = None;
        self.last_outer_minimum = None;
        self.last_outer_maximum = None;
        self.native_base_collapse_range = None;
        self.awaiting_proxy_layout = false;
        self.awaiting_proxy_layout_generation = None;
        self.expected_proxy_outer_range = None;
        self.proxy_layout_generation = self.proxy_layout_generation.wrapping_add(1);
        self.pending_proxy_height = None;
        self.proxy_height_update_generation = self.proxy_height_update_generation.wrapping_add(1);
        self.last_native_scroll_timestamp = 0.0;
        self.native_release_velocity_y = 0.0;
        self.enforce_all_inner_configurations();
    }

    const MOMENTUM_MINIMUM_VELOCITY: f32 = 40.0;
    const DISPLAY_LINK_CADENCE_TOLERANCE: f64 = 0.9;
    const TRACE_THRESHOLD_MS: f64 = 0.75;
    const LEGACY_WHEEL_LINE_HEIGHT: f32 = 20.0;
}

/// handle that implements the shared element return scroller for the coordinator
/// protocol needs a cx the coordinators update cant share
pub struct ProfileScrollCoordinatorReturnScroller {
    coordinator: WeakEntity<ProfileScrollCoordinator>,
    on_return_resumable: Rc<RefCell<Option<SharedElementReturnResumableHandler>>>,
}

impl SharedElementReturnScroller for ProfileScrollCoordinatorReturnScroller {
    fn can_prepare_return(&self, tab_key: i64, cx: &App) -> bool {
        self.coordinator.upgrade().is_some_and(|coordinator| coordinator.read(cx).can_prepare_return(tab_key))
    }

    fn return_viewport_snapshot(&self, tab_key: i64, cx: &App) -> Option<SharedElementReturnViewport> {
        self.coordinator.upgrade()?.read(cx).return_viewport_snapshot(tab_key)
    }

    fn prepare_return(&self, request: SharedElementReturnRequest, cx: &mut App) -> Option<SharedElementReturnPlan> {
        self.coordinator.upgrade()?.update(cx, |coordinator, _| coordinator.prepare_return(request))
    }

    fn resolve_return(&self, epoch: u64, item_content_rect: Bounds<Pixels>, cx: &mut App) -> SharedElementReturnResolution {
        let Some(coordinator) = self.coordinator.upgrade() else {
            return SharedElementReturnResolution::Unavailable;
        };
        coordinator.update(cx, |coordinator, cx| coordinator.resolve_return(epoch, item_content_rect, cx))
    }

    fn cancel_return(&self, epoch: u64, cx: &mut App) {
        if let Some(coordinator) = self.coordinator.upgrade() {
            coordinator.update(cx, |coordinator, cx| coordinator.cancel_return(epoch, cx));
        }
    }

    fn finish_return(&self, epoch: u64, cx: &mut App) {
        if let Some(coordinator) = self.coordinator.upgrade() {
            coordinator.update(cx, |coordinator, cx| coordinator.finish_return(epoch, cx));
        }
    }

    fn on_return_resumable(&self) -> Option<SharedElementReturnResumableHandler> {
        self.on_return_resumable.borrow().clone()
    }

    fn set_on_return_resumable(&self, handler: Option<SharedElementReturnResumableHandler>) {
        *self.on_return_resumable.borrow_mut() = handler;
    }

    fn cancel_live_return(&self, epoch: u64, cx: &mut App) {
        if let Some(coordinator) = self.coordinator.upgrade() {
            coordinator.update(cx, |coordinator, cx| coordinator.cancel_live_return(epoch, cx));
        }
    }

    fn note_open_position(&self, tab_key: i64, cx: &mut App) {
        if let Some(coordinator) = self.coordinator.upgrade() {
            coordinator.update(cx, |coordinator, _| coordinator.note_open_position(tab_key));
        }
    }

    fn reset_to_content_top(&self, tab_key: i64, cx: &mut App) {
        if let Some(coordinator) = self.coordinator.upgrade() {
            coordinator.update(cx, |coordinator, cx| coordinator.reset_tab_to_content_top(tab_key, cx));
        }
    }
}

/// goes first in the outer scroll content so the inner page is mirrored before it prepaints
#[derive(IntoElement)]
pub struct ProfileOuterScrollIntrospector {
    pub coordinator: Entity<ProfileScrollCoordinator>,
    pub on_horizontal_swipe: ProfileOnHorizontalSwipe,
    pub can_horizontal_swipe: ProfileCanHorizontalSwipe,
}

impl ProfileOuterScrollIntrospector {
    pub fn new(
        coordinator: Entity<ProfileScrollCoordinator>,
        on_horizontal_swipe: ProfileOnHorizontalSwipe,
        can_horizontal_swipe: ProfileCanHorizontalSwipe,
    ) -> Self {
        Self { coordinator, on_horizontal_swipe, can_horizontal_swipe }
    }
}

impl RenderOnce for ProfileOuterScrollIntrospector {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = ("profile_outer_scroll_probe", self.coordinator.entity_id().as_u64());
        let probe = window.use_keyed_state(key, cx, |_, cx: &mut Context<ProfileOuterScrollProbeView>| {
            ProfileOuterScrollProbeView::new(cx)
        });
        probe.update(cx, |probe, _| {
            probe.configure(self.coordinator.clone(), self.on_horizontal_swipe, self.can_horizontal_swipe);
        });
        let prepaint_view = self.coordinator.read(cx).outer_host_scroll_view();
        let coordinator = self.coordinator;
        let wheel_coordinator = coordinator.clone();
        let mirror_coordinator = coordinator.clone();
        canvas(
            move |_, window: &mut Window, cx: &mut App| {
                probe.update(cx, |probe, cx| probe.layout_subviews(&prepaint_view, window, cx));
                drive_outer_frame(&coordinator, &prepaint_view, window, cx);
                window.insert_hitbox(prepaint_view.frame(), HitboxBehavior::Normal)
            },
            move |_, hitbox: Hitbox, window: &mut Window, cx: &mut App| {
                let mirrored =
                    mirror_coordinator.update(cx, |coordinator, _| std::mem::take(&mut coordinator.prepaint_mirrors));
                for scroll_view in mirrored {
                    scroll_view.redraw_if_prepaint_offset_unlaid(cx);
                }
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let hovered = hitbox.should_handle_scroll(window);
                    let consumed = wheel_coordinator
                        .update(cx, |coordinator, cx| coordinator.outer_scroll_view_wheel(event, hovered, cx));
                    if consumed {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }
}

fn drive_outer_frame(
    coordinator: &Entity<ProfileScrollCoordinator>,
    scroll_view: &ProfileScrollView,
    window: &mut Window,
    cx: &mut App,
) {
    let landed = scroll_view.advance_offset_animation(window);
    let did_scroll = scroll_view.take_did_scroll();
    let frame_rate_range = coordinator.read(cx).display_link_running.then(|| {
        ProfileMotionFrameRatePolicy::frame_rate_range(i64::from(maximum_frames_per_second(window)))
    });
    if !landed && !did_scroll && frame_rate_range.is_none() {
        return;
    }
    let keeps_ticking = coordinator.update(cx, |coordinator, cx| {
        if did_scroll {
            coordinator.mirrors_in_prepaint = true;
            coordinator.outer_scroll_view_did_scroll(scroll_view, cx);
            coordinator.mirrors_in_prepaint = false;
        }
        if landed {
            coordinator.complete_status_bar_scroll_to_top(scroll_view, cx);
            coordinator.outer_scroll_view_did_end_scrolling_animation(scroll_view, cx);
        }
        if let Some(range) = frame_rate_range {
            coordinator.display_link_did_fire(ca_current_media_time(), range, cx);
        }
        coordinator.display_link_running
    });
    if keeps_ticking {
        window.request_animation_frame();
    }
}

pub struct ProfileOuterScrollProbeView {
    owner: Option<WeakEntity<ProfileScrollCoordinator>>,
    bound_scroll_view: Option<ProfileScrollView>,
    token: Option<Uuid>,
    bound_bounds_size: Option<Size<f32>>,
    bound_content_size: Option<Size<f32>>,
    on_horizontal_swipe: ProfileOnHorizontalSwipe,
    can_horizontal_swipe: ProfileCanHorizontalSwipe,
}

impl ProfileOuterScrollProbeView {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|probe, cx| {
            if let Some(owner) = probe.owner.as_ref().and_then(WeakEntity::upgrade) {
                owner.update(cx, |owner, cx| owner.pause_interaction(cx));
            }
            probe.invalidate(cx);
        })
        .detach();
        Self {
            owner: None,
            bound_scroll_view: None,
            token: None,
            bound_bounds_size: None,
            bound_content_size: None,
            on_horizontal_swipe: Rc::new(|_: i64, _: &mut App| {}),
            can_horizontal_swipe: Rc::new(|_: i64, _: &App| false),
        }
    }

    fn configure(
        &mut self,
        coordinator: Entity<ProfileScrollCoordinator>,
        on_horizontal_swipe: ProfileOnHorizontalSwipe,
        can_horizontal_swipe: ProfileCanHorizontalSwipe,
    ) {
        self.owner = Some(coordinator.downgrade());
        self.on_horizontal_swipe = on_horizontal_swipe;
        self.can_horizontal_swipe = can_horizontal_swipe;
    }

    fn layout_subviews(&mut self, scroll_view: &ProfileScrollView, window: &Window, cx: &mut App) {
        let Some(owner) = self.owner.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        scroll_view.did_lay_out(window);
        if self.bound_scroll_view.as_ref() != Some(scroll_view) {
            self.detach_binding(cx);
            let on_horizontal_swipe = self.on_horizontal_swipe.clone();
            let can_horizontal_swipe = self.can_horizontal_swipe.clone();
            let token = owner.update(cx, |owner, cx| {
                owner.bind_outer(scroll_view, on_horizontal_swipe, can_horizontal_swipe, cx)
            });
            self.token = Some(token);
            self.bound_scroll_view = Some(scroll_view.clone());
            self.update_geometry_snapshot(scroll_view);
            owner.update(cx, |owner, _| owner.axis_owners.clear());
            return;
        }
        owner.update(cx, |owner, cx| {
            owner.axis_owners.clear();
            let on_horizontal_swipe = self.on_horizontal_swipe.clone();
            let can_horizontal_swipe = self.can_horizontal_swipe.clone();
            self.token = Some(owner.bind_outer(scroll_view, on_horizontal_swipe, can_horizontal_swipe, cx));
        });
        let old_content_size = self.bound_content_size;
        let new_content_size = scroll_view.content_size();
        let bounds_changed = self.update_geometry_snapshot(scroll_view);
        let content_changed = old_content_size.is_some_and(|old| {
            ProfileGeometryRefreshPolicy::content_size_changed(
                old,
                new_content_size,
                ProfileGeometryRefreshPolicy::CONTENT_SIZE_EPSILON,
            )
        });
        owner.update(cx, |owner, cx| {
            if content_changed {
                owner.outer_content_size_did_change(scroll_view, old_content_size, Some(new_content_size), cx);
            }
            if bounds_changed {
                owner.schedule_outer_refresh(scroll_view, cx);
            }
        });
    }

    fn invalidate(&mut self, cx: &mut App) {
        self.detach_binding(cx);
        self.owner = None;
        self.on_horizontal_swipe = Rc::new(|_: i64, _: &mut App| {});
        self.can_horizontal_swipe = Rc::new(|_: i64, _: &App| false);
    }

    fn detach_binding(&mut self, cx: &mut App) {
        if let (Some(owner), Some(scroll_view), Some(token)) =
            (self.owner.as_ref().and_then(WeakEntity::upgrade), self.bound_scroll_view.as_ref(), self.token)
        {
            owner.update(cx, |owner, cx| owner.unbind_outer(scroll_view, token, cx));
        }
        if let Some(scroll_view) = self.bound_scroll_view.take() {
            scroll_view.did_leave_window();
        }
        self.token = None;
        self.bound_bounds_size = None;
        self.bound_content_size = None;
    }

    fn update_geometry_snapshot(&mut self, scroll_view: &ProfileScrollView) -> bool {
        let bounds_size = size(scroll_view.bounds_width(), scroll_view.bounds_height());
        let changed = self.bound_bounds_size != Some(bounds_size);
        self.bound_bounds_size = Some(bounds_size);
        self.bound_content_size = Some(scroll_view.content_size());
        changed
    }
}

/// goes inside the page scroll content
#[derive(IntoElement)]
pub struct ProfileInnerScrollIntrospector {
    pub tab_key: i64,
    pub coordinator: Entity<ProfileScrollCoordinator>,
}

impl ProfileInnerScrollIntrospector {
    pub fn new(tab: ProfileContentTab, coordinator: Entity<ProfileScrollCoordinator>) -> Self {
        Self { tab_key: tab.raw_value(), coordinator }
    }

    pub fn new_key(tab_key: i64, coordinator: Entity<ProfileScrollCoordinator>) -> Self {
        Self { tab_key, coordinator }
    }
}

impl RenderOnce for ProfileInnerScrollIntrospector {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = ("profile_inner_scroll_probe", self.tab_key as u64);
        let probe = window.use_keyed_state(key, cx, |_, cx: &mut Context<ProfileInnerScrollProbeView>| {
            ProfileInnerScrollProbeView::new(cx)
        });
        probe.update(cx, |probe, cx| probe.configure(self.tab_key, &self.coordinator, cx));
        let scroll_view = self.coordinator.read(cx).page_scroll_view(self.tab_key);
        canvas(
            move |_, window: &mut Window, cx: &mut App| {
                probe.update(cx, |probe, cx| probe.layout_subviews(&scroll_view, window, cx));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }
}

pub struct ProfileInnerScrollProbeView {
    owner: Option<WeakEntity<ProfileScrollCoordinator>>,
    bound_scroll_view: Option<ProfileScrollView>,
    tab_key: i64,
    token: Option<Uuid>,
    bound_bounds_size: Option<Size<f32>>,
    bound_content_size: Option<Size<f32>>,
}

impl ProfileInnerScrollProbeView {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|probe, cx| probe.invalidate(cx)).detach();
        Self {
            owner: None,
            bound_scroll_view: None,
            tab_key: 0,
            token: None,
            bound_bounds_size: None,
            bound_content_size: None,
        }
    }

    fn configure(&mut self, tab_key: i64, coordinator: &Entity<ProfileScrollCoordinator>, cx: &mut App) {
        let same_owner = self.owner.as_ref().is_some_and(|owner| owner.entity_id() == coordinator.entity_id());
        if !same_owner || self.tab_key != tab_key {
            self.invalidate(cx);
            self.owner = Some(coordinator.downgrade());
            self.tab_key = tab_key;
        }
    }

    fn layout_subviews(&mut self, scroll_view: &ProfileScrollView, window: &Window, cx: &mut App) {
        let Some(owner) = self.owner.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        scroll_view.did_lay_out(window);
        let tab_key = self.tab_key;
        if self.bound_scroll_view.as_ref() != Some(scroll_view) {
            self.detach_binding(cx);
            self.bound_scroll_view = Some(scroll_view.clone());
            self.token = Some(owner.update(cx, |owner, cx| owner.register_inner(scroll_view, tab_key, cx)));
            self.update_geometry_snapshot(scroll_view);
            return;
        }
        self.token = Some(owner.update(cx, |owner, cx| owner.register_inner(scroll_view, tab_key, cx)));
        let old_content_size = self.bound_content_size;
        let new_content_size = scroll_view.content_size();
        let bounds_changed = self.update_geometry_snapshot(scroll_view);
        let content_changed = old_content_size.is_some_and(|old| {
            ProfileGeometryRefreshPolicy::content_size_changed(
                old,
                new_content_size,
                ProfileGeometryRefreshPolicy::CONTENT_SIZE_EPSILON,
            )
        });
        if content_changed || bounds_changed {
            owner.update(cx, |owner, cx| owner.schedule_inner_refresh(tab_key, scroll_view, cx));
        }
    }

    fn invalidate(&mut self, cx: &mut App) {
        self.detach_binding(cx);
        self.owner = None;
    }

    fn detach_binding(&mut self, cx: &mut App) {
        if let (Some(owner), Some(scroll_view), Some(token)) =
            (self.owner.as_ref().and_then(WeakEntity::upgrade), self.bound_scroll_view.as_ref(), self.token)
        {
            let tab_key = self.tab_key;
            owner.update(cx, |owner, cx| owner.unregister_inner(scroll_view, tab_key, token, cx));
        }
        if let Some(scroll_view) = self.bound_scroll_view.take() {
            scroll_view.did_leave_window();
        }
        self.token = None;
        self.bound_bounds_size = None;
        self.bound_content_size = None;
    }

    fn update_geometry_snapshot(&mut self, scroll_view: &ProfileScrollView) -> bool {
        let bounds_size = size(scroll_view.bounds_width(), scroll_view.bounds_height());
        let changed = self.bound_bounds_size != Some(bounds_size);
        self.bound_bounds_size = Some(bounds_size);
        self.bound_content_size = Some(scroll_view.content_size());
        changed
    }
}

/// goes inside the strip next to the pages
#[derive(IntoElement)]
pub struct ProfileHorizontalPagerIntrospector {
    pub coordinator: Entity<ProfileScrollCoordinator>,
    pub selected_index: i64,
    pub page_count: i64,
    pub page_width: f32,
}

impl ProfileHorizontalPagerIntrospector {
    pub fn new(
        coordinator: Entity<ProfileScrollCoordinator>,
        selected_index: i64,
        page_count: i64,
        page_width: f32,
    ) -> Self {
        Self { coordinator, selected_index, page_count, page_width }
    }
}

impl RenderOnce for ProfileHorizontalPagerIntrospector {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = ("profile_horizontal_pager_probe", self.coordinator.entity_id().as_u64());
        let probe = window.use_keyed_state(key, cx, |_, cx: &mut Context<ProfileHorizontalPagerProbeView>| {
            ProfileHorizontalPagerProbeView::new(cx)
        });
        probe.update(cx, |probe, cx| {
            probe.configure(&self.coordinator, self.selected_index, self.page_count, self.page_width, cx);
        });
        let (scroll_view, motion_view) = {
            let coordinator = self.coordinator.read(cx);
            (coordinator.horizontal_pager_host_scroll_view(), coordinator.horizontal_pager_host_motion_view())
        };
        canvas(
            move |_, window: &mut Window, cx: &mut App| {
                probe.update(cx, |probe, cx| probe.layout_subviews(&scroll_view, &motion_view, window, cx));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }
}

#[derive(Clone, Copy, PartialEq)]
struct ProfileHorizontalPagerProbeViewBindingSnapshot {
    selected_index: i64,
    page_count: i64,
    page_width: f32,
    bounds_size: Size<f32>,
}

pub struct ProfileHorizontalPagerProbeView {
    owner: Option<WeakEntity<ProfileScrollCoordinator>>,
    bound_scroll_view: Option<ProfileScrollView>,
    bound_motion_view: Option<ProfileHorizontalMotionView>,
    token: Option<Uuid>,
    selected_index: i64,
    page_count: i64,
    page_width: f32,
    last_binding_snapshot: Option<ProfileHorizontalPagerProbeViewBindingSnapshot>,
}

impl ProfileHorizontalPagerProbeView {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|probe, cx| probe.invalidate(cx)).detach();
        Self {
            owner: None,
            bound_scroll_view: None,
            bound_motion_view: None,
            token: None,
            selected_index: 0,
            page_count: 0,
            page_width: 1.0,
            last_binding_snapshot: None,
        }
    }

    fn configure(
        &mut self,
        coordinator: &Entity<ProfileScrollCoordinator>,
        selected_index: i64,
        page_count: i64,
        page_width: f32,
        cx: &mut App,
    ) {
        let same_owner = self.owner.as_ref().is_some_and(|owner| owner.entity_id() == coordinator.entity_id());
        if !same_owner {
            self.invalidate(cx);
            self.owner = Some(coordinator.downgrade());
        }
        self.selected_index = selected_index;
        self.page_count = page_count;
        self.page_width = page_width.max(1.0);
    }

    fn layout_subviews(
        &mut self,
        scroll_view: &ProfileScrollView,
        motion_view: &ProfileHorizontalMotionView,
        window: &Window,
        cx: &mut App,
    ) {
        let Some(owner) = self.owner.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        scroll_view.did_lay_out(window);
        let is_bound = self.bound_scroll_view.as_ref() == Some(scroll_view)
            && self.bound_motion_view.as_ref() == Some(motion_view)
            && owner.read(cx).is_valid_horizontal_pager(scroll_view)
            && owner.read(cx).holds_horizontal_pager(scroll_view, self.token);
        if !is_bound {
            self.detach_binding(cx);
            if !owner.read(cx).is_valid_horizontal_pager(scroll_view) {
                return;
            }
            self.bound_scroll_view = Some(scroll_view.clone());
            self.bound_motion_view = Some(motion_view.clone());
            let (selected_index, page_count, page_width) = (self.selected_index, self.page_count, self.page_width);
            self.token = Some(owner.update(cx, |owner, cx| {
                owner.bind_horizontal_pager(scroll_view, motion_view, selected_index, page_count, page_width, cx)
            }));
            self.last_binding_snapshot = Some(self.binding_snapshot(scroll_view));
            return;
        }
        let snapshot = self.binding_snapshot(scroll_view);
        let (selected_index, page_count, page_width) = (self.selected_index, self.page_count, self.page_width);
        if Some(snapshot) == self.last_binding_snapshot {
            owner.update(cx, |owner, cx| owner.reconcile_horizontal_pager_rest(scroll_view, selected_index, cx));
            return;
        }
        owner.update(cx, |owner, cx| {
            owner.update_horizontal_pager(scroll_view, motion_view, selected_index, page_count, page_width, cx);
        });
        self.last_binding_snapshot = Some(snapshot);
    }

    fn invalidate(&mut self, cx: &mut App) {
        self.detach_binding(cx);
        self.owner = None;
    }

    fn detach_binding(&mut self, cx: &mut App) {
        if let (Some(owner), Some(scroll_view), Some(token)) =
            (self.owner.as_ref().and_then(WeakEntity::upgrade), self.bound_scroll_view.as_ref(), self.token)
        {
            owner.update(cx, |owner, cx| owner.unbind_horizontal_pager(scroll_view, token, cx));
        }
        if let Some(scroll_view) = self.bound_scroll_view.take() {
            scroll_view.did_leave_window();
        }
        self.bound_motion_view = None;
        self.token = None;
        self.last_binding_snapshot = None;
    }

    fn binding_snapshot(&self, scroll_view: &ProfileScrollView) -> ProfileHorizontalPagerProbeViewBindingSnapshot {
        ProfileHorizontalPagerProbeViewBindingSnapshot {
            selected_index: self.selected_index,
            page_count: self.page_count,
            page_width: self.page_width,
            bounds_size: size(scroll_view.bounds_width(), scroll_view.bounds_height()),
        }
    }
}
