use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::FileTimes;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::AppPaths;
use rise_platform::file_manager::{
    CocoaError, CocoaErrorCode, DirectoryEnumerationOptions, FileManager,
};
use rise_platform::paths::temporary_directory;

use crate::core::config::app_environment::AppEnvironment;

static CACHE_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::compiled().canon())
        .ok()
        .map(|paths| paths.cache_dir().to_path_buf())
});

static FILE_LOCK: LazyLock<Mutex<MusicPlaybackDiskCacheLockedState>> = LazyLock::new(|| {
    Mutex::new(MusicPlaybackDiskCacheLockedState {
        prepared_directories: HashSet::new(),
        legacy_temp_file_names_by_track_id: None,
    })
});

struct MusicPlaybackDiskCacheLockedState {
    prepared_directories: HashSet<PathBuf>,
    legacy_temp_file_names_by_track_id: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug)]
struct CacheFileRecord {
    url: PathBuf,
    bytes: i64,
    modified_at: Date,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicPlaybackDiskCacheListedEntry {
    pub id: String,
    pub track_id: String,
    pub file_name: String,
    pub file_bytes: i64,
    pub format_label: String,
    pub modified_at: Date,
}

impl MusicPlaybackDiskCacheListedEntry {
    pub fn cache_key(&self) -> &str {
        &self.track_id
    }
}

/// disk cache of played audio, one folder per account
/// everything is blocking file io so call it off the main thread
pub enum MusicPlaybackDiskCache {}

impl MusicPlaybackDiskCache {
    const SUBFOLDER: &'static str = "MusicPlaybackCache";
    const INDEX_FILE_NAME: &'static str = "_track_index.json";
    const FILE_PREFIX: &'static str = "music-";
    const ACCOUNTS_FOLDER: &'static str = "accounts";
    const MINIMUM_VALID_FILE_BYTES: i64 = 1_024;
    pub const MAXIMUM_FILE_BYTE_COUNT: i64 = 512 * 1_024 * 1_024;
    const MAXIMUM_ENTRY_COUNT: usize = 1_200;
    const MAXIMUM_TOTAL_BYTES: i64 = 2 * 1_024 * 1_024 * 1_024;
    const AMBIENT_KEY_PREFIX: &'static str = "ambient:";
    const AMBIENT_FILE_PREFIX: &'static str = "music-amb-";
    const AMBIENT_MAXIMUM_ENTRY_COUNT: usize = 120;
    const AMBIENT_MAXIMUM_TOTAL_BYTES: i64 = 200 * 1_024 * 1_024;
    const ANONYMOUS_ACCOUNT: &'static str = "anonymous";
    const LEGACY_TEMP_PREFIX: &'static str = "music-preview-";
    const INCOMING_PREFIX: &'static str = ".incoming-";
    const UNAVAILABLE_FOLDER: &'static str = "(unavailable)";
    const FILE_HEAD_BYTES: usize = 16;

    fn root_directory() -> Result<PathBuf, CocoaError> {
        let root = CACHE_ROOT
            .as_ref()
            .ok_or_else(|| CocoaError::new(CocoaErrorCode::FileNoSuchFile))?;
        let dir = root.join(Self::SUBFOLDER);
        Self::ensure_directory(&dir)?;
        Ok(dir)
    }

    fn base_directory(
        state: &mut MusicPlaybackDiskCacheLockedState,
        user_id: Option<&str>,
    ) -> Result<PathBuf, CocoaError> {
        let root = Self::root_directory()?;
        let accounts = root.join(Self::ACCOUNTS_FOLDER);
        Self::ensure_directory(&accounts)?;
        let directory = accounts.join(Self::account_directory_name(user_id));
        Self::ensure_directory(&directory)?;
        Self::prepare_directory_if_needed(state, &directory);
        Ok(directory)
    }

    fn account_directory_name(user_id: Option<&str>) -> String {
        let value = user_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if value.is_empty() {
            return Self::ANONYMOUS_ACCOUNT.to_owned();
        }
        sha256_hex(value.as_bytes())
    }

    fn index_url(directory: &Path) -> PathBuf {
        directory.join(Self::INDEX_FILE_NAME)
    }

    fn read_index(directory: &Path) -> HashMap<String, String> {
        let url = Self::index_url(directory);
        if !FileManager.file_exists(&url) {
            return HashMap::new();
        }
        std::fs::read(&url)
            .ok()
            .and_then(|data| serde_json::from_slice::<HashMap<String, String>>(&data).ok())
            .unwrap_or_default()
    }

