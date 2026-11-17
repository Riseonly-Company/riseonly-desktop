use std::time::Duration;

use gpui::{ListAlignment, ListOffset, ListState, Pixels, px};

use crate::list_ui::list_scroll_animation::ListScrollAnimation;
use crate::swiftui::motion::ease_out;

fn at(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds)
}

fn row(item_ix: usize) -> ListOffset {
    ListOffset { item_ix, offset_in_item: Pixels::ZERO }
}

fn twenty_rows() -> ListState {
    ListState::new(20, ListAlignment::Top, px(100.0))
}

#[test]
fn reduce_motion_lands_at_once() {
    let list = twenty_rows();
    let mut animation = ListScrollAnimation::default();
    animation.start(&list, row(10), ease_out(0.22), at(1.0), true);
    assert_eq!(list.logical_scroll_top().item_ix, 10);
    assert!(!animation.is_running());
}

#[test]
fn start_keeps_the_list_where_it_is_until_the_first_frame() {
    let list = twenty_rows();
    list.scroll_to(row(3));
    let mut animation = ListScrollAnimation::default();
    animation.start(&list, row(10), ease_out(0.22), at(1.0), false);
    assert_eq!(list.logical_scroll_top().item_ix, 3);
    assert!(animation.is_running());
}

#[test]
fn the_motion_passes_between_the_ends_and_lands_on_the_exact_row() {
    let list = twenty_rows();
    let mut animation = ListScrollAnimation::default();
    animation.start(&list, row(10), ease_out(0.22), at(1.0), false);

    assert!(animation.tick(&list, at(1.11)));
    let middle = list.logical_scroll_top().item_ix;
    assert!(middle > 0 && middle < 10, "expected a row between the ends, got {middle}");

    assert!(!animation.tick(&list, at(1.3)));
    assert_eq!(list.logical_scroll_top().item_ix, 10);
    assert_eq!(list.logical_scroll_top().offset_in_item, Pixels::ZERO);
    assert!(!animation.is_running());
}

#[test]
fn the_motion_runs_upward_too() {
    let list = twenty_rows();
    list.scroll_to(row(12));
    let mut animation = ListScrollAnimation::default();
    animation.start(&list, row(2), ease_out(0.22), at(1.0), false);

    assert!(animation.tick(&list, at(1.11)));
    let middle = list.logical_scroll_top().item_ix;
    assert!(middle > 2 && middle < 12, "expected a row between the ends, got {middle}");

    assert!(!animation.tick(&list, at(1.3)));
    assert_eq!(list.logical_scroll_top().item_ix, 2);
}

#[test]
fn a_scroll_from_outside_ends_the_motion_where_it_is() {
    let list = twenty_rows();
    let mut animation = ListScrollAnimation::default();
    animation.start(&list, row(10), ease_out(0.22), at(1.0), false);
    assert!(animation.tick(&list, at(1.05)));

    list.scroll_to(row(17));
    assert!(!animation.tick(&list, at(1.11)));
    assert_eq!(list.logical_scroll_top().item_ix, 17);
    assert!(!animation.is_running());
}

#[test]
fn a_changed_row_count_drops_the_stale_target() {
    let list = twenty_rows();
    let mut animation = ListScrollAnimation::default();
    animation.start(&list, row(10), ease_out(0.22), at(1.0), false);

    list.splice(20..20, 5);
    assert!(!animation.tick(&list, at(1.3)));
    assert_eq!(list.logical_scroll_top().item_ix, 0);
}

#[test]
fn a_new_target_replaces_the_running_motion() {
    let list = twenty_rows();
    let mut animation = ListScrollAnimation::default();
    animation.start(&list, row(10), ease_out(0.22), at(1.0), false);
    assert!(animation.tick(&list, at(1.11)));

    animation.start(&list, row(15), ease_out(0.22), at(1.11), false);
    assert!(!animation.tick(&list, at(1.4)));
    assert_eq!(list.logical_scroll_top().item_ix, 15);
}
