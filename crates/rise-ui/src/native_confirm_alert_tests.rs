use std::cell::Cell;
use std::rc::Rc;

use gpui::{Empty, TestAppContext};

use super::*;

#[gpui::test]
fn nothing_is_presenting_before_a_dialog(cx: &mut TestAppContext) {
    cx.update(|cx| assert!(!NativeConfirmAlert::is_presenting(cx)));
}

#[gpui::test]
fn a_delete_without_a_window_to_host_it_cancels(cx: &mut TestAppContext) {
    let cancelled = Rc::new(Cell::new(false));
    let confirmed = Rc::new(Cell::new(false));
    let on_cancel_flag = cancelled.clone();
    let on_confirm_flag = confirmed.clone();
    cx.update(|cx| {
        NativeConfirmAlert::present_delete(
            "title",
            "message",
            None,
            None,
            true,
            None,
            Some(Box::new(move |_| on_cancel_flag.set(true))),
            move |_| on_confirm_flag.set(true),
            cx,
        );
        assert!(!NativeConfirmAlert::is_presenting(cx));
    });
    assert!(cancelled.get());
    assert!(!confirmed.get());
}

#[gpui::test]
fn a_notice_without_a_window_runs_nothing(cx: &mut TestAppContext) {
    let dismissed = Rc::new(Cell::new(false));
    let flag = dismissed.clone();
    cx.update(|cx| {
        NativeConfirmAlert::present_notice("title", "message", None, None, move |_| flag.set(true), cx);
        assert!(!NativeConfirmAlert::is_presenting(cx));
    });
    assert!(!dismissed.get());
}

#[gpui::test]
fn a_delete_from_inside_the_window_update_opens_its_dialog(cx: &mut TestAppContext) {
    let cancelled = Rc::new(Cell::new(false));
    let on_cancel_flag = cancelled.clone();
    let window = cx.add_window(|_, _| Empty);
    window
        .update(cx, |_, _, cx| {
            NativeConfirmAlert::present_delete(
                "title",
                "message",
                None,
                None,
                true,
                None,
                Some(Box::new(move |_| on_cancel_flag.set(true))),
                |_| {},
                cx,
            );
            assert!(NativeConfirmAlert::is_presenting(cx));
        })
        .unwrap();
    assert!(cx.has_pending_prompt());

    cx.simulate_prompt_answer(&tr("common_cancel"));
    cx.run_until_parked();
    assert!(cancelled.get());
    cx.update(|cx| assert!(!NativeConfirmAlert::is_presenting(cx)));
}

#[gpui::test]
fn a_window_gone_before_the_dialog_opens_cancels(cx: &mut TestAppContext) {
    let cancelled = Rc::new(Cell::new(false));
    let on_cancel_flag = cancelled.clone();
    let window = cx.add_window(|_, _| Empty);
    cx.update(|cx| {
        NativeConfirmAlert::present_delete(
            "title",
            "message",
            None,
            None,
            true,
            None,
            Some(Box::new(move |_| on_cancel_flag.set(true))),
            |_| {},
            cx,
        );
        window.update(cx, |_, window, _| window.remove_window()).unwrap();
    });
    assert!(!cx.has_pending_prompt());
    assert!(cancelled.get());
    cx.update(|cx| assert!(!NativeConfirmAlert::is_presenting(cx)));
}
