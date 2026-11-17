use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{
    App, Bounds, DragMoveEvent, ElementId, MouseButton, MouseDownEvent, MouseMoveEvent, FontWeight, Hsla, ImageSource, IntoElement, ObjectFit, Pixels, Render,
    RenderOnce, SharedString, Subscription, Window, black, img, px, red, size, transparent_black, white,
};
use parking_lot::Mutex;
use rise_core::foundation::foundation_character::scalar_is_control_character;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_regex::escaping_stray_percent_signs;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{
    Url, UrlAllowedCharacters, UrlComponents, adding_percent_encoding,
};
use rise_i18n::{FormatArg, format_printf, tr};
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::animated_tabs_ui::animated_tabs_types::TabHeaderConfig;
use rise_ui::liquid_chrome::LiquidChrome;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize, LoaderUiType};
use rise_ui::skeleton_ui::{SkeletonBlock, SkeletonShimmerExt};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::motion::{linear, spring};
use rise_ui::scroll_edge_blur_ui::{ScrollEdgeBlurFalloff, ScrollEdgeBlurUi};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::transition::UnitPoint;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::byte_format::ByteFormat;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::stores::http::http_service::HttpService;
use crate::core::ui::animated_gif_view::{AnimatedGifContentMode, AnimatedGifView};
use crate::core::ui::animated_media_playback_coordinator::AnimatedMediaPlaybackKind;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::lottie_sticker::lottie_sticker_view::LottieStickerView;
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieFirstFrameCache;
use crate::modules::chat::components::chat::bubble::message_sticker_view::{
    ChatAnimatedMediaPlaybackKind, ChatAnimatedMediaSourcePolicy, ChatLoopingVideoView, ChatWebMVideoStack,
    VideoStickerPosterView,
};
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit_hooks::ChatInputPickerKitHooks;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::{
    StickerHoldPreviewModifiers, StickerPreviewAsset,
};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::media_picker::api::media_picker_result::MediaPickerPhotoAssetSource;
use crate::modules::stickers::engine::rise_sticker_models::{
    RiseStickerMutationState, RiseStickerPerformancePolicy, RiseStickerUploadPresentation,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    StickerAssetType, StickerItem, StickerPack,
};

pub enum ChatInputPickerMetrics {}

impl ChatInputPickerMetrics {
    pub const STICKER_COLUMNS: usize = 5;
    pub const EMOJI_COLUMNS: usize = 8;
    pub const GIF_COLUMNS: usize = 2;

    pub const EMOJI_MIN_CELL_WIDTH: f32 = 52.0;
    pub const STICKER_MIN_CELL_WIDTH: f32 = 80.0;
    pub const GIF_MIN_TILE_WIDTH: f32 = 160.0;

    /// base count on a narrow panel, wider panels add columns instead of growing cells
    pub fn columns(base: usize, width: f32, min_cell: f32) -> usize {
        if !width.is_finite() || width <= 0.0 {
            return base;
        }
        base.max((width / min_cell) as usize)
    }

    pub fn emoji_columns(width: f32) -> usize {
        Self::columns(Self::EMOJI_COLUMNS, width, Self::EMOJI_MIN_CELL_WIDTH)
    }

    pub fn sticker_columns(width: f32) -> usize {
        Self::columns(Self::STICKER_COLUMNS, width, Self::STICKER_MIN_CELL_WIDTH)
    }

    /// gif column count for a width, base defaults to GIF_COLUMNS
    pub fn gif_columns(width: f32, base: usize) -> usize {
        Self::columns(base, width, Self::GIF_MIN_TILE_WIDTH)
    }

    pub const TOP_STRIP_HEIGHT: f32 = 44.0;
    pub const STRIP_EDGE_BLUR_HEIGHT: f32 = 24.0;
    pub const STRIP_SHADOW_PEAK: f32 = 0.92;
    pub const STRIP_SHADOW_SOLID_END: f32 = 0.12;

    /// band of the blur below the pack strip, growing with the first pixels scrolled under it
    /// zero at rest, so nothing but the panel shows behind the strip
    pub fn strip_edge_blur_height(scroll_top: gpui::ListOffset) -> f32 {
        if scroll_top.item_ix > 0 {
            Self::STRIP_EDGE_BLUR_HEIGHT
        } else {
            f32::from(scroll_top.offset_in_item).clamp(0.0, Self::STRIP_EDGE_BLUR_HEIGHT)
        }
    }

    /// edge blur behind the pack strip with the grid under it
    /// blur and tint both fall from the top edge down on one eased curve with no held part
    /// a held plate under the strip met the falling curve at a kink the eye read as a line
    /// tint and radius fade in with the band so the strip at rest has no plate of its own
    pub fn strip_edge_blur(scroll_top: gpui::ListOffset, theme: &AppTheme) -> Option<gpui::AnyElement> {
        let band = Self::strip_edge_blur_height(scroll_top);
        if band <= 0.0 {
            return None;
        }
        let dark = theme.bg._100.l < 0.5;
        let height = Self::TOP_STRIP_HEIGHT + band;
        let progress = band / Self::STRIP_EDGE_BLUR_HEIGHT;
        let shadow = ScrollEdgeBlurFalloff::mask_image(theme.bg._100.opacity(Self::STRIP_SHADOW_PEAK * progress), Self::STRIP_SHADOW_SOLID_END);
        Some(
            pass_through()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(height))
                .child(
                    ScrollEdgeBlurUi::new()
                        .solid_end(0.0)
                        .max_radius(ScrollEdgeBlurFalloff::EDGE_RADIUS * progress)
                        .tint_peak(ScrollEdgeBlurFalloff::header_tint_peak(dark) * progress),
                )
                .child(
                    pass_through()
                        .absolute()
                        .inset_0()
                        .child(LinearGradient::new(shadow, UnitPoint::TOP, UnitPoint::BOTTOM)),
                )
                .into_any_element(),
        )
    }
    pub const GRID_BOTTOM_INSET: f32 = 8.0;
    pub const BOTTOM_BAR_VERTICAL_PADDING: f32 = 8.0;
    pub const SEGMENT_HEIGHT: f32 = 38.0;
    pub const FLAT_BAR_BUTTON_FILL_ALPHA: f32 = 0.78;
    pub const FLAT_BAR_BUTTON_STROKE_ALPHA: f32 = 0.08;
    pub const FLAT_BAR_BUTTON_STROKE_WIDTH: f32 = 0.6;

    pub const SECTION_HEADER_FONT_SIZE: f32 = 12.5;
    pub const SECTION_HEADER_CONTENT_HEIGHT: f32 = 24.0;
    pub const GRID_SPACING: f32 = 4.0;
    pub const STICKER_CELL_INSET: f32 = 3.0;
}

