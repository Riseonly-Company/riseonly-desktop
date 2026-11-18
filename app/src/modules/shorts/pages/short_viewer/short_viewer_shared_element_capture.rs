use std::rc::Rc;

use gpui::{AnyWindowHandle, App, AppContext as _, Bounds, Pixels};
use rise_media::video::av_player::AVLayerVideoGravity;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementGeometry, SharedElementKey, SharedElementPart,
};
use crate::core::shared_element_transition::renderer::shared_element_bitmap_resolver::{
    SharedElementBitmapResolver, SharedElementBitmapResolverBitmap,
};
use crate::core::shared_element_transition::renderer::shared_element_live_video_view::SharedElementLiveVideoView;
use crate::core::shared_element_transition::renderer::shared_element_video_frame_grabber::{
    SharedElementPlayerLayerView, SharedElementVideoFrameGrabber,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCapture, SharedElementFlightContent, SharedElementLiveReadiness,
};
use crate::core::ui::clever_image_disk_cache::CleverImageBitmapLoader;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::comment::stores::comment_services::comments_sheet_motion_relay::CommentsSheetMotionRelay;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::shorts_feed_cell::ShortsCellMedia;
use crate::modules::shorts::components::shorts_feed_player::ShortsFeedPlayer;
use crate::modules::shorts::components::shorts_media_prefetch::ShortsMediaPrefetchPolicy;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::ProfileGridPreviewSourcePolicy;

/// host view of the shared element, gpui has no view tree so the destination keeps it here
#[derive(Clone)]
pub struct ShortViewerSharedElementHostView {
    pub window: AnyWindowHandle,
    pub frame: Bounds<Pixels>,
    pub scale: f32,
    pub player_layers: Vec<SharedElementPlayerLayerView>,
    pub is_in_moving_scroll: bool,
}

/// what a riff viewer page flies with, from frame registry and player layer and image caches
/// never a snapshot since a player layer snapshots black
pub enum ShortViewerSharedElementCapture {}

impl ShortViewerSharedElementCapture {
    pub const LIVE_VIDEO_IDENTITY: &'static str = "live";

    const PAGE_TOLERANCE: f32 = 1.0;
    const MINIMUM_SHARPNESS: f32 = 0.5;

    /// screen frame of a page anchor under the host view, only while that page fills the viewer
    pub fn page_frame(key: &str, host_view: &ShortViewerSharedElementHostView) -> Option<Bounds<Pixels>> {
        let view = MediaCellScreenFrameRegistry::attached_view(key)?;
        if view.window()? != host_view.window {
            return None;
        }
        let frame = MediaCellScreenFrameRegistry::attached_frame(key)?;
        let viewport = host_view.frame;
        if (f32::from(frame.origin.y) - f32::from(viewport.origin.y)).abs() > Self::PAGE_TOLERANCE
            || (f32::from(frame.size.height) - f32::from(viewport.size.height)).abs() > Self::PAGE_TOLERANCE
        {
            return None;
        }
        Some(frame)
    }

