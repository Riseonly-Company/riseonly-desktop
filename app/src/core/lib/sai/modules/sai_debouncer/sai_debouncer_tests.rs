use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::TestAppContext;

use super::*;

type Log = Rc<RefCell<Vec<String>>>;

fn record(log: &Log, entry: &str) -> SaiDebouncedCallback {
    let log = Rc::clone(log);
    let entry = entry.to_owned();
    Box::new(move |_| log.borrow_mut().push(entry))
}

fn options(group_key: &str, max_cache_data: i64) -> SaiDebouncerOptions {
    SaiDebouncerOptions {
        group_key: group_key.to_owned(),
        max_cache_data,
        ..SaiDebouncerOptions::default()
    }
}

#[test]
fn defaults_match_ios() {
    let defaults = SaiDebouncerOptions::default();
    assert_eq!(defaults.delay_milliseconds, 500);
    assert_eq!(defaults.group_key, "default");
    assert_eq!(defaults.max_cache_data, 10);
}

#[gpui::test]
fn a_new_call_replaces_the_pending_one(cx: &mut TestAppContext) {
    let log: Log = Rc::default();
    let handle = cx.update(|cx| {
        SaiDebouncer::debounced_action_id("tag", record(&log, "first"), options("search", 10), cx);
        SaiDebouncer::debounced_action_id("tag", record(&log, "second"), options("search", 10), cx)
    });
    assert_eq!(handle.composite_key, "search_tag");
    assert!(cx.update(|cx| handle.is_debouncing(cx)));

    cx.executor().advance_clock(Duration::from_millis(499));
    cx.run_until_parked();
    assert!(log.borrow().is_empty());

    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(*log.borrow(), vec!["second"]);
    assert!(!cx.update(|cx| handle.is_debouncing(cx)));
}

#[gpui::test]
fn flush_runs_at_once_and_the_timer_stays_silent(cx: &mut TestAppContext) {
    let log: Log = Rc::default();
    let handle =
        cx.update(|cx| SaiDebouncer::debounced_action_id(7, record(&log, "flushed"), SaiDebouncerOptions::default(), cx));
    cx.update(|cx| handle.flush(cx));
    assert_eq!(*log.borrow(), vec!["flushed"]);

    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(log.borrow().len(), 1);
}

#[gpui::test]
fn cancel_calls_on_invalidate_and_drops_the_action(cx: &mut TestAppContext) {
    let log: Log = Rc::default();
    cx.update(|cx| {
        SaiDebouncer::debounced_action_on_invalidate(
            "a",
            record(&log, "action"),
            SaiDebouncerOptions::default(),
            Some(record(&log, "invalidated")),
            cx,
        );
        SaiDebouncer::cancel("a", "default", cx);
    });
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(*log.borrow(), vec!["invalidated"]);
}

#[gpui::test]
fn the_oldest_entries_of_a_full_group_are_cancelled(cx: &mut TestAppContext) {
    let log: Log = Rc::default();
    cx.update(|cx| {
        for id in ["a", "b", "c"] {
            SaiDebouncer::debounced_action_id(id, record(&log, id), options("feed", 2), cx);
        }
    });
    let mut active = cx.update(|cx| SaiDebouncer::active_ids("feed", cx));
    active.sort();
    assert_eq!(active, vec!["b", "c"]);

    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    let mut fired = log.borrow().clone();
    fired.sort();
    assert_eq!(fired, vec!["b", "c"]);
}

#[gpui::test]
fn cancel_all_clears_every_group(cx: &mut TestAppContext) {
    let log: Log = Rc::default();
    cx.update(|cx| {
        SaiDebouncer::debounced_action_id("x", record(&log, "x"), options("one", 10), cx);
        SaiDebouncer::debounced_action_id("y", record(&log, "y"), options("two", 10), cx);
        sai_cancel_all_debounced_actions(cx);
    });
    assert_eq!(cx.update(|cx| SaiDebouncer::active_count("one", cx)), 0);
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(log.borrow().is_empty());
}
