use itertools::Itertools;
use scheduler::{Instant, SpawnTime};
use std::{
    cell::{Cell, LazyCell},
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::ThreadId,
    time::Duration,
};

mod actions;
pub use actions::{ActionStatistics, ActionTiming, take_action_stats};
pub(crate) use actions::{save_action_timing, update_running_action};

use serde::{Deserialize, Serialize};

use crate::{EntityId, SharedString, TasksIncluded, WindowId};

#[cfg(feature = "profiler")]
#[doc(hidden)]
pub fn get_all_timings(included: gpui::TasksIncluded) -> Vec<gpui::ThreadTaskTimings> {
    ThreadTaskTimings::collect(upgraded_thread_timings(), included)
}

#[cfg(feature = "profiler")]
#[doc(hidden)]
pub fn get_current_thread_timings(included: TasksIncluded) -> gpui::ThreadTaskTimings {
    gpui::profiler::get_current_thread_task_timings(included)
}

#[cfg(feature = "profiler")]
#[doc(hidden)]
pub fn take_all_stats(included: TasksIncluded) -> Vec<gpui::ThreadTaskStatistics> {
    ThreadTaskStatistics::collect_and_reset(upgraded_thread_timings(), included)
}

#[cfg(not(feature = "profiler"))]
#[doc(hidden)]
pub fn get_all_timings(_included: gpui::TasksIncluded) -> Vec<gpui::ThreadTaskTimings> {
    Vec::new()
}
#[cfg(not(feature = "profiler"))]
#[doc(hidden)]
pub fn get_current_thread_timings(_included: TasksIncluded) -> gpui::ThreadTaskTimings {
    gpui::ThreadTaskTimings {
        thread_name: None,
        thread_id: std::thread::current().id(),
        timings: Vec::new(),
        stats: TaskStatistics::default(),
        total_pushed: 0,
    }
}
#[cfg(not(feature = "profiler"))]
#[doc(hidden)]
pub fn take_all_stats(_included: TasksIncluded) -> Vec<gpui::ThreadTaskStatistics> {
    Vec::new()
}

#[doc(hidden)]
#[derive(Debug, Copy, Clone)]
pub struct YieldTime(pub Instant);

#[doc(hidden)]
#[derive(Copy, Clone)]
pub struct TaskTiming {
    pub location: &'static core::panic::Location<'static>,
    pub spawned: SpawnTime,
    pub start: Instant,
    pub end: YieldTime,
}

impl std::fmt::Debug for TaskTiming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskTiming")
            .field("location", &self.location)
            .field("since_spawned", &self.spawned.0.elapsed())
            .field("last_poll_duration", &self.poll_duration())
            .field("total_runtime", &self.since_spawn())
            .finish()
    }
}

#[doc(hidden)]
#[derive(Debug, Copy, Clone)]
pub struct ActiveTiming {
    pub location: &'static core::panic::Location<'static>,
    pub spawned: SpawnTime,
    pub start: Instant,
}

impl TaskTiming {
    /// A task timing with a duration of zero. Any task will replace this in history.
    pub fn placeholder() -> Self {
        let now = Instant::now();
        Self {
            location: std::panic::Location::caller(),
            spawned: SpawnTime(now),
            start: now,
            end: YieldTime(now),
        }
    }

    #[inline(always)]
    pub fn poll_duration(&self) -> Duration {
        self.end.0 - self.start
    }

    #[inline(always)]
    fn since_spawn(&self) -> Duration {
        self.end.0 - self.spawned.0
    }
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ThreadTaskTimings {
    pub thread_name: Option<String>,
    pub thread_id: ThreadId,
    pub timings: Vec<TaskTiming>,
    pub stats: TaskStatistics,
    pub total_pushed: u64,
}

impl ThreadTaskTimings {
    /// Convert upgraded per-thread timings into their structured format.
    pub fn collect(
        timings: Vec<(ThreadId, Arc<GuardedTaskTimings>)>,
        included: TasksIncluded,
    ) -> Vec<Self> {
        timings
            .into_iter()
            .map(|(thread_id, timings)| {
                let timings = timings.lock();
                let thread_name = timings.thread_name.clone();
                let total_pushed = timings.total_pushed;
                let completed = &timings.timings;

                let mut vec = Vec::with_capacity(completed.len() + 1); // +1 for running task
                let (s1, s2) = completed.as_slices();
                vec.extend_from_slice(s1);
                vec.extend_from_slice(s2);
                if let TasksIncluded::CompletedAndRunning = included
                    && let Some(running) = timings.running
                {
                    vec.push(TaskTiming {
                        location: running.location,
                        spawned: running.spawned,
                        start: running.start,
                        end: YieldTime(Instant::now()),
                    })
                }

                ThreadTaskTimings {
                    thread_name,
                    thread_id,
                    timings: vec,
                    stats: timings.stats.clone(),
                    total_pushed,
                }
            })
            .collect()
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct ThreadTaskStatistics {
    pub thread_name: Option<String>,
    pub thread_id: ThreadId,
    pub stats: TaskStatistics,
}

impl ThreadTaskStatistics {
    pub fn collect_and_reset(
        timings: Vec<(ThreadId, Arc<GuardedTaskTimings>)>,
        include_running: TasksIncluded,
    ) -> Vec<Self> {
        timings
            .into_iter()
            .map(|(thread_id, timings)| {
                let mut timings = timings.lock();
                let thread_name = timings.thread_name.clone();

                let mut stats = std::mem::take(&mut timings.stats);
                if let TasksIncluded::CompletedAndRunning = include_running
                    && let Some(ActiveTiming {
                        location,
                        spawned,
                        start,
                    }) = timings.running
                {
                    let end = YieldTime(Instant::now());
                    let timing = TaskTiming {
                        location,
                        spawned,
                        start,
                        end,
                    };
                    stats.add_runtime(timing);
                    stats.add_yield_timing(timing);
                }

                Self {
                    thread_name,
                    thread_id,
                    stats,
                }
            })
            .collect()
    }
}

/// Serializable variant of [`core::panic::Location`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedLocation {
    /// Name of the source file
    pub file: SharedString,
    /// Line in the source file
    pub line: u32,
    /// Column in the source file
    pub column: u32,
}

impl From<&core::panic::Location<'static>> for SerializedLocation {
    fn from(value: &core::panic::Location<'static>) -> Self {
        SerializedLocation {
            file: value.file().into(),
            line: value.line(),
            column: value.column(),
        }
    }
}

/// Serializable variant of [`TaskTiming`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedTaskTiming {
    /// Location of the timing
    pub location: SerializedLocation,
    /// Time at which the measurement was reported in nanoseconds
    pub start: u128,
    /// Duration of the measurement in nanoseconds
    pub duration: u128,
}

