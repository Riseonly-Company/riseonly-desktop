use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine, RiseWireError};
use tokio::task::AbortHandle;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::user::engine::core::rise_user_achievement_wire_models::{
    RiseAchievement, RiseUserAchievementsError,
};
use crate::modules::user::engine::core::rise_user_achievements_resource_repository::{
    RiseUserAchievementsCacheSource, RiseUserAchievementsResourcePayload,
    RiseUserAchievementsResourceRepository,
};
use crate::modules::user::engine::core::rise_user_debug_trace::RiseUserDebugTrace;
use crate::modules::user::engine::core::rise_user_rpc::RiseUserRPC;
use crate::modules::user::engine::rise_user_achievements_presentation::{
    RiseUserAchievementsLoadState, RiseUserAchievementsPresentation, RiseUserAchievementsShowcase,
    RiseUserAchievementsSnapshot,
};
use crate::modules::user::engine::rise_user_achievements_realtime::RiseAchievementGrant;

static REFRESH_INTERVAL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::freshness_ms("user_achievements"));

/// the running refresh, done resolves when it ends without cancelling it and abort cancels
struct RiseUserAchievementsInFlight {
    generation: u64,
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

struct RiseUserAchievementsRepositoryState {
    snapshot: RiseUserAchievementsSnapshot,
    request_generation: u64,
    mutation_generation: u64,
    last_refreshed_at_ms: i64,
    is_shutdown: bool,
    in_flight_refresh: Option<RiseUserAchievementsInFlight>,
    refresh_user_asked: bool,
    showcase_generation_clock: u64,
    showcase_generations: HashMap<String, u64>,
    showcase_refreshed_at_ms: HashMap<String, i64>,
    showcase_in_flight: HashMap<String, RiseUserAchievementsInFlight>,
}

enum RiseUserAchievementsLoadStep {
    Skip,
    Join(Shared<BoxFuture<'static, ()>>),
    Start {
        generation: u64,
        done: Shared<BoxFuture<'static, ()>>,
    },
}

struct RiseUserAchievementsMutation {
    previous: RiseAchievement,
    previous_hidden_count: i64,
    generation: u64,
}

/// owns the accounts achievements and a bounded set of other showcases
/// cache first then hidden refresh, failure keeps snapshot
pub struct RiseUserAchievementsRepository {
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    resource: RiseUserAchievementsResourceRepository,
    rpc: RiseUserRPC,
    state: RiseIsolated<RiseUserAchievementsRepositoryState>,
}

impl RiseUserAchievementsRepository {
    pub fn maximum_achievements() -> i64 {
        RiseLimits::client_value("maximum_achievements")
    }

    pub fn maximum_showcases() -> i64 {
        RiseLimits::client_value("maximum_showcases")
    }

    pub fn new(
        engine: Arc<RiseEngine>,
        account_id: String,
        account_epoch: String,
        session_id: Uuid,
    ) -> Self {
        let rpc = RiseUserRPC::new(engine.clone());
        let resource = RiseUserAchievementsResourceRepository::new(engine, &account_id);
        Self {
            account_id,
            account_epoch,
            session_id,
            resource,
            rpc,
            state: RiseIsolated::new(RiseUserAchievementsRepositoryState {
                snapshot: RiseUserAchievementsSnapshot::empty(),
                request_generation: 0,
                mutation_generation: 0,
                last_refreshed_at_ms: 0,
                is_shutdown: false,
                in_flight_refresh: None,
                refresh_user_asked: false,
                showcase_generation_clock: 0,
                showcase_generations: HashMap::new(),
                showcase_refreshed_at_ms: HashMap::new(),
                showcase_in_flight: HashMap::new(),
            }),
        }
    }

