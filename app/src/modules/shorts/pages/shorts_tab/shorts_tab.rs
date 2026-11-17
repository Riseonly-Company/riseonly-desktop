use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{AnyView, App, Context, ElementId, Entity, StyleRefinement, FontWeight, Pixels, PromptButton, PromptLevel, SharedString, Size, Subscription, Window, black, white};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_ui::button_ui::ButtonUi;
use rise_ui::liquid_chrome::{LiquidChromeModifiers, LiquidGlassContrast};
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::status_bar_style_anchor::{StatusBarStyleAnchor, UserInterfaceStyle};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::modifiers::{ModifierExt, ViewModifier};
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::{IconUi, MainText, TextAlignT};

use crate::app::aside::aside_navigator;
use crate::app::router::main_tab_chrome::main_tab_bar_underlap;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{CreatePostIntent, MainTab, RootRoute};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::widgets::navigations::bottom_chrome_insets::{BottomChromeInsets, BottomChromeOwner, BottomChromePolicy};
use crate::core::widgets::wrappers::list_shell_ui::{ListShellFloatingTopInsetKey, ListShellUi};
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementSurface;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::comment::widgets::comments_sheet_progress_fade::CommentsSheetProgressFade;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_refresh_control::PostListRefreshControl;
use crate::modules::shorts::components::shorts_feed_cell::{ShortsFeedCell, ShortsReturnRewind};
use crate::modules::shorts::components::shorts_media_prefetch::ShortsMediaPrefetchController;
use crate::modules::shorts::pages::shorts_vertical_pager::{ShortsVerticalPager, ShortsVerticalPagerController};
use crate::modules::shorts::stores::shorts::shorts_actions::shorts_actions::ShortsActionsStore;
use crate::modules::shorts::stores::shorts::shorts_interactions::shorts_interactions::ShortsInteractionsStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

const PREFETCH_THRESHOLD: usize = 8;
const MINIMUM_PAGE_HEIGHT: f32 = 320.0;
const IMPRESSION_VISIBLE_MS: i64 = 500;

