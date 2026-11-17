use gpui::App;

use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetCoreWidgetsHooks, set_account_state_reset_core_widgets_hooks,
};

pub(crate) fn install_account_state_reset_core_widgets_hooks() {
    set_account_state_reset_core_widgets_hooks(AccountStateResetCoreWidgetsHooks {
        reset_account_state: |cx: &mut App| {
            ReportSheetStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        },
    });
}
