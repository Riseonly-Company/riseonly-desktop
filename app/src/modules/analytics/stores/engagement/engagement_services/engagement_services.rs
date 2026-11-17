use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Subscription, Task};
use rise_canon::RiseTimingCanon;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_isolated::RiseIsolated;
use rise_platform::AppPaths;
use rise_platform::file_manager::FileManager;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::core::lib::canon_timing::{RiseTimingBatchExt, RiseTimingCanonExt};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_actions::EngagementActionsStore;
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementEvent;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

static PERSISTENCE_QUEUE: LazyLock<RiseIsolated<u64>> = LazyLock::new(|| RiseIsolated::new(0));

static QUEUES_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::current().canon())
        .ok()
        .map(|paths| paths.cache_dir().join(EngagementServicesStore::QUEUES_DIRECTORY_NAME))
});

pub struct EngagementServicesStore {
    queue_cap: usize,
    flush_threshold: usize,
    batch_max: usize,
    flush_interval_sec: u64,
    max_retries: i64,

    session_id: String,

    queue: Vec<EngagementEvent>,
    seen_impressions: HashSet<String>,
    is_flushing: bool,
    in_flight_retry: i64,
    started: bool,
    timer_task: Option<Task<()>>,
    account_generation: u64,
    persist_task: Option<Task<()>>,
    persist_pending: bool,
    persist_ticket: u64,
    background_observer: Option<RiseSubscription>,
    background_delivery: Option<Task<()>>,
    app_quit_subscription: Option<Subscription>,
}

