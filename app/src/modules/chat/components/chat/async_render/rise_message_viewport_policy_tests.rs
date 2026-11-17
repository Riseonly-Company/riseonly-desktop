use std::cell::{Cell, RefCell};

use gpui::{AppContext, TestAppContext};
use rise_core::foundation::foundation_date::TimeInterval;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::chat::components::chat::async_render::rise_message_list_view::{
    RiseMessageActiveScrollEdgeCommitPolicy, RiseMessageActiveScrollFallbackMeasurementPolicy, RiseMessageHistoryEdgePolicy,
    RiseMessagePendingPresentationCommit, RiseMessagePendingPresentationCommitPolicy, RiseMessageViewportPolicy,
};
use crate::modules::chat::pages::chats::chat::chat_screen::chat_screen_support::{ChatJumpToLatestPresentationPolicy, ChatMessageViewportChromeState};
use crate::modules::chat::stores::message::message_actions::message_types::MessageHistoryPaginationDispatch;

#[test]
fn latest_tail_is_at_latest_without_jump_button() {
    let state = RiseMessageViewportPolicy::resolve(0.0, false, None);

    assert!(state.is_at_latest);
    assert!(!state.shows_jump_to_latest);
}

#[test]
fn distant_viewport_shows_jump_button_with_hysteresis() {
    let distant = RiseMessageViewportPolicy::resolve(RiseMessageViewportPolicy::SHOW_THRESHOLD + 1.0, false, None);
    let returning =
        RiseMessageViewportPolicy::resolve(RiseMessageViewportPolicy::LATEST_THRESHOLD + 1.0, false, Some(distant));
    let latest = RiseMessageViewportPolicy::resolve(RiseMessageViewportPolicy::LATEST_THRESHOLD, false, Some(returning));

    assert!(distant.shows_jump_to_latest);
    assert!(returning.shows_jump_to_latest);
    assert!(latest.is_at_latest);
    assert!(!latest.shows_jump_to_latest);
}

#[test]
fn missing_newer_cache_always_offers_latest_jump() {
    let state = RiseMessageViewportPolicy::resolve(0.0, true, None);

    assert!(!state.is_at_latest);
    assert!(state.shows_jump_to_latest);
}

#[test]
fn jump_button_keeps_a_stable_slot_and_only_changes_visibility() {
    assert!(ChatJumpToLatestPresentationPolicy::KEEPS_LAYOUT_SLOT_MOUNTED);
    assert!(ChatJumpToLatestPresentationPolicy::is_visible(true, false, false));
    assert!(!ChatJumpToLatestPresentationPolicy::is_visible(false, false, false));
    assert!(!ChatJumpToLatestPresentationPolicy::is_visible(true, true, false));
    assert!(!ChatJumpToLatestPresentationPolicy::is_visible(true, false, true));
}

#[gpui::test]
fn jump_button_viewport_state_lives_in_its_own_reference_model(cx: &mut TestAppContext) {
    let chrome = cx.new(|_| ChatMessageViewportChromeState::default());
    let read = |cx: &TestAppContext| chrome.read_with(cx, |chrome, _| (chrome.is_at_latest(), chrome.shows_jump_to_bottom()));

    assert_eq!(read(cx), (false, false));

    chrome.update(cx, |chrome, cx| chrome.update(false, true, cx));
    assert_eq!(read(cx), (false, true));

    chrome.update(cx, |chrome, cx| chrome.finish_latest_jump(cx));
    assert_eq!(read(cx), (true, false));

    chrome.update(cx, |chrome, cx| chrome.reset(cx));
    assert_eq!(read(cx), (false, false));
}

#[test]
fn history_edge_must_leave_load_zone_before_rearming() {
    let thresholds = RiseMessageHistoryEdgePolicy::thresholds(852.0, 12_000.0);

    assert!(thresholds.rearm > thresholds.load);
    assert!(thresholds.rearm - thresholds.load >= 600.0);
}

