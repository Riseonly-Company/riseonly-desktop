use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{App, TestAppContext};
use rise_canon::rise_key::bot::{HAS_MORE, RELATIVE_ID};
use rise_canon::rise_wire_canon::key::{DATA, METHOD, REQUEST_ID, SERVICE};
use rise_canon::rise_wire_methods::message;
use rise_core::foundation::foundation_uuid::Uuid;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::sai_ws_disk_cache::SaiWsDiskCache;
use super::sai_ws_pagination::SaiWsPaginationOptions;
use super::sai_ws_pending_registry::SaiWsPendingRegistry;
use super::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsPendingCacheBinding, SaiWsRequestOptions,
    SaiWsRequestOptionsInit,
};
use super::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice};
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWsScreenRefreshCacheHitPolicy;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SaiWsCallbackCachePolicyFixture {
    marker: String,
}

fn options(init: SaiWsRequestOptionsInit) -> SaiWsRequestOptions {
    SaiWsRequestOptions::new(init)
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// map of strings, one non string value fails the whole thing
fn string_dict(value: Option<Value>) -> Option<Map<String, Value>> {
    let map = value?.as_object()?.clone();
    map.values().all(Value::is_string).then_some(map)
}

fn marker(value: Option<Value>) -> Option<String> {
    string_dict(value)?.get("marker")?.as_str().map(str::to_owned)
}

fn unique(prefix: &str) -> String {
    format!("{prefix}{}", Uuid::new_v4().uuid_string())
}

struct RemoveOnDrop<'a> {
    cache_key: String,
    cx: &'a mut App,
}

impl Drop for RemoveOnDrop<'_> {
    fn drop(&mut self) {
        SaiWsResponseCache::remove(&self.cache_key, SaiCacheCleanLayer::Everywhere, self.cx);
    }
}

#[gpui::test]
fn cached_success_callback_policy_defaults_on_and_can_be_disabled(cx: &mut TestAppContext) {
    cx.update(|_cx| {
        let defaults = options(SaiWsRequestOptionsInit {
            service: "test".to_owned(),
            method: "defaults".to_owned(),
            ..SaiWsRequestOptionsInit::default()
        });
        let wire_only = options(SaiWsRequestOptionsInit {
            service: "test".to_owned(),
            method: "wire_only".to_owned(),
            invoke_success_for_cached_response: false,
            ..SaiWsRequestOptionsInit::default()
        });
        let dictionary = SaiWsRequestOptions::from_options_dictionary(&object(json!({
            (SERVICE): "test",
            (METHOD): "dictionary",
            "invokeSuccessForCachedResponse": false
        })))
        .expect("dictionary options parse");
        let strict_memory_only = SaiWsRequestOptions::from_options_dictionary(&object(json!({
            (SERVICE): "test",
            (METHOD): "memory_only",
            "diskCachePolicy": "memoryOnly"
        })))
        .expect("memory-only options parse");

        assert!(defaults.invoke_success_for_cached_response);
        assert_eq!(defaults.disk_cache_policy, SaiWsDiskCachePolicy::PreserveExisting);
        assert!(!wire_only.invoke_success_for_cached_response);
        assert!(!dictionary.invoke_success_for_cached_response);
        assert_eq!(strict_memory_only.disk_cache_policy, SaiWsDiskCachePolicy::MemoryOnly);
        assert!(!strict_memory_only.persist_cache_to_disk());
    });
}

#[test]
fn granted_screen_refresh_always_follows_cached_state_with_wire_revalidation() {
    assert!(SaiWsScreenRefreshCacheHitPolicy::should_send_granted_revalidation_wire(true, false));
    assert!(SaiWsScreenRefreshCacheHitPolicy::should_send_granted_revalidation_wire(false, false));
    assert!(SaiWsScreenRefreshCacheHitPolicy::should_send_granted_revalidation_wire(true, true));
}

