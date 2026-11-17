use std::path::Path;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, App, AppContext, Context, Div, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, Pixels,
    Point, Render, RenderOnce, SharedString, Size, StyleRefinement, Styled, Subscription, WeakEntity, Window, black, div, px,
    size,
    white,
};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::environment::{EnvKey, env, env_scope};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{Alignment, vstack, zstack};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementReturnScrollMath, SharedElementSurface,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::SharedElementOriginContainer;
use crate::core::shared_element_transition::navigation::shared_element_navigation_transitioning::SharedElementNavigationController;
use crate::core::shared_element_transition::runtime::shared_element_host_probe::{
    SharedElementHostProbe, SharedElementStackHost, SharedElementStackHostKey,
};
use crate::core::shared_element_transition::runtime::shared_element_lift_board::{
    SharedElementLiftModifiers, SharedElementSurfaceKey,
};
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_container::{
    HoldContextMenuContainer, HoldContextMenuSourceContent,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::media_count_plate::MediaCountPlate;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementSurface;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::gifts::components::gift_shop_shelf_grid::{GiftScrollWindowProbe, GiftShopShelfRows};
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::components::post_collection_ui::{
    IndexedRandomAccessCollection, PostGridTextUi, PostPageStateUi, PostPaginationTriggerModifiers,
};
use crate::modules::post::components::post_media_upload_overlay::PostMediaUploadOverlay;
use crate::modules::post::components::post_moderation_disabled_overlay::PostModerationGridOverlay;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPagePresentation, RisePostPageScope, RisePostPageStatus, RisePostProfileKind,
};
use crate::modules::post::engine::rise_post_presentation::RisePostPresentation;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::user::engine::rise_user_block_presentation::RiseUserBlockPresentation;
use crate::modules::shorts::components::shorts_zoom_namespace::{
    ShortsZoomNamespaceId, ShortsZoomNamespaceKey, ShortsZoomSource,
};
use crate::modules::user::components::profile_content::pages::grid_posts::profile_grid_shared_element_origin::{
    ProfileGridMetrics, ProfileGridSharedElementLayout, ProfileGridSharedElementOrigin,
};
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::{
    SharedElementReturnScrollerKey, SharedElementReturnScrollerRef,
};
use crate::modules::story::components::stories_grid::stories_grid_view::{initial_container_width, remember_container_width};

/// environment flag, the grid menu acts as owner
pub struct ProfileGridMenuAsOwnerKey;

impl EnvKey for ProfileGridMenuAsOwnerKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// the profile is still loading so a tab without an owner id is pending and not empty
pub struct ProfileOwnerResolvingKey;

impl EnvKey for ProfileOwnerResolvingKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

#[derive(Clone, PartialEq)]
pub struct ProfileGridSharedElementInputs {
    pub user_id: String,
    pub tag: String,
    pub kind: RisePostProfileKind,
    pub scroller: Option<SharedElementReturnScrollerRef>,
}

pub enum ProfileThumbnailDecodePolicy {}

impl ProfileThumbnailDecodePolicy {
    pub const MAXIMUM_DISPLAY_SCALE: f32 = 3.0;

    pub fn maximum_pixel_dimension(rendered_size: Size<Pixels>, display_scale: f32) -> i64 {
        let longest_side = f32::from(rendered_size.width).max(f32::from(rendered_size.height));
        if !longest_side.is_finite() || longest_side <= 0.0 {
            return 1;
        }
        let finite_scale = if display_scale.is_finite() { display_scale } else { 1.0 };
        let bounded_scale = finite_scale.clamp(1.0, Self::MAXIMUM_DISPLAY_SCALE);
        ((longest_side * bounded_scale).ceil() as i64).max(1)
    }

    /// decode size a grid cell asks for, the rendered pixels rounded up to a ladder step
    pub fn bucketed_pixel_dimension(rendered_size: Size<Pixels>, display_scale: f32) -> i64 {
        let exact = Self::maximum_pixel_dimension(rendered_size, display_scale);
        let mut step: i64 = 64;
        loop {
            if exact <= step {
                return step;
            }
            let half_step = step + step / 2;
            if step >= 512 && exact <= half_step {
                return half_step;
            }
            step *= 2;
        }
    }
}

pub enum ProfileGridPreviewSourcePolicy {}

impl ProfileGridPreviewSourcePolicy {
    pub fn source(
        local_preview_url: Option<&str>,
        first_image: Option<&str>,
        video_thumbnail: Option<&str>,
    ) -> Option<String> {
        if let Some(local) = Self::local_file_source(local_preview_url) {
            return Some(local);
        }
        if let Some(remote) = Self::remote_source(first_image) {
            return Some(remote);
        }
        if let Some(thumbnail) = Self::remote_source(video_thumbnail).or_else(|| Self::local_file_source(video_thumbnail)) {
            return Some(thumbnail);
        }
        Self::local_file_source(first_image)
    }

