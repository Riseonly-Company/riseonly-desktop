use std::collections::HashSet;
use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::AsyncApp;
use rise_canon::RiseTimingCanon;
use rise_canon::rise_view::user::ThemeSettings;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::THEME_PENDING_CHOICE_PREFIX;
use rise_theme::{RiseThemeAppearance, RiseThemePlatform, RiseThemePreferences};
use tokio::task::JoinHandle;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::user::engine::core::rise_user_debug_trace::RiseUserDebugTrace;
use crate::modules::user::engine::core::rise_user_rpc::{RiseUserEndpoints, RiseUserRPC};
use crate::modules::user::engine::core::rise_user_theme_settings_resource_repository::RiseUserThemeSettingsResourceRepository;
use crate::modules::user::engine::core::rise_user_theme_wire_models::ThemeSettingsPreferences;
use crate::modules::user::engine::rise_user_domain::RiseUserDomainRegistry;
use crate::modules::user::engine::rise_user_engine_models::{
    RiseUserSettingsError, RiseUserThemeMutationBuffer, RiseUserThemeMutationFailurePolicy,
    RiseUserThemeMutationIntent, RiseUserThemeSettingsCacheSource, RiseUserThemeSettingsRepositorySnapshot,
};
use crate::modules::user::engine::rise_user_presentation::RiseUserPresentation;

/// mutable state of the repository, one closure is one synchronous stretch
struct RiseUserSettingsRepositoryState {
    committed_preferences: RiseThemePreferences,
    preferences: RiseThemePreferences,
    mutation_buffer: RiseUserThemeMutationBuffer,
    unsynced_platforms: HashSet<RiseThemePlatform>,
    suppress_resend_after_failure: bool,
    revision: i64,
    cache_source: RiseUserThemeSettingsCacheSource,
    has_value: bool,
    is_refreshing: bool,
    error_code: Option<String>,
    state_generation: u64,
    refresh_generation: u64,
    mutation_rpc_generation: u64,
    last_refresh_at_milliseconds: Option<i64>,
    refresh_after_mutations: bool,
    refresh_task: Option<JoinHandle<()>>,
    refresh_task_id: Option<Uuid>,
    mutation_task: Option<JoinHandle<()>>,
    mutation_task_id: Option<Uuid>,
    is_shutdown: bool,
}

struct RiseUserSettingsRepositoryOptimisticWrite {
    intent: RiseUserThemeMutationIntent,
    generation: u64,
    committed_preferences: RiseThemePreferences,
    unsynced_platforms: HashSet<RiseThemePlatform>,
}

/// what a settled mutation hands to the disk write after it
struct RiseUserSettingsRepositorySettledWrite {
    generation: u64,
    committed_preferences: RiseThemePreferences,
    unsynced_platforms: HashSet<RiseThemePlatform>,
}

pub struct RiseUserSettingsRepository {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,

    rpc: RiseUserRPC,
    resource: RiseUserThemeSettingsResourceRepository,
    refresh_interval_milliseconds: i64,
    this: Weak<RiseUserSettingsRepository>,
    state: RiseIsolated<RiseUserSettingsRepositoryState>,
}

impl RiseUserSettingsRepository {
    const FRESHNESS: &'static str = "user_settings";

    /// new repository with the canon freshness window
    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Arc<Self> {
        Self::with_refresh_interval_milliseconds(
            engine,
            account_id,
            account_epoch,
            session_id,
            RiseTimingCanon::freshness_ms(Self::FRESHNESS),
        )
    }

