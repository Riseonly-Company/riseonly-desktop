use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext, Entity, TestAppContext};

use super::repeating_timer::{
    MINIMUM_TIME_INTERVAL, RepeatingTimer, effective_interval, next_fire_date,
};

const INTERVAL: Duration = Duration::from_millis(100);

struct Ticker {
    fires: usize,
    stop_after: Option<usize>,
    stall: Option<Duration>,
    timer: Option<RepeatingTimer>,
}

fn ticker(cx: &mut TestAppContext, repeats: bool, stop_after: Option<usize>) -> Entity<Ticker> {
    stalling_ticker(cx, repeats, stop_after, None)
}

fn stalling_ticker(
    cx: &mut TestAppContext,
    repeats: bool,
    stop_after: Option<usize>,
    stall: Option<Duration>,
) -> Entity<Ticker> {
    let entity = cx.new(|cx| Ticker {
        fires: 0,
        stop_after,
        stall,
        timer: Some(RepeatingTimer::scheduled_timer(
            INTERVAL,
            repeats,
            cx,
            |this: &mut Ticker, cx| {
                this.fires += 1;
                if let Some(busy) = this.stall.take() {
                    cx.background_executor().advance_clock(busy);
                }
                if this.stop_after == Some(this.fires)
                    && let Some(timer) = this.timer.as_mut()
                {
                    timer.invalidate();
                }
            },
        )),
    });
    cx.run_until_parked();
    entity
}

fn advance(cx: &mut TestAppContext, by: Duration) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

fn fires(cx: &mut TestAppContext, entity: &Entity<Ticker>) -> usize {
    entity.read_with(cx, |ticker, _| ticker.fires)
}

fn is_valid(cx: &mut TestAppContext, entity: &Entity<Ticker>) -> bool {
    entity.read_with(cx, |ticker, _| {
        ticker.timer.as_ref().is_some_and(RepeatingTimer::is_valid)
    })
}

#[gpui::test]
fn a_repeating_timer_fires_once_per_interval(cx: &mut TestAppContext) {
    let entity = ticker(cx, true, None);
    advance(cx, INTERVAL - Duration::from_millis(1));
    assert_eq!(fires(cx, &entity), 0);
    advance(cx, Duration::from_millis(1));
    assert_eq!(fires(cx, &entity), 1);
    advance(cx, INTERVAL);
    advance(cx, INTERVAL);
    assert_eq!(fires(cx, &entity), 3);
    assert!(is_valid(cx, &entity));
}

#[gpui::test]
fn a_late_fire_runs_once_and_keeps_the_grid(cx: &mut TestAppContext) {
    let entity = stalling_ticker(cx, true, None, Some(Duration::from_millis(250)));
    advance(cx, INTERVAL);
    assert_eq!(fires(cx, &entity), 1, "missed fires are not replayed");
    advance(cx, Duration::from_millis(49));
    assert_eq!(fires(cx, &entity), 1);
    advance(cx, Duration::from_millis(1));
    assert_eq!(fires(cx, &entity), 2, "the next fire lands on start + 4 * interval");
}

#[gpui::test]
fn a_non_repeating_timer_fires_once_and_turns_invalid(cx: &mut TestAppContext) {
    let entity = ticker(cx, false, None);
    advance(cx, INTERVAL);
    assert_eq!(fires(cx, &entity), 1);
    assert!(!is_valid(cx, &entity));
    advance(cx, INTERVAL * 3);
    assert_eq!(fires(cx, &entity), 1);
}

#[gpui::test]
fn invalidate_from_inside_the_block_stops_it(cx: &mut TestAppContext) {
    let entity = ticker(cx, true, Some(2));
    advance(cx, INTERVAL);
    advance(cx, INTERVAL);
    advance(cx, INTERVAL);
    advance(cx, INTERVAL);
    assert_eq!(fires(cx, &entity), 2);
    assert!(!is_valid(cx, &entity));
}

#[gpui::test]
fn dropping_the_timer_is_invalidate(cx: &mut TestAppContext) {
    let entity = ticker(cx, true, None);
    advance(cx, INTERVAL);
    entity.update(cx, |ticker, _| ticker.timer = None);
    advance(cx, INTERVAL * 3);
    assert_eq!(fires(cx, &entity), 1);
}

#[gpui::test]
fn an_app_timer_fires_until_invalidated(cx: &mut TestAppContext) {
    let count = Rc::new(Cell::new(0));
    let seen = count.clone();
    let mut timer = cx.update(|cx| {
        RepeatingTimer::scheduled_timer_in_app(INTERVAL, true, cx, move |_| {
            seen.set(seen.get() + 1);
        })
    });
    advance(cx, INTERVAL);
    advance(cx, INTERVAL);
    assert_eq!(count.get(), 2);
    timer.invalidate();
    assert!(!timer.is_valid());
    advance(cx, INTERVAL * 2);
    assert_eq!(count.get(), 2);
}

#[gpui::test]
fn tolerance_and_interval_read_back(cx: &mut TestAppContext) {
    let mut timer = cx.update(|cx| {
        RepeatingTimer::scheduled_timer_in_app(INTERVAL, true, cx, |_| {})
    });
    assert_eq!(timer.time_interval(), INTERVAL);
    assert_eq!(timer.tolerance(), Duration::ZERO);
    timer.set_tolerance(INTERVAL / 10);
    assert_eq!(timer.tolerance(), INTERVAL / 10);
}

#[gpui::test]
fn next_fire_date_skips_to_the_first_grid_point_not_in_the_past(cx: &mut TestAppContext) {
    let start = cx.executor().now();
    assert_eq!(next_fire_date(start, INTERVAL, start), start + INTERVAL);
    assert_eq!(
        next_fire_date(start, INTERVAL, start + Duration::from_millis(250)),
        start + Duration::from_millis(300)
    );
    assert_eq!(
        next_fire_date(start, INTERVAL, start + Duration::from_millis(300)),
        start + Duration::from_millis(300)
    );
}

#[test]
fn a_zero_interval_becomes_the_foundation_minimum() {
    assert_eq!(effective_interval(Duration::ZERO), MINIMUM_TIME_INTERVAL);
    assert_eq!(effective_interval(INTERVAL), INTERVAL);
}
