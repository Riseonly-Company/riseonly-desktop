use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::{LocalBoxFuture, Shared};
use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use parking_lot::RwLock;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_core::rise_secure_key_store::RiseKeychainKeyStore;
use rise_engine::{
    CancellationError, RiseEngine, RiseEngineConfiguration, RiseEngineConfigurationArgs,
    RiseWireAccountContext, RiseWireConfiguration, RiseWireInboundDisposition,
    RiseWireSocketSession,
};
use rise_platform::AppPaths;
use rise_platform::file_manager::{CocoaError, CocoaErrorCode, FileManager};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_configuration::SaiWebSocketConfiguration;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{
    SaiWebSocketManager, SaiWebSocketManagerPhase,
};
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineBootstrapHook, RiseAppEngineCancellation, RiseAppEngineDomainBootstrap,
    RiseAppEngineSession, RiseAppEngineSharedError,
};
use crate::core::rise_engine_bridge::rise_canon_events::RiseWireConfigurationCanonical;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::rise_engine_bridge::rise_inbound_ownership_router::RiseInboundOwnershipRouter;
use crate::core::rise_engine_bridge::rise_wire_http_adapter::RiseWireHttpAdapter;
use crate::core::rise_engine_bridge::rise_wire_sai_socket_adapter::RiseWireSaiSocketAdapter;

/// cold open stage marks, where a deep linked chat waits before its cached history
pub enum RiseColdOpenTrace {}

impl RiseColdOpenTrace {
    pub fn mark(stage: impl FnOnce() -> String) {
        let text = stage();
        let uptime_ms = rise_platform::process_info::system_uptime().map_or(0, |uptime| uptime.as_millis());
        tracing::info!(target: "net.riseonly.ColdOpen", "[8311] t={uptime_ms} {text}");
    }

    /// short per process tag of a message local id
    pub fn tag(local_id: &str) -> String {
        Self::hash_tag(&format!("msg-{local_id}"))
    }

    /// short per process tag of any text
    pub fn hash_tag(text: &str) -> String {
        use std::hash::BuildHasher;
        let hash = COLD_OPEN_HASH_SEED.hash_one(text) as i64;
        (hash.unsigned_abs() % 10_000).to_string()
    }
}

static COLD_OPEN_HASH_SEED: std::sync::LazyLock<std::hash::RandomState> =
    std::sync::LazyLock::new(std::hash::RandomState::new);

impl RiseColdOpenTrace {
    pub fn page_outcome(outcome: &crate::modules::chat::engine::messages::rise_message_scope::RiseMessagePageLoadOutcome) -> String {
        use crate::modules::chat::engine::messages::rise_message_scope::RiseMessagePageLoadOutcome;
        match outcome {
            RiseMessagePageLoadOutcome::Loaded { count } => format!("loaded(count: {count})"),
            RiseMessagePageLoadOutcome::StaleResponseDropped => "staleResponseDropped".to_owned(),
            RiseMessagePageLoadOutcome::CursorStalled => "cursorStalled".to_owned(),
            RiseMessagePageLoadOutcome::EndReached => "endReached".to_owned(),
            RiseMessagePageLoadOutcome::AlreadyLoading => "alreadyLoading".to_owned(),
        }
    }

    pub fn jump_outcome(outcome: &crate::modules::chat::engine::messages::rise_message_scope::RiseMessageJumpOutcome) -> &'static str {
        use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageJumpOutcome;
        match outcome {
            RiseMessageJumpOutcome::Installed => "installed",
            RiseMessageJumpOutcome::Superseded => "superseded",
            RiseMessageJumpOutcome::Failed => "failed",
        }
    }
}

pub const RISE_ENGINE_CONNECTION_READY: RiseNotificationName =
    RiseNotificationName::new("riseEngineConnectionReady");

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseEngineAppRuntimeError {
    #[error("missingActiveAccount")]
    MissingActiveAccount,
}

static APPLICATION_SUPPORT_DIRECTORY: RwLock<Option<PathBuf>> = RwLock::new(None);

const LAUNCH_SCOPED_ENGINE_DIRECTORY_NAME: &str = "RiseEngineLaunchScoped";

type RiseEngineAppRuntimeTail = Shared<Task<()>>;
type RiseEngineAppRuntimePreparationTask =
    Shared<Task<Result<Arc<RiseAppEngineSession>, RiseAppEngineSharedError>>>;
type RiseEngineAppRuntimeCloseTask = Shared<Task<Result<(), RiseAppEngineSharedError>>>;

struct Preparation {
    generation: u64,
    account: RiseWireAccountContext,
    task: RiseEngineAppRuntimePreparationTask,
    cancellation: RiseAppEngineCancellation,
}

struct CloseFlight {
    identifier: Uuid,
    task: RiseEngineAppRuntimeCloseTask,
}

