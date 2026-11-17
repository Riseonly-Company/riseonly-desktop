use gpui::{App, Context};

use crate::core::ui::emoji_picker_ui::emoji_picker_recents_store::EmojiPickerRecentsStore;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetEmojiPickerRecentsHooks, set_account_state_reset_emoji_picker_recents_hooks,
};

impl AccountResettable for EmojiPickerRecentsStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        EmojiPickerRecentsStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_emoji_picker_recents_hooks() {
    set_account_state_reset_emoji_picker_recents_hooks(AccountStateResetEmojiPickerRecentsHooks {
        activate_account_state: |cx: &mut App| {
            EmojiPickerRecentsStore::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
        },
        remove_persisted_account: |user_id, cx| {
            EmojiPickerRecentsStore::shared(cx).read(cx).remove_persisted_account(user_id);
        },
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&EmojiPickerRecentsStore::shared(cx), cx);
        },
    });
}
