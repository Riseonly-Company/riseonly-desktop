use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, TestAppContext};
use rise_core::foundation::foundation_uuid::Uuid;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{
    SaiSliceState, SaiTypedSlice, SaiWsSliceStatus,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationCursorFallback, SaiWsPaginationDuplicatePolicy, SaiWsPaginationEdge,
    SaiWsPaginationError, SaiWsPaginationFailureKind, SaiWsPaginationMergeDirection,
    SaiWsPaginationMergeResult, SaiWsPaginationMerger, SaiWsPaginationOptions,
    SaiWsPaginationRelativeIdArgs, SaiWsPaginationStateStore,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsPendingCacheBinding,
    SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SaiWsPaginationFixtureItem {
    id: String,
    value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SaiWsPaginationFixtureEnvelope {
    items: Vec<SaiWsPaginationFixtureItem>,
    #[serde(rename = "relative_id", default)]
    relative_id: Option<String>,
    #[serde(rename = "has_more")]
    has_more: bool,
}

fn dictionary(value: Option<&Value>) -> &Map<String, Value> {
    value
        .and_then(Value::as_object)
        .expect("value is a dictionary")
}

fn dictionaries(value: Option<&Value>) -> Vec<&Map<String, Value>> {
    value
        .and_then(Value::as_array)
        .expect("value is an array")
        .iter()
        .filter_map(Value::as_object)
        .collect()
}

fn nested<'a>(root: &'a Map<String, Value>, key: &str) -> &'a Map<String, Value> {
    dictionary(root.get(key))
}

fn ids(rows: &[&Map<String, Value>]) -> Vec<String> {
    ids_at(rows, "id")
}

fn ids_at(rows: &[&Map<String, Value>], path: &str) -> Vec<String> {
    let components: Vec<&str> = path.split('.').collect();
    rows.iter()
        .filter_map(|row| {
            let (first, rest) = components.split_first()?;
            let mut value = row.get(*first)?;
            for component in rest {
                value = value.as_object()?.get(*component)?;
            }
            match value {
                Value::String(string) => Some(string.clone()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            }
        })
        .collect()
}

fn string_at<'a>(row: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str)
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("fixture is an object"),
    }
}

fn relative_id_options(args: SaiWsPaginationRelativeIdArgs) -> SaiWsPaginationOptions {
    SaiWsPaginationOptions::relative_id(args)
}

fn relative_id(cursor: Option<&str>) -> SaiWsPaginationOptions {
    relative_id_options(SaiWsPaginationRelativeIdArgs {
        relative_id: cursor.map(str::to_string),
        ..Default::default()
    })
}

fn merge(
    existing: Option<&Value>,
    page: &Value,
    options: &SaiWsPaginationOptions,
    items_path: &str,
    item_id_path: &str,
) -> SaiWsPaginationMergeResult {
    SaiWsPaginationMerger::merge(existing, Some(page), options, items_path, Some(item_id_path))
        .expect("merge succeeds")
}

fn page_request_options(cache_key: &str, pagination: SaiWsPaginationOptions) -> SaiWsRequestOptions {
    SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: "test".to_string(),
        method: "page".to_string(),
        cache_key: Some(cache_key.to_string()),
        path_to_array: Some("items".to_string()),
        path_to_id: Some("id".to_string()),
        pagination: Some(pagination),
        ..Default::default()
    })
}

fn disk_first_request_options(cache_key: &str) -> SaiWsRequestOptions {
    SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: "test".to_string(),
        method: "page".to_string(),
        cache_key: Some(cache_key.to_string()),
        store_response_in_cache: true,
        persist_cache_to_disk: true,
        preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
        path_to_array: Some("items".to_string()),
        path_to_id: Some("id".to_string()),
        pagination: Some(relative_id(Some("cursor-1"))),
        ..Default::default()
    })
}

