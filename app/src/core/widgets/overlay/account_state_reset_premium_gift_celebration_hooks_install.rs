use gpui::{App, Context};

use crate::core::widgets::overlay::premium_gift_celebration_store::PremiumGiftCelebrationStore;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetPremiumGiftCelebrationHooks, set_account_state_reset_premium_gift_celebration_hooks,
};

impl AccountResettable for PremiumGiftCelebrationStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        PremiumGiftCelebrationStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_premium_gift_celebration_hooks() {
    set_account_state_reset_premium_gift_celebration_hooks(AccountStateResetPremiumGiftCelebrationHooks {
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&PremiumGiftCelebrationStore::shared(cx), cx);
        },
    });
}