impl SerializedTaskTiming {
    /// Convert an array of [`TaskTiming`] into their serializable format
    ///
    /// # Params
    ///
    /// `anchor` - [`Instant`] that should be earlier than all timings to use as base anchor
    pub fn convert(anchor: Instant, timings: &[TaskTiming]) -> Vec<SerializedTaskTiming> {
        let serialized = timings
            .iter()
            .map(|timing| {
                let start = timing.start.duration_since(anchor).as_nanos();
                let duration = timing.end.0.duration_since(timing.start).as_nanos();
                SerializedTaskTiming {
                    location: timing.location.into(),
                    start,
                    duration,
                }
            })
            .collect::<Vec<_>>();

        serialized
    }

    /// `anchor` - [`Instant`] that should be earlier than all timings to use as base anchor
    pub fn from(anchor: Instant, timing: TaskTiming) -> SerializedTaskTiming {
        let start = timing.start.duration_since(anchor).as_nanos();
        let duration = timing.end.0.duration_since(timing.start).as_nanos();
        SerializedTaskTiming {
            location: timing.location.into(),
            start,
            duration,
        }
    }
}

/// Serializable variant of [`ThreadTaskTimings`]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedThreadTaskTimings {
    /// Thread name
    pub thread_name: Option<String>,
    /// Hash of the thread id
    pub thread_id: u64,
    /// Timing records for this thread
    pub timings: Vec<SerializedTaskTiming>,
}

impl SerializedThreadTaskTimings {
    /// Convert [`ThreadTaskTimings`] into their serializable format
    ///
    /// # Params
    ///
    /// `anchor` - [`Instant`] that should be earlier than all timings to use as base anchor
    pub fn convert(anchor: Instant, timings: ThreadTaskTimings) -> SerializedThreadTaskTimings {
        let serialized_timings = SerializedTaskTiming::convert(anchor, &timings.timings);

        let mut hasher = DefaultHasher::new();
        timings.thread_id.hash(&mut hasher);
        let thread_id = hasher.finish();

        SerializedThreadTaskTimings {
            thread_name: timings.thread_name,
            thread_id,
            timings: serialized_timings,
        }
    }
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ThreadTimingsDelta {
    /// Hashed thread id
    pub thread_id: u64,
    /// Thread name, if known
    pub thread_name: Option<String>,
    /// New timings since the last call. If the circular buffer wrapped around
    /// since the previous poll, some entries may have been lost.
    pub new_timings: Vec<SerializedTaskTiming>,
}

/// Tracks which timing events have already been seen so that callers can request only unseen events.
#[doc(hidden)]
pub struct ProfilingCollector {
    startup_time: Instant,
    cursors: HashMap<ThreadId, u64>,
}

impl ProfilingCollector {
    pub fn new(startup_time: Instant) -> Self {
        Self {
            startup_time,
            cursors: HashMap::default(),
        }
    }

    pub fn startup_time(&self) -> Instant {
        self.startup_time
    }

