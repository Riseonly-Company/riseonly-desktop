use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

use futures::channel::oneshot;
use gpui::{TestAppContext, VisualTestContext};
use parking_lot::{Mutex, MutexGuard};
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::Value;

use crate::modules::user::components::profile_content::profile_performance_trace::{
    ProfilePerformanceGestureDirection, ProfilePerformanceGesturePhase, ProfilePerformanceTrace,
    ProfilePerformanceVerticalGeometry,
};

static SERIAL: Mutex<()> = Mutex::new(());

fn serialized() -> MutexGuard<'static, ()> {
    SERIAL.lock()
}

struct ProfilePerformanceTraceTests;

#[test]
fn control_is_unavailable_in_production_without_explicit_argument() {
    assert!(!ProfilePerformanceTrace::control_availability(false, &["Riseonly".to_owned()]));
    assert!(!ProfilePerformanceTrace::control_availability(
        false,
        &["Riseonly".to_owned(), "-riseChatPerformanceTools".to_owned()],
    ));
}

#[test]
fn debug_or_explicit_launch_argument_makes_control_available() {
    assert!(ProfilePerformanceTrace::control_availability(true, &["Riseonly".to_owned()]));
    assert!(ProfilePerformanceTrace::control_availability(
        false,
        &["Riseonly".to_owned(), ProfilePerformanceTrace::LAUNCH_ARGUMENT.to_owned()],
    ));
}

#[test]
fn frame_policy_uses_promotion_cadence() {
    let classification = ProfilePerformanceTrace::classify_frame(1.0 / 60.0, 1.0 / 120.0);

    assert_eq!(classification.missed_frames, 1);
    assert!(classification.is_hitch);
    assert_eq!(ProfilePerformanceTrace::frame_delta(0.0, 100.0), None);
}

#[test]
fn sampler_requests_the_physical_promotion_range() {
    let promotion = ProfilePerformanceTrace::sampler_frame_rate_range(120);
    let standard = ProfilePerformanceTrace::sampler_frame_rate_range(60);

    assert_eq!(promotion.minimum, 30.0);
    assert_eq!(promotion.maximum, 120.0);
    assert_eq!(promotion.preferred, 120.0);
    assert_eq!(standard.minimum, 30.0);
    assert_eq!(standard.maximum, 60.0);
    assert_eq!(standard.preferred, 60.0);
    assert_eq!(ProfilePerformanceTrace::RUN_LOOP_SLOW_THRESHOLD_MS, 12.0);
    assert!(ProfilePerformanceTrace::HITCH_SAMPLE_INTERVAL >= 0.20);
}

#[gpui::test]
async fn disabled_hot_path_does_not_construct_event_fields(cx: &mut TestAppContext) {
    let _serial = serialized();
    let cx = cx.add_empty_window();
    ProfilePerformanceTraceTests::ensure_idle(cx);

    let did_construct_fields = Cell::new(false);
    cx.update(|_, _| {
        ProfilePerformanceTrace::shared().record_with("disabled.event", || {
            did_construct_fields.set(true);
            HashMap::from([("unexpected".to_owned(), "1".to_owned())])
        });
    });

    assert!(!did_construct_fields.get());
}

#[gpui::test]
async fn trace_exports_motion_context_without_raw_profile_scope(cx: &mut TestAppContext) {
    let _serial = serialized();
    let cx = cx.add_empty_window();
    ProfilePerformanceTraceTests::ensure_idle(cx);

    let token = Uuid::new_v4().uuid_string().to_lowercase();
    let raw_scope = format!("profile-user-{token}");
    assert!(start(cx, &raw_scope));
    cx.update(|_, _| {
        let trace = ProfilePerformanceTrace::shared();
        trace.update_vertical_context(
            ProfilePerformanceGesturePhase::Began,
            ProfilePerformanceGestureDirection::Down,
            920.0,
            74.0,
            312.0,
            1,
            ProfilePerformanceVerticalGeometry::default(),
        );
        trace.update_horizontal_context(
            ProfilePerformanceGesturePhase::Changed,
            ProfilePerformanceGestureDirection::Left,
            -1_140.0,
            0.42,
            1,
            Some(2),
        );
        trace.record_horizontal_settle_began(1, 2, 0.42, -1_140.0);
        trace.record_horizontal_settle_ended(2, false);
    });

    let url = ProfilePerformanceTraceTests::stop(cx).expect("the trace file");
    let file_name = url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    assert!(file_name.starts_with("riseonly-profileperf-"));
    let contents = std::fs::read_to_string(&url).expect("readable trace");
    assert!(!contents.contains(&raw_scope));
    assert!(contents.contains("\"event\":\"vertical.phase\""));
    assert!(contents.contains("\"event\":\"horizontal.phase\""));
    assert!(contents.contains("\"event\":\"horizontal.settle_begin\""));
    assert!(contents.contains("\"event\":\"horizontal.settle_end\""));

    let objects = ProfilePerformanceTraceTests::decode_json_lines(&contents);
    let metadata = fields_of(&objects, "metadata").expect("metadata fields");
    let vertical = fields_of(&objects, "vertical.phase").expect("vertical fields");
    let horizontal = fields_of(&objects, "horizontal.phase").expect("horizontal fields");

    assert_eq!(metadata.get("profile").and_then(Value::as_str).map(str::len), Some(8));
    assert_eq!(vertical.get("phase").and_then(Value::as_str), Some("began"));
    assert_eq!(vertical.get("direction").and_then(Value::as_str), Some("down"));
    assert_eq!(vertical.get("outer_offset_y").and_then(Value::as_str), Some("74.00"));
    assert_eq!(vertical.get("inner_offset_y").and_then(Value::as_str), Some("312.00"));
    assert_eq!(horizontal.get("phase").and_then(Value::as_str), Some("changed"));
    assert_eq!(horizontal.get("direction").and_then(Value::as_str), Some("left"));
    assert_eq!(horizontal.get("progress").and_then(Value::as_str), Some("0.42"));
    assert_eq!(horizontal.get("target_tab_index").and_then(Value::as_str), Some("2"));
}

