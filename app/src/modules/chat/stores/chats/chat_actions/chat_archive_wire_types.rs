use rise_canon::rise_key::chat as key;
use rise_canon::{RiseWireKey, rise_decode, rise_view};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::chat_wire_types::RiseStoredChat;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;

#[derive(Clone, Debug)]
pub struct RiseStoredArchivedChatPage {
    pub chats: Option<Vec<RiseStoredChat>>,
    pub relative_id: Option<String>,
    pub limit: Option<i64>,
    pub is_have_more: Option<bool>,
    pub total_count: Option<i64>,
    pub total_unread_count: Option<i64>,
    pub unread_chats_count: Option<i64>,
    pub success: Option<bool>,
    pub error: Option<String>,
}

impl RiseStoredArchivedChatPage {
    /// every argument after chats is optional, pass none for those
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chats: Option<Vec<RiseStoredChat>>,
        relative_id: Option<String>,
        limit: Option<i64>,
        is_have_more: Option<bool>,
        total_count: Option<i64>,
        total_unread_count: Option<i64>,
        unread_chats_count: Option<i64>,
        success: Option<bool>,
        error: Option<String>,
    ) -> Self {
        Self {
            chats,
            relative_id,
            limit,
            is_have_more,
            total_count,
            total_unread_count,
            unread_chats_count,
            success,
            error,
        }
    }
}

impl<'de> Deserialize<'de> for RiseStoredArchivedChatPage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                chats: c.rise::<Vec<RiseStoredChat>>(key::CHATS)?,
                relative_id: c.rise::<String>(key::RELATIVE_ID)?,
                limit: c.rise::<i64>(key::LIMIT)?,
                is_have_more: c.rise::<bool>(key::IS_HAVE_MORE)?,
                total_count: c.rise::<i64>(key::TOTAL_COUNT)?,
                total_unread_count: c.rise::<i64>(key::TOTAL_UNREAD_COUNT)?,
                unread_chats_count: c.rise::<i64>(key::UNREAD_CHATS_COUNT)?,
                success: c.rise::<bool>(key::SUCCESS)?,
                error: c.rise::<String>(key::ERROR)?,
            })
        })
    }
}

impl Serialize for RiseStoredArchivedChatPage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        if let Some(chats) = &self.chats {
            container.serialize_entry(key::CHATS, chats)?;
        }
        if let Some(relative_id) = &self.relative_id {
            container.serialize_entry(&RiseWireKey::camel(key::RELATIVE_ID), relative_id)?;
        }
        if let Some(limit) = &self.limit {
            container.serialize_entry(key::LIMIT, limit)?;
        }
        if let Some(is_have_more) = &self.is_have_more {
            container.serialize_entry(&RiseWireKey::camel(key::IS_HAVE_MORE), is_have_more)?;
        }
        if let Some(total_count) = &self.total_count {
            container.serialize_entry(&RiseWireKey::camel(key::TOTAL_COUNT), total_count)?;
        }
        if let Some(total_unread_count) = &self.total_unread_count {
            container.serialize_entry(&RiseWireKey::camel(key::TOTAL_UNREAD_COUNT), total_unread_count)?;
        }
        if let Some(unread_chats_count) = &self.unread_chats_count {
            container.serialize_entry(&RiseWireKey::camel(key::UNREAD_CHATS_COUNT), unread_chats_count)?;
        }
        if let Some(success) = &self.success {
            container.serialize_entry(key::SUCCESS, success)?;
        }
        if let Some(error) = &self.error {
            container.serialize_entry(key::ERROR, error)?;
        }
        container.end()
    }
}

pub type ChatArchiveMutationGatewayDTO = rise_view::chat::ArchiveState;