    pub fn with_refresh_interval_milliseconds(
        engine: Arc<RiseEngine>,
        account_id: String,
        account_epoch: String,
        session_id: Uuid,
        refresh_interval_milliseconds: i64,
    ) -> Arc<Self> {
        assert!(refresh_interval_milliseconds >= 0);
        let rpc = RiseUserRPC::new(engine.clone());
        let resource = RiseUserThemeSettingsResourceRepository::new(engine, account_id.clone());
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id,
            rpc,
            resource,
            refresh_interval_milliseconds,
            this: this.clone(),
            state: RiseIsolated::new(RiseUserSettingsRepositoryState {
                committed_preferences: RiseThemePreferences::default(),
                preferences: RiseThemePreferences::default(),
                mutation_buffer: RiseUserThemeMutationBuffer::default(),
                unsynced_platforms: HashSet::new(),
                suppress_resend_after_failure: false,
                revision: 0,
                cache_source: RiseUserThemeSettingsCacheSource::Miss,
                has_value: false,
                is_refreshing: false,
                error_code: None,
                state_generation: 0,
                refresh_generation: 0,
                mutation_rpc_generation: 0,
                last_refresh_at_milliseconds: None,
                refresh_after_mutations: false,
                refresh_task: None,
                refresh_task_id: None,
                mutation_task: None,
                mutation_task_id: None,
                is_shutdown: false,
            }),
        })
    }

    pub async fn activate(&self) -> RiseUserThemeSettingsRepositorySnapshot {
        let is_shutdown = self.state.with(|s| s.is_shutdown);
        if is_shutdown || !self.is_current_session(false).await {
            let label = RiseUserDebugTrace::error_label(&anyhow::Error::new(RiseUserSettingsError::StaleAccount));
            return self.state.with(|s| {
                s.error_code = Some(label);
                self.make_snapshot(s)
            });
        }

        match self.resource.cached().await {
            Ok(cached) => {
                RiseUserDebugTrace::record_cache_read(cached.source, &self.resource.debug_resource_key, cached.revision)
                    .await;
                if let Some(cached_preferences) = cached.preferences {
                    let pending = RiseThemePendingChoice::all(&self.account_id);
                    let rollback = self.state.with(|s| {
                        s.committed_preferences = cached_preferences;
                        s.preferences = cached_preferences;
                        s.unsynced_platforms = cached.unsynced_platforms.clone();
                        s.revision = cached.revision;
                        s.cache_source = cached.source;
                        s.has_value = true;
                        for &(platform, appearance) in &pending {
                            if s.committed_preferences.appearance(platform) != appearance {
                                s.committed_preferences = s.committed_preferences.setting(appearance, platform);
                                s.unsynced_platforms.insert(platform);
                            }
                        }
                        s.preferences = s.committed_preferences;
                        (cached.requires_optimistic_rollback || !pending.is_empty())
                            .then(|| (s.committed_preferences, s.unsynced_platforms.clone()))
                    });
                    if let Some((committed_preferences, unsynced_platforms)) = rollback {
                        match self.resource.store(committed_preferences, None, unsynced_platforms).await {
                            Ok(revision) => {
                                self.state.with(|s| {
                                    s.revision = revision;
                                    s.cache_source = RiseUserThemeSettingsCacheSource::PersistentStore;
                                });
                                for &(platform, appearance) in &pending {
                                    RiseThemePendingChoice::clear(appearance, platform, &self.account_id);
                                }
                            }
                            Err(error) => {
                                let label = RiseUserDebugTrace::error_label(&error);
                                self.state.with(|s| s.error_code = Some(label));
                            }
                        }
                    }
                }
            }
            Err(_) => self.state.with(|s| {
                s.cache_source = RiseUserThemeSettingsCacheSource::Miss;
                s.has_value = false;
                s.error_code = None;
            }),
        }
        self.state.with(|s| self.make_snapshot(s))
    }

    pub async fn load(&self, force: bool) {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_session(true).await {
            return;
        }
        let started = self.state.with(|s| {
            if s.mutation_buffer.is_mutating() {
                s.refresh_after_mutations = true;
                return false;
            }
            if s.refresh_task.is_some() {
                return false;
            }
            if !force
                && let Some(last_refresh_at_milliseconds) = s.last_refresh_at_milliseconds
                && Self::now_milliseconds() - last_refresh_at_milliseconds < self.refresh_interval_milliseconds
            {
                return false;
            }

            s.refresh_generation = s.refresh_generation.wrapping_add(1);
            let generation = s.refresh_generation;
            let task_id = Uuid::new_v4();
            s.is_refreshing = true;
            s.error_code = None;
            s.state_generation = s.state_generation.wrapping_add(1);
            s.refresh_task_id = Some(task_id);
            let this = self.this.clone();
            s.refresh_task = Some(RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = this.upgrade() {
                    this.perform_refresh(task_id, generation).await;
                }
            }));
            true
        });
        if started {
            self.publish_snapshot().await;
        }
    }

    pub async fn set_appearance(
        &self,
        appearance: Option<RiseThemeAppearance>,
        platform: RiseThemePlatform,
    ) -> anyhow::Result<()> {
        if self.state.with(|s| s.is_shutdown) {
            return Err(RiseUserSettingsError::MissingActiveSession.into());
        }
        if !self.is_current_session(true).await {
            return Err(RiseUserSettingsError::StaleAccount.into());
        }
        let write = self.state.with(|s| {
            if s.preferences.appearance(platform) == appearance
                && !s.mutation_buffer.mutating_platforms().contains(&platform)
            {
                return None;
            }
            let intent = s.mutation_buffer.enqueue(platform, appearance);
            s.error_code = None;
            s.has_value = true;
            s.preferences = s.mutation_buffer.applying_optimistic(&s.committed_preferences);
            s.state_generation = s.state_generation.wrapping_add(1);
            let optimistic_preferences = s.preferences;
            let (committed_preferences, unsynced_platforms) =
                Self::store_keeping_pending_choices(s, &optimistic_preferences);
            Some(RiseUserSettingsRepositoryOptimisticWrite {
                intent,
                generation: s.state_generation,
                committed_preferences,
                unsynced_platforms,
            })
        });
        let Some(write) = write else {
            RiseThemePendingChoice::clear(appearance, platform, &self.account_id);
            return Ok(());
        };

        let stored = self.resource.store(write.committed_preferences, None, write.unsynced_platforms).await;
        match stored {
            Ok(stored_revision) => {
                RiseThemePendingChoice::clear(appearance, platform, &self.account_id);
                if self.state.with(|s| s.is_shutdown) || !self.is_current_session(true).await {
                    return Ok(());
                }
                let publish = self.state.with(|s| {
                    if !s.mutation_buffer.mark_ready(&write.intent) {
                        return None;
                    }
                    if s.state_generation == write.generation {
                        s.revision = stored_revision;
                        s.cache_source = RiseUserThemeSettingsCacheSource::PersistentStore;
                        return Some(true);
                    }
                    Some(false)
                });
                let Some(publish) = publish else {
                    return Ok(());
                };
                if publish {
                    self.publish_snapshot().await;
                }
                self.ensure_mutation_task();
                Ok(())
            }
            Err(error) if error.is::<CancellationError>() => Ok(()),
            Err(error) => {
                if self.state.with(|s| s.is_shutdown) || !self.is_current_session(true).await {
                    return Ok(());
                }
                let label = RiseUserDebugTrace::error_label(&error);
                let publish = self.state.with(|s| {
                    if !s.mutation_buffer.mark_ready(&write.intent) {
                        return None;
                    }
                    s.error_code = Some(label);
                    Some(s.state_generation == write.generation)
                });
                let Some(publish) = publish else {
                    return Ok(());
                };
                if publish {
                    self.publish_snapshot().await;
                }
                self.ensure_mutation_task();
                Ok(())
            }
        }
    }

    pub async fn shutdown(&self) {
        let tasks = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.is_shutdown = true;
            s.is_refreshing = false;
            s.refresh_after_mutations = false;
            let tasks: Vec<JoinHandle<()>> = [s.refresh_task.take(), s.mutation_task.take()].into_iter().flatten().collect();
            s.refresh_task_id = None;
            s.mutation_task_id = None;
            Some(tasks)
        });
        let Some(tasks) = tasks else {
            return;
        };
        for task in &tasks {
            task.abort();
        }
        self.resource.shutdown().await;
        for task in tasks {
            let _ = task.await;
        }
    }

    async fn perform_refresh(&self, task_id: Uuid, generation: u64) {
        let span = RiseUserDebugTrace::begin_network(RiseUserEndpoints::get_theme_settings().clone(), generation).await;
        let outcome: anyhow::Result<()> = async {
            let response = self.rpc.get_theme_settings().await?;
            let server_preferences = Self::preferences(response.settings.as_ref())?;
            if !self.owns_refresh(task_id) || !self.is_current_session(true).await {
                return Err(CancellationError.into());
            }
            let deferred = self.state.with(|s| {
                if !s.mutation_buffer.is_mutating() {
                    return false;
                }
                Self::defer_refresh_to_mutations(s);
                true
            });
            if deferred {
                let published = self.publish_snapshot().await;
                RiseUserDebugTrace::finish_network(&span, false, published, None).await;
                return Ok(());
            }

            let (request_state_generation, merged_preferences, unsynced_platforms) = self.state.with(|s| {
                (
                    s.state_generation,
                    Self::applying_unsynced(&s.committed_preferences, &s.unsynced_platforms, server_preferences),
                    s.unsynced_platforms.clone(),
                )
            });
            let stored_revision = self.resource.store(merged_preferences, None, unsynced_platforms).await?;
            if !self.owns_refresh(task_id) || !self.is_current_session(true).await {
                return Err(CancellationError.into());
            }
            let superseded = self.state.with(|s| {
                if s.state_generation != request_state_generation || s.mutation_buffer.is_mutating() {
                    Self::defer_refresh_to_mutations(s);
                    return true;
                }
                s.committed_preferences = merged_preferences;
                s.preferences = merged_preferences;
                s.revision = stored_revision;
                s.cache_source = RiseUserThemeSettingsCacheSource::PersistentStore;
                s.has_value = true;
                s.error_code = None;
                s.is_refreshing = false;
                s.last_refresh_at_milliseconds = Some(Self::now_milliseconds());
                s.refresh_task = None;
                s.refresh_task_id = None;
                s.state_generation = s.state_generation.wrapping_add(1);
                false
            });
            let published = self.publish_snapshot().await;
            RiseUserDebugTrace::finish_network(&span, true, published, None).await;
            if !superseded {
                self.resend_unsynced_if_needed();
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;

        let Err(error) = outcome else {
            return;
        };
        let owns_session = self.is_current_session(true).await;
        let should_publish = self.state.with(|s| {
            let owns_task = s.refresh_task_id == Some(task_id);
            let should_publish = !s.is_shutdown && owns_task && owns_session;
            if owns_task {
                s.is_refreshing = false;
                s.refresh_task = None;
                s.refresh_task_id = None;
            }
            should_publish
        });
        if should_publish && !error.is::<CancellationError>() {
            let label = RiseUserDebugTrace::error_label(&error);
            self.state.with(|s| {
                s.error_code = Some(label);
                s.state_generation = s.state_generation.wrapping_add(1);
            });
        }
        let published = if should_publish { self.publish_snapshot().await } else { false };
        RiseUserDebugTrace::finish_network(&span, false, published, Some(&error)).await;
    }

    /// a refresh interrupted by a mutation hands its turn to the drain,
    /// which refreshes once the buffer is empty
    fn defer_refresh_to_mutations(s: &mut RiseUserSettingsRepositoryState) {
        s.refresh_after_mutations = true;
        s.is_refreshing = false;
        s.refresh_task = None;
        s.refresh_task_id = None;
        s.state_generation = s.state_generation.wrapping_add(1);
    }

    fn owns_refresh(&self, task_id: Uuid) -> bool {
        self.state.with(|s| !s.is_shutdown && s.refresh_task_id == Some(task_id))
    }

    fn applying_unsynced(
        local: &RiseThemePreferences,
        platforms: &HashSet<RiseThemePlatform>,
        to: RiseThemePreferences,
    ) -> RiseThemePreferences {
        let mut result = to;
        for platform in platforms {
            result = result.setting(local.appearance(*platform), *platform);
        }
        result
    }

    /// unacked choice is a committed unsynced value, not an overlay
    /// activation rolls overlays back, so a kill reopens the old theme
    fn store_keeping_pending_choices(
        s: &RiseUserSettingsRepositoryState,
        visible_preferences: &RiseThemePreferences,
    ) -> (RiseThemePreferences, HashSet<RiseThemePlatform>) {
        let pending_platforms = s.mutation_buffer.mutating_platforms();
        if pending_platforms.is_empty() {
            return (s.committed_preferences, s.unsynced_platforms.clone());
        }
        (
            Self::applying_unsynced(visible_preferences, &pending_platforms, s.committed_preferences),
            s.unsynced_platforms.union(&pending_platforms).copied().collect(),
        )
    }

    fn resend_unsynced_if_needed(&self) {
        let resend = self.state.with(|s| {
            let suppressed = s.suppress_resend_after_failure;
            s.suppress_resend_after_failure = false;
            if s.is_shutdown || suppressed || s.unsynced_platforms.is_empty() {
                return false;
            }
            let mut platforms: Vec<RiseThemePlatform> = s.unsynced_platforms.iter().copied().collect();
            platforms.sort_by_key(|platform| Self::platform_raw_value(*platform));
            let mutating = s.mutation_buffer.mutating_platforms();
            for platform in platforms.into_iter().filter(|platform| !mutating.contains(platform)) {
                let appearance = s.committed_preferences.appearance(platform);
                let intent = s.mutation_buffer.enqueue(platform, appearance);
                s.mutation_buffer.mark_ready(&intent);
            }
            true
        });
        if resend {
            self.ensure_mutation_task();
        }
    }

    fn platform_raw_value(platform: RiseThemePlatform) -> &'static str {
        match platform {
            RiseThemePlatform::Mobile => "mobile",
            RiseThemePlatform::Desktop => "desktop",
            RiseThemePlatform::Web => "web",
        }
    }

    fn ensure_mutation_task(&self) {
        self.state.with(|s| {
            if s.is_shutdown
                || s.mutation_task.is_some()
                || s.mutation_buffer.in_flight().is_some()
                || !s.mutation_buffer.pending().values().any(|intent| intent.is_ready)
            {
                return;
            }
            let task_id = Uuid::new_v4();
            s.mutation_task_id = Some(task_id);
            let this = self.this.clone();
            s.mutation_task = Some(RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = this.upgrade() {
                    this.drain_mutations(task_id).await;
                }
            }));
        });
    }

    async fn drain_mutations(&self, task_id: Uuid) {
        loop {
            if self.state.with(|s| s.is_shutdown || s.mutation_task_id != Some(task_id)) {
                break;
            }
            if !self.is_current_session(true).await {
                break;
            }
            let next = self.state.with(|s| {
                let intent = s.mutation_buffer.begin_next()?;
                s.mutation_rpc_generation = s.mutation_rpc_generation.wrapping_add(1);
                Some((intent, s.mutation_rpc_generation))
            });
            let Some((intent, generation)) = next else {
                break;
            };
            let span =
                RiseUserDebugTrace::begin_network(RiseUserEndpoints::update_theme_settings().clone(), generation).await;
            let outcome: anyhow::Result<RiseThemePreferences> = async {
                let response = self.rpc.update_theme_settings(intent.platform, intent.appearance).await?;
                let server_preferences = Self::preferences(response.settings.as_ref())?;
                let owns_task = self.state.with(|s| !s.is_shutdown && s.mutation_task_id == Some(task_id));
                if !owns_task || !self.is_current_session(true).await {
                    return Err(CancellationError.into());
                }
                Ok::<RiseThemePreferences, anyhow::Error>(server_preferences)
            }
            .await;
            match outcome {
                Ok(server_preferences) => {
                    let (cache_committed, published) = self.apply_mutation_success(intent, server_preferences).await;
                    RiseUserDebugTrace::finish_network(&span, cache_committed, published, None).await;
                }
                Err(error) => {
                    let (cache_committed, published) = self.apply_mutation_failure(intent, &error).await;
                    RiseUserDebugTrace::finish_network(&span, cache_committed, published, Some(&error)).await;
                    if error.is::<CancellationError>() {
                        break;
                    }
                }
            }
        }

        let refresh = self.state.with(|s| {
            if s.mutation_task_id != Some(task_id) {
                return None;
            }
            s.mutation_task = None;
            s.mutation_task_id = None;
            if !s.mutation_buffer.is_mutating() && s.refresh_after_mutations {
                s.refresh_after_mutations = false;
                s.suppress_resend_after_failure = !s.unsynced_platforms.is_empty();
                return Some(true);
            }
            Some(false)
        });
        match refresh {
            Some(true) => self.load(true).await,
            Some(false) => self.ensure_mutation_task(),
            None => {}
        }
    }

    async fn apply_mutation_success(
        &self,
        intent: RiseUserThemeMutationIntent,
        server_preferences: RiseThemePreferences,
    ) -> (bool, bool) {
        let write = self.state.with(|s| {
            if !s.mutation_buffer.finish(&intent) {
                return None;
            }
            s.unsynced_platforms.remove(&intent.platform);
            s.committed_preferences =
                Self::applying_unsynced(&s.committed_preferences, &s.unsynced_platforms, server_preferences);
            s.mutation_buffer.discard_intents_satisfied(&s.committed_preferences);
            s.preferences = s.mutation_buffer.applying_optimistic(&s.committed_preferences);
            s.has_value = true;
            s.error_code = None;
            s.state_generation = s.state_generation.wrapping_add(1);
            let value = s.preferences;
            let (committed_preferences, unsynced_platforms) = Self::store_keeping_pending_choices(s, &value);
            Some(RiseUserSettingsRepositorySettledWrite {
                generation: s.state_generation,
                committed_preferences,
                unsynced_platforms,
            })
        });
        let Some(write) = write else {
            return (false, false);
        };
        self.store_settled(write, true).await
    }

    async fn apply_mutation_failure(&self, intent: RiseUserThemeMutationIntent, error: &anyhow::Error) -> (bool, bool) {
        let finished = self.state.with(|s| s.mutation_buffer.finish(&intent) && !s.is_shutdown);
        if !finished || !self.is_current_session(true).await {
            return (false, false);
        }
        let is_cancellation = error.is::<CancellationError>();
        let label = (!is_cancellation).then(|| RiseUserDebugTrace::error_label(error));
        let write = self.state.with(|s| {
            if RiseUserThemeMutationFailurePolicy::requires_authoritative_refresh(error) {
                s.refresh_after_mutations = true;
            }
            if let Some(label) = label {
                s.committed_preferences = s.committed_preferences.setting(intent.appearance, intent.platform);
                s.unsynced_platforms.insert(intent.platform);
                s.error_code = Some(label);
            }
            s.preferences = s.mutation_buffer.applying_optimistic(&s.committed_preferences);
            s.state_generation = s.state_generation.wrapping_add(1);
            let value = s.preferences;
            let (committed_preferences, unsynced_platforms) = Self::store_keeping_pending_choices(s, &value);
            RiseUserSettingsRepositorySettledWrite {
                generation: s.state_generation,
                committed_preferences,
                unsynced_platforms,
            }
        });
        self.store_settled(write, false).await
    }

    /// disk write and publish both settle paths end with, only success restamps freshness
    async fn store_settled(&self, write: RiseUserSettingsRepositorySettledWrite, stamps_refresh: bool) -> (bool, bool) {
        let stored = self.resource.store(write.committed_preferences, None, write.unsynced_platforms).await;
        match stored {
            Ok(stored_revision) => {
                let current = self.state.with(|s| !s.is_shutdown && s.state_generation == write.generation);
                if !current || !self.is_current_session(true).await {
                    return (true, false);
                }
                self.state.with(|s| {
                    s.revision = stored_revision;
                    s.cache_source = RiseUserThemeSettingsCacheSource::PersistentStore;
                    if stamps_refresh {
                        s.last_refresh_at_milliseconds = Some(Self::now_milliseconds());
                    }
                });
                (true, self.publish_snapshot().await)
            }
            Err(error) => {
                let label = RiseUserDebugTrace::error_label(&error);
                let current = self.state.with(|s| {
                    if s.state_generation != write.generation {
                        return false;
                    }
                    s.error_code = Some(label);
                    true
                });
                if !current {
                    return (false, false);
                }
                (false, self.publish_snapshot().await)
            }
        }
    }

    async fn publish_snapshot(&self) -> bool {
        let snapshot = self.state.with(|s| self.make_snapshot(s));
        let Some(hop) = RiseEngineMainHop::installed() else {
            return false;
        };
        hop.run(move |cx: AsyncApp| {
            async move {
                Ok(cx.update(|cx| {
                    RiseUserPresentation::shared(cx).update(cx, |presentation, cx| presentation.publish(&snapshot, cx))
                }))
            }
            .boxed_local()
        })
        .await
        .unwrap_or(false)
    }

    fn make_snapshot(&self, s: &RiseUserSettingsRepositoryState) -> RiseUserThemeSettingsRepositorySnapshot {
        RiseUserThemeSettingsRepositorySnapshot {
            account_id: self.account_id.clone(),
            account_epoch: self.account_epoch.clone(),
            session_id: self.session_id,
            revision: s.revision,
            preferences: s.preferences,
            has_value: s.has_value,
            cache_source: s.cache_source,
            is_refreshing: s.is_refreshing,
            mutating_platforms: s.mutation_buffer.mutating_platforms(),
            error_code: s.error_code.clone(),
        }
    }

    async fn is_current_session(&self, require_registry: bool) -> bool {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return false;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        hop.run(move |cx: AsyncApp| {
            async move {
                Ok(cx.update(|cx| {
                    if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch)) {
                        return false;
                    }
                    if !require_registry {
                        return true;
                    }
                    RiseUserDomainRegistry::shared(cx).read(cx).matches(&account_id, &account_epoch, session_id)
                }))
            }
            .boxed_local()
        })
        .await
        .unwrap_or(false)
    }

    fn preferences(settings: Option<&ThemeSettings>) -> anyhow::Result<RiseThemePreferences> {
        let Some(settings) = settings else {
            return Err(RiseUserSettingsError::InvalidResponse.into());
        };
        Ok(settings.preferences())
    }

    pub async fn delete_my_account(&self) -> anyhow::Result<i64> {
        if self.state.with(|s| s.is_shutdown) {
            return Err(RiseUserSettingsError::MissingActiveSession.into());
        }
        let response = self.rpc.delete_my_account().await?;
        if response.delete_after <= 0 {
            return Err(RiseUserSettingsError::InvalidResponse.into());
        }
        Ok(response.delete_after)
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}