struct ShortsTabState {
    active_post_id: Option<String>,
    return_rewind: ShortsReturnRewind,
    show_publish_error: bool,
    feed_stalled_offline: bool,
    playback_owner: String,
    pull_refresh_armed: bool,
    pull_refreshing: bool,
    media_prefetch: ShortsMediaPrefetchController,
    pager: Entity<ShortsVerticalPagerController>,
    items: Rc<Vec<Rc<GetPostFeedResponse>>>,
    feed_relative_id: Option<String>,
    feed_has_more: bool,
    size: Size<Pixels>,
    last_chrome_lift: Option<f32>,
    feed_cells: HashMap<String, (Entity<ShortsTabFeedCellView>, u64)>,
    feed_cell_pass: u64,
    nav_store: Option<Entity<NavigationStore>>,
    nav_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl ShortsTabState {
    fn new(cx: &mut Context<Self>) -> Self {
        let services = ShortsServicesStore::shared(cx);
        let actions = ShortsActionsStore::shared(cx);
        let post_services = PostServicesStore::shared(cx);
        let presentation = RisePostContentPresentation::shared(cx);
        let socket = SaiWs::manager(cx);
        let mut state = Self {
            active_post_id: None,
            return_rewind: ShortsReturnRewind::new(),
            show_publish_error: false,
            feed_stalled_offline: false,
            playback_owner: Uuid::new_v4().uuid_string(),
            pull_refresh_armed: true,
            pull_refreshing: false,
            media_prefetch: ShortsMediaPrefetchController::new(&Uuid::new_v4().uuid_string()),
            pager: cx.new(|_| ShortsVerticalPagerController::new(None)),
            items: Rc::new(Vec::new()),
            feed_relative_id: None,
            feed_has_more: false,
            size: Size::default(),
            last_chrome_lift: None,
            feed_cells: HashMap::new(),
            feed_cell_pass: 0,
            nav_store: None,
            nav_subscription: None,
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&post_services, |_, _, cx| cx.notify()),
                cx.observe(&presentation, |_, _, cx| cx.notify()),
                cx.observe(&socket, |_, _, cx| cx.notify()),
            ],
        };
        state.refresh(cx);
        state
    }

    fn bind_navigation(&mut self, nav_store: Option<Entity<NavigationStore>>, cx: &mut Context<Self>) {
        if self.nav_store != nav_store {
            self.nav_subscription = nav_store.as_ref().map(|nav_store| cx.observe(nav_store, |_, _, cx| cx.notify()));
            self.nav_store = nav_store;
        }
    }

    fn refresh_if_settled(&mut self, cx: &mut Context<Self>) {
        if !self.pager.read(cx).is_moving() {
            self.refresh(cx);
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let services = ShortsServicesStore::shared(cx);
        let (items, relative_id, has_more) =
            services.update(cx, |service, cx| (service.items(cx), service.relative_id(cx), service.has_more(cx)));
        let unchanged = self.items.len() == items.len() && self.items.iter().zip(&items).all(|(old, new)| **old == *new);
        if !unchanged {
            let previous: HashMap<&str, &Rc<GetPostFeedResponse>> =
                self.items.iter().map(|post| (post.id.as_str(), post)).collect();
            let next: Vec<Rc<GetPostFeedResponse>> = items
                .into_iter()
                .map(|post| match previous.get(post.id.as_str()) {
                    Some(old) if ***old == post => Rc::clone(old),
                    _ => Rc::new(post),
                })
                .collect();
            self.items = Rc::new(next);
        }
        self.feed_relative_id = relative_id;
        self.feed_has_more = has_more;
    }

    /// visibility comes from navigation not the view lifecycle,
    /// since kept mounted tabs and lazy rows never report disappear reliably
    fn is_screen_visible(&self, cx: &App) -> bool {
        self.nav_store.as_ref().is_some_and(|nav_store| {
            let nav_store = nav_store.read(cx);
            nav_store.current_tab == MainTab::Shorts && nav_store.path.is_empty()
        })
    }

    fn riffs_chrome_lift(&self, page_bottom_in_window: f32, page_height: f32, bar_top: Option<f32>, tab_underlap: f32, cx: &App) -> f32 {
        let fallback = self.last_chrome_lift.unwrap_or(tab_underlap);
        let Some(bar_top) = bar_top.filter(|_| self.is_screen_visible(cx)) else {
            return fallback;
        };
        let lift = page_bottom_in_window - bar_top;
        if lift <= 0.0 || lift > BottomChromePolicy::max_clearance(page_height) {
            return fallback;
        }
        lift
    }

    fn sync_full_screen_playback_ownership(&mut self, cx: &mut Context<Self>) {
        let owner = self.playback_owner.clone();
        let visible = self.is_screen_visible(cx);
        ShortsServicesStore::shared(cx).update(cx, |service, cx| {
            if visible {
                service.begin_full_screen_playback(owner, cx);
            } else {
                service.end_full_screen_playback(owner, cx);
            }
        });
    }

    fn warm_neighbours(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if !self.is_screen_visible(cx) {
            return;
        }
        let posts: Vec<GetPostFeedResponse> = self.items.iter().map(|post| (**post).clone()).collect();
        self.media_prefetch.update("shorts-tab", post_id, &posts, cx);
    }

    fn fire_impression(&self, post_id: &str, cx: &mut Context<Self>) {
        let Some(post) = self.items.iter().find(|post| post.id == post_id).cloned() else {
            return;
        };
        let author_id = post.author.id.clone().unwrap_or_else(|| post.author_id.clone());
        EngagementInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.impression(
                &post.rise_view_target_post_id(),
                &author_id,
                EngagementSurface::Shorts,
                -1,
                true,
                IMPRESSION_VISIBLE_MS,
                cx,
            );
        });
    }

    fn page_aligned(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if self.active_post_id.as_deref() == Some(post_id) {
            return;
        }
        let order: Vec<String> = self.items.iter().map(|post| post.id.clone()).collect();
        self.return_rewind.move_to(self.active_post_id.as_deref(), post_id, &order);
        self.active_post_id = Some(post_id.to_owned());
        if self.is_screen_visible(cx) {
            self.fire_impression(post_id, cx);
        }
        self.warm_neighbours(post_id, cx);
        cx.notify();
    }

    fn visibility_changed(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.sync_full_screen_playback_ownership(cx);
        match self.active_post_id.clone() {
            Some(active_post_id) if visible => {
                self.fire_impression(&active_post_id, cx);
                self.warm_neighbours(&active_post_id, cx);
            }
            _ => self.media_prefetch.cancel_all(cx),
        }
    }

    fn first_item_changed(&mut self, first_id: Option<&str>, cx: &mut Context<Self>) {
        self.return_rewind.forget_returns();
        if self.active_post_id.is_none()
            && let Some(first_id) = first_id
        {
            self.active_post_id = Some(first_id.to_owned());
            if self.is_screen_visible(cx) {
                self.fire_impression(first_id, cx);
            }
            cx.notify();
        }
        if let Some(active_post_id) = self.active_post_id.clone() {
            self.warm_neighbours(&active_post_id, cx);
        }
    }

    fn track_pull_refresh(&mut self, top: Option<f32>, is_pending: bool, cx: &mut Context<Self>) {
        let Some(top) = top else {
            return;
        };
        if top > PostListRefreshControl::TRIGGER_DISTANCE {
            if !self.pull_refresh_armed || is_pending {
                return;
            }
            self.pull_refresh_armed = false;
            self.pull_refreshing = true;
            cx.notify();
            ShortsInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh(cx));
        } else if top <= 1.0 && !self.pull_refresh_armed {
            self.pull_refresh_armed = true;
        }
    }

    fn scroll_to_feed_top(&mut self, cx: &mut Context<Self>) {
        let services = ShortsServicesStore::shared(cx);
        let Some(first_id) = services.update(cx, |service, cx| service.items(cx).first().map(|post| post.id.clone()))
        else {
            return;
        };
        self.pager.update(cx, |pager, cx| pager.scroll_to_animated(&first_id, 0.3, cx));
    }

    fn item_count_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(active_post_id) = self.active_post_id.clone() {
            self.warm_neighbours(&active_post_id, cx);
        }
    }

    fn sync_docked_comments(&mut self, post_id: Option<&str>, cx: &mut Context<Self>) {
        let comments = CommentInteractionsStore::shared(cx);
        let Some(post_id) = post_id else {
            if comments.read(cx).comments_sheet_visible {
                comments.update(cx, |store, cx| store.close_comments_sheet(cx));
            }
            return;
        };
        let Some(post) = self.items.iter().find(|post| post.id == post_id).cloned() else {
            return;
        };
        if post.is_temp {
            if comments.read(cx).comments_sheet_visible {
                comments.update(cx, |store, cx| store.close_comments_sheet(cx));
            }
            return;
        }
        let shows_post = PostServicesStore::shared(cx).read(cx).selected_post.as_ref().is_some_and(|selected| selected.id == post_id);
        if comments.read(cx).comments_sheet_visible && shows_post {
            return;
        }
        comments.update(cx, |store, cx| store.open_comments_sheet(&post, None, false, false, cx));
    }

    fn disappeared(&mut self, cx: &mut Context<Self>) {
        let comments = CommentInteractionsStore::shared(cx);
        let docked_here = comments.read(cx).comments_sheet_visible
            && aside_navigator::docks_comments(cx)
            && PostServicesStore::shared(cx)
                .read(cx)
                .selected_post
                .as_ref()
                .is_some_and(|selected| self.items.iter().any(|post| post.id == selected.id));
        if docked_here {
            comments.update(cx, |store, cx| store.close_comments_sheet(cx));
        }
        let owner = self.playback_owner.clone();
        ShortsServicesStore::shared(cx).update(cx, |service, cx| service.end_full_screen_playback(owner, cx));
        self.media_prefetch.cancel_all(cx);
    }

    fn begin_feed_cell_pass(&mut self) -> u64 {
        self.feed_cell_pass += 1;
        let pass = self.feed_cell_pass;
        self.feed_cells.retain(|_, (_, placed)| *placed + 1 >= pass);
        pass
    }

    fn feed_cell(&mut self, post_id: &str, pass: u64, cx: &mut Context<Self>) -> Entity<ShortsTabFeedCellView> {
        if let Some((cell, placed)) = self.feed_cells.get_mut(post_id) {
            *placed = pass;
            return cell.clone();
        }
        let cell = cx.new(|_| ShortsTabFeedCellView { props: None });
        self.feed_cells.insert(post_id.to_owned(), (cell.clone(), pass));
        cell
    }
}

