use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use futures::future::BoxFuture;
use rise_canon::rise_upload::PROGRESS_PUBLISH_CADENCE_MS;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

const PROGRESS_HUB_DEFAULT_CAPACITY: usize = 1_024;
const PROGRESS_HUB_DEFAULT_SUBSCRIBER_CAPACITY: usize = 1_024;
const REALTIME_HUB_DEFAULT_CAPACITY: usize = 256;
const CANCELLATION_HUB_DEFAULT_CAPACITY: usize = 1_024;
const STREAM_BUFFER: usize = 1;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RiseUploadKey {
    #[serde(rename = "ownerID")]
    pub owner_id: String,
    #[serde(rename = "slotIndex")]
    pub slot_index: i64,
}

impl RiseUploadKey {
    pub fn new(owner_id: impl Into<String>, slot_index: i64) -> Self {
        let owner_id = owner_id.into();
        assert!(!owner_id.is_empty());
        assert!(slot_index >= 0);
        Self {
            owner_id,
            slot_index,
        }
    }
}

/// local upload slot phase, carries the error code the cell renders
/// the canon Phase is the wire spelling with no payload so this stays separate
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseUploadPhase {
    Preparing,
    Uploading,
    Processing,
    Completed,
    Failed { code: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseUploadProgress {
    key: RiseUploadKey,
    fraction: f64,
    completed_bytes: i64,
    total_bytes: i64,
    phase: RiseUploadPhase,
}

impl RiseUploadProgress {
    pub fn new(
        key: RiseUploadKey,
        fraction: f64,
        completed_bytes: i64,
        total_bytes: i64,
        phase: RiseUploadPhase,
    ) -> Self {
        let fraction = 1.0_f64.min(0.0_f64.max(fraction));
        let total_bytes = total_bytes.max(0);
        Self {
            key,
            fraction,
            completed_bytes: completed_bytes.max(0).min(total_bytes),
            total_bytes,
            phase,
        }
    }

    pub fn key(&self) -> &RiseUploadKey {
        &self.key
    }

    pub fn fraction(&self) -> f64 {
        self.fraction
    }

    pub fn completed_bytes(&self) -> i64 {
        self.completed_bytes
    }

    pub fn total_bytes(&self) -> i64 {
        self.total_bytes
    }

    pub fn phase(&self) -> &RiseUploadPhase {
        &self.phase
    }
}

/// scoped and coalesced change set from the progress hub
/// first element has every active matching upload then only changes and removals
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiseUploadProgressDelta {
    pub values: HashMap<RiseUploadKey, RiseUploadProgress>,
    pub removed_keys: HashSet<RiseUploadKey>,
}

impl RiseUploadProgressDelta {
    pub fn new(
        values: HashMap<RiseUploadKey, RiseUploadProgress>,
        removed_keys: HashSet<RiseUploadKey>,
    ) -> Self {
        Self {
            values,
            removed_keys,
        }
    }
}

/// stream from the progress hub, ends when the hub shuts down or drops the subscription
/// dropping it ends the subscription
pub struct RiseUploadProgressStream<Element: Clone> {
    receiver: Option<broadcast::Receiver<Element>>,
    termination: Option<RiseUploadProgressStreamTermination>,
}

struct RiseUploadProgressStreamTermination {
    hub: Weak<RiseIsolated<RiseUploadProgressHubState>>,
    identifier: Uuid,
    remove: fn(&mut RiseUploadProgressHubState, Uuid),
}

impl<Element: Clone> RiseUploadProgressStream<Element> {
    fn finished() -> Self {
        Self {
            receiver: None,
            termination: None,
        }
    }

    pub async fn next(&mut self) -> Option<Element> {
        loop {
            let result = self.receiver.as_mut()?.recv().await;
            match result {
                Ok(element) => return Some(element),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    self.receiver = None;
                    return None;
                }
            }
        }
    }
}

impl<Element: Clone> Drop for RiseUploadProgressStream<Element> {
    fn drop(&mut self) {
        let Some(termination) = self.termination.take() else { return };
        if let Some(hub) = termination.hub.upgrade() {
            hub.with(|state| (termination.remove)(state, termination.identifier));
        }
    }
}

struct RiseUploadProgressHubKeyContinuation {
    key: RiseUploadKey,
    after_version: u64,
    continuation: broadcast::Sender<Option<RiseUploadProgress>>,
}

struct RiseUploadProgressHubDeltaContinuation {
    owner_id: String,
    after_version: u64,
    continuation: broadcast::Sender<RiseUploadProgressDelta>,
}

struct RiseUploadProgressHubPendingChange {
    version: u64,
    value: Option<RiseUploadProgress>,
}

#[derive(Default)]
struct RiseUploadProgressHubDeltaAccumulator {
    values: HashMap<RiseUploadKey, RiseUploadProgress>,
    removed_keys: HashSet<RiseUploadKey>,
}

impl RiseUploadProgressHubDeltaAccumulator {
    fn apply(&mut self, key: &RiseUploadKey, value: Option<&RiseUploadProgress>) {
        if let Some(value) = value {
            self.values.insert(key.clone(), value.clone());
            self.removed_keys.remove(key);
        } else {
            self.values.remove(key);
            self.removed_keys.insert(key.clone());
        }
    }

    fn delta(self) -> RiseUploadProgressDelta {
        RiseUploadProgressDelta::new(self.values, self.removed_keys)
    }
}

struct RiseUploadProgressHubState {
    values: HashMap<RiseUploadKey, RiseUploadProgress>,
    insertion_order: VecDeque<RiseUploadKey>,
    terminal_keys: HashSet<RiseUploadKey>,
    terminal_order: VecDeque<RiseUploadKey>,
    continuations: HashMap<Uuid, broadcast::Sender<HashMap<RiseUploadKey, RiseUploadProgress>>>,
    continuation_order: VecDeque<Uuid>,
    key_continuations: HashMap<Uuid, RiseUploadProgressHubKeyContinuation>,
    key_continuation_order: VecDeque<Uuid>,
    key_continuations_by_key: HashMap<RiseUploadKey, HashSet<Uuid>>,
    delta_continuations: HashMap<Uuid, RiseUploadProgressHubDeltaContinuation>,
    delta_continuation_order: VecDeque<Uuid>,
    delta_continuations_by_owner: HashMap<String, HashSet<Uuid>>,
    pending_changes: HashMap<RiseUploadKey, RiseUploadProgressHubPendingChange>,
    mutation_version: u64,
    publish_task: Option<JoinHandle<()>>,
    is_shutdown: bool,
    capacity: usize,
    subscriber_capacity: usize,
    publish_cadence: Duration,
}

pub struct RiseUploadProgressHub {
    state: Arc<RiseIsolated<RiseUploadProgressHubState>>,
}

impl Default for RiseUploadProgressHub {
    fn default() -> Self {
        Self::new(
            PROGRESS_HUB_DEFAULT_CAPACITY,
            PROGRESS_HUB_DEFAULT_SUBSCRIBER_CAPACITY,
            Duration::from_millis(PROGRESS_PUBLISH_CADENCE_MS),
        )
    }
}

impl RiseUploadProgressHub {
    pub fn new(capacity: usize, subscriber_capacity: usize, publish_cadence: Duration) -> Self {
        Self {
            state: Arc::new(RiseIsolated::new(RiseUploadProgressHubState {
                values: HashMap::new(),
                insertion_order: VecDeque::new(),
                terminal_keys: HashSet::new(),
                terminal_order: VecDeque::new(),
                continuations: HashMap::new(),
                continuation_order: VecDeque::new(),
                key_continuations: HashMap::new(),
                key_continuation_order: VecDeque::new(),
                key_continuations_by_key: HashMap::new(),
                delta_continuations: HashMap::new(),
                delta_continuation_order: VecDeque::new(),
                delta_continuations_by_owner: HashMap::new(),
                pending_changes: HashMap::new(),
                mutation_version: 0,
                publish_task: None,
                is_shutdown: false,
                capacity: capacity.max(1),
                subscriber_capacity: subscriber_capacity.max(1),
                publish_cadence,
            })),
        }
    }

    pub async fn updates(&self) -> RiseUploadProgressStream<HashMap<RiseUploadKey, RiseUploadProgress>> {
        let hub = Arc::downgrade(&self.state);
        self.state.with(|state| {
            if state.is_shutdown {
                return RiseUploadProgressStream::finished();
            }
            let identifier = Uuid::new_v4();
            let (continuation, receiver) = broadcast::channel(STREAM_BUFFER);
            let _ = continuation.send(state.values.clone());
            state.continuations.insert(identifier, continuation);
            state.continuation_order.push_back(identifier);
            state.evict_continuations_if_needed();
            RiseUploadProgressStream {
                receiver: Some(receiver),
                termination: Some(RiseUploadProgressStreamTermination {
                    hub,
                    identifier,
                    remove: RiseUploadProgressHubState::remove_continuation,
                }),
            }
        })
    }

    /// watches one upload slot without building the full snapshot
    /// first element is the current value and none means the slot was removed or reset by begin
    pub async fn updates_for(
        &self,
        key: RiseUploadKey,
    ) -> RiseUploadProgressStream<Option<RiseUploadProgress>> {
        self.key_stream(key)
    }

    /// watches changes of all slots of owner_id
    /// first element has every active slot of that owner in one batch
    /// so the one element buffer cant lose part of the replay
    pub async fn deltas(&self, owner_id: &str) -> RiseUploadProgressStream<RiseUploadProgressDelta> {
        if owner_id.is_empty() {
            return RiseUploadProgressStream::finished();
        }
        self.delta_stream(owner_id)
    }

    pub async fn snapshot(&self) -> HashMap<RiseUploadKey, RiseUploadProgress> {
        self.state.with(|state| state.values.clone())
    }

    pub async fn publish(&self, progress: RiseUploadProgress) {
        let hub = Arc::downgrade(&self.state);
        self.state.with(|state| {
            if state.is_shutdown {
                return;
            }
            let key = progress.key.clone();
            if state.terminal_keys.contains(&key) {
                return;
            }
            if let Some(current) = state.values.get(&key) {
                let rejected = matches!(
                    (&current.phase, &progress.phase),
                    (RiseUploadPhase::Uploading, RiseUploadPhase::Preparing)
                        | (RiseUploadPhase::Processing, RiseUploadPhase::Preparing)
                        | (RiseUploadPhase::Processing, RiseUploadPhase::Uploading)
                        | (RiseUploadPhase::Completed, _)
                        | (RiseUploadPhase::Failed { .. }, _)
                );
                if rejected {
                    return;
                }
            } else {
                state.insertion_order.push_back(key.clone());
            }

            let normalized = match state.values.get(&key) {
                Some(current) if current.phase == progress.phase => RiseUploadProgress::new(
                    key.clone(),
                    current.fraction.max(progress.fraction),
                    current.completed_bytes.max(progress.completed_bytes),
                    current.total_bytes.max(progress.total_bytes),
                    progress.phase.clone(),
                ),
                _ => progress.clone(),
            };
            state.values.insert(key.clone(), normalized.clone());
            state.record_pending_change(&key, Some(normalized));

            match progress.phase {
                RiseUploadPhase::Completed | RiseUploadPhase::Failed { .. } => {
                    state.mark_terminal(&key);
                }
                RiseUploadPhase::Preparing
                | RiseUploadPhase::Uploading
                | RiseUploadPhase::Processing => {}
            }
            state.evict_if_needed();
            state.schedule_publish(hub);
        });
    }

    pub async fn begin(&self, owner_id: &str) {
        let hub = Arc::downgrade(&self.state);
        self.state.with(|state| {
            if state.is_shutdown || owner_id.is_empty() {
                return;
            }
            let keys = state.keys_owned_by(owner_id);
            for key in &keys {
                state.values.remove(key);
                state.record_pending_change(key, None);
            }
            state.terminal_keys.retain(|key| key.owner_id != owner_id);
            state.terminal_order.retain(|key| key.owner_id != owner_id);
            state.insertion_order.retain(|key| key.owner_id != owner_id);
            if !keys.is_empty() {
                state.schedule_publish(hub);
            }
        });
    }

    pub async fn remove_owner_id(&self, owner_id: &str) {
        let hub = Arc::downgrade(&self.state);
        self.state.with(|state| {
            if state.is_shutdown || owner_id.is_empty() {
                return;
            }
            let keys = state.keys_owned_by(owner_id);
            for key in &keys {
                state.values.remove(key);
                state.record_pending_change(key, None);
                state.mark_terminal(key);
            }
            state.insertion_order.retain(|key| key.owner_id != owner_id);
            state.schedule_publish(hub);
        });
    }

    pub async fn remove(&self, key: RiseUploadKey) {
        let hub = Arc::downgrade(&self.state);
        self.state.with(|state| {
            if state.is_shutdown {
                return;
            }
            if state.values.remove(&key).is_some() {
                state.record_pending_change(&key, None);
            }
            state.insertion_order.retain(|candidate| candidate != &key);
            state.mark_terminal(&key);
            state.schedule_publish(hub);
        });
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| {
            if state.is_shutdown {
                return;
            }
            state.is_shutdown = true;
            if let Some(task) = state.publish_task.take() {
                task.abort();
            }
            state.values.clear();
            state.insertion_order.clear();
            state.terminal_keys.clear();
            state.terminal_order.clear();
            state.pending_changes.clear();
            state.continuations.clear();
            state.continuation_order.clear();
            state.key_continuations.clear();
            state.key_continuation_order.clear();
            state.key_continuations_by_key.clear();
            state.delta_continuations.clear();
            state.delta_continuation_order.clear();
            state.delta_continuations_by_owner.clear();
        });
    }

    fn key_stream(&self, key: RiseUploadKey) -> RiseUploadProgressStream<Option<RiseUploadProgress>> {
        let hub = Arc::downgrade(&self.state);
        self.state.with(|state| {
            if state.is_shutdown {
                return RiseUploadProgressStream::finished();
            }
            let identifier = Uuid::new_v4();
            let (continuation, receiver) = broadcast::channel(STREAM_BUFFER);
            let _ = continuation.send(state.values.get(&key).cloned());
            state.key_continuations.insert(
                identifier,
                RiseUploadProgressHubKeyContinuation {
                    key: key.clone(),
                    after_version: state.mutation_version,
                    continuation,
                },
            );
            state.key_continuation_order.push_back(identifier);
            state
                .key_continuations_by_key
                .entry(key)
                .or_default()
                .insert(identifier);
            state.evict_key_continuations_if_needed();
            RiseUploadProgressStream {
                receiver: Some(receiver),
                termination: Some(RiseUploadProgressStreamTermination {
                    hub,
                    identifier,
                    remove: RiseUploadProgressHubState::remove_key_continuation,
                }),
            }
        })
    }

    fn delta_stream(&self, owner_id: &str) -> RiseUploadProgressStream<RiseUploadProgressDelta> {
        let hub = Arc::downgrade(&self.state);
        self.state.with(|state| {
            if state.is_shutdown {
                return RiseUploadProgressStream::finished();
            }
            let identifier = Uuid::new_v4();
            let (continuation, receiver) = broadcast::channel(STREAM_BUFFER);
            let _ = continuation.send(state.current_delta(owner_id));
            state.delta_continuations.insert(
                identifier,
                RiseUploadProgressHubDeltaContinuation {
                    owner_id: owner_id.to_string(),
                    after_version: state.mutation_version,
                    continuation,
                },
            );
            state.delta_continuation_order.push_back(identifier);
            state
                .delta_continuations_by_owner
                .entry(owner_id.to_string())
                .or_default()
                .insert(identifier);
            state.evict_delta_continuations_if_needed();
            RiseUploadProgressStream {
                receiver: Some(receiver),
                termination: Some(RiseUploadProgressStreamTermination {
                    hub,
                    identifier,
                    remove: RiseUploadProgressHubState::remove_delta_continuation,
                }),
            }
        })
    }
}

