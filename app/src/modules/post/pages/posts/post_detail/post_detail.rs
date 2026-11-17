use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    AnyElement, App, AsyncApp, AsyncWindowContext, Context, Entity, KeyDownEvent, ListAlignment, ListOffset, ListState, SharedString,
    Subscription, WeakEntity, Window, div, list, prelude::*, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};
use rise_ui::swiftui::page_refresh::{PageFocus, PageRefreshKeys};
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::stacks::vstack;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_data_render::sai_data_render::SaiDataRender;
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, ListShellUi, list_shell_floating_top_inset};
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::post::post_skeleton::{PostCardSkeleton, PostCardSkeletonVariant};
use crate::modules::post::components::post_feed_column::PostFeedColumn;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostFeedKind, RisePostPagePresentation, RisePostPageScope, RisePostPageStatus, RisePostProfileKind,
    RisePostSearchKind, RisePostViewSurface,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_list_engine_policy::PostListEnginePolicy;
use crate::modules::post::pages::posts::post_detail::native_list::profile_post_native_list_view::ProfilePostNativeListView;
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_destination::PostListSharedElementDestination;
use crate::modules::post::pages::posts::post_detail::profile_post_list_body::{ProfilePostListBody, splice_list_rows};
use crate::core::shared_element_transition::kernel::shared_element_types::ProfilePostListWindowPolicy;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_guide_sandbox_hooks::PostGuideSandboxHooks;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

const LIST_OVERDRAW: f32 = 600.0;
const COMMENT_FOCUS_DELAY: Duration = Duration::from_millis(450);

/// fields of the post services store this page draws from
#[derive(Clone, Default, PartialEq)]
struct PostDetailServiceState {
    selected_post: Option<GetPostFeedResponse>,
    anchor_deleted_live: bool,
    last_own_deleted_is_resolved: bool,
}

impl PostDetailServiceState {
    fn read(post_id: &str, service: &PostServicesStore) -> Self {
        Self {
            selected_post: service.selected_post.as_ref().filter(|selected| selected.id == post_id).cloned(),
            anchor_deleted_live: service.live_deleted_post_ids().contains(post_id),
            last_own_deleted_is_resolved: service.last_own_deleted_post_id() == Some(post_id),
        }
    }
}

pub struct PostDetail {
    post_id: Option<String>,
    origin: Option<RisePostPageScope>,
    did_scroll_to_anchor: bool,
    opened_at_ms: i64,
    detail_owner_id: Uuid,
    shared_element_destination: Rc<PostListSharedElementDestination>,
    window_latched: bool,
    is_on_screen: bool,
    anchor_vanished: bool,
    list: ListState,
    rendered_row_ids: Vec<String>,
    observed_pending_focus_post_id: Option<String>,
    is_menu_locked: bool,
    observed_service: PostDetailServiceState,
    feed_owner_id: Uuid,
    feed_tail_observed: Option<RisePostFeedKind>,
    _subscriptions: Vec<Subscription>,
}