#[test]
fn active_velocity_starts_network_prefetch_before_scroll_settle() {
    let idle = RiseMessageHistoryEdgePolicy::thresholds(852.0, 0.0);
    let decelerating = RiseMessageHistoryEdgePolicy::thresholds(852.0, 8_000.0);
    let distance_inside_only_active_lookahead = (idle.load + decelerating.load) / 2.0;

    assert!(decelerating.load > idle.load);
    assert!(!RiseMessageHistoryEdgePolicy::is_inside_load_zone(
        distance_inside_only_active_lookahead,
        idle.load,
        false,
        None,
        true
    ));
    assert!(RiseMessageHistoryEdgePolicy::is_inside_load_zone(
        distance_inside_only_active_lookahead,
        decelerating.load,
        false,
        None,
        true
    ));
}

#[test]
fn cached_row_lead_skips_the_edge_behind_the_motion() {
    assert!(!RiseMessageHistoryEdgePolicy::is_inside_load_zone(20_418.0, 2_622.0, true, Some(34), false));
    assert!(RiseMessageHistoryEdgePolicy::is_inside_load_zone(2_000.0, 2_622.0, true, Some(34), false));
    assert!(!RiseMessageHistoryEdgePolicy::heads_toward_edge(false, 0.0, Some(true)));
    assert!(RiseMessageHistoryEdgePolicy::heads_toward_edge(true, 0.0, Some(true)));
    assert!(RiseMessageHistoryEdgePolicy::heads_toward_edge(false, 120.0, Some(true)));
    assert!(!RiseMessageHistoryEdgePolicy::heads_toward_edge(true, 0.0, None));
}

#[test]
fn cached_window_prefetches_well_before_its_last_eight_rows() {
    let threshold = RiseMessageHistoryEdgePolicy::CACHED_ENTRY_LOAD_THRESHOLD;

    assert!(threshold > 8);
    assert!(RiseMessageHistoryEdgePolicy::is_inside_load_zone(20_000.0, 1_700.0, true, Some(threshold), true));
    assert!(!RiseMessageHistoryEdgePolicy::is_inside_load_zone(20_000.0, 1_700.0, true, Some(threshold + 1), true));
}

#[test]
fn cached_edge_hysteresis_requires_leaving_both_load_zones() {
    let threshold = RiseMessageHistoryEdgePolicy::cached_entry_rearm_threshold();

    assert!(!RiseMessageHistoryEdgePolicy::is_outside_rearm_zone(5_000.0, 3_000.0, true, Some(threshold)));
    assert!(!RiseMessageHistoryEdgePolicy::is_outside_rearm_zone(2_999.0, 3_000.0, true, Some(threshold + 1)));
    assert!(RiseMessageHistoryEdgePolicy::is_outside_rearm_zone(5_000.0, 3_000.0, true, Some(threshold + 1)));
}

#[test]
fn committed_history_progress_rearms_without_scrolling_away_from_the_edge() {
    assert!(RiseMessageHistoryEdgePolicy::should_rearm_after_history_progress(true, true));
    assert!(!RiseMessageHistoryEdgePolicy::should_rearm_after_history_progress(false, true));
    assert!(!RiseMessageHistoryEdgePolicy::should_rearm_after_history_progress(true, false));
}

#[test]
fn availability_progress_completes_pending_request_without_visible_mutation() {
    assert!(RiseMessageHistoryEdgePolicy::did_complete_pending_request(false, true));
    assert!(RiseMessageHistoryEdgePolicy::did_complete_pending_request(true, false));
    assert!(!RiseMessageHistoryEdgePolicy::did_complete_pending_request(false, false));
}

#[test]
fn stale_commit_cannot_complete_a_newer_pending_request() {
    assert!(RiseMessageHistoryEdgePolicy::commit_belongs_to_pending_request(7, Some(7)));
    assert!(!RiseMessageHistoryEdgePolicy::commit_belongs_to_pending_request(8, Some(7)));
    assert!(!RiseMessageHistoryEdgePolicy::commit_belongs_to_pending_request(7, None));
}