impl RiseUploadProgressHubState {
    fn keys_owned_by(&self, owner_id: &str) -> Vec<RiseUploadKey> {
        self.values
            .keys()
            .filter(|key| key.owner_id == owner_id)
            .cloned()
            .collect()
    }

    fn schedule_publish(&mut self, hub: Weak<RiseIsolated<RiseUploadProgressHubState>>) {
        if self.is_shutdown || self.publish_task.is_some() {
            return;
        }
        let cadence = self.publish_cadence;
        self.publish_task = Some(RiseAccountRuntime::current().spawn(async move {
            tokio::time::sleep(cadence).await;
            if let Some(hub) = hub.upgrade() {
                hub.with(|state| state.flush());
            }
        }));
    }

    fn flush(&mut self) {
        self.publish_task = None;
        if self.is_shutdown {
            return;
        }
        for continuation in self.continuations.values() {
            let _ = continuation.send(self.values.clone());
        }
        self.flush_deltas();
    }

    fn evict_if_needed(&mut self) {
        while self.insertion_order.len() > self.capacity {
            let Some(key) = self.insertion_order.pop_front() else { return };
            if self.values.remove(&key).is_some() {
                self.record_pending_change(&key, None);
            }
        }
    }

    fn mark_terminal(&mut self, key: &RiseUploadKey) {
        if !self.terminal_keys.insert(key.clone()) {
            return;
        }
        self.terminal_order.push_back(key.clone());
        while self.terminal_order.len() > self.capacity {
            if let Some(oldest) = self.terminal_order.pop_front() {
                self.terminal_keys.remove(&oldest);
            }
        }
    }

