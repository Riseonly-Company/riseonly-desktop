use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::TestAppContext;
use rise_theme::AppTheme;

use super::*;
use crate::app::root_view::RootView;
use crate::app::shell::window_presenter::WindowPresenter;
use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::desktop_platform::rise_app_lifecycle::LifecycleSignal;

const PAST_THE_FOCUS_SETTLE: Duration = Duration::from_secs(1);

#[gpui::test]
fn two_windows_run_the_process_work_once_per_phase_change(cx: &mut TestAppContext) {
    let seen: Rc<RefCell<Vec<ScenePhase>>> = Rc::default();
    let recorded = seen.clone();
    cx.update(|cx| {
        if !cx.has_global::<AppTheme>() {
            rise_ui::install_theme(AppTheme::dark(), cx);
        }
        RiseAppLifecycle::install_with(RiseAppEvents::new(), cx);
        RootProcessLifecycle::install_with(Rc::new(move |phase: ScenePhase, _: &mut App| recorded.borrow_mut().push(phase)), cx);
    });
    let first = cx.update(|cx| WindowPresenter::open_handle(cx, RootView::new)).expect("the shell window opens");
    let second = cx.update(|cx| WindowPresenter::open_handle(cx, RootView::new)).expect("the shell window opens");
    let first_id = gpui::AnyWindowHandle::from(first).window_id();
    let second_id = gpui::AnyWindowHandle::from(second).window_id();
    cx.executor().advance_clock(PAST_THE_FOCUS_SETTLE);
    cx.run_until_parked();

    seen.borrow_mut().clear();
    cx.update(|cx| RiseAppLifecycle::apply(LifecycleSignal::WindowFocus(first_id, true), cx));
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), vec![ScenePhase::Active], "one return to the foreground, not one per window");

    seen.borrow_mut().clear();
    cx.update(|cx| {
        RiseAppLifecycle::apply(LifecycleSignal::WindowMinimised(first_id, true), cx);
        RiseAppLifecycle::apply(LifecycleSignal::WindowMinimised(second_id, true), cx);
    });
    cx.executor().advance_clock(PAST_THE_FOCUS_SETTLE);
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), vec![ScenePhase::Background], "one exit to the server, not one per window");
}

#[gpui::test]
fn a_new_window_does_not_bootstrap_the_account_again(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RootProcessLifecycle::install_if_needed(cx);
        RootProcessLifecycle::set_bootstrapped_account_id(Some("account".to_owned()), cx);
        RootProcessLifecycle::install_if_needed(cx);
        assert_eq!(
            RootProcessLifecycle::bootstrapped_account_id(cx),
            Some("account"),
            "installing from a second root view keeps what the first one recorded"
        );
    });
}
