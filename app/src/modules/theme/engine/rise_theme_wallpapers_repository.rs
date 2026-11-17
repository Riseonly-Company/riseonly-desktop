use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AsyncApp};
use rise_canon::RiseIdentityCanon;
use rise_canon::{RiseLimits, RiseTimingCanon, rise_request};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine};
use rise_i18n::tr;
use rise_limits::service_limit::MAX_WALLPAPERS_PER_USER_PREMIUM;
use tokio::task::AbortHandle;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{
    ChatWallpaperLook, ChatWallpaperStore,
};
use crate::modules::theme::engine::core::rise_theme_rpc::RiseThemeRPC;
use crate::modules::theme::engine::core::rise_theme_wallpaper_wire_models::{
    RiseChatWallpaper, RiseWallpaperPreferences,
};
use crate::modules::theme::engine::core::rise_theme_wallpapers_resource_repository::{
    RiseStoredThemeWallpapers, RiseThemeWallpapersCacheSource, RiseThemeWallpapersResourceRepository,
};
use crate::modules::theme::engine::rise_theme_wallpaper_upload::{
    RiseChatWallpaperDraft, RiseChatWallpaperUpload,
};
use crate::modules::theme::engine::rise_theme_wallpapers_presentation::{
    RiseThemeWallpapersLoadState, RiseThemeWallpapersPresentation, RiseThemeWallpapersSnapshot,
};

const OPTIMISTIC_TEMPORARY_PREFIX: &str = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;

#[derive(Debug, thiserror::Error)]
pub enum RiseThemeWallpapersError {
    #[error("missingActiveSession")]
    MissingActiveSession,
    #[error("staleAccount")]
    StaleAccount,
    #[error("{0}")]
    Rejected(String),
    #[error("limitReached")]
    LimitReached,
}

impl RiseThemeWallpapersError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            Self::MissingActiveSession | Self::StaleAccount => None,
            Self::Rejected(message) => (!message.is_empty()).then(|| message.clone()),
            Self::LimitReached => Some(tr("chat_wallpaper_limit_reached")),
        }
    }
}

static REFRESH_INTERVAL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::freshness_ms("user_settings"));

static MAXIMUM_WALLPAPERS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("max_wallpapers_per_user"));

const CATALOGUE_FILTER: &str = "custom";

