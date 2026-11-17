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
use rise_theme::{RiseThemePlatform, RiseThemePreferences};
use serde::{Deserialize, Deserializer, Serialize};

use crate::modules::user::engine::core::rise_user_rpc::update_theme_settings_platform;
use crate::modules::user::engine::rise_user_engine_models::RiseUserThemeSettingsCacheSource;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("user.resources.v1"));
const THEME_SETTINGS_COMPONENT: &str = "theme-settings";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserThemeSettingsResourceSnapshot {
    pub preferences: Option<RiseThemePreferences>,
    pub revision: i64,
    pub source: RiseUserThemeSettingsCacheSource,
    pub requires_optimistic_rollback: bool,
    pub unsynced_platforms: HashSet<RiseThemePlatform>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserThemeSettingsPersistedState {
    pub committed_preferences: RiseThemePreferences,
    pub optimistic_preferences: Option<RiseThemePreferences>,
    pub unsynced_platforms: HashSet<RiseThemePlatform>,
}

impl RiseUserThemeSettingsPersistedState {
    /// new state with unsynced platforms left empty
    pub fn new(
        committed_preferences: RiseThemePreferences,
        optimistic_preferences: Option<RiseThemePreferences>,
    ) -> Self {
        Self::with_unsynced_platforms(committed_preferences, optimistic_preferences, HashSet::new())
    }

    pub fn with_unsynced_platforms(
        committed_preferences: RiseThemePreferences,
        optimistic_preferences: Option<RiseThemePreferences>,
        unsynced_platforms: HashSet<RiseThemePlatform>,
    ) -> Self {
        Self { committed_preferences, optimistic_preferences, unsynced_platforms }
    }

    pub fn preferences_for_activation(&self) -> RiseThemePreferences {
        self.committed_preferences
    }

    pub fn requires_optimistic_rollback(&self) -> bool {
        self.optimistic_preferences.is_some()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseUserThemeSettingsPersistedCodecHeader {
    #[serde(default, deserialize_with = "present_value")]
    schema_version: Option<serde_json::Value>,
}

fn present_value<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Value>, D::Error> {
    serde_json::Value::deserialize(deserializer).map(Some)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseUserThemeSettingsPersistedCodecEnvelope {
    schema_version: i64,
    committed_preferences: RiseThemePreferences,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    optimistic_preferences: Option<RiseThemePreferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unsynced_platforms: Option<Vec<RiseThemePlatform>>,
}

#[derive(Debug, thiserror::Error)]
enum RiseUserThemeSettingsPersistedCodecCodecError {
    #[error("invalidEnvelope")]
    InvalidEnvelope,
    #[error("unsupportedSchemaVersion({0})")]
    UnsupportedSchemaVersion(i64),
}

pub enum RiseUserThemeSettingsPersistedCodec {}

impl RiseUserThemeSettingsPersistedCodec {
    const CURRENT_SCHEMA_VERSION: i64 = 1;

    pub fn encode(state: &RiseUserThemeSettingsPersistedState) -> anyhow::Result<Vec<u8>> {
        let unsynced_platforms = if state.unsynced_platforms.is_empty() {
            None
        } else {
            let mut platforms: Vec<RiseThemePlatform> =
                state.unsynced_platforms.iter().copied().collect();
            platforms.sort_by_key(|platform| update_theme_settings_platform(*platform).raw_value());
            Some(platforms)
        };
        let envelope = RiseUserThemeSettingsPersistedCodecEnvelope {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            committed_preferences: state.committed_preferences,
            optimistic_preferences: state.optimistic_preferences,
            unsynced_platforms,
        };
        let mut value = serde_json::to_value(&envelope)?;
        value.sort_all_objects();
        Ok(serde_json::to_vec(&value)?)
    }

    pub fn decode(data: &[u8]) -> anyhow::Result<RiseUserThemeSettingsPersistedState> {
        let header: RiseUserThemeSettingsPersistedCodecHeader = serde_json::from_slice(data)?;
        let Some(schema_version) = header.schema_version else {
            return Ok(RiseUserThemeSettingsPersistedState::new(
                serde_json::from_slice::<RiseThemePreferences>(data)?,
                None,
            ));
        };
        let Some(schema_version) = serde_json::from_value::<Option<i64>>(schema_version)? else {
            return Err(RiseUserThemeSettingsPersistedCodecCodecError::InvalidEnvelope.into());
        };
        if schema_version != Self::CURRENT_SCHEMA_VERSION {
            return Err(
                RiseUserThemeSettingsPersistedCodecCodecError::UnsupportedSchemaVersion(schema_version)
                    .into(),
            );
        }
        let envelope: RiseUserThemeSettingsPersistedCodecEnvelope = serde_json::from_slice(data)?;
        Ok(RiseUserThemeSettingsPersistedState::with_unsynced_platforms(
            envelope.committed_preferences,
            envelope.optimistic_preferences,
            envelope.unsynced_platforms.unwrap_or_default().into_iter().collect(),
        ))
    }
}

struct RiseUserThemeSettingsResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
}

struct RiseUserThemeSettingsResourceRepositoryState {
    memory_entry: Option<RiseUserThemeSettingsResourceRepositoryMemoryEntry>,
    version_clock: i64,
    is_shutdown: bool,
}

pub struct RiseUserThemeSettingsResourceRepository {
    engine: Arc<RiseEngine>,
    resource_key: String,
    maximum_memory_bytes: usize,
    pub debug_resource_key: String,
    state: RiseIsolated<RiseUserThemeSettingsResourceRepositoryState>,
}

impl RiseUserThemeSettingsResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 64 * 1_024;

    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Self {
        Self::with_maximum_memory_bytes(engine, account_id, Self::DEFAULT_MAXIMUM_MEMORY_BYTES)
    }

    pub fn with_maximum_memory_bytes(
        engine: Arc<RiseEngine>,
        account_id: String,
        maximum_memory_bytes: usize,
    ) -> Self {
        assert!(maximum_memory_bytes > 0);
        let resource_key = Self::make_key(&[account_id.as_str(), THEME_SETTINGS_COMPONENT]);
        Self {
            engine,
            debug_resource_key: resource_key.clone(),
            resource_key,
            maximum_memory_bytes,
            state: RiseIsolated::new(RiseUserThemeSettingsResourceRepositoryState {
                memory_entry: None,
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached(&self) -> anyhow::Result<RiseUserThemeSettingsResourceSnapshot> {
        let memory = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(s.memory_entry.as_ref().map(|entry| (entry.data.clone(), entry.revision)))
        })?;

        if let Some((data, revision)) = memory {
            let state = Self::decode(data).await?;
            self.ensure_running()?;
            return Ok(RiseUserThemeSettingsResourceSnapshot {
                preferences: Some(state.preferences_for_activation()),
                revision,
                source: RiseUserThemeSettingsCacheSource::Memory,
                requires_optimistic_rollback: state.requires_optimistic_rollback(),
                unsynced_platforms: state.unsynced_platforms,
            });
        }

        let stored = self.engine.value(NAMESPACE.clone(), self.resource_key.clone()).await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(RiseUserThemeSettingsResourceSnapshot {
                preferences: None,
                revision: 0,
                source: RiseUserThemeSettingsCacheSource::Miss,
                requires_optimistic_rollback: false,
                unsynced_platforms: HashSet::new(),
            });
        };

        let data = Arc::new(stored.data);
        let state = Self::decode(data.clone()).await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            s.version_clock = s.version_clock.max(stored.version);
            Self::install_memory(s, self.maximum_memory_bytes, data, stored.version);
            Ok(())
        })?;
        Ok(RiseUserThemeSettingsResourceSnapshot {
            preferences: Some(state.preferences_for_activation()),
            revision: stored.version,
            source: RiseUserThemeSettingsCacheSource::PersistentStore,
            requires_optimistic_rollback: state.requires_optimistic_rollback(),
            unsynced_platforms: state.unsynced_platforms,
        })
    }

    /// stores committed and optimistic prefs, pass an empty HashSet for unsynced platforms
    pub async fn store(
        &self,
        committed_preferences: RiseThemePreferences,
        optimistic_preferences: Option<RiseThemePreferences>,
        unsynced_platforms: HashSet<RiseThemePlatform>,
    ) -> anyhow::Result<i64> {
        let version = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(Self::next_version(s))
        })?;
        let data = Arc::new(
            Self::encode(RiseUserThemeSettingsPersistedState::with_unsynced_platforms(
                committed_preferences,
                optimistic_preferences,
                unsynced_platforms,
            ))
            .await?,
        );
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
        if self.state.with(|s| s.is_shutdown) { Err(CancellationError) } else { Ok(()) }
    }

    fn next_version(state: &mut RiseUserThemeSettingsResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        state: &mut RiseUserThemeSettingsResourceRepositoryState,
        maximum_memory_bytes: usize,
        data: Arc<Vec<u8>>,
        revision: i64,
    ) {
        let newest = state.memory_entry.as_ref().map_or(0, |entry| entry.revision);
        if data.len() > maximum_memory_bytes || revision < newest {
            return;
        }
        state.memory_entry =
            Some(RiseUserThemeSettingsResourceRepositoryMemoryEntry { data, revision });
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

    /// encodes in the background at low priority
    async fn encode(state: RiseUserThemeSettingsPersistedState) -> anyhow::Result<Vec<u8>> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || RiseUserThemeSettingsPersistedCodec::encode(&state))
            .await?
    }

    async fn decode(data: Arc<Vec<u8>>) -> anyhow::Result<RiseUserThemeSettingsPersistedState> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || RiseUserThemeSettingsPersistedCodec::decode(&data))
            .await?
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
