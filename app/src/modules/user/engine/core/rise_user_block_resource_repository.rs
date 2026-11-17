use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch,
    RiseStoredValue, RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("user.resources.v1"));
const CURRENT_SCHEMA_VERSION: i64 = 1;
const BLOCKED_USERS_COMPONENT: &str = "blocked-users";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUserBlockCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

impl RiseUserBlockCacheSource {
    pub const ALL_CASES: [Self; 4] = [Self::Miss, Self::Memory, Self::PersistentStore, Self::Network];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Miss => "miss",
            Self::Memory => "memory",
            Self::PersistentStore => "persistent-store",
            Self::Network => "network",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|source| source.raw_value() == raw)
    }
}

/// persisted row,
/// separate from RiseBlockedUser so the presentation model can change without a schema bump
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct RiseUserBlockPersistedEntry {
    #[serde(rename = "userID")]
    pub user_id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
    #[serde(rename = "isPremium")]
    pub is_premium: bool,
    #[serde(rename = "isDeleted")]
    pub is_deleted: bool,
    pub scope: RiseBlockScope,
    #[serde(rename = "blockedAtMs")]
    pub blocked_at_ms: i64,
}

impl Default for RiseUserBlockPersistedEntry {
    fn default() -> Self {
        Self {
            user_id: String::new(),
            name: String::new(),
            tag: String::new(),
            logo: String::new(),
            is_premium: false,
            is_deleted: false,
            scope: RiseBlockScope::Standard,
            blocked_at_ms: 0,
        }
    }
}

#[derive(Deserialize)]
struct StoredRiseUserBlockPersistedEntry {
    #[serde(default, rename = "userID")]
    user_id: Option<serde_json::Value>,
    #[serde(default)]
    name: Option<serde_json::Value>,
    #[serde(default)]
    tag: Option<serde_json::Value>,
    #[serde(default)]
    logo: Option<serde_json::Value>,
    #[serde(default, rename = "isPremium")]
    is_premium: Option<serde_json::Value>,
    #[serde(default, rename = "isDeleted")]
    is_deleted: Option<serde_json::Value>,
    #[serde(default)]
    scope: Option<serde_json::Value>,
    #[serde(default, rename = "blockedAtMs")]
    blocked_at_ms: Option<serde_json::Value>,
}

