use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetUserHooks, set_account_state_reset_user_hooks,
};
use crate::modules::user::components::premium_badge_picker::premium_badge_picker_popover::PremiumBadgePickerPopover;
use crate::modules::user::engine::rise_user_presentation::RiseUserPresentation;
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::modules::user::stores::subscription::premium_gift_store::PremiumGiftStore;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::SubscriptionServicesStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub fn install_account_state_reset_user_hooks() {
    set_account_state_reset_user_hooks(AccountStateResetUserHooks {
        activate_account_state: |cx| {
            RiseUserTabBarPresentation::shared(cx).update(cx, |presentation, cx| presentation.activate_account_state(cx));
        },
        rise_user_presentation_report_theme_settings_failure: |error, account_id, account_epoch, cx| {
            RiseUserPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.report_theme_settings_failure(error, Some(account_id), account_epoch, cx);
            });
        },
        reset_account_state: |cx| {
            UserServicesStore::shared(cx).update(cx, |service, cx| service.reset_account_state(cx));
            SubscriptionServicesStore::shared(cx).update(cx, |service, cx| service.reset_account_state(cx));
            PremiumGiftStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        },
        premium_badge_picker_popover_dismiss: |cx| {
            PremiumBadgePickerPopover::shared(cx).update(cx, |popover, cx| popover.dismiss(cx));
        },
    });
}
