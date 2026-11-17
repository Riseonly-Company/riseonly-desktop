use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use rise_canon::rise_proto;
use rise_canon::rise_request;
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine, RiseWireError};
use tokio::task::AbortHandle;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::user::engine::core::rise_user_block_resource_repository::{
    RiseUserBlockCacheSource, RiseUserBlockPersistedEntry, RiseUserBlockResourcePayload,
    RiseUserBlockResourceRepository,
};
use crate::modules::user::engine::core::rise_user_block_wire_models::{
    RiseBlockScope, RiseUserBlockChange,
};
use crate::modules::user::engine::core::rise_user_rpc::RiseUserRPC;
use crate::modules::user::engine::rise_user_block_presentation::{
    RiseBlockedUser, RiseUserBlockLoadState, RiseUserBlockPresentation, RiseUserBlockSnapshot,
};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseUserBlockError {
    #[error("{0}")]
    Rejected(String),
    #[error("invalidTarget")]
    InvalidTarget,
}

impl RiseUserBlockError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            RiseUserBlockError::Rejected(message) => (!message.is_empty()).then(|| message.clone()),
            RiseUserBlockError::InvalidTarget => None,
        }
    }
}

static REFRESH_INTERVAL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::freshness_ms("user_settings"));

static MAXIMUM_PAGE_SIZE: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("max_blocked_users_page"));

/// the running refresh, done resolves when it ends without cancelling it and abort cancels
struct RiseUserBlockInFlightRefresh {
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

impl RiseUserBlockInFlightRefresh {
    fn cancel(&self) {
        self.abort.abort();
    }
}

struct RiseUserBlockRepositoryState {
    snapshot: RiseUserBlockSnapshot,
    is_shutdown: bool,
    request_generation: u64,
    mutation_generations: HashMap<String, u64>,
    last_refreshed_at_ms: i64,
    in_flight_refresh: Option<RiseUserBlockInFlightRefresh>,
    scope_stamps_ms: HashMap<String, i64>,
}

enum RiseUserBlockLoadStep {
    Skip,
    Join(Shared<BoxFuture<'static, ()>>),
    Start {
        generation: u64,
        done: Shared<BoxFuture<'static, ()>>,
    },
}

struct RiseUserBlockMutationBase {
    generation: u64,
    previous_scope: Option<RiseBlockScope>,
    previous_entry: Option<RiseBlockedUser>,
    previous_stamp: Option<i64>,
    previous_refreshed_at_ms: i64,
}

/// owns the accounts personal blocks, the privacy list and the per target tier every gate reads
/// only blocks it placed
pub struct RiseUserBlockRepository {
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    resource: RiseUserBlockResourceRepository,
    rpc: RiseUserRPC,
    state: RiseIsolated<RiseUserBlockRepositoryState>,
}

impl RiseUserBlockRepository {
    /// silent server ceiling on one page
    pub fn maximum_page_size() -> i64 {
        *MAXIMUM_PAGE_SIZE
    }

    /// tiers kept resident, listed rows are never evicted so this only bounds the tail of visited profiles
    fn maximum_scopes() -> usize {
        usize::try_from(Self::maximum_page_size().saturating_mul(12)).unwrap_or(0)
    }

    /// rows written to disk, two pages is enough for a cold launch and keeps the encrypted blob small
    fn persisted_entries() -> usize {
        usize::try_from(Self::maximum_page_size().saturating_mul(2)).unwrap_or(0)
    }

    fn page_size() -> usize {
        usize::try_from(Self::maximum_page_size()).unwrap_or(0)
    }

    pub fn new(
        engine: Arc<RiseEngine>,
        account_id: String,
        account_epoch: String,
        session_id: Uuid,
    ) -> Self {
        let rpc = RiseUserRPC::new(engine.clone());
        let resource = RiseUserBlockResourceRepository::new(engine, &account_id);
        Self {
            account_id,
            account_epoch,
            session_id,
            resource,
            rpc,
            state: RiseIsolated::new(RiseUserBlockRepositoryState {
                snapshot: RiseUserBlockSnapshot::empty(),
                is_shutdown: false,
                request_generation: 0,
                mutation_generations: HashMap::new(),
                last_refreshed_at_ms: 0,
                in_flight_refresh: None,
                scope_stamps_ms: HashMap::new(),
            }),
        }
    }

