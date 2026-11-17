use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use parking_lot::{Condvar, Mutex};
use rise_canon::rise_store::IdentifierKind;
use rise_canon::rise_sync::{FailureCode as RiseSyncFailureCode, PUSH_BATCH_WINDOW_MS};
use rise_canon::rise_wire::{
    KEY_CODE, KEY_DATA, KEY_ERROR, KEY_FRAME_TYPE, KEY_ID, KEY_MESSAGE, KEY_REQUEST_ID,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::FileManager;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, oneshot};

use super::rise_sync::{
    RiseDurableLifecycleEvent, RiseDurableLifecycleResult, RiseDurableLifecycleStreamTermination,
    RiseSync, RiseSyncConfiguration, RiseSyncDurableContext, RiseSyncDurableEndpoint,
    RiseSyncDurableFailure, RiseSyncDurableFailureDisposition, RiseSyncError,
    RiseSyncPushHandler, RiseSyncSubmitOptions,
};
use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_core::rise_identifiers::{RiseRequestID, RiseRequestIDAllocator};
use crate::rise_core::rise_secure_key_store::{
    RiseEngineKeyStore, RiseEngineKeyStoreError, RiseEngineKeyStoreFuture,
};
use crate::rise_engine::{
    RiseEngine, RiseEngineConfiguration, RiseEngineConfigurationArgs, RiseEngineError,
};
use crate::rise_store::rise_store::{
    RiseEngineClock, RiseStore, RiseStoreConfiguration, RiseSystemClock,
};
use crate::rise_store::rise_store_cipher::RisePlaintextStoreCipher;
use crate::rise_store::rise_store_models::{
    RiseDurableOperationState, RiseDurableOrderingKey, RiseDurableTerminalResolution,
    RiseStoreCommit, RiseStoreLimits, RiseStoreMutation, RiseStoreNamespace,
    RiseStoreSnapshotRequest, RiseStoreValueKey, RiseStoreVersionPrecondition,
    RiseStoreWriteBatch, RiseStoredValue, RiseStoredValueWritePolicy,
};
use crate::rise_wire::rise_json_value::RiseJSONValue;
use crate::rise_wire::rise_wire::{
    CancellationError, RiseWire, RiseWireAccountContext, RiseWireClosureSender,
    RiseWireConfiguration, RiseWireFrameSender, RiseWireSocketSession,
};
use crate::rise_wire::rise_wire_contracts::{
    RiseWireMethod, RiseWireMethodDescriptor, RiseWirePushEnvelope, RiseWireReplayPolicy,
};

const TEST_SERVICE: &str = "sync-test";
const IDEMPOTENCY_FIELD: &str = "idempotency_key";
const SYNC_NAMESPACE: &str = "sync";
const ENGINE_DATABASE_FILE_NAME: &str = "engine.sqlite";
const EPOCH: &str = "epoch";
const DURABLE_LIFECYCLE_SUBSCRIBER_LIMIT: usize = 8;
const DURABLE_LIFECYCLE_BUFFER_SIZE: usize = 64;

fn sync_namespace() -> RiseStoreNamespace {
    RiseStoreNamespace::new(SYNC_NAMESPACE)
}

fn set_value(
    namespace: &RiseStoreNamespace,
    key: impl Into<String>,
    data: &[u8],
    version: i64,
    updated_at_milliseconds: i64,
    policy: RiseStoredValueWritePolicy,
) -> RiseStoreMutation {
    RiseStoreMutation::SetValue {
        namespace: namespace.clone(),
        key: key.into(),
        value: RiseStoredValue::new(data.to_vec(), version, updated_at_milliseconds),
        precondition: RiseStoreVersionPrecondition::Any,
        policy,
    }
}

fn plain_value(namespace: &RiseStoreNamespace, key: impl Into<String>, data: &[u8]) -> RiseStoreMutation {
    set_value(namespace, key, data, 1, 1, RiseStoredValueWritePolicy::RequireIdenticalReplay)
}

fn batch(mutations: Vec<RiseStoreMutation>) -> RiseStoreWriteBatch {
    RiseStoreWriteBatch::new(mutations, HashSet::new())
}

#[derive(Default)]
struct RiseMemoryKeyStore {
    values: Mutex<HashMap<String, Vec<u8>>>,
}

impl RiseEngineKeyStore for RiseMemoryKeyStore {
    fn key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Option<Vec<u8>>> {
        let value = self.values.lock().get(account_id).cloned();
        Box::pin(async move { Ok(value) })
    }

    fn create_key_if_absent<'a>(
        &'a self,
        account_id: &'a str,
    ) -> RiseEngineKeyStoreFuture<'a, Vec<u8>> {
        let mut values = self.values.lock();
        let value = match values.get(account_id) {
            Some(value) => value.clone(),
            None => {
                let fill = u8::try_from(values.len() + 1).unwrap_or(u8::MAX);
                let value = vec![fill; 32];
                values.insert(account_id.to_owned(), value.clone());
                value
            }
        };
        Box::pin(async move { Ok(value) })
    }

    fn delete_key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, ()> {
        self.values.lock().remove(account_id);
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RiseDeleteProbeError {
    Expected,
}

impl From<RiseDeleteProbeError> for RiseEngineKeyStoreError {
    fn from(error: RiseDeleteProbeError) -> Self {
        RiseEngineKeyStoreError::RandomGeneration(format!("{error:?}"))
    }
}

fn delete_probe_error() -> RiseEngineKeyStoreError {
    RiseDeleteProbeError::Expected.into()
}

#[derive(Default)]
struct RiseFailingDeleteKeyStore {
    delete_was_attempted: AtomicBool,
}

impl RiseFailingDeleteKeyStore {
    fn attempted_delete(&self) -> bool {
        self.delete_was_attempted.load(Ordering::SeqCst)
    }
}

impl RiseEngineKeyStore for RiseFailingDeleteKeyStore {
    fn key<'a>(&'a self, _account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Option<Vec<u8>>> {
        Box::pin(async { Ok(None) })
    }

    fn create_key_if_absent<'a>(
        &'a self,
        _account_id: &'a str,
    ) -> RiseEngineKeyStoreFuture<'a, Vec<u8>> {
        Box::pin(async { Ok(vec![1; 32]) })
    }

    fn delete_key<'a>(&'a self, _account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, ()> {
        self.delete_was_attempted.store(true, Ordering::SeqCst);
        Box::pin(async { Err(delete_probe_error()) })
    }
}

#[derive(Default)]
struct RiseSyncSenderProbe {
    frames: Mutex<VecDeque<(Vec<u8>, RiseWireSocketSession)>>,
}

impl RiseSyncSenderProbe {
    fn has_frame(&self) -> bool {
        !self.frames.lock().is_empty()
    }

    fn pop(&self) -> Option<(Vec<u8>, RiseWireSocketSession)> {
        self.frames.lock().pop_front()
    }
}

impl RiseWireFrameSender for RiseSyncSenderProbe {
    fn send(
        &self,
        frame: Vec<u8>,
        session: RiseWireSocketSession,
    ) -> BoxFuture<'static, anyhow::Result<()>> {
        self.frames.lock().push_back((frame, session));
        async { Ok(()) }.boxed()
    }
}

fn silent_sender() -> Arc<dyn RiseWireFrameSender> {
    Arc::new(RiseWireClosureSender::new(Arc::new(
        |_frame: Vec<u8>, _session: RiseWireSocketSession| -> BoxFuture<'static, anyhow::Result<()>> {
            async { Ok(()) }.boxed()
        },
    )))
}

#[derive(Default)]
struct RiseBlockingClockState {
    consumed_block: bool,
    did_enter: bool,
    released: bool,
}

#[derive(Default)]
struct RiseBlockingClock {
    state: Mutex<RiseBlockingClockState>,
    release_signal: Condvar,
    entered: Notify,
}

impl RiseBlockingClock {
    async fn wait_until_entered(&self) {
        loop {
            let notified = self.entered.notified();
            if self.state.lock().did_enter {
                return;
            }
            notified.await;
        }
    }

    fn release(&self) {
        self.state.lock().released = true;
        self.release_signal.notify_all();
    }
}

impl RiseEngineClock for RiseBlockingClock {
    fn now_milliseconds(&self) -> i64 {
        let mut state = self.state.lock();
        if !state.consumed_block {
            state.consumed_block = true;
            state.did_enter = true;
            self.entered.notify_one();
            while !state.released {
                self.release_signal.wait(&mut state);
            }
        }
        1
    }
}

#[derive(Default)]
struct RiseCompletionFlag {
    completed: AtomicBool,
}

impl RiseCompletionFlag {
    fn mark_completed(&self) {
        self.completed.store(true, Ordering::SeqCst);
    }

    fn is_completed(&self) -> bool {
        self.completed.load(Ordering::SeqCst)
    }
}

#[derive(Debug, thiserror::Error)]
enum RiseSyncTestTimeout {
    #[error("Condition did not become true")]
    TimedOut,
}

struct RiseMutableClock {
    value: AtomicI64,
}

impl RiseMutableClock {
    fn new(now_milliseconds: i64) -> Self {
        Self { value: AtomicI64::new(now_milliseconds) }
    }

