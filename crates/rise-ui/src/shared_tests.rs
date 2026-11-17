use std::cell::Cell;
use std::rc::Rc;

use gpui::{Context, TestAppContext};

use crate::shared::shared;

struct Counter {
    value: usize,
}

impl Counter {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self { value: 0 }
    }
}

struct Other {
    label: &'static str,
}

#[gpui::test]
fn the_first_call_builds_and_later_calls_return_the_same_entity(cx: &mut TestAppContext) {
    let builds = Rc::new(Cell::new(0_usize));
    let first = cx.update(|cx| {
        let builds = builds.clone();
        shared(cx, move |_| {
            builds.set(builds.get() + 1);
            Counter { value: 0 }
        })
    });
    let second = cx.update(|cx| {
        let builds = builds.clone();
        shared(cx, move |_| {
            builds.set(builds.get() + 1);
            Counter { value: 99 }
        })
    });
    assert_eq!(builds.get(), 1);
    assert_eq!(first.entity_id(), second.entity_id());
    cx.update(|cx| assert_eq!(second.read(cx).value, 0));
}

#[gpui::test]
fn state_written_through_one_handle_is_seen_through_the_next(cx: &mut TestAppContext) {
    cx.update(|cx| {
        shared(cx, Counter::new).update(cx, |counter, cx| {
            counter.value = 7;
            cx.notify();
        });
    });
    let value = cx.update(|cx| shared(cx, Counter::new).read(cx).value);
    assert_eq!(value, 7);
}

#[gpui::test]
fn each_type_owns_its_own_slot(cx: &mut TestAppContext) {
    cx.update(|cx| {
        shared(cx, Counter::new).update(cx, |counter, _| counter.value = 3);
        let other = shared(cx, |_| Other { label: "other" });
        assert_eq!(other.read(cx).label, "other");
        assert_eq!(shared(cx, Counter::new).read(cx).value, 3);
    });
}

#[gpui::test]
fn separate_apps_get_separate_instances(cx_a: &mut TestAppContext, cx_b: &mut TestAppContext) {
    cx_a.update(|cx| shared(cx, Counter::new).update(cx, |counter, _| counter.value = 5));
    let value_b = cx_b.update(|cx| shared(cx, Counter::new).read(cx).value);
    assert_eq!(value_b, 0);
}