impl PostDetail {
    pub fn new(post_id: Option<String>, origin: Option<RisePostPageScope>, cx: &mut Context<Self>) -> Self {
        let service = PostServicesStore::shared(cx);
        let comments = CommentInteractionsStore::shared(cx);
        let observed_pending_focus_post_id = comments.read(cx).pending_focus_post_id().map(str::to_owned);
        let hold_context_menu = HoldContextMenuPresenter::shared(cx);
        let is_menu_locked = HoldContextMenuPresenter::has_active_request(cx);
        let observed_service = PostDetailServiceState::read(post_id.as_deref().unwrap_or_default(), service.read(cx));
        Self {
            post_id,
            origin,
            did_scroll_to_anchor: false,
            opened_at_ms: 0,
            detail_owner_id: Uuid::new_v4(),
            shared_element_destination: PostListSharedElementDestination::new(
                SharedElementTransitionCoordinator::next_session(),
            ),
            window_latched: false,
            is_on_screen: false,
            anchor_vanished: false,
            list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            rendered_row_ids: Vec::new(),
            observed_pending_focus_post_id,
            is_menu_locked,
            observed_service,
            feed_owner_id: Uuid::new_v4(),
            feed_tail_observed: None,
            _subscriptions: vec![
                cx.observe(&service, |this, service, cx| {
                    let next = PostDetailServiceState::read(this.post_id.as_deref().unwrap_or_default(), service.read(cx));
                    if this.observed_service != next {
                        this.observed_service = next;
                        cx.notify();
                    }
                }),
                cx.observe(&comments, |this, comments, cx| {
                    let pending = comments.read(cx).pending_focus_post_id().map(str::to_owned);
                    if this.observed_pending_focus_post_id != pending {
                        this.observed_pending_focus_post_id = pending;
                        cx.notify();
                    }
                }),
                cx.observe(&hold_context_menu, |this, _, cx| {
                    let is_locked = HoldContextMenuPresenter::has_active_request(cx);
                    if this.is_menu_locked != is_locked {
                        this.is_menu_locked = is_locked;
                        cx.notify();
                    }
                }),
            ],
        }
    }

    fn watch_owner(detail_owner_id: Uuid) -> String {
        format!("post-detail-{}", detail_owner_id.uuid_string())
    }

    fn list_title(origin: Option<&RisePostPageScope>) -> String {
        match origin {
            Some(RisePostPageScope::Profile { kind: RisePostProfileKind::Threads, .. })
            | Some(RisePostPageScope::Search { kind: RisePostSearchKind::Threads, .. })
            | Some(RisePostPageScope::Feed(RisePostFeedKind::Threads)) => tr("post_feed_tab_threads"),
            _ => tr("post_detail"),
        }
    }