fn merge_result(
    has_more: bool,
    relative_id: Option<&str>,
    received_count: i64,
    inserted_count: i64,
    protocol_error: Option<&str>,
) -> SaiWsPaginationMergeResult {
    SaiWsPaginationMergeResult {
        root: Map::new(),
        has_more,
        relative_id: relative_id.map(str::to_string),
        received_count,
        inserted_count,
        protocol_error: protocol_error.map(str::to_string),
    }
}

fn unique_key(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().uuid_string())
}

fn random_request_id() -> u64 {
    rand::random::<u64>().max(100_000)
}

#[test]
fn initial_page_supports_nested_paths_numeric_cursor_and_stable_deduplication() {
    let page = json!({
        "payload": {
            "records": [
                {"identity": {"relative_id": 1}, "value": "first"},
                {"identity": {"relative_id": "1"}, "value": "duplicate"},
                {"identity": {"relative_id": 2}, "value": "second"},
                {"value": "idless-a"},
                {"value": "idless-b"}
            ],
            "paging": {"cursor": 42, "has_more": "1"}
        },
        "request_metadata": {"source": "wire"}
    });
    let options = relative_id_options(SaiWsPaginationRelativeIdArgs {
        relative_id: None,
        response_relative_id_path: "payload.paging.cursor".to_string(),
        has_more_path: Some("payload.paging.has_more".to_string()),
        ..Default::default()
    });

    let result = merge(None, &page, &options, "payload.records", "identity.relative_id");

    let payload = nested(&result.root, "payload");
    let rows = dictionaries(payload.get("records"));
    let metadata = nested(&result.root, "request_metadata");

    assert_eq!(rows.len(), 4);
    assert_eq!(ids_at(&rows, "identity.relative_id"), vec!["1", "2"]);
    assert_eq!(rows.first().and_then(|row| string_at(row, "value")), Some("first"));
    assert_eq!(
        rows[rows.len() - 2..]
            .iter()
            .filter_map(|row| string_at(row, "value"))
            .collect::<Vec<_>>(),
        vec!["idless-a", "idless-b"]
    );
    assert_eq!(result.relative_id.as_deref(), Some("42"));
    assert!(result.has_more);
    assert_eq!(result.received_count, 5);
    assert_eq!(result.inserted_count, 4);
    assert_eq!(result.protocol_error, None);
    assert_eq!(string_at(metadata, "source"), Some("wire"));
}

#[test]
fn append_keep_existing_cleans_dirty_history_and_deep_merges_metadata() {
    let existing = json!({
        "payload": {
            "records": [
                {"id": "one", "value": "first-one"},
                {"id": "one", "value": "dirty-one"},
                {"id": "two", "value": "existing-two"}
            ],
            "paging": {"cursor": "old", "has_more": true, "page_size": 20}
        },
        "metadata": {"stable": "keep", "replace": "old"},
        "existing_only": true
    });
    let page = json!({
        "payload": {
            "records": [
                {"id": "two", "value": "incoming-two"},
                {"id": "three", "value": "first-three"},
                {"id": "three", "value": "duplicate-three"},
                {"value": "idless"}
            ],
            "paging": {"cursor": "next", "has_more": false}
        },
        "metadata": {"replace": "new", "incoming": "present"}
    });
    let options = relative_id_options(SaiWsPaginationRelativeIdArgs {
        relative_id: Some("old".to_string()),
        response_relative_id_path: "payload.paging.cursor".to_string(),
        has_more_path: Some("payload.paging.has_more".to_string()),
        merge_direction: SaiWsPaginationMergeDirection::Append,
        duplicate_policy: SaiWsPaginationDuplicatePolicy::KeepExisting,
        ..Default::default()
    });

    let result = merge(Some(&existing), &page, &options, "payload.records", "id");

    let payload = nested(&result.root, "payload");
    let paging = nested(payload, "paging");
    let rows = dictionaries(payload.get("records"));
    let metadata = nested(&result.root, "metadata");
    let value_of = |id: &str| {
        rows.iter()
            .find(|row| string_at(row, "id") == Some(id))
            .and_then(|row| string_at(row, "value"))
    };

    assert_eq!(ids(&rows), vec!["one", "two", "three"]);
    assert_eq!(rows.len(), 4);
    assert_eq!(rows.first().and_then(|row| string_at(row, "value")), Some("first-one"));
    assert_eq!(value_of("two"), Some("existing-two"));
    assert_eq!(value_of("three"), Some("first-three"));
    assert_eq!(rows.last().and_then(|row| string_at(row, "value")), Some("idless"));
    assert_eq!(result.root.get("existing_only").and_then(Value::as_bool), Some(true));
    assert_eq!(string_at(metadata, "stable"), Some("keep"));
    assert_eq!(string_at(metadata, "replace"), Some("new"));
    assert_eq!(string_at(metadata, "incoming"), Some("present"));
    assert_eq!(paging.get("page_size").and_then(Value::as_i64), Some(20));
    assert_eq!(string_at(paging, "cursor"), Some("next"));
    assert_eq!(result.inserted_count, 2);
    assert!(!result.has_more);
}

