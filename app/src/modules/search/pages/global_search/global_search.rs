use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyView, App, AsyncWindowContext, Entity, FontWeight, KeyDownEvent, ListAlignment, ListOffset, ListScrollEvent, ListState,
    Pixels, SharedString, StyleRefinement, Subscription, Task, Window,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsMetrics, TabHeaderConfig};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuLocksScrollKey;
use rise_ui::icon_ui::IconUi;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::UnitPoint;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::animated_tabs_ui::animated_tabs_header::AnimatedTabsHeader;
use crate::core::ui::animated_tabs_ui::animated_tabs_pager::{AnimatedTabsPager, AnimatedTabsPagerLiveState};
use crate::core::ui::animated_tabs_ui::keyboard_docked_bottom_strip::KeyboardDockedBottomStripModifiers;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::sheets::report_sheet::{ReportSheetStore, ReportSubject};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellUi, list_shell_floating_top_inset};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat_search::chat_search_section::ChatSearchSectionHeader;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post::post_skeleton::{
    PostCardSkeletonVariant, PostFeedSkeleton, PostGridSkeleton,
};
use crate::modules::post::components::post_collection_ui::{
    PostGridTextUi, PostHoldContextMenu, PostLayoutToggleUi, PostPageStateUi, PostPaginationTriggerModifiers,
};
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostFeedKind, RisePostPagePresentation, RisePostPageScope, RisePostPageStatus, RisePostSearchKind,
    RisePostViewSurface,
};
use crate::modules::post::pages::posts::post_detail::list_rows_splice_hooks::splice_list_rows;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, GetPostFeedResponse};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::search::components::global_search_result_rows::{
    GlobalSearchChatResultRow, GlobalSearchProfileResultRow,
};
use crate::modules::search::engine::rise_search_engine_models::{
    RiseGlobalSearchCategory, RiseGlobalSearchPagePresentation, RiseGlobalSearchRow, RiseGlobalSearchScope,
};
use crate::modules::search::pages::global_search::global_search_aside_hooks::global_search_aside_hooks;
use crate::modules::search::shared::global_search_route_resolver::GlobalSearchRouteResolver;
use crate::modules::search::stores::search_actions::search_types::{SearchEntityRow, SearchResultsTab};
use crate::modules::search::stores::search_interactions::search_interactions::SearchInteractionsStore;
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::ProfileGridPreviewSourcePolicy;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

const KEEP_FIELD_FOCUS_STEP: Duration = Duration::from_millis(50);
const KEEP_FIELD_FOCUS_CHECKS: usize = 12;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(350);
const GRID_MIN_COLUMNS: usize = 3;
const GRID_MAX_COLUMNS: usize = 5;
const GRID_TILE_MIN_WIDTH: f32 = 240.0;
const GRID_SPACING: f32 = 2.0;
const GRID_TOP_GAP: f32 = 8.0;
const GRID_LOAD_MORE_THRESHOLD: i64 = 9;
const ENTITY_LOAD_MORE_THRESHOLD: i64 = 8;

/// publication grid columns for the center width, three at phone width up to five
/// fills_height is for a pager page
fn search_list_column(width: Pixels, fills_height: bool, content: AnyElement) -> AnyElement {
    let column = div().relative().w_full().max_w(width).child(content);
    let column = if fills_height { column.h_full() } else { column };
    let row = div().w_full().flex().flex_row().justify_center().child(column);
    let row = if fills_height { row.h_full() } else { row };
    row.into_any_element()
}

fn grid_column_count(width: f32) -> usize {
    if width <= 0.0 {
        return GRID_MIN_COLUMNS;
    }
    let fitting = ((width + GRID_SPACING) / (GRID_TILE_MIN_WIDTH + GRID_SPACING)).floor() as usize;
    fitting.clamp(GRID_MIN_COLUMNS, GRID_MAX_COLUMNS)
}
const STATE_MIN_HEIGHT: f32 = 320.0;
const BOTTOM_SPACER: f32 = 80.0;
const BOTTOM_ROW_ID: &str = "global-search-bottom-spacer";
const LEAD_ROW_ID: &str = "global-search-lead";
const CHROME_REFREEZE_DELAY: Duration = Duration::from_secs(1);
const HASHTAG_TABS: [SearchResultsTab; 2] = [SearchResultsTab::Publications, SearchResultsTab::Threads];

/// top margin settles over a few frames at launch, list is re placed at top until first drag
/// frozen at the first drag
struct SearchChromeSettle {
    generation: u64,
    touched: bool,
    frozen_inset: Option<f32>,
    width: f32,
    live_inset: f32,
    observed_inset: Option<f32>,
    refreeze: Option<Task<()>>,
}

impl SearchChromeSettle {
    fn new() -> Self {
        Self {
            generation: 0,
            touched: false,
            frozen_inset: None,
            width: 0.0,
            live_inset: 0.0,
            observed_inset: None,
            refreeze: None,
        }
    }

    fn margin(&self) -> f32 {
        self.frozen_inset.unwrap_or(self.live_inset).max(0.0)
    }

    /// first drag freezes the margin
    fn did_drag(&mut self) {
        if self.touched {
            return;
        }
        self.touched = true;
        self.frozen_inset = Some(self.live_inset);
    }

    /// true when the list must be rebuilt at top, the first reading only records the margin
    fn inset_did_change(&mut self, inset: f32, is_at_top: bool) -> bool {
        self.live_inset = inset;
        let new = inset.round();
        let Some(old) = self.observed_inset.replace(new) else {
            return false;
        };
        let remade = old != new && !self.touched && is_at_top;
        if remade {
            self.generation = self.generation.wrapping_add(1);
        }
        remade
    }

    /// a new width thaws a touched list, the first width doesnt count
    fn width_did_change<V: 'static>(
        view: &mut V,
        next: f32,
        settle: fn(&mut V) -> &mut SearchChromeSettle,
        cx: &mut Context<V>,
    ) {
        let next = next.round();
        let chrome = settle(view);
        if chrome.width == 0.0 || next == chrome.width {
            chrome.width = next;
            return;
        }
        chrome.width = next;
        if chrome.touched {
            chrome.frozen_inset = None;
        }
        if !chrome.touched {
            return;
        }
        chrome.refreeze = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHROME_REFREEZE_DELAY).await;
            let _ = this.update(cx, |view, cx| {
                let chrome = settle(view);
                if chrome.touched && chrome.frozen_inset.is_none() {
                    chrome.frozen_inset = Some(chrome.live_inset);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }
}

trait SearchResultsTabPage {
    fn title_key(&self) -> &'static str;
    fn global_category(&self) -> Option<RiseGlobalSearchCategory>;
    fn post_kind(&self) -> Option<RisePostSearchKind>;
    fn is_content(&self) -> bool;
}

impl SearchResultsTabPage for SearchResultsTab {
    fn title_key(&self) -> &'static str {
        match self {
            SearchResultsTab::Publications => "global_search_tab_publications",
            SearchResultsTab::Threads => "global_search_tab_threads",
            SearchResultsTab::Profiles => "global_search_tab_profiles",
            SearchResultsTab::Channels => "global_search_tab_channels",
            SearchResultsTab::Groups => "global_search_tab_groups",
        }
    }

    fn global_category(&self) -> Option<RiseGlobalSearchCategory> {
        match self {
            SearchResultsTab::Profiles => Some(RiseGlobalSearchCategory::Profiles),
            SearchResultsTab::Channels => Some(RiseGlobalSearchCategory::Channels),
            SearchResultsTab::Groups => Some(RiseGlobalSearchCategory::Groups),
            SearchResultsTab::Publications | SearchResultsTab::Threads => None,
        }
    }

    fn post_kind(&self) -> Option<RisePostSearchKind> {
        match self {
            SearchResultsTab::Publications => Some(RisePostSearchKind::Publications),
            SearchResultsTab::Threads => Some(RisePostSearchKind::Threads),
            SearchResultsTab::Profiles | SearchResultsTab::Channels | SearchResultsTab::Groups => None,
        }
    }

    fn is_content(&self) -> bool {
        self.post_kind().is_some()
    }
}

/// virtual list needs a definite height or gpui draws a black page, dont ask
/// list fills the page, state rides on top
fn list_with_state(rows: impl IntoElement, state: PostPageStateUi) -> AnyElement {
    div()
        .relative()
        .size_full()
        .child(rows)
        .child(div().absolute().top_0().left_0().right_0().child(state.content(|_, _| div())))
        .into_any_element()
}

/// empty page that still has an error text shows the reason instead of plain empty
fn empty_or_refusal_text(
    status: RisePostPageStatus,
    is_empty: bool,
    error_message: &Option<String>,
    empty_text: String,
) -> String {
    match error_message {
        Some(text) if is_empty && status == RisePostPageStatus::Loaded => text.clone(),
        _ => empty_text,
    }
}

/// redraws the page only when the scroll lock flips, not on every row press
fn observe_menu_lock<V: 'static>(menu: &Entity<HoldContextMenuPresenter>, cx: &mut Context<V>) -> Subscription {
    let locked = Cell::new(menu.read(cx).active_request().is_some());
    cx.observe(menu, move |_, menu, cx| {
        let now = menu.read(cx).active_request().is_some();
        if locked.replace(now) != now {
            cx.notify();
        }
    })
}

