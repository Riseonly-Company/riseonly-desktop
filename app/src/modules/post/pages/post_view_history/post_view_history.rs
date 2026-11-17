use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{AnyView, App, ElementId, Entity, Pixels, Point, SharedString, StyleRefinement, Subscription, Task, Window};
use rise_core::foundation::foundation_character::int_from_string;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::components::gift_shop_shelf_grid::GiftShopShelfRows;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::search::pages::global_search::global_search::GridPostCell;
use crate::modules::story::components::stories_grid::stories_grid_view::{container_width_reader, use_container_width};
use crate::modules::user::components::profile_content::pages::grid_reposts::grid_reposts::lazy_grid_mounted_range;

const GRID_COLUMNS: i64 = 3;
const GRID_SPACING: f32 = 2.0;
const GRID_BOTTOM_PADDING: f32 = 80.0;
const SKELETON_ROWS: i64 = 5;
const LOAD_MORE_REMAINING: i64 = 8;

/// navigator the grid cells push onto, read at tap time
/// the page sets it each render
type SharedNavStore = Rc<RefCell<Option<Entity<NavigationStore>>>>;

struct PostViewHistoryPageState {
    posts: Rc<Vec<GetPostFeedResponse>>,
    slice_status: SaiWsSliceStatus,
    error_text: String,
    nav_store: SharedNavStore,
    grid: Entity<PostViewHistoryGridView>,
    _subscriptions: Vec<Subscription>,
}

impl PostViewHistoryPageState {
    fn new(cx: &mut Context<Self>) -> Self {
        let interactions = PostInteractionsStore::shared(cx);
        let services = PostServicesStore::shared(cx);
        let actions = PostActionsStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        let posts = Rc::new(PostViewHistory::displayed_posts(cx));
        let nav_store: SharedNavStore = Rc::new(RefCell::new(None));
        let grid_posts = posts.clone();
        let grid_nav_store = nav_store.clone();
        let grid = cx.new(|cx| PostViewHistoryGridView::new(grid_posts, grid_nav_store, cx));
        Self {
            posts,
            slice_status: PostViewHistory::slice_status(cx),
            error_text: PostViewHistory::error_text(cx),
            nav_store,
            grid,
            _subscriptions: vec![
                cx.observe(&interactions, |state, _, cx| state.refresh(cx)),
                cx.observe(&services, |state, _, cx| state.refresh(cx)),
                cx.observe(&actions, |state, _, cx| state.refresh(cx)),
                cx.observe(&cache_revisions, |state, _, cx| state.refresh(cx)),
                cx.observe(&pending_requests, |state, _, cx| state.refresh(cx)),
            ],
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let posts = PostViewHistory::displayed_posts(cx);
        if *self.posts != posts {
            self.posts = Rc::new(posts);
            let posts = self.posts.clone();
            self.grid.update(cx, |grid, cx| grid.set_posts(posts, cx));
        }
        self.slice_status = PostViewHistory::slice_status(cx);
        self.error_text = PostViewHistory::error_text(cx);
        cx.notify();
    }
}

#[derive(IntoElement, Default)]
pub struct PostViewHistory;

impl PostViewHistory {
    pub fn new() -> Self {
        Self
    }

    fn displayed_posts(cx: &mut App) -> Vec<GetPostFeedResponse> {
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.post_view_history_items(cx))
    }

