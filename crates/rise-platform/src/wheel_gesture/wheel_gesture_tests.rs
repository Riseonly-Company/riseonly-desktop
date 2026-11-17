use std::time::Duration;

use gpui::{Modifiers, Pixels, Point, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px};

use crate::host_os::HostOs;
use crate::wheel_gesture::*;

const LINE: Pixels = px(20.0);

fn pixels(y: f32, phase: TouchPhase) -> ScrollWheelEvent {
    ScrollWheelEvent {
        position: point(px(10.0), px(10.0)),
        delta: ScrollDelta::Pixels(point(px(0.0), px(y))),
        modifiers: Modifiers::default(),
        touch_phase: phase,
    }
}

fn lines(y: f32) -> ScrollWheelEvent {
    ScrollWheelEvent {
        position: point(px(10.0), px(10.0)),
        delta: ScrollDelta::Lines(point(0.0, y)),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    }
}

fn up(y: f32) -> Point<Pixels> {
    point(px(0.0), px(y))
}

#[test]
fn phased_trackpad_gesture_ends_on_ended() {
    let mut tracker = WheelGestureTracker::new();
    let start = tracker.feed(&pixels(0.0, TouchPhase::Started), LINE);
    assert_eq!(start.step, WheelStep::Moved(up(0.0)));
    assert_eq!(start.settle, None);
    assert!(tracker.is_tracking());
    let moved = tracker.feed(&pixels(-12.0, TouchPhase::Moved), LINE);
    assert_eq!(moved.step, WheelStep::Moved(up(-12.0)));
    assert_eq!(moved.settle, None);
    let moved = tracker.feed(&pixels(-8.0, TouchPhase::Moved), LINE);
    assert_eq!(moved.step, WheelStep::Moved(up(-20.0)));
    let ended = tracker.feed(&pixels(-1.0, TouchPhase::Ended), LINE);
    assert_eq!(ended.step, WheelStep::Ended(up(-21.0)));
    assert!(ended.settle.is_some());
    assert!(!tracker.is_tracking());
}

#[test]
fn macos_momentum_after_ended_is_swallowed_until_quiet() {
    let mut tracker = WheelGestureTracker::new();
    tracker.feed(&pixels(0.0, TouchPhase::Started), LINE);
    tracker.feed(&pixels(-30.0, TouchPhase::Moved), LINE);
    tracker.feed(&pixels(0.0, TouchPhase::Ended), LINE);
    let first = tracker.feed(&pixels(-40.0, TouchPhase::Moved), LINE);
    assert_eq!(first.step, WheelStep::Ignored);
    let last = tracker.feed(&pixels(-2.0, TouchPhase::Moved), LINE);
    assert_eq!(last.step, WheelStep::Ignored);
    let stale = first.settle.expect("momentum keeps a settle token");
    assert_eq!(tracker.settle(stale), None);
    assert_eq!(tracker.feed(&pixels(-1.0, TouchPhase::Moved), LINE).step, WheelStep::Ignored);
    let quiet = tracker.feed(&pixels(-1.0, TouchPhase::Moved), LINE).settle.expect("token");
    assert_eq!(tracker.settle(quiet), None);
    let next = tracker.feed(&pixels(-5.0, TouchPhase::Moved), LINE);
    assert_eq!(next.step, WheelStep::Moved(up(-5.0)));
}

#[test]
fn mouse_wheel_notches_end_when_quiet() {
    let mut tracker = WheelGestureTracker::new();
    let first = tracker.feed(&lines(-1.0), LINE);
    assert_eq!(first.step, WheelStep::Moved(up(-20.0)));
    let second = tracker.feed(&lines(-2.0), LINE);
    assert_eq!(second.step, WheelStep::Moved(up(-60.0)));
    assert_eq!(tracker.settle(first.settle.expect("token")), None);
    assert!(tracker.is_tracking());
    assert_eq!(tracker.settle(second.settle.expect("token")), Some(WheelStep::Ended(up(-60.0))));
    assert!(!tracker.is_tracking());
    assert_eq!(tracker.settle(second.settle.expect("token")), None);
}

#[test]
fn unphased_pixel_scroll_of_linux_and_windows_ends_when_quiet() {
    let mut tracker = WheelGestureTracker::new();
    tracker.feed(&pixels(-7.0, TouchPhase::Moved), LINE);
    let last = tracker.feed(&pixels(-9.0, TouchPhase::Moved), LINE);
    assert_eq!(last.step, WheelStep::Moved(up(-16.0)));
    assert_eq!(tracker.settle(last.settle.expect("token")), Some(WheelStep::Ended(up(-16.0))));
}

#[test]
fn stray_end_and_cancel_are_ignored() {
    let mut tracker = WheelGestureTracker::new();
    assert_eq!(tracker.feed(&pixels(0.0, TouchPhase::Ended), LINE).step, WheelStep::Ignored);
    assert_eq!(tracker.feed(&pixels(0.0, TouchPhase::Cancelled), LINE).step, WheelStep::Ignored);
    assert!(!tracker.is_tracking());
}

#[test]
fn cancel_unwinds_the_gesture_and_voids_its_token() {
    let mut tracker = WheelGestureTracker::new();
    let moved = tracker.feed(&lines(-1.0), LINE);
    let cancelled = tracker.feed(&pixels(0.0, TouchPhase::Cancelled), LINE);
    assert_eq!(cancelled.step, WheelStep::Cancelled);
    assert_eq!(cancelled.settle, None);
    assert_eq!(tracker.settle(moved.settle.expect("token")), None);
    assert!(!tracker.is_tracking());
}

