use std::collections::HashSet;

use rise_canon::RiseTimingCanon;
use rise_canon::rise_response::rtc::JoinRoom;
use rise_canon::rise_view::rtc::CallCurrent;
use rise_engine::{RiseWireError, RiseWireReplayPolicy};
use rise_endpoints::ENDPOINTS;
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::rtc::engine::core::rise_rtcrpc::RiseRTCEndpoints;
use crate::modules::rtc::engine::core::rise_rtc_store_layout::RiseRTCStoreLayout;
use crate::modules::rtc::engine::core::rise_rtc_wire_models::{RiseRTCJoinCredentialsWire, RiseRTCWireModel};
use crate::modules::rtc::engine::rise_rtc_connection_wait_policy::{
    RiseRTCConnectionWaitPolicy, RiseRTCConnectionWaitPolicySignallingFailure,
};
use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCallMediaMode, RiseRTCCallState, RiseRTCCancelReconciliationAction, RiseRTCCancelReconciliationPolicy,
    RiseRTCCommittedCallMediaKind, RiseRTCCommittedMediaFailureAction, RiseRTCCommittedMediaFailurePolicy,
    RiseRTCDesiredMediaState, RiseRTCDirectCall, RiseRTCJoinCredentials, RiseRTCMediaDisconnectCode, RiseRTCMediaEvent,
    RiseRTCMediaSessionKey, RiseRTCParticipantRole, RiseRTCRoomMode, RiseRTCScreenShareQuality, RiseRTCStateError,
    RiseRTCVideoCeiling, decode_flexible_field, decode_wire_raw_value, encode_flexible_field, encode_wire_raw_value,
};
use crate::modules::rtc::system::rise_rtc_system_bridges::RiseRTCSystemDomainBridgeError;

const RTC_SERVICE: &str = "rtc";
const PARTICIPANT_HEARTBEAT: &str = "rtc_participant";

