use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_engine::RiseEngine;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue,
    RiseStoredValueWritePolicy,
};
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::comment::engine::rise_comment_engine_models::{
    RiseCommentCacheSource, RiseCommentRecord, RiseCommentScope,
};
use crate::modules::comment::stores::comment_actions::comment_types::GetCommentsResponse;

static NAMESPACE: LazyLock<RiseStoreNamespace> = LazyLock::new(|| RiseStoreNamespace::new("comment.resources.v1"));
const CURRENT_SCHEMA_VERSION: i64 = 1;
static PERSISTED_LISTS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::client_value("comment_lists_persisted")).unwrap_or(0));
static PERSISTED_ITEMS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::client_value("comment_list_persisted_items")).unwrap_or(0));

#[derive(Clone, Debug)]
pub struct RiseCommentResourceSnapshot {
    pub records: Vec<RiseCommentRecord>,
    pub has_more: bool,
    pub source: RiseCommentCacheSource,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseCommentResourceRepositoryEnvelope {
    schema_version: i64,
    has_more: bool,
    records: Vec<RiseCommentRecord>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct RiseCommentResourceRepositoryIndex {
    keys: Vec<String>,
}

struct RiseCommentResourceRepositoryState {
    index: Option<RiseCommentResourceRepositoryIndex>,
    version_clock: i64,
    is_shutdown: bool,
}

/// heads of comment lists in the encrypted store so a cold launch shows last seen comments
/// not a skeleton
/// bounded twice, each list keeps its head and the list count is capped
pub struct RiseCommentResourceRepository {
    engine: Arc<RiseEngine>,
    index_key: String,
    key_prefix: String,
    state: RiseIsolated<RiseCommentResourceRepositoryState>,
}

impl RiseCommentResourceRepository {
    pub fn new(engine: Arc<RiseEngine>, account_id: &str) -> Self {
        let key_prefix = format!("{}:{}|", account_id.len(), account_id);
        Self {
            engine,
            index_key: format!("{key_prefix}index"),
            key_prefix,
            state: RiseIsolated::new(RiseCommentResourceRepositoryState {
                index: None,
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached(&self, scope: &RiseCommentScope) -> Option<RiseCommentResourceSnapshot> {
        if self.is_shutdown() || !scope.is_persisted() {
            return None;
        }
        let stored = self.engine.value(NAMESPACE.clone(), self.resource_key(scope)).await.ok().flatten()?;
        let is_running = self.state.with(|state| {
            if state.is_shutdown {
                return false;
            }
            state.version_clock = state.version_clock.max(stored.version);
            true
        });
        if !is_running {
            return None;
        }

        let data = stored.data;
        let envelope = RiseAccountRuntime::current()
            .spawn_blocking(move || serde_json::from_slice::<RiseCommentResourceRepositoryEnvelope>(&data).ok())
            .await
            .ok()
            .flatten()?;
        if envelope.schema_version != CURRENT_SCHEMA_VERSION || envelope.records.is_empty() {
            return None;
        }
        Some(RiseCommentResourceSnapshot {
            records: envelope.records,
            has_more: envelope.has_more,
            source: RiseCommentCacheSource::PersistentStore,
        })
    }

    pub async fn store(&self, scope: &RiseCommentScope, comments: &[GetCommentsResponse], has_more: bool) {
        if self.is_shutdown() || !scope.is_persisted() {
            return;
        }
        let records: Vec<RiseCommentRecord> = comments
            .iter()
            .filter(|comment| !comment.is_temp)
            .take(*PERSISTED_ITEMS)
            .map(RiseCommentRecord::new)
            .collect();
        if records.is_empty() {
            self.remove(scope).await;
            return;
        }
        let persisted_has_more = has_more || comments.len() > records.len();
        let envelope = RiseCommentResourceRepositoryEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            has_more: persisted_has_more,
            records,
        };
        let Some(data) = Self::encode(envelope).await else {
            return;
        };
        if self.is_shutdown() {
            return;
        }

        let key = self.resource_key(scope);
        let mut next_index = self.load_index().await;
        next_index.keys.retain(|existing| *existing != key);
        next_index.keys.push(key.clone());
        let overflow = next_index.keys.len().saturating_sub(*PERSISTED_LISTS);
        let evicted: Vec<String> = next_index.keys.drain(..overflow).collect();
        let Some(index_data) = Self::encode(next_index.clone()).await else {
            return;
        };
        if self.is_shutdown() {
            return;
        }

        let now = Self::now_milliseconds();
        let (data_version, index_version) = self.state.with(|state| (Self::next_version(state), Self::next_version(state)));
        let mut mutations = vec![
            Self::set_value(key, data, data_version, now),
            Self::set_value(self.index_key.clone(), index_data, index_version, now),
        ];
        mutations.extend(evicted.into_iter().map(Self::remove_value));
        if self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await.is_err() {
            return;
        }
        self.state.with(|state| state.index = Some(next_index));
    }

    pub async fn remove(&self, scope: &RiseCommentScope) {
        if self.is_shutdown() || !scope.is_persisted() {
            return;
        }
        let key = self.resource_key(scope);
        let mut next_index = self.load_index().await;
        if !next_index.keys.contains(&key) {
            return;
        }
        next_index.keys.retain(|existing| *existing != key);
        let Some(index_data) = Self::encode(next_index.clone()).await else {
            return;
        };
        if self.is_shutdown() {
            return;
        }
        let version = self.state.with(Self::next_version);
        let batch = RiseStoreWriteBatch::new(
            vec![
                Self::remove_value(key),
                Self::set_value(self.index_key.clone(), index_data, version, Self::now_milliseconds()),
            ],
            HashSet::new(),
        );
        if self.engine.write(batch).await.is_err() {
            return;
        }
        self.state.with(|state| state.index = Some(next_index));
    }

    pub fn shutdown(&self) {
        self.state.with(|state| {
            state.is_shutdown = true;
            state.index = None;
        });
    }

    fn is_shutdown(&self) -> bool {
        self.state.with(|state| state.is_shutdown)
    }

    async fn load_index(&self) -> RiseCommentResourceRepositoryIndex {
        if let Some(index) = self.state.with(|state| state.index.clone()) {
            return index;
        }
        let Some(stored) = self.engine.value(NAMESPACE.clone(), self.index_key.clone()).await.ok().flatten() else {
            return RiseCommentResourceRepositoryIndex::default();
        };
        let decoded: RiseCommentResourceRepositoryIndex = serde_json::from_slice(&stored.data).unwrap_or_default();
        self.state.with(|state| {
            state.version_clock = state.version_clock.max(stored.version);
            state.index = Some(decoded.clone());
        });
        decoded
    }

    fn resource_key(&self, scope: &RiseCommentScope) -> String {
        format!("{}{}", self.key_prefix, scope.storage_key())
    }

    fn next_version(state: &mut RiseCommentResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn set_value(key: String, data: Vec<u8>, version: i64, now: i64) -> RiseStoreMutation {
        RiseStoreMutation::SetValue {
            namespace: NAMESPACE.clone(),
            key,
            value: RiseStoredValue::new(data, version, now),
            precondition: RiseStoreVersionPrecondition::Any,
            policy: RiseStoredValueWritePolicy::ServerAuthoritative,
        }
    }

    fn remove_value(key: String) -> RiseStoreMutation {
        RiseStoreMutation::RemoveValue {
            namespace: NAMESPACE.clone(),
            key,
            precondition: RiseStoreVersionPrecondition::Any,
        }
    }

    async fn encode<Value: Serialize + Send + 'static>(value: Value) -> Option<Vec<u8>> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let mut tree = serde_json::to_value(&value).ok()?;
                tree.sort_all_objects();
                serde_json::to_vec(&tree).ok()
            })
            .await
            .ok()
            .flatten()
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
