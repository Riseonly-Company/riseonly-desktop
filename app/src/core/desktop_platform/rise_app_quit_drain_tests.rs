use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui::TestAppContext;

use super::super::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use super::*;

struct Probe {
    exits: Rc<Cell<usize>>,
    hides: Rc<Cell<usize>>,
    will_terminate: Arc<AtomicUsize>,
    _subscription: RiseSubscription,
}

impl Probe {
    fn will_terminate(&self) -> usize {
        self.will_terminate.load(Ordering::SeqCst)
    }
}

fn install(cx: &mut TestAppContext) -> Probe {
    let events = RiseAppEvents::new();
    let will_terminate = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&will_terminate);
    let subscription = events.observe(UIApplication::WILL_TERMINATE_NOTIFICATION, move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let exits = Rc::new(Cell::new(0));
    let hides = Rc::new(Cell::new(0));
    let (exit_count, hide_count) = (Rc::clone(&exits), Rc::clone(&hides));
    cx.update(|cx| {
        RiseAppLifecycle::install_with(events, cx);
        cx.default_global::<QuitDrain>().actions = QuitActions {
            hide_windows: Rc::new(move |_: &mut App| hide_count.set(hide_count.get() + 1)),
            exit: Rc::new(move |_: &mut App| exit_count.set(exit_count.get() + 1)),
        };
    });
    Probe { exits, hides, will_terminate, _subscription: subscription }
}

fn active_leases(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| cx.global::<QuitDrain>().ledger.lock().active.len())
}

#[gpui::test]
fn quit_without_leases_exits_at_once_and_keeps_windows(cx: &mut TestAppContext) {
    let probe = install(cx);

    cx.update(request_quit);

    assert_eq!(probe.exits.get(), 1);
    assert_eq!(probe.hides.get(), 0);
    assert_eq!(probe.will_terminate(), 1);
}

#[gpui::test]
fn a_system_request_waits_for_the_lease_and_a_repeat_joins_it(cx: &mut TestAppContext) {
    let probe = install(cx);
    let lease = cx.update(|cx| QuitDrain::lease("publish", cx));
    let (sender, requests) = mpsc::unbounded();

    cx.update(|cx| forward_requests(requests, cx));
    sender.unbounded_send(()).ok();
    sender.unbounded_send(()).ok();
    cx.run_until_parked();

    assert_eq!(probe.exits.get(), 0);
    assert_eq!(probe.hides.get(), 1);
    assert_eq!(probe.will_terminate(), 1);

    drop(lease);
    cx.run_until_parked();

    assert_eq!(probe.exits.get(), 1);
}

#[gpui::test]
fn quit_with_a_lease_hides_and_exits_when_it_is_released(cx: &mut TestAppContext) {
    let probe = install(cx);
    let lease = cx.update(|cx| QuitDrain::lease("publish", cx));

    cx.update(request_quit);
    cx.run_until_parked();

    assert_eq!(probe.exits.get(), 0);
    assert_eq!(probe.hides.get(), 1);
    assert_eq!(probe.will_terminate(), 1);

    drop(lease);
    cx.run_until_parked();

    assert_eq!(probe.exits.get(), 1);
    assert_eq!(active_leases(cx), 0);
}

#[gpui::test]
fn quit_exits_only_after_the_last_lease(cx: &mut TestAppContext) {
    let probe = install(cx);
    let first = cx.update(|cx| QuitDrain::lease("upload", cx));
    let second = cx.update(|cx| QuitDrain::lease("publish", cx));

    cx.update(request_quit);
    cx.run_until_parked();

    drop(first);
    cx.run_until_parked();
    assert_eq!(probe.exits.get(), 0);

    drop(second);
    cx.run_until_parked();
    assert_eq!(probe.exits.get(), 1);
}

#[gpui::test]
fn a_lease_taken_during_the_wait_keeps_the_exit_waiting(cx: &mut TestAppContext) {
    let probe = install(cx);
    let upload = cx.update(|cx| QuitDrain::lease("upload", cx));

    cx.update(request_quit);
    cx.run_until_parked();

    let publish = cx.update(|cx| QuitDrain::lease("publish", cx));
    drop(upload);
    cx.run_until_parked();
    assert_eq!(probe.exits.get(), 0);

    drop(publish);
    cx.run_until_parked();
    assert_eq!(probe.exits.get(), 1);
}

#[gpui::test]
fn quit_exits_at_the_limit_with_a_lease_still_held(cx: &mut TestAppContext) {
    let probe = install(cx);
    let lease = cx.update(|cx| QuitDrain::lease("publish", cx));

    cx.update(request_quit);
    cx.run_until_parked();

    cx.executor().advance_clock(drain_limit() - Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(probe.exits.get(), 0);

    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(probe.exits.get(), 1);
    assert_eq!(active_leases(cx), 1);

    drop(lease);
    cx.run_until_parked();
    assert_eq!(probe.exits.get(), 1);
}

#[gpui::test]
fn the_limit_is_one_for_the_whole_exit(cx: &mut TestAppContext) {
    let probe = install(cx);
    let upload = cx.update(|cx| QuitDrain::lease("upload", cx));

    cx.update(request_quit);
    cx.run_until_parked();

    cx.executor().advance_clock(drain_limit() - Duration::from_millis(1));
    let publish = cx.update(|cx| QuitDrain::lease("publish", cx));
    drop(upload);
    cx.run_until_parked();
    assert_eq!(probe.exits.get(), 0);

    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(probe.exits.get(), 1);

    drop(publish);
}

#[gpui::test]
fn repeated_requests_post_will_terminate_and_exit_once(cx: &mut TestAppContext) {
    let probe = install(cx);
    let lease = cx.update(|cx| QuitDrain::lease("publish", cx));

    cx.update(|cx| {
        request_quit(cx);
        request_quit(cx);
    });
    cx.update(request_quit);
    cx.run_until_parked();
    assert_eq!(probe.will_terminate(), 1);
    assert_eq!(probe.hides.get(), 1);

    drop(lease);
    cx.run_until_parked();
    cx.update(request_quit);
    cx.run_until_parked();

    assert_eq!(probe.exits.get(), 1);
    assert_eq!(probe.will_terminate(), 1);
}

#[gpui::test]
fn the_quit_signal_of_the_real_exit_does_not_post_will_terminate_again(cx: &mut TestAppContext) {
    let probe = install(cx);

    cx.update(request_quit);
    cx.update(|cx| RiseAppLifecycle::apply(LifecycleSignal::Quit, cx));

    assert_eq!(probe.will_terminate(), 1);
}

#[gpui::test]
fn a_lease_released_before_the_request_does_not_hold_the_exit(cx: &mut TestAppContext) {
    let probe = install(cx);
    let lease = cx.update(|cx| QuitDrain::lease("publish", cx));
    drop(lease);

    cx.update(request_quit);

    assert_eq!(probe.exits.get(), 1);
    assert_eq!(probe.hides.get(), 0);
}
