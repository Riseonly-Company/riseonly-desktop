use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::{Arc, Weak};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use parking_lot::Mutex;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use tokio::sync::oneshot;

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_store::rise_store_models::{RiseStoreCommit, RiseStoreError, RiseStoreViewKey};

fn rise_store_error_limit(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

pub(crate) struct RiseStoreReadBudget {
    maximum_items: usize,
    maximum_bytes: usize,
    consumed_items: usize,
    consumed_bytes: usize,
}

impl RiseStoreReadBudget {
    pub(crate) fn new(maximum_items: usize, maximum_bytes: usize) -> Self {
        Self {
            maximum_items,
            maximum_bytes,
            consumed_items: 0,
            consumed_bytes: 0,
        }
    }

    pub(crate) fn consume(&mut self, item_bytes: usize) -> Result<(), RiseStoreError> {
        let Some(next_items) = self
            .consumed_items
            .checked_add(1)
            .filter(|next| *next <= self.maximum_items)
        else {
            return Err(RiseStoreError::ReadResultItemLimitReached(
                rise_store_error_limit(self.maximum_items),
            ));
        };
        let Some(next_bytes) = self
            .consumed_bytes
            .checked_add(item_bytes)
            .filter(|next| *next <= self.maximum_bytes)
        else {
            return Err(RiseStoreError::ReadResultByteLimitReached(
                rise_store_error_limit(self.maximum_bytes),
            ));
        };
        self.consumed_items = next_items;
        self.consumed_bytes = next_bytes;
        Ok(())
    }
}

struct RiseStoreAdmissionGateWaiter {
    identifier: Uuid,
    continuation: oneshot::Sender<()>,
}

#[derive(Default)]
struct RiseStoreAdmissionGateState {
    is_held: bool,
    waiters: VecDeque<RiseStoreAdmissionGateWaiter>,
    critical_waiter: Option<oneshot::Sender<()>>,
}

pub(crate) struct RiseStoreAdmissionGate {
    maximum_waiters: usize,
    state: RiseIsolated<RiseStoreAdmissionGateState>,
}

impl RiseStoreAdmissionGate {
    pub(crate) fn new(maximum_waiters: usize) -> Self {
        assert!(maximum_waiters > 0);
        Self {
            maximum_waiters,
            state: RiseIsolated::new(RiseStoreAdmissionGateState::default()),
        }
    }

    pub(crate) async fn acquire(&self) -> Result<(), RiseStoreError> {
        let identifier = Uuid::new_v4();
        let maximum_waiters = self.maximum_waiters;
        let continuation = self.state.with(|state| {
            if !state.is_held {
                state.is_held = true;
                return Ok(None);
            }
            if state.waiters.len() >= maximum_waiters {
                return Err(RiseStoreError::AdmissionQueueFull {
                    limit: rise_store_error_limit(maximum_waiters),
                });
            }
            let (sender, receiver) = oneshot::channel();
            state.waiters.push_back(RiseStoreAdmissionGateWaiter {
                identifier,
                continuation: sender,
            });
            Ok(Some(receiver))
        })?;
        let Some(continuation) = continuation else {
            return Ok(());
        };

        let waiting = RiseStoreAdmissionGateWaitGuard {
            gate: self,
            kind: RiseStoreAdmissionGateWaitKind::Ordinary(identifier),
            is_armed: true,
        };
        let _ = continuation.await;
        waiting.disarm();
        Ok(())
    }

    pub(crate) async fn acquire_critical(&self) {
        let continuation = self.state.with(|state| {
            if !state.is_held {
                state.is_held = true;
                return None;
            }
            assert!(state.critical_waiter.is_none());
            let (sender, receiver) = oneshot::channel();
            state.critical_waiter = Some(sender);
            Some(receiver)
        });
        let Some(continuation) = continuation else {
            return;
        };

        let waiting = RiseStoreAdmissionGateWaitGuard {
            gate: self,
            kind: RiseStoreAdmissionGateWaitKind::Critical,
            is_armed: true,
        };
        let _ = continuation.await;
        waiting.disarm();
    }

    pub(crate) async fn release(&self) {
        self.release_turn();
    }

    fn release_turn(&self) {
        let next = self.state.with(|state| {
            if let Some(critical_waiter) = state.critical_waiter.take() {
                return Some(critical_waiter);
            }
            match state.waiters.pop_front() {
                Some(waiter) => Some(waiter.continuation),
                None => {
                    state.is_held = false;
                    None
                }
            }
        });
        if let Some(continuation) = next {
            let _ = continuation.send(());
        }
    }

    fn cancel(&self, kind: RiseStoreAdmissionGateWaitKind) -> bool {
        self.state.with(|state| match kind {
            RiseStoreAdmissionGateWaitKind::Ordinary(identifier) => {
                let Some(index) = state
                    .waiters
                    .iter()
                    .position(|waiter| waiter.identifier == identifier)
                else {
                    return false;
                };
                state.waiters.remove(index);
                true
            }
            RiseStoreAdmissionGateWaitKind::Critical => state.critical_waiter.take().is_some(),
        })
    }
}

#[derive(Clone, Copy)]
enum RiseStoreAdmissionGateWaitKind {
    Ordinary(Uuid),
    Critical,
}

struct RiseStoreAdmissionGateWaitGuard<'gate> {
    gate: &'gate RiseStoreAdmissionGate,
    kind: RiseStoreAdmissionGateWaitKind,
    is_armed: bool,
}

