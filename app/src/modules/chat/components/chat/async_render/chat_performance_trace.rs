use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{App, BackgroundExecutor, ForegroundExecutor, ScrollHandle, Task, Window};
use parking_lot::Mutex;
use rise_canon::rise_key::{
    event_collector, events, file, ledger_admin, map, music, notify, otp, post, product_admin, user,
};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, TimeZone};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_platform::accessibility::reduce_motion;
use rise_platform::app_info::AppInfo;
use rise_platform::crash_diagnostics::resident_memory_mb;
use rise_platform::display_cadence::maximum_frames_per_second;
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileManager};
use rise_platform::power::system_low_power_mode;
use rise_platform::process_info::{arguments, is_debugger_attached, thermal_state};
use serde_json::{Map, Value};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::diagnostics::rise_diagnostics::RiseDiagnostics;
use crate::core::lib::device_info::DeviceInfo;

const SESSION_TIMEOUT_SECONDS: u64 = 120;
const SCROLL_SETTLE_DELAY: Duration = Duration::from_millis(800);
const SLOW_RUN_LOOP_TURN_MS: f64 = 8.0;
const RUN_LOOP_PROBE_INTERVAL: Duration = Duration::from_millis(4);
const MINIMUM_FRAME_INTERVAL: f64 = 1.0 / 240.0;
const TRACE_FILE_PREFIX: &str = "riseonly-chatperf-";
const KEPT_OLD_TRACES: usize = 4;

fn current_media_time() -> f64 {
    RiseClock::monotonic().as_secs_f64()
}

/// opaque to callers, only the trace that issued it reads it back
#[derive(Clone, Debug)]
pub struct OperationToken {
    name: String,
    started_at: f64,
    fields: HashMap<String, String>,
    session_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameClassification {
    pub missed_frames: i64,
    pub is_hitch: bool,
}

#[derive(Clone, Copy, Default)]
struct FrameAggregate {
    first_timestamp: f64,
    last_timestamp: f64,
    frame_count: i64,
    frames_over12_ms: i64,
    frames_over20_ms: i64,
    frames_over50_ms: i64,
    missed_frames: i64,
    worst_frame: f64,
}

impl FrameAggregate {
    fn reset(&mut self, timestamp: f64) {
        *self = Self { first_timestamp: timestamp, last_timestamp: timestamp, ..Self::default() };
    }
}

const OPERATION_BUCKET_COUNT: usize = 18;

static OPERATION_UPPER_BOUNDS_MS: LazyLock<[f64; OPERATION_BUCKET_COUNT]> = LazyLock::new(|| {
    let mut bounds = [0.0; OPERATION_BUCKET_COUNT];
    for (index, bound) in bounds.iter_mut().enumerate() {
        *bound = 0.125 * 2f64.powi(index as i32);
    }
    bounds
});

#[derive(Clone)]
struct OperationAggregate {
    buckets: [i64; OPERATION_BUCKET_COUNT + 1],
    count: i64,
    total_ms: f64,
    maximum_ms: f64,
}

impl Default for OperationAggregate {
    fn default() -> Self {
        Self { buckets: [0; OPERATION_BUCKET_COUNT + 1], count: 0, total_ms: 0.0, maximum_ms: 0.0 }
    }
}

impl OperationAggregate {
    fn upper_bounds_ms() -> &'static [f64; OPERATION_BUCKET_COUNT] {
        &OPERATION_UPPER_BOUNDS_MS
    }

    fn add(&mut self, duration_ms: f64) {
        let index = Self::upper_bounds_ms()
            .iter()
            .position(|bound| duration_ms <= *bound)
            .unwrap_or(OPERATION_BUCKET_COUNT);
        self.buckets[index] += 1;
        self.count += 1;
        self.total_ms += duration_ms;
        self.maximum_ms = self.maximum_ms.max(duration_ms);
    }

    fn percentile_upper_bound(&self, fraction: f64) -> f64 {
        let target = ((self.count as f64 * fraction).ceil() as i64).max(1);
        let mut seen = 0;
        for (index, count) in self.buckets.iter().enumerate() {
            seen += count;
            if seen >= target {
                return Self::upper_bounds_ms().get(index).copied().unwrap_or(self.maximum_ms);
            }
        }
        self.maximum_ms
    }
}

#[derive(Clone)]
struct ScrollContext {
    phase: String,
    offset_y: f32,
    velocity_y: f32,
    visible_count: i64,
    mounted_count: i64,
    placeholder_count: i64,
    last_scroll_end: Option<f64>,
}

impl Default for ScrollContext {
    fn default() -> Self {
        Self {
            phase: "idle".to_owned(),
            offset_y: 0.0,
            velocity_y: 0.0,
            visible_count: 0,
            mounted_count: 0,
            placeholder_count: 0,
            last_scroll_end: None,
        }
    }
}

type EventObserver = Rc<dyn Fn(&str, &HashMap<String, String>)>;
type FinishCompletion = Box<dyn FnOnce(Option<PathBuf>)>;

/// executors the trace schedules on, captured from the first App the trace sees
#[derive(Clone)]
struct TraceExecutors {
    foreground: ForegroundExecutor,
    background: BackgroundExecutor,
}

impl TraceExecutors {
    fn new(cx: &App) -> Self {
        Self { foreground: cx.foreground_executor().clone(), background: cx.background_executor().clone() }
    }
}

/// display link, one sampler per recording session bound to the window that shows the list
struct DisplayLink {
    generation: u64,
    is_paused: bool,
    expected_interval: f64,
}

/// chat list scroll view the probe attached, with the probe that owns the attachment
struct ChatListScrollView {
    owner: u64,
    scroll_handle: ScrollHandle,
}

struct TraceState {
    writer: ChatPerformanceTraceWriter,
    executors: Option<TraceExecutors>,
    is_recording: bool,
    event_observer: Option<EventObserver>,
    observer_fingerprint_salt: String,
    is_finishing: bool,
    latest_trace_url: Option<PathBuf>,
    active_chat_fingerprint: String,
    session_id: String,
    fingerprint_salt: String,
    session_generation: u64,
    session_started_at: f64,
    display_link: Option<DisplayLink>,
    display_link_generation: u64,
    chat_list_scroll_view: Option<ChatListScrollView>,
    is_chat_list_session: bool,
    previous_chat_list_offset: Option<f32>,
    previous_chat_list_timestamp: Option<f64>,
    run_loop_observer: Option<Task<()>>,
    frame_aggregate: FrameAggregate,
    scroll_context: ScrollContext,
    counters: HashMap<String, i64>,
    operations: HashMap<String, OperationAggregate>,
    last_activity: String,
    last_activity_at: f64,
    recording_timeout: Option<Task<()>>,
    scroll_phase_generation: u64,
    is_paused_for_application_lifecycle: bool,
    application_lifecycle_observers: Vec<RiseSubscription>,
    pending_finish_completions: Vec<FinishCompletion>,
}