    /// picture of a reposts cell, the posts own else its track cover and only when remote
    pub fn repost_source(
        local_preview_url: Option<&str>,
        first_image: Option<&str>,
        video_thumbnail: Option<&str>,
        track_cover_url: Option<&str>,
    ) -> Option<String> {
        let picture = Self::source(local_preview_url, first_image, video_thumbnail)
            .or_else(|| track_cover_url.map(str::to_owned))?;
        picture.to_lowercase().starts_with("http").then_some(picture)
    }

    fn remote_source(raw: Option<&str>) -> Option<String> {
        let source = Self::normalized(raw)?;
        let url = Url::from_string(source)?;
        let scheme = url.scheme()?.to_lowercase();
        (scheme == "http" || scheme == "https").then(|| source.to_owned())
    }

    fn local_file_source(raw: Option<&str>) -> Option<String> {
        let source = Self::normalized(raw)?;
        if is_local_absolute_path(source) {
            return Url::from_file_path(Path::new(source)).map(|url| url.standardized().absolute_string().to_owned());
        }
        let url = Url::from_string(source)?;
        if !url.is_file_url() {
            return None;
        }
        Some(url.standardized().absolute_string().to_owned())
    }

    fn normalized(raw: Option<&str>) -> Option<&str> {
        let source = trimming_whitespaces_and_newlines(raw?);
        (!source.is_empty()).then_some(source)
    }
}

struct GridPostsState {
    content_owner_id: Uuid,
    shared_element_origin: Rc<ProfileGridSharedElementOrigin>,
    seen: GridPostsSeen,
    subject: (String, String, RisePostProfileKind),
    evicted: bool,
    reloading: bool,
    host: Option<SharedElementStackHost>,
    nav_store: Option<WeakEntity<NavigationStore>>,
    scroller: Option<SharedElementReturnScrollerRef>,
    _navigation: Option<Subscription>,
    _subscriptions: [Subscription; 4],
}

/// what the grid last drew from the shared post presentations
#[derive(PartialEq)]
struct GridPostsSeen {
    page: RisePostPagePresentation,
    view_counts: Vec<i64>,
}

impl GridPostsSeen {
    fn read(user_id: &str, tag: &str, kind: RisePostProfileKind, cx: &mut App) -> Self {
        PostServicesStore::shared(cx).update(cx, |service, cx| {
            let page = service.profile_page_with_tag(user_id, tag, kind, cx);
            let view_counts =
                page.items.iter().filter(|post| post.is_riff).map(|post| service.post_view_count(post, cx)).collect();
            Self { page, view_counts }
        })
    }

    fn riff_view_counts(&self) -> Vec<(String, i64)> {
        self.page
            .items
            .iter()
            .filter(|post| post.is_riff)
            .zip(self.view_counts.iter())
            .map(|(post, count)| (post.id.clone(), *count))
            .collect()
    }
}

impl GridPostsState {
    fn new(
        user_id: String,
        tag: String,
        kind: RisePostProfileKind,
        host: Option<SharedElementStackHost>,
        nav_store: Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let pages = RisePostContentPresentation::shared(cx);
        let view_counts = RisePostPresentation::shared(cx);
        let auth = AuthServicesStore::shared(cx);
        let blocks = RiseUserBlockPresentation::shared(cx);
        let seen = GridPostsSeen::read(&user_id, &tag, kind, cx);
        let subject = (user_id.clone(), tag.clone(), kind);
        let (pages_user, pages_tag) = (user_id.clone(), tag.clone());
        let (blocks_user, blocks_tag) = (user_id.clone(), tag.clone());
        Self {
            content_owner_id: Uuid::new_v4(),
            shared_element_origin: ProfileGridSharedElementOrigin::new(),
            seen,
            subject,
            evicted: false,
            reloading: false,
            host,
            nav_store: nav_store.as_ref().map(Entity::downgrade),
            scroller: None,
            _navigation: nav_store.map(|nav_store| {
                cx.observe_in(&nav_store, window, |state, _, window, cx| state.schedule_reobserve_after_draw(window, cx))
            }),
            _subscriptions: [
                cx.observe(&pages, move |state, _, cx| state.refresh(&pages_user, &pages_tag, kind, cx)),
                cx.observe(&view_counts, move |state, _, cx| state.refresh(&user_id, &tag, kind, cx)),
                cx.observe(&auth, |_, _, cx| cx.notify()),
                cx.observe(&blocks, move |state, _, cx| state.refresh(&blocks_user, &blocks_tag, kind, cx)),
            ],
        }
    }

    fn refresh(&mut self, user_id: &str, tag: &str, kind: RisePostProfileKind, cx: &mut Context<Self>) {
        let seen = GridPostsSeen::read(user_id, tag, kind, cx);
        if seen != self.seen {
            let was_shown = self.seen.page.status != RisePostPageStatus::Idle || !self.seen.page.items.is_empty();
            let is_idle = seen.page.status == RisePostPageStatus::Idle && seen.page.items.is_empty();
            let answered = !seen.page.items.is_empty() || seen.page.error_message.is_some();
            self.seen = seen;
            if was_shown && is_idle {
                self.evicted = true;
                self.schedule_reobserve(cx);
            }
            cx.notify();
            if self.reloading && answered {
                self.reloading = false;
                cx.refresh_windows();
            }
        }
    }

