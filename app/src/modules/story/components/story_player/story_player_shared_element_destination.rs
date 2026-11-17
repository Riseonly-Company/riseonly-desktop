use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::{App, AppContext, Bounds, Entity, Pixels, Point, WeakEntity, point, px, size};
use rise_core::foundation::foundation_date::RiseClock;
use rise_media::video::av_player::{AVLayerVideoGravity, AVPlayer};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementGeometry, SharedElementKey, SharedElementPart, SharedElementSurface,
};
use crate::core::shared_element_transition::overlay::shared_element_overlay_contracts::{
    SharedElementOverlayDestination, SharedElementOverlayPhase, SharedElementOverlayProbeAnchor,
};
use crate::core::shared_element_transition::overlay::shared_element_overlay_driver::{
    SharedElementOverlayDriver, SharedElementOverlayExitRequest,
};
use crate::core::shared_element_transition::renderer::shared_element_bitmap_resolver::SharedElementBitmapResolver;
use crate::core::shared_element_transition::renderer::shared_element_live_video_view::SharedElementLiveVideoView;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementFlightContent, SharedElementLiveReadiness,
};
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::story::components::stories_grid::stories_grid_shared_element_origin::StoriesGridSharedElementOrigin;
use crate::modules::story::components::story_player::story_video_segment::StoryVideoSegment;
use crate::modules::story::shared::story_poster_cache::StoryPosterCache;
use crate::modules::story::stores::story::story_actions::story_types::{
    StoryPlaybackPolicy, StoryPlayerSharedElementOrigin,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::{StoryDTO, StoryDTOExt};

pub struct StorySharedElementPictureResult {
    pub image: CleverImage,
    pub aspect: f32,
    pub identity: String,
}

/// memory only pictures of a story for flights and covers
pub enum StorySharedElementPicture {}

impl StorySharedElementPicture {
    pub fn player_bucket() -> i64 {
        StoryPlaybackPolicy::IMAGE_MAXIMUM_PIXEL_DIMENSION
    }

    pub fn is_video(story: &StoryDTO) -> bool {
        story.media_type.to_lowercase() == "video"
    }

    pub fn source(story: &StoryDTO) -> String {
        story.media_source()
    }

    pub fn poster_identity(story_id: i64) -> String {
        format!("story-poster:{story_id}")
    }

    /// first frame poster of a video story from the grid cell
    pub fn poster(story_id: i64) -> Option<StorySharedElementPictureResult> {
        let image = StoryPosterCache::cached(story_id)?;
        let pixels = image.size(0);
        if pixels.width.0 <= 0 || pixels.height.0 <= 0 {
            return None;
        }
        Some(StorySharedElementPictureResult {
            aspect: pixels.width.0 as f32 / pixels.height.0 as f32,
            image,
            identity: Self::poster_identity(story_id),
        })
    }

    /// photos draw persisted to disk with the standard policy,
    /// a decode far under minimum_longest would fly soft
    pub fn photo(
        source: &str,
        buckets: &[Option<i64>],
        minimum_longest: i64,
    ) -> Option<StorySharedElementPictureResult> {
        if source.is_empty() {
            return None;
        }
        let bitmap =
            SharedElementBitmapResolver::memory_bitmap(source, true, CleverImageCachePolicy::Standard, buckets)?;
        let pixels = bitmap.image.size(0);
        if i64::from(pixels.width.0.max(pixels.height.0)) * 2 < minimum_longest {
            return None;
        }
        Some(StorySharedElementPictureResult { image: bitmap.image, aspect: bitmap.aspect, identity: bitmap.source })
    }

    pub fn picture(
        story: &StoryDTO,
        buckets: &[Option<i64>],
        minimum_longest: i64,
    ) -> Option<StorySharedElementPictureResult> {
        if Self::is_video(story) {
            return Self::poster(story.id);
        }
        Self::photo(&Self::source(story), buckets, minimum_longest)
    }
}

pub type StoryPlayerSharedElementPhaseHandler = Rc<dyn Fn(SharedElementOverlayPhase, &mut App)>;

/// overlay destination of one story player request,
/// a plain cache the player writes from media callbacks
/// nothing here is observed
pub struct StoryPlayerSharedElementDestination {
    surface: SharedElementSurface,
    pub probe: SharedElementOverlayProbeAnchor,
    media_size: Cell<gpui::Size<Pixels>>,
    on_phase: RefCell<Option<StoryPlayerSharedElementPhaseHandler>>,
    interactive_exit: RefCell<Weak<SharedElementOverlayDriver>>,

    story_id: Cell<Option<i64>>,
    source: RefCell<String>,
    is_video: Cell<bool>,
    video_player: RefCell<Option<WeakEntity<AVPlayer>>>,
    video_story_id: Cell<Option<i64>>,
    video_ready: Cell<bool>,
    video_aspect: Cell<Option<f32>>,
    photo_story_id: Cell<Option<i64>>,
    photo_aspect: Cell<Option<f32>>,
    photo_fills: Cell<bool>,
}

impl StoryPlayerSharedElementDestination {
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            surface: SharedElementSurface::StoryPlayer { request_id: request_id.into() },
            probe: SharedElementOverlayProbeAnchor::new(),
            media_size: Cell::new(size(px(0.0), px(0.0))),
            on_phase: RefCell::new(None),
            interactive_exit: RefCell::new(Weak::new()),
            story_id: Cell::new(None),
            source: RefCell::new(String::new()),
            is_video: Cell::new(false),
            video_player: RefCell::new(None),
            video_story_id: Cell::new(None),
            video_ready: Cell::new(false),
            video_aspect: Cell::new(None),
            photo_story_id: Cell::new(None),
            photo_aspect: Cell::new(None),
            photo_fills: Cell::new(true),
        }
    }

    pub fn media_size(&self) -> gpui::Size<Pixels> {
        self.media_size.get()
    }

    pub fn set_media_size(&self, media_size: gpui::Size<Pixels>) {
        self.media_size.set(media_size);
    }

    pub fn set_on_phase(&self, on_phase: Option<StoryPlayerSharedElementPhaseHandler>) {
        *self.on_phase.borrow_mut() = on_phase;
    }

    pub fn interactive_exit(&self) -> Option<Rc<SharedElementOverlayDriver>> {
        self.interactive_exit.borrow().upgrade()
    }

    pub fn set_interactive_exit(&self, interactive_exit: Option<&Rc<SharedElementOverlayDriver>>) {
        *self.interactive_exit.borrow_mut() = interactive_exit.map_or_else(Weak::new, Rc::downgrade);
    }

    pub fn set_current(&self, story_id: Option<i64>, source: &str, is_video: bool) {
        self.story_id.set(story_id);
        *self.source.borrow_mut() = source.to_owned();
        self.is_video.set(is_video);
    }

    pub fn record_video(&self, story_id: i64, player: Option<&Entity<AVPlayer>>) {
        if let Some(player) = player {
            if self.video_story_id.get() != Some(story_id) {
                self.video_aspect.set(None);
            }
            *self.video_player.borrow_mut() = Some(player.downgrade());
            self.video_story_id.set(Some(story_id));
            self.video_ready.set(false);
        } else if self.video_story_id.get() == Some(story_id) {
            *self.video_player.borrow_mut() = None;
            self.video_ready.set(false);
        }
    }

    pub fn record_video_ready(&self, story_id: i64, ready: bool) {
        if self.video_story_id.get() != Some(story_id) {
            return;
        }
        self.video_ready.set(ready);
    }

    pub fn record_video_aspect(&self, story_id: i64, aspect: f32) {
        if !aspect.is_finite() || aspect <= 0.0 || self.video_story_id.get() != Some(story_id) {
            return;
        }
        self.video_aspect.set(Some(aspect));
    }

    pub fn record_photo_geometry(&self, story_id: i64, aspect: Option<f32>, fills: bool) {
        self.photo_story_id.set(Some(story_id));
        self.photo_aspect.set(aspect);
        self.photo_fills.set(fills);
    }

    pub fn begin_exit(
        self: &Rc<Self>,
        origin: &StoryPlayerSharedElementOrigin,
        story_id: i64,
        drag_offset: f32,
        touch_start: Option<Point<Pixels>>,
        cx: &mut App,
    ) -> Option<Rc<SharedElementOverlayDriver>> {
        if self.story_id.get() != Some(story_id) {
            return None;
        }
        let window = self.probe.window()?;
        let base = self.media_frame(cx)?;
        let clip = Bounds::new(point(base.origin.x, base.origin.y + px(drag_offset.max(0.0))), base.size);
        let capture = self.exit_capture(story_id, clip, cx);
        let destination: Rc<dyn SharedElementOverlayDestination> = self.clone();
        SharedElementOverlayDriver::begin_exit(
            SharedElementOverlayExitRequest {
                destination,
                origin_surface: origin.surface.clone(),
                origin_item_id: origin
                    .item_id
                    .clone()
                    .unwrap_or_else(|| StoriesGridSharedElementOrigin::item_id(story_id)),
                capture,
                window,
                touch_start,
            },
            cx,
        )
    }

    fn fills_video(aspect: f32, clip: Bounds<Pixels>) -> bool {
        StoryVideoSegment::fills(Some(aspect), size(f32::from(clip.size.width), f32::from(clip.size.height)))
    }

    fn content_rect(fills: bool, aspect: f32, clip: Bounds<Pixels>) -> Bounds<Pixels> {
        if fills {
            SharedElementGeometry::fill_rect(aspect, clip)
        } else {
            SharedElementGeometry::fit_rect(aspect, clip)
        }
    }

    fn landing_mode(fills: bool) -> SharedElementContentMode {
        if fills { SharedElementContentMode::Fill } else { SharedElementContentMode::Fit { backdrop: false } }
    }

    fn exit_capture(&self, story_id: i64, clip: Bounds<Pixels>, cx: &mut App) -> Option<SharedElementCapture> {
        let item_id = StoriesGridSharedElementOrigin::item_id(story_id);
        let key = SharedElementKey::new(self.surface.clone(), item_id.clone(), SharedElementPart::Media);
        let clip_aspect = f32::from(clip.size.width) / f32::from(clip.size.height).max(1.0);
        let bitmap_capture = |picture: StorySharedElementPictureResult, fills: bool| SharedElementCapture {
            item_id: item_id.clone(),
            part: SharedElementPart::Media,
            source_key: key.clone(),
            clip,
            content_rect: Self::content_rect(fills, picture.aspect, clip),
            content: SharedElementFlightContent::Bitmap(picture.image, picture.aspect),
            mode: Self::landing_mode(fills),
            backdrop: None,
            flying_identity: Some(picture.identity),
            stop_frame: None,
            live_readiness: None,
            placeholder: None,
        };
        if self.is_video.get() {
            let poster = StorySharedElementPicture::poster(story_id);
            let player = self.video_player.borrow().as_ref().and_then(WeakEntity::upgrade);
            if self.video_story_id.get() == Some(story_id)
                && let Some(player) = player
            {
                let aspect = self
                    .video_aspect
                    .get()
                    .or_else(|| Self::presentation_aspect(&player, cx))
                    .or_else(|| poster.as_ref().map(|poster| poster.aspect))
                    .unwrap_or(clip_aspect);
                let fills = Self::fills_video(aspect, clip);
                let content_rect = Self::content_rect(fills, aspect, clip);
                let readiness = Rc::new(SharedElementLiveReadiness::new(RiseClock::monotonic()));
                let downgraded = Rc::downgrade(&readiness);
                let live_view = cx.new(|cx| {
                    let mut live_view =
                        SharedElementLiveVideoView::new(&player, AVLayerVideoGravity::ResizeAspect, cx);
                    live_view.set_readiness(Some(downgraded), cx);
                    live_view
                });
                return Some(SharedElementCapture {
                    item_id: item_id.clone(),
                    part: SharedElementPart::Media,
                    source_key: key.clone(),
                    clip,
                    content: SharedElementFlightContent::Replicant(live_view.into(), content_rect.size),
                    content_rect,
                    mode: Self::landing_mode(fills),
                    backdrop: None,
                    flying_identity: None,
                    stop_frame: None,
                    live_readiness: Some(readiness),
                    placeholder: poster.map(|poster| poster.image),
                });
            }
            let poster = poster?;
            let poster_fills = Self::fills_video(poster.aspect, clip);
            return Some(bitmap_capture(poster, poster_fills));
        }
        let picture = StorySharedElementPicture::photo(
            &self.source.borrow(),
            &[Some(StorySharedElementPicture::player_bucket())],
            1,
        )?;
        let fills = if self.photo_story_id.get() == Some(story_id) && self.photo_aspect.get().is_some() {
            self.photo_fills.get()
        } else {
            picture.aspect < clip_aspect
        };
        Some(bitmap_capture(picture, fills))
    }

    fn presentation_aspect(player: &Entity<AVPlayer>, cx: &App) -> Option<f32> {
        let presentation_size = player.read(cx).presentation_size();
        if presentation_size.width <= 0.0 || presentation_size.height <= 0.0 {
            return None;
        }
        Some(presentation_size.width / presentation_size.height)
    }
}

