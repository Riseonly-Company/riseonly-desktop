use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use rise_canon::RiseWireRequestMetadata;
use rise_canon::rise_wire::{
    KEY_DATA, KEY_FRAME_TYPE, KEY_ID, KEY_METADATA, KEY_METHOD, KEY_SERVICE, KEY_TIMESTAMP,
    METADATA_KEY_AUTH_REQUIRED, METADATA_KEY_CORRELATION_ID, OUTBOUND_FRAME_TYPE,
};
use rise_canon::rise_wire_methods::message;
use rise_engine::rise_core::rise_identifiers::{RiseRequestID, RiseRequestIDAllocator};
use rise_engine::rise_wire::rise_json_value::RiseJSONValue;
use rise_engine::rise_wire::rise_wire::{
    RiseWire, RiseWireAccountContext, RiseWireClosureSender, RiseWireConfiguration,
    RiseWireEmptyResponse, RiseWireError, RiseWireEvent, RiseWireFrameSender,
    RiseWireInboundDisposition, RiseWireSocketSession,
};
use rise_engine::rise_wire::rise_wire_contracts::{
    RiseWireContractError, RiseWireIncomingFrame, RiseWireMethod, RiseWireMethodDescriptor,
    RiseWireReplayPolicy, RiseWireRequestEncoder,
};
use rise_engine::rise_wire::rise_wire_frame_decoder::RiseWireFrameDecoder;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify, mpsc};

type SentFrame = (Vec<u8>, RiseWireSocketSession);

struct RiseWireSenderProbe {
    frames: mpsc::UnboundedSender<SentFrame>,
    waiters: Mutex<mpsc::UnboundedReceiver<SentFrame>>,
}

impl RiseWireSenderProbe {
    fn new() -> Arc<Self> {
        let (frames, waiters) = mpsc::unbounded_channel();
        Arc::new(Self { frames, waiters: Mutex::new(waiters) })
    }

    async fn next(&self) -> SentFrame {
        self.waiters.lock().await.recv().await.expect("the probe outlives every sent frame")
    }
}

impl RiseWireFrameSender for RiseWireSenderProbe {
    fn send(
        &self,
        frame: Vec<u8>,
        session: RiseWireSocketSession,
    ) -> BoxFuture<'static, anyhow::Result<()>> {
        let frames = self.frames.clone();
        async move {
            let _ = frames.send((frame, session));
            anyhow::Ok(())
        }
        .boxed()
    }
}

struct RiseWireCancellableSenderProbeState {
    did_start: AtomicBool,
    did_cancel: AtomicBool,
    start_waiters: Notify,
}

struct RiseWireCancellableSenderProbe {
    state: Arc<RiseWireCancellableSenderProbeState>,
}

struct RiseWireCancellableSenderProbeCancellation {
    state: Arc<RiseWireCancellableSenderProbeState>,
    is_armed: bool,
}

impl Drop for RiseWireCancellableSenderProbeCancellation {
    fn drop(&mut self) {
        if self.is_armed {
            self.state.did_cancel.store(true, Ordering::SeqCst);
        }
    }
}

impl RiseWireCancellableSenderProbe {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(RiseWireCancellableSenderProbeState {
                did_start: AtomicBool::new(false),
                did_cancel: AtomicBool::new(false),
                start_waiters: Notify::new(),
            }),
        })
    }

    async fn wait_until_started(&self) {
        if self.state.did_start.load(Ordering::SeqCst) {
            return;
        }
        self.state.start_waiters.notified().await;
    }

    fn was_cancelled(&self) -> bool {
        self.state.did_cancel.load(Ordering::SeqCst)
    }
}

