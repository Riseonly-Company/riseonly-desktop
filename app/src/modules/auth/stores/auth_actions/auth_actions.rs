use std::sync::Arc;

use crate::modules::auth::engine::rise_auth_domain::RiseAuthDomain;
use crate::modules::auth::engine::rise_auth_repository::AuthCommand;

#[derive(Clone)]
pub struct AuthActionsStore {
    domain: Arc<RiseAuthDomain>,
}

impl AuthActionsStore {
    pub fn new(domain: Arc<RiseAuthDomain>) -> Self {
        Self { domain }
    }

    pub fn restore_session_action(&self) {
        self.domain.dispatch(AuthCommand::Restore);
    }

    pub fn sign_in_action(&self, phone: String, password: String) {
        self.domain
            .dispatch(AuthCommand::SignIn { phone, password });
    }

    pub fn send_code_action(&self, phone: String) {
        self.domain.dispatch(AuthCommand::SendCode { phone });
    }

    pub fn register_action(
        &self,
        name: String,
        tag: String,
        phone: String,
        password: String,
        code: String,
    ) {
        self.domain.dispatch(AuthCommand::Register {
            name,
            tag,
            phone,
            password,
            code,
        });
    }

    pub fn check_tag_action(&self, tag: String) {
        self.domain.dispatch(AuthCommand::CheckTag { tag });
    }

    pub fn confirm_telegram_completed_action(&self) {
        self.domain.dispatch(AuthCommand::ConfirmTelegramCompleted);
    }

    pub fn cancel_code_entry_action(&self) {
        self.domain.dispatch(AuthCommand::CancelCodeEntry);
    }

    pub fn clear_error_action(&self) {
        self.domain.dispatch(AuthCommand::ClearError);
    }

    pub fn reset_flow_action(&self) {
        self.domain.dispatch(AuthCommand::ResetFlow);
    }

    pub fn sign_out_action(&self) {
        self.domain.dispatch(AuthCommand::SignOut);
    }

    pub fn switch_account_action(&self, user_id: String) {
        self.domain.dispatch(AuthCommand::SwitchAccount { user_id });
    }

    pub fn note_remote_failure_action(&self, code: Option<String>, message: Option<String>) {
        self.domain
            .dispatch(AuthCommand::NoteRemoteFailure { code, message });
    }
}
