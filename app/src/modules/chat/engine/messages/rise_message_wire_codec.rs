use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap, HashSet};

use rise_canon::RiseWireKey;
use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_key::events as events_key;
use rise_canon::rise_key::message as message_key;
use rise_canon::rise_key::payload as payload_key;
use rise_chat::LinkPreviewSize;
use rise_core::foundation::foundation_character::{double_from_string, int_from_string};
use rise_core::foundation::foundation_compare::localized_case_insensitive_contains;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::{RiseJSONValue, RiseWirePushEnvelope};
use serde::Serialize;

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::{
    SaiFetchJsonEncoder, SaiFetchKeyEncodingStrategy,
};
use crate::modules::chat::components::chat::composer::chat_link_preview_draft::ChatLinkPreviewURL;
use crate::modules::chat::engine::core::rise_conversation_entity_records::RiseMessageEntityRecord;
use crate::modules::chat::engine::messages::rise_message_contracts::RiseMessageRepositoryError;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CreateMessageGatewayDTO, MessageReactionSummaryWireDTO, MessageWireDTO, RiseStoredOutgoingMessage,
};

type RiseJSONObject = BTreeMap<String, RiseJSONValue>;

const MESSAGE_INFO: &str = "message_info";
const RISE_TOMBSTONE: &str = "rise_tombstone";
const RISE_DELETED_AT_MS: &str = "rise_deleted_at_ms";
const RISE_TOMBSTONE_PROBE: &[u8] = b"\"rise_tombstone\":true";

pub enum RiseMessageWireCodec {}

impl RiseMessageWireCodec {
    pub fn reaction_limit(is_premium: bool) -> i64 {
        rise_limits::signed(rise_limits::limit::reactions_per_message(is_premium))
    }

    pub fn push_object(push: &RiseWirePushEnvelope) -> RiseJSONObject {
        let empty = RiseJSONObject::new();
        let root = push.data.object_value().unwrap_or(&empty);
        let camel_message_info = RiseWireKey::foundation_camel(MESSAGE_INFO);
        let camel_server_id = RiseWireKey::foundation_camel(RiseLocalKey::SERVER_ID);
        let mut object = root
            .get(bot_key::MESSAGE)
            .and_then(RiseJSONValue::object_value)
            .or_else(|| root.get(MESSAGE_INFO).and_then(RiseJSONValue::object_value))
            .or_else(|| root.get(&camel_message_info).and_then(RiseJSONValue::object_value))
            .unwrap_or(root)
            .clone();
        if !object.contains_key(bot_key::ID)
            && let Some(id) = Self::value(&object, &[message_key::MESSAGE_ID])
                .or_else(|| Self::value(root, &[message_key::MESSAGE_ID, bot_key::ID]))
                .cloned()
        {
            object.insert(bot_key::ID.to_owned(), id);
        }
        if !object.contains_key(bot_key::CHAT_ID)
            && let Some(chat_id) = Self::value(root, &[message_key::CHAT_ID]).cloned()
        {
            object.insert(bot_key::CHAT_ID.to_owned(), chat_id);
        }
        if !object.contains_key(bot_key::SENDER)
            && let Some(sender) = Self::value(&object, &[events_key::SENDER_INFO])
                .or_else(|| Self::value(root, &[events_key::SENDER_INFO, bot_key::SENDER]))
                .cloned()
        {
            object.insert(bot_key::SENDER.to_owned(), sender);
        }
        if !object.contains_key(bot_key::TOPIC_ID)
            && let Some(topic_id) = Self::value(root, &[message_key::TOPIC_ID]).cloned()
        {
            object.insert(bot_key::TOPIC_ID.to_owned(), topic_id);
        }
        if !object.contains_key(bot_key::TEMP_ID)
            && let Some(temp_id) = Self::value(root, &[message_key::TEMP_ID]).cloned()
        {
            object.insert(bot_key::TEMP_ID.to_owned(), temp_id);
        }
        if !object.contains_key(RiseLocalKey::SERVER_ID)
            && let Some(server_id) = Self::value(root, &[RiseLocalKey::SERVER_ID, camel_server_id.as_str()]).cloned()
        {
            object.insert(RiseLocalKey::SERVER_ID.to_owned(), server_id);
        }
        object
    }

    /// takes the object by value or reference
    /// the decoder reads either key spelling
    pub fn wire(object: impl Borrow<RiseJSONObject>) -> anyhow::Result<MessageWireDTO> {
        let object = RiseJSONValue::Object(Self::entities_as_string(object.borrow()));
        Ok(object.decode_as::<MessageWireDTO>()?)
    }

    fn entities_as_string(object: &RiseJSONObject) -> RiseJSONObject {
        let mut normalized = object.clone();
        if let Some(raw) = object.get(bot_key::ENTITIES)
            && matches!(raw, RiseJSONValue::Array(_) | RiseJSONValue::Object(_))
        {
            let text = raw
                .data(true)
                .ok()
                .and_then(|data| String::from_utf8(data).ok())
                .map_or(RiseJSONValue::Null, RiseJSONValue::String);
            normalized.insert(bot_key::ENTITIES.to_owned(), text);
        }
        normalized
    }

    pub fn object(wire: &MessageWireDTO) -> anyhow::Result<RiseJSONObject> {
        match Self::snake_cased(wire)? {
            RiseJSONValue::Object(object) => Ok(object),
            _ => Err(RiseMessageRepositoryError::InvalidWirePayload.into()),
        }
    }

    /// encodes with snake case keys into a json value
    fn snake_cased<Value: Serialize>(value: &Value) -> anyhow::Result<RiseJSONValue> {
        let encoder = SaiFetchJsonEncoder { key_encoding_strategy: SaiFetchKeyEncodingStrategy::ConvertToSnakeCase };
        Ok(RiseJSONValue::from(encoder.encode(value)?))
    }

    pub fn record_data(record: &RiseMessageEntityRecord) -> anyhow::Result<Vec<u8>> {
        Ok(RiseJSONValue::encode(record)?.data(true)?)
    }

    pub fn record(data: &[u8]) -> anyhow::Result<RiseMessageEntityRecord> {
        Ok(serde_json::from_slice(data)?)
    }

    pub fn tombstone_data(record: &RiseMessageEntityRecord, deleted_at_milliseconds: i64) -> anyhow::Result<Vec<u8>> {
        let mut object = match RiseJSONValue::encode(record)? {
            RiseJSONValue::Object(object) => object,
            _ => RiseJSONObject::new(),
        };
        object.insert(RISE_TOMBSTONE.to_owned(), RiseJSONValue::Bool(true));
        object.insert(RISE_DELETED_AT_MS.to_owned(), RiseJSONValue::Integer(deleted_at_milliseconds));
        Ok(RiseJSONValue::Object(object).data(true)?)
    }

    pub fn is_tombstone(data: &[u8]) -> bool {
        data.windows(RISE_TOMBSTONE_PROBE.len()).any(|window| window == RISE_TOMBSTONE_PROBE)
    }

