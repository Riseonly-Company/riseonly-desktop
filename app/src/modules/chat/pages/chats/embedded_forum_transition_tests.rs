use std::cell::Cell;
use std::rc::Rc;

use gpui::{AppContext, TestAppContext, point};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, DateComponents, TimeZone};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::language_store::Locale;

use crate::modules::chat::components::chat_item::chat_item::ChatListTimestampFormatter;
use crate::modules::chat::components::chat_item::chat_list_row_activity::ChatListRowActivity;
use crate::modules::chat::pages::chats::chats::ChatListAvatarPrefetchPolicy;
use crate::modules::chat::pages::chats::embedded_forum_horizontal_scroll_gate::EmbeddedForumHorizontalPanIntentPolicy;
use crate::modules::chat::pages::chats::embedded_forum_transition::{
    EmbeddedForumSwipeAxisLock, EmbeddedForumSwipeAxisLockDecision, EmbeddedForumTransitionMetrics,
    EmbeddedForumTransitionState,
};

const WIDTH: f32 = 390.0;

#[gpui::test]
fn covered_list_activity_only_invalidates_subscribers_in_that_list(cx: &mut TestAppContext) {
    let inbox = cx.new(|_| ChatListRowActivity::new());
    let other_page = cx.new(|_| ChatListRowActivity::new());
    let inbox_changes = Rc::new(Cell::new(0));
    let other_page_changes = Rc::new(Cell::new(0));
    let _subscriptions = cx.update(|cx| {
        let (inbox_counter, other_counter) = (inbox_changes.clone(), other_page_changes.clone());
        [
            cx.observe(&inbox, move |_, _| inbox_counter.set(inbox_counter.get() + 1)),
            cx.observe(&other_page, move |_, _| other_counter.set(other_counter.get() + 1)),
        ]
    });

    inbox.update(cx, |activity, cx| activity.set_active(true, cx));
    cx.run_until_parked();
    assert_eq!(inbox_changes.get(), 0);
    inbox.update(cx, |activity, cx| activity.set_active(false, cx));
    cx.run_until_parked();
    assert!(!inbox.read_with(cx, |activity, _| activity.is_active()));
    assert_eq!(inbox_changes.get(), 1);
    assert_eq!(other_page_changes.get(), 0);
    assert!(other_page.read_with(cx, |activity, _| activity.is_active()));

    inbox.update(cx, |activity, cx| activity.set_active(false, cx));
    cx.run_until_parked();
    assert_eq!(inbox_changes.get(), 1);
    inbox.update(cx, |activity, cx| activity.set_active(true, cx));
    cx.run_until_parked();
    assert!(inbox.read_with(cx, |activity, _| activity.is_active()));
    assert_eq!(inbox_changes.get(), 2);
}

#[test]
fn avatar_prefetch_window_stays_bounded_and_directional() {
    let initial = ChatListAvatarPrefetchPolicy::candidate_indices(100, 20, None);
    let newer = ChatListAvatarPrefetchPolicy::candidate_indices(100, 20, Some(18));
    let older = ChatListAvatarPrefetchPolicy::candidate_indices(100, 20, Some(22));

    assert_eq!(initial, (18..=30).collect::<Vec<_>>());
    assert_eq!(newer, (20..=30).collect::<Vec<_>>());
    assert_eq!(older, (10..=20).collect::<Vec<_>>());
    let lookahead = ChatListAvatarPrefetchPolicy::lookahead();
    assert!(initial.len() as i64 <= lookahead + 3);
    assert!(newer.len() as i64 <= lookahead + 1);
    assert!(older.len() as i64 <= lookahead + 1);
    assert_eq!(ChatListAvatarPrefetchPolicy::candidate_indices(3, 0, None), [0, 1, 2]);
}

