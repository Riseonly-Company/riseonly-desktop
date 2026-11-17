use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use gpui::{App, AsyncApp};
use rise_canon::RiseTimingCanon;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::rise_map_resource_repository::{
    RiseMapFailure, RiseMapResourceRepository, RiseMapResourceRepositoryResource,
};
use crate::modules::map::engine::core::rise_map_rpc::RiseMapRPC;
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapCacheSource, RiseMapLoadState, RiseMapMutationOutcome,
};
use crate::modules::map::engine::rise_map_presentation::{RiseMapPresentation, RiseMapSessionIdentity};
use crate::modules::map::stores::map_actions::map_privacy_types::{
    MapPrivacy, MapPrivacyAction, MapPrivacyKind, MapPrivacyRule,
};
use crate::modules::map::stores::map_actions::map_types::MapUser;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapPrivacySnapshot {
    pub privacy: Option<MapPrivacy>,
    pub load_state: RiseMapLoadState,
    pub error_text: Option<String>,
    pub cache_source: RiseMapCacheSource,
    pub revision: u64,
}

impl RiseMapPrivacySnapshot {
    pub const EMPTY: RiseMapPrivacySnapshot = RiseMapPrivacySnapshot {
        privacy: None,
        load_state: RiseMapLoadState::Idle,
        error_text: None,
        cache_source: RiseMapCacheSource::Miss,
        revision: 0,
    };

    pub fn shows_initial_loader(&self) -> bool {
        self.privacy.is_none() && matches!(self.load_state, RiseMapLoadState::Idle | RiseMapLoadState::Loading)
    }
}

struct RiseMapPrivacyRepositoryState {
    privacy: Option<MapPrivacy>,
    load_state: RiseMapLoadState,
    error_text: Option<String>,
    cache_source: RiseMapCacheSource,
    revision: u64,
    did_read_store: bool,
    last_refresh_ms: i64,
    mutation_generation: u64,
    is_shutdown: bool,
}

impl RiseMapPrivacyRepositoryState {
    fn next_snapshot(&mut self) -> RiseMapPrivacySnapshot {
        self.revision += 1;
        RiseMapPrivacySnapshot {
            privacy: self.privacy.clone(),
            load_state: self.load_state,
            error_text: self.error_text.clone(),
            cache_source: self.cache_source,
            revision: self.revision,
        }
    }
}

static REFRESH_INTERVAL_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("map_state"));

struct RiseMapPrivacyRollback<'a> {
    repository: &'a RiseMapPrivacyRepository,
    generation: u64,
    previous: Option<MapPrivacy>,
}

impl RiseMapPrivacyRollback<'_> {
    fn disarm(&mut self) {
        self.previous = None;
    }
}

impl Drop for RiseMapPrivacyRollback<'_> {
    fn drop(&mut self) {
        let Some(previous) = self.previous.take() else {
            return;
        };
        let generation = self.generation;
        let snapshot = self.repository.state.with(|s| {
            if s.is_shutdown || generation != s.mutation_generation {
                return None;
            }
            s.privacy = Some(previous);
            Some(s.next_snapshot())
        });
        if let Some(snapshot) = snapshot {
            RiseMapPrivacyRepository::post_snapshot(snapshot, self.repository.identity.clone());
        }
    }
}

/// map privacy, two rules and four exception lists owned by map-service alone
/// server precedence, always show beats a rule and deny beats allow
/// sharing switch and friendship and block beat both
pub struct RiseMapPrivacyRepository {
    rpc: RiseMapRPC,
    resources: Arc<RiseMapResourceRepository>,
    identity: RiseMapSessionIdentity,
    state: RiseIsolated<RiseMapPrivacyRepositoryState>,
}

