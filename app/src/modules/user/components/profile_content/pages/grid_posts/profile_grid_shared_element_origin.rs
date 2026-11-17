use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, AppContext, Bounds, Context, IntoElement, ParentElement, Pixels, Render,
    Size, Styled, Window, div, px,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::guide_anchor_ui::guide_scroll_host::GuideScrollHost;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementGeometry, SharedElementKey, SharedElementPart, SharedElementReturnScrollMath,
    SharedElementReturnScrollMathBand, SharedElementReturnScrollMathClamp, SharedElementSurface,
};
use crate::core::shared_element_transition::navigation::shared_element_navigation_driver::SharedElementNavigationDriver;
use crate::core::shared_element_transition::renderer::shared_element_bitmap_resolver::{
    SharedElementBitmapResolver, SharedElementBitmapResolverBitmap,
};
use crate::core::shared_element_transition::renderer::shared_element_cover_rasterizer::SharedElementCoverRasterizer;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementCover, SharedElementCoverContent, SharedElementFlightContent,
    SharedElementOriginContainer, SharedElementReturnPlan, SharedElementReturnRequest, SharedElementReturnResolution,
    SharedElementReturnScroller, SharedElementReturnViewport, SharedElementTargetResolution,
};
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementStackHost;
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::ui::clever_image_disk_cache::{CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsets;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostPageScope, RisePostPageStatus, RisePostProfileKind,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_list_engine_policy::PostListEnginePolicy;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::shorts_media_prefetch::ShortsMediaPrefetchPolicy;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::{
    ProfileGridPreviewSourcePolicy, ProfileThumbnailDecodePolicy,
};
use crate::modules::user::components::profile_content::pages::grid_posts::post_row_warm_anchor_hooks::post_row_warm_anchor_hooks;
use crate::modules::user::components::profile_content::pages::grid_riffs::grid_riffs::{
    ProfileRiffCoverSourcePolicy, RiffsGridMetrics,
};
use crate::modules::user::components::profile_content::profile_content::ProfileViewportLayoutPolicy;
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::SharedElementReturnScrollerRef;

/// fixed geometry of the profile post grid
pub enum ProfileGridMetrics {}

impl ProfileGridMetrics {
    pub const CELL_HEIGHT: f32 = 150.0;
    pub const COLUMNS: i64 = 3;
    pub const MAX_COLUMNS: i64 = 5;
    pub const WIDE_CELL_WIDTH: f32 = 180.0;
    pub const SPACING: f32 = 2.0;
    pub const SKELETON_ROWS: i64 = 2;
    pub const MINIMUM_OUTER_HEIGHT: f32 = 220.0;

    /// three columns when narrow, up to five in a wide center
    pub fn columns(container_width: f32) -> i64 {
        if !container_width.is_finite() {
            return Self::COLUMNS;
        }
        let fit = ((container_width + Self::SPACING) / (Self::WIDE_CELL_WIDTH + Self::SPACING)).floor() as i64;
        fit.clamp(Self::COLUMNS, Self::MAX_COLUMNS)
    }

    pub fn cell_width(container_width: f32) -> f32 {
        let columns = Self::columns(container_width);
        let total_spacing = Self::SPACING * (columns - 1) as f32;
        ((container_width - total_spacing) / columns as f32).max(0.0)
    }

    pub fn row_count(items: i64, container_width: f32) -> i64 {
        let columns = Self::columns(container_width);
        (items.max(0) + columns - 1) / columns
    }

    pub fn row_height(cell_width: f32) -> f32 {
        if !cell_width.is_finite() {
            return Self::CELL_HEIGHT;
        }
        Self::CELL_HEIGHT.max(cell_width)
    }

    pub fn row_height_container_width(container_width: f32) -> f32 {
        Self::row_height(Self::cell_width(container_width))
    }

    pub fn grid_height(rows: i64, row_height: f32) -> f32 {
        rows as f32 * row_height + (rows - 1).max(0) as f32 * Self::SPACING
    }

    /// frame height of the grid page, holds a skeleton whole so it doesnt spill over the tabs
    pub fn outer_height(
        grid_height: f32,
        row_height: f32,
        status: RisePostPageStatus,
        has_items: bool,
        has_subject: bool,
    ) -> f32 {
        let shows_skeleton = has_subject
            && !has_items
            && (status == RisePostPageStatus::Idle || status == RisePostPageStatus::Loading);
        let skeleton_height = if shows_skeleton { Self::grid_height(Self::SKELETON_ROWS, row_height) } else { 0.0 };
        grid_height.max(skeleton_height).max(Self::MINIMUM_OUTER_HEIGHT)
    }
}

/// what differs between the profile grids one origin type serves
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileGridSharedElementLayout {
    Posts,
    Riffs,
}

impl ProfileGridSharedElementLayout {
    pub fn spacing(self) -> f32 {
        match self {
            Self::Posts => ProfileGridMetrics::SPACING,
            Self::Riffs => RiffsGridMetrics::SPACING,
        }
    }

    /// true for tabs of this layout that may fly
    pub fn flies(self, kind: RisePostProfileKind) -> bool {
        match self {
            Self::Posts => SharedElementNavigationPolicy::ENABLED_KINDS.contains(&kind),
            Self::Riffs => SharedElementNavigationPolicy::VIEWER_KINDS.contains(&kind),
        }
    }

    /// true if the route is one this layout flies to,
    /// posts grid to the list and riff-only rows to the viewer
    pub fn route_flies(self, route: &RootRoute) -> bool {
        match (self, route) {
            (Self::Posts, RootRoute::PostDetail { .. }) => true,
            (Self::Posts | Self::Riffs, RootRoute::ShortViewer { .. }) => {
                SharedElementNavigationPolicy::uses_shared_element_navigation_route(route)
            }
            _ => false,
        }
    }
}

struct PendingReturn {
    epoch: u64,
    item_id: String,
    scroller: Weak<dyn SharedElementReturnScroller>,
    kept_frame: Option<Bounds<Pixels>>,
    keeps_position: bool,
    prepared_item_rect: Bounds<Pixels>,
}

/// flown copy of a cell, gpui cant snapshot a region so the cell is drawn again inside the flight
struct ProfileGridCellReplicant {
    content: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>,
}

impl Render for ProfileGridCellReplicant {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app: &mut App = cx;
        div().size_full().overflow_hidden().child((self.content)(window, app))
    }
}