#[test]
fn timestamp_formatter_preserves_patterns_across_environment_changes() {
    let mut calendar = Calendar::gregorian();
    calendar.time_zone = TimeZone::gmt();

    let mut components = DateComponents::ymd(2020, 4, 5);
    components.hour = Some(12);
    let old_date = calendar.date_from_components(&components).expect("a fixed date resolves");
    let english = ChatListTimestampFormatter::string(
        old_date.time_interval_since_1970(),
        &Locale::new("en_US"),
        &calendar,
    );
    let russian = ChatListTimestampFormatter::string(
        old_date.time_interval_since_1970(),
        &Locale::new("ru_RU"),
        &calendar,
    );
    let english_again = ChatListTimestampFormatter::string(
        old_date.time_interval_since_1970(),
        &Locale::new("en_US"),
        &calendar,
    );

    assert!(english.contains("2020"));
    assert!(russian.contains("2020"));
    assert_ne!(english, russian);
    assert_eq!(english_again, english);

    let now = Date::now();
    let expected_time = format!(
        "{:02}:{:02}",
        calendar.component(CalendarComponent::Hour, &now),
        calendar.component(CalendarComponent::Minute, &now)
    );
    assert_eq!(
        ChatListTimestampFormatter::string(
            now.time_interval_since_1970(),
            &Locale::new("en_US"),
            &calendar
        ),
        expected_time
    );
}

#[test]
fn embedded_forum_transition_preserves_avatar_rail_and_clamps_drag() {
    let travel = EmbeddedForumTransitionMetrics::travel_distance(WIDTH);

    assert_eq!(travel, 320.0);
    assert_eq!(EmbeddedForumTransitionMetrics::panel_offset(false, 0.0, WIDTH), travel);
    assert_eq!(EmbeddedForumTransitionMetrics::progress(true, travel / 2.0, WIDTH), 0.5);
    assert_eq!(EmbeddedForumTransitionMetrics::panel_offset(true, -40.0, WIDTH), 0.0);
    assert_eq!(EmbeddedForumTransitionMetrics::panel_offset(true, travel * 2.0, WIDTH), travel);
}

#[test]
fn embedded_forum_swipe_back_uses_distance_and_velocity_thresholds() {
    assert!(EmbeddedForumTransitionMetrics::SWIPE_MINIMUM_DISTANCE > 0.0);
    assert!(!EmbeddedForumTransitionMetrics::should_close(60.0, 100.0, WIDTH));
    assert!(EmbeddedForumTransitionMetrics::should_close(80.0, 80.0, WIDTH));
    assert!(EmbeddedForumTransitionMetrics::should_close(35.0, 150.0, WIDTH));
}

#[test]
fn embedded_forum_swipe_axis_waits_for_directional_intent() {
    let mut lock = EmbeddedForumSwipeAxisLock::new(8.0, 1.2);

    assert_eq!(lock.update(7.0, 1.0), EmbeddedForumSwipeAxisLockDecision::Undecided);
    assert!(!lock.accepts_close_drag());
    assert!(!lock.suppresses_vertical_scroll());

    assert_eq!(lock.update(10.0, 5.0), EmbeddedForumSwipeAxisLockDecision::Horizontal);
    assert!(lock.accepts_close_drag());
    assert!(lock.suppresses_vertical_scroll());
    assert_eq!(lock.horizontal_activation_offset(), 8.0);
    assert_eq!(lock.interactive_translation(10.0), 2.0);
}

#[test]
fn embedded_forum_horizontal_axis_remains_locked_until_reset() {
    let mut lock = EmbeddedForumSwipeAxisLock::new(8.0, 1.2);

    assert_eq!(lock.update(12.0, 2.0), EmbeddedForumSwipeAxisLockDecision::Horizontal);
    assert_eq!(lock.update(13.0, 80.0), EmbeddedForumSwipeAxisLockDecision::Horizontal);
    assert!(lock.suppresses_vertical_scroll());

    lock.reset();
    assert_eq!(lock.decision(), EmbeddedForumSwipeAxisLockDecision::Undecided);
    assert!(!lock.accepts_close_drag());
    assert!(!lock.suppresses_vertical_scroll());
    assert_eq!(lock.horizontal_activation_offset(), 0.0);
    assert_eq!(lock.interactive_translation(40.0), 0.0);
}

