use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetDiagnosticsHooks, set_account_state_reset_diagnostics_hooks,
};
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportInteractionsStore;

pub fn install_account_state_reset_diagnostics_hooks() {
    set_account_state_reset_diagnostics_hooks(AccountStateResetDiagnosticsHooks {
        reset_account_state: |cx: &mut App| {
            BugReportInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.account_did_change(cx));
        },
    });
}
