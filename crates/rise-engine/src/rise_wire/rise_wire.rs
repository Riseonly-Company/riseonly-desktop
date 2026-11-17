use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::future::BoxFuture;
use rise_canon::RiseWireRequestMetadata;
use rise_canon::rise_rpc::{
    LIMITS_EVENT_BUFFER_CAPACITY, LIMITS_MAXIMUM_EVENT_SUBSCRIBERS,
    LIMITS_MAXIMUM_EXPLICIT_REQUEST_IDENTITIES, LIMITS_MAXIMUM_INBOUND_FRAME_BYTES,
    LIMITS_MAXIMUM_OUTBOUND_FRAME_BYTES, LIMITS_MAXIMUM_PENDING_REQUESTS,
};
use rise_canon::rise_wire::ERROR_CODE_REMOTE_DEFAULT;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_crypto::{Sha256, Sha256Digest};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_uuid::Uuid;
use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_core::rise_identifiers::{RiseRequestID, RiseRequestIDAllocator};
use crate::rise_engine::RiseEngine;
use crate::rise_engine_observability::{RiseEngineCallContext, RiseEngineObservability};
use crate::rise_wire::rise_json_value::RiseJSONValue;
use crate::rise_wire::rise_wire_contracts::{
    RiseWireContractError, RiseWireIncomingFrame, RiseWireMethod, RiseWireMethodDescriptor,
    RiseWirePushEnvelope, RiseWireRemoteError, RiseWireReplayPolicy, RiseWireRequestEncoder,
    RiseWireTransport,
};
use crate::rise_wire::rise_wire_frame_decoder::{DEFAULT_REMOTE_MESSAGE, RiseWireFrameDecoder};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RiseWireAccountContext {
    #[serde(rename = "accountID")]
    pub account_id: String,
    pub epoch: String,
}

