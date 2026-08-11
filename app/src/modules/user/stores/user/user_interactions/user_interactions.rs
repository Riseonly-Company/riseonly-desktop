use gpui::{App, Entity};

use crate::modules::user::engine::rise_user_engine_models::ProfileKey;

use super::super::user_actions::UserActionsStore;
use super::super::user_services::UserServicesStore;

#[derive(Clone)]
pub struct UserInteractionsStore {
    actions: UserActionsStore,
    services: Entity<UserServicesStore>,
}

impl UserInteractionsStore {
    pub fn new(actions: UserActionsStore, services: Entity<UserServicesStore>) -> Self {
        Self { actions, services }
    }

    pub fn services(&self) -> &Entity<UserServicesStore> {
        &self.services
    }

    pub fn open_profile(&self, key: ProfileKey) {
        self.actions.activate_profile_action(key);
    }

    pub fn refresh_profile(&self, key: ProfileKey) {
        self.actions.refresh_profile_action(key);
    }

    pub fn toggle_follow(&self, key: ProfileKey, cx: &App) {
        let services = self.services.read(cx);
        if services.is_viewer(&key) || services.is_follow_in_flight(&key) {
            return;
        }
        self.actions.toggle_follow_action(key);
    }
}
