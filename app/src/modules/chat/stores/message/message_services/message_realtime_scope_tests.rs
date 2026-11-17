use std::collections::HashMap;

use gpui::{App, Entity, TestAppContext};
use rise_canon::rise_key::message as key;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn object(entries: Vec<(&str, Value)>) -> Map<String, Value> {
    entries.into_iter().map(|(entry_key, value)| (entry_key.to_owned(), value)).collect()
}

fn revision(cache_key: &str, cx: &mut App) -> i64 {
    SaiCacheRevisionTracker::shared(cx).read(cx).revision(cache_key)
}

fn message_rows(scope: &str, service: &Entity<MessageServicesStore>, cx: &mut App) -> Vec<GetMessageMessage> {
    service.update(cx, |service, cx| service.messages_in_memory(scope, cx))
}

fn revisions(scopes: &[String], keys: &HashMap<String, String>, cx: &mut App) -> HashMap<String, i64> {
    scopes.iter().map(|scope| (scope.clone(), revision(&keys[scope], cx))).collect()
}

#[gpui::test]
fn create_edit_delete_mirrors_loaded_topic_and_thread_exactly_once(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let token = Uuid::new_v4().uuid_string();
        let chat_id = format!("test-realtime-scope-{token}");
        let topic_id = format!("topic-{token}");
        let thread_root_id = format!("thread-{token}");
        let missing_topic_id = format!("missing-{token}");
        let service = MessageServicesStore::shared(cx);
        let base_scope = MessageServicesStore::message_scope(&chat_id, None, None);
        let topic_scope = MessageServicesStore::message_scope(&chat_id, Some(&topic_id), None);
        let thread_scope = MessageServicesStore::message_scope(&chat_id, None, Some(&thread_root_id));
        let missing_topic_scope = MessageServicesStore::message_scope(&chat_id, Some(&missing_topic_id), None);
        let scopes = vec![base_scope, topic_scope, thread_scope];
        for scope in &scopes {
            let root = object(vec![(key::CHAT_ID, Value::from(scope.as_str())), (key::MESSAGES, Value::Array(Vec::new()))]);
            let replaced = SaiUpdater::replace_root(
                &MessageServicesStore::messages_cache_key(scope),
                root,
                SaiUpdaterWriteScope::Cache,
                cx,
            );
            assert!(replaced.is_ok());
        }
        let keys: HashMap<String, String> =
            scopes.iter().map(|scope| (scope.clone(), MessageServicesStore::messages_cache_key(scope))).collect();
        let revisions_before_create = revisions(&scopes, &keys, cx);
        let message_id = format!("message-{token}");
        let created = Value::Object(object(vec![
            (key::ID, Value::from(message_id.as_str())),
            (key::CHAT_ID, Value::from(chat_id.as_str())),
            (key::SENDER_ID, Value::from(format!("sender-{token}"))),
            (key::CONTENT, Value::from("created")),
            (key::CONTENT_TYPE, Value::from("text")),
            (key::CREATED_AT, Value::from(10.0)),
            (key::TOPIC_ID, Value::from(topic_id.as_str())),
            ("thread_root_id", Value::from(thread_root_id.as_str())),
        ]));

        service.update(cx, |service, cx| service.message_created_thread_handler(Some(&created), cx));

        for scope in &scopes {
            let ids: Vec<String> = message_rows(scope, &service, cx).into_iter().map(|message| message.id).collect();
            assert_eq!(ids, vec![message_id.clone()]);
            assert_eq!(revision(&keys[scope], cx), revisions_before_create[scope] + 1);
        }
        assert!(
            SaiWsResponseCache::current_payload(
                &MessageServicesStore::messages_cache_key(&missing_topic_scope),
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
            .is_none()
        );

        let revisions_before_edit = revisions(&scopes, &keys, cx);
        let edited = Value::Object(object(vec![
            (key::ID, Value::from(message_id.as_str())),
            (key::CHAT_ID, Value::from(chat_id.as_str())),
            (key::TOPIC_ID, Value::from(topic_id.as_str())),
            ("thread_root_id", Value::from(thread_root_id.as_str())),
            ("new_content", Value::from("edited")),
            ("edit_date", Value::from(20.0)),
        ]));
        service.update(cx, |service, cx| service.message_edited_thread_handler(Some(&edited), cx));

        for scope in &scopes {
            let rows = message_rows(scope, &service, cx);
            assert_eq!(rows.first().map(|message| message.text.as_str()), Some("edited"));
            assert_eq!(revision(&keys[scope], cx), revisions_before_edit[scope] + 1);
        }

        let revisions_before_delete = revisions(&scopes, &keys, cx);
        let deleted = Value::Object(object(vec![
            ("message_ids", Value::from(vec![message_id.clone()])),
            (key::CHAT_ID, Value::from(chat_id.as_str())),
            (key::TOPIC_ID, Value::from(topic_id.as_str())),
            ("thread_root_id", Value::from(thread_root_id.as_str())),
        ]));
        service.update(cx, |service, cx| service.message_deleted_thread_handler(Some(&deleted), cx));

        for scope in &scopes {
            assert!(message_rows(scope, &service, cx).is_empty());
            assert_eq!(revision(&keys[scope], cx), revisions_before_delete[scope] + 1);
        }

        for scope in scopes.iter().chain(std::iter::once(&missing_topic_scope)) {
            service.update(cx, |service, cx| service.flush_messages_disk_persistence(scope, cx));
            SaiWsResponseCache::remove(
                &MessageServicesStore::messages_cache_key(scope),
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
            SaiWsResponseCache::remove(
                &MessageServicesStore::latest_messages_cache_key(scope),
                SaiCacheCleanLayer::Everywhere,
                cx,
            );
        }
    });
}