/// what one riff page draws from, posts compared by pointer so a changed post is a new Rc
#[derive(Clone)]
struct ShortsTabFeedCellProps {
    post: Rc<GetPostFeedResponse>,
    is_active: bool,
    bottom_inset: f32,
    progress_floor: f32,
    rewind_token: i64,
    index: usize,
    item_count: usize,
    cache_key: SharedString,
    relative_id: Option<String>,
    has_more: bool,
}

impl ShortsTabFeedCellProps {
    fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.post, &other.post)
            && self.is_active == other.is_active
            && self.bottom_inset == other.bottom_inset
            && self.progress_floor == other.progress_floor
            && self.rewind_token == other.rewind_token
            && self.index == other.index
            && self.item_count == other.item_count
            && self.cache_key == other.cache_key
            && self.relative_id == other.relative_id
            && self.has_more == other.has_more
    }
}

struct ShortsTabFeedCellView {
    props: Option<ShortsTabFeedCellProps>,
}

impl ShortsTabFeedCellView {
    fn set_props(&mut self, props: ShortsTabFeedCellProps, cx: &mut Context<Self>) {
        if self.props.as_ref().is_some_and(|current| current.same(&props)) {
            return;
        }
        self.props = Some(props);
        cx.notify();
    }
}

impl Render for ShortsTabFeedCellView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(props) = self.props.clone() else {
            return div().into_any_element();
        };
        let prefetch_id = ElementId::from(SharedString::from(format!("shorts-tab-prefetch-{}", props.post.id)));
        ShortsFeedCell::new(props.post.clone(), props.is_active, props.bottom_inset)
            .progress_floor(Some(props.progress_floor))
            .rewind_token(props.rewind_token)
            .sai_pagination_prefetch(
                prefetch_id,
                SaiPaginationPrefetchOptions::new(
                    props.index,
                    props.item_count,
                    props.cache_key.clone(),
                    props.relative_id.clone(),
                    props.has_more,
                )
                .threshold(PREFETCH_THRESHOLD),
                |_, cx| {
                    ShortsInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more(cx));
                },
            )
            .into_any_element()
    }
}

