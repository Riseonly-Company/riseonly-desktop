use std::collections::{BTreeMap, HashMap, HashSet};

use gpui::{App, TestAppContext};
use rise_canon::rise_key::bot;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseJSONValue;
use rise_engine::rise_core::rise_identifiers::RiseRequestID;
use serde_json::Value;

use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_entry::{
    SaiHttpTraceEntry, SaiTraceTag, SaiTraceTransport,
};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::SaiHttpTraceStore;
use crate::core::lib::sai::modules::sai_updater::sai_updater::SaiUpdaterWriteScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsRequestOptions, SaiWsRequestOptionsInit};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationCacheWriteSource, RiseConversationResolutionSource,
};
use crate::modules::chat::engine::core::rise_conversation_debug_trace::{
    RiseConversationDebugTrace, RiseConversationDebugTraceFlowToken, RiseConversationDebugTraceRepositoryPolicy,
};
use crate::modules::chat::engine::core::rise_conversation_rpc::RiseConversationEndpoints;

fn fields(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn trace_entry(id: Uuid, cx: &mut App) -> Option<SaiHttpTraceEntry> {
    SaiHttpTraceStore::shared(cx).read(cx).entries().iter().find(|entry| entry.id == id).cloned()
}

fn remove_trace(id: Uuid, cx: &mut App) {
    SaiHttpTraceStore::shared(cx).update(cx, |store, cx| store.remove_entry(id, cx));
}

fn stored_entry(token: &RiseConversationDebugTraceFlowToken, cx: &mut TestAppContext) -> SaiHttpTraceEntry {
    let id = token.id;
    let entry = cx.update(|cx| trace_entry(id, cx)).expect("the flow keeps its row");
    cx.update(|cx| remove_trace(id, cx));
    entry
}

fn has_tag(entry: &SaiHttpTraceEntry, tag: &str) -> bool {
    entry.trace_tags.iter().any(|candidate| candidate.as_str() == tag)
}

fn header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers.get(name).map(String::as_str)
}

async fn normalized_repository_flow_owns_nested_rpc_until_presentation_publishes(cx: &mut TestAppContext) {
    let body_fields = fields(&[bot::CHAT_ID, "limit", "up"]);
    let token = RiseConversationDebugTrace::begin_repository_flow(
        &RiseConversationEndpoints::get_messages(),
        RiseConversationDebugTraceRepositoryPolicy::CacheFirst,
        true,
        &body_fields,
        Some("chat-1::t::topic-2"),
    )
    .await
    .expect("debug builds trace");

    RiseConversationDebugTrace::with_repository_flow(Some(&token), async {
        let span =
            RiseConversationDebugTrace::begin_network_endpoint(&RiseConversationEndpoints::get_messages(), &body_fields)
                .await;
        RiseConversationDebugTrace::finish_network(span, None, None).await;
    })
    .await;
    RiseConversationDebugTrace::complete_repository_flow(
        Some(&token),
        Some(RiseConversationResolutionSource::Network),
        RiseConversationCacheWriteSource::PersistentStore,
        false,
        false,
        true,
        None,
    )
    .await;

    let entry = stored_entry(&token, cx);
    assert_eq!(entry.url, "RiseEngine · normalized repository");
    assert_eq!(header(&entry.request_headers, "x-rise-owner"), Some("normalized-repository"));
    assert_eq!(header(&entry.request_headers, "x-rise-scope"), Some("chat-1::t::topic-2"));
    assert!(entry.request_body_wire.contains("\"scope\": \"chat-1::t::topic-2\""));
    assert_eq!(header(&entry.response_headers, "x-rise-rpc-attempts"), Some("1"));
    assert_eq!(header(&entry.response_headers, "x-rise-cache-lookup"), Some("persistent-store"));
    assert_eq!(header(&entry.response_headers, "x-rise-cache-write"), Some("persistent-store"));
    assert_eq!(header(&entry.response_headers, "x-rise-ui-result"), Some("published"));
    assert!(!has_tag(&entry, SaiTraceTag::DIRECT_RPC));
}

async fn durable_enqueue_is_a_store_owned_published_flow_without_fake_rpc(cx: &mut TestAppContext) {
    let token = RiseConversationDebugTrace::begin_repository_flow(
        &RiseConversationEndpoints::create_message(),
        RiseConversationDebugTraceRepositoryPolicy::DurableOutbox,
        true,
        &fields(&[bot::CHAT_ID, "content", "temp_id"]),
        None,
    )
    .await
    .expect("debug builds trace");

    RiseConversationDebugTrace::complete_repository_flow(
        Some(&token),
        Some(RiseConversationResolutionSource::NormalizedStore),
        RiseConversationCacheWriteSource::PersistentStore,
        false,
        false,
        true,
        None,
    )
    .await;

    let entry = stored_entry(&token, cx);
    assert_eq!(header(&entry.request_headers, "x-rise-policy"), Some("durable-outbox"));
    assert_eq!(header(&entry.response_headers, "x-rise-rpc-attempts"), Some("0"));
    assert_eq!(header(&entry.response_headers, "x-rise-source"), Some("normalized-store"));
    assert_eq!(header(&entry.response_headers, "x-rise-ui-result"), Some("published"));
    assert!(!has_tag(&entry, SaiTraceTag::DIRECT_RPC));
}

