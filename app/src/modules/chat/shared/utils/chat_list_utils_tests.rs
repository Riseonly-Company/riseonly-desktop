use std::collections::HashSet;

use gpui::TestAppContext;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::chat::shared::utils::chat_list_utils::ChatListUtils;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChatPreview;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageThreadOrder;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MarkChatAsReadBody;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("test payload is valid JSON")
}

fn map(text: &str) -> Map<String, Value> {
    match value(text) {
        Value::Object(row) => row,
        _ => panic!("test payload is a JSON object"),
    }
}

fn ids(values: &[&str]) -> HashSet<String> {
    values.iter().map(|id| (*id).to_owned()).collect()
}

fn last_message_read(container: &Map<String, Value>) -> Option<bool> {
    container.get("last_message")?.get("is_read")?.as_bool()
}

fn cached_rows(cache_key: &str, cx: &mut gpui::App) -> Vec<Value> {
    let root = SaiWsResponseCache::memory_only(cache_key, cx).expect("cache holds the page");
    root.get("messages").and_then(Value::as_array).expect("page holds messages").clone()
}

fn row_is_read(rows: &[Value], id: &str) -> Option<bool> {
    rows.iter().find(|row| row.get("id").and_then(Value::as_str) == Some(id))?.get("is_read")?.as_bool()
}

fn topic_last_message(cache_key: &str, topic_id: &str, cx: &mut gpui::App) -> Option<Value> {
    let root = SaiWsResponseCache::memory_only(cache_key, cx).expect("cache holds the topics");
    let topics = root.get("topics").and_then(Value::as_array).expect("page holds topics").clone();
    let topic = topics
        .into_iter()
        .find(|topic| topic.get("topic_id").and_then(Value::as_str) == Some(topic_id))
        .expect("topic is cached");
    topic.get("last_message").filter(|last_message| last_message.is_object()).cloned()
}

#[test]
fn whole_chat_read_omits_topic_while_general_preserves_empty_topic() {
    let whole_chat = SaiJSONCodec::encode(&MarkChatAsReadBody::new("chat-1".to_owned())).expect("body encodes");
    let mut general_body = MarkChatAsReadBody::new("chat-1".to_owned());
    general_body.topic_id = Some(String::new());
    let general = SaiJSONCodec::encode(&general_body).expect("body encodes");

    assert!(!whole_chat.contains_key("topic_id"));
    assert_eq!(general.get("topic_id").and_then(Value::as_str), Some(""));
}