    fn slice_status(cx: &mut App) -> SaiWsSliceStatus {
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.post_view_history_status(cx))
    }

    fn error_text(cx: &mut App) -> String {
        PostInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.post_view_history_error_text(cx))
            .unwrap_or_else(|| tr("error_fetching_data"))
    }

    fn show_clear_confirm(cx: &mut App) {
        NativeConfirmAlert::present_delete(
            &tr("view_history_clear_confirm"),
            "",
            Some(tr("view_history_clear")),
            None,
            false,
            None,
            None,
            |cx| PostInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_post_view_history(cx)),
            cx,
        );
    }

    fn post_hold_actions(post: &GetPostFeedResponse, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let auth = AuthServicesStore::shared(cx).read(cx);
        let me = auth.profile().map(|profile| profile.id.as_str()).unwrap_or_default();
        let mine = post.author.id.as_deref() == Some(me) || post.author_id == me;
        let acts_as_author = mine || auth.viewer_acts_as_author();
        let mut items = vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
        ];
        if acts_as_author {
            items.push(HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true));
        }
        if !mine {
            items.push(
                HoldContextMenuAction::new("report", tr("ctx_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            );
        }
        items
    }

    fn open_post(post: &GetPostFeedResponse, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            let route = RootRoute::PostDetail { post_id: Some(post.id.clone()), origin: None };
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn handle_post_action(
        post: &GetPostFeedResponse,
        id: &str,
        nav_store: Option<&Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        match id {
            "open" => Self::open_post(post, nav_store, cx),
            "copy" => {
                let link = RiseonlyPublicLink::Post { id: post.id.clone() };
                if let Some(url) = link.web_url(AppEnvironment::current()) {
                    UIPasteboard::general().set_string(url.absolute_string(), cx);
                }
                Notifier::copied(cx);
            }
            "share" => {
                let post = post.clone();
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_post(post, cx));
            }
            "delete" => {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.delete_post_from_view_history(post, cx));
            }
            "report" => {
                if let Some(post_id) = int_from_string(&post.id) {
                    ReportSheetStore::shared(cx)
                        .update(cx, |sheet, cx| sheet.present(ReportTarget::Post(post_id), None, None, cx));
                }
            }
            _ => {}
        }
    }
}

impl RenderOnce for PostViewHistory {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("post-view-history-state", cx, |_, cx| PostViewHistoryPageState::new(cx));
        let nav_store = env::<NavigationStoreKey>(cx);
        let (grid, slice_status, error_text, is_empty) = {
            let state = state.read(cx);
            *state.nav_store.borrow_mut() = nav_store.clone();
            (state.grid.clone(), state.slice_status, state.error_text.clone(), state.posts.is_empty())
        };

        let back_store = nav_store.clone();
        let mut wrapper = SettingsWrapperUi::new(tr("view_history_page_title"), move |_, cx| {
            if let Some(nav_store) = &back_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        });
        if !is_empty {
            wrapper = wrapper.right_jsx(PageHeaderIconButton::new("post-view-history-clear", "trash", |_, cx| {
                Self::show_clear_confirm(cx);
            }));
        }
        div()
            .size_full()
            .child(
                wrapper.content(
                    SaiDataRender::new(slice_status, move |_, _| {
                        AnyView::from(grid).cached(StyleRefinement::default().size_full())
                    })
                    .is_empty(is_empty)
                    .no_data_text(tr("view_history_empty"))
                    .error_text(error_text)
                    .message_height_percent(40.0)
                    .no_data_height_percent(25.0)
                    .sai_data_render_loading(|_, cx| {
                        scroll_view("post-view-history-skeleton")
                            .scroll_disabled(true)
                            .shows_indicators(false)
                            .list_shell_scroll_under_chrome(cx)
                            .child(PostGridSkeleton::default().rows(SKELETON_ROWS))
                    }),
                ),
            )
            .on_appear("post-view-history-appear", |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_post_view_history_if_needed(cx));
            })
    }
}

/// one square cell of the grid as a cached view
/// a wheel step redraws the grid and every mounted cell rebuilt its hold menu and tile each frame
/// now it replays its last frame moved
struct PostViewHistoryCellView {
    post: Rc<GetPostFeedResponse>,
    side: f32,
    nav_store: SharedNavStore,
}

impl Render for PostViewHistoryCellView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let post = self.post.clone();
        let side = self.side;
        let actions = PostViewHistory::post_hold_actions(&post, cx);

        let content_post = post.clone();
        let action_post = post.clone();
        let action_store = self.nav_store.clone();
        let tap_post = post.clone();
        let tap_store = self.nav_store.clone();
        let menu = HoldContextMenuContainer::new(
            SharedString::from(format!("post-view-history-hold-{}", post.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| {
                div().size(px(side)).child(GridPostCell::new((*content_post).clone())).into_any_element()
            }),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            let nav_store = action_store.borrow().clone();
            PostViewHistory::handle_post_action(&action_post, &id, nav_store.as_ref(), cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            let nav_store = tap_store.borrow().clone();
            PostViewHistory::open_post(&tap_post, nav_store.as_ref(), cx);
        }));

        div().size_full().child(menu)
    }
}

/// grid of three flexible columns
/// square cells placed by index, only those near the screen mounted
/// own view so a wheel step redraws the grid and replays cached cells
struct PostViewHistoryGridView {
    posts: Rc<Vec<GetPostFeedResponse>>,
    nav_store: SharedNavStore,
    cells: HashMap<String, Entity<PostViewHistoryCellView>>,
    scroll_locked: bool,
    _menu_lock: Subscription,
}