impl EngagementServicesStore {
    const ENGAGEMENT_EVENT_BATCH: &'static str = "analytics_events";
    const QUEUES_DIRECTORY_NAME: &'static str = "EngagementQueues";
    const ANONYMOUS_ACCOUNT_KEY: &'static str = "anonymous";
    const QUEUE_FILE_NAME: &'static str = "engagement_queue.json";
    const PERSIST_COALESCE_WINDOW: Duration = Duration::from_secs(2);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        let batch = RiseTimingCanon::batch(Self::ENGAGEMENT_EVENT_BATCH);
        Self {
            queue_cap: usize::try_from(batch.queue_cap_items()).unwrap_or(0),
            flush_threshold: usize::try_from(batch.flush_threshold_items()).unwrap_or(0),
            batch_max: usize::try_from(batch.client_batch_items).unwrap_or(0),
            flush_interval_sec: batch.flush_interval_seconds(),
            max_retries: batch.maximum_retries(),
            session_id: Self::new_session_id(),
            queue: Vec::new(),
            seen_impressions: HashSet::new(),
            is_flushing: false,
            in_flight_retry: 0,
            started: false,
            timer_task: None,
            account_generation: 0,
            persist_task: None,
            persist_pending: false,
            persist_ticket: 0,
            background_observer: None,
            background_delivery: None,
            app_quit_subscription: None,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    fn new_session_id() -> String {
        format!("app_{}", Uuid::new_v4().uuid_string())
    }

    pub fn reset_account_state(&mut self, _cx: &mut Context<Self>) {
        self.persist_task = None;
        self.persist_pending = false;
        self.persist_synchronously();
        self.account_generation = self.account_generation.wrapping_add(1);
        self.session_id = Self::new_session_id();
        self.queue.clear();
        self.seen_impressions.clear();
        self.is_flushing = false;
        self.in_flight_retry = 0;
    }

    pub fn activate_account_state(&mut self, _cx: &mut Context<Self>) {
        self.queue.clear();
        self.seen_impressions.clear();
        self.load_persisted();
    }

    pub fn remove_persisted_account(&self, user_id: &str) {
        let Some(directory) = self.persist_directory(Some(user_id)) else {
            return;
        };
        PERSISTENCE_QUEUE.with(|_| {
            FileManager.remove_item(&directory).ok();
        });
    }

    pub fn storage_directory_url(user_id: Option<&str>) -> Option<PathBuf> {
        Self::directory_url(user_id)
    }

    fn persist_directory(&self, user_id: Option<&str>) -> Option<PathBuf> {
        Self::directory_url(user_id)
    }

    fn directory_url(user_id: Option<&str>) -> Option<PathBuf> {
        let root = QUEUES_ROOT.as_ref()?;
        let value = trimming_whitespaces_and_newlines(user_id.unwrap_or(""));
        let account_key = if value.is_empty() {
            Self::ANONYMOUS_ACCOUNT_KEY.to_owned()
        } else {
            sha256_hex(value.as_bytes())
        };
        Some(root.join(account_key))
    }

    fn persist_url(&self, user_id: Option<&str>) -> Option<PathBuf> {
        self.persist_directory(user_id)
            .map(|directory| directory.join(Self::QUEUE_FILE_NAME))
    }

    fn is_authorized(cx: &mut App) -> bool {
        AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .is_some_and(|profile| !profile.id.is_empty())
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.started {
            return;
        }
        self.started = true;
        self.load_persisted();
        self.start_timer(cx);

        let (sender, mut receiver) = mpsc::unbounded::<()>();
        self.background_observer = Some(RiseAppEvents::default_center().observe(
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
            move |_| {
                let _ = sender.unbounded_send(());
            },
        ));
        self.background_delivery = Some(cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                if this.update(cx, |store, cx| store.on_background(cx)).is_err() {
                    break;
                }
            }
        }));
        self.app_quit_subscription = Some(cx.on_app_quit(|store, cx| {
            let write = store.persist_now(cx);
            store.flush(cx);
            async move {
                if let Some(write) = write {
                    write.await;
                }
            }
        }));
    }

    pub fn enqueue(&mut self, event: EngagementEvent, dedup_key: Option<String>, cx: &mut Context<Self>) {
        if !Self::is_authorized(cx) {
            return;
        }
        if let Some(dedup_key) = dedup_key {
            if self.seen_impressions.contains(&dedup_key) {
                return;
            }
            self.seen_impressions.insert(dedup_key);
        }
        self.queue.push(event);
        if self.queue.len() > self.queue_cap {
            let overflow = self.queue.len() - self.queue_cap;
            self.queue.drain(..overflow);
        }
        self.schedule_persist(cx);
        if self.queue.len() >= self.flush_threshold {
            self.flush(cx);
        }
    }

    pub fn flush(&mut self, cx: &mut Context<Self>) {
        if !Self::is_authorized(cx) || self.is_flushing || self.queue.is_empty() {
            return;
        }

        let generation = self.account_generation;
        let count = self.batch_max.min(self.queue.len());
        let snapshot: Rc<Vec<EngagementEvent>> = Rc::new(self.queue.drain(..count).collect());
        self.is_flushing = true;

        let weak = cx.weak_entity();
        let session_id = self.session_id.clone();
        let completed = Rc::clone(&snapshot);
        let actions = EngagementActionsStore::shared(cx);
        actions.update(cx, |actions, cx| {
            actions.track_events_action(
                &snapshot,
                &session_id,
                move |ok, cx| {
                    let snapshot = Rc::clone(&completed);
                    let _ = weak.update(cx, |store, cx| {
                        store.on_track_events_complete(ok, generation, &snapshot, cx);
                    });
                },
                cx,
            );
        });
    }

    fn on_track_events_complete(
        &mut self,
        ok: bool,
        generation: u64,
        snapshot: &[EngagementEvent],
        cx: &mut Context<Self>,
    ) {
        if self.account_generation != generation {
            return;
        }
        self.is_flushing = false;
        if ok {
            self.in_flight_retry = 0;
        } else {
            self.in_flight_retry += 1;
            if self.in_flight_retry < self.max_retries {
                let tail = std::mem::take(&mut self.queue);
                self.queue = snapshot.iter().cloned().chain(tail).collect();
                if self.queue.len() > self.queue_cap {
                    let overflow = self.queue.len() - self.queue_cap;
                    self.queue.drain(..overflow);
                }
            } else {
                self.in_flight_retry = 0;
            }
        }
        self.schedule_persist(cx);
        if ok && !self.queue.is_empty() {
            self.flush(cx);
        }
    }

    fn on_background(&mut self, cx: &mut Context<Self>) {
        if let Some(write) = self.persist_now(cx) {
            write.detach();
        }
        self.flush(cx);
    }

    fn start_timer(&mut self, cx: &mut Context<Self>) {
        self.timer_task = None;
        let interval = Duration::from_secs(self.flush_interval_sec);
        self.timer_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                if this.update(cx, |store, cx| store.flush(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn schedule_persist(&mut self, cx: &mut Context<Self>) {
        if self.persist_pending {
            return;
        }
        self.persist_pending = true;
        self.persist_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Self::PERSIST_COALESCE_WINDOW).await;
            let _ = this.update(cx, |store, cx| {
                store.persist_pending = false;
                if let Some(write) = store.persist(cx) {
                    write.detach();
                }
            });
        }));
    }

    fn persist_now(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        self.persist_task = None;
        self.persist_pending = false;
        self.persist(cx)
    }

    fn persist(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let url = self.persist_url(account_user_id.as_deref())?;
        let snapshot = self.queue.clone();
        let ticket = self.next_persist_ticket();
        Some(cx.background_executor().spawn(async move {
            if !SaiAccountCacheScope::is_current_account_epoch(
                account_user_id.as_deref(),
                account_epoch.as_deref(),
            ) {
                return;
            }
            PERSISTENCE_QUEUE.with(|last_ticket| {
                if ticket < *last_ticket {
                    return;
                }
                *last_ticket = ticket;
                Self::write_queue(&url, &snapshot);
            });
        }))
    }

    fn persist_synchronously(&mut self) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let Some(url) = self.persist_url(account_user_id.as_deref()) else {
            return;
        };
        let snapshot = self.queue.clone();
        let ticket = self.next_persist_ticket();
        PERSISTENCE_QUEUE.with(|last_ticket| {
            *last_ticket = ticket;
            if snapshot.is_empty() {
                FileManager.remove_item(&url).ok();
                return;
            }
            Self::write_queue(&url, &snapshot);
        });
    }

    fn next_persist_ticket(&mut self) -> u64 {
        self.persist_ticket = self.persist_ticket.wrapping_add(1);
        self.persist_ticket
    }

    fn write_queue(url: &Path, snapshot: &[EngagementEvent]) {
        if let Some(directory) = url.parent() {
            FileManager.create_directory(directory, true).ok();
        }
        if let Ok(data) = serde_json::to_vec(snapshot) {
            Self::write_atomically(url, &data).ok();
        }
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = url.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, url).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }

    fn load_persisted(&mut self) {
        let Some(url) = self.persist_url(SaiAccountCacheScope::active_user_id().as_deref()) else {
            return;
        };
        let Ok(data) = std::fs::read(&url) else {
            return;
        };
        let Ok(mut saved) = serde_json::from_slice::<Vec<EngagementEvent>>(&data) else {
            return;
        };
        let start = saved.len().saturating_sub(self.queue_cap);
        saved.drain(..start);
        self.queue = saved;
    }
}
