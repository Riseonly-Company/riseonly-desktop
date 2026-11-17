use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    InteractiveElement,
    AnyElement, Context, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, ParentElement,
    Render, Styled, TestAppContext, VisualTestContext, Window, div, point, px,
};
use rise_theme::AppTheme;

use crate::pressable_ui::{PressableState, PressableUi};

const PRESSABLE: &str = "pressable";

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

fn counted(presses: Rc<Cell<u32>>, disabled: bool) -> impl Fn() -> AnyElement + 'static {
    move || {
        let count = presses.clone();
        div()
            .debug_selector(|| PRESSABLE.to_string())
            .w(px(100.))
            .h(px(40.))
            .child(
                PressableUi::new("pressable")
                    .disabled(disabled)
                    .on_press(move |_, _| count.set(count.get() + 1))
                    .child(div().w(px(100.)).h(px(40.))),
            )
            .into_any_element()
    }
}

fn click(cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds(PRESSABLE).expect("the pressable was laid out");
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

#[test]
fn a_press_pulses_and_settles_back_at_rest() {
    let mut state = PressableState::new();
    let start = Duration::from_secs(1);
    state.press_began(0.98, start, false);
    let dip = state.scale(start + Duration::from_millis(20));
    assert!(dip < 1.0, "the scale leaves 1 on touch-down: {dip}");
    assert!(dip > 0.98, "the press is recognised before the tap scale is reached: {dip}");
    assert!((state.scale(start + Duration::from_secs(2)) - 1.0).abs() < 1e-3);
}

#[test]
fn reduce_motion_keeps_the_scale_at_rest() {
    let mut state = PressableState::new();
    state.press_began(0.98, Duration::from_secs(1), true);
    assert_eq!(state.scale(Duration::from_secs(1)), 1.0);
}

#[test]
fn a_plain_tap_presses() {
    let mut state = PressableState::new();
    state.press_down(point(px(1.), px(1.)));
    state.press_ended();
    assert!(state.tap_ended());
    assert!(!state.did_long_press());
}

#[test]
fn a_hold_fires_only_for_the_press_it_started_with() {
    let mut state = PressableState::new();
    state.press_down(point(px(10.), px(10.)));
    let current = state.hold_generation();
    assert!(!state.hold_elapsed(current + 1, Duration::from_secs(1), false), "a stale hold does not fire");
    assert!(state.hold_elapsed(current, Duration::from_secs(1), false));
    assert!(state.did_long_press());
    assert!(!state.tap_ended(), "the tap that ends a long press only clears it");
    assert!(state.tap_ended(), "the next tap presses again");
}

#[test]
fn moving_past_ten_points_cancels_the_hold() {
    let mut state = PressableState::new();
    state.press_down(point(px(0.), px(0.)));
    assert!(!state.moved_too_far(point(px(6.), px(8.))), "exactly ten points still counts");
    assert!(state.moved_too_far(point(px(8.), px(8.))));
    let current = state.hold_generation();
    state.press_ended();
    assert!(!state.hold_elapsed(current, Duration::from_secs(1), false), "a released press does not fire");
}

#[gpui::test]
fn a_click_presses(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), false));
    click(cx);
    assert_eq!(presses.get(), 1);
}

#[gpui::test]
fn a_disabled_pressable_ignores_clicks(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), true));
    click(cx);
    assert_eq!(presses.get(), 0);
}

#[gpui::test]
fn enter_presses_a_focused_pressable(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), false));
    cx.update(|window, cx| window.focus_next(cx));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let keystroke = Keystroke::parse("enter").expect("a valid keystroke");
    cx.simulate_event(KeyDownEvent { keystroke: keystroke.clone(), is_held: false, prefer_character_input: false });
    cx.simulate_event(KeyUpEvent { keystroke });
    cx.run_until_parked();
    assert_eq!(presses.get(), 1);
}
