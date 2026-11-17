use gpui::{App, Context};

use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetPremiumUpsellHooks, set_account_state_reset_premium_upsell_hooks,
};

impl AccountResettable for PremiumUpsellStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        PremiumUpsellStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_premium_upsell_hooks() {
    set_account_state_reset_premium_upsell_hooks(AccountStateResetPremiumUpsellHooks {
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&PremiumUpsellStore::shared(cx), cx);
        },
    });
}
