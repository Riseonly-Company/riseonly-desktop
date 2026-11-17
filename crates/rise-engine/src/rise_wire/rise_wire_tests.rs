use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use rise_canon::RiseWireRequestMetadata;
use rise_canon::rise_wire::{
    KEY_CODE, KEY_DATA, KEY_ERROR, KEY_FRAME_TYPE, KEY_ID, KEY_MESSAGE, KEY_METHOD,
    KEY_REQUEST_ID, KEY_SERVICE, KEY_STATUS, KEY_TIMESTAMP, OUTBOUND_FRAME_TYPE, STATUS_ERROR,
    STATUS_SUCCESS,
};
use rise_canon::rise_wire_methods::map;

use crate::rise_core::rise_identifiers::RiseRequestIDAllocator;
use crate::rise_wire::rise_json_value::RiseJSONValue;
use crate::rise_wire::rise_wire::{
    RiseWire, RiseWireAccountContext, RiseWireConfiguration, RiseWireError, RiseWireFrameSender,
    RiseWireHttpSender, RiseWireSocketSession,
};
use crate::rise_wire::rise_wire_contracts::{
    RiseWireContractError, RiseWireMethodDescriptor, RiseWireRemoteError, RiseWireTransport,
};

#[derive(Default)]
struct NoSocketSender {
    sends: AtomicUsize,
}

impl NoSocketSender {
    fn sends(&self) -> usize {
        self.sends.load(Ordering::SeqCst)
    }
}

impl RiseWireFrameSender for NoSocketSender {
    fn send(
        &self,
        _frame: Vec<u8>,
        _session: RiseWireSocketSession,
    ) -> BoxFuture<'static, anyhow::Result<()>> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        async { Ok(()) }.boxed()
    }
}

type GatewayReply = Arc<dyn Fn(RiseJSONValue) -> anyhow::Result<RiseJSONValue> + Send + Sync>;

/// answers every frame with reply(frame) like the gateway POST /api/rpc does
struct GatewayOverHttp {
    frames: Mutex<Vec<Vec<u8>>>,
    reply: GatewayReply,
}

impl GatewayOverHttp {
    fn new(
        reply: impl Fn(RiseJSONValue) -> anyhow::Result<RiseJSONValue> + Send + Sync + 'static,
    ) -> Self {
        Self { frames: Mutex::new(Vec::new()), reply: Arc::new(reply) }
    }

    fn frames(&self) -> Vec<Vec<u8>> {
        self.frames.lock().clone()
    }
}

impl RiseWireHttpSender for GatewayOverHttp {
    fn post(
        &self,
        frame: Vec<u8>,
        _account: RiseWireAccountContext,
        _timeout: Duration,
    ) -> BoxFuture<'static, anyhow::Result<Vec<u8>>> {
        self.frames.lock().push(frame.clone());
        let reply = self.reply.clone();
        async move {
            let answer = reply(RiseJSONValue::decode(&frame)?)?;
            Ok(answer.data(false)?)
        }
        .boxed()
    }
}

fn object(entries: Vec<(&str, RiseJSONValue)>) -> RiseJSONValue {
    RiseJSONValue::Object(
        entries.into_iter().map(|(key, value)| (key.to_owned(), value)).collect::<BTreeMap<_, _>>(),
    )
}

fn field(value: &RiseJSONValue, key: &str) -> RiseJSONValue {
    value.subscript_key(key).cloned().unwrap_or(RiseJSONValue::Null)
}

fn account() -> RiseWireAccountContext {
    RiseWireAccountContext::new("http-test".to_owned(), "first".to_owned())
}

fn report() -> RiseWireMethodDescriptor {
    RiseWireMethodDescriptor::new(map::SERVICE, map::UPDATE_LOCATION)
        .with_timeout(Duration::from_secs(2))
        .with_survives_suspension(true)
        .with_transport(RiseWireTransport::Http)
}

fn wire(
    sender: Arc<dyn RiseWireFrameSender>,
    http_sender: Option<Arc<dyn RiseWireHttpSender>>,
) -> Arc<RiseWire> {
    RiseWire::new(
        account(),
        sender,
        http_sender,
        RiseRequestIDAllocator::shared(),
        RiseWireConfiguration::production(),
    )
}

async fn call(wire: &RiseWire, descriptor: &RiseWireMethodDescriptor) -> anyhow::Result<RiseJSONValue> {
    wire.call_descriptor(descriptor, &object(vec![]), None, RiseWireRequestMetadata::default())
        .await
}