fn push_route(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

/// desktop search, a pick previews in the right panel and results stay in center
/// next pick replaces it, narrow window pushes a page
fn preview_route(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    let previews = nav_store.as_ref().is_some_and(|store| store.read(cx).aside_takes_details());
    if previews {
        if let Some(hooks) = global_search_aside_hooks() {
            (hooks.replace)(route, cx);
        }
        return;
    }
    push_route(nav_store, route, cx);
}

const SEARCH_FOCUS_KEY: &str = "f";

/// mounted page of the pager, kept across frames so its list keeps its offset
enum GlobalSearchResultsPage {
    Post(Entity<GlobalSearchPostPage>),
    Entity(Entity<GlobalSearchEntityPage>),
}

pub struct GlobalSearch {
    initial_query: String,
    query: String,
    selected_tab: SearchResultsTab,
    publications_use_grid: bool,
    threads_use_grid: bool,
    mounted_tabs: HashSet<SearchResultsTab>,
    search_focus_requests: u64,
    seen_field_focus_request: u64,
    is_on_top: bool,
    query_edited_by_field: bool,
    mount_window_generation: u64,
    scroll_anchors: Rc<RefCell<SearchScrollAnchorStore>>,
    pager_live_state: Entity<AnimatedTabsPagerLiveState>,
    observed_mount_anchor: i64,
    settled_drag_progress: f32,
    page_width: f32,
    hold_context_menu: Entity<HoldContextMenuPresenter>,
    pages: HashMap<SearchResultsTab, GlobalSearchResultsPage>,
    default_feed_page: Option<Entity<GlobalSearchDefaultFeedPage>>,
    _subscriptions: Vec<Subscription>,
}

impl GlobalSearch {
    pub fn new(initial_query: impl Into<String>, initial_kind: RisePostSearchKind, cx: &mut Context<Self>) -> Self {
        let initial_query = initial_query.into();
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let pager_live_state = cx.new(|_| AnimatedTabsPagerLiveState::new());
        let search_service = SearchServicesStore::shared(cx);
        let seen_field_focus_request = search_service.read(cx).field_focus_request();
        let subscriptions = vec![
            observe_menu_lock(&hold_context_menu, cx),
            cx.observe(&search_service, |view: &mut Self, service, cx| {
                let request = service.read(cx).field_focus_request();
                if request == view.seen_field_focus_request {
                    return;
                }
                view.seen_field_focus_request = request;
                if view.is_on_top && view.initial_query.is_empty() {
                    view.search_focus_requests += 1;
                    cx.notify();
                }
            }),
            cx.observe(&pager_live_state, |view, live, cx| {
                let anchor = live.read(cx).resolved_mount_anchor(view.active_index());
                if anchor != view.observed_mount_anchor {
                    view.observed_mount_anchor = anchor;
                    cx.notify();
                }
            }),
        ];
        let selected_tab = if initial_kind == RisePostSearchKind::Threads {
            SearchResultsTab::Threads
        } else {
            SearchResultsTab::Publications
        };
        let mut view = Self {
            query: initial_query.clone(),
            initial_query,
            selected_tab,
            publications_use_grid: true,
            threads_use_grid: false,
            mounted_tabs: HashSet::from(HASHTAG_TABS),
            search_focus_requests: 0,
            seen_field_focus_request,
            is_on_top: false,
            query_edited_by_field: false,
            mount_window_generation: 0,
            scroll_anchors: Rc::new(RefCell::new(SearchScrollAnchorStore::default())),
            pager_live_state,
            observed_mount_anchor: 0,
            settled_drag_progress: 0.0,
            page_width: 0.0,
            hold_context_menu,
            pages: HashMap::new(),
            default_feed_page: None,
            _subscriptions: subscriptions,
        };
        view.observed_mount_anchor = view.active_index();
        view
    }

    fn trimmed_query(&self) -> String {
        trimming_whitespaces_and_newlines(&self.query).to_owned()
    }

    fn shows_result_tabs(&self) -> bool {
        !self.trimmed_query().is_empty()
    }

    fn visible_tabs(&self) -> &'static [SearchResultsTab] {
        if self.trimmed_query().starts_with('#') {
            return &HASHTAG_TABS;
        }
        &SearchResultsTab::ALL_CASES
    }

    fn active_index(&self) -> i64 {
        self.visible_tabs().iter().position(|tab| *tab == self.selected_tab).unwrap_or(0) as i64
    }

    fn tab_at(&self, index: i64) -> Option<SearchResultsTab> {
        usize::try_from(index).ok().and_then(|index| self.visible_tabs().get(index).copied())
    }

    fn screen_did_appear(&mut self, cx: &mut Context<Self>) {
        self.is_on_top = true;
        if self.query.is_empty() && !self.initial_query.is_empty() {
            self.query = self.initial_query.clone();
        }
        self.normalize_selection();
        self.mounted_tabs = self.mount_window(self.selected_tab);
        if self.trimmed_query().is_empty() {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.activate_feed(RisePostFeedKind::All, cx));
        } else {
            self.activate_selected_search(cx);
        }
        cx.notify();
    }

    fn trimmed_query_did_change(&mut self, value: &str, cx: &mut Context<Self>) {
        self.scroll_anchors.borrow_mut().tabs.retain(|_, anchor| anchor.query == value);
        self.normalize_selection();
        self.prepare_mount_window(self.selected_tab, cx);
        self.schedule_mount_window_trim(self.selected_tab, 0.0, cx);
        if !value.is_empty() {
            return;
        }
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.activate_feed(RisePostFeedKind::All, cx));
    }

    fn search_pager(&mut self, this: &Entity<Self>, cx: &mut Context<Self>) -> AnyElement {
        let entity_id = cx.entity_id();
        let tabs = self.visible_tabs();
        let list_width = rise_ui::theme(cx).shell.list_max_width;
        let mut pages: Vec<AnyElement> = Vec::with_capacity(tabs.len());
        for tab in tabs.iter().copied() {
            let content = if self.mounted_tabs.contains(&tab) { self.search_page(tab, cx) } else { div().into_any_element() };
            let content = if tab.post_kind().is_some() { content } else { search_list_column(list_width, true, content) };
            pages.push(div().size_full().child(content).into_any_element());
        }

        let tap_state = self.pager_live_state.clone();
        AnimatedTabsPager::new(
            ("global-search-pager", entity_id),
            tabs.len() as i64,
            Self::active_tab_binding(this),
            Binding::entity(
                this.clone(),
                |view: &GlobalSearch| view.settled_drag_progress,
                |view, value, _| view.settled_drag_progress = value,
            ),
            Binding::entity(
                this.clone(),
                |view: &GlobalSearch| view.page_width,
                |view, value, _| view.page_width = value,
            ),
        )
        .live_drag_state(Some(self.pager_live_state.clone()))
        .yields_first_tab_back_swipe_to_navigation(true)
        .drag_locked(self.hold_context_menu.read(cx).active_request().is_some())
        .on_siblings_tap_suppressed(Some(Rc::new(move |suppressed: bool, _window: &mut Window, cx: &mut App| {
            Self::update_header_tap_suppression(&tap_state, suppressed, cx);
        })))
        .children(pages)
        .into_any_element()
    }

    fn search_page(&mut self, tab: SearchResultsTab, cx: &mut Context<Self>) -> AnyElement {
        let query = self.trimmed_query();
        let scroll_anchor = self.scroll_anchor_binding(tab);
        if let Some(kind) = tab.post_kind() {
            let uses_grid =
                if tab == SearchResultsTab::Publications { self.publications_use_grid } else { self.threads_use_grid };
            let on_show_all_people = self.show_all_people_action(cx);
            let page = match self.pages.get(&tab) {
                Some(GlobalSearchResultsPage::Post(page)) => page.clone(),
                _ => {
                    let created = cx.new(|cx| {
                        GlobalSearchPostPage::new(
                            query.clone(),
                            kind,
                            uses_grid,
                            scroll_anchor.clone(),
                            on_show_all_people.clone(),
                            cx,
                        )
                    });
                    self.pages.insert(tab, GlobalSearchResultsPage::Post(created.clone()));
                    created
                }
            };
            let stale = page.update(cx, |page, _| {
                let stale = page.query != query
                    || page.uses_grid != uses_grid
                    || page.on_show_all_people.is_some() != on_show_all_people.is_some();
                page.query = query;
                page.uses_grid = uses_grid;
                page.scroll_anchor = scroll_anchor;
                page.on_show_all_people = on_show_all_people;
                stale
            });
            return Self::page_view(AnyView::from(page), stale);
        }
        if let Some(category) = tab.global_category() {
            let page = match self.pages.get(&tab) {
                Some(GlobalSearchResultsPage::Entity(page)) => page.clone(),
                _ => {
                    let created = cx.new(|cx| GlobalSearchEntityPage::new(query.clone(), category, scroll_anchor.clone(), cx));
                    self.pages.insert(tab, GlobalSearchResultsPage::Entity(created.clone()));
                    created
                }
            };
            let stale = page.update(cx, |page, _| {
                let stale = page.query != query;
                page.query = query;
                page.scroll_anchor = scroll_anchor;
                stale
            });
            return Self::page_view(AnyView::from(page), stale);
        }
        div().into_any_element()
    }

    /// draws the cached page alone so a wheel tick on results doesnt redraw every mounted page
    /// that was 14-16 ms per tick
    fn page_view(page: AnyView, stale: bool) -> AnyElement {
        if stale {
            page.into_any_element()
        } else {
            page.cached(StyleRefinement::default().size_full()).into_any_element()
        }
    }

    fn show_all_people_action(&self, cx: &mut Context<Self>) -> Option<Rc<dyn Fn(&mut Window, &mut App)>> {
        let index = self.visible_tabs().iter().position(|tab| *tab == SearchResultsTab::Profiles)? as i64;
        let view = cx.weak_entity();
        Some(Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = view.update(cx, |view, cx| view.select_tab(index, cx));
        }))
    }

    fn active_tab_binding(this: &Entity<Self>) -> Binding<i64> {
        Binding::entity(
            this.clone(),
            |view: &GlobalSearch| view.active_index(),
            |view, index, _| {
                if let Some(tab) = view.tab_at(index) {
                    view.selected_tab = tab;
                }
            },
        )
    }

    fn selected_layout_binding(&self, this: &Entity<Self>) -> Binding<bool> {
        match self.selected_tab {
            SearchResultsTab::Threads => Binding::entity(
                this.clone(),
                |view: &GlobalSearch| view.threads_use_grid,
                |view, value, _| view.threads_use_grid = value,
            ),
            _ => Binding::entity(
                this.clone(),
                |view: &GlobalSearch| view.publications_use_grid,
                |view, value, _| view.publications_use_grid = value,
            ),
        }
    }

    fn select_tab(&mut self, index: i64, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_at(index) else {
            return;
        };
        if tab == self.selected_tab {
            return;
        }
        let _ = selection_changed();
        self.selected_tab = tab;
        cx.notify();
    }

    fn normalize_selection(&mut self) {
        let tabs = self.visible_tabs();
        if tabs.contains(&self.selected_tab) {
            return;
        }
        if let Some(first) = tabs.first() {
            self.selected_tab = *first;
        }
    }

    fn mount_window(&self, tab: SearchResultsTab) -> HashSet<SearchResultsTab> {
        let tabs = self.visible_tabs();
        let Some(index) = tabs.iter().position(|candidate| *candidate == tab) else {
            return HashSet::from([tab]);
        };
        let lower = index.saturating_sub(1);
        let upper = (index + 1).min(tabs.len() - 1);
        tabs[lower..=upper].iter().copied().collect()
    }

    fn prepare_mount_window(&mut self, tab: SearchResultsTab, cx: &mut Context<Self>) {
        let window = self.mount_window(tab);
        self.mounted_tabs.extend(window);
        cx.notify();
    }

    fn schedule_mount_window_trim(&mut self, tab: SearchResultsTab, delay: f64, cx: &mut Context<Self>) {
        self.mount_window_generation = self.mount_window_generation.wrapping_add(1);
        let generation = self.mount_window_generation;
        cx.spawn(async move |this, cx| {
            if delay > 0.0 {
                cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            }
            let _ = this.update(cx, |view, cx| {
                if view.mount_window_generation != generation || view.selected_tab != tab {
                    return;
                }
                view.mounted_tabs = view.mount_window(tab);
                cx.notify();
            });
        })
        .detach();
    }

    fn activate_selected_search(&self, cx: &mut App) {
        let value = self.trimmed_query();
        if value.is_empty() {
            return;
        }
        if let Some(kind) = self.selected_tab.post_kind() {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.activate_post_search(&value, kind, cx));
        } else if let Some(category) = self.selected_tab.global_category() {
            SearchInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.activate_global_search(&value, category, cx));
        }
    }

    fn scroll_anchor_binding(&self, tab: SearchResultsTab) -> Binding<Option<String>> {
        let query = self.trimmed_query();
        let read_store = self.scroll_anchors.clone();
        let read_query = query.clone();
        let write_store = self.scroll_anchors.clone();
        Binding::new(
            move |_| {
                let store = read_store.borrow();
                store.tabs.get(&tab).filter(|anchor| anchor.query == read_query).map(|anchor| anchor.id.clone())
            },
            move |value: Option<String>, _, _| {
                let mut store = write_store.borrow_mut();
                match value {
                    Some(id) => {
                        store.tabs.insert(tab, SearchScrollAnchor { query: query.clone(), id });
                    }
                    None => {
                        store.tabs.remove(&tab);
                    }
                }
            },
        )
    }

    fn default_feed_anchor_binding(&self) -> Binding<Option<String>> {
        let read_store = self.scroll_anchors.clone();
        let write_store = self.scroll_anchors.clone();
        Binding::new(
            move |_| read_store.borrow().default_feed.clone(),
            move |value: Option<String>, _, _| write_store.borrow_mut().default_feed = value,
        )
    }

    fn update_header_tap_suppression(
        live_state: &Entity<AnimatedTabsPagerLiveState>,
        suppressed: bool,
        cx: &mut App,
    ) {
        live_state.update(cx, |live, cx| {
            live.set_tab_taps_suppressed(suppressed, AnimatedTabsMetrics::TAB_TAP_RESTORE_DELAY, cx);
        });
    }
}

