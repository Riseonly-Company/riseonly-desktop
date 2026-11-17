use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use rise_canon::rise_request;
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::RiseIsolated;
use rise_engine::RiseEngine;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::language_store::LanguageStore;
use tokio::task::AbortHandle;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::notify::engine::core::rise_announcements_resource_repository::{
    RiseAnnouncementClose, RiseAnnouncementsCacheSource, RiseAnnouncementsResourceRepository,
};
use crate::modules::notify::engine::core::rise_announcements_wire_models::{
    RiseAnnouncement, RiseAnnouncementAckOutcome,
};
use crate::modules::notify::engine::core::rise_notify_rpc::RiseNotifyRPC;
use crate::modules::notify::engine::core::rise_release_media_warmer::RiseReleaseMediaWarmer;
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;
use crate::modules::notify::engine::core::rise_release_notes_wire_models::{
    RiseReleaseNote, RiseReleaseSlide,
};
use crate::modules::notify::engine::rise_announcements_presentation::{
    RiseAnnouncementsPresentation, RiseAnnouncementsSnapshot,
};

static REFRESH_INTERVAL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::freshness_ms("announcements"));
static MAXIMUM_RESOLVED: LazyLock<usize> = LazyLock::new(|| {
    usize::try_from(RiseLimits::service_value("max_resolved_announcements")).unwrap_or(0)
});
static MAXIMUM_SEEN_SLIDES: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("announcement_seen_slides_clamp"));

/// the running refresh
/// done resolves when it ends without cancelling it, abort cancels
struct RiseAnnouncementsInFlightRefresh {
    generation: u64,
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

struct RiseAnnouncementsRepositoryState {
    snapshot: RiseAnnouncementsSnapshot,
    request_generation: u64,
    open_generation: u64,
    last_refreshed_at_ms: i64,
    is_shutdown: bool,
    in_flight_refresh: Option<RiseAnnouncementsInFlightRefresh>,
    media_warmup: Option<AbortHandle>,
    acknowledged_shown: HashSet<i64>,
    dismissed_this_session: HashSet<i64>,
    unacked_closes: Vec<RiseAnnouncementClose>,
    pinned: Option<RiseAnnouncement>,
    deferred_open_id: Option<i64>,
    reopened: HashMap<i64, RiseAnnouncement>,
}

enum RiseAnnouncementsLoadStep {
    Skip,
    Join(Shared<BoxFuture<'static, ()>>),
    Start {
        generation: u64,
        done: Shared<BoxFuture<'static, ()>>,
    },
}

enum RiseAnnouncementsOpenStep {
    Skip,
    Fetch(u64),
}

struct RequestContext {
    language: String,
    is_current_account: bool,
}

/// operator broadcasts addressed to this account
/// cache first, memory then store then one hidden refresh
/// a network failure never clears a valid snapshot
pub struct RiseAnnouncementsRepository {
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    rpc: RiseNotifyRPC,
    resource: RiseAnnouncementsResourceRepository,
    app_version: String,
    state: RiseIsolated<RiseAnnouncementsRepositoryState>,
}

impl RiseAnnouncementsRepository {
    pub fn new(
        engine: Arc<RiseEngine>,
        account_id: String,
        account_epoch: String,
        session_id: Uuid,
    ) -> Self {
        let rpc = RiseNotifyRPC::new(engine.clone());
        let app_version = RiseReleaseNotesDeviceContext::app_version().to_owned();
        let resource = RiseAnnouncementsResourceRepository::new(engine, account_id.clone());
        Self {
            account_id,
            account_epoch,
            session_id,
            rpc,
            resource,
            app_version,
            state: RiseIsolated::new(RiseAnnouncementsRepositoryState {
                snapshot: RiseAnnouncementsSnapshot::EMPTY,
                request_generation: 0,
                open_generation: 0,
                last_refreshed_at_ms: 0,
                is_shutdown: false,
                in_flight_refresh: None,
                media_warmup: None,
                acknowledged_shown: HashSet::new(),
                dismissed_this_session: HashSet::new(),
                unacked_closes: Vec::new(),
                pinned: None,
                deferred_open_id: None,
                reopened: HashMap::new(),
            }),
        }
    }