/// the running refresh, done resolves when it ends without cancelling it and abort cancels
struct RiseThemeWallpapersInFlight {
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

struct RiseThemeWallpapersRepositoryState {
    snapshot: RiseThemeWallpapersSnapshot,
    is_shutdown: bool,
    generation_clock: u64,
    generation: u64,
    selection_generation: u64,
    preferences_generation: u64,
    refreshed_at_ms: i64,
    in_flight: Option<RiseThemeWallpapersInFlight>,
    pending_drafts: HashMap<String, RiseChatWallpaperDraft>,
}

enum RiseThemeWallpapersLoadStep {
    Skip,
    Join(Shared<BoxFuture<'static, ()>>),
    Start(u64, Shared<BoxFuture<'static, ()>>),
}

/// owns the accounts chat wallpapers, uploads and pinned one
/// and how others wallpapers on shared chats are treated
/// cache first, failure keeps it
pub struct RiseThemeWallpapersRepository {
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    resource: RiseThemeWallpapersResourceRepository,
    rpc: RiseThemeRPC,
    state: RiseIsolated<RiseThemeWallpapersRepositoryState>,
}

impl RiseThemeWallpapersRepository {
    /// subscriber edge of the same ceiling,
    /// the catalogue asks for this many on every tier so one page covers an account
    /// that kept uploads after premium ended
    pub fn maximum_wallpapers_premium() -> i64 {
        rise_limits::signed(MAX_WALLPAPERS_PER_USER_PREMIUM)
    }

    pub fn maximum_wallpapers_free() -> i64 {
        *MAXIMUM_WALLPAPERS
    }

    pub fn maximum_wallpapers(is_premium: bool) -> i64 {
        if is_premium { Self::maximum_wallpapers_premium() } else { Self::maximum_wallpapers_free() }
    }

    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Self {
        let rpc = RiseThemeRPC::new(engine.clone());
        let resource = RiseThemeWallpapersResourceRepository::new(engine, account_id.clone());
        Self {
            account_id,
            account_epoch,
            session_id,
            resource,
            rpc,
            state: RiseIsolated::new(RiseThemeWallpapersRepositoryState {
                snapshot: RiseThemeWallpapersSnapshot::empty(),
                is_shutdown: false,
                generation_clock: 0,
                generation: 0,
                selection_generation: 0,
                preferences_generation: 0,
                refreshed_at_ms: 0,
                in_flight: None,
                pending_drafts: HashMap::new(),
            }),
        }
    }

    pub fn current(&self) -> RiseThemeWallpapersSnapshot {
        self.state.with(|s| s.snapshot.clone())
    }

    /// cache only seed so the screen opens on known rows before the refresh extends them
    pub async fn activate(&self) -> RiseThemeWallpapersSnapshot {
        if self.state.with(|s| s.is_shutdown) {
            return RiseThemeWallpapersSnapshot::empty();
        }
        let Ok(cached) = self.resource.cached().await else {
            return self.current();
        };
        let Some(payload) = cached.payload else {
            return self.current();
        };
        self.state.with(|s| {
            if s.is_shutdown || cached.source == RiseThemeWallpapersCacheSource::Miss {
                return s.snapshot.clone();
            }
            let mut next = s.snapshot.clone();
            next.wallpapers = Self::ordered(payload.wallpapers);
            next.selected_id = payload.default_wallpaper_id;
            next.preferences = payload.preferences.unwrap_or_else(RiseWallpaperPreferences::default_preferences);
            next.load_state = RiseThemeWallpapersLoadState::Loaded;
            s.snapshot = next;
            s.snapshot.clone()
        })
    }

    pub async fn load(self: &Arc<Self>, force: bool) {
        if self.state.with(|s| s.is_shutdown) {
            return;
        }

        let now = Self::now_milliseconds();
        let weak = Arc::downgrade(self);
        let (step, published) = self.state.with(|s| {
            if !force
                && s.snapshot.load_state == RiseThemeWallpapersLoadState::Loaded
                && now - s.refreshed_at_ms < *REFRESH_INTERVAL_MS
            {
                if s.snapshot.error.is_some() {
                    let mut next = s.snapshot.clone();
                    next.error = None;
                    return (RiseThemeWallpapersLoadStep::Skip, Self::publish_in(s, next));
                }
                return (RiseThemeWallpapersLoadStep::Skip, None);
            }
            if let Some(in_flight) = s.in_flight.as_ref() {
                return (RiseThemeWallpapersLoadStep::Join(in_flight.done.clone()), None);
            }

            s.generation_clock = s.generation_clock.wrapping_add(1);
            let generation = s.generation_clock;
            s.generation = generation;

            let mut pending = s.snapshot.clone();
            pending.load_state = if pending.wallpapers.is_empty() {
                RiseThemeWallpapersLoadState::Loading
            } else {
                RiseThemeWallpapersLoadState::Loaded
            };
            pending.is_refreshing = true;
            pending.error = None;
            let published = Self::publish_in(s, pending);

            let selection_at_start = s.selection_generation;
            let preferences_at_start = s.preferences_generation;
            let handle = RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = weak.upgrade() {
                    this.perform_refresh(generation, selection_at_start, preferences_at_start).await;
                }
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.in_flight = Some(RiseThemeWallpapersInFlight { abort, done: done.clone() });
            (RiseThemeWallpapersLoadStep::Start(generation, done), published)
        });
        self.post(published);

        match step {
            RiseThemeWallpapersLoadStep::Skip => {}
            RiseThemeWallpapersLoadStep::Join(done) => done.await,
            RiseThemeWallpapersLoadStep::Start(generation, done) => {
                done.await;
                self.state.with(|s| {
                    if s.generation == generation {
                        s.in_flight = None;
                    }
                });
            }
        }
    }

    async fn perform_refresh(&self, generation: u64, selection_at_start: u64, preferences_at_start: u64) {
        let catalogue_request = rise_request::theme::ListWallpapers {
            filter: Some(CATALOGUE_FILTER.to_owned()),
            limit: Some(Self::maximum_wallpapers_premium()),
            ..Default::default()
        };
        let (catalogue, preferences_reply) = futures::join!(
            self.rpc.list_wallpapers(catalogue_request),
            self.rpc.get_wallpaper_preferences(),
        );
        let response = match catalogue {
            Ok(response) => response,
            Err(error) => {
                let message = CanonErrors::failure(&error)
                    .and_then(|failure| CanonErrors::text_for(&failure))
                    .unwrap_or_default();
                let published = self.state.with(|s| {
                    if s.is_shutdown || generation != s.generation {
                        return None;
                    }
                    Self::finish_with_load_error(s, &message)
                });
                self.post(published);
                return;
            }
        };
        let server_preferences = preferences_reply.ok().map(|reply| RiseWallpaperPreferences::from_canon(&reply));
        let server = Self::ordered(response.items.iter().map(RiseChatWallpaper::from_canon).collect());

        let stored = self.state.with(|s| {
            if s.is_shutdown || generation != s.generation {
                return None;
            }
            s.refreshed_at_ms = Self::now_milliseconds();
            let preferences = if s.preferences_generation == preferences_at_start {
                server_preferences.clone().unwrap_or_else(|| s.snapshot.preferences.clone())
            } else {
                s.snapshot.preferences.clone()
            };
            let default_wallpaper_id = if s.selection_generation == selection_at_start {
                response.default_wallpaper_id.clone()
            } else {
                s.snapshot.selected_id.clone()
            };
            Some(RiseStoredThemeWallpapers::new(server.clone(), default_wallpaper_id, Some(preferences)))
        });
        let Some(stored) = stored else {
            return;
        };
        let _ = self.resource.store(stored).await;

        let published = self.state.with(|s| {
            if s.is_shutdown || generation != s.generation {
                return None;
            }
            let mut next = s.snapshot.clone();
            let mut rows: Vec<RiseChatWallpaper> =
                next.wallpapers.iter().filter(|row| row.is_pending).cloned().collect();
            rows.extend(server);
            next.wallpapers = rows;
            if s.selection_generation == selection_at_start {
                next.selected_id = response.default_wallpaper_id.clone();
            }
            if s.preferences_generation == preferences_at_start
                && let Some(server_preferences) = server_preferences
            {
                next.preferences = server_preferences;
            }
            next.load_state = RiseThemeWallpapersLoadState::Loaded;
            next.is_refreshing = false;
            next.error = None;
            Self::publish_in(s, next)
        });
        self.post(published);
    }

    fn finish_with_load_error(
        s: &mut RiseThemeWallpapersRepositoryState,
        message: &str,
    ) -> Option<RiseThemeWallpapersSnapshot> {
        let mut next = s.snapshot.clone();
        next.is_refreshing = false;
        next.load_state = if next.wallpapers.is_empty() {
            RiseThemeWallpapersLoadState::Failed
        } else {
            RiseThemeWallpapersLoadState::Loaded
        };
        next.error = Some(if message.is_empty() { tr("chat_wallpaper_load_failed") } else { message.to_owned() });
        Self::publish_in(s, next)
    }

    async fn maximum_wallpapers_for_active_account(&self) -> i64 {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return Self::maximum_wallpapers(false);
        };
        let account_id = self.account_id.clone();
        let is_premium = hop
            .run(move |cx: AsyncApp| {
                async move {
                    Ok(cx.update(|cx| {
                        let auth = AuthServicesStore::shared(cx);
                        let auth = auth.read(cx);
                        if auth.effective_user_id_for_wire().as_deref() != Some(account_id.as_str()) {
                            return false;
                        }
                        let Some(profile) = auth.profile() else {
                            return false;
                        };
                        if profile.id != account_id {
                            return false;
                        }
                        profile.more.is_premium
                    }))
                }
                .boxed_local()
            })
            .await
            .unwrap_or(false);
        Self::maximum_wallpapers(is_premium)
    }

    /// takes a picked image with its look, tile shows now and bytes upload later
    /// joins the catalogue but nothing is pinned
    /// returns the new row id, none if upload failed
    pub async fn add(&self, draft: RiseChatWallpaperDraft, look: ChatWallpaperLook) -> Option<String> {
        let ceiling = self.maximum_wallpapers_for_active_account().await;
        let look = look.clamped();
        let temp_id = format!("{OPTIMISTIC_TEMPORARY_PREFIX}{}", Uuid::new_v4().uuid_string());

        enum AddStart {
            Shutdown,
            LimitReached(Option<RiseThemeWallpapersSnapshot>),
            Started(Option<RiseThemeWallpapersSnapshot>),
        }
        let start = self.state.with(|s| {
            if s.is_shutdown {
                return AddStart::Shutdown;
            }
            let live_count = s.snapshot.wallpapers.len() as i64;
            if live_count >= ceiling {
                let mut next = s.snapshot.clone();
                next.mutation_error = RiseThemeWallpapersError::LimitReached.error_description();
                return AddStart::LimitReached(Self::publish_in(s, next));
            }
            s.pending_drafts.insert(temp_id.clone(), draft.clone());
            let mut optimistic = s.snapshot.clone();
            optimistic.wallpapers.insert(
                0,
                RiseChatWallpaper::new(
                    temp_id.clone(),
                    tr("chat_wallpaper_custom_name"),
                    draft.local_url.absolute_string().to_owned(),
                    draft.local_url.absolute_string().to_owned(),
                    false,
                    false,
                    Self::now_milliseconds(),
                    look.dim_percent,
                    look.blur_percent,
                    true,
                ),
            );
            optimistic.mutating_ids.insert(temp_id.clone());
            optimistic.mutation_error = None;
            AddStart::Started(Self::publish_in(s, optimistic))
        });
        match start {
            AddStart::Shutdown => return None,
            AddStart::LimitReached(published) => {
                self.post(published);
                RiseChatWallpaperUpload::discard_staged_file(&draft.local_url);
                return None;
            }
            AddStart::Started(published) => self.post(published),
        }

        match self.create_remote(&draft, look).await {
            Ok(wallpaper) => {
                let published = self.state.with(|s| {
                    s.pending_drafts.remove(&temp_id);
                    let mut next = s.snapshot.clone();
                    next.wallpapers.retain(|row| row.id != temp_id);
                    next.wallpapers.insert(0, wallpaper.clone());
                    next.wallpapers = Self::ordered(next.wallpapers);
                    next.mutating_ids.remove(&temp_id);
                    next.mutation_error = None;
                    Self::publish_in(s, next)
                });
                RiseChatWallpaperUpload::discard_staged_file(&draft.local_url);
                self.post(published);
                self.persist_current().await;
                Some(wallpaper.id)
            }
            Err(error) => {
                let fallback = tr("chat_wallpaper_upload_failed");
                let published = self.state.with(|s| {
                    if s.is_shutdown {
                        return None;
                    }
                    s.pending_drafts.remove(&temp_id);
                    let mut next = s.snapshot.clone();
                    next.wallpapers.retain(|row| row.id != temp_id);
                    next.mutating_ids.remove(&temp_id);
                    next.mutation_error = Self::message(&error, &fallback);
                    Some(Self::publish_in(s, next))
                });
                let Some(published) = published else {
                    return None;
                };
                RiseChatWallpaperUpload::discard_staged_file(&draft.local_url);
                self.post(published);
                None
            }
        }
    }

    async fn create_remote(
        &self,
        draft: &RiseChatWallpaperDraft,
        look: ChatWallpaperLook,
    ) -> anyhow::Result<RiseChatWallpaper> {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return Err(CancellationError.into());
        };
        let upload_draft = draft.clone();
        let remote_url = hop
            .run(move |mut cx: AsyncApp| {
                async move { RiseChatWallpaperUpload::upload(&upload_draft, &mut cx).await }.boxed_local()
            })
            .await?;
        self.guard_alive()?;

        let created = self
            .rpc
            .create_wallpaper(rise_request::theme::CreateWallpaper {
                blur_percent: Some(look.blur_percent),
                dim_percent: Some(look.dim_percent),
                image_url: Some(remote_url.clone()),
                name: Some(tr("chat_wallpaper_custom_name")),
                preview_url: Some(remote_url),
            })
            .await?;
        self.guard_alive()?;
        let Some(row) = created.wallpaper.filter(|row| !row.id.is_empty()) else {
            return Err(RiseThemeWallpapersError::Rejected(created.error).into());
        };
        Ok(RiseChatWallpaper::from_canon(&row))
    }