    fn remove_continuation(&mut self, identifier: Uuid) {
        self.continuations.remove(&identifier);
        self.continuation_order.retain(|candidate| *candidate != identifier);
    }

    fn current_delta(&self, owner_id: &str) -> RiseUploadProgressDelta {
        RiseUploadProgressDelta::new(
            self.values
                .iter()
                .filter(|(key, _)| key.owner_id == owner_id)
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            HashSet::new(),
        )
    }

    fn record_pending_change(&mut self, key: &RiseUploadKey, value: Option<RiseUploadProgress>) {
        self.mutation_version = self.mutation_version.wrapping_add(1);
        self.pending_changes.insert(
            key.clone(),
            RiseUploadProgressHubPendingChange {
                version: self.mutation_version,
                value,
            },
        );
    }

    fn flush_deltas(&mut self) {
        if self.pending_changes.is_empty() {
            return;
        }
        let changes = std::mem::take(&mut self.pending_changes);

        let mut accumulated: HashMap<Uuid, RiseUploadProgressHubDeltaAccumulator> = HashMap::new();
        for (key, change) in &changes {
            if let Some(identifiers) = self.key_continuations_by_key.get(key) {
                for identifier in identifiers {
                    let Some(subscription) = self.key_continuations.get(identifier) else {
                        continue;
                    };
                    if change.version <= subscription.after_version {
                        continue;
                    }
                    let _ = subscription.continuation.send(change.value.clone());
                }
            }

            if let Some(identifiers) = self.delta_continuations_by_owner.get(&key.owner_id) {
                for identifier in identifiers {
                    let Some(subscription) = self.delta_continuations.get(identifier) else {
                        continue;
                    };
                    if change.version <= subscription.after_version {
                        continue;
                    }
                    accumulated
                        .entry(*identifier)
                        .or_default()
                        .apply(key, change.value.as_ref());
                }
            }
        }

        for (identifier, accumulator) in accumulated {
            if let Some(subscription) = self.delta_continuations.get(&identifier) {
                let _ = subscription.continuation.send(accumulator.delta());
            }
        }
    }