impl TraceState {
    fn new() -> Self {
        Self {
            writer: ChatPerformanceTraceWriter::new(),
            executors: None,
            is_recording: false,
            event_observer: None,
            observer_fingerprint_salt: String::new(),
            is_finishing: false,
            latest_trace_url: None,
            active_chat_fingerprint: String::new(),
            session_id: String::new(),
            fingerprint_salt: String::new(),
            session_generation: 0,
            session_started_at: 0.0,
            display_link: None,
            display_link_generation: 0,
            chat_list_scroll_view: None,
            is_chat_list_session: false,
            previous_chat_list_offset: None,
            previous_chat_list_timestamp: None,
            run_loop_observer: None,
            frame_aggregate: FrameAggregate::default(),
            scroll_context: ScrollContext::default(),
            counters: HashMap::new(),
            operations: HashMap::new(),
            last_activity: "idle".to_owned(),
            last_activity_at: 0.0,
            recording_timeout: None,
            scroll_phase_generation: 0,
            is_paused_for_application_lifecycle: false,
            application_lifecycle_observers: Vec::new(),
            pending_finish_completions: Vec::new(),
        }
    }

    fn is_enabled(&self) -> bool {
        self.is_recording || self.event_observer.is_some()
    }

    fn active_salt(&self) -> &str {
        if self.is_recording { &self.fingerprint_salt } else { &self.observer_fingerprint_salt }
    }

    fn update_scroll_context(
        &mut self,
        phase: &str,
        offset_y: f32,
        velocity_y: f32,
        visible_count: i64,
        mounted_count: i64,
        placeholder_count: i64,
    ) {
        if !self.is_enabled() {
            return;
        }
        self.scroll_context.phase = phase.to_owned();
        self.scroll_context.offset_y = offset_y;
        self.scroll_context.velocity_y = velocity_y;
        self.scroll_context.visible_count = visible_count;
        self.scroll_context.mounted_count = mounted_count;
        self.scroll_context.placeholder_count = placeholder_count;
    }
}

thread_local! {
    static STATE: RefCell<TraceState> = RefCell::new(TraceState::new());
}

fn with_state<R>(body: impl FnOnce(&mut TraceState) -> R) -> R {
    STATE.with_borrow_mut(body)
}

static SHARED: ChatPerformanceTrace = ChatPerformanceTrace { _private: () };

static REQUESTS_MAXIMUM_SAMPLER_REFRESH: LazyLock<bool> =
    LazyLock::new(|| arguments().iter().any(|argument| argument == "-riseChatPerformanceMaxRefresh"));

static IS_CONTROL_AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
    cfg!(debug_assertions) || arguments().iter().any(|argument| argument == "-riseChatPerformanceTools")
});

/// handle to the main thread trace
pub struct ChatPerformanceTrace {
    _private: (),
}

