use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Weak};
use std::task::{Context as TaskContext, Poll, Waker};
use std::time::Duration;

use futures::Stream;
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use futures::future::{AbortHandle, AbortRegistration, Abortable};
use gpui::{App, AsyncApp};
use parking_lot::Mutex;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::file_manager::FileManager;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDiskCacheError, RemoteMediaDiskCacheLimits, RemoteMediaDiskCacheMaterializeOptions,
    RemoteMediaDiskCacheProgressHandler, RemoteMediaDownloadProgress, RemoteMediaTransferError,
};
use crate::core::media::rise_media_stream_session::{RiseMediaStreamError, RiseMediaStreamRegistry};
use crate::core::media::rise_media_transfer_hub_hooks::RiseMediaTransferHubHooks;

/// one cache identity per remote object
/// bubble and viewer and save resolve to the same string so a video is fetched once
/// query is dropped because urls are presigned and the signature changes every time
pub enum RiseMediaCacheIdentity {}

impl RiseMediaCacheIdentity {
    pub fn key(remote: &Url) -> String {
        let components = UrlComponents::from_url(remote, false);
        let scheme = remote.scheme().map(str::to_lowercase).unwrap_or_default();
        let host = remote.host().map(|host| host.to_lowercase()).unwrap_or_default();
        let raw_path = components
            .as_ref()
            .map_or_else(|| remote.path(), |components| components.percent_encoded_path().to_owned());
        let path = if raw_path.is_empty() { "/".to_owned() } else { raw_path };
        format!("media:{scheme}://{host}{path}")
    }
}

/// turns a viewer item raw url into the shared cache identity
/// keying by item id made one object download twice
pub enum MediaViewerVideoCacheKey {}