/// shared element origin of a profile grid, open capture and return handling
/// through the profile scroll coordinator
pub struct ProfileGridSharedElementOrigin {
    this: Weak<Self>,
    surface: RefCell<SharedElementSurface>,
    cell_width: Cell<Option<f32>>,
    drawn_columns: Cell<usize>,
    grid_content_origin_y: Cell<Option<f32>>,
    user_id: RefCell<String>,
    tag: RefCell<String>,
    kind: Cell<Option<RisePostProfileKind>>,
    layout: Cell<ProfileGridSharedElementLayout>,
    measured_cell_height: Cell<Option<f32>>,
    grid_origin_x: Cell<f32>,
    scroller_ref: RefCell<Option<SharedElementReturnScrollerRef>>,
    display_scale: Cell<Option<f32>>,
    probe_window: Cell<Option<AnyWindowHandle>>,
    probe_window_size: Cell<Option<Size<Pixels>>>,
    arming_post: RefCell<Option<GetPostFeedResponse>>,
    armed_route_opens_viewer: Cell<bool>,
    pending_return: RefCell<Option<PendingReturn>>,
    opened_cell: RefCell<Option<(String, Bounds<Pixels>)>>,
    opened_item_id: RefCell<Option<String>>,
}

impl ProfileGridSharedElementOrigin {
    const REPLICANT_IDENTITY: &'static str = "replicant";

