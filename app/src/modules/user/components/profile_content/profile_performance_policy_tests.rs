use std::collections::HashSet;
use std::path::Path;

use gpui::{AppContext as _, TestAppContext, point, px, size};
use rise_core::foundation::foundation_url::Url;
use rise_ui::animated_tabs_ui::animated_tabs_types::AnimatedTabsMetrics;

use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::{
    ProfileGridPreviewSourcePolicy, ProfileThumbnailDecodePolicy,
};
use crate::modules::user::components::profile_content::profile_content::ProfileContentTab;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::{
    PROFILE_DECELERATION_RATE_FAST, PROFILE_DECELERATION_RATE_NORMAL, ProfileContentLayoutPolicy,
    ProfileGeometryRefreshPolicy, ProfileHorizontalMotionView, ProfileHorizontalPagerPolicy,
    ProfileHorizontalPagerPolicySelectionDisposition, ProfileMotionFrameRatePolicy, ProfilePagerAxisLockPolicy,
    ProfilePagerAxisLockPolicyAxis, ProfileScrollCoordinator, ProfileScrollToTopOwnershipPolicy,
    ProfileScrollToTopOwnershipPolicyRole, ProfileScrollView, ProfileVerticalMasterScrollPolicy,
    ProfileVerticalMasterScrollPolicyPosition,
};

use ProfileContentTab as Tab;
use ProfileHorizontalPagerPolicy as Pager;
use ProfileHorizontalPagerPolicySelectionDisposition as Disposition;
use ProfilePagerAxisLockPolicyAxis as Axis;
use ProfileVerticalMasterScrollPolicy as Master;
use ProfileVerticalMasterScrollPolicyPosition as Position;

const POSTS: Tab = Tab::POSTS;

fn tabs_set<const N: usize>(tabs: [Tab; N]) -> HashSet<Tab> {
    HashSet::from(tabs)
}

fn directions<const N: usize>(values: [i64; N]) -> HashSet<i64> {
    HashSet::from(values)
}

fn position(collapse_progress: f32, inner_offset: f32) -> Position {
    Position { collapse_progress, inner_offset }
}

#[test]
fn profile_thumbnail_decode_size_tracks_rendered_pixels_and_caps_scale() {
    let tall = size(px(130.0), px(150.0));
    assert_eq!(ProfileThumbnailDecodePolicy::maximum_pixel_dimension(tall, 3.0), 450);
    assert_eq!(ProfileThumbnailDecodePolicy::maximum_pixel_dimension(size(px(40.0), px(40.0)), 3.0), 120);
    assert_eq!(ProfileThumbnailDecodePolicy::maximum_pixel_dimension(tall, 4.0), 450);
}

#[test]
fn profile_thumbnail_bucket_survives_a_column_reflow() {
    let bucket = ProfileThumbnailDecodePolicy::bucketed_pixel_dimension;
    for scale in [1.0, 2.0, 3.0] {
        let five = bucket(size(px(198.4), px(198.4)), scale);
        let four = bucket(size(px(224.5), px(224.5)), scale);
        assert_eq!(five, four);
        assert!(five >= ProfileThumbnailDecodePolicy::maximum_pixel_dimension(size(px(224.5), px(224.5)), scale));
    }
    assert_eq!(bucket(size(px(200.0), px(200.0)), 2.0), 512);
    assert_eq!(bucket(size(px(200.0), px(200.0)), 3.0), 768);
    assert_eq!(bucket(size(px(40.0), px(40.0)), 3.0), 128);
}

#[test]
fn profile_grid_preview_keeps_local_files_on_the_async_image_path() {
    let local = Url::from_file_path(Path::new("/tmp/profile preview.jpg")).map(|url| url.absolute_string().to_owned());
    let remote = "https://cdn.example.test/fallback.jpg";

    assert_eq!(
        ProfileGridPreviewSourcePolicy::source(local.as_deref(), Some(remote), None),
        local.as_deref().and_then(Url::from_string).map(|url| url.standardized().absolute_string().to_owned())
    );
    assert_eq!(
        ProfileGridPreviewSourcePolicy::source(None, Some("/tmp/profile-grid.jpg"), None),
        Url::from_file_path(Path::new("/tmp/profile-grid.jpg")).map(|url| url.absolute_string().to_owned())
    );
}

