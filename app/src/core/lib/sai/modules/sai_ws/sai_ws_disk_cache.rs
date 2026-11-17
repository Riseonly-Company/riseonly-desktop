use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, UNIX_EPOCH};

use futures::channel::oneshot;
use gpui::App;
use parking_lot::{Mutex, ReentrantMutex};
use rise_core::foundation::foundation_character::characters;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::AppPaths;
use rise_platform::file_manager::{CocoaError, FileManager};
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};
use serde_json::{Map, Value};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_payload_sanitizer::SaiWsDiskPayloadSanitizer;
use crate::core::lib::sai::sai::Sai;

/// what save and the optimistic meta writer can fail with, file system or encoder
#[derive(Debug, thiserror::Error)]
pub enum SaiWsDiskCacheError {
    #[error(transparent)]
    File(#[from] CocoaError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("JSON data from cache value")]
    Encoding,
    #[error("application data directory unavailable")]
    NoDataDirectory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaiWsDiskCacheLimits {
    pub maximum_entry_count_per_account: i64,
    pub maximum_total_bytes_per_account: i64,
}

impl SaiWsDiskCacheLimits {
    pub fn new(maximum_entry_count_per_account: i64, maximum_total_bytes_per_account: i64) -> Self {
        Self {
            maximum_entry_count_per_account: maximum_entry_count_per_account.max(1),
            maximum_total_bytes_per_account: maximum_total_bytes_per_account.max(1),
        }
    }

    pub const PRODUCTION: Self = Self {
        maximum_entry_count_per_account: 4_000,
        maximum_total_bytes_per_account: 512 * 1_024 * 1_024,
    };
}

/// lru record of one cache file
#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsDiskCacheLRURecord {
    pub digest_hex: String,
    pub byte_count: i64,
    pub last_accessed_at: TimeInterval,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiWsDiskCacheDebugPayloadSummary {
    pub digest: String,
    pub cache_key: Option<String>,
    pub file_bytes: i64,
}

const SUBFOLDER: &str = "SaiWsResponseCache";
const KEY_INDEX_FILE_NAME: &str = "_cache_key_index.json";
const OPTIMISTIC_META_SUFFIX: &str = ".saiws-meta.json";
const PAYLOAD_SUFFIX: &str = ".json";
const UNSCOPED_OWNER: &str = "__unscoped__";
const ACCOUNT_PREFIX: &str = "account:";
const PATH_TO_ARRAY: &str = "pathToArray";
const PATH_TO_ID: &str = "pathToId";
const WS_CACHE_VALUE: &str = "_ws_cache_value";
const LIMITS: SaiWsDiskCacheLimits = SaiWsDiskCacheLimits::PRODUCTION;

static BASE_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::current().canon())
        .ok()
        .map(|paths| paths.data_dir().to_path_buf())
});
static ACCESS_LOCK: LazyLock<ReentrantMutex<()>> = LazyLock::new(|| ReentrantMutex::new(()));

type WriteJob = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct WriteLane {
    jobs: VecDeque<WriteJob>,
    draining: bool,
}

static WRITE_LANE: LazyLock<Mutex<WriteLane>> = LazyLock::new(Mutex::default);
static WRITE_IO: Mutex<()> = Mutex::new(());

thread_local! {
    static ON_WRITE_LANE: Cell<bool> = const { Cell::new(false) };
}

struct WriteLaneMark;

impl WriteLaneMark {
    fn enter() -> Self {
        ON_WRITE_LANE.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for WriteLaneMark {
    fn drop(&mut self) {
        ON_WRITE_LANE.with(|flag| flag.set(false));
    }
}

struct PendingWrite {
    generation: u64,
    value: Value,
}

#[derive(Default)]
struct PendingWrites {
    entries: HashMap<String, PendingWrite>,
    generation: u64,
}

static PENDING_WRITES: LazyLock<Mutex<PendingWrites>> = LazyLock::new(Mutex::default);

#[derive(Clone)]
enum DiskReadEntry {
    Present(Arc<Value>),
    Absent,
}

#[derive(Clone)]
enum MetaReadEntry {
    Present { path_to_array: Option<String>, path_to_id: Option<String> },
    Absent,
}

const READ_CACHE_LIMIT: usize = 96;
const TOUCH_INTERVAL: TimeInterval = 30.0;
const DIGEST_CACHE_LIMIT: usize = 512;

#[derive(Default)]
struct ReadCache {
    payload: HashMap<String, DiskReadEntry>,
    meta: HashMap<String, MetaReadEntry>,
    last_touched_at: HashMap<String, TimeInterval>,
    payload_order: Vec<String>,
    meta_order: Vec<String>,
    base_directory: Option<PathBuf>,
}

static READ_CACHE: LazyLock<Mutex<ReadCache>> = LazyLock::new(Mutex::default);

static DIGEST_BY_KEY: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Mutex::default);

/// frees evicted decoded trees off the caller thread, they can be big
fn release_off_thread<T: Send + 'static>(released: Vec<T>) {
    if released.is_empty() {
        return;
    }
    drop(RiseAccountRuntime::current().spawn_blocking(move || drop(released)));
}