    pub fn optimistic_wire(
        body: &RiseStoredOutgoingMessage,
        scope: &RiseMessageScope,
        local_id: &str,
        timestamp: f64,
        overlay: Option<&MessageWireDTO>,
        replying_to: Option<&MessageWireDTO>,
        sender_id: Option<&str>,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = match Self::snake_cased(body)? {
            RiseJSONValue::Object(object) => object,
            _ => RiseJSONObject::new(),
        };
        if let Some(overlay) = overlay {
            object.extend(Self::object(overlay)?);
        }
        for (request_key, row_key) in [
            (message_key::SHARED_POST_INFO, payload_key::SHARED_POST),
            (message_key::STORY_REPLY_INFO, message_key::STORY_REPLY),
            (message_key::REPLY_MARKUP, message_key::REPLY_MARKUP),
            (message_key::WEB_APP_DATA, message_key::WEB_APP_DATA),
        ] {
            if object.get(row_key).and_then(RiseJSONValue::object_value).is_some() {
                continue;
            }
            if let Some(RiseJSONValue::String(raw)) = object.get(request_key)
                && let Ok(RiseJSONValue::Object(mut value)) = RiseJSONValue::decode(raw.as_bytes())
            {
                if row_key == message_key::WEB_APP_DATA {
                    value.remove(bot_key::INIT_DATA);
                }
                object.insert(row_key.to_owned(), RiseJSONValue::Object(value));
            }
        }
        if Self::string(&object, &[bot_key::SENDER_ID]).is_none()
            && let Some(sender_id) = sender_id
        {
            object.insert(bot_key::SENDER_ID.to_owned(), RiseJSONValue::String(sender_id.to_owned()));
        }
        if let Some(reply_id) = Self::normalized(body.reply_to_id.as_deref()) {
            let mut reply =
                object.get(bot_key::REPLY_TO).and_then(RiseJSONValue::object_value).cloned().unwrap_or_default();
            if let Some(source) = replying_to {
                let source_object = Self::object(source)?;
                for key in [
                    message_key::SENDER_ID,
                    bot_key::CONTENT,
                    message_key::CONTENT_TYPE,
                    message_key::CREATED_AT,
                    bot_key::CAPTION,
                    message_key::MEDIA_ITEMS,
                    bot_key::SENDER,
                ] {
                    if let Some(value) = source_object.get(key) {
                        reply.insert(key.to_owned(), value.clone());
                    }
                }
                if !reply.contains_key(bot_key::MEDIA_ITEMS)
                    && let Some(media) = source_object.get(bot_key::MEDIA_INFO)
                {
                    reply.insert(bot_key::MEDIA_ITEMS.to_owned(), RiseJSONValue::Array(vec![media.clone()]));
                }
                if !reply.contains_key(bot_key::SENDER)
                    && let Some(name) = Self::normalized(source.sender_name.as_deref())
                {
                    reply.insert(
                        bot_key::SENDER.to_owned(),
                        RiseJSONValue::Object(BTreeMap::from([
                            (bot_key::ID.to_owned(), RiseJSONValue::String(source.sender_id.clone().unwrap_or_default())),
                            (bot_key::NAME.to_owned(), RiseJSONValue::String(name)),
                        ])),
                    );
                }
            }
            reply.insert(bot_key::MESSAGE_ID.to_owned(), RiseJSONValue::String(reply_id));
            object.insert(bot_key::REPLY_TO.to_owned(), RiseJSONValue::Object(reply));
            object.insert(bot_key::IS_REPLY.to_owned(), RiseJSONValue::Bool(true));
        }
        object.insert(bot_key::ID.to_owned(), RiseJSONValue::String(local_id.to_owned()));
        object.insert(bot_key::TEMP_ID.to_owned(), RiseJSONValue::String(local_id.to_owned()));
        object.insert(bot_key::CHAT_ID.to_owned(), RiseJSONValue::String(scope.chat_id.clone()));
        object.insert(bot_key::CREATED_AT.to_owned(), RiseJSONValue::Number(timestamp));
        object.insert(bot_key::TIMESTAMP.to_owned(), RiseJSONValue::Number(timestamp));
        object.insert(RiseLocalKey::IS_TEMP.to_owned(), RiseJSONValue::Bool(true));
        object.insert(bot_key::IS_READ.to_owned(), RiseJSONValue::Bool(false));
        object.insert(bot_key::IS_PINNED.to_owned(), RiseJSONValue::Bool(false));
        object.insert(bot_key::HAS_REACTIONS.to_owned(), RiseJSONValue::Bool(false));
        object.insert(bot_key::REACTIONS.to_owned(), RiseJSONValue::Array(Vec::new()));
        if let Some(media_items) = object.get(bot_key::MEDIA_ITEMS) {
            let annotated = Self::installing_local_media_slot_indices(media_items);
            object.insert(bot_key::MEDIA_ITEMS.to_owned(), annotated);
        }
        if let Some(RiseJSONValue::Array(items)) = object.get(bot_key::MEDIA_ITEMS) {
            let has_attachments = !items.is_empty();
            object.insert(bot_key::HAS_ATTACHMENTS.to_owned(), RiseJSONValue::Bool(has_attachments));
        }
        if let Some(topic_id) = &scope.topic_id {
            object.insert(bot_key::TOPIC_ID.to_owned(), RiseJSONValue::String(topic_id.clone()));
        }
        Self::wire(object)
    }

    pub fn forwarded_optimistic_wire(
        source: &MessageWireDTO,
        scope: &RiseMessageScope,
        local_id: &str,
        sender_id: &str,
        timestamp: f64,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(source)?;
        let forward_info = match Self::value(&object, &[bot_key::FORWARD_INFO]) {
            Some(inherited) => inherited.clone(),
            None => {
                let source_sender = object.get(bot_key::SENDER);
                let source_sender_id = Self::value(&object, &[bot_key::SENDER_ID]).or_else(|| {
                    source_sender.and_then(RiseJSONValue::object_value).and_then(|sender| sender.get(bot_key::ID))
                });
                let source_sender_name = Self::normalized(source.sender_name.as_deref())
                    .or_else(|| Self::normalized(source.sender.as_ref().and_then(|sender| sender.name.as_deref())));
                let source_server_id = Self::canonical_server_id(&[source.server_id.as_deref(), source.id.as_deref()]);
                RiseJSONValue::Object(BTreeMap::from([
                    (
                        bot_key::FROM_CHAT_ID.to_owned(),
                        Self::value(&object, &[bot_key::CHAT_ID]).cloned().unwrap_or(RiseJSONValue::Null),
                    ),
                    (
                        bot_key::FROM_MESSAGE_ID.to_owned(),
                        source_server_id.map_or(RiseJSONValue::Null, RiseJSONValue::String),
                    ),
                    (bot_key::SENDER_ID.to_owned(), source_sender_id.cloned().unwrap_or(RiseJSONValue::Null)),
                    (
                        bot_key::SENDER_NAME.to_owned(),
                        source_sender_name.map_or(RiseJSONValue::Null, RiseJSONValue::String),
                    ),
                    (bot_key::SENDER.to_owned(), source_sender.cloned().unwrap_or(RiseJSONValue::Null)),
                    (
                        bot_key::DATE.to_owned(),
                        Self::value(&object, &[message_key::CREATED_AT, bot_key::TIMESTAMP])
                            .cloned()
                            .unwrap_or(RiseJSONValue::Null),
                    ),
                ]))
            }
        };
        for key in [
            RiseLocalKey::SERVER_ID,
            message_key::MESSAGE_ID,
            message_key::TOPIC_ID,
            message_key::REPLY_TO,
            message_key::REPLY_TO_ID,
            message_key::DISCUSSION_CHAT_ID,
            message_key::DISCUSSION_MESSAGE_ID,
            message_key::RECENT_COMMENTERS,
            message_key::EDIT_DATE,
            message_key::VIEW_COUNT,
            message_key::COMMENTS_COUNT,
            bot_key::SENDER,
        ] {
            object.remove(key);
        }
        object.insert(bot_key::FORWARD_INFO.to_owned(), forward_info);
        object.insert(bot_key::IS_FORWARDED.to_owned(), RiseJSONValue::Bool(true));
        object.insert(bot_key::IS_REPLY.to_owned(), RiseJSONValue::Bool(false));
        object.insert(bot_key::SENDER_ID.to_owned(), RiseJSONValue::String(sender_id.to_owned()));
        object.remove(message_key::SENDER_NAME);
        object.insert(bot_key::PLAYED_BY.to_owned(), RiseJSONValue::Array(Vec::new()));
        object.insert(bot_key::REACTED_BY.to_owned(), RiseJSONValue::Array(Vec::new()));
        object.insert(bot_key::REACTIONS.to_owned(), RiseJSONValue::Array(Vec::new()));
        object.insert(bot_key::HAS_REACTIONS.to_owned(), RiseJSONValue::Bool(false));
        object.insert(bot_key::REACTIONS_UNREADED.to_owned(), RiseJSONValue::Bool(false));
        object.insert(bot_key::IS_MENTIONED_YOU.to_owned(), RiseJSONValue::Bool(false));
        object.insert(message_key::IS_REPLIED_YOU.to_owned(), RiseJSONValue::Bool(false));
        object.insert(bot_key::IS_PINNED.to_owned(), RiseJSONValue::Bool(false));
        object.insert(bot_key::IS_READ.to_owned(), RiseJSONValue::Bool(false));
        object.insert(bot_key::CREATED_AT.to_owned(), RiseJSONValue::Number(timestamp));
        object.insert(bot_key::TIMESTAMP.to_owned(), RiseJSONValue::Number(timestamp));
        Self::installing_local_identity(&Self::wire(object)?, scope, local_id, timestamp)
    }