#[derive(IntoElement, Default)]
pub struct ShortsTab;

impl ShortsTab {
    pub fn new() -> Self {
        Self
    }

    fn publish_error_alert(state: &Entity<ShortsTabState>, error: Option<String>, window: &mut Window, cx: &mut App) {
        let state = state.clone();
        on_change("shorts-tab-publish-error", error, false, window, cx, move |_, error, window, cx| {
            state.update(cx, |state, cx| {
                state.show_publish_error = error.is_some();
                cx.notify();
            });
            let Some(error) = error else {
                return;
            };
            let interactions = PostInteractionsStore::shared(cx);
            let can_retry = interactions.update(cx, |store, cx| store.can_retry_publish(cx));
            let mut buttons = Vec::new();
            if can_retry {
                buttons.push(PromptButton::new(tr("retry")));
            }
            buttons.push(PromptButton::cancel(tr("ok")));
            let answer = window.prompt(
                PromptLevel::Info,
                &tr("create_post_error_text"),
                Some(error.as_str()),
                &buttons,
                cx,
            );
            cx.spawn(async move |cx| {
                let answered = answer.await.ok();
                let retries = can_retry && answered == Some(0);
                state.update(cx, |state, cx| {
                    state.show_publish_error = false;
                    cx.notify();
                });
                interactions.update(cx, |store, cx| {
                    if retries {
                        store.retry_publish(cx);
                    } else {
                        store.dismiss_publish_error(cx);
                    }
                });
            })
            .detach();
        });
    }

