use std::sync::atomic::{AtomicU32, Ordering};

use gpui::{Size, Window, size};
use rise_chat::ChatType;
use rise_theme::ThemeStore;
use rise_ui::message_bubble_geometry::MessageBubbleGeometry;

use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;

static DEVICE_SCALE: AtomicU32 = AtomicU32::new(0x3F80_0000);

pub enum MessageSizes {}

impl MessageSizes {
    /// one point of house tuning plus whatever the text size slider adds
    pub fn text_delta() -> f32 {
        1.0 + ThemeStore::text_delta_snapshot()
    }

    pub const BUBBLE_DELTA: f32 = 0.0;

    pub const TEXT_FACTOR: f32 = 1.0;
    pub const BUBBLE_FACTOR: f32 = 1.0;

    pub fn text(base: f32) -> f32 {
        ((base + Self::text_delta()) * Self::TEXT_FACTOR).round()
    }

    pub fn bubble(base: f32) -> f32 {
        ((base + Self::BUBBLE_DELTA) * Self::BUBBLE_FACTOR).round()
    }

    pub fn text_font_size() -> f32 {
        Self::text(16.0)
    }

    pub fn text_line_spacing() -> f32 {
        Self::text(2.0)
    }

    pub fn text_subtitle_font_size() -> f32 {
        Self::text(12.0)
    }

    pub fn text_meta_flow_spacing() -> f32 {
        Self::text(8.0)
    }

    pub fn sender_name_font_size() -> f32 {
        Self::text(13.0)
    }

    pub fn bubble_max_width() -> f32 {
        Self::bubble(345.0)
    }

    pub fn bubble_minimum_width() -> f32 {
        Self::bubble(40.0)
    }

    pub fn bubble_inner_horizontal_padding() -> f32 {
        Self::bubble(11.0)
    }

    pub fn bubble_inner_top_padding() -> f32 {
        Self::bubble(6.0) + Self::device_pixel()
    }

    pub fn bubble_inner_bottom_padding() -> f32 {
        (Self::bubble(6.0) - Self::device_pixel()).max(0.0)
    }

    pub fn bubble_inner_vertical_padding() -> f32 {
        Self::bubble(6.0)
    }

    pub fn bubble_corner_radius() -> f32 {
        Self::bubble(16.0)
    }

    pub fn bubble_stacked_corner_radius() -> f32 {
        Self::bubble(8.0)
    }

    pub fn bubble_tail_inset() -> f32 {
        f32::from(MessageBubbleGeometry::TAIL_INSET)
    }

    pub fn media_max_width() -> f32 {
        Self::bubble(300.0)
    }

    pub fn media_max_height() -> f32 {
        Self::bubble(380.0)
    }

    pub fn media_minimum_width() -> f32 {
        Self::bubble(170.0)
    }

    pub fn media_minimum_height() -> f32 {
        Self::bubble(74.0)
    }

    pub fn media_inset() -> f32 {
        Self::bubble(2.0)
    }

    pub fn media_mosaic_separator_width() -> f32 {
        Self::device_pixel()
    }

    pub fn bubble_column_spacing() -> f32 {
        Self::bubble(4.0)
    }

    pub fn horizontal_bubble_padding(viewport_width: f32) -> f32 {
        Self::bubble(if viewport_width > 680.0 { 10.0 } else { 11.0 })
    }

    pub fn media_maximum_size(viewport_width: f32) -> Size<f32> {
        if viewport_width > 680.0 {
            size(Self::bubble(440.0), Self::bubble(440.0))
        } else {
            size(Self::media_max_width(), Self::media_max_height())
        }
    }

    pub fn list_row_spacing() -> f32 {
        0.0
    }

    pub fn list_first_in_group_top_padding() -> f32 {
        2.0 + Self::device_pixel()
    }

    pub fn list_continuation_top_padding() -> f32 {
        2.0
    }

    pub fn list_last_in_group_bottom_padding() -> f32 {
        2.0 + Self::device_pixel()
    }

    /// rows keep the avatar edge distance on both sides
    pub fn row_side_inset() -> f32 {
        Self::avatar_leading_inset()
    }

    pub fn avatar_size() -> f32 {
        Self::bubble(34.0)
    }

    pub fn avatar_gap() -> f32 {
        Self::bubble(4.0)
    }

    /// sender avatar keeps a fixed distance from the window edge
    pub fn avatar_leading_inset() -> f32 {
        7.0
    }

    /// own bubbles keep the same distance from the right edge as the avatar from the left
    pub fn own_trailing_inset() -> f32 {
        Self::row_side_inset().max(Self::avatar_leading_inset())
    }

    /// what an own row adds after row_side_inset on the trailing side
    pub fn own_trailing_extra() -> f32 {
        Self::own_trailing_inset() - Self::row_side_inset()
    }

