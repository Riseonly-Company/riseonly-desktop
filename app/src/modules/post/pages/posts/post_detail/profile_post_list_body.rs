use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{
    AnyView, App, Context, ElementId, Empty, IntoElement, ListAlignment, ListState, ParentElement, Render, RenderOnce,
    SharedString, Styled, Subscription, Window, div, list, prelude::*, px,
};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::{env, env_scope};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};
use rise_ui::swiftui::stacks::vstack;
use rise_ui::window_screen_frame_reader::WindowScreenFrameReader;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementHostProbe;
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementSurfaceKey;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::wrappers::list_shell_ui::{
    ListShellScrollUnderChromeModifiers, list_shell_floating_top_inset, list_shell_safe_area_insets,
};
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::post::post_caption_prewarm::PostCaptionPrewarm;
use crate::modules::post::components::post_collection_ui::PostPaginationTriggerModifiers;
use crate::modules::post::components::post_feed_media_prefetch::PostFeedMediaPrefetchController;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPagePresentation, RisePostPageScope, RisePostPageStatus, RisePostProfileKind, RisePostViewSurface,
};
pub use crate::modules::post::pages::posts::post_detail::list_rows_splice_hooks::splice_list_rows;
use crate::modules::post::pages::posts::post_detail::post_list_anchor_keeper::PostListAnchorKeeper;
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_destination::PostListSharedElementDestination;
use crate::core::shared_element_transition::kernel::shared_element_types::ProfilePostListWindowPolicy;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

const LIST_OVERDRAW: f32 = 600.0;

type ConsumeCommentFocus = Rc<dyn Fn(&GetPostFeedResponse, &mut App)>;

/// what the screen hands the list on every pass
#[derive(Clone)]
struct ProfilePostListBodyProps {
    anchor_id: String,
    anchor_post: Option<GetPostFeedResponse>,
    user_id: String,
    tag: String,
    kind: RisePostProfileKind,
    page: RisePostPagePresentation,
    destination: Rc<PostListSharedElementDestination>,
    latched: Binding<bool>,
    consume_comment_focus: ConsumeCommentFocus,
}

#[derive(IntoElement)]
pub struct ProfilePostListBody {
    anchor_index: i64,
    props: ProfilePostListBodyProps,
}

impl ProfilePostListBody {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        anchor_id: impl Into<String>,
        anchor_index: i64,
        anchor_post: Option<GetPostFeedResponse>,
        user_id: impl Into<String>,
        tag: impl Into<String>,
        kind: RisePostProfileKind,
        page: RisePostPagePresentation,
        destination: Rc<PostListSharedElementDestination>,
        latched: Binding<bool>,
        consume_comment_focus: impl Fn(&GetPostFeedResponse, &mut App) + 'static,
    ) -> Self {
        Self {
            anchor_index,
            props: ProfilePostListBodyProps {
                anchor_id: anchor_id.into(),
                anchor_post,
                user_id: user_id.into(),
                tag: tag.into(),
                kind,
                page,
                destination,
                latched,
                consume_comment_focus: Rc::new(consume_comment_focus),
            },
        }
    }
}

impl RenderOnce for ProfilePostListBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = SharedString::from(format!("profile-post-list-body-{}", self.props.destination.session));
        let anchor_index = self.anchor_index;
        let initial = self.props.clone();
        let view = window.use_keyed_state(ElementId::Name(key), cx, move |_, cx| {
            ProfilePostListBodyView::new(initial, anchor_index, cx)
        });
        view.update(cx, |view, _| view.props = self.props);
        AnyView::from(view)
    }
}

struct ProfilePostListBodyView {
    props: ProfilePostListBodyProps,
    revealed_from_id: String,
    ledger: Rc<RefCell<ProfilePostListLedger>>,
    list: ListState,
    rendered_row_ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProfilePostListBodyView {
    fn new(props: ProfilePostListBodyProps, anchor_index: i64, cx: &mut Context<Self>) -> Self {
        let presenter = HoldContextMenuPresenter::shared(cx);
        Self {
            revealed_from_id: props.anchor_id.clone(),
            props,
            ledger: Rc::new(RefCell::new(ProfilePostListLedger::new(anchor_index))),
            list: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            rendered_row_ids: Vec::new(),
            _subscriptions: vec![cx.observe(&presenter, |_, _, cx| cx.notify())],
        }
    }

    /// origin scope can get evicted while this page sits in
    /// the stack, reload it so rows around the anchor come back
    fn reactivate_origin_if_evicted(props: &ProfilePostListBodyProps, status: RisePostPageStatus, cx: &mut App) {
        if status != RisePostPageStatus::Idle && status != RisePostPageStatus::Failed {
            return;
        }
        PostInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.activate_profile(&props.user_id, &props.tag, props.kind, cx));
    }