impl Render for GlobalSearch {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let this = cx.entity();
        let weak = cx.weak_entity();
        let entity_id = cx.entity_id();
        let nav_store = env::<NavigationStoreKey>(cx);
        let trimmed_query = self.trimmed_query();
        let selected_tab = self.selected_tab;

        let query_view = weak.clone();
        on_change(
            ("global-search-trimmed-query", entity_id),
            trimmed_query.clone(),
            false,
            window,
            cx,
            move |_: &String, value: &String, _, cx| {
                let value = value.clone();
                let _ = query_view.update(cx, |view, cx| view.trimmed_query_did_change(&value, cx));
            },
        );

        let debounce_view = weak.clone();
        let debounce_query = trimmed_query.clone();
        use_task(
            ("global-search-debounce", entity_id),
            trimmed_query.clone(),
            window,
            cx,
            async move |cx: &mut AsyncWindowContext| {
                if debounce_query.is_empty() {
                    return;
                }
                cx.background_executor().timer(SEARCH_DEBOUNCE).await;
                let _ = cx.update(|_, cx| {
                    let _ = debounce_view.update(cx, |view, cx| view.activate_selected_search(cx));
                });
            },
        );

        let tab_view = weak.clone();
        on_change(
            ("global-search-selected-tab", entity_id),
            selected_tab,
            false,
            window,
            cx,
            move |_: &SearchResultsTab, _: &SearchResultsTab, _, cx| {
                let _ = tab_view.update(cx, |view, cx| view.activate_selected_search(cx));
            },
        );

        let anchor = self.pager_live_state.read(cx).resolved_mount_anchor(self.active_index());
        let anchor_view = weak.clone();
        on_change(("global-search-mount-anchor", entity_id), anchor, true, window, cx, move |_, anchor: &i64, _, cx| {
            let anchor = *anchor;
            let _ = anchor_view.update(cx, |view, cx| {
                let Some(tab) = view.tab_at(anchor) else {
                    return;
                };
                view.prepare_mount_window(tab, cx);
                view.schedule_mount_window_trim(tab, 0.0, cx);
            });
        });

        let shows_result_tabs = self.shows_result_tabs();
        let typing_in_field =
            std::mem::take(&mut self.query_edited_by_field) || ScreenFocus::text_field_is_focused(window, cx);
        let keep_focus_view = weak.clone();
        use_task(
            ("global-search-keep-field-focus", entity_id),
            trimmed_query.clone(),
            window,
            cx,
            async move |cx: &mut AsyncWindowContext| {
                if !typing_in_field {
                    return;
                }
                let mut reported_kept = false;
                for step in 0..KEEP_FIELD_FOCUS_CHECKS {
                    if step > 0 {
                        cx.background_executor().timer(KEEP_FIELD_FOCUS_STEP).await;
                    }
                    let window_gone = cx
                        .update(|window, cx| {
                            if ScreenFocus::text_field_is_focused(window, cx) {
                                if !reported_kept {
                                    reported_kept = true;
                                    tracing::warn!(
                                        target: "riseonly::X735",
                                        "search field keeps the focus at step {step}, focused {:?} active {}",
                                        window.focused(cx),
                                        window.is_window_active(),
                                    );
                                }
                                return;
                            }
                            tracing::warn!(
                                target: "riseonly::X735",
                                "search field lost the focus to {:?} at step {step}, restoring",
                                window.focused(cx),
                            );
                            let _ = keep_focus_view.update(cx, |view, cx| {
                                view.search_focus_requests += 1;
                                cx.notify();
                            });
                        })
                        .is_err();
                    if window_gone {
                        return;
                    }
                }
            },
        );
        let tabs_row = shows_result_tabs.then(|| {
            let select_view = weak.clone();
            let header = GlobalSearchTabsHeader {
                tabs: self.visible_tabs(),
                selected_index: Self::active_tab_binding(&this),
                live_state: self.pager_live_state.clone(),
                on_select: Rc::new(move |index: i64, _: &mut Window, cx: &mut App| {
                    let _ = select_view.update(cx, |view, cx| view.select_tab(index, cx));
                }),
            };
            hstack(8.0)
                .w_full()
                .child(div().flex_1().min_w(px(0.0)).h(AnimatedTabsMetrics::BAR_HEIGHT).child(header))
                .children(selected_tab.is_content().then(|| {
                    PostLayoutToggleUi::new(("global-search-layout", entity_id), self.selected_layout_binding(&this))
                }))
        });

        let content = if shows_result_tabs {
            self.default_feed_page = None;
            self.search_pager(&this, cx)
        } else {
            self.pages.clear();
            let scroll_anchor = self.default_feed_anchor_binding();
            let page = self
                .default_feed_page
                .get_or_insert_with(|| cx.new(|cx| GlobalSearchDefaultFeedPage::new(scroll_anchor, cx)))
                .clone();
            AnyView::from(page).into_any_element()
        };

        let back_store = nav_store;
        let appear_view = weak.clone();
        let focus_view = weak.clone();
        let disappear_view = weak;
        div()
            .size_full()
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != SEARCH_FOCUS_KEY || !event.keystroke.modifiers.secondary() {
                    return;
                }
                cx.stop_propagation();
                let _ = focus_view.update(cx, |view, cx| {
                    view.search_focus_requests += 1;
                    cx.notify();
                });
            })
            .child(
                ListShellUi::new()
                    .title(SharedString::from(tr("globalsearch_title")))
                    .shows_back_button(!trimming_whitespaces_and_newlines(&self.initial_query).is_empty())
                    .on_back(move |_, cx| {
                        if let Some(nav_store) = &back_store {
                            nav_store.update(cx, |store, cx| store.pop(cx));
                        }
                    })
                    .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_blurred_header()))
                    .below_header(search_list_column(
                        theme.shell.list_max_width,
                        false,
                        div().w_full().px(px(10.0)).pb(px(10.0)).child(
                            SearchFieldUi::new(
                                ("global-search-field", entity_id),
                                Binding::entity(
                                    this.clone(),
                                    |view: &GlobalSearch| view.query.clone(),
                                    |view, value, _| {
                                        view.query_edited_by_field = true;
                                        view.query = value;
                                    },
                                ),
                            )
                            .uses_inline_text_field(true)
                            .focus_requests(self.search_focus_requests),
                        )
                        .into_any_element(),
                    ))
                    .content(div().relative().size_full().bg(theme.bg._100).child(content))
                    .keyboard_docked_bottom_strip(
                        shows_result_tabs,
                        f32::from(AnimatedTabsMetrics::BAR_HEIGHT) + 16.0,
                        true,
                        match tabs_row {
                            Some(row) => search_list_column(
                                theme.shell.list_max_width,
                                false,
                                row.px(px(10.0)).py(px(8.0)).into_any_element(),
                            ),
                            None => div().into_any_element(),
                        },
                    ),
            )
            .on_appear(("global-search-appear", entity_id), move |_, cx| {
                let _ = appear_view.update(cx, |view, cx| view.screen_did_appear(cx));
            })
            .on_disappear(("global-search-disappear", entity_id), move |_, cx| {
                let _ = disappear_view.update(cx, |view, _| {
                    view.is_on_top = false;
                    view.mount_window_generation = view.mount_window_generation.wrapping_add(1);
                });
            })
    }
}

struct SearchScrollAnchor {
    query: String,
    id: String,
}

/// remounted page restores the saved scroll anchor once its row is loaded, not before
struct SearchScrollRestore {
    pending: Option<String>,
}

impl SearchScrollRestore {
    fn new(anchor: &Binding<Option<String>>, cx: &App) -> Self {
        Self { pending: anchor.get(cx) }
    }