    /// pins one wallpaper for every chat, none releases the pin as choosing a gradient preset does
    pub async fn select(&self, id: Option<&str>) {
        if self.state.with(|s| s.is_shutdown) {
            return;
        }
        let wallpaper_id = id.map(|id| trimming_whitespaces_and_newlines(id).to_owned());
        if wallpaper_id.as_deref() == Some("") {
            return;
        }

        let started = self.state.with(|s| {
            if s.snapshot.selected_id == wallpaper_id {
                return None;
            }
            let previous = s.snapshot.selected_id.clone();
            let mut optimistic = s.snapshot.clone();
            optimistic.selected_id = wallpaper_id.clone();
            optimistic.mutation_error = None;
            let published = Self::publish_in(s, optimistic);

            s.generation_clock = s.generation_clock.wrapping_add(1);
            let mutation = s.generation_clock;
            s.selection_generation = mutation;
            Some((previous, mutation, published))
        });
        let Some((previous, mutation, published)) = started else {
            return;
        };
        self.post(published);

        let result = self
            .rpc
            .set_default_wallpaper(rise_request::theme::SetDefaultWallpaper {
                wallpaper_id: Some(wallpaper_id.unwrap_or_default()),
            })
            .await
            .and_then(|_| self.guard_alive().map_err(anyhow::Error::from));
        match result {
            Ok(()) => self.persist_current().await,
            Err(error) => {
                let published = self.state.with(|s| {
                    if s.is_shutdown || s.selection_generation != mutation {
                        return None;
                    }
                    let mut next = s.snapshot.clone();
                    next.selected_id = previous;
                    next.mutation_error = Self::message(&error, "");
                    Self::publish_in(s, next)
                });
                self.post(published);
            }
        }
    }