#[test]
fn prepend_use_incoming_updates_overlap_and_keeps_new_rows_in_page_order() {
    let existing = json!({
        "items": [
            {"id": "two", "value": "old-two"},
            {"id": "three", "value": "old-three"}
        ],
        "relative_id": "cursor-2",
        "has_more": true
    });
    let page = json!({
        "items": [
            {"id": "two", "value": "new-two"},
            {"id": "one", "value": "first-one"},
            {"id": "one", "value": "latest-one"},
            {"value": "idless"}
        ],
        "relative_id": "cursor-1",
        "has_more": false
    });
    let options = relative_id_options(SaiWsPaginationRelativeIdArgs {
        edge: SaiWsPaginationEdge::Leading,
        relative_id: Some("cursor-2".to_string()),
        merge_direction: SaiWsPaginationMergeDirection::Prepend,
        duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
        ..Default::default()
    });

    let result = merge(Some(&existing), &page, &options, "items", "id");
    let rows = dictionaries(result.root.get("items"));

    assert_eq!(ids(&rows), vec!["one", "two", "three"]);
    assert_eq!(rows.len(), 4);
    assert_eq!(string_at(rows[0], "value"), Some("latest-one"));
    assert_eq!(string_at(rows[1], "value"), Some("idless"));
    assert_eq!(string_at(rows[2], "value"), Some("new-two"));
    assert_eq!(string_at(rows[3], "value"), Some("old-three"));
    assert_eq!(result.inserted_count, 2);
}

#[test]
fn protocol_errors_detect_missing_and_non_advancing_relative_id() {
    let existing = json!({"items": [{"id": "one"}]});
    let missing_cursor = merge(
        Some(&existing),
        &json!({"items": [{"id": "two"}], "has_more": true}),
        &relative_id(Some("cursor-1")),
        "items",
        "id",
    );
    let repeated_cursor = merge(
        Some(&existing),
        &json!({"items": [{"id": "two"}], "relative_id": 7, "has_more": true}),
        &relative_id(Some("7")),
        "items",
        "id",
    );
    let terminal_page = merge(
        Some(&existing),
        &json!({"items": [{"id": "two"}], "has_more": false}),
        &relative_id(Some("cursor-1")),
        "items",
        "id",
    );

    assert_eq!(
        missing_cursor.protocol_error.as_deref(),
        Some("Pagination returned has_more=true without relative_id.")
    );
    assert_eq!(
        repeated_cursor.protocol_error.as_deref(),
        Some("Pagination relative_id did not advance.")
    );
    assert_eq!(terminal_page.protocol_error, None);
    assert!(!terminal_page.has_more);
}

fn legacy_cursor_options(cursor: Option<&str>) -> SaiWsPaginationOptions {
    relative_id_options(SaiWsPaginationRelativeIdArgs {
        relative_id: cursor.map(str::to_string),
        response_relative_id_path: "cursor".to_string(),
        response_relative_id_fallback_path: Some("relative_id".to_string()),
        ..Default::default()
    })
}

