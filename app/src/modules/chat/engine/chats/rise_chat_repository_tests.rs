use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Context as _;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_core::rise_secure_key_store::{RiseEngineKeyStore, RiseEngineKeyStoreFuture};
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityViewLoadState, RiseEntityWindow, RiseEntityWindowQuery,
    RiseEntityWindowResult, RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::RiseStoreWriteBatch;
use rise_engine::rise_upload::rise_upload_progress::RiseUploadRealtimeProgressHub;
use rise_engine::rise_wire::rise_wire::{RiseWireAccountContext, RiseWireClosureSender, RiseWireSocketSession};
use rise_engine::rise_wire::rise_wire_contracts::{RiseWirePushEnvelope, RiseWireTimestamp};
use rise_engine::{RiseEngine, RiseEngineConfiguration, RiseEngineConfigurationArgs, RiseJSONValue};
use rise_platform::file_manager::FileManager;

use crate::modules::chat::engine::chats::rise_chat_repository::{RiseChatRepository, RiseChatRepositoryPage};
use crate::modules::chat::engine::chats::rise_chat_repository_models::{
    RiseChatOptimisticOperation, RiseChatWireCodec, RiseStoreWriteBatchConversationExt,
};
use crate::modules::chat::engine::core::rise_conversation_compatibility_bridge::RiseConversationCompatibilityBridge;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseChatEntityRecord, RiseConversationEntitySource, RiseMessageEntityRecord, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseMessagePresentationPreparer,
};
use crate::modules::chat::engine::core::rise_conversation_store_layout::RiseConversationStoreLayout;
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseMessageAuxiliaryInvalidation, RiseMessageAuxiliaryInvalidationBatchHook, RiseMessageAuxiliaryProjection,
    RiseMessageChatPreviewBatchHook, RiseMessageChatReadOptimisticHook, RiseMessagePreviewChange,
    RiseMessageReadReceiptHintHook,
};
use crate::modules::chat::engine::messages::rise_message_durable_outbox::{
    RiseCreateMessageDurableEndpoint, RiseCreateMessageDurablePayload,
};
use crate::modules::chat::engine::messages::rise_message_realtime::RiseMessageRepositoryRealtimeEvent;
use crate::modules::chat::engine::messages::rise_message_repository::{
    RiseMessageRepository, RiseMessageRepositoryEncodedRow,
};
use crate::modules::chat::engine::messages::rise_message_scope::{
    RiseMessageAnchor, RiseMessageScope, RiseMessageWindowConfiguration,
};
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::message::message_actions::message_types::MessageDeliveryStatus;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CreateMessageGatewayDTO, MessageWireDTO, RiseStoredOutgoingMessage,
};

#[derive(Default)]
struct RiseChatTestKeyStore {
    keys: Mutex<HashMap<String, Vec<u8>>>,
}

impl RiseEngineKeyStore for RiseChatTestKeyStore {
    fn key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(self.keys.lock().unwrap_or_else(PoisonError::into_inner).get(account_id).cloned()) })
    }

    fn create_key_if_absent<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let mut keys = self.keys.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(keys.entry(account_id.to_owned()).or_insert_with(|| vec![0x51; 32]).clone())
        })
    }

    fn delete_key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, ()> {
        Box::pin(async move {
            self.keys.lock().unwrap_or_else(PoisonError::into_inner).remove(account_id);
            Ok(())
        })
    }
}

#[derive(Default)]
struct RiseMessageAuxiliaryInvalidationProbe {
    values: Mutex<Vec<RiseMessageAuxiliaryInvalidation>>,
}

impl RiseMessageAuxiliaryInvalidationProbe {
    fn record(&self, invalidation: RiseMessageAuxiliaryInvalidation) -> RiseStoreWriteBatch {
        self.values.lock().unwrap_or_else(PoisonError::into_inner).push(invalidation);
        RiseStoreWriteBatch::empty()
    }

    fn recorded(&self) -> Vec<RiseMessageAuxiliaryInvalidation> {
        self.values.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

fn json_map<const N: usize>(entries: [(&str, RiseJSONValue); N]) -> BTreeMap<String, RiseJSONValue> {
    entries.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

fn json_object<const N: usize>(entries: [(&str, RiseJSONValue); N]) -> RiseJSONValue {
    RiseJSONValue::Object(json_map(entries))
}

fn json_string(value: &str) -> RiseJSONValue {
    RiseJSONValue::String(value.to_owned())
}

fn push(event_type: &str, data: RiseJSONValue, timestamp: f64) -> RiseWirePushEnvelope {
    RiseWirePushEnvelope::new(
        event_type.to_owned(),
        data,
        Some(RiseWireTimestamp::UnixSeconds(timestamp)),
        BTreeMap::new(),
    )
}

fn scope_of(chat_id: &str) -> RiseMessageScope {
    RiseMessageScope::new(chat_id.to_owned(), None, None)
}

fn temporary_alias(value: &str) -> RiseEntityAlias {
    RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), value)
}

fn server_alias(value: &str) -> RiseEntityAlias {
    RiseEntityAlias::new(RiseEntityAliasKind::server_id(), value)
}

async fn unread_of(repository: &RiseChatRepository, chat_id: &str) -> anyhow::Result<Option<i64>> {
    Ok(repository.chat(chat_id).await?.map(|record| record.wire.to_chat_info().unread_count))
}

async fn write_optional(engine: &RiseEngine, batch: Option<RiseStoreWriteBatch>) -> anyhow::Result<()> {
    engine.write(batch.context("expected a write batch")?).await?;
    Ok(())
}

#[tokio::test]
async fn test_empty_chat_page_is_still_a_materialized_offline_view() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        assert!(!repository.has_materialized_view(RiseChatListKind::Inbox).await?);

        write_page(&[], None, false, true, RiseChatListKind::Inbox, &engine, &repository).await?;

