use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{App, BackgroundExecutor, ForegroundExecutor, Task, Window};
use parking_lot::Mutex;
use rise_canon::rise_key::{event_collector, file, ledger_admin, map, music, notify, otp, post, product_admin};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication};
use rise_platform::accessibility::reduce_motion;
use rise_platform::app_info::AppInfo;
use rise_platform::crash_diagnostics::resident_memory_mb;
use rise_platform::display_cadence::maximum_frames_per_second;
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileManager};
use rise_platform::power::{POWER_STATE_DID_CHANGE, system_low_power_mode};
use rise_platform::process_info::{THERMAL_STATE_DID_CHANGE, arguments, is_debugger_attached, thermal_state};
use rise_ui::rise_frame_rate::RiseFrameRateRange;
use serde_json::{Map, Value};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::diagnostics::rise_diagnostics::RiseDiagnostics;
use crate::core::lib::device_info::DeviceInfo;
use crate::core::lib::rise_main_thread_sampler::RiseMainThreadSampler;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::{
    ChatPerformanceTrace, FrameClassification as ChatFrameClassification,
};

const SESSION_TIMEOUT_SECONDS: u64 = 120;
const RUN_LOOP_PROBE_INTERVAL: Duration = Duration::from_millis(4);
const MINIMUM_FRAME_INTERVAL: f64 = 1.0 / 240.0;
const TRACE_FILE_PREFIX: &str = "riseonly-profileperf-";
const KEPT_OLD_TRACES: usize = 4;
const THERMAL_STATE_DID_CHANGE_NOTIFICATION: RiseNotificationName =
    RiseNotificationName::new(THERMAL_STATE_DID_CHANGE);
const POWER_STATE_DID_CHANGE_NOTIFICATION: RiseNotificationName = RiseNotificationName::new(POWER_STATE_DID_CHANGE);

pub const PROFILE_PERFORMANCE_TRACE_STATE_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("riseonly.profile-performance-trace.state-did-change");

