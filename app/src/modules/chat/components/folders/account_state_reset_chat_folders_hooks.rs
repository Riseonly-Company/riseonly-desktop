use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

/// folder editor reset the chat ui reset runs before its own
pub struct AccountStateResetChatFoldersHooks {
    pub reset_account_state: fn(cx: &mut App),
}

pub(crate) static ACCOUNT_STATE_RESET_CHAT_FOLDERS_HOOKS: FeatureHookSlot<AccountStateResetChatFoldersHooks> =
    FeatureHookSlot::new("ACCOUNT_STATE_RESET_CHAT_FOLDERS_HOOKS");

pub fn set_account_state_reset_chat_folders_hooks(hooks: AccountStateResetChatFoldersHooks) {
    let _ = ACCOUNT_STATE_RESET_CHAT_FOLDERS_HOOKS.set(hooks);
}

pub fn account_state_reset_chat_folders_hooks() -> Option<&'static AccountStateResetChatFoldersHooks> {
    ACCOUNT_STATE_RESET_CHAT_FOLDERS_HOOKS.get()
}