        let snapshot = repository.snapshot(RiseChatListKind::Inbox).await?;
        assert!(repository.has_materialized_view(RiseChatListKind::Inbox).await?);
        assert!(snapshot.rows.is_empty());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_empty_message_timeline_is_still_a_materialized_offline_view() -> anyhow::Result<()> {
    with_repository(|engine, chats| async move {
        let account_id = chats.account_id.clone();
        let repository = message_repository(&engine, &account_id, &chats);
        let scope = scope_of("empty-chat");
        assert!(!repository.has_materialized_view(&scope).await?);

        let revision = repository.next_version();

        repository.write_state(&scope, revision, None, None, false, false, RiseEntityViewLoadState::Loaded, None).await?;

        let snapshot =
            repository.snapshot(&scope, RiseMessageAnchor::Latest, RiseMessageWindowConfiguration::presentation()).await?;
        assert!(repository.has_materialized_view(&scope).await?);
        assert!(snapshot.rows.is_empty());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_continuation_pages_retain_durable_membership_beyond_presentation_window() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        for page_index in 0..3_i64 {
            let upper_bound = (page_index + 1) * 200;
            let wires = (page_index * 200..upper_bound)
                .map(|index| chat_wire(&format!("chat-{index}"), index % 4, index as f64))
                .collect::<anyhow::Result<Vec<_>>>()?;
            write_page(
                &wires,
                Some(format!("cursor-{upper_bound}")),
                page_index < 2,
                page_index == 0,
                RiseChatListKind::Inbox,
                &engine,
                &repository,
            )
            .await?;
        }

        let first_window = window(&engine, RiseChatListKind::Inbox, RiseEntityWindow::First { limit: 512 }).await?;
        let latest_window = window(&engine, RiseChatListKind::Inbox, RiseEntityWindow::Latest { limit: 512 }).await?;
        let all_visible_ids = first_window
            .items
            .iter()
            .chain(latest_window.items.iter())
            .map(|item| item.entity.local_id.clone())
            .collect::<HashSet<_>>();
        assert_eq!(first_window.total_count, 600);
        assert_eq!(latest_window.total_count, 600);
        assert_eq!(all_visible_ids.len(), 600);

        let presentation = repository.snapshot(RiseChatListKind::Inbox).await?;
        assert_eq!(presentation.rows.len(), 512);
        assert_eq!(presentation.loaded_row_count, 600);
        assert_eq!(presentation.cursor.as_deref(), Some("cursor-600"));
        assert!(!presentation.has_more);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_overlapping_continuation_page_updates_rows_without_duplicates_or_replacement() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let first = (0..200_i64)
            .map(|index| chat_wire(&format!("chat-{index}"), 0, index as f64))
            .collect::<anyhow::Result<Vec<_>>>()?;
        write_page(&first, Some("cursor-200".to_owned()), true, true, RiseChatListKind::Inbox, &engine, &repository)
            .await?;

        let continuation = (150..350_i64)
            .map(|index| chat_wire(&format!("chat-{index}"), if index == 175 { 17 } else { 0 }, index as f64))
            .collect::<anyhow::Result<Vec<_>>>()?;
        write_page(
            &continuation,
            Some("cursor-350".to_owned()),
            false,
            false,
            RiseChatListKind::Inbox,
            &engine,
            &repository,
        )
        .await?;

        let durable = window(&engine, RiseChatListKind::Inbox, RiseEntityWindow::First { limit: 512 }).await?;
        let durable_ids = durable.items.iter().map(|item| item.entity.local_id.clone()).collect::<HashSet<_>>();
        assert_eq!(durable.total_count, 350);
        assert_eq!(durable_ids.len(), 350);
        assert!(durable_ids.contains("chat-0"));
        assert!(durable_ids.contains("chat-349"));
        let overlapping_chat = repository.chat("chat-175").await?;
        assert_eq!(overlapping_chat.and_then(|record| record.wire.unread_count), Some(17));

        let presentation = repository.snapshot(RiseChatListKind::Inbox).await?;
        assert_eq!(presentation.loaded_row_count, 350);
        assert_eq!(presentation.cursor.as_deref(), Some("cursor-350"));
        assert!(!presentation.has_more);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_stale_inbox_page_cannot_undo_realtime_archive() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let original = chat_wire("chat-archive", 2, 10.0)?;
        seed(&original, RiseChatListKind::Inbox, &engine, &repository).await?;

        let stale_version = repository.next_version();
        let stale_rows = repository
            .encode_rows(std::slice::from_ref(&original), RiseChatListKind::Inbox, stale_version)
            .await?;
        let realtime = push("chat_archived", json_object([("chat_id", json_string(&original.id))]), 20.0);
        let maybe_realtime_batch = repository.make_realtime_batch(&realtime, None).await?;
        write_optional(&engine, maybe_realtime_batch).await?;

        let stale_batch = repository.make_page_batch(
            &stale_rows,
            &[],
            RiseChatListKind::Inbox,
            &RiseChatRepositoryPage {
                rows: vec![original.clone()],
                cursor: Some(original.id.clone()),
                has_more: false,
                aggregates: None,
            },
            stale_version,
            repository.next_version(),
            true,
        )?;
        engine.write(stale_batch).await?;

        let inbox_ids = ids(&engine, RiseChatListKind::Inbox).await?;
        let archive_ids = ids(&engine, RiseChatListKind::Archive).await?;
        assert!(inbox_ids.is_empty());
        assert_eq!(archive_ids, vec![original.id.clone()]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_read_rollback_preserves_realtime_unread_increment() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let chat = chat_wire("chat-read", 5, 10.0)?;
        seed(&chat, RiseChatListKind::Archive, &engine, &repository).await?;

        let token = repository.apply_optimistic(&chat.id, RiseChatOptimisticOperation::Read).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(0));

        let event = push(
            "message_created",
            json_object([
                ("id", json_string("message-1")),
                ("chat_id", json_string(&chat.id)),
                ("sender_id", json_string("another-user")),
                ("content", json_string("new")),
                ("created_at", RiseJSONValue::Number(20.0)),
            ]),
            20.0,
        );
        let maybe_realtime_batch = repository.make_realtime_batch(&event, None).await?;
        write_optional(&engine, maybe_realtime_batch).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(1));

        repository.rollback_optimistic(&token).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(6));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_read_optimistic_batch_clears_badge_before_acknowledgement() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let chat = chat_wire("chat-optimistic-read", 3, 10.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &repository).await?;

        let (token, batch) =
            repository.make_read_optimistic_batch(&chat.id).await?.context("expected an optimistic batch")?;
        engine.write(batch).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(0));

        let maybe_realtime_batch = repository
            .make_realtime_batch(&created_push("message-1", &chat.id, "another-user", 20.0), None)
            .await?;
        write_optional(&engine, maybe_realtime_batch).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(1));

        repository.rollback_optimistic(&token).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(4));
        Ok(())
    })
    .await
}

#[test]
fn test_topic_read_estimate_counts_only_incoming_unread_messages() -> anyhow::Result<()> {
    let account_id = "me";
    let entities = [
        ("incoming-unread", "another-user", false),
        ("incoming-read", "another-user", true),
        ("outgoing-unread", account_id, false),
        ("outgoing-read", account_id, true),
    ]
    .into_iter()
    .map(|(id, sender_id, is_read)| -> anyhow::Result<RiseStoreEntity> {
        let wire = RiseMessageWireCodec::setting_boolean(
            &message_wire(id, "chat-estimate", sender_id, 1.0)?,
            "is_read",
            is_read,
        )?;
        Ok(RiseStoreEntity::new(
            id,
            RiseMessageWireCodec::record_data(&RiseMessageEntityRecord::new(
                wire,
                RiseConversationEntitySource::ServerPage,
            ))?,
            1,
            1,
        ))
    })
    .collect::<anyhow::Result<Vec<_>>>()?;

    assert_eq!(RiseMessageRepository::incoming_unread_count(&entities, account_id), 1);
    Ok(())
}

#[tokio::test]
async fn test_topic_read_optimistic_batch_decrements_instead_of_zeroing_the_aggregate() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let chat = chat_wire("chat-forum-read", 7, 10.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &repository).await?;

        let (token, batch) = repository
            .make_topic_read_optimistic_batch(&chat.id, 3)
            .await?
            .context("expected an optimistic batch")?;
        engine.write(batch).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(4));

        repository.rollback_optimistic(&token).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(7));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_topic_read_estimate_is_clamped_and_skipped_when_it_clears_nothing() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let chat = chat_wire("chat-forum-clamp", 2, 10.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &repository).await?;

        let noop = repository.make_topic_read_optimistic_batch(&chat.id, 0).await?;
        assert!(noop.is_none());

        let (token, batch) = repository
            .make_topic_read_optimistic_batch(&chat.id, 9)
            .await?
            .context("expected an optimistic batch")?;
        engine.write(batch).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(0));

        repository.rollback_optimistic(&token).await?;
        assert_eq!(unread_of(&repository, &chat.id).await?, Some(2));
        Ok(())
    })
    .await
}

