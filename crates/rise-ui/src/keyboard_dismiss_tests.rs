use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Bounds, Context, FocusHandle, InteractiveElement, IntoElement, Modifiers, ParentElement, Pixels, Point, Render,
    Styled, TestAppContext, VisualTestContext, Window, div, point, px, size,
};

use crate::keyboard_dismiss::{
    KeyboardDismiss, KeyboardDismissExcludeMarkerView, KeyboardDismissExclusionHitTest, KeyboardDismissModifiers,
    ScreenFocus, SheetConfirmSurface, SheetFocusKey,
};
use crate::swiftui::environment::env_scope;

fn composer_branch_marker(is_visible: bool) -> KeyboardDismissExcludeMarkerView {
    KeyboardDismissExcludeMarkerView {
        bounds: Bounds::new(point(px(0.0), px(700.0)), size(px(390.0), px(100.0))),
        is_visible,
    }
}

#[test]
fn nested_sibling_exclusion_contains_the_touch_point() {
    let markers = [composer_branch_marker(true)];

    assert!(KeyboardDismissExclusionHitTest::contains(
        point(px(330.0), px(750.0)),
        &markers
    ));
    assert!(!KeyboardDismissExclusionHitTest::contains(
        point(px(195.0), px(400.0)),
        &markers
    ));
}

#[test]
fn hidden_exclusion_does_not_block_dismissal() {
    let markers = [composer_branch_marker(false)];

    assert!(!KeyboardDismissExclusionHitTest::contains(
        point(px(330.0), px(750.0)),
        &markers
    ));
}

struct FocusProbe {
    root: FocusHandle,
    screen: FocusHandle,
    next_screen: FocusHandle,
    field: FocusHandle,
}

impl Render for FocusProbe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.root)
            .child(div().track_focus(&self.screen).child(div().track_focus(&self.field)))
            .child(div().track_focus(&self.next_screen))
    }
}

fn open_focus_probe(cx: &mut TestAppContext) -> (FocusHandle, FocusHandle, FocusHandle, FocusHandle, &mut VisualTestContext) {
    let (root, screen, next_screen, field) = cx.update(|cx| {
        (cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle())
    });
    let probe = FocusProbe {
        root: root.clone(),
        screen: screen.clone(),
        next_screen: next_screen.clone(),
        field: field.clone(),
    };
    let (_, cx) = cx.add_window_view(move |_, _| probe);
    cx.update(|window, cx| {
        ScreenFocus::set_window_root(&root, window, cx);
        ScreenFocus::register_text_field(&field, cx);
    });
    cx.run_until_parked();
    (root, screen, next_screen, field, cx)
}

#[gpui::test]
fn a_screen_takes_the_focus_the_window_root_took_first(cx: &mut TestAppContext) {
    let (root, screen, _, _, cx) = open_focus_probe(cx);

    cx.update(|window, cx| {
        assert!(ScreenFocus::is_idle(window, cx));
        window.focus(&root, cx);
        assert!(ScreenFocus::is_idle(window, cx));
        ScreenFocus::claim(&screen, window, cx);
        assert!(screen.is_focused(window));
        assert!(!ScreenFocus::is_idle(window, cx));
    });
}

#[gpui::test]
fn a_pushed_screen_takes_the_focus_from_the_screen_under_it(cx: &mut TestAppContext) {
    let (_, screen, next_screen, _, cx) = open_focus_probe(cx);

    cx.update(|window, cx| {
        window.focus(&screen, cx);
        ScreenFocus::claim(&next_screen, window, cx);
        assert!(next_screen.is_focused(window));
    });
}

#[gpui::test]
fn a_text_field_keeps_the_focus(cx: &mut TestAppContext) {
    let (_, screen, _, field, cx) = open_focus_probe(cx);

    cx.update(|window, cx| {
        window.focus(&field, cx);
        assert!(ScreenFocus::text_field_is_focused(window, cx));
        ScreenFocus::claim(&screen, window, cx);
        assert!(field.is_focused(window));
        assert!(!ScreenFocus::is_idle(window, cx));
    });
}