#[test]
fn cached_to_network_boundary_counts_as_history_availability_change() {
    assert!(RiseMessageHistoryEdgePolicy::did_availability_change(true, true, false, true));
    assert!(RiseMessageHistoryEdgePolicy::did_availability_change(true, true, true, false));
    assert!(!RiseMessageHistoryEdgePolicy::did_availability_change(true, true, true, true));
    assert!(RiseMessageHistoryEdgePolicy::did_availability_change(true, false, false, false));
}

#[test]
fn history_prefetch_deduplicates_and_applies_backpressure() {
    let ready_elapsed = RiseMessageHistoryEdgePolicy::MINIMUM_REQUEST_INTERVAL;

    assert!(RiseMessageHistoryEdgePolicy::can_request(true, false, true, true, ready_elapsed));
    assert!(!RiseMessageHistoryEdgePolicy::can_request(true, true, true, true, 100.0));
    assert!(!RiseMessageHistoryEdgePolicy::can_request(true, false, true, true, ready_elapsed - 0.01));
    assert!(RiseMessageHistoryEdgePolicy::pending_watchdog_delay(true) > RiseMessageHistoryEdgePolicy::pending_watchdog_delay(false));
}

#[test]
fn prefetch_requires_motion_interaction_or_settled_underfill() {
    let evaluate = RiseMessageHistoryEdgePolicy::can_evaluate_prefetch;

    assert!(evaluate(true, true, false, false, false, false, true, true, true, false));
    assert!(evaluate(true, true, false, false, false, false, true, false, false, true));
    assert!(!evaluate(true, true, true, false, false, false, false, false, false, false));
    assert!(evaluate(true, true, true, false, true, false, false, false, false, false));
    assert!(evaluate(true, true, true, true, false, false, true, false, false, false));
    assert!(!evaluate(false, true, true, true, false, false, false, true, true, false));
    assert!(!evaluate(true, false, true, true, false, false, false, true, true, false));
    assert!(!evaluate(true, true, false, false, true, false, false, false, false, false));
    assert!(!evaluate(true, true, true, true, false, true, false, true, true, false));
}

fn pending_commit(account: &str, epoch: Uuid, source_version: i64) -> RiseMessagePendingPresentationCommit {
    RiseMessagePendingPresentationCommit {
        account_user_id: Some(format!("account-{account}")),
        account_epoch: Some(format!("epoch-{account}")),
        chat_id: "chat-a".to_owned(),
        presentation_epoch: Some(epoch),
        source_version,
    }
}

#[test]
fn pending_presentation_clears_only_for_its_committed_identity() {
    let epoch = Uuid::new_v4();
    let pending = pending_commit("a", epoch, 41);
    let replacement = pending_commit("a", epoch, 42);
    let unrelated = pending_commit("b", epoch, 42);

    assert!(RiseMessagePendingPresentationCommitPolicy::should_track_incoming(Some(&pending), &replacement, false, false));
    assert!(!RiseMessagePendingPresentationCommitPolicy::should_track_incoming(Some(&pending), &unrelated, false, false));
    assert!(!RiseMessagePendingPresentationCommitPolicy::should_clear_after_commit(Some(&replacement), &replacement, false));
    assert!(!RiseMessagePendingPresentationCommitPolicy::should_clear_after_commit(Some(&replacement), &pending, true));
    assert!(!RiseMessagePendingPresentationCommitPolicy::should_clear_after_commit(Some(&replacement), &unrelated, true));
    assert!(RiseMessagePendingPresentationCommitPolicy::should_clear_after_commit(Some(&replacement), &replacement, true));
}

