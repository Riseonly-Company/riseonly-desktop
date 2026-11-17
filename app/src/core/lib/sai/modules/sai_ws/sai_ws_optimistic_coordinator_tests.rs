use std::sync::Arc;

use gpui::{App, TestAppContext};
use rise_canon::rise_wire_methods::message;
use serde_json::{Map, Value, json};

use super::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use super::sai_ws_optimistic_coordinator::SaiWsOptimisticCoordinator;
use super::sai_ws_optimistic_update::{
    SaiWsOptimisticAddStrategy, SaiWsOptimisticSuccessContext, SaiWsOptimisticUpdate,
};
use super::sai_ws_pending_registry::{SaiWsPendingRegistry, SaiWsResponseError};
use super::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsPendingCacheBinding, SaiWsRequestOptions,
    SaiWsRequestOptionsInit,
};
use super::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;

const CACHE_KEY: &str = "test-optimistic-cache";
const PATH_TO_ARRAY: &str = "messages";

fn reset_all(cx: &mut App) {
    SaiWsOptimisticCoordinator::reset(cx);
    SaiWsPendingRegistry::shared(cx).update(cx, |registry, cx| registry.reset(cx));
    SaiCacheRevisionTracker::shared(cx).update(cx, |tracker, cx| tracker.reset(cx));
    SaiWsResponseCache::remove(CACHE_KEY, SaiCacheCleanLayer::Everywhere, cx);
}

fn seed_cache(rows: Vec<Value>, cx: &mut App) {
    SaiWsResponseCache::install_optimistic_root(
        json!({ PATH_TO_ARRAY: rows }),
        CACHE_KEY,
        true,
        false,
        SaiWsDiskCachePolicy::PreserveExisting,
        false,
        cx,
    )
    .expect("seeding the in-memory optimistic root");
}

fn current_rows(cx: &mut App) -> Vec<Map<String, Value>> {
    let Some(root) = SaiWsResponseCache::current_payload(
        CACHE_KEY,
        SaiWsCachePreferredLayer::MemoryFirst,
        SaiWsDiskCachePolicy::PreserveExisting,
        cx,
    ) else {
        return Vec::new();
    };
    let Some(arr) = root.get(PATH_TO_ARRAY).and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter().filter_map(|row| row.as_object().cloned()).collect()
}

fn insert_options() -> SaiWsRequestOptions {
    let opt = SaiWsOptimisticUpdate {
        enabled: true,
        temp_id_key: "tempId".to_owned(),
        temp_flag_key: "isTemp".to_owned(),
        add_strategy: SaiWsOptimisticAddStrategy::Start,
        create_temp_data: Some(Arc::new(|body: &Value, _cx: &mut App| -> anyhow::Result<Value> {
            let content = body.get("content").and_then(Value::as_str).unwrap_or("");
            Ok(json!({ "content": content }))
        })),
        extract_real_data: Some(Arc::new(
            |payload: &Value, _context: &SaiWsOptimisticSuccessContext, _cx: &mut App| -> anyhow::Result<Value> {
                let dict = payload.as_object().cloned().unwrap_or_default();
                let real = dict.get("message").and_then(Value::as_object).cloned();
                Ok(Value::Object(real.unwrap_or(dict)))
            },
        )),
        ..SaiWsOptimisticUpdate::default()
    };
    SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: message::SERVICE.to_owned(),
        method: message::CREATE_MESSAGE.to_owned(),
        cache_key: Some(CACHE_KEY.to_owned()),
        store_response_in_cache: true,
        persist_cache_to_disk: false,
        path_to_array: Some(PATH_TO_ARRAY.to_owned()),
        path_to_id: Some("id".to_owned()),
        optimistic_update: Some(opt),
        ..SaiWsRequestOptionsInit::default()
    })
}

fn str_field<'a>(row: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str)
}

fn bool_field(row: &Map<String, Value>, key: &str) -> Option<bool> {
    row.get(key).and_then(Value::as_bool)
}