impl ChatPerformanceTrace {
    pub fn shared() -> &'static ChatPerformanceTrace {
        &SHARED
    }

    pub fn is_control_available() -> bool {
        *IS_CONTROL_AVAILABLE
    }

    pub fn is_recording(&self) -> bool {
        with_state(|state| state.is_recording)
    }

    pub fn is_enabled(&self) -> bool {
        with_state(|state| state.is_enabled())
    }

    pub fn is_finishing(&self) -> bool {
        with_state(|state| state.is_finishing)
    }

    pub fn latest_trace_url(&self) -> Option<PathBuf> {
        with_state(|state| state.latest_trace_url.clone())
    }

    /// reuses operation instrumentation in the app export without a second frame sampler
    pub fn set_event_observer(&self, observer: Option<Box<dyn Fn(&str, &HashMap<String, String>)>>, cx: &App) {
        with_state(|state| {
            if state.executors.is_none() {
                state.executors = Some(TraceExecutors::new(cx));
            }
            state.session_generation = state.session_generation.wrapping_add(1);
            state.observer_fingerprint_salt =
                if observer.is_some() { Uuid::new_v4().uuid_string() } else { String::new() };
            state.event_observer = observer.map(EventObserver::from);
            if !state.is_recording {
                state.scroll_context = ScrollContext::default();
                state.last_activity = "idle".to_owned();
                state.last_activity_at = 0.0;
            }
        });
    }

    pub fn diagnostic_context(&self) -> HashMap<String, String> {
        with_state(|state| {
            if !state.is_enabled() {
                return HashMap::new();
            }
            let activity_age_ms = if state.last_activity_at > 0.0 {
                Self::decimal((current_media_time() - state.last_activity_at) * 1_000.0)
            } else {
                "-1".to_owned()
            };
            HashMap::from([
                ("chat_activity".to_owned(), state.last_activity.clone()),
                ("chat_activity_age_ms".to_owned(), activity_age_ms),
                ("chat_phase".to_owned(), state.scroll_context.phase.clone()),
                ("chat_visible".to_owned(), state.scroll_context.visible_count.to_string()),
                ("chat_mounted".to_owned(), state.scroll_context.mounted_count.to_string()),
                ("chat_placeholders".to_owned(), state.scroll_context.placeholder_count.to_string()),
            ])
        })
    }

    /// starts a session sampled on window, the one showing the chat list or history
    /// its display gives the frame cadence and its backing scale gives the scale metadata
    pub fn start(&self, chat_id: &str, is_chat_list: bool, window: &mut Window, cx: &mut App) -> bool {
        let refused = with_state(|state| state.is_recording || state.is_finishing);
        if !Self::is_control_available() || refused || chat_id.is_empty() {
            return false;
        }
        let next_session_id = Uuid::new_v4().uuid_string().to_lowercase();
        let Some(url) = Self::make_trace_url(&next_session_id) else {
            return false;
        };
        let executors = TraceExecutors::new(cx);
        let maximum_fps = maximum_frames_per_second(window);
        let scale = window.scale_factor();
        let is_paused = RiseAppLifecycle::scene_phase(cx) != ScenePhase::Active;

        let metadata = with_state(|state| {
            state.executors = Some(executors.clone());
            state.session_generation = state.session_generation.wrapping_add(1);
            state.session_id = next_session_id;
            state.fingerprint_salt = Uuid::new_v4().uuid_string().to_lowercase();
            state.is_recording = true;
            state.latest_trace_url = Some(url.clone());
            state.session_started_at = current_media_time();
            state.active_chat_fingerprint = Self::fingerprint(chat_id, &state.fingerprint_salt);
            state.scroll_context = ScrollContext::default();
            state.is_chat_list_session = is_chat_list;
            state.previous_chat_list_offset = None;
            state.previous_chat_list_timestamp = None;
            state.scroll_phase_generation = 0;
            state.counters.clear();
            state.operations.clear();
            state.last_activity = "recording.start".to_owned();
            state.last_activity_at = state.session_started_at;
            state.frame_aggregate = FrameAggregate::default();
            Self::session_metadata(&state.session_id, &state.active_chat_fingerprint, maximum_fps, scale)
        });

        with_state(|state| state.writer.start(url, metadata, &executors.background));
        self.install_frame_sampler(maximum_fps, is_paused, window);
        self.install_run_loop_observer(&executors);
        self.install_application_lifecycle_observers();
        let surface = if is_chat_list { "chat_list" } else { "history" };
        self.record("session.start", HashMap::from([(post::SURFACE.to_owned(), surface.to_owned())]));

        let background = executors.background.clone();
        let timeout = executors.foreground.spawn(async move {
            background.timer(Duration::from_secs(SESSION_TIMEOUT_SECONDS)).await;
            ChatPerformanceTrace::shared().recording_did_time_out();
        });
        with_state(|state| state.recording_timeout = Some(timeout));
        true
    }

    fn recording_did_time_out(&self) {
        let is_recording = with_state(|state| {
            if let Some(timeout) = state.recording_timeout.take() {
                timeout.detach();
            }
            state.is_recording
        });
        if !is_recording {
            return;
        }
        self.record(
            "session.timeout",
            HashMap::from([("limit_s".to_owned(), SESSION_TIMEOUT_SECONDS.to_string())]),
        );
        self.finish_recording("timeout");
    }

    pub fn stop(&self, reason: &str, completion: Option<Box<dyn FnOnce(Option<PathBuf>)>>) {
        enum Next {
            Done,
            Answer(FinishCompletion, Option<PathBuf>),
            Finish,
        }
        let next = with_state(|state| {
            if state.is_finishing {
                if let Some(completion) = completion {
                    state.pending_finish_completions.push(completion);
                }
                return Next::Done;
            }
            if !state.is_recording {
                return match completion {
                    Some(completion) => Next::Answer(completion, state.latest_trace_url.clone()),
                    None => Next::Done,
                };
            }
            if let Some(completion) = completion {
                state.pending_finish_completions.push(completion);
            }
            Next::Finish
        });
        match next {
            Next::Done => {}
            Next::Answer(completion, url) => completion(url),
            Next::Finish => self.finish_recording(reason),
        }
    }

    pub fn stop_if_recording(&self, chat_id: &str, reason: &str) -> bool {
        let matches = with_state(|state| {
            state.is_recording
                && !state.is_finishing
                && !chat_id.is_empty()
                && state.active_chat_fingerprint == Self::fingerprint(chat_id, &state.fingerprint_salt)
        });
        if !matches {
            return false;
        }
        self.finish_recording(reason);
        true
    }

    pub fn record(&self, event: &str, fields: HashMap<String, String>) {
        if !self.is_enabled() {
            return;
        }
        self.emit(event, fields, true);
    }

    /// records a chat scoped lifecycle event without writing the raw cache key or message scope
    /// into the trace
    pub fn record_scope_event(&self, event: &str, scope: &str, fields: HashMap<String, String>) {
        let Some(salt) = with_state(|state| state.is_enabled().then(|| state.active_salt().to_owned())) else {
            return;
        };
        let mut resolved_fields = fields;
        resolved_fields.insert(otp::SCOPE.to_owned(), Self::fingerprint(scope, &salt));
        self.emit(event, resolved_fields, true);
    }

    fn emit(&self, event: &str, fields: HashMap<String, String>, increments_counter: bool) {
        let Some((now, observer)) = with_state(|state| {
            if !state.is_enabled() {
                return None;
            }
            let now = current_media_time();
            if Self::advances_activity(event) {
                state.last_activity = event.to_owned();
                state.last_activity_at = now;
            }
            Some((now, state.event_observer.clone()))
        }) else {
            return;
        };
        if let Some(observer) = observer {
            observer(event, &fields);
        }
        with_state(|state| {
            if !state.is_recording {
                return;
            }
            if increments_counter {
                *state.counters.entry(event.to_owned()).or_insert(0) += 1;
            }
            tracing::trace!(target: "ChatPerformance", "{event}");
            if let Some(executors) = state.executors.as_ref() {
                state.writer.append(event, now - state.session_started_at, fields, &executors.background);
            }
        });
    }

    pub fn begin_operation(&self, name: &str, fields: HashMap<String, String>) -> Option<OperationToken> {
        with_state(|state| {
            if !state.is_enabled() {
                return None;
            }
            let started_at = current_media_time();
            state.last_activity = name.to_owned();
            state.last_activity_at = started_at;
            Some(OperationToken {
                name: name.to_owned(),
                started_at,
                fields,
                session_generation: state.session_generation,
            })
        })
    }

    pub fn end_operation(&self, token: Option<OperationToken>, threshold_ms: f64, fields: HashMap<String, String>) {
        let Some(token) = token else {
            return;
        };
        let event_fields = with_state(|state| {
            if !state.is_enabled() || token.session_generation != state.session_generation {
                return None;
            }
            let duration_ms = (current_media_time() - token.started_at) * 1_000.0;
            if state.is_recording {
                *state.counters.entry(format!("operation.{}", token.name)).or_insert(0) += 1;
                state.operations.entry(token.name.clone()).or_default().add(duration_ms);
            }
            if duration_ms < threshold_ms {
                return None;
            }
            let mut event_fields = token.fields.clone();
            event_fields.extend(fields);
            event_fields.insert(otp::DURATION_MS.to_owned(), Self::decimal(duration_ms));
            event_fields.insert("phase".to_owned(), state.scroll_context.phase.clone());
            event_fields.insert("velocity".to_owned(), Self::decimal(f64::from(state.scroll_context.velocity_y)));
            event_fields.insert("context_visible".to_owned(), state.scroll_context.visible_count.to_string());
            event_fields.insert("context_mounted".to_owned(), state.scroll_context.mounted_count.to_string());
            event_fields
                .insert("context_placeholders".to_owned(), state.scroll_context.placeholder_count.to_string());
            event_fields
                .insert("context_offset_y".to_owned(), Self::decimal(f64::from(state.scroll_context.offset_y)));
            Some(event_fields)
        });
        if let Some(event_fields) = event_fields {
            self.emit(&format!("operation.{}", token.name), event_fields, false);
        }
    }

    pub fn update_scroll_context(
        &self,
        phase: &str,
        offset_y: f32,
        velocity_y: f32,
        visible_count: i64,
        mounted_count: i64,
        placeholder_count: i64,
    ) {
        with_state(|state| {
            state.update_scroll_context(phase, offset_y, velocity_y, visible_count, mounted_count, placeholder_count);
        });
    }

    pub fn scroll_began(&self, offset_y: f32) {
        let began = with_state(|state| {
            if !state.is_enabled() {
                return false;
            }
            state.scroll_phase_generation = state.scroll_phase_generation.wrapping_add(1);
            state.scroll_context.phase = "dragging".to_owned();
            state.scroll_context.offset_y = offset_y;
            state.scroll_context.last_scroll_end = None;
            true
        });
        if began {
            self.record(
                "scroll.begin",
                HashMap::from([("offset_y".to_owned(), Self::decimal(f64::from(offset_y)))]),
            );
        }
    }

    pub fn scroll_will_end(&self, velocity_y: f32, target_offset_y: f32) {
        if !self.is_enabled() {
            return;
        }
        self.record(
            "scroll.will_end",
            HashMap::from([
                ("velocity".to_owned(), Self::decimal(f64::from(velocity_y))),
                ("target_offset_y".to_owned(), Self::decimal(f64::from(target_offset_y))),
            ]),
        );
    }

    pub fn scroll_ended(&self, reason: &str, offset_y: f32) {
        let Some((phase_generation, executors)) = with_state(|state| {
            if !state.is_enabled() {
                return None;
            }
            let now = current_media_time();
            state.scroll_phase_generation = state.scroll_phase_generation.wrapping_add(1);
            state.scroll_context.phase = "settling".to_owned();
            state.scroll_context.offset_y = offset_y;
            state.scroll_context.velocity_y = 0.0;
            state.scroll_context.last_scroll_end = Some(now);
            Some((state.scroll_phase_generation, state.executors.clone()))
        }) else {
            return;
        };
        self.record(
            "scroll.end",
            HashMap::from([
                (otp::REASON.to_owned(), reason.to_owned()),
                ("offset_y".to_owned(), Self::decimal(f64::from(offset_y))),
            ]),
        );
        let Some(executors) = executors else {
            return;
        };
        let background = executors.background.clone();
        executors
            .foreground
            .spawn(async move {
                background.timer(SCROLL_SETTLE_DELAY).await;
                ChatPerformanceTrace::shared().scroll_did_settle(phase_generation);
            })
            .detach();
    }

    fn scroll_did_settle(&self, phase_generation: u64) {
        let settled = with_state(|state| {
            if !state.is_enabled()
                || state.scroll_phase_generation != phase_generation
                || state.scroll_context.phase != "settling"
            {
                return false;
            }
            state.scroll_context.phase = "idle".to_owned();
            true
        });
        if settled {
            self.record("scroll.settle_end", HashMap::new());
        }
    }

    pub fn record_height_mismatch(
        &self,
        row_id: &str,
        content_kind: &str,
        planned: f32,
        actual: f32,
        is_visible: bool,
        source_version: i64,
    ) {
        let Some(fields) = with_state(|state| {
            if !state.is_enabled() {
                return None;
            }
            let delta = actual - planned;
            let mut fields = HashMap::from([
                ("row".to_owned(), Self::fingerprint(row_id, state.active_salt())),
                (otp::KIND.to_owned(), content_kind.to_owned()),
                ("planned".to_owned(), Self::decimal(f64::from(planned))),
                ("actual".to_owned(), Self::decimal(f64::from(actual))),
                (events::DELTA.to_owned(), Self::decimal(f64::from(delta))),
                (user::VISIBLE.to_owned(), if is_visible { "1" } else { "0" }.to_owned()),
                ("source_version".to_owned(), source_version.to_string()),
                ("phase".to_owned(), state.scroll_context.phase.clone()),
            ]);
            if let Some(scroll_end) = state.scroll_context.last_scroll_end {
                fields.insert(
                    "since_scroll_end_ms".to_owned(),
                    Self::decimal((current_media_time() - scroll_end) * 1_000.0),
                );
            }
            Some(fields)
        }) else {
            return;
        };
        self.record("height.mismatch", fields);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_geometry_summary(
        &self,
        purpose: &str,
        range: Range<i64>,
        native: i64,
        exact: i64,
        cache_hits: i64,
        hosted_measurements: i64,
        estimates: i64,
        duration_ms: f64,
    ) {
        if !self.is_enabled() {
            return;
        }
        self.record(
            "geometry.summary",
            HashMap::from([
                (otp::PURPOSE.to_owned(), purpose.to_owned()),
                (music::RANGE.to_owned(), format!("{}..<{}", range.start, range.end)),
                ("native".to_owned(), native.to_string()),
                ("exact".to_owned(), exact.to_string()),
                ("cache_hits".to_owned(), cache_hits.to_string()),
                ("hosting_measurements".to_owned(), hosted_measurements.to_string()),
                ("estimates".to_owned(), estimates.to_string()),
                (otp::DURATION_MS.to_owned(), Self::decimal(duration_ms)),
            ]),
        );
    }

    pub fn record_correction_batch(
        &self,
        count: i64,
        total_absolute_delta: f32,
        maximum_delta: f32,
        visible_count: i64,
    ) {
        let Some(fields) = with_state(|state| {
            if !state.is_enabled() {
                return None;
            }
            let mut fields = HashMap::from([
                (map::COUNT.to_owned(), count.to_string()),
                ("visible_count".to_owned(), visible_count.to_string()),
                ("sum_abs_delta".to_owned(), Self::decimal(f64::from(total_absolute_delta))),
                ("max_delta".to_owned(), Self::decimal(f64::from(maximum_delta))),
                ("phase".to_owned(), state.scroll_context.phase.clone()),
            ]);
            if let Some(scroll_end) = state.scroll_context.last_scroll_end {
                fields.insert(
                    "since_scroll_end_ms".to_owned(),
                    Self::decimal((current_media_time() - scroll_end) * 1_000.0),
                );
            }
            Some(fields)
        }) else {
            return;
        };
        self.record("height.correction_batch", fields);
    }

    fn finish_recording(&self, reason: &str) {
        let operations = with_state(|state| {
            state.recording_timeout = None;
            let mut operations: Vec<(String, OperationAggregate)> =
                state.operations.iter().map(|(name, aggregate)| (name.clone(), aggregate.clone())).collect();
            operations.sort_by(|left, right| left.0.cmp(&right.0));
            operations
        });
        for (name, aggregate) in operations {
            let bucket_upper_ms = OperationAggregate::upper_bounds_ms()
                .iter()
                .map(|bound| Self::decimal(*bound))
                .collect::<Vec<_>>()
                .join(",");
            let bucket_counts =
                aggregate.buckets.iter().map(|count| count.to_string()).collect::<Vec<_>>().join(",");
            self.record(
                "operation.summary",
                HashMap::from([
                    (otp::NAME.to_owned(), name),
                    (map::COUNT.to_owned(), aggregate.count.to_string()),
                    ("total_ms".to_owned(), Self::decimal(aggregate.total_ms)),
                    ("max_ms".to_owned(), Self::decimal(aggregate.maximum_ms)),
                    ("p95_upper_ms".to_owned(), Self::decimal(aggregate.percentile_upper_bound(0.95))),
                    ("p99_upper_ms".to_owned(), Self::decimal(aggregate.percentile_upper_bound(0.99))),
                    ("bucket_upper_ms".to_owned(), bucket_upper_ms),
                    ("bucket_counts".to_owned(), bucket_counts),
                    ("sampling".to_owned(), "all_completed_operations".to_owned()),
                ]),
            );
        }
        let counters = with_state(|state| {
            let mut counters: Vec<(&String, &i64)> = state.counters.iter().collect();
            counters.sort_by(|left, right| left.0.cmp(right.0));
            counters.iter().map(|(key, value)| format!("{key}:{value}")).collect::<Vec<_>>().join(",")
        });
        self.record(
            "session.end",
            HashMap::from([
                (otp::REASON.to_owned(), reason.to_owned()),
                ("thermal".to_owned(), thermal_state().name().to_owned()),
                ("memory_mb".to_owned(), resident_memory_mb().to_string()),
                ("counters".to_owned(), counters),
            ]),
        );
        let finished = with_state(|state| {
            state.is_finishing = true;
            state.is_recording = false;
            state.display_link = None;
            state.display_link_generation = state.display_link_generation.wrapping_add(1);
            state.run_loop_observer = None;
            state.application_lifecycle_observers.clear();
            state.is_paused_for_application_lifecycle = false;
            state.frame_aggregate = FrameAggregate::default();
            let url = state.latest_trace_url.clone();
            let relative_time = current_media_time() - state.session_started_at;
            let executors = state.executors.clone()?;
            let finished = state.writer.finish(relative_time, &executors.background);
            Some((executors, finished, url))
        });
        let Some((executors, finished, url)) = finished else {
            self.writer_did_finish(false, None);
            return;
        };
        executors
            .foreground
            .spawn(async move {
                let succeeded = finished.await.unwrap_or(false);
                ChatPerformanceTrace::shared().writer_did_finish(succeeded, url);
            })
            .detach();
    }

    fn writer_did_finish(&self, succeeded: bool, url: Option<PathBuf>) {
        let (completions, completed_url) = with_state(|state| {
            let completed_url = if succeeded { url } else { None };
            if !succeeded {
                state.latest_trace_url = None;
            }
            state.last_activity = "idle".to_owned();
            state.last_activity_at = 0.0;
            state.active_chat_fingerprint = String::new();
            state.session_id = String::new();
            state.fingerprint_salt = String::new();
            state.is_finishing = false;
            (std::mem::take(&mut state.pending_finish_completions), completed_url)
        });
        for completion in completions {
            completion(completed_url.clone());
        }
    }

    pub fn requests_maximum_sampler_refresh() -> bool {
        *REQUESTS_MAXIMUM_SAMPLER_REFRESH
    }

    fn install_frame_sampler(&self, maximum_fps: u32, is_paused: bool, window: &Window) {
        let generation = with_state(|state| {
            state.display_link_generation = state.display_link_generation.wrapping_add(1);
            state.is_paused_for_application_lifecycle = is_paused;
            state.display_link = Some(DisplayLink {
                generation: state.display_link_generation,
                is_paused,
                expected_interval: 1.0 / f64::from(maximum_fps.max(1)),
            });
            state.display_link_generation
        });
        Self::schedule_display_link(window, generation);
    }

    fn schedule_display_link(window: &Window, generation: u64) {
        window.on_next_frame(move |window, _cx| {
            if ChatPerformanceTrace::shared().display_link_did_fire(generation) {
                Self::schedule_display_link(window, generation);
            }
        });
    }

    fn display_link_did_fire(&self, generation: u64) -> bool {
        enum Fire {
            Invalidated,
            Skipped,
            Sampled { hitch: Option<HashMap<String, String>>, summary: Option<HashMap<String, String>> },
        }
        let fire = with_state(|state| {
            let Some(link) = state.display_link.as_ref() else {
                return Fire::Invalidated;
            };
            if link.generation != generation {
                return Fire::Invalidated;
            }
            if !state.is_recording || link.is_paused {
                return Fire::Skipped;
            }
            let link_interval = link.expected_interval;
            let timestamp = current_media_time();
            if state.is_chat_list_session {
                if let Some(scroll_view) = state.chat_list_scroll_view.as_ref() {
                    let offset = -f32::from(scroll_view.scroll_handle.offset().y);
                    let elapsed = state.previous_chat_list_timestamp.map_or(0.0, |previous| timestamp - previous);
                    let velocity = if elapsed > 0.0 {
                        (offset - state.previous_chat_list_offset.unwrap_or(offset)) / elapsed as f32
                    } else {
                        0.0
                    };
                    let phase = if velocity != 0.0 { "dragging" } else { "idle" };
                    state.update_scroll_context(phase, offset, velocity, -1, -1, -1);
                    state.previous_chat_list_offset = Some(offset);
                    state.previous_chat_list_timestamp = Some(timestamp);
                }
            }
            let Some(delta) = Self::frame_delta(state.frame_aggregate.last_timestamp, timestamp) else {
                state.frame_aggregate.reset(timestamp);
                return Fire::Skipped;
            };
            let aggregate = &mut state.frame_aggregate;
            aggregate.last_timestamp = timestamp;
            aggregate.frame_count += 1;
            aggregate.worst_frame = aggregate.worst_frame.max(delta);
            if delta > 0.012 {
                aggregate.frames_over12_ms += 1;
            }
            if delta > 0.020 {
                aggregate.frames_over20_ms += 1;
            }
            if delta > 0.050 {
                aggregate.frames_over50_ms += 1;
            }
            let expected = MINIMUM_FRAME_INTERVAL.max(link_interval);
            let classification = Self::classify_frame(delta, expected);
            aggregate.missed_frames += classification.missed_frames;
            let hitch = classification.is_hitch.then(|| {
                let sampled_at = current_media_time();
                HashMap::from([
                    ("frame_ms".to_owned(), Self::decimal(delta * 1_000.0)),
                    ("expected_ms".to_owned(), Self::decimal(expected * 1_000.0)),
                    ("missed".to_owned(), classification.missed_frames.to_string()),
                    (product_admin::ACTIVITY.to_owned(), state.last_activity.clone()),
                    (
                        "activity_age_ms".to_owned(),
                        Self::decimal((sampled_at - state.last_activity_at).max(0.0) * 1_000.0),
                    ),
                    ("phase".to_owned(), state.scroll_context.phase.clone()),
                    ("velocity".to_owned(), Self::decimal(f64::from(state.scroll_context.velocity_y))),
                    (user::VISIBLE.to_owned(), state.scroll_context.visible_count.to_string()),
                    ("mounted".to_owned(), state.scroll_context.mounted_count.to_string()),
                    ("placeholders".to_owned(), state.scroll_context.placeholder_count.to_string()),
                ])
            });
            let summary = (timestamp - state.frame_aggregate.first_timestamp >= 1.0).then(|| {
                let fields = Self::frame_summary_fields(state, timestamp, expected);
                state.frame_aggregate.reset(timestamp);
                fields
            });
            Fire::Sampled { hitch, summary }
        });
        match fire {
            Fire::Invalidated => false,
            Fire::Skipped => true,
            Fire::Sampled { hitch, summary } => {
                if let Some(fields) = hitch {
                    self.record("frame.hitch", fields);
                }
                if let Some(fields) = summary {
                    self.record("frame.summary", fields);
                }
                true
            }
        }
    }

    pub fn classify_frame(delta: f64, expected: f64) -> FrameClassification {
        if !delta.is_finite() || delta <= 0.0 || !expected.is_finite() || expected <= 0.0 {
            return FrameClassification { missed_frames: 0, is_hitch: false };
        }
        let cadence = MINIMUM_FRAME_INTERVAL.max(expected);
        let interval_ratio = ((i64::MAX - 1) as f64).min((delta / cadence).round().max(1.0));
        let missed_frames = (interval_ratio as i64 - 1).max(0);
        FrameClassification { missed_frames, is_hitch: missed_frames > 0 || delta >= cadence * 1.5 }
    }

    pub fn frame_delta(previous_timestamp: f64, timestamp: f64) -> Option<f64> {
        if !previous_timestamp.is_finite() || previous_timestamp <= 0.0 || !timestamp.is_finite() {
            return None;
        }
        let delta = timestamp - previous_timestamp;
        (delta.is_finite() && delta > 0.0).then_some(delta)
    }

    fn frame_summary_fields(state: &TraceState, timestamp: f64, expected: f64) -> HashMap<String, String> {
        let aggregate = &state.frame_aggregate;
        let duration = expected.max(timestamp - aggregate.first_timestamp);
        let fps = aggregate.frame_count as f64 / duration;
        HashMap::from([
            (file::FPS.to_owned(), Self::decimal(fps)),
            ("cadence_hz".to_owned(), Self::decimal(fps)),
            ("measurement".to_owned(), "display_link_callbacks_not_presented_frames".to_owned()),
            ("worst_ms".to_owned(), Self::decimal(aggregate.worst_frame * 1_000.0)),
            ("frames_gt12ms".to_owned(), aggregate.frames_over12_ms.to_string()),
            ("frames_gt20ms".to_owned(), aggregate.frames_over20_ms.to_string()),
            ("frames_gt50ms".to_owned(), aggregate.frames_over50_ms.to_string()),
            ("missed".to_owned(), aggregate.missed_frames.to_string()),
            ("phase".to_owned(), state.scroll_context.phase.clone()),
            ("velocity".to_owned(), Self::decimal(f64::from(state.scroll_context.velocity_y))),
            ("thermal".to_owned(), thermal_state().name().to_owned()),
            ("memory_mb".to_owned(), resident_memory_mb().to_string()),
        ])
    }

    fn install_run_loop_observer(&self, executors: &TraceExecutors) {
        let background = executors.background.clone();
        let observer = executors.foreground.spawn(async move {
            loop {
                let turn_started_at = current_media_time();
                background.timer(RUN_LOOP_PROBE_INTERVAL).await;
                let due_at = turn_started_at + RUN_LOOP_PROBE_INTERVAL.as_secs_f64();
                if !ChatPerformanceTrace::shared().run_loop_turn_did_end(due_at) {
                    break;
                }
            }
        });
        with_state(|state| state.run_loop_observer = Some(observer));
    }

    fn run_loop_turn_did_end(&self, due_at: f64) -> bool {
        let Some(fields) = with_state(|state| {
            if !state.is_recording {
                return None;
            }
            let now = current_media_time();
            let duration_ms = (now - due_at) * 1_000.0;
            if duration_ms < SLOW_RUN_LOOP_TURN_MS {
                return Some(None);
            }
            Some(Some(HashMap::from([
                (otp::DURATION_MS.to_owned(), Self::decimal(duration_ms)),
                (product_admin::ACTIVITY.to_owned(), state.last_activity.clone()),
                (
                    "activity_age_ms".to_owned(),
                    Self::decimal((now - state.last_activity_at).max(0.0) * 1_000.0),
                ),
                ("phase".to_owned(), state.scroll_context.phase.clone()),
            ])))
        }) else {
            return false;
        };
        if let Some(fields) = fields {
            self.record("main_runloop.slow", fields);
        }
        true
    }

    fn install_application_lifecycle_observers(&self) {
        let center = RiseAppEvents::default_center();
        let observers = vec![
            center.observe(UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION, |_| {
                ChatPerformanceTrace::shared().application_will_resign_active();
            }),
            center.observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, |_| {
                ChatPerformanceTrace::shared().application_did_become_active();
            }),
        ];
        with_state(|state| state.application_lifecycle_observers = observers);
    }

    fn application_will_resign_active(&self) {
        let paused = with_state(|state| state.is_recording && !state.is_paused_for_application_lifecycle);
        if !paused {
            return;
        }
        self.record("lifecycle.inactive", HashMap::new());
        with_state(|state| {
            state.is_paused_for_application_lifecycle = true;
            if let Some(link) = state.display_link.as_mut() {
                link.is_paused = true;
            }
            state.frame_aggregate = FrameAggregate::default();
        });
    }

    fn application_did_become_active(&self) {
        let resumed = with_state(|state| {
            if !state.is_recording || !state.is_paused_for_application_lifecycle {
                return false;
            }
            state.frame_aggregate = FrameAggregate::default();
            state.is_paused_for_application_lifecycle = false;
            if let Some(link) = state.display_link.as_mut() {
                link.is_paused = false;
            }
            true
        });
        if resumed {
            self.record("lifecycle.active", HashMap::new());
        }
    }

    fn attach_chat_list_scroll_view(&self, owner: u64, scroll_handle: ScrollHandle) {
        with_state(|state| state.chat_list_scroll_view = Some(ChatListScrollView { owner, scroll_handle }));
    }

    /// detaches only the attachment owner made
    fn detach_chat_list_scroll_view(&self, owner: u64) {
        let _ = STATE.try_with(|cell| {
            let mut state = cell.borrow_mut();
            if state.chat_list_scroll_view.as_ref().is_some_and(|attached| attached.owner == owner) {
                state.chat_list_scroll_view = None;
            }
        });
    }

    fn make_trace_url(session_id: &str) -> Option<PathBuf> {
        let base = RiseDiagnostics::application_support_directory()?;
        let directory = base.join("Diagnostics").join("ChatPerformance");
        FileManager.create_directory(&directory, true).ok()?;
        let calendar = Calendar { time_zone: TimeZone::current(), ..Calendar::gregorian() };
        let components = calendar.date_components(
            &[
                CalendarComponent::Year,
                CalendarComponent::Month,
                CalendarComponent::Day,
                CalendarComponent::Hour,
                CalendarComponent::Minute,
                CalendarComponent::Second,
            ],
            &RiseClock::now(),
        );
        let stamp = format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            components.year.unwrap_or(0),
            components.month.unwrap_or(0),
            components.day.unwrap_or(0),
            components.hour.unwrap_or(0),
            components.minute.unwrap_or(0),
            components.second.unwrap_or(0),
        );
        Some(directory.join(format!("{TRACE_FILE_PREFIX}{stamp}-{session_id}.jsonl")))
    }

    fn session_metadata(
        session_id: &str,
        chat_fingerprint: &str,
        maximum_fps: u32,
        scale: f32,
    ) -> HashMap<String, String> {
        let build_configuration = if cfg!(debug_assertions) { "debug" } else { "release" };
        let app_info = AppInfo::current(AppEnvironment::current().as_str());
        let build = app_info.as_ref().map_or_else(|| "unknown".to_owned(), |info| info.build.clone());
        let version = app_info.as_ref().map_or_else(|| "unknown".to_owned(), |info| info.short_version.clone());
        let sampler_refresh =
            if Self::requests_maximum_sampler_refresh() { "requests_maximum" } else { "system_default" };
        HashMap::from([
            (music::SESSION.to_owned(), session_id.to_owned()),
            (events::CHAT.to_owned(), chat_fingerprint.to_owned()),
            ("build".to_owned(), build),
            (music::VERSION.to_owned(), version),
            ("configuration".to_owned(), build_configuration.to_owned()),
            ("debugger_attached".to_owned(), if is_debugger_attached() { "1" } else { "0" }.to_owned()),
            (event_collector::DEVICE.to_owned(), DeviceInfo::machine_identifier()),
            (ledger_admin::SYSTEM.to_owned(), DeviceInfo::system_version()),
            ("max_fps".to_owned(), maximum_fps.to_string()),
            ("frame_measurement".to_owned(), "display_link_cadence".to_owned()),
            ("sampler_refresh".to_owned(), sampler_refresh.to_owned()),
            ("operation_samples".to_owned(), "slow_events_plus_all_operation_histograms".to_owned()),
            (post::SCALE.to_owned(), Self::decimal(f64::from(scale))),
            ("low_power".to_owned(), if system_low_power_mode() { "1" } else { "0" }.to_owned()),
            ("thermal".to_owned(), thermal_state().name().to_owned()),
            ("reduce_motion".to_owned(), if reduce_motion() { "1" } else { "0" }.to_owned()),
            ("memory_mb".to_owned(), resident_memory_mb().to_string()),
        ])
    }

    pub fn advances_activity(event: &str) -> bool {
        !matches!(event, "frame.hitch" | "frame.summary" | "main_runloop.slow")
    }

    fn fingerprint(value: &str, salt: &str) -> String {
        let mut hash: u64 = 14_695_981_039_346_656_037;
        for byte in salt.bytes().chain("|".bytes()).chain(value.bytes()) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(1_099_511_628_211);
        }
        format!("{:08x}", hash & 0xffff_ffff)
    }

    fn decimal(value: f64) -> String {
        format!("{value:.2}")
    }
}

