use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, LazyLock, Weak};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::BoxFuture;
use rise_canon::{RiseEvent, RiseTimingCanon};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine, RiseJSONValue, RiseWireRemoteError};
use rise_i18n::tr;
use rise_canon::rise_ledger::{HistoryFilter as RiseLedgerHistoryFilter, TransactionKind as RiseLedgerTransactionKind};
use tokio::task::{AbortHandle, JoinHandle};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_failure_bridge::CanonFailure;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::sparks::engine::core::rise_sparks_resource_repository::RiseSparksResourceRepository;
use crate::modules::sparks::engine::core::rise_sparks_rpc::RiseSparksRPC;
use crate::modules::sparks::engine::rise_sparks_engine_models::{
    RiseSparksBalanceDTO, RiseSparksBalanceEvent, RiseSparksBalanceState, RiseSparksEarningsDTO,
    RiseSparksHistoryItemDTO, RiseSparksHistoryPage, RiseSparksHistoryResponseDTO, RiseSparksLimits,
    RiseSparksLoadPhase, RiseSparksTransferResponseDTO, SparksBalanceExt,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseSparksRepositorySnapshot {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub balance: RiseSparksBalanceDTO,
    pub has_balance: bool,
    pub balance_phase: RiseSparksLoadPhase,
    pub earnings: Option<RiseSparksEarningsDTO>,
    pub earnings_phase: RiseSparksLoadPhase,
    pub is_transferring: bool,
    pub history: HashMap<RiseLedgerHistoryFilter, RiseSparksHistoryPage>,
    pub history_phase: HashMap<RiseLedgerHistoryFilter, RiseSparksLoadPhase>,
    pub loading_more: HashSet<RiseLedgerHistoryFilter>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseSparksTransferError {
    #[error("sparks transfer in flight")]
    InFlight,
}

pub type RiseSparksRepositoryPublisher =
    Arc<dyn Fn(RiseSparksRepositorySnapshot) -> BoxFuture<'static, ()> + Send + Sync>;

static BALANCE_FRESHNESS_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("sparks_balance"));
static HISTORY_FRESHNESS_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::freshness_ms("sparks_history_page"));

const PERSIST_DELAY: Duration = Duration::from_millis(300);

struct RiseSparksRepositoryState {
    balance: RiseSparksBalanceState,
    has_balance: bool,
    balance_phase: RiseSparksLoadPhase,
    balance_read_at: Option<Date>,
    balance_task: Option<AbortHandle>,
    balance_generation: u64,

    earnings: Option<RiseSparksEarningsDTO>,
    earnings_phase: RiseSparksLoadPhase,
    earnings_read_at: Option<Date>,
    earnings_task: Option<AbortHandle>,
    earnings_generation: u64,
    is_transferring: bool,

    history: HashMap<RiseLedgerHistoryFilter, RiseSparksHistoryPage>,
    history_phase: HashMap<RiseLedgerHistoryFilter, RiseSparksLoadPhase>,
    history_read_at: HashMap<RiseLedgerHistoryFilter, Date>,
    history_tasks: HashMap<RiseLedgerHistoryFilter, AbortHandle>,
    history_generation: HashMap<RiseLedgerHistoryFilter, u64>,
    loading_more: HashSet<RiseLedgerHistoryFilter>,

    persist_task: Option<AbortHandle>,
    is_shutdown: bool,
    publish: Option<RiseSparksRepositoryPublisher>,
}

pub struct RiseSparksRepository {
    rpc: RiseSparksRPC,
    resources: Arc<RiseSparksResourceRepository>,
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    this: Weak<RiseSparksRepository>,
    state: RiseIsolated<RiseSparksRepositoryState>,
}