    fn schedule_reobserve(&mut self, cx: &mut Context<Self>) {
        if !self.evicted {
            return;
        }
        let state = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = state.update(cx, |state, cx| state.reobserve_if_shown(cx));
        });
    }

    fn schedule_reobserve_after_draw(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.evicted {
            return;
        }
        self.schedule_reobserve(cx);
        cx.on_next_frame(window, |_, window, cx| {
            cx.on_next_frame(window, |state, _, cx| state.reobserve_if_shown(cx));
        });
    }

    /// a page that waited under the top asks again once its on top
    /// hidden pages wait so ten profiles under the top dont evict each other in turn
    fn reobserve_if_shown(&mut self, cx: &mut Context<Self>) {
        if !self.evicted || !self.is_shown(cx) {
            return;
        }
        self.evicted = false;
        let (user_id, tag, kind) = self.subject.clone();
        if user_id.is_empty() {
            return;
        }
        self.reloading = true;
        let owner_id = self.content_owner_id;
        if let Some(scroller) = self.scroller.as_ref().and_then(|scroller| Some((scroller.scroller()?, scroller.tab_key))) {
            scroller.0.reset_to_content_top(scroller.1, cx);
        }
        PostInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.observe_profile(&user_id, &tag, kind, owner_id, cx);
        });
    }

    fn is_shown(&self, cx: &App) -> bool {
        let Some(host) = self.host else {
            return true;
        };
        let Some(nav_store) = self.nav_store.as_ref().and_then(WeakEntity::upgrade) else {
            return true;
        };
        let Some(stack) = nav_store.read(cx).navigation_window_stack() else {
            return true;
        };
        stack.top_view_controller(cx).is_none_or(|top| top == host)
    }
}

#[derive(Clone, IntoElement)]
pub struct GridPosts {
    pub profile_tag: String,
    pub profile_user_id: String,
    pub kind: RisePostProfileKind,
}

struct GridPostsFrame {
    grid: GridPosts,
    origin: Rc<ProfileGridSharedElementOrigin>,
    nav_store: Option<Entity<NavigationStore>>,
    shorts_zoom_namespace: Option<ShortsZoomNamespaceId>,
    menu_as_owner: bool,
    viewer_acts_as_author: bool,
    anchor_surface: Option<SharedElementSurface>,
    item_count: i64,
    has_more: bool,
    is_loading_more: bool,
    cell_width: f32,
    cell_height: f32,
    view_counts: Rc<Vec<(String, i64)>>,
}

impl GridPosts {
    pub fn new(profile_tag: impl Into<String>, profile_user_id: impl Into<String>, kind: RisePostProfileKind) -> Self {
        Self { profile_tag: profile_tag.into(), profile_user_id: profile_user_id.into(), kind }
    }

    fn child_id(&self, name: &str) -> ElementId {
        ElementId::Name(SharedString::from(format!(
            "grid-posts-{name}-{}-{}-{}",
            self.profile_user_id,
            self.profile_tag,
            self.kind.wire()
        )))
    }

    fn cell_id(&self, name: &str, presentation_id: &str) -> ElementId {
        self.child_id(&format!("{name}-{presentation_id}"))
    }