struct PendingEvent {
    event: String,
    relative_time: f64,
    fields: HashMap<String, String>,
}

#[derive(Default)]
struct InboxState {
    events: Vec<PendingEvent>,
    is_drain_scheduled: bool,
    accepts_events: bool,
    dropped_events: usize,
}

#[derive(Default)]
struct WriterFile {
    handle: Option<File>,
    buffered_data: Vec<u8>,
    buffered_line_count: usize,
    written_bytes: usize,
    dropped_events: usize,
    file_limit_reached: bool,
    did_encounter_write_failure: bool,
}

enum WriterJob {
    Start { url: PathBuf, metadata: HashMap<String, String> },
    Drain,
    Finish { relative_time: f64, completion: oneshot::Sender<bool> },
}

#[derive(Default)]
struct WriterQueue {
    jobs: VecDeque<WriterJob>,
    is_running: bool,
}

#[derive(Default)]
struct WriterShared {
    inbox: Mutex<InboxState>,
    queue: Mutex<WriterQueue>,
    file: Mutex<WriterFile>,
}

const MAXIMUM_FILE_BYTES: usize = 4 * 1_024 * 1_024;
const MAXIMUM_BUFFERED_LINES: usize = 32;
const MAXIMUM_PENDING_EVENTS: usize = 512;

