use std::rc::Rc;

use gpui::{AnyView, App};

use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::native_bubbles::sticker::native_interactive_message_cell::NativeInteractiveMessageCell;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

/// base row for the native voice and video note bubbles
/// like sticker and document kinds the row is the regular bubble row
/// at the exact height the kind answered
pub enum NativeVoiceNoteInteractionCell {}

impl NativeVoiceNoteInteractionCell {
    pub fn make_view(
        reusing: Option<AnyView>,
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &Rc<RiseMessageCollectionConfiguration>,
        viewport_width: f32,
        height: Option<f32>,
        cx: &mut App,
    ) -> AnyView {
        NativeInteractiveMessageCell::make_view(reusing, message, group, configuration, viewport_width, height, cx)
    }
}