impl RiseSparksRepository {
    pub fn new(engine: Arc<RiseEngine>, account_id: &str, account_epoch: &str, session_id: Uuid) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            rpc: RiseSparksRPC::new(engine.clone()),
            resources: Arc::new(RiseSparksResourceRepository::new(engine, account_id)),
            account_id: account_id.to_owned(),
            account_epoch: account_epoch.to_owned(),
            session_id,
            this: this.clone(),
            state: RiseIsolated::new(RiseSparksRepositoryState {
                balance: RiseSparksBalanceState::empty(),
                has_balance: false,
                balance_phase: RiseSparksLoadPhase::Idle,
                balance_read_at: None,
                balance_task: None,
                balance_generation: 0,
                earnings: None,
                earnings_phase: RiseSparksLoadPhase::Idle,
                earnings_read_at: None,
                earnings_task: None,
                earnings_generation: 0,
                is_transferring: false,
                history: HashMap::new(),
                history_phase: HashMap::new(),
                history_read_at: HashMap::new(),
                history_tasks: HashMap::new(),
                history_generation: HashMap::new(),
                loading_more: HashSet::new(),
                persist_task: None,
                is_shutdown: false,
                publish: None,
            }),
        })
    }

    pub async fn set_publisher(&self, publish: RiseSparksRepositoryPublisher) {
        self.state.with(|state| state.publish = Some(publish));
    }

    /// cache first, the last stored wallet is the seed and later reads only replace it
    pub async fn activate(&self) -> RiseSparksRepositorySnapshot {
        let cached = self.resources.cached().await.ok().flatten();
        self.state.with(|state| {
            if let Some(cached) = cached {
                state.has_balance = cached.balance.read_updated_at_ms > 0;
                state.balance = cached.balance;
                if cached.earnings.is_some() {
                    state.earnings_phase = RiseSparksLoadPhase::Loaded;
                }
                state.earnings = cached.earnings;
                for filter in cached.history.keys() {
                    state.history_phase.insert(*filter, RiseSparksLoadPhase::Loaded);
                }
                state.history = cached.history;
            }
            self.snapshot(state)
        })
    }

    pub async fn refresh_balance(&self, force: bool) {
        let started = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            if !force && Self::is_fresh(state.balance_read_at.as_ref(), *BALANCE_FRESHNESS_MS) {
                return None;
            }
            if state.balance_task.is_some() && !force {
                return None;
            }
            if let Some(task) = state.balance_task.take() {
                task.abort();
            }
            state.balance_generation = state.balance_generation.wrapping_add(1);
            let loading = !state.has_balance;
            if loading {
                state.balance_phase = RiseSparksLoadPhase::Loading;
            }
            Some((state.balance_generation, loading))
        });
        let Some((generation, loading)) = started else {
            return;
        };
        if loading {
            self.emit().await;
        }
        let rpc = self.rpc.clone();
        let this = self.this.clone();
        let (task, start) = Self::spawn_gated(async move {
            let result = rpc.balance().await;
            let Some(this) = this.upgrade() else {
                return;
            };
            match result {
                Ok(response) => this.apply_balance_read(response.balance, generation).await,
                Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                    this.finish_balance_task(generation);
                }
                Err(error) => this.fail_balance_read(&error, generation).await,
            }
        });
        self.state.with(|state| state.balance_task = Some(task.abort_handle()));
        let _ = start.send(());
        let _ = task.await;
    }

    async fn apply_balance_read(&self, server: Option<RiseSparksBalanceDTO>, generation: u64) {
        let applied = self.state.with(|state| {
            if state.is_shutdown || generation != state.balance_generation {
                return false;
            }
            state.balance_task = None;
            state.balance_read_at = Some(Date::now());
            state.balance.replace(server.unwrap_or_else(RiseSparksBalanceDTO::zero));
            state.has_balance = true;
            state.balance_phase = RiseSparksLoadPhase::Loaded;
            true
        });
        if !applied {
            return;
        }
        self.schedule_persist();
        self.emit().await;
    }

    async fn fail_balance_read(&self, error: &anyhow::Error, generation: u64) {
        let message = RiseSparksFailure::message(error);
        let failed = self.state.with(|state| {
            if state.is_shutdown || generation != state.balance_generation {
                return false;
            }
            state.balance_task = None;
            state.balance_phase =
                if state.has_balance { RiseSparksLoadPhase::Loaded } else { RiseSparksLoadPhase::Failed(message) };
            true
        });
        if failed {
            self.emit().await;
        }
    }

    fn finish_balance_task(&self, generation: u64) {
        self.state.with(|state| {
            if generation == state.balance_generation {
                state.balance_task = None;
            }
        });
    }

    /// reads received sparks with the balance freshness window, the wallet asks on every open
    pub async fn refresh_earnings(&self, force: bool) {
        let started = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            if !force && Self::is_fresh(state.earnings_read_at.as_ref(), *BALANCE_FRESHNESS_MS) {
                return None;
            }
            if state.earnings_task.is_some() && !force {
                return None;
            }
            if let Some(task) = state.earnings_task.take() {
                task.abort();
            }
            state.earnings_generation = state.earnings_generation.wrapping_add(1);
            let loading = state.earnings.is_none();
            if loading {
                state.earnings_phase = RiseSparksLoadPhase::Loading;
            }
            Some((state.earnings_generation, loading))
        });
        let Some((generation, loading)) = started else {
            return;
        };
        if loading {
            self.emit().await;
        }
        let rpc = self.rpc.clone();
        let this = self.this.clone();
        let (task, start) = Self::spawn_gated(async move {
            let result = rpc.earnings().await;
            let Some(this) = this.upgrade() else {
                return;
            };
            match result {
                Ok(response) => this.apply_earnings_read(response.earnings, generation).await,
                Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                    this.finish_earnings_task(generation);
                }
                Err(error) => this.fail_earnings_read(&error, generation).await,
            }
        });
        self.state.with(|state| state.earnings_task = Some(task.abort_handle()));
        let _ = start.send(());
        let _ = task.await;
    }

    async fn apply_earnings_read(&self, server: Option<RiseSparksEarningsDTO>, generation: u64) {
        let applied = self.state.with(|state| {
            if state.is_shutdown || generation != state.earnings_generation {
                return false;
            }
            state.earnings_task = None;
            state.earnings_read_at = Some(Date::now());
            state.earnings = server;
            state.earnings_phase = RiseSparksLoadPhase::Loaded;
            true
        });
        if !applied {
            return;
        }
        self.schedule_persist();
        self.emit().await;
    }

    async fn fail_earnings_read(&self, error: &anyhow::Error, generation: u64) {
        let message = RiseSparksFailure::message(error);
        let failed = self.state.with(|state| {
            if state.is_shutdown || generation != state.earnings_generation {
                return false;
            }
            state.earnings_task = None;
            state.earnings_phase = if state.earnings.is_none() {
                RiseSparksLoadPhase::Failed(message)
            } else {
                RiseSparksLoadPhase::Loaded
            };
            true
        });
        if failed {
            self.emit().await;
        }
    }

    fn finish_earnings_task(&self, generation: u64) {
        self.state.with(|state| {
            if generation == state.earnings_generation {
                state.earnings_task = None;
            }
        });
    }

    pub async fn transfer_earned(&self) -> anyhow::Result<RiseSparksTransferResponseDTO> {
        let began = self.state.with(|state| {
            if state.is_shutdown {
                return Err(anyhow::Error::from(CancellationError));
            }
            if state.is_transferring {
                return Err(RiseSparksTransferError::InFlight.into());
            }
            state.is_transferring = true;
            Ok(())
        });
        began?;
        self.emit().await;
        let response = match self.rpc.transfer_earned().await {
            Ok(response) => response,
            Err(error) => {
                self.state.with(|state| state.is_transferring = false);
                self.emit().await;
                return Err(error);
            }
        };
        let landed = self.state.with(|state| {
            if state.is_shutdown {
                return false;
            }
            state.is_transferring = false;
            if let Some(server) = response.balance.clone() {
                state.balance.replace(server);
                state.has_balance = true;
            }
            if let Some(received) = response.earnings.clone() {
                if let Some(task) = state.earnings_task.take() {
                    task.abort();
                }
                state.earnings_generation = state.earnings_generation.wrapping_add(1);
                state.earnings = Some(received);
                state.earnings_read_at = Some(Date::now());
                state.earnings_phase = RiseSparksLoadPhase::Loaded;
            }
            state.history_read_at.clear();
            true
        });
        if !landed {
            return Err(CancellationError.into());
        }
        self.schedule_persist();
        self.emit().await;
        Ok(response)
    }

    /// balance after a purchase or send, the journal answered so no read needed
    pub async fn apply_spendable_after(&self, spendable: i64, updated_at_ms: i64) {
        let applied = self.state.with(|state| {
            if state.is_shutdown {
                return false;
            }
            let current = state.balance.balance.clone();
            if updated_at_ms < current.updated_at_ms {
                return false;
            }
            state.balance.balance = RiseSparksBalanceDTO {
                available: spendable - current.held,
                earned: current.earned,
                held: current.held,
                spendable,
                spends_blocked: spendable < 0,
                updated_at_ms,
            };
            state.has_balance = true;
            true
        });
        if !applied {
            return;
        }
        self.schedule_persist();
        self.emit().await;
    }

    pub async fn reduce_balance_event(&self, root: BTreeMap<String, RiseJSONValue>) {
        if self.state.with(|state| state.is_shutdown) {
            return;
        }
        let Some(event) = Self::balance_event(root) else {
            return;
        };
        let moved = self.state.with(|state| {
            if !state.balance.apply(&event) {
                return false;
            }
            state.has_balance = true;
            state.history_read_at.clear();
            true
        });
        if !moved {
            return;
        }
        self.schedule_persist();
        self.emit().await;
        if RiseLedgerTransactionKind::from_slug(&event.kind) == Some(RiseLedgerTransactionKind::EarnedTransfer) {
            let this = self.this.clone();
            drop(RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = this.upgrade() {
                    this.refresh_earnings(true).await;
                }
            }));
        }
    }

    pub fn balance_event(root: BTreeMap<String, RiseJSONValue>) -> Option<RiseSparksBalanceEvent> {
        let data = RiseJSONValue::Object(root).data(false).ok()?;
        let Ok(RiseEvent::SparksBalanceChanged(event)) =
            RiseEvent::decode(rise_events::name::SPARKS_BALANCE_CHANGED, &data)
        else {
            return None;
        };
        let transaction_id = event.transaction_id.parse::<i64>().ok()?;
        Some(RiseSparksBalanceEvent {
            transaction_id,
            kind: event.kind,
            spendable: event.balances.spendable,
            earned: event.balances.earned,
            held: event.balances.held,
            spends_blocked: event.balances.spends_blocked,
            updated_at_ms: event.updated_at_ms,
        })
    }

    pub async fn load_history(&self, filter: RiseLedgerHistoryFilter, force: bool) {
        let started = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            if !force && Self::is_fresh(state.history_read_at.get(&filter), *HISTORY_FRESHNESS_MS) {
                return None;
            }
            if state.history_tasks.contains_key(&filter) && !force {
                return None;
            }
            if let Some(task) = state.history_tasks.remove(&filter) {
                task.abort();
            }
            let generation = state.history_generation.get(&filter).copied().unwrap_or(0).wrapping_add(1);
            state.history_generation.insert(filter, generation);
            let loading = !state.history.contains_key(&filter);
            if loading {
                state.history_phase.insert(filter, RiseSparksLoadPhase::Loading);
            }
            Some((generation, loading))
        });
        let Some((generation, loading)) = started else {
            return;
        };
        if loading {
            self.emit().await;
        }
        let rpc = self.rpc.clone();
        let this = self.this.clone();
        let limit = RiseSparksLimits::history_page_size();
        let (task, start) = Self::spawn_gated(async move {
            let result = rpc.history(filter, None, limit).await;
            let Some(this) = this.upgrade() else {
                return;
            };
            match result {
                Ok(page) => this.apply_head(page, filter, generation).await,
                Err(error) if error.downcast_ref::<CancellationError>().is_some() => {
                    this.finish_history_task(filter, generation);
                }
                Err(error) => this.fail_history(&error, filter, generation).await,
            }
        });
        self.state.with(|state| {
            state.history_tasks.insert(filter, task.abort_handle());
        });
        let _ = start.send(());
        let _ = task.await;
    }

    pub async fn load_more_history(&self, filter: RiseLedgerHistoryFilter) {
        let started = self.state.with(|state| {
            if state.is_shutdown || state.loading_more.contains(&filter) || state.history_tasks.contains_key(&filter) {
                return None;
            }
            let page = state.history.get(&filter)?;
            if !page.has_more {
                return None;
            }
            let cursor = page.items.last()?.id.clone();
            state.loading_more.insert(filter);
            Some((cursor, state.history_generation.get(&filter).copied().unwrap_or(0)))
        });
        let Some((cursor, generation)) = started else {
            return;
        };
        self.emit().await;
        match self.rpc.history(filter, Some(&cursor), RiseSparksLimits::history_page_size()).await {
            Ok(next) => self.apply_tail(next, filter, &cursor, generation).await,
            Err(_) => {
                self.state.with(|state| state.loading_more.remove(&filter));
                self.emit().await;
            }
        }
    }

    async fn apply_head(&self, response: RiseSparksHistoryResponseDTO, filter: RiseLedgerHistoryFilter, generation: u64) {
        let applied = self.state.with(|state| {
            if state.is_shutdown || state.history_generation.get(&filter) != Some(&generation) {
                return false;
            }
            state.history_tasks.remove(&filter);
            state.history_read_at.insert(filter, Date::now());
            let items = Self::deduplicated(response.items);
            let merged = Self::merged_head(items, response.has_more, state.history.get(&filter));
            state.history.insert(filter, merged);
            state.history_phase.insert(filter, RiseSparksLoadPhase::Loaded);
            true
        });
        if !applied {
            return;
        }
        self.schedule_persist();
        self.emit().await;
    }

    async fn apply_tail(
        &self,
        response: RiseSparksHistoryResponseDTO,
        filter: RiseLedgerHistoryFilter,
        cursor: &str,
        generation: u64,
    ) {
        self.state.with(|state| {
            state.loading_more.remove(&filter);
            if state.is_shutdown || state.history_generation.get(&filter) != Some(&generation) {
                return;
            }
            let Some(page) = state.history.get_mut(&filter) else {
                return;
            };
            if page.items.last().map(|item| item.id.as_str()) != Some(cursor) {
                return;
            }
            let known: HashSet<String> = page.items.iter().map(|item| item.id.clone()).collect();
            let fresh: Vec<RiseSparksHistoryItemDTO> =
                response.items.into_iter().filter(|item| !known.contains(&item.id)).collect();
            page.has_more = response.has_more && !fresh.is_empty();
            page.items.extend(fresh);
        });
        self.emit().await;
    }

    async fn fail_history(&self, error: &anyhow::Error, filter: RiseLedgerHistoryFilter, generation: u64) {
        let message = RiseSparksFailure::message(error);
        let failed = self.state.with(|state| {
            if state.is_shutdown || state.history_generation.get(&filter) != Some(&generation) {
                return false;
            }
            state.history_tasks.remove(&filter);
            let phase = if state.history.contains_key(&filter) {
                RiseSparksLoadPhase::Loaded
            } else {
                RiseSparksLoadPhase::Failed(message)
            };
            state.history_phase.insert(filter, phase);
            true
        });
        if failed {
            self.emit().await;
        }
    }

    fn finish_history_task(&self, filter: RiseLedgerHistoryFilter, generation: u64) {
        self.state.with(|state| {
            if state.history_generation.get(&filter) == Some(&generation) {
                state.history_tasks.remove(&filter);
            }
        });
    }

    fn merged_head(
        head: Vec<RiseSparksHistoryItemDTO>,
        has_more: bool,
        loaded: Option<&RiseSparksHistoryPage>,
    ) -> RiseSparksHistoryPage {
        let anchored = loaded.filter(|loaded| has_more && loaded.items.len() > head.len()).and_then(|loaded| {
            let anchor = head.last()?.id.as_str();
            let index = loaded.items.iter().position(|item| item.id == anchor)?;
            Some((loaded, index))
        });
        let Some((loaded, index)) = anchored else {
            return RiseSparksHistoryPage { items: head, has_more };
        };
        let known: HashSet<&str> = head.iter().map(|item| item.id.as_str()).collect();
        let tail: Vec<RiseSparksHistoryItemDTO> =
            loaded.items[index + 1..].iter().filter(|item| !known.contains(item.id.as_str())).cloned().collect();
        let mut items = head;
        items.extend(tail);
        RiseSparksHistoryPage { items, has_more: loaded.has_more }
    }

    fn deduplicated(items: Vec<RiseSparksHistoryItemDTO>) -> Vec<RiseSparksHistoryItemDTO> {
        let mut seen = HashSet::new();
        items.into_iter().filter(|item| seen.insert(item.id.clone())).collect()
    }

    pub async fn invalidate_history(&self) {
        self.state.with(|state| state.history_read_at.clear());
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| {
            state.is_shutdown = true;
            if let Some(task) = state.balance_task.take() {
                task.abort();
            }
            if let Some(task) = state.earnings_task.take() {
                task.abort();
            }
            for (_, task) in state.history_tasks.drain() {
                task.abort();
            }
            if let Some(task) = state.persist_task.take() {
                task.abort();
            }
            state.publish = None;
        });
        self.resources.shutdown().await;
    }

    fn schedule_persist(&self) {
        let resources = self.resources.clone();
        self.state.with(|state| {
            if let Some(task) = state.persist_task.take() {
                task.abort();
            }
            let balance = state.balance.clone();
            let history = state.history.clone();
            let earnings = state.earnings.clone();
            let task = RiseAccountRuntime::current().spawn(async move {
                tokio::time::sleep(PERSIST_DELAY).await;
                let _ = resources.store(balance, history, earnings).await;
            });
            state.persist_task = Some(task.abort_handle());
        });
    }

    fn snapshot(&self, state: &RiseSparksRepositoryState) -> RiseSparksRepositorySnapshot {
        RiseSparksRepositorySnapshot {
            account_id: self.account_id.clone(),
            account_epoch: self.account_epoch.clone(),
            session_id: self.session_id,
            balance: state.balance.balance.clone(),
            has_balance: state.has_balance,
            balance_phase: state.balance_phase.clone(),
            earnings: state.earnings.clone(),
            earnings_phase: state.earnings_phase.clone(),
            is_transferring: state.is_transferring,
            history: state.history.clone(),
            history_phase: state.history_phase.clone(),
            loading_more: state.loading_more.clone(),
        }
    }

    async fn emit(&self) {
        let pending = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            let publish = state.publish.clone()?;
            Some((publish, self.snapshot(state)))
        });
        if let Some((publish, snapshot)) = pending {
            publish(snapshot).await;
        }
    }

    fn is_fresh(read_at: Option<&Date>, freshness_ms: i64) -> bool {
        read_at.is_some_and(|read_at| Date::now().time_interval_since(read_at) * 1_000.0 < freshness_ms as f64)
    }

    /// a gate holds the work until the handle is kept, so a fast answer never clears a slot filled after it
    fn spawn_gated(work: impl Future<Output = ()> + Send + 'static) -> (JoinHandle<()>, oneshot::Sender<()>) {
        let (start, started) = oneshot::channel::<()>();
        let task = RiseAccountRuntime::current().spawn(async move {
            if started.await.is_ok() {
                work.await;
            }
        });
        (task, start)
    }
}