#[test]
fn wheel_notch_inside_an_unended_phased_gesture_opens_an_unphased_one() {
    let mut tracker = WheelGestureTracker::new();
    tracker.feed(&pixels(0.0, TouchPhase::Started), LINE);
    tracker.feed(&pixels(0.0, TouchPhase::Moved), LINE);
    let notch = tracker.feed(&lines(-1.0), LINE);
    assert_eq!(notch.step, WheelStep::Moved(up(-20.0)));
    assert_eq!(tracker.settle(notch.settle.expect("token")), Some(WheelStep::Ended(up(-20.0))));
}

#[test]
fn new_start_replaces_an_unphased_gesture_and_voids_its_token() {
    let mut tracker = WheelGestureTracker::new();
    let wheel = tracker.feed(&lines(-1.0), LINE);
    let start = tracker.feed(&pixels(-3.0, TouchPhase::Started), LINE);
    assert_eq!(start.step, WheelStep::Moved(up(-3.0)));
    assert_eq!(tracker.settle(wheel.settle.expect("token")), None);
    assert!(tracker.is_tracking());
}

#[test]
fn reset_forgets_the_gesture() {
    let mut tracker = WheelGestureTracker::default();
    let moved = tracker.feed(&lines(-1.0), LINE);
    tracker.reset();
    assert!(!tracker.is_tracking());
    assert_eq!(tracker.settle(moved.settle.expect("token")), None);
}

#[test]
fn a_burst_of_fractional_lines_turns_one_page() {
    let at = |ms: u64| Duration::from_millis(1_000 + ms);
    let mut burst = None;
    let mut verdicts = Vec::new();
    for step in 0..12u64 {
        let (next, verdict) = line_burst_step(burst, -0.3, 0.02, true, 1.0, at(step * 10));
        burst = next;
        verdicts.push(verdict);
    }
    assert_eq!(verdicts[..3], [LineBurstVerdict::Claimed; 3], "under one line nothing turns yet");
    assert_eq!(verdicts[3], LineBurstVerdict::TurnForward);
    assert!(verdicts[4..].iter().all(|verdict| *verdict == LineBurstVerdict::Claimed));

    let (_, verdict) = line_burst_step(burst, 1.0, 0.0, true, 1.0, at(110) + WHEEL_SETTLE_DELAY);
    assert_eq!(verdict, LineBurstVerdict::TurnBack, "a pause starts the next gesture");
}

#[test]
fn a_tremor_under_the_threshold_turns_nothing() {
    let now = Duration::from_secs(3);
    let (burst, verdict) = line_burst_step(None, 0.05, 0.0, true, 1.0, now);
    assert_eq!(verdict, LineBurstVerdict::Claimed);
    let (_, verdict) = line_burst_step(burst, -0.05, 0.0, true, 1.0, now + WHEEL_SETTLE_DELAY);
    assert_eq!(verdict, LineBurstVerdict::Claimed, "the pause dropped what the tremor had saved");
}

#[test]
fn a_wheel_notch_turns_at_once_and_a_burst_across_stays_with_what_lies_under() {
    let now = Duration::from_secs(5);
    let (_, verdict) = line_burst_step(None, 3.0, 0.0, true, 1.0, now);
    assert_eq!(verdict, LineBurstVerdict::TurnBack);

    let (_, verdict) = line_burst_step(None, -1.0, 0.0, true, 1.0, now);
    assert_eq!(verdict, LineBurstVerdict::TurnForward);

    let (burst, verdict) = line_burst_step(None, 0.1, -3.0, true, 1.0, now);
    assert_eq!(verdict, LineBurstVerdict::NotMine);
    let (_, verdict) = line_burst_step(burst, 0.5, 0.0, true, 1.0, now + Duration::from_millis(10));
    assert_eq!(verdict, LineBurstVerdict::NotMine, "a burst across does not turn into a page swipe");

    let (burst, verdict) = line_burst_step(None, -3.0, 0.0, false, 1.0, now);
    assert_eq!((burst, verdict), (None, LineBurstVerdict::NotMine), "outside the pager");
}

#[test]
fn a_slow_macos_notch_of_any_size_turns_and_its_flick_stays_one_page() {
    let turn_lines = line_burst_turn_lines(HostOs::MacOs);
    let now = Duration::from_secs(7);
    let (burst, verdict) = line_burst_step(None, -0.1, 0.0, true, turn_lines, now);
    assert_eq!(verdict, LineBurstVerdict::TurnForward);
    let (_, verdict) = line_burst_step(burst, -0.1, 0.0, true, turn_lines, now + Duration::from_millis(20));
    assert_eq!(verdict, LineBurstVerdict::Claimed);
    let (_, verdict) = line_burst_step(burst, -0.1, 0.0, true, turn_lines, now + WHEEL_SETTLE_DELAY);
    assert_eq!(verdict, LineBurstVerdict::TurnForward, "the next slow notch turns again");
}

#[test]
fn fractional_hosts_need_a_whole_line() {
    assert_eq!(line_burst_turn_lines(HostOs::Linux), 1.0);
    assert_eq!(line_burst_turn_lines(HostOs::Windows), 1.0);
}