    /// what an incoming row adds after row_side_inset before its bubble: avatar, gap and extra
    /// edge room
    pub fn avatar_column_width() -> f32 {
        (Self::avatar_leading_inset() - Self::row_side_inset()).max(0.0) + Self::avatar_size() + Self::avatar_gap()
    }

    pub fn opposite_side_min_width() -> f32 {
        Self::bubble(36.0)
    }

    pub fn row_gap() -> f32 {
        Self::bubble(3.0)
    }

    pub fn min_message_row_height() -> f32 {
        Self::bubble(35.0)
    }

    /// favourites reserves avatar space like a group does
    /// it holds saved messages from different authors so not own only means one of them
    pub fn reserves_avatar_space(chat: &ChatInfo, is_own: bool) -> bool {
        (chat.r#type == ChatType::Group || chat.r#type == ChatType::Favourites) && !is_own
    }

    /// viewport_width is the chat column not the window
    /// beside the chat list the column is what the bubble fills
    pub fn available_bubble_width(viewport_width: f32, chat: &ChatInfo, is_own: bool) -> f32 {
        let finite_width = if viewport_width.is_finite() { viewport_width.max(1.0) } else { 1.0 };
        let maximum_fill = if finite_width <= 500.0 {
            (finite_width - 36.0).max(1.0)
        } else {
            (finite_width * if finite_width > 680.0 { 0.65 } else { 0.85 }).floor()
        };
        let avatar_width = if Self::reserves_avatar_space(chat, is_own) {
            Self::avatar_column_width()
        } else if is_own {
            Self::own_trailing_extra()
        } else {
            0.0
        };
        Self::bubble_minimum_width().max((maximum_fill - Self::row_side_inset() * 3.0 - avatar_width).floor())
    }

    pub fn meta_font_size() -> f32 {
        (11.0 * Self::TEXT_FACTOR).round()
    }

    pub fn meta_icon_font_size() -> f32 {
        (11.0 * Self::TEXT_FACTOR).round()
    }

    pub fn meta_content_spacing() -> f32 {
        Self::bubble(2.0)
    }

    pub fn meta_overlay_horizontal_padding() -> f32 {
        Self::bubble(7.0)
    }

    pub fn meta_overlay_vertical_padding() -> f32 {
        Self::bubble(2.0)
    }

    pub fn reaction_emoji_font_size() -> f32 {
        Self::text(19.0)
    }

    pub fn reaction_count_font_size() -> f32 {
        (11.0 * Self::TEXT_FACTOR).round()
    }

    pub fn reaction_pill_horizontal_padding() -> f32 {
        Self::bubble(11.0)
    }

    pub fn reaction_pill_vertical_padding() -> f32 {
        Self::bubble(4.0)
    }

    pub fn reaction_pill_content_spacing() -> f32 {
        Self::bubble(2.0)
    }

    pub fn reaction_avatar_size() -> f32 {
        Self::bubble(20.0)
    }

    pub fn reaction_avatar_overlap() -> f32 {
        Self::bubble(7.0)
    }

    pub fn system_pill_font_size() -> f32 {
        Self::text(12.0)
    }

    pub fn link_preview_site_font_size() -> f32 {
        Self::text(12.0)
    }

    pub fn link_preview_title_font_size() -> f32 {
        Self::text(14.0)
    }

    pub fn link_preview_description_small_font_size() -> f32 {
        Self::text(12.0)
    }

    pub fn link_preview_description_large_font_size() -> f32 {
        Self::text(13.0)
    }

    pub fn composer_text_font_size() -> f32 {
        Self::text(15.0)
    }

    pub fn device_pixel() -> f32 {
        1.0 / f32::from_bits(DEVICE_SCALE.load(Ordering::Relaxed)).max(1.0)
    }

    pub fn sync_device_scale(window: &Window) {
        let scale = window.scale_factor();
        if scale.is_finite() && scale > 0.0 {
            DEVICE_SCALE.store(scale.to_bits(), Ordering::Relaxed);
        }
    }
}

pub enum MessageAccessoryStackGeometry {}

impl MessageAccessoryStackGeometry {
    pub const INLINE_KEYBOARD_SPACING: f32 = 4.0;
    pub const INLINE_KEYBOARD_BOTTOM_RESERVE: f32 = 1.0;

    pub fn embedded_reaction_top_spacing() -> f32 {
        MessageSizes::bubble_column_spacing()
    }

    pub fn additional_height(inline_keyboard_height: Option<f32>, embedded_reaction_height: Option<f32>) -> f32 {
        let mut result = 0.0;
        if let Some(inline_keyboard_height) = inline_keyboard_height {
            result += Self::INLINE_KEYBOARD_SPACING + inline_keyboard_height.max(0.0) + Self::INLINE_KEYBOARD_BOTTOM_RESERVE;
        }
        if let Some(embedded_reaction_height) = embedded_reaction_height {
            result += Self::embedded_reaction_top_spacing() + embedded_reaction_height.max(0.0);
        }
        result
    }
}
