use rise_canon::RiseEvent;
use rise_canon::rise_key::chat::UNREAD_COUNT;
use rise_engine::{RiseJSONValue, RiseWirePushEnvelope};

pub enum RiseNotifyUnreadRealtime {}

impl RiseNotifyUnreadRealtime {
    pub const EVENT_TYPE: &'static str = rise_events::name::NOTIFICATION_CREATED;

    pub fn decode_notification_id(envelope: &RiseWirePushEnvelope) -> Option<i64> {
        let data = envelope.data.data(false).ok()?;
        match RiseEvent::decode(&envelope.r#type, &data) {
            Ok(RiseEvent::NotificationCreated(created)) => Some(created.notification_id),
            _ => None,
        }
    }

    pub fn decode(envelope: &RiseWirePushEnvelope) -> Option<i64> {
        if let Ok(data) = envelope.data.data(false)
            && let Ok(RiseEvent::NotificationCreated(created)) = RiseEvent::decode(&envelope.r#type, &data)
        {
            return Some(created.unread_count.max(0));
        }
        let RiseJSONValue::Object(root) = &envelope.data else {
            return None;
        };
        match root.get(UNREAD_COUNT) {
            Some(RiseJSONValue::Integer(value)) => Some((*value).max(0)),
            Some(RiseJSONValue::UnsignedInteger(value)) => Some(i64::try_from(*value).unwrap_or(i64::MAX)),
            Some(RiseJSONValue::Number(value)) => Some((*value as i64).max(0)),
            Some(RiseJSONValue::String(value)) => value.parse::<i64>().ok().map(|parsed| parsed.max(0)),
            _ => None,
        }
    }
}