impl SharedElementOverlayDestination for StoryPlayerSharedElementDestination {
    fn surface(&self) -> SharedElementSurface {
        self.surface.clone()
    }

    fn media_frame(&self, _cx: &App) -> Option<Bounds<Pixels>> {
        let origin = self.probe.screen_origin()?;
        let media_size = self.media_size.get();
        if media_size.width <= px(1.0) || media_size.height <= px(1.0) {
            return None;
        }
        Some(Bounds::new(origin, media_size))
    }

    fn open_landing_mode(&self, content_aspect: f32, clip: Bounds<Pixels>, _cx: &App) -> SharedElementContentMode {
        let clip_height = f32::from(clip.size.height);
        if clip_height <= 0.0 {
            return SharedElementContentMode::Fill;
        }
        if self.is_video.get() {
            return Self::landing_mode(Self::fills_video(content_aspect, clip));
        }
        Self::landing_mode(content_aspect < f32::from(clip.size.width) / clip_height)
    }

    fn current_item_id(&self, _cx: &App) -> Option<String> {
        self.story_id.get().map(StoriesGridSharedElementOrigin::item_id)
    }

    fn media_is_ready(&self, item_id: &str, _cx: &App) -> bool {
        let Some(story_id) = self.story_id.get() else {
            return false;
        };
        if StoriesGridSharedElementOrigin::item_id(story_id) != item_id {
            return false;
        }
        if self.is_video.get() {
            return self.video_story_id.get() == Some(story_id) && self.video_ready.get();
        }
        if self.photo_story_id.get() != Some(story_id) || self.photo_aspect.get().is_none() {
            return false;
        }
        StorySharedElementPicture::photo(
            &self.source.borrow(),
            &[Some(StorySharedElementPicture::player_bucket())],
            1,
        )
        .is_some()
    }

    fn apply_overlay_phase(&self, phase: SharedElementOverlayPhase, _epoch: u64, cx: &mut App) {
        let on_phase = self.on_phase.borrow().clone();
        if let Some(on_phase) = on_phase {
            on_phase(phase, cx);
        }
    }
}