    fn restore(
        &mut self,
        anchor: &Binding<Option<String>>,
        item_count: usize,
        contains: impl Fn(&str) -> Option<usize>,
        list: &ListState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if item_count == 0 {
            return;
        }
        let Some(id) = self.pending.take() else {
            return;
        };
        let Some(item_ix) = contains(&id) else {
            return;
        };
        anchor.set(Some(id), window, cx);
        list.scroll_to(ListOffset { item_ix, offset_in_item: px(0.0) });
    }
}

/// default feed anchor survives the recommendation grid that is torn down while typing
#[derive(Default)]
struct SearchScrollAnchorStore {
    tabs: HashMap<SearchResultsTab, SearchScrollAnchor>,
    default_feed: Option<String>,
}

#[derive(IntoElement)]
struct GlobalSearchTabsHeader {
    tabs: &'static [SearchResultsTab],
    selected_index: Binding<i64>,
    live_state: Entity<AnimatedTabsPagerLiveState>,
    on_select: Rc<dyn Fn(i64, &mut Window, &mut App)>,
}

impl RenderOnce for GlobalSearchTabsHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let tabs = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| TabHeaderConfig::new(index as i64, tr(tab.title_key())))
            .collect();
        let progress_state = self.live_state.clone();
        let tap_state = self.live_state;
        AnimatedTabsHeader::new(
            "global-search-tabs-header",
            tabs,
            self.selected_index,
            Rc::new(move |cx: &App| progress_state.read(cx).drag_progress()),
            0.0,
            self.on_select,
        )
        .suppresses_tap(Rc::new(move |cx: &App| tap_state.read(cx).suppresses_tab_taps()))
        .force_scrollable(true)
    }
}

struct GlobalSearchDefaultFeedPage {
    scroll_anchor: Binding<Option<String>>,
    owner_id: Uuid,
    collection: Option<Entity<SearchPostCollectionPage>>,
    drawn_page: Option<RisePostPagePresentation>,
    _subscriptions: Vec<Subscription>,
}

impl GlobalSearchDefaultFeedPage {
    fn new(scroll_anchor: Binding<Option<String>>, cx: &mut Context<Self>) -> Self {
        let post_service = PostServicesStore::shared(cx);
        Self {
            scroll_anchor,
            owner_id: Uuid::new_v4(),
            collection: None,
            drawn_page: None,
            _subscriptions: vec![cx.observe(&post_service, |page, service, cx| {
                let next = service.update(cx, |service, cx| service.feed_page(RisePostFeedKind::All, cx));
                if page.drawn_page.as_ref() != Some(&next) {
                    cx.notify();
                }
            })],
        }
    }
}

impl Render for GlobalSearchDefaultFeedPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity_id = cx.entity_id();
        let owner_id = self.owner_id;
        let page = PostServicesStore::shared(cx).update(cx, |service, cx| service.feed_page(RisePostFeedKind::All, cx));
        self.drawn_page = Some(page.clone());
        let props = SearchPostCollectionProps {
            page: Rc::new(page),
            uses_grid: true,
            skeleton_variant: None,
            empty_text: tr("post_feed_empty_all"),
            scroll_anchor: self.scroll_anchor.clone(),
            scroll_identity: String::new(),
            on_activate: Rc::new(|cx: &mut App| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.activate_feed(RisePostFeedKind::All, cx));
            }),
            on_refresh: Rc::new(|cx: &mut App| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_feed_kind(RisePostFeedKind::All, cx))
            }),
            on_load_more: Rc::new(|cx: &mut App| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_more_feed_kind(RisePostFeedKind::All, cx));
            }),
            opens_interesting: true,
            origin: None,
            people_query: None,
            on_show_all_people: None,
        };
        div()
            .size_full()
            .child(SearchPostCollectionPage::mount(&mut self.collection, props, cx))
            .on_appear(("global-search-default-feed-appear", entity_id), move |_, cx| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.observe_feed(RisePostFeedKind::All, owner_id, cx));
            })
            .on_disappear(("global-search-default-feed-disappear", entity_id), move |_, cx| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.stop_observing_feed(RisePostFeedKind::All, owner_id, cx));
            })
    }
}

struct GlobalSearchPostPage {
    query: String,
    kind: RisePostSearchKind,
    uses_grid: bool,
    scroll_anchor: Binding<Option<String>>,
    on_show_all_people: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    owner_id: Uuid,
    observed_query: Rc<RefCell<Option<String>>>,
    collection: Option<Entity<SearchPostCollectionPage>>,
    drawn_page: Option<RisePostPagePresentation>,
    _subscriptions: Vec<Subscription>,
}

impl GlobalSearchPostPage {
    fn new(
        query: String,
        kind: RisePostSearchKind,
        uses_grid: bool,
        scroll_anchor: Binding<Option<String>>,
        on_show_all_people: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let post_service = PostServicesStore::shared(cx);
        Self {
            query,
            kind,
            uses_grid,
            scroll_anchor,
            on_show_all_people,
            owner_id: Uuid::new_v4(),
            observed_query: Rc::new(RefCell::new(None)),
            collection: None,
            drawn_page: None,
            _subscriptions: vec![cx.observe(&post_service, |page, service, cx| {
                let (query, kind) = (page.query.clone(), page.kind);
                let next = service.update(cx, |service, cx| service.search_post_page(&query, kind, cx));
                if page.drawn_page.as_ref() != Some(&next) {
                    cx.notify();
                }
            })],
        }
    }

    fn update_observation(
        query: &str,
        kind: RisePostSearchKind,
        owner_id: Uuid,
        observed_query: &Rc<RefCell<Option<String>>>,
        cx: &mut App,
    ) {
        let normalized = RisePostPageScope::normalized_query(query);
        if observed_query.borrow().as_deref() == Some(normalized.as_str()) {
            return;
        }
        Self::stop_observation(kind, owner_id, observed_query, cx);
        *observed_query.borrow_mut() = Some(normalized);
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.observe_post_search(query, kind, owner_id, cx));
    }

    fn stop_observation(
        kind: RisePostSearchKind,
        owner_id: Uuid,
        observed_query: &Rc<RefCell<Option<String>>>,
        cx: &mut App,
    ) {
        let Some(query) = observed_query.borrow_mut().take() else {
            return;
        };
        PostInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.stop_observing_post_search(&query, kind, owner_id, cx));
    }
}

impl Render for GlobalSearchPostPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity_id = cx.entity_id();
        let kind = self.kind;
        let owner_id = self.owner_id;
        let query = self.query.clone();
        let page = PostServicesStore::shared(cx).update(cx, |service, cx| service.search_post_page(&query, kind, cx));
        self.drawn_page = Some(page.clone());
        let identity = format!("{}|{}", RisePostPageScope::normalized_query(&query), kind.wire());

        let task_query = query.clone();
        let task_observed = self.observed_query.clone();
        use_task(
            ("global-search-post-observation", entity_id),
            identity.clone(),
            window,
            cx,
            async move |cx: &mut AsyncWindowContext| {
                let _ = cx.update(|_, cx| {
                    Self::update_observation(&task_query, kind, owner_id, &task_observed, cx);
                });
            },
        );

        let (activate_query, refresh_query, load_query) = (query.clone(), query.clone(), query.clone());
        let props = SearchPostCollectionProps {
            page: Rc::new(page),
            uses_grid: self.uses_grid,
            skeleton_variant: Some(if kind == RisePostSearchKind::Threads {
                PostCardSkeletonVariant::Text
            } else {
                PostCardSkeletonVariant::Media { aspect: PostMediaBox::PLACEHOLDER_ASPECT }
            }),
            empty_text: tr(if kind == RisePostSearchKind::Threads {
                "global_search_threads_empty"
            } else {
                "global_search_publications_empty"
            }),
            scroll_anchor: self.scroll_anchor.clone(),
            scroll_identity: identity,
            on_activate: Rc::new(move |cx: &mut App| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.activate_post_search(&activate_query, kind, cx));
            }),
            on_refresh: Rc::new(move |cx: &mut App| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.refresh_post_search(&refresh_query, kind, cx))
            }),
            on_load_more: Rc::new(move |cx: &mut App| {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_more_post_search(&load_query, kind, cx));
            }),
            opens_interesting: false,
            people_query: (kind == RisePostSearchKind::Publications && self.on_show_all_people.is_some())
                .then(|| query.clone()),
            on_show_all_people: self.on_show_all_people.clone(),
            origin: Some(RisePostPageScope::Search { query, kind }),
        };

        let disappear_observed = self.observed_query.clone();
        div()
            .size_full()
            .child(SearchPostCollectionPage::mount(&mut self.collection, props, cx))
            .on_disappear(("global-search-post-disappear", entity_id), move |_, cx| {
                Self::stop_observation(kind, owner_id, &disappear_observed, cx);
            })
    }
}

#[derive(Clone)]
struct SearchPostCollectionProps {
    page: Rc<RisePostPagePresentation>,
    uses_grid: bool,
    skeleton_variant: Option<PostCardSkeletonVariant>,
    empty_text: String,
    scroll_anchor: Binding<Option<String>>,
    scroll_identity: String,
    on_activate: Rc<dyn Fn(&mut App)>,
    on_refresh: Rc<dyn Fn(&mut App) -> Task<()>>,
    on_load_more: Rc<dyn Fn(&mut App)>,
    opens_interesting: bool,
    origin: Option<RisePostPageScope>,
    people_query: Option<String>,
    on_show_all_people: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl SearchPostCollectionProps {
    fn draws_as(&self, next: &Self) -> bool {
        self.page == next.page
            && self.uses_grid == next.uses_grid
            && self.skeleton_variant == next.skeleton_variant
            && self.empty_text == next.empty_text
            && self.scroll_identity == next.scroll_identity
            && self.opens_interesting == next.opens_interesting
            && self.origin == next.origin
            && self.people_query == next.people_query
            && self.on_show_all_people.is_some() == next.on_show_all_people.is_some()
    }
}

/// what a row of the collection reads, rows are built inside the list closure
struct SearchPostRowFrame {
    page: Rc<RisePostPagePresentation>,
    top_margin: f32,
    people: Option<Entity<GlobalSearchPeopleSection>>,
    cell_side: f32,
    grid_width: f32,
    columns: usize,
    uses_grid: bool,
    opens_interesting: bool,
    origin: Option<RisePostPageScope>,
    on_load_more: Rc<dyn Fn(&mut App)>,
    nav_store: Option<Entity<NavigationStore>>,
    theme: AppTheme,
}

struct SearchPostCollectionPage {
    props: SearchPostCollectionProps,
    list: ListUi,
    rendered_row_ids: Vec<String>,
    rendered_identity: String,
    rendered_uses_grid: bool,
    scroll_restore: SearchScrollRestore,
    container_width: f32,
    chrome: SearchChromeSettle,
    people_section: Option<Entity<GlobalSearchPeopleSection>>,
    hold_context_menu: Entity<HoldContextMenuPresenter>,
    traced_state: String,
    traced_origin: Option<(f32, f32)>,
    _subscriptions: Vec<Subscription>,
}

impl SearchPostCollectionPage {
    fn new(props: SearchPostCollectionProps, cx: &mut Context<Self>) -> Self {
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let list = ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top);
        let scrolled = cx.weak_entity();
        list.state().set_scroll_handler(move |event: &ListScrollEvent, window: &mut Window, cx: &mut App| {
            let first_visible = event.visible_range.start;
            let _ = scrolled.update(cx, |page, cx| {
                page.chrome.did_drag();
                let value = page
                    .rendered_row_ids
                    .get(first_visible)
                    .filter(|id| id.as_str() != BOTTOM_ROW_ID && id.as_str() != LEAD_ROW_ID)
                    .cloned();
                if page.props.scroll_anchor.get(cx) != value {
                    page.props.scroll_anchor.set(value, window, cx);
                }
            });
        });
        let chrome = SearchChromeSettle::new();
        Self {
            rendered_row_ids: Vec::new(),
            rendered_identity: format!("{}|{}", props.scroll_identity, chrome.generation),
            rendered_uses_grid: props.uses_grid,
            scroll_restore: SearchScrollRestore::new(&props.scroll_anchor, cx),
            container_width: 0.0,
            chrome,
            people_section: None,
            list,
            traced_state: String::new(),
            traced_origin: None,
            _subscriptions: vec![observe_menu_lock(&hold_context_menu, cx)],
            hold_context_menu,
            props,
        }
    }

