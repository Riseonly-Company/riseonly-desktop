use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::panic::Location;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{App, FrameTiming, FrameTimingCollector, Global};
use rise_core::foundation::foundation_date::RiseClock;

pub const PERF_LOG_VARIABLE: &str = "RISE_PERF_LOG";
pub const FRAME_BUDGET_MS: f64 = 8.0;
pub const FRAME_CEILING_MS: f64 = 16.0;
pub const LOG_HEADER: &str =
    "# riseonly perf log v1: frame epoch_ms window draw_ms layout_ms paint_ms views invalidations shaped_lines shape_ms fonts font_ms splice cpu_ms faults";

const DRAIN_INTERVAL: Duration = Duration::from_millis(500);

/// percentiles and slow frame counts over the draw times of a run
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PerfSummary {
    pub frames: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub over_budget: usize,
    pub over_ceiling: usize,
}

impl PerfSummary {
    /// nearest rank percentiles, an empty run gives all zeros
    pub fn of(draws_ms: &[f64]) -> Self {
        let mut sorted: Vec<f64> = draws_ms.iter().copied().filter(|value| value.is_finite()).collect();
        if sorted.is_empty() {
            return Self::default();
        }
        sorted.sort_by(f64::total_cmp);
        Self {
            frames: sorted.len(),
            p50_ms: nearest_rank(&sorted, 50.0),
            p95_ms: nearest_rank(&sorted, 95.0),
            p99_ms: nearest_rank(&sorted, 99.0),
            max_ms: sorted.last().copied().unwrap_or_default(),
            over_budget: sorted.iter().filter(|value| **value > FRAME_BUDGET_MS).count(),
            over_ceiling: sorted.iter().filter(|value| **value > FRAME_CEILING_MS).count(),
        }
    }

    pub fn line(&self) -> String {
        format!(
            "summary frames={} p50={:.2} p95={:.2} p99={:.2} max={:.2} over8={} over16={}",
            self.frames, self.p50_ms, self.p95_ms, self.p99_ms, self.max_ms, self.over_budget, self.over_ceiling
        )
    }
}