    pub async fn update_look(&self, id: &str, look: ChatWallpaperLook) {
        let clamped = look.clamped();
        let started = self.state.with(|s| {
            if s.is_shutdown || id.is_empty() {
                return None;
            }
            let index = s.snapshot.wallpapers.iter().position(|row| row.id == id)?;
            let row = &s.snapshot.wallpapers[index];
            if row.is_pending || row.is_built_in {
                return None;
            }
            let previous = row.clone();
            if previous.look() == clamped {
                return None;
            }
            let mut optimistic = s.snapshot.clone();
            optimistic.wallpapers[index].dim_percent = clamped.dim_percent;
            optimistic.wallpapers[index].blur_percent = clamped.blur_percent;
            optimistic.mutation_error = None;
            Some((previous, Self::publish_in(s, optimistic)))
        });
        let Some((previous, published)) = started else {
            return;
        };
        self.post(published);

        let result = self.update_look_remote(id, clamped).await;
        match result {
            Ok(updated) => {
                let published = self.state.with(|s| {
                    let current = s.snapshot.wallpapers.iter().position(|row| row.id == id)?;
                    if s.snapshot.wallpapers[current].look() != clamped {
                        return None;
                    }
                    let mut next = s.snapshot.clone();
                    next.wallpapers[current] = updated;
                    Some(Self::publish_in(s, next))
                });
                let Some(published) = published else {
                    return;
                };
                self.post(published);
                self.persist_current().await;
            }
            Err(error) => {
                let published = self.state.with(|s| {
                    if s.is_shutdown {
                        return None;
                    }
                    let current = s.snapshot.wallpapers.iter().position(|row| row.id == id)?;
                    if s.snapshot.wallpapers[current].look() != clamped {
                        return None;
                    }
                    let mut next = s.snapshot.clone();
                    next.wallpapers[current].dim_percent = previous.dim_percent;
                    next.wallpapers[current].blur_percent = previous.blur_percent;
                    next.mutation_error = Self::message(&error, "");
                    Self::publish_in(s, next)
                });
                self.post(published);
            }
        }
    }

