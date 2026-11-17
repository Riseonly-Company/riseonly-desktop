use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AsyncApp};
use rise_canon::RiseTimingCanon;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use tokio::task::AbortHandle;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::rise_map_resource_repository::{
    RiseMapFailure, RiseMapResourceRepository, RiseMapResourceRepositoryResource,
};
use crate::modules::map::engine::core::rise_map_rpc::RiseMapRPC;
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapCacheSource, RiseMapLoadState, RiseMapMutationOutcome, RiseMapStateSnapshot,
};
use crate::modules::map::engine::rise_map_presentation::{RiseMapPresentation, RiseMapSessionIdentity};
use crate::modules::map::stores::map_actions::map_types::{MapSettingsPatch, MapState};

type RiseMapStateChangedHandler = Arc<dyn Fn(MapState) -> BoxFuture<'static, ()> + Send + Sync>;

/// the running refresh
/// done resolves when it ends without cancelling it
/// abort cancels it
struct RiseMapStateRefreshTask {
    token: u64,
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

struct RiseMapStateRepositoryState {
    state: Option<MapState>,
    is_joining: bool,
    load_state: RiseMapLoadState,
    error_text: Option<String>,
    cache_source: RiseMapCacheSource,
    revision: u64,
    did_read_store: bool,
    last_refresh_ms: i64,
    refresh_task: Option<RiseMapStateRefreshTask>,
    refresh_token: u64,
    mutation_generation: u64,
    on_state_changed: Option<RiseMapStateChangedHandler>,
    is_shutdown: bool,
}

impl RiseMapStateRepositoryState {
    fn next_snapshot(&mut self) -> RiseMapStateSnapshot {
        self.revision += 1;
        RiseMapStateSnapshot {
            state: self.state.clone(),
            is_joining: self.is_joining,
            load_state: self.load_state,
            error_text: self.error_text.clone(),
            cache_source: self.cache_source,
            revision: self.revision,
        }
    }
}

static REFRESH_INTERVAL_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("map_state"));

struct RiseMapStateJoinGuard<'a> {
    repository: &'a RiseMapStateRepository,
    is_armed: bool,
}

impl Drop for RiseMapStateJoinGuard<'_> {
    fn drop(&mut self) {
        if !self.is_armed {
            return;
        }
        let snapshot = self.repository.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.is_joining = false;
            Some(s.next_snapshot())
        });
        if let Some(snapshot) = snapshot {
            RiseMapStateRepository::post_snapshot(snapshot, self.repository.identity.clone());
        }
    }
}

struct RiseMapStateSettingsRollback<'a> {
    repository: &'a RiseMapStateRepository,
    generation: u64,
    previous: Option<MapState>,
}

impl Drop for RiseMapStateSettingsRollback<'_> {
    fn drop(&mut self) {
        let Some(previous) = self.previous.take() else {
            return;
        };
        let generation = self.generation;
        let rolled_back = self.repository.state.with(|s| {
            if s.is_shutdown || generation != s.mutation_generation {
                return None;
            }
            s.state = Some(previous.clone());
            Some((s.next_snapshot(), s.on_state_changed.clone()))
        });
        let Some((snapshot, on_state_changed)) = rolled_back else {
            return;
        };
        RiseMapStateRepository::post_snapshot(snapshot, self.repository.identity.clone());
        if let Some(on_state_changed) = on_state_changed {
            RiseAccountRuntime::current().spawn(on_state_changed(previous));
        }
    }
}

/// membership and settings of the map
/// one small resource
/// cache first from the encrypted store and revalidated inside the canon freshness window
pub struct RiseMapStateRepository {
    rpc: RiseMapRPC,
    resources: Arc<RiseMapResourceRepository>,
    identity: RiseMapSessionIdentity,
    state: RiseIsolated<RiseMapStateRepositoryState>,
}

impl RiseMapStateRepository {
    pub fn new(rpc: RiseMapRPC, resources: Arc<RiseMapResourceRepository>, identity: RiseMapSessionIdentity) -> Self {
        Self {
            rpc,
            resources,
            identity,
            state: RiseIsolated::new(RiseMapStateRepositoryState {
                state: None,
                is_joining: false,
                load_state: RiseMapLoadState::Idle,
                error_text: None,
                cache_source: RiseMapCacheSource::Miss,
                revision: 0,
                did_read_store: false,
                last_refresh_ms: 0,
                refresh_task: None,
                refresh_token: 0,
                mutation_generation: 0,
                on_state_changed: None,
                is_shutdown: false,
            }),
        }
    }

    pub async fn is_member(&self) -> bool {
        self.state.with(|s| s.state.as_ref().is_some_and(|state| state.is_member))
    }