    /// reads what is already known, no network. gates get real tiers before the first profile opens
    pub async fn activate(&self) -> RiseUserBlockSnapshot {
        if let Some(snapshot) = self
            .state
            .with(|s| s.is_shutdown.then(|| s.snapshot.clone()))
        {
            return snapshot;
        }

        let Ok(cached) = self.resource.cached().await else {
            return self.current();
        };
        self.state.with(|s| {
            if s.is_shutdown || cached.source == RiseUserBlockCacheSource::Miss {
                return s.snapshot.clone();
            }

            let entries = Self::ordered(cached.payload.entries.iter().map(Self::restored).collect());
            let mut scopes = cached.payload.scopes.clone();
            let mut stamps: HashMap<String, i64> = HashMap::new();
            for entry in &entries {
                scopes.insert(entry.user_id.clone(), entry.scope);
                stamps.insert(entry.user_id.clone(), entry.blocked_at_ms);
            }
            s.scope_stamps_ms = stamps;

            let mut seed = RiseUserBlockSnapshot {
                entries,
                scopes,
                has_more: cached.payload.has_more,
                next_cursor: cached.payload.next_cursor.clone(),
                load_state: RiseUserBlockLoadState::Loaded,
                pending: HashSet::new(),
                error: None,
                is_loading_more: false,
            };
            Self::bound_scopes(&mut s.scope_stamps_ms, &mut seed);
            s.snapshot = seed;
            s.snapshot.clone()
        })
    }

    pub fn current(&self) -> RiseUserBlockSnapshot {
        self.state.with(|s| s.snapshot.clone())
    }

    pub async fn load(self: &Arc<Self>, force: bool) {
        let now = Self::now_milliseconds();
        let weak = Arc::downgrade(self);
        let step = self.state.with(|s| {
            if s.is_shutdown {
                return RiseUserBlockLoadStep::Skip;
            }
            if !force
                && s.snapshot.load_state == RiseUserBlockLoadState::Loaded
                && now - s.last_refreshed_at_ms < *REFRESH_INTERVAL_MS
            {
                if s.snapshot.error.is_some() {
                    let mut cleared = s.snapshot.clone();
                    cleared.error = None;
                    self.publish(s, cleared);
                }
                return RiseUserBlockLoadStep::Skip;
            }
            if !force && let Some(in_flight) = s.in_flight_refresh.as_ref() {
                return RiseUserBlockLoadStep::Join(in_flight.done.clone());
            }
            if let Some(in_flight) = s.in_flight_refresh.take() {
                in_flight.cancel();
            }

            s.request_generation = s.request_generation.wrapping_add(1);
            let generation = s.request_generation;

            let mut pending_snapshot = s.snapshot.clone();
            pending_snapshot.error = None;
            if pending_snapshot.entries.is_empty()
                && pending_snapshot.load_state != RiseUserBlockLoadState::Loaded
            {
                pending_snapshot.load_state = RiseUserBlockLoadState::Loading;
            }
            self.publish(s, pending_snapshot);

            let handle = RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = weak.upgrade() {
                    this.perform_refresh(generation).await;
                }
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.in_flight_refresh = Some(RiseUserBlockInFlightRefresh {
                abort,
                done: done.clone(),
            });
            RiseUserBlockLoadStep::Start { generation, done }
        });

        let (generation, done) = match step {
            RiseUserBlockLoadStep::Skip => return,
            RiseUserBlockLoadStep::Join(done) => {
                done.await;
                return;
            }
            RiseUserBlockLoadStep::Start { generation, done } => (generation, done),
        };

        done.await;
        self.state.with(|s| {
            if s.request_generation == generation {
                s.in_flight_refresh = None;
            }
        });
    }

