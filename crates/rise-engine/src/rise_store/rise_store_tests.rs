use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rise_canon::rise_crypto::AAD_PREFIX;
use rise_canon::rise_store::{CipherContextKind, MIGRATIONS, StatementId};
use rise_core::foundation::foundation_uuid::Uuid;
use tokio::sync::Mutex;

use crate::rise_core::rise_identifiers::RiseRequestID;
use crate::rise_store::rise_sq_lite::RiseSQLiteConnection;
use crate::rise_store::rise_store::{
    RiseEngineClock, RiseStore, RiseStoreConfiguration, RiseSystemClock,
};
use crate::rise_store::rise_store_cipher::{
    RiseAESGCMStoreCipher, RisePlaintextStoreCipher, RiseStoreValueCipher,
};
use crate::rise_store::rise_store_models::{
    RiseCollectionAnchor, RiseCollectionState, RiseDurableOperation, RiseDurableOperationState,
    RiseDurableOrderingKey, RiseDurableTerminalResolution, RiseOrderedItem,
    RiseOrderedItemPosition, RiseOrderedItemWritePolicy, RiseStoreCollectionKey,
    RiseStoreCollectionQuery, RiseStoreError, RiseStoreLimits, RiseStoreMutation,
    RiseStoreNamespace, RiseStoreSnapshotRequest, RiseStoreValueKey, RiseStoreVersionPrecondition,
    RiseStoreViewKey, RiseStoreWriteBatch, RiseStoredValue, RiseStoredValueWritePolicy,
};

static SERIALIZED: Mutex<()> = Mutex::const_new(());

struct FixedRiseClock {
    value: i64,
}

impl RiseEngineClock for FixedRiseClock {
    fn now_milliseconds(&self) -> i64 {
        self.value
    }
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(prefix: &str, creates: bool) -> Self {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4().uuid_string()));
        if creates {
            std::fs::create_dir_all(&path).expect("temporary directory");
        }
        Self { path }
    }

    fn database_url(&self) -> PathBuf {
        self.path.join("engine.sqlite")
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn namespace() -> RiseStoreNamespace {
    RiseStoreNamespace::new("tests")
}

fn request_id(raw_value: u64) -> RiseRequestID {
    RiseRequestID::new(raw_value).expect("test request id is above the canon minimum")
}

fn batch(mutations: Vec<RiseStoreMutation>) -> RiseStoreWriteBatch {
    RiseStoreWriteBatch::new(mutations, HashSet::new())
}

fn stored_value(text: &str, version: i64, updated_at_milliseconds: i64) -> RiseStoredValue {
    RiseStoredValue::new(text.as_bytes().to_vec(), version, updated_at_milliseconds)
}

fn ordered_item(
    item_id: &str,
    sort_key: i64,
    text: &str,
    version: i64,
    updated_at_milliseconds: i64,
) -> RiseOrderedItem {
    RiseOrderedItem::new(
        item_id,
        None,
        sort_key,
        text.as_bytes().to_vec(),
        version,
        updated_at_milliseconds,
    )
}

fn set_value_with_policy(
    key: &str,
    value: RiseStoredValue,
    policy: RiseStoredValueWritePolicy,
) -> RiseStoreMutation {
    RiseStoreMutation::SetValue {
        namespace: namespace(),
        key: key.to_owned(),
        value,
        precondition: RiseStoreVersionPrecondition::Any,
        policy,
    }
}

fn set_value(key: &str, value: RiseStoredValue) -> RiseStoreMutation {
    set_value_with_policy(key, value, RiseStoredValueWritePolicy::RequireIdenticalReplay)
}

fn upsert_ordered_item(collection: &str, item: RiseOrderedItem) -> RiseStoreMutation {
    RiseStoreMutation::UpsertOrderedItem {
        namespace: namespace(),
        collection: collection.to_owned(),
        item,
        policy: RiseOrderedItemWritePolicy::RequireIdenticalReplay,
    }
}

fn durable_operation(
    raw_value: u64,
    payload: Vec<u8>,
    ordering_key: Option<&str>,
    created_at_milliseconds: i64,
) -> RiseDurableOperation {
    let operation = RiseDurableOperation::new(
        request_id(raw_value),
        "send",
        payload,
        0,
        created_at_milliseconds,
        created_at_milliseconds,
    );
    match ordering_key {
        Some(lane) => RiseDurableOperation {
            ordering_key: RiseDurableOrderingKey::new(lane),
            ..operation
        }
        .validated(),
        None => operation,
    }
}

async fn open_in_memory(
    account_id: &str,
    clock: Arc<dyn RiseEngineClock>,
    limits: RiseStoreLimits,
) -> Result<Arc<RiseStore>, RiseStoreError> {
    let configuration = RiseStoreConfiguration::in_memory(
        account_id.to_owned(),
        Arc::new(RisePlaintextStoreCipher),
        clock,
        limits,
    );
    RiseStore::open(configuration).await
}

async fn open_default_in_memory(account_id: &str) -> Result<Arc<RiseStore>, RiseStoreError> {
    open_in_memory(account_id, Arc::new(RiseSystemClock), RiseStoreLimits::production()).await
}

fn file_configuration(
    file_url: &Path,
    account_id: &str,
    cipher: Arc<dyn RiseStoreValueCipher>,
) -> RiseStoreConfiguration {
    RiseStoreConfiguration::new(
        file_url.to_path_buf(),
        account_id.to_owned(),
        cipher,
        Arc::new(RiseSystemClock),
        RiseStoreLimits::production(),
        false,
    )
}

fn database_path(file_url: &Path) -> Cow<'_, str> {
    file_url.to_string_lossy()
}