/// serial trace queue, jobs run one at a time in order on a background thread
struct ChatPerformanceTraceWriter {
    shared: Arc<WriterShared>,
}

impl ChatPerformanceTraceWriter {
    fn new() -> Self {
        Self { shared: Arc::new(WriterShared::default()) }
    }

    fn start(&self, url: PathBuf, metadata: HashMap<String, String>, executor: &BackgroundExecutor) {
        {
            let mut inbox = self.shared.inbox.lock();
            inbox.events.clear();
            inbox.is_drain_scheduled = false;
            inbox.accepts_events = true;
            inbox.dropped_events = 0;
        }
        self.enqueue(WriterJob::Start { url, metadata }, executor);
    }

    fn append(&self, event: &str, relative_time: f64, fields: HashMap<String, String>, executor: &BackgroundExecutor) {
        let pending_event = PendingEvent { event: event.to_owned(), relative_time, fields };
        let should_schedule_drain = {
            let mut inbox = self.shared.inbox.lock();
            if !inbox.accepts_events || inbox.events.len() >= MAXIMUM_PENDING_EVENTS {
                inbox.dropped_events += 1;
                false
            } else {
                inbox.events.push(pending_event);
                !std::mem::replace(&mut inbox.is_drain_scheduled, true)
            }
        };
        if should_schedule_drain {
            self.enqueue(WriterJob::Drain, executor);
        }
    }

