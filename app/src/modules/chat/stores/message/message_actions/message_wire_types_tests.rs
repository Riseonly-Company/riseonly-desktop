use std::collections::HashSet;

use gpui::{App, TestAppContext};
use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_key::message as message_key;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::chat::stores::message::message_actions::message_types::MessageDeleteBatchPlanner;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    DeleteMessagesBody, MessagePayloadCodec, MessageWireDTO,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

#[test]
fn encode_writes_id_under_bot_key() {
    let wire = MessageWireDTO {
        id: Some("message-1".to_owned()),
        ..MessageWireDTO::default()
    };

    let encoded = serde_json::to_value(&wire).expect("MessageWireDTO encodes to a JSON value");

    assert_eq!(encoded.get(bot_key::ID), Some(&Value::String("message-1".to_owned())));
}

#[test]
fn encode_omits_absent_id() {
    let encoded = serde_json::to_value(MessageWireDTO::default()).expect("MessageWireDTO encodes to a JSON value");

    assert_eq!(encoded.get(bot_key::ID), None);
}

#[test]
fn id_survives_encode_decode_round_trip() {
    let wire = MessageWireDTO {
        id: Some("message-1".to_owned()),
        server_id: Some("server-1".to_owned()),
        ..MessageWireDTO::default()
    };

    let encoded = serde_json::to_string(&wire).expect("MessageWireDTO encodes to JSON text");
    let decoded: MessageWireDTO = serde_json::from_str(&encoded).expect("encoded MessageWireDTO decodes back");

    assert_eq!(decoded.id.as_deref(), Some("message-1"));
    assert_eq!(decoded.server_id.as_deref(), Some("server-1"));
}

#[derive(Debug)]
enum TestDeleteError {
    Failed,
}

impl std::fmt::Display for TestDeleteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed => formatter.write_str("failed"),
        }
    }
}

impl std::error::Error for TestDeleteError {}

fn delete_row(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(entries.into_iter().map(|(entry_key, value)| (entry_key.to_owned(), value)).collect())
}

fn delete_message_rows(cache_key: &str, cx: &mut App) -> Vec<Map<String, Value>> {
    let payload = SaiWsResponseCache::current_payload(
        cache_key,
        SaiWsCachePreferredLayer::MemoryFirst,
        SaiWsDiskCachePolicy::PreserveExisting,
        cx,
    );
    let root = MessagePayloadCodec::dictionary(payload.as_ref()).unwrap_or_default();
    MessagePayloadCodec::array_of_dictionaries(root.get(message_key::MESSAGES))
}

fn delete_row_ids(rows: &[Map<String, Value>]) -> Vec<String> {
    rows.iter().filter_map(|row| row.get(message_key::ID).and_then(Value::as_str).map(str::to_owned)).collect()
}

fn install_delete_root(chat_id: &str, cache_key: &str, rows: Vec<Value>, cx: &mut App) {
    let root = delete_row(vec![(message_key::CHAT_ID, Value::from(chat_id)), (message_key::MESSAGES, Value::Array(rows))]);
    let installed = SaiWsResponseCache::install_optimistic_root(
        root,
        cache_key,
        true,
        false,
        SaiWsDiskCachePolicy::PreserveExisting,
        false,
        cx,
    );
    assert!(installed.is_ok());
}

#[test]
fn delete_planner_deduplicates_and_honours_backend_limit() {
    let mut ids: Vec<String> = (0..205).map(|index| format!("m-{index}")).collect();
    ids.extend(["m-0".to_owned(), " ".to_owned(), "m-204".to_owned()]);
    let chunks = MessageDeleteBatchPlanner::chunks(&ids);

    assert_eq!(chunks.iter().map(Vec::len).collect::<Vec<_>>(), vec![100, 100, 5]);
    assert_eq!(chunks.iter().flatten().collect::<HashSet<_>>().len(), 205);
}

