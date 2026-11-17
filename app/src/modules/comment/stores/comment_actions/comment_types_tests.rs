use crate::modules::comment::components::native_list::comment_list_controller::CommentListPolicy;
use crate::modules::comment::engine::rise_comment_engine_models::{RiseCommentRecord, RiseCommentScope};
use crate::modules::comment::engine::rise_comment_policies::{
    RiseCommentReactionSession, RiseCommentReactionSessionResolution, RiseCommentReactionSessionStep,
    RiseCommentReactionState, RiseCommentReplyPlacement,
};
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentMedia, CommentMediaType, CommentReaction, CommentReactionCommand, CommentReactionEndpoint, CommentSortType,
    GetCommentsResponse, RepliesSortType,
};

fn session(base: CommentReaction) -> RiseCommentReactionSession {
    RiseCommentReactionSession {
        base: state(base, 3, 1),
        desired: base,
        generation: 0,
        inflight_generation: None,
    }
}

fn state(reaction: CommentReaction, likes_count: i64, dislikes_count: i64) -> RiseCommentReactionState {
    RiseCommentReactionState { reaction, likes_count, dislikes_count }
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn derived_state_moves_the_counters_with_the_desired_reaction() {
    let mut liked = session(CommentReaction::None);
    liked.desired = CommentReaction::Like;
    assert_eq!(liked.derived(), state(CommentReaction::Like, 4, 1));

    let mut switched = session(CommentReaction::Like);
    switched.desired = CommentReaction::Dislike;
    assert_eq!(switched.derived(), state(CommentReaction::Dislike, 2, 2));
}

#[test]
fn a_tap_back_to_the_confirmed_state_sends_nothing() {
    let mut value = session(CommentReaction::Like);
    value.desired = CommentReaction::None;
    value.desired = CommentReaction::Like;
    value.generation = 2;
    assert_eq!(value.next_step("1"), RiseCommentReactionSessionStep::Settle(value.base));
}

#[test]
fn removing_a_reaction_goes_through_the_endpoint_that_set_it() {
    let mut value = session(CommentReaction::Dislike);
    value.desired = CommentReaction::None;
    value.generation = 1;
    assert_eq!(
        value.next_step("7"),
        RiseCommentReactionSessionStep::Send(CommentReactionCommand {
            comment_id: "7".to_owned(),
            endpoint: CommentReactionEndpoint::Dislike,
            enabled: false,
            generation: 1,
        })
    );
}

#[test]
fn a_second_tap_waits_for_the_request_in_flight() {
    let mut value = session(CommentReaction::None);
    value.desired = CommentReaction::Like;
    value.generation = 1;
    let _ = value.next_step("1");
    value.desired = CommentReaction::Dislike;
    value.generation = 2;
    assert_eq!(value.next_step("1"), RiseCommentReactionSessionStep::Wait);

    let acknowledged = state(CommentReaction::Like, 4, 1);
    assert_eq!(value.acknowledge(1, acknowledged), RiseCommentReactionSessionResolution::Continues);
    assert_eq!(value.base, acknowledged);
    assert_eq!(
        value.next_step("1"),
        RiseCommentReactionSessionStep::Send(CommentReactionCommand {
            comment_id: "1".to_owned(),
            endpoint: CommentReactionEndpoint::Dislike,
            enabled: true,
            generation: 2,
        })
    );
}

#[test]
fn a_failed_request_falls_back_to_the_confirmed_state() {
    let mut value = session(CommentReaction::None);
    value.desired = CommentReaction::Like;
    value.generation = 1;
    let _ = value.next_step("1");
    assert_eq!(value.fail(1), RiseCommentReactionSessionResolution::Finished(value.base));
}

#[test]
fn an_answer_for_a_foreign_generation_is_ignored() {
    let mut value = session(CommentReaction::None);
    value.desired = CommentReaction::Like;
    value.generation = 5;
    let _ = value.next_step("1");
    let answer = state(CommentReaction::Like, 9, 9);
    assert_eq!(value.acknowledge(4, answer), RiseCommentReactionSessionResolution::Ignored);
    assert_eq!(value.fail(4), RiseCommentReactionSessionResolution::Ignored);
}

#[test]
fn a_reply_to_another_reply_lands_under_its_addressee() {
    let mut placement = RiseCommentReplyPlacement::new(8);
    placement.remember("temp_a", "2");
    assert_eq!(placement.arranged(&ids(&["1", "2", "3", "temp_a"]), "root"), ids(&["1", "2", "temp_a", "3"]));
}

#[test]
fn a_reply_to_the_root_continues_at_the_tail() {
    let mut placement = RiseCommentReplyPlacement::new(8);
    placement.remember("temp_a", "root");
    placement.remember("temp_b", "root");
    assert_eq!(placement.arranged(&ids(&["temp_b", "1", "temp_a", "2"]), "root"), ids(&["1", "2", "temp_a", "temp_b"]));
}

#[test]
fn the_acknowledgement_keeps_the_place() {
    let mut placement = RiseCommentReplyPlacement::new(8);
    placement.remember("temp_a", "1");
    placement.rename("temp_a", "90");
    assert_eq!(placement.arranged(&ids(&["1", "2", "90"]), "root"), ids(&["1", "90", "2"]));
}

#[test]
fn the_table_is_bounded() {
    let mut placement = RiseCommentReplyPlacement::new(2);
    placement.remember("a", "root");
    placement.remember("b", "root");
    placement.remember("c", "root");
    assert_eq!(placement.order().to_vec(), ids(&["b", "c"]));
    assert!(!placement.target_by_reply().contains_key("a"));
}

#[test]
fn the_next_page_is_requested_before_the_end() {
    assert!(CommentListPolicy::should_load_more(900.0, 700.0, true, false));
    assert!(!CommentListPolicy::should_load_more(1_400.0, 700.0, true, false));
}

#[test]
fn nothing_is_requested_without_more_or_while_a_page_is_in_flight() {
    assert!(!CommentListPolicy::should_load_more(0.0, 700.0, false, false));
    assert!(!CommentListPolicy::should_load_more(0.0, 700.0, true, true));
}

#[test]
fn scope_keys_are_collision_safe() {
    let first = RiseCommentScope::Post { post_id: "1|feed".to_owned(), sort: CommentSortType::New }.storage_key();
    let second = RiseCommentScope::Post { post_id: "1".to_owned(), sort: CommentSortType::Feed }.storage_key();
    assert_ne!(first, second);
    assert!(!RiseCommentScope::User { user_id: "u".to_owned() }.is_persisted());
    assert!(RiseCommentScope::Replies { root_id: "9".to_owned(), sort: RepliesSortType::Old }.is_persisted());
}

#[test]
fn a_record_survives_the_round_trip() -> serde_json::Result<()> {
    let mut media = CommentMedia::new(CommentMediaType::Photo, "https://cdn.example/p.jpg".to_owned());
    media.width = 10;
    media.height = 20;
    let comment = GetCommentsResponse {
        id: "42".to_owned(),
        content: "hello".to_owned(),
        post_id: "7".to_owned(),
        likes_count: 3,
        user_liked: true,
        media: Some(media),
        ..GetCommentsResponse::default()
    };

    let data = serde_json::to_vec(&RiseCommentRecord::new(&comment))?;
    let restored = serde_json::from_slice::<RiseCommentRecord>(&data)?.as_comment();
    assert_eq!(restored, comment);
    Ok(())
}