impl RiseWireAccountContext {
    pub fn new(account_id: String, epoch: String) -> Self {
        assert!(!account_id.is_empty() && !epoch.is_empty());
        Self { account_id, epoch }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RiseWireSocketSession {
    pub account: RiseWireAccountContext,
    #[serde(rename = "transportEpoch")]
    pub transport_epoch: u64,
}

impl RiseWireSocketSession {
    pub fn new(account: RiseWireAccountContext, transport_epoch: u64) -> Self {
        Self { account, transport_epoch }
    }
}

pub trait RiseWireFrameSender: Send + Sync {
    fn send(
        &self,
        frame: Vec<u8>,
        session: RiseWireSocketSession,
    ) -> BoxFuture<'static, anyhow::Result<()>>;
}

/// posts one request frame and returns the gateway response frame
/// host owns address, token and refresh
/// the wire owns framing, deadlines and decoding so http answers what the socket would have
pub trait RiseWireHttpSender: Send + Sync {
    fn post(
        &self,
        frame: Vec<u8>,
        account: RiseWireAccountContext,
        timeout: Duration,
    ) -> BoxFuture<'static, anyhow::Result<Vec<u8>>>;
}

pub type RiseWireClosureSenderOperation = Arc<
    dyn Fn(Vec<u8>, RiseWireSocketSession) -> BoxFuture<'static, anyhow::Result<()>> + Send + Sync,
>;

#[derive(Clone)]
pub struct RiseWireClosureSender {
    operation: RiseWireClosureSenderOperation,
}

impl RiseWireClosureSender {
    pub fn new(operation: RiseWireClosureSenderOperation) -> Self {
        Self { operation }
    }
}

impl RiseWireFrameSender for RiseWireClosureSender {
    fn send(
        &self,
        frame: Vec<u8>,
        session: RiseWireSocketSession,
    ) -> BoxFuture<'static, anyhow::Result<()>> {
        (self.operation)(frame, session)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseWireConfiguration {
    pub maximum_pending_requests: usize,
    pub maximum_inbound_frame_bytes: usize,
    pub maximum_outbound_frame_bytes: usize,
    pub maximum_event_subscribers: usize,
    pub event_buffer_capacity: usize,
    pub maximum_explicit_request_identities: usize,
    pub frame_extras: BTreeMap<String, Vec<String>>,
}

impl RiseWireConfiguration {
    pub fn new(
        maximum_pending_requests: usize,
        maximum_inbound_frame_bytes: usize,
        maximum_outbound_frame_bytes: usize,
        maximum_event_subscribers: usize,
        event_buffer_capacity: usize,
        maximum_explicit_request_identities: usize,
        frame_extras: BTreeMap<String, Vec<String>>,
    ) -> Self {
        assert!(maximum_pending_requests > 0);
        assert!(maximum_inbound_frame_bytes > 0);
        assert!(maximum_outbound_frame_bytes > 0);
        assert!(maximum_event_subscribers > 0);
        assert!(event_buffer_capacity > 0);
        assert!(maximum_explicit_request_identities > 0);
        Self {
            maximum_pending_requests,
            maximum_inbound_frame_bytes,
            maximum_outbound_frame_bytes,
            maximum_event_subscribers,
            event_buffer_capacity,
            maximum_explicit_request_identities,
            frame_extras,
        }
    }

    pub fn production() -> Self {
        Self::default()
    }
}

impl Default for RiseWireConfiguration {
    fn default() -> Self {
        Self::new(
            LIMITS_MAXIMUM_PENDING_REQUESTS,
            usize::try_from(LIMITS_MAXIMUM_INBOUND_FRAME_BYTES).unwrap_or(usize::MAX),
            usize::try_from(LIMITS_MAXIMUM_OUTBOUND_FRAME_BYTES).unwrap_or(usize::MAX),
            LIMITS_MAXIMUM_EVENT_SUBSCRIBERS,
            LIMITS_EVENT_BUFFER_CAPACITY,
            LIMITS_MAXIMUM_EXPLICIT_REQUEST_IDENTITIES,
            BTreeMap::new(),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RiseWireEvent {
    SessionStarted(RiseWireSocketSession),
    Connected(RiseWireSocketSession),
    Disconnected(RiseWireSocketSession, String),
    Suspended(Option<RiseWireSocketSession>),
    Resumed(Option<RiseWireSocketSession>),
    Push(RiseWirePushEnvelope, RiseWireSocketSession),
    GatewayError(RiseWireRemoteError, RiseWireSocketSession),
    ProtocolViolation(String, Option<RiseWireSocketSession>),
    EventBufferOverflow(Option<RiseWireSocketSession>),
}

/// stream of wire events, ends when the wire shuts down or the subscription is cancelled
/// dropping it ends the subscription
pub struct RiseWireEventStream {
    receiver: Option<broadcast::Receiver<RiseWireEvent>>,
    termination: Option<(Weak<RiseWire>, Uuid)>,
}

impl RiseWireEventStream {
    fn finished() -> Self {
        Self { receiver: None, termination: None }
    }

    pub async fn next(&mut self) -> Option<RiseWireEvent> {
        loop {
            let result = self.receiver.as_mut()?.recv().await;
            match result {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    self.receiver = None;
                    return None;
                }
            }
        }
    }
}

impl Drop for RiseWireEventStream {
    fn drop(&mut self) {
        let Some((wire, identifier)) = self.termination.take() else { return };
        if let Some(wire) = wire.upgrade() {
            wire.remove_subscriber(identifier);
        }
    }
}

pub struct RiseWireEventSubscription {
    pub stream: RiseWireEventStream,
    cancel_operation: Option<(Weak<RiseWire>, Uuid)>,
}

impl RiseWireEventSubscription {
    pub async fn cancel(&self) {
        let Some((wire, identifier)) = &self.cancel_operation else { return };
        if let Some(wire) = wire.upgrade() {
            wire.cancel_subscriber(*identifier);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseWireInboundDisposition {
    ConsumedResponse,
    Event,
    Ignored,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseWireError {
    #[error("noSocketSession")]
    NoSocketSession,
    #[error("notConnected")]
    NotConnected,
    #[error("suspended")]
    Suspended,
    #[error("staleAccount")]
    StaleAccount,
    #[error("staleSession")]
    StaleSession,
    #[error("pendingLimitReached({0})")]
    PendingLimitReached(usize),
    #[error("duplicateRequestID({0})")]
    DuplicateRequestID(RiseRequestID),
    #[error("unsafeRequestIDReuse({0})")]
    UnsafeRequestIDReuse(RiseRequestID),
    #[error("explicitRequestIdentityLimitReached({0})")]
    ExplicitRequestIdentityLimitReached(usize),
    #[error("outboundFrameTooLarge(limit: {limit})")]
    OutboundFrameTooLarge { limit: usize },
    #[error("inboundFrameTooLarge(limit: {limit})")]
    InboundFrameTooLarge { limit: usize },
    #[error("timeout({0})")]
    Timeout(RiseRequestID),
    #[error("disconnected({0})")]
    Disconnected(String),
    #[error("missingResponseData")]
    MissingResponseData,
    #[error("responseDecodingFailed")]
    ResponseDecodingFailed,
    #[error("eventSubscriberLimitReached({0})")]
    EventSubscriberLimitReached(usize),
    #[error("shutdown")]
    Shutdown,
}

/// what a waiter gets when the wire cancels it on purpose
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, thiserror::Error)]
#[error("CancellationError")]
pub struct CancellationError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RiseWireEmptyResponse;

impl<'de> Deserialize<'de> for RiseWireEmptyResponse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(deserializer)?;
        Ok(Self)
    }
}

struct ConnectionWaiter {
    survives_suspension: bool,
    continuation: oneshot::Sender<anyhow::Result<()>>,
    timeout_task: JoinHandle<()>,
}

struct PendingRequest {
    attempt_id: Uuid,
    session: RiseWireSocketSession,
    survives_suspension: bool,
    continuation: oneshot::Sender<anyhow::Result<RiseJSONValue>>,
    timeout_task: JoinHandle<()>,
    send_task: Option<JoinHandle<()>>,
}

impl PendingRequest {
    fn resume(self, result: anyhow::Result<RiseJSONValue>) {
        self.timeout_task.abort();
        if let Some(send_task) = &self.send_task {
            send_task.abort();
        }
        let _ = self.continuation.send(result);
    }
}

struct ExplicitRequestIdentity {
    session: RiseWireSocketSession,
    fingerprint: Sha256Digest,
    replay_policy: RiseWireReplayPolicy,
}

struct EventSubscriber {
    continuation: broadcast::Sender<RiseWireEvent>,
}

struct RiseWireState {
    current_session: Option<RiseWireSocketSession>,
    is_connected: bool,
    is_application_active: bool,
    is_shutdown: bool,
    pending: HashMap<RiseRequestID, PendingRequest>,
    connection_waiters: HashMap<Uuid, ConnectionWaiter>,
    explicit_request_identities: HashMap<RiseRequestID, ExplicitRequestIdentity>,
    subscribers: HashMap<Uuid, EventSubscriber>,
    event_buffer_capacity: usize,
}

pub struct RiseWire {
    pub account: RiseWireAccountContext,
    sender: Arc<dyn RiseWireFrameSender>,
    http_sender: Option<Arc<dyn RiseWireHttpSender>>,
    allocator: &'static RiseRequestIDAllocator,
    configuration: RiseWireConfiguration,
    frame_decoder: RiseWireFrameDecoder,
    state: RiseIsolated<RiseWireState>,
    this: Weak<RiseWire>,
}

impl RiseWire {
    pub fn new(
        account: RiseWireAccountContext,
        sender: Arc<dyn RiseWireFrameSender>,
        http_sender: Option<Arc<dyn RiseWireHttpSender>>,
        allocator: &'static RiseRequestIDAllocator,
        configuration: RiseWireConfiguration,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            account,
            sender,
            http_sender,
            allocator,
            frame_decoder: RiseWireFrameDecoder::new(configuration.frame_extras.clone()),
            state: RiseIsolated::new(RiseWireState {
                current_session: None,
                is_connected: false,
                is_application_active: true,
                is_shutdown: false,
                pending: HashMap::new(),
                connection_waiters: HashMap::new(),
                explicit_request_identities: HashMap::new(),
                subscribers: HashMap::new(),
                event_buffer_capacity: configuration.event_buffer_capacity,
            }),
            configuration,
            this: this.clone(),
        })
    }

    pub async fn events(&self) -> Result<RiseWireEventStream, RiseWireError> {
        Ok(self.event_subscription().await?.stream)
    }

    pub async fn event_subscription(&self) -> Result<RiseWireEventSubscription, RiseWireError> {
        let maximum = self.configuration.maximum_event_subscribers;
        self.state.with(|state| {
            if state.is_shutdown {
                return Ok(RiseWireEventSubscription {
                    stream: RiseWireEventStream::finished(),
                    cancel_operation: None,
                });
            }
            if state.subscribers.len() >= maximum {
                return Err(RiseWireError::EventSubscriberLimitReached(maximum));
            }
            let identifier = Uuid::new_v4();
            let (continuation, receiver) = broadcast::channel(state.event_buffer_capacity);
            state.subscribers.insert(identifier, EventSubscriber { continuation });
            Ok(RiseWireEventSubscription {
                stream: RiseWireEventStream {
                    receiver: Some(receiver),
                    termination: Some((self.this.clone(), identifier)),
                },
                cancel_operation: Some((self.this.clone(), identifier)),
            })
        })
    }

    pub async fn call<Method: RiseWireMethod>(
        &self,
        request: &Method::Request,
        request_id: Option<RiseRequestID>,
        metadata: RiseWireRequestMetadata,
    ) -> anyhow::Result<Method::Response> {
        let descriptor = Method::descriptor();
        let payload = self
            .exchange(&descriptor, request_id, |id, timestamp| {
                RiseWireRequestEncoder::encode_method::<Method>(request, id, timestamp, metadata)
            })
            .await?;
        payload
            .decode_as::<Method::Response>()
            .map_err(|_| RiseWireError::ResponseDecodingFailed.into())
    }

    pub async fn call_descriptor(
        &self,
        descriptor: &RiseWireMethodDescriptor,
        request: &RiseJSONValue,
        request_id: Option<RiseRequestID>,
        metadata: RiseWireRequestMetadata,
    ) -> anyhow::Result<RiseJSONValue> {
        self.exchange(descriptor, request_id, |id, timestamp| {
            RiseWireRequestEncoder::encode(descriptor, request, id, timestamp, metadata)
        })
        .await
    }

    async fn exchange(
        &self,
        descriptor: &RiseWireMethodDescriptor,
        explicit_request_id: Option<RiseRequestID>,
        encode: impl FnOnce(RiseRequestID, i64) -> anyhow::Result<Vec<u8>>,
    ) -> anyhow::Result<RiseJSONValue> {
        let deadline = RiseClock::monotonic() + descriptor.timeout;
        if descriptor.transport == RiseWireTransport::Http
            && let Some(http_sender) = self.http_sender.clone()
        {
            let request_id =
                explicit_request_id.unwrap_or_else(|| self.allocator.next(Date::now()));
            let timestamp = RiseRequestIDAllocator::timestamp_seconds(request_id);
            let frame = encode(request_id, timestamp)?;
            return self.post(frame, request_id, descriptor, deadline, http_sender).await;
        }
        let session = self.ready_session_for_call(descriptor).await?;
        let request_id = explicit_request_id.unwrap_or_else(|| self.allocator.next(Date::now()));
        let timestamp = RiseRequestIDAllocator::timestamp_seconds(request_id);
        let frame = encode(request_id, timestamp)?;
        let fingerprint = explicit_request_id.map(|_| Sha256::hash(&frame));
        if frame.len() > self.configuration.maximum_outbound_frame_bytes {
            return Err(RiseWireError::OutboundFrameTooLarge {
                limit: self.configuration.maximum_outbound_frame_bytes,
            }
            .into());
        }
        self.state.with(|state| {
            state.validate_still_ready(&session, descriptor.survives_suspension)
        })?;

        let Some(remaining_timeout) = remaining(deadline) else {
            return Err(RiseWireError::Timeout(request_id).into());
        };
        self.wait_for_response(
            descriptor,
            request_id,
            frame,
            session,
            remaining_timeout,
            fingerprint,
            descriptor.replay_policy.clone(),
            descriptor.survives_suspension,
        )
        .await
    }

    pub async fn socket_session_started(&self, session: RiseWireSocketSession) {
        self.state.with(|state| {
            if state.is_shutdown || session.account != self.account {
                return;
            }
            if let Some(previous) = state.current_session.clone()
                && previous != session
            {
                state.fail_pending_matching(&previous, &|| {
                    RiseWireError::Disconnected("socket session replaced".to_owned()).into()
                });
                state.explicit_request_identities.clear();
            }
            state.current_session = Some(session.clone());
            state.is_connected = false;
            tracing::warn!(target: "riseonly::X1586", epoch = session.transport_epoch, "wire session started");
            state.publish(RiseWireEvent::SessionStarted(session));
        });
    }

    pub async fn socket_connected(&self, session: RiseWireSocketSession) {
        self.state.with(|state| {
            if state.is_shutdown
                || state.current_session.as_ref() != Some(&session)
                || session.account != self.account
            {
                tracing::warn!(
                    target: "riseonly::X1586",
                    epoch = session.transport_epoch,
                    current = ?state.current_session.as_ref().map(|current| current.transport_epoch),
                    shutdown = state.is_shutdown,
                    "wire connect ignored"
                );
                return;
            }
            tracing::warn!(target: "riseonly::X1586", epoch = session.transport_epoch, "wire connected");
            state.is_connected = true;
            state.resume_connection_waiters(&self.account);
            state.publish(RiseWireEvent::Connected(session));
        });
    }

    pub async fn socket_disconnected(&self, session: RiseWireSocketSession, reason: String) {
        self.state.with(|state| {
            if session.account != self.account {
                return;
            }
            let failure = reason.clone();
            state.fail_pending_matching(&session, &|| {
                RiseWireError::Disconnected(failure.clone()).into()
            });
            let is_current = state.current_session.as_ref() == Some(&session);
            tracing::warn!(target: "riseonly::X1586", epoch = session.transport_epoch, is_current, "wire disconnected");
            if is_current {
                state.current_session = None;
                state.is_connected = false;
                state.explicit_request_identities.clear();
            }
            state.publish(RiseWireEvent::Disconnected(session, reason));
        });
    }

    pub async fn socket_handshake_failed(&self) {
        self.state.with(|state| {
            if state.is_shutdown || state.is_connected {
                return;
            }
            state.fail_connection_waiters(&|| RiseWireError::NotConnected.into());
        });
    }

    pub async fn set_application_active(
        &self,
        active: bool,
        session: Option<RiseWireSocketSession>,
    ) {
        self.state.with(|state| {
            if state.is_shutdown {
                return;
            }
            state.is_application_active = active;
            tracing::warn!(target: "riseonly::X1586", active, connected = state.is_connected, "wire application active");
            let session = session.or_else(|| state.current_session.clone());
            if active {
                state.resume_connection_waiters(&self.account);
                state.publish(RiseWireEvent::Resumed(session));
            } else {
                let suspendable: Vec<Uuid> = state
                    .connection_waiters
                    .iter()
                    .filter(|(_, waiter)| !waiter.survives_suspension)
                    .map(|(id, _)| *id)
                    .collect();
                for id in suspendable {
                    state.finish_connection_waiter(id, Some(RiseWireError::Suspended.into()));
                }
                state.fail_pending_suspendable(&|| RiseWireError::Suspended.into());
                state.publish(RiseWireEvent::Suspended(session));
            }
        });
    }

    pub async fn is_ready_for_calls(&self, survives_suspension: bool) -> bool {
        self.state.with(|state| state.is_ready_for_calls(&self.account, survives_suspension))
    }

    /// each input of is_ready_for_calls, for send diagnostics
    pub(crate) fn readiness_probe(&self) -> String {
        self.state.with(|state| {
            format!(
                "shutdown={} app_active={} connected={} session_epoch={:?} session_account_current={}",
                state.is_shutdown,
                state.is_application_active,
                state.is_connected,
                state.current_session.as_ref().map(|session| session.transport_epoch),
                state.current_session.as_ref().is_some_and(|session| session.account == self.account),
            )
        })
    }

    /// true if a call with this descriptor can go out now, an http call needs no socket
    pub async fn can_call(&self, descriptor: &RiseWireMethodDescriptor) -> bool {
        if descriptor.transport == RiseWireTransport::Http && self.http_sender.is_some() {
            return self.state.with(|state| !state.is_shutdown);
        }
        self.state.with(|state| {
            state.is_ready_for_calls(&self.account, descriptor.survives_suspension)
        })
    }

    pub(crate) async fn cancel_outstanding_calls(&self) {
        self.state.with(|state| {
            state.fail_connection_waiters(&|| CancellationError.into());
            state.fail_all_pending(&|| CancellationError.into());
        });
    }

    pub async fn accept(
        &self,
        data: &[u8],
        session: &RiseWireSocketSession,
    ) -> RiseWireInboundDisposition {
        let admitted = self.state.with(|state| {
            !state.is_shutdown
                && session.account == self.account
                && state.current_session.as_ref() == Some(session)
        });
        if !admitted {
            return RiseWireInboundDisposition::Ignored;
        }
        if data.len() > self.configuration.maximum_inbound_frame_bytes {
            self.state.with(|state| {
                state.publish(RiseWireEvent::ProtocolViolation(
                    "inbound frame too large".to_owned(),
                    Some(session.clone()),
                ));
            });
            return RiseWireInboundDisposition::Ignored;
        }

        let Ok(incoming) = self.frame_decoder.decode(data) else {
            self.state.with(|state| {
                state.publish(RiseWireEvent::ProtocolViolation(
                    "invalid inbound frame".to_owned(),
                    Some(session.clone()),
                ));
            });
            return RiseWireInboundDisposition::Ignored;
        };

        self.state.with(|state| {
            if state.current_session.as_ref() != Some(session) {
                return RiseWireInboundDisposition::Ignored;
            }
            match incoming {
                RiseWireIncomingFrame::Response(response) => {
                    let matches = state
                        .pending
                        .get(&response.request_id)
                        .is_some_and(|request| request.session == *session);
                    if !matches {
                        return RiseWireInboundDisposition::Ignored;
                    }
                    let Some(request) = state.pending.remove(&response.request_id) else {
                        return RiseWireInboundDisposition::Ignored;
                    };
                    if let Some(error) = response.error {
                        request.resume(Err(error.into()));
                    } else if response.status.is_error() {
                        request.resume(Err(remote_request_failed().into()));
                    } else {
                        request.resume(Ok(response.data.unwrap_or(RiseJSONValue::Null)));
                    }
                    RiseWireInboundDisposition::ConsumedResponse
                }
                RiseWireIncomingFrame::Push(push) => {
                    state.publish(RiseWireEvent::Push(push, session.clone()));
                    RiseWireInboundDisposition::Event
                }
                RiseWireIncomingFrame::GatewayError(error) => {
                    state.publish(RiseWireEvent::GatewayError(error, session.clone()));
                    RiseWireInboundDisposition::Event
                }
            }
        })
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| {
            if state.is_shutdown {
                return;
            }
            state.is_shutdown = true;
            state.fail_connection_waiters(&|| RiseWireError::Shutdown.into());
            state.fail_all_pending(&|| RiseWireError::Shutdown.into());
            state.explicit_request_identities.clear();
            state.subscribers.clear();
            state.current_session = None;
            state.is_connected = false;
        });
    }

    async fn ready_session_for_call(
        &self,
        descriptor: &RiseWireMethodDescriptor,
    ) -> anyhow::Result<RiseWireSocketSession> {
        let survives_suspension = descriptor.survives_suspension;
        let is_read = descriptor.replay_policy == RiseWireReplayPolicy::ReadOnly;
        let needs_socket = || {
            self.state.with(|state| {
                !state.is_shutdown && !state.is_ready_for_calls(&self.account, survives_suspension)
            })
        };
        if is_read
            && needs_socket()
            && let Some(socket_demand) = RiseEngine::socket_demand()
            && !socket_demand().await
            && !self
                .state
                .with(|state| state.is_ready_for_calls(&self.account, survives_suspension))
        {
            return Err(RiseWireError::NotConnected.into());
        }
        if is_read && needs_socket() {
            let maximum = self.configuration.maximum_pending_requests;
            let id = Uuid::new_v4();
            let (continuation, resumed) = oneshot::channel();
            let waits = self.state.with(|state| {
                if state.is_shutdown
                    || state.is_ready_for_calls(&self.account, survives_suspension)
                {
                    return Ok(false);
                }
                if state.connection_waiters.len() + state.pending.len() >= maximum {
                    return Err(RiseWireError::PendingLimitReached(maximum));
                }
                let wire = self.this.clone();
                let timeout = descriptor.timeout;
                let timeout_task = RiseAccountRuntime::current().spawn(async move {
                    tokio::time::sleep(timeout).await;
                    if let Some(wire) = wire.upgrade() {
                        wire.finish_connection_waiter(id, Some(RiseWireError::NotConnected.into()));
                    }
                });
                state.connection_waiters.insert(
                    id,
                    ConnectionWaiter { survives_suspension, continuation, timeout_task },
                );
                Ok(true)
            })?;
            if waits {
                let _cancellation = ConnectionWaiterCancellation { wire: self.this.clone(), id };
                match resumed.await {
                    Ok(result) => result?,
                    Err(_) => return Err(CancellationError.into()),
                }
            }
        }
        Ok(self.state.with(|state| state.ready_session(&self.account, survives_suspension))?)
    }

    fn finish_connection_waiter(&self, id: Uuid, error: Option<anyhow::Error>) {
        self.state.with(|state| state.finish_connection_waiter(id, error));
    }

    /// one http exchange, no socket and no pending table and no replay
    /// the response frame goes through the socket decoder and must answer this request id
    async fn post(
        &self,
        frame: Vec<u8>,
        request_id: RiseRequestID,
        descriptor: &RiseWireMethodDescriptor,
        deadline: Duration,
        sender: Arc<dyn RiseWireHttpSender>,
    ) -> anyhow::Result<RiseJSONValue> {
        if self.state.with(|state| state.is_shutdown) {
            return Err(RiseWireError::Shutdown.into());
        }
        if frame.len() > self.configuration.maximum_outbound_frame_bytes {
            return Err(RiseWireError::OutboundFrameTooLarge {
                limit: self.configuration.maximum_outbound_frame_bytes,
            }
            .into());
        }
        let Some(timeout) = remaining(deadline) else {
            return Err(RiseWireError::Timeout(request_id).into());
        };
        let (context, trace) = self.begin_trace(descriptor);
        let result = self.post_exchange(frame, request_id, timeout, context, sender).await;
        trace.finish(result.as_ref().err());
        result
    }

    async fn post_exchange(
        &self,
        frame: Vec<u8>,
        request_id: RiseRequestID,
        timeout: Duration,
        context: RiseEngineCallContext,
        sender: Arc<dyn RiseWireHttpSender>,
    ) -> anyhow::Result<RiseJSONValue> {
        let account = self.account.clone();
        let data = RiseEngineObservability::with_call_context(
            context,
            RiseEngineObservability::with_request_id(
                request_id,
                sender.post(frame, account, timeout),
            ),
        )
        .await?;
        if self.state.with(|state| state.is_shutdown) {
            return Err(RiseWireError::Shutdown.into());
        }
        if data.len() > self.configuration.maximum_inbound_frame_bytes {
            return Err(RiseWireError::InboundFrameTooLarge {
                limit: self.configuration.maximum_inbound_frame_bytes,
            }
            .into());
        }
        let incoming = self.frame_decoder.decode(&data)?;
        let RiseWireIncomingFrame::Response(response) = incoming else {
            return Err(RiseWireContractError::InvalidFrame.into());
        };
        if response.request_id != request_id {
            return Err(RiseWireContractError::InvalidFrame.into());
        }
        if let Some(error) = response.error {
            return Err(error.into());
        }
        if response.status.is_error() {
            return Err(remote_request_failed().into());
        }
        Ok(response.data.unwrap_or(RiseJSONValue::Null))
    }

    #[allow(clippy::too_many_arguments)]
    async fn wait_for_response(
        &self,
        descriptor: &RiseWireMethodDescriptor,
        request_id: RiseRequestID,
        frame: Vec<u8>,
        session: RiseWireSocketSession,
        timeout: Duration,
        explicit_fingerprint: Option<Sha256Digest>,
        replay_policy: RiseWireReplayPolicy,
        survives_suspension: bool,
    ) -> anyhow::Result<RiseJSONValue> {
        let maximum_pending = self.configuration.maximum_pending_requests;
        let maximum_identities = self.configuration.maximum_explicit_request_identities;
        self.state.with(|state| {
            state.validate_still_ready(&session, survives_suspension)?;
            if state.pending.len() + state.connection_waiters.len() >= maximum_pending {
                return Err(RiseWireError::PendingLimitReached(maximum_pending));
            }
            if state.pending.contains_key(&request_id) {
                return Err(RiseWireError::DuplicateRequestID(request_id));
            }
            if let Some(fingerprint) = explicit_fingerprint {
                state.register_explicit_request_identity(
                    request_id,
                    &session,
                    fingerprint,
                    replay_policy,
                    maximum_identities,
                )?;
            }
            Ok(())
        })?;
        let attempt_id = Uuid::new_v4();
        let (context, trace) = self.begin_trace(descriptor);

        let (continuation, resumed) = oneshot::channel();
        self.state.with(|state| {
            let runtime = RiseAccountRuntime::current();
            let wire = self.this.clone();
            let timeout_task = runtime.spawn(async move {
                tokio::time::sleep(timeout).await;
                if let Some(wire) = wire.upgrade() {
                    wire.timeout(request_id, attempt_id);
                }
            });
            state.pending.insert(
                request_id,
                PendingRequest {
                    attempt_id,
                    session: session.clone(),
                    survives_suspension,
                    continuation,
                    timeout_task,
                    send_task: None,
                },
            );
            let wire = self.this.clone();
            let sender = self.sender.clone();
            let send_task = runtime.spawn(async move {
                let sent = RiseEngineObservability::with_call_context(
                    context,
                    RiseEngineObservability::with_request_id(
                        request_id,
                        sender.send(frame, session),
                    ),
                )
                .await;
                if let Err(error) = sent
                    && let Some(wire) = wire.upgrade()
                {
                    wire.fail(request_id, attempt_id, error);
                }
            });
            match state.pending.get_mut(&request_id) {
                Some(request) if request.attempt_id == attempt_id => {
                    request.send_task = Some(send_task);
                }
                _ => send_task.abort(),
            }
        });
        let _cancellation =
            PendingRequestCancellation { wire: self.this.clone(), request_id, attempt_id };
        let result = match resumed.await {
            Ok(result) => result,
            Err(_) => Err(CancellationError.into()),
        };
        trace.finish(result.as_ref().err());
        result
    }

    fn begin_trace(
        &self,
        descriptor: &RiseWireMethodDescriptor,
    ) -> (RiseEngineCallContext, RiseWireTrace) {
        let existing = RiseEngineObservability::call_context();
        let owns_trace = existing.is_none();
        let context = existing.unwrap_or_else(|| {
            RiseEngineCallContext::new(
                Uuid::new_v4(),
                descriptor.service.clone(),
                descriptor.method.clone(),
                descriptor.replay_policy.clone(),
                self.account.account_id.clone(),
                Date::now(),
            )
        });
        if owns_trace {
            RiseEngineObservability::begin(&context);
        }
        let trace = RiseWireTrace { context: owns_trace.then(|| context.clone()) };
        (context, trace)
    }

    fn timeout(&self, request_id: RiseRequestID, attempt_id: Uuid) {
        self.state.with(|state| {
            if !state.pending.get(&request_id).is_some_and(|r| r.attempt_id == attempt_id) {
                return;
            }
            let Some(request) = state.pending.remove(&request_id) else { return };
            if let Some(send_task) = &request.send_task {
                send_task.abort();
            }
            let _ = request.continuation.send(Err(RiseWireError::Timeout(request_id).into()));
        });
    }

    fn cancel(&self, request_id: RiseRequestID, attempt_id: Uuid) {
        self.finish_attempt(request_id, attempt_id, CancellationError.into());
    }

    fn fail(&self, request_id: RiseRequestID, attempt_id: Uuid, error: anyhow::Error) {
        self.finish_attempt(request_id, attempt_id, error);
    }

    fn finish_attempt(&self, request_id: RiseRequestID, attempt_id: Uuid, error: anyhow::Error) {
        self.state.with(|state| {
            if !state.pending.get(&request_id).is_some_and(|r| r.attempt_id == attempt_id) {
                return;
            }
            if let Some(request) = state.pending.remove(&request_id) {
                request.resume(Err(error));
            }
        });
    }

    fn remove_subscriber(&self, identifier: Uuid) {
        self.state.with(|state| {
            state.subscribers.remove(&identifier);
        });
    }

    fn cancel_subscriber(&self, identifier: Uuid) {
        self.state.with(|state| {
            state.subscribers.remove(&identifier);
        });
    }
}

impl RiseWireState {
    fn resume_connection_waiters(&mut self, account: &RiseWireAccountContext) {
        let ready: Vec<Uuid> = self
            .connection_waiters
            .iter()
            .filter(|(_, waiter)| self.is_ready_for_calls(account, waiter.survives_suspension))
            .map(|(id, _)| *id)
            .collect();
        for id in ready {
            self.finish_connection_waiter(id, None);
        }
    }

    fn fail_connection_waiters(&mut self, error: &dyn Fn() -> anyhow::Error) {
        let identifiers: Vec<Uuid> = self.connection_waiters.keys().copied().collect();
        for id in identifiers {
            self.finish_connection_waiter(id, Some(error()));
        }
    }

    fn finish_connection_waiter(&mut self, id: Uuid, error: Option<anyhow::Error>) {
        let Some(waiter) = self.connection_waiters.remove(&id) else { return };
        waiter.timeout_task.abort();
        let _ = waiter.continuation.send(match error {
            Some(error) => Err(error),
            None => Ok(()),
        });
    }

    fn is_ready_for_calls(
        &self,
        account: &RiseWireAccountContext,
        survives_suspension: bool,
    ) -> bool {
        !self.is_shutdown
            && (self.is_application_active || survives_suspension)
            && self.is_connected
            && self.current_session.as_ref().is_some_and(|session| session.account == *account)
    }

    fn ready_session(
        &self,
        account: &RiseWireAccountContext,
        survives_suspension: bool,
    ) -> Result<RiseWireSocketSession, RiseWireError> {
        if self.is_shutdown {
            return Err(RiseWireError::Shutdown);
        }
        if !(self.is_application_active || survives_suspension) {
            return Err(RiseWireError::Suspended);
        }
        let Some(session) = &self.current_session else {
            return Err(RiseWireError::NoSocketSession);
        };
        if session.account != *account {
            return Err(RiseWireError::StaleAccount);
        }
        if !self.is_connected {
            return Err(RiseWireError::NotConnected);
        }
        Ok(session.clone())
    }

    fn validate_still_ready(
        &self,
        session: &RiseWireSocketSession,
        survives_suspension: bool,
    ) -> Result<(), RiseWireError> {
        if self.is_shutdown {
            return Err(RiseWireError::Shutdown);
        }
        if !(self.is_application_active || survives_suspension) {
            return Err(RiseWireError::Suspended);
        }
        if self.current_session.as_ref() != Some(session) {
            return Err(RiseWireError::StaleSession);
        }
        if !self.is_connected {
            return Err(RiseWireError::NotConnected);
        }
        Ok(())
    }

    fn fail_pending_matching(
        &mut self,
        session: &RiseWireSocketSession,
        error: &dyn Fn() -> anyhow::Error,
    ) {
        let identifiers: Vec<RiseRequestID> = self
            .pending
            .iter()
            .filter(|(_, request)| request.session == *session)
            .map(|(id, _)| *id)
            .collect();
        for identifier in identifiers {
            if let Some(request) = self.pending.remove(&identifier) {
                request.resume(Err(error()));
            }
        }
    }

    fn fail_pending_suspendable(&mut self, error: &dyn Fn() -> anyhow::Error) {
        let identifiers: Vec<RiseRequestID> = self
            .pending
            .iter()
            .filter(|(_, request)| !request.survives_suspension)
            .map(|(id, _)| *id)
            .collect();
        for identifier in identifiers {
            if let Some(request) = self.pending.remove(&identifier) {
                request.resume(Err(error()));
            }
        }
    }

    fn fail_all_pending(&mut self, error: &dyn Fn() -> anyhow::Error) {
        for (_, request) in self.pending.drain() {
            request.resume(Err(error()));
        }
    }

    fn publish(&self, event: RiseWireEvent) {
        for subscriber in self.subscribers.values() {
            let dropped = subscriber.continuation.len() >= self.event_buffer_capacity;
            let _ = subscriber.continuation.send(event.clone());
            if dropped {
                let _ = subscriber
                    .continuation
                    .send(RiseWireEvent::EventBufferOverflow(self.current_session.clone()));
            }
        }
    }

    fn register_explicit_request_identity(
        &mut self,
        request_id: RiseRequestID,
        session: &RiseWireSocketSession,
        fingerprint: Sha256Digest,
        replay_policy: RiseWireReplayPolicy,
        maximum: usize,
    ) -> Result<(), RiseWireError> {
        if let Some(previous) = self.explicit_request_identities.get(&request_id) {
            if previous.session != *session
                || previous.fingerprint != fingerprint
                || !previous.replay_policy.permits_replay()
                || !replay_policy.permits_replay()
            {
                return Err(RiseWireError::UnsafeRequestIDReuse(request_id));
            }
            return Ok(());
        }
        if self.explicit_request_identities.len() >= maximum {
            return Err(RiseWireError::ExplicitRequestIdentityLimitReached(maximum));
        }
        self.explicit_request_identities.insert(
            request_id,
            ExplicitRequestIdentity { session: session.clone(), fingerprint, replay_policy },
        );
        Ok(())
    }
}

fn remote_request_failed() -> RiseWireRemoteError {
    RiseWireRemoteError::new(
        ERROR_CODE_REMOTE_DEFAULT.to_owned(),
        DEFAULT_REMOTE_MESSAGE.to_owned(),
        None,
    )
}

/// time left until the deadline, only if still positive
fn remaining(deadline: Duration) -> Option<Duration> {
    deadline.checked_sub(RiseClock::monotonic()).filter(|left| !left.is_zero())
}

/// trace of a call that owns it, finishes on drop
struct RiseWireTrace {
    context: Option<RiseEngineCallContext>,
}

impl RiseWireTrace {
    fn finish(mut self, error: Option<&anyhow::Error>) {
        if let Some(context) = self.context.take() {
            RiseEngineObservability::finish(&context, error);
        }
    }
}

impl Drop for RiseWireTrace {
    fn drop(&mut self) {
        if let Some(context) = self.context.take() {
            let cancelled = anyhow::Error::new(CancellationError);
            RiseEngineObservability::finish(&context, Some(&cancelled));
        }
    }
}

struct ConnectionWaiterCancellation {
    wire: Weak<RiseWire>,
    id: Uuid,
}

impl Drop for ConnectionWaiterCancellation {
    fn drop(&mut self) {
        if let Some(wire) = self.wire.upgrade() {
            wire.finish_connection_waiter(self.id, Some(CancellationError.into()));
        }
    }
}

struct PendingRequestCancellation {
    wire: Weak<RiseWire>,
    request_id: RiseRequestID,
    attempt_id: Uuid,
}

impl Drop for PendingRequestCancellation {
    fn drop(&mut self) {
        if let Some(wire) = self.wire.upgrade() {
            wire.cancel(self.request_id, self.attempt_id);
        }
    }
}