    pub fn new() -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            surface: RefCell::new(SharedElementSurface::ProfileGrid { scope_key: String::new() }),
            cell_width: Cell::new(None),
            drawn_columns: Cell::new(ProfileGridMetrics::COLUMNS as usize),
            grid_content_origin_y: Cell::new(None),
            user_id: RefCell::new(String::new()),
            tag: RefCell::new(String::new()),
            kind: Cell::new(None),
            layout: Cell::new(ProfileGridSharedElementLayout::Posts),
            measured_cell_height: Cell::new(None),
            grid_origin_x: Cell::new(0.0),
            scroller_ref: RefCell::new(None),
            display_scale: Cell::new(None),
            probe_window: Cell::new(None),
            probe_window_size: Cell::new(None),
            arming_post: RefCell::new(None),
            armed_route_opens_viewer: Cell::new(false),
            pending_return: RefCell::new(None),
            opened_cell: RefCell::new(None),
            opened_item_id: RefCell::new(None),
        })
    }

    pub fn cell_width(&self) -> Option<f32> {
        self.cell_width.get()
    }

    pub fn grid_content_origin_y(&self) -> Option<f32> {
        self.grid_content_origin_y.get()
    }

    pub fn surface(user_id: &str, tag: &str, kind: RisePostProfileKind) -> SharedElementSurface {
        SharedElementSurface::ProfileGrid {
            scope_key: RisePostPageScope::Profile { user_id: user_id.to_owned(), tag: tag.to_owned(), kind }.key(),
        }
    }

    /// anchor part of a grid cell, text cells have no preview source
    pub fn part(post: &GetPostFeedResponse) -> SharedElementPart {
        if SharedElementBitmapResolver::grid_cover_source(post).is_none() {
            SharedElementPart::Text
        } else {
            SharedElementPart::Media
        }
    }

    /// anchor part of a grid cell for a given profile tab kind
    pub fn part_kind(post: &GetPostFeedResponse, kind: RisePostProfileKind) -> SharedElementPart {
        if Self::cover_source(post, Some(kind)).is_none() { SharedElementPart::Text } else { SharedElementPart::Media }
    }

    pub fn part_layout(post: &GetPostFeedResponse, layout: ProfileGridSharedElementLayout) -> SharedElementPart {
        match layout {
            ProfileGridSharedElementLayout::Posts => Self::part(post),
            ProfileGridSharedElementLayout::Riffs => {
                if ProfileRiffCoverSourcePolicy::source(post).is_none() {
                    SharedElementPart::Text
                } else {
                    SharedElementPart::Media
                }
            }
        }
    }

    /// picture a cell draws in the grid of this kind, none for a text cell
    pub fn cover_source(post: &GetPostFeedResponse, kind: Option<RisePostProfileKind>) -> Option<String> {
        if kind != Some(RisePostProfileKind::Reposts) {
            return SharedElementBitmapResolver::grid_cover_source(post);
        }
        ProfileGridPreviewSourcePolicy::repost_source(
            post.local_preview_url.as_deref(),
            post.images.first().map(String::as_str),
            post.video_thumbnail.as_deref(),
            post.track.as_ref().and_then(|track| track.cover_url.as_deref()),
        )
    }

    fn text_cover(
        post: &GetPostFeedResponse,
        kind: Option<RisePostProfileKind>,
        cell_size: Size<Pixels>,
        display_scale: f32,
        cx: &App,
    ) -> Option<SharedElementCover> {
        let epoch =
            rise_ui::shared::try_read::<SharedElementTransitionCoordinator>(cx).map_or(0, |coordinator| coordinator.epoch());
        if kind != Some(RisePostProfileKind::Reposts) {
            return SharedElementCoverRasterizer::text_cover(post, cell_size, display_scale, epoch);
        }
        SharedElementCoverRasterizer::repost_text_cover(post, cell_size, display_scale, epoch)
    }

    pub fn configure(
        &self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        scroller: Option<SharedElementReturnScrollerRef>,
        layout: ProfileGridSharedElementLayout,
    ) {
        *self.user_id.borrow_mut() = user_id.to_owned();
        *self.tag.borrow_mut() = tag.to_owned();
        self.kind.set(Some(kind));
        self.layout.set(layout);
        *self.scroller_ref.borrow_mut() = scroller;
        *self.surface.borrow_mut() = Self::surface(user_id, tag, kind);
    }

    /// grid reports its column count so a return lands on the right cell
    pub fn record_columns(&self, columns: i64) {
        self.drawn_columns.set(columns.max(1) as usize);
    }

    pub fn record_layout(
        &self,
        probe: Bounds<Pixels>,
        cell_width: f32,
        cell_height: Option<f32>,
        origin_x: f32,
        window: &Window,
    ) {
        if cell_width.is_finite() && cell_width > 0.0 {
            self.cell_width.set(Some(cell_width));
        }
        if let Some(cell_height) = cell_height
            && cell_height.is_finite()
            && cell_height > 0.0
        {
            self.measured_cell_height.set(Some(cell_height));
        }
        if origin_x.is_finite() && origin_x >= 0.0 {
            self.grid_origin_x.set(origin_x);
        }
        let scale = window.scale_factor();
        if scale.is_finite() && scale > 0.0 {
            self.display_scale.set(Some(scale));
        }
        let handle = window.window_handle();
        self.probe_window.set(Some(handle));
        self.probe_window_size.set(Some(window.viewport_size()));
        let Some(scroll) = Self::enclosing_scroll_view(probe, handle) else { return };
        let Some(frame) = scroll.frame() else { return };
        let origin_y = f32::from(probe.origin.y - frame.origin.y) + f32::from(scroll.content_offset().y);
        if !origin_y.is_finite() || origin_y < 0.0 {
            return;
        }
        self.grid_content_origin_y.set(Some(origin_y));
    }

    /// arms an open flight for a grid tap, false means nothing armed and the plain push runs
    pub fn arm_open_if_possible(self: &Rc<Self>, post: &GetPostFeedResponse, cx: &mut App) -> bool {
        *self.opened_cell.borrow_mut() = None;
        *self.opened_item_id.borrow_mut() = None;
        let Some(kind) = self.kind.get() else { return false };
        if !SharedElementNavigationPolicy::flights_enabled()
            || !self.layout.get().flies(kind)
            || self.user_id.borrow().is_empty()
            || post.shows_upload_overlay()
        {
            return false;
        }

        let origin =
            RisePostPageScope::Profile { user_id: self.user_id.borrow().clone(), tag: self.tag.borrow().clone(), kind };
        let route = RootRoute::post_content(post, Some(origin));
        let route_flies = self.layout.get().route_flies(&route);
        if !route_flies {
            return false;
        }
        self.armed_route_opens_viewer.set(matches!(route, RootRoute::ShortViewer { .. }));

        *self.arming_post.borrow_mut() = Some(post.clone());
        let container: Rc<dyn SharedElementOriginContainer> = self.clone();
        let armed = SharedElementTransitionCoordinator::arm_open(&container, &post.id, route_flies, cx);
        *self.arming_post.borrow_mut() = None;
        if armed {
            *self.opened_item_id.borrow_mut() = Some(post.id.clone());
            self.note_open_position(cx);
            self.warm_full_image(post, cx);
            if !self.armed_route_opens_viewer.get() && PostListEnginePolicy::uses_native_list() {
                let window_size = self.host_view(cx).and(self.probe_window_size.get());
                if let Some(hooks) = post_row_warm_anchor_hooks() {
                    (hooks.warm_anchor)(
                        post,
                        window_size.map(|window_size| f32::from(window_size.width)),
                        window_size.and(self.display_scale.get()),
                        None,
                        cx,
                    );
                }
            }
        }
        armed
    }

    fn note_open_position(&self, cx: &mut App) {
        let scroller_ref = self.scroller_ref.borrow().clone();
        if let Some(scroller_ref) = scroller_ref
            && let Some(scroller) = scroller_ref.scroller()
        {
            scroller.note_open_position(scroller_ref.tab_key, cx);
        }
    }

    /// posts rows follow cell width, riffs grid answers only after a cell was measured
    fn resolved_cell_height(&self) -> Option<f32> {
        match self.layout.get() {
            ProfileGridSharedElementLayout::Posts => Some(
                self.cell_width.get().map_or(ProfileGridMetrics::CELL_HEIGHT, ProfileGridMetrics::row_height),
            ),
            ProfileGridSharedElementLayout::Riffs => self.measured_cell_height.get(),
        }
    }

    fn cover_source_for_layout(&self, post: &GetPostFeedResponse) -> Option<String> {
        match self.layout.get() {
            ProfileGridSharedElementLayout::Posts => Self::cover_source(post, self.kind.get()),
            ProfileGridSharedElementLayout::Riffs => ProfileRiffCoverSourcePolicy::source(post),
        }
    }

    fn return_flies_live_video(epoch: u64, cx: &mut App) -> bool {
        let Some(intent) = SharedElementTransitionCoordinator::shared(cx).read(cx).intent() else {
            return false;
        };
        if intent.epoch != epoch {
            return false;
        }
        intent.capture().is_some_and(|capture| capture.live_readiness.is_some())
    }

    fn is_own_read_only(post: &GetPostFeedResponse, cx: &App) -> bool {
        let viewer =
            rise_ui::shared::try_read::<AuthServicesStore>(cx).and_then(AuthServicesStore::effective_user_id_for_wire);
        RiseIdentity::is_own(
            &RiseIdentity::author(Some(post.author_id.as_str()), None, post.author.id.as_deref()),
            viewer.as_deref(),
            None,
        )
    }

    fn page_items(&self, cx: &App) -> Option<Vec<GetPostFeedResponse>> {
        let kind = self.kind.get()?;
        let user_id = self.user_id.borrow();
        if user_id.is_empty() {
            return None;
        }
        let tag = self.tag.borrow();
        let presentation = rise_ui::shared::try_read::<RisePostContentPresentation>(cx)?;
        Some(presentation.profile_page_with_tag(&user_id, &tag, kind, cx).into_owned().items)
    }

    fn post(&self, item_id: &str, cx: &App) -> Option<GetPostFeedResponse> {
        if let Some(arming_post) = self.arming_post.borrow().as_ref()
            && arming_post.id == item_id
        {
            return Some(arming_post.clone());
        }
        self.page_items(cx)?.into_iter().find(|post| post.id == item_id)
    }

    fn index(&self, item_id: &str, cx: &App) -> Option<usize> {
        self.page_items(cx)?.iter().position(|post| post.id == item_id)
    }

    fn item_content_rect(&self, index: usize) -> Option<Bounds<Pixels>> {
        let cell_width = self.cell_width.get().filter(|cell_width| *cell_width > 0.0)?;
        let cell_height = self.resolved_cell_height()?;
        let layout = self.layout.get();
        let columns = self.drawn_columns.get();
        Some(SharedElementReturnScrollMath::grid_item_rect_with_origin_x(
            index,
            columns,
            cell_width,
            cell_height,
            layout.spacing(),
            self.grid_content_origin_y.get().unwrap_or_else(ProfileViewportLayoutPolicy::page_content_top_inset),
            if layout == ProfileGridSharedElementLayout::Riffs { self.grid_origin_x.get() } else { 0.0 },
        ))
    }

    fn kept_open_frame(
        &self,
        item_id: &str,
        viewport: SharedElementReturnViewport,
        band: SharedElementReturnScrollMathBand,
    ) -> Option<Bounds<Pixels>> {
        let opened_cell = self.opened_cell.borrow();
        let (opened_item_id, frame) = opened_cell.as_ref()?;
        if opened_item_id != item_id {
            return None;
        }
        let tolerance = 1.0;
        let top = f32::from(viewport.screen_origin.y) + band.top;
        let bottom = f32::from(viewport.screen_origin.y) + viewport.height - band.bottom_inset;
        if f32::from(frame.top()) < top - tolerance || f32::from(frame.bottom()) > bottom + tolerance {
            return None;
        }
        Some(*frame)
    }

    fn return_band(
        &self,
        viewport: SharedElementReturnViewport,
        bottom_inset: f32,
        cx: &mut App,
    ) -> SharedElementReturnScrollMathBand {
        let top = ProfileViewportLayoutPolicy::page_content_top_inset();
        let outer_max_y = f32::from(viewport.screen_origin.y) + viewport.height;
        let mut inset = if bottom_inset.is_finite() { bottom_inset.max(0.0) } else { 0.0 };
        if let Some(chrome_top) = BottomChromeInsets::shared(cx).read(cx).top_edge_in_window(&[]) {
            let chrome_inset = outer_max_y - chrome_top;
            if chrome_inset.is_finite() {
                inset = inset.max(chrome_inset);
            }
        }
        if let Some(safe_bottom) = self.safe_area_bottom_edge_on_screen() {
            let safe_inset = outer_max_y - safe_bottom;
            if safe_inset.is_finite() {
                inset = inset.max(safe_inset);
            }
        }
        SharedElementReturnScrollMathBand { top, bottom_inset: inset.min((viewport.height - top).max(0.0)) }
    }

    fn safe_area_bottom_edge_on_screen(&self) -> Option<f32> {
        let window_size = self.probe_window_size.get()?;
        let y = f32::from(window_size.height);
        y.is_finite().then_some(y)
    }

    fn return_scroller(&self, epoch: u64) -> Option<Rc<dyn SharedElementReturnScroller>> {
        if let Some(pending) = self.pending_return.borrow().as_ref()
            && pending.epoch == epoch
        {
            return pending.scroller.upgrade();
        }
        self.scroller_ref.borrow().as_ref().and_then(SharedElementReturnScrollerRef::scroller)
    }

    fn clear_pending_return(&self, epoch: u64) {
        let pending = self.pending_return.borrow_mut().take_if(|pending| pending.epoch == epoch);
        if let Some(scroller) = pending.and_then(|pending| pending.scroller.upgrade()) {
            scroller.set_on_return_resumable(None);
        }
    }

    /// anchor bounds only if inside this origins host window
    /// so a same post mounted elsewhere never answers
    /// topmost live host wins
    fn attached_anchor(&self, key: &SharedElementKey, cx: &App) -> Option<Bounds<Pixels>> {
        let id = key.frame_registry_key();
        let host_view = self.host_view(cx)?;
        let view = MediaCellScreenFrameRegistry::attached_view(&id)?;
        if view.window() != Some(host_view) {
            return None;
        }
        MediaCellScreenFrameRegistry::attached_frame(&id)
    }

    /// press scale shrinks the measured cell around its center so the real cell is rebuilt from that
    fn nominal_cell_rect(&self, frame: Bounds<Pixels>) -> Bounds<Pixels> {
        let Some(cell_width) = self.cell_width.get().filter(|cell_width| *cell_width > 0.0) else {
            return frame;
        };
        let height = self.resolved_cell_height().unwrap_or_else(|| f32::from(frame.size.height));
        SharedElementGeometry::rect(frame.center(), gpui::size(px(cell_width), px(height)))
    }

    fn open_bitmap(
        &self,
        post: &GetPostFeedResponse,
        cell_size: Size<Pixels>,
        cx: &mut App,
    ) -> Option<SharedElementBitmapResolverBitmap> {
        let cover_source = self.cover_source_for_layout(post)?;
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        let cell_bucket = SharedElementBitmapResolver::cell_bucket(cell_size, self.resolved_display_scale());
        let list_slide = post.cover_slide();
        let lands_on_same_picture = Self::lands_on_same_picture(post, &cover_source);
        let sources = [cover_source];

        let resolved = if lands_on_same_picture && list_slide.is_some_and(|slide| slide.is_video()) {
            SharedElementBitmapResolver::poster(post, cx)
                .or_else(|| SharedElementBitmapResolver::bitmap(&sources, is_own, &[cell_bucket, None]))
        } else {
            let buckets = if lands_on_same_picture { [None, cell_bucket] } else { [cell_bucket, None] };
            SharedElementBitmapResolver::bitmap(&sources, is_own, &buckets)
        }?;
        let pixels = resolved.image.size(0);
        let longest = i64::from(pixels.width.0.max(pixels.height.0));
        cell_bucket.is_none_or(|bucket| longest * 2 >= bucket).then_some(resolved)
    }

    /// true if the landings first page shows the same picture as the grid cell
    fn lands_on_same_picture(post: &GetPostFeedResponse, cover_source: &str) -> bool {
        post.cover_slide()
            .and_then(|slide| slide.poster_url().map(|poster| trimming_whitespaces_and_newlines(poster) == cover_source))
            .unwrap_or(false)
    }

    fn warm_full_image(&self, post: &GetPostFeedResponse, cx: &mut App) {
        let Some(PostMediaSlide::Image { url, .. }) = post.cover_slide() else { return };
        let source = trimming_whitespaces_and_newlines(&url).to_owned();
        if source.is_empty() {
            return;
        }
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        let policy = PostMediaCachePolicy::image_policy(is_own);
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let bucket =
            self.armed_route_opens_viewer.get().then_some(ShortsMediaPrefetchPolicy::POSTER_PIXEL_DIMENSION);
        cx.spawn(async move |cx| {
            let _ = CleverImageBitmapLoader::load(
                &source,
                is_own,
                true,
                account_user_id,
                account_epoch,
                bucket,
                policy,
                CleverImageWorkLane::Visible,
                cx,
            )
            .await;
        })
        .detach();
    }

    fn replicant(
        &self,
        post: &GetPostFeedResponse,
        screen_rect: Bounds<Pixels>,
        cx: &mut App,
    ) -> Option<(AnyView, Bounds<Pixels>)> {
        let content: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement> = match self.cover_source_for_layout(post) {
            Some(source) => {
                let is_own = PostMediaCachePolicy::is_own_post(post, cx);
                Rc::new(move |_: &mut Window, _: &mut App| {
                    CleverImageUi::new(source.clone())
                        .content_mode(ContentMode::Fill)
                        .persist_to_disk(is_own)
                        .shows_progress_indicator(false)
                        .into_any_element()
                })
            }
            None => {
                let display_scale = self.resolved_display_scale();
                let cover = Self::text_cover(post, self.kind.get(), screen_rect.size, display_scale, cx)?;
                match cover.image {
                    SharedElementCoverContent::Element(element) => element,
                    SharedElementCoverContent::Image(_) => return None,
                }
            }
        };
        let view: AnyView = cx.new(|_| ProfileGridCellReplicant { content }).into();
        Some((view, screen_rect))
    }

    /// display scale, only the probe can read it and only during a draw
    fn resolved_display_scale(&self) -> f32 {
        self.display_scale.get().unwrap_or(ProfileThumbnailDecodePolicy::MAXIMUM_DISPLAY_SCALE)
    }

    fn enclosing_scroll_view(probe: Bounds<Pixels>, window: AnyWindowHandle) -> Option<GuideScrollHost> {
        let area = |host: &GuideScrollHost| {
            host.frame().map_or(f32::INFINITY, |frame| f32::from(frame.size.width) * f32::from(frame.size.height))
        };
        GuideScrollHost::live()
            .into_iter()
            .filter(|host| host.is_in_window() && host.window() == Some(window))
            .filter(|host| {
                host.frame().is_some_and(|frame| {
                    frame.left() <= probe.origin.x
                        && probe.origin.x <= frame.right()
                        && frame.top() <= probe.origin.y
                        && probe.origin.y <= frame.bottom()
                })
            })
            .min_by(|lhs, rhs| {
                area(lhs).total_cmp(&area(rhs)).then_with(|| {
                    f32::from(rhs.max_content_offset().y).total_cmp(&f32::from(lhs.max_content_offset().y))
                })
            })
    }
}

