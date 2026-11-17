use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use rise_canon::RiseWireRequestMetadata;
use rise_canon::rise_wire::{KEY_DATA, KEY_ID, KEY_REQUEST_ID};
use rise_canon::rise_wire_methods::notify;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_core::rise_identifiers::RiseRequestIDAllocator;
use crate::rise_wire::rise_json_value::RiseJSONValue;
use crate::rise_wire::rise_wire::{
    RiseWire, RiseWireAccountContext, RiseWireConfiguration, RiseWireError, RiseWireFrameSender,
    RiseWireSocketSession,
};
use crate::rise_wire::rise_wire_contracts::{RiseWireMethodDescriptor, RiseWireReplayPolicy};

/// counts sends and hands every frame to the test in order
struct StartupSender {
    sends: AtomicUsize,
    frames: mpsc::UnboundedSender<Vec<u8>>,
}

impl StartupSender {
    fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<Vec<u8>>) {
        let (frames, receiver) = mpsc::unbounded_channel();
        (Arc::new(Self { sends: AtomicUsize::new(0), frames }), receiver)
    }

    fn sends(&self) -> usize {
        self.sends.load(Ordering::SeqCst)
    }
}

impl RiseWireFrameSender for StartupSender {
    fn send(
        &self,
        frame: Vec<u8>,
        _session: RiseWireSocketSession,
    ) -> BoxFuture<'static, anyhow::Result<()>> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        let _ = self.frames.send(frame);
        async { Ok(()) }.boxed()
    }
}

fn account() -> RiseWireAccountContext {
    RiseWireAccountContext::new("startup-test".to_owned(), "first".to_owned())
}

fn read() -> RiseWireMethodDescriptor {
    RiseWireMethodDescriptor::new("test", "startup_read")
        .with_timeout(Duration::from_secs(2))
        .with_replay_policy(RiseWireReplayPolicy::ReadOnly)
}

fn wire(sender: Arc<StartupSender>, configuration: RiseWireConfiguration) -> Arc<RiseWire> {
    RiseWire::new(account(), sender, None, RiseRequestIDAllocator::shared(), configuration)
}

fn empty_object() -> RiseJSONValue {
    RiseJSONValue::Object(BTreeMap::new())
}

async fn call(wire: &RiseWire, descriptor: &RiseWireMethodDescriptor) -> anyhow::Result<RiseJSONValue> {
    wire.call_descriptor(descriptor, &empty_object(), None, RiseWireRequestMetadata::default())
        .await
}

fn spawn_call(
    wire: &Arc<RiseWire>,
    descriptor: RiseWireMethodDescriptor,
) -> JoinHandle<anyhow::Result<RiseJSONValue>> {
    let wire = wire.clone();
    RiseAccountRuntime::current().spawn(async move { call(&wire, &descriptor).await })
}

fn wire_error(result: anyhow::Result<RiseJSONValue>) -> Option<RiseWireError> {
    result.err().and_then(|error| error.downcast_ref::<RiseWireError>().cloned())
}

#[tokio::test]
async fn read_waits_for_connection_and_sends_exactly_once() {
    let (sender, mut frames) = StartupSender::new();
    let wire = wire(sender.clone(), RiseWireConfiguration::production());
    let call = spawn_call(&wire, read());
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(sender.sends(), 0);
    let session = RiseWireSocketSession::new(account(), 1);
    wire.socket_session_started(session.clone()).await;
    assert_eq!(sender.sends(), 0);
    wire.socket_connected(session.clone()).await;
    let sent = frames.recv().await.expect("the read is sent after connect");
    let frame = RiseJSONValue::decode(&sent).expect("the frame is JSON");
    let id = frame.subscript_key(KEY_ID).cloned().expect("the frame carries an id");
    let mut ready = BTreeMap::new();
    ready.insert("ready".to_owned(), RiseJSONValue::Bool(true));
    let mut response = BTreeMap::new();
    response.insert(KEY_REQUEST_ID.to_owned(), id);
    response.insert(KEY_DATA.to_owned(), RiseJSONValue::Object(ready));
    let response = RiseJSONValue::Object(response).data(false).expect("the response encodes");
    let _ = wire.accept(&response, &session).await;
    let answer = call.await.expect("call task finished").expect("the read is answered");
    assert_eq!(answer.subscript_key("ready"), Some(&RiseJSONValue::Bool(true)));
    assert_eq!(sender.sends(), 1);
    wire.shutdown().await;
}

#[tokio::test]
async fn mutation_does_not_become_an_implicit_deferred_registration() {
    let (sender, _frames) = StartupSender::new();
    let wire = wire(sender.clone(), RiseWireConfiguration::production());
    let mutation = RiseWireMethodDescriptor::new(notify::SERVICE, notify::REGISTER_DEVICE_TOKEN)
        .with_replay_policy(RiseWireReplayPolicy::Never);
    assert_eq!(wire_error(call(&wire, &mutation).await), Some(RiseWireError::NoSocketSession));
    assert_eq!(sender.sends(), 0);
    wire.shutdown().await;
}

#[tokio::test]
async fn cancelled_read_is_never_sent_after_connect() {
    let (sender, _frames) = StartupSender::new();
    let wire = wire(sender.clone(), RiseWireConfiguration::production());
    let call = spawn_call(&wire, read());
    tokio::time::sleep(Duration::from_millis(30)).await;
    call.abort();
    assert!(call.await.is_err_and(|error| error.is_cancelled()));
    let session = RiseWireSocketSession::new(account(), 2);
    wire.socket_session_started(session.clone()).await;
    wire.socket_connected(session).await;
    assert_eq!(sender.sends(), 0);
    wire.shutdown().await;
}

#[tokio::test]
async fn unsent_reads_share_the_pending_limit_and_release_it_on_cancellation() {
    let (sender, _frames) = StartupSender::new();
    let configuration =
        RiseWireConfiguration { maximum_pending_requests: 1, ..RiseWireConfiguration::production() };
    let wire = wire(sender.clone(), configuration);
    let first = spawn_call(&wire, read());
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(wire_error(call(&wire, &read()).await), Some(RiseWireError::PendingLimitReached(1)));
    first.abort();
    assert!(first.await.is_err_and(|error| error.is_cancelled()));
    let replacement = spawn_call(&wire, read());
    tokio::time::sleep(Duration::from_millis(30)).await;
    wire.shutdown().await;
    let replaced = replacement.await.expect("replacement task finished");
    assert_eq!(wire_error(replaced), Some(RiseWireError::Shutdown));
    assert_eq!(sender.sends(), 0);
}

#[tokio::test]
async fn connection_timeout_does_not_send_or_retry() {
    let (sender, _frames) = StartupSender::new();
    let wire = wire(sender.clone(), RiseWireConfiguration::production());
    let short_read = RiseWireMethodDescriptor::new("test", "offline")
        .with_timeout(Duration::from_millis(30))
        .with_replay_policy(RiseWireReplayPolicy::ReadOnly);
    assert_eq!(wire_error(call(&wire, &short_read).await), Some(RiseWireError::NotConnected));
    let session = RiseWireSocketSession::new(account(), 3);
    wire.socket_session_started(session.clone()).await;
    wire.socket_connected(session).await;
    assert_eq!(sender.sends(), 0);
    wire.shutdown().await;
}
