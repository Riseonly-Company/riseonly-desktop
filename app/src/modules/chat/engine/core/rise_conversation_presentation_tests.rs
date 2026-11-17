use gpui::{App, TestAppContext};
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

struct Fixture {
    token: String,
    user_id: String,
    previous_user_id: Option<String>,
    previous_presentation_account: Option<String>,
}

impl Fixture {
    fn restore(&self, cx: &mut App) {
        if let Some(account) = &self.previous_presentation_account {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| presentation.activate(account, cx));
        }
        match &self.previous_user_id {
            Some(user_id) => SaiAccountCacheScope::activate(user_id),
            None => SaiAccountCacheScope::clear_active_account(),
        }
    }
}

fn activate_fixture(cx: &mut App) -> Fixture {
    let token = Uuid::new_v4().uuid_string();
    let previous_user_id = SaiAccountCacheScope::active_user_id();
    let user_id = format!("realtime-performance-{token}");
    SaiAccountCacheScope::activate(&user_id);
    let presentation = RiseConversationPresentation::shared(cx);
    let previous_presentation_account = presentation.read(cx).account_id().map(str::to_owned);
    if previous_presentation_account.is_some() {
        presentation.update(cx, |presentation, cx| presentation.deactivate(cx));
    }
    Fixture { token, user_id, previous_user_id, previous_presentation_account }
}

fn parse(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => value,
        Err(error) => panic!("test payload is JSON: {error}"),
    }
}

fn chat_row(id: &str, updated_at: f64) -> String {
    format!(
        r#"{{"id":"{id}","title":"{id}","type":"PRIVATE","updated_at":{updated_at:?},"unread_count":0,"is_pinned":false,"member_count":2}}"#
    )
}

fn message_row(id: &str, chat_id: &str, sender_id: &str, timestamp: f64) -> String {
    format!(
        r#"{{"id":"{id}","chat_id":"{chat_id}","sender_id":"{sender_id}","content":"{id}","content_type":"text","created_at":{timestamp:?},"is_read":false}}"#
    )
}

fn revision(cache_key: &str, cx: &mut App) -> i64 {
    SaiCacheRevisionTracker::shared(cx).read(cx).revision(cache_key)
}

fn root_marker(payload: Option<&Value>) -> Option<&str> {
    payload?.get("marker")?.as_str()
}

fn rows<'a>(payload: Option<&'a Value>, list_key: &str) -> Vec<&'a Map<String, Value>> {
    payload
        .and_then(|payload| payload.get(list_key))
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(Value::as_object).collect())
        .unwrap_or_default()
}

fn row_id(row: &Map<String, Value>) -> Option<&str> {
    row.get("id").and_then(Value::as_str)
}

fn save_to_disk(cache_key: &str, value: &Value) {
    assert!(SaiWsDiskCache::save(cache_key, value).is_ok());
}

#[gpui::test]
fn typing_is_transient_deduplicated_and_does_not_touch_chat_cache(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let fixture = activate_fixture(cx);

        let chat_id = format!("typing-{}", fixture.token);
        let cache_key = ChatServicesStore::chats_cache_key_for_user(&fixture.user_id);
        let chat = chat_row(&chat_id, 1.0);
        let disk_root = parse(&format!(r#"{{"marker":"disk-checkpoint","chats":[{chat}]}}"#));
        let memory_root = parse(&format!(r#"{{"marker":"live-memory","chats":[{chat}]}}"#));

        save_to_disk(&cache_key, &disk_root);
        SaiWsResponseCache::hydrate_memory(&cache_key, &memory_root, false, cx);
        let revision_before = revision(&cache_key, cx);

        let peer = format!("peer-{}", fixture.token);
        let typing = parse(&format!(
            r#"{{"chat_id":"{chat_id}","user_id":"{peer}","typing":true,"user_info":{{"id":"{peer}","name":"Alice"}}}}"#
        ));
        let service = MessageServicesStore::shared(cx);
        for _ in 0..25 {
            service.update(cx, |service, cx| service.typing_handler(Some(&typing), cx));
        }

        assert_eq!(service.read(cx).typists(&chat_id), vec!["Alice".to_owned()]);
        assert_eq!(service.read(cx).typing_peers_by_chat.get(&chat_id).map(Vec::len), Some(1));
        assert_eq!(revision(&cache_key, cx), revision_before);
        let memory = SaiWsResponseCache::memory_only(&cache_key, cx);
        assert_eq!(root_marker(memory.as_ref()), Some("live-memory"));
        let disk = SaiWsResponseCache::disk_only(&cache_key, cx);
        assert_eq!(root_marker(disk.as_ref()), Some("disk-checkpoint"));

        service.update(cx, |service, cx| {
            service.typing_peers_by_chat.remove(&chat_id);
            cx.notify();
        });
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
        fixture.restore(cx);
    });
}

#[gpui::test]
fn chat_realtime_uses_one_memory_delta_and_defers_disk_checkpoint(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let fixture = activate_fixture(cx);

        let chat_id = format!("chat-live-{}", fixture.token);
        let other_chat_id = format!("chat-other-{}", fixture.token);
        let cache_key = ChatServicesStore::chats_cache_key_for_user(&fixture.user_id);
        let disk_root = parse(&format!(
            r#"{{"marker":"disk-checkpoint","chats":[{},{}]}}"#,
            chat_row(&other_chat_id, 20.0),
            chat_row(&chat_id, 1.0)
        ));

        save_to_disk(&cache_key, &disk_root);
        SaiWsResponseCache::hydrate_memory(&cache_key, &disk_root, false, cx);
        let revision_before = revision(&cache_key, cx);

        let created = parse(&format!(
            r#"{{"id":"message-{}","chat_id":"{chat_id}","sender_id":"","content":"new","content_type":"text","created_at":30.0}}"#,
            fixture.token
        ));
        let chats = ChatServicesStore::shared(cx);
        chats.update(cx, |store, cx| store.message_created_success_handler(Some(&created), cx));

        let memory = SaiWsResponseCache::memory_only(&cache_key, cx);
        let ids: Vec<&str> = rows(memory.as_ref(), "chats").into_iter().filter_map(row_id).collect();
        assert_eq!(ids, vec![chat_id.as_str(), other_chat_id.as_str()]);
        assert_eq!(root_marker(memory.as_ref()), Some("disk-checkpoint"));
        assert_eq!(revision(&cache_key, cx), revision_before + 1);
        let disk = SaiWsResponseCache::disk_only(&cache_key, cx);
        assert_eq!(rows(disk.as_ref(), "chats").first().and_then(|row| row_id(row)), Some(other_chat_id.as_str()));

        chats.update(cx, |store, cx| store.flush_chats_disk_persistence(cx));
        SaiWsDiskCache::wait_for_asynchronous_writes();
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
        fixture.restore(cx);
    });
}

