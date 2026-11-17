use std::cell::Cell;
use std::collections::HashMap;

use gpui::{App, Window};
use rise_core::foundation::foundation_date::RiseClock;

use crate::modules::chat::components::chat::async_render::chat_performance_trace::{
    ChatPerformanceTrace, OperationToken,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_list_engine_policy::PostListEnginePolicy;

/// timed spans of the native post list
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostListTraceSpan {
    SnapshotPrepare,
    Commit,
    OpenSyncAnchorPrepare,
    OpenSyncFallbackMeasure,
    RowConfigureNative,
    LeafMount,
    FallbackMeasure,
}

impl PostListTraceSpan {
    pub const ALL_CASES: [PostListTraceSpan; 7] = [
        PostListTraceSpan::SnapshotPrepare,
        PostListTraceSpan::Commit,
        PostListTraceSpan::OpenSyncAnchorPrepare,
        PostListTraceSpan::OpenSyncFallbackMeasure,
        PostListTraceSpan::RowConfigureNative,
        PostListTraceSpan::LeafMount,
        PostListTraceSpan::FallbackMeasure,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            PostListTraceSpan::SnapshotPrepare => "post.snapshot.prepare",
            PostListTraceSpan::Commit => "post.commit",
            PostListTraceSpan::OpenSyncAnchorPrepare => "post.open.sync_anchor_prepare",
            PostListTraceSpan::OpenSyncFallbackMeasure => "post.open.sync_fallback_measure",
            PostListTraceSpan::RowConfigureNative => "post.row.configure.native",
            PostListTraceSpan::LeafMount => "post.leaf.mount",
            PostListTraceSpan::FallbackMeasure => "post.fallback.measure",
        }
    }
}

/// point events and counters of the native post list, counted by name in the session summary
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PostListTraceEvent {
    OpenFirstViewport,
    AspectLateUpdate,
    RevealBlocked,
    PageRequest,
    NativeRows,
    FallbackRows,
    LateHeightUpdates,
    OffsetWrites,
}

impl PostListTraceEvent {
    pub const ALL_CASES: [PostListTraceEvent; 8] = [
        PostListTraceEvent::OpenFirstViewport,
        PostListTraceEvent::AspectLateUpdate,
        PostListTraceEvent::RevealBlocked,
        PostListTraceEvent::PageRequest,
        PostListTraceEvent::NativeRows,
        PostListTraceEvent::FallbackRows,
        PostListTraceEvent::LateHeightUpdates,
        PostListTraceEvent::OffsetWrites,
    ];

    pub fn raw_value(self) -> &'static str {
        match self {
            PostListTraceEvent::OpenFirstViewport => "post.open.first_viewport_ms",
            PostListTraceEvent::AspectLateUpdate => "post.aspect.late_update",
            PostListTraceEvent::RevealBlocked => "post.reveal.blocked",
            PostListTraceEvent::PageRequest => "post.page.request",
            PostListTraceEvent::NativeRows => "post.native_rows",
            PostListTraceEvent::FallbackRows => "post.fallback_rows",
            PostListTraceEvent::LateHeightUpdates => "post.late_height_updates",
            PostListTraceEvent::OffsetWrites => "post.offset_writes",
        }
    }
}

pub struct PostListPerformanceTraceSpan {
    operation: Option<OperationToken>,
}

