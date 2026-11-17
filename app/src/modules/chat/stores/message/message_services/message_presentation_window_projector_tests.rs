use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::TestAppContext;
use rise_canon::rise_key;
use serde_json::{Map, Value};

use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::{
    ChatMessageRenderEntryKind, ChatMessageRenderSnapshot, ChatMessageRenderSnapshotBuilder,
};
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageFeedSourceVersionPolicy;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::chat::stores::message::message_services::message_presentation_window_projector::{
    MessagePresentationDecodedRowCache, MessagePresentationDecodedRowTraceMetrics,
    MessagePresentationDecodedRowTraceOutcome, MessagePresentationDecodedRowTraceSample, MessagePresentationSlice,
    MessagePresentationSourceOrder, MessagePresentationWindowProjector, MessagePresentationWindowState,
};

#[derive(Clone, Debug, PartialEq)]
struct ProjectedMessage {
    id: String,
    timestamp: i64,
}

fn row_map(id: &str, created_at: i64) -> Map<String, Value> {
    let mut row = Map::new();
    row.insert(rise_key::message::ID.to_owned(), Value::String(id.to_owned()));
    row.insert(rise_key::message::CREATED_AT.to_owned(), Value::from(created_at));
    row
}

fn make_rows(count: i64, descending: bool, leading_rows: i64) -> Vec<Value> {
    let mut rows = Vec::with_capacity((count + leading_rows) as usize);
    for index in 0..leading_rows {
        rows.push(Value::Object(row_map(&format!("older-{index:05}"), index + 1)));
    }
    let indices: Vec<i64> = if descending { (0..count).rev().collect() } else { (0..count).collect() };
    for index in indices {
        rows.push(Value::Object(row_map(&format!("m-{index:05}"), leading_rows + index + 1)));
    }
    rows
}

fn rows_of(count: i64) -> Vec<Value> {
    make_rows(count, false, 0)
}

fn decode(row: &Map<String, Value>) -> Option<ProjectedMessage> {
    let id = row.get(rise_key::message::ID)?.as_str()?.to_owned();
    let timestamp = row.get(rise_key::message::CREATED_AT)?.as_i64()?;
    Some(ProjectedMessage { id, timestamp })
}

fn first_id(slice: &MessagePresentationSlice<ProjectedMessage>) -> Option<&str> {
    slice.messages.first().map(|message| message.id.as_str())
}

fn last_id(slice: &MessagePresentationSlice<ProjectedMessage>) -> Option<&str> {
    slice.messages.last().map(|message| message.id.as_str())
}

fn window_state(
    revision: i64,
    generation: u64,
    canonical_count: i64,
    range: std::ops::Range<i64>,
    anchor_id: Option<&str>,
) -> MessagePresentationWindowState {
    MessagePresentationWindowState {
        revision,
        generation,
        source_order: MessagePresentationSourceOrder::Ascending,
        canonical_count,
        range,
        anchor_id: anchor_id.map(str::to_owned),
        anchor_offset: 0,
    }
}

fn projected(id: &str, timestamp: i64) -> ProjectedMessage {
    ProjectedMessage { id: id.to_owned(), timestamp }
}

#[test]
fn projects_only_bounded_middle_window_from_ten_thousand_rows() {
    let rows = rows_of(10_000);
    let mut decode_count = 0;
    let slice = MessagePresentationWindowProjector::seed(&rows, 7, Some("msg-m-05000"), |row| {
        decode_count += 1;
        decode(row)
    });

    assert_eq!(MessagePresentationWindowProjector::limit(), 320);
    assert_eq!(MessagePresentationWindowProjector::stride(), 96);
    assert_eq!(slice.state.range, 4_840..5_160);
    assert_eq!(slice.state.anchor_id.as_deref(), Some("m-05000"));
    assert_eq!(slice.state.anchor_offset, 160);
    assert_eq!(slice.state.canonical_count, 10_000);
    assert_eq!(slice.state.source_order, MessagePresentationSourceOrder::Ascending);
    assert!(slice.has_cached_before());
    assert!(slice.has_cached_after());
    assert_eq!(decode_count, 320);
    assert_eq!(slice.messages.len(), 320);
    assert_eq!(first_id(&slice), Some("m-04840"));
    assert_eq!(last_id(&slice), Some("m-05159"));
    assert_eq!(MessagePresentationWindowProjector::normalize_anchor_id(Some("sys-m-1")).as_deref(), Some("m-1"));
    assert_eq!(MessagePresentationWindowProjector::normalize_anchor_id(Some("topic-m-1")).as_deref(), Some("m-1"));
    assert_eq!(MessagePresentationWindowProjector::normalize_anchor_id(Some("msg-topic-m-1")).as_deref(), Some("m-1"));
}