impl RiseStoreAdmissionGateWaitGuard<'_> {
    fn disarm(mut self) {
        self.is_armed = false;
    }
}

impl Drop for RiseStoreAdmissionGateWaitGuard<'_> {
    fn drop(&mut self) {
        if !self.is_armed {
            return;
        }
        let was_waiting = self.gate.cancel(self.kind);
        if !was_waiting {
            self.gate.release_turn();
        }
    }
}

type RiseStoreCloseCoordinatorFlight = Shared<BoxFuture<'static, Result<(), RiseStoreError>>>;

#[derive(Default)]
pub(crate) struct RiseStoreCloseCoordinator {
    flight: RiseIsolated<Option<RiseStoreCloseCoordinatorFlight>>,
}

impl RiseStoreCloseCoordinator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn run<Operation, Flight>(
        &self,
        operation: Operation,
    ) -> Result<(), RiseStoreError>
    where
        Operation: FnOnce() -> Flight,
        Flight: Future<Output = Result<(), RiseStoreError>> + Send + 'static,
    {
        let candidate = operation().boxed().shared();
        let (flight, is_owner) = self.flight.with(|slot| {
            if let Some(running) = slot.as_ref().filter(|flight| flight.peek().is_none()) {
                return (running.clone(), false);
            }
            *slot = Some(candidate.clone());
            (candidate, true)
        });
        if is_owner {
            drop(RiseAccountRuntime::current().spawn(flight.clone()));
        }
        let result = flight.clone().await;
        if is_owner {
            self.flight.with(|slot| {
                if slot.as_ref().is_some_and(|current| current.ptr_eq(&flight)) {
                    *slot = None;
                }
            });
        }
        result
    }
}

pub type RiseStoreUpdateSequenceElement = RiseStoreCommit;

#[derive(Clone)]
pub struct RiseStoreUpdateSequence {
    lifetime: Arc<RiseStoreUpdateSequenceLifetime>,
}

pub struct RiseStoreUpdateSequenceAsyncIterator {
    lifetime: Arc<RiseStoreUpdateSequenceLifetime>,
}

impl RiseStoreUpdateSequenceAsyncIterator {
    pub async fn next(&mut self) -> Option<RiseStoreCommit> {
        self.lifetime.channel.next().await
    }
}

struct RiseStoreUpdateSequenceLifetime {
    channel: Arc<RiseStoreObservationChannel>,
}

impl Drop for RiseStoreUpdateSequenceLifetime {
    fn drop(&mut self) {
        self.channel.finish();
    }
}