#[gpui::test]
fn non_storing_response_delivers_wire_payload_without_replacing_cache(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let logical_key = unique("test-callback-policy-");
        let options = options(SaiWsRequestOptionsInit {
            service: "test".to_owned(),
            method: "callback_policy".to_owned(),
            cache_key: Some(logical_key),
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });
        let cache_key = options.cache_key.clone().expect("scoped cache key");
        let mut guard = RemoveOnDrop { cache_key: cache_key.clone(), cx };
        let app = &mut *guard.cx;

        SaiWsResponseCache::install_optimistic_root(
            json!({ "marker": "cached" }),
            &cache_key,
            true,
            true,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            app,
        )
        .expect("cached root installs");

        let slice = SaiTypedSlice::<SaiWsCallbackCachePolicyFixture>::new(app);
        let request_id: u64 = rand::random_range(100_000..=u64::MAX);
        let callback_value: Rc<RefCell<Option<SaiWsCallbackCachePolicyFixture>>> = Rc::default();
        let sink = Rc::clone(&callback_value);
        let binding = SaiWsPendingCacheBinding::new(&options);
        let slice_state: Rc<dyn SaiSliceState> = slice.clone();
        let account_user_id = options.account_user_id.clone();
        let account_epoch = options.account_epoch.clone();
        let registry = SaiWsPendingRegistry::shared(app);
        registry.update(app, |registry, cx| {
            registry.register(
                request_id,
                Some(binding),
                None,
                Some(slice_state),
                account_user_id,
                account_epoch,
                None,
                Some(Box::new(move |value: Option<Box<dyn Any>>, _cx: &mut App| {
                    *sink.borrow_mut() = value
                        .and_then(|value| value.downcast::<SaiWsCallbackCachePolicyFixture>().ok())
                        .map(|value| *value);
                })),
                None,
                cx,
            );
        });

        let frame = object(json!({
            (REQUEST_ID): request_id.to_string(),
            (DATA): { "marker": "wire" }
        }));
        let handled = registry.update(app, |registry, cx| {
            registry.handle_incoming_json_object(&frame, cx)
        });

        assert!(handled);
        assert_eq!(
            callback_value.borrow().as_ref().map(|value| value.marker.as_str()),
            Some("wire")
        );
        assert_eq!(slice.data().map(|value| value.marker), Some("cached".to_owned()));
        assert_eq!(
            marker(SaiWsResponseCache::memory_only(&cache_key, app)),
            Some("cached".to_owned())
        );
        assert_eq!(
            marker(SaiWsResponseCache::disk_only(&cache_key, app)),
            Some("cached".to_owned())
        );
    });
}

#[gpui::test]
fn explicit_disk_persistence_still_writes_when_memory_storage_is_disabled(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let options = options(SaiWsRequestOptionsInit {
            service: "test".to_owned(),
            method: "callback_policy_disk".to_owned(),
            cache_key: Some(unique("test-callback-policy-disk-")),
            store_response_in_cache: false,
            persist_cache_to_disk: true,
            ..SaiWsRequestOptionsInit::default()
        });
        let cache_key = options.cache_key.clone().expect("scoped cache key");
        let mut guard = RemoveOnDrop { cache_key: cache_key.clone(), cx };
        let app = &mut *guard.cx;

        SaiWsResponseCache::install_optimistic_root(
            json!({ "marker": "cached" }),
            &cache_key,
            true,
            true,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            app,
        )
        .expect("cached root installs");
        SaiWsResponseCache::store_success_payload_binding(
            Some(json!({ "marker": "wire" })),
            &SaiWsPendingCacheBinding::new(&options),
            app,
        );

        assert_eq!(
            marker(SaiWsResponseCache::memory_only(&cache_key, app)),
            Some("cached".to_owned())
        );
        assert_eq!(
            marker(SaiWsResponseCache::disk_only(&cache_key, app)),
            Some("wire".to_owned())
        );
    });
}

#[gpui::test]
fn message_history_keeps_disk_checkpoint_until_explicit_flush(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let options = options(SaiWsRequestOptionsInit {
            service: message::SERVICE.to_owned(),
            method: message::GET_MESSAGES.to_owned(),
            cache_key: Some(unique("getMessages-test-deferred-disk-")),
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });
        let cache_key = options.cache_key.clone().expect("scoped cache key");
        let mut guard = RemoveOnDrop { cache_key: cache_key.clone(), cx };
        let app = &mut *guard.cx;

        SaiWsResponseCache::install_optimistic_root(
            json!({ "marker": "checkpoint" }),
            &cache_key,
            true,
            true,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            app,
        )
        .expect("checkpoint root installs");
        SaiWsResponseCache::store_success_payload_binding(
            Some(json!({ "marker": "memory" })),
            &SaiWsPendingCacheBinding::new(&options),
            app,
        );

        assert_eq!(
            marker(SaiWsResponseCache::memory_only(&cache_key, app)),
            Some("memory".to_owned())
        );
        assert_eq!(
            marker(SaiWsResponseCache::disk_only(&cache_key, app)),
            Some("checkpoint".to_owned())
        );
    });
}