    fn remove_key_continuation(&mut self, identifier: Uuid) {
        let Some(subscription) = self.key_continuations.remove(&identifier) else { return };
        self.key_continuation_order.retain(|candidate| *candidate != identifier);
        let key = subscription.key;
        if let Some(identifiers) = self.key_continuations_by_key.get_mut(&key) {
            identifiers.remove(&identifier);
            if identifiers.is_empty() {
                self.key_continuations_by_key.remove(&key);
            }
        }
    }

    fn remove_delta_continuation(&mut self, identifier: Uuid) {
        let Some(subscription) = self.delta_continuations.remove(&identifier) else { return };
        self.delta_continuation_order.retain(|candidate| *candidate != identifier);

        let owner_id = subscription.owner_id;
        if let Some(identifiers) = self.delta_continuations_by_owner.get_mut(&owner_id) {
            identifiers.remove(&identifier);
            if identifiers.is_empty() {
                self.delta_continuations_by_owner.remove(&owner_id);
            }
        }
    }

    fn evict_continuations_if_needed(&mut self) {
        while self.continuation_order.len() > self.subscriber_capacity {
            let Some(identifier) = self.continuation_order.pop_front() else { return };
            self.continuations.remove(&identifier);
        }
    }

    fn evict_key_continuations_if_needed(&mut self) {
        while self.key_continuation_order.len() > self.subscriber_capacity {
            let Some(identifier) = self.key_continuation_order.front().copied() else { return };
            self.remove_key_continuation(identifier);
        }
    }

