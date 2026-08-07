use gpui::{App, Entity};

use super::super::auth_actions::AuthActionsStore;
use super::super::auth_actions::auth_types::{self, AuthStep, AuthStepForm, StepValidation};
use super::super::auth_services::AuthServicesStore;

/// The only thing a view may call.
///
/// Validation, guards and the choice of command live here; nothing else does.
/// It holds no server state — every read goes through the services store, and
/// every write is a command the repository serialises.
#[derive(Clone)]
pub struct AuthInteractionsStore {
    actions: AuthActionsStore,
    services: Entity<AuthServicesStore>,
}

impl AuthInteractionsStore {
    pub fn new(actions: AuthActionsStore, services: Entity<AuthServicesStore>) -> Self {
        Self { actions, services }
    }

    pub fn services(&self) -> &Entity<AuthServicesStore> {
        &self.services
    }

    pub fn restore_session(&self) {
        self.actions.restore_session_action();
    }

    /// Runs the step the user is on, if it is complete.
    ///
    /// Returns the next step, or `None` when the step handed the flow to the
    /// server and the answer decides what happens next.
    pub fn submit_step(
        &self,
        step: AuthStep,
        form: &AuthStepForm,
        registering: bool,
        cx: &App,
    ) -> StepOutcome {
        if self.services.read(cx).is_busy() {
            return StepOutcome::Ignored;
        }

        if let StepValidation::Invalid(key) = auth_types::validate(step, form) {
            return StepOutcome::Invalid(key);
        }

        match step {
            AuthStep::Phone => {
                if registering {
                    StepOutcome::Advance(AuthStep::RegistrationName)
                } else {
                    StepOutcome::Advance(AuthStep::LoginPassword)
                }
            }
            AuthStep::LoginPassword => {
                self.actions
                    .sign_in_action(form.composed_phone(), form.login_password.clone());
                StepOutcome::Submitted
            }
            AuthStep::RegistrationName => StepOutcome::Advance(AuthStep::RegistrationTag),
            AuthStep::RegistrationTag => StepOutcome::Advance(AuthStep::RegistrationPassword),
            AuthStep::RegistrationPassword => {
                StepOutcome::Advance(AuthStep::RegistrationPasswordConfirmation)
            }
            AuthStep::RegistrationPasswordConfirmation => {
                self.actions.send_code_action(form.composed_phone());
                StepOutcome::Submitted
            }
            AuthStep::VerificationCode => {
                self.actions.register_action(
                    form.name.clone(),
                    form.tag.clone(),
                    form.composed_phone(),
                    form.registration_password.clone(),
                    form.code.clone(),
                );
                StepOutcome::Submitted
            }
        }
    }

    /// Asks whether a tag is free, but only for a tag that could be.
    ///
    /// A locally invalid tag is answered without a request: `/api/auth/check-tag`
    /// is rate limited per client, and spending that budget on a tag the rules
    /// already reject means the real check gets refused later.
    pub fn check_tag(&self, tag: &str) {
        self.actions.check_tag_action(tag.to_owned());
    }

    pub fn complete_telegram_redirect(&self) {
        self.actions.confirm_telegram_completed_action();
    }

    pub fn cancel_verification_code_entry(&self) {
        self.actions.cancel_code_entry_action();
    }

    pub fn clear_error(&self) {
        self.actions.clear_error_action();
    }

    pub fn reset_flow(&self) {
        self.actions.reset_flow_action();
    }

    pub fn sign_out(&self) {
        self.actions.sign_out_action();
    }

    pub fn switch_account(&self, user_id: &str, cx: &App) {
        if self.services.read(cx).is_current_user(user_id) {
            return;
        }
        self.actions.switch_account_action(user_id.to_owned());
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepOutcome {
    Advance(AuthStep),
    Submitted,
    Invalid(&'static str),
    /// A request is already in flight. Pressing the button twice must not send
    /// a second registration.
    Ignored,
}
