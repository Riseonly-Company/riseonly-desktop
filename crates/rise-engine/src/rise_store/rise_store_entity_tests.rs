use std::path::PathBuf;
use std::sync::Arc;

use rise_canon::rise_store::MIGRATIONS;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::FileManager;

use crate::rise_store::rise_sq_lite::RiseSQLiteConnection;
use crate::rise_store::rise_store::{RiseStore, RiseStoreConfiguration, RiseSystemClock};
use crate::rise_store::rise_store_cipher::{RiseAESGCMStoreCipher, RisePlaintextStoreCipher};
use crate::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityViewLoadState, RiseEntityViewMembership,
    RiseEntityViewPosition, RiseEntityViewRetention, RiseEntityViewState, RiseEntityWindow,
    RiseEntityWindowQuery, RiseEntityWindowResult, RiseEntityWritePolicy, RisePositionKey, RisePositionKeyOrder,
    RiseStoreEntity,
};
use crate::rise_store::rise_store_models::{
    RiseStoreError, RiseStoreLimits, RiseStoreMutation, RiseStoreNamespace,
    RiseStoreSnapshotRequest, RiseStoreViewKey, RiseStoreWriteBatch,
};

fn namespace() -> RiseStoreNamespace {
    RiseStoreNamespace::new("entity-tests")
}

