use gpui::{Bounds, Pixels, point, px, size};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementPart, SharedElementVisibilityPolicy, SharedElementVisibilityPolicyCandidate,
};

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

fn band() -> Bounds<Pixels> {
    rect(0.0, 100.0, 390.0, 680.0)
}

fn candidate(
    id: &str,
    media: Option<Bounds<Pixels>>,
    text: Option<Bounds<Pixels>>,
) -> SharedElementVisibilityPolicyCandidate {
    SharedElementVisibilityPolicyCandidate { item_id: id.to_string(), media_frame: media, text_frame: text }
}

fn picked_id(candidates: &[SharedElementVisibilityPolicyCandidate]) -> Option<String> {
    SharedElementVisibilityPolicy::pick_current(candidates, band()).map(|(item_id, _)| item_id)
}

#[test]
fn band_runs_from_the_floating_inset_to_the_nearest_bottom_edge() {
    let viewport = rect(0.0, 0.0, 390.0, 844.0);
    let with_chrome = SharedElementVisibilityPolicy::band(viewport, 100.0, Some(780.0), 34.0);
    assert_eq!(with_chrome, rect(0.0, 100.0, 390.0, 680.0));

    let without_chrome = SharedElementVisibilityPolicy::band(viewport, 100.0, None, 34.0);
    assert_eq!(without_chrome, rect(0.0, 100.0, 390.0, 710.0));

    let collapsed = SharedElementVisibilityPolicy::band(viewport, 900.0, None, 0.0);
    assert_eq!(f32::from(collapsed.size.height), 0.0);
}

#[test]
fn media_post_beats_a_larger_text_block_of_another_post() {
    let media = candidate("media", Some(rect(0.0, 110.0, 390.0, 300.0)), None);
    let text = candidate("text", None, Some(rect(16.0, 530.0, 358.0, 900.0)));
    let result = SharedElementVisibilityPolicy::pick_current(&[text, media], band());
    assert_eq!(result.as_ref().map(|(item_id, _)| item_id.as_str()), Some("media"));
    assert_eq!(result.map(|(_, part)| part), Some(SharedElementPart::Media));
}

#[test]
fn text_counts_only_without_media() {
    let clipped_media = candidate(
        "clipped",
        Some(rect(0.0, -200.0, 390.0, 400.0)),
        Some(rect(16.0, 220.0, 358.0, 500.0)),
    );
    let visible_media = candidate("visible", Some(rect(0.0, 620.0, 390.0, 300.0)), None);
    assert_eq!(picked_id(&[clipped_media, visible_media]).as_deref(), Some("visible"));

    let hidden_media = candidate(
        "hidden",
        Some(rect(0.0, -600.0, 390.0, 400.0)),
        Some(rect(16.0, 200.0, 358.0, 400.0)),
    );
    assert_eq!(SharedElementVisibilityPolicy::pick_current(&[hidden_media], band()), None);

    let text_only = candidate("thread", None, Some(rect(16.0, 200.0, 358.0, 120.0)));
    let text_result = SharedElementVisibilityPolicy::pick_current(&[text_only], band());
    assert_eq!(text_result.as_ref().map(|(item_id, _)| item_id.as_str()), Some("thread"));
    assert_eq!(text_result.map(|(_, part)| part), Some(SharedElementPart::Text));
}

#[test]
fn tie_goes_to_the_frame_closest_to_the_band_centre() {
    let near = candidate("near", Some(rect(0.0, 360.0, 390.0, 200.0)), None);
    let far = candidate("far", Some(rect(0.0, 100.0, 390.0, 200.0)), None);
    assert_eq!(picked_id(&[far.clone(), near.clone()]).as_deref(), Some("near"));
    assert_eq!(picked_id(&[near.clone(), far]).as_deref(), Some("near"));

    let almost_equal_far = candidate("far", Some(rect(0.0, 100.0, 390.0, 200.002)), None);
    assert_eq!(picked_id(&[almost_equal_far, near]).as_deref(), Some("near"));
}

#[test]
fn clearly_larger_score_wins_over_centre_distance() {
    let centred_small = candidate("small", Some(rect(0.0, 400.0, 390.0, 80.0)), None);
    let off_centre_large = candidate("large", Some(rect(0.0, 100.0, 390.0, 250.0)), None);
    assert_eq!(picked_id(&[centred_small, off_centre_large]).as_deref(), Some("large"));
}

#[test]
fn zero_score_returns_nil() {
    assert_eq!(SharedElementVisibilityPolicy::pick_current(&[], band()), None);
    assert_eq!(SharedElementVisibilityPolicy::pick_current(&[candidate("empty", None, None)], band()), None);

    let above = candidate("above", Some(rect(0.0, -300.0, 390.0, 400.0)), None);
    let below = candidate("below", None, Some(rect(0.0, 780.0, 390.0, 200.0)));
    assert_eq!(SharedElementVisibilityPolicy::pick_current(&[above, below], band()), None);
}

#[test]
fn page_with_the_largest_visible_area_is_current() {
    let clip = rect(0.0, 120.0, 390.0, 488.0);
    let pages = [
        (0, rect(-490.0, 120.0, 390.0, 488.0)),
        (1, rect(-100.0, 120.0, 390.0, 488.0)),
        (2, rect(290.0, 120.0, 390.0, 488.0)),
    ];
    assert_eq!(SharedElementVisibilityPolicy::pick_page(&pages, clip), Some(1));
}

#[test]
fn page_tie_goes_to_the_closest_centre_and_no_overlap_is_nil() {
    let clip = rect(0.0, 0.0, 390.0, 400.0);
    let equal_areas = [(5, rect(195.0, 0.0, 390.0, 400.0)), (6, rect(-100.0, 0.0, 295.0, 400.0))];
    assert_eq!(SharedElementVisibilityPolicy::pick_page(&equal_areas, clip), Some(6));

    let outside = [(0, rect(390.0, 0.0, 390.0, 400.0)), (1, rect(-390.0, 0.0, 390.0, 400.0))];
    assert_eq!(SharedElementVisibilityPolicy::pick_page(&outside, clip), None);
    assert_eq!(SharedElementVisibilityPolicy::pick_page(&[], clip), None);
}
