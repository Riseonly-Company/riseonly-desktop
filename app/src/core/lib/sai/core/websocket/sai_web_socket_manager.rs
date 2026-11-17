use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::{mpsc as futures_mpsc, oneshot};
use futures::future::{LocalBoxFuture, Shared};
use futures::{FutureExt, SinkExt, StreamExt};
use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::rise_key::bot as rise_key_bot;
use rise_canon::rise_rpc::LIMITS_MAXIMUM_INBOUND_FRAME_BYTES;
use rise_canon::rise_wire_methods::gateway;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_core::rise_identifiers::RiseRequestID;
use rise_engine::rise_engine::RiseEngineSocketDemand;
use rise_engine::{
    CancellationError, RiseEngine, RiseEngineObservability, RiseWireInboundDisposition,
    RiseWireSocketSession,
};
use rise_platform::system_activity::{
    ApplicationActivityGate, SystemActivityEvent, SystemActivityWatches, watch,
};
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message, WebSocketConfig};

use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication,
};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::engine_bridge::network_transport_config;
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiPayloadPath;
use crate::core::lib::sai::core::websocket::sai_web_socket_configuration::SaiWebSocketConfiguration;
use crate::core::lib::sai::core::websocket::sai_web_socket_environment::SaiWebSocketEnvironment;
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredOutboundCoordinator;
use crate::core::lib::sai::core::websocket::sai_ws_response_debt_coordinator::SaiWsResponseDebtCoordinator;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_entry::{
    SaiHttpTraceEntry, SaiTraceTag, SaiTraceTransport,
};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_recorder::SaiHttpTraceRecorder;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::{
    SaiCompletedWsTraceOptions, SaiHttpTraceStore,
};
#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use crate::core::lib::sai::modules::sai_debugger::sai_ws_ignored_requests_debug::SaiWsIgnoredRequestsDebugRecorder;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_body_deep_compare::{
    SaiWsJSONObjectDeepCompare, SaiWsLastOutboundBodyRegistry,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_coalescing::{
    SaiWsCoalescingConsumeResult, SaiWsCoalescingRegistry,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_envelope::SaiWsEnvelopeBuilder;
use crate::core::lib::sai::modules::sai_ws::sai_ws_event_registry::SaiWsEventRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_coordinator::{
    SaiWsOptimisticCoordinator, SaiWsPreparedOptimisticSend,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_file_upload_runner::SaiWsOptimisticFileUploadRunner;
use crate::core::lib::sai::modules::sai_ws::sai_ws_outbound_cancellation::{
    SaiWsOutboundCancellation, SaiWsOutboundCancelledError,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationError, SaiWsPaginationStateStore,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::{
    SaiWsPendingOnError, SaiWsPendingOnSuccess, SaiWsPendingRegistry,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsPendingCacheBinding, SaiWsRequestOptions, SaiWsRequestOptionsInit, SaiWsSendError,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::{
    SaiWsCacheSelectionSource, SaiWsColdDiskLoaded, SaiWsResponseCache,
};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_coordinator::SaiWsScreenRefreshCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_debug::SaiWsScreenRefreshDebug;
use crate::core::lib::sai::sai::Sai;
use crate::core::lib::sai::sai_auth_hooks::{
    sai_auth_access_token, sai_auth_handle_auth_error, sai_auth_is_expired,
    sai_auth_refresh_access_token,
};
use crate::core::lib::sai::sai_conversation_hooks::sai_conversation_hooks;
use crate::core::lib::sai::sai_guide_hooks::sai_guide_hooks;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::core::rise_engine_bridge::rise_canon_endpoints::RiseCanonEndpoints;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_inbound_ownership_router::{
    RiseInboundOwner, RiseInboundOwnershipRouter,
};

pub const SAI_WS_DID_CONNECT: RiseNotificationName = RiseNotificationName::new("SaiWsDidConnect");
pub const SAI_WS_DID_DISCONNECT: RiseNotificationName =
    RiseNotificationName::new("SaiWsDidDisconnect");

const LOG_NAME: &str = "WS";
const SAI_DEBUG_TOOLS: bool = cfg!(any(debug_assertions, feature = "sai_debug_tools"));
const ACCESS_TOKEN_LEEWAY_SECONDS: f64 = 30.0;
const PRE_WIRE_SOCKET_WAIT: Duration = Duration::from_secs(3);
const PRE_WIRE_SOCKET_POLL: Duration = Duration::from_millis(50);
const CONSECUTIVE_FAILURES_CEILING: u64 = 1_000_000;

#[derive(Clone, Debug, PartialEq)]
pub enum SaiWebSocketAutoReconnectStatus {
    Idle,
    Scheduled {
        attempt_index: u64,
        fire_at: Date,
        last_reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaiWsInboundFrame {
    String(String),
    Data(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsDecodedJSONObject {
    pub value: Map<String, Value>,
    pub raw_data: Vec<u8>,
}

pub enum SaiWsScreenRefreshCacheHitPolicy {}

impl SaiWsScreenRefreshCacheHitPolicy {
    /// caller was already granted the current screen refresh cycle
    /// cached state is only the foreground half so a wire request must follow even when the epoch was valid
    pub fn should_send_granted_revalidation_wire(
        _reload_if_cached: bool,
        _needs_epoch_revalidation: bool,
    ) -> bool {
        true
    }
}

pub enum SaiWsInboundFrameDecoder {}

impl SaiWsInboundFrameDecoder {
    /// parses off the main thread, dropping the future aborts the worker
    pub async fn decode(
        frame: SaiWsInboundFrame,
    ) -> Result<Option<SaiWsDecodedJSONObject>, CancellationError> {
        let worker = RiseAccountRuntime::current().spawn(async move { Self::decode_now(frame) });
        let _abort_on_drop = SaiAbortOnDrop(worker.abort_handle());
        worker.await.map_err(|_| CancellationError)
    }

    fn decode_now(frame: SaiWsInboundFrame) -> Option<SaiWsDecodedJSONObject> {
        let data = match frame {
            SaiWsInboundFrame::String(text) => text.into_bytes(),
            SaiWsInboundFrame::Data(bytes) => {
                std::str::from_utf8(&bytes).ok()?;
                bytes
            }
        };
        let object: Value = serde_json::from_slice(&data).ok()?;
        let Value::Object(dictionary) = object else {
            return None;
        };
        Some(SaiWsDecodedJSONObject {
            value: dictionary,
            raw_data: data,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SaiWebSocketManagerPhase {
    Idle,
    Connecting(Url),
    Connected(Url, Date),
    Disconnected,
    Failed(String),
}

/// what other threads hand to the manager, one by one
enum SaiWebSocketManagerSignal {
    DidBecomeActive,
    NetworkPath(bool),
    SleepWake(SystemActivityEvent),
    SocketDemand(oneshot::Sender<bool>),
}

/// runs a closure on the main thread inside the serialized wire queue
type SaiWireSerialBody<T> =
    Box<dyn FnOnce(AsyncApp) -> LocalBoxFuture<'static, anyhow::Result<T>> + 'static>;

pub struct SaiWebSocketManager {
    phase: SaiWebSocketManagerPhase,

    web_socket_task: Option<Arc<SaiWebSocketTask>>,
    receive_loop_task: Option<Task<()>>,
    heartbeat_task: Option<Task<()>>,
    wire_serial_tail: Option<Shared<Task<()>>>,

    user_stopped_reconnect: bool,
    reported_application_active: Option<bool>,
    application_activity: ApplicationActivityGate,
    transport_hold_owners: HashSet<String>,

    last_connect_access_token: Option<String>,

    consecutive_transport_failures: u64,
    upgrade_auth_recovery_armed: bool,

    auto_reconnect_task: Option<Task<()>>,
    last_demand_wake_at: Option<Duration>,
    durable_outbound_waiting_ids: HashSet<RiseRequestID>,
    foreground_read_waiting: bool,
    transport_ever_connected: bool,

    _network_path_monitor: SystemActivityWatches,
    network_path_satisfied: Option<bool>,

    transport_epoch: u64,
    connect_notification_epoch: Option<u64>,
    transport_account_user_id: Option<String>,
    transport_account_epoch: Option<String>,

    handshake_ping_task: Option<Task<()>>,
    handshake_watchdog_task: Option<Task<()>>,
    foreground_probe_task: Option<Task<()>>,
    last_inbound_at: Duration,

    auto_reconnect_status: SaiWebSocketAutoReconnectStatus,

    _did_become_active_observer: RiseSubscription,
    _signal_loop: Task<()>,
}

impl SaiWebSocketManager {
    const FOREGROUND_READ_BACKOFF_STEP_CAP: u64 = 3;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (signals, mut inbox) = futures_mpsc::unbounded::<SaiWebSocketManagerSignal>();
        let signal_loop = cx.spawn(async move |this, cx| {
            while let Some(signal) = inbox.next().await {
                if this
                    .update(cx, |manager, cx| manager.handle_signal(signal, cx))
                    .is_err()
                {
                    return;
                }
            }
        });

        let active_signals = signals.clone();
        let did_become_active_observer = RiseAppEvents::default_center().observe(
            UIApplication::DID_BECOME_ACTIVE_NOTIFICATION,
            move |_| {
                let _ = active_signals.unbounded_send(SaiWebSocketManagerSignal::DidBecomeActive);
            },
        );

        let path_signals = signals.clone();
        let network_path_monitor = watch(move |event| {
            let signal = match event {
                SystemActivityEvent::NetworkPath { satisfied } => {
                    SaiWebSocketManagerSignal::NetworkPath(satisfied)
                }
                SystemActivityEvent::WillSleep | SystemActivityEvent::DidWake => {
                    SaiWebSocketManagerSignal::SleepWake(event)
                }
            };
            let _ = path_signals.unbounded_send(signal);
        });

        let mut application_activity = ApplicationActivityGate::default();
        application_activity.window_visibility(true);

        let demand_signals = signals;
        let socket_demand: RiseEngineSocketDemand = Arc::new(move || {
            let demand_signals = demand_signals.clone();
            async move {
                let (reply, answer) = oneshot::channel();
                if demand_signals
                    .unbounded_send(SaiWebSocketManagerSignal::SocketDemand(reply))
                    .is_err()
                {
                    return true;
                }
                answer.await.unwrap_or(true)
            }
            .boxed()
        });
        RiseEngine::set_socket_demand(Some(socket_demand));

        Self {
            phase: SaiWebSocketManagerPhase::Idle,
            web_socket_task: None,
            receive_loop_task: None,
            heartbeat_task: None,
            wire_serial_tail: None,
            user_stopped_reconnect: true,
            reported_application_active: None,
            application_activity,
            transport_hold_owners: HashSet::new(),
            last_connect_access_token: None,
            consecutive_transport_failures: 0,
            upgrade_auth_recovery_armed: true,
            auto_reconnect_task: None,
            last_demand_wake_at: None,
            durable_outbound_waiting_ids: HashSet::new(),
            foreground_read_waiting: false,
            transport_ever_connected: false,
            _network_path_monitor: network_path_monitor,
            network_path_satisfied: None,
            transport_epoch: 0,
            connect_notification_epoch: None,
            transport_account_user_id: None,
            transport_account_epoch: None,
            handshake_ping_task: None,
            handshake_watchdog_task: None,
            foreground_probe_task: None,
            last_inbound_at: RiseClock::monotonic(),
            auto_reconnect_status: SaiWebSocketAutoReconnectStatus::Idle,
            _did_become_active_observer: did_become_active_observer,
            _signal_loop: signal_loop,
        }
    }

    fn handle_signal(&mut self, signal: SaiWebSocketManagerSignal, cx: &mut Context<Self>) {
        match signal {
            SaiWebSocketManagerSignal::DidBecomeActive => {
                self.application_activity.window_visibility(true);
                if self.reported_application_active != Some(true) {
                    let token = self.last_connect_access_token.clone();
                    self.set_application_active(true, token, cx);
                }
            }
            SaiWebSocketManagerSignal::NetworkPath(satisfied) => {
                let satisfied = satisfied && !network_transport_config::tester_network_down();
                self.network_path_changed(satisfied, cx);
            }
            SaiWebSocketManagerSignal::SleepWake(event) => {
                if let Some(active) = self.application_activity.system_event(event)
                    && self.reported_application_active != Some(active)
                {
                    let token = self.last_connect_access_token.clone();
                    self.set_application_active(active, token, cx);
                }
            }
            SaiWebSocketManagerSignal::SocketDemand(reply) => {
                let _ = reply.send(self.answer_engine_socket_demand(cx));
            }
        }
    }

    pub fn phase(&self) -> &SaiWebSocketManagerPhase {
        &self.phase
    }

    pub fn auto_reconnect_status(&self) -> &SaiWebSocketAutoReconnectStatus {
        &self.auto_reconnect_status
    }

    /// a connect after a failed one, its connecting phase alone would read as online
    pub fn is_reconnecting_after_failure(&self) -> bool {
        self.consecutive_transport_failures > 0
    }

    fn set_phase(&mut self, phase: SaiWebSocketManagerPhase, cx: &mut Context<Self>) {
        self.phase = phase;
        cx.notify();
    }

    fn set_auto_reconnect_status(
        &mut self,
        status: SaiWebSocketAutoReconnectStatus,
        cx: &mut Context<Self>,
    ) {
        self.auto_reconnect_status = status;
        cx.notify();
    }

    fn application_is_active(&self, cx: &App) -> bool {
        self.reported_application_active
            .unwrap_or_else(|| RiseAppLifecycle::scene_phase(cx) != ScenePhase::Background)
    }

    fn transport_is_live(&self, cx: &App) -> bool {
        self.application_is_active(cx) || !self.transport_hold_owners.is_empty()
    }

    fn durable_outbound_waiting(&self) -> bool {
        !self.durable_outbound_waiting_ids.is_empty()
    }

    fn is_current_task(&self, task: &Arc<SaiWebSocketTask>) -> bool {
        self.web_socket_task
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, task))
    }

    fn drop_web_socket_task(&mut self, code: SaiWebSocketCloseCode) {
        if let Some(task) = self.web_socket_task.take() {
            task.cancel(code);
        }
    }

    fn network_path_changed(&mut self, satisfied: bool, cx: &mut Context<Self>) {
        let was_satisfied = self.network_path_satisfied;
        self.network_path_satisfied = Some(satisfied);
        if was_satisfied != Some(satisfied) {
            cx.notify();
        }
        if !satisfied || was_satisfied != Some(false) {
            return;
        }
        if self.auto_reconnect_task.is_none()
            || !self.transport_is_live(cx)
            || self.user_stopped_reconnect
        {
            return;
        }
        if self.is_connected() || self.is_connecting() {
            return;
        }
        let Some(token) = self.resolve_reconnect_access_token(cx) else {
            return;
        };
        self.cancel_auto_reconnect();
        self.consecutive_transport_failures = 0;
        self.set_auto_reconnect_status(SaiWebSocketAutoReconnectStatus::Idle, cx);
        Sai::log(
            LOG_NAME,
            || "network is back — reconnecting now".to_owned(),
            SaiLogKind::Websocket,
            cx,
        );
        self.connect(&token, cx);
    }

    pub fn tester_network_changed(&mut self, up: bool, cx: &mut Context<Self>) {
        self.network_path_changed(up, cx);
        if up || !(self.is_connected() || self.is_connecting()) {
            return;
        }
        let reason = "network switched off (dev-drive)".to_owned();
        self.cancel_handshake_tasks();
        self.stop_application_heartbeat();
        self.receive_loop_task = None;
        self.drop_web_socket_task(SaiWebSocketCloseCode::AbnormalClosure);
        self.reset_transport_state_for_dropped_socket(reason.clone(), cx);
        self.set_phase(SaiWebSocketManagerPhase::Disconnected, cx);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Socket, || "disconnected".to_owned());
        self.bump_transport_epoch();
        self.schedule_auto_reconnect(reason, cx);
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        let (user_id, epoch, transport_epoch) = self.transport_identity();
        RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| {
            runtime.socket_disconnected(
                user_id,
                epoch,
                transport_epoch,
                "WebSocket manager reset".to_owned(),
                cx,
            )
        });
        self.drop_web_socket_task(SaiWebSocketCloseCode::AbnormalClosure);
        self.receive_loop_task = None;
        self.heartbeat_task = None;
        self.wire_serial_tail = None;
        self.auto_reconnect_task = None;
        self.handshake_ping_task = None;
        self.handshake_watchdog_task = None;
        self.foreground_probe_task = None;

        self.phase = SaiWebSocketManagerPhase::Idle;
        self.user_stopped_reconnect = true;
        self.reported_application_active = None;
        self.last_connect_access_token = None;
        self.consecutive_transport_failures = 0;
        self.durable_outbound_waiting_ids.clear();
        self.foreground_read_waiting = false;
        self.transport_ever_connected = false;
        self.bump_transport_epoch();
        self.transport_account_user_id = None;
        self.transport_account_epoch = None;
        self.auto_reconnect_status = SaiWebSocketAutoReconnectStatus::Idle;
        cx.notify();
    }

    fn transport_identity(&self) -> (Option<String>, Option<String>, u64) {
        (
            self.transport_account_user_id.clone(),
            self.transport_account_epoch.clone(),
            self.transport_epoch,
        )
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.phase, SaiWebSocketManagerPhase::Connected(..))
    }

    pub fn is_connecting(&self) -> bool {
        matches!(self.phase, SaiWebSocketManagerPhase::Connecting(_))
    }

    /// true only while the network path is known to be down (system path or tester switch)
    /// unknown counts as up
    pub fn is_network_path_down(&self) -> bool {
        self.network_path_satisfied == Some(false)
    }

    pub fn should_defer_tracked_outbound(
        track_until_acknowledged: bool,
        is_connected: bool,
    ) -> bool {
        track_until_acknowledged && !is_connected
    }

    pub fn active_url_display_string(&self) -> String {
        match &self.phase {
            SaiWebSocketManagerPhase::Connecting(url)
            | SaiWebSocketManagerPhase::Connected(url, _) => url.absolute_string().to_owned(),
            SaiWebSocketManagerPhase::Failed(_)
            | SaiWebSocketManagerPhase::Idle
            | SaiWebSocketManagerPhase::Disconnected => {
                SaiWebSocketEnvironment::resolved_web_socket_display_string()
            }
        }
    }

    pub fn connected_since(&self) -> Option<Date> {
        match &self.phase {
            SaiWebSocketManagerPhase::Connected(_, since) => Some(*since),
            _ => None,
        }
    }

    pub fn connect_if_needed(&mut self, access_token: Option<&str>, cx: &mut Context<Self>) {
        let token = access_token.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if token.is_empty() {
            Sai::log(
                LOG_NAME,
                || "connectIfNeeded skipped — no access token".to_owned(),
                SaiLogKind::Websocket,
                cx,
            );
            return;
        }
        if !self.transport_is_live(cx) {
            self.last_connect_access_token = Some(token.to_owned());
            return;
        }
        self.cancel_auto_reconnect();
        if self.is_connected() {
            return;
        }
        self.connect(token, cx);
    }

    pub fn reconnect_with_access_token(&mut self, access_token: &str, cx: &mut Context<Self>) {
        let token = trimming_whitespaces_and_newlines(access_token);
        if token.is_empty() {
            return;
        }
        self.cancel_auto_reconnect();
        self.user_stopped_reconnect = false;
        self.last_connect_access_token = Some(token.to_owned());
        tracing::warn!(target: "riseonly::X753", "reconnect after refresh live={}", self.transport_is_live(cx));
        self.disconnect_silently(cx);
        self.connect(token, cx);
        if self.web_socket_task.is_none() && (self.is_connected() || self.is_connecting()) {
            self.set_phase(SaiWebSocketManagerPhase::Disconnected, cx);
            self.bump_transport_epoch();
            tracing::warn!(target: "riseonly::X1525", "refresh reconnect parked, phase disconnected");
        }
        Sai::log(
            LOG_NAME,
            || "reconnected after token refresh".to_owned(),
            SaiLogKind::Websocket,
            cx,
        );
    }

    pub fn connect(&mut self, access_token: &str, cx: &mut Context<Self>) {
        let token = trimming_whitespaces_and_newlines(access_token).to_owned();
        if token.is_empty() {
            Sai::log(
                LOG_NAME,
                || "connect skipped — empty access token".to_owned(),
                SaiLogKind::Websocket,
                cx,
            );
            return;
        }
        if MaintenanceGate::is_closed() {
            self.cancel_auto_reconnect();
            self.user_stopped_reconnect = true;
            Sai::log(
                LOG_NAME,
                || "connect refused — product is closed for maintenance".to_owned(),
                SaiLogKind::Websocket,
                cx,
            );
            return;
        }
        if !self.transport_is_live(cx) {
            self.last_connect_access_token = Some(token);
            self.user_stopped_reconnect = false;
            return;
        }

        if sai_auth_is_expired(&token, ACCESS_TOKEN_LEEWAY_SECONDS) {
            self.cancel_auto_reconnect();
            self.user_stopped_reconnect = false;
            Sai::log(
                LOG_NAME,
                || "connect deferred — access token expired, refreshing".to_owned(),
                SaiLogKind::Websocket,
                cx,
            );
            let refresh = sai_auth_refresh_access_token(cx);
            cx.spawn(async move |this, cx| {
                if refresh.await.is_ok() {
                    return;
                }
                let _ = this.update(cx, |manager, cx| match sai_auth_access_token(cx) {
                    Some(stored) => manager.open_socket(stored, cx),
                    None => {
                        Sai::log(
                            LOG_NAME,
                            || "connect cancelled — session cleared during refresh".to_owned(),
                            SaiLogKind::Websocket,
                            cx,
                        );
                        manager.set_auto_reconnect_status(
                            SaiWebSocketAutoReconnectStatus::Idle,
                            cx,
                        );
                    }
                });
            })
            .detach();
            return;
        }

        self.open_socket(token, cx);
    }

    fn open_socket(&mut self, token: String, cx: &mut Context<Self>) {
        if !self.transport_is_live(cx) {
            self.last_connect_access_token = Some(token);
            self.user_stopped_reconnect = false;
            return;
        }
        self.cancel_auto_reconnect();
        self.user_stopped_reconnect = false;
        self.last_connect_access_token = Some(token.clone());

        self.disconnect_silently(cx);

        self.transport_epoch = self.transport_epoch.wrapping_add(1);
        let epoch = self.transport_epoch;
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        self.transport_account_user_id = account_user_id.clone();
        self.transport_account_epoch = account_epoch.clone();

        let url = SaiWebSocketEnvironment::resolved_web_socket_url();
        self.set_phase(SaiWebSocketManagerPhase::Connecting(url.clone()), cx);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Socket, || "connecting".to_owned());
        Sai::log(
            LOG_NAME,
            || "connecting (subprotocol Bearer…)".to_owned(),
            SaiLogKind::Websocket,
            cx,
        );

        let task = SaiWebSocketTask::resume(
            &url,
            &format!("Bearer.{token}"),
            usize::try_from(LIMITS_MAXIMUM_INBOUND_FRAME_BYTES).unwrap_or(usize::MAX),
        );
        self.web_socket_task = Some(Arc::clone(&task));

        let loop_task = Arc::clone(&task);
        let loop_user_id = account_user_id.clone();
        let loop_epoch = account_epoch.clone();
        self.receive_loop_task = Some(cx.spawn(async move |this, cx| {
            Self::run_receive_loop(this, loop_task, epoch, loop_user_id, loop_epoch, cx).await;
        }));

        let ping_task = Arc::clone(&task);
        let ping_delay = Duration::from_nanos(SaiWebSocketConfiguration::POST_OPEN_PING_DELAY_NANOSECONDS);
        let ping_timer = cx.background_executor().timer(ping_delay);
        self.handshake_ping_task = Some(cx.spawn(async move |this, cx| {
            ping_timer.await;
            let _ = this.update(cx, |manager, cx| {
                if manager.transport_epoch != epoch || !manager.is_current_task(&ping_task) {
                    return;
                }
                manager.begin_handshake_ping(epoch, account_user_id, account_epoch, cx);
            });
        }));

        let watchdog_task = task;
        let watchdog_delay =
            Duration::from_nanos(SaiWebSocketConfiguration::HANDSHAKE_WATCHDOG_NANOSECONDS);
        let watchdog_timer = cx.background_executor().timer(watchdog_delay);
        self.handshake_watchdog_task = Some(cx.spawn(async move |this, cx| {
            watchdog_timer.await;
            let _ = this.update(cx, |manager, cx| {
                if manager.transport_epoch != epoch || !manager.is_current_task(&watchdog_task) {
                    return;
                }
                if !manager.is_connecting() {
                    return;
                }
                Sai::log(
                    LOG_NAME,
                    || "handshake watchdog — still connecting, recycling socket".to_owned(),
                    SaiLogKind::Websocket,
                    cx,
                );
                manager.teardown_stale_connecting_and_reconnect(
                    "handshake watchdog timeout".to_owned(),
                    cx,
                );
            });
        }));
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.user_stopped_reconnect = true;
        self.cancel_auto_reconnect();
        self.set_auto_reconnect_status(SaiWebSocketAutoReconnectStatus::Idle, cx);

        self.stop_application_heartbeat();
        self.cancel_handshake_tasks();
        self.receive_loop_task = None;

        self.drop_web_socket_task(SaiWebSocketCloseCode::GoingAway);

        self.reset_transport_state_for_dropped_socket("WebSocket disconnected (user)".to_owned(), cx);
        self.set_phase(SaiWebSocketManagerPhase::Disconnected, cx);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Socket, || "disconnected".to_owned());
        self.bump_transport_epoch();
        let not_connected = anyhow::Error::new(SaiWsSendError::NotConnected);
        cx.defer(move |cx| {
            SaiWsDeferredOutboundCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.fail_all_pending(not_connected, cx));
        });
        self.durable_outbound_waiting_ids.clear();
        self.foreground_read_waiting = false;
        Sai::log(LOG_NAME, || "disconnect (user)".to_owned(), SaiLogKind::Websocket, cx);
    }

    pub fn set_application_active(
        &mut self,
        active: bool,
        access_token: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.reported_application_active = Some(active);
        let (user_id, epoch, transport_epoch) = self.transport_identity();
        RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| {
            runtime.set_application_active(active, user_id, epoch, transport_epoch, cx)
        });
        let access_token = access_token
            .map(|token| trimming_whitespaces_and_newlines(&token).to_owned())
            .filter(|token| !token.is_empty());
        if let Some(token) = &access_token {
            self.last_connect_access_token = Some(token.clone());
        }
        let token = access_token.or_else(|| self.last_connect_access_token.clone());

        if !active {
            self.foreground_probe_task = None;
            if !self.transport_hold_owners.is_empty() {
                if self.is_connected() {
                    self.start_application_heartbeat(cx);
                } else {
                    self.connect_if_needed(token.as_deref(), cx);
                }
                return;
            }
            self.stop_application_heartbeat();
            self.cancel_auto_reconnect();
            self.set_auto_reconnect_status(SaiWebSocketAutoReconnectStatus::Idle, cx);
            return;
        }

        if self.is_connected() {
            self.start_application_heartbeat(cx);
            self.probe_socket_after_foreground(cx);
        } else {
            self.connect_if_needed(token.as_deref(), cx);
        }
    }

    fn probe_socket_after_foreground(&mut self, cx: &mut Context<Self>) {
        self.probe_socket("foreground probe", cx);
    }

    fn probe_socket(&mut self, reason: &'static str, cx: &mut Context<Self>) {
        let Some(task) = self.web_socket_task.clone() else {
            return;
        };
        let epoch = self.transport_epoch;
        let watchdog_delay =
            Duration::from_nanos(SaiWebSocketConfiguration::HANDSHAKE_WATCHDOG_NANOSECONDS);
        let watchdog_timer = cx.background_executor().timer(watchdog_delay);
        let pong = task.send_ping();
        self.foreground_probe_task = Some(cx.spawn(async move |this, cx| {
            let outcome = futures::future::select(Box::pin(pong), Box::pin(watchdog_timer)).await;
            let failure = match outcome {
                futures::future::Either::Left((Ok(()), _)) => return,
                futures::future::Either::Left((Err(error), _)) => format!("{reason}: {error}"),
                futures::future::Either::Right(_) => format!("{reason} timeout"),
            };
            let _ = this.update(cx, |manager, cx| {
                manager.recycle_connected_socket(&task, epoch, failure, cx);
            });
        }));
    }

    fn recycle_connected_socket(
        &mut self,
        task: &Arc<SaiWebSocketTask>,
        expected_epoch: u64,
        reason: String,
        cx: &mut Context<Self>,
    ) {
        if self.transport_epoch != expected_epoch || !self.is_current_task(task) || !self.is_connected()
        {
            return;
        }
        self.stop_application_heartbeat();
        self.cancel_handshake_tasks();
        self.receive_loop_task = None;
        self.drop_web_socket_task(SaiWebSocketCloseCode::AbnormalClosure);
        self.reset_transport_state_for_dropped_socket(reason.clone(), cx);
        self.set_phase(SaiWebSocketManagerPhase::Disconnected, cx);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Socket, || "disconnected".to_owned());
        self.bump_transport_epoch();
        let logged = reason.clone();
        Sai::log(LOG_NAME, move || logged, SaiLogKind::Websocket, cx);
        self.schedule_auto_reconnect(reason, cx);
    }

    /// transport hold by a named owner
    /// the owner key makes it idempotent so a repeated acquire or lost release doesnt pile up
    /// overlapping owners dont cancel each other
    pub fn set_transport_hold(&mut self, owner: &str, active: bool, cx: &mut Context<Self>) {
        let changed = if active {
            self.transport_hold_owners.insert(owner.to_owned())
        } else {
            self.transport_hold_owners.remove(owner)
        };
        if !changed || self.application_is_active(cx) {
            return;
        }

        if self.transport_hold_owners.is_empty() {
            self.stop_application_heartbeat();
            self.cancel_auto_reconnect();
            self.set_auto_reconnect_status(SaiWebSocketAutoReconnectStatus::Idle, cx);
        } else if self.is_connected() {
            self.start_application_heartbeat(cx);
        } else {
            let token = self.last_connect_access_token.clone();
            self.connect_if_needed(token.as_deref(), cx);
        }
    }

    fn disconnect_silently(&mut self, cx: &mut Context<Self>) {
        self.stop_application_heartbeat();
        self.cancel_handshake_tasks();
        self.receive_loop_task = None;
        self.drop_web_socket_task(SaiWebSocketCloseCode::AbnormalClosure);
        self.reset_transport_state_for_dropped_socket(
            "WebSocket reconnecting / silent close".to_owned(),
            cx,
        );
    }

    fn bump_transport_epoch(&mut self) {
        self.transport_epoch = self.transport_epoch.wrapping_add(1);
    }

    fn cancel_handshake_tasks(&mut self) {
        self.handshake_ping_task = None;
        self.handshake_watchdog_task = None;
    }

    fn teardown_stale_connecting_and_reconnect(&mut self, reason: String, cx: &mut Context<Self>) {
        if !self.is_connecting() {
            return;
        }
        self.cancel_handshake_tasks();
        self.stop_application_heartbeat();
        self.receive_loop_task = None;
        self.drop_web_socket_task(SaiWebSocketCloseCode::AbnormalClosure);
        self.reset_transport_state_for_dropped_socket(reason.clone(), cx);
        self.set_phase(SaiWebSocketManagerPhase::Disconnected, cx);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Socket, || "disconnected".to_owned());
        self.bump_transport_epoch();
        self.schedule_auto_reconnect(reason, cx);
    }

    fn reset_transport_state_for_dropped_socket(&mut self, reason: String, cx: &mut Context<Self>) {
        let reason_for_trace = SAI_DEBUG_TOOLS.then(|| reason.clone());
        let (user_id, epoch, transport_epoch) = self.transport_identity();
        RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| {
            runtime.socket_disconnected(user_id, epoch, transport_epoch, reason, cx)
        });
        self.transport_account_user_id = None;
        self.transport_account_epoch = None;
        self.connect_notification_epoch = None;
        SaiWsResponseDebtCoordinator::shared(cx)
            .update(cx, |debt, _| debt.prepare_for_socket_transport_drop());
        SaiWsPendingRegistry::shared(cx)
            .update(cx, |registry, cx| registry.reset_after_socket_drop(cx));
        RiseAppEvents::default_center().post(SAI_WS_DID_DISCONNECT);
        if let Some(reason) = reason_for_trace {
            SaiHttpTraceStore::shared(cx)
                .update(cx, |store, cx| store.fail_all_awaiting_ws(&reason, cx));
        }
    }

    fn begin_handshake_ping(
        &mut self,
        expected_epoch: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.transport_epoch != expected_epoch {
            return;
        }
        let Some(task) = self.web_socket_task.clone() else {
            return;
        };
        let pong = task.send_ping();
        cx.spawn(async move |this, cx| {
            let result = pong.await;
            let _ = this.update(cx, |manager, cx| {
                manager.finish_handshake_ping(
                    &task,
                    result,
                    expected_epoch,
                    account_user_id,
                    account_epoch,
                    cx,
                );
            });
        })
        .detach();
    }

    fn finish_handshake_ping(
        &mut self,
        task: &Arc<SaiWebSocketTask>,
        result: Result<(), String>,
        expected_epoch: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.transport_epoch != expected_epoch
            || !self.is_current_task(task)
            || !SaiAccountCacheScope::is_current_account_epoch(
                account_user_id.as_deref(),
                account_epoch.as_deref(),
            )
        {
            return;
        }

        self.handshake_watchdog_task = None;

        match result {
            Err(error) => {
                self.stop_application_heartbeat();
                self.cancel_handshake_tasks();
                self.receive_loop_task = None;
                self.drop_web_socket_task(SaiWebSocketCloseCode::AbnormalClosure);
                self.reset_transport_state_for_dropped_socket(
                    format!("WebSocket ping failed: {error}"),
                    cx,
                );
                self.set_phase(SaiWebSocketManagerPhase::Disconnected, cx);
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Socket, || {
                    "disconnected".to_owned()
                });
                self.bump_transport_epoch();
                let logged = error.clone();
                Sai::log(LOG_NAME, move || format!("ping failed: {logged}"), SaiLogKind::Websocket, cx);
                self.recover_session_if_upgrade_unauthorized(task, cx);
                self.schedule_auto_reconnect(format!("ping: {error}"), cx);
            }
            Ok(()) => {
                let SaiWebSocketManagerPhase::Connecting(url) = self.phase.clone() else {
                    return;
                };
                self.consecutive_transport_failures = 0;
                self.upgrade_auth_recovery_armed = true;
                self.durable_outbound_waiting_ids.clear();
                self.foreground_read_waiting = false;
                self.transport_ever_connected = true;
                self.auto_reconnect_status = SaiWebSocketAutoReconnectStatus::Idle;
                self.set_phase(SaiWebSocketManagerPhase::Connected(url, Date::now()), cx);
                self.last_inbound_at = RiseClock::monotonic();
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Socket, || "connected".to_owned());
                RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| {
                    runtime.socket_connected(
                        account_user_id,
                        account_epoch,
                        expected_epoch,
                        false,
                        cx,
                    )
                });
                Sai::log(LOG_NAME, || "connected".to_owned(), SaiLogKind::Websocket, cx);
                self.start_application_heartbeat(cx);
                let manager = cx.entity();
                cx.defer(move |cx| {
                    SaiWsDeferredOutboundCoordinator::shared(cx).update(cx, |coordinator, cx| {
                        coordinator.notify_connected(manager.clone(), cx)
                    });
                    SaiWsResponseDebtCoordinator::shared(cx)
                        .update(cx, |debt, cx| debt.schedule_flush_when_connected(manager, cx));
                });
            }
        }
    }

    async fn run_receive_loop(
        this: WeakEntity<Self>,
        task: Arc<SaiWebSocketTask>,
        expected_epoch: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) {
        let mut release_on_drop = SaiReleaseSocketOnDrop(Some(Arc::clone(&task)));

        let installation = cx.update(|cx| {
            RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| {
                runtime.socket_session_started(
                    account_user_id.clone(),
                    account_epoch.clone(),
                    expected_epoch,
                    cx,
                )
            })
        });
        let _ = installation.await;

        let still_current = |manager: &Self| {
            manager.transport_epoch == expected_epoch
                && manager.is_current_task(&task)
                && SaiAccountCacheScope::is_current_account_epoch(
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                )
        };

        loop {
            match task.receive().await {
                Ok(frame) => {
                    let Ok(current) = this.update(cx, |manager, _| {
                        manager.last_inbound_at = RiseClock::monotonic();
                        still_current(manager)
                    }) else {
                        return;
                    };
                    if !current {
                        cx.update(|cx| Sai::log(
                            LOG_NAME,
                            || "ignored frame from stale account transport".to_owned(),
                            SaiLogKind::Cache,
                            cx,
                        ));
                        release_on_drop.disarm();
                        return;
                    }
                    match &frame {
                        SaiWsInboundFrame::String(text) => {
                            let count = text.chars().count();
                            cx.update(|cx| Sai::log(
                                LOG_NAME,
                                move || format!("← string ({count} chars)"),
                                SaiLogKind::Websocket,
                                cx,
                            ));
                        }
                        SaiWsInboundFrame::Data(data) => {
                            let count = data.len();
                            cx.update(|cx| Sai::log(
                                LOG_NAME,
                                move || format!("← data ({count} bytes)"),
                                SaiLogKind::Websocket,
                                cx,
                            ));
                        }
                    }
                    let Ok(Some(decoded)) = SaiWsInboundFrameDecoder::decode(frame).await else {
                        continue;
                    };
                    let Ok(current) = this.read_with(&*cx, |manager, _| still_current(manager))
                    else {
                        return;
                    };
                    if !current {
                        release_on_drop.disarm();
                        return;
                    }
                    Self::dispatch_incoming(
                        decoded,
                        account_user_id.clone(),
                        account_epoch.clone(),
                        expected_epoch,
                        cx,
                    )
                    .await;
                }
                Err(message) => {
                    let _ = this.update(cx, |manager, cx| {
                        if manager.transport_epoch != expected_epoch || !manager.is_current_task(&task)
                        {
                            return;
                        }
                        manager.stop_application_heartbeat();
                        manager.drop_web_socket_task(SaiWebSocketCloseCode::AbnormalClosure);
                        manager.reset_transport_state_for_dropped_socket(
                            format!("WebSocket receive ended: {message}"),
                            cx,
                        );
                        manager.set_phase(SaiWebSocketManagerPhase::Disconnected, cx);
                        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Socket, || {
                            "disconnected".to_owned()
                        });
                        manager.bump_transport_epoch();
                        let logged = message.clone();
                        Sai::log(
                            LOG_NAME,
                            move || format!("receive ended: {logged}"),
                            SaiLogKind::Websocket,
                            cx,
                        );
                        manager.recover_session_if_upgrade_unauthorized(&task, cx);
                        manager.schedule_auto_reconnect(format!("receive: {message}"), cx);
                        manager.receive_loop_task = None;
                    });
                    release_on_drop.disarm();
                    return;
                }
            }
        }
    }

    /// an upgrade refused with 401 means a stale token or revoked session
    /// backoff alone would keep the app on cache forever
    /// refresh once per failure series and a refused refresh signs out
    fn recover_session_if_upgrade_unauthorized(
        &mut self,
        task: &Arc<SaiWebSocketTask>,
        cx: &mut Context<Self>,
    ) {
        let unauthorized = tokio_tungstenite::tungstenite::http::StatusCode::UNAUTHORIZED.as_u16();
        if !self.upgrade_auth_recovery_armed || task.upgrade_status() != Some(unauthorized) {
            return;
        }
        self.upgrade_auth_recovery_armed = false;
        Sai::log(
            LOG_NAME,
            || "upgrade refused 401, recovering session".to_owned(),
            SaiLogKind::Websocket,
            cx,
        );
        sai_auth_handle_auth_error(cx);
    }

    fn cancel_auto_reconnect(&mut self) {
        self.auto_reconnect_task = None;
    }

    fn wake_transport_on_demand(&mut self, cx: &mut Context<Self>) {
        if self.is_connected()
            || self.is_connecting()
            || self.auto_reconnect_task.is_none()
            || self.user_stopped_reconnect
            || self.network_path_satisfied == Some(false)
        {
            return;
        }
        let now = RiseClock::monotonic();
        let spacing =
            Duration::from_nanos(SaiWebSocketConfiguration::RECONNECT_BASE_DELAY_NANOSECONDS);
        if let Some(last) = self.last_demand_wake_at
            && now.saturating_sub(last) < spacing
        {
            return;
        }
        self.last_demand_wake_at = Some(now);
        let token = self.resolve_reconnect_access_token(cx);
        self.connect_if_needed(token.as_deref(), cx);
    }

    /// a durable send was queued or paused without a socket
    /// the planned long backoff attempt fires now and later failures retry on the first step
    pub fn note_durable_outbound_waiting(&mut self, request_id: RiseRequestID, cx: &mut Context<Self>) {
        if self.is_connected() {
            return;
        }
        self.durable_outbound_waiting_ids.insert(request_id);
        self.wake_transport_on_demand(cx);
    }

    /// the send left the outbox (acked, deleted or out of attempts)
    /// so it no longer keeps the reconnect on the first backoff step
    pub fn note_durable_outbound_settled(&mut self, request_id: RiseRequestID) {
        self.durable_outbound_waiting_ids.remove(&request_id);
    }

    fn answer_engine_socket_demand(&mut self, cx: &mut Context<Self>) -> bool {
        self.wake_transport_on_demand(cx);
        let active = self.application_is_active(cx);
        if active && !self.is_connected() && self.transport_ever_connected {
            self.foreground_read_waiting = true;
        }
        if self.is_connected() || self.is_connecting() || !active {
            return true;
        }
        self.auto_reconnect_task.is_none() || self.user_stopped_reconnect
    }

    fn resolve_reconnect_access_token(&self, cx: &App) -> Option<String> {
        if let Some(from_auth) = sai_auth_access_token(cx) {
            return Some(from_auth);
        }
        self.last_connect_access_token
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|cached| !cached.is_empty())
            .map(str::to_owned)
    }

    fn schedule_auto_reconnect(&mut self, reason: String, cx: &mut Context<Self>) {
        SaiWsDeferredOutboundCoordinator::shared(cx).update(cx, |coordinator, cx| {
            coordinator.settle_single_handshake_entries(
                |options, on_success, cx| {
                    if Self::try_serve_cached_response_when_wire_unavailable(
                        options,
                        on_success,
                        "handshake-failed",
                        cx,
                    ) {
                        return true;
                    }
                    Self::record_pagination_failure(
                        options,
                        &anyhow::Error::new(SaiWsSendError::NotConnected),
                        cx,
                    );
                    false
                },
                cx,
            )
        });
        if self.application_is_active(cx) && self.transport_ever_connected {
            RiseEngineAppRuntime::shared(cx)
                .update(cx, |runtime, cx| runtime.socket_handshake_failed(cx));
        }
        if !self.transport_is_live(cx) {
            self.set_auto_reconnect_status(SaiWebSocketAutoReconnectStatus::Idle, cx);
            return;
        }
        if self.user_stopped_reconnect {
            return;
        }
        MaintenanceGate::shared(cx).update(cx, |gate, cx| gate.transport_did_drop(cx));
        if self.resolve_reconnect_access_token(cx).is_none() {
            Sai::log(
                LOG_NAME,
                || "auto-reconnect stopped — no access token".to_owned(),
                SaiLogKind::Websocket,
                cx,
            );
            self.set_auto_reconnect_status(SaiWebSocketAutoReconnectStatus::Idle, cx);
            return;
        }
        self.cancel_auto_reconnect();
        let active_with_path =
            self.application_is_active(cx) && self.network_path_satisfied != Some(false);
        let has_pending_outbound = SaiWsDeferredOutboundCoordinator::shared(cx)
            .read(cx)
            .has_pending_outbound();
        let outbox_waiting =
            active_with_path && (has_pending_outbound || self.durable_outbound_waiting());
        let read_waiting = active_with_path && self.foreground_read_waiting;
        let backoff_step = if outbox_waiting {
            0
        } else if read_waiting {
            self.consecutive_transport_failures
                .min(Self::FOREGROUND_READ_BACKOFF_STEP_CAP)
        } else {
            self.consecutive_transport_failures
        };
        let delay_ns = SaiWebSocketConfiguration::reconnect_delay_nanoseconds(backoff_step);
        let attempt_index = self.consecutive_transport_failures + 1;
        self.consecutive_transport_failures =
            (self.consecutive_transport_failures + 1).min(CONSECUTIVE_FAILURES_CEILING);
        let delay = Duration::from_nanos(delay_ns);
        let delay_seconds = delay.as_secs_f64();
        let fire_at = Date::now().adding_time_interval(delay_seconds);
        let logged_reason = reason.clone();
        self.set_auto_reconnect_status(
            SaiWebSocketAutoReconnectStatus::Scheduled {
                attempt_index,
                fire_at,
                last_reason: reason,
            },
            cx,
        );
        Sai::log(
            LOG_NAME,
            move || {
                format!(
                    "auto-reconnect #{attempt_index} scheduled in {delay_seconds:.2}s — {logged_reason}"
                )
            },
            SaiLogKind::Websocket,
            cx,
        );
        let timer = cx.background_executor().timer(delay);
        self.auto_reconnect_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |manager, cx| {
                if manager.user_stopped_reconnect {
                    manager.set_auto_reconnect_status(SaiWebSocketAutoReconnectStatus::Idle, cx);
                    return;
                }
                let Some(token) = manager.resolve_reconnect_access_token(cx) else {
                    Sai::log(
                        LOG_NAME,
                        || "auto-reconnect cancelled — token missing".to_owned(),
                        SaiLogKind::Websocket,
                        cx,
                    );
                    manager.set_auto_reconnect_status(SaiWebSocketAutoReconnectStatus::Idle, cx);
                    return;
                };
                manager.set_auto_reconnect_status(SaiWebSocketAutoReconnectStatus::Idle, cx);
                manager.connect(&token, cx);
            });
        }));
    }

    fn gateway_ping_options() -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: gateway::SERVICE.to_owned(),
            method: gateway::PING.to_owned(),
            reflect_pending_in_ui: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn start_application_heartbeat(&mut self, cx: &mut Context<Self>) {
        self.stop_application_heartbeat();
        if !self.transport_is_live(cx) {
            return;
        }
        let interval = Duration::from_nanos(SaiWebSocketConfiguration::HEARTBEAT_INTERVAL_NANOSECONDS);
        let silence_limit = Duration::from_nanos(
            SaiWebSocketConfiguration::HEARTBEAT_INTERVAL_NANOSECONDS
                + SaiWebSocketConfiguration::HANDSHAKE_WATCHDOG_NANOSECONDS,
        );
        self.heartbeat_task = Some(cx.spawn(async move |this, cx| {
            let connected = this.read_with(&*cx, |manager, _| manager.is_connected());
            if matches!(connected, Ok(true))
                && let Some(manager) = this.upgrade()
            {
                let _ = Self::send_service_call_raw_body(
                    manager,
                    Map::new(),
                    Self::gateway_ping_options(),
                    None,
                    None,
                    cx.clone(),
                )
                .await;
                let _ = this.update(cx, |manager, _| {
                    if manager.connect_notification_epoch != Some(manager.transport_epoch) {
                        manager.connect_notification_epoch = Some(manager.transport_epoch);
                        RiseAppEvents::default_center().post(SAI_WS_DID_CONNECT);
                    }
                });
            }
            loop {
                cx.background_executor().timer(interval).await;
                let Ok(beat) = this.update(cx, |manager, cx| {
                    if !manager.transport_is_live(cx) || !manager.is_connected() {
                        return false;
                    }
                    if RiseClock::monotonic().saturating_sub(manager.last_inbound_at) > silence_limit {
                        manager.probe_socket("silent socket probe", cx);
                    }
                    true
                }) else {
                    return;
                };
                if !beat {
                    return;
                }
                let Some(manager) = this.upgrade() else {
                    return;
                };
                if let Err(error) = Self::send_service_call_raw_body(
                    manager,
                    Map::new(),
                    Self::gateway_ping_options(),
                    None,
                    None,
                    cx.clone(),
                )
                .await
                {
                    cx.update(|cx| Sai::log(
                        LOG_NAME,
                        move || format!("heartbeat gateway.ping: {error}"),
                        SaiLogKind::Websocket,
                        cx,
                    ));
                }
            }
        }));
    }

    fn stop_application_heartbeat(&mut self) {
        self.heartbeat_task = None;
    }

    pub async fn send_service_call_body<Body: Serialize>(
        this: Entity<Self>,
        body: &Body,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: AsyncApp,
    ) -> anyhow::Result<u64> {
        let data_value = match SaiWsEnvelopeBuilder::json_object_from_encodable(body) {
            Ok(value) => value,
            Err(error) => {
                cx.update(|cx| Self::record_pagination_failure(&options, &error, cx));
                return Err(error);
            }
        };
        Self::send_service_call_payload(this, data_value, options, on_success, on_error, cx).await
    }

    pub async fn send_service_call_raw_body(
        this: Entity<Self>,
        raw_body: Map<String, Value>,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: AsyncApp,
    ) -> anyhow::Result<u64> {
        Self::send_service_call_payload(
            this,
            Value::Object(raw_body),
            options,
            on_success,
            on_error,
            cx,
        )
        .await
    }

    async fn send_service_call_payload(
        this: Entity<Self>,
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        mut cx: AsyncApp,
    ) -> anyhow::Result<u64> {
        if !Self::is_current_account(&options) {
            Self::log_async(
                &cx,
                format!("skipped stale account request {}", Self::fetch_id(&options)),
                SaiLogKind::Cache,
            );
            return Ok(0);
        }

        let refused = sai_guide_hooks().is_some_and(|guide| {
            (guide.refuses_json_object)(&data_value)
                || (guide.refuses_service_method)(&options.service, &options.method)
        });
        if refused {
            cx.update(|cx| {
                Sai::log(
                    "Guides",
                    || "sandbox refused".to_owned(),
                    SaiLogKind::Custom("guide".to_owned()),
                    cx,
                );
                if let Some(on_error) = &on_error {
                    on_error(&anyhow::Error::new(SaiWsSendError::GuideSandbox), cx);
                }
            });
            return Ok(0);
        }

        if let Some(conversation) = sai_conversation_hooks() {
            let handled = cx.update(|cx| {
                (conversation.handle)(data_value.clone(), options.clone(), on_success.clone(), cx)
            });
            if let Some(request_id) = handled.await? {
                return Ok(request_id);
            }
        }

        if MaintenanceGate::is_closed() {
            cx.update(|cx| {
                if Self::try_serve_cached_response_when_wire_unavailable(
                    &options,
                    on_success.as_ref(),
                    "maintenance",
                    cx,
                ) {
                    return;
                }
                let error = anyhow::Error::new(SaiWsSendError::Maintenance);
                Self::record_pagination_failure(&options, &error, cx);
                if let Some(on_error) = &on_error {
                    on_error(&error, cx);
                }
            });
            return Ok(0);
        }

        let validation = cx.update(|cx| {
            SaiWsResponseCache::prepare_for_request(&options, cx);
            let validation = Self::validate_pagination_configuration(&options);
            if let Err(error) = &validation {
                Self::record_pagination_failure(&options, error, cx);
            }
            validation
        });
        validation?;

        let (connected, connecting, has_task) = this.read_with(&cx, |manager, _| {
            (
                manager.is_connected(),
                manager.is_connecting(),
                manager.web_socket_task.is_some(),
            )
        });

        if Self::should_defer_tracked_outbound(options.track_until_acknowledged, connected) {
            if !connecting || !has_task {
                this.update(&mut cx, |manager, cx| {
                    let token = sai_auth_access_token(cx);
                    manager.connect_if_needed(token.as_deref(), cx);
                });
            }
            return Self::enqueue_await_connected_send(
                &this, data_value, options, on_success, on_error, false, &mut cx,
            )
            .await;
        }

        if !connected {
            if connecting && has_task {
                return Self::enqueue_await_connected_send(
                    &this, data_value, options, on_success, on_error, true, &mut cx,
                )
                .await;
            }
            this.update(&mut cx, |manager, cx| manager.wake_transport_on_demand(cx));
            let served = cx.update(|cx| {
                Self::try_serve_cached_response_when_wire_unavailable(
                    &options,
                    on_success.as_ref(),
                    "not-connected",
                    cx,
                )
            });
            if served {
                return Ok(0);
            }
            let connecting_now = this.read_with(&cx, |manager, _| {
                manager.is_connecting() && manager.web_socket_task.is_some()
            });
            if connecting_now && options.allows_deferred_send {
                return Self::enqueue_await_connected_send(
                    &this, data_value, options, on_success, on_error, true, &mut cx,
                )
                .await;
            }
            let error = anyhow::Error::new(SaiWsSendError::NotConnected);
            cx.update(|cx| {
                Self::record_pagination_failure(&options, &error, cx);
                Self::append_ws_trace_not_connected(&options, "not-connected", cx);
            });
            return Err(error);
        }
        if !has_task {
            let served = cx.update(|cx| {
                Self::try_serve_cached_response_when_wire_unavailable(
                    &options,
                    on_success.as_ref(),
                    "no-task",
                    cx,
                )
            });
            if served {
                return Ok(0);
            }
            let error = anyhow::Error::new(SaiWsSendError::NotConnected);
            cx.update(|cx| {
                Self::record_pagination_failure(&options, &error, cx);
                Self::append_ws_trace_not_connected(&options, "no-task", cx);
            });
            return Err(error);
        }
        Self::send_service_call_payload_when_connected(
            this, data_value, options, on_success, on_error, None, false, cx,
        )
        .await
    }

    async fn enqueue_await_connected_send(
        this: &Entity<Self>,
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        single_handshake: bool,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<u64> {
        let manager = this.clone();
        let queued = cx.update(|cx| {
            SaiWsDeferredOutboundCoordinator::shared(cx).update(cx, |coordinator, cx| {
                coordinator.enqueue_await_connected_send(
                    manager,
                    data_value,
                    options,
                    on_success,
                    on_error,
                    single_handshake,
                    cx,
                )
            })
        });
        queued.await
    }

    /// connected half of a send, boxed because a screen refresh re-enters it from a task it spawned itself
    #[allow(clippy::too_many_arguments)]
    pub fn send_service_call_payload_when_connected(
        this: Entity<Self>,
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        prepared_optimistic_send: Option<SaiWsPreparedOptimisticSend>,
        preserve_prepared_optimistic_on_failure: bool,
        cx: AsyncApp,
    ) -> LocalBoxFuture<'static, anyhow::Result<u64>> {
        Self::send_when_connected(
            this,
            data_value,
            options,
            on_success,
            on_error,
            prepared_optimistic_send,
            preserve_prepared_optimistic_on_failure,
            cx,
        )
        .boxed_local()
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_when_connected(
        this: Entity<Self>,
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        prepared_optimistic_send: Option<SaiWsPreparedOptimisticSend>,
        preserve_prepared_optimistic_on_failure: bool,
        mut cx: AsyncApp,
    ) -> anyhow::Result<u64> {
        if !Self::is_current_account(&options) {
            Self::log_async(
                &cx,
                format!("skipped stale connected request {}", Self::fetch_id(&options)),
                SaiLogKind::Cache,
            );
            return Ok(0);
        }

        cx.update(|cx| SaiWsResponseCache::prepare_for_request(&options, cx));

        let live = this.read_with(&cx, |manager, _| {
            manager.web_socket_task.is_some() && manager.is_connected()
        });
        if !live {
            if options.track_until_acknowledged
                && prepared_optimistic_send.is_none()
                && !preserve_prepared_optimistic_on_failure
            {
                this.update(&mut cx, |manager, cx| manager.wake_transport_on_demand(cx));
                return Self::enqueue_await_connected_send(
                    &this, data_value, options, on_success, on_error, false, &mut cx,
                )
                .await;
            }
            return Err(anyhow::Error::new(SaiWsSendError::NotConnected));
        }

        let has_handlers = on_success.is_some() || on_error.is_some();
        let has_slice = options.sai_slice.is_some();
        let tracks_wire_completion = has_handlers || has_slice;
        let cache_binding = SaiWsPendingCacheBinding::new(&options);
        let cache_key = options.cache_key.clone().filter(|key| !key.is_empty());
        let is_initial_page = options
            .pagination
            .as_ref()
            .is_none_or(|pagination| pagination.is_initial_page());

        cx.update(|cx| {
            SaiWsResponseCache::note_active_slice(
                options.sai_slice.as_ref(),
                options.cache_key.as_deref(),
                options.take_path.as_deref(),
                cx,
            );
            Self::register_screen_refresh_resender_if_needed(
                &this,
                &data_value,
                &options,
                on_success.as_ref(),
                on_error.as_ref(),
                cx,
            );
        });

        if let Some(ck) = cache_key.as_deref()
            && !options.force_bypass_cache_read
            && is_initial_page
        {
            let served = cx.update(|cx| {
                Self::serve_connected_cache_hit(
                    &this,
                    ck,
                    &data_value,
                    &options,
                    on_success.as_ref(),
                    on_error.as_ref(),
                    cx,
                )
            });
            if served {
                return Ok(0);
            }
        }

        if let Some(ck) = cache_key.as_deref()
            && !options.allow_concurrent_while_pending
            && (tracks_wire_completion || options.pagination.is_some())
        {
            let attached = cx.update(|cx| {
                let registry = SaiWsPendingRegistry::shared(cx);
                let existing_id = registry.read(cx).in_flight_request_id(ck)?;
                if options.pagination.is_some() {
                    tracing::warn!(target: "riseonly::X2174", existing_id, cache_key = %ck, "list request skipped while another is in flight, handlers dropped");
                    let log_key = ck.to_owned();
                    Sai::log(
                        LOG_NAME,
                        move || {
                            format!(
                                "pagination skipped while list request is in flight id={existing_id} cacheKey={log_key}"
                            )
                        },
                        SaiLogKind::Websocket,
                        cx,
                    );
                    return Some(existing_id);
                }
                let (on_success, on_error) =
                    Self::pending_handlers(on_success.as_ref(), on_error.as_ref());
                let attached = registry.update(cx, |registry, _cx| {
                    registry.attach_handlers(existing_id, on_success, on_error)
                });
                if !attached {
                    return None;
                }
                let log_key = ck.to_owned();
                Sai::log(
                    LOG_NAME,
                    move || {
                        format!("deduped → attach handlers to in-flight id={existing_id} cacheKey={log_key}")
                    },
                    SaiLogKind::Websocket,
                    cx,
                );
                if SAI_DEBUG_TOOLS {
                    Self::append_ws_trace_deduped(&this, &options, ck, &data_value, existing_id, cx);
                }
                Some(existing_id)
            });
            if let Some(existing_id) = attached {
                return Ok(existing_id);
            }
        }

        if let Some(coalescing) = &options.coalescing {
            let decision = cx.update(|cx| SaiWsCoalescingRegistry::consume(coalescing, cx));
            match decision {
                SaiWsCoalescingConsumeResult::SkippedEvenParity { tap_count } => {
                    cx.update(|cx| {
                        let fetch_id = Self::fetch_id(&options);
                        let key = coalescing.key.clone();
                        Sai::log(
                            LOG_NAME,
                            move || {
                                format!("coalescing skip {fetch_id} key={key} tapCount={tap_count} (even)")
                            },
                            SaiLogKind::Websocket,
                            cx,
                        );
                        #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
                        if !options.exclude_from_debugger {
                            SaiWsIgnoredRequestsDebugRecorder::record_coalescing_skip(
                                &options.service,
                                &options.method,
                                &Self::trace_ws_request_body(&data_value),
                                &coalescing.key,
                                &format!("toggleParity: tapCount={tap_count} (even) → no wire"),
                                cx,
                            );
                        };
                    });
                    return Ok(0);
                }
                SaiWsCoalescingConsumeResult::Send { tap_count } => {
                    if tap_count > 0 {
                        cx.update(|cx| {
                            let fetch_id = Self::fetch_id(&options);
                            let key = coalescing.key.clone();
                            Sai::log(
                                LOG_NAME,
                                move || {
                                    format!("coalescing emit {fetch_id} key={key} tapCount={tap_count} (odd)")
                                },
                                SaiLogKind::Websocket,
                                cx,
                            )
                        });
                    }
                }
            }
        }

        if let Some(scope) = options.deep_compare_scope_key()
            && let Some(last) = SaiWsLastOutboundBodyRegistry::last_body(&scope)
            && SaiWsJSONObjectDeepCompare::bodies_equal(&last, &data_value)
        {
            cx.update(|cx| {
                let fetch_id = Self::fetch_id(&options);
                let log_scope = scope.clone();
                Sai::log(
                    LOG_NAME,
                    move || format!("deepCompare skip {fetch_id} scope={log_scope}"),
                    SaiLogKind::Websocket,
                    cx,
                );
                #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
                if !options.exclude_from_debugger {
                    SaiWsIgnoredRequestsDebugRecorder::record_deep_compare_skip(
                        &options.service,
                        &options.method,
                        &Self::trace_ws_request_body(&data_value),
                        &scope,
                        "outbound data deep-equals last sent body for this scope (body only)",
                        cx,
                    );
                };
            });
            return Ok(0);
        }

        let send_options = options.clone();
        let send_this = this.clone();
        let send_work: SaiWireSerialBody<u64> = Box::new(move |cx| {
            Self::send_one_service_call_wire(
                send_this,
                data_value,
                send_options,
                cache_binding,
                tracks_wire_completion,
                on_success,
                on_error,
                prepared_optimistic_send,
                preserve_prepared_optimistic_on_failure,
                cx,
            )
            .boxed_local()
        });

        if options.send_immediately {
            return send_work(cx).await;
        }
        Self::with_serialized_wire_send(&this, send_work, &mut cx).await
    }

    fn serve_connected_cache_hit(
        this: &Entity<Self>,
        ck: &str,
        data_value: &Value,
        options: &SaiWsRequestOptions,
        on_success: Option<&SaiWsOnSuccess>,
        on_error: Option<&SaiWsOnError>,
        cx: &mut App,
    ) -> bool {
        let cold_slice = options.sai_slice.clone();
        let cold_take_path = options.take_path.clone();
        let on_cold_disk_loaded: SaiWsColdDiskLoaded = Box::new(move |value: &Value, _cx: &mut App| {
            let Some(cold_slice) = cold_slice else {
                return;
            };
            if cold_slice.status() == SaiWsSliceStatus::Fulfilled {
                return;
            }
            cold_slice.mark_fulfilled((!value.is_null()).then_some(value), cold_take_path.as_deref());
        });
        let (cached, had_memory, had_disk, selected_source) =
            SaiWsResponseCache::peek_with_source(options, false, Some(on_cold_disk_loaded), cx);
        if options.screen_refresh.as_ref().is_some_and(|refresh| refresh.enabled) {
            let fetch_id = Self::fetch_id(options);
            let key = ck.to_owned();
            let is_cached = cached.is_some();
            let reload_if_cached = options.reload_if_cached;
            let bypass = options.force_bypass_cache_read;
            SaiWsScreenRefreshDebug::log(
                move || {
                    format!(
                        "peek {fetch_id} key={key} cached={is_cached} mem={had_memory} disk={had_disk} reloadIfCached={reload_if_cached} bypass={bypass}"
                    )
                },
                cx,
            );
        }
        let Some(cached) = cached else {
            return false;
        };
        let bypass_optimistic_cache =
            SaiWsOptimisticCoordinator::should_bypass_cache_only_short_circuit(options, cx);
        let from_legacy_disk = selected_source == Some(SaiWsCacheSelectionSource::LegacyDisk);

        if let Some(refresh) = &options.screen_refresh
            && SaiWsScreenRefreshCoordinator::should_background_revalidate(ck, refresh, cx)
        {
            let fetch_id = Self::fetch_id(options);
            let branch_fetch_id = fetch_id.clone();
            let branch_key = ck.to_owned();
            SaiWsScreenRefreshDebug::log(
                move || format!("→ screen refresh branch {branch_fetch_id} key={branch_key}"),
                cx,
            );
            SaiWsScreenRefreshCoordinator::install_lifecycle_observers_if_needed(cx);
            SaiWsResponseCache::hydrate_memory(ck, &cached, from_legacy_disk, cx);
            let served = Self::ws_served_cache_layer(selected_source);
            let log_key = ck.to_owned();
            Sai::log(
                LOG_NAME,
                move || format!("cache hit {fetch_id} key={log_key} from={served} → screen refresh (wire)"),
                SaiLogKind::Cache,
                cx,
            );
            Self::fulfil_from_cache(options, on_success, &cached, cx);
            if SAI_DEBUG_TOOLS {
                Self::append_ws_trace_screen_refresh_cache_served(
                    this, options, ck, data_value, had_memory, had_disk, &cached, served, cx,
                );
            }
            if SaiWsScreenRefreshCacheHitPolicy::should_send_granted_revalidation_wire(
                options.reload_if_cached,
                SaiWsScreenRefreshCoordinator::needs_epoch_revalidation(ck, cx),
            ) {
                Self::spawn_screen_refresh_wire(
                    this.clone(),
                    data_value.clone(),
                    options.clone(),
                    on_success.cloned(),
                    on_error.cloned(),
                    cx,
                );
            }
            return true;
        }

        if !options.reload_if_cached && !bypass_optimistic_cache {
            SaiWsResponseCache::hydrate_memory(ck, &cached, from_legacy_disk, cx);
            let served = Self::ws_served_cache_layer(selected_source);
            let fetch_id = Self::fetch_id(options);
            let log_key = ck.to_owned();
            Sai::log(
                LOG_NAME,
                move || format!("cache hit {fetch_id} key={log_key} from={served} (no wire)"),
                SaiLogKind::Cache,
                cx,
            );
            Self::fulfil_from_cache(options, on_success, &cached, cx);
            if SAI_DEBUG_TOOLS {
                Self::append_ws_trace_short_circuit(
                    this,
                    options,
                    ck,
                    data_value,
                    had_memory,
                    had_disk,
                    &cached,
                    served,
                    "cache-hit-no-reload",
                    cx,
                );
            }
            Self::send_epoch_revalidation_wire_if_needed(
                this, data_value, options, ck, on_success, on_error, cx,
            );
            return true;
        }
        if options.reload_if_cached
            && !options.reload_when_only_disk_cached
            && !had_memory
            && had_disk
            && !bypass_optimistic_cache
        {
            SaiWsResponseCache::hydrate_memory(ck, &cached, from_legacy_disk, cx);
            let fetch_id = Self::fetch_id(options);
            let log_key = ck.to_owned();
            Sai::log(
                LOG_NAME,
                move || format!("disk-only cache hit {fetch_id} key={log_key} (no wire)"),
                SaiLogKind::Cache,
                cx,
            );
            Self::fulfil_from_cache(options, on_success, &cached, cx);
            if SAI_DEBUG_TOOLS {
                Self::append_ws_trace_short_circuit(
                    this,
                    options,
                    ck,
                    data_value,
                    had_memory,
                    had_disk,
                    &cached,
                    "disk",
                    "disk-only-cache-hit-skip-network",
                    cx,
                );
            }
            Self::send_epoch_revalidation_wire_if_needed(
                this, data_value, options, ck, on_success, on_error, cx,
            );
            return true;
        }
        false
    }

    /// the slice and success callback of a request answered from cache
    fn fulfil_from_cache(
        options: &SaiWsRequestOptions,
        on_success: Option<&SaiWsOnSuccess>,
        cached: &Value,
        cx: &mut App,
    ) {
        let value = (!cached.is_null()).then_some(cached);
        let take_path = options.take_path.as_deref();
        if let Some(slice) = &options.sai_slice {
            slice.mark_fulfilled(value, take_path);
        }
        if options.invoke_success_for_cached_response
            && let Some(on_success) = on_success
        {
            on_success(Self::callback_payload(options, value), cx);
        }
    }

    /// the registry owns boxed handlers and the manager shares one rc across retries and paths
    fn pending_handlers(
        on_success: Option<&SaiWsOnSuccess>,
        on_error: Option<&SaiWsOnError>,
    ) -> (Option<SaiWsPendingOnSuccess>, Option<SaiWsPendingOnError>) {
        let on_success = on_success.cloned().map(|handler| -> SaiWsPendingOnSuccess {
            Box::new(move |value: Option<Box<dyn std::any::Any>>, cx: &mut App| handler(value, cx))
        });
        let on_error = on_error.cloned().map(|handler| -> SaiWsPendingOnError {
            Box::new(move |error: &anyhow::Error, cx: &mut App| handler(error, cx))
        });
        (on_success, on_error)
    }

    fn callback_payload(
        options: &SaiWsRequestOptions,
        value: Option<&Value>,
    ) -> Option<Box<dyn std::any::Any>> {
        let take_path = options.take_path.as_deref();
        options
            .sai_slice
            .as_ref()
            .and_then(|slice| slice.callback_payload(value, take_path))
            .or_else(|| {
                SaiPayloadPath::extract(value, take_path)
                    .map(|extracted| Box::new(extracted.clone()) as Box<dyn std::any::Any>)
            })
    }

    async fn with_serialized_wire_send<T: 'static>(
        this: &Entity<Self>,
        body: SaiWireSerialBody<T>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<T> {
        let (result_sender, result) = oneshot::channel::<anyhow::Result<T>>();
        let abandoned = Arc::new(AtomicBool::new(false));
        let caller_left = SaiMarkOnDrop(Arc::clone(&abandoned));
        this.update(cx, |manager, cx| {
            let previous = manager.wire_serial_tail.take();
            let next = cx.spawn(async move |_, cx| {
                if let Some(previous) = previous {
                    previous.await;
                }
                if abandoned.load(Ordering::Acquire) {
                    return;
                }
                let outcome = body(cx.clone()).await;
                let _ = result_sender.send(outcome);
            });
            manager.wire_serial_tail = Some(next.shared());
        });
        let outcome = result.await;
        drop(caller_left);
        outcome.unwrap_or_else(|_| {
            Err(anyhow::Error::new(SaiWsSendError::SendFailed(
                "wire serialization lost result".to_owned(),
            )))
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_one_service_call_wire(
        this: Entity<Self>,
        data_value: Value,
        options: SaiWsRequestOptions,
        cache_binding: SaiWsPendingCacheBinding,
        tracks_wire_completion: bool,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        prepared_optimistic_send: Option<SaiWsPreparedOptimisticSend>,
        preserve_prepared_optimistic_on_failure: bool,
        mut cx: AsyncApp,
    ) -> anyhow::Result<u64> {
        if !Self::is_current_account(&options) {
            return Ok(0);
        }
        let live = this.read_with(&cx, |manager, _| {
            manager.web_socket_task.is_some() && manager.is_connected()
        });
        if !live {
            return Err(anyhow::Error::new(SaiWsSendError::NotConnected));
        }

        let id_millis = prepared_optimistic_send
            .as_ref()
            .map_or_else(SaiWsEnvelopeBuilder::make_outbound_id, |prepared| {
                prepared.wire_request_id
            });
        let timestamp_seconds = SaiWsEnvelopeBuilder::make_timestamp_seconds(id_millis);

        let optimistic_outcome = match &prepared_optimistic_send {
            Some(prepared) => prepared.outcome.clone(),
            None => {
                let applied = cx.update(|cx| {
                    SaiWsOptimisticCoordinator::apply_before_send_if_needed(
                        id_millis,
                        &data_value,
                        &options,
                        cx,
                    )
                });
                match applied {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        Self::log_async(
                            &cx,
                            format!("optimistic apply failed: {error}"),
                            SaiLogKind::Websocket,
                        );
                        return Err(error.into());
                    }
                }
            }
        };

        let protects_pre_wire_optimistic = optimistic_outcome.did_register_session;
        if protects_pre_wire_optimistic {
            cx.update(|cx| SaiWsOptimisticCoordinator::begin_pre_wire_optimistic(id_millis, cx));
        }
        let _finish_pre_wire = SaiFinishPreWireOnDrop {
            wire_request_id: protects_pre_wire_optimistic.then_some(id_millis),
            cx: cx.clone(),
        };
        let insert_temp_id = optimistic_outcome.insert_temp_id.clone();
        let clear_temp_id = || {
            if let Some(temp_id) = &insert_temp_id {
                SaiWsOutboundCancellation::clear(temp_id);
            }
        };
        let fail_optimistic = |error: &anyhow::Error, cx: &mut AsyncApp| {
            cx.update(|cx| {
                SaiWsOptimisticCoordinator::handle_failure(
                    id_millis,
                    &**error,
                    options.sai_slice.clone(),
                    Some(&cache_binding),
                    cx,
                );
            });
            if let Some(slice) = &options.sai_slice {
                slice.mark_rejected(error);
            }
        };

        let merge = cx.update(|cx| {
            SaiWsOptimisticFileUploadRunner::merge_uploaded_files_if_needed(
                data_value.clone(),
                insert_temp_id.clone(),
                options.clone(),
                cx,
            )
        });
        let wire_payload = match merge.await {
            Ok(payload) => payload,
            Err(error) => {
                if !Self::is_current_account(&options) {
                    clear_temp_id();
                    Self::log_async(
                        &cx,
                        "cancelled stale account upload before wire send",
                        SaiLogKind::Cache,
                    );
                    return Ok(0);
                }
                if optimistic_outcome.did_register_session && !preserve_prepared_optimistic_on_failure
                {
                    fail_optimistic(&error, &mut cx);
                }
                clear_temp_id();
                Self::log_async(
                    &cx,
                    format!("optimistic file upload failed: {error}"),
                    SaiLogKind::Websocket,
                );
                return Err(error);
            }
        };

        if !Self::is_current_account(&options) {
            clear_temp_id();
            Self::log_async(&cx, "cancelled stale account request after upload", SaiLogKind::Cache);
            return Ok(0);
        }

        let Some(live_task) = Self::live_web_socket_task_after_pre_wire_work(&this, &mut cx).await
        else {
            if options.track_until_acknowledged
                && optimistic_outcome.did_register_session
                && !preserve_prepared_optimistic_on_failure
            {
                let mut replay_options = options.clone();
                if let Some(update) = replay_options.optimistic_update.as_mut() {
                    update.files = None;
                }
                this.update(&mut cx, |manager, cx| manager.wake_transport_on_demand(cx));
                let manager = this.clone();
                let prepared = SaiWsPreparedOptimisticSend {
                    wire_request_id: id_millis,
                    outcome: optimistic_outcome.clone(),
                };
                let queued = cx.update(|cx| {
                    SaiWsDeferredOutboundCoordinator::shared(cx).update(cx, |coordinator, cx| {
                        coordinator.enqueue_prepared_await_connected_send(
                            manager,
                            wire_payload,
                            replay_options,
                            on_success,
                            on_error,
                            prepared,
                            cx,
                        )
                    })
                });
                return queued.await;
            }
            let error = anyhow::Error::new(SaiWsSendError::NotConnected);
            if optimistic_outcome.did_register_session && !preserve_prepared_optimistic_on_failure {
                fail_optimistic(&error, &mut cx);
            }
            clear_temp_id();
            return Err(error);
        };

        if let Some(temp_id) = &insert_temp_id
            && SaiWsOutboundCancellation::is_request_cancelled(temp_id)
        {
            let error = anyhow::Error::new(SaiWsOutboundCancelledError);
            if optimistic_outcome.did_register_session {
                fail_optimistic(&error, &mut cx);
            }
            clear_temp_id();
            return Err(error);
        }
        clear_temp_id();

        let json = SaiWsEnvelopeBuilder::json_string(
            &options.service,
            &options.method,
            &wire_payload,
            id_millis,
            timestamp_seconds,
        )?;
        cx.update(|cx| {
            let fetch_id = Self::fetch_id(&options);
            Sai::log(
                LOG_NAME,
                move || format!("→ service_call {fetch_id} id={id_millis}"),
                SaiLogKind::Websocket,
                cx,
            );
            if SAI_DEBUG_TOOLS && !RiseCanonEndpoints::is_known_on_wire(&options.service, &options.method) {
                let fetch_id = Self::fetch_id(&options);
                Sai::log(
                    LOG_NAME,
                    move || format!("method {fetch_id} is not in the canon catalog"),
                    SaiLogKind::Websocket,
                    cx,
                );
            }
        });

        let wire_key = SaiWsPendingRegistry::wire_id_key(id_millis);

        let did_apply_optimistic = optimistic_outcome.did_register_session;
        let debt_tracking =
            options.track_until_acknowledged && SaiWsResponseDebtCoordinator::is_feature_enabled();
        let needs_registry = tracks_wire_completion
            || did_apply_optimistic
            || debt_tracking
            || (options.cache_key.as_deref().is_some_and(|key| !key.is_empty())
                && (cache_binding.store_response_in_cache
                    || cache_binding.persist_cache_to_disk()
                    || !options.allow_concurrent_while_pending));
        let is_initial_page = options
            .pagination
            .as_ref()
            .is_none_or(|pagination| pagination.is_initial_page());

        if SAI_DEBUG_TOOLS && !options.exclude_from_debugger {
            cx.update(|cx| {
                let tid = Uuid::new_v4();
                let req_pretty = SaiHttpTraceRecorder::pretty_json_string_from_raw_utf8(&json);
                let tags = if options.is_screen_refresh_wire {
                    SaiTraceTag::screen_refresh_wire()
                } else {
                    Vec::new()
                };
                if options.is_screen_refresh_wire {
                    let fetch_id = Self::fetch_id(&options);
                    let log_tags = tags.clone();
                    SaiWsScreenRefreshDebug::log(
                        move || format!("debugger upsert PENDING {fetch_id} traceId={tid} tags={log_tags:?}"),
                        cx,
                    );
                }
                let url = this.read(cx).active_url_display_string();
                let entry = SaiHttpTraceEntry {
                    id: tid,
                    transport: SaiTraceTransport::Websocket,
                    created_at: Date::now(),
                    is_pending: true,
                    method: "WS".to_owned(),
                    url,
                    fetch_id: Some(Self::fetch_id(&options)),
                    duration_ms: 0,
                    request_headers: HashMap::from([(
                        "x-sai-wire-attempts".to_owned(),
                        "1".to_owned(),
                    )]),
                    request_body_wire: req_pretty,
                    response_status: 0,
                    response_headers: HashMap::new(),
                    response_body_preview: String::new(),
                    transport_error: None,
                    trace_tags: tags,
                };
                let wire_key = wire_key.clone();
                SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
                    store.upsert(entry, cx);
                    store.register_ws_awaiting_response(wire_key, tid);
                });
            });
        }

        cx.update(|cx| {
            if needs_registry {
                let (on_success, on_error) =
                    Self::pending_handlers(on_success.as_ref(), on_error.as_ref());
                SaiWsPendingRegistry::shared(cx).update(cx, |registry, cx| {
                    registry.register(
                        id_millis,
                        Some(cache_binding.clone()),
                        options.take_path.clone(),
                        options.sai_slice.clone(),
                        options.account_user_id.clone(),
                        options.account_epoch.clone(),
                        Some((options.service.clone(), options.method.clone())),
                        on_success,
                        on_error,
                        cx,
                    )
                });
            }
            let optimistic_root = options
                .cache_key
                .as_deref()
                .filter(|key| did_apply_optimistic && !key.is_empty() && options.sai_slice.is_some())
                .and_then(|key| {
                    SaiWsResponseCache::current_payload(
                        key,
                        options.preferred_cache_layer.clone(),
                        options.disk_cache_policy.clone(),
                        cx,
                    )
                });
            if let (Some(root), Some(slice)) = (optimistic_root, &options.sai_slice) {
                slice.mark_fulfilled((!root.is_null()).then_some(&root), options.take_path.as_deref());
            } else if options.reflect_pending_in_ui
                && is_initial_page
                && let Some(slice) = &options.sai_slice
            {
                slice.mark_pending();
            }
        });

        let sent = Self::send_wire_text(json, &live_task).await;
        let remove_pending = |cx: &mut AsyncApp| {
            cx.update(|cx| {
                SaiWsPendingRegistry::shared(cx)
                    .update(cx, |registry, cx| registry.remove_pending(id_millis, cx))
            });
        };
        match sent {
            Ok(()) => {
                if !Self::is_current_account(&options) {
                    remove_pending(&mut cx);
                    Self::log_async(&cx, "discarded stale account send completion", SaiLogKind::Cache);
                    return Ok(0);
                }
                if let Some(scope) = options.deep_compare_scope_key() {
                    SaiWsLastOutboundBodyRegistry::remember_body(&wire_payload, &scope);
                }
            }
            Err(error) => {
                if !Self::is_current_account(&options) {
                    remove_pending(&mut cx);
                    Self::log_async(&cx, "discarded stale account send failure", SaiLogKind::Cache);
                    return Ok(0);
                }
                if needs_registry && !preserve_prepared_optimistic_on_failure {
                    cx.update(|cx| {
                        SaiWsOptimisticCoordinator::handle_failure(
                            id_millis,
                            &*error,
                            options.sai_slice.clone(),
                            Some(&cache_binding),
                            cx,
                        );
                        SaiWsPaginationStateStore::fail(Some(&cache_binding), &error, cx);
                    });
                    if is_initial_page && let Some(slice) = &options.sai_slice {
                        slice.mark_rejected(&error);
                    }
                }
                if needs_registry {
                    remove_pending(&mut cx);
                }
                if SAI_DEBUG_TOOLS && !options.exclude_from_debugger {
                    let message = error.to_string();
                    cx.update(|cx| {
                        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
                            store.fail_ws_trace_if_linked(&wire_key, &message, cx)
                        })
                    });
                }
                return Err(error);
            }
        }
        cx.update(|cx| {
            SaiWsResponseDebtCoordinator::shared(cx).update(cx, |debt, cx| {
                debt.track_after_successful_wire_send_if_needed(id_millis, &wire_payload, &options, cx)
            })
        });
        Ok(id_millis)
    }

    async fn live_web_socket_task_after_pre_wire_work(
        this: &Entity<Self>,
        cx: &mut AsyncApp,
    ) -> Option<Arc<SaiWebSocketTask>> {
        let live = |manager: &Self| {
            manager
                .web_socket_task
                .clone()
                .filter(|_| manager.is_connected())
        };
        if let Some(task) = this.read_with(&*cx, |manager, _| live(manager)) {
            return Some(task);
        }
        let deadline = RiseClock::monotonic() + PRE_WIRE_SOCKET_WAIT;
        while RiseClock::monotonic() < deadline {
            let (task, stopped) = this.read_with(&*cx, |manager, _| {
                (live(manager), manager.user_stopped_reconnect)
            });
            if task.is_some() {
                return task;
            }
            if stopped {
                return None;
            }
            cx.background_executor().timer(PRE_WIRE_SOCKET_POLL).await;
        }
        this.read_with(&*cx, |manager, _| live(manager))
    }

    fn is_current_account(options: &SaiWsRequestOptions) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(
            options.account_user_id.as_deref(),
            options.account_epoch.as_deref(),
        )
    }

    pub async fn resend_persisted_wire_frame(
        this: Entity<Self>,
        wire_request_id_millis: u64,
        data_value: Value,
        options: SaiWsRequestOptions,
        mut cx: AsyncApp,
    ) -> anyhow::Result<()> {
        if !Self::is_current_account(&options) {
            Self::log_async(&cx, "skipped stale account ack-debt replay", SaiLogKind::Cache);
            return Ok(());
        }
        let Some(task) = this.read_with(&cx, |manager, _| {
            manager
                .web_socket_task
                .clone()
                .filter(|_| manager.is_connected())
        }) else {
            return Err(anyhow::Error::new(SaiWsSendError::NotConnected));
        };

        cx.update(|cx| {
            let registry = SaiWsPendingRegistry::shared(cx);
            if !registry.read(cx).has_pending(wire_request_id_millis) {
                registry.update(cx, |registry, cx| {
                    registry.register(
                        wire_request_id_millis,
                        Some(SaiWsPendingCacheBinding::new(&options)),
                        options.take_path.clone(),
                        options.sai_slice.clone(),
                        options.account_user_id.clone(),
                        options.account_epoch.clone(),
                        Some((options.service.clone(), options.method.clone())),
                        None,
                        None,
                        cx,
                    )
                });
            }
        });

        let timestamp_seconds = SaiWsEnvelopeBuilder::make_timestamp_seconds(wire_request_id_millis);
        let json = SaiWsEnvelopeBuilder::json_string(
            &options.service,
            &options.method,
            &data_value,
            wire_request_id_millis,
            timestamp_seconds,
        )?;
        Self::log_async(
            &cx,
            format!(
                "→ service_call (ack-debt replay) {} id={wire_request_id_millis}",
                Self::fetch_id(&options)
            ),
            SaiLogKind::Websocket,
        );
        Self::send_wire_text(json, &task).await
    }

    fn register_screen_refresh_resender_if_needed(
        this: &Entity<Self>,
        data_value: &Value,
        options: &SaiWsRequestOptions,
        on_success: Option<&SaiWsOnSuccess>,
        on_error: Option<&SaiWsOnError>,
        cx: &mut App,
    ) {
        if options.is_screen_refresh_wire
            || !options
                .pagination
                .as_ref()
                .is_none_or(|pagination| pagination.is_initial_page())
        {
            return;
        }
        let Some(refresh) = options.screen_refresh.clone().filter(|refresh| refresh.enabled) else {
            return;
        };
        let Some(ck) = options.cache_key.clone().filter(|key| !key.is_empty()) else {
            return;
        };

        SaiWsScreenRefreshCoordinator::install_lifecycle_observers_if_needed(cx);
        let manager = this.downgrade();
        let captured_body = data_value.clone();
        let captured_options = options.clone();
        let captured_success = on_success.cloned();
        let captured_error = on_error.cloned();
        SaiWsScreenRefreshCoordinator::register(
            &ck,
            &refresh,
            Box::new(move |cx: &mut App| {
                let Some(manager) = manager.upgrade() else {
                    return;
                };
                if !Self::is_current_account(&captured_options) {
                    return;
                }
                let resend_callbacks = captured_options
                    .make_screen_refresh_resend_callbacks
                    .as_ref()
                    .map(|make| make(cx));
                let (resend_success, resend_error) = match resend_callbacks {
                    Some(callbacks) => (Some(callbacks.on_success), Some(callbacks.on_error)),
                    None => (captured_success.clone(), captured_error.clone()),
                };
                Self::spawn_screen_refresh_wire(
                    manager,
                    captured_body.clone(),
                    captured_options.clone(),
                    resend_success,
                    resend_error,
                    cx,
                );
            }),
            cx,
        );
    }

    fn send_epoch_revalidation_wire_if_needed(
        this: &Entity<Self>,
        data_value: &Value,
        options: &SaiWsRequestOptions,
        cache_key: &str,
        on_success: Option<&SaiWsOnSuccess>,
        on_error: Option<&SaiWsOnError>,
        cx: &mut App,
    ) {
        let enabled = options
            .screen_refresh
            .as_ref()
            .is_some_and(|refresh| refresh.enabled);
        if !enabled || !SaiWsScreenRefreshCoordinator::needs_epoch_revalidation(cache_key, cx) {
            return;
        }
        Self::spawn_screen_refresh_wire(
            this.clone(),
            data_value.clone(),
            options.clone(),
            on_success.cloned(),
            on_error.cloned(),
            cx,
        );
    }

    fn spawn_screen_refresh_wire(
        this: Entity<Self>,
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        cx.spawn(async move |cx| {
            Self::send_screen_refresh_wire(this, data_value, options, on_success, on_error, cx.clone())
                .await;
        })
        .detach();
    }

    async fn send_screen_refresh_wire(
        this: Entity<Self>,
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        cx: AsyncApp,
    ) {
        let mut wire_options = options.clone();
        wire_options.force_bypass_cache_read = true;
        wire_options.reflect_pending_in_ui = false;
        wire_options.screen_refresh = None;
        wire_options.is_screen_refresh_wire = true;
        if let Some(ck) = options.cache_key.as_deref().filter(|key| !key.is_empty()) {
            cx.update(|cx| SaiWsScreenRefreshCoordinator::mark_revalidated(ck, cx));
        }
        let sent = Self::send_service_call_payload_when_connected(
            this,
            data_value,
            wire_options,
            on_success,
            on_error.clone(),
            None,
            false,
            cx.clone(),
        )
        .await;
        if let Err(error) = sent
            && let Some(on_error) = on_error
        {
            cx.update(|cx| on_error(&error, cx));
        }
    }

    async fn dispatch_incoming(
        decoded: SaiWsDecodedJSONObject,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        transport_epoch: u64,
        cx: &mut AsyncApp,
    ) {
        let dict = &decoded.value;
        let consumed = cx.update(|cx| {
            SaiWsPendingRegistry::shared(cx)
                .update(cx, |registry, cx| registry.handle_incoming_json_object(dict, cx))
        });
        if consumed {
            return;
        }
        let accept = |cx: &mut AsyncApp| {
            let raw_data = decoded.raw_data.clone();
            let user_id = account_user_id.clone();
            let epoch = account_epoch.clone();
            cx.update(|cx| {
                RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| {
                    runtime.accept_account_user_id(raw_data, user_id, epoch, transport_epoch, cx)
                })
            })
        };
        let dispatch_legacy = |cx: &mut AsyncApp| {
            cx.update(|cx| {
                SaiWsEventRegistry::shared(cx).update(cx, |registry, cx| {
                    let _ = registry.dispatch_if_needed(dict, cx);
                })
            });
        };
        if dict.contains_key(rise_key_bot::REQUEST_ID) {
            let disposition = accept(cx).await;
            if matches!(disposition, RiseWireInboundDisposition::ConsumedResponse) {
                return;
            }
            dispatch_legacy(cx);
        } else {
            cx.update(|cx| SaiWsEventRegistry::observe_frame(dict, cx));
            let event_type = SaiWsEventRegistry::event_type(dict);
            tracing::warn!(target: "riseonly::X753", "socket frame type={event_type:?}");
            match RiseInboundOwnershipRouter::shared().owner(
                event_type.as_deref(),
                account_user_id.as_deref(),
                account_epoch.as_deref(),
            ) {
                RiseInboundOwner::Legacy => dispatch_legacy(cx),
                RiseInboundOwner::RiseEngine => {
                    let disposition = accept(cx).await;
                    if matches!(disposition, RiseWireInboundDisposition::Ignored) {
                        cx.update(|cx| {
                            RiseEngineAppRuntime::shared(cx)
                                .update(cx, |runtime, cx| runtime.owned_push_was_ignored(cx))
                        });
                    }
                }
            }
        }
    }

    pub async fn rise_wire_synchronize_attached_engine(this: Entity<Self>, cx: AsyncApp) {
        let (user_id, epoch, transport_epoch) =
            this.read_with(&cx, |manager, _| manager.transport_identity());
        let started = cx.update(|cx| {
            RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| {
                runtime.socket_session_started(user_id.clone(), epoch.clone(), transport_epoch, cx)
            })
        });
        let session = started.await;
        let synchronized = cx.update(|cx| {
            let (active, connected) = this.read_with(&*cx, |manager, cx| {
                (manager.application_is_active(cx), manager.is_connected())
            });
            let runtime = RiseEngineAppRuntime::shared(cx);
            runtime.update(cx, |runtime, cx| {
                runtime.set_application_active(
                    active,
                    user_id.clone(),
                    epoch.clone(),
                    transport_epoch,
                    cx,
                );
                if connected && session.is_some() {
                    runtime.socket_connected(user_id, epoch, transport_epoch, true, cx);
                }
                runtime.wait_for_socket_lifecycle_synchronization(cx)
            })
        });
        synchronized.await;
    }

    pub async fn rise_wire_send_prepared_text(
        this: Entity<Self>,
        text: String,
        session: RiseWireSocketSession,
        request_summary: String,
        mut cx: AsyncApp,
    ) -> anyhow::Result<()> {
        let task = this.read_with(&cx, |manager, cx| {
            let owns_session = manager.transport_account_user_id.as_deref()
                == Some(session.account.account_id.as_str())
                && manager.transport_account_epoch.as_deref() == Some(session.account.epoch.as_str());
            manager
                .web_socket_task
                .clone()
                .filter(|_| owns_session && manager.session_is_live(&session, cx))
        });
        let Some(task) = task else {
            return Err(anyhow::Error::new(SaiWsSendError::NotConnected));
        };
        let send_this = this.clone();
        let body: SaiWireSerialBody<()> = Box::new(move |cx| {
            async move {
                let still_live = send_this.read_with(&cx, |manager, cx| {
                    manager.is_current_task(&task) && manager.session_is_live(&session, cx)
                });
                if !still_live {
                    return Err(anyhow::Error::new(SaiWsSendError::NotConnected));
                }
                RiseEngineObservability::transport_will_send(&request_summary);
                Self::send_wire_text(text, &task).await
            }
            .boxed_local()
        });
        Self::with_serialized_wire_send(&this, body, &mut cx).await
    }

    fn session_is_live(&self, session: &RiseWireSocketSession, cx: &App) -> bool {
        self.transport_is_live(cx)
            && self.is_connected()
            && session.transport_epoch == self.transport_epoch
            && SaiAccountCacheScope::is_current_account_epoch(
                Some(&session.account.account_id),
                Some(&session.account.epoch),
            )
    }

    async fn send_wire_text(text: String, task: &Arc<SaiWebSocketTask>) -> anyhow::Result<()> {
        task.send(text)
            .await
            .map_err(|error| anyhow::Error::new(SaiWsSendError::SendFailed(error)))
    }

    fn try_serve_cached_response_when_wire_unavailable(
        options: &SaiWsRequestOptions,
        on_success: Option<&SaiWsOnSuccess>,
        reason: &'static str,
        cx: &mut App,
    ) -> bool {
        let is_initial_page = options
            .pagination
            .as_ref()
            .is_none_or(|pagination| pagination.is_initial_page());
        let Some(ck) = options.cache_key.as_deref().filter(|key| !key.is_empty()) else {
            return false;
        };
        if !is_initial_page || options.force_bypass_cache_read {
            return false;
        }
        let (cached, _, _, selected_source) = SaiWsResponseCache::peek_with_source(options, true, None, cx);
        let Some(cached) = cached else {
            return false;
        };
        SaiWsResponseCache::hydrate_memory(
            ck,
            &cached,
            selected_source == Some(SaiWsCacheSelectionSource::LegacyDisk),
            cx,
        );
        let served = Self::ws_served_cache_layer(selected_source);
        let fetch_id = Self::fetch_id(options);
        let log_fetch_id = fetch_id.clone();
        let log_key = ck.to_owned();
        Sai::log(
            LOG_NAME,
            move || format!("offline cache hit {log_fetch_id} key={log_key} from={served} ({reason})"),
            SaiLogKind::Cache,
            cx,
        );
        Self::fulfil_from_cache(options, on_success, &cached, cx);
        if SAI_DEBUG_TOOLS && !options.exclude_from_debugger {
            let served_headers = HashMap::from([
                ("x-sai-resolve".to_owned(), "offline-cache-hit".to_owned()),
                ("x-sai-served-from".to_owned(), served.to_owned()),
                ("x-sai-cache-key".to_owned(), ck.to_owned()),
                ("x-sai-reason".to_owned(), reason.to_owned()),
            ]);
            let response_body_preview = SaiHttpTraceRecorder::pretty_json_string_from_any(
                (!cached.is_null()).then_some(&cached),
            );
            let url = ck.to_owned();
            SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
                store.append_completed_ws_trace(
                    fetch_id,
                    url,
                    String::new(),
                    served_headers,
                    response_body_preview,
                    SaiCompletedWsTraceOptions {
                        trace_tags: vec!["Offline".to_owned(), "Cache".to_owned()],
                        ..SaiCompletedWsTraceOptions::default()
                    },
                    cx,
                )
            });
        }
        true
    }

    fn validate_pagination_configuration(
        options: &SaiWsRequestOptions,
    ) -> Result<(), anyhow::Error> {
        let Some(pagination) = &options.pagination else {
            return Ok(());
        };
        if options.cache_key.as_deref().is_none_or(str::is_empty) {
            return Err(SaiWsPaginationError::CacheKeyRequired.into());
        }
        if !options.store_response_in_cache {
            return Err(SaiWsPaginationError::CacheStorageRequired.into());
        }
        if !Self::has_pagination_path_components(options.path_to_array.as_deref()) {
            return Err(SaiWsPaginationError::InvalidItemsPath(
                options.path_to_array.clone().unwrap_or_default(),
            )
            .into());
        }
        if !Self::has_pagination_path_components(Some(&pagination.response_relative_id_path)) {
            return Err(SaiWsPaginationError::InvalidRelativeIdPath(
                pagination.response_relative_id_path.clone(),
            )
            .into());
        }
        if let Some(fallback_path) = &pagination.response_relative_id_fallback_path
            && !Self::has_pagination_path_components(Some(fallback_path))
        {
            return Err(SaiWsPaginationError::InvalidRelativeIdPath(fallback_path.clone()).into());
        }
        if let Some(has_more_path) = &pagination.has_more_path
            && !Self::has_pagination_path_components(Some(has_more_path))
        {
            return Err(SaiWsPaginationError::InvalidHasMorePath(has_more_path.clone()).into());
        }
        if options.allow_concurrent_while_pending {
            return Err(SaiWsPaginationError::ConcurrentRequestsNotSupported.into());
        }
        Ok(())
    }

    fn has_pagination_path_components(path: Option<&str>) -> bool {
        path.is_some_and(|path| path.split('.').any(|component| !trimming_whitespaces_and_newlines(component).is_empty()))
    }

    fn record_pagination_failure(options: &SaiWsRequestOptions, error: &anyhow::Error, cx: &mut App) {
        let binding = SaiWsPendingCacheBinding::new(options);
        SaiWsPaginationStateStore::fail(Some(&binding), error, cx);
        let is_initial_page = options
            .pagination
            .as_ref()
            .is_none_or(|pagination| pagination.is_initial_page());
        if is_initial_page && let Some(slice) = &options.sai_slice {
            slice.mark_rejected(error);
        }
    }

    fn ws_served_cache_layer(source: Option<SaiWsCacheSelectionSource>) -> &'static str {
        match source {
            Some(SaiWsCacheSelectionSource::Memory) => "memory",
            Some(SaiWsCacheSelectionSource::LegacyDisk) => "disk",
            None => "unknown",
        }
    }

    fn trace_ws_request_body(data_value: &Value) -> String {
        match data_value {
            Value::Object(d) => SaiHttpTraceRecorder::pretty_json_string_from_json_object(d),
            other => SaiHttpTraceRecorder::pretty_json_string_from_any(Some(other)),
        }
    }

    fn fetch_id(options: &SaiWsRequestOptions) -> String {
        format!("{}.{}", options.service, options.method)
    }

    #[allow(clippy::too_many_arguments)]
    fn append_ws_trace_screen_refresh_cache_served(
        manager: &Entity<Self>,
        options: &SaiWsRequestOptions,
        cache_key: &str,
        data_value: &Value,
        had_memory: bool,
        had_disk: bool,
        cached: &Value,
        served: &str,
        cx: &mut App,
    ) {
        if options.exclude_from_debugger {
            return;
        }
        let headers = HashMap::from([
            ("x-sai-resolve".to_owned(), "screen-refresh-cache-served".to_owned()),
            ("x-sai-cache-key".to_owned(), cache_key.to_owned()),
            ("x-sai-served-from".to_owned(), served.to_owned()),
            ("x-sai-screen-refresh".to_owned(), "wire-scheduled".to_owned()),
            ("x-sai-had-memory".to_owned(), had_memory.to_string()),
            ("x-sai-had-disk".to_owned(), had_disk.to_string()),
        ]);
        let fetch_id = Self::fetch_id(options);
        let log_fetch_id = fetch_id.clone();
        let log_key = cache_key.to_owned();
        SaiWsScreenRefreshDebug::log(
            move || format!("debugger appendCompletedWsTrace cache-served {log_fetch_id} key={log_key}"),
            cx,
        );
        let url = manager.read(cx).active_url_display_string();
        let request_body_wire = Self::trace_ws_request_body(data_value);
        let response_body_preview =
            SaiHttpTraceRecorder::pretty_json_string_from_any((!cached.is_null()).then_some(cached));
        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
            store.append_completed_ws_trace(
                fetch_id,
                url,
                request_body_wire,
                headers,
                response_body_preview,
                SaiCompletedWsTraceOptions {
                    trace_tags: SaiTraceTag::screen_refresh_wire(),
                    ..SaiCompletedWsTraceOptions::default()
                },
                cx,
            )
        });
        let count = SaiHttpTraceStore::shared(cx).read(cx).entries().len();
        SaiWsScreenRefreshDebug::log(move || format!("debugger entries.count={count}"), cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn append_ws_trace_short_circuit(
        manager: &Entity<Self>,
        options: &SaiWsRequestOptions,
        cache_key: &str,
        data_value: &Value,
        had_memory: bool,
        had_disk: bool,
        cached: &Value,
        served: &str,
        resolve: &str,
        cx: &mut App,
    ) {
        if options.exclude_from_debugger {
            return;
        }
        let headers = HashMap::from([
            ("x-sai-resolve".to_owned(), resolve.to_owned()),
            ("x-sai-cache-key".to_owned(), cache_key.to_owned()),
            ("x-sai-served-from".to_owned(), served.to_owned()),
            ("x-sai-had-memory".to_owned(), had_memory.to_string()),
            ("x-sai-had-disk".to_owned(), had_disk.to_string()),
            (
                "x-sai-preferred-layer".to_owned(),
                options.preferred_cache_layer.raw_value().to_owned(),
            ),
            ("x-sai-reload-if-cached".to_owned(), options.reload_if_cached.to_string()),
            (
                "x-sai-reload-when-only-disk-cached".to_owned(),
                options.reload_when_only_disk_cached.to_string(),
            ),
        ]);
        let url = manager.read(cx).active_url_display_string();
        let request_body_wire = Self::trace_ws_request_body(data_value);
        let response_body_preview =
            SaiHttpTraceRecorder::pretty_json_string_from_any((!cached.is_null()).then_some(cached));
        let fetch_id = Self::fetch_id(options);
        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
            store.append_completed_ws_trace(
                fetch_id,
                url,
                request_body_wire,
                headers,
                response_body_preview,
                SaiCompletedWsTraceOptions::default(),
                cx,
            )
        });
    }

    fn append_ws_trace_deduped(
        manager: &Entity<Self>,
        options: &SaiWsRequestOptions,
        cache_key: &str,
        data_value: &Value,
        existing_wire_id: u64,
        cx: &mut App,
    ) {
        if options.exclude_from_debugger {
            return;
        }
        let headers = HashMap::from([
            ("x-sai-resolve".to_owned(), "deduped-in-flight".to_owned()),
            ("x-sai-cache-key".to_owned(), cache_key.to_owned()),
            ("x-sai-inflight-wire-id".to_owned(), existing_wire_id.to_string()),
        ]);
        let url = manager.read(cx).active_url_display_string();
        let request_body_wire = Self::trace_ws_request_body(data_value);
        let fetch_id = Self::fetch_id(options);
        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
            store.append_completed_ws_trace(
                fetch_id,
                url,
                request_body_wire,
                headers,
                format!(
                    "Handlers attached to pending wire request id={existing_wire_id}. Final payload is recorded when that request completes."
                ),
                SaiCompletedWsTraceOptions::default(),
                cx,
            )
        });
    }

    /// trace of a send refused before any wire when no socket can carry it
    fn append_ws_trace_not_connected(options: &SaiWsRequestOptions, reason: &str, cx: &mut App) {
        if !SAI_DEBUG_TOOLS || options.exclude_from_debugger {
            return;
        }
        let fetch_id = Self::fetch_id(options);
        let url = options.cache_key.clone().unwrap_or_else(|| fetch_id.clone());
        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
            store.append_completed_ws_trace(
                fetch_id,
                url,
                String::new(),
                HashMap::new(),
                String::new(),
                SaiCompletedWsTraceOptions {
                    response_status: 0,
                    transport_error: Some(format!("WebSocket not connected (reason: {reason})")),
                    trace_tags: vec!["Offline".to_owned(), "NoCache".to_owned()],
                    ..SaiCompletedWsTraceOptions::default()
                },
                cx,
            )
        });
    }

    fn log_async(cx: &AsyncApp, message: impl Into<String>, kind: SaiLogKind) {
        let message = message.into();
        cx.update(|cx| Sai::log(LOG_NAME, move || message, kind, cx));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaiWebSocketCloseCode {
    GoingAway,
    AbnormalClosure,
}

enum SaiWebSocketTaskCommand {
    Send(String, oneshot::Sender<Result<(), String>>),
    Ping(oneshot::Sender<Result<(), String>>),
    Cancel(SaiWebSocketCloseCode),
}

/// one socket connection
/// commands sent before the upgrade completes wait for it and fail with it
struct SaiWebSocketTask {
    commands: tokio_mpsc::UnboundedSender<SaiWebSocketTaskCommand>,
    frames: futures::lock::Mutex<tokio_mpsc::Receiver<Result<SaiWsInboundFrame, String>>>,
    upgrade_status: Arc<Mutex<Option<u16>>>,
}

impl SaiWebSocketTask {
    fn resume(url: &Url, subprotocol: &str, maximum_message_size: usize) -> Arc<Self> {
        let (commands, inbox) = tokio_mpsc::unbounded_channel();
        let (frames_sender, frames) = tokio_mpsc::channel(1);
        let upgrade_status = Arc::new(Mutex::new(None));
        let worker = SaiWebSocketWorker {
            url: url.absolute_string().to_owned(),
            subprotocol: subprotocol.to_owned(),
            maximum_message_size,
            upgrade_status: Arc::clone(&upgrade_status),
        };
        drop(RiseAccountRuntime::current().spawn(worker.run(inbox, frames_sender)));
        Arc::new(Self {
            commands,
            frames: futures::lock::Mutex::new(frames),
            upgrade_status,
        })
    }

    async fn receive(&self) -> Result<SaiWsInboundFrame, String> {
        let mut frames = self.frames.lock().await;
        frames
            .recv()
            .await
            .unwrap_or_else(|| Err("socket closed".to_owned()))
    }

    fn send(&self, text: String) -> impl std::future::Future<Output = Result<(), String>> + use<> {
        let (reply, answer) = oneshot::channel();
        let queued = self
            .commands
            .send(SaiWebSocketTaskCommand::Send(text, reply))
            .is_ok();
        async move {
            if !queued {
                return Err("socket closed".to_owned());
            }
            answer.await.unwrap_or_else(|_| Err("socket closed".to_owned()))
        }
    }

    fn send_ping(&self) -> impl std::future::Future<Output = Result<(), String>> + use<> {
        let (reply, answer) = oneshot::channel();
        let queued = self.commands.send(SaiWebSocketTaskCommand::Ping(reply)).is_ok();
        async move {
            if !queued {
                return Err("socket closed".to_owned());
            }
            answer.await.unwrap_or_else(|_| Err("socket closed".to_owned()))
        }
    }

    fn cancel(&self, code: SaiWebSocketCloseCode) {
        let _ = self.commands.send(SaiWebSocketTaskCommand::Cancel(code));
    }

    /// status code of a refused upgrade
    fn upgrade_status(&self) -> Option<u16> {
        self.upgrade_status.lock().ok().and_then(|status| *status)
    }
}

struct SaiWebSocketWorker {
    url: String,
    subprotocol: String,
    maximum_message_size: usize,
    upgrade_status: Arc<Mutex<Option<u16>>>,
}

impl SaiWebSocketWorker {
    async fn run(
        self,
        mut inbox: tokio_mpsc::UnboundedReceiver<SaiWebSocketTaskCommand>,
        frames: tokio_mpsc::Sender<Result<SaiWsInboundFrame, String>>,
    ) {
        let socket = match self.connect().await {
            Ok(socket) => socket,
            Err(error) => {
                inbox.close();
                while let Some(command) = inbox.recv().await {
                    match command {
                        SaiWebSocketTaskCommand::Send(_, reply)
                        | SaiWebSocketTaskCommand::Ping(reply) => {
                            let _ = reply.send(Err(error.clone()));
                        }
                        SaiWebSocketTaskCommand::Cancel(_) => {}
                    }
                }
                let _ = frames.send(Err(error)).await;
                return;
            }
        };
        let (mut writer, mut reader) = socket.split();
        let mut pending_pongs: VecDeque<oneshot::Sender<Result<(), String>>> = VecDeque::new();
        let mut held_frame: Option<Result<SaiWsInboundFrame, String>> = None;
        let failure = loop {
            tokio::select! {
                command = inbox.recv() => match command {
                    Some(SaiWebSocketTaskCommand::Send(text, reply)) => {
                        let sent = writer.send(Message::text(text)).await;
                        let _ = reply.send(sent.map_err(|error| error.to_string()));
                    }
                    Some(SaiWebSocketTaskCommand::Ping(reply)) => {
                        match writer.send(Message::Ping(Default::default())).await {
                            Ok(()) => pending_pongs.push_back(reply),
                            Err(error) => {
                                let _ = reply.send(Err(error.to_string()));
                            }
                        }
                    }
                    Some(SaiWebSocketTaskCommand::Cancel(SaiWebSocketCloseCode::GoingAway)) => {
                        let _ = writer
                            .send(Message::Close(Some(CloseFrame {
                                code: CloseCode::Away,
                                reason: Default::default(),
                            })))
                            .await;
                        break "socket cancelled".to_owned();
                    }
                    Some(SaiWebSocketTaskCommand::Cancel(SaiWebSocketCloseCode::AbnormalClosure))
                    | None => break "socket cancelled".to_owned(),
                },
                permit = frames.reserve(), if held_frame.is_some() => match permit {
                    Ok(permit) => {
                        if let Some(frame) = held_frame.take() {
                            permit.send(frame);
                        }
                    }
                    Err(_) => break "socket released".to_owned(),
                },
                inbound = reader.next(), if held_frame.is_none() => match inbound {
                    Some(Ok(Message::Text(text))) => {
                        held_frame = Some(Ok(SaiWsInboundFrame::String(text.as_str().to_owned())));
                    }
                    Some(Ok(Message::Binary(data))) => {
                        held_frame = Some(Ok(SaiWsInboundFrame::Data(data.to_vec())));
                    }
                    Some(Ok(Message::Pong(_))) => {
                        if let Some(reply) = pending_pongs.pop_front() {
                            let _ = reply.send(Ok(()));
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break "socket closed by peer".to_owned(),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => break error.to_string(),
                },
            }
        };
        for reply in pending_pongs {
            let _ = reply.send(Err(failure.clone()));
        }
        if let Some(frame) = held_frame.take() {
            let _ = frames.send(frame).await;
        }
        let _ = frames.send(Err(failure)).await;
    }

    async fn connect(
        &self,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        String,
    > {
        let mut request = self
            .url
            .as_str()
            .into_client_request()
            .map_err(|error| error.to_string())?;
        let protocol = HeaderValue::from_str(&self.subprotocol).map_err(|error| error.to_string())?;
        request.headers_mut().insert(
            tokio_tungstenite::tungstenite::http::header::SEC_WEBSOCKET_PROTOCOL,
            protocol,
        );
        let config = WebSocketConfig::default()
            .max_message_size(Some(self.maximum_message_size))
            .max_frame_size(Some(self.maximum_message_size));
        match crate::core::engine_bridge::network_transport_config::connect_websocket(request, Some(config)).await {
            Ok((socket, _)) => Ok(socket),
            Err(error) => {
                if let tokio_tungstenite::tungstenite::Error::Http(response) = &error
                    && let Ok(mut status) = self.upgrade_status.lock()
                {
                    *status = Some(response.status().as_u16());
                }
                Err(error.to_string())
            }
        }
    }
}

struct SaiAbortOnDrop(tokio::task::AbortHandle);

impl Drop for SaiAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct SaiMarkOnDrop(Arc<AtomicBool>);

impl Drop for SaiMarkOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct SaiReleaseSocketOnDrop(Option<Arc<SaiWebSocketTask>>);

impl SaiReleaseSocketOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for SaiReleaseSocketOnDrop {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.cancel(SaiWebSocketCloseCode::GoingAway);
        }
    }
}

struct SaiFinishPreWireOnDrop {
    wire_request_id: Option<u64>,
    cx: AsyncApp,
}

impl Drop for SaiFinishPreWireOnDrop {
    fn drop(&mut self) {
        if let Some(wire_request_id) = self.wire_request_id.take() {
            self.cx.update(|cx| {
                SaiWsOptimisticCoordinator::finish_pre_wire_optimistic(wire_request_id, cx)
            });
        }
    }
}