fn execute_legacy_schema_v2(
    legacy: &RiseSQLiteConnection,
    operation: &str,
) -> Result<(), RiseStoreError> {
    for step in MIGRATIONS.iter().filter(|step| step.to_version <= 2) {
        for statement in step.statements {
            legacy.execute(statement, operation)?;
        }
    }
    Ok(())
}

fn write_store_id(
    legacy: &RiseSQLiteConnection,
    store_id: &str,
    operation: &str,
) -> Result<(), RiseStoreError> {
    let mut statement = legacy.statement(StatementId::WriteStoreId.sql(), operation)?;
    statement.bind(store_id, 1)?;
    statement.step()?;
    statement.release();
    Ok(())
}

fn write_key_verifier(
    legacy: &RiseSQLiteConnection,
    sealed: &[u8],
    operation: &str,
) -> Result<(), RiseStoreError> {
    let mut statement = legacy.statement(StatementId::WriteKeyVerifier.sql(), operation)?;
    statement.bind(sealed, 1)?;
    statement.step()?;
    statement.release();
    Ok(())
}

async fn schema_version(
    store: &Arc<RiseStore>,
    operation: &'static str,
    corrupted: &'static str,
) -> Result<i64, RiseStoreError> {
    store
        .enqueue(move |_, database| {
            let mut statement = database.statement("PRAGMA user_version", operation)?;
            if !statement.step()? {
                return Err(RiseStoreError::CorruptedValue(corrupted.to_owned()));
            }
            Ok(statement.int64(0))
        })
        .await
}

#[tokio::test]
async fn atomic_batch_publishes_only_after_commit_and_rolls_back_on_conflict()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let store = open_in_memory(
        "account-a",
        Arc::new(FixedRiseClock { value: 100 }),
        RiseStoreLimits::production(),
    )
    .await?;

    let value_key = RiseStoreValueKey::new(namespace(), "profile");
    let collection_key = RiseStoreCollectionKey::new(namespace(), "timeline");
    let stream = store.updates(&HashSet::from([
        RiseStoreViewKey::value(&namespace(), "profile"),
        RiseStoreViewKey::collection(&namespace(), "timeline"),
    ]))?;
    let mut iterator = stream.make_async_iterator();

    let item = ordered_item("item-a", 10, "first", 1, 100);
    let commit = store
        .write(batch(vec![
            set_value("profile", stored_value("value-1", 1, 100)),
            upsert_ordered_item("timeline", item),
            RiseStoreMutation::SetCollectionState {
                namespace: namespace(),
                collection: "timeline".to_owned(),
                state: RiseCollectionState::new(1, None, None, true, false),
            },
        ]))
        .await?;

    assert_eq!(commit.revision, 1);
    assert_eq!(commit.committed_at_milliseconds, 100);
    assert_eq!(iterator.next().await, Some(commit.clone()));

    let snapshot = store
        .snapshot(RiseStoreSnapshotRequest {
            values: HashSet::from([value_key.clone()]),
            collections: HashSet::from([RiseStoreCollectionQuery::new(
                namespace(),
                "timeline",
                10,
                false,
                None,
            )]),
            collection_states: HashSet::from([collection_key.clone()]),
            ..RiseStoreSnapshotRequest::default()
        })
        .await?;
    assert_eq!(snapshot.revision, 1);
    assert_eq!(
        snapshot.values.get(&value_key).map(|value| value.data.as_slice()),
        Some(b"value-1".as_slice())
    );
    assert_eq!(
        snapshot.collection_states.get(&collection_key).map(|state| state.revision),
        Some(1)
    );

    let conflict = store
        .write(batch(vec![
            set_value("profile", stored_value("must-rollback", 2, 101)),
            upsert_ordered_item("timeline", ordered_item("item-a", 11, "stale", 0, 101)),
        ]))
        .await;
    assert!(conflict.is_err());

    let after_rollback = store
        .snapshot(RiseStoreSnapshotRequest {
            values: HashSet::from([value_key.clone()]),
            ..RiseStoreSnapshotRequest::default()
        })
        .await?;
    assert_eq!(after_rollback.revision, 1);
    assert_eq!(
        after_rollback.values.get(&value_key).map(|value| value.data.as_slice()),
        Some(b"value-1".as_slice())
    );
    store.close().await
}

