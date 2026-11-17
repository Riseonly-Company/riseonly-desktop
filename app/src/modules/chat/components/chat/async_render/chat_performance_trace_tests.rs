use std::path::PathBuf;

use futures::channel::oneshot;
use gpui::{TestAppContext, VisualTestContext};
use parking_lot::{Mutex, MutexGuard};
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::Value;

use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;

static SERIAL: Mutex<()> = Mutex::new(());

fn serialized() -> MutexGuard<'static, ()> {
    SERIAL.lock()
}

struct ChatPerformanceTraceTests;

#[gpui::test]
async fn trace_exports_jsonl_without_raw_chat_or_message_identifiers(cx: &mut TestAppContext) {
    let _serial = serialized();
    let cx = cx.add_empty_window();
    ChatPerformanceTraceTests::ensure_idle(cx);

    let token = Uuid::new_v4().uuid_string();
    let raw_chat_id = format!("private-chat-{token}");
    let raw_message_id = format!("message-{token}");
    assert!(start(cx, &raw_chat_id));
    cx.update(|_, _| {
        ChatPerformanceTrace::shared().record_height_mismatch(&raw_message_id, "IMAGE", 120.0, 184.0, true, 7);
    });

    let url = ChatPerformanceTraceTests::stop(cx).expect("the trace file");
    let contents = std::fs::read_to_string(&url).expect("readable trace");
    assert!(contents.contains("\"event\":\"metadata\""));
    assert!(contents.contains("\"event\":\"height.mismatch\""));
    assert!(!contents.contains(&raw_chat_id));
    assert!(!contents.contains(&raw_message_id));

    let objects = parse_lines(&contents);
    let metadata = fields_of(&objects, "metadata").expect("metadata fields");
    let mismatch = fields_of(&objects, "height.mismatch").expect("mismatch fields");
    let exported_session_id = metadata.get("session").and_then(Value::as_str).expect("session");
    let configuration = metadata.get("configuration").and_then(Value::as_str).expect("configuration");
    let debugger_attached =
        metadata.get("debugger_attached").and_then(Value::as_str).expect("debugger_attached");
    let row_fingerprint = mismatch.get("row").and_then(Value::as_str).expect("row");
    assert!(configuration == "debug" || configuration == "release");
    assert!(debugger_attached == "0" || debugger_attached == "1");
    assert_ne!(row_fingerprint, ChatPerformanceTraceTests::fingerprint(&raw_message_id, exported_session_id));
}

#[test]
fn one_missed_frame_at120_hz_is_classified_as_a_hitch() {
    let classification = ChatPerformanceTrace::classify_frame(1.0 / 60.0, 1.0 / 120.0);

    assert_eq!(classification.missed_frames, 1);
    assert!(classification.is_hitch);
}

#[test]
fn first_display_link_sample_only_establishes_frame_baseline() {
    assert_eq!(ChatPerformanceTrace::frame_delta(0.0, 100.125), None);
    assert!(ChatPerformanceTrace::frame_delta(100.125, 100.125 + 1.0 / 120.0).is_some());
}

#[test]
fn diagnostic_events_do_not_replace_the_last_attributed_work() {
    assert!(!ChatPerformanceTrace::advances_activity("frame.hitch"));
    assert!(!ChatPerformanceTrace::advances_activity("frame.summary"));
    assert!(!ChatPerformanceTrace::advances_activity("main_runloop.slow"));
    assert!(ChatPerformanceTrace::advances_activity("viewport.state"));
    assert!(ChatPerformanceTrace::advances_activity("hosting.layout_subviews"));
}

#[gpui::test]
async fn slow_operation_is_counted_exactly_once_in_session_summary(cx: &mut TestAppContext) {
    let _serial = serialized();
    let cx = cx.add_empty_window();
    ChatPerformanceTraceTests::ensure_idle(cx);

    assert!(start(cx, "trace-operation-counter"));
    cx.update(|_, _| {
        let trace = ChatPerformanceTrace::shared();
        let operation = trace.begin_operation("single.count", Default::default());
        assert!(operation.is_some());
        trace.end_operation(operation, 0.0, Default::default());
    });

    let url = ChatPerformanceTraceTests::stop(cx).expect("the trace file");
    let contents = std::fs::read_to_string(&url).expect("readable trace");
    let objects = parse_lines(&contents);
    let operation_events = objects
        .iter()
        .filter(|object| object.get("event").and_then(Value::as_str) == Some("operation.single.count"))
        .count();
    let end_fields = fields_of(&objects, "session.end").expect("session end fields");
    let counters = end_fields.get("counters").and_then(Value::as_str).expect("counters");

    assert_eq!(operation_events, 1);
    assert!(counters.contains("operation.single.count:1"));
    assert!(!counters.contains("operation.single.count:2"));
}

