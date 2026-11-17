use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, Task, Window};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::crash_diagnostics;
use rise_platform::display_cadence::maximum_frames_per_second;
use rise_platform::power::system_low_power_mode;
use rise_platform::process_info::{ThermalState, thermal_state};

use crate::modules::media_picker::library::media_picker_thumbnail_pipeline::MediaPickerThumbnailPipelineQuality;

const LOG_TARGET: &str = "MediaPickerPerf";
const PERIODIC_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct MediaPickerPerformanceDiagnosticsState {
    live_grid_ids: HashSet<String>,
    asset_count: usize,
    counters: HashMap<String, i64>,
    pipeline_active: usize,
    pipeline_queued: usize,
    phase: String,
    velocity: f32,
    visible: usize,
}

impl Default for MediaPickerPerformanceDiagnosticsState {
    fn default() -> Self {
        Self {
            live_grid_ids: HashSet::new(),
            asset_count: 0,
            counters: HashMap::new(),
            pipeline_active: 0,
            pipeline_queued: 0,
            phase: "closed".to_owned(),
            velocity: 0.0,
            visible: 0,
        }
    }
}

/// debug build counters of the picker grid, logged every two seconds while a grid is open
pub struct MediaPickerPerformanceDiagnostics {
    lock: Mutex<MediaPickerPerformanceDiagnosticsState>,
    timer: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

static SHARED: LazyLock<MediaPickerPerformanceDiagnostics> = LazyLock::new(|| MediaPickerPerformanceDiagnostics {
    lock: Mutex::new(MediaPickerPerformanceDiagnosticsState::default()),
    timer: Mutex::new(None),
});

impl MediaPickerPerformanceDiagnostics {
    pub fn shared() -> &'static Self {
        &SHARED
    }

    pub fn grid_opened(&self, id: &str, asset_count: usize) {
        if !cfg!(debug_assertions) {
            return;
        }
        let should_start = self.with_state(|state| {
            state.live_grid_ids.insert(id.to_owned());
            state.asset_count = asset_count;
            state.phase = "open".to_owned();
            *state.counters.entry("grid_open".to_owned()).or_insert(0) += 1;
            state.live_grid_ids.len() == 1
        });
        if should_start {
            self.start_timer();
        }
        self.emit(&format!("grid_open_{id}"));
    }

    pub fn grid_closed(&self, id: &str) {
        if !cfg!(debug_assertions) {
            return;
        }
        let should_stop = self.with_state(|state| {
            state.live_grid_ids.remove(id);
            *state.counters.entry("grid_close".to_owned()).or_insert(0) += 1;
            if state.live_grid_ids.is_empty() {
                state.phase = "closed".to_owned();
            }
            state.live_grid_ids.is_empty()
        });
        self.emit(&format!("grid_close_{id}"));
        if should_stop {
            self.stop_timer();
        }
    }

    pub fn record_thumbnail_cache_hit(&self, quality: MediaPickerThumbnailPipelineQuality) {
        if cfg!(debug_assertions) {
            self.increment(&format!("thumb_hit_{}", Self::quality_name(quality)), 1);
        }
    }

    pub fn record_thumbnail_requested(&self, quality: MediaPickerThumbnailPipelineQuality) {
        if cfg!(debug_assertions) {
            self.increment(&format!("thumb_req_{}", Self::quality_name(quality)), 1);
        }
    }

    pub fn record_thumbnail_started(&self, quality: MediaPickerThumbnailPipelineQuality) {
        if cfg!(debug_assertions) {
            self.increment(&format!("thumb_start_{}", Self::quality_name(quality)), 1);
        }
    }

    pub fn record_thumbnail_cancelled(&self, quality: Option<MediaPickerThumbnailPipelineQuality>) {
        if cfg!(debug_assertions) {
            let name = quality.map_or("unknown", Self::quality_name);
            self.increment(&format!("thumb_cancel_{name}"), 1);
        }
    }

    pub fn record_thumbnail_delivery(&self, degraded: bool, quality: MediaPickerThumbnailPipelineQuality) {
        if cfg!(debug_assertions) {
            let stage = if degraded { "degraded" } else { "final" };
            self.increment(&format!("thumb_{stage}_{}", Self::quality_name(quality)), 1);
        }
    }