#[gpui::test]
fn delete_resolves_server_id_before_removing_optimistic_row_and_restores_it_on_error(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let token = Uuid::new_v4().uuid_string();
        let chat_id = format!("test-delete-resolve-{token}");
        let temp_id = format!("temp_{token}");
        let server_id = Uuid::new_v4().uuid_string().to_lowercase();
        let service = MessageServicesStore::shared(cx);
        let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
        let marker = delete_row(vec![("value", Value::from("preserve"))]);
        let rows = vec![
            delete_row(vec![(message_key::ID, Value::from("before")), (message_key::CREATED_AT, Value::from(1.0))]),
            delete_row(vec![
                (message_key::ID, Value::from(temp_id.as_str())),
                ("server_id", Value::from(server_id.as_str())),
                (message_key::CONTENT, Value::from("delete me")),
                (message_key::CREATED_AT, Value::from(2.0)),
                ("custom_marker", marker),
            ]),
            delete_row(vec![(message_key::ID, Value::from("after")), (message_key::CREATED_AT, Value::from(3.0))]),
        ];
        install_delete_root(&chat_id, &cache_key, rows, cx);

        let (wire_ids, snapshot) = service
            .update(cx, |service, cx| service.prepare_delete_optimistic(std::slice::from_ref(&temp_id), &chat_id, cx));

        assert_eq!(wire_ids, vec![server_id.clone()]);
        assert_eq!(delete_row_ids(&delete_message_rows(&cache_key, cx)), vec!["before".to_owned(), "after".to_owned()]);

        let error = anyhow::Error::new(TestDeleteError::Failed);
        service.update(cx, |service, cx| service.delete_messages_error_handler(&error, &chat_id, snapshot, cx));

        let restored = delete_message_rows(&cache_key, cx);
        assert_eq!(delete_row_ids(&restored), vec!["before".to_owned(), temp_id.clone(), "after".to_owned()]);
        let marker = MessagePayloadCodec::dictionary(restored[1].get("custom_marker")).unwrap_or_default();
        assert_eq!(marker.get("value").and_then(Value::as_str), Some("preserve"));

        service.update(cx, |service, cx| service.flush_messages_disk_persistence(&chat_id, cx));
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn partial_delete_acknowledgement_restores_only_unconfirmed_rows(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let token = Uuid::new_v4().uuid_string();
        let chat_id = format!("test-delete-partial-{token}");
        let first_temp_id = format!("temp_first_{token}");
        let second_temp_id = format!("temp_second_{token}");
        let first_server_id = Uuid::new_v4().uuid_string().to_lowercase();
        let second_server_id = Uuid::new_v4().uuid_string().to_lowercase();
        let service = MessageServicesStore::shared(cx);
        let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
        let rows = vec![
            delete_row(vec![
                (message_key::ID, Value::from(first_temp_id.as_str())),
                ("server_id", Value::from(first_server_id.as_str())),
                (message_key::CREATED_AT, Value::from(1.0)),
            ]),
            delete_row(vec![
                (message_key::ID, Value::from(second_temp_id.as_str())),
                ("server_id", Value::from(second_server_id.as_str())),
                (message_key::CREATED_AT, Value::from(2.0)),
            ]),
            delete_row(vec![
                (message_key::ID, Value::from("unaffected-tail")),
                (message_key::CREATED_AT, Value::from(3.0)),
            ]),
        ];
        install_delete_root(&chat_id, &cache_key, rows, cx);

        let (_, snapshot) = service.update(cx, |service, cx| {
            service.prepare_delete_optimistic(&[first_temp_id.clone(), second_temp_id.clone()], &chat_id, cx)
        });
        let acknowledgement = delete_row(vec![
            ("deleted_message_ids", Value::from(vec![first_server_id.clone()])),
            ("error", Value::from("")),
            ("success", Value::from(true)),
        ]);
        service.update(cx, |service, cx| {
            service.delete_messages_success_handler(Some(&acknowledgement), &chat_id, snapshot, cx);
        });

        let remaining = delete_message_rows(&cache_key, cx);
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].get(message_key::ID).and_then(Value::as_str), Some(second_temp_id.as_str()));
        assert_eq!(remaining[0].get("server_id").and_then(Value::as_str), Some(second_server_id.as_str()));
        assert_eq!(remaining[1].get(message_key::ID).and_then(Value::as_str), Some("unaffected-tail"));

        service.update(cx, |service, cx| service.flush_messages_disk_persistence(&chat_id, cx));
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[test]
fn delete_for_everyone_uses_expected_wire_key() {
    let body = DeleteMessagesBody {
        for_everyone: Some(true),
        message_ids: Some(vec!["message-id".to_owned()]),
        ..DeleteMessagesBody::default()
    };
    let json = serde_json::to_value(&body).unwrap_or(Value::Null);

    assert_eq!(json.get("for_everyone").and_then(Value::as_bool), Some(true));
    assert_eq!(json.get("message_ids"), Some(&Value::from(vec!["message-id".to_owned()])));
}
