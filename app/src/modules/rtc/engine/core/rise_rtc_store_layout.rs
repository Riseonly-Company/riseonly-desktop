use std::sync::LazyLock;

use rise_engine::rise_store::rise_store_entities::RisePositionKey;
use rise_engine::rise_store::rise_store_models::RiseStoreNamespace;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCallStateExt, RiseRTCDirectCall, RiseRTCParticipant, RiseRTCParticipantRole, RiseRTCRaisedHand,
    RiseRTCRoomConfig, RiseRTCUserProfile,
};

fn canon_namespace(name: &str) -> RiseStoreNamespace {
    match rise_rtc::store_namespace(name) {
        Some(entry) => RiseStoreNamespace::new(entry.namespace),
        None => panic!("Canon rtc store namespace {name} is missing"),
    }
}

static ROOMS: LazyLock<RiseStoreNamespace> = LazyLock::new(|| canon_namespace("rooms"));
static PARTICIPANTS: LazyLock<RiseStoreNamespace> = LazyLock::new(|| canon_namespace("participants"));
static RAISED_HANDS: LazyLock<RiseStoreNamespace> = LazyLock::new(|| canon_namespace("raised_hands"));
static CALLS: LazyLock<RiseStoreNamespace> = LazyLock::new(|| canon_namespace("calls"));
static LISTENER_VOLUMES: LazyLock<RiseStoreNamespace> = LazyLock::new(|| canon_namespace("listener_volumes"));

pub enum RiseRTCStoreLayout {}

impl RiseRTCStoreLayout {
    pub const RECENT_ROOMS_VIEW: &'static str = "rtc.rooms.recent";
    pub const RECENT_CALLS_VIEW: &'static str = "rtc.calls.recent";
    pub const ACTIVE_ROOM_MARKER_KEY: &'static str = "rtc.active-room-marker.v1";

    pub fn rooms() -> RiseStoreNamespace {
        ROOMS.clone()
    }

    pub fn participants() -> RiseStoreNamespace {
        PARTICIPANTS.clone()
    }

    pub fn raised_hands() -> RiseStoreNamespace {
        RAISED_HANDS.clone()
    }

    pub fn calls() -> RiseStoreNamespace {
        CALLS.clone()
    }

    pub fn listener_volumes() -> RiseStoreNamespace {
        LISTENER_VOLUMES.clone()
    }

    pub fn participant_view(room_id: &str) -> String {
        Self::encoded(&["rtc.room.participants", room_id])
    }

    pub fn raised_hands_view(room_id: &str) -> String {
        Self::encoded(&["rtc.room.raised-hands", room_id])
    }

    pub fn room_meta_key(room_id: &str) -> String {
        Self::encoded(&["rtc.room.meta", room_id])
    }

    pub fn participant_local_id(room_id: &str, user_id: &str) -> String {
        Self::encoded(&[room_id, user_id])
    }

    pub fn raised_hand_local_id(room_id: &str, user_id: &str) -> String {
        Self::encoded(&[room_id, user_id])
    }

    pub fn call_version(call: &RiseRTCDirectCall) -> i64 {
        Self::composite_version(call.updated_at_ms, call.state.merge_rank())
    }

    pub fn room_version(room: &RiseRTCRoomConfig) -> i64 {
        Self::composite_version(room.updated_at_ms, 0).max(room.revision)
    }

    pub fn participant_version(participant: &RiseRTCParticipant) -> i64 {
        Self::composite_version(participant.updated_at_ms, 0)
    }

    pub fn hand_version(hand: &RiseRTCRaisedHand) -> i64 {
        Self::composite_version(hand.requested_at_ms, 0)
    }

    /// timestamp version from milliseconds and rank, rank defaults to 0 on ios
    pub fn timestamp_version(milliseconds: i64, rank: i64) -> i64 {
        Self::composite_version(milliseconds, rank)
    }

    pub fn call_position(call: &RiseRTCDirectCall) -> RisePositionKey {
        RisePositionKey::make(|key| {
            key.append_signed_int64(call.updated_at_ms);
            key.append_string(&call.call_id);
        })
    }

    pub fn room_position(room: &RiseRTCRoomConfig) -> RisePositionKey {
        RisePositionKey::make(|key| {
            key.append_signed_int64(room.updated_at_ms);
            key.append_string(&room.room_id);
        })
    }

    pub fn participant_position(participant: &RiseRTCParticipant) -> RisePositionKey {
        let role_rank: i64 = match participant.role {
            RiseRTCParticipantRole::Moderator => 0,
            RiseRTCParticipantRole::Speaker => 1,
            RiseRTCParticipantRole::Listener => 2,
            RiseRTCParticipantRole::Unspecified => 3,
        };
        RisePositionKey::make(|key| {
            key.append_signed_int64(role_rank);
            key.append_signed_int64(participant.order_sequence);
            key.append_signed_int64(participant.joined_at_ms);
            key.append_string(&participant.user_id);
        })
    }

    pub fn hand_position(hand: &RiseRTCRaisedHand) -> RisePositionKey {
        RisePositionKey::make(|key| {
            key.append_signed_int64(hand.requested_at_ms);
            key.append_string(&hand.user_id);
        })
    }

    /// sorted keys and slashes unescaped, so the bytes of one value never depend on field order
    pub fn encode<T: Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
        let tree = serde_json::to_value(value)?;
        Ok(serde_json::to_vec(&tree)?)
    }

    pub fn decode<T: DeserializeOwned>(data: &[u8]) -> anyhow::Result<T> {
        Ok(serde_json::from_slice(data)?)
    }

    fn composite_version(milliseconds: i64, rank: i64) -> i64 {
        let safe_milliseconds = milliseconds.max(0);
        let bounded_rank = rank.clamp(0, 15);
        match safe_milliseconds.checked_mul(16) {
            Some(scaled) => scaled + bounded_rank,
            None => i64::MAX - 15 + bounded_rank,
        }
    }

    fn encoded(values: &[&str]) -> String {
        values.iter().map(|value| format!("{}:{}", value.len(), value)).collect::<Vec<_>>().join("|")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseRTCRoomMeta {
    pub participant_count: i64,
    pub speaker_count: i64,
    pub generated_at_ms: i64,
    pub profiles_complete: bool,
    pub profile_error: Option<String>,
    pub has_more_participants: bool,
    pub next_participant_cursor: Option<String>,
    pub has_more_raised_hands: bool,
    pub next_raised_hand_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseRTCStoredCall {
    pub call: RiseRTCDirectCall,
    pub caller: Option<RiseRTCUserProfile>,
    pub event_id: Option<String>,
    pub occurred_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct RiseRTCCallRepositorySnapshot {
    pub account_id: String,
    pub revision: i64,
    pub calls: Vec<RiseRTCStoredCall>,
}

#[derive(Clone, Debug)]
pub struct RiseRTCRoomRepositorySnapshot {
    pub account_id: String,
    pub revision: i64,
    pub room: Option<RiseRTCRoomConfig>,
    pub participants: Vec<RiseRTCParticipant>,
    pub raised_hands: Vec<RiseRTCRaisedHand>,
    pub meta: Option<RiseRTCRoomMeta>,
}
