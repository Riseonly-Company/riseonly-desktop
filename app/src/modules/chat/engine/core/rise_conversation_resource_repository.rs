use std::borrow::Borrow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue,
    RiseStoredValueWritePolicy,
};
use rise_engine::{RiseEngine, RiseJSONValue};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpoint, RiseConversationRPC};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseConversationResourceKeyPersistence {
    Persistent,
    MemoryOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseConversationResourceKey {
    pub key: String,
    pub persistence: RiseConversationResourceKeyPersistence,
}

impl RiseConversationResourceKey {
    /// new key from components, persistent by default
    pub fn new<Component: AsRef<str>>(components: impl IntoIterator<Item = Component>) -> Self {
        Self::with_persistence(components, RiseConversationResourceKeyPersistence::Persistent)
    }

    pub fn with_persistence<Component: AsRef<str>>(
        components: impl IntoIterator<Item = Component>,
        persistence: RiseConversationResourceKeyPersistence,
    ) -> Self {
        let encoded: Vec<String> = components.into_iter().map(|component| Self::encode(component.as_ref())).collect();
        assert!(!encoded.is_empty());
        Self { key: encoded.join("|"), persistence }
    }

    fn encode(component: &str) -> String {
        format!("{}:{component}", component.len())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseConversationResourceReadSource {
    Memory,
    PersistentStore,
    Miss,
}

impl RiseConversationResourceReadSource {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::PersistentStore => "persistent-store",
            Self::Miss => "miss",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RiseConversationResourceSnapshot<Value> {
    pub value: Option<Value>,
    pub revision: i64,
    pub is_persisted: bool,
    pub read_source: RiseConversationResourceReadSource,
}

static NAMESPACE: LazyLock<RiseStoreNamespace> = LazyLock::new(|| RiseStoreNamespace::new("conversation.resources.v1"));

struct RiseConversationResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
    cost: usize,
    access: u64,
}

#[derive(Default)]
struct RiseConversationResourceRepositoryState {
    memory: HashMap<RiseConversationResourceKey, RiseConversationResourceRepositoryMemoryEntry>,
    memory_bytes: usize,
    access_clock: u64,
    version_clock: i64,
    request_generation: HashMap<RiseConversationResourceKey, u64>,
}

/// encrypted repo for bounded low frequency resources
/// settings and folders and roles and members and invites and analytics
/// chat rows and timelines use normalized views instead
pub struct RiseConversationResourceRepository {
    engine: Arc<RiseEngine>,
    rpc: RiseConversationRPC,
    maximum_memory_entries: usize,
    maximum_memory_bytes: usize,
    state: RiseIsolated<RiseConversationResourceRepositoryState>,
}

impl RiseConversationResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_ENTRIES: usize = 128;
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 16 * 1_024 * 1_024;

    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self::with_memory_limits(engine, Self::DEFAULT_MAXIMUM_MEMORY_ENTRIES, Self::DEFAULT_MAXIMUM_MEMORY_BYTES)
    }

    pub fn with_memory_limits(
        engine: Arc<RiseEngine>,
        maximum_memory_entries: usize,
        maximum_memory_bytes: usize,
    ) -> Self {
        assert!(maximum_memory_entries > 0);
        assert!(maximum_memory_bytes > 0);
        Self {
            rpc: RiseConversationRPC::new(engine.clone()),
            engine,
            maximum_memory_entries,
            maximum_memory_bytes,
            state: RiseIsolated::new(RiseConversationResourceRepositoryState::default()),
        }
    }

    pub async fn cached<Value>(
        &self,
        key: &RiseConversationResourceKey,
    ) -> anyhow::Result<RiseConversationResourceSnapshot<Value>>
    where
        Value: DeserializeOwned + Send + 'static,
    {
        let memory = self.state.with(|state| {
            let data_and_revision =
                state.memory.get(key).map(|entry| (entry.data.clone(), entry.revision))?;
            Self::touch(state, key);
            Some(data_and_revision)
        });
        if let Some((data, revision)) = memory {
            return Ok(RiseConversationResourceSnapshot {
                value: Some(Self::decode::<Value>(data).await?),
                revision,
                is_persisted: key.persistence == RiseConversationResourceKeyPersistence::Persistent,
                read_source: RiseConversationResourceReadSource::Memory,
            });
        }

        let stored = if key.persistence == RiseConversationResourceKeyPersistence::Persistent {
            self.engine.value(NAMESPACE.clone(), key.key.clone()).await?
        } else {
            None
        };
        let Some(stored) = stored else {
            return Ok(RiseConversationResourceSnapshot {
                value: None,
                revision: 0,
                is_persisted: false,
                read_source: RiseConversationResourceReadSource::Miss,
            });
        };
        let data = Arc::new(stored.data);
        self.state.with(|state| {
            self.install_memory(state, data.clone(), stored.version, key);
            state.version_clock = state.version_clock.max(stored.version);
        });
        Ok(RiseConversationResourceSnapshot {
            value: Some(Self::decode::<Value>(data).await?),
            revision: stored.version,
            is_persisted: true,
            read_source: RiseConversationResourceReadSource::PersistentStore,
        })
    }

    /// persists only if this is still the newest request for the resource
    /// the decoded response comes back even if a newer one won, so callers finish their own lifecycle
    pub async fn refresh<Request, Response>(
        &self,
        key: &RiseConversationResourceKey,
        endpoint: impl Borrow<RiseConversationEndpoint> + Send,
        request: Request,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Send,
        Response: Serialize + DeserializeOwned + Send + 'static,
    {
        self.refresh_body(key, endpoint, RiseJSONValue::encode(&request)?).await
    }

    pub async fn refresh_body<Response>(
        &self,
        key: &RiseConversationResourceKey,
        endpoint: impl Borrow<RiseConversationEndpoint> + Send,
        body: RiseJSONValue,
    ) -> anyhow::Result<Response>
    where
        Response: Serialize + DeserializeOwned + Send + 'static,
    {
        let generation = self.next_request_generation(key);
        let value: Response = self.rpc.call(endpoint, body).await?;
        if !self.is_newest_request(key, generation) {
            return Ok(value);
        }
        let (data, value) = Self::encode(value).await?;
        self.store_encoded(data, key).await?;
        Ok(value)
    }

    pub async fn store<Value>(&self, value: &Value, key: &RiseConversationResourceKey) -> anyhow::Result<i64>
    where
        Value: Serialize + Clone + Send + Sync + 'static,
    {
        let (data, _) = Self::encode(value.clone()).await?;
        self.store_encoded(data, key).await
    }

    async fn store_encoded(&self, data: Vec<u8>, key: &RiseConversationResourceKey) -> anyhow::Result<i64> {
        let data = Arc::new(data);
        let version = self.state.with(|state| {
            let version = Self::next_version(state);
            self.install_memory(state, data.clone(), version, key);
            version
        });
        if key.persistence != RiseConversationResourceKeyPersistence::Persistent {
            return Ok(version);
        }

        self.engine.write(Self::set_value_batch(key, data, version)).await?;
        Ok(version)
    }

    /// replaces the cached value in memory and returns the persistent mutation without committing
    /// so a realtime reducer can update a derived resource in the same transaction
    pub async fn replacement_batch<Value>(
        &self,
        value: &Value,
        key: &RiseConversationResourceKey,
    ) -> anyhow::Result<RiseStoreWriteBatch>
    where
        Value: Serialize + Clone + Send + Sync + 'static,
    {
        let (data, _) = Self::encode(value.clone()).await?;
        let data = Arc::new(data);
        let version = self.state.with(|state| {
            let version = Self::next_version(state);
            let generation = state.request_generation.entry(key.clone()).or_insert(0);
            *generation = generation.wrapping_add(1);
            self.install_memory(state, data.clone(), version, key);
            version
        });
        if key.persistence != RiseConversationResourceKeyPersistence::Persistent {
            return Ok(RiseStoreWriteBatch::empty());
        }
        Ok(Self::set_value_batch(key, data, version))
    }

    pub async fn remove(&self, key: &RiseConversationResourceKey) -> anyhow::Result<()> {
        let batch = self.removal_batch(&HashSet::from([key.clone()])).await;
        if batch.mutations.is_empty() {
            return Ok(());
        }
        self.engine.write(batch).await?;
        Ok(())
    }

    /// evicts memory entries now and returns one store batch for all matching persistent values
    /// so a realtime reducer can fold the invalidation in its transaction
    pub async fn removal_batch(&self, keys: &HashSet<RiseConversationResourceKey>) -> RiseStoreWriteBatch {
        if keys.is_empty() {
            return RiseStoreWriteBatch::empty();
        }
        let persistent_keys: BTreeSet<String> = self.state.with(|state| {
            let mut persistent_keys = BTreeSet::new();
            for key in keys {
                if let Some(entry) = state.memory.remove(key) {
                    state.memory_bytes = state.memory_bytes.saturating_sub(entry.cost);
                }
                let generation = state.request_generation.entry(key.clone()).or_insert(0);
                *generation = generation.wrapping_add(1);
                if key.persistence == RiseConversationResourceKeyPersistence::Persistent {
                    persistent_keys.insert(key.key.clone());
                }
            }
            persistent_keys
        });
        RiseStoreWriteBatch::new(
            persistent_keys
                .into_iter()
                .map(|key| RiseStoreMutation::RemoveValue {
                    namespace: NAMESPACE.clone(),
                    key,
                    precondition: RiseStoreVersionPrecondition::Any,
                })
                .collect(),
            HashSet::new(),
        )
    }

    pub async fn invalidate_requests(&self) {
        self.state.with(|state| {
            for generation in state.request_generation.values_mut() {
                *generation = generation.wrapping_add(1);
            }
        });
    }

    pub async fn clear_memory(&self) {
        self.state.with(|state| {
            state.memory = HashMap::new();
            state.memory_bytes = 0;
        });
    }

    fn set_value_batch(key: &RiseConversationResourceKey, data: Arc<Vec<u8>>, version: i64) -> RiseStoreWriteBatch {
        RiseStoreWriteBatch::new(
            vec![RiseStoreMutation::SetValue {
                namespace: NAMESPACE.clone(),
                key: key.key.clone(),
                value: RiseStoredValue::new(
                    Arc::unwrap_or_clone(data),
                    version,
                    Self::now_milliseconds(),
                ),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            }],
            HashSet::new(),
        )
    }

    fn next_request_generation(&self, key: &RiseConversationResourceKey) -> u64 {
        self.state.with(|state| {
            let generation = state.request_generation.entry(key.clone()).or_insert(0);
            *generation = generation.wrapping_add(1);
            *generation
        })
    }

    fn is_newest_request(&self, key: &RiseConversationResourceKey, generation: u64) -> bool {
        self.state.with(|state| state.request_generation.get(key) == Some(&generation))
    }

    fn next_version(state: &mut RiseConversationResourceRepositoryState) -> i64 {
        let wall = Self::now_milliseconds();
        state.version_clock = (state.version_clock + 1).max(wall);
        state.version_clock
    }

    async fn encode<Value>(value: Value) -> anyhow::Result<(Vec<u8>, Value)>
    where
        Value: Serialize + Send + 'static,
    {
        let encoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<(Vec<u8>, Value)> {
                let mut sorted = serde_json::to_value(&value)?;
                sorted.sort_all_objects();
                Ok((serde_json::to_vec(&sorted)?, value))
            })
            .await??;
        Ok(encoded)
    }

    async fn decode<Value>(data: Arc<Vec<u8>>) -> anyhow::Result<Value>
    where
        Value: DeserializeOwned + Send + 'static,
    {
        let decoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Value> { Ok(serde_json::from_slice(&data)?) })
            .await??;
        Ok(decoded)
    }

    fn install_memory(
        &self,
        state: &mut RiseConversationResourceRepositoryState,
        data: Arc<Vec<u8>>,
        revision: i64,
        key: &RiseConversationResourceKey,
    ) {
        if let Some(previous) = state.memory.get(key) {
            state.memory_bytes = state.memory_bytes.saturating_sub(previous.cost);
        }
        state.access_clock = state.access_clock.wrapping_add(1);
        let cost = data.len();
        state.memory.insert(
            key.clone(),
            RiseConversationResourceRepositoryMemoryEntry { data, revision, cost, access: state.access_clock },
        );
        state.memory_bytes += cost;
        self.evict_memory_if_needed(state, key);
    }

    fn touch(state: &mut RiseConversationResourceRepositoryState, key: &RiseConversationResourceKey) {
        state.access_clock = state.access_clock.wrapping_add(1);
        let access_clock = state.access_clock;
        if let Some(entry) = state.memory.get_mut(key) {
            entry.access = access_clock;
        }
    }

    fn evict_memory_if_needed(
        &self,
        state: &mut RiseConversationResourceRepositoryState,
        protected_key: &RiseConversationResourceKey,
    ) {
        while state.memory.len() > self.maximum_memory_entries || state.memory_bytes > self.maximum_memory_bytes {
            let Some(victim) = state
                .memory
                .iter()
                .filter(|(key, _)| *key != protected_key)
                .min_by_key(|(_, entry)| entry.access)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(entry) = state.memory.remove(&victim) {
                state.memory_bytes = state.memory_bytes.saturating_sub(entry.cost);
            }
        }
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
