use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    InteractiveElement,
    AnyElement, Context, IntoElement, Modifiers, ParentElement, Render, Styled, TestAppContext,
    VisualTestContext, Window, div, hsla, px, transparent_black,
};
use rise_theme::AppTheme;

use crate::simple_button_ui::{
    PRESSED_WHITEN_OPACITY, SimpleButtonPressState, SimpleButtonUi, SimpleButtonVariant,
};

const BUTTON: &str = "button";

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

fn counted(presses: Rc<Cell<u32>>, variant: SimpleButtonVariant, disabled: bool) -> impl Fn() -> AnyElement + 'static {
    move || {
        let count = presses.clone();
        div()
            .debug_selector(|| BUTTON.to_string())
            .w(px(160.))
            .child(
                SimpleButtonUi::new("simple")
                    .variant(variant)
                    .disabled(disabled)
                    .width(120.)
                    .height(40.)
                    .on_press(move |_, _| count.set(count.get() + 1))
                    .child("Label"),
            )
            .into_any_element()
    }
}

fn click(cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds(BUTTON).expect("the button was laid out");
    cx.simulate_click(bounds.origin + gpui::point(px(20.), px(20.)), Modifiers::none());
    cx.run_until_parked();
}

#[test]
fn the_grouped_variant_sits_on_bg_200_and_the_default_on_its_own_colour() {
    let theme = AppTheme::dark();
    let own = hsla(0.3, 0.5, 0.5, 1.0);
    assert_eq!(SimpleButtonVariant::Default.resolved_bg(Some(own), &theme), own);
    assert_eq!(SimpleButtonVariant::Default.resolved_bg(None, &theme), transparent_black());
    assert_eq!(SimpleButtonVariant::GroupedBtn.resolved_bg(Some(own), &theme), theme.bg._200);
    assert!(SimpleButtonVariant::GroupedBtn.press_style().whiten_on_press);
    assert!(!SimpleButtonVariant::Default.press_style().whiten_on_press);
}

#[test]
fn the_press_whitening_eases_in_and_out_over_0_12_s() {
    let mut state = SimpleButtonPressState::new();
    let start = Duration::from_secs(1);
    state.set_pressed(true, start, false);
    assert!(state.is_running(start + Duration::from_millis(60)));
    let midway = state.whiten_opacity(start + Duration::from_millis(60));
    assert!(midway > 0.0 && midway < PRESSED_WHITEN_OPACITY);
    let pressed = start + Duration::from_millis(120);
    assert!((state.whiten_opacity(pressed) - PRESSED_WHITEN_OPACITY).abs() < 1e-4);
    state.set_pressed(false, pressed, false);
    assert!(state.whiten_opacity(pressed + Duration::from_millis(120)).abs() < 1e-4);
}

#[gpui::test]
fn a_click_presses(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), SimpleButtonVariant::GroupedBtn, false));
    click(cx);
    assert_eq!(presses.get(), 1);
}

#[gpui::test]
fn a_disabled_button_does_not_press(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), SimpleButtonVariant::Default, true));
    click(cx);
    assert_eq!(presses.get(), 0);
}
