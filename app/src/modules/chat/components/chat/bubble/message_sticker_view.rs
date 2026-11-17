use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AccessibleAction, AnyElement, App, AppContext, Bounds, Context, ElementId, Entity, ImageSource,
    InteractiveElement, IntoElement, ObjectFit, ParentElement, Pixels, Render, RenderImage, RenderOnce, Role,
    SharedString, Size, StatefulInteractiveElement, Styled, StyledImage, Subscription, Task, WeakEntity, Window,
    black, canvas, div, img, point, px, size,
};
use image::{Frame, RgbaImage};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_i18n::tr;
use rise_media::video::av_player::{
    AVLayerVideoGravity, AVPlayerActionAtItemEnd, AVPlayerItem, AVPlayerLayer, AVPlayerLooper, AVQueuePlayer,
};
use rise_ui::animated_media_playback_scope::{
    AnimatedMediaViewportPlaybackPolicy, AnimatedMediaViewportScopeProviding, is_animated_media_playback_active,
    nearest_animated_media_viewport_scope_view,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::alpha_shadow::{AlphaShadow, AlphaShadowModifiers};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, use_task};
use rise_ui::swiftui::stacks::{Alignment, zstack};
use smallvec::SmallVec;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheLimits, RemoteMediaDiskCacheMaterializeOptions, RemoteMediaFileLease,
};
use crate::core::media::rise_media_streaming_asset::RiseMediaStreamingAsset;
use crate::core::media::rise_media_transfer_hub::RiseMediaCacheIdentity;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::ui::animated_gif_view::{AnimatedGifContentMode, AnimatedGifView};
use crate::core::ui::animated_media_playback_coordinator::{
    AnimatedMediaParticipantId, AnimatedMediaPlaybackCoordinator, AnimatedMediaPlaybackKind,
};
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::lottie_sticker::lottie_sticker_prefetch::LottieStickerPrefetch;
use crate::core::ui::lottie_sticker::lottie_sticker_view::LottieStickerView;
use crate::core::ui::lottie_sticker::sticker_premium_effect::{
    STICKER_PREMIUM_EFFECT_DID_START, StickerPremiumEffectAnchor, StickerPremiumEffectAnchorView,
    StickerPremiumEffectPlayer, StickerPremiumEffectRegistry,
};
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::{
    RISE_LOTTIE_FIRST_FRAME_CACHE_DID_CHANGE, RiseLottieFirstFrameCache,
};
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::bubble::message_sticker_view_hooks::{
    MessageStickerViewHooks, MessageStickerViewPackSheetHooks,
};
use crate::modules::chat::components::chat::bubble::web_m_sticker_playback::{
    WebMStickerFile, WebMStickerLayer, WebMStickerPlayback, WebMStickerSupport, WebMStickerTrackStore,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageMediaItem, MessageMediaKind, MessageStickerKind,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    GifItem, PackKind, StickerAssetType, StickerAssetTypeInfer, StickerItem,
};

const STICKER_SHADOW_OPACITY: f32 = 0.18;
const STICKER_SHADOW_RADIUS: f32 = 6.0;
const STICKER_SHADOW_OFFSET_Y: f32 = 3.0;
const LOTTIE_FIRST_FRAME_SIDE: f32 = 180.0;
const EMOJI_FALLBACK: &str = "🙂";
const EMOJI_FALLBACK_MAXIMUM_SIZE: f32 = 96.0;
const EMOJI_FALLBACK_SIZE_FRACTION: f32 = 0.54;
const DECODE_SCALE: f32 = 3.0;
const PREFERRED_FORWARD_BUFFER_DURATION: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatAnimatedMediaPlaybackKind {
    AnimatedImage,
    AvFoundationVideo,
    WebMVideo,
    UnsupportedVideo,
    Unknown,
}

pub enum ChatAnimatedMediaSourcePolicy {}

impl ChatAnimatedMediaSourcePolicy {
    pub fn playback_kind(source: &str, mime_type: Option<&str>, codec: Option<&str>) -> ChatAnimatedMediaPlaybackKind {
        let mime = mime_type
            .and_then(|value| first_piece(value, ';'))
            .map(|value| trimming_whitespaces_and_newlines(value).to_lowercase());
        let family = Self::codec_family(codec);

        match mime.as_deref() {
            Some("image/gif" | "image/apng" | "image/x-apng") => return ChatAnimatedMediaPlaybackKind::AnimatedImage,
            _ if family == Some(CodecFamily::Undecodable) => return ChatAnimatedMediaPlaybackKind::UnsupportedVideo,
            Some("video/webm" | "video/x-webm" | "application/webm") => {
                return ChatAnimatedMediaPlaybackKind::WebMVideo;
            }
            _ => {}
        }

        match family {
            Some(CodecFamily::WebM) => return ChatAnimatedMediaPlaybackKind::WebMVideo,
            Some(CodecFamily::Undecodable) => return ChatAnimatedMediaPlaybackKind::UnsupportedVideo,
            Some(CodecFamily::AvFoundation) => return ChatAnimatedMediaPlaybackKind::AvFoundationVideo,
            None => {}
        }

        match mime.as_deref() {
            Some("video/mp4" | "video/quicktime" | "video/x-m4v" | "video/m4v" | "application/mp4") => {
                return ChatAnimatedMediaPlaybackKind::AvFoundationVideo;
            }
            Some(value) if value.starts_with("video/") => return ChatAnimatedMediaPlaybackKind::UnsupportedVideo,
            _ => {}
        }

        match Self::path_extension(source).as_str() {
            "gif" | "apng" => ChatAnimatedMediaPlaybackKind::AnimatedImage,
            "mp4" | "mov" | "m4v" => ChatAnimatedMediaPlaybackKind::AvFoundationVideo,
            "webm" => ChatAnimatedMediaPlaybackKind::WebMVideo,
            _ => ChatAnimatedMediaPlaybackKind::Unknown,
        }
    }

    /// path extension of the url, else of the raw text
    fn path_extension(source: &str) -> String {
        if let Some(url) = Url::from_string(source) {
            let extension = url.path_extension();
            if !extension.is_empty() {
                return extension.to_lowercase();
            }
        }
        Path::new(source)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default()
    }

    fn codec_family(raw_value: Option<&str>) -> Option<CodecFamily> {
        let raw_value = raw_value?;
        let mut value = first_piece(raw_value, ',')
            .and_then(|value| first_piece(value, ';'))
            .map(|value| trimming_whitespaces_and_newlines(value).to_lowercase())
            .unwrap_or_default();
        value = trim_quotes(&value).to_owned();
        if let Some(rest) = value.strip_prefix("codec=") {
            value = trim_quotes(rest).to_owned();
        }
        if let Some(rest) = value.strip_prefix("video/") {
            value = rest.to_owned();
        }

        if Self::matches_codec(&value, &["vp8", "vp08"]) {
            return Some(CodecFamily::Undecodable);
        }
        if Self::matches_codec(&value, &["vp9", "vp09"]) {
            return Some(CodecFamily::WebM);
        }
        if Self::matches_codec(
            &value,
            &["h264", "h.264", "avc", "avc1", "avc3", "hevc", "h265", "h.265", "hvc1", "hev1"],
        ) {
            return Some(CodecFamily::AvFoundation);
        }
        None
    }

