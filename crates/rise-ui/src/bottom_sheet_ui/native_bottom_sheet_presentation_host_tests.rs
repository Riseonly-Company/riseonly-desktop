use gpui::{
    Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Styled,
    TestAppContext, VisualTestContext, Window, div,
};
use rise_theme::AppTheme;

use super::modal_sheet_surface::ModalSheetFrame;
use super::native_bottom_sheet_presentation_host::{
    NativeBottomSheetPresentationHost, PresentationContentInteraction, PresentationDetent,
};
use crate::swiftui::prelude::Binding;

#[test]
fn detents_resolve_against_the_container_height() {
    assert_eq!(PresentationDetent::Medium.resolved_height(800.0), 400.0);
    assert_eq!(PresentationDetent::Large.resolved_height(800.0), 800.0);
    assert_eq!(PresentationDetent::Fraction(0.25).resolved_height(800.0), 200.0);
    assert_eq!(PresentationDetent::Height(220.0).resolved_height(800.0), 220.0);
}

#[test]
fn detents_never_exceed_the_container() {
    assert_eq!(PresentationDetent::Height(1200.0).resolved_height(800.0), 800.0);
    assert_eq!(PresentationDetent::Fraction(1.5).resolved_height(800.0), 800.0);
    assert_eq!(PresentationDetent::Fraction(-0.5).resolved_height(800.0), 0.0);
    assert_eq!(PresentationDetent::Large.resolved_height(-10.0), 0.0);
}

#[test]
fn only_scrolls_keeps_the_scroll_in_the_content() {
    assert!(PresentationContentInteraction::Automatic.resizes_first());
    assert!(PresentationContentInteraction::Resizes.resizes_first());
    assert!(!PresentationContentInteraction::Scrolls.resizes_first());
    assert_eq!(
        PresentationContentInteraction::default(),
        PresentationContentInteraction::Automatic
    );
}

struct SheetProbe {
    root_focus: FocusHandle,
    field_focus: FocusHandle,
    content_focus: FocusHandle,
    shows_field: bool,
    is_presented: bool,
    root_takes_empty_focus: bool,
    escapes_past_the_sheet: usize,
}

impl Render for SheetProbe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.root_takes_empty_focus && window.focused(cx).is_none() {
            window.focus(&self.root_focus, cx);
        }
        let is_presented = Binding::entity(
            cx.entity(),
            |probe: &SheetProbe| probe.is_presented,
            |probe, value, _| probe.is_presented = value,
        );
        let content_focus = self.content_focus.clone();
        let mut root = div().size_full().track_focus(&self.root_focus).on_key_down(cx.listener(
            |probe, event: &KeyDownEvent, _, _| {
                if event.keystroke.key == "escape" {
                    probe.escapes_past_the_sheet += 1;
                }
            },
        ));
        if self.shows_field {
            root = root.child(div().track_focus(&self.field_focus));
        }
        root.child(NativeBottomSheetPresentationHost::new("sheet", is_presented, move |_, _| {
            div().size_full().track_focus(&content_focus)
        }))
    }
}

fn open_probe(cx: &mut TestAppContext) -> (Entity<SheetProbe>, &mut VisualTestContext) {
    cx.update(|cx| crate::install_theme(AppTheme::dark(), cx));
    let (probe, cx) = cx.add_window_view(|_, cx| SheetProbe {
        root_focus: cx.focus_handle(),
        field_focus: cx.focus_handle(),
        content_focus: cx.focus_handle(),
        shows_field: true,
        is_presented: false,
        root_takes_empty_focus: false,
        escapes_past_the_sheet: 0,
    });
    cx.run_until_parked();
    (probe, cx)
}

fn set_presented(probe: &Entity<SheetProbe>, is_presented: bool, cx: &mut VisualTestContext) {
    probe.update(cx, |probe, cx| {
        probe.is_presented = is_presented;
        cx.notify();
    });
    cx.run_until_parked();
}