    /// cache only read so the first snapshot on activation costs no round trip
    pub async fn activate(&self) -> RiseAnnouncementsSnapshot {
        if let Some(snapshot) = self.state.with(|s| s.is_shutdown.then(|| s.snapshot.clone())) {
            return snapshot;
        }
        let closes = self.resource.unacked_closes().await;
        let stopped = self.state.with(|s| {
            s.dismissed_this_session
                .extend(closes.iter().map(|close| close.announcement_id));
            s.unacked_closes = closes;
            s.is_shutdown.then(|| s.snapshot.clone())
        });
        if let Some(snapshot) = stopped {
            return snapshot;
        }
        if let Ok(cached) = self.resource.cached().await
            && !cached.announcements.is_empty()
        {
            return self.state.with(|s| {
                if s.is_shutdown {
                    return s.snapshot.clone();
                }
                let source = cached.source;
                let pending = Self::presentable(s, &cached.announcements);
                Self::apply(s, |snapshot| {
                    snapshot.pending = pending;
                    snapshot.cache_source = source;
                    snapshot.is_refreshing = false;
                });
                s.snapshot.clone()
            });
        }
        self.state.with(|s| s.snapshot.clone())
    }

    pub async fn load(self: &Arc<Self>, force: bool) {
        if self.account_id.is_empty() {
            return;
        }

        let now = Self::now_milliseconds();
        let weak = Arc::downgrade(self);
        let step = self.state.with(|s| {
            if s.is_shutdown {
                return RiseAnnouncementsLoadStep::Skip;
            }
            if !force
                && s.last_refreshed_at_ms > 0
                && now - s.last_refreshed_at_ms < *REFRESH_INTERVAL_MS
            {
                return RiseAnnouncementsLoadStep::Skip;
            }
            if !force
                && let Some(in_flight) = s.in_flight_refresh.as_ref()
                && in_flight.done.peek().is_none()
            {
                return RiseAnnouncementsLoadStep::Join(in_flight.done.clone());
            }
            if let Some(in_flight) = s.in_flight_refresh.take() {
                in_flight.abort.abort();
            }
            s.request_generation = s.request_generation.wrapping_add(1);
            let generation = s.request_generation;
            self.publish(s, |snapshot| snapshot.is_refreshing = true);
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
            s.in_flight_refresh = Some(RiseAnnouncementsInFlightRefresh {
                generation,
                abort,
                done: done.clone(),
            });
            RiseAnnouncementsLoadStep::Start { generation, done }
        });

        let (generation, done) = match step {
            RiseAnnouncementsLoadStep::Skip => return,
            RiseAnnouncementsLoadStep::Join(done) => {
                done.await;
                return;
            }
            RiseAnnouncementsLoadStep::Start { generation, done } => (generation, done),
        };

        done.await;
        self.state.with(|s| {
            if s.in_flight_refresh.as_ref().map(|in_flight| in_flight.generation) == Some(generation)
            {
                s.in_flight_refresh = None;
            }
        });
    }

    async fn perform_refresh(&self, generation: u64) {
        let context = Self::current_context(&self.account_id, &self.account_epoch).await;
        if !self.is_current_request(generation) || !context.is_current_account {
            return;
        }

        let request = rise_request::notify::GetPendingAnnouncements {
            announcement_id: Some(0),
            app_version: Some(self.app_version.clone()),
            locale: Some(context.language.clone()),
            platform: Some(RiseReleaseNotesDeviceContext::platform().to_owned()),
        };
        match self.rpc.get_pending_announcements(request).await {
            Ok(response) => {
                if !self.is_current_request(generation) {
                    return;
                }

                let announcements: Vec<RiseAnnouncement> = response
                    .announcements
                    .iter()
                    .map(RiseAnnouncement::from_dto)
                    .filter(RiseAnnouncement::is_presentable)
                    .collect();

                self.state
                    .with(|s| s.last_refreshed_at_ms = Self::now_milliseconds());
                let _ = self.resource.store(&announcements).await;
                if !self.is_current_request(generation) {
                    return;
                }

                self.state.with(|s| Self::prune(s, &announcements));
                self.resend_unacked_closes().await;
                let pending = self.state.with(|s| {
                    (!s.is_shutdown && generation == s.request_generation)
                        .then(|| Self::presentable(s, &announcements))
                });
                let Some(pending) = pending else {
                    return;
                };

                if let Some(first) = pending.first() {
                    RiseReleaseMediaWarmer::warm_first_slide(
                        &Self::media_carrier(first),
                        RiseReleaseMediaWarmer::FIRST_SLIDE_DEADLINE,
                    )
                    .await;
                    if !self.is_current_request(generation) {
                        return;
                    }
                    self.warm_remaining_media(first);
                }

                self.state.with(|s| {
                    if s.is_shutdown || generation != s.request_generation {
                        return;
                    }
                    self.publish(s, |snapshot| {
                        snapshot.pending = pending;
                        snapshot.cache_source = RiseAnnouncementsCacheSource::Network;
                        snapshot.is_refreshing = false;
                    });
                });
            }
            Err(_) => {
                self.state.with(|s| {
                    if s.is_shutdown || generation != s.request_generation {
                        return;
                    }
                    self.publish(s, |snapshot| snapshot.is_refreshing = false);
                });
            }
        }
    }

