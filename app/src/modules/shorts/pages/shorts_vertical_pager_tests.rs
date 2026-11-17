use std::time::Duration;

use gpui::{AppContext, Entity, TestAppContext, point, px};
use rise_platform::wheel_gesture::{WHEEL_SETTLE_DELAY, WheelStep};

use super::shorts_vertical_pager::ShortsVerticalPagerController;

const PAGES: usize = 5;
const WHOLE_LINE: f32 = 1.0;
const SETTLED: Duration = Duration::from_millis(300);

fn pager(cx: &mut TestAppContext) -> Entity<ShortsVerticalPagerController> {
    let pager = cx.new(|_| ShortsVerticalPagerController::new(None));
    pager.update(cx, |pager, _| pager.update_pages(&[], PAGES, 800.0));
    pager
}

fn scroll(pager: &Entity<ShortsVerticalPagerController>, y: f32, at: Duration, cx: &mut TestAppContext) -> (bool, usize) {
    pager.update(cx, |pager, cx| {
        let claimed = pager.handle_line_scroll(point(0.0, y), WHOLE_LINE, at, cx);
        (claimed, pager.shown_page)
    })
}

#[gpui::test]
fn a_long_x11_swipe_of_fractional_lines_turns_one_page(cx: &mut TestAppContext) {
    let pager = pager(cx);
    let start = Duration::from_secs(1);
    let shown: Vec<usize> = (0..40u32)
        .map(|step| scroll(&pager, -0.3, start + Duration::from_millis(10) * step, cx).1)
        .collect();
    assert_eq!(shown[..3], [0; 3], "under one line nothing turns yet");
    assert!(shown[3..].iter().all(|page| *page == 1), "the swipe outlasted the settle animation: {shown:?}");
}

#[gpui::test]
fn a_finger_tremor_turns_nothing_and_stays_in_the_pager(cx: &mut TestAppContext) {
    let pager = pager(cx);
    assert_eq!(scroll(&pager, -0.05, Duration::from_secs(1), cx), (true, 0));
}

#[gpui::test]
fn notches_apart_turn_a_page_each_but_not_while_the_page_still_settles(cx: &mut TestAppContext) {
    let pager = pager(cx);
    let start = Duration::from_secs(1);
    assert_eq!(scroll(&pager, -3.0, start, cx), (true, 1));
    assert_eq!(scroll(&pager, -3.0, start + WHEEL_SETTLE_DELAY, cx), (true, 1), "a notch during the settle is dropped");
    assert_eq!(scroll(&pager, -3.0, start + SETTLED * 2, cx), (true, 2));
    assert_eq!(scroll(&pager, 3.0, start + SETTLED * 4, cx), (true, 1));
}

#[gpui::test]
fn the_first_page_stays_put_and_a_sideways_burst_goes_to_the_riff_pager(cx: &mut TestAppContext) {
    let pager = pager(cx);
    assert_eq!(scroll(&pager, 3.0, Duration::from_secs(1), cx), (true, 0));
    let claimed = pager.update(cx, |pager, cx| {
        pager.handle_line_scroll(point(-3.0, 0.0), WHOLE_LINE, Duration::from_secs(2), cx)
    });
    assert!(!claimed);
}

fn swipe(
    pager: &Entity<ShortsVerticalPagerController>,
    step: WheelStep,
    starts: bool,
    cx: &mut TestAppContext,
) -> (bool, usize) {
    swipe_at(pager, step, starts, Duration::from_secs(1), cx)
}

fn swipe_at(
    pager: &Entity<ShortsVerticalPagerController>,
    step: WheelStep,
    starts: bool,
    now: Duration,
    cx: &mut TestAppContext,
) -> (bool, usize) {
    pager.update(cx, |pager, cx| {
        let claimed = pager.apply_wheel_step(step, starts, now, cx);
        (claimed, pager.shown_page)
    })
}

fn along(y: f32) -> gpui::Point<gpui::Pixels> {
    point(px(0.0), px(y))
}

#[gpui::test]
fn one_trackpad_swipe_and_its_momentum_turn_exactly_one_page(cx: &mut TestAppContext) {
    let pager = pager(cx);
    assert_eq!(swipe(&pager, WheelStep::Moved(along(-5.0)), true, cx), (true, 0));
    assert_eq!(swipe(&pager, WheelStep::Moved(along(-30.0)), false, cx), (true, 1));
    assert_eq!(swipe(&pager, WheelStep::Moved(along(-600.0)), false, cx), (true, 1));
    assert_eq!(swipe(&pager, WheelStep::Ended(along(-900.0)), false, cx), (true, 1));
    for _ in 0..20 {
        assert_eq!(swipe(&pager, WheelStep::Ignored, false, cx), (true, 1), "momentum stays on the turned page");
    }
}

