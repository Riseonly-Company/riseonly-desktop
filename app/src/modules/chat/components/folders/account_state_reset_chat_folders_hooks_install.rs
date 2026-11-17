use gpui::App;

use crate::modules::chat::components::folders::account_state_reset_chat_folders_hooks::{
    AccountStateResetChatFoldersHooks, set_account_state_reset_chat_folders_hooks,
};
use crate::modules::chat::components::folders::chat_folder_editor_store::ChatFolderEditorStore;

pub fn install_account_state_reset_chat_folders_hooks() {
    set_account_state_reset_chat_folders_hooks(AccountStateResetChatFoldersHooks {
        reset_account_state: |cx: &mut App| {
            ChatFolderEditorStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        },
    });
}