    async fn update_look_remote(&self, id: &str, clamped: ChatWallpaperLook) -> anyhow::Result<RiseChatWallpaper> {
        let response = self
            .rpc
            .update_wallpaper(rise_request::theme::UpdateWallpaper {
                blur_percent: Some(clamped.blur_percent),
                dim_percent: Some(clamped.dim_percent),
                name: None,
                wallpaper_id: Some(id.to_owned()),
            })
            .await?;
        self.guard_alive()?;
        let Some(row) = response.wallpaper else {
            return Err(RiseThemeWallpapersError::Rejected(response.error).into());
        };
        Ok(RiseChatWallpaper::from_canon(&row))
    }

    pub async fn set_own_only(&self, own_only: bool) {
        let started = self.state.with(|s| {
            if s.is_shutdown || s.snapshot.preferences.own_only == own_only {
                return None;
            }
            let previous = s.snapshot.preferences.clone();
            let mut optimistic = s.snapshot.clone();
            optimistic.preferences.own_only = own_only;
            optimistic.mutation_error = None;
            let published = Self::publish_in(s, optimistic);

            s.generation_clock = s.generation_clock.wrapping_add(1);
            let mutation = s.generation_clock;
            s.preferences_generation = mutation;
            Some((previous, mutation, published))
        });
        let Some((previous, mutation, published)) = started else {
            return;
        };
        self.post(published);

        let result = self
            .rpc
            .set_wallpaper_preferences(rise_request::theme::SetWallpaperPreferences { own_only: Some(own_only) })
            .await;
        self.finish_preferences_mutation(result, previous, mutation).await;
    }