impl SharedElementOriginContainer for ProfileGridSharedElementOrigin {
    fn surface(&self) -> SharedElementSurface {
        self.surface.borrow().clone()
    }

    fn host_view_controller(&self, cx: &App) -> Option<SharedElementStackHost> {
        let this: Rc<dyn SharedElementOriginContainer> = self.this.upgrade()?;
        rise_ui::shared::try_read::<SharedElementTransitionCoordinator>(cx)?.registered_host_of_origin(&this)
    }

    fn host_view(&self, cx: &App) -> Option<AnyWindowHandle> {
        self.host_view_controller(cx).and(self.probe_window.get())
    }

    fn open_capture(&self, item_id: &str, cx: &mut App) -> Option<SharedElementCapture> {
        let kind = self.kind.get()?;
        let post = self.post(item_id, cx)?;
        let part = if self.cover_source_for_layout(&post).is_none() {
            SharedElementPart::Text
        } else {
            SharedElementPart::Media
        };
        let key = SharedElementKey::new(self.surface.borrow().clone(), item_id, part);
        let anchor = self.attached_anchor(&key, cx)?;
        let clip = self.nominal_cell_rect(anchor);
        if clip.size.width <= px(1.0) || clip.size.height <= px(1.0) {
            return None;
        }
        *self.opened_cell.borrow_mut() = Some((item_id.to_owned(), clip));
        self.note_open_position(cx);

        if part == SharedElementPart::Media
            && let Some(bitmap) = self.open_bitmap(&post, clip.size, cx)
        {
            let flies_video_poster = post.cover_slide().is_some_and(|slide| slide.is_video())
                && self.open_lands_on_same_picture(item_id, cx);
            return Some(SharedElementCapture {
                item_id: item_id.to_owned(),
                part,
                source_key: key,
                clip,
                content: bitmap.flight_content(),
                content_rect: SharedElementGeometry::fill_rect(bitmap.aspect, clip),
                mode: SharedElementContentMode::Fill,
                backdrop: None,
                flying_identity: if flies_video_poster { None } else { Some(bitmap.source) },
                stop_frame: None,
                live_readiness: None,
                placeholder: None,
            });
        }

        if self.armed_route_opens_viewer.get() {
            return None;
        }

        if HoldContextMenuPresenter::has_active_request(cx) {
            if part != SharedElementPart::Text {
                return None;
            }
            let display_scale = self.resolved_display_scale();
            let cover = Self::text_cover(&post, Some(kind), clip.size, display_scale, cx)?;
            let content = match cover.image {
                SharedElementCoverContent::Image(image) => SharedElementFlightContent::Bitmap(image, cover.aspect),
                SharedElementCoverContent::Element(element) => {
                    let view: AnyView = cx.new(|_| ProfileGridCellReplicant { content: element }).into();
                    SharedElementFlightContent::Replicant(view, clip.size)
                }
            };
            return Some(SharedElementCapture {
                item_id: item_id.to_owned(),
                part,
                source_key: key,
                clip,
                content,
                content_rect: clip,
                mode: SharedElementContentMode::AnchoredWidthFit,
                backdrop: None,
                flying_identity: Some(cover.identity),
                stop_frame: None,
                live_readiness: None,
                placeholder: None,
            });
        }

        let (view, screen_rect) = self.replicant(&post, clip, cx)?;
        Some(SharedElementCapture {
            item_id: item_id.to_owned(),
            part,
            source_key: key,
            clip,
            content: SharedElementFlightContent::Replicant(view, screen_rect.size),
            content_rect: screen_rect,
            mode: if part == SharedElementPart::Text {
                SharedElementContentMode::AnchoredWidthFit
            } else {
                SharedElementContentMode::Fill
            },
            backdrop: None,
            flying_identity: Some(Self::REPLICANT_IDENTITY.to_owned()),
            stop_frame: None,
            live_readiness: None,
            placeholder: None,
        })
    }

