use crate::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use futures::stream::{FuturesUnordered, StreamExt};
use parking_lot::{Mutex, RwLock};
use rise_canon::rise_sync::{
    FailureCode as RiseSyncFailureCode, OUTBOX_MAXIMUM_CONCURRENT_OPERATIONS,
    PUSH_BATCH_MAXIMUM_BYTES, PUSH_BATCH_MAXIMUM_EVENTS, PUSH_BATCH_MAXIMUM_MUTATIONS,
    PUSH_BATCH_WINDOW_MS, RETRY_BASE_MS, RETRY_CEILING_MS, RETRY_DEFER_DELAY_MS,
    RETRY_EXPONENT_MAXIMUM, RETRY_MAXIMUM_ATTEMPTS, RETRY_MINIMUM_DEFER_MS,
    claim_recovery_delay_ms, retry_delay_ms,
};
use rise_canon::RiseWireRequestMetadata;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Notify, broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_core::rise_identifiers::{RiseRequestID, RiseRequestIDAllocator};
use crate::rise_store::rise_store::{RiseEngineClock, RiseStore, RiseSystemClock};
use crate::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityViewRetention, RiseEntityWritePolicy, RiseStoreEntity,
};
use crate::rise_store::rise_store_models::{
    RiseDurableClaim, RiseDurableOperation, RiseDurableOperationState, RiseDurableOrderingKey,
    RiseDurableTerminalResolution, RiseStoreCommit, RiseStoreMutation, RiseStoreNamespace,
    RiseStoreVersionPrecondition, RiseStoreViewKey, RiseStoreWriteBatch,
};
use crate::rise_wire::rise_json_value::RiseJSONValue;
use crate::rise_wire::rise_wire::{
    CancellationError, RiseWire, RiseWireError, RiseWireEvent, RiseWireEventSubscription,
};
use crate::rise_wire::rise_wire_contracts::{
    RiseWireMethod, RiseWireMethodDescriptor, RiseWirePushEnvelope, RiseWireRemoteError,
    RiseWireReplayPolicy,
};

const _: () = assert!(
    RETRY_BASE_MS << RETRY_EXPONENT_MAXIMUM == RETRY_CEILING_MS,
    "canon retry ceiling disagrees with base << exponent"
);

const MAXIMUM_DURABLE_LIFECYCLE_SUBSCRIBERS: usize = 8;
const DURABLE_LIFECYCLE_BUFFER_SIZE: usize = 64;
const HEALTH_BUFFER_SIZE: usize = 32;
const DEFER_DELAY_MILLISECONDS: i64 = RETRY_DEFER_DELAY_MS as i64;
const MINIMUM_DEFER_MILLISECONDS: i64 = RETRY_MINIMUM_DEFER_MS as i64;
const DRAIN_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(1);

pub type RiseSyncPushReducer = Arc<
    dyn Fn(RiseWirePushEnvelope) -> BoxFuture<'static, anyhow::Result<Option<RiseStoreWriteBatch>>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct RiseSyncPushHandler {
    pub event_type: String,
    reduce: RiseSyncPushReducer,
}

