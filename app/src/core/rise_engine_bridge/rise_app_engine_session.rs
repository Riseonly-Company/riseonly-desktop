use std::any::Any;
use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::future::{BoxFuture, Shared, join_all};
use futures::FutureExt;
use parking_lot::Mutex;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_sync::{RiseSyncDurableEndpoint, RiseSyncDurableHandler, RiseSyncPushHandler};
use rise_engine::{
    CancellationError, RiseEngine, RiseEngineConfiguration, RiseWireAccountContext,
};

use crate::core::diagnostics::rise_diagnostics_uploader::RiseDiagnosticsUploader;
use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::{
    RiseAppEngineCommentDomainHooks, RiseAppEngineConversationDomainHooks,
    RiseAppEngineDomainPrepare, RiseAppEngineGiftsDomainHooks, RiseAppEngineGuidesDomainHooks,
    RiseAppEngineMapDomainHooks, RiseAppEngineModerationDomainHooks,
    RiseAppEngineMusicDomainHooks, RiseAppEngineNotifyDomainHooks, RiseAppEnginePostDomainHooks,
    RiseAppEngineRtcDomainHooks, RiseAppEngineSearchDomainHooks, RiseAppEngineSparksDomainHooks,
    RiseAppEngineThemeDomainHooks, RiseAppEngineUserDomainHooks,
};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::core::rise_engine_bridge::rise_inbound_ownership_router::{
    RiseInboundOwnershipPolicy, RiseInboundOwnershipRouter,
};

pub trait RiseAppEngineImporter: Send + Sync {
    fn import_data<'a>(&'a self, engine: &'a RiseEngine) -> BoxFuture<'a, anyhow::Result<()>>;
}

type RiseAppEngineDomainThrowingOperation =
    Arc<dyn Fn() -> BoxFuture<'static, anyhow::Result<()>> + Send + Sync>;
type RiseAppEngineDomainOperation = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

/// parameters of the domain lifetime init, every operation is a no-op unless the domain gives it
pub struct RiseAppEngineDomainLifetimeArgs {
    pub activate: RiseAppEngineDomainThrowingOperation,
    pub did_activate_ownership: RiseAppEngineDomainOperation,
    pub socket_connected: RiseAppEngineDomainOperation,
    pub application_did_become_active: RiseAppEngineDomainOperation,
    pub shutdown: RiseAppEngineDomainOperation,
}

impl Default for RiseAppEngineDomainLifetimeArgs {
    fn default() -> Self {
        Self {
            activate: Arc::new(|| async { Ok::<(), anyhow::Error>(()) }.boxed()),
            did_activate_ownership: Arc::new(|| async {}.boxed()),
            socket_connected: Arc::new(|| async {}.boxed()),
            application_did_become_active: Arc::new(|| async {}.boxed()),
            shutdown: Arc::new(|| async {}.boxed()),
        }
    }
}

#[derive(Clone)]
pub struct RiseAppEngineDomainLifetime {
    activate_operation: RiseAppEngineDomainThrowingOperation,
    did_activate_ownership_operation: RiseAppEngineDomainOperation,
    socket_connected_operation: RiseAppEngineDomainOperation,
    application_did_become_active_operation: RiseAppEngineDomainOperation,
    shutdown_operation: RiseAppEngineDomainOperation,
    domain: Option<Arc<dyn Any + Send + Sync>>,
}

impl RiseAppEngineDomainLifetime {
    pub fn empty() -> Self {
        Self::new(RiseAppEngineDomainLifetimeArgs::default())
    }

    pub fn new(args: RiseAppEngineDomainLifetimeArgs) -> Self {
        Self {
            activate_operation: args.activate,
            did_activate_ownership_operation: args.did_activate_ownership,
            socket_connected_operation: args.socket_connected,
            application_did_become_active_operation: args.application_did_become_active,
            shutdown_operation: args.shutdown,
            domain: None,
        }
    }

    /// the domain object this lifetime drives, for composition between domains
    pub fn with_domain(mut self, domain: Arc<dyn Any + Send + Sync>) -> Self {
        self.domain = Some(domain);
        self
    }

    pub fn domain(&self) -> Option<&Arc<dyn Any + Send + Sync>> {
        self.domain.as_ref()
    }

    pub async fn application_did_become_active(&self) {
        (self.application_did_become_active_operation)().await;
    }

    pub async fn activate(&self) -> anyhow::Result<()> {
        (self.activate_operation)().await
    }

