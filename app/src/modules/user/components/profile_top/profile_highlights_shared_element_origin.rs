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
    SharedElementReturnScrollMathClamp, SharedElementSurface,
};
use crate::core::shared_element_transition::overlay::shared_element_overlay_contracts::SharedElementOverlayOrigin;
use crate::core::shared_element_transition::overlay::shared_element_overlay_driver::SharedElementOverlayDriver;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementCover, SharedElementCoverContent, SharedElementFlightContent,
    SharedElementOriginContainer, SharedElementReturnPlan, SharedElementTargetResolution,
};
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementStackHost;
use crate::core::shared_element_transition::runtime::shared_element_navigation_policy::SharedElementNavigationPolicy;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::modules::story::components::story_player::story_player_shared_element_destination::{
    StorySharedElementPicture, StorySharedElementPictureResult,
};
use crate::modules::story::stores::story::story_actions::story_types::{
    StoryPlayerEntrance, StoryPlayerSharedElementOrigin,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::{HighlightDTO, StoryDTO};
use crate::modules::user::components::profile_top::profile_highlights::ProfileHighlightsLayoutPolicy;

/// overlay origin of the highlight circles,
/// no positioning. exit lands only on a circle thats on screen and opaque
pub struct ProfileHighlightsSharedElementOrigin {
    this: Weak<Self>,
    surface: RefCell<SharedElementSurface>,
    user_id: RefCell<String>,
    probe_window: Cell<Option<AnyWindowHandle>>,
    probe_window_size: Cell<Size<Pixels>>,
    covers_by_highlight_id: RefCell<HashMap<i64, StoryDTO>>,
    latest_highlights: RefCell<Rc<Vec<HighlightDTO>>>,
    pending_return: RefCell<Option<(u64, String)>>,
}

impl ProfileHighlightsSharedElementOrigin {
    const ROW_TOP_PADDING: f32 = 2.0;

    fn circle_bucket() -> i64 {
        (ProfileHighlightsLayoutPolicy::CIRCLE_SIZE * 3.0).ceil() as i64
    }

    pub fn new() -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            surface: RefCell::new(SharedElementSurface::ProfileHighlights { scope_key: String::new() }),
            user_id: RefCell::new(String::new()),
            probe_window: Cell::new(None),
            probe_window_size: Cell::new(size(px(0.0), px(0.0))),
            covers_by_highlight_id: RefCell::new(HashMap::new()),
            latest_highlights: RefCell::new(Rc::new(Vec::new())),
            pending_return: RefCell::new(None),
        })
    }

    pub fn surface(user_id: &str) -> SharedElementSurface {
        SharedElementSurface::ProfileHighlights { scope_key: user_id.to_owned() }
    }

    pub fn item_id(highlight_id: i64) -> String {
        highlight_id.to_string()
    }

    pub fn player_origin(&self, highlight_id: i64) -> StoryPlayerSharedElementOrigin {
        StoryPlayerSharedElementOrigin {
            surface: self.surface.borrow().clone(),
            item_id: Some(Self::item_id(highlight_id)),
        }
    }

    pub fn configure(&self, user_id: &str) {
        *self.user_id.borrow_mut() = user_id.to_owned();
        *self.surface.borrow_mut() = Self::surface(user_id);
    }

    pub fn record_layout(&self, window: &Window) {
        self.probe_window.set(Some(window.window_handle()));
        self.probe_window_size.set(window.viewport_size());
    }

    pub(super) fn record_latest_highlights(&self, highlights: &Rc<Vec<HighlightDTO>>) {
        *self.latest_highlights.borrow_mut() = highlights.clone();
    }

    pub fn arm_open(self: &Rc<Self>, highlight: &HighlightDTO, request_id: Uuid, cx: &mut App) -> StoryPlayerEntrance {
        if !SharedElementNavigationPolicy::stories_enabled() || self.user_id.borrow().is_empty() {
            return StoryPlayerEntrance::Slide;
        }
        self.store_cover(highlight);
        let origin: Rc<dyn SharedElementOverlayOrigin> = self.clone();
        let Some(epoch) = SharedElementOverlayDriver::open(
            &origin,
            &Self::item_id(highlight.id),
            SharedElementSurface::StoryPlayer { request_id: request_id.uuid_string() },
            cx,
        ) else {
            return StoryPlayerEntrance::Slide;
        };
        StoryPlayerEntrance::SharedElement { epoch }
    }

    fn store_cover(&self, highlight: &HighlightDTO) {
        let mut covers = self.covers_by_highlight_id.borrow_mut();
        match highlight.cover_story.as_ref() {
            Some(cover) => {
                covers.insert(highlight.id, cover.clone());
            }
            None => {
                covers.remove(&highlight.id);
            }
        }
    }

    fn cover_picture(&self, item_id: &str) -> Option<StorySharedElementPictureResult> {
        let id = int_from_string(item_id)?;
        let cover = self.covers_by_highlight_id.borrow().get(&id).cloned()?;
        StorySharedElementPicture::picture(
            &cover,
            &[Some(Self::circle_bucket()), Some(StorySharedElementPicture::player_bucket())],
            Self::circle_bucket(),
        )
    }

    fn media_key(&self, item_id: &str) -> SharedElementKey {
        SharedElementKey::new(self.surface.borrow().clone(), item_id, SharedElementPart::Media)
    }

    fn clear_pending_return(&self, epoch: u64) {
        let _ = self.pending_return.borrow_mut().take_if(|pending| pending.0 == epoch);
    }

    /// window frame of an attached circle fully inside its window,
    /// and inside a row not yet fading if requires_opaque_row
    fn circle_frame(&self, item_id: &str, requires_opaque_row: bool, cx: &App) -> Option<Bounds<Pixels>> {
        let id = self.media_key(item_id).frame_registry_key();
        let view = MediaCellScreenFrameRegistry::attached_view(&id)?;
        let frame = MediaCellScreenFrameRegistry::attached_frame(&id)?;
        let window = view.window()?;
        if let Some(host_view) = self.host_view(cx)
            && window != host_view
        {
            return None;
        }
        let viewport = self.probe_window_size.get();
        let inside_window = frame.left() >= px(0.0)
            && frame.top() >= px(0.0)
            && frame.right() <= viewport.width
            && frame.bottom() <= viewport.height;
        if !inside_window {
            return None;
        }
        if requires_opaque_row && Self::row_opacity(frame, window) < 0.99 {
            return None;
        }
        Some(frame)
    }

    fn row_opacity(frame: Bounds<Pixels>, window: AnyWindowHandle) -> f32 {
        let area = |bounds: Bounds<Pixels>| f32::from(bounds.size.width) * f32::from(bounds.size.height);
        let vertical = GuideScrollHost::live()
            .into_iter()
            .filter(|host| host.is_in_window() && host.window() == Some(window))
            .filter_map(|host| host.frame())
            .filter(|bounds| {
                f32::from(bounds.size.height) > ProfileHighlightsLayoutPolicy::ROW_HEIGHT + 1.0
                    && bounds.left() <= frame.origin.x
                    && frame.origin.x <= bounds.right()
                    && frame.top() < bounds.bottom()
                    && bounds.top() < frame.bottom()
            })
            .min_by(|lhs, rhs| area(*lhs).total_cmp(&area(*rhs)));
        let Some(vertical) = vertical else {
            return 1.0;
        };
        let row_max_y = f32::from(frame.origin.y - vertical.origin.y) - Self::ROW_TOP_PADDING
            + ProfileHighlightsLayoutPolicy::ROW_HEIGHT;
        ProfileHighlightsLayoutPolicy::scroll_opacity_at_profile_metrics(row_max_y)
    }
}

