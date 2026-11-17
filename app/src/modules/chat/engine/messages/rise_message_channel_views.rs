use std::collections::BTreeMap;

use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_key::message as message_key;
use rise_canon::rise_request::message::MarkMessageAsViewed;
use rise_engine::RiseJSONValue;

use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;

impl RiseMessageWireCodec {
    /// channel view counts only grow so a late answer or a repeated count event never lowers the row
    pub fn raising_view_count(
        existing: &MessageWireDTO,
        payload: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<MessageWireDTO> {
        let raw = Self::double(payload, &[message_key::VIEW_COUNT, message_key::VIEWS_COUNT]);
        let Some(raw) = raw.filter(|raw| raw.is_finite() && *raw > existing.view_count.unwrap_or(0) as f64) else {
            return Ok(existing.clone());
        };
        let mut object = Self::object(existing)?;
        object.insert(bot_key::VIEW_COUNT.to_string(), RiseJSONValue::Integer(raw.round() as i64));
        Self::wire(object)
    }
}

impl RiseMessageRepository {
    pub async fn mark_channel_post_viewed(&self, server_id: &str, scope: &RiseMessageScope) -> anyhow::Result<()> {
        let response = self
            .remote
            .mark_viewed(MarkMessageAsViewed { chat_id: scope.chat_id.clone(), message_id: server_id.to_string() })
            .await?;
        self.validate(Some(response.success), Some(response.error.as_str()))?;
        let Some(views) = response.views_count else {
            return Ok(());
        };
        let Some(entity) = self.message_entity(server_id).await? else {
            return Ok(());
        };
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return Ok(());
        }
        let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
            return Ok(());
        };
        if views <= record.wire.view_count.unwrap_or(0) {
            return Ok(());
        }
        let payload = BTreeMap::from([(bot_key::VIEW_COUNT.to_string(), RiseJSONValue::Integer(views))]);
        let checkpoint = self
            .begin_patch(server_id, None, move |wire| RiseMessageWireCodec::raising_view_count(&wire, &payload))
            .await?;
        self.acknowledge(&checkpoint).await
    }
}
