use std::collections::{HashMap, HashSet};

use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewItem;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::engine::chats::rise_chat_repository_models::{RiseChatRepositoryError, RiseChatWireCodec};
use crate::modules::chat::engine::core::rise_conversation_entity_records::RiseChatEntityRecord;

#[derive(Clone, PartialEq, Eq, Hash)]
struct RiseChatDecodedRecordCacheKey {
    local_id: String,
    version: i64,
    updated_at_milliseconds: i64,
}

struct RiseChatDecodedRecordCacheEntry {
    record: RiseChatEntityRecord,
    cost: i64,
    access: u64,
}

struct RiseChatDecodedRecordCacheState {
    entries: HashMap<RiseChatDecodedRecordCacheKey, RiseChatDecodedRecordCacheEntry>,
    keys_by_local_id: HashMap<String, HashSet<RiseChatDecodedRecordCacheKey>>,
    total_bytes: i64,
    access_clock: u64,
}

/// decoded object cache shared by inbox, archive and requests
/// key has the entity version so an update is a miss, bounded by count and payload cost
pub struct RiseChatDecodedRecordCache {
    maximum_entries: i64,
    maximum_bytes: i64,
    state: RiseIsolated<RiseChatDecodedRecordCacheState>,
}

impl RiseChatDecodedRecordCache {
    pub fn default_maximum_entries() -> i64 {
        RiseLimits::client_value("chat_decoded_record_cache_entries")
    }

    pub fn default_maximum_bytes() -> i64 {
        RiseLimits::client_value("chat_decoded_record_cache_bytes")
    }

    pub fn new(maximum_entries: i64, maximum_bytes: i64) -> Self {
        assert!(maximum_entries > 0 && maximum_bytes > 0);
        Self {
            maximum_entries,
            maximum_bytes,
            state: RiseIsolated::new(RiseChatDecodedRecordCacheState {
                entries: HashMap::new(),
                keys_by_local_id: HashMap::new(),
                total_bytes: 0,
                access_clock: 0,
            }),
        }
    }

    pub async fn records(&self, items: &[RiseEntityViewItem]) -> anyhow::Result<Vec<RiseChatEntityRecord>> {
        let mut result: Vec<Option<RiseChatEntityRecord>> = vec![None; items.len()];
        let mut misses: Vec<(usize, RiseChatDecodedRecordCacheKey)> = Vec::with_capacity(items.len());

        self.state.with(|state| {
            for (index, item) in items.iter().enumerate() {
                let key = RiseChatDecodedRecordCacheKey {
                    local_id: item.entity.local_id.clone(),
                    version: item.entity.version,
                    updated_at_milliseconds: item.entity.updated_at_milliseconds,
                };
                let access = state.next_access();
                if let Some(entry) = state.entries.get_mut(&key) {
                    entry.access = access;
                    result[index] = Some(entry.record.clone());
                } else {
                    misses.push((index, key));
                }
            }
        });

        if !misses.is_empty() {
            let mut decoded = Vec::with_capacity(misses.len());
            for (index, key) in misses {
                let data = &items[index].entity.data;
                let record = RiseChatWireCodec::decode_record(data)?;
                decoded.push((index, key, record, (data.len() as i64).max(1)));
            }
            let maximum_entries = self.maximum_entries;
            let maximum_bytes = self.maximum_bytes;
            self.state.with(|state| {
                for (index, key, record, cost) in decoded {
                    result[index] = Some(record.clone());
                    state.install(record, key, cost, maximum_entries, maximum_bytes);
                }
            });
        }

        result
            .into_iter()
            .map(|record| {
                record.ok_or_else(|| {
                    anyhow::Error::from(RiseChatRepositoryError::CorruptedChat("decoded_cache".to_owned()))
                })
            })
            .collect()
    }
}

impl Default for RiseChatDecodedRecordCache {
    fn default() -> Self {
        Self::new(Self::default_maximum_entries(), Self::default_maximum_bytes())
    }
}

impl RiseChatDecodedRecordCacheState {
    fn install(
        &mut self,
        record: RiseChatEntityRecord,
        key: RiseChatDecodedRecordCacheKey,
        cost: i64,
        maximum_entries: i64,
        maximum_bytes: i64,
    ) {
        let stale: Vec<RiseChatDecodedRecordCacheKey> = self
            .keys_by_local_id
            .get(&key.local_id)
            .map(|keys| keys.iter().filter(|old_key| **old_key != key).cloned().collect())
            .unwrap_or_default();
        for old_key in &stale {
            self.remove(old_key);
        }
        if let Some(old) = self.entries.remove(&key) {
            self.total_bytes = (self.total_bytes - old.cost).max(0);
        }
        let access = self.next_access();
        self.keys_by_local_id.entry(key.local_id.clone()).or_default().insert(key.clone());
        self.entries.insert(key, RiseChatDecodedRecordCacheEntry { record, cost, access });
        self.total_bytes += cost;
        self.evict_if_needed(maximum_entries, maximum_bytes);
    }

    fn evict_if_needed(&mut self, maximum_entries: i64, maximum_bytes: i64) {
        while self.entries.len() as i64 > maximum_entries || self.total_bytes > maximum_bytes {
            let victim = self.entries.iter().min_by_key(|(_, entry)| entry.access).map(|(key, _)| key.clone());
            let Some(victim) = victim else {
                break;
            };
            self.remove(&victim);
        }
    }

    fn remove(&mut self, key: &RiseChatDecodedRecordCacheKey) {
        let Some(removed) = self.entries.remove(key) else {
            return;
        };
        self.total_bytes = (self.total_bytes - removed.cost).max(0);
        if let Some(keys) = self.keys_by_local_id.get_mut(&key.local_id) {
            keys.remove(key);
            if keys.is_empty() {
                self.keys_by_local_id.remove(&key.local_id);
            }
        }
    }

    fn next_access(&mut self) -> u64 {
        self.access_clock = self.access_clock.wrapping_add(1);
        self.access_clock
    }
}