    pub fn collect_unseen(
        &mut self,
        all_timings: Vec<ThreadTaskTimings>,
    ) -> Vec<ThreadTimingsDelta> {
        let mut deltas = Vec::with_capacity(all_timings.len());

        for thread in all_timings {
            let mut hasher = DefaultHasher::new();
            thread.thread_id.hash(&mut hasher);
            let hashed_id = hasher.finish();

            let prev_cursor = self.cursors.get(&thread.thread_id).copied().unwrap_or(0);
            let buffer_len = thread.timings.len() as u64;
            let buffer_start = thread.total_pushed.saturating_sub(buffer_len);

            let mut slice = if prev_cursor < buffer_start {
                // Cursor fell behind the buffer — some entries were evicted.
                // Return everything still in the buffer.
                thread.timings.as_slice()
            } else {
                let skip = (prev_cursor - buffer_start) as usize;
                &thread.timings[skip.min(thread.timings.len())..]
            };

            let cursor_advance = thread.total_pushed;
            self.cursors.insert(thread.thread_id, cursor_advance);

            if slice.is_empty() {
                continue;
            }

            let new_timings = SerializedTaskTiming::convert(self.startup_time, slice);

            deltas.push(ThreadTimingsDelta {
                thread_id: hashed_id,
                thread_name: thread.thread_name,
                new_timings,
            });
        }

        deltas
    }

    pub fn reset(&mut self) {
        self.cursors.clear();
    }
}

// Allow 16MiB of task timing entries.
// VecDeque grows by doubling its capacity when full, so keep this a power of 2 to avoid wasting
// memory.
#[cfg(feature = "profiler")]
const MAX_TASK_TIMINGS: usize = (16 * 1024 * 1024) / core::mem::size_of::<TaskTiming>();

#[doc(hidden)]
pub(crate) type TaskTimings = VecDeque<TaskTiming>;

#[doc(hidden)]
pub type GuardedTaskTimings = spin::Mutex<ThreadTimings>;

#[doc(hidden)]
pub struct GlobalThreadTimings {
    pub thread_id: ThreadId,
    pub timings: std::sync::Weak<GuardedTaskTimings>,
}

#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct TaskStatistics {
    pub poll_time_to_beat: Duration,
    pub runtime_to_beat: Duration,
    pub longest_poll_times: [TaskTiming; 5],
    pub longest_runtimes: [TaskTiming; 5],
}

impl std::fmt::Display for TaskStatistics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tasks that blocked the longest before yielding\n")?;
        for timing in self.longest_poll_times {
            f.write_fmt(format_args!(
                "{:<20} - {}:{}\n",
                format!("{:?}", timing.poll_duration()),
                timing.location.file(),
                timing.location.column()
            ))?;
        }
        f.write_str("Tasks that ran the longest\n")?;
        for timing in self.longest_runtimes {
            f.write_fmt(format_args!(
                "{:<20} - {}:{}\n",
                format!("{:?}", timing.since_spawn()),
                timing.location.file(),
                timing.location.column()
            ))?;
        }
        Ok(())
    }
}

impl Default for TaskStatistics {
    fn default() -> Self {
        Self {
            // Do not track polls that are not problematic
            // this keeps more calls on the fast path
            poll_time_to_beat: Duration::from_micros(100),
            runtime_to_beat: Duration::from_micros(100),
            longest_poll_times: [TaskTiming::placeholder(); 5],
            longest_runtimes: [TaskTiming::placeholder(); 5],
        }
    }
}

impl TaskStatistics {
    #[inline(always)]
    fn add_yield_timing(&mut self, task: TaskTiming) {
        let yielded_after = task.poll_duration();
        if yielded_after >= self.poll_time_to_beat {
            std::hint::cold_path(); // most tasks are not the worst, optimize for that
            let to_replace = self
                .longest_poll_times
                .iter()
                .position_min_by_key(|task| task.since_spawn())
                .expect("guarded by the comparison with nth_longest_yield_time");
            self.longest_poll_times[to_replace] = task;

            self.poll_time_to_beat = self
                .longest_poll_times
                .iter()
                .map(|task| task.since_spawn())
                .min()
                .expect("never empty");
        }
    }

    #[inline(always)]
    fn add_runtime(&mut self, task: TaskTiming) {
        let runtime = task.since_spawn();
        if runtime >= self.runtime_to_beat {
            std::hint::cold_path(); // most tasks are not the worst, optimize for that
            let to_replace = self
                .longest_runtimes
                .iter()
                .position_min_by_key(|task| task.since_spawn())
                .expect("guarded by the comparison with nth_longest_yield_time");
            self.longest_runtimes[to_replace] = task;

            self.runtime_to_beat = self
                .longest_runtimes
                .iter()
                .map(|task| task.since_spawn())
                .min()
                .expect("never empty");
        }
    }
}

#[doc(hidden)]
pub static GLOBAL_THREAD_TIMINGS: spin::Mutex<Vec<GlobalThreadTimings>> =
    spin::Mutex::new(Vec::new());

/// Upgrades all live per-thread timing handles, holding the global registry
/// lock only for the duration of the upgrades.
///
/// The upgraded `Arc`s must never be dropped while `GLOBAL_THREAD_TIMINGS` is
/// locked: dropping the last strong reference runs [`ThreadTimings::drop`],
/// which locks `GLOBAL_THREAD_TIMINGS` again and would deadlock the
/// non-reentrant spinlock. A thread exiting concurrently can hand off its last
/// reference to us at any time, so callers of this function process (lock,
/// read, drop) the returned handles only after the global lock is released.
fn upgraded_thread_timings() -> Vec<(ThreadId, Arc<GuardedTaskTimings>)> {
    let global_thread_timings = GLOBAL_THREAD_TIMINGS.lock();
    global_thread_timings
        .iter()
        .filter_map(|t| Some((t.thread_id, t.timings.upgrade()?)))
        .collect()
}

