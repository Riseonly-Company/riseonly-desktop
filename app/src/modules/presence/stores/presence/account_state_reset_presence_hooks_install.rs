use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetPresenceHooks, set_account_state_reset_presence_hooks,
};
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;

pub(crate) fn install_account_state_reset_presence_hooks() {
    set_account_state_reset_presence_hooks(AccountStateResetPresenceHooks {
        reset_account_state: |cx: &mut App| {
            PresenceServicesStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        },
    });
}
