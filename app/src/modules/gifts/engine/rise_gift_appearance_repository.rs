use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::AsyncApp;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::user_defaults::UserDefaults;
use tokio::task::AbortHandle;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::gifts::components::gift_cosmetic_files::GiftCosmeticFiles;
use crate::modules::gifts::engine::core::rise_gifts_rpc::RiseGiftsRPC;
use crate::modules::gifts::engine::rise_gift_appearance_presentation::RiseGiftAppearancePresentation;
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftAppearanceDTO, RiseGiftAppearanceExt, RiseGiftItemKind, RiseGiftWornDTO,
};

struct Entry {
    appearance: RiseGiftAppearanceDTO,
    read_at: Duration,
    stale: bool,
}

struct PendingRead {
    read: u64,
    revision: u64,
    task: Shared<BoxFuture<'static, ()>>,
    abort: AbortHandle,
}

struct RiseGiftAppearanceRepositoryState {
    entries: HashMap<String, Entry>,
    failed_at: HashMap<String, Duration>,
    pending: HashMap<String, PendingRead>,
    revisions: HashMap<String, u64>,
    next_read: u64,
    is_shutdown: bool,
}

pub struct RiseGiftAppearanceRepository {
    rpc: RiseGiftsRPC,
    account_id: String,
    account_epoch: String,
    this: Weak<RiseGiftAppearanceRepository>,
    state: RiseIsolated<RiseGiftAppearanceRepositoryState>,
}

impl RiseGiftAppearanceRepository {
    pub const FRESHNESS: Duration = Duration::from_millis(300_000);
    pub const FORCED_REUSE: Duration = Duration::from_secs(5);
    pub const FAILURE_BACKOFF: Duration = Duration::from_secs(30);
    pub const ENTRY_LIMIT: usize = 2_048;
    const OWN_DEFAULTS_KEY: &'static str = "gifts.own_appearance.v1";