#[test]
fn response_cursor_falls_back_to_legacy_response_path_without_using_item_identity() {
    let result = merge(
        None,
        &json!({
            "items": [{"entry_key": "user:one"}],
            "relative_id": 42,
            "has_more": true
        }),
        &legacy_cursor_options(None),
        "items",
        "entry_key",
    );

    assert_eq!(result.relative_id.as_deref(), Some("42"));
    assert_eq!(result.protocol_error, None);
    assert!(result.root.get("cursor").is_none());
    assert_eq!(result.root.get("relative_id").and_then(Value::as_i64), Some(42));
}

#[test]
fn legacy_response_path_removes_stale_primary_cursor_from_merged_continuation() {
    let existing = json!({
        "items": [{"entry_key": "user:one"}],
        "cursor": "opaque-old",
        "relative_id": 42,
        "has_more": true
    });
    let result = merge(
        Some(&existing),
        &json!({
            "items": [{"entry_key": "user:two"}],
            "relative_id": 41,
            "has_more": true
        }),
        &legacy_cursor_options(Some("opaque-old")),
        "items",
        "entry_key",
    );

    assert_eq!(result.relative_id.as_deref(), Some("41"));
    assert_eq!(result.protocol_error, None);
    assert!(result.root.get("cursor").is_none());
    assert_eq!(result.root.get("relative_id").and_then(Value::as_i64), Some(41));
}

#[test]
fn primary_response_cursor_wins_when_both_cursor_paths_are_present() {
    let result = merge(
        None,
        &json!({
            "items": [{"entry_key": "user:one"}],
            "cursor": "opaque-next",
            "relative_id": 42,
            "has_more": true
        }),
        &legacy_cursor_options(None),
        "items",
        "entry_key",
    );

    assert_eq!(result.relative_id.as_deref(), Some("opaque-next"));
    assert_eq!(result.protocol_error, None);
    assert_eq!(string_at(&result.root, "cursor"), Some("opaque-next"));
}

#[test]
fn terminal_legacy_response_removes_both_cursor_paths() {
    let existing = json!({
        "items": [{"entry_key": "user:one"}],
        "cursor": "opaque-old",
        "relative_id": 42,
        "has_more": true
    });
    let result = merge(
        Some(&existing),
        &json!({
            "items": [{"entry_key": "user:two"}],
            "relative_id": 41,
            "has_more": false
        }),
        &legacy_cursor_options(Some("opaque-old")),
        "items",
        "entry_key",
    );

    assert_eq!(result.relative_id, None);
    assert!(!result.has_more);
    assert!(result.root.get("cursor").is_none());
    assert!(result.root.get("relative_id").is_none());
}

#[test]
fn cursor_fallback_uses_boundary_item_and_preserves_its_wire_type() {
    let fallback_options = |cursor: &str| {
        relative_id_options(SaiWsPaginationRelativeIdArgs {
            relative_id: Some(cursor.to_string()),
            response_relative_id_fallback: Some(SaiWsPaginationCursorFallback::LastItemId),
            ..Default::default()
        })
    };
    let result = merge(
        Some(&json!({
            "items": [{"id": 1}],
            "relative_id": 1,
            "has_more": true
        })),
        &json!({
            "items": [{"id": 2}, {"id": 3}],
            "has_more": true
        }),
        &fallback_options("1"),
        "items",
        "id",
    );

    assert_eq!(result.relative_id.as_deref(), Some("3"));
    assert_eq!(result.root.get("relative_id").and_then(Value::as_i64), Some(3));
    assert_eq!(result.protocol_error, None);

    let terminal = merge(
        Some(&Value::Object(result.root.clone())),
        &json!({
            "items": [{"id": 4}],
            "relative_id": 4,
            "has_more": false
        }),
        &fallback_options("3"),
        "items",
        "id",
    );
    assert_eq!(terminal.relative_id, None);
    assert!(terminal.root.get("relative_id").is_none());
}

