use std::collections::{HashMap, HashSet};
use std::fmt;

use ::rise_store::view as canon_view;
use ::rise_store::{IdentifierKind, Limit};
use serde::{Deserialize, Serialize};

use super::rise_store_cipher::RiseAESGCMStoreCipherError;
use super::rise_store_concurrency::RiseStoreUpdateSequence;
use super::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasLookup, RiseEntityKey, RiseEntityViewKey,
    RiseEntityViewMembership, RiseEntityViewRetention, RiseEntityViewState, RiseEntityWindowQuery,
    RiseEntityWindowResult, RiseEntityWritePolicy, RiseStoreEntity,
};
use crate::rise_core::rise_identifiers::RiseRequestID;

fn is_identifier(value: &str, kind: IdentifierKind) -> bool {
    !value.is_empty() && value.len() <= kind.maximum_bytes()
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RiseStoreNamespace {
    raw_value: String,
}

impl RiseStoreNamespace {
    pub fn new(raw_value: impl Into<String>) -> Self {
        let raw_value = raw_value.into();
        assert!(is_identifier(&raw_value, IdentifierKind::Namespace));
        Self { raw_value }
    }

    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }

    pub fn description(&self) -> String {
        self.raw_value.clone()
    }
}

impl TryFrom<String> for RiseStoreNamespace {
    type Error = RiseStoreError;

    fn try_from(raw_value: String) -> Result<Self, Self::Error> {
        if !is_identifier(&raw_value, IdentifierKind::Namespace) {
            return Err(RiseStoreError::InvalidIdentifier(raw_value));
        }
        Ok(Self { raw_value })
    }
}

impl From<RiseStoreNamespace> for String {
    fn from(namespace: RiseStoreNamespace) -> Self {
        namespace.raw_value
    }
}

impl fmt::Display for RiseStoreNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw_value)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RiseStoreViewKey {
    raw_value: String,
}

impl RiseStoreViewKey {
    pub fn new(raw_value: impl Into<String>) -> Self {
        let raw_value = raw_value.into();
        assert!(is_identifier(&raw_value, IdentifierKind::ViewKey));
        Self { raw_value }
    }

    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }

    pub fn value(namespace: &RiseStoreNamespace, key: &str) -> Self {
        Self::new(canon_view::value(namespace.raw_value(), key))
    }

    pub fn collection(namespace: &RiseStoreNamespace, key: &str) -> Self {
        Self::new(canon_view::collection(namespace.raw_value(), key))
    }

    pub fn entity(namespace: &RiseStoreNamespace, local_id: &str) -> Self {
        Self::new(canon_view::entity(namespace.raw_value(), local_id))
    }

    pub fn entity_alias(namespace: &RiseStoreNamespace, alias: &RiseEntityAlias) -> Self {
        Self::new(canon_view::entity_alias(
            namespace.raw_value(),
            alias.kind.raw_value(),
            &alias.value,
        ))
    }

    pub fn entity_view(namespace: &RiseStoreNamespace, view: &str) -> Self {
        Self::new(canon_view::entity_view(namespace.raw_value(), view))
    }

    pub fn durable_operations() -> Self {
        Self::new(canon_view::DURABLE_OPERATIONS)
    }

    pub fn description(&self) -> String {
        self.raw_value.clone()
    }
}

impl TryFrom<String> for RiseStoreViewKey {
    type Error = RiseStoreError;

    fn try_from(raw_value: String) -> Result<Self, Self::Error> {
        if !is_identifier(&raw_value, IdentifierKind::ViewKey) {
            return Err(RiseStoreError::InvalidIdentifier(raw_value));
        }
        Ok(Self { raw_value })
    }
}

impl From<RiseStoreViewKey> for String {
    fn from(key: RiseStoreViewKey) -> Self {
        key.raw_value
    }
}

impl fmt::Display for RiseStoreViewKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw_value)
    }
}

/// any or missing or equals(i64), the canon type so the probe rule is shared
pub type RiseStoreVersionPrecondition = ::rise_store::VersionPrecondition;