#[test]
fn profile_grid_preview_preserves_remote_and_thumbnail_fallback_order() {
    let primary = "https://cdn.example.test/primary.jpg";
    let thumbnail = "https://cdn.example.test/thumb.jpg";

    assert_eq!(
        ProfileGridPreviewSourcePolicy::source(None, Some(primary), Some(thumbnail)).as_deref(),
        Some(primary)
    );
    assert_eq!(
        ProfileGridPreviewSourcePolicy::source(None, Some("not-an-image-source"), Some(thumbnail)).as_deref(),
        Some(thumbnail)
    );
}

#[test]
fn outer_scroll_is_the_only_status_bar_scroll_to_top_owner() {
    let owners = ProfileScrollToTopOwnershipPolicyRole::ALL_CASES
        .iter()
        .filter(|role| ProfileScrollToTopOwnershipPolicy::is_owner(**role))
        .count();

    assert_eq!(owners, 1);
    assert!(ProfileScrollToTopOwnershipPolicy::is_owner(ProfileScrollToTopOwnershipPolicyRole::Outer));
    assert!(!ProfileScrollToTopOwnershipPolicy::is_owner(ProfileScrollToTopOwnershipPolicyRole::Inner));
}

#[gpui::test]
fn status_bar_route_resets_outer_and_active_inner_to_their_real_minimums(cx: &mut TestAppContext) {
    let coordinator = cx.new(ProfileScrollCoordinator::new);
    let outer = ProfileScrollView::with_test_geometry(390.0, 800.0, 390.0, 1_000.0);
    outer.set_scrolls_to_top(false);
    let inner = ProfileScrollView::with_test_geometry(390.0, 800.0, 390.0, 3_000.0);
    inner.set_scrolls_to_top(true);

    let outer_token = coordinator.update(cx, |coordinator, cx| {
        coordinator.bind_outer(
            &outer,
            std::rc::Rc::new(|_: i64, _: &mut gpui::App| {}),
            std::rc::Rc::new(|_: i64, _: &gpui::App| false),
            cx,
        )
    });
    let inner_token = coordinator.update(cx, |coordinator, cx| coordinator.register_inner_for(&inner, POSTS, cx));

    cx.update(|cx| {
        outer.set_content_offset(0.0, 200.0, cx);
        inner.set_content_offset(0.0, 900.0, cx);
    });

    assert!(outer.scrolls_to_top());
    assert!(!inner.scrolls_to_top());
    assert!(coordinator.update(cx, |coordinator, cx| coordinator.prepare_for_status_bar_scroll_to_top(&outer, false, cx)));

    cx.update(|cx| outer.set_content_offset(outer.content_offset_x(), 0.0, cx));
    coordinator.update(cx, |coordinator, cx| coordinator.complete_status_bar_scroll_to_top(&outer, cx));

    assert_eq!(outer.content_offset_y(), 0.0);
    assert_eq!(inner.content_offset_y(), 0.0);

    coordinator.update(cx, |coordinator, cx| {
        coordinator.unregister_inner_for(&inner, POSTS, inner_token, cx);
        coordinator.unbind_outer(&outer, outer_token, cx);
    });
    assert!(!outer.scrolls_to_top());
    assert!(inner.scrolls_to_top());
}

#[test]
fn pager_initial_mount_window_stays_bounded_to_active_page_and_neighbors() {
    let tabs = Tab::ALL_CASES;

    assert_eq!(ProfileContentLayoutPolicy::rendered_tabs(&tabs, POSTS), tabs_set([POSTS, Tab::Riffs]));
    assert_eq!(
        ProfileContentLayoutPolicy::rendered_tabs(&tabs, Tab::Reposts),
        tabs_set([Tab::Comments, Tab::Reposts, Tab::Mentions])
    );
    assert_eq!(ProfileContentLayoutPolicy::rendered_tabs(&tabs, Tab::Plans), tabs_set([Tab::Goals, Tab::Plans]));
    assert!(ProfileContentLayoutPolicy::rendered_tabs(&tabs, Tab::Reposts).len() <= 3);
    assert_eq!(Pager::PAGE_PRELOAD_DELAY, Pager::SETTLE_DURATION + Pager::POST_SETTLE_MOUNT_QUIET_DELAY);
}