const STICKER_MEDIA_DISPLAY_URL_CACHE_LIMIT: usize = 2_048;

static STICKER_MEDIA_DISPLAY_URL_CACHE: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub enum StickerMediaDisplayURL {}

impl StickerMediaDisplayURL {
    pub fn resolve(raw: &str) -> String {
        if let Some(hit) = STICKER_MEDIA_DISPLAY_URL_CACHE.lock().get(raw) {
            return hit.clone();
        }

        let resolved = Self::compute(raw);

        let mut cache = STICKER_MEDIA_DISPLAY_URL_CACHE.lock();
        if cache.len() >= STICKER_MEDIA_DISPLAY_URL_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(raw.to_owned(), resolved.clone());
        resolved
    }

    fn compute(raw: &str) -> String {
        let value = trimming_whitespaces_and_newlines(raw).to_owned();
        if value.is_empty() {
            return String::new();
        }

        if is_local_absolute_path(&value) && Path::new(&value).exists() {
            return Url::from_file_path(Path::new(&value))
                .map_or_else(|| value.clone(), |url| url.absolute_string().to_owned());
        }
        if let Some(normalized) = RemoteMediaURL::optional_normalized(Some(value.as_str()))
            && (normalized != value || Url::from_string(&normalized).and_then(|url| url.scheme().map(str::to_owned)).is_some())
        {
            return normalized;
        }
        if !value.starts_with('/')
            || value.len() > 2_048
            || value.contains("..")
            || value.contains('\\')
            || value.contains('#')
            || value.chars().any(scalar_is_control_character)
        {
            return value;
        }
        let Some(mut components) = UrlComponents::from_url(&HttpService::resolved_api_base_url(), false) else {
            return value;
        };
        let scheme = components.scheme().map(str::to_lowercase);
        if !matches!(scheme.as_deref(), Some("http" | "https")) || components.host().is_none() {
            return value;
        }
        let (path, query) = match value.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (value.as_str(), None),
        };
        components.set_path(path);
        match query {
            Some(query) => {
                let query = escaping_stray_percent_signs(query);
                let allowed = UrlAllowedCharacters::URL_QUERY_ALLOWED.union("%");
                let encoded_query = adding_percent_encoding(&query, allowed);
                components.set_percent_encoded_query(Some(&encoded_query));
            }
            None => components.set_percent_encoded_query(None),
        }
        components.set_fragment(None);
        components.url().map_or(value, |url| url.absolute_string().to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatInputPackSectionPlan {
    pub resolved_pack_ids: Vec<String>,
    pub next_unresolved_pack_id: Option<String>,
}

/// single pass over the saved pack list giving all a picker tab body needs
/// deduped order, section plan and resolved subset
/// plus the next pack to load
#[derive(Clone, Debug)]
pub struct ChatInputPackLayout {
    pub ordered: Vec<StickerPack>,
    pub visible: Vec<StickerPack>,
    pub plan: ChatInputPackSectionPlan,
    pub resolved: Vec<StickerPack>,
    pub next_unresolved: Option<StickerPack>,
}

impl ChatInputPackLayout {
    /// builds the layout with no pack unavailable and the picker pack limit
    pub fn new(saved: &[StickerPack], is_resolved: impl Fn(&str) -> bool) -> Self {
        Self::with_options(saved, is_resolved, |_| false, RiseStickerPerformancePolicy::packs_limit())
    }

    /// builds the layout with unavailable check and limit
    pub fn with_options(
        saved: &[StickerPack],
        is_resolved: impl Fn(&str) -> bool,
        is_unavailable: impl Fn(&str) -> bool,
        limit: i64,
    ) -> Self {
        let mut seen = HashSet::new();
        let ordered: Vec<StickerPack> = saved
            .iter()
            .filter(|pack| !is_unavailable(&pack.id) && seen.insert(pack.id.clone()))
            .cloned()
            .collect();

        let resolved_ids: HashSet<String> =
            ordered.iter().filter(|pack| is_resolved(&pack.id)).map(|pack| pack.id.clone()).collect();
        let ordered_ids: Vec<String> = ordered.iter().map(|pack| pack.id.clone()).collect();
        let plan = ChatInputPackSectionPolicy::plan(&ordered_ids, &resolved_ids, limit);
        let visible = ordered.iter().take(bounded_limit(limit)).cloned().collect();

        let planned: HashSet<&str> = plan.resolved_pack_ids.iter().map(String::as_str).collect();
        let resolved = ordered.iter().filter(|pack| planned.contains(pack.id.as_str())).cloned().collect();
        let next_unresolved = plan
            .next_unresolved_pack_id
            .as_ref()
            .and_then(|id| ordered.iter().find(|pack| &pack.id == id).cloned());

        Self { ordered, visible, plan, resolved, next_unresolved }
    }
}

fn bounded_limit(limit: i64) -> usize {
    usize::try_from(limit.max(0)).unwrap_or(usize::MAX)
}

pub enum ChatInputPackSectionPolicy {}

impl ChatInputPackSectionPolicy {
    pub const PLACEHOLDER_ROWS: usize = 2;

    /// plans sections from ordered pack ids, limit defaults to the sticker packs limit
    pub fn plan(ordered_pack_ids: &[String], resolved_pack_ids: &HashSet<String>, limit: i64) -> ChatInputPackSectionPlan {
        let mut seen = HashSet::new();
        let bounded: Vec<&String> = ordered_pack_ids
            .iter()
            .filter(|id| !id.is_empty() && seen.insert(id.as_str()))
            .take(bounded_limit(limit))
            .collect();
        ChatInputPackSectionPlan {
            resolved_pack_ids: bounded.iter().filter(|id| resolved_pack_ids.contains(**id)).map(|id| (*id).clone()).collect(),
            next_unresolved_pack_id: bounded.iter().find(|id| !resolved_pack_ids.contains(**id)).map(|id| (*id).clone()),
        }
    }
}

fn is_renderable(source: &str) -> bool {
    let Some(scheme) = Url::from_string(source).and_then(|url| url.scheme().map(str::to_lowercase)) else {
        return false;
    };
    matches!(scheme.as_str(), "http" | "https" | "file")
}

fn decode_dimension(size: f32) -> i64 {
    ((size * 3.0).ceil() as i64).max(1)
}

#[derive(IntoElement)]
pub struct ChatInputStickerAsset {
    item: StickerItem,
    size: f32,
    is_playing: bool,
    loops: bool,
    shows_poster_until_first_frame: bool,
    playback_kind: AnimatedMediaPlaybackKind,
}

impl ChatInputStickerAsset {
    pub fn new(item: StickerItem, size: f32) -> Self {
        Self {
            item,
            size,
            is_playing: true,
            loops: true,
            shows_poster_until_first_frame: false,
            playback_kind: AnimatedMediaPlaybackKind::Lottie,
        }
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn loops(mut self, loops: bool) -> Self {
        self.loops = loops;
        self
    }

    pub fn shows_poster_until_first_frame(mut self, shows_poster_until_first_frame: bool) -> Self {
        self.shows_poster_until_first_frame = shows_poster_until_first_frame;
        self
    }

    pub fn playback_kind(mut self, playback_kind: AnimatedMediaPlaybackKind) -> Self {
        self.playback_kind = playback_kind;
        self
    }

    fn element_id(&self, part: &str) -> ElementId {
        ElementId::Name(format!("chat-input-sticker-asset/{part}/{}", self.item.id).into())
    }

    /// offline a fetch only waits for the network so the cell rests on its grey block instead of spinning
    fn shows_progress_indicator(&self, cx: &mut App) -> bool {
        !self.shows_poster_until_first_frame && RiseTransportStatus::shown(cx).is_connected()
    }

    fn glyph_placeholder(&self) -> Option<Hsla> {
        self.shows_poster_until_first_frame.then(transparent_black)
    }

    fn resolved_thumbnail(&self) -> Option<String> {
        self.item
            .thumbnail_url
            .as_deref()
            .map(StickerMediaDisplayURL::resolve)
            .filter(|thumbnail| is_renderable(thumbnail))
    }

    fn raster_preview_url(&self) -> String {
        if self.item.asset_type == StickerAssetType::Video
            && let Some(thumbnail) = self.resolved_thumbnail()
        {
            return thumbnail;
        }
        StickerMediaDisplayURL::resolve(&self.item.file_url)
    }

    fn photo_library_preview(source: &str) -> Option<ImageSource> {
        let prefix = format!("{}://", MediaPickerPhotoAssetSource::SCHEME);
        if !source.to_lowercase().starts_with(&prefix) {
            return None;
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        MediaViewerImageMemoryCache::memory_only(source, account_user_id.as_deref()).map(ImageSource::Render)
    }

    fn clever_image(&self, id: &str, source: String, shows_progress_indicator: bool) -> CleverImageUi {
        CleverImageUi::new(source)
            .id(self.element_id(id))
            .content_mode(ContentMode::Fit)
            .corner_radius(0.0)
            .persist_to_disk(true)
            .shows_progress_indicator(shows_progress_indicator)
            .maximum_decode_pixel_dimension(decode_dimension(self.size))
            .shows_failure_icon(!self.shows_poster_until_first_frame)
            .placeholder_color(self.glyph_placeholder())
    }

    fn lottie_poster(&self, url: &str) -> AnyElement {
        if let Some(thumbnail) = self.resolved_thumbnail() {
            let poster = CleverImageUi::new(thumbnail)
                .id(self.element_id("lottie-poster"))
                .content_mode(ContentMode::Fit)
                .corner_radius(0.0)
                .persist_to_disk(true)
                .shows_progress_indicator(false)
                .maximum_decode_pixel_dimension(decode_dimension(self.size))
                .shows_failure_icon(false)
                .placeholder_color(Some(transparent_black()));
            return div().frame_size(self.size, self.size).child(poster).into_any_element();
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let point_size = size(px(self.size), px(self.size));
        let frame = RiseLottieFirstFrameCache::memory_image(url, point_size, account_user_id.as_deref())
            .or_else(|| RiseLottieFirstFrameCache::latest_memory_image(url, account_user_id.as_deref()));
        match frame {
            Some(image) => img(ImageSource::Render(image)).object_fit(ObjectFit::Contain).size_full().into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn video_poster(&self, source: &str, kind: ChatAnimatedMediaPlaybackKind, cx: &mut App) -> AnyElement {
        if let Some(thumbnail) = self.resolved_thumbnail() {
            let shows_progress_indicator = self.shows_progress_indicator(cx);
            return self.clever_image("video-poster", thumbnail, shows_progress_indicator).into_any_element();
        }
        let is_local_web_m = kind == ChatAnimatedMediaPlaybackKind::WebMVideo && source.to_lowercase().starts_with("file:");
        if kind == ChatAnimatedMediaPlaybackKind::AvFoundationVideo || is_local_web_m {
            return VideoStickerPosterView::new(self.element_id("video-poster"), source, size(px(self.size), px(self.size)))
                .into_any_element();
        }
        div().into_any_element()
    }

    fn video_asset(&self, source: String, cx: &mut App) -> AnyElement {
        let kind = ChatAnimatedMediaSourcePolicy::playback_kind(&source, self.item.mime_type.as_deref(), self.item.codec.as_deref());
        if kind == ChatAnimatedMediaPlaybackKind::WebMVideo && self.is_playing {
            let poster = self.video_poster(&source, kind, cx);
            return div()
                .frame_size(self.size, self.size)
                .overflow_hidden()
                .child(ChatWebMVideoStack::new(self.element_id("webm"), source).poster(poster))
                .into_any_element();
        }
        let mut stack = zstack(Alignment::Center).child(self.video_poster(&source, kind, cx));
        if kind == ChatAnimatedMediaPlaybackKind::AnimatedImage {
            stack = stack.child(
                AnimatedGifView::new(self.element_id("gif"), source)
                    .content_mode(AnimatedGifContentMode::ScaleAspectFit)
                    .is_playing(self.is_playing),
            );
        } else if kind == ChatAnimatedMediaPlaybackKind::AvFoundationVideo && self.is_playing {
            stack = stack.child(ChatLoopingVideoView::new(self.element_id("looping"), source));
        }
        stack.frame_size(self.size, self.size).overflow_hidden().into_any_element()
    }

    fn placeholder(&self) -> AnyElement {
        let glyph = self.item.associated_emojis.first().cloned().unwrap_or_else(|| "\u{1F642}".to_owned());
        div().swiftui_font(Font::system_size(self.size * 0.66)).child(glyph).into_any_element()
    }
}

impl RenderOnce for ChatInputStickerAsset {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let url = StickerMediaDisplayURL::resolve(&self.item.file_url);
        let lottie_identity = format!("{}|{}", self.item.id, url);
        let rendered_lottie_id = window.use_keyed_state(self.element_id("rendered-lottie"), cx, |_, _| None::<String>);

        let content = if let Some(image) = Self::photo_library_preview(&url) {
            img(image).object_fit(ObjectFit::Contain).size_full().into_any_element()
        } else if is_renderable(&url) {
            match self.item.asset_type {
                StickerAssetType::Lottie => {
                    let mut stack = zstack(Alignment::Center);
                    if self.shows_poster_until_first_frame
                        && rendered_lottie_id.read(cx).as_deref() != Some(lottie_identity.as_str())
                    {
                        stack = stack.child(self.lottie_poster(&url));
                    }
                    let mut player = LottieStickerView::new(self.element_id("lottie"), url.clone())
                        .size(self.size)
                        .r#loop(self.loops)
                        .is_playing(self.is_playing)
                        .playback_id(Some(self.item.id.clone()))
                        .playback_kind(self.playback_kind);
                    if self.shows_poster_until_first_frame {
                        let rendered = rendered_lottie_id.clone();
                        player = player.on_first_frame(move |cx| {
                            let identity = lottie_identity.clone();
                            rendered.update(cx, |rendered, cx| {
                                *rendered = Some(identity);
                                cx.notify();
                            });
                        });
                    }
                    stack.child(player).into_any_element()
                }
                StickerAssetType::Video => self.video_asset(url, cx),
                StickerAssetType::StaticImage => {
                    let shows_progress_indicator = self.shows_progress_indicator(cx);
                    div()
                        .frame_size(self.size, self.size)
                        .child(self.clever_image("static", self.raster_preview_url(), shows_progress_indicator))
                        .into_any_element()
                }
            }
        } else {
            self.placeholder()
        };

        zstack(Alignment::Center).child(content).frame_size(self.size, self.size)
    }
}

#[derive(IntoElement)]
pub struct ChatInputStickerCell {
    item: StickerItem,
    size: f32,
    is_playing: bool,
    allows_hold_preview: bool,
}

impl ChatInputStickerCell {
    pub fn new(item: StickerItem, size: f32) -> Self {
        Self { item, size, is_playing: true, allows_hold_preview: true }
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn allows_hold_preview(mut self, allows_hold_preview: bool) -> Self {
        self.allows_hold_preview = allows_hold_preview;
        self
    }

    fn cancel(upload: &RiseStickerUploadPresentation, cx: &mut App) {
        impact(ImpactStyle::Light);
        ChatInputPickerKitHooks::resolved_cancel_upload(upload, cx);
    }
}

struct StickerServicesObservation {
    _subscription: Option<Subscription>,
}

impl RenderOnce for ChatInputStickerCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let observation_id = ElementId::Name(format!("chat-input-sticker-cell/services/{}", self.item.id).into());
        let view = window.current_view();
        let observed_id = self.item.id.clone();
        window.use_keyed_state(observation_id, cx, move |_, cx| {
            let last_upload = Rc::new(RefCell::new(ChatInputPickerKitHooks::resolved_sticker_upload(&observed_id, cx)));
            StickerServicesObservation {
                _subscription: ChatInputPickerKitHooks::resolved_observe(
                    Box::new(move |cx: &mut App| {
                        let upload = ChatInputPickerKitHooks::resolved_sticker_upload(&observed_id, cx);
                        if *last_upload.borrow() != upload {
                            *last_upload.borrow_mut() = upload;
                            cx.notify(view);
                        }
                    }),
                    cx,
                ),
            }
        });

        let upload = ChatInputPickerKitHooks::resolved_sticker_upload(&self.item.id, cx);
        let asset = ChatInputStickerAsset::new(
            self.item.clone(),
            (self.size - ChatInputPickerMetrics::STICKER_CELL_INSET * 2.0).max(1.0),
        )
        .is_playing(self.is_playing && upload.is_none());
        let cell = zstack(Alignment::Center).child(asset).frame_size(self.size, self.size);

        if let Some(upload) = upload {
            return cell
                .child(StickerUploadCellOverlay {
                    id: ElementId::Name(format!("chat-input-sticker-cell/upload/{}", self.item.id).into()),
                    upload: upload.clone(),
                    size: self.size,
                    on_cancel: Rc::new(move |_window: &mut Window, cx: &mut App| Self::cancel(&upload, cx)),
                })
                .into_any_element();
        }
        if self.allows_hold_preview {
            let main_emoji = self.item.associated_emojis.first().cloned();
            let key = self.item.id.clone();
            return cell
                .sticker_hold_cell_frame(key, StickerPreviewAsset::Sticker(self.item), main_emoji)
                .into_any_element();
        }
        cell.into_any_element()
    }
}

const STICKER_UPLOAD_RING_ANIMATION_SECONDS: f64 = 0.12;

#[derive(IntoElement)]
struct StickerUploadCellOverlay {
    id: ElementId,
    upload: RiseStickerUploadPresentation,
    size: f32,
    on_cancel: ButtonAction,
}

impl StickerUploadCellOverlay {
    fn ring_size(&self) -> f32 {
        (self.size * 0.44).clamp(18.0, 34.0)
    }

    fn icon_size(&self) -> f32 {
        (self.ring_size() * 0.30).clamp(6.0, 10.0)
    }

    fn ring_thickness(&self) -> f32 {
        (self.ring_size() * 0.055).clamp(1.1, 1.8)
    }

    fn ring_gap(&self) -> f32 {
        (self.ring_size() * 0.07).clamp(1.5, 2.5)
    }

    fn status_font_size(&self) -> f32 {
        (self.size * 0.09).clamp(6.0, 7.0)
    }

    fn hit_target_size(&self) -> f32 {
        self.ring_size().max(self.size * 0.82).min(44.0)
    }

    fn status_text(&self) -> String {
        if self.upload.phase == RiseStickerMutationState::Submitting {
            return tr("sticker_pack_media_sending");
        }
        if self.upload.phase == RiseStickerMutationState::Preparing || self.has_transferred_all_bytes() {
            return tr("sticker_pack_media_processing");
        }
        if self.upload.total_bytes <= 0 {
            return format!("{}%", (self.upload.fraction * 100.0).round() as i64);
        }
        let completed = ByteFormat::telegram_transferred(self.upload.completed_bytes, self.upload.total_bytes);
        let total = ByteFormat::telegram(self.upload.total_bytes);
        if self.size < 58.0 {
            return format!("{}/{}", Self::compact(&completed), Self::compact(&total));
        }
        format_printf(
            &tr("chat_media_upload_progress"),
            &[FormatArg::Text(completed.into()), FormatArg::Text(total.into())],
        )
    }

    fn displayed_progress(&self) -> f64 {
        if self.upload.phase == RiseStickerMutationState::Submitting || self.has_transferred_all_bytes() {
            return 1.0;
        }
        self.upload.fraction.clamp(0.08, 1.0)
    }

    fn has_transferred_all_bytes(&self) -> bool {
        self.upload.phase == RiseStickerMutationState::Uploading
            && self.upload.total_bytes > 0
            && self.upload.completed_bytes >= self.upload.total_bytes
    }

    fn compact(value: &str) -> String {
        value.chars().filter(|c| !matches!(c, ' ' | '\u{00A0}' | '\u{202F}')).collect()
    }

    fn child_id(&self, part: &str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), part.into())
    }
}

impl RenderOnce for StickerUploadCellOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ring = self.ring_size();
        let now = RiseClock::monotonic();
        let target = self.displayed_progress() as f32;
        let progress = window.use_keyed_state(self.child_id("ring-progress"), cx, |_, _| Animated::new(target));
        progress.update(cx, |animated, cx| {
            animated.set_in(target, linear(STICKER_UPLOAD_RING_ANIMATION_SECONDS), now, cx);
        });
        let shown_progress = progress.read(cx).frame_value(now, window);

        let status = div()
            .px(px((self.size * 0.03).clamp(1.5, 2.5)))
            .py(px((self.size * 0.018).clamp(1.0, 2.0)))
            .capsule()
            .bg(black().opacity(0.48))
            .swiftui_font(Font::system_weight(self.status_font_size(), FontWeight::SEMIBOLD))
            .text_color(white().opacity(0.96))
            .child(MinimumScaleText::new(self.status_text(), 0.72).line_limit(1));

        let on_cancel = self.on_cancel.clone();
        let cancel = plain(self.child_id("cancel"))
            .on_press(move |window, cx| on_cancel(window, cx))
            .frame_size(self.hit_target_size(), self.hit_target_size())
            .circle()
            .flex()
            .items_center()
            .justify_center()
            .child(
                zstack(Alignment::Center)
                    .frame_size(ring, ring)
                    .child(div().size_full().circle().bg(black().opacity(0.50)))
                    .child(
                        LoaderUi::new()
                            .size(LoaderUiSize::Custom(ring))
                            .progress(f64::from(shown_progress))
                            .r#type(LoaderUiType::Progress)
                            .thickness(self.ring_thickness())
                            .track_color(Some(white().opacity(0.28)))
                            .ring_animated(true)
                            .speed(3.0)
                            .gap(self.ring_gap()),
                    )
                    .children(IconUi::sized("xmark", px(self.icon_size()), white(), false)),
            );

        zstack(Alignment::Center)
            .size_full()
            .child(div().size_full().bg(black().opacity(0.30)))
            .child(
                vstack(0.0)
                    .size_full()
                    .px(px(2.0))
                    .pt(px((self.size * 0.03).max(2.0)))
                    .child(status)
                    .child(spacer_min_h(0.0)),
            )
            .child(cancel)
            .clip_shape(Shape::continuous((self.size * 0.22).min(15.0)))
    }
}

/// true where the picker sits on the theme background and not dark glass
/// so labels take theme colors and not white
pub struct ChatInputPickerFlatChromeKey;

impl EnvKey for ChatInputPickerFlatChromeKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// reads the flat chrome flag from the environment
pub fn chat_input_picker_flat_chrome(cx: &App) -> bool {
    env::<ChatInputPickerFlatChromeKey>(cx)
}

/// height of the picker bar floating over the grids, added to each grid tail
/// so grids run under the bar and the last row scrolls clear. zero outside the picker
pub struct ChatInputPickerBarReserveKey;

impl EnvKey for ChatInputPickerBarReserveKey {
    type Value = f32;

