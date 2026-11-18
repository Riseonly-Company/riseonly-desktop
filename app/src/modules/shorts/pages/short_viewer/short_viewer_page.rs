use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    AnyView, App, Context, ElementId, Entity, FontWeight, Pixels, SharedString, Size, StyleRefinement, Subscription, Window, black,
    white,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::button_ui::ButtonUi;
use rise_ui::expandable_text_ui::EdgeInsets;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::page_header_ui::PageHeaderUi;
use rise_ui::swiftui::modifiers::ModifierExt;
use rise_ui::swiftui::prelude::*;
use rise_ui::{MainText, TextAlignT};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::RootRoute;
use crate::core::shared_element_transition::kernel::shared_element_types::{SharedElementPart, SharedElementSurface};
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementHostProbe;
use crate::core::shared_element_transition::runtime::shared_element_lift_board::{
    SharedElementLiftModifiers, SharedElementSurfaceKey,
};
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementSurface;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::comment::widgets::comments_sheet_progress_fade::CommentsSheetProgressFade;
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::post::components::post_collection_ui::PostPaginationTriggerModifiers;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPagePresentation, RisePostPageScope, RisePostPageStatus, RisePostProfileKind, RisePostViewSurface,
};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_audio_services::PostServicesStoreAudioPostsState;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::components::shorts_feed_cell::{ShortsAutoAdvance, ShortsFeedCell, ShortsReturnRewind};
use crate::modules::shorts::components::shorts_media_prefetch::ShortsMediaPrefetchController;
use crate::modules::shorts::pages::short_viewer::short_viewer_shared_element_destination::ShortViewerSharedElementDestination;
use crate::modules::shorts::pages::shorts_vertical_pager::{ShortsVerticalPager, ShortsVerticalPagerController};
use crate::modules::shorts::pages::shorts_tab::shorts_tab::ShortsChromeFade;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;
use crate::modules::user::components::profile_content::pages::grid_posts::profile_grid_shared_element_origin::ProfileGridSharedElementOrigin;
use crate::modules::user::shared::rise_blocked_content::RiseBlockedContent;

const IMPRESSION_VISIBLE_MS: i64 = 500;
const FLIGHT_SETTLE_TIMEOUT: f64 = 1.0;

/// vertical stack behind the opened riff,
/// from a profile its own list in grid order,
/// from a sound page that sounds list
#[derive(Clone, PartialEq)]
enum ShortViewerContext {
    Feed,
    Profile { user_id: String, tag: String, kind: RisePostProfileKind },
    Sound { track_id: String },
}

impl ShortViewerContext {
    fn new(origin: Option<&RisePostPageScope>, sound_track_id: Option<&str>) -> Self {
        if let Some(track_id) = sound_track_id.filter(|track_id| !track_id.is_empty()) {
            return Self::Sound { track_id: track_id.to_owned() };
        }
        match origin {
            Some(RisePostPageScope::Profile { user_id, tag, kind }) if !user_id.is_empty() => {
                Self::Profile { user_id: user_id.clone(), tag: tag.clone(), kind: *kind }
            }
            _ => Self::Feed,
        }
    }
}

struct ShortViewerPageState {
    post: GetPostFeedResponse,
    origin: Option<RisePostPageScope>,
    context: ShortViewerContext,
    flies_shared_elements: bool,
    active_post_id: String,
    return_rewind: ShortsReturnRewind,
    playback_owner: String,
    content_owner_id: Uuid,
    media_prefetch: ShortsMediaPrefetchController,
    shared_element_destination: Rc<ShortViewerSharedElementDestination>,
    pager: Entity<ShortsVerticalPagerController>,
    viewer_posts: Rc<Vec<Rc<GetPostFeedResponse>>>,
    stack_page: RisePostPagePresentation,
    sound_posts: Option<PostServicesStoreAudioPostsState>,
    is_audio_pending: bool,
    last_own_deleted_post_id: Option<String>,
    size: Size<Pixels>,
    bottom_overhang: f32,
    feed_cells: HashMap<String, (Entity<ShortViewerFeedCellView>, u64)>,
    feed_cell_pass: u64,
    _subscriptions: Vec<Subscription>,
}

