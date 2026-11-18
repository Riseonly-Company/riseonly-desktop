use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::{BoxFuture, Shared};
use futures::{FutureExt, StreamExt};
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::rise_key::rtc::ANSWERED_ELSEWHERE;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine};
use ::rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use rise_rtc::call_room_id;
use tokio::task::JoinHandle;

use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingHeartbeatExt};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::messages::rise_call_history_message::RiseCallHistoryRecord;
use crate::modules::rtc::engine::core::rise_rtc_call_crypto::{RiseRTCCallCrypto, RiseRTCCallCryptoIdentity};
use crate::modules::rtc::engine::rise_rtc_call_system_hooks::RiseRTCCallSystemHooks;
use crate::modules::rtc::engine::core::rise_rtc_listener_volume_repository::RiseRTCListenerVolumeRepository;
use crate::modules::rtc::engine::core::rise_rtcrpc::{RiseRTCEndpoint, RiseRTCEndpoints};
use crate::modules::rtc::engine::core::rise_rtc_store_layout::{
    RiseRTCCallRepositorySnapshot, RiseRTCRoomRepositorySnapshot,
};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCAnswerOutcome, RiseRTCCallMediaMode, RiseRTCCallState, RiseRTCCallStateExt,
    RiseRTCCancelReconciliationAction, RiseRTCCancelReconciliationPolicy,
    RiseRTCCommittedMediaFailureAction, RiseRTCCommittedMediaFailurePolicy,
    RiseRTCCameraPosition, RiseRTCCommittedCallMediaKind, RiseRTCConnectionState,
    RiseRTCDesiredMediaState, RiseRTCDirectCall, RiseRTCEvictionReason, RiseRTCFlexibleWireEnum,
    RiseRTCJoinCredentials, RiseRTCMediaDisconnectCode, RiseRTCMediaDriving, RiseRTCMediaEvent, RiseRTCMediaIntent, RiseRTCMediaSessionKey,
    RiseRTCModerationAction, RiseRTCParticipant, RiseRTCParticipantRole, RiseRTCRoomEviction,
    RiseRTCRoomMode, RiseRTCScreenShareQuality, RiseRTCStateError, RiseRTCUserProfile,
};
use crate::modules::rtc::engine::rise_rtc_connection_wait_policy::RiseRTCConnectionWaitPolicy;
use crate::modules::rtc::engine::rise_rtc_listener_volume_presentation::{
    RiseRTCListenerVolume, RiseRTCListenerVolumePresentation,
};
use crate::modules::rtc::engine::rise_rtc_media_boundary::RiseRTCMediaRuntime;
use crate::modules::rtc::engine::rise_rtc_presentation::RiseRTCPresentation;
use crate::modules::rtc::engine::rise_rtc_repository::RiseRTCRepository;
use crate::modules::rtc::engine::rise_rtc_system_domain_adapter::RiseRTCDomainSystemAdapter;
use crate::modules::rtc::shared::rise_rtc_device_refusal::RiseRTCDeviceRefusal;
use crate::modules::rtc::system::rise_rtc_system_bridges::{
    RiseRTCSystemDomainBridge, RiseRTCSystemDomainClient,
};
use crate::modules::rtc::system::rise_rtc_system_models::{
    RiseRTCCallActivationGate, RiseRTCCallSystemEventMapper, RiseRTCSystemIncomingEnvelope,
    RiseRTCSystemVoIPEvent,
};
use crate::modules::user::engine::rise_user_domain::RiseUserDomainRegistry;
use crate::modules::user::shared::contact_device_book::RiseInstallationIdentity;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCHangupAction {
    End,
    Cancel,
    Decline,
}

/// provider of the media driver for the rtc domain
pub type RiseRTCDomainMediaDriverProvider =
    Arc<dyn Fn() -> BoxFuture<'static, Arc<dyn RiseRTCMediaDriving>> + Send + Sync>;

/// call history sink installed by the session composition
/// rtc doesnt know chat types so the record is neutral
pub type RiseRTCCallHistorySink =
    Arc<dyn Fn(RiseCallHistoryRecord) -> BoxFuture<'static, ()> + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct RiseRTCDomainPendingActivation {
    room_id: String,
    generation: u64,
}

type RiseRTCDomainJoinOutcome = Shared<BoxFuture<'static, Result<(), Arc<anyhow::Error>>>>;

struct RiseRTCDomainInFlightJoin {
    room_id: String,
    token: Uuid,
    task: RiseRTCDomainJoinOutcome,
    cancelled: Arc<AtomicBool>,
}

struct RiseRTCDomainDeparture {
    token: Uuid,
    task: Shared<BoxFuture<'static, ()>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RiseRTCDomainAppliedServerModeration {
    session: RiseRTCMediaSessionKey,
    silenced: bool,
    deafened: bool,
    camera_blocked: bool,
    screen_share_blocked: bool,
}

#[derive(Clone)]
struct RiseRTCDomainCommittedCallMediaActivation {
    task_id: Uuid,
    call: RiseRTCDirectCall,
    credentials: RiseRTCJoinCredentials,
    desired_media: RiseRTCDesiredMediaState,
    activation_generation: u64,
    kind: RiseRTCCommittedCallMediaKind,
    recovering_from: Option<String>,
    restores_deafen: bool,
}

tokio::task_local! {
    static RISE_RTC_TASK_CANCELLED: Arc<AtomicBool>;
}

/// true if the task this domain spawned was cancelled, false outside one
fn task_is_cancelled() -> bool {
    RISE_RTC_TASK_CANCELLED
        .try_with(|cancelled| cancelled.load(Ordering::Acquire))
        .unwrap_or(false)
}

/// stored task of the actor, cancel flags it and stops it at its next suspension
struct RiseRTCDomainTask {
    cancelled: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl RiseRTCDomainTask {
    fn spawn(work: impl Future<Output = ()> + Send + 'static) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let handle =
            RiseAccountRuntime::current().spawn(RISE_RTC_TASK_CANCELLED.scope(cancelled.clone(), work));
        Self { cancelled, handle }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.handle.abort();
    }

    async fn value(self) {
        let _ = self.handle.await;
    }
}

struct RiseRTCDomainCommittedCallMediaTask {
    task_id: Uuid,
    task: RiseRTCDomainTask,
}

struct RiseRTCDomainState {
    media_driver: Option<Arc<dyn RiseRTCMediaDriving>>,
    active_credentials: Option<RiseRTCJoinCredentials>,
    pending_activation: Option<RiseRTCDomainPendingActivation>,
    activation_generation: u64,
    in_flight_join: Option<RiseRTCDomainInFlightJoin>,
    departures: HashMap<String, RiseRTCDomainDeparture>,
    media_connection_state: RiseRTCConnectionState,
    microphone_mutation_generation: u64,
    camera_mutation_generation: u64,
    screen_share_mutation_generation: u64,
    microphone_mutation_lane_busy: bool,
    camera_mutation_lane_busy: bool,
    screen_share_mutation_lane_busy: bool,
    microphone_mutation_waiters: Vec<oneshot::Sender<()>>,
    camera_mutation_waiters: Vec<oneshot::Sender<()>>,
    screen_share_mutation_waiters: Vec<oneshot::Sender<()>>,
    committed_call_media_activations: HashMap<String, RiseRTCDomainCommittedCallMediaActivation>,
    committed_call_media_tasks: HashMap<String, RiseRTCDomainCommittedCallMediaTask>,
    reconnect_waiters: HashMap<Uuid, oneshot::Sender<()>>,
    system_domain_adapter: Option<Arc<RiseRTCDomainSystemAdapter>>,
    call_identities: HashMap<String, RiseRTCCallCryptoIdentity>,
    call_identity_order: Vec<String>,
    applied_call_keys: HashSet<String>,
    peer_half_snapshot_calls: HashSet<String>,
    peer_half_reads_in_flight: HashSet<String>,
    call_history_sink: Option<RiseRTCCallHistorySink>,
    ingested_call_history_ids: HashSet<String>,
    ingested_call_history_order: Vec<String>,
    latest_calls: Option<RiseRTCCallRepositorySnapshot>,
    latest_room_snapshot: Option<RiseRTCRoomRepositorySnapshot>,
    applied_server_moderation: Option<RiseRTCDomainAppliedServerModeration>,
    pushed_listener_gains: HashMap<String, f64>,
    pushed_listener_session: Option<RiseRTCMediaSessionKey>,
    resolved_listener_roster: HashSet<String>,
    resolved_listener_revision: u64,
    listener_blocks_session: Option<RiseRTCMediaSessionKey>,
    listener_blocks_refresh_task: Option<RiseRTCDomainTask>,
    listener_intent_sequence: u64,
    call_observation_task: Option<RiseRTCDomainTask>,
    room_observation_task: Option<RiseRTCDomainTask>,
    media_event_task: Option<RiseRTCDomainTask>,
    heartbeat_task: Option<RiseRTCDomainTask>,
    reconciliation_task: Option<RiseRTCDomainTask>,
    active_room_id: Option<String>,
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
}

impl RiseRTCDomainState {
    fn advance_activation_generation(&mut self) -> u64 {
        self.activation_generation = self.activation_generation.wrapping_add(1);
        if self.activation_generation == 0 {
            self.activation_generation = 1;
        }
        self.activation_generation
    }

    fn advance_microphone_mutation_generation(&mut self) -> u64 {
        self.microphone_mutation_generation = self.microphone_mutation_generation.wrapping_add(1);
        if self.microphone_mutation_generation == 0 {
            self.microphone_mutation_generation = 1;
        }
        self.microphone_mutation_generation
    }

    fn advance_camera_mutation_generation(&mut self) -> u64 {
        self.camera_mutation_generation = self.camera_mutation_generation.wrapping_add(1);
        if self.camera_mutation_generation == 0 {
            self.camera_mutation_generation = 1;
        }
        self.camera_mutation_generation
    }

    fn advance_screen_share_mutation_generation(&mut self) -> u64 {
        self.screen_share_mutation_generation =
            self.screen_share_mutation_generation.wrapping_add(1);
        if self.screen_share_mutation_generation == 0 {
            self.screen_share_mutation_generation = 1;
        }
        self.screen_share_mutation_generation
    }

    fn active_session_key(&self) -> Option<RiseRTCMediaSessionKey> {
        self.active_credentials.as_ref().map(RiseRTCJoinCredentials::session_key)
    }

    fn clear_ephemeral_session(&mut self) {
        self.active_credentials = None;
        self.active_room_id = None;
        self.applied_server_moderation = None;
        self.pushed_listener_gains = HashMap::new();
        self.pushed_listener_session = None;
        self.resolved_listener_roster = HashSet::new();
        self.resolved_listener_revision = 0;
        if let Some(task) = self.listener_blocks_refresh_task.take() {
            task.cancel();
        }
        self.listener_blocks_session = None;
        self.advance_microphone_mutation_generation();
        self.advance_camera_mutation_generation();
        self.advance_screen_share_mutation_generation();
        for task in [
            self.heartbeat_task.take(),
            self.room_observation_task.take(),
            self.reconciliation_task.take(),
        ]
        .into_iter()
        .flatten()
        {
            task.cancel();
        }
    }
}

/// releases a mutation lane when the mutation ends, even if it errored or got cancelled
struct RiseRTCDomainLaneRelease<'a> {
    domain: &'a RiseRTCDomain,
    release: fn(&RiseRTCDomain),
}

impl Drop for RiseRTCDomainLaneRelease<'_> {
    fn drop(&mut self) {
        (self.release)(self.domain);
    }
}

pub struct RiseRTCDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub repository: Arc<RiseRTCRepository>,
    pub listener_volumes: Arc<RiseRTCListenerVolumeRepository>,
    media_driver_provider: RiseRTCDomainMediaDriverProvider,
    system_bridge_owner_id: Uuid,
    this: Weak<RiseRTCDomain>,
    state: RiseIsolated<RiseRTCDomainState>,
}

impl RiseRTCDomain {
    const CALL_IDENTITY_LIMIT: usize = 32;
    const CALL_HISTORY_IDENTITY_LIMIT: usize = 4096;
    const ROOM_MEDIA_JOIN_ATTEMPTS: i64 = 2;
    const OBSERVATION_RETRY_BACKOFF_MS: u64 = 400;
    const OBSERVATION_RETRY_CEILING: u64 = 8;

    pub const CAMERA_PUBLISH_FAILED_CODE: &'static str = "rtc_media_camera_failed";
    pub const MICROPHONE_PUBLISH_FAILED_CODE: &'static str = "rtc_media_microphone_failed";
    pub const MICROPHONE_UNAVAILABLE_CODE: &'static str = "desktop_rtc_microphone_unavailable";
    pub const SPEAKER_UNAVAILABLE_CODE: &'static str = "desktop_rtc_speaker_unavailable";
    pub const SCREEN_SHARE_NO_PORTAL_CODE: &'static str = "desktop_screen_share_no_portal";

    /// text of a desktop media failure code that has its own message, none for the generic ones
    pub fn desktop_failure_text(error: &anyhow::Error) -> Option<String> {
        let Some(RiseRTCStateError::Remote(code)) = error.downcast_ref::<RiseRTCStateError>() else {
            return None;
        };
        Self::desktop_failure_code_text(code)
    }

    /// text of a desktop media failure code, none for codes other screens already explain
    pub fn desktop_failure_code_text(code: &str) -> Option<String> {
        [Self::MICROPHONE_UNAVAILABLE_CODE, Self::SPEAKER_UNAVAILABLE_CODE, Self::SCREEN_SHARE_NO_PORTAL_CODE]
            .contains(&code)
            .then(|| rise_i18n::tr(code))
    }