    fn default_value() -> Self::Value {
        0.0
    }
}

pub fn chat_input_picker_bar_reserve(cx: &App) -> f32 {
    env::<ChatInputPickerBarReserveKey>(cx)
}

#[derive(IntoElement)]
pub struct ChatInputSectionHeader {
    id: ElementId,
    title: SharedString,
    trailing: Option<AnyElement>,
    title_action: Option<ButtonAction>,
}

impl ChatInputSectionHeader {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self { id: id.into(), title: title.into(), trailing: None, title_action: None }
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }

    pub fn title_action(mut self, title_action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.title_action = Some(Rc::new(title_action));
        self
    }
}

impl RenderOnce for ChatInputSectionHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let flat_chrome = chat_input_picker_flat_chrome(cx);
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let color = if flat_chrome { theme.text.secondary } else { white().opacity(0.38) };
        let title_label = div()
            .w_full()
            .swiftui_font(Font::system_weight(ChatInputPickerMetrics::SECTION_HEADER_FONT_SIZE, FontWeight::SEMIBOLD))
            .text_color(color)
            .whitespace_nowrap()
            .truncate()
            .child(self.title.to_uppercase());

        let title = match self.title_action {
            Some(action) => plain(ElementId::NamedChild(Arc::new(self.id.clone()), "title".into()))
                .on_press(move |window, cx| action(window, cx))
                .w_full()
                .child(title_label)
                .into_any_element(),
            None => title_label.into_any_element(),
        };
        let mut stack = zstack(Alignment::Leading).size_full().child(title);
        if let Some(trailing) = self.trailing {
            stack = stack.child(
                hstack(DEFAULT_STACK_SPACING)
                    .size_full()
                    .child(spacer())
                    .child(div().flex().items_center().occlude().child(trailing)),
            );
        }
        div()
            .h(px(ChatInputPickerMetrics::SECTION_HEADER_CONTENT_HEIGHT + 12.0 + 4.0))
            .px(px(10.0))
            .pt(px(12.0))
            .pb(px(4.0))
            .child(stack)
    }
}