    fn finish(&self, relative_time: f64, executor: &BackgroundExecutor) -> oneshot::Receiver<bool> {
        self.shared.inbox.lock().accepts_events = false;
        let (completion, finished) = oneshot::channel();
        self.enqueue(WriterJob::Finish { relative_time, completion }, executor);
        finished
    }

    fn enqueue(&self, job: WriterJob, executor: &BackgroundExecutor) {
        let should_spawn = {
            let mut queue = self.shared.queue.lock();
            queue.jobs.push_back(job);
            !std::mem::replace(&mut queue.is_running, true)
        };
        if should_spawn {
            let shared = Arc::clone(&self.shared);
            executor.spawn(async move { shared.run_jobs() }).detach();
        }
    }
}

impl WriterShared {
    fn run_jobs(&self) {
        loop {
            let job = {
                let mut queue = self.queue.lock();
                match queue.jobs.pop_front() {
                    Some(job) => job,
                    None => {
                        queue.is_running = false;
                        return;
                    }
                }
            };
            match job {
                WriterJob::Start { url, metadata } => self.start_file(&url, &metadata),
                WriterJob::Drain => self.drain_pending_events(),
                WriterJob::Finish { relative_time, completion } => {
                    let succeeded = self.finish_file(relative_time);
                    let _ = completion.send(succeeded);
                }
            }
        }
    }

