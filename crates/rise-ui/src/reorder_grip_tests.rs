use std::time::Duration;

use crate::reorder_grip::ReorderMotion;

const STRIDE: f32 = 50.0;

fn order() -> Vec<&'static str> {
    vec!["a", "b", "c", "d"]
}

#[test]
fn the_dragged_item_follows_the_pointer_until_it_passes_half_a_neighbour() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    assert!(!motion.drag(&"a", 20.0, &order(), |_| STRIDE, now, true));
    assert_eq!(motion.shown_order(&order()), order());
    assert_eq!(motion.offset_at(&"a", now), 20.0);
}

#[test]
fn passing_neighbours_moves_the_item_and_keeps_it_under_the_pointer() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    assert!(motion.drag(&"a", 120.0, &order(), |_| STRIDE, now, true));
    assert_eq!(motion.shown_order(&order()), vec!["b", "c", "a", "d"]);
    assert_eq!(motion.offset_at(&"a", now), 20.0);
    assert_eq!(motion.offset_at(&"b", now), 0.0);
}

#[test]
fn a_displaced_neighbour_starts_where_it_stood_and_springs_home() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    motion.drag(&"a", 30.0, &order(), |_| STRIDE, now, false);
    assert_eq!(motion.shown_order(&order()), vec!["b", "a", "c", "d"]);
    assert_eq!(motion.offset_at(&"b", now), STRIDE);
    let later = Duration::from_secs(3);
    assert!(motion.offset_at(&"b", later).abs() < 0.5);
}

#[test]
fn moving_back_up_restores_the_order() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    motion.drag(&"c", -60.0, &order(), |_| STRIDE, now, true);
    assert_eq!(motion.shown_order(&order()), vec!["a", "c", "b", "d"]);
    motion.drag(&"c", 0.0, &order(), |_| STRIDE, now, true);
    assert_eq!(motion.shown_order(&order()), order());
}

#[test]
fn a_drop_hands_back_the_order_and_glides_home() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    motion.drag(&"a", 70.0, &order(), |_| STRIDE, now, false);
    assert_eq!(motion.end(now, false), Some(vec!["b", "a", "c", "d"]));
    assert!(motion.dragging().is_none());
    assert_eq!(motion.shown_order(&order()), order());
    assert_eq!(motion.offset_at(&"a", now), 20.0);
    assert!(motion.offset_at(&"a", Duration::from_secs(3)).abs() < 0.5);
    assert!(motion.lift_at(&"a", Duration::from_secs(3)) < 0.01);
    assert_eq!(motion.end(now, false), None);
}

#[test]
fn reduce_motion_lands_everything_at_once() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    motion.drag(&"a", 70.0, &order(), |_| STRIDE, now, true);
    assert_eq!(motion.offset_at(&"b", now), 0.0);
    motion.end(now, true);
    assert_eq!(motion.offset_at(&"a", now), 0.0);
    assert!(!motion.is_running(now));
}

#[test]
fn an_item_outside_the_list_never_starts_a_drag() {
    let mut motion = ReorderMotion::new();
    assert!(!motion.drag(&"z", 90.0, &order(), |_| STRIDE, Duration::ZERO, true));
    assert!(motion.dragging().is_none());
}

#[test]
fn a_list_that_keeps_its_order_draws_every_item_where_the_motion_puts_it() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    let base = order();
    motion.drag(&"a", 70.0, &base, |_| STRIDE, now, true);
    assert_eq!(motion.offset_from_base_at(&"a", &base, |_| STRIDE, now), 70.0);
    assert_eq!(motion.offset_from_base_at(&"b", &base, |_| STRIDE, now), -STRIDE);
    assert_eq!(motion.offset_from_base_at(&"c", &base, |_| STRIDE, now), 0.0);
    motion.end(now, true);
    assert_eq!(motion.offset_from_base_at(&"a", &base, |_| STRIDE, now), STRIDE);
    assert_eq!(motion.offset_from_base_at(&"b", &base, |_| STRIDE, now), -STRIDE);
    motion.settle();
    let landed = vec!["b", "a", "c", "d"];
    assert_eq!(motion.offset_from_base_at(&"a", &landed, |_| STRIDE, now), 0.0);
}

#[test]
fn a_displaced_neighbour_glides_out_of_the_way_in_a_kept_order() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    let base = order();
    motion.drag(&"a", 30.0, &base, |_| STRIDE, now, false);
    assert!(motion.offset_from_base_at(&"b", &base, |_| STRIDE, now).abs() < 0.5);
    let later = Duration::from_secs(3);
    assert!((motion.offset_from_base_at(&"b", &base, |_| STRIDE, later) + STRIDE).abs() < 0.5);
}

fn rail_slot(index: usize) -> f32 {
    [0.0, 50.0, 100.0, 400.0][index]
}

#[test]
fn a_drag_across_a_gap_swaps_only_past_half_the_gap() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    assert!(!motion.drag_slots(&"c", 140.0, &order(), rail_slot, now, true));
    assert!(motion.drag_slots(&"c", 160.0, &order(), rail_slot, now, true));
    assert_eq!(motion.offset_from_slots_at(&"c", &order(), rail_slot, now), 160.0);
    assert_eq!(motion.offset_from_slots_at(&"d", &order(), rail_slot, now), -300.0);
}

#[test]
fn a_neighbour_pushed_across_the_gap_starts_where_it_stood() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    motion.drag_slots(&"c", 160.0, &order(), rail_slot, now, false);
    assert_eq!(motion.offset_from_slots_at(&"d", &order(), rail_slot, now), 0.0);
    let later = Duration::from_secs(3);
    assert!((motion.offset_from_slots_at(&"d", &order(), rail_slot, later) + 300.0).abs() < 0.5);
}

#[test]
fn a_landed_order_draws_in_place_once_the_list_lays_it_out() {
    let mut motion = ReorderMotion::new();
    let now = Duration::ZERO;
    motion.drag_slots(&"c", 300.0, &order(), rail_slot, now, true);
    let landed = motion.end(now, true).unwrap_or_default();
    assert_eq!(landed, vec!["a", "b", "d", "c"]);
    motion.settle();
    let later = Duration::from_secs(3);
    for key in &landed {
        assert!(motion.offset_from_slots_at(key, &landed, rail_slot, later).abs() < 0.5);
    }
}