impl RiseSyncPushHandler {
    pub fn new<Reduce, Reduction>(event_type: impl Into<String>, reduce: Reduce) -> Self
    where
        Reduce: Fn(RiseWirePushEnvelope) -> Reduction + Send + Sync + 'static,
        Reduction: Future<Output = anyhow::Result<Option<RiseStoreWriteBatch>>> + Send + 'static,
    {
        let event_type = event_type.into();
        assert!(!event_type.is_empty(), "push handler event type is empty");
        Self {
            event_type,
            reduce: Arc::new(move |push| reduce(push).boxed()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseSyncDurableContext {
    pub request_id: RiseRequestID,
    pub idempotency_key: String,
    pub attempt_count: i64,
}

impl RiseSyncDurableContext {
    fn new(request_id: RiseRequestID, attempt_count: i64) -> Self {
        Self {
            request_id,
            idempotency_key: request_id.raw_value().to_string(),
            attempt_count,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseSyncDurableFailure {
    Unavailable,
    Cancelled,
    Timeout,
    MalformedResponse,
    UnsafeReplay,
    Remote(RiseWireRemoteError),
    Transport { code: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseSyncDurableFailureDisposition {
    Pause,
    Retry { code: String },
    Terminal { code: String },
}

impl RiseSyncDurableFailureDisposition {
    /// default disposition for an endpoint that refines only some failures
    pub fn standard(failure: &RiseSyncDurableFailure) -> Self {
        match failure {
            RiseSyncDurableFailure::Unavailable | RiseSyncDurableFailure::Cancelled => Self::Pause,
            RiseSyncDurableFailure::Timeout => Self::Retry {
                code: RiseSyncFailureCode::Timeout.wire().to_string(),
            },
            RiseSyncDurableFailure::MalformedResponse => Self::Retry {
                code: RiseSyncFailureCode::MalformedResponse.wire().to_string(),
            },
            RiseSyncDurableFailure::UnsafeReplay => Self::Terminal {
                code: RiseSyncFailureCode::UnsafeReplay.wire().to_string(),
            },
            RiseSyncDurableFailure::Remote(error) => Self::Terminal {
                code: error.code.clone(),
            },
            RiseSyncDurableFailure::Transport { code } => Self::Retry { code: code.clone() },
        }
    }
}

pub trait RiseSyncDurableEndpoint: 'static {
    type Payload: Serialize + DeserializeOwned + Send + Sync + 'static;
    type Method: RiseWireMethod<Request: DeserializeOwned + 'static, Response: 'static>;

    fn kind() -> &'static str;

    fn maximum_attempts() -> i64 {
        i64::try_from(RETRY_MAXIMUM_ATTEMPTS).unwrap_or(i64::MAX)
    }

    fn make_request(
        payload: &Self::Payload,
        context: &RiseSyncDurableContext,
    ) -> anyhow::Result<<Self::Method as RiseWireMethod>::Request>;

    fn reconcile(
        payload: &Self::Payload,
        response: &<Self::Method as RiseWireMethod>::Response,
        context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch>;

    fn terminal_reconciliation(
        payload: &Self::Payload,
        code: &str,
        context: &RiseSyncDurableContext,
    ) -> anyhow::Result<RiseStoreWriteBatch>;

    fn classify_failure(
        failure: &RiseSyncDurableFailure,
        _context: &RiseSyncDurableContext,
    ) -> RiseSyncDurableFailureDisposition {
        RiseSyncDurableFailureDisposition::standard(failure)
    }
}

type RiseSyncPrepareOperation = Arc<
    dyn Fn(AnyRiseSendable, RiseSyncDurableContext) -> BoxFuture<'static, anyhow::Result<Vec<u8>>>
        + Send
        + Sync,
>;

type RiseSyncExecuteOperation = Arc<
    dyn Fn(
            Vec<u8>,
            RiseSyncDurableContext,
            Arc<RiseWire>,
            CancellationToken,
        ) -> BoxFuture<'static, RiseSyncDurableWorkerOutcome>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct RiseSyncDurableHandler {
    pub kind: String,
    endpoint_type: TypeId,
    prepare_operation: RiseSyncPrepareOperation,
    execute: RiseSyncExecuteOperation,
}

impl RiseSyncDurableHandler {
    pub fn new<Endpoint: RiseSyncDurableEndpoint>() -> Result<Self, RiseSyncError> {
        let kind = trimming_whitespaces_and_newlines(Endpoint::kind()).to_string();
        if kind.is_empty() {
            return Err(RiseSyncError::InvalidDurableEndpointKind);
        }
        if Endpoint::maximum_attempts() <= 0 {
            return Err(RiseSyncError::InvalidDurableMaximumAttempts);
        }
        match <Endpoint::Method as RiseWireMethod>::descriptor().replay_policy {
            RiseWireReplayPolicy::RequestID | RiseWireReplayPolicy::IdempotencyKey { .. } => {}
            RiseWireReplayPolicy::Never | RiseWireReplayPolicy::ReadOnly => {
                return Err(RiseSyncError::DurableEndpointRequiresIdempotencyKey(kind));
            }
        }
        let mismatch_kind = kind.clone();
        let prepare_operation: RiseSyncPrepareOperation = Arc::new(move |boxed, context| {
            let kind = mismatch_kind.clone();
            async move {
                let payload = boxed
                    .value
                    .downcast::<Endpoint::Payload>()
                    .map_err(|_| RiseSyncError::DurableEndpointTypeMismatch(kind))?;
                prepare_operation_for::<Endpoint>(*payload, context).await
            }
            .boxed()
        });
        let execute: RiseSyncExecuteOperation = Arc::new(|data, context, wire, cancel| {
            execute_for::<Endpoint>(data, context, wire, cancel).boxed()
        });
        Ok(Self {
            kind,
            endpoint_type: TypeId::of::<Endpoint>(),
            prepare_operation,
            execute,
        })
    }
}

pub(crate) struct AnyRiseSendable {
    value: Box<dyn Any + Send>,
}

#[derive(Serialize, Deserialize)]
struct RiseSyncPersistedDurableOperation {
    format_version: i64,
    payload: String,
    request: String,
}

impl RiseSyncPersistedDurableOperation {
    const CURRENT_FORMAT_VERSION: i64 = 1;

    fn new(payload: String, request: String) -> Self {
        Self {
            format_version: Self::CURRENT_FORMAT_VERSION,
            payload,
            request,
        }
    }

    fn decode(data: &[u8]) -> anyhow::Result<Self> {
        let persisted: Self = serde_json::from_slice(data)?;
        if persisted.format_version != Self::CURRENT_FORMAT_VERSION {
            return Err(
                RiseSyncError::UnsupportedDurableOperationFormat(persisted.format_version).into(),
            );
        }
        Ok(persisted)
    }
}

impl RiseDurableOperation {
    /// decodes the payload as the endpoint payload
    /// the stored one is an envelope (payload plus prepared request) so no direct decode
    pub fn decoded_payload<Endpoint: RiseSyncDurableEndpoint>(
        &self,
    ) -> anyhow::Result<Endpoint::Payload> {
        let persisted = RiseSyncPersistedDurableOperation::decode(&self.payload)?;
        Ok(serde_json::from_str(&persisted.payload)?)
    }
}

pub(crate) enum RiseSyncDurableWorkerOutcome {
    Acknowledged(RiseStoreWriteBatch),
    Paused,
    Deferred { code: String, delay_milliseconds: i64 },
    Retry { code: String },
    ReconciliationFailed { code: String },
    Terminal { code: String, reconciliation: RiseStoreWriteBatch },
}

struct RiseSyncDurableWorkerCompletion {
    claim: RiseDurableClaim,
    outcome: RiseSyncDurableWorkerOutcome,
}

#[derive(Clone)]
struct RiseSyncPendingDurableClaimRecovery {
    request_id: RiseRequestID,
    kind: String,
    claim_token: String,
    attempt: i64,
    next_attempt_at_milliseconds: i64,
    generation: u64,
    failure_count: i64,
    next_recovery_at_milliseconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseDurableSubmissionReceipt {
    pub request_id: RiseRequestID,
    pub commit: RiseStoreCommit,
}

impl RiseDurableSubmissionReceipt {
    pub fn new(request_id: RiseRequestID, commit: RiseStoreCommit) -> Self {
        Self { request_id, commit }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseDurableLifecycleResult {
    Acknowledged,
    Paused,
    Deferred,
    RetryableFailure,
    ReconciliationFailed,
    TerminalFailure,
    StoreCommitFailed,
}

impl RiseDurableLifecycleResult {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Acknowledged => "acknowledged",
            Self::Paused => "paused",
            Self::Deferred => "deferred",
            Self::RetryableFailure => "retryableFailure",
            Self::ReconciliationFailed => "reconciliationFailed",
            Self::TerminalFailure => "terminalFailure",
            Self::StoreCommitFailed => "storeCommitFailed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseDurableLifecycleStreamTermination {
    SubscriberEvicted,
    EngineShutdown,
}

impl RiseDurableLifecycleStreamTermination {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::SubscriberEvicted => "subscriberEvicted",
            Self::EngineShutdown => "engineShutdown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseDurableLifecycleEvent {
    Gap,
    StreamTerminated {
        reason: RiseDurableLifecycleStreamTermination,
    },
    Enqueued {
        request_id: RiseRequestID,
        kind: String,
        store_revision: i64,
    },
    AttemptStarted {
        request_id: RiseRequestID,
        kind: String,
        attempt: i64,
    },
    AttemptFinished {
        request_id: RiseRequestID,
        kind: String,
        attempt: i64,
        result: RiseDurableLifecycleResult,
    },
    ReconciliationCommitted {
        request_id: RiseRequestID,
        kind: String,
        attempt: i64,
        result: RiseDurableLifecycleResult,
        store_revision: i64,
    },
    RetryScheduled {
        request_id: RiseRequestID,
        kind: String,
        attempt: i64,
        result: RiseDurableLifecycleResult,
        store_revision: i64,
    },
    Terminal {
        request_id: RiseRequestID,
        kind: String,
        attempt: i64,
        result: RiseDurableLifecycleResult,
        store_revision: i64,
    },
    ReconciliationFailed {
        request_id: RiseRequestID,
        kind: String,
        attempt: i64,
        result: RiseDurableLifecycleResult,
    },
}

struct RiseDurableLifecycleBuffer {
    events: VecDeque<RiseDurableLifecycleEvent>,
    finished: bool,
    terminated: bool,
}

struct RiseDurableLifecycleShared {
    buffer: Mutex<RiseDurableLifecycleBuffer>,
    wake: Notify,
}

/// event stream with a newest first buffer, a full buffer drops its oldest
/// and reports the yield as dropped so Gap can follow at the tail
struct RiseDurableLifecycleContinuation {
    shared: Arc<RiseDurableLifecycleShared>,
    capacity: usize,
}

impl RiseDurableLifecycleContinuation {
    fn new(capacity: usize) -> (Self, RiseDurableLifecycleEvents) {
        let shared = Arc::new(RiseDurableLifecycleShared {
            buffer: Mutex::new(RiseDurableLifecycleBuffer {
                events: VecDeque::new(),
                finished: false,
                terminated: false,
            }),
            wake: Notify::new(),
        });
        let stream = RiseDurableLifecycleEvents {
            shared: Arc::clone(&shared),
        };
        (Self { shared, capacity }, stream)
    }

    /// true if the yield was dropped, buffer was full and lost its oldest event
    fn yield_event(&self, event: RiseDurableLifecycleEvent) -> bool {
        let dropped = {
            let mut buffer = self.shared.buffer.lock();
            if buffer.finished || buffer.terminated {
                return false;
            }
            buffer.events.push_back(event);
            let overflow = buffer.events.len() > self.capacity.max(1);
            if overflow {
                buffer.events.pop_front();
            }
            overflow
        };
        self.shared.wake.notify_one();
        dropped
    }

    fn finish(&self) {
        self.shared.buffer.lock().finished = true;
        self.shared.wake.notify_one();
    }

    fn is_terminated(&self) -> bool {
        self.shared.buffer.lock().terminated
    }
}

impl Drop for RiseDurableLifecycleContinuation {
    fn drop(&mut self) {
        self.finish();
    }
}

/// stream of durable lifecycle events, buffered ones drain first then a finished stream ends
pub struct RiseDurableLifecycleEvents {
    shared: Arc<RiseDurableLifecycleShared>,
}

impl RiseDurableLifecycleEvents {
    pub async fn next(&mut self) -> Option<RiseDurableLifecycleEvent> {
        loop {
            {
                let mut buffer = self.shared.buffer.lock();
                if let Some(event) = buffer.events.pop_front() {
                    return Some(event);
                }
                if buffer.finished {
                    return None;
                }
            }
            self.shared.wake.notified().await;
        }
    }
}

impl Drop for RiseDurableLifecycleEvents {
    fn drop(&mut self) {
        let mut buffer = self.shared.buffer.lock();
        buffer.terminated = true;
        buffer.events.clear();
    }
}

async fn prepare_operation_for<Endpoint: RiseSyncDurableEndpoint>(
    payload: Endpoint::Payload,
    context: RiseSyncDurableContext,
) -> anyhow::Result<Vec<u8>> {
    let work = RiseAccountRuntime::current().spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let request = Endpoint::make_request(&payload, &context)?;
        validate_idempotency_key(
            &request,
            &<Endpoint::Method as RiseWireMethod>::descriptor(),
            &context.idempotency_key,
        )?;
        let persisted = RiseSyncPersistedDurableOperation::new(
            serde_json::to_string(&payload)?,
            serde_json::to_string(&request)?,
        );
        Ok(serde_json::to_vec(&persisted)?)
    });
    work.await?
}

type RiseSyncPreparedOperation<Endpoint> = (
    <Endpoint as RiseSyncDurableEndpoint>::Payload,
    <<Endpoint as RiseSyncDurableEndpoint>::Method as RiseWireMethod>::Request,
);

async fn decode_prepared_operation<Endpoint: RiseSyncDurableEndpoint>(
    operation_data: Vec<u8>,
    context: RiseSyncDurableContext,
) -> anyhow::Result<RiseSyncPreparedOperation<Endpoint>> {
    let work = RiseAccountRuntime::current().spawn_blocking(
        move || -> anyhow::Result<RiseSyncPreparedOperation<Endpoint>> {
            let persisted = RiseSyncPersistedDurableOperation::decode(&operation_data)?;
            let payload: Endpoint::Payload = serde_json::from_str(&persisted.payload)?;
            let request: <Endpoint::Method as RiseWireMethod>::Request =
                serde_json::from_str(&persisted.request)?;
            validate_idempotency_key(
                &request,
                &<Endpoint::Method as RiseWireMethod>::descriptor(),
                &context.idempotency_key,
            )?;
            Ok((payload, request))
        },
    );
    work.await?
}

async fn execute_for<Endpoint: RiseSyncDurableEndpoint>(
    payload_data: Vec<u8>,
    context: RiseSyncDurableContext,
    wire: Arc<RiseWire>,
    cancel: CancellationToken,
) -> RiseSyncDurableWorkerOutcome {
    let Ok((payload, request)) =
        decode_prepared_operation::<Endpoint>(payload_data, context.clone()).await
    else {
        return RiseSyncDurableWorkerOutcome::ReconciliationFailed {
            code: RiseSyncFailureCode::InvalidPayload.wire().to_string(),
        };
    };

    let called = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(anyhow::Error::new(CancellationError)),
        called = wire.call::<Endpoint::Method>(
            &request,
            Some(context.request_id),
            RiseWireRequestMetadata::default(),
        ) => called,
    };
    let response = match called {
        Ok(response) => response,
        Err(error) => {
            let failure = failure_from(&error);
            match Endpoint::classify_failure(&failure, &context) {
                RiseSyncDurableFailureDisposition::Pause => {
                    return RiseSyncDurableWorkerOutcome::Paused;
                }
                RiseSyncDurableFailureDisposition::Retry { code } => {
                    if context.attempt_count.saturating_add(1) >= Endpoint::maximum_attempts() {
                        return terminal_for::<Endpoint>(
                            payload,
                            RiseSyncFailureCode::AttemptLimit.wire().to_string(),
                            context,
                        )
                        .await;
                    }
                    return RiseSyncDurableWorkerOutcome::Retry { code };
                }
                RiseSyncDurableFailureDisposition::Terminal { code } => {
                    return terminal_for::<Endpoint>(payload, code, context).await;
                }
            }
        }
    };

    let work = RiseAccountRuntime::current()
        .spawn_blocking(move || Endpoint::reconcile(&payload, &response, &context));
    match work.await {
        Ok(Ok(batch)) => RiseSyncDurableWorkerOutcome::Acknowledged(batch),
        Ok(Err(_)) | Err(_) => RiseSyncDurableWorkerOutcome::ReconciliationFailed {
            code: RiseSyncFailureCode::ReconciliationFailed.wire().to_string(),
        },
    }
}

async fn terminal_for<Endpoint: RiseSyncDurableEndpoint>(
    payload: Endpoint::Payload,
    code: String,
    context: RiseSyncDurableContext,
) -> RiseSyncDurableWorkerOutcome {
    let reconciliation_code = code.clone();
    let work = RiseAccountRuntime::current().spawn_blocking(move || {
        Endpoint::terminal_reconciliation(&payload, &reconciliation_code, &context)
    });
    match work.await {
        Ok(Ok(reconciliation)) => RiseSyncDurableWorkerOutcome::Terminal {
            code,
            reconciliation,
        },
        Ok(Err(_)) | Err(_) => RiseSyncDurableWorkerOutcome::ReconciliationFailed {
            code: RiseSyncFailureCode::ReconciliationFailed.wire().to_string(),
        },
    }
}

fn failure_from(error: &anyhow::Error) -> RiseSyncDurableFailure {
    if error.is::<CancellationError>() {
        return RiseSyncDurableFailure::Cancelled;
    }
    if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
        return RiseSyncDurableFailure::Remote(remote.clone());
    }
    let Some(wire) = error.downcast_ref::<RiseWireError>() else {
        return RiseSyncDurableFailure::Transport {
            code: RiseSyncFailureCode::Transport.wire().to_string(),
        };
    };
    match wire {
        RiseWireError::NoSocketSession
        | RiseWireError::NotConnected
        | RiseWireError::Suspended
        | RiseWireError::StaleAccount
        | RiseWireError::StaleSession
        | RiseWireError::Disconnected { .. }
        | RiseWireError::Shutdown => RiseSyncDurableFailure::Unavailable,
        RiseWireError::Timeout { .. } => RiseSyncDurableFailure::Timeout,
        RiseWireError::MissingResponseData | RiseWireError::ResponseDecodingFailed => {
            RiseSyncDurableFailure::MalformedResponse
        }
        RiseWireError::UnsafeRequestIDReuse { .. } => RiseSyncDurableFailure::UnsafeReplay,
        RiseWireError::PendingLimitReached { .. }
        | RiseWireError::DuplicateRequestID { .. }
        | RiseWireError::ExplicitRequestIdentityLimitReached { .. }
        | RiseWireError::OutboundFrameTooLarge { .. }
        | RiseWireError::InboundFrameTooLarge { .. }
        | RiseWireError::EventSubscriberLimitReached { .. } => RiseSyncDurableFailure::Transport {
            code: RiseSyncFailureCode::Transport.wire().to_string(),
        },
    }
}

fn validate_idempotency_key<Request: Serialize>(
    request: &Request,
    descriptor: &RiseWireMethodDescriptor,
    expected: &str,
) -> anyhow::Result<()> {
    match &descriptor.replay_policy {
        RiseWireReplayPolicy::RequestID => Ok(()),
        RiseWireReplayPolicy::IdempotencyKey { field } => {
            let root = RiseJSONValue::encode(request)?;
            let value = field
                .split('.')
                .try_fold(&root, |partial, component| partial.subscript_key(component));
            if value.and_then(RiseJSONValue::string_value) != Some(expected) {
                return Err(RiseSyncError::InvalidDurableIdempotencyKey {
                    field: field.clone(),
                }
                .into());
            }
            Ok(())
        }
        RiseWireReplayPolicy::Never | RiseWireReplayPolicy::ReadOnly => {
            Err(RiseSyncError::DurableEndpointRequiresIdempotencyKey(format!(
                "{}.{}",
                descriptor.service, descriptor.method
            ))
            .into())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseSyncHealthEvent {
    Started,
    Stopped,
    ResyncRequired { reason: String },
    DurableOperationFailed { request_id: RiseRequestID, code: String },
}

/// stream of health events, newest 32 kept
/// a lagged receiver skips dropped ones
pub struct RiseSyncHealthEvents {
    receiver: broadcast::Receiver<RiseSyncHealthEvent>,
}

impl RiseSyncHealthEvents {
    pub async fn next(&mut self) -> Option<RiseSyncHealthEvent> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => return Some(event),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return None,
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseSyncError {
    #[error("sync is not running")]
    NotRunning,
    #[error("sync generation changed")]
    StaleGeneration,
    #[error("duplicate push handler {0}")]
    DuplicatePushHandler(String),
    #[error("duplicate durable handler {0}")]
    DuplicateDurableHandler(String),
    #[error("unknown durable handler {0}")]
    UnknownDurableHandler(String),
    #[error("durable endpoint kind is empty")]
    InvalidDurableEndpointKind,
    #[error("durable endpoint maximum attempts must be positive")]
    InvalidDurableMaximumAttempts,
    #[error("durable endpoint {0} requires an idempotency key")]
    DurableEndpointRequiresIdempotencyKey(String),
    #[error("durable endpoint type mismatch for {0}")]
    DurableEndpointTypeMismatch(String),
    #[error("durable idempotency key field {field} does not carry the request id")]
    InvalidDurableIdempotencyKey { field: String },
    #[error("unsupported durable operation format {0}")]
    UnsupportedDurableOperationFormat(i64),
    #[error("sync is shut down")]
    Shutdown,
}

pub type RiseSyncPushObserver = Arc<dyn Fn(&RiseWirePushEnvelope) + Send + Sync>;

pub type RiseSyncResyncHandler = Arc<dyn Fn(String) -> BoxFuture<'static, ()> + Send + Sync>;

pub type RiseSyncSleep =
    Arc<dyn Fn(Duration) -> BoxFuture<'static, anyhow::Result<()>> + Send + Sync>;

pub(crate) type RiseSyncClaimRecoveryWrite = Arc<
    dyn Fn(Arc<RiseStore>, RiseStoreWriteBatch) -> BoxFuture<'static, anyhow::Result<RiseStoreCommit>>
        + Send
        + Sync,
>;

static PUSH_OBSERVER: RwLock<Option<RiseSyncPushObserver>> = parking_lot::const_rwlock(None);

/// defaulted sync config params, canon values unless a test overrides
#[derive(Clone)]
pub(crate) struct RiseSyncConfiguration {
    pub(crate) maximum_concurrent_durable_operations: usize,
    pub(crate) maximum_push_batch_events: usize,
    pub(crate) maximum_push_batch_mutations: usize,
    pub(crate) maximum_push_batch_bytes: usize,
    pub(crate) push_batch_delay: Duration,
    pub(crate) push_batch_sleep: RiseSyncSleep,
    pub(crate) clock: Arc<dyn RiseEngineClock>,
    pub(crate) durable_claim_recovery_write: RiseSyncClaimRecoveryWrite,
}

impl Default for RiseSyncConfiguration {
    fn default() -> Self {
        Self {
            maximum_concurrent_durable_operations: canon_count(
                OUTBOX_MAXIMUM_CONCURRENT_OPERATIONS,
            ),
            maximum_push_batch_events: canon_count(PUSH_BATCH_MAXIMUM_EVENTS),
            maximum_push_batch_mutations: canon_count(PUSH_BATCH_MAXIMUM_MUTATIONS),
            maximum_push_batch_bytes: canon_count(PUSH_BATCH_MAXIMUM_BYTES),
            push_batch_delay: Duration::from_millis(PUSH_BATCH_WINDOW_MS),
            push_batch_sleep: Arc::new(default_push_batch_sleep),
            clock: Arc::new(RiseSystemClock),
            durable_claim_recovery_write: Arc::new(default_durable_claim_recovery_write),
        }
    }
}

fn canon_count(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn default_push_batch_sleep(delay: Duration) -> BoxFuture<'static, anyhow::Result<()>> {
    async move {
        RiseRuntimeTimer::sleep(delay).await;
        Ok(())
    }
    .boxed()
}

fn default_durable_claim_recovery_write(
    store: Arc<RiseStore>,
    batch: RiseStoreWriteBatch,
) -> BoxFuture<'static, anyhow::Result<RiseStoreCommit>> {
    async move { Ok(store.write(batch).await?) }.boxed()
}

/// defaulted params of durable submit
pub struct RiseSyncSubmitOptions {
    pub ordering_key: RiseDurableOrderingKey,
    pub request_id: Option<RiseRequestID>,
    pub optimistic: RiseStoreWriteBatch,
}

impl Default for RiseSyncSubmitOptions {
    fn default() -> Self {
        Self {
            ordering_key: RiseDurableOrderingKey::global(),
            request_id: None,
            optimistic: RiseStoreWriteBatch::empty(),
        }
    }
}

type RiseSyncStartFuture = Shared<BoxFuture<'static, Result<(), Arc<anyhow::Error>>>>;

struct RiseSyncStartTask {
    future: RiseSyncStartFuture,
    cancel: CancellationToken,
}

struct RiseSyncSpawnedTask {
    handle: JoinHandle<()>,
    cancel: CancellationToken,
}

#[derive(Default)]
struct RiseSyncState {
    push_handlers: HashMap<String, RiseSyncPushHandler>,
    durable_handlers: HashMap<String, RiseSyncDurableHandler>,
    start_task: Option<RiseSyncStartTask>,
    start_task_id: Option<u64>,
    event_task: Option<RiseSyncSpawnedTask>,
    replay_task: Option<RiseSyncSpawnedTask>,
    replay_task_id: Option<u64>,
    replay_task_is_sleeping: bool,
    requested_replay_delay: Option<Duration>,
    pending_durable_claim_recoveries: HashMap<RiseRequestID, RiseSyncPendingDurableClaimRecovery>,
    generation: u64,
    is_starting: bool,
    is_running: bool,
    is_stopping: bool,
    is_shutdown: bool,
    stop_waiters: Vec<oneshot::Sender<()>>,
    health: Option<broadcast::Sender<RiseSyncHealthEvent>>,
    durable_lifecycle_continuations: HashMap<u64, RiseDurableLifecycleContinuation>,
    durable_lifecycle_continuation_order: Vec<u64>,
    resync_handler: Option<RiseSyncResyncHandler>,
    pending_push_batches: Vec<RiseStoreWriteBatch>,
    pending_push_mutation_count: usize,
    pending_push_estimated_bytes: usize,
    push_flush_task: Option<RiseSyncSpawnedTask>,
    push_flush_task_id: Option<u64>,
    push_commits_in_flight: usize,
    push_commit_waiters: Vec<oneshot::Sender<()>>,
    next_task_identifier: u64,
}

impl RiseSyncState {
    fn next_identifier(&mut self) -> u64 {
        self.next_task_identifier = self.next_task_identifier.wrapping_add(1);
        self.next_task_identifier
    }

    fn is_current(&self, generation: u64) -> bool {
        self.is_running && self.generation == generation
    }

    fn cancel_scheduled_push_flush(&mut self) {
        if let Some(task) = self.push_flush_task.take() {
            task.cancel.cancel();
        }
        self.push_flush_task_id = None;
    }

    fn reset_pending_push_batches(&mut self) {
        self.pending_push_batches.clear();
        self.pending_push_mutation_count = 0;
        self.pending_push_estimated_bytes = 0;
    }

    fn request_replay_after_current(&mut self, delay: Duration) {
        self.requested_replay_delay = Some(
            self.requested_replay_delay
                .map_or(delay, |requested| requested.min(delay)),
        );
    }

    fn publish_health(&self, event: RiseSyncHealthEvent) {
        if let Some(health) = &self.health {
            let _ = health.send(event);
        }
    }

    fn publish_durable_lifecycle(&mut self, event: impl FnOnce() -> RiseDurableLifecycleEvent) {
        self.prune_durable_lifecycle_continuations();
        if self.durable_lifecycle_continuations.is_empty() {
            return;
        }
        let value = event();
        for continuation in self.durable_lifecycle_continuations.values() {
            if continuation.yield_event(value.clone()) {
                continuation.yield_event(RiseDurableLifecycleEvent::Gap);
            }
        }
    }

    fn prune_durable_lifecycle_continuations(&mut self) {
        self.durable_lifecycle_continuations
            .retain(|_, continuation| !continuation.is_terminated());
        let live = &self.durable_lifecycle_continuations;
        self.durable_lifecycle_continuation_order
            .retain(|identifier| live.contains_key(identifier));
    }

    fn finish_stopping(&mut self) {
        self.is_stopping = false;
        for waiter in self.stop_waiters.drain(..) {
            let _ = waiter.send(());
        }
    }
}

enum RiseSyncStartBegin {
    AlreadyRunning,
    Join(RiseSyncStartFuture),
    Own(u64, RiseSyncStartFuture),
}

enum RiseSyncStopBegin {
    Nothing,
    Wait(oneshot::Receiver<()>),
    Stop {
        should_flush_accepted_pushes: bool,
        starting: Option<RiseSyncStartFuture>,
        events: Option<RiseSyncSpawnedTask>,
        replay: Option<RiseSyncSpawnedTask>,
        push_flush: Option<RiseSyncSpawnedTask>,
    },
}

enum RiseSyncRecoveryStep {
    Skip,
    Abort,
    Wait(Duration),
    Write(RiseSyncPendingDurableClaimRecovery),
}

pub struct RiseSync {
    pub wire: Arc<RiseWire>,
    store: Arc<RiseStore>,
    allocator: &'static RiseRequestIDAllocator,
    maximum_concurrent_durable_operations: usize,
    maximum_push_batch_events: usize,
    maximum_push_batch_mutations: usize,
    maximum_push_batch_bytes: usize,
    push_batch_delay: Duration,
    push_batch_sleep: RiseSyncSleep,
    clock: Arc<dyn RiseEngineClock>,
    durable_claim_recovery_write: RiseSyncClaimRecoveryWrite,
    state: RiseIsolated<RiseSyncState>,
    this: Weak<RiseSync>,
}

impl RiseSync {
    pub fn push_observer() -> Option<RiseSyncPushObserver> {
        PUSH_OBSERVER.read().clone()
    }

    pub fn set_push_observer(observer: Option<RiseSyncPushObserver>) {
        *PUSH_OBSERVER.write() = observer;
    }

    pub(crate) fn new(
        store: Arc<RiseStore>,
        wire: Arc<RiseWire>,
        allocator: &'static RiseRequestIDAllocator,
    ) -> Arc<Self> {
        Self::with_configuration(store, wire, allocator, RiseSyncConfiguration::default())
    }

    pub(crate) fn with_configuration(
        store: Arc<RiseStore>,
        wire: Arc<RiseWire>,
        allocator: &'static RiseRequestIDAllocator,
        configuration: RiseSyncConfiguration,
    ) -> Arc<Self> {
        assert!(configuration.maximum_concurrent_durable_operations > 0);
        assert!(configuration.maximum_push_batch_events > 0);
        assert!(configuration.maximum_push_batch_mutations > 0);
        assert!(configuration.maximum_push_batch_bytes > 0);
        let (health, _) = broadcast::channel(HEALTH_BUFFER_SIZE);
        Arc::new_cyclic(|this| Self {
            wire,
            store,
            allocator,
            maximum_concurrent_durable_operations: configuration
                .maximum_concurrent_durable_operations,
            maximum_push_batch_events: configuration.maximum_push_batch_events,
            maximum_push_batch_mutations: configuration.maximum_push_batch_mutations,
            maximum_push_batch_bytes: configuration.maximum_push_batch_bytes,
            push_batch_delay: configuration.push_batch_delay,
            push_batch_sleep: configuration.push_batch_sleep,
            clock: configuration.clock,
            durable_claim_recovery_write: configuration.durable_claim_recovery_write,
            state: RiseIsolated::new(RiseSyncState {
                health: Some(health),
                ..RiseSyncState::default()
            }),
            this: this.clone(),
        })
    }

    pub async fn health_events(&self) -> RiseSyncHealthEvents {
        let receiver = self.state.with(|state| {
            state
                .health
                .as_ref()
                .filter(|_| !state.is_shutdown)
                .map(broadcast::Sender::subscribe)
        });
        let receiver = receiver.unwrap_or_else(|| broadcast::channel(1).1);
        RiseSyncHealthEvents { receiver }
    }

    pub async fn durable_lifecycle_events(&self) -> RiseDurableLifecycleEvents {
        self.state.with(|state| {
            if state.is_shutdown {
                let (continuation, stream) = RiseDurableLifecycleContinuation::new(1);
                continuation.yield_event(RiseDurableLifecycleEvent::StreamTerminated {
                    reason: RiseDurableLifecycleStreamTermination::EngineShutdown,
                });
                continuation.finish();
                return stream;
            }
            state.prune_durable_lifecycle_continuations();
            if state.durable_lifecycle_continuation_order.len()
                >= MAXIMUM_DURABLE_LIFECYCLE_SUBSCRIBERS
                && !state.durable_lifecycle_continuation_order.is_empty()
            {
                let oldest = state.durable_lifecycle_continuation_order.remove(0);
                if let Some(evicted) = state.durable_lifecycle_continuations.remove(&oldest) {
                    evicted.yield_event(RiseDurableLifecycleEvent::StreamTerminated {
                        reason: RiseDurableLifecycleStreamTermination::SubscriberEvicted,
                    });
                    evicted.finish();
                }
            }
            let identifier = state.next_identifier();
            let (continuation, stream) =
                RiseDurableLifecycleContinuation::new(DURABLE_LIFECYCLE_BUFFER_SIZE);
            state
                .durable_lifecycle_continuations
                .insert(identifier, continuation);
            state.durable_lifecycle_continuation_order.push(identifier);
            stream
        })
    }

    pub async fn register_push_handler(
        &self,
        push_handler: RiseSyncPushHandler,
    ) -> Result<(), RiseSyncError> {
        self.state.with(|state| {
            if state.push_handlers.contains_key(&push_handler.event_type) {
                return Err(RiseSyncError::DuplicatePushHandler(push_handler.event_type));
            }
            state
                .push_handlers
                .insert(push_handler.event_type.clone(), push_handler);
            Ok(())
        })
    }

    pub async fn register_durable_handler(
        &self,
        durable_handler: RiseSyncDurableHandler,
    ) -> Result<(), RiseSyncError> {
        self.state.with(|state| {
            if state.durable_handlers.contains_key(&durable_handler.kind) {
                return Err(RiseSyncError::DuplicateDurableHandler(durable_handler.kind));
            }
            state
                .durable_handlers
                .insert(durable_handler.kind.clone(), durable_handler);
            Ok(())
        })
    }

    pub async fn register_durable_endpoint<Endpoint: RiseSyncDurableEndpoint>(
        &self,
    ) -> Result<(), RiseSyncError> {
        self.register_durable_handler(RiseSyncDurableHandler::new::<Endpoint>()?)
            .await
    }

    pub async fn on_resync_required<Handler, Handled>(&self, handler: Handler)
    where
        Handler: Fn(String) -> Handled + Send + Sync + 'static,
        Handled: Future<Output = ()> + Send + 'static,
    {
        let handler: RiseSyncResyncHandler = Arc::new(move |reason| handler(reason).boxed());
        self.state.with(|state| state.resync_handler = Some(handler));
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        let runtime = RiseAccountRuntime::current();
        let weak = self.this.clone();
        let begin = self.state.with(|state| -> anyhow::Result<RiseSyncStartBegin> {
            if state.is_shutdown {
                return Err(RiseSyncError::Shutdown.into());
            }
            if state.is_stopping {
                return Err(CancellationError.into());
            }
            if state.is_running {
                return Ok(RiseSyncStartBegin::AlreadyRunning);
            }
            if let Some(task) = &state.start_task {
                return Ok(RiseSyncStartBegin::Join(task.future.clone()));
            }
            let identifier = state.next_identifier();
            let cancel = CancellationToken::new();
            let task_cancel = cancel.clone();
            let handle = runtime.spawn(async move {
                match weak.upgrade() {
                    Some(this) => this.perform_start(&task_cancel).await.map_err(Arc::new),
                    None => Ok(()),
                }
            });
            let future = handle
                .map(|joined| joined.unwrap_or_else(|error| Err(Arc::new(error.into()))))
                .boxed()
                .shared();
            state.start_task = Some(RiseSyncStartTask {
                future: future.clone(),
                cancel,
            });
            state.start_task_id = Some(identifier);
            Ok(RiseSyncStartBegin::Own(identifier, future))
        })?;
        match begin {
            RiseSyncStartBegin::AlreadyRunning => Ok(()),
            RiseSyncStartBegin::Join(future) => future.await.map_err(|error| rethrow(&error)),
            RiseSyncStartBegin::Own(identifier, future) => {
                let result = future.await;
                self.clear_start_task(identifier);
                result.map_err(|error| rethrow(&error))
            }
        }
    }

    async fn perform_start(&self, cancel: &CancellationToken) -> anyhow::Result<()> {
        let active_generation = self.state.with(|state| {
            state.generation = state.generation.wrapping_add(1);
            state.is_starting = true;
            state.generation
        });
        let mut acquired_subscription: Option<RiseWireEventSubscription> = None;
        let started = self
            .perform_start_steps(cancel, active_generation, &mut acquired_subscription)
            .await;
        if let Err(error) = started {
            if let Some(subscription) = acquired_subscription {
                subscription.cancel().await;
            }
            self.state.with(|state| {
                if state.generation == active_generation {
                    state.is_starting = false;
                }
            });
            return Err(error);
        }
        if self.wire.is_ready_for_calls(false).await {
            self.schedule_replay(Duration::ZERO, active_generation);
        }
        Ok(())
    }

    async fn perform_start_steps(
        &self,
        cancel: &CancellationToken,
        active_generation: u64,
        acquired_subscription: &mut Option<RiseWireEventSubscription>,
    ) -> anyhow::Result<()> {
        self.state
            .with(|state| state.pending_durable_claim_recoveries.clear());
        self.store.recover_in_flight_operations(None).await?;
        let existing_terminal_operations = self.store.terminal_operations().await?;
        check_cancellation(cancel)?;
        self.ensure_starting(active_generation)?;
        *acquired_subscription = Some(self.wire.event_subscription().await?);
        check_cancellation(cancel)?;
        self.ensure_starting(active_generation)?;
        let Some(mut subscription) = acquired_subscription.take() else {
            return Err(RiseSyncError::StaleGeneration.into());
        };
        let runtime = RiseAccountRuntime::current();
        let weak = self.this.clone();
        let started = self.state.with(move |state| {
            if !(state.is_starting && state.generation == active_generation) {
                return Err(subscription);
            }
            state.is_starting = false;
            state.is_running = true;
            let event_cancel = CancellationToken::new();
            let task_cancel = event_cancel.clone();
            let handle = runtime.spawn(async move {
                loop {
                    let event = tokio::select! {
                        biased;
                        _ = task_cancel.cancelled() => None,
                        event = subscription.stream.next() => event,
                    };
                    let Some(event) = event else { break };
                    if task_cancel.is_cancelled() {
                        break;
                    }
                    let Some(this) = weak.upgrade() else { break };
                    this.handle(event, active_generation).await;
                }
                subscription.cancel().await;
            });
            state.event_task = Some(RiseSyncSpawnedTask {
                handle,
                cancel: event_cancel,
            });
            state.publish_health(RiseSyncHealthEvent::Started);
            for operation in &existing_terminal_operations {
                state.publish_health(RiseSyncHealthEvent::DurableOperationFailed {
                    request_id: operation.request_id,
                    code: operation
                        .last_error_code
                        .clone()
                        .unwrap_or_else(|| RiseSyncFailureCode::Remote.wire().to_string()),
                });
            }
            Ok(())
        });
        if let Err(subscription) = started {
            *acquired_subscription = Some(subscription);
            return Err(RiseSyncError::StaleGeneration.into());
        }
        Ok(())
    }

    fn ensure_starting(&self, active_generation: u64) -> Result<(), RiseSyncError> {
        let starting = self
            .state
            .with(|state| state.is_starting && state.generation == active_generation);
        if starting {
            Ok(())
        } else {
            Err(RiseSyncError::StaleGeneration)
        }
    }

    pub async fn stop(&self) {
        let begin = self.state.with(|state| {
            if state.is_stopping {
                let (waiter, finished) = oneshot::channel();
                state.stop_waiters.push(waiter);
                return RiseSyncStopBegin::Wait(finished);
            }
            let has_work = state.is_running
                || state.is_starting
                || state.start_task.is_some()
                || state.push_flush_task.is_some()
                || state.push_commits_in_flight > 0
                || !state.pending_push_batches.is_empty()
                || !state.pending_durable_claim_recoveries.is_empty();
            if !has_work {
                return RiseSyncStopBegin::Nothing;
            }
            state.is_stopping = true;
            let should_flush_accepted_pushes = state.is_running;
            state.generation = state.generation.wrapping_add(1);
            state.is_starting = false;
            state.is_running = false;
            let starting = state.start_task.as_ref().map(|task| {
                task.cancel.cancel();
                task.future.clone()
            });
            let events = state.event_task.take();
            let replay = state.replay_task.take();
            let push_flush = state.push_flush_task.take();
            for task in [&events, &replay, &push_flush].into_iter().flatten() {
                task.cancel.cancel();
            }
            state.cancel_scheduled_push_flush();
            RiseSyncStopBegin::Stop {
                should_flush_accepted_pushes,
                starting,
                events,
                replay,
                push_flush,
            }
        });
        let (should_flush_accepted_pushes, starting, events, replay, push_flush) =
            match begin {
                RiseSyncStopBegin::Nothing => return,
                RiseSyncStopBegin::Wait(finished) => {
                    let _ = finished.await;
                    return;
                }
                RiseSyncStopBegin::Stop {
                    should_flush_accepted_pushes,
                    starting,
                    events,
                    replay,
                    push_flush,
                } => (
                    should_flush_accepted_pushes,
                    starting,
                    events,
                    replay,
                    push_flush,
                ),
            };

        if let Some(starting) = starting {
            let _ = starting.await;
        }
        for task in [events, replay, push_flush].into_iter().flatten() {
            let _ = task.handle.await;
        }

        let has_pending_pushes = self
            .state
            .with(|state| !state.pending_push_batches.is_empty());
        if should_flush_accepted_pushes || has_pending_pushes {
            self.flush_pending_push_batches_for_stop().await;
        } else {
            self.state.with(RiseSyncState::reset_pending_push_batches);
        }
        self.wait_for_push_commits_to_finish().await;

        self.state.with(|state| {
            state.start_task = None;
            state.start_task_id = None;
            state.event_task = None;
            state.replay_task = None;
            state.replay_task_id = None;
            state.replay_task_is_sleeping = false;
            state.requested_replay_delay = None;
            state.pending_durable_claim_recoveries.clear();
            state.finish_stopping();
            state.publish_health(RiseSyncHealthEvent::Stopped);
        });
    }

    pub(crate) async fn shutdown(&self) {
        let already_shut_down = self.state.with(|state| {
            let already_shut_down = state.is_shutdown;
            state.is_shutdown = true;
            already_shut_down
        });
        if already_shut_down {
            return;
        }
        self.stop().await;
        self.state.with(|state| {
            state.health = None;
            for (_, stream) in state.durable_lifecycle_continuations.drain() {
                stream.yield_event(RiseDurableLifecycleEvent::StreamTerminated {
                    reason: RiseDurableLifecycleStreamTermination::EngineShutdown,
                });
                stream.finish();
            }
            state.durable_lifecycle_continuation_order.clear();
            state.resync_handler = None;
            state.push_handlers.clear();
            state.durable_handlers.clear();
            state.reset_pending_push_batches();
        });
    }

    fn clear_start_task(&self, identifier: u64) {
        self.state.with(|state| {
            if state.start_task_id != Some(identifier) {
                return;
            }
            state.start_task = None;
            state.start_task_id = None;
        });
    }

    pub async fn apply<Input, Reducer>(
        &self,
        input: Input,
        reducer: Reducer,
    ) -> anyhow::Result<RiseStoreCommit>
    where
        Input: Send + 'static,
        Reducer: FnOnce(Input) -> anyhow::Result<RiseStoreWriteBatch> + Send + 'static,
    {
        let active_generation = self
            .state
            .with(|state| state.is_running.then_some(state.generation))
            .ok_or(RiseSyncError::NotRunning)?;
        let batch = RiseAccountRuntime::current()
            .spawn_blocking(move || reducer(input))
            .await??;
        if !self.is_current(active_generation) {
            return Err(RiseSyncError::StaleGeneration.into());
        }
        Ok(self.store.write(batch).await?)
    }

    pub async fn submit_durable<Endpoint: RiseSyncDurableEndpoint>(
        &self,
        payload: Endpoint::Payload,
        options: RiseSyncSubmitOptions,
    ) -> anyhow::Result<RiseRequestID> {
        let receipt = self
            .submit_durable_receipt::<Endpoint>(payload, options)
            .await?;
        Ok(receipt.request_id)
    }

    pub async fn submit_durable_receipt<Endpoint: RiseSyncDurableEndpoint>(
        &self,
        payload: Endpoint::Payload,
        options: RiseSyncSubmitOptions,
    ) -> anyhow::Result<RiseDurableSubmissionReceipt> {
        let kind = trimming_whitespaces_and_newlines(Endpoint::kind()).to_string();
        let handler = self
            .state
            .with(|state| state.durable_handlers.get(&kind).cloned())
            .ok_or_else(|| RiseSyncError::UnknownDurableHandler(kind.clone()))?;
        if handler.endpoint_type != TypeId::of::<Endpoint>() {
            return Err(RiseSyncError::DurableEndpointTypeMismatch(kind).into());
        }
        let request_id = options
            .request_id
            .unwrap_or_else(|| self.allocator.next(RiseClock::now()));
        let context = RiseSyncDurableContext::new(request_id, 0);
        let persisted_operation = (handler.prepare_operation)(
            AnyRiseSendable {
                value: Box::new(payload),
            },
            context,
        )
        .await?;
        let still_registered = self.state.with(|state| {
            state
                .durable_handlers
                .get(&kind)
                .is_some_and(|current| current.endpoint_type == handler.endpoint_type)
        });
        if !still_registered {
            return Err(RiseSyncError::DurableEndpointTypeMismatch(kind).into());
        }
        let now = self.now_milliseconds();
        let operation = RiseDurableOperation {
            request_id,
            kind: kind.clone(),
            payload: persisted_operation,
            ordering_key: options.ordering_key,
            state: RiseDurableOperationState::Queued,
            attempt_count: 0,
            next_attempt_at_milliseconds: now,
            last_error_code: None,
            created_at_milliseconds: now,
            updated_at_milliseconds: now,
        };
        let mut mutations = options.optimistic.mutations;
        mutations.push(RiseStoreMutation::PutDurableOperation(operation));
        let commit = self
            .store
            .write(RiseStoreWriteBatch::new(
                mutations,
                options.optimistic.invalidated_views,
            ))
            .await?;
        let store_revision = commit.revision;
        let (is_running, replay_busy, pending_recoveries) = self.state.with(|state| {
            (state.is_running, state.replay_task_id.is_some(), state.pending_durable_claim_recoveries.len())
        });
        tracing::warn!(
            target: "riseonly::X1586",
            "durable enqueued kind={kind} running={is_running} replay_busy={replay_busy} pending_recoveries={pending_recoveries} {}",
            self.wire.readiness_probe()
        );
        self.state.with(|state| {
            state.publish_durable_lifecycle(|| RiseDurableLifecycleEvent::Enqueued {
                request_id,
                kind,
                store_revision,
            });
        });
        if self.state.with(|state| state.is_running) && self.wire.is_ready_for_calls(false).await
        {
            let generation = self.state.with(|state| state.generation);
            self.schedule_replay(Duration::ZERO, generation);
        }
        Ok(RiseDurableSubmissionReceipt::new(request_id, commit))
    }

    pub async fn terminal_operations(&self, limit: i64) -> anyhow::Result<Vec<RiseDurableOperation>> {
        Ok(self.store.terminal_operations_limit(limit).await?)
    }

    pub async fn resolve_terminal_operation(
        &self,
        request_id: RiseRequestID,
        resolution: RiseDurableTerminalResolution,
    ) -> anyhow::Result<()> {
        let now = self.now_milliseconds();
        self.store
            .write(write_batch(vec![RiseStoreMutation::ResolveTerminalOperation {
                request_id,
                resolution: resolution.clone(),
                updated_at_milliseconds: now,
            }]))
            .await?;
        if !self.state.with(|state| state.is_running) || !self.wire.is_ready_for_calls(false).await
        {
            return Ok(());
        }
        match resolution {
            RiseDurableTerminalResolution::Remove => {}
            RiseDurableTerminalResolution::Requeue {
                next_attempt_at_milliseconds,
                ..
            } => {
                let generation = self.state.with(|state| state.generation);
                self.schedule_replay(
                    milliseconds(next_attempt_at_milliseconds - now),
                    generation,
                );
            }
        }
        Ok(())
    }

    async fn handle(&self, event: RiseWireEvent, active_generation: u64) {
        if !self.is_current(active_generation) {
            return;
        }
        match event {
            RiseWireEvent::Connected(..) | RiseWireEvent::Resumed(..) => {
                if self.wire.is_ready_for_calls(false).await {
                    self.schedule_replay(Duration::ZERO, active_generation);
                }
            }
            RiseWireEvent::Push(push, _) => {
                if let Some(observer) = Self::push_observer() {
                    observer(&push);
                }
                let Some(handler) = self
                    .state
                    .with(|state| state.push_handlers.get(&push.r#type).cloned())
                else {
                    return;
                };
                let event_type = push.r#type.clone();
                match (handler.reduce)(push).await {
                    Ok(batch) => {
                        let Some(batch) = batch else { return };
                        if !self.is_current(active_generation) {
                            return;
                        }
                        self.enqueue_push_batch(batch, active_generation).await;
                    }
                    Err(_) => {
                        if !self.is_current(active_generation) {
                            return;
                        }
                        self.require_resync(format!("push reducer failed for {event_type}"))
                            .await;
                    }
                }
            }
            RiseWireEvent::EventBufferOverflow(..) => {
                self.require_resync("wire event buffer overflow".to_string())
                    .await;
            }
            RiseWireEvent::ProtocolViolation(reason, _) => {
                self.require_resync(format!("wire protocol violation: {reason}"))
                    .await;
            }
            RiseWireEvent::Disconnected(..) | RiseWireEvent::Suspended(..) => {
                self.state.with(|state| {
                    state.requested_replay_delay = None;
                    if let Some(replay) = &state.replay_task {
                        replay.cancel.cancel();
                    }
                });
            }
            RiseWireEvent::SessionStarted(..) | RiseWireEvent::GatewayError(..) => {}
        }
    }

    async fn enqueue_push_batch(&self, batch: RiseStoreWriteBatch, active_generation: u64) {
        if !self.is_current(active_generation) {
            return;
        }

        let mutation_count = batch.mutations.len();
        let estimated_bytes = estimated_push_batch_byte_count(&batch);
        let over_limit = if mutation_count > self.maximum_push_batch_mutations {
            Some("push batch exceeds mutation limit")
        } else if estimated_bytes > self.maximum_push_batch_bytes {
            Some("push batch exceeds estimated byte limit")
        } else {
            None
        };
        if let Some(reason) = over_limit {
            let Some(has_pending) = self.state.with(|state| {
                if !state.is_current(active_generation) {
                    return None;
                }
                state.cancel_scheduled_push_flush();
                Some(!state.pending_push_batches.is_empty())
            }) else {
                return;
            };
            if has_pending && !self.flush_pending_push_batches(active_generation).await {
                return;
            }
            if !self.is_current(active_generation) {
                return;
            }
            self.require_resync(reason.to_string()).await;
            return;
        }

        let Some(must_flush_first) = self.state.with(|state| {
            if !state.is_current(active_generation) {
                return None;
            }
            let exceeds = !state.pending_push_batches.is_empty()
                && (would_exceed_push_limit(
                    state.pending_push_batches.len(),
                    1,
                    self.maximum_push_batch_events,
                ) || would_exceed_push_limit(
                    state.pending_push_mutation_count,
                    mutation_count,
                    self.maximum_push_batch_mutations,
                ) || would_exceed_push_limit(
                    state.pending_push_estimated_bytes,
                    estimated_bytes,
                    self.maximum_push_batch_bytes,
                ));
            if exceeds {
                state.cancel_scheduled_push_flush();
            }
            Some(exceeds)
        }) else {
            return;
        };
        if must_flush_first {
            if !self.flush_pending_push_batches(active_generation).await {
                return;
            }
            if !self.is_current(active_generation) {
                return;
            }
        }

        let Some(flush_now) = self.state.with(|state| {
            if !state.is_current(active_generation) {
                return None;
            }
            state.pending_push_batches.push(batch);
            state.pending_push_mutation_count += mutation_count;
            state.pending_push_estimated_bytes += estimated_bytes;
            let full = state.pending_push_batches.len() >= self.maximum_push_batch_events
                || state.pending_push_mutation_count >= self.maximum_push_batch_mutations
                || state.pending_push_estimated_bytes >= self.maximum_push_batch_bytes
                || self.push_batch_delay.is_zero();
            if full {
                state.cancel_scheduled_push_flush();
            }
            Some(full)
        }) else {
            return;
        };
        if flush_now {
            self.flush_pending_push_batches(active_generation).await;
            return;
        }

        self.schedule_push_flush(active_generation);
    }

    fn schedule_push_flush(&self, active_generation: u64) {
        let runtime = RiseAccountRuntime::current();
        let weak = self.this.clone();
        let delay = self.push_batch_delay;
        let sleep = self.push_batch_sleep.clone();
        self.state.with(|state| {
            if state.push_flush_task.is_some()
                || !state.is_current(active_generation)
                || state.pending_push_batches.is_empty()
            {
                return;
            }
            let identifier = state.next_identifier();
            let cancel = CancellationToken::new();
            let task_cancel = cancel.clone();
            state.push_flush_task_id = Some(identifier);
            let handle = runtime.spawn(async move {
                let slept = tokio::select! {
                    biased;
                    _ = task_cancel.cancelled() => false,
                    slept = sleep(delay) => slept.is_ok(),
                };
                if !slept || task_cancel.is_cancelled() {
                    return;
                }
                let Some(this) = weak.upgrade() else { return };
                this.scheduled_push_flush(identifier, active_generation)
                    .await;
            });
            state.push_flush_task = Some(RiseSyncSpawnedTask { handle, cancel });
        });
    }

    async fn scheduled_push_flush(&self, identifier: u64, active_generation: u64) {
        let owns_flush = self.state.with(|state| {
            if state.push_flush_task_id != Some(identifier) {
                return false;
            }
            state.push_flush_task = None;
            state.push_flush_task_id = None;
            true
        });
        if owns_flush {
            self.flush_pending_push_batches(active_generation).await;
        }
    }

    async fn flush_pending_push_batches(&self, active_generation: u64) -> bool {
        if !self.is_current(active_generation) {
            return false;
        }
        self.wait_for_push_commits_to_finish().await;
        if !self.is_current(active_generation) {
            return false;
        }
        if self
            .state
            .with(|state| state.pending_push_batches.is_empty())
        {
            return true;
        }
        self.commit_pending_push_batches().await
    }

    async fn flush_pending_push_batches_for_stop(&self) {
        self.wait_for_push_commits_to_finish().await;
        if self
            .state
            .with(|state| state.pending_push_batches.is_empty())
        {
            return;
        }
        self.commit_pending_push_batches().await;
    }

    async fn commit_pending_push_batches(&self) -> bool {
        let batches = self.state.with(|state| {
            let batches = std::mem::take(&mut state.pending_push_batches);
            state.reset_pending_push_batches();
            state.push_commits_in_flight += 1;
            batches
        });
        let mut mutations: Vec<RiseStoreMutation> =
            Vec::with_capacity(batches.iter().map(|batch| batch.mutations.len()).sum());
        let mut invalidated_views: HashSet<RiseStoreViewKey> = HashSet::new();
        for batch in batches {
            mutations.extend(batch.mutations);
            invalidated_views.extend(batch.invalidated_views);
        }
        let written = self
            .store
            .write(RiseStoreWriteBatch::new(mutations, invalidated_views))
            .await;
        self.finish_push_commit();
        if written.is_err() {
            self.require_resync("push batch commit failed".to_string())
                .await;
            return false;
        }
        true
    }

    async fn wait_for_push_commits_to_finish(&self) {
        loop {
            let finished = self.state.with(|state| {
                if state.push_commits_in_flight == 0 {
                    return None;
                }
                let (waiter, finished) = oneshot::channel();
                state.push_commit_waiters.push(waiter);
                Some(finished)
            });
            let Some(finished) = finished else { return };
            let _ = finished.await;
        }
    }

    fn finish_push_commit(&self) {
        self.state.with(|state| {
            assert!(state.push_commits_in_flight > 0);
            state.push_commits_in_flight -= 1;
            if state.push_commits_in_flight != 0 {
                return;
            }
            for waiter in state.push_commit_waiters.drain(..) {
                let _ = waiter.send(());
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn pending_push_event_count_for_testing(&self) -> usize {
        self.state.with(|state| state.pending_push_batches.len())
    }

    #[cfg(test)]
    pub(crate) fn pending_durable_claim_recovery_count_for_testing(&self) -> usize {
        self.state
            .with(|state| state.pending_durable_claim_recoveries.len())
    }

    fn schedule_replay(&self, delay: Duration, active_generation: u64) {
        let runtime = RiseAccountRuntime::current();
        let weak = self.this.clone();
        self.state.with(|state| {
            if !state.is_current(active_generation) {
                return;
            }
            if state.replay_task_id.is_some() {
                state.request_replay_after_current(delay);
                if state.replay_task_is_sleeping
                    && delay.is_zero()
                    && let Some(replay) = &state.replay_task
                {
                    replay.cancel.cancel();
                }
                return;
            }

            let identifier = state.next_identifier();
            state.replay_task_id = Some(identifier);
            state.replay_task_is_sleeping = !delay.is_zero();
            let cancel = CancellationToken::new();
            let task_cancel = cancel.clone();
            let handle = runtime.spawn(async move {
                if !delay.is_zero() {
                    let slept = tokio::select! {
                        biased;
                        _ = task_cancel.cancelled() => false,
                        () = tokio::time::sleep(delay) => true,
                    };
                    if !slept {
                        if let Some(this) = weak.upgrade() {
                            this.replay_task_finished(identifier, active_generation)
                                .await;
                        }
                        return;
                    }
                }
                let Some(this) = weak.upgrade() else { return };
                if !this.mark_replay_task_awake(identifier) || task_cancel.is_cancelled() {
                    this.replay_task_finished(identifier, active_generation)
                        .await;
                    return;
                }
                this.drain_durable_operations(active_generation, &task_cancel)
                    .await;
                this.replay_task_finished(identifier, active_generation)
                    .await;
            });
            state.replay_task = Some(RiseSyncSpawnedTask { handle, cancel });
        });
    }

    async fn drain_durable_operations(&self, active_generation: u64, cancel: &CancellationToken) {
        if !self.is_current(active_generation) || !self.wire.is_ready_for_calls(false).await {
            tracing::warn!(
                target: "riseonly::X1586",
                "durable drain skipped current={} {}",
                self.is_current(active_generation),
                self.wire.readiness_probe()
            );
            return;
        }
        if !self.recover_pending_durable_claims(active_generation).await {
            tracing::warn!(target: "riseonly::X1586", "durable drain held by claim recovery");
            return;
        }

        let runtime = RiseAccountRuntime::current();
        let group_cancel = cancel.child_token();
        let mut group: FuturesUnordered<JoinHandle<RiseSyncDurableWorkerCompletion>> =
            FuturesUnordered::new();
        let mut active_workers = 0usize;
        let mut is_stopping = false;

        loop {
            if cancel.is_cancelled() || !self.is_current(active_generation) {
                is_stopping = true;
                group_cancel.cancel();
            }

            let may_claim = !is_stopping
                && active_workers < self.maximum_concurrent_durable_operations
                && self
                    .state
                    .with(|state| state.pending_durable_claim_recoveries.is_empty());
            if may_claim {
                if self.wire.is_ready_for_calls(false).await {
                    let limit = self.maximum_concurrent_durable_operations - active_workers;
                    let claimed = self
                        .store
                        .claim_replayable_operations(
                            self.now_milliseconds(),
                            i64::try_from(limit).unwrap_or(i64::MAX),
                        )
                        .await;
                    match claimed {
                        Ok(claims) => {
                            let no_claims = claims.is_empty();
                            tracing::warn!(target: "riseonly::X1586", claimed = claims.len(), active_workers, "durable claim");
                            for claim in claims {
                                let handler = self.state.with(|state| {
                                    let handler =
                                        state.durable_handlers.get(&claim.operation.kind).cloned();
                                    state.publish_durable_lifecycle(|| {
                                        RiseDurableLifecycleEvent::AttemptStarted {
                                            request_id: claim.operation.request_id,
                                            kind: claim.operation.kind.clone(),
                                            attempt: claim.operation.attempt_count + 1,
                                        }
                                    });
                                    handler
                                });
                                let wire = self.wire.clone();
                                let worker_cancel = group_cancel.clone();
                                group.push(runtime.spawn(async move {
                                    run_durable_worker(claim, handler, wire, worker_cancel).await
                                }));
                                active_workers += 1;
                            }

                            if no_claims && active_workers == 0 {
                                let now = self.now_milliseconds();
                                match self.store.next_replay_at_milliseconds().await {
                                    Ok(Some(next)) if next > now => {
                                        self.state.with(|state| {
                                            state.request_replay_after_current(milliseconds(
                                                next - now,
                                            ));
                                        });
                                    }
                                    Ok(_) => {}
                                    Err(_) => {
                                        self.state.with(|state| {
                                            state.request_replay_after_current(
                                                DRAIN_FAILURE_RETRY_DELAY,
                                            );
                                        });
                                    }
                                }
                                return;
                            }
                        }
                        Err(_) => {
                            self.state.with(|state| {
                                state.request_replay_after_current(DRAIN_FAILURE_RETRY_DELAY);
                            });
                            if active_workers == 0 {
                                return;
                            }
                            is_stopping = true;
                            group_cancel.cancel();
                        }
                    }
                } else {
                    is_stopping = true;
                    group_cancel.cancel();
                }
            }

            if active_workers == 0 {
                return;
            }
            let Some(joined) = group.next().await else { return };
            active_workers -= 1;
            let Ok(completion) = joined else { continue };
            let claim = completion.claim.clone();
            let operation = &claim.operation;
            let result = lifecycle_result(&completion.outcome);
            tracing::warn!(target: "riseonly::X1586", kind = %operation.kind, ?result, "durable attempt finished");
            self.state.with(|state| {
                state.publish_durable_lifecycle(|| RiseDurableLifecycleEvent::AttemptFinished {
                    request_id: operation.request_id,
                    kind: operation.kind.clone(),
                    attempt: operation.attempt_count + 1,
                    result,
                });
            });
            if self.commit(completion).await.is_ok() {
                continue;
            }
            self.state.with(|state| {
                state.publish_durable_lifecycle(|| {
                    RiseDurableLifecycleEvent::ReconciliationFailed {
                        request_id: operation.request_id,
                        kind: operation.kind.clone(),
                        attempt: operation.attempt_count + 1,
                        result: RiseDurableLifecycleResult::StoreCommitFailed,
                    }
                });
            });
            let recovery = self.make_durable_claim_recovery(&claim, active_generation);
            let replay_delay = match self.write_durable_claim_recovery(&recovery).await {
                Ok(commit) => {
                    if self.is_current(active_generation) {
                        self.publish_durable_claim_recovery(&recovery, &commit);
                    }
                    self.durable_replay_delay(&recovery)
                }
                Err(_) => {
                    let mut pending = recovery;
                    pending.failure_count = 1;
                    let delay_milliseconds =
                        durable_claim_recovery_retry_delay_milliseconds(pending.failure_count);
                    pending.next_recovery_at_milliseconds =
                        self.now_milliseconds() + delay_milliseconds;
                    self.remember_pending_durable_claim_recovery(pending);
                    milliseconds(delay_milliseconds)
                }
            };
            self.require_resync(format!(
                "durable completion failed for {}",
                operation.kind
            ))
            .await;
            self.state.with(|state| {
                if state.is_current(active_generation) {
                    state.request_replay_after_current(replay_delay);
                }
            });
        }
    }

    fn make_durable_claim_recovery(
        &self,
        claim: &RiseDurableClaim,
        active_generation: u64,
    ) -> RiseSyncPendingDurableClaimRecovery {
        let operation = &claim.operation;
        let attempt = operation.attempt_count + 1;
        let delay_milliseconds = retry_delay_milliseconds(operation.request_id, attempt);
        let now = self.now_milliseconds();
        RiseSyncPendingDurableClaimRecovery {
            request_id: operation.request_id,
            kind: operation.kind.clone(),
            claim_token: claim.token.clone(),
            attempt,
            next_attempt_at_milliseconds: now + delay_milliseconds,
            generation: active_generation,
            failure_count: 0,
            next_recovery_at_milliseconds: 0,
        }
    }

    async fn write_durable_claim_recovery(
        &self,
        recovery: &RiseSyncPendingDurableClaimRecovery,
    ) -> anyhow::Result<RiseStoreCommit> {
        (self.durable_claim_recovery_write)(
            self.store.clone(),
            write_batch(vec![RiseStoreMutation::TransitionClaimedDurableOperation {
                request_id: recovery.request_id,
                claim_token: recovery.claim_token.clone(),
                state: RiseDurableOperationState::RetryWaiting,
                attempt_count: recovery.attempt,
                next_attempt_at_milliseconds: recovery.next_attempt_at_milliseconds,
                last_error_code: Some(RiseSyncFailureCode::StoreCommitFailed.wire().to_string()),
                updated_at_milliseconds: self.now_milliseconds(),
            }]),
        )
        .await
    }

    fn publish_durable_claim_recovery(
        &self,
        recovery: &RiseSyncPendingDurableClaimRecovery,
        commit: &RiseStoreCommit,
    ) {
        self.state.with(|state| {
            state.publish_durable_lifecycle(|| RiseDurableLifecycleEvent::RetryScheduled {
                request_id: recovery.request_id,
                kind: recovery.kind.clone(),
                attempt: recovery.attempt,
                result: RiseDurableLifecycleResult::StoreCommitFailed,
                store_revision: commit.revision,
            });
        });
    }

    fn remember_pending_durable_claim_recovery(
        &self,
        recovery: RiseSyncPendingDurableClaimRecovery,
    ) {
        self.state.with(|state| {
            let request_id = recovery.request_id;
            if let Some(existing) = state.pending_durable_claim_recoveries.get(&request_id) {
                assert!(existing.claim_token == recovery.claim_token);
            } else {
                assert!(
                    state.pending_durable_claim_recoveries.len()
                        < self.maximum_concurrent_durable_operations
                );
            }
            state
                .pending_durable_claim_recoveries
                .insert(request_id, recovery);
        });
    }

    async fn recover_pending_durable_claims(&self, active_generation: u64) -> bool {
        let request_ids = self.state.with(|state| {
            let mut request_ids: Vec<RiseRequestID> = state
                .pending_durable_claim_recoveries
                .keys()
                .copied()
                .collect();
            request_ids.sort();
            request_ids
        });
        if request_ids.is_empty() {
            return true;
        }
        let mut next_delay: Option<Duration> = None;

        for request_id in request_ids {
            let step = self.state.with(|state| {
                let Some(recovery) = state
                    .pending_durable_claim_recoveries
                    .get(&request_id)
                    .cloned()
                else {
                    return RiseSyncRecoveryStep::Skip;
                };
                if recovery.generation != active_generation {
                    state.pending_durable_claim_recoveries.remove(&request_id);
                    return RiseSyncRecoveryStep::Skip;
                }
                if !state.is_current(active_generation) {
                    return RiseSyncRecoveryStep::Abort;
                }
                let now = self.now_milliseconds();
                if recovery.next_recovery_at_milliseconds > now {
                    return RiseSyncRecoveryStep::Wait(milliseconds(
                        recovery.next_recovery_at_milliseconds - now,
                    ));
                }
                RiseSyncRecoveryStep::Write(recovery)
            });
            let mut recovery = match step {
                RiseSyncRecoveryStep::Skip => continue,
                RiseSyncRecoveryStep::Abort => return false,
                RiseSyncRecoveryStep::Wait(delay) => {
                    next_delay = Some(next_delay.map_or(delay, |current| current.min(delay)));
                    continue;
                }
                RiseSyncRecoveryStep::Write(recovery) => recovery,
            };
            match self.write_durable_claim_recovery(&recovery).await {
                Ok(commit) => {
                    self.state.with(|state| {
                        let same_claim = state
                            .pending_durable_claim_recoveries
                            .get(&request_id)
                            .is_some_and(|pending| pending.claim_token == recovery.claim_token);
                        if same_claim {
                            state.pending_durable_claim_recoveries.remove(&request_id);
                        }
                    });
                    if self.is_current(active_generation) {
                        self.publish_durable_claim_recovery(&recovery, &commit);
                    }
                }
                Err(_) => {
                    recovery.failure_count += 1;
                    let delay_milliseconds =
                        durable_claim_recovery_retry_delay_milliseconds(recovery.failure_count);
                    recovery.next_recovery_at_milliseconds =
                        self.now_milliseconds() + delay_milliseconds;
                    let still_pending = self.state.with(|state| {
                        let same_claim = state
                            .pending_durable_claim_recoveries
                            .get(&request_id)
                            .is_some_and(|pending| pending.claim_token == recovery.claim_token);
                        if same_claim {
                            state
                                .pending_durable_claim_recoveries
                                .insert(request_id, recovery);
                        }
                        same_claim
                    });
                    if !still_pending {
                        continue;
                    }
                    let delay = milliseconds(delay_milliseconds);
                    next_delay = Some(next_delay.map_or(delay, |current| current.min(delay)));
                }
            }
        }

        self.state.with(|state| {
            if !state.pending_durable_claim_recoveries.is_empty() {
                if state.is_current(active_generation) {
                    state.request_replay_after_current(
                        next_delay.unwrap_or(DRAIN_FAILURE_RETRY_DELAY),
                    );
                }
                return false;
            }
            state.is_current(active_generation)
        })
    }

    fn durable_replay_delay(&self, recovery: &RiseSyncPendingDurableClaimRecovery) -> Duration {
        milliseconds(recovery.next_attempt_at_milliseconds - self.now_milliseconds())
    }

    async fn commit(&self, completion: RiseSyncDurableWorkerCompletion) -> anyhow::Result<()> {
        let RiseSyncDurableWorkerCompletion { claim, outcome } = completion;
        let operation = &claim.operation;
        let now = self.now_milliseconds();
        let attempt = operation.attempt_count + 1;
        let transition = |state: RiseDurableOperationState,
                          attempt_count: i64,
                          next_attempt_at_milliseconds: i64,
                          last_error_code: Option<String>| {
            RiseStoreMutation::TransitionClaimedDurableOperation {
                request_id: operation.request_id,
                claim_token: claim.token.clone(),
                state,
                attempt_count,
                next_attempt_at_milliseconds,
                last_error_code,
                updated_at_milliseconds: now,
            }
        };
        let (event, terminal_code, resync_reason) = match outcome {
            RiseSyncDurableWorkerOutcome::Acknowledged(reconciliation) => {
                let commit = self
                    .write(
                        reconciliation,
                        RiseStoreMutation::RemoveClaimedDurableOperation {
                            request_id: operation.request_id,
                            claim_token: claim.token.clone(),
                        },
                    )
                    .await?;
                (
                    RiseDurableLifecycleEvent::ReconciliationCommitted {
                        request_id: operation.request_id,
                        kind: operation.kind.clone(),
                        attempt,
                        result: RiseDurableLifecycleResult::Acknowledged,
                        store_revision: commit.revision,
                    },
                    None,
                    None,
                )
            }
            RiseSyncDurableWorkerOutcome::Paused => {
                let commit = self
                    .store
                    .write(write_batch(vec![transition(
                        RiseDurableOperationState::Queued,
                        operation.attempt_count,
                        now,
                        None,
                    )]))
                    .await?;
                (
                    retry_scheduled(operation, attempt, RiseDurableLifecycleResult::Paused, &commit),
                    None,
                    None,
                )
            }
            RiseSyncDurableWorkerOutcome::Deferred {
                code,
                delay_milliseconds,
            } => {
                let commit = self
                    .store
                    .write(write_batch(vec![transition(
                        RiseDurableOperationState::RetryWaiting,
                        operation.attempt_count,
                        now + MINIMUM_DEFER_MILLISECONDS.max(delay_milliseconds),
                        Some(code),
                    )]))
                    .await?;
                (
                    retry_scheduled(operation, attempt, RiseDurableLifecycleResult::Deferred, &commit),
                    None,
                    None,
                )
            }
            RiseSyncDurableWorkerOutcome::Retry { code } => {
                let attempts = operation.attempt_count + 1;
                let retry_at = now + retry_delay_milliseconds(operation.request_id, attempts);
                let commit = self
                    .store
                    .write(write_batch(vec![transition(
                        RiseDurableOperationState::RetryWaiting,
                        attempts,
                        retry_at,
                        Some(code),
                    )]))
                    .await?;
                (
                    retry_scheduled(
                        operation,
                        attempt,
                        RiseDurableLifecycleResult::RetryableFailure,
                        &commit,
                    ),
                    None,
                    None,
                )
            }
            RiseSyncDurableWorkerOutcome::ReconciliationFailed { code } => {
                let attempts = operation.attempt_count + 1;
                let retry_at = now + retry_delay_milliseconds(operation.request_id, attempts);
                let commit = self
                    .store
                    .write(write_batch(vec![transition(
                        RiseDurableOperationState::RetryWaiting,
                        attempts,
                        retry_at,
                        Some(code),
                    )]))
                    .await?;
                (
                    retry_scheduled(
                        operation,
                        attempt,
                        RiseDurableLifecycleResult::ReconciliationFailed,
                        &commit,
                    ),
                    None,
                    Some(format!("durable reconciliation failed for {}", operation.kind)),
                )
            }
            RiseSyncDurableWorkerOutcome::Terminal {
                code,
                reconciliation,
            } => {
                let commit = self
                    .write(
                        reconciliation,
                        transition(
                            RiseDurableOperationState::TerminalFailure,
                            operation.attempt_count + 1,
                            now,
                            Some(code.clone()),
                        ),
                    )
                    .await?;
                (
                    RiseDurableLifecycleEvent::Terminal {
                        request_id: operation.request_id,
                        kind: operation.kind.clone(),
                        attempt,
                        result: RiseDurableLifecycleResult::TerminalFailure,
                        store_revision: commit.revision,
                    },
                    Some(code),
                    None,
                )
            }
        };
        self.state.with(|state| {
            state.publish_durable_lifecycle(|| event);
            if let Some(code) = terminal_code {
                state.publish_health(RiseSyncHealthEvent::DurableOperationFailed {
                    request_id: operation.request_id,
                    code,
                });
            }
        });
        if let Some(reason) = resync_reason {
            self.require_resync(reason).await;
        }
        Ok(())
    }

    async fn write(
        &self,
        reconciliation: RiseStoreWriteBatch,
        final_mutation: RiseStoreMutation,
    ) -> anyhow::Result<RiseStoreCommit> {
        let mut mutations = reconciliation.mutations;
        mutations.push(final_mutation);
        Ok(self
            .store
            .write(RiseStoreWriteBatch::new(
                mutations,
                reconciliation.invalidated_views,
            ))
            .await?)
    }

    fn mark_replay_task_awake(&self, identifier: u64) -> bool {
        self.state.with(|state| {
            if state.replay_task_id != Some(identifier) {
                return false;
            }
            state.replay_task_is_sleeping = false;
            true
        })
    }

    async fn replay_task_finished(&self, identifier: u64, active_generation: u64) {
        let delay = self.state.with(|state| {
            if state.replay_task_id != Some(identifier) {
                return None;
            }
            state.replay_task = None;
            state.replay_task_id = None;
            state.replay_task_is_sleeping = false;
            state
                .requested_replay_delay
                .take()
                .filter(|_| state.is_current(active_generation))
        });
        let Some(delay) = delay else { return };
        if !self.wire.is_ready_for_calls(false).await {
            return;
        }
        self.schedule_replay(delay, active_generation);
    }

    async fn require_resync(&self, reason: String) {
        let handler = self.state.with(|state| {
            state.publish_health(RiseSyncHealthEvent::ResyncRequired {
                reason: reason.clone(),
            });
            state.resync_handler.clone()
        });
        if let Some(handler) = handler {
            handler(reason).await;
        }
    }

    fn is_current(&self, active_generation: u64) -> bool {
        self.state.with(|state| state.is_current(active_generation))
    }

    fn now_milliseconds(&self) -> i64 {
        self.clock.now_milliseconds()
    }
}

async fn run_durable_worker(
    claim: RiseDurableClaim,
    handler: Option<RiseSyncDurableHandler>,
    wire: Arc<RiseWire>,
    cancel: CancellationToken,
) -> RiseSyncDurableWorkerCompletion {
    let Some(handler) = handler else {
        return RiseSyncDurableWorkerCompletion {
            claim,
            outcome: RiseSyncDurableWorkerOutcome::Deferred {
                code: RiseSyncFailureCode::MissingHandler.wire().to_string(),
                delay_milliseconds: DEFER_DELAY_MILLISECONDS,
            },
        };
    };
    let context =
        RiseSyncDurableContext::new(claim.operation.request_id, claim.operation.attempt_count);
    let outcome = (handler.execute)(claim.operation.payload.clone(), context, wire, cancel).await;
    RiseSyncDurableWorkerCompletion { claim, outcome }
}

fn retry_scheduled(
    operation: &RiseDurableOperation,
    attempt: i64,
    result: RiseDurableLifecycleResult,
    commit: &RiseStoreCommit,
) -> RiseDurableLifecycleEvent {
    RiseDurableLifecycleEvent::RetryScheduled {
        request_id: operation.request_id,
        kind: operation.kind.clone(),
        attempt,
        result,
        store_revision: commit.revision,
    }
}

fn lifecycle_result(outcome: &RiseSyncDurableWorkerOutcome) -> RiseDurableLifecycleResult {
    match outcome {
        RiseSyncDurableWorkerOutcome::Acknowledged(_) => RiseDurableLifecycleResult::Acknowledged,
        RiseSyncDurableWorkerOutcome::Paused => RiseDurableLifecycleResult::Paused,
        RiseSyncDurableWorkerOutcome::Deferred { .. } => RiseDurableLifecycleResult::Deferred,
        RiseSyncDurableWorkerOutcome::Retry { .. } => RiseDurableLifecycleResult::RetryableFailure,
        RiseSyncDurableWorkerOutcome::ReconciliationFailed { .. } => {
            RiseDurableLifecycleResult::ReconciliationFailed
        }
        RiseSyncDurableWorkerOutcome::Terminal { .. } => {
            RiseDurableLifecycleResult::TerminalFailure
        }
    }
}

fn retry_delay_milliseconds(request_id: RiseRequestID, attempt_count: i64) -> i64 {
    let attempt = u64::try_from(attempt_count).unwrap_or(0);
    i64::try_from(retry_delay_ms(request_id.raw_value(), attempt)).unwrap_or(i64::MAX)
}

fn durable_claim_recovery_retry_delay_milliseconds(failure_count: i64) -> i64 {
    let failures = u64::try_from(failure_count).unwrap_or(0);
    i64::try_from(claim_recovery_delay_ms(failures)).unwrap_or(i64::MAX)
}

fn would_exceed_push_limit(current: usize, incoming: usize, limit: usize) -> bool {
    incoming > limit || current > limit - incoming
}

fn estimated_push_batch_byte_count(batch: &RiseStoreWriteBatch) -> usize {
    let mut result: usize = 32;
    for mutation in &batch.mutations {
        result = result.saturating_add(estimated_push_payload_byte_count(mutation));
        result = result.saturating_add(64);
    }
    for view in &batch.invalidated_views {
        result = result.saturating_add(view.raw_value().len());
        result = result.saturating_add(16);
    }
    result
}

fn estimated_push_payload_byte_count(mutation: &RiseStoreMutation) -> usize {
    let fields: usize = match mutation {
        RiseStoreMutation::SetValue {
            namespace,
            key,
            value,
            precondition,
            policy: _,
        } => sum(&[
            namespace_bytes(namespace),
            key.len(),
            composite(&[value.data.len(), SCALAR_BYTES, SCALAR_BYTES]),
            precondition_bytes(precondition),
            SCALAR_BYTES,
        ]),
        RiseStoreMutation::RemoveValue {
            namespace,
            key,
            precondition,
        } => sum(&[
            namespace_bytes(namespace),
            key.len(),
            precondition_bytes(precondition),
        ]),
        RiseStoreMutation::UpsertOrderedItem {
            namespace,
            collection,
            item,
            policy: _,
        } => sum(&[
            namespace_bytes(namespace),
            collection.len(),
            composite(&[
                item.item_id.len(),
                optional_bytes(item.server_id.as_ref().map(String::len)),
                SCALAR_BYTES,
                item.data.len(),
                SCALAR_BYTES,
                SCALAR_BYTES,
            ]),
            SCALAR_BYTES,
        ]),
        RiseStoreMutation::RemoveOrderedItem {
            namespace,
            collection,
            item_id,
        } => sum(&[namespace_bytes(namespace), collection.len(), item_id.len()]),
        RiseStoreMutation::SetCollectionState {
            namespace,
            collection,
            state,
        } => sum(&[
            namespace_bytes(namespace),
            collection.len(),
            composite(&[
                SCALAR_BYTES,
                optional_bytes(state.before_cursor.as_ref().map(Vec::len)),
                optional_bytes(state.after_cursor.as_ref().map(Vec::len)),
                SCALAR_BYTES,
                SCALAR_BYTES,
            ]),
        ]),
        RiseStoreMutation::UpsertEntity {
            namespace,
            entity,
            policy,
        } => sum(&[
            namespace_bytes(namespace),
            entity_bytes(entity),
            match policy {
                RiseEntityWritePolicy::RequireIdenticalReplay
                | RiseEntityWritePolicy::ServerAuthoritative
                | RiseEntityWritePolicy::IgnoreIfNotNewer => SCALAR_BYTES,
                RiseEntityWritePolicy::CompareAndSwap { expected } => {
                    composite(&[entity_bytes(expected)])
                }
            },
        ]),
        RiseStoreMutation::RemoveEntity {
            namespace,
            local_id,
        } => sum(&[namespace_bytes(namespace), local_id.len()]),
        RiseStoreMutation::SetEntityAlias {
            namespace,
            alias,
            local_id,
        } => sum(&[namespace_bytes(namespace), alias_bytes(alias), local_id.len()]),
        RiseStoreMutation::RemoveEntityAlias { namespace, alias } => {
            sum(&[namespace_bytes(namespace), alias_bytes(alias)])
        }
        RiseStoreMutation::UpsertEntityViewMembership {
            namespace,
            view,
            membership,
        } => sum(&[
            namespace_bytes(namespace),
            view.len(),
            composite(&[
                membership.local_id.len(),
                composite(&[membership.position_key.raw_value().len()]),
                SCALAR_BYTES,
            ]),
        ]),
        RiseStoreMutation::RemoveEntityViewMembership {
            namespace,
            view,
            local_id,
            if_version_at_most,
        } => sum(&[
            namespace_bytes(namespace),
            view.len(),
            local_id.len(),
            optional_bytes(if_version_at_most.map(|_| SCALAR_BYTES)),
        ]),
        RiseStoreMutation::RemoveAllEntityViewMemberships {
            namespace,
            view,
            if_version_at_most,
        } => sum(&[
            namespace_bytes(namespace),
            view.len(),
            optional_bytes(if_version_at_most.map(|_| SCALAR_BYTES)),
        ]),
        RiseStoreMutation::TrimEntityView {
            namespace,
            view,
            keeping,
            if_version_at_most,
        } => sum(&[
            namespace_bytes(namespace),
            view.len(),
            match keeping {
                RiseEntityViewRetention::First(_) | RiseEntityViewRetention::Latest(_) => {
                    composite(&[SCALAR_BYTES])
                }
            },
            optional_bytes(if_version_at_most.map(|_| SCALAR_BYTES)),
        ]),
        RiseStoreMutation::PruneUnreferencedEntities { namespace, limit: _ } => {
            sum(&[namespace_bytes(namespace), SCALAR_BYTES])
        }
        RiseStoreMutation::MergeEntities {
            namespace,
            survivor_local_id,
            duplicate_local_id,
        } => sum(&[
            namespace_bytes(namespace),
            survivor_local_id.len(),
            duplicate_local_id.len(),
        ]),
        RiseStoreMutation::SetEntityViewState {
            namespace,
            view,
            state,
        } => sum(&[
            namespace_bytes(namespace),
            view.len(),
            composite(&[
                SCALAR_BYTES,
                optional_bytes(state.before_cursor.as_ref().map(Vec::len)),
                optional_bytes(state.after_cursor.as_ref().map(Vec::len)),
                SCALAR_BYTES,
                SCALAR_BYTES,
                SCALAR_BYTES,
                optional_bytes(state.last_error_code.as_ref().map(String::len)),
                SCALAR_BYTES,
            ]),
        ]),
        RiseStoreMutation::PutDurableOperation(operation) => {
            return composite(&[durable_operation_bytes(operation)]);
        }
        RiseStoreMutation::TransitionClaimedDurableOperation {
            request_id: _,
            claim_token,
            state: _,
            attempt_count: _,
            next_attempt_at_milliseconds: _,
            last_error_code,
            updated_at_milliseconds: _,
        } => sum(&[
            REQUEST_ID_BYTES,
            claim_token.len(),
            SCALAR_BYTES,
            SCALAR_BYTES,
            SCALAR_BYTES,
            optional_bytes(last_error_code.as_ref().map(String::len)),
            SCALAR_BYTES,
        ]),
        RiseStoreMutation::RemoveClaimedDurableOperation {
            request_id: _,
            claim_token,
        } => sum(&[REQUEST_ID_BYTES, claim_token.len()]),
        RiseStoreMutation::RemoveUnclaimedDurableOperation { request_id: _ } => {
            return composite(&[REQUEST_ID_BYTES]);
        }
        RiseStoreMutation::ResolveTerminalOperation {
            request_id: _,
            resolution,
            updated_at_milliseconds: _,
        } => sum(&[
            REQUEST_ID_BYTES,
            match resolution {
                RiseDurableTerminalResolution::Remove => SCALAR_BYTES,
                RiseDurableTerminalResolution::Requeue { .. } => {
                    composite(&[composite(&[SCALAR_BYTES, SCALAR_BYTES])])
                }
            },
            SCALAR_BYTES,
        ]),
    };
    composite(&[composite(&[fields])])
}

const SCALAR_BYTES: usize = 8;
const REQUEST_ID_BYTES: usize = SCALAR_BYTES + SCALAR_BYTES;

fn sum(children: &[usize]) -> usize {
    children
        .iter()
        .fold(0usize, |total, child| total.saturating_add(*child))
}

fn composite(children: &[usize]) -> usize {
    SCALAR_BYTES.saturating_add(sum(children))
}

fn optional_bytes(wrapped: Option<usize>) -> usize {
    composite(&[wrapped.unwrap_or(0)])
}

fn precondition_bytes(precondition: &RiseStoreVersionPrecondition) -> usize {
    match precondition {
        RiseStoreVersionPrecondition::Any | RiseStoreVersionPrecondition::Missing => SCALAR_BYTES,
        RiseStoreVersionPrecondition::Equals(_) => composite(&[SCALAR_BYTES]),
    }
}

fn namespace_bytes(namespace: &RiseStoreNamespace) -> usize {
    composite(&[namespace.raw_value().len()])
}

fn entity_bytes(entity: &RiseStoreEntity) -> usize {
    composite(&[
        entity.local_id.len(),
        entity.data.len(),
        SCALAR_BYTES,
        SCALAR_BYTES,
    ])
}

fn alias_bytes(alias: &RiseEntityAlias) -> usize {
    composite(&[composite(&[alias.kind.raw_value().len()]), alias.value.len()])
}

fn durable_operation_bytes(operation: &RiseDurableOperation) -> usize {
    composite(&[
        REQUEST_ID_BYTES,
        operation.kind.len(),
        operation.payload.len(),
        composite(&[operation.ordering_key.raw_value().len()]),
        SCALAR_BYTES,
        SCALAR_BYTES,
        SCALAR_BYTES,
        optional_bytes(operation.last_error_code.as_ref().map(String::len)),
        SCALAR_BYTES,
        SCALAR_BYTES,
    ])
}

fn write_batch(mutations: Vec<RiseStoreMutation>) -> RiseStoreWriteBatch {
    RiseStoreWriteBatch::new(mutations, HashSet::new())
}

fn milliseconds(value: i64) -> Duration {
    Duration::from_millis(u64::try_from(value).unwrap_or(0))
}

fn check_cancellation(cancel: &CancellationToken) -> Result<(), CancellationError> {
    if cancel.is_cancelled() {
        Err(CancellationError)
    } else {
        Ok(())
    }
}

fn rethrow(error: &anyhow::Error) -> anyhow::Error {
    if let Some(sync) = error.downcast_ref::<RiseSyncError>() {
        return sync.clone().into();
    }
    if error.is::<CancellationError>() {
        return CancellationError.into();
    }
    anyhow::anyhow!("{error:#}")
}