    fn appeared(
        props: &ProfilePostListBodyProps,
        ledger: &Rc<RefCell<ProfilePostListLedger>>,
        lower: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !props.latched.get(cx) {
            props.latched.set(true, window, cx);
        }
        let scope = RisePostPageScope::Profile {
            user_id: props.user_id.clone(),
            tag: props.tag.clone(),
            kind: props.kind,
        };
        let scope_key = scope.key();
        let anchor_id = props.anchor_id.clone();
        props.destination.configure(
            scope_key,
            Rc::new(move |cx: &mut App| {
                let services = PostServicesStore::shared(cx);
                let page_items = services.update(cx, |store, cx| store.page(&scope, cx).items);
                if page_items.iter().any(|item| item.id == anchor_id) {
                    return page_items;
                }
                let Some(anchor) = services.update(cx, |store, cx| store.resolved_post(&anchor_id, cx)) else {
                    return page_items;
                };
                let mut items = Vec::with_capacity(page_items.len() + 1);
                items.push(anchor);
                items.extend(page_items);
                items
            }),
        );
        if let Some(anchor_post) = &props.anchor_post {
            (props.consume_comment_focus)(anchor_post, cx);
        }
        Self::reactivate_origin_if_evicted(props, props.page.status, cx);
        {
            let mut ledger = ledger.borrow_mut();
            ledger.activate();
            ledger.window_top_did_move(lower);
        }
        ProfilePostListLedger::schedule_flush(ledger, cx);
    }
}

