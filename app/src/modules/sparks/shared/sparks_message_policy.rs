use gpui::App;

use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatPerm, ChatPermExt, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageContentType};
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;

/// who may send sparks to a message, same gates as the service
/// not oneself or service lines or bots or calls or moderated
/// private chat offered until server refuses
pub enum SparksMessagePolicy {}

impl SparksMessagePolicy {
    pub fn can_send(message: &GetMessageMessage, chat: &ChatInfo, is_own: bool) -> bool {
        if is_own
            || message.is_system()
            || message.content_type == MessageContentType::Call
            || message.is_pending_outgoing()
            || message.is_bot_message
            || message.is_disabled_by_moderation()
        {
            return false;
        }
        match chat.r#type {
            ChatType::Channel => !ChatPerm::is_owner_or_legacy_admin_role(chat.role.as_deref()),
            ChatType::Group | ChatType::Direct => true,
            ChatType::Favourites => false,
        }
    }

    pub fn offers_send(message: &GetMessageMessage, chat: &ChatInfo, is_own: bool, cx: &mut App) -> bool {
        if chat.is_community_preview() || !Self::can_send(message, chat, is_own) {
            return false;
        }
        match chat.r#type {
            ChatType::Channel | ChatType::Group => {
                ChatServicesStore::shared(cx).read(cx).paid_reactions_allowed(&chat.id, chat.r#type)
            }
            ChatType::Direct => !SparksServicesStore::shared(cx).read(cx).direct_chat_refuses_sparks(&chat.id),
            ChatType::Favourites => false,
        }
    }
}