    /// builds the domain with the default media driver provider
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        Self::new_with_media_driver_provider(
            engine,
            Arc::new(|| async { RiseRTCMediaRuntime::shared().make_driver() }.boxed()),
        )
    }

    pub fn new_with_media_driver_provider(
        engine: Arc<RiseEngine>,
        media_driver_provider: RiseRTCDomainMediaDriverProvider,
    ) -> Arc<Self> {
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let session_id = Uuid::new_v4();
        let repository = Arc::new(RiseRTCRepository::new(engine.clone(), account_id.clone()));
        let listener_volumes = RiseRTCListenerVolumeRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        );
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id,
            engine,
            repository,
            listener_volumes,
            media_driver_provider,
            system_bridge_owner_id: Uuid::new_v4(),
            this: this.clone(),
            state: RiseIsolated::new(RiseRTCDomainState {
                media_driver: None,
                active_credentials: None,
                pending_activation: None,
                activation_generation: 1,
                in_flight_join: None,
                departures: HashMap::new(),
                media_connection_state: RiseRTCConnectionState::Idle,
                microphone_mutation_generation: 1,
                camera_mutation_generation: 1,
                screen_share_mutation_generation: 1,
                microphone_mutation_lane_busy: false,
                camera_mutation_lane_busy: false,
                screen_share_mutation_lane_busy: false,
                microphone_mutation_waiters: Vec::new(),
                camera_mutation_waiters: Vec::new(),
                screen_share_mutation_waiters: Vec::new(),
                committed_call_media_activations: HashMap::new(),
                committed_call_media_tasks: HashMap::new(),
                reconnect_waiters: HashMap::new(),
                system_domain_adapter: None,
                call_identities: HashMap::new(),
                call_identity_order: Vec::new(),
                applied_call_keys: HashSet::new(),
                peer_half_snapshot_calls: HashSet::new(),
                peer_half_reads_in_flight: HashSet::new(),
                call_history_sink: None,
                ingested_call_history_ids: HashSet::new(),
                ingested_call_history_order: Vec::new(),
                latest_calls: None,
                latest_room_snapshot: None,
                applied_server_moderation: None,
                pushed_listener_gains: HashMap::new(),
                pushed_listener_session: None,
                resolved_listener_roster: HashSet::new(),
                resolved_listener_revision: 0,
                listener_blocks_session: None,
                listener_blocks_refresh_task: None,
                listener_intent_sequence: 0,
                call_observation_task: None,
                room_observation_task: None,
                media_event_task: None,
                heartbeat_task: None,
                reconciliation_task: None,
                active_room_id: None,
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
            }),
        })
    }

    pub async fn prepare(&self, context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared) =
            self.state.with(|state| (state.is_shutdown, state.is_prepared));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if is_prepared {
            return Ok(());
        }
        self.repository.cleanup_stale_room_cache().await;
        self.repository.register(context).await?;
        self.state.with(|state| state.is_prepared = true);
        self.start_call_observation();
        Ok(())
    }

    pub async fn activate(&self) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared, is_active) =
            self.state.with(|state| (state.is_shutdown, state.is_prepared, state.is_active));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if !is_prepared {
            return Err(RiseRTCStateError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }
        if !SaiAccountCacheScope::is_current_account(Some(&self.account_id)) {
            return Err(RiseRTCStateError::StaleAccount.into());
        }
        self.ensure_media_driver().await;
        let calls = self.state.with(|state| {
            state.is_active = true;
            state.latest_calls.clone()
        });
        let Some(domain) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let adapter = Arc::new(RiseRTCDomainSystemAdapter::new(domain.clone()));
        let client: Arc<dyn RiseRTCSystemDomainClient> = adapter.clone();
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let owner_id = self.system_bridge_owner_id;
        Self::on_main(move |cx| {
            RiseRTCPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.activate(&account_id, cx);
            });
            RiseRTCListenerVolumePresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.activate(&account_id, &account_epoch, session_id, cx);
            });
            if let Some(calls) = calls {
                RiseRTCPresentation::shared(cx)
                    .update(cx, |presentation, cx| presentation.publish_calls(calls, cx));
            }
            RiseRTCDomainRegistry::shared(cx)
                .update(cx, |registry, cx| registry.install(domain, cx));
            RiseRTCSystemDomainBridge::shared(cx)
                .update(cx, |bridge, cx| bridge.install(&client, owner_id, cx));
        })
        .await;
        self.state.with(|state| state.system_domain_adapter = Some(adapter));
        self.listener_volumes.load().await;
        Ok(())
    }

    pub async fn did_activate_ownership(&self) {
        self.revalidate_active_calls().await;
    }

    pub async fn socket_connected(&self) {
        self.resume_reconnect_waiters();
        self.schedule_authoritative_reconciliation(0).await;
        self.revalidate_active_calls().await;
    }

    pub async fn shutdown(&self) {
        let teardown = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.is_shutdown = true;
            state.is_active = false;
            if let Some(task) = state.listener_blocks_refresh_task.take() {
                task.cancel();
            }
            state.listener_blocks_session = None;

            let credentials = state.active_credentials.clone();
            let committed_contexts: Vec<RiseRTCDomainCommittedCallMediaActivation> =
                std::mem::take(&mut state.committed_call_media_activations).into_values().collect();
            let committed_tasks: Vec<RiseRTCDomainTask> =
                std::mem::take(&mut state.committed_call_media_tasks)
                    .into_values()
                    .map(|entry| entry.task)
                    .collect();
            for task in &committed_tasks {
                task.cancel();
            }
            Self::resume_reconnect_waiters_in(state);
            state.advance_activation_generation();
            state.pending_activation = None;
            if let Some(join) = state.in_flight_join.take() {
                join.cancelled.store(true, Ordering::Release);
            }
            state.active_credentials = None;
            state.call_identities = HashMap::new();
            state.call_identity_order = Vec::new();
            state.applied_call_keys = HashSet::new();
            state.peer_half_snapshot_calls = HashSet::new();
            state.peer_half_reads_in_flight = HashSet::new();
            state.active_room_id = None;
            state.latest_room_snapshot = None;
            state.media_connection_state = RiseRTCConnectionState::Idle;

            let mut tasks: Vec<RiseRTCDomainTask> = [
                state.call_observation_task.take(),
                state.room_observation_task.take(),
                state.media_event_task.take(),
                state.heartbeat_task.take(),
                state.reconciliation_task.take(),
            ]
            .into_iter()
            .flatten()
            .collect();
            for task in &tasks {
                task.cancel();
            }
            tasks.extend(committed_tasks);
            Some((credentials, committed_contexts, tasks, state.media_driver.clone()))
        });
        let Some((credentials, committed_contexts, tasks, media_driver)) = teardown else {
            return;
        };
        if let Some(credentials) = credentials {
            if let Some(media_driver) = &media_driver {
                let _ = media_driver
                    .perform(RiseRTCMediaIntent::Disconnect { session: credentials.session_key() })
                    .await;
            }
            let _ = self.repository.release_room_cache(&credentials.room_id).await;
        }
        for context in committed_contexts {
            self.cleanup_committed_call_media(&context, true).await;
        }
        for task in tasks {
            task.value().await;
        }
        self.state.with(|state| {
            state.media_driver = None;
            state.system_domain_adapter = None;
            state.latest_calls = None;
        });

        self.listener_volumes.shutdown();

        let domain = self.this.clone();
        let account_id = self.account_id.clone();
        let session_id = self.session_id;
        let owner_id = self.system_bridge_owner_id;
        Self::on_main(move |cx| {
            RiseRTCSystemDomainBridge::shared(cx)
                .update(cx, |bridge, cx| bridge.remove(owner_id, cx));
            RiseRTCDomainRegistry::shared(cx)
                .update(cx, |registry, cx| registry.remove(&domain, cx));
            let presentation = RiseRTCPresentation::shared(cx);
            let owns_presentation =
                presentation.read(cx).snapshot().account_id.as_deref() == Some(account_id.as_str());
            if owns_presentation {
                presentation.update(cx, |presentation, cx| presentation.deactivate(cx));
            }
            RiseRTCListenerVolumePresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.deactivate(&account_id, session_id, cx);
            });
        })
        .await;
    }

    pub async fn join_room(
        &self,
        room_id: &str,
        display_name: Option<&str>,
        preferred_role: Option<RiseRTCParticipantRole>,
    ) -> anyhow::Result<()> {
        self.validate_active_account().await?;
        let in_flight = self.state.with(|state| {
            state
                .in_flight_join
                .as_ref()
                .filter(|join| join.room_id == room_id)
                .map(|join| join.task.clone())
        });
        if let Some(task) = in_flight {
            return task.await.map_err(|error| Self::shared_error(&error));
        }
        let active = self.state.with(|state| {
            state
                .active_credentials
                .clone()
                .map(|credentials| (credentials, state.media_connection_state))
        });
        if let Some((active_credentials, media_connection_state)) = active {
            if active_credentials.room_id == room_id
                && media_connection_state != RiseRTCConnectionState::Failed
            {
                if !self.repository.refresh_room(room_id).await? {
                    self.schedule_authoritative_reconciliation(1_000).await;
                }
                return Ok(());
            }
            self.leave_room_expected_room_id(&active_credentials.room_id).await;
        }
        let Some(domain) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let token = Uuid::new_v4();
        let cancelled = Arc::new(AtomicBool::new(false));
        let join_room_id = room_id.to_owned();
        let display_name = display_name.map(str::to_owned);
        let handle = RiseAccountRuntime::current().spawn(RISE_RTC_TASK_CANCELLED.scope(
            cancelled.clone(),
            async move {
                domain
                    .perform_join(&join_room_id, display_name.as_deref(), preferred_role)
                    .await
                    .map_err(Arc::new)
            },
        ));
        let task: RiseRTCDomainJoinOutcome = async move {
            match handle.await {
                Ok(outcome) => outcome,
                Err(_) => Err(Arc::new(anyhow::Error::from(CancellationError))),
            }
        }
        .boxed()
        .shared();
        self.state.with(|state| {
            state.in_flight_join = Some(RiseRTCDomainInFlightJoin {
                room_id: room_id.to_owned(),
                token,
                task: task.clone(),
                cancelled,
            });
        });
        let _clear = RiseRTCDomainJoinRelease { domain: self, token };
        task.await.map_err(|error| Self::shared_error(&error))
    }

    async fn perform_join(
        &self,
        room_id: &str,
        display_name: Option<&str>,
        preferred_role: Option<RiseRTCParticipantRole>,
    ) -> anyhow::Result<()> {
        let mut attempt: i64 = 1;
        loop {
            self.await_departure(room_id).await;
            let generation = self.begin_activation(room_id);
            self.publish_connection(RiseRTCConnectionState::Connecting).await;
            let mut media_failed = false;
            let outcome: anyhow::Result<()> = async {
                let result = self.repository.join_room(room_id, display_name, preferred_role).await?;
                if !self.is_pending_activation(room_id, generation) || task_is_cancelled() {
                    self.rollback_abandoned_join(&result.credentials).await;
                    return Err(CancellationError.into());
                }
                media_failed = true;
                self.activate_media_session(
                    result.credentials,
                    RiseRTCDesiredMediaState::from_participant(&result.participant),
                    generation,
                )
                .await
            }
            .await;
            match outcome {
                Ok(()) => return Ok(()),
                Err(error) if error.is::<CancellationError>() => return Err(CancellationError.into()),
                Err(error) => {
                    if media_failed
                        && attempt < Self::ROOM_MEDIA_JOIN_ATTEMPTS
                        && self.state.with(|state| state.activation_generation == generation)
                    {
                        Self::log(format!("room media connect failed, retrying room={room_id} attempt={attempt}")).await;
                        let pause = RiseRTCConnectionWaitPolicy::backoff_milliseconds(2);
                        RiseRuntimeTimer::sleep(Duration::from_millis(u64::try_from(pause).unwrap_or(0))).await;
                        if self.state.with(|state| state.activation_generation == generation) && !task_is_cancelled() {
                            attempt += 1;
                            continue;
                        }
                        return Err(CancellationError.into());
                    }
                    if self.state.with(|state| state.activation_generation == generation) {
                        self.publish_error(Self::error_code(&error)).await;
                    }
                    return Err(error);
                }
            }
        }
    }

    pub async fn leave_room(&self) {
        let room_id = self.state.with(|state| {
            state
                .active_credentials
                .as_ref()
                .map(|credentials| credentials.room_id.clone())
                .or_else(|| state.pending_activation.as_ref().map(|pending| pending.room_id.clone()))
        });
        let Some(room_id) = room_id else {
            self.clear_active_room_presentation().await;
            return;
        };
        self.leave_room_expected_room_id(&room_id).await;
    }

    /// route safe leave used when a view disappears, a late callback
    /// from an old route must never tear down a newly joined room
    pub async fn leave_room_expected_room_id(&self, expected_room_id: &str) {
        enum Leave {
            Nothing,
            PendingOnly,
            Session(RiseRTCJoinCredentials),
        }
        let leave = self.state.with(|state| {
            let pending_matches = state
                .pending_activation
                .as_ref()
                .is_some_and(|pending| pending.room_id == expected_room_id);
            let credentials = state
                .active_credentials
                .clone()
                .filter(|credentials| credentials.room_id == expected_room_id);
            if !pending_matches && credentials.is_none() {
                return Leave::Nothing;
            }
            state.advance_activation_generation();
            if pending_matches {
                state.pending_activation = None;
            }
            let Some(credentials) = credentials else {
                state.media_connection_state = RiseRTCConnectionState::Idle;
                return Leave::PendingOnly;
            };
            state.clear_ephemeral_session();
            state.latest_room_snapshot = None;
            state.media_connection_state = RiseRTCConnectionState::Idle;
            Leave::Session(credentials)
        });
        let credentials = match leave {
            Leave::Nothing => return,
            Leave::PendingOnly => {
                self.clear_active_room_presentation().await;
                return;
            }
            Leave::Session(credentials) => credentials,
        };
        let departure = self.start_departure(&credentials);
        if let Some(media_driver) = self.media_driver() {
            let _ = media_driver
                .perform(RiseRTCMediaIntent::Disconnect { session: credentials.session_key() })
                .await;
        }
        departure.await;
        let _ = self.repository.release_room_cache(&credentials.room_id).await;
        self.clear_active_room_presentation().await;
    }

    pub async fn refresh_active_room(&self) -> anyhow::Result<()> {
        self.validate_active_account().await?;
        let Some(room_id) = self.state.with(|state| state.active_room_id.clone()) else {
            return Err(RiseRTCStateError::MissingActiveRoom.into());
        };
        self.publish_refreshing(true).await;
        let _refreshing = RiseRTCDomainRefreshingRelease { domain: self.this.clone() };
        if !self.repository.refresh_room(&room_id).await? {
            self.schedule_authoritative_reconciliation(1_000).await;
        }
        Ok(())
    }

    pub async fn load_more_participants(&self) -> anyhow::Result<()> {
        self.validate_active_account().await?;
        let Some(room_id) = self.state.with(|state| state.active_room_id.clone()) else {
            return Err(RiseRTCStateError::MissingActiveRoom.into());
        };
        self.repository.load_more_participants(&room_id).await
    }

    pub async fn request_to_speak(&self, note: Option<&str>) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        self.repository.request_to_speak(&credentials, note).await?;
        Ok(())
    }

    pub async fn cancel_request_to_speak(&self) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        self.repository.cancel_request_to_speak(&credentials).await
    }

    pub async fn promote_to_speaker(&self, user_id: &str) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        self.repository
            .change_role(&credentials.room_id, user_id, RiseRTCParticipantRole::Speaker)
            .await?;
        Ok(())
    }

    pub async fn move_to_audience(&self, user_id: &str) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        self.repository
            .change_role(&credentials.room_id, user_id, RiseRTCParticipantRole::Listener)
            .await?;
        Ok(())
    }

    pub async fn moderate(
        &self,
        user_id: &str,
        action: RiseRTCModerationAction,
        reason: Option<&str>,
    ) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        self.repository.moderate(&credentials.room_id, user_id, action, reason).await?;
        Ok(())
    }

    pub async fn send_reaction(&self, emoji: &str) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        let client_event_id = Uuid::new_v4().uuid_string().to_lowercase();
        let reaction = self.repository.send_reaction(&credentials, emoji, &client_event_id).await?;
        Self::on_main(move |cx| {
            RiseRTCPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.publish_reaction(reaction, cx));
        })
        .await;
        Ok(())
    }

    pub async fn set_microphone(&self, enabled: bool) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        let session = credentials.session_key();
        let previous = self
            .latest_self_participant()
            .map(|participant| participant.microphone_enabled)
            .unwrap_or(!enabled);
        self.acquire_microphone_mutation_lane().await;
        let _lane = RiseRTCDomainLaneRelease {
            domain: self,
            release: Self::release_microphone_mutation_lane,
        };
        let generation = self.advance_microphone_mutation_generation();
        let mut failure_stage = "server-update";
        if !enabled {
            let _ = self
                .perform_media(RiseRTCMediaIntent::SetMicrophone {
                    session: session.clone(),
                    enabled: false,
                })
                .await;
        }
        let outcome: anyhow::Result<()> = async {
            self.ensure_current_microphone_mutation(generation, &session)?;
            let accepted = self
                .repository
                .update_self_media(&credentials, Some(enabled), None, None, None)
                .await?;
            failure_stage = "local-media";
            self.ensure_current_microphone_mutation(generation, &session)?;
            if !accepted.deafened {
                let _ = self
                    .perform_media(RiseRTCMediaIntent::SetRemotePlayoutMuted {
                        session: session.clone(),
                        muted: false,
                    })
                    .await;
            }
            self.perform_media(RiseRTCMediaIntent::SetMicrophone {
                session: session.clone(),
                enabled: accepted.microphone_enabled,
            })
            .await?;
            self.ensure_current_microphone_mutation(generation, &session)
        }
        .await;
        let Err(error) = outcome else {
            return Ok(());
        };
        Self::log(format!(
            "microphone command failed enabled={enabled} stage={failure_stage} room={} {}",
            credentials.room_id,
            Self::error_summary(&error)
        ))
        .await;
        let is_cancellation = error.is::<CancellationError>();
        if !is_cancellation {
            let error_type = Self::error_type_name(&error);
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || {
                format!(
                    "microphone {} failed at {failure_stage}: {error_type}",
                    if enabled { "on" } else { "off" }
                )
            });
        }
        if !enabled
            && !is_cancellation
            && self.is_current_microphone_mutation(generation, &session)
        {
            let _ = self
                .repository
                .update_self_media(&credentials, Some(false), None, None, None)
                .await;
        }
        if enabled && self.is_current_microphone_mutation(generation, &session) {
            let _ = self
                .perform_media(RiseRTCMediaIntent::SetMicrophone {
                    session: session.clone(),
                    enabled: previous,
                })
                .await;
        }
        if enabled && self.is_current_microphone_mutation(generation, &session) {
            let _ = self
                .repository
                .update_self_media(&credentials, Some(previous), None, None, None)
                .await;
        }
        Err(error)
    }

    pub async fn set_camera(
        &self,
        enabled: bool,
        position: RiseRTCCameraPosition,
    ) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        let session = credentials.session_key();
        let previous = self
            .latest_self_participant()
            .map(|participant| participant.camera_enabled)
            .unwrap_or(!enabled);
        let generation = self.advance_camera_mutation_generation();
        self.acquire_camera_mutation_lane().await;
        let _lane = RiseRTCDomainLaneRelease {
            domain: self,
            release: Self::release_camera_mutation_lane,
        };
        let outcome: anyhow::Result<()> = async {
            self.ensure_current_camera_mutation(generation, &session)?;
            self.perform_media(RiseRTCMediaIntent::SetCamera {
                session: session.clone(),
                enabled,
                position,
            })
            .await?;
            self.ensure_current_camera_mutation(generation, &session)?;
            self.repository
                .update_self_media(&credentials, None, Some(enabled), None, None)
                .await?;
            self.ensure_current_camera_mutation(generation, &session)
        }
        .await;
        let Err(error) = outcome else {
            return Ok(());
        };
        if self.is_current_camera_mutation(generation, &session) {
            let _ = self
                .perform_media(RiseRTCMediaIntent::SetCamera {
                    session: session.clone(),
                    enabled: previous,
                    position,
                })
                .await;
        }
        if self.is_current_camera_mutation(generation, &session) {
            let _ = self
                .repository
                .update_self_media(&credentials, None, Some(previous), None, None)
                .await;
        }
        if !error.is::<CancellationError>() {
            let error_type = Self::error_type_name(&error);
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || {
                format!("camera {} failed: {error_type}", if enabled { "on" } else { "off" })
            });
        }
        Err(error)
    }

    /// self deafen
    /// output is muted locally but remote tracks stay subscribed so talking indicators
    /// keep updating, mic is forced off since hearing nobody means heard by nobody
    pub async fn set_deafened(&self, deafened: bool) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        let session = credentials.session_key();
        let previous = self
            .latest_self_participant()
            .map(|participant| participant.deafened)
            .unwrap_or(!deafened);
        let previous_microphone = self
            .latest_self_participant()
            .map(|participant| participant.microphone_enabled)
            .unwrap_or(false);
        self.acquire_microphone_mutation_lane().await;
        let _lane = RiseRTCDomainLaneRelease {
            domain: self,
            release: Self::release_microphone_mutation_lane,
        };
        let generation = self.advance_microphone_mutation_generation();
        let mut failure_stage = "local-playout";
        let outcome: anyhow::Result<()> = async {
            self.ensure_current_microphone_mutation(generation, &session)?;
            self.perform_media(RiseRTCMediaIntent::SetRemotePlayoutMuted {
                session: session.clone(),
                muted: deafened,
            })
            .await?;
            failure_stage = "server-update";
            if deafened {
                let _ = self
                    .perform_media(RiseRTCMediaIntent::SetMicrophone {
                        session: session.clone(),
                        enabled: false,
                    })
                    .await;
            }
            self.ensure_current_microphone_mutation(generation, &session)?;
            self.repository
                .update_self_media(
                    &credentials,
                    if deafened { Some(false) } else { None },
                    None,
                    None,
                    Some(deafened),
                )
                .await?;
            Ok(())
        }
        .await;
        let Err(error) = outcome else {
            return Ok(());
        };
        Self::log(format!(
            "deafen command failed deafened={deafened} stage={failure_stage} room={} {}",
            credentials.room_id,
            Self::error_summary(&error)
        ))
        .await;
        if self.is_current_microphone_mutation(generation, &session) {
            let _ = self
                .perform_media(RiseRTCMediaIntent::SetRemotePlayoutMuted {
                    session: session.clone(),
                    muted: previous,
                })
                .await;
            if deafened && previous_microphone {
                let _ = self
                    .perform_media(RiseRTCMediaIntent::SetMicrophone {
                        session: session.clone(),
                        enabled: true,
                    })
                    .await;
            }
            let _ = self
                .repository
                .update_self_media(
                    &credentials,
                    if deafened { Some(previous_microphone) } else { None },
                    None,
                    None,
                    Some(previous),
                )
                .await;
        }
        Err(error)
    }

    pub async fn preview_listener_volume(&self, user_id: &str, percent: f64, sequence: u64) {
        let user_id = trimming_whitespaces_and_newlines(user_id);
        if user_id.is_empty() || user_id == self.account_id {
            return;
        }
        let push = self.state.with(|state| {
            if state.is_shutdown || !Self::accept_listener_intent_in(state, sequence) {
                return None;
            }
            let credentials = state.active_credentials.as_ref()?;
            let mut gains = state.pushed_listener_gains.clone();
            let value = RiseRTCListenerVolume::clamped(percent);
            if value == RiseRTCListenerVolume::DEFAULT_PERCENT {
                gains.remove(user_id);
            } else {
                gains.insert(user_id.to_owned(), RiseRTCListenerVolume::gain(value));
            }
            Some((gains, credentials.session_key()))
        });
        let Some((gains, session)) = push else {
            return;
        };
        self.push_listener_gains(gains, session).await;
    }

    /// the finger lifted, 0 is the mute for me end of the
    /// same rail so there is no second flag to keep in step
    pub async fn commit_listener_volume(&self, user_id: &str, percent: f64, sequence: u64) {
        self.store_listener_volume(user_id, Some(percent), sequence).await;
    }

    pub async fn reset_listener_volume(&self, user_id: &str, sequence: u64) {
        self.store_listener_volume(user_id, None, sequence).await;
    }

    /// re resolves and re pushes always
    /// the block tier lives outside this domain and moves without
    /// the roster or prefs changing so the cheap gate cant see it
    pub async fn refresh_listener_volumes(&self) {
        self.apply_listener_volumes(true).await;
    }

    async fn store_listener_volume(&self, user_id: &str, percent: Option<f64>, sequence: u64) {
        let user_id = trimming_whitespaces_and_newlines(user_id);
        if user_id.is_empty() || user_id == self.account_id {
            return;
        }
        let accepted = self.state.with(|state| {
            !state.is_shutdown && Self::accept_listener_intent_in(state, sequence)
        });
        if !accepted {
            return;
        }
        self.listener_volumes.set_percent(percent, user_id).await;
        self.apply_listener_volumes(true).await;
    }

    /// stamped on the main thread where the gesture runs so
    /// a lower number is an older intent and gets dropped
    fn accept_listener_intent_in(state: &mut RiseRTCDomainState, sequence: u64) -> bool {
        if sequence <= state.listener_intent_sequence {
            return false;
        }
        state.listener_intent_sequence = sequence;
        true
    }

    async fn apply_listener_volumes(&self, force: bool) {
        let Some(credentials) = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.active_credentials.clone()
        }) else {
            return;
        };
        let session = credentials.session_key();
        let revision = self.listener_volumes.current_revision();
        let roster = self.state.with(|state| {
            let roster: HashSet<String> = state
                .latest_room_snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot.participants.iter().map(|participant| participant.user_id.clone()).collect()
                })
                .unwrap_or_default();
            if !force
                && revision == state.resolved_listener_revision
                && roster == state.resolved_listener_roster
            {
                return None;
            }
            state.resolved_listener_revision = revision;
            state.resolved_listener_roster = roster.clone();
            Some(roster)
        });
        let Some(roster) = roster else {
            return;
        };

        let overrides = self.listener_volumes.overrides();
        let used: HashSet<String> =
            roster.iter().filter(|user_id| overrides.contains_key(*user_id)).cloned().collect();
        self.listener_volumes.note_used(&used);
        let mut candidates = roster;
        candidates.extend(overrides.keys().cloned());
        candidates.remove(&self.account_id);
        if candidates.is_empty() {
            self.push_listener_gains(HashMap::new(), session).await;
            return;
        }

        self.refresh_listener_blocks_if_needed(&session);
        let probe: Vec<String> = candidates.iter().cloned().collect();
        let blocked: HashSet<String> = Self::on_main(move |cx| {
            let store = UserServicesStore::shared(cx);
            let store = store.read(cx);
            probe.into_iter().filter(|user_id| store.is_totally_blocked(user_id, cx)).collect()
        })
        .await
        .unwrap_or_default();
        let mut gains: HashMap<String, f64> = HashMap::new();
        for user_id in candidates {
            let percent = RiseRTCListenerVolume::resolved_percent(
                overrides.get(&user_id).copied(),
                blocked.contains(&user_id),
            );
            if percent == RiseRTCListenerVolume::DEFAULT_PERCENT {
                continue;
            }
            gains.insert(user_id, RiseRTCListenerVolume::gain(percent));
        }
        if self.state.with(|state| state.active_session_key()).as_ref() != Some(&session) {
            return;
        }
        self.push_listener_gains(gains, session).await;
    }

    fn refresh_listener_blocks_if_needed(&self, session: &RiseRTCMediaSessionKey) {
        let account_id = self.account_id.clone();
        let domain = self.this.clone();
        let refreshed_session = session.clone();
        let work = async move {
            let session = refreshed_session;
            let user_domain = Self::on_main(|cx| {
                RiseUserDomainRegistry::shared(cx).read(cx).active().cloned()
            })
            .await
            .flatten();
            let Some(user_domain) = user_domain.filter(|user_domain| user_domain.account_id == account_id)
            else {
                return;
            };
            user_domain.load_blocked_users(false).await;
            if task_is_cancelled() {
                return;
            }
            let Some(domain) = domain.upgrade() else {
                return;
            };
            domain.listener_blocks_did_refresh(&session).await;
        };
        self.state.with(|state| {
            if state.listener_blocks_session.as_ref() == Some(session) {
                return;
            }
            state.listener_blocks_session = Some(session.clone());
            if let Some(task) = state.listener_blocks_refresh_task.take() {
                task.cancel();
            }
            state.listener_blocks_refresh_task = Some(RiseRTCDomainTask::spawn(work));
        });
    }

    async fn listener_blocks_did_refresh(&self, session: &RiseRTCMediaSessionKey) {
        let current = self.state.with(|state| {
            if state.is_shutdown || state.active_session_key().as_ref() != Some(session) {
                return false;
            }
            state.listener_blocks_refresh_task = None;
            true
        });
        if !current {
            return;
        }
        self.apply_listener_volumes(true).await;
    }

    async fn push_listener_gains(&self, gains: HashMap<String, f64>, session: RiseRTCMediaSessionKey) {
        let changed = self.state.with(|state| {
            if state.pushed_listener_session.as_ref() == Some(&session)
                && state.pushed_listener_gains == gains
            {
                return false;
            }
            state.pushed_listener_session = Some(session.clone());
            state.pushed_listener_gains = gains.clone();
            true
        });
        if !changed {
            return;
        }
        let _ = self
            .perform_media(RiseRTCMediaIntent::SetListenerVolumes { session, gains })
            .await;
    }

    /// toggles screen share, include_device_audio defaults to false and quality to balanced
    pub async fn set_screen_share(
        &self,
        enabled: bool,
        include_device_audio: bool,
        quality: RiseRTCScreenShareQuality,
    ) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        let session = credentials.session_key();
        let previous = self
            .latest_self_participant()
            .map(|participant| participant.screen_share_enabled)
            .unwrap_or(!enabled);
        let generation = self.advance_screen_share_mutation_generation();
        self.acquire_screen_share_mutation_lane().await;
        let _lane = RiseRTCDomainLaneRelease {
            domain: self,
            release: Self::release_screen_share_mutation_lane,
        };
        let server_asked = AtomicBool::new(false);
        let outcome: anyhow::Result<()> = async {
            self.perform_media(RiseRTCMediaIntent::SetScreenShare {
                session: session.clone(),
                enabled,
                include_device_audio,
                quality,
            })
            .await?;
            self.ensure_current_screen_share_mutation(generation, &session)?;
            server_asked.store(true, Ordering::Release);
            self.repository
                .update_self_media(&credentials, None, None, Some(enabled), None)
                .await?;
            self.ensure_current_screen_share_mutation(generation, &session)
        }
        .await;
        let Err(error) = outcome else {
            return Ok(());
        };
        if previous != enabled && self.is_current_screen_share_mutation(generation, &session) {
            let _ = self
                .perform_media(RiseRTCMediaIntent::SetScreenShare {
                    session: session.clone(),
                    enabled: previous,
                    include_device_audio,
                    quality,
                })
                .await;
        }
        if server_asked.load(Ordering::Acquire) && self.is_current_screen_share_mutation(generation, &session) {
            let _ = self
                .repository
                .update_self_media(&credentials, None, None, Some(previous), None)
                .await;
        }
        if !error.is::<CancellationError>() {
            let error_type = Self::error_type_name(&error);
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || {
                format!(
                    "screen share {} failed: {error_type}",
                    if enabled { "start" } else { "stop" }
                )
            });
        }
        Err(error)
    }

    async fn clear_screen_share_after_media_loss(&self, session: &RiseRTCMediaSessionKey) {
        let Some(credentials) = self.state.with(|state| {
            state
                .active_credentials
                .clone()
                .filter(|credentials| &credentials.session_key() == session)
        }) else {
            return;
        };
        let start_in_flight = self.state.with(|state| state.screen_share_mutation_lane_busy);
        if !start_in_flight
            && !self
                .latest_self_participant()
                .is_some_and(|participant| participant.screen_share_enabled)
        {
            return;
        }
        self.advance_screen_share_mutation_generation();
        self.acquire_screen_share_mutation_lane().await;
        let _lane = RiseRTCDomainLaneRelease {
            domain: self,
            release: Self::release_screen_share_mutation_lane,
        };
        if self.state.with(|state| state.active_session_key()).as_ref() != Some(session) {
            return;
        }
        let _ = self
            .repository
            .update_self_media(&credentials, None, None, Some(false), None)
            .await;
    }

    async fn clear_microphone_after_media_loss(&self, session: &RiseRTCMediaSessionKey) {
        let Some(credentials) = self.state.with(|state| {
            state
                .active_credentials
                .clone()
                .filter(|credentials| &credentials.session_key() == session)
        }) else {
            return;
        };
        self.advance_microphone_mutation_generation();
        let _ = self
            .perform_media(RiseRTCMediaIntent::SetMicrophone {
                session: session.clone(),
                enabled: false,
            })
            .await;
        if self.latest_self_participant().is_some_and(|participant| participant.microphone_enabled) {
            let _ = self
                .repository
                .update_self_media(&credentials, Some(false), None, None, None)
                .await;
        }
        self.publish_nonfatal_media_error(
            Self::MICROPHONE_UNAVAILABLE_CODE,
            Some("call microphone did not open, the call goes on with the microphone off".to_owned()),
        )
        .await;
    }

    pub async fn switch_camera(&self, position: RiseRTCCameraPosition) -> anyhow::Result<()> {
        let credentials = self.require_credentials().await?;
        self.perform_media(RiseRTCMediaIntent::SwitchCamera {
            session: credentials.session_key(),
            position,
        })
        .await
    }

    pub async fn select_audio_input(&self, id: Option<&str>) -> anyhow::Result<()> {
        let session = self.require_route_session().await?;
        self.perform_media(RiseRTCMediaIntent::SelectAudioInput {
            session,
            id: id.map(str::to_owned),
        })
        .await
    }

    pub async fn select_audio_output(&self, id: Option<&str>) -> anyhow::Result<()> {
        let session = self.require_route_session().await?;
        self.perform_media(RiseRTCMediaIntent::SelectAudioOutput {
            session,
            id: id.map(str::to_owned),
        })
        .await
    }

    /// ringback starts once the call is committed before its media session is
    /// active, so a route picked in that gap belongs to the session being activated
    async fn require_route_session(&self) -> anyhow::Result<RiseRTCMediaSessionKey> {
        self.validate_active_account().await?;
        self.state
            .with(|state| {
                if let Some(active_credentials) = &state.active_credentials {
                    return Some(active_credentials.session_key());
                }
                let pending_activation = state.pending_activation.as_ref()?;
                state
                    .committed_call_media_activations
                    .values()
                    .find(|context| {
                        context.credentials.room_id == pending_activation.room_id
                            && context.activation_generation == pending_activation.generation
                    })
                    .map(|context| context.credentials.session_key())
            })
            .ok_or_else(|| RiseRTCStateError::MissingJoinCredentials.into())
    }

    pub async fn end_direct_call_for_room_switch(&self, call_id: &str) -> anyhow::Result<()> {
        self.validate_active_account().await?;
        let call = self.repository.get_call(call_id).await?;
        self.validate_active_account().await?;
        if call.state.is_terminal() {
            self.report_system_event(&call).await;
            if let Some(credentials) = self.active_direct_call_credentials(call_id) {
                self.disconnect_media_without_leave(&credentials).await;
            }
            return Ok(());
        }
        let idempotency_key = Uuid::new_v4().uuid_string().to_lowercase();
        if call.state == RiseRTCCallState::Ringing {
            if call.caller_user_id == self.account_id {
                self.cancel_call(call_id, Some("client_cancelled"), &idempotency_key).await?;
            } else {
                self.decline_call(call_id, Some("client_declined"), &idempotency_key).await?;
            }
        } else {
            self.end_call(call_id, Some("client_hangup"), &idempotency_key).await?;
        }
        Ok(())
    }

    pub async fn start_direct_call(
        &self,
        callee_id: &str,
        chat_id: &str,
        media_mode: RiseRTCCallMediaMode,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        self.validate_active_account().await?;
        let identity = RiseRTCCallCryptoIdentity::new();
        let public_key = identity.public_key();
        let result = self
            .repository
            .start_direct_call(callee_id, chat_id, media_mode, idempotency_key, Some(public_key))
            .await?;
        self.remember_call_identity(identity, &result.call.call_id);
        let generation = self.begin_activation(&result.call.room_id);
        self.schedule_committed_call_media_activation(
            result.call.clone(),
            result.credentials,
            RiseRTCDesiredMediaState::from_call(&result.call),
            generation,
            RiseRTCCommittedCallMediaKind::OutgoingStart,
            None,
            false,
        );
        Ok(result.call)
    }

    pub async fn answer_call(
        &self,
        call_id: &str,
        display_name: Option<&str>,
        installation_id: &str,
    ) -> anyhow::Result<RiseRTCAnswerOutcome> {
        self.validate_active_account().await?;
        let identity = RiseRTCCallCryptoIdentity::new();
        let public_key = identity.public_key();
        let (outcome, credentials) = self
            .repository
            .answer_call(call_id, display_name, installation_id, Some(public_key))
            .await?;
        self.remember_call_identity(identity, call_id);
        if let (Some(credentials), RiseRTCAnswerOutcome::Answered(call)) = (credentials, &outcome) {
            let generation = self.begin_activation(&call.room_id);
            self.schedule_committed_call_media_activation(
                call.clone(),
                credentials,
                RiseRTCDesiredMediaState::from_call(call),
                generation,
                RiseRTCCommittedCallMediaKind::AcceptedIncoming,
                None,
                false,
            );
        }
        Ok(outcome)
    }

    pub async fn rejoin_call(
        &self,
        call_id: &str,
        installation_id: Option<&str>,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        self.validate_active_account().await?;
        let result = self.repository.rejoin_call(call_id, installation_id).await?;
        let generation = self.prepare_activation(&result.call.room_id).await;
        self.activate_media_session(
            result.credentials,
            RiseRTCDesiredMediaState::from_call(&result.call),
            generation,
        )
        .await?;
        Ok(result.call)
    }

    pub async fn decline_call(
        &self,
        call_id: &str,
        reason: Option<&str>,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        self.perform_call_action(RiseRTCEndpoints::decline_call(), call_id, reason, idempotency_key)
            .await
    }

    pub async fn cancel_call(
        &self,
        call_id: &str,
        reason: Option<&str>,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        self.perform_call_action(RiseRTCEndpoints::cancel_call(), call_id, reason, idempotency_key)
            .await
    }

    pub async fn end_call(
        &self,
        call_id: &str,
        reason: Option<&str>,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        self.perform_call_action(RiseRTCEndpoints::end_call(), call_id, reason, idempotency_key)
            .await
    }

    pub fn deliver_hangup_when_online(
        &self,
        action: RiseRTCHangupAction,
        call_id: &str,
        reason: &str,
        idempotency_key: &str,
    ) {
        if self.state.with(|state| state.is_shutdown) {
            return;
        }
        let endpoint: &'static RiseRTCEndpoint = match action {
            RiseRTCHangupAction::End => RiseRTCEndpoints::end_call(),
            RiseRTCHangupAction::Cancel => RiseRTCEndpoints::cancel_call(),
            RiseRTCHangupAction::Decline => RiseRTCEndpoints::decline_call(),
        };
        let domain = self.this.clone();
        let call_id = call_id.to_owned();
        let reason = reason.to_owned();
        let idempotency_key = idempotency_key.to_owned();
        RiseAccountRuntime::current().spawn(async move {
            let deadline = RiseClock::monotonic() + RiseRTCConnectionWaitPolicy::window();
            let mut attempt: i64 = 0;
            loop {
                attempt += 1;
                let Some(this) = domain.upgrade() else {
                    return;
                };
                match this
                    .perform_call_action(endpoint, &call_id, Some(&reason), &idempotency_key)
                    .await
                {
                    Ok(_) => return,
                    Err(error) if RiseRTCConnectionWaitPolicy::signalling_failure(&error).is_some() => {
                        if RiseClock::monotonic() >= deadline
                            || this.state.with(|state| state.is_shutdown)
                        {
                            return;
                        }
                        drop(this);
                        let pause = RiseRTCConnectionWaitPolicy::backoff_milliseconds(attempt);
                        tokio::time::sleep(Duration::from_millis(u64::try_from(pause).unwrap_or(0)))
                            .await;
                    }
                    Err(_) => return,
                }
            }
        });
    }

    pub async fn get_call(&self, call_id: &str) -> anyhow::Result<RiseRTCDirectCall> {
        self.validate_active_account().await?;
        self.repository.get_call(call_id).await
    }

    pub async fn start_call_recording(
        &self,
        call_id: &str,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        self.validate_active_account().await?;
        self.repository.call_recording(true, call_id, idempotency_key).await
    }

    pub async fn stop_call_recording(
        &self,
        call_id: &str,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        self.validate_active_account().await?;
        self.repository.call_recording(false, call_id, idempotency_key).await
    }

    pub async fn accept_system_incoming_call(
        &self,
        call_id: &str,
        caller: Option<RiseRTCUserProfile>,
    ) {
        if self.state.with(|state| state.is_shutdown) {
            return;
        }
        self.repository.accept_system_call(call_id, caller).await;
    }

    pub async fn register_system_vo_ip_token(
        &self,
        token: &str,
        environment: &str,
        ownership_generation: i64,
        installation_id: &str,
    ) -> anyhow::Result<()> {
        self.validate_active_account().await?;
        self.repository
            .register_voip_token(token, environment, ownership_generation, installation_id)
            .await
    }

    pub async fn revoke_system_vo_ip_token(
        &self,
        token: &str,
        environment: &str,
    ) -> anyhow::Result<()> {
        if self.state.with(|state| state.is_shutdown) {
            return Err(RiseRTCStateError::MissingActiveSession.into());
        }
        self.repository.revoke_voip_token(token, environment).await
    }

    async fn activate_media_session(
        &self,
        credentials: RiseRTCJoinCredentials,
        desired_media: RiseRTCDesiredMediaState,
        generation: u64,
    ) -> anyhow::Result<()> {
        self.ensure_media_driver().await;
        if !self.is_pending_activation(&credentials.room_id, generation) {
            self.rollback_abandoned_join(&credentials).await;
            return Err(CancellationError.into());
        }

        let session = credentials.session_key();
        let current = self.state.with(|state| {
            state.active_credentials.clone().filter(|current| current.session_key() != session)
        });
        if let Some(current) = current {
            self.clear_ephemeral_session();
            let departure = self.start_departure(&current);
            let _ = self
                .perform_media(RiseRTCMediaIntent::Disconnect { session: current.session_key() })
                .await;
            departure.await;
            let _ = self.repository.release_room_cache(&current.room_id).await;
        }
        if !self.is_pending_activation(&credentials.room_id, generation) || task_is_cancelled() {
            self.rollback_abandoned_join(&credentials).await;
            return Err(CancellationError.into());
        }

        Self::log(format!("room activation store start room={}", credentials.room_id)).await;
        if let Err(error) = self.repository.mark_room_active(&credentials.room_id).await {
            Self::log(format!(
                "room activation store failed room={} {}",
                credentials.room_id,
                Self::error_summary(&error)
            ))
            .await;
            let error_type = Self::error_type_name(&error);
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || {
                format!("rtc room join failed: {error_type}")
            });
            self.state.with(|state| {
                if state.pending_activation.as_ref().map(|pending| pending.generation) == Some(generation) {
                    state.pending_activation = None;
                }
            });
            self.rollback_abandoned_join(&credentials).await;
            return Err(error);
        }
        Self::log(format!("room activation store complete room={}", credentials.room_id)).await;
        if !self.is_pending_activation(&credentials.room_id, generation) || task_is_cancelled() {
            self.rollback_abandoned_join(&credentials).await;
            return Err(CancellationError.into());
        }

        let (initial_microphone_generation, initial_camera_generation) = self.state.with(|state| {
            state.active_credentials = Some(credentials.clone());
            state.active_room_id = Some(credentials.room_id.clone());
            state.pending_activation = None;
            state.media_connection_state = RiseRTCConnectionState::Connecting;
            state.latest_room_snapshot = None;
            (
                state.advance_microphone_mutation_generation(),
                state.advance_camera_mutation_generation(),
            )
        });
        self.start_room_observation(credentials.room_id.clone());
        let selected_room_id = credentials.room_id.clone();
        Self::on_main(move |cx| {
            RiseRTCPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.select_room(Some(selected_room_id), cx);
                presentation.publish_connection_state(RiseRTCConnectionState::Connecting, cx);
            });
        })
        .await;
        let outcome: anyhow::Result<()> = async {
            Self::log(format!("media connect dispatch room={}", credentials.room_id)).await;
            self.perform_media(RiseRTCMediaIntent::Connect(credentials.clone())).await?;
            Self::log(format!("media connect complete room={}", credentials.room_id)).await;
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Session, || "rtc media connected".to_owned());
            if !self.is_active_session(&session, generation) || task_is_cancelled() {
                return Err(CancellationError.into());
            }
            self.start_heartbeat(credentials.clone());
            if let Some(latest_calls) = self.state.with(|state| state.latest_calls.clone()) {
                self.apply_call_encryption_if_possible(&latest_calls).await;
            }
            self.publish_desired_initial_media(
                desired_media,
                &credentials,
                generation,
                initial_microphone_generation,
                initial_camera_generation,
            )
            .await;
            if !self.is_active_session(&session, generation) || task_is_cancelled() {
                return Err(CancellationError.into());
            }
            self.apply_server_moderation(self.latest_self_participant()).await;
            self.apply_listener_volumes(true).await;
            self.schedule_authoritative_reconciliation(0).await;
            Ok(())
        }
        .await;
        let Err(error) = outcome else {
            return Ok(());
        };
        Self::log(format!(
            "media activation failed room={} {}",
            credentials.room_id,
            Self::error_summary(&error)
        ))
        .await;
        let error_type = Self::error_type_name(&error);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || {
            format!("rtc media activation failed: {error_type}")
        });
        self.state.with(|state| {
            if state.active_session_key().as_ref() == Some(&session) {
                state.clear_ephemeral_session();
                state.media_connection_state = RiseRTCConnectionState::Failed;
            }
            if state.pending_activation.as_ref().map(|pending| pending.generation) == Some(generation) {
                state.pending_activation = None;
            }
        });
        let departure = if credentials.room_mode == RiseRTCRoomMode::DirectCall {
            None
        } else {
            Some(self.start_departure(&credentials))
        };
        let _ = self.perform_media(RiseRTCMediaIntent::Disconnect { session }).await;
        if let Some(departure) = departure {
            departure.await;
        }
        Err(error)
    }

    async fn publish_desired_initial_media(
        &self,
        desired: RiseRTCDesiredMediaState,
        credentials: &RiseRTCJoinCredentials,
        activation_generation: u64,
        microphone_generation: u64,
        camera_generation: u64,
    ) {
        let session = credentials.session_key();
        if desired.microphone_enabled
            && self.is_active_session(&session, activation_generation)
            && self.is_current_microphone_mutation(microphone_generation, &session)
        {
            match self
                .publish_initial_microphone(credentials, activation_generation, microphone_generation)
                .await
            {
                Ok(()) => {}
                Err(error) if error.is::<CancellationError>() => {}
                Err(error) => {
                    if self.is_active_session(&session, activation_generation)
                        && self.is_current_microphone_mutation(microphone_generation, &session)
                    {
                        let _ = self
                            .repository
                            .update_self_media(credentials, Some(false), None, None, None)
                            .await;
                        if self.is_current_microphone_mutation(microphone_generation, &session) {
                            let failure = Self::error_code(&error);
                            let refused = failure == RiseRTCDeviceRefusal::MICROPHONE
                                || failure == RiseRTCDeviceRefusal::MICROPHONE_RESTRICTED;
                            let code = if refused {
                                failure.clone()
                            } else {
                                Self::MICROPHONE_PUBLISH_FAILED_CODE.to_owned()
                            };
                            self.publish_nonfatal_media_error(
                                &code,
                                Some(format!("initial microphone publish failed: {failure}")),
                            )
                            .await;
                        }
                    }
                }
            }
        }

        if desired.camera_enabled
            && self.is_active_session(&session, activation_generation)
            && self.is_current_camera_mutation(camera_generation, &session)
        {
            match self
                .publish_initial_camera(credentials, activation_generation, camera_generation)
                .await
            {
                Ok(()) => {}
                Err(error) if error.is::<CancellationError>() => {}
                Err(error) => {
                    if self.is_active_session(&session, activation_generation)
                        && self.is_current_camera_mutation(camera_generation, &session)
                    {
                        let _ = self
                            .repository
                            .update_self_media(credentials, None, Some(false), None, None)
                            .await;
                        if self.is_current_camera_mutation(camera_generation, &session) {
                            self.publish_nonfatal_media_error(
                                Self::CAMERA_PUBLISH_FAILED_CODE,
                                Some(format!(
                                    "initial camera publish failed: {}",
                                    Self::error_code(&error)
                                )),
                            )
                            .await;
                        }
                    }
                }
            }
        }
    }

    async fn publish_initial_microphone(
        &self,
        credentials: &RiseRTCJoinCredentials,
        activation_generation: u64,
        mutation_generation: u64,
    ) -> anyhow::Result<()> {
        self.acquire_microphone_mutation_lane().await;
        let _lane = RiseRTCDomainLaneRelease {
            domain: self,
            release: Self::release_microphone_mutation_lane,
        };
        let session = credentials.session_key();
        if !self.is_active_session(&session, activation_generation) {
            return Err(CancellationError.into());
        }
        self.ensure_current_microphone_mutation(mutation_generation, &session)?;
        self.perform_media(RiseRTCMediaIntent::SetMicrophone { session: session.clone(), enabled: true })
            .await?;
        self.ensure_current_microphone_mutation(mutation_generation, &session)
    }

    async fn publish_initial_camera(
        &self,
        credentials: &RiseRTCJoinCredentials,
        activation_generation: u64,
        mutation_generation: u64,
    ) -> anyhow::Result<()> {
        self.acquire_camera_mutation_lane().await;
        let _lane = RiseRTCDomainLaneRelease {
            domain: self,
            release: Self::release_camera_mutation_lane,
        };
        let session = credentials.session_key();
        if !self.is_active_session(&session, activation_generation) {
            return Err(CancellationError.into());
        }
        self.ensure_current_camera_mutation(mutation_generation, &session)?;
        self.perform_media(RiseRTCMediaIntent::SetCamera {
            session: session.clone(),
            enabled: true,
            position: RiseRTCCameraPosition::Front,
        })
        .await?;
        self.ensure_current_camera_mutation(mutation_generation, &session)
    }

    /// schedules media activation for a committed call,
    /// recovering_from defaults to none and restores_deafen to false
    #[allow(clippy::too_many_arguments)]
    fn schedule_committed_call_media_activation(
        &self,
        call: RiseRTCDirectCall,
        credentials: RiseRTCJoinCredentials,
        desired_media: RiseRTCDesiredMediaState,
        generation: u64,
        kind: RiseRTCCommittedCallMediaKind,
        recovering_from: Option<String>,
        restores_deafen: bool,
    ) {
        let task_id = Uuid::new_v4();
        let call_id = call.call_id.clone();
        let context = RiseRTCDomainCommittedCallMediaActivation {
            task_id,
            call,
            credentials,
            desired_media,
            activation_generation: generation,
            kind,
            recovering_from,
            restores_deafen,
        };
        let domain = self.this.clone();
        self.state.with(|state| {
            if let Some(previous_task) = state.committed_call_media_tasks.remove(&call_id) {
                previous_task.task.cancel();
            }
            state.committed_call_media_activations.insert(call_id.clone(), context.clone());
            let task = RiseRTCDomainTask::spawn(async move {
                let Some(domain) = domain.upgrade() else {
                    return;
                };
                domain.run_committed_call_media_activation(context).await;
            });
            state
                .committed_call_media_tasks
                .insert(call_id, RiseRTCDomainCommittedCallMediaTask { task_id, task });
        });
    }

    async fn run_committed_call_media_activation(
        &self,
        context: RiseRTCDomainCommittedCallMediaActivation,
    ) {
        let _finish = RiseRTCDomainCommittedMediaFinish {
            domain: self,
            call_id: context.call.call_id.clone(),
            task_id: context.task_id,
        };
        if !self.owns_committed_call_media_activation(&context)
            || self.state.with(|state| state.is_shutdown)
            || task_is_cancelled()
        {
            if context.recovering_from.is_none() {
                self.rollback_abandoned_join(&context.credentials).await;
            }
            return;
        }
        let deadline = RiseClock::monotonic() + RiseRTCConnectionWaitPolicy::window();
        let mut attempt = context.clone();
        let mut failure_code = context.recovering_from.clone();
        let mut round: i64 = 0;
        loop {
            if let Some(code) = failure_code.as_deref() {
                round += 1;
                if !self.prepare_committed_media_retry(&mut attempt, code, round, deadline).await {
                    let error: anyhow::Error = RiseRTCStateError::Remote(code.to_owned()).into();
                    self.handle_committed_call_media_activation_failure(&attempt, &error).await;
                    return;
                }
            }
            let outcome: anyhow::Result<()> = async {
                self.activate_media_session(
                    attempt.credentials.clone(),
                    attempt.desired_media,
                    attempt.activation_generation,
                )
                .await?;
                if !self.owns_committed_call_media_activation(&context) || task_is_cancelled() {
                    return Err(CancellationError.into());
                }
                Ok(())
            }
            .await;
            match outcome {
                Ok(()) => {
                    if context.restores_deafen {
                        let _ = self.set_deafened(true).await;
                    }
                    return;
                }
                Err(error) => {
                    let code = Self::error_code(&error);
                    if error.is::<CancellationError>()
                        || !RiseRTCConnectionWaitPolicy::is_transient_media_failure(&code)
                        || RiseClock::monotonic() >= deadline
                        || !self.may_retry_committed_media(&attempt)
                    {
                        self.handle_committed_call_media_activation_failure(&attempt, &error).await;
                        return;
                    }
                    Self::log(format!(
                        "call media connect failed, retrying call={} round={} code={code}",
                        attempt.call.call_id,
                        round + 1
                    ))
                    .await;
                    failure_code = Some(code);
                }
            }
        }
    }

    async fn prepare_committed_media_retry(
        &self,
        attempt: &mut RiseRTCDomainCommittedCallMediaActivation,
        failure_code: &str,
        round: i64,
        deadline: Duration,
    ) -> bool {
        let mut needs_credentials = RiseRTCConnectionWaitPolicy::needs_fresh_credentials(
            failure_code,
            attempt.credentials.expires_at_ms,
            Self::now_milliseconds(),
        );
        let mut pause = round;
        loop {
            self.await_reconnect_or_backoff(RiseRTCConnectionWaitPolicy::backoff_milliseconds(pause))
                .await;
            if RiseClock::monotonic() >= deadline || !self.may_retry_committed_media(attempt) {
                return false;
            }
            if needs_credentials || attempt.credentials.expires_at_ms <= Self::now_milliseconds() {
                let installation_id = RiseInstallationIdentity::current().ok();
                match self
                    .repository
                    .rejoin_call(&attempt.call.call_id, installation_id.as_deref())
                    .await
                {
                    Ok(result) => {
                        attempt.credentials = result.credentials;
                        needs_credentials = false;
                    }
                    Err(error) => {
                        if RiseRTCConnectionWaitPolicy::signalling_failure(&error).is_none() {
                            return false;
                        }
                        pause += 1;
                        continue;
                    }
                }
                if RiseClock::monotonic() >= deadline || !self.may_retry_committed_media(attempt) {
                    return false;
                }
            }
            attempt.activation_generation = self.begin_activation(&attempt.call.room_id);
            let updated = attempt.clone();
            self.state.with(|state| {
                state.committed_call_media_activations.insert(updated.call.call_id.clone(), updated);
            });
            return true;
        }
    }

    fn may_retry_committed_media(&self, attempt: &RiseRTCDomainCommittedCallMediaActivation) -> bool {
        self.owns_committed_call_media_activation(attempt)
            && !task_is_cancelled()
            && !self.state.with(|state| state.is_shutdown)
            && self.cached_call_state(&attempt.call.call_id).map(|state| state.is_terminal()) != Some(true)
    }

    /// sleeps the retry pause or returns as soon as the socket is back or the task is cancelled
    /// waiter is registered before any of them can fire
    async fn await_reconnect_or_backoff(&self, milliseconds: i64) {
        if task_is_cancelled() {
            return;
        }
        let waiter_id = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel();
        self.state.with(|state| {
            state.reconnect_waiters.insert(waiter_id, sender);
        });
        let domain = self.this.clone();
        let delay = Duration::from_millis(u64::try_from(milliseconds).unwrap_or(0));
        let sleeper = RiseAccountRuntime::current().spawn(async move {
            tokio::time::sleep(delay).await;
            if let Some(domain) = domain.upgrade() {
                domain.resume_reconnect_waiter(waiter_id);
            }
        });
        let _release = RiseRTCDomainReconnectWaiterRelease { domain: self, waiter_id, sleeper };
        let _ = receiver.await;
    }

    async fn handle_committed_call_media_activation_failure(
        &self,
        context: &RiseRTCDomainCommittedCallMediaActivation,
        error: &anyhow::Error,
    ) {
        let context_still_owned = self.owns_committed_call_media_activation(context);
        let Some(action) = RiseRTCCommittedMediaFailurePolicy::action(
            context.kind,
            context_still_owned,
            !context_still_owned,
            self.state.with(|state| state.is_shutdown),
        ) else {
            return;
        };

        let failure_generation = self.state.with(|state| {
            if state.activation_generation != context.activation_generation {
                return None;
            }
            state.pending_activation = None;
            Some(state.advance_activation_generation())
        });

        let call_id = context.call.call_id.clone();
        let (local_event, endpoint): (RiseRTCSystemVoIPEvent, &'static RiseRTCEndpoint) = match action {
            RiseRTCCommittedMediaFailureAction::CancelOutgoing => (
                if self.cached_call_state(&call_id) == Some(RiseRTCCallState::Accepted) {
                    RiseRTCSystemVoIPEvent::Ended
                } else {
                    RiseRTCSystemVoIPEvent::Cancelled
                },
                RiseRTCEndpoints::cancel_call(),
            ),
            RiseRTCCommittedMediaFailureAction::EndAcceptedIncoming => {
                (RiseRTCSystemVoIPEvent::Ended, RiseRTCEndpoints::end_call())
            }
        };

        let reported_call_id = call_id.clone();
        Self::on_main(move |cx| {
            RiseRTCCallSystemHooks::report_remote_call_event(&reported_call_id, local_event, cx);
        })
        .await;
        if !self.owns_committed_compensation(context) {
            self.cleanup_committed_call_media(context, false).await;
            return;
        }

        let media_cleanup = self.cleanup_committed_call_media(context, false);
        let compensation = async {
            if !self.owns_committed_compensation(context) {
                return;
            }
            let task_id = context.task_id.uuid_string().to_lowercase();
            let idempotency_key =
                [call_id.as_str(), task_id.as_str(), "media-activation-failed"].join(":");
            let terminal_call = self
                .execute_terminal_call_action(
                    endpoint,
                    &call_id,
                    &idempotency_key,
                    Some("media_activation_failed"),
                    Some(context.task_id),
                )
                .await
                .ok();
            if self.owns_committed_compensation(context)
                && let Some(terminal_call) = terminal_call
            {
                self.report_system_event(&terminal_call).await;
            }
        };
        futures::join!(media_cleanup, compensation);

        if !self.owns_committed_compensation(context) {
            return;
        }
        let publishes_failure = self.state.with(|state| {
            state.committed_call_media_activations.remove(&call_id);
            let Some(failure_generation) = failure_generation else {
                return false;
            };
            if state.activation_generation != failure_generation || state.active_credentials.is_some() {
                return false;
            }
            state.media_connection_state = RiseRTCConnectionState::Failed;
            true
        });
        if publishes_failure {
            self.publish_error(Self::error_code(error)).await;
        }
    }

    /// true when this domain owns the context, the task isnt cancelled and the domain isnt shut down
    fn owns_committed_compensation(&self, context: &RiseRTCDomainCommittedCallMediaActivation) -> bool {
        self.owns_committed_call_media_activation(context)
            && !task_is_cancelled()
            && !self.state.with(|state| state.is_shutdown)
    }

    fn owns_committed_call_media_activation(
        &self,
        context: &RiseRTCDomainCommittedCallMediaActivation,
    ) -> bool {
        self.state.with(|state| {
            state
                .committed_call_media_activations
                .get(&context.call.call_id)
                .map(|activation| activation.task_id)
                == Some(context.task_id)
        })
    }

    fn finish_committed_call_media_activation(&self, call_id: &str, task_id: Uuid) {
        self.state.with(|state| {
            if state.committed_call_media_activations.get(call_id).map(|activation| activation.task_id)
                == Some(task_id)
            {
                state.committed_call_media_activations.remove(call_id);
            }
            if state.committed_call_media_tasks.get(call_id).map(|task| task.task_id) == Some(task_id) {
                state.committed_call_media_tasks.remove(call_id);
            }
        });
    }

    fn invalidate_committed_call_media_activation(
        &self,
        call_id: &str,
    ) -> Option<RiseRTCDomainCommittedCallMediaActivation> {
        self.state.with(|state| {
            let context = state.committed_call_media_activations.remove(call_id)?;
            if let Some(task) = state.committed_call_media_tasks.get(call_id) {
                task.task.cancel();
            }
            let pending = RiseRTCDomainPendingActivation {
                room_id: context.call.room_id.clone(),
                generation: context.activation_generation,
            };
            if state.pending_activation.as_ref() == Some(&pending) {
                state.pending_activation = None;
                state.advance_activation_generation();
            } else if state.activation_generation == context.activation_generation {
                state.advance_activation_generation();
            }
            Some(context)
        })
    }

    async fn cleanup_committed_call_media(
        &self,
        context: &RiseRTCDomainCommittedCallMediaActivation,
        leave_backend: bool,
    ) {
        let session = context.credentials.session_key();
        self.state.with(|state| {
            if state.active_session_key().as_ref() == Some(&session) {
                state.clear_ephemeral_session();
                state.latest_room_snapshot = None;
                state.media_connection_state = RiseRTCConnectionState::Idle;
            }
        });
        let _ = self.perform_media(RiseRTCMediaIntent::Disconnect { session }).await;
        if leave_backend {
            let _ = self.repository.leave_room(&context.credentials).await;
        }
        let another_owner_uses_room = self.state.with(|state| {
            state
                .active_credentials
                .as_ref()
                .is_some_and(|active| active.room_id == context.call.room_id)
                || state
                    .pending_activation
                    .as_ref()
                    .is_some_and(|pending| pending.room_id == context.call.room_id)
        });
        if !another_owner_uses_room {
            let _ = self.repository.release_room_cache(&context.call.room_id).await;
        }
    }

    /// runs the terminal call action, expected committed task id defaults to none
    async fn execute_terminal_call_action(
        &self,
        requested_endpoint: &RiseRTCEndpoint,
        call_id: &str,
        idempotency_key: &str,
        reason: Option<&str>,
        expected_committed_task_id: Option<Uuid>,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        self.validate_terminal_action_fence(call_id, expected_committed_task_id)?;
        let requested_cancel = requested_endpoint.method == RiseRTCEndpoints::cancel_call().method;
        if !requested_cancel {
            return self
                .repository
                .call_action(requested_endpoint, call_id, idempotency_key, reason)
                .await;
        }

        let mut must_end = self.cached_call_state(call_id) == Some(RiseRTCCallState::Accepted);
        let mut last_error: Option<anyhow::Error> = None;
        for _ in 0..3 {
            self.validate_terminal_action_fence(call_id, expected_committed_task_id)?;
            let endpoint = if must_end { RiseRTCEndpoints::end_call() } else { RiseRTCEndpoints::cancel_call() };
            let effective_key = if must_end {
                Self::derived_terminal_idempotency_key(idempotency_key, "accepted-end")
            } else {
                idempotency_key.to_owned()
            };
            let effective_reason =
                if must_end { Some(reason.unwrap_or("accepted_during_cancel")) } else { reason };
            match self
                .repository
                .call_action(endpoint, call_id, &effective_key, effective_reason)
                .await
            {
                Ok(call) => return Ok(call),
                Err(error) => last_error = Some(error),
            }

            self.validate_terminal_action_fence(call_id, expected_committed_task_id)?;
            let Ok(authoritative) = self.repository.get_call(call_id).await else {
                continue;
            };
            self.validate_terminal_action_fence(call_id, expected_committed_task_id)?;
            match RiseRTCCancelReconciliationPolicy::action(authoritative.state, must_end) {
                RiseRTCCancelReconciliationAction::AlreadyTerminal => return Ok(authoritative),
                RiseRTCCancelReconciliationAction::EndAccepted => must_end = true,
                RiseRTCCancelReconciliationAction::RetryCancel => {}
            }
        }
        Err(last_error.unwrap_or_else(|| CancellationError.into()))
    }

    fn validate_terminal_action_fence(
        &self,
        call_id: &str,
        expected_committed_task_id: Option<Uuid>,
    ) -> anyhow::Result<()> {
        let Some(expected_committed_task_id) = expected_committed_task_id else {
            return Ok(());
        };
        let owned = self.state.with(|state| {
            !state.is_shutdown
                && state.committed_call_media_activations.get(call_id).map(|activation| activation.task_id)
                    == Some(expected_committed_task_id)
        });
        if !owned || task_is_cancelled() {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    fn cached_call_state(&self, call_id: &str) -> Option<RiseRTCCallState> {
        self.state.with(|state| {
            state.latest_calls.as_ref().and_then(|latest_calls| {
                latest_calls
                    .calls
                    .iter()
                    .find(|stored| stored.call.call_id == call_id)
                    .map(|stored| stored.call.state)
            })
        })
    }

    fn derived_terminal_idempotency_key(base: &str, suffix: &str) -> String {
        let prefix: String = base.chars().take(96).collect();
        format!("{prefix}:{suffix}")
    }

    async fn perform_call_action(
        &self,
        endpoint: &RiseRTCEndpoint,
        call_id: &str,
        reason: Option<&str>,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseRTCDirectCall> {
        self.validate_active_account().await?;
        let committed_context = self.invalidate_committed_call_media_activation(call_id);
        let active_call_credentials = self.active_direct_call_credentials(call_id);
        let domain = self.this.upgrade();
        let cleanup_task: Option<JoinHandle<()>> = match (domain, active_call_credentials, committed_context) {
            (Some(domain), Some(credentials), _) => Some(RiseAccountRuntime::current().spawn(async move {
                domain.disconnect_media_without_leave(&credentials).await;
            })),
            (Some(domain), None, Some(context)) => Some(RiseAccountRuntime::current().spawn(async move {
                domain.cleanup_committed_call_media(&context, false).await;
            })),
            _ => None,
        };
        let call = match self
            .execute_terminal_call_action(endpoint, call_id, idempotency_key, reason, None)
            .await
        {
            Ok(call) => call,
            Err(error) => {
                if let Some(cleanup_task) = cleanup_task {
                    let _ = cleanup_task.await;
                }
                return Err(error);
            }
        };
        self.report_system_event(&call).await;
        if let Some(cleanup_task) = cleanup_task {
            let _ = cleanup_task.await;
        }
        Ok(call)
    }

    fn active_direct_call_credentials(&self, call_id: &str) -> Option<RiseRTCJoinCredentials> {
        self.state.with(|state| {
            let credentials = state.active_credentials.as_ref()?;
            if credentials.room_mode != RiseRTCRoomMode::DirectCall {
                return None;
            }
            let expected_room_id = state
                .latest_calls
                .as_ref()
                .and_then(|latest_calls| {
                    latest_calls
                        .calls
                        .iter()
                        .find(|stored| stored.call.call_id == call_id)
                        .map(|stored| stored.call.room_id.clone())
                })
                .unwrap_or_else(|| call_room_id(call_id));
            (credentials.room_id == expected_room_id).then(|| credentials.clone())
        })
    }

    pub async fn set_call_history_sink(&self, sink: RiseRTCCallHistorySink) {
        self.state.with(|state| state.call_history_sink = Some(sink));
    }

    fn start_call_observation(&self) {
        let repository = self.repository.clone();
        let domain = self.this.clone();
        let work = async move {
            let mut attempt: u64 = 0;
            while !task_is_cancelled() {
                match repository.observe_calls().await {
                    Ok(mut stream) => {
                        attempt = 0;
                        let mut failure = None;
                        while let Some(next) = stream.recv().await {
                            if task_is_cancelled() {
                                return;
                            }
                            match next {
                                Ok(snapshot) => {
                                    if let Some(domain) = domain.upgrade() {
                                        domain.accept(snapshot).await;
                                    }
                                }
                                Err(error) => {
                                    failure = Some(error);
                                    break;
                                }
                            }
                        }
                        if let Some(error) = failure {
                            if error.is::<CancellationError>() {
                                return;
                            }
                            if let Some(domain) = domain.upgrade() {
                                domain.publish_error(Self::error_code(&error)).await;
                            }
                        }
                    }
                    Err(error) if error.is::<CancellationError>() => return,
                    Err(error) => {
                        if let Some(domain) = domain.upgrade() {
                            domain.publish_error(Self::error_code(&error)).await;
                        }
                    }
                }
                let Some(this) = domain.upgrade() else {
                    return;
                };
                if this.state.with(|state| state.is_shutdown) {
                    return;
                }
                drop(this);
                attempt = (attempt + 1).min(Self::OBSERVATION_RETRY_CEILING);
                tokio::time::sleep(Duration::from_millis(
                    Self::OBSERVATION_RETRY_BACKOFF_MS * attempt,
                ))
                .await;
            }
        };
        self.state.with(|state| {
            let task = RiseRTCDomainTask::spawn(work);
            if let Some(previous) = state.call_observation_task.replace(task) {
                previous.cancel();
            }
        });
    }

    fn start_room_observation(&self, room_id: String) {
        let repository = self.repository.clone();
        let domain = self.this.clone();
        let work = async move {
            let mut attempt: u64 = 0;
            while !task_is_cancelled() {
                match repository.observe_room(&room_id).await {
                    Ok(mut stream) => {
                        attempt = 0;
                        let mut failure = None;
                        while let Some(next) = stream.recv().await {
                            if task_is_cancelled() {
                                return;
                            }
                            match next {
                                Ok(snapshot) => {
                                    if let Some(domain) = domain.upgrade() {
                                        domain.accept_expected_room_id(snapshot, &room_id).await;
                                    }
                                }
                                Err(error) => {
                                    failure = Some(error);
                                    break;
                                }
                            }
                        }
                        if let Some(error) = failure {
                            if error.is::<CancellationError>() {
                                return;
                            }
                            if let Some(domain) = domain.upgrade() {
                                domain.publish_error(Self::error_code(&error)).await;
                            }
                        }
                    }
                    Err(error) if error.is::<CancellationError>() => return,
                    Err(error) => {
                        if let Some(domain) = domain.upgrade() {
                            domain.publish_error(Self::error_code(&error)).await;
                        }
                    }
                }
                let Some(this) = domain.upgrade() else {
                    return;
                };
                if this.state.with(|state| state.is_shutdown) {
                    return;
                }
                drop(this);
                attempt = (attempt + 1).min(Self::OBSERVATION_RETRY_CEILING);
                tokio::time::sleep(Duration::from_millis(
                    Self::OBSERVATION_RETRY_BACKOFF_MS * attempt,
                ))
                .await;
            }
        };
        self.state.with(|state| {
            let task = RiseRTCDomainTask::spawn(work);
            if let Some(previous) = state.room_observation_task.replace(task) {
                previous.cancel();
            }
        });
    }

    fn start_media_observation(&self) {
        let Some(media_driver) = self.state.with(|state| {
            if let Some(previous) = state.media_event_task.take() {
                previous.cancel();
            }
            state.media_driver.clone()
        }) else {
            return;
        };
        let domain = self.this.clone();
        let work = async move {
            let mut events = media_driver.events().await;
            while let Some(event) = events.next().await {
                if task_is_cancelled() {
                    return;
                }
                if let Some(domain) = domain.upgrade() {
                    domain.accept_event(event).await;
                }
            }
        };
        self.state.with(|state| {
            let task = RiseRTCDomainTask::spawn(work);
            if let Some(previous) = state.media_event_task.replace(task) {
                previous.cancel();
            }
        });
    }

    async fn ensure_media_driver(&self) {
        let has_live_driver = self.state.with(|state| {
            state
                .media_driver
                .as_ref()
                .is_some_and(|driver| !driver.is_null_media_driver())
        });
        if has_live_driver {
            return;
        }
        self.state.with(|state| {
            if let Some(task) = state.media_event_task.take() {
                task.cancel();
            }
        });
        let driver = (self.media_driver_provider)().await;
        self.state.with(|state| state.media_driver = Some(driver));
        self.start_media_observation();
    }

    fn start_heartbeat(&self, credentials: RiseRTCJoinCredentials) {
        let interval = Duration::from_secs_f64(
            rise_canon::RiseTimingCanon::heartbeat("rtc_participant").client_interval_seconds(),
        );
        let domain = self.this.clone();
        let work = async move {
            let mut is_first_beat = true;
            while !task_is_cancelled() {
                if is_first_beat {
                    is_first_beat = false;
                } else {
                    tokio::time::sleep(interval).await;
                }
                if task_is_cancelled() {
                    return;
                }
                let Some(this) = domain.upgrade() else {
                    continue;
                };
                match this.heartbeat(&credentials).await {
                    Ok(()) => {}
                    Err(error) if error.is::<CancellationError>() => return,
                    Err(_) => this.schedule_authoritative_reconciliation(500).await,
                }
            }
        };
        self.state.with(|state| {
            let task = RiseRTCDomainTask::spawn(work);
            if let Some(previous) = state.heartbeat_task.replace(task) {
                previous.cancel();
            }
        });
    }

    async fn heartbeat(&self, credentials: &RiseRTCJoinCredentials) -> anyhow::Result<()> {
        let is_current = self.state.with(|state| {
            state.active_credentials.as_ref().is_some_and(|active| {
                active.connection_id == credentials.connection_id
                    && active.room_id == credentials.room_id
            })
        });
        if !is_current {
            return Ok(());
        }
        self.repository.heartbeat(credentials).await?;
        Ok(())
    }

    fn remember_call_identity(&self, identity: RiseRTCCallCryptoIdentity, call_id: &str) {
        self.state.with(|state| {
            if state.call_identities.contains_key(call_id) {
                return;
            }
            state.call_identities.insert(call_id.to_owned(), identity);
            state.call_identity_order.push(call_id.to_owned());
            while state.call_identity_order.len() > Self::CALL_IDENTITY_LIMIT {
                let evicted = state.call_identity_order.remove(0);
                state.call_identities.remove(&evicted);
                state.applied_call_keys.remove(&evicted);
                state.peer_half_snapshot_calls.remove(&evicted);
                state.peer_half_reads_in_flight.remove(&evicted);
            }
        });
    }

    /// derives the media key once the server routed both public halves and hands it to the frame cryptor
    /// runs from the call snapshot not the answer since the caller is already connected by then
    /// once per call, rotating a live key is bad
    async fn apply_call_encryption_if_possible(&self, snapshot: &RiseRTCCallRepositorySnapshot) {
        let Some(credentials) = self.state.with(|state| state.active_credentials.clone()) else {
            return;
        };
        for stored in &snapshot.calls {
            let call = &stored.call;
            let identity = self.state.with(|state| {
                if call.room_id != credentials.room_id || state.applied_call_keys.contains(&call.call_id) {
                    return None;
                }
                state.call_identities.get(&call.call_id).cloned()
            });
            let Some(identity) = identity else {
                continue;
            };

            let is_caller = call.caller_user_id == self.account_id;
            let mut peer = if is_caller {
                call.callee_e2ee_public_key.clone()
            } else {
                call.caller_e2ee_public_key.clone()
            };

            let needs_peer_read = peer.is_none()
                && call.state == RiseRTCCallState::Accepted
                && self.state.with(|state| !state.peer_half_snapshot_calls.contains(&call.call_id));
            if needs_peer_read {
                let starts_read = self.state.with(|state| {
                    if state.peer_half_reads_in_flight.contains(&call.call_id) {
                        return false;
                    }
                    state.peer_half_reads_in_flight.insert(call.call_id.clone());
                    true
                });
                if !starts_read {
                    continue;
                }
                let fresh = self.repository.get_call(&call.call_id).await.ok();
                self.state.with(|state| state.peer_half_reads_in_flight.remove(&call.call_id));
                let Some(fresh) = fresh else {
                    self.schedule_authoritative_reconciliation(1000).await;
                    continue;
                };
                let still_pending = self.state.with(|state| {
                    state.peer_half_snapshot_calls.insert(call.call_id.clone());
                    state
                        .active_credentials
                        .as_ref()
                        .is_some_and(|active| active.room_id == credentials.room_id)
                        && !state.applied_call_keys.contains(&call.call_id)
                });
                if !still_pending {
                    continue;
                }
                peer = if is_caller {
                    fresh.callee_e2ee_public_key
                } else {
                    fresh.caller_e2ee_public_key
                };
            }

            let Some(peer) = peer else {
                if call.state != RiseRTCCallState::Accepted {
                    continue;
                }
                Self::log(format!(
                    "peer sent no key half; encryption disabled for call_id={}",
                    call.call_id
                ))
                .await;
                if let Some(media_driver) = self.media_driver()
                    && media_driver
                        .perform(RiseRTCMediaIntent::SetEncryptionKey {
                            session: credentials.session_key(),
                            key: None,
                        })
                        .await
                        .is_err()
                {
                    continue;
                }
                self.state.with(|state| state.applied_call_keys.insert(call.call_id.clone()));
                continue;
            };

            let Some(material) = RiseRTCCallCrypto::derive(&identity, &peer, &call.call_id) else {
                Self::log(format!(
                    "call key agreement rejected the peer half call_id={}",
                    call.call_id
                ))
                .await;
                continue;
            };
            if let Some(media_driver) = self.media_driver()
                && let Err(error) = media_driver
                    .perform(RiseRTCMediaIntent::SetEncryptionKey {
                        session: credentials.session_key(),
                        key: Some(material.media_key.clone()),
                    })
                    .await
            {
                Self::log(format!(
                    "call key delivery failed call_id={} {}",
                    call.call_id,
                    Self::error_summary(&error)
                ))
                .await;
                continue;
            }
            self.state.with(|state| state.applied_call_keys.insert(call.call_id.clone()));
            let verification_emojis = material.verification_emojis;
            let call_id = call.call_id.clone();
            Self::on_main(move |cx| {
                RiseRTCPresentation::shared(cx).update(cx, |presentation, cx| {
                    presentation.publish_verification_emojis(verification_emojis, &call_id, cx);
                });
            })
            .await;
        }
    }

    async fn ingest_call_history_if_needed(&self, call: &RiseRTCDirectCall) {
        let Some(call_history_sink) = self.state.with(|state| state.call_history_sink.clone()) else {
            return;
        };
        if trimming_whitespaces_and_newlines(&call.chat_id).is_empty()
            || !matches!(call.media_mode, RiseRTCCallMediaMode::Audio | RiseRTCCallMediaMode::Video)
        {
            return;
        }
        let ended_at_ms = call.ended_at_ms.unwrap_or(call.updated_at_ms);
        if call.created_at_ms <= 0 || ended_at_ms <= 0 {
            return;
        }
        let is_new = self.state.with(|state| {
            if state.ingested_call_history_ids.contains(&call.call_id) {
                return false;
            }
            state.ingested_call_history_ids.insert(call.call_id.clone());
            state.ingested_call_history_order.push(call.call_id.clone());
            if state.ingested_call_history_order.len() > Self::CALL_HISTORY_IDENTITY_LIMIT {
                let evicted = state.ingested_call_history_order.remove(0);
                state.ingested_call_history_ids.remove(&evicted);
            }
            true
        });
        if !is_new {
            return;
        }
        call_history_sink(RiseCallHistoryRecord {
            call_id: call.call_id.clone(),
            chat_id: call.chat_id.clone(),
            caller_user_id: call.caller_user_id.clone(),
            callee_user_id: call.callee_user_id.clone(),
            media_mode: call.media_mode.wire_value().to_owned(),
            state: call.state.wire_value().to_owned(),
            started_at_ms: call.created_at_ms,
            answered_at_ms: call.answered_at_ms,
            ended_at_ms,
            ended_by_user_id: call.ended_by_user_id.clone(),
            end_reason: call.end_reason.clone(),
        })
        .await;
    }

    async fn accept(&self, snapshot: RiseRTCCallRepositorySnapshot) {
        let accepts = self.state.with(|state| {
            if state.is_shutdown || snapshot.account_id != self.account_id {
                return false;
            }
            state.latest_calls = Some(snapshot.clone());
            true
        });
        if !accepts {
            return;
        }
        self.apply_call_encryption_if_possible(&snapshot).await;
        let reported = snapshot.clone();
        Self::on_main(move |cx| {
            for stored in &reported.calls {
                if let Some(envelope) =
                    RiseRTCSystemIncomingEnvelope::from_ringing(stored, &reported.account_id)
                {
                    RiseRTCCallSystemHooks::report_realtime_incoming_call(envelope, cx);
                }
                let Some(event) = RiseRTCCallSystemEventMapper::event(stored.call.state) else {
                    continue;
                };
                RiseRTCCallSystemHooks::report_remote_call_event(&stored.call.call_id, event, cx);
            }
        })
        .await;
        if self.is_active() {
            let published = snapshot.clone();
            Self::on_main(move |cx| {
                RiseRTCPresentation::shared(cx)
                    .update(cx, |presentation, cx| presentation.publish_calls(published, cx));
            })
            .await;
        }
        for stored in snapshot.calls.iter().filter(|stored| stored.call.state.is_terminal()) {
            self.ingest_call_history_if_needed(&stored.call).await;
            if let Some(context) = self.invalidate_committed_call_media_activation(&stored.call.call_id) {
                self.cleanup_committed_call_media(&context, false).await;
            }
        }
        let Some(credentials) = self.state.with(|state| state.active_credentials.clone()) else {
            return;
        };
        if credentials.room_mode != RiseRTCRoomMode::DirectCall {
            return;
        }
        let Some(active_call) = snapshot
            .calls
            .iter()
            .find(|stored| stored.call.room_id == credentials.room_id)
        else {
            return;
        };
        self.report_media_connected_if_ready(&active_call.call, &credentials.session_key()).await;
        if active_call.call.state.is_terminal() {
            self.disconnect_media_without_leave(&credentials).await;
        }
    }

    async fn accept_expected_room_id(&self, snapshot: RiseRTCRoomRepositorySnapshot, expected_room_id: &str) {
        let accepts = self.state.with(|state| {
            if state.is_shutdown
                || snapshot.account_id != self.account_id
                || state.active_room_id.as_deref() != Some(expected_room_id)
            {
                return false;
            }
            state.latest_room_snapshot = Some(snapshot.clone());
            true
        });
        if !accepts {
            return;
        }
        self.apply_server_moderation(self.latest_self_participant()).await;
        self.apply_listener_volumes(false).await;
        if self.is_active() {
            Self::on_main(move |cx| {
                RiseRTCPresentation::shared(cx)
                    .update(cx, |presentation, cx| presentation.publish_room(snapshot, cx));
            })
            .await;
        }
    }

    /// local half of a moderator mute or deafen
    /// the sfu already refuses publish and subscribe so this only stops this device capturing
    /// and silences playout, not the same as set_microphone since this is the servers decision
    async fn apply_server_moderation(&self, participant: Option<RiseRTCParticipant>) {
        let transition = self.state.with(|state| {
            let Some(credentials) = state.active_credentials.as_ref() else {
                state.applied_server_moderation = None;
                return None;
            };
            let applied = RiseRTCDomainAppliedServerModeration {
                session: credentials.session_key(),
                silenced: participant.as_ref().is_some_and(RiseRTCParticipant::is_server_silenced),
                deafened: participant.as_ref().is_some_and(|participant| participant.server_deafened),
                camera_blocked: participant.as_ref().is_some_and(|participant| participant.server_camera_blocked),
                screen_share_blocked: participant
                    .as_ref()
                    .is_some_and(|participant| participant.server_screen_share_blocked),
            };
            if state.applied_server_moderation.as_ref() == Some(&applied) {
                return None;
            }
            let previous = state.applied_server_moderation.replace(applied.clone());
            Some((applied, previous))
        });
        let Some((applied, previous)) = transition else {
            return;
        };
        let media_driver = self.media_driver();
        if applied.silenced
            && let Some(media_driver) = media_driver.as_ref()
        {
            let _ = media_driver
                .perform(RiseRTCMediaIntent::SetMicrophone {
                    session: applied.session.clone(),
                    enabled: false,
                })
                .await;
        }
        if applied.camera_blocked
            && let Some(media_driver) = media_driver.as_ref()
        {
            let _ = media_driver
                .perform(RiseRTCMediaIntent::SetCamera {
                    session: applied.session.clone(),
                    enabled: false,
                    position: RiseRTCCameraPosition::Front,
                })
                .await;
        }
        if applied.screen_share_blocked
            && let Some(media_driver) = media_driver.as_ref()
        {
            let _ = media_driver
                .perform(RiseRTCMediaIntent::SetScreenShare {
                    session: applied.session.clone(),
                    enabled: false,
                    include_device_audio: false,
                    quality: RiseRTCScreenShareQuality::Balanced,
                })
                .await;
        }
        if applied.deafened {
            if let Some(media_driver) = media_driver.as_ref() {
                let _ = media_driver
                    .perform(RiseRTCMediaIntent::SetRemotePlayoutMuted {
                        session: applied.session.clone(),
                        muted: true,
                    })
                    .await;
            }
        } else if previous.as_ref().is_some_and(|previous| {
            previous.session == applied.session && previous.deafened
        }) && !participant.as_ref().is_some_and(|participant| participant.deafened)
        {
            if let Some(media_driver) = media_driver.as_ref() {
                let _ = media_driver
                    .perform(RiseRTCMediaIntent::SetRemotePlayoutMuted {
                        session: applied.session.clone(),
                        muted: false,
                    })
                    .await;
            }
        }
    }

    async fn accept_event(&self, event: RiseRTCMediaEvent) {
        if self.state.with(|state| state.is_shutdown) {
            return;
        }
        if let Some(session) = event.session_key()
            && self.state.with(|state| state.active_session_key()).as_ref() != Some(session)
        {
            return;
        }
        let outcome: anyhow::Result<()> = async {
            if let Some(outcome) = self.repository.accept_media_event(&event).await? {
                if let Some(reaction) = outcome.reaction
                    && self.is_active()
                {
                    Self::on_main(move |cx| {
                        RiseRTCPresentation::shared(cx)
                            .update(cx, |presentation, cx| presentation.publish_reaction(reaction, cx));
                    })
                    .await;
                }
                if let Some(eviction) = outcome.eviction {
                    self.evict(&eviction.room_id, eviction.reason).await;
                    return Ok(());
                }
            }
            let owns = |session: &RiseRTCMediaSessionKey| {
                self.state.with(|state| state.active_session_key().as_ref() == Some(session))
            };
            match event {
                RiseRTCMediaEvent::Connected { session } => {
                    if !owns(&session) {
                        return Ok(());
                    }
                    self.state.with(|state| state.media_connection_state = RiseRTCConnectionState::Connected);
                    self.publish_connection(RiseRTCConnectionState::Connected).await;
                    self.report_media_connected_if_accepted(&session).await;
                }
                RiseRTCMediaEvent::Reconnecting { session } => {
                    if !owns(&session) {
                        return Ok(());
                    }
                    self.state.with(|state| state.media_connection_state = RiseRTCConnectionState::Reconnecting);
                    self.publish_connection(RiseRTCConnectionState::Reconnecting).await;
                }
                RiseRTCMediaEvent::Reconnected { session } => {
                    if !owns(&session) {
                        return Ok(());
                    }
                    self.state.with(|state| state.media_connection_state = RiseRTCConnectionState::Connected);
                    self.publish_connection(RiseRTCConnectionState::Connected).await;
                    self.apply_listener_volumes(true).await;
                    self.schedule_authoritative_reconciliation(250).await;
                }
                RiseRTCMediaEvent::Disconnected { session, reason } => {
                    let code = reason.unwrap_or_else(|| RiseRTCMediaDisconnectCode::LINK_LOST.to_owned());
                    self.handle_terminal_media_disconnect(&session, &code).await;
                }
                RiseRTCMediaEvent::Failure { session, code } => {
                    if let Some(session) = session.as_ref()
                        && !owns(session)
                    {
                        return Ok(());
                    }
                    let is_connecting = self.state.with(|state| {
                        state.media_connection_state == RiseRTCConnectionState::Connecting
                    });
                    match session {
                        Some(session) if is_connecting => {
                            self.handle_terminal_media_disconnect(&session, &code).await;
                        }
                        _ => self.publish_error(code).await,
                    }
                }
                RiseRTCMediaEvent::LocalScreenShareEnded { session } => {
                    if !owns(&session) {
                        return Ok(());
                    }
                    self.clear_screen_share_after_media_loss(&session).await;
                }
                RiseRTCMediaEvent::LocalMicrophoneLost { session } => {
                    if !owns(&session) {
                        return Ok(());
                    }
                    self.clear_microphone_after_media_loss(&session).await;
                }
                RiseRTCMediaEvent::LocalPlayoutLost { session } => {
                    if !owns(&session) {
                        return Ok(());
                    }
                    self.publish_nonfatal_media_error(
                        Self::SPEAKER_UNAVAILABLE_CODE,
                        Some("call sound device did not open".to_owned()),
                    )
                    .await;
                }
                RiseRTCMediaEvent::LocalBroadcastStarted { session } => {
                    if !owns(&session) {
                        return Ok(());
                    }
                    let _ = self
                        .set_screen_share(true, false, RiseRTCScreenShareQuality::Balanced)
                        .await;
                }
                RiseRTCMediaEvent::LocalCameraPaused { session } => {
                    let Some(credentials) = self.state.with(|state| state.active_credentials.clone()) else {
                        return Ok(());
                    };
                    if credentials.session_key() != session {
                        return Ok(());
                    }
                    let _ = self
                        .repository
                        .update_self_media(&credentials, None, Some(false), None, None)
                        .await;
                }
                RiseRTCMediaEvent::LocalCameraResumed { session } => {
                    let Some(credentials) = self.state.with(|state| state.active_credentials.clone()) else {
                        return Ok(());
                    };
                    if credentials.session_key() != session {
                        return Ok(());
                    }
                    let _ = self
                        .repository
                        .update_self_media(&credentials, None, Some(true), None, None)
                        .await;
                }
                RiseRTCMediaEvent::LocalCameraResumeRequested { session } => {
                    if !owns(&session) {
                        return Ok(());
                    }
                    let _ = self.set_camera(true, RiseRTCCameraPosition::Front).await;
                }
                RiseRTCMediaEvent::ParticipantJoined { .. } | RiseRTCMediaEvent::ParticipantLeft { .. } => {
                    self.schedule_authoritative_reconciliation(250).await;
                }
                RiseRTCMediaEvent::Data { .. } => {}
            }
            Ok(())
        }
        .await;
        if outcome.is_err() {
            self.schedule_authoritative_reconciliation(250).await;
        }
    }

    async fn report_media_connected_if_accepted(&self, session: &RiseRTCMediaSessionKey) {
        let call = self.state.with(|state| {
            let credentials = state.active_credentials.as_ref()?;
            if credentials.session_key() != *session || credentials.room_mode != RiseRTCRoomMode::DirectCall {
                return None;
            }
            state.latest_calls.as_ref().and_then(|latest_calls| {
                latest_calls
                    .calls
                    .iter()
                    .find(|stored| stored.call.room_id == session.room_id)
                    .map(|stored| stored.call.clone())
            })
        });
        let Some(call) = call else {
            return;
        };
        self.report_media_connected_if_ready(&call, session).await;
    }

    async fn report_media_connected_if_ready(&self, call: &RiseRTCDirectCall, session: &RiseRTCMediaSessionKey) {
        let is_ready = self.state.with(|state| {
            state.active_credentials.as_ref().is_some_and(|credentials| {
                credentials.session_key() == *session && credentials.room_mode == RiseRTCRoomMode::DirectCall
            }) && call.room_id == session.room_id
                && RiseRTCCallActivationGate::is_ready(call.state, state.media_connection_state)
        });
        if !is_ready {
            return;
        }
        let call_id = call.call_id.clone();
        Self::on_main(move |cx| {
            RiseRTCCallSystemHooks::report_media_connected(&call_id, cx);
        })
        .await;
    }

    async fn report_system_event(&self, call: &RiseRTCDirectCall) {
        let Some(event) = RiseRTCCallSystemEventMapper::event(call.state) else {
            return;
        };
        let call_id = call.call_id.clone();
        Self::on_main(move |cx| {
            RiseRTCCallSystemHooks::report_remote_call_event(&call_id, event, cx);
        })
        .await;
    }

    async fn handle_terminal_media_disconnect(&self, session: &RiseRTCMediaSessionKey, code: &str) {
        let Some(credentials) = self.state.with(|state| state.active_credentials.clone()) else {
            return;
        };
        if credentials.session_key() != *session {
            return;
        }
        if credentials.room_mode == RiseRTCRoomMode::DirectCall {
            let cached = self.state.with(|state| {
                state.latest_calls.as_ref().and_then(|latest_calls| {
                    latest_calls
                        .calls
                        .iter()
                        .find(|stored| stored.call.room_id == credentials.room_id)
                        .map(|stored| stored.call.clone())
                })
            });
            if let Some(cached) = cached {
                let call = self.repository.get_call(&cached.call_id).await.ok();
                let still_owned = self.state.with(|state| {
                    !state.is_shutdown && state.active_session_key().as_ref() == Some(session)
                });
                if !still_owned {
                    return;
                }
                if let Some(call) = call.as_ref()
                    && call.state.is_terminal()
                {
                    self.report_system_event(call).await;
                    self.disconnect_media_without_leave(&credentials).await;
                    return;
                }
                if RiseRTCConnectionWaitPolicy::is_transient_media_failure(code) {
                    self.recover_direct_call_media(call.unwrap_or(cached), &credentials, code).await;
                    return;
                }
            }
        }
        if credentials.room_mode != RiseRTCRoomMode::DirectCall
            && let Some(reason) = RiseRTCEvictionReason::from_media_code(code)
        {
            self.evict(&credentials.room_id, reason).await;
            return;
        }
        self.state.with(|state| {
            state.advance_activation_generation();
            state.pending_activation = None;
            state.clear_ephemeral_session();
            state.media_connection_state = RiseRTCConnectionState::Failed;
        });
        let leaves_backend_room = credentials.room_mode != RiseRTCRoomMode::DirectCall;
        let departure = leaves_backend_room.then(|| self.start_departure(&credentials));
        if let Some(media_driver) = self.media_driver() {
            let _ = media_driver
                .perform(RiseRTCMediaIntent::Disconnect { session: session.clone() })
                .await;
        }
        if let Some(departure) = departure {
            departure.await;
        }
        self.publish_error(code.to_owned()).await;
    }

    async fn recover_direct_call_media(
        &self,
        call: RiseRTCDirectCall,
        credentials: &RiseRTCJoinCredentials,
        code: &str,
    ) {
        let self_participant = self.latest_self_participant();
        let desired_media = self_participant
            .as_ref()
            .map(RiseRTCDesiredMediaState::from_participant)
            .unwrap_or_else(|| RiseRTCDesiredMediaState::from_call(&call));
        let generation = self.state.with(|state| {
            state.advance_activation_generation();
            state.pending_activation = None;
            state.clear_ephemeral_session();
            state.media_connection_state = RiseRTCConnectionState::Reconnecting;
            state.activation_generation
        });
        if let Some(media_driver) = self.media_driver() {
            let _ = media_driver
                .perform(RiseRTCMediaIntent::Disconnect { session: credentials.session_key() })
                .await;
        }
        self.publish_connection(RiseRTCConnectionState::Reconnecting).await;
        Self::log(format!(
            "call media dropped, reconnecting call={} code={}",
            call.call_id, code
        ))
        .await;
        let kind = if call.caller_user_id == self.account_id {
            RiseRTCCommittedCallMediaKind::OutgoingStart
        } else {
            RiseRTCCommittedCallMediaKind::AcceptedIncoming
        };
        let restores_deafen = self_participant.is_some_and(|participant| participant.deafened);
        self.schedule_committed_call_media_activation(
            call,
            credentials.clone(),
            desired_media,
            generation,
            kind,
            Some(code.to_owned()),
            restores_deafen,
        );
    }

    fn schedule_authoritative_reconciliation(&self, delay_milliseconds: u64) -> BoxFuture<'_, ()> {
        async move { self.schedule_authoritative_reconciliation_now(delay_milliseconds) }.boxed()
    }

    fn schedule_authoritative_reconciliation_now(&self, delay_milliseconds: u64) {
        let plan = self.state.with(|state| {
            if !state.is_active {
                return None;
            }
            let room_id = state.active_room_id.clone()?;
            if let Some(task) = state.reconciliation_task.take() {
                task.cancel();
            }
            let direct_call_id = state.latest_calls.as_ref().and_then(|calls| {
                calls
                    .calls
                    .iter()
                    .find(|stored| stored.call.room_id == room_id)
                    .map(|stored| stored.call.call_id.clone())
            });
            Some((room_id, direct_call_id))
        });
        let Some((room_id, direct_call_id)) = plan else {
            return;
        };
        let repository = self.repository.clone();
        let domain = self.this.clone();
        let work = async move {
            if delay_milliseconds > 0 {
                tokio::time::sleep(Duration::from_millis(delay_milliseconds)).await;
            }
            if task_is_cancelled() {
                return;
            }
            if let Some(domain) = domain.upgrade() {
                domain.publish_refreshing(true).await;
            }
            let refreshing = RiseRTCDomainRefreshingRelease { domain: domain.clone() };
            let outcome: anyhow::Result<bool> = async {
                if let Some(direct_call_id) = &direct_call_id {
                    let call = repository.get_call(direct_call_id).await?;
                    if task_is_cancelled() {
                        return Err(CancellationError.into());
                    }
                    if call.state.is_terminal() {
                        return Ok(true);
                    }
                }
                repository.refresh_room(&room_id).await
            }
            .await;
            drop(refreshing);
            match outcome {
                Ok(stored) => {
                    if stored || task_is_cancelled() {
                        return;
                    }
                    if let Some(domain) = domain.upgrade() {
                        domain.schedule_authoritative_reconciliation(1_000).await;
                    }
                }
                Err(error) if error.is::<CancellationError>() => {}
                Err(error) => {
                    if let Some(domain) = domain.upgrade() {
                        domain.publish_nonfatal_media_error(&Self::error_code(&error), None).await;
                    }
                }
            }
        };
        self.state.with(|state| {
            let task = RiseRTCDomainTask::spawn(work);
            if let Some(previous) = state.reconciliation_task.replace(task) {
                previous.cancel();
            }
        });
    }

    async fn revalidate_active_calls(&self) {
        let call_ids: Vec<String> = self.state.with(|state| {
            state
                .latest_calls
                .as_ref()
                .map(|calls| {
                    calls
                        .calls
                        .iter()
                        .filter_map(|stored| match stored.call.state {
                            RiseRTCCallState::Ringing | RiseRTCCallState::Accepted => {
                                Some(stored.call.call_id.clone())
                            }
                            RiseRTCCallState::Unspecified
                            | RiseRTCCallState::Declined
                            | RiseRTCCallState::Cancelled
                            | RiseRTCCallState::TimedOut
                            | RiseRTCCallState::Ended => None,
                        })
                        .take(8)
                        .collect()
                })
                .unwrap_or_default()
        });
        let repository = &self.repository;
        futures::future::join_all(call_ids.iter().map(|call_id| async move {
            let _ = repository.get_call(call_id).await;
        }))
        .await;
    }

    async fn disconnect_media_without_leave(&self, credentials: &RiseRTCJoinCredentials) {
        let session = credentials.session_key();
        let owns_session = self.state.with(|state| {
            if state.active_session_key().as_ref() != Some(&session) {
                return false;
            }
            state.advance_activation_generation();
            state.pending_activation = None;
            state.clear_ephemeral_session();
            state.latest_room_snapshot = None;
            state.media_connection_state = RiseRTCConnectionState::Idle;
            true
        });
        if !owns_session {
            return;
        }
        if let Some(media_driver) = self.media_driver() {
            let _ = media_driver.perform(RiseRTCMediaIntent::Disconnect { session }).await;
        }
        let _ = self.repository.release_room_cache(&credentials.room_id).await;
        self.clear_active_room_presentation().await;
    }

    fn clear_ephemeral_session(&self) {
        self.state.with(RiseRTCDomainState::clear_ephemeral_session);
    }

    async fn prepare_activation(&self, room_id: &str) -> u64 {
        let current = self.state.with(|state| {
            state
                .active_credentials
                .as_ref()
                .filter(|current| current.room_id != room_id)
                .map(|current| current.room_id.clone())
        });
        if let Some(current_room_id) = current {
            self.leave_room_expected_room_id(&current_room_id).await;
        }
        self.begin_activation(room_id)
    }

    fn begin_activation(&self, room_id: &str) -> u64 {
        self.state.with(|state| {
            let generation = state.advance_activation_generation();
            state.pending_activation =
                Some(RiseRTCDomainPendingActivation { room_id: room_id.to_owned(), generation });
            generation
        })
    }

    fn is_pending_activation(&self, room_id: &str, generation: u64) -> bool {
        self.state.with(|state| {
            state.activation_generation == generation
                && state.pending_activation.as_ref().is_some_and(|pending| {
                    pending.room_id == room_id && pending.generation == generation
                })
        })
    }

    fn is_active_session(&self, session: &RiseRTCMediaSessionKey, generation: u64) -> bool {
        self.state.with(|state| {
            state.activation_generation == generation
                && state.active_session_key().as_ref() == Some(session)
        })
    }

    fn advance_activation_generation(&self) -> u64 {
        self.state.with(RiseRTCDomainState::advance_activation_generation)
    }

    /// every backend leave_room goes through here so a following join can find it
    /// task is kept not awaited by everyone, only a join into the same room has to wait
    fn start_departure(&self, credentials: &RiseRTCJoinCredentials) -> Shared<BoxFuture<'static, ()>> {
        let room_id = credentials.room_id.clone();
        let token = Uuid::new_v4();
        let repository = self.repository.clone();
        let domain = self.this.clone();
        let credentials = credentials.clone();
        let completed_room_id = room_id.clone();
        let handle = RiseAccountRuntime::current().spawn(async move {
            let _ = repository.leave_room(&credentials).await;
            if let Some(domain) = domain.upgrade() {
                domain.complete_departure(&completed_room_id, token);
            }
        });
        let task = async move {
            let _ = handle.await;
        }
        .boxed()
        .shared();
        self.state.with(|state| {
            state
                .departures
                .insert(room_id, RiseRTCDomainDeparture { token, task: task.clone() });
        });
        task
    }

    fn complete_departure(&self, room_id: &str, token: Uuid) {
        self.state.with(|state| {
            if state.departures.get(room_id).map(|departure| departure.token) != Some(token) {
                return;
            }
            state.departures.remove(room_id);
        });
    }

    async fn await_departure(&self, room_id: &str) {
        let Some(task) =
            self.state.with(|state| state.departures.get(room_id).map(|departure| departure.task.clone()))
        else {
            return;
        };
        task.await;
    }

    async fn rollback_abandoned_join(&self, credentials: &RiseRTCJoinCredentials) {
        let departure = self.start_departure(credentials);
        if let Some(media_driver) = self.media_driver() {
            let _ = media_driver
                .perform(RiseRTCMediaIntent::Disconnect { session: credentials.session_key() })
                .await;
        }
        departure.await;
        let another_session_owns_room = self.state.with(|state| {
            state
                .active_credentials
                .as_ref()
                .is_some_and(|active| active.room_id == credentials.room_id)
                || state
                    .pending_activation
                    .as_ref()
                    .is_some_and(|pending| pending.room_id == credentials.room_id)
        });
        if !another_session_owns_room {
            let _ = self.repository.release_room_cache(&credentials.room_id).await;
        }
    }

    fn latest_self_participant(&self) -> Option<RiseRTCParticipant> {
        self.state.with(|state| {
            state.latest_room_snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .participants
                    .iter()
                    .find(|participant| participant.user_id == self.account_id)
                    .cloned()
            })
        })
    }

    fn media_driver(&self) -> Option<Arc<dyn RiseRTCMediaDriving>> {
        self.state.with(|state| state.media_driver.clone())
    }

    fn advance_microphone_mutation_generation(&self) -> u64 {
        self.state.with(RiseRTCDomainState::advance_microphone_mutation_generation)
    }

    fn is_current_microphone_mutation(&self, generation: u64, session: &RiseRTCMediaSessionKey) -> bool {
        self.state.with(|state| {
            state.microphone_mutation_generation == generation
                && state.active_session_key().as_ref() == Some(session)
        })
    }

    async fn acquire_microphone_mutation_lane(&self) {
        let waiter = self.state.with(|state| {
            if !state.microphone_mutation_lane_busy {
                state.microphone_mutation_lane_busy = true;
                return None;
            }
            let (sender, receiver) = oneshot::channel();
            state.microphone_mutation_waiters.push(sender);
            Some(receiver)
        });
        if let Some(waiter) = waiter {
            let _ = waiter.await;
        }
    }

    fn release_microphone_mutation_lane(&self) {
        self.state.with(|state| {
            Self::hand_over_lane(
                &mut state.microphone_mutation_waiters,
                &mut state.microphone_mutation_lane_busy,
            );
        });
    }

    fn advance_camera_mutation_generation(&self) -> u64 {
        self.state.with(RiseRTCDomainState::advance_camera_mutation_generation)
    }

    fn is_current_camera_mutation(&self, generation: u64, session: &RiseRTCMediaSessionKey) -> bool {
        self.state.with(|state| {
            state.camera_mutation_generation == generation
                && state.active_session_key().as_ref() == Some(session)
        })
    }

    async fn acquire_camera_mutation_lane(&self) {
        let waiter = self.state.with(|state| {
            if !state.camera_mutation_lane_busy {
                state.camera_mutation_lane_busy = true;
                return None;
            }
            let (sender, receiver) = oneshot::channel();
            state.camera_mutation_waiters.push(sender);
            Some(receiver)
        });
        if let Some(waiter) = waiter {
            let _ = waiter.await;
        }
    }

    fn release_camera_mutation_lane(&self) {
        self.state.with(|state| {
            Self::hand_over_lane(&mut state.camera_mutation_waiters, &mut state.camera_mutation_lane_busy);
        });
    }

    async fn acquire_screen_share_mutation_lane(&self) {
        let waiter = self.state.with(|state| {
            if !state.screen_share_mutation_lane_busy {
                state.screen_share_mutation_lane_busy = true;
                return None;
            }
            let (sender, receiver) = oneshot::channel();
            state.screen_share_mutation_waiters.push(sender);
            Some(receiver)
        });
        if let Some(waiter) = waiter {
            let _ = waiter.await;
        }
    }

    fn release_screen_share_mutation_lane(&self) {
        self.state.with(|state| {
            Self::hand_over_lane(
                &mut state.screen_share_mutation_waiters,
                &mut state.screen_share_mutation_lane_busy,
            );
        });
    }

    fn advance_screen_share_mutation_generation(&self) -> u64 {
        self.state.with(RiseRTCDomainState::advance_screen_share_mutation_generation)
    }

    fn is_current_screen_share_mutation(
        &self,
        generation: u64,
        session: &RiseRTCMediaSessionKey,
    ) -> bool {
        self.state.with(|state| {
            state.screen_share_mutation_generation == generation
                && state.active_session_key().as_ref() == Some(session)
        })
    }

    /// guard after every await of a mutation, a newer intent or another
    /// session supersedes this one and a cancelled task stops here
    fn ensure_current_microphone_mutation(
        &self,
        generation: u64,
        session: &RiseRTCMediaSessionKey,
    ) -> anyhow::Result<()> {
        if !self.is_current_microphone_mutation(generation, session) || task_is_cancelled() {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    fn ensure_current_camera_mutation(
        &self,
        generation: u64,
        session: &RiseRTCMediaSessionKey,
    ) -> anyhow::Result<()> {
        if !self.is_current_camera_mutation(generation, session) || task_is_cancelled() {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    fn ensure_current_screen_share_mutation(
        &self,
        generation: u64,
        session: &RiseRTCMediaSessionKey,
    ) -> anyhow::Result<()> {
        if !self.is_current_screen_share_mutation(generation, session) || task_is_cancelled() {
            return Err(CancellationError.into());
        }
        Ok(())
    }

    /// performs the media intent, no driver yet is a no op not a failure
    async fn perform_media(&self, intent: RiseRTCMediaIntent) -> anyhow::Result<()> {
        let Some(media_driver) = self.media_driver() else {
            return Ok(());
        };
        media_driver.perform(intent).await
    }

    /// lane goes to the oldest waiter still waiting, a dropped waiter
    /// can vanish and handing it the lane would hold it forever
    fn hand_over_lane(waiters: &mut Vec<oneshot::Sender<()>>, busy: &mut bool) {
        while !waiters.is_empty() {
            if waiters.remove(0).send(()).is_ok() {
                return;
            }
        }
        *busy = false;
    }

    fn resume_reconnect_waiter(&self, waiter_id: Uuid) {
        self.state.with(|state| {
            if let Some(waiter) = state.reconnect_waiters.remove(&waiter_id) {
                let _ = waiter.send(());
            }
        });
    }

    fn resume_reconnect_waiters(&self) {
        self.state.with(Self::resume_reconnect_waiters_in);
    }

    fn resume_reconnect_waiters_in(state: &mut RiseRTCDomainState) {
        for (_, waiter) in std::mem::take(&mut state.reconnect_waiters) {
            let _ = waiter.send(());
        }
    }

    async fn require_credentials(&self) -> anyhow::Result<RiseRTCJoinCredentials> {
        self.validate_active_account().await?;
        self.state
            .with(|state| state.active_credentials.clone())
            .ok_or_else(|| RiseRTCStateError::MissingJoinCredentials.into())
    }

    async fn validate_active_account(&self) -> anyhow::Result<()> {
        if !self.is_live() {
            return Err(RiseRTCStateError::MissingActiveSession.into());
        }
        let account_id = self.account_id.clone();
        let domain = self.this.clone();
        let valid = Self::on_main(move |cx| {
            SaiAccountCacheScope::is_current_account(Some(&account_id))
                && RiseRTCDomainRegistry::shared(cx).read(cx).is_active(&domain)
        })
        .await
        .unwrap_or(false);
        if !valid {
            return Err(RiseRTCStateError::StaleAccount.into());
        }
        Ok(())
    }

    fn is_live(&self) -> bool {
        self.state.with(|state| !state.is_shutdown && state.is_active)
    }

    fn is_active(&self) -> bool {
        self.state.with(|state| state.is_active)
    }

    async fn publish_connection(&self, connection_state: RiseRTCConnectionState) {
        if !self.is_active() {
            return;
        }
        Self::on_main(move |cx| {
            RiseRTCPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_connection_state(connection_state, cx);
            });
        })
        .await;
    }

    async fn publish_refreshing(&self, refreshing: bool) {
        if !self.is_active() {
            return;
        }
        Self::on_main(move |cx| {
            RiseRTCPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.publish_refreshing(refreshing, cx));
        })
        .await;
    }

    async fn publish_error(&self, code: String) {
        if !self.is_active() {
            return;
        }
        Self::on_main(move |cx| {
            RiseRTCPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.publish_error_code(Some(code), cx));
        })
        .await;
    }

    async fn publish_nonfatal_media_error(&self, code: &str, detail: Option<String>) {
        if !self.is_active() {
            return;
        }
        let code = code.to_owned();
        Self::on_main(move |cx| {
            if let Some(detail) = detail {
                Sai::log("RTC", move || detail, SaiLogKind::Network, cx);
            }
            RiseRTCPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish_nonfatal_error_code(code, cx);
            });
        })
        .await;
    }

    /// disconnect the user didnt choose, same teardown as a normal leave plus a one shot signal up
    /// the signal must outlive the room being cleared
    async fn evict(&self, room_id: &str, reason: RiseRTCEvictionReason) {
        let owns_room = self.state.with(|state| {
            state.active_credentials.as_ref().is_some_and(|credentials| {
                credentials.room_id == room_id && credentials.room_mode != RiseRTCRoomMode::DirectCall
            })
        });
        if !owns_room {
            return;
        }
        Self::log(format!("room eviction reason={}", reason.raw_value())).await;
        self.leave_room_expected_room_id(room_id).await;
        self.publish_eviction(RiseRTCRoomEviction {
            room_id: room_id.to_owned(),
            reason,
            occurred_at_ms: Date::now().ms(),
        })
        .await;
    }

    async fn publish_eviction(&self, eviction: RiseRTCRoomEviction) {
        if !self.is_active() {
            return;
        }
        Self::on_main(move |cx| {
            RiseRTCPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.publish_eviction(eviction, cx));
        })
        .await;
    }

    async fn clear_active_room_presentation(&self) {
        if !self.is_active() {
            return;
        }
        Self::on_main(|cx| {
            RiseRTCPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.select_room(None, cx);
                presentation.publish_connection_state(RiseRTCConnectionState::Idle, cx);
            });
        })
        .await;
    }

    fn error_code(error: &anyhow::Error) -> String {
        if let Some(value) = error.downcast_ref::<RiseRTCStateError>() {
            return match value {
                RiseRTCStateError::MissingActiveSession => "missing_active_session".to_owned(),
                RiseRTCStateError::StaleAccount => "stale_account".to_owned(),
                RiseRTCStateError::MissingActiveRoom => "missing_active_room".to_owned(),
                RiseRTCStateError::MissingJoinCredentials => "missing_join_credentials".to_owned(),
                RiseRTCStateError::MalformedResponse(method) => format!("malformed_{method}"),
                RiseRTCStateError::Validation(field) => format!("validation_{field}"),
                RiseRTCStateError::Remote(code) => code.clone(),
                RiseRTCStateError::AnsweredElsewhere => ANSWERED_ELSEWHERE.to_owned(),
            };
        }
        if error.is::<CancellationError>() {
            return "cancelled".to_owned();
        }
        "rtc_unavailable".to_owned()
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }

    /// name of the concrete error type this domain can give
    fn error_type_name(error: &anyhow::Error) -> &'static str {
        if error.is::<RiseRTCStateError>() {
            "RiseRTCStateError"
        } else if error.is::<CancellationError>() {
            "CancellationError"
        } else {
            "Error"
        }
    }

    fn error_summary(error: &anyhow::Error) -> String {
        format!(
            "error_type={} error_value={:?} error_chain={:#}",
            Self::error_type_name(error),
            error,
            error
        )
    }

    async fn log(message: String) {
        Self::on_main(move |cx| {
            Sai::log("RTC", move || message, SaiLogKind::Network, cx);
        })
        .await;
    }

    /// runs work on the main thread, none only when the
    /// app main thread isnt wired (tests without an app)
    async fn on_main<R, W>(work: W) -> Option<R>
    where
        R: Send + 'static,
        W: FnOnce(&mut App) -> R + Send + 'static,
    {
        let main = RiseEngineMainHop::installed()?;
        main.run(move |cx: AsyncApp| async move { Ok(cx.update(work)) }.boxed_local())
            .await
            .ok()
    }

    /// rebuilds a shared join outcome for each awaiting
    /// caller, keeping the error kinds callers match on
    fn shared_error(error: &Arc<anyhow::Error>) -> anyhow::Error {
        if error.is::<CancellationError>() {
            return CancellationError.into();
        }
        if let Some(value) = error.downcast_ref::<RiseRTCStateError>() {
            return value.clone().into();
        }
        anyhow::anyhow!("{error:#}")
    }
}

