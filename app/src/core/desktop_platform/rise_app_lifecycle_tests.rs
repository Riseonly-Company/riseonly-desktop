use std::sync::Arc;

use gpui::TestAppContext;
use parking_lot::Mutex;

use super::super::rise_app_events::RiseSubscription;
use super::*;

fn window(n: u64) -> WindowId {
    WindowId::from(n)
}

#[test]
fn launch_posts_only_did_become_active() {
    let mut state = AppLifecycleState::new();
    assert_eq!(state.settle(), Vec::<RiseNotificationName>::new());

    let names = state.apply(LifecycleSignal::WindowFocus(window(1), true));

    assert_eq!(names, vec![UIApplication::DID_BECOME_ACTIVE_NOTIFICATION]);
    assert_eq!(state.phase(), ScenePhase::Active);
}

#[test]
fn losing_focus_with_a_visible_window_resigns_active_only() {
    let mut state = AppLifecycleState::new();
    state.apply(LifecycleSignal::WindowFocus(window(1), true));

    let names = state.apply(LifecycleSignal::WindowFocus(window(1), false));

    assert_eq!(names, vec![UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION]);
    assert_eq!(state.phase(), ScenePhase::Inactive);
}

#[test]
fn minimising_the_last_window_enters_background_in_ios_order() {
    let mut state = AppLifecycleState::new();
    state.apply(LifecycleSignal::WindowFocus(window(1), true));

    let names = state.apply(LifecycleSignal::WindowMinimised(window(1), true));

    assert_eq!(
        names,
        vec![
            UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION,
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
        ]
    );
    assert_eq!(state.phase(), ScenePhase::Background);
}

#[test]
fn coming_back_from_background_enters_foreground_then_becomes_active() {
    let mut state = AppLifecycleState::new();
    state.apply(LifecycleSignal::WindowFocus(window(1), true));
    state.apply(LifecycleSignal::WindowMinimised(window(1), true));

    let names = state.apply(LifecycleSignal::WindowFocus(window(1), true));

    assert_eq!(
        names,
        vec![
            UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION,
            UIApplication::DID_BECOME_ACTIVE_NOTIFICATION,
        ]
    );
}

#[test]
fn closing_the_last_window_enters_background() {
    let mut state = AppLifecycleState::new();
    state.apply(LifecycleSignal::WindowFocus(window(1), true));
    state.apply(LifecycleSignal::WindowFocus(window(1), false));

    let names = state.apply(LifecycleSignal::WindowClosed(window(1)));

    assert_eq!(names, vec![UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION]);
}

#[test]
fn a_second_visible_window_keeps_the_app_out_of_background() {
    let mut state = AppLifecycleState::new();
    state.apply(LifecycleSignal::WindowFocus(window(1), true));
    state.apply(LifecycleSignal::WindowFocus(window(2), false));

    let names = state.apply(LifecycleSignal::WindowMinimised(window(1), true));

    assert_eq!(names, vec![UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION]);
    assert_eq!(state.phase(), ScenePhase::Inactive);
}

#[test]
fn focus_moving_between_windows_posts_nothing_once_settled() {
    let mut state = AppLifecycleState::new();
    state.apply(LifecycleSignal::WindowFocus(window(1), true));

    state.record(LifecycleSignal::WindowFocus(window(1), false));
    state.record(LifecycleSignal::WindowFocus(window(2), true));

    assert_eq!(state.settle(), Vec::<RiseNotificationName>::new());
    assert_eq!(state.phase(), ScenePhase::Active);
}

#[test]
fn quit_from_active_passes_through_background_once_and_ignores_later_signals() {
    let mut state = AppLifecycleState::new();
    state.apply(LifecycleSignal::WindowFocus(window(1), true));

    assert_eq!(
        state.apply(LifecycleSignal::Quit),
        vec![
            UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION,
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
            UIApplication::WILL_TERMINATE_NOTIFICATION,
        ]
    );
    assert_eq!(state.phase(), ScenePhase::Background);
    assert_eq!(
        state.apply(LifecycleSignal::WindowClosed(window(1))),
        Vec::<RiseNotificationName>::new()
    );
    assert!(state.is_terminated());
}

