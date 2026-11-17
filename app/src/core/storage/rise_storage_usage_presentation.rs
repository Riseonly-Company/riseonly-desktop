use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::storage::rise_storage_maintenance::RiseStorageMaintenance;
use crate::core::storage::rise_storage_scanner::RiseStorageScanner;
use crate::core::storage::rise_storage_usage_models::{
    RiseStorageChatUsage, RiseStorageMediaEntry, RiseStorageSite, RiseStorageUsageSnapshot,
};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

/// a stored walk, the gpui task that installs the result plus the flag the blocking walk polls
/// dropping it cancels both
struct RiseStorageUsageTask {
    _task: Task<()>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for RiseStorageUsageTask {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// immutable storage snapshots for the settings screens
/// each scan keeps the account and generation it started with
/// a walk of thousands of files outlives its screen so a stale result must be dropped
pub struct RiseStorageUsagePresentation {
    snapshot: RiseStorageUsageSnapshot,
    status: SaiWsSliceStatus,
    is_clearing: bool,

    chats: Vec<RiseStorageChatUsage>,
    chats_status: SaiWsSliceStatus,

    media: Vec<RiseStorageMediaEntry>,
    media_total_count: usize,
    media_status: SaiWsSliceStatus,

    last_clear_freed_bytes: i64,
    clear_completion_token: u64,

    generation: u64,
    scan_task: Option<RiseStorageUsageTask>,
    chats_task: Option<RiseStorageUsageTask>,
    media_task: Option<RiseStorageUsageTask>,
    last_chat_titles: HashMap<String, String>,
    chat_clear_id: Option<Uuid>,
}

impl RiseStorageUsagePresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseStorageUsageSnapshot::empty(),
            status: SaiWsSliceStatus::Idle,
            is_clearing: false,
            chats: Vec::new(),
            chats_status: SaiWsSliceStatus::Idle,
            media: Vec::new(),
            media_total_count: 0,
            media_status: SaiWsSliceStatus::Idle,
            last_clear_freed_bytes: 0,
            clear_completion_token: 0,
            generation: 0,
            scan_task: None,
            chats_task: None,
            media_task: None,
            last_chat_titles: HashMap::new(),
            chat_clear_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseStorageUsageSnapshot {
        &self.snapshot
    }

    pub fn status(&self) -> SaiWsSliceStatus {
        self.status
    }

    pub fn is_clearing(&self) -> bool {
        self.is_clearing
    }

    pub fn chats(&self) -> &[RiseStorageChatUsage] {
        &self.chats
    }

    pub fn chats_status(&self) -> SaiWsSliceStatus {
        self.chats_status
    }

    pub fn media(&self) -> &[RiseStorageMediaEntry] {
        &self.media
    }

    pub fn media_total_count(&self) -> usize {
        self.media_total_count
    }

    pub fn media_status(&self) -> SaiWsSliceStatus {
        self.media_status
    }

    pub fn last_clear_freed_bytes(&self) -> i64 {
        self.last_clear_freed_bytes
    }

    pub fn clear_completion_token(&self) -> u64 {
        self.clear_completion_token
    }

    /// message history lives in the engine database and not in a cache folder
    /// so per chat bytes come from the active conversation domain
    async fn cached_history_bytes(cx: &mut AsyncApp) -> HashMap<String, i64> {
        let domain = cx.update(|cx| {
            let registry = RiseConversationDomainRegistry::shared(cx);
            registry.read(cx).active()
        });
        let Some(domain) = domain else {
            return HashMap::new();
        };
        domain.cached_message_history_bytes().await.unwrap_or_default()
    }

    /// drops cached history of the given chats, all cached chats when none
    /// returns the bytes it held
    async fn evict_cached_history(chat_ids: Option<Vec<String>>, cx: &mut AsyncApp) -> i64 {
        if chat_ids.as_ref().is_some_and(Vec::is_empty) {
            return 0;
        }
        let domain = cx.update(|cx| {
            let registry = RiseConversationDomainRegistry::shared(cx);
            registry.read(cx).active()
        });
        let Some(domain) = domain else {
            return 0;
        };
        let freed = domain.evict_cached_message_history(chat_ids.as_deref()).await.unwrap_or(0);
        cx.update(|cx| MessageServicesStore::shared(cx).update(cx, |store, cx| store.note_history_evicted(cx)));
        freed
    }

    pub fn account_user_id(&self) -> Option<String> {
        SaiAccountCacheScope::active_user_id()
    }

    /// old snapshot stays on screen while a new walk runs
    /// a rescan is a refresh and shouldnt bring back the loader
    pub fn refresh(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.is_clearing {
            return;
        }
        let account_key = self.account_user_id().unwrap_or_default();
        if !force && self.status == SaiWsSliceStatus::Fulfilled && self.snapshot.account_key == account_key {
            return;
        }

        self.scan_task = None;
        let expected = self.begin_generation();
        if self.status != SaiWsSliceStatus::Fulfilled || self.snapshot.account_key != account_key {
            self.status = SaiWsSliceStatus::Pending;
        }
        let account = self.account_user_id();
        let cancelled = Arc::new(AtomicBool::new(false));
        let walk_cancelled = Arc::clone(&cancelled);
        let task = cx.spawn(async move |this, cx| {
            let work = RiseAccountRuntime::current()
                .spawn_blocking(move || RiseStorageScanner::shared().scan(account.as_deref(), &walk_cancelled));
            let Ok(result) = work.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.install(result, expected, &account_key, cx));
        });
        self.scan_task = Some(RiseStorageUsageTask {
            _task: task,
            cancelled,
        });
        cx.notify();
    }