    fn load_more_tail(origin: Option<&RisePostPageScope>, has_more: bool, is_busy: bool, cx: &mut App) {
        if !has_more || is_busy {
            return;
        }
        match origin {
            Some(RisePostPageScope::Profile { user_id, tag, kind }) => {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_more_profile(user_id, tag, *kind, cx));
            }
            Some(RisePostPageScope::Search { query, kind }) => {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_post_search(query, *kind, cx));
            }
            Some(RisePostPageScope::Feed(kind)) => {
                let kind = *kind;
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_feed_kind(kind, cx));
            }
            None => {
                PostInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_more_feed_kind(RisePostFeedKind::All, cx));
            }
            _ => {}
        }
    }

    fn tail_feed_kind(origin: Option<&RisePostPageScope>) -> Option<RisePostFeedKind> {
        match origin {
            None => Some(RisePostFeedKind::All),
            Some(RisePostPageScope::Feed(kind)) => Some(*kind),
            _ => None,
        }
    }

    /// opened by id with no list behind it, eg a link or a push
    /// a riff only post goes on to the riff viewer
    /// a comment link keeps the post page where the comment is
    fn open_riff_viewer_if_needed(
        post: &GetPostFeedResponse,
        post_id: &Option<String>,
        origin: Option<&RisePostPageScope>,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        let comments = CommentInteractionsStore::shared(cx);
        tracing::warn!(
            target: "riseonly::X1458",
            "riff check post={} is_riff={} show_in_publications={} origin={} pending_focus={:?} nav={} top_is_page={:?}",
            post.id,
            post.is_riff,
            post.show_in_publications,
            origin.is_some(),
            comments.read(cx).pending_focus_post_id(),
            nav_store.is_some(),
            nav_store.as_ref().map(|nav| {
                nav.read(cx).path.last() == Some(&RootRoute::PostDetail { post_id: post_id.clone(), origin: None })
            }),
        );
        if origin.is_some() || !post.is_riff_only() {
            return;
        }
        if comments.read(cx).pending_focus_post_id() == Some(post.id.as_str()) {
            return;
        }
        let Some(nav_store) = nav_store else {
            return;
        };
        let expected = RootRoute::PostDetail { post_id: post_id.clone(), origin: None };
        if nav_store.read(cx).path.last() != Some(&expected) {
            return;
        }
        let route = RootRoute::post_content(post, None);
        nav_store.update(cx, |store, cx| store.replace_top(route, cx));
    }

    fn consume_comment_focus(post: &GetPostFeedResponse, cx: &mut App) {
        let comments = CommentInteractionsStore::shared(cx);
        if comments.read(cx).pending_focus_post_id() != Some(post.id.as_str()) {
            return;
        }
        let post = post.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(COMMENT_FOCUS_DELAY).await;
            cx.update(|cx| comments.update(cx, |store, cx| store.consume_pending_focus_if_matches(&post, cx)));
        })
        .detach();
    }

    fn sai_status(status: RisePostPageStatus) -> SaiWsSliceStatus {
        match status {
            RisePostPageStatus::Idle => SaiWsSliceStatus::Idle,
            RisePostPageStatus::Loading => SaiWsSliceStatus::Pending,
            RisePostPageStatus::Loaded => SaiWsSliceStatus::Fulfilled,
            RisePostPageStatus::Failed => SaiWsSliceStatus::Rejected,
        }
    }

    fn scroll_to_anchor(weak: &WeakEntity<Self>, list: &ListState, cx: &mut App) {
        let _ = weak.update(cx, |view, _| {
            if view.did_scroll_to_anchor {
                return;
            }
            list.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.0) });
            view.did_scroll_to_anchor = true;
        });
    }

    fn request_exit(
        destination: &Weak<PostListSharedElementDestination>,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        let pop = |nav_store: &Option<Entity<NavigationStore>>, cx: &mut App| {
            if let Some(nav_store) = nav_store {
                nav_store.update(cx, |store, cx| store.pop(cx));
            }
        };
        match destination.upgrade() {
            Some(destination) => destination.request_exit(|cx| pop(nav_store, cx), cx),
            None => pop(nav_store, cx),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn feed_context_list(
        &mut self,
        feed_context: Vec<GetPostFeedResponse>,
        anchor_post: Option<&GetPostFeedResponse>,
        tail_page: &RisePostPagePresentation,
        resolved_id: &str,
        nav_store: Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let entity_id = cx.entity_id();
        let weak = cx.weak_entity();
        let row_ids: Vec<String> = feed_context.iter().map(GetPostFeedResponse::presentation_id).collect();
        splice_list_rows(&self.list, &mut self.rendered_row_ids, &row_ids);

        let anchor_id = anchor_post.map(|post| post.id.clone());
        let change_view = weak.clone();
        let change_list = self.list.clone();
        let change_anchor = anchor_post.cloned();
        on_change(
            ("post-detail-feed-anchor", entity_id),
            anchor_id.clone(),
            false,
            window,
            cx,
            move |_, new_id: &Option<String>, _, cx| {
                if new_id.is_some() {
                    let list = change_list.clone();
                    let view = change_view.clone();
                    cx.defer(move |cx| Self::scroll_to_anchor(&view, &list, cx));
                }
                if let Some(anchor) = &change_anchor {
                    Self::consume_comment_focus(anchor, cx);
                }
            },
        );

        let items = Rc::new(feed_context);
        let origin = self.origin.clone();
        let has_more = tail_page.has_more;
        let is_busy = tail_page.is_loading_more || tail_page.is_refreshing;
        let row_items = items.clone();
        let separator = theme.border._100;
        let rows = list(self.list.clone(), move |index, _window, cx| {
            let Some(post) = row_items.get(index) else {
                return div().into_any_element();
            };
            let top_inset = if index == 0 { list_shell_floating_top_inset(cx) } else { 0.0 };
            let post = Rc::new(post.clone());
            let presentation_id = post.presentation_id();
            let is_anchor = anchor_id.as_deref() == Some(post.id.as_str());
            let is_last = index + 1 == row_items.len();
            let comments_post = post.clone();
            let avatar_post = post.clone();
            let like_post = post.clone();
            let fav_post = post.clone();
            let avatar_nav = nav_store.clone();
            let tail_origin = origin.clone();
            vstack(5.0)
                .w_full()
                .pt(px(top_inset))
                .pb(px(5.0))
                .child(
                    Post::new(SharedString::from(presentation_id.clone()), post)
                        .feed_row(true)
                        .view_surface(if is_anchor {
                            RisePostViewSurface::PostDetail
                        } else {
                            RisePostViewSurface::ProfileList
                        })
                        .carries_guide_anchors(is_anchor)
                        .on_open_comments(move |_, cx| {
                            PostInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.open_comments(&comments_post, false, cx));
                        })
                        .on_avatar_tap(move |_, cx| {
                            let route = RootRoute::UserPage {
                                user_id: avatar_post.author.id.clone(),
                                tag: Some(avatar_post.author.tag.clone()),
                            };
                            if let Some(nav_store) = &avatar_nav {
                                nav_store.update(cx, |store, cx| {
                                    store.push(route, NavigationStoreSettlingPush::Drop, cx);
                                });
                            }
                        })
                        .on_like(move |_, cx| {
                            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&like_post, cx));
                        })
                        .on_fav(move |_, cx| {
                            PostInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.toggle_favorite(&fav_post, cx));
                        })
                        .on_appear(SharedString::from(format!("post-detail-row-{presentation_id}")), move |_, cx| {
                            if is_last {
                                Self::load_more_tail(tail_origin.as_ref(), has_more, is_busy, cx);
                            }
                        }),
                )
                .child(div().w_full().h(px(1.0)).bg(separator))
                .into_any_element()
        })
        .size_full();

        let locked = self.is_menu_locked;
        let appear_list = self.list.clone();
        let appear_anchor = anchor_post.cloned();
        let refresh_id = resolved_id.to_owned();
        PageRefreshKeys::new(
            ("post-detail-refresh-keys", entity_id),
            move |_, cx| {
                if refresh_id.is_empty() {
                    return;
                }
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_post_detail(&refresh_id, cx));
            },
            div()
                .relative()
                .size_full()
                .bg(theme.bg._100)
                .child(rows)
                .children(locked.then(|| div().absolute().inset_0().occlude()))
                .on_appear(("post-detail-feed-list", entity_id), move |_, cx| {
                    if let Some(anchor) = appear_anchor {
                        let list = appear_list.clone();
                        let view = weak.clone();
                        cx.defer(move |cx| Self::scroll_to_anchor(&view, &list, cx));
                        Self::consume_comment_focus(&anchor, cx);
                    }
                }),
        )
        .into_any_element()
    }
}

