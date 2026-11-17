use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetVacancyHooks, set_account_state_reset_vacancy_hooks,
};
use crate::modules::vacancy::shared::vacancy_mocks::VacancyLocalState;
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

pub(crate) fn install_account_state_reset_vacancy_hooks() {
    set_account_state_reset_vacancy_hooks(AccountStateResetVacancyHooks {
        activate_account_state: |cx| {
            VacancyLocalState::shared(cx).update(cx, |state, cx| state.activate_account_state(cx));
        },
        reset_account_state: |cx| {
            VacancyServicesStore::shared(cx).update(cx, |service, cx| service.reset_account_state(cx));
            VacancyLocalState::shared(cx).update(cx, |state, cx| state.reset_account_state(cx));
        },
        remove_persisted_account: |user_id, cx| {
            VacancyLocalState::shared(cx).update(cx, |state, cx| state.remove_persisted_account(user_id, cx));
        },
    });
}