#[test]
fn pager_initial_warmup_yields_only_the_first_main_queue_turn() {
    assert_eq!(Pager::INITIAL_PAGE_PRELOAD_DELAY, 0.0);
    assert!(Pager::INITIAL_PAGE_PRELOAD_DELAY < Pager::PAGE_PRELOAD_DELAY);
    assert!(Pager::PAGE_PRELOAD_BINDING_RETRY_DELAY > 0.0);
    assert!(Pager::PAGE_PRELOAD_BINDING_RETRY_DELAY <= 1.0 / 30.0);
}

#[test]
fn pager_warmup_mounts_the_selected_window_and_transitions_only_grow() {
    let tabs = Tab::ALL_CASES;
    let mut mounted = tabs_set([POSTS]);

    mounted = ProfileContentLayoutPolicy::warmed_tabs(&tabs, POSTS, &mounted);
    assert_eq!(mounted, tabs_set([POSTS, Tab::Riffs]));

    let after_first_warmup = mounted.clone();
    let during_transition = ProfileContentLayoutPolicy::grown_tabs_during_transition(&tabs, Tab::Riffs, &mounted);
    assert!(during_transition.is_superset(&after_first_warmup));
    assert_eq!(during_transition, tabs_set([POSTS, Tab::Riffs, Tab::Threads]));

    mounted = ProfileContentLayoutPolicy::warmed_tabs(&tabs, Tab::Riffs, &during_transition);
    assert_eq!(mounted, tabs_set([POSTS, Tab::Riffs, Tab::Threads]));

    mounted = ProfileContentLayoutPolicy::warmed_tabs(&tabs, Tab::Plans, &mounted);
    assert_eq!(mounted, tabs_set([Tab::Goals, Tab::Plans]));
    assert!(mounted.len() <= 3);

    let reduced_scope = &tabs[..3];
    assert_eq!(
        ProfileContentLayoutPolicy::retained_tabs_during_transition(reduced_scope, Tab::Riffs, &tabs.into_iter().collect()),
        reduced_scope.iter().copied().collect::<HashSet<_>>()
    );
}

#[test]
fn pager_cannot_swipe_into_an_unmounted_page() {
    let tabs = Tab::ALL_CASES;
    let during_commit = tabs_set([POSTS, Tab::Riffs]);

    assert_eq!(
        ProfileContentLayoutPolicy::prepared_horizontal_directions(&tabs, Tab::Riffs, &during_commit),
        directions([-1])
    );
    assert_eq!(
        ProfileContentLayoutPolicy::prepared_horizontal_directions(
            &tabs,
            Tab::Riffs,
            &tabs_set([POSTS, Tab::Riffs, Tab::Threads])
        ),
        directions([-1, 1])
    );

    assert!(!Pager::can_commit(1, &directions([-1])));
    assert!(Pager::can_commit(-1, &directions([-1])));
}

#[test]
fn pager_release_commits_by_distance_or_velocity() {
    assert!(!Pager::should_commit(Pager::DISTANCE_COMMIT_THRESHOLD - 0.01, 0.0));
    assert!(Pager::should_commit(Pager::DISTANCE_COMMIT_THRESHOLD, 0.0));
    assert!(Pager::should_commit(0.03, -Pager::VELOCITY_COMMIT_THRESHOLD));
}

#[test]
fn pager_commits_toward_the_half_revealed_page_not_the_flick() {
    assert_eq!(Pager::direction(0.02, -900.0, 390.0), 1);
    assert_eq!(Pager::direction(-0.02, 900.0, 390.0), -1);
    assert_eq!(Pager::direction(0.5, 2400.0, 390.0), 1);
    assert_eq!(Pager::direction(-0.5, -2400.0, 390.0), -1);
    assert_eq!(Pager::direction(0.0, -1200.0, 390.0), 1);
}

