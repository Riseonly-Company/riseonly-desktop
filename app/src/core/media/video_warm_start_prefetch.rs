use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::channel::mpsc;
use futures::future::{AbortHandle, AbortRegistration, Abortable, LocalBoxFuture};
use futures::{FutureExt, StreamExt};
use gpui::{App, AppContext as _, AsyncApp, Entity, Global, Task};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::video::av_player::{AVPlayerItem, AVQueuePlayer, player_support};
use rise_platform::power::{POWER_STATE_DID_CHANGE, system_low_power_mode};
use rise_platform::process_info::THERMAL_STATE_DID_CHANGE;

use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication,
};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::RemoteMediaDiskCache;
use crate::core::media::rise_media_transfer_hub::RiseMediaCacheIdentity;
use crate::core::ui::animated_media_playback_coordinator::AnimatedMediaThermalLevel;

pub enum VideoWarmStartPolicy {}

impl VideoWarmStartPolicy {
    pub const PREFERRED_BUFFER_DURATION: Duration = Duration::from_secs(8);

    pub fn allows_prefetch(
        is_application_active: bool,
        is_low_power_mode_enabled: bool,
        thermal_level: AnimatedMediaThermalLevel,
    ) -> bool {
        if !is_application_active || is_low_power_mode_enabled {
            return false;
        }
        match thermal_level {
            AnimatedMediaThermalLevel::Nominal | AnimatedMediaThermalLevel::Fair => true,
            AnimatedMediaThermalLevel::Serious | AnimatedMediaThermalLevel::Critical => false,
        }
    }
}

/// one warmed player
/// cloning shares the same player entity so whoever
/// takes it plays the player the coordinator prepared
#[derive(Clone)]
pub struct VideoWarmPlayerBox {
    pub source: Url,
    pub player: Entity<AVQueuePlayer>,
    pub item: AVPlayerItem,
}

