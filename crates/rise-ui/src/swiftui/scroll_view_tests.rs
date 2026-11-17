use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::channel::oneshot;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, Modifiers, ParentElement, Render, ScrollDelta, ScrollWheelEvent,
    Styled, Task, TestAppContext, TouchPhase, VisualTestContext, Window, div, point, px,
};
use rise_theme::AppTheme;

use crate::swiftui::scroll_view::{
    INDICATOR_INSET, INDICATOR_MIN_LENGTH, PULL_RELEASE_DELAY, REFRESH_TRIGGER_DISTANCE, indicator_thumb, pull_step,
    rubber_band, scroll_view,
};

const FIRST_ROW: &str = "scroll_view_first_row";
const VIEWPORT: f32 = 100.0;

struct Probe {
    build: Box<dyn Fn() -> AnyElement>,
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child((self.build)())
    }
}

fn open(cx: &mut TestAppContext, build: impl Fn() -> AnyElement + 'static) -> &mut VisualTestContext {
    cx.update(|cx| crate::install_theme(AppTheme::dark(), cx));
    let (_, cx) = cx.add_window_view(move |_, _| Probe { build: Box::new(build) });
    cx.run_until_parked();
    cx
}

fn rows() -> impl Iterator<Item = AnyElement> {
    (0..4).map(|row| {
        let cell = div().w_full().h(px(VIEWPORT));
        if row == 0 { cell.debug_selector(|| FIRST_ROW.to_string()).into_any_element() } else { cell.into_any_element() }
    })
}

