use std::fmt;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_rtc::{RoomId, call_room_id, channel_room_id, group_room_id};

use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::modules::rtc::shared::rtc_guide_sandbox_hooks::RtcGuideSandboxHooks;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCRoomLocatorError {
    InvalidChatID,
    InvalidTopicID,
    InvalidCallID,
}

impl fmt::Display for RiseRTCRoomLocatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for RiseRTCRoomLocatorError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseRTCRoomLocatorTarget {
    Topic { chat_id: String, topic_id: String },
    Group { chat_id: String },
    DirectCall { call_id: String },
}

pub enum RiseRTCRoomLocator {}

impl RiseRTCRoomLocator {
    pub fn room_id(target: &RiseRTCRoomLocatorTarget) -> Result<String, RiseRTCRoomLocatorError> {
        match target {
            RiseRTCRoomLocatorTarget::Topic { chat_id, topic_id } => Self::topic_room_id(chat_id, topic_id),
            RiseRTCRoomLocatorTarget::Group { chat_id } => Self::group_room_id(chat_id),
            RiseRTCRoomLocatorTarget::DirectCall { call_id } => Self::direct_call_room_id(call_id),
        }
    }

    pub fn topic_room_id(chat_id: &str, topic_id: &str) -> Result<String, RiseRTCRoomLocatorError> {
        let normalized_chat_id = Self::normalized_uuid(chat_id, RiseRTCRoomLocatorError::InvalidChatID)?;
        let normalized_topic_id = Self::normalized_uuid(topic_id, RiseRTCRoomLocatorError::InvalidTopicID)?;
        Ok(channel_room_id(&normalized_chat_id, &normalized_topic_id))
    }

    pub fn group_room_id(chat_id: &str) -> Result<String, RiseRTCRoomLocatorError> {
        let normalized_chat_id = Self::normalized_uuid(chat_id, RiseRTCRoomLocatorError::InvalidChatID)?;
        Ok(group_room_id(&normalized_chat_id))
    }

    pub fn direct_call_room_id(call_id: &str) -> Result<String, RiseRTCRoomLocatorError> {
        let normalized_call_id = Self::normalized_uuid(call_id, RiseRTCRoomLocatorError::InvalidCallID)?;
        Ok(call_room_id(&normalized_call_id))
    }

    /// inverse of rtc:call:uuid, the media layer needs the call id
    /// to ask the system layer which call owns the audio session
    pub fn direct_call_id(from_room_id: &str) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(from_room_id);
        let Some(RoomId::Call { call }) = RoomId::parse(value) else {
            return None;
        };
        let uuid = Uuid::from_uuid_string(call)?;
        Some(uuid.uuid_string().to_lowercase())
    }

    fn normalized_uuid(raw_value: &str, error: RiseRTCRoomLocatorError) -> Result<String, RiseRTCRoomLocatorError> {
        let value = trimming_whitespaces_and_newlines(raw_value);
        let Some(uuid) = Uuid::from_uuid_string(value) else {
            return Err(error);
        };
        Ok(uuid.uuid_string().to_lowercase())
    }
}

pub trait RiseRTCRoomTargetRoomID {
    /// one room address for every surface so screen, sheet and widget match
    /// server events against the same value and dont each parse the target
    fn room_id(&self) -> Option<String>;
}

impl RiseRTCRoomTargetRoomID for RiseRTCRoomTarget {
    fn room_id(&self) -> Option<String> {
        match self {
            Self::Topic { chat_id, topic_id } => {
                if RtcGuideSandboxHooks::forum_chat_id() == Some(chat_id.as_str()) {
                    return RtcGuideSandboxHooks::voice_room_id(topic_id);
                }
                RiseRTCRoomLocator::topic_room_id(chat_id, topic_id).ok()
            }
            Self::Group { chat_id } => RiseRTCRoomLocator::group_room_id(chat_id).ok(),
            Self::DirectCall { call_id } => RiseRTCRoomLocator::direct_call_room_id(call_id).ok(),
        }
    }
}