    fn observe_changes(
        state: &Entity<ShortsTabState>,
        visible: bool,
        is_pending: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let socket_dropped = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.is_socket_dropped(cx));
        let pending_id = ShortsServicesStore::shared(cx).read(cx).pending_shorts().first().map(|post| post.id.clone());
        let (aligned, first_id, count, pending_id, stalled) = {
            let snapshot = state.read(cx);
            (
                snapshot.pager.read(cx).aligned_id(),
                snapshot.items.first().map(|post| post.id.clone()),
                snapshot.items.len(),
                pending_id,
                socket_dropped && snapshot.items.is_empty() && is_pending,
            )
        };
        let connected = RiseTransportStatus::is_connected(cx);
        let feed_top_request = ShortsServicesStore::shared(cx).read(cx).feed_top_request();
        let pull_zone = {
            let snapshot = state.read(cx);
            snapshot.items.first().and_then(|first| snapshot.pager.read(cx).page_top(&first.id)).map(|top| {
                if top > PostListRefreshControl::TRIGGER_DISTANCE {
                    2_u8
                } else if top <= 1.0 {
                    0
                } else {
                    1
                }
            })
        };
        let on_pull = state.clone();
        on_change("shorts-tab-pull", (pull_zone, is_pending), false, window, cx, move |_, _, _, cx| {
            on_pull.update(cx, |state, cx| {
                let top = state.items.first().and_then(|first| state.pager.read(cx).page_top(&first.id));
                state.track_pull_refresh(top, is_pending, cx);
            });
        });
        let on_top_request = state.clone();
        on_change("shorts-tab-feed-top", feed_top_request, false, window, cx, move |_, _, _, cx| {
            on_top_request.update(cx, |state, cx| state.scroll_to_feed_top(cx));
        });
        let on_pending_end = state.clone();
        on_change("shorts-tab-feed-pending", is_pending, false, window, cx, move |_, pending, _, cx| {
            if !*pending {
                on_pending_end.update(cx, |state, cx| {
                    state.pull_refreshing = false;
                    cx.notify();
                });
            }
        });

