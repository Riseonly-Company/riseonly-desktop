use gpui::App;

use crate::modules::chat::stores::chat_stores_hooks::chat_stores_user_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatMuteScopes};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

impl ChatServicesStore {
    pub fn effective_notification_scopes(
        stored: ChatMuteScopes,
        mute_all: bool,
        is_exception: bool,
    ) -> ChatMuteScopes {
        if mute_all && !is_exception { ChatMuteScopes::MUTED } else { stored }
    }

    pub fn effective_notification_scopes_for_chat(
        &self,
        chat_id: &str,
        stored: ChatMuteScopes,
        cx: &App,
    ) -> ChatMuteScopes {
        let hooks = chat_stores_user_hooks();
        Self::effective_notification_scopes(
            stored,
            hooks.is_some_and(|hooks| (hooks.mute_all_chats_enabled)(cx)),
            hooks.is_some_and(|hooks| (hooks.chat_mute_override_ids)(cx).contains(chat_id)),
        )
    }

    pub fn effective_is_muted(&self, chat: &ChatInfo, cx: &App) -> bool {
        if chat.is_muted {
            return true;
        }
        if let Some(hooks) = chat_stores_user_hooks()
            && (hooks.mute_all_chats_enabled)(cx)
            && !(hooks.chat_mute_override_ids)(cx).contains(chat.id.as_str())
        {
            return ChatMuteScopes::MUTED.is_fully_muted();
        }
        chat.notification_scopes().is_fully_muted()
    }
}