/// what one preparation task captured when it started
struct PreparationScope {
    this: WeakEntity<RiseEngineAppRuntime>,
    account: RiseWireAccountContext,
    active_generation: u64,
    cancellation: RiseAppEngineCancellation,
}

struct PreparationPlan {
    scope: PreparationScope,
    superseded_preparation: Option<Preparation>,
    preceding_close: Option<CloseFlight>,
    previous_session: Option<Arc<RiseAppEngineSession>>,
    ownership_router: &'static RiseInboundOwnershipRouter,
    bootstrap: RiseAppEngineBootstrapHook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DomainRevalidationAction {
    Initial,
    Reconnect,
    Foreground,
}

/// the gateway replays nothing so frames sent while the
/// socket was down exist only in reads a reconnect does
/// the foreground edge keeps a live socket and just refreshes the badge
#[derive(Clone, Debug, PartialEq, Eq)]
enum DomainRevalidationEdge {
    Connected(RiseWireSocketSession),
    Reconnect,
    Foreground(RiseWireSocketSession),
}

pub struct RiseEngineAppRuntime {
    active_session: Option<Arc<RiseAppEngineSession>>,
    ownership_router: &'static RiseInboundOwnershipRouter,
    bootstrap: RiseAppEngineBootstrapHook,
    generation: u64,
    preparation: Option<Preparation>,
    close_flight: Option<CloseFlight>,
    lifecycle_tail: Option<RiseEngineAppRuntimeTail>,
    revalidation_tail: Option<RiseEngineAppRuntimeTail>,
    resync_task: Option<Task<()>>,
    consecutive_resyncs: u64,
    last_resync_at: Option<Duration>,
    activated_session_id: Option<Uuid>,
    connected_wire_session: Option<RiseWireSocketSession>,
    revalidated_wire_session: Option<RiseWireSocketSession>,
    did_start_initial_revalidation: bool,
    application_is_active: bool,
}

impl RiseEngineAppRuntime {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            active_session: None,
            ownership_router: RiseInboundOwnershipRouter::shared(),
            bootstrap: RiseAppEngineDomainBootstrap::live(),
            generation: 0,
            preparation: None,
            close_flight: None,
            lifecycle_tail: None,
            revalidation_tail: None,
            resync_task: None,
            consecutive_resyncs: 0,
            last_resync_at: None,
            activated_session_id: None,
            connected_wire_session: None,
            revalidated_wire_session: None,
            did_start_initial_revalidation: false,
            application_is_active: true,
        }
    }

    pub fn install(paths: &AppPaths) {
        *APPLICATION_SUPPORT_DIRECTORY.write() = Some(paths.data_dir().to_path_buf());
    }

    /// engine keys held only in memory are gone at next launch
    /// and the engine refuses an existing db without its key
    /// so such a run uses a launch scoped dir emptied at next launch, the real db and key are untouched
    pub fn use_launch_scoped_directory() {
        let mut directory = APPLICATION_SUPPORT_DIRECTORY.write();
        let Some(base_directory) = directory.clone() else {
            return;
        };
        let root = base_directory.join(LAUNCH_SCOPED_ENGINE_DIRECTORY_NAME);
        if FileManager.file_exists(&root) {
            let _ = FileManager.remove_item(&root);
        }
        *directory = Some(root.join(Uuid::new_v4().uuid_string()));
    }

    pub fn active_session(&self) -> Option<&Arc<RiseAppEngineSession>> {
        self.active_session.as_ref()
    }

    pub fn engine(&self) -> Option<Arc<RiseEngine>> {
        self.active_session.as_ref().map(|session| Arc::clone(&session.engine))
    }

    pub fn is_socket_dropped(&self, cx: &mut App) -> bool {
        let manager = SaiWebSocketManager::shared(cx);
        let manager = manager.read(cx);
        match manager.phase() {
            SaiWebSocketManagerPhase::Disconnected | SaiWebSocketManagerPhase::Failed(_) => true,
            SaiWebSocketManagerPhase::Connecting(_) => manager.is_reconnecting_after_failure(),
            SaiWebSocketManagerPhase::Idle | SaiWebSocketManagerPhase::Connected(..) => false,
        }
    }

    /// os token callbacks can come before account activation and the socket handshake
    /// they keep their desired registration until this returns a live account connection
    pub fn ready_connection(
        &mut self,
        account_id: &str,
        cx: &mut Context<Self>,
    ) -> Task<Option<RiseWireSocketSession>> {
        let Some(session) = self.active_session.clone() else {
            return Task::ready(None);
        };
        if session.account.account_id != account_id
            || self.activated_session_id != Some(session.identifier)
        {
            return Task::ready(None);
        }
        let Some(connection) = self.connected_wire_session.clone() else {
            return Task::ready(None);
        };
        let lifecycle_tail = self.lifecycle_tail.clone();
        cx.spawn(async move |this, cx| {
            if let Some(lifecycle_tail) = lifecycle_tail {
                lifecycle_tail.await;
            }
            let engine = Arc::clone(&session.engine);
            let ready = Self::on_account_runtime(async move {
                engine.is_ready_for_calls(false).await
            })
            .await;
            if !matches!(ready, Ok(Ok(true))) {
                return None;
            }
            let still_current = this
                .read_with(cx, |runtime, _| {
                    runtime.is_active_session(&session)
                        && runtime.connected_wire_session.as_ref() == Some(&connection)
                })
                .unwrap_or(false);
            if !still_current
                || !SaiAccountCacheScope::is_current_account_epoch(
                    Some(&session.account.account_id),
                    Some(&session.account.epoch),
                )
            {
                return None;
            }
            Some(connection)
        })
    }

    pub fn prepare_active_account(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Arc<RiseEngine>>> {
        let account_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let (Some(account_id), Some(account_epoch)) = (
            Self::normalized(account_id.as_deref()),
            Self::normalized(account_epoch.as_deref()),
        ) else {
            return Task::ready(Err(RiseEngineAppRuntimeError::MissingActiveAccount.into()));
        };
        self.prepare(account_id, account_epoch, cx)
    }

    /// builds a whole account session on the side, domain bootstrap finishes before the engine starts
    /// then session and push ownership install in one main thread turn
    pub fn prepare(
        &mut self,
        account_id: String,
        account_epoch: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Arc<RiseEngine>>> {
        let account = RiseWireAccountContext::new(account_id, account_epoch);
        if let Some(preparation) = &self.preparation
            && preparation.account == account
        {
            let task = preparation.task.clone();
            return cx.spawn(async move |_, _| Self::engine_of(task.await));
        }
        if self.preparation.is_none()
            && self.close_flight.is_none()
            && let Some(active_session) = &self.active_session
            && active_session.account == account
        {
            active_session.activate_inbound_ownership();
            return Task::ready(Ok(Arc::clone(&active_session.engine)));
        }

        self.generation = self.generation.wrapping_add(1);
        let active_generation = self.generation;
        let superseded_preparation = self.preparation.take();
        let preceding_close = self.close_flight.take();
        let previous_session = self.active_session.clone();
        if let Some(superseded_preparation) = &superseded_preparation {
            superseded_preparation.cancellation.cancel();
        }
        if let Some(previous_session) = &previous_session {
            previous_session.deactivate_inbound_ownership();
        }

        let cancellation = RiseAppEngineCancellation::new();
        let plan = PreparationPlan {
            scope: PreparationScope {
                this: cx.weak_entity(),
                account: account.clone(),
                active_generation,
                cancellation: cancellation.clone(),
            },
            superseded_preparation,
            preceding_close,
            previous_session,
            ownership_router: self.ownership_router,
            bootstrap: Arc::clone(&self.bootstrap),
        };
        let task = cx
            .spawn(async move |_, cx| {
                Self::run_preparation(plan, cx).await.map_err(RiseAppEngineSharedError::new)
            })
            .shared();
        self.preparation = Some(Preparation {
            generation: active_generation,
            account,
            task: task.clone(),
            cancellation,
        });

        cx.spawn(async move |this, cx| {
            let opened = task.await;
            let _ = this.update(cx, |runtime, _| {
                if runtime.preparation.as_ref().map(|preparation| preparation.generation)
                    == Some(active_generation)
                {
                    runtime.preparation = None;
                }
            });
            Self::engine_of(opened)
        })
    }

    fn engine_of(
        opened: Result<Arc<RiseAppEngineSession>, RiseAppEngineSharedError>,
    ) -> anyhow::Result<Arc<RiseEngine>> {
        opened.map(|opened| Arc::clone(&opened.engine)).map_err(anyhow::Error::new)
    }

    async fn run_preparation(
        plan: PreparationPlan,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Arc<RiseAppEngineSession>> {
        let PreparationPlan {
            scope,
            superseded_preparation,
            preceding_close,
            previous_session,
            ownership_router,
            bootstrap,
        } = plan;
        if let Some(preceding_close) = preceding_close {
            preceding_close.task.await.map_err(anyhow::Error::new)?;
        }
        if let Some(superseded_preparation) = superseded_preparation {
            let _ = superseded_preparation.task.await;
        }
        scope.cancellation.check_cancellation()?;

        let previous_was_closed_before_open = match &previous_session {
            Some(previous_session)
                if previous_session.account.account_id == scope.account.account_id =>
            {
                previous_session.close().await?;
                let _ = scope.this.update(cx, |runtime, _| {
                    if runtime.generation == scope.active_generation
                        && runtime.is_active_session(previous_session)
                    {
                        runtime.active_session = None;
                    }
                });
                true
            }
            _ => false,
        };
        scope.cancellation.check_cancellation()?;

        let (sender, http_sender) =
            cx.update(|cx| (RiseWireSaiSocketAdapter::new(cx), RiseWireHttpAdapter::new(cx)));
        let account = scope.account.clone();
        let cancellation = scope.cancellation.clone();
        RiseColdOpenTrace::mark(|| "prepare start".to_owned());
        let opening_task = RiseAccountRuntime::current().spawn(Self::open_session(
            account,
            sender,
            http_sender,
            ownership_router,
            bootstrap,
            cancellation,
        ));
        let opened = match opening_task.await {
            Ok(opened) => opened?,
            Err(join_error) => return Err(anyhow::Error::new(join_error)),
        };

        RiseColdOpenTrace::mark(|| "session returned to main".to_owned());
        let mut orphaned_session: Option<Arc<RiseAppEngineSession>> = None;
        let activated = Self::activate_opened(
            &scope,
            &opened,
            previous_was_closed_before_open,
            &mut orphaned_session,
            cx,
        )
        .await;
        match activated {
            Ok(()) => Ok(opened),
            Err(error) => {
                let _ = scope.this.update(cx, |runtime, _| {
                    if runtime.generation == scope.active_generation
                        && runtime.is_active_session(&opened)
                    {
                        runtime.active_session = None;
                    }
                });
                let _ = opened.close().await;
                if let Some(orphaned_session) = orphaned_session {
                    let is_active = scope
                        .this
                        .read_with(cx, |runtime, _| runtime.is_active_session(&orphaned_session))
                        .unwrap_or(false);
                    if !is_active {
                        let _ = orphaned_session.close().await;
                    }
                }
                Err(error)
            }
        }
    }

    async fn open_session(
        account: RiseWireAccountContext,
        sender: RiseWireSaiSocketAdapter,
        http_sender: RiseWireHttpAdapter,
        ownership_router: &'static RiseInboundOwnershipRouter,
        bootstrap: RiseAppEngineBootstrapHook,
        cancellation: RiseAppEngineCancellation,
    ) -> anyhow::Result<Arc<RiseAppEngineSession>> {
        let base_directory = Self::application_support_directory(true)?;
        RiseAppEngineSession::open(
            RiseEngineConfiguration::new(
                base_directory,
                account,
                Arc::new(sender),
                RiseEngineConfigurationArgs {
                    http_sender: Some(Arc::new(http_sender)),
                    wire_configuration: RiseWireConfiguration::canonical(),
                    ..RiseEngineConfigurationArgs::default()
                },
            ),
            ownership_router,
            bootstrap,
            cancellation,
        )
        .await
    }

    async fn activate_opened(
        scope: &PreparationScope,
        opened: &Arc<RiseAppEngineSession>,
        previous_was_closed_before_open: bool,
        orphaned_session: &mut Option<Arc<RiseAppEngineSession>>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        scope.cancellation.check_cancellation()?;
        let installed = scope
            .this
            .update(cx, |runtime, _| {
                if !runtime.is_current_preparation(scope.active_generation, &scope.account) {
                    return false;
                }
                if let Some(replaced_session) = &runtime.active_session
                    && !Arc::ptr_eq(replaced_session, opened)
                    && !previous_was_closed_before_open
                {
                    *orphaned_session = Some(Arc::clone(replaced_session));
                }
                runtime.active_session = Some(Arc::clone(opened));
                runtime.activated_session_id = None;
                runtime.connected_wire_session = None;
                runtime.revalidated_wire_session = None;
                runtime.did_start_initial_revalidation = false;
                true
            })
            .unwrap_or(false);
        if !installed {
            return Err(CancellationError.into());
        }

        RiseColdOpenTrace::mark(|| "session installed".to_owned());
        let main_hop = cx.update(RiseEngineMainHop::shared);
        let runtime = scope.this.clone();
        let weak_opened = Arc::downgrade(opened);
        opened
            .engine
            .on_resync_required(move |_| {
                let main_hop = main_hop.clone();
                let runtime = runtime.clone();
                let weak_opened = weak_opened.clone();
                async move {
                    let Some(opened) = weak_opened.upgrade() else {
                        return;
                    };
                    let _ = main_hop
                        .run(move |cx: AsyncApp| {
                            async move {
                                cx.update(|cx| {
                                    let _ = runtime.update(cx, |runtime, cx| {
                                        runtime.engine_requires_resync(&opened, cx)
                                    });
                                });
                                Ok::<(), anyhow::Error>(())
                            }
                            .boxed_local()
                        })
                        .await;
                }
            })
            .await?;

        RiseColdOpenTrace::mark(|| "session opened".to_owned());
        let manager = cx.update(SaiWebSocketManager::shared);
        let socket_sync = cx.foreground_executor().spawn({
            let cx = cx.clone();
            async move {
                SaiWebSocketManager::rise_wire_synchronize_attached_engine(manager, cx).await;
                RiseColdOpenTrace::mark(|| "socket lifecycle synced".to_owned());
            }
        });
        let activating = Arc::clone(opened);
        let activation = Self::on_account_runtime(async move { activating.activate_domain().await })
            .await
            .and_then(|activated| activated);
        if let Err(error) = activation {
            socket_sync.await;
            return Err(error);
        }
        RiseColdOpenTrace::mark(|| "domains active".to_owned());
        socket_sync.await;
        scope.cancellation.check_cancellation()?;
        Self::ensure_current_preparation(scope, opened, cx)?;

        scope.this.update(cx, |runtime, cx| {
            opened.activate_inbound_ownership();
            runtime.activated_session_id = Some(opened.identifier);
            if let Some(wire_session) = runtime.connected_wire_session.clone() {
                let lifecycle = runtime.lifecycle_tail.clone();
                runtime.schedule_domain_revalidation(
                    opened,
                    lifecycle,
                    DomainRevalidationEdge::Connected(wire_session),
                    cx,
                );
            }
        })?;

        if let Some(orphaned) = orphaned_session.take() {
            let _ = orphaned.close().await;
        }

        scope.cancellation.check_cancellation()?;
        Self::ensure_current_preparation(scope, opened, cx)?;
        Ok(())
    }

    fn ensure_current_preparation(
        scope: &PreparationScope,
        opened: &Arc<RiseAppEngineSession>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let is_current = scope
            .this
            .read_with(cx, |runtime, _| {
                runtime.is_current_preparation(scope.active_generation, &scope.account)
                    && runtime.is_active_session(opened)
            })
            .unwrap_or(false);
        if !is_current {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    pub fn close_current_engine(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        if self.preparation.is_none()
            && self.active_session.is_none()
            && let Some(close_flight) = &self.close_flight
        {
            let task = close_flight.task.clone();
            return cx.spawn(async move |_, _| task.await.map_err(anyhow::Error::new));
        }
        if self.preparation.is_none() && self.active_session.is_none() {
            return Task::ready(Ok(()));
        }

        self.generation = self.generation.wrapping_add(1);
        let superseded_preparation = self.preparation.take();
        let preceding_close = self.close_flight.take();
        let previous_session = self.active_session.take();
        if let Some(previous_session) = &previous_session {
            previous_session.deactivate_inbound_ownership();
        }
        if let Some(superseded_preparation) = &superseded_preparation {
            superseded_preparation.cancellation.cancel();
        }

        let identifier = Uuid::new_v4();
        let task = cx
            .spawn(async move |_, _| {
                Self::run_close(superseded_preparation, preceding_close, previous_session)
                    .await
                    .map_err(RiseAppEngineSharedError::new)
            })
            .shared();
        self.close_flight = Some(CloseFlight { identifier, task: task.clone() });
        cx.spawn(async move |this, cx| {
            let closed = task.await;
            let _ = this.update(cx, |runtime, _| {
                if runtime.close_flight.as_ref().map(|close_flight| close_flight.identifier)
                    == Some(identifier)
                {
                    runtime.close_flight = None;
                }
            });
            closed.map_err(anyhow::Error::new)
        })
    }

    async fn run_close(
        superseded_preparation: Option<Preparation>,
        preceding_close: Option<CloseFlight>,
        previous_session: Option<Arc<RiseAppEngineSession>>,
    ) -> anyhow::Result<()> {
        if let Some(preceding_close) = preceding_close {
            preceding_close.task.await.map_err(anyhow::Error::new)?;
        }
        if let Some(superseded_preparation) = superseded_preparation {
            let _ = superseded_preparation.task.await;
        }
        if let Some(previous_session) = previous_session {
            previous_session.close().await?;
        }
        Ok(())
    }

    pub fn remove_persisted_account(
        &mut self,
        account_id: &str,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let normalized_account_id = trimming_whitespaces_and_newlines(account_id).to_owned();
        if normalized_account_id.is_empty() {
            return Task::ready(Ok(()));
        }
        let is_engine_account = self
            .active_session
            .as_ref()
            .is_some_and(|session| session.account.account_id == normalized_account_id)
            || self
                .preparation
                .as_ref()
                .is_some_and(|preparation| preparation.account.account_id == normalized_account_id);
        let closing = is_engine_account.then(|| self.close_current_engine(cx));
        cx.spawn(async move |_, _| Self::remove_local_account_data(closing, normalized_account_id).await)
    }

    async fn remove_local_account_data(
        closing: Option<Task<anyhow::Result<()>>>,
        account_id: String,
    ) -> anyhow::Result<()> {
        if let Some(closing) = closing {
            closing.await?;
        }
        Self::on_account_runtime(Self::remove_engine_account_data(account_id)).await?
    }

    async fn remove_engine_account_data(account_id: String) -> anyhow::Result<()> {
        let base_directory = Self::application_support_directory(true)?;
        RiseEngine::remove_local_account_data(
            &base_directory,
            &account_id,
            Arc::new(RiseKeychainKeyStore::shared()),
        )
        .await
    }

    pub fn persisted_account_directory_url(account_id: &str) -> Option<PathBuf> {
        let normalized_account_id = trimming_whitespaces_and_newlines(account_id);
        if normalized_account_id.is_empty() {
            return None;
        }
        let base_directory = Self::application_support_directory(false).ok()?;
        Some(RiseEngine::local_account_directory_url(&base_directory, normalized_account_id))
    }

    /// the bound data directory, for features that keep their own files beside the engine
    pub fn application_support_directory(create: bool) -> anyhow::Result<PathBuf> {
        let Some(directory) = APPLICATION_SUPPORT_DIRECTORY.read().clone() else {
            return Err(CocoaError::new(CocoaErrorCode::FileNoSuchFile).into());
        };
        if create {
            FileManager.create_directory(&directory, true)?;
        }
        Ok(directory)
    }

    pub fn socket_session_started(
        &mut self,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        transport_epoch: u64,
        cx: &mut Context<Self>,
    ) -> Task<Option<RiseWireSocketSession>> {
        let Some(wire_session) =
            self.wire_session(account_user_id.as_deref(), account_epoch.as_deref(), transport_epoch)
        else {
            return Task::ready(None);
        };
        let Some(active_session) = &self.active_session else {
            return Task::ready(None);
        };
        self.connected_wire_session = None;
        let engine = Arc::clone(&active_session.engine);
        let started = wire_session.clone();
        let task = self.append_lifecycle_operation(
            async move {
                let _ = engine.socket_session_started(started).await;
            },
            cx,
        );
        cx.spawn(async move |_, _| {
            task.await;
            Some(wire_session)
        })
    }

    /// first live socket starts ownership refresh and only later sockets are reconnects
    pub fn socket_connected(
        &mut self,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        transport_epoch: u64,
        inherits_live_socket: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(wire_session) =
            self.wire_session(account_user_id.as_deref(), account_epoch.as_deref(), transport_epoch)
        else {
            return;
        };
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let engine = Arc::clone(&session.engine);
        self.connected_wire_session = Some(wire_session.clone());
        let connected = wire_session.clone();
        let lifecycle = self.append_lifecycle_operation(
            async move {
                let _ = engine.socket_connected(connected).await;
            },
            cx,
        );
        if inherits_live_socket || self.activated_session_id != Some(session.identifier) {
            return;
        }
        self.schedule_domain_revalidation(
            &session,
            Some(lifecycle),
            DomainRevalidationEdge::Connected(wire_session),
            cx,
        );
    }

    pub fn socket_disconnected(
        &mut self,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        transport_epoch: u64,
        reason: String,
        cx: &mut Context<Self>,
    ) {
        let Some(wire_session) =
            self.wire_session(account_user_id.as_deref(), account_epoch.as_deref(), transport_epoch)
        else {
            return;
        };
        let Some(active_session) = &self.active_session else {
            return;
        };
        if self.connected_wire_session.as_ref() == Some(&wire_session) {
            self.connected_wire_session = None;
        }
        let engine = Arc::clone(&active_session.engine);
        let _ = self.append_lifecycle_operation(
            async move {
                let _ = engine.socket_disconnected(wire_session, reason).await;
            },
            cx,
        );
    }

    pub fn socket_handshake_failed(&mut self, cx: &mut Context<Self>) {
        let Some(active_session) = &self.active_session else {
            return;
        };
        let engine = Arc::clone(&active_session.engine);
        let _ = self.append_lifecycle_operation(
            async move {
                let _ = engine.socket_handshake_failed().await;
            },
            cx,
        );
    }

    pub fn set_application_active(
        &mut self,
        active: bool,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        transport_epoch: u64,
        cx: &mut Context<Self>,
    ) {
        let became_active = active && !self.application_is_active;
        self.application_is_active = active;
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let engine = Arc::clone(&session.engine);
        let current_wire_session =
            self.wire_session(account_user_id.as_deref(), account_epoch.as_deref(), transport_epoch);
        let lifecycle = self.append_lifecycle_operation(
            async move {
                let _ = engine.set_application_active(active, current_wire_session).await;
            },
            cx,
        );
        if became_active && let Some(connected_wire_session) = self.connected_wire_session.clone() {
            let edge = if self.revalidated_wire_session.as_ref() == Some(&connected_wire_session) {
                DomainRevalidationEdge::Foreground(connected_wire_session)
            } else {
                DomainRevalidationEdge::Connected(connected_wire_session)
            };
            self.schedule_domain_revalidation(&session, Some(lifecycle), edge, cx);
        }
    }

    /// domains refetch over the socket here so never share a queue with what accept waits on
    /// else a revalidation blocks its own reply and socket goes deaf till the 30 s timeout
    fn schedule_domain_revalidation(
        &mut self,
        session: &Arc<RiseAppEngineSession>,
        lifecycle: Option<RiseEngineAppRuntimeTail>,
        edge: DomainRevalidationEdge,
        cx: &mut Context<Self>,
    ) {
        let session = Arc::clone(session);
        let this = cx.weak_entity();
        let mut async_cx = cx.to_async();
        let operation = async move {
            if let Some(lifecycle) = lifecycle {
                lifecycle.await;
            }
            let engine = Arc::clone(&session.engine);
            let ready =
                Self::on_account_runtime(async move { engine.is_ready_for_calls(false).await })
                    .await;
            if !matches!(ready, Ok(Ok(true))) {
                return;
            }
            let action = this
                .update(&mut async_cx, |runtime, _| {
                    runtime.claim_domain_revalidation(&session, &edge)
                })
                .ok()
                .flatten();
            let Some(action) = action else {
                return;
            };
            RiseAppEvents::default_center().post(RISE_ENGINE_CONNECTION_READY);
            let _ = Self::on_account_runtime(async move {
                match action {
                    DomainRevalidationAction::Initial => {
                        session.did_activate_inbound_ownership().await;
                    }
                    DomainRevalidationAction::Reconnect => session.socket_connected().await,
                    DomainRevalidationAction::Foreground => {
                        session.application_did_become_active().await;
                    }
                }
            })
            .await;
        }
        .boxed_local();
        self.append_revalidation_operation(operation, cx);
    }

    fn claim_domain_revalidation(
        &mut self,
        session: &Arc<RiseAppEngineSession>,
        edge: &DomainRevalidationEdge,
    ) -> Option<DomainRevalidationAction> {
        if !self.is_active_session(session)
            || self.activated_session_id != Some(session.identifier)
            || !self.application_is_active
        {
            return None;
        }
        let connected_wire_session = self.connected_wire_session.as_ref()?;
        match edge {
            DomainRevalidationEdge::Connected(expected) => {
                if connected_wire_session != expected
                    || self.revalidated_wire_session.as_ref() == Some(expected)
                {
                    return None;
                }
                self.revalidated_wire_session = Some(expected.clone());
                if !self.did_start_initial_revalidation {
                    self.did_start_initial_revalidation = true;
                    return Some(DomainRevalidationAction::Initial);
                }
                Some(DomainRevalidationAction::Reconnect)
            }
            DomainRevalidationEdge::Foreground(expected) => {
                if !self.did_start_initial_revalidation
                    || connected_wire_session != expected
                    || self.revalidated_wire_session.as_ref() != Some(expected)
                {
                    return None;
                }
                Some(DomainRevalidationAction::Foreground)
            }
            DomainRevalidationEdge::Reconnect => self
                .did_start_initial_revalidation
                .then_some(DomainRevalidationAction::Reconnect),
        }
    }

    pub fn owned_push_was_ignored(&mut self, cx: &mut Context<Self>) {
        let Some(active_session) = self.active_session.clone() else {
            return;
        };
        self.engine_requires_resync(&active_session, cx);
    }

    fn engine_requires_resync(
        &mut self,
        session: &Arc<RiseAppEngineSession>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_active_session(session) || self.resync_task.is_some() {
            return;
        }
        let now = RiseClock::monotonic();
        if let Some(last_resync_at) = self.last_resync_at
            && now.saturating_sub(last_resync_at)
                > Duration::from_nanos(SaiWebSocketConfiguration::RECONNECT_MAX_DELAY_NANOSECONDS)
        {
            self.consecutive_resyncs = 0;
        }
        let delay = if self.consecutive_resyncs == 0 {
            0
        } else {
            SaiWebSocketConfiguration::reconnect_delay_nanoseconds(self.consecutive_resyncs)
        };
        let weak_session = Arc::downgrade(session);
        self.resync_task = Some(cx.spawn(async move |this, cx| {
            if delay > 0 {
                cx.background_executor().timer(Duration::from_nanos(delay)).await;
            }
            let _ = this.update(cx, |runtime, cx| {
                runtime.resync_task = None;
                let Some(session) = weak_session.upgrade() else {
                    return;
                };
                if !runtime.is_active_session(&session) {
                    return;
                }
                runtime.consecutive_resyncs += 1;
                runtime.last_resync_at = Some(RiseClock::monotonic());
                let lifecycle = runtime.lifecycle_tail.clone();
                runtime.schedule_domain_revalidation(
                    &session,
                    lifecycle,
                    DomainRevalidationEdge::Reconnect,
                    cx,
                );
            });
        }));
    }

    pub fn accept(
        &mut self,
        data: Vec<u8>,
        wire_session: Option<RiseWireSocketSession>,
        cx: &mut Context<Self>,
    ) -> Task<RiseWireInboundDisposition> {
        let Some(wire_session) = wire_session else {
            return Task::ready(RiseWireInboundDisposition::Ignored);
        };
        let Some(active_session) = self.active_session.clone() else {
            return Task::ready(RiseWireInboundDisposition::Ignored);
        };
        if active_session.account != wire_session.account {
            return Task::ready(RiseWireInboundDisposition::Ignored);
        }
        let lifecycle_tail = self.lifecycle_tail.clone();
        cx.spawn(async move |this, cx| {
            if let Some(lifecycle_tail) = lifecycle_tail {
                lifecycle_tail.await;
            }
            let is_active = this
                .read_with(cx, |runtime, _| runtime.is_active_session(&active_session))
                .unwrap_or(false);
            if !is_active {
                return RiseWireInboundDisposition::Ignored;
            }
            let engine = Arc::clone(&active_session.engine);
            let accepted =
                Self::on_account_runtime(async move { engine.accept(&data, &wire_session).await })
                    .await;
            match accepted {
                Ok(Ok(disposition)) => disposition,
                Ok(Err(_)) | Err(_) => RiseWireInboundDisposition::Ignored,
            }
        })
    }

    pub fn accept_account_user_id(
        &mut self,
        data: Vec<u8>,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        transport_epoch: u64,
        cx: &mut Context<Self>,
    ) -> Task<RiseWireInboundDisposition> {
        let wire_session =
            self.wire_session(account_user_id.as_deref(), account_epoch.as_deref(), transport_epoch);
        self.accept(data, wire_session, cx)
    }

    pub fn wait_for_socket_lifecycle_synchronization(&self, cx: &mut Context<Self>) -> Task<()> {
        let lifecycle_tail = self.lifecycle_tail.clone();
        cx.spawn(async move |_, _| {
            if let Some(lifecycle_tail) = lifecycle_tail {
                lifecycle_tail.await;
            }
        })
    }

    fn wire_session(
        &self,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        transport_epoch: u64,
    ) -> Option<RiseWireSocketSession> {
        let account_user_id = Self::normalized(account_user_id)?;
        let account_epoch = Self::normalized(account_epoch)?;
        let active_session = self.active_session.as_ref()?;
        if active_session.account.account_id != account_user_id
            || active_session.account.epoch != account_epoch
        {
            return None;
        }
        Some(RiseWireSocketSession::new(active_session.account.clone(), transport_epoch))
    }

    fn append_lifecycle_operation(
        &mut self,
        operation: impl Future<Output = ()> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> RiseEngineAppRuntimeTail {
        let previous = self.lifecycle_tail.clone();
        let task = cx
            .spawn(async move |_, _| {
                if let Some(previous) = previous {
                    previous.await;
                }
                let _ = Self::on_account_runtime(operation).await;
            })
            .shared();
        self.lifecycle_tail = Some(task.clone());
        task
    }

    fn append_revalidation_operation(
        &mut self,
        operation: LocalBoxFuture<'static, ()>,
        cx: &mut Context<Self>,
    ) {
        let previous = self.revalidation_tail.clone();
        let task = cx
            .spawn(async move |_, _| {
                if let Some(previous) = previous {
                    previous.await;
                }
                operation.await;
            })
            .shared();
        self.revalidation_tail = Some(task);
    }

    /// moves engine and domain work off the main thread
    async fn on_account_runtime<T>(
        operation: impl Future<Output = T> + Send + 'static,
    ) -> anyhow::Result<T>
    where
        T: Send + 'static,
    {
        RiseAccountRuntime::current().spawn(operation).await.map_err(anyhow::Error::new)
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        (!value.is_empty()).then(|| value.to_owned())
    }

    fn is_active_session(&self, session: &Arc<RiseAppEngineSession>) -> bool {
        self.active_session.as_ref().is_some_and(|active_session| Arc::ptr_eq(active_session, session))
    }

    fn is_current_preparation(&self, generation: u64, account: &RiseWireAccountContext) -> bool {
        self.generation == generation
            && self.preparation.as_ref().map(|preparation| preparation.generation)
                == Some(generation)
            && self.preparation.as_ref().map(|preparation| &preparation.account) == Some(account)
    }
}
