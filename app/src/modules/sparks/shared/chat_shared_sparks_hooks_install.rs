use gpui::App;

use crate::modules::chat::shared::chat_shared_sparks_hooks::{ChatSharedSparksHooks, set_chat_shared_sparks_hooks};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::sparks::shared::sparks_message_policy::SparksMessagePolicy;

pub fn install_chat_shared_sparks_hooks() {
    set_chat_shared_sparks_hooks(ChatSharedSparksHooks {
        offers_send: |message: &GetMessageMessage, chat: &ChatInfo, is_own: bool, cx: &mut App| {
            SparksMessagePolicy::offers_send(message, chat, is_own, cx)
        },
    });
}