    /// reopen one broadcast by id, eg from its notification row
    /// server returns a dismissed one only when named
    /// own generation so it doesnt void an in flight refresh
    pub async fn open(&self, announcement_id: i64) {
        let step = self.state.with(|s| {
            if s.is_shutdown || announcement_id <= 0 {
                return RiseAnnouncementsOpenStep::Skip;
            }
            s.deferred_open_id = None;

            let known = s
                .snapshot
                .pending
                .iter()
                .find(|announcement| announcement.id == announcement_id)
                .or_else(|| s.reopened.get(&announcement_id))
                .cloned();
            if let Some(known) = known {
                s.pinned = Some(known);
                self.publish(s, |_| {});
                return RiseAnnouncementsOpenStep::Skip;
            }

            s.open_generation = s.open_generation.wrapping_add(1);
            RiseAnnouncementsOpenStep::Fetch(s.open_generation)
        });
        let RiseAnnouncementsOpenStep::Fetch(generation) = step else {
            return;
        };

        let context = Self::current_context(&self.account_id, &self.account_epoch).await;
        if !self.is_current_open(generation) || !context.is_current_account {
            return;
        }

        let request = rise_request::notify::GetPendingAnnouncements {
            announcement_id: Some(announcement_id),
            app_version: Some(self.app_version.clone()),
            locale: Some(context.language.clone()),
            platform: Some(RiseReleaseNotesDeviceContext::platform().to_owned()),
        };
        let response = match self.rpc.get_pending_announcements(request).await {
            Ok(response) => response,
            Err(error) => {
                if !CanonErrors::is_connection_failure(&error) {
                    return;
                }
                self.state.with(|s| {
                    if !s.is_shutdown && generation == s.open_generation {
                        s.deferred_open_id = Some(announcement_id);
                    }
                });
                return;
            }
        };
        if !self.is_current_open(generation) {
            return;
        }

        let Some(announcement) = response
            .announcements
            .iter()
            .map(RiseAnnouncement::from_dto)
            .find(|announcement| announcement.id == announcement_id && announcement.is_presentable())
        else {
            return;
        };

        RiseReleaseMediaWarmer::warm_first_slide(
            &Self::media_carrier(&announcement),
            RiseReleaseMediaWarmer::TAP_OPEN_DEADLINE,
        )
        .await;

        let published = self.state.with(|s| {
            if s.is_shutdown || generation != s.open_generation {
                return false;
            }
            s.reopened.insert(announcement.id, announcement.clone());
            s.pinned = Some(announcement.clone());
            self.publish(s, |_| {});
            true
        });
        if published {
            self.warm_remaining_media(&announcement);
        }
    }

    pub async fn retry_deferred_open(&self) {
        let deferred = self
            .state
            .with(|s| if s.is_shutdown { None } else { s.deferred_open_id });
        let Some(announcement_id) = deferred else {
            return;
        };
        self.open(announcement_id).await;
    }

    pub async fn mark_shown(&self, announcement_id: i64) {
        let first_time = self.state.with(|s| {
            if s.is_shutdown || announcement_id <= 0 || s.acknowledged_shown.contains(&announcement_id)
            {
                return false;
            }
            s.acknowledged_shown.insert(announcement_id);
            true
        });
        if !first_time {
            return;
        }
        self.send_ack(announcement_id, 1, RiseAnnouncementAckOutcome::Shown)
            .await;
    }