#[derive(Deserialize)]
struct RoleProbe(#[serde(deserialize_with = "decode_flexible_field")] RiseRTCParticipantRole);

#[derive(Serialize, Deserialize)]
struct RoomModeProbe(
    #[serde(serialize_with = "encode_flexible_field", deserialize_with = "decode_flexible_field")] RiseRTCRoomMode,
);

#[derive(Deserialize)]
struct CallStateProbe(#[serde(deserialize_with = "decode_flexible_field")] RiseRTCCallState);

#[derive(Serialize, Deserialize)]
struct ScreenShareQualityProbe(
    #[serde(serialize_with = "encode_wire_raw_value", deserialize_with = "decode_wire_raw_value")]
    RiseRTCScreenShareQuality,
);

#[test]
fn endpoint_catalogue_owns_every_gateway_rpc_exactly_once() {
    let canon: HashSet<String> = ENDPOINTS
        .iter()
        .filter(|endpoint| endpoint.catalog == Some(RTC_SERVICE))
        .map(|endpoint| endpoint.method.to_owned())
        .collect();
    let endpoints = RiseRTCEndpoints::all();
    let methods: HashSet<String> = endpoints.iter().map(|endpoint| endpoint.method.clone()).collect();
    assert_eq!(methods.len(), endpoints.len());
    assert_eq!(methods, canon);
    assert!(endpoints.iter().all(|endpoint| endpoint.descriptor().service == RTC_SERVICE));
    assert_eq!(RiseRTCEndpoints::get_room_snapshot().descriptor().replay_policy, RiseWireReplayPolicy::ReadOnly);
    assert_eq!(
        RiseRTCEndpoints::send_reaction().descriptor().replay_policy,
        RiseWireReplayPolicy::IdempotencyKey { field: "client_event_id".to_owned() }
    );
    assert_eq!(
        RiseRTCEndpoints::start_direct_call().descriptor().replay_policy,
        RiseWireReplayPolicy::IdempotencyKey { field: "idempotency_key".to_owned() }
    );
    assert_eq!(RiseRTCEndpoints::end_call().descriptor().replay_policy, RiseWireReplayPolicy::Never);
}

#[test]
fn tolerant_enums_accept_proto_integers_and_both_wire_casings() {
    let role = serde_json::from_str::<RoleProbe>("3").map(|probe| probe.0);
    assert_eq!(role.ok(), Some(RiseRTCParticipantRole::Moderator));
    let mode = serde_json::from_str::<RoomModeProbe>(r#""ROOM_MODE_GROUP_VOICE""#).map(|probe| probe.0);
    assert_eq!(mode.ok(), Some(RiseRTCRoomMode::GroupVoice));
    let state = serde_json::from_str::<CallStateProbe>(r#""TIMED_OUT""#).map(|probe| probe.0);
    assert_eq!(state.ok(), Some(RiseRTCCallState::TimedOut));
    let speaker = serde_json::from_str::<RoleProbe>(r#""speaker""#).map(|probe| probe.0);
    assert_eq!(speaker.ok(), Some(RiseRTCParticipantRole::Speaker));
    let keyed = serde_json::from_str::<RoomModeProbe>(r#"{"groupVoice":{}}"#).map(|probe| probe.0);
    assert_eq!(keyed.ok(), Some(RiseRTCRoomMode::GroupVoice));
    let encoded = serde_json::to_string(&RoomModeProbe(RiseRTCRoomMode::GroupVoice));
    assert_eq!(encoded.ok().as_deref(), Some(r#""group_voice""#));
}

const JOIN_ROOM_PAYLOAD: &str = r#"
{
  "success": true,
  "error": null,
  "snapshot": {
    "room": {
      "room_id": "room-1",
      "livekit_room_name": "rise-room-1",
      "chat_id": "chat-1",
      "channel_id": "channel-1",
      "mode": "group_voice",
      "max_participants": 250,
      "max_speakers": 12,
      "video_enabled": true,
      "screen_share_enabled": true,
      "audio_bitrate_kbps": 64,
      "region": "eu",
      "revision": 7,
      "created_at_ms": 1700000000000,
      "updated_at_ms": 1700000000100
    },
    "participant_count": 1,
    "speaker_count": 1,
    "participants": [
      {
        "user_id": "user-1",
        "role": "speaker",
        "connected": true,
        "microphone_enabled": true,
        "camera_enabled": false,
        "screen_share_enabled": false,
        "server_muted": false,
        "server_deafened": false,
        "hand_raised": false,
        "joined_at_ms": 1700000000200,
        "updated_at_ms": 1700000000300,
        "lease_expires_at_ms": 1700000030300,
        "order_sequence": 9,
        "profile": {
          "id": "user-1",
          "tag": "aianov",
          "name": "Aian",
          "avatar_url": "https://cdn.riseonly.net/avatar.webp",
          "is_premium": true,
          "is_online": true,
          "last_seen_ms": 1700000000000
        }
      }
    ],
    "raised_hands": [
      {
        "user_id": "user-2",
        "requested_at_ms": 1700000000400,
        "note": "question",
        "profile": null
      }
    ],
    "generated_at_ms": 1700000000500,
    "profiles_complete": true,
    "profile_error": null
  },
  "participant": {
    "user_id": "user-1",
    "role": "speaker",
    "connected": true,
    "microphone_enabled": true,
    "camera_enabled": false,
    "screen_share_enabled": false,
    "server_muted": false,
    "server_deafened": false,
    "hand_raised": false,
    "joined_at_ms": 1700000000200,
    "updated_at_ms": 1700000000300,
    "lease_expires_at_ms": 1700000030300,
    "order_sequence": 9,
    "profile": null
  },
  "credentials": {
    "server_url": "ws://192.168.31.89:7880",
    "access_token": "secret-token",
    "expires_at_ms": 1700003600000,
    "participant_identity": "rtc-user-1",
    "turn_domain": "192.168.31.89",
    "connection_id": "connection-1"
  }
}
"#;

#[test]
fn join_room_decodes_the_complete_gateway_wire_payload() {
    let response = serde_json::from_str::<JoinRoom>(JOIN_ROOM_PAYLOAD).ok();
    let Some(response) = response else {
        panic!("join_room payload must decode");
    };
    let Some(snapshot) = response.snapshot.as_ref().and_then(|snapshot| snapshot.model()) else {
        panic!("join_room snapshot must carry a room");
    };
    let Some(participant) = response.participant.as_ref().map(|participant| participant.model()) else {
        panic!("join_room must carry the participant");
    };
    let Some(wire_credentials) = response.credentials.as_ref() else {
        panic!("join_room must carry credentials");
    };
    let credentials =
        wire_credentials.credentials(&snapshot.room.room_id, &snapshot.room.livekit_room_name, snapshot.room.mode, None, None);

    assert!(response.success);
    assert_eq!(snapshot.room.room_id, "room-1");
    assert_eq!(snapshot.room.chat_id, "chat-1");
    assert_eq!(snapshot.room.channel_id.as_deref(), Some("channel-1"));
    assert_eq!(snapshot.room.mode, RiseRTCRoomMode::GroupVoice);
    assert_eq!(snapshot.participants.first().map(|participant| participant.user_id.as_str()), Some("user-1"));
    assert_eq!(
        snapshot
            .participants
            .first()
            .and_then(|participant| participant.profile.as_ref())
            .map(|profile| profile.avatar_url.as_str()),
        Some("https://cdn.riseonly.net/avatar.webp")
    );
    assert_eq!(snapshot.raised_hands.first().map(|hand| hand.user_id.as_str()), Some("user-2"));
    assert_eq!(participant.user_id, "user-1");
    assert_eq!(credentials.server_url, "ws://192.168.31.89:7880");
    assert_eq!(credentials.connection_id, "connection-1");
    assert_eq!(credentials.participant_identity, "rtc-user-1");
    assert_eq!(credentials.access_token, "secret-token");
}

const CALL_CURRENT_PAYLOAD: &str = r#"
{
  "success": true,
  "error": null,
  "call": {
    "call_id": "call-1",
    "room_id": "rtc:call:call-1",
    "chat_id": "chat-1",
    "caller_user_id": "caller-1",
    "callee_user_id": "callee-1",
    "media_mode": "video",
    "state": "accepted",
    "created_at_ms": 1700000000000,
    "updated_at_ms": 1700000000100,
    "expires_at_ms": 1700000060000,
    "answered_at_ms": 1700000000050,
    "ended_at_ms": null,
    "ended_by_user_id": null,
    "end_reason": null
  }
}
"#;

#[test]
fn direct_call_decodes_gateway_identifiers_without_changing_store_models() {
    let response = serde_json::from_str::<CallCurrent>(CALL_CURRENT_PAYLOAD).ok();
    let Some(call) = response.and_then(|response| response.call).map(|call| call.model()) else {
        panic!("call_current payload must carry the call");
    };

    assert_eq!(call.call_id, "call-1");
    assert_eq!(call.room_id, "rtc:call:call-1");
    assert_eq!(call.chat_id, "chat-1");
    assert_eq!(call.caller_user_id, "caller-1");
    assert_eq!(call.callee_user_id, "callee-1");
    assert_eq!(call.media_mode, RiseRTCCallMediaMode::Video);
    assert_eq!(call.state, RiseRTCCallState::Accepted);

    let restored = serde_json::to_string(&call)
        .ok()
        .and_then(|stored| serde_json::from_str::<RiseRTCDirectCall>(&stored).ok());
    assert_eq!(restored, Some(call));
}

#[test]
fn equal_timestamp_terminal_call_cannot_be_replaced_by_ringing() {
    let ringing = call(RiseRTCCallState::Ringing, 1_700_000_000_000, RiseRTCCallMediaMode::Audio);
    let accepted = call(RiseRTCCallState::Accepted, ringing.updated_at_ms, RiseRTCCallMediaMode::Audio);
    let ended = call(RiseRTCCallState::Ended, ringing.updated_at_ms, RiseRTCCallMediaMode::Audio);

    assert!(RiseRTCStoreLayout::call_version(&accepted) > RiseRTCStoreLayout::call_version(&ringing));
    assert!(RiseRTCStoreLayout::call_version(&ended) > RiseRTCStoreLayout::call_version(&accepted));
}

#[test]
fn credentials_describe_only_redacted_metadata() {
    let credentials = RiseRTCJoinCredentials {
        room_id: "rtc:call:00000000-0000-0000-0000-000000000001".to_owned(),
        room_name: "room".to_owned(),
        participant_identity: "private-identity".to_owned(),
        connection_id: "private-connection".to_owned(),
        access_token: "super-secret-jwt".to_owned(),
        server_url: "wss://rtc.riseonly.net".to_owned(),
        turn_domain: "turn.riseonly.net".to_owned(),
        expires_at_ms: 9_999_999_999_999,
        room_mode: RiseRTCRoomMode::DirectCall,
        call_media_mode: Some(RiseRTCCallMediaMode::Video),
        audio_bitrate_kbps: None,
        video_ceiling: RiseRTCVideoCeiling::default(),
    };

    assert!(credentials.uses_call_kit_audio_session());
    let description = credentials.description();
    assert!(!description.contains("super-secret-jwt"));
    assert!(!description.contains("private-connection"));
    assert!(!description.contains("private-identity"));
    assert!(description.contains("video"));
    assert!(description.contains("<redacted>"));
}

#[test]
fn media_session_identity_rotates_within_the_same_logical_room() {
    let first = RiseRTCMediaSessionKey {
        room_id: "rtc:room:one".to_owned(),
        connection_id: "connection-1".to_owned(),
        participant_identity: "participant-1".to_owned(),
    };
    let next_connection = RiseRTCMediaSessionKey {
        room_id: first.room_id.clone(),
        connection_id: "connection-2".to_owned(),
        participant_identity: first.participant_identity.clone(),
    };
    let next_identity = RiseRTCMediaSessionKey {
        room_id: first.room_id.clone(),
        connection_id: first.connection_id.clone(),
        participant_identity: "participant-2".to_owned(),
    };

    assert_ne!(first, next_connection);
    assert_ne!(first, next_identity);
    let event = RiseRTCMediaEvent::Connected { session: next_connection.clone() };
    assert_eq!(event.session_key(), Some(&next_connection));
}

#[test]
fn direct_call_desired_media_publishes_mic_and_only_requested_camera() {
    let audio = RiseRTCDesiredMediaState::from_call(&call(
        RiseRTCCallState::Accepted,
        1_700_000_000_000,
        RiseRTCCallMediaMode::Audio,
    ));
    let video = RiseRTCDesiredMediaState::from_call(&call(
        RiseRTCCallState::Accepted,
        1_700_000_000_001,
        RiseRTCCallMediaMode::Video,
    ));

    assert!(audio.microphone_enabled);
    assert!(!audio.camera_enabled);
    assert!(video.microphone_enabled);
    assert!(video.camera_enabled);
}

#[test]
fn screen_share_quality_has_stable_wire_values() {
    assert_eq!(RiseRTCScreenShareQuality::ALL, [RiseRTCScreenShareQuality::Balanced, RiseRTCScreenShareQuality::High]);
    let encoded = serde_json::to_string(&ScreenShareQualityProbe(RiseRTCScreenShareQuality::Balanced));
    assert_eq!(encoded.ok().as_deref(), Some(r#""balanced""#));
    let decoded = serde_json::from_str::<ScreenShareQualityProbe>(r#""high""#).map(|probe| probe.0);
    assert_eq!(decoded.ok(), Some(RiseRTCScreenShareQuality::High));
}

#[test]
fn committed_media_failure_compensates_only_the_owning_live_call() {
    assert_eq!(
        RiseRTCCommittedMediaFailurePolicy::action(RiseRTCCommittedCallMediaKind::OutgoingStart, true, false, false),
        Some(RiseRTCCommittedMediaFailureAction::CancelOutgoing)
    );
    assert_eq!(
        RiseRTCCommittedMediaFailurePolicy::action(RiseRTCCommittedCallMediaKind::AcceptedIncoming, true, false, false),
        Some(RiseRTCCommittedMediaFailureAction::EndAcceptedIncoming)
    );
    assert_eq!(
        RiseRTCCommittedMediaFailurePolicy::action(RiseRTCCommittedCallMediaKind::OutgoingStart, false, false, false),
        None
    );
    assert_eq!(
        RiseRTCCommittedMediaFailurePolicy::action(RiseRTCCommittedCallMediaKind::AcceptedIncoming, true, true, false),
        None
    );
    assert_eq!(
        RiseRTCCommittedMediaFailurePolicy::action(RiseRTCCommittedCallMediaKind::OutgoingStart, true, false, true),
        None
    );
}

#[test]
fn connection_wait_retries_the_link_but_not_a_decision() {
    let failure = |error: anyhow::Error| RiseRTCConnectionWaitPolicy::signalling_failure(&error);
    assert_eq!(failure(RiseWireError::NotConnected.into()), Some(RiseRTCConnectionWaitPolicySignallingFailure::Unsent));
    assert_eq!(failure(RiseWireError::NoSocketSession.into()), Some(RiseRTCConnectionWaitPolicySignallingFailure::Unsent));
    assert_eq!(failure(RiseWireError::Suspended.into()), Some(RiseRTCConnectionWaitPolicySignallingFailure::Unsent));
    assert_eq!(
        failure(RiseWireError::Disconnected("closed".to_owned()).into()),
        Some(RiseRTCConnectionWaitPolicySignallingFailure::Unconfirmed)
    );
    assert_eq!(
        failure(RiseRTCSystemDomainBridgeError::DomainUnavailable.into()),
        Some(RiseRTCConnectionWaitPolicySignallingFailure::Unsent)
    );
    assert_eq!(failure(RiseRTCSystemDomainBridgeError::AccountChanged.into()), None);
    assert_eq!(failure(RiseWireError::Shutdown.into()), None);
    assert_eq!(failure(RiseWireError::StaleAccount.into()), None);
    assert_eq!(failure(RiseRTCStateError::Remote("RTC_RESOURCE_EXHAUSTED".to_owned()).into()), None);

    assert!(RiseRTCConnectionWaitPolicy::is_transient_media_failure(RiseRTCMediaDisconnectCode::LINK_LOST));
    assert!(RiseRTCConnectionWaitPolicy::is_transient_media_failure(RiseRTCMediaDisconnectCode::NETWORK_FAILED));
    assert!(RiseRTCConnectionWaitPolicy::is_transient_media_failure(RiseRTCMediaDisconnectCode::CREDENTIALS_EXPIRED));
    assert!(!RiseRTCConnectionWaitPolicy::is_transient_media_failure(RiseRTCMediaDisconnectCode::PARTICIPANT_REMOVED));
    assert!(!RiseRTCConnectionWaitPolicy::is_transient_media_failure(RiseRTCMediaDisconnectCode::ROOM_DELETED));
    assert!(!RiseRTCConnectionWaitPolicy::is_transient_media_failure("rtc_livekit_identity_mismatch"));
    assert!(!RiseRTCConnectionWaitPolicy::is_transient_media_failure("rtc_livekit_audio_failed"));

    assert!(!RiseRTCConnectionWaitPolicy::needs_fresh_credentials(RiseRTCMediaDisconnectCode::NETWORK_FAILED, 2_000, 1_000));
    assert!(RiseRTCConnectionWaitPolicy::needs_fresh_credentials(RiseRTCMediaDisconnectCode::TOKEN_REJECTED, 2_000, 1_000));
    assert!(RiseRTCConnectionWaitPolicy::needs_fresh_credentials(RiseRTCMediaDisconnectCode::NETWORK_FAILED, 1_000, 1_000));
    assert_eq!(
        RiseRTCConnectionWaitPolicy::window(),
        std::time::Duration::from_millis(RiseTimingCanon::heartbeat(PARTICIPANT_HEARTBEAT).effective_window_ms().max(0) as u64)
    );
    assert_eq!(
        RiseRTCConnectionWaitPolicy::backoff_milliseconds(0),
        RiseRTCConnectionWaitPolicy::backoff_milliseconds(1)
    );
    assert_eq!(
        RiseRTCConnectionWaitPolicy::backoff_milliseconds(99),
        RiseRTCConnectionWaitPolicy::backoff_milliseconds(6)
    );
}

#[test]
fn cancel_reconciliation_is_bounded_and_never_moves_back_from_end() {
    assert_eq!(
        RiseRTCCancelReconciliationPolicy::action(RiseRTCCallState::Ringing, false),
        RiseRTCCancelReconciliationAction::RetryCancel
    );
    assert_eq!(
        RiseRTCCancelReconciliationPolicy::action(RiseRTCCallState::Accepted, false),
        RiseRTCCancelReconciliationAction::EndAccepted
    );
    assert_eq!(
        RiseRTCCancelReconciliationPolicy::action(RiseRTCCallState::Ringing, true),
        RiseRTCCancelReconciliationAction::EndAccepted
    );
    assert_eq!(
        RiseRTCCancelReconciliationPolicy::action(RiseRTCCallState::Cancelled, true),
        RiseRTCCancelReconciliationAction::AlreadyTerminal
    );
}

fn call(state: RiseRTCCallState, updated_at_ms: i64, media_mode: RiseRTCCallMediaMode) -> RiseRTCDirectCall {
    RiseRTCDirectCall {
        call_id: "00000000-0000-0000-0000-000000000001".to_owned(),
        room_id: "rtc:call:00000000-0000-0000-0000-000000000001".to_owned(),
        chat_id: "00000000-0000-0000-0000-000000000002".to_owned(),
        caller_user_id: "00000000-0000-0000-0000-000000000003".to_owned(),
        callee_user_id: "00000000-0000-0000-0000-000000000004".to_owned(),
        media_mode,
        state,
        created_at_ms: updated_at_ms,
        updated_at_ms,
        expires_at_ms: updated_at_ms + 60_000,
        answered_at_ms: None,
        ended_at_ms: None,
        ended_by_user_id: None,
        end_reason: None,
        caller_e2ee_public_key: None,
        callee_e2ee_public_key: None,
        recording_state: None,
        recording_started_by_user_id: None,
        recording_path: None,
    }
}