/// lock in a premium pack header on a free account
/// pack stays visible and the lock says why a tap opens the subscription
#[derive(IntoElement)]
pub struct StickerPackLockBadge {
    id: ElementId,
    action: ButtonAction,
}

impl StickerPackLockBadge {
    pub fn new(id: impl Into<ElementId>, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), action: Rc::new(action) }
    }
}

impl RenderOnce for StickerPackLockBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let flat_chrome = chat_input_picker_flat_chrome(cx);
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let foreground = if flat_chrome { theme.text.secondary } else { white().opacity(0.62) };
        let background = if flat_chrome { theme.bg._300 } else { white().opacity(0.12) };
        let action = self.action.clone();
        plain(self.id).on_press(move |window, cx| action(window, cx)).child(
            hstack(4.0)
                .px(px(8.0))
                .py(px(3.0))
                .bg(background)
                .clip_shape(Shape::Capsule)
                .text_color(foreground)
                .children(IconUi::sized("lock.fill", px(10.0), foreground, true))
                .child(
                    div()
                        .swiftui_font(Font::system_weight(ChatInputPickerMetrics::SECTION_HEADER_FONT_SIZE, FontWeight::SEMIBOLD))
                        .child(tr("sticker_pack_premium_locked").to_uppercase()),
                ),
        )
    }
}