    fn advance(&self, milliseconds: i64) {
        assert!(milliseconds >= 0);
        self.value.fetch_add(milliseconds, Ordering::SeqCst);
    }
}

impl RiseEngineClock for RiseMutableClock {
    fn now_milliseconds(&self) -> i64 {
        self.value.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct RisePushBatchSleepGate {
    requested: Mutex<Vec<Duration>>,
    sleepers: Mutex<Vec<oneshot::Sender<()>>>,
}

impl RisePushBatchSleepGate {
    fn sleep(&self, duration: Duration) -> oneshot::Receiver<()> {
        self.requested.lock().push(duration);
        let (sleeper, woken) = oneshot::channel();
        self.sleepers.lock().push(sleeper);
        woken
    }

    fn requested_durations(&self) -> Vec<Duration> {
        self.requested.lock().clone()
    }

    fn release(&self) {
        let sleepers = std::mem::take(&mut *self.sleepers.lock());
        for sleeper in sleepers {
            let _ = sleeper.send(());
        }
    }
}

#[derive(Default)]
struct RiseAsyncPushReducerGate {
    values: Mutex<Vec<String>>,
    first_call_waiters: Mutex<Vec<oneshot::Sender<()>>>,
}

impl RiseAsyncPushReducerGate {
    async fn enter(&self, value: String) {
        let waiter = {
            let mut values = self.values.lock();
            values.push(value);
            if values.len() != 1 {
                return;
            }
            let (waiter, released) = oneshot::channel();
            self.first_call_waiters.lock().push(waiter);
            released
        };
        let _ = waiter.await;
    }

    fn received_values(&self) -> Vec<String> {
        self.values.lock().clone()
    }

    fn release_first_call(&self) {
        let waiters = std::mem::take(&mut *self.first_call_waiters.lock());
        for waiter in waiters {
            let _ = waiter.send(());
        }
    }
}

#[derive(Default)]
struct RiseSyncResyncProbe {
    values: Mutex<Vec<String>>,
}

impl RiseSyncResyncProbe {
    fn record(&self, reason: String) {
        self.values.lock().push(reason);
    }

    fn reasons(&self) -> Vec<String> {
        self.values.lock().clone()
    }
}

async fn record_resyncs(sync: &RiseSync) -> Arc<RiseSyncResyncProbe> {
    let probe = Arc::new(RiseSyncResyncProbe::default());
    let recorder = probe.clone();
    sync.on_resync_required(move |reason| {
        let recorder = recorder.clone();
        async move { recorder.record(reason) }
    })
    .await;
    probe
}

#[derive(Debug, thiserror::Error)]
#[error("injected claim recovery failure")]
struct RiseDurableClaimRecoveryProbeError;

struct RiseDurableClaimRecoveryWriteProbe {
    remaining_failures: AtomicUsize,
    writes: AtomicUsize,
}

impl RiseDurableClaimRecoveryWriteProbe {
    fn new(failures: usize) -> Self {
        Self { remaining_failures: AtomicUsize::new(failures), writes: AtomicUsize::new(0) }
    }

    async fn write(
        &self,
        store: Arc<RiseStore>,
        batch: RiseStoreWriteBatch,
    ) -> anyhow::Result<RiseStoreCommit> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        let injected = self
            .remaining_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| remaining.checked_sub(1))
            .is_ok();
        if injected {
            return Err(RiseDurableClaimRecoveryProbeError.into());
        }
        Ok(store.write(batch).await?)
    }

    fn write_count(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

fn recovery_configuration(
    writer: &Arc<RiseDurableClaimRecoveryWriteProbe>,
    clock: Option<Arc<dyn RiseEngineClock>>,
) -> RiseSyncConfiguration {
    let writer = writer.clone();
    let defaults = RiseSyncConfiguration::default();
    RiseSyncConfiguration {
        maximum_concurrent_durable_operations: 1,
        clock: clock.unwrap_or(defaults.clock.clone()),
        durable_claim_recovery_write: Arc::new(
            move |store: Arc<RiseStore>,
                  batch: RiseStoreWriteBatch|
                  -> BoxFuture<'static, anyhow::Result<RiseStoreCommit>> {
            let writer = writer.clone();
            async move { writer.write(store, batch).await }.boxed()
        }),
        ..defaults
    }
}

struct RisePreparedRequestVersion {
    value: Mutex<&'static str>,
}

impl RisePreparedRequestVersion {
    const fn new() -> Self {
        Self { value: parking_lot::const_mutex("v1") }
    }

    fn set(&self, value: &'static str) {
        *self.value.lock() = value;
    }

    fn get(&self) -> &'static str {
        *self.value.lock()
    }
}

static RISE_PREPARED_REQUEST_VERSION: RisePreparedRequestVersion = RisePreparedRequestVersion::new();

fn prepared_request_version() -> &'static str {
    RISE_PREPARED_REQUEST_VERSION.get()
}

fn set_prepared_request_version(value: &'static str) {
    RISE_PREPARED_REQUEST_VERSION.set(value);
}

#[derive(Serialize, Deserialize)]
struct RiseSyncTestRequest {
    idempotency_key: String,
    value: String,
}

#[derive(Serialize, Deserialize)]
struct RiseSyncTestResponse {
    stored_value: String,
}

enum RiseSyncTestMethod {}

impl RiseWireMethod for RiseSyncTestMethod {
    type Request = RiseSyncTestRequest;
    type Response = RiseSyncTestResponse;

    fn descriptor() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(TEST_SERVICE, "store")
            .with_timeout(Duration::from_secs(2))
            .with_replay_policy(RiseWireReplayPolicy::IdempotencyKey {
                field: IDEMPOTENCY_FIELD.to_owned(),
            })
    }
}

fn test_request(value: String, context: &RiseSyncDurableContext) -> RiseSyncTestRequest {
    RiseSyncTestRequest { idempotency_key: context.idempotency_key.clone(), value }
}

#[derive(Serialize, Deserialize)]
struct RiseSyncTestPayload {
    key: String,
    value: String,
}

fn keyed(key: &str, value: &str) -> RiseSyncTestPayload {
    RiseSyncTestPayload { key: key.to_owned(), value: value.to_owned() }
}

#[derive(Serialize, Deserialize)]
struct RiseSyncValuePayload {
    value: String,
}

fn valued(value: &str) -> RiseSyncValuePayload {
    RiseSyncValuePayload { value: value.to_owned() }
}

enum RiseSyncTestEndpoint {}

impl RiseSyncDurableEndpoint for RiseSyncTestEndpoint {
    type Payload = RiseSyncTestPayload;
    type Method = RiseSyncTestMethod;

    fn kind() -> &'static str {
        "sync-test.store"
    }

    fn make_request(
        payload: &RiseSyncTestPayload,
        context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseSyncTestRequest> {
        Ok(test_request(payload.value.clone(), context))
    }

    fn reconcile(
        payload: &RiseSyncTestPayload,
        response: &RiseSyncTestResponse,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(batch(vec![plain_value(
            &sync_namespace(),
            payload.key.clone(),
            response.stored_value.as_bytes(),
        )]))
    }

    fn terminal_reconciliation(
        payload: &RiseSyncTestPayload,
        code: &str,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(batch(vec![plain_value(
            &sync_namespace(),
            terminal_key(&payload.key),
            code.as_bytes(),
        )]))
    }
}

fn terminal_key(key: &str) -> String {
    format!("{key}.terminal")
}

enum RiseSyncRetryEndpoint {}

impl RiseSyncDurableEndpoint for RiseSyncRetryEndpoint {
    type Payload = RiseSyncValuePayload;
    type Method = RiseSyncTestMethod;

    fn kind() -> &'static str {
        "sync-test.retry"
    }

    fn make_request(
        payload: &RiseSyncValuePayload,
        context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseSyncTestRequest> {
        Ok(test_request(payload.value.clone(), context))
    }

    fn reconcile(
        _payload: &RiseSyncValuePayload,
        _response: &RiseSyncTestResponse,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }

    fn terminal_reconciliation(
        _payload: &RiseSyncValuePayload,
        _code: &str,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }

    fn classify_failure(
        _failure: &RiseSyncDurableFailure,
        _context: &RiseSyncDurableContext,
    ) -> RiseSyncDurableFailureDisposition {
        RiseSyncDurableFailureDisposition::Retry { code: "deterministic_retry".to_owned() }
    }
}

enum RiseSyncVersionedEndpoint {}

impl RiseSyncDurableEndpoint for RiseSyncVersionedEndpoint {
    type Payload = RiseSyncValuePayload;
    type Method = RiseSyncTestMethod;

    fn kind() -> &'static str {
        "sync-test.versioned"
    }

    fn make_request(
        payload: &RiseSyncValuePayload,
        context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseSyncTestRequest> {
        Ok(test_request(format!("{}-{}", payload.value, prepared_request_version()), context))
    }

    fn reconcile(
        _payload: &RiseSyncValuePayload,
        _response: &RiseSyncTestResponse,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }

    fn terminal_reconciliation(
        _payload: &RiseSyncValuePayload,
        _code: &str,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }
}

#[derive(Serialize, Deserialize)]
struct RiseSyncRequestIDRequest {
    value: String,
}

enum RiseSyncRequestIDMethod {}

impl RiseWireMethod for RiseSyncRequestIDMethod {
    type Request = RiseSyncRequestIDRequest;
    type Response = RiseSyncTestResponse;