impl MediaViewerVideoCacheKey {
    pub fn resolve(raw_url: &str) -> Option<String> {
        let normalized = RiseMediaTransferHubHooks::optional_normalized(Some(raw_url))?;
        let remote = Url::from_string(&normalized)?;
        let scheme = remote.scheme().map(str::to_lowercase);
        if !matches!(scheme.as_deref(), Some("https") | Some("http")) {
            return None;
        }
        Some(RiseMediaCacheIdentity::key(&remote))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseMediaTransferKey {
    pub cache_key: String,
    pub account_epoch: Option<String>,
}

impl RiseMediaTransferKey {
    pub fn new(cache_key: String, account_epoch: Option<String>) -> Self {
        Self {
            cache_key,
            account_epoch,
        }
    }

    /// new key using the epoch of the active account
    pub fn for_active_account(cache_key: String) -> Self {
        Self::new(cache_key, SaiAccountCacheScope::active_epoch())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseMediaTransferPhase {
    Idle,
    Queued,
    Downloading,
    Cached,
    Failed { code: RiseMediaTransferFailure },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMediaTransferFailure {
    Network,
    TooLarge,
    Unsupported,
    Unavailable,
}

impl RiseMediaTransferFailure {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::TooLarge => "tooLarge",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "network" => Some(Self::Network),
            "tooLarge" => Some(Self::TooLarge),
            "unsupported" => Some(Self::Unsupported),
            "unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMediaTransferState {
    pub key: RiseMediaTransferKey,
    pub phase: RiseMediaTransferPhase,
    pub downloaded_bytes: i64,
    pub total_bytes: i64,
}

impl RiseMediaTransferState {
    pub fn is_active(&self) -> bool {
        self.phase == RiseMediaTransferPhase::Queued || self.phase == RiseMediaTransferPhase::Downloading
    }

    pub fn fraction(&self) -> f64 {
        if self.total_bytes <= 0 {
            return 0.;
        }
        (self.downloaded_bytes as f64 / self.total_bytes as f64).clamp(0., 1.)
    }
}

#[derive(Clone, Debug)]
pub struct RiseMediaTransferRequest {
    pub remote: Url,
    pub cache_key: String,
    pub suggested_filename: Option<String>,
    pub declared_byte_count: i64,
    pub authorization_token: Option<String>,
    pub is_streamable: bool,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

/// arguments of a transfer request, account defaults to the active one
#[derive(Clone, Debug)]
pub struct RiseMediaTransferRequestArgs {
    pub cache_key: Option<String>,
    pub suggested_filename: Option<String>,
    pub declared_byte_count: i64,
    pub authorization_token: Option<String>,
    pub is_streamable: bool,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

impl Default for RiseMediaTransferRequestArgs {
    fn default() -> Self {
        Self {
            cache_key: None,
            suggested_filename: None,
            declared_byte_count: 0,
            authorization_token: None,
            is_streamable: false,
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
        }
    }
}

impl RiseMediaTransferRequest {
    pub fn new(remote: Url, args: RiseMediaTransferRequestArgs) -> Self {
        let cache_key = args
            .cache_key
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|cache_key| !cache_key.is_empty())
            .map_or_else(|| RiseMediaCacheIdentity::key(&remote), str::to_owned);
        Self {
            remote,
            cache_key,
            suggested_filename: args.suggested_filename,
            declared_byte_count: args.declared_byte_count.max(0),
            authorization_token: args.authorization_token,
            is_streamable: args.is_streamable,
            account_user_id: args.account_user_id,
            account_epoch: args.account_epoch,
        }
    }

    pub fn key(&self) -> RiseMediaTransferKey {
        RiseMediaTransferKey::new(self.cache_key.clone(), self.account_epoch.clone())
    }
}

/// one stream continuation that buffers only the newest value
struct RiseMediaTransferUpdatesSlot {
    latest: Option<RiseMediaTransferState>,
    finished: bool,
    waker: Option<Waker>,
}

#[derive(Clone)]
struct RiseMediaTransferContinuation {
    slot: Arc<Mutex<RiseMediaTransferUpdatesSlot>>,
}

impl RiseMediaTransferContinuation {
    fn yield_state(&self, state: RiseMediaTransferState) {
        let waker = {
            let mut slot = self.slot.lock();
            if slot.finished {
                return;
            }
            slot.latest = Some(state);
            slot.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn finish(&self) {
        let waker = {
            let mut slot = self.slot.lock();
            slot.finished = true;
            slot.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// stream of updates for a key, current state first and then only the newest change per flush
/// dropping it ends the subscription
pub struct RiseMediaTransferUpdates {
    slot: Arc<Mutex<RiseMediaTransferUpdatesSlot>>,
    hub: Weak<RiseMediaTransferHub>,
    identifier: Uuid,
}

impl Stream for RiseMediaTransferUpdates {
    type Item = RiseMediaTransferState;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let mut slot = self.slot.lock();
        if let Some(state) = slot.latest.take() {
            return Poll::Ready(Some(state));
        }
        if slot.finished {
            return Poll::Ready(None);
        }
        slot.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for RiseMediaTransferUpdates {
    fn drop(&mut self) {
        self.slot.lock().finished = true;
        if let Some(hub) = self.hub.upgrade() {
            hub.remove_subscription(self.identifier);
        }
    }
}

struct RiseMediaTransferHubSubscription {
    key: RiseMediaTransferKey,
    after_version: u64,
    continuation: RiseMediaTransferContinuation,
}

struct RiseMediaTransferHubPendingChange {
    version: u64,
    state: RiseMediaTransferState,
}

struct RiseMediaTransferHubActive {
    request: RiseMediaTransferRequest,
    task: Option<AbortHandle>,
    declared_byte_count: i64,
    user_requested: bool,
}

/// transfer handed to the main thread where materialize runs with the app
struct RiseMediaTransferHubLaunch {
    key: RiseMediaTransferKey,
    request: RiseMediaTransferRequest,
    registration: AbortRegistration,
}

type RiseMediaTransferHubWaiter = oneshot::Sender<Result<PathBuf, RemoteMediaTransferError>>;

#[derive(Default)]
struct RiseMediaTransferHubState {
    states: HashMap<RiseMediaTransferKey, RiseMediaTransferState>,
    state_order: Vec<RiseMediaTransferKey>,
    settled_at: HashMap<RiseMediaTransferKey, Duration>,
    active: HashMap<RiseMediaTransferKey, RiseMediaTransferHubActive>,
    queue: VecDeque<RiseMediaTransferKey>,
    running_count: i64,

    subscriptions: HashMap<Uuid, RiseMediaTransferHubSubscription>,
    subscription_order: Vec<Uuid>,
    subscriptions_by_key: HashMap<RiseMediaTransferKey, HashSet<Uuid>>,
    pending_changes: HashMap<RiseMediaTransferKey, RiseMediaTransferHubPendingChange>,
    mutation_version: u64,
    publish_task: Option<AbortHandle>,

    waiters: HashMap<RiseMediaTransferKey, HashMap<Uuid, RiseMediaTransferHubWaiter>>,
    parked_by_account: HashMap<String, Vec<RiseMediaTransferRequest>>,
    present_digests: HashSet<String>,
    presence_account_user_id: Option<String>,
    presence_loaded: bool,

    foreground: Option<mpsc::UnboundedSender<RiseMediaTransferHubLaunch>>,
}

static SHARED: LazyLock<Arc<RiseMediaTransferHub>> =
    LazyLock::new(|| RiseMediaTransferHub::new(Duration::from_millis(16), 1_024, 512));

/// app wide owner of media byte transfers, in front of the disk cache
/// cache blocks on manifest locks and a progress ring must not queue behind it
/// no main thread state so a byte tick cant redraw a message row
pub struct RiseMediaTransferHub {
    publish_cadence: Duration,
    subscriber_capacity: usize,
    state_capacity: usize,
    state: RiseIsolated<RiseMediaTransferHubState>,
    this: Weak<RiseMediaTransferHub>,
}

struct RiseMediaTransferHubFlushOnDrop {
    hub: Weak<RiseMediaTransferHub>,
}

impl Drop for RiseMediaTransferHubFlushOnDrop {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade() {
            hub.flush();
        }
    }
}

struct RiseMediaTransferHubWaiterGuard {
    hub: Weak<RiseMediaTransferHub>,
    key: RiseMediaTransferKey,
    waiter_id: Uuid,
    armed: bool,
}

impl Drop for RiseMediaTransferHubWaiterGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(hub) = self.hub.upgrade() {
            hub.drop_waiter(self.waiter_id, &self.key);
        }
    }
}

impl RiseMediaTransferHub {
    const MAXIMUM_CONCURRENT_TRANSFERS: i64 = 3;

    const MAXIMUM_PARKED_PER_ACCOUNT: usize = 16;

    const STALE_PROGRESS_WINDOW: Duration = Duration::from_secs(2);

    pub fn shared() -> Arc<RiseMediaTransferHub> {
        Arc::clone(&SHARED)
    }

    /// new hub, shared one uses 16 ms and 1024 and 512
    pub fn new(publish_cadence: Duration, subscriber_capacity: i64, state_capacity: i64) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            publish_cadence,
            subscriber_capacity: usize::try_from(subscriber_capacity.max(1)).unwrap_or(1),
            state_capacity: usize::try_from(state_capacity.max(1)).unwrap_or(1),
            state: RiseIsolated::new(RiseMediaTransferHubState::default()),
            this: this.clone(),
        })
    }

    pub async fn prewarm(&self) {
        self.load_presence_if_needed();
    }

    pub async fn reload_presence(&self) {
        let previous = self.state.with(|state| {
            state.presence_loaded = false;
            state.present_digests.clone()
        });
        self.load_presence_if_needed();
        let state_capacity = self.state_capacity;
        self.state.with(|state| {
            let removed: HashSet<String> = previous.difference(&state.present_digests).cloned().collect();
            if removed.is_empty() {
                return;
            }
            let resets: Vec<RiseMediaTransferState> = state
                .states
                .iter()
                .filter(|(key, current)| {
                    current.phase == RiseMediaTransferPhase::Cached && !state.active.contains_key(*key)
                })
                .filter(|(key, _)| removed.contains(&RemoteMediaDiskCache::key_digest(&key.cache_key)))
                .map(|(key, current)| RiseMediaTransferState {
                    key: key.clone(),
                    phase: RiseMediaTransferPhase::Idle,
                    downloaded_bytes: 0,
                    total_bytes: current.total_bytes,
                })
                .collect();
            for reset in resets {
                state.publish(reset, state_capacity);
            }
        });
        self.schedule_publish();
    }

    pub async fn state(&self, key: &RiseMediaTransferKey) -> RiseMediaTransferState {
        self.load_presence_if_needed();
        self.state.with(|state| state.resolved_state(key))
    }

    /// updates for a key, the stream yields the current state first
    pub fn updates(&self, key: &RiseMediaTransferKey) -> RiseMediaTransferUpdates {
        self.load_presence_if_needed();
        let identifier = Uuid::new_v4();
        let slot = Arc::new(Mutex::new(RiseMediaTransferUpdatesSlot {
            latest: None,
            finished: false,
            waker: None,
        }));
        let continuation = RiseMediaTransferContinuation { slot: Arc::clone(&slot) };
        let stream = RiseMediaTransferUpdates {
            slot,
            hub: self.this.clone(),
            identifier,
        };
        let subscriber_capacity = self.subscriber_capacity;
        self.state.with(|state| {
            state.subscriptions.insert(
                identifier,
                RiseMediaTransferHubSubscription {
                    key: key.clone(),
                    after_version: state.mutation_version,
                    continuation: continuation.clone(),
                },
            );
            state.subscription_order.push(identifier);
            state.subscriptions_by_key.entry(key.clone()).or_default().insert(identifier);
            continuation.yield_state(state.resolved_state(key));
            state.evict_subscriptions_if_needed(subscriber_capacity);
        });
        self.resume_parked(key);
        stream
    }

    /// one button, starts when idle or failed and cancels while running
    /// returns the phase the tap made so callers can do haptics
    pub fn toggle(&self, request: RiseMediaTransferRequest, cx: &App) -> RiseMediaTransferPhase {
        self.attach_foreground(&cx.to_async());
        self.load_presence_if_needed();
        let key = request.key();
        let current = self.state.with(|state| state.resolved_state(&key));
        match current.phase {
            RiseMediaTransferPhase::Queued | RiseMediaTransferPhase::Downloading => {
                self.cancel_transfer(&key);
                RiseMediaTransferPhase::Idle
            }
            RiseMediaTransferPhase::Cached => RiseMediaTransferPhase::Cached,
            RiseMediaTransferPhase::Idle | RiseMediaTransferPhase::Failed { .. } => {
                self.start_transfer(request, true);
                RiseMediaTransferPhase::Queued
            }
        }
    }

    /// starts a transfer, user_requested says if a person asked for it
    pub fn start(&self, request: RiseMediaTransferRequest, user_requested: bool, cx: &App) {
        self.attach_foreground(&cx.to_async());
        self.start_transfer(request, user_requested);
    }

    fn start_transfer(&self, request: RiseMediaTransferRequest, user_requested: bool) {
        self.load_presence_if_needed();
        let key = request.key();
        if !SaiAccountCacheScope::is_current_account_epoch(
            request.account_user_id.as_deref(),
            request.account_epoch.as_deref(),
        ) {
            return;
        }
        let state_capacity = self.state_capacity;
        self.state.with(|state| {
            if user_requested && let Some(user_id) = request.account_user_id.as_deref() {
                state.drop_parked(user_id, &request.cache_key);
            }
            if let Some(existing) = state.active.get_mut(&key) {
                existing.declared_byte_count = existing.declared_byte_count.max(request.declared_byte_count);
                existing.user_requested = existing.user_requested || user_requested;
                return;
            }
            let declared_byte_count = request.declared_byte_count;
            state.active.insert(
                key.clone(),
                RiseMediaTransferHubActive {
                    request,
                    task: None,
                    declared_byte_count,
                    user_requested,
                },
            );
            state.queue.push_back(key.clone());
            state.publish(
                RiseMediaTransferState {
                    key,
                    phase: RiseMediaTransferPhase::Queued,
                    downloaded_bytes: 0,
                    total_bytes: declared_byte_count,
                },
                state_capacity,
            );
            state.pump();
        });
        self.schedule_publish();
    }

    /// awaits the bytes and joins a running transfer
    /// caller cancel drops only its wait so a scrolled away
    /// bubble doesnt kill the download, only cancel stops it
    pub async fn fetch(&self, request: RiseMediaTransferRequest, cx: &AsyncApp) -> anyhow::Result<PathBuf> {
        self.attach_foreground(cx);
        let lookup = request.clone();
        let existing = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                RemoteMediaDiskCache::shared().existing_local_url(
                    &lookup.remote,
                    Some(lookup.cache_key.as_str()),
                    lookup.account_user_id.as_deref(),
                    lookup.account_epoch.as_deref(),
                )
            })
            .await
            .ok()
            .flatten();
        if let Some(existing) = existing {
            self.note(
                request.cache_key.clone(),
                0,
                SaiAccountCacheScope::active_user_id(),
                SaiAccountCacheScope::active_epoch(),
            )
            .await;
            return Ok(existing);
        }

        let key = request.key();
        self.start_transfer(request, false);
        let waiter_id = Uuid::new_v4();
        let receiver = self.state.with(|state| {
            if !state.active.contains_key(&key) {
                return None;
            }
            let (sender, receiver) = oneshot::channel();
            state.waiters.entry(key.clone()).or_default().insert(waiter_id, sender);
            Some(receiver)
        });
        let Some(receiver) = receiver else {
            return Err(anyhow::Error::new(RemoteMediaDiskCacheError::CacheUnavailable));
        };
        let mut guard = RiseMediaTransferHubWaiterGuard {
            hub: self.this.clone(),
            key,
            waiter_id,
            armed: true,
        };
        let result = receiver.await;
        guard.armed = false;
        match result {
            Ok(Ok(local)) => Ok(local),
            Ok(Err(error)) => Err(error.into_anyhow()),
            Err(_) => Err(anyhow::Error::new(CancellationError)),
        }
    }

    fn drop_waiter(&self, waiter_id: Uuid, key: &RiseMediaTransferKey) {
        self.state.with(|state| {
            let Some(waiters) = state.waiters.get_mut(key) else {
                return;
            };
            let Some(waiter) = waiters.remove(&waiter_id) else {
                return;
            };
            if waiters.is_empty() {
                state.waiters.remove(key);
            }
            let _ = waiter.send(Err(RemoteMediaTransferError::Cancelled(CancellationError)));
        });
    }

    pub async fn cancel(&self, key: &RiseMediaTransferKey) {
        self.cancel_transfer(key);
    }

    fn cancel_transfer(&self, key: &RiseMediaTransferKey) {
        let state_capacity = self.state_capacity;
        let request = self.state.with(|state| {
            let entry = state.active.remove(key)?;
            state.queue.retain(|queued| queued != key);
            if entry.task.is_some() {
                state.running_count = (state.running_count - 1).max(0);
            }
            if let Some(task) = &entry.task {
                task.abort();
            }
            state.resume_waiters(key, Err(RemoteMediaTransferError::Cancelled(CancellationError)));
            state.publish(
                RiseMediaTransferState {
                    key: key.clone(),
                    phase: RiseMediaTransferPhase::Idle,
                    downloaded_bytes: 0,
                    total_bytes: entry.declared_byte_count,
                },
                state_capacity,
            );
            state.pump();
            Some(entry.request)
        });
        let Some(request) = request else {
            return;
        };
        RiseAccountRuntime::current().spawn(async move {
            if request.is_streamable {
                RiseMediaStreamRegistry::shared()
                    .cancel(&request.cache_key, request.account_epoch.as_deref())
                    .await;
                return;
            }
            RemoteMediaDiskCache::shared().cancel_transfer(
                &request.remote,
                Some(request.cache_key.as_str()),
                request.account_user_id.as_deref(),
                request.account_epoch.as_deref(),
            );
        });
        self.schedule_publish();
    }

    /// reported by a caller that got the file out of the media cache another way
    /// keeps the presence set honest without a manifest read
    pub async fn note(
        &self,
        cached_key: String,
        byte_count: i64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) {
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return;
        }
        let digest = RemoteMediaDiskCache::key_digest(&cached_key);
        let key = RiseMediaTransferKey::new(cached_key, account_epoch);
        let state_capacity = self.state_capacity;
        self.state.with(|state| {
            state.present_digests.insert(digest);
            state.publish(
                RiseMediaTransferState {
                    key,
                    phase: RiseMediaTransferPhase::Cached,
                    downloaded_bytes: byte_count,
                    total_bytes: byte_count,
                },
                state_capacity,
            );
        });
        self.schedule_publish();
    }

    /// progress made outside the hub by the image pipeline or voice cache or music player
    /// the ring is shared and the account is the active one
    pub async fn ingest(&self, cache_key: String, downloaded_bytes: i64, total_bytes: i64, is_finished: bool) {
        self.ingest_for_account(
            cache_key,
            downloaded_bytes,
            total_bytes,
            is_finished,
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
        )
        .await;
    }

    /// same as ingest but for a given account
    pub async fn ingest_for_account(
        &self,
        cache_key: String,
        downloaded_bytes: i64,
        total_bytes: i64,
        is_finished: bool,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) {
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return;
        }
        let key = RiseMediaTransferKey::new(cache_key, account_epoch);
        let state_capacity = self.state_capacity;
        let now = RiseClock::monotonic();
        self.state.with(|state| {
            if is_finished {
                let byte_count = downloaded_bytes.max(total_bytes);
                state.publish(
                    RiseMediaTransferState {
                        key,
                        phase: RiseMediaTransferPhase::Cached,
                        downloaded_bytes: byte_count,
                        total_bytes: byte_count,
                    },
                    state_capacity,
                );
                return;
            }
            if let Some(current) = state.states.get(&key)
                && !current.is_active()
                && let Some(settled) = state.settled_at.get(&key)
                && now.saturating_sub(*settled) < Self::STALE_PROGRESS_WINDOW
            {
                return;
            }
            state.publish(
                RiseMediaTransferState {
                    key,
                    phase: RiseMediaTransferPhase::Downloading,
                    downloaded_bytes,
                    total_bytes,
                },
                state_capacity,
            );
        });
        self.schedule_publish();
    }

    /// a producer gave up, failed or cancelled before bytes landed
    /// without this the chips keep a ring spinning over nothing
    pub async fn abandon(&self, cache_key: String) {
        self.abandon_for_account(
            cache_key,
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
        )
        .await;
    }

    pub async fn abandon_for_account(
        &self,
        cache_key: String,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) {
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return;
        }
        let key = RiseMediaTransferKey::new(cache_key, account_epoch);
        let state_capacity = self.state_capacity;
        self.state.with(|state| {
            let Some(current) = state.states.get(&key) else {
                return;
            };
            if !current.is_active() {
                return;
            }
            let total_bytes = current.total_bytes;
            state.publish(
                RiseMediaTransferState {
                    key,
                    phase: RiseMediaTransferPhase::Idle,
                    downloaded_bytes: 0,
                    total_bytes,
                },
                state_capacity,
            );
        });
        self.schedule_publish();
    }

    pub async fn reset_account_state(&self) {
        let open = self.state.with(|state| {
            for entry in state.active.values() {
                if let Some(task) = &entry.task {
                    task.abort();
                }
            }
            let parked: Vec<RiseMediaTransferRequest> = state
                .active
                .values()
                .filter(|entry| entry.user_requested)
                .map(|entry| entry.request.clone())
                .collect();
            state.park(parked);
            state.active.clear();
            state.queue.clear();
            state.running_count = 0;
            let pending: Vec<RiseMediaTransferHubWaiter> =
                state.waiters.drain().flat_map(|(_, waiters)| waiters.into_values()).collect();
            for waiter in pending {
                let _ = waiter.send(Err(RemoteMediaTransferError::Cache(RemoteMediaDiskCacheError::AccountChanged)));
            }
            state.states.clear();
            state.state_order.clear();
            state.settled_at.clear();
            state.pending_changes.clear();
            state.present_digests.clear();
            state.presence_account_user_id = None;
            state.presence_loaded = false;
            if let Some(task) = state.publish_task.take() {
                task.abort();
            }
            let open: Vec<RiseMediaTransferContinuation> = state
                .subscriptions
                .drain()
                .map(|(_, subscription)| subscription.continuation)
                .collect();
            state.subscription_order.clear();
            state.subscriptions_by_key.clear();
            open
        });
        for continuation in open {
            continuation.finish();
        }
    }

    fn resume_parked(&self, key: &RiseMediaTransferKey) {
        let (Some(user_id), Some(epoch)) = (SaiAccountCacheScope::active_user_id(), SaiAccountCacheScope::active_epoch())
        else {
            return;
        };
        if key.account_epoch.as_deref() != Some(epoch.as_str()) {
            return;
        }
        let parked = self.state.with(|state| {
            if state.active.contains_key(key) {
                return None;
            }
            state
                .parked_by_account
                .get(&user_id)?
                .iter()
                .find(|parked| parked.cache_key == key.cache_key)
                .cloned()
        });
        let Some(parked) = parked else {
            return;
        };
        let request = RiseMediaTransferRequest::new(
            parked.remote,
            RiseMediaTransferRequestArgs {
                cache_key: Some(parked.cache_key),
                suggested_filename: parked.suggested_filename,
                declared_byte_count: parked.declared_byte_count,
                authorization_token: parked.authorization_token,
                is_streamable: parked.is_streamable,
                account_user_id: Some(user_id),
                account_epoch: Some(epoch),
            },
        );
        self.start_transfer(request, true);
    }

    /// opens the main thread lane once per app, a lane whose app went away is replaced
    fn attach_foreground(&self, cx: &AsyncApp) {
        let attached = self
            .state
            .with(|state| state.foreground.as_ref().is_some_and(|foreground| !foreground.is_closed()));
        if attached {
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<RiseMediaTransferHubLaunch>();
        let hub = self.this.clone();
        cx.spawn(async move |cx| {
            while let Some(launch) = receiver.next().await {
                let Some(hub) = hub.upgrade() else {
                    return;
                };
                let RiseMediaTransferHubLaunch {
                    key,
                    request,
                    registration,
                } = launch;
                cx.spawn(async move |cx| {
                    let _ = Abortable::new(hub.run(key, request, cx), registration).await;
                })
                .detach();
            }
        })
        .detach();
        self.state.with(|state| {
            state.foreground = Some(sender);
            state.pump();
        });
    }

    async fn run(&self, key: RiseMediaTransferKey, request: RiseMediaTransferRequest, cx: &AsyncApp) {
        let state_capacity = self.state_capacity;
        let declared = self.state.with(|state| {
            let declared = state
                .active
                .get(&key)
                .map_or(request.declared_byte_count, |entry| entry.declared_byte_count);
            state.publish(
                RiseMediaTransferState {
                    key: key.clone(),
                    phase: RiseMediaTransferPhase::Downloading,
                    downloaded_bytes: 0,
                    total_bytes: declared,
                },
                state_capacity,
            );
            declared
        });
        self.schedule_publish();

        if request.is_streamable {
            self.run_streaming(key, request, declared, cx).await;
            return;
        }

        let hub = self.this.clone();
        let progress_key = key.clone();
        let on_progress: RemoteMediaDiskCacheProgressHandler = Arc::new(move |progress: RemoteMediaDownloadProgress| {
            let Some(hub) = hub.upgrade() else {
                return;
            };
            hub.apply(progress, &progress_key, declared);
        });
        let options = RemoteMediaDiskCacheMaterializeOptions {
            cache_key: Some(request.cache_key.clone()),
            suggested_filename: request.suggested_filename.clone(),
            maximum_byte_count: None,
            authorization_token: request.authorization_token.clone(),
            expected_account_user_id: request.account_user_id.clone(),
            expected_account_epoch: request.account_epoch.clone(),
            on_progress: Some(on_progress),
        };
        let result = RemoteMediaDiskCache::shared()
            .materialize(&request.remote, options, cx)
            .await
            .map_err(RemoteMediaTransferError::from_anyhow);
        self.settle(&key, result, declared);
    }

    /// result handling shared by both transfer kinds
    fn settle(&self, key: &RiseMediaTransferKey, result: Result<PathBuf, RemoteMediaTransferError>, declared: i64) {
        match result {
            Ok(local) => self.finish(key, &local, declared),
            Err(error) if Self::is_cancellation(&error) => self.finish_cancelled(key, declared),
            Err(error) => self.finish_with_error(key, error, declared),
        }
    }

    async fn run_streaming(
        &self,
        key: RiseMediaTransferKey,
        request: RiseMediaTransferRequest,
        declared: i64,
        cx: &AsyncApp,
    ) {
        let session = RiseMediaStreamRegistry::shared()
            .session(
                request.cache_key.clone(),
                request.remote.clone(),
                request.account_user_id.clone(),
                request.account_epoch.clone(),
                RemoteMediaDiskCacheLimits::default_limits().maximum_file_bytes,
                request.suggested_filename.clone(),
            )
            .await;
        let result = session
            .complete(cx)
            .await
            .map_err(RemoteMediaTransferError::from_anyhow);
        self.settle(&key, result, declared);
    }

    fn apply(&self, progress: RemoteMediaDownloadProgress, key: &RiseMediaTransferKey, declared: i64) {
        let state_capacity = self.state_capacity;
        self.state.with(|state| {
            if !state.active.contains_key(key) {
                return;
            }
            state.publish(
                RiseMediaTransferState {
                    key: key.clone(),
                    phase: RiseMediaTransferPhase::Downloading,
                    downloaded_bytes: progress.downloaded_bytes,
                    total_bytes: declared.max(progress.total_bytes.unwrap_or(0)),
                },
                state_capacity,
            );
        });
        self.schedule_publish();
    }

    /// marks the transfer finished with the local file and declared size
    fn finish(&self, key: &RiseMediaTransferKey, local_url: &Path, declared: i64) {
        let byte_count = declared.max(Self::file_byte_count(local_url));
        let digest = RemoteMediaDiskCache::key_digest(&key.cache_key);
        let state_capacity = self.state_capacity;
        self.state.with(|state| {
            state.release_slot(key);
            state.present_digests.insert(digest);
            state.resume_waiters(key, Ok(local_url.to_path_buf()));
            state.publish(
                RiseMediaTransferState {
                    key: key.clone(),
                    phase: RiseMediaTransferPhase::Cached,
                    downloaded_bytes: byte_count,
                    total_bytes: byte_count,
                },
                state_capacity,
            );
            state.pump();
        });
        self.schedule_publish();
    }

    fn finish_cancelled(&self, key: &RiseMediaTransferKey, declared: i64) {
        let state_capacity = self.state_capacity;
        self.state.with(|state| {
            state.release_slot(key);
            state.resume_waiters(key, Err(RemoteMediaTransferError::Cancelled(CancellationError)));
            state.publish(
                RiseMediaTransferState {
                    key: key.clone(),
                    phase: RiseMediaTransferPhase::Idle,
                    downloaded_bytes: 0,
                    total_bytes: declared,
                },
                state_capacity,
            );
            state.pump();
        });
        self.schedule_publish();
    }

    fn finish_with_error(&self, key: &RiseMediaTransferKey, error: RemoteMediaTransferError, declared: i64) {
        let is_account_change = Self::is_account_change(&error);
        let code = Self::failure(&error);
        let digest = RemoteMediaDiskCache::key_digest(&key.cache_key);
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Media, || {
            format!("media transfer failed: {}", code.raw_value())
        });
        let state_capacity = self.state_capacity;
        self.state.with(|state| {
            if is_account_change
                && let Some(entry) = state.active.get(key)
                && entry.user_requested
            {
                let request = entry.request.clone();
                state.park(vec![request]);
            }
            state.release_slot(key);
            state.resume_waiters(key, Err(error));
            state.present_digests.remove(&digest);
            state.publish(
                RiseMediaTransferState {
                    key: key.clone(),
                    phase: RiseMediaTransferPhase::Failed { code },
                    downloaded_bytes: 0,
                    total_bytes: declared,
                },
                state_capacity,
            );
            state.pump();
        });
        self.schedule_publish();
    }

    /// true for a cancellation or a cancelled stream error
    fn is_cancellation(error: &RemoteMediaTransferError) -> bool {
        matches!(error, RemoteMediaTransferError::Cancelled(_))
            || matches!(Self::stream_error(error), Some(RiseMediaStreamError::Cancelled))
    }

    fn is_account_change(error: &RemoteMediaTransferError) -> bool {
        matches!(
            error,
            RemoteMediaTransferError::Cache(RemoteMediaDiskCacheError::AccountChanged)
        ) || matches!(Self::stream_error(error), Some(RiseMediaStreamError::AccountChanged))
    }

    fn failure(error: &RemoteMediaTransferError) -> RiseMediaTransferFailure {
        if let RemoteMediaTransferError::Cache(error) = error {
            return match error {
                RemoteMediaDiskCacheError::PayloadTooLarge => RiseMediaTransferFailure::TooLarge,
                RemoteMediaDiskCacheError::UnsupportedPayload | RemoteMediaDiskCacheError::InvalidSource => {
                    RiseMediaTransferFailure::Unsupported
                }
                RemoteMediaDiskCacheError::CacheUnavailable | RemoteMediaDiskCacheError::AccountChanged => {
                    RiseMediaTransferFailure::Unavailable
                }
                _ => RiseMediaTransferFailure::Network,
            };
        }
        match Self::stream_error(error) {
            Some(RiseMediaStreamError::TooLarge) => RiseMediaTransferFailure::TooLarge,
            Some(RiseMediaStreamError::AccountChanged) => RiseMediaTransferFailure::Unavailable,
            _ => RiseMediaTransferFailure::Network,
        }
    }

    /// a session failure travels as other, the way the stream session wraps it
    fn stream_error(error: &RemoteMediaTransferError) -> Option<&RiseMediaStreamError> {
        match error {
            RemoteMediaTransferError::Other(error) => error.downcast_ref::<RiseMediaStreamError>(),
            _ => None,
        }
    }

    fn file_byte_count(url: &Path) -> i64 {
        FileManager.attributes_of_item(url).map_or(0, |attributes| attributes.size)
    }

    /// blocking manifest read outside the isolated state
    /// prewarm runs it off main on account activation so later calls find it loaded
    fn load_presence_if_needed(&self) {
        let user_id = SaiAccountCacheScope::active_user_id();
        let loaded = self
            .state
            .with(|state| state.presence_loaded && state.presence_account_user_id == user_id);
        if loaded {
            return;
        }
        let digests: HashSet<String> = RemoteMediaDiskCache::list_entries(user_id.as_deref())
            .into_iter()
            .map(|entry| entry.id)
            .collect();
        self.state.with(|state| {
            state.presence_account_user_id = user_id;
            state.presence_loaded = true;
            state.present_digests = digests;
        });
    }

    fn schedule_publish(&self) {
        let registration = self.state.with(|state| {
            if state.publish_task.is_some() || state.pending_changes.is_empty() {
                return None;
            }
            let (handle, registration) = AbortHandle::new_pair();
            state.publish_task = Some(handle);
            Some(registration)
        });
        let Some(registration) = registration else {
            return;
        };
        let flush = RiseMediaTransferHubFlushOnDrop { hub: self.this.clone() };
        let cadence = self.publish_cadence;
        RiseAccountRuntime::current().spawn(async move {
            let _flush = flush;
            let _ = Abortable::new(tokio::time::sleep(cadence), registration).await;
        });
    }

    fn flush(&self) {
        self.state.with(|state| {
            state.publish_task = None;
            let changes = std::mem::take(&mut state.pending_changes);
            for (key, change) in &changes {
                let Some(identifiers) = state.subscriptions_by_key.get(key) else {
                    continue;
                };
                for identifier in identifiers {
                    let Some(subscription) = state.subscriptions.get(identifier) else {
                        continue;
                    };
                    if change.version <= subscription.after_version {
                        continue;
                    }
                    subscription.continuation.yield_state(change.state.clone());
                }
            }
        });
    }

    fn remove_subscription(&self, identifier: Uuid) {
        self.state.with(|state| state.remove_subscription(identifier));
    }
}

impl RiseMediaTransferHubState {
    fn park(&mut self, requests: Vec<RiseMediaTransferRequest>) {
        for request in requests {
            let Some(user_id) = request.account_user_id.clone() else {
                continue;
            };
            let parked = self.parked_by_account.entry(user_id).or_default();
            parked.retain(|existing| existing.cache_key != request.cache_key);
            parked.push(request);
            if parked.len() > RiseMediaTransferHub::MAXIMUM_PARKED_PER_ACCOUNT {
                let overflow = parked.len() - RiseMediaTransferHub::MAXIMUM_PARKED_PER_ACCOUNT;
                parked.drain(..overflow);
            }
        }
    }