    /// reads what is already known,
    /// no network,
    /// so the first snapshot costs no round trip
    pub async fn activate(&self) -> RiseUserAchievementsSnapshot {
        if let Some(snapshot) = self.stopped_snapshot() {
            return snapshot;
        }

        let Ok(cached) = self.resource.cached().await else {
            return self.current();
        };
        if let Some(snapshot) = self.stopped_snapshot() {
            return snapshot;
        }
        RiseUserDebugTrace::record_achievements_cache_read(
            cached.source,
            self.resource.debug_resource_key(),
            cached.revision,
        )
        .await;
        self.state.with(|s| {
            if s.is_shutdown || cached.source == RiseUserAchievementsCacheSource::Miss {
                return s.snapshot.clone();
            }

            let achievements = Self::normalized(cached.payload.achievements.clone());
            let loaded = i64::try_from(achievements.len()).unwrap_or(i64::MAX);
            let mut seeded = RiseUserAchievementsSnapshot::empty();
            seeded.total = cached.payload.total.max(loaded);
            seeded.achievements = achievements;
            seeded.hidden_count = cached.payload.hidden_count;
            seeded.catalog_total = 0.max(cached.payload.catalog_total);
            if !cached.payload.achievements_rule.is_empty() {
                seeded.achievements_rule = cached.payload.achievements_rule.clone();
            }
            seeded.load_state = RiseUserAchievementsLoadState::Loaded;
            seeded.cache_source = cached.source;
            seeded.is_refreshing = false;
            s.snapshot = Self::with_derived_sections(seeded);
            s.snapshot.clone()
        })
    }

    pub fn current(&self) -> RiseUserAchievementsSnapshot {
        self.state.with(|s| s.snapshot.clone())
    }

    pub async fn load(self: &Arc<Self>, force: bool) {
        let now = Self::now_milliseconds();
        let weak = Arc::downgrade(self);
        let step = self.state.with(|s| {
            if s.is_shutdown {
                return RiseUserAchievementsLoadStep::Skip;
            }
            if !force
                && s.snapshot.load_state == RiseUserAchievementsLoadState::Loaded
                && now - s.last_refreshed_at_ms < *REFRESH_INTERVAL_MS
            {
                self.publish(s, |snapshot| snapshot.error = None);
                return RiseUserAchievementsLoadStep::Skip;
            }
            if let Some(in_flight) = s.in_flight_refresh.as_ref() {
                s.refresh_user_asked |= force;
                return RiseUserAchievementsLoadStep::Join(in_flight.done.clone());
            }

            s.request_generation = s.request_generation.wrapping_add(1);
            let generation = s.request_generation;
            s.refresh_user_asked = force;
            self.publish(s, |snapshot| {
                snapshot.is_refreshing = true;
                snapshot.error = None;
                if snapshot.achievements.is_empty() {
                    snapshot.load_state = RiseUserAchievementsLoadState::Loading;
                }
            });

            let handle = RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = weak.upgrade() {
                    this.perform_refresh(generation, force).await;
                }
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.in_flight_refresh = Some(RiseUserAchievementsInFlight {
                generation,
                abort,
                done: done.clone(),
            });
            RiseUserAchievementsLoadStep::Start { generation, done }
        });

        let (generation, done) = match step {
            RiseUserAchievementsLoadStep::Skip => return,
            RiseUserAchievementsLoadStep::Join(done) => {
                done.await;
                return;
            }
            RiseUserAchievementsLoadStep::Start { generation, done } => (generation, done),
        };

        done.await;
        self.state.with(|s| {
            if s.in_flight_refresh.as_ref().map(|in_flight| in_flight.generation) == Some(generation) {
                s.in_flight_refresh = None;
            }
        });
    }