#[test]
fn pager_shares_animated_tabs_release_contract() {
    assert_eq!(Pager::DISTANCE_COMMIT_THRESHOLD, AnimatedTabsMetrics::PAGE_COMMIT_THRESHOLD);
    assert_eq!(Pager::VELOCITY_COMMIT_THRESHOLD, AnimatedTabsMetrics::PAGE_VELOCITY_COMMIT_THRESHOLD);
    assert_eq!(Pager::SETTLE_DURATION, AnimatedTabsMetrics::RELEASE_DURATION);
}

#[test]
fn page_commit_continuity_does_not_jump_at_finger_release() {
    let old_index = 1.0_f32;
    let progress = 0.37_f32;
    let direction = 1;
    let new_index = old_index + direction as f32;
    let continuity = Pager::continuity_progress(progress, direction);

    assert!((old_index + progress - (new_index + continuity)).abs() < 1e-5);
    assert!(continuity < 0.0);
}

#[test]
fn interrupted_pager_starts_at_the_current_presentation_progress_without_a_jump() {
    let presentation_progress = -0.42;

    assert_eq!(Pager::interactive_progress(presentation_progress, 0.0, 390.0), presentation_progress);
}

#[test]
fn interrupted_pager_can_reverse_direction_around_its_presentation_position() {
    let presentation_progress = -0.42;
    let continued_toward_rest = Pager::interactive_progress(presentation_progress, -78.0, 390.0);
    let reversed_toward_previous_page = Pager::interactive_progress(presentation_progress, 39.0, 390.0);

    assert!((continued_toward_rest - (-0.22)).abs() < 0.0001);
    assert!((reversed_toward_previous_page - (-0.52)).abs() < 0.0001);
    assert!(Pager::moves_toward_rest(presentation_progress, 1));
    assert!(!Pager::moves_toward_rest(presentation_progress, -1));
}

#[test]
fn interrupted_pager_never_reveals_an_unmounted_far_page() {
    let presentation_progress = -0.42;
    let none = HashSet::new();

    assert_eq!(
        Pager::constrained_interactive_progress(-0.70, presentation_progress, &none, &none),
        presentation_progress
    );
    assert_eq!(Pager::constrained_interactive_progress(-0.20, presentation_progress, &none, &none), -0.20);
    assert_eq!(Pager::constrained_interactive_progress(0.35, presentation_progress, &none, &none), 0.0);
    assert_eq!(Pager::constrained_interactive_progress(0.35, presentation_progress, &directions([1]), &none), 0.35);
}

#[test]
fn pager_resists_past_the_last_tab_instead_of_standing_still() {
    let none = HashSet::new();
    assert_eq!(
        Pager::constrained_interactive_progress(0.4, 0.0, &none, &directions([1])),
        0.4 * AnimatedTabsMetrics::EDGE_RESISTANCE
    );
    assert_eq!(
        Pager::constrained_interactive_progress(1.0, 0.0, &none, &directions([1])),
        AnimatedTabsMetrics::EDGE_RESISTANCE.min(AnimatedTabsMetrics::EDGE_MAXIMUM_OVERSHOOT)
    );
    assert_eq!(Pager::constrained_interactive_progress(0.4, 0.0, &none, &none), 0.0);
}

#[test]
fn stale_settlement_converges_monotonically_without_crossing_the_target() {
    let start = -0.64_f32;
    let samples: Vec<f32> = [0.0, 0.1, 0.2, 0.4, 0.7, 0.9, 1.0]
        .into_iter()
        .map(|fraction| Pager::settlement_progress(start, fraction))
        .collect();

    assert_eq!(samples.first().copied(), Some(start));
    assert_eq!(samples.last().copied().map(f32::abs), Some(0.0));
    assert!(samples.iter().all(|sample| *sample <= 0.0 && *sample >= start));
    assert!(samples.windows(2).all(|pair| pair[1].abs() <= pair[0].abs()));
}

