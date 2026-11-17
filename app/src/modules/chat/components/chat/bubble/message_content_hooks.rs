use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;

/// slot for the bubble to read the chat list title for the media ledger note
pub(crate) struct MessageContentHooks {
    pub(crate) chat_title: fn(&ChatInfo, &mut App) -> String,
}

pub(crate) static MESSAGE_CONTENT_HOOKS: FeatureHookSlot<MessageContentHooks> = FeatureHookSlot::new("MESSAGE_CONTENT_HOOKS");

pub(crate) fn set_message_content_hooks(hooks: MessageContentHooks) {
    let _ = MESSAGE_CONTENT_HOOKS.set(hooks);
}

pub(crate) fn message_content_hooks() -> Option<&'static MessageContentHooks> {
    MESSAGE_CONTENT_HOOKS.get()
}