    #[allow(clippy::too_many_arguments)]
    fn grid_body(
        grid: GridPosts,
        posts: Rc<Vec<GetPostFeedResponse>>,
        has_more: bool,
        is_loading_more: bool,
        height: f32,
        anchor_surface: Option<SharedElementSurface>,
        origin: Rc<ProfileGridSharedElementOrigin>,
        width: f32,
        view_counts: Rc<Vec<(String, i64)>>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let w = width.max(0.0);
        let columns = ProfileGridMetrics::columns(w);
        let cell_w = ProfileGridMetrics::cell_width(w);
        let cell_h = ProfileGridMetrics::row_height(cell_w);
        let item_count = posts.len() as i64;

        let row_count = GiftShopShelfRows::count(item_count, columns);
        let initial_rows =
            (f32::from(window.viewport_size().height) / (cell_h + ProfileGridMetrics::SPACING)).ceil() as i64 + 1;
        let rows_state: Entity<Option<GiftShopShelfRows>> =
            window.use_keyed_state(grid.child_id("rows"), cx, |_, _| None);
        let rows = rows_state.read(cx).unwrap_or(GiftShopShelfRows::leading(initial_rows)).clamped(row_count);
        let range = rows.items(columns, item_count);

        let frame = Rc::new(GridPostsFrame {
            grid: grid.clone(),
            origin: origin.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
            shorts_zoom_namespace: env::<ShortsZoomNamespaceKey>(cx),
            menu_as_owner: env::<ProfileGridMenuAsOwnerKey>(cx),
            viewer_acts_as_author: AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author(),
            anchor_surface: anchor_surface.clone(),
            item_count,
            has_more,
            is_loading_more,
            cell_width: cell_w,
            cell_height: cell_h,
            view_counts,
        });

        let indexed = IndexedRandomAccessCollection::new(posts.as_slice());
        let mut cells: Vec<AnyElement> = Vec::with_capacity((range.end - range.start).max(0) as usize);
        for position in range.start as usize..range.end as usize {
            let Some(entry) = indexed.at(position) else { continue };
            let rect = SharedElementReturnScrollMath::grid_item_rect(
                position,
                columns as usize,
                cell_w,
                cell_h,
                ProfileGridMetrics::SPACING,
                0.0,
            );
            cells.push(
                div()
                    .absolute()
                    .left(rect.origin.x)
                    .top(rect.origin.y)
                    .w(rect.size.width)
                    .h(rect.size.height)
                    .child(Self::cell(&frame, entry.element, entry.offset, window, cx))
                    .into_any_element(),
            );
        }

        let probe_rows = rows_state;
        let window_probe = GiftScrollWindowProbe::new(grid.child_id("window-probe"), move |band, band_height, cx| {
            let Some(band) = band else { return };
            if band_height <= 0.0 {
                return;
            }
            probe_rows.update(cx, |mounted, cx| {
                let next = GiftShopShelfRows::following(
                    &band,
                    band_height,
                    row_count,
                    ProfileGridMetrics::SPACING,
                    mounted.unwrap_or(GiftShopShelfRows::leading(initial_rows)),
                );
                if Some(next) != *mounted {
                    *mounted = Some(next);
                    cx.notify();
                }
            });
        });

        let host_probe = anchor_surface.is_some().then(|| {
            let container: Rc<dyn SharedElementOriginContainer> = origin.clone();
            SharedElementHostProbe::origin(container).on_layout(move |probe, window, _| {
                origin.record_columns(columns);
                origin.record_layout(probe, cell_w, None, 0.0, window);
            })
        });

        div()
            .relative()
            .w(px(w))
            .h(px(height))
            .children(host_probe)
            .child(env_scope::<SharedElementSurfaceKey>(anchor_surface).child(div().absolute().inset_0().children(cells)))
            .child(div().absolute().inset_0().child(window_probe))
            .into_any_element()
    }

    fn cell(
        frame: &Rc<GridPostsFrame>,
        post: &GetPostFeedResponse,
        index: i64,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let grid = &frame.grid;
        let presentation_id = post.presentation_id();
        let is_locked = post.shows_upload_overlay();
        let mine = !is_locked && ProfileGridPostMenu::is_mine(post, frame.menu_as_owner, cx);
        let view_count = post.is_riff.then(|| {
            match frame.view_counts.iter().find(|(id, _)| *id == post.id) {
                Some((_, count)) => *count,
                None => PostServicesStore::shared(cx).update(cx, |service, cx| service.post_view_count(post, cx)),
            }
        });

        let cell_post = Rc::new(post.clone());
        let impression_post = cell_post.clone();
        let (cell_width, cell_height) = (frame.cell_width, frame.cell_height);
        let props = ProfileGridPostCellProps {
            post: cell_post,
            cell_width,
            cell_height,
            view_count,
            is_locked,
            mine,
            viewer_acts_as_author: frame.viewer_acts_as_author,
            grid: grid.clone(),
            origin: frame.origin.clone(),
            nav_store: frame.nav_store.clone(),
        };
        let body = div()
            .w(px(cell_width))
            .h(px(cell_height))
            .child(ProfileGridPostCellView::element(grid.cell_id("cell-view", &presentation_id), props, window, cx))
            .on_appear(grid.cell_id("appear", &presentation_id), move |_: &mut Window, cx: &mut App| {
                Self::fire_grid_impression(&impression_post, cx);
            });

        let load_grid = grid.clone();
        let load_surface = frame.anchor_surface.clone();
        grid.grid_zoom_source(post, frame.shorts_zoom_namespace)
            .body(body)
            .post_pagination_trigger(
                grid.cell_id("pagination", &presentation_id),
                index,
                frame.item_count,
                frame.has_more,
                frame.is_loading_more,
                move |cx| load_grid.load_more(load_surface.clone(), cx),
            )
            .into_any_element()
    }