    pub fn update_pipeline_depth(&self, active: usize, queued: usize) {
        if cfg!(debug_assertions) {
            self.with_state(|state| {
                state.pipeline_active = active;
                state.pipeline_queued = queued;
            });
        }
    }

    pub fn record_photo_cache_start(&self, assets: i64) {
        if cfg!(debug_assertions) {
            self.increment("cache_preview_start_assets", assets);
        }
    }

    pub fn record_photo_cache_stop(&self, assets: i64) {
        if cfg!(debug_assertions) {
            self.increment("cache_preview_stop_assets", assets);
        }
    }

    pub fn record_tiny_cache_start(&self, assets: i64) {
        if cfg!(debug_assertions) {
            self.increment("cache_tiny_start_assets", assets);
        }
    }

    pub fn record_tiny_cache_stop(&self, assets: i64) {
        if cfg!(debug_assertions) {
            self.increment("cache_tiny_stop_assets", assets);
        }
    }

    pub fn record_tiny_requested(&self) {
        if cfg!(debug_assertions) {
            self.increment("tiny_req", 1);
        }
    }

    pub fn record_cell_bind(&self) {
        if cfg!(debug_assertions) {
            self.increment("cell_bind", 1);
        }
    }

    pub fn record_scroll_callbacks(&self, count: i64) {
        if !cfg!(debug_assertions) || count <= 0 {
            return;
        }
        self.increment("scroll_cb", count);
    }

    pub fn record_grid_layout(&self, resizing: bool) {
        if cfg!(debug_assertions) {
            self.increment(if resizing { "layout_resize" } else { "layout" }, 1);
        }
    }