    pub async fn current(&self) -> Option<MapState> {
        self.state.with(|s| s.state.clone())
    }

    pub async fn set_on_state_changed(
        &self,
        handler: impl Fn(MapState) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    ) {
        let handler: RiseMapStateChangedHandler = Arc::new(handler);
        self.state.with(|s| s.on_state_changed = Some(handler));
    }

    pub async fn hydrate_from_store(&self) {
        let reads_store = self.state.with(|s| {
            if s.is_shutdown || s.did_read_store {
                return false;
            }
            s.did_read_store = true;
            s.state.is_none()
        });
        if !reads_store {
            return;
        }
        let Some(cached) = self.resources.cached::<MapState>(RiseMapResourceRepositoryResource::State).await else {
            return;
        };
        let applied = self.state.with(|s| {
            if s.is_shutdown || s.state.is_some() {
                return None;
            }
            s.state = Some(cached.clone());
            s.cache_source = RiseMapCacheSource::PersistentStore;
            s.load_state = RiseMapLoadState::Loaded;
            Some(s.next_snapshot())
        });
        let Some(snapshot) = applied else {
            return;
        };
        self.deliver(snapshot).await;
        self.notify_state_changed(cached).await;
    }

    pub async fn open(self: &Arc<Self>, force: bool) {
        self.hydrate_from_store().await;
        let now_ms = Self::now_milliseconds();
        let needs_refresh = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            let is_stale = now_ms - s.last_refresh_ms >= *REFRESH_INTERVAL_MS;
            force || is_stale || s.state.is_none()
        });
        if !needs_refresh {
            return;
        }
        self.refresh().await;
    }

    pub async fn refresh(self: &Arc<Self>) {
        let this = self.clone();
        let step = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            if let Some(refresh_task) = s.refresh_task.as_ref() {
                return Some((None, refresh_task.done.clone()));
            }
            s.refresh_token = s.refresh_token.wrapping_add(1);
            let token = s.refresh_token;
            let handle = RiseAccountRuntime::current().spawn(async move {
                this.perform_refresh().await;
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.refresh_task = Some(RiseMapStateRefreshTask { token, abort, done: done.clone() });
            Some((Some(token), done))
        });
        let Some((started, done)) = step else {
            return;
        };
        done.await;
        if let Some(token) = started {
            self.state.with(|s| {
                if s.refresh_task.as_ref().is_some_and(|task| task.token == token) {
                    s.refresh_task = None;
                }
            });
        }
    }

    pub async fn join(&self) -> RiseMapMutationOutcome {
        let Some(snapshot) = self.state.with(|s| {
            if s.is_shutdown || s.is_joining {
                return None;
            }
            s.is_joining = true;
            s.error_text = None;
            s.mutation_generation += 1;
            Some(s.next_snapshot())
        }) else {
            return RiseMapMutationOutcome::Failed(None);
        };
        let mut joining = RiseMapStateJoinGuard { repository: self, is_armed: true };
        self.deliver(snapshot).await;

        let result = self.rpc.join().await;
        joining.is_armed = false;
        match result {
            Ok(joined) => {
                let is_running = self.state.with(|s| {
                    if s.is_shutdown {
                        return false;
                    }
                    s.is_joining = false;
                    true
                });
                if !is_running {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.accept(joined).await;
                RiseMapMutationOutcome::Done
            }
            Err(error) => {
                let Some(snapshot) = self.state.with(|s| {
                    if s.is_shutdown {
                        return None;
                    }
                    s.is_joining = false;
                    Some(s.next_snapshot())
                }) else {
                    return RiseMapMutationOutcome::Failed(None);
                };
                let text = RiseMapFailure::text(&error);
                self.deliver(snapshot).await;
                RiseMapMutationOutcome::Failed(text)
            }
        }
    }

    pub async fn leave(&self) -> RiseMapMutationOutcome {
        let is_running = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.mutation_generation += 1;
            true
        });
        if !is_running {
            return RiseMapMutationOutcome::Failed(None);
        }
        match self.rpc.leave().await {
            Ok(left) => {
                if self.state.with(|s| s.is_shutdown) {
                    return RiseMapMutationOutcome::Failed(None);
                }
                self.resources.remove_all().await;
                self.accept(left).await;
                RiseMapMutationOutcome::Done
            }
            Err(error) => RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error)),
        }
    }

    pub async fn update_settings(&self, patch: MapSettingsPatch) -> RiseMapMutationOutcome {
        let prepared = self.state.with(|s| {
            if s.is_shutdown {
                tracing::warn!(target: "riseonly::X1453", "map settings update on a shut down repository");
                return Err(RiseMapMutationOutcome::Failed(None));
            }
            let Some(mut optimistic) = s.state.clone().filter(|state| state.is_member) else {
                tracing::warn!(target: "riseonly::X1453", has_state = s.state.is_some(), "map settings update without member state");
                return Err(RiseMapMutationOutcome::Failed(None));
            };
            let previous = optimistic.clone();
            optimistic.settings = patch.applied(&optimistic.settings);
            if optimistic == previous {
                return Err(RiseMapMutationOutcome::Done);
            }
            s.mutation_generation += 1;
            s.state = Some(optimistic.clone());
            Ok((s.mutation_generation, optimistic, previous, s.next_snapshot()))
        });
        let (generation, optimistic, previous, snapshot) = match prepared {
            Ok(prepared) => prepared,
            Err(outcome) => return outcome,
        };
        let mut rollback =
            RiseMapStateSettingsRollback { repository: self, generation, previous: Some(previous.clone()) };
        self.deliver(snapshot).await;
        self.notify_state_changed(optimistic).await;

        let result = self.rpc.update_settings(&patch).await;
        rollback.previous = None;
        match result {
            Ok(confirmed) => {
                let Some(is_current) =
                    self.state.with(|s| (!s.is_shutdown).then_some(generation == s.mutation_generation))
                else {
                    tracing::warn!(target: "riseonly::X1453", "map settings answer after repository shutdown");
                    return RiseMapMutationOutcome::Failed(None);
                };
                if is_current {
                    self.accept(confirmed).await;
                }
                RiseMapMutationOutcome::Done
            }
            Err(error) => {
                tracing::warn!(target: "riseonly::X1453", error = %format!("{error:#}"), "map settings update refused");
                let Some(rolled_back) = self.state.with(|s| {
                    if s.is_shutdown {
                        return None;
                    }
                    if generation != s.mutation_generation {
                        return Some(None);
                    }
                    s.state = Some(previous.clone());
                    Some(Some(s.next_snapshot()))
                }) else {
                    return RiseMapMutationOutcome::Failed(None);
                };
                if let Some(snapshot) = rolled_back {
                    self.deliver(snapshot).await;
                    self.notify_state_changed(previous).await;
                }
                RiseMapMutationOutcome::Failed(RiseMapFailure::text(&error))
            }
        }
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            if let Some(refresh_task) = s.refresh_task.take() {
                refresh_task.abort.abort();
            }
        });
    }

    async fn perform_refresh(&self) {
        let (generation, loading) = self.state.with(|s| {
            let loading = if s.state.is_none() {
                s.load_state = RiseMapLoadState::Loading;
                Some(s.next_snapshot())
            } else {
                None
            };
            (s.mutation_generation, loading)
        });
        if let Some(snapshot) = loading {
            self.deliver(snapshot).await;
        }

        match self.rpc.get_state().await {
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
                    if s.is_shutdown || generation != s.mutation_generation || s.state.is_some() {
                        return None;
                    }
                    s.load_state = RiseMapLoadState::Failed;
                    s.error_text = error_text;
                    Some(s.next_snapshot())
                });
                if let Some(snapshot) = failed {
                    self.deliver(snapshot).await;
                }
            }
        }
    }

    async fn accept(&self, fresh: MapState) {
        let (changed, snapshot) = self.state.with(|s| {
            let changed = s.state.as_ref() != Some(&fresh)
                || s.load_state != RiseMapLoadState::Loaded
                || s.error_text.is_some();
            s.state = Some(fresh.clone());
            s.load_state = RiseMapLoadState::Loaded;
            s.error_text = None;
            s.cache_source = RiseMapCacheSource::Network;
            (changed, s.next_snapshot())
        });
        if changed {
            self.resources.store(RiseMapResourceRepositoryResource::State, fresh.clone()).await;
        }
        self.deliver(snapshot).await;
        if changed {
            self.notify_state_changed(fresh).await;
        }
    }

    async fn notify_state_changed(&self, state: MapState) {
        let Some(on_state_changed) = self.state.with(|s| s.on_state_changed.clone()) else {
            return;
        };
        on_state_changed(state).await;
    }

    async fn deliver(&self, snapshot: RiseMapStateSnapshot) {
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

    fn post_snapshot(snapshot: RiseMapStateSnapshot, identity: RiseMapSessionIdentity) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        hop.post(move |cx: &mut App| Self::publish_on_main(snapshot, &identity, cx));
    }

    fn publish_on_main(snapshot: RiseMapStateSnapshot, identity: &RiseMapSessionIdentity, cx: &mut App) {
        RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.publish_state(snapshot, identity, cx);
        });
    }

    fn now_milliseconds() -> i64 {
        RiseClock::now().ms()
    }
}