    fn refresh(&self, cx: &mut App) {
        PostInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.refresh_profile(&self.profile_user_id, &self.profile_tag, self.kind, cx))
            .detach();
    }

    /// anchors and host probe only exist where a flight can run, threads reuse this grid
    fn shared_elements_active(&self) -> bool {
        SharedElementNavigationPolicy::flights_enabled() && SharedElementNavigationPolicy::ENABLED_KINDS.contains(&self.kind)
    }

    fn shared_element_inputs(&self, scroller: Option<SharedElementReturnScrollerRef>) -> ProfileGridSharedElementInputs {
        ProfileGridSharedElementInputs {
            user_id: self.profile_user_id.clone(),
            tag: self.profile_tag.clone(),
            kind: self.kind,
            scroller,
        }
    }

    fn configure_shared_element_origin(
        origin: &ProfileGridSharedElementOrigin,
        inputs: &ProfileGridSharedElementInputs,
    ) {
        origin.configure(
            &inputs.user_id,
            &inputs.tag,
            inputs.kind,
            inputs.scroller.clone(),
            ProfileGridSharedElementLayout::Posts,
        );
    }

    /// appending never moves earlier cells but still waits for a flight on this grid
    fn load_more(&self, deferred_until_settled: Option<SharedElementSurface>, cx: &mut App) {
        let (user_id, tag, kind) = (self.profile_user_id.clone(), self.profile_tag.clone(), self.kind);
        let load = move |cx: &mut App| {
            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_profile(&user_id, &tag, kind, cx));
        };
        let Some(surface) = deferred_until_settled else {
            load(cx);
            return;
        };
        SharedElementTransitionCoordinator::run_or_defer_until_settled(surface, load, cx);
    }

    fn grid_zoom_source(
        &self,
        post: &GetPostFeedResponse,
        shorts_zoom_namespace: Option<ShortsZoomNamespaceId>,
    ) -> ShortsZoomSource {
        let pushed = self.route(post);
        let namespace = if Self::uses_shared_element_navigation(&pushed) { None } else { shorts_zoom_namespace };
        ShortsZoomSource::new(pushed, namespace)
    }

    fn uses_shared_element_navigation(route: &RootRoute) -> bool {
        let RootRoute::PostDetail { origin, .. } = route else { return false };
        SharedElementNavigationPolicy::uses_shared_element_navigation_post_detail_origin(origin.as_ref())
    }

    fn route(&self, post: &GetPostFeedResponse) -> RootRoute {
        RootRoute::post_content(
            post,
            Some(RisePostPageScope::Profile {
                user_id: self.profile_user_id.clone(),
                tag: self.profile_tag.clone(),
                kind: self.kind,
            }),
        )
    }

    fn open_post(&self, post: &GetPostFeedResponse, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            let route = self.route(post);
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn fire_grid_impression(post: &GetPostFeedResponse, cx: &mut App) {
        let post_id = post.rise_view_target_post_id();
        let author_id = post.author.id.clone().unwrap_or_else(|| post.author_id.clone());
        let is_short = post.is_short;
        EngagementInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.impression(&post_id, &author_id, EngagementSurface::ProfileGrid, -1, is_short, 500, cx);
        });
    }

    fn row_count(count: i64, grid_width: f32) -> i64 {
        ProfileGridMetrics::row_count(count, grid_width)
    }
}

impl RenderOnce for GridPosts {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (user_id, tag, kind) = (self.profile_user_id.clone(), self.profile_tag.clone(), self.kind);
        let stack_host = env::<SharedElementStackHostKey>(cx);
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(self.child_id("state"), cx, |window, cx| {
            GridPostsState::new(user_id.clone(), tag.clone(), kind, stack_host, nav_store, window, cx)
        });
        let content_owner_id = state.read(cx).content_owner_id;
        let shared_element_origin = state.read(cx).shared_element_origin.clone();

        let initial_width = initial_container_width(window, cx);
        let width_state: Entity<f32> = window.use_keyed_state(self.child_id("width"), cx, move |_, _| initial_width);
        let grid_width = *width_state.read(cx);

        let seen = {
            let current = state.read(cx);
            (current.subject == (user_id.clone(), tag.clone(), kind))
                .then(|| (current.seen.page.clone(), current.seen.riff_view_counts()))
        };
        let (page, view_counts) = match seen {
            Some(seen) => seen,
            None => (
                PostServicesStore::shared(cx)
                    .update(cx, |service, cx| service.profile_page_with_tag(&user_id, &tag, kind, cx)),
                Vec::new(),
            ),
        };
        let view_counts = Rc::new(view_counts);
        let RisePostPagePresentation { items, status, error_message, has_more, is_loading_more, .. } = page;
        let posts = Rc::new(items);
        let is_empty = posts.is_empty();
        let rows = Self::row_count(posts.len() as i64, grid_width);
        let row_h = ProfileGridMetrics::row_height_container_width(grid_width);
        let grid_h = ProfileGridMetrics::grid_height(rows, row_h);
        let has_subject = !user_id.is_empty() || env::<ProfileOwnerResolvingKey>(cx);
        let outer_height = ProfileGridMetrics::outer_height(grid_h, row_h, status, !is_empty, has_subject);
        let shared_element_surface = ProfileGridSharedElementOrigin::surface(&user_id, &tag, kind);
        let anchor_surface = self.shared_elements_active().then(|| shared_element_surface.clone());

