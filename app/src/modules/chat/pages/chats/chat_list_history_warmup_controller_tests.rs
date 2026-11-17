use std::collections::HashSet;

use super::*;

#[allow(dead_code)]
struct ChatListHistoryWarmupPolicyTests;

fn replacement_action(
    old_chat_id: &str,
    new_chat_id: &str,
    old_account: (&str, &str),
    new_account: (&str, &str),
    old_lifecycle_generation: u64,
) -> ChatListForumPresentationReplacementPolicyAction {
    ChatListForumPresentationReplacementPolicy::action(
        old_chat_id,
        new_chat_id,
        Some(old_account.0),
        Some(old_account.1),
        Some(new_account.0),
        Some(new_account.1),
        old_lifecycle_generation,
        3,
    )
}

fn signature(ids: &[String], topics_enabled: &[bool]) -> InputSignature {
    ChatListHistoryWarmupPolicy::bounded_input_signature(
        ids.len() as i64,
        50,
        |index| ids[index as usize].clone(),
        |index| topics_enabled[index as usize],
    )
}

fn hundred_chat_ids() -> Vec<String> {
    (0..100).map(|index| format!("chat-{index}")).collect()
}

#[test]
fn initial_candidates_start_at_visible_row_and_alternate_nearby() {
    assert_eq!(
        ChatListHistoryWarmupPolicy::candidate_indices(20, 5, None, Some(5)),
        vec![5, 6, 4, 7, 3]
    );
}

#[test]
fn moving_down_prioritizes_rows_ahead() {
    assert_eq!(
        ChatListHistoryWarmupPolicy::candidate_indices(20, 8, Some(5), Some(5)),
        vec![8, 9, 7, 10, 6]
    );
}

#[test]
fn moving_up_prioritizes_rows_ahead_in_reverse_direction() {
    assert_eq!(
        ChatListHistoryWarmupPolicy::candidate_indices(20, 8, Some(11), Some(5)),
        vec![8, 7, 9, 6, 10]
    );
}

#[test]
fn candidates_stay_in_bounds_and_unique() {
    let indices = ChatListHistoryWarmupPolicy::candidate_indices(3, 0, Some(2), Some(12));
    assert_eq!(indices, vec![0, 1, 2]);
    assert_eq!(indices.iter().collect::<HashSet<_>>().len(), indices.len());
}

#[test]
fn forum_candidates_stay_nearby_and_bounded() {
    let forum_indices: HashSet<i64> = [4, 7, 8, 12, 18].into_iter().collect();
    let candidates =
        ChatListHistoryWarmupPolicy::forum_candidate_indices(20, 5, None, None, |index| forum_indices.contains(&index));

    assert_eq!(candidates, vec![4, 7, 8]);
    assert_eq!(candidates.len() as i64, ChatListHistoryWarmupPolicy::maximum_forum_candidates());
}

#[test]
fn forum_candidates_follow_scroll_direction() {
    let forum_indices: HashSet<i64> = [6, 7, 9, 10, 14].into_iter().collect();

    assert_eq!(
        ChatListHistoryWarmupPolicy::forum_candidate_indices(20, 8, Some(5), None, |index| forum_indices
            .contains(&index)),
        vec![9, 7, 10]
    );
    assert_eq!(
        ChatListHistoryWarmupPolicy::forum_candidate_indices(20, 8, Some(11), None, |index| forum_indices
            .contains(&index)),
        vec![7, 9, 6]
    );
}

#[test]
fn forum_candidate_policy_honors_small_custom_limit_and_invalid_input() {
    assert_eq!(
        ChatListHistoryWarmupPolicy::forum_candidate_indices(100, 50, Some(40), Some(2), |_| true),
        vec![50, 51]
    );
    assert!(ChatListHistoryWarmupPolicy::forum_candidate_indices(4, -1, None, None, |_| true).is_empty());
    assert!(ChatListHistoryWarmupPolicy::forum_candidate_indices(4, 1, None, Some(0), |_| true).is_empty());
}

#[test]
fn bounded_input_signature_detects_relevant_candidate_mutations() {
    let mut ids = hundred_chat_ids();
    let mut topics_enabled = vec![false; ids.len()];

    let original = signature(&ids, &topics_enabled);
    topics_enabled[55] = true;
    assert_ne!(signature(&ids, &topics_enabled), original);

    topics_enabled[55] = false;
    ids[3] = "new-recent-chat".to_owned();
    assert_ne!(signature(&ids, &topics_enabled), original);
}

#[test]
fn bounded_input_signature_ignores_unrelated_far_rows() {
    let mut ids = hundred_chat_ids();
    let topics_enabled = vec![false; ids.len()];

    let original = signature(&ids, &topics_enabled);
    assert!(original.entries.len() as i64 <= ChatListHistoryWarmupPolicy::maximum_scan_distance() * 3 + 1);
    ids[99] = "unrelated-far-chat".to_owned();
    assert_eq!(signature(&ids, &topics_enabled), original);
}

#[test]
fn bounded_input_signature_rejects_invalid_center() {
    assert!(
        ChatListHistoryWarmupPolicy::bounded_input_signature(4, 4, |index| format!("chat-{index}"), |_| false)
            .entries
            .is_empty()
    );
}

#[test]
fn same_forum_reopen_transfers_deferred_opening_payloads() {
    assert_eq!(
        replacement_action("forum-a", "forum-a", ("user", "epoch"), ("user", "epoch"), 3),
        ChatListForumPresentationReplacementPolicyAction::TransferToNewPresentation
    );
}

#[test]
fn different_forum_reopen_releases_only_disjoint_payloads() {
    assert_eq!(
        replacement_action("forum-a", "forum-b", ("user", "epoch"), ("user", "epoch"), 3),
        ChatListForumPresentationReplacementPolicyAction::ReleaseDisjointPayloads
    );
}

#[test]
fn forum_reopen_discards_payloads_across_account_or_lifecycle_changes() {
    assert_eq!(
        replacement_action("forum-a", "forum-a", ("user-a", "epoch"), ("user-b", "epoch"), 3),
        ChatListForumPresentationReplacementPolicyAction::Discard
    );
    assert_eq!(
        replacement_action("forum-a", "forum-a", ("user", "epoch-a"), ("user", "epoch-b"), 3),
        ChatListForumPresentationReplacementPolicyAction::Discard
    );
    assert_eq!(
        replacement_action("forum-a", "forum-a", ("user", "epoch"), ("user", "epoch"), 2),
        ChatListForumPresentationReplacementPolicyAction::Discard
    );
}