#[tokio::test]
async fn atomic_observation_seeds_then_coalesces_ordered_invalidations_without_no_op_revisions()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let store = open_default_in_memory("account-observe").await?;

    let first_key = RiseStoreValueKey::new(namespace(), "first");
    let second_key = RiseStoreValueKey::new(namespace(), "second");
    let observation = store
        .observe(RiseStoreSnapshotRequest {
            values: HashSet::from([first_key.clone(), second_key.clone()]),
            ..RiseStoreSnapshotRequest::default()
        })
        .await?;
    assert_eq!(observation.seed.revision, 0);
    assert!(observation.seed.values.is_empty());

    let first_commit = store
        .write(batch(vec![set_value("first", stored_value("one", 1, 1))]))
        .await?;
    let second_commit = store
        .write(batch(vec![set_value("second", stored_value("two", 1, 2))]))
        .await?;
    let replay_commit = store
        .write(batch(vec![set_value("second", stored_value("two", 1, 2))]))
        .await?;

    assert_eq!(first_commit.revision, 1);
    assert_eq!(second_commit.revision, 2);
    assert_eq!(replay_commit.revision, 2);
    assert!(replay_commit.changed_views.is_empty());

    let mut iterator = observation.updates.make_async_iterator();
    let coalesced = iterator.next().await.expect("coalesced commit");
    assert_eq!(coalesced.revision, 2);
    assert_eq!(
        coalesced.changed_views,
        HashSet::from([
            RiseStoreViewKey::value(&namespace(), "first"),
            RiseStoreViewKey::value(&namespace(), "second"),
        ])
    );
    assert!(!coalesced.requires_full_refresh);

    let refreshed = store
        .snapshot(RiseStoreSnapshotRequest {
            values: HashSet::from([first_key.clone(), second_key.clone()]),
            ..RiseStoreSnapshotRequest::default()
        })
        .await?;
    assert_eq!(refreshed.revision, 2);
    assert_eq!(
        refreshed.values.get(&first_key).map(|value| value.data.as_slice()),
        Some(b"one".as_slice())
    );
    assert_eq!(
        refreshed.values.get(&second_key).map(|value| value.data.as_slice()),
        Some(b"two".as_slice())
    );
    store.close().await
}

#[tokio::test]
async fn stalled_observer_payload_is_bounded_and_overflow_requires_gap_free_refresh()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let limits = RiseStoreLimits {
        maximum_batch_views: 2,
        ..RiseStoreLimits::PRODUCTION
    }
    .validated();
    let store = open_in_memory("account-observer-overflow", Arc::new(RiseSystemClock), limits)
        .await?;
    let updates = store.updates(&HashSet::new())?;
    let mut iterator = updates.make_async_iterator();

    for index in 0..4_i64 {
        store
            .write(batch(vec![set_value(
                &format!("key-{index}"),
                stored_value(&format!("value-{index}"), 1, index),
            )]))
            .await?;
    }

    let overflow = iterator.next().await.expect("overflow commit");
    assert_eq!(overflow.revision, 4);
    assert!(overflow.changed_views.is_empty());
    assert!(overflow.requires_full_refresh);

    let refreshed = store
        .snapshot(RiseStoreSnapshotRequest {
            values: HashSet::from([RiseStoreValueKey::new(namespace(), "key-3")]),
            ..RiseStoreSnapshotRequest::default()
        })
        .await?;
    assert_eq!(refreshed.revision, overflow.revision);

    store
        .write(batch(vec![set_value("key-3", stored_value("latest", 2, 5))]))
        .await?;
    let after_refresh = iterator.next().await.expect("commit after refresh");
    assert_eq!(after_refresh.revision, 5);
    assert!(!after_refresh.requires_full_refresh);
    assert_eq!(
        after_refresh.changed_views,
        HashSet::from([RiseStoreViewKey::value(&namespace(), "key-3")])
    );
    store.close().await
}