async fn unscoped_conversation_rpc_still_identifies_repository_ownership(cx: &mut TestAppContext) {
    let before: HashSet<Uuid> = cx.update(|cx| {
        SaiHttpTraceStore::shared(cx).read(cx).entries().iter().map(|entry| entry.id).collect()
    });
    let span = RiseConversationDebugTrace::begin_network_endpoint(
        &RiseConversationEndpoints::get_messages(),
        &fields(&[bot::CHAT_ID, "limit", "up"]),
    )
    .await;
    RiseConversationDebugTrace::finish_network(span, None, None).await;
    let entry = cx
        .update(|cx| {
            SaiHttpTraceStore::shared(cx)
                .read(cx)
                .entries()
                .iter()
                .find(|entry| {
                    !before.contains(&entry.id) && entry.fetch_id.as_deref() == Some("message.get_messages")
                })
                .cloned()
        })
        .expect("the RPC gets a fallback row");
    cx.update(|cx| remove_trace(entry.id, cx));

    assert_eq!(entry.url, "RiseEngine · repository RPC");
    assert_eq!(header(&entry.request_headers, "x-rise-owner"), Some("normalized-repository"));
    assert!(!has_tag(&entry, SaiTraceTag::DIRECT_RPC));
}

async fn durable_replay_flow_reports_the_real_worker_attempt_separately(cx: &mut TestAppContext) {
    let token = RiseConversationDebugTrace::begin_durable_replay_flow(
        &RiseConversationEndpoints::create_message(),
        RiseRequestID::new(51).expect("a positive request id"),
    )
    .await
    .expect("debug builds trace");
    let span =
        RiseConversationDebugTrace::begin_network_for(Some(&token), &RiseConversationEndpoints::create_message());
    RiseConversationDebugTrace::finish_network(span, None, None).await;
    RiseConversationDebugTrace::complete_repository_flow(
        Some(&token),
        Some(RiseConversationResolutionSource::Network),
        RiseConversationCacheWriteSource::PersistentStore,
        false,
        false,
        true,
        None,
    )
    .await;

    let entry = stored_entry(&token, cx);
    assert_eq!(entry.url, "RiseEngine · durable replay");
    assert_eq!(header(&entry.request_headers, "x-rise-owner"), Some("rise-sync"));
    assert_eq!(header(&entry.request_headers, "x-rise-policy"), Some("durable-replay"));
    assert_eq!(header(&entry.response_headers, "x-rise-rpc-attempts"), Some("1"));
    assert_eq!(header(&entry.response_headers, "x-rise-ui-result"), Some("published"));
}

#[gpui::test]
async fn flows_crossing_the_main_hop(cx: &mut TestAppContext) {
    cx.update(|cx| {
        RiseEngineMainHop::shared(cx);
    });
    normalized_repository_flow_owns_nested_rpc_until_presentation_publishes(cx).await;
    durable_enqueue_is_a_store_owned_published_flow_without_fake_rpc(cx).await;
    unscoped_conversation_rpc_still_identifies_repository_ownership(cx).await;
    durable_replay_flow_reports_the_real_worker_attempt_separately(cx).await;
}

#[test]
fn failed_rpc_attempt_remains_visible_in_the_compact_path() {
    let entry = make_entry(
        &[
            ("x-rise-cache-lookup", "miss"),
            ("x-rise-source", "failure"),
            ("x-rise-coalesced", "false"),
            ("x-rise-cache-write", "none"),
            ("x-rise-rpc-attempts", "1"),
            ("x-rise-rpc-failures", "1"),
            ("x-rise-ui-result", "not-published"),
        ],
        Some("RiseWireError"),
    );

    assert_eq!(entry.rise_path_line().as_deref(), Some("UI MISS → LOOKUP MISS → RPC ATTEMPT FAIL → FAILED"));
}

#[test]
fn fallback_path_does_not_claim_that_network_was_skipped() {
    let entry = make_entry(
        &[
            ("x-rise-cache-lookup", "persistent-store"),
            ("x-rise-source", "network-fallback-persistent-store"),
            ("x-rise-coalesced", "false"),
            ("x-rise-cache-write", "none"),
            ("x-rise-rpc-attempts", "1"),
            ("x-rise-rpc-failures", "1"),
            ("x-rise-ui-result", "published"),
            ("x-sai-served-from", "disk"),
        ],
        None,
    );

    assert_eq!(
        entry.rise_path_line().as_deref(),
        Some("UI MISS → LOOKUP RISESTORE → RPC FAIL / RISESTORE → PRESENTATION UPDATED")
    );
    assert!(!entry.copy_block_text().contains("disk (no network)"));
}