fn data(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

fn message_key(timestamp: i64, server_id: &str) -> RisePositionKey {
    RisePositionKey::make(|builder| {
        builder.append_signed_int64_with_order(timestamp, RisePositionKeyOrder::Ascending);
        builder.append_string_with_order(server_id, RisePositionKeyOrder::Descending);
    })
}

fn entity(local_id: &str, payload: &str, version: i64, updated_at_milliseconds: i64) -> RiseStoreEntity {
    RiseStoreEntity::new(local_id, data(payload), version, updated_at_milliseconds)
}

fn batch(mutations: Vec<RiseStoreMutation>) -> RiseStoreWriteBatch {
    RiseStoreWriteBatch::new(mutations, Default::default())
}

fn upsert(entity: &RiseStoreEntity) -> RiseStoreMutation {
    upsert_with_policy(entity, RiseEntityWritePolicy::RequireIdenticalReplay)
}

fn upsert_with_policy(entity: &RiseStoreEntity, policy: RiseEntityWritePolicy) -> RiseStoreMutation {
    RiseStoreMutation::UpsertEntity {
        namespace: namespace(),
        entity: entity.clone(),
        policy,
    }
}

fn compare_and_swap(entity: &RiseStoreEntity, expected: &RiseStoreEntity) -> RiseStoreMutation {
    upsert_with_policy(
        entity,
        RiseEntityWritePolicy::CompareAndSwap {
            expected: expected.clone(),
        },
    )
}

fn remove_entity(local_id: &str) -> RiseStoreMutation {
    RiseStoreMutation::RemoveEntity {
        namespace: namespace(),
        local_id: local_id.to_owned(),
    }
}

fn membership(view: &str, local_id: &str, position_key: RisePositionKey, version: i64) -> RiseStoreMutation {
    RiseStoreMutation::UpsertEntityViewMembership {
        namespace: namespace(),
        view: view.to_owned(),
        membership: RiseEntityViewMembership::new(local_id, position_key, version),
    }
}

fn set_alias(alias: &RiseEntityAlias, local_id: &str) -> RiseStoreMutation {
    RiseStoreMutation::SetEntityAlias {
        namespace: namespace(),
        alias: alias.clone(),
        local_id: local_id.to_owned(),
    }
}

fn view_state(
    revision: i64,
    has_more_before: bool,
    has_more_after: bool,
    updated_at_milliseconds: i64,
) -> RiseEntityViewState {
    RiseEntityViewState::new(
        revision,
        None,
        None,
        has_more_before,
        has_more_after,
        RiseEntityViewLoadState::Loaded,
        None,
        updated_at_milliseconds,
    )
}

fn window(view: &str, window: RiseEntityWindow) -> RiseEntityWindowQuery {
    RiseEntityWindowQuery::new(namespace(), view, window)
}

fn latest(view: &str, limit: usize) -> RiseEntityWindowQuery {
    window(view, RiseEntityWindow::Latest { limit })
}

async fn open(account_id: &str) -> Result<Arc<RiseStore>, RiseStoreError> {
    let configuration = RiseStoreConfiguration::in_memory_default(account_id.to_owned());
    RiseStore::open(configuration).await
}

async fn stored(store: &Arc<RiseStore>, local_id: &str) -> Result<Option<RiseStoreEntity>, RiseStoreError> {
    store.entity_local_id(namespace(), local_id.to_owned()).await
}

async fn aliased(store: &Arc<RiseStore>, alias: &RiseEntityAlias) -> Result<Option<RiseStoreEntity>, RiseStoreError> {
    store.entity_alias(namespace(), alias.clone()).await
}

fn local_ids(result: &RiseEntityWindowResult) -> Vec<String> {
    result
        .items
        .iter()
        .map(|item| item.entity.local_id.clone())
        .collect()
}

async fn close(store: &Arc<RiseStore>) {
    let _ = store.close().await;
}

#[tokio::test]
async fn compare_and_swap_rejects_reverse_completion_deletion_and_changed_base() -> Result<(), RiseStoreError> {
    let store = open("avatar-cas").await?;
    let base = entity("chat", "old-preview;unread=1;photo=old", 1, 1);
    let avatar = entity("chat", "old-preview;unread=1;photo=new", 3, 3);
    let realtime = entity("chat", "new-preview;unread=2;photo=old", 2, 2);
    store.write(batch(vec![upsert(&base)])).await?;
    store.write(batch(vec![upsert(&realtime)])).await?;
    let rejected = store.write(batch(vec![compare_and_swap(&avatar, &base)])).await?;
    assert!(rejected.changed_views.is_empty());
    assert_eq!(stored(&store, "chat").await?, Some(realtime.clone()));

    store.write(batch(vec![remove_entity("chat")])).await?;
    store.write(batch(vec![compare_and_swap(&avatar, &realtime)])).await?;
    assert_eq!(stored(&store, "chat").await?, None);

    store.write(batch(vec![upsert(&base)])).await?;
    let changed_at_same_version = entity("chat", "tombstone", 1, 1);
    store
        .write(batch(vec![upsert_with_policy(
            &changed_at_same_version,
            RiseEntityWritePolicy::ServerAuthoritative,
        )]))
        .await?;
    store.write(batch(vec![compare_and_swap(&avatar, &base)])).await?;
    assert_eq!(stored(&store, "chat").await?, Some(changed_at_same_version));
    close(&store).await;
    Ok(())
}

#[tokio::test]
async fn compare_and_swap_preserves_membership_and_accepts_only_increasing_version() -> Result<(), RiseStoreError> {
    let store = open("avatar-cas-membership").await?;
    let base = entity("chat", "preview;unread=2;photo=old", 2, 2);
    let next = entity("chat", "preview;unread=2;photo=new", 3, 3);
    let position = message_key(2, "chat");
    store
        .write(batch(vec![
            upsert(&base),
            membership("inbox", "chat", position.clone(), 2),
        ]))
        .await?;
    store.write(batch(vec![compare_and_swap(&next, &base)])).await?;
    assert_eq!(stored(&store, "chat").await?, Some(next.clone()));
    let page = store.entity_window(latest("inbox", 1)).await?;
    assert_eq!(page.items.first().map(|item| &item.entity), Some(&next));
    assert_eq!(page.items.first().map(|item| &item.position_key), Some(&position));
    assert_eq!(page.items.first().map(|item| item.membership_version), Some(3));
    let same_version = entity("chat", "overwrite", 3, 4);
    store.write(batch(vec![compare_and_swap(&same_version, &next)])).await?;
    assert_eq!(stored(&store, "chat").await?, Some(next));
    close(&store).await;
    Ok(())
}

#[tokio::test]
async fn avatar_patch_survives_replacing_head_refresh_and_preserves_other_views() -> Result<(), RiseStoreError> {
    let store = open("avatar-tail").await?;
    let base = entity("tail", "preview;photo=old", 2, 2);
    let avatar = entity("tail", "preview;photo=new", 3, 3);
    let position = message_key(2, "tail");
    let folder_position = message_key(1, "tail");
    store
        .write(batch(vec![
            upsert(&base),
            membership("inbox", "tail", position.clone(), 2),
            membership("folder", "tail", folder_position.clone(), 5),
        ]))
        .await?;
    store.write(batch(vec![compare_and_swap(&avatar, &base)])).await?;
    let tail = store.entity_window(latest("inbox", 1)).await?;
    let preserved = tail
        .items
        .first()
        .cloned()
        .expect("inbox keeps the patched tail membership");
    let head = entity("head", "head", 4, 4);
    store
        .write(batch(vec![
            RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace: namespace(),
                view: "inbox".to_owned(),
                if_version_at_most: Some(4),
            },
            membership(
                "inbox",
                "tail",
                preserved.position_key.clone(),
                preserved.membership_version,
            ),
            upsert(&head),
            membership("inbox", "head", message_key(4, "head"), 4),
        ]))
        .await?;
    let refreshed = store.entity_window(latest("inbox", 2)).await?;
    assert_eq!(refreshed.items.len(), 2);
    let refreshed_tail = refreshed
        .items
        .iter()
        .find(|item| item.entity.local_id == "tail");
    assert_eq!(refreshed_tail.map(|item| &item.entity), Some(&avatar));
    assert_eq!(refreshed_tail.map(|item| &item.position_key), Some(&position));
    let folder = store.entity_window(latest("folder", 1)).await?;
    assert_eq!(folder.items.first().map(|item| item.membership_version), Some(5));
    assert_eq!(
        folder.items.first().map(|item| &item.position_key),
        Some(&folder_position)
    );
    close(&store).await;
    Ok(())
}

