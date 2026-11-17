use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue,
    RiseStoredValueWritePolicy,
};
use rise_engine::{RiseEngine, RiseWireRemoteError};
use rise_errors::ErrorCode;
use rise_i18n::tr;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_failure_bridge::CanonFailure;
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapViewPreferences;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseMapResourceRepositoryResource {
    State,
    Friends,
    Live,
    Tracks,
    Places,
    Privacy,
    Drafts,
    Reactions,
    ViewPreferences,
}

impl RiseMapResourceRepositoryResource {
    pub const ALL_CASES: [RiseMapResourceRepositoryResource; 9] = [
        RiseMapResourceRepositoryResource::State,
        RiseMapResourceRepositoryResource::Friends,
        RiseMapResourceRepositoryResource::Live,
        RiseMapResourceRepositoryResource::Tracks,
        RiseMapResourceRepositoryResource::Places,
        RiseMapResourceRepositoryResource::Privacy,
        RiseMapResourceRepositoryResource::Drafts,
        RiseMapResourceRepositoryResource::Reactions,
        RiseMapResourceRepositoryResource::ViewPreferences,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            RiseMapResourceRepositoryResource::State => "state",
            RiseMapResourceRepositoryResource::Friends => "friends",
            RiseMapResourceRepositoryResource::Live => "live",
            RiseMapResourceRepositoryResource::Tracks => "tracks",
            RiseMapResourceRepositoryResource::Places => "places",
            RiseMapResourceRepositoryResource::Privacy => "privacy",
            RiseMapResourceRepositoryResource::Drafts => "drafts",
            RiseMapResourceRepositoryResource::Reactions => "reactions",
            RiseMapResourceRepositoryResource::ViewPreferences => "viewPreferences",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|resource| resource.raw_value() == raw)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseMapResourceRepositoryEnvelope<Value> {
    schema_version: i64,
    value: Value,
}

struct RiseMapResourceRepositoryState {
    version_clock: i64,
    write_generations: HashMap<RiseMapResourceRepositoryResource, u64>,
    view_preferences_revision: u64,
    is_shutdown: bool,
}

impl RiseMapResourceRepositoryState {
    fn next_version(&mut self, now_ms: i64) -> i64 {
        self.version_clock = (self.version_clock + 1).max(now_ms);
        self.version_clock
    }

    fn bump_generation(&mut self, resource: RiseMapResourceRepositoryResource) -> u64 {
        let generation = self.write_generations.get(&resource).copied().unwrap_or(0).wrapping_add(1);
        self.write_generations.insert(resource, generation);
        generation
    }
}

static NAMESPACE: LazyLock<RiseStoreNamespace> = LazyLock::new(|| RiseStoreNamespace::new("map.resources.v1"));

/// what the map last knew in the encrypted store
/// membership and friend lists and last live picture so a cold launch opens on pins not a skeleton
/// fixed keys per account
pub struct RiseMapResourceRepository {
    engine: Arc<RiseEngine>,
    key_prefix: String,
    state: RiseIsolated<RiseMapResourceRepositoryState>,
}

impl RiseMapResourceRepository {
    const CURRENT_SCHEMA_VERSION: i64 = 1;

    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Self {
        Self {
            engine,
            key_prefix: format!("{}:{}|", account_id.len(), account_id),
            state: RiseIsolated::new(RiseMapResourceRepositoryState {
                version_clock: 0,
                write_generations: HashMap::new(),
                view_preferences_revision: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached<Value>(&self, resource: RiseMapResourceRepositoryResource) -> Option<Value>
    where
        Value: DeserializeOwned + Send + 'static,
    {
        if self.state.with(|s| s.is_shutdown) {
            return None;
        }
        let Ok(Some(stored)) = self.engine.value(NAMESPACE.clone(), self.key(resource)).await else {
            return None;
        };
        let is_running = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.version_clock = s.version_clock.max(stored.version);
            true
        });
        if !is_running {
            return None;
        }

        let data = stored.data;
        let envelope = RiseAccountRuntime::current()
            .spawn_blocking(move || serde_json::from_slice::<RiseMapResourceRepositoryEnvelope<Value>>(&data).ok())
            .await
            .ok()
            .flatten()?;
        if envelope.schema_version != Self::CURRENT_SCHEMA_VERSION {
            return None;
        }
        Some(envelope.value)
    }

    pub async fn store<Value>(&self, resource: RiseMapResourceRepositoryResource, value: Value)
    where
        Value: Serialize + Send + 'static,
    {
        let now_ms = Self::now_milliseconds();
        let Some((generation, version)) = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            let generation = s.bump_generation(resource);
            Some((generation, s.next_version(now_ms)))
        }) else {
            return;
        };
        let envelope = RiseMapResourceRepositoryEnvelope { schema_version: Self::CURRENT_SCHEMA_VERSION, value };
        let data = RiseAccountRuntime::current()
            .spawn_blocking(move || -> Option<Vec<u8>> {
                let mut tree = serde_json::to_value(&envelope).ok()?;
                tree.sort_all_objects();
                serde_json::to_vec(&tree).ok()
            })
            .await
            .ok()
            .flatten();
        let Some(data) = data else {
            return;
        };
        let is_current = self.state.with(|s| !s.is_shutdown && s.write_generations.get(&resource) == Some(&generation));
        if !is_current {
            return;
        }

        let batch = RiseStoreWriteBatch::new(
            vec![RiseStoreMutation::SetValue {
                namespace: NAMESPACE.clone(),
                key: self.key(resource),
                value: RiseStoredValue::new(data, version, Self::now_milliseconds()),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            }],
            HashSet::new(),
        );
        let _ = self.engine.write(batch).await;
    }

