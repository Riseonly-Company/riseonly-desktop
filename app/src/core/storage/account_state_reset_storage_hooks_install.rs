use gpui::App;

use crate::core::storage::rise_storage_usage_presentation::RiseStorageUsagePresentation;
use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetStorageHooks, set_account_state_reset_storage_hooks,
};

pub(crate) fn install_account_state_reset_storage_hooks() {
    set_account_state_reset_storage_hooks(AccountStateResetStorageHooks {
        reset_account_state: |cx: &mut App| {
            RiseStorageUsagePresentation::shared(cx).update(cx, |presentation, cx| presentation.reset_account_state(cx));
        },
    });
}