#[tokio::test]
async fn avatar_conflict_rebases_on_latest_preview_without_reviving_deletion() -> Result<(), RiseStoreError> {
    let store = open("avatar-rebase").await?;
    let base = entity("chat", "old-preview;unread=1;photo=old", 1, 1);
    let realtime = entity("chat", "new-preview;unread=2;photo=old", 2, 2);
    let stale_avatar = entity("chat", "old-preview;unread=1;photo=new", 3, 3);
    let position = message_key(2, "chat");
    store
        .write(batch(vec![
            upsert(&base),
            upsert(&realtime),
            membership("inbox", "chat", position.clone(), 2),
        ]))
        .await?;
    store.write(batch(vec![compare_and_swap(&stale_avatar, &base)])).await?;
    let unchanged = store.entity_window(latest("inbox", 1)).await?;
    assert_eq!(unchanged.items.first().map(|item| item.membership_version), Some(2));
    let fresh = stored(&store, "chat")
        .await?
        .expect("the realtime preview is stored");
    let rebased = entity("chat", "new-preview;unread=2;photo=new", 4, 4);
    store.write(batch(vec![compare_and_swap(&rebased, &fresh)])).await?;
    assert_eq!(stored(&store, "chat").await?, Some(rebased.clone()));
    let updated = store.entity_window(latest("inbox", 1)).await?;
    assert_eq!(updated.items.first().map(|item| item.membership_version), Some(4));
    assert_eq!(updated.items.first().map(|item| &item.position_key), Some(&position));
    store.write(batch(vec![remove_entity("chat")])).await?;
    store.write(batch(vec![compare_and_swap(&rebased, &fresh)])).await?;
    assert_eq!(stored(&store, "chat").await?, None);
    close(&store).await;
    Ok(())
}

