use crate::modules::chat::stores::chats::chat_actions::chat_types_hooks::{ChatTypesHooks, set_chat_types_hooks};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

pub fn install_chat_types_hooks() {
    set_chat_types_hooks(ChatTypesHooks { is_joined_member_role: ChatServicesStore::is_joined_member_role });
}
