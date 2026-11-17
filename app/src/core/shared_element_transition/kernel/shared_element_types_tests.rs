use crate::core::shared_element_transition::kernel::shared_element_types::ProfilePostListWindowPolicy;

const VIEWPORT: f32 = 800.0;
const SPACING: f32 = 5.0;

fn count(heights: &[f32], content_above: f32, viewport: f32, max_rows: usize) -> usize {
    ProfilePostListWindowPolicy::reveal_count(heights, SPACING, content_above, viewport, 3.0, max_rows)
}

fn count_default(heights: &[f32]) -> usize {
    count(heights, 0.0, VIEWPORT, ProfilePostListWindowPolicy::MAXIMUM_REVEAL_ROWS_PER_COMMIT)
}

fn count_above(heights: &[f32], content_above: f32) -> usize {
    count(heights, content_above, VIEWPORT, ProfilePostListWindowPolicy::MAXIMUM_REVEAL_ROWS_PER_COMMIT)
}

#[test]
fn reveal_stops_once_three_viewports_are_covered() {
    assert_eq!(count_default(&[595.0; 8]), 4);
    assert_eq!(count_above(&[595.0; 8], 1_200.0), 2);
}

#[test]
fn reveal_inserts_at_least_one_row_while_the_gate_is_open() {
    assert_eq!(count_default(&[3_000.0]), 1);
    assert_eq!(count_above(&[100.0, 100.0], 2_399.0), 1);
}

#[test]
fn reveal_is_capped_by_max_rows_and_prepared_rows() {
    assert_eq!(count_default(&[50.0; 20]), 8);
    assert_eq!(count(&[50.0; 20], 0.0, VIEWPORT, 3), 3);
    assert_eq!(count_default(&[50.0, 50.0]), 2);
}

#[test]
fn repeated_rows_count_without_height_or_spacing() {
    assert_eq!(count_above(&[1_195.0, 0.0, 1_195.0], 0.0), 3);
}

#[test]
fn reveal_needs_prepared_rows_and_a_viewport() {
    assert_eq!(count_default(&[]), 0);
    assert_eq!(count(&[500.0], 0.0, 0.0, ProfilePostListWindowPolicy::MAXIMUM_REVEAL_ROWS_PER_COMMIT), 0);
    assert_eq!(count(&[500.0], 0.0, VIEWPORT, 0), 0);
}

#[test]
fn scrolled_down_latches_only_under_a_gesture() {
    assert!(ProfilePostListWindowPolicy::latches_scrolled_down(2.0, true, false));
    assert!(ProfilePostListWindowPolicy::latches_scrolled_down(2.0, false, true));
    assert!(!ProfilePostListWindowPolicy::latches_scrolled_down(2.0, false, false));
    assert!(!ProfilePostListWindowPolicy::latches_scrolled_down(1.0, true, false));
}

#[test]
fn pull_down_dismiss_waits_for_the_window_top_after_scrolling_down() {
    assert!(ProfilePostListWindowPolicy::allows_pull_down_dismiss(0, true));
    assert!(ProfilePostListWindowPolicy::allows_pull_down_dismiss(4, false));
    assert!(!ProfilePostListWindowPolicy::allows_pull_down_dismiss(4, true));
}