    fn write_index(dict: &HashMap<String, String>, directory: &Path) -> std::io::Result<()> {
        let url = Self::index_url(directory);
        let sorted: BTreeMap<&String, &String> = dict.iter().collect();
        let data = serde_json::to_vec(&sorted).map_err(std::io::Error::other)?;
        Self::write_atomically(&url, &data)?;
        Self::apply_storage_attributes(&url);
        Ok(())
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

    fn ensure_directory(url: &Path) -> Result<(), CocoaError> {
        FileManager.create_directory(url, true)?;
        Self::apply_storage_attributes(url);
        Ok(())
    }

    fn apply_storage_attributes(_url: &Path) {}

    fn prepare_directory_if_needed(state: &mut MusicPlaybackDiskCacheLockedState, directory: &Path) {
        if !state.prepared_directories.insert(directory.to_path_buf()) {
            return;
        }
        let records = Self::cache_file_records(directory, true);
        let index = Self::reconcile_index(Self::read_index(directory), directory, Some(&records));
        let index = Self::prune_cache(directory, index, Some(&records));
        let _ = Self::write_index(&index, directory);
    }

    /// applying_attributes costs a lot, use it only on the one sweep per process that repairs old files
    /// writers stamp their own file via touch
    fn cache_file_records(directory: &Path, applying_attributes: bool) -> Vec<CacheFileRecord> {
        let Ok(urls) =
            FileManager.contents_of_directory_at(directory, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
        else {
            return Vec::new();
        };
        urls.into_iter()
            .filter_map(|url| {
                let name = Self::last_path_component(&url);
                if !name.starts_with(Self::FILE_PREFIX) {
                    return None;
                }
                let metadata = std::fs::metadata(&url).ok()?;
                if !metadata.is_file() {
                    return None;
                }
                let bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
                if bytes <= Self::MINIMUM_VALID_FILE_BYTES || bytes > Self::MAXIMUM_FILE_BYTE_COUNT {
                    let _ = FileManager.remove_item(&url);
                    return None;
                }
                if applying_attributes {
                    Self::apply_storage_attributes(&url);
                }
                let accessed = metadata.accessed().ok().map_or_else(Date::distant_past, Self::date_from_system_time);
                let modified = metadata.modified().ok().map_or_else(Date::distant_past, Self::date_from_system_time);
                Some(CacheFileRecord {
                    url,
                    bytes,
                    modified_at: accessed.max(modified),
                })
            })
            .collect()
    }

    fn reconcile_index(
        index: HashMap<String, String>,
        directory: &Path,
        records: Option<&[CacheFileRecord]>,
    ) -> HashMap<String, String> {
        let loaded;
        let records = match records {
            Some(records) => records,
            None => {
                loaded = Self::cache_file_records(directory, false);
                &loaded
            }
        };
        let existing_names: HashSet<String> = records
            .iter()
            .map(|record| Self::last_path_component(&record.url))
            .collect();
        index
            .into_iter()
            .filter(|(track_id, file_name)| {
                !track_id.is_empty()
                    && Self::is_bare_file_name(file_name)
                    && file_name.starts_with(Self::FILE_PREFIX)
                    && existing_names.contains(file_name)
            })
            .collect()
    }

    pub fn ambient_key(track_id: &str) -> String {
        format!("{}{track_id}", Self::AMBIENT_KEY_PREFIX)
    }

    fn is_ambient_file(name: &str) -> bool {
        name.starts_with(Self::AMBIENT_FILE_PREFIX)
    }

    fn prune_cache(
        directory: &Path,
        index: HashMap<String, String>,
        preloaded: Option<&[CacheFileRecord]>,
    ) -> HashMap<String, String> {
        let records = match preloaded {
            Some(records) => records.to_vec(),
            None => Self::cache_file_records(directory, false),
        };
        let (ambient, regular): (Vec<CacheFileRecord>, Vec<CacheFileRecord>) = records
            .into_iter()
            .partition(|record| Self::is_ambient_file(&Self::last_path_component(&record.url)));
        let mut removed_names =
            Self::prune_lane(regular, Self::MAXIMUM_ENTRY_COUNT, Self::MAXIMUM_TOTAL_BYTES);
        removed_names.extend(Self::prune_lane(
            ambient,
            Self::AMBIENT_MAXIMUM_ENTRY_COUNT,
            Self::AMBIENT_MAXIMUM_TOTAL_BYTES,
        ));
        if removed_names.is_empty() {
            return index;
        }
        index
            .into_iter()
            .filter(|(_, file_name)| !removed_names.contains(file_name))
            .collect()
    }

    fn prune_lane(
        mut lane: Vec<CacheFileRecord>,
        maximum_entry_count: usize,
        maximum_total_bytes: i64,
    ) -> HashSet<String> {
        lane.sort_by(|lhs, rhs| {
            lhs.modified_at
                .cmp(&rhs.modified_at)
                .then_with(|| Self::last_path_component(&lhs.url).cmp(&Self::last_path_component(&rhs.url)))
        });
        let mut total_bytes: i64 = lane.iter().map(|record| record.bytes).sum();
        let mut removed_names = HashSet::new();
        let mut remaining = lane.len();
        for record in lane {
            if remaining <= maximum_entry_count && total_bytes <= maximum_total_bytes {
                break;
            }
            remaining -= 1;
            let removed = FileManager.remove_item(&record.url).is_ok() || !FileManager.file_exists(&record.url);
            if removed {
                total_bytes = (total_bytes - record.bytes).max(0);
                removed_names.insert(Self::last_path_component(&record.url));
            }
        }
        removed_names
    }

    /// marks a file as used for eviction order
    /// uses access date not modified date, else every cache hit looked like a fresh download
    fn touch(url: &Path) {
        let accessed = Self::system_time_from_date(RiseClock::now());
        if let Ok(file) = std::fs::File::options().write(true).open(url) {
            let _ = file.set_times(FileTimes::new().set_accessed(accessed));
        }
        Self::apply_storage_attributes(url);
    }

    fn file_name(track_id: &str, ext: &str) -> String {
        let safe_ext: String = if ext.is_empty() {
            "mp3".to_owned()
        } else {
            ext.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(8)
                .collect()
        };
        format!("{}.{safe_ext}", Self::file_stem(track_id))
    }

    fn file_stem(track_id: &str) -> String {
        let digest = sha256_hex(track_id.as_bytes());
        let prefix = if track_id.starts_with(Self::AMBIENT_KEY_PREFIX) {
            Self::AMBIENT_FILE_PREFIX
        } else {
            Self::FILE_PREFIX
        };
        format!("{prefix}{digest}")
    }

    pub fn file_url(track_id: &str, ext: &str, user_id: Option<&str>) -> Result<PathBuf, CocoaError> {
        Self::with_file_lock(|state| {
            Ok(Self::base_directory(state, user_id)?.join(Self::file_name(track_id, ext)))
        })
    }

    pub fn find_cached_url(track_id: &str, user_id: Option<&str>) -> Option<PathBuf> {
        Self::with_file_lock(|state| {
            let directory = Self::base_directory(state, user_id).ok()?;
            Self::find_cached_url_in(track_id, &directory)
        })
    }

    fn find_cached_url_in(track_id: &str, directory: &Path) -> Option<PathBuf> {
        let mut index = Self::read_index(directory);
        let mut index_changed = false;
        if let Some(indexed) = index.get(track_id).cloned() {
            let url = directory.join(&indexed);
            let safe_name = Self::is_bare_file_name(&indexed) && indexed.starts_with(Self::FILE_PREFIX);
            if safe_name {
                let metadata = std::fs::metadata(&url).ok();
                let is_regular_file = metadata.as_ref().is_some_and(std::fs::Metadata::is_file);
                let size = metadata
                    .as_ref()
                    .map_or(0, |metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX));
                if is_regular_file
                    && size > Self::MINIMUM_VALID_FILE_BYTES
                    && size <= Self::MAXIMUM_FILE_BYTE_COUNT
                {
                    Self::touch(&url);
                    return Some(url);
                }
                if is_regular_file {
                    let _ = FileManager.remove_item(&url);
                }
            }
            index.remove(track_id);
            index_changed = true;
        }
        let Ok(names) = FileManager.contents_of_directory_at_path(directory) else {
            if index_changed {
                let _ = Self::write_index(&index, directory);
            }
            return None;
        };
        let safe_prefix = format!("{}.", Self::file_stem(track_id));
        let legacy_prefix = format!("{}{track_id}.", Self::FILE_PREFIX);
        let mut candidates: Vec<CacheFileRecord> = names
            .into_iter()
            .filter(|name| name.starts_with(&safe_prefix) || name.starts_with(&legacy_prefix))
            .filter_map(|name| {
                let url = directory.join(&name);
                let metadata = std::fs::metadata(&url).ok()?;
                if !metadata.is_file() {
                    return None;
                }
                let bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
                if bytes <= Self::MINIMUM_VALID_FILE_BYTES || bytes > Self::MAXIMUM_FILE_BYTE_COUNT {
                    let _ = FileManager.remove_item(&url);
                    return None;
                }
                Some(CacheFileRecord {
                    url,
                    bytes,
                    modified_at: metadata
                        .modified()
                        .ok()
                        .map_or_else(Date::distant_past, Self::date_from_system_time),
                })
            })
            .collect();
        candidates.sort_by(|lhs, rhs| rhs.modified_at.cmp(&lhs.modified_at));
        let Some(candidate) = candidates.into_iter().next() else {
            if index_changed {
                let _ = Self::write_index(&index, directory);
            }
            return None;
        };
        index.insert(track_id.to_owned(), Self::last_path_component(&candidate.url));
        Self::touch(&candidate.url);
        let _ = Self::write_index(&index, directory);
        Some(candidate.url)
    }

    pub fn register_saved_file(track_id: &str, file_name: &str, user_id: Option<&str>) {
        Self::with_file_lock(|state| {
            let Ok(directory) = Self::base_directory(state, user_id) else {
                return;
            };
            let url = directory.join(file_name);
            if !Self::is_bare_file_name(file_name) || !file_name.starts_with(Self::FILE_PREFIX) {
                return;
            }
            let size_is_valid = FileManager.attributes_of_item(&url).is_ok_and(|attributes| {
                attributes.size > Self::MINIMUM_VALID_FILE_BYTES
                    && attributes.size <= Self::MAXIMUM_FILE_BYTE_COUNT
            });
            if !size_is_valid {
                let _ = FileManager.remove_item(&url);
                return;
            }
            Self::touch(&url);
            let records = Self::cache_file_records(&directory, false);
            let mut index = Self::reconcile_index(Self::read_index(&directory), &directory, Some(&records));
            index.insert(track_id.to_owned(), file_name.to_owned());
            let index = Self::prune_cache(&directory, index, Some(&records));
            let _ = Self::write_index(&index, &directory);
        });
    }

    pub fn relocate_to_match_format_if_needed(track_id: &str, cached_file: &Path, format_label: &str) -> PathBuf {
        let fallback = cached_file
            .extension()
            .map(|ext| ext.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = Self::file_extension(format_label, &fallback);
        Self::with_file_lock(|_| {
            let Some(directory) = cached_file.parent().map(Path::to_path_buf) else {
                return cached_file.to_path_buf();
            };
            let target = directory.join(Self::file_name(track_id, &ext));
            if target == cached_file {
                Self::touch(cached_file);
                return cached_file.to_path_buf();
            }
            let _ = FileManager.remove_item(&target);
            if FileManager.move_item(cached_file, &target).is_err() {
                return cached_file.to_path_buf();
            }
            Self::touch(&target);
            let cached_name = Self::last_path_component(cached_file);
            let target_name = Self::last_path_component(&target);
            let records = Self::cache_file_records(&directory, false);
            let mut index: HashMap<String, String> =
                Self::reconcile_index(Self::read_index(&directory), &directory, Some(&records))
                    .into_iter()
                    .filter(|(_, file_name)| *file_name != cached_name && *file_name != target_name)
                    .collect();
            index.insert(track_id.to_owned(), target_name);
            let index = Self::prune_cache(&directory, index, Some(&records));
            let _ = Self::write_index(&index, &directory);
            target
        })
    }

    fn file_extension(format: &str, fallback: &str) -> String {
        match format {
            "m4a/mp4" => "m4a".to_owned(),
            "flac" => "flac".to_owned(),
            "ogg" => "ogg".to_owned(),
            "wav" => "wav".to_owned(),
            "mp3-id3" | "mp3-frame" => "mp3".to_owned(),
            _ if fallback.is_empty() => "mp3".to_owned(),
            _ => fallback.to_owned(),
        }
    }

    pub fn remove_file(url: &Path) {
        let name = Self::last_path_component(url);
        if !name.starts_with(Self::FILE_PREFIX) {
            return;
        }
        Self::with_file_lock(|_| {
            let Some(directory) = url.parent() else {
                return;
            };
            let _ = FileManager.remove_item(url);
            let index = Self::read_index(directory);
            let count = index.len();
            let filtered: HashMap<String, String> =
                index.into_iter().filter(|(_, file_name)| *file_name != name).collect();
            if filtered.len() != count {
                let _ = Self::write_index(&filtered, directory);
            }
        });
    }

    pub fn clear_track(track_id: &str, user_id: Option<&str>) {
        Self::with_file_lock(|state| {
            let Ok(directory) = Self::base_directory(state, user_id) else {
                return;
            };
            if let Some(url) = Self::find_cached_url_in(track_id, &directory) {
                let _ = FileManager.remove_item(&url);
            }
            let mut index = Self::read_index(&directory);
            index.remove(track_id);
            let _ = Self::write_index(&index, &directory);
        });
        Self::purge_legacy_temp_files(track_id);
    }

    pub fn clear_all(user_id: Option<&str>) {
        Self::with_file_lock(|state| {
            let Ok(directory) = Self::base_directory(state, user_id) else {
                return;
            };
            if let Ok(items) =
                FileManager.contents_of_directory_at(&directory, DirectoryEnumerationOptions::default())
            {
                for url in items {
                    let _ = FileManager.remove_item(&url);
                }
            }
            state.prepared_directories.remove(&directory);
            let _ = Self::ensure_directory(&directory);
            Self::prepare_directory_if_needed(state, &directory);
        });
        Self::purge_all_legacy_temp_files();
    }

    pub fn remove_persisted_account(user_id: &str) {
        Self::with_file_lock(|state| {
            let Ok(root) = Self::root_directory() else {
                return;
            };
            let directory = root
                .join(Self::ACCOUNTS_FOLDER)
                .join(Self::account_directory_name(Some(user_id)));
            let _ = FileManager.remove_item(&directory);
            state.prepared_directories.remove(&directory);
        });
    }

    pub fn folder_path_for_inspector(user_id: Option<&str>) -> String {
        Self::with_file_lock(|state| {
            Self::base_directory(state, user_id)
                .map(|directory| directory.to_string_lossy().into_owned())
                .unwrap_or_else(|_| Self::UNAVAILABLE_FOLDER.to_owned())
        })
    }

    pub fn storage_directory_url(user_id: Option<&str>) -> Option<PathBuf> {
        let root = CACHE_ROOT.as_ref()?;
        Some(
            root.join(Self::SUBFOLDER)
                .join(Self::ACCOUNTS_FOLDER)
                .join(Self::account_directory_name(user_id)),
        )
    }

    fn legacy_temp_file_names() -> HashMap<String, String> {
        if let Some(cached) = Self::with_file_lock(|state| state.legacy_temp_file_names_by_track_id.clone()) {
            return cached;
        }
        let mut index = HashMap::new();
        let names = FileManager
            .contents_of_directory_at_path(&temporary_directory())
            .unwrap_or_default();
        for name in names {
            let Some(rest) = name.strip_prefix(Self::LEGACY_TEMP_PREFIX) else {
                continue;
            };
            let Some(dot) = rest.rfind('.') else {
                continue;
            };
            let track_id = &rest[..dot];
            if track_id.is_empty() || index.contains_key(track_id) {
                continue;
            }
            index.insert(track_id.to_owned(), name.clone());
        }
        Self::with_file_lock(|state| state.legacy_temp_file_names_by_track_id = Some(index.clone()));
        index
    }

    pub fn adopt_legacy_temp_file_if_needed(
        track_id: &str,
        preferred_ext: &str,
        user_id: Option<&str>,
    ) -> Option<PathBuf> {
        let legacy_name = Self::legacy_temp_file_names().get(track_id).cloned()?;
        Self::with_file_lock(|state| {
            if let Some(names) = state.legacy_temp_file_names_by_track_id.as_mut() {
                names.remove(track_id);
            }
        });
        let legacy_url = temporary_directory().join(&legacy_name);
        let legacy_bytes = FileManager
            .attributes_of_item(&legacy_url)
            .map_or(0, |attributes| attributes.size);
        if legacy_bytes <= Self::MINIMUM_VALID_FILE_BYTES
            || legacy_bytes > Self::MAXIMUM_FILE_BYTE_COUNT
            || Self::audio_format_label(&Self::read_file_head(&legacy_url, Self::FILE_HEAD_BYTES)) == "unknown"
        {
            let _ = FileManager.remove_item(&legacy_url);
            return None;
        }
        let dest = Self::file_url(track_id, preferred_ext, user_id).ok()?;
        let directory = dest.parent()?.to_path_buf();
        let staging = directory.join(Self::incoming_name());
        let _staging_guard = MusicPlaybackDiskCacheStagingGuard { url: staging.clone() };
        FileManager.move_item(&legacy_url, &staging).ok()?;
        Self::replace_or_move(&staging, &dest).ok()?;
        Self::register_saved_file(track_id, &Self::last_path_component(&dest), user_id);
        Some(dest)
    }

    /// copies bytes the app already owns into the cache so the next play of track_id hits
    /// for device uploads turned into a track and chat attachments played from the local copy
    pub fn adopt_external_file(
        track_id: &str,
        source_url: &Path,
        preferred_extension: Option<&str>,
        user_id: Option<&str>,
    ) -> Option<PathBuf> {
        let key = trimming_whitespaces_and_newlines(track_id);
        if key.is_empty() {
            return None;
        }
        let metadata = std::fs::metadata(source_url).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
        if bytes <= Self::MINIMUM_VALID_FILE_BYTES || bytes > Self::MAXIMUM_FILE_BYTE_COUNT {
            return None;
        }
        let format = Self::audio_format_label(&Self::read_file_head(source_url, Self::FILE_HEAD_BYTES));
        if format == "unknown" {
            return None;
        }
        let source_extension = source_url
            .extension()
            .map(|ext| ext.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fallback_extension = match preferred_extension {
            Some(preferred) if !preferred.is_empty() => preferred.to_owned(),
            _ => source_extension,
        };
        let ext = Self::file_extension(format, &fallback_extension);

        Self::with_file_lock(|state| {
            let directory = Self::base_directory(state, user_id).ok()?;
            if source_url.parent() == Some(directory.as_path()) {
                Self::touch(source_url);
                return Some(source_url.to_path_buf());
            }
            if let Some(existing) = Self::find_cached_url_in(key, &directory) {
                return Some(existing);
            }
            let destination = directory.join(Self::file_name(key, &ext));
            let staging = directory.join(Self::incoming_name());
            let _staging_guard = MusicPlaybackDiskCacheStagingGuard { url: staging.clone() };
            FileManager.copy_item(source_url, &staging).ok()?;
            Self::replace_or_move(&staging, &destination).ok()?;
            if !FileManager.file_exists(&destination) {
                return None;
            }
            Self::touch(&destination);
            let records = Self::cache_file_records(&directory, false);
            let mut index = Self::reconcile_index(Self::read_index(&directory), &directory, Some(&records));
            index.insert(key.to_owned(), Self::last_path_component(&destination));
            let index = Self::prune_cache(&directory, index, Some(&records));
            let _ = Self::write_index(&index, &directory);
            FileManager.file_exists(&destination).then_some(destination)
        })
    }

    fn replace_or_move(staging: &Path, destination: &Path) -> Result<(), CocoaError> {
        if FileManager.file_exists(destination) {
            return std::fs::rename(staging, destination)
                .map_err(|_| CocoaError::new(CocoaErrorCode::FileWriteUnknown));
        }
        FileManager.move_item(staging, destination)
    }

    fn incoming_name() -> String {
        format!("{}{}", Self::INCOMING_PREFIX, Uuid::new_v4().uuid_string().to_lowercase())
    }

    fn purge_legacy_temp_files(track_id: &str) {
        let tmp = temporary_directory();
        let Ok(names) = FileManager.contents_of_directory_at_path(&tmp) else {
            return;
        };
        let prefix = format!("{}{track_id}.", Self::LEGACY_TEMP_PREFIX);
        for name in names.iter().filter(|name| name.starts_with(&prefix)) {
            let _ = FileManager.remove_item(&tmp.join(name));
        }
    }

    fn purge_all_legacy_temp_files() {
        Self::with_file_lock(|state| state.legacy_temp_file_names_by_track_id = Some(HashMap::new()));
        let tmp = temporary_directory();
        let Ok(names) = FileManager.contents_of_directory_at_path(&tmp) else {
            return;
        };
        for name in names.iter().filter(|name| name.starts_with(Self::LEGACY_TEMP_PREFIX)) {
            let _ = FileManager.remove_item(&tmp.join(name));
        }
    }

    fn with_file_lock<T>(body: impl FnOnce(&mut MusicPlaybackDiskCacheLockedState) -> T) -> T {
        let mut state = FILE_LOCK.lock();
        body(&mut state)
    }

    pub fn list_entries(user_id: Option<&str>) -> Vec<MusicPlaybackDiskCacheListedEntry> {
        Self::with_file_lock(|state| {
            let Ok(directory) = Self::base_directory(state, user_id) else {
                return Vec::new();
            };
            let index = Self::read_index(&directory);
            let track_id_by_file: HashMap<String, String> = index
                .into_iter()
                .map(|(track_id, file_name)| (file_name, track_id))
                .collect();
            let Ok(names) = FileManager.contents_of_directory_at_path(&directory) else {
                return Vec::new();
            };
            let mut entries: Vec<MusicPlaybackDiskCacheListedEntry> = names
                .into_iter()
                .filter(|name| name.starts_with(Self::FILE_PREFIX) && !name.ends_with(Self::INDEX_FILE_NAME))
                .map(|name| {
                    let url = directory.join(&name);
                    let attributes = FileManager.attributes_of_item(&url).ok();
                    let bytes = attributes.as_ref().map_or(0, |attributes| attributes.size);
                    let modified_at = attributes
                        .and_then(|attributes| attributes.modification_date)
                        .map_or_else(Date::distant_past, Self::date_from_system_time);
                    let track_id = track_id_by_file
                        .get(&name)
                        .cloned()
                        .or_else(|| Self::parse_track_id(&name))
                        .unwrap_or_else(|| name.clone());
                    let head = Self::read_file_head(&url, Self::FILE_HEAD_BYTES);
                    let format = Self::audio_format_label(&head);
                    MusicPlaybackDiskCacheListedEntry {
                        id: track_id.clone(),
                        track_id,
                        file_name: name,
                        file_bytes: bytes,
                        format_label: format.to_owned(),
                        modified_at,
                    }
                })
                .collect();
            entries.sort_by(|lhs, rhs| rhs.modified_at.cmp(&lhs.modified_at));
            entries
        })
    }

    fn parse_track_id(file_name: &str) -> Option<String> {
        let rest = file_name.strip_prefix(Self::FILE_PREFIX)?;
        match rest.rfind('.') {
            Some(dot) => Some(rest[..dot].to_owned()),
            None => Some(rest.to_owned()),
        }
    }

    fn read_file_head(url: &Path, count: usize) -> Vec<u8> {
        let Ok(file) = std::fs::File::open(url) else {
            return Vec::new();
        };
        let mut data = Vec::with_capacity(count);
        let limit = u64::try_from(count).unwrap_or(u64::MAX);
        if file.take(limit).read_to_end(&mut data).is_err() {
            return Vec::new();
        }
        data
    }

    fn audio_format_label(head: &[u8]) -> &'static str {
        if head.len() >= 3 && head.starts_with(&[0x49, 0x44, 0x33]) {
            return "mp3-id3";
        }
        if head.len() >= 2 && head[0] == 0xFF && (head[1] & 0xE0) == 0xE0 {
            return "mp3-frame";
        }
        if head.len() >= 8 && head[4..8] == [0x66, 0x74, 0x79, 0x70] {
            return "m4a/mp4";
        }
        if head.starts_with(&[0x66, 0x4C, 0x61, 0x43]) {
            return "flac";
        }
        if head.starts_with(&[0x4F, 0x67, 0x67, 0x53]) {
            return "ogg";
        }
        if head.starts_with(&[0x52, 0x49, 0x46, 0x46]) {
            return "wav";
        }
        "unknown"
    }

    fn last_path_component(url: &Path) -> String {
        url.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn is_bare_file_name(name: &str) -> bool {
        Path::new(name).file_name().and_then(|component| component.to_str()) == Some(name)
    }

    fn date_from_system_time(time: SystemTime) -> Date {
        let seconds = time.duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
        Date::from_time_interval_since_1970(seconds)
    }

    fn system_time_from_date(date: Date) -> SystemTime {
        let ms = u64::try_from(date.ms()).unwrap_or(0);
        UNIX_EPOCH + Duration::from_millis(ms)
    }
}

/// guard that removes the staging file on drop
struct MusicPlaybackDiskCacheStagingGuard {
    url: PathBuf,
}

impl Drop for MusicPlaybackDiskCacheStagingGuard {
    fn drop(&mut self) {
        let _ = FileManager.remove_item(&self.url);
    }
}
