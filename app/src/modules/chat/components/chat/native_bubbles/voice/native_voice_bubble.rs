use std::rc::Rc;

use gpui::{AnyView, App, Window};
use rise_chat::ChatType;
use rise_theme::AppTheme;

use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::bubble::message_content::{MessageContent, MessageContentKind};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::native_bubbles::native_bubble_registry::{NativeBubbleKind, NativeBubbleRegistry};
use crate::modules::chat::components::chat::native_bubbles::voice::native_voice_note_interaction_cell::NativeVoiceNoteInteractionCell;
use crate::modules::chat::components::chat::native_bubbles::voice::native_voice_note_kit::{
    NativeMessageMetaContent, NativeVoiceNoteMetrics,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageMediaItem};

/// voice message priced without the hosted measure
/// bubble with voice and the inline meta
pub enum NativeVoiceBubble {}

impl NativeBubbleKind for NativeVoiceBubble {
    const REUSE_IDENTIFIER: &'static str = "NativeVoiceMessageCell";

    fn row_height(
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        cx: &mut App,
    ) -> Option<f32> {
        let item = NativeVoiceEligibility::voice_item(message, &configuration.chat, cx)?;
        let is_own = MessageList::is_own_bubble(message, &configuration.chat, cx);
        Some(
            NativeVoiceGeometry::new(
                message,
                item,
                group,
                &configuration.chat,
                is_own,
                viewport_width,
                NativeVoiceNoteMetrics::text_delta(),
            )
            .row_height,
        )
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
        NativeVoiceNoteInteractionCell::make_view(reusing, message, group, configuration, viewport_width, height, cx)
    }
}

pub enum NativeVoiceEligibility {}

impl NativeVoiceEligibility {
    pub fn voice_item<'a>(message: &'a GetMessageMessage, chat: &ChatInfo, cx: &mut App) -> Option<&'a MessageMediaItem> {
        if MessageContent::bubble_kind(message, Some(chat)) != MessageContentKind::Voice {
            return None;
        }
        let item = message.first_voice_item()?;
        if item.is_uploading
            || item.is_compressing
            || item.is_cancelled
            || item.has_spoiler
            || item.view_limit.is_some()
            || !NativeVoiceNoteEligibility::supports_message(message, chat, cx)
        {
            return None;
        }
        Some(item)
    }
}

pub enum NativeVoiceNoteEligibility {}

impl NativeVoiceNoteEligibility {
    pub fn supports_message(message: &GetMessageMessage, chat: &ChatInfo, cx: &mut App) -> bool {
        if message.has_reaction_row()
            || message.reply_to.is_some()
            || message.forward_info.is_some()
            || message.story_reply.is_some()
            || message.via_bot_label_tag().is_some()
            || message.link_preview.is_some()
            || message.reply_markup.as_ref().is_some_and(|markup| markup.has_inline_keyboard())
            || !message.discussion_chat_id.as_deref().is_none_or(str::is_empty)
            || message.is_disabled_by_moderation()
        {
            return false;
        }
        let is_own = MessageList::is_own_bubble(message, chat, cx);
        chat.r#type == ChatType::Direct || (chat.r#type == ChatType::Group && is_own)
    }
}

pub struct NativeVoiceGeometry {
    pub is_own: bool,
    pub reserves_avatar_space: bool,
    pub voice_width: f32,
    pub meta_height: f32,
    pub bubble_width: f32,
    pub bubble_height: f32,
    pub top_padding: f32,
    pub row_height: f32,
}

impl NativeVoiceGeometry {
    pub const INSET_TOP: f32 = 15.0;
    pub const INSET_LEFT: f32 = 9.0;
    pub const INSET_BOTTOM: f32 = 15.0;
    pub const INSET_RIGHT: f32 = 12.0;
    pub const VOICE_HEIGHT: f32 = 38.0;
    pub const COLUMN_SPACING: f32 = 6.0;

    pub fn new(
        message: &GetMessageMessage,
        item: &MessageMediaItem,
        group: ChatMessageRenderEntryGroupFlags,
        chat: &ChatInfo,
        is_own: bool,
        viewport_width: f32,
        text_delta: f32,
    ) -> Self {
        let reserves_avatar_space = MessageSizes::reserves_avatar_space(chat, is_own);
        let voice_width = Self::voice_width(item, viewport_width, chat, is_own);
        let meta_height = NativeMessageMetaContent::line_height(text_delta, message.is_pinned, is_own);
        let content_height = Self::VOICE_HEIGHT + Self::COLUMN_SPACING + meta_height;
        let bubble_width = MessageSizes::bubble_minimum_width()
            .max(voice_width + Self::INSET_LEFT + Self::INSET_RIGHT + MessageSizes::bubble_tail_inset());
        let bubble_height =
            MessageSizes::min_message_row_height().max(content_height + Self::INSET_TOP + Self::INSET_BOTTOM);
        let (top, bottom) = NativeBubbleRegistry::row_paddings(group);
        let body = if reserves_avatar_space { MessageSizes::avatar_size().max(bubble_height) } else { bubble_height };
        Self {
            is_own,
            reserves_avatar_space,
            voice_width,
            meta_height,
            bubble_width,
            bubble_height,
            top_padding: top,
            row_height: NativeVoiceNoteMetrics::ceil_pixel(top + body + bottom),
        }
    }

    /// voice width given the viewport width the interactive file content width hands it
    pub fn voice_width(item: &MessageMediaItem, viewport_width: f32, chat: &ChatInfo, is_own: bool) -> f32 {
        let constrained = (MessageSizes::available_bubble_width(viewport_width, chat, is_own)
            - MessageSizes::bubble_tail_inset()
            - Self::INSET_LEFT
            - Self::INSET_RIGHT)
            .max(1.0);
        let min_width = 120.0_f32;
        let max_width = min_width.max(constrained - 36.0);
        let duration = item.duration.unwrap_or(0.0) as f32;
        let normalized = (duration.clamp(2.0, 30.0) - 2.0) / 28.0;
        (min_width + (max_width - min_width) * normalized).floor()
    }
}
