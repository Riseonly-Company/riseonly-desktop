use rise_canon::rise_key::chat as key;
use rise_canon::{RiseWireKey, rise_decode, rise_response, rise_view};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::chat_wire_types::RiseStoredChat;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;

#[derive(Clone, Debug)]
pub struct RiseStoredMessageRequestPage {
    pub chats: Option<Vec<RiseStoredChat>>,
    pub has_more: Option<bool>,
    pub relative_id: Option<String>,
    pub total_count: Option<i64>,
}

impl<'de> Deserialize<'de> for RiseStoredMessageRequestPage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                chats: c.rise::<Vec<RiseStoredChat>>(key::CHATS)?,
                has_more: c.rise::<bool>(key::HAS_MORE)?,
                relative_id: c.rise::<String>(key::RELATIVE_ID)?,
                total_count: c.rise::<i64>(key::TOTAL_COUNT)?,
            })
        })
    }
}

impl Serialize for RiseStoredMessageRequestPage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(chats) = &self.chats {
            container.serialize_entry(key::CHATS, chats)?;
        }
        if let Some(has_more) = &self.has_more {
            container.serialize_entry(&RiseWireKey::camel(key::HAS_MORE), has_more)?;
        }
        if let Some(relative_id) = &self.relative_id {
            container.serialize_entry(&RiseWireKey::camel(key::RELATIVE_ID), relative_id)?;
        }
        if let Some(total_count) = &self.total_count {
            container.serialize_entry(&RiseWireKey::camel(key::TOTAL_COUNT), total_count)?;
        }
        container.end()
    }
}

pub type ResolveMessageRequestGatewayDTO = rise_view::chat::MessageRequestResolved;

pub type DeleteAllMessageRequestsGatewayDTO = rise_response::chat::DeleteAllMessageRequests;
