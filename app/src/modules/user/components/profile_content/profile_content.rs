use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, AppContext, AsyncApp, Bounds, Context, ElementId, Entity, InteractiveElement, IntoElement,
    ParentElement, Render, RenderOnce, SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Subscription,
    Task, Pixels, WeakEntity, Window, canvas, div, prelude::FluentBuilder, px,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_theme::{RiseThemeAppearance, ThemeStore};
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig};
use rise_ui::guide_anchor_ui::guide_scroll_host::GuideScrollHost;
use rise_ui::scroll_edge_blur_ui::ScrollEdgeBlurFalloff;
use rise_ui::swiftui::environment_values::accessibility_reduce_transparency;
use rise_ui::scroll_chrome_shadow_ui::{ScrollChromeShadowStyle, ScrollChromeShadowUi};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::{LayoutDirection, LayoutDirectionKey, env_scope, layout_direction};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::linear_gradient::{GradientStop, LinearGradient};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::scroll_view::{ScrollView, scroll_view};
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::ui::animated_tabs_ui::animated_tabs_header::AnimatedTabsHeader;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager_gesture_gate::AnimatedTabsPagerAxisOwnersKey;
use crate::core::ui::profile_grid_date_scrubber::ProfileGridDateScrubberLayer;
use crate::core::widgets::navigations::bottom_chrome_insets::{
    BottomChromeInsets, BottomChromeOwner, BottomChromePolicy,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::gifts::components::user_gifts_tab::UserGiftsTab;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::post::components::post_collection_ui::PostLayoutToggleUi;
use crate::modules::post::engine::rise_post_engine_models::RisePostProfileKind;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::story::components::stories_grid::stories_grid_cell::StoriesGridMetrics;
use crate::modules::story::components::stories_grid::stories_grid_page::StoriesGridPage;
use crate::modules::story::components::stories_grid::stories_grid_view::ContainerWidthHintKey;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_content::pages::comments::profile_comments::ProfileComments;
use crate::modules::user::components::profile_content::pages::grid_favorites::grid_favorites::GridFavorites;
use crate::modules::user::components::profile_content::pages::grid_mentions::grid_mentions::GridMentions;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::GridPosts;
use crate::modules::user::components::profile_content::pages::grid_reposts::grid_reposts::GridReposts;
use crate::modules::user::components::profile_content::pages::grid_riffs::grid_riffs::GridRiffs;
use crate::modules::user::components::profile_content::pages::list_posts::list_posts::ListPosts;
use crate::modules::user::components::profile_content::pages::plans::plans::Plans;
use crate::modules::user::components::profile_content::profile_content_goals_hooks::profile_content_goals_hooks;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::{
    ProfileContentLayoutPolicy, ProfileHorizontalMotionElement, ProfileHorizontalPagerIntrospector,
    ProfileHorizontalPagerPolicy,
    ProfileInnerScrollIntrospector, ProfileScrollCoordinator, ProfileScrollMetrics,
};
use crate::modules::user::components::profile_content::profile_underlap_touch_router::ProfileUnderlapTouchRouter;
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::{
    SharedElementReturnScrollerKey, SharedElementReturnScrollerRef,
};
use crate::app::router::main_tab_chrome::MainTabChrome;
use crate::modules::user::stores::user::user_actions::user_types::{GoalItem, PlanItem};
use rise_ui::liquid_chrome::LiquidChrome;

/// raw value is only the scroll coordinators key for this tab, never saved or sent
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileContentTab {
    Publications,
    Riffs,
    Threads,
    Stories,
    Comments,
    Reposts,
    Mentions,
    Favorites,
    Gifts,
    Goals,
    Plans,
}

impl ProfileContentTab {
    pub const ALL_CASES: [ProfileContentTab; 11] = [
        Self::Publications,
        Self::Riffs,
        Self::Threads,
        Self::Stories,
        Self::Comments,
        Self::Reposts,
        Self::Mentions,
        Self::Favorites,
        Self::Gifts,
        Self::Goals,
        Self::Plans,
    ];

    pub const POSTS: Self = Self::Publications;

    pub fn raw_value(self) -> i64 {
        self as i64
    }

    pub fn id(self) -> Self {
        self
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Self::Publications => "profile_tab_publications",
            Self::Riffs => "riffs",
            Self::Threads => "profile_tab_threads",
            Self::Stories => "profile_tab_stories",
            Self::Comments => "navbtn_comments",
            Self::Reposts => "profile_tab_reposts",
            Self::Mentions => "profile_tab_mentions",
            Self::Favorites => "chat_profile_tab_fav",
            Self::Gifts => "profile_tab_gifts",
            Self::Goals => "goals_title",
            Self::Plans => "plans_title",
        }
    }

    /// every tab always, blocks never remove one. favorites show only for the owner
    pub fn visible(
        is_own_profile: bool,
        _has_foreign_goals: bool,
        _has_foreign_plans: bool,
    ) -> Vec<ProfileContentTab> {
        Self::ALL_CASES
            .into_iter()
            .filter(|tab| is_own_profile || *tab != Self::Favorites)
            .collect()
    }
}

pub enum ProfileCreateActionMetrics {}

impl ProfileCreateActionMetrics {
    pub const SIDE: f32 = 54.0;
}

/// laid out frame of the profile tabs bar and whether it leaves room for the create action
struct ProfileTabBarReserve {
    bounds: Option<Bounds<Pixels>>,
    reserves: bool,
    _insets_observer: Subscription,
}

pub struct ProfileViewportLayoutPolicy;

impl ProfileViewportLayoutPolicy {
    pub const OUTER_SPACING: f32 = 14.0;

    pub const HEADER_UNDERLAP: f32 = ProfileScrollMetrics::PINNED_TABS_TOP;

    pub fn container_height(viewport_height: f32) -> f32 {
        (viewport_height - ProfileScrollMetrics::PINNED_TABS_TOP).max(220.0)
    }

    pub fn pager_height(container_height: f32) -> f32 {
        container_height + Self::HEADER_UNDERLAP
    }

    pub fn page_content_top_inset() -> f32 {
        Self::HEADER_UNDERLAP + ProfileScrollMetrics::TABS_BAR_HEIGHT
    }

    pub fn initial_tabs_top(profile_top_height: f32) -> f32 {
        profile_top_height + Self::OUTER_SPACING
    }

    pub fn outer_scroll_range(profile_top_height: f32) -> f32 {
        (profile_top_height + Self::OUTER_SPACING - ProfileScrollMetrics::PINNED_TABS_TOP).max(0.0)
    }

    pub fn pinned_tabs_top(profile_top_height: f32) -> f32 {
        Self::initial_tabs_top(profile_top_height) - Self::outer_scroll_range(profile_top_height)
    }
}

