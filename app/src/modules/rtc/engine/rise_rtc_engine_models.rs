use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

use futures::channel::mpsc::UnboundedReceiver;
use futures::future::BoxFuture;
use rise_limits::service_limit::{VIDEO_PUBLISH_MAX_FPS, VIDEO_PUBLISH_MAX_HEIGHT};
use rise_rtc::{CallRecordingState, RtcWireEnum, normalize_wire};
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};

pub trait RiseRTCFlexibleWireEnum: RtcWireEnum + Send + Sync {
    fn parse(value: &str) -> Self;
    fn parse_int(value: i64) -> Self;
    fn wire_value(self) -> &'static str;

    fn decode_flexible<'de, D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        decoder.deserialize_any(RiseRTCFlexibleWireVisitor::<Self>(PhantomData))
    }

    fn encode_flexible<S: Serializer>(self, encoder: S) -> Result<S::Ok, S::Error> {
        encoder.serialize_str(self.wire_value())
    }

    fn normalized(value: &str) -> String {
        normalize_wire(value, false)
    }
}

pub fn encode_flexible_field<T: RiseRTCFlexibleWireEnum, S: Serializer>(value: &T, encoder: S) -> Result<S::Ok, S::Error> {
    value.encode_flexible(encoder)
}

pub fn decode_flexible_field<'de, T: RiseRTCFlexibleWireEnum, D: Deserializer<'de>>(decoder: D) -> Result<T, D::Error> {
    T::decode_flexible(decoder)
}

pub fn encode_wire_raw_value<T: RtcWireEnum, S: Serializer>(value: &T, encoder: S) -> Result<S::Ok, S::Error> {
    encoder.serialize_str(value.wire())
}

pub fn decode_wire_raw_value<'de, T: RtcWireEnum, D: Deserializer<'de>>(decoder: D) -> Result<T, D::Error> {
    let raw = String::deserialize(decoder)?;
    T::VALUES
        .iter()
        .copied()
        .find(|candidate| candidate.wire() == raw)
        .ok_or_else(|| de::Error::unknown_variant(&raw, &[]))
}

struct RiseRTCFlexibleWireVisitor<T>(PhantomData<T>);

impl<'de, T: RiseRTCFlexibleWireEnum> Visitor<'de> for RiseRTCFlexibleWireVisitor<T> {
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an RTC enum string, integer, or legacy keyed value")
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<T, E> {
        Ok(T::parse_int(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<T, E> {
        i64::try_from(value)
            .map(T::parse_int)
            .map_err(|_| de::Error::invalid_value(de::Unexpected::Unsigned(value), &self))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<T, E> {
        Ok(T::parse(value))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<T, A::Error> {
        let mut first: Option<String> = None;
        let mut count = 0usize;
        while let Some(key) = map.next_key::<String>()? {
            map.next_value::<IgnoredAny>()?;
            if first.is_none() {
                first = Some(key);
            }
            count += 1;
        }
        match first {
            Some(key) if count == 1 => Ok(T::parse(&key)),
            _ => Err(de::Error::invalid_length(count, &self)),
        }
    }
}

fn proto_number(value: i64) -> Option<i32> {
    i32::try_from(value).ok()
}

pub type RiseRTCRoomMode = rise_rtc::RoomMode;
pub type RiseRTCParticipantRole = rise_rtc::ParticipantRole;
pub type RiseRTCCallMediaMode = rise_rtc::CallMediaMode;
pub type RiseRTCCallState = rise_rtc::CallState;
pub type RiseRTCModerationAction = rise_rtc::ModerationAction;

impl RiseRTCFlexibleWireEnum for RiseRTCRoomMode {
    fn parse(value: &str) -> Self {
        Self::from_alias(&Self::normalized(value)).unwrap_or(Self::Unspecified)
    }

    fn parse_int(value: i64) -> Self {
        proto_number(value).and_then(Self::from_proto).unwrap_or(Self::Unspecified)
    }

    fn wire_value(self) -> &'static str {
        self.wire()
    }
}

pub trait RiseRTCRoomModeExt {
    fn uses_call_kit_audio_session(self) -> bool;
}

impl RiseRTCRoomModeExt for RiseRTCRoomMode {
    fn uses_call_kit_audio_session(self) -> bool {
        self == Self::DirectCall
    }
}

impl RiseRTCFlexibleWireEnum for RiseRTCParticipantRole {
    fn parse(value: &str) -> Self {
        Self::from_alias(&Self::normalized(value)).unwrap_or(Self::Unspecified)
    }

    fn parse_int(value: i64) -> Self {
        proto_number(value).and_then(Self::from_proto).unwrap_or(Self::Unspecified)
    }

    fn wire_value(self) -> &'static str {
        self.wire()
    }
}

impl RiseRTCFlexibleWireEnum for RiseRTCCallMediaMode {
    fn parse(value: &str) -> Self {
        Self::from_alias(&Self::normalized(value)).unwrap_or(Self::Unspecified)
    }

    fn parse_int(value: i64) -> Self {
        proto_number(value).and_then(Self::from_proto).unwrap_or(Self::Unspecified)
    }

    fn wire_value(self) -> &'static str {
        self.wire()
    }
}

impl RiseRTCFlexibleWireEnum for RiseRTCCallState {
    fn parse(value: &str) -> Self {
        Self::from_alias(&Self::normalized(value)).unwrap_or(Self::Unspecified)
    }

    fn parse_int(value: i64) -> Self {
        proto_number(value).and_then(Self::from_proto).unwrap_or(Self::Unspecified)
    }

    fn wire_value(self) -> &'static str {
        self.wire()
    }
}

pub trait RiseRTCCallStateExt {
    fn is_terminal(self) -> bool;
    fn merge_rank(self) -> i64;
}

impl RiseRTCCallStateExt for RiseRTCCallState {
    fn is_terminal(self) -> bool {
        match self {
            Self::Declined | Self::Cancelled | Self::TimedOut | Self::Ended => true,
            Self::Unspecified | Self::Ringing | Self::Accepted => false,
        }
    }