        let on_aligned = state.clone();
        on_change("shorts-tab-aligned", aligned, true, window, cx, move |_, aligned, _, cx| {
            if let Some(aligned) = aligned {
                on_aligned.update(cx, |state, cx| state.page_aligned(aligned, cx));
            }
        });
        let docked_post_id =
            (visible && aside_navigator::docks_comments(cx)).then(|| state.read(cx).active_post_id.clone()).flatten();
        let docked_temp = docked_post_id.as_deref().is_some_and(|post_id| {
            state.read(cx).items.iter().any(|post| post.id == post_id && post.is_temp)
        });
        let docked = (docked_post_id, aside_navigator::section_visits(cx), docked_temp);
        let on_docked = state.clone();
        on_change("shorts-tab-docked-comments", docked, true, window, cx, move |was, now, _, cx| {
            let post_id = &now.0;
            if was.0.is_none() && post_id.is_none() {
                return;
            }
            on_docked.update(cx, |state, cx| state.sync_docked_comments(post_id.as_deref(), cx));
        });
        let on_pending = state.clone();
        on_change("shorts-tab-pending", pending_id, false, window, cx, move |_, pending_id, _, cx| {
            let Some(pending_id) = pending_id else {
                return;
            };
            let pager = on_pending.read(cx).pager.clone();
            pager.update(cx, |pager, cx| pager.scroll_to(pending_id, cx));
        });
        let on_visible = state.clone();
        on_change("shorts-tab-visible", visible, false, window, cx, move |_, visible, _, cx| {
            on_visible.update(cx, |state, cx| state.visibility_changed(*visible, cx));
        });
        let on_stalled = state.clone();
        on_change("shorts-tab-stalled", stalled, true, window, cx, move |_, stalled, _, cx| {
            if *stalled {
                on_stalled.update(cx, |state, cx| {
                    state.feed_stalled_offline = true;
                    cx.notify();
                });
            }
        });
        let on_connected = state.clone();
        on_change("shorts-tab-connected", connected, false, window, cx, move |_, connected, _, cx| {
            if *connected {
                on_connected.update(cx, |state, cx| {
                    state.feed_stalled_offline = false;
                    cx.notify();
                });
            }
        });
        let on_first = state.clone();
        on_change("shorts-tab-first", first_id, false, window, cx, move |_, first_id, _, cx| {
            on_first.update(cx, |state, cx| state.first_item_changed(first_id.as_deref(), cx));
        });
        let on_count = state.clone();
        on_change("shorts-tab-count", count, false, window, cx, move |_, _, _, cx| {
            on_count.update(cx, |state, cx| state.item_count_changed(cx));
        });
    }

    fn empty_state(failed: bool, secondary: gpui::Hsla) -> AnyElement {
        let text = if failed {
            tr("feed_load_error")
        } else {
            tr_args("shorts_empty", &[FormatArg::Text(Cow::Owned(tr("riffs")))])
        };
        vstack(12.0)
            .size_full()
            .items_center()
            .justify_center()
            .children(IconUi::sized("play.rectangle.on.rectangle", px(44.0), secondary, false))
            .child(div().w_full().px(px(28.0)).child(MainText::new(text).px(15.0).tac(TextAlignT::Center).color(secondary)))
            .when(failed, |stack| {
                stack.child(
                    ButtonUi::new("shorts-tab-retry")
                        .fit_content(true)
                        .padding_horizontal(18.0)
                        .on_press(|_, cx| {
                            ShortsInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh(cx));
                        })
                        .child(MainText::new(tr("shorts_retry")).px(14.0).font_weight(FontWeight::SEMIBOLD)),
                )
            })
            .into_any_element()
    }

    fn feed_pager(state: &Entity<ShortsTabState>, visible: bool, bottom_inset: f32, cx: &mut App) -> AnyElement {
        let snapshot = state.read(cx);
        let size = snapshot.size;
        if size.width <= px(0.0) || size.height <= px(0.0) {
            return div().into_any_element();
        }
        let items = snapshot.items.clone();
        let active_post_id = snapshot.active_post_id.clone();
        let return_rewind = Rc::new(snapshot.return_rewind.clone());
        let relative_id = snapshot.feed_relative_id.clone();
        let has_more = snapshot.feed_has_more;
        let progress_floor = snapshot.last_chrome_lift.unwrap_or(bottom_inset);
        let pager = snapshot.pager.clone();
        let page_height = f32::from(size.height).max(MINIMUM_PAGE_HEIGHT);
        pager.update(cx, |pager, _| pager.update_pages(&items, 0, page_height));
        let cache_key = SharedString::from(ShortsServicesStore::feed_cache_key());
        let pass = state.update(cx, |state, _| state.begin_feed_cell_pass());
        let cells = state.clone();
        ShortsVerticalPager::new(
            "shorts-tab-pager",
            pager,
            Size { width: size.width, height: px(page_height) },
            move |index, _, cx| {
                let Some(post) = items.get(index) else {
                    return div().into_any_element();
                };
                let props = ShortsTabFeedCellProps {
                    post: post.clone(),
                    is_active: visible && active_post_id.as_deref() == Some(post.id.as_str()),
                    bottom_inset,
                    progress_floor,
                    rewind_token: return_rewind.token(&post.id),
                    index,
                    item_count: items.len(),
                    cache_key: cache_key.clone(),
                    relative_id: relative_id.clone(),
                    has_more,
                };
                let cell = cells.update(cx, |state, cx| state.feed_cell(&post.id, pass, cx));
                cell.update(cx, |cell, cx| cell.set_props(props, cx));
                AnyView::from(cell).cached(StyleRefinement::default().size_full()).into_any_element()
            },
        )
        .takes_keys(visible)
        .into_any_element()
    }
}