    fn matches_codec(value: &str, names: &[&str]) -> bool {
        names.iter().any(|name| {
            value == *name || value.strip_prefix(*name).is_some_and(|rest| rest.starts_with('.'))
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodecFamily {
    AvFoundation,
    WebM,
    Undecodable,
}

/// first piece before separator that has text, empty pieces are skipped
fn first_piece(value: &str, separator: char) -> Option<&str> {
    value.split(separator).find(|piece| !piece.is_empty())
}

/// trims single and double quotes from both ends
fn trim_quotes(value: &str) -> &str {
    value.trim_matches(|character| character == '"' || character == '\'')
}

pub enum MessageStickerLayout {}

impl MessageStickerLayout {
    pub const META_OVERHANG: f32 = 3.0;

    pub const CONTEXT_HEADER_SPACING: f32 = 6.0;

    pub fn context_header_max_width(sticker_width: f32, available_width: f32) -> f32 {
        72.0_f32.max(168.0_f32.min(available_width - sticker_width - Self::CONTEXT_HEADER_SPACING))
    }

    pub fn bounding_side(item: &MessageMediaItem) -> f32 {
        match item.sticker_kind {
            Some(MessageStickerKind::Static) | None => 184.0,
            Some(MessageStickerKind::Animated) | Some(MessageStickerKind::Lottie) => 180.0,
        }
    }

    pub fn display_size(item: &MessageMediaItem) -> Size<Pixels> {
        let side = Self::bounding_side(item);
        let width = item.width;
        let height = item.height;
        if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
            return size(px(side), px(side));
        }
        let scale = (side / width).min(side / height);
        size(px((width * scale).floor().max(1.0)), px((height * scale).floor().max(1.0)))
    }
}

fn sticker_shadow() -> AlphaShadow {
    AlphaShadow {
        color: black().opacity(STICKER_SHADOW_OPACITY),
        radius: STICKER_SHADOW_RADIUS,
        x: 0.0,
        y: STICKER_SHADOW_OFFSET_Y,
    }
}

fn lottie_first_frame_point_size(side: f32) -> Size<Pixels> {
    size(px(side), px(side))
}

/// row stand in while the list scrolls fast
/// first frame or thumbnail already in memory at the bubble place and size with the shadow, never loads
pub struct RiseMessageStickerFastScrollPlaceholderView {
    content_view: Option<CleverImage>,
    display_size: Size<Pixels>,
    is_own: bool,
    reserves_avatar_space: bool,
    top_padding: f32,
    lottie_url: Option<String>,
    first_frame_observer: Option<RiseSubscription>,
    first_frame_task: Option<Task<()>>,
}

impl RiseMessageStickerFastScrollPlaceholderView {
    pub fn new(
        item: &MessageMediaItem,
        is_own: bool,
        reserves_avatar_space: bool,
        top_padding: f32,
        cx: &mut Context<Self>,
    ) -> Self {
        let lottie_url = (item.sticker_kind == Some(MessageStickerKind::Lottie)).then(|| item.url.clone());
        let mut this = Self {
            content_view: Self::memory_thumbnail(item),
            display_size: MessageStickerLayout::display_size(item),
            is_own,
            reserves_avatar_space,
            top_padding,
            lottie_url,
            first_frame_observer: None,
            first_frame_task: None,
        };
        if this.content_view.is_none() && this.lottie_url.is_some() {
            this.observe_first_frame_cache(cx);
            LottieStickerPrefetch::warm(std::slice::from_ref(&item.url), cx);
        }
        this
    }

    fn observe_first_frame_cache(&mut self, cx: &mut Context<Self>) {
        let (changed, mut on_changed) = mpsc::unbounded::<()>();
        self.first_frame_observer = Some(RiseAppEvents::default_center().observe(
            RISE_LOTTIE_FIRST_FRAME_CACHE_DID_CHANGE,
            move |_| {
                let _ = changed.unbounded_send(());
            },
        ));
        self.first_frame_task = Some(cx.spawn(async move |this, cx| {
            while on_changed.next().await.is_some() {
                let alive = this.update(cx, |this, cx| this.refresh_first_frame(cx)).is_ok();
                if !alive {
                    break;
                }
            }
        }));
    }

    fn refresh_first_frame(&mut self, cx: &mut Context<Self>) {
        if self.content_view.is_some() {
            return;
        }
        let Some(lottie_url) = self.lottie_url.as_deref() else {
            return;
        };
        let Some(image) = RiseLottieFirstFrameCache::memory_image(
            lottie_url,
            lottie_first_frame_point_size(LOTTIE_FIRST_FRAME_SIDE),
            SaiAccountCacheScope::active_user_id().as_deref(),
        ) else {
            return;
        };
        self.content_view = Some(image);
        self.first_frame_observer = None;
        self.first_frame_task = None;
        cx.notify();
    }

    fn memory_thumbnail(item: &MessageMediaItem) -> Option<CleverImage> {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        if item.sticker_kind == Some(MessageStickerKind::Lottie)
            && let Some(image) = RiseLottieFirstFrameCache::memory_image(
                &item.url,
                lottie_first_frame_point_size(LOTTIE_FIRST_FRAME_SIDE),
                account_user_id.as_deref(),
            )
        {
            return Some(image);
        }
        let mut candidates = vec![item.thumbnail_url.clone()];
        if matches!(item.sticker_kind, Some(MessageStickerKind::Static) | None) {
            candidates.push(Some(item.url.clone()));
        }
        candidates.into_iter().find_map(|candidate| {
            let value = candidate.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or_default();
            if value.is_empty() {
                return None;
            }
            MediaViewerImageMemoryCache::memory_only(value, account_user_id.as_deref())
        })
    }

    /// leading edge of a received sticker inside the row
    fn leading_inset(&self) -> f32 {
        let avatar = if self.reserves_avatar_space { MessageSizes::avatar_column_width() } else { 0.0 };
        MessageSizes::row_side_inset() + MessageSizes::bubble_tail_inset() + avatar
    }

    /// trailing gap of an own sticker, same as the own sticker row keeps in message content
    fn own_trailing_inset() -> f32 {
        MessageSizes::own_trailing_inset() + MessageSizes::bubble_tail_inset() + MessageStickerLayout::META_OVERHANG
    }
}

impl Render for RiseMessageStickerFastScrollPlaceholderView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let display_size = self.display_size;
        let frame = div()
            .absolute()
            .top(px(self.top_padding))
            .w(display_size.width)
            .h(display_size.height)
            .children(self.content_view.clone().map(|image| {
                img(ImageSource::Render(image.clone()))
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .alpha_shadow_source(image, ObjectFit::Contain)
                    .alpha_shadow("sticker-shadow", sticker_shadow())
            }));
        let frame = if self.is_own {
            frame.right(px(Self::own_trailing_inset()))
        } else {
            frame.left(px(self.leading_inset()))
        };
        div().size_full().relative().child(frame)
    }
}

#[derive(Default)]
struct MessageStickerViewState {
    cached_animated_thumbnail: Option<String>,
    loaded_thumbnail_identity: Option<String>,
    loaded_lottie_id: Option<String>,
    effect_anchor: StickerPremiumEffectAnchor,
}

#[derive(IntoElement)]
pub struct MessageStickerView {
    id: ElementId,
    pub item: MessageMediaItem,
}

impl MessageStickerView {
    const LOADING_PLACEHOLDER_CORNER_FRACTION: f32 = 0.22;

    pub fn new(id: impl Into<ElementId>, item: MessageMediaItem) -> Self {
        Self { id: id.into(), item }
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        (self.id.clone(), suffix).into()
    }

    fn display_size(&self) -> Size<Pixels> {
        MessageStickerLayout::display_size(&self.item)
    }

    fn bounding_side(&self) -> f32 {
        MessageStickerLayout::bounding_side(&self.item)
    }

    fn open_pack_sheet(pack_id: Option<&str>, cx: &mut App) {
        let pack_id = trimming_whitespaces_and_newlines(pack_id.unwrap_or_default());
        if pack_id.is_empty() {
            return;
        }
        MessageStickerViewPackSheetHooks::resolved_present(pack_id, PackKind::Sticker, cx);
    }

    fn handle_tap(
        sticker_url: &str,
        playback_id: String,
        pack_id: Option<&str>,
        anchor: &StickerPremiumEffectAnchor,
        cx: &mut App,
    ) {
        if let Some(effect) = StickerPremiumEffectRegistry::shared().effect_url(sticker_url) {
            StickerPremiumEffectPlayer::play(&effect, anchor, move |_: &mut App| Self::restart_sticker(playback_id), cx);
            return;
        }
        Self::open_pack_sheet(pack_id, cx);
    }

