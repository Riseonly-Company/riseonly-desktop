use std::collections::BTreeMap;

use rise_canon::rise_key::{events, message, payload};
use rise_chat::MessageContentType as ChatMessageContentType;
use rise_core::foundation::foundation_crypto::{InsecureSha1, hex_string};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::RiseJSONValue;
use rise_rtc::{CallMediaMode, RtcWireEnum};

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;

/// a finished direct call, shaped without rtc models so two domains of the account can exchange it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseCallHistoryRecord {
    pub call_id: String,
    pub chat_id: String,
    pub caller_user_id: String,
    pub callee_user_id: String,
    pub media_mode: String,
    pub state: String,
    pub started_at_ms: i64,
    pub answered_at_ms: Option<i64>,
    pub ended_at_ms: i64,
    pub ended_by_user_id: Option<String>,
    pub end_reason: Option<String>,
}

const URL_NAMESPACE_BYTES: [u8; 16] =
    [0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8];
const CALL_HISTORY_SCHEMA_VERSION: i64 = 1;

pub enum RiseCallHistoryMessage {}

impl RiseCallHistoryMessage {
    pub const IDENTIFIER_PREFIX: &str = "riseonly:call-history:v1:";

    pub fn message_id(call_id: &str) -> Option<String> {
        let call_id = trimming_whitespaces_and_newlines(call_id).to_lowercase();
        if call_id.is_empty() {
            return None;
        }
        let name = format!("{}{}", Self::IDENTIFIER_PREFIX, call_id);
        Some(Self::uuid_v5(&URL_NAMESPACE_BYTES, name.as_bytes()))
    }

    pub fn wire(record: &RiseCallHistoryRecord) -> anyhow::Result<Option<MessageWireDTO>> {
        let Some(message_id) = Self::message_id(&record.call_id) else {
            return Ok(None);
        };
        let chat_id = Self::normalized(Some(&record.chat_id));
        let caller = Self::normalized(Some(&record.caller_user_id));
        let callee = Self::normalized(Some(&record.callee_user_id));
        let (Some(chat_id), Some(caller), Some(callee)) = (chat_id, caller, callee) else {
            return Ok(None);
        };
        if record.media_mode != CallMediaMode::Audio.wire() && record.media_mode != CallMediaMode::Video.wire() {
            return Ok(None);
        }
        if record.started_at_ms <= 0 || record.ended_at_ms <= 0 {
            return Ok(None);
        }

        let answered_at_ms = record.answered_at_ms.filter(|answered| *answered > 0);
        let duration_ms = answered_at_ms.map_or(0, |answered| (record.ended_at_ms - answered).max(0));
        let mut metadata = BTreeMap::from([
            (payload::SCHEMA_VERSION.to_string(), RiseJSONValue::Integer(CALL_HISTORY_SCHEMA_VERSION)),
            (events::CALL_ID.to_string(), RiseJSONValue::String(record.call_id.clone())),
            (events::STATE.to_string(), RiseJSONValue::String(record.state.clone())),
            (events::MEDIA_MODE.to_string(), RiseJSONValue::String(record.media_mode.clone())),
            (events::CALLER_USER_ID.to_string(), RiseJSONValue::String(caller.clone())),
            (events::CALLEE_USER_ID.to_string(), RiseJSONValue::String(callee)),
            (message::STARTED_AT_MS.to_string(), RiseJSONValue::Integer(record.started_at_ms)),
            (events::ENDED_AT_MS.to_string(), RiseJSONValue::Integer(record.ended_at_ms)),
            (events::DURATION_MS.to_string(), RiseJSONValue::Integer(duration_ms)),
        ]);
        if let Some(answered_at_ms) = answered_at_ms {
            metadata.insert(events::ANSWERED_AT_MS.to_string(), RiseJSONValue::Integer(answered_at_ms));
        }
        if let Some(ended_by_user_id) = Self::normalized(record.ended_by_user_id.as_deref()) {
            metadata.insert(events::ENDED_BY_USER_ID.to_string(), RiseJSONValue::String(ended_by_user_id));
        }
        if let Some(end_reason) = Self::normalized(record.end_reason.as_deref()) {
            metadata.insert(events::END_REASON.to_string(), RiseJSONValue::String(end_reason));
        }

        let object = BTreeMap::from([
            (events::ID.to_string(), RiseJSONValue::String(message_id.clone())),
            (RiseLocalKey::SERVER_ID.to_string(), RiseJSONValue::String(message_id)),
            (events::CHAT_ID.to_string(), RiseJSONValue::String(chat_id)),
            (events::SENDER_ID.to_string(), RiseJSONValue::String(caller)),
            (events::CONTENT.to_string(), RiseJSONValue::String(String::new())),
            (events::CONTENT_TYPE.to_string(), RiseJSONValue::String(ChatMessageContentType::Call.wire().to_string())),
            (events::CREATED_AT.to_string(), RiseJSONValue::Integer(record.ended_at_ms)),
            (events::TIMESTAMP.to_string(), RiseJSONValue::Integer(record.ended_at_ms)),
            (events::IS_SYSTEM_MESSAGE.to_string(), RiseJSONValue::Bool(false)),
            (events::METADATA.to_string(), RiseJSONValue::Object(metadata)),
        ]);
        RiseMessageWireCodec::wire(object).map(Some)
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        (!value.is_empty()).then(|| value.to_string())
    }

    fn uuid_v5(namespace: &[u8], name: &[u8]) -> String {
        let mut digest = InsecureSha1::hash(&[namespace, name].concat());
        digest[6] = (digest[6] & 0x0f) | 0x50;
        digest[8] = (digest[8] & 0x3f) | 0x80;
        let hex = hex_string(&digest[..16]);
        format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
    }
}
