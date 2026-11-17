use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetShortsHooks, set_account_state_reset_shorts_hooks,
};
use crate::modules::shorts::components::shorts_more_sheet::ShortsMoreSheetStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

pub fn install_account_state_reset_shorts_hooks() {
    set_account_state_reset_shorts_hooks(AccountStateResetShortsHooks {
        reset_account_state: |cx: &mut App| {
            ShortsServicesStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            ShortsMoreSheetStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        },
    });
}