#[test]
fn reverses_legacy_descending_rows_without_decoding_outside_window() {
    let rows = make_rows(10_000, true, 0);
    let mut decode_count = 0;
    let slice = MessagePresentationWindowProjector::seed(&rows, 8, Some("sys-m-05000"), |row| {
        decode_count += 1;
        decode(row)
    });

    assert_eq!(slice.state.source_order, MessagePresentationSourceOrder::Descending);
    assert_eq!(slice.state.range, 4_840..5_160);
    assert_eq!(slice.state.anchor_offset, 160);
    assert_eq!(decode_count, 320);
    assert_eq!(
        slice.messages.iter().map(|message| message.timestamp).collect::<Vec<_>>(),
        (4_841..=5_160).collect::<Vec<i64>>()
    );
    assert_eq!(first_id(&slice), Some("m-04840"));
    assert_eq!(last_id(&slice), Some("m-05159"));
}

#[test]
fn detects_source_order_from_ids_when_all_timestamps_match() {
    let canonical_rows: Vec<Value> = ["message-d", "message-c", "message-b", "message-a"]
        .into_iter()
        .map(|id| Value::Object(row_map(id, 42)))
        .collect();
    let cassandra_rows: Vec<Value> = canonical_rows.iter().rev().cloned().collect();

    let canonical = MessagePresentationWindowProjector::seed(&canonical_rows, 1, None, decode);
    let reversed = MessagePresentationWindowProjector::seed(&cassandra_rows, 2, None, decode);

    assert_eq!(canonical.state.source_order, MessagePresentationSourceOrder::Ascending);
    assert_eq!(reversed.state.source_order, MessagePresentationSourceOrder::Descending);
    assert_eq!(
        canonical.messages.iter().map(|message| message.id.as_str()).collect::<Vec<_>>(),
        vec!["message-d", "message-c", "message-b", "message-a"]
    );
    assert_eq!(reversed.messages, canonical.messages);
}

#[test]
fn shifts_older_and_newer_by_fixed_stride() {
    let rows = rows_of(10_000);
    let tail = MessagePresentationWindowProjector::seed(&rows, 9, None, decode);

    assert_eq!(tail.state.range, 9_680..10_000);
    assert!(tail.has_cached_before());
    assert!(!tail.has_cached_after());

    let mut older_decode_count = 0;
    let older = MessagePresentationWindowProjector::shift_older_slice(&rows, 9, &tail, Some("topic-m-09700"), |row| {
        older_decode_count += 1;
        decode(row)
    });

    assert_eq!(older.state.range, 9_584..9_904);
    assert!(older.has_cached_before());
    assert!(older.has_cached_after());
    assert_eq!(older.state.anchor_id.as_deref(), Some("m-09700"));
    assert_eq!(older.state.anchor_offset, 116);
    assert_eq!(first_id(&older), Some("m-09584"));
    assert_eq!(last_id(&older), Some("m-09903"));
    assert_eq!(older_decode_count, MessagePresentationWindowProjector::stride());

    let mut newer_decode_count = 0;
    let newer = MessagePresentationWindowProjector::shift_newer_slice(&rows, 9, &older, Some("msg-m-09800"), |row| {
        newer_decode_count += 1;
        decode(row)
    });

    assert_eq!(newer.state.range, 9_680..10_000);
    assert!(newer.has_cached_before());
    assert!(!newer.has_cached_after());
    assert_eq!(newer.state.anchor_id.as_deref(), Some("m-09800"));
    assert_eq!(newer.state.anchor_offset, 120);
    assert_eq!(newer.state.generation, tail.state.generation + 2);
    assert_eq!(newer_decode_count, MessagePresentationWindowProjector::stride());
}