struct DismissProbe {
    screen: FocusHandle,
    field: FocusHandle,
    sheet: FocusHandle,
    sheet_field: FocusHandle,
}

impl Render for DismissProbe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let row = || div().w(px(200.0)).h(px(100.0));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(row().track_focus(&self.screen))
            .child(row().track_focus(&self.field))
            .child(row().track_focus(&self.sheet).child(row().track_focus(&self.sheet_field)))
            .child(row())
            .install_keyboard_dismiss_on_tap()
    }
}

const ON_THE_SCREEN: Point<Pixels> = point(px(50.0), px(50.0));
const ON_NOTHING: Point<Pixels> = point(px(50.0), px(350.0));

fn open_dismiss_probe(cx: &mut TestAppContext) -> (DismissProbe, &mut VisualTestContext) {
    let (screen, field, sheet, sheet_field) = cx.update(|cx| {
        (cx.focus_handle(), cx.focus_handle(), cx.focus_handle(), cx.focus_handle())
    });
    let handles = DismissProbe {
        screen: screen.clone(),
        field: field.clone(),
        sheet: sheet.clone(),
        sheet_field: sheet_field.clone(),
    };
    let probe = DismissProbe { screen, field, sheet, sheet_field };
    let (_, cx) = cx.add_window_view(move |_, _| probe);
    cx.update(|_, cx| {
        ScreenFocus::register_text_field(&handles.field, cx);
        ScreenFocus::register_text_field(&handles.sheet_field, cx);
        ScreenFocus::register_sheet(&handles.sheet, cx);
    });
    cx.run_until_parked();
    (handles, cx)
}

fn click(position: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_click(position, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn two_clicks_on_a_focused_screen_keep_its_focus(cx: &mut TestAppContext) {
    let (probe, cx) = open_dismiss_probe(cx);
    cx.update(|window, cx| ScreenFocus::claim(&probe.screen, window, cx));
    cx.run_until_parked();

    click(ON_THE_SCREEN, cx);
    cx.update(|window, _| assert!(probe.screen.is_focused(window)));
    click(ON_THE_SCREEN, cx);
    cx.update(|window, _| assert!(probe.screen.is_focused(window)));
}

#[gpui::test]
fn a_click_on_nothing_leaves_a_focused_screen_alone(cx: &mut TestAppContext) {
    let (probe, cx) = open_dismiss_probe(cx);
    cx.update(|window, cx| ScreenFocus::claim(&probe.screen, window, cx));
    cx.run_until_parked();

    click(ON_NOTHING, cx);
    cx.update(|window, _| assert!(probe.screen.is_focused(window)));
}

#[gpui::test]
fn a_click_off_a_field_hands_the_focus_to_the_last_screen(cx: &mut TestAppContext) {
    let (probe, cx) = open_dismiss_probe(cx);
    cx.update(|window, cx| {
        ScreenFocus::claim(&probe.screen, window, cx);
        window.focus(&probe.field, cx);
    });
    cx.run_until_parked();

    click(ON_NOTHING, cx);
    cx.update(|window, _| assert!(probe.screen.is_focused(window)));
}

#[gpui::test]
fn a_keyboard_dismiss_hands_the_focus_to_the_last_screen(cx: &mut TestAppContext) {
    let (probe, cx) = open_dismiss_probe(cx);
    cx.update(|window, cx| {
        ScreenFocus::claim(&probe.screen, window, cx);
        window.focus(&probe.field, cx);
        window.activate_window();
    });
    cx.run_until_parked();

    cx.update(|_, cx| KeyboardDismiss::perform(cx));
    cx.run_until_parked();
    cx.update(|window, _| assert!(probe.screen.is_focused(window)));
}

#[gpui::test]
fn a_field_in_a_sheet_hands_the_focus_to_the_sheet(cx: &mut TestAppContext) {
    let (probe, cx) = open_dismiss_probe(cx);
    cx.update(|window, cx| {
        ScreenFocus::claim(&probe.screen, window, cx);
        window.focus(&probe.sheet_field, cx);
    });
    cx.run_until_parked();

    click(ON_NOTHING, cx);
    cx.update(|window, _| assert!(probe.sheet.is_focused(window)));
}

#[gpui::test]
fn a_screen_out_of_the_frame_does_not_get_the_focus(cx: &mut TestAppContext) {
    let (probe, cx) = open_dismiss_probe(cx);
    let gone = cx.update(|_, cx| cx.focus_handle());
    cx.update(|window, cx| {
        ScreenFocus::claim(&gone, window, cx);
        window.focus(&probe.field, cx);
    });
    cx.run_until_parked();

    click(ON_NOTHING, cx);
    cx.update(|window, cx| assert!(window.focused(cx).is_none()));
}

#[gpui::test]
fn a_leaving_page_hands_the_last_screen_place_back(cx: &mut TestAppContext) {
    let (probe, cx) = open_dismiss_probe(cx);
    let leaving = cx.update(|_, cx| cx.focus_handle());
    cx.update(|window, cx| {
        ScreenFocus::claim(&probe.screen, window, cx);
        ScreenFocus::claim(&leaving, window, cx);
        ScreenFocus::hand_back_screen(&leaving, &probe.screen, window, cx);
        window.focus(&probe.field, cx);
    });
    cx.run_until_parked();

    click(ON_NOTHING, cx);
    cx.update(|window, _| assert!(probe.screen.is_focused(window)));
}

struct ConfirmProbe {
    sheet: FocusHandle,
    button: FocusHandle,
    field: FocusHandle,
    confirms: Rc<Cell<usize>>,
}

impl Render for ConfirmProbe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let confirms = self.confirms.clone();
        div().track_focus(&self.sheet).child(env_scope::<SheetFocusKey>(Some(self.sheet.clone())).child(
            SheetConfirmSurface::new(
                "confirm-probe-surface",
                div().child(div().track_focus(&self.button)).child(div().track_focus(&self.field)),
                move |_, _| confirms.set(confirms.get() + 1),
            ),
        ))
    }
}