impl Render for ProfilePostListBodyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let props = self.props.clone();
        let (rows, lower) = {
            let mut ledger = self.ledger.borrow_mut();
            let rows = ledger.rows(&props.page, &props.anchor_id, props.anchor_post.as_ref());
            let position = ledger.position(&self.revealed_from_id);
            let lower = position.unwrap_or_else(|| {
                ProfilePostListWindowPolicy::lower_index(&rows.ids, &self.revealed_from_id, ledger.hint_index)
            });
            (rows, lower)
        };
        let diag_before = (self.rendered_row_ids.len(), self.rendered_row_ids.first().cloned(), self.list.logical_scroll_top());
        splice_list_rows(&self.list, &mut self.rendered_row_ids, &rows.row_ids[lower..]);
        if diag_before.0 != self.rendered_row_ids.len() || diag_before.1.as_ref() != self.rendered_row_ids.first() {
            let top = self.list.logical_scroll_top();
            tracing::warn!(
                target: "riseonly::x2176",
                "profile list splice: revealed={} lower={} ids={} rows {}->{} first {:?}->{:?} top {}+{:?} -> {}+{:?} anchor_post={}",
                self.revealed_from_id,
                lower,
                rows.ids.len(),
                diag_before.0,
                self.rendered_row_ids.len(),
                diag_before.1,
                self.rendered_row_ids.first(),
                diag_before.2.item_ix,
                diag_before.2.offset_in_item,
                top.item_ix,
                top.offset_in_item,
                props.anchor_post.is_some(),
            );
        }

        let this = cx.entity();
        let entity_id = cx.entity_id();
        let floating_top_inset = list_shell_floating_top_inset(cx);
        let safe_bottom = list_shell_safe_area_insets(cx).bottom;

        let ledger = self.ledger.clone();
        on_change(("profile-post-list-lower", entity_id), lower, false, window, cx, move |_, new_lower, _, cx| {
            ledger.borrow_mut().window_top_did_move(*new_lower);
            ProfilePostListLedger::schedule_flush(&ledger, cx);
        });
        let ledger = self.ledger.clone();
        on_change(("profile-post-list-count", entity_id), rows.ids.len(), false, window, cx, move |_, _, _, cx| {
            ledger.borrow_mut().rows_did_append(lower);
            ProfilePostListLedger::schedule_flush(&ledger, cx);
        });
        let focus_props = props.clone();
        on_change(
            ("profile-post-list-anchor", entity_id),
            props.anchor_post.as_ref().map(|post| post.id.clone()),
            false,
            window,
            cx,
            move |_, _, _, cx| {
                if let Some(anchor_post) = &focus_props.anchor_post {
                    (focus_props.consume_comment_focus)(anchor_post, cx);
                }
            },
        );
        let status_props = props.clone();
        on_change(
            ("profile-post-list-status", entity_id),
            props.page.status,
            false,
            window,
            cx,
            move |_, new_status, _, cx| {
                if *new_status != RisePostPageStatus::Idle {
                    return;
                }
                Self::reactivate_origin_if_evicted(&status_props, *new_status, cx);
            },
        );
        let ledger = self.ledger.clone();
        on_change(
            ("profile-post-list-revealed", entity_id),
            self.revealed_from_id.clone(),
            false,
            window,
            cx,
            move |_, new_id, _, _| {
                let mut ledger = ledger.borrow_mut();
                if let Some(index) = ledger.position(new_id) {
                    ledger.hint_index = index as i64;
                }
            },
        );
        let rekey_view = this.clone();
        let ledger = self.ledger.clone();
        on_change(("profile-post-list-ids", entity_id), rows.ids.clone(), false, window, cx, move |_, new_ids, _, cx| {
            rekey_view.update(cx, |view, cx| {
                if new_ids.is_empty() || new_ids.contains(&view.revealed_from_id) {
                    return;
                }
                let hint = usize::try_from(ledger.borrow().hint_index.max(0)).unwrap_or(0);
                view.revealed_from_id = new_ids[hint.min(new_ids.len() - 1)].clone();
                cx.notify();
            });
        });

        let revealed_get = this.clone();
        let revealed_set = this.clone();
        let revealed_from_id = Binding::new(
            move |cx: &App| revealed_get.read(cx).revealed_from_id.clone(),
            move |value: String, _: &mut Window, cx: &mut App| {
                revealed_set.update(cx, |view, cx| {
                    view.revealed_from_id = value;
                    cx.notify();
                });
            },
        );

        let session = props.destination.session;
        let destination = props.destination.clone();
        let row_props = props.clone();
        let row_rows = rows.clone();
        let row_ledger = self.ledger.clone();
        let separator = theme.border._100;
        let rows_list = list(self.list.clone(), move |offset, _window, _cx| {
            let index = lower + offset;
            let Some(item) = row_rows.items.get(index) else {
                return Empty.into_any_element();
            };
            let is_opened = item.id == row_props.anchor_id;
            let post = if is_opened { row_props.anchor_post.as_ref().unwrap_or(item) } else { item };
            let item_count = row_rows.items.len();
            let presentation_id = post.presentation_id();
            let load_user_id = row_props.user_id.clone();
            let load_tag = row_props.tag.clone();
            let load_kind = row_props.kind;
            let appear_ledger = row_ledger.clone();
            let appear_destination = destination.clone();
            let row = ProfilePostListRow {
                post: Rc::new(post.clone()),
                is_opened,
                frame_key: PostListAnchorKeeper::row_frame_key(session, &item.id),
            }
            .post_pagination_trigger_threshold(
                SharedString::from(format!("profile-post-list-page-{presentation_id}")),
                index as i64,
                item_count as i64,
                row_props.page.has_more,
                row_props.page.is_loading_more,
                10,
                move |cx| {
                    PostInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.load_more_profile(&load_user_id, &load_tag, load_kind, cx));
                },
            )
            .on_appear(
                SharedString::from(format!("profile-post-list-appear-{presentation_id}")),
                move |_, cx| {
                    if offset == 0 {
                        appear_destination.window_top_row_did_appear(cx);
                        return;
                    }
                    appear_ledger.borrow_mut().row_did_appear(index, lower);
                    ProfilePostListLedger::schedule_flush(&appear_ledger, cx);
                },
            );
            vstack(5.0)
                .w_full()
                .pb(px(if index + 1 < item_count { 5.0 } else { 0.0 }))
                .child(row)
                .child(div().w_full().h(px(1.0)).bg(separator))
                .into_any_element()
        })
        .size_full()
        .list_shell_scroll_under_chrome(cx);

        let surface_scope = env_scope::<SharedElementSurfaceKey>(Some(props.destination.surface())).child(rows_list);

        let appear_props = props.clone();
        let appear_ledger = self.ledger.clone();
        let disappear_ledger = self.ledger.clone();
        div()
            .relative()
            .size_full()
            .bg(theme.bg._100)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_0()
                    .child(SharedElementHostProbe::destination(props.destination.clone()))
                    .child(
                        PostListAnchorKeeper::new(
                            props.destination.clone(),
                            revealed_from_id,
                            lower,
                            rows.ids.clone(),
                            floating_top_inset,
                            safe_bottom,
                        )
                        .list_state(self.list.clone()),
                    ),
            )
            .child(surface_scope)
            .children(HoldContextMenuPresenter::has_active_request(cx).then(|| div().absolute().inset_0().occlude()))
            .on_appear(("profile-post-list-root", entity_id), move |window, cx| {
                Self::appeared(&appear_props, &appear_ledger, lower, window, cx);
            })
            .and_on_disappear(move |_, cx| disappear_ledger.borrow_mut().deactivate(cx))
    }
}

