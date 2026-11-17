use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use parking_lot::Mutex;
use rise_canon::rise_crypto::{
    AAD_PREFIX, COMPONENT_ENCODING, FORMAT_VERSION, LEADING_COMPONENT_NAMES, LENGTH_BYTE_ORDER,
    LENGTH_PREFIX_BYTES,
};
use rise_canon::rise_key::{otp, payload};
use rise_canon::rise_store::{
    CipherContextKind, IdentifierKind, MIGRATIONS, PRAGMA_STATEMENTS, SCHEMA_VERSION, StatementId,
};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::FileManager;
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_core::rise_identifiers::RiseRequestID;
use crate::rise_store::rise_sq_lite::{RiseSQLiteConnection, RiseSQLiteStatement};
use crate::rise_store::rise_store_cipher::{RisePlaintextStoreCipher, RiseStoreValueCipher};
use crate::rise_store::rise_store_concurrency::{
    RiseStoreAdmissionGate, RiseStoreChangeHub, RiseStoreCloseCoordinator, RiseStoreReadBudget,
    RiseStoreUpdateSequence,
};
use crate::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityViewPosition, RiseEntityViewState,
    RiseEntityWindow, RiseEntityWindowQuery, RiseEntityWindowResult, RisePositionKey,
    RiseStoreEntity,
};
use crate::rise_store::rise_store_entity_read_cache::{
    RiseEntityReadCacheEntry, RiseEntityReadCacheKey,
};
use crate::rise_store::rise_store_models::{
    RiseCollectionAnchor, RiseCollectionState, RiseDurableClaim, RiseDurableOperation,
    RiseDurableOperationState, RiseOrderedItem, RiseStoreCommit, RiseStoreError,
    RiseStoreLimits, RiseStoreMutation, RiseStoreNamespace, RiseStoreObservation,
    RiseStoreSnapshot, RiseStoreSnapshotRequest, RiseStoreViewKey, RiseStoreWriteBatch,
    RiseStoredValue,
};

pub(crate) trait RiseEngineClock: Send + Sync {
    fn now_milliseconds(&self) -> i64;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RiseSystemClock;

impl RiseEngineClock for RiseSystemClock {
    fn now_milliseconds(&self) -> i64 {
        (Date::now().time_interval_since_1970() * 1_000.0).floor() as i64
    }
}

pub(crate) struct RiseStoreConfiguration {
    pub(crate) file_url: PathBuf,
    pub(crate) account_id: String,
    pub(crate) cipher: Arc<dyn RiseStoreValueCipher>,
    pub(crate) clock: Arc<dyn RiseEngineClock>,
    pub(crate) limits: RiseStoreLimits,
    pub(crate) applies_file_protection: bool,
}

impl RiseStoreConfiguration {
    pub(crate) fn new(
        file_url: PathBuf,
        account_id: String,
        cipher: Arc<dyn RiseStoreValueCipher>,
        clock: Arc<dyn RiseEngineClock>,
        limits: RiseStoreLimits,
        applies_file_protection: bool,
    ) -> Self {
        debug_assert!(!account_id.is_empty() && account_id.len() <= 4_096);
        Self {
            file_url,
            account_id,
            cipher,
            clock,
            limits,
            applies_file_protection,
        }
    }

    pub(crate) fn in_memory(
        account_id: String,
        cipher: Arc<dyn RiseStoreValueCipher>,
        clock: Arc<dyn RiseEngineClock>,
        limits: RiseStoreLimits,
    ) -> Self {
        Self::new(
            PathBuf::from(MEMORY_PATH),
            account_id,
            cipher,
            clock,
            limits,
            false,
        )
    }

