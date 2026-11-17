use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, Global, Subscription, Task};
use rise_platform::process_activity::begin_process_activity;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::{SaiWsDiskCache, SaiWsDiskCacheError};

#[derive(Clone, Debug, PartialEq)]
pub struct ChatScrollPositionStorePosition {
    pub anchor_id: Option<String>,
    pub viewport_offset: f32,
    pub distance_from_top: f32,
}

impl ChatScrollPositionStorePosition {
    pub fn new(anchor_id: Option<String>, viewport_offset: f32, distance_from_top: f32) -> Self {
        Self {
            anchor_id: anchor_id.filter(|anchor_id| !anchor_id.is_empty()),
            viewport_offset: if viewport_offset.is_finite() { viewport_offset } else { 0.0 },
            distance_from_top: if distance_from_top.is_finite() { distance_from_top.max(0.0) } else { 0.0 },
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedChatScrollPosition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anchor_id: Option<Value>,
    #[serde(default)]
    viewport_offset: Option<Value>,
    distance_from_top: Value,
}

#[derive(Clone, Debug)]
struct PendingDiskSnapshot {
    token: u64,
    positions: HashMap<String, ChatScrollPositionStorePosition>,
}

#[derive(Default)]
struct ChatScrollPositionState {
    loaded_disk_cache_key: Option<String>,
    positions_by_chat_id: HashMap<String, ChatScrollPositionStorePosition>,
    persistence_generation: u64,
    persisted_generation: u64,
    persistence_in_flight_generation: Option<u64>,
    persistence_task: Option<Task<()>>,
    disk_snapshot_token: u64,
    pending_disk_snapshots: HashMap<String, PendingDiskSnapshot>,
}

static STATE: LazyLock<Mutex<ChatScrollPositionState>> = LazyLock::new(|| Mutex::new(ChatScrollPositionState::default()));

const LOGICAL_DISK_CACHE_KEY: &str = "chatScrollPositions";
const SAFETY_CHECKPOINT_DELAY: Duration = Duration::from_nanos(3_000_000_000);
const FAILED_WRITE_RETRY_DELAY: Duration = Duration::from_nanos(2_000_000_000);

struct ChatScrollPositionExitObserver {
    _app_quit: Subscription,
}

impl Global for ChatScrollPositionExitObserver {}

pub enum ChatScrollPositionStore {}

impl ChatScrollPositionStore {
    pub const NEAR_BOTTOM_THRESHOLD: f32 = 100.0;

    pub fn saved_position(chat_id: &str, cx: &mut App) -> Option<ChatScrollPositionStorePosition> {
        Self::activate_account_state(cx);
        if chat_id.is_empty() {
            return None;
        }
        let state = STATE.lock();
        let position = state.positions_by_chat_id.get(chat_id)?;
        if position.anchor_id.is_none() && position.distance_from_top <= 1.0 {
            return None;
        }
        Some(position.clone())
    }

    pub fn saved_offset(chat_id: &str, cx: &mut App) -> Option<f32> {
        Self::saved_position(chat_id, cx).map(|position| position.distance_from_top)
    }

    pub fn save_offset(offset_y: f32, chat_id: &str, is_near_bottom: bool, cx: &mut App) {
        Self::save_position(
            ChatScrollPositionStorePosition::new(None, 0.0, offset_y),
            chat_id,
            is_near_bottom,
            cx,
        );
    }

    #[track_caller]
    pub fn save_position(position: ChatScrollPositionStorePosition, chat_id: &str, is_near_bottom: bool, cx: &mut App) {
        tracing::warn!(target: "riseonly::X696", "save key={chat_id} anchor={:?} top={} near_bottom={is_near_bottom} at={}", position.anchor_id, position.distance_from_top, std::panic::Location::caller());
        Self::activate_account_state(cx);
        if chat_id.is_empty() {
            return;
        }
        let did_change = {
            let mut state = STATE.lock();
            if is_near_bottom {
                state.positions_by_chat_id.remove(chat_id).is_some()
            } else if state.positions_by_chat_id.get(chat_id) != Some(&position) {
                state.positions_by_chat_id.insert(chat_id.to_owned(), position);
                true
            } else {
                false
            }
        };
        if did_change {
            Self::mark_pending_persistence(cx);
        }
    }

    #[track_caller]
    pub fn clear(chat_id: &str, cx: &mut App) {
        tracing::warn!(target: "riseonly::X696", "clear key={chat_id} at={}", std::panic::Location::caller());
        Self::activate_account_state(cx);
        if chat_id.is_empty() {
            return;
        }
        let removed = STATE.lock().positions_by_chat_id.remove(chat_id).is_some();
        if removed {
            Self::mark_pending_persistence(cx);
        }
    }

    pub fn persisted_disk_cache_keys() -> Vec<String> {
        SaiWsDiskCache::all_indexed_cache_keys()
            .into_iter()
            .filter(|key| SaiAccountCacheScope::logical_key(key) == LOGICAL_DISK_CACHE_KEY)
            .collect()
    }

    pub fn clear_all() {
        let cancelled = {
            let mut state = STATE.lock();
            let cancelled = state.persistence_task.take();
            state.persistence_generation = state.persistence_generation.wrapping_add(1);
            state.persisted_generation = state.persistence_generation;
            state.persistence_in_flight_generation = None;
            state.pending_disk_snapshots.clear();
            state.positions_by_chat_id.clear();
            state.loaded_disk_cache_key = None;
            cancelled
        };
        drop(cancelled);
        SaiWsDiskCache::wait_for_asynchronous_writes();
        for key in SaiWsDiskCache::all_indexed_cache_keys() {
            if SaiAccountCacheScope::logical_key(&key) == LOGICAL_DISK_CACHE_KEY {
                SaiWsDiskCache::remove(&key);
            }
        }
    }

    pub fn reset_account_state(persist_pending: bool, cx: &mut App) {
        if persist_pending {
            Self::persist_current_snapshot_before_reset(cx);
        } else {
            STATE.lock().pending_disk_snapshots.clear();
        }
        let cancelled = {
            let mut state = STATE.lock();
            let cancelled = state.persistence_task.take();
            state.persistence_generation = state.persistence_generation.wrapping_add(1);
            state.persisted_generation = state.persistence_generation;
            state.persistence_in_flight_generation = None;
            state.positions_by_chat_id.clear();
            state.loaded_disk_cache_key = None;
            cancelled
        };
        drop(cancelled);
    }

    pub fn activate_account_state(cx: &mut App) {
        let cache_key = SaiAccountCacheScope::scoped_key(LOGICAL_DISK_CACHE_KEY);
        if STATE.lock().loaded_disk_cache_key.as_deref() == Some(cache_key.as_str()) {
            return;
        }
        Self::persist_current_snapshot_before_reset(cx);
        let (cancelled, pending_snapshot) = {
            let mut state = STATE.lock();
            let cancelled = state.persistence_task.take();
            state.persistence_in_flight_generation = None;
            (cancelled, state.pending_disk_snapshots.get(&cache_key).cloned())
        };
        drop(cancelled);
        let has_pending_snapshot = pending_snapshot.is_some();
        let positions = match pending_snapshot {
            Some(snapshot) => snapshot.positions,
            None => Self::load_from_disk(&cache_key),
        };
        {
            let mut state = STATE.lock();
            state.positions_by_chat_id = positions;
            state.loaded_disk_cache_key = Some(cache_key);
            state.persistence_generation = state.persistence_generation.wrapping_add(1);
            state.persisted_generation = state.persistence_generation;
            if has_pending_snapshot {
                state.persistence_generation = state.persistence_generation.wrapping_add(1);
            }
        }
        if has_pending_snapshot {
            Self::schedule_safety_checkpoint(SAFETY_CHECKPOINT_DELAY, cx);
        }
    }

    pub fn remove_persisted_account(user_id: &str) {
        let prefix = SaiAccountCacheScope::prefix(user_id);
        STATE.lock().pending_disk_snapshots.retain(|key, _| !key.starts_with(&prefix));
    }

    fn load_from_disk(cache_key: &str) -> HashMap<String, ChatScrollPositionStorePosition> {
        let Some(Value::Object(raw)) = SaiWsDiskCache::load(cache_key) else {
            return HashMap::new();
        };
        let mut out = HashMap::with_capacity(raw.len());
        for (chat_id, value) in raw {
            if let Value::Number(number) = &value {
                let distance = number.as_f64().unwrap_or(0.0) as f32;
                if distance > 1.0 {
                    out.insert(chat_id, ChatScrollPositionStorePosition::new(None, 0.0, distance));
                }
                continue;
            }
            let Ok(persisted) = serde_json::from_value::<PersistedChatScrollPosition>(value) else {
                continue;
            };
            let Some(distance) = persisted.distance_from_top.as_f64() else {
                continue;
            };
            let distance = distance as f32;
            let anchor_id = persisted.anchor_id.as_ref().and_then(Value::as_str).map(str::to_owned);
            if anchor_id.as_deref().is_none_or(str::is_empty) && distance <= 1.0 {
                continue;
            }
            let viewport_offset = persisted.viewport_offset.as_ref().and_then(Value::as_f64).unwrap_or(0.0) as f32;
            out.insert(chat_id, ChatScrollPositionStorePosition::new(anchor_id, viewport_offset, distance));
        }
        out
    }

    pub fn flush_pending_to_disk(cx: &mut App) {
        Self::activate_account_state(cx);
        let (cancelled, write) = {
            let mut state = STATE.lock();
            let cancelled = state.persistence_task.take();
            let write = match &state.loaded_disk_cache_key {
                Some(loaded_disk_cache_key)
                    if state.persisted_generation != state.persistence_generation
                        && state.persistence_in_flight_generation.is_none() =>
                {
                    Some((loaded_disk_cache_key.clone(), state.persistence_generation, state.positions_by_chat_id.clone()))
                }
                _ => None,
            };
            if let Some((_, generation, _)) = &write {
                state.persistence_in_flight_generation = Some(*generation);
            }
            (cancelled, write)
        };
        drop(cancelled);
        let Some((loaded_disk_cache_key, generation, snapshot)) = write else {
            return;
        };
        let completion_key = loaded_disk_cache_key.clone();
        Self::save_asynchronously(
            loaded_disk_cache_key,
            snapshot,
            move |result, cx| {
                {
                    let mut state = STATE.lock();
                    if state.loaded_disk_cache_key.as_deref() != Some(completion_key.as_str())
                        || state.persistence_in_flight_generation != Some(generation)
                    {
                        return;
                    }
                    state.persistence_in_flight_generation = None;
                }
                match result {
                    Ok(()) => {
                        let settled = {
                            let mut state = STATE.lock();
                            if state.persistence_generation == generation {
                                state.persisted_generation = generation;
                                true
                            } else {
                                false
                            }
                        };
                        if !settled {
                            Self::flush_pending_to_disk(cx);
                        }
                    }
                    Err(_) => Self::schedule_safety_checkpoint(FAILED_WRITE_RETRY_DELAY, cx),
                }
            },
            cx,
        );
    }

    pub fn observe_app_exit(cx: &mut App) {
        if cx.has_global::<ChatScrollPositionExitObserver>() {
            return;
        }
        let app_quit = cx.on_app_quit(Self::flush_for_exit);
        cx.set_global(ChatScrollPositionExitObserver { _app_quit: app_quit });
    }

    /// queues the newest positions and returns the wait for the write lane
    /// a desktop quit blocks the main thread so the wait is the lease
    pub fn flush_for_exit(cx: &mut App) -> Task<()> {
        STATE.lock().persistence_in_flight_generation = None;
        Self::flush_pending_to_disk(cx);
        cx.background_executor().spawn(async {
            SaiWsDiskCache::wait_for_asynchronous_writes();
        })
    }

    fn mark_pending_persistence(cx: &mut App) {
        {
            let mut state = STATE.lock();
            state.persistence_generation = state.persistence_generation.wrapping_add(1);
        }
        Self::schedule_safety_checkpoint(SAFETY_CHECKPOINT_DELAY, cx);
    }

    fn schedule_safety_checkpoint(delay: Duration, cx: &mut App) {
        let cancelled = STATE.lock().persistence_task.take();
        drop(cancelled);
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            cx.update(|cx| Self::flush_pending_to_disk(cx));
        });
        STATE.lock().persistence_task = Some(task);
    }