async fn write_read_preview(
    engine: &RiseEngine,
    repository: &RiseChatRepository,
    chat_id: &str,
    topic_id: Option<&str>,
    authoritative_unread: Option<i64>,
) -> anyhow::Result<()> {
    let version = repository.next_version();
    let batch = repository
        .make_message_preview_batch(
            RiseMessagePreviewChange::Read {
                chat_id: chat_id.to_owned(),
                topic_id: topic_id.map(str::to_owned),
                authoritative_unread,
                clears_marks: false,
            },
            version,
        )
        .await?;
    engine.write(batch).await?;
    Ok(())
}

#[tokio::test]
async fn test_topic_read_acknowledgement_applies_the_server_remainder() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let chat = chat_wire("chat-forum-ack", 7, 10.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &repository).await?;

        write_read_preview(&engine, &repository, &chat.id, Some(""), Some(2)).await?;

        assert_eq!(unread_of(&repository, &chat.id).await?, Some(2));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_topic_read_without_server_remainder_leaves_the_aggregate_alone() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let chat = chat_wire("chat-forum-no-remainder", 5, 10.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &repository).await?;

        write_read_preview(&engine, &repository, &chat.id, Some(""), None).await?;

        assert_eq!(unread_of(&repository, &chat.id).await?, Some(5));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_whole_chat_read_acknowledgement_prefers_the_server_remainder_over_zero() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let chat = chat_wire("chat-read-remainder", 4, 10.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &repository).await?;

        write_read_preview(&engine, &repository, &chat.id, None, Some(1)).await?;

        assert_eq!(unread_of(&repository, &chat.id).await?, Some(1));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_read_optimistic_batch_is_skipped_when_there_is_nothing_to_clear() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let read_chat = chat_wire("chat-already-read", 0, 10.0)?;
        seed(&read_chat, RiseChatListKind::Inbox, &engine, &repository).await?;
        let already_read = repository.make_read_optimistic_batch(&read_chat.id).await?;
        assert!(already_read.is_none());

        let missing = repository.make_read_optimistic_batch("chat-that-was-never-stored").await?;
        assert!(missing.is_none());

        let pending_chat = chat_wire("chat-pending", 2, 10.0)?;
        seed(&pending_chat, RiseChatListKind::Inbox, &engine, &repository).await?;
        repository.apply_optimistic(&pending_chat.id, RiseChatOptimisticOperation::Pin(true)).await?;
        let blocked = repository.make_read_optimistic_batch(&pending_chat.id).await?;
        assert!(blocked.is_none());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_chat_created_merges_server_alias_into_permanent_temp_identity() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let temp_id = "temp_chat_123";
        let local = chat_wire(temp_id, 0, 1.0)?;
        seed(&local, RiseChatListKind::Inbox, &engine, &repository).await?;

        let event = push(
            "chat_created",
            json_object([(
                "chat",
                json_object([
                    ("id", json_string("server-chat-123")),
                    ("temp_id", json_string(temp_id)),
                    ("title", json_string("Acknowledged")),
                ]),
            )]),
            2.0,
        );
        let maybe_batch = repository.make_realtime_batch(&event, None).await?;
        write_optional(&engine, maybe_batch).await?;

        let temp_local_id = repository.local_id(temp_id).await?;
        let server_local_id = repository.local_id("server-chat-123").await?;
        let server_wire_id = repository.chat("server-chat-123").await?.map(|record| record.wire.id);
        let inbox_ids = ids(&engine, RiseChatListKind::Inbox).await?;
        assert_eq!(temp_local_id.as_deref(), Some(temp_id));
        assert_eq!(server_local_id.as_deref(), Some(temp_id));
        assert_eq!(server_wire_id.as_deref(), Some("server-chat-123"));
        assert_eq!(inbox_ids, vec![temp_id.to_owned()]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_create_chat_acknowledgement_materializes_and_updates_single_inbox_row() -> anyhow::Result<()> {
    with_repository(|_engine, repository| async move {
        let created = chat_wire("created-chat", 0, 10.0)?;
        repository.store_created_chat(created.clone()).await?;

        let snapshot = repository.snapshot(RiseChatListKind::Inbox).await?;
        assert_eq!(snapshot.rows.iter().map(|row| row.wire.id.as_str()).collect::<Vec<_>>(), vec!["created-chat"]);
        assert_eq!(snapshot.rows.first().map(|row| row.locally_created), Some(false));

        let updated = RiseChatWireCodec::patch(
            &created,
            json_map([("title", json_string("Authoritative title")), ("updatedAt", RiseJSONValue::Number(11.0))]),
        )?;
        repository.store_created_chat(updated).await?;

        let snapshot = repository.snapshot(RiseChatListKind::Inbox).await?;
        assert_eq!(snapshot.rows.len(), 1);
        assert_eq!(snapshot.rows.first().and_then(|row| row.wire.title.as_deref()), Some("Authoritative title"));
        Ok(())
    })
    .await
}

#[test]
fn test_create_chat_compatibility_builds_authoritative_wire_from_minimal_ack() -> anyhow::Result<()> {
    let request = json_object([
        ("title", json_string("New group")),
        ("type", json_string("GROUP")),
        ("participants", RiseJSONValue::Array(vec![json_string("viewer"), json_string("friend")])),
    ]);
    let response = json_object([("chat_id", json_string("server-chat"))]);

    let wire = RiseConversationCompatibilityBridge::created_chat_wire(&response, &request)
        .context("expected a created chat wire")?;
    assert_eq!(wire.id, "server-chat");
    assert_eq!(wire.server_id.as_deref(), Some("server-chat"));
    assert_eq!(wire.title.as_deref(), Some("New group"));
    assert_eq!(wire.r#type.as_deref(), Some("GROUP"));
    assert_eq!(wire.role.as_deref(), Some("OWNER"));
    assert_eq!(wire.member_count, Some(2));
    Ok(())
}

#[tokio::test]
async fn test_request_and_archive_moves_keep_public_views_mutually_exclusive() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let chat = chat_wire("chat-exclusive", 1, 4.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &repository).await?;

        let request = push(
            "message_request_created",
            json_object([("chat", json_object([("id", json_string(&chat.id))]))]),
            5.0,
        );
        let maybe_request_batch = repository.make_realtime_batch(&request, None).await?;
        write_optional(&engine, maybe_request_batch).await?;
        let request_inbox_ids = ids(&engine, RiseChatListKind::Inbox).await?;
        let request_archive_ids = ids(&engine, RiseChatListKind::Archive).await?;
        let request_ids = ids(&engine, RiseChatListKind::Requests).await?;
        assert!(request_inbox_ids.is_empty());
        assert!(request_archive_ids.is_empty());
        assert_eq!(request_ids, vec![chat.id.clone()]);

        let accepted = push(
            "message_request_accepted",
            json_object([("chat", json_object([("id", json_string(&chat.id))]))]),
            6.0,
        );
        let maybe_accepted_batch = repository.make_realtime_batch(&accepted, None).await?;
        write_optional(&engine, maybe_accepted_batch).await?;
        let accepted_inbox_ids = ids(&engine, RiseChatListKind::Inbox).await?;
        let accepted_archive_ids = ids(&engine, RiseChatListKind::Archive).await?;
        let accepted_request_ids = ids(&engine, RiseChatListKind::Requests).await?;
        assert_eq!(accepted_inbox_ids, vec![chat.id.clone()]);
        assert!(accepted_archive_ids.is_empty());
        assert!(accepted_request_ids.is_empty());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_coalesced_message_preview_batches_do_not_lose_unread_increments() -> anyhow::Result<()> {
    with_repository(|engine, repository| async move {
        let chat = chat_wire("chat-coalesced", 0, 1.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &repository).await?;

        let first = message_wire("message-1", &chat.id, "another-user", 2.0)?;
        let second = message_wire("message-2", &chat.id, "another-user", 3.0)?;

        let first_batch = repository
            .make_message_preview_batch(
                RiseMessagePreviewChange::Created {
                    scope: scope_of(&chat.id),
                    message: first,
                    was_already_stored: false,
                },
                repository.next_version(),
            )
            .await?;
        let second_batch = repository
            .make_message_preview_batch(
                RiseMessagePreviewChange::Created {
                    scope: scope_of(&chat.id),
                    message: second,
                    was_already_stored: false,
                },
                repository.next_version(),
            )
            .await?;
        engine.write(RiseStoreWriteBatch::conversation_merging(vec![first_batch, second_batch])).await?;

        repository.reset_reducer_projection();
        let stored = repository.chat(&chat.id).await?.context("expected a stored chat")?;
        assert_eq!(stored.wire.unread_count, Some(2));
        assert_eq!(stored.wire.last_message.and_then(|preview| preview.id).as_deref(), Some("message-2"));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_duplicate_created_is_counted_once_and_older_replay_cannot_regress_preview() -> anyhow::Result<()> {
    with_repository(|engine, chats| async move {
        let chat = chat_wire("chat-replay", 0, 1.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &chats).await?;
        let account_id = chats.account_id.clone();
        let messages = message_repository(&engine, &account_id, &chats);

        let older_push = created_push("message-older", &chat.id, "another-user", 10.0);
        let first = messages
            .reduce(&older_push, RiseMessageRepositoryRealtimeEvent::Created)
            .await?
            .context("expected a created batch")?;
        let duplicate = messages
            .reduce(&older_push, RiseMessageRepositoryRealtimeEvent::Created)
            .await?
            .context("expected a duplicate batch")?;
        engine.write(RiseStoreWriteBatch::conversation_merging(vec![first, duplicate])).await?;

        let newer_push = created_push("message-newer", &chat.id, "another-user", 20.0);
        let maybe_newer = messages.reduce(&newer_push, RiseMessageRepositoryRealtimeEvent::Created).await?;
        write_optional(&engine, maybe_newer).await?;
        let maybe_older_replay = messages.reduce(&older_push, RiseMessageRepositoryRealtimeEvent::Created).await?;
        write_optional(&engine, maybe_older_replay).await?;

        chats.reset_reducer_projection();
        let stored = chats.chat(&chat.id).await?.context("expected a stored chat")?;
        assert_eq!(stored.wire.unread_count, Some(2));
        assert_eq!(stored.wire.last_message.and_then(|preview| preview.id).as_deref(), Some("message-newer"));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_created_ack_invalidates_content_for_existing_optimistic_message() -> anyhow::Result<()> {
    with_repository(|engine, chats| async move {
        let chat = chat_wire("chat-local-ack", 0, 1.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &chats).await?;
        let account_id = chats.account_id.clone();
        let probe = Arc::new(RiseMessageAuxiliaryInvalidationProbe::default());
        let hook_probe = probe.clone();
        let messages = RiseMessageRepository::new(
            engine.clone(),
            account_id.clone(),
            Arc::new(RiseUploadRealtimeProgressHub::default()),
            preview_hook(&chats),
            RiseMessageChatReadOptimisticHook::empty(),
            RiseMessageAuxiliaryInvalidationBatchHook::new(move |invalidation| {
                let probe = hook_probe.clone();
                async move { probe.record(invalidation) }.boxed()
            }),
            RiseMessageReadReceiptHintHook::empty(),
        );
        let wire = message_wire("message-local-ack", &chat.id, &account_id, 10.0)?;
        let local_id = "temp-local-ack";
        let record = RiseMessageEntityRecord {
            local_phase: RiseMessageLocalPhase::Queued,
            client_request_id: Some("request-local-ack".to_owned()),
            ..RiseMessageEntityRecord::new(wire.clone(), RiseConversationEntitySource::Optimistic)
        };
        let mut aliases = RiseConversationStoreLayout::message_aliases(&wire);
        aliases.push(temporary_alias(local_id));
        let row = RiseMessageRepositoryEncodedRow {
            local_id: local_id.to_owned(),
            data: RiseMessageWireCodec::record_data(&record)?,
            position: RiseConversationStoreLayout::message_position(&wire, local_id, RiseMessageLocalPhase::Remote),
            aliases,
            duplicate_local_ids: Vec::new(),
            entity_version: 0,
            is_unchanged: false,
        };
        let version = messages.next_version();
        let optimistic_batch = messages.entity_upsert_batch(&row, &[scope_of(&chat.id)], version, None);
        engine.write(optimistic_batch).await?;

        let push = created_push("message-local-ack", &chat.id, &account_id, 10.0);
        messages.reduce(&push, RiseMessageRepositoryRealtimeEvent::Created).await?;

        let invalidations = probe.recorded();
        assert_eq!(invalidations.len(), 1);
        assert_eq!(invalidations.first().map(|value| value.chat_id.as_str()), Some(chat.id.as_str()));
        assert_eq!(invalidations.first().map(|value| &value.projections), Some(&RiseMessageAuxiliaryProjection::content()));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_created_reconciles_media_into_permanent_optimistic_identity() -> anyhow::Result<()> {
    with_repository(|engine, chats| async move {
        let chat = chat_wire("chat-media-created", 0, 1.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &chats).await?;
        let account_id = chats.account_id.clone();
        let messages = message_repository(&engine, &account_id, &chats);
        let local_id = "temp_media_created";
        let optimistic = RiseMessageWireCodec::wire(json_map([
            ("id", json_string(local_id)),
            ("temp_id", json_string(local_id)),
            ("chat_id", json_string(&chat.id)),
            ("sender_id", json_string(&account_id)),
            ("content_type", json_string("video")),
            ("created_at", RiseJSONValue::Number(10.0)),
            ("is_temp", RiseJSONValue::Bool(true)),
            (
                "media_items",
                RiseJSONValue::Array(vec![
                    json_object([
                        ("media_type", json_string("video")),
                        ("file_url", json_string("file:///local/video.mov")),
                        ("thumbnail_url", json_string("file:///local/poster.jpg")),
                        ("local_preview_url", json_string("file:///local/poster.jpg")),
                        ("is_uploading", RiseJSONValue::Bool(true)),
                        ("upload_progress", RiseJSONValue::Number(42.0)),
                    ]),
                    json_object([
                        ("media_type", json_string("image")),
                        ("file_url", json_string("file:///local/second.jpg")),
                        ("local_preview_url", json_string("file:///local/second.jpg")),
                        ("is_uploading", RiseJSONValue::Bool(true)),
                        ("upload_progress", RiseJSONValue::Number(21.0)),
                    ]),
                ]),
            ),
        ]))?;
        let optimistic_record = RiseMessageEntityRecord {
            local_phase: RiseMessageLocalPhase::Uploading,
            client_request_id: Some("request-media-created".to_owned()),
            ..RiseMessageEntityRecord::new(optimistic.clone(), RiseConversationEntitySource::Optimistic)
        };
        let row = RiseMessageRepositoryEncodedRow {
            local_id: local_id.to_owned(),
            data: RiseMessageWireCodec::record_data(&optimistic_record)?,
            position: RiseConversationStoreLayout::message_position(
                &optimistic,
                local_id,
                RiseMessageLocalPhase::Remote,
            ),
            aliases: RiseConversationStoreLayout::message_aliases(&optimistic),
            duplicate_local_ids: Vec::new(),
            entity_version: 0,
            is_unchanged: false,
        };
        let optimistic_version = messages.next_version();
        let optimistic_batch = messages.entity_upsert_batch(&row, &[scope_of(&chat.id)], optimistic_version, None);
        engine.write(optimistic_batch).await?;

        let server_message_id = "server-media-created";
        let created = push(
            "message_created",
            json_object([
                ("id", json_string(server_message_id)),
                ("server_id", json_string(server_message_id)),
                ("temp_id", json_string(local_id)),
                ("chat_id", json_string(&chat.id)),
                ("sender_id", json_string(&account_id)),
                ("content_type", json_string("video")),
                ("created_at", RiseJSONValue::Number(11.0)),
                (
                    "media_items",
                    RiseJSONValue::Array(vec![json_object([
                        ("media_type", json_string("video")),
                        ("file_id", json_string("server-file-created")),
                        ("file_url", json_string("https://cdn.example/created.mp4")),
                        ("thumbnail_url", json_string("https://cdn.example/created.jpg")),
                        ("local_preview_url", json_string("https://cdn.example/wrong-preview.jpg")),
                    ])]),
                ),
            ]),
            11.0,
        );
        let maybe_created = messages.reduce(&created, RiseMessageRepositoryRealtimeEvent::Created).await?;
        write_optional(&engine, maybe_created).await?;

        let by_temporary_id = engine
            .entity_alias(RiseConversationStoreLayout::messages(), temporary_alias(local_id))
            .await?
            .context("expected the entity by temporary id")?;
        let by_server_id = engine
            .entity_alias(RiseConversationStoreLayout::messages(), server_alias(server_message_id))
            .await?
            .context("expected the entity by server id")?;
        assert_eq!(by_temporary_id.local_id, local_id);
        assert_eq!(by_server_id.local_id, local_id);

        let stored = RiseMessageWireCodec::record(&by_temporary_id.data)?;
        assert_eq!(stored.source, RiseConversationEntitySource::Realtime);
        assert_eq!(stored.local_phase, RiseMessageLocalPhase::Remote);
        assert_eq!(stored.client_request_id.as_deref(), Some("request-media-created"));
        assert_eq!(stored.wire.id.as_deref(), Some(local_id));
        assert_eq!(stored.wire.temp_id.as_deref(), Some(local_id));
        assert_eq!(stored.wire.server_id.as_deref(), Some(server_message_id));
        assert_eq!(stored.wire.is_temp, Some(false));
        let media = stored.wire.media_items.context("expected media items")?;
        assert_eq!(media.len(), 2);
        assert_eq!(media[0].file_id.as_deref(), Some("server-file-created"));
        assert_eq!(media[0].file_url.as_deref(), Some("https://cdn.example/created.mp4"));
        assert_eq!(media[0].local_preview_url.as_deref(), Some("file:///local/poster.jpg"));
        assert_eq!(media[0].is_uploading, Some(false));
        assert_eq!(media[0].upload_progress, Some(100.0));
        assert_eq!(media[1].file_url.as_deref(), Some("file:///local/second.jpg"));
        assert_eq!(media[1].local_preview_url.as_deref(), Some("file:///local/second.jpg"));
        assert_eq!(media[1].is_uploading, Some(false));
        assert_eq!(media[1].upload_progress, Some(100.0));
        Ok(())
    })
    .await
}

#[test]
fn test_acknowledgement_uses_same_stable_media_reconciliation() -> anyhow::Result<()> {
    let local_id = "temp_media_ack";
    let chat_id = "chat-media-ack";
    let optimistic = RiseMessageWireCodec::wire(json_map([
        ("id", json_string(local_id)),
        ("temp_id", json_string(local_id)),
        ("chat_id", json_string(chat_id)),
        ("content_type", json_string("image")),
        ("created_at", RiseJSONValue::Number(10.0)),
        ("is_temp", RiseJSONValue::Bool(true)),
        (
            "media_items",
            RiseJSONValue::Array(vec![
                json_object([
                    ("media_type", json_string("image")),
                    ("file_url", json_string("file:///local/first.jpg")),
                    ("local_preview_url", json_string("file:///local/first.jpg")),
                    ("is_uploading", RiseJSONValue::Bool(true)),
                    ("upload_progress", RiseJSONValue::Number(73.0)),
                ]),
                json_object([
                    ("media_type", json_string("image")),
                    ("file_url", json_string("file:///local/second.jpg")),
                    ("local_preview_url", json_string("file:///local/second.jpg")),
                    ("is_uploading", RiseJSONValue::Bool(true)),
                    ("upload_progress", RiseJSONValue::Number(37.0)),
                ]),
            ]),
        ),
    ]))?;
    let response = json_object([
        ("message_id", json_string("server-media-ack")),
        ("chat_id", json_string(chat_id)),
        ("success", RiseJSONValue::Bool(true)),
        (
            "message",
            json_object([
                ("id", json_string("server-media-ack")),
                ("server_id", json_string("server-media-ack")),
                ("chat_id", json_string(chat_id)),
                ("content_type", json_string("image")),
                ("created_at", RiseJSONValue::Number(11.0)),
                (
                    "media_items",
                    RiseJSONValue::Array(vec![json_object([
                        ("media_type", json_string("image")),
                        ("file_id", json_string("server-file-ack")),
                        ("file_url", json_string("https://cdn.example/ack.jpg")),
                        ("local_preview_url", json_string("https://cdn.example/wrong-preview.jpg")),
                    ])]),
                ),
            ]),
        ),
    ])
    .decode_as::<CreateMessageGatewayDTO>()?;

    let acknowledged =
        RiseMessageWireCodec::acknowledged_wire(&optimistic, &response, local_id, &scope_of(chat_id))?;

    assert_eq!(acknowledged.id.as_deref(), Some(local_id));
    assert_eq!(acknowledged.temp_id.as_deref(), Some(local_id));
    assert_eq!(acknowledged.server_id.as_deref(), Some("server-media-ack"));
    assert_eq!(acknowledged.is_temp, Some(false));
    let media = acknowledged.media_items.context("expected media items")?;
    assert_eq!(media.len(), 2);
    assert_eq!(media[0].file_id.as_deref(), Some("server-file-ack"));
    assert_eq!(media[0].file_url.as_deref(), Some("https://cdn.example/ack.jpg"));
    assert_eq!(media[0].local_preview_url.as_deref(), Some("file:///local/first.jpg"));
    assert_eq!(media[0].is_uploading, Some(false));
    assert_eq!(media[0].upload_progress, Some(100.0));
    assert_eq!(media[1].file_url.as_deref(), Some("file:///local/second.jpg"));
    assert_eq!(media[1].local_preview_url.as_deref(), Some("file:///local/second.jpg"));
    assert_eq!(media[1].is_uploading, Some(false));
    assert_eq!(media[1].upload_progress, Some(100.0));
    Ok(())
}

fn compact_media_item(file_id: &str, file_url: &str) -> RiseJSONValue {
    json_object([
        ("media_type", json_string("image")),
        ("file_id", json_string(file_id)),
        ("file_url", json_string(file_url)),
    ])
}

#[test]
fn test_cancelled_middle_slot_maps_compact_media_to_original_slots() -> anyhow::Result<()> {
    let local_id = "temp_media_middle_cancel";
    let chat_id = "chat-media-middle-cancel";
    let optimistic = RiseMessageWireCodec::wire(json_map([
        ("id", json_string(local_id)),
        ("temp_id", json_string(local_id)),
        ("chat_id", json_string(chat_id)),
        ("content_type", json_string("image")),
        ("created_at", RiseJSONValue::Number(10.0)),
        ("is_temp", RiseJSONValue::Bool(true)),
        (
            "media_items",
            RiseJSONValue::Array(
                (0..3_i32)
                    .map(|index| {
                        json_object([
                            ("media_type", json_string("image")),
                            ("file_url", json_string(&format!("file:///local/{index}.jpg"))),
                            ("local_preview_url", json_string(&format!("file:///local/{index}.jpg"))),
                            ("is_uploading", RiseJSONValue::Bool(true)),
                            ("upload_progress", RiseJSONValue::Number(f64::from(index + 1) * 10.0)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))?;
    let compact_body = json_object([
        ("chat_id", json_string(chat_id)),
        ("content", json_string("")),
        ("original_content", json_string("")),
        ("content_type", json_string("image")),
        (
            "media_items",
            RiseJSONValue::Array(vec![
                compact_media_item("uploaded-slot-0", "https://cdn.example/uploaded-0.jpg"),
                compact_media_item("uploaded-slot-2", "https://cdn.example/uploaded-2.jpg"),
            ]),
        ),
    ])
    .decode_as::<RiseStoredOutgoingMessage>()?;

    let durable = RiseMessageWireCodec::updating_optimistic_wire(
        &optimistic,
        &compact_body,
        &scope_of(chat_id),
        local_id,
        10.0,
        &HashSet::from([1]),
    )?;
    assert_stable_middle_cancellation(&durable, &["uploaded-slot-0", "uploaded-slot-2"])?;

    let authoritative = RiseMessageWireCodec::wire(json_map([
        ("id", json_string("server-middle-cancel")),
        ("server_id", json_string("server-middle-cancel")),
        ("chat_id", json_string(chat_id)),
        ("content_type", json_string("image")),
        ("created_at", RiseJSONValue::Number(11.0)),
        (
            "media_items",
            RiseJSONValue::Array(vec![
                compact_media_item("server-slot-0", "https://cdn.example/server-0.jpg"),
                compact_media_item("server-slot-2", "https://cdn.example/server-2.jpg"),
            ]),
        ),
    ]))?;
    let created = RiseMessageWireCodec::reconciling_authoritative_wire(&durable, &authoritative, local_id)?;
    assert_stable_middle_cancellation(&created, &["server-slot-0", "server-slot-2"])?;

    let response = json_object([
        ("message_id", json_string("server-middle-cancel")),
        ("chat_id", json_string(chat_id)),
        ("success", RiseJSONValue::Bool(true)),
        (
            "message",
            json_object([
                ("id", json_string("server-middle-cancel")),
                ("server_id", json_string("server-middle-cancel")),
                ("chat_id", json_string(chat_id)),
                ("content_type", json_string("image")),
                (
                    "media_items",
                    RiseJSONValue::Array(vec![
                        compact_media_item("ack-slot-0", "https://cdn.example/ack-0.jpg"),
                        compact_media_item("ack-slot-2", "https://cdn.example/ack-2.jpg"),
                    ]),
                ),
            ]),
        ),
    ])
    .decode_as::<CreateMessageGatewayDTO>()?;
    let acknowledged = RiseMessageWireCodec::acknowledged_wire(&durable, &response, local_id, &scope_of(chat_id))?;
    assert_stable_middle_cancellation(&acknowledged, &["ack-slot-0", "ack-slot-2"])?;

    let before = active_render_ids(&durable, local_id);
    let after = active_render_ids(&created, local_id);
    assert_eq!(before, after);
    assert_eq!(after, vec!["24:temp_media_middle_cancel:media:0", "24:temp_media_middle_cancel:media:2"]);
    Ok(())
}

fn active_render_ids(wire: &MessageWireDTO, local_id: &str) -> Vec<String> {
    wire.to_message()
        .media_items
        .iter()
        .filter(|item| !item.is_cancelled)
        .map(|item| item.stable_render_id(local_id))
        .collect()
}

fn assert_stable_middle_cancellation(
    wire: &MessageWireDTO,
    expected_active_file_ids: &[&str; 2],
) -> anyhow::Result<()> {
    let media = wire.media_items.as_ref().context("expected media items")?;
    assert_eq!(media.len(), 3);
    assert_eq!(media.iter().map(|item| item.local_slot_index).collect::<Vec<_>>(), vec![Some(0), Some(1), Some(2)]);
    assert_eq!(media[0].file_id.as_deref(), Some(expected_active_file_ids[0]));
    assert_eq!(media[0].local_preview_url.as_deref(), Some("file:///local/0.jpg"));
    assert_eq!(media[0].is_cancelled, Some(false));
    assert_eq!(media[1].file_id, None);
    assert_eq!(media[1].file_url.as_deref(), Some("file:///local/1.jpg"));
    assert_eq!(media[1].local_preview_url.as_deref(), Some("file:///local/1.jpg"));
    assert_eq!(media[1].is_cancelled, Some(true));
    assert_eq!(media[2].file_id.as_deref(), Some(expected_active_file_ids[1]));
    assert_eq!(media[2].local_preview_url.as_deref(), Some("file:///local/2.jpg"));
    assert_eq!(media[2].is_cancelled, Some(false));
    Ok(())
}

#[tokio::test]
async fn test_prepared_patch_owns_and_finishes_rise_cancellation_session() -> anyhow::Result<()> {
    with_repository(|engine, chats| async move {
        let chat = chat_wire("chat-prepared-patch-cancel", 0, 1.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &chats).await?;
        let account_id = chats.account_id.clone();
        let messages = message_repository(&engine, &account_id, &chats);
        let message_id = "message-prepared-patch-cancel";
        let created = created_push(message_id, &chat.id, &account_id, 10.0);
        let maybe_batch = messages.reduce(&created, RiseMessageRepositoryRealtimeEvent::Created).await?;
        write_optional(&engine, maybe_batch).await?;

        let checkpoint = messages.begin_prepared_patch(message_id, |wire| Ok(wire.clone())).await?;
        let local_id = checkpoint.entity.local_id.clone();
        assert!(!messages.is_upload_cancelled(&local_id, 0_i64).await);

        messages.cancel_upload_slot(&local_id, 0).await;
        assert!(messages.is_upload_cancelled(&local_id, 0_i64).await);

        messages.rollback_prepared_patch(checkpoint).await?;
        assert!(!messages.is_upload_cancelled(&local_id, 0_i64).await);

        messages.begin_prepared_patch_progress(&local_id).await;
        messages.cancel_upload_slot(&local_id, 0).await;
        assert!(messages.is_upload_cancelled(&local_id, 0_i64).await);
        messages.finish_prepared_patch(&local_id).await;
        assert!(!messages.is_upload_cancelled(&local_id, 0_i64).await);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn test_read_before_create_acknowledgement_does_not_strand_queued_message() -> anyhow::Result<()> {
    for reader in ["local-mutation", "local-event", "peer-event"] {
        with_repository(|engine, chats| async move {
            let account_id = chats.account_id.clone();
            let messages = message_repository(&engine, &account_id, &chats);
            let scope = scope_of("chat-read-before-ack");
            let local_id = "temp_read_before_ack";
            let optimistic = RiseMessageWireCodec::wire(json_map([
                ("id", json_string(local_id)),
                ("temp_id", json_string(local_id)),
                ("chat_id", json_string(&scope.chat_id)),
                ("sender_id", json_string(&account_id)),
                ("content", json_string("sent while chat is open")),
                ("content_type", json_string("text")),
                ("created_at", RiseJSONValue::Number(10.0)),
                ("is_temp", RiseJSONValue::Bool(true)),
                ("is_read", RiseJSONValue::Bool(false)),
            ]))?;
            let version = messages.next_version();
            let row = RiseMessageRepositoryEncodedRow {
                local_id: local_id.to_owned(),
                data: RiseMessageWireCodec::record_data(&RiseMessageEntityRecord {
                    local_phase: RiseMessageLocalPhase::Queued,
                    client_request_id: Some("42".to_owned()),
                    ..RiseMessageEntityRecord::new(optimistic.clone(), RiseConversationEntitySource::Optimistic)
                })?,
                position: RiseConversationStoreLayout::message_position(
                    &optimistic,
                    local_id,
                    RiseMessageLocalPhase::Queued,
                ),
                aliases: RiseConversationStoreLayout::message_aliases(&optimistic),
                duplicate_local_ids: Vec::new(),
                entity_version: 0,
                is_unchanged: false,
            };
            let initial_batch = messages.entity_upsert_batch(&row, std::slice::from_ref(&scope), version, None);
            engine.write(initial_batch).await?;
            let acknowledgement_version = messages.next_version();

            if reader == "local-mutation" {
                let window = engine
                    .entity_window(RiseEntityWindowQuery::new(
                        RiseConversationStoreLayout::messages(),
                        RiseConversationStoreLayout::message_view(&scope),
                        RiseEntityWindow::Latest { limit: 320 },
                    ))
                    .await?;
                let entities = window.items.into_iter().map(|item| item.entity).collect::<Vec<_>>();
                let originals = RiseMessageRepository::incoming_unread_entities(&entities, &account_id);
                assert!(originals.is_empty());
                let read_version = messages.next_version();
                let mutations = messages.read_mutations(&originals, true, read_version)?;
                engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
            } else {
                let read = read_push(
                    &scope.chat_id,
                    if reader == "local-event" { &account_id } else { "peer" },
                    "previous-server-message",
                    20.0,
                );
                if let Some(batch) = messages.reduce(&read, RiseMessageRepositoryRealtimeEvent::Read).await? {
                    engine.write(batch).await?;
                }
            }

            let pending = engine
                .entity_local_id(RiseConversationStoreLayout::messages(), local_id.to_owned())
                .await?;
            assert_eq!(pending.map(|entity| entity.version), Some(version), "{reader}");
            let body = serde_json::from_str::<RiseStoredOutgoingMessage>(
                r#"{"content":"sent while chat is open","original_content":"","content_type":"text"}"#,
            )?;
            let response =
                serde_json::from_str::<CreateMessageGatewayDTO>(r#"{"messageId":"server-confirmed","success":true}"#)?;
            let wire = RiseMessageWireCodec::acknowledged_wire(&optimistic, &response, local_id, &scope)?;
            engine
                .write(RiseCreateMessageDurableEndpoint::acknowledgement_batch(
                    &RiseCreateMessageDurablePayload {
                        body,
                        optimistic_wire: optimistic.clone(),
                        scope: scope.clone(),
                        materialized_scopes: vec![scope.clone()],
                        local_id: local_id.to_owned(),
                        client_request_id: "42".to_owned(),
                        version: acknowledgement_version,
                        retention_limit: 320,
                    },
                    &wire,
                )?)
                .await?;
            let snapshot = messages
                .snapshot(&scope, RiseMessageAnchor::Latest, RiseMessageWindowConfiguration::presentation())
                .await?;
            assert_eq!(snapshot.rows.len(), 1, "{reader}");
            assert_eq!(snapshot.rows.first().map(|row| row.local_phase), Some(RiseMessageLocalPhase::Remote), "{reader}");
            let presentation = RiseMessagePresentationPreparer::default()
                .prepare(snapshot, true)
                .await
                .context("expected a presentation")?;
            let message = presentation.messages.first().context("expected a message")?;
            assert_eq!(message.id, local_id, "{reader}");
            assert_eq!(message.server_id.as_deref(), Some("server-confirmed"), "{reader}");
            assert!(!message.is_pending_outgoing(), "{reader}");
            assert_eq!(message.delivery_status, MessageDeliveryStatus::Sent, "{reader}");
            Ok(())
        })
        .await?;
    }
    Ok(())
}

#[test]
fn test_local_read_selection_excludes_outgoing_and_pending_rows() -> anyhow::Result<()> {
    let account_id = "me";
    let cases = [
        ("incoming", "peer", RiseMessageLocalPhase::Remote, false),
        ("already-read", "peer", RiseMessageLocalPhase::Remote, true),
        ("sent", account_id, RiseMessageLocalPhase::Remote, false),
        ("queued", account_id, RiseMessageLocalPhase::Queued, false),
        ("uploading", account_id, RiseMessageLocalPhase::Uploading, false),
        ("sending", account_id, RiseMessageLocalPhase::Sending, false),
        ("failed", account_id, RiseMessageLocalPhase::Failed, false),
    ];
    let entities = cases
        .into_iter()
        .map(|(id, sender, phase, is_read)| -> anyhow::Result<RiseStoreEntity> {
            let wire =
                RiseMessageWireCodec::setting_boolean(&message_wire(id, "chat", sender, 1.0)?, "is_read", is_read)?;
            let source = if phase == RiseMessageLocalPhase::Remote {
                RiseConversationEntitySource::ServerPage
            } else {
                RiseConversationEntitySource::Optimistic
            };
            Ok(RiseStoreEntity::new(
                id,
                RiseMessageWireCodec::record_data(&RiseMessageEntityRecord {
                    local_phase: phase,
                    ..RiseMessageEntityRecord::new(wire, source)
                })?,
                1,
                1,
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let selected = RiseMessageRepository::incoming_unread_entities(&entities, account_id);
    assert_eq!(selected.iter().map(|entity| entity.local_id.as_str()).collect::<Vec<_>>(), vec!["incoming"]);
    Ok(())
}

#[tokio::test]
async fn test_remote_read_updates_receipt_without_clearing_local_unread() -> anyhow::Result<()> {
    with_repository(|engine, chats| async move {
        let chat = chat_wire("chat-receipt", 3, 1.0)?;
        seed(&chat, RiseChatListKind::Inbox, &engine, &chats).await?;
        let account_id = chats.account_id.clone();
        let messages = message_repository(&engine, &account_id, &chats);
        let outgoing_id = "message-outgoing";
        let outgoing = created_push(outgoing_id, &chat.id, &account_id, 10.0);
        let maybe_outgoing = messages.reduce(&outgoing, RiseMessageRepositoryRealtimeEvent::Created).await?;
        write_optional(&engine, maybe_outgoing).await?;

        let remote_receipt = read_push(&chat.id, "another-user", outgoing_id, 20.0);
        let maybe_remote_receipt = messages.reduce(&remote_receipt, RiseMessageRepositoryRealtimeEvent::Read).await?;
        write_optional(&engine, maybe_remote_receipt).await?;
        chats.reset_reducer_projection();

        let after_remote = chats.chat(&chat.id).await?.context("expected a stored chat")?;
        assert_eq!(after_remote.wire.unread_count, Some(3));
        assert_eq!(after_remote.wire.last_message.and_then(|preview| preview.is_read), Some(true));
        let outgoing_entity = engine
            .entity_alias(RiseConversationStoreLayout::messages(), server_alias(outgoing_id))
            .await?
            .context("expected the outgoing entity")?;
        let outgoing_record = RiseMessageWireCodec::record(&outgoing_entity.data)?;
        assert_eq!(outgoing_record.wire.is_read, Some(true));

        let local_read = read_push(&chat.id, &account_id, outgoing_id, 20.0);
        let maybe_local_read = messages.reduce(&local_read, RiseMessageRepositoryRealtimeEvent::Read).await?;
        write_optional(&engine, maybe_local_read).await?;
        chats.reset_reducer_projection();
        let after_local = chats.chat(&chat.id).await?.context("expected a stored chat")?;
        assert_eq!(after_local.wire.unread_count, Some(0));
        Ok(())
    })
    .await
}

async fn seed(
    wire: &RiseStoredChat,
    kind: RiseChatListKind,
    engine: &RiseEngine,
    repository: &RiseChatRepository,
) -> anyhow::Result<()> {
    let version = repository.next_version();
    let record = RiseChatEntityRecord {
        list_kind: Some(kind),
        ..RiseChatEntityRecord::new(wire.clone(), RiseConversationEntitySource::ServerPage)
    };
    let batch = repository.make_upsert_batch(
        record,
        &RiseConversationStoreLayout::local_chat_id(wire),
        RiseConversationStoreLayout::chat_aliases(wire),
        Some(kind),
        version,
        true,
        Vec::new(),
    )?;
    engine.write(batch).await?;
    Ok(())
}

async fn write_page(
    wires: &[RiseStoredChat],
    cursor: Option<String>,
    has_more: bool,
    replaces_loaded_view: bool,
    kind: RiseChatListKind,
    engine: &RiseEngine,
    repository: &RiseChatRepository,
) -> anyhow::Result<()> {
    let entity_version = repository.next_version();
    let rows = repository.encode_rows(wires, kind, entity_version).await?;
    let batch = repository.make_page_batch(
        &rows,
        &[],
        kind,
        &RiseChatRepositoryPage { rows: wires.to_vec(), cursor, has_more, aggregates: None },
        entity_version,
        repository.next_version(),
        replaces_loaded_view,
    )?;
    engine.write(batch).await?;
    Ok(())
}

async fn ids(engine: &RiseEngine, kind: RiseChatListKind) -> anyhow::Result<Vec<String>> {
    let result = window(engine, kind, RiseEntityWindow::First { limit: 100 }).await?;
    Ok(result.items.into_iter().map(|item| item.entity.local_id).collect())
}

async fn window(
    engine: &RiseEngine,
    kind: RiseChatListKind,
    window: RiseEntityWindow,
) -> anyhow::Result<RiseEntityWindowResult> {
    engine
        .entity_window(RiseEntityWindowQuery::new(
            RiseConversationStoreLayout::chats(),
            kind.store_view().raw_value(),
            window,
        ))
        .await
}

/// gateway sends the count twice and no local write updates the nested one
/// seeding both lets tests catch a projection reading the stale copy
fn chat_wire(id: &str, unread: i64, activity: f64) -> anyhow::Result<RiseStoredChat> {
    RiseChatWireCodec::patch(
        &RiseChatWireCodec::minimal_wire(id)?,
        json_map([
            ("unreadCount", RiseJSONValue::Integer(unread)),
            ("unreadInfo", json_object([("messages", json_object([("count", RiseJSONValue::Integer(unread))]))])),
            ("updatedAt", RiseJSONValue::Number(activity)),
            ("lastActivity", RiseJSONValue::Number(activity)),
            ("isPinned", RiseJSONValue::Bool(false)),
        ]),
    )
}

fn message_wire(id: &str, chat_id: &str, sender_id: &str, created_at: f64) -> anyhow::Result<MessageWireDTO> {
    Ok(json_object([
        ("id", json_string(id)),
        ("server_id", json_string(id)),
        ("chat_id", json_string(chat_id)),
        ("sender_id", json_string(sender_id)),
        ("content", json_string(id)),
        ("created_at", RiseJSONValue::Number(created_at)),
    ])
    .decode_as::<MessageWireDTO>()?)
}

fn preview_hook(chats: &Arc<RiseChatRepository>) -> RiseMessageChatPreviewBatchHook {
    let chats = chats.clone();
    RiseMessageChatPreviewBatchHook::new(move |change, version| {
        let chats = chats.clone();
        async move { chats.make_message_preview_batch(change, version).await }.boxed()
    })
}

fn message_repository(
    engine: &Arc<RiseEngine>,
    account_id: &str,
    chats: &Arc<RiseChatRepository>,
) -> Arc<RiseMessageRepository> {
    RiseMessageRepository::new(
        engine.clone(),
        account_id,
        Arc::new(RiseUploadRealtimeProgressHub::default()),
        preview_hook(chats),
        RiseMessageChatReadOptimisticHook::empty(),
        RiseMessageAuxiliaryInvalidationBatchHook::empty(),
        RiseMessageReadReceiptHintHook::empty(),
    )
}

fn created_push(message_id: &str, chat_id: &str, sender_id: &str, created_at: f64) -> RiseWirePushEnvelope {
    push(
        "message_created",
        json_object([
            ("id", json_string(message_id)),
            ("server_id", json_string(message_id)),
            ("chat_id", json_string(chat_id)),
            ("sender_id", json_string(sender_id)),
            ("content", json_string(message_id)),
            ("created_at", RiseJSONValue::Number(created_at)),
        ]),
        created_at,
    )
}

fn read_push(chat_id: &str, reader_id: &str, message_id: &str, cutoff: f64) -> RiseWirePushEnvelope {
    push(
        "chat_read",
        json_object([
            ("chat_id", json_string(chat_id)),
            ("user_id", json_string(reader_id)),
            ("last_read_message_id", json_string(message_id)),
            ("last_read_created_at", RiseJSONValue::Number(cutoff)),
            ("read_message_ids", RiseJSONValue::Array(vec![json_string(message_id)])),
        ]),
        cutoff,
    )
}

async fn with_repository<Body, BodyFuture>(body: Body) -> anyhow::Result<()>
where
    Body: FnOnce(Arc<RiseEngine>, Arc<RiseChatRepository>) -> BodyFuture,
    BodyFuture: Future<Output = anyhow::Result<()>>,
{
    let root = std::env::temp_dir().join(format!("rise-chat-repository-{}", Uuid::new_v4().uuid_string()));
    let account_id = format!("chat-tests-{}", Uuid::new_v4().uuid_string());
    let engine = RiseEngine::open(RiseEngineConfiguration::new(
        root.clone(),
        RiseWireAccountContext::new(account_id.clone(), "test".to_owned()),
        Arc::new(RiseWireClosureSender::new(Arc::new(
            |_frame: Vec<u8>, _session: RiseWireSocketSession| -> BoxFuture<'static, anyhow::Result<()>> {
                async { Ok(()) }.boxed()
            },
        ))),
        RiseEngineConfigurationArgs {
            key_store: Arc::new(RiseChatTestKeyStore::default()),
            ..RiseEngineConfigurationArgs::default()
        },
    ))
    .await?;
    let repository = RiseChatRepository::new(engine.clone(), account_id, None, None);
    match body(engine.clone(), repository).await {
        Ok(()) => {
            engine.close().await?;
            let _ = FileManager::default().remove_item(&root);
            Ok(())
        }
        Err(error) => {
            let _ = engine.close().await;
            let _ = FileManager::default().remove_item(&root);
            Err(error)
        }
    }
}
