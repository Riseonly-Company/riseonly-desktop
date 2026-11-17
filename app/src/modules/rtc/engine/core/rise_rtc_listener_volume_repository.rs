use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Weak};

use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};
use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseEngine;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue, RiseStoredValueWritePolicy,
};
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::rtc::engine::core::rise_rtc_store_layout::RiseRTCStoreLayout;
use crate::modules::rtc::engine::rise_rtc_listener_volume_presentation::{
    RiseRTCListenerVolume, RiseRTCListenerVolumePresentation, RiseRTCListenerVolumeSnapshot,
};

const CURRENT_SCHEMA_VERSION: i64 = 1;

static MAXIMUM_ENTRIES: LazyLock<usize> = LazyLock::new(|| {
    usize::try_from(RiseLimits::client_value("rtc_listener_volume_preferences")).unwrap_or(0)
});

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    percent: f64,
    used_at_ms: i64,
}

/// every member is defaulted so an older schema payload decodes and
/// then gets rejected by the version guard, not a failed cache read
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    #[serde(default)]
    schema_version: i64,
    #[serde(default)]
    entries: HashMap<String, Entry>,
}

type LoadTask = Shared<BoxFuture<'static, ()>>;

struct RiseRTCListenerVolumeRepositoryState {
    entries: HashMap<String, Entry>,
    is_loaded: bool,
    is_shutdown: bool,
    load_task: Option<LoadTask>,
    version_clock: i64,
    revision: u64,
}

pub struct RiseRTCListenerVolumeRepository {
    engine: Arc<RiseEngine>,
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    storage_key: String,
    this: Weak<Self>,
    state: RiseIsolated<RiseRTCListenerVolumeRepositoryState>,
}

impl RiseRTCListenerVolumeRepository {
    pub fn maximum_entries() -> usize {
        *MAXIMUM_ENTRIES
    }

    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Arc<Self> {
        assert!(!account_id.is_empty());
        assert!(*MAXIMUM_ENTRIES > 0);
        let storage_key = Self::make_key(&[account_id.as_str(), "listener-volumes"]);
        Arc::new_cyclic(|this| Self {
            engine,
            account_id,
            account_epoch,
            session_id,
            storage_key,
            this: this.clone(),
            state: RiseIsolated::new(RiseRTCListenerVolumeRepositoryState {
                entries: HashMap::new(),
                is_loaded: false,
                is_shutdown: false,
                load_task: None,
                version_clock: 0,
                revision: 0,
            }),
        })
    }

    pub fn current_revision(&self) -> u64 {
        self.state.with(|state| state.revision)
    }

    pub fn overrides(&self) -> HashMap<String, f64> {
        self.state.with(|state| Self::percents(&state.entries))
    }

    pub fn percent(&self, user_id: &str) -> Option<f64> {
        self.state.with(|state| state.entries.get(user_id).map(|entry| entry.percent))
    }

    /// cache first hydrate, deduped
    /// activation and every mutation call it to order against an unfinished load,
    /// a second decode could publish an older map over a newer optimistic one
    pub async fn load(&self) {
        let task = self.state.with(|state| {
            if state.is_shutdown || state.is_loaded {
                return None;
            }
            if let Some(task) = &state.load_task {
                return Some((task.clone(), false));
            }
            let this = self.this.clone();
            let task: LoadTask = async move {
                if let Some(this) = this.upgrade() {
                    this.perform_load().await;
                }
            }
            .boxed()
            .shared();
            state.load_task = Some(task.clone());
            Some((task, true))
        });
        let Some((task, owns_task)) = task else {
            return;
        };
        task.await;
        if owns_task {
            self.state.with(|state| state.load_task = None);
        }
    }

