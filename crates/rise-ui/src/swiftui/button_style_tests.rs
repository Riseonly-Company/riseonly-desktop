use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    InteractiveElement,
    AnyElement, Context, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, ParentElement,
    Render, Styled, TestAppContext, VisualTestContext, Window, div, hsla, px,
};
use rise_theme::AppTheme;

use crate::swiftui::button_style::{
    BORDERED_CORNER_RADIUS, BORDERED_FILL_OPACITY, BORDERED_PADDING_HORIZONTAL, BORDERED_PADDING_VERTICAL,
    BorderedStyle, bordered_colors, bordered_metrics, button_chrome, button_chrome_hover, plain, system_button,
};
use crate::swiftui::progress_view::ControlSize;

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

fn click(cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds(BUTTON).expect("the button was laid out");
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

fn press_key(cx: &mut VisualTestContext, key: &str) {
    cx.update(|window, cx| window.focus_next(cx));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let keystroke = Keystroke::parse(key).expect("a valid keystroke");
    cx.simulate_event(KeyDownEvent { keystroke: keystroke.clone(), is_held: false, prefer_character_input: false });
    cx.simulate_event(KeyUpEvent { keystroke });
    cx.run_until_parked();
}

fn counted_plain(presses: Rc<Cell<u32>>, disabled: bool) -> impl Fn() -> AnyElement + 'static {
    move || {
        let count = presses.clone();
        div()
            .debug_selector(|| BUTTON.to_string())
            .w(px(120.))
            .h(px(40.))
            .child(
                plain("plain")
                    .disabled(disabled)
                    .size_full()
                    .on_press(move |_, _| count.set(count.get() + 1))
                    .child("Label"),
            )
            .into_any_element()
    }
}

#[test]
fn bordered_is_a_faint_tint_and_prominent_a_full_one() {
    let tint = hsla(0.0, 0.8, 0.5, 1.0);
    let ink = hsla(0.0, 0.0, 1.0, 1.0);
    assert_eq!(bordered_colors(BorderedStyle::Bordered, tint, ink), (tint.opacity(BORDERED_FILL_OPACITY), tint));
    assert_eq!(bordered_colors(BorderedStyle::Prominent, tint, ink), (tint, ink));
}

#[test]
fn control_size_grows_the_bordered_button() {
    let regular = bordered_metrics(ControlSize::Regular);
    assert_eq!(regular.padding_horizontal, BORDERED_PADDING_HORIZONTAL);
    assert_eq!(regular.padding_vertical, BORDERED_PADDING_VERTICAL);
    assert_eq!(regular.corner_radius, BORDERED_CORNER_RADIUS);

    let sizes = [ControlSize::Mini, ControlSize::Small, ControlSize::Regular, ControlSize::Large];
    for pair in sizes.windows(2) {
        let (smaller, larger) = (bordered_metrics(pair[0]), bordered_metrics(pair[1]));
        assert!(smaller.padding_vertical < larger.padding_vertical);
        assert!(smaller.padding_horizontal < larger.padding_horizontal);
    }
}

#[gpui::test]
fn a_plain_button_presses_on_click(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted_plain(presses.clone(), false));
    click(cx);
    assert_eq!(presses.get(), 1);
}

#[gpui::test]
fn a_disabled_plain_button_does_not_press(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted_plain(presses.clone(), true));
    click(cx);
    press_key(cx, "enter");
    assert_eq!(presses.get(), 0);
}

#[gpui::test]
fn enter_presses_a_focused_plain_button(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted_plain(presses.clone(), false));
    press_key(cx, "enter");
    assert_eq!(presses.get(), 1);
}

#[gpui::test]
fn space_presses_a_focused_plain_button(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted_plain(presses.clone(), false));
    press_key(cx, "space");
    assert_eq!(presses.get(), 1);
}

#[gpui::test]
fn a_system_button_presses_and_a_disabled_one_does_not(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let disabled = Rc::new(Cell::new(false));
    let build = {
        let counter = presses.clone();
        let is_disabled = disabled.clone();
        move || {
            let count = counter.clone();
            div()
                .debug_selector(|| BUTTON.to_string())
                .child(
                    system_button("system", BorderedStyle::Prominent)
                        .disabled(is_disabled.get())
                        .on_press(move |_, _| count.set(count.get() + 1))
                        .child("Label"),
                )
                .into_any_element()
        }
    };
    let cx = open(cx, build);
    click(cx);
    assert_eq!(presses.get(), 1);

    disabled.set(true);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    click(cx);
    assert_eq!(presses.get(), 1);
}

#[test]
fn button_chrome_sets_a_single_hover_style() {
    let _ = button_chrome(div().id("default-hover"), false, None);
    let _ = button_chrome_hover(div().id("own-hover"), false, None, |style| style.opacity(0.5));
}
