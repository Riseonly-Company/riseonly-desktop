use gpui::{App, Entity};

use super::super::presence_actions::PresenceActionsStore;
use super::super::presence_services::PresenceServicesStore;

#[derive(Clone)]
pub struct PresenceInteractionsStore {
    actions: PresenceActionsStore,
    services: Entity<PresenceServicesStore>,
}

impl PresenceInteractionsStore {
    pub fn new(actions: PresenceActionsStore, services: Entity<PresenceServicesStore>) -> Self {
        Self { actions, services }
    }

    pub fn observe(&self, owner: &str, user_ids: Vec<String>, cx: &mut App) {
        self.services.update(cx, |store, cx| {
            store.observe(owner, user_ids, cx);
        });
    }

    pub fn stop_observing(&self, owner: &str, cx: &mut App) {
        self.services.update(cx, |store, cx| {
            store.stop_observing(owner, cx);
        });
    }

    pub fn enter_chat(&self, chat_id: &str) {
        self.actions.enter_chat_action(chat_id.to_owned());
    }

    pub fn leave_chat(&self, chat_id: &str) {
        self.actions.leave_chat_action(chat_id.to_owned());
    }

    pub fn resubscribe_all(&self, cx: &App) {
        self.services.read(cx).resubscribe_all();
    }

    pub fn reset_account_state(&self, cx: &mut App) {
        self.services.update(cx, |store, cx| {
            store.reset_account_state(cx);
        });
    }

    pub fn services(&self) -> &Entity<PresenceServicesStore> {
        &self.services
    }
}
