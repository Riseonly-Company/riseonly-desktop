use gpui::{App, Context};

use crate::core::media::media_save_coordinator::MediaSaveCoordinator;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetMediaSaveHooks, set_account_state_reset_media_save_hooks,
};

impl AccountResettable for MediaSaveCoordinator {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        MediaSaveCoordinator::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_media_save_hooks() {
    set_account_state_reset_media_save_hooks(AccountStateResetMediaSaveHooks {
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&MediaSaveCoordinator::shared(cx), cx);
        },
    });
}