/// drops the oldest quarter of a read cache
/// the order list can hold forgotten digests, they get skipped and compacted here
fn evict_oldest<Entry>(cache: &mut HashMap<String, Entry>, order: &mut Vec<String>) -> Vec<Entry> {
    let target = (READ_CACHE_LIMIT / 4).max(1);
    let mut evicted = Vec::with_capacity(target);
    let mut consumed = 0;
    while evicted.len() < target && consumed < order.len() {
        if let Some(entry) = cache.remove(&order[consumed]) {
            evicted.push(entry);
        }
        consumed += 1;
    }
    order.drain(..consumed);
    if order.len() > READ_CACHE_LIMIT * 2 {
        order.retain(|digest| cache.contains_key(digest));
    }
    evicted
}

pub enum SaiWsDiskCache {}

impl SaiWsDiskCache {
    fn with_access_lock<T>(body: impl FnOnce() -> T) -> T {
        let _guard = ACCESS_LOCK.lock();
        body()
    }

    fn pending_write_value(cache_key: &str) -> Option<Value> {
        PENDING_WRITES.lock().entries.get(cache_key).map(|entry| entry.value.clone())
    }

    fn has_pending_write(cache_key: &str) -> bool {
        PENDING_WRITES.lock().entries.contains_key(cache_key)
    }

    fn drop_pending_write(cache_key: &str) {
        PENDING_WRITES.lock().entries.remove(cache_key);
    }

    fn cached_payload(digest: &str) -> Option<DiskReadEntry> {
        READ_CACHE.lock().payload.get(digest).cloned()
    }

    fn cached_meta(digest: &str) -> Option<MetaReadEntry> {
        READ_CACHE.lock().meta.get(digest).cloned()
    }

    fn remember_payload(entry: DiskReadEntry, digest: &str) {
        let evicted = {
            let mut cache = READ_CACHE.lock();
            let cache = &mut *cache;
            let mut evicted = Vec::new();
            if !cache.payload.contains_key(digest) {
                if cache.payload.len() >= READ_CACHE_LIMIT {
                    evicted = evict_oldest(&mut cache.payload, &mut cache.payload_order);
                }
                cache.payload_order.push(digest.to_owned());
            }
            cache.payload.insert(digest.to_owned(), entry);
            evicted
        };
        release_off_thread(evicted);
    }

    fn remember_meta(entry: MetaReadEntry, digest: &str) {
        let mut cache = READ_CACHE.lock();
        let cache = &mut *cache;
        if !cache.meta.contains_key(digest) {
            if cache.meta.len() >= READ_CACHE_LIMIT {
                let _ = evict_oldest(&mut cache.meta, &mut cache.meta_order);
            }
            cache.meta_order.push(digest.to_owned());
        }
        cache.meta.insert(digest.to_owned(), entry);
    }

    fn forget_read(digest: &str) {
        let mut cache = READ_CACHE.lock();
        cache.payload.remove(digest);
        cache.meta.remove(digest);
    }

    fn forget_all_reads() {
        let dropped: Vec<DiskReadEntry> = {
            let mut cache = READ_CACHE.lock();
            cache.meta.clear();
            cache.payload_order.clear();
            cache.meta_order.clear();
            cache.last_touched_at.clear();
            let dropped: Vec<DiskReadEntry> = cache.payload.drain().map(|(_, entry)| entry).collect();
            dropped
        };
        release_off_thread(dropped);
    }

    /// marks a file as recently used, written on the write lane and at most once per touch interval
    /// so a read never pays for an attribute write
    fn schedule_touch(url: PathBuf, key: String) {
        let now = RiseClock::now().time_interval_since_1970();
        let due = {
            let mut cache = READ_CACHE.lock();
            let last = cache.last_touched_at.get(&key).copied().unwrap_or(0.0);
            let due = now - last >= TOUCH_INTERVAL;
            if due {
                if cache.last_touched_at.len() >= READ_CACHE_LIMIT * 2 {
                    cache.last_touched_at.clear();
                }
                cache.last_touched_at.insert(key, now);
            }
            due
        };
        if !due {
            return;
        }
        Self::enqueue_write(Box::new(move || {
            Self::with_access_lock(|| {
                if FileManager.file_exists(&url) {
                    Self::touch_cache_item(&url);
                }
            });
        }));
    }

