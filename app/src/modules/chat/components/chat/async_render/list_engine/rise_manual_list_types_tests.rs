use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext, Bounds, Empty, Entity, Pixels, TestAppContext, VisualTestContext, Window, point, px, size,
};

use super::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListTransaction, RiseManualListTransactionAnchor,
    RiseManualListTransactionEdgeMutation,
};
use super::rise_manual_list_view::{RiseManualListTransactionCallback, RiseManualListView};

type EdgeMutation = RiseManualListTransactionEdgeMutation;

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn diff(from: &[&str], to: &[&str]) -> Option<EdgeMutation> {
    EdgeMutation::diff(&ids(from), &ids(to))
}

/// edge mutation with every count defaulting to zero
fn mutation(
    removed_prefix_count: usize,
    removed_suffix_count: usize,
    inserted_prefix_count: usize,
    inserted_suffix_count: usize,
) -> EdgeMutation {
    EdgeMutation::new(removed_prefix_count, removed_suffix_count, inserted_prefix_count, inserted_suffix_count)
}

#[test]
fn equal_ids_give_the_zero_mutation() {
    assert_eq!(diff(&["a", "b"], &["a", "b"]), Some(EdgeMutation::default()));
    assert_eq!(diff(&[], &[]), Some(EdgeMutation::default()));
}

#[test]
fn prefix_insert() {
    assert_eq!(diff(&["a", "b", "c"], &["x", "y", "a", "b", "c"]), Some(mutation(0, 0, 2, 0)));
}

#[test]
fn suffix_insert() {
    assert_eq!(diff(&["a", "b"], &["a", "b", "y"]), Some(mutation(0, 0, 0, 1)));
    assert_eq!(diff(&[], &["a", "b"]), Some(mutation(0, 0, 0, 2)));
}

#[test]
fn inserts_on_both_edges() {
    assert_eq!(diff(&["a", "b"], &["x", "a", "b", "y", "z"]), Some(mutation(0, 0, 1, 2)));
}

#[test]
fn removals_on_both_edges() {
    assert_eq!(diff(&["a", "b", "c", "d", "e"], &["b", "c", "d"]), Some(mutation(1, 1, 0, 0)));
    assert_eq!(diff(&["a", "b", "c"], &["b", "c", "d"]), Some(mutation(1, 0, 0, 1)));
}

#[test]
fn no_retained_run_replaces_everything() {
    assert_eq!(diff(&["a", "b"], &["x", "y", "z"]), Some(mutation(0, 2, 0, 3)));
    assert_eq!(diff(&["a", "b"], &[]), Some(mutation(0, 2, 0, 0)));
}

#[test]
fn longest_run_is_retained() {
    assert_eq!(diff(&["a", "b", "c", "d", "e"], &["d", "e", "x", "a", "b", "c"]), Some(mutation(0, 2, 3, 0)));
}

#[test]
fn repeated_ids_give_no_mutation() {
    assert_eq!(diff(&["a", "a"], &["a", "b"]), None);
    assert_eq!(diff(&["a", "b"], &["b", "b"]), None);
}

#[gpui::test]
fn prefix_insert_with_empty_updated_ids_applies_while_scrolling(cx: &mut TestAppContext) {
    let base = rows(0..8);
    let (list, cx) = make_list(cx, base.clone());
    assert!(with_list(&list, cx, |list, window, cx| list.set_content_offset_y(px(150.0), false, true, window, cx)));
    set_user_scrolling(&list, cx, Some(true));
    let anchor_id = first_visible_id(&list, cx);
    let before = list.read_with(cx, |list, _| list.capture_viewport_anchor(&anchor_id)).expect("anchor before");

    let mut next = ids(&["new-0", "new-1"]);
    next.extend(base.iter().cloned());
    let edge_mutation = EdgeMutation::diff(&base, &next).expect("edge mutation");
    let completions = Rc::new(Cell::new(0usize));
    let discards = Rc::new(Cell::new(0usize));
    let transaction = transaction(
        next,
        RiseManualListTransactionAnchor::PreserveVisible,
        Some(HashSet::new()),
        Some(edge_mutation),
        true,
    );
    let (completion, discard) = (counter(&completions), counter(&discards));
    with_list(&list, cx, |list, window, cx| list.enqueue(transaction, Some(completion), Some(discard), window, cx));
    yield_main_turns(cx);

    list.read_with(cx, |list, _| {
        assert!(list.is_scroll_in_progress());
        assert_eq!(completions.get(), 1);
        assert_eq!(discards.get(), 0);
        assert!(!list.has_pending_transaction());
        assert_eq!(list.item_count(), 10);
        assert!((list.scroll_metrics().content_height - px(600.0)).abs() <= px(0.5));
        let after = list.capture_viewport_anchor(&anchor_id).expect("anchor after");
        assert!((after.viewport_offset - before.viewport_offset).abs() <= 0.5);
    });
    set_user_scrolling(&list, cx, Some(false));
}