    pub async fn did_activate_ownership(&self) {
        (self.did_activate_ownership_operation)().await;
    }

    pub async fn socket_connected(&self) {
        (self.socket_connected_operation)().await;
    }

    pub async fn shutdown(&self) {
        (self.shutdown_operation)().await;
    }
}

#[derive(Default)]
struct RiseInboundOwnershipClaims {
    event_types: Mutex<HashSet<String>>,
}

impl RiseInboundOwnershipClaims {
    fn insert(&self, event_type: &str) {
        let Some(normalized) = RiseInboundOwnershipPolicy::normalized(Some(event_type)) else {
            return;
        };
        self.event_types.lock().insert(normalized);
    }

    fn snapshot(&self) -> HashSet<String> {
        self.event_types.lock().clone()
    }
}

#[derive(Clone)]
pub struct RiseAppEngineBootstrapContext {
    pub engine: Arc<RiseEngine>,
    inbound_ownership_claims: Arc<RiseInboundOwnershipClaims>,
}

impl RiseAppEngineBootstrapContext {
    fn new(engine: Arc<RiseEngine>, inbound_ownership_claims: Arc<RiseInboundOwnershipClaims>) -> Self {
        Self { engine, inbound_ownership_claims }
    }

    /// a push type is claimed only after its reducer registered fine
    /// the claim is published later when the app runtime installs the whole session atomically
    pub async fn register_push_handler(&self, push_handler: RiseSyncPushHandler) -> anyhow::Result<()> {
        let event_type = push_handler.event_type.clone();
        self.engine.register_push_handler(push_handler).await?;
        self.inbound_ownership_claims.insert(&event_type);
        Ok(())
    }

    pub async fn register_durable_handler(
        &self,
        durable_handler: RiseSyncDurableHandler,
    ) -> anyhow::Result<()> {
        self.engine.register_durable_handler(durable_handler).await
    }

    pub async fn register_durable_endpoint<Endpoint: RiseSyncDurableEndpoint>(
        &self,
    ) -> anyhow::Result<()> {
        self.engine.register_durable_endpoint::<Endpoint>().await
    }

    pub async fn run_importer<Importer: RiseAppEngineImporter>(
        &self,
        importer: &Importer,
    ) -> anyhow::Result<()> {
        importer.import_data(&self.engine).await
    }

    pub async fn run_importer_operation<Operation, Imported>(
        &self,
        operation: Operation,
    ) -> anyhow::Result<()>
    where
        Operation: FnOnce(Arc<RiseEngine>) -> Imported,
        Imported: Future<Output = anyhow::Result<()>>,
    {
        operation(Arc::clone(&self.engine)).await
    }
}

pub type RiseAppEngineBootstrapHook = Arc<
    dyn Fn(RiseAppEngineBootstrapContext) -> BoxFuture<'static, anyhow::Result<RiseAppEngineDomainLifetime>>
        + Send
        + Sync,
>;

/// central composition point for feature migrations
/// a domain must register reducers and durable endpoints and legacy
/// importers here before start, the empty one leaves every push to sai
pub enum RiseAppEngineDomainBootstrap {}

/// position of a domain in the composition order
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RiseAppEngineDomainSlot {
    User,
    Music,
    Post,
    Search,
    Conversation,
    Rtc,
    Notify,
    Guides,
    Theme,
    Moderation,
    Comment,
    Map,
    Sparks,
    Gifts,
}

impl RiseAppEngineDomainSlot {
    const ALL: [RiseAppEngineDomainSlot; 14] = [
        Self::User,
        Self::Music,
        Self::Post,
        Self::Search,
        Self::Conversation,
        Self::Rtc,
        Self::Notify,
        Self::Guides,
        Self::Theme,
        Self::Moderation,
        Self::Comment,
        Self::Map,
        Self::Sparks,
        Self::Gifts,
    ];

    const APPLICATION_DID_BECOME_ACTIVE: [RiseAppEngineDomainSlot; 4] =
        [Self::Rtc, Self::Conversation, Self::Notify, Self::Map];

