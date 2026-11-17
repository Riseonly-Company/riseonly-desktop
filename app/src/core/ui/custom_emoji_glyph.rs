use gpui::{App, IntoElement, ParentElement, Pixels, RenderOnce, Size, Styled, Window, div, px, size};

use crate::core::ui::animated_media_playback_coordinator::AnimatedMediaPlaybackKind;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::ChatInputStickerAsset;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerAssetType, StickerItem};

pub enum CustomEmojiGlyphMetrics {}

impl CustomEmojiGlyphMetrics {
    pub const UNICODE_EMOJI_INK_RATIO: f32 = 1.16;

    pub const PACK_INK_BOOST: f32 = 1.22;

    pub const MAXIMUM_ASPECT: f32 = 1.6;

    pub const TEXT_MAXIMUM_ASPECT: f32 = 4.0;

    /// height of a glyph standing in for a unicode emoji at font_size
    /// rounded here and not at the call site
    /// size is a raster cache key and a fractional height rasterizes per caller
    pub fn ink_height(font_size: f32) -> f32 {
        (font_size * Self::UNICODE_EMOJI_INK_RATIO * Self::PACK_INK_BOOST).round()
    }

    /// glyph width over height, zero in both fields is normal since the wire may not carry them
    /// and a lottie raster is square anyway
    pub fn aspect(width: i64, height: i64, is_lottie: bool) -> f32 {
        if is_lottie || width <= 0 || height <= 0 {
            return 1.0;
        }
        width as f32 / height as f32
    }

    /// glyph frame, height is given and width follows the aspect
    /// maximum_width is passed where the glyph has a slot with other content beside it like the count
    /// in a reaction pill there a panoramic glyph shrinks in height instead of overflowing
    pub fn ink_size(height: f32, raw_aspect: f32, maximum_width: Option<f32>) -> Size<Pixels> {
        Self::ink_size_capped(height, raw_aspect, Self::MAXIMUM_ASPECT, maximum_width)
    }

    /// ink_size with an explicit aspect cap, TEXT_MAXIMUM_ASPECT for glyphs inside a text line
    pub fn ink_size_capped(
        height: f32,
        raw_aspect: f32,
        maximum_aspect: f32,
        maximum_width: Option<f32>,
    ) -> Size<Pixels> {
        let aspect = raw_aspect.max(1.0).min(maximum_aspect.max(1.0));
        let mut ink_height = height.round();
        let mut width = (ink_height * aspect).round();
        if let Some(maximum_width) = maximum_width
            && width > maximum_width
        {
            width = maximum_width.round();
            ink_height = (width / aspect).round();
        }
        size(px(width), px(ink_height))
    }
}

/// custom emoji as a glyph not a picker cell, the sticker cell observes download state
/// so every glyph redraws on unrelated progress and it has a cancel overlay
/// asset fits a fixed square so pass height times aspect as the side
#[derive(IntoElement)]
pub struct CustomEmojiGlyph {
    pub item: StickerItem,
    pub height: f32,
    pub maximum_aspect: f32,
    pub is_playing: bool,
    pub loops: bool,
}

impl CustomEmojiGlyph {
    pub fn new(item: StickerItem, height: f32) -> Self {
        Self {
            item,
            height,
            maximum_aspect: CustomEmojiGlyphMetrics::MAXIMUM_ASPECT,
            is_playing: true,
            loops: true,
        }
    }

    pub fn maximum_aspect(mut self, maximum_aspect: f32) -> Self {
        self.maximum_aspect = maximum_aspect;
        self
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn loops(mut self, loops: bool) -> Self {
        self.loops = loops;
        self
    }
}

impl RenderOnce for CustomEmojiGlyph {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let aspect = CustomEmojiGlyphMetrics::aspect(
            self.item.width,
            self.item.height,
            self.item.asset_type == StickerAssetType::Lottie,
        )
        .min(self.maximum_aspect);
        let ink = CustomEmojiGlyphMetrics::ink_size(self.height, aspect, None);
        let ink_height = ink.height;
        let side = ink.width;

        div()
            .w(side)
            .h(ink_height)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .child(
                ChatInputStickerAsset::new(self.item, f32::from(side))
                    .is_playing(self.is_playing)
                    .loops(self.loops)
                    .shows_poster_until_first_frame(true)
                    .playback_kind(AnimatedMediaPlaybackKind::AnimatedEmoji),
            )
    }
}