#[gpui::test]
fn suffix_insert_with_empty_updated_ids_applies_while_scrolling(cx: &mut TestAppContext) {
    let base = rows(0..8);
    let (list, cx) = make_list(cx, base.clone());
    assert!(with_list(&list, cx, |list, window, cx| list.set_content_offset_y(px(150.0), false, true, window, cx)));
    set_user_scrolling(&list, cx, Some(true));
    let anchor_id = first_visible_id(&list, cx);
    let before = list.read_with(cx, |list, _| list.capture_viewport_anchor(&anchor_id)).expect("anchor before");

    let mut next = base.clone();
    next.extend(ids(&["new-8", "new-9"]));
    let edge_mutation = EdgeMutation::diff(&base, &next).expect("edge mutation");
    let completions = Rc::new(Cell::new(0usize));
    let transaction = transaction(
        next,
        RiseManualListTransactionAnchor::PreserveVisible,
        Some(HashSet::new()),
        Some(edge_mutation),
        true,
    );
    let completion = counter(&completions);
    with_list(&list, cx, |list, window, cx| list.enqueue(transaction, Some(completion), None, window, cx));
    yield_main_turns(cx);

    list.read_with(cx, |list, _| {
        assert_eq!(completions.get(), 1);
        assert!(!list.has_pending_transaction());
        assert_eq!(list.item_count(), 10);
        assert!((list.scroll_metrics().content_height - px(600.0)).abs() <= px(0.5));
        let after = list.capture_viewport_anchor(&anchor_id).expect("anchor after");
        assert!((after.viewport_offset - before.viewport_offset).abs() <= 0.5);
    });
    set_user_scrolling(&list, cx, Some(false));
}

#[gpui::test]
fn prefix_insert_without_updated_ids_waits_for_the_scroll_to_end(cx: &mut TestAppContext) {
    let base = rows(0..8);
    let (list, cx) = make_list(cx, base.clone());
    assert!(with_list(&list, cx, |list, window, cx| list.set_content_offset_y(px(150.0), false, true, window, cx)));
    set_user_scrolling(&list, cx, Some(true));
    let anchor_id = first_visible_id(&list, cx);
    let before = list.read_with(cx, |list, _| list.capture_viewport_anchor(&anchor_id)).expect("anchor before");

    let mut next = ids(&["new-0", "new-1"]);
    next.extend(base.iter().cloned());
    let edge_mutation = EdgeMutation::diff(&base, &next).expect("edge mutation");
    let completions = Rc::new(Cell::new(0usize));
    let discards = Rc::new(Cell::new(0usize));
    let transaction =
        transaction(next, RiseManualListTransactionAnchor::PreserveVisible, None, Some(edge_mutation), true);
    let (completion, discard) = (counter(&completions), counter(&discards));
    with_list(&list, cx, |list, window, cx| list.enqueue(transaction, Some(completion), Some(discard), window, cx));
    yield_main_turns(cx);

    list.read_with(cx, |list, _| {
        assert_eq!(completions.get(), 0);
        assert_eq!(discards.get(), 0);
        assert!(list.has_pending_transaction());
        assert_eq!(list.item_count(), 8);
        assert!((list.scroll_metrics().content_height - px(480.0)).abs() <= px(0.5));
    });

    set_user_scrolling(&list, cx, Some(false));
    layout_if_needed(&list, cx);

    list.read_with(cx, |list, _| {
        assert_eq!(completions.get(), 1);
        assert_eq!(discards.get(), 0);
        assert!(!list.has_pending_transaction());
        assert_eq!(list.item_count(), 10);
        let after = list.capture_viewport_anchor(&anchor_id).expect("anchor after");
        assert!((after.viewport_offset - before.viewport_offset).abs() <= 0.5);
    });
}