pub type ProfileSelectTab = Rc<dyn Fn(ProfileContentTab, &mut Window, &mut App)>;

type ProfilePage = Rc<dyn Fn(ProfileContentTab, &mut Window, &mut App) -> AnyElement>;
/// everything a tab page reads from its host that no store reports, a change drops the cached frame
type ProfilePageKey = Rc<dyn Fn(ProfileContentTab, &mut App) -> u64>;

/// one mounted tab page as its own cached view so frames above it dont relayout the grid
struct ProfileTabPageHost {
    tab: ProfileContentTab,
    page: Option<ProfilePage>,
    key: Option<u64>,
}

impl ProfileTabPageHost {
    fn new(tab: ProfileContentTab) -> Self {
        Self { tab, page: None, key: None }
    }

    fn set_page(&mut self, page: ProfilePage) {
        self.page = Some(page);
    }

    fn set_key(&mut self, key: u64) -> bool {
        if self.key == Some(key) {
            return false;
        }
        let drawn_before = self.key.is_some();
        self.key = Some(key);
        drawn_before
    }
}

/// keeps a mounted tab page host alive across pager frames
struct ProfileTabPageHostHolder {
    host: Entity<ProfileTabPageHost>,
}

impl Render for ProfileTabPageHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tab;
        let element = self.page.clone().map(|page| page(tab, window, cx));
        div().relative().size_full().children(element)
    }
}

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

fn tab_id(name: &'static str, tab: ProfileContentTab) -> ElementId {
    ElementId::NamedInteger(SharedString::new_static(name), tab.raw_value() as u64)
}

/// what the deferred mount steps see of the view
#[derive(Clone)]
struct ProfileContentMount {
    visible_tabs: Vec<ProfileContentTab>,
    selected_tab: Binding<ProfileContentTab>,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
}

/// view state plus the strip geometry
struct ProfileContentState {
    mounted_tabs: HashSet<ProfileContentTab>,
    mount_window_generation: u64,
    initial_mount_window_pending: bool,
    page_width: f32,
    mount_window_task: Option<Task<()>>,
    grow_task: Option<Task<()>>,
}

impl ProfileContentState {
    fn new() -> Self {
        Self {
            mounted_tabs: HashSet::new(),
            mount_window_generation: 0,
            initial_mount_window_pending: true,
            page_width: 1.0,
            mount_window_task: None,
            grow_task: None,
        }
    }

    fn effective_mounted_tabs(
        &self,
        visible_tabs: &[ProfileContentTab],
        selected_tab: ProfileContentTab,
    ) -> HashSet<ProfileContentTab> {
        let mut tabs: HashSet<ProfileContentTab> =
            self.mounted_tabs.iter().copied().filter(|tab| visible_tabs.contains(tab)).collect();
        tabs.insert(selected_tab);
        tabs
    }

    fn normalize_mounted_tabs(
        &mut self,
        tab: ProfileContentTab,
        mount: &ProfileContentMount,
        cx: &mut Context<Self>,
    ) {
        let tabs = ProfileContentLayoutPolicy::retained_tabs_during_transition(
            &mount.visible_tabs,
            tab,
            &self.mounted_tabs,
        );
        self.replace_mounted_tabs(tabs, mount, cx);
    }

    fn grow_mounted_tabs_after_continuity_frame(
        &mut self,
        tab: ProfileContentTab,
        mount: &ProfileContentMount,
        cx: &mut Context<Self>,
    ) {
        let mount = mount.clone();
        self.grow_task = Some(cx.spawn(async move |state: WeakEntity<Self>, cx: &mut AsyncApp| {
            let _ = state.update(cx, |state, cx| {
                if mount.selected_tab.get(cx) != tab {
                    return;
                }
                let tabs = ProfileContentLayoutPolicy::grown_tabs_during_transition(
                    &mount.visible_tabs,
                    tab,
                    &state.mounted_tabs,
                );
                state.replace_mounted_tabs(tabs, &mount, cx);
            });
        }));
    }

    fn schedule_mount_window(
        &mut self,
        around: ProfileContentTab,
        delay: TimeInterval,
        mount: &ProfileContentMount,
        cx: &mut Context<Self>,
    ) {
        self.mount_window_generation = self.mount_window_generation.wrapping_add(1);
        let generation = self.mount_window_generation;
        let mount = mount.clone();
        let executor = cx.background_executor().clone();
        self.mount_window_task = Some(cx.spawn(async move |state: WeakEntity<Self>, cx: &mut AsyncApp| {
            if delay > 0.0 {
                executor.timer(Duration::from_secs_f64(delay)).await;
            }
            loop {
                let retry = state.update(cx, |state, cx| {
                    state.complete_mount_window_when_idle(generation, around, &mount, cx)
                });
                match retry {
                    Ok(Some(retry)) => executor.timer(Duration::from_secs_f64(retry)).await,
                    Ok(None) | Err(_) => return,
                }
            }
        }));
    }

    fn complete_mount_window_when_idle(
        &mut self,
        generation: u64,
        around: ProfileContentTab,
        mount: &ProfileContentMount,
        cx: &mut Context<Self>,
    ) -> Option<TimeInterval> {
        if self.mount_window_generation != generation || mount.selected_tab.get(cx) != around {
            return None;
        }
        let (bound, idle) = {
            let coordinator = mount.scroll_coordinator.read(cx);
            (coordinator.has_outer_binding_for_page_preload(), coordinator.is_idle_for_page_preload())
        };
        if !bound {
            return Some(ProfileHorizontalPagerPolicy::PAGE_PRELOAD_BINDING_RETRY_DELAY);
        }
        if !idle {
            return Some(ProfileHorizontalPagerPolicy::PAGE_PRELOAD_IDLE_RETRY_DELAY);
        }

        self.initial_mount_window_pending = false;
        let tabs = ProfileContentLayoutPolicy::warmed_tabs(&mount.visible_tabs, around, &self.mounted_tabs);
        self.replace_mounted_tabs(tabs, mount, cx);
        None
    }

    fn replace_mounted_tabs(
        &mut self,
        tabs: HashSet<ProfileContentTab>,
        mount: &ProfileContentMount,
        cx: &mut Context<Self>,
    ) {
        if self.mounted_tabs != tabs {
            self.mounted_tabs = tabs;
            cx.notify();
        }
        self.synchronize_prepared_directions(mount, cx);
    }