impl Render for PostDetail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let weak = cx.weak_entity();
        let entity_id = cx.entity_id();
        let nav_store = env::<NavigationStoreKey>(cx);
        let resolved_id = self.post_id.clone().unwrap_or_default();
        let presentation = RisePostContentPresentation::shared(cx);
        let mut anchor_post = PostGuideSandboxHooks::post(&resolved_id, cx);
        if anchor_post.is_none() {
            anchor_post = presentation.read(cx).resolved_post(&resolved_id).cloned();
        }
        if anchor_post.is_none() {
            anchor_post = self.observed_service.selected_post.clone();
        }
        let detail_page = presentation.read(cx).detail_page(&resolved_id).clone();

        let tail_page = {
            let content = presentation.read(cx);
            match &self.origin {
                Some(origin) => content.page(origin, cx).into_owned(),
                None => content.feed_page(RisePostFeedKind::All, cx).into_owned(),
            }
        };
        let window_anchor_index: Option<usize> = match &self.origin {
            Some(RisePostPageScope::Profile { kind, .. }) if SharedElementNavigationPolicy::ENABLED_KINDS.contains(kind) => {
                let ids: Vec<String> = tail_page.items.iter().map(|item| item.id.clone()).collect();
                ProfilePostListWindowPolicy::anchor_index(true, &ids, &resolved_id, self.window_latched)
            }
            _ => None,
        };
        let anchor_deleted_live = self.observed_service.anchor_deleted_live;
        let keeps_window = window_anchor_index.is_some()
            && self.window_latched
            && !tail_page.items.is_empty()
            && (!self.anchor_vanished || anchor_deleted_live);
        let render_status = if anchor_post.is_some() || keeps_window {
            SaiWsSliceStatus::Fulfilled
        } else {
            Self::sai_status(detail_page.status)
        };

        let feed_list = if window_anchor_index.is_none() {
            let feed_context: Vec<GetPostFeedResponse> = match &anchor_post {
                Some(anchor) => std::iter::once(anchor.clone())
                    .chain(tail_page.items.iter().filter(|item| item.id != anchor.id).cloned())
                    .collect(),
                None => Vec::new(),
            };
            Some(self.feed_context_list(
                feed_context,
                anchor_post.as_ref(),
                &tail_page,
                &resolved_id,
                nav_store.clone(),
                window,
                cx,
            ))
        } else {
            None
        };

        let task_view = weak.clone();
        let task_id = resolved_id.clone();
        let detail_owner_id = self.detail_owner_id;
        use_task(
            ("post-detail-task", entity_id),
            resolved_id.clone(),
            window,
            cx,
            async move |cx: &mut AsyncWindowContext| {
                if task_id.is_empty() {
                    return;
                }
                let _ = cx.update(|_, cx| {
                    let _ = task_view.update(cx, |view, _| view.did_scroll_to_anchor = false);
                    PostInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.observe_post_detail(&task_id, detail_owner_id, cx));
                });
            },
        );

        let pending_focus_post_id = self.observed_pending_focus_post_id.clone();
        let focus_view = weak.clone();
        let focus_anchor = anchor_post.clone();
        on_change(
            ("post-detail-pending-focus", entity_id),
            pending_focus_post_id,
            false,
            window,
            cx,
            move |_, pending_post_id: &Option<String>, _, cx| {
                let is_on_screen = focus_view.read_with(cx, |view, _| view.is_on_screen).unwrap_or(false);
                let Some(anchor) = focus_anchor.filter(|_| is_on_screen) else {
                    return;
                };
                if pending_post_id.as_deref() == Some(anchor.id.as_str()) {
                    Self::consume_comment_focus(&anchor, cx);
                }
            },
        );

        let riff_key = anchor_post.as_ref().map(|post| (post.id.clone(), post.is_riff_only()));
        let riff_anchor = anchor_post.clone();
        let riff_nav = nav_store.clone();
        let riff_post_id = self.post_id.clone();
        let riff_origin = self.origin.clone();
        on_change(
            ("post-detail-riff-only", entity_id),
            riff_key,
            true,
            window,
            cx,
            move |_, riff_key: &Option<(String, bool)>, _, cx| {
                if riff_key.is_some() && let Some(anchor) = &riff_anchor {
                    Self::open_riff_viewer_if_needed(anchor, &riff_post_id, riff_origin.as_ref(), &riff_nav, cx);
                }
            },
        );

        let last_own_deleted_is_resolved = self.observed_service.last_own_deleted_is_resolved;
        let vanish_view = weak.clone();
        on_change(
            ("post-detail-anchor-nil", entity_id),
            anchor_post.is_none(),
            false,
            window,
            cx,
            move |_, is_nil: &bool, _, cx| {
                let is_nil = *is_nil;
                let _ = vanish_view.update(cx, |view, cx| {
                    view.anchor_vanished = is_nil && !last_own_deleted_is_resolved;
                    cx.notify();
                });
            },
        );

        let gone_view = weak.clone();
        let gone_nav = nav_store.clone();
        let gone_service = PostServicesStore::shared(cx);
        let gone_id = resolved_id.clone();
        on_change(
            ("post-detail-gone", entity_id),
            anchor_post.is_none() && !keeps_window,
            false,
            window,
            cx,
            move |_, gone: &bool, _, cx| {
                let is_on_screen = gone_view.read_with(cx, |view, _| view.is_on_screen).unwrap_or(false);
                if !*gone
                    || !is_on_screen
                    || gone_id.is_empty()
                    || gone_service.read(cx).last_own_deleted_post_id() != Some(gone_id.as_str())
                {
                    return;
                }
                gone_service.update(cx, |service, cx| service.mark_own_post_deleted(None, cx));
                if let Some(nav_store) = &gone_nav {
                    nav_store.update(cx, |store, cx| store.pop(cx));
                }
            },
        );

        let latched: Binding<bool> = Binding::entity(
            this,
            |view: &PostDetail| view.window_latched,
            |view, value, _| view.window_latched = value,
        );
        let destination = self.shared_element_destination.clone();
        let content_origin = self.origin.clone();
        let content_id = resolved_id.clone();
        let content_anchor = anchor_post.clone();
        let content_page = tail_page.clone();
        let content = move |_window: &mut Window, _cx: &mut App| -> AnyElement {
            if let (Some(window_anchor_index), Some(RisePostPageScope::Profile { user_id, tag, kind })) =
                (window_anchor_index, content_origin)
            {
                if PostListEnginePolicy::uses_native_list() {
                    return ProfilePostNativeListView::new(
                        content_id,
                        window_anchor_index as i64,
                        content_anchor,
                        user_id,
                        tag,
                        kind,
                        content_page,
                        destination,
                        latched,
                        Self::consume_comment_focus,
                    )
                    .into_any_element();
                }
                return ProfilePostListBody::new(
                    content_id,
                    window_anchor_index as i64,
                    content_anchor,
                    user_id,
                    tag,
                    kind,
                    content_page,
                    destination,
                    latched,
                    Self::consume_comment_focus,
                )
                .into_any_element();
            }
            feed_list.unwrap_or_else(|| div().into_any_element())
        };

        let title = SharedString::from(Self::list_title(self.origin.as_ref()));
        let exit_destination = Rc::downgrade(&self.shared_element_destination);
        let back_nav = nav_store.clone();
        let back_destination = exit_destination.clone();

        let shell = ListShellUi::new()
            .title(title)
            .shows_back_button(true)
            .on_back(move |_, cx| Self::request_exit(&back_destination, &back_nav, cx))
            .header_chrome_shadow(Some(ScrollChromeShadowStyle::list_shell_tall_header()))
            .content(PostFeedColumn::new(
                ("post-detail-column", entity_id),
                SaiDataRender::new(render_status, content)
                    .id(("post-detail-data", entity_id))
                    .is_empty(anchor_post.is_none() && !keeps_window)
                    .no_data_text(tr("post_not_found"))
                    .error_text(tr("post_load_error"))
                    .message_height_percent(40.0)
                    .no_data_height_percent(30.0)
                    .sai_data_render_loading(move |_, cx| {
                        let theme = rise_ui::theme(cx).clone();
                        scroll_view(("post-detail-skeleton", entity_id))
                            .shows_indicators(false)
                            .scroll_disabled(true)
                            .child(
                                div().w_full().size_full().bg(theme.bg._100).list_shell_scroll_under_chrome(cx).child(
                                    div().pt(px(8.0)).child(PostCardSkeleton::default().variant(
                                        PostCardSkeletonVariant::Media { aspect: PostMediaBox::PLACEHOLDER_ASPECT },
                                    )),
                                ),
                            )
                    }),
            ));

        let esc_nav = nav_store.clone();
        let appear_view = weak.clone();
        let appear_id = resolved_id.clone();
        let appear_anchor = anchor_post.clone();
        let appear_nav = nav_store;
        let appear_post_id = self.post_id.clone();
        let appear_origin = self.origin.clone();
        let watch_owner = Self::watch_owner(detail_owner_id);
        let appear_owner = watch_owner.clone();
        let disappear_view = weak;
        let opened_at_ms = self.opened_at_ms;
        let page_focus = PageFocus::keyed(("post-detail-focus", entity_id), window, cx);
        let focus = page_focus.read(cx).handle.clone();
        let leaving_focus = page_focus.clone();
        div()
            .size_full()
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, _window, cx| {
                if event.keystroke.key != "escape" || event.keystroke.modifiers.modified() {
                    return;
                }
                let sheet_open = CommentInteractionsStore::shared(cx).read(cx).comments_sheet_visible;
                if sheet_open || HoldContextMenuPresenter::has_active_request(cx) {
                    return;
                }
                cx.stop_propagation();
                Self::request_exit(&exit_destination, &esc_nav, cx);
            })
            .child(shell)
            .on_appear(("post-detail-root", entity_id), move |window, cx| {
                PageFocus::did_appear(&page_focus, window, cx);
                let _ = appear_view.update(cx, |view, _| {
                    view.is_on_screen = true;
                    view.opened_at_ms = RiseClock::now().ms();
                });
                if let Some(kind) = Self::tail_feed_kind(appear_origin.as_ref()) {
                    let is_empty = RisePostContentPresentation::shared(cx).read(cx).feed_page(kind, cx).items.is_empty();
                    let feed_owner_id =
                        appear_view.read_with(cx, |view, _| view.feed_owner_id).ok().filter(|_| is_empty);
                    if let Some(feed_owner_id) = feed_owner_id {
                        let _ = appear_view.update(cx, |view, _| view.feed_tail_observed = Some(kind));
                        PostInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.observe_feed(kind, feed_owner_id, cx));
                    }
                }
                if !appear_id.is_empty() {
                    PostInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.watch_posts(vec![appear_id], &appear_owner, cx));
                }
                if let Some(anchor) = &appear_anchor {
                    Self::open_riff_viewer_if_needed(
                        anchor,
                        &appear_post_id,
                        appear_origin.as_ref(),
                        &appear_nav,
                        cx,
                    );
                }
            })
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
                let feed_tail = disappear_view
                    .update(cx, |view, _| {
                        view.is_on_screen = false;
                        view.feed_tail_observed.take().map(|kind| (kind, view.feed_owner_id))
                    })
                    .ok()
                    .flatten();
                let interactions = PostInteractionsStore::shared(cx);
                interactions.update(cx, |store, cx| store.stop_watching_posts(&watch_owner, cx));
                if let Some((kind, feed_owner_id)) = feed_tail {
                    interactions.update(cx, |store, cx| store.stop_observing_feed(kind, feed_owner_id, cx));
                }
                if resolved_id.is_empty() {
                    return;
                }
                interactions.update(cx, |store, cx| {
                    store.stop_observing_post_detail(&resolved_id, detail_owner_id, cx);
                });
                let opened_at_ms =
                    disappear_view.read_with(cx, |view, _| view.opened_at_ms).unwrap_or(opened_at_ms);
                let dwell_ms = RiseClock::now().ms() - opened_at_ms;
                if dwell_ms <= 0 {
                    return;
                }
                let post_id = anchor_post
                    .as_ref()
                    .map(GetPostFeedResponse::rise_view_target_post_id)
                    .unwrap_or_else(|| resolved_id.clone());
                let author_id = anchor_post.as_ref().and_then(|post| post.author.id.clone());
                EngagementInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.dwell(&post_id, author_id.as_deref(), dwell_ms, cx));
            })
    }
}