    async fn perform_refresh(&self, generation: u64, user_initiated: bool) {
        let response = match self.rpc.get_my_achievements().await {
            Ok(response) => response,
            Err(error) => {
                self.state.with(|s| {
                    if s.is_shutdown || generation != s.request_generation {
                        return;
                    }
                    if !user_initiated && !s.refresh_user_asked && Self::is_transport_unavailable(&error) {
                        self.publish(s, |snapshot| snapshot.is_refreshing = false);
                        return;
                    }
                    let message = CanonErrors::load_failure_text(&error);
                    self.publish(s, |snapshot| {
                        snapshot.is_refreshing = false;
                        snapshot.load_state = if snapshot.achievements.is_empty() {
                            RiseUserAchievementsLoadState::Failed
                        } else {
                            RiseUserAchievementsLoadState::Loaded
                        };
                        snapshot.error = Some(message);
                    });
                });
                return;
            }
        };

        let Some(catalog_total) = self.state.with(|s| {
            if s.is_shutdown || generation != s.request_generation {
                return None;
            }
            let catalog_total = if response.catalog_total > 0 {
                response.catalog_total
            } else {
                s.snapshot.catalog_total
            };
            s.last_refreshed_at_ms = Self::now_milliseconds();
            Some(catalog_total)
        }) else {
            return;
        };

        let achievements = Self::normalized(
            response
                .achievements
                .iter()
                .map(RiseAchievement::from_canon)
                .collect(),
        );
        let revision = self
            .resource
            .store(&RiseUserAchievementsResourcePayload {
                achievements: achievements.clone(),
                total: response.total,
                hidden_count: response.hidden_count,
                catalog_total,
                achievements_rule: response.achievements_rule.clone(),
            })
            .await
            .ok();
        if !self.is_current_request(generation) {
            return;
        }
        RiseUserDebugTrace::record_achievements_cache_read(
            RiseUserAchievementsCacheSource::Network,
            self.resource.debug_resource_key(),
            revision.unwrap_or(0),
        )
        .await;

        self.state.with(|s| {
            if s.is_shutdown || generation != s.request_generation {
                return;
            }
            let loaded = i64::try_from(achievements.len()).unwrap_or(i64::MAX);
            self.publish(s, |snapshot| {
                snapshot.achievements = achievements;
                snapshot.total = response.total.max(loaded);
                snapshot.hidden_count = 0.max(response.hidden_count);
                snapshot.catalog_total = catalog_total;
                if !response.achievements_rule.is_empty() {
                    snapshot.achievements_rule = response.achievements_rule.clone();
                }
                snapshot.load_state = RiseUserAchievementsLoadState::Loaded;
                snapshot.cache_source = RiseUserAchievementsCacheSource::Network;
                snapshot.is_refreshing = false;
                snapshot.mutating_ids = HashSet::new();
                snapshot.error = None;
            });
        });
    }

    fn is_transport_unavailable(error: &anyhow::Error) -> bool {
        matches!(
            error.downcast_ref::<RiseWireError>(),
            Some(RiseWireError::NoSocketSession | RiseWireError::NotConnected | RiseWireError::Suspended)
        )
    }

    pub async fn set_hidden(&self, achievement_id: i64, hidden: bool) -> anyhow::Result<()> {
        let step = self.state.with(|s| -> anyhow::Result<Option<RiseUserAchievementsMutation>> {
            if s.is_shutdown {
                return Err(CancellationError.into());
            }
            let Some(previous) = s
                .snapshot
                .achievements
                .iter()
                .find(|achievement| achievement.id == achievement_id)
                .cloned()
            else {
                return Err(RiseUserAchievementsError::InvalidResponse.into());
            };

            let previous_hidden_count = s.snapshot.hidden_count;
            if previous.hidden == hidden {
                return Ok(None);
            }
            let mut optimistic = previous.clone();
            optimistic.hidden = hidden;

            s.mutation_generation = s.mutation_generation.wrapping_add(1);
            let generation = s.mutation_generation;
            self.publish(s, |snapshot| {
                snapshot.achievements = Self::replacing(optimistic, &snapshot.achievements);
                snapshot.hidden_count = 0.max(previous_hidden_count + if hidden { 1 } else { -1 });
                snapshot.mutating_ids.insert(achievement_id);
                snapshot.mutation_error = None;
            });
            Ok(Some(RiseUserAchievementsMutation {
                previous,
                previous_hidden_count,
                generation,
            }))
        })?;
        let Some(mutation) = step else {
            return Ok(());
        };
        self.persist_current().await;

        let outcome = match self
            .rpc
            .set_achievement_hidden(rise_canon::rise_request::user::SetAchievementHidden::new(
                achievement_id,
                hidden,
            ))
            .await
        {
            Ok(response) => {
                if !self.is_current_mutation(mutation.generation) {
                    return Ok(());
                }
                match response.achievement.as_ref().map(RiseAchievement::from_canon) {
                    Some(confirmed) => Ok((confirmed, response.hidden_count)),
                    None => Err(anyhow::Error::from(RiseUserAchievementsError::InvalidResponse)),
                }
            }
            Err(error) => Err(error),
        };

        match outcome {
            Ok((confirmed, hidden_count)) => {
                let current = self.state.with(|s| {
                    if s.is_shutdown || mutation.generation != s.mutation_generation {
                        return false;
                    }
                    self.publish(s, |snapshot| {
                        snapshot.achievements = Self::replacing(confirmed, &snapshot.achievements);
                        snapshot.hidden_count = 0.max(hidden_count);
                        snapshot.mutating_ids.remove(&achievement_id);
                        snapshot.mutation_error = None;
                    });
                    true
                });
                if current {
                    self.persist_current().await;
                }
                Ok(())
            }
            Err(error) => {
                let current = self.state.with(|s| {
                    if s.is_shutdown || mutation.generation != s.mutation_generation {
                        return false;
                    }
                    let message = CanonErrors::refusal_text(&error);
                    let previous = mutation.previous.clone();
                    self.publish(s, |snapshot| {
                        snapshot.achievements = Self::replacing(previous, &snapshot.achievements);
                        snapshot.hidden_count = 0.max(mutation.previous_hidden_count);
                        snapshot.mutating_ids.remove(&achievement_id);
                        snapshot.mutation_error = Some(message);
                    });
                    true
                });
                if current {
                    self.persist_current().await;
                }
                Err(error)
            }
        }
    }