    fn drop_parked(&mut self, user_id: &str, cache_key: &str) {
        let Some(parked) = self.parked_by_account.get_mut(user_id) else {
            return;
        };
        parked.retain(|existing| existing.cache_key != cache_key);
        if parked.is_empty() {
            self.parked_by_account.remove(user_id);
        }
    }

    fn pump(&mut self) {
        let Some(foreground) = self.foreground.clone() else {
            return;
        };
        while self.running_count < RiseMediaTransferHub::MAXIMUM_CONCURRENT_TRANSFERS {
            let Some(key) = self.queue.pop_front() else {
                return;
            };
            let Some(entry) = self.active.get_mut(&key) else {
                continue;
            };
            if entry.task.is_some() {
                continue;
            }
            let (handle, registration) = AbortHandle::new_pair();
            let launch = RiseMediaTransferHubLaunch {
                key: key.clone(),
                request: entry.request.clone(),
                registration,
            };
            if foreground.unbounded_send(launch).is_err() {
                self.foreground = None;
                self.queue.push_front(key);
                return;
            }
            entry.task = Some(handle);
            self.running_count += 1;
        }
    }

    fn resume_waiters(&mut self, key: &RiseMediaTransferKey, result: Result<PathBuf, RemoteMediaTransferError>) {
        let Some(pending) = self.waiters.remove(key) else {
            return;
        };
        for waiter in pending.into_values() {
            let _ = waiter.send(result.clone());
        }
    }