#[test]
fn absolute_pager_settlement_is_continuous_and_monotonic_across_committed_index() {
    let start = 1.37_f32;
    let target = 2.0_f32;
    let samples: Vec<f32> = [0.0, 0.1, 0.25, 0.5, 0.8, 1.0]
        .into_iter()
        .map(|fraction| Pager::settlement_position(start, target, fraction))
        .collect();

    assert_eq!(samples.first().copied(), Some(start));
    assert_eq!(samples.last().copied(), Some(target));
    assert!(samples.iter().all(|sample| *sample >= start && *sample <= target));
    assert!(samples.windows(2).all(|pair| pair[1] >= pair[0]));
}

#[test]
fn pager_width_change_preserves_fractional_absolute_index() {
    let old_width = 390.0;
    let new_width = 430.0;
    let fractional_index = 1.37_f32;
    let old_offset = Pager::offset_x(fractional_index, -12.0, old_width, 5);
    let recovered_index = Pager::absolute_index(old_offset, -12.0, old_width, 5);
    let resized_offset = Pager::offset_x(recovered_index, -20.0, new_width, 5);

    assert!((recovered_index - fractional_index).abs() < 0.0001);
    assert!((resized_offset - (-20.0 + fractional_index * new_width)).abs() < 0.01);
}

#[test]
fn pager_motion_layer_round_trips_visual_position_without_moving_resting_offset() {
    let resting_index = 2.0_f32;
    let visual_index = 2.43_f32;
    let page_width = 393.0;
    let translation = Pager::motion_translation_x(visual_index, resting_index, page_width);
    let recovered = Pager::visual_absolute_index(resting_index, translation, page_width, 5);

    assert!((translation - (-visual_index + resting_index) * page_width).abs() < 0.0001);
    assert!((recovered - visual_index).abs() < 0.0001);
}

#[test]
fn pager_motion_layer_reads_back_the_edge_overshoot_it_was_asked_to_draw() {
    let page_count = 4;
    let page_width = 393.0;
    let resting_index = (page_count - 1) as f32;
    let visual_index = resting_index + AnimatedTabsMetrics::EDGE_MAXIMUM_OVERSHOOT;
    let translation = Pager::motion_translation_x(visual_index, resting_index, page_width);
    let recovered = Pager::visual_absolute_index(resting_index, translation, page_width, page_count);

    assert!((recovered - visual_index).abs() < 0.0001);
    assert!(recovered > resting_index);
    assert_eq!(
        Pager::visual_absolute_index(resting_index, -5.0 * page_width, page_width, page_count),
        resting_index + AnimatedTabsMetrics::EDGE_MAXIMUM_OVERSHOOT
    );
}

#[test]
fn pager_selection_echo_defers_but_external_replacement_cancels_settle() {
    assert_eq!(
        Pager::selection_disposition(POSTS, Some(Tab::Comments), Tab::Comments, Some(1), false, true, Some(1)),
        Disposition::DeferUntilSettle
    );
    assert_eq!(
        Pager::selection_disposition(POSTS, Some(Tab::Comments), POSTS, Some(0), false, true, Some(1)),
        Disposition::ReplaceImmediately
    );
    assert_eq!(
        Pager::selection_disposition(POSTS, Some(Tab::Comments), Tab::Reposts, Some(2), false, true, Some(1)),
        Disposition::ReplaceImmediately
    );
    assert_eq!(
        Pager::selection_disposition(POSTS, None, POSTS, Some(0), false, false, None),
        Disposition::Unchanged
    );
}

#[test]
fn pager_axis_lock_rejects_vertical_intent_before_native_pan_hysteresis() {
    assert_eq!(ProfilePagerAxisLockPolicy::early_axis(point(0.8, 2.0)), Axis::Vertical);
    assert_eq!(ProfilePagerAxisLockPolicy::early_axis(point(2.0, 0.8)), Axis::Horizontal);
    assert_eq!(ProfilePagerAxisLockPolicy::early_axis(point(4.0, 4.5)), Axis::Undecided);
    assert_eq!(ProfilePagerAxisLockPolicy::early_axis(point(9.0, 10.0)), Axis::Vertical);
    assert_eq!(ProfilePagerAxisLockPolicy::early_axis(point(10.0, 10.0)), Axis::Vertical);
    assert_eq!(ProfilePagerAxisLockPolicy::axis(point(3.0, 0.5), point(-40.0, 900.0)), Axis::Horizontal);
}