    /// makes the child view on first draw and after that just hands it new props, no notify needed
    fn mount<V: 'static>(
        slot: &mut Option<Entity<Self>>,
        props: SearchPostCollectionProps,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let (entity, stale) = match slot {
            Some(entity) => {
                let stale = entity.update(cx, |page, _| {
                    let stale = !page.props.draws_as(&props);
                    page.props = props;
                    stale
                });
                (entity.clone(), stale)
            }
            None => {
                let entity = cx.new(|cx| Self::new(props, cx));
                *slot = Some(entity.clone());
                (entity, false)
            }
        };
        GlobalSearch::page_view(AnyView::from(entity), stale)
    }

    fn chrome_of(page: &mut Self) -> &mut SearchChromeSettle {
        &mut page.chrome
    }

    fn trace_state(&mut self, page: &RisePostPagePresentation, top_margin: f32, row_count: usize) {
        let status = if page.status == RisePostPageStatus::Idle {
            "idle"
        } else if page.status == RisePostPageStatus::Loading {
            "loading"
        } else if page.status == RisePostPageStatus::Loaded {
            "loaded"
        } else if page.status == RisePostPageStatus::Failed {
            "failed"
        } else {
            "other"
        };
        let branch = if page.items.is_empty() {
            "state"
        } else if self.props.uses_grid && self.container_width <= 0.0 {
            "grid_unmeasured"
        } else {
            "list"
        };
        let signature = format!(
            "status={status} items={} rows={row_count} has_more={} grid={} width={} top_margin={top_margin} branch={branch} error={:?} scope={}",
            page.items.len(),
            page.has_more,
            self.props.uses_grid,
            self.container_width,
            page.error_message,
            self.props.scroll_identity,
        );
        if signature != self.traced_state {
            tracing::warn!(target: "riseonly::global_search", "{signature}");
            self.traced_state = signature;
        }
    }

    fn row_ids(items: &[GetPostFeedResponse], uses_grid: bool, columns: usize) -> Vec<String> {
        if items.is_empty() {
            return Vec::new();
        }
        let mut ids = vec![LEAD_ROW_ID.to_owned()];
        if uses_grid {
            ids.extend(items.chunks(columns.max(1)).map(|row| row[0].presentation_id()));
        } else {
            ids.extend(items.iter().map(GetPostFeedResponse::presentation_id));
        }
        ids.push(BOTTOM_ROW_ID.to_owned());
        ids
    }

    fn lead_row(frame: &SearchPostRowFrame) -> AnyElement {
        vstack(0.0)
            .w_full()
            .child(div().w_full().h(px(frame.top_margin)))
            .children(frame.people.clone().map(|people| AnyView::from(people).into_any_element()))
            .when(frame.uses_grid, |lead| lead.child(div().w_full().h(px(GRID_TOP_GAP))))
            .into_any_element()
    }

    /// people section lives while props ask for it, a new query reaches it in the same pass
    fn people_section(&mut self, cx: &mut Context<Self>) -> Option<Entity<GlobalSearchPeopleSection>> {
        let Some(query) = self.props.people_query.clone() else {
            self.people_section = None;
            return None;
        };
        let on_show_all = self.props.on_show_all_people.clone();
        let section = match &self.people_section {
            Some(section) => section.clone(),
            None => {
                let created = cx.new(|cx| GlobalSearchPeopleSection::new(query.clone(), on_show_all.clone(), cx));
                self.people_section = Some(created.clone());
                created
            }
        };
        section.update(cx, |section, _| {
            section.query = query;
            section.on_show_all = on_show_all;
        });
        Some(section)
    }

    fn posts_grid(frame: &SearchPostRowFrame, row: usize) -> AnyElement {
        let count = frame.page.items.len();
        let start = row * frame.columns;
        let end = (start + frame.columns).min(count);
        div()
            .w_full()
            .flex()
            .flex_row()
            .justify_center()
            .child(
                hstack(GRID_SPACING)
                    .w(px(frame.grid_width))
                    .items_start()
                    .when(end < count, |line| line.pb(px(GRID_SPACING)))
                    .children(
                        (start..end).map(|index| div().size(px(frame.cell_side)).child(Self::grid_cell(frame, index))),
                    ),
            )
            .into_any_element()
    }

    fn grid_cell(frame: &SearchPostRowFrame, index: usize) -> AnyElement {
        let post = Rc::new(frame.page.items[index].clone());
        let presentation_id = post.presentation_id();
        let open_post = post.clone();
        let content_post = post.clone();
        let opens_interesting = frame.opens_interesting;
        let origin = frame.origin.clone();
        let nav_store = frame.nav_store.clone();
        let on_load_more = frame.on_load_more.clone();
        let side = frame.cell_side;
        PostHoldContextMenu::new(
            SharedString::from(format!("global-search-post-hold-{presentation_id}")),
            post,
            move |_: &mut Window, cx: &mut App| {
                let previews = nav_store.as_ref().is_some_and(|store| store.read(cx).aside_takes_details());
                if previews && !open_post.is_riff_only() {
                    let route = if opens_interesting {
                        RootRoute::PostsInteresting { selected_post: Some((*open_post).clone()) }
                    } else {
                        RootRoute::PostDetail { post_id: Some(open_post.id.clone()), origin: origin.clone() }
                    };
                    preview_route(&nav_store, route, cx);
                    return;
                }
                let route = if opens_interesting && !open_post.is_riff_only() {
                    RootRoute::PostsInteresting { selected_post: Some((*open_post).clone()) }
                } else {
                    RootRoute::post_content(&open_post, origin.clone())
                };
                push_route(&nav_store, route, cx);
            },
            move |_: &mut Window, _: &mut App| {
                div().size(px(side)).child(GridPostCell::new((*content_post).clone()))
            },
        )
        .post_pagination_trigger_threshold(
            SharedString::from(format!("global-search-post-pagination-{presentation_id}")),
            index as i64,
            frame.page.items.len() as i64,
            frame.page.has_more,
            frame.page.is_loading_more,
            GRID_LOAD_MORE_THRESHOLD,
            move |cx: &mut App| on_load_more(cx),
        )
        .into_any_element()
    }

    fn posts_list(frame: &SearchPostRowFrame, index: usize) -> AnyElement {
        let post = Rc::new(frame.page.items[index].clone());
        let presentation_id = post.presentation_id();
        let comments_post = post.clone();
        let avatar_post = post.clone();
        let like_post = post.clone();
        let fav_post = post.clone();
        let nav_store = frame.nav_store.clone();
        let on_load_more = frame.on_load_more.clone();
        let count = frame.page.items.len();
        let column = vstack(5.0)
            .w_full()
            .when(index + 1 < count, |row| row.pb(px(5.0)))
            .child(
                div().w_full().child(
                    Post::new(SharedString::from(presentation_id.clone()), post)
                        .feed_row(true)
                        .view_surface(RisePostViewSurface::Search)
                        .on_open_comments(move |_, cx| {
                            PostInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.open_comments(&comments_post, false, cx));
                        })
                        .on_avatar_tap(move |_, cx| {
                            let route = RootRoute::UserPage {
                                user_id: avatar_post.author.id.clone(),
                                tag: Some(avatar_post.author.tag.clone()),
                            };
                            preview_route(&nav_store, route, cx);
                        })
                        .on_like(move |_, cx| {
                            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&like_post, cx));
                        })
                        .on_fav(move |_, cx| {
                            PostInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.toggle_favorite(&fav_post, cx));
                        })
                        .post_pagination_trigger(
                            SharedString::from(format!("global-search-post-pagination-{presentation_id}")),
                            index as i64,
                            count as i64,
                            frame.page.has_more,
                            frame.page.is_loading_more,
                            move |cx: &mut App| on_load_more(cx),
                        ),
                ),
            )
            .child(div().w_full().h(px(1.0)).bg(frame.theme.border._100));
        div()
            .w_full()
            .flex()
            .flex_row()
            .justify_center()
            .child(div().w_full().max_w(frame.theme.feed.content_width).child(column))
            .into_any_element()
    }

    fn retry(on_refresh: &Rc<dyn Fn(&mut App) -> Task<()>>, cx: &mut App) {
        on_refresh(cx).detach();
    }
}

