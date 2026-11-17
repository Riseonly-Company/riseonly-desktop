use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::Mutex;

use super::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication,
};

const FIRST: RiseNotificationName = RiseNotificationName::new("RiseTestFirst");
const SECOND: RiseNotificationName = RiseNotificationName::new("RiseTestSecond");

type PayloadSeen = Arc<Mutex<Option<(Option<u32>, bool)>>>;

fn recorder() -> Arc<Mutex<Vec<String>>> {
    Arc::new(Mutex::new(Vec::new()))
}

#[test]
fn observers_are_called_in_registration_order() {
    let center = RiseAppEvents::new();
    let log = recorder();
    let mut subscriptions = Vec::new();
    for label in ["a", "b", "c"] {
        let log = Arc::clone(&log);
        subscriptions.push(center.observe(FIRST, move |_| log.lock().push(label.to_owned())));
    }

    center.post(FIRST);

    assert_eq!(*log.lock(), vec!["a", "b", "c"]);
}

#[test]
fn post_is_synchronous_and_only_reaches_the_named_observers() {
    let center = RiseAppEvents::new();
    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let _a = {
        let first = Arc::clone(&first);
        center.observe(FIRST, move |_| {
            first.fetch_add(1, Ordering::SeqCst);
        })
    };
    let _b = {
        let second = Arc::clone(&second);
        center.observe(SECOND, move |_| {
            second.fetch_add(1, Ordering::SeqCst);
        })
    };

    center.post(FIRST);

    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 0);
}

#[test]
fn dropping_the_subscription_removes_the_observer() {
    let center = RiseAppEvents::new();
    let hits = Arc::new(AtomicUsize::new(0));
    let subscription = {
        let hits = Arc::clone(&hits);
        center.observe(FIRST, move |_| {
            hits.fetch_add(1, Ordering::SeqCst);
        })
    };
    center.post(FIRST);
    assert_eq!(center.observer_count(FIRST), 1);

    drop(subscription);
    center.post(FIRST);

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(center.observer_count(FIRST), 0);
}

#[test]
fn an_observer_dropped_during_delivery_is_not_called_afterwards() {
    let center = RiseAppEvents::new();
    let log = recorder();
    let victim: Arc<Mutex<Option<RiseSubscription>>> = Arc::new(Mutex::new(None));
    let _killer = {
        let victim = Arc::clone(&victim);
        let log = Arc::clone(&log);
        center.observe(FIRST, move |_| {
            log.lock().push("killer".to_owned());
            victim.lock().take();
        })
    };
    let victim_subscription = {
        let log = Arc::clone(&log);
        center.observe(FIRST, move |_| log.lock().push("victim".to_owned()))
    };
    *victim.lock() = Some(victim_subscription);

    center.post(FIRST);

    assert_eq!(*log.lock(), vec!["killer"]);
}

#[test]
fn a_nested_post_is_delivered_before_the_outer_post_continues() {
    let center = RiseAppEvents::new();
    let log = recorder();
    let _outer = {
        let log = Arc::clone(&log);
        let inner_center = center.clone();
        center.observe(FIRST, move |_| {
            log.lock().push("first:1".to_owned());
            inner_center.post(SECOND);
        })
    };
    let _later = {
        let log = Arc::clone(&log);
        center.observe(FIRST, move |_| log.lock().push("first:2".to_owned()))
    };
    let _inner = {
        let log = Arc::clone(&log);
        center.observe(SECOND, move |_| log.lock().push("second".to_owned()))
    };

    center.post(FIRST);

    assert_eq!(*log.lock(), vec!["first:1", "second", "first:2"]);
}

#[test]
fn an_observer_added_during_delivery_waits_for_the_next_post() {
    let center = RiseAppEvents::new();
    let hits = Arc::new(AtomicUsize::new(0));
    let added: Arc<Mutex<Vec<RiseSubscription>>> = Arc::new(Mutex::new(Vec::new()));
    let _adder = {
        let added = Arc::clone(&added);
        let hits = Arc::clone(&hits);
        let inner_center = center.clone();
        center.observe(FIRST, move |_| {
            let hits = Arc::clone(&hits);
            added.lock().push(inner_center.observe(FIRST, move |_| {
                hits.fetch_add(1, Ordering::SeqCst);
            }));
        })
    };

    center.post(FIRST);
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    center.post(FIRST);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[test]
fn payload_is_typed_and_absent_for_another_type() {
    let center = RiseAppEvents::new();
    let seen: PayloadSeen = Arc::new(Mutex::new(None));
    let _s = {
        let seen = Arc::clone(&seen);
        center.observe(FIRST, move |n| {
            *seen.lock() = Some((n.payload::<u32>().copied(), n.payload::<String>().is_some()));
        })
    };

    center.post_with(FIRST, 7_u32);

    assert_eq!(*seen.lock(), Some((Some(7), false)));
}

#[test]
fn observe_all_sees_every_name() {
    let center = RiseAppEvents::new();
    let log = recorder();
    let _all = {
        let log = Arc::clone(&log);
        center.observe_all(move |n| log.lock().push(n.name.raw_value().to_owned()))
    };

    center.post(FIRST);
    center.post(SECOND);

    assert_eq!(*log.lock(), vec!["RiseTestFirst", "RiseTestSecond"]);
}

#[test]
fn a_post_from_another_thread_runs_on_that_thread() {
    let center = RiseAppEvents::new();
    let poster = Arc::new(Mutex::new(None));
    let _s = {
        let poster = Arc::clone(&poster);
        center.observe(FIRST, move |_| {
            *poster.lock() = Some(std::thread::current().id());
        })
    };

    let thread_center = center.clone();
    let handle = std::thread::Builder::new()
        .name("rise-app-events-test".to_owned())
        .spawn(move || {
            thread_center.post(FIRST);
            std::thread::current().id()
        })
        .expect("test thread starts");
    let posting_thread = handle.join().expect("test thread finishes");

    assert_eq!(*poster.lock(), Some(posting_thread));
}

#[test]
fn ui_application_names_keep_the_ios_raw_values() {
    assert_eq!(
        UIApplication::DID_BECOME_ACTIVE_NOTIFICATION.raw_value(),
        "UIApplicationDidBecomeActiveNotification"
    );
    assert_eq!(
        UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION.raw_value(),
        "UIApplicationWillResignActiveNotification"
    );
    assert_eq!(
        UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION.raw_value(),
        "UIApplicationDidEnterBackgroundNotification"
    );
    assert_eq!(
        UIApplication::WILL_TERMINATE_NOTIFICATION.raw_value(),
        "UIApplicationWillTerminateNotification"
    );
}
