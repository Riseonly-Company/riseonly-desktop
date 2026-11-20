use rise_canon::rise_proto::rtc::{JoinCredentials, RaisedHand, RtcUserProfile};
use rise_canon::rise_view::rtc::{Call, Participant, Room, RoomSnapshot};

use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCCallMediaMode, RiseRTCCallState, RiseRTCDirectCall, RiseRTCFlexibleWireEnum, RiseRTCJoinCredentials,
    RiseRTCParticipant, RiseRTCParticipantRole, RiseRTCRaisedHand, RiseRTCRoomConfig, RiseRTCRoomMode, RiseRTCRoomSnapshot,
    RiseRTCUserProfile, RiseRTCVideoCeiling,
};

/// rtc replies are generated canon types, these converters map them into domain models
/// rtc enums come as wire strings and parse with the same tolerant rules as the others
pub trait RiseRTCWireModel {
    type Model;

    fn model(&self) -> Self::Model;
}

impl RiseRTCWireModel for RtcUserProfile {
    type Model = RiseRTCUserProfile;

    fn model(&self) -> RiseRTCUserProfile {
        RiseRTCUserProfile {
            id: self.id.clone(),
            tag: self.tag.clone(),
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
            is_premium: self.is_premium,
            is_official: self.is_official,
            is_online: self.is_online,
            last_seen_ms: self.last_seen_ms,
        }
    }
}

impl RiseRTCWireModel for Room {
    type Model = RiseRTCRoomConfig;

    fn model(&self) -> RiseRTCRoomConfig {
        RiseRTCRoomConfig {
            room_id: self.room_id.clone(),
            livekit_room_name: self.livekit_room_name.clone(),
            chat_id: self.chat_id.clone(),
            channel_id: self.channel_id.clone(),
            mode: RiseRTCRoomMode::parse(&self.mode),
            max_participants: self.max_participants,
            max_speakers: self.max_speakers,
            video_enabled: self.video_enabled,
            screen_share_enabled: self.screen_share_enabled,
            audio_bitrate_kbps: self.audio_bitrate_kbps,
            region: self.region.clone(),
            revision: self.revision,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        }
    }
}

impl RiseRTCWireModel for Participant {
    type Model = RiseRTCParticipant;

    fn model(&self) -> RiseRTCParticipant {
        RiseRTCParticipant {
            user_id: self.user_id.clone(),
            role: RiseRTCParticipantRole::parse(&self.role),
            connected: self.connected,
            microphone_enabled: self.microphone_enabled,
            camera_enabled: self.camera_enabled,
            screen_share_enabled: self.screen_share_enabled,
            server_muted: self.server_muted,
            server_deafened: self.server_deafened,
            server_camera_blocked: self.server_camera_blocked,
            server_screen_share_blocked: self.server_screen_share_blocked,
            deafened: self.deafened,
            hand_raised: self.hand_raised,
            joined_at_ms: self.joined_at_ms,
            updated_at_ms: self.updated_at_ms,
            lease_expires_at_ms: self.lease_expires_at_ms,
            order_sequence: self.order_sequence,
            watching_user_id: self.watching_user_id.clone(),
            profile: self.profile.as_ref().map(RiseRTCWireModel::model),
        }
    }
}

impl RiseRTCWireModel for RaisedHand {
    type Model = RiseRTCRaisedHand;

    fn model(&self) -> RiseRTCRaisedHand {
        RiseRTCRaisedHand {
            user_id: self.user_id.clone(),
            requested_at_ms: self.requested_at_ms,
            expires_at_ms: (self.expires_at_ms > 0).then_some(self.expires_at_ms),
            note: self.note.clone(),
            profile: self.profile.as_ref().map(RiseRTCWireModel::model),
        }
    }
}

impl RiseRTCWireModel for RoomSnapshot {
    type Model = Option<RiseRTCRoomSnapshot>;

    fn model(&self) -> Option<RiseRTCRoomSnapshot> {
        let room = self.room.as_ref()?;
        Some(RiseRTCRoomSnapshot {
            room: room.model(),
            participant_count: self.participant_count,
            speaker_count: self.speaker_count,
            participants: self.participants.iter().map(RiseRTCWireModel::model).collect(),
            raised_hands: self.raised_hands.iter().map(RiseRTCWireModel::model).collect(),
            generated_at_ms: self.generated_at_ms,
            profiles_complete: self.profiles_complete,
            profile_error: self.profile_error.clone(),
        })
    }
}

impl RiseRTCWireModel for Call {
    type Model = RiseRTCDirectCall;

    fn model(&self) -> RiseRTCDirectCall {
        RiseRTCDirectCall {
            call_id: self.call_id.clone(),
            room_id: self.room_id.clone(),
            chat_id: self.chat_id.clone(),
            caller_user_id: self.caller_user_id.clone(),
            callee_user_id: self.callee_user_id.clone(),
            media_mode: RiseRTCCallMediaMode::parse(&self.media_mode),
            state: RiseRTCCallState::parse(&self.state),
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            expires_at_ms: self.expires_at_ms,
            answered_at_ms: self.answered_at_ms,
            ended_at_ms: self.ended_at_ms,
            ended_by_user_id: self.ended_by_user_id.clone(),
            end_reason: self.end_reason.clone(),
            caller_e2ee_public_key: self.caller_e2ee_public_key.clone(),
            callee_e2ee_public_key: self.callee_e2ee_public_key.clone(),
            recording_state: None,
            recording_started_by_user_id: None,
            recording_path: None,
        }
    }
}

pub trait RiseRTCJoinCredentialsWire {
    fn credentials(
        &self,
        room_id: &str,
        room_name: &str,
        mode: RiseRTCRoomMode,
        call_media_mode: Option<RiseRTCCallMediaMode>,
        audio_bitrate_kbps: Option<i64>,
    ) -> RiseRTCJoinCredentials;
}

impl RiseRTCJoinCredentialsWire for JoinCredentials {
    fn credentials(
        &self,
        room_id: &str,
        room_name: &str,
        mode: RiseRTCRoomMode,
        call_media_mode: Option<RiseRTCCallMediaMode>,
        audio_bitrate_kbps: Option<i64>,
    ) -> RiseRTCJoinCredentials {
        RiseRTCJoinCredentials {
            room_id: room_id.to_owned(),
            room_name: room_name.to_owned(),
            participant_identity: self.participant_identity.clone(),
            connection_id: self.connection_id.clone(),
            access_token: self.access_token.clone(),
            server_url: self.server_url.clone(),
            turn_domain: self.turn_domain.clone(),
            expires_at_ms: self.expires_at_ms,
            room_mode: mode,
            call_media_mode,
            audio_bitrate_kbps,
            video_ceiling: RiseRTCVideoCeiling::from_wire(Some(self.video_max_height), Some(self.video_max_fps)),
        }
    }
}