#[test]
fn keeps_visible_anchor_inside_shifted_window() {
    let rows = rows_of(1_000);
    let middle = MessagePresentationWindowProjector::seed(&rows, 31, Some("m-00500"), decode);

    let shifted =
        MessagePresentationWindowProjector::shift_older_slice(&rows, 31, &middle, Some("msg-m-00650"), decode);

    assert!(shifted.state.range.contains(&650));
    assert_eq!(shifted.state.anchor_id.as_deref(), Some("m-00650"));
    assert!(shifted.messages.iter().any(|message| message.id == "m-00650"));
}

#[test]
fn reanchors_same_visible_row_after_older_rows_are_inserted() {
    let original_rows = rows_of(10_000);
    let original = MessagePresentationWindowProjector::seed(&original_rows, 10, Some("msg-m-05000"), decode);
    let revised_rows = make_rows(10_000, false, 200);
    let mut decode_count = 0;
    let revised =
        MessagePresentationWindowProjector::reanchor(&revised_rows, 11, &original.state, Some("topic-m-05000"), |row| {
            decode_count += 1;
            decode(row)
        });

    assert_eq!(revised.state.revision, 11);
    assert_eq!(revised.state.range, 5_040..5_360);
    assert_eq!(revised.state.anchor_id.as_deref(), Some("m-05000"));
    assert_eq!(revised.state.anchor_offset, original.state.anchor_offset);
    assert_eq!(revised.state.canonical_count, 10_200);
    assert!(revised.has_cached_before());
    assert!(revised.has_cached_after());
    assert_eq!(decode_count, 320);
    assert_eq!(first_id(&revised), first_id(&original));
    assert_eq!(last_id(&revised), last_id(&original));
}

#[test]
fn keeps_following_tail_when_revision_appends_incoming_row() {
    let original_rows = rows_of(10_000);
    let original = MessagePresentationWindowProjector::seed(&original_rows, 12, None, decode);
    let revised_rows = rows_of(10_001);
    let mut decode_count = 0;
    let revised = MessagePresentationWindowProjector::reanchor(&revised_rows, 13, &original.state, None, |row| {
        decode_count += 1;
        decode(row)
    });

    assert_eq!(original.state.range, 9_680..10_000);
    assert_eq!(revised.state.range, 9_681..10_001);
    assert_eq!(revised.state.anchor_id.as_deref(), Some("m-09999"));
    assert_eq!(revised.state.anchor_offset, 318);
    assert!(revised.has_cached_before());
    assert!(!revised.has_cached_after());
    assert_eq!(decode_count, 320);
    assert_eq!(first_id(&revised), Some("m-09681"));
    assert_eq!(last_id(&revised), Some("m-10000"));
}

#[test]
fn reuses_structurally_unchanged_rows_across_cache_revisions() {
    let mut cache = MessagePresentationDecodedRowCache::<ProjectedMessage>::new(512);
    let decode_count = Cell::new(0);
    let counting_decode = |row: &Map<String, Value>| {
        decode_count.set(decode_count.get() + 1);
        decode(row)
    };
    let original_rows = rows_of(10_000);
    let original = MessagePresentationWindowProjector::seed(&original_rows, 41, None, |row| {
        cache.value(row, 41, counting_decode)
    });
    let revised_rows = rows_of(10_001);
    let revised = MessagePresentationWindowProjector::reanchor(&revised_rows, 42, &original.state, None, |row| {
        cache.value(row, 42, counting_decode)
    });

    assert_eq!(original.messages.len(), 320);
    assert_eq!(revised.messages.len(), 320);
    assert_eq!(decode_count.get(), 321);
    assert_eq!(last_id(&revised), Some("m-10000"));
}

#[test]
fn preserves_presentation_generation_when_visible_content_is_unchanged() {
    let previous_state = window_state(3, 7, 100, 20..22, Some("a"));
    let projected_state = window_state(4, 8, 101, 21..23, Some("a"));
    let messages = vec![projected("a", 1), projected("b", 2)];
    let stabilized = MessagePresentationWindowProjector::preserving_content_generation(
        MessagePresentationSlice { state: previous_state, messages: messages.clone() },
        MessagePresentationSlice { state: projected_state, messages },
    );

    assert_eq!(stabilized.state.revision, 4);
    assert_eq!(stabilized.state.generation, 7);
    assert_eq!(stabilized.state.canonical_count, 101);
    assert_eq!(stabilized.state.range, 21..23);
}