pub enum RiseSparksFailure {}

impl RiseSparksFailure {
    /// canon text for a refused request, the connection phrase otherwise
    pub fn message(error: &anyhow::Error) -> String {
        if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            let failure = remote.canon_failure();
            if let Some(text) = CanonErrors::text(Some(&remote.code), &failure.details) {
                return text;
            }
            return if remote.message.is_empty() { tr("retry_later_error") } else { remote.message.clone() };
        }
        tr("retry_later_error")
    }

    pub fn code(error: &anyhow::Error) -> Option<String> {
        error.downcast_ref::<RiseWireRemoteError>().map(|remote| remote.code.clone())
    }

    pub fn integer_detail(key: &str, error: &anyhow::Error) -> Option<i64> {
        let details = error.downcast_ref::<RiseWireRemoteError>()?.details.as_ref()?.object_value()?;
        match details.get(key)? {
            RiseJSONValue::Integer(number) => Some(*number),
            RiseJSONValue::UnsignedInteger(number) => i64::try_from(*number).ok(),
            RiseJSONValue::String(text) => text.parse::<i64>().ok(),
            RiseJSONValue::Number(number) => {
                let rounded = number.round();
                (rounded >= i64::MIN as f64 && rounded < i64::MAX as f64).then_some(rounded as i64)
            }
            _ => None,
        }
    }
}