    fn open_lands_on_same_picture(&self, item_id: &str, cx: &App) -> bool {
        let Some(post) = self.post(item_id, cx) else { return true };
        let Some(cover_source) = self.cover_source_for_layout(&post) else { return true };
        Self::lands_on_same_picture(&post, &cover_source)
    }

    fn can_return(&self, item_id: &str, cx: &App) -> bool {
        let Some(kind) = self.kind.get() else {
            return false;
        };
        if !self.layout.get().flies(kind)
            || !self.cell_width.get().is_some_and(|cell_width| cell_width > 0.0)
            || self.resolved_cell_height().is_none()
        {
            return false;
        }
        let scroller_ref = self.scroller_ref.borrow().clone();
        let Some(scroller_ref) = scroller_ref else {
            return false;
        };
        let Some(scroller) = scroller_ref.scroller() else {
            return false;
        };
        let index = self.index(item_id, cx);
        let can_prepare = index.is_some() && scroller.can_prepare_return(scroller_ref.tab_key, cx);
        can_prepare
    }

    fn return_cover(&self, item_id: &str, cx: &App) -> Option<SharedElementCover> {
        let cell_width = self.cell_width.get().filter(|cell_width| *cell_width > 0.0)?;
        let cell_height = self.resolved_cell_height()?;
        let post = self.post(item_id, cx)?;
        let cell_size = gpui::size(px(cell_width), px(cell_height));
        let display_scale = self.resolved_display_scale();
        let Some(source) = self.cover_source_for_layout(&post) else {
            return Self::text_cover(&post, self.kind.get(), cell_size, display_scale, cx);
        };
        SharedElementBitmapResolver::cover(&source, Self::is_own_read_only(&post, cx), cell_size, display_scale)
    }

