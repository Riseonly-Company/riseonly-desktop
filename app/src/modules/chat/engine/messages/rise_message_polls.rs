use std::collections::BTreeMap;

use rise_canon::{RiseRequestJSON, rise_key, rise_request};
use rise_engine::RiseJSONValue;

use crate::modules::chat::engine::core::rise_conversation_store_layout::RiseConversationStoreLayout;
use crate::modules::chat::engine::messages::rise_message_contracts::RiseMessageRepositoryError;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;

impl RiseMessageWireCodec {
    /// puts fresh poll content on the row
    /// poll_updated has no own choice so answers stay from the stored poll
    /// chosen_option_ids frame wins, even empty
    pub fn setting_poll(existing: &MessageWireDTO, content: &str, keeps_own_choice: bool) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        let mut next = content.to_owned();
        if keeps_own_choice
            && let Some(mut merged) = Self::poll_object(content)
            && !merged.contains_key(rise_key::message_poll::CHOSEN_OPTION_IDS)
            && let Some(stored) =
                Self::poll_object(&Self::string(&object, &[rise_key::message::CONTENT]).unwrap_or_default())
        {
            match stored.get(rise_key::message_poll::CHOSEN_OPTION_IDS) {
                Some(chosen) => {
                    merged.insert(rise_key::message_poll::CHOSEN_OPTION_IDS.to_owned(), chosen.clone());
                }
                None => {
                    merged.remove(rise_key::message_poll::CHOSEN_OPTION_IDS);
                }
            }
            if let Ok(data) = RiseJSONValue::Object(merged).data(true)
                && let Ok(text) = String::from_utf8(data)
            {
                next = text;
            }
        }
        object.insert(rise_key::message::CONTENT.to_owned(), RiseJSONValue::String(next));
        Self::wire(&object)
    }

    fn poll_object(content: &str) -> Option<BTreeMap<String, RiseJSONValue>> {
        match RiseJSONValue::decode(content.as_bytes()).ok()? {
            RiseJSONValue::Object(object) => Some(object),
            _ => None,
        }
    }

    pub fn poll_content(payload: &BTreeMap<String, RiseJSONValue>) -> Option<String> {
        Self::string(payload, &[rise_key::message::CONTENT])
    }
}

impl RiseMessageRepository {
    pub async fn vote_poll(&self, local_id: &str, option_ids: &[String]) -> anyhow::Result<()> {
        let Some(entity) = self.message_entity(local_id).await? else {
            return Err(RiseMessageRepositoryError::MessageNotFound(local_id.to_owned()).into());
        };
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return Err(RiseMessageRepositoryError::MessageNotFound(local_id.to_owned()).into());
        }
        let record = RiseMessageWireCodec::record(&entity.data)?;
        let server_id = RiseConversationStoreLayout::canonical_message_server_id(&record.wire);
        let chat_id = record.wire.chat_id.clone().filter(|chat_id| !chat_id.is_empty());
        let (Some(server_id), Some(chat_id)) = (server_id, chat_id) else {
            return Err(RiseMessageRepositoryError::MissingServerID(entity.local_id).into());
        };
        let response = self
            .remote
            .vote_poll(rise_request::message::VotePoll::new(
                chat_id,
                server_id,
                option_ids.iter().map(|option_id| RiseRequestJSON::String(option_id.clone())).collect(),
            ))
            .await?;
        let content = response.content;
        if content.is_empty() {
            return Ok(());
        }
        let checkpoint = self
            .begin_patch(local_id, None, move |wire: &MessageWireDTO| RiseMessageWireCodec::setting_poll(wire, &content, false))
            .await?;
        self.acknowledge(&checkpoint).await
    }
}