    async fn perform_refresh(&self, generation: u64) {
        let request = rise_request::user::GetBlockedUsers {
            limit: Some(Self::maximum_page_size()),
            relative_id: None,
        };
        let asked_scopes = self.state.with(|s| s.snapshot.scopes.clone());
        match self.rpc.get_blocked_users(request).await {
            Ok(response) => {
                let page = Self::ordered(Self::presentation(&response.items));
                let published = self.state.with(|s| {
                    if s.is_shutdown || generation != s.request_generation {
                        return false;
                    }
                    s.last_refreshed_at_ms = Self::now_milliseconds();

                    let mut next = s.snapshot.clone();
                    next.entries = Self::with_pending_overlay(&s.snapshot, page.clone());
                    next.has_more = response.has_more;
                    next.next_cursor = if response.has_more {
                        response.next_cursor.clone()
                    } else {
                        None
                    };
                    next.load_state = RiseUserBlockLoadState::Loaded;
                    next.error = None;
                    Self::adopt_scopes(&mut s.scope_stamps_ms, &page, &mut next);
                    if !response.has_more {
                        Self::drop_unlisted_scopes(&mut s.scope_stamps_ms, &asked_scopes, &page, &mut next);
                    }
                    self.publish(s, next)
                });
                if published {
                    self.persist().await;
                }
            }
            Err(error) => {
                if Self::is_transport_unavailable(&error) {
                    return;
                }
                let message = error.to_string();
                self.state.with(|s| {
                    if s.is_shutdown || generation != s.request_generation {
                        return;
                    }
                    self.finish_with_load_error(s, &message);
                });
            }
        }
    }

    /// fetches the next page and appends it, a prepend would move rows under the finger
    pub async fn load_more(&self) {
        let started = self.state.with(|s| {
            if s.is_shutdown
                || !s.snapshot.has_more
                || s.snapshot.is_loading_more
                || s.in_flight_refresh.is_some()
            {
                return None;
            }
            let cursor = s.snapshot.next_cursor.clone()?;
            let mut loading = s.snapshot.clone();
            loading.is_loading_more = true;
            self.publish(s, loading);

            s.request_generation = s.request_generation.wrapping_add(1);
            Some((cursor, s.request_generation))
        });
        let Some((cursor, generation)) = started else {
            return;
        };

        let request = rise_request::user::GetBlockedUsers {
            limit: Some(Self::maximum_page_size()),
            relative_id: Some(cursor),
        };
        let result = self.rpc.get_blocked_users(request).await;

        let published = match result {
            Ok(response) => self.state.with(|s| {
                if s.is_shutdown || generation != s.request_generation {
                    return false;
                }
                let page = Self::presentation(&response.items);
                let mut merged = s.snapshot.entries.clone();
                let known: HashSet<String> = merged.iter().map(|entry| entry.user_id.clone()).collect();
                merged.extend(
                    Self::ordered(page.clone())
                        .into_iter()
                        .filter(|entry| !known.contains(&entry.user_id)),
                );

                let mut next = s.snapshot.clone();
                next.is_loading_more = false;
                next.entries = Self::with_pending_overlay(&s.snapshot, merged);
                next.has_more = response.has_more;
                next.next_cursor = if response.has_more {
                    response.next_cursor.clone()
                } else {
                    None
                };
                Self::adopt_scopes(&mut s.scope_stamps_ms, &page, &mut next);
                self.publish(s, next)
            }),
            Err(_) => false,
        };
        if published {
            self.persist().await;
        }
        self.finish_load_more();
    }

    pub async fn trim_unobserved(&self) {
        self.state.with(|s| {
            if s.is_shutdown || s.snapshot.entries.len() <= Self::page_size() {
                return;
            }
            s.request_generation = s.request_generation.wrapping_add(1);
            if let Some(in_flight) = s.in_flight_refresh.take() {
                in_flight.cancel();
            }
            let mut next = s.snapshot.clone();
            next.entries.truncate(Self::page_size());
            next.has_more = true;
            next.next_cursor = None;
            s.last_refreshed_at_ms = 0;
            self.publish(s, next);
        });
    }

