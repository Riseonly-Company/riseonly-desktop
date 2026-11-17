use super::notify_services::NotifyServicesStore;
use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetNotifyHooks, set_account_state_reset_notify_hooks,
};
use crate::modules::notify::engine::rise_notify_unread_presentation::RiseNotifyUnreadPresentation;

pub fn install_account_state_reset_notify_hooks() {
    set_account_state_reset_notify_hooks(AccountStateResetNotifyHooks {
        fence_account_transition: |cx| {
            RiseNotifyUnreadPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.fence_account_transition(cx);
            });
        },
        reset_account_state: |cx| {
            NotifyServicesStore::shared(cx).update(cx, |service, cx| service.reset_account_state(cx));
        },
    });
}