    fn file_key(url: &Path) -> String {
        url.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn base_directory() -> Result<PathBuf, SaiWsDiskCacheError> {
        if let Some(known) = READ_CACHE.lock().base_directory.clone() {
            return Ok(known);
        }
        let root = BASE_ROOT.as_ref().ok_or(SaiWsDiskCacheError::NoDataDirectory)?;
        let dir = root.join(SUBFOLDER);
        FileManager.create_directory(&dir, true)?;
        Self::configure_cache_item(&dir);
        READ_CACHE.lock().base_directory = Some(dir.clone());
        Ok(dir)
    }

    fn digest_hex(cache_key: &str) -> String {
        if let Some(known) = DIGEST_BY_KEY.lock().get(cache_key) {
            return known.clone();
        }
        let hex = sha256_hex(cache_key.as_bytes());
        let mut digests = DIGEST_BY_KEY.lock();
        if digests.len() >= DIGEST_CACHE_LIMIT {
            digests.clear();
        }
        digests.insert(cache_key.to_owned(), hex.clone());
        hex
    }

    fn key_index_url() -> Result<PathBuf, SaiWsDiskCacheError> {
        Ok(Self::base_directory()?.join(KEY_INDEX_FILE_NAME))
    }

    /// parses json bytes, bare fragments like a lone number are allowed
    fn json_fragment(data: &[u8]) -> Option<Value> {
        serde_json::from_slice(data).ok()
    }

    fn read_key_index() -> HashMap<String, String> {
        Self::with_access_lock(|| {
            let Ok(url) = Self::key_index_url() else {
                return HashMap::new();
            };
            if !FileManager.file_exists(&url) {
                return HashMap::new();
            }
            let Ok(data) = std::fs::read(&url) else {
                return HashMap::new();
            };
            let Some(Value::Object(object)) = Self::json_fragment(&data) else {
                return HashMap::new();
            };
            let mut dict = HashMap::with_capacity(object.len());
            for (digest, cache_key) in object {
                let Value::String(cache_key) = cache_key else {
                    return HashMap::new();
                };
                dict.insert(digest, cache_key);
            }
            dict
        })
    }

    fn write_key_index(dict: &HashMap<String, String>) -> Result<(), SaiWsDiskCacheError> {
        Self::with_access_lock(|| {
            let url = Self::key_index_url()?;
            let object: Map<String, Value> = dict
                .iter()
                .map(|(digest, cache_key)| (digest.clone(), Value::String(cache_key.clone())))
                .collect();
            let options = JSONWritingOptions {
                sorted_keys: true,
                ..JSONWritingOptions::default()
            };
            let data = JSONSerialization::data_with_json_object(&Value::Object(object), options)
                .ok_or(SaiWsDiskCacheError::Encoding)?;
            Self::write_atomically(&url, &data)?;
            Self::configure_cache_item(&url);
            Ok(())
        })
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut staging = url.as_os_str().to_owned();
        staging.push(".tmp");
        let staging = PathBuf::from(staging);
        std::fs::write(&staging, data)?;
        std::fs::rename(&staging, url)
    }

    fn set_index_mapping(cache_key: &str, digest_hex: &str) {
        if cache_key.is_empty() {
            return;
        }
        Self::with_access_lock(|| {
            let mut idx = Self::read_key_index();
            idx.insert(digest_hex.to_owned(), cache_key.to_owned());
            if let Ok(directory) = Self::base_directory() {
                idx = Self::trimmed_index(
                    &idx,
                    &Self::owner_scope_identifier(cache_key),
                    &directory,
                    &LIMITS,
                );
            }
            let _ = Self::write_key_index(&idx);
        });
    }

    fn remove_index_mapping(cache_key: &str) {
        Self::with_access_lock(|| {
            let hex = Self::digest_hex(cache_key);
            let mut idx = Self::read_key_index();
            if idx.remove(&hex).is_some() {
                let _ = Self::write_key_index(&idx);
            }
        });
    }

    fn file_url_for_digest_hex(hex: &str) -> Result<PathBuf, SaiWsDiskCacheError> {
        Ok(Self::base_directory()?.join(format!("{hex}{PAYLOAD_SUFFIX}")))
    }

    fn file_url_for_cache_key(cache_key: &str) -> Result<PathBuf, SaiWsDiskCacheError> {
        Self::file_url_for_digest_hex(&Self::digest_hex(cache_key))
    }

    fn meta_file_url_for_digest_hex(hex: &str) -> Result<PathBuf, SaiWsDiskCacheError> {
        Ok(Self::base_directory()?.join(format!("{hex}{OPTIMISTIC_META_SUFFIX}")))
    }

    fn meta_file_url_for_cache_key(cache_key: &str) -> Result<PathBuf, SaiWsDiskCacheError> {
        Self::meta_file_url_for_digest_hex(&Self::digest_hex(cache_key))
    }

    /// optimistic meta of a key, path to the array and path to the id
    pub fn load_optimistic_meta(cache_key: &str) -> Option<(Option<String>, Option<String>)> {
        if cache_key.is_empty() {
            return None;
        }
        let digest = Self::digest_hex(cache_key);
        if let Some(known) = Self::cached_meta(&digest) {
            return match known {
                MetaReadEntry::Absent => None,
                MetaReadEntry::Present { path_to_array, path_to_id } => {
                    if let Ok(url) = Self::meta_file_url_for_digest_hex(&digest) {
                        let key = Self::file_key(&url);
                        Self::schedule_touch(url, key);
                    }
                    Some((path_to_array, path_to_id))
                }
            };
        }
        Self::with_access_lock(|| {
            if let Some(known) = Self::cached_meta(&digest) {
                return match known {
                    MetaReadEntry::Present { path_to_array, path_to_id } => Some((path_to_array, path_to_id)),
                    MetaReadEntry::Absent => None,
                };
            }
            let url = Self::meta_file_url_for_digest_hex(&digest).ok()?;
            let obj = if FileManager.file_exists(&url) {
                std::fs::read(&url)
                    .ok()
                    .filter(|data| !data.is_empty())
                    .and_then(|data| Self::json_fragment(&data))
            } else {
                None
            };
            let Some(Value::Object(obj)) = obj else {
                Self::remember_meta(MetaReadEntry::Absent, &digest);
                return None;
            };
            let pa = obj.get(PATH_TO_ARRAY).and_then(Value::as_str).map(str::to_owned);
            let pi = obj.get(PATH_TO_ID).and_then(Value::as_str).map(str::to_owned);
            if pa.is_none() && pi.is_none() {
                Self::remember_meta(MetaReadEntry::Absent, &digest);
                return None;
            }
            Self::remember_meta(
                MetaReadEntry::Present { path_to_array: pa.clone(), path_to_id: pi.clone() },
                &digest,
            );
            let key = Self::file_key(&url);
            Self::schedule_touch(url, key);
            Some((pa, pi))
        })
    }

    pub fn save_optimistic_meta(
        cache_key: &str,
        path_to_array: Option<&str>,
        path_to_id: Option<&str>,
    ) -> Result<(), SaiWsDiskCacheError> {
        if cache_key.is_empty() {
            return Ok(());
        }
        let url = Self::meta_file_url_for_cache_key(cache_key)?;
        let mut dict = Map::new();
        if let Some(path_to_array) = path_to_array.filter(|path| !path.is_empty()) {
            dict.insert(PATH_TO_ARRAY.to_owned(), Value::from(path_to_array));
        }
        if let Some(path_to_id) = path_to_id.filter(|path| !path.is_empty()) {
            dict.insert(PATH_TO_ID.to_owned(), Value::from(path_to_id));
        }
        let meta_digest = Self::digest_hex(cache_key);
        if dict.is_empty() {
            Self::with_access_lock(|| {
                let _ = FileManager.remove_item(&url);
                Self::forget_read(&meta_digest);
            });
            return Ok(());
        }
        let options = JSONWritingOptions {
            sorted_keys: true,
            ..JSONWritingOptions::default()
        };
        let data = JSONSerialization::data_with_json_object(&Value::Object(dict), options)
            .ok_or(SaiWsDiskCacheError::Encoding)?;
        Self::with_access_lock(|| {
            let written = Self::write_atomically(&url, &data);
            Self::forget_read(&meta_digest);
            written?;
            Self::configure_cache_item(&url);
            Self::set_index_mapping(cache_key, &meta_digest);
            Ok(())
        })
    }

    fn remove_optimistic_meta_file_for_cache_key(cache_key: &str) {
        if cache_key.is_empty() {
            return;
        }
        Self::with_access_lock(|| {
            let Ok(url) = Self::meta_file_url_for_cache_key(cache_key) else {
                return;
            };
            let _ = FileManager.remove_item(&url);
            Self::forget_read(&Self::digest_hex(cache_key));
        });
    }

    fn remove_optimistic_meta_file_for_digest_hex(hex: &str) {
        Self::with_access_lock(|| {
            let Ok(url) = Self::meta_file_url_for_digest_hex(hex) else {
                return;
            };
            let _ = FileManager.remove_item(&url);
            Self::forget_read(hex);
        });
    }

    /// digest stems of the files remove_all_payload_files keeps for these keys
    pub fn payload_digests(for_cache_keys: &[String]) -> HashSet<String> {
        let mut digests = HashSet::new();
        for key in for_cache_keys {
            let trimmed = trimming_whitespaces_and_newlines(key);
            if trimmed.is_empty() {
                continue;
            }
            digests.insert(Self::digest_hex(trimmed));
        }
        digests
    }

    /// digest a payload or meta file is stored under, none for bookkeeping files
    pub fn payload_digest(of_file_name: &str) -> Option<String> {
        if of_file_name == KEY_INDEX_FILE_NAME {
            return None;
        }
        if let Some(stem) = of_file_name.strip_suffix(OPTIMISTIC_META_SUFFIX) {
            return Some(stem.to_owned());
        }
        of_file_name.strip_suffix(PAYLOAD_SUFFIX).map(str::to_owned)
    }

    pub fn all_indexed_cache_keys() -> Vec<String> {
        let unique: HashSet<String> = Self::read_key_index().into_values().collect();
        let mut keys: Vec<String> = unique.into_iter().collect();
        keys.sort();
        keys
    }

    pub fn remove_all_payload_files_and_index() {
        Self::remove_all_payload_files(&[]);
    }

    /// empties the store except the named keys
    /// some entries like chat scroll anchors cant be refetched so the caller says what survives
    pub fn remove_all_payload_files(preserving_cache_keys: &[String]) {
        {
            let preserved: HashSet<&str> = preserving_cache_keys.iter().map(String::as_str).collect();
            PENDING_WRITES.lock().entries.retain(|cache_key, _| preserved.contains(cache_key.as_str()));
        }
        Self::wait_for_asynchronous_writes();
        Self::with_access_lock(|| {
            Self::remove_all_payload_files_locked(preserving_cache_keys);
            Self::forget_all_reads();
        });
    }

    fn remove_all_payload_files_locked(preserving_cache_keys: &[String]) {
        {
            let Ok(dir) = Self::base_directory() else {
                return;
            };
            let Ok(names) = FileManager.contents_of_directory_at_path(&dir) else {
                return;
            };

            let preserved_digests = Self::payload_digests(preserving_cache_keys);

            for name in names {
                let Some(stem) = Self::payload_digest(&name) else {
                    continue;
                };
                if preserved_digests.contains(&stem) {
                    continue;
                }
                let _ = FileManager.remove_item(&dir.join(&name));
            }

            let index = Self::read_key_index();
            let kept: HashMap<String, String> = index
                .into_iter()
                .filter(|(digest, _)| preserved_digests.contains(digest))
                .collect();
            if kept.is_empty() {
                let _ = FileManager.remove_item(&dir.join(KEY_INDEX_FILE_NAME));
            } else {
                let _ = Self::write_key_index(&kept);
            }
        }
    }

    pub fn remove_all(cache_key_prefix: &str) {
        if cache_key_prefix.is_empty() {
            return;
        }
        Self::wait_for_asynchronous_writes();
        Self::with_access_lock(|| {
            let keys: Vec<String> = Self::all_indexed_cache_keys()
                .into_iter()
                .filter(|key| key.starts_with(cache_key_prefix))
                .collect();
            for key in keys {
                Self::remove(&key);
            }
        });
    }

    fn is_hex_digest64(stem: &str) -> bool {
        stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit())
    }

