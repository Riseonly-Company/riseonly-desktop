use std::cell::Cell;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, AppContext, AsyncApp, Context, ElementId, Empty, Entity, EntityId, InteractiveElement,
    IntoElement, ParentElement, Render, RenderOnce, SharedString, StyleRefinement, Styled, Task, WeakEntity, Window,
    canvas, div, px,
};
use rise_chat::ChatType;
use rise_core::foundation::foundation_date::TimeInterval;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::hold_context_menu_locks_scroll;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::{EnvKey, LayoutDirection, LayoutDirectionKey, env, env_scope};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};

use crate::core::ui::animated_tabs_ui::animated_tabs_header::AnimatedTabsHeader;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager_gesture_gate::AnimatedTabsPagerAxisOwnersKey;
use crate::core::ui::profile_grid_date_scrubber::ProfileGridDateScrubberLayer;
use crate::core::widgets::wrappers::list_shell_ui::ListShellFloatingTopInsetKey;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatDecorativeBackdrop;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::{
    ChatProfileEmptyState, ChatProfilePageScrollKey, ChatProfilePinnedTabsBottom, ChatProfilePinnedTabsBottomKey,
    ChatProfileUsesParentScrollKey,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_tab::{
    ChatProfileTab, ChatProfileTabExt,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_fav_tab::ChatProfileFavTab;
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_file_tab::ChatProfileFileTab;
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_gif_tab::ChatProfileGifTab;
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_groups_tab::ChatProfileGroupsTab;
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_link_tab::ChatProfileLinkTab;
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_media_tab::{
    ChatProfileMediaDateScrubberLayer, ChatProfileMediaTab,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_members_tab::ChatProfileMembersTab;
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_music_tab::ChatProfileMusicTab;
use crate::modules::chat::components::tabs::chat_profile_tabs::pages::chat_profile_voice_tab::ChatProfileVoiceTab;
use crate::modules::gifts::components::user_gifts_tab::UserGiftsTab;
use crate::modules::story::components::stories_grid::stories_grid_cell::StoriesGridMetrics;
use crate::modules::story::components::stories_grid::stories_grid_page::StoriesGridPage;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_content::profile_content::ProfilePinnedChromePlate;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::{
    ProfileHorizontalMotionElement, ProfileHorizontalPagerIntrospector, ProfileHorizontalPagerPolicy,
    ProfileInnerScrollIntrospector, ProfileScrollCoordinator, ProfileScrollMetrics,
};
use crate::modules::user::components::profile_content::profile_underlap_touch_router::ProfileUnderlapTouchRouter;
use rise_ui::liquid_chrome::LiquidChrome;
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::{
    SharedElementReturnScrollerKey, SharedElementReturnScrollerRef,
};

pub enum ChatProfileViewportLayoutPolicy {}

impl ChatProfileViewportLayoutPolicy {
    pub const OUTER_SPACING: f32 = 14.0;
    pub const HEADER_UNDERLAP: f32 = ProfileScrollMetrics::CHAT_PINNED_TABS_TOP;
    pub const PINNED_GAP_FILL: f32 = 14.0;

    pub fn container_height(viewport_height: f32) -> f32 {
        (viewport_height - ProfileScrollMetrics::CHAT_PINNED_TABS_TOP).max(220.0)
    }

    /// height of the pinned container
    /// scroller starts under the floating title bar and pins tabs near the top
    /// viewport height alone is too short
    pub fn pinned_container_height(viewport_height: f32, floating_top_inset: f32) -> f32 {
        let pinned_top = ProfileScrollMetrics::CHAT_PINNED_TABS_TOP.min(floating_top_inset.max(0.0));
        Self::container_height(viewport_height) + (ProfileScrollMetrics::CHAT_PINNED_TABS_TOP - pinned_top)
    }

    pub fn pager_height(container_height: f32) -> f32 {
        container_height + Self::HEADER_UNDERLAP
    }

    pub fn page_content_top_inset() -> f32 {
        Self::HEADER_UNDERLAP + ProfileScrollMetrics::TABS_BAR_HEIGHT
    }
}

pub enum ChatProfilePaneMountPolicy {}

impl ChatProfilePaneMountPolicy {
    /// set of tabs to keep mounted around the selected one, radius 1 by default
    pub fn window(visible_tabs: &[ChatProfileTab], selected_tab: ChatProfileTab) -> HashSet<ChatProfileTab> {
        Self::window_with_radius(visible_tabs, selected_tab, 1)
    }

    pub fn window_with_radius(
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        radius: usize,
    ) -> HashSet<ChatProfileTab> {
        let Some(index) = visible_tabs.iter().position(|tab| *tab == selected_tab) else {
            return HashSet::from([selected_tab]);
        };
        let lower = index.saturating_sub(radius);
        let upper = (index + radius).min(visible_tabs.len() - 1);
        visible_tabs[lower..=upper].iter().copied().collect()
    }

    pub fn retained_during_transition(
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mounted_tabs: &HashSet<ChatProfileTab>,
    ) -> HashSet<ChatProfileTab> {
        let mut tabs: HashSet<ChatProfileTab> =
            mounted_tabs.intersection(&Self::window(visible_tabs, selected_tab)).copied().collect();
        tabs.insert(selected_tab);
        tabs
    }

    pub fn grown_during_transition(
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mounted_tabs: &HashSet<ChatProfileTab>,
    ) -> HashSet<ChatProfileTab> {
        let mut tabs: HashSet<ChatProfileTab> = mounted_tabs
            .intersection(&Self::window_with_radius(visible_tabs, selected_tab, 2))
            .copied()
            .collect();
        tabs.extend(Self::window(visible_tabs, selected_tab));
        tabs
    }

    pub fn prepared_directions(
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mounted_tabs: &HashSet<ChatProfileTab>,
    ) -> HashSet<i64> {
        let Some(index) = visible_tabs.iter().position(|tab| *tab == selected_tab) else {
            return HashSet::new();
        };
        [-1_i64, 1]
            .into_iter()
            .filter(|direction| {
                usize::try_from(index as i64 + direction)
                    .ok()
                    .and_then(|target| visible_tabs.get(target))
                    .is_some_and(|tab| mounted_tabs.contains(tab))
            })
            .collect()
    }
}

type ChatProfileSelectTab = Rc<dyn Fn(i64, &mut Window, &mut App)>;
type ChatProfilePage = Rc<dyn Fn(ChatProfileTab, &mut Window, &mut App) -> AnyElement>;
/// everything a tab page reads from its host that no observed store reports
/// change of it drops the cached page frame
type ChatProfilePageKey = Rc<dyn Fn(ChatProfileTab, &mut App) -> u64>;

/// one mounted tab page as its own cached view
/// renders only when stores or scroll or key changed, offscreen replays
struct ChatProfileTabPageHost {
    tab: ChatProfileTab,
    page: Option<ChatProfilePage>,
    scroll_coordinator: Option<EntityId>,
}

struct ChatProfileTabPageHostHolder {
    host: Entity<ChatProfileTabPageHost>,
}

impl ChatProfileTabPageHost {
    fn new(tab: ChatProfileTab) -> Self {
        Self { tab, page: None, scroll_coordinator: None }
    }

    fn set_page(&mut self, page: ChatProfilePage, scroll_coordinator: EntityId) {
        self.page = Some(page);
        self.scroll_coordinator = Some(scroll_coordinator);
    }
}

impl Render for ChatProfileTabPageHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tab;
        if let Some(coordinator) = self.scroll_coordinator {
            gpui::hear_entity_through_events_only(coordinator);
        }
        let element = self.page.clone().map(|page| page(tab, window, cx));
        div().relative().size_full().children(element)
    }
}

struct ChatProfileTabPageKey;

impl EnvKey for ChatProfileTabPageKey {
    type Value = u64;

    fn default_value() -> Self::Value {
        0
    }

    fn fingerprint(value: &Self::Value) -> Option<u64> {
        Some(*value)
    }
}

/// what the deferred mount steps see of the view
#[derive(Clone)]
struct ChatProfileTabsMount {
    active_tab: Binding<i64>,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
}

/// view state plus strip geometry and the latest tab list a deferred step compares against
struct ChatProfileTabsState {
    mounted_tabs: HashSet<ChatProfileTab>,
    mount_generation: u64,
    initial_mount_pending: bool,
    page_width: f32,
    current_tabs: Vec<ChatProfileTab>,
    mount_window_task: Option<Task<()>>,
    grow_task: Option<Task<()>>,
}

impl ChatProfileTabsState {
    fn new() -> Self {
        Self {
            mounted_tabs: HashSet::new(),
            mount_generation: 0,
            initial_mount_pending: true,
            page_width: 1.0,
            current_tabs: Vec::new(),
            mount_window_task: None,
            grow_task: None,
        }
    }

    fn effective_mounted_tabs(
        &self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
    ) -> HashSet<ChatProfileTab> {
        let mut tabs: HashSet<ChatProfileTab> =
            self.mounted_tabs.iter().copied().filter(|tab| visible_tabs.contains(tab)).collect();
        tabs.insert(selected_tab);
        tabs
    }

    /// true when the tab list and the selected tab are still the same as when the step was scheduled
    fn selection_is_current(
        &self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mount: &ChatProfileTabsMount,
        cx: &App,
    ) -> bool {
        self.current_tabs == visible_tabs
            && usize::try_from(mount.active_tab.get(cx))
                .ok()
                .and_then(|index| self.current_tabs.get(index))
                .is_some_and(|tab| *tab == selected_tab)
    }

    fn normalize_mounted_tabs(
        &mut self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        cx: &mut Context<Self>,
    ) {
        let tabs = ChatProfilePaneMountPolicy::retained_during_transition(
            visible_tabs,
            selected_tab,
            &self.mounted_tabs,
        );
        self.replace_mounted_tabs(tabs, cx);
    }

    fn grow_mounted_tabs_after_continuity_frame(
        &mut self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mount: &ChatProfileTabsMount,
        cx: &mut Context<Self>,
    ) {
        let visible_tabs = visible_tabs.to_vec();
        let mount = mount.clone();
        self.grow_task = Some(cx.spawn(async move |state: WeakEntity<Self>, cx: &mut AsyncApp| {
            let _ = state.update(cx, |state, cx| {
                if !state.selection_is_current(&visible_tabs, selected_tab, &mount, cx) {
                    return;
                }
                let tabs = ChatProfilePaneMountPolicy::grown_during_transition(
                    &visible_tabs,
                    selected_tab,
                    &state.mounted_tabs,
                );
                state.replace_mounted_tabs(tabs, cx);
                state.synchronize_prepared_directions(&visible_tabs, selected_tab, &mount, cx);
            });
        }));
    }

    fn schedule_mount_window(
        &mut self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        delay: TimeInterval,
        mount: &ChatProfileTabsMount,
        cx: &mut Context<Self>,
    ) {
        self.mount_generation = self.mount_generation.wrapping_add(1);
        let generation = self.mount_generation;
        let visible_tabs = visible_tabs.to_vec();
        let mount = mount.clone();
        let executor = cx.background_executor().clone();
        self.mount_window_task = Some(cx.spawn(async move |state: WeakEntity<Self>, cx: &mut AsyncApp| {
            if delay > 0.0 {
                executor.timer(Duration::from_secs_f64(delay)).await;
            }
            loop {
                let retry = state.update(cx, |state, cx| {
                    state.complete_mount_window_when_idle(generation, &visible_tabs, selected_tab, &mount, cx)
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
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mount: &ChatProfileTabsMount,
        cx: &mut Context<Self>,
    ) -> Option<TimeInterval> {
        if self.mount_generation != generation
            || !self.selection_is_current(visible_tabs, selected_tab, mount, cx)
        {
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

        self.initial_mount_pending = false;
        self.replace_mounted_tabs(ChatProfilePaneMountPolicy::window(visible_tabs, selected_tab), cx);
        self.synchronize_prepared_directions(visible_tabs, selected_tab, mount, cx);
        None
    }

    fn replace_mounted_tabs(&mut self, tabs: HashSet<ChatProfileTab>, cx: &mut Context<Self>) {
        if self.mounted_tabs != tabs {
            self.mounted_tabs = tabs;
            cx.notify();
        }
    }

    fn synchronize_prepared_directions(
        &mut self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mount: &ChatProfileTabsMount,
        cx: &mut Context<Self>,
    ) {
        let mounted = self.effective_mounted_tabs(visible_tabs, selected_tab);
        let directions = ChatProfilePaneMountPolicy::prepared_directions(visible_tabs, selected_tab, &mounted);
        mount.scroll_coordinator.update(cx, |coordinator, cx| {
            coordinator.set_visible_horizontal_tab_keys(scroll_keys(visible_tabs), cx);
            coordinator.set_prepared_horizontal_directions(directions, cx);
        });
    }

    fn on_appear(
        &mut self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mount: &ChatProfileTabsMount,
        cx: &mut Context<Self>,
    ) {
        self.initial_mount_pending = true;
        mount.scroll_coordinator.update(cx, |coordinator, cx| {
            coordinator.set_horizontal_paging_top_inset(ChatProfileViewportLayoutPolicy::HEADER_UNDERLAP, cx);
            coordinator.set_visible_horizontal_tab_keys(scroll_keys(visible_tabs), cx);
            coordinator.set_active_tab_key(selected_tab.scroll_key(), cx);
        });
        self.normalize_mounted_tabs(visible_tabs, selected_tab, cx);
        self.synchronize_prepared_directions(visible_tabs, selected_tab, mount, cx);
        self.schedule_mount_window(
            visible_tabs,
            selected_tab,
            ProfileHorizontalPagerPolicy::INITIAL_PAGE_PRELOAD_DELAY,
            mount,
            cx,
        );
    }

    fn on_active_tab_change(
        &mut self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mount: &ChatProfileTabsMount,
        cx: &mut Context<Self>,
    ) {
        mount.scroll_coordinator.update(cx, |coordinator, cx| {
            coordinator.set_visible_horizontal_tab_keys(scroll_keys(visible_tabs), cx);
            coordinator.set_active_tab_key(selected_tab.scroll_key(), cx);
        });
        self.normalize_mounted_tabs(visible_tabs, selected_tab, cx);
        self.synchronize_prepared_directions(visible_tabs, selected_tab, mount, cx);
        self.grow_mounted_tabs_after_continuity_frame(visible_tabs, selected_tab, mount, cx);
        let delay = if self.initial_mount_pending {
            ProfileHorizontalPagerPolicy::INITIAL_PAGE_PRELOAD_DELAY
        } else {
            ProfileHorizontalPagerPolicy::PAGE_PRELOAD_DELAY
        };
        self.schedule_mount_window(visible_tabs, selected_tab, delay, mount, cx);
    }

    fn on_visible_tabs_change(
        &mut self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mount: &ChatProfileTabsMount,
        cx: &mut Context<Self>,
    ) {
        self.normalize_mounted_tabs(visible_tabs, selected_tab, cx);
        self.synchronize_prepared_directions(visible_tabs, selected_tab, mount, cx);
        self.schedule_mount_window(
            visible_tabs,
            selected_tab,
            ProfileHorizontalPagerPolicy::INITIAL_PAGE_PRELOAD_DELAY,
            mount,
            cx,
        );
    }

    fn on_scope_key_change(
        &mut self,
        visible_tabs: &[ChatProfileTab],
        selected_tab: ChatProfileTab,
        mount: &ChatProfileTabsMount,
        cx: &mut Context<Self>,
    ) {
        self.initial_mount_pending = true;
        self.replace_mounted_tabs(HashSet::from([selected_tab]), cx);
        self.synchronize_prepared_directions(visible_tabs, selected_tab, mount, cx);
        self.schedule_mount_window(
            visible_tabs,
            selected_tab,
            ProfileHorizontalPagerPolicy::INITIAL_PAGE_PRELOAD_DELAY,
            mount,
            cx,
        );
    }

    fn on_disappear(&mut self, mount: &ChatProfileTabsMount, cx: &mut Context<Self>) {
        self.mount_generation = self.mount_generation.wrapping_add(1);
        self.initial_mount_pending = true;
        self.mount_window_task = None;
        self.grow_task = None;
        mount.scroll_coordinator.update(cx, |coordinator, cx| {
            coordinator.set_prepared_horizontal_directions(HashSet::new(), cx);
        });
    }
}

fn scroll_keys(tabs: &[ChatProfileTab]) -> Vec<i64> {
    tabs.iter().map(|tab| tab.scroll_key()).collect()
}

#[derive(IntoElement)]
pub struct ChatProfileTabs {
    id: SharedString,
    active_tab: Binding<i64>,
    chat_id: String,
    chat_type: ChatType,
    topic_id: Option<String>,
    group_by_role: bool,
    members_first: bool,
    gifts_first: bool,
    shows_members: bool,
    content_restricted: bool,
    peer_user_id: String,
    viewport_height: f32,
    bottom_inset: f32,
    scope_key: String,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    on_select_tab: ChatProfileSelectTab,
}

impl ChatProfileTabs {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<SharedString>,
        active_tab: Binding<i64>,
        chat_id: impl Into<String>,
        chat_type: ChatType,
        viewport_height: f32,
        bottom_inset: f32,
        scope_key: impl Into<String>,
        scroll_coordinator: Entity<ProfileScrollCoordinator>,
        on_select_tab: impl Fn(i64, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            active_tab,
            chat_id: chat_id.into(),
            chat_type,
            topic_id: None,
            group_by_role: false,
            members_first: false,
            gifts_first: false,
            shows_members: true,
            content_restricted: false,
            peer_user_id: String::new(),
            viewport_height,
            bottom_inset,
            scope_key: scope_key.into(),
            scroll_coordinator,
            on_select_tab: Rc::new(on_select_tab),
        }
    }

    pub fn topic_id(mut self, topic_id: Option<String>) -> Self {
        self.topic_id = topic_id;
        self
    }

    pub fn group_by_role(mut self, group_by_role: bool) -> Self {
        self.group_by_role = group_by_role;
        self
    }

    pub fn members_first(mut self, members_first: bool) -> Self {
        self.members_first = members_first;
        self
    }

    pub fn gifts_first(mut self, gifts_first: bool) -> Self {
        self.gifts_first = gifts_first;
        self
    }

    pub fn shows_members(mut self, shows_members: bool) -> Self {
        self.shows_members = shows_members;
        self
    }

    pub fn content_restricted(mut self, content_restricted: bool) -> Self {
        self.content_restricted = content_restricted;
        self
    }

    /// peer of a direct chat, empty for a community where the shared groups tab doesnt exist
    pub fn peer_user_id(mut self, peer_user_id: impl Into<String>) -> Self {
        self.peer_user_id = peer_user_id.into();
        self
    }

    fn child_id(&self, name: &str) -> ElementId {
        ElementId::Name(format!("{}-{name}", self.id).into())
    }

    fn tab_id(&self, name: &str, tab: ChatProfileTab) -> ElementId {
        ElementId::Name(format!("{}-{name}-{}", self.id, tab.id()).into())
    }

    fn mount(&self) -> ChatProfileTabsMount {
        ChatProfileTabsMount { active_tab: self.active_tab.clone(), scroll_coordinator: self.scroll_coordinator.clone() }
    }

    fn observe_lifecycle(
        &self,
        state: &Entity<ChatProfileTabsState>,
        ordered_tabs: &[ChatProfileTab],
        active_tab: i64,
        selected_tab: ChatProfileTab,
        window: &mut Window,
        cx: &mut App,
    ) {
        let on_active = (state.clone(), ordered_tabs.to_vec(), self.mount());
        on_change(self.child_id("active-tab"), active_tab, false, window, cx, move |_, _, _, cx| {
            let (state, tabs, mount) = on_active;
            state.update(cx, |state, cx| state.on_active_tab_change(&tabs, selected_tab, &mount, cx));
        });
        let on_tabs = (state.clone(), self.mount());
        on_change(self.child_id("ordered-tabs"), ordered_tabs.to_vec(), false, window, cx, move |_, tabs, _, cx| {
            let (state, mount) = on_tabs;
            state.update(cx, |state, cx| state.on_visible_tabs_change(tabs, selected_tab, &mount, cx));
        });
        let on_scope = (state.clone(), ordered_tabs.to_vec(), self.mount());
        on_change(self.child_id("scope-key"), self.scope_key.clone(), false, window, cx, move |_, _, _, cx| {
            let (state, tabs, mount) = on_scope;
            state.update(cx, |state, cx| state.on_scope_key_change(&tabs, selected_tab, &mount, cx));
        });
    }

    /// inputs of tab_scroll that reach the page as plain values
    /// cached frame drops only when one moved
    fn page_key(&self, tab: ChatProfileTab, pager_height: f32, cx: &mut App) -> u64 {
        let mut hasher = DefaultHasher::new();
        tab.hash(&mut hasher);
        pager_height.to_bits().hash(&mut hasher);
        self.bottom_inset.to_bits().hash(&mut hasher);
        self.chat_id.hash(&mut hasher);
        self.topic_id.hash(&mut hasher);
        self.peer_user_id.hash(&mut hasher);
        self.scope_key.hash(&mut hasher);
        self.group_by_role.hash(&mut hasher);
        self.content_restricted.hash(&mut hasher);
        hold_context_menu_locks_scroll(cx).hash(&mut hasher);
        self.scroll_coordinator.read(cx).page_scroll_view(tab.scroll_key()).pan_enabled().hash(&mut hasher);
        let background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        for channel in [background.h, background.s, background.l, background.a] {
            channel.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }

    fn page_bottom_spacer_height(&self) -> f32 {
        self.bottom_inset.max(80.0) + 24.0
    }

    fn tab_scroll(&self, tab: ChatProfileTab, pager_height: f32, cx: &mut App) -> AnyElement {
        let body = self.tab_scroll_body(tab, pager_height, cx);
        let shows_scrubber = !self.content_restricted && matches!(tab, ChatProfileTab::Media | ChatProfileTab::Stories);
        let scrubber = shows_scrubber.then(|| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right_0()
                .w(px(ProfileGridDateScrubberLayer::LAYER_WIDTH))
                .child(self.date_scrubber(tab, cx))
        });
        div()
            .relative()
            .size_full()
            .children((tab == ChatProfileTab::Fav && !self.content_restricted).then(|| {
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .top(px(ChatProfileViewportLayoutPolicy::HEADER_UNDERLAP))
                    .overflow_hidden()
                    .child(ChatDecorativeBackdrop::new().ignores_safe_area(false).id(self.tab_id("backdrop", tab)))
            }))
            .child(body)
            .children(scrubber)
            .into_any_element()
    }

    fn date_scrubber(&self, tab: ChatProfileTab, cx: &mut App) -> AnyElement {
        let top_inset = ChatProfileViewportLayoutPolicy::page_content_top_inset();
        let bottom_inset = self.bottom_inset.max(80.0) + 24.0;
        if tab == ChatProfileTab::Media {
            return ChatProfileMediaDateScrubberLayer::new(
                self.tab_id("media-date-scrubber", tab),
                self.chat_id.clone(),
                self.topic_id.clone(),
                tab.scroll_key(),
                self.scroll_coordinator.clone(),
                top_inset,
                bottom_inset,
            )
            .into_any_element();
        }
        let timestamps: Vec<TimeInterval> = StoryServicesStore::shared(cx)
            .update(cx, |service, cx| service.profile_stories(&self.peer_user_id, StoryProfileScope::Profile, cx))
            .iter()
            .map(|story| story.created_at_ms as TimeInterval / 1_000.0)
            .collect();
        ProfileGridDateScrubberLayer::new(
            self.tab_id("grid-date-scrubber", tab),
            timestamps,
            StoriesGridMetrics::COLUMNS,
            StoriesGridMetrics::CELL_ASPECT,
            StoriesGridMetrics::SPACING,
            StoriesGridMetrics::HORIZONTAL_PADDING,
            tab.scroll_key(),
            self.scroll_coordinator.clone(),
            top_inset,
            bottom_inset,
        )
        .columns_for_width(StoriesGridMetrics::columns)
        .into_any_element()
    }

    fn tab_scroll_body(&self, tab: ChatProfileTab, pager_height: f32, cx: &mut App) -> AnyElement {
        let top_inset = ChatProfileViewportLayoutPolicy::page_content_top_inset();
        let spacer_height = self.page_bottom_spacer_height();
        let tab_key = tab.scroll_key();
        let page_min_height = (pager_height - top_inset - spacer_height).max(152.0);
        let content = div()
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .child(div().flex_none().h(px(top_inset)))
            .child(div().flex().flex_col().w_full().min_h(px(page_min_height)).child(self.tab_body(tab, cx)))
            .child(div().flex_none().h(px(spacer_height)))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size(px(1.0))
                    .child(ProfileInnerScrollIntrospector::new_key(tab_key, self.scroll_coordinator.clone())),
            );
        let (page_scroll, return_scroller) = {
            let coordinator = self.scroll_coordinator.read(cx);
            (coordinator.page_scroll_view(tab_key), coordinator.return_scroller())
        };
        let hold_locks_scroll = hold_context_menu_locks_scroll(cx);
        let page_handle = page_scroll.handle();
        let scroller = scroll_view(self.tab_id("scroll", tab))
            .shows_indicators(false)
            .scroll_handle(page_handle.clone())
            .restrict_scroll_to_axis(true)
            .scroll_disabled(!page_scroll.pan_enabled() || hold_locks_scroll)
            .child(
                env_scope::<SharedElementReturnScrollerKey>(Some(SharedElementReturnScrollerRef::new(
                    &return_scroller,
                    tab_key,
                    self.bottom_inset,
                )))
                .child(content),
            );
        env_scope::<ChatProfileUsesParentScrollKey>(true)
            .child(env_scope::<ChatProfilePageScrollKey>(Some(page_handle)).child(scroller))
            .into_any_element()
    }

    fn tab_body(&self, tab: ChatProfileTab, cx: &mut App) -> AnyElement {
        if self.content_restricted {
            let (icon, text_key) = Self::restricted_empty_state(tab);
            return ChatProfileEmptyState::new(icon, tr(text_key)).into_any_element();
        }
        self.unrestricted_tab_body(tab, cx)
    }

    fn restricted_empty_state(tab: ChatProfileTab) -> (&'static str, &'static str) {
        match tab {
            ChatProfileTab::Media => ("photo.on.rectangle", "chat_profile_empty_media"),
            ChatProfileTab::Fav => ("bookmark.fill", "chat_profile_empty_fav"),
            ChatProfileTab::File => ("doc", "chat_profile_empty_files"),
            ChatProfileTab::Link => ("link", "chat_profile_empty_links"),
            ChatProfileTab::Music => ("music.note", "chat_profile_empty_music"),
            ChatProfileTab::Voice => ("waveform", "chat_profile_empty_voice"),
            ChatProfileTab::Stories => ("play.rectangle", "profile_stories_empty"),
            ChatProfileTab::Groups => ("person.2", "chat_profile_empty_groups"),
            ChatProfileTab::Members => ("person.2", "chat_profile_empty_members"),
            ChatProfileTab::Gif => ("photo", "chat_profile_empty_gif"),
            ChatProfileTab::Gifts => ("gift", "gifts_empty"),
        }
    }

    fn unrestricted_tab_body(&self, tab: ChatProfileTab, cx: &mut App) -> AnyElement {
        let chat_id = self.chat_id.clone();
        let topic_id = self.topic_id.clone();
        match tab {
            ChatProfileTab::Media => ChatProfileMediaTab::new(chat_id).topic_id(topic_id).into_any_element(),
            ChatProfileTab::Fav => ChatProfileFavTab::new(chat_id).into_any_element(),
            ChatProfileTab::File => ChatProfileFileTab::new(chat_id).topic_id(topic_id).into_any_element(),
            ChatProfileTab::Link => ChatProfileLinkTab::new(chat_id).topic_id(topic_id).into_any_element(),
            ChatProfileTab::Music => ChatProfileMusicTab::new(chat_id).into_any_element(),
            ChatProfileTab::Voice => ChatProfileVoiceTab::new(chat_id).into_any_element(),
            ChatProfileTab::Stories => {
                let is_owner = {
                    let auth = AuthServicesStore::shared(cx).read(cx);
                    auth.is_current_user(&self.peer_user_id) || auth.viewer_acts_as_author()
                };
                StoriesGridPage::new(self.peer_user_id.clone(), is_owner).into_any_element()
            }
            ChatProfileTab::Groups => ChatProfileGroupsTab::new(self.peer_user_id.clone()).into_any_element(),
            ChatProfileTab::Members => {
                ChatProfileMembersTab::new(chat_id).topic_id(topic_id).group_by_role(self.group_by_role).into_any_element()
            }
            ChatProfileTab::Gif => ChatProfileGifTab::new(chat_id).into_any_element(),
            ChatProfileTab::Gifts => UserGiftsTab::new(self.peer_user_id.clone()).into_any_element(),
        }
    }
}

impl RenderOnce for ChatProfileTabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        gpui::hear_entity_through_events_only(self.scroll_coordinator.entity_id());
        let ordered_tabs =
            ChatProfileTab::ordered(self.chat_type, self.members_first, self.gifts_first, self.shows_members);
        let active_tab = self.active_tab.get(cx);
        let selected_index = active_tab.min(ordered_tabs.len() as i64 - 1).max(0);
        let Some(selected_tab) = ordered_tabs.get(selected_index as usize).copied() else {
            return Empty.into_any_element();
        };
        let background = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let floating_top_inset = env::<ListShellFloatingTopInsetKey>(cx);
        let container_height =
            ChatProfileViewportLayoutPolicy::pinned_container_height(self.viewport_height, floating_top_inset);
        let pager_height = ChatProfileViewportLayoutPolicy::pager_height(container_height);
        let header_underlap = ChatProfileViewportLayoutPolicy::HEADER_UNDERLAP;

        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| ChatProfileTabsState::new());
        let tabs_bottom: ChatProfilePinnedTabsBottom = window
            .use_keyed_state(self.child_id("tabs-bottom"), cx, |_, _| Rc::new(Cell::new(None)))
            .read(cx)
            .clone();
        state.update(cx, |state, _| {
            if state.current_tabs != ordered_tabs {
                state.current_tabs = ordered_tabs.clone();
            }
        });
        self.observe_lifecycle(&state, &ordered_tabs, active_tab, selected_tab, window, cx);

        let (rendered_tabs, page_width) = {
            let current = state.read(cx);
            (current.effective_mounted_tabs(&ordered_tabs, selected_tab), current.page_width)
        };

        let on_appear = (state.clone(), ordered_tabs.clone(), self.mount());
        let on_disappear = (state.clone(), self.mount());
        let appear_id = self.child_id("appear");
        let disappear_id = self.child_id("disappear");
        let tab_bar = ChatProfileTabBar {
            id: self.child_id("tab-bar"),
            visible_tabs: ordered_tabs.clone(),
            selected_index,
            scroll_coordinator: self.scroll_coordinator.clone(),
            on_select_tab: self.on_select_tab.clone(),
        };
        let this = Rc::new(self);
        let page: ChatProfilePage = {
            let this = this.clone();
            Rc::new(move |tab: ChatProfileTab, _: &mut Window, cx: &mut App| this.tab_scroll(tab, pager_height, cx))
        };
        let page_key: ChatProfilePageKey = {
            let this = this.clone();
            Rc::new(move |tab: ChatProfileTab, cx: &mut App| this.page_key(tab, pager_height, cx))
        };

        div()
            .relative()
            .w_full()
            .h(px(container_height))
            .bg(background)
            .child({
                let tabs_bottom = tabs_bottom.clone();
                canvas(
                    move |bounds, _, _| {
                        tabs_bottom.set(Some(f32::from(bounds.top()) + ProfileScrollMetrics::TABS_BAR_HEIGHT));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size(px(1.0))
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(-header_underlap))
                    .h(px(pager_height))
                    .child(env_scope::<ChatProfilePinnedTabsBottomKey>(Some(tabs_bottom)).child(ChatProfilePagesPager {
                        id: this.child_id("pages-pager"),
                        visible_tabs: ordered_tabs,
                        selected_tab,
                        rendered_tabs,
                        scroll_coordinator: this.scroll_coordinator.clone(),
                        state,
                        page_width,
                        height: pager_height,
                        page,
                        page_key,
                    })),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(-header_underlap))
                    .h(px(ProfilePinnedChromePlate::height()))
                    .child(ProfilePinnedChromePlate::new().pinned_min_y(-floating_top_inset.max(0.0))),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(-ChatProfileViewportLayoutPolicy::PINNED_GAP_FILL))
                    .h(px(ProfileScrollMetrics::TABS_BAR_HEIGHT + ChatProfileViewportLayoutPolicy::PINNED_GAP_FILL))
                    .bg(background),
            )
            .child(div().absolute().left_0().right_0().top_0().child(tab_bar))
            .children(
                ProfileUnderlapTouchRouter::is_needed()
                    .then(|| ProfileUnderlapTouchRouter::new(this.scroll_coordinator.clone(), header_underlap)),
            )
            .on_appear(appear_id, move |_, cx| {
                let (state, tabs, mount) = on_appear;
                state.update(cx, |state, cx| state.on_appear(&tabs, selected_tab, &mount, cx));
            })
            .on_disappear(disappear_id, move |_, cx| {
                let (state, mount) = on_disappear;
                state.update(cx, |state, cx| state.on_disappear(&mount, cx));
            })
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct ChatProfileTabBar {
    id: ElementId,
    visible_tabs: Vec<ChatProfileTab>,
    selected_index: i64,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    on_select_tab: ChatProfileSelectTab,
}

impl RenderOnce for ChatProfileTabBar {
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
            .map(|(index, tab)| TabHeaderConfig::new(index as i64, tr(tab.title_key())))
            .collect();
        let selected_index = self.selected_index;
        let binding_select = self.on_select_tab.clone();
        let tap_select = self.on_select_tab;
        let progress_coordinator = self.scroll_coordinator.clone();
        let base_coordinator = self.scroll_coordinator;
        let header = AnimatedTabsHeader::new(
            self.id,
            tabs,
            Binding::new(move |_| selected_index, move |index, window, cx| binding_select(index, window, cx)),
            Rc::new(move |cx: &App| progress_coordinator.read(cx).tabs_live_state.read(cx).drag_progress()),
            0.0,
            Rc::new(move |index: i64, window: &mut Window, cx: &mut App| tap_select(index, window, cx)),
        )
        .drag_base_index(Rc::new(move |cx: &App| base_coordinator.read(cx).tabs_live_state.read(cx).drag_base_index()))
        .shows_pill_background(false)
        .arrows_step_selection(true)
        .fade_surface(capsule_fill);

        div()
            .flex()
            .flex_row()
            .items_center()
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
                    .child(
                        env_scope::<AnimatedTabsPagerAxisOwnersKey>(Some(axis_owners))
                            .child(env_scope::<LayoutDirectionKey>(LayoutDirection::LeftToRight).child(header)),
                    )
                    .child(centred_stroke(capsule_stroke, 0.5, Shape::Capsule)),
            )
    }
}

#[derive(IntoElement)]
struct ChatProfilePagerInteractionShape {
    top: f32,
    bottom: f32,
}

impl RenderOnce for ChatProfilePagerInteractionShape {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(px(self.top))
            .h(px((self.bottom - self.top).max(0.0)))
            .block_mouse_except_scroll()
    }
}

#[derive(IntoElement)]
struct ChatProfilePagesPager {
    id: ElementId,
    visible_tabs: Vec<ChatProfileTab>,
    selected_tab: ChatProfileTab,
    rendered_tabs: HashSet<ChatProfileTab>,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    state: Entity<ChatProfileTabsState>,
    page_width: f32,
    height: f32,
    page: ChatProfilePage,
    page_key: ChatProfilePageKey,
}

impl RenderOnce for ChatProfilePagesPager {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        gpui::hear_entity_through_events_only(self.scroll_coordinator.entity_id());
        let page_scrolled = self
            .scroll_coordinator
            .read(cx)
            .page_scroll_view(self.selected_tab.scroll_key())
            .content_offset_y()
            > 0.0;
        let width = self.page_width.max(1.0);
        let selected_index = self.visible_tabs.iter().position(|tab| *tab == self.selected_tab).unwrap_or(0) as i64;
        let page_count = self.visible_tabs.len() as i64;
        let state = self.state;
        let height = self.height;
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(measure(self.id, move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                state.update(cx, |state, cx| {
                    if state.page_width != width {
                        state.page_width = width;
                        cx.notify();
                    }
                });
            }))
            .child(ChatProfileStablePages {
                visible_tabs: self.visible_tabs,
                selected_tab: self.selected_tab,
                rendered_tabs: self.rendered_tabs,
                width,
                height,
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
            .child(ChatProfilePagerInteractionShape {
                top: if page_scrolled { 0.0 } else { ChatProfileViewportLayoutPolicy::HEADER_UNDERLAP.min(height) },
                bottom: ChatProfileViewportLayoutPolicy::page_content_top_inset().min(height),
            })
    }
}

#[derive(IntoElement)]
struct ChatProfileStablePages {
    visible_tabs: Vec<ChatProfileTab>,
    selected_tab: ChatProfileTab,
    rendered_tabs: HashSet<ChatProfileTab>,
    width: f32,
    height: f32,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    page: ChatProfilePage,
    page_key: ChatProfilePageKey,
}

impl RenderOnce for ChatProfileStablePages {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        gpui::hear_entity_through_events_only(self.scroll_coordinator.entity_id());
        let measured = self.width > 1.0;
        let visual_index = self.scroll_coordinator.read(cx).horizontal_pager_visual_index();
        let pages: Vec<AnyElement> = self
            .visible_tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let frame = div().flex_none().w(px(self.width)).h(px(self.height));
                let off_screen = *tab != self.selected_tab
                    && visual_index.is_some_and(|visual| (index as f32 - visual).abs() >= 0.999);
                let frame = if off_screen { frame.invisible() } else { frame };
                if measured && self.rendered_tabs.contains(tab) {
                    let tab = *tab;
                    let host_id = ElementId::NamedInteger(
                        SharedString::new_static("chat-profile-tab-page-host"),
                        tab.scroll_key() as u64,
                    );
                    let host = window
                        .use_keyed_state(host_id, cx, move |_, cx| ChatProfileTabPageHostHolder {
                            host: cx.new(|_| ChatProfileTabPageHost::new(tab)),
                        })
                        .read(cx)
                        .host
                        .clone();
                    let key = (self.page_key)(tab, cx);
                    let page = self.page.clone();
                    let coordinator_id = self.scroll_coordinator.entity_id();
                    host.update(cx, |host, _| host.set_page(page, coordinator_id));
                    let view = env_scope::<ChatProfileTabPageKey>(key)
                        .child(AnyView::from(host).cached(StyleRefinement::default().size_full()));
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
        host.horizontal_pager_container("chat-profile-pages-pager-strip")
            .items_start()
            .child(ProfileHorizontalMotionElement::new(motion, strip))
    }
}
