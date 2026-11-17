use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Entity, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, point, px,
};
use rise_theme::AppTheme;

use super::onboarding::{
    GESTURE_PAUSE, Onboarding, onboarding_illustration_height, onboarding_illustration_size,
    phaseless_move_starts_swipe,
};

const SLIDE_COUNT: usize = 4;

fn open(cx: &mut TestAppContext) -> (Entity<Onboarding>, Rc<Cell<usize>>, &mut VisualTestContext) {
    cx.update(|cx| rise_ui::install_theme(AppTheme::dark(), cx));
    let handed_over = Rc::new(Cell::new(0));
    let counter = handed_over.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| {
        let counter = counter.clone();
        Onboarding::new(move |_, _| counter.set(counter.get() + 1), window, cx)
    });
    (view, handed_over, cx)
}

fn current(view: &Entity<Onboarding>, cx: &mut VisualTestContext) -> usize {
    view.read_with(cx, |view, _| view.current_slide())
}

#[gpui::test]
fn return_walks_the_slides_and_hands_over_on_the_last_one(cx: &mut TestAppContext) {
    let (view, handed_over, cx) = open(cx);

    for expected in 1..SLIDE_COUNT {
        cx.simulate_keystrokes("enter");
        assert_eq!(current(&view, cx), expected);
        assert_eq!(handed_over.get(), 0);
    }

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(current(&view, cx), SLIDE_COUNT - 1, "get started never walks past the last slide");
    assert_eq!(handed_over.get(), 1);
}

#[gpui::test]
fn escape_skips_except_on_the_last_slide(cx: &mut TestAppContext) {
    let (view, handed_over, cx) = open(cx);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(handed_over.get(), 1);

    for _ in 1..SLIDE_COUNT {
        cx.simulate_keystrokes("right");
    }
    assert!(view.read_with(cx, |view, _| view.is_last_slide()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(handed_over.get(), 1, "skip is hidden and disabled on the last slide");
}

#[gpui::test]
fn arrows_turn_pages_within_the_bounds(cx: &mut TestAppContext) {
    let (view, _, cx) = open(cx);

    cx.simulate_keystrokes("left");
    assert_eq!(current(&view, cx), 0);
    cx.simulate_keystrokes("right right");
    assert_eq!(current(&view, cx), 2);
    cx.simulate_keystrokes("left");
    assert_eq!(current(&view, cx), 1);
    cx.simulate_keystrokes("right right right right");
    assert_eq!(current(&view, cx), SLIDE_COUNT - 1);
}

#[test]
fn the_illustration_yields_to_the_texts_on_short_windows() {
    assert_eq!(onboarding_illustration_height(1000.0, 200.0), 340.0);
    assert_eq!(onboarding_illustration_height(500.0, 200.0), 250.0);
    assert_eq!(onboarding_illustration_height(500.0, 400.0), 100.0);
    assert_eq!(onboarding_illustration_height(300.0, 400.0), 0.0);

    assert_eq!(onboarding_illustration_size(250.0, 340.0, 800.0), 250.0);
    assert_eq!(onboarding_illustration_size(250.0, 100.0, 800.0), 60.0);
    assert_eq!(onboarding_illustration_size(250.0, 340.0, 100.0), 60.0);
    assert_eq!(onboarding_illustration_size(250.0, 20.0, 800.0), 0.0);
}

#[test]
fn a_phaseless_move_opens_a_swipe_only_after_a_quiet_spell() {
    let now = Duration::from_secs(9);
    assert!(phaseless_move_starts_swipe(true, None, now));
    assert!(phaseless_move_starts_swipe(true, Some(now - GESTURE_PAUSE), now));
    assert!(
        !phaseless_move_starts_swipe(true, Some(now - Duration::from_millis(16)), now),
        "momentum right behind an ended swipe turns nothing"
    );
    assert!(!phaseless_move_starts_swipe(false, None, now));
}

fn wheel(cx: &mut VisualTestContext, dx: f32, touch_phase: TouchPhase) {
    cx.simulate_event(ScrollWheelEvent {
        position: point(px(300.0), px(300.0)),
        delta: ScrollDelta::Pixels(point(px(dx), px(0.0))),
        modifiers: Modifiers::default(),
        touch_phase,
    });
}

fn settle_pause(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(GESTURE_PAUSE + Duration::from_millis(50));
    cx.run_until_parked();
}

#[gpui::test]
fn a_phaseless_sideways_swipe_turns_the_page_both_ways(cx: &mut TestAppContext) {
    let (view, _, cx) = open(cx);
    cx.run_until_parked();

    wheel(cx, -600.0, TouchPhase::Moved);
    settle_pause(cx);
    assert_eq!(current(&view, cx), 1, "fingers to the left show the next slide");

    std::thread::sleep(GESTURE_PAUSE);
    wheel(cx, 600.0, TouchPhase::Moved);
    settle_pause(cx);
    assert_eq!(current(&view, cx), 0, "fingers to the right show the previous slide");
}

#[gpui::test]
fn a_phased_trackpad_swipe_turns_the_page(cx: &mut TestAppContext) {
    let (view, _, cx) = open(cx);
    cx.run_until_parked();

    wheel(cx, 0.0, TouchPhase::Started);
    for _ in 0..6 {
        wheel(cx, -100.0, TouchPhase::Moved);
    }
    wheel(cx, 0.0, TouchPhase::Ended);
    cx.run_until_parked();
    assert_eq!(current(&view, cx), 1);
}