    pub fn new(rpc: RiseGiftsRPC, account_id: String, account_epoch: String) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            rpc,
            account_id,
            account_epoch,
            this: this.clone(),
            state: RiseIsolated::new(RiseGiftAppearanceRepositoryState {
                entries: HashMap::new(),
                failed_at: HashMap::new(),
                pending: HashMap::new(),
                revisions: HashMap::new(),
                next_read: 0,
                is_shutdown: false,
            }),
        })
    }

    /// reads whoever is unknown or stale or past the window and joins reads already under way
    /// returns once every person asked has an answer or a failure
    pub async fn load(&self, user_ids: &[String], force: bool) {
        let planned = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            let now = RiseClock::monotonic();
            let mut known: HashMap<String, RiseGiftAppearanceDTO> = HashMap::new();
            let mut waits: Vec<Shared<BoxFuture<'static, ()>>> = Vec::new();
            let mut missing: Vec<String> = Vec::new();
            let mut seen: HashSet<&str> = HashSet::new();
            for id in user_ids {
                if id.is_empty() || !seen.insert(id.as_str()) {
                    continue;
                }
                let revision = state.revisions.get(id).copied().unwrap_or(0);
                if let Some(under_way) = state.pending.get(id)
                    && under_way.revision == revision
                {
                    waits.push(under_way.task.clone());
                    continue;
                }
                let window = if force { Self::FORCED_REUSE } else { Self::FRESHNESS };
                if let Some(entry) = state.entries.get(id)
                    && !entry.stale
                    && now.saturating_sub(entry.read_at) < window
                {
                    known.insert(id.clone(), entry.appearance.clone());
                    continue;
                }
                if !force
                    && let Some(failed) = state.failed_at.get(id)
                    && now.saturating_sub(*failed) < Self::FAILURE_BACKOFF
                {
                    continue;
                }
                missing.push(id.clone());
            }
            let batch = RiseGiftsRPC::appearance_batch_max().max(1);
            for chunk in missing.chunks(batch) {
                waits.push(self.read(state, chunk.to_vec()));
            }
            Some((known, waits))
        });
        let Some((known, waits)) = planned else {
            return;
        };
        self.publish(known).await;
        for task in waits {
            task.await;
        }
    }

    pub async fn warm_own(&self) {
        let restored = self.state.with(|state| {
            if state.is_shutdown {
                return Err(());
            }
            if state.entries.contains_key(&self.account_id) {
                return Ok(None);
            }
            let Some(saved) = Self::saved_own(&self.account_id) else {
                return Ok(None);
            };
            state.entries.insert(
                self.account_id.clone(),
                Entry {
                    appearance: saved.clone(),
                    read_at: RiseClock::monotonic(),
                    stale: true,
                },
            );
            Ok(Some(saved))
        });
        let Ok(restored) = restored else {
            return;
        };
        if let Some(saved) = restored {
            self.publish(HashMap::from([(self.account_id.clone(), saved.clone())])).await;
            self.warm_files(saved);
        }
        self.load(std::slice::from_ref(&self.account_id), true).await;
    }

    /// the account is being removed from this machine
    pub fn remove_persisted_account(user_id: &str) {
        UserDefaults::standard().remove_object(&Self::own_defaults_key(user_id));
    }

    /// what one person wears, read first when needed
    /// none when nothing is known and the read failed
    pub async fn resolve(&self, user_id: &str, force: bool) -> Option<RiseGiftAppearanceDTO> {
        self.load(&[user_id.to_owned()], force).await;
        self.state
            .with(|state| state.entries.get(user_id).map(|entry| entry.appearance.clone()))
    }

    pub async fn apply_equipped(
        &self,
        user_id: &str,
        worn: RiseGiftWornDTO,
        kind: RiseGiftItemKind,
        equipped: bool,
    ) {
        let next = self.state.with(|state| {
            if state.is_shutdown || !kind.equippable() {
                return None;
            }
            let current = state
                .entries
                .get(user_id)
                .map(|entry| entry.appearance.clone())
                .unwrap_or_else(|| RiseGiftAppearanceDTO::nothing(user_id));
            let next = if equipped {
                current.replacing(kind, Some(worn))
            } else if current.worn(kind).map(|worn| worn.instance_id.as_str())
                == Some(worn.instance_id.as_str())
            {
                current.replacing(kind, None)
            } else {
                current
            };
            let revision = state.revisions.entry(user_id.to_owned()).or_insert(0);
            *revision = revision.wrapping_add(1);
            state.entries.insert(
                user_id.to_owned(),
                Entry {
                    appearance: next.clone(),
                    read_at: RiseClock::monotonic(),
                    stale: true,
                },
            );
            Some(next)
        });
        let Some(next) = next else {
            return;
        };
        if user_id == self.account_id {
            self.keep_own(&next);
        }
        self.publish(HashMap::from([(user_id.to_owned(), next)])).await;
    }

    /// someone look changed elsewhere, what is shown stays until the next load replaces it
    /// and that load cant be answered from memory
    pub async fn invalidate(&self, user_id: &str) {
        self.state.with(|state| {
            if state.is_shutdown {
                return;
            }
            let revision = state.revisions.entry(user_id.to_owned()).or_insert(0);
            *revision = revision.wrapping_add(1);
            if let Some(entry) = state.entries.get_mut(user_id) {
                entry.stale = true;
            }
        });
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| {
            state.is_shutdown = true;
            for read in state.pending.values() {
                read.abort.abort();
            }
            state.pending.clear();
            state.entries.clear();
            state.failed_at.clear();
            state.revisions.clear();
        });
    }

    fn read(
        &self,
        state: &mut RiseGiftAppearanceRepositoryState,
        chunk: Vec<String>,
    ) -> Shared<BoxFuture<'static, ()>> {
        state.next_read = state.next_read.wrapping_add(1);
        let read = state.next_read;
        let mut stamps: HashMap<String, u64> = HashMap::new();
        for id in &chunk {
            stamps
                .entry(id.clone())
                .or_insert_with(|| state.revisions.get(id).copied().unwrap_or(0));
        }
        let rpc = self.rpc.clone();
        let this = self.this.clone();
        let reading = chunk.clone();
        let finishing = stamps.clone();
        let handle = RiseAccountRuntime::current().spawn(async move {
            let items = rpc.appearances(&reading).await.ok();
            if let Some(this) = this.upgrade() {
                this.finish(read, reading, finishing, items).await;
            }
        });
        let abort = handle.abort_handle();
        let task = async move {
            let _ = handle.await;
        }
        .boxed()
        .shared();
        for id in chunk {
            let revision = stamps.get(&id).copied().unwrap_or(0);
            state.pending.insert(
                id,
                PendingRead {
                    read,
                    revision,
                    task: task.clone(),
                    abort: abort.clone(),
                },
            );
        }
        task
    }

    async fn finish(
        &self,
        read: u64,
        chunk: Vec<String>,
        stamps: HashMap<String, u64>,
        items: Option<Vec<RiseGiftAppearanceDTO>>,
    ) {
        let answered = self.state.with(|state| {
            for id in &chunk {
                if state.pending.get(id).map(|pending| pending.read) == Some(read) {
                    state.pending.remove(id);
                }
            }
            if state.is_shutdown {
                return None;
            }
            let now = RiseClock::monotonic();
            let mut listed: HashMap<String, RiseGiftAppearanceDTO> = HashMap::new();
            for item in items.iter().flatten() {
                listed.insert(item.user_id.clone(), item.clone());
            }
            let mut answered: HashMap<String, RiseGiftAppearanceDTO> = HashMap::new();
            for id in &chunk {
                if stamps.get(id).copied() != Some(state.revisions.get(id).copied().unwrap_or(0)) {
                    continue;
                }
                if items.is_none() {
                    state.failed_at.insert(id.clone(), now);
                    continue;
                }
                state.failed_at.remove(id);
                let appearance = listed
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| RiseGiftAppearanceDTO::nothing(id.clone()));
                state.entries.insert(
                    id.clone(),
                    Entry {
                        appearance: appearance.clone(),
                        read_at: now,
                        stale: false,
                    },
                );
                answered.insert(id.clone(), appearance);
            }
            Self::trim(state, &self.account_id, now);
            Some(answered)
        });
        let Some(answered) = answered else {
            return;
        };
        if let Some(own) = answered.get(&self.account_id) {
            self.keep_own(own);
        }
        self.publish(answered).await;
    }

    fn own_defaults_key(user_id: &str) -> String {
        SaiAccountCacheScope::prefix(user_id) + Self::OWN_DEFAULTS_KEY
    }

    fn saved_own(account_id: &str) -> Option<RiseGiftAppearanceDTO> {
        let data = UserDefaults::standard().data(&Self::own_defaults_key(account_id))?;
        serde_json::from_slice(&data).ok()
    }

    /// the viewer look on disk and its files warmed, nothing worn leaves nothing behind
    fn keep_own(&self, appearance: &RiseGiftAppearanceDTO) {
        let key = Self::own_defaults_key(&self.account_id);
        if appearance.wears_nothing() {
            UserDefaults::standard().remove_object(&key);
            return;
        }
        if let Ok(data) = serde_json::to_vec(appearance) {
            UserDefaults::standard().set(&key, data);
        }
        self.warm_files(appearance.clone());
    }

    fn warm_files(&self, appearance: RiseGiftAppearanceDTO) {
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        main.post(move |cx| {
            GiftCosmeticFiles::warm(&appearance, Some(account_id), Some(account_epoch), cx);
        });
    }

    async fn publish(&self, values: HashMap<String, RiseGiftAppearanceDTO>) {
        if values.is_empty() {
            return;
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
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
                        RiseGiftAppearancePresentation::shared(cx)
                            .update(cx, |presentation, cx| presentation.apply(values, cx));
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    /// oldest reads go first, the viewer own entry stays
    fn trim(state: &mut RiseGiftAppearanceRepositoryState, account_id: &str, now: Duration) {
        if state.failed_at.len() > Self::ENTRY_LIMIT {
            state
                .failed_at
                .retain(|_, failed| now.saturating_sub(*failed) < Self::FAILURE_BACKOFF);
        }
        if state.entries.len() <= Self::ENTRY_LIMIT {
            return;
        }
        let mut candidates: Vec<(&String, Duration)> = state
            .entries
            .iter()
            .filter(|(id, _)| id.as_str() != account_id)
            .map(|(id, entry)| (id, entry.read_at))
            .collect();
        candidates.sort_by_key(|(_, read_at)| *read_at);
        let evicted: Vec<String> = candidates
            .into_iter()
            .take(state.entries.len() - Self::ENTRY_LIMIT * 3 / 4)
            .map(|(id, _)| id.clone())
            .collect();
        for id in evicted {
            state.entries.remove(&id);
        }
    }
}