    fn prepare(self) -> Option<&'static RiseAppEngineDomainPrepare> {
        match self {
            Self::User => RiseAppEngineUserDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Music => RiseAppEngineMusicDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Post => RiseAppEnginePostDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Search => RiseAppEngineSearchDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Conversation => {
                RiseAppEngineConversationDomainHooks::installed().map(|hooks| &hooks.prepare)
            }
            Self::Rtc => RiseAppEngineRtcDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Notify => RiseAppEngineNotifyDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Guides => RiseAppEngineGuidesDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Theme => RiseAppEngineThemeDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Moderation => {
                RiseAppEngineModerationDomainHooks::installed().map(|hooks| &hooks.prepare)
            }
            Self::Comment => {
                RiseAppEngineCommentDomainHooks::installed().map(|hooks| &hooks.prepare)
            }
            Self::Map => RiseAppEngineMapDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Sparks => RiseAppEngineSparksDomainHooks::installed().map(|hooks| &hooks.prepare),
            Self::Gifts => RiseAppEngineGiftsDomainHooks::installed().map(|hooks| &hooks.prepare),
        }
    }
}

impl RiseAppEngineDomainSlot {
    fn cold_open_stage(self) -> Option<&'static str> {
        match self {
            Self::Conversation => Some("activated conversation"),
            Self::User => Some("activated user"),
            Self::Music => Some("activated music"),
            Self::Post => Some("activated post"),
            Self::Search => Some("activated search"),
            _ => None,
        }
    }
}

impl RiseAppEngineDomainBootstrap {
    pub fn live() -> RiseAppEngineBootstrapHook {
        Arc::new(|context| Self::prepare(context).boxed())
    }

    pub async fn prepare(
        context: RiseAppEngineBootstrapContext,
    ) -> anyhow::Result<RiseAppEngineDomainLifetime> {
        let engine = Arc::clone(&context.engine);
        let mut prepared: Vec<(RiseAppEngineDomainSlot, RiseAppEngineDomainLifetime)> =
            Vec::with_capacity(RiseAppEngineDomainSlot::ALL.len());
        for slot in RiseAppEngineDomainSlot::ALL {
            let Some(prepare) = slot.prepare() else {
                continue;
            };
            match prepare(context.clone()).await {
                Ok(lifetime) => prepared.push((slot, lifetime)),
                Err(error) => {
                    for (_, lifetime) in prepared.iter().rev() {
                        lifetime.shutdown().await;
                    }
                    return Err(error);
                }
            }
            if slot == RiseAppEngineDomainSlot::Rtc {
                Self::set_call_history_sink(&prepared).await;
            }
        }
        Ok(Self::composed_lifetime(engine, prepared))
    }

    async fn set_call_history_sink(
        prepared: &[(RiseAppEngineDomainSlot, RiseAppEngineDomainLifetime)],
    ) {
        let Some(hooks) = RiseAppEngineRtcDomainHooks::installed() else {
            return;
        };
        let domain_of = |wanted: RiseAppEngineDomainSlot| {
            prepared
                .iter()
                .find(|(slot, _)| *slot == wanted)
                .and_then(|(_, lifetime)| lifetime.domain().cloned())
        };
        let (Some(rtc), Some(conversation)) = (
            domain_of(RiseAppEngineDomainSlot::Rtc),
            domain_of(RiseAppEngineDomainSlot::Conversation),
        ) else {
            return;
        };
        (hooks.set_call_history_sink)(rtc, Arc::downgrade(&conversation)).await;
    }

