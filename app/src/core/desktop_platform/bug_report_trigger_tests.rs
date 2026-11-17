use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyWindowHandle, AppContext as _, Context, IntoElement, Render, TestAppContext, Window, WindowId, WindowOptions, div,
};

use super::*;
use crate::app::shell::shell_actions;
use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::desktop_platform::rise_app_lifecycle::LifecycleSignal;

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn counting_trigger(cx: &mut TestAppContext) -> (Rc<Cell<usize>>, WindowId) {
    let presented: Rc<Cell<usize>> = Rc::default();
    let counter = presented.clone();
    let window = cx.update(|cx| {
        RiseAppLifecycle::install_with(RiseAppEvents::new(), cx);
        let present = move |_: RiseBugReportSurface, _: &mut App| counter.set(counter.get() + 1);
        install_with(present, cx);
        let handle = cx
            .open_window(WindowOptions::default(), |_, cx| cx.new(|_| Blank))
            .expect("the test window opens");
        AnyWindowHandle::from(handle).window_id()
    });
    (presented, window)
}

fn press(cx: &mut TestAppContext) {
    cx.update(|cx| cx.dispatch_action(&ReportBug));
    cx.run_until_parked();
}

#[gpui::test]
fn the_action_reaches_the_store_once_in_an_active_window(cx: &mut TestAppContext) {
    let (presented, window) = counting_trigger(cx);
    cx.update(|cx| RiseAppLifecycle::apply(LifecycleSignal::WindowFocus(window, true), cx));
    cx.run_until_parked();

    press(cx);

    assert_eq!(presented.get(), 1);
}

#[gpui::test]
fn a_second_install_does_not_double_the_listener(cx: &mut TestAppContext) {
    let (presented, window) = counting_trigger(cx);
    cx.update(|cx| {
        install_with(|_, _| panic!("the second install must not listen"), cx);
        RiseAppLifecycle::apply(LifecycleSignal::WindowFocus(window, true), cx);
    });
    cx.run_until_parked();

    press(cx);

    assert_eq!(presented.get(), 1);
}

#[gpui::test]
fn an_inactive_app_ignores_the_action(cx: &mut TestAppContext) {
    let (presented, window) = counting_trigger(cx);

    press(cx);
    assert_eq!(presented.get(), 0, "no window has taken focus yet");

    cx.update(|cx| {
        RiseAppLifecycle::apply(LifecycleSignal::WindowFocus(window, true), cx);
        RiseAppLifecycle::apply(LifecycleSignal::WindowClosed(window), cx);
    });
    cx.run_until_parked();

    press(cx);
    assert_eq!(presented.get(), 0, "the last window closed");
}

#[gpui::test]
fn the_shortcut_lives_in_the_shell_keymap(cx: &mut TestAppContext) {
    cx.update(|cx| {
        shell_actions::install_key_bindings(cx);
        let keymap = cx.key_bindings();
        assert_eq!(keymap.borrow().bindings_for_action(&ReportBug).count(), 1);
    });
}
