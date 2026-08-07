use std::sync::Arc;

use crate::modules::session::engine::rise_session_domain::RiseSessionDomain;
use crate::modules::session::engine::rise_session_repository::{SessionCommand, SessionIdentity};

#[derive(Clone)]
pub struct SessionActionsStore {
    domain: Arc<RiseSessionDomain>,
}

impl SessionActionsStore {
    pub fn new(domain: Arc<RiseSessionDomain>) -> Self {
        Self { domain }
    }

    pub fn load_sessions_action(&self, identity: SessionIdentity, force: bool) {
        self.domain
            .dispatch(SessionCommand::Load { identity, force });
    }

    pub fn load_more_sessions_action(&self, identity: SessionIdentity) {
        self.domain.dispatch(SessionCommand::LoadMore { identity });
    }

    pub fn terminate_session_action(&self, identity: SessionIdentity, session_id: String) {
        self.domain.dispatch(SessionCommand::Terminate {
            identity,
            session_id,
        });
    }

    pub fn terminate_all_others_action(&self, identity: SessionIdentity) {
        self.domain
            .dispatch(SessionCommand::TerminateAllOthers { identity });
    }

    pub fn reset_action(&self) {
        self.domain.dispatch(SessionCommand::Reset);
    }
}
