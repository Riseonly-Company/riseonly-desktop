use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::oneshot;

use rise_core::{RequestId, RequestIdAllocator};

use super::rise_wire_contracts::{MethodDescriptor, RemoteError, RequestEnvelope, ResponseStatus};
use super::rise_wire_frame_decoder::{IncomingFrame, decode_frame};

#[derive(Debug, Error)]
pub enum WireError {
    #[error("not connected")]
    NotConnected,
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("remote: {}", .0.message)]
    Remote(RemoteError),
    #[error("encode: {0}")]
    Encode(String),
    #[error("decode: {0}")]
    Decode(String),
    #[error("the socket dropped before the response arrived")]
    Disconnected,
}

/// Whatever actually puts bytes on a socket.
///
/// The engine deliberately does not own the socket: the reference keeps
/// reconnect, backoff and auth in the host, and hands the engine a sender plus
/// a stream of inbound frames. That is also what makes every path below
/// testable without a network.
pub trait FrameSender: Send + Sync {
    fn send(&self, frame: String) -> Result<(), WireError>;
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RemoteError>>>>>;

/// Correlates requests with responses and dispatches pushes.
pub struct RiseWire {
    sender: Arc<dyn FrameSender>,
    requests: Arc<RequestIdAllocator>,
    pending: Pending,
    pushes: tokio::sync::broadcast::Sender<PushEvent>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct PushEvent {
    pub event_type: String,
    pub payload: Value,
}

impl RiseWire {
    pub fn new(sender: Arc<dyn FrameSender>, requests: Arc<RequestIdAllocator>) -> Self {
        let (pushes, _) = tokio::sync::broadcast::channel(64);
        Self {
            sender,
            requests,
            pending: Arc::new(Mutex::new(HashMap::new())),
            pushes,
        }
    }

    pub fn subscribe_pushes(&self) -> tokio::sync::broadcast::Receiver<PushEvent> {
        self.pushes.subscribe()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub async fn call<T: Serialize>(
        &self,
        descriptor: &MethodDescriptor,
        request: T,
        now_ms: u64,
    ) -> Result<Value, WireError> {
        let id = self.requests.next();
        let envelope = RequestEnvelope::new(id, descriptor, request, now_ms);
        let frame =
            serde_json::to_string(&envelope).map_err(|e| WireError::Encode(e.to_string()))?;

        let (respond, response) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.get(), respond);

        if let Err(error) = self.sender.send(frame) {
            self.forget(id);
            return Err(error);
        }

        match tokio::time::timeout(descriptor.timeout, response).await {
            Ok(Ok(Ok(payload))) => Ok(payload),
            Ok(Ok(Err(remote))) => Err(WireError::Remote(remote)),
            Ok(Err(_)) => Err(WireError::Disconnected),
            Err(_) => {
                self.forget(id);
                Err(WireError::Timeout(descriptor.timeout))
            }
        }
    }

    /// Feeds one inbound frame in. Unknown correlation ids are dropped rather
    /// than treated as errors: a response can outlive a timed-out request, and
    /// that is normal rather than exceptional.
    pub fn accept(&self, raw: &str) {
        let Ok(root) = serde_json::from_str::<Value>(raw) else {
            return;
        };

        match decode_frame(&root) {
            IncomingFrame::Response {
                request_id,
                status,
                payload,
                error,
            } => {
                let Some(respond) = self.take(request_id) else {
                    return;
                };
                let result = match (status, error) {
                    (ResponseStatus::Error, Some(remote)) => Err(remote),
                    (ResponseStatus::Error, None) => Err(RemoteError {
                        code: None,
                        message: "the server reported an error with no detail".into(),
                    }),
                    (ResponseStatus::Success, _) => Ok(payload),
                };
                let _ = respond.send(result);
            }

            IncomingFrame::Push {
                event_type,
                payload,
            } => {
                let _ = self.pushes.send(PushEvent {
                    event_type,
                    payload,
                });
            }

            IncomingFrame::GatewayError(_) | IncomingFrame::Unrecognized => {}
        }
    }

    /// Fails every in-flight request. Called when the socket drops, so a caller
    /// awaiting a response learns immediately instead of waiting for a timeout.
    pub fn fail_all_pending(&self) {
        let taken: Vec<_> = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .collect();

        for (_, respond) in taken {
            let _ = respond.send(Err(RemoteError {
                code: Some("disconnected".into()),
                message: "the socket dropped".into(),
            }));
        }
    }

    fn take(&self, id: RequestId) -> Option<oneshot::Sender<Result<Value, RemoteError>>> {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id.get())
    }

    fn forget(&self, id: RequestId) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id.get());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct RecordingSender {
        sent: Mutex<Vec<String>>,
        fail: bool,
    }

    impl RecordingSender {
        fn frames(&self) -> Vec<String> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl FrameSender for RecordingSender {
        fn send(&self, frame: String) -> Result<(), WireError> {
            if self.fail {
                return Err(WireError::NotConnected);
            }
            self.sent.lock().unwrap().push(frame);
            Ok(())
        }
    }

    fn wire(sender: Arc<RecordingSender>) -> RiseWire {
        RiseWire::new(sender, Arc::new(RequestIdAllocator::new()))
    }

    fn sent_id(sender: &RecordingSender) -> u64 {
        let frame = &sender.frames()[0];
        serde_json::from_str::<Value>(frame).unwrap()["id"]
            .as_u64()
            .unwrap()
    }