#[test]
fn position_keys_order_signed_strings_and_mixed_schemas_deterministically() {
    fn sorted(keys: &[RisePositionKey]) -> Vec<RisePositionKey> {
        let mut keys = keys.to_vec();
        keys.sort();
        keys
    }

    let signed: [i64; 7] = [i64::MIN, -10, -1, 0, 1, 10, i64::MAX];
    let signed_ascending: Vec<RisePositionKey> = signed
        .iter()
        .map(|value| {
            RisePositionKey::make(|builder| {
                builder.append_signed_int64_with_order(*value, RisePositionKeyOrder::Ascending);
            })
        })
        .collect();
    assert_eq!(signed_ascending, sorted(&signed_ascending));

    let signed_descending: Vec<RisePositionKey> = signed
        .iter()
        .rev()
        .map(|value| {
            RisePositionKey::make(|builder| {
                builder.append_signed_int64_with_order(*value, RisePositionKeyOrder::Descending);
            })
        })
        .collect();
    assert_eq!(signed_descending, sorted(&signed_descending));

    let strings = ["", "a", "a\0", "aa", "b", "z"];
    let string_ascending: Vec<RisePositionKey> = strings
        .iter()
        .map(|value| {
            RisePositionKey::make(|builder| {
                builder.append_string_with_order(value, RisePositionKeyOrder::Ascending);
            })
        })
        .collect();
    assert_eq!(string_ascending, sorted(&string_ascending));

    let string_descending: Vec<RisePositionKey> = strings
        .iter()
        .rev()
        .map(|value| {
            RisePositionKey::make(|builder| {
                builder.append_string_with_order(value, RisePositionKeyOrder::Descending);
            })
        })
        .collect();
    assert_eq!(string_descending, sorted(&string_descending));

    struct Message {
        timestamp: i64,
        server_id: &'static str,
    }
    let mut messages = vec![
        Message { timestamp: 2, server_id: "10" },
        Message { timestamp: 1, server_id: "20" },
        Message { timestamp: 1, server_id: "10" },
        Message { timestamp: 2, server_id: "20" },
    ];
    messages.sort_by(|lhs, rhs| {
        message_key(lhs.timestamp, lhs.server_id).cmp(&message_key(rhs.timestamp, rhs.server_id))
    });
    assert_eq!(
        messages
            .iter()
            .map(|message| format!("{}:{}", message.timestamp, message.server_id))
            .collect::<Vec<_>>(),
        ["1:20", "1:10", "2:20", "2:10"]
    );

    struct Chat {
        pin_group: i64,
        pin_position: i64,
        activity: i64,
        id: &'static str,
    }
    fn chat_key(chat: &Chat) -> RisePositionKey {
        RisePositionKey::make(|builder| {
            builder.append_signed_int64_with_order(chat.pin_group, RisePositionKeyOrder::Ascending);
            builder.append_signed_int64_with_order(chat.pin_position, RisePositionKeyOrder::Ascending);
            builder.append_signed_int64_with_order(chat.activity, RisePositionKeyOrder::Descending);
            builder.append_string_with_order(chat.id, RisePositionKeyOrder::Ascending);
        })
    }
    let mut chats = vec![
        Chat { pin_group: 1, pin_position: 0, activity: 5, id: "b" },
        Chat { pin_group: 0, pin_position: 2, activity: 8, id: "a" },
        Chat { pin_group: 0, pin_position: 1, activity: 1, id: "z" },
        Chat { pin_group: 1, pin_position: 0, activity: 5, id: "a" },
    ];
    chats.sort_by(|lhs, rhs| chat_key(lhs).cmp(&chat_key(rhs)));
    assert_eq!(
        chats
            .iter()
            .map(|chat| format!("{}:{}:{}", chat.pin_group, chat.pin_position, chat.id))
            .collect::<Vec<_>>(),
        ["0:1:z", "0:2:a", "1:0:a", "1:0:b"]
    );
}