#[gpui::test]
fn message_create_and_read_mutate_only_loaded_memory_rows(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let fixture = activate_fixture(cx);

        let scope = format!("message-scope-{}", fixture.token);
        let service = MessageServicesStore::shared(cx);
        let cache_key = MessageServicesStore::messages_cache_key(&scope);
        let first_id = format!("first-{}", fixture.token);
        let second_id = format!("second-{}", fixture.token);
        let incoming_id = format!("incoming-{}", fixture.token);
        let root = parse(&format!(
            r#"{{"marker":"preserved","chat_id":"{scope}","messages":[{},{}]}}"#,
            message_row(&first_id, &scope, "owner", 1.0),
            message_row(&second_id, &scope, "owner", 3.0)
        ));

        save_to_disk(&cache_key, &root);
        SaiWsResponseCache::hydrate_memory(&cache_key, &root, false, cx);
        let revision_before_create = revision(&cache_key, cx);
        let event = parse(&format!(
            r#"{{"id":"{incoming_id}","chat_id":"{scope}","sender_id":"other","content":"incoming","content_type":"text","created_at":2.0}}"#
        ));

        service.update(cx, |service, cx| service.message_created_thread_handler(Some(&event), cx));

        let memory = SaiWsResponseCache::memory_only(&cache_key, cx);
        let ids: Vec<&str> = rows(memory.as_ref(), "messages").into_iter().filter_map(row_id).collect();
        assert_eq!(ids, vec![first_id.as_str(), incoming_id.as_str(), second_id.as_str()]);
        assert_eq!(root_marker(memory.as_ref()), Some("preserved"));
        assert_eq!(revision(&cache_key, cx), revision_before_create + 1);

        service.update(cx, |service, cx| service.message_created_thread_handler(Some(&event), cx));
        assert_eq!(revision(&cache_key, cx), revision_before_create + 1);

        let revision_before_read = revision(&cache_key, cx);
        assert!(service.update(cx, |service, cx| {
            service.mark_outgoing_messages_read(&scope, "owner", None, 1.5, None, None, cx)
        }));
        let memory = SaiWsResponseCache::memory_only(&cache_key, cx);
        let read_flags: Vec<Option<bool>> = rows(memory.as_ref(), "messages")
            .into_iter()
            .map(|row| row.get("is_read").and_then(Value::as_bool))
            .collect();
        assert_eq!(read_flags, vec![Some(true), Some(false), Some(false)]);
        assert_eq!(revision(&cache_key, cx), revision_before_read + 1);
        assert!(!service.update(cx, |service, cx| {
            service.mark_outgoing_messages_read(&scope, "owner", None, 1.5, None, None, cx)
        }));
        assert_eq!(revision(&cache_key, cx), revision_before_read + 1);
        let disk = SaiWsResponseCache::disk_only(&cache_key, cx);
        assert_eq!(rows(disk.as_ref(), "messages").len(), 2);

        service.update(cx, |service, cx| service.flush_messages_disk_persistence(&scope, cx));
        SaiWsDiskCache::wait_for_asynchronous_writes();
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
        SaiWsResponseCache::remove(
            &MessageServicesStore::latest_messages_cache_key(&scope),
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        fixture.restore(cx);
    });
}