impl RiseMapPrivacyRepository {
    pub fn new(rpc: RiseMapRPC, resources: Arc<RiseMapResourceRepository>, identity: RiseMapSessionIdentity) -> Self {
        Self {
            rpc,
            resources,
            identity,
            state: RiseIsolated::new(RiseMapPrivacyRepositoryState {
                privacy: None,
                load_state: RiseMapLoadState::Idle,
                error_text: None,
                cache_source: RiseMapCacheSource::Miss,
                revision: 0,
                did_read_store: false,
                last_refresh_ms: 0,
                mutation_generation: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn open(&self, force: bool) {
        let Some(reads_store) = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            if s.did_read_store {
                return Some(false);
            }
            s.did_read_store = true;
            Some(s.privacy.is_none())
        }) else {
            return;
        };
        if reads_store
            && let Some(cached) =
                self.resources.cached::<MapPrivacy>(RiseMapResourceRepositoryResource::Privacy).await
        {
            let applied = self.state.with(|s| {
                if s.is_shutdown {
                    return None;
                }
                if s.privacy.is_some() {
                    return Some(false);
                }
                s.privacy = Some(cached);
                s.cache_source = RiseMapCacheSource::PersistentStore;
                s.load_state = RiseMapLoadState::Loaded;
                Some(true)
            });
            match applied {
                None => return,
                Some(true) => self.publish().await,
                Some(false) => {}
            }
        }

        let now_ms = Self::now_milliseconds();
        let Some((generation, shows_loading)) = self.state.with(|s| {
            let is_stale = now_ms - s.last_refresh_ms >= *REFRESH_INTERVAL_MS;
            if !(force || is_stale || s.privacy.is_none()) {
                return None;
            }
            let shows_loading = s.privacy.is_none();
            if shows_loading {
                s.load_state = RiseMapLoadState::Loading;
            }
            Some((s.mutation_generation, shows_loading))
        }) else {
            return;
        };
        if shows_loading {
            self.publish().await;
        }
        match self.rpc.get_privacy().await {
            Ok(fresh) => {
                let is_current = self.state.with(|s| {
                    if s.is_shutdown || generation != s.mutation_generation {
                        return false;
                    }
                    s.last_refresh_ms = Self::now_milliseconds();
                    true
                });
                if is_current {
                    self.accept(fresh).await;
                }
            }
            Err(error) => {
                let error_text = RiseMapFailure::text(&error);
                let failed = self.state.with(|s| {
                    if s.is_shutdown || generation != s.mutation_generation || s.privacy.is_some() {
                        return false;
                    }
                    s.load_state = RiseMapLoadState::Failed;
                    s.error_text = error_text;
                    true
                });
                if failed {
                    self.publish().await;
                }
            }
        }
    }

    pub async fn set_rule(&self, rule: MapPrivacyRule, kind: MapPrivacyKind) -> RiseMapMutationOutcome {
        let prepared = self.state.with(|s| {
            if s.is_shutdown {
                return Err(RiseMapMutationOutcome::Failed(None));
            }
            let Some(mut optimistic) = s.privacy.clone() else {
                return Err(RiseMapMutationOutcome::Failed(None));
            };
            if !kind.rules().contains(&rule) {
                return Err(RiseMapMutationOutcome::Failed(None));
            }
            if optimistic.rule(kind) == rule {
                return Err(RiseMapMutationOutcome::Done);
            }
            let previous = optimistic.clone();
            optimistic.set_rule(rule, kind);
            Ok((optimistic, previous))
        });
        let (optimistic, previous) = match prepared {
            Ok(prepared) => prepared,
            Err(outcome) => {
                if outcome != RiseMapMutationOutcome::Done {
                    tracing::warn!(target: "riseonly::X1174", kind = kind.raw_value(), rule = rule.wire(), "privacy rule refused locally");
                }
                return outcome;
            }
        };
        tracing::warn!(target: "riseonly::X1174", kind = kind.raw_value(), rule = rule.wire(), "privacy rule send");
        self.commit(optimistic, previous, self.rpc.update_privacy(kind, rule)).await
    }

    pub async fn set_users(
        &self,
        users: Vec<MapUser>,
        kind: MapPrivacyKind,
        action: MapPrivacyAction,
    ) -> RiseMapMutationOutcome {
        let user_ids: Vec<String> = users.iter().map(|user| user.id.clone()).collect();
        let prepared = self.state.with(|s| {
            if s.is_shutdown {
                return Err(RiseMapMutationOutcome::Failed(None));
            }
            let Some(mut optimistic) = s.privacy.clone() else {
                return Err(RiseMapMutationOutcome::Failed(None));
            };
            if optimistic.users(kind, action).iter().map(|user| &user.id).eq(user_ids.iter()) {
                return Err(RiseMapMutationOutcome::Done);
            }
            let previous = optimistic.clone();
            optimistic.set_users(users, kind, action);
            let opposite = if action == MapPrivacyAction::Allow { MapPrivacyAction::Deny } else { MapPrivacyAction::Allow };
            let chosen: HashSet<&String> = user_ids.iter().collect();
            let kept: Vec<MapUser> =
                optimistic.users(kind, opposite).iter().filter(|user| !chosen.contains(&user.id)).cloned().collect();
            optimistic.set_users(kept, kind, opposite);
            Ok((optimistic, previous))
        });
        let (optimistic, previous) = match prepared {
            Ok(prepared) => prepared,
            Err(outcome) => {
                if outcome != RiseMapMutationOutcome::Done {
                    tracing::warn!(target: "riseonly::X1174", kind = kind.raw_value(), action = action.raw_value(), "privacy list refused locally");
                }
                return outcome;
            }
        };
        tracing::warn!(
            target: "riseonly::X1174",
            kind = kind.raw_value(),
            action = action.raw_value(),
            count = user_ids.len(),
            "privacy list send"
        );
        self.commit(optimistic, previous, self.rpc.set_privacy_exceptions(kind, action, user_ids)).await
    }

    pub async fn reset(&self) {
        let snapshot = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.mutation_generation += 1;
            s.privacy = None;
            s.load_state = RiseMapLoadState::Idle;
            s.error_text = None;
            s.cache_source = RiseMapCacheSource::Miss;
            s.last_refresh_ms = 0;
            Some(s.next_snapshot())
        });
        if let Some(snapshot) = snapshot {
            self.deliver(snapshot).await;
        }
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| s.is_shutdown = true);
    }

    async fn commit(
        &self,
        optimistic: MapPrivacy,
        previous: MapPrivacy,
        send: impl Future<Output = anyhow::Result<MapPrivacy>>,
    ) -> RiseMapMutationOutcome {
        let generation = self.state.with(|s| {
            s.mutation_generation += 1;
            s.privacy = Some(optimistic);
            s.mutation_generation
        });
        let mut rollback =
            RiseMapPrivacyRollback { repository: self, generation, previous: Some(previous.clone()) };
        self.publish().await;

        let result = send.await;
        rollback.disarm();
        match &result {
            Ok(_) => tracing::warn!(target: "riseonly::X1174", generation, "privacy edit confirmed"),
            Err(error) => tracing::warn!(target: "riseonly::X1174", generation, error = %format!("{error:#}"), "privacy edit failed"),
        }
        match result {
            Ok(confirmed) => {
                let Some(is_current) =
                    self.state.with(|s| (!s.is_shutdown).then_some(generation == s.mutation_generation))
                else {
                    return RiseMapMutationOutcome::Failed(None);
                };
                if is_current {
                    self.accept(confirmed).await;
                }
                RiseMapMutationOutcome::Done
            }
            Err(error) => {
                let Some(rolled_back) = self.state.with(|s| {
                    if s.is_shutdown {
                        return None;
                    }
                    if generation != s.mutation_generation {
                        return Some(false);
                    }
                    s.privacy = Some(previous);
                    Some(true)
                }) else {
                    return RiseMapMutationOutcome::Failed(None);
                };
                if rolled_back {
                    self.publish().await;
                }
                RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error))
            }
        }
    }

    async fn accept(&self, fresh: MapPrivacy) {
        let stored = fresh.clone();
        let changed = self.state.with(|s| {
            let changed = s.privacy.as_ref() != Some(&fresh);
            s.privacy = Some(fresh);
            s.load_state = RiseMapLoadState::Loaded;
            s.error_text = None;
            s.cache_source = RiseMapCacheSource::Network;
            changed
        });
        if changed {
            self.resources.store(RiseMapResourceRepositoryResource::Privacy, stored).await;
        }
        self.publish().await;
    }

    async fn publish(&self) {
        let snapshot = self.state.with(RiseMapPrivacyRepositoryState::next_snapshot);
        self.deliver(snapshot).await;
    }

    async fn deliver(&self, snapshot: RiseMapPrivacySnapshot) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let identity = self.identity.clone();
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| Self::publish_on_main(snapshot, &identity, cx));
                    Ok::<_, anyhow::Error>(())
                }
                .boxed_local()
            })
            .await;
    }

    fn post_snapshot(snapshot: RiseMapPrivacySnapshot, identity: RiseMapSessionIdentity) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        hop.post(move |cx: &mut App| Self::publish_on_main(snapshot, &identity, cx));
    }

    fn publish_on_main(snapshot: RiseMapPrivacySnapshot, identity: &RiseMapSessionIdentity, cx: &mut App) {
        RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.publish_privacy(snapshot, identity, cx);
        });
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}