impl RiseWireFrameSender for RiseWireCancellableSenderProbe {
    fn send(
        &self,
        _frame: Vec<u8>,
        _session: RiseWireSocketSession,
    ) -> BoxFuture<'static, anyhow::Result<()>> {
        let state = Arc::clone(&self.state);
        async move {
            state.did_start.store(true, Ordering::SeqCst);
            state.start_waiters.notify_one();
            let mut cancellation =
                RiseWireCancellableSenderProbeCancellation { state, is_armed: true };
            tokio::time::sleep(Duration::from_secs(60)).await;
            cancellation.is_armed = false;
            anyhow::Ok(())
        }
        .boxed()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct WireEchoMethodRequest {
    message_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct WireEchoMethodResponse {
    echoed_text: String,
}

enum WireEchoMethod {}

impl RiseWireMethod for WireEchoMethod {
    type Request = WireEchoMethodRequest;
    type Response = WireEchoMethodResponse;

    fn descriptor() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new("test", "echo")
            .with_timeout(Duration::from_secs(2))
            .with_replay_policy(RiseWireReplayPolicy::ReadOnly)
    }
}

#[derive(Clone, Debug, Serialize)]
struct WireFastTimeoutMethodRequest {
    value: String,
}

enum WireFastTimeoutMethod {}

impl RiseWireMethod for WireFastTimeoutMethod {
    type Request = WireFastTimeoutMethodRequest;
    type Response = RiseWireEmptyResponse;

    fn descriptor() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new("test", "timeout").with_timeout(Duration::from_millis(20))
    }
}

enum WireArrayRequestMethod {}

impl RiseWireMethod for WireArrayRequestMethod {
    type Request = Vec<String>;
    type Response = RiseWireEmptyResponse;

    fn descriptor() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new("test", "array")
    }
}

fn account() -> RiseWireAccountContext {
    RiseWireAccountContext::new("account".to_owned(), "epoch-1".to_owned())
}

fn request_id(raw_value: u64) -> RiseRequestID {
    RiseRequestID::new(raw_value).expect("test request ids are above the canon minimum")
}

fn make_wire(
    sender: Arc<dyn RiseWireFrameSender>,
    configuration: RiseWireConfiguration,
) -> Arc<RiseWire> {
    RiseWire::new(account(), sender, None, RiseRequestIDAllocator::shared(), configuration)
}

fn wire_error(error: &anyhow::Error) -> Option<&RiseWireError> {
    error.downcast_ref::<RiseWireError>()
}

fn decode_root(data: &[u8]) -> BTreeMap<String, RiseJSONValue> {
    let value = RiseJSONValue::decode(data).expect("the encoder writes valid JSON");
    value.object_value().cloned().expect("the frame root is an object")
}

fn member<'a>(
    root: &'a BTreeMap<String, RiseJSONValue>,
    key: &str,
    nested: &str,
) -> Option<&'a RiseJSONValue> {
    root.get(key).and_then(|value| value.subscript_key(nested))
}

fn echo(
    wire: &Arc<RiseWire>,
    message_text: &str,
    id: RiseRequestID,
) -> tokio::task::JoinHandle<anyhow::Result<WireEchoMethodResponse>> {
    let wire = Arc::clone(wire);
    let request = WireEchoMethodRequest { message_text: message_text.to_owned() };
    tokio::spawn(async move {
        wire.call::<WireEchoMethod>(&request, Some(id), RiseWireRequestMetadata::default()).await
    })
}

