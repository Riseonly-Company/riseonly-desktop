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
use serde::{Deserialize, Serialize};

use crate::modules::notify::engine::core::rise_release_notes_wire_models::RiseReleaseNote;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("notify.resources.v1"));
const CURRENT_SCHEMA_VERSION: i64 = 1;
const RELEASE_NOTES_COMPONENT: &str = "release-notes";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseReleaseNotesResourceSnapshot {
    pub releases: Vec<RiseReleaseNote>,
    pub revision: i64,
    pub source: RiseReleaseNotesCacheSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseReleaseNotesCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

struct RiseReleaseNotesResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseReleaseNotesResourceRepositoryEnvelope {
    schema_version: i64,
    releases: Vec<RiseReleaseNote>,
}

struct RiseReleaseNotesResourceRepositoryState {
    memory_entry: Option<RiseReleaseNotesResourceRepositoryMemoryEntry>,
    version_clock: i64,
    is_shutdown: bool,
}

/// bounded memory accelerator over the encrypted store for the release notes this build matches
/// a miss falls through to the store not to no release
pub struct RiseReleaseNotesResourceRepository {
    engine: Arc<RiseEngine>,
    resource_key: String,
    maximum_memory_bytes: usize,
    state: RiseIsolated<RiseReleaseNotesResourceRepositoryState>,
}

impl RiseReleaseNotesResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 256 * 1_024;

    pub fn new(engine: Arc<RiseEngine>, account_id: String, app_version: String) -> Self {
        Self::with_maximum_memory_bytes(
            engine,
            account_id,
            app_version,
            Self::DEFAULT_MAXIMUM_MEMORY_BYTES,
        )
    }

    pub fn with_maximum_memory_bytes(
        engine: Arc<RiseEngine>,
        account_id: String,
        app_version: String,
        maximum_memory_bytes: usize,
    ) -> Self {
        assert!(maximum_memory_bytes > 0);
        Self {
            engine,
            resource_key: Self::make_key(&[
                account_id.as_str(),
                RELEASE_NOTES_COMPONENT,
                app_version.as_str(),
            ]),
            maximum_memory_bytes,
            state: RiseIsolated::new(RiseReleaseNotesResourceRepositoryState {
                memory_entry: None,
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached(&self) -> anyhow::Result<RiseReleaseNotesResourceSnapshot> {
        let memory = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(s.memory_entry
                .as_ref()
                .map(|entry| (entry.data.clone(), entry.revision)))
        })?;

        if let Some((data, revision)) = memory {
            let releases = Self::decode(data).await?;
            self.ensure_running()?;
            return Ok(RiseReleaseNotesResourceSnapshot {
                releases,
                revision,
                source: RiseReleaseNotesCacheSource::Memory,
            });
        }

        let stored = self
            .engine
            .value(NAMESPACE.clone(), self.resource_key.clone())
            .await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(RiseReleaseNotesResourceSnapshot {
                releases: Vec::new(),
                revision: 0,
                source: RiseReleaseNotesCacheSource::Miss,
            });
        };

        let data = Arc::new(stored.data);
        let releases = Self::decode(data.clone()).await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            s.version_clock = s.version_clock.max(stored.version);
            Self::install_memory(s, self.maximum_memory_bytes, data, stored.version);
            Ok(())
        })?;
        Ok(RiseReleaseNotesResourceSnapshot {
            releases,
            revision: stored.version,
            source: RiseReleaseNotesCacheSource::PersistentStore,
        })
    }

    pub async fn store(&self, releases: &[RiseReleaseNote]) -> anyhow::Result<i64> {
        let version = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(Self::next_version(s))
        })?;
        let data = Arc::new(Self::encode(releases.to_vec()).await?);
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

    fn next_version(state: &mut RiseReleaseNotesResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        state: &mut RiseReleaseNotesResourceRepositoryState,
        maximum_memory_bytes: usize,
        data: Arc<Vec<u8>>,
        revision: i64,
    ) {
        let newest = state.memory_entry.as_ref().map_or(0, |entry| entry.revision);
        if data.len() > maximum_memory_bytes || revision < newest {
            return;
        }
        state.memory_entry = Some(RiseReleaseNotesResourceRepositoryMemoryEntry { data, revision });
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
    async fn encode(releases: Vec<RiseReleaseNote>) -> anyhow::Result<Vec<u8>> {
        let envelope = RiseReleaseNotesResourceRepositoryEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            releases,
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

    async fn decode(data: Arc<Vec<u8>>) -> anyhow::Result<Vec<RiseReleaseNote>> {
        let decoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<RiseReleaseNote>> {
                let envelope: RiseReleaseNotesResourceRepositoryEnvelope =
                    serde_json::from_slice(&data)?;
                if envelope.schema_version != CURRENT_SCHEMA_VERSION {
                    return Ok(Vec::new());
                }
                Ok(envelope.releases)
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