    pub async fn set_block(&self, target_user_id: &str, scope: Option<RiseBlockScope>) -> anyhow::Result<()> {
        if self.state.with(|s| s.is_shutdown) {
            return Err(CancellationError.into());
        }
        let target_user_id = trimming_whitespaces_and_newlines(target_user_id).to_owned();
        if target_user_id.is_empty() || target_user_id == self.account_id {
            return Err(RiseUserBlockError::InvalidTarget.into());
        }

        let base = self.state.with(|s| {
            let previous_scope = s.snapshot.scopes.get(&target_user_id).copied();
            if let Some(scope) = scope
                && previous_scope == Some(scope)
            {
                return None;
            }

            let previous_index = s
                .snapshot
                .entries
                .iter()
                .position(|entry| entry.user_id == target_user_id);
            let previous_entry = previous_index.map(|index| s.snapshot.entries[index].clone());
            let previous_stamp = s.scope_stamps_ms.get(&target_user_id).copied();
            let previous_refreshed_at_ms = s.last_refreshed_at_ms;

            let generation = s
                .mutation_generations
                .get(&target_user_id)
                .copied()
                .unwrap_or(0)
                .wrapping_add(1);
            s.mutation_generations.insert(target_user_id.clone(), generation);

            let mut optimistic = s.snapshot.clone();
            optimistic.pending.insert(target_user_id.clone());
            optimistic.error = None;
            if let Some(scope) = scope {
                optimistic.scopes.insert(target_user_id.clone(), scope);
                if let (Some(index), Some(entry)) = (previous_index, previous_entry.as_ref()) {
                    optimistic.entries[index] = Self::re_scoped(entry, scope);
                } else {
                    s.last_refreshed_at_ms = 0;
                }
            } else {
                optimistic.scopes.remove(&target_user_id);
                if let Some(index) = previous_index {
                    optimistic.entries.remove(index);
                }
            }
            Self::bound_scopes(&mut s.scope_stamps_ms, &mut optimistic);
            self.publish(s, optimistic);

            Some(RiseUserBlockMutationBase {
                generation,
                previous_scope,
                previous_entry,
                previous_stamp,
                previous_refreshed_at_ms,
            })
        });
        let Some(base) = base else {
            return Ok(());
        };
        self.persist().await;

        let client_request_id = Uuid::new_v4().uuid_string();
        let result: anyhow::Result<Option<rise_proto::user::ContactBlockRelation>> = match scope {
            Some(scope) => {
                let mut request =
                    rise_request::user::BlockUser::new(client_request_id, target_user_id.clone());
                request.block_scope = Some(scope.raw_value().to_owned());
                self.rpc.block_user(request).await.map(|response| response.relation)
            }
            None => {
                let request =
                    rise_request::user::UnblockUser::new(client_request_id, target_user_id.clone());
                self.rpc.unblock_user(request).await.map(|response| response.relation)
            }
        };

        match result {
            Ok(relation) => {
                let settled_now = self.state.with(|s| {
                    if s.is_shutdown
                        || s.mutation_generations.get(&target_user_id) != Some(&base.generation)
                    {
                        return false;
                    }

                    let settled_scope = match relation.as_ref() {
                        Some(relation) if !relation.target_user_id.is_empty() => relation
                            .is_blocked
                            .then(|| {
                                RiseBlockScope::from_raw_value(&relation.block_scope)
                                    .or(scope)
                                    .unwrap_or(RiseBlockScope::Standard)
                            }),
                        _ => scope,
                    };

                    let mut settled = s.snapshot.clone();
                    settled.pending.remove(&target_user_id);
                    settled.error = None;
                    let blocked_at_ms = relation
                        .as_ref()
                        .map(|relation| relation.blocked_at_ms)
                        .unwrap_or_else(Self::now_milliseconds);
                    Self::apply(
                        &mut s.scope_stamps_ms,
                        &target_user_id,
                        settled_scope,
                        blocked_at_ms,
                        &mut settled,
                    );
                    self.publish(s, settled);
                    true
                });
                if settled_now {
                    self.persist().await;
                }
                Ok(())
            }
            Err(error) => {
                let message = Self::message(&error.to_string());
                let reverted_now = self.state.with(|s| {
                    if s.is_shutdown
                        || s.mutation_generations.get(&target_user_id) != Some(&base.generation)
                    {
                        return false;
                    }

                    let mut reverted = s.snapshot.clone();
                    reverted.pending.remove(&target_user_id);
                    match base.previous_scope {
                        Some(previous_scope) => {
                            reverted.scopes.insert(target_user_id.clone(), previous_scope);
                        }
                        None => {
                            reverted.scopes.remove(&target_user_id);
                        }
                    }
                    if let Some(previous_entry) = base.previous_entry.as_ref() {
                        if let Some(index) = reverted
                            .entries
                            .iter()
                            .position(|entry| entry.user_id == target_user_id)
                        {
                            reverted.entries[index] = previous_entry.clone();
                        } else {
                            let mut entries = reverted.entries.clone();
                            entries.push(previous_entry.clone());
                            reverted.entries = Self::ordered(entries);
                        }
                    }
                    match base.previous_stamp {
                        Some(previous_stamp) => {
                            s.scope_stamps_ms.insert(target_user_id.clone(), previous_stamp);
                        }
                        None => {
                            s.scope_stamps_ms.remove(&target_user_id);
                        }
                    }
                    s.last_refreshed_at_ms = s.last_refreshed_at_ms.min(base.previous_refreshed_at_ms);
                    reverted.error = message;
                    Self::bound_scopes(&mut s.scope_stamps_ms, &mut reverted);
                    self.publish(s, reverted);
                    true
                });
                if reverted_now {
                    self.persist().await;
                }
                Err(error)
            }
        }
    }