impl<'de> Deserialize<'de> for RiseUserBlockPersistedEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let c = StoredRiseUserBlockPersistedEntry::deserialize(deserializer)?;
        Ok(Self {
            user_id: lenient(c.user_id).unwrap_or_default(),
            name: lenient(c.name).unwrap_or_default(),
            tag: lenient(c.tag).unwrap_or_default(),
            logo: lenient(c.logo).unwrap_or_default(),
            is_premium: lenient(c.is_premium).unwrap_or(false),
            is_deleted: lenient(c.is_deleted).unwrap_or(false),
            scope: lenient(c.scope).unwrap_or(RiseBlockScope::Standard),
            blocked_at_ms: lenient(c.blocked_at_ms).unwrap_or(0),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseUserBlockResourcePayload {
    pub entries: Vec<RiseUserBlockPersistedEntry>,
    pub scopes: HashMap<String, RiseBlockScope>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

impl RiseUserBlockResourcePayload {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserBlockResourceSnapshot {
    pub payload: RiseUserBlockResourcePayload,
    pub revision: i64,
    pub source: RiseUserBlockCacheSource,
}

struct RiseUserBlockResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RiseUserBlockResourceRepositoryEnvelope {
    schema_version: i64,
    entries: Vec<RiseUserBlockPersistedEntry>,
    scopes: HashMap<String, RiseBlockScope>,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseUserBlockResourceRepositoryStoredEnvelope {
    #[serde(default)]
    schema_version: Option<serde_json::Value>,
    #[serde(default)]
    entries: Option<serde_json::Value>,
    #[serde(default)]
    scopes: Option<serde_json::Value>,
    #[serde(default)]
    has_more: Option<serde_json::Value>,
    #[serde(default)]
    next_cursor: Option<serde_json::Value>,
}

impl RiseUserBlockResourceRepositoryStoredEnvelope {
    fn envelope(self) -> RiseUserBlockResourceRepositoryEnvelope {
        RiseUserBlockResourceRepositoryEnvelope {
            schema_version: lenient(self.schema_version).unwrap_or(0),
            entries: lenient(self.entries).unwrap_or_default(),
            scopes: lenient(self.scopes).unwrap_or_default(),
            has_more: lenient(self.has_more).unwrap_or(false),
            next_cursor: lenient(self.next_cursor),
        }
    }
}

struct RiseUserBlockResourceRepositoryState {
    memory_entry: Option<RiseUserBlockResourceRepositoryMemoryEntry>,
    version_clock: i64,
    is_shutdown: bool,
}

/// bounded encrypted store for the accounts own block list,
/// one key per account and never another accounts list
pub struct RiseUserBlockResourceRepository {
    engine: Arc<RiseEngine>,
    maximum_memory_bytes: usize,
    resource_key: String,
    debug_resource_key: String,
    state: RiseIsolated<RiseUserBlockResourceRepositoryState>,
}

impl RiseUserBlockResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 64 * 1_024;

    pub fn new(engine: Arc<RiseEngine>, account_id: &str) -> Self {
        Self::with_maximum_memory_bytes(engine, account_id, Self::DEFAULT_MAXIMUM_MEMORY_BYTES)
    }

    pub fn with_maximum_memory_bytes(
        engine: Arc<RiseEngine>,
        account_id: &str,
        maximum_memory_bytes: usize,
    ) -> Self {
        assert!(maximum_memory_bytes > 0);
        let resource_key = Self::make_key(&[account_id, BLOCKED_USERS_COMPONENT]);
        Self {
            engine,
            maximum_memory_bytes,
            debug_resource_key: resource_key.clone(),
            resource_key,
            state: RiseIsolated::new(RiseUserBlockResourceRepositoryState {
                memory_entry: None,
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    /// key used in debug output
    pub fn debug_resource_key(&self) -> &str {
        &self.debug_resource_key
    }

    pub async fn cached(&self) -> anyhow::Result<RiseUserBlockResourceSnapshot> {
        let memory = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(s.memory_entry
                .as_ref()
                .map(|entry| (entry.data.clone(), entry.revision)))
        })?;

        if let Some((data, revision)) = memory {
            let payload = Self::decode(data).await?;
            self.ensure_running()?;
            return Ok(RiseUserBlockResourceSnapshot {
                payload,
                revision,
                source: RiseUserBlockCacheSource::Memory,
            });
        }

        let stored = self
            .engine
            .value(NAMESPACE.clone(), self.resource_key.clone())
            .await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(RiseUserBlockResourceSnapshot {
                payload: RiseUserBlockResourcePayload::empty(),
                revision: 0,
                source: RiseUserBlockCacheSource::Miss,
            });
        };

        let data = Arc::new(stored.data);
        let payload = Self::decode(data.clone()).await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            s.version_clock = s.version_clock.max(stored.version);
            Self::install_memory(s, self.maximum_memory_bytes, data, stored.version);
            Ok(())
        })?;
        Ok(RiseUserBlockResourceSnapshot {
            payload,
            revision: stored.version,
            source: RiseUserBlockCacheSource::PersistentStore,
        })
    }

    pub async fn store(&self, payload: &RiseUserBlockResourcePayload) -> anyhow::Result<i64> {
        let version = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(Self::next_version(s))
        })?;
        let data = Arc::new(Self::encode(payload.clone()).await?);
        self.ensure_running()?;

        self.engine
            .write(Self::set_value_batch(&self.resource_key, data.as_ref().clone(), version))
            .await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Self::install_memory(s, self.maximum_memory_bytes, data, version);
            Ok(())
        })?;
        Ok(version)
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.memory_entry = None;
        });
    }

    fn ensure_running(&self) -> Result<(), CancellationError> {
        if self.state.with(|s| s.is_shutdown) {
            Err(CancellationError)
        } else {
            Ok(())
        }
    }

    /// monotonic version floored at wall clock, makes a write from a superseded request a no-op
    fn next_version(state: &mut RiseUserBlockResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        state: &mut RiseUserBlockResourceRepositoryState,
        maximum_memory_bytes: usize,
        data: Arc<Vec<u8>>,
        revision: i64,
    ) {
        let newest = state.memory_entry.as_ref().map_or(0, |entry| entry.revision);
        if data.len() > maximum_memory_bytes || revision < newest {
            return;
        }
        state.memory_entry = Some(RiseUserBlockResourceRepositoryMemoryEntry { data, revision });
    }

    fn set_value_batch(key: &str, data: Vec<u8>, version: i64) -> RiseStoreWriteBatch {
        RiseStoreWriteBatch::new(
            vec![RiseStoreMutation::SetValue {
                namespace: NAMESPACE.clone(),
                key: key.to_owned(),
                value: RiseStoredValue::new(data, version, Self::now_milliseconds()),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            }],
            HashSet::new(),
        )
    }

    /// encoding runs off the callers task, privacy screen open path mustnt block the first frame
    async fn encode(payload: RiseUserBlockResourcePayload) -> anyhow::Result<Vec<u8>> {
        let envelope = RiseUserBlockResourceRepositoryEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            entries: payload.entries,
            scopes: payload.scopes,
            has_more: payload.has_more,
            next_cursor: payload.next_cursor,
        };
        let encoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                let mut value = serde_json::to_value(&envelope)?;
                value.sort_all_objects();
                Ok(serde_json::to_vec(&value)?)
            })
            .await??;
        Ok(encoded)
    }

    /// schema mismatch gives an empty cache not an error, the authoritative refresh refills it
    async fn decode(data: Arc<Vec<u8>>) -> anyhow::Result<RiseUserBlockResourcePayload> {
        let decoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<RiseUserBlockResourcePayload> {
                let envelope =
                    serde_json::from_slice::<RiseUserBlockResourceRepositoryStoredEnvelope>(&data)?.envelope();
                if envelope.schema_version != CURRENT_SCHEMA_VERSION {
                    return Ok(RiseUserBlockResourcePayload::empty());
                }
                Ok(RiseUserBlockResourcePayload {
                    entries: envelope.entries,
                    scopes: envelope.scopes,
                    has_more: envelope.has_more,
                    next_cursor: envelope.next_cursor,
                })
            })
            .await??;
        Ok(decoded)
    }

    fn make_key(components: &[&str]) -> String {
        components
            .iter()
            .map(|component| format!("{}:{}", component.len(), component))
            .collect::<Vec<_>>()
            .join("|")
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}

fn lenient<T: DeserializeOwned>(field: Option<serde_json::Value>) -> Option<T> {
    serde_json::from_value(field?).ok()
}
