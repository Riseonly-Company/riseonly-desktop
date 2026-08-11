use std::sync::Arc;

use crate::modules::presence::engine::rise_presence_repository::SubscriptionDelta;
use crate::modules::presence::engine::wire::PresenceWire;

#[derive(Clone)]
pub struct PresenceActionsStore {
    wire: Arc<dyn PresenceWire>,
}

impl PresenceActionsStore {
    pub fn new(wire: Arc<dyn PresenceWire>) -> Self {
        Self { wire }
    }

    pub fn apply_subscription_delta_action(&self, delta: SubscriptionDelta) {
        if !delta.subscribe.is_empty() {
            self.wire.subscribe(delta.subscribe);
        }
        if !delta.unsubscribe.is_empty() {
            self.wire.unsubscribe(delta.unsubscribe);
        }
    }

    pub fn resubscribe_action(&self, user_ids: Vec<String>) {
        if user_ids.is_empty() {
            return;
        }
        self.wire.subscribe(user_ids);
    }

    pub fn enter_chat_action(&self, chat_id: String) {
        if chat_id.is_empty() {
            return;
        }
        self.wire.enter_chat(chat_id);
    }

    pub fn leave_chat_action(&self, chat_id: String) {
        self.wire.leave_chat(chat_id);
    }

    pub fn set_away_action(&self, status: String) {
        self.wire.set_away(status);
    }
}