    fn synchronize_prepared_directions(&mut self, mount: &ProfileContentMount, cx: &mut Context<Self>) {
        let selected_tab = mount.selected_tab.get(cx);
        let directions = ProfileContentLayoutPolicy::prepared_horizontal_directions(
            &mount.visible_tabs,
            selected_tab,
            &self.effective_mounted_tabs(&mount.visible_tabs, selected_tab),
        );
        mount.scroll_coordinator.update(cx, |coordinator, cx| {
            coordinator.set_visible_horizontal_tabs(&mount.visible_tabs, cx);
            coordinator.set_prepared_horizontal_directions(directions, cx);
        });
    }

    fn on_appear(&mut self, mount: &ProfileContentMount, cx: &mut Context<Self>) {
        self.initial_mount_window_pending = true;
        let selected_tab = mount.selected_tab.get(cx);
        mount.scroll_coordinator.update(cx, |coordinator, cx| {
            coordinator.set_horizontal_paging_top_inset(ProfileViewportLayoutPolicy::HEADER_UNDERLAP, cx);
        });
        self.normalize_mounted_tabs(selected_tab, mount, cx);
        self.synchronize_prepared_directions(mount, cx);
        mount.scroll_coordinator.update(cx, |coordinator, cx| coordinator.set_active_tab(selected_tab, cx));
        self.schedule_mount_window(selected_tab, ProfileHorizontalPagerPolicy::INITIAL_PAGE_PRELOAD_DELAY, mount, cx);
    }

    fn on_selected_tab_change(&mut self, tab: ProfileContentTab, mount: &ProfileContentMount, cx: &mut Context<Self>) {
        mount.scroll_coordinator.update(cx, |coordinator, cx| coordinator.set_active_tab(tab, cx));
        self.normalize_mounted_tabs(tab, mount, cx);
        self.synchronize_prepared_directions(mount, cx);
        self.grow_mounted_tabs_after_continuity_frame(tab, mount, cx);
        let delay = if self.initial_mount_window_pending {
            ProfileHorizontalPagerPolicy::INITIAL_PAGE_PRELOAD_DELAY
        } else {
            ProfileHorizontalPagerPolicy::PAGE_PRELOAD_DELAY
        };
        self.schedule_mount_window(tab, delay, mount, cx);
    }

    fn on_visible_tabs_change(&mut self, mount: &ProfileContentMount, cx: &mut Context<Self>) {
        let selected_tab = mount.selected_tab.get(cx);
        self.normalize_mounted_tabs(selected_tab, mount, cx);
        self.synchronize_prepared_directions(mount, cx);
        self.schedule_mount_window(selected_tab, ProfileHorizontalPagerPolicy::INITIAL_PAGE_PRELOAD_DELAY, mount, cx);
    }

    fn on_scope_key_change(&mut self, mount: &ProfileContentMount, cx: &mut Context<Self>) {
        self.initial_mount_window_pending = true;
        let selected_tab = mount.selected_tab.get(cx);
        self.replace_mounted_tabs(HashSet::from([selected_tab]), mount, cx);
        self.synchronize_prepared_directions(mount, cx);
        self.schedule_mount_window(selected_tab, ProfileHorizontalPagerPolicy::INITIAL_PAGE_PRELOAD_DELAY, mount, cx);
    }

    fn on_disappear(&mut self, mount: &ProfileContentMount, cx: &mut Context<Self>) {
        self.mount_window_generation = self.mount_window_generation.wrapping_add(1);
        self.initial_mount_window_pending = true;
        self.mount_window_task = None;
        self.grow_task = None;
        mount.scroll_coordinator.update(cx, |coordinator, cx| {
            coordinator.set_prepared_horizontal_directions(HashSet::new(), cx);
        });
    }
}

#[derive(Clone, IntoElement)]
pub struct ProfileContent {
    pub id: ElementId,
    pub profile_tag: String,
    pub profile_user_id: String,
    pub is_own_profile: bool,
    pub foreign_goals: Vec<GoalItem>,
    pub foreign_plans: Vec<PlanItem>,
    pub foreign_closed_tabs: HashSet<ProfileContentTab>,
    pub foreign_owner_totally_blocked: bool,

    pub visible_tabs: Vec<ProfileContentTab>,
    pub selected_tab: Binding<ProfileContentTab>,
    pub publications_grid: Binding<bool>,
    pub threads_grid: Binding<bool>,
    pub viewport_height: f32,
    pub bottom_inset: f32,
    pub scope_key: String,
    pub scroll_coordinator: Entity<ProfileScrollCoordinator>,
    pub on_select_tab: ProfileSelectTab,
}