    fn start_file(&self, url: &Path, metadata: &HashMap<String, String>) {
        if let Some(directory) = url.parent() {
            Self::cleanup_old_traces(directory);
        }
        let mut file = self.file.lock();
        *file = WriterFile::default();
        let handle = FileManager
            .create_file(url, None)
            .then(|| OpenOptions::new().write(true).open(url).ok())
            .flatten();
        let Some(handle) = handle else {
            file.did_encounter_write_failure = true;
            return;
        };
        file.handle = Some(handle);
        Self::append_encoded(&mut file, "metadata", 0.0, metadata, true);
        Self::flush(&mut file);
    }

    fn drain_pending_events(&self) {
        loop {
            let events = {
                let mut inbox = self.inbox.lock();
                if inbox.events.is_empty() {
                    inbox.is_drain_scheduled = false;
                    Vec::new()
                } else {
                    std::mem::take(&mut inbox.events)
                }
            };
            if events.is_empty() {
                return;
            }
            let mut file = self.file.lock();
            for event in events {
                Self::append_encoded(&mut file, &event.event, event.relative_time, &event.fields, true);
            }
        }
    }

    fn finish_file(&self, relative_time: f64) -> bool {
        self.drain_pending_events();
        let inbox_dropped_events = {
            let mut inbox = self.inbox.lock();
            let count = inbox.dropped_events;
            inbox.dropped_events = 0;
            inbox.events.clear();
            inbox.is_drain_scheduled = false;
            count
        };
        let mut file = self.file.lock();
        let total_dropped_events = file.dropped_events + inbox_dropped_events;
        if total_dropped_events > 0 {
            let fields = HashMap::from([(map::COUNT.to_owned(), total_dropped_events.to_string())]);
            Self::append_encoded(&mut file, "writer.dropped", relative_time, &fields, false);
        }
        Self::flush(&mut file);
        let had_handle = file.handle.is_some();
        if let Some(mut handle) = file.handle.take()
            && handle.flush().is_err()
        {
            file.did_encounter_write_failure = true;
        }
        had_handle && !file.did_encounter_write_failure
    }