#[tokio::test]
async fn stored_values_reject_or_ignore_stale_versions_by_policy() -> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let store = open_default_in_memory("account-value-version").await?;

    store
        .write(batch(vec![set_value("versioned", stored_value("v2", 2, 2))]))
        .await?;

    let stale = store
        .write(batch(vec![set_value("versioned", stored_value("stale", 1, 3))]))
        .await;
    assert!(stale.is_err());

    let ignored = store
        .write(batch(vec![set_value_with_policy(
            "versioned",
            stored_value("stale", 1, 3),
            RiseStoredValueWritePolicy::IgnoreIfNotNewer,
        )]))
        .await?;
    assert_eq!(ignored.revision, 1);
    assert!(ignored.changed_views.is_empty());
    assert_eq!(
        store
            .value(namespace(), "versioned".to_owned())
            .await?
            .map(|value| value.data),
        Some(b"v2".to_vec())
    );

    store
        .write(batch(vec![set_value_with_policy(
            "versioned",
            stored_value("authoritative", 2, 4),
            RiseStoredValueWritePolicy::ServerAuthoritative,
        )]))
        .await?;
    assert_eq!(
        store
            .value(namespace(), "versioned".to_owned())
            .await?
            .map(|value| value.data),
        Some(b"authoritative".to_vec())
    );
    store.close().await
}

#[tokio::test]
async fn ordered_pagination_uses_stable_sort_key_and_item_id_tuple() -> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let store = open_default_in_memory("account-order").await?;

    let items = ["b", "a", "c"]
        .into_iter()
        .map(|identifier| {
            upsert_ordered_item("same-sort-key", ordered_item(identifier, 42, identifier, 1, 1))
        })
        .collect();
    store.write(batch(items)).await?;

    fn item_ids(items: &[RiseOrderedItem]) -> Vec<&str> {
        items.iter().map(|item| item.item_id.as_str()).collect()
    }

    let descending = store
        .ordered_items(namespace(), "same-sort-key".to_owned(), 10, false, None)
        .await?;
    assert_eq!(item_ids(&descending), ["c", "b", "a"]);

    let before_b = store
        .ordered_items(
            namespace(),
            "same-sort-key".to_owned(),
            10,
            false,
            Some(RiseCollectionAnchor::Before(RiseOrderedItemPosition::new(42, "b"))),
        )
        .await?;
    assert_eq!(item_ids(&before_b), ["a"]);

    let after_b = store
        .ordered_items(
            namespace(),
            "same-sort-key".to_owned(),
            10,
            true,
            Some(RiseCollectionAnchor::After(RiseOrderedItemPosition::new(42, "b"))),
        )
        .await?;
    assert_eq!(item_ids(&after_b), ["c"]);
    store.close().await
}

#[tokio::test]
async fn durable_operations_preserve_ordering_lanes_across_restart_recovery()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let store = open_default_in_memory("account-outbox").await?;

    let first_id = request_id(101);
    let second_id = request_id(102);
    let independent_id = request_id(103);
    fn operation(id: RiseRequestID, lane: &str, created: i64) -> RiseDurableOperation {
        durable_operation(
            id.raw_value(),
            id.raw_value().to_string().into_bytes(),
            Some(lane),
            created,
        )
    }

    store
        .write(batch(vec![
            RiseStoreMutation::PutDurableOperation(operation(first_id, "chat-a", 1)),
            RiseStoreMutation::PutDurableOperation(operation(second_id, "chat-a", 2)),
            RiseStoreMutation::PutDurableOperation(operation(independent_id, "chat-b", 3)),
        ]))
        .await?;

    let first_replay = store.replayable_operations(10, 10).await?;
    assert_eq!(
        first_replay.iter().map(|operation| operation.request_id).collect::<Vec<_>>(),
        [first_id, independent_id]
    );

    let claims = store.claim_replayable_operations(10, 1).await?;
    assert_eq!(claims.first().map(|claim| claim.operation.request_id), Some(first_id));
    let while_first_is_in_flight = store.replayable_operations(10, 10).await?;
    assert_eq!(
        while_first_is_in_flight
            .iter()
            .map(|operation| operation.request_id)
            .collect::<Vec<_>>(),
        [independent_id]
    );

    store.recover_in_flight_operations(Some(11)).await?;
    assert_eq!(
        store.operation(first_id).await?.map(|operation| operation.state),
        Some(RiseDurableOperationState::Queued)
    );
    store.close().await
}

