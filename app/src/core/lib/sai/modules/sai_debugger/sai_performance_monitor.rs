#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{App, BackgroundExecutor, Context, Entity, Task, WeakEntity, Window, WindowId};
use parking_lot::Mutex;
use rise_canon::rise_key::{event_collector, ledger_admin, map, music, notify, post};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, TimeZone};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_platform::accessibility::reduce_motion;
use rise_platform::app_info::AppInfo;
use rise_platform::crash_diagnostics::resident_memory_mb;
use rise_platform::display_cadence::{FALLBACK_FRAMES_PER_SECOND, maximum_frames_per_second};
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileManager};
use rise_platform::power::system_low_power_mode;
use rise_platform::process_info::{is_debugger_attached, process_cpu_seconds, thermal_reading};
use serde_json::{Map, Value};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::diagnostics::rise_diagnostics::RiseDiagnostics;
use crate::core::lib::device_info::DeviceInfo;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiNavigationBridge;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;

const MINIMUM_FRAME_INTERVAL: f64 = 1.0 / 240.0;
const RUN_LOOP_PROBE_INTERVAL: Duration = Duration::from_millis(4);
const TRACE_FILE_PREFIX: &str = "riseonly-perf-";
const KEPT_TRACES: usize = 8;
const MAXIMUM_PENDING_EVENTS: usize = 512;
const MAXIMUM_BUFFERED_LINES: usize = 32;
const MAXIMUM_FILE_BYTES: usize = 4 * 1_024 * 1_024;