    fn drain_diagnostics(engine: Arc<RiseEngine>) -> BoxFuture<'static, ()> {
        async move { RiseDiagnosticsUploader::shared().drain(engine).await }.boxed()
    }

    fn composed_lifetime(
        engine: Arc<RiseEngine>,
        prepared: Vec<(RiseAppEngineDomainSlot, RiseAppEngineDomainLifetime)>,
    ) -> RiseAppEngineDomainLifetime {
        let prepared = Arc::new(prepared);
        let activate = {
            let prepared = Arc::clone(&prepared);
            Arc::new(move || {
                let prepared = Arc::clone(&prepared);
                async move {
                    let conversation_first = prepared
                        .iter()
                        .filter(|(slot, _)| *slot == RiseAppEngineDomainSlot::Conversation)
                        .chain(
                            prepared
                                .iter()
                                .filter(|(slot, _)| *slot != RiseAppEngineDomainSlot::Conversation),
                        );
                    for (slot, lifetime) in conversation_first {
                        lifetime.activate().await?;
                        if let Some(stage) = slot.cold_open_stage() {
                            RiseColdOpenTrace::mark(|| stage.to_owned());
                        }
                    }
                    Ok::<(), anyhow::Error>(())
                }
                .boxed()
            })
        };
        let did_activate_ownership = {
            let prepared = Arc::clone(&prepared);
            let engine = Arc::clone(&engine);
            Arc::new(move || {
                let prepared = Arc::clone(&prepared);
                let engine = Arc::clone(&engine);
                async move {
                    let mut operations: Vec<BoxFuture<'static, ()>> =
                        vec![Self::drain_diagnostics(engine)];
                    for (_, lifetime) in prepared.iter() {
                        let lifetime = lifetime.clone();
                        operations.push(async move { lifetime.did_activate_ownership().await }.boxed());
                    }
                    join_all(operations).await;
                }
                .boxed()
            })
        };
        let socket_connected = {
            let prepared = Arc::clone(&prepared);
            let engine = Arc::clone(&engine);
            Arc::new(move || {
                let prepared = Arc::clone(&prepared);
                let engine = Arc::clone(&engine);
                async move {
                    let mut operations: Vec<BoxFuture<'static, ()>> =
                        vec![Self::drain_diagnostics(engine)];
                    for (_, lifetime) in prepared.iter() {
                        let lifetime = lifetime.clone();
                        operations.push(async move { lifetime.socket_connected().await }.boxed());
                    }
                    join_all(operations).await;
                }
                .boxed()
            })
        };
        let application_did_become_active = {
            let prepared = Arc::clone(&prepared);
            Arc::new(move || {
                let prepared = Arc::clone(&prepared);
                async move {
                    for wanted in RiseAppEngineDomainSlot::APPLICATION_DID_BECOME_ACTIVE {
                        let lifetime = prepared
                            .iter()
                            .find(|(slot, _)| *slot == wanted)
                            .map(|(_, lifetime)| lifetime.clone());
                        if let Some(lifetime) = lifetime {
                            lifetime.application_did_become_active().await;
                        }
                    }
                }
                .boxed()
            })
        };
        let shutdown = {
            let prepared = Arc::clone(&prepared);
            Arc::new(move || {
                let prepared = Arc::clone(&prepared);
                async move {
                    for (_, lifetime) in prepared.iter().rev() {
                        lifetime.shutdown().await;
                    }
                }
                .boxed()
            })
        };
        RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
            activate,
            did_activate_ownership,
            socket_connected,
            application_did_become_active,
            shutdown,
        })
    }
}

/// an anyhow error that several waiters receive
/// a shared close or prepare gives every waiter the same failure
#[derive(Clone)]
pub struct RiseAppEngineSharedError(Arc<anyhow::Error>);

impl RiseAppEngineSharedError {
    pub fn new(error: anyhow::Error) -> Self {
        Self(Arc::new(error))
    }
}

impl fmt::Debug for RiseAppEngineSharedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.0.as_ref(), f)
    }
}

impl fmt::Display for RiseAppEngineSharedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.0.as_ref(), f)
    }
}

impl std::error::Error for RiseAppEngineSharedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let error: &(dyn std::error::Error + 'static) = self.0.as_ref().as_ref();
        Some(error)
    }
}

/// cancellation flag for work that must still clean up
/// dropping the future would skip domain shutdown and engine close
/// so the work reads the flag between steps
#[derive(Clone, Default)]
pub struct RiseAppEngineCancellation {
    cancelled: Arc<AtomicBool>,
}

impl RiseAppEngineCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn check_cancellation(&self) -> anyhow::Result<()> {
        if self.is_cancelled() {
            return Err(CancellationError.into());
        }
        Ok(())
    }
}

type RiseAppEngineCloseTask = Shared<BoxFuture<'static, Result<(), RiseAppEngineSharedError>>>;

#[derive(Default)]
struct RiseAppEngineSessionCloseGate {
    close_task: Mutex<Option<RiseAppEngineCloseTask>>,
}

impl RiseAppEngineSessionCloseGate {
    async fn close<Operation>(&self, operation: Operation) -> anyhow::Result<()>
    where
        Operation: FnOnce() -> BoxFuture<'static, anyhow::Result<()>>,
    {
        let close_task = {
            let mut slot = self.close_task.lock();
            match slot.as_ref() {
                Some(close_task) => close_task.clone(),
                None => {
                    let handle = RiseAccountRuntime::current().spawn(operation());
                    let close_task = async move {
                        match handle.await {
                            Ok(result) => result.map_err(RiseAppEngineSharedError::new),
                            Err(join_error) => {
                                Err(RiseAppEngineSharedError::new(anyhow::Error::new(join_error)))
                            }
                        }
                    }
                    .boxed()
                    .shared();
                    *slot = Some(close_task.clone());
                    close_task
                }
            }
        };
        close_task.await.map_err(anyhow::Error::new)
    }
}