    pub async fn set_shows_chat_wallpaper(&self, shows: bool, chat_id: &str) {
        let started = self.state.with(|s| {
            if s.is_shutdown || chat_id.is_empty() || s.snapshot.preferences.shows_chat_wallpaper(chat_id) == shows {
                return None;
            }
            let previous = s.snapshot.preferences.clone();
            let mut optimistic = s.snapshot.clone();
            optimistic.preferences.set_shows_chat_wallpaper(shows, chat_id);
            optimistic.mutation_error = None;
            let published = Self::publish_in(s, optimistic);

            s.generation_clock = s.generation_clock.wrapping_add(1);
            let mutation = s.generation_clock;
            s.preferences_generation = mutation;
            Some((previous, mutation, published))
        });
        let Some((previous, mutation, published)) = started else {
            return;
        };
        self.post(published);

        let result = self
            .rpc
            .set_chat_wallpaper_preference(rise_request::theme::SetChatWallpaperPreference {
                chat_id: Some(chat_id.to_owned()),
                show_chat_wallpaper: Some(shows),
            })
            .await;
        self.finish_preferences_mutation(result, previous, mutation).await;
    }

    async fn finish_preferences_mutation(
        &self,
        result: anyhow::Result<rise_canon::rise_response::theme::GetWallpaperPreferences>,
        previous: RiseWallpaperPreferences,
        mutation: u64,
    ) {
        let result = result.and_then(|reply| {
            self.guard_alive()?;
            Ok(reply)
        });
        match result {
            Ok(reply) => {
                let published = self.state.with(|s| {
                    if s.preferences_generation != mutation {
                        return None;
                    }
                    let mut next = s.snapshot.clone();
                    next.preferences = RiseWallpaperPreferences::from_canon(&reply);
                    Some(Self::publish_in(s, next))
                });
                let Some(published) = published else {
                    return;
                };
                self.post(published);
                self.persist_current().await;
            }
            Err(error) => {
                let published = self.state.with(|s| {
                    if s.is_shutdown || s.preferences_generation != mutation {
                        return None;
                    }
                    let mut next = s.snapshot.clone();
                    next.preferences = previous;
                    next.mutation_error = Self::message(&error, "");
                    Self::publish_in(s, next)
                });
                self.post(published);
            }
        }
    }

