use serde::{Deserialize, Serialize};

use crate::modules::chat::engine::chats::rise_chat_repository_models::RiseChatTombstone;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseChatListKind;
use crate::modules::chat::engine::messages::rise_message_upload_retry::RiseMessageUploadRetry;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiseConversationEntitySource {
    Migration,
    ServerPage,
    Realtime,
    Optimistic,
    Acknowledgement,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseChatEntityRecord {
    pub wire: RiseStoredChat,
    pub source: RiseConversationEntitySource,
    pub locally_created: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_kind: Option<RiseChatListKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tombstone: Option<RiseChatTombstone>,
}

impl RiseChatEntityRecord {
    /// defaults for everything else, override the public fields if you need other values
    pub fn new(wire: RiseStoredChat, source: RiseConversationEntitySource) -> Self {
        Self { wire, source, locally_created: false, list_kind: None, tombstone: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiseMessageLocalPhase {
    Remote,
    Queued,
    Uploading,
    Sending,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseMessageEntityRecord {
    pub wire: MessageWireDTO,
    pub source: RiseConversationEntitySource,
    pub local_phase: RiseMessageLocalPhase,
    #[serde(rename = "clientRequestID", default, skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_storage_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_replayable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_retry: Option<RiseMessageUploadRetry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_retry: Option<RiseMessageForwardRetry>,
}

impl RiseMessageEntityRecord {
    /// defaults for everything else, override the public fields if you need other values
    pub fn new(wire: MessageWireDTO, source: RiseConversationEntitySource) -> Self {
        Self {
            wire,
            source,
            local_phase: RiseMessageLocalPhase::Remote,
            client_request_id: None,
            last_error_code: None,
            scope_storage_key: None,
            is_replayable: None,
            upload_retry: None,
            forward_retry: None,
        }
    }
}

/// the forward a failed copy came from
/// server binds the idempotency key to source ids and caption
/// it takes any subset of destinations so both are kept whole
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseMessageForwardRetry {
    pub idempotency_key: String,
    pub message_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}
