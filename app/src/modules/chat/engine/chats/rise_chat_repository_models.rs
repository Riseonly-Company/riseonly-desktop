use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

use futures::future::BoxFuture;
use rise_canon::rise_key;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_store::rise_store_entities::{RiseEntityViewPosition, RiseStoreEntity};
use rise_engine::rise_store::rise_store_models::RiseStoreWriteBatch;
use rise_engine::{RiseJSONValue, RiseWirePushEnvelope};
use serde::{Deserialize, Serialize};

use crate::modules::chat::engine::core::rise_conversation_entity_records::RiseChatEntityRecord;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseChatListKind;
use crate::modules::chat::engine::core::rise_conversation_store_layout::RiseConversationStoreLayout;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatMuteScopes;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    RiseStoredChat, RiseStoredChatPreview, RiseStoredChatPreviewMedia,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseChatListAggregates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_unread_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread_chats_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_unread_media_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_unread_replies_count: Option<i64>,
}

impl RiseChatListAggregates {
    pub fn new(
        total_count: Option<i64>,
        total_unread_count: Option<i64>,
        unread_chats_count: Option<i64>,
        total_unread_media_count: Option<i64>,
        total_unread_replies_count: Option<i64>,
    ) -> Self {
        Self {
            total_count: total_count.map(|value| value.max(0)),
            total_unread_count: total_unread_count.map(|value| value.max(0)),
            unread_chats_count: unread_chats_count.map(|value| value.max(0)),
            total_unread_media_count: total_unread_media_count.map(|value| value.max(0)),
            total_unread_replies_count: total_unread_replies_count.map(|value| value.max(0)),
        }
    }

    pub fn applying(&self, delta: &RiseChatAggregateDelta) -> Self {
        self.applying_floors(delta, 0, 0, 0)
    }

    pub fn applying_floors(
        &self,
        delta: &RiseChatAggregateDelta,
        loaded_count_floor: i64,
        loaded_unread_floor: i64,
        loaded_unread_chats_floor: i64,
    ) -> Self {
        Self::new(
            Some(loaded_count_floor.max(self.total_count.unwrap_or(loaded_count_floor) + delta.total)),
            Some(loaded_unread_floor.max(self.total_unread_count.unwrap_or(loaded_unread_floor) + delta.unread)),
            Some(
                loaded_unread_chats_floor
                    .max(self.unread_chats_count.unwrap_or(loaded_unread_chats_floor) + delta.unread_chats),
            ),
            Some((self.total_unread_media_count.unwrap_or(0) + delta.unread_media).max(0)),
            self.total_unread_replies_count,
        )
    }
}

/// one local change to a list wide aggregate
/// inbox badge ignores total and only sees chats with notifications on
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RiseChatAggregateDelta {
    pub total: i64,
    pub unread: i64,
    pub unread_chats: i64,
    pub unread_media: i64,
}

impl RiseChatAggregateDelta {
    pub const fn zero() -> RiseChatAggregateDelta {
        RiseChatAggregateDelta { total: 0, unread: 0, unread_chats: 0, unread_media: 0 }
    }

    pub fn is_zero(&self) -> bool {
        *self == Self::zero()
    }

    pub fn negated(&self) -> RiseChatAggregateDelta {
        RiseChatAggregateDelta {
            total: -self.total,
            unread: -self.unread,
            unread_chats: -self.unread_chats,
            unread_media: -self.unread_media,
        }
    }

