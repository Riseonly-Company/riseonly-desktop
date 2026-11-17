use std::rc::Rc;

use gpui::{AnyView, App, Window};
use rise_theme::AppTheme;

use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::bubble::message_content::{MessageContent, MessageContentKind};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::native_bubbles::native_bubble_registry::{NativeBubbleKind, NativeBubbleRegistry};
use crate::modules::chat::components::chat::native_bubbles::voice::native_voice_bubble::NativeVoiceNoteEligibility;
use crate::modules::chat::components::chat::native_bubbles::voice::native_voice_note_interaction_cell::NativeVoiceNoteInteractionCell;
use crate::modules::chat::components::chat::native_bubbles::voice::native_voice_note_kit::{
    NativeMessageMetaContent, NativeVoiceNoteMetrics,
};
use crate::modules::chat::components::chat::video_note::video_note_message_view::VideoNoteMessageView;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageMediaItem, MessageMediaKind,
};

/// video note at rest priced without the hosted measure
/// while it plays the row goes back to the regular measure and returns here when playback ends
pub enum NativeVideoNoteBubble {}

impl NativeBubbleKind for NativeVideoNoteBubble {
    const REUSE_IDENTIFIER: &'static str = "NativeVideoNoteMessageCell";

    fn row_height(
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        cx: &mut App,
    ) -> Option<f32> {
        NativeVideoNoteEligibility::note_item(message, &configuration.chat, cx)?;
        let is_own = MessageList::is_own_bubble(message, &configuration.chat, cx);
        Some(
            NativeVideoNoteGeometry::new(
                message,
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

pub enum NativeVideoNoteEligibility {}

impl NativeVideoNoteEligibility {
    pub fn note_item<'a>(message: &'a GetMessageMessage, chat: &ChatInfo, cx: &mut App) -> Option<&'a MessageMediaItem> {
        if MessageContent::bubble_kind(message, Some(chat)) != MessageContentKind::VideoNote {
            return None;
        }
        let item = message.media_items.iter().find(|item| item.kind == MessageMediaKind::VideoNote)?;
        if item.is_uploading
            || item.is_compressing
            || item.is_cancelled
            || item.has_spoiler
            || item.view_limit.is_some()
            || !NativeVoiceNoteEligibility::supports_message(message, chat, cx)
            || Self::is_playing(&message.id, cx)
        {
            return None;
        }
        Some(item)
    }

    /// true while this video note is playing
    pub fn is_playing(message_id: &str, cx: &mut App) -> bool {
        let entity = VideoNotePlayerStore::shared(cx);
        let store = entity.read(cx);
        store.is_active() && store.message_id() == message_id
    }
}

pub struct NativeVideoNoteGeometry {
    pub is_own: bool,
    pub reserves_avatar_space: bool,
    pub circle_size: f32,
    pub meta_row_height: f32,
    pub column_width: f32,
    pub column_height: f32,
    pub top_padding: f32,
    pub row_height: f32,
}

impl NativeVideoNoteGeometry {
    pub const COLUMN_SPACING: f32 = 6.0;

    pub fn new(
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        chat: &ChatInfo,
        is_own: bool,
        viewport_width: f32,
        text_delta: f32,
    ) -> Self {
        let reserves = MessageSizes::reserves_avatar_space(chat, is_own);
        let circle_size = VideoNoteMessageView::expanded_size(viewport_width, reserves);
        let duration_font = NativeVoiceNoteMetrics::main_text_font_size(MessageSizes::meta_font_size(), text_delta);
        let duration_plate = NativeVoiceNoteMetrics::text_height(duration_font).max(6.0)
            + MessageSizes::meta_overlay_vertical_padding() * 2.0;
        let meta_plate = NativeMessageMetaContent::overlay_height(text_delta, message.is_pinned, is_own);
        let meta_row_height = duration_plate.max(meta_plate);
        let column_height = circle_size + Self::COLUMN_SPACING + meta_row_height;
        let (top, bottom) = NativeBubbleRegistry::row_paddings(group);
        let body = if reserves { MessageSizes::avatar_size().max(column_height) } else { column_height };
        Self {
            is_own,
            reserves_avatar_space: reserves,
            circle_size,
            meta_row_height,
            column_width: circle_size,
            column_height,
            top_padding: top,
            row_height: NativeVoiceNoteMetrics::ceil_pixel(top + body + bottom),
        }
    }
}
