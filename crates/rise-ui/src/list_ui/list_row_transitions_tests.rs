use std::rc::Rc;
use std::time::Duration;

use gpui::{Context, Entity, IntoElement, ListAlignment, ListOffset, ParentElement, Render, Styled, TestAppContext, VisualTestContext, Window, div, px};
use rise_theme::AppTheme;

use crate::list_ui::list_row_transitions::{ListRowTransitions, ListShownRow, merged_rows};
use crate::list_ui::list_ui::ListUi;
use crate::swiftui::animated::MotionTime;
use crate::swiftui::motion::{MotionSpec, ease_in_out};

const AFTER_THE_MOTION: Duration = Duration::from_secs(1);

struct Probe {
    list: Rc<ListUi>,
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.list.element(|_, _, _| div().w_full().h(px(40.0)).into_any_element());
        div().w(px(200.0)).h(px(400.0)).child(list)
    }
}

/// syncs from render where sync is meant to run
/// a running motion asks the window for the next frame and needs the view being drawn
struct SyncingProbe {
    list: Rc<ListUi>,
    transitions: ListRowTransitions<()>,
    pending: Option<(Vec<(String, ())>, Option<MotionSpec>, MotionTime)>,
    shown: Vec<(String, bool)>,
}

impl SyncingProbe {
    fn new(list: Rc<ListUi>) -> Self {
        Self { list, transitions: ListRowTransitions::new(), pending: None, shown: Vec::new() }
    }
}

impl Render for SyncingProbe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((next, spec, now)) = self.pending.take() {
            let frame = self.transitions.sync(&self.list, next, spec, now, window, cx);
            self.shown = frame.rows().iter().map(|row| (row.id.clone(), row.is_leaving)).collect();
        }
        let list = self.list.element(|_, _, _| div().w_full().h(px(40.0)).into_any_element());
        div().w(px(200.0)).h(px(400.0)).child(list)
    }
}

fn sync_in_render(
    probe: &Entity<SyncingProbe>,
    cx: &mut VisualTestContext,
    next: Vec<(String, ())>,
    spec: Option<MotionSpec>,
    now: MotionTime,
) {
    probe.update(cx, |probe, cx| {
        probe.pending = Some((next, spec, now));
        cx.notify();
    });
    cx.run_until_parked();
}

fn rows(ids: &[&str]) -> Vec<(String, ())> {
    ids.iter().map(|id| ((*id).to_owned(), ())).collect()
}

fn shown_ids(rows: &[ListShownRow<()>]) -> Vec<&str> {
    rows.iter().map(|row| row.id.as_str()).collect()
}

fn shown(ids: &[&str]) -> Vec<ListShownRow<()>> {
    ids.iter().map(|id| ListShownRow { id: (*id).to_owned(), row: (), is_leaving: false }).collect()
}

#[test]
fn a_leaving_row_stays_behind_the_row_it_followed() {
    let merged = merged_rows(&shown(&["a", "b", "c"]), rows(&["a", "c"]), |id| id == "b");
    assert_eq!(shown_ids(&merged), ["a", "b", "c"]);
    assert!(merged[1].is_leaving);
    assert!(!merged[0].is_leaving);
}

#[test]
fn a_leaving_first_row_stays_first() {
    let merged = merged_rows(&shown(&["a", "b"]), rows(&["b"]), |id| id == "a");
    assert_eq!(shown_ids(&merged), ["a", "b"]);
}

#[test]
fn a_row_that_finished_leaving_is_gone() {
    let merged = merged_rows(&shown(&["a", "b"]), rows(&["b"]), |_| false);
    assert_eq!(shown_ids(&merged), ["b"]);
}

#[gpui::test]
fn rows_inserted_under_an_animation_settle(cx: &mut TestAppContext) {
    let theme = AppTheme::dark();
    let list = Rc::new(cx.update(|_| ListUi::new(&theme, 0, ListAlignment::Top)));
    let drawn = list.clone();
    let (probe, cx) = cx.add_window_view(move |_, _| SyncingProbe::new(drawn));

    sync_in_render(&probe, cx, rows(&["a"]), None, Duration::ZERO);
    assert_eq!(list.item_count(), 1);
    assert!(!probe.read_with(cx, |probe, _| probe.transitions.is_animating()));

    sync_in_render(&probe, cx, rows(&["a", "b"]), Some(ease_in_out(0.18)), Duration::ZERO);
    assert_eq!(list.item_count(), 2);
    assert!(probe.read_with(cx, |probe, _| probe.transitions.is_animating()));

    sync_in_render(&probe, cx, rows(&["a", "b"]), None, AFTER_THE_MOTION);
    assert_eq!(list.item_count(), 2);
    assert!(!probe.read_with(cx, |probe, _| probe.transitions.is_animating()));
}

