use gpui::App;

use crate::modules::analytics::stores::engagement::engagement_services::engagement_services::EngagementServicesStore;
use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetAnalyticsHooks, set_account_state_reset_analytics_hooks,
};

pub fn install_account_state_reset_analytics_hooks() {
    set_account_state_reset_analytics_hooks(AccountStateResetAnalyticsHooks {
        activate_account_state: |cx: &mut App| {
            EngagementServicesStore::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
        },
        remove_persisted_account: |user_id: &str, cx: &mut App| {
            EngagementServicesStore::shared(cx).read(cx).remove_persisted_account(user_id);
        },
        reset_account_state: |cx: &mut App| {
            EngagementServicesStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        },
    });
}