#[test]
fn advances_presentation_generation_when_visible_content_changes() {
    let previous_state = window_state(3, 7, 2, 0..2, Some("a"));
    let projected_state = window_state(4, 8, 2, 0..2, Some("a"));
    let stabilized = MessagePresentationWindowProjector::preserving_content_generation(
        MessagePresentationSlice { state: previous_state, messages: vec![projected("a", 1)] },
        MessagePresentationSlice { state: projected_state, messages: vec![projected("a", 2)] },
    );

    assert_eq!(stabilized.state.generation, 8);
    assert_eq!(stabilized.messages.first().map(|message| message.timestamp), Some(2));
}

#[test]
fn bounded_feed_data_version_tracks_scope_content_revision() {
    let first_state = window_state(3, 7, 100, 0..100, None);
    let second_state = window_state(4, 7, 100, 0..100, None);
    let shifted_state = window_state(4, 7, 101, 1..101, None);

    assert_ne!(
        RiseMessageFeedSourceVersionPolicy::data_version(3, Some(&first_state)),
        RiseMessageFeedSourceVersionPolicy::data_version(4, Some(&second_state))
    );
    assert_ne!(
        RiseMessageFeedSourceVersionPolicy::data_version(4, Some(&second_state)),
        RiseMessageFeedSourceVersionPolicy::data_version(4, Some(&shifted_state))
    );
    assert_ne!(
        RiseMessageFeedSourceVersionPolicy::data_version(3, None),
        RiseMessageFeedSourceVersionPolicy::data_version(4, None)
    );
}

#[test]
fn decodes_changed_rows_and_keeps_cache_bounded() {
    let mut cache = MessagePresentationDecodedRowCache::<ProjectedMessage>::new(2);
    let decode_count = Cell::new(0);
    let decoder = |row: &Map<String, Value>| {
        decode_count.set(decode_count.get() + 1);
        decode(row)
    };
    let first = row_map("a", 1);
    let unchanged_copy = row_map("a", 1);
    let changed = row_map("a", 2);
    let second = row_map("b", 2);
    let third = row_map("c", 3);

    assert_eq!(cache.value(&first, 1, decoder).map(|message| message.timestamp), Some(1));
    assert_eq!(cache.value(&unchanged_copy, 2, decoder).map(|message| message.timestamp), Some(1));
    assert_eq!(decode_count.get(), 1);
    assert_eq!(cache.value(&changed, 3, decoder).map(|message| message.timestamp), Some(2));
    assert_eq!(decode_count.get(), 2);
    assert_eq!(cache.value(&second, 3, decoder).map(|message| message.id), Some("b".to_owned()));
    assert_eq!(cache.value(&third, 3, decoder).map(|message| message.id), Some("c".to_owned()));
    assert_eq!(cache.count(), 2);
    assert_eq!(cache.value(&changed, 3, decoder).map(|message| message.timestamp), Some(2));
    assert_eq!(decode_count.get(), 5);
}

#[test]
fn detects_in_place_and_nested_mutable_row_changes_across_revisions() {
    let mut cache = MessagePresentationDecodedRowCache::<ProjectedMessage>::new(2);
    let decode_count = Cell::new(0);
    let mut nested = Map::new();
    nested.insert("count".to_owned(), Value::from(1));
    let mut row = row_map("mutable", 1);
    row.insert("reaction".to_owned(), Value::Object(nested));
    let decoder = |row: &Map<String, Value>| {
        decode_count.set(decode_count.get() + 1);
        decode(row)
    };

    assert_eq!(cache.value(&row, 1, decoder).map(|message| message.timestamp), Some(1));
    row.insert(rise_key::message::CREATED_AT.to_owned(), Value::from(2));
    assert_eq!(cache.value(&row, 2, decoder).map(|message| message.timestamp), Some(2));
    if let Some(nested) = row.get_mut("reaction").and_then(Value::as_object_mut) {
        nested.insert("count".to_owned(), Value::from(2));
    }
    assert_eq!(cache.value(&row, 3, decoder).map(|message| message.timestamp), Some(2));
    assert_eq!(decode_count.get(), 3);
}

