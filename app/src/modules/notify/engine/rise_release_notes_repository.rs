use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use rise_canon::RiseTimingCanon;
use rise_canon::rise_request;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseEngine;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::language_store::LanguageStore;
use tokio::task::AbortHandle;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::notify::engine::core::rise_notify_rpc::RiseNotifyRPC;
use crate::modules::notify::engine::core::rise_release_media_warmer::RiseReleaseMediaWarmer;
use crate::modules::notify::engine::core::rise_release_notes_resource_repository::{
    RiseReleaseNotesCacheSource, RiseReleaseNotesResourceRepository,
};
use crate::modules::notify::engine::core::rise_release_notes_seen_store::{
    RiseReleaseNotesDeviceContext, RiseReleaseNotesSeenStore,
};
use crate::modules::notify::engine::core::rise_release_notes_wire_models::{
    RiseReleaseAckOutcome, RiseReleaseNote,
};
use crate::modules::notify::engine::rise_release_notes_presentation::{
    RiseReleaseNotesPresentation, RiseReleaseNotesSnapshot,
};

static REFRESH_INTERVAL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::freshness_ms("release_notes"));

/// the running refresh
/// done resolves when it ends without cancelling it, abort cancels
struct RiseReleaseNotesInFlightRefresh {
    generation: u64,
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

struct RiseReleaseNotesRepositoryState {
    snapshot: RiseReleaseNotesSnapshot,
    request_generation: u64,
    last_refreshed_at_ms: i64,
    is_shutdown: bool,
    in_flight_refresh: Option<RiseReleaseNotesInFlightRefresh>,
    media_warmup: Option<AbortHandle>,
    acknowledged_shown: HashSet<i64>,
}

enum RiseReleaseNotesLoadStep {
    Skip,
    Join(Shared<BoxFuture<'static, ()>>),
    Start(u64, Shared<BoxFuture<'static, ()>>),
}

/// release notes this build matches
/// cache first, memory then store then one hidden refresh
/// failure never clears a valid snapshot, blocking release never filtered
pub struct RiseReleaseNotesRepository {
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    rpc: RiseNotifyRPC,
    resource: RiseReleaseNotesResourceRepository,
    app_version: String,
    device_id: Option<String>,
    state: RiseIsolated<RiseReleaseNotesRepositoryState>,
}

impl RiseReleaseNotesRepository {
    pub fn new(
        engine: Arc<RiseEngine>,
        account_id: String,
        account_epoch: String,
        session_id: Uuid,
    ) -> Self {
        let rpc = RiseNotifyRPC::new(engine.clone());
        let app_version = RiseReleaseNotesDeviceContext::app_version();
        let device_id = RiseReleaseNotesDeviceContext::device_id();
        let resource = RiseReleaseNotesResourceRepository::new(
            engine,
            account_id.clone(),
            RiseReleaseNotesDeviceContext::app_version(),
        );
        Self {
            account_id,
            account_epoch,
            session_id,
            rpc,
            resource,
            app_version,
            device_id,
            state: RiseIsolated::new(RiseReleaseNotesRepositoryState {
                snapshot: RiseReleaseNotesSnapshot::EMPTY,
                request_generation: 0,
                last_refreshed_at_ms: 0,
                is_shutdown: false,
                in_flight_refresh: None,
                media_warmup: None,
                acknowledged_shown: HashSet::new(),
            }),
        }
    }

    /// cache only read so the first published snapshot on activation costs no round trip
    pub async fn activate(&self) -> RiseReleaseNotesSnapshot {
        if let Some(snapshot) = self.state.with(|s| s.is_shutdown.then(|| s.snapshot.clone())) {
            return snapshot;
        }
        if let Ok(cached) = self.resource.cached().await
            && !cached.releases.is_empty()
        {
            let pending = Self::presentable(&cached.releases);
            return self.state.with(|s| {
                s.snapshot = RiseReleaseNotesSnapshot {
                    pending,
                    cache_source: cached.source,
                    is_refreshing: false,
                    did_refresh: false,
                };
                s.snapshot.clone()
            });
        }
        self.state.with(|s| s.snapshot.clone())
    }

    pub fn current(&self) -> RiseReleaseNotesSnapshot {
        self.state.with(|s| s.snapshot.clone())
    }

    pub async fn load(self: &Arc<Self>, force: bool) {
        if self.state.with(|s| s.is_shutdown) {
            return;
        }
        let Some(device_id) = self.device_id.clone().filter(|id| !id.is_empty()) else {
            self.publish(None, |snapshot| snapshot.did_refresh = true);
            return;
        };

        let now = Self::now_milliseconds();
        let weak = Arc::downgrade(self);
        let (step, published) = self.state.with(|s| {
            if s.is_shutdown {
                return (RiseReleaseNotesLoadStep::Skip, None);
            }
            if !force
                && s.last_refreshed_at_ms > 0
                && now - s.last_refreshed_at_ms < *REFRESH_INTERVAL_MS
            {
                return (RiseReleaseNotesLoadStep::Skip, None);
            }
            if !force && let Some(in_flight) = s.in_flight_refresh.as_ref() {
                return (RiseReleaseNotesLoadStep::Join(in_flight.done.clone()), None);
            }
            if let Some(in_flight) = s.in_flight_refresh.take() {
                in_flight.abort.abort();
            }
            s.request_generation = s.request_generation.wrapping_add(1);
            let generation = s.request_generation;
            let published = Self::apply(s, |snapshot| snapshot.is_refreshing = true);
            let handle = RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = weak.upgrade() {
                    this.perform_refresh(generation, device_id).await;
                }
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.in_flight_refresh = Some(RiseReleaseNotesInFlightRefresh {
                generation,
                abort,
                done: done.clone(),
            });
            (RiseReleaseNotesLoadStep::Start(generation, done), published)
        });
        if let Some(next) = published {
            self.post(next);
        }

        let (generation, done) = match step {
            RiseReleaseNotesLoadStep::Skip => return,
            RiseReleaseNotesLoadStep::Join(done) => {
                done.await;
                return;
            }
            RiseReleaseNotesLoadStep::Start(generation, done) => (generation, done),
        };

        done.await;
        self.state.with(|s| {
            if s.in_flight_refresh.as_ref().map(|in_flight| in_flight.generation) == Some(generation)
            {
                s.in_flight_refresh = None;
            }
        });
    }

    async fn perform_refresh(&self, generation: u64, device_id: String) {
        let Some(language) = Self::language_code().await else {
            self.publish(Some(generation), |snapshot| {
                snapshot.is_refreshing = false;
                snapshot.did_refresh = true;
            });
            return;
        };

        let request = rise_request::notify::GetAppReleaseNotes {
            app_version: Some(self.app_version.clone()),
            device_id: Some(device_id),
            locale: Some(language),
            platform: Some(RiseReleaseNotesDeviceContext::platform().to_owned()),
        };
        let response = match self.rpc.get_app_release_notes(request).await {
            Ok(response) => response,
            Err(_) => {
                self.publish(Some(generation), |snapshot| {
                    snapshot.is_refreshing = false;
                    snapshot.did_refresh = true;
                });
                return;
            }
        };
        if !self.is_current_request(generation) {
            return;
        }

        let releases: Vec<RiseReleaseNote> = response
            .releases
            .iter()
            .map(RiseReleaseNote::from_dto)
            .filter(RiseReleaseNote::is_presentable)
            .collect();

        self.state.with(|s| s.last_refreshed_at_ms = Self::now_milliseconds());
        self.resource.store(&releases).await.ok();
        if !self.is_current_request(generation) {
            return;
        }

        RiseReleaseNotesSeenStore::record_run_version(&self.app_version);

        let pending = Self::presentable(&releases);
        if let Some(first) = pending.first() {
            RiseReleaseMediaWarmer::warm_first_slide(first, RiseReleaseMediaWarmer::FIRST_SLIDE_DEADLINE)
                .await;
            if !self.is_current_request(generation) {
                return;
            }
            self.warm_remaining_media(first);
        }

        self.publish(Some(generation), |snapshot| {
            snapshot.pending = pending;
            snapshot.cache_source = RiseReleaseNotesCacheSource::Network;
            snapshot.is_refreshing = false;
            snapshot.did_refresh = true;
        });
    }

    fn warm_remaining_media(&self, note: &RiseReleaseNote) {
        let note = note.clone();
        self.state.with(|s| {
            let handle = RiseAccountRuntime::current().spawn(async move {
                RiseReleaseMediaWarmer::warm_remaining_slides(&note).await;
            });
            if let Some(previous) = s.media_warmup.replace(handle.abort_handle()) {
                previous.abort();
            }
        });
    }

    /// local removal first in the same turn as the tap
    /// then the ack, which is analytics and nothing on screen waits for it
    pub async fn dismiss(&self, release_id: i64, seen_slides: i64) {
        self.complete(release_id, seen_slides, RiseReleaseAckOutcome::Dismissed).await;
    }

    pub async fn opened_action(&self, release_id: i64, seen_slides: i64) {
        self.complete(release_id, seen_slides, RiseReleaseAckOutcome::Action).await;
    }

    async fn complete(&self, release_id: i64, seen_slides: i64, outcome: RiseReleaseAckOutcome) {
        if release_id <= 0 {
            return;
        }
        let is_blocking = self.state.with(|s| {
            (!s.is_shutdown).then(|| {
                s.snapshot
                    .pending
                    .iter()
                    .find(|release| release.id == release_id)
                    .is_some_and(|release| release.is_blocking)
            })
        });
        let Some(is_blocking) = is_blocking else {
            return;
        };
        if !is_blocking {
            RiseReleaseNotesSeenStore::mark_seen([release_id]);
            self.publish(None, |snapshot| {
                snapshot.pending.retain(|release| release.id != release_id);
            });
        }

        self.send_ack(release_id, seen_slides, outcome).await;
    }

    pub async fn mark_shown(&self, release_id: i64) {
        let accepted = self.state.with(|s| {
            if s.is_shutdown || release_id <= 0 || s.acknowledged_shown.contains(&release_id) {
                return false;
            }
            s.acknowledged_shown.insert(release_id);
            true
        });
        if !accepted {
            return;
        }
        self.send_ack(release_id, 1, RiseReleaseAckOutcome::Shown).await;
    }

    async fn send_ack(&self, release_id: i64, seen_slides: i64, outcome: RiseReleaseAckOutcome) {
        let Some(device_id) = self.device_id.clone().filter(|id| !id.is_empty()) else {
            return;
        };
        let Some(language) = Self::language_code().await else {
            return;
        };

        let request = rise_request::notify::AckAppRelease {
            app_version: Some(self.app_version.clone()),
            device_id: Some(device_id),
            locale: Some(language),
            outcome: Some(outcome.raw_value().to_owned()),
            platform: Some(RiseReleaseNotesDeviceContext::platform().to_owned()),
            release_id: Some(release_id),
            seen_slides: Some(seen_slides.max(0)),
        };
        self.rpc.ack_app_release(request).await.ok();
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
            if let Some(media_warmup) = s.media_warmup.take() {
                media_warmup.abort();
            }
            true
        });
        if !stopped_now {
            return;
        }
        self.resource.shutdown().await;
    }