    pub fn debug_payload_digest_stems() -> Vec<String> {
        let Ok(dir) = Self::base_directory() else {
            return Vec::new();
        };
        let Ok(names) = FileManager.contents_of_directory_at_path(&dir) else {
            return Vec::new();
        };
        let mut stems: Vec<String> = names
            .iter()
            .filter(|name| name.as_str() != KEY_INDEX_FILE_NAME)
            .filter_map(|name| name.strip_suffix(PAYLOAD_SUFFIX))
            .filter(|stem| Self::is_hex_digest64(stem))
            .map(str::to_owned)
            .collect();
        stems.sort();
        stems
    }

    fn file_bytes(url: &Path) -> Option<i64> {
        FileManager.attributes_of_item(url).ok().map(|attributes| attributes.size)
    }

    pub fn debug_payload_file_bytes(for_cache_key: &str) -> Option<i64> {
        let url = Self::file_url_for_cache_key(for_cache_key).ok()?;
        if !FileManager.file_exists(&url) {
            return None;
        }
        Self::file_bytes(&url)
    }

    pub fn debug_list_payload_summaries() -> Vec<SaiWsDiskCacheDebugPayloadSummary> {
        let index = Self::read_key_index();
        Self::debug_payload_digest_stems()
            .into_iter()
            .map(|digest| {
                let bytes = Self::file_url_for_digest_hex(&digest)
                    .ok()
                    .and_then(|url| Self::file_bytes(&url))
                    .unwrap_or(0);
                SaiWsDiskCacheDebugPayloadSummary {
                    cache_key: index.get(&digest).cloned(),
                    digest,
                    file_bytes: bytes,
                }
            })
            .collect()
    }