#[test]
fn interrupted_pager_rejects_the_cancelled_settlement_completion() {
    assert!(!Pager::accepts_settlement_callback(7, 8));
    assert!(Pager::accepts_settlement_callback(8, 8));
}

#[test]
fn geometry_refresh_is_deferred_for_every_profile_motion_phase() {
    assert!(ProfileGeometryRefreshPolicy::should_defer(true, false, false));
    assert!(ProfileGeometryRefreshPolicy::should_defer(false, true, false));
    assert!(ProfileGeometryRefreshPolicy::should_defer(false, false, true));
    assert!(!ProfileGeometryRefreshPolicy::should_defer(false, false, false));
    assert!(!ProfileGeometryRefreshPolicy::accepts_flush(11, 12));
    assert!(ProfileGeometryRefreshPolicy::accepts_flush(12, 12));
    assert!(ProfileGeometryRefreshPolicy::STABLE_IDLE_DELAY >= 0.10);
    let epsilon = ProfileGeometryRefreshPolicy::CONTENT_SIZE_EPSILON;
    assert!(!ProfileGeometryRefreshPolicy::content_size_changed(size(390.0, 800.0), size(390.2, 800.3), epsilon));
    assert!(ProfileGeometryRefreshPolicy::content_size_changed(size(390.0, 800.0), size(390.0, 801.0), epsilon));
}

#[test]
fn motion_cadence_runs_at_the_display_maximum() {
    assert_eq!(ProfileMotionFrameRatePolicy::frame_rate_range(120).preferred, 120.0);
    assert_eq!(ProfileMotionFrameRatePolicy::frame_rate_range(60).preferred, 60.0);
    assert_eq!(ProfileMotionFrameRatePolicy::frame_rate_range(120).minimum, 30.0);
}

#[test]
fn native_master_scroll_maps_one_continuous_range_across_header_and_feed() {
    assert!(Master::uses_native_master_engine(&["Riseonly".to_owned()]));
    assert!(!Master::uses_native_master_engine(&[
        "Riseonly".to_owned(),
        Master::LEGACY_LAUNCH_ARGUMENT.to_owned()
    ]));

    let collapse_range = Master::collapse_range(1_000.0, 700.0);
    assert_eq!(collapse_range, 300.0);

    let collapsing = Master::mapping(150.0, collapse_range, 700.0);
    assert_eq!(collapsing.collapse_progress, 0.5);
    assert_eq!(collapsing.inner_offset, 0.0);
    assert_eq!(collapsing.pin_correction, 0.0);

    let feeding = Master::mapping(450.0, collapse_range, 700.0);
    assert_eq!(feeding.collapse_progress, 1.0);
    assert_eq!(feeding.inner_offset, 150.0);
    assert_eq!(feeding.pin_correction, 150.0);

    let bottom = Master::mapping(1_200.0, collapse_range, 700.0);
    assert_eq!(bottom.inner_offset, 700.0);
}

#[test]
fn native_master_scroll_restores_semantic_position_across_geometry_changes() {
    assert!((Master::master_offset(position(0.4, 0.0), 300.0, 900.0) - 120.0).abs() < 1e-3);
    assert_eq!(Master::master_offset(position(0.4, 200.0), 300.0, 900.0), 500.0);
    assert_eq!(Master::master_offset(position(1.0, 1_200.0), 300.0, 900.0), 1_200.0);
}

#[test]
fn horizontal_tab_transfer_preserves_header_and_restores_each_feed_depth() {
    assert_eq!(
        Master::restored_tab_position(position(0.4, 0.0), Some(position(1.0, 480.0)), 900.0),
        position(0.4, 0.0)
    );
    assert_eq!(
        Master::restored_tab_position(position(1.0, 480.0), Some(position(1.0, 120.0)), 220.0),
        position(1.0, 120.0)
    );
    assert_eq!(
        Master::restored_tab_position(position(1.0, 480.0), Some(position(1.0, 420.0)), 220.0),
        position(1.0, 220.0)
    );
    assert_eq!(Master::restored_tab_position(position(1.0, 480.0), None, 0.0), position(1.0, 0.0));
}