    /// removes one wallpaper, a pending row is dropped locally since the server never saw it
    pub async fn delete(&self, id: &str) {
        enum DeleteStart {
            Skip,
            DroppedPending(Option<RiseThemeWallpapersSnapshot>),
            Started(Vec<RiseChatWallpaper>, Option<String>, Option<RiseThemeWallpapersSnapshot>),
        }
        let start = self.state.with(|s| {
            if s.is_shutdown || id.is_empty() {
                return DeleteStart::Skip;
            }
            let Some(row) = s.snapshot.wallpapers.iter().find(|row| row.id == id) else {
                return DeleteStart::Skip;
            };
            if row.is_built_in {
                return DeleteStart::Skip;
            }
            if row.is_pending {
                s.pending_drafts.remove(id);
                let mut next = s.snapshot.clone();
                next.wallpapers.retain(|row| row.id != id);
                next.mutating_ids.remove(id);
                return DeleteStart::DroppedPending(Self::publish_in(s, next));
            }

            let previous_rows = s.snapshot.wallpapers.clone();
            let previous_selection = s.snapshot.selected_id.clone();
            let mut optimistic = s.snapshot.clone();
            optimistic.wallpapers.retain(|row| row.id != id);
            if optimistic.selected_id.as_deref() == Some(id) {
                optimistic.selected_id = None;
            }
            optimistic.mutating_ids.insert(id.to_owned());
            optimistic.mutation_error = None;
            DeleteStart::Started(previous_rows, previous_selection, Self::publish_in(s, optimistic))
        });
        let (previous_rows, previous_selection) = match start {
            DeleteStart::Skip => return,
            DeleteStart::DroppedPending(published) => {
                self.post(published);
                return;
            }
            DeleteStart::Started(previous_rows, previous_selection, published) => {
                self.post(published);
                (previous_rows, previous_selection)
            }
        };

        let result = self
            .rpc
            .delete_wallpaper(rise_request::theme::DeleteWallpaper { wallpaper_id: Some(id.to_owned()) })
            .await
            .and_then(|_| self.guard_alive().map_err(anyhow::Error::from));
        match result {
            Ok(()) => {
                let published = self.state.with(|s| {
                    let mut next = s.snapshot.clone();
                    next.mutating_ids.remove(id);
                    Self::publish_in(s, next)
                });
                self.post(published);
                self.persist_current().await;
            }
            Err(error) => {
                let published = self.state.with(|s| {
                    if s.is_shutdown {
                        return None;
                    }
                    let mut next = s.snapshot.clone();
                    next.wallpapers = previous_rows;
                    next.selected_id = previous_selection;
                    next.mutating_ids.remove(id);
                    next.mutation_error = Self::message(&error, "");
                    Self::publish_in(s, next)
                });
                self.post(published);
            }
        }
    }

