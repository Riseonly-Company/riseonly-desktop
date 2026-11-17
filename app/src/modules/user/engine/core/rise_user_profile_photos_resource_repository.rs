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
use serde::{Deserialize, Serialize};

use crate::modules::user::engine::core::rise_user_profile_photo_wire_models::RiseProfilePhoto;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("user.resources.v1"));
const CURRENT_SCHEMA_VERSION: i64 = 1;
const PROFILE_PHOTOS_COMPONENT: &str = "profile-photos";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUserProfilePhotosCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

impl RiseUserProfilePhotosCacheSource {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserProfilePhotosResourcePayload {
    pub photos: Vec<RiseProfilePhoto>,
    pub total: i64,
    pub has_more: bool,
    pub next_cursor: Option<String>,
    pub visible: bool,
}

impl RiseUserProfilePhotosResourcePayload {
    pub fn empty() -> Self {
        Self { photos: Vec::new(), total: 0, has_more: false, next_cursor: None, visible: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserProfilePhotosResourceSnapshot {
    pub payload: RiseUserProfilePhotosResourcePayload,
    pub revision: i64,
    pub source: RiseUserProfilePhotosCacheSource,
}

struct RiseUserProfilePhotosResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RiseUserProfilePhotosResourceRepositoryEnvelope {
    schema_version: i64,
    photos: Vec<RiseProfilePhoto>,
    total: i64,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
    visible: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseUserProfilePhotosResourceRepositoryStoredEnvelope {
    #[serde(default)]
    schema_version: Option<serde_json::Value>,
    #[serde(default)]
    photos: Option<serde_json::Value>,
    #[serde(default)]
    total: Option<serde_json::Value>,
    #[serde(default)]
    has_more: Option<serde_json::Value>,
    #[serde(default)]
    next_cursor: Option<serde_json::Value>,
    #[serde(default)]
    visible: Option<serde_json::Value>,
}

impl RiseUserProfilePhotosResourceRepositoryStoredEnvelope {
    fn envelope(self) -> RiseUserProfilePhotosResourceRepositoryEnvelope {
        RiseUserProfilePhotosResourceRepositoryEnvelope {
            schema_version: lenient(self.schema_version).unwrap_or(0),
            photos: lenient(self.photos).unwrap_or_default(),
            total: lenient(self.total).unwrap_or(0),
            has_more: lenient(self.has_more).unwrap_or(false),
            next_cursor: lenient(self.next_cursor),
            visible: lenient(self.visible).unwrap_or(true),
        }
    }
}

struct RiseUserProfilePhotosResourceRepositoryState {
    memory_entries: HashMap<String, RiseUserProfilePhotosResourceRepositoryMemoryEntry>,
    version_clock: i64,
    is_shutdown: bool,
}

/// bounded memory accelerator over the encrypted store, keyed by viewer and viewed profile
/// foreign albums persisted too, a privacy refusal overwrites the cache
pub struct RiseUserProfilePhotosResourceRepository {
    engine: Arc<RiseEngine>,
    account_id: String,
    maximum_memory_bytes: usize,
    maximum_resident_albums: usize,
    state: RiseIsolated<RiseUserProfilePhotosResourceRepositoryState>,
}

impl RiseUserProfilePhotosResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 64 * 1_024;
    pub const DEFAULT_MAXIMUM_RESIDENT_ALBUMS: usize = 8;

    pub fn new(engine: Arc<RiseEngine>, account_id: &str) -> Self {
        Self::with_limits(
            engine,
            account_id,
            Self::DEFAULT_MAXIMUM_MEMORY_BYTES,
            Self::DEFAULT_MAXIMUM_RESIDENT_ALBUMS,
        )
    }

    pub fn with_limits(
        engine: Arc<RiseEngine>,
        account_id: &str,
        maximum_memory_bytes: usize,
        maximum_resident_albums: usize,
    ) -> Self {
        assert!(maximum_memory_bytes > 0);
        assert!(maximum_resident_albums > 0);
        Self {
            engine,
            account_id: account_id.to_owned(),
            maximum_memory_bytes,
            maximum_resident_albums,
            state: RiseIsolated::new(RiseUserProfilePhotosResourceRepositoryState {
                memory_entries: HashMap::new(),
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    /// key used in debug output for this user
    pub fn debug_resource_key(&self, user_id: &str) -> String {
        Self::make_key(&[self.account_id.as_str(), PROFILE_PHOTOS_COMPONENT, user_id])
    }

    pub async fn cached(&self, user_id: &str) -> anyhow::Result<RiseUserProfilePhotosResourceSnapshot> {
        let key = self.debug_resource_key(user_id);
        let memory = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(s.memory_entries
                .get(&key)
                .map(|entry| (entry.data.clone(), entry.revision)))
        })?;

        if let Some((data, revision)) = memory {
            let payload = Self::decode(data).await?;
            self.ensure_running()?;
            return Ok(RiseUserProfilePhotosResourceSnapshot {
                payload,
                revision,
                source: RiseUserProfilePhotosCacheSource::Memory,
            });
        }

        let stored = self.engine.value(NAMESPACE.clone(), key.clone()).await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(RiseUserProfilePhotosResourceSnapshot {
                payload: RiseUserProfilePhotosResourcePayload::empty(),
                revision: 0,
                source: RiseUserProfilePhotosCacheSource::Miss,
            });
        };

        let data = Arc::new(stored.data);
        let payload = Self::decode(data.clone()).await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            s.version_clock = s.version_clock.max(stored.version);
            self.install_memory(s, key, data, stored.version);
            Ok(())
        })?;
        Ok(RiseUserProfilePhotosResourceSnapshot {
            payload,
            revision: stored.version,
            source: RiseUserProfilePhotosCacheSource::PersistentStore,
        })
    }

    pub async fn store(
        &self,
        payload: &RiseUserProfilePhotosResourcePayload,
        user_id: &str,
    ) -> anyhow::Result<i64> {
        let key = self.debug_resource_key(user_id);
        let version = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(Self::next_version(s))
        })?;
        let data = Arc::new(Self::encode(payload.clone()).await?);
        self.ensure_running()?;

        self.engine
            .write(Self::set_value_batch(&key, data.as_ref().clone(), version))
            .await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            self.install_memory(s, key, data, version);
            Ok(())
        })?;
        Ok(version)
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.memory_entries.clear();
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
    fn next_version(state: &mut RiseUserProfilePhotosResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        &self,
        state: &mut RiseUserProfilePhotosResourceRepositoryState,
        key: String,
        data: Arc<Vec<u8>>,
        revision: i64,
    ) {
        let newest = state.memory_entries.get(&key).map_or(0, |entry| entry.revision);
        if data.len() > self.maximum_memory_bytes || revision < newest {
            return;
        }
        state
            .memory_entries
            .insert(key, RiseUserProfilePhotosResourceRepositoryMemoryEntry { data, revision });
        if state.memory_entries.len() <= self.maximum_resident_albums {
            return;
        }
        let mut by_revision: Vec<(String, i64)> = state
            .memory_entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.revision))
            .collect();
        by_revision.sort_by_key(|(_, revision)| *revision);
        let excess = state.memory_entries.len() - self.maximum_resident_albums;
        for (victim, _) in by_revision.into_iter().take(excess) {
            state.memory_entries.remove(&victim);
        }
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

    /// encoding runs off the callers task, viewer open path mustnt block the first frame
    async fn encode(payload: RiseUserProfilePhotosResourcePayload) -> anyhow::Result<Vec<u8>> {
        let envelope = RiseUserProfilePhotosResourceRepositoryEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            photos: payload.photos,
            total: payload.total,
            has_more: payload.has_more,
            next_cursor: payload.next_cursor,
            visible: payload.visible,
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
    async fn decode(data: Arc<Vec<u8>>) -> anyhow::Result<RiseUserProfilePhotosResourcePayload> {
        let decoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<RiseUserProfilePhotosResourcePayload> {
                let envelope =
                    serde_json::from_slice::<RiseUserProfilePhotosResourceRepositoryStoredEnvelope>(&data)?
                        .envelope();
                if envelope.schema_version != CURRENT_SCHEMA_VERSION {
                    return Ok(RiseUserProfilePhotosResourcePayload::empty());
                }
                Ok(RiseUserProfilePhotosResourcePayload {
                    photos: envelope.photos,
                    total: envelope.total,
                    has_more: envelope.has_more,
                    next_cursor: envelope.next_cursor,
                    visible: envelope.visible,
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
