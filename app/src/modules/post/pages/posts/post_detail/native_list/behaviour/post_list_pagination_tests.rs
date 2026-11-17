use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_pagination::{PostListPaginationPolicy, PostListPaginationPolicyThresholds};

const VIEWPORT: f32 = 800.0;

fn thresholds(heights: &[f32], unpresented: i64, velocity: f32) -> Option<PostListPaginationPolicyThresholds> {
    PostListPaginationPolicy::thresholds(VIEWPORT, velocity, heights, unpresented, 5.0)
}

#[test]
fn ten_tall_rows_fire_before_the_chat_edge_distance() {
    let result = thresholds(&[500.0; 12], 0, 0.0);
    assert_eq!(result.map(|value| value.load), Some(5_045.0));
    assert_eq!(result.map(|value| value.rearm), Some(5_645.0));
}

#[test]
fn short_rows_keep_the_chat_edge_distance() {
    let result = thresholds(&[40.0; 10], 0, 0.0);
    assert_eq!(result.map(|value| value.load), Some(1_600.0));
    assert_eq!(result.map(|value| value.rearm), Some(2_200.0));
}

#[test]
fn unpresented_rows_shorten_the_tail() {
    let result = thresholds(&[500.0; 10], 6, 0.0);
    assert_eq!(result.map(|value| value.load), Some(2_015.0));
    assert_eq!(thresholds(&[500.0; 10], 10, 0.0), None);
}

#[test]
fn velocity_toward_the_end_reaches_further() {
    let idle = thresholds(&[100.0], 0, 0.0);
    let flinging = thresholds(&[100.0], 0, 3_000.0);
    assert!(flinging.map_or(0.0, |value| value.load) > idle.map_or(0.0, |value| value.load));
    assert_eq!(thresholds(&[100.0], 0, -3_000.0).map(|value| value.load), idle.map(|value| value.load));
}

#[test]
fn request_needs_every_gate() {
    let gate = Some(PostListPaginationPolicyThresholds { load: 1_000.0, rearm: 2_000.0 });
    let request = PostListPaginationPolicy::should_request;

    assert!(request(900.0, gate, true, true, false, false, 1.0));
    assert!(!request(1_001.0, gate, true, true, false, false, 1.0));
    assert!(!request(900.0, None, true, true, false, false, 1.0));
    assert!(!request(900.0, gate, false, true, false, false, 1.0));
    assert!(!request(900.0, gate, true, false, false, false, 1.0));
    assert!(!request(900.0, gate, true, true, true, false, 1.0));
    assert!(!request(900.0, gate, true, true, false, true, 1.0));
    assert!(!request(900.0, gate, true, true, false, false, 0.2));
}

#[test]
fn rearm_needs_the_rearm_distance() {
    let gate = Some(PostListPaginationPolicyThresholds { load: 1_000.0, rearm: 2_000.0 });
    assert!(PostListPaginationPolicy::should_rearm(2_001.0, gate));
    assert!(!PostListPaginationPolicy::should_rearm(1_500.0, gate));
    assert!(PostListPaginationPolicy::should_rearm(0.0, None));
}

#[test]
fn suffix_lookahead_covers_the_load_distance_and_a_viewport() {
    assert_eq!(PostListPaginationPolicy::suffix_lookahead(VIEWPORT, 0.0), 2_400.0);
    assert!(PostListPaginationPolicy::needs_suffix_rows(2_400.0, VIEWPORT, 0.0));
    assert!(!PostListPaginationPolicy::needs_suffix_rows(2_401.0, VIEWPORT, 0.0));
}
