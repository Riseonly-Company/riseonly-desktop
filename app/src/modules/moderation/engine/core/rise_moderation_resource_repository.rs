use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue,
    RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseModerationCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

#[derive(Clone, Debug)]
pub struct RiseModerationCached<Value> {
    pub value: Value,
    pub revision: i64,
    pub source: RiseModerationCacheSource,
}

static NAMESPACE: LazyLock<RiseStoreNamespace> = LazyLock::new(|| RiseStoreNamespace::new("moderation.resources.v1"));

struct RiseModerationResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
    access: u64,
}

struct RiseModerationResourceRepositoryState {
    memory: HashMap<String, RiseModerationResourceRepositoryMemoryEntry>,
    memory_bytes: usize,
    access_clock: u64,
    version_clock: i64,
    is_shutdown: bool,
}

/// bounded memory cache over the encrypted store for moderation resources
/// keyed by account so a second account never reads the first ones queue
/// every miss falls through to the store so eviction is safe
pub struct RiseModerationResourceRepository {
    engine: Arc<RiseEngine>,
    account_id: String,
    maximum_memory_entries: usize,
    maximum_memory_bytes: usize,
    state: RiseIsolated<RiseModerationResourceRepositoryState>,
}

impl RiseModerationResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_ENTRIES: usize = 64;
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 6 * 1_024 * 1_024;

    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Self {
        Self::with_limits(
            engine,
            account_id,
            Self::DEFAULT_MAXIMUM_MEMORY_ENTRIES,
            Self::DEFAULT_MAXIMUM_MEMORY_BYTES,
        )
    }

    pub fn with_limits(
        engine: Arc<RiseEngine>,
        account_id: String,
        maximum_memory_entries: usize,
        maximum_memory_bytes: usize,
    ) -> Self {
        assert!(maximum_memory_entries > 0);
        assert!(maximum_memory_bytes > 0);
        Self {
            engine,
            account_id,
            maximum_memory_entries,
            maximum_memory_bytes,
            state: RiseIsolated::new(RiseModerationResourceRepositoryState {
                memory: HashMap::new(),
                memory_bytes: 0,
                access_clock: 0,
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached<Value>(&self, name: &str, params: &[String]) -> anyhow::Result<Option<RiseModerationCached<Value>>>
    where
        Value: DeserializeOwned + Send + 'static,
    {
        let key = self.make_key(name, params);
        let memory = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            if !s.memory.contains_key(&key) {
                return Ok(None);
            }
            s.access_clock = s.access_clock.wrapping_add(1);
            let access_clock = s.access_clock;
            Ok(s.memory.get_mut(&key).map(|entry| {
                entry.access = access_clock;
                (entry.data.clone(), entry.revision)
            }))
        })?;

        if let Some((data, revision)) = memory {
            let value = Self::decode::<Value>(data).await?;
            return Ok(Some(RiseModerationCached {
                value,
                revision,
                source: RiseModerationCacheSource::Memory,
            }));
        }

        let Some(stored) = self.engine.value(NAMESPACE.clone(), key.clone()).await? else {
            return Ok(None);
        };
        self.ensure_running()?;
        let data = Arc::new(stored.data);
        let value = Self::decode::<Value>(data.clone()).await?;
        self.state.with(|s| {
            s.version_clock = s.version_clock.max(stored.version);
            Self::install_memory(s, self.maximum_memory_entries, self.maximum_memory_bytes, data, stored.version, &key);
        });
        Ok(Some(RiseModerationCached {
            value,
            revision: stored.version,
            source: RiseModerationCacheSource::PersistentStore,
        }))
    }

    pub async fn store<Value>(&self, value: &Value, name: &str, params: &[String]) -> anyhow::Result<i64>
    where
        Value: Serialize + Clone + Send + 'static,
    {
        self.ensure_running()?;
        let key = self.make_key(name, params);
        let data = Arc::new(Self::encode(value.clone()).await?);
        let version = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            let version = Self::next_version(s);
            Self::install_memory(s, self.maximum_memory_entries, self.maximum_memory_bytes, data.clone(), version, &key);
            Ok(version)
        })?;
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: NAMESPACE.clone(),
                    key,
                    value: RiseStoredValue::new(data.as_ref().clone(), version, Self::now_milliseconds()),
                    precondition: RiseStoreVersionPrecondition::Any,
                    policy: RiseStoredValueWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await?;
        Ok(version)
    }

    pub async fn remove(&self, name: &str, params: &[String]) {
        let key = self.make_key(name, params);
        self.state.with(|s| {
            if let Some(entry) = s.memory.remove(&key) {
                s.memory_bytes = s.memory_bytes.saturating_sub(entry.data.len());
            }
        });
        let _ = self
            .engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::RemoveValue {
                    namespace: NAMESPACE.clone(),
                    key,
                    precondition: RiseStoreVersionPrecondition::Any,
                }],
                HashSet::new(),
            ))
            .await;
    }

    /// drops every memory entry under a name
    /// persisted copies stay and get overwritten by the next refresh
    pub fn forget_memory(&self, name: &str) {
        let bare = self.make_key(name, &[]);
        let prefix = format!("{bare}|");
        self.state.with(|s| {
            let doomed: Vec<String> = s
                .memory
                .keys()
                .filter(|key| key.starts_with(&prefix) || **key == bare)
                .cloned()
                .collect();
            for key in doomed {
                if let Some(entry) = s.memory.remove(&key) {
                    s.memory_bytes = s.memory_bytes.saturating_sub(entry.data.len());
                }
            }
        });
    }

    pub fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.memory = HashMap::new();
            s.memory_bytes = 0;
        });
    }

    fn ensure_running(&self) -> Result<(), CancellationError> {
        if self.state.with(|s| s.is_shutdown) { Err(CancellationError) } else { Ok(()) }
    }

    fn make_key(&self, name: &str, params: &[String]) -> String {
        [self.account_id.as_str(), name]
            .into_iter()
            .chain(params.iter().map(String::as_str))
            .map(|component| format!("{}:{}", component.len(), component))
            .collect::<Vec<_>>()
            .join("|")
    }

    fn next_version(state: &mut RiseModerationResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        state: &mut RiseModerationResourceRepositoryState,
        maximum_memory_entries: usize,
        maximum_memory_bytes: usize,
        data: Arc<Vec<u8>>,
        revision: i64,
        key: &str,
    ) {
        if let Some(previous) = state.memory.get(key) {
            state.memory_bytes = state.memory_bytes.saturating_sub(previous.data.len());
        }
        state.access_clock = state.access_clock.wrapping_add(1);
        state.memory_bytes += data.len();
        state.memory.insert(
            key.to_owned(),
            RiseModerationResourceRepositoryMemoryEntry {
                data,
                revision,
                access: state.access_clock,
            },
        );
        while state.memory.len() > maximum_memory_entries || state.memory_bytes > maximum_memory_bytes {
            let Some(victim) = state
                .memory
                .iter()
                .filter(|(candidate, _)| candidate.as_str() != key)
                .min_by_key(|(_, entry)| entry.access)
                .map(|(candidate, _)| candidate.clone())
            else {
                break;
            };
            if let Some(entry) = state.memory.remove(&victim) {
                state.memory_bytes = state.memory_bytes.saturating_sub(entry.data.len());
            }
        }
    }

    /// coding runs off the callers task
    async fn encode<Value>(value: Value) -> anyhow::Result<Vec<u8>>
    where
        Value: Serialize + Send + 'static,
    {
        let encoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                let mut value = serde_json::to_value(&value)?;
                value.sort_all_objects();
                Ok(serde_json::to_vec(&value)?)
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

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