    fn prepare_return(&self, item_id: &str, epoch: u64, cx: &mut App) -> Option<SharedElementReturnPlan> {
        let scroller_ref = self.scroller_ref.borrow().clone()?;
        let scroller = scroller_ref.scroller()?;
        let index = self.index(item_id, cx)?;
        let item_rect = self.item_content_rect(index)?;
        let viewport = scroller.return_viewport_snapshot(scroller_ref.tab_key, cx)?;

        let band = self.return_band(viewport, scroller_ref.bottom_inset, cx);
        let kept_frame = self.kept_open_frame(item_id, viewport, band);
        let keeps_position = kept_frame.is_some() || self.opened_item_id.borrow().as_deref() == Some(item_id);
        let request = SharedElementReturnRequest {
            tab_key: scroller_ref.tab_key,
            item_content_rect: item_rect,
            band,
            epoch,
            keeps_position,
        };
        let mut plan = scroller.prepare_return(request, cx)?;
        if let Some(kept_frame) = kept_frame {
            plan = SharedElementReturnPlan {
                epoch: plan.epoch,
                predicted_frame: kept_frame,
                clamp: SharedElementReturnScrollMathClamp::None,
            };
        }

        *self.pending_return.borrow_mut() = Some(PendingReturn {
            epoch,
            item_id: item_id.to_owned(),
            scroller: Rc::downgrade(&scroller),
            kept_frame,
            keeps_position,
            prepared_item_rect: item_rect,
        });
        let this = self.this.clone();
        scroller.set_on_return_resumable(Some(Rc::new(move |resumed_epoch: u64, cx: &mut App| {
            let Some(this) = this.upgrade() else { return };
            let is_pending = this.pending_return.borrow().as_ref().is_some_and(|pending| pending.epoch == resumed_epoch);
            if !is_pending || SharedElementTransitionCoordinator::shared(cx).read(cx).epoch() != resumed_epoch {
                return;
            }
            SharedElementNavigationDriver::shared().return_did_become_resumable(resumed_epoch, cx);
        })));
        Some(plan)
    }

