use std::collections::HashSet;
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
use serde::{Deserialize, Serialize};

use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("user.resources.v1"));
const CURRENT_SCHEMA_VERSION: i64 = 2;
const ACHIEVEMENTS_COMPONENT: &str = "achievements";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUserAchievementsCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

impl RiseUserAchievementsCacheSource {
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseUserAchievementsResourcePayload {
    pub achievements: Vec<RiseAchievement>,
    pub total: i64,
    pub hidden_count: i64,
    pub catalog_total: i64,
    pub achievements_rule: String,
}

impl RiseUserAchievementsResourcePayload {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserAchievementsResourceSnapshot {
    pub payload: RiseUserAchievementsResourcePayload,
    pub revision: i64,
    pub source: RiseUserAchievementsCacheSource,
}

struct RiseUserAchievementsResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RiseUserAchievementsResourceRepositoryEnvelope {
    schema_version: i64,
    achievements: Vec<RiseAchievement>,
    total: i64,
    hidden_count: i64,
    catalog_total: i64,
    achievements_rule: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseUserAchievementsResourceRepositoryStoredEnvelope {
    #[serde(default)]
    schema_version: Option<serde_json::Value>,
    #[serde(default)]
    achievements: Option<serde_json::Value>,
    #[serde(default)]
    total: Option<serde_json::Value>,
    #[serde(default)]
    hidden_count: Option<serde_json::Value>,
    #[serde(default)]
    catalog_total: Option<serde_json::Value>,
    #[serde(default)]
    achievements_rule: Option<serde_json::Value>,
}

impl RiseUserAchievementsResourceRepositoryStoredEnvelope {
    fn envelope(self) -> RiseUserAchievementsResourceRepositoryEnvelope {
        RiseUserAchievementsResourceRepositoryEnvelope {
            schema_version: lenient(self.schema_version).unwrap_or(0),
            achievements: lenient(self.achievements).unwrap_or_default(),
            total: lenient(self.total).unwrap_or(0),
            hidden_count: lenient(self.hidden_count).unwrap_or(0),
            catalog_total: lenient(self.catalog_total).unwrap_or(0),
            achievements_rule: lenient(self.achievements_rule).unwrap_or_default(),
        }
    }
}

struct RiseUserAchievementsResourceRepositoryState {
    memory_entry: Option<RiseUserAchievementsResourceRepositoryMemoryEntry>,
    version_clock: i64,
    is_shutdown: bool,
}

/// bounded memory accelerator over the encrypted store for the accounts own achievements
/// eviction is safe, a miss reads the store
pub struct RiseUserAchievementsResourceRepository {
    engine: Arc<RiseEngine>,
    resource_key: String,
    maximum_memory_bytes: usize,
    debug_resource_key: String,
    state: RiseIsolated<RiseUserAchievementsResourceRepositoryState>,
}

impl RiseUserAchievementsResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 256 * 1_024;

    pub fn new(engine: Arc<RiseEngine>, account_id: &str) -> Self {
        Self::with_maximum_memory_bytes(engine, account_id, Self::DEFAULT_MAXIMUM_MEMORY_BYTES)
    }

    pub fn with_maximum_memory_bytes(
        engine: Arc<RiseEngine>,
        account_id: &str,
        maximum_memory_bytes: usize,
    ) -> Self {
        assert!(maximum_memory_bytes > 0);
        let resource_key = Self::make_key(&[account_id, ACHIEVEMENTS_COMPONENT]);
        Self {
            engine,
            debug_resource_key: resource_key.clone(),
            resource_key,
            maximum_memory_bytes,
            state: RiseIsolated::new(RiseUserAchievementsResourceRepositoryState {
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

    pub async fn cached(&self) -> anyhow::Result<RiseUserAchievementsResourceSnapshot> {
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
            return Ok(RiseUserAchievementsResourceSnapshot {
                payload,
                revision,
                source: RiseUserAchievementsCacheSource::Memory,
            });
        }

        let stored = self
            .engine
            .value(NAMESPACE.clone(), self.resource_key.clone())
            .await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(RiseUserAchievementsResourceSnapshot {
                payload: RiseUserAchievementsResourcePayload::empty(),
                revision: 0,
                source: RiseUserAchievementsCacheSource::Miss,
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
        Ok(RiseUserAchievementsResourceSnapshot {
            payload,
            revision: stored.version,
            source: RiseUserAchievementsCacheSource::PersistentStore,
        })
    }

    pub async fn store(&self, payload: &RiseUserAchievementsResourcePayload) -> anyhow::Result<i64> {
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
    fn next_version(state: &mut RiseUserAchievementsResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        state: &mut RiseUserAchievementsResourceRepositoryState,
        maximum_memory_bytes: usize,
        data: Arc<Vec<u8>>,
        revision: i64,
    ) {
        let newest = state.memory_entry.as_ref().map_or(0, |entry| entry.revision);
        if data.len() > maximum_memory_bytes || revision < newest {
            return;
        }
        state.memory_entry = Some(RiseUserAchievementsResourceRepositoryMemoryEntry { data, revision });
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

    /// encoding runs off the callers task, this is on the activation path and mustnt delay the first frame
    async fn encode(payload: RiseUserAchievementsResourcePayload) -> anyhow::Result<Vec<u8>> {
        let envelope = RiseUserAchievementsResourceRepositoryEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            achievements: payload.achievements,
            total: payload.total,
            hidden_count: payload.hidden_count,
            catalog_total: payload.catalog_total,
            achievements_rule: payload.achievements_rule,
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

    /// schema mismatch gives an empty cache not an error, the authoritative refresh fills it
    async fn decode(data: Arc<Vec<u8>>) -> anyhow::Result<RiseUserAchievementsResourcePayload> {
        let decoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<RiseUserAchievementsResourcePayload> {
                let envelope =
                    serde_json::from_slice::<RiseUserAchievementsResourceRepositoryStoredEnvelope>(&data)?
                        .envelope();
                if envelope.schema_version != CURRENT_SCHEMA_VERSION {
                    return Ok(RiseUserAchievementsResourcePayload::empty());
                }
                Ok(RiseUserAchievementsResourcePayload {
                    achievements: envelope.achievements,
                    total: envelope.total,
                    hidden_count: envelope.hidden_count,
                    catalog_total: envelope.catalog_total,
                    achievements_rule: envelope.achievements_rule,
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