#[derive(IntoElement)]
struct ProfilePostListRow {
    post: Rc<GetPostFeedResponse>,
    is_opened: bool,
    frame_key: String,
}

impl RenderOnce for ProfilePostListRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let surface = if self.is_opened { RisePostViewSurface::PostDetail } else { RisePostViewSurface::ProfileList };
        let comments_post = self.post.clone();
        let avatar_post = self.post.clone();
        let like_post = self.post.clone();
        let fav_post = self.post.clone();
        div()
            .relative()
            .w_full()
            .child(div().absolute().inset_0().child(WindowScreenFrameReader::new(self.frame_key)))
            .child(
                Post::new(SharedString::from(self.post.presentation_id()), self.post)
                    .feed_row(true)
                    .view_surface(surface)
                    .on_open_comments(move |_, cx| {
                        PostInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.open_comments(&comments_post, false, cx));
                    })
                    .on_avatar_tap(move |_, cx| {
                        let route = RootRoute::UserPage {
                            user_id: avatar_post.author.id.clone(),
                            tag: Some(avatar_post.author.tag.clone()),
                        };
                        if let Some(nav_store) = &nav_store {
                            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                        }
                    })
                    .on_like(move |_, cx| {
                        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&like_post, cx));
                    })
                    .on_fav(move |_, cx| {
                        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_favorite(&fav_post, cx));
                    }),
            )
    }
}

#[derive(Clone)]
struct ProfilePostListRows {
    items: Rc<Vec<GetPostFeedResponse>>,
    ids: Rc<Vec<String>>,
    row_ids: Rc<Vec<String>>,
}

/// bookkeeping the list body reads but never observes
/// rows memoized per page array, window hint and warm ups for rows about to show
struct ProfilePostListLedger {
    hint_index: i64,
    current: ProfilePostListRows,
    source: Vec<GetPostFeedResponse>,
    source_revision: Option<i64>,
    source_anchor_id: String,
    source_anchor_post_id: Option<String>,
    positions: Option<HashMap<String, usize>>,
    appended_from: Option<usize>,

    media_prefetch: Option<PostFeedMediaPrefetchController>,
    is_active: bool,
    flush_scheduled: bool,
    flush_token: u64,
    pending_above: Option<usize>,
    pending_below: Option<(usize, usize)>,
    pending_appended_from: Option<usize>,
    last_below_index: Option<usize>,
}

impl ProfilePostListLedger {
    const CAPTION_ROWS: usize = 6;

    fn new(hint_index: i64) -> Self {
        Self {
            hint_index,
            current: ProfilePostListRows {
                items: Rc::new(Vec::new()),
                ids: Rc::new(Vec::new()),
                row_ids: Rc::new(Vec::new()),
            },
            source: Vec::new(),
            source_revision: None,
            source_anchor_id: String::new(),
            source_anchor_post_id: None,
            positions: None,
            appended_from: None,
            media_prefetch: None,
            is_active: true,
            flush_scheduled: false,
            flush_token: 0,
            pending_above: None,
            pending_below: None,
            pending_appended_from: None,
            last_below_index: None,
        }
    }

    /// rebuilt only when the page array, its revision or the anchor changed
    /// a reveal re runs the body over the same page and reads from here
    fn rows(
        &mut self,
        page: &RisePostPagePresentation,
        anchor_id: &str,
        anchor_post: Option<&GetPostFeedResponse>,
    ) -> ProfilePostListRows {
        let anchor_post_id = anchor_post.map(|post| post.id.as_str());
        if self.source_revision == Some(page.revision)
            && self.source_anchor_id == anchor_id
            && self.source_anchor_post_id.as_deref() == anchor_post_id
            && self.source == page.items
        {
            return self.current.clone();
        }
        let mut items = Vec::with_capacity(page.items.len() + 1);
        if !page.items.iter().any(|item| item.id == anchor_id) {
            items.extend(anchor_post.cloned());
        }
        items.extend(page.items.iter().cloned());
        let previous_ids = &self.current.ids;
        if let Some(first_id) = previous_ids.first()
            && items.len() > previous_ids.len()
            && items.first().is_some_and(|item| &item.id == first_id)
        {
            self.appended_from = Some(previous_ids.len());
        }
        self.current = ProfilePostListRows {
            ids: Rc::new(items.iter().map(|item| item.id.clone()).collect()),
            row_ids: Rc::new(items.iter().map(GetPostFeedResponse::presentation_id).collect()),
            items: Rc::new(items),
        };
        self.source = page.items.clone();
        self.source_revision = Some(page.revision);
        self.source_anchor_id = anchor_id.to_owned();
        self.source_anchor_post_id = anchor_post_id.map(str::to_owned);
        self.positions = None;
        self.current.clone()
    }