    fn descriptor() -> RiseWireMethodDescriptor {
        RiseWireMethodDescriptor::new(TEST_SERVICE, "request-id-store")
            .with_timeout(Duration::from_secs(2))
            .with_replay_policy(RiseWireReplayPolicy::RequestID)
    }
}

enum RiseSyncRequestIDEndpoint {}

impl RiseSyncDurableEndpoint for RiseSyncRequestIDEndpoint {
    type Payload = RiseSyncValuePayload;
    type Method = RiseSyncRequestIDMethod;

    fn kind() -> &'static str {
        "sync-test.request-id-store"
    }

    fn make_request(
        payload: &RiseSyncValuePayload,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseSyncRequestIDRequest> {
        Ok(RiseSyncRequestIDRequest { value: payload.value.clone() })
    }

    fn reconcile(
        _payload: &RiseSyncValuePayload,
        _response: &RiseSyncTestResponse,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }

    fn terminal_reconciliation(
        _payload: &RiseSyncValuePayload,
        _code: &str,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }
}

#[derive(Serialize, Deserialize)]
struct RiseSyncCommitFailurePayload {
    key: String,
    value: String,
    fail_first_commit: bool,
}

fn commit_failure(key: &str, value: &str, fail_first_commit: bool) -> RiseSyncCommitFailurePayload {
    RiseSyncCommitFailurePayload {
        key: key.to_owned(),
        value: value.to_owned(),
        fail_first_commit,
    }
}

enum RiseSyncCommitFailureEndpoint {}

impl RiseSyncDurableEndpoint for RiseSyncCommitFailureEndpoint {
    type Payload = RiseSyncCommitFailurePayload;
    type Method = RiseSyncTestMethod;

    fn kind() -> &'static str {
        "sync-test.commit-failure"
    }

    fn make_request(
        payload: &RiseSyncCommitFailurePayload,
        context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseSyncTestRequest> {
        Ok(test_request(payload.value.clone(), context))
    }

    fn reconcile(
        payload: &RiseSyncCommitFailurePayload,
        response: &RiseSyncTestResponse,
        context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let key = if payload.fail_first_commit && context.attempt_count == 0 {
            "x".repeat(IdentifierKind::ValueKey.maximum_bytes() + 1)
        } else {
            payload.key.clone()
        };
        Ok(batch(vec![set_value(
            &sync_namespace(),
            key,
            response.stored_value.as_bytes(),
            context.attempt_count + 1,
            1,
            RiseStoredValueWritePolicy::RequireIdenticalReplay,
        )]))
    }

    fn terminal_reconciliation(
        _payload: &RiseSyncCommitFailurePayload,
        _code: &str,
        _context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        Ok(RiseStoreWriteBatch::empty())
    }
}

fn account(account_id: &str) -> RiseWireAccountContext {
    RiseWireAccountContext::new(account_id.to_owned(), EPOCH.to_owned())
}

fn wire_with(account: &RiseWireAccountContext, sender: Arc<dyn RiseWireFrameSender>) -> Arc<RiseWire> {
    RiseWire::new(
        account.clone(),
        sender,
        None,
        RiseRequestIDAllocator::shared(),
        RiseWireConfiguration::production(),
    )
}

fn sync_with(
    store: &Arc<RiseStore>,
    wire: &Arc<RiseWire>,
    configuration: RiseSyncConfiguration,
) -> Arc<RiseSync> {
    RiseSync::with_configuration(
        store.clone(),
        wire.clone(),
        RiseRequestIDAllocator::shared(),
        configuration,
    )
}

fn default_sync(store: &Arc<RiseStore>, wire: &Arc<RiseWire>) -> Arc<RiseSync> {
    RiseSync::new(store.clone(), wire.clone(), RiseRequestIDAllocator::shared())
}

async fn memory_store(account_id: &str) -> anyhow::Result<Arc<RiseStore>> {
    let configuration = RiseStoreConfiguration::in_memory_default(account_id.to_owned());
    Ok(RiseStore::open(configuration).await?)
}

async fn memory_store_with_clock(
    account_id: &str,
    clock: Arc<dyn RiseEngineClock>,
) -> anyhow::Result<Arc<RiseStore>> {
    let configuration = RiseStoreConfiguration::in_memory(
        account_id.to_owned(),
        Arc::new(RisePlaintextStoreCipher),
        clock,
        RiseStoreLimits::production(),
    );
    Ok(RiseStore::open(configuration).await?)
}

async fn connect(wire: &RiseWire, session: &RiseWireSocketSession) {
    wire.socket_session_started(session.clone()).await;
    wire.socket_connected(session.clone()).await;
}

fn object(entries: Vec<(&str, RiseJSONValue)>) -> RiseJSONValue {
    RiseJSONValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn text(value: &str) -> RiseJSONValue {
    RiseJSONValue::String(value.to_owned())
}

fn encoded(frame: RiseJSONValue) -> Vec<u8> {
    frame.data(false).expect("a test frame encodes")
}

fn push_frame(event_type: &str, data: Vec<(&str, RiseJSONValue)>) -> Vec<u8> {
    encoded(object(vec![(KEY_FRAME_TYPE, text(event_type)), (KEY_DATA, object(data))]))
}

fn success_frame(request_id: RiseRequestID, stored_value: &str) -> Vec<u8> {
    encoded(object(vec![
        (KEY_REQUEST_ID, text(&request_id.raw_value().to_string())),
        (KEY_DATA, object(vec![("stored_value", text(stored_value))])),
    ]))
}

fn error_frame(request_id: RiseRequestID, code: &str, message: &str) -> Vec<u8> {
    encoded(object(vec![
        (KEY_REQUEST_ID, text(&request_id.raw_value().to_string())),
        (KEY_ERROR, object(vec![(KEY_CODE, text(code)), (KEY_MESSAGE, text(message))])),
    ]))
}

fn root_of(frame: &[u8]) -> anyhow::Result<BTreeMap<String, RiseJSONValue>> {
    RiseJSONValue::decode(frame)?
        .object_value()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("frame root is not an object"))
}

fn carries_request_id(root: &BTreeMap<String, RiseJSONValue>, request_id: RiseRequestID) -> bool {
    let id = root.get(KEY_ID);
    id == Some(&RiseJSONValue::UnsignedInteger(request_id.raw_value()))
        || i64::try_from(request_id.raw_value())
            .is_ok_and(|signed| id == Some(&RiseJSONValue::Integer(signed)))
}

fn data_field<'a>(root: &'a BTreeMap<String, RiseJSONValue>, key: &str) -> Option<&'a RiseJSONValue> {
    root.get(KEY_DATA).and_then(|data| data.subscript_key(key))
}

fn push_text(push: &RiseWirePushEnvelope, key: &str) -> Option<String> {
    push.data.subscript_key(key).and_then(RiseJSONValue::string_value).map(str::to_owned)
}

async fn eventually<Condition, Checked>(mut condition: Condition) -> anyhow::Result<()>
where
    Condition: FnMut() -> Checked,
    Checked: Future<Output = anyhow::Result<bool>>,
{
    for _ in 0..200 {
        if condition().await? {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    Err(RiseSyncTestTimeout::TimedOut.into())
}

async fn next_frame(sender: &RiseSyncSenderProbe) -> anyhow::Result<(Vec<u8>, RiseWireSocketSession)> {
    eventually(move || async move { Ok(sender.has_frame()) }).await?;
    sender.pop().ok_or_else(|| anyhow::anyhow!("frame vanished"))
}

async fn yield_a_while() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

async fn revision(store: &Arc<RiseStore>) -> anyhow::Result<i64> {
    Ok(store.snapshot(RiseStoreSnapshotRequest::default()).await?.revision)
}

async fn stored_data(store: &Arc<RiseStore>, key: &str) -> anyhow::Result<Option<Vec<u8>>> {
    Ok(store.value(sync_namespace(), key.to_owned()).await?.map(|value| value.data))
}

async fn operation_state(
    store: &Arc<RiseStore>,
    request_id: RiseRequestID,
) -> anyhow::Result<Option<RiseDurableOperationState>> {
    Ok(store.operation(request_id).await?.map(|operation| operation.state))
}

async fn is_removed(store: &Arc<RiseStore>, request_id: RiseRequestID) -> anyhow::Result<bool> {
    Ok(store.operation(request_id).await?.is_none())
}

async fn is_requeued_unburned(store: &Arc<RiseStore>, request_id: RiseRequestID) -> anyhow::Result<bool> {
    Ok(store.operation(request_id).await?.is_some_and(|operation| {
        operation.state == RiseDurableOperationState::Queued && operation.attempt_count == 0
    }))
}

fn on_lane(lane: &str) -> RiseSyncSubmitOptions {
    RiseSyncSubmitOptions {
        ordering_key: RiseDurableOrderingKey::new(lane),
        ..RiseSyncSubmitOptions::default()
    }
}

fn expect_error<Value, Expected>(result: anyhow::Result<Value>, expected: &Expected)
where
    Expected: std::error::Error + PartialEq + std::fmt::Debug + Send + Sync + 'static,
{
    match result {
        Ok(_) => panic!("expected {expected:?}"),
        Err(error) => assert_eq!(error.downcast_ref::<Expected>(), Some(expected), "{error:#}"),
    }
}

fn expect_cancellation<Value>(result: anyhow::Result<Value>) {
    match result {
        Ok(_) => panic!("expected CancellationError"),
        Err(error) => assert!(error.is::<CancellationError>(), "{error:#}"),
    }
}

fn temporary_root(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4().uuid_string()))
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = FileManager::default().remove_item(&self.0);
    }
}