#[test]
fn embedded_forum_slow_horizontal_pan_uses_stable_translation_intent() {
    assert!(EmbeddedForumHorizontalPanIntentPolicy::accepts(point(9.0, 2.0), point(0.1, 8.0)));
    assert!(!EmbeddedForumHorizontalPanIntentPolicy::accepts(point(2.0, 9.0), point(80.0, 1.0)));
    assert!(EmbeddedForumHorizontalPanIntentPolicy::accepts(point(0.0, 0.0), point(80.0, 2.0)));
}

#[test]
fn embedded_forum_pan_prediction_uses_compensated_visual_distance() {
    let activation = EmbeddedForumHorizontalPanIntentPolicy::activation_offset(12.0);
    assert_eq!(activation, 8.0);
    assert_eq!(EmbeddedForumHorizontalPanIntentPolicy::compensated_translation(12.0, activation), 4.0);
    assert_eq!(EmbeddedForumHorizontalPanIntentPolicy::predicted_translation(12.0, 100.0, activation), 22.0);
}

#[test]
fn embedded_forum_header_progress_sampling_is_bounded_and_stable() {
    assert_eq!(EmbeddedForumTransitionMetrics::sampled_header_progress(-1.0), 0.0);
    assert_eq!(EmbeddedForumTransitionMetrics::sampled_header_progress(2.0), 1.0);
    assert_eq!(
        EmbeddedForumTransitionMetrics::sampled_header_progress(0.5001),
        EmbeddedForumTransitionMetrics::sampled_header_progress(0.5002)
    );
}

#[test]
fn embedded_forum_animation_settles_before_layer_cleanup() {
    assert!(EmbeddedForumTransitionMetrics::ANIMATION_DURATION < 0.48);
}

#[test]
fn embedded_forum_vertical_and_leftward_gestures_never_activate_close() {
    let mut vertical = EmbeddedForumSwipeAxisLock::new(8.0, 1.2);
    assert_eq!(vertical.update(4.0, 10.0), EmbeddedForumSwipeAxisLockDecision::Rejected);
    assert_eq!(vertical.update(30.0, 10.0), EmbeddedForumSwipeAxisLockDecision::Rejected);
    assert!(!vertical.accepts_close_drag());
    assert!(!vertical.suppresses_vertical_scroll());

    let mut leftward = EmbeddedForumSwipeAxisLock::new(8.0, 1.2);
    assert_eq!(leftward.update(-12.0, 1.0), EmbeddedForumSwipeAxisLockDecision::Rejected);
    assert_eq!(leftward.update(30.0, 1.0), EmbeddedForumSwipeAxisLockDecision::Rejected);
    assert!(!leftward.accepts_close_drag());
    assert!(!leftward.suppresses_vertical_scroll());
}

#[gpui::test]
fn embedded_forum_interactive_state_clamps_and_resets(cx: &mut TestAppContext) {
    let transition = cx.new(|_| EmbeddedForumTransitionState::new());

    transition.update(cx, |transition, cx| {
        transition.prepare_for_presentation(cx);
        assert_eq!(transition.progress(WIDTH), 0.0);

        transition.set_presented(true, cx);
        transition.update_drag_offset(160.0, WIDTH, cx);
        assert_eq!(transition.drag_offset(), 160.0);
        assert_eq!(transition.progress(WIDTH), 0.5);

        transition.update_drag_offset(1_000.0, WIDTH, cx);
        assert_eq!(transition.drag_offset(), 320.0);
        assert_eq!(transition.panel_offset(WIDTH), 320.0);

        transition.dismiss_immediately(cx);
        assert!(!transition.is_presented());
        assert_eq!(transition.drag_offset(), 0.0);
    });
}