    fn play_armed_send_effect(sticker_url: &str, playback_id: String, anchor: StickerPremiumEffectAnchor, cx: &mut App) {
        let Some(effect) = StickerPremiumEffectRegistry::shared().consume_armed_send(sticker_url) else {
            return;
        };
        cx.defer(move |cx| {
            StickerPremiumEffectPlayer::play(&effect, &anchor, move |_: &mut App| Self::restart_sticker(playback_id), cx);
        });
    }

    /// effect art is drawn frame for frame against the sticker so both start together
    fn restart_sticker(playback_id: String) {
        RiseAppEvents::default_center().post_with(STICKER_PREMIUM_EFFECT_DID_START, playback_id);
    }

    fn can_open_pack(&self) -> bool {
        !trimming_whitespaces_and_newlines(self.item.pack_id.as_deref().unwrap_or_default()).is_empty()
    }

    fn lottie_body(&self, state: &Entity<MessageStickerViewState>, cx: &App) -> AnyElement {
        let url = MessageStickerViewHooks::resolved_resolve(&self.item.url);
        let identity = format!("{}|{}", self.item.id, url);
        if !(url.to_lowercase().starts_with("http") || MessageMediaItem::is_local_preview_url(&url)) {
            return self.emoji_fallback();
        }
        let display_size = self.display_size();
        let shows_fallback = state.read(cx).loaded_lottie_id.as_deref() != Some(identity.as_str());
        let first_frame_state = state.clone();
        let lottie = LottieStickerView::new(self.child_id("lottie"), url)
            .size(self.bounding_side())
            .playback_id(Some(self.item.id.clone()))
            .casts_shadow(true)
            .on_first_frame(move |cx: &mut App| {
                let identity = identity.clone();
                first_frame_state.update(cx, |state, cx| {
                    state.loaded_lottie_id = Some(identity);
                    cx.notify();
                });
            });
        zstack(Alignment::Center)
            .children(shows_fallback.then(|| self.lottie_fallback()))
            .child(lottie)
            .w(display_size.width)
            .h(display_size.height)
            .overflow_hidden()
            .into_any_element()
    }

    fn lottie_fallback(&self) -> AnyElement {
        if let Some(thumbnail) = Self::usable_image_source(self.item.thumbnail_url.as_deref()) {
            return CleverImageUi::new(thumbnail)
                .id(self.child_id("lottie-thumbnail"))
                .content_mode(ContentMode::Fit)
                .corner_radius(0.0)
                .persist_to_disk(true)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension((self.bounding_side() * DECODE_SCALE) as i64)
                .into_any_element();
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let side = self.bounding_side();
        let first_frame = RiseLottieFirstFrameCache::memory_image(
            &self.item.url,
            lottie_first_frame_point_size(side),
            account_user_id.as_deref(),
        )
        .or_else(|| RiseLottieFirstFrameCache::latest_memory_image(&self.item.url, account_user_id.as_deref()));
        match first_frame {
            Some(image) => img(ImageSource::Render(image.clone()))
                .size_full()
                .object_fit(ObjectFit::Contain)
                .alpha_shadow_source(image, ObjectFit::Contain)
                .into_any_element(),
            None => self.loading_placeholder(),
        }
    }

    /// holds the sticker place until the first frame on slow network so the feed has no hole
    fn loading_placeholder(&self) -> AnyElement {
        let corner_radius = self.bounding_side() * Self::LOADING_PLACEHOLDER_CORNER_FRACTION;
        div()
            .size_full()
            .child(
                SkeletonBlock {
                    corner_radius,
                    ..SkeletonBlock::default()
                }
                    .skeleton_shimmer(None),
            )
            .alpha_shadow_opaque_source(corner_radius)
            .into_any_element()
    }

    fn animated_video_body(&self, state: &MessageStickerViewState) -> AnyElement {
        if let Some(source) = self.animated_image_source() {
            return AnimatedGifView::new(self.child_id("animated-image"), source)
                .content_mode(AnimatedGifContentMode::ScaleAspectFit)
                .into_any_element();
        }
        if let Some(video) = self.animated_video_source() {
            let is_web_m = video == trimming_whitespaces_and_newlines(&self.item.url)
                && ChatAnimatedMediaSourcePolicy::playback_kind(
                    &video,
                    self.item.mime_type.as_deref(),
                    self.item.codec.as_deref(),
                ) == ChatAnimatedMediaPlaybackKind::WebMVideo;
            return VideoStickerPlaybackView {
                id: self.child_id("video-sticker"),
                source: video,
                poster_source: self.animated_thumbnail_source(state),
                size: self.display_size(),
                is_web_m,
            }
            .into_any_element();
        }
        if let Some(thumbnail) = self.animated_thumbnail_source(state) {
            return CleverImageUi::new(thumbnail)
                .id(self.child_id("animated-thumbnail"))
                .content_mode(ContentMode::Fit)
                .corner_radius(0.0)
                .persist_to_disk(true)
                .maximum_decode_pixel_dimension((self.bounding_side() * DECODE_SCALE) as i64)
                .into_any_element();
        }
        self.loading_placeholder()
    }

    fn raster_or_emoji_body(&self) -> AnyElement {
        let url = trimming_whitespaces_and_newlines(&self.item.url);
        if url.to_lowercase().starts_with("http") || MessageMediaItem::is_local_preview_url(url) {
            return CleverImageUi::new(url.to_owned())
                .id(self.child_id("raster"))
                .content_mode(ContentMode::Fit)
                .corner_radius(0.0)
                .persist_to_disk(true)
                .maximum_decode_pixel_dimension((self.bounding_side() * DECODE_SCALE) as i64)
                .into_any_element();
        }
        self.emoji_fallback()
    }

    fn emoji_fallback(&self) -> AnyElement {
        let display_size = self.display_size();
        let longest = f32::from(display_size.width).max(f32::from(display_size.height));
        let font_size = EMOJI_FALLBACK_MAXIMUM_SIZE.min(longest * EMOJI_FALLBACK_SIZE_FRACTION);
        let emoji = self.item.sticker_emojis.first().cloned().unwrap_or_else(|| EMOJI_FALLBACK.to_owned());
        div().text_size(px(font_size)).child(SharedString::from(emoji)).into_any_element()
    }

    fn animated_thumbnail_source(&self, state: &MessageStickerViewState) -> Option<String> {
        let fallback = if state.loaded_thumbnail_identity.as_deref() == Some(self.animated_thumbnail_lookup_identity().as_str())
        {
            state.cached_animated_thumbnail.as_deref()
        } else {
            None
        };
        [self.item.thumbnail_url.as_deref(), fallback]
            .into_iter()
            .find_map(Self::usable_image_source)
    }

    fn animated_thumbnail_lookup_identity(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.item.id,
            self.item.pack_id.as_deref().unwrap_or_default(),
            self.item.thumbnail_url.as_deref().unwrap_or_default(),
            self.item.sticker_kind.as_ref().map_or("none", |kind| kind.raw_value()),
        )
    }

    fn thumbnail_from_cached_pack(pack_id: Option<&str>, item_id: &str, cx: &App) -> Option<String> {
        let pack_id = trimming_whitespaces_and_newlines(pack_id.unwrap_or_default());
        if pack_id.is_empty() {
            return None;
        }
        let raw_value = MessageStickerViewHooks::resolved_pack_stickers(pack_id, cx)
            .into_iter()
            .find(|sticker| sticker.id == item_id)
            .and_then(|sticker| sticker.thumbnail_url);
        Self::usable_image_source(raw_value.as_deref())
    }

    fn animated_image_source(&self) -> Option<String> {
        let source = trimming_whitespaces_and_newlines(&self.item.url);
        let is_animated_image = source.to_lowercase().starts_with("http")
            && ChatAnimatedMediaSourcePolicy::playback_kind(
                source,
                self.item.mime_type.as_deref(),
                self.item.codec.as_deref(),
            ) == ChatAnimatedMediaPlaybackKind::AnimatedImage;
        is_animated_image.then(|| source.to_owned())
    }

