use std::cell::Cell;
use std::rc::Rc;

use gpui::TestAppContext;

use crate::overlay::blocking_loader_store::BlockingLoaderStore;

fn is_visible(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| BlockingLoaderStore::shared(cx).read(cx).is_visible())
}

#[gpui::test]
fn starts_hidden_and_shows_on_request(cx: &mut TestAppContext) {
    assert!(!is_visible(cx));
    cx.update(|cx| BlockingLoaderStore::shared(cx).update(cx, |store, cx| store.show(|_| {}, cx)));
    assert!(is_visible(cx));
}

#[gpui::test]
fn hide_drops_the_cancel_callback(cx: &mut TestAppContext) {
    let cancelled = Rc::new(Cell::new(0));
    let counter = cancelled.clone();
    cx.update(|cx| {
        let store = BlockingLoaderStore::shared(cx);
        store.update(cx, |store, cx| store.show(move |_| counter.set(counter.get() + 1), cx));
        store.update(cx, |store, cx| store.hide(cx));
        store.update(cx, |store, cx| store.cancel(cx));
    });
    assert!(!is_visible(cx));
    assert_eq!(cancelled.get(), 0);
}

#[gpui::test]
fn cancel_hides_and_runs_the_callback_once(cx: &mut TestAppContext) {
    let cancelled = Rc::new(Cell::new(0));
    let counter = cancelled.clone();
    cx.update(|cx| {
        let store = BlockingLoaderStore::shared(cx);
        store.update(cx, |store, cx| store.show(move |_| counter.set(counter.get() + 1), cx));
        store.update(cx, |store, cx| store.cancel(cx));
        store.update(cx, |store, cx| store.cancel(cx));
    });
    cx.run_until_parked();
    assert!(!is_visible(cx));
    assert_eq!(cancelled.get(), 1);
}

#[gpui::test]
fn the_callback_may_reach_back_into_the_store(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let store = BlockingLoaderStore::shared(cx);
        store.update(cx, |store, cx| {
            store.show(|cx| BlockingLoaderStore::shared(cx).update(cx, |store, cx| store.hide(cx)), cx)
        });
        store.update(cx, |store, cx| store.cancel(cx));
    });
    cx.run_until_parked();
    assert!(!is_visible(cx));
}

#[gpui::test]
fn reset_account_state_hides(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let store = BlockingLoaderStore::shared(cx);
        store.update(cx, |store, cx| store.show(|_| {}, cx));
        store.update(cx, |store, cx| store.reset_account_state(cx));
    });
    assert!(!is_visible(cx));
    assert!((BlockingLoaderStore::APPEARANCE_DELAY - 1.0).abs() < f64::EPSILON);
}