    pub fn debug_body_text(digest_hex: &str) -> String {
        const MAX_CHARS: usize = 48_000;
        let parsed = Self::file_url_for_digest_hex(digest_hex)
            .ok()
            .and_then(|url| std::fs::read(url).ok())
            .and_then(|data| Self::json_fragment(&data));
        let Some(obj) = parsed else {
            return "(could not read or parse JSON)".to_owned();
        };
        let options = JSONWritingOptions {
            sorted_keys: true,
            pretty_printed: true,
            ..JSONWritingOptions::default()
        };
        if JSONSerialization::is_valid_json_object(&obj)
            && let Some(pretty) = JSONSerialization::data_with_json_object(&obj, options)
            && let Ok(s) = String::from_utf8(pretty)
        {
            let mut prefix = String::new();
            for (count, character) in characters(&s).enumerate() {
                if count == MAX_CHARS {
                    prefix.push('\u{2026}');
                    return prefix;
                }
                prefix.push_str(character.as_str());
            }
            return s;
        }
        match obj {
            Value::String(s) => s,
            other => other.to_string(),
        }
    }

    pub fn debug_folder_path_for_inspector() -> String {
        Self::base_directory()
            .map(|dir| dir.display().to_string())
            .unwrap_or_else(|_| "(Application Support unavailable)".to_owned())
    }

    pub fn storage_directory_url() -> Option<PathBuf> {
        Self::base_directory().ok()
    }

    pub fn remove_payload_file(digest_hex: &str) -> Option<String> {
        if !Self::is_hex_digest64(digest_hex) {
            return None;
        }
        Self::wait_for_asynchronous_writes();
        Self::with_access_lock(|| {
            let mut idx = Self::read_key_index();
            let ck = idx.remove(digest_hex);
            let _ = Self::write_key_index(&idx);
            if let Ok(url) = Self::file_url_for_digest_hex(digest_hex) {
                let _ = FileManager.remove_item(&url);
            }
            Self::forget_read(digest_hex);
            Self::remove_optimistic_meta_file_for_digest_hex(digest_hex);
            ck
        })
    }

    /// the pending payload as the disk will hold it, through the same sanitizers
    fn pending_as_on_disk(cache_key: &str, pending: Value) -> Value {
        SaiWsDiskPayloadSanitizer::prepare_from_disk(
            cache_key,
            SaiWsDiskPayloadSanitizer::prepare_for_disk(cache_key, pending),
        )
    }