fn engine_configuration(
    root: PathBuf,
    account: &RiseWireAccountContext,
    sender: &Arc<dyn RiseWireFrameSender>,
    key_store: Arc<dyn RiseEngineKeyStore>,
) -> RiseEngineConfiguration {
    RiseEngineConfiguration::new(
        root,
        account.clone(),
        sender.clone(),
        RiseEngineConfigurationArgs { key_store, ..RiseEngineConfigurationArgs::default() },
    )
}

fn lifecycle_revision(
    event: Option<RiseDurableLifecycleEvent>,
    expected_request_id: RiseRequestID,
    expected_kind: &str,
    expected_result: RiseDurableLifecycleResult,
    pick: fn(&RiseDurableLifecycleEvent) -> bool,
) -> Option<i64> {
    let event = event?;
    if !pick(&event) {
        panic!("unexpected lifecycle event {event:?}");
    }
    match event {
        RiseDurableLifecycleEvent::ReconciliationCommitted {
            request_id, kind, attempt, result, store_revision,
        }
        | RiseDurableLifecycleEvent::RetryScheduled {
            request_id, kind, attempt, result, store_revision,
        }
        | RiseDurableLifecycleEvent::Terminal {
            request_id, kind, attempt, result, store_revision,
        } => {
            assert_eq!(request_id, expected_request_id);
            assert_eq!(kind, expected_kind);
            assert_eq!(attempt, 1);
            assert_eq!(result, expected_result);
            Some(store_revision)
        }
        _ => None,
    }
}

fn is_reconciliation_committed(event: &RiseDurableLifecycleEvent) -> bool {
    matches!(event, RiseDurableLifecycleEvent::ReconciliationCommitted { .. })
}

fn is_retry_scheduled(event: &RiseDurableLifecycleEvent) -> bool {
    matches!(event, RiseDurableLifecycleEvent::RetryScheduled { .. })
}

fn is_terminal(event: &RiseDurableLifecycleEvent) -> bool {
    matches!(event, RiseDurableLifecycleEvent::Terminal { .. })
}

fn enqueued(request_id: RiseRequestID, kind: &str, store_revision: i64) -> Option<RiseDurableLifecycleEvent> {
    Some(RiseDurableLifecycleEvent::Enqueued { request_id, kind: kind.to_owned(), store_revision })
}

fn attempt_started(request_id: RiseRequestID, kind: &str) -> Option<RiseDurableLifecycleEvent> {
    Some(RiseDurableLifecycleEvent::AttemptStarted { request_id, kind: kind.to_owned(), attempt: 1 })
}

fn attempt_finished(
    request_id: RiseRequestID,
    kind: &str,
    result: RiseDurableLifecycleResult,
) -> Option<RiseDurableLifecycleEvent> {
    Some(RiseDurableLifecycleEvent::AttemptFinished {
        request_id,
        kind: kind.to_owned(),
        attempt: 1,
        result,
    })
}

fn reconciliation_failed(request_id: RiseRequestID, kind: &str) -> Option<RiseDurableLifecycleEvent> {
    Some(RiseDurableLifecycleEvent::ReconciliationFailed {
        request_id,
        kind: kind.to_owned(),
        attempt: 1,
        result: RiseDurableLifecycleResult::StoreCommitFailed,
    })
}

fn terminated(reason: RiseDurableLifecycleStreamTermination) -> Option<RiseDurableLifecycleEvent> {
    Some(RiseDurableLifecycleEvent::StreamTerminated { reason })
}

#[tokio::test]
async fn local_account_removal_deletes_key_before_touching_database_files() -> anyhow::Result<()> {
    let root = temporary_root("rise-engine-delete-order");
    let _cleanup = RemoveOnDrop(root.clone());
    let account_id = "delete-order-account";
    let directory = RiseEngine::account_directory(&root, account_id);
    FileManager::default().create_directory(&directory, true)?;
    let marker = directory.join(ENGINE_DATABASE_FILE_NAME);
    std::fs::write(&marker, b"must-remain")?;
    let key_store = Arc::new(RiseFailingDeleteKeyStore::default());

    expect_error(
        RiseEngine::remove_local_account_data(&root, account_id, key_store.clone()).await,
        &delete_probe_error(),
    );
    assert!(key_store.attempted_delete());
    assert!(FileManager::default().file_exists(&marker));
    Ok(())
}

#[tokio::test]
async fn engine_reopens_account_scoped_encrypted_state_and_fails_closed_without_key() -> anyhow::Result<()> {
    let root = temporary_root("rise-engine-facade");
    let _cleanup = RemoveOnDrop(root.clone());
    let key_store = Arc::new(RiseMemoryKeyStore::default());
    let account = account("engine-account");
    let sender = silent_sender();
    let configuration = || engine_configuration(root.clone(), &account, &sender, key_store.clone());
    let namespace = RiseStoreNamespace::new("engine-test");

    let first = RiseEngine::open(configuration()).await?;
    expect_error(RiseEngine::open(configuration()).await, &RiseEngineError::AccountAlreadyOpen);
    expect_error(
        RiseEngine::open(engine_configuration(
            root.join("alternate-root"),
            &account,
            &sender,
            key_store.clone(),
        ))
        .await,
        &RiseEngineError::AccountAlreadyOpen,
    );
    expect_error(
        RiseEngine::remove_local_account_data(&root, &account.account_id, key_store.clone()).await,
        &RiseEngineError::AccountInUse,
    );
    first.write(batch(vec![plain_value(&namespace, "value", b"persisted")])).await?;
    first.close().await?;

    let reopened = RiseEngine::open(configuration()).await?;
    assert_eq!(
        reopened.value(namespace.clone(), "value".to_owned()).await?.map(|value| value.data),
        Some(b"persisted".to_vec())
    );
    reopened.close().await?;

    key_store.delete_key(&account.account_id).await?;
    expect_error(
        RiseEngine::open(configuration()).await,
        &RiseEngineError::MissingKeyForExistingDatabase,
    );
    Ok(())
}