    pub async fn mark_seen(&self, ids: &[i64]) {
        let requested = Self::normalized_ids(ids);
        let Some((targets, generation)) = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            let targets: Vec<i64> = if requested.is_empty() {
                s.snapshot
                    .achievements
                    .iter()
                    .filter(|achievement| !achievement.is_seen())
                    .map(|achievement| achievement.id)
                    .collect()
            } else {
                requested.clone()
            };
            if targets.is_empty() {
                return None;
            }
            Some((targets, s.request_generation))
        }) else {
            return;
        };

        let acknowledged = self
            .rpc
            .mark_achievements_seen(rise_canon::rise_request::user::MarkAchievementsSeen {
                achievement_ids: Some(requested),
            })
            .await
            .is_ok();
        if !acknowledged {
            return;
        }

        let seen_at_ms = Self::now_milliseconds();
        let marked: HashSet<i64> = targets.into_iter().collect();
        let current = self.state.with(|s| {
            if s.is_shutdown || generation != s.request_generation {
                return false;
            }
            self.publish(s, |snapshot| {
                for achievement in &mut snapshot.achievements {
                    if marked.contains(&achievement.id) && !achievement.is_seen() {
                        achievement.seen_at_ms = seen_at_ms;
                    }
                }
            });
            true
        });
        if current {
            self.persist_current().await;
        }
    }

    pub async fn apply_grant(&self, grant: RiseAchievementGrant) {
        let applied = self.state.with(|s| {
            if s.is_shutdown || grant.achievement.id <= 0 {
                return false;
            }

            let achievement = grant.achievement;
            let existing_index = s
                .snapshot
                .achievements
                .iter()
                .position(|existing| existing.id == achievement.id);
            if let Some(index) = existing_index
                && s.snapshot.achievements[index] == achievement
            {
                return false;
            }

            let mut next = s.snapshot.achievements.clone();
            match existing_index {
                Some(index) => next[index] = achievement.clone(),
                None => next.insert(0, achievement.clone()),
            }
            let normalized = Self::normalized(next);
            let is_new = existing_index.is_none();

            self.publish(s, |snapshot| {
                snapshot.achievements = normalized;
                if is_new {
                    snapshot.total += 1;
                    if achievement.hidden {
                        snapshot.hidden_count += 1;
                    }
                }
                snapshot.load_state = RiseUserAchievementsLoadState::Loaded;
            });
            true
        });
        if applied {
            self.persist_current().await;
        }
    }