    pub fn load(cache_key: &str) -> Option<Value> {
        if cache_key.is_empty() {
            return None;
        }
        if let Some(pending) = Self::pending_write_value(cache_key) {
            return Some(Self::pending_as_on_disk(cache_key, pending));
        }
        let digest = Self::digest_hex(cache_key);
        if let Some(known) = Self::cached_payload(&digest) {
            return match known {
                DiskReadEntry::Absent => None,
                DiskReadEntry::Present(value) => {
                    if let Ok(url) = Self::file_url_for_digest_hex(&digest) {
                        let key = Self::file_key(&url);
                        Self::schedule_touch(url, key);
                    }
                    Some(SaiWsDiskPayloadSanitizer::prepare_from_disk(cache_key, (*value).clone()))
                }
            };
        }
        Self::with_access_lock(|| {
            let url = Self::file_url_for_digest_hex(&digest).ok()?;
            if let Some(known) = Self::cached_payload(&digest) {
                let DiskReadEntry::Present(value) = known else {
                    return None;
                };
                return Some(SaiWsDiskPayloadSanitizer::prepare_from_disk(cache_key, (*value).clone()));
            }
            if !FileManager.file_exists(&url) {
                Self::remember_payload(DiskReadEntry::Absent, &digest);
                return None;
            }
            let data = std::fs::read(&url).ok()?;
            if data.is_empty() {
                Self::remember_payload(DiskReadEntry::Absent, &digest);
                return None;
            }
            let value = Self::json_fragment(&data)?;
            Self::remember_payload(DiskReadEntry::Present(Arc::new(value.clone())), &digest);
            let key = Self::file_key(&url);
            Self::schedule_touch(url, key);
            Some(SaiWsDiskPayloadSanitizer::prepare_from_disk(cache_key, value))
        })
    }

    pub fn load_if_known(cache_key: &str) -> (bool, Option<Value>) {
        if cache_key.is_empty() {
            return (true, None);
        }
        if let Some(pending) = Self::pending_write_value(cache_key) {
            return (true, Some(Self::pending_as_on_disk(cache_key, pending)));
        }
        match Self::cached_payload(&Self::digest_hex(cache_key)) {
            None => (false, None),
            Some(DiskReadEntry::Absent) => (true, None),
            Some(DiskReadEntry::Present(value)) => {
                (true, Some(SaiWsDiskPayloadSanitizer::prepare_from_disk(cache_key, (*value).clone())))
            }
        }
    }

    pub fn warm_asynchronously(
        cache_key: String,
        completion: impl FnOnce(Option<Value>, &mut App) + 'static,
        cx: &mut App,
    ) {
        let read = cx.background_executor().spawn(async move {
            let value = Self::load(&cache_key);
            let _ = Self::load_optimistic_meta(&cache_key);
            value
        });
        cx.spawn(async move |cx| {
            let value = read.await;
            cx.update(|cx| completion(value, cx));
        })
        .detach();
    }

    pub fn contains(cache_key: &str) -> bool {
        if cache_key.is_empty() {
            return false;
        }
        if Self::has_pending_write(cache_key) {
            return true;
        }
        let digest = Self::digest_hex(cache_key);
        if let Some(known) = Self::cached_payload(&digest) {
            return matches!(known, DiskReadEntry::Present(_));
        }
        let Ok(url) = Self::file_url_for_digest_hex(&digest) else {
            return false;
        };
        Self::with_access_lock(|| {
            let exists = FileManager.file_exists(&url);
            if !exists {
                Self::remember_payload(DiskReadEntry::Absent, &digest);
            }
            exists
        })
    }

    /// reads on the concurrent queue go straight to the background executor
    /// they never wait behind the write lane
    pub fn load_asynchronously(
        cache_key: String,
        completion: impl FnOnce(Option<Value>, &mut App) + 'static,
        cx: &mut App,
    ) {
        let read = cx.background_executor().spawn(async move { Self::load(&cache_key) });
        cx.spawn(async move |cx| {
            let value = read.await;
            cx.update(|cx| completion(value, cx));
        })
        .detach();
    }

    pub fn save(cache_key: &str, value: &Value) -> Result<(), SaiWsDiskCacheError> {
        if cache_key.is_empty() {
            return Ok(());
        }
        let url = Self::file_url_for_cache_key(cache_key)?;
        let fragment = JSONWritingOptions {
            fragments_allowed: true,
            ..JSONWritingOptions::default()
        };
        let data = match value {
            Value::Object(_) | Value::Array(_) | Value::String(_) | Value::Number(_) | Value::Bool(_) => {
                JSONSerialization::data_with_json_object(value, fragment)
            }
            Value::Null => {
                let mut wrapped = Map::new();
                wrapped.insert(WS_CACHE_VALUE.to_owned(), Value::from("<null>"));
                JSONSerialization::data_with_json_object(&Value::Object(wrapped), JSONWritingOptions::default())
            }
        }
        .ok_or(SaiWsDiskCacheError::Encoding)?;
        let digest = Self::digest_hex(cache_key);
        Self::with_access_lock(|| {
            let written = Self::write_atomically(&url, &data);
            Self::forget_read(&digest);
            written?;
            Self::configure_cache_item(&url);
            Self::set_index_mapping(cache_key, &digest);
            Ok(())
        })
    }