#[tokio::test]
async fn normalized_entities_support_aliases_metadata_and_bounded_windows_of_320() -> Result<(), RiseStoreError> {
    let store = open("entity-window").await?;

    let mut mutations: Vec<RiseStoreMutation> = Vec::with_capacity(802);
    for index in 0..400_i64 {
        let local_id = format!("message-{index}");
        mutations.push(upsert(&RiseStoreEntity::new(
            local_id.as_str(),
            data(&format!("body-{index}")),
            1,
            index,
        )));
        mutations.push(membership(
            "messages",
            &local_id,
            message_key(index, &format!("{index:04}")),
            1,
        ));
    }
    let alias = RiseEntityAlias::new(RiseEntityAliasKind::server_id(), "server-200");
    mutations.push(set_alias(&alias, "message-200"));
    mutations.push(RiseStoreMutation::SetEntityViewState {
        namespace: namespace(),
        view: "messages".to_owned(),
        state: RiseEntityViewState::new(
            7,
            Some(data("before")),
            Some(data("after")),
            true,
            false,
            RiseEntityViewLoadState::Loaded,
            None,
            400,
        ),
    });
    store.write(batch(mutations)).await?;

    assert_eq!(
        aliased(&store, &alias).await?.map(|entity| entity.local_id),
        Some("message-200".to_owned())
    );
    let latest_window = store.entity_window(latest("messages", 320)).await?;
    assert_eq!(latest_window.items.len(), 320);
    assert_eq!(
        latest_window.items.first().map(|item| item.entity.local_id.as_str()),
        Some("message-80")
    );
    assert_eq!(
        latest_window.items.last().map(|item| item.entity.local_id.as_str()),
        Some("message-399")
    );
    assert_eq!(latest_window.total_count, 400);
    assert!(latest_window.has_cached_before);
    assert!(!latest_window.has_cached_after);
    assert_eq!(latest_window.state.as_ref().map(|state| state.revision), Some(7));
    assert_eq!(
        latest_window
            .state
            .as_ref()
            .and_then(|state| state.before_cursor.clone()),
        Some(data("before"))
    );

    let anchor = RiseEntityViewPosition::new(message_key(200, "0200"), "message-200");
    assert_eq!(
        store
            .entity_view_position(namespace(), "messages".to_owned(), "message-200".to_owned())
            .await?,
        Some(anchor.clone())
    );
    let before = store
        .entity_window(window(
            "messages",
            RiseEntityWindow::Before {
                anchor: anchor.clone(),
                limit: 3,
            },
        ))
        .await?;
    assert_eq!(local_ids(&before), ["message-197", "message-198", "message-199"]);
    assert!(before.has_cached_before);
    assert!(before.has_cached_after);

    let after = store
        .entity_window(window(
            "messages",
            RiseEntityWindow::After {
                anchor: anchor.clone(),
                limit: 3,
            },
        ))
        .await?;
    assert_eq!(local_ids(&after), ["message-201", "message-202", "message-203"]);

    let around = store
        .entity_window(window(
            "messages",
            RiseEntityWindow::Around {
                anchor: anchor.clone(),
                before_limit: 2,
                after_limit: 2,
            },
        ))
        .await?;
    assert_eq!(
        local_ids(&around),
        ["message-198", "message-199", "message-200", "message-201", "message-202"]
    );
    assert_eq!(around.anchor_index, Some(2));
    assert_eq!(around.resolved_anchor, Some(anchor));
    assert_eq!(around.total_count, 400);
    close(&store).await;
    Ok(())
}

#[tokio::test]
async fn stale_membership_and_view_state_cannot_overwrite_newer_realtime_state() -> Result<(), RiseStoreError> {
    let store = open("entity-stale-page").await?;

    let fresh_position = message_key(100, "100");
    store
        .write(batch(vec![
            upsert(&entity("message", "fresh", 10, 10)),
            membership("messages", "message", fresh_position.clone(), 10),
            RiseStoreMutation::SetEntityViewState {
                namespace: namespace(),
                view: "messages".to_owned(),
                state: view_state(10, true, false, 10),
            },
        ]))
        .await?;

    let stale_commit = store
        .write(batch(vec![membership("messages", "message", message_key(1, "1"), 9)]))
        .await?;
    assert!(stale_commit.changed_views.is_empty());

    let guarded_removal = store
        .write(batch(vec![RiseStoreMutation::RemoveEntityViewMembership {
            namespace: namespace(),
            view: "messages".to_owned(),
            local_id: "message".to_owned(),
            if_version_at_most: Some(9),
        }]))
        .await?;
    assert!(guarded_removal.changed_views.is_empty());

    store
        .write(batch(vec![RiseStoreMutation::RemoveEntityViewMembership {
            namespace: namespace(),
            view: "messages".to_owned(),
            local_id: "message".to_owned(),
            if_version_at_most: Some(10),
        }]))
        .await?;
    let stale_missing_insert = store
        .write(batch(vec![membership("messages", "message", message_key(1, "1"), 9)]))
        .await?;
    assert!(stale_missing_insert.changed_views.is_empty());

    store
        .write(batch(vec![membership("messages", "message", fresh_position.clone(), 10)]))
        .await?;

    let membership_conflict = store
        .write(batch(vec![membership("messages", "message", message_key(2, "2"), 10)]))
        .await;
    assert_eq!(
        membership_conflict.err(),
        Some(RiseStoreError::EntityViewMembershipVersionConflict {
            namespace: namespace(),
            view: "messages".to_owned(),
            local_id: "message".to_owned(),
        })
    );

    let revision_conflict = store
        .write(batch(vec![RiseStoreMutation::SetEntityViewState {
            namespace: namespace(),
            view: "messages".to_owned(),
            state: view_state(9, false, true, 11),
        }]))
        .await;
    assert_eq!(
        revision_conflict.err(),
        Some(RiseStoreError::EntityViewRevisionConflict {
            namespace: namespace(),
            view: "messages".to_owned(),
        })
    );

    let result = store.entity_window(latest("messages", 1)).await?;
    assert_eq!(
        result.items.first().map(|item| &item.position_key),
        Some(&fresh_position)
    );
    assert_eq!(result.items.first().map(|item| item.membership_version), Some(10));
    assert_eq!(result.state.as_ref().map(|state| state.revision), Some(10));
    close(&store).await;
    Ok(())
}

