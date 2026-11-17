use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{AnyWindowHandle, App, Bounds, Pixels, Size, Window, px, size};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::guide_anchor_ui::guide_scroll_host::GuideScrollHost;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementGeometry, SharedElementKey, SharedElementPart,
    SharedElementReturnScrollMath, SharedElementReturnScrollMathBand, SharedElementSurface,
};
use crate::core::shared_element_transition::overlay::shared_element_overlay_contracts::SharedElementOverlayOrigin;
use crate::core::shared_element_transition::overlay::shared_element_overlay_driver::SharedElementOverlayDriver;
use crate::core::shared_element_transition::renderer::shared_element_bitmap_resolver::SharedElementBitmapResolver;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementCover, SharedElementCoverContent, SharedElementFlightContent,
    SharedElementOriginContainer, SharedElementReturnPlan, SharedElementReturnRequest,
    SharedElementReturnResolution, SharedElementReturnScroller, SharedElementReturnViewport,
    SharedElementTargetResolution,
};
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementStackHost;
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsets;
use crate::modules::story::components::stories_grid::stories_grid_cell::StoriesGridMetrics;
use crate::modules::story::components::story_player::story_player_shared_element_destination::StorySharedElementPicture;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::StoryProfileScope;
use crate::modules::story::stores::story::story_actions::story_types::{
    StoryPlayerEntrance, StoryPlayerSharedElementOrigin,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryDTO;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::ProfileThumbnailDecodePolicy;
use crate::modules::user::components::profile_content::profile_content::ProfileViewportLayoutPolicy;
use crate::modules::user::components::profile_content::shared_element_return_scroller_environment::SharedElementReturnScrollerRef;

struct PendingReturn {
    epoch: u64,
    story_id: i64,
    scroller: Weak<dyn SharedElementReturnScroller>,
}

/// overlay origin of the profile stories grid,
/// open capture and return cover and return positioning through the scroll coordinator
pub struct StoriesGridSharedElementOrigin {
    this: Weak<Self>,
    surface: RefCell<SharedElementSurface>,
    owner_user_id: RefCell<String>,
    scroller_ref: RefCell<Option<SharedElementReturnScrollerRef>>,
    probe_window: Cell<Option<AnyWindowHandle>>,
    probe_window_height: Cell<f32>,
    cell_size: Cell<Size<Pixels>>,
    grid_width: Cell<f32>,
    grid_content_origin_y: Cell<Option<f32>>,
    display_scale: Cell<Option<f32>>,
    story_ids: RefCell<Vec<i64>>,
    stories_by_id: RefCell<HashMap<i64, StoryDTO>>,
    latest_stories: RefCell<Rc<Vec<StoryDTO>>>,
    pending_return: RefCell<Option<PendingReturn>>,
    opened_story_id: Cell<Option<i64>>,
}

impl StoriesGridSharedElementOrigin {
    pub fn new() -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            surface: RefCell::new(SharedElementSurface::StoriesGrid { scope_key: String::new() }),
            owner_user_id: RefCell::new(String::new()),
            scroller_ref: RefCell::new(None),
            probe_window: Cell::new(None),
            probe_window_height: Cell::new(0.0),
            cell_size: Cell::new(size(px(0.0), px(0.0))),
            grid_width: Cell::new(0.0),
            grid_content_origin_y: Cell::new(None),
            display_scale: Cell::new(None),
            story_ids: RefCell::new(Vec::new()),
            stories_by_id: RefCell::new(HashMap::new()),
            latest_stories: RefCell::new(Rc::new(Vec::new())),
            pending_return: RefCell::new(None),
            opened_story_id: Cell::new(None),
        })
    }

    pub fn surface(owner_user_id: &str) -> SharedElementSurface {
        SharedElementSurface::StoriesGrid {
            scope_key: StoryServicesStore::profile_stories_cache_key(owner_user_id, StoryProfileScope::Profile),
        }
    }

    pub fn item_id(story_id: i64) -> String {
        story_id.to_string()
    }

    /// exit origin of a player opened from this grid, the story on screen
    pub fn player_origin(&self) -> StoryPlayerSharedElementOrigin {
        StoryPlayerSharedElementOrigin { surface: self.surface.borrow().clone(), item_id: None }
    }

    pub fn configure(&self, owner_user_id: &str, scroller: Option<SharedElementReturnScrollerRef>) {
        *self.owner_user_id.borrow_mut() = owner_user_id.to_owned();
        *self.scroller_ref.borrow_mut() = scroller;
        *self.surface.borrow_mut() = Self::surface(owner_user_id);
    }

    pub(super) fn record_latest_stories(&self, stories: &Rc<Vec<StoryDTO>>) {
        *self.latest_stories.borrow_mut() = stories.clone();
    }

    pub fn record_layout(&self, probe: Bounds<Pixels>, container_width: f32, window: &Window) {
        let handle = window.window_handle();
        self.probe_window.set(Some(handle));
        self.probe_window_height.set(f32::from(window.viewport_size().height));
        if container_width.is_finite() && container_width > 1.0 {
            self.grid_width.set(container_width);
            self.cell_size.set(size(
                px(StoriesGridMetrics::cell_width(container_width)),
                px(StoriesGridMetrics::cell_height(container_width)),
            ));
        }
        let scale = window.scale_factor();
        if scale.is_finite() && scale > 0.0 {
            self.display_scale.set(Some(scale));
        }
        let Some(scroll) = Self::enclosing_scroll_host(probe, handle) else { return };
        let Some(frame) = scroll.frame() else { return };
        let origin_y = f32::from(probe.origin.y - frame.origin.y) + f32::from(scroll.content_offset().y);
        if !origin_y.is_finite() || origin_y < 0.0 {
            return;
        }
        self.grid_content_origin_y.set(Some(origin_y));
    }

    fn enclosing_scroll_host(probe: Bounds<Pixels>, window: AnyWindowHandle) -> Option<GuideScrollHost> {
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

    pub fn arm_open(
        self: &Rc<Self>,
        story: &StoryDTO,
        stories: &[StoryDTO],
        request_id: Uuid,
        cx: &mut App,
    ) -> StoryPlayerEntrance {
        if !SharedElementNavigationPolicy::stories_enabled()
            || self.owner_user_id.borrow().is_empty()
            || self.scroller_ref.borrow().is_none()
            || Self::is_pending_story(story.id, cx)
        {
            return StoryPlayerEntrance::Slide;
        }
        self.snapshot(stories);
        let origin: Rc<dyn SharedElementOverlayOrigin> = self.clone();
        let Some(epoch) = SharedElementOverlayDriver::open(
            &origin,
            &Self::item_id(story.id),
            SharedElementSurface::StoryPlayer { request_id: request_id.uuid_string() },
            cx,
        ) else {
            return StoryPlayerEntrance::Slide;
        };
        self.opened_story_id.set(Some(story.id));
        self.note_open_position(cx);
        if HoldContextMenuPresenter::has_active_request(cx) {
            HoldContextMenuPresenter::dismiss(cx);
        }
        StoryPlayerEntrance::SharedElement { epoch }
    }

    fn note_open_position(&self, cx: &mut App) {
        let scroller_ref = self.scroller_ref.borrow().clone();
        if let Some(scroller_ref) = scroller_ref
            && let Some(scroller) = scroller_ref.scroller()
        {
            scroller.note_open_position(scroller_ref.tab_key, cx);
        }
    }

    /// nothing is pending before the first shared
    fn is_pending_story(story_id: i64, cx: &App) -> bool {
        rise_ui::shared::try_read::<StoryServicesStore>(cx).is_some_and(|store| store.is_pending_story(story_id, cx))
    }

    fn snapshot(&self, stories: &[StoryDTO]) {
        *self.story_ids.borrow_mut() = stories.iter().map(|story| story.id).collect();
        let mut by_id: HashMap<i64, StoryDTO> = HashMap::with_capacity(stories.len());
        for story in stories {
            by_id.insert(story.id, story.clone());
        }
        *self.stories_by_id.borrow_mut() = by_id;
    }

    fn cell_bucket(&self) -> Option<i64> {
        SharedElementBitmapResolver::cell_bucket(self.cell_size.get(), self.resolved_display_scale())
    }

    /// the fixed columns are centered in the grid width, their count follows that width
    fn item_content_rect(&self, index: usize) -> Bounds<Pixels> {
        let columns = StoriesGridMetrics::columns(self.grid_width.get()) as usize;
        let spacing = StoriesGridMetrics::SPACING;
        let cell_size = self.cell_size.get();
        let cell_width = f32::from(cell_size.width);
        let columns_width = columns as f32 * cell_width + (columns - 1) as f32 * spacing;
        SharedElementReturnScrollMath::grid_item_rect_with_origin_x(
            index,
            columns,
            cell_width,
            f32::from(cell_size.height),
            spacing,
            self.grid_content_origin_y.get().unwrap_or_else(ProfileViewportLayoutPolicy::page_content_top_inset),
            ((self.grid_width.get() - columns_width) / 2.0).max(0.0),
        )
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
        if self.overlay_window(cx).is_some() {
            let safe_inset = outer_max_y - self.probe_window_height.get();
            if safe_inset.is_finite() {
                inset = inset.max(safe_inset);
            }
        }
        SharedElementReturnScrollMathBand { top, bottom_inset: inset.min((viewport.height - top).max(0.0)) }
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

    /// accepted only inside this origins host window so the same story mounted elsewhere never answers
    fn attached_anchor_frame(&self, key: &SharedElementKey, cx: &App) -> Option<Bounds<Pixels>> {
        let id = key.frame_registry_key();
        let view = MediaCellScreenFrameRegistry::attached_view(&id)?;
        if let Some(host_view) = self.host_view(cx)
            && view.window() != Some(host_view)
        {
            return None;
        }
        MediaCellScreenFrameRegistry::attached_frame(&id)
    }

    /// the hold menu press scale shrinks the measured cell about its center
    fn nominal_cell_rect(&self, frame: Bounds<Pixels>) -> Bounds<Pixels> {
        let cell_size = self.cell_size.get();
        if cell_size.width <= px(1.0) || cell_size.height <= px(1.0) {
            return frame;
        }
        SharedElementGeometry::rect(frame.center(), cell_size)
    }

    fn resolved_display_scale(&self) -> f32 {
        self.display_scale.get().unwrap_or(ProfileThumbnailDecodePolicy::MAXIMUM_DISPLAY_SCALE)
    }

    fn media_key(&self, item_id: &str) -> SharedElementKey {
        SharedElementKey::new(self.surface.borrow().clone(), item_id, SharedElementPart::Media)
    }
}

impl SharedElementOriginContainer for StoriesGridSharedElementOrigin {
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
        let id = int_from_string(item_id)?;
        let story = self.stories_by_id.borrow().get(&id).cloned()?;
        if self.cell_size.get().width <= px(1.0) {
            return None;
        }
        let key = self.media_key(item_id);
        let anchor = self.attached_anchor_frame(&key, cx)?;
        let clip = self.nominal_cell_rect(anchor);
        let bucket = self.cell_bucket()?;
        let picture = StorySharedElementPicture::picture(
            &story,
            &[Some(StorySharedElementPicture::player_bucket()), Some(bucket)],
            bucket,
        )?;
        Some(SharedElementCapture {
            item_id: item_id.to_owned(),
            part: SharedElementPart::Media,
            source_key: key,
            clip,
            content_rect: SharedElementGeometry::fill_rect(picture.aspect, clip),
            content: SharedElementFlightContent::Bitmap(picture.image, picture.aspect),
            mode: SharedElementContentMode::Fill,
            backdrop: None,
            flying_identity: Some(picture.identity),
            stop_frame: None,
            live_readiness: None,
            placeholder: None,
        })
    }

    fn open_lands_on_same_picture(&self, _item_id: &str, _cx: &App) -> bool {
        true
    }

    fn can_return(&self, item_id: &str, cx: &App) -> bool {
        if !SharedElementNavigationPolicy::stories_enabled()
            || self.owner_user_id.borrow().is_empty()
            || self.cell_size.get().width <= px(1.0)
        {
            return false;
        }
        let Some(id) = int_from_string(item_id) else {
            return false;
        };
        let scroller_ref = self.scroller_ref.borrow().clone();
        let Some(scroller_ref) = scroller_ref else {
            return false;
        };
        let Some(scroller) = scroller_ref.scroller() else {
            return false;
        };
        let stories = self.latest_stories.borrow().clone();
        self.snapshot(&stories);
        if !self.stories_by_id.borrow().contains_key(&id) || Self::is_pending_story(id, cx) {
            return false;
        }
        scroller.can_prepare_return(scroller_ref.tab_key, cx)
    }

    fn return_cover(&self, item_id: &str, _cx: &App) -> Option<SharedElementCover> {
        let id = int_from_string(item_id)?;
        let story = self.stories_by_id.borrow().get(&id).cloned()?;
        if self.cell_size.get().width <= px(1.0) {
            return None;
        }
        let bucket = self.cell_bucket()?;
        let picture = StorySharedElementPicture::picture(
            &story,
            &[Some(bucket), Some(StorySharedElementPicture::player_bucket())],
            bucket,
        )?;
        Some(SharedElementCover {
            image: SharedElementCoverContent::Image(picture.image),
            aspect: picture.aspect,
            identity: picture.identity,
        })
    }

    fn prepare_return(&self, item_id: &str, epoch: u64, cx: &mut App) -> Option<SharedElementReturnPlan> {
        let scroller_ref = self.scroller_ref.borrow().clone()?;
        let scroller = scroller_ref.scroller()?;
        let id = int_from_string(item_id)?;
        let index = self.story_ids.borrow().iter().position(|story_id| *story_id == id)?;
        if self.cell_size.get().width <= px(1.0) {
            return None;
        }
        let viewport = scroller.return_viewport_snapshot(scroller_ref.tab_key, cx)?;
        let request = SharedElementReturnRequest {
            tab_key: scroller_ref.tab_key,
            item_content_rect: self.item_content_rect(index),
            band: self.return_band(viewport, scroller_ref.bottom_inset, cx),
            epoch,
            keeps_position: self.opened_story_id.get() == Some(id),
        };
        let plan = scroller.prepare_return(request, cx)?;
        *self.pending_return.borrow_mut() =
            Some(PendingReturn { epoch, story_id: id, scroller: Rc::downgrade(&scroller) });
        let this = self.this.clone();
        scroller.set_on_return_resumable(Some(Rc::new(move |resumed_epoch: u64, cx: &mut App| {
            let Some(this) = this.upgrade() else { return };
            let is_pending = this.pending_return.borrow().as_ref().is_some_and(|pending| pending.epoch == resumed_epoch);
            if !is_pending {
                return;
            }
            SharedElementOverlayDriver::return_did_become_resumable(resumed_epoch, cx);
        })));
        Some(plan)
    }

    fn resolve_return(&self, epoch: u64, cx: &mut App) -> SharedElementTargetResolution {
        let (story_id, scroller) = {
            let pending = self.pending_return.borrow();
            let Some(pending) = pending.as_ref().filter(|pending| pending.epoch == epoch) else {
                return SharedElementTargetResolution::Unavailable;
            };
            (pending.story_id, pending.scroller.upgrade())
        };
        let Some(scroller) = scroller else {
            return SharedElementTargetResolution::Unavailable;
        };
        let index = self.story_ids.borrow().iter().position(|id| *id == story_id);
        let Some(index) = index else {
            return SharedElementTargetResolution::Unavailable;
        };
        match scroller.resolve_return(epoch, self.item_content_rect(index), cx) {
            SharedElementReturnResolution::Ready { predicted_frame } => {
                SharedElementTargetResolution::Ready(predicted_frame)
            }
            SharedElementReturnResolution::Deferred => SharedElementTargetResolution::Deferred,
            SharedElementReturnResolution::Unavailable => SharedElementTargetResolution::Unavailable,
        }
    }

    fn measured_target_frame(&self, item_id: &str, cx: &App) -> Option<Bounds<Pixels>> {
        let key = self.media_key(item_id);
        self.attached_anchor_frame(&key, cx).map(|frame| self.nominal_cell_rect(frame))
    }

    fn cancel_return(&self, epoch: u64, cx: &mut App) {
        if let Some(scroller) = self.return_scroller(epoch) {
            scroller.cancel_live_return(epoch, cx);
        }
        self.clear_pending_return(epoch);
    }

    fn finish_return(&self, epoch: u64, cx: &mut App) {
        if let Some(scroller) = self.return_scroller(epoch) {
            scroller.finish_return(epoch, cx);
        }
        self.clear_pending_return(epoch);
        self.opened_story_id.set(None);
    }

    fn as_overlay_origin(self: Rc<Self>) -> Option<Rc<dyn SharedElementOverlayOrigin>> {
        Some(self)
    }
}

impl SharedElementOverlayOrigin for StoriesGridSharedElementOrigin {
    fn overlay_window(&self, cx: &App) -> Option<AnyWindowHandle> {
        self.probe_window.get().or_else(|| self.host_view(cx))
    }

    fn overlay_corner_radius(&self, _item_id: &str, _cx: &App) -> f32 {
        0.0
    }
}
