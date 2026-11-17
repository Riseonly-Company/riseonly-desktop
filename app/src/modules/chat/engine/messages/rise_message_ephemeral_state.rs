use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures::Stream;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::rise_isolated::RiseIsolated;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use tokio::task::JoinHandle;

use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;

const DEFAULT_TTL: Duration = Duration::from_secs(6);
const MAXIMUM_PEERS_PER_SCOPE: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseMessageTypingPeer {
    pub user_id: String,
    pub name: String,
    pub action: Option<String>,
    pub status_text: Option<String>,
    pub updated_at: Date,
}

#[derive(Default)]
struct RiseMessageTypingChannel {
    latest: Option<Vec<RiseMessageTypingPeer>>,
    is_finished: bool,
    waker: Option<Waker>,
}

/// keeps the newest only, a lagging reader sees just the latest peer list
#[derive(Clone)]
struct RiseMessageTypingContinuation {
    channel: Arc<Mutex<RiseMessageTypingChannel>>,
}

impl RiseMessageTypingContinuation {
    fn yield_value(&self, value: Vec<RiseMessageTypingPeer>) {
        let mut channel = self.channel.lock().unwrap_or_else(PoisonError::into_inner);
        channel.latest = Some(value);
        if let Some(waker) = channel.waker.take() {
            waker.wake();
        }
    }

    fn finish(&self) {
        let mut channel = self.channel.lock().unwrap_or_else(PoisonError::into_inner);
        channel.is_finished = true;
        if let Some(waker) = channel.waker.take() {
            waker.wake();
        }
    }
}

pub struct RiseMessageTypingStream {
    channel: Arc<Mutex<RiseMessageTypingChannel>>,
    identifier: u64,
    scope: RiseMessageScope,
    hub: Weak<RiseIsolated<RiseMessageTypingHubState>>,
}

impl Stream for RiseMessageTypingStream {
    type Item = Vec<RiseMessageTypingPeer>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut channel = self.channel.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(value) = channel.latest.take() {
            return Poll::Ready(Some(value));
        }
        if channel.is_finished {
            return Poll::Ready(None);
        }
        channel.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for RiseMessageTypingStream {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade() {
            hub.with(|state| state.remove_continuation(self.identifier, &self.scope));
        }
    }
}

#[derive(Clone)]
struct Entry {
    peer: RiseMessageTypingPeer,
    generation: u64,
    started_at: Date,
}

#[derive(Default)]
struct RiseMessageTypingHubState {
    peers: HashMap<RiseMessageScope, HashMap<String, Entry>>,
    expiry_tasks: HashMap<RiseMessageScope, HashMap<String, JoinHandle<()>>>,
    continuations: HashMap<RiseMessageScope, HashMap<u64, RiseMessageTypingContinuation>>,
    list_continuation_ids: HashSet<u64>,
    identifier_clock: u64,
    generation_clock: u64,
}

impl RiseMessageTypingHubState {
    fn cancel_expiry(&mut self, scope: &RiseMessageScope, user_id: &str) {
        if let Some(task) = self.expiry_tasks.get_mut(scope).and_then(|tasks| tasks.remove(user_id)) {
            task.abort();
        }
    }

    fn expire(&mut self, scope: &RiseMessageScope, user_id: &str, generation: u64) {
        let is_current = self
            .peers
            .get(scope)
            .and_then(|peers| peers.get(user_id))
            .is_some_and(|entry| entry.generation == generation);
        if !is_current {
            return;
        }
        if let Some(peers) = self.peers.get_mut(scope) {
            peers.remove(user_id);
        }
        if let Some(tasks) = self.expiry_tasks.get_mut(scope) {
            tasks.remove(user_id);
        }
        self.remove_empty_state(scope);
        self.publish(scope);
    }

    fn snapshot(&self, scope: &RiseMessageScope) -> Vec<RiseMessageTypingPeer> {
        let Some(peers) = self.peers.get(scope) else {
            return Vec::new();
        };
        let mut entries: Vec<&Entry> = peers.values().collect();
        entries.sort_by(|left, right| {
            left.started_at.cmp(&right.started_at).then_with(|| left.peer.user_id.cmp(&right.peer.user_id))
        });
        entries.into_iter().map(|entry| entry.peer.clone()).collect()
    }

    fn publish(&self, scope: &RiseMessageScope) {
        let value = self.snapshot(scope);
        if let Some(continuations) = self.continuations.get(scope) {
            for continuation in continuations.values() {
                continuation.yield_value(value.clone());
            }
        }
    }

    fn remove_continuation(&mut self, identifier: u64, scope: &RiseMessageScope) {
        self.list_continuation_ids.remove(&identifier);
        if let Some(continuations) = self.continuations.get_mut(scope) {
            continuations.remove(&identifier);
            if continuations.is_empty() {
                self.continuations.remove(scope);
            }
        }
    }

    fn remove_empty_state(&mut self, scope: &RiseMessageScope) {
        if self.peers.get(scope).is_some_and(HashMap::is_empty) {
            self.peers.remove(scope);
        }
        if self.expiry_tasks.get(scope).is_some_and(HashMap::is_empty) {
            self.expiry_tasks.remove(scope);
        }
    }
}