    pub async fn load_showcase(self: &Arc<Self>, user_id: &str, force: bool) {
        let user_id = trimming_whitespaces_and_newlines(user_id).to_owned();
        if user_id.is_empty() {
            return;
        }

        let now = Self::now_milliseconds();
        let weak = Arc::downgrade(self);
        let step = self.state.with(|s| {
            if s.is_shutdown {
                return RiseUserAchievementsLoadStep::Skip;
            }
            if !force
                && let Some(refreshed_at_ms) = s.showcase_refreshed_at_ms.get(&user_id)
                && now - refreshed_at_ms < *REFRESH_INTERVAL_MS
                && s.snapshot.showcase(&user_id).is_some()
            {
                return RiseUserAchievementsLoadStep::Skip;
            }
            if !force && let Some(in_flight) = s.showcase_in_flight.get(&user_id) {
                return RiseUserAchievementsLoadStep::Join(in_flight.done.clone());
            }
            if let Some(in_flight) = s.showcase_in_flight.remove(&user_id) {
                in_flight.abort.abort();
            }

            s.showcase_generation_clock = s.showcase_generation_clock.wrapping_add(1);
            let generation = s.showcase_generation_clock;
            s.showcase_generations.insert(user_id.clone(), generation);

            let mut pending = s
                .snapshot
                .showcase(&user_id)
                .cloned()
                .unwrap_or_else(|| RiseUserAchievementsShowcase::empty(&user_id));
            pending.load_state = if pending.achievements.is_empty() {
                RiseUserAchievementsLoadState::Loading
            } else {
                RiseUserAchievementsLoadState::Loaded
            };
            pending.error = None;
            self.upsert_showcase(s, pending);

            let task_user_id = user_id.clone();
            let handle = RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = weak.upgrade() {
                    this.perform_showcase_refresh(task_user_id, generation).await;
                }
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.showcase_in_flight.insert(
                user_id.clone(),
                RiseUserAchievementsInFlight {
                    generation,
                    abort,
                    done: done.clone(),
                },
            );
            RiseUserAchievementsLoadStep::Start { generation, done }
        });

        let (generation, done) = match step {
            RiseUserAchievementsLoadStep::Skip => return,
            RiseUserAchievementsLoadStep::Join(done) => {
                done.await;
                return;
            }
            RiseUserAchievementsLoadStep::Start { generation, done } => (generation, done),
        };

        done.await;
        self.state.with(|s| {
            if s.showcase_generations.get(&user_id) == Some(&generation) {
                s.showcase_in_flight.remove(&user_id);
            }
        });
    }

    async fn perform_showcase_refresh(&self, user_id: String, generation: u64) {
        let result = self
            .rpc
            .get_user_achievements(rise_canon::rise_request::user::GetUserAchievements::new(user_id.clone()))
            .await;
        self.state.with(|s| {
            if s.is_shutdown || s.showcase_generations.get(&user_id) != Some(&generation) {
                return;
            }
            match result {
                Ok(response) => {
                    let achievements = Self::normalized(
                        response
                            .achievements
                            .iter()
                            .map(RiseAchievement::from_canon)
                            .collect(),
                    );
                    s.showcase_refreshed_at_ms
                        .insert(user_id.clone(), Self::now_milliseconds());
                    let loaded = i64::try_from(achievements.len()).unwrap_or(i64::MAX);
                    let entry = RiseUserAchievementsShowcase {
                        user_id: user_id.clone(),
                        total: if response.visible { response.total.max(loaded) } else { 0 },
                        achievements: if response.visible { achievements } else { Vec::new() },
                        is_visible: response.visible,
                        load_state: RiseUserAchievementsLoadState::Loaded,
                        error: None,
                    };
                    self.upsert_showcase(s, entry);
                }
                Err(error) => {
                    let message = CanonErrors::load_failure_text(&error);
                    self.fail_showcase(s, &user_id, &message);
                }
            }
        });
    }

    fn fail_showcase(&self, s: &mut RiseUserAchievementsRepositoryState, user_id: &str, message: &str) {
        let mut entry = s
            .snapshot
            .showcase(user_id)
            .cloned()
            .unwrap_or_else(|| RiseUserAchievementsShowcase::empty(user_id));
        entry.load_state = if entry.achievements.is_empty() {
            RiseUserAchievementsLoadState::Failed
        } else {
            RiseUserAchievementsLoadState::Loaded
        };
        entry.error = Self::message(message);
        self.upsert_showcase(s, entry);
    }

    fn upsert_showcase(&self, s: &mut RiseUserAchievementsRepositoryState, entry: RiseUserAchievementsShowcase) {
        let mut next = s.snapshot.showcases.clone();
        match next.iter().position(|existing| existing.user_id == entry.user_id) {
            Some(index) => next[index] = entry,
            None => next.insert(0, entry),
        }

        let maximum = usize::try_from(Self::maximum_showcases()).unwrap_or(0);
        if next.len() > maximum {
            let dropped: Vec<String> = next.split_off(maximum).into_iter().map(|showcase| showcase.user_id).collect();
            for user_id in dropped {
                if let Some(in_flight) = s.showcase_in_flight.remove(&user_id) {
                    in_flight.abort.abort();
                }
                s.showcase_generations.remove(&user_id);
                s.showcase_refreshed_at_ms.remove(&user_id);
            }
        }

        self.publish(s, |snapshot| snapshot.showcases = next);
    }

