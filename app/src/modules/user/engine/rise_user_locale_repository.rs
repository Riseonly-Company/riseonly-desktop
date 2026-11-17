use std::sync::{Arc, LazyLock, Weak};

use futures::FutureExt as _;
use futures::future::{AbortHandle, Abortable, BoxFuture, Shared};
use rise_canon::RiseTimingCanon;
use rise_canon::rise_request::user::ObserveLocaleSignals;
use rise_canon::rise_request::user::UpdateLocaleSettings;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::{CancellationError, RiseEngine, RiseWireError};
use rise_i18n::AppLanguageCatalog;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::user::engine::core::rise_user_locale_resource_repository::RiseUserLocaleResourceRepository;
use crate::modules::user::engine::core::rise_user_locale_wire_models::{
    RiseLocaleSource, RiseUserLocaleError, RiseUserLocaleSettings,
};
use crate::modules::user::engine::core::rise_user_rpc::RiseUserRPC;
use crate::modules::user::engine::rise_user_locale_presentation::RiseUserLocalePresentation;

static REFRESH_INTERVAL_MS: LazyLock<i64> = LazyLock::new(|| RiseTimingCanon::freshness_ms("user_locale"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseUserLocaleCacheSource {
    Miss,
    Memory,
    PersistentStore,
    Network,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseUserLocaleSnapshot {
    pub settings: RiseUserLocaleSettings,
    pub has_settings: bool,
    pub cache_source: RiseUserLocaleCacheSource,
    pub is_refreshing: bool,
    pub is_mutating: bool,
    pub error: Option<String>,
}

impl RiseUserLocaleSnapshot {
    pub fn empty() -> Self {
        Self {
            settings: RiseUserLocaleSettings::empty(),
            has_settings: false,
            cache_source: RiseUserLocaleCacheSource::Miss,
            is_refreshing: false,
            is_mutating: false,
            error: None,
        }
    }
}

type RefreshTask = Shared<BoxFuture<'static, ()>>;

struct InFlightRefresh {
    task: RefreshTask,
    abort: AbortHandle,
    generation: u64,
}

struct RiseUserLocaleRepositoryState {
    snapshot: RiseUserLocaleSnapshot,
    request_generation: u64,
    last_refreshed_at_ms: i64,
    is_shutdown: bool,
    has_observed_device_signals: bool,
    in_flight_refresh: Option<InFlightRefresh>,
    unsent: Option<UpdateLocaleSettings>,
    unsent_base: Option<RiseUserLocaleSnapshot>,
}

impl RiseUserLocaleRepositoryState {
    /// mutates in one step and returns what to publish so state and presentation cant disagree
    fn mutate(&mut self, mutate: impl FnOnce(&mut RiseUserLocaleSnapshot)) -> Option<RiseUserLocaleSnapshot> {
        let mut next = self.snapshot.clone();
        mutate(&mut next);
        if next == self.snapshot {
            return None;
        }
        self.snapshot = next.clone();
        Some(next)
    }
}

enum RiseUserLocaleLoadStep {
    Skip,
    Join(RefreshTask),
    Run(RefreshTask, u64, Option<RiseUserLocaleSnapshot>),
}

pub struct RiseUserLocaleRepository {
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    resource: RiseUserLocaleResourceRepository,
    rpc: RiseUserRPC,
    this: Weak<Self>,
    state: RiseIsolated<RiseUserLocaleRepositoryState>,
}

impl RiseUserLocaleRepository {
    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            rpc: RiseUserRPC::new(engine.clone()),
            resource: RiseUserLocaleResourceRepository::new(engine, &account_id),
            account_id,
            account_epoch,
            session_id,
            this: this.clone(),
            state: RiseIsolated::new(RiseUserLocaleRepositoryState {
                snapshot: RiseUserLocaleSnapshot::empty(),
                request_generation: 0,
                last_refreshed_at_ms: 0,
                is_shutdown: false,
                has_observed_device_signals: false,
                in_flight_refresh: None,
                unsent: None,
                unsent_base: None,
            }),
        })
    }

    /// reads what is already known,
    /// no network,
    /// so the first snapshot is instant
    pub async fn activate(&self) -> RiseUserLocaleSnapshot {
        if let Some(snapshot) = self.state.with(|state| state.is_shutdown.then(|| state.snapshot.clone())) {
            return snapshot;
        }
        let cached = self.resource.cached().await.ok();
        self.state.with(|state| {
            if let Some(cached) = cached
                && let Some(settings) = cached.settings
            {
                state.snapshot = RiseUserLocaleSnapshot {
                    settings,
                    has_settings: true,
                    cache_source: cached.source,
                    is_refreshing: false,
                    is_mutating: false,
                    error: None,
                };
            }
            state.snapshot.clone()
        })
    }

    pub fn current(&self) -> RiseUserLocaleSnapshot {
        self.state.with(|state| state.snapshot.clone())
    }

    pub async fn load(&self, force: bool) {
        let now = Self::now_milliseconds();
        let step = self.state.with(|state| {
            if state.is_shutdown {
                return RiseUserLocaleLoadStep::Skip;
            }
            if !force && state.snapshot.has_settings && now - state.last_refreshed_at_ms < *REFRESH_INTERVAL_MS {
                return RiseUserLocaleLoadStep::Skip;
            }
            if let Some(in_flight) = &state.in_flight_refresh
                && !force
            {
                return RiseUserLocaleLoadStep::Join(in_flight.task.clone());
            }
            if let Some(in_flight) = state.in_flight_refresh.take() {
                in_flight.abort.abort();
            }

            state.request_generation = state.request_generation.wrapping_add(1);
            let generation = state.request_generation;
            let published = state.mutate(|snapshot| snapshot.is_refreshing = true);

            let this = self.this.clone();
            let (abort, registration) = AbortHandle::new_pair();
            let task: RefreshTask = Abortable::new(
                async move {
                    if let Some(this) = this.upgrade() {
                        this.perform_refresh(generation).await;
                    }
                },
                registration,
            )
            .map(|_| ())
            .boxed()
            .shared();
            state.in_flight_refresh = Some(InFlightRefresh { task: task.clone(), abort, generation });
            RiseUserLocaleLoadStep::Run(task, generation, published)
        });
        match step {
            RiseUserLocaleLoadStep::Skip => {}
            RiseUserLocaleLoadStep::Join(task) => task.await,
            RiseUserLocaleLoadStep::Run(task, generation, published) => {
                if let Some(next) = published {
                    self.publish(next);
                }
                task.await;
                self.state.with(|state| {
                    if state.in_flight_refresh.as_ref().is_some_and(|in_flight| in_flight.generation == generation) {
                        state.in_flight_refresh = None;
                    }
                });
            }
        }
    }

    async fn perform_refresh(&self, generation: u64) {
        match self.rpc.get_locale_settings().await {
            Ok(response) => {
                if !self.is_current_request(generation) {
                    return;
                }
                let Some(settings) = response.settings.as_ref().map(RiseUserLocaleSettings::from_canon) else {
                    let error = response.error;
                    self.mutate_and_publish(|snapshot| {
                        snapshot.is_refreshing = false;
                        snapshot.error = Some(error);
                    });
                    return;
                };
                let shown = self.state.with(|state| {
                    state.last_refreshed_at_ms = Self::now_milliseconds();
                    if let Some(unsent) = &state.unsent {
                        if let Some(unsent_base) = &mut state.unsent_base {
                            unsent_base.settings = settings.clone();
                        }
                        Self::applying(unsent, &settings)
                    } else {
                        settings
                    }
                });
                let _ = self.resource.store(&shown).await;
                if !self.is_current_request(generation) {
                    return;
                }
                self.mutate_and_publish(|snapshot| {
                    snapshot.settings = shown;
                    snapshot.has_settings = true;
                    snapshot.cache_source = RiseUserLocaleCacheSource::Network;
                    snapshot.is_refreshing = false;
                    snapshot.error = None;
                });
            }
            Err(error) => {
                if !self.is_current_request(generation) {
                    return;
                }
                let message = error.to_string();
                self.mutate_and_publish(|snapshot| {
                    snapshot.is_refreshing = false;
                    snapshot.error = Some(message);
                });
            }
        }
    }

    /// optimistic write,
    /// new value shows before the request leaves and the old one comes back if the server rejects
    pub async fn update(&self, request: UpdateLocaleSettings) -> anyhow::Result<()> {
        let prepared = self.state.with(|state| {
            (!state.is_shutdown).then(|| {
                (
                    Self::merging(&request, state.unsent.as_ref()),
                    Self::applying(&request, &state.snapshot.settings),
                )
            })
        });
        let Some((merged, optimistic)) = prepared else {
            return Err(CancellationError.into());
        };
        self.send(merged, optimistic).await
    }

    pub async fn flush_unsent(&self) {
        let pending = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.unsent.clone().map(|pending| (pending, state.snapshot.settings.clone()))
        });
        let Some((pending, optimistic)) = pending else {
            return;
        };
        let _ = self.send(pending, optimistic).await;
    }

    async fn send(&self, request: UpdateLocaleSettings, optimistic: RiseUserLocaleSettings) -> anyhow::Result<()> {
        let (generation, previous, published) = self.state.with(|state| {
            let previous = state.unsent_base.take().unwrap_or_else(|| state.snapshot.clone());
            state.unsent = None;

            state.request_generation = state.request_generation.wrapping_add(1);
            let generation = state.request_generation;
            let optimistic = optimistic.clone();
            let published = state.mutate(|snapshot| {
                snapshot.settings = optimistic;
                snapshot.has_settings = true;
                snapshot.is_mutating = true;
                snapshot.error = None;
            });
            (generation, previous, published)
        });
        if let Some(next) = published {
            self.publish(next);
        }
        let _ = self.resource.store(&optimistic).await;

        let failure = match self.rpc.update_locale_settings(request.clone()).await {
            Ok(response) => {
                if !self.is_current_request(generation) {
                    return Ok(());
                }
                match response.settings.as_ref().map(RiseUserLocaleSettings::from_canon) {
                    Some(settings) => {
                        self.state.with(|state| state.last_refreshed_at_ms = Self::now_milliseconds());
                        let _ = self.resource.store(&settings).await;
                        self.mutate_and_publish(|snapshot| {
                            snapshot.settings = settings;
                            snapshot.cache_source = RiseUserLocaleCacheSource::Network;
                            snapshot.is_mutating = false;
                            snapshot.error = None;
                        });
                        return Ok(());
                    }
                    None => anyhow::Error::from(RiseUserLocaleError::InvalidResponse),
                }
            }
            Err(error) => error,
        };

        if !self.is_current_request(generation) {
            return Err(failure);
        }
        if Self::never_left_device(&failure) {
            self.state.with(|state| {
                state.unsent = Some(request);
                state.unsent_base = Some(previous);
            });
            self.mutate_and_publish(|snapshot| {
                snapshot.is_mutating = false;
                snapshot.error = None;
            });
            return Ok(());
        }
        let _ = self.resource.store(&previous.settings).await;
        let message = failure.to_string();
        self.mutate_and_publish(|snapshot| {
            snapshot.settings = previous.settings;
            snapshot.has_settings = previous.has_settings;
            snapshot.cache_source = previous.cache_source;
            snapshot.is_mutating = false;
            snapshot.error = Some(message);
        });
        Err(failure)
    }

    /// reports what this device is so account region and language can follow when not pinned
    /// silent with no spinner or error, sent once per session
    pub async fn observe_device_signals(&self) {
        let should_observe = self.state.with(|state| {
            if state.is_shutdown || state.has_observed_device_signals {
                return false;
            }
            state.has_observed_device_signals = true;
            true
        });
        if !should_observe {
            return;
        }

        let request = ObserveLocaleSignals {
            device_languages: Some(
                AppLanguageCatalog::device_preferences().into_iter().map(str::to_owned).collect(),
            ),
            device_region: rise_platform::device_locale::current().region,
            timezone: Some(TimeZone::current().identifier()),
            ..ObserveLocaleSignals::default()
        };
        let has_languages = request.device_languages.as_ref().is_some_and(|languages| !languages.is_empty());
        if !has_languages && request.timezone.is_none() && request.device_region.is_none() {
            return;
        }

        let Ok(response) = self.rpc.observe_locale_signals(request).await else {
            return;
        };
        let Some(settings) = response.settings.as_ref().map(RiseUserLocaleSettings::from_canon) else {
            return;
        };
        if self.is_shutdown() {
            return;
        }

        let _ = self.resource.store(&settings).await;
        if self.is_shutdown() {
            return;
        }
        self.mutate_and_publish(|snapshot| {
            snapshot.settings = settings;
            snapshot.has_settings = true;
            snapshot.cache_source = RiseUserLocaleCacheSource::Network;
            snapshot.error = None;
        });
    }

    pub async fn shutdown(&self) {
        let was_shutdown = self.state.with(|state| {
            if state.is_shutdown {
                return true;
            }
            state.is_shutdown = true;
            if let Some(in_flight) = state.in_flight_refresh.take() {
                in_flight.abort.abort();
            }
            false
        });
        if was_shutdown {
            return;
        }
        self.resource.shutdown().await;
    }

    fn is_shutdown(&self) -> bool {
        self.state.with(|state| state.is_shutdown)
    }

    fn is_current_request(&self, generation: u64) -> bool {
        self.state.with(|state| !state.is_shutdown && generation == state.request_generation)
    }

    fn mutate_and_publish(&self, mutate: impl FnOnce(&mut RiseUserLocaleSnapshot)) {
        if let Some(next) = self.state.with(|state| state.mutate(mutate)) {
            self.publish(next);
        }
    }

    fn publish(&self, next: RiseUserLocaleSnapshot) {
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        main.post(move |cx| {
            RiseUserLocalePresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(next, &account_id, &account_epoch, session_id, cx);
            });
        });
    }

    fn applying(request: &UpdateLocaleSettings, base: &RiseUserLocaleSettings) -> RiseUserLocaleSettings {
        let mut settings = base.clone();
        if let Some(region) = &request.region_code {
            settings.region_code = region.clone();
            settings.region_source = RiseLocaleSource::Explicit;
        }
        if let Some(language) = &request.app_language {
            settings.app_language = language.clone();
            settings.app_language_source = RiseLocaleSource::Explicit;
            if settings.notification_language_follows_app {
                settings.notification_language = language.clone();
            }
        }
        if request.notification_language_follows_app == Some(true) {
            settings.notification_language_follows_app = true;
            settings.notification_language = settings.app_language.clone();
        } else if let Some(notification) = &request.notification_language {
            settings.notification_language = notification.clone();
            settings.notification_language_follows_app = false;
        }
        if let Some(timezone) = &request.timezone {
            settings.timezone = timezone.clone();
        }
        settings
    }

    fn merging(request: &UpdateLocaleSettings, earlier: Option<&UpdateLocaleSettings>) -> UpdateLocaleSettings {
        let Some(earlier) = earlier else {
            return request.clone();
        };
        let mut merged = earlier.clone();
        if let Some(region) = &request.region_code {
            merged.region_code = Some(region.clone());
        }
        if let Some(language) = &request.app_language {
            merged.app_language = Some(language.clone());
        }
        if request.notification_language_follows_app == Some(true) {
            merged.notification_language = None;
            merged.notification_language_follows_app = Some(true);
        } else if let Some(notification) = &request.notification_language {
            merged.notification_language = Some(notification.clone());
            merged.notification_language_follows_app = request.notification_language_follows_app;
        }
        if let Some(timezone) = &request.timezone {
            merged.timezone = Some(timezone.clone());
        }
        merged
    }

    fn never_left_device(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            matches!(
                cause.downcast_ref::<RiseWireError>(),
                Some(RiseWireError::NotConnected | RiseWireError::NoSocketSession)
            )
        })
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
