use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetSessionHooks, set_account_state_reset_session_hooks,
};
use crate::modules::session::stores::session_services::session_services::SessionServicesStore;

pub(crate) fn install_account_state_reset_session_hooks() {
    set_account_state_reset_session_hooks(AccountStateResetSessionHooks {
        reset_account_state: |cx| {
            SessionServicesStore::shared(cx).update(cx, |service, cx| service.reset_account_state(cx));
        },
    });
}