impl VideoWarmPlayerBox {
    pub fn new(source: Url, player: Entity<AVQueuePlayer>, item: AVPlayerItem) -> Self {
        Self { source, player, item }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VideoWarmStartCoordinatorRequest {
    pub url_string: String,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VideoWarmStartCoordinatorOwner {
    pub scope: String,
    pub generation: u64,
}

pub type VideoWarmStartCoordinatorOperation = Arc<
    dyn Fn(VideoWarmStartCoordinatorRequest, AsyncApp) -> LocalBoxFuture<'static, Option<VideoWarmPlayerBox>>
        + Send
        + Sync,
>;

struct VideoWarmStartCoordinatorPendingJob {
    owners: HashSet<VideoWarmStartCoordinatorOwner>,
}

struct VideoWarmStartCoordinatorActiveJob {
    owners: HashSet<VideoWarmStartCoordinatorOwner>,
    task_id: u64,
    task: AbortHandle,
    cancellation_requested: bool,
}

struct VideoWarmStartCoordinatorPreparedEntry {
    player_box: VideoWarmPlayerBox,
    owners: HashSet<VideoWarmStartCoordinatorOwner>,
    prepared_at: TimeInterval,
}

#[derive(Default)]
struct VideoWarmStartCoordinatorState {
    pending_order: Vec<VideoWarmStartCoordinatorRequest>,
    pending_by_request: HashMap<VideoWarmStartCoordinatorRequest, VideoWarmStartCoordinatorPendingJob>,
    active_by_request: HashMap<VideoWarmStartCoordinatorRequest, VideoWarmStartCoordinatorActiveJob>,
    prepared_by_request: HashMap<VideoWarmStartCoordinatorRequest, VideoWarmStartCoordinatorPreparedEntry>,
    prepared_order: Vec<VideoWarmStartCoordinatorRequest>,
    latest_generation_by_scope: HashMap<String, u64>,
    scope_order: Vec<String>,
    next_task_id: u64,
}

type VideoWarmStartCoordinatorStart = (VideoWarmStartCoordinatorRequest, u64, AbortRegistration);

/// video warm start state, one lock and every method a short critical section
pub struct VideoWarmStartCoordinator {
    maximum_pending: usize,
    maximum_prepared: usize,
    maximum_tracked_scopes: usize,
    prepared_lifetime: TimeInterval,
    operation: VideoWarmStartCoordinatorOperation,
    state: RiseIsolated<VideoWarmStartCoordinatorState>,
}

impl VideoWarmStartCoordinator {
    pub const DEFAULT_MAXIMUM_PENDING: usize = 4;
    pub const DEFAULT_MAXIMUM_PREPARED: usize = 3;
    pub const DEFAULT_MAXIMUM_TRACKED_SCOPES: usize = 16;
    pub const DEFAULT_PREPARED_LIFETIME: TimeInterval = 60.0;

    /// none takes the default for that argument
    pub fn new(
        maximum_pending: Option<usize>,
        maximum_prepared: Option<usize>,
        maximum_tracked_scopes: Option<usize>,
        prepared_lifetime: Option<TimeInterval>,
        operation: VideoWarmStartCoordinatorOperation,
    ) -> Arc<Self> {
        Arc::new(Self {
            maximum_pending: maximum_pending.unwrap_or(Self::DEFAULT_MAXIMUM_PENDING).max(1),
            maximum_prepared: maximum_prepared.unwrap_or(Self::DEFAULT_MAXIMUM_PREPARED).max(1),
            maximum_tracked_scopes: maximum_tracked_scopes
                .unwrap_or(Self::DEFAULT_MAXIMUM_TRACKED_SCOPES)
                .max(1),
            prepared_lifetime: prepared_lifetime.unwrap_or(Self::DEFAULT_PREPARED_LIFETIME).max(1.0),
            operation,
            state: RiseIsolated::new(VideoWarmStartCoordinatorState::default()),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace_directional(
        self: &Arc<Self>,
        url_strings: &[String],
        scope: &str,
        generation: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &AsyncApp,
    ) {
        let requests = self.requests(url_strings, account_user_id.as_deref(), account_epoch.as_deref());
        let start = self.state.with(|state| {
            if !self.accept(state, generation, scope) {
                return None;
            }
            Self::remove_owners(state, scope);
            let owner = VideoWarmStartCoordinatorOwner {
                scope: scope.to_owned(),
                generation,
            };
            for request in requests {
                self.add(state, request, owner.clone());
            }
            self.prune_prepared(state);
            Self::pump(state)
        });
        self.start(start, cx);
    }

    pub async fn cancel_directional(self: &Arc<Self>, scope: &str, generation: u64, cx: &AsyncApp) {
        let start = self.state.with(|state| {
            if !self.accept(state, generation, scope) {
                return None;
            }
            Self::remove_owners(state, scope);
            Self::pump(state)
        });
        self.start(start, cx);
    }

    pub async fn take_prepared(
        &self,
        url_string: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) -> Option<VideoWarmPlayerBox> {
        self.state.with(|state| {
            self.prune_prepared(state);
            let normalized = Self::normalized_http_url(url_string)?;
            let request = VideoWarmStartCoordinatorRequest {
                url_string: normalized,
                account_user_id,
                account_epoch,
            };
            let Some(prepared) = state.prepared_by_request.remove(&request) else {
                if let Some(active) = state.active_by_request.get_mut(&request)
                    && !active.cancellation_requested
                {
                    active.owners.clear();
                    active.cancellation_requested = true;
                    active.task.abort();
                }
                state.pending_by_request.remove(&request);
                state.pending_order.retain(|pending| *pending != request);
                return None;
            };
            state.prepared_order.retain(|prepared| *prepared != request);
            Some(prepared.player_box)
        })
    }

    pub async fn reset(&self) {
        self.state.with(|state| {
            state.pending_order = Vec::new();
            state.pending_by_request = HashMap::new();
            state.latest_generation_by_scope = HashMap::new();
            state.scope_order = Vec::new();
            for active in state.active_by_request.values_mut() {
                active.owners.clear();
                active.cancellation_requested = true;
                active.task.abort();
            }
            state.prepared_by_request = HashMap::new();
            state.prepared_order = Vec::new();
        });
    }

    pub async fn pending_count_for_testing(&self) -> usize {
        self.state.with(|state| state.pending_by_request.len())
    }

    pub async fn prepared_count_for_testing(&self) -> usize {
        self.state.with(|state| state.prepared_by_request.len())
    }

    fn accept(&self, state: &mut VideoWarmStartCoordinatorState, generation: u64, scope: &str) -> bool {
        if generation < state.latest_generation_by_scope.get(scope).copied().unwrap_or(0) {
            return false;
        }
        state.latest_generation_by_scope.insert(scope.to_owned(), generation);
        state.scope_order.retain(|tracked| tracked != scope);
        state.scope_order.push(scope.to_owned());
        while state.scope_order.len() > self.maximum_tracked_scopes {
            let evicted = state.scope_order.remove(0);
            state.latest_generation_by_scope.remove(&evicted);
            Self::remove_owners(state, &evicted);
        }
        true
    }

    fn requests(
        &self,
        url_strings: &[String],
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Vec<VideoWarmStartCoordinatorRequest> {
        let mut seen = HashSet::new();
        let mut result = Vec::with_capacity(url_strings.len().min(self.maximum_pending));
        for raw in url_strings {
            let Some(url_string) = Self::normalized_http_url(raw) else {
                continue;
            };
            if !seen.insert(url_string.clone()) {
                continue;
            }
            result.push(VideoWarmStartCoordinatorRequest {
                url_string,
                account_user_id: account_user_id.map(str::to_owned),
                account_epoch: account_epoch.map(str::to_owned),
            });
            if result.len() == self.maximum_pending {
                break;
            }
        }
        result
    }

    fn add(
        &self,
        state: &mut VideoWarmStartCoordinatorState,
        request: VideoWarmStartCoordinatorRequest,
        owner: VideoWarmStartCoordinatorOwner,
    ) {
        if let Some(prepared) = state.prepared_by_request.get_mut(&request) {
            prepared.owners.insert(owner);
            state.prepared_order.retain(|prepared| *prepared != request);
            state.prepared_order.push(request);
            return;
        }
        if let Some(active) = state.active_by_request.get_mut(&request)
            && !active.cancellation_requested
        {
            active.owners.insert(owner);
            return;
        }
        if let Some(pending) = state.pending_by_request.get_mut(&request) {
            pending.owners.insert(owner);
            return;
        }
        if state.pending_by_request.len() >= self.maximum_pending {
            return;
        }
        state.pending_by_request.insert(
            request.clone(),
            VideoWarmStartCoordinatorPendingJob {
                owners: HashSet::from([owner]),
            },
        );
        state.pending_order.push(request);
    }

    fn remove_owners(state: &mut VideoWarmStartCoordinatorState, scope: &str) {
        state.pending_by_request.retain(|_, pending| {
            pending.owners.retain(|owner| owner.scope != scope);
            !pending.owners.is_empty()
        });
        let pending_by_request = &state.pending_by_request;
        state
            .pending_order
            .retain(|request| pending_by_request.contains_key(request));

        for active in state.active_by_request.values_mut() {
            active.owners.retain(|owner| owner.scope != scope);
            if active.owners.is_empty() && !active.cancellation_requested {
                active.cancellation_requested = true;
                active.task.abort();
            }
        }

        state.prepared_by_request.retain(|_, prepared| {
            prepared.owners.retain(|owner| owner.scope != scope);
            !prepared.owners.is_empty()
        });
        let prepared_by_request = &state.prepared_by_request;
        state
            .prepared_order
            .retain(|request| prepared_by_request.contains_key(request));
    }

    fn pump(state: &mut VideoWarmStartCoordinatorState) -> Option<VideoWarmStartCoordinatorStart> {
        if !state.active_by_request.is_empty() {
            return None;
        }
        while !state.pending_order.is_empty() {
            let request = state.pending_order.remove(0);
            let Some(pending) = state.pending_by_request.remove(&request) else {
                continue;
            };
            if pending.owners.is_empty() {
                continue;
            }
            state.next_task_id = state.next_task_id.wrapping_add(1);
            let task_id = state.next_task_id;
            let (task, registration) = AbortHandle::new_pair();
            state.active_by_request.insert(
                request.clone(),
                VideoWarmStartCoordinatorActiveJob {
                    owners: pending.owners,
                    task_id,
                    task,
                    cancellation_requested: false,
                },
            );
            return Some((request, task_id, registration));
        }
        None
    }

    fn start(self: &Arc<Self>, start: Option<VideoWarmStartCoordinatorStart>, cx: &AsyncApp) {
        let Some((request, task_id, registration)) = start else {
            return;
        };
        let coordinator = Arc::clone(self);
        let operation = Arc::clone(&self.operation);
        cx.spawn(async move |cx| {
            let work = operation(request.clone(), cx.clone());
            let player_box = Abortable::new(work, registration).await.ok().flatten();
            coordinator.finish(request, task_id, player_box, cx);
        })
        .detach();
    }

    fn finish(
        self: &Arc<Self>,
        request: VideoWarmStartCoordinatorRequest,
        task_id: u64,
        player_box: Option<VideoWarmPlayerBox>,
        cx: &AsyncApp,
    ) {
        let start = self.state.with(|state| {
            if state.active_by_request.get(&request).map(|active| active.task_id) != Some(task_id) {
                return None;
            }
            let active = state.active_by_request.remove(&request)?;
            if let Some(player_box) = player_box
                && !active.cancellation_requested
                && !active.owners.is_empty()
            {
                state.prepared_by_request.insert(
                    request.clone(),
                    VideoWarmStartCoordinatorPreparedEntry {
                        player_box,
                        owners: active.owners,
                        prepared_at: RiseClock::now().time_interval_since_1970(),
                    },
                );
                state.prepared_order.retain(|prepared| *prepared != request);
                state.prepared_order.push(request);
                self.prune_prepared(state);
            }
            Self::pump(state)
        });
        self.start(start, cx);
    }

    fn prune_prepared(&self, state: &mut VideoWarmStartCoordinatorState) {
        let cutoff = RiseClock::now().time_interval_since_1970() - self.prepared_lifetime;
        for request in &state.prepared_order {
            let prepared_at = state
                .prepared_by_request
                .get(request)
                .map_or(0.0, |prepared| prepared.prepared_at);
            if prepared_at < cutoff {
                state.prepared_by_request.remove(request);
            }
        }
        let prepared_by_request = &state.prepared_by_request;
        state
            .prepared_order
            .retain(|request| prepared_by_request.contains_key(request));
        while state.prepared_order.len() > self.maximum_prepared {
            let request = state.prepared_order.remove(0);
            state.prepared_by_request.remove(&request);
        }
    }

    fn normalized_http_url(raw: &str) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw);
        let url = Url::from_string(value)?;
        let scheme = url.scheme()?.to_lowercase();
        if scheme != "https" && scheme != "http" {
            return None;
        }
        Some(value.to_owned())
    }
}

pub enum VideoWarmStartPrefetch {}

static COORDINATOR: LazyLock<Arc<VideoWarmStartCoordinator>> = LazyLock::new(|| {
    VideoWarmStartCoordinator::new(
        None,
        None,
        None,
        None,
        Arc::new(|request: VideoWarmStartCoordinatorRequest, cx: AsyncApp| {
            async move { VideoWarmStartPrefetch::prepare(request, cx).await }.boxed_local()
        }),
    )
});

struct VideoWarmStartPrefetchLifecycleObservers {
    _notifications: Vec<RiseSubscription>,
    _delivery: Task<()>,
}

impl Global for VideoWarmStartPrefetchLifecycleObservers {}

impl VideoWarmStartPrefetch {
    pub fn replace_directional(
        urls: &[String],
        scope: &str,
        generation: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut App,
    ) {
        Self::install_lifecycle_observers(cx);
        let reduces_prefetch = RiseEnergySavingStore::shared(cx)
            .read(cx)
            .reduces(RiseEnergySavingFeature::MediaPrefetch);
        if !VideoWarmStartPolicy::allows_prefetch(
            RiseAppLifecycle::scene_phase(cx) == ScenePhase::Active,
            system_low_power_mode() || reduces_prefetch,
            Self::current_thermal_level(),
        ) {
            Self::cancel_directional(scope, generation, cx);
            return;
        }
        let urls = urls.to_vec();
        let scope = scope.to_owned();
        cx.spawn(async move |cx| {
            COORDINATOR
                .replace_directional(&urls, &scope, generation, account_user_id, account_epoch, cx)
                .await;
        })
        .detach();
    }

    pub fn cancel_directional(scope: &str, generation: u64, cx: &mut App) {
        let scope = scope.to_owned();
        cx.spawn(async move |cx| {
            COORDINATOR.cancel_directional(&scope, generation, cx).await;
        })
        .detach();
    }

    pub async fn take_prepared_player(
        url: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) -> Option<VideoWarmPlayerBox> {
        COORDINATOR.take_prepared(url, account_user_id, account_epoch).await
    }

    pub fn reset_account_state() {
        let coordinator = Arc::clone(&COORDINATOR);
        RiseAccountRuntime::current().spawn(async move {
            coordinator.reset().await;
        });
    }

    fn install_lifecycle_observers(cx: &mut App) {
        if cx.has_global::<VideoWarmStartPrefetchLifecycleObservers>() {
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let names = [
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
            UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION,
            RiseNotificationName::new(POWER_STATE_DID_CHANGE),
            RiseNotificationName::new(THERMAL_STATE_DID_CHANGE),
        ];
        let notifications = names
            .into_iter()
            .map(|name| {
                let sender = sender.clone();
                RiseAppEvents::default_center().observe(name, move |_| {
                    let _ = sender.unbounded_send(());
                })
            })
            .collect();
        let delivery = cx.spawn(async move |_| {
            while receiver.next().await.is_some() {
                COORDINATOR.reset().await;
            }
        });
        cx.set_global(VideoWarmStartPrefetchLifecycleObservers {
            _notifications: notifications,
            _delivery: delivery,
        });
    }

    fn current_thermal_level() -> AnimatedMediaThermalLevel {
        AnimatedMediaThermalLevel::current()
    }

    async fn prepare(request: VideoWarmStartCoordinatorRequest, cx: AsyncApp) -> Option<VideoWarmPlayerBox> {
        if !SaiAccountCacheScope::is_current_account_epoch(
            request.account_user_id.as_deref(),
            request.account_epoch.as_deref(),
        ) {
            return None;
        }
        let source = Url::from_string(&request.url_string)?;

        let cache_key = RiseMediaCacheIdentity::key(&source);
        let probe_source = source.clone();
        let account_user_id = request.account_user_id.clone();
        let account_epoch = request.account_epoch.clone();
        let is_cached = cx
            .background_spawn(async move {
                RemoteMediaDiskCache::shared()
                    .existing_local_url(
                        &probe_source,
                        Some(&cache_key),
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                    )
                    .is_some()
            })
            .await;
        if is_cached {
            return None;
        }

        if !player_support().is_supported() {
            return None;
        }
        let mut item = AVPlayerItem::new(source.clone());
        item.set_preferred_forward_buffer_duration(VideoWarmStartPolicy::PREFERRED_BUFFER_DURATION);
        let player_item = item.clone();
        let player = cx.update(|cx| cx.new(|cx| AVQueuePlayer::new(player_item, cx)));
        let is_playable = cx.update(|cx| player.read(cx).error().is_none());
        if !is_playable
            || !SaiAccountCacheScope::is_current_account_epoch(
                request.account_user_id.as_deref(),
                request.account_epoch.as_deref(),
            )
        {
            return None;
        }
        cx.update(|cx| {
            player.update(cx, |player, _| {
                player.set_is_muted(true);
                player.set_automatically_waits_to_minimize_stalling(true);
            });
        });
        Some(VideoWarmPlayerBox::new(source, player, item))
    }
}

#[cfg(test)]
#[path = "video_warm_start_prefetch_tests.rs"]
mod tests;
