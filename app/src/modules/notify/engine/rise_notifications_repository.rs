use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp};
use rise_canon::rise_request::notify::DeleteNotification;
use rise_canon::rise_request::notify::GetNotifications;
use rise_canon::rise_request::notify::MarkAllNotificationsRead;
use rise_canon::rise_request::notify::MarkNotificationRead;
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseEngine;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::language_store::LanguageStore;
use rise_i18n::tr;
use rise_platform::notifications::{NotificationAuthorizationStatus, authorization_status};
use serde_json::{Map, Value};
use tokio::task::JoinHandle;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiNavigationBridge;
use crate::core::push_notifications::push_notification_manager::PushNotificationManager;
use crate::core::push_notifications::push_notification_presentation::PushNotificationRouteParser;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::widgets::notifier::notifier_store::{
    NotifierAction, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;
use crate::modules::notify::engine::core::rise_announcements_resource_repository::RiseAnnouncementsCacheSource;
use crate::modules::notify::engine::core::rise_notifications_resource_repository::RiseNotificationsResourceRepository;
use crate::modules::notify::engine::core::rise_notifications_wire_models::RiseNotificationDTOItem;
use crate::modules::notify::engine::core::rise_notify_rpc::RiseNotifyRPC;
use crate::modules::notify::engine::rise_notifications_presentation::{
    NotifyFeedChrome, RiseNotificationsLoadState, RiseNotificationsPrepareSeed,
    RiseNotificationsPresentation, RiseNotificationsSnapshot, RiseNotificationsSnapshotBuilder,
};
use crate::modules::notify::engine::rise_notify_unread_repository::RiseNotifyUnreadRepository;
use crate::modules::notify::stores::notify_actions::notify_types::{
    NotificationActorItem, NotificationItem, NotificationKind,
};

const MODERATION_BANNER_SYMBOL: &str = "shield.lefthalf.filled";

static REFRESH_INTERVAL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::freshness_ms("notifications_feed"));
static PAGE_SIZE: LazyLock<i64> = LazyLock::new(|| {
    RiseTimingCanon::page("notifications_feed_page").client_page_size_items.unwrap_or(20)
});
static PERSISTED_ITEMS: LazyLock<usize> = LazyLock::new(|| {
    usize::try_from(RiseLimits::client_value("notification_feed_persisted_items")).unwrap_or(0)
});
static MEMORY_ITEMS: LazyLock<usize> = LazyLock::new(|| {
    usize::try_from(RiseLimits::client_value("notification_feed_memory_items")).unwrap_or(0)
});

struct RiseNotificationsRepositoryState {
    items: Vec<NotificationItem>,
    has_more: bool,
    has_newer: bool,
    is_loading_newer: bool,
    head_generation: u64,
    total_unread: i64,
    load_state: RiseNotificationsLoadState,
    is_refreshing: bool,
    is_loading_more: bool,
    is_marking_all_read: bool,
    refresh_after_mark_all_read: bool,
    cache_source: RiseAnnouncementsCacheSource,
    revision: u64,
    request_generation: u64,
    pagination_generation: u64,
    observers: HashSet<Uuid>,
    prepare_generation: u64,
    last_refreshed_at_ms: i64,
    presentation_language: Option<String>,
    was_requested: bool,
    head_refresh: Option<JoinHandle<()>>,
    created_ids: Vec<i64>,
    is_shutdown: bool,
}

pub struct RiseNotificationsRepository {
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    rpc: RiseNotifyRPC,
    resource: Arc<RiseNotificationsResourceRepository>,
    unread: Arc<RiseNotifyUnreadRepository>,
    this: Weak<RiseNotificationsRepository>,
    state: RiseIsolated<RiseNotificationsRepositoryState>,
}