/// the three conflict resolutions, canon
pub type RiseStoredValueWritePolicy = ::rise_store::WritePolicy;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseStoredValue {
    pub data: Vec<u8>,
    pub version: i64,
    pub updated_at_milliseconds: i64,
}

impl RiseStoredValue {
    pub fn new(data: Vec<u8>, version: i64, updated_at_milliseconds: i64) -> Self {
        Self {
            data,
            version,
            updated_at_milliseconds,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseOrderedItem {
    pub item_id: String,
    pub server_id: Option<String>,
    pub sort_key: i64,
    pub data: Vec<u8>,
    pub version: i64,
    pub updated_at_milliseconds: i64,
}

impl RiseOrderedItem {
    pub fn new(
        item_id: impl Into<String>,
        server_id: Option<String>,
        sort_key: i64,
        data: Vec<u8>,
        version: i64,
        updated_at_milliseconds: i64,
    ) -> Self {
        let item_id = item_id.into();
        assert!(is_identifier(&item_id, IdentifierKind::ItemId));
        Self {
            item_id,
            server_id,
            sort_key,
            data,
            version,
            updated_at_milliseconds,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseCollectionState {
    pub revision: i64,
    pub before_cursor: Option<Vec<u8>>,
    pub after_cursor: Option<Vec<u8>>,
    pub has_more_before: bool,
    pub has_more_after: bool,
}

impl RiseCollectionState {
    pub fn new(
        revision: i64,
        before_cursor: Option<Vec<u8>>,
        after_cursor: Option<Vec<u8>>,
        has_more_before: bool,
        has_more_after: bool,
    ) -> Self {
        Self {
            revision,
            before_cursor,
            after_cursor,
            has_more_before,
            has_more_after,
        }
    }
}

pub type RiseOrderedItemWritePolicy = ::rise_store::WritePolicy;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum RiseDurableOperationState {
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "in_flight")]
    InFlight,
    #[serde(rename = "retry_waiting")]
    RetryWaiting,
    #[serde(rename = "terminal_failure")]
    TerminalFailure,
}

impl RiseDurableOperationState {
    pub const ALL_CASES: [Self; 4] = [
        Self::Queued,
        Self::InFlight,
        Self::RetryWaiting,
        Self::TerminalFailure,
    ];

    pub const fn durable_state(self) -> ::rise_store::DurableState {
        match self {
            Self::Queued => ::rise_store::DurableState::Queued,
            Self::InFlight => ::rise_store::DurableState::InFlight,
            Self::RetryWaiting => ::rise_store::DurableState::RetryWaiting,
            Self::TerminalFailure => ::rise_store::DurableState::TerminalFailure,
        }
    }

    pub const fn raw_value(self) -> &'static str {
        self.durable_state().wire()
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL_CASES
            .into_iter()
            .find(|state| state.raw_value() == raw_value)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RiseDurableOrderingKey {
    raw_value: String,
}

impl RiseDurableOrderingKey {
    pub fn new(raw_value: impl Into<String>) -> Self {
        let raw_value = raw_value.into();
        assert!(is_identifier(&raw_value, IdentifierKind::OrderingKey));
        Self { raw_value }
    }

    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }

    pub fn global() -> Self {
        Self::new("global")
    }
}

impl TryFrom<String> for RiseDurableOrderingKey {
    type Error = RiseStoreError;

    fn try_from(raw_value: String) -> Result<Self, Self::Error> {
        if !is_identifier(&raw_value, IdentifierKind::OrderingKey) {
            return Err(RiseStoreError::InvalidIdentifier(raw_value));
        }
        Ok(Self { raw_value })
    }
}

impl From<RiseDurableOrderingKey> for String {
    fn from(key: RiseDurableOrderingKey) -> Self {
        key.raw_value
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseDurableOperation {
    pub request_id: RiseRequestID,
    pub kind: String,
    pub payload: Vec<u8>,
    pub ordering_key: RiseDurableOrderingKey,
    pub state: RiseDurableOperationState,
    pub attempt_count: i64,
    pub next_attempt_at_milliseconds: i64,
    pub last_error_code: Option<String>,
    pub created_at_milliseconds: i64,
    pub updated_at_milliseconds: i64,
}

impl RiseDurableOperation {
    /// new durable op with defaults, global ordering and queued and no attempts or error
    /// set other fields with struct update, validated checks them
    pub fn new(
        request_id: RiseRequestID,
        kind: impl Into<String>,
        payload: Vec<u8>,
        next_attempt_at_milliseconds: i64,
        created_at_milliseconds: i64,
        updated_at_milliseconds: i64,
    ) -> Self {
        Self {
            request_id,
            kind: kind.into(),
            payload,
            ordering_key: RiseDurableOrderingKey::global(),
            state: RiseDurableOperationState::Queued,
            attempt_count: 0,
            next_attempt_at_milliseconds,
            last_error_code: None,
            created_at_milliseconds,
            updated_at_milliseconds,
        }
        .validated()
    }

    pub fn validated(self) -> Self {
        assert!(is_identifier(&self.kind, IdentifierKind::OperationKind));
        assert!(self.attempt_count >= 0);
        self
    }

    pub fn replacing(
        &self,
        state: RiseDurableOperationState,
        attempt_count: i64,
        next_attempt_at_milliseconds: i64,
        last_error_code: Option<String>,
        updated_at_milliseconds: i64,
    ) -> Self {
        Self {
            request_id: self.request_id,
            kind: self.kind.clone(),
            payload: self.payload.clone(),
            ordering_key: self.ordering_key.clone(),
            state,
            attempt_count,
            next_attempt_at_milliseconds,
            last_error_code,
            created_at_milliseconds: self.created_at_milliseconds,
            updated_at_milliseconds,
        }
        .validated()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseDurableClaim {
    pub operation: RiseDurableOperation,
    pub token: String,
}

impl RiseDurableClaim {
    pub fn new(operation: RiseDurableOperation, token: impl Into<String>) -> Self {
        let token = token.into();
        assert!(is_identifier(&token, IdentifierKind::ClaimToken));
        Self { operation, token }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RiseDurableTerminalResolution {
    Remove,
    Requeue {
        next_attempt_at_milliseconds: i64,
        reset_attempt_count: bool,
    },
}

#[derive(Clone, Debug)]
pub enum RiseStoreMutation {
    SetValue {
        namespace: RiseStoreNamespace,
        key: String,
        value: RiseStoredValue,
        precondition: RiseStoreVersionPrecondition,
        policy: RiseStoredValueWritePolicy,
    },
    RemoveValue {
        namespace: RiseStoreNamespace,
        key: String,
        precondition: RiseStoreVersionPrecondition,
    },
    UpsertOrderedItem {
        namespace: RiseStoreNamespace,
        collection: String,
        item: RiseOrderedItem,
        policy: RiseOrderedItemWritePolicy,
    },
    RemoveOrderedItem {
        namespace: RiseStoreNamespace,
        collection: String,
        item_id: String,
    },
    SetCollectionState {
        namespace: RiseStoreNamespace,
        collection: String,
        state: RiseCollectionState,
    },
    UpsertEntity {
        namespace: RiseStoreNamespace,
        entity: RiseStoreEntity,
        policy: RiseEntityWritePolicy,
    },
    RemoveEntity {
        namespace: RiseStoreNamespace,
        local_id: String,
    },
    SetEntityAlias {
        namespace: RiseStoreNamespace,
        alias: RiseEntityAlias,
        local_id: String,
    },
    RemoveEntityAlias {
        namespace: RiseStoreNamespace,
        alias: RiseEntityAlias,
    },
    UpsertEntityViewMembership {
        namespace: RiseStoreNamespace,
        view: String,
        membership: RiseEntityViewMembership,
    },
    RemoveEntityViewMembership {
        namespace: RiseStoreNamespace,
        view: String,
        local_id: String,
        if_version_at_most: Option<i64>,
    },
    RemoveAllEntityViewMemberships {
        namespace: RiseStoreNamespace,
        view: String,
        if_version_at_most: Option<i64>,
    },
    TrimEntityView {
        namespace: RiseStoreNamespace,
        view: String,
        keeping: RiseEntityViewRetention,
        if_version_at_most: Option<i64>,
    },
    PruneUnreferencedEntities {
        namespace: RiseStoreNamespace,
        limit: i64,
    },
    MergeEntities {
        namespace: RiseStoreNamespace,
        survivor_local_id: String,
        duplicate_local_id: String,
    },
    SetEntityViewState {
        namespace: RiseStoreNamespace,
        view: String,
        state: RiseEntityViewState,
    },
    PutDurableOperation(RiseDurableOperation),
    TransitionClaimedDurableOperation {
        request_id: RiseRequestID,
        claim_token: String,
        state: RiseDurableOperationState,
        attempt_count: i64,
        next_attempt_at_milliseconds: i64,
        last_error_code: Option<String>,
        updated_at_milliseconds: i64,
    },
    RemoveClaimedDurableOperation {
        request_id: RiseRequestID,
        claim_token: String,
    },
    RemoveUnclaimedDurableOperation {
        request_id: RiseRequestID,
    },
    ResolveTerminalOperation {
        request_id: RiseRequestID,
        resolution: RiseDurableTerminalResolution,
        updated_at_milliseconds: i64,
    },
}

#[derive(Clone, Debug, Default)]
pub struct RiseStoreWriteBatch {
    pub mutations: Vec<RiseStoreMutation>,
    pub invalidated_views: HashSet<RiseStoreViewKey>,
}

impl RiseStoreWriteBatch {
    pub fn new(
        mutations: Vec<RiseStoreMutation>,
        invalidated_views: HashSet<RiseStoreViewKey>,
    ) -> Self {
        Self {
            mutations,
            invalidated_views,
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub struct RiseStoreCommit {
    pub revision: i64,
    pub changed_views: HashSet<RiseStoreViewKey>,
    pub committed_at_milliseconds: i64,
    pub requires_full_refresh: bool,
}

impl RiseStoreCommit {
    /// requires_full_refresh defaults to false
    /// the change hub sets it directly when it coalesces an overflow
    pub(crate) fn new(
        revision: i64,
        changed_views: HashSet<RiseStoreViewKey>,
        committed_at_milliseconds: i64,
    ) -> Self {
        Self {
            revision,
            changed_views,
            committed_at_milliseconds,
            requires_full_refresh: false,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseStoreValueKey {
    pub namespace: RiseStoreNamespace,
    pub key: String,
}

impl RiseStoreValueKey {
    pub fn new(namespace: RiseStoreNamespace, key: impl Into<String>) -> Self {
        let key = key.into();
        assert!(is_identifier(&key, IdentifierKind::ValueKey));
        Self { namespace, key }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseStoreCollectionKey {
    pub namespace: RiseStoreNamespace,
    pub collection: String,
}

impl RiseStoreCollectionKey {
    pub fn new(namespace: RiseStoreNamespace, collection: impl Into<String>) -> Self {
        let collection = collection.into();
        assert!(is_identifier(&collection, IdentifierKind::CollectionKey));
        Self {
            namespace,
            collection,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseOrderedItemPosition {
    pub sort_key: i64,
    pub item_id: String,
}

impl RiseOrderedItemPosition {
    pub fn new(sort_key: i64, item_id: impl Into<String>) -> Self {
        let item_id = item_id.into();
        assert!(is_identifier(&item_id, IdentifierKind::ItemId));
        Self { sort_key, item_id }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RiseCollectionAnchor {
    Before(RiseOrderedItemPosition),
    After(RiseOrderedItemPosition),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseStoreCollectionQuery {
    pub key: RiseStoreCollectionKey,
    pub limit: i64,
    pub ascending: bool,
    pub anchor: Option<RiseCollectionAnchor>,
}

impl RiseStoreCollectionQuery {
    pub fn new(
        namespace: RiseStoreNamespace,
        collection: impl Into<String>,
        limit: i64,
        ascending: bool,
        anchor: Option<RiseCollectionAnchor>,
    ) -> Self {
        Self {
            key: RiseStoreCollectionKey::new(namespace, collection),
            limit,
            ascending,
            anchor,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RiseStoreSnapshotRequest {
    pub values: HashSet<RiseStoreValueKey>,
    pub collections: HashSet<RiseStoreCollectionQuery>,
    pub collection_states: HashSet<RiseStoreCollectionKey>,
    pub entities: HashSet<RiseEntityKey>,
    pub entity_aliases: HashSet<RiseEntityAliasLookup>,
    pub entity_windows: HashSet<RiseEntityWindowQuery>,
    pub entity_view_states: HashSet<RiseEntityViewKey>,
}

impl RiseStoreSnapshotRequest {
    pub fn required_view_keys(&self) -> HashSet<RiseStoreViewKey> {
        let mut result: HashSet<RiseStoreViewKey> = self
            .values
            .iter()
            .map(|value| RiseStoreViewKey::value(&value.namespace, &value.key))
            .collect();
        result.extend(self.collections.iter().map(|query| {
            RiseStoreViewKey::collection(&query.key.namespace, &query.key.collection)
        }));
        result.extend(
            self.collection_states
                .iter()
                .map(|key| RiseStoreViewKey::collection(&key.namespace, &key.collection)),
        );
        result.extend(
            self.entities
                .iter()
                .map(|key| RiseStoreViewKey::entity(&key.namespace, &key.local_id)),
        );
        result.extend(
            self.entity_aliases
                .iter()
                .map(|lookup| RiseStoreViewKey::entity_alias(&lookup.namespace, &lookup.alias)),
        );
        result.extend(
            self.entity_windows
                .iter()
                .map(|query| RiseStoreViewKey::entity_view(&query.key.namespace, &query.key.view)),
        );
        result.extend(
            self.entity_view_states
                .iter()
                .map(|key| RiseStoreViewKey::entity_view(&key.namespace, &key.view)),
        );
        result
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RiseStoreSnapshot {
    pub revision: i64,
    pub values: HashMap<RiseStoreValueKey, RiseStoredValue>,
    pub collections: HashMap<RiseStoreCollectionQuery, Vec<RiseOrderedItem>>,
    pub collection_states: HashMap<RiseStoreCollectionKey, RiseCollectionState>,
    pub entities: HashMap<RiseEntityKey, RiseStoreEntity>,
    pub entity_aliases: HashMap<RiseEntityAliasLookup, RiseStoreEntity>,
    pub entity_windows: HashMap<RiseEntityWindowQuery, RiseEntityWindowResult>,
    pub entity_view_states: HashMap<RiseEntityViewKey, RiseEntityViewState>,
}

#[non_exhaustive]
pub struct RiseStoreObservation {
    pub seed: RiseStoreSnapshot,
    pub updates: RiseStoreUpdateSequence,
}

impl RiseStoreObservation {
    pub(crate) fn new(seed: RiseStoreSnapshot, updates: RiseStoreUpdateSequence) -> Self {
        Self { seed, updates }
    }
}

const fn canon(limit: Limit) -> i64 {
    limit.value() as i64
}

const MINIMUM_ENTITY_WINDOW_ITEMS: i64 = 320;
const MAXIMUM_POSITION_KEY_BYTES_CEILING: i64 = 8_192;

/// custom limits are built from PRODUCTION with struct update and passed through validated
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RiseStoreLimits {
    pub maximum_pending_operations: i64,
    pub maximum_batch_mutations: i64,
    pub maximum_batch_views: i64,
    pub maximum_value_bytes: i64,
    pub maximum_batch_value_bytes: i64,
    pub maximum_snapshot_keys: i64,
    pub maximum_snapshot_collection_queries: i64,
    pub maximum_collection_page_items: i64,
    pub maximum_snapshot_entity_window_queries: i64,
    pub maximum_entity_window_items: i64,
    pub maximum_position_key_bytes: i64,
    pub maximum_read_result_items: i64,
    pub maximum_read_result_bytes: i64,
    pub maximum_durable_read_operations: i64,
    pub maximum_durable_operations: i64,
    pub maximum_durable_bytes: i64,
    pub maximum_durable_payload_bytes: i64,
    pub maximum_observers: i64,
}

impl RiseStoreLimits {
    pub const PRODUCTION: Self = Self {
        maximum_pending_operations: canon(Limit::MaximumPendingOperations),
        maximum_batch_mutations: canon(Limit::MaximumBatchMutations),
        maximum_batch_views: canon(Limit::MaximumBatchViews),
        maximum_value_bytes: canon(Limit::MaximumValueBytes),
        maximum_batch_value_bytes: canon(Limit::MaximumBatchValueBytes),
        maximum_snapshot_keys: canon(Limit::MaximumSnapshotKeys),
        maximum_snapshot_collection_queries: canon(Limit::MaximumSnapshotCollectionQueries),
        maximum_collection_page_items: canon(Limit::MaximumCollectionPageItems),
        maximum_snapshot_entity_window_queries: canon(Limit::MaximumSnapshotEntityWindowQueries),
        maximum_entity_window_items: canon(Limit::MaximumEntityWindowItems),
        maximum_position_key_bytes: canon(Limit::MaximumPositionKeyBytes),
        maximum_read_result_items: canon(Limit::MaximumReadResultItems),
        maximum_read_result_bytes: canon(Limit::MaximumReadResultBytes),
        maximum_durable_read_operations: canon(Limit::MaximumDurableReadOperations),
        maximum_durable_operations: canon(Limit::MaximumDurableOperations),
        maximum_durable_bytes: canon(Limit::MaximumDurableBytes),
        maximum_durable_payload_bytes: canon(Limit::MaximumDurablePayloadBytes),
        maximum_observers: canon(Limit::MaximumObservers),
    };

    pub fn production() -> Self {
        Self::PRODUCTION.validated()
    }

    pub fn validated(self) -> Self {
        assert!(self.maximum_pending_operations > 0);
        assert!(self.maximum_batch_mutations > 0);
        assert!(self.maximum_batch_views > 0);
        assert!(self.maximum_value_bytes > 0);
        assert!(self.maximum_batch_value_bytes >= self.maximum_value_bytes);
        assert!(self.maximum_snapshot_keys > 0);
        assert!(self.maximum_snapshot_collection_queries > 0);
        assert!(self.maximum_collection_page_items > 0);
        assert!(self.maximum_snapshot_entity_window_queries > 0);
        assert!(self.maximum_entity_window_items >= MINIMUM_ENTITY_WINDOW_ITEMS);
        assert!(
            self.maximum_position_key_bytes > 0
                && self.maximum_position_key_bytes <= MAXIMUM_POSITION_KEY_BYTES_CEILING
        );
        assert!(self.maximum_read_result_items >= self.maximum_collection_page_items);
        assert!(self.maximum_read_result_bytes >= self.maximum_value_bytes);
        assert!(self.maximum_durable_read_operations > 0);
        assert!(self.maximum_durable_operations > 0);
        assert!(self.maximum_durable_bytes > 0);
        assert!(self.maximum_durable_payload_bytes > 0);
        assert!(self.maximum_durable_payload_bytes <= self.maximum_value_bytes);
        assert!(self.maximum_durable_bytes >= self.maximum_durable_payload_bytes);
        assert!(self.maximum_observers > 0);
        self
    }
}

impl Default for RiseStoreLimits {
    fn default() -> Self {
        Self::production()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum RiseStoreError {
    #[error("store is not open")]
    NotOpen,
    #[error("store is already closed")]
    AlreadyClosed,
    #[error("admission queue is full (limit {limit})")]
    AdmissionQueueFull { limit: i64 },
    #[error("batch mutation limit reached ({0})")]
    BatchMutationLimitReached(i64),
    #[error("batch view limit reached ({0})")]
    BatchViewLimitReached(i64),
    #[error("value byte limit reached ({0})")]
    ValueByteLimitReached(i64),
    #[error("batch value byte limit reached ({0})")]
    BatchValueByteLimitReached(i64),
    #[error("snapshot key limit reached ({0})")]
    SnapshotKeyLimitReached(i64),
    #[error("snapshot collection query limit reached ({0})")]
    SnapshotCollectionQueryLimitReached(i64),
    #[error("collection page limit reached ({0})")]
    CollectionPageLimitReached(i64),
    #[error("snapshot entity window query limit reached ({0})")]
    SnapshotEntityWindowQueryLimitReached(i64),
    #[error("entity window limit reached ({0})")]
    EntityWindowLimitReached(i64),
    #[error("read result item limit reached ({0})")]
    ReadResultItemLimitReached(i64),
    #[error("read result byte limit reached ({0})")]
    ReadResultByteLimitReached(i64),
    #[error("durable operation count limit reached ({0})")]
    DurableOperationCountLimitReached(i64),
    #[error("durable operation byte limit reached ({0})")]
    DurableOperationByteLimitReached(i64),
    #[error("durable operation payload too large ({0})")]
    DurableOperationPayloadTooLarge(i64),
    #[error("durable operation state precondition failed for {0}")]
    DurableOperationStatePreconditionFailed(RiseRequestID),
    #[error("ordered item version conflict in {namespace}/{collection}/{item_id}")]
    OrderedItemVersionConflict {
        namespace: RiseStoreNamespace,
        collection: String,
        item_id: String,
    },
    #[error("stored value version conflict in {namespace}/{key}")]
    StoredValueVersionConflict {
        namespace: RiseStoreNamespace,
        key: String,
    },
    #[error("collection revision conflict in {namespace}/{collection}")]
    CollectionRevisionConflict {
        namespace: RiseStoreNamespace,
        collection: String,
    },
    #[error("entity version conflict in {namespace}/{local_id}")]
    EntityVersionConflict {
        namespace: RiseStoreNamespace,
        local_id: String,
    },
    #[error("entity reference not found in {namespace}/{local_id}")]
    EntityReferenceNotFound {
        namespace: RiseStoreNamespace,
        local_id: String,
    },
    #[error("entity view membership version conflict in {namespace}/{view}/{local_id}")]
    EntityViewMembershipVersionConflict {
        namespace: RiseStoreNamespace,
        view: String,
        local_id: String,
    },
    #[error("entity view revision conflict in {namespace}/{view}")]
    EntityViewRevisionConflict {
        namespace: RiseStoreNamespace,
        view: String,
    },
    #[error("entity merge precondition failed: {survivor_local_id} <- {duplicate_local_id}")]
    EntityMergePreconditionFailed {
        namespace: RiseStoreNamespace,
        survivor_local_id: String,
        duplicate_local_id: String,
    },
    #[error("version precondition failed in {namespace}/{key}")]
    VersionPreconditionFailed {
        namespace: RiseStoreNamespace,
        key: String,
    },
    #[error("duplicate operation {0}")]
    DuplicateOperation(RiseRequestID),
    #[error("observer limit reached ({0})")]
    ObserverLimitReached(i64),
    #[error("unsupported schema {found}, supported {supported}")]
    UnsupportedSchema { found: i32, supported: i32 },
    #[error("invalid encryption key")]
    InvalidEncryptionKey,
    #[error("invalid identifier {0}")]
    InvalidIdentifier(String),
    #[error("corrupted value {0}")]
    CorruptedValue(String),
    #[error("sqlite error {code} in {operation}")]
    Sqlite { code: i32, operation: String },
    #[error(transparent)]
    Cipher(#[from] RiseAESGCMStoreCipherError),
}
