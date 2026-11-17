use gpui::{App, Context};

use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetShareSheetHooks, set_account_state_reset_share_sheet_hooks,
};

impl AccountResettable for ShareSheetStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        ShareSheetStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_share_sheet_hooks() {
    set_account_state_reset_share_sheet_hooks(AccountStateResetShareSheetHooks {
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&ShareSheetStore::shared(cx), cx);
        },
    });
}