impl ProfileContent {
    const PAGE_EMPTY_STATE_PADDING: f32 = 24.0;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        visible_tabs: Vec<ProfileContentTab>,
        selected_tab: Binding<ProfileContentTab>,
        publications_grid: Binding<bool>,
        threads_grid: Binding<bool>,
        viewport_height: f32,
        bottom_inset: f32,
        scope_key: impl Into<String>,
        scroll_coordinator: Entity<ProfileScrollCoordinator>,
        on_select_tab: ProfileSelectTab,
    ) -> Self {
        Self {
            id: id.into(),
            profile_tag: String::new(),
            profile_user_id: String::new(),
            is_own_profile: true,
            foreign_goals: Vec::new(),
            foreign_plans: Vec::new(),
            foreign_closed_tabs: HashSet::new(),
            foreign_owner_totally_blocked: false,
            visible_tabs,
            selected_tab,
            publications_grid,
            threads_grid,
            viewport_height,
            bottom_inset,
            scope_key: scope_key.into(),
            scroll_coordinator,
            on_select_tab,
        }
    }

    pub fn profile_tag(mut self, profile_tag: impl Into<String>) -> Self {
        self.profile_tag = profile_tag.into();
        self
    }

    pub fn profile_user_id(mut self, profile_user_id: impl Into<String>) -> Self {
        self.profile_user_id = profile_user_id.into();
        self
    }

    pub fn is_own_profile(mut self, is_own_profile: bool) -> Self {
        self.is_own_profile = is_own_profile;
        self
    }

    pub fn foreign_goals(mut self, foreign_goals: Vec<GoalItem>) -> Self {
        self.foreign_goals = foreign_goals;
        self
    }

    pub fn foreign_plans(mut self, foreign_plans: Vec<PlanItem>) -> Self {
        self.foreign_plans = foreign_plans;
        self
    }

    pub fn foreign_closed_tabs(mut self, foreign_closed_tabs: HashSet<ProfileContentTab>) -> Self {
        self.foreign_closed_tabs = foreign_closed_tabs;
        self
    }

    pub fn foreign_owner_totally_blocked(mut self, foreign_owner_totally_blocked: bool) -> Self {
        self.foreign_owner_totally_blocked = foreign_owner_totally_blocked;
        self
    }

    fn mount(&self) -> ProfileContentMount {
        ProfileContentMount {
            visible_tabs: self.visible_tabs.clone(),
            selected_tab: self.selected_tab.clone(),
            scroll_coordinator: self.scroll_coordinator.clone(),
        }
    }

    fn observe_lifecycle(
        &self,
        state: &Entity<ProfileContentState>,
        selected_tab: ProfileContentTab,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mount = self.mount();
        let on_tab = (state.clone(), mount.clone());
        on_change(child_id(&self.id, "selected-tab"), selected_tab, false, window, cx, move |_, tab, _, cx| {
            let (state, mount) = on_tab;
            state.update(cx, |state, cx| state.on_selected_tab_change(*tab, &mount, cx));
        });
        let on_tabs = (state.clone(), mount.clone());
        on_change(child_id(&self.id, "visible-tabs"), self.visible_tabs.clone(), false, window, cx, move |_, _, _, cx| {
            let (state, mount) = on_tabs;
            state.update(cx, |state, cx| state.on_visible_tabs_change(&mount, cx));
        });
        let on_scope = (state.clone(), mount);
        on_change(child_id(&self.id, "scope-key"), self.scope_key.clone(), false, window, cx, move |_, _, _, cx| {
            let (state, mount) = on_scope;
            state.update(cx, |state, cx| state.on_scope_key_change(&mount, cx));
        });

        let is_own_profile = self.is_own_profile;
        for (name, tab, fact) in [
            ("stories-tab-open", ProfileContentTab::Stories, RiseGuideSignal::ProfileStoriesTabOpen),
            ("goals-tab-open", ProfileContentTab::Goals, RiseGuideSignal::ProfileGoalsTabOpen),
            ("plans-tab-open", ProfileContentTab::Plans, RiseGuideSignal::ProfilePlansTabOpen),
        ] {
            let open = is_own_profile && selected_tab == tab;
            on_change(child_id(&self.id, name), open, true, window, cx, move |_, open, _, cx| {
                if is_own_profile {
                    GuideFacts::set(fact, *open, cx);
                }
            });
        }
    }

    /// timestamps in the grids own order so the scrubber names the month of the top row
    fn story_timestamps(&self, cx: &mut App) -> Vec<TimeInterval> {
        StoryServicesStore::shared(cx)
            .update(cx, |service, cx| service.profile_stories(&self.profile_user_id, StoryProfileScope::Profile, cx))
            .iter()
            .map(|story| story.created_at_ms as TimeInterval / 1_000.0)
            .collect()
    }

    fn page_bottom_spacer_height(&self) -> f32 {
        if !self.is_own_profile && self.bottom_inset <= 0.0 {
            return 0.0;
        }
        let base = self.bottom_inset.max(72.0) + 24.0;
        if !self.is_own_profile {
            return base;
        }
        base + ProfileCreateActionMetrics::SIDE + BottomChromePolicy::FLOATING_ACTION_GAP
    }

    /// tab_scroll inputs that reach the page as plain values, cached frame drops only when one changes
    fn page_key(&self, tab: ProfileContentTab, pager_height: f32, cx: &mut App) -> u64 {
        let mut hasher = DefaultHasher::new();
        tab.hash(&mut hasher);
        pager_height.to_bits().hash(&mut hasher);
        self.bottom_inset.to_bits().hash(&mut hasher);
        self.profile_tag.hash(&mut hasher);
        self.profile_user_id.hash(&mut hasher);
        self.scope_key.hash(&mut hasher);
        self.is_own_profile.hash(&mut hasher);
        self.foreign_owner_totally_blocked.hash(&mut hasher);
        self.foreign_closed_tabs.contains(&tab).hash(&mut hasher);
        self.scroll_coordinator.read(cx).page_scroll_view(tab.raw_value()).pan_enabled().hash(&mut hasher);
        let background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        for channel in [background.h, background.s, background.l, background.a] {
            channel.to_bits().hash(&mut hasher);
        }
        match tab {
            ProfileContentTab::Publications => self.publications_grid.get(cx).hash(&mut hasher),
            ProfileContentTab::Threads => self.threads_grid.get(cx).hash(&mut hasher),
            ProfileContentTab::Goals => self.foreign_goals.hash(&mut hasher),
            ProfileContentTab::Plans => self.foreign_plans.hash(&mut hasher),
            ProfileContentTab::Stories => {
                AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author().hash(&mut hasher);
                for timestamp in self.story_timestamps(cx) {
                    timestamp.to_bits().hash(&mut hasher);
                }
            }
            _ => {}
        }
        hasher.finish()
    }

    fn tab_scroll(&self, tab: ProfileContentTab, pager_height: f32, cx: &mut App) -> AnyElement {
        let body = self.tab_scroll_body(tab, pager_height, cx);
        let root = div().relative().size_full().child(body);
        if tab != ProfileContentTab::Stories {
            return root.into_any_element();
        }
        let scrubber = ProfileGridDateScrubberLayer::new(
            tab_id("profile-grid-date-scrubber", tab),
            self.story_timestamps(cx),
            StoriesGridMetrics::COLUMNS,
            StoriesGridMetrics::CELL_ASPECT,
            StoriesGridMetrics::SPACING,
            StoriesGridMetrics::HORIZONTAL_PADDING,
            tab.raw_value(),
            self.scroll_coordinator.clone(),
            ProfileViewportLayoutPolicy::page_content_top_inset(),
            self.bottom_inset.max(72.0) + 24.0,
        )
        .columns_for_width(StoriesGridMetrics::columns);
        root.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right_0()
                .w(px(ProfileGridDateScrubberLayer::LAYER_WIDTH))
                .child(scrubber),
        )
        .into_any_element()
    }

    fn tab_scroll_body(&self, tab: ProfileContentTab, pager_height: f32, cx: &mut App) -> AnyElement {
        let top_inset = ProfileViewportLayoutPolicy::page_content_top_inset();
        let spacer_height = self.page_bottom_spacer_height();
        let page_min_height = (pager_height - top_inset - spacer_height).max(152.0);
        let content = div()
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .child(div().flex_none().h(px(top_inset)))
            .child(div().flex().flex_col().w_full().min_h(px(page_min_height)).child(self.profile_page(tab, cx)))
            .child(div().flex_none().h(px(spacer_height)))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size(px(1.0))
                    .child(ProfileInnerScrollIntrospector::new(tab, self.scroll_coordinator.clone())),
            );
        let page_scroll = self.scroll_coordinator.read(cx).page_scroll_view(tab.raw_value());
        let scroller = scroll_view(tab_id("profile-tab-scroll", tab))
            .shows_indicators(false)
            .scroll_handle(page_scroll.handle())
            .restrict_scroll_to_axis(true)
            .scroll_disabled(!page_scroll.pan_enabled());
        ProfilePostRefreshModifier {
            tab,
            profile_user_id: self.profile_user_id.clone(),
            profile_tag: self.profile_tag.clone(),
        }
        .body(scroller)
        .child(
            env_scope::<SharedElementReturnScrollerKey>(Some(SharedElementReturnScrollerRef::new(
                &self.scroll_coordinator.read(cx).return_scroller(),
                tab.raw_value(),
                self.bottom_inset,
            )))
            .child(content),
        )
        .into_any_element()
    }

    fn profile_page(&self, tab: ProfileContentTab, cx: &mut App) -> AnyElement {
        if self.foreign_owner_totally_blocked
            && matches!(tab, ProfileContentTab::Publications | ProfileContentTab::Threads | ProfileContentTab::Riffs)
        {
            let (icon, text) = match tab {
                ProfileContentTab::Threads => ("text.bubble", tr("profile_threads_empty")),
                ProfileContentTab::Riffs => {
                    let riffs = tr("riffs");
                    ("play.rectangle", tr_args("shorts_empty", &[FormatArg::Text(riffs.into())]))
                }
                _ => ("photo.on.rectangle.angled", tr("profile_publications_empty")),
            };
            Self::page_empty_state(ChatProfileEmptyState::new(icon, text))
        } else if self.foreign_closed_tabs.contains(&tab) {
            Self::page_empty_state(ChatProfileEmptyState::new("lock", tr("profile_section_hidden")))
        } else {
            self.open_profile_page(tab, cx)
        }
    }

    fn page_empty_state(state: ChatProfileEmptyState) -> AnyElement {
        state.vertical_padding(Self::PAGE_EMPTY_STATE_PADDING).into_any_element()
    }

    fn posts_page(&self, uses_grid: bool, kind: RisePostProfileKind, cx: &mut App) -> AnyElement {
        if uses_grid {
            GridPosts::new(self.profile_tag.clone(), self.profile_user_id.clone(), kind).into_any_element()
        } else {
            Self::feed_column(ListPosts::new(self.profile_tag.clone(), self.profile_user_id.clone(), kind), cx)
        }
    }

    /// full post rows sit in the feed column in the middle of the center, like the posts tab
    fn feed_column(page: impl IntoElement, cx: &mut App) -> AnyElement {
        let width = rise_ui::theme(cx).feed.content_width;
        div()
            .w_full()
            .flex()
            .justify_center()
            .child(div().w_full().min_w_0().max_w(width).child(page))
            .into_any_element()
    }

    fn open_profile_page(&self, tab: ProfileContentTab, cx: &mut App) -> AnyElement {
        match tab {
            ProfileContentTab::Publications => {
                self.posts_page(self.publications_grid.get(cx), RisePostProfileKind::Publications, cx)
            }
            ProfileContentTab::Riffs => {
                GridRiffs::new(self.profile_tag.clone(), self.profile_user_id.clone()).into_any_element()
            }
            ProfileContentTab::Threads => self.posts_page(self.threads_grid.get(cx), RisePostProfileKind::Threads, cx),
            ProfileContentTab::Stories => {
                let is_owner = self.is_own_profile || AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author();
                StoriesGridPage::new(self.profile_user_id.clone(), is_owner).into_any_element()
            }
            ProfileContentTab::Comments => {
                Self::feed_column(ProfileComments::new(tab_id("profile-tab-page", tab), self.profile_user_id.clone()), cx)
            }
            ProfileContentTab::Reposts => {
                GridReposts::new(self.profile_tag.clone(), self.profile_user_id.clone()).into_any_element()
            }
            ProfileContentTab::Mentions => {
                GridMentions::new(tab_id("profile-tab-page", tab), self.profile_tag.clone(), self.profile_user_id.clone())
                    .into_any_element()
            }
            ProfileContentTab::Favorites => {
                GridFavorites::new(tab_id("profile-tab-page", tab), self.profile_tag.clone(), self.profile_user_id.clone())
                    .into_any_element()
            }
            ProfileContentTab::Gifts => UserGiftsTab::new(self.profile_user_id.clone()).into_any_element(),
            ProfileContentTab::Goals => profile_content_goals_hooks()
                .map(|hooks| {
                    (hooks.goals_page)(tab_id("profile-tab-page", tab), self.is_own_profile, self.foreign_goals.clone())
                })
                .unwrap_or_else(|| gpui::Empty.into_any_element()),
            ProfileContentTab::Plans => Plans::new(tab_id("profile-tab-page", tab))
                .is_own_profile(self.is_own_profile)
                .foreign_plans(self.foreign_plans.clone())
                .into_any_element(),
        }
    }
}

