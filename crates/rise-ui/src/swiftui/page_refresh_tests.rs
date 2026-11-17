use gpui::{
    AppContext as _, Context, FocusHandle, InteractiveElement, IntoElement, Keystroke, ParentElement, Render,
    TestAppContext, VisualTestContext, Window, div,
};

use super::page_refresh::{PAGE_REFRESH_BINDING, PageFocus, is_page_refresh_keystroke};
use crate::keyboard_dismiss::ScreenFocus;

fn keystroke(source: &str) -> Keystroke {
    Keystroke::parse(source).expect("a valid keystroke")
}

#[test]
fn the_binding_is_the_shortcut() {
    assert!(is_page_refresh_keystroke(&keystroke(PAGE_REFRESH_BINDING)));
}

#[test]
fn the_key_alone_is_not_the_shortcut() {
    assert!(!is_page_refresh_keystroke(&keystroke("r")));
}

#[test]
fn another_key_with_the_modifier_is_not_the_shortcut() {
    assert!(!is_page_refresh_keystroke(&keystroke("secondary-t")));
}

struct PageProbe {
    covered: FocusHandle,
    page: FocusHandle,
    elsewhere: FocusHandle,
}

impl Render for PageProbe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(div().track_focus(&self.covered))
            .child(div().track_focus(&self.page))
            .child(div().track_focus(&self.elsewhere))
    }
}

fn open_page_probe(cx: &mut TestAppContext) -> (FocusHandle, FocusHandle, FocusHandle, &mut VisualTestContext) {
    let (covered, page, elsewhere) =
        cx.update(|cx| (cx.focus_handle(), cx.focus_handle(), cx.focus_handle()));
    let probe = PageProbe { covered: covered.clone(), page: page.clone(), elsewhere: elsewhere.clone() };
    let (_, cx) = cx.add_window_view(move |_, _| probe);
    cx.run_until_parked();
    (covered, page, elsewhere, cx)
}

#[gpui::test]
fn a_page_hands_the_focus_back_when_it_leaves(cx: &mut TestAppContext) {
    let (covered, page, _, cx) = open_page_probe(cx);

    cx.update(|window, cx| {
        window.focus(&covered, cx);
        let focus = cx.new(|_| PageFocus::with_handle(page.clone()));
        PageFocus::did_appear(&focus, window, cx);
        assert!(page.is_focused(window));
        PageFocus::will_disappear(&focus, window, cx);
        assert!(covered.is_focused(window));
    });
}

#[gpui::test]
fn a_page_leaves_a_focus_that_moved_elsewhere_alone(cx: &mut TestAppContext) {
    let (covered, page, elsewhere, cx) = open_page_probe(cx);

    cx.update(|window, cx| {
        window.focus(&covered, cx);
        let focus = cx.new(|_| PageFocus::with_handle(page.clone()));
        PageFocus::did_appear(&focus, window, cx);
        window.focus(&elsewhere, cx);
        PageFocus::will_disappear(&focus, window, cx);
        assert!(elsewhere.is_focused(window));
    });
}

#[gpui::test]
fn a_focus_that_left_the_frame_returns_to_the_covered_page(cx: &mut TestAppContext) {
    let (covered, page, _, cx) = open_page_probe(cx);

    cx.update(|window, cx| {
        window.focus(&covered, cx);
        let focus = cx.new(|_| PageFocus::with_handle(page.clone()));
        PageFocus::did_appear(&focus, window, cx);
        let gone = cx.focus_handle();
        window.focus(&gone, cx);
        PageFocus::will_disappear(&focus, window, cx);
        assert!(covered.is_focused(window));
    });
}

#[gpui::test]
fn a_text_field_keeps_the_focus_when_a_page_appears(cx: &mut TestAppContext) {
    let (covered, page, _, cx) = open_page_probe(cx);

    cx.update(|window, cx| {
        ScreenFocus::register_text_field(&covered, cx);
        window.focus(&covered, cx);
        let focus = cx.new(|_| PageFocus::with_handle(page.clone()));
        PageFocus::did_appear(&focus, window, cx);
        assert!(covered.is_focused(window));
    });
}