impl Render for SearchPostCollectionPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity_id = cx.entity_id();
        let props = self.props.clone();
        let page = props.page.clone();
        let uses_grid = props.uses_grid;
        let locked = self.hold_context_menu.read(cx).active_request().is_some();
        let people = self.people_section(cx);

        let anchor = props.scroll_anchor.get(cx);
        let is_at_top =
            anchor.is_none() || anchor == page.items.first().map(GetPostFeedResponse::presentation_id);
        if self.chrome.inset_did_change(list_shell_floating_top_inset(cx), is_at_top) {
            props.scroll_anchor.set(None, window, cx);
        }
        let top_margin = self.chrome.margin();
        let identity = format!("{}|{}", props.scroll_identity, self.chrome.generation);
        if self.rendered_identity != identity {
            self.rendered_identity = identity;
            self.list.replaced(0);
            self.rendered_row_ids.clear();
            self.scroll_restore = SearchScrollRestore::new(&props.scroll_anchor, cx);
        }
        if self.rendered_uses_grid != uses_grid {
            self.rendered_uses_grid = uses_grid;
            self.list.replaced(0);
            self.rendered_row_ids.clear();
            self.scroll_restore = SearchScrollRestore::new(&props.scroll_anchor, cx);
        }

        let columns = grid_column_count(self.container_width);
        let row_ids = Self::row_ids(&page.items, uses_grid, columns);
        splice_list_rows(self.list.state(), &mut self.rendered_row_ids, &row_ids);
        let anchored_items = page.clone();
        self.scroll_restore.restore(
            &props.scroll_anchor,
            page.items.len(),
            move |id| {
                let position = anchored_items.items.iter().position(|post| post.presentation_id() == id)?;
                Some(1 + if uses_grid { position / columns } else { position })
            },
            self.list.state(),
            window,
            cx,
        );

        let retry_action = props.on_refresh.clone();
        let error_text = page.error_message.clone().unwrap_or_else(|| tr("error_fetching_data"));
        let skeleton_variant = props.skeleton_variant;
        let state = PostPageStateUi::new(
            ("global-search-post-state", entity_id),
            page.status,
            !page.items.is_empty(),
            if uses_grid { "square.grid.3x3" } else { "text.bubble" },
            empty_or_refusal_text(page.status, page.items.is_empty(), &page.error_message, props.empty_text.clone()),
            error_text,
            move |cx| Self::retry(&retry_action, cx),
        );

        let refresh_action = props.on_refresh.clone();
        let refresh = move |_: &mut Window, cx: &mut App| refresh_action(cx);
        let feed_width = f32::from(rise_ui::theme(cx).feed.content_width);
        let grid_width = self.container_width.max(0.0);
        let cell_side = ((grid_width - GRID_SPACING * (columns - 1) as f32) / columns as f32).max(0.0);
        let skeleton_width = self.container_width.min(feed_width).max(0.0);
        let skeleton_side =
            ((skeleton_width - GRID_SPACING * (GRID_MIN_COLUMNS - 1) as f32) / GRID_MIN_COLUMNS as f32).max(0.0);
        let skeleton_cell_height = (skeleton_side > 0.0).then_some(skeleton_side);
        self.trace_state(&page, top_margin, row_ids.len());
        let body = if page.items.is_empty() {
            scroll_view(("global-search-post-state-scroll", entity_id))
                .shows_indicators(false)
                .scroll_disabled(locked)
                .refreshable(refresh)
                .child(
                    vstack(0.0)
                        .w_full()
                        .child(div().w_full().h(px(top_margin)))
                        .children(people.clone().map(|people| AnyView::from(people).into_any_element()))
                        .child(
                            div().w_full().min_h(px(STATE_MIN_HEIGHT)).child(state.loading(move |_, _| {
                                if uses_grid {
                                    div()
                                        .w_full()
                                        .flex()
                                        .flex_row()
                                        .justify_center()
                                        .child(
                                            div().w_full().max_w(px(feed_width)).child(
                                                PostGridSkeleton::default().cell_height(skeleton_cell_height).rows(5),
                                            ),
                                        )
                                        .into_any_element()
                                } else {
                                    PostFeedSkeleton::default()
                                        .card_count(5)
                                        .top_padding(0.0)
                                        .fixed_variant(skeleton_variant)
                                        .into_any_element()
                                }
                            })),
                        )
                        .child(spacer_min_h(BOTTOM_SPACER)),
                )
                .into_any_element()
        } else if uses_grid && self.container_width <= 0.0 {
            div()
                .size_full()
                .overflow_hidden()
                .child(div().w_full().h(px(top_margin)))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .justify_center()
                        .child(div().w_full().max_w(px(feed_width)).child(PostGridSkeleton::default().rows(5))),
                )
                .into_any_element()
        } else {
            let frame = SearchPostRowFrame {
                page: page.clone(),
                top_margin,
                people,
                cell_side,
                grid_width,
                columns,
                uses_grid,
                opens_interesting: props.opens_interesting,
                origin: props.origin.clone(),
                on_load_more: props.on_load_more.clone(),
                nav_store: env::<NavigationStoreKey>(cx),
                theme: rise_ui::theme(cx).clone(),
            };
            let row_count = row_ids.len();
            let rows = self.list.refreshable(
                ("global-search-post-list", entity_id),
                refresh,
                move |index: usize, _: &mut Window, _: &mut App| {
                    if index + 1 >= row_count {
                        return div().w_full().h(px(BOTTOM_SPACER)).into_any_element();
                    }
                    if index == 0 {
                        return Self::lead_row(&frame);
                    }
                    if uses_grid { Self::posts_grid(&frame, index - 1) } else { Self::posts_list(&frame, index - 1) }
                },
            );
            list_with_state(rows, state)
        };

        let on_activate = props.on_activate.clone();
        let measured = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .child(env_scope::<HoldContextMenuLocksScrollKey>(locked).child(body))
            .children(locked.then(|| div().absolute().inset_0().occlude()))
            .child(measure(("global-search-post-geometry", entity_id), move |bounds, _, cx| {
                let _ = measured.update(cx, |page, cx| {
                    let origin = (f32::from(bounds.origin.x).round(), f32::from(bounds.size.height).round());
                    if page.traced_origin != Some(origin) {
                        tracing::warn!(
                            target: "riseonly::global_search",
                            "X541 page at {bounds:?} scope={} rows={} visible={:?}",
                            page.props.scroll_identity,
                            page.rendered_row_ids.len(),
                            page.list.state().logical_scroll_top(),
                        );
                        page.traced_origin = Some(origin);
                    }
                    let width = f32::from(bounds.size.width);
                    if page.container_width != width {
                        tracing::warn!(target: "riseonly::global_search", "collection bounds {bounds:?}");
                        page.container_width = width;
                        page.list.state().remeasure();
                        SearchChromeSettle::width_did_change(page, width, Self::chrome_of, cx);
                        cx.notify();
                    }
                });
            }))
            .on_appear(("global-search-post-activate", entity_id), move |_, cx| on_activate(cx))
    }
}

/// what a row of the entity page reads, rows are built inside the list closure
struct GlobalSearchEntityRowFrame {
    page: Rc<RiseGlobalSearchPagePresentation>,
    top_margin: f32,
    query: String,
    category: RiseGlobalSearchCategory,
    nav_store: Option<Entity<NavigationStore>>,
    theme: AppTheme,
}

struct GlobalSearchEntityPage {
    query: String,
    category: RiseGlobalSearchCategory,
    scroll_anchor: Binding<Option<String>>,
    owner_id: Uuid,
    observed_scope: Rc<RefCell<Option<RiseGlobalSearchScope>>>,
    list: ListUi,
    rendered_row_ids: Vec<String>,
    rendered_identity: String,
    scroll_restore: SearchScrollRestore,
    chrome: SearchChromeSettle,
    hold_context_menu: Entity<HoldContextMenuPresenter>,
    shown_page: Rc<RiseGlobalSearchPagePresentation>,
    _subscriptions: Vec<Subscription>,
}

impl GlobalSearchEntityPage {
    fn search_service_did_change(&mut self, cx: &mut Context<Self>) {
        let (query, category) = (self.query.clone(), self.category);
        let next = SearchServicesStore::shared(cx).update(cx, |service, cx| service.global_search_page(&query, category, cx));
        if *self.shown_page == next {
            return;
        }
        self.shown_page = Rc::new(next);
        cx.notify();
    }