fn trace_fields<const N: usize>(pairs: [(&str, String); N]) -> HashMap<String, String> {
    pairs.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

/// post list spans on the chat performance trace
/// always tagged session_kind post_list so an overlapping chat trace stays separate
pub struct PostListPerformanceTrace {
    pub session: u64,
    pub is_enabled: bool,
    offset_write_count: Cell<i64>,
    native_row_count: Cell<i64>,
    fallback_row_count: Cell<i64>,
    late_height_update_count: Cell<i64>,
    owns_recording: Cell<bool>,
}

impl PostListPerformanceTrace {
    pub const SESSION_KIND: &'static str = "post_list";

    pub fn new(session: u64) -> Self {
        Self {
            session,
            is_enabled: PostListEnginePolicy::trace_enabled() && ChatPerformanceTrace::is_control_available(),
            offset_write_count: Cell::new(0),
            native_row_count: Cell::new(0),
            fallback_row_count: Cell::new(0),
            late_height_update_count: Cell::new(0),
            owns_recording: Cell::new(false),
        }
    }

    pub fn offset_write_count(&self) -> i64 {
        self.offset_write_count.get()
    }

    pub fn native_row_count(&self) -> i64 {
        self.native_row_count.get()
    }

    pub fn fallback_row_count(&self) -> i64 {
        self.fallback_row_count.get()
    }

    pub fn late_height_update_count(&self) -> i64 {
        self.late_height_update_count.get()
    }

    pub fn trace_chat_id(&self) -> String {
        format!("{}.{}", Self::SESSION_KIND, self.session)
    }

    pub fn is_recording(&self) -> bool {
        ChatPerformanceTrace::shared().is_recording()
    }

    pub fn start_recording_if_enabled(&self, window: &mut Window, cx: &mut App) {
        if !self.is_enabled || self.owns_recording.get() {
            return;
        }
        let started = ChatPerformanceTrace::shared().start(&self.trace_chat_id(), false, window, cx);
        self.owns_recording.set(started);
    }

    pub fn stop_recording(&self, reason: &str) {
        if !self.owns_recording.get() {
            return;
        }
        self.owns_recording.set(false);
        ChatPerformanceTrace::shared().stop_if_recording(&self.trace_chat_id(), reason);
    }

    pub fn begin(
        &self,
        kind: PostListTraceSpan,
        fields: impl FnOnce() -> HashMap<String, String>,
    ) -> Option<PostListPerformanceTraceSpan> {
        let recorder = ChatPerformanceTrace::shared();
        if !recorder.is_recording() {
            return None;
        }
        let operation = recorder.begin_operation(kind.raw_value(), self.tagged(fields()));
        Some(PostListPerformanceTraceSpan { operation })
    }

    pub fn end(
        &self,
        span: Option<PostListPerformanceTraceSpan>,
        threshold_ms: f64,
        fields: impl FnOnce() -> HashMap<String, String>,
    ) {
        let Some(span) = span else {
            return;
        };
        if span.operation.is_none() {
            return;
        }
        ChatPerformanceTrace::shared().end_operation(span.operation, threshold_ms, fields());
    }

    pub fn record(&self, event: PostListTraceEvent, fields: impl FnOnce() -> HashMap<String, String>) {
        let recorder = ChatPerformanceTrace::shared();
        if !recorder.is_recording() {
            return;
        }
        recorder.record(event.raw_value(), self.tagged(fields()));
    }

    pub fn count_offset_write(&self, reason: &str) {
        let total = self.offset_write_count.get() + 1;
        self.offset_write_count.set(total);
        self.record(PostListTraceEvent::OffsetWrites, || {
            trace_fields([("reason", reason.to_owned()), ("total", total.to_string())])
        });
    }

    pub fn count_inserted_rows(&self, native: i64, fallback: i64) {
        if native > 0 {
            let total = self.native_row_count.get() + native;
            self.native_row_count.set(total);
            self.record(PostListTraceEvent::NativeRows, || {
                trace_fields([("count", native.to_string()), ("total", total.to_string())])
            });
        }
        if fallback > 0 {
            let total = self.fallback_row_count.get() + fallback;
            self.fallback_row_count.set(total);
            self.record(PostListTraceEvent::FallbackRows, || {
                trace_fields([("count", fallback.to_string()), ("total", total.to_string())])
            });
        }
    }

    pub fn count_late_height_update(&self, delta: f32, is_visible: bool, reason: &str) {
        let total = self.late_height_update_count.get() + 1;
        self.late_height_update_count.set(total);
        self.record(PostListTraceEvent::LateHeightUpdates, || {
            trace_fields([
                ("delta", Self::decimal(f64::from(delta))),
                ("visible", if is_visible { "1" } else { "0" }.to_owned()),
                ("reason", reason.to_owned()),
                ("total", total.to_string()),
            ])
        });
    }

    pub fn record_first_viewport(&self, started_at: f64) {
        let elapsed = Self::milliseconds(started_at);
        self.record(PostListTraceEvent::OpenFirstViewport, || trace_fields([("ms", Self::decimal(elapsed))]));
    }

    pub fn record_reveal_blocked(&self, reason: &str) {
        self.record(PostListTraceEvent::RevealBlocked, || trace_fields([("reason", reason.to_owned())]));
    }

    pub fn record_page_request(&self, distance_from_bottom: f32, velocity_y: f32) {
        self.record(PostListTraceEvent::PageRequest, || {
            trace_fields([
                ("distance", Self::decimal(f64::from(distance_from_bottom))),
                ("velocity", Self::decimal(f64::from(velocity_y))),
            ])
        });
    }

    pub fn milliseconds(since_start: f64) -> f64 {
        ((RiseClock::monotonic().as_secs_f64() - since_start) * 1_000.0).max(0.0)
    }

    pub fn decimal(value: f64) -> String {
        if !value.is_finite() {
            return "0".to_owned();
        }
        format!("{value:.2}")
    }

    fn tagged(&self, mut fields: HashMap<String, String>) -> HashMap<String, String> {
        fields.insert("session_kind".to_owned(), Self::SESSION_KIND.to_owned());
        fields.insert("list_session".to_owned(), self.session.to_string());
        fields
    }
}
