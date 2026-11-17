use std::collections::HashMap;
use std::ops::Range;
use std::ptr;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use rise_canon::{rise_key, RiseLimits};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Value};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageThreadOrder, MessageThreadOrderKey,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessagePresentationSourceOrder {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessagePresentationWindowShift {
    Older,
    Newer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessagePresentationWindowState {
    pub revision: i64,
    pub generation: u64,
    pub source_order: MessagePresentationSourceOrder,
    pub canonical_count: i64,
    pub range: Range<i64>,
    pub anchor_id: Option<String>,
    pub anchor_offset: i64,
}

impl MessagePresentationWindowState {
    pub fn has_cached_before(&self) -> bool {
        self.range.start > 0
    }

    pub fn has_cached_after(&self) -> bool {
        self.range.end < self.canonical_count
    }
}

#[derive(Clone, Debug)]
pub struct MessagePresentationSlice<Element> {
    pub state: MessagePresentationWindowState,
    pub messages: Vec<Element>,
}

impl<Element> MessagePresentationSlice<Element> {
    pub fn has_cached_before(&self) -> bool {
        self.state.has_cached_before()
    }

    pub fn has_cached_after(&self) -> bool {
        self.state.has_cached_after()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessagePresentationDecodedRowTraceOutcome {
    ReferenceHit,
    StructuralHit,
    DecodedNew,
    DecodedChanged,
    DecodedWithoutIdentity,
    #[default]
    DecodeFailed,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MessagePresentationDecodedRowTraceSample {
    pub outcome: MessagePresentationDecodedRowTraceOutcome,
    pub total_duration_ms: f64,
    pub structural_comparison_duration_ms: f64,
    pub decoder_duration_ms: f64,
    pub snapshot_duration_ms: f64,
}

#[derive(Clone, Debug, Default)]
pub struct MessagePresentationDecodedRowTraceMetrics {
    lookups: i64,
    reference_hits: i64,
    structural_hits: i64,
    decoded_new: i64,
    decoded_changed: i64,
    decoded_without_identity: i64,
    decode_failures: i64,
    total_duration_ms: f64,
    structural_comparison_duration_ms: f64,
    decoder_duration_ms: f64,
    snapshot_duration_ms: f64,
}

impl MessagePresentationDecodedRowTraceMetrics {
    pub fn lookups(&self) -> i64 {
        self.lookups
    }

    pub fn reference_hits(&self) -> i64 {
        self.reference_hits
    }

    pub fn structural_hits(&self) -> i64 {
        self.structural_hits
    }

    pub fn decoded_new(&self) -> i64 {
        self.decoded_new
    }

    pub fn decoded_changed(&self) -> i64 {
        self.decoded_changed
    }

    pub fn decoded_without_identity(&self) -> i64 {
        self.decoded_without_identity
    }

    pub fn decode_failures(&self) -> i64 {
        self.decode_failures
    }

    pub fn total_duration_ms(&self) -> f64 {
        self.total_duration_ms
    }

    pub fn structural_comparison_duration_ms(&self) -> f64 {
        self.structural_comparison_duration_ms
    }

    pub fn decoder_duration_ms(&self) -> f64 {
        self.decoder_duration_ms
    }

    pub fn snapshot_duration_ms(&self) -> f64 {
        self.snapshot_duration_ms
    }

    pub fn record(&mut self, sample: MessagePresentationDecodedRowTraceSample) {
        self.lookups += 1;
        self.total_duration_ms += sample.total_duration_ms;
        self.structural_comparison_duration_ms += sample.structural_comparison_duration_ms;
        self.decoder_duration_ms += sample.decoder_duration_ms;
        self.snapshot_duration_ms += sample.snapshot_duration_ms;
        match sample.outcome {
            MessagePresentationDecodedRowTraceOutcome::ReferenceHit => self.reference_hits += 1,
            MessagePresentationDecodedRowTraceOutcome::StructuralHit => self.structural_hits += 1,
            MessagePresentationDecodedRowTraceOutcome::DecodedNew => self.decoded_new += 1,
            MessagePresentationDecodedRowTraceOutcome::DecodedChanged => self.decoded_changed += 1,
            MessagePresentationDecodedRowTraceOutcome::DecodedWithoutIdentity => self.decoded_without_identity += 1,
            MessagePresentationDecodedRowTraceOutcome::DecodeFailed => self.decode_failures += 1,
        }
    }

    pub fn trace_fields(&self) -> HashMap<String, String> {
        HashMap::from([
            ("lookups".to_owned(), self.lookups.to_string()),
            ("reference_hits".to_owned(), self.reference_hits.to_string()),
            ("structural_hits".to_owned(), self.structural_hits.to_string()),
            ("decoded_new".to_owned(), self.decoded_new.to_string()),
            ("decoded_changed".to_owned(), self.decoded_changed.to_string()),
            ("decoded_without_identity".to_owned(), self.decoded_without_identity.to_string()),
            ("decode_failures".to_owned(), self.decode_failures.to_string()),
            ("total_ms".to_owned(), Self::decimal(self.total_duration_ms)),
            ("structural_compare_ms".to_owned(), Self::decimal(self.structural_comparison_duration_ms)),
            ("decoder_ms".to_owned(), Self::decimal(self.decoder_duration_ms)),
            ("snapshot_ms".to_owned(), Self::decimal(self.snapshot_duration_ms)),
        ])
    }

    fn decimal(value: f64) -> String {
        format!("{value:.3}")
    }
}

#[derive(Default)]
struct MessagePresentationDecodedRowTraceBatchState {
    metrics: MessagePresentationDecodedRowTraceMetrics,
    flush_scheduled: bool,
}

#[derive(Default)]
struct MessagePresentationDecodedRowTraceBatch {
    state: Mutex<MessagePresentationDecodedRowTraceBatchState>,
}

impl MessagePresentationDecodedRowTraceBatch {
    fn record(self: &Arc<Self>, sample: MessagePresentationDecodedRowTraceSample) {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.metrics.record(sample);
            if state.flush_scheduled {
                return;
            }
            state.flush_scheduled = true;
        }
        match RiseEngineMainHop::installed() {
            Some(hop) => {
                let batch = Arc::downgrade(self);
                hop.post(move |_| {
                    if let Some(batch) = batch.upgrade() {
                        batch.flush();
                    }
                });
            }
            None => self.flush(),
        }
    }

    fn flush(&self) {
        let flushed = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.flush_scheduled = false;
            std::mem::take(&mut state.metrics)
        };
        if flushed.lookups() <= 0 || !ChatPerformanceTrace::shared().is_enabled() {
            return;
        }
        ChatPerformanceTrace::shared().record("presentation.row_cache.batch", flushed.trace_fields());
    }
}

#[derive(Clone)]
struct Entry<Element> {
    revision: i64,
    source_row: usize,
    row_snapshot: Map<String, Value>,
    value: Element,
    generation: u64,
}

#[derive(Clone)]
struct EvictionEntry {
    identity: String,
    generation: u64,
}

#[derive(Clone)]
pub struct MessagePresentationDecodedRowCache<Element> {
    capacity: usize,
    entries: HashMap<String, Entry<Element>>,
    eviction_order: Vec<EvictionEntry>,
    eviction_index: usize,
    next_generation: u64,
    trace_batch: Arc<MessagePresentationDecodedRowTraceBatch>,
}

impl<Element: Clone> MessagePresentationDecodedRowCache<Element> {
    pub fn new(capacity: i64) -> Self {
        Self {
            capacity: capacity.max(1) as usize,
            entries: HashMap::new(),
            eviction_order: Vec::new(),
            eviction_index: 0,
            next_generation: 0,
            trace_batch: Arc::new(MessagePresentationDecodedRowTraceBatch::default()),
        }
    }

    pub fn count(&self) -> i64 {
        self.entries.len() as i64
    }

    pub fn value(
        &mut self,
        row: &Map<String, Value>,
        revision: i64,
        decoder: impl FnOnce(&Map<String, Value>) -> Option<Element>,
    ) -> Option<Element> {
        let traces_performance = ChatPerformanceTrace::shared().is_enabled();
        let lookup_started_at = traces_performance.then(RiseClock::monotonic);
        let mut trace_sample = MessagePresentationDecodedRowTraceSample::default();
        let value = self.lookup(row, revision, decoder, traces_performance, &mut trace_sample);
        if traces_performance {
            trace_sample.total_duration_ms = elapsed_ms(lookup_started_at);
            self.trace_batch.record(trace_sample);
        }
        value
    }

    fn lookup(
        &mut self,
        row: &Map<String, Value>,
        revision: i64,
        decoder: impl FnOnce(&Map<String, Value>) -> Option<Element>,
        traces_performance: bool,
        trace_sample: &mut MessagePresentationDecodedRowTraceSample,
    ) -> Option<Element> {
        let Some(identity) = Self::identity(row) else {
            let decoder_started_at = traces_performance.then(RiseClock::monotonic);
            let value = decoder(row);
            if traces_performance {
                trace_sample.decoder_duration_ms = elapsed_ms(decoder_started_at);
                trace_sample.outcome = if value.is_none() {
                    MessagePresentationDecodedRowTraceOutcome::DecodeFailed
                } else {
                    MessagePresentationDecodedRowTraceOutcome::DecodedWithoutIdentity
                };
            }
            return value;
        };

        let source_row = ptr::from_ref(row) as usize;
        if let Some(entry) = self.entries.get_mut(&identity) {
            let is_reference_hit = entry.revision == revision && entry.source_row == source_row;
            let is_structurally_unchanged = if is_reference_hit {
                false
            } else {
                let comparison_started_at = traces_performance.then(RiseClock::monotonic);
                let is_equal = entry.row_snapshot == *row;
                if traces_performance {
                    trace_sample.structural_comparison_duration_ms = elapsed_ms(comparison_started_at);
                }
                is_equal
            };
            if is_reference_hit || is_structurally_unchanged {
                entry.revision = revision;
                entry.source_row = source_row;
                if traces_performance {
                    trace_sample.outcome = if is_reference_hit {
                        MessagePresentationDecodedRowTraceOutcome::ReferenceHit
                    } else {
                        MessagePresentationDecodedRowTraceOutcome::StructuralHit
                    };
                }
                return Some(entry.value.clone());
            }
        }

        let had_existing_entry = self.entries.contains_key(&identity);
        let decoder_started_at = traces_performance.then(RiseClock::monotonic);
        let Some(value) = decoder(row) else {
            if traces_performance {
                trace_sample.decoder_duration_ms = elapsed_ms(decoder_started_at);
                trace_sample.outcome = MessagePresentationDecodedRowTraceOutcome::DecodeFailed;
            }
            self.entries.remove(&identity);
            return None;
        };
        if traces_performance {
            trace_sample.decoder_duration_ms = elapsed_ms(decoder_started_at);
            trace_sample.outcome = if had_existing_entry {
                MessagePresentationDecodedRowTraceOutcome::DecodedChanged
            } else {
                MessagePresentationDecodedRowTraceOutcome::DecodedNew
            };
        }

        let generation = match self.entries.get(&identity) {
            Some(existing) => existing.generation,
            None => {
                self.next_generation = self.next_generation.wrapping_add(1);
                if self.next_generation == 0 {
                    self.entries.clear();
                    self.eviction_order.clear();
                    self.eviction_index = 0;
                    self.next_generation = 1;
                }
                let generation = self.next_generation;
                self.eviction_order.push(EvictionEntry { identity: identity.clone(), generation });
                generation
            }
        };
        let snapshot_started_at = traces_performance.then(RiseClock::monotonic);
        let row_snapshot = Self::snapshot(row);
        if traces_performance {
            trace_sample.snapshot_duration_ms = elapsed_ms(snapshot_started_at);
        }
        self.entries.insert(
            identity,
            Entry { revision, source_row, row_snapshot, value: value.clone(), generation },
        );
        self.evict_if_needed();
        Some(value)
    }

    pub fn remove_all(&mut self) {
        self.entries = HashMap::new();
        self.eviction_order = Vec::new();
        self.eviction_index = 0;
        self.next_generation = 0;
    }

    fn evict_if_needed(&mut self) {
        while self.entries.len() > self.capacity && self.eviction_index < self.eviction_order.len() {
            let candidate = &self.eviction_order[self.eviction_index];
            self.eviction_index += 1;
            if self.entries.get(&candidate.identity).map(|entry| entry.generation) != Some(candidate.generation) {
                continue;
            }
            self.entries.remove(&candidate.identity);
        }

        if self.eviction_index >= self.capacity && self.eviction_index * 2 >= self.eviction_order.len() {
            self.eviction_order.drain(..self.eviction_index);
            self.eviction_index = 0;
        }
    }

    fn identity(row: &Map<String, Value>) -> Option<String> {
        [rise_key::message::ID, RiseLocalKey::SERVER_ID, rise_key::message::TEMP_ID].into_iter().find_map(|key| {
            let normalized = MessagePresentationWindowProjector::normalized_string(row.get(key))?;
            Some(format!("{key}:{normalized}"))
        })
    }

    fn snapshot(dictionary: &Map<String, Value>) -> Map<String, Value> {
        dictionary.iter().map(|(key, value)| (key.clone(), Self::snapshot_value(value))).collect()
    }

    fn snapshot_value(value: &Value) -> Value {
        match value {
            Value::Object(dictionary) => Value::Object(Self::snapshot(dictionary)),
            Value::Array(array) => Value::Array(array.iter().map(Self::snapshot_value).collect()),
            other => other.clone(),
        }
    }
}

fn elapsed_ms(started_at: Option<Duration>) -> f64 {
    started_at.map_or(0.0, |started_at| RiseClock::monotonic().saturating_sub(started_at).as_secs_f64() * 1_000.0)
}

fn range_count(range: &Range<i64>) -> usize {
    (range.end - range.start).max(0) as usize
}

fn range_last(range: &Range<i64>) -> Option<i64> {
    (!range.is_empty()).then(|| range.end - 1)
}

static LIMIT: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("message_presentation_window_rows"));
static STRIDE: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("message_presentation_window_stride"));

pub enum MessagePresentationWindowProjector {}

impl MessagePresentationWindowProjector {
    pub fn limit() -> i64 {
        *LIMIT
    }

    pub fn stride() -> i64 {
        *STRIDE
    }

    pub fn preserving_content_generation<Element: PartialEq>(
        previous: MessagePresentationSlice<Element>,
        projected: MessagePresentationSlice<Element>,
    ) -> MessagePresentationSlice<Element> {
        if previous.messages != projected.messages {
            return projected;
        }
        let state = MessagePresentationWindowState {
            generation: previous.state.generation,
            ..projected.state
        };
        MessagePresentationSlice { state, messages: previous.messages }
    }

    pub fn is_at_latest_cached_boundary(state: &MessagePresentationWindowState) -> bool {
        state.canonical_count > 0 && state.range.end == state.canonical_count
    }

    pub fn normalize_anchor_id(value: Option<&str>) -> Option<String> {
        let mut value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() || value.starts_with("date-") {
            return None;
        }

        let mut removed_prefix = true;
        while removed_prefix {
            removed_prefix = false;
            for prefix in ["msg-", "sys-", "topic-"] {
                if let Some(stripped) = value.strip_prefix(prefix) {
                    value = stripped;
                    removed_prefix = true;
                    break;
                }
            }
        }

        let value = trimming_whitespaces_and_newlines(value);
        if value.is_empty() {
            return None;
        }
        Some(value.to_owned())
    }

    pub fn seed<Element>(
        rows: &[Value],
        revision: i64,
        saved_anchor_id: Option<&str>,
        row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        let order = Self::source_order(rows);
        let count = rows.len() as i64;
        let normalized_anchor = Self::normalize_anchor_id(saved_anchor_id);
        let anchor_index = normalized_anchor
            .as_deref()
            .and_then(|anchor| Self::canonical_index(anchor, rows, order, None));
        let range = match anchor_index {
            Some(anchor_index) => Self::bounded_range(anchor_index - Self::limit() / 2, count),
            None => Self::bounded_range(count - Self::limit(), count),
        };

        let resolved_anchor_index = anchor_index.or_else(|| range_last(&range));
        let resolved_anchor_id =
            resolved_anchor_index.and_then(|index| Self::canonical_row_id(index, rows, order));
        let anchor_offset = resolved_anchor_index.map_or(0, |index| (index - range.start).max(0));
        let state = MessagePresentationWindowState {
            revision,
            generation: 1,
            source_order: order,
            canonical_count: count,
            range,
            anchor_id: if anchor_index.is_none() { resolved_anchor_id } else { normalized_anchor },
            anchor_offset,
        };
        Self::project(rows, state, row_decoder)
    }

    pub fn shift_older<Element>(
        rows: &[Value],
        revision: i64,
        state: &MessagePresentationWindowState,
        visible_anchor_id: Option<&str>,
        row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        Self::shift(MessagePresentationWindowShift::Older, rows, revision, state, visible_anchor_id, row_decoder)
    }

    pub fn shift_older_slice<Element: Clone>(
        rows: &[Value],
        revision: i64,
        slice: &MessagePresentationSlice<Element>,
        visible_anchor_id: Option<&str>,
        row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        Self::shift_slice(MessagePresentationWindowShift::Older, rows, revision, slice, visible_anchor_id, row_decoder)
    }

    pub fn shift_newer<Element>(
        rows: &[Value],
        revision: i64,
        state: &MessagePresentationWindowState,
        visible_anchor_id: Option<&str>,
        row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        Self::shift(MessagePresentationWindowShift::Newer, rows, revision, state, visible_anchor_id, row_decoder)
    }

    pub fn shift_newer_slice<Element: Clone>(
        rows: &[Value],
        revision: i64,
        slice: &MessagePresentationSlice<Element>,
        visible_anchor_id: Option<&str>,
        row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        Self::shift_slice(MessagePresentationWindowShift::Newer, rows, revision, slice, visible_anchor_id, row_decoder)
    }

    pub fn reanchor<Element>(
        rows: &[Value],
        revision: i64,
        state: &MessagePresentationWindowState,
        anchor_id: Option<&str>,
        row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        let next_state =
            Self::reanchored_state(rows, revision, state, anchor_id, state.generation.wrapping_add(1));
        Self::project(rows, next_state, row_decoder)
    }

    pub fn latest<Element>(
        rows: &[Value],
        revision: i64,
        state: &MessagePresentationWindowState,
        row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        let order = Self::source_order(rows);
        let count = rows.len() as i64;
        let range = Self::bounded_range(count - Self::limit(), count);
        let anchor_index = range_last(&range);
        let anchor_offset = anchor_index.map_or(0, |index| (index - range.start).max(0));
        let next_state = MessagePresentationWindowState {
            revision,
            generation: state.generation.wrapping_add(1),
            source_order: order,
            canonical_count: count,
            range,
            anchor_id: anchor_index.and_then(|index| Self::canonical_row_id(index, rows, order)),
            anchor_offset,
        };
        Self::project(rows, next_state, row_decoder)
    }

    fn shift<Element>(
        direction: MessagePresentationWindowShift,
        rows: &[Value],
        revision: i64,
        state: &MessagePresentationWindowState,
        visible_anchor_id: Option<&str>,
        row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        let next_state = Self::shifted_state(direction, rows, revision, state, visible_anchor_id);
        Self::project(rows, next_state, row_decoder)
    }

    fn shift_slice<Element: Clone>(
        direction: MessagePresentationWindowShift,
        rows: &[Value],
        revision: i64,
        slice: &MessagePresentationSlice<Element>,
        visible_anchor_id: Option<&str>,
        mut row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        let next_state = Self::shifted_state(direction, rows, revision, &slice.state, visible_anchor_id);
        if revision != slice.state.revision
            || rows.len() as i64 != slice.state.canonical_count
            || next_state.source_order != slice.state.source_order
            || slice.messages.len() != range_count(&slice.state.range)
        {
            return Self::project(rows, next_state, row_decoder);
        }

        let trace_started_at = Self::projection_trace_start();
        let mut messages: Vec<Element> = Vec::with_capacity(range_count(&next_state.range));
        for canonical_index in next_state.range.clone() {
            if slice.state.range.contains(&canonical_index) {
                messages.push(slice.messages[(canonical_index - slice.state.range.start) as usize].clone());
                continue;
            }
            let raw_index = Self::raw_index(canonical_index, rows.len() as i64, next_state.source_order);
            if raw_index < 0 || raw_index >= rows.len() as i64 {
                continue;
            }
            let Some(message) = rows[raw_index as usize].as_object().and_then(&mut row_decoder) else {
                continue;
            };
            messages.push(message);
        }
        Self::record_projection_trace(
            trace_started_at,
            "incremental_shift",
            &next_state,
            range_count(&next_state.range) as i64,
            messages.len() as i64,
        );
        MessagePresentationSlice { state: next_state, messages }
    }

    fn shifted_state(
        direction: MessagePresentationWindowShift,
        rows: &[Value],
        revision: i64,
        state: &MessagePresentationWindowState,
        visible_anchor_id: Option<&str>,
    ) -> MessagePresentationWindowState {
        let order = Self::source_order(rows);
        let count = rows.len() as i64;
        let base_state = if revision != state.revision || count != state.canonical_count || order != state.source_order
        {
            Self::reanchored_state(rows, revision, state, visible_anchor_id, state.generation)
        } else {
            state.clone()
        };

        let normalized_visible_anchor = Self::normalize_anchor_id(visible_anchor_id);
        let preferred_anchor = normalized_visible_anchor.or_else(|| base_state.anchor_id.clone());
        let preferred_index = preferred_anchor
            .as_deref()
            .and_then(|anchor| Self::canonical_index(anchor, rows, order, Some(&base_state.range)));
        let retained_base_anchor_index = preferred_index.filter(|index| base_state.range.contains(index));
        let range = Self::shifted_range(direction, &base_state.range, count, retained_base_anchor_index);
        let retained_preferred_index = preferred_index.filter(|index| range.contains(index));
        let fallback_index = Self::retained_anchor_index(direction, &base_state.range, &range);
        let resolved_anchor_index = retained_preferred_index.or(fallback_index);
        let resolved_anchor_id = if retained_preferred_index.is_some() {
            preferred_anchor
        } else {
            resolved_anchor_index.and_then(|index| Self::canonical_row_id(index, rows, order))
        };
        let anchor_offset = resolved_anchor_index.map_or(0, |index| (index - range.start).max(0));
        MessagePresentationWindowState {
            revision,
            generation: state.generation.wrapping_add(1),
            source_order: order,
            canonical_count: count,
            range,
            anchor_id: resolved_anchor_id,
            anchor_offset,
        }
    }

    fn reanchored_state(
        rows: &[Value],
        revision: i64,
        state: &MessagePresentationWindowState,
        anchor_id: Option<&str>,
        generation: u64,
    ) -> MessagePresentationWindowState {
        let order = Self::source_order(rows);
        let count = rows.len() as i64;
        let normalized_anchor = Self::normalize_anchor_id(anchor_id).or_else(|| state.anchor_id.clone());
        let anchor_index = normalized_anchor
            .as_deref()
            .and_then(|anchor| Self::canonical_index(anchor, rows, order, Some(&state.range)));
        let range = if !state.has_cached_after() {
            Self::bounded_range(count - Self::limit(), count)
        } else if let Some(anchor_index) = anchor_index {
            Self::bounded_range(anchor_index - state.anchor_offset, count)
        } else {
            Self::bounded_range(state.range.start, count)
        };

        let fallback_anchor_index =
            (!range.is_empty()).then(|| (range.end - 1).min(range.start + state.anchor_offset));
        let resolved_anchor_index = anchor_index.or(fallback_anchor_index);
        let resolved_anchor_id = if anchor_index.is_none() {
            resolved_anchor_index.and_then(|index| Self::canonical_row_id(index, rows, order))
        } else {
            normalized_anchor
        };
        let anchor_offset = resolved_anchor_index.map_or(0, |index| (index - range.start).max(0));
        MessagePresentationWindowState {
            revision,
            generation,
            source_order: order,
            canonical_count: count,
            range,
            anchor_id: resolved_anchor_id,
            anchor_offset,
        }
    }

    fn project<Element>(
        rows: &[Value],
        state: MessagePresentationWindowState,
        mut row_decoder: impl FnMut(&Map<String, Value>) -> Option<Element>,
    ) -> MessagePresentationSlice<Element> {
        let trace_started_at = Self::projection_trace_start();
        let mut messages: Vec<Element> = Vec::with_capacity(range_count(&state.range));
        for canonical_index in state.range.clone() {
            let raw_index = Self::raw_index(canonical_index, rows.len() as i64, state.source_order);
            if raw_index < 0 || raw_index >= rows.len() as i64 {
                continue;
            }
            let Some(message) = rows[raw_index as usize].as_object().and_then(&mut row_decoder) else {
                continue;
            };
            messages.push(message);
        }
        Self::record_projection_trace(
            trace_started_at,
            "full",
            &state,
            range_count(&state.range) as i64,
            messages.len() as i64,
        );
        MessagePresentationSlice { state, messages }
    }

    fn projection_trace_start() -> Option<Duration> {
        ChatPerformanceTrace::shared().is_enabled().then(RiseClock::monotonic)
    }

    fn record_projection_trace(
        started_at: Option<Duration>,
        mode: &str,
        state: &MessagePresentationWindowState,
        requested_count: i64,
        result_count: i64,
    ) {
        if started_at.is_none() || !ChatPerformanceTrace::shared().is_enabled() {
            return;
        }
        let duration_ms = elapsed_ms(started_at);
        ChatPerformanceTrace::shared().record(
            "presentation.project",
            HashMap::from([
                ("mode".to_owned(), mode.to_owned()),
                ("revision".to_owned(), state.revision.to_string()),
                ("generation".to_owned(), state.generation.to_string()),
                ("requested".to_owned(), requested_count.to_string()),
                ("result".to_owned(), result_count.to_string()),
                (rise_key::otp::DURATION_MS.to_owned(), format!("{duration_ms:.3}")),
            ]),
        );
    }

    fn bounded_range(lower_bound: i64, count: i64) -> Range<i64> {
        if count <= 0 {
            return 0..0;
        }
        let length = Self::limit().min(count);
        let maximum_lower_bound = count - length;
        let lower_bound = lower_bound.max(0).min(maximum_lower_bound);
        lower_bound..(lower_bound + length)
    }

    fn shifted_range(
        direction: MessagePresentationWindowShift,
        range: &Range<i64>,
        count: i64,
        retained_anchor_index: Option<i64>,
    ) -> Range<i64> {
        if count <= 0 {
            return 0..0;
        }

        let requested_lower_bound = match direction {
            MessagePresentationWindowShift::Older => range.start - Self::stride(),
            MessagePresentationWindowShift::Newer => range.start + Self::stride(),
        };

        let length = Self::limit().min(count);
        let mut lower_bound = requested_lower_bound;
        if let Some(retained_anchor_index) = retained_anchor_index {
            lower_bound = lower_bound.min(retained_anchor_index);
            lower_bound = lower_bound.max(retained_anchor_index - length + 1);
        }
        lower_bound = match direction {
            MessagePresentationWindowShift::Older => lower_bound.min(range.start),
            MessagePresentationWindowShift::Newer => lower_bound.max(range.start),
        };
        Self::bounded_range(lower_bound, count)
    }

    fn retained_anchor_index(
        direction: MessagePresentationWindowShift,
        old_range: &Range<i64>,
        new_range: &Range<i64>,
    ) -> Option<i64> {
        let lower_bound = old_range.start.max(new_range.start);
        let upper_bound = old_range.end.min(new_range.end);
        if lower_bound >= upper_bound {
            return range_last(new_range);
        }
        match direction {
            MessagePresentationWindowShift::Older => Some(lower_bound),
            MessagePresentationWindowShift::Newer => Some(upper_bound - 1),
        }
    }

    fn source_order(rows: &[Value]) -> MessagePresentationSourceOrder {
        let mut previous_timestamp: Option<f64> = None;
        let mut previous_id: Option<String> = None;
        for row in rows {
            let Some(row) = row.as_object() else {
                continue;
            };
            let Some(timestamp) = Self::timestamp(row) else {
                continue;
            };
            let id = Self::source_order_id(row);
            if let Some(previous_timestamp) = previous_timestamp {
                if timestamp != previous_timestamp {
                    return if timestamp < previous_timestamp {
                        MessagePresentationSourceOrder::Descending
                    } else {
                        MessagePresentationSourceOrder::Ascending
                    };
                }
                if let (Some(previous_id), Some(id)) = (previous_id.as_ref(), id.as_ref())
                    && id != previous_id
                {
                    let current_precedes_previous = MessageThreadOrder::precedes(
                        &MessageThreadOrderKey { timestamp, id: id.clone() },
                        &MessageThreadOrderKey { timestamp: previous_timestamp, id: previous_id.clone() },
                    );
                    return if current_precedes_previous {
                        MessagePresentationSourceOrder::Descending
                    } else {
                        MessagePresentationSourceOrder::Ascending
                    };
                }
            }
            previous_timestamp = Some(timestamp);
            previous_id = id.or(previous_id);
        }
        MessagePresentationSourceOrder::Ascending
    }

    fn source_order_id(row: &Map<String, Value>) -> Option<String> {
        [RiseLocalKey::SERVER_ID, rise_key::message::ID, rise_key::message::TEMP_ID]
            .into_iter()
            .find_map(|key| Self::normalized_string(row.get(key)))
    }

    fn canonical_index(
        anchor_id: &str,
        rows: &[Value],
        order: MessagePresentationSourceOrder,
        preferred_range: Option<&Range<i64>>,
    ) -> Option<i64> {
        let count = rows.len() as i64;
        if let Some(preferred_range) = preferred_range {
            for canonical_index in preferred_range.start.max(0)..preferred_range.end.min(count) {
                let raw_index = Self::raw_index(canonical_index, count, order);
                let Some(row) = rows[raw_index as usize].as_object() else {
                    continue;
                };
                if Self::row_matches(anchor_id, row) {
                    return Some(canonical_index);
                }
            }
        }

        for (raw_index, row) in rows.iter().enumerate() {
            let Some(row) = row.as_object() else {
                continue;
            };
            if !Self::row_matches(anchor_id, row) {
                continue;
            }
            return Some(match order {
                MessagePresentationSourceOrder::Ascending => raw_index as i64,
                MessagePresentationSourceOrder::Descending => count - raw_index as i64 - 1,
            });
        }
        None
    }

    fn canonical_row_id(
        canonical_index: i64,
        rows: &[Value],
        order: MessagePresentationSourceOrder,
    ) -> Option<String> {
        let raw_index = Self::raw_index(canonical_index, rows.len() as i64, order);
        if raw_index < 0 || raw_index >= rows.len() as i64 {
            return None;
        }
        let row = rows[raw_index as usize].as_object()?;
        let value = [rise_key::message::ID, RiseLocalKey::SERVER_ID, rise_key::message::TEMP_ID]
            .into_iter()
            .find_map(|key| Self::normalized_string(row.get(key)))?;
        Self::normalize_anchor_id(Some(&value))
    }

    fn raw_index(canonical_index: i64, count: i64, order: MessagePresentationSourceOrder) -> i64 {
        match order {
            MessagePresentationSourceOrder::Ascending => canonical_index,
            MessagePresentationSourceOrder::Descending => count - canonical_index - 1,
        }
    }

    fn row_matches(anchor_id: &str, row: &Map<String, Value>) -> bool {
        [rise_key::message::ID, RiseLocalKey::SERVER_ID, rise_key::message::TEMP_ID].into_iter().any(|key| {
            Self::normalized_string(row.get(key))
                .is_some_and(|value| Self::normalize_anchor_id(Some(&value)).as_deref() == Some(anchor_id))
        })
    }

    fn timestamp(row: &Map<String, Value>) -> Option<f64> {
        [rise_key::message::CREATED_AT, rise_key::message::TIMESTAMP].into_iter().find_map(|key| {
            let value = match row.get(key)? {
                Value::Number(number) => number.as_f64()?,
                Value::String(text) => trimming_whitespaces_and_newlines(text).parse::<f64>().ok()?,
                _ => return None,
            };
            (value > 0.0).then_some(value)
        })
    }

    fn normalized_string(value: Option<&Value>) -> Option<String> {
        let string = match value? {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => return None,
        };
        let trimmed = trimming_whitespaces_and_newlines(&string);
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.to_owned())
    }
}
