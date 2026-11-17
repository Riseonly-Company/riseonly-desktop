use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, TestAppContext};
use serde_json::{Value, json};

use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{
    SaiWebSocketAutoReconnectStatus, SaiWebSocketManager, SaiWebSocketManagerPhase,
};
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredOutboundCoordinator;
use crate::core::lib::sai::core::websocket::sai_ws_response_debt_coordinator::SaiWsResponseDebtCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_coordinator::SaiWsOptimisticCoordinator;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticUpdate;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{
    SaiActiveScreen, SaiNavigationBridge, SaiWsScreenRefresh, SaiWsScreenRefreshParams,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_coordinator::SaiWsScreenRefreshCoordinator;

fn reset_all(cx: &mut App) {
    SaiWsPendingRegistry::shared(cx).update(cx, |registry, cx| registry.reset(cx));
    SaiWsOptimisticCoordinator::reset(cx);
    SaiWsResponseDebtCoordinator::shared(cx).update(cx, |debt, _| debt.reset());
    SaiWsDeferredOutboundCoordinator::shared(cx).update(cx, |deferred, _| deferred.reset());
}

fn pending_count(cx: &mut App) -> usize {
    SaiWsPendingRegistry::shared(cx).read(cx).test_pending_count()
}

fn in_flight_request_id(cache_key: &str, cx: &mut App) -> Option<u64> {
    SaiWsPendingRegistry::shared(cx)
        .read(cx)
        .in_flight_request_id(cache_key)
}

fn register_pending(request_id: u64, cache_key: &str, cx: &mut App) {
    SaiWsPendingRegistry::shared(cx).update(cx, |registry, cx| {
        registry.test_register_pending(request_id, Some(cache_key.to_string()), cx)
    });
}

fn reset_registry_after_socket_drop(cx: &mut App) {
    SaiWsPendingRegistry::shared(cx).update(cx, |registry, cx| registry.reset_after_socket_drop(cx));
}

fn install_empty_messages_root(cache_key: &str, cx: &mut App) {
    SaiWsResponseCache::install_optimistic_root(
        json!({"messages": []}),
        cache_key,
        true,
        false,
        SaiWsDiskCachePolicy::PreserveExisting,
        false,
        cx,
    )
    .expect("optimistic root installs");
}

fn optimistic_insert(temp_data: Value) -> SaiWsOptimisticUpdate {
    let mut optimistic = SaiWsOptimisticUpdate::default();
    optimistic.enabled = true;
    optimistic.create_temp_data = Some(Arc::new(
        move |_: &Value, _: &mut App| -> anyhow::Result<Value> { Ok(temp_data.clone()) },
    ));
    optimistic
}

fn create_message_options(
    cache_key: &str,
    optimistic: SaiWsOptimisticUpdate,
) -> SaiWsRequestOptions {
    SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
        service: "message".to_string(),
        method: "create_message".to_string(),
        cache_key: Some(cache_key.to_string()),
        path_to_array: Some("messages".to_string()),
        optimistic_update: Some(optimistic),
        ..Default::default()
    })
}

fn debt_record_count(cx: &mut App) -> usize {
    SaiWsResponseDebtCoordinator::shared(cx).read(cx).test_record_count()
}

fn fifo_count(cx: &mut App) -> usize {
    SaiWsDeferredOutboundCoordinator::shared(cx).read(cx).test_fifo_count()
}

fn deferred_disk_record_count(cx: &mut App) -> usize {
    SaiWsDeferredOutboundCoordinator::shared(cx)
        .read(cx)
        .test_disk_record_count()
}

fn enqueue_deferred(method: &str, cx: &mut App) {
    SaiWsDeferredOutboundCoordinator::shared(cx).update(cx, |deferred, _| {
        deferred.test_enqueue_without_continuation("message", method)
    });
}

fn fail_all_deferred(cx: &mut App) {
    SaiWsDeferredOutboundCoordinator::shared(cx).update(cx, |deferred, cx| deferred.fail_all_pending(anyhow::Error::new(SaiWsSendError::NotConnected), cx));
}

