use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_url::Url;
use rise_engine::rise_upload::rise_url_session::URLRequest;
use rise_platform::AppPaths;
use rise_platform::file_manager::FileManager;
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::media::remote_media_disk_cache::RemoteMediaHttpResponse;

const DIRECTORY_NAME: &str = "URLCache";
const BODIES_DIRECTORY_NAME: &str = "fsCachedData";
const RESPONSES_DIRECTORY_NAME: &str = "responses";
const PARTIAL_EXTENSION: &str = "partial";

const MAX_ENTRY_SHARE_DIVISOR: usize = 20;

static CACHE_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::current().canon())
        .ok()
        .map(|paths| paths.cache_dir().join(DIRECTORY_NAME))
});

static SHARED: LazyLock<URLCache> = LazyLock::new(|| {
    URLCache::new(
        usize::try_from(RiseLimits::client_value("url_cache_memory_bytes")).unwrap_or(0),
        usize::try_from(RiseLimits::client_value("url_cache_disk_bytes")).unwrap_or(0),
        CACHE_ROOT.clone(),
    )
});

/// cached response, the response and body stored for a request
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedURLResponse {
    pub response: RemoteMediaHttpResponse,
    pub data: Vec<u8>,
}

impl CachedURLResponse {
    pub fn new(response: RemoteMediaHttpResponse, data: Vec<u8>) -> Self {
        Self { response, data }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredResponse {
    url: String,
    status_code: i64,
    header_fields: Vec<(String, String)>,
}

#[derive(Default)]
struct MemoryState {
    entries: HashMap<String, CachedURLResponse>,
    order: VecDeque<String>,
    usage: usize,
}

impl MemoryState {
    fn remove(&mut self, key: &str) {
        if let Some(previous) = self.entries.remove(key) {
            self.usage = self.usage.saturating_sub(previous.data.len());
            self.order.retain(|existing| existing != key);
        }
    }

    fn touch(&mut self, key: &str) {
        if let Some(position) = self.order.iter().position(|existing| existing == key) {
            if let Some(existing) = self.order.remove(position) {
                self.order.push_back(existing);
            }
        }
    }
}

/// url cache, blocking so keep callers off the gpui main thread
pub struct URLCache {
    memory_capacity: usize,
    disk_capacity: usize,
    directory: Option<PathBuf>,
    memory: Mutex<MemoryState>,
    disk: Mutex<()>,
}

impl URLCache {
    /// the shared url cache
    pub fn shared() -> &'static URLCache {
        &SHARED
    }

    /// new url cache with memory and disk capacity and an optional directory
    pub fn new(memory_capacity: usize, disk_capacity: usize, directory: Option<PathBuf>) -> Self {
        Self {
            memory_capacity,
            disk_capacity,
            directory,
            memory: Mutex::new(MemoryState::default()),
            disk: Mutex::new(()),
        }
    }

    /// where response bodies sit, the storage screen counts it as http responses
    pub fn response_bodies_directory(&self) -> Option<PathBuf> {
        self.directory.as_ref().map(|root| root.join(BODIES_DIRECTORY_NAME))
    }

    pub fn store_cached_response(&self, cached: CachedURLResponse, for_request: &URLRequest) {
        let key = Self::key(for_request);
        let size = cached.data.len();
        self.store_on_disk(&key, &cached);
        let mut memory = self.memory.lock();
        memory.remove(&key);
        if !Self::fits(size, self.memory_capacity) {
            return;
        }
        memory.usage += size;
        memory.order.push_back(key.clone());
        memory.entries.insert(key, cached);
        while memory.usage > self.memory_capacity {
            let Some(oldest) = memory.order.pop_front() else {
                break;
            };
            if let Some(evicted) = memory.entries.remove(&oldest) {
                memory.usage = memory.usage.saturating_sub(evicted.data.len());
            }
        }
    }

    pub fn cached_response(&self, for_request: &URLRequest) -> Option<CachedURLResponse> {
        let key = Self::key(for_request);
        {
            let mut memory = self.memory.lock();
            if let Some(hit) = memory.entries.get(&key).cloned() {
                memory.touch(&key);
                return Some(hit);
            }
        }
        let cached = self.load_from_disk(&key)?;
        let size = cached.data.len();
        let mut memory = self.memory.lock();
        if Self::fits(size, self.memory_capacity) && !memory.entries.contains_key(&key) {
            memory.usage += size;
            memory.order.push_back(key.clone());
            memory.entries.insert(key, cached.clone());
            while memory.usage > self.memory_capacity {
                let Some(oldest) = memory.order.pop_front() else {
                    break;
                };
                if let Some(evicted) = memory.entries.remove(&oldest) {
                    memory.usage = memory.usage.saturating_sub(evicted.data.len());
                }
            }
        }
        Some(cached)
    }

    pub fn remove_cached_response(&self, for_request: &URLRequest) {
        let key = Self::key(for_request);
        self.memory.lock().remove(&key);
        let Some(root) = self.directory.as_ref() else {
            return;
        };
        let _disk = self.disk.lock();
        let digest = sha256_hex(key.as_bytes());
        let _ = FileManager.remove_item(&root.join(BODIES_DIRECTORY_NAME).join(&digest));
        let _ = FileManager.remove_item(&root.join(RESPONSES_DIRECTORY_NAME).join(&digest));
    }

    pub fn remove_all_cached_responses(&self) {
        *self.memory.lock() = MemoryState::default();
        let Some(root) = self.directory.as_ref() else {
            return;
        };
        let _disk = self.disk.lock();
        let _ = FileManager.remove_item(&root.join(BODIES_DIRECTORY_NAME));
        let _ = FileManager.remove_item(&root.join(RESPONSES_DIRECTORY_NAME));
    }

    pub fn current_memory_usage(&self) -> usize {
        self.memory.lock().usage
    }

    /// current disk usage in bytes, stored bodies only
    pub fn current_disk_usage(&self) -> usize {
        let Some(bodies) = self.response_bodies_directory() else {
            return 0;
        };
        let _disk = self.disk.lock();
        Self::disk_entries(&bodies).iter().map(|entry| entry.size).sum()
    }

    fn key(request: &URLRequest) -> String {
        request.url.absolute_string().to_owned()
    }

    fn fits(size: usize, capacity: usize) -> bool {
        capacity > 0 && size <= capacity / MAX_ENTRY_SHARE_DIVISOR
    }

    fn store_on_disk(&self, key: &str, cached: &CachedURLResponse) {
        let Some(root) = self.directory.as_ref() else {
            return;
        };
        let digest = sha256_hex(key.as_bytes());
        let bodies = root.join(BODIES_DIRECTORY_NAME);
        let responses = root.join(RESPONSES_DIRECTORY_NAME);
        let _disk = self.disk.lock();
        let body_path = bodies.join(&digest);
        let response_path = responses.join(&digest);
        if !Self::fits(cached.data.len(), self.disk_capacity) {
            let _ = FileManager.remove_item(&body_path);
            let _ = FileManager.remove_item(&response_path);
            return;
        }
        if FileManager.create_directory(&bodies, true).is_err()
            || FileManager.create_directory(&responses, true).is_err()
        {
            return;
        }
        let stored = StoredResponse {
            url: cached.response.url.absolute_string().to_owned(),
            status_code: cached.response.status_code,
            header_fields: cached.response.all_header_fields().to_vec(),
        };
        let Ok(encoded) = serde_json::to_vec(&stored) else {
            return;
        };
        if !Self::write_atomically(&body_path, &cached.data) {
            return;
        }
        if !Self::write_atomically(&response_path, &encoded) {
            let _ = FileManager.remove_item(&body_path);
            return;
        }
        self.trim_disk(&bodies, &responses);
    }

    fn load_from_disk(&self, key: &str) -> Option<CachedURLResponse> {
        let root = self.directory.as_ref()?;
        let digest = sha256_hex(key.as_bytes());
        let _disk = self.disk.lock();
        let encoded = std::fs::read(root.join(RESPONSES_DIRECTORY_NAME).join(&digest)).ok()?;
        let data = std::fs::read(root.join(BODIES_DIRECTORY_NAME).join(&digest)).ok()?;
        let stored: StoredResponse = serde_json::from_slice(&encoded).ok()?;
        let url = Url::from_string(&stored.url)?;
        Some(CachedURLResponse::new(
            RemoteMediaHttpResponse::new(url, stored.status_code, stored.header_fields),
            data,
        ))
    }

    fn write_atomically(path: &Path, contents: &[u8]) -> bool {
        let partial = path.with_extension(PARTIAL_EXTENSION);
        if std::fs::write(&partial, contents).is_err() {
            let _ = FileManager.remove_item(&partial);
            return false;
        }
        if std::fs::rename(&partial, path).is_err() {
            let _ = FileManager.remove_item(&partial);
            return false;
        }
        true
    }

    fn trim_disk(&self, bodies: &Path, responses: &Path) {
        let mut entries = Self::disk_entries(bodies);
        let mut usage: usize = entries.iter().map(|entry| entry.size).sum();
        if usage <= self.disk_capacity {
            return;
        }
        entries.sort_by_key(|entry| entry.modified);
        for entry in entries {
            if usage <= self.disk_capacity {
                break;
            }
            let _ = FileManager.remove_item(&bodies.join(&entry.name));
            let _ = FileManager.remove_item(&responses.join(&entry.name));
            usage = usage.saturating_sub(entry.size);
        }
    }

    fn disk_entries(bodies: &Path) -> Vec<DiskEntry> {
        let Ok(names) = FileManager.contents_of_directory_at_path(bodies) else {
            return Vec::new();
        };
        names
            .into_iter()
            .filter(|name| Path::new(name).extension().is_none())
            .filter_map(|name| {
                let metadata = std::fs::metadata(bodies.join(&name)).ok()?;
                metadata.is_file().then(|| DiskEntry {
                    size: usize::try_from(metadata.len()).unwrap_or(usize::MAX),
                    modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    name,
                })
            })
            .collect()
    }
}

struct DiskEntry {
    name: String,
    size: usize,
    modified: SystemTime,
}

#[cfg(test)]
#[path = "url_cache_tests.rs"]
mod tests;