#[gpui::test]
fn a_row_removed_under_an_animation_leaves_when_the_motion_ends(cx: &mut TestAppContext) {
    let theme = AppTheme::dark();
    let list = Rc::new(cx.update(|_| ListUi::new(&theme, 0, ListAlignment::Top)));
    let drawn = list.clone();
    let (probe, cx) = cx.add_window_view(move |_, _| SyncingProbe::new(drawn));

    sync_in_render(&probe, cx, rows(&["a", "b", "c"]), None, Duration::ZERO);

    sync_in_render(&probe, cx, rows(&["a", "c"]), Some(ease_in_out(0.18)), Duration::ZERO);
    let shown = probe.read_with(cx, |probe, _| probe.shown.clone());
    assert_eq!(shown, [("a".to_owned(), false), ("b".to_owned(), true), ("c".to_owned(), false)]);
    assert_eq!(list.item_count(), 3);

    sync_in_render(&probe, cx, rows(&["a", "c"]), None, AFTER_THE_MOTION);
    let shown = probe.read_with(cx, |probe, _| probe.shown.clone());
    assert_eq!(shown, [("a".to_owned(), false), ("c".to_owned(), false)]);
    assert_eq!(list.item_count(), 2);
}

#[gpui::test]
fn no_animation_lands_at_once(cx: &mut TestAppContext) {
    let theme = AppTheme::dark();
    let list = Rc::new(cx.update(|_| ListUi::new(&theme, 0, ListAlignment::Top)));
    let drawn = list.clone();
    let (probe, cx) = cx.add_window_view(move |_, _| Probe { list: drawn });
    let mut transitions = ListRowTransitions::<()>::new();

    cx.update(|window, cx| transitions.sync(&list, rows(&["a", "b"]), None, Duration::ZERO, window, cx));
    probe.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    let frame = cx.update(|window, cx| transitions.sync(&list, rows(&["b"]), None, Duration::ZERO, window, cx));
    assert_eq!(shown_ids(frame.rows()), ["b"]);
    assert_eq!(list.item_count(), 1);
    assert!(!transitions.is_animating());
}

fn numbered(ids: std::ops::Range<usize>) -> Vec<(String, ())> {
    ["header".to_owned()]
        .into_iter()
        .chain(ids.map(|id| format!("hit-{id}")))
        .chain(["footer".to_owned()])
        .map(|id| (id, ()))
        .collect()
}

#[gpui::test]
fn a_sliding_window_keeps_the_row_under_the_scroll_top(cx: &mut TestAppContext) {
    let theme = AppTheme::dark();
    let list = Rc::new(cx.update(|_| ListUi::new(&theme, 0, ListAlignment::Top)));
    let drawn = list.clone();
    let (_probe, cx) = cx.add_window_view(move |_, _| Probe { list: drawn });
    let mut transitions = ListRowTransitions::<()>::new();

    cx.update(|window, cx| transitions.sync(&list, numbered(0..20), None, Duration::ZERO, window, cx));
    list.state().scroll_to(ListOffset { item_ix: 16, offset_in_item: px(0.0) });

    let frame = cx.update(|window, cx| transitions.sync(&list, numbered(5..25), None, Duration::ZERO, window, cx));
    assert_eq!(list.item_count(), 22);
    let top = list.state().logical_scroll_top().item_ix;
    assert_eq!(frame.rows()[top].id, "hit-15");

    let frame = cx.update(|window, cx| transitions.sync(&list, numbered(0..20), None, Duration::ZERO, window, cx));
    assert_eq!(list.item_count(), 22);
    let top = list.state().logical_scroll_top().item_ix;
    assert_eq!(frame.rows()[top].id, "hit-15");
}
