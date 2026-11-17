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

use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseMusicResourceRepositoryEnvelope {
    schema_version: i64,
    tracks: Vec<RiseStoredMusicTrack>,
}

struct RiseMusicResourceRepositoryState {
    version_clock: i64,
    is_shutdown: bool,
}

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("music.resources.v1"));

pub struct RiseMusicResourceRepository {
    engine: Arc<RiseEngine>,
    key: String,
    state: RiseIsolated<RiseMusicResourceRepositoryState>,
}

impl RiseMusicResourceRepository {
    pub const DEFAULT_SCOPE: &'static str = "local-track-overrides";
    const SCHEMA_VERSION: i64 = 1;
    const MAXIMUM_STORED_TRACKS: usize = 32;

    pub fn new(engine: Arc<RiseEngine>, account_id: &str) -> Self {
        Self::with_scope(engine, account_id, Self::DEFAULT_SCOPE)
    }

    /// engine with the scope spelled out
    pub fn with_scope(engine: Arc<RiseEngine>, account_id: &str, scope: &str) -> Self {
        Self {
            engine,
            key: Self::make_key(&[account_id, scope]),
            state: RiseIsolated::new(RiseMusicResourceRepositoryState {
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached(&self) -> anyhow::Result<Vec<RiseStoredMusicTrack>> {
        self.ensure_running()?;
        let Some(value) = self.engine.value(NAMESPACE.clone(), self.key.clone()).await? else {
            return Ok(Vec::new());
        };
        self.state.with(|s| s.version_clock = s.version_clock.max(value.version));
        let data = value.data;
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<RiseStoredMusicTrack>> {
                let envelope: RiseMusicResourceRepositoryEnvelope = serde_json::from_slice(&data)?;
                Ok(envelope.tracks)
            })
            .await?
    }

    pub async fn store(&self, tracks: Vec<RiseStoredMusicTrack>) -> anyhow::Result<()> {
        self.ensure_running()?;
        let skipped = tracks.len().saturating_sub(Self::MAXIMUM_STORED_TRACKS);
        let bounded: Vec<RiseStoredMusicTrack> = tracks.into_iter().skip(skipped).collect();
        let data = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                let envelope = RiseMusicResourceRepositoryEnvelope {
                    schema_version: Self::SCHEMA_VERSION,
                    tracks: bounded,
                };
                let mut value = serde_json::to_value(&envelope)?;
                value.sort_all_objects();
                Ok(serde_json::to_vec(&value)?)
            })
            .await??;
        let now = Self::now_milliseconds();
        let version = self.state.with(|s| {
            s.version_clock = (s.version_clock + 1).max(now);
            s.version_clock
        });
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: NAMESPACE.clone(),
                    key: self.key.clone(),
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