#[derive(Clone, Debug)]
pub struct StickerItemDeletionRequest {
    pub item: StickerItem,
    pub pack: StickerPack,
}

impl StickerItemDeletionRequest {
    pub fn id(&self) -> &str {
        &self.item.id
    }
}

/// value a sticker carries while dragged in the edit grid
#[derive(Clone, Debug)]
pub struct StickerReorderDrag {
    pub sticker_id: String,
}

pub struct StickerReorderDragPreview {
    item: StickerItem,
    size: f32,
}

impl Render for StickerReorderDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().p(px(4.0)).child(ChatInputStickerAsset::new(self.item.clone(), self.size).is_playing(false))
    }
}

#[derive(IntoElement)]
pub struct ChatInputEditableStickerCell {
    item: StickerItem,
    size: f32,
    is_playing: bool,
    is_editing: bool,
    order: Binding<Vec<String>>,
    dragged_sticker_id: Binding<Option<String>>,
    on_delete: Option<ButtonAction>,
    can_reorder: bool,
}

impl ChatInputEditableStickerCell {
    pub fn new(
        item: StickerItem,
        size: f32,
        is_playing: bool,
        is_editing: bool,
        order: Binding<Vec<String>>,
        dragged_sticker_id: Binding<Option<String>>,
        on_delete: Option<ButtonAction>,
    ) -> Self {
        Self { item, size, is_playing, is_editing, order, dragged_sticker_id, on_delete, can_reorder: true }
    }

