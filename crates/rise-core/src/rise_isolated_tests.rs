use std::sync::Arc;
use std::thread;

use super::rise_isolated::RiseIsolated;

#[derive(Default)]
struct Counter {
    value: u64,
}

#[test]
fn with_returns_the_closure_value() {
    let isolated = RiseIsolated::new(Counter { value: 41 });
    let next = isolated.with(|counter| {
        counter.value += 1;
        counter.value
    });
    assert_eq!(next, 42);
    assert_eq!(isolated.with(|counter| counter.value), 42);
}

#[test]
fn two_threads_never_lose_an_increment() {
    let isolated = Arc::new(RiseIsolated::<Counter>::default());
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let isolated = Arc::clone(&isolated);
            thread::spawn(move || {
                for _ in 0..10_000 {
                    isolated.with(|counter| counter.value += 1);
                }
            })
        })
        .collect();
    for worker in workers {
        assert!(worker.join().is_ok());
    }
    assert_eq!(isolated.with(|counter| counter.value), 20_000);
}

#[test]
fn a_panicking_closure_leaves_the_state_usable() {
    let isolated = Arc::new(RiseIsolated::new(Counter { value: 1 }));
    let worker = {
        let isolated = Arc::clone(&isolated);
        thread::spawn(move || {
            isolated.with(|counter| {
                counter.value = 2;
                panic!("closure trapped");
            });
        })
    };
    assert!(worker.join().is_err());
    assert_eq!(isolated.with(|counter| counter.value), 2);
}

#[test]
fn is_send_and_sync_for_send_state() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<RiseIsolated<Counter>>();
}