#[test]
fn recovered_rpc_path_keeps_the_failed_attempt_visible() {
    let entry = make_entry(
        &[
            ("x-rise-cache-lookup", "persistent-store"),
            ("x-rise-source", "network"),
            ("x-rise-coalesced", "false"),
            ("x-rise-cache-write", "persistent-store"),
            ("x-rise-rpc-attempts", "2"),
            ("x-rise-rpc-failures", "1"),
            ("x-rise-ui-result", "published"),
        ],
        None,
    );

    assert_eq!(
        entry.rise_path_line().as_deref(),
        Some("UI MISS → LOOKUP RISESTORE → RPC FAIL / RECOVERED → NETWORK → WRITE RISESTORE → PRESENTATION UPDATED")
    );
}

#[gpui::test]
fn legacy_disk_hydration_keeps_its_presentation_provenance(cx: &mut TestAppContext) {
    let key = format!("rise-debug-provenance-{}", Uuid::new_v4().uuid_string());
    cx.update(|cx| {
        SaiWsResponseCache::hydrate_memory(&key, Value::Object([("value".to_owned(), Value::Bool(true))].into_iter().collect()), true, cx);
        assert_eq!(SaiWsResponseCache::debug_presentation_source(&key, cx), "legacy-disk-hydrated");

        SaiWsResponseCache::hydrate_memory(&key, Value::Object([("value".to_owned(), Value::Bool(false))].into_iter().collect()), false, cx);
        assert_eq!(SaiWsResponseCache::debug_presentation_source(&key, cx), "memory-hit");

        SaiWsResponseCache::apply_sai_updater_payload(
            Value::Object([("value".to_owned(), Value::Bool(true))].into_iter().collect()),
            &key,
            SaiUpdaterWriteScope::Cache,
            true,
            cx,
        )
        .expect("an object payload applies");
        assert_eq!(SaiWsResponseCache::debug_presentation_source(&key, cx), "legacy-disk-hydrated");

        SaiWsResponseCache::remove(&key, SaiCacheCleanLayer::Memory, cx);
    });
}

#[gpui::test]
fn failed_flow_is_redacted_and_never_masquerades_as_normalized_store(cx: &mut TestAppContext) {
    let sensitive_cache_key = "private-chat-id|secret-search";
    let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: "chat".to_owned(),
        method: "get_topics".to_owned(),
        cache_key: Some(sensitive_cache_key.to_owned()),
        reflect_pending_in_ui: false,
        ..SaiWsRequestOptionsInit::default()
    });
    let body = RiseJSONValue::Object(BTreeMap::from([(
        bot::CHAT_ID.to_owned(),
        RiseJSONValue::String("private-chat-id".to_owned()),
    )]));
    let token = cx
        .update(|cx| RiseConversationDebugTrace::begin_flow(&body, &options, false, cx))
        .expect("debug builds trace");

    cx.update(|cx| {
        RiseConversationDebugTrace::complete_flow(
            Some(&token),
            None,
            None,
            RiseConversationCacheWriteSource::None,
            false,
            false,
            false,
            Some("TestFailure"),
            cx,
        );
    });

    let entry = stored_entry(&token, cx);
    assert_eq!(header(&entry.response_headers, "x-rise-source"), Some("failure"));
    assert!(!has_tag(&entry, SaiTraceTag::NORMALIZED_STORE));
    assert!(!entry.response_body_preview.contains("normalized-store"));
    assert!(!entry.request_body_wire.contains("private-chat-id"));
    assert!(!entry.request_headers.values().any(|value| value.contains(sensitive_cache_key)));
}

fn make_entry(response_headers: &[(&str, &str)], transport_error: Option<&str>) -> SaiHttpTraceEntry {
    SaiHttpTraceEntry {
        id: Uuid::new_v4(),
        transport: SaiTraceTransport::Websocket,
        created_at: Date::now(),
        is_pending: false,
        method: "READ".to_owned(),
        url: "RiseEngine".to_owned(),
        fetch_id: Some("chat.get_topics".to_owned()),
        duration_ms: 10,
        request_headers: HashMap::from([("x-rise-presentation-cache".to_owned(), "miss".to_owned())]),
        request_body_wire: "{\"payload\":\"omitted\"}".to_owned(),
        response_status: if transport_error.is_none() { 200 } else { 0 },
        response_headers: response_headers.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect(),
        response_body_preview: "{\"payload\":\"omitted\"}".to_owned(),
        transport_error: transport_error.map(str::to_owned),
        trace_tags: vec![SaiTraceTag::RISE_ENGINE.to_owned()],
    }
}