    fn resolve_return(&self, epoch: u64, cx: &mut App) -> SharedElementTargetResolution {
        let (item_id, scroller, kept_frame, keeps_position, prepared_item_rect) = {
            let pending = self.pending_return.borrow();
            let Some(pending) = pending.as_ref().filter(|pending| pending.epoch == epoch) else {
                return SharedElementTargetResolution::Unavailable;
            };
            (
                pending.item_id.clone(),
                pending.scroller.upgrade(),
                pending.kept_frame,
                pending.keeps_position,
                pending.prepared_item_rect,
            )
        };
        let Some(scroller) = scroller else {
            return SharedElementTargetResolution::Unavailable;
        };
        let Some(item_rect) = self.index(&item_id, cx).and_then(|index| self.item_content_rect(index)) else {
            return SharedElementTargetResolution::Unavailable;
        };

        match scroller.resolve_return(epoch, item_rect, cx) {
            SharedElementReturnResolution::Ready { predicted_frame } => {
                if item_rect != prepared_item_rect {
                    return SharedElementTargetResolution::Ready(predicted_frame);
                }
                if let Some(kept_frame) = kept_frame {
                    return SharedElementTargetResolution::Ready(kept_frame);
                }
                if !keeps_position {
                    return SharedElementTargetResolution::Ready(predicted_frame);
                }
                SharedElementTargetResolution::Ready(self.measured_target_frame(&item_id, cx).unwrap_or(predicted_frame))
            }
            SharedElementReturnResolution::Deferred => SharedElementTargetResolution::Deferred,
            SharedElementReturnResolution::Unavailable => SharedElementTargetResolution::Unavailable,
        }
    }