impl RiseNotificationsRepository {
    pub fn new(
        engine: Arc<RiseEngine>,
        account_id: String,
        account_epoch: String,
        session_id: Uuid,
        unread: Arc<RiseNotifyUnreadRepository>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            rpc: RiseNotifyRPC::new(engine.clone()),
            resource: Arc::new(RiseNotificationsResourceRepository::new(engine, account_id.clone())),
            account_id,
            account_epoch,
            session_id,
            unread,
            this: this.clone(),
            state: RiseIsolated::new(RiseNotificationsRepositoryState {
                items: Vec::new(),
                has_more: false,
                has_newer: false,
                is_loading_newer: false,
                head_generation: 0,
                total_unread: 0,
                load_state: RiseNotificationsLoadState::Idle,
                is_refreshing: false,
                is_loading_more: false,
                is_marking_all_read: false,
                refresh_after_mark_all_read: false,
                cache_source: RiseAnnouncementsCacheSource::Miss,
                revision: 0,
                request_generation: 0,
                pagination_generation: 0,
                observers: HashSet::new(),
                prepare_generation: 0,
                last_refreshed_at_ms: 0,
                presentation_language: None,
                was_requested: false,
                head_refresh: None,
                created_ids: Vec::new(),
                is_shutdown: false,
            }),
        })
    }

    /// cache only read, gives the first snapshot for the domain to publish as the seed
    pub async fn activate(&self) -> RiseNotificationsSnapshot {
        if self.state.with(|state| state.is_shutdown) {
            return RiseNotificationsSnapshot::EMPTY;
        }
        let cached = self.resource.cached().await.ok();
        self.state.with(|state| {
            if let Some(cached) = cached
                && !cached.items.is_empty()
                && !state.is_shutdown
            {
                state.items = cached.items.into_iter().take(*MEMORY_ITEMS).collect();
                state.total_unread = cached.total_unread;
                state.has_more = true;
                state.load_state = RiseNotificationsLoadState::Loaded;
                state.cache_source = cached.source;
            }
            state.revision = state.revision.wrapping_add(1);
        });
        self.prepare_snapshot().await
    }

    pub async fn load(&self, force: bool) {
        if self.state.with(|state| state.is_shutdown) || self.account_id.is_empty() {
            return;
        }
        let Some(language) = Self::language_code().await else {
            return;
        };
        let language_changed = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            let changed = state.presentation_language.as_deref() != Some(language.as_str());
            if changed {
                state.presentation_language = Some(language);
                state.revision = state.revision.wrapping_add(1);
            }
            Some(changed)
        });
        let Some(language_changed) = language_changed else {
            return;
        };
        if language_changed {
            self.publish();
        }

        let now = Self::now_milliseconds();
        let generation = self.state.with(|state| {
            state.was_requested = true;
            if state.is_marking_all_read {
                state.refresh_after_mark_all_read = true;
                return None;
            }
            if !force
                && !state.has_newer
                && state.load_state == RiseNotificationsLoadState::Loaded
                && state.last_refreshed_at_ms > 0
                && now - state.last_refreshed_at_ms < *REFRESH_INTERVAL_MS
            {
                return None;
            }
            state.request_generation = state.request_generation.wrapping_add(1);
            if state.items.is_empty() {
                state.load_state = RiseNotificationsLoadState::Loading;
            }
            state.is_refreshing = true;
            Some(state.request_generation)
        });
        let Some(generation) = generation else {
            return;
        };
        self.publish();

        if !Self::is_current_account(&self.account_id, &self.account_epoch)
            || !self.is_current_request(generation)
        {
            return;
        }

        let response = self
            .rpc
            .get_notifications(GetNotifications {
                limit: Some(*PAGE_SIZE),
                relative_id: None,
                up: Some(false),
            })
            .await;
        let page = match response {
            Ok(page) => page,
            Err(_) => {
                let failed = self.state.with(|state| {
                    if state.is_shutdown || generation != state.request_generation {
                        return false;
                    }
                    if state.items.is_empty() {
                        state.load_state = RiseNotificationsLoadState::Failed;
                    }
                    state.is_refreshing = false;
                    true
                });
                if failed {
                    self.publish();
                }
                return;
            }
        };
        let fresh: Vec<NotificationItem> =
            page.items.iter().filter_map(|item| item.to_notification_item()).collect();
        let total_unread = self.state.with(|state| {
            if state.is_shutdown || generation != state.request_generation {
                return None;
            }
            let fresh_ids: HashSet<&str> = fresh.iter().map(|item| item.id.as_str()).collect();
            let was_slid = state.has_newer;
            let tail: Vec<NotificationItem> = match fresh.last() {
                Some(floor)
                    if !was_slid
                        && page.is_have_more
                        && state.items.iter().any(|item| item.id == floor.id) =>
                {
                    state
                        .items
                        .iter()
                        .filter(|item| {
                            !fresh_ids.contains(item.id.as_str())
                                && item.created_at_ms <= floor.created_at_ms
                        })
                        .cloned()
                        .collect()
                }
                _ => Vec::new(),
            };
            let keeps_tail = !tail.is_empty();
            let mut items = fresh.clone();
            items.extend(tail);
            state.items = items;
            if !keeps_tail {
                state.has_more = page.is_have_more;
            }
            if was_slid {
                state.has_newer = false;
                state.is_loading_newer = false;
                state.is_loading_more = false;
                state.pagination_generation = state.pagination_generation.wrapping_add(1);
                state.head_generation = state.head_generation.wrapping_add(1);
            }
            Self::cap_window_from_tail(state);
            state.total_unread = page.total_unread;
            state.last_refreshed_at_ms = Self::now_milliseconds();
            state.load_state = RiseNotificationsLoadState::Loaded;
            state.is_refreshing = false;
            state.cache_source = RiseAnnouncementsCacheSource::Network;
            state.revision = state.revision.wrapping_add(1);
            Some(state.total_unread)
        });
        let Some(total_unread) = total_unread else {
            return;
        };
        self.publish();
        self.persist();
        self.unread.apply_authoritative(total_unread).await;
    }

    /// pages only below a head read from the network this session
    /// a cached seed offline would hang a loader under its last row until the connection returns
    pub async fn load_more(&self) {
        let started = self.state.with(|state| {
            if state.is_shutdown
                || state.observers.is_empty()
                || state.is_marking_all_read
                || state.cache_source != RiseAnnouncementsCacheSource::Network
                || !state.has_more
                || state.is_loading_more
                || state.is_loading_newer
                || state.load_state != RiseNotificationsLoadState::Loaded
            {
                return None;
            }
            let last = state.items.last()?;
            let relative_id = last.id.parse::<i64>().ok()?;
            let last_id = last.id.clone();
            state.is_loading_more = true;
            Some((last_id, relative_id, state.pagination_generation))
        });
        let Some((last_id, relative_id, generation)) = started else {
            return;
        };
        self.publish();

        let response = self
            .rpc
            .get_notifications(GetNotifications {
                limit: Some(*PAGE_SIZE),
                relative_id: Some(relative_id),
                up: Some(false),
            })
            .await;
        let published = self.state.with(|state| {
            if state.is_shutdown || generation != state.pagination_generation {
                return false;
            }
            let Ok(page) = &response else {
                state.is_loading_more = false;
                return true;
            };
            if !state.has_more || state.items.last().map(|item| item.id.as_str()) != Some(last_id.as_str()) {
                state.is_loading_more = false;
                return true;
            }
            let known: HashSet<String> = state.items.iter().map(|item| item.id.clone()).collect();
            let next: Vec<NotificationItem> = page
                .items
                .iter()
                .filter_map(|item| item.to_notification_item())
                .filter(|item| !known.contains(&item.id))
                .collect();
            let advanced = !next.is_empty();
            state.items.extend(next);
            state.has_more = page.is_have_more;
            state.is_loading_more = false;
            if state.items.len() > *MEMORY_ITEMS {
                let overflow = state.items.len() - *MEMORY_ITEMS;
                state.items.drain(..overflow);
                state.has_newer = true;
            }
            if advanced {
                state.revision = state.revision.wrapping_add(1);
            }
            true
        });
        if published {
            self.publish();
        }
    }

    pub async fn load_newer(&self) {
        let started = self.state.with(|state| {
            if state.is_shutdown
                || state.observers.is_empty()
                || state.is_marking_all_read
                || !state.has_newer
                || state.is_loading_newer
                || state.is_loading_more
                || state.load_state != RiseNotificationsLoadState::Loaded
            {
                return None;
            }
            let first = state.items.first()?;
            let relative_id = first.id.parse::<i64>().ok()?;
            let first_id = first.id.clone();
            state.is_loading_newer = true;
            Some((first_id, relative_id, state.pagination_generation))
        });
        let Some((first_id, relative_id, generation)) = started else {
            return;
        };
        self.publish();

        let response = self
            .rpc
            .get_notifications(GetNotifications {
                limit: Some(*PAGE_SIZE),
                relative_id: Some(relative_id),
                up: Some(true),
            })
            .await;
        let published = self.state.with(|state| {
            if state.is_shutdown || generation != state.pagination_generation {
                return false;
            }
            state.is_loading_newer = false;
            let Ok(page) = &response else {
                return true;
            };
            if !state.has_newer
                || state.items.first().map(|item| item.id.as_str()) != Some(first_id.as_str())
            {
                return true;
            }
            let known: HashSet<String> = state.items.iter().map(|item| item.id.clone()).collect();
            let mut newer: Vec<NotificationItem> = page
                .items
                .iter()
                .filter_map(|item| item.to_notification_item())
                .filter(|item| !known.contains(&item.id))
                .collect();
            let arrived = !newer.is_empty();
            newer.reverse();
            newer.append(&mut state.items);
            state.items = newer;
            state.has_newer = page.is_have_more;
            Self::cap_window_from_tail(state);
            if arrived || !state.has_newer {
                state.revision = state.revision.wrapping_add(1);
            }
            true
        });
        if published {
            self.publish();
        }
    }

    pub async fn begin_observing(&self, observer_id: Uuid) {
        let inserted = self.state.with(|state| {
            if state.is_shutdown || !state.observers.insert(observer_id) {
                return false;
            }
            state.revision = state.revision.wrapping_add(1);
            true
        });
        if inserted {
            self.publish();
        }
    }

    pub async fn end_observing(&self, observer_id: Uuid) {
        let was_loading = self.state.with(|state| {
            if !state.observers.remove(&observer_id) || !state.observers.is_empty() {
                return false;
            }
            state.pagination_generation = state.pagination_generation.wrapping_add(1);
            let was_loading = state.is_loading_more || state.is_loading_newer;
            state.is_loading_more = false;
            state.is_loading_newer = false;
            if was_loading {
                state.revision = state.revision.wrapping_add(1);
            }
            was_loading
        });
        if was_loading {
            self.publish();
        }
    }

    /// canon limit is the window size not the end of the feed
    /// rows past it are dropped from the far end and read again when scrolled to
    fn cap_window_from_tail(state: &mut RiseNotificationsRepositoryState) {
        if state.items.len() <= *MEMORY_ITEMS {
            return;
        }
        state.items.truncate(*MEMORY_ITEMS);
        state.has_more = true;
    }

    pub async fn note_created(&self, unread_count: Option<i64>, notification_id: Option<i64>) {
        let changed = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            if let Some(notification_id) = notification_id
                && !state.created_ids.contains(&notification_id)
            {
                state.created_ids.push(notification_id);
            }
            let Some(unread_count) = unread_count else {
                return Some(false);
            };
            state.total_unread = unread_count.max(0);
            state.revision = state.revision.wrapping_add(1);
            Some(true)
        });
        let Some(changed) = changed else {
            return;
        };
        if changed {
            self.publish();
        }
        let this = self.this.clone();
        self.state.with(|state| {
            tracing::warn!(target: "riseonly::X753", "note created unread={unread_count:?} head_busy={}", state.head_refresh.is_some());
            if state.head_refresh.is_some() {
                return;
            }
            state.head_refresh = Some(RiseAccountRuntime::current().spawn(async move {
                if let Some(repository) = this.upgrade() {
                    repository.refresh_head_after_created().await;
                }
                if let Some(repository) = this.upgrade() {
                    repository.clear_head_refresh();
                }
            }));
        });
    }

    async fn refresh_head_after_created(&self) {
        let announces = Self::system_notifications_are_off().await;
        let newest_before = self.state.with(|state| {
            if state.is_shutdown || !(state.was_requested || announces) {
                return None;
            }
            if state.has_newer && !state.observers.is_empty() {
                return None;
            }
            let newest = if state.has_newer {
                None
            } else {
                state.items.iter().filter_map(|item| item.id.parse::<i64>().ok()).max()
            };
            Some(newest)
        });
        tracing::warn!(target: "riseonly::X753", "head after created announces={announces} newest_before={newest_before:?}");
        let Some(newest_before) = newest_before else {
            self.state.with(|state| state.created_ids.clear());
            return;
        };
        self.load(true).await;
        let created_ids = self.state.with(|state| std::mem::take(&mut state.created_ids));
        if !announces || (newest_before.is_none() && created_ids.is_empty()) {
            return;
        }
        let arrival = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state
                .items
                .iter()
                .find(|item| {
                    !item.is_read
                        && item.id.parse::<i64>().is_ok_and(|id| {
                            created_ids.contains(&id) || newest_before.is_some_and(|newest| id > newest)
                        })
                })
                .cloned()
        });
        tracing::warn!(target: "riseonly::X753", "head after created arrival={:?}", arrival.as_ref().map(|item| item.id.clone()));
        let Some(arrival) = arrival else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        if !SaiAccountCacheScope::is_current_account_epoch(
                            Some(&account_id),
                            Some(&account_epoch),
                        ) {
                            return;
                        }
                        NotificationInAppBanner::present(&arrival, cx);
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    async fn system_notifications_are_off() -> bool {
        match authorization_status().await {
            NotificationAuthorizationStatus::Denied
            | NotificationAuthorizationStatus::NotDetermined => true,
            _ => false,
        }
    }

    fn clear_head_refresh(&self) {
        self.state.with(|state| state.head_refresh = None);
    }

    pub async fn created_item(&self, notification_id: i64) -> Option<NotificationItem> {
        let page = self
            .rpc
            .get_notifications_surviving_suspension(GetNotifications {
                limit: Some(*PAGE_SIZE),
                relative_id: None,
                up: Some(false),
            })
            .await
            .inspect_err(|error| {
                tracing::warn!(target: "riseonly::X753", "created id={notification_id} feed read failed: {error:#}");
            })
            .ok()?;
        let id = notification_id.to_string();
        page.items
            .iter()
            .filter_map(|item| item.to_notification_item())
            .find(|item| item.id == id)
    }

    pub async fn mark_read(&self, id: &str) {
        let marked = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            let index = state.items.iter().position(|item| item.id == id)?;
            if state.items[index].is_read {
                return None;
            }
            let notification_id = id.parse::<i64>().ok()?;
            state.items[index] = state.items[index].with_read(true);
            state.total_unread = (state.total_unread - 1).max(0);
            state.revision = state.revision.wrapping_add(1);
            Some(notification_id)
        });
        let Some(notification_id) = marked else {
            return;
        };
        self.publish();
        self.persist();
        self.unread.adjust(-1).await;

        let response = self
            .rpc
            .mark_notification_read(MarkNotificationRead { notification_id: Some(notification_id) })
            .await;
        if response.is_ok() {
            return;
        }
        let rolled_back = self.state.with(|state| {
            if state.is_shutdown {
                return false;
            }
            let Some(index) = state.items.iter().position(|item| item.id == id) else {
                return false;
            };
            state.items[index] = state.items[index].with_read(false);
            state.total_unread += 1;
            state.revision = state.revision.wrapping_add(1);
            true
        });
        if !rolled_back {
            return;
        }
        self.publish();
        self.persist();
        self.unread.adjust(1).await;
    }

    pub async fn mark_all_read(&self) {
        let previous = self.state.with(|state| {
            if state.is_shutdown || state.is_marking_all_read {
                return None;
            }
            let previous = state.items.clone();
            let previous_unread = state.total_unread;
            if previous_unread <= 0 && previous.iter().all(|item| item.is_read) {
                return None;
            }
            state.is_marking_all_read = true;
            state.refresh_after_mark_all_read = state.is_refreshing || state.is_loading_more;
            state.request_generation = state.request_generation.wrapping_add(1);
            state.pagination_generation = state.pagination_generation.wrapping_add(1);
            state.is_refreshing = false;
            state.is_loading_more = false;
            state.is_loading_newer = false;
            state.items = state.items.iter().map(|item| item.with_read(true)).collect();
            state.total_unread = 0;
            state.revision = state.revision.wrapping_add(1);
            Some((previous, previous_unread))
        });
        let Some((previous, previous_unread)) = previous else {
            return;
        };
        self.publish();
        self.persist();
        self.unread.clear().await;

        let response = self.rpc.mark_all_notifications_read(MarkAllNotificationsRead {}).await;
        if response.is_err() {
            let rolled_back = self.state.with(|state| {
                if state.is_shutdown {
                    return false;
                }
                let read_now: HashSet<&str> = previous
                    .iter()
                    .filter(|item| item.is_read)
                    .map(|item| item.id.as_str())
                    .collect();
                state.items = state
                    .items
                    .iter()
                    .map(|item| {
                        if read_now.contains(item.id.as_str()) {
                            item.clone()
                        } else {
                            item.with_read(false)
                        }
                    })
                    .collect();
                state.total_unread = previous_unread;
                state.revision = state.revision.wrapping_add(1);
                true
            });
            if !rolled_back {
                return;
            }
            self.publish();
            self.persist();
            self.unread.apply_authoritative(previous_unread).await;
        }
        let refresh = self.state.with(|state| {
            state.is_marking_all_read = false;
            std::mem::replace(&mut state.refresh_after_mark_all_read, false)
        });
        if refresh {
            self.load(true).await;
        }
    }

    pub async fn delete(&self, id: &str) {
        let removed = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            let index = state.items.iter().position(|item| item.id == id)?;
            let notification_id = id.parse::<i64>().ok()?;
            let removed = state.items.remove(index);
            let unread_delta = if removed.is_read { 0 } else { -1 };
            state.total_unread = (state.total_unread + unread_delta).max(0);
            state.revision = state.revision.wrapping_add(1);
            Some((index, removed, unread_delta, notification_id))
        });
        let Some((index, removed, unread_delta, notification_id)) = removed else {
            return;
        };
        self.publish();
        self.persist();
        if unread_delta != 0 {
            self.unread.adjust(unread_delta).await;
        }

        let response = self
            .rpc
            .delete_notification(DeleteNotification { notification_id: Some(notification_id) })
            .await;
        if response.is_ok() {
            return;
        }
        let restored = self.state.with(|state| {
            if state.is_shutdown || state.items.iter().any(|item| item.id == id) {
                return false;
            }
            let at = index.min(state.items.len());
            state.items.insert(at, removed);
            state.total_unread -= unread_delta;
            state.revision = state.revision.wrapping_add(1);
            true
        });
        if !restored {
            return;
        }
        self.publish();
        self.persist();
        if unread_delta != 0 {
            self.unread.adjust(-unread_delta).await;
        }
    }

    /// on reconnect and foreground refresh only for an account that opened the feed and only
    /// outside the freshness window
    pub async fn refresh_if_observed(&self) {
        if !self.state.with(|state| state.was_requested) {
            return;
        }
        self.load(false).await;
    }

    pub async fn shutdown(&self) {
        let head_refresh = self.state.with(|state| {
            state.is_shutdown = true;
            state.pagination_generation = state.pagination_generation.wrapping_add(1);
            state.observers.clear();
            state.is_loading_more = false;
            state.is_loading_newer = false;
            state.head_refresh.take()
        });
        if let Some(head_refresh) = head_refresh {
            head_refresh.abort();
        }
        self.resource.shutdown().await;
    }

    /// only the head of the feed is a valid cache, a slid window isnt written
    fn persist(&self) {
        let head = self.state.with(|state| {
            if state.has_newer {
                return None;
            }
            let head: Vec<NotificationItem> =
                state.items.iter().take(*PERSISTED_ITEMS).cloned().collect();
            Some((head, state.total_unread))
        });
        let Some((head, unread_total)) = head else {
            return;
        };
        let resource = self.resource.clone();
        RiseAccountRuntime::current().spawn(async move {
            let _ = resource.store(&head, unread_total).await;
        });
    }

    fn seed(state: &RiseNotificationsRepositoryState) -> RiseNotificationsPrepareSeed {
        RiseNotificationsPrepareSeed {
            items: state.items.clone(),
            has_more: state.has_more,
            is_loading_more: state.is_loading_more,
            total_unread: state.total_unread,
            load_state: state.load_state,
            is_refreshing: state.is_refreshing,
            cache_source: state.cache_source,
            revision: state.revision,
            has_newer: state.has_newer,
            is_loading_newer: state.is_loading_newer,
            head_generation: state.head_generation,
        }
    }

    async fn prepare_snapshot(&self) -> RiseNotificationsSnapshot {
        let seed = self.state.with(|state| Self::seed(state));
        let Some(chrome) = Self::capture_chrome().await else {
            return RiseNotificationsSnapshot::EMPTY;
        };
        Self::build_detached(seed, chrome).await.unwrap_or(RiseNotificationsSnapshot::EMPTY)
    }

    /// fire and forget
    /// prepare off the isolation then one main thread commit
    /// a prepare overtaken by a newer one is dropped
    fn publish(&self) {
        let (generation, seed) = self.state.with(|state| {
            state.prepare_generation = state.prepare_generation.wrapping_add(1);
            (state.prepare_generation, Self::seed(state))
        });
        let this = self.this.clone();
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        RiseAccountRuntime::current().spawn(async move {
            let Some(chrome) = Self::capture_chrome().await else {
                return;
            };
            let language_code = chrome.language_code.clone();
            let Some(snapshot) = Self::build_detached(seed, chrome).await else {
                return;
            };
            let Some(repository) = this.upgrade() else {
                return;
            };
            if !repository.is_current_prepare(generation) {
                return;
            }
            drop(repository);
            let Some(main) = RiseEngineMainHop::installed() else {
                return;
            };
            main.post(move |cx| {
                if LanguageStore::shared(cx).code() != language_code {
                    return;
                }
                RiseNotificationsPresentation::shared(cx).update(cx, |presentation, cx| {
                    presentation.publish(snapshot, &account_id, &account_epoch, session_id, cx);
                });
            });
        });
    }

    async fn capture_chrome() -> Option<NotifyFeedChrome> {
        let main = RiseEngineMainHop::installed()?;
        main.run(|cx: AsyncApp| {
            async move { Ok(cx.update(NotifyFeedChrome::capture)) }.boxed_local()
        })
        .await
        .ok()
    }

    async fn build_detached(
        seed: RiseNotificationsPrepareSeed,
        chrome: NotifyFeedChrome,
    ) -> Option<RiseNotificationsSnapshot> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let locale = chrome.locale.clone();
                RiseNotificationsSnapshotBuilder::build(
                    &seed,
                    &chrome,
                    Date::now(),
                    &Calendar::current(),
                    &locale,
                )
            })
            .await
            .ok()
    }

    fn is_current_prepare(&self, generation: u64) -> bool {
        self.state.with(|state| !state.is_shutdown && generation == state.prepare_generation)
    }

    fn is_current_request(&self, generation: u64) -> bool {
        self.state.with(|state| !state.is_shutdown && generation == state.request_generation)
    }

    fn is_current_account(account_id: &str, account_epoch: &str) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(Some(account_id), Some(account_epoch))
    }

    async fn language_code() -> Option<String> {
        let main = RiseEngineMainHop::installed()?;
        main.run(|cx: AsyncApp| {
            async move { Ok(cx.update(|cx| LanguageStore::shared(cx).code().to_owned())) }
                .boxed_local()
        })
        .await
        .ok()
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}