fn focus(handle: &FocusHandle, cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.focus(handle, cx));
    cx.run_until_parked();
}

fn handles(probe: &Entity<SheetProbe>, cx: &VisualTestContext) -> (FocusHandle, FocusHandle, FocusHandle) {
    probe.read_with(cx, |probe, _| {
        (probe.root_focus.clone(), probe.field_focus.clone(), probe.content_focus.clone())
    })
}

#[gpui::test]
fn presenting_takes_the_focus_and_closing_through_the_binding_returns_it(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);
    let (root, _, _) = handles(&probe, cx);
    focus(&root, cx);

    set_presented(&probe, true, cx);
    cx.update(|window, _| assert!(!root.is_focused(window)));

    set_presented(&probe, false, cx);
    cx.update(|window, _| assert!(root.is_focused(window)));
}

#[gpui::test]
fn escape_returns_the_focus_and_writes_the_binding(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);
    let (_, field, _) = handles(&probe, cx);
    focus(&field, cx);
    set_presented(&probe, true, cx);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    cx.update(|window, _| assert!(field.is_focused(window)));
    assert!(!probe.read_with(cx, |probe, _| probe.is_presented));
}

#[gpui::test]
fn focus_inside_the_sheet_content_also_returns(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);
    let (root, _, content) = handles(&probe, cx);
    focus(&root, cx);
    set_presented(&probe, true, cx);
    focus(&content, cx);

    set_presented(&probe, false, cx);
    cx.update(|window, _| assert!(root.is_focused(window)));
}

#[gpui::test]
fn focus_moved_out_of_the_sheet_stays_where_it_went(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);
    let (root, field, _) = handles(&probe, cx);
    focus(&root, cx);
    set_presented(&probe, true, cx);
    focus(&field, cx);

    set_presented(&probe, false, cx);
    cx.update(|window, _| assert!(field.is_focused(window)));
}

#[gpui::test]
fn a_remembered_focus_whose_element_left_the_frame_is_blurred(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);
    let (_, field, _) = handles(&probe, cx);
    focus(&field, cx);
    set_presented(&probe, true, cx);
    probe.update(cx, |probe, cx| {
        probe.shows_field = false;
        cx.notify();
    });
    cx.run_until_parked();

    set_presented(&probe, false, cx);
    cx.update(|window, cx| assert!(window.focused(cx).is_none()));
}

#[gpui::test]
fn no_focus_before_presenting_is_blurred_on_close(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);
    set_presented(&probe, true, cx);

    set_presented(&probe, false, cx);
    cx.update(|window, cx| assert!(window.focused(cx).is_none()));
}

fn blur(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
}

fn press_escape(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
}

#[gpui::test]
fn escape_still_closes_after_the_content_blurs(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);
    let (root, _, _) = handles(&probe, cx);
    focus(&root, cx);
    set_presented(&probe, true, cx);

    blur(cx);
    cx.update(|window, cx| assert!(window.focused(cx).is_some()));

    press_escape(cx);
    assert!(!probe.read_with(cx, |probe, _| probe.is_presented));
    cx.update(|window, _| assert!(root.is_focused(window)));
}

#[gpui::test]
fn escape_still_closes_when_the_root_takes_the_empty_focus_first(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);
    let (root, _, _) = handles(&probe, cx);
    probe.update(cx, |probe, _| probe.root_takes_empty_focus = true);
    focus(&root, cx);
    set_presented(&probe, true, cx);

    blur(cx);
    cx.update(|window, _| assert!(!root.is_focused(window)));

    press_escape(cx);
    assert!(!probe.read_with(cx, |probe, _| probe.is_presented));
}

#[gpui::test]
fn escape_in_the_sheet_closes_it_and_does_not_reach_the_root(cx: &mut TestAppContext) {
    let (probe, cx) = open_probe(cx);
    let (root, _, content) = handles(&probe, cx);
    focus(&root, cx);
    set_presented(&probe, true, cx);
    focus(&content, cx);

    press_escape(cx);
    assert!(!probe.read_with(cx, |probe, _| probe.is_presented));
    assert_eq!(probe.read_with(cx, |probe, _| probe.escapes_past_the_sheet), 0);

    press_escape(cx);
    assert_eq!(probe.read_with(cx, |probe, _| probe.escapes_past_the_sheet), 1);
}