#[test]
fn aggregates_decoded_row_trace_samples_without_changing_cache_semantics() {
    let mut metrics = MessagePresentationDecodedRowTraceMetrics::default();
    metrics.record(MessagePresentationDecodedRowTraceSample {
        outcome: MessagePresentationDecodedRowTraceOutcome::StructuralHit,
        total_duration_ms: 1.25,
        structural_comparison_duration_ms: 0.75,
        decoder_duration_ms: 0.0,
        snapshot_duration_ms: 0.0,
    });
    metrics.record(MessagePresentationDecodedRowTraceSample {
        outcome: MessagePresentationDecodedRowTraceOutcome::DecodedChanged,
        total_duration_ms: 2.5,
        structural_comparison_duration_ms: 0.5,
        decoder_duration_ms: 1.25,
        snapshot_duration_ms: 0.25,
    });

    assert_eq!(metrics.lookups(), 2);
    assert_eq!(metrics.structural_hits(), 1);
    assert_eq!(metrics.decoded_changed(), 1);
    assert_eq!(metrics.reference_hits(), 0);
    let trace_fields: HashMap<String, String> = metrics.trace_fields();
    assert_eq!(trace_fields.get("total_ms").map(String::as_str), Some("3.750"));
    assert_eq!(trace_fields.get("structural_compare_ms").map(String::as_str), Some("1.250"));
    assert_eq!(trace_fields.get("decoder_ms").map(String::as_str), Some("1.250"));
    assert_eq!(trace_fields.get("snapshot_ms").map(String::as_str), Some("0.250"));
}

#[test]
fn projects_latest_tail_directly_from_distant_window() {
    let rows = rows_of(10_000);
    let middle = MessagePresentationWindowProjector::seed(&rows, 21, Some("msg-m-01234"), decode);
    let latest = MessagePresentationWindowProjector::latest(&rows, 21, &middle.state, decode);

    assert!(middle.has_cached_after());
    assert_eq!(latest.state.range, 9_680..10_000);
    assert_eq!(latest.state.generation, middle.state.generation + 1);
    assert_eq!(latest.state.anchor_id.as_deref(), Some("m-09999"));
    assert_eq!(latest.state.anchor_offset, 319);
    assert!(latest.has_cached_before());
    assert!(!latest.has_cached_after());
    assert_eq!(first_id(&latest), Some("m-09680"));
    assert_eq!(last_id(&latest), Some("m-09999"));
    assert!(MessagePresentationWindowProjector::is_at_latest_cached_boundary(&latest.state));
    assert!(!MessagePresentationWindowProjector::is_at_latest_cached_boundary(&middle.state));
}

fn message_entry_version(snapshot: &ChatMessageRenderSnapshot) -> Option<i64> {
    snapshot
        .entries
        .iter()
        .find(|entry| entry.kind == ChatMessageRenderEntryKind::Message)
        .map(|entry| entry.content_version)
}

#[gpui::test]
fn tracing_keeps_content_version_reuse_and_invalidation_semantics(cx: &mut TestAppContext) {
    let mut message = GetMessageMessage::new("render-version-message".to_owned());
    message.chat_id = "render-version-chat".to_owned();
    message.sender_id = "sender".to_owned();
    message.text = "before".to_owned();
    message.created_at = 1_750_000_000.0;

    cx.update(|cx| {
        let first = ChatMessageRenderSnapshotBuilder::make(
            vec![Arc::new(message.clone())],
            1,
            HashMap::new(),
            false,
            HashMap::new(),
            false,
            None,
            cx,
        );
        let unchanged = ChatMessageRenderSnapshotBuilder::make(
            vec![Arc::new(message.clone())],
            2,
            HashMap::new(),
            false,
            HashMap::new(),
            false,
            Some(&first),
            cx,
        );
        message.text = "after".to_owned();
        let changed = ChatMessageRenderSnapshotBuilder::make(
            vec![Arc::new(message.clone())],
            3,
            HashMap::new(),
            false,
            HashMap::new(),
            false,
            Some(&unchanged),
            cx,
        );

        assert_eq!(message_entry_version(&first), message_entry_version(&unchanged));
        assert_ne!(message_entry_version(&changed), message_entry_version(&unchanged));
    });
}