impl RenderOnce for ProfileContent {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let container_height = ProfileViewportLayoutPolicy::container_height(self.viewport_height);
        let pager_height = ProfileViewportLayoutPolicy::pager_height(container_height);
        let header_underlap = ProfileViewportLayoutPolicy::HEADER_UNDERLAP;
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| ProfileContentState::new());
        let selected_tab = self.selected_tab.get(cx);
        self.observe_lifecycle(&state, selected_tab, window, cx);

        let (rendered_tabs, page_width) = {
            let current = state.read(cx);
            (current.effective_mounted_tabs(&self.visible_tabs, selected_tab), current.page_width)
        };

        let on_appear = (state.clone(), self.mount());
        let on_disappear = (state.clone(), self.mount());
        let is_own_profile = self.is_own_profile;
        let appear_id = child_id(&self.id, "appear");
        let disappear_id = child_id(&self.id, "disappear");
        let tab_bar_reserve = window.use_keyed_state(child_id(&self.id, "tab-bar-bounds"), cx, |_, cx| {
            let insets = BottomChromeInsets::shared(cx);
            let insets_observer = cx.observe(&insets, |reserve: &mut ProfileTabBarReserve, _, cx| {
                let reserves = ProfileTabBar::meets_create_action(reserve.bounds, cx);
                if reserve.reserves != reserves {
                    reserve.reserves = reserves;
                    cx.notify();
                }
            });
            ProfileTabBarReserve {
                bounds: None,
                reserves: false,
                _insets_observer: insets_observer,
            }
        });
        let reserves_create_action = is_own_profile && tab_bar_reserve.read(cx).reserves;
        let tab_bar = ProfileTabBar::new(
            self.visible_tabs.clone(),
            selected_tab,
            self.publications_grid.clone(),
            self.threads_grid.clone(),
            self.scroll_coordinator.clone(),
            self.on_select_tab.clone(),
        )
        .reserves_create_action(reserves_create_action);
        let this = Rc::new(self);
        let page: ProfilePage = {
            let this = this.clone();
            Rc::new(move |tab: ProfileContentTab, _: &mut Window, cx: &mut App| this.tab_scroll(tab, pager_height, cx))
        };
        let page_key: ProfilePageKey = {
            let this = this.clone();
            Rc::new(move |tab: ProfileContentTab, cx: &mut App| this.page_key(tab, pager_height, cx))
        };

        div()
            .relative()
            .w_full()
            .h(px(container_height))
            .bg(background)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(-header_underlap))
                    .h(px(pager_height))
                    .child(ProfilePagesPager {
                        visible_tabs: this.visible_tabs.clone(),
                        selected_tab,
                        rendered_tabs,
                        scroll_coordinator: this.scroll_coordinator.clone(),
                        state,
                        page_width,
                        height: pager_height,
                        page,
                        page_key,
                    }),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(-header_underlap))
                    .h(px(ProfilePinnedChromePlate::height()))
                    .child(ProfilePinnedChromePlate::new()),
            )
            .child(div().absolute().left_0().right_0().top_0().child(tab_bar).child(measure(
                child_id(&this.id, "tab-bar-measure"),
                move |bounds, _, cx| {
                    let reserves = ProfileTabBar::meets_create_action(Some(bounds), cx);
                    tab_bar_reserve.update(cx, |reserve, cx| {
                        reserve.bounds = Some(bounds);
                        if reserve.reserves != reserves {
                            reserve.reserves = reserves;
                            cx.notify();
                        }
                    });
                },
            )))
            .children(
                ProfileUnderlapTouchRouter::is_needed()
                    .then(|| ProfileUnderlapTouchRouter::new(this.scroll_coordinator.clone(), header_underlap)),
            )
            .on_appear(appear_id, move |_, cx| {
                let (state, mount) = on_appear;
                state.update(cx, |state, cx| state.on_appear(&mount, cx));
            })
            .on_disappear(disappear_id, move |_, cx| {
                let (state, mount) = on_disappear;
                state.update(cx, |state, cx| state.on_disappear(&mount, cx));
                if is_own_profile {
                    GuideFacts::set(RiseGuideSignal::ProfileStoriesTabOpen, false, cx);
                    GuideFacts::set(RiseGuideSignal::ProfileGoalsTabOpen, false, cx);
                    GuideFacts::set(RiseGuideSignal::ProfilePlansTabOpen, false, cx);
                }
            })
    }
}