    fn release_slot(&mut self, key: &RiseMediaTransferKey) {
        let Some(entry) = self.active.remove(key) else {
            return;
        };
        self.queue.retain(|queued| queued != key);
        if entry.task.is_some() {
            self.running_count = (self.running_count - 1).max(0);
        }
    }

    fn resolved_state(&self, key: &RiseMediaTransferKey) -> RiseMediaTransferState {
        if let Some(known) = self.states.get(key) {
            return known.clone();
        }
        let phase = if self.is_present(key) {
            RiseMediaTransferPhase::Cached
        } else {
            RiseMediaTransferPhase::Idle
        };
        RiseMediaTransferState {
            key: key.clone(),
            phase,
            downloaded_bytes: 0,
            total_bytes: 0,
        }
    }

    fn is_present(&self, key: &RiseMediaTransferKey) -> bool {
        self.present_digests
            .contains(&RemoteMediaDiskCache::key_digest(&key.cache_key))
    }

    fn publish(&mut self, state: RiseMediaTransferState, state_capacity: usize) {
        let key = state.key.clone();
        if self.states.get(&key) == Some(&state) {
            return;
        }
        if !self.states.contains_key(&key) {
            self.state_order.push(key.clone());
        }
        if state.is_active() {
            self.settled_at.remove(&key);
        } else {
            self.settled_at.insert(key.clone(), RiseClock::monotonic());
        }
        self.states.insert(key.clone(), state.clone());
        self.mutation_version = self.mutation_version.wrapping_add(1);
        self.pending_changes.insert(
            key,
            RiseMediaTransferHubPendingChange {
                version: self.mutation_version,
                state,
            },
        );
        self.evict_states_if_needed(state_capacity);
    }

