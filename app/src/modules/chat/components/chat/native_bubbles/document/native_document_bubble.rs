use std::rc::Rc;

use gpui::{AnyView, App, Window, px};
use rise_chat::ChatType;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::AppTheme;
use rise_ui::swiftui::font::NATURAL_LINE_HEIGHT;

use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::bubble::message_content::{MessageContent, MessageContentKind};
use crate::modules::chat::components::chat::bubble::message_meta::MessageDeliveryStatusGeometry;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::native_bubbles::native_bubble_registry::{NativeBubbleKind, NativeBubbleRegistry};
use crate::modules::chat::components::chat::native_bubbles::sticker::native_interactive_message_cell::{
    NativeInteractiveMessageCell, NativeMessageMetaView, native_bubble_pixel_ceil,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::utils::chat_document_open::ChatDocumentOpen;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageMediaItem};

/// file bubble priced without the hosted measure
/// fill and coloured extension circle and name and size and the inline time row
pub enum NativeDocumentBubble {}

impl NativeBubbleKind for NativeDocumentBubble {
    const REUSE_IDENTIFIER: &'static str = "NativeDocumentBubble";

    fn row_height(
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &RiseMessageCollectionConfiguration,
        _viewport_width: f32,
        cx: &mut App,
    ) -> Option<f32> {
        if !Self::supports(message, &configuration.chat, group, cx) {
            return None;
        }
        let is_own = MessageList::is_own_bubble(message, &configuration.chat, cx);
        let (top, bottom) = NativeBubbleRegistry::row_paddings(group);
        let bubble = NativeDocumentLayout::bubble_height(message, is_own);
        let body = if MessageSizes::reserves_avatar_space(&configuration.chat, is_own) {
            MessageSizes::avatar_size().max(bubble)
        } else {
            bubble
        };
        Some(native_bubble_pixel_ceil(top + body + bottom))
    }

    fn make_view(
        reusing: Option<AnyView>,
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        viewport_width: f32,
        _theme: &AppTheme,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyView {
        let height = Self::row_height(message, group, configuration, viewport_width, cx);
        NativeInteractiveMessageCell::make_view(reusing, message, group, configuration, viewport_width, height, cx)
    }
}

impl NativeDocumentBubble {
    /// true for one local file not uploading with no caption or previews
    /// no reply or story or forward plate, no reactions or comments or via bot
    /// remote files and group sender header stay regular
    pub fn supports(
        message: &GetMessageMessage,
        chat: &ChatInfo,
        group: ChatMessageRenderEntryGroupFlags,
        cx: &mut App,
    ) -> bool {
        let is_own = MessageList::is_own_bubble(message, chat, cx);
        let chat_takes = chat.r#type == ChatType::Direct
            || (chat.r#type == ChatType::Group && (is_own || !group.is_first));
        if !chat_takes
            || MessageContent::bubble_kind(message, Some(chat)) != MessageContentKind::Document
            || message.media_items.len() != 1
        {
            return false;
        }
        let documents = message.document_items();
        let [item] = documents.as_slice() else {
            return false;
        };
        message.text.is_empty()
            && message.link_preview.is_none()
            && message.reply_to.is_none()
            && message.story_reply.is_none()
            && message.forward_info.is_none()
            && message.via_bot_label_tag().is_none()
            && !message.has_reaction_row()
            && !message.reply_markup.as_ref().is_some_and(|markup| markup.has_inline_keyboard())
            && message.discussion_chat_id.as_deref().is_none_or(str::is_empty)
            && !message.is_disabled_by_moderation()
            && !item.is_uploading
            && !item.is_compressing
            && !item.is_cancelled
            && ChatDocumentOpen::transfer_request(item).is_none()
            && NativeDocumentLayout::preview_thumbnail(item).is_none()
    }
}

/// geometry of the document bubble, 44 pt circle 3 pt in with 8 pt gaps
/// column spacing 6, name and size 2 pt apart
pub enum NativeDocumentLayout {}

impl NativeDocumentLayout {
    pub const INSET_TOP: f32 = 15.0;
    pub const INSET_LEADING: f32 = 9.0;
    pub const INSET_BOTTOM: f32 = 15.0;
    pub const INSET_TRAILING: f32 = 12.0;
    pub const ICON_SIDE: f32 = 44.0;
    pub const ICON_LEADING_INSET: f32 = 3.0;
    pub const ROW_SPACING: f32 = 8.0;
    pub const TEXT_SPACING: f32 = 2.0;
    pub const COLUMN_SPACING: f32 = 6.0;
    pub const MAXIMUM_DOCUMENT_WIDTH: f32 = 280.0;

    pub fn bubble_height(message: &GetMessageMessage, is_own: bool) -> f32 {
        let height = Self::INSET_TOP
            + Self::ICON_SIDE
            + Self::COLUMN_SPACING
            + Self::meta_height(message, is_own)
            + Self::INSET_BOTTOM;
        MessageSizes::min_message_row_height().max(height)
    }

    /// height of the meta row
    /// time text, pin glyph and status slot
    pub fn meta_height(message: &GetMessageMessage, is_own: bool) -> f32 {
        let icon_size = MessageSizes::meta_icon_font_size();
        let mut height = native_bubble_pixel_ceil(NativeMessageMetaView::time_font_size() * NATURAL_LINE_HEIGHT);
        if message.is_pinned {
            height = height.max(icon_size);
        }
        if is_own {
            let state = NativeMessageMetaView::delivery_state(message);
            height = height.max(f32::from(MessageDeliveryStatusGeometry::slot_size(state, px(icon_size)).height));
        }
        height
    }

    pub fn preview_thumbnail(item: &MessageMediaItem) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(item.thumbnail_url.as_deref()?);
        if value.is_empty() || !MessageMediaItem::is_decodable_image_source(value) {
            return None;
        }
        Some(value.to_owned())
    }
}