    pub fn installing_local_identity(
        existing: &MessageWireDTO,
        scope: &RiseMessageScope,
        local_id: &str,
        timestamp: f64,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        object.insert(bot_key::ID.to_owned(), RiseJSONValue::String(local_id.to_owned()));
        object.insert(bot_key::TEMP_ID.to_owned(), RiseJSONValue::String(local_id.to_owned()));
        object.insert(bot_key::CHAT_ID.to_owned(), RiseJSONValue::String(scope.chat_id.clone()));
        object.insert(RiseLocalKey::IS_TEMP.to_owned(), RiseJSONValue::Bool(true));
        if let Some(media_items) = object.get(bot_key::MEDIA_ITEMS) {
            let annotated = Self::installing_local_media_slot_indices(media_items);
            object.insert(bot_key::MEDIA_ITEMS.to_owned(), annotated);
        }
        if !object.contains_key(bot_key::CREATED_AT) {
            object.insert(bot_key::CREATED_AT.to_owned(), RiseJSONValue::Number(timestamp));
        }
        if !object.contains_key(bot_key::TIMESTAMP) {
            object.insert(bot_key::TIMESTAMP.to_owned(), RiseJSONValue::Number(timestamp));
        }
        if let Some(topic_id) = &scope.topic_id {
            object.insert(bot_key::TOPIC_ID.to_owned(), RiseJSONValue::String(topic_id.clone()));
        }
        Self::wire(object)
    }

