use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use rise_canon::RiseTimingCanon;
use rise_canon::rise_response::post as post_response;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue,
    RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine};
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_timing::{RiseTimingBatchExt, RiseTimingCanonExt};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::post::engine::rise_post_debug_trace::RisePostDebugTrace;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostCacheSource, RisePostPresentationSnapshot, RisePostStatsPresentationState, RisePostViewRecordResult,
    RisePostViewSurface, RisePostVisibilityIntent,
};
use crate::modules::post::engine::rise_post_rpc::{RisePostEndpoints, RisePostRPC};
use crate::modules::post::stores::post_actions::post_types::RiseStoredPostStats;
use crate::modules::post::stores::post_actions::post_wire_types::{
    PostStatsGatewayDTO, PostStatsGatewayDTOExt, RecordPostLinkOpenGatewayDTO, RecordPostShareGatewayDTO,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RisePostRepositoryError {
    #[error("stale account")]
    StaleAccount,
    #[error("missing active session")]
    MissingActiveSession,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RisePostMutationError {
    #[error("{0}")]
    InvalidDraft(String),
    #[error("{0}")]
    Server(String),
}

impl RisePostMutationError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            Self::InvalidDraft(message) | Self::Server(message) => Some(message.clone()),
        }
    }
}

#[derive(Clone)]
pub struct RisePostRepositoryStatsRefreshTicket {
    pub generation: Option<u64>,
    pub snapshot: RisePostPresentationSnapshot,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RisePostRepositoryPersistentStatsEntry {
    value: RiseStoredPostStats,
    updated_at_milliseconds: i64,
    access: u64,
    cost: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RisePostRepositoryPersistentViewEntry {
    count: i64,
    access: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RisePostRepositoryPersistentViewAlias {
    canonical_post_id: Option<i64>,
    access: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RisePostRepositoryPersistentCache {
    stats_by_post_id: HashMap<String, RisePostRepositoryPersistentStatsEntry>,
    view_counts_by_post_id: HashMap<String, RisePostRepositoryPersistentViewEntry>,
    view_aliases_by_post_id: Option<HashMap<String, RisePostRepositoryPersistentViewAlias>>,
}

#[derive(Clone)]
struct RisePostRepositoryStatsRuntimeState {
    is_loading: bool,
    error_message: Option<String>,
    source: RisePostCacheSource,
}

impl Default for RisePostRepositoryStatsRuntimeState {
    fn default() -> Self {
        Self {
            is_loading: false,
            error_message: None,
            source: RisePostCacheSource::Miss,
        }
    }
}

/// a view waiting to be sent
#[derive(Clone, Copy)]
struct RisePostRepositoryPendingView {
    post_id: i64,
    surface: RisePostViewSurface,
    access: u64,
}

#[derive(Clone, Copy)]
struct RisePostRepositoryScopeGeneration {
    generation: u64,
    access: u64,
}

/// the running view flush
/// awaiting done doesnt cancel it, abort does
struct RisePostRepositoryViewFlush {
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, Option<RisePostPresentationSnapshot>>>,
}

/// the actors mutable state
/// one with closure is one sync stretch of an actor method
struct RisePostRepositoryState {
    stats_by_post_id: HashMap<i64, RisePostRepositoryPersistentStatsEntry>,
    stats_runtime_by_post_id: HashMap<i64, RisePostRepositoryStatsRuntimeState>,
    stats_memory_bytes: i64,
    view_counts_by_post_id: HashMap<i64, RisePostRepositoryPersistentViewEntry>,
    view_aliases_by_post_id: HashMap<i64, RisePostRepositoryPersistentViewAlias>,
    pending_views: HashMap<i64, RisePostRepositoryPendingView>,
    in_flight_view_ids: HashSet<i64>,
    history_refreshed_at: HashMap<i64, Duration>,
    scope_generations: HashMap<Uuid, RisePostRepositoryScopeGeneration>,
    stats_request_generations: HashMap<i64, u64>,
    in_flight_stats: HashSet<i64>,
    view_flush_task: Option<RisePostRepositoryViewFlush>,
    access_clock: u64,
    presentation_revision: i64,
    store_version: i64,
    session_generation: u64,
    is_activated: bool,
    is_shutdown: bool,
}

/// what one isolated stretch of record_view decided
enum RisePostRepositoryViewStep {
    Answer(RisePostViewRecordResult),
    Await(Option<Shared<BoxFuture<'static, Option<RisePostPresentationSnapshot>>>>),
}

struct RisePostRepositoryStatsCancelGuard<'a> {
    repository: &'a RisePostRepository,
    post_id: i64,
    generation: u64,
    armed: bool,
}

impl Drop for RisePostRepositoryStatsCancelGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.repository.fail_stats_refresh(self.post_id, self.generation, None);
        }
    }
}

pub struct RisePostRepository {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,