    /// blocking first then server priority order
    /// anything the device already closed is dropped and a release with no slides never reaches the ui
    fn presentable(releases: &[RiseReleaseNote]) -> Vec<RiseReleaseNote> {
        let seen = RiseReleaseNotesSeenStore::seen_release_ids();
        let visible: Vec<&RiseReleaseNote> = releases
            .iter()
            .filter(|release| {
                release.is_presentable() && (release.is_blocking || !seen.contains(&release.id))
            })
            .collect();
        visible
            .iter()
            .filter(|release| release.is_blocking)
            .chain(visible.iter().filter(|release| !release.is_blocking))
            .map(|release| (*release).clone())
            .collect()
    }

    fn is_current_request(&self, generation: u64) -> bool {
        self.state
            .with(|s| !s.is_shutdown && generation == s.request_generation)
    }

    /// mutation half of publish
    /// gives the changed snapshot or none when nothing moved
    fn apply(
        state: &mut RiseReleaseNotesRepositoryState,
        mutate: impl FnOnce(&mut RiseReleaseNotesSnapshot),
    ) -> Option<RiseReleaseNotesSnapshot> {
        let mut next = state.snapshot.clone();
        mutate(&mut next);
        if next == state.snapshot {
            return None;
        }
        state.snapshot = next.clone();
        Some(next)
    }

    /// mutate and publish in one step so state and presentation cant disagree
    /// with a generation the mutation lands only while that request is still current
    fn publish(&self, generation: Option<u64>, mutate: impl FnOnce(&mut RiseReleaseNotesSnapshot)) {
        let published = self.state.with(|s| {
            if s.is_shutdown || generation.is_some_and(|generation| generation != s.request_generation) {
                return None;
            }
            Self::apply(s, mutate)
        });
        if let Some(next) = published {
            self.post(next);
        }
    }

    /// posts the snapshot to the main thread presentation
    fn post(&self, next: RiseReleaseNotesSnapshot) {
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        hop.post(move |cx| {
            RiseReleaseNotesPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(next, &account_id, &account_epoch, session_id, cx);
            });
        });
    }

    /// current language code from the language store on the main thread
    async fn language_code() -> Option<String> {
        let hop = RiseEngineMainHop::installed()?;
        hop.run(|cx| {
            async move {
                Ok::<_, anyhow::Error>(cx.update(|cx| LanguageStore::shared(cx).code().to_owned()))
            }
            .boxed_local()
        })
        .await
        .ok()
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