async fn eventually(attempts: usize, condition: impl Fn() -> bool) {
    for _ in 0..attempts {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("Condition did not become true");
}

#[test]
fn request_codec_matches_gateway_and_rejects_non_object_bodies() {
    let data = RiseWireRequestEncoder::encode_method::<WireEchoMethod>(
        &WireEchoMethodRequest { message_text: "hello".to_owned() },
        request_id(42),
        123,
        RiseWireRequestMetadata::default(),
    )
    .expect("an object body encodes");
    let root = decode_root(&data);

    assert_eq!(root.get(KEY_ID), Some(&RiseJSONValue::Integer(42)));
    assert_eq!(
        root.get(KEY_FRAME_TYPE),
        Some(&RiseJSONValue::String(OUTBOUND_FRAME_TYPE.to_owned()))
    );
    assert_eq!(root.get(KEY_SERVICE), Some(&RiseJSONValue::String("test".to_owned())));
    assert_eq!(root.get(KEY_METHOD), Some(&RiseJSONValue::String("echo".to_owned())));
    assert_eq!(root.get(KEY_TIMESTAMP), Some(&RiseJSONValue::Integer(123)));
    assert_eq!(
        member(&root, KEY_DATA, "message_text"),
        Some(&RiseJSONValue::String("hello".to_owned()))
    );
    assert_eq!(
        member(&root, KEY_METADATA, METADATA_KEY_AUTH_REQUIRED),
        Some(&RiseJSONValue::Bool(false))
    );
    assert_eq!(
        member(&root, KEY_METADATA, METADATA_KEY_CORRELATION_ID),
        Some(&RiseJSONValue::Null)
    );

    let rejected = RiseWireRequestEncoder::encode_method::<WireArrayRequestMethod>(
        &vec!["invalid".to_owned()],
        request_id(43),
        123,
        RiseWireRequestMetadata::default(),
    )
    .expect_err("an array body is refused");
    assert_eq!(
        rejected.downcast_ref::<RiseWireContractError>(),
        Some(&RiseWireContractError::RequestDataMustBeObject)
    );
}

#[test]
fn attested_body_travels_as_the_signed_text() {
    let descriptor = RiseWireMethodDescriptor::new("test", "signed");
    let body = r#"{"signed_transaction":"a.b.c","consumption_consent":true}"#;
    let data = RiseWireRequestEncoder::encode(
        &descriptor,
        &RiseJSONValue::String(body.to_owned()),
        request_id(44),
        123,
        RiseWireRequestMetadata::default(),
    )
    .expect("an object text encodes");
    let root = decode_root(&data);
    assert_eq!(root.get(KEY_DATA), Some(&RiseJSONValue::String(body.to_owned())));

    let rejected = RiseWireRequestEncoder::encode(
        &descriptor,
        &RiseJSONValue::String(r#"["not","an","object"]"#.to_owned()),
        request_id(45),
        123,
        RiseWireRequestMetadata::default(),
    )
    .expect_err("an array text is refused");
    assert_eq!(
        rejected.downcast_ref::<RiseWireContractError>(),
        Some(&RiseWireContractError::RequestDataMustBeObject)
    );
}

#[test]
fn frame_decoder_keeps_legacy_gateway_compatibility() {
    let decoder = RiseWireFrameDecoder::default();
    let success = decoder.decode(br#"{"request_id":"42","result":{"value":1}}"#);
    let Ok(RiseWireIncomingFrame::Response(response)) = success else {
        panic!("Expected response");
    };
    assert_eq!(Some(response.request_id), RiseRequestID::new(42));
    assert!(response.status.is_success());
    assert_eq!(
        response.data.as_ref().and_then(|data| data.subscript_key("value")),
        Some(&RiseJSONValue::Integer(1))
    );

    let failure =
        decoder.decode(br#"{"request_id":43,"status":"Error","error":"denied","code":403}"#);
    let Ok(RiseWireIncomingFrame::Response(failed_response)) = failure else {
        panic!("Expected error response");
    };
    assert_eq!(failed_response.error.as_ref().map(|error| error.code.as_str()), Some("403"));
    assert_eq!(failed_response.error.as_ref().map(|error| error.message.as_str()), Some("denied"));

    let nested_push = decoder.decode(br#"{"data":{"type":"message.created","id":"m1"}}"#);
    let Ok(RiseWireIncomingFrame::Push(push)) = nested_push else {
        panic!("Expected push");
    };
    assert_eq!(push.r#type, "message.created");
    assert_eq!(push.data.subscript_key("id"), Some(&RiseJSONValue::String("m1".to_owned())));
}

#[tokio::test(flavor = "multi_thread")]
async fn typed_call_round_trips_through_injected_single_socket_sender() {
    let probe = RiseWireSenderProbe::new();
    let wire = make_wire(probe.clone(), RiseWireConfiguration::production());
    let session = RiseWireSocketSession::new(account(), 7);
    wire.socket_session_started(session.clone()).await;
    wire.socket_connected(session.clone()).await;

    let call = echo(&wire, "hello", request_id(700));
    let (outbound, sent_session) = probe.next().await;
    assert_eq!(sent_session, session);
    let root = decode_root(&outbound);
    assert_eq!(root.get(KEY_ID), Some(&RiseJSONValue::Integer(700)));
    assert_eq!(
        member(&root, KEY_DATA, "message_text"),
        Some(&RiseJSONValue::String("hello".to_owned()))
    );

    let disposition = wire
        .accept(
            br#"{"request_id":"700","status":"Success","data":{"echoed_text":"hello"}}"#,
            &session,
        )
        .await;
    assert_eq!(disposition, RiseWireInboundDisposition::ConsumedResponse);
    let response = call.await.expect("the call task is not aborted").expect("the call answers");
    assert_eq!(response, WireEchoMethodResponse { echoed_text: "hello".to_owned() });
    wire.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_call_uses_runtime_descriptor_without_losing_wire_identity() {
    let probe = RiseWireSenderProbe::new();
    let wire = make_wire(probe.clone(), RiseWireConfiguration::production());
    let session = RiseWireSocketSession::new(account(), 8);
    wire.socket_session_started(session.clone()).await;
    wire.socket_connected(session.clone()).await;

    let descriptor = RiseWireMethodDescriptor::new(message::SERVICE, message::GET_MESSAGES)
        .with_timeout(Duration::from_secs(2))
        .with_replay_policy(RiseWireReplayPolicy::ReadOnly);
    let request = RiseJSONValue::Object(BTreeMap::from([
        ("chat_id".to_owned(), RiseJSONValue::String("chat-1".to_owned())),
        ("limit".to_owned(), RiseJSONValue::Integer(100)),
    ]));
    let call = {
        let wire = Arc::clone(&wire);
        tokio::spawn(async move {
            wire.call_descriptor(
                &descriptor,
                &request,
                Some(request_id(701)),
                RiseWireRequestMetadata::default(),
            )
            .await
        })
    };
    let (outbound, sent_session) = probe.next().await;
    assert_eq!(sent_session, session);
    let root = decode_root(&outbound);
    assert_eq!(root.get(KEY_ID), Some(&RiseJSONValue::Integer(701)));
    assert_eq!(root.get(KEY_SERVICE), Some(&RiseJSONValue::String(message::SERVICE.to_owned())));
    assert_eq!(
        root.get(KEY_METHOD),
        Some(&RiseJSONValue::String(message::GET_MESSAGES.to_owned()))
    );
    assert_eq!(
        member(&root, KEY_DATA, "chat_id"),
        Some(&RiseJSONValue::String("chat-1".to_owned()))
    );

    let disposition = wire
        .accept(br#"{"request_id":701,"data":{"messages":[{"id":"m1"}]}}"#, &session)
        .await;
    assert_eq!(disposition, RiseWireInboundDisposition::ConsumedResponse);
    let value = call.await.expect("the call task is not aborted").expect("the call answers");
    let first_id = value
        .subscript_key("messages")
        .and_then(|messages| messages.subscript_index(0))
        .and_then(|first| first.subscript_key("id"));
    assert_eq!(first_id, Some(&RiseJSONValue::String("m1".to_owned())));
    wire.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_cancellation_and_disconnect_remove_only_exact_pending_attempt() {
    let probe = RiseWireSenderProbe::new();
    let wire = make_wire(probe.clone(), RiseWireConfiguration::production());
    let first_session = RiseWireSocketSession::new(account(), 1);
    wire.socket_session_started(first_session.clone()).await;
    wire.socket_connected(first_session.clone()).await;

    let timed_out = {
        let wire = Arc::clone(&wire);
        let request = WireFastTimeoutMethodRequest { value: "timeout".to_owned() };
        tokio::spawn(async move {
            wire.call::<WireFastTimeoutMethod>(
                &request,
                Some(request_id(800)),
                RiseWireRequestMetadata::default(),
            )
            .await
        })
    };
    let _ = probe.next().await;
    let error = timed_out
        .await
        .expect("the call task is not aborted")
        .expect_err("the call times out");
    assert_eq!(wire_error(&error), Some(&RiseWireError::Timeout(request_id(800))));

    let cancelled = echo(&wire, "cancel", request_id(801));
    let _ = probe.next().await;
    cancelled.abort();
    let outcome = cancelled.await;
    assert!(outcome.is_err_and(|error| error.is_cancelled()));

    let replacement = echo(&wire, "cancel", request_id(801));
    let _ = probe.next().await;
    let _ = wire
        .accept(br#"{"request_id":801,"data":{"echoed_text":"cancel"}}"#, &first_session)
        .await;
    let response =
        replacement.await.expect("the call task is not aborted").expect("the call answers");
    assert_eq!(response.echoed_text, "cancel");

    let disconnected = echo(&wire, "disconnect", request_id(802));
    let _ = probe.next().await;
    wire.socket_disconnected(first_session.clone(), "network lost".to_owned()).await;
    let error = disconnected
        .await
        .expect("the call task is not aborted")
        .expect_err("the call fails with the socket");
    assert_eq!(
        wire_error(&error),
        Some(&RiseWireError::Disconnected("network lost".to_owned()))
    );

    let second_session = RiseWireSocketSession::new(account(), 2);
    wire.socket_session_started(second_session.clone()).await;
    wire.socket_connected(second_session).await;
    let late_old_response =
        wire.accept(br#"{"request_id":802,"data":{}}"#, &first_session).await;
    assert_eq!(late_old_response, RiseWireInboundDisposition::Ignored);
    wire.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_stops_the_owned_send_task() {
    let probe = RiseWireCancellableSenderProbe::new();
    let wire = make_wire(probe.clone(), RiseWireConfiguration::production());
    let session = RiseWireSocketSession::new(account(), 3);
    wire.socket_session_started(session.clone()).await;
    wire.socket_connected(session).await;

    let call = echo(&wire, "cancel-before-wire-completes", request_id(803));
    probe.wait_until_started().await;
    call.abort();
    let outcome = call.await;
    assert!(outcome.is_err_and(|error| error.is_cancelled()));
    eventually(200, || probe.was_cancelled()).await;
    wire.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_request_id_reuse_requires_an_identical_replay_safe_call() {
    let probe = RiseWireSenderProbe::new();
    let wire = make_wire(probe.clone(), RiseWireConfiguration::production());
    let session = RiseWireSocketSession::new(account(), 4);
    wire.socket_session_started(session.clone()).await;
    wire.socket_connected(session.clone()).await;
    let id = request_id(804);

    for _ in 0..2 {
        let call = echo(&wire, "stable", id);
        let _ = probe.next().await;
        let _ = wire
            .accept(br#"{"request_id":804,"data":{"echoed_text":"stable"}}"#, &session)
            .await;
        let response = call.await.expect("the call task is not aborted").expect("the call answers");
        assert_eq!(response.echoed_text, "stable");
    }

    let error = wire
        .call::<WireEchoMethod>(
            &WireEchoMethodRequest { message_text: "changed".to_owned() },
            Some(id),
            RiseWireRequestMetadata::default(),
        )
        .await
        .expect_err("a changed body under the same id is refused");
    assert_eq!(wire_error(&error), Some(&RiseWireError::UnsafeRequestIDReuse(id)));
    wire.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn event_buffer_overflow_is_visible_to_sync_consumer() {
    let probe = RiseWireSenderProbe::new();
    let wire = make_wire(
        probe.clone(),
        RiseWireConfiguration { event_buffer_capacity: 1, ..RiseWireConfiguration::production() },
    );
    let session = RiseWireSocketSession::new(account(), 5);
    let mut events = wire.events().await.expect("the first subscriber fits the limit");
    wire.socket_session_started(session.clone()).await;
    wire.socket_connected(session.clone()).await;
    let _ = wire.accept(br#"{"type":"one","data":{"id":1}}"#, &session).await;
    let _ = wire.accept(br#"{"type":"two","data":{"id":2}}"#, &session).await;

    let Some(event) = events.next().await else {
        panic!("Expected overflow event");
    };
    match event {
        RiseWireEvent::EventBufferOverflow(overflow_session) => {
            assert_eq!(overflow_session, Some(session));
        }
        other => panic!("Expected explicit overflow, got {other:?}"),
    }
    wire.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn explicit_event_subscription_cancellation_releases_capacity() {
    let sender = RiseWireClosureSender::new(Arc::new(
        |_frame: Vec<u8>, _session: RiseWireSocketSession| async { anyhow::Ok(()) }.boxed(),
    ));
    let wire = make_wire(
        Arc::new(sender),
        RiseWireConfiguration { maximum_event_subscribers: 1, ..RiseWireConfiguration::production() },
    );

    let first = wire.event_subscription().await.expect("the first subscriber fits the limit");
    let refused = wire.event_subscription().await;
    assert!(matches!(refused, Err(RiseWireError::EventSubscriberLimitReached(1))));
    first.cancel().await;

    let replacement =
        wire.event_subscription().await.expect("the cancelled subscriber released its slot");
    replacement.cancel().await;
    wire.shutdown().await;
}