    /// local removal first in the same turn as the click
    /// then the ack that stops it returning on another device
    pub async fn dismiss(&self, announcement_id: i64, seen_slides: i64) {
        self.complete(
            announcement_id,
            seen_slides,
            RiseAnnouncementAckOutcome::Dismissed,
        )
        .await;
    }

    pub async fn opened_action(&self, announcement_id: i64, seen_slides: i64) {
        self.complete(announcement_id, seen_slides, RiseAnnouncementAckOutcome::Action)
            .await;
    }

    async fn complete(
        &self,
        announcement_id: i64,
        seen_slides: i64,
        outcome: RiseAnnouncementAckOutcome,
    ) {
        let remaining = self.state.with(|s| {
            if s.is_shutdown || announcement_id <= 0 {
                return None;
            }
            s.dismissed_this_session.insert(announcement_id);
            if s.pinned.as_ref().map(|pinned| pinned.id) == Some(announcement_id) {
                s.pinned = None;
            }
            self.publish(s, |snapshot| {
                snapshot
                    .pending
                    .retain(|announcement| announcement.id != announcement_id);
            });
            Some(s.snapshot.pending.clone())
        });
        let Some(remaining) = remaining else {
            return;
        };

        let _ = self.resource.store(&remaining).await;

        let close = RiseAnnouncementClose {
            announcement_id,
            seen_slides,
            outcome: outcome.raw_value().to_owned(),
        };
        let closes = self.state.with(|s| {
            s.unacked_closes
                .retain(|pending| pending.announcement_id != announcement_id);
            s.unacked_closes.push(close.clone());
            s.unacked_closes.clone()
        });
        self.resource.store_unacked_closes(&closes).await;

        let delivered = self.send_ack(announcement_id, seen_slides, outcome).await;
        if delivered {
            self.forget_unacked_close(&close).await;
        }
    }

    async fn resend_unacked_closes(&self) {
        let closes = self.state.with(|s| s.unacked_closes.clone());
        for close in closes {
            if self.state.with(|s| s.is_shutdown) {
                return;
            }
            let Some(outcome) = RiseAnnouncementAckOutcome::from_raw_value(&close.outcome) else {
                return;
            };
            let delivered = self
                .send_ack(close.announcement_id, close.seen_slides, outcome)
                .await;
            if !delivered {
                return;
            }
            self.forget_unacked_close(&close).await;
        }
    }

    async fn forget_unacked_close(&self, close: &RiseAnnouncementClose) {
        let closes = self.state.with(|s| {
            if s.is_shutdown || !s.unacked_closes.contains(close) {
                return None;
            }
            s.unacked_closes.retain(|pending| pending != close);
            Some(s.unacked_closes.clone())
        });
        let Some(closes) = closes else {
            return;
        };
        self.resource.store_unacked_closes(&closes).await;
    }

    async fn send_ack(
        &self,
        announcement_id: i64,
        seen_slides: i64,
        outcome: RiseAnnouncementAckOutcome,
    ) -> bool {
        let context = Self::current_context(&self.account_id, &self.account_epoch).await;
        if !context.is_current_account {
            return false;
        }

        let request = rise_request::notify::AckAnnouncement {
            announcement_id: Some(announcement_id),
            app_version: Some(self.app_version.clone()),
            locale: Some(context.language),
            outcome: Some(outcome.raw_value().to_owned()),
            platform: Some(RiseReleaseNotesDeviceContext::platform().to_owned()),
            seen_slides: Some(seen_slides.max(0).min(*MAXIMUM_SEEN_SLIDES)),
        };
        match self.rpc.ack_announcement(request).await {
            Ok(_) => true,
            Err(error) => !CanonErrors::is_connection_failure(&error),
        }
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
            s.pinned = None;
            s.deferred_open_id = None;
            s.reopened.clear();
            true
        });
        if !stopped_now {
            return;
        }
        self.resource.shutdown().await;
    }

