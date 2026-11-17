use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::ProfilePostListWindowPolicy;

fn ids() -> Vec<String> {
    (0..20).map(|index| format!("p{index}")).collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn eligible_anchor_on_the_page_opens_the_window() {
    assert_eq!(ProfilePostListWindowPolicy::anchor_index(true, &ids(), "p7", false), Some(7));
}

#[test]
fn ineligible_or_missing_anchor_keeps_the_legacy_list() {
    assert_eq!(ProfilePostListWindowPolicy::anchor_index(false, &ids(), "p7", false), None);
    assert_eq!(ProfilePostListWindowPolicy::anchor_index(true, &ids(), "gone", false), None);
    assert_eq!(ProfilePostListWindowPolicy::anchor_index(true, &[], "p7", false), None);
}

#[test]
fn latched_window_never_flips_back() {
    assert_eq!(ProfilePostListWindowPolicy::anchor_index(true, &ids(), "gone", true), Some(0));
    assert_eq!(ProfilePostListWindowPolicy::anchor_index(false, &ids(), "p4", true), Some(4));
    assert_eq!(ProfilePostListWindowPolicy::anchor_index(true, &[], "p4", true), Some(0));
}

#[test]
fn lower_follows_the_revealed_id_across_shifts() {
    assert_eq!(ProfilePostListWindowPolicy::lower_index(&ids(), "p9", 3), 9);
    let mut prepended = strings(&["n0", "n1"]);
    prepended.extend(ids());
    assert_eq!(ProfilePostListWindowPolicy::lower_index(&prepended, "p9", 9), 11);
}

#[test]
fn missing_id_after_a_delete_uses_the_hint() {
    let after_delete: Vec<String> = ids().into_iter().filter(|id| id != "p9").collect();
    assert_eq!(ProfilePostListWindowPolicy::lower_index(&after_delete, "p9", 9), 9);
    assert_eq!(ProfilePostListWindowPolicy::lower_index(&after_delete, "p9", 40), after_delete.len() - 1);
    assert_eq!(ProfilePostListWindowPolicy::lower_index(&after_delete, "p9", -2), 0);
    assert_eq!(ProfilePostListWindowPolicy::lower_index(&[], "p9", 5), 0);
}

#[test]
fn chunk_reveal_clamps_at_the_first_row() {
    assert_eq!(ProfilePostListWindowPolicy::next_revealed_from_id(&ids(), 10, 6), Some("p4".to_string()));
    assert_eq!(ProfilePostListWindowPolicy::next_revealed_from_id(&ids(), 6, 6), Some("p0".to_string()));
    assert_eq!(ProfilePostListWindowPolicy::next_revealed_from_id(&ids(), 4, 6), Some("p0".to_string()));
    assert_eq!(ProfilePostListWindowPolicy::next_revealed_from_id(&ids(), 0, 6), None);
    assert_eq!(ProfilePostListWindowPolicy::next_revealed_from_id(&[], 4, 6), None);
}

#[test]
fn chunk_reveal_handles_out_of_range_inputs() {
    assert_eq!(ProfilePostListWindowPolicy::next_revealed_from_id(&ids(), 50, 6), Some("p13".to_string()));
    assert_eq!(ProfilePostListWindowPolicy::next_revealed_from_id(&ids(), 5, 0), Some("p4".to_string()));
}

#[test]
fn reveal_waits_for_settle_scroll_down_and_near_top() {
    let viewport: f32 = 700.0;
    let above = SharedElementMotion::REVEAL_ABOVE_VIEWPORTS;
    let reveal = |settled: bool, scrolled_down: bool, content_above: f32, lower: usize| {
        ProfilePostListWindowPolicy::should_reveal(settled, scrolled_down, content_above, viewport, lower, above)
    };

    assert!(reveal(true, true, 300.0, 6));
    assert!(!reveal(false, true, 300.0, 6));
    assert!(!reveal(true, false, 300.0, 6));
    assert!(!reveal(true, true, 300.0, 0));
    assert!(!reveal(true, true, viewport * above, 6));
    assert!(reveal(true, true, viewport * above - 1.0, 6));
}

#[test]
fn reveal_needs_a_laid_out_viewport() {
    assert!(!ProfilePostListWindowPolicy::should_reveal(true, true, 0.0, 0.0, 6, 2.0));
}

#[test]
fn refresh_is_allowed_only_at_the_first_row() {
    assert!(ProfilePostListWindowPolicy::refresh_allowed(0));
    assert!(!ProfilePostListWindowPolicy::refresh_allowed(1));
    assert!(!ProfilePostListWindowPolicy::refresh_allowed(6));
}
