use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetChatUiHooks, set_account_state_reset_chat_ui_hooks,
};
use crate::modules::chat::components::chat::async_render::async_text_bubble_predisplay::AsyncTextBubblePredisplayCache;
use crate::modules::chat::components::chat::async_render::async_text_bubble_prepared_layout_cache::AsyncTextBubblePreparedLayoutCache;
use crate::modules::chat::components::chat::async_render::rise_message_fallback_height_cache::RiseMessageFallbackHeightCache;
use crate::modules::chat::components::folders::chat_folder_editor_store::ChatFolderEditorStore;
use crate::modules::chat::shared::chat_message_jump_progress::ChatMessageJumpProgress;

pub(crate) fn install_account_state_reset_chat_ui_hooks() {
    set_account_state_reset_chat_ui_hooks(AccountStateResetChatUiHooks {
        reset_account_state: |cx: &mut App| {
            ChatFolderEditorStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            ChatMessageJumpProgress::shared(cx).update(cx, |progress, cx| progress.reset_account_state(cx));
        },
        hard_reset_render_caches: |cx: &mut App| {
            AsyncTextBubblePredisplayCache::shared().remove_all_images();
            AsyncTextBubblePreparedLayoutCache::shared().remove_all();
            RiseMessageFallbackHeightCache::shared(cx).update(cx, |cache, _| cache.remove_all());
        },
    });
}
