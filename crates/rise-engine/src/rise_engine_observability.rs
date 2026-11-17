use std::future::Future;
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::rise_core::rise_identifiers::RiseRequestID;
use crate::rise_wire::rise_wire_contracts::RiseWireReplayPolicy;

tokio::task_local! {
    static CALL_CONTEXT: Option<RiseEngineCallContext>;
    static REQUEST_ID: Option<RiseRequestID>;
}

#[derive(Clone)]
pub struct RiseEngineCallContext {
    pub id: Uuid,
    pub service: String,
    pub method: String,
    pub replay_policy: RiseWireReplayPolicy,
    pub account_id: String,
    pub started_at: Date,
    transmission: Arc<RiseEngineCallContextTransmission>,
}

#[derive(Clone, Debug)]
pub struct RiseEngineCallContextTransportAttempt {
    pub request_id: RiseRequestID,
    pub started_at: Date,
    pub request_summary: String,
}

#[derive(Default)]
struct RiseEngineCallContextTransmission {
    attempt: Mutex<Option<RiseEngineCallContextTransportAttempt>>,
}

impl RiseEngineCallContext {
    pub fn new(
        id: Uuid,
        service: String,
        method: String,
        replay_policy: RiseWireReplayPolicy,
        account_id: String,
        started_at: Date,
    ) -> Self {
        Self {
            id,
            service,
            method,
            replay_policy,
            account_id,
            started_at,
            transmission: Arc::new(RiseEngineCallContextTransmission::default()),
        }
    }

    pub fn transport_attempt(&self) -> Option<RiseEngineCallContextTransportAttempt> {
        self.transmission.attempt.lock().clone()
    }

    fn record_transport_attempt(&self, request_id: RiseRequestID, request_summary: String) {
        *self.transmission.attempt.lock() = Some(RiseEngineCallContextTransportAttempt {
            request_id,
            started_at: Date::now(),
            request_summary,
        });
    }
}

/// gets every call the engine makes
/// instrumentation lives here so a repo that forgets to wrap its rpc cant vanish from the debugger
pub trait RiseEngineCallObserver: Send + Sync {
    fn engine_call_did_begin(&self, context: &RiseEngineCallContext);

    fn engine_call_will_send(&self, context: &RiseEngineCallContext) {
        let _ = context;
    }

    fn engine_call_did_finish(
        &self,
        context: &RiseEngineCallContext,
        duration_milliseconds: i64,
        error: Option<&anyhow::Error>,
    );
}

static OBSERVERS: LazyLock<Mutex<Vec<Arc<dyn RiseEngineCallObserver>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

/// process wide observer registry, global on purpose
/// the debugger outlives accounts and would lose the switch own calls
pub struct RiseEngineObservability;

impl RiseEngineObservability {
    /// call context bound around every engine call
    pub fn call_context() -> Option<RiseEngineCallContext> {
        CALL_CONTEXT.try_with(Clone::clone).ok().flatten()
    }

    /// request id bound around one transport attempt
    pub fn request_id() -> Option<RiseRequestID> {
        REQUEST_ID.try_with(|request_id| *request_id).ok().flatten()
    }

    /// runs the future with this call context bound
    pub fn with_call_context<F: Future>(
        context: RiseEngineCallContext,
        future: F,
    ) -> impl Future<Output = F::Output> {
        CALL_CONTEXT.scope(Some(context), future)
    }

    /// runs the future with this request id bound
    pub fn with_request_id<F: Future>(
        request_id: RiseRequestID,
        future: F,
    ) -> impl Future<Output = F::Output> {
        REQUEST_ID.scope(Some(request_id), future)
    }

    /// called by the transport after its session guards right before send
    /// encoding and cache hits and local rejects are not transport attempts
    pub fn transport_will_send(request_summary: &str) {
        let (Some(context), Some(request_id)) = (Self::call_context(), Self::request_id()) else {
            return;
        };
        context.record_transport_attempt(request_id, request_summary.to_owned());
        for observer in Self::current() {
            observer.engine_call_will_send(&context);
        }
    }

    pub fn install(observer: Arc<dyn RiseEngineCallObserver>) {
        let mut observers = OBSERVERS.lock();
        if observers.iter().any(|installed| Arc::ptr_eq(installed, &observer)) {
            return;
        }
        observers.push(observer);
    }

    pub fn remove(observer: &Arc<dyn RiseEngineCallObserver>) {
        OBSERVERS.lock().retain(|installed| !Arc::ptr_eq(installed, observer));
    }

    pub(crate) fn has_observers() -> bool {
        !OBSERVERS.lock().is_empty()
    }

    pub(crate) fn current() -> Vec<Arc<dyn RiseEngineCallObserver>> {
        OBSERVERS.lock().clone()
    }

    pub(crate) fn begin(context: &RiseEngineCallContext) {
        for observer in Self::current() {
            observer.engine_call_did_begin(context);
        }
    }

    pub(crate) fn finish(context: &RiseEngineCallContext, error: Option<&anyhow::Error>) {
        let elapsed = Date::now().time_interval_since(&context.started_at) * 1_000.0;
        let duration = (elapsed as i64).max(0);
        for observer in Self::current() {
            observer.engine_call_did_finish(context, duration, error);
        }
    }
}