impl RenderOnce for ShortsTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let bottom_inset = main_tab_bar_underlap(cx);
        let state = window.use_keyed_state("shorts-tab-state", cx, |_, cx| ShortsTabState::new(cx));
        state.update(cx, |state, cx| {
            state.bind_navigation(nav_store.clone(), cx);
            state.refresh_if_settled(cx);
        });

        let (primary, secondary) = {
            let theme = rise_ui::theme(cx);
            (theme.primary._100, theme.text.secondary)
        };
        let visible = state.read(cx).is_screen_visible(cx);
        let actions = ShortsActionsStore::shared(cx);
        let (is_pending, slice_error) = {
            let actions = actions.read(cx);
            (actions.get_shorts_feed_sai.is_pending(), actions.get_shorts_feed_sai.error().is_some())
        };
        let last_error = ShortsServicesStore::shared(cx).read(cx).last_error_text().is_some();
        let publish_error = PostServicesStore::shared(cx).read(cx).last_publish_error.clone();

        Self::observe_changes(&state, visible, is_pending, window, cx);
        Self::publish_error_alert(&state, publish_error, window, cx);
        use_task("shorts-tab-task", (), window, cx, async move |cx| {
            let _ = cx.update(|_, cx| {
                ShortsInteractionsStore::shared(cx).update(cx, |store, cx| store.load_initial(cx));
            });
        });

        let (is_empty, stalled, show_publish_error, pull_refreshing) = {
            let snapshot = state.read(cx);
            (
                snapshot.items.is_empty(),
                snapshot.feed_stalled_offline,
                snapshot.show_publish_error,
                snapshot.pull_refreshing,
            )
        };
        let body: AnyElement = if is_empty && is_pending && !stalled {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(LoaderUi::new().size(LoaderUiSize::Large).color(Some(primary)))
                .into_any_element()
        } else if is_empty {
            Self::empty_state(stalled || last_error || slice_error, secondary)
        } else {
            Self::feed_pager(&state, visible, bottom_inset, cx)
        };

        let measured = state.clone();
        let content = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(black())
            .child(measure("shorts-tab-geometry", move |bounds, _, cx| {
                let bar_top = BottomChromeInsets::shared(cx).read(cx).top_edge_in_window(&[BottomChromeOwner::FloatingAction]);
                measured.update(cx, |state, cx| {
                    if state.size != bounds.size {
                        state.size = bounds.size;
                        cx.notify();
                    }
                    let lift =
                        state.riffs_chrome_lift(f32::from(bounds.bottom()), f32::from(bounds.size.height), bar_top, bottom_inset, cx);
                    if state.last_chrome_lift != Some(lift) {
                        state.last_chrome_lift = Some(lift);
                        cx.notify();
                    }
                });
            }))
            .child(body)
            .when(pull_refreshing && !is_empty, |content| content.child(ShortsPullRefreshIndicator));

        let push_store = nav_store;
        let shell = ListShellUi::new()
            .center_slot(
                CommentsSheetProgressFade::new()
                    .child(
                        MainText::new(tr("riffs"))
                            .px(18.0)
                            .font_weight(FontWeight::BOLD)
                            .color(white())
                            .liquid_glass_nav_pill(10.0, None),
                    )
                    .liquid_glass_contrast(LiquidGlassContrast::Media)
                    .modifier(ShortsChromeFade::new(true)),
            )
            .center_slot_self_styled(true)
            .use_safe_zone(false)
            .shows_back_button(false)
            .right_jsx(
                CommentsSheetProgressFade::new()
                    .child(
                        PageHeaderIconButton::new("shorts-tab-create", "plus", move |_, cx| {
                            if let Some(nav_store) = &push_store {
                                nav_store.update(cx, |nav_store, cx| {
                                    nav_store.push(
                                        RootRoute::CreatePostMedia {
                                            intent: CreatePostIntent::Reel,
                                            track_seed: None,
                                            journey_id: None,
                                        },
                                        NavigationStoreSettlingPush::Drop,
                                        cx,
                                    );
                                });
                            }
                        })
                        .icon_color(white())
                        .into_any_element(),
                    )
                    .liquid_glass_contrast(LiquidGlassContrast::Media)
                    .modifier(ShortsChromeFade::new(true)),
            )
            .fills_viewport_background(false)
            .header_chrome_shadow(None)
            .content(content);

        let on_appear = state.clone();
        let on_disappear = state;
        div()
            .relative()
            .size_full()
            .child(
                StatusBarStyleAnchor::new("shorts-tab-status-bar", UserInterfaceStyle::Dark)
                    .is_active(visible && !show_publish_error),
            )
            .child(shell)
            .on_appear("shorts-tab-lifecycle", move |_, cx| {
                on_appear.update(cx, |state, cx| state.sync_full_screen_playback_ownership(cx));
            })
            .and_on_disappear(move |_, cx| {
                on_disappear.update(cx, |state, cx| state.disappeared(cx));
            })
    }
}