#[gpui::test]
fn swipes_in_a_row_turn_one_page_each_in_their_own_direction(cx: &mut TestAppContext) {
    let pager = pager(cx);
    let mut now = Duration::from_secs(1);
    for expected in 1..PAGES {
        now += SETTLED;
        assert_eq!(swipe_at(&pager, WheelStep::Moved(along(-40.0)), true, now, cx), (true, expected));
        assert_eq!(swipe_at(&pager, WheelStep::Ended(along(-300.0)), false, now, cx), (true, expected));
    }
    now += SETTLED;
    let last = swipe_at(&pager, WheelStep::Moved(along(-40.0)), true, now, cx);
    assert_eq!(last, (true, PAGES - 1), "the last page stays");
    assert_eq!(swipe_at(&pager, WheelStep::Moved(along(40.0)), true, now + SETTLED, cx), (true, PAGES - 2));
    assert_eq!(swipe_at(&pager, WheelStep::Moved(along(40.0)), true, now + SETTLED * 2, cx), (true, PAGES - 3));
}

#[gpui::test]
fn fast_swipes_during_the_settle_are_swallowed_whole(cx: &mut TestAppContext) {
    let pager = pager(cx);
    let start = Duration::from_secs(1);
    assert_eq!(swipe_at(&pager, WheelStep::Moved(along(-40.0)), true, start, cx), (true, 1));
    assert_eq!(swipe_at(&pager, WheelStep::Ended(along(-300.0)), false, start, cx), (true, 1));
    let quick = start + Duration::from_millis(80);
    assert_eq!(swipe_at(&pager, WheelStep::Moved(along(-40.0)), true, quick, cx), (true, 1));
    for step in [WheelStep::Moved(along(-400.0)), WheelStep::Ended(along(-900.0)), WheelStep::Ignored] {
        let shown = swipe_at(&pager, step, false, quick + SETTLED, cx);
        assert_eq!(shown, (true, 1), "the gesture opened in the lock stays spent");
    }
    assert_eq!(swipe_at(&pager, WheelStep::Moved(along(40.0)), true, start + SETTLED * 3, cx), (true, 0));
}

#[gpui::test]
fn a_quick_flick_turns_at_the_lift_and_a_brush_turns_nothing(cx: &mut TestAppContext) {
    let pager = pager(cx);
    assert_eq!(swipe(&pager, WheelStep::Moved(along(-3.0)), true, cx), (true, 0));
    assert_eq!(swipe(&pager, WheelStep::Ended(along(-10.0)), false, cx), (true, 1));
    assert_eq!(swipe(&pager, WheelStep::Moved(along(-2.0)), true, cx), (true, 1));
    assert_eq!(swipe(&pager, WheelStep::Ended(along(-4.0)), false, cx), (true, 1));
    assert_eq!(swipe(&pager, WheelStep::Ignored, false, cx), (false, 1));
    let later = Duration::from_secs(1) + SETTLED;
    assert_eq!(swipe_at(&pager, WheelStep::Moved(along(-2.0)), true, later, cx), (true, 1));
    assert_eq!(swipe_at(&pager, WheelStep::Ended(along(-10.0)), false, later, cx), (true, 2));
}

#[gpui::test]
fn back_on_the_first_page_pulls_and_sideways_goes_to_the_riff_pager(cx: &mut TestAppContext) {
    let pager = pager(cx);
    assert_eq!(swipe(&pager, WheelStep::Moved(along(60.0)), true, cx), (true, 0));
    assert!(pager.update(cx, |pager, _| pager.is_moving()), "the pull follows the fingers");
    assert_eq!(swipe(&pager, WheelStep::Ended(along(120.0)), false, cx), (true, 0));
    assert_eq!(swipe(&pager, WheelStep::Moved(point(px(-40.0), px(5.0))), true, cx), (false, 0));
    assert_eq!(swipe(&pager, WheelStep::Moved(point(px(-80.0), px(-30.0))), false, cx), (false, 0));
}

#[gpui::test]
fn arrow_keys_turn_one_page_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let pager = pager(cx);
    let step = |step: f32, cx: &mut TestAppContext| {
        pager.update(cx, |pager, cx| {
            let moved = pager.step_page(step, cx);
            (moved, pager.shown_page)
        })
    };
    assert_eq!(step(-1.0, cx), (false, 0));
    assert_eq!(step(1.0, cx), (true, 1));
    assert_eq!(step(1.0, cx), (true, 2));
    assert_eq!(step(-1.0, cx), (true, 1));
}