    /// can reorder, default true
    pub fn can_reorder(mut self, can_reorder: bool) -> Self {
        self.can_reorder = can_reorder;
        self
    }

    fn element_id(&self, part: &str) -> ElementId {
        ElementId::Name(format!("chat-input-editable-sticker/{part}/{}", self.item.id).into())
    }
}

impl RenderOnce for ChatInputEditableStickerCell {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut stack = zstack(Alignment::TopTrailing).child(
            ChatInputStickerCell::new(self.item.clone(), self.size)
                .is_playing(self.is_playing)
                .allows_hold_preview(!self.is_editing),
        );

        if self.is_editing && self.can_reorder {
            let dragged = self.dragged_sticker_id.clone();
            let drag_item = self.item.clone();
            let drag_size = self.size;
            stack = stack.child(
                div()
                    .id(self.element_id("drag"))
                    .frame_size(self.size, self.size)
                    .cursor_grab()
                    .on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, _, _| {
                        tracing::warn!(target: "riseonly::X1622", position = ?event.position, "reorder handle press");
                    })
                    .on_mouse_move(|event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            tracing::warn!(target: "riseonly::X1622", position = ?event.position, active_drag = cx.has_active_drag(), "reorder handle pressed move");
                        }
                    })
                    .on_drag(StickerReorderDrag { sticker_id: self.item.id.clone() }, move |drag, _, window, cx| {
                        tracing::warn!(target: "riseonly::X1622", sticker = %drag.sticker_id, "reorder drag began");
                        dragged.set(Some(drag.sticker_id.clone()), window, cx);
                        impact(ImpactStyle::Medium);
                        let item = drag_item.clone();
                        cx.new(|_| StickerReorderDragPreview { item, size: drag_size })
                    }),
            );
        }
        if self.is_editing
            && let Some(on_delete) = self.on_delete.clone()
        {
            stack = stack.child(
                div().mt(px(2.0)).mr(px(2.0)).child(
                    plain(self.element_id("delete"))
                        .on_press(move |window, cx| {
                            impact(ImpactStyle::Light);
                            on_delete(window, cx)
                        })
                        .child(
                            div()
                                .id(self.element_id("delete-label"))
                                .tooltip_label(tr("delete_action"))
                                .child(
                                    zstack(Alignment::Center)
                                        .frame_size(20.0, 20.0)
                                        .child(div().size_full().circle().bg(red()))
                                        .child(centred_stroke(white().opacity(0.92), 1.4, Shape::Circle).size_full())
                                        .children(IconUi::sized("minus", px(10.0), white(), false)),
                                ),
                        ),
                ),
            );
        }

        let destination_id = self.item.id.clone();
        let is_enabled = self.is_editing;
        let order = self.order.clone();
        let dragged_for_move = self.dragged_sticker_id.clone();
        let drop_delegate = StickerReorderDropDelegate {
            destination_id: self.item.id.clone(),
            is_enabled,
            order: self.order.clone(),
            dragged_sticker_id: self.dragged_sticker_id.clone(),
        };
        div()
            .id(self.element_id("drop"))
            .frame_size(self.size, self.size)
            .child(stack)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, _| {
                if is_enabled {
                    tracing::warn!(target: "riseonly::X1622", position = ?event.position, "reorder cell press");
                }
            })
            .on_drag_move::<StickerReorderDrag>(move |event: &DragMoveEvent<StickerReorderDrag>, window, cx| {
                let delegate = StickerReorderDropDelegate {
                    destination_id: destination_id.clone(),
                    is_enabled,
                    order: order.clone(),
                    dragged_sticker_id: dragged_for_move.clone(),
                };
                if !event.bounds.contains(&event.event.position) {
                    delegate.drop_exited();
                    return;
                }
                if delegate.drop_updated() == StickerReorderDropOperation::Move {
                    delegate.drop_entered(window, cx);
                }
            })
            .on_drop::<StickerReorderDrag>(move |_, window, cx| {
                drop_delegate.perform_drop(window, cx);
            })
    }
}

/// what the cell under the dragged sticker accepts
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StickerReorderDropOperation {
    Forbidden,
    Move,
}

struct StickerReorderDropDelegate {
    destination_id: String,
    is_enabled: bool,
    order: Binding<Vec<String>>,
    dragged_sticker_id: Binding<Option<String>>,
}

impl StickerReorderDropDelegate {
    fn drop_entered(&self, window: &mut Window, cx: &mut App) {
        if !self.is_enabled {
            return;
        }
        let Some(dragged_sticker_id) = self.dragged_sticker_id.get(cx) else {
            tracing::warn!(target: "riseonly::X1622", destination = %self.destination_id, "reorder entry without a dragged sticker");
            return;
        };
        if dragged_sticker_id == self.destination_id {
            return;
        }
        let mut order = self.order.get(cx);
        let Some(source) = order.iter().position(|id| *id == dragged_sticker_id) else {
            return;
        };
        let Some(destination) = order.iter().position(|id| *id == self.destination_id) else {
            return;
        };
        tracing::warn!(target: "riseonly::X1622", source, destination, "reorder moved");
        let moved = order.remove(source);
        order.insert(destination, moved);
        self.order.set(order, window, cx);
    }

    fn drop_updated(&self) -> StickerReorderDropOperation {
        if self.is_enabled { StickerReorderDropOperation::Move } else { StickerReorderDropOperation::Forbidden }
    }

    fn perform_drop(&self, window: &mut Window, cx: &mut App) -> bool {
        if !self.is_enabled {
            return false;
        }
        self.dragged_sticker_id.set(None, window, cx);
        true
    }

    fn drop_exited(&self) {}
}