#[tokio::test]
async fn concurrent_start_is_single_flight_and_stop_waits_for_startup_rollback() -> anyhow::Result<()> {
    let clock = Arc::new(RiseBlockingClock::default());
    let store = memory_store_with_clock("sync-start", clock.clone()).await?;
    let account = account("sync-start");
    let wire = wire_with(&account, silent_sender());
    let sync = default_sync(&store, &wire);
    let second_completed = Arc::new(RiseCompletionFlag::default());
    let stop_completed = Arc::new(RiseCompletionFlag::default());
    let runtime = RiseAccountRuntime::current();

    let first = runtime.spawn({
        let sync = sync.clone();
        async move { sync.start().await }
    });
    clock.wait_until_entered().await;
    let second = runtime.spawn({
        let sync = sync.clone();
        let completed = second_completed.clone();
        async move {
            let result = sync.start().await;
            completed.mark_completed();
            result
        }
    });
    let stop = runtime.spawn({
        let sync = sync.clone();
        let completed = stop_completed.clone();
        async move {
            sync.stop().await;
            completed.mark_completed();
        }
    });
    yield_a_while().await;
    assert!(!second_completed.is_completed());
    assert!(!stop_completed.is_completed());

    clock.release();
    expect_cancellation(first.await?);
    expect_cancellation(second.await?);
    stop.await?;
    assert!(stop_completed.is_completed());

    sync.start().await?;
    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn shutdown_finishes_health_streams_and_rejects_restart() -> anyhow::Result<()> {
    let store = memory_store("sync-health-shutdown").await?;
    let account = account("sync-health-shutdown");
    let wire = wire_with(&account, silent_sender());
    let sync = default_sync(&store, &wire);
    let mut events = sync.health_events().await;

    sync.shutdown().await;

    assert_eq!(events.next().await, None);
    expect_error(sync.start().await, &RiseSyncError::Shutdown);
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn durable_lifecycle_acknowledgement_carries_exact_store_revisions() -> anyhow::Result<()> {
    let store = memory_store("sync-durable-lifecycle-ack").await?;
    let account = account("sync-durable-lifecycle-ack");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let sync = default_sync(&store, &wire);
    let kind = RiseSyncTestEndpoint::kind();
    sync.register_durable_endpoint::<RiseSyncTestEndpoint>().await?;
    sync.start().await?;
    let mut events = sync.durable_lifecycle_events().await;

    let receipt = sync
        .submit_durable_receipt::<RiseSyncTestEndpoint>(
            keyed("lifecycle", "body"),
            RiseSyncSubmitOptions::default(),
        )
        .await?;
    assert_eq!(events.next().await, enqueued(receipt.request_id, kind, receipt.commit.revision));
    assert_eq!(revision(&store).await?, receipt.commit.revision);

    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    assert_eq!(events.next().await, attempt_started(receipt.request_id, kind));
    next_frame(&sender).await?;
    let _ = wire.accept(&success_frame(receipt.request_id, "done"), &session).await;
    assert_eq!(
        events.next().await,
        attempt_finished(receipt.request_id, kind, RiseDurableLifecycleResult::Acknowledged)
    );
    let reconciliation_revision = lifecycle_revision(
        events.next().await,
        receipt.request_id,
        kind,
        RiseDurableLifecycleResult::Acknowledged,
        is_reconciliation_committed,
    );
    assert!(reconciliation_revision.is_some(), "Expected reconciliationCommitted lifecycle event");
    assert_eq!(Some(revision(&store).await?), reconciliation_revision);
    assert!(is_removed(&store, receipt.request_id).await?);

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn failed_durable_completion_commit_requeues_the_exact_claim() -> anyhow::Result<()> {
    let store = memory_store("sync-durable-commit-failure").await?;
    let account = account("sync-durable-commit-failure");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let sync = default_sync(&store, &wire);
    let resync = record_resyncs(&sync).await;
    let kind = RiseSyncCommitFailureEndpoint::kind();
    sync.register_durable_endpoint::<RiseSyncCommitFailureEndpoint>().await?;
    sync.start().await?;
    let mut events = sync.durable_lifecycle_events().await;
    let lane = "commit-failure-lane";

    let failed = sync
        .submit_durable_receipt::<RiseSyncCommitFailureEndpoint>(
            commit_failure("first", "first-value", true),
            on_lane(lane),
        )
        .await?;
    assert_eq!(events.next().await, enqueued(failed.request_id, kind, failed.commit.revision));
    let following = sync
        .submit_durable_receipt::<RiseSyncCommitFailureEndpoint>(
            commit_failure("following", "following-value", false),
            on_lane(lane),
        )
        .await?;
    assert_eq!(
        events.next().await,
        enqueued(following.request_id, kind, following.commit.revision)
    );

    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    assert_eq!(events.next().await, attempt_started(failed.request_id, kind));
    let (frame, _) = next_frame(&sender).await?;
    assert!(carries_request_id(&root_of(&frame)?, failed.request_id));
    let _ = wire.accept(&success_frame(failed.request_id, "done"), &session).await;
    assert_eq!(
        events.next().await,
        attempt_finished(failed.request_id, kind, RiseDurableLifecycleResult::Acknowledged)
    );
    assert_eq!(events.next().await, reconciliation_failed(failed.request_id, kind));
    let retry_revision = lifecycle_revision(
        events.next().await,
        failed.request_id,
        kind,
        RiseDurableLifecycleResult::StoreCommitFailed,
        is_retry_scheduled,
    );
    assert!(retry_revision.is_some(), "Expected retryScheduled lifecycle event");

    let failed_operation = store
        .operation(failed.request_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("failed operation missing"))?;
    assert_eq!(failed_operation.state, RiseDurableOperationState::RetryWaiting);
    assert_eq!(failed_operation.attempt_count, 1);
    assert_eq!(
        failed_operation.last_error_code.as_deref(),
        Some(RiseSyncFailureCode::StoreCommitFailed.wire())
    );
    let following_operation = store
        .operation(following.request_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("following operation missing"))?;
    assert_eq!(following_operation.state, RiseDurableOperationState::Queued);
    assert_eq!(following_operation.attempt_count, 0);
    assert_eq!(Some(revision(&store).await?), retry_revision);
    assert_eq!(
        store
            .replayable_operations(i64::MAX, 10)
            .await?
            .into_iter()
            .map(|operation| operation.request_id)
            .collect::<Vec<_>>(),
        vec![failed.request_id]
    );
    assert_eq!(resync.reasons(), vec![format!("durable completion failed for {kind}")]);

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn failed_claim_recovery_write_is_bounded_and_retried_before_the_next_drain() -> anyhow::Result<()> {
    let clock = Arc::new(RiseMutableClock::new(10_000));
    let store = memory_store_with_clock("sync-claim-recovery-retry", clock.clone()).await?;
    let account = account("sync-claim-recovery-retry");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let recovery_writer = Arc::new(RiseDurableClaimRecoveryWriteProbe::new(1));
    let sync = sync_with(
        &store,
        &wire,
        recovery_configuration(&recovery_writer, Some(clock.clone() as Arc<dyn RiseEngineClock>)),
    );
    let resync = record_resyncs(&sync).await;
    let kind = RiseSyncCommitFailureEndpoint::kind();
    sync.register_durable_endpoint::<RiseSyncCommitFailureEndpoint>().await?;
    sync.start().await?;
    let mut events = sync.durable_lifecycle_events().await;

    let failed = sync
        .submit_durable_receipt::<RiseSyncCommitFailureEndpoint>(
            commit_failure("recover-first", "first-value", true),
            RiseSyncSubmitOptions {
                request_id: RiseRequestID::new(1),
                ..on_lane("recovery-first-lane")
            },
        )
        .await?;
    events.next().await;
    let following = sync
        .submit_durable_receipt::<RiseSyncCommitFailureEndpoint>(
            commit_failure("recover-following", "following-value", false),
            on_lane("recovery-following-lane"),
        )
        .await?;
    events.next().await;

    let first_session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &first_session).await;
    events.next().await;
    next_frame(&sender).await?;
    let _ = wire.accept(&success_frame(failed.request_id, "done"), &first_session).await;
    events.next().await;
    assert_eq!(events.next().await, reconciliation_failed(failed.request_id, kind));
    {
        let (writer, sync, resync) = (&recovery_writer, &sync, &resync);
        eventually(move || async move {
            Ok(writer.write_count() == 1
                && sync.pending_durable_claim_recovery_count_for_testing() == 1
                && resync.reasons().len() == 1)
        })
        .await?;
    }
    let late_submission = sync
        .submit_durable_receipt::<RiseSyncCommitFailureEndpoint>(
            commit_failure("recover-late", "late-value", false),
            on_lane("recovery-late-lane"),
        )
        .await?;
    assert_eq!(
        events.next().await,
        enqueued(late_submission.request_id, kind, late_submission.commit.revision)
    );
    yield_a_while().await;
    assert_eq!(recovery_writer.write_count(), 1);
    assert_eq!(sync.pending_durable_claim_recovery_count_for_testing(), 1);
    assert!(!sender.has_frame());
    assert_eq!(
        operation_state(&store, failed.request_id).await?,
        Some(RiseDurableOperationState::InFlight)
    );
    assert_eq!(
        operation_state(&store, following.request_id).await?,
        Some(RiseDurableOperationState::Queued)
    );
    assert_eq!(
        operation_state(&store, late_submission.request_id).await?,
        Some(RiseDurableOperationState::Queued)
    );

    clock.advance(1_000);
    wire.socket_disconnected(first_session, "retry-recovery".to_owned()).await;
    let second_session = RiseWireSocketSession::new(account.clone(), 2);
    connect(&wire, &second_session).await;
    match events.next().await {
        Some(RiseDurableLifecycleEvent::RetryScheduled { request_id, kind: event_kind, attempt, result, .. }) => {
            assert_eq!(request_id, failed.request_id);
            assert_eq!(event_kind, kind);
            assert_eq!(attempt, 1);
            assert_eq!(result, RiseDurableLifecycleResult::StoreCommitFailed);
        }
        other => panic!("Expected recovered claim retry lifecycle event, got {other:?}"),
    }
    assert_eq!(events.next().await, attempt_started(following.request_id, kind));
    let (following_frame, _) = next_frame(&sender).await?;
    assert!(carries_request_id(&root_of(&following_frame)?, following.request_id));
    assert_eq!(recovery_writer.write_count(), 2);
    assert_eq!(sync.pending_durable_claim_recovery_count_for_testing(), 0);
    assert_eq!(
        operation_state(&store, failed.request_id).await?,
        Some(RiseDurableOperationState::RetryWaiting)
    );
    let _ = wire
        .accept(&success_frame(following.request_id, "done"), &second_session)
        .await;
    {
        let (store, request_id) = (&store, following.request_id);
        eventually(move || async move { is_removed(store, request_id).await }).await?;
    }

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn stop_clears_pending_claim_recovery_and_start_recovers_the_store_claim() -> anyhow::Result<()> {
    let store = memory_store("sync-claim-recovery-restart").await?;
    let account = account("sync-claim-recovery-restart");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let recovery_writer = Arc::new(RiseDurableClaimRecoveryWriteProbe::new(2));
    let sync = sync_with(&store, &wire, recovery_configuration(&recovery_writer, None));
    sync.register_durable_endpoint::<RiseSyncCommitFailureEndpoint>().await?;
    sync.start().await?;
    let mut events = sync.durable_lifecycle_events().await;
    let receipt = sync
        .submit_durable_receipt::<RiseSyncCommitFailureEndpoint>(
            commit_failure("restart-recovery", "value", true),
            RiseSyncSubmitOptions::default(),
        )
        .await?;
    events.next().await;

    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    events.next().await;
    next_frame(&sender).await?;
    let _ = wire.accept(&success_frame(receipt.request_id, "done"), &session).await;
    events.next().await;
    events.next().await;
    {
        let (writer, sync) = (&recovery_writer, &sync);
        eventually(move || async move {
            Ok(writer.write_count() == 1
                && sync.pending_durable_claim_recovery_count_for_testing() == 1)
        })
        .await?;
    }

    wire.socket_disconnected(session, "stop".to_owned()).await;
    sync.stop().await;
    assert_eq!(sync.pending_durable_claim_recovery_count_for_testing(), 0);
    assert_eq!(
        operation_state(&store, receipt.request_id).await?,
        Some(RiseDurableOperationState::InFlight)
    );

    sync.start().await?;
    assert_eq!(sync.pending_durable_claim_recovery_count_for_testing(), 0);
    let recovered = store
        .operation(receipt.request_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("recovered operation missing"))?;
    assert_eq!(recovered.state, RiseDurableOperationState::Queued);
    assert_eq!(recovered.attempt_count, 0);
    assert_eq!(recovery_writer.write_count(), 1);

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn durable_lifecycle_retry_carries_committed_retry_revision() -> anyhow::Result<()> {
    let store = memory_store("sync-durable-lifecycle-retry").await?;
    let account = account("sync-durable-lifecycle-retry");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let sync = default_sync(&store, &wire);
    let kind = RiseSyncRetryEndpoint::kind();
    sync.register_durable_endpoint::<RiseSyncRetryEndpoint>().await?;
    sync.start().await?;
    let mut events = sync.durable_lifecycle_events().await;
    let receipt = sync
        .submit_durable_receipt::<RiseSyncRetryEndpoint>(valued("body"), RiseSyncSubmitOptions::default())
        .await?;
    events.next().await;

    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    assert_eq!(events.next().await, attempt_started(receipt.request_id, kind));
    next_frame(&sender).await?;
    let _ = wire
        .accept(&error_frame(receipt.request_id, "retry_me", "retry"), &session)
        .await;
    assert_eq!(
        events.next().await,
        attempt_finished(receipt.request_id, kind, RiseDurableLifecycleResult::RetryableFailure)
    );
    let retry_revision = lifecycle_revision(
        events.next().await,
        receipt.request_id,
        kind,
        RiseDurableLifecycleResult::RetryableFailure,
        is_retry_scheduled,
    );
    assert!(retry_revision.is_some(), "Expected retryScheduled lifecycle event");
    assert_eq!(Some(revision(&store).await?), retry_revision);
    let operation = store
        .operation(receipt.request_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("operation missing"))?;
    assert_eq!(operation.state, RiseDurableOperationState::RetryWaiting);
    assert_eq!(operation.attempt_count, 1);

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn durable_lifecycle_terminal_carries_atomic_terminal_revision() -> anyhow::Result<()> {
    let store = memory_store("sync-durable-lifecycle-terminal").await?;
    let account = account("sync-durable-lifecycle-terminal");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let sync = default_sync(&store, &wire);
    let kind = RiseSyncTestEndpoint::kind();
    sync.register_durable_endpoint::<RiseSyncTestEndpoint>().await?;
    sync.start().await?;
    let mut events = sync.durable_lifecycle_events().await;
    let receipt = sync
        .submit_durable_receipt::<RiseSyncTestEndpoint>(
            keyed("terminal-lifecycle", "body"),
            RiseSyncSubmitOptions::default(),
        )
        .await?;
    events.next().await;

    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    events.next().await;
    next_frame(&sender).await?;
    let _ = wire
        .accept(&error_frame(receipt.request_id, "denied", "denied"), &session)
        .await;
    assert_eq!(
        events.next().await,
        attempt_finished(receipt.request_id, kind, RiseDurableLifecycleResult::TerminalFailure)
    );
    let terminal_revision = lifecycle_revision(
        events.next().await,
        receipt.request_id,
        kind,
        RiseDurableLifecycleResult::TerminalFailure,
        is_terminal,
    );
    assert!(terminal_revision.is_some(), "Expected terminal lifecycle event");
    assert_eq!(Some(revision(&store).await?), terminal_revision);
    let operation = store
        .operation(receipt.request_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("operation missing"))?;
    assert_eq!(operation.state, RiseDurableOperationState::TerminalFailure);
    assert_eq!(
        stored_data(&store, &terminal_key("terminal-lifecycle")).await?,
        Some(b"denied".to_vec())
    );

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn durable_lifecycle_subscriber_capacity_evicts_oldest_stream() -> anyhow::Result<()> {
    let store = memory_store("sync-durable-lifecycle-capacity").await?;
    let account = account("sync-durable-lifecycle-capacity");
    let wire = wire_with(&account, silent_sender());
    let sync = default_sync(&store, &wire);
    let mut streams = Vec::new();
    for _ in 0..=DURABLE_LIFECYCLE_SUBSCRIBER_LIMIT {
        streams.push(sync.durable_lifecycle_events().await);
    }
    let oldest = &mut streams[0];

    assert_eq!(
        oldest.next().await,
        terminated(RiseDurableLifecycleStreamTermination::SubscriberEvicted)
    );
    assert_eq!(oldest.next().await, None);

    sync.shutdown().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn durable_lifecycle_buffer_drop_publishes_gap_to_that_subscriber() -> anyhow::Result<()> {
    let store = memory_store("sync-durable-lifecycle-gap").await?;
    let account = account("sync-durable-lifecycle-gap");
    let wire = wire_with(&account, silent_sender());
    let sync = default_sync(&store, &wire);
    sync.register_durable_endpoint::<RiseSyncTestEndpoint>().await?;
    let mut events = sync.durable_lifecycle_events().await;

    for index in 0..=DURABLE_LIFECYCLE_BUFFER_SIZE {
        sync.submit_durable_receipt::<RiseSyncTestEndpoint>(
            keyed(&format!("gap-{index}"), "body"),
            RiseSyncSubmitOptions::default(),
        )
        .await?;
    }

    let mut received = Vec::with_capacity(DURABLE_LIFECYCLE_BUFFER_SIZE);
    for _ in 0..DURABLE_LIFECYCLE_BUFFER_SIZE {
        received.push(events.next().await.ok_or_else(|| anyhow::anyhow!("stream finished"))?);
    }
    assert!(received.contains(&RiseDurableLifecycleEvent::Gap));
    assert_eq!(received.last(), Some(&RiseDurableLifecycleEvent::Gap));

    sync.shutdown().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn durable_lifecycle_shutdown_reports_reason_before_finishing() -> anyhow::Result<()> {
    let store = memory_store("sync-durable-lifecycle-shutdown").await?;
    let account = account("sync-durable-lifecycle-shutdown");
    let wire = wire_with(&account, silent_sender());
    let sync = default_sync(&store, &wire);
    let mut events = sync.durable_lifecycle_events().await;

    sync.shutdown().await;

    assert_eq!(events.next().await, terminated(RiseDurableLifecycleStreamTermination::EngineShutdown));
    assert_eq!(events.next().await, None);
    let mut after_shutdown = sync.durable_lifecycle_events().await;
    assert_eq!(
        after_shutdown.next().await,
        terminated(RiseDurableLifecycleStreamTermination::EngineShutdown)
    );
    assert_eq!(after_shutdown.next().await, None);
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn push_reducer_commits_off_wire_and_durable_outbox_uses_same_request_id() -> anyhow::Result<()> {
    let store = memory_store("sync-account").await?;
    let account = account("sync-account");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let sync = default_sync(&store, &wire);
    let namespace = sync_namespace();

    sync.register_push_handler(RiseSyncPushHandler::new("value.changed", {
        let namespace = namespace.clone();
        move |push: RiseWirePushEnvelope| {
            let namespace = namespace.clone();
            async move {
                let Some(text) = push_text(&push, "value") else {
                    return Ok(None);
                };
                Ok(Some(batch(vec![plain_value(&namespace, "remote", text.as_bytes())])))
            }
        }
    }))
    .await?;
    sync.register_durable_endpoint::<RiseSyncTestEndpoint>().await?;
    sync.start().await?;

    let optimistic_id = sync
        .submit_durable::<RiseSyncTestEndpoint>(
            keyed("acknowledged", "body"),
            RiseSyncSubmitOptions {
                optimistic: batch(vec![plain_value(&namespace, "optimistic", b"visible")]),
                ..on_lane("chat-1")
            },
        )
        .await?;
    assert_eq!(stored_data(&store, "optimistic").await?, Some(b"visible".to_vec()));
    assert!(store.operation(optimistic_id).await?.is_some());

    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    let _ = wire
        .accept(&push_frame("value.changed", vec![("value", text("remote-value"))]), &session)
        .await;

    let (outbound, outbound_session) = next_frame(&sender).await?;
    assert_eq!(outbound_session, session);
    let root = root_of(&outbound)?;
    assert!(carries_request_id(&root, optimistic_id));
    assert_eq!(
        data_field(&root, IDEMPOTENCY_FIELD),
        Some(&text(&optimistic_id.raw_value().to_string()))
    );
    let _ = wire.accept(&success_frame(optimistic_id, "server-body"), &session).await;

    {
        let store = &store;
        eventually(move || async move {
            Ok(is_removed(store, optimistic_id).await?
                && stored_data(store, "remote").await? == Some(b"remote-value".to_vec())
                && stored_data(store, "acknowledged").await? == Some(b"server-body".to_vec()))
        })
        .await?;
    }
    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn push_burst_uses_async_reducers_in_arrival_order_and_commits_one_transaction() -> anyhow::Result<()> {
    let store = memory_store("sync-push-burst").await?;
    let account = account("sync-push-burst");
    let wire = wire_with(&account, silent_sender());
    let reducer_gate = Arc::new(RiseAsyncPushReducerGate::default());
    let namespace = sync_namespace();
    let key = RiseStoreValueKey::new(namespace.clone(), "ordered");
    let sync = sync_with(
        &store,
        &wire,
        RiseSyncConfiguration {
            maximum_push_batch_events: 3,
            maximum_push_batch_mutations: 16,
            maximum_push_batch_bytes: 64 * 1_024,
            push_batch_delay: Duration::from_secs(60),
            ..RiseSyncConfiguration::default()
        },
    );
    sync.register_push_handler(RiseSyncPushHandler::new("ordered.value", {
        let (gate, store, namespace, key) =
            (reducer_gate.clone(), store.clone(), namespace.clone(), key.key.clone());
        move |push: RiseWirePushEnvelope| {
            let (gate, store, namespace, key) =
                (gate.clone(), store.clone(), namespace.clone(), key.clone());
            async move {
                let (Some(text), Some(RiseJSONValue::Integer(version))) =
                    (push_text(&push, "value"), push.data.subscript_key("version").cloned())
                else {
                    return Ok(None);
                };
                gate.enter(text.clone()).await;
                store.value(namespace.clone(), "lookup".to_owned()).await?;
                Ok::<_, anyhow::Error>(Some(batch(vec![set_value(
                    &namespace,
                    key,
                    text.as_bytes(),
                    version,
                    version,
                    RiseStoredValueWritePolicy::ServerAuthoritative,
                )])))
            }
        }
    }))
    .await?;
    sync.start().await?;
    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;

    for (value, version) in [("first", 1), ("second", 2), ("third", 3)] {
        let _ = wire
            .accept(
                &push_frame(
                    "ordered.value",
                    vec![("value", text(value)), ("version", RiseJSONValue::Integer(version))],
                ),
                &session,
            )
            .await;
    }

    {
        let gate = &reducer_gate;
        eventually(move || async move { Ok(gate.received_values() == ["first"]) }).await?;
    }
    yield_a_while().await;
    assert_eq!(reducer_gate.received_values(), ["first"]);
    reducer_gate.release_first_call();

    {
        let store = &store;
        eventually(move || async move {
            Ok(stored_data(store, "ordered").await? == Some(b"third".to_vec()))
        })
        .await?;
    }
    let snapshot = store
        .snapshot(RiseStoreSnapshotRequest {
            values: HashSet::from([key.clone()]),
            ..RiseStoreSnapshotRequest::default()
        })
        .await?;
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.values.get(&key).map(|value| value.data.clone()), Some(b"third".to_vec()));
    assert_eq!(reducer_gate.received_values(), ["first", "second", "third"]);

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn push_batch_timer_flushes_at_eight_milliseconds_deterministically() -> anyhow::Result<()> {
    let store = memory_store("sync-push-timer").await?;
    let account = account("sync-push-timer");
    let wire = wire_with(&account, silent_sender());
    let sleep_gate = Arc::new(RisePushBatchSleepGate::default());
    let namespace = sync_namespace();
    let key = RiseStoreValueKey::new(namespace.clone(), "timer");
    let sync = sync_with(
        &store,
        &wire,
        RiseSyncConfiguration {
            push_batch_sleep: {
                let gate = sleep_gate.clone();
                Arc::new(move |duration: Duration| -> BoxFuture<'static, anyhow::Result<()>> {
                    let woken = gate.sleep(duration);
                    async move {
                        let _ = woken.await;
                        Ok(())
                    }
                    .boxed()
                })
            },
            ..RiseSyncConfiguration::default()
        },
    );
    sync.register_push_handler(RiseSyncPushHandler::new("timer.value", {
        let (namespace, key) = (namespace.clone(), key.key.clone());
        move |push: RiseWirePushEnvelope| {
            let (namespace, key) = (namespace.clone(), key.clone());
            async move {
                let Some(text) = push_text(&push, "value") else {
                    return Ok(None);
                };
                Ok(Some(batch(vec![plain_value(&namespace, key, text.as_bytes())])))
            }
        }
    }))
    .await?;
    sync.start().await?;
    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    let _ = wire
        .accept(&push_frame("timer.value", vec![("value", text("flushed"))]), &session)
        .await;

    {
        let gate = &sleep_gate;
        eventually(move || async move { Ok(!gate.requested_durations().is_empty()) }).await?;
    }
    assert_eq!(
        sleep_gate.requested_durations(),
        vec![Duration::from_millis(PUSH_BATCH_WINDOW_MS)]
    );
    assert_eq!(stored_data(&store, "timer").await?, None);
    sleep_gate.release();

    {
        let store = &store;
        eventually(move || async move {
            Ok(stored_data(store, "timer").await? == Some(b"flushed".to_vec()))
        })
        .await?;
    }
    let snapshot = store
        .snapshot(RiseStoreSnapshotRequest {
            values: HashSet::from([key]),
            ..RiseStoreSnapshotRequest::default()
        })
        .await?;
    assert_eq!(snapshot.revision, 1);

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn stop_flushes_accepted_push_and_rejects_post_stop_events() -> anyhow::Result<()> {
    let store = memory_store("sync-push-stop").await?;
    let account = account("sync-push-stop");
    let wire = wire_with(&account, silent_sender());
    let namespace = sync_namespace();
    let sync = sync_with(
        &store,
        &wire,
        RiseSyncConfiguration {
            push_batch_delay: Duration::from_secs(60),
            ..RiseSyncConfiguration::default()
        },
    );
    sync.register_push_handler(RiseSyncPushHandler::new("stop.value", {
        let namespace = namespace.clone();
        move |push: RiseWirePushEnvelope| {
            let namespace = namespace.clone();
            async move {
                let Some(key) = push_text(&push, "key") else {
                    return Ok(None);
                };
                Ok(Some(batch(vec![plain_value(&namespace, key.clone(), key.as_bytes())])))
            }
        }
    }))
    .await?;
    sync.start().await?;
    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    let _ = wire
        .accept(&push_frame("stop.value", vec![("key", text("accepted"))]), &session)
        .await;
    {
        let sync = &sync;
        eventually(move || async move { Ok(sync.pending_push_event_count_for_testing() == 1) })
            .await?;
    }

    sync.stop().await;
    assert_eq!(stored_data(&store, "accepted").await?, Some(b"accepted".to_vec()));
    assert_eq!(revision(&store).await?, 1);

    let _ = wire
        .accept(&push_frame("stop.value", vec![("key", text("rejected"))]), &session)
        .await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(stored_data(&store, "rejected").await?, None);
    assert_eq!(revision(&store).await?, 1);
    assert_eq!(sync.pending_push_event_count_for_testing(), 0);

    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn oversized_pushes_request_resync_without_entering_the_store() -> anyhow::Result<()> {
    let store = memory_store("sync-push-limits").await?;
    let account = account("sync-push-limits");
    let wire = wire_with(&account, silent_sender());
    let namespace = sync_namespace();
    let sync = sync_with(
        &store,
        &wire,
        RiseSyncConfiguration {
            maximum_push_batch_events: 8,
            maximum_push_batch_mutations: 1,
            maximum_push_batch_bytes: 256,
            push_batch_delay: Duration::from_secs(60),
            ..RiseSyncConfiguration::default()
        },
    );
    let resync = record_resyncs(&sync).await;
    sync.register_push_handler(RiseSyncPushHandler::new("too-many-mutations", {
        let namespace = namespace.clone();
        move |_push: RiseWirePushEnvelope| {
            let namespace = namespace.clone();
            async move {
                let remove = |key: &str| RiseStoreMutation::RemoveValue {
                    namespace: namespace.clone(),
                    key: key.to_owned(),
                    precondition: RiseStoreVersionPrecondition::Any,
                };
                Ok(Some(batch(vec![remove("first"), remove("second")])))
            }
        }
    }))
    .await?;
    sync.register_push_handler(RiseSyncPushHandler::new("too-many-bytes", {
        let namespace = namespace.clone();
        move |_push: RiseWirePushEnvelope| {
            let namespace = namespace.clone();
            async move { Ok(Some(batch(vec![plain_value(&namespace, "large", &[1; 1_024])]))) }
        }
    }))
    .await?;
    sync.start().await?;
    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    let _ = wire.accept(&push_frame("too-many-mutations", vec![]), &session).await;
    let _ = wire.accept(&push_frame("too-many-bytes", vec![]), &session).await;

    {
        let resync = &resync;
        eventually(move || async move { Ok(resync.reasons().len() == 2) }).await?;
    }
    assert_eq!(
        resync.reasons(),
        ["push batch exceeds mutation limit", "push batch exceeds estimated byte limit"]
    );
    assert_eq!(revision(&store).await?, 0);
    assert_eq!(sync.pending_push_event_count_for_testing(), 0);

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn failed_push_transaction_rolls_back_and_requests_resync() -> anyhow::Result<()> {
    let store = memory_store("sync-push-failure").await?;
    let namespace = sync_namespace();
    store.write(batch(vec![plain_value(&namespace, "existing", b"old")])).await?;
    let account = account("sync-push-failure");
    let wire = wire_with(&account, silent_sender());
    let sync = sync_with(
        &store,
        &wire,
        RiseSyncConfiguration {
            push_batch_delay: Duration::ZERO,
            ..RiseSyncConfiguration::default()
        },
    );
    let resync = record_resyncs(&sync).await;
    sync.register_push_handler(RiseSyncPushHandler::new("conflicting.batch", {
        let namespace = namespace.clone();
        move |_push: RiseWirePushEnvelope| {
            let namespace = namespace.clone();
            async move {
                let strict = RiseStoredValueWritePolicy::RequireIdenticalReplay;
                Ok(Some(batch(vec![
                    set_value(&namespace, "new", b"must-rollback", 1, 2, strict),
                    set_value(&namespace, "existing", b"conflict", 1, 2, strict),
                ])))
            }
        }
    }))
    .await?;
    sync.start().await?;
    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    let _ = wire.accept(&push_frame("conflicting.batch", vec![]), &session).await;

    {
        let resync = &resync;
        eventually(move || async move { Ok(resync.reasons() == ["push batch commit failed"]) })
            .await?;
    }
    assert_eq!(stored_data(&store, "new").await?, None);
    assert_eq!(stored_data(&store, "existing").await?, Some(b"old".to_vec()));
    assert_eq!(revision(&store).await?, 1);

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn durable_endpoint_can_use_stable_envelope_request_id_as_replay_identity() -> anyhow::Result<()> {
    let store = memory_store("sync-request-id").await?;
    let account = account("sync-request-id");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let sync = default_sync(&store, &wire);
    sync.register_durable_endpoint::<RiseSyncRequestIDEndpoint>().await?;
    sync.start().await?;
    let request_id = sync
        .submit_durable::<RiseSyncRequestIDEndpoint>(valued("body"), on_lane("chat-1"))
        .await?;

    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    let (frame, _) = next_frame(&sender).await?;
    let root = root_of(&frame)?;
    assert!(carries_request_id(&root, request_id));
    assert_eq!(data_field(&root, "value"), Some(&text("body")));
    assert_eq!(data_field(&root, IDEMPOTENCY_FIELD), None);

    let _ = wire.accept(&success_frame(request_id, "done"), &session).await;
    {
        let store = &store;
        eventually(move || async move { is_removed(store, request_id).await }).await?;
    }

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

#[tokio::test]
async fn disconnect_requeues_claim_without_burning_attempt_and_replays_same_envelope_id() -> anyhow::Result<()> {
    let store = memory_store("sync-disconnect").await?;
    let account = account("sync-disconnect");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let sync = default_sync(&store, &wire);
    sync.register_durable_endpoint::<RiseSyncTestEndpoint>().await?;
    sync.start().await?;
    let request_id = sync
        .submit_durable::<RiseSyncTestEndpoint>(keyed("replayed", "body"), on_lane("chat"))
        .await?;

    let first_session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &first_session).await;
    let (first_frame, _) = next_frame(&sender).await?;
    let first_root = root_of(&first_frame)?;
    assert!(carries_request_id(&first_root, request_id));

    wire.socket_disconnected(first_session, "offline".to_owned()).await;
    {
        let store = &store;
        eventually(move || async move { is_requeued_unburned(store, request_id).await }).await?;
    }

    let second_session = RiseWireSocketSession::new(account.clone(), 2);
    connect(&wire, &second_session).await;
    let (second_frame, _) = next_frame(&sender).await?;
    let second_root = root_of(&second_frame)?;
    assert_eq!(second_root.get(KEY_ID), first_root.get(KEY_ID));
    assert_eq!(second_root.get(KEY_DATA), first_root.get(KEY_DATA));
    let _ = wire.accept(&success_frame(request_id, "done"), &second_session).await;
    {
        let store = &store;
        eventually(move || async move { is_removed(store, request_id).await }).await?;
    }

    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}

struct ResetPreparedRequestVersion;

impl Drop for ResetPreparedRequestVersion {
    fn drop(&mut self) {
        set_prepared_request_version("v1");
    }
}

#[tokio::test]
async fn restart_replays_the_persisted_prepared_request_without_rebuilding_its_body() -> anyhow::Result<()> {
    let directory = temporary_root("rise-sync-restart");
    let _cleanup = RemoveOnDrop(directory.clone());
    let _reset_version = ResetPreparedRequestVersion;
    FileManager::default().create_directory(&directory, true)?;
    let account_id = "sync-restart";
    let database_url = directory.join(ENGINE_DATABASE_FILE_NAME);
    let configuration = || {
        RiseStoreConfiguration::new(
            database_url.clone(),
            account_id.to_owned(),
            Arc::new(RisePlaintextStoreCipher),
            Arc::new(RiseSystemClock),
            RiseStoreLimits::production(),
            false,
        )
    };
    let account = account(account_id);

    set_prepared_request_version("v1");
    let first_store = RiseStore::open(configuration()).await?;
    let first_sender = Arc::new(RiseSyncSenderProbe::default());
    let first_wire = wire_with(&account, first_sender.clone());
    let first_sync = default_sync(&first_store, &first_wire);
    first_sync.register_durable_endpoint::<RiseSyncVersionedEndpoint>().await?;
    first_sync.start().await?;
    let request_id = first_sync
        .submit_durable::<RiseSyncVersionedEndpoint>(valued("body"), on_lane("chat"))
        .await?;
    let first_session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&first_wire, &first_session).await;
    let (first_frame, _) = next_frame(&first_sender).await?;
    let first_root = root_of(&first_frame)?;
    assert_eq!(data_field(&first_root, "value"), Some(&text("body-v1")));

    first_wire.socket_disconnected(first_session, "restart".to_owned()).await;
    {
        let store = &first_store;
        eventually(move || async move { is_requeued_unburned(store, request_id).await }).await?;
    }
    first_sync.stop().await;
    first_wire.shutdown().await;
    first_store.close().await?;

    set_prepared_request_version("v2");
    let reopened_store = RiseStore::open(configuration()).await?;
    let second_sender = Arc::new(RiseSyncSenderProbe::default());
    let second_wire = wire_with(&account, second_sender.clone());
    let second_sync = default_sync(&reopened_store, &second_wire);
    second_sync.register_durable_endpoint::<RiseSyncVersionedEndpoint>().await?;
    second_sync.start().await?;
    let second_session = RiseWireSocketSession::new(account.clone(), 2);
    connect(&second_wire, &second_session).await;
    let (second_frame, _) = next_frame(&second_sender).await?;
    let second_root = root_of(&second_frame)?;

    assert_eq!(second_root.get(KEY_ID), first_root.get(KEY_ID));
    assert_eq!(second_root.get(KEY_DATA), first_root.get(KEY_DATA));
    assert_eq!(data_field(&second_root, "value"), Some(&text("body-v1")));

    let _ = second_wire
        .accept(&success_frame(request_id, "done"), &second_session)
        .await;
    {
        let store = &reopened_store;
        eventually(move || async move { is_removed(store, request_id).await }).await?;
    }
    second_sync.stop().await;
    second_wire.shutdown().await;
    reopened_store.close().await?;
    Ok(())
}

#[tokio::test]
async fn terminal_reconciliation_is_atomic_and_does_not_block_the_ordering_lane() -> anyhow::Result<()> {
    let store = memory_store("sync-terminal").await?;
    let account = account("sync-terminal");
    let sender = Arc::new(RiseSyncSenderProbe::default());
    let wire = wire_with(&account, sender.clone());
    let sync = default_sync(&store, &wire);
    sync.register_durable_endpoint::<RiseSyncTestEndpoint>().await?;
    sync.start().await?;
    let lane = "chat";
    let failed_id = sync
        .submit_durable::<RiseSyncTestEndpoint>(keyed("failed", "first"), on_lane(lane))
        .await?;
    let following_id = sync
        .submit_durable::<RiseSyncTestEndpoint>(keyed("following", "second"), on_lane(lane))
        .await?;

    let session = RiseWireSocketSession::new(account.clone(), 1);
    connect(&wire, &session).await;
    let (failed_frame, _) = next_frame(&sender).await?;
    assert!(carries_request_id(&root_of(&failed_frame)?, failed_id));

    let _ = wire.accept(&error_frame(failed_id, "denied", "denied"), &session).await;
    let (following_frame, _) = next_frame(&sender).await?;
    assert!(carries_request_id(&root_of(&following_frame)?, following_id));

    {
        let (sync, store) = (&sync, &store);
        eventually(move || async move {
            let terminal: Vec<RiseRequestID> = sync
                .terminal_operations(10)
                .await?
                .into_iter()
                .map(|operation| operation.request_id)
                .collect();
            let reconciliation = stored_data(store, &terminal_key("failed")).await?;
            Ok(terminal == vec![failed_id] && reconciliation == Some(b"denied".to_vec()))
        })
        .await?;
    }
    let _ = wire.accept(&success_frame(following_id, "done"), &session).await;
    {
        let store = &store;
        eventually(move || async move { is_removed(store, following_id).await }).await?;
    }

    sync.resolve_terminal_operation(failed_id, RiseDurableTerminalResolution::Remove)
        .await?;
    assert!(is_removed(&store, failed_id).await?);
    sync.stop().await;
    wire.shutdown().await;
    let _ = store.close().await;
    Ok(())
}