    pub fn save_asynchronously(
        cache_key: String,
        value: Value,
        completion: impl FnOnce(Result<(), SaiWsDiskCacheError>, &mut App) + 'static,
        cx: &mut App,
    ) {
        Self::save_asynchronously_prepare(cache_key, value, |value| value, completion, cx);
    }

    pub fn save_asynchronously_prepare(
        cache_key: String,
        value: Value,
        prepare: impl FnOnce(Value) -> Value + Send + 'static,
        completion: impl FnOnce(Result<(), SaiWsDiskCacheError>, &mut App) + 'static,
        cx: &mut App,
    ) {
        let (sender, receiver) = oneshot::channel::<Result<(), SaiWsDiskCacheError>>();
        Self::submit_write(
            Box::new(move || {
                let prepared = prepare(value);
                let disk_payload = SaiWsDiskPayloadSanitizer::prepare_for_disk(&cache_key, prepared);
                let _ = sender.send(Self::save(&cache_key, &disk_payload));
            }),
            cx,
        );
        cx.spawn(async move |cx| {
            let Ok(result) = receiver.await else {
                return;
            };
            cx.update(|cx| completion(result, cx));
        })
        .detach();
    }

    pub fn save_deferred(cache_key: String, value: Value, cx: &mut App) {
        if cache_key.is_empty() {
            return;
        }
        let generation = {
            let mut pending = PENDING_WRITES.lock();
            pending.generation = pending.generation.wrapping_add(1);
            let generation = pending.generation;
            pending.entries.insert(cache_key.clone(), PendingWrite { generation, value });
            generation
        };
        let (sender, receiver) = oneshot::channel::<String>();
        Self::submit_write(
            Box::new(move || {
                let value = {
                    let pending = PENDING_WRITES.lock();
                    match pending.entries.get(&cache_key) {
                        Some(entry) if entry.generation == generation => entry.value.clone(),
                        _ => return,
                    }
                };
                let disk_payload = SaiWsDiskPayloadSanitizer::prepare_for_disk(&cache_key, value);
                if let Err(error) = Self::save(&cache_key, &disk_payload) {
                    let _ = sender.send(format!("deferred disk persistence failed for {cache_key}: {error}"));
                }
                let mut pending = PENDING_WRITES.lock();
                if pending.entries.get(&cache_key).is_some_and(|entry| entry.generation == generation) {
                    pending.entries.remove(&cache_key);
                }
            }),
            cx,
        );
        cx.spawn(async move |cx| {
            let Ok(message) = receiver.await else {
                return;
            };
            cx.update(|cx| Sai::log("SaiWs", move || message, SaiLogKind::Cache, cx));
        })
        .detach();
    }

    pub fn save_optimistic_meta_deferred(
        cache_key: String,
        path_to_array: Option<String>,
        path_to_id: Option<String>,
        cx: &App,
    ) {
        if cache_key.is_empty() {
            return;
        }
        Self::submit_write(
            Box::new(move || {
                let _ = Self::save_optimistic_meta(&cache_key, path_to_array.as_deref(), path_to_id.as_deref());
            }),
            cx,
        );
    }

    fn submit_write(job: WriteJob, cx: &App) {
        let start_drainer = {
            let mut lane = WRITE_LANE.lock();
            lane.jobs.push_back(job);
            !std::mem::replace(&mut lane.draining, true)
        };
        if start_drainer {
            cx.background_executor()
                .spawn(async move { Self::run_write_lane() })
                .detach();
        }
    }

    fn enqueue_write(job: WriteJob) {
        let start_drainer = {
            let mut lane = WRITE_LANE.lock();
            lane.jobs.push_back(job);
            !std::mem::replace(&mut lane.draining, true)
        };
        if start_drainer {
            drop(RiseAccountRuntime::current().spawn_blocking(Self::run_write_lane));
        }
    }

    fn run_write_lane() {
        loop {
            let _io = WRITE_IO.lock();
            let job = {
                let mut lane = WRITE_LANE.lock();
                match lane.jobs.pop_front() {
                    Some(job) => job,
                    None => {
                        lane.draining = false;
                        return;
                    }
                }
            };
            let _mark = WriteLaneMark::enter();
            job();
        }
    }

    /// blocks until every write queued before the call has landed
    /// runs queued work itself instead of parking, so a test executor doesnt wait on itself
    pub fn wait_for_asynchronous_writes() {
        if ON_WRITE_LANE.with(Cell::get) {
            return;
        }
        loop {
            let _io = WRITE_IO.lock();
            let job = WRITE_LANE.lock().jobs.pop_front();
            let Some(job) = job else {
                return;
            };
            let _mark = WriteLaneMark::enter();
            job();
        }
    }