impl PostViewHistoryGridView {
    fn new(posts: Rc<Vec<GetPostFeedResponse>>, nav_store: SharedNavStore, cx: &mut Context<Self>) -> Self {
        let menu = HoldContextMenuPresenter::shared(cx);
        let scroll_locked = menu.read(cx).active_request().is_some();
        let menu_lock = cx.observe(&menu, |grid, menu, cx| {
            let locked = menu.read(cx).active_request().is_some();
            if grid.scroll_locked != locked {
                grid.scroll_locked = locked;
                cx.notify();
            }
        });
        Self { posts, nav_store, cells: HashMap::new(), scroll_locked, _menu_lock: menu_lock }
    }

    fn set_posts(&mut self, posts: Rc<Vec<GetPostFeedResponse>>, cx: &mut Context<Self>) {
        let ids: HashSet<&str> = posts.iter().map(|post| post.id.as_str()).collect();
        self.cells.retain(|id, _| ids.contains(id.as_str()));
        for post in posts.iter() {
            if let Some(cell) = self.cells.get(&post.id) {
                cell.update(cx, |cell, cx| {
                    if *cell.post != *post {
                        cell.post = Rc::new(post.clone());
                        cx.notify();
                    }
                });
            }
        }
        self.posts = posts;
        cx.notify();
    }

    fn cell(&mut self, index: usize, side: f32, cx: &mut Context<Self>) -> AnyView {
        let post = &self.posts[index];
        if let Some(cell) = self.cells.get(&post.id) {
            if cell.read(cx).side != side {
                cell.update(cx, |cell, _| cell.side = side);
            }
            return AnyView::from(cell.clone());
        }
        let nav_store = self.nav_store.clone();
        let post = Rc::new(post.clone());
        let id = post.id.clone();
        let cell = cx.new(|_| PostViewHistoryCellView { post, side, nav_store });
        self.cells.insert(id, cell.clone());
        AnyView::from(cell)
    }
}

impl Render for PostViewHistoryGridView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let grid_id: ElementId = "post-view-history-grid".into();
        let width_state = use_container_width("post-view-history-width".into(), window, cx);
        let width = width_state.read(cx).max(0.0);
        let cell_side = ((width - GRID_SPACING * (GRID_COLUMNS - 1) as f32) / GRID_COLUMNS as f32).max(0.0);
        let item_count = self.posts.len() as i64;
        let rows = GiftShopShelfRows::count(item_count, GRID_COLUMNS);
        let height = (rows as f32 * (cell_side + GRID_SPACING) - GRID_SPACING).max(0.0);
        let (range, window_probe) =
            lazy_grid_mounted_range(&grid_id, item_count, GRID_COLUMNS, GRID_SPACING, cell_side, window, cx);

        let step = cell_side + GRID_SPACING;
        let mut cells: Vec<AnyElement> = Vec::with_capacity((range.end - range.start).max(0) as usize);
        for index in range.start..range.end {
            let (row, column) = (index / GRID_COLUMNS, index % GRID_COLUMNS);
            let view = self.cell(index as usize, cell_side, cx);
            let post_id = self.posts[index as usize].id.clone();
            cells.push(
                div()
                    .absolute()
                    .left(px(column as f32 * step))
                    .top(px(row as f32 * step))
                    .size(px(cell_side))
                    .child(view.cached(StyleRefinement::default().size(px(cell_side))).replay_when_moved())
                    .on_appear(SharedString::from(format!("post-view-history-appear-{post_id}")), move |_, cx| {
                        if item_count - index <= LOAD_MORE_REMAINING {
                            PostInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.load_more_post_view_history(cx));
                        }
                    })
                    .into_any_element(),
            );
        }

        scroll_view("post-view-history-scroll")
            .scroll_disabled(self.scroll_locked)
            .list_shell_scroll_under_chrome(cx)
            .refreshable(|_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_post_view_history(cx));
                Task::ready(())
            })
            .child(
                div().w_full().pb(px(GRID_BOTTOM_PADDING)).child(
                    div()
                        .relative()
                        .w_full()
                        .h(px(height))
                        .child(container_width_reader("post-view-history-width-reader".into(), width_state))
                        .children(cells)
                        .child(div().absolute().inset_0().child(window_probe)),
                ),
            )
    }
}