#[tokio::test]
async fn durable_claims_are_token_bound_and_terminal_rows_do_not_brick_ordering_lane()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let store = open_default_in_memory("account-claims").await?;

    fn operation(raw_id: u64) -> RiseDurableOperation {
        let created = i64::try_from(raw_id).expect("test request id fits Int64");
        durable_operation(
            raw_id,
            format!("payload-{raw_id}").into_bytes(),
            Some("chat-a"),
            created,
        )
    }
    let first = operation(201);
    let second = operation(202);
    store
        .write(batch(vec![
            RiseStoreMutation::PutDurableOperation(first.clone()),
            RiseStoreMutation::PutDurableOperation(second.clone()),
        ]))
        .await?;

    let first_claims = store.claim_replayable_operations(1, 10).await?;
    assert_eq!(first_claims.len(), 1);
    assert_eq!(
        first_claims.first().map(|claim| claim.operation.request_id),
        Some(first.request_id)
    );
    assert_eq!(first_claims.first().map(|claim| claim.operation.attempt_count), Some(0));
    let first_token = first_claims.first().expect("first claim").token.clone();

    let wrong_token = store
        .write(batch(vec![RiseStoreMutation::TransitionClaimedDurableOperation {
            request_id: first.request_id,
            claim_token: "wrong-token".to_owned(),
            state: RiseDurableOperationState::TerminalFailure,
            attempt_count: 1,
            next_attempt_at_milliseconds: 1,
            last_error_code: Some("terminal".to_owned()),
            updated_at_milliseconds: 1,
        }]))
        .await;
    assert!(wrong_token.is_err());
    store
        .write(batch(vec![RiseStoreMutation::TransitionClaimedDurableOperation {
            request_id: first.request_id,
            claim_token: first_token,
            state: RiseDurableOperationState::TerminalFailure,
            attempt_count: 1,
            next_attempt_at_milliseconds: 1,
            last_error_code: Some("terminal".to_owned()),
            updated_at_milliseconds: 1,
        }]))
        .await?;
    let terminal = store.terminal_operations_limit(10).await?;
    assert_eq!(
        terminal.iter().map(|operation| operation.request_id).collect::<Vec<_>>(),
        [first.request_id]
    );
    assert_eq!(
        terminal.first().and_then(|operation| operation.last_error_code.as_deref()),
        Some("terminal")
    );

    let second_claims = store.claim_replayable_operations(1, 10).await?;
    assert_eq!(
        second_claims
            .iter()
            .map(|claim| claim.operation.request_id)
            .collect::<Vec<_>>(),
        [second.request_id]
    );
    let second_token = second_claims.first().expect("second claim").token.clone();
    store
        .write(batch(vec![
            RiseStoreMutation::RemoveClaimedDurableOperation {
                request_id: second.request_id,
                claim_token: second_token,
            },
            RiseStoreMutation::ResolveTerminalOperation {
                request_id: first.request_id,
                resolution: RiseDurableTerminalResolution::Requeue {
                    next_attempt_at_milliseconds: 2,
                    reset_attempt_count: true,
                },
                updated_at_milliseconds: 2,
            },
        ]))
        .await?;

    let reclaimed = store.claim_replayable_operations(2, 10).await?;
    assert_eq!(
        reclaimed.first().map(|claim| claim.operation.request_id),
        Some(first.request_id)
    );
    assert_eq!(reclaimed.first().map(|claim| claim.operation.attempt_count), Some(0));
    store.close().await
}

#[tokio::test]
async fn stale_durable_claim_cannot_overwrite_a_recovered_and_reclaimed_operation()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let store = open_default_in_memory("account-stale-claim").await?;
    let request_id = request_id(301);
    let operation = durable_operation(301, b"payload".to_vec(), Some("chat-a"), 1);
    store
        .write(batch(vec![RiseStoreMutation::PutDurableOperation(operation)]))
        .await?;

    let first_claim = store
        .claim_replayable_operations(1, 1)
        .await?
        .into_iter()
        .next()
        .expect("first claim");
    store.recover_in_flight_operations(Some(2)).await?;
    let second_claim = store
        .claim_replayable_operations(2, 1)
        .await?
        .into_iter()
        .next()
        .expect("second claim");
    assert_ne!(first_claim.token, second_claim.token);

    let stale_transition = store
        .write(batch(vec![RiseStoreMutation::TransitionClaimedDurableOperation {
            request_id,
            claim_token: first_claim.token.clone(),
            state: RiseDurableOperationState::RetryWaiting,
            attempt_count: 1,
            next_attempt_at_milliseconds: 3,
            last_error_code: Some("stale".to_owned()),
            updated_at_milliseconds: 3,
        }]))
        .await;
    assert!(stale_transition.is_err());
    let stale_removal = store
        .write(batch(vec![RiseStoreMutation::RemoveClaimedDurableOperation {
            request_id,
            claim_token: first_claim.token.clone(),
        }]))
        .await;
    assert!(stale_removal.is_err());
    assert_eq!(
        store.operation(request_id).await?.map(|operation| operation.state),
        Some(RiseDurableOperationState::InFlight)
    );

    store
        .write(batch(vec![RiseStoreMutation::RemoveClaimedDurableOperation {
            request_id,
            claim_token: second_claim.token,
        }]))
        .await?;
    assert_eq!(store.operation(request_id).await?, None);
    store.close().await
}