    /// none while comments shrink the reel, while a scroll above the page moves and when nothing may fly
    pub fn capture(
        post: &GetPostFeedResponse,
        slide: i64,
        source_key: &SharedElementKey,
        host_view: &ShortViewerSharedElementHostView,
        cx: &mut App,
    ) -> Option<SharedElementCapture> {
        let relay = CommentsSheetMotionRelay::shared(cx);
        if relay.read(cx).engaged_post_id().is_some() || relay.read(cx).progress() > 0.0 {
            tracing::warn!(target: "riseonly::X1625", "viewer capture refused: comments engaged");
            return None;
        }
        let Some(clip) = Self::page_frame(&source_key.frame_registry_key(), host_view) else {
            tracing::warn!(
                target: "riseonly::X1625",
                item = %source_key.item_id,
                attached = MediaCellScreenFrameRegistry::attached_frame(&source_key.frame_registry_key()).is_some(),
                "viewer capture refused: page not filling the viewer"
            );
            return None;
        };
        if host_view.is_in_moving_scroll {
            tracing::warn!(target: "riseonly::X1625", item = %source_key.item_id, "viewer capture refused: pager moving");
            return None;
        }
        let media = ShortsCellMedia::new(post, Some(slide));
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        if media.shows_video() {
            return Self::video_capture(post, &media, clip, source_key, host_view, is_own, cx);
        }
        let Some(PostMediaSlide::Image { url, .. }) = media.current_slide() else {
            return None;
        };
        let Some(bitmap) = Self::sharp_bitmap(url, is_own, clip, host_view.scale) else {
            tracing::warn!(target: "riseonly::X1625", item = %source_key.item_id, "viewer capture refused: photo not sharp in memory");
            return None;
        };
        let fills = ShortsFeedPlayer::fills(bitmap.aspect, clip.size);
        let flying_identity = ProfileGridPreviewSourcePolicy::source(None, Some(url.as_str()), None)
            .unwrap_or_else(|| bitmap.source.clone());
        Some(SharedElementCapture {
            item_id: source_key.item_id.clone(),
            part: SharedElementPart::Media,
            source_key: source_key.clone(),
            clip,
            content: bitmap.flight_content(),
            content_rect: if fills {
                SharedElementGeometry::fill_rect(bitmap.aspect, clip)
            } else {
                SharedElementGeometry::fit_rect(bitmap.aspect, clip)
            },
            mode: if fills {
                SharedElementContentMode::Fill
            } else {
                SharedElementContentMode::Fit { backdrop: false }
            },
            backdrop: None,
            flying_identity: Some(flying_identity),
            stop_frame: None,
            live_readiness: None,
            placeholder: None,
        })
    }

    pub fn landing_is_ready(
        post: &GetPostFeedResponse,
        host_view: Option<&ShortViewerSharedElementHostView>,
        cx: &App,
    ) -> bool {
        let media = ShortsCellMedia::new(post, Some(0));
        if media.shows_video()
            && let Some(host_view) = host_view
            && let Some(player_view) =
                SharedElementVideoFrameGrabber::player_layer_view(&host_view.player_layers, host_view.frame)
            && let Some(player) = player_view.player()
            && player.read(cx).is_ready_for_display()
        {
            return true;
        }
        let Some(source) = Self::still_source(post, &media) else {
            return true;
        };
        let viewer = AuthServicesStore::try_read(cx).and_then(|store| store.effective_user_id_for_wire());
        let is_own = RiseIdentity::is_own(
            &RiseIdentity::author(Some(post.author_id.as_str()), None, post.author.id.as_deref()),
            viewer.as_deref(),
            None,
        );
        let user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        CleverImageBitmapLoader::memory_image(
            &source,
            is_own,
            user_id.as_deref(),
            account_epoch.as_deref(),
            Some(ShortsMediaPrefetchPolicy::POSTER_PIXEL_DIMENSION),
            PostMediaCachePolicy::image_policy(is_own),
            None,
            false,
        )
        .is_some()
    }

    fn video_capture(
        post: &GetPostFeedResponse,
        media: &ShortsCellMedia,
        clip: Bounds<Pixels>,
        source_key: &SharedElementKey,
        host_view: &ShortViewerSharedElementHostView,
        is_own: bool,
        cx: &mut App,
    ) -> Option<SharedElementCapture> {
        let still = Self::still_source(post, media);
        let poster = still
            .as_deref()
            .and_then(|source| Self::sharp_bitmap(source, is_own, clip, host_view.scale));
        let live = SharedElementVideoFrameGrabber::player_layer_view(&host_view.player_layers, clip).and_then(|view| {
            let frame = view.frame()?;
            let player = view.player()?;
            player.read(cx).is_ready_for_display().then_some((frame, player))
        });
        let Some((frame, player)) = live else {
            let poster = poster.or_else(|| still.as_deref().and_then(|source| Self::memory_bitmap(source, is_own)));
            if poster.is_none() {
                tracing::warn!(target: "riseonly::X1625", item = %source_key.item_id, "viewer capture refused: no live frame, no poster in memory");
            }
            return poster.map(|poster| Self::poster_capture(post, &poster, clip, source_key));
        };
        let presentation = player.read(cx).presentation_size();
        let aspect = (presentation.width > 0.0 && presentation.height > 0.0)
            .then(|| presentation.width / presentation.height);
        let fills = aspect.is_none_or(|aspect| ShortsFeedPlayer::fills(aspect, clip.size));
        let gravity = if fills { AVLayerVideoGravity::ResizeAspectFill } else { AVLayerVideoGravity::ResizeAspect };
        let content_rect = match aspect {
            Some(aspect) if fills => SharedElementGeometry::fill_rect(aspect, frame),
            Some(aspect) => SharedElementGeometry::fit_rect(aspect, frame),
            None => frame,
        };
        if f32::from(content_rect.size.width) <= 1.0 || f32::from(content_rect.size.height) <= 1.0 {
            return None;
        }

        let readiness = Rc::new(SharedElementLiveReadiness::new(RiseClock::monotonic()));
        let live_view = cx.new(|cx| {
            let mut view = SharedElementLiveVideoView::new(&player, gravity, cx);
            view.set_readiness(Some(Rc::downgrade(&readiness)), cx);
            view
        });
        Some(SharedElementCapture {
            item_id: source_key.item_id.clone(),
            part: SharedElementPart::Media,
            source_key: source_key.clone(),
            clip,
            content: SharedElementFlightContent::Replicant(live_view.into(), content_rect.size),
            content_rect,
            mode: if fills {
                SharedElementContentMode::Fill
            } else {
                SharedElementContentMode::Fit { backdrop: false }
            },
            backdrop: None,
            flying_identity: Some(Self::LIVE_VIDEO_IDENTITY.to_owned()),
            stop_frame: None,
            live_readiness: Some(readiness),
            placeholder: poster.map(|poster| poster.image),
        })
    }