    pub fn clear_mutation_error(&self) {
        let published = self.state.with(|s| {
            if s.snapshot.mutation_error.is_none() {
                return None;
            }
            let mut next = s.snapshot.clone();
            next.mutation_error = None;
            Self::publish_in(s, next)
        });
        self.post(published);
    }

    pub async fn shutdown(&self) {
        let stopped = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.is_shutdown = true;
            if let Some(in_flight) = s.in_flight.take() {
                in_flight.abort.abort();
            }
            s.pending_drafts.clear();
            true
        });
        if !stopped {
            return;
        }
        self.resource.shutdown().await;
    }

    /// writes only server confirmed rows, a pending row address is a temp file gone on next launch
    async fn persist_current(&self) {
        let stored = self.state.with(|s| {
            let settled = s.snapshot.wallpapers.iter().filter(|row| !row.is_pending).cloned().collect();
            RiseStoredThemeWallpapers::new(
                settled,
                s.snapshot.selected_id.clone(),
                Some(s.snapshot.preferences.clone()),
            )
        });
        let _ = self.resource.store(stored).await;
    }

    /// session liveness only, the account is gated one layer up by the domain and the presentation
    fn guard_alive(&self) -> Result<(), CancellationError> {
        if self.state.with(|s| s.is_shutdown) { Err(CancellationError) } else { Ok(()) }
    }

    /// isolated half of publish, the snapshot changes in the same turn as the decision
    fn publish_in(
        s: &mut RiseThemeWallpapersRepositoryState,
        next: RiseThemeWallpapersSnapshot,
    ) -> Option<RiseThemeWallpapersSnapshot> {
        if s.is_shutdown {
            return None;
        }
        s.snapshot = next.clone();
        Some(next)
    }

    /// main thread half of publish
    fn post(&self, next: Option<RiseThemeWallpapersSnapshot>) {
        let Some(next) = next else {
            return;
        };
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let selected = next.selected().cloned();
        let preferences = next.preferences.clone();
        hop.post(move |cx: &mut App| {
            RiseThemeWallpapersPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(next, &account_id, &account_epoch, session_id, cx);
            });
            ChatWallpaperStore::shared(cx).update(cx, |store, cx| {
                store.apply_image_wallpaper(
                    selected.as_ref().map(|row| row.id.as_str()),
                    selected.as_ref().map(|row| row.image_url.as_str()),
                    selected.as_ref().map(RiseChatWallpaper::look),
                    &account_id,
                    &account_epoch,
                    cx,
                );
                store.apply_preferences(&preferences, &account_id, &account_epoch, cx);
            });
        });
    }

    /// newest first with pending rows ahead of everything the server knows
    fn ordered(rows: Vec<RiseChatWallpaper>) -> Vec<RiseChatWallpaper> {
        let (mut pending, mut settled): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| row.is_pending);
        settled.sort_by(|left, right| right.created_at_ms.cmp(&left.created_at_ms));
        pending.extend(settled);
        pending
    }

    /// banner text or none
    /// cancel and stale session give none, refusal or lost link gets the canon phrase
    /// else the screens own sentence
    fn message(error: &anyhow::Error, fallback: &str) -> Option<String> {
        if error.downcast_ref::<CancellationError>().is_some() {
            return None;
        }
        if let Some(theme_error) = error.downcast_ref::<RiseThemeWallpapersError>() {
            return match theme_error {
                RiseThemeWallpapersError::MissingActiveSession | RiseThemeWallpapersError::StaleAccount => None,
                RiseThemeWallpapersError::Rejected(reason) => {
                    let trimmed = trimming_whitespaces_and_newlines(reason);
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_owned());
                    }
                    (!fallback.is_empty()).then(|| fallback.to_owned())
                }
                RiseThemeWallpapersError::LimitReached => theme_error.error_description(),
            };
        }
        if !fallback.is_empty()
            && CanonErrors::failure(error).is_none()
            && !CanonErrors::is_connection_failure(error)
        {
            return Some(fallback.to_owned());
        }
        Some(CanonErrors::refusal_text(error))
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