#[tokio::test]
async fn unclaimed_durable_cancellation_is_atomic_and_refuses_owned_work()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let store = open_default_in_memory("account-cancel-durable").await?;
    fn operation(raw: u64) -> RiseDurableOperation {
        durable_operation(raw, format!("payload-{raw}").into_bytes(), None, 1)
    }

    let queued = operation(311);
    store
        .write(batch(vec![RiseStoreMutation::PutDurableOperation(queued.clone())]))
        .await?;
    store
        .write(batch(vec![RiseStoreMutation::RemoveUnclaimedDurableOperation {
            request_id: queued.request_id,
        }]))
        .await?;
    assert_eq!(store.operation(queued.request_id).await?, None);

    let claimed = operation(312);
    store
        .write(batch(vec![RiseStoreMutation::PutDurableOperation(claimed.clone())]))
        .await?;
    store
        .claim_replayable_operations(1, 1)
        .await?
        .into_iter()
        .next()
        .expect("claim");
    let owned_removal = store
        .write(batch(vec![RiseStoreMutation::RemoveUnclaimedDurableOperation {
            request_id: claimed.request_id,
        }]))
        .await;
    assert!(owned_removal.is_err());
    assert_eq!(
        store.operation(claimed.request_id).await?.map(|operation| operation.state),
        Some(RiseDurableOperationState::InFlight)
    );
    store.close().await
}

#[tokio::test]
async fn durable_insert_rejects_preclaimed_or_partially_processed_state()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let store = open_default_in_memory("account-invalid-durable-insert").await?;
    let request_id = request_id(302);
    let operation = RiseDurableOperation {
        state: RiseDurableOperationState::InFlight,
        attempt_count: 1,
        ..durable_operation(302, b"payload".to_vec(), None, 1)
    }
    .validated();

    let insert = store
        .write(batch(vec![RiseStoreMutation::PutDurableOperation(operation)]))
        .await;
    assert!(insert.is_err());
    assert_eq!(store.operation(request_id).await?, None);
    store.close().await
}

const LEGACY_DURABLE_ROWS: &str = "\
INSERT INTO durable_operations(
    request_id, enqueue_sequence, ordering_key, kind, payload, state,
    attempt_count, next_attempt_ms, last_error_code, created_at_ms,
    updated_at_ms, claim_token
) VALUES (
    '401', 1, 'chat-a', 'send', X'7061796c6f6164', 'in_flight',
    1, 0, NULL, 1, 1, 'legacy-claim'
);
INSERT INTO durable_operations(
    request_id, enqueue_sequence, ordering_key, kind, payload, state,
    attempt_count, next_attempt_ms, last_error_code, created_at_ms,
    updated_at_ms, claim_token
) VALUES (
    '403', 2, 'chat-b', 'send', X'71', 'queued',
    0, 0, NULL, 2, 2, NULL
);
INSERT INTO durable_operations(
    request_id, enqueue_sequence, ordering_key, kind, payload, state,
    attempt_count, next_attempt_ms, last_error_code, created_at_ms,
    updated_at_ms, claim_token
) VALUES (
    '404', 3, 'chat-c', 'send', X'7478', 'terminal_failure',
    8, 0, 'terminal', 3, 3, NULL
);
UPDATE durable_operation_usage
SET row_count = 999, payload_bytes = 999
WHERE singleton = 1;
";

const INVALID_CLAIM_INVARIANT_ROW: &str = "\
INSERT INTO durable_operations(
    request_id, enqueue_sequence, ordering_key, kind, payload, state,
    attempt_count, next_attempt_ms, last_error_code, created_at_ms,
    updated_at_ms, claim_token
) VALUES (
    '402', 4, 'chat-d', 'send', X'00', 'in_flight',
    0, 0, NULL, 1, 1, NULL
)";

