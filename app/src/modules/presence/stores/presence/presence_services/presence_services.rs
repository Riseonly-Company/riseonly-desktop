use gpui::{App, Context};
use serde_json::Value;

use crate::modules::presence::engine::core::rise_presence_rpc::{
    self, decode_in_chat, decode_online_status,
};
use crate::modules::presence::engine::rise_presence_repository::{
    PresenceIndicator, PresenceRepository, PresenceState, SubscriptionDelta,
};

use super::super::presence_actions::PresenceActionsStore;

pub struct PresenceServicesStore {
    repository: PresenceRepository,
    actions: PresenceActionsStore,
    self_id: Option<String>,
}

impl PresenceServicesStore {
    pub fn new(actions: PresenceActionsStore) -> Self {
        Self {
            repository: PresenceRepository::new(),
            actions,
            self_id: None,
        }
    }

    pub fn set_self_id(&mut self, user_id: Option<String>, cx: &mut Context<Self>) {
        if self.self_id == user_id {
            return;
        }
        self.self_id = user_id;
        cx.notify();
    }

    pub fn apply_event(
        &mut self,
        event_type: &str,
        payload: &Value,
        cx: &mut Context<Self>,
    ) -> bool {
        let update = match event_type {
            rise_presence_rpc::ONLINE_STATUS_EVENT => decode_online_status(payload),
            rise_presence_rpc::IN_CHAT_EVENT => decode_in_chat(payload),
            _ => return false,
        };

        if let Some(update) = update
            && self.repository.apply(update)
        {
            cx.notify();
        }
        true
    }

    pub fn observe(&mut self, owner: &str, user_ids: Vec<String>, cx: &mut Context<Self>) {
        let delta = self
            .repository
            .set_desired(owner, user_ids, self.self_id.as_deref());
        self.send(delta, cx);
    }

    pub fn stop_observing(&mut self, owner: &str, cx: &mut Context<Self>) {
        let delta = self.repository.stop_observing(owner);
        self.send(delta, cx);
    }

    pub fn resubscribe_all(&self) {
        self.actions
            .resubscribe_action(self.repository.resubscribe_all());
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.repository.reset();
        self.self_id = None;
        cx.notify();
    }

    pub fn indicator(
        &self,
        user_id: &str,
        chat_id: &str,
        fallback_online: bool,
    ) -> PresenceIndicator {
        self.repository.indicator(user_id, chat_id, fallback_online)
    }

    pub fn is_online(&self, user_id: &str, fallback: bool) -> bool {
        match self.repository.state(user_id) {
            Some(state) if state.is_known => state.is_online,
            _ => fallback,
        }
    }

    pub fn last_seen_ms(&self, user_id: &str, fallback: Option<i64>) -> Option<i64> {
        match self.repository.state(user_id) {
            Some(state) if state.is_known => state.last_seen_ms,
            _ => fallback,
        }
    }

    pub fn state(&self, user_id: &str) -> Option<&PresenceState> {
        self.repository.state(user_id)
    }

    fn send(&self, delta: SubscriptionDelta, cx: &mut Context<Self>) {
        if delta.is_empty() {
            return;
        }
        self.actions.apply_subscription_delta_action(delta);
        cx.notify();
    }
}

pub struct PresenceStores {
    pub interactions: super::super::presence_interactions::PresenceInteractionsStore,
}

impl gpui::Global for PresenceStores {}

pub fn presence(cx: &App) -> Option<&PresenceStores> {
    cx.try_global::<PresenceStores>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::presence::engine::wire::PresenceWire;
    use gpui::{AppContext, Entity, TestAppContext};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingWire {
        sent: Mutex<Vec<String>>,
    }

    impl RecordingWire {
        fn sent(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl PresenceWire for RecordingWire {
        fn subscribe(&self, user_ids: Vec<String>) {
            self.sent
                .lock()
                .unwrap()
                .push(format!("subscribe:{}", user_ids.join(",")));
        }

        fn unsubscribe(&self, user_ids: Vec<String>) {
            self.sent
                .lock()
                .unwrap()
                .push(format!("unsubscribe:{}", user_ids.join(",")));
        }

        fn enter_chat(&self, chat_id: String) {
            self.sent.lock().unwrap().push(format!("enter:{chat_id}"));
        }

        fn leave_chat(&self, chat_id: String) {
            self.sent.lock().unwrap().push(format!("leave:{chat_id}"));
        }

        fn set_away(&self, status: String) {
            self.sent.lock().unwrap().push(format!("away:{status}"));
        }
    }

    fn store(cx: &mut TestAppContext) -> (Entity<PresenceServicesStore>, Arc<RecordingWire>) {
        let wire = Arc::new(RecordingWire::default());
        let actions = PresenceActionsStore::new(Arc::clone(&wire) as Arc<dyn PresenceWire>);
        let entity = cx.update(|cx| cx.new(|_| PresenceServicesStore::new(actions)));
        (entity, wire)
    }

    #[gpui::test]
    fn observing_sends_only_the_users_nobody_was_watching_yet(cx: &mut TestAppContext) {
        let (store, wire) = store(cx);

        store.update(cx, |store, cx| {
            store.observe("ChatList", vec!["u1".into(), "u2".into()], cx);
            store.observe("Chat:c1", vec!["u1".into()], cx);
        });

        assert_eq!(wire.sent(), vec!["subscribe:u1,u2"]);
    }

    #[gpui::test]
    fn a_push_for_a_user_updates_what_the_row_draws(cx: &mut TestAppContext) {
        let (store, _wire) = store(cx);

        store.update(cx, |store, cx| {
            assert!(store.apply_event(
                rise_presence_rpc::ONLINE_STATUS_EVENT,
                &json!({"user_id": "u1", "is_online": true}),
                cx,
            ));
            assert_eq!(
                store.indicator("u1", "c1", false),
                PresenceIndicator::Online
            );
        });
    }

    #[gpui::test]
    fn an_event_presence_does_not_own_is_reported_as_unhandled(cx: &mut TestAppContext) {
        let (store, _wire) = store(cx);

        store.update(cx, |store, cx| {
            assert!(
                !store.apply_event("message_created", &json!({"id": "m1"}), cx),
                "the router has to be able to tell an unhandled type from a handled one"
            );
        });
    }

    #[gpui::test]
    fn the_signed_in_user_is_never_subscribed_to(cx: &mut TestAppContext) {
        let (store, wire) = store(cx);

        store.update(cx, |store, cx| {
            store.set_self_id(Some("me".into()), cx);
            store.observe("Chat:c1", vec!["me".into(), "u1".into()], cx);
        });

        assert_eq!(wire.sent(), vec!["subscribe:u1"]);
    }

    #[gpui::test]
    fn a_reconnect_re_sends_every_live_subscription(cx: &mut TestAppContext) {
        let (store, wire) = store(cx);

        store.update(cx, |store, cx| {
            store.observe("ChatList", vec!["u1".into(), "u2".into()], cx);
            store.resubscribe_all();
        });

        assert_eq!(wire.sent(), vec!["subscribe:u1,u2", "subscribe:u1,u2"]);
    }

    #[gpui::test]
    fn switching_accounts_forgets_the_previous_ones_contacts(cx: &mut TestAppContext) {
        let (store, _wire) = store(cx);

        store.update(cx, |store, cx| {
            store.apply_event(
                rise_presence_rpc::ONLINE_STATUS_EVENT,
                &json!({"user_id": "u1", "is_online": true}),
                cx,
            );
            store.reset_account_state(cx);

            assert!(!store.is_online("u1", false));
            assert_eq!(store.state("u1"), None);
        });
    }
}
