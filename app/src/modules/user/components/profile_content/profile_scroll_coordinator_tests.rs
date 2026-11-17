use crate::modules::user::components::profile_content::profile_content::ProfileViewportLayoutPolicy;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::{
    ProfileMasterPinningPolicy, ProfileScrollMetrics,
};
use crate::modules::user::components::profile_top::profile_highlights::ProfileHighlightsLayoutPolicy;
use crate::modules::user::components::profile_top::profile_top::ProfileTopFadeTargetPolicy;

#[test]
fn tab_bar_starts_after_profile_top_and_pins_without_negative_geometry() {
    let profile_top_height = 486.0;
    let initial_top = ProfileViewportLayoutPolicy::initial_tabs_top(profile_top_height);
    let scroll_range = ProfileViewportLayoutPolicy::outer_scroll_range(profile_top_height);

    assert_eq!(initial_top - profile_top_height, ProfileViewportLayoutPolicy::OUTER_SPACING);
    assert_eq!(initial_top - scroll_range, ProfileScrollMetrics::PINNED_TABS_TOP);
    assert_eq!(ProfileViewportLayoutPolicy::pinned_tabs_top(profile_top_height), ProfileScrollMetrics::PINNED_TABS_TOP);
}

#[test]
fn profile_feed_viewport_underlaps_the_floating_header_without_changing_scroll_range() {
    let viewport_height = 844.0;
    let container_height = ProfileViewportLayoutPolicy::container_height(viewport_height);
    let pager_height = ProfileViewportLayoutPolicy::pager_height(container_height);

    assert_eq!(container_height, viewport_height - ProfileScrollMetrics::PINNED_TABS_TOP);
    assert_eq!(pager_height, viewport_height);
    assert_eq!(
        ProfileViewportLayoutPolicy::page_content_top_inset() - ProfileViewportLayoutPolicy::HEADER_UNDERLAP,
        ProfileScrollMetrics::TABS_BAR_HEIGHT
    );
    assert_eq!(
        pager_height - ProfileViewportLayoutPolicy::page_content_top_inset(),
        container_height - ProfileScrollMetrics::TABS_BAR_HEIGHT
    );
}

#[test]
fn highlights_have_stable_cross_axis_height() {
    assert_eq!(
        ProfileHighlightsLayoutPolicy::ROW_HEIGHT,
        ProfileHighlightsLayoutPolicy::CIRCLE_SIZE
            + ProfileHighlightsLayoutPolicy::ITEM_SPACING
            + ProfileHighlightsLayoutPolicy::LABEL_HEIGHT
            + ProfileHighlightsLayoutPolicy::VERTICAL_PADDING
    );
    assert_eq!(ProfileHighlightsLayoutPolicy::ROW_HEIGHT, 92.0);
}

#[test]
fn highlights_fade_to_black_before_tabs_become_pinned() {
    let fully_hidden_max_y = ProfileScrollMetrics::PINNED_TABS_TOP - ProfileViewportLayoutPolicy::OUTER_SPACING;

    assert_eq!(
        ProfileHighlightsLayoutPolicy::scroll_opacity_at_profile_metrics(fully_hidden_max_y + ProfileHighlightsLayoutPolicy::ROW_HEIGHT),
        1.0
    );
    assert_eq!(
        ProfileHighlightsLayoutPolicy::scroll_opacity_at_profile_metrics(
            fully_hidden_max_y + ProfileHighlightsLayoutPolicy::ROW_HEIGHT / 2.0
        ),
        0.5
    );
    assert_eq!(ProfileHighlightsLayoutPolicy::scroll_opacity_at_profile_metrics(fully_hidden_max_y), 0.0);
    assert_eq!(ProfileHighlightsLayoutPolicy::scroll_opacity_at_profile_metrics(fully_hidden_max_y - 40.0), 0.0);
}

#[test]
fn foreign_profile_without_highlights_fades_its_last_visible_content() {
    assert!(ProfileTopFadeTargetPolicy::uses_tail_fallback(true, false));
    assert!(!ProfileTopFadeTargetPolicy::uses_tail_fallback(true, true));
    assert!(!ProfileTopFadeTargetPolicy::uses_tail_fallback(false, false));
}

#[test]
fn native_master_pins_content_only_across_the_proxy_feed_range() {
    let top = ProfileScrollMetrics::PINNED_TABS_TOP;
    assert_eq!(ProfileMasterPinningPolicy::correction(top, 600.0, true, top), 0.0);
    assert_eq!(ProfileMasterPinningPolicy::correction(0.0, 600.0, true, top), top);
    assert_eq!(ProfileMasterPinningPolicy::correction(-500.0, 300.0, true, top), 300.0);
    assert_eq!(ProfileMasterPinningPolicy::correction(-500.0, 300.0, false, top), 0.0);
}