/// theme choice written right at the tap until the repository store holds it, survives a kill
pub enum RiseThemePendingChoice {}

impl RiseThemePendingChoice {
    const PREFIX: &'static str = THEME_PENDING_CHOICE_PREFIX;
    const SYSTEM_VALUE: &'static str = "system";

    fn key(platform: RiseThemePlatform, account_id: &str) -> String {
        format!("{}.{}.{}", Self::PREFIX, account_id, RiseUserSettingsRepository::platform_raw_value(platform))
    }

    fn stored_value(appearance: Option<RiseThemeAppearance>) -> &'static str {
        appearance.map_or(Self::SYSTEM_VALUE, RiseThemeAppearance::raw_value)
    }

    pub fn remember(appearance: Option<RiseThemeAppearance>, platform: RiseThemePlatform, account_id: &str) {
        UserDefaults::standard().set(&Self::key(platform, account_id), Self::stored_value(appearance));
    }

    /// clears only the same choice, a newer tap stays pending until its own store
    pub fn clear(appearance: Option<RiseThemeAppearance>, platform: RiseThemePlatform, account_id: &str) {
        let defaults = UserDefaults::standard();
        let store_key = Self::key(platform, account_id);
        if defaults.string(&store_key).as_deref() != Some(Self::stored_value(appearance)) {
            return;
        }
        defaults.remove_object(&store_key);
    }

    pub fn all(account_id: &str) -> Vec<(RiseThemePlatform, Option<RiseThemeAppearance>)> {
        let defaults = UserDefaults::standard();
        RiseThemePlatform::ALL_CASES
            .into_iter()
            .filter_map(|platform| {
                let raw = defaults.string(&Self::key(platform, account_id))?;
                if raw == Self::SYSTEM_VALUE {
                    return Some((platform, None));
                }
                let appearance = RiseThemeAppearance::from_raw_value(&raw)?;
                Some((platform, Some(appearance)))
            })
            .collect()
    }
}