    fn animated_video_source(&self) -> Option<String> {
        let sources: [(Option<&str>, Option<&str>, Option<&str>); 3] = [
            (self.item.local_preview_url.as_deref(), None, self.item.codec.as_deref()),
            (Some(self.item.url.as_str()), self.item.mime_type.as_deref(), self.item.codec.as_deref()),
            (self.item.thumbnail_url.as_deref(), None, None),
        ];
        for (raw_value, mime_type, codec) in sources {
            let Some(value) = raw_value.map(trimming_whitespaces_and_newlines) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            let kind = ChatAnimatedMediaSourcePolicy::playback_kind(value, mime_type, codec);
            if !matches!(
                kind,
                ChatAnimatedMediaPlaybackKind::AvFoundationVideo | ChatAnimatedMediaPlaybackKind::WebMVideo
            ) {
                continue;
            }
            if value.to_lowercase().starts_with("http") {
                return Some(value.to_owned());
            }
            if MessageMediaItem::is_local_preview_url(value) && MessageMediaItem::is_available_local_preview_url(value) {
                return Some(value.to_owned());
            }
        }
        None
    }

    fn is_likely_video_source(source: &str) -> bool {
        match ChatAnimatedMediaSourcePolicy::playback_kind(source, None, None) {
            ChatAnimatedMediaPlaybackKind::AvFoundationVideo
            | ChatAnimatedMediaPlaybackKind::WebMVideo
            | ChatAnimatedMediaPlaybackKind::UnsupportedVideo => true,
            ChatAnimatedMediaPlaybackKind::AnimatedImage | ChatAnimatedMediaPlaybackKind::Unknown => false,
        }
    }

    fn usable_image_source(raw_value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw_value?);
        let usable = !value.is_empty()
            && !Self::is_likely_video_source(value)
            && (value.to_lowercase().starts_with("http") || MessageMediaItem::is_local_preview_url(value));
        usable.then(|| value.to_owned())
    }
}

impl RenderOnce for MessageStickerView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| MessageStickerViewState::default());
        let is_layout_measurement = is_message_layout_measurement(cx);

        let identity = self.animated_thumbnail_lookup_identity();
        let wants_lookup = !is_layout_measurement
            && self.item.sticker_kind == Some(MessageStickerKind::Animated)
            && Self::usable_image_source(self.item.thumbnail_url.as_deref()).is_none();
        let task_state = state.clone();
        let task_identity = identity.clone();
        let pack_id = self.item.pack_id.clone();
        let item_id = self.item.id.clone();
        use_task(self.child_id("thumbnail-lookup"), identity, window, cx, async move |cx| {
            let _ = task_state.update(cx, |state, cx| {
                state.cached_animated_thumbnail = None;
                state.loaded_thumbnail_identity = None;
                cx.notify();
            });
            if !wants_lookup {
                return;
            }
            let Ok(cached) =
                cx.update(|_, cx| Self::thumbnail_from_cached_pack(pack_id.as_deref(), &item_id, cx))
            else {
                return;
            };
            let _ = task_state.update(cx, |state, cx| {
                state.cached_animated_thumbnail = cached;
                state.loaded_thumbnail_identity = Some(task_identity);
                cx.notify();
            });
        });

        let body = match self.item.sticker_kind {
            Some(MessageStickerKind::Lottie) => self.lottie_body(&state, cx),
            Some(MessageStickerKind::Animated) => self.animated_video_body(state.read(cx)),
            Some(MessageStickerKind::Static) | None => self.raster_or_emoji_body(),
        };
        let display_size = self.display_size();
        let can_open_pack = self.can_open_pack();
        let tap_pack_id = self.item.pack_id.clone();
        let effect_anchor = state.read(cx).effect_anchor.clone();
        let (tap_anchor, tap_url, tap_playback_id) = (effect_anchor.clone(), self.item.url.clone(), self.item.id.clone());
        let content = div()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .w(display_size.width)
            .h(display_size.height)
            .children((!is_layout_measurement).then(|| StickerPremiumEffectAnchorView::new(effect_anchor.clone())))
            .child(body.alpha_shadow(self.child_id("shadow"), sticker_shadow()))
            .bubble_tap_target(self.child_id("tap"), move |_window: &mut Window, cx: &mut App| {
                Self::handle_tap(&tap_url, tap_playback_id.clone(), tap_pack_id.as_deref(), &tap_anchor, cx);
            });
        let action_pack_id = self.item.pack_id.clone();
        let (appear_url, appear_playback_id) = (self.item.url.clone(), self.item.id.clone());
        MessageStickerAccessibilityModifier {
            can_open_pack,
            action: Rc::new(move |cx: &mut App| Self::open_pack_sheet(action_pack_id.as_deref(), cx)),
        }
        .body(self.child_id("accessibility"), content.into_any_element())
        .on_appear(self.child_id("armed-send-effect"), move |_window, cx| {
            if !is_layout_measurement {
                Self::play_armed_send_effect(&appear_url, appear_playback_id, effect_anchor, cx);
            }
        })
    }
}

struct MessageStickerAccessibilityModifier {
    can_open_pack: bool,
    action: Rc<dyn Fn(&mut App)>,
}

impl MessageStickerAccessibilityModifier {
    fn body(self, id: ElementId, content: AnyElement) -> impl IntoElement {
        let element = div().id(id).aria_label(tr("msg_preview_sticker")).child(content);
        if !self.can_open_pack {
            return element.role(Role::Image);
        }
        let action = self.action;
        element
            .role(Role::Button)
            .on_a11y_action(AccessibleAction::Click, move |_, _window, cx| action(cx))
    }
}

#[derive(IntoElement)]
struct VideoStickerPlaybackView {
    id: ElementId,
    source: String,
    poster_source: Option<String>,
    size: Size<Pixels>,
    is_web_m: bool,
}

impl VideoStickerPlaybackView {
    fn child_id(&self, suffix: &'static str) -> ElementId {
        (self.id.clone(), suffix).into()
    }

    fn maximum_decode_pixel_dimension(&self) -> i64 {
        (f32::from(self.size.width).max(f32::from(self.size.height)) * DECODE_SCALE).ceil() as i64
    }

    fn poster(&self, suffix: &'static str) -> Option<AnyElement> {
        self.poster_source.clone().map(|poster_source| {
            CleverImageUi::new(poster_source)
                .id(self.child_id(suffix))
                .content_mode(ContentMode::Fit)
                .corner_radius(0.0)
                .persist_to_disk(true)
                .maximum_decode_pixel_dimension(self.maximum_decode_pixel_dimension())
                .into_any_element()
        })
    }

    fn av_foundation_body(self) -> AnyElement {
        let poster = self.poster("poster");
        zstack(Alignment::Center)
            .child(VideoStickerPosterView::new(self.child_id("first-frame"), self.source.clone(), self.size))
            .children(poster)
            .child(
                ChatLoopingVideoView::new(self.child_id("player"), self.source.clone())
                    .video_gravity(AVLayerVideoGravity::ResizeAspect),
            )
            .w(self.size.width)
            .h(self.size.height)
            .overflow_hidden()
            .into_any_element()
    }
}

impl RenderOnce for VideoStickerPlaybackView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if !(self.is_web_m || WebMStickerSupport::is_web_m(&self.source)) {
            return self.av_foundation_body();
        }
        let mut stack = ChatWebMVideoStack::new(self.child_id("web-m"), self.source.clone());
        if let Some(poster) = self.poster("web-m-poster") {
            stack = stack.poster(poster);
        }
        div()
            .w(self.size.width)
            .h(self.size.height)
            .overflow_hidden()
            .child(stack)
            .into_any_element()
    }
}

/// webm frame can be transparent so the poster leaves once the first frame is up
/// it would show through otherwise
#[derive(IntoElement)]
pub struct ChatWebMVideoStack {
    id: ElementId,
    pub source: String,
    pub video_gravity: AVLayerVideoGravity,
    poster: Option<AnyElement>,
}

impl ChatWebMVideoStack {
    pub fn new(id: impl Into<ElementId>, source: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            video_gravity: AVLayerVideoGravity::ResizeAspect,
            poster: None,
        }
    }

    pub fn video_gravity(mut self, video_gravity: AVLayerVideoGravity) -> Self {
        self.video_gravity = video_gravity;
        self
    }

    /// the poster shown until the first frame
    pub fn poster(mut self, poster: impl IntoElement) -> Self {
        self.poster = Some(poster.into_any_element());
        self
    }
}