    fn measured_target_frame(&self, item_id: &str, cx: &App) -> Option<Bounds<Pixels>> {
        let parts: Vec<SharedElementPart> = match self.post(item_id, cx) {
            Some(post) => vec![if self.cover_source_for_layout(&post).is_none() {
                SharedElementPart::Text
            } else {
                SharedElementPart::Media
            }],
            None => vec![SharedElementPart::Media, SharedElementPart::Text],
        };
        for part in parts {
            let key = SharedElementKey::new(self.surface.borrow().clone(), item_id, part);
            if let Some(anchor) = self.attached_anchor(&key, cx) {
                return Some(self.nominal_cell_rect(anchor));
            }
        }
        None
    }

    fn cancel_return(&self, epoch: u64, cx: &mut App) {
        let scroller = self.return_scroller(epoch);
        if let Some(scroller) = scroller {
            if Self::return_flies_live_video(epoch, cx) {
                scroller.cancel_live_return(epoch, cx);
            } else {
                scroller.cancel_return(epoch, cx);
            }
        }
        self.clear_pending_return(epoch);
    }

    fn finish_return(&self, epoch: u64, cx: &mut App) {
        if let Some(scroller) = self.return_scroller(epoch) {
            scroller.finish_return(epoch, cx);
        }
        self.clear_pending_return(epoch);
        *self.opened_cell.borrow_mut() = None;
        *self.opened_item_id.borrow_mut() = None;
    }
}
