use gpui::{App, Context};

use crate::core::widgets::overlay::achievement_earned_store::AchievementEarnedStore;
use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetAchievementEarnedHooks, set_account_state_reset_achievement_earned_hooks,
};

impl AccountResettable for AchievementEarnedStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        AchievementEarnedStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_achievement_earned_hooks() {
    set_account_state_reset_achievement_earned_hooks(AccountStateResetAchievementEarnedHooks {
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&AchievementEarnedStore::shared(cx), cx);
        },
    });
}