struct NestedSheetProbe {
    outer_presented: bool,
    inner_presented: bool,
}

impl Render for NestedSheetProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let outer = Binding::entity(
            cx.entity(),
            |probe: &NestedSheetProbe| probe.outer_presented,
            |probe, value, _| probe.outer_presented = value,
        );
        let inner = Binding::entity(
            cx.entity(),
            |probe: &NestedSheetProbe| probe.inner_presented,
            |probe, value, _| probe.inner_presented = value,
        );
        div().size_full().child(NativeBottomSheetPresentationHost::new("outer", outer, move |_, _| {
            div()
                .size_full()
                .child(NativeBottomSheetPresentationHost::new("inner", inner.clone(), |_, _| div().size_full()))
        }))
    }
}

fn open_nested_probe(cx: &mut TestAppContext) -> (Entity<NestedSheetProbe>, &mut VisualTestContext) {
    cx.update(|cx| crate::install_theme(AppTheme::dark(), cx));
    let (probe, cx) =
        cx.add_window_view(|_, _| NestedSheetProbe { outer_presented: true, inner_presented: false });
    cx.run_until_parked();
    probe.update(cx, |probe, cx| {
        probe.inner_presented = true;
        cx.notify();
    });
    cx.run_until_parked();
    (probe, cx)
}

#[gpui::test]
fn escape_after_a_blur_in_the_inner_sheet_closes_only_the_inner_one(cx: &mut TestAppContext) {
    let (probe, cx) = open_nested_probe(cx);

    blur(cx);
    press_escape(cx);

    probe.read_with(cx, |probe, _| {
        assert!(!probe.inner_presented);
        assert!(probe.outer_presented);
    });
}

#[gpui::test]
fn the_outer_sheet_answers_escape_after_the_inner_one_closed_and_focus_emptied(cx: &mut TestAppContext) {
    let (probe, cx) = open_nested_probe(cx);
    press_escape(cx);
    assert!(!probe.read_with(cx, |probe, _| probe.inner_presented));

    blur(cx);
    press_escape(cx);

    assert!(!probe.read_with(cx, |probe, _| probe.outer_presented));
}

struct TabProbe {
    page_stop: FocusHandle,
    first_stop: FocusHandle,
    second_stop: FocusHandle,
    has_stops_inside: bool,
    second_stop_moves_the_focus: bool,
    is_presented: bool,
}

impl Render for TabProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_presented = Binding::entity(
            cx.entity(),
            |probe: &TabProbe| probe.is_presented,
            |probe, value, _| probe.is_presented = value,
        );
        let first_stop = self.first_stop.clone();
        let second_stop = self.second_stop.clone();
        let has_stops_inside = self.has_stops_inside;
        let second_stop_moves_the_focus = self.second_stop_moves_the_focus;
        div().size_full().child(div().track_focus(&self.page_stop)).child(NativeBottomSheetPresentationHost::new(
            "sheet",
            is_presented,
            move |_, _| {
                let content = div().size_full();
                if has_stops_inside {
                    content
                        .child(div().track_focus(&first_stop))
                        .child(div().track_focus(&second_stop).on_key_down(move |event, window, cx| {
                            if second_stop_moves_the_focus && event.keystroke.key == "tab" {
                                window.focus_next(cx);
                                cx.stop_propagation();
                            }
                        }))
                } else {
                    content
                }
            },
        ))
    }
}

