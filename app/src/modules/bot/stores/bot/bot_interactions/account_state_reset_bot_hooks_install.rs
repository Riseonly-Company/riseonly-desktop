use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetBotHooks, set_account_state_reset_bot_hooks,
};
use crate::modules::bot::shared::bot_ui_validation::BotCreateFlowState;
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;

pub(crate) fn install_account_state_reset_bot_hooks() {
    set_account_state_reset_bot_hooks(AccountStateResetBotHooks {
        reset_account_state: |cx: &mut App| {
            BotServicesStore::shared(cx).update(cx, |services, cx| services.reset_account_state(cx));
            BotCreateFlowState::shared(cx).update(cx, |state, cx| state.reset_account_state(cx));
            BotInteractionsStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        },
    });
}