    engine: Arc<RiseEngine>,
    rpc: RisePostRPC,
    maximum_stats_entries: usize,
    maximum_stats_bytes: i64,
    maximum_view_entries: usize,
    maximum_view_alias_entries: usize,
    maximum_scope_entries: usize,
    maximum_pending_views: usize,
    maximum_view_batch: usize,
    stats_refresh_interval_milliseconds: i64,
    history_refresh_interval: Duration,
    maximum_history_refreshes: usize,
    this: Weak<RisePostRepository>,
    state: RiseIsolated<RisePostRepositoryState>,
}

impl RisePostRepository {
    const NAMESPACE: &'static str = "post.resources.v1";
    const CACHE_KEY: &'static str = "bounded-stats-and-views.v1";
    const VIEW_RECEIPTS_BATCH: &'static str = "post_view_receipts";
    const STATS_FRESHNESS: &'static str = "post_stats";
    const VIEW_FLUSH_DEBOUNCE: Duration = Duration::from_millis(180);

    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Arc<Self> {
        let rpc = RisePostRPC::new(engine.clone());
        let batch = RiseTimingCanon::batch(Self::VIEW_RECEIPTS_BATCH);
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id,
            engine,
            rpc,
            maximum_stats_entries: 64,
            maximum_stats_bytes: 2 * 1_024 * 1_024,
            maximum_view_entries: 512,
            maximum_view_alias_entries: 128,
            maximum_scope_entries: 256,
            maximum_pending_views: usize::try_from(batch.queue_cap_items()).unwrap_or(0),
            maximum_view_batch: usize::try_from(batch.client_batch_items).unwrap_or(usize::MAX),
            stats_refresh_interval_milliseconds: RiseTimingCanon::freshness_ms(Self::STATS_FRESHNESS),
            history_refresh_interval: Duration::from_secs(30),
            maximum_history_refreshes: 128,
            this: this.clone(),
            state: RiseIsolated::new(RisePostRepositoryState {
                stats_by_post_id: HashMap::new(),
                stats_runtime_by_post_id: HashMap::new(),
                stats_memory_bytes: 0,
                view_counts_by_post_id: HashMap::new(),
                view_aliases_by_post_id: HashMap::new(),
                pending_views: HashMap::new(),
                in_flight_view_ids: HashSet::new(),
                history_refreshed_at: HashMap::new(),
                scope_generations: HashMap::new(),
                stats_request_generations: HashMap::new(),
                in_flight_stats: HashSet::new(),
                view_flush_task: None,
                access_clock: 0,
                presentation_revision: 0,
                store_version: 0,
                session_generation: 1,
                is_activated: false,
                is_shutdown: false,
            }),
        })
    }

    fn namespace() -> RiseStoreNamespace {
        RiseStoreNamespace::new(Self::NAMESPACE)
    }

    pub async fn activate(&self) -> RisePostPresentationSnapshot {
        enum Step {
            Done(RisePostPresentationSnapshot),
            Load,
        }
        let step = self.state.with(|s| {
            if s.is_shutdown {
                return Step::Done(self.snapshot(s));
            }
            if s.is_activated {
                RisePostDebugTrace::cache(None, RisePostCacheSource::Memory, s.presentation_revision);
                return Step::Done(self.snapshot(s));
            }
            s.is_activated = true;
            Step::Load
        });
        if let Step::Done(snapshot) = step {
            return snapshot;
        }

        let stored = self.engine.value(Self::namespace(), Self::CACHE_KEY.to_owned()).await.ok().flatten();
        let decoded = match stored {
            Some(stored) => Self::decode(stored.data.clone()).await.ok().map(|payload| (stored, payload)),
            None => None,
        };
        let Some((stored, payload)) = decoded else {
            return self.state.with(|s| {
                RisePostDebugTrace::cache(None, RisePostCacheSource::Miss, s.presentation_revision);
                self.snapshot(s)
            });
        };

        self.state.with(|s| {
            s.store_version = stored.version;
            s.presentation_revision = s.presentation_revision.max(stored.version);
            for (raw_id, entry) in payload.stats_by_post_id {
                let Some(post_id) = raw_id.parse::<i64>().ok().filter(|id| *id > 0) else {
                    continue;
                };
                s.stats_memory_bytes += entry.cost.max(0);
                s.access_clock = s.access_clock.max(entry.access);
                s.stats_by_post_id.insert(post_id, entry);
                s.stats_runtime_by_post_id.insert(
                    post_id,
                    RisePostRepositoryStatsRuntimeState {
                        is_loading: false,
                        error_message: None,
                        source: RisePostCacheSource::PersistentStore,
                    },
                );
            }
            for (raw_id, entry) in payload.view_counts_by_post_id {
                let Some(post_id) = raw_id.parse::<i64>().ok().filter(|id| *id > 0) else {
                    continue;
                };
                s.access_clock = s.access_clock.max(entry.access);
                s.view_counts_by_post_id.insert(post_id, entry);
            }
            for (raw_id, entry) in payload.view_aliases_by_post_id.unwrap_or_default() {
                let Some(post_id) = raw_id.parse::<i64>().ok().filter(|id| *id > 0) else {
                    continue;
                };
                s.access_clock = s.access_clock.max(entry.access);
                s.view_aliases_by_post_id.insert(post_id, entry);
            }
            self.trim_stats(s, None);
            self.trim_view_counts(s, None);
            self.trim_view_aliases(s, None);
            Self::bump_presentation_revision(s);
            RisePostDebugTrace::cache(None, RisePostCacheSource::PersistentStore, s.presentation_revision);
            self.snapshot(s)
        })
    }

    pub async fn begin_stats_refresh(&self, post_id: i64, force: bool) -> RisePostRepositoryStatsRefreshTicket {
        self.state.with(|s| {
            if s.is_shutdown || !s.is_activated {
                return RisePostRepositoryStatsRefreshTicket { generation: None, snapshot: self.snapshot(s) };
            }
            Self::touch_stats(s, post_id);
            let now = Self::now_milliseconds();
            if !force
                && let Some(cached) = s.stats_by_post_id.get(&post_id)
                && now - cached.updated_at_milliseconds < self.stats_refresh_interval_milliseconds
            {
                RisePostDebugTrace::cache(Some(post_id), RisePostCacheSource::Memory, s.presentation_revision);
                return RisePostRepositoryStatsRefreshTicket { generation: None, snapshot: self.snapshot(s) };
            }
            if s.in_flight_stats.contains(&post_id) {
                return RisePostRepositoryStatsRefreshTicket { generation: None, snapshot: self.snapshot(s) };
            }

            let slot = s.stats_request_generations.entry(post_id).or_insert(0);
            *slot = slot.wrapping_add(1);
            let generation = *slot;
            s.in_flight_stats.insert(post_id);
            let has_stats = s.stats_by_post_id.contains_key(&post_id);
            let runtime = s.stats_runtime_by_post_id.entry(post_id).or_default();
            runtime.is_loading = true;
            runtime.error_message = None;
            if has_stats {
                runtime.source = RisePostCacheSource::Memory;
            }
            Self::bump_presentation_revision(s);
            RisePostRepositoryStatsRefreshTicket { generation: Some(generation), snapshot: self.snapshot(s) }
        })
    }

    pub async fn finish_stats_refresh(&self, post_id: i64, days: i64, generation: u64) -> RisePostPresentationSnapshot {
        let expected_session_generation = self.state.with(|s| {
            Self::accepts_stats_request(s, post_id, generation).then_some(s.session_generation)
        });
        let Some(expected_session_generation) = expected_session_generation else {
            return self.state.with(|s| self.snapshot(s));
        };
        let mut guard = RisePostRepositoryStatsCancelGuard { repository: self, post_id, generation, armed: true };

        let result: anyhow::Result<PostStatsGatewayDTO> = self
            .rpc
            .call(
                RisePostEndpoints::get_post_stats(),
                &rise_canon::rise_request::post::GetPostStats { days: Some(days.clamp(1, 90)), post_id: Some(post_id) },
            )
            .await;
        let response = result.and_then(|response| {
            let accepted = self.state.with(|s| {
                expected_session_generation == s.session_generation
                    && Self::accepts_stats_request(s, post_id, generation)
            });
            if accepted && self.is_current_account() {
                Ok(response)
            } else {
                Err(CancellationError.into())
            }
        });
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                guard.armed = false;
                let message = if error.downcast_ref::<CancellationError>().is_some() {
                    None
                } else {
                    Some(CanonErrors::load_failure_text(&error))
                };
                return self.fail_stats_refresh(post_id, generation, message);
            }
        };

        let value = response.as_domain();
        let cost = Self::encoded_cost(value.clone()).await;
        guard.armed = false;
        self.state.with(|s| {
            self.install_stats(s, value, post_id, cost, RisePostCacheSource::Network);
            s.in_flight_stats.remove(&post_id);
            Self::bump_presentation_revision(s);
        });
        self.persist().await;
        self.state.with(|s| self.snapshot(s))
    }

    /// failure branch of finishing a stats refresh
    /// error_message is none for a cancellation
    fn fail_stats_refresh(&self, post_id: i64, generation: u64, error_message: Option<String>) -> RisePostPresentationSnapshot {
        self.state.with(|s| {
            if s.stats_request_generations.get(&post_id) != Some(&generation) {
                return self.snapshot(s);
            }
            s.in_flight_stats.remove(&post_id);
            let source = if s.stats_by_post_id.contains_key(&post_id) {
                RisePostCacheSource::Memory
            } else {
                RisePostCacheSource::Miss
            };
            let runtime = s.stats_runtime_by_post_id.entry(post_id).or_default();
            runtime.is_loading = false;
            runtime.error_message = error_message;
            runtime.source = source;
            Self::bump_presentation_revision(s);
            self.snapshot(s)
        })
    }

    /// a receipt not a counter edit
    /// reply carries the live counter and the caller patches its row
    /// nothing is retried since the server dedupes by receipt
    pub async fn record_share(&self, post_id: i64, channel: &str) -> anyhow::Result<RecordPostShareGatewayDTO> {
        if self.state.with(|s| s.is_shutdown) || post_id <= 0 || !self.is_current_account() {
            return Err(CancellationError.into());
        }
        self.rpc
            .call(
                RisePostEndpoints::record_post_share(),
                &rise_canon::rise_request::post::RecordPostShare { channel: Some(channel.to_owned()), post_id: Some(post_id) },
            )
            .await
    }

    pub async fn record_link_open(&self, post_id: i64) -> anyhow::Result<RecordPostLinkOpenGatewayDTO> {
        if self.state.with(|s| s.is_shutdown) || post_id <= 0 || !self.is_current_account() {
            return Err(CancellationError.into());
        }
        self.rpc
            .call(
                RisePostEndpoints::record_post_link_open(),
                &rise_canon::rise_request::post::RecordPostLinkOpen { post_id: Some(post_id) },
            )
            .await
    }

    pub async fn record_view(&self, intent: &RisePostVisibilityIntent) -> RisePostViewRecordResult {
        let refused = RisePostViewRecordResult { handled: false, snapshot: None };
        let accepted = self.state.with(|s| !s.is_shutdown && s.is_activated)
            && intent.account_id == self.account_id
            && intent.account_epoch == self.account_epoch
            && intent.post_id > 0
            && self.is_current_account();
        if !accepted {
            return refused;
        }

        let step = self.state.with(|s| {
            if s.is_shutdown || !s.is_activated {
                return RisePostRepositoryViewStep::Answer(RisePostViewRecordResult { handled: false, snapshot: None });
            }
            s.access_clock = s.access_clock.wrapping_add(1);
            if let Some(scope) = s.scope_generations.get(&intent.scope_id)
                && scope.generation > intent.scope_generation
            {
                return RisePostRepositoryViewStep::Answer(RisePostViewRecordResult { handled: true, snapshot: None });
            }
            s.scope_generations.insert(
                intent.scope_id,
                RisePostRepositoryScopeGeneration { generation: intent.scope_generation, access: s.access_clock },
            );
            self.trim_scope_generations(s);

            let refreshes_history = Self::refreshes_view_history(intent.surface)
                && (intent.surface == RisePostViewSurface::PostDetail
                    || self.claim_history_refresh(s, intent.post_id));
            if !refreshes_history && Self::is_view_handled(s, intent.post_id) {
                return RisePostRepositoryViewStep::Answer(RisePostViewRecordResult { handled: true, snapshot: None });
            }
            if refreshes_history
                && let Some(pending) = s.pending_views.get(&intent.post_id).copied()
                && !Self::refreshes_view_history(pending.surface)
            {
                s.pending_views.insert(
                    intent.post_id,
                    RisePostRepositoryPendingView { post_id: intent.post_id, surface: intent.surface, access: pending.access },
                );
            }
            if s.pending_views.contains_key(&intent.post_id)
                || (!refreshes_history && s.in_flight_view_ids.contains(&intent.post_id))
            {
                return RisePostRepositoryViewStep::Await(s.view_flush_task.as_ref().map(|task| task.done.clone()));
            }
            if s.pending_views.len() >= self.maximum_pending_views
                && let Some(oldest) =
                    s.pending_views.iter().min_by_key(|(_, pending)| pending.access).map(|(post_id, _)| *post_id)
            {
                s.pending_views.remove(&oldest);
            }
            s.pending_views.insert(
                intent.post_id,
                RisePostRepositoryPendingView { post_id: intent.post_id, surface: intent.surface, access: s.access_clock },
            );

            if s.view_flush_task.is_none() {
                let expected_session_generation = s.session_generation;
                let should_debounce = s.pending_views.len() < self.maximum_view_batch;
                let weak = self.this.clone();
                let handle = RiseAccountRuntime::current().spawn(async move {
                    if should_debounce {
                        tokio::time::sleep(Self::VIEW_FLUSH_DEBOUNCE).await;
                    }
                    let this = weak.upgrade()?;
                    this.flush_views(expected_session_generation).await
                });
                let abort = handle.abort_handle();
                let done: Shared<BoxFuture<'static, Option<RisePostPresentationSnapshot>>> =
                    async move { handle.await.ok().flatten() }.boxed().shared();
                s.view_flush_task = Some(RisePostRepositoryViewFlush { abort, done });
            }
            RisePostRepositoryViewStep::Await(s.view_flush_task.as_ref().map(|task| task.done.clone()))
        });

        match step {
            RisePostRepositoryViewStep::Answer(result) => result,
            RisePostRepositoryViewStep::Await(task) => {
                let snapshot = match task {
                    Some(task) => task.await,
                    None => None,
                };
                let handled = self.state.with(|s| Self::is_view_handled(s, intent.post_id));
                RisePostViewRecordResult { handled, snapshot }
            }
        }
    }

    pub async fn shutdown(&self) {
        let task = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.is_shutdown = true;
            s.session_generation = s.session_generation.wrapping_add(1);
            for generation in s.stats_request_generations.values_mut() {
                *generation = generation.wrapping_add(1);
            }
            s.in_flight_stats.clear();
            s.pending_views.clear();
            s.in_flight_view_ids.clear();
            s.history_refreshed_at.clear();
            s.scope_generations.clear();
            let task = s.view_flush_task.take();
            if let Some(task) = &task {
                task.abort.abort();
            }
            Some(task)
        });
        let Some(task) = task else {
            return;
        };
        if let Some(task) = task {
            let _ = task.done.await;
        }
        self.state.with(|s| {
            s.stats_by_post_id = HashMap::new();
            s.stats_runtime_by_post_id = HashMap::new();
            s.view_counts_by_post_id = HashMap::new();
            s.view_aliases_by_post_id = HashMap::new();
            s.stats_memory_bytes = 0;
        });
    }

    async fn flush_views(&self, expected_session_generation: u64) -> Option<RisePostPresentationSnapshot> {
        let mut changed = false;
        loop {
            let planned = self.state.with(|s| {
                let first = s.pending_views.values().min_by_key(|pending| pending.access).copied();
                let Some(first) =
                    first.filter(|_| !s.is_shutdown && s.session_generation == expected_session_generation)
                else {
                    s.view_flush_task = None;
                    if changed {
                        Self::bump_presentation_revision(s);
                    }
                    return None;
                };
                let mut entries: Vec<RisePostRepositoryPendingView> =
                    s.pending_views.values().filter(|pending| pending.surface == first.surface).copied().collect();
                entries.sort_by_key(|pending| pending.access);
                entries.truncate(self.maximum_view_batch);
                for entry in &entries {
                    s.pending_views.remove(&entry.post_id);
                    s.in_flight_view_ids.insert(entry.post_id);
                }
                Some((first, entries))
            });
            let Some((first, entries)) = planned else {
                break;
            };
            let ids: Vec<i64> = entries.iter().map(|entry| entry.post_id).collect();
            if ids.is_empty() {
                continue;
            }

            let response: anyhow::Result<post_response::RecordPostViews> = self
                .rpc
                .call(
                    RisePostEndpoints::record_post_views(),
                    &rise_canon::rise_request::post::RecordPostViews {
                        post_ids: Some(ids),
                        surface: Some(first.surface.wire().to_owned()),
                    },
                )
                .await;
            let response = response.ok().filter(|_| {
                self.state.with(|s| !s.is_shutdown && s.session_generation == expected_session_generation)
                    && self.is_current_account()
            });
            self.state.with(|s| {
                if let Some(response) = response {
                    let results: Vec<_> = response.results.iter().filter(|result| result.post_id > 0).collect();
                    for result in &results {
                        self.install_view_count(s, result.views_count.max(0), result.post_id);
                        changed = true;
                    }
                    for (index, entry) in entries.iter().enumerate() {
                        let exact = results.iter().find(|result| result.post_id == entry.post_id);
                        let positional = (results.len() == entries.len()).then(|| results[index]);
                        let canonical_post_id = exact.map(|result| result.post_id).or(positional.map(|result| result.post_id));
                        self.install_view_alias(s, entry.post_id, canonical_post_id);
                        changed = true;
                    }
                }
                for entry in &entries {
                    s.in_flight_view_ids.remove(&entry.post_id);
                }
            });
        }
        if !changed {
            return None;
        }
        self.persist().await;
        Some(self.state.with(|s| self.snapshot(s)))
    }

    fn install_stats(
        &self,
        s: &mut RisePostRepositoryState,
        value: RiseStoredPostStats,
        post_id: i64,
        cost: i64,
        source: RisePostCacheSource,
    ) {
        if let Some(previous) = s.stats_by_post_id.get(&post_id) {
            s.stats_memory_bytes = (s.stats_memory_bytes - previous.cost).max(0);
        }
        s.access_clock = s.access_clock.wrapping_add(1);
        s.stats_by_post_id.insert(
            post_id,
            RisePostRepositoryPersistentStatsEntry {
                value,
                updated_at_milliseconds: Self::now_milliseconds(),
                access: s.access_clock,
                cost: cost.max(0),
            },
        );
        s.stats_memory_bytes += cost.max(0);
        s.stats_runtime_by_post_id.insert(
            post_id,
            RisePostRepositoryStatsRuntimeState { is_loading: false, error_message: None, source },
        );
        self.trim_stats(s, Some(post_id));
    }

    fn install_view_count(&self, s: &mut RisePostRepositoryState, count: i64, post_id: i64) {
        s.access_clock = s.access_clock.wrapping_add(1);
        s.view_aliases_by_post_id.remove(&post_id);
        s.view_counts_by_post_id
            .insert(post_id, RisePostRepositoryPersistentViewEntry { count, access: s.access_clock });
        if let Some(stats) = s.stats_by_post_id.get_mut(&post_id) {
            stats.value.views_total = count;
            stats.updated_at_milliseconds = Self::now_milliseconds();
            stats.access = s.access_clock;
            s.stats_runtime_by_post_id.insert(
                post_id,
                RisePostRepositoryStatsRuntimeState {
                    is_loading: false,
                    error_message: None,
                    source: RisePostCacheSource::Network,
                },
            );
        }
        self.trim_view_counts(s, Some(post_id));
    }

    /// an opened post moves to the top of view history on every open
    /// so its sent again even if the feed or history list already counted it
    fn refreshes_view_history(surface: RisePostViewSurface) -> bool {
        surface == RisePostViewSurface::PostDetail || surface == RisePostViewSurface::ShortViewer
    }

    fn claim_history_refresh(&self, s: &mut RisePostRepositoryState, post_id: i64) -> bool {
        let now = RiseClock::monotonic();
        if let Some(last) = s.history_refreshed_at.get(&post_id)
            && now.saturating_sub(*last) < self.history_refresh_interval
        {
            return false;
        }
        s.history_refreshed_at.insert(post_id, now);
        if s.history_refreshed_at.len() > self.maximum_history_refreshes
            && let Some(oldest) =
                s.history_refreshed_at.iter().min_by_key(|(_, refreshed_at)| **refreshed_at).map(|(id, _)| *id)
        {
            s.history_refreshed_at.remove(&oldest);
        }
        true
    }

    fn is_view_handled(s: &RisePostRepositoryState, post_id: i64) -> bool {
        s.view_counts_by_post_id.contains_key(&post_id) || s.view_aliases_by_post_id.contains_key(&post_id)
    }

    fn install_view_alias(&self, s: &mut RisePostRepositoryState, requested_post_id: i64, canonical_post_id: Option<i64>) {
        if s.view_counts_by_post_id.contains_key(&requested_post_id) {
            return;
        }
        s.access_clock = s.access_clock.wrapping_add(1);
        s.view_aliases_by_post_id.insert(
            requested_post_id,
            RisePostRepositoryPersistentViewAlias { canonical_post_id, access: s.access_clock },
        );
        self.trim_view_aliases(s, Some(requested_post_id));
    }

    fn touch_stats(s: &mut RisePostRepositoryState, post_id: i64) {
        if !s.stats_by_post_id.contains_key(&post_id) {
            return;
        }
        s.access_clock = s.access_clock.wrapping_add(1);
        let access = s.access_clock;
        if let Some(entry) = s.stats_by_post_id.get_mut(&post_id) {
            entry.access = access;
        }
    }

    fn trim_stats(&self, s: &mut RisePostRepositoryState, protected_post_id: Option<i64>) {
        while s.stats_by_post_id.len() > self.maximum_stats_entries || s.stats_memory_bytes > self.maximum_stats_bytes {
            let Some((victim, cost)) = s
                .stats_by_post_id
                .iter()
                .filter(|(post_id, _)| Some(**post_id) != protected_post_id)
                .min_by_key(|(_, entry)| entry.access)
                .map(|(post_id, entry)| (*post_id, entry.cost))
            else {
                break;
            };
            s.stats_by_post_id.remove(&victim);
            s.stats_runtime_by_post_id.remove(&victim);
            s.stats_memory_bytes = (s.stats_memory_bytes - cost.max(0)).max(0);
        }
    }

    fn trim_view_counts(&self, s: &mut RisePostRepositoryState, protected_post_id: Option<i64>) {
        while s.view_counts_by_post_id.len() > self.maximum_view_entries {
            let Some(victim) = s
                .view_counts_by_post_id
                .iter()
                .filter(|(post_id, _)| Some(**post_id) != protected_post_id)
                .min_by_key(|(_, entry)| entry.access)
                .map(|(post_id, _)| *post_id)
            else {
                break;
            };
            s.view_counts_by_post_id.remove(&victim);
        }
    }

    fn trim_view_aliases(&self, s: &mut RisePostRepositoryState, protected_post_id: Option<i64>) {
        while s.view_aliases_by_post_id.len() > self.maximum_view_alias_entries {
            let Some(victim) = s
                .view_aliases_by_post_id
                .iter()
                .filter(|(post_id, _)| Some(**post_id) != protected_post_id)
                .min_by_key(|(_, entry)| entry.access)
                .map(|(post_id, _)| *post_id)
            else {
                break;
            };
            s.view_aliases_by_post_id.remove(&victim);
        }
    }

    fn trim_scope_generations(&self, s: &mut RisePostRepositoryState) {
        while s.scope_generations.len() > self.maximum_scope_entries {
            let Some(victim) =
                s.scope_generations.iter().min_by_key(|(_, scope)| scope.access).map(|(scope_id, _)| *scope_id)
            else {
                break;
            };
            s.scope_generations.remove(&victim);
        }
    }

    fn accepts_stats_request(s: &RisePostRepositoryState, post_id: i64, generation: u64) -> bool {
        !s.is_shutdown
            && s.is_activated
            && s.in_flight_stats.contains(&post_id)
            && s.stats_request_generations.get(&post_id) == Some(&generation)
    }

    fn snapshot(&self, s: &RisePostRepositoryState) -> RisePostPresentationSnapshot {
        let mut stats: HashMap<String, RisePostStatsPresentationState> = s
            .stats_by_post_id
            .iter()
            .map(|(post_id, entry)| {
                let runtime = s.stats_runtime_by_post_id.get(post_id).cloned().unwrap_or(
                    RisePostRepositoryStatsRuntimeState {
                        source: RisePostCacheSource::Memory,
                        ..RisePostRepositoryStatsRuntimeState::default()
                    },
                );
                (
                    post_id.to_string(),
                    RisePostStatsPresentationState {
                        value: Some(entry.value.clone()),
                        is_loading: runtime.is_loading,
                        error_message: runtime.error_message,
                        source: runtime.source,
                        updated_at_milliseconds: entry.updated_at_milliseconds,
                    },
                )
            })
            .collect();
        let view_counts: HashMap<String, i64> =
            s.view_counts_by_post_id.iter().map(|(post_id, entry)| (post_id.to_string(), entry.count)).collect();
        for (post_id, runtime) in &s.stats_runtime_by_post_id {
            if s.stats_by_post_id.contains_key(post_id) {
                continue;
            }
            stats.insert(
                post_id.to_string(),
                RisePostStatsPresentationState {
                    value: None,
                    is_loading: runtime.is_loading,
                    error_message: runtime.error_message.clone(),
                    source: runtime.source,
                    updated_at_milliseconds: 0,
                },
            );
        }
        RisePostPresentationSnapshot {
            account_id: self.account_id.clone(),
            account_epoch: self.account_epoch.clone(),
            session_id: self.session_id,
            revision: s.presentation_revision,
            stats_by_post_id: stats,
            view_counts_by_post_id: view_counts,
        }
    }

    async fn persist(&self) {
        let payload = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            Some(RisePostRepositoryPersistentCache {
                stats_by_post_id: s.stats_by_post_id.iter().map(|(id, entry)| (id.to_string(), entry.clone())).collect(),
                view_counts_by_post_id: s
                    .view_counts_by_post_id
                    .iter()
                    .map(|(id, entry)| (id.to_string(), entry.clone()))
                    .collect(),
                view_aliases_by_post_id: Some(
                    s.view_aliases_by_post_id.iter().map(|(id, entry)| (id.to_string(), entry.clone())).collect(),
                ),
            })
        });
        let Some(payload) = payload else {
            return;
        };
        let Ok(data) = Self::encode(payload).await else {
            return;
        };
        let version = self.state.with(|s| {
            s.store_version = (s.store_version + 1).max(Self::now_milliseconds());
            s.store_version
        });
        let value = RiseStoredValue::new(data, version, Self::now_milliseconds());
        let _ = self
            .engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: Self::namespace(),
                    key: Self::CACHE_KEY.to_owned(),
                    value,
                    precondition: RiseStoreVersionPrecondition::Any,
                    policy: RiseStoredValueWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await;
    }

    fn bump_presentation_revision(s: &mut RisePostRepositoryState) {
        s.presentation_revision = (s.presentation_revision + 1).max(Self::now_milliseconds());
    }

    fn is_current_account(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(Some(&self.account_id), Some(&self.account_epoch))
    }

    async fn encode(value: RisePostRepositoryPersistentCache) -> anyhow::Result<Vec<u8>> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<u8>> { Ok(serde_json::to_vec(&value)?) })
            .await?
    }

    async fn decode(data: Vec<u8>) -> anyhow::Result<RisePostRepositoryPersistentCache> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<RisePostRepositoryPersistentCache> {
                Ok(serde_json::from_slice(&data)?)
            })
            .await?
    }

    async fn encoded_cost(value: RiseStoredPostStats) -> i64 {
        RiseAccountRuntime::current()
            .spawn_blocking(move || serde_json::to_vec(&value).map(|data| data.len()).unwrap_or(0))
            .await
            .ok()
            .and_then(|count| i64::try_from(count).ok())
            .unwrap_or(0)
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