#[test]
fn quit_from_inactive_enters_background_then_terminates() {
    let mut state = AppLifecycleState::new();
    state.apply(LifecycleSignal::WindowFocus(window(1), true));
    state.apply(LifecycleSignal::WindowFocus(window(1), false));

    assert_eq!(
        state.apply(LifecycleSignal::Quit),
        vec![
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
            UIApplication::WILL_TERMINATE_NOTIFICATION,
        ]
    );
}

#[test]
fn quit_from_background_posts_only_will_terminate() {
    let mut state = AppLifecycleState::new();
    state.apply(LifecycleSignal::WindowFocus(window(1), true));
    state.apply(LifecycleSignal::WindowClosed(window(1)));

    assert_eq!(
        state.apply(LifecycleSignal::Quit),
        vec![UIApplication::WILL_TERMINATE_NOTIFICATION]
    );
}

#[test]
fn transition_table_matches_ios() {
    use ScenePhase::{Active, Background, Inactive};
    assert!(transition(Active, Active).is_empty());
    assert_eq!(
        transition(Inactive, Active),
        vec![UIApplication::DID_BECOME_ACTIVE_NOTIFICATION]
    );
    assert_eq!(
        transition(Background, Inactive),
        vec![UIApplication::WILL_ENTER_FOREGROUND_NOTIFICATION]
    );
    assert_eq!(
        transition(Inactive, Background),
        vec![UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION]
    );
}

fn recorded(events: &RiseAppEvents) -> (Arc<Mutex<Vec<&'static str>>>, RiseSubscription) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    let subscription = events.observe_all(move |n| sink.lock().push(n.name.raw_value()));
    (log, subscription)
}

#[gpui::test]
fn the_global_posts_activation_at_once_and_focus_loss_after_settling(cx: &mut TestAppContext) {
    let events = RiseAppEvents::new();
    let (log, _subscription) = recorded(&events);
    cx.update(|cx| {
        RiseAppLifecycle::install_with(events.clone(), cx);
        RiseAppLifecycle::apply(LifecycleSignal::WindowFocus(window(1), true), cx);
    });
    assert_eq!(*log.lock(), vec!["UIApplicationDidBecomeActiveNotification"]);
    assert_eq!(cx.update(|cx| RiseAppLifecycle::scene_phase(cx)), ScenePhase::Active);

    cx.update(|cx| RiseAppLifecycle::apply(LifecycleSignal::WindowFocus(window(1), false), cx));
    assert_eq!(log.lock().len(), 1);

    cx.executor().advance_clock(FOCUS_LOSS_SETTLE);
    cx.run_until_parked();

    assert_eq!(
        *log.lock(),
        vec![
            "UIApplicationDidBecomeActiveNotification",
            "UIApplicationWillResignActiveNotification",
        ]
    );
    assert_eq!(cx.update(|cx| RiseAppLifecycle::scene_phase(cx)), ScenePhase::Inactive);
}

#[gpui::test]
fn refocus_within_the_settle_window_posts_nothing(cx: &mut TestAppContext) {
    let events = RiseAppEvents::new();
    let (log, _subscription) = recorded(&events);
    cx.update(|cx| {
        RiseAppLifecycle::install_with(events.clone(), cx);
        RiseAppLifecycle::apply(LifecycleSignal::WindowFocus(window(1), true), cx);
        RiseAppLifecycle::apply(LifecycleSignal::WindowFocus(window(1), false), cx);
        RiseAppLifecycle::apply(LifecycleSignal::WindowFocus(window(2), true), cx);
    });

    cx.executor().advance_clock(FOCUS_LOSS_SETTLE);
    cx.run_until_parked();

    assert_eq!(*log.lock(), vec!["UIApplicationDidBecomeActiveNotification"]);
}