fn message_rows(cache_key: &str, cx: &mut App) -> Vec<Value> {
    let root = SaiWsResponseCache::current_payload(
        cache_key,
        SaiWsCachePreferredLayer::MemoryFirst,
        SaiWsDiskCachePolicy::PreserveExisting,
        cx,
    )
    .expect("cache holds a payload");
    root.as_object()
        .and_then(|root| root.get("messages"))
        .and_then(Value::as_array)
        .expect("payload holds a messages array")
        .clone()
}

fn reset_screen_refresh(cx: &mut App) {
    SaiWsScreenRefreshCoordinator::reset_account_state(cx);
    SaiNavigationBridge::install(Box::new(|_: &App| -> Option<SaiActiveScreen> { None }), cx);
}

#[gpui::test]
async fn screen_refresh_catch_up_runs_once_after_disconnect_but_not_on_initial_connect(
    cx: &mut TestAppContext,
) {
    let cache_key = "getMessages-chat-reconnect";
    let resend_count = Rc::new(Cell::new(0));

    cx.update(|cx| {
        SaiWsScreenRefreshCoordinator::reset_account_state(cx);
        SaiNavigationBridge::install(
            Box::new(|_: &App| {
                Some(SaiActiveScreen::new(
                    "Chat".to_string(),
                    HashMap::from([("chatId".to_string(), "chat-reconnect".to_string())]),
                ))
            }),
            cx,
        );

        let counter = resend_count.clone();
        SaiWsScreenRefreshCoordinator::register(
            cache_key,
            SaiWsScreenRefresh {
                enabled: true,
                when_on_screen: vec!["Chat".to_string()],
                route_params: Some(SaiWsScreenRefreshParams::PerScreen(HashMap::from([(
                    "Chat".to_string(),
                    HashMap::from([("chatId".to_string(), "chat-reconnect".to_string())]),
                )]))),
            },
            Box::new(move |_: &mut App| counter.set(counter.get() + 1)),
            cx,
        );
        SaiWsScreenRefreshCoordinator::mark_revalidated(cache_key, cx);

        SaiWsScreenRefreshCoordinator::on_transport_disconnected(cx);
    });
    cx.update(|cx| SaiWsScreenRefreshCoordinator::on_transport_connected(cx))
        .await;
    assert_eq!(resend_count.get(), 0);

    cx.update(|cx| SaiWsScreenRefreshCoordinator::on_transport_disconnected(cx));
    cx.update(|cx| SaiWsScreenRefreshCoordinator::on_transport_connected(cx))
        .await;
    assert_eq!(resend_count.get(), 1);

    cx.update(|cx| SaiWsScreenRefreshCoordinator::on_transport_connected(cx))
        .await;
    assert_eq!(resend_count.get(), 1);

    cx.update(reset_screen_refresh);
}

#[test]
fn acknowledged_outbound_uses_live_socket_instead_of_connected_queue() {
    assert!(!SaiWebSocketManager::should_defer_tracked_outbound(true, true));
    assert!(SaiWebSocketManager::should_defer_tracked_outbound(true, false));
    assert!(!SaiWebSocketManager::should_defer_tracked_outbound(false, false));
}