    const READ: MethodDescriptor = MethodDescriptor::read("auth", "sign_in");

    #[tokio::test]
    async fn a_call_sends_an_envelope_and_resolves_on_its_response() {
        let sender = Arc::new(RecordingSender::default());
        let wire = Arc::new(wire(Arc::clone(&sender)));

        let caller = {
            let wire = Arc::clone(&wire);
            tokio::spawn(async move { wire.call(&READ, json!({"login": "a"}), 1).await })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        let id = sent_id(&sender);
        wire.accept(&json!({"request_id": id, "data": {"token": "t"}}).to_string());

        let payload = caller.await.unwrap().unwrap();
        assert_eq!(payload["token"], "t");
        assert_eq!(wire.pending_count(), 0);
    }

    #[tokio::test]
    async fn a_server_error_surfaces_as_a_remote_error() {
        let sender = Arc::new(RecordingSender::default());
        let wire = Arc::new(wire(Arc::clone(&sender)));

        let caller = {
            let wire = Arc::clone(&wire);
            tokio::spawn(async move { wire.call(&READ, json!({}), 1).await })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        let id = sent_id(&sender);
        wire.accept(
            &json!({"request_id": id, "error": {"code": "AUTH_ERROR", "message": "bad"}})
                .to_string(),
        );

        match caller.await.unwrap() {
            Err(WireError::Remote(remote)) => {
                assert_eq!(remote.code.as_deref(), Some("AUTH_ERROR"));
            }
            other => panic!("expected a remote error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_failed_send_does_not_leak_a_pending_entry() {
        let sender = Arc::new(RecordingSender {
            fail: true,
            ..Default::default()
        });
        let wire = wire(sender);

        assert!(wire.call(&READ, json!({}), 1).await.is_err());
        assert_eq!(
            wire.pending_count(),
            0,
            "a request that never left must not occupy a correlation slot"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_times_out_and_releases_its_slot() {
        let sender = Arc::new(RecordingSender::default());
        let wire = wire(sender);

        let descriptor =
            MethodDescriptor::read("auth", "slow").with_timeout(Duration::from_secs(5));
        let result = wire.call(&descriptor, json!({}), 1).await;

        assert!(matches!(result, Err(WireError::Timeout(_))));
        assert_eq!(wire.pending_count(), 0);
    }

    #[tokio::test]
    async fn a_response_for_an_unknown_request_is_ignored_rather_than_fatal() {
        let sender = Arc::new(RecordingSender::default());
        let wire = wire(sender);

        wire.accept(&json!({"request_id": 999_999, "data": {}}).to_string());
        assert_eq!(wire.pending_count(), 0);
    }

    #[tokio::test]
    async fn a_push_reaches_subscribers_without_a_correlation_id() {
        let sender = Arc::new(RecordingSender::default());
        let wire = wire(sender);
        let mut pushes = wire.subscribe_pushes();

        wire.accept(&json!({"type": "message_created", "data": {"id": "m1"}}).to_string());

        let event = pushes.recv().await.unwrap();
        assert_eq!(event.event_type, "message_created");
        assert_eq!(event.payload["id"], "m1");
    }

    #[tokio::test]
    async fn malformed_json_is_dropped_rather_than_panicking() {
        let sender = Arc::new(RecordingSender::default());
        let wire = wire(sender);
        wire.accept("this is not json");
        wire.accept("");
    }

    #[tokio::test]
    async fn a_dropped_socket_fails_every_in_flight_request_immediately() {
        let sender = Arc::new(RecordingSender::default());
        let wire = Arc::new(wire(Arc::clone(&sender)));

        let caller = {
            let wire = Arc::clone(&wire);
            tokio::spawn(async move { wire.call(&READ, json!({}), 1).await })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(wire.pending_count(), 1);
        wire.fail_all_pending();

        let result = caller.await.unwrap();
        assert!(
            matches!(result, Err(WireError::Remote(_))),
            "a caller must not wait out the timeout after the socket dropped"
        );
        assert_eq!(wire.pending_count(), 0);
    }

    #[tokio::test]
    async fn concurrent_calls_do_not_cross_their_responses() {
        let sender = Arc::new(RecordingSender::default());
        let wire = Arc::new(wire(Arc::clone(&sender)));

        let first = {
            let wire = Arc::clone(&wire);
            tokio::spawn(async move { wire.call(&READ, json!({"n": 1}), 1).await })
        };
        let second = {
            let wire = Arc::clone(&wire);
            tokio::spawn(async move { wire.call(&READ, json!({"n": 2}), 2).await })
        };

        tokio::time::sleep(Duration::from_millis(30)).await;
        let frames = sender.frames();
        assert_eq!(frames.len(), 2);

        let ids: Vec<u64> = frames
            .iter()
            .map(|f| {
                serde_json::from_str::<Value>(f).unwrap()["id"]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        let payloads: Vec<i64> = frames
            .iter()
            .map(|f| {
                serde_json::from_str::<Value>(f).unwrap()["data"]["n"]
                    .as_i64()
                    .unwrap()
            })
            .collect();

        // Answer them in reverse order to prove correlation is by id, not arrival.
        wire.accept(&json!({"request_id": ids[1], "data": {"echo": payloads[1]}}).to_string());
        wire.accept(&json!({"request_id": ids[0], "data": {"echo": payloads[0]}}).to_string());

        assert_eq!(first.await.unwrap().unwrap()["echo"], payloads[0]);
        assert_eq!(second.await.unwrap().unwrap()["echo"], payloads[1]);
    }
}