        let inputs = self.shared_element_inputs(env::<SharedElementReturnScrollerKey>(cx));
        if state.read(cx).scroller != inputs.scroller {
            let scroller = inputs.scroller.clone();
            state.update(cx, |state, _| state.scroller = scroller);
        }
        let configure_origin = shared_element_origin.clone();
        on_change(self.child_id("inputs"), inputs, true, window, cx, move |_, inputs, _, _| {
            Self::configure_shared_element_origin(&configure_origin, inputs);
        });

        let (task_user, task_tag) = (user_id.clone(), tag.clone());
        use_task(
            self.child_id("observe"),
            (user_id.clone(), tag.to_lowercase(), kind),
            window,
            cx,
            async move |cx| {
                let settled = cx.update(|_, cx| {
                    SharedElementTransitionCoordinator::wait_until_settled(shared_element_surface, 1.0, cx)
                });
                if let Ok(settled) = settled {
                    settled.await;
                }
                if task_user.is_empty() {
                    return;
                }
                let _ = cx.update(|_, cx| {
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.observe_profile(&task_user, &task_tag, kind, content_owner_id, cx);
                    });
                });
            },
        );

        let empty_icon = if kind == RisePostProfileKind::Threads { "text.bubble" } else { "photo.on.rectangle.angled" };
        let empty_text =
            tr(if kind == RisePostProfileKind::Threads { "profile_threads_empty" } else { "profile_publications_empty" });
        let error_text = error_message.unwrap_or_else(|| tr("feed_load_error"));

        let retry_grid = self.clone();
        let content_grid = self.clone();
        let state_ui = PostPageStateUi::new(
            self.child_id("page-state"),
            status,
            !is_empty,
            empty_icon,
            empty_text,
            error_text,
            move |cx: &mut App| retry_grid.refresh(cx),
        )
        .has_subject(has_subject)
        .loading(move |_: &mut Window, _: &mut App| {
            PostGridSkeleton::default()
                .columns(ProfileGridMetrics::columns(grid_width))
                .spacing(ProfileGridMetrics::SPACING)
                .cell_height(row_h)
                .rows(ProfileGridMetrics::SKELETON_ROWS)
        })
        .content(move |window: &mut Window, cx: &mut App| {
            Self::grid_body(
                content_grid,
                posts,
                has_more,
                is_loading_more,
                grid_h,
                anchor_surface,
                shared_element_origin,
                grid_width,
                view_counts,
                window,
                cx,
            )
        });

        let reader_state = width_state;
        let width_reader = measure(self.child_id("width-reader"), move |bounds, _, cx| {
            let width = f32::from(bounds.size.width).max(0.0);
            remember_container_width(width);
            reader_state.update(cx, |current, cx| {
                if *current != width {
                    *current = width;
                    cx.notify();
                }
            });
        });

        let disappear_id = self.child_id("disappear");
        div()
            .relative()
            .flex()
            .flex_col()
            .justify_center()
            .w_full()
            .h(px(outer_height))
            .child(width_reader)
            .child(state_ui)
            .on_disappear(disappear_id, move |_: Option<&mut Window>, cx: &mut App| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.stop_observing_profile(&user_id, &tag, kind, content_owner_id, cx);
                });
            })
    }
}

/// hold menu of a profile grid cell,
/// same actions in publications,
/// threads and riffs
pub enum ProfileGridPostMenu {}

impl ProfileGridPostMenu {
    /// coauthored or tagged post on your own profile still counts as someone elses post
    pub fn owner_menu_applies(post: &GetPostFeedResponse, menu_as_owner: bool) -> bool {
        menu_as_owner && post.my_collaboration.is_none()
    }

    pub fn is_mine(post: &GetPostFeedResponse, menu_as_owner: bool, cx: &mut App) -> bool {
        if Self::owner_menu_applies(post, menu_as_owner) {
            return true;
        }
        let auth = AuthServicesStore::shared(cx).read(cx);
        auth.is_current_user(&post.author_id) || auth.is_current_user(post.author.id.as_deref().unwrap_or(""))
    }

    pub fn actions(post: &GetPostFeedResponse, mine: bool, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let viewer_acts_as_author = AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author();
        Self::actions_for(post, mine, viewer_acts_as_author)
    }

