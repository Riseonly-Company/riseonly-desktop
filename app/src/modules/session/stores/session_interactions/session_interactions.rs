use gpui::{App, Entity};

use crate::modules::session::engine::rise_session_repository::SessionIdentity;

use super::super::session_actions::SessionActionsStore;
use super::super::session_services::SessionServicesStore;

#[derive(Clone)]
pub struct SessionInteractionsStore {
    actions: SessionActionsStore,
    services: Entity<SessionServicesStore>,
}

impl SessionInteractionsStore {
    pub fn new(actions: SessionActionsStore, services: Entity<SessionServicesStore>) -> Self {
        Self { actions, services }
    }

    pub fn services(&self) -> &Entity<SessionServicesStore> {
        &self.services
    }

    /// Opening the screen. The repository's own gate stops a second open from
    /// re-fetching, so this is safe to call on every appearance.
    pub fn ensure_loaded(&self, identity: SessionIdentity) {
        self.actions.load_sessions_action(identity, false);
    }

    pub fn refresh(&self, identity: SessionIdentity) {
        self.actions.load_sessions_action(identity, true);
    }

    pub fn load_more(&self, identity: SessionIdentity, cx: &App) {
        let services = self.services.read(cx);
        if !services.has_more() || services.is_loading() {
            return;
        }
        self.actions.load_more_sessions_action(identity);
    }

    pub fn terminate(&self, identity: SessionIdentity, session_id: &str, cx: &App) {
        if self.services.read(cx).is_mutating() {
            return;
        }
        self.actions
            .terminate_session_action(identity, session_id.to_owned());
    }

    pub fn terminate_all_others(&self, identity: SessionIdentity, cx: &App) {
        if self.services.read(cx).is_mutating() {
            return;
        }
        self.actions.terminate_all_others_action(identity);
    }

    pub fn reset_account_state(&self) {
        self.actions.reset_action();
    }
}