impl ShortViewerPageState {
    fn new(
        post: GetPostFeedResponse,
        origin: Option<RisePostPageScope>,
        sound_track_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let services = ShortsServicesStore::shared(cx);
        let post_services = PostServicesStore::shared(cx);
        let presentation = RisePostContentPresentation::shared(cx);
        let context = ShortViewerContext::new(origin.as_ref(), sound_track_id.as_deref());
        let flies_shared_elements =
            SharedElementNavigationPolicy::uses_shared_element_navigation_short_viewer_origin(origin.as_ref());
        let shared_element_destination =
            ShortViewerSharedElementDestination::new(SharedElementTransitionCoordinator::next_session());
        if flies_shared_elements && let Some(origin) = &origin {
            shared_element_destination.configure(origin.clone(), &post);
        }
        let opened_id = post.id.clone();
        let mut state = Self {
            post,
            origin,
            context,
            flies_shared_elements,
            active_post_id: opened_id.clone(),
            return_rewind: ShortsReturnRewind::new(),
            playback_owner: Uuid::new_v4().uuid_string(),
            content_owner_id: Uuid::new_v4(),
            media_prefetch: ShortsMediaPrefetchController::new(&Uuid::new_v4().uuid_string()),
            shared_element_destination,
            pager: cx.new(|_| ShortsVerticalPagerController::new(Some(opened_id))),
            viewer_posts: Rc::new(Vec::new()),
            stack_page: RisePostPagePresentation::IDLE,
            sound_posts: None,
            is_audio_pending: false,
            last_own_deleted_post_id: None,
            size: Size::default(),
            bottom_overhang: 0.0,
            feed_cells: HashMap::new(),
            feed_cell_pass: 0,
            _subscriptions: vec![
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&post_services, |state, _, cx| state.refresh_and_notify_if_changed(cx)),
                cx.observe(&presentation, |state, _, cx| state.refresh_and_notify_if_changed(cx)),
            ],
        };
        state.refresh(cx);
        state
    }

    fn begin_feed_cell_pass(&mut self) -> u64 {
        self.feed_cell_pass += 1;
        let pass = self.feed_cell_pass;
        self.feed_cells.retain(|_, (_, placed)| *placed + 1 >= pass);
        pass
    }

    fn feed_cell(&mut self, post_id: &str, pass: u64, cx: &mut Context<Self>) -> Entity<ShortViewerFeedCellView> {
        if let Some((cell, placed)) = self.feed_cells.get_mut(post_id) {
            *placed = pass;
            return cell.clone();
        }
        let cell = cx.new(|_| ShortViewerFeedCellView { props: None });
        self.feed_cells.insert(post_id.to_owned(), (cell.clone(), pass));
        cell
    }

    fn flight_destination(&self) -> Option<Rc<ShortViewerSharedElementDestination>> {
        (self.flies_shared_elements && self.origin.is_some()).then(|| self.shared_element_destination.clone())
    }

    /// grid a flying viewer came from, its surface is active from arm to landing
    fn flight_grid_surface(&self) -> Option<SharedElementSurface> {
        if !self.flies_shared_elements {
            return None;
        }
        let Some(RisePostPageScope::Profile { user_id, tag, kind }) = &self.origin else {
            return None;
        };
        Some(ProfileGridSharedElementOrigin::surface(user_id, tag, *kind))
    }

    fn sound_track_id(&self) -> Option<&str> {
        match &self.context {
            ShortViewerContext::Sound { track_id } => Some(track_id.as_str()),
            _ => None,
        }
    }

    fn is_profile(&self) -> bool {
        matches!(self.context, ShortViewerContext::Profile { .. })
    }

    fn refresh_if_settled(&mut self, cx: &mut Context<Self>) {
        if !self.pager.read(cx).is_moving() {
            self.refresh(cx);
        }
    }

    /// recopies the feed from the post stores and redraws only if what the page draws moved
    /// mid snap the copy waits for the render after the pager settles
    fn refresh_and_notify_if_changed(&mut self, cx: &mut Context<Self>) {
        if self.pager.read(cx).is_moving() {
            cx.notify();
            return;
        }
        let viewer_posts = self.viewer_posts.clone();
        let stack_page = self.stack_page.clone();
        let sound_posts = self.sound_posts.clone();
        let is_audio_pending = self.is_audio_pending;
        let last_own_deleted_post_id = self.last_own_deleted_post_id.clone();
        self.refresh(cx);
        if *viewer_posts != *self.viewer_posts
            || stack_page != self.stack_page
            || sound_posts != self.sound_posts
            || is_audio_pending != self.is_audio_pending
            || last_own_deleted_post_id != self.last_own_deleted_post_id
        {
            cx.notify();
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let post_services = PostServicesStore::shared(cx);
        let shorts_store = ShortsServicesStore::shared(cx);
        self.last_own_deleted_post_id = post_services.read(cx).last_own_deleted_post_id().map(str::to_owned);
        self.stack_page = match &self.context {
            ShortViewerContext::Profile { user_id, tag, kind } => post_services
                .update(cx, |service, cx| service.profile_page_with_tag(user_id, tag, *kind, cx)),
            _ => post_services.update(cx, |service, cx| service.page(&RisePostPageScope::Shorts, cx)),
        };
        let track_id = self.sound_track_id().map(str::to_owned);
        self.sound_posts =
            track_id.as_deref().map(|id| post_services.update(cx, |service, cx| service.audio_posts(id, cx)));
        self.is_audio_pending =
            track_id.as_deref().is_some_and(|id| post_services.read(cx).is_audio_posts_pending(id));

        let live_post = post_services
            .update(cx, |service, cx| service.resolved_post(&self.post.id, cx))
            .or_else(|| shorts_store.update(cx, |service, cx| service.resolved_short(&self.post.id, cx)))
            .unwrap_or_else(|| self.post.clone());
        let hidden: HashSet<String> = shorts_store.read(cx).hidden_short_ids().clone();
        let items: &[GetPostFeedResponse] =
            self.sound_posts.as_ref().map_or(&self.stack_page.items, |sound_posts| &sound_posts.items);
        let keeps_list_order = self.is_profile() || track_id.is_some();
        let pins_opened = !(keeps_list_order && items.iter().any(|item| item.id == self.post.id));
        let mut seen = HashSet::new();
        let mut posts = Vec::with_capacity(items.len() + 1);
        for item in pins_opened.then_some(&live_post).into_iter().chain(items) {
            if item.id != self.post.id && !item.is_riff {
                continue;
            }
            if hidden.contains(&item.id) {
                continue;
            }
            if RiseBlockedContent::hides_post(item, cx) || !seen.insert(item.id.clone()) {
                continue;
            }
            posts.push(Rc::new(item.clone()));
        }
        self.viewer_posts = Rc::new(posts);
    }

    fn stack_has_more(&self) -> bool {
        if let Some(sound_posts) = &self.sound_posts {
            return sound_posts.has_more;
        }
        self.stack_page.has_more && self.stack_page.error_message.is_none()
    }

    fn stack_is_loading(&self) -> bool {
        if self.sound_track_id().is_some() {
            return self.is_audio_pending;
        }
        let page = &self.stack_page;
        page.status == RisePostPageStatus::Loading || page.is_loading_more || page.is_refreshing
    }

    fn load_next_page(&self, cx: &mut App) {
        match self.context.clone() {
            ShortViewerContext::Feed => {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_shorts(cx));
            }
            ShortViewerContext::Sound { track_id } => {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_audio_posts(&track_id, cx));
            }
            ShortViewerContext::Profile { user_id, tag, kind } => {
                let load = move |cx: &mut App| {
                    PostInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.load_more_profile(&user_id, &tag, kind, cx));
                };
                match self.flight_grid_surface() {
                    Some(surface) => SharedElementTransitionCoordinator::run_or_defer_until_settled(surface, load, cx),
                    None => load(cx),
                }
            }
        }
    }

    fn load_stack(&self, cx: &mut Context<Self>) {
        let post = self.post.clone();
        ShortsServicesStore::shared(cx).update(cx, |store, cx| store.register_external_short_if_needed(post, cx));
        let owner_id = self.content_owner_id;
        PostInteractionsStore::shared(cx).update(cx, |store, cx| match &self.context {
            ShortViewerContext::Feed => store.observe_shorts(owner_id, cx),
            ShortViewerContext::Sound { track_id } => store.load_audio_posts_if_needed(track_id, cx),
            ShortViewerContext::Profile { user_id, tag, kind } => {
                store.observe_profile(user_id, tag, *kind, owner_id, cx);
            }
        });
    }

    fn stop_observing_stack(&self, cx: &mut Context<Self>) {
        let owner_id = self.content_owner_id;
        PostInteractionsStore::shared(cx).update(cx, |store, cx| match &self.context {
            ShortViewerContext::Feed => store.stop_observing_shorts(owner_id, cx),
            ShortViewerContext::Sound { .. } => {}
            ShortViewerContext::Profile { user_id, tag, kind } => {
                store.stop_observing_profile(user_id, tag, *kind, owner_id, cx);
            }
        });
    }

    fn continue_paging_near_tail(&self, cx: &mut Context<Self>) {
        if self.sound_track_id().is_none() && self.stack_page.status != RisePostPageStatus::Loaded {
            return;
        }
        if !self.stack_has_more() || self.stack_is_loading() {
            return;
        }
        let Some(active_index) = self.viewer_posts.iter().position(|post| post.id == self.active_post_id) else {
            return;
        };
        if active_index >= self.viewer_posts.len().saturating_sub(ShortViewerPage::PAGINATION_THRESHOLD as usize) {
            self.load_next_page(cx);
        }
    }

    fn warm_neighbours(&mut self, post_id: &str, cx: &mut Context<Self>) {
        let posts: Vec<GetPostFeedResponse> = self.viewer_posts.iter().map(|post| (**post).clone()).collect();
        self.media_prefetch.update("short-viewer", post_id, &posts, cx);
    }

    /// riff on screen and the ones a swipe away vanish live when the author deletes them
    fn watch_neighbours(&self, post_id: &str, cx: &mut Context<Self>) {
        let posts = &self.viewer_posts;
        let ids: Vec<String> = match posts.iter().position(|post| post.id == post_id) {
            Some(index) => {
                let high = (index + 2).min(posts.len() - 1);
                posts[index.saturating_sub(1)..=high].iter().map(|post| post.id.clone()).collect()
            }
            None => vec![post_id.to_owned()],
        };
        let owner = self.playback_owner.clone();
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.watch_posts(ids, &owner, cx));
    }

    fn fire_impression(&self, post_id: &str, cx: &mut Context<Self>) {
        let Some(item) = self.viewer_posts.iter().find(|post| post.id == post_id).cloned() else {
            return;
        };
        let author_id = item.author.id.clone().unwrap_or_else(|| item.author_id.clone());
        EngagementInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.impression(
                &item.rise_view_target_post_id(),
                &author_id,
                EngagementSurface::ShortViewer,
                -1,
                true,
                IMPRESSION_VISIBLE_MS,
                cx,
            );
        });
    }

    fn page_aligned(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if self.active_post_id == post_id {
            return;
        }
        let order: Vec<String> = self.viewer_posts.iter().map(|post| post.id.clone()).collect();
        self.return_rewind.move_to(Some(&self.active_post_id), post_id, &order);
        self.active_post_id = post_id.to_owned();
        self.shared_element_destination.update_active_post(post_id);
        self.fire_impression(post_id, cx);
        self.warm_neighbours(post_id, cx);
        self.watch_neighbours(post_id, cx);
        self.continue_paging_near_tail(cx);
        cx.notify();
    }

    fn stack_changed(&mut self, cx: &mut Context<Self>) {
        let active_post_id = self.active_post_id.clone();
        self.warm_neighbours(&active_post_id, cx);
        self.continue_paging_near_tail(cx);
    }

    fn appeared(&mut self, cx: &mut Context<Self>) {
        let owner = self.playback_owner.clone();
        ShortsServicesStore::shared(cx).update(cx, |store, cx| store.begin_full_screen_playback(owner, cx));
        let opened_id = self.post.id.clone();
        self.fire_impression(&opened_id, cx);
        let active_post_id = self.active_post_id.clone();
        self.warm_neighbours(&active_post_id, cx);
        self.watch_neighbours(&active_post_id, cx);
    }

    fn disappeared(&mut self, cx: &mut Context<Self>) {
        let owner = self.playback_owner.clone();
        ShortsServicesStore::shared(cx).update(cx, |store, cx| store.end_full_screen_playback(owner.clone(), cx));
        self.stop_observing_stack(cx);
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.stop_watching_posts(&owner, cx));
        self.media_prefetch.cancel_all(cx);
    }

    fn own_post_deleted(&self, deleted: &str, nav_store: Option<&Entity<NavigationStore>>, cx: &mut Context<Self>) {
        let Some(nav_store) = nav_store else {
            return;
        };
        if deleted != self.active_post_id {
            return;
        }
        let is_top = matches!(
            nav_store.read(cx).path.last(),
            Some(RootRoute::ShortViewer { post: top, .. }) if top.id == self.post.id
        );
        if is_top {
            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
        }
    }
}