thread_local! {
    #[doc(hidden)]
    pub static THREAD_TIMINGS: LazyCell<Arc<GuardedTaskTimings>> = LazyCell::new(|| {
        let current_thread = std::thread::current();
        let thread_name = current_thread.name();
        let thread_id = current_thread.id();
        let timings = ThreadTimings::new(thread_name.map(|e| e.to_string()), thread_id);
        let timings = Arc::new(spin::Mutex::new(timings));

        {
            let timings = Arc::downgrade(&timings);
            let global_timings = GlobalThreadTimings {
                thread_id: std::thread::current().id(),
                timings,
            };
            GLOBAL_THREAD_TIMINGS.lock().push(global_timings);
        }

        timings
    });
}

#[doc(hidden)]
pub struct ThreadTimings {
    pub thread_name: Option<String>,
    pub thread_id: ThreadId,
    pub timings: TaskTimings,
    pub running: Option<ActiveTiming>,
    pub stats: TaskStatistics,
    pub total_pushed: u64,
}

impl ThreadTimings {
    pub fn new(thread_name: Option<String>, thread_id: ThreadId) -> Self {
        ThreadTimings {
            thread_name,
            thread_id,
            timings: TaskTimings::new(),
            stats: TaskStatistics::default(),
            total_pushed: 0,
            running: None,
        }
    }

    #[cfg(feature = "profiler")]
    pub fn update_running_task(
        &mut self,
        spawned: SpawnTime,
        location: &'static std::panic::Location<'_>,
    ) {
        let start = Instant::now();
        self.running = Some(ActiveTiming {
            spawned,
            location,
            start,
        });
    }
    #[cfg(not(feature = "profiler"))]
    pub fn update_running_task(&mut self, _: SpawnTime, _: &'static std::panic::Location<'_>) {}

    #[cfg(feature = "profiler")]
    pub fn save_task_timing(&mut self, ended: YieldTime) {
        let ActiveTiming {
            location,
            start,
            spawned,
        } = self
            .running
            .take()
            .expect("this function is only ever called after register_task_start");

        let timing = TaskTiming {
            location,
            spawned,
            start,
            end: ended,
        };
        self.stats.add_yield_timing(timing);
        self.stats.add_runtime(timing);

        if trace_enabled() {
            std::hint::cold_path(); // optimize for when the profiling is off
            if self.timings.len() >= MAX_TASK_TIMINGS {
                self.timings.pop_front();
            }
            self.timings.push_back(timing);
            self.total_pushed += 1;
        }
    }
    #[cfg(not(feature = "profiler"))]
    pub fn save_task_timing(&mut self, _: YieldTime) {}

    // Running tasks are included in the reliability trace, which is written
    // whenever the foreground executor makes no progress for > n seconds
    pub fn get_thread_task_timings(&self, includes: TasksIncluded) -> ThreadTaskTimings {
        ThreadTaskTimings {
            thread_name: self.thread_name.clone(),
            thread_id: self.thread_id,
            timings: self
                .timings
                .iter()
                .cloned()
                .chain(
                    self.running
                        .filter(|_| matches!(includes, TasksIncluded::CompletedAndRunning))
                        .map(|running| TaskTiming {
                            spawned: running.spawned,
                            location: running.location,
                            start: running.start,
                            end: YieldTime(Instant::now()),
                        }),
                )
                .collect(),
            stats: self.stats.clone(),
            total_pushed: self.total_pushed,
        }
    }
}

impl Drop for ThreadTimings {
    fn drop(&mut self) {
        let mut thread_timings = GLOBAL_THREAD_TIMINGS.lock();

        let Some((index, _)) = thread_timings
            .iter()
            .enumerate()
            .find(|(_, t)| t.thread_id == self.thread_id)
        else {
            return;
        };
        thread_timings.swap_remove(index);
    }
}

#[doc(hidden)]
pub fn update_running_task(spawned: SpawnTime, location: &'static std::panic::Location<'_>) {
    THREAD_TIMINGS.with(|timings| {
        timings.lock().update_running_task(spawned, location);
    });
}

#[doc(hidden)]
pub fn save_task_timing() {
    let yielded_at = YieldTime(Instant::now());
    THREAD_TIMINGS.with(|timings| {
        timings.lock().save_task_timing(yielded_at);
    });
}

#[doc(hidden)]
pub fn get_current_thread_task_timings(include_running: TasksIncluded) -> ThreadTaskTimings {
    THREAD_TIMINGS.with(|timings| timings.lock().get_thread_task_timings(include_running))
}

static PROFILER_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enables or disables task timing trace collection at runtime.
///
/// When transitioning from enabled to disabled, `add_task_timing` becomes a
/// cheaper since only cheap statistics are gathered. The existing per-thread
/// buffers for traces are cleared so stale data isn't reported after a later
/// re-enable. Calls with the current value are a no-op.
pub fn set_trace_enabled(enabled: bool) -> bool {
    if PROFILER_ENABLED.swap(enabled, Ordering::AcqRel) == enabled {
        return false;
    }

    if !enabled {
        for (_, timings) in upgraded_thread_timings() {
            let mut timings = timings.lock();
            timings.timings.clear();
            timings.timings.shrink_to_fit();
            timings.total_pushed = 0;
        }
    }
    true
}