#[gpui::test]
fn native_master_scroll_restores_outer_ui_kit_configuration_on_detach(cx: &mut TestAppContext) {
    let coordinator = cx.new(ProfileScrollCoordinator::new);
    let outer = ProfileScrollView::with_test_geometry(390.0, 800.0, 390.0, 1_100.0);
    outer.set_bounces(true);
    outer.set_deceleration_rate(PROFILE_DECELERATION_RATE_FAST);
    outer.set_is_directional_lock_enabled(false);

    let token = coordinator.update(cx, |coordinator, cx| {
        coordinator.bind_outer(
            &outer,
            std::rc::Rc::new(|_: i64, _: &mut gpui::App| {}),
            std::rc::Rc::new(|_: i64, _: &gpui::App| false),
            cx,
        )
    });

    assert!(outer.pan_enabled());
    assert!(outer.bounces());
    assert_eq!(outer.deceleration_rate(), PROFILE_DECELERATION_RATE_NORMAL);
    assert!(outer.is_directional_lock_enabled());

    coordinator.update(cx, |coordinator, cx| coordinator.unbind_outer(&outer, token, cx));
    assert!(outer.pan_enabled());
    assert!(outer.bounces());
    assert_eq!(outer.deceleration_rate(), PROFILE_DECELERATION_RATE_FAST);
    assert!(!outer.is_directional_lock_enabled());
}

#[gpui::test]
fn horizontal_pager_restores_ui_kit_configuration_on_detach(cx: &mut TestAppContext) {
    let coordinator = cx.new(ProfileScrollCoordinator::new);
    let pager = ProfileScrollView::with_test_geometry(390.0, 800.0, 390.0 * 5.0, 800.0);
    let motion_view = ProfileHorizontalMotionView::new();
    pager.set_pan_enabled(true);
    pager.set_bounces(true);
    pager.set_scrolls_to_top(true);
    pager.set_is_directional_lock_enabled(false);

    let token = coordinator
        .update(cx, |coordinator, cx| coordinator.bind_horizontal_pager(&pager, &motion_view, 2, 5, 390.0, cx));

    assert!(!pager.pan_enabled());
    assert!(!pager.bounces());
    assert!(!pager.scrolls_to_top());
    assert!(pager.is_directional_lock_enabled());
    assert!((pager.content_offset_x() - 780.0).abs() < 0.1);

    coordinator.update(cx, |coordinator, cx| coordinator.set_interaction_locked(true, cx));
    assert!(!pager.is_user_interaction_enabled());
    coordinator.update(cx, |coordinator, cx| coordinator.set_interaction_locked(false, cx));
    assert!(pager.is_user_interaction_enabled());

    coordinator.update(cx, |coordinator, cx| coordinator.unbind_horizontal_pager(&pager, token, cx));
    assert!(pager.pan_enabled());
    assert!(pager.bounces());
    assert!(pager.scrolls_to_top());
    assert!(!pager.is_directional_lock_enabled());
    assert!(pager.is_user_interaction_enabled());
}

#[test]
fn one_commit_can_advance_exactly_one_page() {
    assert_eq!(Pager::target_index(1, 1, 5), Some(2));
    assert_eq!(Pager::target_index(3, -1, 5), Some(2));
    assert_eq!(Pager::target_index(1, 2, 5), None);
    assert_eq!(Pager::target_index(0, -1, 5), None);
}

#[test]
fn trace_transition_uses_the_actual_visible_tab_order() {
    let foreign_tabs = [POSTS, Tab::Comments, Tab::Reposts, Tab::Plans];
    let transition = Pager::tab_transition(&Tab::Reposts, 1, &foreign_tabs).expect("a tab follows");

    assert_eq!(transition.source_index, 2);
    assert_eq!(transition.target_index, 3);
}
