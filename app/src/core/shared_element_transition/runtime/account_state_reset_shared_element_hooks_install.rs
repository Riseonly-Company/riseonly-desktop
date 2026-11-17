use gpui::App;

use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetSharedElementHooks, set_account_state_reset_shared_element_hooks,
};

pub(crate) fn install_account_state_reset_shared_element_hooks() {
    set_account_state_reset_shared_element_hooks(AccountStateResetSharedElementHooks {
        reset_account_state: |cx: &mut App| {
            SharedElementTransitionCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.reset_account_state(cx));
        },
    });
}