    fn position(&mut self, id: &str) -> Option<usize> {
        let ids = &self.current.ids;
        self.positions
            .get_or_insert_with(|| {
                let mut map = HashMap::with_capacity(ids.len());
                for (index, row_id) in ids.iter().enumerate() {
                    map.entry(row_id.clone()).or_insert(index);
                }
                map
            })
            .get(id)
            .copied()
    }

    fn activate(&mut self) {
        self.is_active = true;
    }

    fn deactivate(&mut self, cx: &mut App) {
        self.is_active = false;
        self.flush_token = self.flush_token.wrapping_add(1);
        self.flush_scheduled = false;
        self.pending_above = None;
        self.pending_below = None;
        self.pending_appended_from = None;
        self.last_below_index = None;
        if let Some(media_prefetch) = &mut self.media_prefetch {
            media_prefetch.cancel_all(cx);
        }
    }

    /// window top moved so load media and captions for the rows the keeper reveals next
    fn window_top_did_move(&mut self, lower: usize) {
        self.pending_above = Some(lower);
    }

    fn row_did_appear(&mut self, index: usize, lower: usize) {
        self.pending_below = Some((index, lower));
        self.last_below_index = Some(index);
    }

    fn rows_did_append(&mut self, lower: usize) {
        let Some(from) = self.appended_from.take() else {
            return;
        };
        self.pending_appended_from = Some(from);
        if self.pending_below.is_none()
            && let Some(last_below_index) = self.last_below_index
        {
            self.pending_below = Some((last_below_index, lower));
        }
    }

    /// one flush per main turn, outside the pass that revealed or realized the rows
    fn schedule_flush(ledger: &Rc<RefCell<Self>>, cx: &mut App) {
        let token = {
            let mut this = ledger.borrow_mut();
            let has_work =
                this.pending_above.is_some() || this.pending_below.is_some() || this.pending_appended_from.is_some();
            if !this.is_active || this.flush_scheduled || !has_work {
                return;
            }
            this.flush_scheduled = true;
            this.flush_token
        };
        let weak: Weak<RefCell<Self>> = Rc::downgrade(ledger);
        cx.defer(move |cx| {
            let Some(ledger) = weak.upgrade() else {
                return;
            };
            let mut this = ledger.borrow_mut();
            if this.flush_token == token {
                this.flush(cx);
            }
        });
    }

    fn flush(&mut self, cx: &mut App) {
        self.flush_scheduled = false;
        let posts = self.current.items.clone();
        let count = posts.len();
        if let Some(requested) = self.pending_above.take() {
            let lower = requested.min(count);
            self.prefetch().update_edge_window(
                "above",
                lower as i64,
                &posts,
                true,
                lower.saturating_sub(1) as i64..lower as i64,
                cx,
            );
            Self::warm_captions(&posts[lower.saturating_sub(Self::CAPTION_ROWS)..lower], cx);
        }
        if let Some((index, below_lower)) = self.pending_below.take() {
            let lower = below_lower.min(count);
            self.prefetch().update_edge_window("below", index as i64, &posts, false, lower as i64..count as i64, cx);
            let start = (index + 1).min(count);
            Self::warm_captions(&posts[start..(start + Self::CAPTION_ROWS).min(count)], cx);
        }
        if let Some(from) = self.pending_appended_from.take() {
            Self::warm_captions(&posts[from.min(count)..], cx);
        }
    }

    fn prefetch(&mut self) -> &mut PostFeedMediaPrefetchController {
        self.media_prefetch
            .get_or_insert_with(|| PostFeedMediaPrefetchController::new(&Uuid::new_v4().uuid_string()))
    }

    fn warm_captions(slice: &[GetPostFeedResponse], cx: &mut App) {
        if slice.is_empty() {
            return;
        }
        PostCaptionPrewarm::warm(slice, cx);
    }
}

