use std::time::Duration;

use super::*;

#[test]
fn a_name_is_searched_in_the_red_panda_folder_then_animations_then_the_root() {
    assert_eq!(
        candidate_paths("hello"),
        vec![
            "animations/red_panda/hello.json".to_owned(),
            "animations/hello.json".to_owned(),
            "hello.json".to_owned(),
        ]
    );
}

#[test]
fn a_paused_view_rests_halfway_through_the_timeline() {
    assert_eq!(paused_frame_index(0), 0);
    assert_eq!(paused_frame_index(1), 0);
    assert_eq!(paused_frame_index(61), 30);
    assert_eq!(paused_frame_index(60), 30);
}

#[test]
fn double_speed_halves_the_frame_delay() {
    let normal = Duration::from(frame_delay(30.0, 1, 1.0)).as_secs_f64();
    let double = Duration::from(frame_delay(30.0, 1, 2.0)).as_secs_f64();
    let ratio = normal / double;
    assert!((ratio - 2.0).abs() < 0.05, "ratio {ratio}");
}

#[test]
fn a_stride_keeps_the_authored_duration() {
    let one = Duration::from(frame_delay(60.0, 1, 1.0)).as_secs_f64();
    let two = Duration::from(frame_delay(60.0, 2, 1.0)).as_secs_f64();
    let ratio = two / one;
    assert!((ratio - 2.0).abs() < 0.05, "ratio {ratio}");
}

#[test]
fn an_extreme_speed_never_yields_a_zero_or_endless_delay() {
    for speed in [0.000_001_f32, -1.0, 1000.0] {
        let delay = Duration::from(frame_delay(30.0, 1, speed));
        assert!(delay >= Duration::from_millis(1), "{speed} gave {delay:?}");
        assert!(delay <= Duration::from_secs(1), "{speed} gave {delay:?}");
    }
}

#[test]
fn a_looping_timeline_advances_and_wraps() {
    let delay = Duration::from_millis(100);
    assert_eq!(timeline_frame_index(Duration::ZERO, delay, 4, true), 0);
    assert_eq!(timeline_frame_index(Duration::from_millis(250), delay, 4, true), 2);
    assert_eq!(timeline_frame_index(Duration::from_millis(400), delay, 4, true), 0);
}

#[test]
fn a_play_once_timeline_rests_on_its_last_frame() {
    let delay = Duration::from_millis(100);
    assert_eq!(timeline_frame_index(Duration::from_millis(250), delay, 4, false), 2);
    assert_eq!(timeline_frame_index(Duration::from_secs(60), delay, 4, false), 3);
}

#[test]
fn a_timeline_without_motion_stays_on_the_first_frame() {
    assert_eq!(timeline_frame_index(Duration::from_secs(5), Duration::from_millis(100), 1, true), 0);
    assert_eq!(timeline_frame_index(Duration::from_secs(5), Duration::ZERO, 4, true), 0);
    assert_eq!(timeline_frame_index(Duration::from_secs(5), Duration::from_millis(100), 0, true), 0);
}
