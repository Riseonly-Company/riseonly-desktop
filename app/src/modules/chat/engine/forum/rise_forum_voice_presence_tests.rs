use super::rise_forum_voice_presence::{
    RiseForumVoicePresenceRepository, RiseForumVoiceUpdate, RiseForumVoiceUpdateKind, RiseForumVoiceUpdateScope,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatTopic, VoiceChannelParticipant, VoiceChannelState,
};
use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCParticipantRole;

const ACCOUNT_ID: &str = "self-user";
const OTHER_ID: &str = "other-user";
const CHAT_ID: &str = "claim-chat";
const TOPIC_ID: &str = "voice";

fn participant(user_id: &str, sequence: i64) -> VoiceChannelParticipant {
    VoiceChannelParticipant {
        user_id: user_id.to_owned(),
        name: user_id.to_owned(),
        tag: user_id.to_owned(),
        avatar_url: String::new(),
        is_premium: false,
        is_official: false,
        role_name: String::new(),
        role_color: 0,
        microphone_enabled: true,
        camera_enabled: false,
        screen_share_enabled: false,
        deafened: false,
        server_muted: false,
        server_deafened: false,
        connected: true,
        joined_at_ms: if sequence == i64::MAX { i64::MAX } else { 1_000 + sequence },
        order_sequence: sequence,
        role: RiseRTCParticipantRole::Unspecified,
        hand_raised: false,
        hand_expires_at_ms: None,
    }
}

fn listing(topic_id: &str, participants: Vec<VoiceChannelParticipant>, generated_at_ms: i64) -> ChatTopic {
    let mut topic =
        ChatTopic::new(topic_id.to_owned(), topic_id.to_owned(), String::new(), 0, false, false);
    topic.channel_kind = ChatChannelKind::Voice;
    topic.voice_state = Some(VoiceChannelState {
        room_id: format!("room-{topic_id}"),
        participant_count: i64::try_from(participants.len()).unwrap_or(i64::MAX),
        user_limit: 0,
        participants,
        has_more: false,
        generated_at_ms,
        session_started_at_ms: 0,
        visible_limit: VoiceChannelState::COLLAPSED_ROWS,
    });
    topic
}

fn server_event(
    kind: RiseForumVoiceUpdateKind,
    user_id: &str,
    count: i64,
    participant: Option<VoiceChannelParticipant>,
    occurred_at_ms: i64,
) -> RiseForumVoiceUpdate {
    RiseForumVoiceUpdate {
        chat_id: CHAT_ID.to_owned(),
        topic_id: TOPIC_ID.to_owned(),
        scope: RiseForumVoiceUpdateScope::Channel,
        room_id: format!("room-{TOPIC_ID}"),
        kind,
        user_id: user_id.to_owned(),
        participant_count: count,
        participant,
        occurred_at_ms,
    }
}

fn state(repository: &RiseForumVoicePresenceRepository) -> Option<VoiceChannelState> {
    repository.snapshot().state(CHAT_ID, TOPIC_ID)
}

fn roster(repository: &RiseForumVoicePresenceRepository) -> Vec<String> {
    state(repository)
        .map(|state| state.participants.into_iter().map(|row| row.user_id).collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn a_listing_that_predates_the_join_keeps_the_claimed_seat() {
    let repository = RiseForumVoicePresenceRepository::new();
    repository.activate(ACCOUNT_ID).await;
    repository.adopt(CHAT_ID, &[listing(TOPIC_ID, vec![participant(OTHER_ID, 1)], 10_000)]).await;

    repository.apply_optimistic_self(CHAT_ID, TOPIC_ID, true, Some(participant(ACCOUNT_ID, i64::MAX))).await;
    assert_eq!(roster(&repository), vec![OTHER_ID, ACCOUNT_ID]);

    repository.adopt(CHAT_ID, &[listing(TOPIC_ID, vec![participant(OTHER_ID, 1)], 20_000)]).await;
    let held = state(&repository);
    assert_eq!(roster(&repository), vec![OTHER_ID, ACCOUNT_ID]);
    assert_eq!(held.as_ref().map(|state| state.participant_count), Some(2));
    assert_eq!(held.as_ref().map(|state| state.generated_at_ms), Some(20_000));
}

#[tokio::test]
async fn the_servers_own_joined_settles_the_claim_so_a_later_listing_is_authoritative() {
    let repository = RiseForumVoicePresenceRepository::new();
    repository.activate(ACCOUNT_ID).await;
    repository.apply_optimistic_self(CHAT_ID, TOPIC_ID, true, Some(participant(ACCOUNT_ID, i64::MAX))).await;
    repository
        .apply(server_event(RiseForumVoiceUpdateKind::Joined, ACCOUNT_ID, 1, Some(participant(ACCOUNT_ID, 7)), 30_000))
        .await;
    assert_eq!(roster(&repository), vec![ACCOUNT_ID]);

    repository.adopt(CHAT_ID, &[listing(TOPIC_ID, Vec::new(), 40_000)]).await;
    assert!(roster(&repository).is_empty());
}

#[tokio::test]
async fn a_tap_after_a_heard_left_claims_the_seat_again() {
    let repository = RiseForumVoicePresenceRepository::new();
    repository.activate(ACCOUNT_ID).await;
    let self_row = participant(ACCOUNT_ID, i64::MAX);

    repository.apply_optimistic_self(CHAT_ID, TOPIC_ID, true, Some(self_row.clone())).await;
    repository.apply(server_event(RiseForumVoiceUpdateKind::Left, ACCOUNT_ID, 0, None, 50_000)).await;
    assert!(roster(&repository).is_empty());

    repository.apply_optimistic_self(CHAT_ID, TOPIC_ID, true, Some(self_row)).await;
    assert_eq!(roster(&repository), vec![ACCOUNT_ID]);
    assert_eq!(state(&repository).map(|state| state.participant_count), Some(1));
}

#[tokio::test]
async fn repeated_leaves_do_not_drift_the_head_count() {
    let repository = RiseForumVoicePresenceRepository::new();
    repository.activate(ACCOUNT_ID).await;
    repository.adopt(CHAT_ID, &[listing(TOPIC_ID, vec![participant(OTHER_ID, 1)], 10_000)]).await;
    repository.apply_optimistic_self(CHAT_ID, TOPIC_ID, true, Some(participant(ACCOUNT_ID, i64::MAX))).await;
    for _ in 0..3 {
        repository.apply_optimistic_self(CHAT_ID, TOPIC_ID, false, None).await;
    }
    assert_eq!(roster(&repository), vec![OTHER_ID]);
    assert_eq!(state(&repository).map(|state| state.participant_count), Some(1));
}

#[tokio::test]
async fn focusing_the_room_again_keeps_the_seated_mute() {
    let repository = RiseForumVoicePresenceRepository::new();
    repository.activate(ACCOUNT_ID).await;
    let mut muted = participant(ACCOUNT_ID, 3);
    muted.microphone_enabled = false;
    repository.adopt(CHAT_ID, &[listing(TOPIC_ID, vec![participant(OTHER_ID, 1), muted], 10_000)]).await;

    repository.apply_optimistic_self(CHAT_ID, TOPIC_ID, true, Some(participant(ACCOUNT_ID, i64::MAX))).await;
    let mine = state(&repository).and_then(|state| state.participants.into_iter().find(|row| row.user_id == ACCOUNT_ID));
    assert_eq!(mine.map(|row| row.is_microphone_muted()), Some(true));
    assert_eq!(roster(&repository), vec![OTHER_ID, ACCOUNT_ID]);
}