impl RenderOnce for ChatWebMVideoStack {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let identity: ElementId = (self.id.clone(), self.source.clone()).into();
        let has_first_frame = window.use_keyed_state((identity.clone(), "has-first-frame"), cx, |_, _| false);
        let shows_poster = !*has_first_frame.read(cx);
        let first_frame = has_first_frame.clone();
        let paint_view = window.current_view();
        let player = ChatLoopingVideoView::new((identity, "player"), self.source)
            .video_gravity(self.video_gravity)
            .forces_web_m(true)
            .on_web_m_first_frame(move |cx: &mut App| {
                first_frame.update(cx, |has_first_frame, cx| {
                    *has_first_frame = true;
                    cx.notify();
                });
                cx.notify(paint_view);
            });
        zstack(Alignment::Center)
            .size_full()
            .children(self.poster.filter(|_| shows_poster))
            .child(player)
    }
}

/// called when the first webm frame is on screen
pub type ChatLoopingVideoFirstFrame = Rc<dyn Fn(&mut App)>;

#[derive(IntoElement)]
pub struct ChatLoopingVideoView {
    id: ElementId,
    pub source: String,
    pub video_gravity: AVLayerVideoGravity,
    pub forces_web_m: bool,
    pub on_web_m_first_frame: Option<ChatLoopingVideoFirstFrame>,
}

impl ChatLoopingVideoView {
    pub fn new(id: impl Into<ElementId>, source: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            video_gravity: AVLayerVideoGravity::ResizeAspect,
            forces_web_m: false,
            on_web_m_first_frame: None,
        }
    }

    pub fn video_gravity(mut self, video_gravity: AVLayerVideoGravity) -> Self {
        self.video_gravity = video_gravity;
        self
    }

    pub fn forces_web_m(mut self, forces_web_m: bool) -> Self {
        self.forces_web_m = forces_web_m;
        self
    }

    pub fn on_web_m_first_frame(mut self, on_web_m_first_frame: impl Fn(&mut App) + 'static) -> Self {
        self.on_web_m_first_frame = Some(Rc::new(on_web_m_first_frame));
        self
    }

    fn make_coordinator(&self, window: &mut Window, cx: &mut App) -> Entity<ChatLoopingVideoViewCoordinator> {
        window.use_keyed_state((self.id.clone(), "coordinator"), cx, |_, _| ChatLoopingVideoViewCoordinator::new())
    }

    fn make_ui_view(&self, window: &mut Window, cx: &mut App) -> Entity<VideoStickerPlayerHostView> {
        window.use_keyed_state((self.id.clone(), "host"), cx, |window, cx| {
            let mut host = VideoStickerPlayerHostView::new(cx);
            host.did_move_to_window(window, cx);
            host
        })
    }

    fn update_ui_view(
        &self,
        host: &Entity<VideoStickerPlayerHostView>,
        coordinator: &Entity<ChatLoopingVideoViewCoordinator>,
        is_playback_scope_active: bool,
        cx: &mut App,
    ) {
        let video_gravity = self.video_gravity;
        host.update(cx, |host, cx| host.set_video_gravity(video_gravity, cx));
        let on_web_m_first_frame = self.on_web_m_first_frame.clone();
        coordinator.update(cx, |coordinator, cx| {
            coordinator.on_web_m_first_frame = on_web_m_first_frame;
            coordinator.update(&self.source, self.forces_web_m, is_playback_scope_active, cx);
        });
    }

    fn dismantle_ui_view(coordinator: &Entity<ChatLoopingVideoViewCoordinator>, cx: &mut App) {
        coordinator.update(cx, |coordinator, cx| coordinator.teardown(cx));
    }
}

