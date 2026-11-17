use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use futures::future::select;
use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity, WindowHandle};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseWireRemoteError, RiseWireSocketSession};
use rise_i18n::tr;
use rise_platform::call_window::{
    CallAttention, IncomingCallNotice, IncomingCallResponse, incoming_window_options, keep_call_window_above,
    present_incoming_call, retract_call_notification,
};
use rise_platform::notifications::GpuiNotificationPresenter;

use crate::app::shell::handover;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription};
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::sai::Sai;
use crate::core::push_notifications::desktop_notification_delivery::DesktopNotificationDelivery;
use crate::core::push_notifications::push_ownership_generation::{
    PushOwnershipGenerationPolicy, PushOwnershipGenerationStore, PushOwnershipRegistrationContext,
};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::{RISE_ENGINE_CONNECTION_READY, RiseEngineAppRuntime};
use crate::modules::auth::services::auth_local_storage::AuthLocalStorage;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::rtc::engine::rise_rtc_connection_wait_policy::{
    RiseRTCConnectionWaitPolicy, RiseRTCConnectionWaitPolicySignallingFailure,
};
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCStateError;
use crate::modules::rtc::media::rise_rtc_ringback_player::RiseRTCRingbackPlayer;
use crate::modules::rtc::system::rise_rtc_call_window_hooks::rise_rtc_call_window_hooks;
use crate::modules::rtc::system::rise_rtc_incoming_call_window::RiseRTCIncomingCallWindow;
use crate::modules::rtc::system::rise_rtc_incoming_ringtone_player::RiseRTCIncomingRingtonePlayer;
use crate::modules::rtc::system::rise_rtc_system_bridges::{
    RiseRTCMediaSystemBridge, RiseRTCSystemAnswerResult, RiseRTCSystemDomainBridge, RiseRTCSystemDomainBridgeError,
    RiseRTCSystemDomainClient,
    RiseRTCSystemOutgoingRequest, RiseRTCSystemStartedCall, RiseRTCSystemVoIPAPNSEnvironment,
    RiseRTCSystemVoIPTokenRegistration, RiseRTCSystemVoIPTokenRevocation,
};
use crate::modules::rtc::system::rise_rtc_system_call_presentation::{
    RiseRTCSystemCallPresentation, RiseRTCSystemCallPresentationStore,
};
use crate::modules::rtc::system::rise_rtc_system_models::{
    RiseRTCSystemAccountFence, RiseRTCSystemBackendEndAction, RiseRTCSystemCallDirection, RiseRTCSystemCallPhase,
    RiseRTCSystemCallRacePolicy, RiseRTCSystemCallTerminalOutcome, RiseRTCSystemIncomingEnvelope, RiseRTCSystemMediaMode,
    RiseRTCSystemVoIPEvent,
};
use crate::modules::user::shared::contact_device_book::RiseInstallationIdentity;

const LOG_NAME: &str = "RTC";
const TRANSPORT_HOLD_OWNER: &str = "rtc.system";
const DISPLAY_NAME_FALLBACK: &str = "Riseonly";
const HANDLE_FALLBACK: &str = "riseonly";
const CALL_START_FAILED: &str = "rtc_call_start_failed";
const CALL_KIT_UNAVAILABLE: &str = "rtc_callkit_unavailable";
const DEFERRED_SIGNALS_LIMIT: usize = 128;

const TERMINAL_RECONCILE_WINDOW: Duration = Duration::from_secs(60);
const MANAGED_CALL_REMOVAL_DELAY: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct ManagedCall {
    local_uuid: Uuid,
    server_call_id: Option<String>,
    direction: RiseRTCSystemCallDirection,
    phase: RiseRTCSystemCallPhase,
    account_fence: RiseRTCSystemAccountFence,
    remote_user_id: Option<String>,
    display_name: String,
    handle: String,
    avatar_url: Option<String>,
    has_video: bool,
    requested_at: Date,
    connected_at: Option<Date>,
    terminal_at: Option<Date>,
    terminal_outcome: Option<RiseRTCSystemCallTerminalOutcome>,
    operation_id: Option<Uuid>,
    end_requested: bool,
    end_idempotency_key: Option<String>,
    outgoing_request: Option<RiseRTCSystemOutgoingRequest>,
    system_refused: bool,
    requires_in_app_answer_ui: bool,
    awaiting_connection: bool,
}

#[derive(Clone)]
struct ConfirmedVoIPRegistration {
    account_id: String,
    registration: RiseRTCSystemVoIPTokenRegistration,
}

impl ConfirmedVoIPRegistration {
    fn key(&self) -> String {
        format!("{}|{}", self.account_id, self.registration.confirmation_key())
    }
}

/// what the ring surface currently shows so a later change of the caller updates it in place
struct IncomingCallSurface {
    local_uuid: Uuid,
    server_call_id: String,
    display_name: String,
    avatar_url: Option<String>,
    remote_user_id: Option<String>,
    has_video: bool,
    window: Option<WindowHandle<RiseRTCIncomingCallWindow>>,
}

pub struct RiseRTCCallSystemCoordinator {
    calls: HashMap<Uuid, ManagedCall>,
    server_call_index: HashMap<String, Uuid>,
    deferred_remote_events: HashMap<String, RiseRTCSystemVoIPEvent>,
    deferred_media_connected: HashSet<String>,
    operation_tasks: HashMap<Uuid, Task<()>>,
    cleanup_tasks: HashMap<Uuid, Task<()>>,
    engine_warmup_tasks: HashMap<Uuid, Task<()>>,
    #[allow(dead_code)]
    connection_observer: Option<RiseSubscription>,
    #[allow(dead_code)]
    connection_delivery: Option<Task<()>>,
    did_start: bool,

    account_generation: u64,
    last_activated_account_id: Option<String>,

    reconnect_waiters: HashMap<Uuid, oneshot::Sender<()>>,
    transport_hold_active: bool,
    vo_ip_token: Option<String>,
    ownership_context: Option<PushOwnershipRegistrationContext>,
    confirmed_vo_ip_registration: Option<ConfirmedVoIPRegistration>,
    registration_in_flight_key: Option<String>,
    #[allow(dead_code)]
    registration_task: Option<Task<()>>,
    registration_id: Option<Uuid>,
    last_registration_attempt: Option<(String, RiseWireSocketSession)>,

    incoming_call_surface: Option<IncomingCallSurface>,
    incoming_call_attention: CallAttention,
    incoming_surface_sync_scheduled: bool,
    start_refusals: HashMap<Uuid, String>,
}

/// a start the server refused with a reason the user can act on (the peer is already in a call)
/// callers show its text instead of the generic call failure
#[derive(Debug, Clone)]
pub struct RiseRTCCallStartRefusal {
    pub text: String,
}

impl std::fmt::Display for RiseRTCCallStartRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl std::error::Error for RiseRTCCallStartRefusal {}

impl RiseRTCCallStartRefusal {
    /// the refusal text when the server named a reason or else the generic call failure
    pub fn text_or_failed(error: &anyhow::Error) -> String {
        match error.downcast_ref::<Self>() {
            Some(refusal) => refusal.text.clone(),
            None => tr("rtc_call_failed"),
        }
    }
}