#[gpui::test]
fn preserve_existing_policy_keeps_legacy_false_persistence_behavior(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let options = options(SaiWsRequestOptionsInit {
            service: "test".to_owned(),
            method: "preserve_existing".to_owned(),
            cache_key: Some(unique("test-preserve-existing-")),
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });
        let cache_key = options.cache_key.clone().expect("scoped cache key");
        let mut guard = RemoveOnDrop { cache_key: cache_key.clone(), cx };
        let app = &mut *guard.cx;

        SaiWsDiskCache::save(&cache_key, &json!({ "marker": "legacy" }))
            .expect("legacy disk entry saves");
        SaiWsResponseCache::store_success_payload_options(
            Some(json!({ "marker": "updated" })),
            &options,
            app,
        );

        assert_eq!(options.disk_cache_policy, SaiWsDiskCachePolicy::PreserveExisting);
        assert_eq!(
            marker(SaiWsResponseCache::disk_only(&cache_key, app)),
            Some("updated".to_owned())
        );
    });
}

#[gpui::test]
fn memory_only_policy_preserves_memory_and_removes_legacy_disk_state(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let options = options(SaiWsRequestOptionsInit {
            service: "test".to_owned(),
            method: "strict_memory".to_owned(),
            cache_key: Some(unique("test-memory-only-legacy-")),
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            ..SaiWsRequestOptionsInit::default()
        });
        let cache_key = options.cache_key.clone().expect("scoped cache key");
        let mut guard = RemoveOnDrop { cache_key: cache_key.clone(), cx };
        let app = &mut *guard.cx;

        SaiWsDiskCache::save(&cache_key, &json!({ "marker": "legacy-disk" }))
            .expect("legacy disk entry saves");
        SaiWsDiskCache::save_optimistic_meta(&cache_key, Some("items"), Some("id"))
            .expect("legacy optimistic meta saves");
        SaiWsResponseCache::hydrate_memory(
            &cache_key,
            json!({ "marker": "live-memory" }),
            false,
            app,
        );

        let (value, had_memory, had_disk) = SaiWsResponseCache::peek(&options, None, app);

        assert!(had_memory);
        assert!(!had_disk);
        assert_eq!(marker(value), Some("live-memory".to_owned()));
        assert_eq!(SaiWsResponseCache::disk_only(&cache_key, app), None);
        assert_eq!(SaiWsDiskCache::load_optimistic_meta(&cache_key), None);
    });
}

#[gpui::test]
fn memory_only_observed_projection_cannot_reuse_disk_backed_projection(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let cache_key = unique("test-memory-only-observed-");
        let mut guard = RemoveOnDrop { cache_key: cache_key.clone(), cx };
        let app = &mut *guard.cx;

        SaiWsDiskCache::save(&cache_key, &json!({ "marker": "legacy-disk" }))
            .expect("legacy disk entry saves");
        let projection_runs = Cell::new(0);

        let first: Option<String> = SaiWsResponseCache::observed_projection(
            &cache_key,
            "marker",
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            |payload: Option<&Value>| {
                projection_runs.set(projection_runs.get() + 1);
                marker(payload.cloned())
            },
            app,
        );
        let strict: Option<String> = SaiWsResponseCache::observed_projection(
            &cache_key,
            "marker",
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::MemoryOnly,
            |payload: Option<&Value>| {
                projection_runs.set(projection_runs.get() + 1);
                marker(payload.cloned())
            },
            app,
        );

        assert_eq!(first.as_deref(), Some("legacy-disk"));
        assert_eq!(strict.as_deref(), Some("legacy-disk"));
        assert_eq!(projection_runs.get(), 2);
        assert_eq!(SaiWsResponseCache::disk_only(&cache_key, app), None);
    });
}

