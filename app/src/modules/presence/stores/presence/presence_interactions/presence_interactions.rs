use std::collections::HashSet;

use gpui::{App, Context, Entity};

use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::modules::presence::stores::presence::presence_actions::presence_actions::PresenceActionsStore;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;

pub struct PresenceInteractionsStore {}

impl PresenceInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn observe(&self, user_ids: &[String], owner: &str, cx: &mut App) {
        let desired: HashSet<String> = user_ids.iter().cloned().collect();
        PresenceServicesStore::shared(cx).update(cx, |store, cx| store.set_desired(desired, owner, cx));
    }

    pub fn stop_observing(&self, owner: &str, cx: &mut App) {
        PresenceServicesStore::shared(cx).update(cx, |store, cx| store.set_desired(HashSet::new(), owner, cx));
    }

    pub fn enter_chat(&self, chat_id: &str, cx: &mut App) {
        if GetSaiServerId::is_temp_id(Some(chat_id)) {
            return;
        }
        PresenceActionsStore::shared(cx).update(cx, |store, cx| store.enter_chat_action(chat_id, cx));
    }

    pub fn leave_chat(&self, chat_id: &str, cx: &mut App) {
        if GetSaiServerId::is_temp_id(Some(chat_id)) {
            return;
        }
        PresenceActionsStore::shared(cx).update(cx, |store, cx| store.leave_chat_action(chat_id, cx));
    }

    pub fn resubscribe_all(&self, cx: &mut App) {
        PresenceServicesStore::shared(cx).update(cx, |store, cx| store.resubscribe_all(cx));
    }
}