#[tokio::test]
async fn decrypted_entity_read_cache_never_returns_payload_replaced_at_the_same_version() -> Result<(), RiseStoreError> {
    let store = open("entity-read-cache-replacement").await?;

    let original = entity("message", "optimistic", 7, 1);
    store.write(batch(vec![upsert(&original)])).await?;

    assert_eq!(
        stored(&store, "message").await?.map(|entity| entity.data),
        Some(original.data.clone())
    );

    let authoritative = entity("message", "authoritative", 7, 2);
    store
        .write(batch(vec![upsert_with_policy(
            &authoritative,
            RiseEntityWritePolicy::ServerAuthoritative,
        )]))
        .await?;

    assert_eq!(
        stored(&store, "message").await?.map(|entity| entity.data),
        Some(authoritative.data)
    );
    close(&store).await;
    Ok(())
}

#[tokio::test]
async fn merge_preserves_stable_identity_and_deterministically_rewires_aliases_and_views() -> Result<(), RiseStoreError> {
    let store = open("entity-merge").await?;

    let client_alias = RiseEntityAlias::new(RiseEntityAliasKind::client_id(), "client-temp");
    let server_alias = RiseEntityAlias::new(RiseEntityAliasKind::server_id(), "server-42");
    store
        .write(batch(vec![
            upsert(&entity("stable-temp", "optimistic", 1, 1)),
            set_alias(&client_alias, "stable-temp"),
            membership("messages", "stable-temp", message_key(1, "temp"), 1),
            upsert(&entity("realtime-server", "authoritative", 2, 2)),
            set_alias(&server_alias, "realtime-server"),
            membership("messages", "realtime-server", message_key(2, "42"), 2),
        ]))
        .await?;

    assert_eq!(
        stored(&store, "stable-temp").await?.map(|entity| entity.data),
        Some(data("optimistic"))
    );
    assert_eq!(
        stored(&store, "realtime-server").await?.map(|entity| entity.data),
        Some(data("authoritative"))
    );

    store
        .write(batch(vec![RiseStoreMutation::MergeEntities {
            namespace: namespace(),
            survivor_local_id: "stable-temp".to_owned(),
            duplicate_local_id: "realtime-server".to_owned(),
        }]))
        .await?;

    let survivor = stored(&store, "stable-temp")
        .await?
        .expect("the merge survivor keeps its local identity");
    assert_eq!(survivor.data, data("authoritative"));
    assert_eq!(survivor.version, 2);
    assert_eq!(stored(&store, "realtime-server").await?, None);
    assert_eq!(
        aliased(&store, &client_alias).await?.map(|entity| entity.local_id),
        Some("stable-temp".to_owned())
    );
    assert_eq!(
        aliased(&store, &server_alias).await?.map(|entity| entity.local_id),
        Some("stable-temp".to_owned())
    );
    let merged_window = store.entity_window(latest("messages", 10)).await?;
    assert_eq!(merged_window.total_count, 1);
    assert_eq!(local_ids(&merged_window), ["stable-temp"]);
    assert_eq!(
        merged_window.items.first().map(|item| item.membership_version),
        Some(2)
    );

    store.write(batch(vec![remove_entity("stable-temp")])).await?;
    assert_eq!(aliased(&store, &client_alias).await?, None);
    assert_eq!(aliased(&store, &server_alias).await?, None);
    assert_eq!(
        store.entity_window(latest("messages", 10)).await?.total_count,
        0
    );
    close(&store).await;
    Ok(())
}