    /// builds the menu from facts read up front so the cell can do it only on open
    pub fn actions_for(post: &GetPostFeedResponse, mine: bool, viewer_acts_as_author: bool) -> Vec<HoldContextMenuAction> {
        let acts_as_author = mine || viewer_acts_as_author;
        let mut items = vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
        ];
        if !mine {
            let title = if post.is_reposted { tr("repost_delete_action") } else { tr("repost_action") };
            items.push(HoldContextMenuAction::new("repost", title).system_image("arrow.2.squarepath"));
        }
        if acts_as_author {
            items.push(HoldContextMenuAction::new("stats", tr("post_stats_menu_item")).system_image("chart.bar"));
            items.push(HoldContextMenuAction::new("edit", tr("ctx_edit")).system_image("pencil"));
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

    pub fn handle(
        id: &str,
        post: &GetPostFeedResponse,
        nav_store: Option<&Entity<NavigationStore>>,
        open: impl FnOnce(&mut App),
        cx: &mut App,
    ) {
        match id {
            "open" => open(cx),
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
            "stats" => {
                if let Some(nav_store) = nav_store {
                    let route = RootRoute::PostStats { post_id: post.rise_view_target_post_id() };
                    nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            }
            "edit" => {
                if let Some(nav_store) = nav_store {
                    let route = RootRoute::EditPost { post: post.clone() };
                    nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            }
            "delete" => {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.delete_own_post(post, None, cx));
            }
            "repost" => {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_repost(post, None, cx));
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

/// what one post cell draws and answers, equal props keep the last frame
#[derive(Clone)]
struct ProfileGridPostCellProps {
    post: Rc<GetPostFeedResponse>,
    cell_width: f32,
    cell_height: f32,
    view_count: Option<i64>,
    is_locked: bool,
    mine: bool,
    viewer_acts_as_author: bool,
    grid: GridPosts,
    origin: Rc<ProfileGridSharedElementOrigin>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl PartialEq for ProfileGridPostCellProps {
    fn eq(&self, other: &Self) -> bool {
        self.post == other.post
            && self.cell_width == other.cell_width
            && self.cell_height == other.cell_height
            && self.view_count == other.view_count
            && self.is_locked == other.is_locked
            && self.mine == other.mine
            && self.viewer_acts_as_author == other.viewer_acts_as_author
            && self.grid.profile_tag == other.grid.profile_tag
            && self.grid.profile_user_id == other.grid.profile_user_id
            && self.grid.kind == other.grid.kind
            && Rc::ptr_eq(&self.origin, &other.origin)
            && self.nav_store == other.nav_store
    }
}

/// one post cell as its own cached view
/// page host rerenders on every wheel step so it keeps only a slot per cell
struct ProfileGridPostCellView {
    props: Option<ProfileGridPostCellProps>,
}

struct ProfileGridPostCellHolder {
    cell: Entity<ProfileGridPostCellView>,
}

impl ProfileGridPostCellView {
    fn element(id: ElementId, props: ProfileGridPostCellProps, window: &mut Window, cx: &mut App) -> AnyElement {
        let style = StyleRefinement::default().w(px(props.cell_width)).h(px(props.cell_height));
        let host = window
            .use_keyed_state(id, cx, |_, cx| ProfileGridPostCellHolder { cell: cx.new(|_| Self { props: None }) })
            .read(cx)
            .cell
            .clone();
        let stale = host.update(cx, |host, _| host.set_props(props));
        if stale {
            AnyView::from(host).into_any_element()
        } else {
            host.cached(style).replay_when_moved().into_any_element()
        }
    }

    fn set_props(&mut self, props: ProfileGridPostCellProps) -> bool {
        if self.props.as_ref() == Some(&props) {
            return false;
        }
        let drawn_before = self.props.is_some();
        self.props = Some(props);
        drawn_before
    }

    fn hold(props: ProfileGridPostCellProps) -> HoldContextMenuContainer {
        let ProfileGridPostCellProps {
            post,
            cell_width,
            cell_height,
            view_count,
            is_locked,
            mine,
            viewer_acts_as_author,
            grid,
            origin,
            nav_store,
        } = props;
        let presentation_id = post.presentation_id();
        let content_post = post.clone();
        let content: HoldContextMenuSourceContent = Rc::new(move |_: &mut Window, _: &mut App| {
            div()
                .w(px(cell_width))
                .h(px(cell_height))
                .when(!is_locked, |cell| cell.cursor_pointer().hover(|cell| cell.opacity(0.92)))
                .child(ProfileGridPostCell::new(content_post.clone(), cell_width, cell_height).view_count(view_count))
                .shared_element_anchor(content_post.id.clone(), ProfileGridSharedElementOrigin::part(&content_post))
                .into_any_element()
        });

        let menu_post = post.clone();
        let (action_post, action_grid, action_origin, action_nav) =
            (post.clone(), grid.clone(), origin.clone(), nav_store.clone());
        let (tap_post, tap_grid, tap_origin, tap_nav) = (post, grid.clone(), origin, nav_store);
        HoldContextMenuContainer::new(
            grid.cell_id("hold", &presentation_id),
            HoldContextMenuAlignment::Leading,
            Rc::new(move |_: &App| {
                if is_locked {
                    return Vec::new();
                }
                ProfileGridPostMenu::actions_for(&menu_post, mine, viewer_acts_as_author)
            }),
            content,
        )
        .show_emoji_list(false)
        .tap_movement_tolerance(4.0)
        .on_action(Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            let (grid, origin, nav, post) =
                (action_grid.clone(), action_origin.clone(), action_nav.clone(), action_post.clone());
            ProfileGridPostMenu::handle(
                &id,
                &action_post,
                action_nav.as_ref(),
                move |cx: &mut App| {
                    if origin.arm_open_if_possible(&post, cx) {
                        HoldContextMenuPresenter::dismiss(cx);
                    }
                    grid.open_post(&post, nav.as_ref(), cx);
                },
                cx,
            );
        }))
        .on_tap(Rc::new(move |_: Point<Pixels>, _: &mut Window, cx: &mut App| {
            if is_locked {
                return;
            }
            tap_origin.arm_open_if_possible(&tap_post, cx);
            tap_grid.open_post(&tap_post, tap_nav.as_ref(), cx);
        }))
    }
}

impl Render for ProfileGridPostCellView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (width, height) = self.props.as_ref().map_or((0.0, 0.0), |props| (props.cell_width, props.cell_height));
        let hold = self.props.clone().map(Self::hold);
        div().w(px(width)).h(px(height)).children(hold)
    }
}

#[derive(IntoElement)]
pub struct ProfileGridPostCell {
    pub post: Rc<GetPostFeedResponse>,
    pub cell_width: f32,
    pub cell_height: f32,
    pub view_count: Option<i64>,
}

impl ProfileGridPostCell {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>, cell_width: f32, cell_height: f32) -> Self {
        Self { post: post.into(), cell_width, cell_height, view_count: None }
    }

    pub fn view_count(mut self, view_count: Option<i64>) -> Self {
        self.view_count = view_count;
        self
    }

    fn corner_mark(symbol: &str, side: f32) -> Div {
        div()
            .absolute()
            .top(px(6.0))
            .p(px(6.0))
            .circle()
            .bg(black().opacity(0.42))
            .children(IconUi::sized(symbol, px(side), white(), false))
    }
}

impl RenderOnce for ProfileGridPostCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let post = self.post;
        let (cell_width, cell_height) = (self.cell_width, self.cell_height);
        let is_own_post = PostMediaCachePolicy::is_own_post(&post, cx);
        let preview_source = ProfileGridPreviewSourcePolicy::source(
            post.local_preview_url.as_deref(),
            post.images.first().map(String::as_str),
            post.video_thumbnail.as_deref(),
        );
        let cell_size = size(px(cell_width), px(cell_height));
        let maximum_decode_pixel_dimension =
            ProfileThumbnailDecodePolicy::bucketed_pixel_dimension(cell_size, window.scale_factor());