struct ProfilePostRefreshModifier {
    tab: ProfileContentTab,
    profile_user_id: String,
    profile_tag: String,
}

impl ProfilePostRefreshModifier {
    fn body(self, content: ScrollView) -> ScrollView {
        let Self { tab, profile_user_id, profile_tag } = self;
        let kind = match tab {
            ProfileContentTab::Publications => RisePostProfileKind::Publications,
            ProfileContentTab::Riffs => RisePostProfileKind::Riffs,
            ProfileContentTab::Threads => RisePostProfileKind::Threads,
            ProfileContentTab::Reposts => RisePostProfileKind::Reposts,
            ProfileContentTab::Mentions => RisePostProfileKind::Mentions,
            ProfileContentTab::Favorites => RisePostProfileKind::Favorites,
            ProfileContentTab::Stories => {
                return content.refreshable(move |_, cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.refresh_profile_stories(Some(&profile_user_id), StoryProfileScope::Profile, cx);
                    });
                    Task::ready(())
                });
            }
            ProfileContentTab::Gifts => {
                return content.refreshable(move |_, cx| {
                    let owner_id = profile_user_id.clone();
                    cx.spawn(async move |cx: &mut AsyncApp| GiftsActionsStore::refresh_tab(&owner_id, cx).await)
                });
            }
            ProfileContentTab::Comments | ProfileContentTab::Goals | ProfileContentTab::Plans => return content,
        };
        content.refreshable(move |_, cx| {
            PostInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.refresh_profile(&profile_user_id, &profile_tag, kind, cx))
        })
    }
}

/// blurred plate under the floating header once tabs pin so posts dont show through
/// also used by the chat profile
#[derive(IntoElement)]
pub struct ProfilePinnedChromePlate {
    pub pinned_min_y: f32,
}

impl ProfilePinnedChromePlate {
    pub const REVEAL_DISTANCE: f32 = 24.0;

    pub fn style() -> ScrollChromeShadowStyle {
        ScrollChromeShadowStyle::list_shell_blurred_header()
    }

    pub fn fade_height() -> f32 {
        Self::style().overflow_below_header
    }

    pub fn height() -> f32 {
        ProfileViewportLayoutPolicy::page_content_top_inset() + Self::fade_height()
    }

    pub fn new() -> Self {
        Self { pinned_min_y: 0.0 }
    }

    pub fn pinned_min_y(mut self, pinned_min_y: f32) -> Self {
        self.pinned_min_y = pinned_min_y;
        self
    }
}

impl Default for ProfilePinnedChromePlate {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for ProfilePinnedChromePlate {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let page = rise_ui::theme(cx).bg._100;
        let tint_peak = if ThemeStore::shared(cx).read(cx).resolved_appearance() == RiseThemeAppearance::Dark {
            ScrollEdgeBlurFalloff::HEADER_TINT_PEAK_DARK
        } else {
            ScrollEdgeBlurFalloff::HEADER_TINT_PEAK
        };
        let blur_stops = ScrollEdgeBlurFalloff::tint_stops_with_peak(
            page,
            ProfileViewportLayoutPolicy::page_content_top_inset() / Self::height(),
            tint_peak,
            accessibility_reduce_transparency(cx),
        );
        let stops = ScrollChromeShadowUi::new(Self::style()).stops(cx);
        let pinned_min_y = self.pinned_min_y;
        canvas(
            move |bounds, _, cx: &mut App| {
                let frame = GuideScrollHost::current().and_then(|host| host.frame());
                let reveal = frame.map_or(1.0, |frame| {
                    let top = f32::from(bounds.origin.y - frame.origin.y) - pinned_min_y;
                    (1.0 - top / Self::REVEAL_DISTANCE).clamp(0.0, 1.0)
                });
                (reveal, layout_direction(cx))
            },
            move |bounds, (reveal, direction): (f32, LayoutDirection), window, _| {
                if reveal <= 0.0 {
                    return;
                }
                for layer in [&blur_stops, &stops] {
                    let faded = layer
                        .iter()
                        .map(|stop| GradientStop::new(stop.color.opacity(reveal), stop.location))
                        .collect();
                    LinearGradient::new(faded, UnitPoint::TOP, UnitPoint::BOTTOM).paint(bounds, direction, window);
                }
            },
        )
        .size_full()
    }
}

