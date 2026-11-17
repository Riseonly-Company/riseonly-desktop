use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Size, Styled, Window, div,
};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_media::video::av_player::AVLayerVideoGravity;
use rise_ui::swiftui::content_mode::ContentMode;

use crate::core::ui::animated_gif_view::{AnimatedGifContentMode, AnimatedGifView};
use crate::core::ui::animated_media_playback_coordinator::AnimatedMediaPlaybackKind;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::custom_emoji_glyph::CustomEmojiGlyphMetrics;
use crate::core::ui::lottie_sticker::lottie_sticker_view::LottieStickerView;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::bubble::message_sticker_view::{
    ChatAnimatedMediaPlaybackKind, ChatAnimatedMediaSourcePolicy, ChatLoopingVideoView, ChatWebMVideoStack,
};
use crate::modules::chat::components::chat::bubble::web_m_sticker_playback::WebMStickerSupport;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::StickerMediaDisplayURL;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    PackKind, StickerAssetType, StickerAssetTypeInfer,
};
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

pub enum InlineCustomEmojiVideoPlaybackPolicy {}

impl InlineCustomEmojiVideoPlaybackPolicy {
    pub fn playable_source(raw_source: &str) -> Option<String> {
        let source = trimming_whitespaces_and_newlines(raw_source);
        if !Self::is_supported_location(source) {
            return None;
        }
        matches!(
            ChatAnimatedMediaSourcePolicy::playback_kind(source, None, None),
            ChatAnimatedMediaPlaybackKind::AvFoundationVideo | ChatAnimatedMediaPlaybackKind::WebMVideo
        )
        .then(|| source.to_owned())
    }

    pub fn playable_kind(
        raw_source: &str,
        mime_type: Option<&str>,
        codec: Option<&str>,
    ) -> Option<(String, ChatAnimatedMediaPlaybackKind)> {
        let source = trimming_whitespaces_and_newlines(raw_source);
        if !Self::is_supported_location(source) {
            return None;
        }
        let kind = match ChatAnimatedMediaSourcePolicy::playback_kind(source, mime_type, codec) {
            ChatAnimatedMediaPlaybackKind::AvFoundationVideo if WebMStickerSupport::is_web_m(source) => {
                ChatAnimatedMediaPlaybackKind::WebMVideo
            }
            kind => kind,
        };
        matches!(
            kind,
            ChatAnimatedMediaPlaybackKind::AvFoundationVideo
                | ChatAnimatedMediaPlaybackKind::WebMVideo
                | ChatAnimatedMediaPlaybackKind::AnimatedImage
        )
        .then(|| (source.to_owned(), kind))
    }

    fn is_supported_location(source: &str) -> bool {
        if source.is_empty() {
            return false;
        }
        if is_local_absolute_path(source) {
            return true;
        }
        let Some(scheme) = Url::from_string(source).and_then(|url| url.scheme().map(str::to_lowercase)) else {
            return false;
        };
        scheme == "http" || scheme == "https" || scheme == "file"
    }
}

#[derive(IntoElement)]
pub struct InlineCustomEmojiRun {
    id: ElementId,
    pub custom_emoji_id: String,
    pub file_url: String,
    pub asset_type: String,
    pub size: f32,
    pub canvas_width: i64,
    pub canvas_height: i64,
    pub maximum_width: Option<f32>,
    pub maximum_aspect: f32,
    pub pack_id: String,
    pub opens_pack_on_tap: bool,
}