    fn new(
        query: String,
        category: RiseGlobalSearchCategory,
        scroll_anchor: Binding<Option<String>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_service = SearchServicesStore::shared(cx);
        let shown_page = Rc::new(search_service.update(cx, |service, cx| service.global_search_page(&query, category, cx)));
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let list = ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top);
        let scrolled = cx.weak_entity();
        list.state().set_scroll_handler(move |event: &ListScrollEvent, window: &mut Window, cx: &mut App| {
            let first_visible = event.visible_range.start;
            let _ = scrolled.update(cx, |page, cx| {
                page.chrome.did_drag();
                let value = page
                    .rendered_row_ids
                    .get(first_visible)
                    .filter(|id| id.as_str() != BOTTOM_ROW_ID && id.as_str() != LEAD_ROW_ID)
                    .cloned();
                if page.scroll_anchor.get(cx) != value {
                    page.scroll_anchor.set(value, window, cx);
                }
            });
        });
        let chrome = SearchChromeSettle::new();
        Self {
            rendered_identity: format!(
                "{}|{}",
                RiseGlobalSearchScope { query: query.clone(), category }.key(),
                chrome.generation
            ),
            chrome,
            query,
            category,
            scroll_restore: SearchScrollRestore::new(&scroll_anchor, cx),
            scroll_anchor,
            owner_id: Uuid::new_v4(),
            observed_scope: Rc::new(RefCell::new(None)),
            list,
            rendered_row_ids: Vec::new(),
            shown_page,
            _subscriptions: vec![
                cx.observe(&search_service, |page, _, cx| page.search_service_did_change(cx)),
                observe_menu_lock(&hold_context_menu, cx),
            ],
            hold_context_menu,
        }
    }

    fn update_observation(
        next: RiseGlobalSearchScope,
        owner_id: Uuid,
        observed_scope: &Rc<RefCell<Option<RiseGlobalSearchScope>>>,
        cx: &mut App,
    ) {
        if observed_scope.borrow().as_ref() == Some(&next) {
            return;
        }
        Self::stop_observation(owner_id, observed_scope, cx);
        SearchInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.observe_global_search(&next.query, next.category, owner_id, cx));
        *observed_scope.borrow_mut() = Some(next);
    }

    fn stop_observation(owner_id: Uuid, observed_scope: &Rc<RefCell<Option<RiseGlobalSearchScope>>>, cx: &mut App) {
        let Some(scope) = observed_scope.borrow_mut().take() else {
            return;
        };
        SearchInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.stop_observing_global_search(&scope.query, scope.category, owner_id, cx));
    }

    fn chrome_of(page: &mut Self) -> &mut SearchChromeSettle {
        &mut page.chrome
    }

    fn entity_row(frame: &GlobalSearchEntityRowFrame, row: &RiseGlobalSearchRow) -> AnyElement {
        match row {
            RiseGlobalSearchRow::Profile(profile) => {
                let tap_profile = profile.clone();
                let tap_store = frame.nav_store.clone();
                let action_profile = profile.clone();
                let action_store = frame.nav_store.clone();
                GlobalSearchProfileResultRow::new(
                    profile.clone(),
                    move |_: &mut Window, cx: &mut App| Self::open_profile(&tap_profile, &tap_store, cx),
                    move |action: String, _: &mut Window, cx: &mut App| {
                        Self::handle_profile_action(&action, &action_profile, &action_store, cx);
                    },
                )
                .into_any_element()
            }
            RiseGlobalSearchRow::Channel(chat) | RiseGlobalSearchRow::Group(chat) => {
                let tap_chat = chat.clone();
                let tap_store = frame.nav_store.clone();
                let action_chat = chat.clone();
                let action_store = frame.nav_store.clone();
                GlobalSearchChatResultRow::new(
                    chat.clone(),
                    move |_: &mut Window, cx: &mut App| Self::open_chat(&tap_chat, &tap_store, cx),
                    move |action: String, _: &mut Window, cx: &mut App| {
                        Self::handle_chat_action(&action, &action_chat, &action_store, cx);
                    },
                )
                .into_any_element()
            }
            RiseGlobalSearchRow::Post(_) => div().into_any_element(),
        }
    }

    fn list_row(frame: &GlobalSearchEntityRowFrame, index: usize) -> AnyElement {
        let row = &frame.page.items[index];
        let query = frame.query.clone();
        let category = frame.category;
        vstack(0.0)
            .w_full()
            .child(
                div().w_full().child(Self::entity_row(frame, row).post_pagination_trigger_threshold(
                    SharedString::from(format!("global-search-entity-pagination-{}", row.id())),
                    index as i64,
                    frame.page.items.len() as i64,
                    frame.page.has_more,
                    frame.page.is_loading_more,
                    ENTITY_LOAD_MORE_THRESHOLD,
                    move |cx: &mut App| {
                        SearchInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.load_more_global_search(&query, category, cx));
                    },
                )),
            )
            .child(div().w_full().h(px(1.0)).bg(frame.theme.border._100))
            .into_any_element()
    }

    fn empty_icon(category: RiseGlobalSearchCategory) -> &'static str {
        match category {
            RiseGlobalSearchCategory::Profiles => "person.2",
            RiseGlobalSearchCategory::Channels => "megaphone",
            RiseGlobalSearchCategory::Groups => "person.3",
            RiseGlobalSearchCategory::Publications => "photo.on.rectangle.angled",
            RiseGlobalSearchCategory::Threads => "text.bubble",
        }
    }

    fn empty_text_key(category: RiseGlobalSearchCategory) -> &'static str {
        match category {
            RiseGlobalSearchCategory::Profiles => "global_search_profiles_empty",
            RiseGlobalSearchCategory::Channels => "global_search_channels_empty",
            RiseGlobalSearchCategory::Groups => "global_search_groups_empty",
            RiseGlobalSearchCategory::Publications => "global_search_publications_empty",
            RiseGlobalSearchCategory::Threads => "global_search_threads_empty",
        }
    }

    fn refresh(query: &str, category: RiseGlobalSearchCategory, cx: &mut App) -> Task<()> {
        SearchInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_global_search(query, category, cx))
    }

    fn open_profile(profile: &SearchEntityRow, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        GlobalSearchProfileActions::open(profile, nav_store, cx);
    }

    fn handle_profile_action(
        action: &str,
        profile: &SearchEntityRow,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        GlobalSearchProfileActions::handle(action, profile, nav_store, cx);
    }

    fn handle_chat_action(
        action: &str,
        chat: &SearchEntityRow,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        let Some(chat_id) = Self::normalized(chat.chat_id.as_deref()) else {
            return;
        };
        match action {
            "open" => Self::push_chat(chat, chat_id, nav_store, cx),
            "copy" => {
                let link = RiseonlyPublicLink::Chat { id: chat_id };
                if let Some(url) = link.web_url(AppEnvironment::current()) {
                    UIPasteboard::general().set_url(&url, cx);
                }
            }
            _ => {}
        }
    }

    fn open_chat(chat: &SearchEntityRow, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        let Some(chat_id) = Self::normalized(chat.chat_id.as_deref()) else {
            return;
        };
        Self::push_chat(chat, chat_id, nav_store, cx);
    }

    fn push_chat(chat: &SearchEntityRow, chat_id: String, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.note_community_opening(
                &chat_id,
                chat.is_channel(),
                &chat.display_name,
                &chat.logo_url,
                chat.user_tag.as_deref(),
                chat.is_premium,
                chat.is_public.unwrap_or(true),
                chat.member_count,
                cx,
            );
        });
        preview_route(nav_store, RootRoute::Chat { chat_id: Some(chat_id) }, cx);
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(value?);
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }
}

pub(crate) enum GlobalSearchProfileActions {}

impl GlobalSearchProfileActions {
    pub(crate) fn open(profile: &SearchEntityRow, nav_store: &Option<Entity<NavigationStore>>, cx: &mut App) {
        let Some(route) = GlobalSearchRouteResolver::route(profile) else {
            return;
        };
        preview_route(nav_store, route, cx);
    }

    pub(crate) fn handle(
        action: &str,
        profile: &SearchEntityRow,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        match action {
            "open" => Self::open(profile, nav_store, cx),
            "copy" => UIPasteboard::general().set_string(&profile.tag_line, cx),
            "report" => {
                let me = AuthServicesStore::shared(cx)
                    .read(cx)
                    .profile()
                    .map(|profile| profile.id.clone())
                    .unwrap_or_default();
                if let Some(user_id) = profile.user_id.clone()
                    && !user_id.is_empty()
                    && user_id != me
                {
                    let subject = ReportSubject {
                        name: profile.display_name.clone(),
                        tag: profile.user_tag.clone().unwrap_or_default(),
                        avatar_url: profile.logo_url.clone(),
                        user_id: Some(user_id.clone()),
                    };
                    ReportSheetStore::shared(cx)
                        .update(cx, |sheet, cx| sheet.present(ReportTarget::User(user_id), Some(subject), None, cx));
                }
            }
            _ => {}
        }
    }
}

impl Render for GlobalSearchEntityPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity_id = cx.entity_id();
        let query = self.query.clone();
        let category = self.category;
        let owner_id = self.owner_id;
        let page = self.shown_page.clone();
        let scope = RiseGlobalSearchScope { query: query.clone(), category };
        let identity = scope.key();
        let locked = self.hold_context_menu.read(cx).active_request().is_some();

        let task_observed = self.observed_scope.clone();
        use_task(
            ("global-search-entity-observation", entity_id),
            identity.clone(),
            window,
            cx,
            async move |cx: &mut AsyncWindowContext| {
                let _ = cx.update(|_, cx| Self::update_observation(scope, owner_id, &task_observed, cx));
            },
        );

        let anchor = self.scroll_anchor.get(cx);
        let is_at_top = anchor.is_none() || anchor == page.items.first().map(RiseGlobalSearchRow::id);
        if self.chrome.inset_did_change(list_shell_floating_top_inset(cx), is_at_top) {
            self.scroll_anchor.set(None, window, cx);
        }
        let top_margin = self.chrome.margin();
        let identity = format!("{}|{}", identity, self.chrome.generation);
        if self.rendered_identity != identity {
            self.rendered_identity = identity;
            self.list.replaced(0);
            self.rendered_row_ids.clear();
            self.scroll_restore = SearchScrollRestore::new(&self.scroll_anchor, cx);
        }

        let mut row_ids: Vec<String> = Vec::new();
        if !page.items.is_empty() {
            row_ids.push(LEAD_ROW_ID.to_owned());
            row_ids.extend(page.items.iter().map(RiseGlobalSearchRow::id));
            row_ids.push(BOTTOM_ROW_ID.to_owned());
        }
        splice_list_rows(self.list.state(), &mut self.rendered_row_ids, &row_ids);
        let anchored_items = page.clone();
        self.scroll_restore.restore(
            &self.scroll_anchor,
            page.items.len(),
            move |id| anchored_items.items.iter().position(|row| row.id() == id).map(|position| position + 1),
            self.list.state(),
            window,
            cx,
        );

        let retry_query = query.clone();
        let state = PostPageStateUi::new(
            ("global-search-entity-state", entity_id),
            page.status,
            !page.items.is_empty(),
            Self::empty_icon(category),
            empty_or_refusal_text(page.status, page.items.is_empty(), &page.error_message, tr(Self::empty_text_key(category))),
            page.error_message.clone().unwrap_or_else(|| tr("error_fetching_data")),
            move |cx| Self::refresh(&retry_query, category, cx).detach(),
        );

        let refresh_query = query.clone();
        let refresh = move |_: &mut Window, cx: &mut App| Self::refresh(&refresh_query, category, cx);
        let body = if page.items.is_empty() {
            scroll_view(("global-search-entity-state-scroll", entity_id))
                .shows_indicators(false)
                .scroll_disabled(locked)
                .refreshable(refresh)
                .child(
                    vstack(0.0)
                        .w_full()
                        .child(div().w_full().h(px(top_margin)))
                        .child(
                            div()
                                .w_full()
                                .min_h(px(STATE_MIN_HEIGHT))
                                .child(state.loading(|_, _| UserListSkeleton::new())),
                        )
                        .child(spacer_min_h(BOTTOM_SPACER)),
                )
                .into_any_element()
        } else {
            let frame = GlobalSearchEntityRowFrame {
                page: page.clone(),
                top_margin,
                query,
                category,
                nav_store: env::<NavigationStoreKey>(cx),
                theme: rise_ui::theme(cx).clone(),
            };
            let row_count = row_ids.len();
            let rows = self.list.refreshable(
                ("global-search-entity-list", entity_id),
                refresh,
                move |index: usize, _: &mut Window, _: &mut App| {
                    if index + 1 >= row_count {
                        return div().w_full().h(px(BOTTOM_SPACER)).into_any_element();
                    }
                    if index == 0 {
                        return div().w_full().h(px(frame.top_margin)).into_any_element();
                    }
                    Self::list_row(&frame, index - 1)
                },
            );
            list_with_state(rows, state)
        };

        let disappear_observed = self.observed_scope.clone();
        let measured = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .child(env_scope::<HoldContextMenuLocksScrollKey>(locked).child(body))
            .children(locked.then(|| div().absolute().inset_0().occlude()))
            .child(measure(("global-search-entity-geometry", entity_id), move |bounds, _, cx| {
                let _ = measured.update(cx, |page, cx| {
                    SearchChromeSettle::width_did_change(page, f32::from(bounds.size.width), Self::chrome_of, cx);
                });
            }))
            .on_disappear(("global-search-entity-disappear", entity_id), move |_, cx| {
                Self::stop_observation(owner_id, &disappear_observed, cx);
            })
    }
}

