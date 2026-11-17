use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{RiseStoredChat, RiseStoredChatPreview};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types_hooks::{
    ChatWireTypesHooks, set_chat_wire_types_hooks,
};

pub(crate) fn install_chat_wire_types_hooks() {
    set_chat_wire_types_hooks(ChatWireTypesHooks {
        stored_chat_from_canon: RiseStoredChat::from_canon,
        stored_chat_preview_from_canon: RiseStoredChatPreview::from_canon,
    });
}
