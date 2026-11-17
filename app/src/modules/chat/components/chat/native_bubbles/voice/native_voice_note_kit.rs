use gpui::px;
use rise_i18n::tr;
use rise_theme::ThemeStore;
use rise_ui::swiftui::font::NATURAL_LINE_HEIGHT;

use crate::modules::chat::components::chat::async_render::async_text_bubble_types::AsyncTextBubbleDeliveryState;
use crate::modules::chat::components::chat::bubble::message_meta::MessageDeliveryStatusGeometry;
use crate::modules::chat::components::chat::native_bubbles::sticker::native_interactive_message_cell::{
    NativeMessageMetaView, native_bubble_pixel_ceil,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

/// shared measurements of voice and video note rows
/// every size repeats what the regular bubble lays out so rows keep the measured height
pub enum NativeVoiceNoteMetrics {}

impl NativeVoiceNoteMetrics {
    /// hosted rows are rounded up to the pixel grid
    pub fn ceil_pixel(value: f32) -> f32 {
        native_bubble_pixel_ceil(value)
    }

    pub fn round_pixel(value: f32) -> f32 {
        value.round()
    }

    /// main text adds the text size slider to its size, plain text doesnt
    pub fn main_text_font_size(px: f32, text_delta: f32) -> f32 {
        (px + text_delta).max(1.0)
    }

    /// line height of one text line at font_size, on the pixel grid
    pub fn text_height(font_size: f32) -> f32 {
        Self::ceil_pixel(font_size * NATURAL_LINE_HEIGHT)
    }

    /// side of the pin glyph in the meta line
    pub fn pin_size() -> f32 {
        MessageSizes::meta_icon_font_size()
    }

    /// delivery state the regular meta derives from the message
    pub fn delivery_state(message: &GetMessageMessage) -> AsyncTextBubbleDeliveryState {
        NativeMessageMetaView::delivery_state(message)
    }

    pub fn status_accessibility_text(state: AsyncTextBubbleDeliveryState) -> Option<String> {
        match state {
            AsyncTextBubbleDeliveryState::None => None,
            AsyncTextBubbleDeliveryState::Sending => Some(tr("chat_message_status_sending")),
            AsyncTextBubbleDeliveryState::Sent => Some(tr("chat_message_status_sent")),
            AsyncTextBubbleDeliveryState::Delivered => Some(tr("chat_message_status_delivered")),
            AsyncTextBubbleDeliveryState::Read => Some(tr("chat_message_status_read")),
            AsyncTextBubbleDeliveryState::Failed => Some(tr("chat_message_status_failed")),
        }
    }

    /// text size slider the rows price with
    pub fn text_delta() -> f32 {
        ThemeStore::text_delta_snapshot()
    }
}

/// parts of the meta a voice or video note row shows
/// edited mark then time then pin then delivery glyph
/// 2 pt apart on one line
pub enum NativeMessageMetaContent {}

impl NativeMessageMetaContent {
    /// height of the meta line without the plate
    pub fn line_height(text_delta: f32, pinned: bool, has_status: bool) -> f32 {
        let font_size = NativeVoiceNoteMetrics::main_text_font_size(MessageSizes::meta_font_size(), text_delta);
        let mut height = NativeVoiceNoteMetrics::text_height(font_size);
        if pinned {
            height = height.max(NativeVoiceNoteMetrics::pin_size());
        }
        if has_status {
            let slot = MessageDeliveryStatusGeometry::slot_size(
                AsyncTextBubbleDeliveryState::Sent,
                px(MessageSizes::meta_icon_font_size()),
            );
            height = height.max(f32::from(slot.height));
        }
        height
    }

    pub fn overlay_height(text_delta: f32, pinned: bool, has_status: bool) -> f32 {
        Self::line_height(text_delta, pinned, has_status) + MessageSizes::meta_overlay_vertical_padding() * 2.0
    }
}