    pub async fn apply_remote_change(&self, change: RiseUserBlockChange) {
        let published = self.state.with(|s| {
            if s.is_shutdown || change.target_user_id.is_empty() {
                return false;
            }
            if s.snapshot.pending.contains(&change.target_user_id) {
                return false;
            }
            let known = s
                .scope_stamps_ms
                .get(&change.target_user_id)
                .copied()
                .unwrap_or(0);
            if change.is_blocked && change.blocked_at_ms > 0 && change.blocked_at_ms < known {
                return false;
            }

            let mut next = s.snapshot.clone();
            Self::apply(
                &mut s.scope_stamps_ms,
                &change.target_user_id,
                change.scope,
                change.blocked_at_ms,
                &mut next,
            );
            self.publish(s, next)
        });
        if published {
            self.persist().await;
        }
    }

    pub async fn note_block_scope(&self, target_user_id: &str, scope: Option<RiseBlockScope>) {
        let published = self.state.with(|s| {
            if s.is_shutdown || target_user_id.is_empty() || target_user_id == self.account_id {
                return false;
            }
            if s.snapshot.pending.contains(target_user_id) {
                return false;
            }
            if s.snapshot.scopes.get(target_user_id).copied() == scope {
                return false;
            }

            let mut next = s.snapshot.clone();
            let stamp = s.scope_stamps_ms.get(target_user_id).copied().unwrap_or(0);
            Self::apply(&mut s.scope_stamps_ms, target_user_id, scope, stamp, &mut next);
            self.publish(s, next)
        });
        if published {
            self.persist().await;
        }
    }

    pub async fn shutdown(&self) {
        let stopped_now = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.is_shutdown = true;
            if let Some(in_flight) = s.in_flight_refresh.take() {
                in_flight.cancel();
            }
            s.mutation_generations.clear();
            s.scope_stamps_ms.clear();
            true
        });
        if !stopped_now {
            return;
        }
        self.resource.shutdown().await;
    }

    fn apply(
        scope_stamps_ms: &mut HashMap<String, i64>,
        target_user_id: &str,
        scope: Option<RiseBlockScope>,
        blocked_at_ms: i64,
        next: &mut RiseUserBlockSnapshot,
    ) {
        match scope {
            Some(scope) => {
                next.scopes.insert(target_user_id.to_owned(), scope);
                if let Some(index) = next
                    .entries
                    .iter()
                    .position(|entry| entry.user_id == target_user_id)
                {
                    next.entries[index] = Self::re_scoped(&next.entries[index], scope);
                }
                if blocked_at_ms > 0 {
                    scope_stamps_ms.insert(target_user_id.to_owned(), blocked_at_ms);
                }
            }
            None => {
                next.scopes.remove(target_user_id);
                next.entries.retain(|entry| entry.user_id != target_user_id);
                scope_stamps_ms.remove(target_user_id);
            }
        }
        Self::bound_scopes(scope_stamps_ms, next);
    }