#[derive(IntoElement)]
pub struct ShortViewerPage {
    post: GetPostFeedResponse,
    origin: Option<RisePostPageScope>,
    sound_track_id: Option<String>,
}

impl ShortViewerPage {
    const PAGINATION_THRESHOLD: i64 = 6;

    pub fn new(post: GetPostFeedResponse, origin: Option<RisePostPageScope>, sound_track_id: Option<String>) -> Self {
        Self { post, origin, sound_track_id }
    }

    fn observe_changes(
        id: &str,
        state: &Entity<ShortViewerPageState>,
        nav_store: Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (aligned, revision, sound_count) = {
            let snapshot = state.read(cx);
            (
                snapshot.pager.read(cx).aligned_id(),
                snapshot.stack_page.revision,
                snapshot.sound_posts.as_ref().map(|sound_posts| sound_posts.items.len()),
            )
        };
        let deleted = PostServicesStore::shared(cx).read(cx).last_own_deleted_post_id().map(str::to_owned);

        let on_aligned = state.clone();
        on_change(child_id(id, "aligned"), aligned, true, window, cx, move |_, aligned, _, cx| {
            if let Some(aligned) = aligned {
                on_aligned.update(cx, |state, cx| state.page_aligned(aligned, cx));
            }
        });
        let on_revision = state.clone();
        on_change(child_id(id, "revision"), revision, false, window, cx, move |_, _, _, cx| {
            on_revision.update(cx, |state, cx| state.stack_changed(cx));
        });
        let on_sound = state.clone();
        on_change(child_id(id, "sound-count"), sound_count, false, window, cx, move |_, _, _, cx| {
            on_sound.update(cx, |state, cx| state.stack_changed(cx));
        });
        let on_deleted = state.clone();
        on_change(child_id(id, "deleted"), deleted, false, window, cx, move |_, deleted, _, cx| {
            if let Some(deleted) = deleted {
                on_deleted.update(cx, |state, cx| state.own_post_deleted(deleted, nav_store.as_ref(), cx));
            }
        });
    }