#[derive(IntoElement)]
pub struct ChatInputPackLoadingSection {
    title: SharedString,
    anchor: SharedString,
    columns: usize,
    cell_size: f32,
    is_active: bool,
}

impl ChatInputPackLoadingSection {
    pub fn new(
        title: impl Into<SharedString>,
        anchor: impl Into<SharedString>,
        columns: usize,
        cell_size: f32,
        is_active: bool,
    ) -> Self {
        Self { title: title.into(), anchor: anchor.into(), columns, cell_size, is_active }
    }

    fn placeholder_grid(&self) -> Div {
        let side = (self.cell_size - 8.0).max(1.0);
        let count = self.columns * ChatInputPackSectionPolicy::PLACEHOLDER_ROWS;
        div()
            .grid()
            .grid_cols(self.columns.max(1) as u16)
            .min_h(px(self.cell_size * ChatInputPackSectionPolicy::PLACEHOLDER_ROWS as f32))
            .children((0..count).map(|_| {
                div()
                    .frame_size(self.cell_size, self.cell_size)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(SkeletonBlock::new(side, side, 12.0))
            }))
    }
}

impl RenderOnce for ChatInputPackLoadingSection {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let header = ChatInputSectionHeader::new(ElementId::Name(self.anchor.clone()), self.title.clone());
        let grid = self.placeholder_grid();
        let grid = if self.is_active { grid.skeleton_shimmer(None).into_any_element() } else { grid.into_any_element() };
        vstack(0.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(div().id(ElementId::Name(self.anchor)).w_full().child(header))
            .child(div().w_full().child(grid))
    }
}

const SEGMENT_INDICATOR_SPRING: (f64, f64) = (0.36, 0.86);

/// tab label frames of the segmented control in window coords
/// plus the control own origin that makes them local
#[derive(Default)]
struct SegFrameKey {
    origin: Option<gpui::Point<Pixels>>,
    frames: HashMap<i64, Bounds<Pixels>>,
    notified_shape: Option<Vec<(i64, i32, i32)>>,
    check_pending: bool,
}

impl SegFrameKey {
    /// merges frames, later ones win
    fn reduce(&mut self, next: HashMap<i64, Bounds<Pixels>>) {
        for (id, bounds) in next {
            self.frames.insert(id, bounds);
        }
    }

    fn local_shape(&self) -> Option<Vec<(i64, i32, i32)>> {
        let origin = self.origin?;
        let quarter = |value: Pixels| (f32::from(value) * 4.0).round() as i32;
        let mut shape: Vec<(i64, i32, i32)> = self
            .frames
            .iter()
            .map(|(id, bounds)| (*id, quarter(bounds.origin.x - origin.x), quarter(bounds.size.width)))
            .collect();
        shape.sort_unstable();
        Some(shape)
    }

    /// compares local geometry once after every measurer of this frame has reported
    fn schedule_check(this: &gpui::Entity<Self>, window: &mut Window, cx: &mut App) {
        if this.update(cx, |key, _| std::mem::replace(&mut key.check_pending, true)) {
            return;
        }
        let this = this.clone();
        window.defer(cx, move |_, cx| {
            this.update(cx, |key, cx| {
                key.check_pending = false;
                let shape = key.local_shape();
                if shape.is_some() && shape != key.notified_shape {
                    key.notified_shape = shape;
                    cx.notify();
                }
            });
        });
    }
}

#[derive(IntoElement)]
pub struct ChatInputSegmentedTabs {
    id: ElementId,
    tabs: Vec<TabHeaderConfig>,
    active_tab: Binding<i64>,
    drag_progress: Rc<dyn Fn(&App) -> f32>,
    on_tap: Rc<dyn Fn(i64, &mut Window, &mut App)>,
    suppresses_tap: Rc<dyn Fn(&App) -> bool>,
    tab_h_padding: f32,
    uses_flat_chrome: bool,
}

impl ChatInputSegmentedTabs {
    pub fn new(
        id: impl Into<ElementId>,
        tabs: Vec<TabHeaderConfig>,
        active_tab: Binding<i64>,
        drag_progress: impl Fn(&App) -> f32 + 'static,
        on_tap: impl Fn(i64, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            tabs,
            active_tab,
            drag_progress: Rc::new(drag_progress),
            on_tap: Rc::new(on_tap),
            suppresses_tap: Rc::new(|_| false),
            tab_h_padding: 15.0,
            uses_flat_chrome: false,
        }
    }

    pub fn suppresses_tap(mut self, suppresses_tap: impl Fn(&App) -> bool + 'static) -> Self {
        self.suppresses_tap = Rc::new(suppresses_tap);
        self
    }

    pub fn tab_h_padding(mut self, tab_h_padding: f32) -> Self {
        self.tab_h_padding = tab_h_padding;
        self
    }

    pub fn uses_flat_chrome(mut self, uses_flat_chrome: bool) -> Self {
        self.uses_flat_chrome = uses_flat_chrome;
        self
    }

    fn child_id(&self, part: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), part.into())
    }

    fn label_color(&self, is_active: bool, theme: &AppTheme) -> Hsla {
        if !self.uses_flat_chrome {
            return white().opacity(if is_active { 0.96 } else { 0.5 });
        }
        if is_active { theme.text.primary } else { theme.text.secondary }
    }
}

