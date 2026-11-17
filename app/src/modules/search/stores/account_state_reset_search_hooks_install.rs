use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetSearchHooks, set_account_state_reset_search_hooks,
};
use crate::modules::search::stores::search_services::search_services::SearchServicesStore;

pub fn install_account_state_reset_search_hooks() {
    set_account_state_reset_search_hooks(AccountStateResetSearchHooks {
        reset_account_state: |cx: &mut App| {
            SearchServicesStore::shared(cx).update(cx, |services, cx| services.reset_account_state(cx));
        },
    });
}