    /// in memory engine with defaults, plaintext cipher and system clock and production limits
    pub(crate) fn in_memory_default(account_id: String) -> Self {
        Self::in_memory(
            account_id,
            Arc::new(RisePlaintextStoreCipher),
            Arc::new(RiseSystemClock),
            RiseStoreLimits::production(),
        )
    }
}

const MEMORY_PATH: &str = ":memory:";

fn count<T>(value: T) -> i64
where
    i64: TryFrom<T>,
{
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn size<T>(value: i64) -> T
where
    T: TryFrom<i64> + Default,
{
    T::try_from(value).unwrap_or_default()
}

static CIPHER_AAD_PREFIX: LazyLock<Vec<u8>> = LazyLock::new(|| {
    assert!(
        LENGTH_PREFIX_BYTES == std::mem::size_of::<u64>(),
        "canon length prefix is not UInt64"
    );
    assert!(
        LENGTH_BYTE_ORDER == "big_endian",
        "canon length prefix is not big-endian"
    );
    assert!(
        COMPONENT_ENCODING == "utf8",
        "canon component encoding is not UTF-8"
    );
    assert!(
        AAD_PREFIX.ends_with(&format!("v{FORMAT_VERSION}")),
        "canon AAD prefix does not carry the canon format version"
    );
    assert!(
        LEADING_COMPONENT_NAMES == [otp::ACCOUNT_ID, "domain"],
        "canon changed the leading AAD components"
    );
    AAD_PREFIX.as_bytes().to_vec()
});

pub(crate) struct RiseStore {
    pub(crate) configuration: RiseStoreConfiguration,
    pub(crate) admission_gate: RiseStoreAdmissionGate,
    pub(crate) close_coordinator: RiseStoreCloseCoordinator,
    pub(crate) change_hub: RiseStoreChangeHub,

    pub(crate) connection: Mutex<Option<RiseSQLiteConnection>>,
    pub(crate) is_closed: AtomicBool,
    pub(crate) entity_read_cache: Mutex<HashMap<RiseEntityReadCacheKey, RiseEntityReadCacheEntry>>,
    pub(crate) entity_read_cache_bytes: Mutex<usize>,
    pub(crate) entity_read_cache_access: Mutex<u64>,
}

impl RiseStore {
    pub(crate) const fn supported_schema_version() -> i32 {
        SCHEMA_VERSION as i32
    }

    pub(crate) const fn key_verifier_plaintext() -> &'static [u8] {
        b"rise.engine.store.key-verifier.v1"
    }

    fn new(configuration: RiseStoreConfiguration) -> Self {
        let admission_gate =
            RiseStoreAdmissionGate::new(size(configuration.limits.maximum_pending_operations));
        let change_hub = RiseStoreChangeHub::new(
            size(configuration.limits.maximum_observers),
            size(configuration.limits.maximum_batch_views),
        );
        Self {
            configuration,
            admission_gate,
            close_coordinator: RiseStoreCloseCoordinator::new(),
            change_hub,
            connection: Mutex::new(None),
            is_closed: AtomicBool::new(false),
            entity_read_cache: Mutex::new(HashMap::new()),
            entity_read_cache_bytes: Mutex::new(0),
            entity_read_cache_access: Mutex::new(0),
        }
    }

    pub(crate) async fn open(
        configuration: RiseStoreConfiguration,
    ) -> Result<Arc<RiseStore>, RiseStoreError> {
        let store = Arc::new(RiseStore::new(configuration));
        store.open_on_queue().await?;
        Ok(store)
    }

    pub(crate) async fn close(self: &Arc<Self>) -> Result<(), RiseStoreError> {
        let weak = Arc::downgrade(self);
        self.close_coordinator
            .run(move || async move {
                let Some(store) = weak.upgrade() else {
                    return Ok(());
                };
                store.admission_gate.acquire_critical().await;
                let result = store.close_on_queue().await;
                store.admission_gate.release().await;
                result
            })
            .await
    }

    pub(crate) fn updates(
        &self,
        matching: &HashSet<RiseStoreViewKey>,
    ) -> Result<RiseStoreUpdateSequence, RiseStoreError> {
        self.change_hub.stream(matching)
    }

    pub(crate) async fn observe(
        self: &Arc<Self>,
        request: RiseStoreSnapshotRequest,
    ) -> Result<RiseStoreObservation, RiseStoreError> {
        self.validate_snapshot_request(&request)?;
        self.enqueue(move |store, database| {
            let seed = store.read_snapshot(&request, database)?;
            let updates = store.change_hub.stream(&request.required_view_keys())?;
            Ok(RiseStoreObservation::new(seed, updates))
        })
        .await
    }

    pub(crate) async fn write(
        self: &Arc<Self>,
        batch: RiseStoreWriteBatch,
    ) -> Result<RiseStoreCommit, RiseStoreError> {
        let limits = &self.configuration.limits;
        if count(batch.mutations.len()) > limits.maximum_batch_mutations {
            return Err(RiseStoreError::BatchMutationLimitReached(
                limits.maximum_batch_mutations,
            ));
        }
        self.validate_identifiers(&batch)?;
        let potential_changed_views = self.static_changed_views_for_batch(&batch);
        if count(potential_changed_views.len()) > limits.maximum_batch_views {
            return Err(RiseStoreError::BatchViewLimitReached(limits.maximum_batch_views));
        }
        self.validate_value_bounds(&batch)?;

        self.enqueue(move |store, database| {
            let now = store.configuration.clock.now_milliseconds();
            if batch.mutations.is_empty() && batch.invalidated_views.is_empty() {
                return Ok(RiseStoreCommit::new(
                    store.global_revision(database)?,
                    HashSet::new(),
                    now,
                ));
            }

            database.execute(StatementId::BeginImmediate.sql(), "transaction.begin")?;
            match store.write_transaction(&batch, now, database) {
                Ok(commit) => Ok(commit),
                Err(error) => {
                    let _ = database.execute(StatementId::Rollback.sql(), "transaction.rollback");
                    Err(error)
                }
            }
        })
        .await
    }

    fn write_transaction(
        &self,
        batch: &RiseStoreWriteBatch,
        now: i64,
        database: &RiseSQLiteConnection,
    ) -> Result<RiseStoreCommit, RiseStoreError> {
        let maximum_batch_views = self.configuration.limits.maximum_batch_views;
        let mut changed_views = batch.invalidated_views.clone();
        let mut timeline_trace: Vec<String> = Vec::new();
        for mutation in Self::coalescing_prunes(&batch.mutations) {
            let mutation: &RiseStoreMutation = &mutation;
            let mutation_views: HashSet<RiseStoreViewKey>;
            let did_change: bool;
            if let RiseStoreMutation::PruneUnreferencedEntities {
                namespace, limit, ..
            } = mutation
            {
                let mut references = HashSet::new();
                let local_ids = self.select_pruned_entity_references(
                    namespace,
                    *limit,
                    &mut references,
                    database,
                )?;
                did_change = self.apply_pruned_entities(namespace, &local_ids, database)?;
                if did_change
                    && references
                        .iter()
                        .any(|view| view.raw_value().contains("message.timeline"))
                {
                    let tags: Vec<String> =
                        local_ids.iter().map(|local_id| Self::a3a1_tag(local_id)).collect();
                    timeline_trace.push(format!("prune[{}]", tags.join(",")));
                }
                mutation_views = references;
            } else {
                mutation_views = self.changed_views(mutation, database)?;
                did_change = self.apply(mutation, database)?;
                if did_change && let Some(entry) = Self::a3a1_timeline_entry(mutation) {
                    timeline_trace.push(entry);
                }
            }
            if did_change {
                changed_views.extend(mutation_views);
                if count(changed_views.len()) > maximum_batch_views {
                    return Err(RiseStoreError::BatchViewLimitReached(maximum_batch_views));
                }
            }
        }
        if changed_views.is_empty() {
            let revision = self.global_revision(database)?;
            database.execute(StatementId::Commit.sql(), "transaction.commit")?;
            return Ok(RiseStoreCommit::new(revision, HashSet::new(), now));
        }
        let next_revision = self.global_revision(database)? + 1;
        self.set_global_revision(next_revision, database)?;
        for view in &changed_views {
            self.increment_view_revision(view, database)?;
        }
        database.execute(StatementId::Commit.sql(), "transaction.commit")?;
        if timeline_trace.iter().any(|entry| !entry.starts_with("up")) {
            let trace_text = Self::a3a1_compact(&timeline_trace);
            tracing::info!(
                target: "net.riseonly.ColdOpen",
                "[8311] a3a1 store rev={next_revision} {trace_text}"
            );
        }
        let commit = RiseStoreCommit::new(next_revision, changed_views, now);
        self.change_hub.publish(&commit);
        Ok(commit)
    }

    pub(crate) async fn value(
        self: &Arc<Self>,
        namespace: RiseStoreNamespace,
        key: String,
    ) -> Result<Option<RiseStoredValue>, RiseStoreError> {
        self.validate_identifier(&key, IdentifierKind::ValueKey.maximum_bytes(), "value.key")?;
        self.enqueue(move |store, database| {
            let mut budget = RiseStoreReadBudget::new(
                1,
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            store.read_value(&namespace, &key, Some(&mut budget), database)
        })
        .await
    }

    pub(crate) async fn ordered_items(
        self: &Arc<Self>,
        namespace: RiseStoreNamespace,
        collection: String,
        limit: i64,
        ascending: bool,
        anchor: Option<RiseCollectionAnchor>,
    ) -> Result<Vec<RiseOrderedItem>, RiseStoreError> {
        self.validate_identifier(
            &collection,
            IdentifierKind::CollectionKey.maximum_bytes(),
            "collection.key",
        )?;
        let maximum_page_items = self.configuration.limits.maximum_collection_page_items;
        if !(1..=maximum_page_items).contains(&limit) {
            return Err(RiseStoreError::CollectionPageLimitReached(maximum_page_items));
        }
        self.enqueue(move |store, database| {
            let mut budget = RiseStoreReadBudget::new(
                size(store.configuration.limits.maximum_read_result_items),
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            store.read_ordered_items(
                &namespace,
                &collection,
                limit,
                ascending,
                anchor.as_ref(),
                Some(&mut budget),
                database,
            )
        })
        .await
    }

    pub(crate) async fn collection_state(
        self: &Arc<Self>,
        namespace: RiseStoreNamespace,
        collection: String,
    ) -> Result<Option<RiseCollectionState>, RiseStoreError> {
        self.validate_identifier(
            &collection,
            IdentifierKind::CollectionKey.maximum_bytes(),
            "collection.key",
        )?;
        self.enqueue(move |store, database| {
            let mut budget = RiseStoreReadBudget::new(
                1,
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            store.read_collection_state(&namespace, &collection, Some(&mut budget), database)
        })
        .await
    }

    pub(crate) async fn entity_local_id(
        self: &Arc<Self>,
        namespace: RiseStoreNamespace,
        local_id: String,
    ) -> Result<Option<RiseStoreEntity>, RiseStoreError> {
        self.validate_identifier(
            &local_id,
            IdentifierKind::LocalId.maximum_bytes(),
            "entity.local_id",
        )?;
        self.enqueue(move |store, database| {
            let mut budget = RiseStoreReadBudget::new(
                1,
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            store.read_entity_local_id(&namespace, &local_id, Some(&mut budget), database)
        })
        .await
    }

    pub(crate) async fn entity_alias(
        self: &Arc<Self>,
        namespace: RiseStoreNamespace,
        alias: RiseEntityAlias,
    ) -> Result<Option<RiseStoreEntity>, RiseStoreError> {
        self.validate_entity_alias(&alias)?;
        self.enqueue(move |store, database| {
            let mut budget = RiseStoreReadBudget::new(
                1,
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            store.read_entity_alias(&namespace, &alias, Some(&mut budget), database)
        })
        .await
    }

    pub(crate) async fn entity_window(
        self: &Arc<Self>,
        query: RiseEntityWindowQuery,
    ) -> Result<RiseEntityWindowResult, RiseStoreError> {
        self.validate_entity_window_query(&query)?;
        self.enqueue(move |store, database| {
            let mut budget = RiseStoreReadBudget::new(
                size(store.configuration.limits.maximum_read_result_items),
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            database.execute(StatementId::BeginDeferred.sql(), "entity_window.begin")?;
            let result = store
                .read_entity_window(&query, Some(&mut budget), database)
                .and_then(|result| {
                    database.execute(StatementId::Commit.sql(), "entity_window.commit")?;
                    Ok(result)
                });
            if result.is_err() {
                let _ = database.execute(StatementId::Rollback.sql(), "entity_window.rollback");
            }
            result
        })
        .await
    }

    pub(crate) async fn entity_view_state(
        self: &Arc<Self>,
        namespace: RiseStoreNamespace,
        view: String,
    ) -> Result<Option<RiseEntityViewState>, RiseStoreError> {
        self.validate_identifier(&view, 4_096, "entity_view.id")?;
        self.enqueue(move |store, database| {
            let mut budget = RiseStoreReadBudget::new(
                1,
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            store.read_entity_view_state(&namespace, &view, Some(&mut budget), database)
        })
        .await
    }

    pub(crate) async fn entity_view_payload_bytes(
        self: &Arc<Self>,
        namespace: RiseStoreNamespace,
    ) -> Result<HashMap<String, i64>, RiseStoreError> {
        self.enqueue(move |_store, database| {
            let mut statement = database.statement(
                "SELECT v.view_id, SUM(length(e.payload)) \
                 FROM entity_views v \
                 JOIN entities e \
                   ON e.namespace = v.namespace AND e.local_id = v.local_id \
                 WHERE v.namespace = ? \
                 GROUP BY v.view_id",
                "entity_view.read_payload_bytes",
            )?;
            statement.bind(namespace.raw_value(), 1)?;
            let mut result = HashMap::new();
            while statement.step()? {
                let Some(view) = statement.text(0) else {
                    continue;
                };
                result.insert(view, statement.int64(1).max(0));
            }
            Ok(result)
        })
        .await
    }

    pub(crate) async fn entity_view_position(
        self: &Arc<Self>,
        namespace: RiseStoreNamespace,
        view: String,
        local_id: String,
    ) -> Result<Option<RiseEntityViewPosition>, RiseStoreError> {
        self.validate_identifier(&view, 4_096, "entity_view.id")?;
        self.validate_identifier(
            &local_id,
            IdentifierKind::LocalId.maximum_bytes(),
            "entity.local_id",
        )?;
        self.enqueue(move |store, database| {
            store.read_entity_view_position(&namespace, &view, &local_id, database)
        })
        .await
    }

    pub(crate) async fn retained_entity_view_positions(
        self: &Arc<Self>,
        namespace: RiseStoreNamespace,
        view: String,
    ) -> Result<Vec<RiseEntityViewPosition>, RiseStoreError> {
        self.validate_identifier(&view, 4_096, "entity_view.id")?;
        self.enqueue(move |store, database| {
            let limits = &store.configuration.limits;
            let mut statement = database.statement(
                "SELECT v.local_id, v.position_key FROM entity_views v \
                 JOIN entity_aliases a \
                   ON a.namespace = v.namespace AND a.local_id = v.local_id \
                 WHERE v.namespace = ? AND v.view_id = ? AND a.alias_kind = ? \
                 ORDER BY v.position_key ASC, v.local_id ASC \
                 LIMIT ?",
                "entity_view.read_retained",
            )?;
            let retained = RiseEntityAliasKind::retained();
            statement.bind(namespace.raw_value(), 1)?;
            statement.bind(view.as_str(), 2)?;
            statement.bind(retained.raw_value(), 3)?;
            statement.bind(limits.maximum_entity_window_items, 4)?;
            let mut positions = Vec::new();
            while statement.step()? {
                let local_id = statement.text(0).filter(|local_id| !local_id.is_empty());
                let raw_position = statement.data(1).filter(|raw_position| {
                    !raw_position.is_empty()
                        && count(raw_position.len()) <= limits.maximum_position_key_bytes
                });
                let (Some(local_id), Some(raw_position)) = (local_id, raw_position) else {
                    return Err(RiseStoreError::CorruptedValue(
                        "entity_view_position".to_owned(),
                    ));
                };
                positions.push(RiseEntityViewPosition::new(
                    RisePositionKey::new(raw_position),
                    local_id,
                ));
            }
            Ok(positions)
        })
        .await
    }

    pub(crate) async fn snapshot(
        self: &Arc<Self>,
        request: RiseStoreSnapshotRequest,
    ) -> Result<RiseStoreSnapshot, RiseStoreError> {
        self.validate_snapshot_request(&request)?;
        self.enqueue(move |store, database| store.read_snapshot(&request, database))
            .await
    }

    pub(crate) async fn operation(
        self: &Arc<Self>,
        request_id: RiseRequestID,
    ) -> Result<Option<RiseDurableOperation>, RiseStoreError> {
        self.enqueue(move |store, database| {
            let mut budget = RiseStoreReadBudget::new(
                1,
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            store.read_operation(request_id, Some(&mut budget), database)
        })
        .await
    }

    pub(crate) async fn terminal_operations_limit(
        self: &Arc<Self>,
        limit: i64,
    ) -> Result<Vec<RiseDurableOperation>, RiseStoreError> {
        let maximum = self.configuration.limits.maximum_durable_read_operations;
        if !(1..=maximum).contains(&limit) {
            return Err(RiseStoreError::ReadResultItemLimitReached(maximum));
        }
        self.enqueue(move |store, database| {
            let mut statement = database.statement(
                "SELECT request_id, ordering_key, kind, payload, state, attempt_count, \
                        next_attempt_ms, last_error_code, created_at_ms, updated_at_ms \
                 FROM durable_operations \
                 WHERE state = 'terminal_failure' \
                 ORDER BY enqueue_sequence ASC \
                 LIMIT ?",
                "durable.terminal",
            )?;
            statement.bind(limit, 1)?;
            let mut budget = RiseStoreReadBudget::new(
                size(store.configuration.limits.maximum_durable_read_operations),
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            store.read_operations(&mut statement, Some(&mut budget))
        })
        .await
    }

    pub(crate) async fn terminal_operations(
        self: &Arc<Self>,
    ) -> Result<Vec<RiseDurableOperation>, RiseStoreError> {
        self.terminal_operations_limit(self.configuration.limits.maximum_durable_read_operations)
            .await
    }

    pub(crate) async fn replayable_operations(
        self: &Arc<Self>,
        due_at_milliseconds: i64,
        limit: i64,
    ) -> Result<Vec<RiseDurableOperation>, RiseStoreError> {
        let maximum = self.configuration.limits.maximum_durable_read_operations;
        if !(1..=maximum).contains(&limit) {
            return Err(RiseStoreError::ReadResultItemLimitReached(maximum));
        }
        self.enqueue(move |store, database| {
            let mut statement = database.statement(
                "SELECT request_id, ordering_key, kind, payload, state, attempt_count, \
                        next_attempt_ms, last_error_code, created_at_ms, updated_at_ms \
                 FROM durable_operations AS candidate \
                 WHERE candidate.state IN ('queued', 'retry_waiting') \
                   AND candidate.next_attempt_ms <= ? \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM durable_operations AS earlier \
                       WHERE earlier.ordering_key = candidate.ordering_key \
                         AND earlier.enqueue_sequence < candidate.enqueue_sequence \
                         AND earlier.state != 'terminal_failure' \
                   ) \
                 ORDER BY candidate.enqueue_sequence ASC \
                 LIMIT ?",
                "durable.replayable",
            )?;
            statement.bind(due_at_milliseconds, 1)?;
            statement.bind(limit, 2)?;
            let mut budget = RiseStoreReadBudget::new(
                size(store.configuration.limits.maximum_durable_read_operations),
                size(store.configuration.limits.maximum_read_result_bytes),
            );
            store.read_operations(&mut statement, Some(&mut budget))
        })
        .await
    }

    pub(crate) async fn claim_replayable_operations(
        self: &Arc<Self>,
        due_at_milliseconds: i64,
        limit: i64,
    ) -> Result<Vec<RiseDurableClaim>, RiseStoreError> {
        let maximum = self.configuration.limits.maximum_durable_read_operations;
        if !(1..=maximum).contains(&limit) {
            return Err(RiseStoreError::ReadResultItemLimitReached(maximum));
        }
        self.enqueue(move |store, database| {
            database.execute(StatementId::BeginImmediate.sql(), "durable.claim.begin")?;
            match store.claim_transaction(due_at_milliseconds, limit, database) {
                Ok(claims) => Ok(claims),
                Err(error) => {
                    let _ = database.execute(StatementId::Rollback.sql(), "durable.claim.rollback");
                    Err(error)
                }
            }
        })
        .await
    }

    fn claim_transaction(
        &self,
        due_at_milliseconds: i64,
        limit: i64,
        database: &RiseSQLiteConnection,
    ) -> Result<Vec<RiseDurableClaim>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT request_id, ordering_key, kind, payload, state, attempt_count, \
                    next_attempt_ms, last_error_code, created_at_ms, updated_at_ms \
             FROM durable_operations AS candidate \
             WHERE candidate.state IN ('queued', 'retry_waiting') \
               AND candidate.claim_token IS NULL \
               AND candidate.next_attempt_ms <= ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM durable_operations AS earlier \
                   WHERE earlier.ordering_key = candidate.ordering_key \
                     AND earlier.enqueue_sequence < candidate.enqueue_sequence \
                     AND earlier.state != 'terminal_failure' \
               ) \
             ORDER BY candidate.enqueue_sequence ASC \
             LIMIT ?",
            "durable.claim.candidates",
        )?;
        statement.bind(due_at_milliseconds, 1)?;
        statement.bind(limit, 2)?;
        let mut budget = RiseStoreReadBudget::new(
            size(self.configuration.limits.maximum_durable_read_operations),
            size(self.configuration.limits.maximum_read_result_bytes),
        );
        let candidates = self.read_operations(&mut statement, Some(&mut budget))?;
        drop(statement);
        let now = self.configuration.clock.now_milliseconds();
        let mut claims = Vec::with_capacity(candidates.len());
        for operation in candidates {
            let token = Uuid::new_v4().uuid_string().to_lowercase();
            let mut update = database.statement(
                "UPDATE durable_operations \
                 SET state = 'in_flight', claim_token = ?, updated_at_ms = ? \
                 WHERE request_id = ? \
                   AND state IN ('queued', 'retry_waiting') \
                   AND attempt_count = ? \
                   AND claim_token IS NULL",
                "durable.claim.cas",
            )?;
            update.bind(token.as_str(), 1)?;
            update.bind(now, 2)?;
            update.bind(operation.request_id.raw_value().to_string().as_str(), 3)?;
            update.bind(operation.attempt_count, 4)?;
            update.step()?;
            drop(update);
            if database.changed_rows() != 1 {
                continue;
            }
            let claimed = operation.replacing(
                RiseDurableOperationState::InFlight,
                operation.attempt_count,
                operation.next_attempt_at_milliseconds,
                operation.last_error_code.clone(),
                now,
            );
            claims.push(RiseDurableClaim::new(claimed, token));
        }

        if claims.is_empty() {
            database.execute(StatementId::Commit.sql(), "durable.claim.commit")?;
            return Ok(Vec::new());
        }
        let durable_operations = RiseStoreViewKey::durable_operations();
        let next_revision = self.global_revision(database)? + 1;
        self.set_global_revision(next_revision, database)?;
        self.increment_view_revision(&durable_operations, database)?;
        database.execute(StatementId::Commit.sql(), "durable.claim.commit")?;
        self.change_hub.publish(&RiseStoreCommit::new(
            next_revision,
            HashSet::from([durable_operations]),
            now,
        ));
        Ok(claims)
    }

    /// crash recovery at startup only, not a store mutation
    /// so nobody can revoke another worker live claim through the public write path
    pub(crate) async fn recover_in_flight_operations(
        self: &Arc<Self>,
        updated_at_milliseconds: Option<i64>,
    ) -> Result<RiseStoreCommit, RiseStoreError> {
        self.enqueue(move |store, database| {
            let now = updated_at_milliseconds
                .unwrap_or_else(|| store.configuration.clock.now_milliseconds());
            database.execute(StatementId::BeginImmediate.sql(), "durable.recover.begin")?;
            match store.recover_transaction(now, database) {
                Ok(commit) => Ok(commit),
                Err(error) => {
                    let _ =
                        database.execute(StatementId::Rollback.sql(), "durable.recover.rollback");
                    Err(error)
                }
            }
        })
        .await
    }

    fn recover_transaction(
        &self,
        now: i64,
        database: &RiseSQLiteConnection,
    ) -> Result<RiseStoreCommit, RiseStoreError> {
        let mut statement = database.statement(
            "UPDATE durable_operations \
             SET state = 'queued', next_attempt_ms = ?, updated_at_ms = ?, \
                 claim_token = NULL \
             WHERE state = 'in_flight' AND claim_token IS NOT NULL",
            "durable.recover.in_flight",
        )?;
        statement.bind(now, 1)?;
        statement.bind(now, 2)?;
        statement.step()?;
        drop(statement);
        let changed = database.changed_rows() > 0;
        let durable_operations = RiseStoreViewKey::durable_operations();
        let revision = if changed {
            let revision = self.global_revision(database)? + 1;
            self.set_global_revision(revision, database)?;
            self.increment_view_revision(&durable_operations, database)?;
            revision
        } else {
            self.global_revision(database)?
        };
        database.execute(StatementId::Commit.sql(), "durable.recover.commit")?;
        let changed_views = if changed {
            HashSet::from([durable_operations])
        } else {
            HashSet::new()
        };
        let commit = RiseStoreCommit::new(revision, changed_views, now);
        if changed {
            self.change_hub.publish(&commit);
        }
        Ok(commit)
    }

    pub(crate) async fn next_replay_at_milliseconds(
        self: &Arc<Self>,
    ) -> Result<Option<i64>, RiseStoreError> {
        self.enqueue(move |_store, database| {
            let mut statement = database.statement(
                "SELECT MIN(candidate.next_attempt_ms) \
                 FROM durable_operations AS candidate \
                 WHERE candidate.state IN ('queued', 'retry_waiting') \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM durable_operations AS earlier \
                       WHERE earlier.ordering_key = candidate.ordering_key \
                         AND earlier.enqueue_sequence < candidate.enqueue_sequence \
                         AND earlier.state != 'terminal_failure' \
                   )",
                "durable.next_replay",
            )?;
            if !statement.step()? || statement.is_null(0) {
                return Ok(None);
            }
            Ok(Some(statement.int64(0)))
        })
        .await
    }

    pub(crate) async fn open_on_queue(self: &Arc<Self>) -> Result<(), RiseStoreError> {
        let store = Arc::clone(self);
        RiseAccountRuntime::current()
            .spawn_blocking(move || store.open_on_queue_blocking())
            .await
            .unwrap_or_else(|_| Err(RiseStoreError::NotOpen))
    }

    fn open_on_queue_blocking(&self) -> Result<(), RiseStoreError> {
        let mut slot = self.connection.lock();
        let result = self.open_connection(&mut slot);
        if result.is_err() {
            *slot = None;
        }
        result
    }

    fn open_connection(
        &self,
        slot: &mut Option<RiseSQLiteConnection>,
    ) -> Result<(), RiseStoreError> {
        let url = self.configuration.file_url.as_path();
        let is_memory = url.as_os_str() == MEMORY_PATH
            || url.file_name() == Some(OsStr::new(MEMORY_PATH));
        if !is_memory {
            if let Some(directory) = url.parent() {
                FileManager
                    .create_directory(directory, true)
                    .map_err(|_| Self::file_system_error("open.directory"))?;
                if self.configuration.applies_file_protection {
                    self.protect(directory)?;
                }
            }
        }
        let path = if is_memory {
            MEMORY_PATH
        } else {
            url.to_str().ok_or_else(|| Self::file_system_error("open"))?
        };
        let database = slot.insert(RiseSQLiteConnection::new(path)?);
        self.configure(database)?;
        self.migrate(database)?;
        if !is_memory && self.configuration.applies_file_protection {
            self.protect_database_files(url)?;
        }
        Ok(())
    }

    fn file_system_error(operation: &str) -> RiseStoreError {
        RiseStoreError::Sqlite {
            code: rusqlite::ffi::SQLITE_CANTOPEN,
            operation: operation.to_owned(),
        }
    }

    pub(crate) async fn enqueue<T, F>(self: &Arc<Self>, operation: F) -> Result<T, RiseStoreError>
    where
        T: Send + 'static,
        F: FnOnce(&RiseStore, &RiseSQLiteConnection) -> Result<T, RiseStoreError>
            + Send
            + 'static,
    {
        self.admission_gate.acquire().await?;
        let store = Arc::clone(self);
        let runtime = RiseAccountRuntime::current();
        let spawner = Arc::clone(&runtime);
        let flight = spawner.spawn(async move {
            let worker = Arc::clone(&store);
            let result = runtime
                .spawn_blocking(move || worker.run_on_queue(operation))
                .await;
            store.admission_gate.release().await;
            result
        });
        match flight.await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) | Err(_) => Err(RiseStoreError::NotOpen),
        }
    }

    fn run_on_queue<T, F>(&self, operation: F) -> Result<T, RiseStoreError>
    where
        F: FnOnce(&RiseStore, &RiseSQLiteConnection) -> Result<T, RiseStoreError>,
    {
        let connection = self.connection.lock();
        match connection.as_ref() {
            Some(database) if !self.is_closed.load(Ordering::Acquire) => operation(self, database),
            _ => Err(RiseStoreError::NotOpen),
        }
    }

    pub(crate) async fn close_on_queue(self: &Arc<Self>) -> Result<(), RiseStoreError> {
        let store = Arc::clone(self);
        RiseAccountRuntime::current()
            .spawn_blocking(move || store.close_on_queue_blocking())
            .await
            .unwrap_or_else(|_| Err(RiseStoreError::NotOpen))
    }

    fn close_on_queue_blocking(&self) -> Result<(), RiseStoreError> {
        let mut connection = self.connection.lock();
        if self.is_closed.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(database) = connection.as_mut() {
            database.close()?;
        }
        *connection = None;
        self.clear_entity_read_cache(None);
        self.is_closed.store(true, Ordering::Release);
        self.change_hub.finish();
        Ok(())
    }

    pub(crate) fn configure(&self, database: &RiseSQLiteConnection) -> Result<(), RiseStoreError> {
        for pragma in PRAGMA_STATEMENTS {
            let name = pragma
                .strip_prefix("PRAGMA ")
                .and_then(|setting| setting.split('=').next())
                .unwrap_or(pragma);
            database.execute(pragma, &format!("pragma.{name}"))?;
        }
        Ok(())
    }

    pub(crate) fn migrate(&self, database: &RiseSQLiteConnection) -> Result<(), RiseStoreError> {
        let mut version_statement = database.statement("PRAGMA user_version", "schema.version")?;
        if !version_statement.step()? {
            return Err(RiseStoreError::CorruptedValue(
                payload::SCHEMA_VERSION.to_owned(),
            ));
        }
        let found = i32::try_from(version_statement.int64(0))
            .map_err(|_| RiseStoreError::CorruptedValue(payload::SCHEMA_VERSION.to_owned()))?;
        version_statement.release();
        drop(version_statement);
        let supported = Self::supported_schema_version();
        if found > supported {
            return Err(RiseStoreError::UnsupportedSchema { found, supported });
        }
        if found == supported {
            return self.verify_key_verifier(database);
        }

        if found >= 2 {
            self.verify_key_verifier(database)?;
        }

        database.execute("BEGIN EXCLUSIVE", "migration.begin")?;
        let result = self.migrate_steps(found, database).and_then(|()| {
            database.execute(StatementId::Commit.sql(), "migration.commit")
        });
        if result.is_err() {
            let _ = database.execute(StatementId::Rollback.sql(), "migration.rollback");
        }
        result
    }

    fn migrate_steps(&self, found: i32, database: &RiseSQLiteConnection) -> Result<(), RiseStoreError> {
        for step in MIGRATIONS.iter() {
            if i64::from(step.to_version) <= i64::from(found) {
                continue;
            }
            if step.to_version == 2 && found == 1 {
                self.verify_legacy_cipher(database)?;
            }
            let operation = format!("migration.v{}", step.to_version);
            for statement in step.statements {
                database.execute(statement, &operation)?;
            }
            if step.to_version == 2 {
                self.create_key_verifier(database)?;
            }
        }
        Ok(())
    }

    pub(crate) fn create_key_verifier(
        &self,
        database: &RiseSQLiteConnection,
    ) -> Result<(), RiseStoreError> {
        let store_id = Uuid::new_v4().uuid_string().to_lowercase();
        let mut store_id_statement =
            database.statement(StatementId::WriteStoreId.sql(), "crypto.store_id.create")?;
        store_id_statement.bind(store_id.as_str(), 1)?;
        store_id_statement.step()?;
        drop(store_id_statement);

        let context = self.cipher_context(CipherContextKind::KeyVerifier.domain(), &[store_id.as_str()]);
        let sealed = self
            .configuration
            .cipher
            .seal(Self::key_verifier_plaintext(), &context)?;
        let mut verifier_statement =
            database.statement(StatementId::WriteKeyVerifier.sql(), "crypto.verifier.create")?;
        verifier_statement.bind(sealed.as_slice(), 1)?;
        verifier_statement.step()?;
        Ok(())
    }

    pub(crate) fn verify_key_verifier(
        &self,
        database: &RiseSQLiteConnection,
    ) -> Result<(), RiseStoreError> {
        let mut store_id_statement =
            database.statement(StatementId::ReadStoreId.sql(), "crypto.store_id.read")?;
        if !store_id_statement.step()? {
            return Err(RiseStoreError::CorruptedValue("store_id".to_owned()));
        }
        let Some(store_id) = store_id_statement
            .text(0)
            .filter(|store_id| !store_id.is_empty())
        else {
            return Err(RiseStoreError::CorruptedValue("store_id".to_owned()));
        };
        let mut verifier_statement =
            database.statement(StatementId::ReadKeyVerifier.sql(), "crypto.verifier.read")?;
        if !verifier_statement.step()? || verifier_statement.byte_count(0) > 256 {
            return Err(RiseStoreError::CorruptedValue("key_verifier".to_owned()));
        }
        let Some(sealed) = verifier_statement.data(0) else {
            return Err(RiseStoreError::CorruptedValue("key_verifier".to_owned()));
        };
        let context = self.cipher_context(CipherContextKind::KeyVerifier.domain(), &[store_id.as_str()]);
        match self.configuration.cipher.open(&sealed, &context) {
            Ok(plaintext) if plaintext.as_slice() == Self::key_verifier_plaintext() => Ok(()),
            _ => Err(RiseStoreError::InvalidEncryptionKey),
        }
    }

    pub(crate) fn verify_legacy_cipher(
        &self,
        database: &RiseSQLiteConnection,
    ) -> Result<(), RiseStoreError> {
        let verified = (|| -> Result<(), RiseStoreError> {
            if self.verify_legacy_key_value(database)? {
                return Ok(());
            }
            if self.verify_legacy_ordered_item(database)? {
                return Ok(());
            }
            if self.verify_legacy_collection_cursor(database, "before_cursor")? {
                return Ok(());
            }
            if self.verify_legacy_collection_cursor(database, "after_cursor")? {
                return Ok(());
            }
            self.verify_legacy_durable_operation(database)?;
            Ok(())
        })();
        verified.map_err(|_| RiseStoreError::InvalidEncryptionKey)
    }

    fn open_legacy(&self, sealed: &[u8], domain: &str, components: &[&str]) -> Result<(), RiseStoreError> {
        let context = self.cipher_context(domain, components);
        self.configuration
            .cipher
            .open(sealed, &context)
            .map(|_| ())
            .map_err(|_| RiseStoreError::InvalidEncryptionKey)
    }

    fn verify_legacy_key_value(&self, database: &RiseSQLiteConnection) -> Result<bool, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT namespace, key, value FROM key_values LIMIT 1",
            "crypto.legacy.key_value",
        )?;
        if !statement.step()? {
            return Ok(false);
        }
        let (Some(namespace), Some(key)) = (statement.text(0), statement.text(1)) else {
            return Ok(false);
        };
        let Some(sealed) =
            self.legacy_sealed_data(&statement, 2, self.configuration.limits.maximum_value_bytes)?
        else {
            return Ok(false);
        };
        self.open_legacy(
            &sealed,
            CipherContextKind::KeyValue.domain(),
            &[namespace.as_str(), key.as_str()],
        )?;
        Ok(true)
    }

    fn verify_legacy_ordered_item(
        &self,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT namespace, collection_key, item_id, value FROM ordered_items LIMIT 1",
            "crypto.legacy.ordered_item",
        )?;
        if !statement.step()? {
            return Ok(false);
        }
        let (Some(namespace), Some(collection), Some(item_id)) =
            (statement.text(0), statement.text(1), statement.text(2))
        else {
            return Ok(false);
        };
        let Some(sealed) =
            self.legacy_sealed_data(&statement, 3, self.configuration.limits.maximum_value_bytes)?
        else {
            return Ok(false);
        };
        self.open_legacy(
            &sealed,
            CipherContextKind::OrderedItem.domain(),
            &[namespace.as_str(), collection.as_str(), item_id.as_str()],
        )?;
        Ok(true)
    }

    fn verify_legacy_collection_cursor(
        &self,
        database: &RiseSQLiteConnection,
        column: &str,
    ) -> Result<bool, RiseStoreError> {
        let domain = if column == "before_cursor" {
            CipherContextKind::CollectionCursorBefore.domain()
        } else {
            CipherContextKind::CollectionCursorAfter.domain()
        };
        let mut statement = database.statement(
            &format!(
                "SELECT namespace, collection_key, {column} \
                 FROM collection_state WHERE {column} IS NOT NULL LIMIT 1"
            ),
            &format!("crypto.legacy.{column}"),
        )?;
        if !statement.step()? {
            return Ok(false);
        }
        let (Some(namespace), Some(collection)) = (statement.text(0), statement.text(1)) else {
            return Ok(false);
        };
        let Some(sealed) =
            self.legacy_sealed_data(&statement, 2, self.configuration.limits.maximum_value_bytes)?
        else {
            return Ok(false);
        };
        self.open_legacy(&sealed, domain, &[namespace.as_str(), collection.as_str()])?;
        Ok(true)
    }

    fn verify_legacy_durable_operation(
        &self,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT request_id, payload FROM durable_operations LIMIT 1",
            "crypto.legacy.durable_operation",
        )?;
        if !statement.step()? {
            return Ok(false);
        }
        let Some(request_id) = statement.text(0) else {
            return Ok(false);
        };
        let Some(sealed) = self.legacy_sealed_data(
            &statement,
            1,
            self.configuration.limits.maximum_durable_payload_bytes,
        )?
        else {
            return Ok(false);
        };
        self.open_legacy(
            &sealed,
            CipherContextKind::DurableOperation.domain(),
            &[request_id.as_str()],
        )?;
        Ok(true)
    }

    fn legacy_sealed_data(
        &self,
        statement: &RiseSQLiteStatement<'_>,
        at: i32,
        plaintext_limit: i64,
    ) -> Result<Option<Vec<u8>>, RiseStoreError> {
        if statement.is_null(at) {
            return Ok(None);
        }
        match plaintext_limit.checked_add(128) {
            Some(maximum_bytes) if statement.byte_count(at) <= maximum_bytes => {
                Ok(statement.data(at))
            }
            _ => Err(RiseStoreError::CorruptedValue(
                "oversized_legacy_encrypted_value".to_owned(),
            )),
        }
    }

    pub(crate) fn read_snapshot(
        &self,
        request: &RiseStoreSnapshotRequest,
        database: &RiseSQLiteConnection,
    ) -> Result<RiseStoreSnapshot, RiseStoreError> {
        database.execute(StatementId::BeginDeferred.sql(), "snapshot.begin")?;
        let result = self.read_snapshot_transaction(request, database);
        if result.is_err() {
            let _ = database.execute(StatementId::Rollback.sql(), "snapshot.rollback");
        }
        result
    }

    fn read_snapshot_transaction(
        &self,
        request: &RiseStoreSnapshotRequest,
        database: &RiseSQLiteConnection,
    ) -> Result<RiseStoreSnapshot, RiseStoreError> {
        let mut budget = RiseStoreReadBudget::new(
            size(self.configuration.limits.maximum_read_result_items),
            size(self.configuration.limits.maximum_read_result_bytes),
        );
        let mut values = HashMap::with_capacity(request.values.len());
        for key in &request.values {
            if let Some(value) =
                self.read_value(&key.namespace, &key.key, Some(&mut budget), database)?
            {
                values.insert(key.clone(), value);
            }
        }

        let mut collections = HashMap::with_capacity(request.collections.len());
        for query in &request.collections {
            let items = self.read_ordered_items(
                &query.key.namespace,
                &query.key.collection,
                query.limit,
                query.ascending,
                query.anchor.as_ref(),
                Some(&mut budget),
                database,
            )?;
            collections.insert(query.clone(), items);
        }

        let mut collection_states = HashMap::with_capacity(request.collection_states.len());
        for key in &request.collection_states {
            if let Some(state) =
                self.read_collection_state(&key.namespace, &key.collection, Some(&mut budget), database)?
            {
                collection_states.insert(key.clone(), state);
            }
        }

        let mut entities = HashMap::with_capacity(request.entities.len());
        for key in &request.entities {
            if let Some(entity) =
                self.read_entity_local_id(&key.namespace, &key.local_id, Some(&mut budget), database)?
            {
                entities.insert(key.clone(), entity);
            }
        }

        let mut entity_aliases = HashMap::with_capacity(request.entity_aliases.len());
        for lookup in &request.entity_aliases {
            if let Some(entity) =
                self.read_entity_alias(&lookup.namespace, &lookup.alias, Some(&mut budget), database)?
            {
                entity_aliases.insert(lookup.clone(), entity);
            }
        }

        let mut entity_windows = HashMap::with_capacity(request.entity_windows.len());
        for query in &request.entity_windows {
            let window = self.read_entity_window(query, Some(&mut budget), database)?;
            entity_windows.insert(query.clone(), window);
        }

        let mut entity_view_states = HashMap::with_capacity(request.entity_view_states.len());
        for key in &request.entity_view_states {
            if let Some(state) =
                self.read_entity_view_state(&key.namespace, &key.view, Some(&mut budget), database)?
            {
                entity_view_states.insert(key.clone(), state);
            }
        }
        let snapshot = RiseStoreSnapshot {
            revision: self.global_revision(database)?,
            values,
            collections,
            collection_states,
            entities,
            entity_aliases,
            entity_windows,
            entity_view_states,
        };
        database.execute(StatementId::Commit.sql(), "snapshot.commit")?;
        Ok(snapshot)
    }

    pub(crate) fn validate_snapshot_request(
        &self,
        request: &RiseStoreSnapshotRequest,
    ) -> Result<(), RiseStoreError> {
        let limits = &self.configuration.limits;
        let key_count = request.values.len()
            + request.collections.len()
            + request.collection_states.len()
            + request.entities.len()
            + request.entity_aliases.len()
            + request.entity_windows.len()
            + request.entity_view_states.len();
        if count(key_count) > limits.maximum_snapshot_keys {
            return Err(RiseStoreError::SnapshotKeyLimitReached(limits.maximum_snapshot_keys));
        }
        if count(request.collections.len()) > limits.maximum_snapshot_collection_queries {
            return Err(RiseStoreError::SnapshotCollectionQueryLimitReached(
                limits.maximum_snapshot_collection_queries,
            ));
        }
        if count(request.entity_windows.len()) > limits.maximum_snapshot_entity_window_queries {
            return Err(RiseStoreError::SnapshotEntityWindowQueryLimitReached(
                limits.maximum_snapshot_entity_window_queries,
            ));
        }

        let mut requested_items = count(
            request.values.len()
                + request.collection_states.len()
                + request.entities.len()
                + request.entity_aliases.len()
                + request.entity_view_states.len()
                + request.entity_windows.len(),
        );
        for query in &request.collections {
            if !(1..=limits.maximum_collection_page_items).contains(&query.limit) {
                return Err(RiseStoreError::CollectionPageLimitReached(
                    limits.maximum_collection_page_items,
                ));
            }
            match requested_items.checked_add(query.limit) {
                Some(next) if next <= limits.maximum_read_result_items => requested_items = next,
                _ => {
                    return Err(RiseStoreError::ReadResultItemLimitReached(
                        limits.maximum_read_result_items,
                    ));
                }
            }
        }
        for lookup in &request.entity_aliases {
            self.validate_entity_alias(&lookup.alias)?;
        }
        for query in &request.entity_windows {
            self.validate_entity_window_query(query)?;
            match requested_items.checked_add(count(query.window.requested_item_count())) {
                Some(next) if next <= limits.maximum_read_result_items => requested_items = next,
                _ => {
                    return Err(RiseStoreError::ReadResultItemLimitReached(
                        limits.maximum_read_result_items,
                    ));
                }
            }
        }
        if requested_items > limits.maximum_read_result_items {
            return Err(RiseStoreError::ReadResultItemLimitReached(
                limits.maximum_read_result_items,
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_entity_window_query(
        &self,
        query: &RiseEntityWindowQuery,
    ) -> Result<(), RiseStoreError> {
        self.validate_identifier(&query.key.view, 4_096, "entity_view.id")?;
        let maximum = self.configuration.limits.maximum_entity_window_items;
        match &query.window {
            RiseEntityWindow::First { limit }
            | RiseEntityWindow::Latest { limit }
            | RiseEntityWindow::Before { limit, .. }
            | RiseEntityWindow::After { limit, .. } => {
                if !(1..=maximum).contains(&count(*limit)) {
                    return Err(RiseStoreError::EntityWindowLimitReached(maximum));
                }
            }
            RiseEntityWindow::Around {
                before_limit,
                after_limit,
                ..
            } => {
                if count(*before_limit) < 0
                    || count(*after_limit) < 0
                    || count(query.window.requested_item_count()) > maximum
                {
                    return Err(RiseStoreError::EntityWindowLimitReached(maximum));
                }
            }
        }

        let position = match &query.window {
            RiseEntityWindow::First { .. } | RiseEntityWindow::Latest { .. } => None,
            RiseEntityWindow::Before { anchor, .. }
            | RiseEntityWindow::After { anchor, .. }
            | RiseEntityWindow::Around { anchor, .. } => Some(anchor),
        };
        if let Some(position) = position {
            if count(position.position_key.raw_value().len())
                > self.configuration.limits.maximum_position_key_bytes
            {
                return Err(RiseStoreError::InvalidIdentifier(
                    "entity_view.position_key".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn validate_entity_alias(&self, alias: &RiseEntityAlias) -> Result<(), RiseStoreError> {
        self.validate_identifier(alias.kind.raw_value(), 256, "entity_alias.kind")?;
        self.validate_identifier(&alias.value, 4_096, "entity_alias.value")
    }

    pub(crate) fn validate_identifiers(&self, batch: &RiseStoreWriteBatch) -> Result<(), RiseStoreError> {
        let local_id_bytes = IdentifierKind::LocalId.maximum_bytes();
        let collection_bytes = IdentifierKind::CollectionKey.maximum_bytes();
        let claim_token_bytes = IdentifierKind::ClaimToken.maximum_bytes();
        let maximum_window_items = self.configuration.limits.maximum_entity_window_items;
        for mutation in &batch.mutations {
            match mutation {
                RiseStoreMutation::SetValue { key, .. }
                | RiseStoreMutation::RemoveValue { key, .. } => {
                    self.validate_identifier(
                        key,
                        IdentifierKind::ValueKey.maximum_bytes(),
                        "value.key",
                    )?;
                }
                RiseStoreMutation::UpsertOrderedItem {
                    collection, item, ..
                } => {
                    self.validate_identifier(collection, collection_bytes, "collection.key")?;
                    self.validate_identifier(
                        &item.item_id,
                        IdentifierKind::ItemId.maximum_bytes(),
                        "item.id",
                    )?;
                    if let Some(server_id) = &item.server_id {
                        self.validate_identifier(server_id, 4_096, "server.id")?;
                    }
                }
                RiseStoreMutation::RemoveOrderedItem {
                    collection,
                    item_id,
                    ..
                } => {
                    self.validate_identifier(collection, collection_bytes, "collection.key")?;
                    self.validate_identifier(
                        item_id,
                        IdentifierKind::ItemId.maximum_bytes(),
                        "item.id",
                    )?;
                }
                RiseStoreMutation::SetCollectionState { collection, .. } => {
                    self.validate_identifier(collection, collection_bytes, "collection.key")?;
                }
                RiseStoreMutation::UpsertEntity { entity, .. } => {
                    self.validate_identifier(&entity.local_id, local_id_bytes, "entity.local_id")?;
                }
                RiseStoreMutation::RemoveEntity { local_id, .. } => {
                    self.validate_identifier(local_id, local_id_bytes, "entity.local_id")?;
                }
                RiseStoreMutation::SetEntityAlias {
                    alias, local_id, ..
                } => {
                    self.validate_entity_alias(alias)?;
                    self.validate_identifier(local_id, local_id_bytes, "entity.local_id")?;
                }
                RiseStoreMutation::RemoveEntityAlias { alias, .. } => {
                    self.validate_entity_alias(alias)?;
                }
                RiseStoreMutation::UpsertEntityViewMembership {
                    view, membership, ..
                } => {
                    self.validate_identifier(view, 4_096, "entity_view.id")?;
                    self.validate_identifier(
                        &membership.local_id,
                        local_id_bytes,
                        "entity.local_id",
                    )?;
                    if count(membership.position_key.raw_value().len())
                        > self.configuration.limits.maximum_position_key_bytes
                    {
                        return Err(RiseStoreError::InvalidIdentifier(
                            "entity_view.position_key".to_owned(),
                        ));
                    }
                }
                RiseStoreMutation::RemoveEntityViewMembership { view, local_id, .. } => {
                    self.validate_identifier(view, 4_096, "entity_view.id")?;
                    self.validate_identifier(local_id, local_id_bytes, "entity.local_id")?;
                }
                RiseStoreMutation::RemoveAllEntityViewMemberships { view, .. } => {
                    self.validate_identifier(view, 4_096, "entity_view.id")?;
                }
                RiseStoreMutation::TrimEntityView { view, keeping, .. } => {
                    self.validate_identifier(view, 4_096, "entity_view.id")?;
                    if !(1..=maximum_window_items).contains(&count(keeping.limit())) {
                        return Err(RiseStoreError::EntityWindowLimitReached(
                            maximum_window_items,
                        ));
                    }
                }
                RiseStoreMutation::PruneUnreferencedEntities { limit, .. } => {
                    if !(1..=maximum_window_items).contains(limit) {
                        return Err(RiseStoreError::EntityWindowLimitReached(
                            maximum_window_items,
                        ));
                    }
                }
                RiseStoreMutation::MergeEntities {
                    survivor_local_id,
                    duplicate_local_id,
                    ..
                } => {
                    self.validate_identifier(
                        survivor_local_id,
                        local_id_bytes,
                        "entity.survivor_local_id",
                    )?;
                    self.validate_identifier(
                        duplicate_local_id,
                        local_id_bytes,
                        "entity.duplicate_local_id",
                    )?;
                    if survivor_local_id == duplicate_local_id {
                        return Err(RiseStoreError::InvalidIdentifier(
                            "entity.merge_same_local_id".to_owned(),
                        ));
                    }
                }
                RiseStoreMutation::SetEntityViewState { view, state, .. } => {
                    self.validate_identifier(view, 4_096, "entity_view.id")?;
                    self.validate_optional_text(
                        state.last_error_code.as_deref(),
                        1_024,
                        "entity_view.last_error",
                    )?;
                }
                RiseStoreMutation::PutDurableOperation(operation) => {
                    self.validate_identifier(
                        &operation.kind,
                        IdentifierKind::OperationKind.maximum_bytes(),
                        "operation.kind",
                    )?;
                    self.validate_identifier(
                        operation.ordering_key.raw_value(),
                        IdentifierKind::OrderingKey.maximum_bytes(),
                        "operation.ordering_key",
                    )?;
                    self.validate_optional_text(
                        operation.last_error_code.as_deref(),
                        1_024,
                        "operation.last_error",
                    )?;
                }
                RiseStoreMutation::ResolveTerminalOperation { .. } => {}
                RiseStoreMutation::TransitionClaimedDurableOperation {
                    request_id,
                    claim_token,
                    state,
                    last_error_code,
                    ..
                } => {
                    self.validate_identifier(
                        claim_token,
                        claim_token_bytes,
                        "operation.claim_token",
                    )?;
                    if *state == RiseDurableOperationState::InFlight {
                        return Err(RiseStoreError::DurableOperationStatePreconditionFailed(
                            *request_id,
                        ));
                    }
                    self.validate_optional_text(
                        last_error_code.as_deref(),
                        1_024,
                        "operation.last_error",
                    )?;
                }
                RiseStoreMutation::RemoveClaimedDurableOperation { claim_token, .. } => {
                    self.validate_identifier(
                        claim_token,
                        claim_token_bytes,
                        "operation.claim_token",
                    )?;
                }
                RiseStoreMutation::RemoveUnclaimedDurableOperation { .. } => {}
            }
        }
        Ok(())
    }

    pub(crate) fn validate_identifier(
        &self,
        value: &str,
        maximum_bytes: usize,
        field: &str,
    ) -> Result<(), RiseStoreError> {
        if value.is_empty() || value.len() > maximum_bytes {
            return Err(RiseStoreError::InvalidIdentifier(field.to_owned()));
        }
        Ok(())
    }

    pub(crate) fn validate_optional_text(
        &self,
        value: Option<&str>,
        maximum_bytes: usize,
        field: &str,
    ) -> Result<(), RiseStoreError> {
        let Some(value) = value else {
            return Ok(());
        };
        if value.len() > maximum_bytes {
            return Err(RiseStoreError::InvalidIdentifier(field.to_owned()));
        }
        Ok(())
    }

    pub(crate) fn static_changed_views_for_mutation(
        &self,
        mutation: &RiseStoreMutation,
    ) -> HashSet<RiseStoreViewKey> {
        match mutation {
            RiseStoreMutation::SetValue { namespace, key, .. }
            | RiseStoreMutation::RemoveValue { namespace, key, .. } => {
                HashSet::from([RiseStoreViewKey::value(namespace, key)])
            }
            RiseStoreMutation::UpsertOrderedItem {
                namespace,
                collection,
                ..
            }
            | RiseStoreMutation::RemoveOrderedItem {
                namespace,
                collection,
                ..
            }
            | RiseStoreMutation::SetCollectionState {
                namespace,
                collection,
                ..
            } => HashSet::from([RiseStoreViewKey::collection(namespace, collection)]),
            RiseStoreMutation::UpsertEntity {
                namespace, entity, ..
            } => HashSet::from([RiseStoreViewKey::entity(namespace, &entity.local_id)]),
            RiseStoreMutation::RemoveEntity {
                namespace,
                local_id,
                ..
            } => HashSet::from([RiseStoreViewKey::entity(namespace, local_id)]),
            RiseStoreMutation::SetEntityAlias {
                namespace, alias, ..
            }
            | RiseStoreMutation::RemoveEntityAlias {
                namespace, alias, ..
            } => {
                HashSet::from([RiseStoreViewKey::entity_alias(namespace, alias)])
            }
            RiseStoreMutation::UpsertEntityViewMembership {
                namespace, view, ..
            }
            | RiseStoreMutation::RemoveEntityViewMembership {
                namespace, view, ..
            }
            | RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace, view, ..
            }
            | RiseStoreMutation::TrimEntityView {
                namespace, view, ..
            }
            | RiseStoreMutation::SetEntityViewState {
                namespace, view, ..
            } => HashSet::from([RiseStoreViewKey::entity_view(namespace, view)]),
            RiseStoreMutation::PruneUnreferencedEntities { .. } => HashSet::new(),
            RiseStoreMutation::MergeEntities {
                namespace,
                survivor_local_id,
                duplicate_local_id,
                ..
            } => HashSet::from([
                RiseStoreViewKey::entity(namespace, survivor_local_id),
                RiseStoreViewKey::entity(namespace, duplicate_local_id),
            ]),
            RiseStoreMutation::PutDurableOperation(_)
            | RiseStoreMutation::TransitionClaimedDurableOperation { .. }
            | RiseStoreMutation::RemoveClaimedDurableOperation { .. }
            | RiseStoreMutation::RemoveUnclaimedDurableOperation { .. }
            | RiseStoreMutation::ResolveTerminalOperation { .. } => {
                HashSet::from([RiseStoreViewKey::durable_operations()])
            }
        }
    }

    pub(crate) fn static_changed_views_for_batch(
        &self,
        batch: &RiseStoreWriteBatch,
    ) -> HashSet<RiseStoreViewKey> {
        let mut result = batch.invalidated_views.clone();
        for mutation in &batch.mutations {
            result.extend(self.static_changed_views_for_mutation(mutation));
        }
        result
    }

    pub(crate) fn changed_views(
        &self,
        mutation: &RiseStoreMutation,
        database: &RiseSQLiteConnection,
    ) -> Result<HashSet<RiseStoreViewKey>, RiseStoreError> {
        let mut result = self.static_changed_views_for_mutation(mutation);
        match mutation {
            RiseStoreMutation::UpsertEntity {
                namespace, entity, ..
            } => {
                self.append_entity_references(namespace, &entity.local_id, &mut result, database)?;
            }
            RiseStoreMutation::RemoveEntity {
                namespace,
                local_id,
                ..
            } => {
                self.append_entity_references(namespace, local_id, &mut result, database)?;
            }
            RiseStoreMutation::PruneUnreferencedEntities {
                namespace, limit, ..
            } => {
                self.select_pruned_entity_references(namespace, *limit, &mut result, database)?;
            }
            RiseStoreMutation::MergeEntities {
                namespace,
                survivor_local_id,
                duplicate_local_id,
                ..
            } => {
                self.append_entity_references(
                    namespace,
                    survivor_local_id,
                    &mut result,
                    database,
                )?;
                self.append_entity_references(
                    namespace,
                    duplicate_local_id,
                    &mut result,
                    database,
                )?;
            }
            _ => {}
        }
        let maximum_batch_views = self.configuration.limits.maximum_batch_views;
        if count(result.len()) > maximum_batch_views {
            return Err(RiseStoreError::BatchViewLimitReached(maximum_batch_views));
        }
        Ok(result)
    }

    fn append_entity_references(
        &self,
        namespace: &RiseStoreNamespace,
        local_id: &str,
        result: &mut HashSet<RiseStoreViewKey>,
        database: &RiseSQLiteConnection,
    ) -> Result<(), RiseStoreError> {
        let maximum_batch_views = self.configuration.limits.maximum_batch_views;
        let mut views = database.statement(
            "SELECT view_id FROM entity_views \
             WHERE namespace = ? AND local_id = ? \
             LIMIT ?",
            "entity.references.views",
        )?;
        views.bind(namespace.raw_value(), 1)?;
        views.bind(local_id, 2)?;
        views.bind(maximum_batch_views.saturating_add(1), 3)?;
        while views.step()? {
            let Some(view) = views
                .text(0)
                .filter(|view| !view.is_empty() && view.len() <= 4_096)
            else {
                return Err(RiseStoreError::CorruptedValue(
                    "entity_view_reference".to_owned(),
                ));
            };
            result.insert(RiseStoreViewKey::entity_view(namespace, &view));
            if count(result.len()) > maximum_batch_views {
                return Err(RiseStoreError::BatchViewLimitReached(maximum_batch_views));
            }
        }
        drop(views);

        let mut aliases = database.statement(
            "SELECT alias_kind, alias_value FROM entity_aliases \
             WHERE namespace = ? AND local_id = ? \
             LIMIT ?",
            "entity.references.aliases",
        )?;
        aliases.bind(namespace.raw_value(), 1)?;
        aliases.bind(local_id, 2)?;
        aliases.bind(maximum_batch_views.saturating_add(1), 3)?;
        while aliases.step()? {
            let kind = aliases
                .text(0)
                .filter(|kind| !kind.is_empty() && kind.len() <= 256);
            let value = aliases
                .text(1)
                .filter(|value| !value.is_empty() && value.len() <= 4_096);
            let (Some(kind), Some(value)) = (kind, value) else {
                return Err(RiseStoreError::CorruptedValue(
                    "entity_alias_reference".to_owned(),
                ));
            };
            result.insert(RiseStoreViewKey::entity_alias(
                namespace,
                &RiseEntityAlias::new(RiseEntityAliasKind::new(kind), value),
            ));
            if count(result.len()) > maximum_batch_views {
                return Err(RiseStoreError::BatchViewLimitReached(maximum_batch_views));
            }
        }
        Ok(())
    }

    pub(crate) fn select_pruned_entity_references(
        &self,
        namespace: &RiseStoreNamespace,
        limit: i64,
        result: &mut HashSet<RiseStoreViewKey>,
        database: &RiseSQLiteConnection,
    ) -> Result<Vec<String>, RiseStoreError> {
        let maximum_batch_views = self.configuration.limits.maximum_batch_views;
        let mut local_ids: Vec<String> = Vec::new();
        let mut statement = database.statement(
            "WITH candidates AS ( \
                 SELECT entity.local_id \
                 FROM entities AS entity \
                 WHERE entity.namespace = ? \
                   AND NOT EXISTS ( \
                       SELECT 1 FROM entity_views AS membership \
                       WHERE membership.namespace = entity.namespace \
                         AND membership.local_id = entity.local_id \
                   ) \
                 ORDER BY entity.local_id ASC \
                 LIMIT ? \
             ) \
             SELECT candidate.local_id, alias.alias_kind, alias.alias_value \
             FROM candidates AS candidate \
             LEFT JOIN entity_aliases AS alias \
               ON alias.namespace = ? AND alias.local_id = candidate.local_id \
             ORDER BY candidate.local_id ASC",
            "entity.prune_references",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(limit, 2)?;
        statement.bind(namespace.raw_value(), 3)?;
        while statement.step()? {
            let Some(local_id) = statement
                .text(0)
                .filter(|local_id| !local_id.is_empty() && local_id.len() <= 4_096)
            else {
                return Err(RiseStoreError::CorruptedValue(
                    "pruned_entity_reference".to_owned(),
                ));
            };
            result.insert(RiseStoreViewKey::entity(namespace, &local_id));
            if local_ids.last() != Some(&local_id) {
                local_ids.push(local_id);
            }
            let kind = statement.text(1);
            let value = statement.text(2);
            if kind.is_some() || value.is_some() {
                let kind = kind.filter(|kind| !kind.is_empty() && kind.len() <= 256);
                let value = value.filter(|value| !value.is_empty() && value.len() <= 4_096);
                let (Some(kind), Some(value)) = (kind, value) else {
                    return Err(RiseStoreError::CorruptedValue(
                        "pruned_entity_alias_reference".to_owned(),
                    ));
                };
                result.insert(RiseStoreViewKey::entity_alias(
                    namespace,
                    &RiseEntityAlias::new(RiseEntityAliasKind::new(kind), value),
                ));
            }
            if count(result.len()) > maximum_batch_views {
                return Err(RiseStoreError::BatchViewLimitReached(maximum_batch_views));
            }
        }
        Ok(local_ids)
    }

    fn coalescing_prunes(mutations: &[RiseStoreMutation]) -> Vec<Cow<'_, RiseStoreMutation>> {
        let mut result: Vec<Cow<'_, RiseStoreMutation>> = Vec::with_capacity(mutations.len());
        for mutation in mutations {
            let merged = match (mutation, result.last().map(|last| &**last)) {
                (
                    RiseStoreMutation::PruneUnreferencedEntities { namespace, limit },
                    Some(RiseStoreMutation::PruneUnreferencedEntities {
                        namespace: previous_namespace,
                        limit: previous_limit,
                    }),
                ) if namespace == previous_namespace => {
                    Some(RiseStoreMutation::PruneUnreferencedEntities {
                        namespace: namespace.clone(),
                        limit: (*previous_limit).max(*limit),
                    })
                }
                _ => None,
            };
            match (merged, result.last_mut()) {
                (Some(merged), Some(last)) => *last = Cow::Owned(merged),
                _ => result.push(Cow::Borrowed(mutation)),
            }
        }
        result
    }

    pub(crate) fn validate_value_bounds(
        &self,
        batch: &RiseStoreWriteBatch,
    ) -> Result<(), RiseStoreError> {
        let limits = &self.configuration.limits;
        let mut byte_count: i64 = 0;
        for mutation in &batch.mutations {
            let value_bytes = count(match mutation {
                RiseStoreMutation::SetValue { value, .. } => value.data.len(),
                RiseStoreMutation::UpsertOrderedItem { item, .. } => item.data.len(),
                RiseStoreMutation::SetCollectionState { state, .. } => {
                    state.before_cursor.as_ref().map_or(0, |cursor| cursor.len())
                        + state.after_cursor.as_ref().map_or(0, |cursor| cursor.len())
                }
                RiseStoreMutation::UpsertEntity { entity, .. } => entity.data.len(),
                RiseStoreMutation::SetEntityViewState { state, .. } => {
                    state.before_cursor.as_ref().map_or(0, |cursor| cursor.len())
                        + state.after_cursor.as_ref().map_or(0, |cursor| cursor.len())
                }
                RiseStoreMutation::PutDurableOperation(operation) => {
                    if count(operation.payload.len()) > limits.maximum_durable_payload_bytes {
                        return Err(RiseStoreError::DurableOperationPayloadTooLarge(
                            limits.maximum_durable_payload_bytes,
                        ));
                    }
                    operation.payload.len()
                }
                _ => 0,
            });
            if value_bytes > limits.maximum_value_bytes {
                return Err(RiseStoreError::ValueByteLimitReached(limits.maximum_value_bytes));
            }
            match byte_count.checked_add(value_bytes) {
                Some(next) if next <= limits.maximum_batch_value_bytes => byte_count = next,
                _ => {
                    return Err(RiseStoreError::BatchValueByteLimitReached(
                        limits.maximum_batch_value_bytes,
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn global_revision(&self, database: &RiseSQLiteConnection) -> Result<i64, RiseStoreError> {
        let mut statement =
            database.statement(StatementId::ReadGlobalRevision.sql(), "revision.read")?;
        if !statement.step()? {
            return Err(RiseStoreError::CorruptedValue("global_revision".to_owned()));
        }
        statement
            .text(0)
            .and_then(|text| text.parse::<i64>().ok())
            .ok_or_else(|| RiseStoreError::CorruptedValue("global_revision".to_owned()))
    }

    pub(crate) fn set_global_revision(
        &self,
        revision: i64,
        database: &RiseSQLiteConnection,
    ) -> Result<(), RiseStoreError> {
        let mut statement =
            database.statement(StatementId::WriteGlobalRevision.sql(), "revision.write")?;
        statement.bind(revision.to_string().as_str(), 1)?;
        statement.step()?;
        Ok(())
    }

    pub(crate) fn increment_view_revision(
        &self,
        key: &RiseStoreViewKey,
        database: &RiseSQLiteConnection,
    ) -> Result<(), RiseStoreError> {
        let mut statement = database.statement(
            StatementId::IncrementViewRevision.sql(),
            "view_revision.increment",
        )?;
        statement.bind(key.raw_value(), 1)?;
        statement.step()?;
        Ok(())
    }

    pub(crate) fn cipher_context(&self, domain: &str, components: &[&str]) -> Vec<u8> {
        fn append(result: &mut Vec<u8>, value: &str) {
            result.extend_from_slice(&(value.len() as u64).to_be_bytes());
            result.extend_from_slice(value.as_bytes());
        }
        let prefix: &[u8] = &CIPHER_AAD_PREFIX;
        let capacity = prefix.len()
            + (2 + components.len()) * LENGTH_PREFIX_BYTES
            + self.configuration.account_id.len()
            + domain.len()
            + components.iter().map(|value| value.len()).sum::<usize>();
        let mut result = Vec::with_capacity(capacity);
        result.extend_from_slice(prefix);
        append(&mut result, &self.configuration.account_id);
        append(&mut result, domain);
        for value in components {
            append(&mut result, value);
        }
        result
    }

    fn protect(&self, directory_url: &Path) -> Result<(), RiseStoreError> {
        let values = URLResourceValues {
            is_excluded_from_backup: Some(true),
        };
        set_resource_values(directory_url, &values)
            .map_err(|_| Self::file_system_error("open.protect"))
    }

    fn protect_database_files(&self, database_url: &Path) -> Result<(), RiseStoreError> {
        let values = URLResourceValues {
            is_excluded_from_backup: Some(true),
        };
        for url in [
            database_url.to_path_buf(),
            Self::sibling_path(database_url, "-wal"),
            Self::sibling_path(database_url, "-shm"),
        ] {
            if !FileManager.file_exists(&url) {
                continue;
            }
            set_resource_values(&url, &values)
                .map_err(|_| Self::file_system_error("open.protect"))?;
        }
        Ok(())
    }

    fn sibling_path(database_url: &Path, suffix: &str) -> PathBuf {
        let mut path = OsString::from(database_url.as_os_str());
        path.push(suffix);
        PathBuf::from(path)
    }
}

static A3A1_HASH_SEED: LazyLock<std::hash::RandomState> = LazyLock::new(std::hash::RandomState::new);

impl RiseStore {
    fn a3a1_tag(local_id: &str) -> String {
        use std::hash::BuildHasher;
        let hash = A3A1_HASH_SEED.hash_one(format!("msg-{local_id}")) as i64;
        (hash.unsigned_abs() % 10_000).to_string()
    }

    fn a3a1_timeline_entry(mutation: &RiseStoreMutation) -> Option<String> {
        match mutation {
            RiseStoreMutation::UpsertEntityViewMembership { view, membership, .. }
                if view.contains("message.timeline") =>
            {
                Some(format!("up:{}", Self::a3a1_tag(&membership.local_id)))
            }
            RiseStoreMutation::RemoveEntityViewMembership { view, local_id, .. }
                if view.contains("message.timeline") =>
            {
                Some(format!("rm:{}", Self::a3a1_tag(local_id)))
            }
            RiseStoreMutation::RemoveAllEntityViewMemberships { view, .. }
                if view.contains("message.timeline") =>
            {
                Some("rmAll".to_owned())
            }
            RiseStoreMutation::TrimEntityView { view, .. } if view.contains("message.timeline") => {
                Some("trim".to_owned())
            }
            RiseStoreMutation::MergeEntities {
                namespace,
                survivor_local_id,
                duplicate_local_id,
            } if namespace.raw_value().contains("messages") => Some(format!(
                "merge:{}<{}",
                Self::a3a1_tag(survivor_local_id),
                Self::a3a1_tag(duplicate_local_id)
            )),
            RiseStoreMutation::RemoveEntity { namespace, local_id }
                if namespace.raw_value().contains("messages") =>
            {
                Some(format!("rmEntity:{}", Self::a3a1_tag(local_id)))
            }
            _ => None,
        }
    }

    fn a3a1_compact(entries: &[String]) -> String {
        let ups: Vec<&str> =
            entries.iter().filter(|entry| entry.starts_with("up:")).map(String::as_str).collect();
        let rest: Vec<&str> =
            entries.iter().filter(|entry| !entry.starts_with("up:")).map(String::as_str).collect();
        let newest_ups = ups[ups.len().saturating_sub(3)..].join(",");
        format!("{} ups={} lastUps={}", rest.join(" "), ups.len(), newest_ups)
    }
}