/// Returns whether task timing tracing is enabled.
pub fn trace_enabled() -> bool {
    PROFILER_ENABLED.load(Ordering::Relaxed)
}

/// Timing for a single drawn window frame.
#[derive(Debug, Copy, Clone)]
pub struct FrameTiming {
    /// The window that was drawn.
    pub window_id: WindowId,
    /// When the frame first became dirty (its first invalidation). `None` if
    /// frame tracing was not yet enabled when the invalidation occurred.
    pub dirty_at: Option<Instant>,
    /// Number of invalidations coalesced into this frame.
    pub invalidations: u64,
    /// When `Window::draw` started.
    pub draw_start: Instant,
    /// When `Window::draw` finished.
    pub draw_end: Instant,
    /// When the paint phase started; `None` when the draw painted nothing.
    pub paint_start: Option<Instant>,
    /// When the paint phase finished.
    pub paint_end: Option<Instant>,
    /// Views whose `render` ran in this frame; a cached view that was reused does not count.
    pub rendered_views: u32,
    /// Riseonly 0081: lines shaped by the platform text system in this frame (line cache misses).
    pub shaped_lines: u32,
    /// Riseonly 0081: time spent shaping those lines.
    pub shape_time: Duration,
    /// Riseonly 0081: fonts resolved by the platform for the first time in this frame.
    pub resolved_fonts: u32,
    /// Riseonly 0081: time spent resolving those fonts.
    pub font_resolve_time: Duration,
    /// Riseonly 0090: how the frame was drawn with respect to the 0088 splice: `spliced`, or why
    /// the full draw ran (`refresh`, `root`, `no_notify`, `unspliceable`, `viewport`, `a11y`,
    /// `prompt`, `drag`, `inspector`, `no_hole:<view type>`, `order`, `size:<view type>`); empty
    /// when no draw pass ran.
    pub splice: &'static str,
    /// Riseonly 0092: CPU time the drawing thread consumed inside `Window::draw`; `None` where the
    /// platform has no per-thread clock. Far below the draw duration means the thread waited
    /// (a lock, a page fault, the scheduler) instead of working.
    pub thread_cpu: Option<Duration>,
    /// Riseonly 0092: page faults (minor and major) the whole process took during the draw.
    pub page_faults: Option<u64>,
}

impl FrameTiming {
    /// Time spent inside `Window::draw`.
    pub fn draw_duration(&self) -> Duration {
        self.draw_end.duration_since(self.draw_start)
    }

    /// Render, request_layout and prepaint: from the start of the draw to the start of paint.
    pub fn layout_duration(&self) -> Option<Duration> {
        self.paint_start
            .map(|paint_start| paint_start.duration_since(self.draw_start))
    }

    /// Time spent painting the frame.
    pub fn paint_duration(&self) -> Option<Duration> {
        Some(self.paint_end?.duration_since(self.paint_start?))
    }

    /// Time from the frame's first invalidation to the end of its draw, if the
    /// first invalidation was observed.
    pub fn dirty_to_draw_duration(&self) -> Option<Duration> {
        self.dirty_at
            .map(|dirty_at| self.draw_end.duration_since(dirty_at))
    }
}

// Allow 16MiB of frame timing entries.
const MAX_FRAME_TIMINGS: usize = (16 * 1024 * 1024) / core::mem::size_of::<FrameTiming>();

struct FrameTimings {
    timings: VecDeque<FrameTiming>,
    total_pushed: u64,
}

static FRAME_TIMINGS: spin::Mutex<FrameTimings> = spin::Mutex::new(FrameTimings {
    timings: VecDeque::new(),
    total_pushed: 0,
});

static FRAME_TRACE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Phase marks of the frame being drawn on this thread.
#[derive(Debug, Default, Copy, Clone)]
pub(crate) struct FramePhases {
    pub(crate) paint_start: Option<Instant>,
    pub(crate) paint_end: Option<Instant>,
    pub(crate) rendered_views: u32,
    pub(crate) shaped_lines: u32,
    pub(crate) shape_time: Duration,
    pub(crate) resolved_fonts: u32,
    pub(crate) font_resolve_time: Duration,
    pub(crate) splice: Option<&'static str>,
}

/// Riseonly 0090: the splice outcome of the frame being drawn on this thread (see
/// [`FrameTiming::splice`]). No-op unless frame tracing is enabled.
pub(crate) fn note_splice(outcome: &'static str) {
    update_frame_phases(|phases| phases.splice = Some(outcome));
}

static VIEW_TYPES: spin::Mutex<Option<HashMap<EntityId, &'static str>>> = spin::Mutex::new(None);

/// Riseonly 0090: remembers the `Render` type of a view that rendered, so a splice refusal can name
/// the notified view. No-op unless frame tracing is enabled.
pub(crate) fn note_view_type(view_id: EntityId, type_name: &'static str) {
    if !frame_trace_enabled() {
        return;
    }
    VIEW_TYPES
        .lock()
        .get_or_insert_with(HashMap::default)
        .insert(view_id, type_name);
}