        let picture: AnyElement = match preview_source {
            Some(preview_source) => CleverImageUi::new(preview_source)
                .content_mode(ContentMode::Fill)
                .corner_radius(0.0)
                .persist_to_disk(is_own_post)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(maximum_decode_pixel_dimension)
                .cache_policy(PostMediaCachePolicy::image_policy(is_own_post))
                .into_any_element(),
            None => PostGridTextUi::new(post.clone()).into_any_element(),
        };

        let shows_track_mark = post.track_id.is_some() && !post.is_short && !post.shows_upload_overlay();
        let shows_upload_overlay = post.shows_upload_overlay();
        let moderation_banner = post.moderation_banner();
        let coauthors_count = post.coauthors_count;
        let view_count = self.view_count;

        zstack(Alignment::Center)
            .relative()
            .w(px(cell_width))
            .h(px(cell_height))
            .overflow_hidden()
            .child(div().w(px(cell_width)).h(px(cell_height)).overflow_hidden().child(picture))
            .when(post.is_short, |cell| cell.child(Self::corner_mark("play.fill", 13.0).right(px(6.0))))
            .when(post.is_image_reel, |cell| cell.child(Self::corner_mark("photo.stack", 12.0).right(px(6.0))))
            .when(shows_track_mark, |cell| cell.child(Self::corner_mark("music.note", 12.0).left(px(6.0))))
            .when(coauthors_count > 0 || view_count.is_some(), |cell| {
                cell.child(
                    vstack(3.0)
                        .absolute()
                        .bottom(px(6.0))
                        .left(px(6.0))
                        .items_start()
                        .when(coauthors_count > 0, |plates| {
                            plates.child(
                                div()
                                    .px(px(7.0))
                                    .py(px(5.0))
                                    .capsule()
                                    .bg(black().opacity(0.42))
                                    .children(IconUi::sized("person.2.fill", px(11.0), white(), false)),
                            )
                        })
                        .when_some(view_count, |plates, view_count| {
                            plates.child(MediaCountPlate::view_count(view_count))
                        }),
                )
            })
            .when(shows_upload_overlay, |cell| {
                cell.child(
                    div().absolute().inset_0().child(PostMediaUploadOverlay::new(post.clone()).cell_size(cell_size)),
                )
            })
            .when_some(moderation_banner, |cell, banner| {
                cell.child(div().absolute().inset_0().child(PostModerationGridOverlay::new(banner)))
            })
    }
}