    fn evict_delta_continuations_if_needed(&mut self) {
        while self.delta_continuation_order.len() > self.subscriber_capacity {
            let Some(identifier) = self.delta_continuation_order.front().copied() else { return };
            self.remove_delta_continuation(identifier);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseUploadRealtimeProgress {
    upload_id: String,
    fraction: f64,
    stage: Option<String>,
    message: Option<String>,
}

impl RiseUploadRealtimeProgress {
    pub fn new(
        upload_id: impl Into<String>,
        fraction: f64,
        stage: Option<String>,
        message: Option<String>,
    ) -> Self {
        let upload_id = upload_id.into();
        assert!(!upload_id.is_empty());
        Self {
            upload_id,
            fraction: 1.0_f64.min(0.0_f64.max(fraction)),
            stage,
            message,
        }
    }

    pub fn upload_id(&self) -> &str {
        &self.upload_id
    }

    pub fn fraction(&self) -> f64 {
        self.fraction
    }

    pub fn stage(&self) -> Option<&str> {
        self.stage.as_deref()
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

pub type RiseUploadRealtimeProgressHubHandler =
    Arc<dyn Fn(RiseUploadRealtimeProgress) -> BoxFuture<'static, ()> + Send + Sync>;

struct RiseUploadRealtimeProgressHubState {
    handlers: HashMap<String, RiseUploadRealtimeProgressHubHandler>,
    latest: HashMap<String, RiseUploadRealtimeProgress>,
    order: VecDeque<String>,
    capacity: usize,
}

/// joins server processing events to the local upload that started them
/// only live uploads are bound and a small latest value buffer closes the initiate vs websocket race
pub struct RiseUploadRealtimeProgressHub {
    state: RiseIsolated<RiseUploadRealtimeProgressHubState>,
}

impl Default for RiseUploadRealtimeProgressHub {
    fn default() -> Self {
        Self::new(REALTIME_HUB_DEFAULT_CAPACITY)
    }
}

impl RiseUploadRealtimeProgressHub {
    pub fn new(capacity: usize) -> Self {
        Self {
            state: RiseIsolated::new(RiseUploadRealtimeProgressHubState {
                handlers: HashMap::new(),
                latest: HashMap::new(),
                order: VecDeque::new(),
                capacity: capacity.max(1),
            }),
        }
    }

    pub async fn bind(&self, upload_id: &str, handler: RiseUploadRealtimeProgressHubHandler) {
        if upload_id.is_empty() {
            return;
        }
        let replay = self.state.with(|state| {
            state.handlers.insert(upload_id.to_string(), Arc::clone(&handler));
            state.touch(upload_id);
            state.latest.get(upload_id).cloned()
        });
        if let Some(progress) = replay {
            handler(progress).await;
        }
        self.state.with(|state| state.evict_if_needed());
    }

    pub async fn publish(&self, progress: RiseUploadRealtimeProgress) {
        let handler = self.state.with(|state| {
            state
                .latest
                .insert(progress.upload_id.clone(), progress.clone());
            state.touch(&progress.upload_id);
            state.evict_if_needed();
            state.handlers.get(&progress.upload_id).cloned()
        });
        if let Some(handler) = handler {
            handler(progress).await;
        }
    }

    pub async fn unbind(&self, upload_id: &str) {
        self.state.with(|state| {
            state.handlers.remove(upload_id);
            state.latest.remove(upload_id);
            state.order.retain(|candidate| candidate != upload_id);
        });
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| {
            state.handlers.clear();
            state.latest.clear();
            state.order.clear();
        });
    }
}

impl RiseUploadRealtimeProgressHubState {
    fn touch(&mut self, upload_id: &str) {
        self.order.retain(|candidate| candidate != upload_id);
        self.order.push_back(upload_id.to_string());
    }

    fn evict_if_needed(&mut self) {
        while self.order.len() > self.capacity {
            let Some(upload_id) = self.order.pop_front() else { return };
            self.handlers.remove(&upload_id);
            self.latest.remove(&upload_id);
        }
    }
}

type RiseUploadCancellationUnregister = Box<dyn FnOnce() + Send>;

pub struct RiseUploadCancellationRegistration {
    unregister: Mutex<Option<RiseUploadCancellationUnregister>>,
}

impl RiseUploadCancellationRegistration {
    fn new(unregister: Option<RiseUploadCancellationUnregister>) -> Self {
        Self {
            unregister: Mutex::new(unregister),
        }
    }

    pub fn cancel(&self) {
        let unregister = self
            .unregister
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(unregister) = unregister {
            unregister();
        }
    }
}

impl Drop for RiseUploadCancellationRegistration {
    fn drop(&mut self) {
        self.cancel();
    }
}

type RiseUploadCancellationHandler = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct RiseUploadCancellationSignalState {
    cancelled: bool,
    handlers: HashMap<Uuid, RiseUploadCancellationHandler>,
}

/// lock backed cancel edge that aborts in flight transport work right away
/// consumers register a light callback and never poll or touch the ui executor
#[derive(Clone, Default)]
pub struct RiseUploadCancellationSignal {
    state: Arc<Mutex<RiseUploadCancellationSignalState>>,
}

impl RiseUploadCancellationSignal {
    pub fn is_cancelled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cancelled
    }

    pub fn on_cancel(
        &self,
        handler: impl FnOnce() + Send + 'static,
    ) -> RiseUploadCancellationRegistration {
        let identifier = Uuid::new_v4();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.cancelled {
            drop(state);
            handler();
            return RiseUploadCancellationRegistration::new(None);
        }
        state.handlers.insert(identifier, Box::new(handler));
        drop(state);
        let signal = Arc::downgrade(&self.state);
        RiseUploadCancellationRegistration::new(Some(Box::new(move || {
            if let Some(signal) = signal.upgrade() {
                signal
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .handlers
                    .remove(&identifier);
            }
        })))
    }

    fn cancel(&self) {
        let handlers = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.cancelled {
                return;
            }
            state.cancelled = true;
            std::mem::take(&mut state.handlers)
        };
        for handler in handlers.into_values() {
            handler();
        }
    }

    fn cancelled() -> Self {
        let signal = Self::default();
        signal.cancel();
        signal
    }
}

#[derive(Default)]
struct RiseUploadCancellationHubSession {
    cancel_all: bool,
    cancelled_slots: HashSet<i64>,
    sealed_slots: HashSet<i64>,
    whole_signal: RiseUploadCancellationSignal,
    slot_signals: HashMap<i64, RiseUploadCancellationSignal>,
}

impl RiseUploadCancellationHubSession {
    fn signal(&mut self, slot_index: Option<i64>) -> RiseUploadCancellationSignal {
        let Some(slot_index) = slot_index else {
            if self.cancel_all {
                self.whole_signal.cancel();
            }
            return self.whole_signal.clone();
        };
        let signal = self.slot_signals.entry(slot_index).or_default().clone();
        if self.cancel_all || self.cancelled_slots.contains(&slot_index) {
            signal.cancel();
        }
        signal
    }

    fn cancel_signals(&self, slot_index: Option<i64>) {
        if let Some(slot_index) = slot_index {
            if let Some(signal) = self.slot_signals.get(&slot_index) {
                signal.cancel();
            }
            return;
        }
        self.whole_signal.cancel();
        for signal in self.slot_signals.values() {
            signal.cancel();
        }
    }
}

struct RiseUploadCancellationHubState {
    sessions: HashMap<String, RiseUploadCancellationHubSession>,
    order: VecDeque<String>,
    capacity: usize,
}

pub struct RiseUploadCancellationHub {
    state: RiseIsolated<RiseUploadCancellationHubState>,
}

impl Default for RiseUploadCancellationHub {
    fn default() -> Self {
        Self::new(CANCELLATION_HUB_DEFAULT_CAPACITY)
    }
}

impl RiseUploadCancellationHub {
    pub fn new(capacity: usize) -> Self {
        Self {
            state: RiseIsolated::new(RiseUploadCancellationHubState {
                sessions: HashMap::new(),
                order: VecDeque::new(),
                capacity: capacity.max(1),
            }),
        }
    }

    pub async fn begin(&self, owner_id: &str) {
        if owner_id.is_empty() {
            return;
        }
        self.state.with(|state| {
            if let Some(session) = state.sessions.get(owner_id) {
                session.cancel_signals(None);
            }
            state
                .sessions
                .insert(owner_id.to_string(), RiseUploadCancellationHubSession::default());
            state.touch(owner_id);
            state.evict_if_needed();
        });
    }

    /// false only when an exact slot already crossed its atomic completion point
    /// so ui cancel and outbound finalize decide without a race
    pub async fn cancel(&self, owner_id: &str, slot_index: Option<i64>) -> bool {
        if owner_id.is_empty() {
            return false;
        }
        self.state.with(|state| {
            if let Some(slot_index) = slot_index {
                if slot_index < 0 {
                    return false;
                }
                let Some(session) = state.sessions.get_mut(owner_id) else { return false };
                if session.sealed_slots.contains(&slot_index) {
                    return false;
                }
                session.cancelled_slots.insert(slot_index);
                session.cancel_signals(Some(slot_index));
            } else {
                let session = state.sessions.entry(owner_id.to_string()).or_default();
                session.cancel_all = true;
                session.cancel_signals(None);
            }
            state.touch(owner_id);
            state.evict_if_needed();
            true
        })
    }

    /// closes cancellation for a prepared slot atomically
    /// if cancel won the race the caller must throw that slot away
    pub async fn seal(&self, owner_id: &str, slot_index: i64) -> bool {
        if owner_id.is_empty() || slot_index < 0 {
            return false;
        }
        self.state.with(|state| {
            let Some(session) = state.sessions.get_mut(owner_id) else { return false };
            if session.cancel_all || session.cancelled_slots.contains(&slot_index) {
                return false;
            }
            session.sealed_slots.insert(slot_index);
            state.touch(owner_id);
            state.evict_if_needed();
            true
        })
    }

    pub async fn is_cancelled(&self, owner_id: &str, slot_index: Option<i64>) -> bool {
        self.state.with(|state| {
            let Some(session) = state.sessions.get(owner_id) else { return false };
            if session.cancel_all {
                return true;
            }
            let Some(slot_index) = slot_index else { return false };
            session.cancelled_slots.contains(&slot_index)
        })
    }

    pub async fn cancelled_slot_indices(&self, owner_id: &str) -> HashSet<i64> {
        self.state.with(|state| {
            state
                .sessions
                .get(owner_id)
                .map(|session| session.cancelled_slots.clone())
                .unwrap_or_default()
        })
    }

    pub async fn signal(
        &self,
        owner_id: &str,
        slot_index: Option<i64>,
    ) -> RiseUploadCancellationSignal {
        if owner_id.is_empty() || slot_index.is_some_and(|slot_index| slot_index < 0) {
            return RiseUploadCancellationSignal::cancelled();
        }
        self.state.with(|state| {
            let Some(session) = state.sessions.get_mut(owner_id) else {
                return RiseUploadCancellationSignal::cancelled();
            };
            let signal = session.signal(slot_index);
            state.touch(owner_id);
            state.evict_if_needed();
            signal
        })
    }

    pub async fn finish(&self, owner_id: &str) {
        self.state.with(|state| {
            if let Some(session) = state.sessions.remove(owner_id) {
                session.cancel_signals(None);
            }
            state.order.retain(|candidate| candidate != owner_id);
        });
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| {
            for session in state.sessions.values() {
                session.cancel_signals(None);
            }
            state.sessions.clear();
            state.order.clear();
        });
    }
}

impl RiseUploadCancellationHubState {
    fn touch(&mut self, owner_id: &str) {
        self.order.retain(|candidate| candidate != owner_id);
        self.order.push_back(owner_id.to_string());
    }

    fn evict_if_needed(&mut self) {
        while self.order.len() > self.capacity {
            let Some(owner_id) = self.order.pop_front() else { return };
            if let Some(session) = self.sessions.remove(&owner_id) {
                session.cancel_signals(None);
            }
        }
    }
}
