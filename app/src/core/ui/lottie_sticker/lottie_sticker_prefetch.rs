use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::FutureExt;
use futures::future::{AbortHandle, AbortRegistration, Abortable, LocalBoxFuture};
use gpui::{App, AsyncApp, Pixels, Size, px};
use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{MediaPrefetchEnvironment, MediaPrefetchEnvironmentSnapshot};
use crate::core::ui::lottie_sticker::lottie_sticker_loader::{LottieStickerLoader, LottieStickerPayload};
use crate::core::ui::lottie_sticker::rise_lottie_frame_cache::RiseLottieFrameCache;
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieFirstFrameCache;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LottieStickerPrefetchQueueRequest {
    pub url_string: String,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LottieStickerPrefetchQueueOwner {
    Persistent,
    Directional { scope: String, generation: u64 },
}

pub type LottieStickerPrefetchQueueOperation =
    Arc<dyn Fn(LottieStickerPrefetchQueueRequest, AsyncApp) -> LocalBoxFuture<'static, ()> + Send + Sync>;

struct LottieStickerPrefetchQueuePendingJob {
    owners: HashSet<LottieStickerPrefetchQueueOwner>,
}

struct LottieStickerPrefetchQueueActiveJob {
    owners: HashSet<LottieStickerPrefetchQueueOwner>,
    task_id: u64,
    task: AbortHandle,
    cancellation_requested: bool,
}

struct LottieStickerPrefetchQueueState {
    pending_order: Vec<LottieStickerPrefetchQueueRequest>,
    pending_by_request: HashMap<LottieStickerPrefetchQueueRequest, LottieStickerPrefetchQueuePendingJob>,
    active_by_request: HashMap<LottieStickerPrefetchQueueRequest, LottieStickerPrefetchQueueActiveJob>,
    successor_by_request: HashMap<LottieStickerPrefetchQueueRequest, LottieStickerPrefetchQueuePendingJob>,
    latest_generation_by_scope: HashMap<String, u64>,
    scope_order: Vec<String>,
    next_task_id: u64,
    environment_generation: u64,
    environment_snapshot: Option<MediaPrefetchEnvironmentSnapshot>,
}

type LottieStickerPrefetchQueueStart = (
    LottieStickerPrefetchQueueRequest,
    u64,
    AbortRegistration,
    Option<MediaPrefetchEnvironmentSnapshot>,
);

/// queue behind one lock, every method is a short critical section
pub struct LottieStickerPrefetchQueue {
    maximum_concurrent: usize,
    maximum_pending: usize,
    maximum_tracked_scopes: usize,
    operation: LottieStickerPrefetchQueueOperation,
    state: RiseIsolated<LottieStickerPrefetchQueueState>,
}

impl LottieStickerPrefetchQueue {
    pub fn default_maximum_pending() -> i64 {
        RiseLimits::client_value("lottie_prefetch_queue_pending")
    }

    pub fn default_maximum_tracked_scopes() -> i64 {
        RiseLimits::client_value("lottie_prefetch_tracked_scopes")
    }

    /// none takes the canon default
    pub fn new(
        maximum_concurrent: i64,
        maximum_pending: Option<i64>,
        maximum_tracked_scopes: Option<i64>,
        operation: LottieStickerPrefetchQueueOperation,
    ) -> Arc<Self> {
        let maximum_pending = maximum_pending.unwrap_or_else(Self::default_maximum_pending);
        let maximum_tracked_scopes = maximum_tracked_scopes.unwrap_or_else(Self::default_maximum_tracked_scopes);
        Arc::new(Self {
            maximum_concurrent: usize::try_from(maximum_concurrent.max(1)).unwrap_or(1),
            maximum_pending: usize::try_from(maximum_pending.max(1)).unwrap_or(1),
            maximum_tracked_scopes: usize::try_from(maximum_tracked_scopes.max(1)).unwrap_or(1),
            operation,
            state: RiseIsolated::new(LottieStickerPrefetchQueueState {
                pending_order: Vec::new(),
                pending_by_request: HashMap::new(),
                active_by_request: HashMap::new(),
                successor_by_request: HashMap::new(),
                latest_generation_by_scope: HashMap::new(),
                scope_order: Vec::new(),
                next_task_id: 0,
                environment_generation: 0,
                environment_snapshot: None,
            }),
        })
    }

    pub async fn enqueue(
        self: &Arc<Self>,
        url_strings: &[String],
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        environment: Option<MediaPrefetchEnvironmentSnapshot>,
        cx: &AsyncApp,
    ) {
        if let Some(environment) = environment
            && !self.update_environment(environment).await
        {
            return;
        }
        let requests = self.requests(url_strings, account_user_id.as_deref(), account_epoch.as_deref());
        let starts = self.state.with(|state| {
            for request in requests {
                self.add(state, request, LottieStickerPrefetchQueueOwner::Persistent);
            }
            self.pump(state)
        });
        self.start(starts, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace_directional(
        self: &Arc<Self>,
        url_strings: &[String],
        scope: &str,
        generation: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        environment: Option<MediaPrefetchEnvironmentSnapshot>,
        cx: &AsyncApp,
    ) {
        if let Some(environment) = environment
            && !self.update_environment(environment).await
        {
            return;
        }
        let requests = self.requests(url_strings, account_user_id.as_deref(), account_epoch.as_deref());
        let starts = self.state.with(|state| {
            if !self.accept(state, generation, scope) {
                return Vec::new();
            }
            Self::remove_directional_owners(state, scope);
            let owner = LottieStickerPrefetchQueueOwner::Directional {
                scope: scope.to_owned(),
                generation,
            };
            for request in requests {
                self.add(state, request, owner.clone());
            }
            self.pump(state)
        });
        self.start(starts, cx);
    }

    pub async fn cancel_directional(self: &Arc<Self>, scope: &str, generation: u64, cx: &AsyncApp) {
        let starts = self.state.with(|state| {
            if !self.accept(state, generation, scope) {
                return Vec::new();
            }
            Self::remove_directional_owners(state, scope);
            self.pump(state)
        });
        self.start(starts, cx);
    }

    pub async fn update_environment(&self, snapshot: MediaPrefetchEnvironmentSnapshot) -> bool {
        self.state.with(|state| {
            if snapshot.generation < state.environment_generation {
                return false;
            }
            if snapshot.generation > state.environment_generation {
                state.environment_generation = snapshot.generation;
                Self::reset_locked(state, true);
            }
            state.environment_snapshot = Some(snapshot);
            snapshot.is_current()
        })
    }

    pub async fn reset(&self, preserving_directional_generations: bool) {
        self.state
            .with(|state| Self::reset_locked(state, preserving_directional_generations));
    }

    pub async fn pending_count(&self) -> usize {
        self.state
            .with(|state| state.pending_by_request.len() + state.successor_by_request.len())
    }

    pub async fn active_count(&self) -> usize {
        self.state.with(|state| state.active_by_request.len())
    }

    fn reset_locked(state: &mut LottieStickerPrefetchQueueState, preserving_directional_generations: bool) {
        state.pending_order.clear();
        state.pending_by_request.clear();
        state.successor_by_request.clear();
        if !preserving_directional_generations {
            state.latest_generation_by_scope.clear();
            state.scope_order.clear();
        }
        for active in state.active_by_request.values_mut() {
            active.owners.clear();
            active.cancellation_requested = true;
            active.task.abort();
        }
    }

    fn requests(
        &self,
        url_strings: &[String],
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Vec<LottieStickerPrefetchQueueRequest> {
        let mut seen = HashSet::new();
        let mut result = Vec::with_capacity(url_strings.len().min(self.maximum_pending));
        for raw in url_strings {
            let url_string = trimming_whitespaces_and_newlines(raw);
            if !url_string.to_lowercase().starts_with("http")
                || Url::from_string(url_string).is_none()
                || !seen.insert(url_string.to_owned())
            {
                continue;
            }
            result.push(LottieStickerPrefetchQueueRequest {
                url_string: url_string.to_owned(),
                account_user_id: account_user_id.map(str::to_owned),
                account_epoch: account_epoch.map(str::to_owned),
            });
            if result.len() == self.maximum_pending {
                break;
            }
        }
        result
    }

    fn accept(&self, state: &mut LottieStickerPrefetchQueueState, generation: u64, scope: &str) -> bool {
        if generation < state.latest_generation_by_scope.get(scope).copied().unwrap_or(0) {
            return false;
        }
        state.latest_generation_by_scope.insert(scope.to_owned(), generation);
        state.scope_order.retain(|tracked| tracked != scope);
        state.scope_order.push(scope.to_owned());
        while state.scope_order.len() > self.maximum_tracked_scopes {
            let evicted = state.scope_order.remove(0);
            state.latest_generation_by_scope.remove(&evicted);
            Self::remove_directional_owners(state, &evicted);
        }
        true
    }

    fn add(
        &self,
        state: &mut LottieStickerPrefetchQueueState,
        request: LottieStickerPrefetchQueueRequest,
        owner: LottieStickerPrefetchQueueOwner,
    ) {
        if let Some(active) = state.active_by_request.get_mut(&request) {
            if active.cancellation_requested {
                self.merge_successor(state, request, owner);
            } else {
                active.owners.insert(owner);
            }
            return;
        }
        if let Some(pending) = state.pending_by_request.get_mut(&request) {
            pending.owners.insert(owner);
            return;
        }
        if let Some(successor) = state.successor_by_request.get_mut(&request) {
            successor.owners.insert(owner);
            return;
        }
        if !self.make_room_for_pending(state, &owner) {
            return;
        }
        state.pending_by_request.insert(
            request.clone(),
            LottieStickerPrefetchQueuePendingJob {
                owners: HashSet::from([owner]),
            },
        );
        state.pending_order.push(request);
    }

    fn merge_successor(
        &self,
        state: &mut LottieStickerPrefetchQueueState,
        request: LottieStickerPrefetchQueueRequest,
        owner: LottieStickerPrefetchQueueOwner,
    ) {
        if let Some(successor) = state.successor_by_request.get_mut(&request) {
            successor.owners.insert(owner);
            return;
        }
        if !self.make_room_for_pending(state, &owner) {
            return;
        }
        state.successor_by_request.insert(
            request,
            LottieStickerPrefetchQueuePendingJob {
                owners: HashSet::from([owner]),
            },
        );
    }

    fn make_room_for_pending(
        &self,
        state: &mut LottieStickerPrefetchQueueState,
        owner: &LottieStickerPrefetchQueueOwner,
    ) -> bool {
        if state.pending_by_request.len() + state.successor_by_request.len() < self.maximum_pending {
            return true;
        }
        if !Self::is_directional_owner(owner) {
            return false;
        }
        let persistent_only = state.pending_order.iter().rposition(|request| {
            state
                .pending_by_request
                .get(request)
                .is_some_and(|pending| !pending.owners.iter().any(Self::is_directional_owner))
        });
        if let Some(index) = persistent_only {
            let request = state.pending_order.remove(index);
            state.pending_by_request.remove(&request);
            return true;
        }
        let Some(request) = state.pending_order.pop() else {
            return false;
        };
        state.pending_by_request.remove(&request);
        true
    }

    fn remove_directional_owners(state: &mut LottieStickerPrefetchQueueState, scope: &str) {
        state.pending_by_request.retain(|_, pending| {
            pending.owners.retain(|owner| !Self::is_directional_owner_in_scope(owner, scope));
            !pending.owners.is_empty()
        });
        state.successor_by_request.retain(|_, successor| {
            successor.owners.retain(|owner| !Self::is_directional_owner_in_scope(owner, scope));
            !successor.owners.is_empty()
        });
        for active in state.active_by_request.values_mut() {
            active.owners.retain(|owner| !Self::is_directional_owner_in_scope(owner, scope));
            if active.owners.is_empty() && !active.cancellation_requested {
                active.cancellation_requested = true;
                active.task.abort();
            }
        }
        let pending_by_request = &state.pending_by_request;
        state
            .pending_order
            .retain(|request| pending_by_request.contains_key(request));
    }

    fn is_directional_owner(owner: &LottieStickerPrefetchQueueOwner) -> bool {
        matches!(owner, LottieStickerPrefetchQueueOwner::Directional { .. })
    }

    fn is_directional_owner_in_scope(owner: &LottieStickerPrefetchQueueOwner, scope: &str) -> bool {
        matches!(owner, LottieStickerPrefetchQueueOwner::Directional { scope: owner_scope, .. } if owner_scope == scope)
    }

    fn pump(&self, state: &mut LottieStickerPrefetchQueueState) -> Vec<LottieStickerPrefetchQueueStart> {
        let mut starts = Vec::new();
        while state.active_by_request.len() < self.maximum_concurrent {
            let Some(index) = Self::next_pending_index(state) else {
                break;
            };
            let request = state.pending_order.remove(index);
            let Some(pending) = state.pending_by_request.remove(&request) else {
                continue;
            };
            if pending.owners.is_empty() || state.active_by_request.contains_key(&request) {
                continue;
            }
            state.next_task_id = state.next_task_id.wrapping_add(1);
            let task_id = state.next_task_id;
            let (task, registration) = AbortHandle::new_pair();
            state.active_by_request.insert(
                request.clone(),
                LottieStickerPrefetchQueueActiveJob {
                    owners: pending.owners,
                    task_id,
                    task,
                    cancellation_requested: false,
                },
            );
            starts.push((request, task_id, registration, state.environment_snapshot));
        }
        starts
    }

    fn next_pending_index(state: &LottieStickerPrefetchQueueState) -> Option<usize> {
        let directional = state.pending_order.iter().position(|request| {
            state
                .pending_by_request
                .get(request)
                .is_some_and(|pending| pending.owners.iter().any(Self::is_directional_owner))
        });
        if directional.is_some() {
            return directional;
        }
        if state.pending_order.is_empty() { None } else { Some(0) }
    }

    fn start(self: &Arc<Self>, starts: Vec<LottieStickerPrefetchQueueStart>, cx: &AsyncApp) {
        for (request, task_id, registration, environment) in starts {
            let queue = Arc::clone(self);
            let operation = Arc::clone(&self.operation);
            cx.spawn(async move |cx| {
                if environment.is_none_or(|environment| environment.is_current()) {
                    let work = operation(request.clone(), cx.clone());
                    let _ = Abortable::new(work, registration).await;
                }
                queue.finish(request, task_id, cx);
            })
            .detach();
        }
    }

    fn finish(self: &Arc<Self>, request: LottieStickerPrefetchQueueRequest, task_id: u64, cx: &AsyncApp) {
        let starts = self.state.with(|state| {
            if state.active_by_request.get(&request).map(|active| active.task_id) != Some(task_id) {
                return Vec::new();
            }
            state.active_by_request.remove(&request);
            if let Some(successor) = state.successor_by_request.remove(&request)
                && !successor.owners.is_empty()
            {
                state.pending_by_request.insert(request.clone(), successor);
                state.pending_order.push(request);
            }
            self.pump(state)
        });
        self.start(starts, cx);
    }
}

pub enum LottieStickerPrefetch {}

static QUEUE: LazyLock<Arc<LottieStickerPrefetchQueue>> = LazyLock::new(|| {
    LottieStickerPrefetchQueue::new(
        RiseLimits::client_value("lottie_prefetch_queue_concurrency"),
        None,
        None,
        Arc::new(|request: LottieStickerPrefetchQueueRequest, cx: AsyncApp| {
            async move { LottieStickerPrefetch::prepare(request, cx).await }.boxed_local()
        }),
    )
});

impl LottieStickerPrefetch {
    pub const FIRST_FRAME_POINT_SIZE: Size<Pixels> = Size {
        width: px(180.0),
        height: px(180.0),
    };
    pub const PREPARE_FIRST_FRAMES_MAXIMUM_WAIT: Duration = Duration::from_millis(160);
    const PREPARE_FIRST_FRAMES_LIMIT: usize = 12;
    const PREPARE_FIRST_FRAMES_BATCH: usize = 3;

    pub fn warm(urls: &[String], cx: &mut App) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let environment = MediaPrefetchEnvironment::snapshot(cx);
        if !environment.is_current() {
            return;
        }
        let urls = urls.to_vec();
        cx.spawn(async move |cx| {
            QUEUE
                .enqueue(&urls, account_user_id, account_epoch, Some(environment), cx)
                .await;
        })
        .detach();
    }

    pub async fn prepare_first_frames(urls: &[String], maximum_wait: Duration, cx: &AsyncApp) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let mut seen = HashSet::new();
        let requests: Vec<LottieStickerPrefetchQueueRequest> = urls
            .iter()
            .filter_map(|raw| {
                let url = trimming_whitespaces_and_newlines(raw);
                if !url.to_lowercase().starts_with("http") || Url::from_string(url).is_none() || !seen.insert(url.to_owned()) {
                    return None;
                }
                Some(LottieStickerPrefetchQueueRequest {
                    url_string: url.to_owned(),
                    account_user_id: account_user_id.clone(),
                    account_epoch: account_epoch.clone(),
                })
            })
            .take(Self::PREPARE_FIRST_FRAMES_LIMIT)
            .collect();
        if requests.is_empty() {
            return;
        }
        let preparation = async {
            for batch in requests.chunks(Self::PREPARE_FIRST_FRAMES_BATCH) {
                futures::future::join_all(batch.iter().map(Self::prepare_cached)).await;
            }
        };
        let timeout = cx.background_executor().timer(maximum_wait);
        let _ = futures::future::select(Box::pin(preparation), timeout).await;
    }

    pub fn replace_directional(
        urls: &[String],
        scope: &str,
        generation: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut App,
    ) {
        let environment = MediaPrefetchEnvironment::snapshot(cx);
        let urls = urls.to_vec();
        let scope = scope.to_owned();
        cx.spawn(async move |cx| {
            if !environment.is_current() {
                QUEUE.cancel_directional(&scope, generation, cx).await;
                return;
            }
            QUEUE
                .replace_directional(&urls, &scope, generation, account_user_id, account_epoch, Some(environment), cx)
                .await;
        })
        .detach();
    }

    pub fn cancel_directional(scope: &str, generation: u64, cx: &mut App) {
        let scope = scope.to_owned();
        cx.spawn(async move |cx| {
            QUEUE.cancel_directional(&scope, generation, cx).await;
        })
        .detach();
    }

    pub async fn environment_did_change(snapshot: MediaPrefetchEnvironmentSnapshot) {
        QUEUE.update_environment(snapshot).await;
    }

    pub fn reset_account_state() {
        let queue = Arc::clone(&QUEUE);
        RiseAccountRuntime::current().spawn(async move {
            queue.reset(false).await;
            LottieStickerLoader::shared().reset_active_loads().await;
            RiseLottieFirstFrameCache::reset_memory();
            RiseLottieFrameCache::shared().release_all();
        });
    }

    async fn prepare(request: LottieStickerPrefetchQueueRequest, cx: AsyncApp) {
        let environment = cx.update(MediaPrefetchEnvironment::snapshot);
        if !environment.is_current()
            || !SaiAccountCacheScope::is_current_account_epoch(
                request.account_user_id.as_deref(),
                request.account_epoch.as_deref(),
            )
        {
            return;
        }
        let Ok(payload) = LottieStickerLoader::shared().payload(&request.url_string).await else {
            return;
        };
        if !environment.is_current() {
            return;
        }
        if let LottieStickerPayload::Animation(render_source) = payload {
            RiseLottieFirstFrameCache::prepare(
                render_source,
                Self::FIRST_FRAME_POINT_SIZE,
                request.account_user_id,
                request.account_epoch,
                Some(environment),
            )
            .await;
        }
    }

    async fn prepare_cached(request: &LottieStickerPrefetchQueueRequest) {
        if !SaiAccountCacheScope::is_current_account_epoch(
            request.account_user_id.as_deref(),
            request.account_epoch.as_deref(),
        ) {
            return;
        }
        let Some(payload) = LottieStickerLoader::shared()
            .cached_payload_from_disk(&request.url_string)
            .await
        else {
            return;
        };
        if let LottieStickerPayload::Animation(render_source) = payload {
            RiseLottieFirstFrameCache::prepare(
                render_source,
                Self::FIRST_FRAME_POINT_SIZE,
                request.account_user_id.clone(),
                request.account_epoch.clone(),
                None,
            )
            .await;
        }
    }
}

#[cfg(test)]
#[path = "lottie_sticker_prefetch_tests.rs"]
mod tests;