struct RiseRTCDomainCommittedMediaFinish<'a> {
    domain: &'a RiseRTCDomain,
    call_id: String,
    task_id: Uuid,
}

impl Drop for RiseRTCDomainCommittedMediaFinish<'_> {
    fn drop(&mut self) {
        self.domain.finish_committed_call_media_activation(&self.call_id, self.task_id);
    }
}

struct RiseRTCDomainReconnectWaiterRelease<'a> {
    domain: &'a RiseRTCDomain,
    waiter_id: Uuid,
    sleeper: JoinHandle<()>,
}

impl Drop for RiseRTCDomainReconnectWaiterRelease<'_> {
    fn drop(&mut self) {
        self.sleeper.abort();
        self.domain.resume_reconnect_waiter(self.waiter_id);
    }
}

struct RiseRTCDomainJoinRelease<'a> {
    domain: &'a RiseRTCDomain,
    token: Uuid,
}

impl Drop for RiseRTCDomainJoinRelease<'_> {
    fn drop(&mut self) {
        let token = self.token;
        self.domain.state.with(|state| {
            if state.in_flight_join.as_ref().map(|join| join.token) == Some(token) {
                state.in_flight_join = None;
            }
        });
    }
}

/// lowers the refreshing indicator when a refresh ends however it ends
struct RiseRTCDomainRefreshingRelease {
    domain: Weak<RiseRTCDomain>,
}

impl Drop for RiseRTCDomainRefreshingRelease {
    fn drop(&mut self) {
        let Some(domain) = self.domain.upgrade() else {
            return;
        };
        RiseAccountRuntime::current().spawn(async move {
            domain.publish_refreshing(false).await;
        });
    }
}

pub struct RiseRTCDomainRegistry {
    active: Option<Arc<RiseRTCDomain>>,
}

impl RiseRTCDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseRTCDomain>> {
        self.active.as_ref()
    }

    fn is_active(&self, domain: &Weak<RiseRTCDomain>) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| std::ptr::eq(Arc::as_ptr(active), domain.as_ptr()))
    }

    pub fn install(&mut self, domain: Arc<RiseRTCDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RiseRTCDomain>, cx: &mut Context<Self>) {
        if !self.is_active(domain) {
            return;
        }
        self.active = None;
        cx.notify();
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseRTCDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx)
            .update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseRTCStateError::MissingActiveSession.into())
        })
    }
}

pub enum RiseRTCActionExecutor {}

impl RiseRTCActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseRTCDomain>>> {
        RiseRTCDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }
}
