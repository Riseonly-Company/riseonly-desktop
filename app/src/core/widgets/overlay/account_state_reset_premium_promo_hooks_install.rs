use gpui::{App, Context};

use crate::core::widgets::overlay::premium_promo_store::PremiumPromoStore;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetPremiumPromoHooks, set_account_state_reset_premium_promo_hooks,
};

impl AccountResettable for PremiumPromoStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        PremiumPromoStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_premium_promo_hooks() {
    set_account_state_reset_premium_promo_hooks(AccountStateResetPremiumPromoHooks {
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&PremiumPromoStore::shared(cx), cx);
        },
    });
}