#[gpui::test]
fn apply_before_send_inserts_temp_row_and_registers_session(cx: &mut TestAppContext) {
    cx.update(|cx| {
        reset_all(cx);
        seed_cache(vec![json!({ "id": "real-1", "content": "hello" })], cx);

        let wire_id: u64 = 1001;
        let body = json!({ "content": "optimistic text" });
        let outcome = SaiWsOptimisticCoordinator::apply_before_send_if_needed(
            wire_id,
            &body,
            &insert_options(),
            cx,
        )
        .expect("optimistic insert applies");

        assert!(outcome.did_register_session);
        let temp_id = outcome.insert_temp_id.clone().expect("insert produces a temp id");
        assert!(temp_id.starts_with("temp_"));
        assert!(SaiWsOptimisticCoordinator::has_insert_session(wire_id, cx));

        let rows = current_rows(cx);
        assert_eq!(rows.len(), 2);
        let temp_row = rows.first().expect("temp row is first");
        assert_eq!(str_field(temp_row, "tempId"), Some(temp_id.as_str()));
        assert_eq!(bool_field(temp_row, "isTemp"), Some(true));
        assert_eq!(str_field(temp_row, "content"), Some("optimistic text"));
    });
}

#[gpui::test]
fn on_success_swaps_temp_row_for_real_row_and_removes_session(cx: &mut TestAppContext) {
    cx.update(|cx| {
        reset_all(cx);
        seed_cache(vec![json!({ "id": "real-1", "content": "hello" })], cx);

        let wire_id: u64 = 2002;
        let options = insert_options();
        let outcome = SaiWsOptimisticCoordinator::apply_before_send_if_needed(
            wire_id,
            &json!({ "content": "pending msg" }),
            &options,
            cx,
        )
        .expect("optimistic insert applies");
        let temp_id = outcome.insert_temp_id.clone().expect("insert produces a temp id");
        assert!(SaiWsOptimisticCoordinator::has_insert_session(wire_id, cx));

        let server_payload = json!({
            "message": { "id": "server-99", "content": "pending msg", "isTemp": false }
        });
        let binding = SaiWsPendingCacheBinding::new(&options);
        let merged = SaiWsOptimisticCoordinator::merged_payload_for_cache_if_needed(
            wire_id,
            Some(server_payload),
            &binding,
            cx,
        );
        SaiWsResponseCache::store_success_payload_binding(merged, &binding, cx);

        assert!(!SaiWsOptimisticCoordinator::has_insert_session(wire_id, cx));

        let rows = current_rows(cx);
        assert_eq!(rows.len(), 2);
        assert!(!rows.iter().any(|row| bool_field(row, "isTemp") == Some(true)));
        let swapped = rows.first().expect("swapped row is first");
        assert_eq!(str_field(swapped, "id"), Some("server-99"));
        assert_eq!(bool_field(swapped, "isTemp"), Some(false));
        assert_eq!(str_field(swapped, "content"), Some("pending msg"));
        let _ = temp_id;
    });
}

#[gpui::test]
fn on_failure_removes_temp_row_and_session(cx: &mut TestAppContext) {
    cx.update(|cx| {
        reset_all(cx);
        seed_cache(vec![json!({ "id": "real-1", "content": "hello" })], cx);

        let wire_id: u64 = 3003;
        let options = insert_options();
        let outcome = SaiWsOptimisticCoordinator::apply_before_send_if_needed(
            wire_id,
            &json!({ "content": "will fail" }),
            &options,
            cx,
        )
        .expect("optimistic insert applies");
        let temp_id = outcome.insert_temp_id.clone().expect("insert produces a temp id");
        assert_eq!(current_rows(cx).len(), 2);

        let error = SaiWsResponseError {
            message: "boom".to_owned(),
            ..SaiWsResponseError::default()
        };
        SaiWsOptimisticCoordinator::handle_failure(
            wire_id,
            &error,
            None,
            Some(&SaiWsPendingCacheBinding::new(&options)),
            cx,
        );

        assert!(!SaiWsOptimisticCoordinator::has_insert_session(wire_id, cx));
        let rows = current_rows(cx);
        assert_eq!(rows.len(), 1);
        assert!(!rows.iter().any(|row| str_field(row, "tempId") == Some(temp_id.as_str())));
        assert_eq!(rows.first().and_then(|row| str_field(row, "id")), Some("real-1"));
    });
}