#[tokio::test]
async fn schema_v2_migration_recovers_claims_and_installs_database_invariant()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let directory = TemporaryDirectory::new("rise-engine-v2", true);
    let database_url = directory.database_url();
    {
        let mut legacy = RiseSQLiteConnection::new(&database_path(&database_url))?;
        execute_legacy_schema_v2(&legacy, "test.schema.v2")?;
        write_store_id(&legacy, "legacy-store", "test.store_id")?;
        write_key_verifier(&legacy, RiseStore::key_verifier_plaintext(), "test.key_verifier")?;
        legacy.execute(LEGACY_DURABLE_ROWS, "test.legacy_claim")?;
        legacy.close()?;
    }

    let store = RiseStore::open(file_configuration(
        &database_url,
        "account-v2-migration",
        Arc::new(RisePlaintextStoreCipher),
    ))
    .await?;
    let request_id = request_id(401);
    assert_eq!(
        store.operation(request_id).await?.map(|operation| operation.state),
        Some(RiseDurableOperationState::Queued)
    );
    assert_eq!(
        store.operation(self::request_id(403)).await?.map(|operation| operation.state),
        Some(RiseDurableOperationState::Queued)
    );
    assert_eq!(
        store
            .terminal_operations_limit(10)
            .await?
            .iter()
            .map(|operation| operation.request_id)
            .collect::<Vec<_>>(),
        [self::request_id(404)]
    );
    let version = schema_version(&store, "test.schema.version", "test_schema_version").await?;
    assert_eq!(version, i64::from(RiseStore::supported_schema_version()));
    let usage = store
        .enqueue(|_, database| {
            let mut statement = database.statement(
                "SELECT row_count, payload_bytes FROM durable_operation_usage WHERE singleton = 1",
                "test.durable_usage",
            )?;
            if !statement.step()? {
                return Err(RiseStoreError::CorruptedValue("test_durable_usage".to_owned()));
            }
            Ok((statement.int64(0), statement.int64(1)))
        })
        .await?;
    assert_eq!(usage.0, 3);
    assert_eq!(usage.1, 10);

    let invalid_claim = store
        .enqueue(|_, database| {
            database.execute(INVALID_CLAIM_INVARIANT_ROW, "test.invalid_claim_invariant")
        })
        .await;
    assert!(invalid_claim.is_err());

    let claim = store
        .claim_replayable_operations(1, 1)
        .await?
        .into_iter()
        .next()
        .expect("recovered claim");
    assert_eq!(claim.operation.request_id, request_id);
    assert_ne!(claim.token, "legacy-claim");

    store.close().await?;
    let reopened = RiseStore::open(file_configuration(
        &database_url,
        "account-v2-migration",
        Arc::new(RisePlaintextStoreCipher),
    ))
    .await?;
    assert_eq!(
        reopened.operation(self::request_id(403)).await?.map(|operation| operation.state),
        Some(RiseDurableOperationState::Queued)
    );
    assert_eq!(
        reopened
            .terminal_operations_limit(10)
            .await?
            .iter()
            .map(|operation| operation.request_id)
            .collect::<Vec<_>>(),
        [self::request_id(404)]
    );
    reopened.close().await
}

#[tokio::test]
async fn wrong_key_cannot_mutate_schema_v2_before_successful_migration()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let directory = TemporaryDirectory::new("rise-engine-v2-key", true);
    let database_url = directory.database_url();
    let account_id = "account-v2-wrong-key";
    let correct_cipher = Arc::new(RiseAESGCMStoreCipher::new(&[3; 32])?);
    let wrong_cipher = Arc::new(RiseAESGCMStoreCipher::new(&[4; 32])?);
    let store_id = "legacy-encrypted-store";

    let cipher_context = |domain: &str, components: &[&str]| -> Vec<u8> {
        let mut result = AAD_PREFIX.as_bytes().to_vec();
        for value in [account_id, domain].iter().chain(components) {
            result.extend_from_slice(&(value.len() as u64).to_be_bytes());
            result.extend_from_slice(value.as_bytes());
        }
        result
    };

    {
        let mut legacy = RiseSQLiteConnection::new(&database_path(&database_url))?;
        execute_legacy_schema_v2(&legacy, "test.key.schema.v2")?;
        write_store_id(&legacy, store_id, "test.key.store_id")?;
        let sealed_verifier = correct_cipher.seal(
            RiseStore::key_verifier_plaintext(),
            &cipher_context(CipherContextKind::KeyVerifier.domain(), &[store_id]),
        )?;
        write_key_verifier(&legacy, &sealed_verifier, "test.key.verifier")?;
        legacy.close()?;
    }

    let wrong_key_open =
        RiseStore::open(file_configuration(&database_url, account_id, wrong_cipher)).await;
    assert!(wrong_key_open.is_err());

    {
        let mut untouched = RiseSQLiteConnection::new(&database_path(&database_url))?;
        {
            let mut version_statement =
                untouched.statement("PRAGMA user_version", "test.key.version")?;
            assert!(version_statement.step()?);
            assert_eq!(version_statement.int64(0), 2);
            version_statement.release();
        }
        untouched.close()?;
    }

    let migrated =
        RiseStore::open(file_configuration(&database_url, account_id, correct_cipher)).await?;
    let migrated_version =
        schema_version(&migrated, "test.key.migrated_version", "test_key_migrated_version")
            .await?;
    assert_eq!(migrated_version, i64::from(RiseStore::supported_schema_version()));
    migrated.close().await
}