    fn footer_page(id: &str, is_loading: bool, size: Size<Pixels>) -> AnyElement {
        let body: AnyElement = if is_loading {
            LoaderUi::new().size(LoaderUiSize::Large).color(Some(white())).into_any_element()
        } else {
            vstack(12.0)
                .px(px(28.0))
                .items_center()
                .child(MainText::new(tr("feed_load_error")).px(15.0).tac(TextAlignT::Center).color(white()))
                .child(
                    ButtonUi::new(child_id(id, "retry"))
                        .fit_content(true)
                        .padding_horizontal(18.0)
                        .on_press(|_, cx| {
                            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.retry_shorts(cx));
                        })
                        .child(MainText::new(tr("shorts_retry")).px(14.0).font_weight(FontWeight::SEMIBOLD)),
                )
                .into_any_element()
        };
        div()
            .w(size.width)
            .h(size.height)
            .flex()
            .items_center()
            .justify_center()
            .child(body)
            .into_any_element()
    }

    /// part of the page below the window bottom, implausible values count as none
    fn bottom_overhang(page_bottom_in_window: f32, window_height: f32) -> f32 {
        let overhang = page_bottom_in_window - window_height;
        if overhang > 0.0 && overhang < window_height / 4.0 { overhang } else { 0.0 }
    }

    fn stack_pager(id: &str, state: &Entity<ShortViewerPageState>, bottom_inset: f32, cx: &mut App) -> AnyElement {
        let snapshot = state.read(cx);
        let size = snapshot.size;
        if size.width <= px(0.0) || size.height <= px(0.0) {
            return div().into_any_element();
        }
        let posts = snapshot.viewer_posts.clone();
        let active_post_id = snapshot.active_post_id.clone();
        let return_rewind = Rc::new(snapshot.return_rewind.clone());
        let has_more = snapshot.stack_has_more();
        let is_loading = snapshot.stack_is_loading();
        let flight_destination = snapshot.flight_destination();
        let pager = snapshot.pager.clone();
        let progress_floor = bottom_inset + snapshot.bottom_overhang;
        let page = &snapshot.stack_page;
        let is_feed = snapshot.context == ShortViewerContext::Feed;
        let footer_loads = is_feed && (page.status == RisePostPageStatus::Loading || page.is_loading_more);
        let footer_fails = is_feed && !footer_loads && page.error_message.is_some();
        let extra_pages = usize::from(footer_loads || footer_fails);
        pager.update(cx, |pager, _| pager.update_pages(&posts, extra_pages, f32::from(size.height)));

        let page_id = id.to_owned();
        let cells = state.clone();
        let pass = state.update(cx, |state, _| state.begin_feed_cell_pass());
        ShortsVerticalPager::new(child_id(id, "pager"), pager, size, move |index, _, cx| {
            let Some(item) = posts.get(index) else {
                return Self::footer_page(&page_id, footer_loads, size);
            };
            let props = ShortViewerFeedCellProps {
                post: item.clone(),
                is_active: active_post_id == item.id,
                bottom_inset,
                progress_floor,
                flight_destination: flight_destination.clone(),
                rewind_token: return_rewind.token(&item.id),
                page_id: page_id.clone(),
                index,
                item_count: posts.len(),
                has_more,
                is_loading,
                state: cells.clone(),
            };
            let cell = cells.update(cx, |state, cx| state.feed_cell(&item.id, pass, cx));
            let stale = cell.update(cx, |cell, _| cell.set_props(props));
            if stale {
                AnyView::from(cell).into_any_element()
            } else {
                AnyView::from(cell).cached(StyleRefinement::default().size_full()).replay_when_moved().into_any_element()
            }
        })
        .into_any_element()
    }
}