fn open_confirm_probe(cx: &mut TestAppContext) -> (FocusHandle, FocusHandle, FocusHandle, Rc<Cell<usize>>, &mut VisualTestContext) {
    let (sheet, button, field) = cx.update(|cx| (cx.focus_handle(), cx.focus_handle(), cx.focus_handle()));
    let confirms = Rc::new(Cell::new(0));
    let probe = ConfirmProbe {
        sheet: sheet.clone(),
        button: button.clone(),
        field: field.clone(),
        confirms: confirms.clone(),
    };
    let (_, cx) = cx.add_window_view(move |_, _| probe);
    cx.update(|window, cx| {
        ScreenFocus::register_text_field(&field, cx);
        window.focus(&sheet, cx);
    });
    cx.run_until_parked();
    (sheet, button, field, confirms, cx)
}

#[gpui::test]
fn return_sends_once_the_surface_took_the_focus_of_the_sheet(cx: &mut TestAppContext) {
    let (sheet, _, _, confirms, cx) = open_confirm_probe(cx);
    cx.update(|window, _| assert!(!sheet.is_focused(window)));
    cx.simulate_keystrokes("enter");
    assert_eq!(confirms.get(), 1);
}

#[gpui::test]
fn return_on_a_button_of_the_sheet_does_not_send(cx: &mut TestAppContext) {
    let (_, button, _, confirms, cx) = open_confirm_probe(cx);
    cx.update(|window, cx| window.focus(&button, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    assert_eq!(confirms.get(), 0);
    cx.simulate_keystrokes("secondary-enter");
    assert_eq!(confirms.get(), 1);
}

#[gpui::test]
fn return_in_a_field_of_the_sheet_is_a_new_line(cx: &mut TestAppContext) {
    let (_, _, field, confirms, cx) = open_confirm_probe(cx);
    cx.update(|window, cx| window.focus(&field, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    assert_eq!(confirms.get(), 0);
    cx.simulate_keystrokes("secondary-enter");
    assert_eq!(confirms.get(), 1);
}

#[gpui::test]
fn another_modifier_on_the_surface_does_not_send(cx: &mut TestAppContext) {
    let (_, _, _, confirms, cx) = open_confirm_probe(cx);
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(confirms.get(), 0);
}