#[gpui::test]
fn newer_enqueue_discards_the_pending_transaction(cx: &mut TestAppContext) {
    let base = rows(0..8);
    let (list, cx) = make_list(cx, base.clone());
    set_user_scrolling(&list, cx, Some(true));

    let events: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
    let deleted: Vec<String> = base.iter().filter(|id| id.as_str() != "row-7").cloned().collect();
    let delete = transaction(deleted, RiseManualListTransactionAnchor::PreserveVisible, None, None, false);
    let (completion, discard) = (event(&events, "delete.completion"), event(&events, "delete.discard"));
    with_list(&list, cx, |list, window, cx| list.enqueue(delete, Some(completion), Some(discard), window, cx));
    assert!(list.read_with(cx, |list, _| list.has_pending_transaction()));
    assert!(events.borrow().is_empty());

    let mut next = base.clone();
    next.push("row-8".to_string());
    let edge_mutation = EdgeMutation::diff(&base, &next);
    let append =
        transaction(next, RiseManualListTransactionAnchor::PreserveVisible, Some(HashSet::new()), edge_mutation, false);
    let (completion, discard) = (event(&events, "append.completion"), event(&events, "append.discard"));
    with_list(&list, cx, |list, window, cx| list.enqueue(append, Some(completion), Some(discard), window, cx));
    assert_eq!(*events.borrow(), vec!["delete.discard"]);
    assert!(list.read_with(cx, |list, _| list.has_pending_transaction()));

    set_user_scrolling(&list, cx, Some(false));
    layout_if_needed(&list, cx);

    assert_eq!(*events.borrow(), vec!["delete.discard", "append.completion"]);
    list.read_with(cx, |list, _| {
        assert!(!list.has_pending_transaction());
        assert_eq!(list.item_count(), 9);
    });
}

fn frame() -> Bounds<Pixels> {
    Bounds::new(point(px(0.0), px(0.0)), size(px(320.0), px(150.0)))
}

fn rows(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|index| format!("row-{index}")).collect()
}

fn row() -> RiseManualListItem {
    RiseManualListItem::new(
        60.0,
        "edge-contract".to_string(),
        1,
        RiseManualListItemMountPolicy::Immediate,
        None,
        None,
        None,
        None,
        Rc::new(|reusable: Option<AnyView>, _: &mut Window, cx: &mut App| {
            reusable.unwrap_or_else(|| cx.new(|_| Empty).into())
        }),
    )
}

/// transaction with defaults for everything but ids and anchor and updated ids and edge mutation
fn transaction(
    ids: Vec<String>,
    anchor: RiseManualListTransactionAnchor,
    updated_ids: Option<HashSet<String>>,
    edge_mutation: Option<EdgeMutation>,
    allows_application_during_scroll: bool,
) -> RiseManualListTransaction {
    RiseManualListTransaction::new(
        ids,
        anchor,
        None,
        updated_ids,
        None,
        None,
        edge_mutation,
        allows_application_during_scroll,
        HashSet::new(),
        Rc::new(|_: &str, _: f32, _: &mut Window, _: &mut App| row()),
    )
}

fn make_list(cx: &mut TestAppContext, ids: Vec<String>) -> (Entity<RiseManualListView>, &mut VisualTestContext) {
    let (list, cx) = cx.add_window_view(|window, cx| RiseManualListView::new(window, cx));
    cx.simulate_resize(frame().size);
    layout_if_needed(&list, cx);
    let transaction = transaction(ids, RiseManualListTransactionAnchor::Top, None, None, false);
    with_list(&list, cx, |list, window, cx| list.enqueue(transaction, None, None, window, cx));
    (list, cx)
}

fn with_list<R>(
    list: &Entity<RiseManualListView>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut RiseManualListView, &mut Window, &mut gpui::Context<RiseManualListView>) -> R,
) -> R {
    cx.update(|window, app| list.update(app, |list, cx| f(list, window, cx)))
}

/// test helper, layout pass for the list
fn layout_if_needed(list: &Entity<RiseManualListView>, cx: &mut VisualTestContext) {
    with_list(list, cx, |list, window, cx| list.layout_subviews(frame(), window, cx));
}

fn set_user_scrolling(list: &Entity<RiseManualListView>, cx: &mut VisualTestContext, is_scrolling: Option<bool>) {
    with_list(list, cx, |list, window, cx| list.set_user_scrolling_override_for_testing(is_scrolling, window, cx));
}

fn first_visible_id(list: &Entity<RiseManualListView>, cx: &mut VisualTestContext) -> String {
    list.read_with(cx, |list, _| list.visible_ids().first().cloned()).expect("a visible row")
}

fn counter(count: &Rc<Cell<usize>>) -> RiseManualListTransactionCallback {
    let count = count.clone();
    Box::new(move |_: &mut Window, _: &mut App| count.set(count.get() + 1))
}

fn event(events: &Rc<RefCell<Vec<&'static str>>>, name: &'static str) -> RiseManualListTransactionCallback {
    let events = events.clone();
    Box::new(move |_: &mut Window, _: &mut App| events.borrow_mut().push(name))
}

/// scroll safe transactions drain on the next main turn so tests yield a few
fn yield_main_turns(cx: &mut VisualTestContext) {
    cx.run_until_parked();
}
