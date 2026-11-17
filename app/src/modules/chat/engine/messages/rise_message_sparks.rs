use std::collections::BTreeMap;

use rise_canon::{rise_key, rise_request};
use rise_engine::RiseJSONValue;

use crate::modules::chat::engine::core::rise_conversation_store_layout::RiseConversationStoreLayout;
use crate::modules::chat::engine::messages::rise_message_contracts::RiseMessageRepositoryError;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;
use crate::modules::sparks::engine::rise_sparks_engine_models::{RiseStoredSparksSendResult, RiseStoredSparksTop};

pub type SendMessageSparksBody = rise_request::message::SendMessageSparks;
pub type GetMessageSparksBody = rise_request::message::GetMessageSparks;

impl RiseMessageWireCodec {
    pub fn adding_sparks(existing: &MessageWireDTO, delta: i64) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        object.insert(
            rise_key::bot::SPARKS_TOTAL.to_owned(),
            RiseJSONValue::Integer((existing.sparks_total.unwrap_or(0) + delta).max(0)),
        );
        object.insert(
            rise_key::bot::MY_SPARKS.to_owned(),
            RiseJSONValue::Integer((existing.my_sparks.unwrap_or(0) + delta).max(0)),
        );
        Self::wire(&object)
    }

    pub fn settling_sparks(
        existing: &MessageWireDTO,
        total: i64,
        mine: i64,
        pending: i64,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        let others = (existing.sparks_total.unwrap_or(0) - pending).max(0);
        object.insert(rise_key::bot::SPARKS_TOTAL.to_owned(), RiseJSONValue::Integer(total.max(others)));
        object.insert(rise_key::bot::MY_SPARKS.to_owned(), RiseJSONValue::Integer(mine.max(0)));
        Self::wire(&object)
    }

    pub fn raising_sparks(
        existing: &MessageWireDTO,
        payload: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<MessageWireDTO> {
        let mut object = Self::object(existing)?;
        if let Some(total) = Self::sparks_integer(Self::value(payload, &[rise_key::message::SPARKS_TOTAL]))
            && total > existing.sparks_total.unwrap_or(0)
        {
            object.insert(rise_key::bot::SPARKS_TOTAL.to_owned(), RiseJSONValue::Integer(total));
        }
        if let Some(mine) = Self::sparks_integer(Self::value(payload, &[rise_key::message::MY_SPARKS]))
            && mine > existing.my_sparks.unwrap_or(0)
        {
            object.insert(rise_key::bot::MY_SPARKS.to_owned(), RiseJSONValue::Integer(mine));
        }
        Self::wire(&object)
    }

    fn sparks_integer(value: Option<&RiseJSONValue>) -> Option<i64> {
        match value? {
            RiseJSONValue::Integer(number) => Some(*number),
            RiseJSONValue::UnsignedInteger(number) => i64::try_from(*number).ok(),
            RiseJSONValue::Number(number) => {
                let rounded = number.round();
                let is_exact = rounded >= i64::MIN as f64 && rounded < i64::MAX as f64;
                is_exact.then_some(rounded as i64)
            }
            RiseJSONValue::String(text) => text.parse().ok(),
            _ => None,
        }
    }
}

impl RiseMessageRepository {
    pub async fn apply_sparks_delta(&self, local_id: &str, delta: i64) -> anyhow::Result<()> {
        if delta == 0 {
            return Ok(());
        }
        let checkpoint = self
            .begin_patch(local_id, None, move |wire: &MessageWireDTO| RiseMessageWireCodec::adding_sparks(wire, delta))
            .await?;
        self.acknowledge(&checkpoint).await
    }

    pub async fn send_sparks(
        &self,
        local_id: &str,
        amount: i64,
        anonymous: bool,
        idempotency_key: &str,
    ) -> anyhow::Result<RiseStoredSparksSendResult> {
        let server_id = match self.sparks_server_id(local_id).await {
            Ok(server_id) => server_id,
            Err(error) => {
                let _ = self.apply_sparks_delta(local_id, -amount).await;
                return Err(error);
            }
        };
        match self.settle_sparks_send(local_id, amount, anonymous, idempotency_key, server_id).await {
            Ok(response) => Ok(response),
            Err(error) => {
                let _ = self.apply_sparks_delta(local_id, -amount).await;
                Err(error)
            }
        }
    }

    async fn settle_sparks_send(
        &self,
        local_id: &str,
        amount: i64,
        anonymous: bool,
        idempotency_key: &str,
        server_id: String,
    ) -> anyhow::Result<RiseStoredSparksSendResult> {
        let response = self
            .remote
            .send_sparks(SendMessageSparksBody {
                amount: Some(amount),
                anonymous: Some(anonymous),
                idempotency_key: Some(idempotency_key.to_owned()),
                message_id: Some(server_id),
            })
            .await?;
        let total = response.sparks_total;
        let mine = response.my_sparks;
        let checkpoint = self
            .begin_patch(local_id, None, move |wire: &MessageWireDTO| {
                RiseMessageWireCodec::settling_sparks(wire, total, mine, amount)
            })
            .await?;
        self.acknowledge(&checkpoint).await?;
        Ok(response)
    }

    pub async fn sparks_top(&self, local_id: &str, limit: i64) -> anyhow::Result<RiseStoredSparksTop> {
        let server_id = self.sparks_server_id(local_id).await?;
        self.remote
            .sparks_top(GetMessageSparksBody { limit: Some(limit), message_id: Some(server_id) })
            .await
    }

    async fn sparks_server_id(&self, local_id: &str) -> anyhow::Result<String> {
        let entity = self
            .message_entity(local_id)
            .await?
            .filter(|entity| !RiseMessageWireCodec::is_tombstone(&entity.data))
            .ok_or_else(|| RiseMessageRepositoryError::MessageNotFound(local_id.to_owned()))?;
        let record = RiseMessageWireCodec::record(&entity.data)?;
        let server_id = RiseConversationStoreLayout::canonical_message_server_id(&record.wire)
            .ok_or_else(|| RiseMessageRepositoryError::MissingServerID(entity.local_id.clone()))?;
        Ok(server_id)
    }
}
