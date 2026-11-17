use std::collections::HashMap;

use crate::rise_store::rise_store::RiseStore;
use crate::rise_store::rise_store_models::RiseStoreNamespace;

const MAXIMUM_ENTITY_READ_CACHE_ITEMS: usize = 1_024;
const MAXIMUM_ENTITY_READ_CACHE_BYTES: usize = 32 * 1_024 * 1_024;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct RiseEntityReadCacheKey {
    pub(crate) namespace: RiseStoreNamespace,
    pub(crate) local_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RiseEntityReadCacheEntry {
    pub(crate) version: i64,
    pub(crate) data: Vec<u8>,
    pub(crate) access: u64,
}

impl RiseStore {
    pub(crate) fn cached_entity_data(
        &self,
        namespace: &RiseStoreNamespace,
        local_id: &str,
        version: i64,
    ) -> Option<Vec<u8>> {
        let key = RiseEntityReadCacheKey {
            namespace: namespace.clone(),
            local_id: local_id.to_owned(),
        };
        let mut entity_read_cache = self.entity_read_cache.lock();
        let entry = entity_read_cache
            .get_mut(&key)
            .filter(|entry| entry.version == version)?;
        let mut entity_read_cache_access = self.entity_read_cache_access.lock();
        *entity_read_cache_access = entity_read_cache_access.wrapping_add(1);
        entry.access = *entity_read_cache_access;
        Some(entry.data.clone())
    }

    pub(crate) fn cache_entity_data(
        &self,
        data: Vec<u8>,
        namespace: &RiseStoreNamespace,
        local_id: &str,
        version: i64,
    ) {
        if data.len() > MAXIMUM_ENTITY_READ_CACHE_BYTES {
            return;
        }
        let key = RiseEntityReadCacheKey {
            namespace: namespace.clone(),
            local_id: local_id.to_owned(),
        };
        let mut entity_read_cache = self.entity_read_cache.lock();
        let mut entity_read_cache_bytes = self.entity_read_cache_bytes.lock();
        let mut entity_read_cache_access = self.entity_read_cache_access.lock();
        let data_bytes = data.len();
        *entity_read_cache_access = entity_read_cache_access.wrapping_add(1);
        let entry = RiseEntityReadCacheEntry {
            version,
            data,
            access: *entity_read_cache_access,
        };
        if let Some(previous) = entity_read_cache.insert(key.clone(), entry) {
            *entity_read_cache_bytes = entity_read_cache_bytes.saturating_sub(previous.data.len());
        }
        *entity_read_cache_bytes = entity_read_cache_bytes.saturating_add(data_bytes);
        Self::evict_entity_read_cache_if_needed(
            &mut entity_read_cache,
            &mut entity_read_cache_bytes,
            &key,
        );
    }

    pub(crate) fn invalidate_entity_read_cache(
        &self,
        namespace: &RiseStoreNamespace,
        local_id: &str,
    ) {
        let key = RiseEntityReadCacheKey {
            namespace: namespace.clone(),
            local_id: local_id.to_owned(),
        };
        let mut entity_read_cache = self.entity_read_cache.lock();
        if let Some(removed) = entity_read_cache.remove(&key) {
            let mut entity_read_cache_bytes = self.entity_read_cache_bytes.lock();
            *entity_read_cache_bytes = entity_read_cache_bytes.saturating_sub(removed.data.len());
        }
    }

    pub(crate) fn clear_entity_read_cache(&self, namespace: Option<&RiseStoreNamespace>) {
        let mut entity_read_cache = self.entity_read_cache.lock();
        let mut entity_read_cache_bytes = self.entity_read_cache_bytes.lock();
        let Some(namespace) = namespace else {
            *entity_read_cache = HashMap::new();
            *entity_read_cache_bytes = 0;
            return;
        };
        let mut removed_bytes = 0usize;
        entity_read_cache.retain(|key, entry| {
            if key.namespace == *namespace {
                removed_bytes = removed_bytes.saturating_add(entry.data.len());
                false
            } else {
                true
            }
        });
        *entity_read_cache_bytes = entity_read_cache_bytes.saturating_sub(removed_bytes);
    }

    fn evict_entity_read_cache_if_needed(
        entity_read_cache: &mut HashMap<RiseEntityReadCacheKey, RiseEntityReadCacheEntry>,
        entity_read_cache_bytes: &mut usize,
        protecting: &RiseEntityReadCacheKey,
    ) {
        while entity_read_cache.len() > MAXIMUM_ENTITY_READ_CACHE_ITEMS
            || *entity_read_cache_bytes > MAXIMUM_ENTITY_READ_CACHE_BYTES
        {
            let Some(victim) = entity_read_cache
                .iter()
                .filter(|(key, _)| *key != protecting)
                .min_by_key(|(_, entry)| entry.access)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(removed) = entity_read_cache.remove(&victim) {
                *entity_read_cache_bytes =
                    entity_read_cache_bytes.saturating_sub(removed.data.len());
            }
        }
    }
}