    fn persist_current_snapshot_before_reset(cx: &mut App) {
        let write = {
            let state = STATE.lock();
            match &state.loaded_disk_cache_key {
                Some(loaded_disk_cache_key) if state.persisted_generation != state.persistence_generation => {
                    Some((loaded_disk_cache_key.clone(), state.positions_by_chat_id.clone()))
                }
                _ => None,
            }
        };
        let Some((loaded_disk_cache_key, positions)) = write else {
            return;
        };
        let retry_key = loaded_disk_cache_key.clone();
        Self::save_asynchronously(
            loaded_disk_cache_key,
            positions,
            move |result, cx| {
                if result.is_ok() {
                    return;
                }
                let pending = STATE.lock().pending_disk_snapshots.get(&retry_key).map(|pending| pending.positions.clone());
                let Some(pending) = pending else {
                    return;
                };
                Self::save_asynchronously(retry_key, pending, |_, _| {}, cx);
            },
            cx,
        );
    }

    fn save_asynchronously(
        cache_key: String,
        positions: HashMap<String, ChatScrollPositionStorePosition>,
        completion: impl FnOnce(Result<(), SaiWsDiskCacheError>, &mut App) + 'static,
        cx: &mut App,
    ) {
        let payload = Self::persistence_payload(&positions);
        let token = {
            let mut state = STATE.lock();
            state.disk_snapshot_token = state.disk_snapshot_token.wrapping_add(1);
            let token = state.disk_snapshot_token;
            state.pending_disk_snapshots.insert(cache_key.clone(), PendingDiskSnapshot { token, positions });
            token
        };
        let disk_lease = begin_process_activity("ChatScrollPositionCheckpoint");
        let snapshot_key = cache_key.clone();
        SaiWsDiskCache::save_asynchronously(
            cache_key,
            payload,
            move |result, cx| {
                let _disk_lease = disk_lease;
                if result.is_ok() {
                    let mut state = STATE.lock();
                    if state.pending_disk_snapshots.get(&snapshot_key).map(|pending| pending.token) == Some(token) {
                        state.pending_disk_snapshots.remove(&snapshot_key);
                    }
                }
                completion(result, cx);
            },
            cx,
        );
    }

    fn persistence_payload(positions: &HashMap<String, ChatScrollPositionStorePosition>) -> Value {
        let mut payload = Map::with_capacity(positions.len());
        for (chat_id, position) in positions {
            let persisted = PersistedChatScrollPosition {
                anchor_id: position.anchor_id.clone().map(Value::from),
                viewport_offset: Some(Value::from(f64::from(position.viewport_offset))),
                distance_from_top: Value::from(f64::from(position.distance_from_top)),
            };
            if let Ok(value) = serde_json::to_value(persisted) {
                payload.insert(chat_id.clone(), value);
            }
        }
        Value::Object(payload)
    }
}