pub struct RiseAppEngineSession {
    pub identifier: Uuid,
    pub engine: Arc<RiseEngine>,
    pub account: RiseWireAccountContext,
    pub rise_owned_push_event_types: HashSet<String>,

    domain_lifetime: RiseAppEngineDomainLifetime,
    ownership_router: &'static RiseInboundOwnershipRouter,
    close_gate: RiseAppEngineSessionCloseGate,
}

impl RiseAppEngineSession {
    fn new(
        engine: Arc<RiseEngine>,
        domain_lifetime: RiseAppEngineDomainLifetime,
        rise_owned_push_event_types: HashSet<String>,
        ownership_router: &'static RiseInboundOwnershipRouter,
    ) -> Self {
        let account = engine.account.clone();
        Self {
            identifier: Uuid::new_v4(),
            engine,
            account,
            rise_owned_push_event_types,
            domain_lifetime,
            ownership_router,
            close_gate: RiseAppEngineSessionCloseGate::default(),
        }
    }

    pub async fn open(
        configuration: RiseEngineConfiguration,
        ownership_router: &'static RiseInboundOwnershipRouter,
        bootstrap: RiseAppEngineBootstrapHook,
        cancellation: RiseAppEngineCancellation,
    ) -> anyhow::Result<Arc<RiseAppEngineSession>> {
        cancellation.check_cancellation()?;
        let engine = RiseEngine::open(configuration).await?;
        RiseColdOpenTrace::mark(|| "engine open".to_owned());
        let claims = Arc::new(RiseInboundOwnershipClaims::default());
        let mut domain_lifetime: Option<RiseAppEngineDomainLifetime> = None;
        let started = Self::bootstrap_and_start(
            &engine,
            &claims,
            &bootstrap,
            &cancellation,
            &mut domain_lifetime,
        )
        .await;
        match started {
            Ok(()) => Ok(Arc::new(RiseAppEngineSession::new(
                engine,
                domain_lifetime.unwrap_or_else(RiseAppEngineDomainLifetime::empty),
                claims.snapshot(),
                ownership_router,
            ))),
            Err(error) => {
                if let Some(domain_lifetime) = &domain_lifetime {
                    domain_lifetime.shutdown().await;
                }
                let _ = engine.close().await;
                Err(error)
            }
        }
    }

    async fn bootstrap_and_start(
        engine: &Arc<RiseEngine>,
        claims: &Arc<RiseInboundOwnershipClaims>,
        bootstrap: &RiseAppEngineBootstrapHook,
        cancellation: &RiseAppEngineCancellation,
        domain_lifetime: &mut Option<RiseAppEngineDomainLifetime>,
    ) -> anyhow::Result<()> {
        let context = RiseAppEngineBootstrapContext::new(Arc::clone(engine), Arc::clone(claims));
        *domain_lifetime = Some(bootstrap(context).await?);
        RiseColdOpenTrace::mark(|| "domains prepared".to_owned());
        cancellation.check_cancellation()?;
        engine.start().await?;
        RiseColdOpenTrace::mark(|| "engine started".to_owned());
        cancellation.check_cancellation()?;
        Ok(())
    }

    pub fn activate_inbound_ownership(&self) {
        self.ownership_router.install(
            self.identifier,
            &self.account.account_id,
            &self.account.epoch,
            &self.rise_owned_push_event_types,
        );
    }

    pub async fn activate_domain(&self) -> anyhow::Result<()> {
        self.domain_lifetime.activate().await
    }

    pub async fn did_activate_inbound_ownership(&self) {
        self.domain_lifetime.did_activate_ownership().await;
    }

    pub async fn socket_connected(&self) {
        self.domain_lifetime.socket_connected().await;
    }

    pub async fn application_did_become_active(&self) {
        self.domain_lifetime.application_did_become_active().await;
    }

    pub fn deactivate_inbound_ownership(&self) {
        self.ownership_router.relinquish(self.identifier);
    }

    pub async fn close(&self) -> anyhow::Result<()> {
        self.deactivate_inbound_ownership();
        let domain_lifetime = self.domain_lifetime.clone();
        let engine = Arc::clone(&self.engine);
        self.close_gate
            .close(move || {
                async move {
                    domain_lifetime.shutdown().await;
                    engine.close().await
                }
                .boxed()
            })
            .await
    }
}
