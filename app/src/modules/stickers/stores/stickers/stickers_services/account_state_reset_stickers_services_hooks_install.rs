use gpui::{App, Context};

use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetStickersServicesHooks, set_account_state_reset_stickers_services_hooks,
};
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

impl AccountResettable for StickersServicesStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        StickersServicesStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_stickers_services_hooks() {
    set_account_state_reset_stickers_services_hooks(AccountStateResetStickersServicesHooks {
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&StickersServicesStore::shared(cx), cx);
        },
    });
}