    fn poster_capture(
        post: &GetPostFeedResponse,
        poster: &SharedElementBitmapResolverBitmap,
        clip: Bounds<Pixels>,
        source_key: &SharedElementKey,
    ) -> SharedElementCapture {
        let fills = ShortsFeedPlayer::fills(poster.aspect, clip.size);
        let flying_identity = ProfileGridPreviewSourcePolicy::source(None, None, post.video_thumbnail.as_deref())
            .unwrap_or_else(|| poster.source.clone());
        SharedElementCapture {
            item_id: source_key.item_id.clone(),
            part: SharedElementPart::Media,
            source_key: source_key.clone(),
            clip,
            content: poster.flight_content(),
            content_rect: if fills {
                SharedElementGeometry::fill_rect(poster.aspect, clip)
            } else {
                SharedElementGeometry::fit_rect(poster.aspect, clip)
            },
            mode: if fills {
                SharedElementContentMode::Fill
            } else {
                SharedElementContentMode::Fit { backdrop: false }
            },
            backdrop: None,
            flying_identity: Some(flying_identity),
            stop_frame: None,
            live_readiness: None,
            placeholder: None,
        }
    }

    /// the still the page draws at viewer decode size, the photo or the player poster cover
    fn still_source(post: &GetPostFeedResponse, media: &ShortsCellMedia) -> Option<String> {
        let source = match media.current_slide() {
            Some(PostMediaSlide::Image { url, .. }) => url.clone(),
            Some(PostMediaSlide::Video { url, thumbnail, .. }) => {
                RemoteMediaURL::display_source(url, thumbnail.as_deref())
            }
            None => {
                let url = media.single_video_url.as_deref()?;
                RemoteMediaURL::display_source(url, post.video_thumbnail.as_deref())
            }
        };
        let trimmed = trimming_whitespaces_and_newlines(&source);
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    fn memory_bitmap(source: &str, is_own: bool) -> Option<SharedElementBitmapResolverBitmap> {
        SharedElementBitmapResolver::bitmap(
            &[source.to_owned()],
            is_own,
            &[Some(ShortsMediaPrefetchPolicy::POSTER_PIXEL_DIMENSION), None],
        )
    }

    /// memory only, viewer bucket then full size and any live decode if sharp enough
    fn sharp_bitmap(
        source: &str,
        is_own: bool,
        clip: Bounds<Pixels>,
        scale: f32,
    ) -> Option<SharedElementBitmapResolverBitmap> {
        let bitmap = SharedElementBitmapResolver::bitmap(
            &[source.to_owned()],
            is_own,
            &[Some(ShortsMediaPrefetchPolicy::POSTER_PIXEL_DIMENSION), None],
        )?;
        let needed = f32::from(clip.size.width).max(f32::from(clip.size.height)) * scale.max(1.0) * Self::MINIMUM_SHARPNESS;
        let pixels = bitmap.image.size(0);
        let longest = pixels.width.0.max(pixels.height.0) as f32;
        (longest >= needed).then_some(bitmap)
    }
}