#[gpui::test]
async fn stale_operation_cannot_leak_into_next_session(cx: &mut TestAppContext) {
    let _serial = serialized();
    let cx = cx.add_empty_window();
    ProfilePerformanceTraceTests::ensure_idle(cx);

    assert!(start(cx, "profile-session-a"));
    let stale_operation = cx
        .update(|_, _| ProfilePerformanceTrace::shared().begin_operation("stale.profile.operation", HashMap::new))
        .expect("an operation token");
    assert!(ProfilePerformanceTraceTests::stop(cx).is_some());

    assert!(start(cx, "profile-session-b"));
    cx.update(|_, _| {
        ProfilePerformanceTrace::shared().end_operation(Some(stale_operation), 0.0, HashMap::new);
    });
    let second_url = ProfilePerformanceTraceTests::stop(cx).expect("the trace file");
    let second_contents = std::fs::read_to_string(&second_url).expect("readable trace");

    assert!(!second_contents.contains("operation.stale.profile.operation"));
}

#[gpui::test]
async fn scoped_stop_cannot_finish_another_profiles_trace(cx: &mut TestAppContext) {
    let _serial = serialized();
    let cx = cx.add_empty_window();
    ProfilePerformanceTraceTests::ensure_idle(cx);

    assert!(start(cx, "profile-scope-a"));
    cx.update(|_, _| {
        let trace = ProfilePerformanceTrace::shared();
        assert!(!trace.stop_if_recording("profile-scope-b", "wrong_scope", None));
        assert!(trace.is_recording_any_scope());
        assert!(trace.stop_if_recording("profile-scope-a", "profile_disappear", None));
    });

    let url = ProfilePerformanceTraceTests::stop(cx).expect("the trace file");
    let contents = std::fs::read_to_string(&url).expect("readable trace");
    assert!(contents.contains("profile_disappear"));
}

fn start(cx: &mut VisualTestContext, scope: &str) -> bool {
    cx.update(|window, cx| ProfilePerformanceTrace::shared().start(scope, window, cx))
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

impl ProfilePerformanceTraceTests {
    fn ensure_idle(cx: &mut VisualTestContext) {
        let busy = cx.update(|_, _| {
            let trace = ProfilePerformanceTrace::shared();
            trace.is_recording_any_scope() || trace.is_finishing()
        });
        if busy {
            let _ = Self::stop(cx);
        }
    }

    fn decode_json_lines(contents: &str) -> Vec<serde_json::Map<String, Value>> {
        contents
            .split('\n')
            .filter(|line| !line.is_empty())
            .map(|line| match serde_json::from_str::<Value>(line) {
                Ok(Value::Object(object)) => object,
                other => panic!("not a JSON object line: {other:?}"),
            })
            .collect()
    }

    fn stop(cx: &mut VisualTestContext) -> Option<PathBuf> {
        let (sender, mut receiver) = oneshot::channel();
        cx.update(|_, _| {
            ProfilePerformanceTrace::shared().stop(
                "test",
                Some(Box::new(move |url| {
                    let _ = sender.send(url);
                })),
            );
        });
        cx.run_until_parked();
        receiver.try_recv().ok().flatten().flatten()
    }
}
