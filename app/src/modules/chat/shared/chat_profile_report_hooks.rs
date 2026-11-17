use gpui::App;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::bot::shared::bot_ui_validation::BotChatManagement;
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotTagRules;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_profile_permissions::ChatProfilePermissions;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// what a report from the chat profile is about
/// the person or the whole group or channel or the bot
/// lives here since the overflow menu offers it now
pub enum ChatProfileReportTarget {}

impl ChatProfileReportTarget {
    pub fn resolve(chat: &ChatInfo, cx: &mut App) -> Option<ReportTarget> {
        match chat.r#type {
            ChatType::Group | ChatType::Channel => {
                if ChatProfilePermissions::is_owner(chat) || chat.id.is_empty() {
                    return None;
                }
                Some(ReportTarget::Chat(chat.id.clone()))
            }
            ChatType::Direct => {
                let wire_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
                if chat.is_bot_chat() {
                    let tag = BotChatManagement::normalized_tag(chat);
                    let bot = if BotTagRules::is_valid(&tag) { BotServicesStore::bot_by_tag(&tag, cx) } else { None };
                    if BotChatManagement::can_manage(chat, wire_user_id.as_deref().unwrap_or(""), bot.as_ref()) {
                        return None;
                    }
                    if let Some(bot_id) = bot.map(|bot| bot.id).filter(|id| !id.is_empty()) {
                        return Some(ReportTarget::Bot(bot_id));
                    }
                    return (!chat.id.is_empty()).then(|| ReportTarget::Chat(chat.id.clone()));
                }
                let peer = &chat.participant.id;
                if peer.is_empty()
                    || wire_user_id.as_deref() == Some(peer.as_str())
                    || UserServicesStore::is_contact_join_virtual_chat_id(&chat.id)
                {
                    return None;
                }
                Some(ReportTarget::User(peer.clone()))
            }
            ChatType::Favourites => None,
        }
    }
}