#[tokio::test]
async fn atomic_rollback_retention_and_entity_observation_remain_consistent() -> Result<(), RiseStoreError> {
    let store = open("entity-atomic-retention").await?;

    let rolled_back = store
        .write(batch(vec![
            upsert(&entity("must-rollback", "rollback", 1, 1)),
            set_alias(
                &RiseEntityAlias::new(RiseEntityAliasKind::server_id(), "missing-target"),
                "does-not-exist",
            ),
        ]))
        .await;
    assert!(rolled_back.is_err());
    assert_eq!(stored(&store, "must-rollback").await?, None);

    let mut setup: Vec<RiseStoreMutation> = Vec::new();
    for index in 0..5_i64 {
        let local_id = format!("retained-{index}");
        setup.push(upsert(&RiseStoreEntity::new(
            local_id.as_str(),
            data(&format!("v{index}")),
            1,
            index,
        )));
        setup.push(set_alias(
            &RiseEntityAlias::new(
                RiseEntityAliasKind::server_id(),
                format!("retained-server-{index}"),
            ),
            &local_id,
        ));
        setup.push(membership(
            "messages",
            &local_id,
            message_key(index, &index.to_string()),
            1,
        ));
    }
    store.write(batch(setup)).await?;

    let observed_query = latest("messages", 5);
    let observation = store
        .observe(RiseStoreSnapshotRequest {
            entity_windows: [observed_query.clone()].into_iter().collect(),
            ..Default::default()
        })
        .await?;
    assert_eq!(
        observation
            .seed
            .entity_windows
            .get(&observed_query)
            .map(|result| result.total_count),
        Some(5)
    );

    let updated = store
        .write(batch(vec![upsert(&entity("retained-4", "updated", 2, 10))]))
        .await?;
    assert!(updated
        .changed_views
        .contains(&RiseStoreViewKey::entity_view(&namespace(), "messages")));
    let mut iterator = observation.updates.make_async_iterator();
    assert_eq!(
        iterator.next().await.map(|commit| commit.revision),
        Some(updated.revision)
    );

    store
        .write(batch(vec![
            RiseStoreMutation::TrimEntityView {
                namespace: namespace(),
                view: "messages".to_owned(),
                keeping: RiseEntityViewRetention::Latest(2),
                if_version_at_most: None,
            },
            RiseStoreMutation::PruneUnreferencedEntities {
                namespace: namespace(),
                limit: 5,
            },
        ]))
        .await?;
    let retained = store.entity_window(latest("messages", 5)).await?;
    assert_eq!(retained.total_count, 2);
    assert_eq!(local_ids(&retained), ["retained-3", "retained-4"]);
    assert_eq!(stored(&store, "retained-0").await?, None);
    assert_eq!(
        aliased(
            &store,
            &RiseEntityAlias::new(RiseEntityAliasKind::server_id(), "retained-server-0"),
        )
        .await?,
        None
    );
    close(&store).await;
    Ok(())
}