enum NotificationInAppBanner {}

impl NotificationInAppBanner {
    /// a row nobody wrote (moderation decision, appeal answer
    /// system notice) has no face
    /// an empty avatar draws a bare skeleton circle so the event symbol takes the slot
    fn icon(item: &NotificationItem, actor: Option<&NotificationActorItem>) -> NotifierIcon {
        let Some(actor) = actor else {
            let symbol = if item.kind == NotificationKind::Moderation {
                MODERATION_BANNER_SYMBOL
            } else {
                item.badge_symbol()
            };
            return NotifierIcon::System(symbol.to_owned());
        };
        NotifierIcon::Avatar {
            source: actor.actor_avatar.clone(),
            name: Some(actor.display_name()),
            user_id: Some(actor.actor_id.clone()),
        }
    }

    fn present(item: &NotificationItem, cx: &mut App) {
        let user_info: Map<String, Value> = item
            .data
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect();
        if !PushNotificationManager::shared(cx).read(cx).admits_gift_banner(&user_info) {
            return;
        }
        let route = PushNotificationRouteParser::route(&user_info);
        let visible = SaiNavigationBridge::visible_screens(cx);
        if route
            .as_ref()
            .is_some_and(|route| visible.iter().any(|screen| route.silences_banner(Some(screen))))
        {
            return;
        }
        let names = item.names_string(&|key| tr(key));
        let action = item.action_suffix(&|key| tr(key));
        let title = if names.is_empty() { item.title.clone() } else { names };
        if title.is_empty() && action.is_empty() {
            return;
        }
        let actors = item.facepile_actors();
        let actor = actors.first();
        let (send_title, content) =
            if title.is_empty() { (action, None) } else { (title, Some(action)) };
        ModerationServicesStore::send_after_sanction_toast(
            send_title,
            NotifierSendOptions {
                content,
                r#type: NotifierType::Info,
                position: NotifierPosition::Top,
                icon: Self::icon(item, actor),
                action: route.map(NotifierAction::DeepLink),
                duration: 5.2,
                is_notification: true,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }
}