#[test]
fn realtime_preview_uses_nested_sender_name() {
    let payload = map(r#"{"sender_info": {"name": "  Alice  ", "tag": "alice_tag"}}"#);

    assert_eq!(ChatListUtils::inbound_sender_name(&payload), "Alice");
}

#[test]
fn realtime_preview_falls_back_to_nested_sender_tag() {
    let payload = map(r#"{"sender_info": {"name": "   ", "tag": "  alice_tag  "}}"#);

    assert_eq!(ChatListUtils::inbound_sender_name(&payload), "alice_tag");
}

#[test]
fn loaded_last_message_preview_preserves_read_state() {
    let payload = value(
        r#"{
            "id": "message-1",
            "topic_id": "",
            "sender_id": "me",
            "content": "hello",
            "content_type": "text",
            "timestamp": 1.0,
            "is_read": true
        }"#,
    );
    let wire = SaiJSONCodec::decode::<RiseStoredChatPreview>(Some(&payload)).expect("preview decodes");

    assert!(wire.to_message().is_read);
    assert_eq!(wire.to_message().topic_id.as_deref(), Some(""));
}

#[test]
fn scoped_preview_receipt_normalizes_general_and_rejects_other_topics() {
    let mut legacy_container =
        map(r#"{"last_message": {"id": "legacy-message", "sender_id": "me", "created_at": 1.0, "is_read": false}}"#);
    let mut general_container = map(
        r#"{"last_message": {"id": "general-message", "sender_id": "me", "topic_id": "", "created_at": 1.0, "is_read": false}}"#,
    );
    let mut topic_container = map(
        r#"{"last_message": {"id": "topic-message", "sender_id": "me", "topic_id": "topic-1", "created_at": 1.0, "is_read": false}}"#,
    );

    ChatServicesStore::mark_raw_last_message_read(&mut legacy_container, "me", Some(""), 2.0, None, None);
    ChatServicesStore::mark_raw_last_message_read(&mut general_container, "me", Some(""), 2.0, None, None);
    ChatServicesStore::mark_raw_last_message_read(&mut topic_container, "me", Some("topic-2"), 2.0, None, None);

    assert_eq!(last_message_read(&legacy_container), Some(true));
    assert_eq!(last_message_read(&general_container), Some(true));
    assert_eq!(last_message_read(&topic_container), Some(false));
}

#[test]
fn millisecond_receipt_cutoff_can_mark_a_closed_cache_preview() {
    let cutoff = MessageThreadOrder::timestamp_seconds(&map(r#"{"created_at": 1720000000000}"#)).expect("cutoff reads");
    let mut preview_container = map(
        r#"{"last_message": {"id": "topic-message", "sender_id": "me", "topic_id": "topic-1", "created_at": 1720000000.0, "is_read": false}}"#,
    );

    ChatServicesStore::mark_raw_last_message_read(&mut preview_container, "me", Some("topic-1"), cutoff, None, None);

    assert_eq!(cutoff, 1_720_000_000.0);
    assert_eq!(last_message_read(&preview_container), Some(true));
}

#[test]
fn exact_millisecond_receipt_only_marks_captured_preview_ids() {
    let mut included =
        map(r#"{"last_message": {"id": "captured-message", "sender_id": "me", "created_at": 2.0, "is_read": false}}"#);
    let mut late =
        map(r#"{"last_message": {"id": "late-message", "sender_id": "me", "created_at": 2.0, "is_read": false}}"#);
    let captured_ids = ids(&["captured-message"]);

    ChatServicesStore::mark_raw_last_message_read(&mut included, "me", None, 2.0, Some(&captured_ids), None);
    ChatServicesStore::mark_raw_last_message_read(&mut late, "me", None, 2.0, Some(&captured_ids), None);

    assert_eq!(last_message_read(&included), Some(true));
    assert_eq!(last_message_read(&late), Some(false));
}

#[test]
fn legacy_millisecond_receipt_remains_inclusive_without_exact_ids() {
    assert!(MessageServicesStore::is_within_read_boundary("legacy-message", 2.0, 2.0, None, None));
}

#[test]
fn general_receipt_matches_preview_without_explicit_topic_id() {
    let mut preview =
        map(r#"{"last_message": {"id": "general-message", "sender_id": "me", "created_at": 2.0, "is_read": false}}"#);
    let read_ids = ids(&["general-message"]);

    ChatServicesStore::mark_raw_last_message_read(&mut preview, "me", Some(""), 2.0, None, Some(&read_ids));

    assert_eq!(last_message_read(&preview), Some(true));
}

#[test]
fn exact_read_message_ids_override_timestamp_ordering() {
    let read_ids = ids(&["captured-delayed-message"]);

    assert!(MessageServicesStore::is_within_read_boundary(
        "captured-delayed-message",
        1.0,
        10.0,
        None,
        Some(&read_ids),
    ));
    assert!(!MessageServicesStore::is_within_read_boundary("late-old-message", 1.0, 10.0, None, Some(&read_ids)));
}

#[gpui::test]
fn topic_read_receipt_only_marks_matching_outgoing_messages(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let scope = format!("topic-read-{}", Uuid::new_v4().uuid_string());
        let service = MessageServicesStore::shared(cx);
        let cache_key = MessageServicesStore::messages_cache_key(&scope);
        let page = value(
            r#"{
                "messages": [
                    {"id": "general-own", "sender_id": "me", "created_at": 1.0, "is_read": false},
                    {"id": "topic-own", "sender_id": "me", "topic_id": "topic-1", "created_at": 2.0, "is_read": false},
                    {"id": "topic-own-late", "sender_id": "me", "topic_id": "topic-1", "created_at": 2.0, "is_read": false},
                    {"id": "topic-peer", "sender_id": "peer", "topic_id": "topic-1", "created_at": 3.0, "is_read": false}
                ]
            }"#,
        );
        SaiWsResponseCache::hydrate_memory(&cache_key, &page, false, cx);

        assert!(service.update(cx, |service, cx| {
            service.mark_outgoing_messages_read(&scope, "me", Some(""), 10.0, None, None, cx)
        }));

        let rows = cached_rows(&cache_key, cx);
        assert_eq!(row_is_read(&rows, "general-own"), Some(true));
        assert_eq!(row_is_read(&rows, "topic-own"), Some(false));

        let captured = ids(&["topic-own"]);
        assert!(service.update(cx, |service, cx| {
            service.mark_outgoing_messages_read(&scope, "me", Some("topic-1"), 2.0, Some(&captured), Some(&captured), cx)
        }));

        let rows = cached_rows(&cache_key, cx);
        assert_eq!(row_is_read(&rows, "topic-own"), Some(true));
        assert_eq!(row_is_read(&rows, "topic-own-late"), Some(false));
        assert_eq!(row_is_read(&rows, "topic-peer"), Some(false));

        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn topic_delete_requests_summary_recovery_only_for_the_current_preview(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let chat_id = format!("topic-delete-{}", Uuid::new_v4().uuid_string());
        let cache_key = ChatServicesStore::topics_cache_key(&chat_id);
        let service = ChatServicesStore::shared(cx);
        let page = value(
            r#"{
                "topics": [
                    {
                        "topic_id": "topic-1",
                        "last_message": {"id": "current-message", "sender_id": "peer", "created_at": 1.0}
                    }
                ]
            }"#,
        );
        SaiWsResponseCache::hydrate_memory(&cache_key, &page, false, cx);

        let deleted_other = service.update(cx, |service, cx| {
            service.apply_topic_message_deleted(&chat_id, Some("topic-1"), "different-message", cx)
        });
        assert!(!deleted_other);
        assert!(topic_last_message(&cache_key, "topic-1", cx).is_some());

        let deleted_current = service.update(cx, |service, cx| {
            service.apply_topic_message_deleted(&chat_id, Some("topic-1"), "current-message", cx)
        });
        assert!(deleted_current);
        assert!(topic_last_message(&cache_key, "topic-1", cx).is_none());

        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}
