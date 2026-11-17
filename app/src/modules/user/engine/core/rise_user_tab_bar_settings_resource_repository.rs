use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch,
    RiseStoredValue, RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine};
use serde::{Deserialize, Serialize};

use crate::core::widgets::navigations::main_tab_bar_settings::RiseTabBarSettingsSet;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("user.resources.v1"));
const TAB_BAR_SETTINGS_COMPONENT: &str = "tab-bar-settings";

/// what the device keeps of the accounts bar between launches
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseUserTabBarPersistedState {
    pub schema_version: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<RiseTabBarSettingsSet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<RiseTabBarSettingsSet>,
}

impl RiseUserTabBarPersistedState {
    const CURRENT_SCHEMA_VERSION: i64 = 1;

    /// new state with the schema version at the current one
    pub fn new(server: Option<RiseTabBarSettingsSet>, local: Option<RiseTabBarSettingsSet>) -> Self {
        Self { schema_version: Self::CURRENT_SCHEMA_VERSION, server, local }
    }

    pub fn is_current_schema(&self) -> bool {
        self.schema_version == Self::CURRENT_SCHEMA_VERSION
    }
}

struct RiseUserTabBarSettingsResourceRepositoryState {
    version_clock: i64,
    is_shutdown: bool,
}

pub struct RiseUserTabBarSettingsResourceRepository {
    engine: Arc<RiseEngine>,
    resource_key: String,
    state: RiseIsolated<RiseUserTabBarSettingsResourceRepositoryState>,
}

impl RiseUserTabBarSettingsResourceRepository {
    pub fn new(engine: Arc<RiseEngine>, account_id: &str) -> Self {
        let resource_key = [account_id, TAB_BAR_SETTINGS_COMPONENT]
            .iter()
            .map(|component| format!("{}:{}", component.len(), component))
            .collect::<Vec<_>>()
            .join("|");
        Self {
            engine,
            resource_key,
            state: RiseIsolated::new(RiseUserTabBarSettingsResourceRepositoryState {
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    /// none if the device never had this accounts bar or its saved shape isnt readable anymore
    pub async fn cached(&self) -> anyhow::Result<Option<RiseUserTabBarPersistedState>> {
        self.ensure_running()?;
        let stored = self.engine.value(NAMESPACE.clone(), self.resource_key.clone()).await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(None);
        };
        self.state.with(|s| s.version_clock = s.version_clock.max(stored.version));
        let state = serde_json::from_slice::<RiseUserTabBarPersistedState>(&stored.data).ok();
        Ok(state.filter(RiseUserTabBarPersistedState::is_current_schema))
    }

    pub async fn store(&self, state: &RiseUserTabBarPersistedState) -> anyhow::Result<()> {
        self.ensure_running()?;
        let mut value = serde_json::to_value(state)?;
        value.sort_all_objects();
        let data = serde_json::to_vec(&value)?;
        let version = self.state.with(|s| {
            s.version_clock = (s.version_clock + 1).max(Self::now_milliseconds());
            s.version_clock
        });
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: NAMESPACE.clone(),
                    key: self.resource_key.clone(),
                    value: RiseStoredValue::new(data, version, Self::now_milliseconds()),
                    precondition: RiseStoreVersionPrecondition::Any,
                    policy: RiseStoredValueWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await?;
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| s.is_shutdown = true);
    }

    fn ensure_running(&self) -> Result<(), CancellationError> {
        if self.state.with(|s| s.is_shutdown) { Err(CancellationError) } else { Ok(()) }
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