/// best people above the posts for a plain text query, up to three rows
/// hidden while loading, failed or when nobody matches
struct GlobalSearchPeopleSection {
    query: String,
    on_show_all: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    owner_id: Uuid,
    observed_scope: Rc<RefCell<Option<RiseGlobalSearchScope>>>,
    shown_page: Rc<RiseGlobalSearchPagePresentation>,
    _subscriptions: Vec<Subscription>,
}

impl GlobalSearchPeopleSection {
    const COLLAPSED_LIMIT: usize = 3;

    fn search_service_did_change(&mut self, cx: &mut Context<Self>) {
        let query = self.query.clone();
        let next = SearchServicesStore::shared(cx)
            .update(cx, |service, cx| service.global_search_page(&query, RiseGlobalSearchCategory::Profiles, cx));
        if *self.shown_page == next {
            return;
        }
        self.shown_page = Rc::new(next);
        cx.notify();
    }

    fn new(query: String, on_show_all: Option<Rc<dyn Fn(&mut Window, &mut App)>>, cx: &mut Context<Self>) -> Self {
        let search_service = SearchServicesStore::shared(cx);
        let shown_page = Rc::new(
            search_service.update(cx, |service, cx| service.global_search_page(&query, RiseGlobalSearchCategory::Profiles, cx)),
        );
        Self {
            query,
            on_show_all,
            owner_id: Uuid::new_v4(),
            observed_scope: Rc::new(RefCell::new(None)),
            shown_page,
            _subscriptions: vec![cx.observe(&search_service, |section, _, cx| section.search_service_did_change(cx))],
        }
    }

    fn scope(query: &str) -> RiseGlobalSearchScope {
        RiseGlobalSearchScope { query: query.to_owned(), category: RiseGlobalSearchCategory::Profiles }
    }
}

impl Render for GlobalSearchPeopleSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity_id = cx.entity_id();
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let query = self.query.clone();
        let owner_id = self.owner_id;
        let page = self.shown_page.clone();
        let rows: Vec<SearchEntityRow> = page
            .items
            .iter()
            .filter_map(|row| match row {
                RiseGlobalSearchRow::Profile(profile) => Some(profile.clone()),
                _ => None,
            })
            .collect();

        let task_observed = self.observed_scope.clone();
        let task_query = query.clone();
        use_task(
            ("global-search-people-observation", entity_id),
            Self::scope(&query).key(),
            window,
            cx,
            async move |cx: &mut AsyncWindowContext| {
                let _ = cx.update(|_, cx| {
                    GlobalSearchEntityPage::update_observation(
                        Self::scope(&task_query),
                        owner_id,
                        &task_observed,
                        cx,
                    );
                    SearchInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.activate_global_search(&task_query, RiseGlobalSearchCategory::Profiles, cx);
                    });
                });
            },
        );

        let section = (!rows.is_empty()).then(|| {
            let shows_more = (rows.len() > Self::COLLAPSED_LIMIT || page.has_more) && self.on_show_all.is_some();
            let mut header = ChatSearchSectionHeader::new(
                ("global-search-people-header", entity_id),
                tr("global_search_tab_profiles").to_uppercase(),
            )
            .right_title(shows_more.then(|| tr("common_show_more")));
            if let Some(on_show_all) = self.on_show_all.clone() {
                header = header.on_right(move |window: &mut Window, cx: &mut App| on_show_all(window, cx));
            }
            let profiles = rows.iter().take(Self::COLLAPSED_LIMIT).map(|profile| {
                let (tap_profile, tap_store) = (profile.clone(), nav_store.clone());
                let (action_profile, action_store) = (profile.clone(), nav_store.clone());
                vstack(0.0)
                    .w_full()
                    .child(GlobalSearchProfileResultRow::new(
                        profile.clone(),
                        move |_: &mut Window, cx: &mut App| {
                            GlobalSearchProfileActions::open(&tap_profile, &tap_store, cx);
                        },
                        move |action: String, _: &mut Window, cx: &mut App| {
                            GlobalSearchProfileActions::handle(&action, &action_profile, &action_store, cx);
                        },
                    ))
                    .child(div().w_full().h(px(1.0)).bg(theme.border._100))
            });
            vstack(0.0).w_full().child(header).children(profiles).child(div().w_full().h(px(8.0)))
        });

        let appear_observed = self.observed_scope.clone();
        let appear_query = query;
        let disappear_observed = self.observed_scope.clone();
        vstack(0.0)
            .w_full()
            .children(section)
            .on_appear(("global-search-people-appear", entity_id), move |_, cx| {
                GlobalSearchEntityPage::update_observation(Self::scope(&appear_query), owner_id, &appear_observed, cx);
            })
            .on_disappear(("global-search-people-disappear", entity_id), move |_, cx| {
                GlobalSearchEntityPage::stop_observation(owner_id, &disappear_observed, cx);
            })
    }
}

/// fills its parent which is the square, the grid sizes rows from the measured column
#[derive(IntoElement)]
pub struct GridPostCell {
    pub post: GetPostFeedResponse,
}

impl GridPostCell {
    pub fn new(post: GetPostFeedResponse) -> Self {
        Self { post }
    }

    fn corner_badge(icon: &str, top_trailing: bool) -> Div {
        div()
            .absolute()
            .top(px(6.0))
            .when(top_trailing, |badge| badge.right(px(6.0)))
            .when(!top_trailing, |badge| badge.left(px(6.0)))
            .p(px(6.0))
            .rounded_full()
            .bg(gpui::black().opacity(0.42))
            .children(IconUi::sized(icon, px(12.0), gpui::white(), icon.ends_with(".fill")))
    }
}

impl RenderOnce for GridPostCell {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let content_post = self.post;
        let is_own_post = PostMediaCachePolicy::is_own_post(&content_post, cx);
        let preview_source = ProfileGridPreviewSourcePolicy::source(
            content_post.local_preview_url.as_deref(),
            content_post.images.first().map(String::as_str),
            content_post.video_thumbnail.as_deref(),
        )
        .or_else(|| {
            ProfileGridPreviewSourcePolicy::source(
                None,
                content_post.track.as_ref().and_then(|track| track.cover_url.as_deref()),
                None,
            )
        });
        let has_preview = preview_source.is_some();
        let image_count = content_post.images.len();
        let shows_count = has_preview && image_count > 1 && !content_post.is_short && !content_post.is_image_reel;
        let is_short = content_post.is_short;
        let is_image_reel = content_post.is_image_reel;
        let shows_track = content_post.track_id.is_some() && !content_post.is_short;

        let media = match preview_source {
            Some(preview_source) => CleverImageUi::new(preview_source)
                .content_mode(ContentMode::Fill)
                .corner_radius(0.0)
                .persist_to_disk(is_own_post)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(720)
                .cache_policy(PostMediaCachePolicy::image_policy(is_own_post))
                .into_any_element(),
            None => PostGridTextUi::new(content_post).into_any_element(),
        };

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(div().absolute().inset_0().child(media))
            .when(has_preview, |cell| {
                cell.child(
                    div().absolute().inset_0().child(
                        LinearGradient::colors(
                            [gpui::transparent_black(), gpui::black().opacity(0.4)],
                            UnitPoint::CENTER,
                            UnitPoint::BOTTOM,
                        )
                        .view(),
                    ),
                )
            })
            .when(shows_count, |cell| {
                cell.child(
                    hstack(3.0)
                        .absolute()
                        .top(px(6.0))
                        .right(px(6.0))
                        .px(px(6.0))
                        .py(px(3.0))
                        .rounded_full()
                        .bg(gpui::black().opacity(0.4))
                        .children(IconUi::sized("square.on.square", px(10.0), gpui::white(), false))
                        .child(
                            MainText::new(image_count.to_string())
                                .px(10.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(gpui::white()),
                        ),
                )
            })
            .when(is_short, |cell| cell.child(Self::corner_badge("play.fill", true)))
            .when(!is_short && is_image_reel, |cell| cell.child(Self::corner_badge("photo.stack", true)))
            .when(shows_track, |cell| cell.child(Self::corner_badge("music.note", false)))
    }
}

#[derive(IntoElement)]
pub struct UserRow {
    pub author: AuthorInfo,
}

impl UserRow {
    pub fn new(author: AuthorInfo) -> Self {
        Self { author }
    }
}

impl RenderOnce for UserRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let author = self.author;
        let user_id = author.id.clone().map(SharedString::from);
        let name_color = PremiumNameColor::color_for_user(author.id.as_deref(), Some(&author.more), cx);
        let logo_id = SharedString::from(format!("user-row-logo-{}-{}", author.id.clone().unwrap_or_default(), author.tag));
        hstack(10.0)
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .bg(theme.bg._100)
            .child(
                UserLogoUi::new(logo_id)
                    .source(SharedString::from(author.more.logo.clone()))
                    .name(SharedString::from(author.name.clone()))
                    .user_id(user_id.clone())
                    .size(40.0),
            )
            .child(
                vstack(2.0)
                    .items_start()
                    .min_w(px(0.0))
                    .child(
                        hstack(4.0)
                            .min_w(px(0.0))
                            .child(
                                MainText::new(author.name.clone())
                                    .px(15.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(name_color),
                            )
                            .when(author.more.is_official, |line| line.child(OfficialBadge::new().size(px(14.0))))
                            .when(author.more.is_premium, |line| {
                                line.child(
                                    PremiumBadge::new()
                                        .user_id(user_id)
                                        .emoji(author.more.premium_badge_emoji.clone().map(SharedString::from))
                                        .size(14.0),
                                )
                            }),
                    )
                    .when(!author.tag.is_empty(), |column| {
                        column.child(
                            MainText::new(format!("@{}", author.tag)).px(12.0).color(theme.text.secondary),
                        )
                    }),
            )
            .child(spacer())
    }
}