    pub async fn set_percent(&self, percent: Option<f64>, user_id: &str) {
        let user_id = trimming_whitespaces_and_newlines(user_id).to_owned();
        if self.state.with(|state| state.is_shutdown) || user_id.is_empty() {
            return;
        }
        self.load().await;

        let published = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            if let Some(percent) = percent {
                let value = RiseRTCListenerVolume::normalized(percent);
                if let Some(entry) = state.entries.get_mut(&user_id)
                    && entry.percent == value
                {
                    entry.used_at_ms = Self::now_milliseconds();
                    return None;
                }
                state.entries.insert(user_id.clone(), Entry { percent: value, used_at_ms: Self::now_milliseconds() });
                Self::evict_overflow(&mut state.entries);
            } else if state.entries.remove(&user_id).is_none() {
                return None;
            }
            state.revision = state.revision.wrapping_add(1);
            Some(Self::snapshot(state))
        });
        let Some(snapshot) = published else {
            return;
        };
        self.publish(snapshot);
        self.persist().await;
    }

    pub fn note_used(&self, user_ids: &HashSet<String>) {
        self.state.with(|state| {
            if state.is_shutdown || state.entries.is_empty() {
                return;
            }
            let now = Self::now_milliseconds();
            for user_id in user_ids {
                if let Some(entry) = state.entries.get_mut(user_id) {
                    entry.used_at_ms = now;
                }
            }
        });
    }

    pub fn shutdown(&self) {
        self.state.with(|state| {
            state.is_shutdown = true;
            state.load_task = None;
            state.entries = HashMap::new();
        });
    }

    async fn perform_load(&self) {
        if self.state.with(|state| state.is_shutdown || state.is_loaded) {
            return;
        }
        let stored = self
            .engine
            .value(RiseRTCStoreLayout::listener_volumes(), self.storage_key.clone())
            .await
            .ok()
            .flatten();
        if self.state.with(|state| state.is_shutdown || state.is_loaded) {
            return;
        }
        let decoded = stored.as_ref().map(|stored| (stored.version, Self::decode(&stored.data)));
        let published = self.state.with(|state| {
            if state.is_shutdown || state.is_loaded {
                return None;
            }
            if let Some((version, decoded)) = decoded {
                state.version_clock = state.version_clock.max(version);
                for (user_id, entry) in decoded {
                    state.entries.entry(user_id).or_insert(entry);
                }
                Self::evict_overflow(&mut state.entries);
            }
            state.is_loaded = true;
            state.revision = state.revision.wrapping_add(1);
            Some(Self::snapshot(state))
        });
        if let Some(snapshot) = published {
            self.publish(snapshot);
        }
    }

    fn evict_overflow(entries: &mut HashMap<String, Entry>) {
        let maximum = *MAXIMUM_ENTRIES;
        if entries.len() <= maximum {
            return;
        }
        let mut by_use: Vec<(String, i64)> =
            entries.iter().map(|(user_id, entry)| (user_id.clone(), entry.used_at_ms)).collect();
        by_use.sort_by_key(|(_, used_at_ms)| *used_at_ms);
        let overflow = entries.len() - maximum;
        for (user_id, _) in by_use.into_iter().take(overflow) {
            entries.remove(&user_id);
        }
    }

    fn snapshot(state: &RiseRTCListenerVolumeRepositoryState) -> RiseRTCListenerVolumeSnapshot {
        RiseRTCListenerVolumeSnapshot { overrides: Self::percents(&state.entries), is_loaded: state.is_loaded }
    }

    fn percents(entries: &HashMap<String, Entry>) -> HashMap<String, f64> {
        entries.iter().map(|(user_id, entry)| (user_id.clone(), entry.percent)).collect()
    }

    fn publish(&self, next: RiseRTCListenerVolumeSnapshot) {
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        main.post(move |cx| {
            RiseRTCListenerVolumePresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(next, &account_id, &account_epoch, session_id, cx);
            });
        });
    }

    async fn persist(&self) {
        let prepared = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
            Some((state.entries.clone(), state.version_clock))
        });
        let Some((snapshot, version)) = prepared else {
            return;
        };
        let data = Self::encode(&Envelope { schema_version: CURRENT_SCHEMA_VERSION, entries: snapshot });
        let Some(data) = data else {
            return;
        };
        if self.state.with(|state| state.is_shutdown) {
            return;
        }
        let _ = self
            .engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: RiseRTCStoreLayout::listener_volumes(),
                    key: self.storage_key.clone(),
                    value: RiseStoredValue::new(data, version, Self::now_milliseconds()),
                    precondition: RiseStoreVersionPrecondition::Any,
                    policy: RiseStoredValueWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await;
    }

    fn encode(envelope: &Envelope) -> Option<Vec<u8>> {
        RiseRTCStoreLayout::encode(envelope).ok()
    }

    /// a schema mismatch gives an empty map not an error, the listener
    /// falls back to derived defaults instead of losing the room audio
    fn decode(data: &[u8]) -> HashMap<String, Entry> {
        let Ok(envelope) = serde_json::from_slice::<Envelope>(data) else {
            return HashMap::new();
        };
        if envelope.schema_version != CURRENT_SCHEMA_VERSION {
            return HashMap::new();
        }
        envelope
            .entries
            .into_iter()
            .filter(|(_, entry)| entry.percent.is_finite())
            .map(|(user_id, entry)| {
                (
                    user_id,
                    Entry {
                        percent: RiseRTCListenerVolume::normalized(entry.percent),
                        used_at_ms: entry.used_at_ms.max(0),
                    },
                )
            })
            .collect()
    }

    fn make_key(values: &[&str]) -> String {
        values.iter().map(|value| format!("{}:{}", value.len(), value)).collect::<Vec<_>>().join("|")
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