impl RenderOnce for ChatLoopingVideoView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let coordinator = self.make_coordinator(window, cx);
        let host = self.make_ui_view(window, cx);
        if coordinator.read(cx).host.is_none() {
            coordinator.update(cx, |coordinator, cx| coordinator.bind(&host, cx));
        }
        let is_playback_scope_active = is_animated_media_playback_active(cx);
        self.update_ui_view(&host, &coordinator, is_playback_scope_active, cx);

        let viewport_scope = nearest_animated_media_viewport_scope_view(cx);
        let probe_host = host.clone();
        let replay_host = host.clone();
        let visibility = canvas(
            move |bounds, window, _cx| {
                let is_in_viewport_scope = VideoStickerVisibilityProbe::is_in_viewport_scope(viewport_scope.as_deref());
                let mask = window.content_mask().bounds;
                let moved_host = replay_host.clone();
                window.on_replayed_move(move |offset, mask_offset, window, cx| {
                    let moved = VideoStickerVisibilityProbe::measure_at(
                        bounds + offset,
                        mask + mask_offset,
                        window.viewport_size(),
                        is_in_viewport_scope && is_playback_scope_active,
                    );
                    let host = moved_host.clone();
                    window.defer(cx, move |_window, cx| host.update(cx, |host, cx| host.layout_subviews(moved, cx)));
                });
                VideoStickerVisibilityProbe::measure_at(
                    bounds,
                    mask,
                    window.viewport_size(),
                    is_in_viewport_scope && is_playback_scope_active,
                )
            },
            move |_bounds, measured, window, cx| {
                window.defer(cx, move |_window, cx| {
                    probe_host.update(cx, |host, cx| host.layout_subviews(measured, cx));
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let (player, video_gravity, frame_layer) = {
            let host = host.read(cx);
            (host.player.clone(), host.video_gravity, host.frame_layer.clone())
        };
        let frame_layer = div().absolute().top_0().left_0().size_full().child(frame_layer);

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(AVPlayerLayer::new(player).video_gravity(video_gravity))
            .child(frame_layer)
            .child(visibility)
            .on_disappear((self.id, "chat-looping-video-view"), move |_window, cx| {
                Self::dismantle_ui_view(&coordinator, cx);
            })
    }
}

/// owns the player, the webm playback and the playback slot
pub struct ChatLoopingVideoViewCoordinator {
    host: Option<WeakEntity<VideoStickerPlayerHostView>>,
    player: Option<Entity<AVQueuePlayer>>,
    looper: Option<AVPlayerLooper>,
    streaming_asset: Option<RiseMediaStreamingAsset>,
    media_lease: Option<RemoteMediaFileLease>,
    web_m_playback: Option<WebMStickerPlayback>,
    pub on_web_m_first_frame: Option<ChatLoopingVideoFirstFrame>,
    load_task: Option<Task<()>>,
    load_generation: u64,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    loaded_source: Option<String>,
    is_visible: bool,
    is_playback_active: bool,
    owns_playback_slot: bool,
    is_waiting_for_playback_slot: bool,
    playback_token: AnimatedMediaParticipantId,
}

impl ChatLoopingVideoViewCoordinator {
    fn new() -> Self {
        Self {
            host: None,
            player: None,
            looper: None,
            streaming_asset: None,
            media_lease: None,
            web_m_playback: None,
            on_web_m_first_frame: None,
            load_task: None,
            load_generation: 0,
            account_user_id: None,
            account_epoch: None,
            loaded_source: None,
            is_visible: false,
            is_playback_active: true,
            owns_playback_slot: false,
            is_waiting_for_playback_slot: false,
            playback_token: AnimatedMediaParticipantId::next(),
        }
    }

    fn bind(&mut self, host: &Entity<VideoStickerPlayerHostView>, cx: &mut Context<Self>) {
        self.host = Some(host.downgrade());
        let coordinator = cx.weak_entity();
        let bound = host.downgrade();
        let visibility_changed: VideoStickerVisibilityChanged = Rc::new(move |visible: bool, cx: &mut App| {
            let coordinator = coordinator.clone();
            let bound = bound.clone();
            cx.defer(move |cx| {
                coordinator
                    .update(cx, |this, cx| {
                        if this.host.as_ref().is_some_and(|host| *host == bound) {
                            this.set_visible(visible, cx);
                        }
                    })
                    .ok();
            });
        });
        let visible = host.update(cx, |host, _| {
            host.visibility_changed = Some(visibility_changed);
            host.is_effectively_visible()
        });
        self.set_visible(visible, cx);
    }

    fn update(&mut self, source: &str, forces_web_m: bool, is_playback_active: bool, cx: &mut Context<Self>) {
        self.is_playback_active = is_playback_active;
        let normalized = trimming_whitespaces_and_newlines(source).to_owned();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if self.loaded_source.as_deref() == Some(normalized.as_str())
            && self.account_user_id == account_user_id
            && self.account_epoch == account_epoch
        {
            self.update_playback(cx);
            return;
        }
        self.reset_player(cx);
        self.loaded_source = Some(normalized.clone());
        self.account_user_id = account_user_id.clone();
        self.account_epoch = account_epoch.clone();
        let Some(url) = Self::source_url(&normalized) else {
            return;
        };

        if forces_web_m || WebMStickerSupport::is_web_m(&normalized) {
            self.load_web_m(url, account_user_id, account_epoch, cx);
            return;
        }
        if url.is_file_url() {
            self.install_player_item(AVPlayerItem::new(url), cx);
            return;
        }
        let generation = self.load_generation;
        let cache_key = RiseMediaCacheIdentity::key(&url);
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let lookup_url = url.clone();
            let lookup_key = cache_key.clone();
            let lookup_user_id = account_user_id.clone();
            let lookup_epoch = account_epoch.clone();
            let cached = cx
                .background_spawn(async move {
                    RemoteMediaDiskCache::shared().existing_local_url(
                        &lookup_url,
                        Some(lookup_key.as_str()),
                        lookup_user_id.as_deref(),
                        lookup_epoch.as_deref(),
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                if this.load_generation != generation
                    || !SaiAccountCacheScope::is_current_account_epoch(
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                    )
                {
                    return;
                }
                this.load_task = None;
                if let Some(cached) = cached {
                    this.media_lease = RemoteMediaDiskCache::lease(Some(cached.as_path()));
                    if let Some(file) = file_url(&cached) {
                        this.install_player_item(AVPlayerItem::new(file), cx);
                    }
                    return;
                }
                let Some(streaming) = RiseMediaStreamingAsset::make(
                    &url,
                    &cache_key,
                    RemoteMediaDiskCacheLimits::default().maximum_file_bytes,
                    account_user_id,
                    account_epoch,
                ) else {
                    return;
                };
                let item = AVPlayerItem::with_asset(streaming.asset.clone());
                this.streaming_asset = Some(streaming);
                this.install_player_item(item, cx);
            })
            .ok();
        }));
    }

    fn load_web_m(
        &mut self,
        url: Url,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let generation = self.load_generation;
        let key = url.absolute_string().to_owned();
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let file = if url.is_file_url() {
                url.to_file_path()
            } else {
                let options = RemoteMediaDiskCacheMaterializeOptions {
                    cache_key: Some(RiseMediaCacheIdentity::key(&url)),
                    suggested_filename: Some(url.last_path_component()),
                    expected_account_user_id: account_user_id,
                    expected_account_epoch: account_epoch,
                    ..RemoteMediaDiskCacheMaterializeOptions::default()
                };
                RemoteMediaDiskCache::shared().ensure_cached(&url, options, cx).await
            };
            let Some(file) = file else {
                return;
            };
            let lease = RemoteMediaDiskCache::lease(Some(file.as_path()));
            let track = WebMStickerTrackStore::track(file, key).await;
            this.update(cx, |this, cx| {
                let Some(track) = track else {
                    return;
                };
                if this.load_generation != generation {
                    return;
                }
                let Some(host) = this.host.as_ref().and_then(WeakEntity::upgrade) else {
                    return;
                };
                let layer = host.read(cx).frame_layer();
                this.load_task = None;
                this.media_lease = lease;
                let playback = WebMStickerPlayback::new(track, layer);
                let coordinator = cx.weak_entity();
                playback.set_on_first_frame(Some(Rc::new(move |cx: &mut App| {
                    let on_first_frame = coordinator
                        .read_with(cx, |this, _| this.on_web_m_first_frame.clone())
                        .ok()
                        .flatten();
                    if let Some(on_first_frame) = on_first_frame {
                        on_first_frame(cx);
                    }
                })));
                this.web_m_playback = Some(playback);
                this.update_playback(cx);
            })
            .ok();
        }));
    }

    fn install_player_item(&mut self, mut item: AVPlayerItem, cx: &mut Context<Self>) {
        item.set_preferred_forward_buffer_duration(PREFERRED_FORWARD_BUFFER_DURATION);
        let template = item.clone();
        let player = cx.new(|cx| {
            let mut player = AVQueuePlayer::new(item, cx);
            player.set_is_muted(true);
            player.set_action_at_item_end(AVPlayerActionAtItemEnd::None);
            player.set_automatically_waits_to_minimize_stalling(true);
            player
        });
        self.looper = Some(AVPlayerLooper::new(&player, &template, cx));
        self.player = Some(player.clone());
        self.with_host(cx, |host, cx| host.set_player(Some(player), cx));
        self.update_playback(cx);
    }

    fn teardown(&mut self, cx: &mut Context<Self>) {
        self.with_host(cx, |host, _| {
            host.visibility_changed = None;
            host.stop_visibility_tracking();
        });
        self.is_visible = false;
        self.is_playback_active = false;
        self.loaded_source = None;
        self.reset_player(cx);
        self.host = None;
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.is_visible {
            return;
        }
        self.is_visible = visible;
        self.update_playback(cx);
    }

    fn update_playback(&mut self, cx: &mut Context<Self>) {
        if self.is_visible && self.is_playback_active {
            self.request_playback_slot_if_needed(cx);
        } else {
            self.pause_and_release_playback_slot(cx);
        }
    }

    fn reset_player(&mut self, cx: &mut Context<Self>) {
        self.load_task = None;
        self.load_generation = self.load_generation.wrapping_add(1);
        self.pause_and_release_playback_slot(cx);
        self.with_host(cx, |host, cx| host.set_player(None, cx));
        self.looper = None;
        self.player = None;
        if let Some(mut playback) = self.web_m_playback.take() {
            playback.stop(cx);
        }
        self.with_host(cx, |host, cx| host.clear_frame_layer(cx));
        if let Some(streaming) = self.streaming_asset.take() {
            streaming.teardown();
        }
        self.media_lease = None;
    }

    fn request_playback_slot_if_needed(&mut self, cx: &mut Context<Self>) {
        if (self.player.is_none() && self.web_m_playback.is_none())
            || self.owns_playback_slot
            || self.is_waiting_for_playback_slot
        {
            return;
        }
        let token = self.playback_token;
        self.is_waiting_for_playback_slot = true;
        let granted = cx.weak_entity();
        let revoked = cx.weak_entity();
        AnimatedMediaPlaybackCoordinator::shared(cx).update(cx, |coordinator, cx| {
            coordinator.request(
                token,
                AnimatedMediaPlaybackKind::VideoSticker,
                move |_, cx: &mut App| {
                    let still_here = granted.update(cx, |this, cx| this.playback_slot_granted(cx)).is_ok();
                    if !still_here {
                        release_playback_slot(token, cx);
                    }
                },
                move |_, cx: &mut App| {
                    revoked.update(cx, |this, cx| this.playback_slot_revoked(cx)).ok();
                },
                cx,
            );
        });
    }

    fn playback_slot_granted(&mut self, cx: &mut Context<Self>) {
        self.is_waiting_for_playback_slot = false;
        if !self.is_visible
            || !self.is_playback_active
            || (self.player.is_none() && self.web_m_playback.is_none())
        {
            release_playback_slot(self.playback_token, cx);
            return;
        }
        self.owns_playback_slot = true;
        if let Some(player) = &self.player {
            player.update(cx, |player, cx| player.play(cx));
        }
        if let Some(playback) = &mut self.web_m_playback {
            playback.play(cx);
        }
    }

    fn playback_slot_revoked(&mut self, cx: &mut Context<Self>) {
        self.owns_playback_slot = false;
        self.is_waiting_for_playback_slot = true;
        self.pause_players(cx);
    }

    fn pause_and_release_playback_slot(&mut self, cx: &mut Context<Self>) {
        self.pause_players(cx);
        if !self.owns_playback_slot && !self.is_waiting_for_playback_slot {
            return;
        }
        self.owns_playback_slot = false;
        self.is_waiting_for_playback_slot = false;
        release_playback_slot(self.playback_token, cx);
    }

    fn pause_players(&mut self, cx: &mut Context<Self>) {
        if let Some(player) = &self.player {
            player.update(cx, |player, cx| player.pause(cx));
        }
        if let Some(playback) = &mut self.web_m_playback {
            playback.pause();
        }
    }

    fn with_host(
        &self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut VideoStickerPlayerHostView, &mut Context<VideoStickerPlayerHostView>),
    ) {
        if let Some(host) = self.host.as_ref().and_then(WeakEntity::upgrade) {
            host.update(cx, update);
        }
    }

    fn source_url(source: &str) -> Option<Url> {
        if source.to_lowercase().starts_with("file://") {
            return Url::from_string(source);
        }
        if is_local_absolute_path(source) {
            return file_url(Path::new(source));
        }
        let url = Url::from_string(source)?;
        let scheme = url.scheme().map(str::to_lowercase);
        matches!(scheme.as_deref(), Some("https" | "http")).then_some(url)
    }
}