/// Riseonly 0090: the `Render` type a view last rendered as, `?` when unknown.
pub(crate) fn view_type_name(view_id: EntityId) -> &'static str {
    VIEW_TYPES
        .lock()
        .as_ref()
        .and_then(|types| types.get(&view_id).copied())
        .unwrap_or("?")
}

static NOTIFY_SITES: spin::Mutex<Option<HashMap<EntityId, &'static std::panic::Location<'static>>>> =
    spin::Mutex::new(None);

static ROOT_REASONS: spin::Mutex<Option<HashMap<(&'static str, &'static std::panic::Location<'static>), &'static str>>> =
    spin::Mutex::new(None);

thread_local! {
    static ROOT_DIRTY_REASON: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// Riseonly 0103: remembers where an entity was last notified (a relay counts under its
/// origin, 0051). No-op unless frame tracing is enabled.
pub(crate) fn note_notify_site(entity: EntityId, location: &'static std::panic::Location<'static>) {
    if !frame_trace_enabled() {
        return;
    }
    std::hint::cold_path();
    let location = DIRTY_ORIGIN.with(|origin| origin.get()).unwrap_or(location);
    NOTIFY_SITES
        .lock()
        .get_or_insert_with(HashMap::default)
        .insert(entity, location);
}

/// Riseonly 0103: the root read `entity` was notified; the frame's splice refusal becomes
/// `root:<view type or store>@file:line`. No-op unless frame tracing is enabled.
pub(crate) fn note_root_dirty(entity: EntityId) {
    if !frame_trace_enabled() {
        return;
    }
    let type_name = match view_type_name(entity) {
        "?" => "store",
        name => name.rsplit("::").next().unwrap_or(name),
    };
    let Some(location) = NOTIFY_SITES.lock().as_ref().and_then(|sites| sites.get(&entity).copied()) else {
        ROOT_DIRTY_REASON.with(|reason| reason.set(Some(detailed_reason("root", type_name))));
        return;
    };
    let mut reasons = ROOT_REASONS.lock();
    let reason = *reasons
        .get_or_insert_with(HashMap::default)
        .entry((type_name, location))
        .or_insert_with(|| {
            &*Box::leak(format!("root:{type_name}@{}:{}", location.file(), location.line()).into_boxed_str())
        });
    ROOT_DIRTY_REASON.with(|cell| cell.set(Some(reason)));
}

/// Riseonly 0103: the named root refusal of the frame being drawn, if any.
pub(crate) fn root_dirty_reason() -> Option<&'static str> {
    ROOT_DIRTY_REASON.with(|reason| reason.get())
}

/// Riseonly 0103: a drawn frame starts with no root refusal.
pub(crate) fn clear_root_dirty() {
    ROOT_DIRTY_REASON.with(|reason| reason.set(None));
}

/// Riseonly 0081: counts a line the platform text system shaped on this thread's frame.
/// No-op unless frame tracing is enabled.
pub(crate) fn note_text_shaped(elapsed: Duration) {
    update_frame_phases(|phases| {
        phases.shaped_lines = phases.shaped_lines.saturating_add(1);
        phases.shape_time += elapsed;
    });
}

/// Riseonly 0081: counts a font the platform resolved for the first time on this thread's frame.
/// No-op unless frame tracing is enabled.
pub(crate) fn note_font_resolved(elapsed: Duration) {
    update_frame_phases(|phases| {
        phases.resolved_fonts = phases.resolved_fonts.saturating_add(1);
        phases.font_resolve_time += elapsed;
    });
}

thread_local! {
    static FRAME_PHASES: Cell<FramePhases> = Cell::new(FramePhases::default());
}

fn update_frame_phases(update: impl FnOnce(&mut FramePhases)) {
    if !frame_trace_enabled() {
        return;
    }
    FRAME_PHASES.with(|phases| {
        let mut current = phases.get();
        update(&mut current);
        phases.set(current);
    });
}

static VIEW_RENDERS_BY_TYPE: spin::Mutex<Option<HashMap<&'static str, u64>>> = spin::Mutex::new(None);

/// Counts a view render of the frame being drawn, and its `Render` type in the run totals.
/// No-op unless frame tracing is enabled.
pub(crate) fn note_view_rendered(type_name: &'static str) {
    if !frame_trace_enabled() {
        return;
    }
    update_frame_phases(|phases| phases.rendered_views = phases.rendered_views.saturating_add(1));
    if let Some(reason) = PENDING_CACHE_MISS.with(|pending| pending.take()) {
        *CACHE_MISSES
            .lock()
            .get_or_insert_with(HashMap::default)
            .entry((type_name, reason))
            .or_insert(0) += 1;
    }
    let mut counts = VIEW_RENDERS_BY_TYPE.lock();
    *counts
        .get_or_insert_with(HashMap::default)
        .entry(type_name)
        .or_insert(0) += 1;
}

static DETAILED_REASONS: spin::Mutex<Option<HashMap<(&'static str, &'static str), &'static str>>> =
    spin::Mutex::new(None);

/// Riseonly 0086: `reason:detail` as one static reason for the miss counts. The pairs are few (a
/// reason, a text style field or an environment key name), so each is leaked once.
pub(crate) fn detailed_reason(reason: &'static str, detail: &'static str) -> &'static str {
    let mut reasons = DETAILED_REASONS.lock();
    *reasons
        .get_or_insert_with(HashMap::default)
        .entry((reason, detail))
        .or_insert_with(|| &*Box::leak(format!("{reason}:{detail}").into_boxed_str()))
}