    fn append_encoded(
        file: &mut WriterFile,
        event: &str,
        relative_time: f64,
        fields: &HashMap<String, String>,
        enforces_file_limit: bool,
    ) {
        if file.handle.is_none() {
            file.did_encounter_write_failure = true;
            return;
        }
        if enforces_file_limit && file.file_limit_reached {
            file.dropped_events += 1;
            return;
        }
        let mut object = Map::new();
        object.insert("t_us".to_owned(), Value::from((relative_time.max(0.0) * 1_000_000.0) as i64));
        object.insert(notify::EVENT.to_owned(), Value::from(event));
        if !fields.is_empty() {
            let fields: Map<String, Value> =
                fields.iter().map(|(key, value)| (key.clone(), Value::from(value.as_str()))).collect();
            object.insert("fields".to_owned(), Value::Object(fields));
        }
        let options = JSONWritingOptions { sorted_keys: true, ..JSONWritingOptions::default() };
        let Some(mut data) = JSONSerialization::data_with_json_object(&Value::Object(object), options) else {
            return;
        };
        data.push(b'\n');
        if enforces_file_limit && file.written_bytes + file.buffered_data.len() + data.len() > MAXIMUM_FILE_BYTES {
            file.file_limit_reached = true;
            file.dropped_events += 1;
            return;
        }
        file.buffered_data.extend_from_slice(&data);
        file.buffered_line_count += 1;
        if file.buffered_line_count >= MAXIMUM_BUFFERED_LINES {
            Self::flush(file);
        }
    }

    fn flush(file: &mut WriterFile) {
        if file.buffered_data.is_empty() {
            return;
        }
        let Some(handle) = file.handle.as_mut() else {
            return;
        };
        match handle.write_all(&file.buffered_data) {
            Ok(()) => {
                file.written_bytes += file.buffered_data.len();
            }
            Err(_) => {
                file.dropped_events += 1;
                file.did_encounter_write_failure = true;
            }
        }
        file.buffered_data.clear();
        file.buffered_line_count = 0;
    }

    fn cleanup_old_traces(directory: &Path) {
        let Ok(files) = FileManager.contents_of_directory_at(directory, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
        else {
            return;
        };
        let mut traces: Vec<(PathBuf, Option<std::time::SystemTime>)> = files
            .into_iter()
            .filter(|path| {
                path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(TRACE_FILE_PREFIX))
            })
            .map(|path| {
                let modified = FileManager.attributes_of_item(&path).ok().and_then(|attributes| attributes.modification_date);
                (path, modified)
            })
            .collect();
        traces.sort_by(|left, right| right.1.cmp(&left.1));
        for (path, _) in traces.into_iter().skip(KEPT_OLD_TRACES) {
            let _ = FileManager.remove_item(&path);
        }
    }
}

static NEXT_PROBE_OWNER: AtomicU64 = AtomicU64::new(1);

/// hands the chat list scroll handle to the opt in trace
/// sampling belongs to the trace display link so the production list gets no timer or observer
pub struct ChatListPerformanceProbe {
    pub is_active: bool,
}

impl ChatListPerformanceProbe {
    pub fn make_coordinator(&self) -> Coordinator {
        Coordinator::new()
    }

    pub fn make_ui_view(&self, scroll_view: &ScrollHandle, coordinator: &mut Coordinator) {
        self.configure(scroll_view, coordinator);
    }

    pub fn update_ui_view(&self, scroll_view: &ScrollHandle, coordinator: &mut Coordinator) {
        self.configure(scroll_view, coordinator);
    }

    fn configure(&self, scroll_view: &ScrollHandle, coordinator: &mut Coordinator) {
        coordinator.is_active = self.is_active;
        coordinator.scroll_view = Some(scroll_view.clone());
        coordinator.update();
    }

    pub fn dismantle_ui_view(coordinator: &mut Coordinator) {
        coordinator.is_active = false;
        coordinator.update();
        coordinator.scroll_view = None;
    }
}

pub struct Coordinator {
    owner: u64,
    scroll_view: Option<ScrollHandle>,
    is_active: bool,
}

impl Coordinator {
    fn new() -> Self {
        Self { owner: NEXT_PROBE_OWNER.fetch_add(1, Ordering::Relaxed), scroll_view: None, is_active: false }
    }

    pub fn update(&self) {
        let trace = ChatPerformanceTrace::shared();
        match (self.is_active, self.scroll_view.as_ref()) {
            (true, Some(scroll_view)) => trace.attach_chat_list_scroll_view(self.owner, scroll_view.clone()),
            (false, Some(_)) => trace.detach_chat_list_scroll_view(self.owner),
            (_, None) => {}
        }
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        if self.scroll_view.is_some() {
            ChatPerformanceTrace::shared().detach_chat_list_scroll_view(self.owner);
        }
    }
}