fn nearest_rank(sorted: &[f64], percentile: f64) -> f64 {
    let rank = ((percentile / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// path of the perf log, the variable if set and not empty
pub fn perf_log_path(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// one log line of a frame
/// views_by_type line is name and count pairs of renders since previous, most rendered first
/// none if nothing rendered
pub fn views_by_type_line(
    epoch_ms: f64,
    totals: &[(&'static str, u64)],
    previous: &mut HashMap<&'static str, u64>,
) -> Option<String> {
    let mut deltas: Vec<(&'static str, u64)> = Vec::new();
    for &(name, total) in totals {
        let seen = previous.insert(name, total).unwrap_or_default();
        if total > seen {
            deltas.push((name, total - seen));
        }
    }
    if deltas.is_empty() {
        return None;
    }
    deltas.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    let mut line = format!("views_by_type\t{epoch_ms:.1}");
    for (name, count) in deltas {
        line.push('\t');
        line.push_str(&name.replace(char::is_whitespace, ""));
        line.push('\t');
        line.push_str(&count.to_string());
    }
    line.push('\n');
    Some(line)
}

pub const DIRTY_BY_TOP: usize = 120;

/// dirty_by line is file:line and count pairs of notifies and refreshes since previous
/// most frequent first, none if there were none
pub fn dirty_by_line(
    epoch_ms: f64,
    totals: &[(&'static Location<'static>, u64)],
    previous: &mut HashMap<&'static Location<'static>, u64>,
) -> Option<String> {
    let mut deltas: Vec<(&'static Location<'static>, u64)> = Vec::new();
    for &(location, total) in totals {
        let seen = previous.insert(location, total).unwrap_or_default();
        if total > seen {
            deltas.push((location, total - seen));
        }
    }
    if deltas.is_empty() {
        return None;
    }
    deltas.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    let mut line = format!("dirty_by\t{epoch_ms:.1}");
    for (location, count) in deltas.into_iter().take(DIRTY_BY_TOP) {
        line.push('\t');
        line.push_str(&format!("{}:{}", location.file(), location.line()).replace(char::is_whitespace, ""));
        line.push('\t');
        line.push_str(&count.to_string());
    }
    line.push('\n');
    Some(line)
}

/// miss_by line is type and reason pairs of cached views that rendered instead of replaying
/// since previous, none if there were none
pub fn miss_by_line(
    epoch_ms: f64,
    totals: &[((&'static str, &'static str), u64)],
    previous: &mut HashMap<(&'static str, &'static str), u64>,
) -> Option<String> {
    let mut deltas: Vec<((&'static str, &'static str), u64)> = Vec::new();
    for &(key, total) in totals {
        let seen = previous.insert(key, total).unwrap_or_default();
        if total > seen {
            deltas.push((key, total - seen));
        }
    }
    if deltas.is_empty() {
        return None;
    }
    deltas.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    let mut line = format!("miss_by\t{epoch_ms:.1}");
    for ((name, reason), count) in deltas.into_iter().take(DIRTY_BY_TOP) {
        line.push('\t');
        line.push_str(&name.replace(char::is_whitespace, ""));
        line.push('|');
        line.push_str(reason);
        line.push('\t');
        line.push_str(&count.to_string());
    }
    line.push('\n');
    Some(line)
}

pub fn frame_line(timing: &FrameTiming, anchor: (Instant, f64)) -> String {
    let (anchor_instant, anchor_epoch_ms) = anchor;
    let epoch_ms = anchor_epoch_ms + millis(timing.draw_end.saturating_duration_since(anchor_instant));
    format!(
        "frame\t{:.1}\t{}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}\t{}\t{:.3}\t{}\t{:.3}\t{}\t{}\t{}\n",
        epoch_ms,
        timing.window_id.as_u64(),
        millis(timing.draw_duration()),
        timing.layout_duration().map(millis).unwrap_or_default(),
        timing.paint_duration().map(millis).unwrap_or_default(),
        timing.rendered_views,
        timing.invalidations,
        timing.shaped_lines,
        millis(timing.shape_time),
        timing.resolved_fonts,
        millis(timing.font_resolve_time),
        if timing.splice.is_empty() { "-" } else { timing.splice },
        timing.thread_cpu.map_or_else(|| "-".to_owned(), |cpu| format!("{:.3}", millis(cpu))),
        timing.page_faults.map_or_else(|| "-".to_owned(), |faults| faults.to_string()),
    )
}

struct PerfLogState {
    writer: BufWriter<File>,
    collector: FrameTimingCollector,
    draws_ms: Vec<f64>,
    anchor: (Instant, f64),
    view_totals: HashMap<&'static str, u64>,
    dirty_totals: HashMap<&'static Location<'static>, u64>,
    miss_totals: HashMap<(&'static str, &'static str), u64>,
}

impl PerfLogState {
    fn drain(&mut self) {
        for timing in self.collector.collect_unseen() {
            self.draws_ms.push(millis(timing.draw_duration()));
            let _ = self.writer.write_all(frame_line(&timing, self.anchor).as_bytes());
        }
        let epoch_ms = RiseClock::now().time_interval_since_1970() * 1000.0;
        if let Some(line) = views_by_type_line(epoch_ms, &gpui::view_render_counts(), &mut self.view_totals) {
            let _ = self.writer.write_all(line.as_bytes());
        }
        if let Some(line) = dirty_by_line(epoch_ms, &gpui::dirty_source_counts(), &mut self.dirty_totals) {
            let _ = self.writer.write_all(line.as_bytes());
        }
        if let Some(line) = miss_by_line(epoch_ms, &gpui::cache_miss_counts(), &mut self.miss_totals) {
            let _ = self.writer.write_all(line.as_bytes());
        }
        let _ = self.writer.flush();
    }
}

pub struct RisePerfLog {
    state: Arc<Mutex<PerfLogState>>,
    stopped: Arc<AtomicBool>,
}

impl Global for RisePerfLog {}

impl RisePerfLog {
    pub fn install_from_launch(cx: &mut App) {
        let Some(path) = perf_log_path(std::env::var_os(PERF_LOG_VARIABLE)) else {
            return;
        };
        let file = match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!(target: "riseonly", "perf log {} not opened: {error}", path.display());
                return;
            }
        };
        let mut writer = BufWriter::new(file);
        let _ = writeln!(writer, "{LOG_HEADER}");
        let _ = writer.flush();
        gpui::set_frame_trace_enabled(true);
        let anchor_instant = cx.background_executor().now();
        let anchor_epoch_ms = RiseClock::now().time_interval_since_1970() * 1000.0;
        let state = Arc::new(Mutex::new(PerfLogState {
            writer,
            collector: FrameTimingCollector::new(),
            draws_ms: Vec::new(),
            anchor: (anchor_instant, anchor_epoch_ms),
            view_totals: HashMap::new(),
            dirty_totals: HashMap::new(),
            miss_totals: HashMap::new(),
        }));
        let stopped = Arc::new(AtomicBool::new(false));
        Self::spawn_drain(state.clone(), stopped.clone(), cx);
        cx.on_app_quit(|cx| {
            if let Some(log) = cx.try_global::<Self>() {
                log.finish();
            }
            async {}
        })
        .detach();
        cx.set_global(Self { state, stopped });
    }

    fn spawn_drain(state: Arc<Mutex<PerfLogState>>, stopped: Arc<AtomicBool>, cx: &App) {
        let executor = cx.background_executor().clone();
        cx.background_executor()
            .spawn(async move {
                while !stopped.load(Ordering::Acquire) {
                    executor.timer(DRAIN_INTERVAL).await;
                    if let Ok(mut state) = state.lock() {
                        state.drain();
                    }
                }
            })
            .detach();
    }

    /// writes the frames still buffered and the summary of the whole run
    fn finish(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(mut state) = self.state.lock() {
            state.drain();
            let summary = PerfSummary::of(&state.draws_ms).line();
            let _ = writeln!(state.writer, "{summary}");
            let _ = state.writer.flush();
        }
    }
}