impl RenderOnce for ChatInputSegmentedTabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let height = ChatInputPickerMetrics::SEGMENT_HEIGHT;
        let active_tab = self.active_tab.get(cx);
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let frames = window.use_keyed_state(self.child_id("frames"), cx, |_, _| SegFrameKey::default());

        let mut labels = hstack(2.0).px(px(4.0));
        for tab in &self.tabs {
            let tab_id = tab.id;
            let is_active = tab_id == active_tab;
            let on_tap = self.on_tap.clone();
            let suppresses_tap = self.suppresses_tap.clone();
            let frames_for_measure = frames.clone();
            labels = labels.child(
                div()
                    .id(self.child_id(format!("tab/{tab_id}")))
                    .relative()
                    .h(px(height))
                    .px(px(self.tab_h_padding))
                    .flex()
                    .items_center()
                    .flex_none()
                    .cursor_pointer()
                    .swiftui_font(Font::system_weight(14.0, if is_active { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM }))
                    .text_color(self.label_color(is_active, &theme))
                    .whitespace_nowrap()
                    .child(tab.text.clone())
                    .child(measure(self.child_id(format!("frame/{tab_id}")), move |bounds, window, cx| {
                        frames_for_measure.update(cx, |frames, _| frames.reduce(HashMap::from([(tab_id, bounds)])));
                        SegFrameKey::schedule_check(&frames_for_measure, window, cx);
                    }))
                    .on_click(move |_, window, cx| {
                        if suppresses_tap(cx) {
                            return;
                        }
                        on_tap(tab_id, window, cx);
                    }),
            );
        }

        let fill = if self.uses_flat_chrome { LiquidChrome::surface_fill(&theme, 1) } else { white().opacity(0.16) };
        let indicator = ChatInputSegmentedIndicator {
            id: self.child_id("indicator"),
            tabs: self.tabs.iter().map(|tab| tab.id).collect(),
            active_tab,
            frames: frames.clone(),
            height,
            drag_progress: self.drag_progress.clone(),
            fill,
        };

        let frames_for_origin = frames.clone();
        let backing_level = if self.uses_flat_chrome { 0 } else { 1 };
        let backing = div().size_full().rounded_full().bg(LiquidChrome::surface_fill(&theme, backing_level));
        let mut control = zstack(Alignment::Leading)
            .h(px(height))
            .child(backing)
            .child(indicator)
            .child(labels)
            .child(measure(self.child_id("origin"), move |bounds, window, cx| {
                frames_for_origin.update(cx, |frames, _| frames.origin = Some(bounds.origin));
                SegFrameKey::schedule_check(&frames_for_origin, window, cx);
            }));
        if self.uses_flat_chrome {
            control = control.child(
                centred_stroke(
                    theme.text.primary.opacity(ChatInputPickerMetrics::FLAT_BAR_BUTTON_STROKE_ALPHA),
                    ChatInputPickerMetrics::FLAT_BAR_BUTTON_STROKE_WIDTH,
                    Shape::Capsule,
                )
                .size_full(),
            );
        } else {
            control = control.child(centred_stroke(white().opacity(0.12), 0.6, Shape::Capsule).size_full());
        }
        control
            .occlude()
            .capture_any_mouse_down(|_, window, _| window.prevent_default())
            .clip_shape(Shape::Capsule)
    }
}

#[derive(IntoElement)]
struct ChatInputSegmentedIndicator {
    id: ElementId,
    tabs: Vec<i64>,
    active_tab: i64,
    frames: gpui::Entity<SegFrameKey>,
    height: f32,
    drag_progress: Rc<dyn Fn(&App) -> f32>,
    fill: Hsla,
}

impl ChatInputSegmentedIndicator {
    fn indicator_geom(&self, cx: &App) -> Option<(f32, f32)> {
        let frames = self.frames.read(cx);
        let origin = frames.origin?;
        let local = |id: i64| {
            frames.frames.get(&id).map(|bounds| (f32::from(bounds.origin.x - origin.x), f32::from(bounds.size.width)))
        };
        let index = self.tabs.iter().position(|id| *id == self.active_tab)?;
        let (current_x, current_width) = local(self.tabs[index])?;
        let progress = (self.drag_progress)(cx).clamp(-1.0, 1.0);
        let abs_progress = progress.abs().min(1.0);
        let neighbour = if progress < -0.001 {
            index.checked_sub(1)
        } else if progress > 0.001 {
            (index + 1 < self.tabs.len()).then_some(index + 1)
        } else {
            None
        };
        let Some((next_x, next_width)) = neighbour.and_then(|n| local(self.tabs[n])) else {
            return Some((current_x, current_width));
        };
        Some((
            current_x + (next_x - current_x) * abs_progress,
            current_width + (next_width - current_width) * abs_progress,
        ))
    }
}

impl RenderOnce for ChatInputSegmentedIndicator {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some((x, width)) = self.indicator_geom(cx) else {
            return div().into_any_element();
        };
        let now = RiseClock::monotonic();
        let active_tab = self.active_tab;
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| (active_tab, Animated::new(x)));
        state.update(cx, |(last_tab, animated), cx| {
            if *last_tab != active_tab {
                *last_tab = active_tab;
                animated.set_in(x, spring(SEGMENT_INDICATOR_SPRING.0, SEGMENT_INDICATOR_SPRING.1), now, cx);
            } else if !animated.is_running(now) {
                animated.jump(x);
            }
        });
        let shown_x = {
            let animated = &state.read(cx).1;
            if animated.is_running(now) { animated.frame_value(now, window) } else { x }
        };
        let pill_height = self.height - 8.0;
        div()
            .size_full()
            .flex()
            .items_center()
            .child(
                div()
                    .ml(px(shown_x))
                    .w(px(width))
                    .h(px(pill_height))
                    .continuous_corners(pill_height / 2.0)
                    .bg(self.fill),
            )
            .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct ChatInputStripIconButton {
    id: ElementId,
    symbol: Option<&'static str>,
    cover_item: Option<StickerItem>,
    is_active: bool,
    accessibility_label: SharedString,
    on_tap: ButtonAction,
    width: f32,
}

impl ChatInputStripIconButton {
    pub const WIDTH: f32 = 36.0;

    pub fn new(
        id: impl Into<ElementId>,
        symbol: Option<&'static str>,
        cover_item: Option<StickerItem>,
        is_active: bool,
        accessibility_label: impl Into<SharedString>,
        on_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            symbol,
            cover_item,
            is_active,
            accessibility_label: accessibility_label.into(),
            on_tap: Rc::new(on_tap),
            width: Self::WIDTH,
        }
    }

    /// narrower slot so a strip of system sections fits its row instead of clipping the last one
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
}

impl RenderOnce for ChatInputStripIconButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let flat_chrome = chat_input_picker_flat_chrome(cx);
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let highlight = if flat_chrome { theme.text.primary } else { white() };
        let symbol_color = match (flat_chrome, self.is_active) {
            (false, true) => white().opacity(0.95),
            (false, false) => white().opacity(0.45),
            (true, true) => theme.text.primary,
            (true, false) => theme.text.secondary,
        };
        let content = if let Some(cover_item) = self.cover_item {
            div()
                .opacity(if self.is_active { 1.0 } else { 0.6 })
                .child(ChatInputStickerAsset::new(cover_item, 26.0).is_playing(false))
                .into_any_element()
        } else if let Some(symbol) = self.symbol {
            div().children(IconUi::sized(symbol, px(16.0), symbol_color, false)).into_any_element()
        } else {
            div().into_any_element()
        };
        let on_tap = self.on_tap.clone();
        let label = self.accessibility_label.clone();
        plain(self.id.clone())
            .on_press(move |window, cx| {
                selection_changed();
                on_tap(window, cx)
            })
            .child(
            div()
                .id(ElementId::NamedChild(Arc::new(self.id), "label".into()))
                .frame_size(self.width, 32.0)
                .flex()
                .items_center()
                .justify_center()
                .continuous_corners(9.0)
                .bg(highlight.opacity(if self.is_active { 0.12 } else { 0.0 }))
                .tooltip_label(label)
                .child(content),
        )
    }
}

#[cfg(test)]
#[path = "chat_input_picker_kit_tests.rs"]
mod tests;
