use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use rise_canon::rise_key::notify::ANNOUNCEMENTS;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch,
    RiseStoredValue, RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine};
use serde::{Deserialize, Serialize};

use crate::modules::notify::engine::core::rise_announcements_wire_models::RiseAnnouncement;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("notify.resources.v1"));
const CURRENT_SCHEMA_VERSION: i64 = 1;
const UNACKED_CLOSES_COMPONENT: &str = "unacked_closes";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseAnnouncementsResourceSnapshot {
    pub announcements: Vec<RiseAnnouncement>,
    pub revision: i64,
    pub source: RiseAnnouncementsCacheSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseAnnouncementClose {
    pub announcement_id: i64,
    pub seen_slides: i64,
    pub outcome: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseAnnouncementsCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

struct RiseAnnouncementsResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseAnnouncementsResourceRepositoryEnvelope {
    schema_version: i64,
    announcements: Vec<RiseAnnouncement>,
}

struct RiseAnnouncementsResourceRepositoryState {
    memory_entry: Option<RiseAnnouncementsResourceRepositoryMemoryEntry>,
    version_clock: i64,
    is_shutdown: bool,
}

/// bounded memory accelerator over the encrypted store for undismissed broadcasts
/// a miss falls through to the store, not to no announcements
/// key is the account only
pub struct RiseAnnouncementsResourceRepository {
    engine: Arc<RiseEngine>,
    resource_key: String,
    closes_key: String,
    maximum_memory_bytes: usize,
    state: RiseIsolated<RiseAnnouncementsResourceRepositoryState>,
}

impl RiseAnnouncementsResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 256 * 1_024;

    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Self {
        Self::with_maximum_memory_bytes(engine, account_id, Self::DEFAULT_MAXIMUM_MEMORY_BYTES)
    }

    pub fn with_maximum_memory_bytes(
        engine: Arc<RiseEngine>,
        account_id: String,
        maximum_memory_bytes: usize,
    ) -> Self {
        assert!(maximum_memory_bytes > 0);
        Self {
            engine,
            resource_key: Self::make_key(&[account_id.as_str(), ANNOUNCEMENTS]),
            closes_key: Self::make_key(&[account_id.as_str(), ANNOUNCEMENTS, UNACKED_CLOSES_COMPONENT]),
            maximum_memory_bytes,
            state: RiseIsolated::new(RiseAnnouncementsResourceRepositoryState {
                memory_entry: None,
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached(&self) -> anyhow::Result<RiseAnnouncementsResourceSnapshot> {
        let memory = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(s.memory_entry
                .as_ref()
                .map(|entry| (entry.data.clone(), entry.revision)))
        })?;

        if let Some((data, revision)) = memory {
            let announcements = Self::decode(data).await?;
            self.ensure_running()?;
            return Ok(RiseAnnouncementsResourceSnapshot {
                announcements,
                revision,
                source: RiseAnnouncementsCacheSource::Memory,
            });
        }

        let stored = self
            .engine
            .value(NAMESPACE.clone(), self.resource_key.clone())
            .await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(RiseAnnouncementsResourceSnapshot {
                announcements: Vec::new(),
                revision: 0,
                source: RiseAnnouncementsCacheSource::Miss,
            });
        };

        let data = Arc::new(stored.data);
        let announcements = Self::decode(data.clone()).await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            s.version_clock = s.version_clock.max(stored.version);
            Self::install_memory(s, self.maximum_memory_bytes, data, stored.version);
            Ok(())
        })?;
        Ok(RiseAnnouncementsResourceSnapshot {
            announcements,
            revision: stored.version,
            source: RiseAnnouncementsCacheSource::PersistentStore,
        })
    }

    pub async fn store(&self, announcements: &[RiseAnnouncement]) -> anyhow::Result<i64> {
        let version = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(Self::next_version(s))
        })?;
        let data = Arc::new(Self::encode(announcements.to_vec()).await?);
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

    /// closes whose ack hasnt reached the server yet
    /// kept apart from the list so a cold launch with the ack in flight doesnt bring the closed modal back
    pub async fn unacked_closes(&self) -> Vec<RiseAnnouncementClose> {
        if self.state.with(|s| s.is_shutdown) {
            return Vec::new();
        }
        let Ok(Some(stored)) = self
            .engine
            .value(NAMESPACE.clone(), self.closes_key.clone())
            .await
        else {
            return Vec::new();
        };
        if self.state.with(|s| s.is_shutdown) {
            return Vec::new();
        }
        serde_json::from_slice(&stored.data).unwrap_or_default()
    }

    pub async fn store_unacked_closes(&self, closes: &[RiseAnnouncementClose]) {
        let version = self.state.with(|s| (!s.is_shutdown).then(|| Self::next_version(s)));
        let Some(version) = version else {
            return;
        };
        let Ok(data) = serde_json::to_vec(closes) else {
            return;
        };
        self.engine
            .write(Self::set_value_batch(&self.closes_key, data, version))
            .await
            .ok();
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

    fn next_version(state: &mut RiseAnnouncementsResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        state: &mut RiseAnnouncementsResourceRepositoryState,
        maximum_memory_bytes: usize,
        data: Arc<Vec<u8>>,
        revision: i64,
    ) {
        let newest = state.memory_entry.as_ref().map_or(0, |entry| entry.revision);
        if data.len() > maximum_memory_bytes || revision < newest {
            return;
        }
        state.memory_entry = Some(RiseAnnouncementsResourceRepositoryMemoryEntry { data, revision });
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

    /// coding runs off the callers task
    /// this sits on the activation path and must not delay the first frame
    async fn encode(announcements: Vec<RiseAnnouncement>) -> anyhow::Result<Vec<u8>> {
        let envelope = RiseAnnouncementsResourceRepositoryEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            announcements,
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

    async fn decode(data: Arc<Vec<u8>>) -> anyhow::Result<Vec<RiseAnnouncement>> {
        let decoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<RiseAnnouncement>> {
                let envelope: RiseAnnouncementsResourceRepositoryEnvelope =
                    serde_json::from_slice(&data)?;
                if envelope.schema_version != CURRENT_SCHEMA_VERSION {
                    return Ok(Vec::new());
                }
                Ok(envelope.announcements)
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