pub struct RiseMessageTypingHub {
    ttl: Duration,
    state: Arc<RiseIsolated<RiseMessageTypingHubState>>,
}

impl Default for RiseMessageTypingHub {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseMessageTypingHub {
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_TTL)
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self { ttl, state: Arc::new(RiseIsolated::new(RiseMessageTypingHubState::default())) }
    }

    pub async fn stream(&self, scope: &RiseMessageScope, survives_message_release: bool) -> RiseMessageTypingStream {
        let channel = Arc::new(Mutex::new(RiseMessageTypingChannel::default()));
        let continuation = RiseMessageTypingContinuation { channel: channel.clone() };
        let identifier = self.state.with(|state| {
            state.identifier_clock = state.identifier_clock.wrapping_add(1);
            let identifier = state.identifier_clock;
            state.continuations.entry(scope.clone()).or_default().insert(identifier, continuation.clone());
            if survives_message_release {
                state.list_continuation_ids.insert(identifier);
            }
            continuation.yield_value(state.snapshot(scope));
            identifier
        });
        RiseMessageTypingStream { channel, identifier, scope: scope.clone(), hub: Arc::downgrade(&self.state) }
    }

    pub async fn update(
        &self,
        scopes: &[RiseMessageScope],
        user_id: &str,
        name: &str,
        is_typing: bool,
        action: Option<String>,
        status_text: Option<String>,
    ) {
        if user_id.is_empty() {
            return;
        }
        let runtime = RiseAccountRuntime::current();
        let hub = Arc::downgrade(&self.state);
        let ttl = self.ttl;
        let unique: HashSet<&RiseMessageScope> = scopes.iter().collect();
        self.state.with(|state| {
            for scope in unique {
                state.cancel_expiry(scope, user_id);
                if is_typing {
                    state.generation_clock = state.generation_clock.wrapping_add(1);
                    let generation = state.generation_clock;
                    let now = RiseClock::now();
                    let started_at = state
                        .peers
                        .get(scope)
                        .and_then(|peers| peers.get(user_id))
                        .map_or(now, |entry| entry.started_at);
                    let peers = state.peers.entry(scope.clone()).or_default();
                    peers.insert(
                        user_id.to_owned(),
                        Entry {
                            peer: RiseMessageTypingPeer {
                                user_id: user_id.to_owned(),
                                name: name.to_owned(),
                                action: action.clone(),
                                status_text: status_text.clone(),
                                updated_at: now,
                            },
                            generation,
                            started_at,
                        },
                    );
                    let oldest = (peers.len() > MAXIMUM_PEERS_PER_SCOPE)
                        .then(|| {
                            peers
                                .iter()
                                .min_by_key(|(_, entry)| entry.peer.updated_at)
                                .map(|(user_id, _)| user_id.clone())
                        })
                        .flatten()
                        .filter(|oldest| oldest != user_id);
                    if let Some(oldest) = oldest {
                        peers.remove(&oldest);
                        state.cancel_expiry(scope, &oldest);
                    }
                    let task_hub = hub.clone();
                    let task_scope = scope.clone();
                    let task_user_id = user_id.to_owned();
                    let task = runtime.spawn(async move {
                        tokio::time::sleep(ttl).await;
                        if let Some(hub) = task_hub.upgrade() {
                            hub.with(|state| state.expire(&task_scope, &task_user_id, generation));
                        }
                    });
                    state.expiry_tasks.entry(scope.clone()).or_default().insert(user_id.to_owned(), task);
                } else {
                    if let Some(peers) = state.peers.get_mut(scope) {
                        peers.remove(user_id);
                    }
                    state.remove_empty_state(scope);
                }
                state.publish(scope);
            }
        });
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| {
            for tasks in state.expiry_tasks.values() {
                for task in tasks.values() {
                    task.abort();
                }
            }
            state.expiry_tasks.clear();
            state.peers.clear();
            let all: Vec<RiseMessageTypingContinuation> =
                state.continuations.drain().flat_map(|(_, continuations)| continuations.into_values()).collect();
            state.list_continuation_ids.clear();
            for continuation in all {
                continuation.finish();
            }
        });
    }

    pub async fn release(&self, scope: &RiseMessageScope) {
        self.state.with(|state| {
            if scope.topic_id.is_some() || scope.thread_root_id.is_some() {
                if let Some(tasks) = state.expiry_tasks.remove(scope) {
                    for task in tasks.values() {
                        task.abort();
                    }
                }
                state.peers.remove(scope);
            }
            let released: Vec<(u64, RiseMessageTypingContinuation)> = state
                .continuations
                .get(scope)
                .map(|continuations| {
                    continuations
                        .iter()
                        .filter(|(identifier, _)| !state.list_continuation_ids.contains(identifier))
                        .map(|(identifier, continuation)| (*identifier, continuation.clone()))
                        .collect()
                })
                .unwrap_or_default();
            for (identifier, continuation) in released {
                state.remove_continuation(identifier, scope);
                continuation.finish();
            }
        });
    }
}