fn wheel(cx: &mut VisualTestContext, dy: f32, phase: TouchPhase) {
    cx.simulate_event(ScrollWheelEvent {
        position: point(px(10.), px(10.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        modifiers: Modifiers::none(),
        touch_phase: phase,
    });
    cx.run_until_parked();
}

/// one trackpad pull far past the top, then the fingers lift
fn pull_down(cx: &mut VisualTestContext) {
    wheel(cx, 0.0, TouchPhase::Started);
    for _ in 0..10 {
        wheel(cx, 50.0, TouchPhase::Moved);
    }
    wheel(cx, 0.0, TouchPhase::Ended);
}

fn first_row_top(cx: &mut VisualTestContext) -> f32 {
    f32::from(cx.debug_bounds(FIRST_ROW).expect("the first row was laid out").origin.y)
}

#[test]
fn rubber_band_slows_down_and_never_reaches_the_view_length() {
    assert_eq!(rubber_band(0.0, VIEWPORT), 0.0);
    assert_eq!(rubber_band(-10.0, VIEWPORT), 0.0);
    let short = rubber_band(50.0, VIEWPORT);
    let long = rubber_band(100.0, VIEWPORT);
    assert!(short > 0.0 && short < 50.0);
    assert!(long - short < short, "the second half of the pull moves the content less");
    assert!(rubber_band(1.0e6, VIEWPORT) < VIEWPORT);
    assert!(rubber_band(500.0, VIEWPORT) >= REFRESH_TRIGGER_DISTANCE);
}

#[test]
fn only_the_part_past_the_top_counts_as_pull() {
    assert_eq!(pull_step(0.0, 10.0, 10.0), 10.0, "at the top the whole step pulls");
    assert_eq!(pull_step(0.0, 10.0, 4.0), 4.0, "a step that first reaches the top pulls the rest");
    assert_eq!(pull_step(0.0, 10.0, 0.0), 0.0, "a step inside the content does not pull");
    assert_eq!(pull_step(30.0, -10.0, 0.0), 20.0, "pushing back gives the pull up");
    assert_eq!(pull_step(5.0, -10.0, 0.0), 0.0);
}

#[test]
fn the_thumb_follows_the_offset_and_keeps_a_minimum_length() {
    assert_eq!(indicator_thumb(VIEWPORT, 0.0, 0.0), None, "nothing scrolls, no thumb");
    let (top, length) = indicator_thumb(VIEWPORT, 300.0, 0.0).expect("the content scrolls");
    assert_eq!(top, INDICATOR_INSET);
    assert!(length >= INDICATOR_MIN_LENGTH);
    let (bottom_top, bottom_length) = indicator_thumb(VIEWPORT, 300.0, 300.0).expect("the content scrolls");
    assert!((bottom_top + bottom_length - (VIEWPORT - INDICATOR_INSET)).abs() < 1e-3);
    let (long_top, long_length) = indicator_thumb(VIEWPORT, 10.0, 5.0).expect("the content scrolls");
    assert!(long_length > INDICATOR_MIN_LENGTH && long_top > INDICATOR_INSET);
}

#[gpui::test]
fn a_disabled_scroll_view_does_not_scroll(cx: &mut TestAppContext) {
    let cx = open(cx, || {
        scroll_view("disabled").scroll_disabled(true).w(px(200.)).h(px(VIEWPORT)).children(rows()).into_any_element()
    });
    let before = first_row_top(cx);
    wheel(cx, -50.0, TouchPhase::Moved);
    assert_eq!(first_row_top(cx), before);
}

#[gpui::test]
fn an_enabled_scroll_view_scrolls(cx: &mut TestAppContext) {
    let cx = open(cx, || scroll_view("enabled").w(px(200.)).h(px(VIEWPORT)).children(rows()).into_any_element());
    let before = first_row_top(cx);
    wheel(cx, -50.0, TouchPhase::Moved);
    assert_eq!(first_row_top(cx), before - 50.0);
}

#[gpui::test]
fn a_pull_while_refreshing_does_not_start_a_second_refresh(cx: &mut TestAppContext) {
    let pending: Rc<RefCell<Vec<oneshot::Sender<()>>>> = Rc::default();
    let calls = pending.clone();
    let cx = open(cx, move || {
        let calls = calls.clone();
        scroll_view("refreshable")
            .w(px(200.))
            .h(px(VIEWPORT))
            .refreshable(move |_, cx| {
                let (finish, finished) = oneshot::channel::<()>();
                calls.borrow_mut().push(finish);
                cx.background_executor().spawn(async move {
                    finished.await.ok();
                })
            })
            .children(rows())
            .into_any_element()
    });

    pull_down(cx);
    assert_eq!(pending.borrow().len(), 1, "a pull past the top starts a refresh");

    pull_down(cx);
    assert_eq!(pending.borrow().len(), 1, "the first refresh still runs");

    let first = pending.borrow_mut().remove(0);
    first.send(()).ok();
    cx.run_until_parked();

    pull_down(cx);
    assert_eq!(pending.borrow().len(), 1, "after the first refresh ends a pull starts the next");
}

fn open_counting(cx: &mut TestAppContext, calls: Rc<Cell<usize>>) -> &mut VisualTestContext {
    open(cx, move || {
        let calls = calls.clone();
        scroll_view("counting")
            .w(px(200.))
            .h(px(VIEWPORT))
            .refreshable(move |_, _| {
                calls.set(calls.get() + 1);
                Task::ready(())
            })
            .children(rows())
            .into_any_element()
    })
}

#[gpui::test]
fn the_momentum_after_a_lift_does_not_pull_past_the_top(cx: &mut TestAppContext) {
    let calls = Rc::new(Cell::new(0));
    let cx = open_counting(cx, calls.clone());

    wheel(cx, 0.0, TouchPhase::Started);
    wheel(cx, 0.0, TouchPhase::Ended);
    for _ in 0..10 {
        wheel(cx, 50.0, TouchPhase::Moved);
    }

    assert_eq!(calls.get(), 0, "momentum starts no refresh");
    assert_eq!(first_row_top(cx), 0.0, "momentum opens no gap above the content");
}

#[gpui::test]
fn an_upward_step_on_an_open_pull_closes_it_without_scrolling_the_content(cx: &mut TestAppContext) {
    let calls = Rc::new(Cell::new(0));
    let cx = open_counting(cx, calls.clone());

    wheel(cx, 0.0, TouchPhase::Started);
    wheel(cx, 50.0, TouchPhase::Moved);
    assert!(first_row_top(cx) > 0.0, "the pull opens a band above the content");

    wheel(cx, -50.0, TouchPhase::Moved);
    assert_eq!(first_row_top(cx), 0.0, "the same step back closes the band and scrolls nothing");
    assert_eq!(calls.get(), 0);
}

#[gpui::test]
fn a_pull_whose_end_never_comes_springs_back(cx: &mut TestAppContext) {
    let calls = Rc::new(Cell::new(0));
    let cx = open_counting(cx, calls.clone());

    wheel(cx, 0.0, TouchPhase::Started);
    wheel(cx, 50.0, TouchPhase::Moved);
    assert!(first_row_top(cx) > 0.0);

    cx.executor().advance_clock(PULL_RELEASE_DELAY);
    cx.run_until_parked();
    assert_eq!(first_row_top(cx), 0.0, "a quiet pull springs back");
}