#[tokio::test]
async fn canonical_entity_payload_is_encrypted_and_bound_to_its_identity_aad() -> Result<(), RiseStoreError> {
    let cipher = RiseAESGCMStoreCipher::new(&[0x33; 32])?;
    let configuration = RiseStoreConfiguration::in_memory(
        "entity-encryption-account".to_owned(),
        Arc::new(cipher),
        Arc::new(RiseSystemClock),
        RiseStoreLimits::production(),
    );
    let store = RiseStore::open(configuration).await?;
    let plaintext = data("canonical-secret");
    store
        .write(batch(vec![upsert(&RiseStoreEntity::new(
            "secret",
            plaintext.clone(),
            1,
            1,
        ))]))
        .await?;

    let encrypted_namespace = namespace();
    let encrypted = store
        .enqueue(move |_store, database| {
            let mut statement = database.statement(
                "SELECT payload FROM entities WHERE namespace = ? AND local_id = ?",
                "test.entity.encrypted_payload",
            )?;
            statement.bind(encrypted_namespace.raw_value(), 1)?;
            statement.bind("secret", 2)?;
            if !statement.step()? {
                return Err(RiseStoreError::CorruptedValue("test_entity_payload".to_owned()));
            }
            statement
                .data(0)
                .ok_or_else(|| RiseStoreError::CorruptedValue("test_entity_payload".to_owned()))
        })
        .await?;
    assert_ne!(encrypted, plaintext);
    assert_eq!(
        stored(&store, "secret").await?.map(|entity| entity.data),
        Some(plaintext)
    );

    store
        .enqueue(|_store, database| {
            database.execute(
                "UPDATE entities SET version = 2 WHERE local_id = 'secret'",
                "test.entity.tamper_aad",
            )
        })
        .await?;
    assert!(stored(&store, "secret").await.is_err());
    close(&store).await;
    Ok(())
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = FileManager.remove_item(&self.path);
    }
}

fn apply_schema(
    database: &RiseSQLiteConnection,
    version: i64,
    operation: &str,
) -> Result<(), RiseStoreError> {
    for step in MIGRATIONS.iter() {
        if i64::from(step.to_version) != version {
            continue;
        }
        for statement in step.statements {
            database.execute(statement, operation)?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn schema_v3_migrates_additively_without_losing_legacy_rows() -> Result<(), RiseStoreError> {
    let directory = TemporaryDirectory {
        path: std::env::temp_dir().join(format!(
            "rise-engine-v3-{}",
            Uuid::new_v4().uuid_string()
        )),
    };
    FileManager
        .create_directory(&directory.path, true)
        .expect("temporary directory is writable");
    let database_url = directory.path.join("engine.sqlite");

    let mut legacy = RiseSQLiteConnection::new(&database_url.to_string_lossy())?;
    apply_schema(&legacy, 1, "test.v3.schema.v1")?;
    apply_schema(&legacy, 2, "test.v3.schema.v2")?;
    legacy.execute(
        "INSERT INTO metadata(key, value) VALUES ('store_id', 'v3-store')",
        "test.v3.store_id",
    )?;
    let mut verifier = legacy.statement(
        "INSERT INTO crypto_metadata(key, value) VALUES ('key_verifier', ?)",
        "test.v3.verifier",
    )?;
    verifier.bind(RiseStore::key_verifier_plaintext(), 1)?;
    verifier.step()?;
    verifier.release();
    drop(verifier);
    apply_schema(&legacy, 3, "test.v3.schema.v3")?;
    let mut legacy_value = legacy.statement(
        "INSERT INTO key_values(namespace, key, value, version, updated_at_ms) \
         VALUES (?, 'legacy', ?, 1, 1)",
        "test.v3.legacy_value",
    )?;
    legacy_value.bind(namespace().raw_value(), 1)?;
    legacy_value.bind(data("survives"), 2)?;
    legacy_value.step()?;
    legacy_value.release();
    drop(legacy_value);
    legacy.close()?;

    let configuration = RiseStoreConfiguration::new(
        database_url,
        "v3-account".to_owned(),
        Arc::new(RisePlaintextStoreCipher),
        Arc::new(RiseSystemClock),
        RiseStoreLimits::production(),
        false,
    );
    let migrated = RiseStore::open(configuration).await?;
    assert_eq!(
        migrated
            .value(namespace(), "legacy".to_owned())
            .await?
            .map(|value| value.data),
        Some(data("survives"))
    );
    let schema = migrated
        .enqueue(|_store, database| {
            let mut version = database.statement("PRAGMA user_version", "test.v3.migrated_version")?;
            if !version.step()? {
                return Err(RiseStoreError::CorruptedValue("test_v3_version".to_owned()));
            }
            let value = version.int64(0);
            version.release();
            let mut table = database.statement(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'entities'",
                "test.v3.entities_table",
            )?;
            let has_entities = table.step()?;
            Ok((value, has_entities))
        })
        .await?;
    assert_eq!(schema.0, 4);
    assert!(schema.1);
    migrated.close().await?;
    Ok(())
}
