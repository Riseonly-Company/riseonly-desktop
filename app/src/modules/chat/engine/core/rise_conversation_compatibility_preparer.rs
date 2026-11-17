use std::sync::LazyLock;

use rise_engine::RiseJSONValue;
use serde_json::Value;

use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationMergeResult, SaiWsPaginationMerger, SaiWsPaginationOptions,
};
use crate::modules::chat::engine::core::rise_conversation_compatibility_bridge::{
    RiseConversationCompatibilityBridge, RiseConversationCompatibilityBridgeError,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;

pub enum RiseConversationCompatibilityJSON {}

impl RiseConversationCompatibilityJSON {
    pub fn value(raw: &Value) -> Result<RiseJSONValue, RiseConversationCompatibilityBridgeError> {
        Ok(RiseJSONValue::from(raw.clone()))
    }
}

pub struct RiseConversationPreparedCompatibilityResponse {
    pub raw: Value,
    pub fulfilled: Value,
    pub pagination: Option<SaiWsPaginationMergeResult>,
    pub merged_value: Option<RiseJSONValue>,
    pub created_chat: Option<RiseStoredChat>,
}

#[derive(Default)]
pub struct RiseConversationCompatibilityPreparer;

static SHARED: LazyLock<RiseConversationCompatibilityPreparer> = LazyLock::new(Default::default);

impl RiseConversationCompatibilityPreparer {
    pub fn shared() -> &'static Self {
        &SHARED
    }

    pub fn prepare(
        &self,
        response: &RiseJSONValue,
        existing: Option<&RiseJSONValue>,
        pagination: Option<&SaiWsPaginationOptions>,
        items_path: &str,
        item_id_path: Option<&str>,
        created_chat_request: Option<&RiseJSONValue>,
    ) -> anyhow::Result<RiseConversationPreparedCompatibilityResponse> {
        let raw = Self::response_object(response)?;
        let result = match pagination {
            Some(pagination) => {
                let existing_payload = existing.map(Self::response_object).transpose()?;
                Some(SaiWsPaginationMerger::merge(
                    existing_payload.as_ref(),
                    Some(&raw),
                    pagination,
                    items_path,
                    item_id_path,
                )?)
            }
            None => None,
        };
        let fulfilled = match &result {
            Some(result) => Value::Object(result.root.clone()),
            None => raw.clone(),
        };
        let merged_value =
            if result.is_some() { Some(RiseConversationCompatibilityJSON::value(&fulfilled)?) } else { None };
        let created_chat = created_chat_request
            .and_then(|request| RiseConversationCompatibilityBridge::created_chat_wire(response, request));
        Ok(RiseConversationPreparedCompatibilityResponse {
            raw,
            fulfilled,
            pagination: result,
            merged_value,
            created_chat,
        })
    }

    fn response_object(value: &RiseJSONValue) -> Result<Value, RiseConversationCompatibilityBridgeError> {
        value
            .data(false)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .ok_or(RiseConversationCompatibilityBridgeError::InvalidResponseBody)
    }
}
