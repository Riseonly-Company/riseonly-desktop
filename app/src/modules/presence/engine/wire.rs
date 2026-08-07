use std::sync::Arc;

use rise_engine::{MethodDescriptor, RiseWire};
use serde::Serialize;
use serde_json::Value;
use tokio::runtime::Handle;

use super::core::rise_presence_rpc as rpc;

/// The presence verbs, as something a store can hold.
///
/// Every one of them is fire-and-forget: the gateway answers a subscription with
/// an acknowledgement carrying no state, and the state itself arrives later as a
/// push. So there is nothing to await and nothing to reconcile — which is also
/// why presence has no actor repository, only a projection over the pushes.
pub trait PresenceWire: Send + Sync {
    fn subscribe(&self, user_ids: Vec<String>);
    fn unsubscribe(&self, user_ids: Vec<String>);
    fn enter_chat(&self, chat_id: String);
    fn leave_chat(&self, chat_id: String);
    fn set_away(&self, status: String);
}

pub struct LivePresenceWire {
    wire: Arc<RiseWire>,
    runtime: Handle,
}

impl LivePresenceWire {
    pub fn new(wire: Arc<RiseWire>, runtime: Handle) -> Self {
        Self { wire, runtime }
    }

    fn send<T: Serialize + Send + 'static>(&self, descriptor: &'static MethodDescriptor, body: T) {
        let wire = Arc::clone(&self.wire);
        self.runtime.spawn(async move {
            let payload = serde_json::to_value(body).unwrap_or(Value::Null);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_millis() as u64)
                .unwrap_or_default();

            // A failure here is not actionable: presence is decoration, and a
            // reconnect re-subscribes everything anyway. Logged rather than
            // surfaced, so a flaky network cannot put an error on a chat list.
            if let Err(error) = wire.call(descriptor, payload, now).await {
                tracing::debug!(
                    target: "riseonly::presence",
                    "{} failed: {error}",
                    descriptor.qualified_name()
                );
            }
        });
    }
}

impl PresenceWire for LivePresenceWire {
    fn subscribe(&self, user_ids: Vec<String>) {
        self.send(&rpc::SUBSCRIBE, rpc::SubscribeRequest { users: user_ids });
    }

    fn unsubscribe(&self, user_ids: Vec<String>) {
        self.send(&rpc::UNSUBSCRIBE, rpc::SubscribeRequest { users: user_ids });
    }

    fn enter_chat(&self, chat_id: String) {
        self.send(&rpc::ENTER_CHAT, rpc::ChatPresenceRequest { chat_id });
    }

    fn leave_chat(&self, chat_id: String) {
        self.send(&rpc::LEAVE_CHAT, rpc::ChatPresenceRequest { chat_id });
    }

    fn set_away(&self, status: String) {
        self.send(&rpc::SET_AWAY, rpc::SetAwayRequest { status });
    }
}