    pub async fn shutdown(&self) {
        let stopped_now = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.is_shutdown = true;
            if let Some(in_flight) = s.in_flight_refresh.take() {
                in_flight.abort.abort();
            }
            for (_, in_flight) in s.showcase_in_flight.drain() {
                in_flight.abort.abort();
            }
            s.showcase_generations.clear();
            s.showcase_refreshed_at_ms.clear();
            true
        });
        if stopped_now {
            self.resource.shutdown().await;
        }
    }

    async fn persist_current(&self) {
        let payload = self.state.with(|s| RiseUserAchievementsResourcePayload {
            achievements: s.snapshot.achievements.clone(),
            total: s.snapshot.total,
            hidden_count: s.snapshot.hidden_count,
            catalog_total: s.snapshot.catalog_total,
            achievements_rule: s.snapshot.achievements_rule.clone(),
        });
        let _ = self.resource.store(&payload).await;
    }

    /// mutates, derives and publishes in one step so state and presentation cant disagree
    fn publish(
        &self,
        state: &mut RiseUserAchievementsRepositoryState,
        mutate: impl FnOnce(&mut RiseUserAchievementsSnapshot),
    ) {
        let mut next = state.snapshot.clone();
        mutate(&mut next);
        let next = Self::with_derived_sections(next);
        if next == state.snapshot {
            return;
        }
        state.snapshot = next.clone();

        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        hop.post(move |cx| {
            RiseUserAchievementsPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(next, &account_id, &account_epoch, session_id, cx);
            });
        });
    }

    fn with_derived_sections(snapshot: RiseUserAchievementsSnapshot) -> RiseUserAchievementsSnapshot {
        let mut next = snapshot;
        next.visible_achievements = next
            .achievements
            .iter()
            .filter(|achievement| !achievement.hidden)
            .cloned()
            .collect();
        next.hidden_achievements = next
            .achievements
            .iter()
            .filter(|achievement| achievement.hidden)
            .cloned()
            .collect();
        next
    }

    fn replacing(achievement: RiseAchievement, achievements: &[RiseAchievement]) -> Vec<RiseAchievement> {
        let mut next = achievements.to_vec();
        if let Some(index) = next.iter().position(|existing| existing.id == achievement.id) {
            next[index] = achievement;
        }
        next
    }

    /// dedups by stable id,
    /// bounds the list and drops rows without an id. keeps server order,
    /// newest first
    fn normalized(achievements: Vec<RiseAchievement>) -> Vec<RiseAchievement> {
        let maximum = usize::try_from(Self::maximum_achievements()).unwrap_or(0);
        let mut seen = HashSet::new();
        let mut result = Vec::with_capacity(achievements.len().min(maximum));
        for achievement in achievements {
            if result.len() == maximum {
                break;
            }
            if achievement.id <= 0 || !seen.insert(achievement.id) {
                continue;
            }
            result.push(achievement);
        }
        result
    }

    /// empty gateway error isnt an error, proto3 has no absent string so success carries an empty error
    fn message(raw: &str) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() {
            None
        } else {
            Some(character_prefix(trimmed, 256).to_owned())
        }
    }

    fn normalized_ids(ids: &[i64]) -> Vec<i64> {
        let maximum = usize::try_from(Self::maximum_achievements()).unwrap_or(0);
        let mut seen = HashSet::new();
        let mut result = Vec::with_capacity(ids.len().min(maximum));
        for &id in ids {
            if result.len() == maximum {
                break;
            }
            if id <= 0 || !seen.insert(id) {
                continue;
            }
            result.push(id);
        }
        result
    }

    fn stopped_snapshot(&self) -> Option<RiseUserAchievementsSnapshot> {
        self.state.with(|s| s.is_shutdown.then(|| s.snapshot.clone()))
    }

    fn is_current_request(&self, generation: u64) -> bool {
        self.state
            .with(|s| !s.is_shutdown && generation == s.request_generation)
    }

    fn is_current_mutation(&self, generation: u64) -> bool {
        self.state
            .with(|s| !s.is_shutdown && generation == s.mutation_generation)
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