fn current_media_time() -> f64 {
    RiseClock::monotonic().as_secs_f64()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProfilePerformanceGesturePhase {
    #[default]
    Idle,
    Began,
    Changed,
    Decelerating,
    Settling,
    Ended,
    Cancelled,
    Failed,
}

impl ProfilePerformanceGesturePhase {
    pub const ALL_CASES: [Self; 8] = [
        Self::Idle,
        Self::Began,
        Self::Changed,
        Self::Decelerating,
        Self::Settling,
        Self::Ended,
        Self::Cancelled,
        Self::Failed,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Began => "began",
            Self::Changed => "changed",
            Self::Decelerating => "decelerating",
            Self::Settling => "settling",
            Self::Ended => "ended",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

/// vertical geometry at context update, both sides of the tabs pin and the fence state
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfilePerformanceVerticalGeometry {
    pub collapse_range: f32,
    pub outer_range: f32,
    pub inner_range: f32,
    pub applied_proxy_height: f32,
    pub observed_proxy_height: f32,
    pub laid_out_proxy_height: Option<f32>,
    pub expected_pin: f32,
    pub visual_pin: f32,
    pub visual_min_y: f32,
    pub visual_proxy_height: f32,
    pub visual_pinned_tabs_top: f32,
    pub visual_sequence: u64,
    pub awaiting_proxy_layout: bool,
    pub vertical_motion_active: bool,
    pub deferred_refreshes: i64,
    pub inner_registered: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProfilePerformanceGestureDirection {
    #[default]
    None,
    Up,
    Down,
    Left,
    Right,
}

impl ProfilePerformanceGestureDirection {
    pub const ALL_CASES: [Self; 5] = [Self::None, Self::Up, Self::Down, Self::Left, Self::Right];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

pub type FrameClassification = ChatFrameClassification;

/// opaque to callers, only the trace that issued it reads it back
#[derive(Clone, Debug)]
pub struct OperationToken {
    name: String,
    started_at: f64,
    fields: HashMap<String, String>,
    session_generation: u64,
}

#[derive(Clone, Default)]
struct FrameAggregate {
    first_timestamp: f64,
    last_timestamp: f64,
    frame_count: i64,
    frames_over12_ms: i64,
    frames_over20_ms: i64,
    frames_over50_ms: i64,
    missed_frames: i64,
    suppressed_hitch_samples: i64,
    vertical_updates: i64,
    horizontal_updates: i64,
    worst_frame: f64,
}

impl FrameAggregate {
    fn reset(&mut self, timestamp: f64) {
        *self = Self { first_timestamp: timestamp, last_timestamp: timestamp, ..Self::default() };
    }
}

struct MotionContext {
    vertical_phase: ProfilePerformanceGesturePhase,
    vertical_direction: ProfilePerformanceGestureDirection,
    vertical_velocity: f32,
    outer_offset_y: f32,
    active_inner_offset_y: f32,
    geometry: ProfilePerformanceVerticalGeometry,
    last_pin_mismatch_at: f64,
    horizontal_phase: ProfilePerformanceGesturePhase,
    horizontal_direction: ProfilePerformanceGestureDirection,
    horizontal_velocity: f32,
    horizontal_progress: f32,
    horizontal_target_tab_index: Option<i64>,
    active_tab_index: i64,
}

impl Default for MotionContext {
    fn default() -> Self {
        Self {
            vertical_phase: ProfilePerformanceGesturePhase::Idle,
            vertical_direction: ProfilePerformanceGestureDirection::None,
            vertical_velocity: 0.0,
            outer_offset_y: 0.0,
            active_inner_offset_y: 0.0,
            geometry: ProfilePerformanceVerticalGeometry::default(),
            last_pin_mismatch_at: 0.0,
            horizontal_phase: ProfilePerformanceGesturePhase::Idle,
            horizontal_direction: ProfilePerformanceGestureDirection::None,
            horizontal_velocity: 0.0,
            horizontal_progress: 0.0,
            horizontal_target_tab_index: None,
            active_tab_index: -1,
        }
    }
}

pub type FinishCompletion = Box<dyn FnOnce(Option<PathBuf>)>;

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

/// display link sampler, one per recording session and bound to the window that started it
struct DisplayLink {
    generation: u64,
    is_paused: bool,
    expected_interval: f64,
}

struct TraceState {
    writer: ProfilePerformanceTraceWriter,
    executors: Option<TraceExecutors>,
    is_recording: bool,
    is_finishing: bool,
    latest_trace_url: Option<PathBuf>,
    active_scope_fingerprint: String,
    session_id: String,
    fingerprint_salt: String,
    session_generation: u64,
    session_started_at: f64,
    display_link: Option<DisplayLink>,
    display_link_generation: u64,
    run_loop_observer: Option<Task<()>>,
    stack_sampler: Option<RiseMainThreadSampler>,
    stack_sample_pump: Option<Task<()>>,
    stack_leaf_counters: HashMap<String, i64>,
    frame_aggregate: FrameAggregate,
    motion_context: MotionContext,
    counters: HashMap<String, i64>,
    last_activity: String,
    last_activity_at: f64,
    last_hitch_sample_at: f64,
    recording_timeout: Option<Task<()>>,
    is_paused_for_application_lifecycle: bool,
    lifecycle_observers: Vec<RiseSubscription>,
    pending_finish_completions: Vec<FinishCompletion>,
}

impl TraceState {
    fn new() -> Self {
        Self {
            writer: ProfilePerformanceTraceWriter::new(),
            executors: None,
            is_recording: false,
            is_finishing: false,
            latest_trace_url: None,
            active_scope_fingerprint: String::new(),
            session_id: String::new(),
            fingerprint_salt: String::new(),
            session_generation: 0,
            session_started_at: 0.0,
            display_link: None,
            display_link_generation: 0,
            run_loop_observer: None,
            stack_sampler: None,
            stack_sample_pump: None,
            stack_leaf_counters: HashMap::new(),
            frame_aggregate: FrameAggregate::default(),
            motion_context: MotionContext::default(),
            counters: HashMap::new(),
            last_activity: "idle".to_owned(),
            last_activity_at: 0.0,
            last_hitch_sample_at: 0.0,
            recording_timeout: None,
            is_paused_for_application_lifecycle: false,
            lifecycle_observers: Vec::new(),
            pending_finish_completions: Vec::new(),
        }
    }

    fn matches_scope(&self, scope: &str) -> bool {
        self.active_scope_fingerprint == ProfilePerformanceTrace::fingerprint(scope, &self.fingerprint_salt)
    }

    fn vertical_fields(&self) -> HashMap<String, String> {
        let motion = &self.motion_context;
        let mut fields = HashMap::from([
            ("phase".to_owned(), motion.vertical_phase.raw_value().to_owned()),
            ("direction".to_owned(), motion.vertical_direction.raw_value().to_owned()),
            ("velocity_y".to_owned(), decimal(f64::from(motion.vertical_velocity))),
            ("outer_offset_y".to_owned(), decimal(f64::from(motion.outer_offset_y))),
            ("inner_offset_y".to_owned(), decimal(f64::from(motion.active_inner_offset_y))),
            ("tab_index".to_owned(), motion.active_tab_index.to_string()),
        ]);
        fields.extend(self.geometry_fields());
        fields
    }

    fn geometry_fields(&self) -> HashMap<String, String> {
        let geometry = &self.motion_context.geometry;
        HashMap::from([
            ("collapse_range".to_owned(), decimal(f64::from(geometry.collapse_range))),
            ("outer_range".to_owned(), decimal(f64::from(geometry.outer_range))),
            ("inner_range".to_owned(), decimal(f64::from(geometry.inner_range))),
            ("applied_proxy".to_owned(), decimal(f64::from(geometry.applied_proxy_height))),
            ("observed_proxy".to_owned(), decimal(f64::from(geometry.observed_proxy_height))),
            (
                "laid_out_proxy".to_owned(),
                geometry
                    .laid_out_proxy_height
                    .map_or_else(|| "unknown".to_owned(), |height| decimal(f64::from(height))),
            ),
            ("expected_pin".to_owned(), decimal(f64::from(geometry.expected_pin))),
            ("visual_pin".to_owned(), decimal(f64::from(geometry.visual_pin))),
            ("visual_min_y".to_owned(), decimal(f64::from(geometry.visual_min_y))),
            ("visual_proxy".to_owned(), decimal(f64::from(geometry.visual_proxy_height))),
            ("visual_pinned_top".to_owned(), decimal(f64::from(geometry.visual_pinned_tabs_top))),
            ("visual_seq".to_owned(), geometry.visual_sequence.to_string()),
            ("awaiting_proxy_layout".to_owned(), flag(geometry.awaiting_proxy_layout)),
            ("motion_flag".to_owned(), flag(geometry.vertical_motion_active)),
            ("deferred_refreshes".to_owned(), geometry.deferred_refreshes.to_string()),
            ("inner_registered".to_owned(), flag(geometry.inner_registered)),
        ])
    }

    fn horizontal_fields(&self) -> HashMap<String, String> {
        let motion = &self.motion_context;
        let mut fields = HashMap::from([
            ("phase".to_owned(), motion.horizontal_phase.raw_value().to_owned()),
            ("direction".to_owned(), motion.horizontal_direction.raw_value().to_owned()),
            ("velocity_x".to_owned(), decimal(f64::from(motion.horizontal_velocity))),
            ("progress".to_owned(), decimal(f64::from(motion.horizontal_progress))),
            ("tab_index".to_owned(), motion.active_tab_index.to_string()),
        ]);
        if let Some(horizontal_target_tab_index) = motion.horizontal_target_tab_index {
            fields.insert("target_tab_index".to_owned(), horizontal_target_tab_index.to_string());
        }
        fields
    }

    fn context_fields(&self) -> HashMap<String, String> {
        let motion = &self.motion_context;
        let is_settling = motion.horizontal_phase == ProfilePerformanceGesturePhase::Settling;
        let mut fields = HashMap::from([
            ("vertical_phase".to_owned(), motion.vertical_phase.raw_value().to_owned()),
            ("vertical_direction".to_owned(), motion.vertical_direction.raw_value().to_owned()),
            ("velocity_y".to_owned(), decimal(f64::from(motion.vertical_velocity))),
            ("outer_offset_y".to_owned(), decimal(f64::from(motion.outer_offset_y))),
            ("inner_offset_y".to_owned(), decimal(f64::from(motion.active_inner_offset_y))),
            ("horizontal_phase".to_owned(), motion.horizontal_phase.raw_value().to_owned()),
            ("horizontal_direction".to_owned(), motion.horizontal_direction.raw_value().to_owned()),
            ("velocity_x".to_owned(), decimal(f64::from(motion.horizontal_velocity))),
            ("horizontal_progress".to_owned(), decimal(f64::from(motion.horizontal_progress))),
            ("horizontal_settling".to_owned(), flag(is_settling)),
            ("tab_index".to_owned(), motion.active_tab_index.to_string()),
        ]);
        fields.extend(self.geometry_fields());
        if let Some(horizontal_target_tab_index) = motion.horizontal_target_tab_index {
            fields.insert("target_tab_index".to_owned(), horizontal_target_tab_index.to_string());
        }
        fields
    }

    fn frame_summary_fields(&self, timestamp: f64, expected: f64) -> HashMap<String, String> {
        let aggregate = &self.frame_aggregate;
        let duration = expected.max(timestamp - aggregate.first_timestamp);
        let fps = aggregate.frame_count as f64 / duration;
        let mut fields = self.context_fields();
        fields.insert(file::FPS.to_owned(), decimal(fps));
        fields.insert("worst_ms".to_owned(), decimal(aggregate.worst_frame * 1_000.0));
        fields.insert("frames_gt12ms".to_owned(), aggregate.frames_over12_ms.to_string());
        fields.insert("frames_gt20ms".to_owned(), aggregate.frames_over20_ms.to_string());
        fields.insert("frames_gt50ms".to_owned(), aggregate.frames_over50_ms.to_string());
        fields.insert("missed".to_owned(), aggregate.missed_frames.to_string());
        fields.insert("hitch_samples_suppressed".to_owned(), aggregate.suppressed_hitch_samples.to_string());
        fields.insert("vertical_updates".to_owned(), aggregate.vertical_updates.to_string());
        fields.insert("horizontal_updates".to_owned(), aggregate.horizontal_updates.to_string());
        fields.insert("thermal".to_owned(), thermal_state().name().to_owned());
        fields.insert("memory_mb".to_owned(), resident_memory_mb().to_string());
        fields
    }

    fn stack_summary(&self) -> String {
        let mut leaves: Vec<(&String, &i64)> = self.stack_leaf_counters.iter().collect();
        leaves.sort_by(|left, right| right.1.cmp(left.1));
        leaves.iter().take(12).map(|(key, value)| format!("{key}:{value}")).collect::<Vec<_>>().join(",")
    }
}

thread_local! {
    static STATE: RefCell<TraceState> = RefCell::new(TraceState::new());
}

fn with_state<R>(body: impl FnOnce(&mut TraceState) -> R) -> R {
    STATE.with_borrow_mut(body)
}

fn post_state_did_change() {
    RiseAppEvents::default_center().post(PROFILE_PERFORMANCE_TRACE_STATE_DID_CHANGE);
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn decimal(value: f64) -> String {
    format!("{value:.2}")
}

fn flag(value: bool) -> String {
    if value { "1" } else { "0" }.to_owned()
}

static SHARED: ProfilePerformanceTrace = ProfilePerformanceTrace { _private: () };

static IS_CONTROL_AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
    ProfilePerformanceTrace::control_availability(
        cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools"),
        &arguments(),
    )
});

/// handle to the main thread trace
pub struct ProfilePerformanceTrace {
    _private: (),
}

impl ProfilePerformanceTrace {
    pub const LAUNCH_ARGUMENT: &'static str = "-riseProfilePerformanceTools";
    pub const RUN_LOOP_SLOW_THRESHOLD_MS: f64 = 12.0;
    pub const HITCH_SAMPLE_INTERVAL: f64 = 0.20;

    pub fn shared() -> &'static ProfilePerformanceTrace {
        &SHARED
    }

    pub fn is_control_available() -> bool {
        *IS_CONTROL_AVAILABLE
    }

    pub fn is_recording_any_scope(&self) -> bool {
        with_state(|state| state.is_recording)
    }

    pub fn is_finishing(&self) -> bool {
        with_state(|state| state.is_finishing)
    }

    pub fn latest_trace_url(&self) -> Option<PathBuf> {
        with_state(|state| state.latest_trace_url.clone())
    }

    pub fn control_availability(is_debug_or_tools_build: bool, launch_arguments: &[String]) -> bool {
        is_debug_or_tools_build || launch_arguments.iter().any(|argument| argument == Self::LAUNCH_ARGUMENT)
    }

    pub fn sampler_frame_rate_range(maximum_frames_per_second: u32) -> RiseFrameRateRange {
        let maximum = maximum_frames_per_second.max(30) as f32;
        RiseFrameRateRange { minimum: maximum.min(30.0), maximum, preferred: maximum }
    }

    pub fn classify_frame(delta: f64, expected: f64) -> FrameClassification {
        ChatPerformanceTrace::classify_frame(delta, expected)
    }

    pub fn frame_delta(previous_timestamp: f64, timestamp: f64) -> Option<f64> {
        ChatPerformanceTrace::frame_delta(previous_timestamp, timestamp)
    }

    pub fn start(&self, scope: &str, window: &mut Window, cx: &mut App) -> bool {
        let refused = with_state(|state| state.is_recording || state.is_finishing);
        if !Self::is_control_available() || refused || scope.is_empty() {
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
            state.active_scope_fingerprint = Self::fingerprint(scope, &state.fingerprint_salt);
            state.is_recording = true;
            state.stack_leaf_counters.clear();
            state.latest_trace_url = Some(url.clone());
            state.session_started_at = current_media_time();
            state.motion_context = MotionContext::default();
            state.counters.clear();
            state.last_activity = "recording.start".to_owned();
            state.last_activity_at = state.session_started_at;
            state.last_hitch_sample_at = 0.0;
            state.frame_aggregate = FrameAggregate::default();
            Self::session_metadata(&state.session_id, &state.active_scope_fingerprint, maximum_fps, scale)
        });

        with_state(|state| state.writer.start(url, metadata, &executors.background));
        self.install_frame_sampler(maximum_fps, is_paused, window);
        self.install_run_loop_observer(&executors);
        self.install_lifecycle_observers();
        self.record("session.start", HashMap::new());
        post_state_did_change();

        let background = executors.background.clone();
        let timeout = executors.foreground.spawn(async move {
            background.timer(Duration::from_secs(SESSION_TIMEOUT_SECONDS)).await;
            ProfilePerformanceTrace::shared().recording_did_time_out();
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

    pub fn stop(&self, reason: &str, completion: Option<FinishCompletion>) {
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

    pub fn stop_if_recording(&self, scope: &str, reason: &str, completion: Option<FinishCompletion>) -> bool {
        let matches = with_state(|state| {
            let matches = state.is_recording && !state.is_finishing && !scope.is_empty() && state.matches_scope(scope);
            if matches && let Some(completion) = completion {
                state.pending_finish_completions.push(completion);
            }
            matches
        });
        if !matches {
            return false;
        }
        self.finish_recording(reason);
        true
    }

    pub fn is_recording(&self, scope: &str) -> bool {
        with_state(|state| state.is_recording && !scope.is_empty() && state.matches_scope(scope))
    }

    pub fn record(&self, event: &str, fields: HashMap<String, String>) {
        if !self.is_recording_any_scope() {
            return;
        }
        self.emit(event, fields, true);
    }

    /// records an event, fields are built lazily so hot paths cost nothing when no trace runs
    pub fn record_with(&self, event: &str, fields: impl FnOnce() -> HashMap<String, String>) {
        if !self.is_recording_any_scope() {
            return;
        }
        self.emit(event, fields(), true);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_vertical_context(
        &self,
        phase: ProfilePerformanceGesturePhase,
        direction: ProfilePerformanceGestureDirection,
        velocity_y: f32,
        outer_offset_y: f32,
        active_inner_offset_y: f32,
        active_tab_index: i64,
        geometry: ProfilePerformanceVerticalGeometry,
    ) {
        let Some((pin_mismatch, phase_fields)) = with_state(|state| {
            if !state.is_recording {
                return None;
            }
            let previous_phase = state.motion_context.vertical_phase;
            let motion = &mut state.motion_context;
            motion.vertical_phase = phase;
            motion.vertical_direction = direction;
            motion.vertical_velocity = finite(velocity_y);
            motion.outer_offset_y = finite(outer_offset_y);
            motion.active_inner_offset_y = finite(active_inner_offset_y);
            motion.active_tab_index = active_tab_index;
            motion.geometry = geometry;
            state.frame_aggregate.vertical_updates += 1;

            let pin_mismatch = Self::pin_mismatch_fields_if_needed(state);
            let phase_fields = (previous_phase != phase).then(|| state.vertical_fields());
            Some((pin_mismatch, phase_fields))
        }) else {
            return;
        };
        if let Some(fields) = pin_mismatch {
            self.record("pin.mismatch", fields);
        }
        if let Some(fields) = phase_fields {
            self.record("vertical.phase", fields);
        }
    }

    fn pin_mismatch_fields_if_needed(state: &mut TraceState) -> Option<HashMap<String, String>> {
        let geometry = &state.motion_context.geometry;
        if geometry.visual_sequence == 0 {
            return None;
        }
        let delta = geometry.visual_pin - geometry.expected_pin;
        if delta.abs() <= 2.0 {
            return None;
        }
        let now = current_media_time();
        if now - state.motion_context.last_pin_mismatch_at < 0.25 {
            return None;
        }
        state.motion_context.last_pin_mismatch_at = now;
        let mut fields = state.vertical_fields();
        fields.insert("delta".to_owned(), decimal(f64::from(delta)));
        Some(fields)
    }

    pub fn update_horizontal_context(
        &self,
        phase: ProfilePerformanceGesturePhase,
        direction: ProfilePerformanceGestureDirection,
        velocity_x: f32,
        progress: f32,
        active_tab_index: i64,
        target_tab_index: Option<i64>,
    ) {
        let phase_fields = with_state(|state| {
            if !state.is_recording {
                return None;
            }
            let previous_phase = state.motion_context.horizontal_phase;
            let motion = &mut state.motion_context;
            motion.horizontal_phase = phase;
            motion.horizontal_direction = direction;
            motion.horizontal_velocity = finite(velocity_x);
            motion.horizontal_progress = finite(progress);
            motion.horizontal_target_tab_index = target_tab_index;
            motion.active_tab_index = active_tab_index;
            state.frame_aggregate.horizontal_updates += 1;
            (previous_phase != phase).then(|| state.horizontal_fields())
        });
        if let Some(fields) = phase_fields {
            self.record("horizontal.phase", fields);
        }
    }

    pub fn record_horizontal_settle_began(
        &self,
        from_tab_index: i64,
        target_tab_index: i64,
        progress: f32,
        velocity_x: f32,
    ) {
        let fields = with_state(|state| {
            if !state.is_recording {
                return None;
            }
            let motion = &mut state.motion_context;
            motion.horizontal_phase = ProfilePerformanceGesturePhase::Settling;
            motion.horizontal_progress = finite(progress);
            motion.horizontal_velocity = finite(velocity_x);
            motion.horizontal_target_tab_index = Some(target_tab_index);
            motion.active_tab_index = from_tab_index;
            Some(state.horizontal_fields())
        });
        if let Some(fields) = fields {
            self.record("horizontal.settle_begin", fields);
        }
    }

    pub fn record_horizontal_settle_ended(&self, selected_tab_index: i64, was_cancelled: bool) {
        let fields = with_state(|state| {
            if !state.is_recording {
                return None;
            }
            let motion = &mut state.motion_context;
            motion.horizontal_phase = if was_cancelled {
                ProfilePerformanceGesturePhase::Cancelled
            } else {
                ProfilePerformanceGesturePhase::Ended
            };
            motion.horizontal_velocity = 0.0;
            motion.horizontal_progress = 0.0;
            motion.horizontal_target_tab_index = None;
            motion.active_tab_index = selected_tab_index;
            let mut fields = state.horizontal_fields();
            fields.insert("cancelled".to_owned(), flag(was_cancelled));
            state.motion_context.horizontal_phase = ProfilePerformanceGesturePhase::Idle;
            state.motion_context.horizontal_direction = ProfilePerformanceGestureDirection::None;
            Some(fields)
        });
        if let Some(fields) = fields {
            self.record("horizontal.settle_end", fields);
        }
    }

    pub fn begin_operation(
        &self,
        name: &str,
        fields: impl FnOnce() -> HashMap<String, String>,
    ) -> Option<OperationToken> {
        let (started_at, session_generation) = with_state(|state| {
            if !state.is_recording {
                return None;
            }
            let started_at = current_media_time();
            state.last_activity = name.to_owned();
            state.last_activity_at = started_at;
            Some((started_at, state.session_generation))
        })?;
        Some(OperationToken { name: name.to_owned(), started_at, fields: fields(), session_generation })
    }

    pub fn end_operation(
        &self,
        token: Option<OperationToken>,
        threshold_ms: f64,
        additional_fields: impl FnOnce() -> HashMap<String, String>,
    ) {
        let Some(token) = token else {
            return;
        };
        let duration_ms = (current_media_time() - token.started_at) * 1_000.0;
        let event = format!("operation.{}", token.name);
        let context = with_state(|state| {
            if !state.is_recording || token.session_generation != state.session_generation {
                return None;
            }
            *state.counters.entry(event.clone()).or_insert(0) += 1;
            (duration_ms >= threshold_ms).then(|| state.context_fields())
        });
        let Some(context) = context else {
            return;
        };
        let mut fields = token.fields;
        fields.extend(additional_fields());
        fields.insert(otp::DURATION_MS.to_owned(), decimal(duration_ms));
        fields.extend(context);
        self.emit(&event, fields, false);
    }

    fn emit(&self, event: &str, fields: HashMap<String, String>, increments_counter: bool) {
        with_state(|state| {
            if !state.is_recording {
                return;
            }
            let now = current_media_time();
            if Self::advances_activity(event) {
                state.last_activity = event.to_owned();
                state.last_activity_at = now;
            }
            if increments_counter {
                *state.counters.entry(event.to_owned()).or_insert(0) += 1;
            }
            tracing::trace!(target: "ProfilePerformance", "{event}");
            if let Some(executors) = state.executors.as_ref() {
                state.writer.append(event, now - state.session_started_at, fields, &executors.background);
            }
        });
    }

    fn finish_recording(&self, reason: &str) {
        let (counters, hot_frames) = with_state(|state| {
            state.recording_timeout = None;
            let mut counters: Vec<(&String, &i64)> = state.counters.iter().collect();
            counters.sort_by(|left, right| left.0.cmp(right.0));
            let counters = counters.iter().map(|(key, value)| format!("{key}:{value}")).collect::<Vec<_>>().join(",");
            (counters, state.stack_summary())
        });
        self.record(
            "session.end",
            HashMap::from([
                (otp::REASON.to_owned(), reason.to_owned()),
                ("thermal".to_owned(), thermal_state().name().to_owned()),
                ("memory_mb".to_owned(), resident_memory_mb().to_string()),
                ("counters".to_owned(), counters),
                ("hot_frames".to_owned(), hot_frames),
            ]),
        );
        with_state(|state| {
            state.is_finishing = true;
            state.is_recording = false;
        });
        post_state_did_change();
        let finished = with_state(|state| {
            state.display_link = None;
            state.display_link_generation = state.display_link_generation.wrapping_add(1);
            state.run_loop_observer = None;
            if let Some(sampler) = state.stack_sampler.take() {
                sampler.stop();
            }
            state.stack_sample_pump = None;
            state.lifecycle_observers.clear();
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
                ProfilePerformanceTrace::shared().writer_did_finish(succeeded, url);
            })
            .detach();
    }

    fn writer_did_finish(&self, succeeded: bool, url: Option<PathBuf>) {
        let (completions, completed_url) = with_state(|state| {
            let completed_url = if succeeded { url } else { None };
            if !succeeded {
                state.latest_trace_url = None;
            }
            state.active_scope_fingerprint = String::new();
            state.session_id = String::new();
            state.fingerprint_salt = String::new();
            state.last_activity = "idle".to_owned();
            state.last_activity_at = 0.0;
            state.is_finishing = false;
            (std::mem::take(&mut state.pending_finish_completions), completed_url)
        });
        post_state_did_change();
        for completion in completions {
            completion(completed_url.clone());
        }
    }

    fn install_frame_sampler(&self, maximum_fps: u32, is_paused: bool, window: &Window) {
        let range = Self::sampler_frame_rate_range(maximum_fps);
        let generation = with_state(|state| {
            state.display_link_generation = state.display_link_generation.wrapping_add(1);
            state.is_paused_for_application_lifecycle = is_paused;
            state.display_link = Some(DisplayLink {
                generation: state.display_link_generation,
                is_paused,
                expected_interval: 1.0 / f64::from(range.preferred),
            });
            state.display_link_generation
        });
        Self::schedule_display_link(window, generation);
    }

    fn schedule_display_link(window: &Window, generation: u64) {
        window.on_next_frame(move |window, _cx| {
            if ProfilePerformanceTrace::shared().display_link_did_fire(generation) {
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
            let mut hitch = None;
            if classification.is_hitch {
                let sampled_at = current_media_time();
                if state.last_hitch_sample_at == 0.0
                    || sampled_at - state.last_hitch_sample_at >= Self::HITCH_SAMPLE_INTERVAL
                {
                    state.last_hitch_sample_at = sampled_at;
                    let mut fields = state.context_fields();
                    fields.insert("frame_ms".to_owned(), decimal(delta * 1_000.0));
                    fields.insert("expected_ms".to_owned(), decimal(expected * 1_000.0));
                    fields.insert("missed".to_owned(), classification.missed_frames.to_string());
                    fields.insert(product_admin::ACTIVITY.to_owned(), state.last_activity.clone());
                    fields.insert(
                        "activity_age_ms".to_owned(),
                        decimal((sampled_at - state.last_activity_at).max(0.0) * 1_000.0),
                    );
                    hitch = Some(fields);
                } else {
                    state.frame_aggregate.suppressed_hitch_samples += 1;
                }
            }
            let summary = (timestamp - state.frame_aggregate.first_timestamp >= 1.0).then(|| {
                let fields = state.frame_summary_fields(timestamp, expected);
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

    fn install_run_loop_observer(&self, executors: &TraceExecutors) {
        let background = executors.background.clone();
        let observer = executors.foreground.spawn(async move {
            loop {
                let turn_started_at = current_media_time();
                background.timer(RUN_LOOP_PROBE_INTERVAL).await;
                let due_at = turn_started_at + RUN_LOOP_PROBE_INTERVAL.as_secs_f64();
                if !ProfilePerformanceTrace::shared().run_loop_turn_did_end(due_at) {
                    break;
                }
            }
        });
        with_state(|state| state.run_loop_observer = Some(observer));
        self.install_stack_sampler(executors);
    }

    fn run_loop_turn_did_end(&self, due_at: f64) -> bool {
        let Some(fields) = with_state(|state| {
            if !state.is_recording {
                return None;
            }
            let now = current_media_time();
            if let Some(sampler) = state.stack_sampler.as_ref() {
                sampler.note_turn_ended();
                sampler.note_turn_began(now);
            }
            let duration_ms = (now - due_at) * 1_000.0;
            if duration_ms < Self::RUN_LOOP_SLOW_THRESHOLD_MS {
                return Some(None);
            }
            let mut fields = state.context_fields();
            fields.insert(otp::DURATION_MS.to_owned(), decimal(duration_ms));
            fields.insert(product_admin::ACTIVITY.to_owned(), state.last_activity.clone());
            fields.insert(
                "activity_age_ms".to_owned(),
                decimal((now - state.last_activity_at).max(0.0) * 1_000.0),
            );
            Some(Some(fields))
        }) else {
            return false;
        };
        if let Some(fields) = fields {
            self.record("main_runloop.slow", fields);
        }
        true
    }

    fn install_stack_sampler(&self, executors: &TraceExecutors) {
        let already_installed = with_state(|state| state.stack_sampler.is_some());
        if !RiseMainThreadSampler::is_supported() || already_installed {
            return;
        }
        let (sender, mut inbox) = mpsc::unbounded::<(Vec<String>, f64)>();
        let sampler = RiseMainThreadSampler::new(move |symbols, duration_ms| {
            let _ = sender.unbounded_send((symbols, duration_ms));
        });
        let pump = executors.foreground.spawn(async move {
            while let Some((symbols, busy_for_ms)) = inbox.next().await {
                ProfilePerformanceTrace::shared().record_stack_sample(&symbols, busy_for_ms);
            }
        });
        sampler.start();
        with_state(|state| {
            state.stack_sampler = Some(sampler);
            state.stack_sample_pump = Some(pump);
        });
    }

    fn record_stack_sample(&self, symbols: &[String], busy_for_ms: f64) {
        let leaf = symbols
            .iter()
            .find(|symbol| !symbol.starts_with("libsystem_"))
            .map_or_else(|| "?".to_owned(), Clone::clone);
        let is_recording = with_state(|state| {
            if !state.is_recording {
                return false;
            }
            *state.stack_leaf_counters.entry(leaf.clone()).or_insert(0) += 1;
            true
        });
        if !is_recording {
            return;
        }
        let frames = symbols.iter().take(28).map(String::as_str).collect::<Vec<_>>().join(" < ");
        self.emit(
            "main_thread.stack",
            HashMap::from([
                ("busy_ms".to_owned(), decimal(busy_for_ms)),
                ("leaf".to_owned(), leaf),
                ("frames".to_owned(), frames),
            ]),
            false,
        );
    }

    fn install_lifecycle_observers(&self) {
        let center = RiseAppEvents::default_center();
        let observers = vec![
            center.observe(UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION, |_| {
                ProfilePerformanceTrace::shared().application_will_resign_active();
            }),
            center.observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, |_| {
                ProfilePerformanceTrace::shared().application_did_become_active();
            }),
            center.observe(UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION, |_| {
                ProfilePerformanceTrace::shared().application_did_receive_memory_warning();
            }),
            center.observe(THERMAL_STATE_DID_CHANGE_NOTIFICATION, |_| {
                ProfilePerformanceTrace::shared().thermal_state_did_change();
            }),
            center.observe(POWER_STATE_DID_CHANGE_NOTIFICATION, |_| {
                ProfilePerformanceTrace::shared().power_state_did_change();
            }),
        ];
        with_state(|state| state.lifecycle_observers = observers);
    }

    fn application_will_resign_active(&self) {
        let pauses = with_state(|state| state.is_recording && !state.is_paused_for_application_lifecycle);
        if !pauses {
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
        let resumes = with_state(|state| {
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
        if resumes {
            self.record("lifecycle.active", HashMap::new());
        }
    }

    fn application_did_receive_memory_warning(&self) {
        self.record_with("memory.warning", || {
            HashMap::from([("memory_mb".to_owned(), resident_memory_mb().to_string())])
        });
    }

    fn thermal_state_did_change(&self) {
        self.record_with("thermal.changed", || {
            HashMap::from([("thermal".to_owned(), thermal_state().name().to_owned())])
        });
    }

    fn power_state_did_change(&self) {
        self.record_with("power.changed", || HashMap::from([("low_power".to_owned(), flag(system_low_power_mode()))]));
    }

    fn make_trace_url(session_id: &str) -> Option<PathBuf> {
        let base = RiseDiagnostics::application_support_directory()?;
        let directory = base.join("Diagnostics").join("ProfilePerformance");
        let timestamp = RiseClock::now().time_interval_since_1970() as i64;
        Some(directory.join(format!("{TRACE_FILE_PREFIX}{timestamp}-{session_id}.jsonl")))
    }

    fn session_metadata(
        session_id: &str,
        scope_fingerprint: &str,
        maximum_fps: u32,
        scale: f32,
    ) -> HashMap<String, String> {
        let build_configuration = if cfg!(debug_assertions) { "debug" } else { "release" };
        let app_info = AppInfo::current(AppEnvironment::current().as_str());
        let build = app_info.as_ref().map_or_else(|| "unknown".to_owned(), |info| info.build.clone());
        let version = app_info.as_ref().map_or_else(|| "unknown".to_owned(), |info| info.short_version.clone());
        HashMap::from([
            (music::SESSION.to_owned(), session_id.to_owned()),
            ("profile".to_owned(), scope_fingerprint.to_owned()),
            ("build".to_owned(), build),
            (music::VERSION.to_owned(), version),
            ("configuration".to_owned(), build_configuration.to_owned()),
            ("debugger_attached".to_owned(), flag(is_debugger_attached())),
            (event_collector::DEVICE.to_owned(), DeviceInfo::machine_identifier()),
            (ledger_admin::SYSTEM.to_owned(), DeviceInfo::system_version()),
            ("max_fps".to_owned(), maximum_fps.to_string()),
            ("sampler_preferred_fps".to_owned(), maximum_fps.to_string()),
            ("runloop_slow_threshold_ms".to_owned(), decimal(Self::RUN_LOOP_SLOW_THRESHOLD_MS)),
            (post::SCALE.to_owned(), decimal(f64::from(scale))),
            ("low_power".to_owned(), flag(system_low_power_mode())),
            ("thermal".to_owned(), thermal_state().name().to_owned()),
            ("reduce_motion".to_owned(), flag(reduce_motion())),
            ("memory_mb".to_owned(), resident_memory_mb().to_string()),
        ])
    }

    fn advances_activity(event: &str) -> bool {
        ChatPerformanceTrace::advances_activity(event)
    }

    fn fingerprint(value: &str, salt: &str) -> String {
        let mut hash: u64 = 14_695_981_039_346_656_037;
        for byte in salt.bytes().chain("|".bytes()).chain(value.bytes()) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(1_099_511_628_211);
        }
        format!("{:08x}", hash & 0xffff_ffff)
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

/// serial writer queue, jobs run one at a time in order on a background thread
struct ProfilePerformanceTraceWriter {
    shared: Arc<WriterShared>,
}

impl ProfilePerformanceTraceWriter {
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
        let mut file = self.file.lock();
        *file = WriterFile::default();
        let Some(directory) = url.parent() else {
            file.did_encounter_write_failure = true;
            return;
        };
        if FileManager.create_directory(directory, true).is_err() {
            file.did_encounter_write_failure = true;
            return;
        }
        Self::cleanup_old_traces(directory);
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
