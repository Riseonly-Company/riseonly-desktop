use rise_engine::MethodDescriptor;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SUBSCRIBE: MethodDescriptor = MethodDescriptor::mutation("gateway", "presence_subscribe");
pub const UNSUBSCRIBE: MethodDescriptor =
    MethodDescriptor::mutation("gateway", "presence_unsubscribe");
pub const SET_AWAY: MethodDescriptor = MethodDescriptor::mutation("gateway", "set_away");
pub const ENTER_CHAT: MethodDescriptor = MethodDescriptor::mutation("gateway", "enter_chat");
pub const LEAVE_CHAT: MethodDescriptor = MethodDescriptor::mutation("gateway", "leave_chat");

pub const ONLINE_STATUS_EVENT: &str = "user_online_status_changed";
pub const IN_CHAT_EVENT: &str = "in_chat_update";

#[derive(Clone, Debug, Serialize)]
pub struct SubscribeRequest {
    pub users: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SetAwayRequest {
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChatPresenceRequest {
    pub chat_id: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PresenceUpdate {
    pub user_id: String,
    pub is_online: Option<bool>,
    pub last_seen_ms: Option<i64>,
    pub in_chat_id: Option<String>,
    pub clears_in_chat: bool,
}

pub fn decode_online_status(payload: &Value) -> Option<PresenceUpdate> {
    let user_id = string(payload, &["user_id", "userId"])?;

    let is_online = payload
        .get("is_online")
        .and_then(Value::as_bool)
        .or_else(|| {
            payload
                .get("status")
                .and_then(Value::as_str)
                .map(|status| status == "online")
        })
        .unwrap_or(false);

    Some(PresenceUpdate {
        user_id,
        is_online: Some(is_online),
        last_seen_ms: number(payload, &["last_seen", "lastSeen"]),
        in_chat_id: None,
        clears_in_chat: !is_online,
    })
}

pub fn decode_in_chat(payload: &Value) -> Option<PresenceUpdate> {
    let user_id = string(payload, &["user_id", "userId"])?;
    let in_chat = payload
        .get("in_chat")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let chat_id = string(payload, &["chat_id", "chatId"]).filter(|id| !id.is_empty());

    Some(PresenceUpdate {
        user_id,
        is_online: None,
        last_seen_ms: None,
        in_chat_id: if in_chat { chat_id } else { None },
        clears_in_chat: !in_chat,
    })
}

fn string(payload: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
}

fn number(payload: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        payload.get(*key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_f64().map(|seconds| seconds as i64))
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
    })
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct PresenceAck {
    pub success: Option<bool>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_presence_verb_is_a_gateway_method() {
        for descriptor in [SUBSCRIBE, UNSUBSCRIBE, SET_AWAY, ENTER_CHAT, LEAVE_CHAT] {
            assert_eq!(
                descriptor.service, "gateway",
                "presence-service has no client-facing RPC; the gateway owns these"
            );
        }
    }

    #[test]
    fn an_online_push_reads_both_key_styles() {
        let snake = decode_online_status(&json!({"user_id": "u1", "is_online": true})).unwrap();
        assert_eq!(snake.user_id, "u1");
        assert_eq!(snake.is_online, Some(true));

        let camel = decode_online_status(&json!({"userId": "u1", "is_online": true})).unwrap();
        assert_eq!(camel.user_id, "u1");
    }

    #[test]
    fn the_older_status_string_still_reads_as_online() {
        let update = decode_online_status(&json!({"user_id": "u1", "status": "online"})).unwrap();
        assert_eq!(update.is_online, Some(true));

        let offline = decode_online_status(&json!({"user_id": "u1", "status": "away"})).unwrap();
        assert_eq!(offline.is_online, Some(false));
    }

    #[test]
    fn going_offline_clears_the_conversation_the_user_was_in() {
        let update = decode_online_status(&json!({"user_id": "u1", "is_online": false})).unwrap();
        assert!(
            update.clears_in_chat,
            "otherwise a closed app keeps showing an in-this-chat marker"
        );

        let online = decode_online_status(&json!({"user_id": "u1", "is_online": true})).unwrap();
        assert!(!online.clears_in_chat);
    }

    #[test]
    fn last_seen_reads_as_a_number_a_float_or_a_string() {
        for raw in [
            json!(1_700_000_000_000i64),
            json!(1.7e12),
            json!("1700000000000"),
        ] {
            let update = decode_online_status(&json!({"user_id": "u1", "last_seen": raw})).unwrap();
            assert_eq!(update.last_seen_ms, Some(1_700_000_000_000));
        }
    }

    #[test]
    fn an_in_chat_push_carries_the_conversation_only_while_it_is_true() {
        let entered =
            decode_in_chat(&json!({"user_id": "u1", "in_chat": true, "chat_id": "c1"})).unwrap();
        assert_eq!(entered.in_chat_id.as_deref(), Some("c1"));
        assert!(!entered.clears_in_chat);

        let left = decode_in_chat(&json!({"user_id": "u1", "in_chat": false})).unwrap();
        assert_eq!(left.in_chat_id, None);
        assert!(left.clears_in_chat);
    }

    #[test]
    fn an_in_chat_true_with_no_chat_id_is_not_a_conversation() {
        let update = decode_in_chat(&json!({"user_id": "u1", "in_chat": true})).unwrap();
        assert_eq!(update.in_chat_id, None);
    }

    #[test]
    fn a_push_with_no_user_is_dropped_rather_than_applied_to_an_empty_id() {
        assert!(decode_online_status(&json!({"is_online": true})).is_none());
        assert!(decode_in_chat(&json!({"in_chat": true, "chat_id": "c1"})).is_none());
        assert!(decode_online_status(&json!({"user_id": "", "is_online": true})).is_none());
    }
}