    fn remove_subscription(&mut self, identifier: Uuid) {
        let Some(subscription) = self.subscriptions.remove(&identifier) else {
            return;
        };
        self.subscription_order.retain(|existing| *existing != identifier);
        if let Some(identifiers) = self.subscriptions_by_key.get_mut(&subscription.key) {
            identifiers.remove(&identifier);
            if identifiers.is_empty() {
                self.subscriptions_by_key.remove(&subscription.key);
            }
        }
    }

    fn evict_subscriptions_if_needed(&mut self, subscriber_capacity: usize) {
        while self.subscription_order.len() > subscriber_capacity {
            let Some(identifier) = self.subscription_order.first().copied() else {
                return;
            };
            let Some(continuation) = self
                .subscriptions
                .get(&identifier)
                .map(|subscription| subscription.continuation.clone())
            else {
                self.subscription_order.remove(0);
                continue;
            };
            self.remove_subscription(identifier);
            continuation.finish();
        }
    }

    fn evict_states_if_needed(&mut self, state_capacity: usize) {
        while self.state_order.len() > state_capacity {
            let Some(index) = self
                .state_order
                .iter()
                .position(|key| !self.active.contains_key(key) && !self.subscriptions_by_key.contains_key(key))
            else {
                return;
            };
            let key = self.state_order.remove(index);
            self.states.remove(&key);
            self.settled_at.remove(&key);
        }
    }
}