    fn warm_remaining_media(&self, announcement: &RiseAnnouncement) {
        let carrier = Self::media_carrier(announcement);
        self.state.with(|s| {
            let handle = RiseAccountRuntime::current().spawn(async move {
                RiseReleaseMediaWarmer::warm_remaining_slides(&carrier).await;
            });
            if let Some(previous) = s.media_warmup.replace(handle.abort_handle()) {
                previous.abort();
            }
        });
    }

    /// server order minus anything closed this session, bounded by the server resolve cap
    fn presentable(
        state: &RiseAnnouncementsRepositoryState,
        announcements: &[RiseAnnouncement],
    ) -> Vec<RiseAnnouncement> {
        announcements
            .iter()
            .filter(|announcement| {
                announcement.is_presentable()
                    && !state.dismissed_this_session.contains(&announcement.id)
            })
            .take(*MAXIMUM_RESOLVED)
            .cloned()
            .collect()
    }

    fn prune(state: &mut RiseAnnouncementsRepositoryState, announcements: &[RiseAnnouncement]) {
        let offered: HashSet<i64> = announcements
            .iter()
            .map(|announcement| announcement.id)
            .collect();
        state.dismissed_this_session.retain(|id| offered.contains(id));
    }

    /// current is derived here and nowhere else so a pinned announcement always beats the head of the list
    /// returns whether anything changed
    fn apply(
        state: &mut RiseAnnouncementsRepositoryState,
        mutate: impl FnOnce(&mut RiseAnnouncementsSnapshot),
    ) -> bool {
        let mut next = state.snapshot.clone();
        mutate(&mut next);
        next.current = state
            .pinned
            .clone()
            .or_else(|| next.pending.first().cloned());
        if next == state.snapshot {
            return false;
        }
        state.snapshot = next;
        true
    }

    /// mutate and publish in one step so state and presentation cant disagree
    fn publish(
        &self,
        state: &mut RiseAnnouncementsRepositoryState,
        mutate: impl FnOnce(&mut RiseAnnouncementsSnapshot),
    ) {
        if !Self::apply(state, mutate) {
            return;
        }

        let next = state.snapshot.clone();
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        hop.post(move |cx| {
            RiseAnnouncementsPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(next, &account_id, &account_epoch, session_id, cx);
            });
        });
    }

    fn is_current_request(&self, generation: u64) -> bool {
        self.state
            .with(|s| !s.is_shutdown && generation == s.request_generation)
    }

    fn is_current_open(&self, generation: u64) -> bool {
        self.state
            .with(|s| !s.is_shutdown && generation == s.open_generation)
    }

    /// language and account check in one main thread hop, both are read on every network turn
    async fn current_context(account_id: &str, account_epoch: &str) -> RequestContext {
        let not_current = RequestContext {
            language: String::new(),
            is_current_account: false,
        };
        let Some(hop) = RiseEngineMainHop::installed() else {
            return not_current;
        };
        let account_id = account_id.to_owned();
        let account_epoch = account_epoch.to_owned();
        let answer = hop
            .run(move |cx| {
                async move {
                    Ok::<_, anyhow::Error>(cx.update(|cx| {
                        (
                            LanguageStore::shared(cx).code().to_owned(),
                            SaiAccountCacheScope::is_current_account_epoch(
                                Some(&account_id),
                                Some(&account_epoch),
                            ),
                        )
                    }))
                }
                .boxed_local()
            })
            .await;
        match answer {
            Ok((language, is_current_account)) => RequestContext {
                language,
                is_current_account,
            },
            Err(_) => not_current,
        }
    }

    /// the release media warmer owns the disk cache key policy and video destination
    /// a second warmer would fork both so slides are handed to it as a release note
    fn media_carrier(announcement: &RiseAnnouncement) -> RiseReleaseNote {
        RiseReleaseNote::new(
            announcement.id,
            String::new(),
            false,
            announcement.locale.clone(),
            announcement.title.clone(),
            announcement.sent_at,
            announcement
                .slides
                .iter()
                .map(|slide| RiseReleaseSlide {
                    id: slide.id,
                    position: slide.position,
                    media_url: slide.media_url.clone(),
                    media_kind: slide.media_kind.clone(),
                    title: slide.title.clone(),
                    body: slide.body.clone(),
                    action_title: slide.action_title.clone(),
                    action_url: slide.action_url.clone(),
                })
                .collect(),
        )
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
