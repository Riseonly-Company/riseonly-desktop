use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetModerationHooks, set_account_state_reset_moderation_hooks,
};
use crate::modules::moderation::engine::rise_moderation_presentation::RiseModerationPresentation;
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;

pub fn install_account_state_reset_moderation_hooks() {
    set_account_state_reset_moderation_hooks(AccountStateResetModerationHooks {
        reset_account_state: |cx: &mut App| {
            ModerationServicesStore::shared(cx).update(cx, |services, cx| services.reset_account_state(cx));
            RiseModerationPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.reset_account_state(cx));
        },
    });
}