impl RiseStoreUpdateSequence {
    fn new(channel: Arc<RiseStoreObservationChannel>) -> Self {
        Self {
            lifetime: Arc::new(RiseStoreUpdateSequenceLifetime { channel }),
        }
    }

    pub fn make_async_iterator(&self) -> RiseStoreUpdateSequenceAsyncIterator {
        RiseStoreUpdateSequenceAsyncIterator {
            lifetime: Arc::clone(&self.lifetime),
        }
    }
}

type RiseStoreObservationChannelTermination = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct RiseStoreObservationChannelState {
    pending: Option<RiseStoreCommit>,
    waiter: Option<oneshot::Sender<Option<RiseStoreCommit>>>,
    is_finished: bool,
    termination: Option<RiseStoreObservationChannelTermination>,
}

struct RiseStoreObservationChannel {
    maximum_pending_changed_views: usize,
    lock: Mutex<RiseStoreObservationChannelState>,
}

impl RiseStoreObservationChannel {
    fn new(maximum_pending_changed_views: usize) -> Self {
        assert!(maximum_pending_changed_views > 0);
        Self {
            maximum_pending_changed_views,
            lock: Mutex::new(RiseStoreObservationChannelState::default()),
        }
    }

    fn install_termination(&self, termination: RiseStoreObservationChannelTermination) {
        let mut state = self.lock.lock();
        if state.is_finished {
            drop(state);
            termination();
            return;
        }
        state.termination = Some(termination);
    }

    fn publish(&self, commit: RiseStoreCommit) {
        let mut state = self.lock.lock();
        if state.is_finished {
            return;
        }
        if let Some(waiter) = state.waiter.take() {
            drop(state);
            let _ = waiter.send(Some(commit));
            return;
        }
        let next = match state.pending.take() {
            Some(older) => self.coalesce(&older, &commit),
            None => commit,
        };
        state.pending = Some(next);
    }

    async fn next(&self) -> Option<RiseStoreCommit> {
        let receiver = {
            let mut state = self.lock.lock();
            if let Some(pending) = state.pending.take() {
                return Some(pending);
            }
            if state.is_finished || state.waiter.is_some() {
                return None;
            }
            let (sender, receiver) = oneshot::channel();
            state.waiter = Some(sender);
            receiver
        };

        let waiting = RiseStoreObservationChannelWaitGuard {
            channel: self,
            is_armed: true,
        };
        let commit = receiver.await.ok().flatten();
        waiting.disarm();
        commit
    }

    fn finish(&self) {
        let (waiter, termination) = {
            let mut state = self.lock.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            state.pending = None;
            (state.waiter.take(), state.termination.take())
        };
        if let Some(waiter) = waiter {
            let _ = waiter.send(None);
        }
        if let Some(termination) = termination {
            termination();
        }
    }

    fn coalesce(&self, older: &RiseStoreCommit, newer: &RiseStoreCommit) -> RiseStoreCommit {
        let revision = older.revision.max(newer.revision);
        let committed_at_milliseconds = older
            .committed_at_milliseconds
            .max(newer.committed_at_milliseconds);
        if older.requires_full_refresh || newer.requires_full_refresh {
            return Self::commit_requiring_full_refresh(revision, committed_at_milliseconds);
        }
        let changed_views: HashSet<RiseStoreViewKey> = older
            .changed_views
            .union(&newer.changed_views)
            .cloned()
            .collect();
        if changed_views.len() > self.maximum_pending_changed_views {
            return Self::commit_requiring_full_refresh(revision, committed_at_milliseconds);
        }
        RiseStoreCommit::new(revision, changed_views, committed_at_milliseconds)
    }

    fn commit_requiring_full_refresh(revision: i64, committed_at_milliseconds: i64) -> RiseStoreCommit {
        let mut commit = RiseStoreCommit::new(revision, HashSet::new(), committed_at_milliseconds);
        commit.requires_full_refresh = true;
        commit
    }
}

struct RiseStoreObservationChannelWaitGuard<'channel> {
    channel: &'channel RiseStoreObservationChannel,
    is_armed: bool,
}

