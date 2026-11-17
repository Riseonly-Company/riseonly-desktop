#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use gpui::{TestAppContext, point, px};

use crate::core::lib::sai::modules::sai_debugger::sai_debugger_overlay_state::{
    SaiDebuggerOverlayState, SaiDebuggerUnlockPolicy, SaiFloatingDebugMotionPolicy,
};
use crate::core::lib::sai::modules::sai_debugger::sai_performance_monitor::SaiPerformanceSamplingPolicy;

#[gpui::test]
fn orb_is_on_screen_from_launch_and_comes_back_after_a_dismiss(cx: &mut TestAppContext) {
    let overlay = cx.update(SaiDebuggerOverlayState::shared);
    assert!(cx.update(|cx| overlay.read(cx).is_orb_visible()));

    cx.update(|cx| overlay.update(cx, |state, cx| state.dismiss_orb(cx)));
    assert!(!cx.update(|cx| overlay.read(cx).is_orb_visible()));

    cx.update(|cx| overlay.update(cx, |state, cx| state.show_orb(cx)));
    assert!(cx.update(|cx| overlay.read(cx).is_orb_visible()));
}

#[test]
fn debugger_unlocks_on_exactly_ten_quick_more_taps() {
    let mut policy = SaiDebuggerUnlockPolicy::default();

    for index in 0..9 {
        let unlocked = policy.register_tap(f64::from(index) * 0.2);
        assert!(!unlocked);
    }
    assert_eq!(policy.tap_count(), 9);
    let unlocked = policy.register_tap(1.8);
    assert!(unlocked);
    assert_eq!(policy.tap_count(), 0);
}

#[test]
fn slow_tap_breaks_the_unlock_sequence() {
    let mut policy = SaiDebuggerUnlockPolicy::default();

    for index in 0..6 {
        let unlocked = policy.register_tap(f64::from(index) * 0.2);
        assert!(!unlocked);
    }
    let unlocked = policy.register_tap(3.0);
    assert!(!unlocked);
    assert_eq!(policy.tap_count(), 1);
}

#[test]
fn only_strong_horizontal_offscreen_fling_dismisses_floating_tool() {
    let current = point(px(42.), px(300.));

    assert!(SaiFloatingDebugMotionPolicy::should_dismiss(
        current,
        point(px(-180.), px(315.)),
        px(390.),
        px(54.),
    ));
    assert!(!SaiFloatingDebugMotionPolicy::should_dismiss(
        current,
        point(px(-180.), px(520.)),
        px(390.),
        px(54.),
    ));
    assert!(!SaiFloatingDebugMotionPolicy::should_dismiss(
        current,
        point(px(18.), px(300.)),
        px(390.),
        px(54.),
    ));
}

#[test]
fn frame_classifier_counts_missed_cadence_intervals() {
    let classification = SaiPerformanceSamplingPolicy::classify_frame(0.025, 1.0 / 120.0);

    assert!(classification.is_hitch);
    assert_eq!(classification.missed_frames, 2);
}

#[test]
fn performance_recorder_remains_bounded() {
    assert!(SaiPerformanceSamplingPolicy::HITCH_RECORD_INTERVAL >= 0.20);
    assert!(SaiPerformanceSamplingPolicy::MAXIMUM_RECORDING_DURATION <= 300.0);
    assert!(SaiPerformanceSamplingPolicy::RUN_LOOP_SLOW_THRESHOLD_MS >= 8.0);
}
