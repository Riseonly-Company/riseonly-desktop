use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use futures::channel::oneshot;
use gpui::App;
use rise_canon::rise_key;
use serde_json::Value;

use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;

const PEER_NAME: &str = "peer_name";
const PEER_TAG: &str = "peer_tag";
const PEER_LOGO: &str = "peer_logo";

#[derive(Clone, Debug, PartialEq)]
pub struct RiseUndeliverableDirectDraftLoaderEntry {
    pub id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub peer_tag: String,
    pub peer_logo: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub preview: Option<String>,
    pub sender_id: String,
    pub content_type: String,
}

/// handle for a draft restore, only used to cancel one that isnt wanted anymore
#[derive(Clone, Default)]
pub struct RiseUndeliverableDirectDraftLoaderOperation {
    is_cancelled: Arc<AtomicBool>,
}

impl RiseUndeliverableDirectDraftLoaderOperation {
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::SeqCst)
    }
}

static QUEUE_TAIL: LazyLock<Mutex<Option<oneshot::Receiver<()>>>> = LazyLock::new(|| Mutex::new(None));

pub enum RiseUndeliverableDirectDraftLoader {}

impl RiseUndeliverableDirectDraftLoader {
    pub fn load(
        cache_key: String,
        draft_prefix: String,
        limit: i64,
        completion: impl FnOnce(Vec<RiseUndeliverableDirectDraftLoaderEntry>, &mut App) + 'static,
        cx: &mut App,
    ) -> RiseUndeliverableDirectDraftLoaderOperation {
        let operation = RiseUndeliverableDirectDraftLoaderOperation::default();
        let (finished, finished_signal) = oneshot::channel::<()>();
        let previous = QUEUE_TAIL.lock().unwrap_or_else(PoisonError::into_inner).replace(finished_signal);

        let read_operation = operation.clone();
        let read = cx.background_executor().spawn(async move {
            let _finished = finished;
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            if read_operation.is_cancelled() {
                return None;
            }
            let payload = SaiWsDiskCache::load(&cache_key);
            let entries = Self::prepare(payload.as_ref(), &draft_prefix, limit);
            if read_operation.is_cancelled() { None } else { Some(entries) }
        });

        let delivery_operation = operation.clone();
        cx.spawn(async move |cx| {
            let Some(entries) = read.await else {
                return;
            };
            if delivery_operation.is_cancelled() {
                return;
            }
            cx.update(|cx| completion(entries, cx));
        })
        .detach();
        operation
    }

    pub fn prepare(
        payload: Option<&Value>,
        draft_prefix: &str,
        limit: i64,
    ) -> Vec<RiseUndeliverableDirectDraftLoaderEntry> {
        if limit <= 0 {
            return Vec::new();
        }
        let Some(rows) = payload.and_then(Value::as_array) else {
            return Vec::new();
        };
        let Some(rows) = rows.iter().map(Value::as_object).collect::<Option<Vec<_>>>() else {
            return Vec::new();
        };

        let draft_prefix_count = draft_prefix.chars().count();
        let mut entries = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for row in rows.into_iter().take(limit as usize) {
            let Some(id) = row.get(rise_key::bot::ID).and_then(Value::as_str) else {
                continue;
            };
            let Some(peer_id) = id.strip_prefix(draft_prefix) else {
                continue;
            };
            if id.chars().count() <= draft_prefix_count || !seen.insert(id) {
                continue;
            }

            let text = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            let updated_at = row.get(rise_key::bot::UPDATED_AT).and_then(Value::as_f64).unwrap_or(0.0);
            entries.push(RiseUndeliverableDirectDraftLoaderEntry {
                id: id.to_owned(),
                peer_id: peer_id.to_owned(),
                peer_name: text(PEER_NAME),
                peer_tag: text(PEER_TAG),
                peer_logo: text(PEER_LOGO),
                created_at: row.get(rise_key::bot::CREATED_AT).and_then(Value::as_f64).unwrap_or(updated_at),
                updated_at,
                preview: row.get(rise_key::music::PREVIEW).and_then(Value::as_str).map(str::to_owned),
                sender_id: text(rise_key::bot::SENDER_ID),
                content_type: text(rise_key::bot::CONTENT_TYPE),
            });
        }
        entries
    }
}