    /// folds the tiers a server page states into the map, targets it didnt name stay
    /// a page is only the newest fifty so an old block is just absent
    fn adopt_scopes(
        scope_stamps_ms: &mut HashMap<String, i64>,
        page: &[RiseBlockedUser],
        next: &mut RiseUserBlockSnapshot,
    ) {
        for entry in page {
            if next.pending.contains(&entry.user_id) {
                continue;
            }
            next.scopes.insert(entry.user_id.clone(), entry.scope);
            if entry.blocked_at_ms > 0 {
                scope_stamps_ms.insert(entry.user_id.clone(), entry.blocked_at_ms);
            }
        }
        Self::bound_scopes(scope_stamps_ms, next);
    }

    fn drop_unlisted_scopes(
        scope_stamps_ms: &mut HashMap<String, i64>,
        asked_scopes: &HashMap<String, RiseBlockScope>,
        page: &[RiseBlockedUser],
        next: &mut RiseUserBlockSnapshot,
    ) {
        let listed: HashSet<&str> = page.iter().map(|entry| entry.user_id.as_str()).collect();
        let RiseUserBlockSnapshot { scopes, pending, .. } = next;
        let before = scopes.len();
        scopes.retain(|key, scope| {
            listed.contains(key.as_str()) || pending.contains(key) || asked_scopes.get(key) != Some(&*scope)
        });
        if scopes.len() != before {
            tracing::warn!(
                target: "riseonly::X1903",
                "block map dropped unlisted scopes before={} after={} listed={}",
                before,
                scopes.len(),
                listed.len()
            );
        }
        scope_stamps_ms.retain(|key, _| scopes.contains_key(key));
    }

    /// server rows with this devices in flight intent laid over them
    /// so a refresh during unblock doesnt flicker it back
    fn with_pending_overlay(
        snapshot: &RiseUserBlockSnapshot,
        entries: Vec<RiseBlockedUser>,
    ) -> Vec<RiseBlockedUser> {
        if snapshot.pending.is_empty() {
            return entries;
        }
        entries
            .into_iter()
            .filter_map(|entry| {
                if !snapshot.pending.contains(&entry.user_id) {
                    return Some(entry);
                }
                let scope = snapshot.scopes.get(&entry.user_id).copied()?;
                Some(Self::re_scoped(&entry, scope))
            })
            .collect()
    }

    /// drops the tail of profiles this session only visited, never rows on screen
    /// dropped tier is relearned on the next profile read
    fn bound_scopes(scope_stamps_ms: &mut HashMap<String, i64>, next: &mut RiseUserBlockSnapshot) {
        let maximum_scopes = Self::maximum_scopes();
        if next.scopes.len() <= maximum_scopes {
            Self::prune_stamps(scope_stamps_ms, &next.scopes);
            return;
        }
        let listed: HashSet<String> = next.entries.iter().map(|entry| entry.user_id.clone()).collect();
        let room = maximum_scopes.saturating_sub(listed.len());
        let mut survivors: Vec<String> = next
            .scopes
            .keys()
            .filter(|key| !listed.contains(*key))
            .cloned()
            .collect();
        survivors.sort_by(|left_key, right_key| {
            let left = scope_stamps_ms.get(left_key).copied().unwrap_or(0);
            let right = scope_stamps_ms.get(right_key).copied().unwrap_or(0);
            if left == right {
                right_key.cmp(left_key)
            } else {
                right.cmp(&left)
            }
        });
        survivors.truncate(room);
        let mut keep = listed;
        keep.extend(survivors);
        next.scopes.retain(|key, _| keep.contains(key));
        Self::prune_stamps(scope_stamps_ms, &next.scopes);
    }

    fn prune_stamps(
        scope_stamps_ms: &mut HashMap<String, i64>,
        scopes: &HashMap<String, RiseBlockScope>,
    ) {
        if scope_stamps_ms.len() <= scopes.len() {
            return;
        }
        scope_stamps_ms.retain(|key, _| scopes.contains_key(key));
    }

    fn finish_load_more(&self) {
        self.state.with(|s| {
            if !s.snapshot.is_loading_more {
                return;
            }
            let mut next = s.snapshot.clone();
            next.is_loading_more = false;
            self.publish(s, next);
        });
    }

