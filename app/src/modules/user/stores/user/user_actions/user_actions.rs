use std::sync::Arc;

use crate::modules::user::engine::rise_user_domain::RiseUserDomain;
use crate::modules::user::engine::rise_user_engine_models::ProfileKey;
use crate::modules::user::engine::rise_user_repository::UserCommand;

#[derive(Clone)]
pub struct UserActionsStore {
    domain: Arc<RiseUserDomain>,
}

impl UserActionsStore {
    pub fn new(domain: Arc<RiseUserDomain>) -> Self {
        Self { domain }
    }

    pub fn activate_profile_action(&self, key: ProfileKey) {
        self.domain.dispatch(UserCommand::Activate { key });
    }

    pub fn refresh_profile_action(&self, key: ProfileKey) {
        self.domain.dispatch(UserCommand::Refresh { key });
    }

    pub fn toggle_follow_action(&self, key: ProfileKey) {
        self.domain.dispatch(UserCommand::ToggleFollow { key });
    }

    pub fn set_viewer_action(&self, user_id: Option<String>) {
        self.domain.dispatch(UserCommand::SetViewer { user_id });
    }

    pub fn reset_action(&self) {
        self.domain.dispatch(UserCommand::Reset);
    }
}
