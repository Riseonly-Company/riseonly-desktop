use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::display_sleep::*;
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Default)]
struct Calls {
    requests: AtomicUsize,
    releases: AtomicUsize,
    reasons: Mutex<Vec<String>>,
}

struct Release(Arc<Calls>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.releases.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeSystem {
    calls: Arc<Calls>,
    refuse: bool,
}

impl DisplaySleepInhibitor for FakeSystem {
    fn inhibit(&self, reason: &str) -> Result<SystemInhibition, DisplaySleepUnavailable> {
        self.calls.requests.fetch_add(1, Ordering::SeqCst);
        self.calls.reasons.lock().push(reason.to_owned());
        if self.refuse {
            return Err(DisplaySleepUnavailable::SystemRefused("denied".to_owned()));
        }
        Ok(SystemInhibition::new(Release(Arc::clone(&self.calls))))
    }
}

struct Unwritten;

impl DisplaySleepInhibitor for Unwritten {}

fn counter(refuse: bool) -> (DisplaySleep, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let sleep = DisplaySleep::new(FakeSystem {
        calls: Arc::clone(&calls),
        refuse,
    });
    (sleep, calls)
}

#[test]
fn the_first_guard_asks_the_system_once() {
    let (sleep, calls) = counter(false);
    let guard = sleep.acquire(CALL_IN_PROGRESS_REASON);
    assert_eq!(calls.requests.load(Ordering::SeqCst), 1);
    assert_eq!(calls.reasons.lock().as_slice(), [CALL_IN_PROGRESS_REASON]);
    assert_eq!(guard.outcome(), &Ok(()));
    assert_eq!(guard.support(), PlatformSupport::Performed);
    assert_eq!(sleep.support(), PlatformSupport::Performed);
}

#[test]
fn a_second_guard_shares_the_request() {
    let (sleep, calls) = counter(false);
    let first = sleep.acquire("first");
    let second = sleep.acquire("second");
    assert_eq!(calls.requests.load(Ordering::SeqCst), 1);
    assert_eq!(sleep.holders(), 2);
    assert_eq!(second.outcome(), &Ok(()));
    drop(first);
    drop(second);
}

#[test]
fn an_old_session_clearing_up_keeps_the_hold_of_a_new_one() {
    let (sleep, calls) = counter(false);
    let old = sleep.acquire("old");
    let new = sleep.acquire("new");
    drop(old);
    assert_eq!(calls.releases.load(Ordering::SeqCst), 0);
    assert_eq!(sleep.support(), PlatformSupport::Performed);
    drop(new);
    assert_eq!(calls.releases.load(Ordering::SeqCst), 1);
    assert_eq!(sleep.support(), PlatformSupport::Unsupported);
    assert_eq!(sleep.holders(), 0);
}

#[test]
fn a_hold_after_the_last_release_asks_the_system_again() {
    let (sleep, calls) = counter(false);
    drop(sleep.acquire("call one"));
    let again = sleep.acquire("call two");
    assert_eq!(calls.requests.load(Ordering::SeqCst), 2);
    assert_eq!(calls.releases.load(Ordering::SeqCst), 1);
    drop(again);
    assert_eq!(calls.releases.load(Ordering::SeqCst), 2);
}

#[test]
fn a_refused_request_is_not_asked_again_while_held() {
    let (sleep, calls) = counter(true);
    let first = sleep.acquire("first");
    let second = sleep.acquire("second");
    assert_eq!(calls.requests.load(Ordering::SeqCst), 1);
    let refused = Err(DisplaySleepUnavailable::SystemRefused("denied".to_owned()));
    assert_eq!(first.outcome(), &refused);
    assert_eq!(second.outcome(), &refused);
    assert_eq!(second.support(), PlatformSupport::Unsupported);
    assert_eq!(sleep.support(), PlatformSupport::Unsupported);
    drop(first);
    drop(second);
    let retry = sleep.acquire("retry");
    assert_eq!(calls.requests.load(Ordering::SeqCst), 2);
    assert_eq!(calls.releases.load(Ordering::SeqCst), 0);
    drop(retry);
}

#[test]
fn an_unwritten_layer_answers_branch_not_written_for_the_host() {
    let sleep = DisplaySleep::new(Unwritten);
    let guard = sleep.acquire(CALL_IN_PROGRESS_REASON);
    assert_eq!(
        guard.outcome(),
        &Err(DisplaySleepUnavailable::BranchNotWritten(HostOs::current()))
    );
    assert_eq!(sleep.holders(), 1);
    drop(guard);
    assert_eq!(sleep.holders(), 0);
}

#[test]
fn guards_released_from_other_threads_release_once() {
    let (sleep, calls) = counter(false);
    let guards: Vec<_> = (0..8).map(|_| sleep.acquire("call")).collect();
    let handles: Vec<_> = guards
        .into_iter()
        .map(|guard| std::thread::spawn(move || drop(guard)))
        .collect();
    for handle in handles {
        handle.join().expect("release thread");
    }
    assert_eq!(calls.requests.load(Ordering::SeqCst), 1);
    assert_eq!(calls.releases.load(Ordering::SeqCst), 1);
    assert_eq!(sleep.holders(), 0);
}