impl InlineCustomEmojiRun {
    pub fn new(
        id: impl Into<ElementId>,
        custom_emoji_id: impl Into<String>,
        file_url: impl Into<String>,
        asset_type: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            custom_emoji_id: custom_emoji_id.into(),
            file_url: file_url.into(),
            asset_type: asset_type.into(),
            size: 20.0,
            canvas_width: 0,
            canvas_height: 0,
            maximum_width: None,
            maximum_aspect: CustomEmojiGlyphMetrics::MAXIMUM_ASPECT,
            pack_id: String::new(),
            opens_pack_on_tap: false,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn canvas_width(mut self, canvas_width: i64) -> Self {
        self.canvas_width = canvas_width;
        self
    }

    pub fn canvas_height(mut self, canvas_height: i64) -> Self {
        self.canvas_height = canvas_height;
        self
    }

    pub fn maximum_width(mut self, maximum_width: Option<f32>) -> Self {
        self.maximum_width = maximum_width;
        self
    }

    pub fn maximum_aspect(mut self, maximum_aspect: f32) -> Self {
        self.maximum_aspect = maximum_aspect;
        self
    }

    pub fn pack_id(mut self, pack_id: impl Into<String>) -> Self {
        self.pack_id = pack_id.into();
        self
    }

    pub fn opens_pack_on_tap(mut self, opens_pack_on_tap: bool) -> Self {
        self.opens_pack_on_tap = opens_pack_on_tap;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn normalized_file_url(&self) -> String {
        RemoteMediaURL::optional_normalized(Some(self.file_url.as_str()))
            .unwrap_or_else(|| trimming_whitespaces_and_newlines(&self.file_url).to_owned())
    }

    fn resolved_asset(&self, normalized_file_url: &str) -> StickerAssetType {
        <StickerAssetType as StickerAssetTypeInfer>::infer(Some(self.asset_type.as_str()), normalized_file_url)
    }

    /// still frame of a video emoji from the item the sticker store resolved for the id
    fn video_poster(&self, ink: Size<Pixels>, cx: &mut App) -> Option<AnyElement> {
        let store = StickersServicesStore::shared(cx);
        let poster = store
            .read(cx)
            .custom_emoji_item(&self.custom_emoji_id, cx)
            .and_then(|item| item.thumbnail_url)
            .filter(|poster| !trimming_whitespaces_and_newlines(poster).is_empty())?;
        let decode_side = (f32::from(ink.width).max(f32::from(ink.height)) * 3.0).ceil() as i64;
        Some(
            CleverImageUi::new(StickerMediaDisplayURL::resolve(&poster))
                .id(self.child_id("video-poster"))
                .content_mode(ContentMode::Fit)
                .corner_radius(0.0)
                .persist_to_disk(true)
                .maximum_decode_pixel_dimension(decode_side.max(1))
                .into_any_element(),
        )
    }

    fn open_pack_sheet(custom_emoji_id: &str, pack_id: &str, cx: &mut App) {
        let explicit = trimming_whitespaces_and_newlines(pack_id);
        let resolved = if explicit.is_empty() {
            let stickers = StickersServicesStore::shared(cx);
            stickers.read(cx).custom_emoji_item(custom_emoji_id, cx).map(|item| item.pack_id).unwrap_or_default()
        } else {
            explicit.to_owned()
        };
        let pack = trimming_whitespaces_and_newlines(&resolved);
        if pack.is_empty() {
            return;
        }
        StickerPackSheetStore::shared(cx).update(cx, |store, cx| store.present(pack, PackKind::Emoji, cx));
    }
}

impl RenderOnce for InlineCustomEmojiRun {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let url = self.normalized_file_url();
        let resolved_asset = self.resolved_asset(&url);
        let ink = CustomEmojiGlyphMetrics::ink_size_capped(
            self.size,
            CustomEmojiGlyphMetrics::aspect(
                self.canvas_width,
                self.canvas_height,
                matches!(resolved_asset, StickerAssetType::Lottie),
            ),
            self.maximum_aspect,
            self.maximum_width,
        );
        let is_remote = url.to_lowercase().starts_with("http");
        let asset: Option<AnyElement> = match resolved_asset {
            StickerAssetType::Lottie => is_remote.then(|| {
                LottieStickerView::new(self.child_id("lottie"), url.clone())
                    .size(f32::from(ink.height))
                    .playback_id(Some(format!("ce-{}", self.custom_emoji_id)))
                    .playback_kind(AnimatedMediaPlaybackKind::AnimatedEmoji)
                    .into_any_element()
            }),
            StickerAssetType::Video => {
                let item = StickersServicesStore::shared(cx).read(cx).custom_emoji_item(&self.custom_emoji_id, cx);
                let (mime_type, codec) = item.map(|item| (item.mime_type, item.codec)).unwrap_or_default();
                match InlineCustomEmojiVideoPlaybackPolicy::playable_kind(&url, mime_type.as_deref(), codec.as_deref()) {
                    Some((source, ChatAnimatedMediaPlaybackKind::WebMVideo)) => {
                        let stack = ChatWebMVideoStack::new(self.child_id("video"), source);
                        let stack = match self.video_poster(ink, cx) {
                            Some(poster) => stack.poster(poster),
                            None => stack,
                        };
                        Some(stack.into_any_element())
                    }
                    Some((source, ChatAnimatedMediaPlaybackKind::AnimatedImage)) => Some(
                        div()
                            .relative()
                            .size_full()
                            .children(self.video_poster(ink, cx).map(|poster| div().absolute().inset_0().child(poster)))
                            .child(
                                div().absolute().inset_0().child(
                                    AnimatedGifView::new(self.child_id("gif"), source)
                                        .content_mode(AnimatedGifContentMode::ScaleAspectFit)
                                        .playback_kind(AnimatedMediaPlaybackKind::AnimatedEmoji),
                                ),
                            )
                            .into_any_element(),
                    ),
                    Some((source, _)) => Some(
                        div()
                            .relative()
                            .size_full()
                            .children(self.video_poster(ink, cx).map(|poster| div().absolute().inset_0().child(poster)))
                            .child(
                                div().absolute().inset_0().child(
                                    ChatLoopingVideoView::new(self.child_id("video"), source)
                                        .video_gravity(AVLayerVideoGravity::ResizeAspect),
                                ),
                            )
                            .into_any_element(),
                    ),
                    None => self.video_poster(ink, cx),
                }
            }
            StickerAssetType::StaticImage => is_remote.then(|| {
                let decode_side = (f32::from(ink.width).max(f32::from(ink.height)) * 3.0).ceil() as i64;
                CleverImageUi::new(url.clone())
                    .id(self.child_id("image"))
                    .content_mode(ContentMode::Fit)
                    .corner_radius(0.0)
                    .persist_to_disk(true)
                    .maximum_decode_pixel_dimension(decode_side.max(1))
                    .into_any_element()
            }),
        };
        let frame = div().w(ink.width).h(ink.height).children(asset);
        if !self.opens_pack_on_tap {
            return frame.into_any_element();
        }
        let tap_id = self.child_id("pack-tap");
        let custom_emoji_id = self.custom_emoji_id;
        let pack_id = self.pack_id;
        frame
            .bubble_tap_target(tap_id, move |_window: &mut Window, cx: &mut App| {
                Self::open_pack_sheet(&custom_emoji_id, &pack_id, cx);
            })
            .into_any_element()
    }
}