    pub fn halting_media_uploads(existing: &MessageWireDTO) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        let halted: Vec<RiseJSONValue> = match object.get(bot_key::MEDIA_ITEMS) {
            Some(RiseJSONValue::Array(items)) if !items.is_empty() => items
                .iter()
                .map(|item| {
                    let Some(slot) = item.object_value() else {
                        return item.clone();
                    };
                    let mut slot = slot.clone();
                    slot.insert(RiseLocalKey::IS_UPLOADING.to_owned(), RiseJSONValue::Bool(false));
                    slot.insert(RiseLocalKey::IS_COMPRESSING.to_owned(), RiseJSONValue::Bool(false));
                    RiseJSONValue::Object(slot)
                })
                .collect(),
            _ => return Ok(existing.clone()),
        };
        object.insert(bot_key::MEDIA_ITEMS.to_owned(), RiseJSONValue::Array(halted));
        Self::wire(object)
    }

    /// a resent row takes the moment of the retry
    pub fn restamped(existing: &MessageWireDTO, timestamp: f64) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        object.insert(bot_key::CREATED_AT.to_owned(), RiseJSONValue::Number(timestamp));
        object.insert(bot_key::TIMESTAMP.to_owned(), RiseJSONValue::Number(timestamp));
        Self::wire(object)
    }

    pub fn updating_optimistic_wire(
        existing: &MessageWireDTO,
        body: &RiseStoredOutgoingMessage,
        scope: &RiseMessageScope,
        local_id: &str,
        timestamp: f64,
        cancelled_slot_indices: &HashSet<i64>,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        let body_object = match Self::snake_cased(body)? {
            RiseJSONValue::Object(object) => object,
            _ => RiseJSONObject::new(),
        };
        for (key, value) in body_object {
            if matches!(value, RiseJSONValue::Null) {
                continue;
            }
            if key == message_key::MEDIA_ITEMS {
                let merged =
                    Self::merge_media_items(object.get(&key), &value, true, true, false, cancelled_slot_indices);
                object.insert(key, merged);
            } else {
                object.insert(key, value);
            }
        }
        let merged = Self::wire(object)?;
        Self::installing_local_identity(&merged, scope, local_id, timestamp)
    }

    /// merges the server message into the already rendered optimistic one
    /// local identity stays, server identity goes in as an alias field
    pub fn reconciling_authoritative_wire(
        optimistic: &MessageWireDTO,
        authoritative: &MessageWireDTO,
        local_id: &str,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::merged_authoritative_object(optimistic, authoritative)?;
        let server_id = Self::canonical_server_id(&[authoritative.server_id.as_deref(), authoritative.id.as_deref()]);
        object.insert(bot_key::ID.to_owned(), RiseJSONValue::String(local_id.to_owned()));
        object.insert(bot_key::TEMP_ID.to_owned(), RiseJSONValue::String(local_id.to_owned()));
        object.insert(RiseLocalKey::IS_TEMP.to_owned(), RiseJSONValue::Bool(false));
        if let Some(server_id) = server_id {
            object.insert(RiseLocalKey::SERVER_ID.to_owned(), RiseJSONValue::String(server_id));
        }
        Self::wire(object)
    }

    pub fn acknowledged_wire(
        optimistic: &MessageWireDTO,
        response: &CreateMessageGatewayDTO,
        local_id: &str,
        scope: &RiseMessageScope,
    ) -> anyhow::Result<MessageWireDTO> {
        let first_message = response.messages.as_ref().and_then(|messages| messages.first());
        let mut object = match response.message.as_ref().or(first_message) {
            Some(server) => Self::object(&Self::reconciling_authoritative_wire(optimistic, server, local_id)?)?,
            None => Self::object(optimistic)?,
        };
        let server_id = Self::canonical_server_id(&[
            response.message.as_ref().and_then(|message| message.server_id.as_deref()),
            response.message.as_ref().and_then(|message| message.id.as_deref()),
            first_message.and_then(|message| message.server_id.as_deref()),
            first_message.and_then(|message| message.id.as_deref()),
            response.message_id.as_deref(),
            response.id.as_deref(),
        ]);
        object.insert(bot_key::ID.to_owned(), RiseJSONValue::String(local_id.to_owned()));
        object.insert(bot_key::TEMP_ID.to_owned(), RiseJSONValue::String(local_id.to_owned()));
        object.insert(
            bot_key::CHAT_ID.to_owned(),
            RiseJSONValue::String(response.chat_id.clone().unwrap_or_else(|| scope.chat_id.clone())),
        );
        object.insert(RiseLocalKey::IS_TEMP.to_owned(), RiseJSONValue::Bool(false));
        if let Some(server_id) = server_id {
            object.insert(RiseLocalKey::SERVER_ID.to_owned(), RiseJSONValue::String(server_id));
        }
        if !object.contains_key(bot_key::CREATED_AT)
            && let Some(created_at) = response.created_at
        {
            object.insert(bot_key::CREATED_AT.to_owned(), RiseJSONValue::Number(created_at));
        }
        if !object.contains_key(bot_key::TIMESTAMP)
            && let Some(timestamp) = response.timestamp
        {
            object.insert(bot_key::TIMESTAMP.to_owned(), RiseJSONValue::Number(timestamp));
        }
        if let Some(topic_id) = &scope.topic_id
            && !object.contains_key(bot_key::TOPIC_ID)
        {
            object.insert(bot_key::TOPIC_ID.to_owned(), RiseJSONValue::String(topic_id.clone()));
        }
        Self::wire(object)
    }

    fn merged_authoritative_object(
        local: &MessageWireDTO,
        authoritative: &MessageWireDTO,
    ) -> anyhow::Result<RiseJSONObject> {
        let mut object = Self::object(local)?;
        let authoritative_object = Self::object(authoritative)?;
        for (key, value) in authoritative_object {
            if matches!(value, RiseJSONValue::Null) {
                continue;
            }
            if key == message_key::MEDIA_ITEMS {
                let merged = Self::merge_media_items(object.get(&key), &value, true, true, false, &HashSet::new());
                object.insert(key, merged);
            } else if [message_key::REPLY_TO, message_key::FORWARD_INFO, bot_key::SENDER, message_key::LINK_PREVIEW]
                .contains(&key.as_str())
            {
                let merged = Self::merge_presentation_object(object.get(&key), &value);
                object.insert(key, merged);
            } else {
                object.insert(key, value);
            }
        }
        Ok(object)
    }

    fn merge_presentation_object(local: Option<&RiseJSONValue>, authoritative: &RiseJSONValue) -> RiseJSONValue {
        let (Some(RiseJSONValue::Object(local_object)), RiseJSONValue::Object(incoming)) = (local, authoritative)
        else {
            return authoritative.clone();
        };
        let mut result = local_object.clone();
        for key in [
            bot_key::ID,
            message_key::MESSAGE_ID,
            message_key::FROM_MESSAGE_ID,
            message_key::FROM_CHAT_ID,
            message_key::SENDER_ID,
        ] {
            if let Some(old_id) = Self::string(&result, &[key])
                && let Some(new_id) = Self::string(incoming, &[key])
                && old_id != new_id
            {
                return authoritative.clone();
            }
        }
        for (key, value) in incoming {
            if matches!(value, RiseJSONValue::Null) {
                continue;
            }
            if key == bot_key::SENDER || key == bot_key::MORE {
                let merged = Self::merge_presentation_object(result.get(key), value);
                result.insert(key.clone(), merged);
            } else {
                result.insert(key.clone(), value.clone());
            }
        }
        RiseJSONValue::Object(result)
    }

    /// server media is merged into the existing slot and doesnt replace the array
    /// so the local preview survives while server file ids and urls still land
    fn merge_media_items(
        local: Option<&RiseJSONValue>,
        authoritative: &RiseJSONValue,
        upload_finished: bool,
        preserve_local_slots: bool,
        invalidates_replaced_previews: bool,
        cancelled_slot_indices: &HashSet<i64>,
    ) -> RiseJSONValue {
        let RiseJSONValue::Array(remote_items) = authoritative else {
            return if preserve_local_slots { local.unwrap_or(authoritative).clone() } else { authoritative.clone() };
        };
        let raw_local_items: &[RiseJSONValue] = match local {
            Some(RiseJSONValue::Array(values)) => values.as_slice(),
            _ => &[],
        };

        let local_items = Self::annotated_media_items(raw_local_items, cancelled_slot_indices);
        if !preserve_local_slots {
            let merged = remote_items
                .iter()
                .enumerate()
                .map(|(index, remote_item)| {
                    let local_item = local_items.get(index);
                    let slot_index = local_item
                        .and_then(RiseJSONValue::object_value)
                        .and_then(Self::media_slot_index)
                        .or_else(|| remote_item.object_value().and_then(Self::media_slot_index))
                        .unwrap_or(index as i64);
                    Self::merged_media_item(
                        local_item,
                        remote_item,
                        slot_index,
                        upload_finished,
                        invalidates_replaced_previews,
                    )
                })
                .collect();
            return RiseJSONValue::Array(merged);
        }

        if local_items.is_empty() {
            let annotated = Self::annotated_media_items(remote_items, cancelled_slot_indices)
                .into_iter()
                .map(|item| Self::finalized_media_item(item, upload_finished))
                .collect();
            return RiseJSONValue::Array(annotated);
        }

        let mut merged = local_items;
        let mut array_index_by_slot: HashMap<i64, usize> = HashMap::new();
        let mut used_slot_indices: HashSet<i64> = HashSet::new();
        for (array_index, item) in merged.iter().enumerate() {
            let Some(slot_index) = item.object_value().and_then(Self::media_slot_index) else {
                continue;
            };
            array_index_by_slot.insert(slot_index, array_index);
            used_slot_indices.insert(slot_index);
        }

        let active_array_indices: Vec<usize> =
            (0..merged.len()).filter(|index| !Self::media_item_is_cancelled(Some(&merged[*index]))).collect();
        let mut active_cursor = 0;
        let mut consumed_array_indices: HashSet<usize> = HashSet::new();

        for remote_item in remote_items {
            let explicit_slot = remote_item.object_value().and_then(Self::media_slot_index);
            let mut target_array_index: Option<usize> = None;
            if let Some(explicit_slot) = explicit_slot
                && let Some(&explicit_array_index) = array_index_by_slot.get(&explicit_slot)
                && !Self::media_item_is_cancelled(Some(&merged[explicit_array_index]))
                && consumed_array_indices.insert(explicit_array_index)
            {
                target_array_index = Some(explicit_array_index);
            }
            if target_array_index.is_none() {
                while active_cursor < active_array_indices.len() {
                    let candidate = active_array_indices[active_cursor];
                    active_cursor += 1;
                    if consumed_array_indices.insert(candidate) {
                        target_array_index = Some(candidate);
                        break;
                    }
                }
            }

            let target = target_array_index.and_then(|array_index| {
                merged[array_index]
                    .object_value()
                    .and_then(Self::media_slot_index)
                    .map(|slot_index| (array_index, slot_index))
            });
            if let Some((array_index, slot_index)) = target {
                let next = Self::merged_media_item(
                    Some(&merged[array_index]),
                    remote_item,
                    slot_index,
                    upload_finished,
                    invalidates_replaced_previews,
                );
                merged[array_index] = next;
            } else {
                let slot_index = Self::available_media_slot_index(explicit_slot, &used_slot_indices);
                used_slot_indices.insert(slot_index);
                merged.push(Self::merged_media_item(None, remote_item, slot_index, upload_finished, false));
            }
        }

        RiseJSONValue::Array(merged.into_iter().map(|item| Self::finalized_media_item(item, upload_finished)).collect())
    }

    fn installing_local_media_slot_indices(media_items: &RiseJSONValue) -> RiseJSONValue {
        let RiseJSONValue::Array(items) = media_items else {
            return media_items.clone();
        };
        RiseJSONValue::Array(Self::annotated_media_items(items, &HashSet::new()))
    }

    fn annotated_media_items(items: &[RiseJSONValue], cancelled_slot_indices: &HashSet<i64>) -> Vec<RiseJSONValue> {
        let mut used_slot_indices: HashSet<i64> = HashSet::new();
        items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let Some(source) = item.object_value() else {
                    return item.clone();
                };
                let mut object = source.clone();
                let preferred = Self::media_slot_index(&object).unwrap_or(index as i64);
                let slot_index = Self::available_media_slot_index(Some(preferred), &used_slot_indices);
                used_slot_indices.insert(slot_index);
                object.insert(RiseLocalKey::LOCAL_SLOT_INDEX.to_owned(), RiseJSONValue::Integer(slot_index));
                if cancelled_slot_indices.contains(&slot_index) {
                    object.insert(RiseLocalKey::IS_CANCELLED.to_owned(), RiseJSONValue::Bool(true));
                }
                RiseJSONValue::Object(object)
            })
            .collect()
    }

    fn merged_media_item(
        local: Option<&RiseJSONValue>,
        authoritative: &RiseJSONValue,
        local_slot_index: i64,
        upload_finished: bool,
        invalidates_replaced_previews: bool,
    ) -> RiseJSONValue {
        let mut combined = local.and_then(RiseJSONValue::object_value).cloned().unwrap_or_default();
        if let Some(remote_object) = authoritative.object_value() {
            if invalidates_replaced_previews
                && let Some(next_url) =
                    Self::normalized(remote_object.get(bot_key::FILE_URL).and_then(RiseJSONValue::string_value))
                && Some(&next_url)
                    != Self::normalized(combined.get(bot_key::FILE_URL).and_then(RiseJSONValue::string_value)).as_ref()
            {
                combined.remove(RiseLocalKey::LOCAL_PREVIEW_URL);
                combined.remove(bot_key::THUMBNAIL_URL);
            }
            for (key, value) in remote_object {
                if !matches!(value, RiseJSONValue::Null) {
                    combined.insert(key.clone(), value.clone());
                }
            }
        }
        combined.insert(RiseLocalKey::LOCAL_SLOT_INDEX.to_owned(), RiseJSONValue::Integer(local_slot_index.max(0)));
        if Self::media_item_is_cancelled(local) {
            combined.insert(RiseLocalKey::IS_CANCELLED.to_owned(), RiseJSONValue::Bool(true));
        }
        Self::finalized_media_item(RiseJSONValue::Object(combined), upload_finished)
    }

    fn finalized_media_item(item: RiseJSONValue, upload_finished: bool) -> RiseJSONValue {
        if !upload_finished {
            return item;
        }
        match item {
            RiseJSONValue::Object(mut object) => {
                object.insert(RiseLocalKey::IS_UPLOADING.to_owned(), RiseJSONValue::Bool(false));
                object.insert(RiseLocalKey::IS_COMPRESSING.to_owned(), RiseJSONValue::Bool(false));
                if !Self::object_is_cancelled(&object) {
                    object.insert(RiseLocalKey::UPLOAD_PROGRESS.to_owned(), RiseJSONValue::Number(100.0));
                    object.insert(RiseLocalKey::COMPRESSION_PROGRESS.to_owned(), RiseJSONValue::Number(100.0));
                }
                RiseJSONValue::Object(object)
            }
            other => other,
        }
    }

    fn media_item_is_cancelled(item: Option<&RiseJSONValue>) -> bool {
        item.and_then(RiseJSONValue::object_value).is_some_and(Self::object_is_cancelled)
    }

    fn object_is_cancelled(object: &RiseJSONObject) -> bool {
        let camel = RiseWireKey::foundation_camel(RiseLocalKey::IS_CANCELLED);
        Self::bool(object, &[RiseLocalKey::IS_CANCELLED, camel.as_str()]) == Some(true)
    }

    fn media_slot_index(object: &RiseJSONObject) -> Option<i64> {
        let camel = RiseWireKey::foundation_camel(RiseLocalKey::LOCAL_SLOT_INDEX);
        let slot_index = match Self::value(object, &[RiseLocalKey::LOCAL_SLOT_INDEX, camel.as_str()])? {
            RiseJSONValue::Integer(value) => Some(*value),
            RiseJSONValue::UnsignedInteger(value) => i64::try_from(*value).ok(),
            RiseJSONValue::Number(value) => {
                let exact = value.is_finite()
                    && value.round() == *value
                    && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(value);
                exact.then_some(*value as i64)
            }
            RiseJSONValue::String(value) => int_from_string(value),
            _ => None,
        }?;
        (slot_index >= 0).then_some(slot_index)
    }

    fn available_media_slot_index(preferred: Option<i64>, used: &HashSet<i64>) -> i64 {
        if let Some(preferred) = preferred
            && preferred >= 0
            && !used.contains(&preferred)
        {
            return preferred;
        }
        let mut candidate = 0;
        while used.contains(&candidate) {
            candidate += 1;
        }
        candidate
    }

    fn canonical_server_id(candidates: &[Option<&str>]) -> Option<String> {
        candidates
            .iter()
            .filter_map(|candidate| Self::normalized(*candidate))
            .find(|candidate| !GetSaiServerId::is_temp_id(Some(candidate.as_str())))
    }

    pub fn editing(
        existing: &MessageWireDTO,
        payload: &RiseJSONObject,
        upload_finished: bool,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        let previous_edit_date = object.get(bot_key::EDIT_DATE).cloned();
        if !object.contains_key(bot_key::ORIGINAL_CONTENT)
            && let Some(content) = object.get(bot_key::CONTENT).cloned()
        {
            object.insert(bot_key::ORIGINAL_CONTENT.to_owned(), content);
        }
        if let Some(content) = Self::value(payload, &[message_key::NEW_CONTENT, bot_key::CONTENT, bot_key::TEXT]) {
            object.insert(bot_key::CONTENT.to_owned(), content.clone());
        }
        let edit_date = match Self::value(payload, &[message_key::EDIT_DATE]) {
            Some(edit_date) => edit_date.clone(),
            None => RiseJSONValue::Number(RiseClock::now().time_interval_since_1970()),
        };
        object.insert(bot_key::EDIT_DATE.to_owned(), edit_date);
        let view_limited = existing
            .media_items
            .as_ref()
            .is_some_and(|items| items.iter().any(|item| !item.view_mode.as_deref().unwrap_or_default().is_empty()));
        if !view_limited {
            object.insert(bot_key::IS_EDITED.to_owned(), RiseJSONValue::Bool(true));
        } else {
            match previous_edit_date {
                Some(previous) => {
                    object.insert(bot_key::EDIT_DATE.to_owned(), previous);
                }
                None => {
                    object.remove(bot_key::EDIT_DATE);
                }
            }
        }
        for key in [
            bot_key::ENTITIES,
            bot_key::CAPTION,
            message_key::REPLY_MARKUP,
            message_key::WEB_APP_DATA,
            bot_key::DOCUMENT,
            message_key::MODERATION_STATUS,
        ] {
            if let Some(new_value) = payload.get(key) {
                object.insert(key.to_owned(), new_value.clone());
            }
        }
        if !payload.contains_key(bot_key::MODERATION_STATUS)
            && let Some(message) = payload.get(bot_key::MESSAGE).and_then(RiseJSONValue::object_value)
            && let Some(status) = message.get(bot_key::MODERATION_STATUS)
        {
            let previous = existing.moderation_status.as_deref().map_or("", trimming_whitespaces_and_newlines);
            if status.string_value().unwrap_or("") != previous {
                object.insert(bot_key::MODERATION_STATUS.to_owned(), status.clone());
                if let Some(edited) = message.get(bot_key::IS_EDITED) {
                    object.insert(bot_key::IS_EDITED.to_owned(), edited.clone());
                }
                if let Some(edit_date) = message.get(bot_key::EDIT_DATE) {
                    object.insert(bot_key::EDIT_DATE.to_owned(), edit_date.clone());
                }
            }
        }
        let previous_link = ChatLinkPreviewURL::first(Self::link_source(existing));
        let edited_text = Self::string(&object, &[bot_key::CONTENT])
            .or_else(|| Self::string(&object, &[bot_key::CAPTION]))
            .unwrap_or_default();
        let edited_link = ChatLinkPreviewURL::first(&edited_text);
        if previous_link != edited_link {
            if let Some(mode) = existing.link_preview.as_ref().and_then(|preview| preview.link_preview_type.clone()) {
                object.insert(
                    bot_key::LINK_PREVIEW.to_owned(),
                    RiseJSONValue::Object(BTreeMap::from([(
                        bot_key::LINK_PREVIEW_TYPE.to_owned(),
                        RiseJSONValue::String(mode),
                    )])),
                );
            } else {
                object.remove(message_key::LINK_PREVIEW);
            }
        }
        if edited_link.is_some()
            && let Some(mode) = payload.get(bot_key::LINK_PREVIEW_TYPE).and_then(RiseJSONValue::string_value)
        {
            let mut card = if previous_link == edited_link {
                object.get(bot_key::LINK_PREVIEW).and_then(RiseJSONValue::object_value).cloned().unwrap_or_default()
            } else {
                BTreeMap::new()
            };
            card.insert(bot_key::LINK_PREVIEW_TYPE.to_owned(), RiseJSONValue::String(mode.to_owned()));
            object.insert(bot_key::LINK_PREVIEW.to_owned(), RiseJSONValue::Object(card));
        }
        let edited_row_media_items = Self::edited_row_media_items(payload);
        if let Some(media_items) =
            Self::value(payload, &[message_key::MEDIA_ITEMS]).or(edited_row_media_items.as_ref())
        {
            let merged = Self::merge_media_items(
                object.get(bot_key::MEDIA_ITEMS),
                media_items,
                upload_finished,
                false,
                true,
                &HashSet::new(),
            );
            object.insert(bot_key::MEDIA_ITEMS.to_owned(), merged);
            object.insert(bot_key::HAS_ATTACHMENTS.to_owned(), RiseJSONValue::Bool(true));
        }
        Self::wire(object)
    }

    /// media of the reread row inside an edit event, an empty list is ignored
    /// so a text edit never turns a message into an attachment
    fn edited_row_media_items(payload: &RiseJSONObject) -> Option<RiseJSONValue> {
        let message = payload.get(bot_key::MESSAGE).and_then(RiseJSONValue::object_value)?;
        match message.get(bot_key::MEDIA_ITEMS) {
            Some(RiseJSONValue::Array(items)) if !items.is_empty() => Some(RiseJSONValue::Array(items.clone())),
            _ => None,
        }
    }

    fn link_source(wire: &MessageWireDTO) -> &str {
        wire.content.as_deref().or(wire.caption.as_deref()).unwrap_or("")
    }

    pub fn requoting(existing: &MessageWireDTO, source: Option<&MessageWireDTO>) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        let Some(mut reply) = object.get(bot_key::REPLY_TO).and_then(RiseJSONValue::object_value).cloned() else {
            return Ok(existing.clone());
        };
        if let Some(source) = source {
            let source_object = Self::object(source)?;
            for key in [bot_key::CONTENT, message_key::CONTENT_TYPE, bot_key::CAPTION, message_key::MEDIA_ITEMS] {
                match source_object.get(key) {
                    Some(value) if !matches!(value, RiseJSONValue::Null) => {
                        reply.insert(key.to_owned(), value.clone());
                    }
                    _ if key != message_key::CONTENT_TYPE => {
                        reply.remove(key);
                    }
                    _ => {}
                }
            }
            object.insert(bot_key::REPLY_TO.to_owned(), RiseJSONValue::Object(reply));
        } else {
            object.remove(message_key::REPLY_TO);
        }
        Self::wire(object)
    }

    pub fn resolving_link_preview(
        existing: &MessageWireDTO,
        payload: &RiseJSONObject,
    ) -> anyhow::Result<MessageWireDTO> {
        let preview_type = existing
            .link_preview
            .as_ref()
            .and_then(|preview| preview.link_preview_type.as_deref())
            .map(str::to_uppercase);
        if preview_type.as_deref() == Some(LinkPreviewSize::None.wire()) {
            return Ok(existing.clone());
        }
        let Some(incoming) = Self::value(payload, &[message_key::LINK_PREVIEW]) else {
            return Ok(existing.clone());
        };
        if incoming.object_value().is_none() {
            return Ok(existing.clone());
        }
        let text = Self::link_source(existing);
        let current_link = ChatLinkPreviewURL::first(text);
        if current_link.is_none() {
            return Ok(existing.clone());
        }
        if let Some(link) = Self::string(payload, &[message_key::LINK])
            && !Self::resolved_link_still_in_text(&link, text, current_link.as_deref())
        {
            return Ok(existing.clone());
        }
        let mut object = Self::object(existing)?;
        let merged = Self::merge_presentation_object(object.get(bot_key::LINK_PREVIEW), incoming);
        object.insert(bot_key::LINK_PREVIEW.to_owned(), merged);
        Self::wire(object)
    }

    fn resolved_link_still_in_text(link: &str, text: &str, current_link: Option<&str>) -> bool {
        if current_link == ChatLinkPreviewURL::first(link).as_deref() {
            return true;
        }
        let trimmed = trimming_whitespaces_and_newlines(link);
        if trimmed.is_empty() {
            return false;
        }
        if localized_case_insensitive_contains(text, trimmed) {
            return true;
        }
        if !trimmed.to_lowercase().starts_with("https://") {
            return false;
        }
        let bare: String = trimmed.chars().skip(8).collect();
        !bare.is_empty() && localized_case_insensitive_contains(text, &bare)
    }

    pub fn setting_boolean(existing: &MessageWireDTO, key: &str, value: bool) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        object.insert(key.to_owned(), RiseJSONValue::Bool(value));
        Self::wire(object)
    }

    pub fn setting_comments(existing: &MessageWireDTO, payload: &RiseJSONObject) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        if let Some(count) = Self::value(payload, &[message_key::COMMENTS_COUNT]) {
            object.insert(bot_key::COMMENTS_COUNT.to_owned(), count.clone());
        }
        if let Some(chat) = Self::value(payload, &[message_key::DISCUSSION_CHAT_ID]) {
            object.insert(bot_key::DISCUSSION_CHAT_ID.to_owned(), chat.clone());
        }
        if let Some(message) = Self::value(payload, &[message_key::DISCUSSION_MESSAGE_ID]) {
            object.insert(bot_key::DISCUSSION_MESSAGE_ID.to_owned(), message.clone());
        }
        if let Some(recent) = Self::value(payload, &[message_key::RECENT_COMMENTERS]) {
            object.insert(bot_key::RECENT_COMMENTERS.to_owned(), recent.clone());
        }
        Self::wire(object)
    }

    pub fn merging_played_by(existing: &MessageWireDTO, payload: &RiseJSONObject) -> anyhow::Result<MessageWireDTO> {
        let mut played = existing.played_by.clone().unwrap_or_default();
        for identifier in Self::strings(payload, &[message_key::PLAYED_BY]) {
            if !played.contains(&identifier) {
                played.push(identifier);
            }
        }
        let mut object = Self::object(existing)?;
        object.insert(
            bot_key::PLAYED_BY.to_owned(),
            RiseJSONValue::Array(played.into_iter().map(RiseJSONValue::String).collect()),
        );
        Self::wire(object)
    }

    pub fn applying_reaction(
        existing: &MessageWireDTO,
        payload: &RiseJSONObject,
        account_id: &str,
        is_adding: bool,
        own_reaction_limit: Option<i64>,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        let payload_reacted_by = Self::value(payload, &[message_key::REACTED_BY]);
        let carries_reactors =
            matches!(payload_reacted_by, Some(RiseJSONValue::Array(entries)) if !entries.is_empty());
        let mut reacted_by_entries = match (payload_reacted_by, object.get(bot_key::REACTED_BY)) {
            (Some(RiseJSONValue::Array(entries)), _) if carries_reactors => {
                Self::keeping_known_reactor_photos(entries, &object)
            }
            (_, Some(RiseJSONValue::Array(entries))) => entries.clone(),
            _ => Vec::new(),
        };

        if let Some(RiseJSONValue::Array(values)) = Self::value(payload, &[bot_key::REACTIONS]) {
            let frame_reaction = Self::string(payload, &[bot_key::REACTION]);
            let remover_id = Self::string(payload, &[events_key::REMOVER_ID]);
            if !carries_reactors
                && let Some(frame_reaction) = frame_reaction.as_deref()
                && !frame_reaction.is_empty()
            {
                let changed_id = if is_adding {
                    Self::string(payload, &[events_key::ACTOR_ID])
                } else {
                    remover_id.clone()
                };
                if let Some(changed_id) = changed_id.filter(|changed_id| !changed_id.is_empty()) {
                    reacted_by_entries.retain(|entry| !Self::reacted_by_entry_matches(entry, frame_reaction, &changed_id));
                    if is_adding {
                        let added = Self::reacted_by_entry(frame_reaction, &changed_id, existing);
                        let mut next = Self::keeping_known_reactor_photos(&[added], &object);
                        next.append(&mut reacted_by_entries);
                        reacted_by_entries = next;
                    }
                }
                let summary_reactions: HashSet<String> = values
                    .iter()
                    .filter_map(|value| Self::string(value.object_value()?, &[bot_key::REACTION]))
                    .collect();
                reacted_by_entries.retain(|entry_value| {
                    entry_value
                        .object_value()
                        .and_then(|entry| Self::string(entry, &[bot_key::REACTION]))
                        .is_some_and(|reaction| summary_reactions.contains(&reaction))
                });
            }
            let mine = Self::own_reaction_set(&reacted_by_entries, account_id);
            let existing_mine: HashSet<String> = existing
                .reactions
                .iter()
                .flatten()
                .filter_map(|summary| {
                    if summary.reacted_by_you != Some(true) {
                        return None;
                    }
                    if !is_adding
                        && remover_id.as_deref() == Some(account_id)
                        && frame_reaction.as_deref() == Some(summary.reaction.as_str())
                    {
                        return None;
                    }
                    Some(summary.reaction.clone())
                })
                .collect();
            let normalized: Vec<RiseJSONValue> = values
                .iter()
                .filter_map(|value| {
                    let mut summary = value.object_value()?.clone();
                    let reaction = Self::string(&summary, &[bot_key::REACTION])?;
                    summary.insert(
                        bot_key::REACTED_BY_YOU.to_owned(),
                        RiseJSONValue::Bool(mine.contains(&reaction) || existing_mine.contains(&reaction)),
                    );
                    Some(RiseJSONValue::Object(summary))
                })
                .collect();
            let has_reactions = !normalized.is_empty();
            object.insert(bot_key::REACTIONS.to_owned(), RiseJSONValue::Array(normalized));
            object.insert(bot_key::REACTED_BY.to_owned(), RiseJSONValue::Array(reacted_by_entries));
            object.insert(bot_key::HAS_REACTIONS.to_owned(), RiseJSONValue::Bool(has_reactions));
            return Self::wire(object);
        }

        let Some(reaction) = Self::string(payload, &[bot_key::REACTION]).filter(|reaction| !reaction.is_empty()) else {
            return Ok(existing.clone());
        };
        let reactor_id =
            Self::string(payload, &[message_key::USER_ID, message_key::SENDER_ID, events_key::REMOVER_ID]);
        let reactor_is_account = reactor_id.as_deref() == Some(account_id);
        let mut summaries: Vec<MessageReactionSummaryWireDTO> = existing.reactions.clone().unwrap_or_default();

        if is_adding
            && reactor_is_account
            && let Some(own_reaction_limit) = own_reaction_limit
            && summaries.iter().find(|summary| summary.reaction == reaction).and_then(|summary| summary.reacted_by_you)
                != Some(true)
        {
            let own_indices: Vec<usize> =
                (0..summaries.len()).filter(|index| summaries[*index].reacted_by_you == Some(true)).collect();
            if own_indices.len() as i64 >= own_reaction_limit
                && let Some(evicted_index) = own_indices.iter().copied().min_by(|lhs, rhs| {
                    let lhs_timestamp =
                        Self::own_reaction_timestamp(&summaries[*lhs].reaction, &reacted_by_entries, account_id)
                            .unwrap_or(f64::MIN);
                    let rhs_timestamp =
                        Self::own_reaction_timestamp(&summaries[*rhs].reaction, &reacted_by_entries, account_id)
                            .unwrap_or(f64::MIN);
                    lhs_timestamp.total_cmp(&rhs_timestamp).then(lhs.cmp(rhs))
                })
            {
                let evicted_reaction = summaries[evicted_index].reaction.clone();
                let remaining_count = (summaries[evicted_index].count - 1).max(0);
                if remaining_count == 0 {
                    summaries.remove(evicted_index);
                } else {
                    summaries[evicted_index] = MessageReactionSummaryWireDTO {
                        reaction: evicted_reaction.clone(),
                        count: remaining_count,
                        reacted_by_you: Some(false),
                    };
                }
                reacted_by_entries
                    .retain(|entry| !Self::reacted_by_entry_matches(entry, &evicted_reaction, account_id));
            }
        }

        if let Some(index) = summaries.iter().position(|summary| summary.reaction == reaction) {
            let current_count = summaries[index].count;
            let current_reacted_by_you = summaries[index].reacted_by_you;
            let already_mine = current_reacted_by_you == Some(true) && reactor_is_account;
            let count = if !is_adding {
                (current_count - 1).max(0)
            } else if already_mine {
                current_count
            } else {
                current_count + 1
            };
            if count == 0 {
                summaries.remove(index);
            } else {
                summaries[index] = MessageReactionSummaryWireDTO {
                    reaction: reaction.clone(),
                    count,
                    reacted_by_you: if reactor_is_account { Some(is_adding) } else { current_reacted_by_you },
                };
            }
        } else if is_adding {
            summaries.push(MessageReactionSummaryWireDTO {
                reaction: reaction.clone(),
                count: 1,
                reacted_by_you: Some(reactor_is_account),
            });
        }

        if reactor_is_account {
            reacted_by_entries.retain(|entry| !Self::reacted_by_entry_matches(entry, &reaction, account_id));
            if is_adding {
                reacted_by_entries.insert(0, Self::reacted_by_entry(&reaction, account_id, existing));
            }
        } else if payload_reacted_by.is_some() {
            let mine = Self::own_reaction_set(&reacted_by_entries, account_id);
            summaries = summaries
                .into_iter()
                .map(|summary| MessageReactionSummaryWireDTO {
                    reacted_by_you: Some(mine.contains(&summary.reaction) || summary.reacted_by_you == Some(true)),
                    reaction: summary.reaction,
                    count: summary.count,
                })
                .collect();
        }

        let has_reactions = !summaries.is_empty();
        object.insert(bot_key::REACTIONS.to_owned(), Self::snake_cased(&summaries)?);
        object.insert(bot_key::REACTED_BY.to_owned(), RiseJSONValue::Array(reacted_by_entries));
        object.insert(bot_key::HAS_REACTIONS.to_owned(), RiseJSONValue::Bool(has_reactions));
        Self::wire(object)
    }

    fn reacted_by_entry(reaction: &str, user_id: &str, existing: &MessageWireDTO) -> RiseJSONValue {
        let created_at = (RiseClock::now().time_interval_since_1970() * 1000.0).round() as i64;
        RiseJSONValue::Object(BTreeMap::from([
            (bot_key::REACTION.to_owned(), RiseJSONValue::String(reaction.to_owned())),
            (bot_key::CREATED_AT.to_owned(), RiseJSONValue::Integer(created_at)),
            (
                bot_key::SENDER.to_owned(),
                RiseJSONValue::Object(BTreeMap::from([(
                    bot_key::ID.to_owned(),
                    RiseJSONValue::String(user_id.to_owned()),
                )])),
            ),
            (bot_key::CHAT_ID.to_owned(), RiseJSONValue::String(existing.chat_id.clone().unwrap_or_default())),
        ]))
    }

    /// reaction frames reach the whole chat so the server strips reactor photos
    /// and delta frames name no reactor
    /// photo or name from the last read for the same person is kept
    pub fn keeping_known_reactor_photos(entries: &[RiseJSONValue], existing: &RiseJSONObject) -> Vec<RiseJSONValue> {
        let mut logos: HashMap<String, String> = HashMap::new();
        let mut names: HashMap<String, String> = HashMap::new();
        if let Some(RiseJSONValue::Array(previous)) = existing.get(bot_key::REACTED_BY) {
            for entry_value in previous {
                let Some(entry) = entry_value.object_value() else {
                    continue;
                };
                let Some(user_id) = Self::reacted_by_user_id(entry) else {
                    continue;
                };
                let sender = Self::value(entry, &[bot_key::SENDER]).and_then(RiseJSONValue::object_value);
                if !logos.contains_key(&user_id)
                    && let Some(logo) = Self::sender_logo(sender)
                {
                    logos.insert(user_id.clone(), logo);
                }
                if !names.contains_key(&user_id)
                    && let Some(name) = Self::sender_name(sender)
                {
                    names.insert(user_id, name);
                }
            }
        }
        if let Some(sender) = Self::value(existing, &[bot_key::SENDER]).and_then(RiseJSONValue::object_value)
            && let Some(user_id) = Self::string(sender, &[bot_key::ID])
        {
            if !logos.contains_key(&user_id)
                && let Some(logo) =
                    Self::sender_logo(Some(sender)).or_else(|| Self::string(existing, &[bot_key::SENDER_LOGO]))
            {
                logos.insert(user_id.clone(), logo);
            }
            if !names.contains_key(&user_id)
                && let Some(name) =
                    Self::sender_name(Some(sender)).or_else(|| Self::string(existing, &[bot_key::SENDER_NAME]))
            {
                names.insert(user_id, name);
            }
        }
        if logos.is_empty() && names.is_empty() {
            return entries.to_vec();
        }
        entries
            .iter()
            .map(|entry_value| {
                let Some(entry) = entry_value.object_value() else {
                    return entry_value.clone();
                };
                let Some(source_sender) = Self::value(entry, &[bot_key::SENDER]).and_then(RiseJSONValue::object_value)
                else {
                    return entry_value.clone();
                };
                let Some(user_id) = Self::reacted_by_user_id(entry) else {
                    return entry_value.clone();
                };
                let mut sender = source_sender.clone();
                let mut changed = false;
                if Self::sender_logo(Some(&sender)).is_none()
                    && let Some(logo) = logos.get(&user_id)
                {
                    let mut more = Self::value(&sender, &[bot_key::MORE])
                        .and_then(RiseJSONValue::object_value)
                        .cloned()
                        .unwrap_or_default();
                    more.insert(bot_key::LOGO.to_owned(), RiseJSONValue::String(logo.clone()));
                    sender.insert(bot_key::MORE.to_owned(), RiseJSONValue::Object(more));
                    changed = true;
                }
                if Self::sender_name(Some(&sender)).is_none()
                    && let Some(name) = names.get(&user_id)
                {
                    sender.insert(bot_key::NAME.to_owned(), RiseJSONValue::String(name.clone()));
                    changed = true;
                }
                if !changed {
                    return entry_value.clone();
                }
                let mut entry = entry.clone();
                entry.insert(bot_key::SENDER.to_owned(), RiseJSONValue::Object(sender));
                RiseJSONValue::Object(entry)
            })
            .collect()
    }

    pub fn has_unnamed_reactor(wire: &MessageWireDTO) -> bool {
        let Ok(object) = Self::object(wire) else {
            return false;
        };
        let Some(RiseJSONValue::Array(entries)) = object.get(bot_key::REACTED_BY) else {
            return false;
        };
        entries.iter().any(|entry_value| {
            let Some(entry) = entry_value.object_value() else {
                return false;
            };
            Self::sender_name(Self::value(entry, &[bot_key::SENDER]).and_then(RiseJSONValue::object_value)).is_none()
        })
    }

    pub fn filling_reactor_identities(
        existing: &MessageWireDTO,
        fresh: &RiseJSONObject,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        let Some(RiseJSONValue::Array(entries)) = object.get(bot_key::REACTED_BY) else {
            return Ok(existing.clone());
        };
        let filled = Self::keeping_known_reactor_photos(entries, fresh);
        object.insert(bot_key::REACTED_BY.to_owned(), RiseJSONValue::Array(filled));
        Self::wire(object)
    }

    fn sender_name(sender: Option<&RiseJSONObject>) -> Option<String> {
        Self::string(sender?, &[bot_key::NAME]).filter(|name| !trimming_whitespaces_and_newlines(name).is_empty())
    }

    fn sender_logo(sender: Option<&RiseJSONObject>) -> Option<String> {
        let sender = sender?;
        if let Some(more) = Self::value(sender, &[bot_key::MORE]).and_then(RiseJSONValue::object_value)
            && let Some(logo) = Self::string(more, &[bot_key::LOGO])
        {
            return Some(logo);
        }
        Self::string(sender, &[bot_key::LOGO])
    }

    pub fn own_reaction_set(entries: &[RiseJSONValue], account_id: &str) -> HashSet<String> {
        entries
            .iter()
            .filter_map(|entry_value| {
                let entry = entry_value.object_value()?;
                let reaction = Self::string(entry, &[bot_key::REACTION])?;
                (Self::reacted_by_user_id(entry).as_deref() == Some(account_id)).then_some(reaction)
            })
            .collect()
    }

    pub fn own_reaction_timestamp(reaction: &str, entries: &[RiseJSONValue], account_id: &str) -> Option<f64> {
        entries
            .iter()
            .filter_map(|entry_value| {
                let entry = entry_value.object_value()?;
                if Self::string(entry, &[bot_key::REACTION]).as_deref() != Some(reaction)
                    || Self::reacted_by_user_id(entry).as_deref() != Some(account_id)
                {
                    return None;
                }
                Self::double(entry, &[message_key::CREATED_AT])
            })
            .min_by(f64::total_cmp)
    }

    pub fn reacted_by_entry_matches(entry_value: &RiseJSONValue, reaction: &str, user_id: &str) -> bool {
        let Some(entry) = entry_value.object_value() else {
            return false;
        };
        Self::string(entry, &[bot_key::REACTION]).as_deref() == Some(reaction)
            && Self::reacted_by_user_id(entry).as_deref() == Some(user_id)
    }

    pub fn reacted_by_user_id(entry: &RiseJSONObject) -> Option<String> {
        Self::value(entry, &[bot_key::SENDER])
            .and_then(RiseJSONValue::object_value)
            .and_then(|sender| Self::string(sender, &[bot_key::ID]))
            .or_else(|| Self::string(entry, &[message_key::USER_ID, message_key::SENDER_ID]))
    }

    pub fn value<'object>(object: &'object RiseJSONObject, keys: &[&str]) -> Option<&'object RiseJSONValue> {
        keys.iter().find_map(|key| object.get(*key).filter(|value| !matches!(value, RiseJSONValue::Null)))
    }

    pub fn string(object: &RiseJSONObject, keys: &[&str]) -> Option<String> {
        match Self::value(object, keys)? {
            RiseJSONValue::String(value) => Self::normalized(Some(value.as_str())),
            RiseJSONValue::Integer(value) => Some(value.to_string()),
            RiseJSONValue::UnsignedInteger(value) => Some(value.to_string()),
            _ => None,
        }
    }

    pub fn strings(object: &RiseJSONObject, keys: &[&str]) -> Vec<String> {
        let Some(RiseJSONValue::Array(values)) = Self::value(object, keys) else {
            return Vec::new();
        };
        values
            .iter()
            .filter_map(|value| match value {
                RiseJSONValue::String(value) => Self::normalized(Some(value.as_str())),
                _ => None,
            })
            .collect()
    }

    pub fn bool(object: &RiseJSONObject, keys: &[&str]) -> Option<bool> {
        match Self::value(object, keys)? {
            RiseJSONValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn double(object: &RiseJSONObject, keys: &[&str]) -> Option<f64> {
        match Self::value(object, keys)? {
            RiseJSONValue::Number(value) => Some(*value),
            RiseJSONValue::Integer(value) => Some(*value as f64),
            RiseJSONValue::UnsignedInteger(value) => Some(*value as f64),
            RiseJSONValue::String(value) => double_from_string(value),
            _ => None,
        }
    }

    pub fn normalized(raw: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw?);
        (!value.is_empty()).then(|| value.to_owned())
    }
}