#[derive(IntoElement)]
pub struct ProfileTabBar {
    pub visible_tabs: Vec<ProfileContentTab>,
    pub selected_tab: ProfileContentTab,
    pub publications_grid: Binding<bool>,
    pub threads_grid: Binding<bool>,
    pub scroll_coordinator: Entity<ProfileScrollCoordinator>,
    pub on_select_tab: ProfileSelectTab,
    pub reserves_create_action: bool,
}

impl ProfileTabBar {
    pub const CREATE_ACTION_RESERVE: f32 =
        MainTabChrome::FLOATING_BAR_HORIZONTAL_INSET + ProfileCreateActionMetrics::SIDE + 8.0 - 12.0;

    pub fn reserves_create_action(mut self, reserves: bool) -> Self {
        self.reserves_create_action = reserves;
        self
    }

    /// true when a drawn floating action overlaps the bar frame in the window
    pub fn meets_create_action(bar: Option<Bounds<Pixels>>, cx: &mut App) -> bool {
        let Some(bar) = bar else {
            return false;
        };
        BottomChromeInsets::shared(cx)
            .read(cx)
            .frames_in_window(BottomChromeOwner::FloatingAction)
            .iter()
            .any(|action| {
                action.top() < bar.bottom()
                    && action.bottom() > bar.top()
                    && action.left() < bar.right()
                    && action.right() > bar.left()
            })
    }

    /// anchor a guide points at for this tab, none for tabs without one
    pub fn guide_anchor(tab: ProfileContentTab) -> Option<RiseGuideAnchor> {
        match tab {
            ProfileContentTab::Stories => Some(RiseGuideAnchor::ProfileStoriesTab),
            ProfileContentTab::Goals => Some(RiseGuideAnchor::ProfileGoalsTab),
            ProfileContentTab::Plans => Some(RiseGuideAnchor::ProfilePlansTab),
            _ => None,
        }
    }

    pub fn new(
        visible_tabs: Vec<ProfileContentTab>,
        selected_tab: ProfileContentTab,
        publications_grid: Binding<bool>,
        threads_grid: Binding<bool>,
        scroll_coordinator: Entity<ProfileScrollCoordinator>,
        on_select_tab: ProfileSelectTab,
    ) -> Self {
        Self {
            visible_tabs,
            selected_tab,
            publications_grid,
            threads_grid,
            scroll_coordinator,
            on_select_tab,
            reserves_create_action: false,
        }
    }

    fn select(
        visible_tabs: &[ProfileContentTab],
        on_select_tab: &ProfileSelectTab,
        index: i64,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(tab) = usize::try_from(index).ok().and_then(|index| visible_tabs.get(index)) else {
            return;
        };
        on_select_tab(*tab, window, cx);
    }

    fn selected_index_binding(&self) -> Binding<i64> {
        let selected_index =
            self.visible_tabs.iter().position(|tab| *tab == self.selected_tab).unwrap_or(0) as i64;
        let visible_tabs = self.visible_tabs.clone();
        let on_select_tab = self.on_select_tab.clone();
        Binding::new(
            move |_| selected_index,
            move |index, window, cx| Self::select(&visible_tabs, &on_select_tab, index, window, cx),
        )
    }

    fn selected_layout_binding(&self) -> Binding<bool> {
        match self.selected_tab {
            ProfileContentTab::Threads => self.threads_grid.clone(),
            _ => self.publications_grid.clone(),
        }
    }
}

impl RenderOnce for ProfileTabBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        gpui::hear_entity_through_events_only(self.scroll_coordinator.entity_id());
        let axis_owners = self.scroll_coordinator.read(cx).axis_owners();
        let (capsule_fill, capsule_stroke) = {
            let theme = ThemeStore::shared(cx).read(cx).current_theme();
            (LiquidChrome::surface_fill(theme, 0), theme.border._200)
        };
        let tabs = self
            .visible_tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                TabHeaderConfig::new(index as i64, tr(tab.title_key())).guide_anchor(Self::guide_anchor(*tab))
            })
            .collect();
        let visible_tabs = self.visible_tabs.clone();
        let on_select_tab = self.on_select_tab.clone();
        let progress_coordinator = self.scroll_coordinator.clone();
        let base_coordinator = self.scroll_coordinator.clone();
        let header = AnimatedTabsHeader::new(
            "profile-tab-bar-header",
            tabs,
            self.selected_index_binding(),
            Rc::new(move |cx: &App| progress_coordinator.read(cx).tabs_live_state.read(cx).drag_progress()),
            0.0,
            Rc::new(move |index: i64, window: &mut Window, cx: &mut App| {
                Self::select(&visible_tabs, &on_select_tab, index, window, cx)
            }),
        )
        .drag_base_index(Rc::new(move |cx: &App| base_coordinator.read(cx).tabs_live_state.read(cx).drag_base_index()))
        .shows_pill_background(false)
        .arrows_step_selection(true)
        .fade_surface(capsule_fill);

        let shows_layout_toggle =
            matches!(self.selected_tab, ProfileContentTab::Publications | ProfileContentTab::Threads);
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .w_full()
            .h(px(ProfileScrollMetrics::TABS_BAR_HEIGHT))
            .px(px(12.0))
            .py(px(12.0))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h(AnimatedTabsMetrics::BAR_HEIGHT)
                    .capsule()
                    .bg(capsule_fill)
                    .block_mouse_except_scroll()
                    .child(
                        env_scope::<AnimatedTabsPagerAxisOwnersKey>(Some(axis_owners))
                            .child(env_scope::<LayoutDirectionKey>(LayoutDirection::LeftToRight).child(header)),
                    )
                    .child(centred_stroke(capsule_stroke, 0.5, Shape::Capsule)),
            )
            .child(
                div()
                    .flex_none()
                    .when(shows_layout_toggle, |slot| slot.block_mouse_except_scroll())
                    .when(!shows_layout_toggle, |slot| slot.invisible())
                    .child(PostLayoutToggleUi::new("profile-tab-bar-layout-toggle", self.selected_layout_binding())),
            )
            .children(
                self.reserves_create_action
                    .then(|| div().flex_none().w(px(Self::CREATE_ACTION_RESERVE - 8.0))),
            )
    }
}

#[derive(IntoElement)]
struct ProfilePagesPager {
    visible_tabs: Vec<ProfileContentTab>,
    selected_tab: ProfileContentTab,
    rendered_tabs: HashSet<ProfileContentTab>,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    state: Entity<ProfileContentState>,
    page_width: f32,
    height: f32,
    page: ProfilePage,
    page_key: ProfilePageKey,
}