#[derive(IntoElement)]
struct ShortsPullRefreshIndicator;

impl RenderOnce for ShortsPullRefreshIndicator {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top_inset = env::<ListShellFloatingTopInsetKey>(cx);
        pass_through()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .pt(px(top_inset + 12.0))
            .child(LoaderUi::new().size(LoaderUiSize::Small).color(Some(white())))
    }
}

struct ShortsChromeFadeOpacity {
    opacity: Animated<f32>,
    _subscription: Subscription,
}

impl ShortsChromeFadeOpacity {
    fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let services = ShortsServicesStore::shared(cx);
        let hidden = services.read(cx).feed_chrome_hidden();
        Self {
            opacity: Animated::new(if hidden { 0.0 } else { 1.0 }),
            _subscription: cx.observe(&services, |this, services, cx| {
                let target = if services.read(cx).feed_chrome_hidden() { 0.0 } else { 1.0 };
                if *this.opacity.target() != target {
                    this.opacity.set_in(target, ease_in_out(0.2), RiseClock::monotonic(), cx);
                    cx.notify();
                }
            }),
        }
    }
}

/// chrome over the riffs feed fades while the open riff is held or pinched,
/// own modifier so only these layers redraw
pub struct ShortsChromeFade {
    is_enabled: bool,
}

impl ShortsChromeFade {
    pub fn new(is_enabled: bool) -> Self {
        Self { is_enabled }
    }
}

impl Default for ShortsChromeFade {
    fn default() -> Self {
        Self::new(true)
    }
}

impl ViewModifier for ShortsChromeFade {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        if !self.is_enabled {
            return content;
        }
        let hidden = ShortsServicesStore::shared(cx).read(cx).feed_chrome_hidden();
        let fade = ShortsChromeFadeOpacity::shared(cx);
        let opacity = fade.read(cx).opacity.frame_value(RiseClock::monotonic(), window);
        div()
            .relative()
            .opacity(opacity)
            .child(content)
            .when(hidden, |layer| layer.child(div().absolute().inset_0().occlude()))
            .into_any_element()
    }
}