#[gpui::test]
async fn sessions_use_unique_files_and_reject_stale_operation_tokens(cx: &mut TestAppContext) {
    let _serial = serialized();
    let cx = cx.add_empty_window();
    ChatPerformanceTraceTests::ensure_idle(cx);

    assert!(start(cx, "trace-session-a"));
    let stale_token = cx.update(|_, _| ChatPerformanceTrace::shared().begin_operation("stale.operation", Default::default()));
    assert!(stale_token.is_some());
    let first_url = ChatPerformanceTraceTests::stop(cx).expect("the first trace file");

    assert!(start(cx, "trace-session-b"));
    cx.update(|_, _| ChatPerformanceTrace::shared().end_operation(stale_token, 0.0, Default::default()));
    let second_url = ChatPerformanceTraceTests::stop(cx).expect("the second trace file");
    let second_contents = std::fs::read_to_string(&second_url).expect("readable trace");

    assert_ne!(first_url, second_url);
    assert!(!second_contents.contains("operation.stale.operation"));
}

#[gpui::test]
async fn capped_trace_always_includes_dropped_event_trailer(cx: &mut TestAppContext) {
    let _serial = serialized();
    let cx = cx.add_empty_window();
    ChatPerformanceTraceTests::ensure_idle(cx);

    assert!(start(cx, "trace-cap"));
    cx.update(|_, _| {
        ChatPerformanceTrace::shared().record(
            "oversized.event",
            [("payload".to_owned(), "x".repeat(4 * 1_024 * 1_024))].into_iter().collect(),
        );
    });

    let url = ChatPerformanceTraceTests::stop(cx).expect("the trace file");
    let contents = std::fs::read_to_string(&url).expect("readable trace");
    assert!(contents.contains("\"event\":\"writer.dropped\""));
    assert!(!contents.contains("\"event\":\"oversized.event\""));
}

#[gpui::test]
async fn chat_bound_stop_does_not_stop_another_chats_trace(cx: &mut TestAppContext) {
    let _serial = serialized();
    let cx = cx.add_empty_window();
    ChatPerformanceTraceTests::ensure_idle(cx);

    assert!(start(cx, "trace-chat-a"));
    cx.update(|_, _| {
        let trace = ChatPerformanceTrace::shared();
        assert!(!trace.stop_if_recording("trace-chat-b", "chat_switch"));
        assert!(trace.is_recording());
        assert!(trace.stop_if_recording("trace-chat-a", "chat_disappear"));
    });

    let url = ChatPerformanceTraceTests::stop(cx).expect("the trace file");
    let contents = std::fs::read_to_string(&url).expect("readable trace");
    assert!(contents.contains("chat_disappear"));
}

fn start(cx: &mut VisualTestContext, chat_id: &str) -> bool {
    cx.update(|window, cx| ChatPerformanceTrace::shared().start(chat_id, false, window, cx))
}

fn parse_lines(contents: &str) -> Vec<serde_json::Map<String, Value>> {
    contents
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| match serde_json::from_str::<Value>(line) {
            Ok(Value::Object(object)) => object,
            other => panic!("not a JSON object line: {other:?}"),
        })
        .collect()
}

fn fields_of(
    objects: &[serde_json::Map<String, Value>],
    event: &str,
) -> Option<serde_json::Map<String, Value>> {
    objects
        .iter()
        .find(|object| object.get("event").and_then(Value::as_str) == Some(event))
        .and_then(|object| object.get("fields"))
        .and_then(Value::as_object)
        .cloned()
}

impl ChatPerformanceTraceTests {
    fn ensure_idle(cx: &mut VisualTestContext) {
        let busy = cx.update(|_, _| {
            let trace = ChatPerformanceTrace::shared();
            trace.is_recording() || trace.is_finishing()
        });
        if busy {
            let _ = ChatPerformanceTraceTests::stop(cx);
        }
    }

    fn stop(cx: &mut VisualTestContext) -> Option<PathBuf> {
        let (sender, mut receiver) = oneshot::channel();
        cx.update(|_, _| {
            ChatPerformanceTrace::shared().stop(
                "test",
                Some(Box::new(move |url| {
                    let _ = sender.send(url);
                })),
            );
        });
        cx.run_until_parked();
        receiver.try_recv().ok().flatten().flatten()
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