    pub fn row(wire: &RiseStoredChat, sign: i64, counts_total: bool) -> RiseChatAggregateDelta {
        let unread = wire.resolved_unread_count();
        RiseChatAggregateDelta {
            total: if counts_total { sign } else { 0 },
            unread: sign * unread,
            unread_chats: if unread > 0 { sign } else { 0 },
            unread_media: sign * wire.resolved_unread_media_count(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiseChatTombstone {
    PendingLeave,
    PendingDelete,
    ConfirmedLeft,
    ConfirmedDeleted,
}

impl RiseChatTombstone {
    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::ConfirmedLeft | Self::ConfirmedDeleted)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseConversationTopicPresence {
    Absent,
    Null,
    Value(String),
}

impl RiseConversationTopicPresence {
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Value(value) => Some(value.as_str()),
            _ => None,
        }
    }
}

/// raw values must match the canon wire names, a parity check enforces it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseChatRealtimeMessageKind {
    Created,
    Edited,
    Deleted,
}

impl RiseChatRealtimeMessageKind {
    pub fn all_cases() -> &'static [Self] {
        &[Self::Created, Self::Edited, Self::Deleted]
    }

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Created => rise_events::name::MESSAGE_CREATED,
            Self::Edited => rise_events::name::MESSAGE_EDITED,
            Self::Deleted => rise_events::name::MESSAGE_DELETED,
        }
    }
}

#[derive(Clone)]
pub struct RiseChatRealtimeMessageContext {
    pub kind: RiseChatRealtimeMessageKind,
    pub envelope: RiseWirePushEnvelope,
    pub chat_id: String,
    pub message_ids: Vec<String>,
    pub message: Option<MessageWireDTO>,
    pub topic: RiseConversationTopicPresence,
    pub version: i64,
}

pub type RiseChatMessageBatchHook = Arc<
    dyn Fn(RiseChatRealtimeMessageContext) -> BoxFuture<'static, anyhow::Result<RiseStoreWriteBatch>> + Send + Sync,
>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseChatResourceInvalidation {
    Topics {
        chat_id: String,
    },
    Forum {
        chat_id: String,
    },
    Roles {
        chat_id: String,
    },
    Members {
        chat_id: String,
    },
    MemberRole {
        chat_id: String,
        user_id: String,
        role: Option<String>,
        role_ids: Option<Vec<String>>,
        display_color: Option<i64>,
    },
    Settings {
        chat_id: String,
    },
    Roster {
        chat_id: String,
        user_id: String,
        joined: bool,
    },
    Preview {
        chat_id: String,
        topic_id: Option<String>,
        reread: bool,
    },
    RemovedByAdmin {
        chat_id: String,
    },
    TopicDeleted {
        chat_id: String,
        topic_id: String,
        deleted_by: Option<String>,
    },
}

pub type RiseChatResourceInvalidationHook =
    Arc<dyn Fn(HashSet<RiseChatResourceInvalidation>) -> BoxFuture<'static, ()> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseChatNotificationPreference {
    pub messages: bool,
    pub mentions: bool,
    pub replies: bool,
    pub reactions: bool,
    pub events: bool,
}

impl RiseChatNotificationPreference {
    /// replies none follows mentions, like a row cached before the replies scope
    pub fn new(messages: bool, mentions: bool, replies: Option<bool>, reactions: bool, events: bool) -> Self {
        Self { messages, mentions, replies: replies.unwrap_or(mentions), reactions, events }
    }

    pub fn from_scopes(scopes: &ChatMuteScopes) -> Self {
        Self::new(scopes.messages, scopes.mentions, Some(scopes.replies), scopes.reactions, scopes.events)
    }

    pub fn is_all_enabled(&self) -> bool {
        self.messages && self.mentions && self.replies && self.reactions && self.events
    }

    pub fn is_fully_muted(&self) -> bool {
        !(self.messages || self.mentions || self.replies || self.reactions || self.events)
    }