    pub fn refresh_chats(&mut self, titles: HashMap<String, String>, cx: &mut Context<Self>) {
        if self.is_clearing {
            return;
        }
        self.last_chat_titles = titles.clone();
        self.chats_task = None;
        let expected = self.generation;
        if self.chats.is_empty() {
            self.chats_status = SaiWsSliceStatus::Pending;
        }
        let account = self.account_user_id();
        let cancelled = Arc::new(AtomicBool::new(false));
        let walk_cancelled = Arc::clone(&cancelled);
        let task = cx.spawn(async move |this, cx| {
            let history = Self::cached_history_bytes(cx).await;
            let work = RiseAccountRuntime::current().spawn_blocking(move || {
                RiseStorageScanner::shared().chat_usage(&titles, &history, account.as_deref(), &walk_cancelled)
            });
            let Ok(rows) = work.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.install_chats(rows, expected, cx));
        });
        self.chats_task = Some(RiseStorageUsageTask {
            _task: task,
            cancelled,
        });
        cx.notify();
    }

    pub fn refresh_media(&mut self, cx: &mut Context<Self>) {
        self.media_task = None;
        let expected = self.generation;
        if self.media.is_empty() {
            self.media_status = SaiWsSliceStatus::Pending;
        }
        let account = self.account_user_id();
        let task = cx.spawn(async move |this, cx| {
            let work = RiseAccountRuntime::current()
                .spawn_blocking(move || RiseStorageScanner::shared().media_entries(account.as_deref()));
            let Ok(result) = work.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.install_media(result, expected, cx));
        });
        self.media_task = Some(RiseStorageUsageTask {
            _task: task,
            cancelled: Arc::new(AtomicBool::new(false)),
        });
        cx.notify();
    }

    /// purges and then rescans inline so the freed figure is measured
    /// not the number the screen showed at the tap
    /// callers detach the task, a purge never stops halfway
    pub fn clear(&mut self, sites: HashSet<RiseStorageSite>, cx: &mut Context<Self>) -> Task<()> {
        if self.is_clearing {
            return Task::ready(());
        }
        self.is_clearing = true;
        let account = self.account_user_id();
        let account_key = account.clone().unwrap_or_default();
        let before = self.snapshot.clearable_bytes();
        cx.notify();

        let purge = RiseStorageMaintenance::clear(&sites, account.as_deref(), cx);
        cx.spawn(async move |this, cx| {
            purge.await;
            let mut evicted_history: i64 = 0;
            if sites.contains(&RiseStorageSite::WsPayloads) {
                evicted_history = Self::evict_cached_history(None, cx).await;
            }

            let Ok(expected) = this.update(cx, |this, _cx| {
                this.scan_task = None;
                this.begin_generation()
            }) else {
                return;
            };
            let work = RiseAccountRuntime::current().spawn_blocking(move || {
                RiseStorageScanner::shared().scan(account.as_deref(), &AtomicBool::new(false))
            });
            let result = work.await.ok();

            let _ = this.update(cx, |this, cx| {
                if let Some(result) = result {
                    this.install(result, expected, &account_key, cx);
                }
                this.is_clearing = false;
                this.last_clear_freed_bytes = (before - this.snapshot.clearable_bytes()).max(0) + evicted_history;
                this.clear_completion_token = this.clear_completion_token.wrapping_add(1);

                if this.chats_status != SaiWsSliceStatus::Idle {
                    let titles = this.last_chat_titles.clone();
                    this.refresh_chats(titles, cx);
                }
                if this.media_status != SaiWsSliceStatus::Idle {
                    this.refresh_media(cx);
                }
                cx.notify();
            });
        })
    }

    pub fn clear_everything(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let sites: HashSet<RiseStorageSite> =
            RiseStorageSite::ALL_CASES.into_iter().filter(RiseStorageSite::is_clearable).collect();
        self.clear(sites, cx)
    }

    /// callers detach the task, same as clear
    pub fn clear_chat(&mut self, chat_id: String, cx: &mut Context<Self>) -> Task<()> {
        if self.is_clearing {
            return Task::ready(());
        }
        let clear_id = Uuid::new_v4();
        self.chat_clear_id = Some(clear_id);
        self.is_clearing = true;
        let account = self.account_user_id();
        let account_key = account.clone().unwrap_or_default();
        let titles = self.last_chat_titles.clone();
        let before = self.snapshot.clearable_bytes();
        self.scan_task = None;
        self.chats_task = None;
        self.begin_generation();
        cx.notify();

        let purge = RiseStorageMaintenance::clear_chat(&chat_id, account.as_deref(), cx);
        cx.spawn(async move |this, cx| {
            let measured = async {
                purge.await;
                let evicted_history = Self::evict_cached_history(Some(vec![chat_id]), cx).await;
                let still_current = this.read_with(cx, |this, _cx| this.chat_clear_id == Some(clear_id)).unwrap_or(false)
                    && SaiAccountCacheScope::active_user_id().unwrap_or_default() == account_key;
                if !still_current {
                    return None;
                }
                let scan_account = account.clone();
                let result = RiseAccountRuntime::current()
                    .spawn_blocking(move || {
                        RiseStorageScanner::shared().scan(scan_account.as_deref(), &AtomicBool::new(false))
                    })
                    .await
                    .ok()?;
                let history = Self::cached_history_bytes(cx).await;
                let rows = RiseAccountRuntime::current()
                    .spawn_blocking(move || {
                        RiseStorageScanner::shared().chat_usage(
                            &titles,
                            &history,
                            account.as_deref(),
                            &AtomicBool::new(false),
                        )
                    })
                    .await
                    .ok()?;
                Some((result, rows, evicted_history))
            }
            .await;

            let _ = this.update(cx, |this, cx| {
                let still_current = this.chat_clear_id == Some(clear_id)
                    && this.account_user_id().unwrap_or_default() == account_key;
                if let (Some((result, rows, evicted_history)), true) = (measured, still_current) {
                    this.scan_task = None;
                    this.chats_task = None;
                    this.begin_generation();
                    this.last_clear_freed_bytes = (before - result.clearable_bytes()).max(0) + evicted_history;
                    this.snapshot = result;
                    this.status = SaiWsSliceStatus::Fulfilled;
                    this.chats = rows;
                    this.chats_status = SaiWsSliceStatus::Fulfilled;
                    this.clear_completion_token = this.clear_completion_token.wrapping_add(1);
                    if this.media_status != SaiWsSliceStatus::Idle {
                        this.refresh_media(cx);
                    }
                }
                if this.chat_clear_id == Some(clear_id) {
                    this.chat_clear_id = None;
                    this.is_clearing = false;
                }
                cx.notify();
            });
        })
    }

    pub fn clear_media_entry(&mut self, entry: RiseStorageMediaEntry, cx: &mut Context<Self>) -> Task<()> {
        if self.is_clearing {
            return Task::ready(());
        }
        self.is_clearing = true;
        cx.notify();
        let account = self.account_user_id();
        let purge = RiseStorageMaintenance::clear_media_entry(&entry, account.as_deref(), cx);
        cx.spawn(async move |this, cx| {
            purge.await;
            let _ = this.update(cx, |this, cx| {
                this.media.retain(|item| item.id != entry.id);
                this.media_total_count = this.media_total_count.saturating_sub(1);
                this.is_clearing = false;
                this.refresh(true, cx);
                cx.notify();
            });
        })
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.begin_generation();
        self.scan_task = None;
        self.chats_task = None;
        self.media_task = None;
        self.snapshot = RiseStorageUsageSnapshot::empty();
        self.status = SaiWsSliceStatus::Idle;
        self.chats = Vec::new();
        self.chats_status = SaiWsSliceStatus::Idle;
        self.media = Vec::new();
        self.media_total_count = 0;
        self.media_status = SaiWsSliceStatus::Idle;
        self.is_clearing = false;
        self.chat_clear_id = None;
        self.last_chat_titles = HashMap::new();
        self.last_clear_freed_bytes = 0;
        cx.notify();
    }

    fn begin_generation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn install(&mut self, result: RiseStorageUsageSnapshot, expected: u64, account_key: &str, cx: &mut Context<Self>) {
        if self.generation != expected || self.account_user_id().unwrap_or_default() != account_key {
            return;
        }
        self.snapshot = result;
        self.status = SaiWsSliceStatus::Fulfilled;
        cx.notify();
    }

    fn install_chats(&mut self, rows: Vec<RiseStorageChatUsage>, expected: u64, cx: &mut Context<Self>) {
        if self.generation != expected {
            return;
        }
        self.chats = rows;
        self.chats_status = SaiWsSliceStatus::Fulfilled;
        cx.notify();
    }

    fn install_media(&mut self, result: (Vec<RiseStorageMediaEntry>, usize), expected: u64, cx: &mut Context<Self>) {
        if self.generation != expected {
            return;
        }
        let (entries, total) = result;
        self.media = entries;
        self.media_total_count = total;
        self.media_status = SaiWsSliceStatus::Fulfilled;
        cx.notify();
    }
}