impl SharedElementOriginContainer for ProfileHighlightsSharedElementOrigin {
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
        let clip = self.circle_frame(item_id, false, cx)?;
        let picture = self.cover_picture(item_id)?;
        Some(SharedElementCapture {
            item_id: item_id.to_owned(),
            part: SharedElementPart::Media,
            source_key: self.media_key(item_id),
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
        if !SharedElementNavigationPolicy::stories_enabled() || self.user_id.borrow().is_empty() {
            return false;
        }
        let Some(id) = int_from_string(item_id) else {
            return false;
        };
        if self.circle_frame(item_id, true, cx).is_none() {
            return false;
        }
        if !self.covers_by_highlight_id.borrow().contains_key(&id) {
            let highlights = self.latest_highlights.borrow().clone();
            for highlight in highlights.iter() {
                self.store_cover(highlight);
            }
        }
        true
    }

    fn return_cover(&self, item_id: &str, _cx: &App) -> Option<SharedElementCover> {
        let picture = self.cover_picture(item_id)?;
        Some(SharedElementCover {
            image: SharedElementCoverContent::Image(picture.image),
            aspect: picture.aspect,
            identity: picture.identity,
        })
    }

    fn prepare_return(&self, item_id: &str, epoch: u64, cx: &mut App) -> Option<SharedElementReturnPlan> {
        let frame = self.circle_frame(item_id, true, cx)?;
        *self.pending_return.borrow_mut() = Some((epoch, item_id.to_owned()));
        Some(SharedElementReturnPlan {
            epoch,
            predicted_frame: frame,
            clamp: SharedElementReturnScrollMathClamp::None,
        })
    }

    fn resolve_return(&self, epoch: u64, cx: &mut App) -> SharedElementTargetResolution {
        let item_id = self
            .pending_return
            .borrow()
            .as_ref()
            .filter(|pending| pending.0 == epoch)
            .map(|pending| pending.1.clone());
        let Some(item_id) = item_id else {
            return SharedElementTargetResolution::Unavailable;
        };
        match self.circle_frame(&item_id, false, cx) {
            Some(frame) => SharedElementTargetResolution::Ready(frame),
            None => SharedElementTargetResolution::Unavailable,
        }
    }

    fn measured_target_frame(&self, item_id: &str, cx: &App) -> Option<Bounds<Pixels>> {
        self.circle_frame(item_id, false, cx)
    }

    fn cancel_return(&self, epoch: u64, _cx: &mut App) {
        self.clear_pending_return(epoch);
    }

    fn finish_return(&self, epoch: u64, _cx: &mut App) {
        self.clear_pending_return(epoch);
    }

    fn as_overlay_origin(self: Rc<Self>) -> Option<Rc<dyn SharedElementOverlayOrigin>> {
        Some(self)
    }
}

impl SharedElementOverlayOrigin for ProfileHighlightsSharedElementOrigin {
    fn overlay_window(&self, cx: &App) -> Option<AnyWindowHandle> {
        self.probe_window.get().or_else(|| self.host_view(cx))
    }

    fn overlay_corner_radius(&self, _item_id: &str, _cx: &App) -> f32 {
        ProfileHighlightsLayoutPolicy::CIRCLE_SIZE / 2.0
    }
}
