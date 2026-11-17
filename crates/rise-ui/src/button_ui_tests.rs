use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    InteractiveElement,
    AnyElement, App, Context, Hsla, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers,
    ParentElement, Render, RenderOnce, Styled, TestAppContext, VisualTestContext, Window, div, hsla,
    px,
};
use rise_theme::AppTheme;

use crate::button_ui::{ButtonUi, ButtonUiSurface};
use crate::main_text::FillContentColorKey;
use crate::swiftui::environment::env;

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

fn counted(presses: Rc<Cell<u32>>, disabled: bool, is_pending: bool) -> impl Fn() -> AnyElement + 'static {
    move || {
        let count = presses.clone();
        div()
            .debug_selector(|| BUTTON.to_string())
            .w(px(200.))
            .child(
                ButtonUi::new("button-ui")
                    .disabled(disabled)
                    .is_pending(is_pending)
                    .on_press(move |_, _| count.set(count.get() + 1))
                    .child("Label"),
            )
            .into_any_element()
    }
}

/// records the fill ink a MainText with no color of its own would read where its drawn
#[derive(IntoElement)]
struct InkProbe {
    seen: Rc<RefCell<Vec<Option<Hsla>>>>,
}

impl RenderOnce for InkProbe {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.seen.borrow_mut().push(env::<FillContentColorKey>(cx));
        div().w(px(10.)).h(px(10.))
    }
}

#[test]
fn disabled_or_pending_blocks_the_press() {
    assert!(ButtonUi::new("b").accepts_press());
    assert!(!ButtonUi::new("b").disabled(true).accepts_press());
    assert!(!ButtonUi::new("b").is_pending(true).accepts_press());
}

#[test]
fn the_surface_defaults_to_the_primary_fill_and_its_ink() {
    let theme = AppTheme::dark();
    let colors = ButtonUiSurface::new().resolved_colors(&theme);
    assert_eq!(colors.background, theme.primary._100);
    assert_eq!(colors.height, theme.button.height_300);
    assert_eq!(colors.fill_content, theme.content_on(theme.primary._100));
    assert_eq!(colors.loader, colors.fill_content, "the loader takes the fill ink by default");

    let fill = hsla(0.15, 1.0, 0.6, 1.0);
    let loader = hsla(0.6, 1.0, 0.5, 1.0);
    let custom = ButtonUiSurface::new().background_color(fill).height(30.).loader_color(loader).resolved_colors(&theme);
    assert_eq!(custom.background, fill);
    assert_eq!(custom.height, px(30.));
    assert_eq!(custom.fill_content, theme.content_on(fill));
    assert_eq!(custom.loader, loader);
}

#[gpui::test]
fn content_reads_the_ink_of_the_fill_and_nothing_outside(cx: &mut TestAppContext) {
    let seen: Rc<RefCell<Vec<Option<Hsla>>>> = Rc::default();
    let fill = hsla(0.15, 1.0, 0.6, 1.0);
    let build = {
        let log = seen.clone();
        move || {
            div()
                .child(ButtonUiSurface::new().background_color(fill).child(InkProbe { seen: log.clone() }))
                .child(InkProbe { seen: log.clone() })
                .into_any_element()
        }
    };
    let cx = open(cx, build);
    let expected = cx.update(|_, cx| crate::theme(cx).content_on(fill));
    let recorded = seen.borrow();
    assert_eq!(recorded.first(), Some(&Some(expected)), "inside the surface");
    assert_eq!(recorded.get(1), Some(&None), "outside the surface");
}

#[gpui::test]
fn an_enabled_button_presses_on_click(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), false, false));
    click(cx);
    assert_eq!(presses.get(), 1);
}

#[gpui::test]
fn a_disabled_button_does_not_press(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), true, false));
    click(cx);
    press_key(cx, "enter");
    assert_eq!(presses.get(), 0);
}

#[gpui::test]
fn a_pending_button_does_not_press(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), false, true));
    click(cx);
    press_key(cx, "space");
    assert_eq!(presses.get(), 0);
}

#[gpui::test]
fn enter_presses_a_focused_button(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), false, false));
    press_key(cx, "enter");
    assert_eq!(presses.get(), 1);
}

#[gpui::test]
fn space_presses_a_focused_button(cx: &mut TestAppContext) {
    let presses = Rc::new(Cell::new(0));
    let cx = open(cx, counted(presses.clone(), false, false));
    press_key(cx, "space");
    assert_eq!(presses.get(), 1);
}