#[test]
fn active_scroll_fallback_lane_selects_only_critical_unresolved_rows() {
    let selected = RiseMessageActiveScrollFallbackMeasurementPolicy::unresolved_critical_indices(&[7, 2, 7, 9], &[1, 2, 7, 8]);

    assert_eq!(selected, vec![7, 2]);
}

#[test]
fn active_scroll_fallback_lane_spaces_hosted_measurements_by_a_frame() {
    let interval: TimeInterval = RiseMessageActiveScrollFallbackMeasurementPolicy::MINIMUM_ACTOR_INTERVAL;
    let partial_elapsed = interval / 4.0;
    let remaining_delay = RiseMessageActiveScrollFallbackMeasurementPolicy::delay_before_next_measurement(Some(partial_elapsed), interval);

    assert!(interval >= 1.0 / 60.0);
    assert_eq!(RiseMessageActiveScrollFallbackMeasurementPolicy::delay_before_next_measurement(None, interval), interval);
    assert!((remaining_delay - interval * 0.75).abs() < 0.000_001);
    assert_eq!(RiseMessageActiveScrollFallbackMeasurementPolicy::delay_before_next_measurement(Some(interval * 2.0), interval), 0.0);
}

#[test]
fn stale_pre_callback_cancellation_clears_only_its_edge_cooldown() {
    let timestamp: TimeInterval = 42.0;

    assert_eq!(RiseMessageHistoryEdgePolicy::request_timestamp_after_cancellation(timestamp, false), 0.0);
    assert_eq!(RiseMessageHistoryEdgePolicy::request_timestamp_after_cancellation(timestamp, true), timestamp);
    assert!(RiseMessageHistoryEdgePolicy::should_reevaluate_after_cancellation(false, false));
    assert!(!RiseMessageHistoryEdgePolicy::should_reevaluate_after_cancellation(true, false));
    assert!(RiseMessageHistoryEdgePolicy::should_reevaluate_after_cancellation(true, true));
}

#[test]
fn current_history_request_is_delivered_in_the_scroll_callback_turn() {
    let events = RefCell::new(vec!["before"]);

    let delivered = RiseMessageHistoryEdgePolicy::deliver_request_if_current(true, || events.borrow_mut().push("callback"));
    events.borrow_mut().push("after");

    assert!(delivered);
    assert_eq!(*events.borrow(), vec!["before", "callback", "after"]);
}

#[test]
fn cached_window_shift_does_not_delay_remote_history_prefetch() {
    let events = RefCell::new(Vec::new());

    let did_shift = MessageHistoryPaginationDispatch::perform(
        || {
            events.borrow_mut().push("cache");
            true
        },
        || events.borrow_mut().push("remote"),
    );

    assert!(did_shift);
    assert_eq!(*events.borrow(), vec!["cache", "remote"]);
}

#[test]
fn missing_cached_window_still_requests_remote_history() {
    let request_count = Cell::new(0);

    let did_shift = MessageHistoryPaginationDispatch::perform(|| false, || request_count.set(request_count.get() + 1));

    assert!(!did_shift);
    assert_eq!(request_count.get(), 1);
}

#[test]
fn stale_history_request_is_not_delivered() {
    let callback_count = Cell::new(0);

    let delivered = RiseMessageHistoryEdgePolicy::deliver_request_if_current(false, || callback_count.set(callback_count.get() + 1));

    assert!(!delivered);
    assert_eq!(callback_count.get(), 0);
}

#[test]
fn exact_anchored_edge_mutation_can_apply_during_drag_and_deceleration() {
    assert!(RiseMessageActiveScrollEdgeCommitPolicy::allows_commit(true, true, false, true, true, true));
    assert!(RiseMessageActiveScrollEdgeCommitPolicy::allows_commit(false, false, true, true, true, true));
    assert!(!RiseMessageActiveScrollEdgeCommitPolicy::allows_commit(false, true, false, true, true, false));
    assert!(!RiseMessageActiveScrollEdgeCommitPolicy::allows_commit(false, false, true, true, false, true));
}