#[gpui::test]
fn memory_only_policy_prevents_pending_pagination_write_from_resurrecting_disk(
    cx: &mut TestAppContext,
) {
    let logical_key = unique("test-memory-only-pending-write-");
    let persistent = options(SaiWsRequestOptionsInit {
        service: "test".to_owned(),
        method: "persistent_page".to_owned(),
        cache_key: Some(logical_key.clone()),
        store_response_in_cache: true,
        persist_cache_to_disk: true,
        path_to_array: Some("items".to_owned()),
        path_to_id: Some("id".to_owned()),
        pagination: Some(SaiWsPaginationOptions::relative_id(None)),
        ..SaiWsRequestOptionsInit::default()
    });
    let memory_only = options(SaiWsRequestOptionsInit {
        service: "test".to_owned(),
        method: "memory_page".to_owned(),
        cache_key: Some(logical_key),
        store_response_in_cache: true,
        disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
        path_to_array: Some("items".to_owned()),
        path_to_id: Some("id".to_owned()),
        pagination: Some(SaiWsPaginationOptions::relative_id(None)),
        ..SaiWsRequestOptionsInit::default()
    });
    let cache_key = memory_only.cache_key.clone().expect("scoped cache key");

    cx.update(|cx| {
        SaiWsResponseCache::store_success_payload_options(
            Some(json!({ "items": [{ "id": "legacy" }], (RELATIVE_ID): "one" })),
            &persistent,
            cx,
        );
        SaiWsResponseCache::prepare_for_request(&memory_only, cx);
    });
    SaiWsDiskCache::wait_for_asynchronous_writes();
    cx.run_until_parked();

    cx.update(|cx| {
        let mut guard = RemoveOnDrop { cache_key: cache_key.clone(), cx };
        let app = &mut *guard.cx;
        assert_eq!(SaiWsResponseCache::disk_only(&cache_key, app), None);
        let first_id = SaiWsResponseCache::memory_only(&cache_key, app)
            .as_ref()
            .and_then(|payload| payload.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| string_dict(Some(item.clone())))
            .and_then(|item| item.get("id").and_then(Value::as_str).map(str::to_owned));
        assert_eq!(first_id.as_deref(), Some("legacy"));
    });
}

#[gpui::test]
fn memory_only_pagination_still_merges_continuation_in_memory(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let logical_key = unique("test-memory-only-pagination-");
        let initial = options(SaiWsRequestOptionsInit {
            service: "test".to_owned(),
            method: "initial".to_owned(),
            cache_key: Some(logical_key.clone()),
            store_response_in_cache: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            path_to_array: Some("items".to_owned()),
            path_to_id: Some("id".to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(None)),
            ..SaiWsRequestOptionsInit::default()
        });
        let continuation = options(SaiWsRequestOptionsInit {
            service: "test".to_owned(),
            method: "continuation".to_owned(),
            cache_key: Some(logical_key),
            store_response_in_cache: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            path_to_array: Some("items".to_owned()),
            path_to_id: Some("id".to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(Some("cursor-one"))),
            ..SaiWsRequestOptionsInit::default()
        });
        let cache_key = initial.cache_key.clone().expect("scoped cache key");
        let mut guard = RemoveOnDrop { cache_key: cache_key.clone(), cx };
        let app = &mut *guard.cx;

        SaiWsResponseCache::store_success_payload_options(
            Some(json!({
                "items": [{ "id": "one" }],
                (RELATIVE_ID): "cursor-one",
                (HAS_MORE): true
            })),
            &initial,
            app,
        );

        let request_id: u64 = rand::random_range(100_000..=u64::MAX);
        let binding = SaiWsPendingCacheBinding::new(&continuation);
        let account_user_id = continuation.account_user_id.clone();
        let account_epoch = continuation.account_epoch.clone();
        let registry = SaiWsPendingRegistry::shared(app);
        registry.update(app, |registry, cx| {
            registry.register(
                request_id,
                Some(binding),
                None,
                None,
                account_user_id,
                account_epoch,
                None,
                None,
                None,
                cx,
            );
        });
        let frame = object(json!({
            (REQUEST_ID): request_id.to_string(),
            (DATA): {
                "items": [{ "id": "two" }],
                (RELATIVE_ID): "cursor-two",
                (HAS_MORE): false
            }
        }));
        let handled = registry.update(app, |registry, cx| {
            registry.handle_incoming_json_object(&frame, cx)
        });

        let payload = SaiWsResponseCache::memory_only(&cache_key, app)
            .and_then(|payload| payload.as_object().cloned())
            .expect("memory payload is an object");
        let items = payload
            .get("items")
            .and_then(Value::as_array)
            .filter(|items| items.iter().all(Value::is_object))
            .expect("items are objects");
        assert!(handled);
        assert_eq!(
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert_eq!(SaiWsResponseCache::disk_only(&cache_key, app), None);
    });
}