/// what one riff page draws from
#[derive(Clone)]
struct ShortViewerFeedCellProps {
    post: Rc<GetPostFeedResponse>,
    is_active: bool,
    bottom_inset: f32,
    progress_floor: f32,
    flight_destination: Option<Rc<ShortViewerSharedElementDestination>>,
    rewind_token: i64,
    page_id: String,
    index: usize,
    item_count: usize,
    has_more: bool,
    is_loading: bool,
    state: Entity<ShortViewerPageState>,
}

impl ShortViewerFeedCellProps {
    fn same(&self, other: &Self) -> bool {
        (Rc::ptr_eq(&self.post, &other.post) || self.post == other.post)
            && self.is_active == other.is_active
            && self.bottom_inset == other.bottom_inset
            && self.progress_floor == other.progress_floor
            && match (&self.flight_destination, &other.flight_destination) {
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && self.rewind_token == other.rewind_token
            && self.page_id == other.page_id
            && self.index == other.index
            && self.item_count == other.item_count
            && self.has_more == other.has_more
            && self.is_loading == other.is_loading
            && self.state == other.state
    }
}

struct ShortViewerFeedCellView {
    props: Option<ShortViewerFeedCellProps>,
}

impl ShortViewerFeedCellView {
    fn set_props(&mut self, props: ShortViewerFeedCellProps) -> bool {
        if self.props.as_ref().is_some_and(|current| current.same(&props)) {
            return false;
        }
        self.props = Some(props);
        true
    }
}

impl Render for ShortViewerFeedCellView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(props) = self.props.clone() else {
            return div().into_any_element();
        };
        let load_state = props.state.clone();
        let on_auto_advance = (props.index + 1 < props.item_count).then(|| {
            let page_state = props.state.clone();
            Rc::new(move |cx: &mut App| {
                let pager = page_state.read(cx).pager.clone();
                pager.update(cx, |pager, cx| {
                    pager.step_page(1.0, cx);
                });
            }) as ShortsAutoAdvance
        });
        ShortsFeedCell::new(props.post.clone(), props.is_active, props.bottom_inset)
            .view_surface(RisePostViewSurface::ShortViewer)
            .on_auto_advance(on_auto_advance)
            .progress_floor(Some(props.progress_floor))
            .shared_element_destination(props.flight_destination.clone())
            .rewind_token(props.rewind_token)
            .post_pagination_trigger_threshold(
                child_id(&props.page_id, &format!("pagination-{}", props.post.id)),
                props.index as i64,
                props.item_count as i64,
                props.has_more,
                props.is_loading,
                ShortViewerPage::PAGINATION_THRESHOLD,
                move |cx| {
                    load_state.update(cx, |state, cx| state.load_next_page(cx));
                },
            )
            .into_any_element()
    }
}