#[gpui::test]
fn initial_page_blocks_automatic_continuation_without_owning_either_edge(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let cache_key = unique_key("test-pagination-state");

        let initial_options = page_request_options(&cache_key, relative_id(None));
        let leading_options = page_request_options(
            &cache_key,
            relative_id_options(SaiWsPaginationRelativeIdArgs {
                edge: SaiWsPaginationEdge::Leading,
                relative_id: Some("cursor-1".to_string()),
                merge_direction: SaiWsPaginationMergeDirection::Prepend,
                ..Default::default()
            }),
        );
        let state = SaiWsPaginationStateStore::state(&cache_key, cx);

        let initial_binding = SaiWsPendingCacheBinding::new(&initial_options);
        SaiWsPaginationStateStore::begin(Some(&initial_binding), cx);
        {
            let state = state.read(cx);
            assert!(state.is_initial_page_loading());
            assert!(!state.leading().is_loading);
            assert!(!state.trailing().is_loading);
            assert!(!state.can_request_automatically(SaiWsPaginationEdge::Leading));
            assert!(!state.can_request_automatically(SaiWsPaginationEdge::Trailing));
        }

        SaiWsPaginationStateStore::finish(
            Some(&initial_binding),
            &merge_result(true, Some("cursor-1"), 1, 1, None),
            cx,
        );
        {
            let state = state.read(cx);
            assert!(!state.is_initial_page_loading());
            assert!(state.can_request_automatically(SaiWsPaginationEdge::Leading));
            assert!(state.can_request_automatically(SaiWsPaginationEdge::Trailing));
        }

        let binding = SaiWsPendingCacheBinding::new(&leading_options);
        SaiWsPaginationStateStore::begin(Some(&binding), cx);
        {
            let state = state.read(cx);
            assert!(state.leading().is_loading);
            assert!(!state.trailing().is_loading);
        }

        SaiWsPaginationStateStore::finish(
            Some(&binding),
            &merge_result(
                true,
                Some("cursor-1"),
                0,
                0,
                Some("Pagination relative_id did not advance."),
            ),
            cx,
        );
        {
            let state = state.read(cx);
            assert!(!state.leading().is_loading);
            assert_eq!(
                state.leading().error.as_deref(),
                Some("Pagination relative_id did not advance.")
            );
            assert_eq!(
                state.leading().failure_kind,
                Some(SaiWsPaginationFailureKind::ProtocolViolation)
            );
            assert_eq!(state.trailing().error, None);
        }

        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn initial_page_failure_and_cancellation_rearm_automatic_prefetch(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let cache_key = unique_key("test-pagination-initial-lifecycle");

        let options = page_request_options(&cache_key, relative_id(None));
        let binding = SaiWsPendingCacheBinding::new(&options);
        let state = SaiWsPaginationStateStore::state(&cache_key, cx);
        let trailing_open = |cx: &App| {
            state
                .read(cx)
                .can_request_automatically(SaiWsPaginationEdge::Trailing)
        };

        SaiWsPaginationStateStore::begin(Some(&binding), cx);
        assert!(!trailing_open(cx));
        SaiWsPaginationStateStore::fail(
            Some(&binding),
            &anyhow::Error::new(SaiWsPaginationError::ItemsAreNotArray("items".to_string())),
            cx,
        );
        assert!(!state.read(cx).is_initial_page_loading());
        assert!(trailing_open(cx));

        SaiWsPaginationStateStore::begin(Some(&binding), cx);
        assert!(!trailing_open(cx));
        SaiWsPaginationStateStore::cancel(Some(&binding), cx);
        assert!(!state.read(cx).is_initial_page_loading());
        assert!(trailing_open(cx));
        assert_eq!(state.read(cx).trailing().error, None);

        SaiWsPaginationStateStore::begin(Some(&binding), cx);
        SaiWsPaginationStateStore::finish(
            Some(&binding),
            &merge_result(true, None, 1, 1, Some("Initial pagination cursor is missing.")),
            cx,
        );
        {
            let state = state.read(cx);
            assert!(!state.is_initial_page_loading());
            assert_eq!(
                state.trailing().error.as_deref(),
                Some("Initial pagination cursor is missing.")
            );
            assert_eq!(
                state.trailing().failure_kind,
                Some(SaiWsPaginationFailureKind::ProtocolViolation)
            );
        }
        assert!(!trailing_open(cx));

        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn malformed_pagination_response_blocks_automatic_retry_as_protocol_failure(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let cache_key = unique_key("test-pagination-protocol-failure");

        let options = page_request_options(&cache_key, relative_id(Some("cursor-1")));
        let binding = SaiWsPendingCacheBinding::new(&options);
        let state = SaiWsPaginationStateStore::state(&cache_key, cx);

        SaiWsPaginationStateStore::begin(Some(&binding), cx);
        SaiWsPaginationStateStore::fail(
            Some(&binding),
            &anyhow::Error::new(SaiWsPaginationError::ItemsAreNotArray("items".to_string())),
            cx,
        );

        {
            let state = state.read(cx);
            assert!(!state.trailing().is_loading);
            assert!(!state.trailing().can_request_automatically());
            assert_eq!(
                state.trailing().failure_kind,
                Some(SaiWsPaginationFailureKind::ProtocolViolation)
            );
        }

        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn registry_stores_merged_root_in_cache_and_slice_but_callbacks_receive_wire_page(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let logical_key = unique_key("test-pagination-registry");
        let options = page_request_options(&logical_key, relative_id(Some("cursor-1")));
        let cache_key = options.cache_key.clone().expect("options carry a scoped cache key");
        let request_id = random_request_id();
        let existing = json!({
            "items": [
                {"id": "one", "value": "cached-one"},
                {"id": "two", "value": "cached-two"}
            ],
            "relative_id": "cursor-1",
            "has_more": true
        });
        SaiWsResponseCache::install_optimistic_root(
            existing.clone(),
            &cache_key,
            true,
            false,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            cx,
        )
        .expect("optimistic root installs");
        let slice = SaiTypedSlice::<SaiWsPaginationFixtureEnvelope>::new(cx);
        slice.mark_fulfilled(Some(&existing), None);
        let callback_page: Rc<RefCell<Option<SaiWsPaginationFixtureEnvelope>>> =
            Rc::new(RefCell::new(None));
        let callback_error: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        let registry = SaiWsPendingRegistry::shared(cx);
        let page_sink = callback_page.clone();
        let error_sink = callback_error.clone();
        let slice_state: Rc<dyn SaiSliceState> = slice.clone();
        registry.update(cx, |registry, cx| {
            registry.register(
                request_id,
                Some(SaiWsPendingCacheBinding::new(&options)),
                None,
                Some(slice_state),
                options.account_user_id.clone(),
                options.account_epoch.clone(),
                None,
                Some(Box::new(move |page: Option<Box<dyn Any>>, _cx: &mut App| {
                    *page_sink.borrow_mut() = page
                        .and_then(|page| page.downcast::<SaiWsPaginationFixtureEnvelope>().ok())
                        .map(|page| *page);
                })),
                Some(Box::new(move |error: &anyhow::Error, _cx: &mut App| {
                    *error_sink.borrow_mut() = Some(error.to_string());
                })),
                cx,
            );
        });

        let state = SaiWsPaginationStateStore::state(&cache_key, cx);
        assert!(state.read(cx).trailing().is_loading);
        let incoming = object(json!({
            "request_id": request_id.to_string(),
            "data": {
                "items": [
                    {"id": "two", "value": "wire-two"},
                    {"id": "three", "value": "wire-three"}
                ],
                "relative_id": "cursor-2",
                "has_more": false
            }
        }));
        let handled =
            registry.update(cx, |registry, cx| registry.handle_incoming_json_object(&incoming, cx));

        let cached_value = SaiWsResponseCache::memory_only(&cache_key, cx);
        let cached = dictionary(cached_value.as_ref());
        let cached_rows = dictionaries(cached.get("items"));
        let slice_data = slice.data();
        let page = callback_page.borrow().clone();

        assert!(handled);
        assert_eq!(*callback_error.borrow(), None);
        assert_eq!(
            page.as_ref()
                .map(|page| page.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>()),
            Some(vec!["two", "three"])
        );
        assert_eq!(page.as_ref().and_then(|page| page.relative_id.as_deref()), Some("cursor-2"));
        assert_eq!(slice.status(), SaiWsSliceStatus::Fulfilled);
        assert_eq!(
            slice_data
                .as_ref()
                .map(|data| data.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>()),
            Some(vec!["one", "two", "three"])
        );
        assert_eq!(
            slice_data
                .as_ref()
                .and_then(|data| data.items.iter().find(|item| item.id == "two"))
                .map(|item| item.value.as_str()),
            Some("cached-two")
        );
        assert_eq!(ids(&cached_rows), vec!["one", "two", "three"]);
        assert_eq!(string_at(cached, "relative_id"), Some("cursor-2"));
        assert!(!state.read(cx).trailing().is_loading);
        assert_eq!(state.read(cx).trailing().error, None);

        registry.update(cx, |registry, cx| registry.remove_pending(request_id, cx));
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn continuation_failure_keeps_fulfilled_slice_and_cache_while_rejecting_only_the_edge(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let logical_key = unique_key("test-pagination-registry-error");
        let options = page_request_options(
            &logical_key,
            relative_id_options(SaiWsPaginationRelativeIdArgs {
                edge: SaiWsPaginationEdge::Leading,
                relative_id: Some("cursor-1".to_string()),
                merge_direction: SaiWsPaginationMergeDirection::Prepend,
                ..Default::default()
            }),
        );
        let cache_key = options.cache_key.clone().expect("options carry a scoped cache key");
        let request_id = random_request_id();
        let existing = json!({
            "items": [{"id": "one", "value": "cached-one"}],
            "relative_id": "cursor-1",
            "has_more": true
        });
        SaiWsResponseCache::install_optimistic_root(
            existing.clone(),
            &cache_key,
            true,
            false,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            cx,
        )
        .expect("optimistic root installs");
        let slice = SaiTypedSlice::<SaiWsPaginationFixtureEnvelope>::new(cx);
        slice.mark_fulfilled(Some(&existing), None);
        let callback_error: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        let registry = SaiWsPendingRegistry::shared(cx);
        let error_sink = callback_error.clone();
        let slice_state: Rc<dyn SaiSliceState> = slice.clone();
        registry.update(cx, |registry, cx| {
            registry.register(
                request_id,
                Some(SaiWsPendingCacheBinding::new(&options)),
                None,
                Some(slice_state),
                options.account_user_id.clone(),
                options.account_epoch.clone(),
                None,
                None,
                Some(Box::new(move |error: &anyhow::Error, _cx: &mut App| {
                    *error_sink.borrow_mut() = Some(error.to_string());
                })),
                cx,
            );
        });
        let state = SaiWsPaginationStateStore::state(&cache_key, cx);
        assert!(state.read(cx).leading().is_loading);

        let incoming = object(json!({
            "request_id": request_id.to_string(),
            "status": "error",
            "error": {"message": "page failed"}
        }));
        let handled =
            registry.update(cx, |registry, cx| registry.handle_incoming_json_object(&incoming, cx));
        let cached_value = SaiWsResponseCache::memory_only(&cache_key, cx);
        let cached = dictionary(cached_value.as_ref());

        assert!(handled);
        assert_eq!(callback_error.borrow().as_deref(), Some("page failed"));
        assert_eq!(slice.status(), SaiWsSliceStatus::Fulfilled);
        assert_eq!(slice.error(), None);
        assert_eq!(
            slice
                .data()
                .map(|data| data.items.into_iter().map(|item| item.id).collect::<Vec<_>>()),
            Some(vec!["one".to_string()])
        );
        assert_eq!(ids(&dictionaries(cached.get("items"))), vec!["one"]);
        {
            let state = state.read(cx);
            assert!(!state.leading().is_loading);
            assert_eq!(state.leading().error.as_deref(), Some("page failed"));
            assert_eq!(
                state.leading().failure_kind,
                Some(SaiWsPaginationFailureKind::Transport)
            );
            assert_eq!(state.trailing().error, None);
        }

        registry.update(cx, |registry, cx| registry.remove_pending(request_id, cx));
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
async fn disk_first_keeps_dirty_pagination_memory_authoritative_until_disk_catches_up(
    cx: &mut TestAppContext,
) {
    let logical_cache_key = unique_key("test-pagination-disk-race");

    let old_payload = json!({
        "items": [{"id": "one", "value": "old"}],
        "relative_id": "cursor-1",
        "has_more": true
    });
    let merged_payload = json!({
        "items": [
            {"id": "one", "value": "old"},
            {"id": "two", "value": "new"}
        ],
        "relative_id": "cursor-2",
        "has_more": true
    });
    let options = disk_first_request_options(&logical_cache_key);
    let cache_key = options.cache_key.clone().expect("options carry a scoped cache key");

    cx.update(|cx| {
        SaiWsDiskCache::save(&cache_key, &old_payload).expect("old payload saves");
        SaiWsResponseCache::hydrate_memory(&cache_key, old_payload.clone(), false, cx);
        SaiWsResponseCache::store_success_payload_options(Some(merged_payload), &options, cx);

        SaiWsDiskCache::wait_for_asynchronous_writes();
        SaiWsDiskCache::save(&cache_key, &old_payload).expect("old payload saves again");

        let peeked_value = SaiWsResponseCache::peek(&options, None, cx).0;
        let peeked = dictionary(peeked_value.as_ref());
        let resolved_value = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        let resolved = dictionary(resolved_value.as_ref());
        assert_eq!(ids(&dictionaries(peeked.get("items"))), vec!["one", "two"]);
        assert_eq!(ids(&dictionaries(resolved.get("items"))), vec!["one", "two"]);
    });

    cx.run_until_parked();

    cx.update(|cx| SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx));
}

#[gpui::test]
async fn invalidated_disk_write_cannot_own_or_delay_a_new_cache_generation(
    cx: &mut TestAppContext,
) {
    let logical_cache_key = unique_key("test-pagination-disk-generation");

    let options = disk_first_request_options(&logical_cache_key);
    let cache_key = options.cache_key.clone().expect("options carry a scoped cache key");

    cx.update(|cx| {
        SaiWsResponseCache::store_success_payload_options(
            Some(json!({
                "items": [{"id": "old"}],
                "relative_id": "cursor-1",
                "has_more": true
            })),
            &options,
            cx,
        );

        SaiWsDiskCache::wait_for_asynchronous_writes();
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
        assert_eq!(SaiWsResponseCache::disk_only(&cache_key, cx), None);

        SaiWsResponseCache::store_success_payload_options(
            Some(json!({
                "items": [{"id": "new"}],
                "relative_id": "cursor-2",
                "has_more": false
            })),
            &options,
            cx,
        );
        SaiWsDiskCache::wait_for_asynchronous_writes();

        let before_value = SaiWsResponseCache::disk_only(&cache_key, cx);
        let before_old_completion = dictionary(before_value.as_ref());
        assert_eq!(ids(&dictionaries(before_old_completion.get("items"))), vec!["new"]);
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let after_value = SaiWsResponseCache::disk_only(&cache_key, cx);
        let after_old_completion = dictionary(after_value.as_ref());
        assert_eq!(ids(&dictionaries(after_old_completion.get("items"))), vec!["new"]);

        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}