    pub fn update_scroll_context(&self, phase: &str, velocity: f32, visible: usize) {
        if cfg!(debug_assertions) {
            self.with_state(|state| {
                state.phase = phase.to_owned();
                state.velocity = velocity;
                state.visible = visible;
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_scroll_result(
        &self,
        id: &str,
        reason: &str,
        duration: f64,
        fps: f64,
        hitches: i64,
        worst_frame_ms: f64,
        deceleration_rate: f32,
        frames_over12_ms: i64,
        frames_over20_ms: i64,
        frames_over50_ms: i64,
    ) {
        if !cfg!(debug_assertions) {
            return;
        }
        let line = format!(
            "[MediaPickerPerf] scroll_end id={id} reason={reason} duration={duration:.2}s fps={fps:.1} \
             hitches={hitches} frames_gt12ms={frames_over12_ms} frames_gt20ms={frames_over20_ms} \
             frames_gt50ms={frames_over50_ms} worst_ms={worst_frame_ms:.1} decel={deceleration_rate:.4}"
        );
        RiseAccountRuntime::current().spawn_blocking(move || {
            tracing::info!(target: LOG_TARGET, "{line}");
        });
    }

    pub fn emit(&self, reason: &str) {
        if !cfg!(debug_assertions) {
            return;
        }
        let snapshot = self.with_state(|state| {
            let copy = state.clone();
            state.counters.clear();
            copy
        });
        if snapshot.live_grid_ids.is_empty() && !reason.starts_with("grid_close") {
            return;
        }
        let reason = reason.to_owned();
        RiseAccountRuntime::current().spawn_blocking(move || Self::log(&snapshot, &reason));
    }

    fn log(snapshot: &MediaPickerPerformanceDiagnosticsState, reason: &str) {
        let mut counters: Vec<(&String, &i64)> = snapshot.counters.iter().collect();
        counters.sort_unstable_by_key(|(key, _)| *key);
        let counters = counters
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" ");
        let line = format!(
            "[MediaPickerPerf] reason={reason} grids={} assets={} phase={} velocity={} visible={} pipeline={}/{} \
             mem_mb={} thermal={} low_power={} {counters}",
            snapshot.live_grid_ids.len(),
            snapshot.asset_count,
            snapshot.phase,
            snapshot.velocity.round() as i64,
            snapshot.visible,
            snapshot.pipeline_active,
            snapshot.pipeline_queued,
            Self::resident_memory_mb(),
            Self::thermal_name(thermal_state()),
            u8::from(system_low_power_mode()),
        );
        tracing::info!(target: LOG_TARGET, "{line}");
    }

    fn increment(&self, key: &str, amount: i64) {
        self.with_state(|state| *state.counters.entry(key.to_owned()).or_insert(0) += amount);
    }

    fn quality_name(quality: MediaPickerThumbnailPipelineQuality) -> &'static str {
        if quality == MediaPickerThumbnailPipelineQuality::Sharp {
            "sharp"
        } else {
            "preview"
        }
    }

    fn with_state<T>(&self, body: impl FnOnce(&mut MediaPickerPerformanceDiagnosticsState) -> T) -> T {
        let mut state = self.lock.lock();
        body(&mut state)
    }

    fn start_timer(&self) {
        let mut timer = self.timer.lock();
        if timer.is_some() {
            return;
        }
        *timer = Some(RiseAccountRuntime::current().spawn(async {
            loop {
                tokio::time::sleep(PERIODIC_INTERVAL).await;
                Self::shared().emit("periodic");
            }
        }));
    }

    fn stop_timer(&self) {
        let timer = self.timer.lock().take();
        if let Some(timer) = timer {
            timer.abort();
        }
    }

    fn resident_memory_mb() -> i64 {
        crash_diagnostics::resident_memory_mb()
    }

    fn thermal_name(state: ThermalState) -> &'static str {
        state.name()
    }
}

#[derive(Default)]
struct MediaPickerScrollPerformanceSamplerState {
    id: String,
    deceleration_rate: f32,
    first_timestamp: f64,
    last_timestamp: f64,
    frame_count: i64,
    hitch_count: i64,
    worst_frame: f64,
    frames_over12_ms: i64,
    frames_over20_ms: i64,
    frames_over50_ms: i64,
}

impl MediaPickerScrollPerformanceSamplerState {
    fn tick(&mut self, timestamp: f64, link_duration: f64) {
        if self.first_timestamp == 0.0 {
            self.first_timestamp = timestamp;
            self.last_timestamp = timestamp;
            return;
        }
        let delta = timestamp - self.last_timestamp;
        self.last_timestamp = timestamp;
        self.frame_count += 1;
        self.worst_frame = self.worst_frame.max(delta);
        if delta > 0.012 {
            self.frames_over12_ms += 1;
        }
        if delta > 0.020 {
            self.frames_over20_ms += 1;
        }
        if delta > 0.050 {
            self.frames_over50_ms += 1;
        }
        let hitch_threshold = (1.0 / 45.0_f64).max(link_duration * 1.8);
        if delta > hitch_threshold {
            self.hitch_count += 1;
        }
    }
}

/// debug build frame pacing of one grid scroll, reported when the scroll ends
#[derive(Default)]
pub struct MediaPickerScrollPerformanceSampler {
    display_link: Option<Task<()>>,
    state: Rc<RefCell<MediaPickerScrollPerformanceSamplerState>>,
}

impl MediaPickerScrollPerformanceSampler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, id: &str, deceleration_rate: f32, window: &mut Window, cx: &mut App) {
        if !cfg!(debug_assertions) {
            return;
        }
        self.stop("restart");
        *self.state.borrow_mut() = MediaPickerScrollPerformanceSamplerState {
            id: id.to_owned(),
            deceleration_rate,
            ..MediaPickerScrollPerformanceSamplerState::default()
        };
        let maximum_fps = maximum_frames_per_second(window).max(60);
        let link_duration = 1.0 / f64::from(maximum_fps);
        let state = Rc::clone(&self.state);
        self.display_link = Some(window.spawn(cx, async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs_f64(link_duration)).await;
                if cx.update(|_, _| ()).is_err() {
                    break;
                }
                let timestamp = RiseClock::monotonic().as_secs_f64();
                state.borrow_mut().tick(timestamp, link_duration);
            }
        }));
    }

    pub fn stop(&mut self, reason: &str) {
        if !cfg!(debug_assertions) {
            return;
        }
        let Some(display_link) = self.display_link.take() else {
            return;
        };
        drop(display_link);
        let state = self.state.borrow();
        let duration = (state.last_timestamp - state.first_timestamp).max(0.0);
        let fps = if duration > 0.0 { state.frame_count as f64 / duration } else { 0.0 };
        MediaPickerPerformanceDiagnostics::shared().record_scroll_result(
            &state.id,
            reason,
            duration,
            fps,
            state.hitch_count,
            state.worst_frame * 1_000.0,
            state.deceleration_rate,
            state.frames_over12_ms,
            state.frames_over20_ms,
            state.frames_over50_ms,
        );
    }
}