fn child_id(page_id: &str, name: &str) -> ElementId {
    ElementId::from(SharedString::from(format!("short-viewer-{page_id}-{name}")))
}

impl RenderOnce for ShortViewerPage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let id = self.post.id.clone();
        let (post, origin, sound_track_id) = (self.post, self.origin, self.sound_track_id);
        let state = window.use_keyed_state(child_id(&id, "state"), cx, move |_, cx| {
            ShortViewerPageState::new(post, origin, sound_track_id, cx)
        });
        state.update(cx, |state, cx| state.refresh_if_settled(cx));

        let safe = EditorImageLayout::resolved_safe_area_insets(EdgeInsets::new(0.0, 0.0, 0.0, 0.0));

        Self::observe_changes(&id, &state, nav_store.clone(), window, cx);

        let (destination, flight_destination, grid_surface) = {
            let snapshot = state.read(cx);
            snapshot.shared_element_destination.update_pager(&snapshot.pager);
            (
                snapshot.shared_element_destination.clone(),
                snapshot.flight_destination(),
                snapshot.flight_grid_surface(),
            )
        };

        let task_state = state.clone();
        let task_id = id.clone();
        use_task(child_id(&id, "task"), id.clone(), window, cx, async move |cx| {
            if task_id.is_empty() {
                return;
            }
            if let Some(surface) = grid_surface {
                let settled = cx.update(|_, cx| {
                    SharedElementTransitionCoordinator::wait_until_settled(surface, FLIGHT_SETTLE_TIMEOUT, cx)
                });
                if let Ok(settled) = settled {
                    settled.await;
                }
            }
            let _ = cx.update(|_, cx| task_state.update(cx, |state, cx| state.load_stack(cx)));
        });

        let pager = Self::stack_pager(&id, &state, safe.bottom, cx);
        let back_destination = destination.clone();
        let back_store = nav_store;
        let header = div().absolute().left_0().right_0().top(px(safe.top)).child(
            CommentsSheetProgressFade::new()
                .child(
                    PageHeaderUi::new()
                        .shows_back_button(true)
                        .on_back(move |_, cx| {
                            let nav_store = back_store.clone();
                            back_destination.request_exit(
                                move |cx| {
                                    if let Some(nav_store) = nav_store {
                                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                                    }
                                },
                                cx,
                            );
                        })
                        .modifier(ShortsChromeFade::default())
                        .into_any_element(),
                )
                .shared_element_lift(ShortViewerSharedElementDestination::HEADER_ITEM_ID, SharedElementPart::Text),
        );

        let probe = flight_destination
            .as_ref()
            .and_then(|flight_destination| flight_destination.as_container())
            .map(|container| div().absolute().top_0().left_0().size_0().child(SharedElementHostProbe::destination(container)));
        let surface = flight_destination.map(|destination| SharedElementSurface::ShortViewer { session: destination.session });

        let measured = state.clone();
        let frame_destination = destination;
        let root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(black())
            .child(measure(child_id(&id, "geometry"), move |bounds, window, cx| {
                frame_destination.update_host_frame(bounds, window);
                let window_height = f32::from(window.viewport_size().height);
                let overhang = Self::bottom_overhang(f32::from(bounds.bottom()), window_height);
                measured.update(cx, |state, cx| {
                    if state.size != bounds.size {
                        state.size = bounds.size;
                        cx.notify();
                    }
                    if state.bottom_overhang != overhang {
                        state.bottom_overhang = overhang;
                        cx.notify();
                    }
                });
            }))
            .children(probe)
            .child(pager)
            .child(header);

        let on_appear = state.clone();
        let on_disappear = state;
        div()
            .size_full()
            .child(env_scope::<SharedElementSurfaceKey>(surface).child(root))
            .occludes_bottom_chrome()
            .on_appear(child_id(&id, "lifecycle"), move |_, cx| {
                on_appear.update(cx, |state, cx| state.appeared(cx));
            })
            .and_on_disappear(move |_, cx| {
                on_disappear.update(cx, |state, cx| state.disappeared(cx));
            })
    }
}