#[tokio::test]
async fn read_budgets_reject_multiplicative_and_byte_heavy_snapshots_without_poisoning_queue()
-> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let limits = RiseStoreLimits {
        maximum_value_bytes: 8,
        maximum_batch_value_bytes: 64,
        maximum_snapshot_keys: 8,
        maximum_snapshot_collection_queries: 2,
        maximum_collection_page_items: 3,
        maximum_read_result_items: 4,
        maximum_read_result_bytes: 10,
        maximum_durable_payload_bytes: 8,
        ..RiseStoreLimits::PRODUCTION
    }
    .validated();
    let store = open_in_memory("account-bounds", Arc::new(RiseSystemClock), limits).await?;
    store
        .write(batch(vec![
            upsert_ordered_item("a", ordered_item("1", 1, "123456", 1, 1)),
            upsert_ordered_item("a", ordered_item("2", 2, "abcdef", 1, 1)),
        ]))
        .await?;

    let page_limit = store
        .snapshot(RiseStoreSnapshotRequest {
            collections: HashSet::from([RiseStoreCollectionQuery::new(
                namespace(),
                "a",
                4,
                false,
                None,
            )]),
            ..RiseStoreSnapshotRequest::default()
        })
        .await;
    assert_eq!(page_limit.err(), Some(RiseStoreError::CollectionPageLimitReached(3)));

    let item_limit = store
        .snapshot(RiseStoreSnapshotRequest {
            collections: HashSet::from([
                RiseStoreCollectionQuery::new(namespace(), "a", 3, false, None),
                RiseStoreCollectionQuery::new(namespace(), "b", 3, false, None),
            ]),
            ..RiseStoreSnapshotRequest::default()
        })
        .await;
    assert_eq!(item_limit.err(), Some(RiseStoreError::ReadResultItemLimitReached(4)));

    let byte_limit = store
        .ordered_items(namespace(), "a".to_owned(), 2, false, None)
        .await;
    assert_eq!(byte_limit.err(), Some(RiseStoreError::ReadResultByteLimitReached(10)));

    assert_eq!(
        store
            .ordered_items(namespace(), "a".to_owned(), 1, false, None)
            .await?
            .len(),
        1
    );
    store.close().await
}

#[tokio::test]
async fn aes_gcm_data_survives_reopen_and_is_bound_to_account() -> Result<(), RiseStoreError> {
    let _serialized = SERIALIZED.lock().await;
    let directory = TemporaryDirectory::new("rise-engine-tests", false);
    let database_url = directory.database_url();
    let cipher = Arc::new(RiseAESGCMStoreCipher::new(&[7; 32])?);

    let first = RiseStore::open(file_configuration(
        &database_url,
        "account-encrypted",
        cipher.clone(),
    ))
    .await?;
    first
        .write(batch(vec![set_value("secret", stored_value("encrypted-body", 1, 1))]))
        .await?;
    first.close().await?;

    let reopened = RiseStore::open(file_configuration(
        &database_url,
        "account-encrypted",
        cipher.clone(),
    ))
    .await?;
    assert_eq!(
        reopened
            .value(namespace(), "secret".to_owned())
            .await?
            .map(|value| value.data),
        Some(b"encrypted-body".to_vec())
    );
    reopened.close().await?;

    let another_account = RiseStore::open(file_configuration(
        &database_url,
        "another-account",
        cipher.clone(),
    ))
    .await;
    assert_eq!(another_account.err(), Some(RiseStoreError::InvalidEncryptionKey));
    let another_key = RiseStore::open(file_configuration(
        &database_url,
        "account-encrypted",
        Arc::new(RiseAESGCMStoreCipher::new(&[8; 32])?),
    ))
    .await;
    assert_eq!(another_key.err(), Some(RiseStoreError::InvalidEncryptionKey));
    Ok(())
}
