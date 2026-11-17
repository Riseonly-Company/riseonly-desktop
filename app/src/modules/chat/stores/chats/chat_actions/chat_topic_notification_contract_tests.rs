use serde_json::Value;

use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatMuteScopes, ChatTopic};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{RiseStoredChatTopic, RiseStoredChatTopicList};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("test payload is valid JSON")
}

#[test]
fn legacy_realtime_topic_defaults_to_inherited_notifications() {
    let payload = value(r#"{"topic_id": "topic-1", "chat_id": "chat-1", "title": "General"}"#);
    let topic = SaiJSONCodec::decode::<RiseStoredChatTopic>(Some(&payload)).expect("topic decodes").to_topic();

    assert!(topic.notification_inherited);
    assert_eq!(topic.notification_scopes, ChatMuteScopes::ALL_ENABLED);
    assert_eq!(topic.icon, ChatTopic::DEFAULT_ICON);
}

#[test]
fn topic_preview_fields_decode_without_breaking_legacy_payloads() {
    let payload = value(
        r#"{
            "topic_id": "topic-2",
            "chat_id": "chat-1",
            "title": "News",
            "icon": "🎉",
            "unread_count": 5,
            "is_muted": true,
            "updated_at": 1721234567000,
            "last_message": {
                "id": "message-1",
                "chat_id": "chat-1",
                "sender_id": "user-1",
                "sender_name": "Alex",
                "content": "Hello",
                "content_type": "text",
                "created_at": 1721234566000
            }
        }"#,
    );
    let topic = SaiJSONCodec::decode::<RiseStoredChatTopic>(Some(&payload)).expect("topic decodes").to_topic();

    assert_eq!(topic.icon, "\u{1F389}");
    assert_eq!(topic.unread_count, 5);
    assert!(topic.is_muted);
    assert_eq!(topic.updated_at, 1_721_234_567.0);
    let last_message = topic.last_message.expect("preview decodes");
    assert_eq!(last_message.id, "message-1");
    assert_eq!(last_message.text, "Hello");
}

#[test]
fn general_topic_decodes_outside_regular_topic_array() {
    let payload = value(
        r#"{
            "topics": [],
            "general_topic": {
                "topic_id": "",
                "chat_id": "chat-1",
                "title": "General",
                "is_general": true,
                "unread_count": 2
            }
        }"#,
    );
    let gateway = SaiJSONCodec::decode::<RiseStoredChatTopicList>(Some(&payload)).expect("list decodes");

    assert!(gateway.topics.expect("topics decode").is_empty());
    let general = gateway.general_topic.expect("general decodes").to_topic();
    assert!(general.is_general);
    assert_eq!(general.unread_count, 2);
}

#[test]
fn inherited_topic_uses_resolved_chat_scopes() {
    let chat_scopes = ChatMuteScopes { messages: false, mentions: true, replies: true, reactions: false, events: true };
    let topic_scopes = ChatMuteScopes { messages: true, mentions: true, replies: true, reactions: true, events: true };

    let resolved = ChatServicesStore::resolved_topic_notification_scopes(topic_scopes, true, chat_scopes);

    assert_eq!(resolved, chat_scopes);
}

#[test]
fn explicit_topic_can_differ_from_stored_chat_scopes() {
    let stored_chat_scopes = ChatMuteScopes::MUTED;
    let topic_scopes = ChatMuteScopes { messages: true, mentions: false, replies: false, reactions: true, events: false };

    let resolved = ChatServicesStore::resolved_topic_notification_scopes(topic_scopes, false, stored_chat_scopes);

    assert_eq!(resolved, topic_scopes);
}
