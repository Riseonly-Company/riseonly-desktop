use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetSparksHooks, set_account_state_reset_sparks_hooks,
};
use crate::modules::sparks::stores::sparks::sparks_services::sparks_services::SparksServicesStore;

pub fn install_account_state_reset_sparks_hooks() {
    set_account_state_reset_sparks_hooks(AccountStateResetSparksHooks {
        reset_account_state: |cx: &mut App| {
            SparksServicesStore::shared(cx).update(cx, |services, cx| services.reset_account_state(cx));
        },
    });
}
