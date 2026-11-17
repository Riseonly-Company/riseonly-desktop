use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub enum ChatContactProfilePolicy {}

impl ChatContactProfilePolicy {
    pub fn is_eligible(chat: &ChatInfo, current_user_id: &str, is_topic_profile: bool) -> bool {
        !chat.is_bot_chat() && Self::is_blockable(chat, current_user_id, is_topic_profile)
    }

    /// user-service blocks third party bots like telegram
    /// an official bot is a system account its trigger refuses so it gets no block entry
    pub fn is_blockable(chat: &ChatInfo, current_user_id: &str, is_topic_profile: bool) -> bool {
        !is_topic_profile
            && chat.r#type == ChatType::Direct
            && !(chat.is_bot_chat() && chat.participant.more.is_official)
            && !chat.participant.is_deleted
            && !chat.participant.id.is_empty()
            && chat.participant.id != current_user_id
            && !UserServicesStore::is_contact_join_virtual_chat_id(&chat.id)
    }
}