    pub fn encoded_json(&self) -> Option<String> {
        ChatMuteScopes {
            messages: self.messages,
            mentions: self.mentions,
            replies: self.replies,
            reactions: self.reactions,
            events: self.events,
        }
        .encoded_json()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RiseChatOptimisticOperation {
    Archive,
    Unarchive,
    Delete,
    Leave,
    Pin(bool),
    ReorderPinned {
        position: Option<i64>,
    },
    Mute {
        is_muted: bool,
        until: Option<f64>,
    },
    Notifications(RiseChatNotificationPreference),
    Patch(BTreeMap<String, RiseJSONValue>),
    Read,
    ReadTopic {
        clearing: i64,
    },
}

impl RiseChatOptimisticOperation {
    pub fn is_read(&self) -> bool {
        matches!(self, Self::Read | Self::ReadTopic { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseChatOptimisticToken {
    pub id: Uuid,
    pub chat_local_id: String,
    pub applied_version: i64,
}

impl RiseChatOptimisticToken {
    pub fn new(id: Uuid, chat_local_id: String, applied_version: i64) -> Self {
        Self { id, chat_local_id, applied_version }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseChatRepositoryError {
    ChatNotFound(String),
    ChatIsTombstoned(String),
    OptimisticMutationAlreadyPending(String),
    InvalidPush(String),
    CorruptedChat(String),
    Rejected(String),
}

impl RiseChatRepositoryError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            Self::Rejected(message) => {
                let trimmed = trimming_whitespaces_and_newlines(message);
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            }
            _ => None,
        }
    }
}

impl fmt::Display for RiseChatRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for RiseChatRepositoryError {}

#[derive(Clone)]
pub struct RiseChatStoredEntity {
    pub entity: RiseStoreEntity,
    pub record: RiseChatEntityRecord,
}

#[derive(Clone)]
pub struct RiseChatPendingMutation {
    pub token: RiseChatOptimisticToken,
    pub operation: RiseChatOptimisticOperation,
    pub previous: RiseChatStoredEntity,
    pub previous_kind: Option<RiseChatListKind>,
    pub previous_position: Option<RiseEntityViewPosition>,
    pub previous_membership_version: Option<i64>,
    pub cleared_unread_count: i64,
    pub cleared_unread_media_count: i64,
}

pub enum RiseChatWireCodec {}

impl RiseChatWireCodec {
    /// keys sit in a BTreeMap and slashes are not escaped, so output is stable and sorted
    pub fn encode_record(record: &RiseChatEntityRecord) -> anyhow::Result<Vec<u8>> {
        Ok(RiseJSONValue::encode(record)?.data(true)?)
    }

    pub fn decode_record(data: &[u8]) -> anyhow::Result<RiseChatEntityRecord> {
        Ok(serde_json::from_slice(data)?)
    }

    pub fn object(wire: &RiseStoredChat) -> anyhow::Result<BTreeMap<String, RiseJSONValue>> {
        match RiseJSONValue::encode(wire)? {
            RiseJSONValue::Object(object) => Ok(object),
            _ => Err(RiseChatRepositoryError::CorruptedChat(wire.id.clone()).into()),
        }
    }

    /// reads either key spelling, snake or camel
    pub fn decode_wire(object: BTreeMap<String, RiseJSONValue>) -> anyhow::Result<RiseStoredChat> {
        Ok(RiseJSONValue::Object(object).decode_as::<RiseStoredChat>()?)
    }

    pub fn minimal_wire(chat_id: &str) -> anyhow::Result<RiseStoredChat> {
        Self::decode_wire(BTreeMap::from([(
            rise_key::chat::ID.to_owned(),
            RiseJSONValue::String(chat_id.to_owned()),
        )]))
    }

    pub fn patch(
        wire: &RiseStoredChat,
        updates: BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<RiseStoredChat> {
        let mut object = Self::object(wire)?;
        object.extend(Self::normalized_object(&updates));
        Self::decode_wire(object)
    }

    pub fn overlay(
        wire: &RiseStoredChat,
        incoming: &BTreeMap<String, RiseJSONValue>,
        excluding: &HashSet<String>,
    ) -> anyhow::Result<RiseStoredChat> {
        let mut object = Self::object(wire)?;
        for (key, value) in Self::normalized_object(incoming) {
            if !excluding.contains(&key) {
                object.insert(key, value);
            }
        }
        Self::decode_wire(object)
    }

    pub fn normalized_object(object: &BTreeMap<String, RiseJSONValue>) -> BTreeMap<String, RiseJSONValue> {
        object.iter().map(|(key, value)| (Self::camel_case(key), Self::normalized(value))).collect()
    }

    pub fn last_message(message: &MessageWireDTO, topic: &RiseConversationTopicPresence) -> RiseStoredChatPreview {
        let resolved_topic = match topic {
            RiseConversationTopicPresence::Absent => message.topic_id.clone(),
            RiseConversationTopicPresence::Null => None,
            RiseConversationTopicPresence::Value(value) => Some(value.clone()),
        };
        let media = match message.media_items.as_ref().filter(|items| !items.is_empty()) {
            Some(items) => items.clone(),
            None => message.media_info.iter().cloned().collect(),
        };
        RiseStoredChatPreview {
            id: message.server_id.clone().or_else(|| message.id.clone()),
            chat_id: message.chat_id.clone(),
            topic_id: resolved_topic,
            sender_id: message
                .sender_id
                .clone()
                .or_else(|| message.sender.as_ref().and_then(|sender| sender.id.clone())),
            sender_name: message
                .sender_name
                .clone()
                .or_else(|| message.sender.as_ref().and_then(|sender| sender.name.clone())),
            content: message.content.clone(),
            caption: message.caption.clone(),
            content_type: message.content_type.clone(),
            timestamp: message.timestamp,
            created_at: message.created_at,
            has_attachments: message.has_attachments,
            has_reactions: message.has_reactions,
            is_pinned: message.is_pinned,
            is_read: message.is_read,
            is_system_message: message.is_system_message,
            system_message_type: message.system_message_type.clone(),
            metadata: message.metadata.clone(),
            media_items: RiseStoredChatPreviewMedia::make(media),
            system_target: message.system_target.clone().filter(|target| !target.is_empty()),
            author_signature: message.author_signature.clone(),
            ..RiseStoredChatPreview::default()
        }
    }

    pub fn last_message_value(value: Option<&RiseStoredChatPreview>) -> anyhow::Result<RiseJSONValue> {
        match value {
            Some(value) => Ok(RiseJSONValue::encode(value)?),
            None => Ok(RiseJSONValue::Null),
        }
    }

    pub fn timestamp(message: &MessageWireDTO) -> f64 {
        RiseConversationStoreLayout::normalized_timestamp(message.created_at.or(message.timestamp).unwrap_or(0.0))
    }

    fn normalized(value: &RiseJSONValue) -> RiseJSONValue {
        match value {
            RiseJSONValue::Object(object) => RiseJSONValue::Object(Self::normalized_object(object)),
            RiseJSONValue::Array(values) => RiseJSONValue::Array(values.iter().map(Self::normalized).collect()),
            _ => value.clone(),
        }
    }

    /// the key a normalized payload carries for a canon wire key
    pub fn normalized_key(wire: &str) -> String {
        Self::camel_case(wire)
    }

    fn camel_case(key: &str) -> String {
        if !key.contains('_') {
            return key.to_owned();
        }
        let mut components = key.split('_').filter(|component| !component.is_empty());
        let Some(first) = components.next() else {
            return key.to_owned();
        };
        let mut result = first.to_owned();
        for component in components {
            let mut chars = component.chars();
            if let Some(head) = chars.next() {
                result.extend(head.to_uppercase());
                result.push_str(chars.as_str());
            }
        }
        result
    }
}

/// write batch helpers for the conversation engine
pub trait RiseStoreWriteBatchConversationExt {
    fn conversation_merging(batches: Vec<RiseStoreWriteBatch>) -> RiseStoreWriteBatch;
}

impl RiseStoreWriteBatchConversationExt for RiseStoreWriteBatch {
    fn conversation_merging(batches: Vec<RiseStoreWriteBatch>) -> RiseStoreWriteBatch {
        let mut mutations = Vec::new();
        let mut invalidated_views = HashSet::new();
        for batch in batches {
            mutations.extend(batch.mutations);
            invalidated_views.extend(batch.invalidated_views);
        }
        RiseStoreWriteBatch::new(mutations, invalidated_views)
    }
}