static CACHE_MISSES: spin::Mutex<Option<HashMap<(&'static str, &'static str), u64>>> = spin::Mutex::new(None);

thread_local! {
    static PENDING_CACHE_MISS: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// Riseonly 0070: the reason the cached view about to render missed its replay; the render that
/// follows counts it under its type. No-op unless frame tracing is enabled.
pub(crate) fn note_cache_miss(reason: &'static str) {
    if !frame_trace_enabled() {
        return;
    }
    PENDING_CACHE_MISS.with(|pending| pending.set(Some(reason)));
}

/// Riseonly 0070: cached view renders per type and miss reason since frame tracing was enabled,
/// the most frequent first.
pub fn cache_miss_counts() -> Vec<((&'static str, &'static str), u64)> {
    let counts = CACHE_MISSES.lock();
    let mut counts: Vec<((&'static str, &'static str), u64)> = counts
        .as_ref()
        .map(|counts| counts.iter().map(|(key, count)| (*key, *count)).collect())
        .unwrap_or_default();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    counts
}

static DIRTY_SOURCES: spin::Mutex<Option<HashMap<&'static std::panic::Location<'static>, u64>>> =
    spin::Mutex::new(None);

/// Riseonly 0048: counts a notify, refresh or animation frame request by its call site in the run
/// totals. No-op unless frame tracing is enabled.
pub(crate) fn note_dirty_source(location: &'static std::panic::Location<'static>) {
    if !frame_trace_enabled() {
        return;
    }
    std::hint::cold_path();
    let location = DIRTY_ORIGIN.with(|origin| origin.get()).unwrap_or(location);
    let mut counts = DIRTY_SOURCES.lock();
    *counts
        .get_or_insert_with(HashMap::default)
        .entry(location)
        .or_insert(0) += 1;
}

thread_local! {
    static DIRTY_ORIGIN: std::cell::Cell<Option<&'static std::panic::Location<'static>>> =
        const { std::cell::Cell::new(None) };
}

/// Riseonly 0051: runs `f` with every dirty source it notes counted under `origin`. Relays (an
/// animation frame request delivered on the next frame, a keyed state notifying its host) use it
/// so the trace names the code that asked, not the relay inside gpui.
pub(crate) fn with_dirty_origin<R>(origin: &'static std::panic::Location<'static>, f: impl FnOnce() -> R) -> R {
    if !frame_trace_enabled() {
        return f();
    }
    let previous = DIRTY_ORIGIN.with(|cell| cell.replace(Some(origin)));
    let result = f();
    DIRTY_ORIGIN.with(|cell| cell.set(previous));
    result
}

/// Notifies, refreshes and animation frame requests per call site since frame tracing was
/// enabled, the most frequent first.
pub fn dirty_source_counts() -> Vec<(&'static std::panic::Location<'static>, u64)> {
    let counts = DIRTY_SOURCES.lock();
    let mut counts: Vec<(&'static std::panic::Location<'static>, u64)> = counts
        .as_ref()
        .map(|counts| counts.iter().map(|(location, count)| (*location, *count)).collect())
        .unwrap_or_default();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    counts
}

/// Renders per view type since frame tracing was enabled, the most rendered first.
pub fn view_render_counts() -> Vec<(&'static str, u64)> {
    let counts = VIEW_RENDERS_BY_TYPE.lock();
    let mut counts: Vec<(&'static str, u64)> = counts
        .as_ref()
        .map(|counts| counts.iter().map(|(name, count)| (*name, *count)).collect())
        .unwrap_or_default();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    counts
}

/// Marks the start of the paint phase. No-op unless frame tracing is enabled.
pub(crate) fn note_paint_start() {
    update_frame_phases(|phases| phases.paint_start = Some(Instant::now()));
}

/// Marks the end of the paint phase. No-op unless frame tracing is enabled.
pub(crate) fn note_paint_end() {
    update_frame_phases(|phases| phases.paint_end = Some(Instant::now()));
}

/// Returns the phase marks of the frame drawn on this thread and clears them for the next one.
pub(crate) fn take_frame_phases() -> FramePhases {
    FRAME_PHASES.with(|phases| phases.replace(FramePhases::default()))
}

/// Riseonly 0092: the calling thread's CPU clock and the process page fault count, read at the start
/// and the end of a traced draw.
#[derive(Debug, Copy, Clone)]
pub(crate) struct ThreadUsage {
    cpu: Option<Duration>,
    faults: Option<u64>,
}

impl ThreadUsage {
    pub(crate) fn now() -> Self {
        Self {
            cpu: thread_usage::thread_cpu_time(),
            faults: thread_usage::process_page_faults(),
        }
    }

    /// CPU time and page faults from `start` to `self`.
    pub(crate) fn since(self, start: Self) -> (Option<Duration>, Option<u64>) {
        let cpu = match (start.cpu, self.cpu) {
            (Some(start), Some(end)) => Some(end.saturating_sub(start)),
            _ => None,
        };
        let faults = match (start.faults, self.faults) {
            (Some(start), Some(end)) => Some(end.saturating_sub(start)),
            _ => None,
        };
        (cpu, faults)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "freebsd"))]
mod thread_usage {
    use std::ffi::{c_int, c_long};
    use std::time::Duration;

    #[repr(C)]
    struct Timespec {
        tv_sec: c_long,
        tv_nsec: c_long,
    }

    // why: struct rusage is two timevals (16 bytes each on 64-bit targets) and fourteen longs on
    // macOS, Linux and FreeBSD; only ru_minflt and ru_majflt are read.
    #[repr(C)]
    struct Rusage {
        times: [c_long; 4],
        counters: [c_long; 14],
    }

    unsafe extern "C" {
        fn clock_gettime(clock_id: c_int, tp: *mut Timespec) -> c_int;
        fn getrusage(who: c_int, usage: *mut Rusage) -> c_int;
    }

    #[cfg(target_os = "macos")]
    const CLOCK_THREAD_CPUTIME_ID: c_int = 16;
    #[cfg(target_os = "linux")]
    const CLOCK_THREAD_CPUTIME_ID: c_int = 3;
    #[cfg(target_os = "freebsd")]
    const CLOCK_THREAD_CPUTIME_ID: c_int = 14;
    const RUSAGE_SELF: c_int = 0;
    const RU_MINFLT: usize = 4;
    const RU_MAJFLT: usize = 5;

    pub(super) fn thread_cpu_time() -> Option<Duration> {
        let mut time = Timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `time` is a valid, writable timespec for the call.
        let status = unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut time) };
        if status != 0 {
            return None;
        }
        let seconds = u64::try_from(time.tv_sec).ok()?;
        let nanos = u32::try_from(time.tv_nsec).ok()?;
        Some(Duration::new(seconds, nanos))
    }

    pub(super) fn process_page_faults() -> Option<u64> {
        let mut usage = Rusage {
            times: [0; 4],
            counters: [0; 14],
        };
        // SAFETY: `usage` has the size and layout of struct rusage and is writable for the call.
        let status = unsafe { getrusage(RUSAGE_SELF, &mut usage) };
        if status != 0 {
            return None;
        }
        let minor = u64::try_from(usage.counters[RU_MINFLT]).ok()?;
        let major = u64::try_from(usage.counters[RU_MAJFLT]).ok()?;
        Some(minor.saturating_add(major))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
mod thread_usage {
    use std::time::Duration;

    pub(super) fn thread_cpu_time() -> Option<Duration> {
        None
    }

    pub(super) fn process_page_faults() -> Option<u64> {
        None
    }
}

/// Enables or disables frame timing collection at runtime.
///
/// When transitioning from enabled to disabled, the buffered frame timings are
/// cleared so stale data isn't reported after a later re-enable. Returns false
/// if the value was unchanged.
pub fn set_frame_trace_enabled(enabled: bool) -> bool {
    if FRAME_TRACE_ENABLED.swap(enabled, Ordering::AcqRel) == enabled {
        return false;
    }

    if !enabled {
        let mut frames = FRAME_TIMINGS.lock();
        frames.timings.clear();
        frames.timings.shrink_to_fit();
        frames.total_pushed = 0;
    }
    true
}

/// Returns whether frame timing collection is enabled.
pub fn frame_trace_enabled() -> bool {
    FRAME_TRACE_ENABLED.load(Ordering::Relaxed)
}

/// Records the timing of a drawn window frame.
///
/// No-op unless frame tracing is enabled via [`set_frame_trace_enabled`].
pub fn record_frame_timing(timing: FrameTiming) {
    if !frame_trace_enabled() {
        return;
    }
    std::hint::cold_path(); // optimize for when profiling is off

    let mut frames = FRAME_TIMINGS.lock();
    if frames.timings.len() >= MAX_FRAME_TIMINGS {
        frames.timings.pop_front();
    }
    frames.timings.push_back(timing);
    frames.total_pushed += 1;
}

/// Drains frame timings recorded after this collector was created, tracking a
/// cursor so each call to [`Self::collect_unseen`] returns only new entries.
pub struct FrameTimingCollector {
    cursor: u64,
}

impl Default for FrameTimingCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameTimingCollector {
    /// Creates a collector that only sees frames recorded from this point on.
    pub fn new() -> Self {
        Self {
            cursor: FRAME_TIMINGS.lock().total_pushed,
        }
    }

    /// Returns frame timings recorded since the previous call (or since the
    /// collector was created). If the ring buffer wrapped around since the
    /// previous poll, the evicted entries are lost.
    pub fn collect_unseen(&mut self) -> Vec<FrameTiming> {
        let frames = FRAME_TIMINGS.lock();
        let buffer_len = frames.timings.len() as u64;
        let buffer_start = frames.total_pushed.saturating_sub(buffer_len);
        let skip = self.cursor.saturating_sub(buffer_start) as usize;
        let unseen = frames
            .timings
            .iter()
            .skip(skip.min(frames.timings.len()))
            .copied()
            .collect();
        self.cursor = frames.total_pushed;
        unseen
    }
}
