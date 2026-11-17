use std::cell::Cell;
use std::rc::Rc;

use gpui::{AppContext, Context, Entity, IntoElement, Render, TestAppContext, Window, div};

use super::binding::Binding;

struct Counter {
    value: i32,
    enabled: bool,
}

impl Counter {
    fn set_value(&mut self, value: i32) {
        self.value = value;
    }
}

struct Host;

impl Render for Host {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn counter(cx: &mut TestAppContext) -> Entity<Counter> {
    cx.new(|_| Counter {
        value: 1,
        enabled: false,
    })
}

#[gpui::test]
fn entity_binding_reads_writes_and_notifies(cx: &mut TestAppContext) {
    let store = counter(cx);
    let notified = Rc::new(Cell::new(0));
    let seen = notified.clone();
    cx.update(|cx| {
        cx.observe(&store, move |_, _| seen.set(seen.get() + 1))
            .detach();
    });
    let binding = Binding::entity(
        store.clone(),
        |counter: &Counter| counter.value,
        |counter, value, _| counter.set_value(value),
    );
    let window = cx.add_window(|_, _| Host);

    cx.update(|cx| assert_eq!(binding.get(cx), 1));
    window
        .update(cx, |_, window, cx| binding.set(5, window, cx))
        .unwrap();
    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(binding.get(cx), 5);
        assert_eq!(store.read(cx).value, 5);
    });
    assert_eq!(notified.get(), 1, "the setter has to notify the store's observers");
}

#[gpui::test]
fn toggle_flips_a_bool_binding(cx: &mut TestAppContext) {
    let store = counter(cx);
    let binding = Binding::entity(
        store.clone(),
        |counter: &Counter| counter.enabled,
        |counter, enabled, _| counter.enabled = enabled,
    );
    let window = cx.add_window(|_, _| Host);

    window
        .update(cx, |_, window, cx| binding.toggle(window, cx))
        .unwrap();
    cx.update(|cx| assert!(binding.get(cx)));
    window
        .update(cx, |_, window, cx| binding.toggle(window, cx))
        .unwrap();
    cx.update(|cx| assert!(!binding.get(cx)));
}

#[gpui::test]
fn constant_binding_drops_writes(cx: &mut TestAppContext) {
    let binding = Binding::constant(3);
    let window = cx.add_window(|_, _| Host);

    window
        .update(cx, |_, window, cx| binding.set(9, window, cx))
        .unwrap();
    cx.update(|cx| assert_eq!(binding.get(cx), 3));
}

#[gpui::test]
fn closure_binding_and_its_clone_share_the_value(cx: &mut TestAppContext) {
    let cell = Rc::new(Cell::new(0));
    let reader = cell.clone();
    let writer = cell.clone();
    let binding = Binding::new(move |_| reader.get(), move |value, _, _| writer.set(value));
    let copy = binding.clone();
    let window = cx.add_window(|_, _| Host);

    window
        .update(cx, |_, window, cx| copy.set(4, window, cx))
        .unwrap();
    cx.update(|cx| assert_eq!(binding.get(cx), 4));
    assert_eq!(cell.get(), 4);
}