#[gpui::test]
fn socket_drop_clears_pending_and_optimistic_but_keeps_ack_debt(cx: &mut TestAppContext) {
    cx.update(|cx| {
        reset_all(cx);

        register_pending(5005, "getMessages-room1", cx);
        assert_eq!(pending_count(cx), 1);
        assert_eq!(in_flight_request_id("getMessages-room1", cx), Some(5005));

        install_empty_messages_root("getMessages-room1", cx);
        let options =
            create_message_options("getMessages-room1", optimistic_insert(json!({"content": "x"})));
        let outcome = SaiWsOptimisticCoordinator::apply_before_send_if_needed(
            5005,
            &json!({"content": "x"}),
            &options,
            cx,
        )
        .expect("optimistic insert applies");
        assert!(outcome.did_register_session);
        assert!(SaiWsOptimisticCoordinator::has_insert_session(5005, cx));

        let debt = SaiWsResponseDebtCoordinator::shared(cx);
        debt.update(cx, |debt, _| {
            debt.test_append_record(7007, "message", "create_message")
        });
        assert_eq!(debt_record_count(cx), 1);

        reset_registry_after_socket_drop(cx);
        debt.update(cx, |debt, _| debt.prepare_for_socket_transport_drop());

        assert_eq!(pending_count(cx), 0);
        assert_eq!(
            SaiWsPendingRegistry::shared(cx).read(cx).test_in_flight_count(),
            0
        );
        assert_eq!(in_flight_request_id("getMessages-room1", cx), None);

        assert!(!SaiWsOptimisticCoordinator::has_insert_session(5005, cx));

        assert_eq!(debt_record_count(cx), 1);
        assert!(debt.read(cx).test_contains_record(7007));
        assert_eq!(debt.read(cx).test_timer_count(), 0);

        SaiWsResponseCache::remove("getMessages-room1", SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn socket_drop_preserves_pending_and_optimistic_when_same_wire_id_is_ack_debt_tracked(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        reset_all(cx);

        let wire_id: u64 = 6006;
        register_pending(wire_id, "getMessages-room2", cx);

        install_empty_messages_root("getMessages-room2", cx);
        let options =
            create_message_options("getMessages-room2", optimistic_insert(json!({"content": "x"})));
        let outcome = SaiWsOptimisticCoordinator::apply_before_send_if_needed(
            wire_id,
            &json!({"content": "x"}),
            &options,
            cx,
        )
        .expect("optimistic insert applies");
        assert!(outcome.did_register_session);

        let debt = SaiWsResponseDebtCoordinator::shared(cx);
        debt.update(cx, |debt, _| {
            debt.test_append_record(wire_id, "message", "create_message")
        });

        reset_registry_after_socket_drop(cx);
        debt.update(cx, |debt, _| debt.prepare_for_socket_transport_drop());

        assert_eq!(pending_count(cx), 1);
        assert_eq!(in_flight_request_id("getMessages-room2", cx), Some(wire_id));
        assert!(SaiWsOptimisticCoordinator::has_insert_session(wire_id, cx));

        SaiWsResponseCache::remove("getMessages-room2", SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn socket_drop_preserves_optimistic_session_before_wire_send(cx: &mut TestAppContext) {
    cx.update(|cx| {
        reset_all(cx);

        let wire_id: u64 = 6_106;
        let cache_key = "getMessages-pre-wire-upload";
        install_empty_messages_root(cache_key, cx);

        let options =
            create_message_options(cache_key, optimistic_insert(json!({"content": "uploading"})));
        let outcome = SaiWsOptimisticCoordinator::apply_before_send_if_needed(
            wire_id,
            &json!({"content": "uploading"}),
            &options,
            cx,
        )
        .expect("optimistic insert applies");
        assert!(outcome.did_register_session);
        register_pending(wire_id, cache_key, cx);

        SaiWsOptimisticCoordinator::begin_pre_wire_optimistic(wire_id, cx);
        reset_registry_after_socket_drop(cx);
        assert!(SaiWsOptimisticCoordinator::has_insert_session(wire_id, cx));
        assert_eq!(pending_count(cx), 1);

        SaiWsOptimisticCoordinator::finish_pre_wire_optimistic(wire_id, cx);
        reset_registry_after_socket_drop(cx);
        assert!(!SaiWsOptimisticCoordinator::has_insert_session(wire_id, cx));
        assert_eq!(pending_count(cx), 0);

        SaiWsOptimisticCoordinator::finish_pre_wire_optimistic(wire_id, cx);
        reset_all(cx);
        SaiWsResponseCache::remove(cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn sensitive_request_is_never_mirrored_into_ack_debt_storage(cx: &mut TestAppContext) {
    cx.update(|cx| {
        reset_all(cx);
        let previous_feature_state = SaiWsResponseDebtCoordinator::is_feature_enabled();
        SaiWsResponseDebtCoordinator::set_is_feature_enabled(true);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: "user".to_string(),
            method: "upsert_saved_contact".to_string(),
            track_until_acknowledged: true,
            exclude_from_debugger: true,
            ..Default::default()
        });
        let debt = SaiWsResponseDebtCoordinator::shared(cx);
        debt.update(cx, |debt, cx| {
            debt.track_after_successful_wire_send_if_needed(
                8_008,
                &json!({
                    "first_name": "Private",
                    "note": "never persist"
                }),
                &options,
                cx,
            )
        });

        assert_eq!(debt_record_count(cx), 0);
        assert_eq!(debt.read(cx).test_disk_record_count(), 0);

        SaiWsResponseDebtCoordinator::set_is_feature_enabled(previous_feature_state);
        reset_all(cx);
    });
}

#[gpui::test]
fn deferred_outbound_preserves_fifo_order_and_drains_oldest_first(cx: &mut TestAppContext) {
    cx.update(|cx| {
        reset_all(cx);

        enqueue_deferred("first", cx);
        enqueue_deferred("second", cx);
        enqueue_deferred("third", cx);

        assert_eq!(fifo_count(cx), 3);
        assert_eq!(deferred_disk_record_count(cx), 3);
        assert_eq!(
            SaiWsDeferredOutboundCoordinator::shared(cx)
                .read(cx)
                .test_fifo_snapshot(),
            vec!["message.first", "message.second", "message.third"]
        );

        let drained = SaiWsDeferredOutboundCoordinator::shared(cx)
            .update(cx, |deferred, cx| deferred.test_drain_fifo_in_order(cx));
        assert_eq!(drained, vec!["message.first", "message.second", "message.third"]);
        assert_eq!(fifo_count(cx), 0);
        assert_eq!(deferred_disk_record_count(cx), 0);
    });
}

#[gpui::test]
fn deferred_outbound_applies_optimistic_insert_before_connection(cx: &mut TestAppContext) {
    cx.update(|cx| {
        reset_all(cx);

        let cache_key = "getMessages-offline-room";
        install_empty_messages_root(cache_key, cx);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: "message".to_string(),
            method: "create_message".to_string(),
            cache_key: Some(cache_key.to_string()),
            path_to_array: Some("messages".to_string()),
            path_to_id: Some("id".to_string()),
            optimistic_update: Some(optimistic_insert(json!({
                "id": "temp-offline",
                "content": "queued",
                "isTemp": true
            }))),
            track_until_acknowledged: true,
            ..Default::default()
        });

        let prepared_wire_request_id = SaiWsDeferredOutboundCoordinator::shared(cx)
            .update(cx, |deferred, cx| {
                deferred.test_enqueue_prepared_optimistic_without_continuation(
                    json!({"content": "queued"}),
                    options,
                    cx,
                )
            })
            .expect("prepared optimistic send enqueues");
        let wire_request_id =
            prepared_wire_request_id.expect("prepared send carries a wire request id");
        let messages = message_rows(cache_key, cx);

        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages
                .first()
                .and_then(|message| message.get("id"))
                .and_then(Value::as_str),
            Some("temp-offline")
        );
        assert!(SaiWsOptimisticCoordinator::has_insert_session(wire_request_id, cx));

        SaiWsOptimisticCoordinator::reset_after_socket_drop(&HashSet::new(), cx);

        assert!(SaiWsOptimisticCoordinator::has_insert_session(wire_request_id, cx));

        fail_all_deferred(cx);
        let reverted_messages = message_rows(cache_key, cx);

        assert!(reverted_messages.is_empty());
        SaiWsResponseCache::remove(cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn fail_all_pending_drops_fifo_and_disk_mirror(cx: &mut TestAppContext) {
    cx.update(|cx| {
        reset_all(cx);

        enqueue_deferred("a", cx);
        enqueue_deferred("b", cx);
        assert_eq!(fifo_count(cx), 2);

        fail_all_deferred(cx);

        assert_eq!(fifo_count(cx), 0);
        assert_eq!(deferred_disk_record_count(cx), 0);
    });
}

#[gpui::test]
fn manager_reset_restores_idle_state(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let manager = SaiWebSocketManager::shared(cx);
        manager.update(cx, |manager, cx| manager.reset(cx));
        let manager = manager.read(cx);
        assert_eq!(*manager.phase(), SaiWebSocketManagerPhase::Idle);
        assert!(!manager.is_connected());
        assert!(!manager.is_connecting());
        assert_eq!(
            *manager.auto_reconnect_status(),
            SaiWebSocketAutoReconnectStatus::Idle
        );
        assert_eq!(manager.connected_since(), None);
    });
}