    pub fn remove(cache_key: &str) {
        if cache_key.is_empty() {
            return;
        }
        Self::drop_pending_write(cache_key);
        Self::wait_for_asynchronous_writes();
        Self::with_access_lock(|| {
            let Ok(url) = Self::file_url_for_cache_key(cache_key) else {
                return;
            };
            let _ = FileManager.remove_item(&url);
            Self::forget_read(&Self::digest_hex(cache_key));
            Self::remove_optimistic_meta_file_for_cache_key(cache_key);
            Self::remove_index_mapping(cache_key);
        });
    }

    pub fn owner_scope_identifier(for_cache_key: &str) -> String {
        let Some(remainder) = for_cache_key.strip_prefix(ACCOUNT_PREFIX) else {
            return UNSCOPED_OWNER.to_owned();
        };
        let Some(separator) = remainder.find(':') else {
            return UNSCOPED_OWNER.to_owned();
        };
        for_cache_key[..ACCOUNT_PREFIX.len() + separator + 1].to_owned()
    }

    pub fn eviction_digests(records: &[SaiWsDiskCacheLRURecord], limits: &SaiWsDiskCacheLimits) -> Vec<String> {
        let mut remaining_count = i64::try_from(records.len()).unwrap_or(i64::MAX);
        let mut remaining_bytes = records
            .iter()
            .fold(0_i64, |partial, record| partial.saturating_add(record.byte_count.max(0)));
        if remaining_count <= limits.maximum_entry_count_per_account
            && remaining_bytes <= limits.maximum_total_bytes_per_account
        {
            return Vec::new();
        }

        let mut oldest_first: Vec<&SaiWsDiskCacheLRURecord> = records.iter().collect();
        oldest_first.sort_by(|lhs, rhs| {
            if lhs.last_accessed_at == rhs.last_accessed_at {
                return lhs.digest_hex.cmp(&rhs.digest_hex);
            }
            lhs.last_accessed_at.total_cmp(&rhs.last_accessed_at)
        });
        let mut evicted = Vec::new();
        for record in oldest_first {
            if remaining_count <= limits.maximum_entry_count_per_account
                && remaining_bytes <= limits.maximum_total_bytes_per_account
            {
                break;
            }
            evicted.push(record.digest_hex.clone());
            remaining_count -= 1;
            remaining_bytes = (remaining_bytes - record.byte_count.max(0)).max(0);
        }
        evicted
    }

    fn trimmed_index(
        raw_index: &HashMap<String, String>,
        owner_scope: &str,
        directory: &Path,
        limits: &SaiWsDiskCacheLimits,
    ) -> HashMap<String, String> {
        let mut index = raw_index.clone();
        let mut records = Vec::new();

        for (digest, cache_key) in raw_index {
            if Self::owner_scope_identifier(cache_key) != owner_scope {
                continue;
            }
            let payload = directory.join(format!("{digest}{PAYLOAD_SUFFIX}"));
            let meta = directory.join(format!("{digest}{OPTIMISTIC_META_SUFFIX}"));
            let payload_stats = Self::cache_file_stats(&payload);
            let meta_stats = Self::cache_file_stats(&meta);
            if payload_stats.is_none() && meta_stats.is_none() {
                index.remove(digest);
                continue;
            }
            if payload_stats.is_some() {
                Self::configure_cache_item(&payload);
            }
            if meta_stats.is_some() {
                Self::configure_cache_item(&meta);
            }
            let payload_bytes = payload_stats.map_or(0, |stats| stats.0);
            let meta_bytes = meta_stats.map_or(0, |stats| stats.0);
            records.push(SaiWsDiskCacheLRURecord {
                digest_hex: digest.clone(),
                byte_count: payload_bytes.saturating_add(meta_bytes),
                last_accessed_at: payload_stats
                    .map_or(0.0, |stats| stats.1)
                    .max(meta_stats.map_or(0.0, |stats| stats.1)),
            });
        }

        for digest in Self::eviction_digests(&records, limits) {
            let payload = directory.join(format!("{digest}{PAYLOAD_SUFFIX}"));
            let meta = directory.join(format!("{digest}{OPTIMISTIC_META_SUFFIX}"));
            let _ = FileManager.remove_item(&payload);
            let _ = FileManager.remove_item(&meta);
            Self::forget_read(&digest);
            if !FileManager.file_exists(&payload) && !FileManager.file_exists(&meta) {
                index.remove(&digest);
            }
        }
        index
    }

    /// size in bytes and modified time of a regular file
    fn cache_file_stats(url: &Path) -> Option<(i64, TimeInterval)> {
        let metadata = std::fs::metadata(url).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let modified_at = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map_or(0.0, |since| since.as_secs_f64());
        Some((i64::try_from(metadata.len()).unwrap_or(i64::MAX), modified_at))
    }

    fn touch_cache_item(url: &Path) {
        let now_ms = u64::try_from(RiseClock::now().ms()).unwrap_or(0);
        let now = UNIX_EPOCH + Duration::from_millis(now_ms);
        if let Ok(file) = File::options().write(true).open(url) {
            let _ = file.set_modified(now);
        }
        Self::configure_cache_item(url);
    }

    /// cache item setup, desktop has no per file protection so the user data dir is the boundary
    fn configure_cache_item(url: &Path) {
        let values = URLResourceValues {
            is_excluded_from_backup: Some(true),
        };
        let _ = set_resource_values(url, &values);
    }
}