fn open_tab_probe(cx: &mut TestAppContext, has_stops_inside: bool) -> (Entity<TabProbe>, &mut VisualTestContext) {
    cx.update(|cx| crate::install_theme(AppTheme::dark(), cx));
    let (probe, cx) = cx.add_window_view(|_, cx| TabProbe {
        page_stop: cx.focus_handle().tab_stop(true),
        first_stop: cx.focus_handle().tab_stop(true),
        second_stop: cx.focus_handle().tab_stop(true),
        has_stops_inside,
        second_stop_moves_the_focus: false,
        is_presented: true,
    });
    cx.run_until_parked();
    (probe, cx)
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
}

#[gpui::test]
fn tab_goes_around_inside_the_sheet_and_never_reaches_the_page(cx: &mut TestAppContext) {
    let (probe, cx) = open_tab_probe(cx, true);
    let (first, second) = probe.read_with(cx, |probe, _| (probe.first_stop.clone(), probe.second_stop.clone()));
    focus(&first, cx);

    press("tab", cx);
    cx.update(|window, _| assert!(second.is_focused(window)));
    press("tab", cx);
    cx.update(|window, _| assert!(first.is_focused(window)));
}

#[gpui::test]
fn shift_tab_goes_around_the_other_way(cx: &mut TestAppContext) {
    let (probe, cx) = open_tab_probe(cx, true);
    let (first, second) = probe.read_with(cx, |probe, _| (probe.first_stop.clone(), probe.second_stop.clone()));
    focus(&second, cx);

    press("shift-tab", cx);
    cx.update(|window, _| assert!(first.is_focused(window)));
    press("shift-tab", cx);
    cx.update(|window, _| assert!(second.is_focused(window)));
}

#[gpui::test]
fn a_field_that_moves_the_focus_past_the_last_stop_stays_inside(cx: &mut TestAppContext) {
    let (probe, cx) = open_tab_probe(cx, true);
    let (first, second) = probe.read_with(cx, |probe, _| (probe.first_stop.clone(), probe.second_stop.clone()));
    probe.update(cx, |probe, cx| {
        probe.second_stop_moves_the_focus = true;
        cx.notify();
    });
    cx.run_until_parked();
    focus(&second, cx);

    press("tab", cx);
    cx.update(|window, _| assert!(first.is_focused(window)));
}

#[gpui::test]
fn tab_without_a_stop_inside_stays_in_the_sheet_and_escape_still_closes(cx: &mut TestAppContext) {
    let (probe, cx) = open_tab_probe(cx, false);
    let page = probe.read_with(cx, |probe, _| probe.page_stop.clone());

    press("tab", cx);
    cx.update(|window, _| assert!(!page.is_focused(window)));

    press("escape", cx);
    assert!(!probe.read_with(cx, |probe, _| probe.is_presented));
}

#[test]
fn fixed_height_detents_floor_the_surface_at_their_own_height() {
    let frame = ModalSheetFrame { width: 480.0, max_height: 800.0 };
    assert_eq!(NativeBottomSheetPresentationHost::surface_floor(&[], &frame), None);
    assert_eq!(
        NativeBottomSheetPresentationHost::surface_floor(&[PresentationDetent::Height(212.0)], &frame),
        Some(212.0)
    );
    assert_eq!(
        NativeBottomSheetPresentationHost::surface_floor(
            &[PresentationDetent::Height(212.0), PresentationDetent::Height(268.0)],
            &frame
        ),
        Some(268.0)
    );
    assert_eq!(
        NativeBottomSheetPresentationHost::surface_floor(&[PresentationDetent::Height(1200.0)], &frame),
        Some(800.0)
    );
}

#[test]
fn relative_detents_keep_the_fill_floor() {
    let frame = ModalSheetFrame { width: 480.0, max_height: 800.0 };
    let floor = frame.fill_floor();
    assert_eq!(NativeBottomSheetPresentationHost::surface_floor(&[PresentationDetent::Medium], &frame), Some(floor));
    assert_eq!(
        NativeBottomSheetPresentationHost::surface_floor(
            &[PresentationDetent::Height(212.0), PresentationDetent::Large],
            &frame
        ),
        Some(floor)
    );
}