impl RiseRTCCallSystemCoordinator {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            calls: HashMap::new(),
            server_call_index: HashMap::new(),
            deferred_remote_events: HashMap::new(),
            deferred_media_connected: HashSet::new(),
            operation_tasks: HashMap::new(),
            cleanup_tasks: HashMap::new(),
            engine_warmup_tasks: HashMap::new(),
            connection_observer: None,
            connection_delivery: None,
            did_start: false,
            account_generation: 1,
            last_activated_account_id: None,
            reconnect_waiters: HashMap::new(),
            transport_hold_active: false,
            vo_ip_token: None,
            ownership_context: None,
            confirmed_vo_ip_registration: None,
            registration_in_flight_key: None,
            registration_task: None,
            registration_id: None,
            last_registration_attempt: None,
            incoming_call_surface: None,
            incoming_call_attention: CallAttention::new(),
            incoming_surface_sync_scheduled: false,
            start_refusals: HashMap::new(),
        }
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.did_start {
            return;
        }
        self.did_start = true;

        DesktopNotificationDelivery::set_incoming_call_response_handler(Self::receive_incoming_call_notification, cx);

        let (sender, mut receiver) = mpsc::unbounded::<()>();
        self.connection_observer = Some(RiseAppEvents::default_center().observe(RISE_ENGINE_CONNECTION_READY, move |_| {
            let _ = sender.unbounded_send(());
        }));
        self.connection_delivery = Some(cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                let alive = this
                    .update(cx, |this, cx| {
                        this.resume_reconnect_waiters();
                        this.retry_vo_ip_registration(cx);
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
        }));
        self.retry_vo_ip_registration(cx);
    }

    pub fn authenticated_account_did_activate(&mut self, raw_user_id: &str, cx: &mut Context<Self>) {
        let user_id = Self::normalized(Some(raw_user_id));
        if let (Some(previous), Some(user_id)) = (&self.last_activated_account_id, &user_id)
            && previous != user_id
        {
            self.fence_account_transition("account_changed", cx);
        }
        self.last_activated_account_id = user_id;
        self.retry_vo_ip_registration(cx);
    }

    /// called synchronously before the account engine and socket shut down,
    /// invalidates every captured callback before a new identity can become active
    pub fn fence_account_transition(&mut self, reason: &str, cx: &mut Context<Self>) {
        self.advance_account_generation();
        self.last_activated_account_id = None;
        self.registration_task = None;
        self.registration_in_flight_key = None;
        self.registration_id = None;
        self.last_registration_attempt = None;

        let old_registration = self.confirmed_vo_ip_registration.take();
        let old_client = old_registration.as_ref().and_then(|registration| {
            RiseRTCSystemDomainBridge::shared(cx).read(cx).current_client(Some(&registration.account_id))
        });

        self.operation_tasks.clear();
        self.cleanup_tasks.clear();
        self.engine_warmup_tasks.clear();
        self.calls.clear();
        self.calls_did_set(cx);
        self.server_call_index.clear();
        self.deferred_remote_events.clear();
        self.deferred_media_connected.clear();

        cx.spawn(async move |_, cx| {
            if let (Some(old_registration), Some(old_client)) = (old_registration, old_client) {
                let revocation = RiseRTCSystemVoIPTokenRevocation {
                    token: old_registration.registration.token,
                    environment: old_registration.registration.environment,
                };
                let _ = Self::on_account_runtime(async move { old_client.revoke_system_vo_ip_token(revocation).await }).await;
            }
            cx.update(RiseRTCMediaSystemBridge::reset);
        })
        .detach();
        Self::log(format!("system calls fenced reason={reason}"), cx);
    }

    pub fn retry_vo_ip_registration(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.vo_ip_token.clone() else {
            return;
        };
        let Some(account_id) = Self::active_account_id(cx) else {
            return;
        };
        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            return;
        }
        let Some(registration) = self.make_registration(token, &account_id) else {
            return;
        };

        let candidate = ConfirmedVoIPRegistration { account_id: account_id.clone(), registration };
        let candidate_key = candidate.key();
        if self.confirmed_vo_ip_registration.as_ref().map(ConfirmedVoIPRegistration::key).as_ref() == Some(&candidate_key)
            || self.registration_in_flight_key.as_ref() == Some(&candidate_key)
        {
            return;
        }

        self.registration_in_flight_key = Some(candidate_key);
        let identifier = Uuid::new_v4();
        self.registration_id = Some(identifier);
        let expected_account_generation = self.account_generation;
        self.registration_task = Some(cx.spawn(async move |this, cx| {
            Self::run_registration(&this, candidate, identifier, expected_account_generation, cx).await;
            let _ = this.update(cx, |this, cx| {
                if this.registration_id != Some(identifier) {
                    return;
                }
                this.registration_in_flight_key = None;
                this.registration_id = None;
                let previous = this.last_registration_attempt.as_ref().map(|(_, connection)| connection.clone());
                if let Some(previous) = previous {
                    cx.spawn(async move |this, cx| {
                        let ready = cx.update(|cx| {
                            RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.ready_connection(&account_id, cx))
                        });
                        if let Some(current) = ready.await
                            && current != previous
                        {
                            let _ = this.update(cx, |this, cx| this.retry_vo_ip_registration(cx));
                        }
                    })
                    .detach();
                }
                this.registration_task = None;
            });
        }));
    }

    async fn run_registration(
        this: &WeakEntity<Self>,
        candidate: ConfirmedVoIPRegistration,
        identifier: Uuid,
        expected_account_generation: u64,
        cx: &mut AsyncApp,
    ) {
        let account_id = candidate.account_id.clone();
        let is_current = |this: &mut Self, cx: &mut Context<Self>| {
            this.registration_id == Some(identifier)
                && expected_account_generation == this.account_generation
                && Self::active_account_id(cx).as_deref() == Some(account_id.as_str())
        };
        let ready = cx.update(|cx| {
            RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.ready_connection(&candidate.account_id, cx))
        });
        let Some(connection) = ready.await else {
            return;
        };
        if !this.update(cx, |this, cx| is_current(this, cx)).unwrap_or(false) {
            return;
        }
        let key = candidate.key();
        let repeats_attempt = this
            .update(cx, |this, _| {
                this.last_registration_attempt
                    .as_ref()
                    .is_some_and(|(attempt_key, attempt_connection)| *attempt_key == key && *attempt_connection == connection)
            })
            .unwrap_or(true);
        if repeats_attempt {
            return;
        }
        let client =
            match Self::require_client(candidate.account_id.clone(), RiseRTCSystemDomainBridge::RESTORATION_TIMEOUT, cx).await {
                Ok(client) => client,
                Err(_) => return,
            };
        if !this.update(cx, |this, cx| is_current(this, cx)).unwrap_or(false) {
            return;
        }
        let _ = this.update(cx, |this, _| this.last_registration_attempt = Some((key, connection)));
        let registration = candidate.registration.clone();
        let outcome = Self::on_account_runtime(async move { client.register_system_vo_ip_token(registration).await })
            .await
            .and_then(|outcome| outcome);
        match outcome {
            Ok(()) => {
                if !this.update(cx, |this, cx| is_current(this, cx)).unwrap_or(false) {
                    return;
                }
                let _ = this.update(cx, |this, _| this.confirmed_vo_ip_registration = Some(candidate));
                Self::log_async("VoIP token registered".to_owned(), cx);
            }
            Err(error) if error.downcast_ref::<CancellationError>().is_some() => {}
            Err(error) => {
                Self::log_async(format!("VoIP token registration failed: {error}"), cx);
            }
        }
    }

    pub fn request_outgoing_call(
        &mut self,
        callee_id: String,
        chat_id: String,
        display_name: String,
        avatar_url: Option<String>,
        has_video: bool,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Uuid>> {
        let Some(account_id) = Self::active_account_id(cx) else {
            return Task::ready(Err(
                RiseRTCSystemDomainBridgeError::DomainUnavailable.into(),
            ));
        };
        if let Some(live_uuid) = self.live_user_call_uuid() {
            Self::log("outgoing call refused: a call is already live, raising its window".to_owned(), cx);
            if let Some(hooks) = rise_rtc_call_window_hooks() {
                cx.defer(hooks.activate_call_window);
            }
            return Task::ready(Ok(live_uuid));
        }
        let call_uuid = Uuid::new_v4();
        let fence = RiseRTCSystemAccountFence {
            account_id,
            account_epoch: Self::active_account_epoch(),
            generation: self.account_generation,
        };
        let callee = Self::normalized(Some(&callee_id));
        let request = RiseRTCSystemOutgoingRequest {
            local_call_uuid: call_uuid,
            callee_id,
            chat_id,
            media_mode: if has_video { RiseRTCSystemMediaMode::Video } else { RiseRTCSystemMediaMode::Audio },
            idempotency_key: Self::lowercased(call_uuid),
        };
        self.calls.insert(
            call_uuid,
            ManagedCall {
                local_uuid: call_uuid,
                server_call_id: None,
                direction: RiseRTCSystemCallDirection::Outgoing,
                phase: RiseRTCSystemCallPhase::OutgoingRequested,
                account_fence: fence,
                remote_user_id: callee.clone(),
                display_name: Self::normalized(Some(&display_name)).unwrap_or_else(|| DISPLAY_NAME_FALLBACK.to_owned()),
                handle: callee.unwrap_or_else(|| HANDLE_FALLBACK.to_owned()),
                avatar_url: Self::normalized(avatar_url.as_deref()),
                has_video,
                requested_at: RiseClock::now(),
                connected_at: None,
                terminal_at: None,
                terminal_outcome: None,
                operation_id: None,
                end_requested: false,
                end_idempotency_key: None,
                outgoing_request: Some(request),
                system_refused: true,
                requires_in_app_answer_ui: false,
                awaiting_connection: false,
            },
        );
        self.calls_did_set(cx);

        let (resolve, resolved) = oneshot::channel();
        self.perform_start(call_uuid, resolve, cx);
        cx.spawn(async move |this, cx| {
            if resolved.await.unwrap_or(false) {
                return Ok(call_uuid);
            }
            let refusal = this.update(cx, |this, cx| {
                this.calls.remove(&call_uuid);
                this.calls_did_set(cx);
                this.start_refusals.remove(&call_uuid)
            })?;
            if let Some(text) = refusal {
                return Err(RiseRTCCallStartRefusal { text }.into());
            }
            Err(RiseRTCStateError::Remote(CALL_START_FAILED.to_owned()).into())
        })
    }

    fn live_user_call_uuid(&self) -> Option<Uuid> {
        self.calls.values().find(|call| {
            !call.end_requested
                && matches!(
                    call.phase,
                    RiseRTCSystemCallPhase::OutgoingRequested
                        | RiseRTCSystemCallPhase::OutgoingStarting
                        | RiseRTCSystemCallPhase::OutgoingRinging
                        | RiseRTCSystemCallPhase::Answering
                        | RiseRTCSystemCallPhase::Active
                )
        }).map(|call| call.local_uuid)
    }

    /// every caller treats an error as no system framework owns this
    /// call and uses the domain, which is what the desktop always needs
    pub fn request_end_call_uuid(&mut self, call_uuid: Uuid, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        let phase = self.calls.get(&call_uuid).map(|call| call.phase);
        let short_uuid: String = call_uuid.uuid_string().chars().take(8).collect();
        Self::log(format!("end requested by app callUUID={short_uuid} phase={phase:?}"), cx);
        Task::ready(Err(RiseRTCStateError::Remote(CALL_KIT_UNAVAILABLE.to_owned()).into()))
    }

    /// false for a restored or non managed call so the caller uses the idempotent domain path
    pub fn request_end_server_call_id(&mut self, server_call_id: String, cx: &mut Context<Self>) -> Task<anyhow::Result<bool>> {
        let Some(call_uuid) = self.local_call_uuid(&server_call_id) else {
            return Task::ready(Ok(false));
        };
        let request = self.request_end_call_uuid(call_uuid, cx);
        cx.spawn(async move |_, _| {
            request.await?;
            Ok(true)
        })
    }

    pub fn request_muted_call_uuid(&mut self, _is_muted: bool, _call_uuid: Uuid) -> Task<anyhow::Result<()>> {
        Task::ready(Err(RiseRTCStateError::Remote(CALL_KIT_UNAVAILABLE.to_owned()).into()))
    }

    /// false when no system framework owns this restored call
    pub fn request_muted_server_call_id(&mut self, is_muted: bool, server_call_id: String, cx: &mut Context<Self>) -> Task<anyhow::Result<bool>> {
        let Some(call_uuid) = self.local_call_uuid(&server_call_id) else {
            return Task::ready(Ok(false));
        };
        let request = self.request_muted_call_uuid(is_muted, call_uuid);
        cx.spawn(async move |_, _| {
            request.await?;
            Ok(true)
        })
    }

    pub fn report_media_connected(&mut self, server_call_id: &str, cx: &mut Context<Self>) {
        let Some(call_uuid) = self.local_call_uuid(server_call_id) else {
            self.defer_media_connected(server_call_id);
            return;
        };
        self.deferred_media_connected.remove(server_call_id);
        let Some(call) = self.calls.get_mut(&call_uuid) else {
            return;
        };
        if matches!(call.phase, RiseRTCSystemCallPhase::Active | RiseRTCSystemCallPhase::Ending | RiseRTCSystemCallPhase::Ended) {
            return;
        }
        call.phase = RiseRTCSystemCallPhase::Active;
        call.connected_at.get_or_insert_with(RiseClock::now);
        self.calls_did_set(cx);
    }

    pub fn report_remote_call_event(&mut self, server_call_id: &str, event: RiseRTCSystemVoIPEvent, cx: &mut Context<Self>) {
        let Some(call_uuid) = self.local_call_uuid(server_call_id) else {
            self.defer_remote_event(event, server_call_id);
            return;
        };
        self.deferred_remote_events.remove(server_call_id);
        if event.is_terminal() {
            self.deferred_media_connected.remove(server_call_id);
        }
        match event {
            RiseRTCSystemVoIPEvent::Answered => {
                let Some(call) = self.calls.get(&call_uuid) else {
                    return;
                };
                if call.direction == RiseRTCSystemCallDirection::Incoming && call.phase == RiseRTCSystemCallPhase::IncomingRinging {
                    self.finish_managed_call(call_uuid, RiseRTCSystemCallTerminalOutcome::AnsweredElsewhere, cx);
                }
            }
            RiseRTCSystemVoIPEvent::Cancelled
            | RiseRTCSystemVoIPEvent::Ended
            | RiseRTCSystemVoIPEvent::AnsweredElsewhere
            | RiseRTCSystemVoIPEvent::Declined
            | RiseRTCSystemVoIPEvent::TimedOut => {
                self.finish_managed_call(call_uuid, Self::terminal_outcome(&event), cx);
            }
            RiseRTCSystemVoIPEvent::Incoming | RiseRTCSystemVoIPEvent::Unknown(_) => {}
        }
    }

    /// the open socket is the only ring transport on the desktop
    pub fn report_realtime_incoming_call(&mut self, envelope: RiseRTCSystemIncomingEnvelope, cx: &mut Context<Self>) {
        let Some(server_call_id) = envelope.server_call_id.clone() else {
            return;
        };
        let Some(account_id) = Self::active_account_id(cx) else {
            return;
        };
        if envelope.recipient_user_id.as_deref() != Some(account_id.as_str()) {
            return;
        }
        if let Some(call_uuid) = self.local_call_uuid(&server_call_id) {
            self.update_incoming_caller(&envelope, call_uuid, cx);
            return;
        }
        let unexpired = envelope.expires_at_milliseconds.is_some_and(|expires_at| expires_at > Self::now_milliseconds());
        if self.calls.contains_key(&envelope.call_uuid) || envelope.caller.is_none() || !unexpired {
            return;
        }

        let fence = RiseRTCSystemAccountFence {
            account_id,
            account_epoch: Self::active_account_epoch(),
            generation: self.account_generation,
        };
        self.present_incoming_invitation(&envelope, server_call_id, fence, cx);
    }

    fn present_incoming_invitation(
        &mut self,
        envelope: &RiseRTCSystemIncomingEnvelope,
        server_call_id: String,
        fence: RiseRTCSystemAccountFence,
        cx: &mut Context<Self>,
    ) {
        let call_uuid = envelope.call_uuid;
        self.warm_engine_for_incoming_call(call_uuid, &fence, cx);
        if self.calls.contains_key(&call_uuid) {
            self.update_incoming_caller(envelope, call_uuid, cx);
            return;
        }
        let caller = envelope.caller.as_ref();
        self.calls.insert(
            call_uuid,
            ManagedCall {
                local_uuid: call_uuid,
                server_call_id: Some(server_call_id.clone()),
                direction: RiseRTCSystemCallDirection::Incoming,
                phase: RiseRTCSystemCallPhase::IncomingRinging,
                account_fence: fence,
                remote_user_id: caller.and_then(|caller| caller.id.clone()),
                display_name: caller.map_or_else(|| DISPLAY_NAME_FALLBACK.to_owned(), |caller| caller.display_name()),
                handle: caller.map_or_else(|| HANDLE_FALLBACK.to_owned(), |caller| caller.handle()),
                avatar_url: caller.and_then(|caller| caller.avatar_url.clone()),
                has_video: envelope.media_mode == RiseRTCSystemMediaMode::Video,
                requested_at: envelope
                    .created_at_milliseconds
                    .map_or_else(RiseClock::now, |milliseconds| Date::from_time_interval_since_1970(milliseconds as f64 / 1_000.0)),
                connected_at: None,
                terminal_at: None,
                terminal_outcome: None,
                operation_id: None,
                end_requested: false,
                end_idempotency_key: None,
                outgoing_request: None,
                system_refused: true,
                requires_in_app_answer_ui: false,
                awaiting_connection: false,
            },
        );
        self.server_call_index.insert(server_call_id, call_uuid);
        self.calls_did_set(cx);
    }

    fn perform_start(&mut self, call_uuid: Uuid, resolve: oneshot::Sender<bool>, cx: &mut Context<Self>) {
        let mut resolution = RiseRTCSystemResolution::new(resolve);
        let Some(call) = self.calls.get_mut(&call_uuid).filter(|call| call.direction == RiseRTCSystemCallDirection::Outgoing) else {
            resolution.finish(false);
            return;
        };
        let Some(request) = call.outgoing_request.clone() else {
            resolution.finish(false);
            return;
        };
        let operation_id = Uuid::new_v4();
        call.phase = RiseRTCSystemCallPhase::OutgoingStarting;
        call.operation_id = Some(operation_id);
        let fence = call.account_fence.clone();
        self.calls_did_set(cx);

        let task = cx.spawn(async move |this, cx| {
            Self::run_start(this, call_uuid, operation_id, request, fence, resolution, cx).await;
        });
        self.operation_tasks.insert(call_uuid, task);
    }

    async fn run_start(
        this: WeakEntity<Self>,
        call_uuid: Uuid,
        operation_id: Uuid,
        request: RiseRTCSystemOutgoingRequest,
        fence: RiseRTCSystemAccountFence,
        mut resolution: RiseRTCSystemResolution,
        cx: &mut AsyncApp,
    ) {
        let deadline = RiseClock::monotonic() + RiseRTCConnectionWaitPolicy::window();
        if this.update(cx, |this, cx| this.confirm_if_no_connection(&mut resolution, call_uuid, cx)).is_err() {
            return;
        }
        let mut attempt: i64 = 0;
        let mut may_exist_on_server = false;
        loop {
            attempt += 1;
            match Self::attempt_start(&this, &fence, request.clone(), cx).await {
                Ok((client, started)) => {
                    let _ = this.update(cx, |this, cx| this.set_awaiting_connection(false, call_uuid, cx));
                    Self::complete_start(&this, call_uuid, operation_id, started, client, &mut resolution, cx).await;
                }
                Err(error) if error.downcast_ref::<CancellationError>().is_some() => resolution.finish(false),
                Err(error) => {
                    let failure = RiseRTCConnectionWaitPolicy::signalling_failure(&error);
                    if failure == Some(RiseRTCConnectionWaitPolicySignallingFailure::Unconfirmed) {
                        may_exist_on_server = true;
                    }
                    let Ok((still_wanted, call_exists)) = this
                        .update(cx, |this, _| (this.owns_pending_operation(call_uuid, operation_id), this.calls.contains_key(&call_uuid)))
                    else {
                        return;
                    };
                    if failure.is_some()
                        && RiseClock::monotonic() < deadline
                        && (still_wanted || (may_exist_on_server && call_exists))
                    {
                        if still_wanted {
                            resolution.finish(true);
                            let _ = this.update(cx, |this, cx| this.set_awaiting_connection(true, call_uuid, cx));
                        }
                        let summary = Self::safe_call_error_summary(&error);
                        Self::log_async(format!("outgoing call start waits for a connection attempt={attempt} {summary}"), cx);
                        Self::await_reconnect_or_backoff(&this, RiseRTCConnectionWaitPolicy::backoff_milliseconds(attempt), cx).await;
                        let Ok((owned, exists, end_requested)) = this.update(cx, |this, _| {
                            (
                                this.owns_pending_operation(call_uuid, operation_id),
                                this.calls.contains_key(&call_uuid),
                                this.calls.get(&call_uuid).is_some_and(|call| call.end_requested),
                            )
                        }) else {
                            return;
                        };
                        if owned || (may_exist_on_server && exists) {
                            continue;
                        }
                        if end_requested {
                            let _ = this.update(cx, |this, cx| {
                                this.mark_managed_call_ended(call_uuid, Some(RiseRTCSystemCallTerminalOutcome::Cancelled), cx)
                            });
                        }
                        break;
                    }
                    let summary = Self::safe_call_error_summary(&error);
                    tracing::warn!(target: "riseonly::X1842", call = %Self::lowercased(call_uuid), %summary, "outgoing call start refused");
                    let refusal = error
                        .downcast_ref::<RiseWireRemoteError>()
                        .and_then(|remote| CanonErrors::text(Some(remote.code.as_str()), &HashMap::new()));
                    if let Some(text) = refusal {
                        let _ = this.update(cx, |this, _| {
                            this.start_refusals.insert(call_uuid, text);
                        });
                    }
                    resolution.finish(false);
                    let _ = this.update(cx, |this, cx| {
                        this.finish_managed_call(call_uuid, RiseRTCSystemCallTerminalOutcome::Failed, cx)
                    });
                    Self::log_async(format!("outgoing call start failed {summary}"), cx);
                }
            }
            break;
        }
        let _ = this.update(cx, |this, _| {
            this.operation_tasks.remove(&call_uuid);
        });
    }

    async fn attempt_start(
        this: &WeakEntity<Self>,
        fence: &RiseRTCSystemAccountFence,
        request: RiseRTCSystemOutgoingRequest,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<(Arc<dyn RiseRTCSystemDomainClient>, RiseRTCSystemStartedCall)> {
        let client = Self::require_client(fence.account_id.clone(), RiseRTCSystemDomainBridge::RESTORATION_TIMEOUT, cx).await?;
        Self::ensure_fence(this, fence, cx)?;
        let domain_client = client.clone();
        let started = Self::on_account_runtime(async move { domain_client.start_system_outgoing_call(request).await }).await??;
        Ok((client, started))
    }

    async fn complete_start(
        this: &WeakEntity<Self>,
        call_uuid: Uuid,
        operation_id: Uuid,
        started: RiseRTCSystemStartedCall,
        client: Arc<dyn RiseRTCSystemDomainClient>,
        resolution: &mut RiseRTCSystemResolution,
        cx: &mut AsyncApp,
    ) {
        let late_key = format!("{}:late-cancel", Self::lowercased(call_uuid));
        let server_call_id = started.server_call_id;
        let Ok(cancel_key) = this.update(cx, |this, cx| {
            let call = this.calls.get_mut(&call_uuid)?;
            call.server_call_id = Some(server_call_id.clone());
            let accepted = !call.end_requested
                && RiseRTCSystemCallRacePolicy::accepts_async_completion(operation_id, call.operation_id, call.phase);
            let cancel_key = if accepted {
                call.phase = RiseRTCSystemCallPhase::OutgoingRinging;
                call.operation_id = None;
                Err(())
            } else {
                Ok(call.end_idempotency_key.clone().unwrap_or_else(|| late_key.clone()))
            };
            this.server_call_index.insert(server_call_id.clone(), call_uuid);
            this.calls_did_set(cx);
            Some(cancel_key)
        }) else {
            return;
        };
        match cancel_key {
            None => {
                resolution.finish(false);
                let call_id = server_call_id;
                let _ = Self::on_account_runtime(async move { client.cancel_system_call(call_id, late_key).await }).await;
            }
            Some(Ok(key)) => {
                let _ = this.update(cx, |this, _| this.clear_deferred_signals(&server_call_id));
                resolution.finish(true);
                let call_id = server_call_id;
                let _ = Self::on_account_runtime(async move { client.cancel_system_call(call_id, key).await }).await;
                let _ = this.update(cx, |this, cx| this.mark_managed_call_ended(call_uuid, None, cx));
            }
            Some(Err(())) => {
                resolution.finish(true);
                let _ = this.update(cx, |this, cx| this.replay_deferred_signals(&server_call_id, cx));
            }
        }
    }

    /// the window, a key and a button of the notification all answer here
    pub fn respond_to_incoming_call(&mut self, server_call_id: String, response: IncomingCallResponse, cx: &mut Context<Self>) {
        match response {
            IncomingCallResponse::Answer => {
                cx.defer(handover::raise_app);
                self.request_answer(server_call_id, cx).detach();
            }
            IncomingCallResponse::Decline => {
                if let Some(call_uuid) = self.local_call_uuid(&server_call_id) {
                    self.perform_end_action(call_uuid, cx);
                }
            }
        }
    }

    fn receive_incoming_call_notification(server_call_id: &str, response: Option<IncomingCallResponse>, cx: &mut App) {
        let Some(response) = response else {
            handover::raise_app(cx);
            return;
        };
        let server_call_id = server_call_id.to_owned();
        Self::shared(cx).update(cx, |this, cx| this.respond_to_incoming_call(server_call_id, response, cx));
    }

    pub fn request_answer(&mut self, server_call_id: String, cx: &mut Context<Self>) -> Task<bool> {
        self.answer_in_app_call(server_call_id, cx)
    }

    pub fn answer_in_app_call(&mut self, server_call_id: String, cx: &mut Context<Self>) -> Task<bool> {
        let Some(call_uuid) = self.server_call_index.get(&server_call_id).copied().filter(|uuid| self.calls.contains_key(uuid)) else {
            return Task::ready(false);
        };
        let (resolve, resolved) = oneshot::channel();
        self.perform_answer(call_uuid, resolve, cx);
        cx.spawn(async move |_, _| resolved.await.unwrap_or(false))
    }

    fn perform_answer(&mut self, call_uuid: Uuid, resolve: oneshot::Sender<bool>, cx: &mut Context<Self>) {
        let mut resolution = RiseRTCSystemResolution::new(resolve);
        let Some(call) = self.calls.get_mut(&call_uuid).filter(|call| call.direction == RiseRTCSystemCallDirection::Incoming) else {
            resolution.finish(false);
            return;
        };
        let Some(server_call_id) = call.server_call_id.clone() else {
            resolution.finish(false);
            return;
        };
        if call.phase == RiseRTCSystemCallPhase::Active {
            resolution.finish(true);
            return;
        }

        let operation_id = Uuid::new_v4();
        call.phase = RiseRTCSystemCallPhase::Answering;
        call.operation_id = Some(operation_id);
        let fence = call.account_fence.clone();
        self.calls_did_set(cx);

        let task = cx.spawn(async move |this, cx| {
            Self::run_answer(this, call_uuid, operation_id, server_call_id, fence, resolution, cx).await;
        });
        self.operation_tasks.insert(call_uuid, task);
    }

    async fn run_answer(
        this: WeakEntity<Self>,
        call_uuid: Uuid,
        operation_id: Uuid,
        server_call_id: String,
        fence: RiseRTCSystemAccountFence,
        mut resolution: RiseRTCSystemResolution,
        cx: &mut AsyncApp,
    ) {
        let deadline = RiseClock::monotonic() + RiseRTCConnectionWaitPolicy::window();
        if this.update(cx, |this, cx| this.confirm_if_no_connection(&mut resolution, call_uuid, cx)).is_err() {
            return;
        }
        let mut attempt: i64 = 0;
        loop {
            attempt += 1;
            match Self::attempt_answer(&this, &fence, server_call_id.clone(), cx).await {
                Ok((client, result)) => {
                    let _ = this.update(cx, |this, cx| this.set_awaiting_connection(false, call_uuid, cx));
                    Self::complete_answer(&this, call_uuid, operation_id, server_call_id.clone(), result, client, &mut resolution, cx).await;
                    break;
                }
                Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                    resolution.finish(false);
                    break;
                }
                Err(error) => {
                    let owned = this.update(cx, |this, _| this.owns_pending_operation(call_uuid, operation_id)).unwrap_or(false);
                    if RiseRTCConnectionWaitPolicy::signalling_failure(&error).is_some() && RiseClock::monotonic() < deadline && owned {
                        resolution.finish(true);
                        let _ = this.update(cx, |this, cx| this.set_awaiting_connection(true, call_uuid, cx));
                        let summary = Self::safe_call_error_summary(&error);
                        Self::log_async(format!("answer waits for a connection attempt={attempt} {summary}"), cx);
                        Self::await_reconnect_or_backoff(&this, RiseRTCConnectionWaitPolicy::backoff_milliseconds(attempt), cx).await;
                        if this.update(cx, |this, _| this.owns_pending_operation(call_uuid, operation_id)).unwrap_or(false) {
                            continue;
                        }
                        break;
                    }
                    resolution.finish(false);
                    let _ = this.update(cx, |this, cx| {
                        let failed_call = this.calls.get(&call_uuid).cloned();
                        let previous_phase = failed_call.as_ref().map_or(RiseRTCSystemCallPhase::Answering, |call| call.phase);
                        this.finish_managed_call(call_uuid, RiseRTCSystemCallTerminalOutcome::Failed, cx);
                        if let Some(failed_call) = failed_call {
                            let key = failed_call
                                .end_idempotency_key
                                .clone()
                                .unwrap_or_else(|| format!("{}:answer-failed", Self::lowercased(call_uuid)));
                            this.reconcile_backend_end(&failed_call, previous_phase, key, cx);
                        }
                    });
                    Self::log_async(format!("answer call failed: {error}"), cx);
                    break;
                }
            }
        }
        let _ = this.update(cx, |this, _| {
            this.operation_tasks.remove(&call_uuid);
        });
    }

    async fn attempt_answer(
        this: &WeakEntity<Self>,
        fence: &RiseRTCSystemAccountFence,
        server_call_id: String,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<(Arc<dyn RiseRTCSystemDomainClient>, RiseRTCSystemAnswerResult)> {
        let installation_id = RiseInstallationIdentity::current()?;
        let client = Self::require_client(fence.account_id.clone(), RiseRTCSystemDomainBridge::RESTORATION_TIMEOUT, cx).await?;
        Self::ensure_fence(this, fence, cx)?;
        let domain_client = client.clone();
        let result =
            Self::on_account_runtime(async move { domain_client.answer_system_call(server_call_id, installation_id).await }).await??;
        Ok((client, result))
    }

    #[allow(clippy::too_many_arguments)]
    async fn complete_answer(
        this: &WeakEntity<Self>,
        call_uuid: Uuid,
        operation_id: Uuid,
        server_call_id: String,
        result: RiseRTCSystemAnswerResult,
        client: Arc<dyn RiseRTCSystemDomainClient>,
        resolution: &mut RiseRTCSystemResolution,
        cx: &mut AsyncApp,
    ) {
        let late_key = format!("{}:late-end", Self::lowercased(call_uuid));
        let call = this.update(cx, |this, _| this.calls.get(&call_uuid).cloned()).ok().flatten();
        let Some(call) = call else {
            resolution.finish(false);
            let reason = Some("system_call_disappeared".to_owned());
            let _ = Self::on_account_runtime(async move { client.end_system_call(server_call_id, late_key, reason).await }).await;
            return;
        };
        if result != RiseRTCSystemAnswerResult::Answered {
            resolution.finish(false);
            let _ = this.update(cx, |this, cx| {
                this.finish_managed_call(call_uuid, RiseRTCSystemCallTerminalOutcome::AnsweredElsewhere, cx)
            });
            return;
        }
        if call.end_requested
            || !RiseRTCSystemCallRacePolicy::accepts_async_completion(operation_id, call.operation_id, call.phase)
        {
            resolution.finish(false);
            let key = call.end_idempotency_key.unwrap_or(late_key);
            let reason = Some("ended_during_answer".to_owned());
            let _ = Self::on_account_runtime(async move { client.end_system_call(server_call_id, key, reason).await }).await;
            let _ = this.update(cx, |this, cx| this.mark_managed_call_ended(call_uuid, None, cx));
            return;
        }

        let _ = this.update(cx, |this, cx| {
            if let Some(call) = this.calls.get_mut(&call_uuid) {
                call.phase = RiseRTCSystemCallPhase::Active;
                call.connected_at.get_or_insert_with(RiseClock::now);
                call.operation_id = None;
                this.calls_did_set(cx);
            }
        });
        resolution.finish(true);
    }

    fn perform_end_action(&mut self, call_uuid: Uuid, cx: &mut Context<Self>) {
        let Some(call) = self.calls.get_mut(&call_uuid) else {
            return;
        };
        if matches!(call.phase, RiseRTCSystemCallPhase::Ending | RiseRTCSystemCallPhase::Ended) {
            return;
        }

        let previous_phase = call.phase;
        let idempotency_key =
            call.end_idempotency_key.clone().unwrap_or_else(|| format!("{}:system-end", Self::lowercased(call_uuid)));
        call.end_requested = true;
        call.end_idempotency_key = Some(idempotency_key.clone());
        call.phase = RiseRTCSystemCallPhase::Ending;
        call.operation_id = Some(Uuid::new_v4());
        let call = call.clone();
        self.calls_did_set(cx);

        self.reconcile_backend_end(&call, previous_phase, idempotency_key, cx);
        self.schedule_managed_call_removal(call_uuid, cx);
    }

    fn reconcile_backend_end(
        &mut self,
        call: &ManagedCall,
        previous_phase: RiseRTCSystemCallPhase,
        idempotency_key: String,
        cx: &mut Context<Self>,
    ) {
        let backend_action =
            RiseRTCSystemCallRacePolicy::backend_end_action(call.direction, previous_phase, call.server_call_id.is_some());
        Self::log(
            format!("backend end reconcile action={backend_action:?} previousPhase={previous_phase:?} direction={:?}", call.direction),
            cx,
        );
        let Some(server_call_id) = call.server_call_id.clone() else {
            return;
        };
        if backend_action == RiseRTCSystemBackendEndAction::WaitForStartThenCancel {
            return;
        }
        let fence = call.account_fence.clone();
        let deadline = RiseClock::monotonic() + TERMINAL_RECONCILE_WINDOW;
        cx.spawn(async move |this, cx| {
            let mut attempt: i64 = 0;
            loop {
                attempt += 1;
                let client = Self::require_client(fence.account_id.clone(), RiseRTCSystemDomainBridge::RESTORATION_TIMEOUT, cx).await;
                let outcome = match client {
                    Ok(client) => {
                        let call_id = server_call_id.clone();
                        let key = idempotency_key.clone();
                        Self::on_account_runtime(async move { Self::send_backend_end(client, backend_action, call_id, key).await })
                            .await
                            .and_then(|outcome| outcome)
                    }
                    Err(error) => Err(error),
                };
                let error = match outcome {
                    Ok(()) => return,
                    Err(error) if error.downcast_ref::<CancellationError>().is_some() => return,
                    Err(error) => error,
                };
                let Ok(matches) = this.update(cx, |this, cx| this.fence_matches(&fence, cx)) else {
                    return;
                };
                if !matches {
                    return;
                }
                Self::log_async(format!("system end reconciliation attempt {attempt} failed: {error}"), cx);
                if RiseClock::monotonic() >= deadline {
                    Self::log_async(
                        format!(
                            "system end reconciliation gave up after {attempt} attempt(s); the server will supersede this call on the next start"
                        ),
                        cx,
                    );
                    return;
                }
                Self::await_reconnect_or_backoff(&this, RiseRTCConnectionWaitPolicy::backoff_milliseconds(attempt), cx).await;
            }
        })
        .detach();
    }

    async fn send_backend_end(
        client: Arc<dyn RiseRTCSystemDomainClient>,
        action: RiseRTCSystemBackendEndAction,
        call_id: String,
        key: String,
    ) -> anyhow::Result<()> {
        match action {
            RiseRTCSystemBackendEndAction::Decline => {
                client.decline_system_call(call_id, key, Some("system_decline".to_owned())).await
            }
            RiseRTCSystemBackendEndAction::Cancel => client.cancel_system_call(call_id, key).await,
            RiseRTCSystemBackendEndAction::End => client.end_system_call(call_id, key, Some("system_hangup".to_owned())).await,
            RiseRTCSystemBackendEndAction::EndThenDecline => {
                match client.end_system_call(call_id.clone(), key.clone(), Some("system_hangup".to_owned())).await {
                    Ok(()) => Ok(()),
                    Err(error) if error.downcast_ref::<CancellationError>().is_some() => Err(error),
                    Err(_) => client.decline_system_call(call_id, format!("{key}:decline"), Some("system_decline".to_owned())).await,
                }
            }
            RiseRTCSystemBackendEndAction::WaitForStartThenCancel => Ok(()),
        }
    }

    /// sleeps the retry backoff, returns early when the socket is back or the waiting task is dropped
    async fn await_reconnect_or_backoff(this: &WeakEntity<Self>, backoff_milliseconds: i64, cx: &mut AsyncApp) {
        let waiter_id = Uuid::new_v4();
        let (waiter, woken) = oneshot::channel();
        if this.update(cx, |this, _| this.reconnect_waiters.insert(waiter_id, waiter)).is_err() {
            return;
        }
        let sleeper = cx.background_executor().timer(Duration::from_millis(u64::try_from(backoff_milliseconds).unwrap_or(0)));
        let _ = select(woken, sleeper).await;
        let _ = this.update(cx, |this, _| this.resume_reconnect_waiter(waiter_id));
    }

    fn resume_reconnect_waiter(&mut self, waiter_id: Uuid) {
        if let Some(waiter) = self.reconnect_waiters.remove(&waiter_id) {
            let _ = waiter.send(());
        }
    }

    fn resume_reconnect_waiters(&mut self) {
        for (_, waiter) in self.reconnect_waiters.drain() {
            let _ = waiter.send(());
        }
    }

    fn finish_managed_call(&mut self, call_uuid: Uuid, outcome: RiseRTCSystemCallTerminalOutcome, cx: &mut Context<Self>) {
        let Some(call) = self.calls.get(&call_uuid) else {
            return;
        };
        if !RiseRTCSystemCallRacePolicy::should_finish_managed_call(call.phase) {
            return;
        }
        self.mark_managed_call_ended(call_uuid, Some(outcome), cx);
    }

    /// true while the start or answer that operation_id names is still what
    /// the call wants, nobody hung up and no later operation took over
    fn owns_pending_operation(&self, call_uuid: Uuid, operation_id: Uuid) -> bool {
        let Some(call) = self.calls.get(&call_uuid) else {
            return false;
        };
        !call.end_requested && RiseRTCSystemCallRacePolicy::accepts_async_completion(operation_id, call.operation_id, call.phase)
    }

    /// no socket when the user taps, so start or answer is confirmed now and the screen shows the wait
    fn confirm_if_no_connection(&mut self, resolution: &mut RiseRTCSystemResolution, call_uuid: Uuid, cx: &mut Context<Self>) {
        if SaiWebSocketManager::shared(cx).read(cx).is_connected() {
            return;
        }
        resolution.finish(true);
        self.set_awaiting_connection(true, call_uuid, cx);
    }

    fn set_awaiting_connection(&mut self, awaiting: bool, call_uuid: Uuid, cx: &mut Context<Self>) {
        let Some(call) = self.calls.get_mut(&call_uuid) else {
            return;
        };
        if call.awaiting_connection == awaiting {
            return;
        }
        call.awaiting_connection = awaiting;
        self.calls_did_set(cx);
    }

    fn mark_managed_call_ended(&mut self, call_uuid: Uuid, outcome: Option<RiseRTCSystemCallTerminalOutcome>, cx: &mut Context<Self>) {
        self.engine_warmup_tasks.remove(&call_uuid);
        let Some(call) = self.calls.get_mut(&call_uuid) else {
            return;
        };
        call.phase = RiseRTCSystemCallPhase::Ended;
        call.awaiting_connection = false;
        call.terminal_at.get_or_insert_with(RiseClock::now);
        call.terminal_outcome = outcome.or(call.terminal_outcome).or(Some(RiseRTCSystemCallTerminalOutcome::Ended));
        call.end_requested = true;
        call.operation_id = Some(Uuid::new_v4());
        self.calls_did_set(cx);
        self.operation_tasks.remove(&call_uuid);
        self.schedule_managed_call_removal(call_uuid, cx);
    }

    fn schedule_managed_call_removal(&mut self, call_uuid: Uuid, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MANAGED_CALL_REMOVAL_DELAY).await;
            let _ = this.update(cx, |this, cx| this.remove_managed_call(call_uuid, cx));
        });
        self.cleanup_tasks.insert(call_uuid, task);
    }

    fn remove_managed_call(&mut self, call_uuid: Uuid, cx: &mut Context<Self>) {
        let Some(call) = self.calls.remove(&call_uuid) else {
            return;
        };
        self.calls_did_set(cx);
        if let Some(server_call_id) = call.server_call_id
            && self.server_call_index.get(&server_call_id) == Some(&call_uuid)
        {
            self.server_call_index.remove(&server_call_id);
            self.clear_deferred_signals(&server_call_id);
        }
        self.cleanup_tasks.remove(&call_uuid);
        self.operation_tasks.remove(&call_uuid);
        self.engine_warmup_tasks.remove(&call_uuid);
    }

    /// true if the system owns this calls audio, never on desktop so a call raises its own audio
    pub fn drives_system_audio(&self, server_call_id: &str) -> bool {
        let Some(call) = self.server_call_index.get(server_call_id).and_then(|uuid| self.calls.get(uuid)) else {
            return false;
        };
        !call.system_refused && call.phase.keeps_transport_live()
    }

    fn warm_engine_for_incoming_call(&mut self, call_uuid: Uuid, fence: &RiseRTCSystemAccountFence, cx: &mut Context<Self>) {
        if self.engine_warmup_tasks.contains_key(&call_uuid) {
            return;
        }
        let account_id = fence.account_id.clone();
        let task = cx.spawn(async move |this, cx| {
            let _ = Self::require_client(account_id, RiseRTCSystemDomainBridge::RESTORATION_TIMEOUT, cx).await;
            let _ = this.update(cx, |this, _| {
                this.engine_warmup_tasks.remove(&call_uuid);
            });
        });
        self.engine_warmup_tasks.insert(call_uuid, task);
    }

    /// runs on every change of calls
    fn calls_did_set(&mut self, cx: &mut Context<Self>) {
        self.publish_call_presentation(cx);
        self.sync_transport_hold(cx);
        self.sync_ringback(cx);
        self.schedule_incoming_surface_sync(cx);
        cx.notify();
    }

    fn sync_ringback(&mut self, cx: &mut Context<Self>) {
        let ringing = self.calls.values().find(|call| {
            call.direction == RiseRTCSystemCallDirection::Outgoing && call.phase == RiseRTCSystemCallPhase::OutgoingRinging
        });
        let player = RiseRTCRingbackPlayer::shared(cx);
        match ringing {
            Some(ringing) => {
                let identity = ringing.local_uuid.uuid_string();
                player.update(cx, |player, cx| player.start(&identity, cx));
            }
            None => player.update(cx, |player, _| player.stop()),
        }
    }

    fn sync_transport_hold(&mut self, cx: &mut Context<Self>) {
        let holds = self.calls.values().any(|call| call.phase.keeps_transport_live());
        if holds == self.transport_hold_active {
            return;
        }
        self.transport_hold_active = holds;
        SaiWs::set_transport_hold(TRANSPORT_HOLD_OWNER, holds, cx);
    }

    fn publish_call_presentation(&mut self, cx: &mut Context<Self>) {
        let presentations = self
            .calls
            .values()
            .map(|call| RiseRTCSystemCallPresentation {
                local_uuid: call.local_uuid,
                server_call_id: call.server_call_id.clone(),
                direction: call.direction,
                phase: call.phase,
                remote_user_id: call.remote_user_id.clone(),
                display_name: call.display_name.clone(),
                avatar_url: call.avatar_url.clone(),
                has_video: call.has_video,
                requested_at: call.requested_at,
                connected_at: call.connected_at,
                terminal_at: call.terminal_at,
                terminal_outcome: call.terminal_outcome,
                requires_in_app_answer_ui: call.requires_in_app_answer_ui,
                awaiting_connection: call.awaiting_connection,
            })
            .collect();
        RiseRTCSystemCallPresentationStore::shared(cx).update(cx, |store, cx| store.publish(presentations, cx));
    }

    fn schedule_incoming_surface_sync(&mut self, cx: &mut Context<Self>) {
        if self.incoming_surface_sync_scheduled {
            return;
        }
        self.incoming_surface_sync_scheduled = true;
        let this = cx.entity();
        cx.defer(move |cx| {
            this.update(cx, |this, cx| {
                this.incoming_surface_sync_scheduled = false;
                this.sync_incoming_call_surface(cx);
            });
        });
    }

    fn sync_incoming_call_surface(&mut self, cx: &mut Context<Self>) {
        let ringing = self
            .calls
            .values()
            .find(|call| call.direction == RiseRTCSystemCallDirection::Incoming && call.phase == RiseRTCSystemCallPhase::IncomingRinging && call.server_call_id.is_some())
            .cloned();
        if self.incoming_call_surface.as_ref().map(|surface| surface.local_uuid) != ringing.as_ref().map(|call| call.local_uuid) {
            self.dismiss_incoming_call_surface(cx);
        }
        let Some(ringing) = ringing else {
            return;
        };
        let Some(surface) = self.incoming_call_surface.as_mut() else {
            self.present_incoming_call_surface(&ringing, cx);
            return;
        };
        if surface.display_name == ringing.display_name
            && surface.avatar_url == ringing.avatar_url
            && surface.remote_user_id == ringing.remote_user_id
            && surface.has_video == ringing.has_video
        {
            return;
        }
        surface.display_name = ringing.display_name.clone();
        surface.avatar_url = ringing.avatar_url.clone();
        surface.remote_user_id = ringing.remote_user_id.clone();
        surface.has_video = ringing.has_video;
        if let Some(window) = surface.window {
            let _ = window.update(cx, |view, _, cx| {
                view.set_caller(
                    ringing.display_name.clone(),
                    ringing.avatar_url.clone(),
                    ringing.remote_user_id.clone(),
                    ringing.has_video,
                    cx,
                );
            });
        }
        let server_call_id = surface.server_call_id.clone();
        Self::post_incoming_call_notification(&server_call_id, &ringing, cx);
    }

    fn present_incoming_call_surface(&mut self, ringing: &ManagedCall, cx: &mut Context<Self>) {
        let Some(server_call_id) = ringing.server_call_id.clone() else {
            return;
        };
        let identity = ringing.local_uuid.uuid_string();
        RiseRTCIncomingRingtonePlayer::shared(cx).update(cx, |player, cx| player.start(&identity, cx));
        Self::post_incoming_call_notification(&server_call_id, ringing, cx);

        let view_call_id = server_call_id.clone();
        let display_name = ringing.display_name.clone();
        let avatar_url = ringing.avatar_url.clone();
        let remote_user_id = ringing.remote_user_id.clone();
        let has_video = ringing.has_video;
        let opened = match cx.primary_display() {
            Some(display) => cx
                .open_window(gpui::WindowOptions {
                    app_id: rise_rtc_call_window_hooks().and_then(|hooks| (hooks.app_id)()),
                    ..incoming_window_options(display.visible_bounds(), Some(display.id()))
                }, move |window, cx| {
                    let _ = keep_call_window_above(window);
                    RiseRTCIncomingCallWindow::build(
                        view_call_id,
                        display_name,
                        avatar_url,
                        remote_user_id,
                        has_video,
                        window,
                        cx,
                    )
                })
                .map_err(|error| error.to_string()),
            None => Err("no display".to_owned()),
        };
        let window = match opened {
            Ok(window) => {
                let attention = &mut self.incoming_call_attention;
                let _ = window.update(cx, |_, window, _| attention.begin(window, &server_call_id));
                Some(window)
            }
            Err(error) => {
                Self::log(format!("incoming call window failed: {error}"), cx);
                None
            }
        };
        let window_failed = window.is_none();
        self.incoming_call_surface = Some(IncomingCallSurface {
            local_uuid: ringing.local_uuid,
            server_call_id,
            display_name: ringing.display_name.clone(),
            avatar_url: ringing.avatar_url.clone(),
            remote_user_id: ringing.remote_user_id.clone(),
            has_video: ringing.has_video,
            window,
        });
        if window_failed && let Some(call) = self.calls.get_mut(&ringing.local_uuid) {
            call.requires_in_app_answer_ui = true;
            self.calls_did_set(cx);
        }
    }

    fn post_incoming_call_notification(server_call_id: &str, ringing: &ManagedCall, cx: &mut Context<Self>) {
        let notice = IncomingCallNotice {
            call_id: server_call_id.to_owned(),
            caller_name: ringing.display_name.clone(),
            body: tr("rtc_call_incoming"),
            answer_label: tr("rtc_answer_call"),
            decline_label: tr("rtc_decline_call"),
            avatar: None,
        };
        let support = present_incoming_call(&GpuiNotificationPresenter::new(cx), &notice);
        if !support.is_supported() {
            Self::log("incoming call notification is not available".to_owned(), cx);
        }
    }

    fn dismiss_incoming_call_surface(&mut self, cx: &mut Context<Self>) {
        let Some(surface) = self.incoming_call_surface.take() else {
            return;
        };
        self.incoming_call_attention.end_all();
        RiseRTCIncomingRingtonePlayer::shared(cx).update(cx, |player, _| player.stop());
        let _ = retract_call_notification(&GpuiNotificationPresenter::new(cx), &surface.server_call_id);
        if let Some(window) = surface.window {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn defer_remote_event(&mut self, event: RiseRTCSystemVoIPEvent, server_call_id: &str) {
        if self.deferred_remote_events.get(server_call_id).is_some_and(RiseRTCSystemVoIPEvent::is_terminal) && !event.is_terminal() {
            return;
        }
        let is_terminal = event.is_terminal();
        self.deferred_remote_events.insert(server_call_id.to_owned(), event);
        if is_terminal {
            self.deferred_media_connected.remove(server_call_id);
        }
        self.trim_deferred_signals();
    }

    fn defer_media_connected(&mut self, server_call_id: &str) {
        if self.deferred_remote_events.get(server_call_id).is_some_and(RiseRTCSystemVoIPEvent::is_terminal) {
            return;
        }
        self.deferred_media_connected.insert(server_call_id.to_owned());
        self.trim_deferred_signals();
    }

    fn replay_deferred_signals(&mut self, server_call_id: &str, cx: &mut Context<Self>) {
        let event = self.deferred_remote_events.remove(server_call_id);
        let media_connected = self.deferred_media_connected.remove(server_call_id);
        if let Some(event) = event {
            self.report_remote_call_event(server_call_id, event, cx);
        }
        if media_connected {
            self.report_media_connected(server_call_id, cx);
        }
    }

    fn clear_deferred_signals(&mut self, server_call_id: &str) {
        self.deferred_remote_events.remove(server_call_id);
        self.deferred_media_connected.remove(server_call_id);
    }

    fn trim_deferred_signals(&mut self) {
        while self.deferred_remote_events.len() > DEFERRED_SIGNALS_LIMIT {
            let Some(key) = self.deferred_remote_events.keys().next().cloned() else {
                break;
            };
            self.deferred_remote_events.remove(&key);
            self.deferred_media_connected.remove(&key);
        }
        while self.deferred_media_connected.len() > DEFERRED_SIGNALS_LIMIT {
            let Some(key) = self.deferred_media_connected.iter().next().cloned() else {
                break;
            };
            self.deferred_media_connected.remove(&key);
        }
    }

    fn make_registration(&mut self, token: String, account_id: &str) -> Option<RiseRTCSystemVoIPTokenRegistration> {
        let decision = PushOwnershipGenerationPolicy::registration(
            account_id,
            self.ownership_context.as_ref(),
            PushOwnershipGenerationStore::load_high_water(),
            Self::now_milliseconds(),
        )?;
        if let Some(high_water) = decision.high_water_to_persist {
            PushOwnershipGenerationStore::persist_high_water(high_water).ok()?;
        }
        let installation_id = RiseInstallationIdentity::current().ok().filter(|installation_id| !installation_id.is_empty())?;
        let generation = decision.context.generation;
        self.ownership_context = Some(decision.context);
        Some(RiseRTCSystemVoIPTokenRegistration {
            token,
            environment: RiseRTCSystemVoIPAPNSEnvironment::current(),
            ownership_generation: generation,
            installation_id,
        })
    }

    fn local_call_uuid(&self, server_call_id: &str) -> Option<Uuid> {
        if let Some(indexed) = self.server_call_index.get(server_call_id) {
            return Some(*indexed);
        }
        Uuid::from_uuid_string(server_call_id).filter(|direct| self.calls.contains_key(direct))
    }

    fn update_incoming_caller(&mut self, envelope: &RiseRTCSystemIncomingEnvelope, call_uuid: Uuid, cx: &mut Context<Self>) {
        let Some(caller) = envelope.caller.as_ref() else {
            return;
        };
        let active_account_id = Self::active_account_id(cx);
        let active_account_epoch = Self::active_account_epoch();
        let generation = self.account_generation;
        let Some(call) = self.calls.get_mut(&call_uuid) else {
            return;
        };
        if call.direction != RiseRTCSystemCallDirection::Incoming
            || matches!(call.phase, RiseRTCSystemCallPhase::Ending | RiseRTCSystemCallPhase::Ended)
            || !call.account_fence.matches(active_account_id.as_deref(), active_account_epoch.as_deref(), generation)
        {
            return;
        }
        let remote_user_id = Self::normalized(caller.id.as_deref()).or_else(|| call.remote_user_id.clone());
        let display_name = Self::normalized(caller.name.as_deref())
            .or_else(|| Self::normalized(caller.tag.as_deref()))
            .unwrap_or_else(|| call.display_name.clone());
        let handle = Self::normalized(caller.tag.as_deref())
            .or_else(|| Self::normalized(caller.id.as_deref()))
            .unwrap_or_else(|| call.handle.clone());
        let avatar_url = Self::normalized(caller.avatar_url.as_deref()).or_else(|| call.avatar_url.clone());
        if remote_user_id == call.remote_user_id
            && display_name == call.display_name
            && handle == call.handle
            && avatar_url == call.avatar_url
        {
            return;
        }
        call.remote_user_id = remote_user_id;
        call.display_name = display_name;
        call.handle = handle;
        call.avatar_url = avatar_url;
        self.calls_did_set(cx);
    }

    fn active_account_id(cx: &mut App) -> Option<String> {
        let active = AuthServicesStore::shared(cx).read(cx).active_account_id().map(str::to_owned);
        Self::normalized(active.or_else(AuthLocalStorage::active_account_id).as_deref())
    }

    fn active_account_epoch() -> Option<String> {
        Self::normalized(SaiAccountCacheScope::active_epoch().as_deref())
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        (!value.is_empty()).then(|| value.to_owned())
    }

    fn fence_matches(&self, fence: &RiseRTCSystemAccountFence, cx: &mut App) -> bool {
        fence.matches(Self::active_account_id(cx).as_deref(), Self::active_account_epoch().as_deref(), self.account_generation)
    }

    fn ensure_fence(this: &WeakEntity<Self>, fence: &RiseRTCSystemAccountFence, cx: &mut AsyncApp) -> anyhow::Result<()> {
        if this.update(cx, |this, cx| this.fence_matches(fence, cx))? {
            return Ok(());
        }
        Err(RiseRTCSystemDomainBridgeError::AccountChanged.into())
    }

    fn advance_account_generation(&mut self) {
        self.account_generation = self.account_generation.wrapping_add(1);
        if self.account_generation == 0 {
            self.account_generation = 1;
        }
    }

    fn terminal_outcome(event: &RiseRTCSystemVoIPEvent) -> RiseRTCSystemCallTerminalOutcome {
        match event {
            RiseRTCSystemVoIPEvent::Cancelled => RiseRTCSystemCallTerminalOutcome::Cancelled,
            RiseRTCSystemVoIPEvent::Declined => RiseRTCSystemCallTerminalOutcome::Declined,
            RiseRTCSystemVoIPEvent::TimedOut => RiseRTCSystemCallTerminalOutcome::TimedOut,
            RiseRTCSystemVoIPEvent::AnsweredElsewhere | RiseRTCSystemVoIPEvent::Answered => {
                RiseRTCSystemCallTerminalOutcome::AnsweredElsewhere
            }
            RiseRTCSystemVoIPEvent::Ended | RiseRTCSystemVoIPEvent::Incoming | RiseRTCSystemVoIPEvent::Unknown(_) => {
                RiseRTCSystemCallTerminalOutcome::Ended
            }
        }
    }

    fn safe_call_error_summary(error: &anyhow::Error) -> String {
        let Some(remote_error) = error.downcast_ref::<RiseWireRemoteError>() else {
            return "error_type=local".to_owned();
        };
        let code = Self::bounded_log_field(&remote_error.code, 64);
        let message = Self::bounded_log_field(&remote_error.message, 160);
        if message.is_empty() {
            return format!("remote_code={code}");
        }
        format!("remote_code={code} message={message}")
    }

    fn bounded_log_field(value: &str, limit: usize) -> String {
        let sanitized: String = value.chars().map(|scalar| if scalar.is_control() { ' ' } else { scalar }).collect();
        trimming_whitespaces_and_newlines(&sanitized).chars().take(limit).collect()
    }

    fn lowercased(call_uuid: Uuid) -> String {
        call_uuid.uuid_string().to_lowercase()
    }

    fn now_milliseconds() -> i64 {
        (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64
    }

    async fn require_client(
        account_id: String,
        restoration_timeout: Duration,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Arc<dyn RiseRTCSystemDomainClient>> {
        let request = cx.update(|cx| {
            RiseRTCSystemDomainBridge::shared(cx).update(cx, |bridge, cx| bridge.require_client(account_id, restoration_timeout, cx))
        });
        request.await
    }

    /// the domain client needs the account runtime for its timers and sockets, not the gpui executors
    async fn on_account_runtime<T: Send + 'static>(operation: impl Future<Output = T> + Send + 'static) -> anyhow::Result<T> {
        RiseAccountRuntime::current().spawn(operation).await.map_err(|_| CancellationError.into())
    }

    fn log(message: String, cx: &mut App) {
        Sai::log(LOG_NAME, move || message, SaiLogKind::Network, cx);
    }

    fn log_async(message: String, cx: &mut AsyncApp) {
        cx.update(|cx| Self::log(message, cx));
    }
}

/// one answer per operation, an early confirmed start must not answer again when it completes
struct RiseRTCSystemResolution {
    operation: Option<oneshot::Sender<bool>>,
}

impl RiseRTCSystemResolution {
    fn new(operation: oneshot::Sender<bool>) -> Self {
        Self { operation: Some(operation) }
    }

    fn finish(&mut self, value: bool) {
        if let Some(operation) = self.operation.take() {
            let _ = operation.send(value);
        }
    }
}
