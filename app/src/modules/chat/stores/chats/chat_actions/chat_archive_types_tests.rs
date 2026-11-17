use std::collections::HashSet;

use serde_json::{Map, Value, json};

use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::modules::chat::components::chat_item::chat_item::ChatListMenuPolicy;
use crate::modules::chat::pages::chats::chats::{ChatsArchiveScrollResetPolicy, ChatsScreenState};
use crate::modules::chat::pages::chats::chats_archive_reveal::{
    ChatsArchiveRevealPhase, ChatsArchiveRevealPolicy, ChatsArchiveSettleTarget,
};
use crate::modules::chat::pages::chats::chats_archive_reveal_row::ChatsArchiveSummaryRow;
use crate::modules::chat::stores::chats::chat_actions::chat_archive_types::{
    ChatArchiveCursorPolicy, ChatArchiveMutationFailurePolicy, ChatArchiveOptimisticRowPolicy,
    ChatArchiveRealtimeResolution,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChatDetail;
use rise_chat::ChatType;
use rise_i18n::tr;

fn removed(ids: &[&str]) -> HashSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn remaining(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn test_archive_optimistic_destination_clears_pin_without_mutating_snapshot() {
    let mut source = Map::new();
    source.insert("id".to_owned(), json!("chat-1"));
    source.insert("is_pinned".to_owned(), json!(true));
    source.insert("pin_position".to_owned(), json!(2));

    let destination = ChatArchiveOptimisticRowPolicy::destination_row(&source, true);

    assert_eq!(source.get("is_pinned"), Some(&json!(true)));
    assert_eq!(source.get("pin_position"), Some(&json!(2)));
    assert_eq!(destination.get("is_pinned"), Some(&json!(false)));
    assert_eq!(destination.get("pin_position"), Some(&Value::Null));
}

#[test]
fn test_same_direction_event_before_acknowledgement_keeps_pending_mutation() {
    let resolution = ChatArchiveRealtimeResolution::resolve(Some(true), true);

    assert_eq!(resolution, ChatArchiveRealtimeResolution::ConfirmsPendingMutation);
    assert!(!resolution.discards_pending_mutation());
}

#[test]
fn test_conflicting_realtime_event_discards_pending_mutation() {
    let resolution = ChatArchiveRealtimeResolution::resolve(Some(true), false);

    assert_eq!(resolution, ChatArchiveRealtimeResolution::ConflictsWithPendingMutation);
    assert!(resolution.discards_pending_mutation());
}

#[test]
fn test_realtime_confirmation_prevents_timeout_rollback() {
    assert!(!ChatArchiveMutationFailurePolicy::should_revert(true));
    assert!(ChatArchiveMutationFailurePolicy::should_revert(false));
}

#[test]
fn test_get_chat_by_id_decodes_archived_membership() {
    let response: Option<RiseStoredChatDetail> = SaiJSONCodec::decode(Some(&json!({ "is_archived": true })));
    let response = response.expect("the archived flag alone decodes");
    assert_eq!(response.is_archived, Some(true));
    assert!(response.chat.is_none());

    let active: Option<RiseStoredChatDetail> = SaiJSONCodec::decode(Some(&json!({ "is_archived": false })));
    assert_eq!(active.and_then(|response| response.is_archived), Some(false));
}

#[test]
fn test_removing_pagination_boundary_repairs_cursor() {
    assert_eq!(
        ChatArchiveCursorPolicy::repaired_cursor(
            Some("chat-3"),
            &removed(&["chat-3"]),
            &remaining(&["chat-1", "chat-2"])
        ),
        Some("chat-2".to_owned())
    );
    assert_eq!(ChatArchiveCursorPolicy::repaired_cursor(Some("chat-3"), &removed(&["chat-3"]), &[]), None);
    assert_eq!(
        ChatArchiveCursorPolicy::repaired_cursor(
            Some("chat-2"),
            &removed(&["chat-1"]),
            &remaining(&["chat-2", "chat-3"])
        ),
        Some("chat-2".to_owned())
    );
}

#[test]
fn test_archive_reveal_requires_top_pull_and_threshold() {
    assert!(ChatsArchiveRevealPolicy::GESTURE_MINIMUM_DISTANCE > 0.0);
    assert!(ChatsArchiveRevealPolicy::can_begin_pull(ChatsArchiveRevealPhase::Hidden, 0.0, true));
    assert!(!ChatsArchiveRevealPolicy::can_begin_pull(ChatsArchiveRevealPhase::Hidden, -40.0, true));
    assert!(!ChatsArchiveRevealPolicy::should_reveal(ChatsArchiveRevealPolicy::REVEAL_THRESHOLD - 1.0));
    assert!(ChatsArchiveRevealPolicy::should_reveal(ChatsArchiveRevealPolicy::REVEAL_THRESHOLD));
}

#[test]
fn test_marker_comes_from_the_list_top_and_dies_out_below_the_row() {
    assert_eq!(ChatsArchiveRevealPolicy::marker_y(-ChatsArchiveRevealPolicy::ROW_HEIGHT), 0.0);
    assert_eq!(ChatsArchiveRevealPolicy::marker_y(0.0), ChatsArchiveRevealPolicy::ROW_HEIGHT);
    let deep = ChatsArchiveRevealPolicy::marker_y(-4000.0);
    assert_eq!(deep, -ChatsArchiveRevealPolicy::ROW_HEIGHT);
    assert!(!ChatsArchiveRevealPolicy::can_begin_pull(ChatsArchiveRevealPhase::Hidden, deep, true));
    assert!(!ChatsArchiveRevealPolicy::needs_settle(deep, 0.0));
}

#[test]
fn test_short_pull_settles_back_to_the_resting_list() {
    let short_pull = ChatsArchiveRevealPolicy::REVEAL_THRESHOLD - 10.0;
    assert!(ChatsArchiveRevealPolicy::needs_settle(short_pull, 0.0));
    assert_eq!(ChatsArchiveRevealPolicy::settle_target(short_pull, 0.0), ChatsArchiveSettleTarget::Hidden);
    assert_eq!(
        ChatsArchiveRevealPolicy::settle_content_offset_delta(short_pull, ChatsArchiveSettleTarget::Hidden),
        short_pull
    );
    assert_eq!(
        ChatsArchiveRevealPolicy::settle_content_offset_delta(short_pull, ChatsArchiveSettleTarget::Revealed),
        short_pull - ChatsArchiveRevealPolicy::ROW_HEIGHT
    );
}

#[test]
fn test_settle_follows_release_velocity_in_both_directions() {
    let short_pull = ChatsArchiveRevealPolicy::REVEAL_THRESHOLD - 10.0;
    assert_eq!(ChatsArchiveRevealPolicy::settle_target(short_pull, 900.0), ChatsArchiveSettleTarget::Revealed);
    let long_pull = ChatsArchiveRevealPolicy::REVEAL_THRESHOLD + 10.0;
    assert_eq!(ChatsArchiveRevealPolicy::settle_target(long_pull, -900.0), ChatsArchiveSettleTarget::Hidden);
}

#[test]
fn test_resting_positions_are_never_pulled_by_the_settle() {
    assert!(!ChatsArchiveRevealPolicy::needs_settle(0.0, 0.0));
    assert!(!ChatsArchiveRevealPolicy::needs_settle(ChatsArchiveRevealPolicy::ROW_HEIGHT, 0.0));
    assert!(!ChatsArchiveRevealPolicy::needs_settle(-800.0, 0.0));
    assert!(!ChatsArchiveRevealPolicy::needs_settle(ChatsArchiveRevealPolicy::ROW_HEIGHT + 40.0, 0.0));
}

#[test]
fn test_flick_that_carries_past_the_row_keeps_its_inertia() {
    let over_the_row = ChatsArchiveRevealPolicy::ROW_HEIGHT / 2.0;
    assert!(!ChatsArchiveRevealPolicy::needs_settle(over_the_row, -1200.0));
    assert!(!ChatsArchiveRevealPolicy::needs_settle(over_the_row, 1200.0));
    assert!(ChatsArchiveRevealPolicy::needs_settle(over_the_row, 20.0));
}

#[test]
fn test_fast_pull_uses_marker_captured_before_rubber_band_movement() {
    let captured_gesture_start = 0.0;
    let damped_marker_after_fast_pull = 20.0;
    assert!(!ChatsArchiveRevealPolicy::can_begin_pull(
        ChatsArchiveRevealPhase::Hidden,
        damped_marker_after_fast_pull,
        true
    ));
    assert!(ChatsArchiveRevealPolicy::can_begin_pull(ChatsArchiveRevealPhase::Hidden, captured_gesture_start, true));
}

#[test]
fn test_empty_archive_can_still_begin_pull_at_top() {
    assert!(ChatsArchiveRevealPolicy::can_begin_pull(ChatsArchiveRevealPhase::Hidden, 0.0, true));
    assert!(!ChatsArchiveRevealPolicy::can_begin_pull(ChatsArchiveRevealPhase::Hidden, 0.0, false));
    assert!(!ChatsArchiveRevealPolicy::can_begin_pull(ChatsArchiveRevealPhase::Revealed, 0.0, true));
}

#[gpui::test]
fn test_empty_archive_summary_uses_empty_state_copy(cx: &mut gpui::TestAppContext) {
    let row = ChatsArchiveSummaryRow {
        chats: Vec::new(),
        total_count: 0,
        unread_chats_count: 0,
        is_enabled: true,
        action: std::rc::Rc::new(|_: &mut gpui::Window, _: &mut gpui::App| {}),
    };

    assert_eq!(cx.update(|cx| row.subtitle(cx)), tr("chat_archive_empty"));
}

#[test]
fn test_archived_chat_menu_replaces_archive_with_restore_and_hides_pin() {
    let mut chat = ChatInfo::new("archived-chat".to_owned());
    chat.r#type = ChatType::Direct;
    chat.unread_count = 3;
    let actions = ChatListMenuPolicy::actions(&chat, 0, false, false, true, Vec::new(), &HashSet::new());

    assert_eq!(
        actions.iter().map(|action| action.id.as_str()).collect::<Vec<_>>(),
        ["mark_read", "notifications", "wallpaper", "autodelete", "unarchive", "delete"]
    );
    assert!(actions.iter().all(|action| !action.disabled));
    let autodelete = actions
        .iter()
        .find(|action| action.id == "autodelete")
        .and_then(|action| action.submenu.as_ref())
        .expect("auto-delete carries its submenu");
    assert_eq!(
        autodelete.iter().map(|action| action.id.as_str()).collect::<Vec<_>>(),
        ["autodelete_off", "autodelete_1d", "autodelete_1w", "autodelete_1m"]
    );
}

#[test]
fn test_revealed_archive_hides_after_row_leaves_viewport() {
    assert!(ChatsArchiveRevealPolicy::should_hide(ChatsArchiveRevealPhase::Revealed, 0.0));
    assert!(!ChatsArchiveRevealPolicy::should_hide(ChatsArchiveRevealPhase::Hidden, 0.0));
}

#[test]
fn test_interaction_locks_preserve_a_deep_chat_list_position() {
    assert!(!ChatsArchiveScrollResetPolicy::should_normalize_to_normal_top(ChatsArchiveRevealPhase::Hidden, -640.0));
    assert!(!ChatsArchiveScrollResetPolicy::should_normalize_to_normal_top(
        ChatsArchiveRevealPhase::Hidden,
        ChatsArchiveRevealPolicy::TOP_TOLERANCE
    ));
    assert!(ChatsArchiveScrollResetPolicy::should_normalize_to_normal_top(
        ChatsArchiveRevealPhase::Hidden,
        ChatsArchiveRevealPolicy::TOP_TOLERANCE + 1.0
    ));
    assert!(ChatsArchiveScrollResetPolicy::should_normalize_to_normal_top(ChatsArchiveRevealPhase::Pulling, 0.0));
    assert!(ChatsArchiveScrollResetPolicy::should_normalize_to_normal_top(ChatsArchiveRevealPhase::Revealed, -640.0));
}

#[test]
fn test_exiting_inactive_reorder_mode_is_a_no_op() {
    let mut state = ChatsScreenState::new();

    assert!(!state.exit_reorder_mode());

    state.is_reorder_mode = true;
    state.selected_for_reorder = HashSet::from(["chat-1".to_owned()]);
    assert!(state.exit_reorder_mode());
    assert!(!state.is_reorder_mode);
    assert!(state.selected_for_reorder.is_empty());
    assert!(!state.exit_reorder_mode());
}