impl RiseStoreObservationChannelWaitGuard<'_> {
    fn disarm(mut self) {
        self.is_armed = false;
    }
}

impl Drop for RiseStoreObservationChannelWaitGuard<'_> {
    fn drop(&mut self) {
        if self.is_armed {
            self.channel.finish();
        }
    }
}

#[derive(Clone)]
struct RiseStoreChangeHubObserver {
    views: Arc<HashSet<RiseStoreViewKey>>,
    channel: Arc<RiseStoreObservationChannel>,
}

#[derive(Default)]
struct RiseStoreChangeHubState {
    observers: HashMap<Uuid, RiseStoreChangeHubObserver>,
    is_finished: bool,
}

pub(crate) struct RiseStoreChangeHub {
    maximum_observers: usize,
    maximum_pending_changed_views: usize,
    lock: Arc<Mutex<RiseStoreChangeHubState>>,
}

impl RiseStoreChangeHub {
    pub(crate) fn new(maximum_observers: usize, maximum_pending_changed_views: usize) -> Self {
        Self {
            maximum_observers,
            maximum_pending_changed_views,
            lock: Arc::new(Mutex::new(RiseStoreChangeHubState::default())),
        }
    }

    pub(crate) fn stream(
        &self,
        matching: &HashSet<RiseStoreViewKey>,
    ) -> Result<RiseStoreUpdateSequence, RiseStoreError> {
        let mut state = self.lock.lock();
        if state.is_finished {
            let channel = Arc::new(RiseStoreObservationChannel::new(
                self.maximum_pending_changed_views,
            ));
            channel.finish();
            return Ok(RiseStoreUpdateSequence::new(channel));
        }
        if state.observers.len() >= self.maximum_observers {
            return Err(RiseStoreError::ObserverLimitReached(rise_store_error_limit(
                self.maximum_observers,
            )));
        }

        let identifier = Uuid::new_v4();
        let channel = Arc::new(RiseStoreObservationChannel::new(
            self.maximum_pending_changed_views,
        ));
        state.observers.insert(
            identifier,
            RiseStoreChangeHubObserver {
                views: Arc::new(matching.clone()),
                channel: Arc::clone(&channel),
            },
        );
        let hub: Weak<Mutex<RiseStoreChangeHubState>> = Arc::downgrade(&self.lock);
        channel.install_termination(Box::new(move || {
            if let Some(hub) = hub.upgrade() {
                Self::remove(&hub, identifier);
            }
        }));
        Ok(RiseStoreUpdateSequence::new(channel))
    }

    pub(crate) fn publish(&self, commit: &RiseStoreCommit) {
        let current: Vec<RiseStoreChangeHubObserver> = {
            let state = self.lock.lock();
            if state.is_finished {
                Vec::new()
            } else {
                state.observers.values().cloned().collect()
            }
        };
        for observer in current {
            if observer.views.is_empty() {
                observer.channel.publish(commit.clone());
                continue;
            }
            let relevant_views: HashSet<RiseStoreViewKey> = observer
                .views
                .intersection(&commit.changed_views)
                .cloned()
                .collect();
            if relevant_views.is_empty() {
                continue;
            }
            let mut relevant_commit = RiseStoreCommit::new(commit.revision, relevant_views, commit.committed_at_milliseconds);
            relevant_commit.requires_full_refresh = commit.requires_full_refresh;
            observer.channel.publish(relevant_commit);
        }
    }

    pub(crate) fn finish(&self) {
        let channels: Vec<Arc<RiseStoreObservationChannel>> = {
            let mut state = self.lock.lock();
            if state.is_finished {
                return;
            }
            state.is_finished = true;
            state
                .observers
                .drain()
                .map(|(_, observer)| observer.channel)
                .collect()
        };
        for channel in channels {
            channel.finish();
        }
    }

    fn remove(lock: &Mutex<RiseStoreChangeHubState>, identifier: Uuid) {
        lock.lock().observers.remove(&identifier);
    }
}