fn current_media_time() -> f64 {
    RiseClock::monotonic().as_secs_f64()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaiPerformanceFrameClassification {
    pub missed_frames: i64,
    pub is_hitch: bool,
}

pub enum SaiPerformanceSamplingPolicy {}

impl SaiPerformanceSamplingPolicy {
    pub const RUN_LOOP_SLOW_THRESHOLD_MS: f64 = 12.0;
    pub const HITCH_RECORD_INTERVAL: f64 = 0.20;
    pub const MAXIMUM_RECORDING_DURATION: f64 = 300.0;

    /// rate limits small events but never lets a small hitch hide a later stall
    /// frame and main thread events have separate limits at the call sites
    pub fn should_record_hitch(duration_ms: f64, at_timestamp: f64, last_recorded_at: f64) -> bool {
        duration_ms >= 50.0 || at_timestamp - last_recorded_at >= Self::HITCH_RECORD_INTERVAL
    }

    pub fn classify_frame(delta: f64, expected: f64) -> SaiPerformanceFrameClassification {
        if !delta.is_finite() || delta <= 0.0 || !expected.is_finite() || expected <= 0.0 {
            return SaiPerformanceFrameClassification { missed_frames: 0, is_hitch: false };
        }

        let cadence = MINIMUM_FRAME_INTERVAL.max(expected);
        let interval_ratio = ((i64::MAX - 1) as f64).min((delta / cadence).round().max(1.0));
        let missed_frames = (interval_ratio as i64 - 1).max(0);
        SaiPerformanceFrameClassification { missed_frames, is_hitch: missed_frames > 0 || delta >= cadence * 1.5 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaiPerformanceLiveStats {
    pub fps: f64,
    pub maximum_fps: i64,
    pub missed_frames: i64,
    pub p95_frame_ms: f64,
    pub worst_frame_ms: f64,
    pub slow_main_thread_turns: i64,
    pub memory_mb: i64,
    pub thermal_state: String,
    pub screen: String,
    pub recording_duration: f64,
}

impl Default for SaiPerformanceLiveStats {
    fn default() -> Self {
        Self {
            fps: 0.0,
            maximum_fps: i64::from(FALLBACK_FRAMES_PER_SECOND),
            missed_frames: 0,
            p95_frame_ms: 0.0,
            worst_frame_ms: 0.0,
            slow_main_thread_turns: 0,
            memory_mb: 0,
            thermal_state: "nominal".to_owned(),
            screen: "Unknown".to_owned(),
            recording_duration: 0.0,
        }
    }
}

struct PeriodAggregate {
    started_at: f64,
    last_timestamp: f64,
    frames: i64,
    missed_frames: i64,
    slow_run_loop_turns: i64,
    worst_frame: f64,
    histogram: [i64; 201],
}

impl Default for PeriodAggregate {
    fn default() -> Self {
        Self {
            started_at: 0.0,
            last_timestamp: 0.0,
            frames: 0,
            missed_frames: 0,
            slow_run_loop_turns: 0,
            worst_frame: 0.0,
            histogram: [0; 201],
        }
    }
}

impl PeriodAggregate {
    fn reset(&mut self, at_timestamp: f64) {
        self.started_at = at_timestamp;
        self.last_timestamp = at_timestamp;
        self.frames = 0;
        self.missed_frames = 0;
        self.slow_run_loop_turns = 0;
        self.worst_frame = 0.0;
        self.histogram = [0; 201];
    }

    fn add_frame(&mut self, duration: f64, missed: i64) {
        self.frames += 1;
        self.missed_frames += missed;
        self.worst_frame = self.worst_frame.max(duration);
        let milliseconds = ((duration * 1_000.0).round() as i64).clamp(0, 200);
        self.histogram[milliseconds as usize] += 1;
    }

    fn percentile95_ms(&self) -> f64 {
        if self.frames <= 0 {
            return 0.0;
        }
        let target = ((self.frames as f64 * 0.95).ceil() as i64).max(1);
        let mut observed = 0;
        for (milliseconds, count) in self.histogram.iter().enumerate() {
            observed += count;
            if observed >= target {
                return milliseconds as f64;
            }
        }
        200.0
    }
}

/// display link bound to the window that shows the hud since gpui has no screen wide one
struct DisplayLink {
    generation: u64,
    window_id: WindowId,
    is_paused: bool,
    expected_interval: f64,
}

#[derive(Clone, Copy)]
enum ApplicationLifecycleEvent {
    WillResignActive,
    DidBecomeActive,
}

/// completion that gets the finished trace when recording stops
pub type SaiPerformanceRecordingCompletion = Box<dyn FnOnce(Option<PathBuf>, &mut App)>;

pub struct SaiPerformanceMonitor {
    pub stats: SaiPerformanceLiveStats,
    pub is_preparing_recording: bool,
    pub is_recording: bool,
    pub is_finishing: bool,
    pub latest_trace_url: Option<PathBuf>,
    pub last_error_message: Option<String>,

    writer: Arc<SaiPerformanceTraceWriter>,
    display_link: Option<DisplayLink>,
    display_link_generation: u64,
    run_loop_observer: Option<Task<()>>,
    period: PeriodAggregate,
    hud_is_visible: bool,
    lifecycle_observers: Vec<RiseSubscription>,
    lifecycle_drain: Option<Task<()>>,
    last_recorded_frame_hitch_at: f64,
    last_recorded_main_hitch_at: f64,
    recording_started_at: f64,
    recording_generation: u64,
    recording_timeout: Option<Task<()>>,
    finish_completions: Vec<SaiPerformanceRecordingCompletion>,

    session_frames: i64,
    session_missed_frames: i64,
    session_slow_run_loop_turns: i64,
    session_worst_frame_ms: f64,
    session_maximum_memory_mb: i64,
    last_recorded_screen: String,
    previous_cpu_sample: Option<(f64, f64)>,
}

impl SaiPerformanceMonitor {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            stats: SaiPerformanceLiveStats::default(),
            is_preparing_recording: false,
            is_recording: false,
            is_finishing: false,
            latest_trace_url: None,
            last_error_message: None,
            writer: Arc::new(SaiPerformanceTraceWriter::new(cx.background_executor().clone())),
            display_link: None,
            display_link_generation: 0,
            run_loop_observer: None,
            period: PeriodAggregate::default(),
            hud_is_visible: false,
            lifecycle_observers: Vec::new(),
            lifecycle_drain: None,
            last_recorded_frame_hitch_at: 0.0,
            last_recorded_main_hitch_at: 0.0,
            recording_started_at: 0.0,
            recording_generation: 0,
            recording_timeout: None,
            finish_completions: Vec::new(),
            session_frames: 0,
            session_missed_frames: 0,
            session_slow_run_loop_turns: 0,
            session_worst_frame_ms: 0.0,
            session_maximum_memory_mb: 0,
            last_recorded_screen: String::new(),
            previous_cpu_sample: None,
        }
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn hud_did_appear(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.hud_is_visible = true;
        self.start_sampling_if_needed(window, cx);
        cx.notify();
    }

    pub fn hud_did_disappear(&mut self, cx: &mut Context<Self>) {
        self.hud_is_visible = false;
        self.stop_sampling_if_possible();
        cx.notify();
    }

    pub fn start_recording(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.is_preparing_recording || self.is_recording || self.is_finishing {
            return;
        }
        let Some(url) = Self::make_trace_url() else {
            self.last_error_message = Some("Unable to create trace path".to_owned());
            cx.notify();
            return;
        };

        self.recording_generation = self.recording_generation.wrapping_add(1);
        let expected_generation = self.recording_generation;
        self.is_preparing_recording = true;
        self.latest_trace_url = None;
        self.last_error_message = None;
        self.start_sampling_if_needed(window, cx);

        let started = self.writer.start(url.clone(), Self::session_metadata(window, cx));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let succeeded = started.await.unwrap_or(false);
            let _ = this.update(cx, |this, cx| {
                if this.recording_generation != expected_generation {
                    this.writer.abort(url);
                    return;
                }

                this.is_preparing_recording = false;
                if !succeeded {
                    this.last_error_message = Some("Unable to open performance trace".to_owned());
                    this.stop_sampling_if_possible();
                    cx.notify();
                    return;
                }

                this.begin_recording(url, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn stop_recording(
        &mut self,
        reason: &str,
        completion: Option<SaiPerformanceRecordingCompletion>,
        cx: &mut Context<Self>,
    ) {
        if self.is_finishing {
            if let Some(completion) = completion {
                self.finish_completions.push(completion);
            }
            return;
        }

        if self.is_preparing_recording {
            self.recording_generation = self.recording_generation.wrapping_add(1);
            self.is_preparing_recording = false;
            self.last_error_message = Some("Recording cancelled".to_owned());
            if let Some(completion) = completion {
                Self::call_completion(completion, None, cx);
            }
            self.stop_sampling_if_possible();
            cx.notify();
            return;
        }

        if !self.is_recording {
            if let Some(completion) = completion {
                Self::call_completion(completion, self.latest_trace_url.clone(), cx);
            }
            return;
        }
        if let Some(completion) = completion {
            self.finish_completions.push(completion);
        }

        self.recording_timeout = None;
        let duration = current_media_time() - self.recording_started_at;
        let screen = Self::current_screen_id(cx);
        self.record(
            "session.summary",
            string_fields([
                ("reason", reason.to_owned()),
                (map::DURATION_S, Self::decimal(duration)),
                ("frames", self.session_frames.to_string()),
                ("missed", self.session_missed_frames.to_string()),
                ("slow_main_turns", self.session_slow_run_loop_turns.to_string()),
                ("worst_ms", Self::decimal(self.session_worst_frame_ms)),
                ("max_memory_mb", self.session_maximum_memory_mb.to_string()),
                ("screen", screen),
            ]),
        );
        self.record("session.end", string_fields([("reason", reason.to_owned())]));

        self.is_recording = false;
        ChatPerformanceTrace::shared().set_event_observer(None, cx);
        self.previous_cpu_sample = None;
        self.is_finishing = true;
        self.remove_run_loop_observer();
        let url = self.latest_trace_url.clone();
        let finished = self.writer.finish(current_media_time() - self.recording_started_at);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let succeeded = finished.await.unwrap_or(false);
            let _ = this.update(cx, |this, cx| {
                let completed_url = if succeeded { url } else { None };
                if !succeeded {
                    this.latest_trace_url = None;
                    this.last_error_message = Some("Performance trace could not be finalized".to_owned());
                }
                this.is_finishing = false;
                for completion in std::mem::take(&mut this.finish_completions) {
                    Self::call_completion(completion, completed_url.clone(), cx);
                }
                this.stop_sampling_if_possible();
                cx.notify();
            });
        })
        .detach();
    }

    fn call_completion(
        completion: SaiPerformanceRecordingCompletion,
        url: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        cx.defer(move |cx| completion(url, cx));
    }

    fn begin_recording(&mut self, url: PathBuf, cx: &mut Context<Self>) {
        self.latest_trace_url = Some(url);
        self.is_recording = true;
        self.recording_started_at = current_media_time();
        self.last_recorded_frame_hitch_at = 0.0;
        self.last_recorded_main_hitch_at = 0.0;
        self.session_frames = 0;
        self.session_missed_frames = 0;
        self.session_slow_run_loop_turns = 0;
        self.session_worst_frame_ms = 0.0;
        self.session_maximum_memory_mb = resident_memory_mb();
        self.last_recorded_screen = Self::current_screen_id(cx);
        self.period.reset(0.0);
        self.install_run_loop_observer(cx);
        self.record("session.start", string_fields([("screen", self.last_recorded_screen.clone())]));
        self.previous_cpu_sample = Self::cpu_sample();

        let writer = Arc::clone(&self.writer);
        let recording_started_at = self.recording_started_at;
        ChatPerformanceTrace::shared().set_event_observer(
            Some(Box::new(move |event: &str, fields: &HashMap<String, String>| {
                writer.append(
                    &format!("chat.{event}"),
                    (current_media_time() - recording_started_at).max(0.0),
                    fields.clone(),
                );
            })),
            cx,
        );

        let background = cx.background_executor().clone();
        self.recording_timeout = Some(cx.spawn(async move |this, cx| {
            background
                .timer(Duration::from_secs_f64(SaiPerformanceSamplingPolicy::MAXIMUM_RECORDING_DURATION))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.is_recording {
                    this.stop_recording("timeout", None, cx);
                }
            });
        }));
    }

    fn start_sampling_if_needed(&mut self, window: &Window, cx: &mut Context<Self>) {
        let window_id = window.window_handle().window_id();
        if self.display_link.as_ref().is_some_and(|link| link.window_id == window_id) {
            return;
        }
        self.period.reset(0.0);
        self.display_link_generation = self.display_link_generation.wrapping_add(1);
        let generation = self.display_link_generation;
        let maximum_fps = maximum_frames_per_second(window);
        self.stats.maximum_fps = i64::from(maximum_fps);
        self.display_link = Some(DisplayLink {
            generation,
            window_id,
            is_paused: RiseAppLifecycle::scene_phase(cx) != ScenePhase::Active,
            expected_interval: 1.0 / f64::from(maximum_fps.max(1)),
        });
        Self::schedule_display_link(window, cx.weak_entity(), generation);
        self.install_lifecycle_observers(cx);
    }

    fn stop_sampling_if_possible(&mut self) {
        if self.hud_is_visible || self.is_preparing_recording || self.is_recording || self.is_finishing {
            return;
        }
        self.display_link = None;
        self.period.reset(0.0);
        self.remove_lifecycle_observers();
    }

    fn schedule_display_link(window: &Window, this: WeakEntity<Self>, generation: u64) {
        window.on_next_frame(move |window, cx| {
            let is_installed =
                this.update(cx, |this, cx| this.display_link_did_fire(generation, window, cx)).unwrap_or(false);
            if is_installed {
                Self::schedule_display_link(window, this, generation);
            }
        });
    }

    fn display_link_did_fire(&mut self, generation: u64, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(link) = self.display_link.as_ref() else {
            return false;
        };
        if link.generation != generation {
            return false;
        }
        if link.is_paused {
            return true;
        }
        let link_interval = link.expected_interval;

        let timestamp = current_media_time();
        if self.period.last_timestamp <= 0.0 {
            self.period.reset(timestamp);
            return true;
        }

        let delta = timestamp - self.period.last_timestamp;
        self.period.last_timestamp = timestamp;
        if !delta.is_finite() || delta <= 0.0 {
            return true;
        }

        let expected = MINIMUM_FRAME_INTERVAL.max(link_interval);
        let classification = SaiPerformanceSamplingPolicy::classify_frame(delta, expected);
        self.period.add_frame(delta, classification.missed_frames);

        if self.is_recording {
            self.session_frames += 1;
            self.session_missed_frames += classification.missed_frames;
            self.session_worst_frame_ms = self.session_worst_frame_ms.max(delta * 1_000.0);

            if classification.is_hitch
                && SaiPerformanceSamplingPolicy::should_record_hitch(
                    delta * 1_000.0,
                    timestamp,
                    self.last_recorded_frame_hitch_at,
                )
            {
                self.last_recorded_frame_hitch_at = timestamp;
                let screen = Self::current_screen_id(cx);
                self.record(
                    "frame.hitch",
                    merging_current(
                        string_fields([
                            ("frame_ms", Self::decimal(delta * 1_000.0)),
                            ("expected_ms", Self::decimal(expected * 1_000.0)),
                            (
                                "callback_lag_ms",
                                Self::decimal((current_media_time() - timestamp).max(0.0) * 1_000.0),
                            ),
                            ("missed", classification.missed_frames.to_string()),
                            ("screen", screen),
                        ]),
                        ChatPerformanceTrace::shared().diagnostic_context(),
                    ),
                );
            }
        }

        if timestamp - self.period.started_at >= 1.0 {
            self.publish_period(timestamp, window, cx);
            self.period.reset(timestamp);
        }
        true
    }

    fn publish_period(&mut self, timestamp: f64, window: &Window, cx: &mut Context<Self>) {
        let duration = (timestamp - self.period.started_at).max(0.001);
        let fps = self.period.frames as f64 / duration;
        let p95 = self.period.percentile95_ms();
        let worst_ms = self.period.worst_frame * 1_000.0;
        let memory_mb = resident_memory_mb();
        let thermal = Self::thermal_name();
        let screen = Self::current_screen_id(cx);
        let recording_duration =
            if self.is_recording { (current_media_time() - self.recording_started_at).max(0.0) } else { 0.0 };
        let maximum_fps = maximum_frames_per_second(window);
        if let Some(link) = self.display_link.as_mut() {
            link.expected_interval = 1.0 / f64::from(maximum_fps.max(1));
        }

        self.stats = SaiPerformanceLiveStats {
            fps,
            maximum_fps: i64::from(maximum_fps),
            missed_frames: self.period.missed_frames,
            p95_frame_ms: p95,
            worst_frame_ms: worst_ms,
            slow_main_thread_turns: self.period.slow_run_loop_turns,
            memory_mb,
            thermal_state: thermal.to_owned(),
            screen: screen.clone(),
            recording_duration,
        };
        cx.notify();

        if !self.is_recording {
            return;
        }
        let mut workload = ChatPerformanceTrace::shared().diagnostic_context();
        let sampled_at = current_media_time();
        if let Some(seconds) = process_cpu_seconds() {
            if let Some((previous_time, previous_seconds)) = self.previous_cpu_sample
                && sampled_at > previous_time
            {
                workload.insert(
                    "cpu_percent_one_core".to_owned(),
                    Self::decimal((seconds - previous_seconds).max(0.0) / (sampled_at - previous_time) * 100.0),
                );
            }
            self.previous_cpu_sample = Some((sampled_at, seconds));
        }
        self.session_maximum_memory_mb = self.session_maximum_memory_mb.max(memory_mb);
        if screen != self.last_recorded_screen {
            self.record("navigation.screen", string_fields([("screen", screen.clone())]));
            self.last_recorded_screen = screen.clone();
        }
        self.record(
            "frame.summary",
            merging_current(
                string_fields([
                    ("fps", Self::decimal(fps)),
                    ("maximum_fps", maximum_fps.to_string()),
                    ("frames", self.period.frames.to_string()),
                    ("missed", self.period.missed_frames.to_string()),
                    ("p95_ms", Self::decimal(p95)),
                    ("worst_ms", Self::decimal(worst_ms)),
                    ("slow_main_turns", self.period.slow_run_loop_turns.to_string()),
                    ("memory_mb", memory_mb.to_string()),
                    ("thermal", thermal.to_owned()),
                    ("low_power", flag(system_low_power_mode())),
                    ("screen", screen),
                ]),
                workload,
            ),
        );
    }

    fn install_run_loop_observer(&mut self, cx: &mut Context<Self>) {
        if self.run_loop_observer.is_some() {
            return;
        }
        let background = cx.background_executor().clone();
        self.run_loop_observer = Some(cx.spawn(async move |this, cx| {
            loop {
                let turn_started_at = current_media_time();
                background.timer(RUN_LOOP_PROBE_INTERVAL).await;
                let due_at = turn_started_at + RUN_LOOP_PROBE_INTERVAL.as_secs_f64();
                let is_installed = this.update(cx, |this, cx| this.run_loop_turn_did_end(due_at, cx)).unwrap_or(false);
                if !is_installed {
                    break;
                }
            }
        }));
    }

    fn run_loop_turn_did_end(&mut self, due_at: f64, cx: &mut Context<Self>) -> bool {
        if !self.is_recording {
            return false;
        }
        let now = current_media_time();
        let duration_ms = (now - due_at) * 1_000.0;
        if duration_ms < SaiPerformanceSamplingPolicy::RUN_LOOP_SLOW_THRESHOLD_MS {
            return true;
        }
        self.period.slow_run_loop_turns += 1;
        self.session_slow_run_loop_turns += 1;
        if !SaiPerformanceSamplingPolicy::should_record_hitch(duration_ms, now, self.last_recorded_main_hitch_at) {
            return true;
        }
        self.last_recorded_main_hitch_at = now;
        let screen = Self::current_screen_id(cx);
        self.record(
            "main_runloop.slow",
            merging_current(
                string_fields([(map::DURATION_MS, Self::decimal(duration_ms)), ("screen", screen)]),
                ChatPerformanceTrace::shared().diagnostic_context(),
            ),
        );
        true
    }

    fn remove_run_loop_observer(&mut self) {
        self.run_loop_observer = None;
    }

    fn install_lifecycle_observers(&mut self, cx: &mut Context<Self>) {
        if !self.lifecycle_observers.is_empty() {
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<ApplicationLifecycleEvent>();
        let resign_sender = sender.clone();
        let center = RiseAppEvents::default_center();
        self.lifecycle_observers = vec![
            center.observe(UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION, move |_| {
                let _ = resign_sender.unbounded_send(ApplicationLifecycleEvent::WillResignActive);
            }),
            center.observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(ApplicationLifecycleEvent::DidBecomeActive);
            }),
        ];
        self.lifecycle_drain = Some(cx.spawn(async move |this, cx| {
            while let Some(event) = receiver.next().await {
                let updated = this.update(cx, |this, _cx| match event {
                    ApplicationLifecycleEvent::WillResignActive => this.application_will_resign_active(),
                    ApplicationLifecycleEvent::DidBecomeActive => this.application_did_become_active(),
                });
                if updated.is_err() {
                    break;
                }
            }
        }));
    }

    fn remove_lifecycle_observers(&mut self) {
        self.lifecycle_observers.clear();
        self.lifecycle_drain = None;
    }

    fn application_will_resign_active(&mut self) {
        if self.is_recording {
            self.record("lifecycle.inactive", HashMap::new());
        }
        if let Some(link) = self.display_link.as_mut() {
            link.is_paused = true;
        }
        self.period.reset(0.0);
        self.previous_cpu_sample = None;
    }

    fn application_did_become_active(&mut self) {
        self.period.reset(0.0);
        self.previous_cpu_sample = Self::cpu_sample();
        if let Some(link) = self.display_link.as_mut() {
            link.is_paused = false;
        }
        if self.is_recording {
            self.record("lifecycle.active", HashMap::new());
        }
    }

    fn record(&self, event: &str, fields: HashMap<String, String>) {
        if !self.is_recording {
            return;
        }
        self.writer.append(event, (current_media_time() - self.recording_started_at).max(0.0), fields);
    }

    fn cpu_sample() -> Option<(f64, f64)> {
        process_cpu_seconds().map(|seconds| (current_media_time(), seconds))
    }

    fn current_screen_id(cx: &mut App) -> String {
        SaiNavigationBridge::current_screen(cx).map_or_else(|| "Unknown".to_owned(), |screen| screen.id)
    }

    fn make_trace_url() -> Option<PathBuf> {
        let base = RiseDiagnostics::application_support_directory()?;
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
        let identifier = Uuid::new_v4().uuid_string().to_lowercase();
        Some(
            base.join("Diagnostics")
                .join("AppPerformance")
                .join(format!("{TRACE_FILE_PREFIX}{stamp}-{identifier}.jsonl")),
        )
    }

    fn session_metadata(window: &Window, cx: &mut App) -> HashMap<String, String> {
        let configuration = if cfg!(debug_assertions) { "debug" } else { "release" };
        let app_info = AppInfo::current(AppEnvironment::compiled().as_str());
        let build = app_info.as_ref().map_or_else(|| "unknown".to_owned(), |info| info.build.clone());
        let version = app_info.as_ref().map_or_else(|| "unknown".to_owned(), |info| info.short_version.clone());
        string_fields([
            ("schema", "riseonly.performance.v1".to_owned()),
            ("build", build),
            (music::VERSION, version),
            ("configuration", configuration.to_owned()),
            ("debugger_attached", flag(is_debugger_attached())),
            (event_collector::DEVICE, DeviceInfo::machine_identifier()),
            (ledger_admin::SYSTEM, DeviceInfo::system_version()),
            ("maximum_fps", maximum_frames_per_second(window).to_string()),
            (post::SCALE, Self::decimal(f64::from(window.scale_factor()))),
            ("low_power", flag(system_low_power_mode())),
            ("thermal", Self::thermal_name().to_owned()),
            ("reduce_motion", flag(reduce_motion())),
            ("initial_memory_mb", resident_memory_mb().to_string()),
            ("initial_screen", Self::current_screen_id(cx)),
        ])
    }

    fn decimal(value: f64) -> String {
        format!("{value:.2}")
    }

    fn thermal_name() -> &'static str {
        let reading = thermal_reading();
        if reading.unavailable.is_some() { "unknown" } else { reading.state.name() }
    }
}

fn flag(value: bool) -> String {
    String::from(if value { "1" } else { "0" })
}

fn string_fields<const N: usize>(fields: [(&str, String); N]) -> HashMap<String, String> {
    fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

/// merges fields where the current value wins
fn merging_current(fields: HashMap<String, String>, other: HashMap<String, String>) -> HashMap<String, String> {
    let mut merged = other;
    merged.extend(fields);
    merged
}

#[derive(Default)]
struct AdmissionState {
    pending: usize,
    dropped: usize,
}

#[derive(Default)]
struct WriterFile {
    handle: Option<File>,
    buffered_data: Vec<u8>,
    buffered_line_count: usize,
    file_limit_dropped_events: usize,
    written_bytes: usize,
    did_fail: bool,
}

enum WriterJob {
    Start { url: PathBuf, metadata: HashMap<String, String>, completion: oneshot::Sender<bool> },
    Append { event: String, relative_time: f64, fields: HashMap<String, String> },
    Finish { relative_time: f64, completion: oneshot::Sender<bool> },
    Abort { url: PathBuf },
}

#[derive(Default)]
struct WriterQueue {
    jobs: VecDeque<WriterJob>,
    is_running: bool,
}

#[derive(Default)]
struct WriterShared {
    admission: Mutex<AdmissionState>,
    queue: Mutex<WriterQueue>,
    file: Mutex<WriterFile>,
}

/// serial performance trace queue, jobs run one at a time in order on a background thread
struct SaiPerformanceTraceWriter {
    shared: Arc<WriterShared>,
    executor: BackgroundExecutor,
}

impl SaiPerformanceTraceWriter {
    fn new(executor: BackgroundExecutor) -> Self {
        Self { shared: Arc::new(WriterShared::default()), executor }
    }

    fn start(&self, url: PathBuf, metadata: HashMap<String, String>) -> oneshot::Receiver<bool> {
        let (completion, started) = oneshot::channel();
        self.enqueue(WriterJob::Start { url, metadata, completion });
        started
    }

    fn append(&self, event: &str, relative_time: f64, fields: HashMap<String, String>) {
        {
            let mut admission = self.shared.admission.lock();
            if admission.pending >= MAXIMUM_PENDING_EVENTS {
                admission.dropped += 1;
                return;
            }
            admission.pending += 1;
        }
        self.enqueue(WriterJob::Append { event: event.to_owned(), relative_time, fields });
    }

    fn finish(&self, relative_time: f64) -> oneshot::Receiver<bool> {
        let (completion, finished) = oneshot::channel();
        self.enqueue(WriterJob::Finish { relative_time, completion });
        finished
    }

    fn abort(&self, url: PathBuf) {
        self.enqueue(WriterJob::Abort { url });
    }

    fn enqueue(&self, job: WriterJob) {
        let should_spawn = {
            let mut queue = self.shared.queue.lock();
            queue.jobs.push_back(job);
            !std::mem::replace(&mut queue.is_running, true)
        };
        if should_spawn {
            let shared = Arc::clone(&self.shared);
            self.executor.spawn(async move { shared.run_jobs() }).detach();
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
                WriterJob::Start { url, metadata, completion } => {
                    let _ = completion.send(self.start_file(&url, &metadata));
                }
                WriterJob::Append { event, relative_time, fields } => {
                    Self::append_encoded(&mut self.file.lock(), &event, relative_time, &fields, true);
                    let mut admission = self.admission.lock();
                    admission.pending = admission.pending.saturating_sub(1);
                }
                WriterJob::Finish { relative_time, completion } => {
                    let _ = completion.send(self.finish_file(relative_time));
                }
                WriterJob::Abort { url } => self.abort_file(&url),
            }
        }
    }

    fn start_file(&self, url: &Path, metadata: &HashMap<String, String>) -> bool {
        let mut file = self.file.lock();
        let Some(directory) = url.parent() else {
            return false;
        };
        if FileManager.create_directory(directory, true).is_err() {
            file.handle = None;
            return false;
        }
        Self::cleanup_old_traces(directory);
        if !FileManager.create_file(url, None) {
            return false;
        }
        let Ok(handle) = OpenOptions::new().write(true).open(url) else {
            file.handle = None;
            return false;
        };
        *file = WriterFile { handle: Some(handle), ..WriterFile::default() };
        *self.admission.lock() = AdmissionState::default();
        Self::append_encoded(&mut file, "metadata", 0.0, metadata, true);
        Self::flush(&mut file);
        if let Some(handle) = file.handle.as_ref()
            && handle.sync_all().is_err()
        {
            file.handle = None;
            return false;
        }
        !file.did_fail
    }

    fn finish_file(&self, relative_time: f64) -> bool {
        let mut file = self.file.lock();
        let dropped = self.admission.lock().dropped + file.file_limit_dropped_events;
        if dropped > 0 {
            let fields = string_fields([(map::COUNT, dropped.to_string())]);
            Self::append_encoded(&mut file, "writer.dropped", relative_time, &fields, false);
        }
        Self::flush(&mut file);
        let handle = file.handle.take();
        if let Some(handle) = handle.as_ref()
            && handle.sync_all().is_err()
        {
            file.did_fail = true;
        }
        handle.is_some() && !file.did_fail
    }

    fn abort_file(&self, url: &Path) {
        let mut file = self.file.lock();
        file.handle = None;
        file.buffered_data.clear();
        file.buffered_line_count = 0;
        let _ = FileManager.remove_item(url);
    }

    fn append_encoded(
        file: &mut WriterFile,
        event: &str,
        relative_time: f64,
        fields: &HashMap<String, String>,
        enforces_file_limit: bool,
    ) {
        if file.handle.is_none() || file.did_fail {
            return;
        }
        let fields: Map<String, Value> =
            fields.iter().map(|(key, value)| (key.clone(), Value::from(value.as_str()))).collect();
        let mut payload = Map::new();
        payload.insert(notify::EVENT.to_owned(), Value::from(event));
        payload.insert("t_ms".to_owned(), Value::from((relative_time.max(0.0) * 1_000.0) as i64));
        payload.insert("fields".to_owned(), Value::Object(fields));
        let Some(mut data) =
            JSONSerialization::data_with_json_object(&Value::Object(payload), JSONWritingOptions::default())
        else {
            file.did_fail = true;
            return;
        };
        data.push(b'\n');
        if enforces_file_limit && file.written_bytes + file.buffered_data.len() + data.len() > MAXIMUM_FILE_BYTES {
            file.file_limit_dropped_events += 1;
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
            Ok(()) => file.written_bytes += file.buffered_data.len(),
            Err(_) => file.did_fail = true,
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
        for (path, _) in traces.into_iter().skip(KEPT_TRACES) {
            let _ = FileManager.remove_item(&path);
        }
    }
}