impl RenderOnce for ProfilePagesPager {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let width = self.page_width.max(1.0);
        let selected_index =
            self.visible_tabs.iter().position(|tab| *tab == self.selected_tab).unwrap_or(0) as i64;
        let page_count = self.visible_tabs.len() as i64;
        let state = self.state;
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(measure("profile-pages-pager-width", move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                state.update(cx, |state, cx| {
                    if state.page_width != width {
                        state.page_width = width;
                        cx.notify();
                    }
                });
            }))
            .child(ProfilePagerStablePages {
                visible_tabs: self.visible_tabs,
                rendered_tabs: self.rendered_tabs,
                width,
                height: self.height,
                scroll_coordinator: self.scroll_coordinator.clone(),
                page: self.page,
                page_key: self.page_key,
            })
            .child(div().absolute().top_0().left_0().size(px(1.0)).child(ProfileHorizontalPagerIntrospector::new(
                self.scroll_coordinator,
                selected_index,
                page_count,
                width,
            )))
    }
}

/// page strip, the coordinator places page i at i page widths
/// so pages stay left to right even in rtl
/// motion is a prepaint offset
#[derive(IntoElement, Clone)]
struct ProfilePagerStablePages {
    visible_tabs: Vec<ProfileContentTab>,
    rendered_tabs: HashSet<ProfileContentTab>,
    width: f32,
    height: f32,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    page: ProfilePage,
    page_key: ProfilePageKey,
}

impl ProfilePagerStablePages {
    /// everything the strip draws that its view isnt notified about, a change rerenders the strip
    fn signature(&self, cx: &mut App) -> u64 {
        let mut hasher = DefaultHasher::new();
        for tab in &self.visible_tabs {
            tab.raw_value().hash(&mut hasher);
            let rendered = self.rendered_tabs.contains(tab);
            rendered.hash(&mut hasher);
            if rendered {
                (self.page_key)(*tab, cx).hash(&mut hasher);
            }
        }
        self.width.to_bits().hash(&mut hasher);
        self.height.to_bits().hash(&mut hasher);
        let coordinator = self.scroll_coordinator.read(cx);
        coordinator.horizontal_pager_host_scroll_view().content_offset_x().to_bits().hash(&mut hasher);
        coordinator.horizontal_pager_host_motion_view().transform_x().to_bits().hash(&mut hasher);
        hasher.finish()
    }

    fn strip(&self, drawn_tabs: &mut HashSet<ProfileContentTab>, window: &mut Window, cx: &mut App) -> AnyElement {
        let measured = self.width > 1.0;
        drawn_tabs.retain(|tab| self.visible_tabs.contains(tab));
        let pages: Vec<AnyElement> = self
            .visible_tabs
            .iter()
            .map(|tab| {
                let frame = div().flex_none().w(px(self.width)).h(px(self.height));
                if measured && !self.rendered_tabs.contains(tab) && drawn_tabs.contains(tab) {
                    let tab = *tab;
                    let host = window
                        .use_keyed_state(tab_id("profile-tab-page-host", tab), cx, move |_, cx| {
                            ProfileTabPageHostHolder { host: cx.new(|_| ProfileTabPageHost::new(tab)) }
                        })
                        .read(cx)
                        .host
                        .clone();
                    let view = AnyView::from(host)
                        .cached(StyleRefinement::default().size_full())
                        .replay_when_moved()
                        .into_any_element();
                    return frame.child(view).into_any_element();
                }
                if measured && self.rendered_tabs.contains(tab) {
                    drawn_tabs.insert(*tab);
                    let tab = *tab;
                    let host = window
                        .use_keyed_state(tab_id("profile-tab-page-host", tab), cx, move |_, cx| {
                            ProfileTabPageHostHolder { host: cx.new(|_| ProfileTabPageHost::new(tab)) }
                        })
                        .read(cx)
                        .host
                        .clone();
                    let key = (self.page_key)(tab, cx);
                    let page = self.page.clone();
                    let stale = host.update(cx, |host, _| {
                        host.set_page(page);
                        host.set_key(key)
                    });
                    let view = if stale {
                        AnyView::from(host).into_any_element()
                    } else {
                        AnyView::from(host)
                            .cached(StyleRefinement::default().size_full())
                            .replay_when_moved()
                            .into_any_element()
                    };
                    frame.child(view).into_any_element()
                } else {
                    frame.into_any_element()
                }
            })
            .collect();
        let (host, motion) = {
            let coordinator = self.scroll_coordinator.read(cx);
            (coordinator.horizontal_pager_host_scroll_view(), coordinator.horizontal_pager_host_motion_view())
        };
        let strip = div()
            .flex_none()
            .flex()
            .flex_row()
            .items_start()
            .w(px(self.width * self.visible_tabs.len() as f32))
            .h(px(self.height))
            .children(pages);
        env_scope::<ContainerWidthHintKey>(measured.then_some(self.width))
            .child(
                host.horizontal_pager_container("profile-pages-pager-strip")
                    .items_start()
                    .child(ProfileHorizontalMotionElement::new(motion, strip)),
            )
            .into_any_element()
    }
}

/// the strip as its own cached view, motion element asks it for slide frames
struct ProfilePagerStripView {
    props: Option<ProfilePagerStablePages>,
    signature: Option<u64>,
    drawn_tabs: HashSet<ProfileContentTab>,
}

impl ProfilePagerStripView {
    fn set_props(&mut self, props: ProfilePagerStablePages, signature: u64) -> bool {
        self.props = Some(props);
        if self.signature == Some(signature) {
            return false;
        }
        let drawn_before = self.signature.is_some();
        self.signature = Some(signature);
        drawn_before
    }
}

impl Render for ProfilePagerStripView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let drawn_tabs = &mut self.drawn_tabs;
        let strip = self.props.as_ref().map(|props| props.strip(drawn_tabs, window, cx));
        div().size_full().children(strip)
    }
}

struct ProfilePagerStripHolder {
    view: Entity<ProfilePagerStripView>,
}

impl RenderOnce for ProfilePagerStablePages {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let signature = self.signature(cx);
        let view = window
            .use_keyed_state(ElementId::Name(SharedString::new_static("profile-pages-strip-view")), cx, |_, cx| {
                ProfilePagerStripHolder { view: cx.new(|_| ProfilePagerStripView { props: None, signature: None, drawn_tabs: HashSet::new() }) }
            })
            .read(cx)
            .view
            .clone();
        let stale = view.update(cx, |view, _| view.set_props(self, signature));
        if stale {
            AnyView::from(view).into_any_element()
        } else {
            AnyView::from(view).cached(StyleRefinement::default().size_full()).into_any_element()
        }
    }
}