fn release_playback_slot(token: AnimatedMediaParticipantId, cx: &mut App) {
    AnimatedMediaPlaybackCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.release(token, None, cx));
}

/// file url for the players, they take file:// urls
fn file_url(path: &Path) -> Option<Url> {
    Url::from_file_path(path)
}

/// called when the visibility of the host changed
pub type VideoStickerVisibilityChanged = Rc<dyn Fn(bool, &mut App)>;

/// view hosting the clip with player layer, frame layer and visibility tracking
/// every paint reports the clipped frame
pub struct VideoStickerPlayerHostView {
    pub visibility_changed: Option<VideoStickerVisibilityChanged>,
    last_reported_visibility: Option<bool>,
    scene_observers: Vec<Subscription>,
    player: Option<Entity<AVQueuePlayer>>,
    video_gravity: AVLayerVideoGravity,
    frame_layer: Entity<WebMStickerLayer>,
    last_probe: Option<VideoStickerVisibilityProbe>,
}

impl VideoStickerPlayerHostView {
    fn new(cx: &mut App) -> Self {
        Self {
            visibility_changed: None,
            last_reported_visibility: None,
            scene_observers: Vec::new(),
            player: None,
            video_gravity: AVLayerVideoGravity::ResizeAspect,
            frame_layer: cx.new(|cx| {
                let mut layer = WebMStickerLayer::new();
                layer.set_contents_gravity(AVLayerVideoGravity::ResizeAspect, cx);
                layer
            }),
            last_probe: None,
        }
    }

    pub fn player(&self) -> Option<&Entity<AVQueuePlayer>> {
        self.player.as_ref()
    }

    pub fn set_player(&mut self, player: Option<Entity<AVQueuePlayer>>, cx: &mut Context<Self>) {
        self.player = player;
        cx.notify();
    }

    pub fn video_gravity(&self) -> AVLayerVideoGravity {
        self.video_gravity
    }

    pub fn set_video_gravity(&mut self, video_gravity: AVLayerVideoGravity, cx: &mut Context<Self>) {
        if self.video_gravity == video_gravity {
            return;
        }
        self.video_gravity = video_gravity;
        self.frame_layer.update(cx, |layer, cx| layer.set_contents_gravity(video_gravity, cx));
        cx.notify();
    }

    /// frames decoded outside the system player land here, above the empty player layer
    pub fn frame_layer(&self) -> WeakEntity<WebMStickerLayer> {
        self.frame_layer.downgrade()
    }

    pub fn clear_frame_layer(&mut self, cx: &mut Context<Self>) {
        self.frame_layer.update(cx, |layer, cx| layer.clear(cx));
    }

    /// true when drawn in a window, inside an active playback scope and viewport scope
    /// with enough of its height inside every clipping ancestor, as of the last paint
    pub fn is_effectively_visible(&self) -> bool {
        self.last_probe.is_some_and(|probe| probe.is_visible)
    }

    fn did_move_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_scene_subscription(window, cx);
        self.report_visibility_if_changed(cx);
    }

    fn layout_subviews(&mut self, probe: VideoStickerVisibilityProbe, cx: &mut Context<Self>) {
        let first_report = self.last_probe.is_none();
        self.last_probe = Some(probe);
        if first_report {
            self.did_move_to_superview(cx);
        } else {
            self.report_visibility_if_changed(cx);
        }
    }

    fn did_move_to_superview(&mut self, cx: &mut Context<Self>) {
        self.report_visibility_if_changed(cx);
    }

    pub fn stop_visibility_tracking(&mut self) {
        self.remove_scene_observers();
        self.last_reported_visibility = None;
    }

    fn refresh_scene_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remove_scene_observers();
        self.scene_observers.push(cx.observe_window_activation(window, |this, _window, cx| {
            this.report_visibility_if_changed(cx);
            cx.notify();
        }));
    }

    fn remove_scene_observers(&mut self) {
        self.scene_observers.clear();
    }

    fn report_visibility_if_changed(&mut self, cx: &mut Context<Self>) {
        let visible = self.is_effectively_visible();
        if Some(visible) == self.last_reported_visibility {
            return;
        }
        self.last_reported_visibility = Some(visible);
        if let Some(visibility_changed) = self.visibility_changed.clone() {
            visibility_changed(visible, cx);
        }
    }
}

/// what one paint saw of the host frame
#[derive(Clone, Copy, Debug, PartialEq)]
struct VideoStickerVisibilityProbe {
    is_visible: bool,
}

impl VideoStickerVisibilityProbe {
    /// visibility of the frame clipped by every clipping ancestor and the window
    /// inside the active playback scope and the viewport scope when there is one
    fn measure_at(bounds: Bounds<Pixels>, mask: Bounds<Pixels>, viewport: Size<Pixels>, is_in_scopes: bool) -> Self {
        let window_bounds = Bounds::new(point(px(0.), px(0.)), viewport);
        let clip = mask.intersect(&window_bounds);
        let drawable = bounds.size.width > px(0.) && bounds.size.height > px(0.);
        let is_visible = is_in_scopes
            && drawable
            && bounds.intersects(&clip)
            && {
                let visible = bounds.intersect(&clip);
                let fraction = f32::from(visible.size.height) / f32::from(bounds.size.height);
                f32::from(visible.size.width) > 1.0
                    && fraction > AnimatedMediaViewportPlaybackPolicy::MINIMUM_VISIBLE_HEIGHT_FRACTION
            };
        Self { is_visible }
    }

    fn is_in_viewport_scope(viewport_scope: Option<&dyn AnimatedMediaViewportScopeProviding>) -> bool {
        viewport_scope.is_none_or(AnimatedMediaViewportScopeProviding::is_animated_media_visible_in_viewport)
    }
}

#[derive(Default)]
struct VideoStickerPosterViewState {
    poster: Option<CleverImage>,
    loaded_poster_identity: Option<String>,
}

#[derive(IntoElement)]
pub struct VideoStickerPosterView {
    id: ElementId,
    pub source: String,
    pub size: Size<Pixels>,
}

impl VideoStickerPosterView {
    pub fn new(id: impl Into<ElementId>, source: impl Into<String>, size: Size<Pixels>) -> Self {
        Self { id: id.into(), source: source.into(), size }
    }

    fn maximum_pixel_dimension(&self, display_scale: f32) -> i64 {
        let longest = f32::from(self.size.width).max(f32::from(self.size.height));
        (longest * display_scale.clamp(1.0, 3.0)).ceil() as i64
    }
}