    pub async fn store_view_preferences(&self, preferences: RiseMapViewPreferences, revision: u64) {
        let accepted = self.state.with(|s| {
            if s.is_shutdown || revision <= s.view_preferences_revision {
                return false;
            }
            s.view_preferences_revision = revision;
            true
        });
        if !accepted {
            return;
        }
        self.store(RiseMapResourceRepositoryResource::ViewPreferences, preferences).await;
    }

    pub async fn remove_all(&self) {
        let resources: Vec<RiseMapResourceRepositoryResource> = RiseMapResourceRepositoryResource::ALL_CASES
            .into_iter()
            .filter(|resource| *resource != RiseMapResourceRepositoryResource::ViewPreferences)
            .collect();
        let is_running = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            for resource in &resources {
                s.bump_generation(*resource);
            }
            true
        });
        if !is_running {
            return;
        }
        let batch = RiseStoreWriteBatch::new(
            resources
                .iter()
                .map(|resource| RiseStoreMutation::RemoveValue {
                    namespace: NAMESPACE.clone(),
                    key: self.key(*resource),
                    precondition: RiseStoreVersionPrecondition::Any,
                })
                .collect(),
            HashSet::new(),
        );
        let _ = self.engine.write(batch).await;
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| s.is_shutdown = true);
    }

    fn key(&self, resource: RiseMapResourceRepositoryResource) -> String {
        format!("{}{}", self.key_prefix, resource.raw_value())
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}

pub enum RiseMapFailure {}

impl RiseMapFailure {
    /// canon code phrase in the reader's language
    /// backend prose only for a code the dictionary doesnt know yet
    pub fn text(error: &anyhow::Error) -> Option<String> {
        let remote = error.downcast_ref::<RiseWireRemoteError>()?;
        Some(CanonErrors::text_for(&remote.canon_failure()).unwrap_or_else(|| remote.message.clone()))
    }

    pub fn journey_text(error: &anyhow::Error) -> String {
        let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() else {
            return tr("map_journey_prepare_error");
        };
        CanonErrors::text_for(&remote.canon_failure()).unwrap_or_else(|| tr("map_journey_prepare_error"))
    }

    pub fn is_not_member(error: &anyhow::Error) -> bool {
        error
            .downcast_ref::<RiseWireRemoteError>()
            .is_some_and(|remote| remote.code == ErrorCode::MapNotMember.wire())
    }

    pub fn is_reaction_paced(error: &anyhow::Error) -> bool {
        error
            .downcast_ref::<RiseWireRemoteError>()
            .is_some_and(|remote| remote.code == ErrorCode::MapReactionTooFast.wire())
    }
}
