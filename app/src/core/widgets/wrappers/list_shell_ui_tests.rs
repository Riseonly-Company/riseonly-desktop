use gpui::{
    Context, InteractiveElement, IntoElement, Modifiers, ParentElement, Render, ScrollDelta, ScrollWheelEvent, Styled,
    TestAppContext, TouchPhase, VisualTestContext, Window, div, point, px,
};
use rise_theme::AppTheme;
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::scroll_view::scroll_view;

use super::list_shell_ui::{
    ListShellFloatingTopInsetKey, ListShellScrollUnderChromeModifiers, list_shell_under_chrome_box,
};

const FIRST_ROW: &str = "list_shell_first_row";
const INSET: f32 = 56.0;
const ROW_HEIGHT: f32 = 100.0;

fn first_row() -> gpui::Div {
    div().w_full().h(px(ROW_HEIGHT)).debug_selector(|| FIRST_ROW.to_string())
}

fn other_row() -> gpui::Div {
    div().w_full().h(px(ROW_HEIGHT))
}

/// builds the content before entering the scope, like a screen does when it passes it to the shell
struct ScrollerScreen;

impl Render for ScrollerScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = scroll_view("list-shell-scroller")
            .list_shell_scroll_under_chrome(cx)
            .child(first_row())
            .child(other_row())
            .child(other_row())
            .child(other_row());
        div()
            .w(px(300.0))
            .h(px(ROW_HEIGHT))
            .child(env_scope::<ListShellFloatingTopInsetKey>(INSET).child(div().size_full().child(content)))
    }
}

struct BoxScreen;

impl Render for BoxScreen {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let content = list_shell_under_chrome_box().size_full().child(first_row());
        div()
            .w(px(300.0))
            .h(px(ROW_HEIGHT))
            .child(env_scope::<ListShellFloatingTopInsetKey>(INSET).child(div().size_full().child(content)))
    }
}

fn first_row_top(cx: &mut VisualTestContext) -> f32 {
    f32::from(cx.debug_bounds(FIRST_ROW).expect("the first row was laid out").origin.y)
}

#[gpui::test]
fn scroller_built_outside_the_scope_starts_at_the_inset_of_the_scope(cx: &mut TestAppContext) {
    cx.update(|cx| rise_ui::install_theme(AppTheme::dark(), cx));
    let (_, cx) = cx.add_window_view(|_, _| ScrollerScreen);
    cx.run_until_parked();
    assert_eq!(first_row_top(cx), INSET);
}

#[gpui::test]
fn the_inset_scrolls_with_the_rows_so_they_pass_under_the_chrome(cx: &mut TestAppContext) {
    cx.update(|cx| rise_ui::install_theme(AppTheme::dark(), cx));
    let (_, cx) = cx.add_window_view(|_, _| ScrollerScreen);
    cx.run_until_parked();
    cx.simulate_event(ScrollWheelEvent {
        position: point(px(10.0), px(10.0)),
        delta: ScrollDelta::Pixels(point(px(0.0), px(-80.0))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert!(first_row_top(cx) < 0.0, "the first row stays below the top edge");
}

#[gpui::test]
fn box_built_outside_the_scope_pads_by_the_inset_of_the_scope(cx: &mut TestAppContext) {
    cx.update(|cx| rise_ui::install_theme(AppTheme::dark(), cx));
    let (_, cx) = cx.add_window_view(|_, _| BoxScreen);
    cx.run_until_parked();
    assert_eq!(first_row_top(cx), INSET);
}
