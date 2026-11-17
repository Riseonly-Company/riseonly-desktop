use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

pub(crate) struct ChatSharedSparksHooks {
    pub offers_send: fn(message: &GetMessageMessage, chat: &ChatInfo, is_own: bool, cx: &mut App) -> bool,
}

pub(crate) static CHAT_SHARED_SPARKS_HOOKS: FeatureHookSlot<ChatSharedSparksHooks> = FeatureHookSlot::new("CHAT_SHARED_SPARKS_HOOKS");

pub(crate) fn set_chat_shared_sparks_hooks(hooks: ChatSharedSparksHooks) {
    let _ = CHAT_SHARED_SPARKS_HOOKS.set(hooks);
}

pub(crate) fn chat_shared_sparks_hooks() -> Option<&'static ChatSharedSparksHooks> {
    CHAT_SHARED_SPARKS_HOOKS.get()
}
