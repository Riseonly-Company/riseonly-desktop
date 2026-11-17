use gpui::App;

use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{
    ChatMessageRenderEntry, ChatMessageRenderEntryKind,
};
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::bubble::message_call_view::MessageCallBubbleGeometry;
use crate::modules::chat::components::chat::bubble::message_content::{MessageContent, MessageContentKind};
use crate::modules::chat::components::chat::bubble::message_image_grid_layout::MessageGridLayoutEngine;
use crate::modules::chat::components::chat::bubble::message_sticker_view::MessageStickerLayout;
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::shared::chat_saved_message_presentation::ChatSavedMessagePresentation;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;

pub enum RiseExactFallbackGeometry {}

impl RiseExactFallbackGeometry {
    const STICKER_FALLBACK_HEIGHT: f32 = 184.0;

    pub fn row_height(
        entry: &ChatMessageRenderEntry,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        cx: &mut App,
    ) -> Option<f32> {
        if entry.kind != ChatMessageRenderEntryKind::Message {
            return None;
        }
        let message = entry.message.as_ref()?;
        let group = entry.group.as_ref()?;
        if message.via_bot_label_tag().is_some() {
            return None;
        }
        let chat = &configuration.chat;

        let top = if group.is_first {
            MessageSizes::list_first_in_group_top_padding()
        } else {
            MessageSizes::list_continuation_top_padding()
        };
        let bottom = if group.is_last { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 };

        let kind = MessageContent::bubble_kind(message, Some(chat));

        if kind == MessageContentKind::Call {
            return Some(MessageCallBubbleGeometry::row_height(group.is_first, group.is_last));
        }

        let sticker_has_context_header = kind == MessageContentKind::Sticker
            && (message.reply_to.is_some()
                || message.story_reply.is_some()
                || (message.forward_info.is_some()
                    && !ChatSavedMessagePresentation::hides_forward_header(message, chat, cx)));
        if sticker_has_context_header || message.is_disabled_by_moderation() {
            return None;
        }

        let has_inline_keyboard = message.reply_markup.as_ref().is_some_and(|markup| markup.has_inline_keyboard());
        let has_discussion = message.discussion_chat_id.as_ref().is_some_and(|id| !id.is_empty());

        if kind == MessageContentKind::Sticker && !message.has_reaction_row() && !has_inline_keyboard && !has_discussion {
            let sticker_height = message
                .first_sticker_item()
                .map_or(Self::STICKER_FALLBACK_HEIGHT, |item| f32::from(MessageStickerLayout::display_size(item).height));
            return Some((top + sticker_height + MessageStickerLayout::META_OVERHANG + bottom).ceil());
        }

        let grid_media_items = message.grid_media_items();
        if kind != MessageContentKind::MediaGrid
            || grid_media_items.is_empty()
            || message.has_text()
            || message.has_reaction_row()
            || has_inline_keyboard
            || has_discussion
        {
            return None;
        }

        let shows_forward_header =
            message.forward_info.is_some() && !ChatSavedMessagePresentation::hides_forward_header(message, chat, cx);
        if shows_forward_header || message.reply_to.is_some() || message.story_reply.is_some() {
            return None;
        }

        let grid_items: Vec<_> = grid_media_items.into_iter().filter(|item| !item.is_cancelled).cloned().collect();
        let media_inset = MessageSizes::media_inset();
        let is_own = MessageList::is_own_bubble(message, chat, cx);
        let bubble_width = MessageSizes::available_bubble_width(viewport_width, chat, is_own);
        let maximum_media_size = MessageSizes::media_maximum_size(viewport_width);
        let layout = MessageGridLayoutEngine::compute(
            &grid_items,
            maximum_media_size
                .width
                .min(bubble_width - MessageSizes::bubble_tail_inset() - media_inset * 2.0)
                .max(1.0),
            maximum_media_size.height,
            MessageSizes::media_mosaic_separator_width(),
            None,
        );
        let bubble_height = f32::from(layout.container_size.height) + media_inset * 2.0;
        let row_body_height = if MessageSizes::reserves_avatar_space(chat, is_own) {
            MessageSizes::avatar_size().max(bubble_height)
        } else {
            bubble_height
        };
        Some((top + row_body_height + bottom).ceil())
    }
}
