use std::rc::Rc;

use gpui::{App, AsyncWindowContext, ElementId, Entity, Pixels, Point, SharedString, Subscription, Window};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{FormatArg, tr, tr_args, tr_plural};
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::MainText;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiWsSliceStatus};
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::HoldContextMenuContainer;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, ListShellUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::components::gift_shop_shelf_grid::GiftShopShelfRows;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::search::pages::global_search::global_search::GridPostCell;
use crate::modules::search::stores::search_actions::search_actions::SearchActionsStore;
use crate::modules::search::stores::search_interactions::search_interactions::SearchInteractionsStore;
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;
use crate::modules::story::components::stories_grid::stories_grid_view::{container_width_reader, use_container_width};
use crate::modules::user::components::profile_content::pages::grid_reposts::grid_reposts::lazy_grid_mounted_range;
use crate::modules::user::shared::rise_blocked_content::RiseBlockedContent;

const GRID_COLUMNS: i64 = 3;
const GRID_SPACING: f32 = 2.0;
const GRID_BOTTOM_PADDING: f32 = 80.0;
const SKELETON_ROWS: i64 = 5;
const LOAD_MORE_THRESHOLD: usize = 8;

struct HashtagResultsPageState {
    _subscriptions: Vec<Subscription>,
}

impl HashtagResultsPageState {
    fn new(cx: &mut Context<Self>) -> Self {
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let search_store = SearchServicesStore::shared(cx);
        let search_actions = SearchActionsStore::shared(cx);
        let post_services = PostServicesStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        Self {
            _subscriptions: vec![
                cx.observe(&hold_context_menu, |_, _, cx| cx.notify()),
                cx.observe(&search_store, |_, _, cx| cx.notify()),
                cx.observe(&search_actions, |_, _, cx| cx.notify()),
                cx.observe(&post_services, |_, _, cx| cx.notify()),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
                cx.observe(&pending_requests, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct HashtagResults {
    pub tag: String,
}

impl HashtagResults {
    pub fn new(tag: impl Into<String>) -> Self {
        Self { tag: tag.into() }
    }

    fn clean_tag(&self) -> String {
        trimming_whitespaces_and_newlines(&self.tag).trim_matches('#').to_owned()
    }

    fn is_same_tag(clean_tag: &str, cx: &mut App) -> bool {
        SearchServicesStore::shared(cx)
            .read(cx)
            .hashtag_current_tag
            .as_ref()
            .is_some_and(|current| current.to_lowercase() == clean_tag.to_lowercase())
    }

    fn displayed_posts(clean_tag: &str, cx: &mut App) -> Vec<GetPostFeedResponse> {
        if !Self::is_same_tag(clean_tag, cx) {
            return Vec::new();
        }
        let items = SearchServicesStore::shared(cx).update(cx, |store, cx| store.hashtag_post_items(cx));
        let posts = PostServicesStore::shared(cx);
        let posts = posts.read(cx);
        items
            .into_iter()
            .filter(|post| !posts.is_post_hidden(&post.id) && !RiseBlockedContent::hides_post(post, cx))
            .collect()
    }

    fn slice_status(clean_tag: &str, cx: &mut App) -> SaiWsSliceStatus {
        if !Self::is_same_tag(clean_tag, cx) {
            return SaiWsSliceStatus::Pending;
        }
        SearchActionsStore::shared(cx).read(cx).search_posts_by_hashtag_sai.status()
    }

    fn total_count(clean_tag: &str, cx: &mut App) -> Option<i64> {
        if !Self::is_same_tag(clean_tag, cx) {
            return None;
        }
        SearchServicesStore::shared(cx).update(cx, |store, cx| store.hashtag_post_total(cx))
    }

    fn post_hold_actions(post: &GetPostFeedResponse, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let auth = AuthServicesStore::shared(cx);
        let me = auth.read(cx).profile().map(|profile| profile.id.as_str()).unwrap_or_default();
        let mine = post.author.id.as_deref() == Some(me) || post.author_id == me;
        let mut items = vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
        ];
        if mine {
            items.push(HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true));
        } else {
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
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.delete_own_post(post, None, cx));
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

impl RenderOnce for HashtagResults {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let _state = window.use_keyed_state("hashtag-results-state", cx, |_, cx| HashtagResultsPageState::new(cx));
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let clean_tag = self.clean_tag();
        let displayed_posts = Self::displayed_posts(&clean_tag, cx);
        let slice_status = Self::slice_status(&clean_tag, cx);
        let total_count = Self::total_count(&clean_tag, cx);
        let is_same_tag = Self::is_same_tag(&clean_tag, cx);
        let search_store = SearchServicesStore::shared(cx);
        let relative_id = search_store.update(cx, |store, cx| store.hashtag_post_relative_id(cx));
        let has_more = search_store.update(cx, |store, cx| store.hashtag_post_has_more(cx));
        let error_text = search_store
            .read(cx)
            .hashtag_post_last_error()
            .map(str::to_owned)
            .unwrap_or_else(|| tr("error_fetching_data"));
        let is_empty = displayed_posts.is_empty();

        let task_tag = clean_tag.clone();
        use_task(
            "hashtag-results-relative-id",
            relative_id.clone(),
            window,
            cx,
            async move |cx: &mut AsyncWindowContext| {
                let _ = cx.update(|_, cx| {
                    let has_more = SearchServicesStore::shared(cx).update(cx, |store, cx| store.hashtag_post_has_more(cx));
                    if Self::is_same_tag(&task_tag, cx) && Self::displayed_posts(&task_tag, cx).is_empty() && has_more {
                        SearchInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_hashtag(&task_tag, cx));
                    }
                });
            },
        );

        let back_store = nav_store.clone();
        let mut shell = ListShellUi::new()
            .title(SharedString::from(format!("#{clean_tag}")))
            .on_back(move |_, cx| {
                if let Some(nav_store) = &back_store {
                    nav_store.update(cx, |store, cx| store.pop(cx));
                }
            })
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()));
        if let Some(total) = total_count.filter(|total| *total > 0) {
            shell = shell.below_header(
                hstack(None)
                    .w_full()
                    .px(px(16.0))
                    .py(px(8.0))
                    .child(MainText::new(tr_plural("hashtag_publications", total)).px(13.0).color(theme.text.secondary))
                    .child(spacer()),
            );
        }

        let no_data_text = tr_args("hashtag_no_posts", &[FormatArg::Text(clean_tag.as_str().into())]);
        let grid_tag = clean_tag.clone();
        let appear_tag = clean_tag;
        div()
            .size_full()
            .child(
                shell.content(
                    vstack(0.0).size_full().child(
                        SaiDataRender::new(slice_status, move |_, _| HashtagResultsGrid {
                            posts: Rc::new(displayed_posts),
                            clean_tag: grid_tag,
                            relative_id,
                            has_more: is_same_tag && has_more,
                            nav_store,
                        })
                        .is_empty(is_empty)
                        .no_data_text(no_data_text)
                        .error_text(error_text)
                        .message_height_percent(40.0)
                        .no_data_height_percent(25.0)
                        .sai_data_render_loading(|_, cx| {
                            scroll_view("hashtag-results-skeleton")
                                .scroll_disabled(true)
                                .shows_indicators(false)
                                .list_shell_scroll_under_chrome(cx)
                                .child(PostGridSkeleton::default().rows(SKELETON_ROWS))
                        }),
                    ),
                ),
            )
            .on_appear("hashtag-results-appear", move |_, cx| {
                SearchInteractionsStore::shared(cx).update(cx, |store, cx| store.load_hashtag_initial(&appear_tag, cx));
            })
    }
}

#[derive(IntoElement)]
struct HashtagResultsGrid {
    posts: Rc<Vec<GetPostFeedResponse>>,
    clean_tag: String,
    relative_id: Option<String>,
    has_more: bool,
    nav_store: Option<Entity<NavigationStore>>,
}

impl HashtagResultsGrid {
    fn cell(&self, index: usize, cx: &mut App) -> AnyElement {
        let post = Rc::new(self.posts[index].clone());
        let actions = HashtagResults::post_hold_actions(&post, cx);

        let content_post = post.clone();
        let action_post = post.clone();
        let action_store = self.nav_store.clone();
        let tap_post = post.clone();
        let tap_store = self.nav_store.clone();
        let load_tag = self.clean_tag.clone();
        let options = SaiPaginationPrefetchOptions::new(
            index,
            self.posts.len(),
            SearchServicesStore::hashtag_posts_cache_key(&self.clean_tag),
            self.relative_id.clone(),
            self.has_more,
        )
        .threshold(LOAD_MORE_THRESHOLD);
        HoldContextMenuContainer::new(
            SharedString::from(format!("hashtag-results-hold-{}", post.id)),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| actions.clone()),
            Rc::new(move |_: &mut Window, _: &mut App| GridPostCell::new((*content_post).clone()).into_any_element()),
        )
        .show_emoji_list(false)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            HashtagResults::handle_post_action(&action_post, &id, action_store.as_ref(), cx);
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            HashtagResults::open_post(&tap_post, tap_store.as_ref(), cx);
        }))
        .sai_pagination_prefetch(
            SharedString::from(format!("hashtag-results-prefetch-{}", post.id)),
            options,
            move |_: &mut Window, cx: &mut App| {
                SearchInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_hashtag(&load_tag, cx));
            },
        )
        .into_any_element()
    }
}

impl RenderOnce for HashtagResultsGrid {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let grid_id: ElementId = "hashtag-results-grid".into();
        let width_state = use_container_width("hashtag-results-width".into(), window, cx);
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
            cells.push(
                div()
                    .absolute()
                    .left(px(column as f32 * step))
                    .top(px(row as f32 * step))
                    .size(px(cell_side))
                    .child(self.cell(index as usize, cx))
                    .into_any_element(),
            );
        }

        scroll_view("hashtag-results-scroll")
            .scroll_disabled(HoldContextMenuPresenter::has_active_request(cx))
            .list_shell_scroll_under_chrome(cx)
            .child(
                div().w_full().pb(px(GRID_BOTTOM_PADDING)).child(
                    div()
                        .relative()
                        .w_full()
                        .h(px(height))
                        .child(container_width_reader("hashtag-results-width-reader".into(), width_state))
                        .children(cells)
                        .child(div().absolute().inset_0().child(window_probe)),
                ),
            )
    }
}