    fn finish_with_load_error(&self, s: &mut RiseUserBlockRepositoryState, message: &str) {
        let mut next = s.snapshot.clone();
        next.load_state = if next.entries.is_empty() {
            RiseUserBlockLoadState::Failed
        } else {
            RiseUserBlockLoadState::Loaded
        };
        next.error = Self::message(message);
        self.publish(s, next);
    }

    async fn persist(&self) {
        let payload = self.state.with(|s| {
            let persisted_entries = Self::persisted_entries();
            let truncated = s.snapshot.entries.len() > persisted_entries;
            RiseUserBlockResourcePayload {
                entries: s
                    .snapshot
                    .entries
                    .iter()
                    .take(persisted_entries)
                    .map(Self::persisted)
                    .collect(),
                scopes: s.snapshot.scopes.clone(),
                has_more: s.snapshot.has_more || truncated,
                next_cursor: if truncated {
                    None
                } else {
                    s.snapshot.next_cursor.clone()
                },
            }
        });
        let _ = self.resource.store(&payload).await;
    }

    /// mutates and publishes in one step, returns whether the snapshot changed
    fn publish(&self, s: &mut RiseUserBlockRepositoryState, next: RiseUserBlockSnapshot) -> bool {
        if next == s.snapshot {
            return false;
        }
        s.snapshot = next.clone();

        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        if let Some(hop) = RiseEngineMainHop::installed() {
            hop.post(move |cx| {
                RiseUserBlockPresentation::shared(cx).update(cx, |presentation, cx| {
                    presentation.publish(next, &account_id, &account_epoch, session_id, cx);
                });
            });
        }
        true
    }

    fn presentation(items: &[rise_proto::user::BlockedUserDto]) -> Vec<RiseBlockedUser> {
        items
            .iter()
            .filter_map(|item| {
                let user = item.user.as_ref()?;
                if user.id.is_empty() {
                    return None;
                }
                Some(RiseBlockedUser {
                    user_id: user.id.clone(),
                    name: user.name.clone(),
                    tag: user.tag.clone(),
                    logo: user.logo.clone(),
                    is_premium: user.is_premium,
                    is_deleted: user.is_deleted,
                    scope: RiseBlockScope::from_raw_value(&item.block_scope)
                        .unwrap_or(RiseBlockScope::Standard),
                    blocked_at_ms: item.blocked_at_ms,
                })
            })
            .collect()
    }

    fn persisted(entry: &RiseBlockedUser) -> RiseUserBlockPersistedEntry {
        RiseUserBlockPersistedEntry {
            user_id: entry.user_id.clone(),
            name: entry.name.clone(),
            tag: entry.tag.clone(),
            logo: entry.logo.clone(),
            is_premium: entry.is_premium,
            is_deleted: entry.is_deleted,
            scope: entry.scope,
            blocked_at_ms: entry.blocked_at_ms,
        }
    }

    fn restored(entry: &RiseUserBlockPersistedEntry) -> RiseBlockedUser {
        RiseBlockedUser {
            user_id: entry.user_id.clone(),
            name: entry.name.clone(),
            tag: entry.tag.clone(),
            logo: entry.logo.clone(),
            is_premium: entry.is_premium,
            is_deleted: entry.is_deleted,
            scope: entry.scope,
            blocked_at_ms: entry.blocked_at_ms,
        }
    }

    fn re_scoped(entry: &RiseBlockedUser, scope: RiseBlockScope) -> RiseBlockedUser {
        if entry.scope == scope {
            return entry.clone();
        }
        RiseBlockedUser {
            scope,
            ..entry.clone()
        }
    }

    /// newest first,
    /// tie broken by id,
    /// same as server paging so cached and fetched lists dont reshuffle
    fn ordered(mut entries: Vec<RiseBlockedUser>) -> Vec<RiseBlockedUser> {
        entries.sort_by(|left, right| {
            if left.blocked_at_ms == right.blocked_at_ms {
                right.user_id.cmp(&left.user_id)
            } else {
                right.blocked_at_ms.cmp(&left.blocked_at_ms)
            }
        });
        entries
    }

    fn is_transport_unavailable(error: &anyhow::Error) -> bool {
        matches!(
            error.downcast_ref::<RiseWireError>(),
            Some(RiseWireError::NoSocketSession | RiseWireError::NotConnected | RiseWireError::Suspended)
        )
    }

    fn message(raw: &str) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
