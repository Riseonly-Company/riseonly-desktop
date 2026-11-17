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

use crate::modules::user::engine::core::rise_user_locale_wire_models::{
    RiseUserLocaleError, RiseUserLocaleSettings,
};
use crate::modules::user::engine::rise_user_locale_repository::RiseUserLocaleCacheSource;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("user.resources.v1"));
const CURRENT_SCHEMA_VERSION: i64 = 1;
const LOCALE_SETTINGS_COMPONENT: &str = "locale-settings";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserLocaleResourceSnapshot {
    pub settings: Option<RiseUserLocaleSettings>,
    pub revision: i64,
    pub source: RiseUserLocaleCacheSource,
}

struct RiseUserLocaleResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseUserLocaleResourceRepositoryEnvelope {
    schema_version: i64,
    settings: RiseUserLocaleSettings,
}

struct RiseUserLocaleResourceRepositoryState {
    memory_entry: Option<RiseUserLocaleResourceRepositoryMemoryEntry>,
    version_clock: i64,
    is_shutdown: bool,
}

/// bounded memory accelerator over the encrypted store for locale prefs,
/// a memory miss falls through to the store
pub struct RiseUserLocaleResourceRepository {
    engine: Arc<RiseEngine>,
    resource_key: String,
    maximum_memory_bytes: usize,
    state: RiseIsolated<RiseUserLocaleResourceRepositoryState>,
}

impl RiseUserLocaleResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 16 * 1_024;

    pub fn new(engine: Arc<RiseEngine>, account_id: &str) -> Self {
        Self::with_maximum_memory_bytes(engine, account_id, Self::DEFAULT_MAXIMUM_MEMORY_BYTES)
    }

    pub fn with_maximum_memory_bytes(
        engine: Arc<RiseEngine>,
        account_id: &str,
        maximum_memory_bytes: usize,
    ) -> Self {
        assert!(maximum_memory_bytes > 0);
        Self {
            engine,
            resource_key: Self::make_key(&[account_id, LOCALE_SETTINGS_COMPONENT]),
            maximum_memory_bytes,
            state: RiseIsolated::new(RiseUserLocaleResourceRepositoryState {
                memory_entry: None,
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached(&self) -> anyhow::Result<RiseUserLocaleResourceSnapshot> {
        let memory = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(s.memory_entry
                .as_ref()
                .map(|entry| (entry.data.clone(), entry.revision)))
        })?;

        if let Some((data, revision)) = memory {
            let settings = Self::decode(data).await?;
            self.ensure_running()?;
            return Ok(RiseUserLocaleResourceSnapshot {
                settings: Some(settings),
                revision,
                source: RiseUserLocaleCacheSource::Memory,
            });
        }

        let stored = self
            .engine
            .value(NAMESPACE.clone(), self.resource_key.clone())
            .await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(RiseUserLocaleResourceSnapshot {
                settings: None,
                revision: 0,
                source: RiseUserLocaleCacheSource::Miss,
            });
        };

        let data = Arc::new(stored.data);
        let settings = Self::decode(data.clone()).await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            s.version_clock = s.version_clock.max(stored.version);
            Self::install_memory(s, self.maximum_memory_bytes, data, stored.version);
            Ok(())
        })?;
        Ok(RiseUserLocaleResourceSnapshot {
            settings: Some(settings),
            revision: stored.version,
            source: RiseUserLocaleCacheSource::PersistentStore,
        })
    }

    pub async fn store(&self, settings: &RiseUserLocaleSettings) -> anyhow::Result<i64> {
        let version = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(Self::next_version(s))
        })?;
        let data = Arc::new(Self::encode(settings.clone()).await?);
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

    fn next_version(state: &mut RiseUserLocaleResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        state: &mut RiseUserLocaleResourceRepositoryState,
        maximum_memory_bytes: usize,
        data: Arc<Vec<u8>>,
        revision: i64,
    ) {
        let newest = state.memory_entry.as_ref().map_or(0, |entry| entry.revision);
        if data.len() > maximum_memory_bytes || revision < newest {
            return;
        }
        state.memory_entry = Some(RiseUserLocaleResourceRepositoryMemoryEntry { data, revision });
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

    /// encoding runs off the callers task, activation path mustnt block the first frame
    async fn encode(settings: RiseUserLocaleSettings) -> anyhow::Result<Vec<u8>> {
        let envelope = RiseUserLocaleResourceRepositoryEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            settings,
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

    async fn decode(data: Arc<Vec<u8>>) -> anyhow::Result<RiseUserLocaleSettings> {
        let decoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<RiseUserLocaleSettings> {
                let envelope: RiseUserLocaleResourceRepositoryEnvelope = serde_json::from_slice(&data)?;
                if envelope.schema_version != CURRENT_SCHEMA_VERSION {
                    return Err(RiseUserLocaleError::InvalidResponse.into());
                }
                Ok(envelope.settings)
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