    fn merge_rank(self) -> i64 {
        match self {
            Self::Unspecified => 0,
            Self::Ringing => 1,
            Self::Accepted => 2,
            Self::Declined => 3,
            Self::Cancelled => 4,
            Self::TimedOut => 5,
            Self::Ended => 6,
        }
    }
}

impl RiseRTCFlexibleWireEnum for RiseRTCModerationAction {
    fn parse(value: &str) -> Self {
        Self::from_alias(&Self::normalized(value)).unwrap_or(Self::Unspecified)
    }

    fn parse_int(value: i64) -> Self {
        proto_number(value).and_then(Self::from_proto).unwrap_or(Self::Unspecified)
    }

    fn wire_value(self) -> &'static str {
        self.wire()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseRTCUserProfile {
    pub id: String,
    pub tag: String,
    pub name: String,
    pub avatar_url: String,
    pub is_premium: bool,
    pub is_official: bool,
    pub is_online: bool,
    pub last_seen_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseRTCRoomConfig {
    pub room_id: String,
    pub livekit_room_name: String,
    pub chat_id: String,
    pub channel_id: Option<String>,
    #[serde(serialize_with = "encode_flexible_field", deserialize_with = "decode_flexible_field")]
    pub mode: RiseRTCRoomMode,
    pub max_participants: i64,
    pub max_speakers: i64,
    pub video_enabled: bool,
    pub screen_share_enabled: bool,
    pub audio_bitrate_kbps: i64,
    pub region: String,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseRTCParticipant {
    pub user_id: String,
    #[serde(serialize_with = "encode_flexible_field", deserialize_with = "decode_flexible_field")]
    pub role: RiseRTCParticipantRole,
    pub connected: bool,
    pub microphone_enabled: bool,
    pub camera_enabled: bool,
    pub screen_share_enabled: bool,
    pub server_muted: bool,
    pub server_deafened: bool,
    #[serde(default)]
    pub server_camera_blocked: bool,
    #[serde(default)]
    pub server_screen_share_blocked: bool,
    pub deafened: bool,
    pub hand_raised: bool,
    pub joined_at_ms: i64,
    pub updated_at_ms: i64,
    pub lease_expires_at_ms: i64,
    pub order_sequence: i64,
    #[serde(default)]
    pub watching_user_id: String,
    pub profile: Option<RiseRTCUserProfile>,
}

impl RiseRTCParticipant {
    pub fn id(&self) -> &str {
        &self.user_id
    }

    /// silenced by a moderator by either action
    /// a deafen revokes publish grant along with subscription so every surface must read it as a pair
    pub fn is_server_silenced(&self) -> bool {
        self.server_muted || self.server_deafened
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseRTCRaisedHand {
    pub user_id: String,
    pub requested_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub note: Option<String>,
    pub profile: Option<RiseRTCUserProfile>,
}

impl RiseRTCRaisedHand {
    pub fn id(&self) -> &str {
        &self.user_id
    }

    pub fn has_expired(&self, now_ms: i64) -> bool {
        let Some(expires_at_ms) = self.expires_at_ms else {
            return false;
        };
        expires_at_ms <= now_ms
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseRTCDirectCall {
    pub call_id: String,
    pub room_id: String,
    pub chat_id: String,
    pub caller_user_id: String,
    pub callee_user_id: String,
    #[serde(serialize_with = "encode_flexible_field", deserialize_with = "decode_flexible_field")]
    pub media_mode: RiseRTCCallMediaMode,
    #[serde(serialize_with = "encode_flexible_field", deserialize_with = "decode_flexible_field")]
    pub state: RiseRTCCallState,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub expires_at_ms: i64,
    pub answered_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub ended_by_user_id: Option<String>,
    pub end_reason: Option<String>,
    pub caller_e2ee_public_key: Option<String>,
    pub callee_e2ee_public_key: Option<String>,
    #[serde(default)]
    pub recording_state: Option<String>,
    #[serde(default)]
    pub recording_started_by_user_id: Option<String>,
    #[serde(default)]
    pub recording_path: Option<String>,
}

impl RiseRTCDirectCall {
    pub fn id(&self) -> &str {
        &self.call_id
    }

    pub fn is_recording(&self) -> bool {
        self.recording_state.as_deref() == Some(CallRecordingState::Recording.wire())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseRTCIncomingCall {
    pub call: RiseRTCDirectCall,
    pub caller: Option<RiseRTCUserProfile>,
    pub event_id: Option<String>,
    pub occurred_at_ms: i64,
}

impl RiseRTCIncomingCall {
    pub fn id(&self) -> &str {
        &self.call.call_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseRTCRoomSnapshot {
    pub room: RiseRTCRoomConfig,
    pub participant_count: i64,
    pub speaker_count: i64,
    pub participants: Vec<RiseRTCParticipant>,
    pub raised_hands: Vec<RiseRTCRaisedHand>,
    pub generated_at_ms: i64,
    pub profiles_complete: bool,
    pub profile_error: Option<String>,
}

#[derive(Clone)]
pub struct RiseRTCJoinCredentials {
    pub room_id: String,
    pub room_name: String,
    pub participant_identity: String,
    pub connection_id: String,
    pub access_token: String,
    pub server_url: String,
    pub turn_domain: String,
    pub expires_at_ms: i64,
    pub room_mode: RiseRTCRoomMode,
    pub call_media_mode: Option<RiseRTCCallMediaMode>,
    pub audio_bitrate_kbps: Option<i64>,
    pub video_ceiling: RiseRTCVideoCeiling,
}

impl RiseRTCJoinCredentials {
    pub fn uses_call_kit_audio_session(&self) -> bool {
        self.room_mode.uses_call_kit_audio_session()
    }

    pub fn session_key(&self) -> RiseRTCMediaSessionKey {
        RiseRTCMediaSessionKey {
            room_id: self.room_id.clone(),
            connection_id: self.connection_id.clone(),
            participant_identity: self.participant_identity.clone(),
        }
    }
}

impl RiseRTCJoinCredentials {
    pub fn description(&self) -> String {
        let call_media = self.call_media_mode.map_or("none", |mode| mode.wire_value());
        format!(
            "RiseRTCJoinCredentials(roomID: {}, mode: {}, callMedia: {}, secret: <redacted>)",
            self.room_id,
            self.room_mode.wire_value(),
            call_media
        )
    }
}

impl fmt::Display for RiseRTCJoinCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.description())
    }
}

impl fmt::Debug for RiseRTCJoinCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// a join or rejoin rotates both the reservation connection and the identity even if the
/// room id is the same, so media callbacks and mutations fence against all three fields
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseRTCMediaSessionKey {
    pub room_id: String,
    pub connection_id: String,
    pub participant_identity: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseRTCDesiredMediaState {
    pub microphone_enabled: bool,
    pub camera_enabled: bool,
}

impl RiseRTCDesiredMediaState {
    pub const DISABLED: Self = Self { microphone_enabled: false, camera_enabled: false };

    pub fn new(microphone_enabled: bool, camera_enabled: bool) -> Self {
        Self { microphone_enabled, camera_enabled }
    }

    pub fn from_participant(participant: &RiseRTCParticipant) -> Self {
        Self::new(participant.microphone_enabled, participant.camera_enabled)
    }

    pub fn from_call(call: &RiseRTCDirectCall) -> Self {
        Self::new(true, call.media_mode == RiseRTCCallMediaMode::Video)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCAudioProcessingMode {
    VoiceChat,
    VideoChat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseRTCAudioRoutingDecision {
    pub processing_mode: RiseRTCAudioProcessingMode,
    pub prefers_speaker: bool,
}

pub enum RiseRTCAudioRoutingPolicy {}

impl RiseRTCAudioRoutingPolicy {
    pub fn decision(room_mode: RiseRTCRoomMode, call_media_mode: Option<RiseRTCCallMediaMode>) -> RiseRTCAudioRoutingDecision {
        if room_mode == RiseRTCRoomMode::DirectCall && call_media_mode != Some(RiseRTCCallMediaMode::Video) {
            return RiseRTCAudioRoutingDecision { processing_mode: RiseRTCAudioProcessingMode::VoiceChat, prefers_speaker: false };
        }
        RiseRTCAudioRoutingDecision { processing_mode: RiseRTCAudioProcessingMode::VideoChat, prefers_speaker: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCCommittedCallMediaKind {
    OutgoingStart,
    AcceptedIncoming,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCCommittedMediaFailureAction {
    CancelOutgoing,
    EndAcceptedIncoming,
}

pub enum RiseRTCCommittedMediaFailurePolicy {}

impl RiseRTCCommittedMediaFailurePolicy {
    pub fn action(
        kind: RiseRTCCommittedCallMediaKind,
        context_still_owned: bool,
        terminal_action_requested: bool,
        domain_is_shutting_down: bool,
    ) -> Option<RiseRTCCommittedMediaFailureAction> {
        if !context_still_owned || terminal_action_requested || domain_is_shutting_down {
            return None;
        }
        match kind {
            RiseRTCCommittedCallMediaKind::OutgoingStart => Some(RiseRTCCommittedMediaFailureAction::CancelOutgoing),
            RiseRTCCommittedCallMediaKind::AcceptedIncoming => Some(RiseRTCCommittedMediaFailureAction::EndAcceptedIncoming),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCCancelReconciliationAction {
    RetryCancel,
    EndAccepted,
    AlreadyTerminal,
}

pub enum RiseRTCCancelReconciliationPolicy {}

impl RiseRTCCancelReconciliationPolicy {
    pub fn action(authoritative_state: RiseRTCCallState, already_observed_accepted: bool) -> RiseRTCCancelReconciliationAction {
        if authoritative_state.is_terminal() {
            return RiseRTCCancelReconciliationAction::AlreadyTerminal;
        }
        if already_observed_accepted || authoritative_state == RiseRTCCallState::Accepted {
            return RiseRTCCancelReconciliationAction::EndAccepted;
        }
        RiseRTCCancelReconciliationAction::RetryCancel
    }
}

/// how large and how fast this publisher may send camera and screen share
/// numbers come from the server by tier
/// an older server that sends none leaves the free limits
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseRTCVideoCeiling {
    pub max_height: i64,
    pub max_fps: i64,
}

impl RiseRTCVideoCeiling {
    pub const FREE: Self = Self { max_height: VIDEO_PUBLISH_MAX_HEIGHT as i64, max_fps: VIDEO_PUBLISH_MAX_FPS as i64 };

    pub fn new(max_height: i64, max_fps: i64) -> Self {
        Self { max_height, max_fps }
    }

    pub fn from_wire(wire_height: Option<i64>, wire_fps: Option<i64>) -> Self {
        match (wire_height, wire_fps) {
            (Some(wire_height), Some(wire_fps)) if wire_height > 0 && wire_fps > 0 => Self::new(wire_height, wire_fps),
            _ => Self::FREE,
        }
    }

    /// true when a shelf of this height and frame rate fits under the ceiling
    pub fn admits(&self, height: i64, fps: i64) -> bool {
        height <= self.max_height && fps <= self.max_fps
    }

    /// under low power or thermal pressure a publisher above the free frame rate drops to it
    /// resolution stays but the doubled encode cost goes
    pub fn under_pressure(&self, pressured: bool) -> Self {
        if !pressured {
            return *self;
        }
        Self::new(self.max_height, self.max_fps.min(Self::FREE.max_fps))
    }
}

impl Default for RiseRTCVideoCeiling {
    fn default() -> Self {
        Self::FREE
    }
}

pub type RiseRTCCameraPosition = rise_rtc::CameraPosition;
pub type RiseRTCScreenShareQuality = rise_rtc::ScreenShareQuality;

#[derive(Clone, Debug)]
pub enum RiseRTCMediaIntent {
    Connect(RiseRTCJoinCredentials),
    Disconnect {
        session: RiseRTCMediaSessionKey,
    },
    SetMicrophone {
        session: RiseRTCMediaSessionKey,
        enabled: bool,
    },
    SetRemotePlayoutMuted {
        session: RiseRTCMediaSessionKey,
        muted: bool,
    },
    SetListenerVolumes {
        session: RiseRTCMediaSessionKey,
        gains: HashMap<String, f64>,
    },
    SetCamera {
        session: RiseRTCMediaSessionKey,
        enabled: bool,
        position: RiseRTCCameraPosition,
    },
    SetEncryptionKey {
        session: RiseRTCMediaSessionKey,
        key: Option<String>,
    },
    SetScreenShare {
        session: RiseRTCMediaSessionKey,
        enabled: bool,
        include_device_audio: bool,
        quality: RiseRTCScreenShareQuality,
    },
    SwitchCamera {
        session: RiseRTCMediaSessionKey,
        position: RiseRTCCameraPosition,
    },
    SelectAudioInput {
        session: RiseRTCMediaSessionKey,
        id: Option<String>,
    },
    SelectAudioOutput {
        session: RiseRTCMediaSessionKey,
        id: Option<String>,
    },
    SendData {
        session: RiseRTCMediaSessionKey,
        topic: String,
        payload: Vec<u8>,
        reliable: bool,
    },
}

#[derive(Clone, Debug)]
pub enum RiseRTCMediaEvent {
    Connected {
        session: RiseRTCMediaSessionKey,
    },
    Reconnecting {
        session: RiseRTCMediaSessionKey,
    },
    Reconnected {
        session: RiseRTCMediaSessionKey,
    },
    Disconnected {
        session: RiseRTCMediaSessionKey,
        reason: Option<String>,
    },
    ParticipantJoined {
        session: RiseRTCMediaSessionKey,
        participant: RiseRTCParticipant,
    },
    ParticipantLeft {
        session: RiseRTCMediaSessionKey,
        user_id: String,
        occurred_at_ms: i64,
    },
    Data {
        session: RiseRTCMediaSessionKey,
        topic: String,
        payload: Vec<u8>,
    },
    Failure {
        session: Option<RiseRTCMediaSessionKey>,
        code: String,
    },
    LocalScreenShareEnded {
        session: RiseRTCMediaSessionKey,
    },
    LocalMicrophoneLost {
        session: RiseRTCMediaSessionKey,
    },
    LocalPlayoutLost {
        session: RiseRTCMediaSessionKey,
    },
    LocalBroadcastStarted {
        session: RiseRTCMediaSessionKey,
    },
    LocalCameraPaused {
        session: RiseRTCMediaSessionKey,
    },
    LocalCameraResumed {
        session: RiseRTCMediaSessionKey,
    },
    LocalCameraResumeRequested {
        session: RiseRTCMediaSessionKey,
    },
}

impl RiseRTCMediaEvent {
    pub fn session_key(&self) -> Option<&RiseRTCMediaSessionKey> {
        match self {
            Self::Connected { session }
            | Self::Reconnecting { session }
            | Self::Reconnected { session }
            | Self::Disconnected { session, .. }
            | Self::ParticipantJoined { session, .. }
            | Self::ParticipantLeft { session, .. }
            | Self::Data { session, .. }
            | Self::LocalScreenShareEnded { session }
            | Self::LocalMicrophoneLost { session }
            | Self::LocalPlayoutLost { session }
            | Self::LocalBroadcastStarted { session }
            | Self::LocalCameraPaused { session }
            | Self::LocalCameraResumed { session }
            | Self::LocalCameraResumeRequested { session } => Some(session),
            Self::Failure { session, .. } => session.as_ref(),
        }
    }
}

pub trait RiseRTCMediaDriving: Send + Sync {
    fn perform(&self, intent: RiseRTCMediaIntent) -> BoxFuture<'_, anyhow::Result<()>>;
    fn events(&self) -> BoxFuture<'_, UnboundedReceiver<RiseRTCMediaEvent>>;

    fn is_null_media_driver(&self) -> bool {
        false
    }
}

pub type RiseRTCConnectionState = rise_rtc::ConnectionState;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseRTCReaction {
    pub event_id: String,
    pub room_id: String,
    pub actor_user_id: String,
    pub emoji: String,
    pub created_at_ms: i64,
}

impl RiseRTCReaction {
    pub fn id(&self) -> &str {
        &self.event_id
    }
}

/// media failure codes the domain acts on
/// first two are server decisions so no retry
/// the rest are a lost or refused link and the call connects again
pub enum RiseRTCMediaDisconnectCode {}

impl RiseRTCMediaDisconnectCode {
    pub const PARTICIPANT_REMOVED: &'static str = "rtc_livekit_participant_removed";
    pub const ROOM_DELETED: &'static str = "rtc_livekit_room_deleted";
    pub const LINK_LOST: &'static str = "rtc_media_disconnected";
    pub const TIMED_OUT: &'static str = "rtc_livekit_timeout";
    pub const NETWORK_FAILED: &'static str = "rtc_livekit_network_failed";
    pub const CONNECT_FAILED: &'static str = "rtc_livekit_connect_failed";
    pub const JOIN_FAILED: &'static str = "rtc_livekit_join_failed";
    pub const TOKEN_REJECTED: &'static str = "rtc_livekit_token_validation_failed";
    pub const CREDENTIALS_EXPIRED: &'static str = "rtc_join_credentials_expired";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseRTCEvictionReason {
    Removed,
    RoomClosed,
}

impl RiseRTCEvictionReason {
    pub fn from_media_code(media_code: &str) -> Option<Self> {
        match media_code {
            RiseRTCMediaDisconnectCode::PARTICIPANT_REMOVED => Some(Self::Removed),
            RiseRTCMediaDisconnectCode::ROOM_DELETED => Some(Self::RoomClosed),
            _ => None,
        }
    }

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::RoomClosed => "roomClosed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseRTCRoomEviction {
    pub room_id: String,
    pub reason: RiseRTCEvictionReason,
    pub occurred_at_ms: i64,
}

/// result of folding one room control frame, reaction goes to the projection
/// and eviction to the domain which alone may tear down the session
#[derive(Clone, Debug, Default)]
pub struct RiseRTCRoomControlOutcome {
    pub reaction: Option<RiseRTCReaction>,
    pub eviction: Option<RiseRTCRoomEviction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseRTCAnswerOutcome {
    Answered(RiseRTCDirectCall),
    AnsweredElsewhere(Option<RiseRTCDirectCall>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseRTCStateError {
    MissingActiveSession,
    StaleAccount,
    MissingActiveRoom,
    MissingJoinCredentials,
    MalformedResponse(String),
    Validation(String),
    Remote(String),
    AnsweredElsewhere,
}

impl fmt::Display for RiseRTCStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for RiseRTCStateError {}