#[tokio::test]
async fn an_http_call_needs_no_socket_and_returns_the_frames_data() {
    let socket = Arc::new(NoSocketSender::default());
    let gateway = Arc::new(GatewayOverHttp::new(|frame| {
        Ok(object(vec![
            (KEY_REQUEST_ID, field(&frame, KEY_ID)),
            (KEY_STATUS, RiseJSONValue::String(STATUS_SUCCESS.to_owned())),
            (
                KEY_DATA,
                object(vec![
                    ("watched", RiseJSONValue::Bool(true)),
                    ("next_update_in_ms", RiseJSONValue::Integer(5000)),
                ]),
            ),
            (KEY_TIMESTAMP, RiseJSONValue::Integer(1)),
            (KEY_ERROR, RiseJSONValue::Null),
            (KEY_FRAME_TYPE, RiseJSONValue::String("map-update_location".to_owned())),
        ]))
    }));
    let http_sender: Arc<dyn RiseWireHttpSender> = gateway.clone();
    let wire = wire(socket.clone(), Some(http_sender));
    let report = report();
    assert!(wire.can_call(&report).await);
    let request = object(vec![("fix", object(vec![("lat", RiseJSONValue::Number(51.1))]))]);
    let answer = wire
        .call_descriptor(&report, &request, None, RiseWireRequestMetadata::default())
        .await
        .expect("the gateway answers");
    assert_eq!(field(&answer, "watched"), RiseJSONValue::Bool(true));
    assert_eq!(field(&answer, "next_update_in_ms"), RiseJSONValue::Integer(5000));
    assert_eq!(socket.sends(), 0);
    let frames = gateway.frames();
    let first = frames.first().expect("one frame was posted");
    let sent = RiseJSONValue::decode(first).expect("the posted frame is JSON");
    assert_eq!(field(&sent, KEY_SERVICE), RiseJSONValue::String(map::SERVICE.to_owned()));
    assert_eq!(field(&sent, KEY_METHOD), RiseJSONValue::String(map::UPDATE_LOCATION.to_owned()));
    assert_eq!(field(&sent, KEY_FRAME_TYPE), RiseJSONValue::String(OUTBOUND_FRAME_TYPE.to_owned()));
    assert_eq!(field(&field(&field(&sent, KEY_DATA), "fix"), "lat"), RiseJSONValue::Number(51.1));
    wire.shutdown().await;
}

#[tokio::test]
async fn a_refusal_frame_is_the_remote_error() {
    let gateway = Arc::new(GatewayOverHttp::new(|frame| {
        Ok(object(vec![
            (KEY_REQUEST_ID, field(&frame, KEY_ID)),
            (KEY_STATUS, RiseJSONValue::String(STATUS_ERROR.to_owned())),
            (KEY_DATA, RiseJSONValue::Null),
            (
                KEY_ERROR,
                object(vec![
                    (KEY_CODE, RiseJSONValue::String("RATE_LIMITED".to_owned())),
                    (
                        KEY_MESSAGE,
                        RiseJSONValue::String("Rate limit exceeded, retry after 3s".to_owned()),
                    ),
                ]),
            ),
        ]))
    }));
    let wire = wire(Arc::new(NoSocketSender::default()), Some(gateway as Arc<dyn RiseWireHttpSender>));
    let error = call(&wire, &report()).await.expect_err("the gateway refuses");
    assert!(error.downcast_ref::<RiseWireRemoteError>().is_some());
    wire.shutdown().await;
}

#[tokio::test]
async fn an_answer_to_another_request_is_refused() {
    let gateway = Arc::new(GatewayOverHttp::new(|_| {
        Ok(object(vec![
            (KEY_REQUEST_ID, RiseJSONValue::Integer(1)),
            (KEY_STATUS, RiseJSONValue::String(STATUS_SUCCESS.to_owned())),
            (KEY_DATA, object(vec![])),
        ]))
    }));
    let wire = wire(Arc::new(NoSocketSender::default()), Some(gateway as Arc<dyn RiseWireHttpSender>));
    let error = call(&wire, &report()).await.expect_err("the answer is for another id");
    assert_eq!(
        error.downcast_ref::<RiseWireContractError>(),
        Some(&RiseWireContractError::InvalidFrame)
    );
    wire.shutdown().await;
}

#[tokio::test]
async fn a_socket_descriptor_still_waits_for_the_socket() {
    let gateway = Arc::new(GatewayOverHttp::new(|_| Ok(object(vec![]))));
    let http_sender: Arc<dyn RiseWireHttpSender> = gateway.clone();
    let wire = wire(Arc::new(NoSocketSender::default()), Some(http_sender));
    let via_socket = RiseWireMethodDescriptor::new(map::SERVICE, map::OPEN_SESSION);
    assert!(!wire.can_call(&via_socket).await);
    let error = call(&wire, &via_socket).await.expect_err("no socket session");
    assert_eq!(error.downcast_ref::<RiseWireError>(), Some(&RiseWireError::NoSocketSession));
    assert!(gateway.frames().is_empty());
    wire.shutdown().await;
}

#[tokio::test]
async fn without_an_http_sender_the_call_falls_back_to_the_socket() {
    let wire = wire(Arc::new(NoSocketSender::default()), None);
    let report = report();
    assert!(!wire.can_call(&report).await);
    let error = call(&wire, &report).await.expect_err("no socket session");
    assert_eq!(error.downcast_ref::<RiseWireError>(), Some(&RiseWireError::NoSocketSession));
    wire.shutdown().await;
}