impl RenderOnce for VideoStickerPosterView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state((self.id.clone(), "state"), cx, |_, _| {
            VideoStickerPosterViewState::default()
        });
        let maximum_pixel_dimension = self.maximum_pixel_dimension(window.scale_factor());
        let task_identity = format!("{}|{}", self.source, maximum_pixel_dimension);
        let is_layout_measurement = is_message_layout_measurement(cx);

        let task_state = state.clone();
        let source = self.source.clone();
        let identity = task_identity.clone();
        use_task((self.id.clone(), "poster-task"), task_identity.clone(), window, cx, async move |cx| {
            let _ = task_state.update(cx, |state, cx| {
                state.poster = None;
                state.loaded_poster_identity = None;
                cx.notify();
            });
            if is_layout_measurement {
                return;
            }
            let result = VideoStickerPosterLoader::image(&source, maximum_pixel_dimension, cx).await;
            let _ = task_state.update(cx, |state, cx| {
                state.poster = result;
                state.loaded_poster_identity = Some(identity);
                cx.notify();
            });
        });

        let poster = {
            let state = state.read(cx);
            state
                .poster
                .clone()
                .filter(|_| state.loaded_poster_identity.as_deref() == Some(task_identity.as_str()))
        };
        div()
            .w(self.size.width)
            .h(self.size.height)
            .children(poster.map(|poster| {
                img(ImageSource::Render(poster.clone()))
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .alpha_shadow_source(poster, ObjectFit::Contain)
            }))
    }
}

static VIDEO_STICKER_POSTER_MEMORY_CACHE: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(40);
    cache.set_total_cost_limit(24 * 1_024 * 1_024);
    cache
});

enum VideoStickerPosterLoader {}

impl VideoStickerPosterLoader {
    const POSTER_TIME_SECONDS: f64 = 0.05;

    async fn image(source: &str, maximum_pixel_dimension: i64, cx: &gpui::AsyncApp) -> Option<CleverImage> {
        let pixel_limit = maximum_pixel_dimension.clamp(64, 720);
        let key = format!("{source}|{pixel_limit}");
        if let Some(cached) = VIDEO_STICKER_POSTER_MEMORY_CACHE.object(&key) {
            return Some(cached);
        }
        let url = Self::source_url(source)?;
        let image = cx
            .background_spawn(async move {
                let file = if url.is_file_url() {
                    url.to_file_path()?
                } else {
                    RemoteMediaDiskCache::shared().existing_local_url(
                        &url,
                        Some(RiseMediaCacheIdentity::key(&url).as_str()),
                        SaiAccountCacheScope::active_user_id().as_deref(),
                        SaiAccountCacheScope::active_epoch().as_deref(),
                    )?
                };
                if !file.exists() {
                    return None;
                }
                let side = u32::try_from(pixel_limit).ok()?;
                let (width, height, bgra) = if WebMStickerFile::is_web_m_path(&file) {
                    let track = WebMStickerFile::track(&file)?;
                    WebMStickerFile::strip(&track, 1, side).into_iter().next()?
                } else {
                    let frame = rise_media::video_export::poster(&file, Self::POSTER_TIME_SECONDS, side).ok()?;
                    (frame.width, frame.height, frame.bgra)
                };
                let buffer = RgbaImage::from_raw(width, height, bgra)?;
                let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
                let cost = usize::try_from(width).ok()? * 4 * usize::try_from(height).ok()?;
                Some((rise_ui::image_ui::render_image_atlas::RenderImageAtlas::tracked(Arc::new(RenderImage::new(frames))), cost))
            })
            .await;
        let (image, cost) = image?;
        VIDEO_STICKER_POSTER_MEMORY_CACHE.set_object_cost(image.clone(), key, cost);
        Some(image)
    }

    fn source_url(source: &str) -> Option<Url> {
        let value = trimming_whitespaces_and_newlines(source);
        if value.to_lowercase().starts_with("file://") {
            return Url::from_string(value);
        }
        if is_local_absolute_path(value) {
            return file_url(Path::new(value));
        }
        let url = Url::from_string(value)?;
        let scheme = url.scheme().map(str::to_lowercase);
        matches!(scheme.as_deref(), Some("https" | "http")).then_some(url)
    }
}

pub enum MessageStickerFavourite {}

impl MessageStickerFavourite {
    pub const FAVOURITE_ACTION_ID: &'static str = "favourite_sticker";

    pub fn menu_actions(message: &GetMessageMessage, chat: &ChatInfo, cx: &App) -> Vec<HoldContextMenuAction> {
        if MessageStickerViewHooks::resolved_protects_content(message, chat) {
            return Vec::new();
        }
        let user_id = MessageStickerViewHooks::resolved_effective_user_id_for_wire(cx).unwrap_or_default();
        if let Some(sticker) = Self::sticker_item(message) {
            let is_fav = !user_id.is_empty() && MessageStickerViewHooks::resolved_is_favourite(&user_id, &sticker.id, cx);
            return vec![
                HoldContextMenuAction::new(
                    Self::FAVOURITE_ACTION_ID,
                    if is_fav { tr("favourite_remove") } else { tr("favourite_add") },
                )
                .system_image(if is_fav { "star.slash" } else { "star" }),
            ];
        }
        let Some(gif) = Self::gif_item(message) else {
            return Vec::new();
        };
        let is_saved = !user_id.is_empty() && MessageStickerViewHooks::resolved_is_gif_favourite(&user_id, &gif, cx);
        vec![
            HoldContextMenuAction::new(
                Self::FAVOURITE_ACTION_ID,
                if is_saved { tr("gif_unsave") } else { tr("gif_save") },
            )
            .system_image(if is_saved { "bookmark.slash" } else { "bookmark" }),
        ]
    }

    pub fn handle(id: &str, message: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) {
        if id != Self::FAVOURITE_ACTION_ID || MessageStickerViewHooks::resolved_protects_content(message, chat) {
            return;
        }
        let user_id = MessageStickerViewHooks::resolved_effective_user_id_for_wire(cx).unwrap_or_default();
        if user_id.is_empty() {
            return;
        }
        if let Some(sticker) = Self::sticker_item(message) {
            MessageStickerViewHooks::resolved_toggle_favourite(&user_id, &sticker, cx);
        } else if let Some(gif) = Self::gif_item(message) {
            MessageStickerViewHooks::resolved_toggle_gif_favourite(&user_id, &gif, cx);
        }
    }

    pub fn sticker_item(message: &GetMessageMessage) -> Option<StickerItem> {
        let media = message.first_sticker_item()?;
        let url = trimming_whitespaces_and_newlines(&media.url).to_owned();
        if url.is_empty() {
            return None;
        }
        let asset = match media.sticker_kind {
            Some(MessageStickerKind::Static) => StickerAssetType::StaticImage,
            Some(MessageStickerKind::Animated) => StickerAssetType::Video,
            Some(MessageStickerKind::Lottie) => StickerAssetType::Lottie,
            None => StickerAssetType::infer(None, &url),
        };
        Some(StickerItem::new(
            media.id.clone(),
            media.pack_id.clone().unwrap_or_default(),
            url,
            media.thumbnail_url.clone(),
            media.width.round() as i64,
            media.height.round() as i64,
            media.duration.map(|duration| (duration * 1000.0) as i64),
            0,
            0,
            media.sticker_emojis.clone(),
            asset,
            media.mime_type.clone(),
            media.codec.clone(),
        ))
    }

    pub fn gif_item(message: &GetMessageMessage) -> Option<GifItem> {
        let media = message
            .media_items
            .iter()
            .find(|media| matches!(media.kind, MessageMediaKind::Animation | MessageMediaKind::Gif))?;
        let url = trimming_whitespaces_and_newlines(&media.url).to_owned();
        if url.is_empty() {
            return None;
        }
        Some(GifItem {
            id: if media.id.is_empty() { message.id.clone() } else { media.id.clone() },
            preview_url: media.thumbnail_url.clone().unwrap_or_else(|| url.clone()),
            file_url: url,
            width: media.width.round() as i64,
            height: media.height.round() as i64,
            duration_ms: media.duration.map_or(0, |duration| (duration * 1000.0) as i64),
            source: "chat".to_owned(),
            source_id: message.sai_server_id(),
            added_at_ms: 0,
        })
    }
}
