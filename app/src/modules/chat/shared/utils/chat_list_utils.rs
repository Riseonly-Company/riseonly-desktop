use std::cmp::Ordering;

use rise_canon::ChatMessageContentType;
use rise_canon::rise_key::{events, message, user};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::modules::sai_ws::sai_ws_payload_casing::ws_string;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageCallInfo, MessageContentType,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageCallInfoWireDTO;

pub enum ChatListUtils {}

impl ChatListUtils {
    pub fn sort(list: &[ChatInfo]) -> Vec<ChatInfo> {
        let mut sorted = list.to_vec();
        sorted.sort_by(|lhs, rhs| {
            if Self::ordered_before(lhs, rhs) {
                Ordering::Less
            } else if Self::ordered_before(rhs, lhs) {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        });
        sorted
    }

    fn ordered_before(lhs: &ChatInfo, rhs: &ChatInfo) -> bool {
        if lhs.is_pinned != rhs.is_pinned {
            return lhs.is_pinned && !rhs.is_pinned;
        }
        if lhs.is_pinned && rhs.is_pinned {
            let lp = lhs.pin_position.unwrap_or(i64::MAX);
            let rp = rhs.pin_position.unwrap_or(i64::MAX);
            if lp != rp {
                return lp < rp;
            }
        }
        Self::last_activity(lhs) > Self::last_activity(rhs)
    }

    /// the time the row prints and the engine orders by
    /// settings changes bump updated_at without a message and must not lift a chat above newer ones
    pub fn last_activity(chat: &ChatInfo) -> f64 {
        if let Some(sent) = chat.last_message.as_ref().map(|last| last.created_at)
            && sent > 0.0
        {
            return sent;
        }
        chat.updated_at
    }

    /// device local rows like unsent drafts and temp chats take their place by activity in an
    /// ordered server window
    /// below the pinned block not after it
    pub fn merging(local: &[ChatInfo], window: &[ChatInfo]) -> Vec<ChatInfo> {
        if local.is_empty() {
            return window.to_vec();
        }
        let mut ordered: Vec<&ChatInfo> = local.iter().collect();
        ordered.sort_by(|lhs, rhs| {
            Self::last_activity(rhs)
                .partial_cmp(&Self::last_activity(lhs))
                .unwrap_or(Ordering::Equal)
        });
        let mut floor = f64::INFINITY;
        let lowest_so_far: Vec<f64> = window
            .iter()
            .map(|row| {
                let activity = Self::last_activity(row);
                if !row.is_pinned && activity < floor {
                    floor = activity;
                }
                floor
            })
            .collect();
        let mut anchored: Vec<(usize, &ChatInfo)> = ordered
            .into_iter()
            .map(|chat| {
                let activity = Self::last_activity(chat);
                (lowest_so_far.partition_point(|lowest| !(*lowest < activity)), chat)
            })
            .collect();
        anchored.sort_by_key(|(anchor, _)| *anchor);
        let mut result = Vec::with_capacity(window.len() + anchored.len());
        let mut pending = anchored.into_iter().peekable();
        for (index, row) in window.iter().enumerate() {
            while let Some((_, chat)) = pending.next_if(|(anchor, _)| *anchor <= index) {
                result.push(chat.clone());
            }
            result.push(row.clone());
        }
        result.extend(pending.map(|(_, chat)| chat.clone()));
        result
    }

    pub fn map_inbound_message(payload: &Map<String, Value>) -> GetMessageMessage {
        let id = ws_string(payload, &[message::ID, message::MESSAGE_ID]).unwrap_or_else(|| Uuid::new_v4().uuid_string());
        let chat_id = ws_string(payload, &[message::CHAT_ID]).unwrap_or_default();
        let sender_id = ws_string(payload, &[message::SENDER_ID]).unwrap_or_default();
        let sender_name = Self::inbound_sender_name(payload);
        let content = ws_string(payload, &[message::CONTENT, events::TEXT, message::CAPTION]).unwrap_or_default();
        let content_type = ws_string(payload, &[message::CONTENT_TYPE]).unwrap_or_else(|| ChatMessageContentType::Text.raw_value().to_owned());
        let ts_raw = Self::double_value(payload.get(events::TIMESTAMP))
            .or_else(|| Self::double_value(payload.get(events::CREATED_AT)))
            .unwrap_or_else(|| RiseClock::now().time_interval_since_1970());
        let ts = if ts_raw > 1_000_000_000_000.0 { ts_raw / 1000.0 } else { ts_raw };

        let mut msg = GetMessageMessage::new(id);
        msg.chat_id = chat_id;
        msg.sender_id = sender_id;
        msg.sender_name = sender_name;
        msg.author_signature = ws_string(payload, &[message::AUTHOR_SIGNATURE]);
        msg.content_type = MessageContentType::from_raw_value(&content_type);
        if msg.content_type == MessageContentType::Call {
            msg.text = String::new();
            msg.call_info = Self::decode_call_info(payload.get(message::METADATA));
        } else {
            msg.text = content;
            msg.adopt_location_payload();
            msg.adopt_poll_payload();
        }
        msg.created_at = ts;
        msg
    }

    /// reads a float from json, any number works and a bool gives 0 or 1
    fn double_value(value: Option<&Value>) -> Option<f64> {
        match value? {
            Value::Number(number) => number.as_f64(),
            Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn decode_call_info(raw: Option<&Value>) -> Option<MessageCallInfo> {
        let raw = raw?;
        let data = match raw {
            Value::String(string) => Some(string.as_bytes().to_vec()),
            _ if JSONSerialization::is_valid_json_object(raw) => {
                JSONSerialization::data_with_json_object(raw, JSONWritingOptions::default())
            }
            _ => None,
        }?;
        let metadata: MessageCallInfoWireDTO = serde_json::from_slice(&data).ok()?;
        metadata.to_model()
    }

    pub fn inbound_sender_name(payload: &Map<String, Value>) -> String {
        let explicit = ws_string(payload, &[message::SENDER_NAME])
            .map(|value| trimming_whitespaces_and_newlines(&value).to_owned())
            .unwrap_or_default();
        if !explicit.is_empty() {
            return explicit;
        }

        let Some(sender) = AnyDictionaryBridge::dictionary(payload.get(events::SENDER_INFO)) else {
            return String::new();
        };
        let name = ws_string(&sender, &[user::NAME, user::DISPLAY_NAME])
            .map(|value| trimming_whitespaces_and_newlines(&value).to_owned())
            .unwrap_or_default();
        if !name.is_empty() {
            return name;
        }
        ws_string(&sender, &[user::TAG, message::USERNAME])
            .map(|value| trimming_whitespaces_and_newlines(&value).to_owned())
            .unwrap_or_default()
    }
}
