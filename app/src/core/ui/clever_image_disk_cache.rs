use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, UNIX_EPOCH};

use futures::FutureExt;
use futures::channel::{mpsc, oneshot};
use futures::future::{AbortHandle, Abortable, BoxFuture, Shared};
use futures::StreamExt;
use gpui::{App, AsyncApp, Global, RenderImage, Subscription, Task};
use image::codecs::gif::GifDecoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, Delay, DynamicImage, Frame, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, RgbaImage};
use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::URLRequest;
use rise_platform::AppPaths;
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileAttributeType, FileManager};
use rise_platform::power::{POWER_STATE_DID_CHANGE, system_low_power_mode};
use rise_platform::process_info::THERMAL_STATE_DID_CHANGE;
use serde_json::{Map, Value};
use smallvec::SmallVec;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication,
};
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai_auth_hooks::{sai_auth_access_token, sai_auth_hooks, sai_auth_is_expired};
use crate::core::maintenance::maintenance_gate::RiseServerClock;
use crate::modules::auth::services::access_token_user_id::AccessTokenUserId;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDownloadProgress, RemoteMediaDownloadRetryPolicy,
};
use crate::core::media::rise_media_foreground_gate::{RISE_MEDIA_FOREGROUND_DID_CHANGE, RiseMediaForegroundGate};
use crate::core::media::rise_media_prefetch_limits::RiseImagePrefetchLimits;
use crate::core::media::rise_media_transfer_hub::{RiseMediaCacheIdentity, RiseMediaTransferHub};
use crate::core::media::rise_progressive_data_fetcher::{
    RiseProgressiveDataFetcher, RiseProgressiveDataFetcherLane, RiseProgressiveDataFetcherOnProgress,
};
use crate::core::media::url_cache::{CachedURLResponse, URLCache};
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::ui::animated_media_playback_coordinator::AnimatedMediaThermalLevel;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::lottie_sticker::lottie_sticker_prefetch::LottieStickerPrefetch;

pub type CleverImage = Arc<RenderImage>;

const URL_SESSION_TASK_LOW_PRIORITY: f32 = 0.25;

const JPEG_COMPRESSION_QUALITY: u8 = 92;

pub enum CleverImageDiskCache {}

struct CleverImageDiskCacheTokens {
    invalidation_tokens: HashMap<String, u64>,
}

static FILE_LOCK: Mutex<()> = Mutex::new(());
static INVALIDATION_TOKENS: LazyLock<Mutex<CleverImageDiskCacheTokens>> = LazyLock::new(|| {
    Mutex::new(CleverImageDiskCacheTokens {
        invalidation_tokens: HashMap::new(),
    })
});
static ACCOUNT_DIRECTORY_NAMES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static CACHE_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::compiled().canon())
        .ok()
        .map(|paths| paths.cache_dir().to_path_buf())
});

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleverImageDiskCacheListedEntry {
    pub id: String,
    pub cache_key: String,
    pub digest: String,
    pub file_bytes: i64,
    pub modified_at: Date,
}

impl CleverImageDiskCache {
    const SUBFOLDER: &'static str = "CleverImageCache";
    const INDEX_FILE_NAME: &'static str = "_url_index.json";
    const ACCOUNTS_FOLDER: &'static str = "accounts";
    const ANONYMOUS_ACCOUNT: &'static str = "anonymous";
    const BIN_EXTENSION: &'static str = "bin";
    const MAXIMUM_CACHED_ACCOUNT_DIRECTORY_NAMES: usize = 16;

    fn maximum_entry_count() -> usize {
        usize::try_from(RiseLimits::cache_budget("image_disk_entries")).unwrap_or(0)
    }

    fn maximum_total_bytes() -> i64 {
        RiseLimits::cache_budget("image_disk_total_bytes")
    }

    fn maximum_file_bytes() -> i64 {
        RiseLimits::cache_budget("image_disk_file_bytes")
    }

    pub fn normalized_cache_key(url_string: &str) -> String {
        trimming_whitespaces_and_newlines(url_string).to_owned()
    }

    fn root_directory() -> Option<PathBuf> {
        let dir = CACHE_ROOT.as_ref()?.join(Self::SUBFOLDER);
        FileManager.create_directory(&dir, true).ok()?;
        Self::configure_cache_item(&dir);
        Some(dir)
    }

    fn base_directory(user_id: Option<&str>) -> Option<PathBuf> {
        let root = Self::root_directory()?;
        let accounts = root.join(Self::ACCOUNTS_FOLDER);
        FileManager.create_directory(&accounts, true).ok()?;
        Self::configure_cache_item(&accounts);
        let dir = accounts.join(Self::account_directory_name(user_id));
        FileManager.create_directory(&dir, true).ok()?;
        Self::configure_cache_item(&dir);
        Some(dir)
    }

    fn account_directory_name(user_id: Option<&str>) -> String {
        let Some(user_id) = user_id else {
            return Self::ANONYMOUS_ACCOUNT.to_owned();
        };
        if let Some(cached) = ACCOUNT_DIRECTORY_NAMES.lock().get(user_id) {
            return cached.clone();
        }
        let value = trimming_whitespaces_and_newlines(user_id);
        let name = if value.is_empty() {
            Self::ANONYMOUS_ACCOUNT.to_owned()
        } else {
            sha256_hex(value.as_bytes())
        };
        let mut names = ACCOUNT_DIRECTORY_NAMES.lock();
        if names.len() >= Self::MAXIMUM_CACHED_ACCOUNT_DIRECTORY_NAMES {
            names.clear();
        }
        names.insert(user_id.to_owned(), name.clone());
        name
    }

    pub fn digest_hex(cache_key: &str) -> String {
        sha256_hex(cache_key.as_bytes())
    }

    fn key_index_url(directory: &Path) -> PathBuf {
        directory.join(Self::INDEX_FILE_NAME)
    }

    fn read_key_index(directory: &Path) -> HashMap<String, String> {
        let url = Self::key_index_url(directory);
        if !FileManager.file_exists(&url) {
            return HashMap::new();
        }
        std::fs::read(&url)
            .ok()
            .and_then(|data| serde_json::from_slice::<HashMap<String, String>>(&data).ok())
            .unwrap_or_default()
    }

    fn write_key_index(dict: &HashMap<String, String>, directory: &Path) -> std::io::Result<()> {
        let url = Self::key_index_url(directory);
        let sorted: std::collections::BTreeMap<&String, &String> = dict.iter().collect();
        let data = serde_json::to_vec(&sorted).map_err(std::io::Error::other)?;
        Self::write_atomically(&url, &data)?;
        Self::configure_cache_item(&url);
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

    fn file_url(hex: &str, directory: &Path) -> PathBuf {
        directory.join(format!("{hex}.{}", Self::BIN_EXTENSION))
    }

    /// blocking file io, call it off the main thread
    pub fn image_data(
        cache_key: &str,
        user_id: Option<&str>,
        expected_invalidation_token: Option<u64>,
    ) -> Option<Vec<u8>> {
        let key = Self::normalized_cache_key(cache_key);
        if key.is_empty() {
            return None;
        }
        let hex = Self::digest_hex(&key);
        Self::with_file_lock(|| {
            if !Self::token_matches_locked(expected_invalidation_token, user_id) {
                return None;
            }
            let directory = Self::base_directory(user_id)?;
            let url = Self::file_url(&hex, &directory);
            let data = std::fs::read(&url).ok().filter(|data| !data.is_empty());
            let Some(data) = data else {
                let _ = FileManager.remove_item(&url);
                let mut index = Self::read_key_index(&directory);
                if index.remove(&hex).is_some() {
                    let _ = Self::write_key_index(&index, &directory);
                }
                return None;
            };
            Self::configure_cache_item(&url);
            Self::touch_modification_date(&url);
            let mut index = Self::read_key_index(&directory);
            if index.get(&hex) != Some(&key) {
                index.insert(hex.clone(), key.clone());
                let _ = Self::write_key_index(&index, &directory);
            }
            Some(data)
        })
    }

    pub fn contains(cache_key: &str, user_id: Option<&str>, expected_invalidation_token: Option<u64>) -> bool {
        let key = Self::normalized_cache_key(cache_key);
        if key.is_empty() {
            return false;
        }
        let hex = Self::digest_hex(&key);
        Self::with_file_lock(|| {
            if !Self::token_matches_locked(expected_invalidation_token, user_id) {
                return false;
            }
            let Some(directory) = Self::base_directory(user_id) else {
                return false;
            };
            let bytes = Self::file_size(&Self::file_url(&hex, &directory));
            bytes > 0 && bytes <= Self::maximum_file_bytes()
        })
    }

    /// cheap synchronous read of a cached image
    /// the render loader below decodes cached bytes on a worker instead
    #[track_caller]
    pub fn ui_image(cache_key: &str, user_id: Option<&str>) -> Option<CleverImage> {
        let data = Self::image_data(cache_key, user_id, None)?;
        CleverImageBitmapDecoder::decode(&data, None)
    }

    pub fn store_data(
        data: &[u8],
        cache_key: &str,
        user_id: Option<&str>,
        expected_invalidation_token: Option<u64>,
    ) {
        let key = Self::normalized_cache_key(cache_key);
        if key.is_empty() || data.is_empty() || i64::try_from(data.len()).unwrap_or(i64::MAX) > Self::maximum_file_bytes() {
            return;
        }
        let hex = Self::digest_hex(&key);
        Self::with_file_lock(|| {
            if !Self::token_matches_locked(expected_invalidation_token, user_id) {
                return;
            }
            let Some(directory) = Self::base_directory(user_id) else {
                return;
            };
            let url = Self::file_url(&hex, &directory);
            let mut index = Self::reconciled_key_index(Self::read_key_index(&directory), &directory);
            if Self::write_atomically(&url, data).is_err() {
                return;
            }
            Self::configure_cache_item(&url);
            index.insert(hex, key);
            let index = Self::trimmed_key_index(index, &directory);
            let _ = Self::write_key_index(&index, &directory);
        });
    }

    pub fn store_image(
        image: &RenderImage,
        cache_key: &str,
        user_id: Option<&str>,
        expected_invalidation_token: Option<u64>,
    ) {
        if let Some(data) = CleverImageBitmapEncoder::jpeg_data(image, JPEG_COMPRESSION_QUALITY) {
            Self::store_data(&data, cache_key, user_id, expected_invalidation_token);
        } else if let Some(data) = CleverImageBitmapEncoder::png_data(image) {
            Self::store_data(&data, cache_key, user_id, expected_invalidation_token);
        }
    }

    pub fn remove_cache_key(cache_key: &str, user_id: Option<&str>, if_matching: Option<&[u8]>) {
        let key = Self::normalized_cache_key(cache_key);
        if key.is_empty() {
            return;
        }
        Self::remove_digest_hex(&Self::digest_hex(&key), user_id, if_matching);
    }

    pub fn remove_digest_hex(hex: &str, user_id: Option<&str>, if_matching: Option<&[u8]>) {
        if hex.is_empty() {
            return;
        }
        let url_key = Self::with_file_lock(|| {
            let directory = Self::base_directory(user_id)?;
            let url = Self::file_url(hex, &directory);
            if let Some(data) = if_matching
                && std::fs::read(&url).ok().as_deref() != Some(data)
            {
                return None;
            }
            let mut index = Self::read_key_index(&directory);
            let url_key = index.remove(hex);
            if url_key.is_some() {
                let _ = Self::write_key_index(&index, &directory);
            }
            let _ = FileManager.remove_item(&url);
            url_key
        });
        if let Some(url_key) = url_key {
            MediaViewerImageMemoryCache::remove(&url_key, user_id);
        }
    }

    pub fn clear_all(user_id: Option<&str>) {
        let keys = Self::with_file_lock(|| {
            Self::invalidate_locked(user_id);
            let Some(directory) = Self::base_directory(user_id) else {
                return Vec::new();
            };
            let keys: Vec<String> = Self::read_key_index(&directory).into_values().collect();
            if let Ok(items) = FileManager.contents_of_directory_at(&directory, DirectoryEnumerationOptions::default()) {
                for url in items {
                    let _ = FileManager.remove_item(&url);
                }
            }
            let _ = FileManager.create_directory(&directory, true);
            Self::configure_cache_item(&directory);
            keys
        });
        for key in keys {
            MediaViewerImageMemoryCache::remove(&key, user_id);
        }
    }

    pub fn remove_persisted_account(user_id: &str) {
        Self::with_file_lock(|| {
            Self::invalidate_locked(Some(user_id));
            let Some(root) = Self::root_directory() else {
                return;
            };
            let dir = root
                .join(Self::ACCOUNTS_FOLDER)
                .join(Self::account_directory_name(Some(user_id)));
            let _ = FileManager.remove_item(&dir);
        });
        MediaViewerImageMemoryCache::clear_all();
    }

    pub fn folder_path_for_inspector(user_id: Option<&str>) -> String {
        Self::with_file_lock(|| {
            Self::base_directory(user_id)
                .map(|directory| directory.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(unavailable)".to_owned())
        })
    }

    pub fn storage_directory_url(user_id: Option<&str>) -> Option<PathBuf> {
        Self::with_file_lock(|| Self::base_directory(user_id))
    }

    /// manifest as it stands, without the reconcile and trim list_entries does
    /// a screen that lists the cache must not evict while it looks
    /// the trim deletes to budget and rewrites the index so opening a gallery would shrink the cache
    pub fn list_entries_read_only(user_id: Option<&str>) -> Vec<CleverImageDiskCacheListedEntry> {
        Self::with_file_lock(|| {
            let Some(directory) = Self::base_directory(user_id) else {
                return Vec::new();
            };
            let index = Self::read_key_index(&directory);
            let mut entries: Vec<CleverImageDiskCacheListedEntry> = index
                .into_iter()
                .filter_map(|(hex, cache_key)| {
                    let url = Self::file_url(&hex, &directory);
                    let attributes = FileManager.attributes_of_item(&url).ok()?;
                    if attributes.r#type != FileAttributeType::TypeRegular || attributes.size <= 0 {
                        return None;
                    }
                    Some(CleverImageDiskCacheListedEntry {
                        id: hex.clone(),
                        cache_key,
                        digest: hex,
                        file_bytes: attributes.size,
                        modified_at: attributes
                            .modification_date
                            .map_or_else(Date::distant_past, Self::date_from_system_time),
                    })
                })
                .collect();
            entries.sort_by(|lhs, rhs| rhs.modified_at.cmp(&lhs.modified_at));
            entries
        })
    }

    pub fn list_entries(user_id: Option<&str>) -> Vec<CleverImageDiskCacheListedEntry> {
        Self::with_file_lock(|| {
            let Some(directory) = Self::base_directory(user_id) else {
                return Vec::new();
            };
            let index = Self::reconciled_key_index(Self::read_key_index(&directory), &directory);
            let index = Self::trimmed_key_index(index, &directory);
            let _ = Self::write_key_index(&index, &directory);
            let mut entries: Vec<CleverImageDiskCacheListedEntry> = index
                .into_iter()
                .map(|(hex, cache_key)| {
                    let url = Self::file_url(&hex, &directory);
                    let attributes = FileManager.attributes_of_item(&url).ok();
                    CleverImageDiskCacheListedEntry {
                        id: hex.clone(),
                        cache_key,
                        digest: hex,
                        file_bytes: attributes.as_ref().map_or(0, |attributes| attributes.size),
                        modified_at: attributes
                            .and_then(|attributes| attributes.modification_date)
                            .map_or_else(Date::distant_past, Self::date_from_system_time),
                    }
                })
                .collect();
            entries.sort_by(|lhs, rhs| rhs.modified_at.cmp(&lhs.modified_at));
            entries
        })
    }

    fn with_file_lock<T>(body: impl FnOnce() -> T) -> T {
        let _guard = FILE_LOCK.lock();
        body()
    }

    pub fn invalidation_token(user_id: Option<&str>) -> u64 {
        let account = Self::account_directory_name(user_id);
        INVALIDATION_TOKENS
            .lock()
            .invalidation_tokens
            .get(&account)
            .copied()
            .unwrap_or(0)
    }

    pub fn invalidate_pending_operations(user_id: Option<&str>) {
        Self::with_file_lock(|| Self::invalidate_locked(user_id));
    }

    pub fn perform_if_token_is_current(
        user_id: Option<&str>,
        expected_invalidation_token: u64,
        operation: impl FnOnce(),
    ) {
        Self::with_file_lock(|| {
            if !Self::token_matches_locked(Some(expected_invalidation_token), user_id) {
                return;
            }
            operation();
        });
    }

    fn token_matches_locked(expected: Option<u64>, user_id: Option<&str>) -> bool {
        let Some(expected) = expected else {
            return true;
        };
        Self::invalidation_token(user_id) == expected
    }

    fn invalidate_locked(user_id: Option<&str>) {
        let account = Self::account_directory_name(user_id);
        let mut tokens = INVALIDATION_TOKENS.lock();
        let token = tokens.invalidation_tokens.entry(account).or_insert(0);
        *token = token.wrapping_add(1);
    }

    fn reconciled_key_index(raw_index: HashMap<String, String>, directory: &Path) -> HashMap<String, String> {
        let mut index = raw_index;
        let urls = FileManager
            .contents_of_directory_at(directory, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
            .unwrap_or_default();
        let files_by_digest: HashMap<String, PathBuf> = urls
            .into_iter()
            .filter_map(|url| {
                if url.extension().and_then(|extension| extension.to_str()) != Some(Self::BIN_EXTENSION) {
                    return None;
                }
                let digest = url.file_stem()?.to_str()?.to_owned();
                Some((digest, url))
            })
            .collect();

        let hexes: Vec<String> = index.keys().cloned().collect();
        for hex in hexes {
            match files_by_digest.get(&hex) {
                Some(url) if Self::file_size(url) > 0 => Self::configure_cache_item(url),
                found => {
                    index.remove(&hex);
                    if let Some(url) = found {
                        let _ = FileManager.remove_item(url);
                    }
                }
            }
        }
        for (hex, url) in &files_by_digest {
            if !index.contains_key(hex) {
                let _ = FileManager.remove_item(url);
            }
        }
        index
    }

    fn trimmed_key_index(raw_index: HashMap<String, String>, directory: &Path) -> HashMap<String, String> {
        let mut index = raw_index;
        let mut entries: Vec<(String, PathBuf, i64, Date)> = index
            .keys()
            .filter_map(|hex| {
                let url = Self::file_url(hex, directory);
                let attributes = FileManager.attributes_of_item(&url).ok();
                let bytes = attributes.as_ref().map_or(0, |attributes| attributes.size);
                if bytes <= 0 {
                    return None;
                }
                let modified = attributes
                    .and_then(|attributes| attributes.modification_date)
                    .map_or_else(Date::distant_past, Self::date_from_system_time);
                Some((hex.clone(), url, bytes, modified))
            })
            .collect();
        let mut total_bytes: i64 = entries.iter().map(|entry| entry.2).sum();
        entries.sort_by(|lhs, rhs| lhs.3.cmp(&rhs.3));
        let maximum_entry_count = Self::maximum_entry_count();
        let maximum_total_bytes = Self::maximum_total_bytes();
        let mut entries = entries.into_iter();
        let mut remaining = entries.len();
        while remaining > maximum_entry_count || total_bytes > maximum_total_bytes {
            let Some(entry) = entries.next() else {
                break;
            };
            remaining -= 1;
            let _ = FileManager.remove_item(&entry.1);
            index.remove(&entry.0);
            total_bytes = (total_bytes - entry.2).max(0);
        }
        index
    }

    fn file_size(url: &Path) -> i64 {
        FileManager.attributes_of_item(url).map_or(0, |attributes| attributes.size)
    }

    fn configure_cache_item(_url: &Path) {}

    fn touch_modification_date(url: &Path) {
        let now = RiseClock::now();
        let seconds = now.time_interval_since_1970().max(0.0);
        let modified = UNIX_EPOCH + Duration::from_secs_f64(seconds);
        if let Ok(file) = std::fs::File::options().write(true).open(url) {
            let _ = file.set_modified(modified);
        }
    }

    fn date_from_system_time(time: std::time::SystemTime) -> Date {
        let seconds = time.duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64());
        Date::from_time_interval_since_1970(seconds)
    }
}

pub enum CleverImageBitmapDecoder {}

impl CleverImageBitmapDecoder {
    const MAIN_THREAD_DECODE_WARN_MS: f64 = 4.0;

    #[track_caller]
    pub fn decode(data: &[u8], maximum_pixel_dimension: Option<i64>) -> Option<CleverImage> {
        if data.is_empty() {
            return None;
        }
        let started = RiseClock::monotonic();
        let image = Self::decode_frame_at(data, 0, maximum_pixel_dimension);
        Self::report_main_thread_decode(started, data.len(), std::panic::Location::caller());
        image
    }

    fn report_main_thread_decode(
        started: Duration,
        bytes: usize,
        caller: &'static std::panic::Location<'static>,
    ) {
        let elapsed_ms = RiseClock::monotonic().saturating_sub(started).as_secs_f64() * 1000.0;
        if elapsed_ms >= Self::MAIN_THREAD_DECODE_WARN_MS && std::thread::current().name() == Some("main") {
            tracing::warn!(
                target: "riseonly::X2108",
                "main thread image decode ms={elapsed_ms:.1} bytes={bytes} caller={caller}"
            );
        }
    }

    /// decodes one frame of an animated image into a bounded bitmap
    /// safe on a worker, never hands a native resolution lazy frame to the main thread
    pub fn decode_frame(data: &[u8], index: usize, maximum_pixel_dimension: i64) -> Option<CleverImage> {
        if data.is_empty() {
            return None;
        }
        Self::decode_frame_at(data, index, Some(maximum_pixel_dimension.max(1)))
    }

    fn decode_frame_at(data: &[u8], index: usize, maximum_pixel_dimension: Option<i64>) -> Option<CleverImage> {
        let reader = ImageReader::new(Cursor::new(data)).with_guessed_format().ok()?;
        let Some(format) = reader.format() else {
            if index != 0 {
                return None;
            }
            let maximum = maximum_pixel_dimension.map(|value| u32::try_from(value.max(1)).unwrap_or(u32::MAX));
            let decoded = rise_platform::photo_library::decode_system_image_data(data, maximum)?;
            return Some(Self::render_bgra(RgbaImage::from_raw(decoded.width, decoded.height, decoded.bgra)?));
        };
        let image = match format {
            ImageFormat::Gif => Self::animation_frame(GifDecoder::new(Cursor::new(data)).ok()?, index)?,
            ImageFormat::WebP => {
                let decoder = WebPDecoder::new(Cursor::new(data)).ok()?;
                if decoder.has_animation() {
                    Self::animation_frame(decoder, index)?
                } else if index == 0 {
                    Self::still_image(ImageReader::new(Cursor::new(data)).with_guessed_format().ok()?)?
                } else {
                    return None;
                }
            }
            _ if index == 0 => Self::still_image(reader)?,
            _ => return None,
        };

        let source_maximum = i64::from(image.width().max(image.height()));
        let requested_maximum = maximum_pixel_dimension.map(|value| value.max(1));
        let target_maximum = match requested_maximum {
            Some(requested) if source_maximum > 0 => requested.min(source_maximum),
            Some(requested) => requested,
            None => source_maximum.max(1),
        };
        let image = if source_maximum > target_maximum {
            let target = u32::try_from(target_maximum).unwrap_or(u32::MAX);
            image.thumbnail(target, target)
        } else {
            image
        };
        Some(Self::render_image(image.into_rgba8()))
    }

    fn still_image(reader: ImageReader<Cursor<&[u8]>>) -> Option<DynamicImage> {
        let mut decoder = reader.into_decoder().ok()?;
        let orientation = decoder.orientation().ok();
        let mut image = DynamicImage::from_decoder(decoder).ok()?;
        if let Some(orientation) = orientation {
            image.apply_orientation(orientation);
        }
        Some(image)
    }

    fn animation_frame<'a>(decoder: impl AnimationDecoder<'a>, index: usize) -> Option<DynamicImage> {
        let frame = decoder.into_frames().nth(index)?.ok()?;
        Some(DynamicImage::ImageRgba8(frame.into_buffer()))
    }

    /// wraps an rgba bitmap as the bgra render image gpui draws
    pub fn render_image(mut rgba: RgbaImage) -> CleverImage {
        for pixel in rgba.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Self::render_bgra(rgba)
    }

    /// wraps a bitmap already in bgra order as a render image
    fn render_bgra(bgra: RgbaImage) -> CleverImage {
        let frame = Frame::from_parts(bgra, 0, 0, Delay::from_numer_denom_ms(0, 1));
        let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
        rise_ui::image_ui::render_image_atlas::RenderImageAtlas::tracked(Arc::new(RenderImage::new(frames)))
    }
}

pub enum CleverImageBitmapEncoder {}

impl CleverImageBitmapEncoder {
    /// converts the bgra frame gpui holds back to an rgba bitmap
    pub fn rgba(image: &RenderImage) -> Option<RgbaImage> {
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        let mut bytes = image.as_bytes(0)?.to_vec();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        RgbaImage::from_raw(width, height, bytes)
    }

    pub fn jpeg_data(image: &RenderImage, quality: u8) -> Option<Vec<u8>> {
        let rgb = DynamicImage::ImageRgba8(Self::rgba(image)?).into_rgb8();
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, quality).encode_image(&rgb).ok()?;
        Some(data)
    }

    fn png_data(image: &RenderImage) -> Option<Vec<u8>> {
        let rgba = Self::rgba(image)?;
        let mut data = Vec::new();
        PngEncoder::new(&mut data)
            .write_image(rgba.as_raw(), rgba.width(), rgba.height(), image::ExtendedColorType::Rgba8)
            .ok()?;
        Some(data)
    }

    fn cost(image: &RenderImage) -> usize {
        let size = image.size(0);
        let width = usize::try_from(size.width.0).unwrap_or(0);
        let height = usize::try_from(size.height.0).unwrap_or(0);
        width.saturating_mul(4).saturating_mul(height)
    }
}

pub enum CleverImageRenderMemoryCache {}

static RENDER_CACHE: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(usize::try_from(RiseLimits::client_value("image_render_cache_entries")).unwrap_or(0));
    cache.set_total_cost_limit(usize::try_from(RiseLimits::client_value("image_render_cache_bytes")).unwrap_or(0));
    cache
});
static LATEST_BY_SOURCE: LazyLock<Mutex<HashMap<String, std::sync::Weak<RenderImage>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl CleverImageRenderMemoryCache {
    const LATEST_BY_SOURCE_LIMIT: usize = 640;

    pub fn image(
        cache_key: &str,
        maximum_pixel_dimension: i64,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Option<CleverImage> {
        RENDER_CACHE.object(&Self::key(cache_key, maximum_pixel_dimension, account_user_id, account_epoch))
    }

    pub fn latest_image(cache_key: &str, account_user_id: Option<&str>, account_epoch: Option<&str>) -> Option<CleverImage> {
        let key = Self::latest_key(cache_key, account_user_id, account_epoch);
        let mut latest = LATEST_BY_SOURCE.lock();
        let image = latest.get(&key)?.upgrade();
        if image.is_none() {
            latest.remove(&key);
        }
        image
    }

    pub fn store(
        image: CleverImage,
        cache_key: &str,
        maximum_pixel_dimension: i64,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) {
        let cost = CleverImageBitmapEncoder::cost(&image);
        let weak = Arc::downgrade(&image);
        RENDER_CACHE.set_object_cost(
            image,
            Self::key(cache_key, maximum_pixel_dimension, account_user_id, account_epoch),
            cost,
        );
        let source_key = Self::latest_key(cache_key, account_user_id, account_epoch);
        let mut latest = LATEST_BY_SOURCE.lock();
        latest.insert(source_key, weak);
        if latest.len() > Self::LATEST_BY_SOURCE_LIMIT {
            latest.retain(|_, image| image.strong_count() > 0);
            let overflow = latest.len().saturating_sub(Self::LATEST_BY_SOURCE_LIMIT);
            let dropped: Vec<String> = latest.keys().take(overflow).cloned().collect();
            for key in dropped {
                latest.remove(&key);
            }
        }
    }

    pub fn clear_all() {
        RENDER_CACHE.remove_all_objects();
        LATEST_BY_SOURCE.lock().clear();
    }

    fn key(
        cache_key: &str,
        maximum_pixel_dimension: i64,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> String {
        format!(
            "{}|{}|{}|{}",
            account_user_id.unwrap_or("anonymous"),
            account_epoch.unwrap_or("none"),
            maximum_pixel_dimension,
            cache_key
        )
    }

    fn latest_key(cache_key: &str, account_user_id: Option<&str>, account_epoch: Option<&str>) -> String {
        format!(
            "{}|{}|{}",
            account_user_id.unwrap_or("anonymous"),
            account_epoch.unwrap_or("none"),
            cache_key
        )
    }
}

/// one url plus all that makes warming it a hit and not a second load
/// memory key is url plus account plus pixel bucket, disk access comes from the cache policy
/// warming a bare list of urls is wrong, a wrong bucket means a second decode
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CleverImagePrefetchItem {
    pub url_string: String,
    pub maximum_pixel_dimension: Option<i64>,
    pub persist_to_disk: bool,
    pub cache_policy: CleverImageCachePolicy,
}

impl CleverImagePrefetchItem {
    pub fn new(url_string: impl Into<String>) -> Self {
        Self {
            url_string: url_string.into(),
            maximum_pixel_dimension: None,
            persist_to_disk: true,
            cache_policy: CleverImageCachePolicy::Standard,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CleverImagePrefetchQueueRequest {
    pub cache_key: String,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
    pub maximum_pixel_dimension: Option<i64>,
    pub persist_to_disk: bool,
    pub cache_policy: CleverImageCachePolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CleverImagePrefetchQueueOwner {
    Persistent,
    Directional { scope: String, generation: u64 },
}

pub type CleverImagePrefetchQueueOperation =
    Arc<dyn Fn(CleverImagePrefetchQueueRequest, AsyncApp) -> futures::future::LocalBoxFuture<'static, ()> + Send + Sync>;

struct CleverImagePrefetchQueuePendingJob {
    owners: HashSet<CleverImagePrefetchQueueOwner>,
}

struct CleverImagePrefetchQueueActiveJob {
    owners: HashSet<CleverImagePrefetchQueueOwner>,
    task_id: u64,
    task: AbortHandle,
    cancellation_requested: bool,
}

struct CleverImagePrefetchQueueState {
    pending_order: Vec<CleverImagePrefetchQueueRequest>,
    pending_head: usize,
    pending_by_request: HashMap<CleverImagePrefetchQueueRequest, CleverImagePrefetchQueuePendingJob>,
    active_by_request: HashMap<CleverImagePrefetchQueueRequest, CleverImagePrefetchQueueActiveJob>,
    successor_by_request: HashMap<CleverImagePrefetchQueueRequest, CleverImagePrefetchQueuePendingJob>,
    latest_generation_by_scope: HashMap<String, u64>,
    next_task_id: u64,
    environment_generation: u64,
    environment_snapshot: Option<MediaPrefetchEnvironmentSnapshot>,
}

pub struct CleverImagePrefetchQueue {
    maximum_concurrent: usize,
    maximum_pending: usize,
    operation: CleverImagePrefetchQueueOperation,
    state: RiseIsolated<CleverImagePrefetchQueueState>,
}

impl CleverImagePrefetchQueue {
    pub fn new(maximum_concurrent: i64, maximum_pending: i64, operation: CleverImagePrefetchQueueOperation) -> Arc<Self> {
        Arc::new(Self {
            maximum_concurrent: usize::try_from(maximum_concurrent.max(1)).unwrap_or(1),
            maximum_pending: usize::try_from(maximum_pending.max(1)).unwrap_or(1),
            operation,
            state: RiseIsolated::new(CleverImagePrefetchQueueState {
                pending_order: Vec::new(),
                pending_head: 0,
                pending_by_request: HashMap::new(),
                active_by_request: HashMap::new(),
                successor_by_request: HashMap::new(),
                latest_generation_by_scope: HashMap::new(),
                next_task_id: 0,
                environment_generation: 0,
                environment_snapshot: None,
            }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn enqueue(
        self: &Arc<Self>,
        url_strings: &[String],
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        maximum_pixel_dimension: Option<i64>,
        persist_to_disk: bool,
        cache_policy: CleverImageCachePolicy,
        environment: Option<MediaPrefetchEnvironmentSnapshot>,
        cx: &AsyncApp,
    ) {
        if let Some(environment) = environment
            && !self.update_environment(environment).await
        {
            return;
        }
        let items = Self::items(url_strings, maximum_pixel_dimension, persist_to_disk, cache_policy);
        let requests = Self::requests(&items, account_user_id.as_deref(), account_epoch.as_deref());
        let starts = self.state.with(|state| {
            for request in requests {
                self.add(state, request, CleverImagePrefetchQueueOwner::Persistent);
            }
            self.pump(state)
        });
        self.start(starts, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace_directional_urls(
        self: &Arc<Self>,
        url_strings: &[String],
        scope: &str,
        generation: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        maximum_pixel_dimension: Option<i64>,
        persist_to_disk: bool,
        cache_policy: CleverImageCachePolicy,
        cx: &AsyncApp,
    ) {
        let items = Self::items(url_strings, maximum_pixel_dimension, persist_to_disk, cache_policy);
        self.replace_directional(&items, scope, generation, account_user_id, account_epoch, None, cx)
            .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace_directional(
        self: &Arc<Self>,
        items: &[CleverImagePrefetchItem],
        scope: &str,
        generation: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        environment: Option<MediaPrefetchEnvironmentSnapshot>,
        cx: &AsyncApp,
    ) {
        if let Some(environment) = environment
            && !self.update_environment(environment).await
        {
            return;
        }
        let requests = Self::requests(items, account_user_id.as_deref(), account_epoch.as_deref());
        let starts = self.state.with(|state| {
            if generation < state.latest_generation_by_scope.get(scope).copied().unwrap_or(0) {
                return Vec::new();
            }
            state.latest_generation_by_scope.insert(scope.to_owned(), generation);
            Self::remove_directional_owners(state, scope);
            let owner = CleverImagePrefetchQueueOwner::Directional {
                scope: scope.to_owned(),
                generation,
            };
            for request in requests {
                self.add(state, request, owner.clone());
            }
            self.pump(state)
        });
        self.start(starts, cx);
    }

    pub async fn cancel_directional(self: &Arc<Self>, scope: &str, generation: u64, cx: &AsyncApp) {
        let starts = self.state.with(|state| {
            if generation < state.latest_generation_by_scope.get(scope).copied().unwrap_or(0) {
                return Vec::new();
            }
            state.latest_generation_by_scope.insert(scope.to_owned(), generation);
            Self::remove_directional_owners(state, scope);
            self.pump(state)
        });
        self.start(starts, cx);
    }

    pub async fn update_environment(&self, snapshot: MediaPrefetchEnvironmentSnapshot) -> bool {
        self.state.with(|state| {
            if snapshot.generation < state.environment_generation {
                return false;
            }
            if snapshot.generation > state.environment_generation {
                state.environment_generation = snapshot.generation;
                Self::reset_locked(state, true);
            }
            state.environment_snapshot = Some(snapshot);
            snapshot.is_current()
        })
    }

    pub async fn reset(&self, preserving_directional_generations: bool) {
        self.state
            .with(|state| Self::reset_locked(state, preserving_directional_generations));
    }

    fn reset_locked(state: &mut CleverImagePrefetchQueueState, preserving_directional_generations: bool) {
        state.pending_order.clear();
        state.pending_head = 0;
        state.pending_by_request.clear();
        state.successor_by_request.clear();
        if !preserving_directional_generations {
            state.latest_generation_by_scope.clear();
        }
        for active in state.active_by_request.values_mut() {
            active.owners.clear();
            active.cancellation_requested = true;
            active.task.abort();
        }
    }

    fn items(
        url_strings: &[String],
        maximum_pixel_dimension: Option<i64>,
        persist_to_disk: bool,
        cache_policy: CleverImageCachePolicy,
    ) -> Vec<CleverImagePrefetchItem> {
        url_strings
            .iter()
            .map(|url_string| CleverImagePrefetchItem {
                url_string: url_string.clone(),
                maximum_pixel_dimension,
                persist_to_disk,
                cache_policy,
            })
            .collect()
    }

    fn requests(
        items: &[CleverImagePrefetchItem],
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Vec<CleverImagePrefetchQueueRequest> {
        let mut seen = HashSet::new();
        let mut result = Vec::with_capacity(items.len());
        for item in items {
            let cache_key = CleverImageDiskCache::normalized_cache_key(&item.url_string);
            if !cache_key.to_lowercase().starts_with("http") || Url::from_string(&cache_key).is_none() {
                continue;
            }
            let request = CleverImagePrefetchQueueRequest {
                cache_key,
                account_user_id: account_user_id.map(str::to_owned),
                account_epoch: account_epoch.map(str::to_owned),
                maximum_pixel_dimension: item.maximum_pixel_dimension,
                persist_to_disk: item.persist_to_disk,
                cache_policy: item.cache_policy,
            };
            if !seen.insert(request.clone()) {
                continue;
            }
            result.push(request);
        }
        result
    }

    fn add(
        &self,
        state: &mut CleverImagePrefetchQueueState,
        request: CleverImagePrefetchQueueRequest,
        owner: CleverImagePrefetchQueueOwner,
    ) {
        if let Some(active) = state.active_by_request.get_mut(&request) {
            if active.cancellation_requested {
                Self::merge_successor(state, request, owner);
            } else {
                active.owners.insert(owner);
            }
            return;
        }
        if let Some(pending) = state.pending_by_request.get_mut(&request) {
            pending.owners.insert(owner);
            return;
        }
        if let Some(successor) = state.successor_by_request.get_mut(&request) {
            successor.owners.insert(owner);
            return;
        }
        state.pending_by_request.insert(
            request.clone(),
            CleverImagePrefetchQueuePendingJob {
                owners: HashSet::from([owner]),
            },
        );
        state.pending_order.push(request);
        self.trim_pending_if_needed(state);
    }

    fn trim_pending_if_needed(&self, state: &mut CleverImagePrefetchQueueState) {
        while state.pending_by_request.len() > self.maximum_pending && state.pending_head < state.pending_order.len() {
            let oldest = state.pending_order[state.pending_head].clone();
            state.pending_head += 1;
            state.pending_by_request.remove(&oldest);
        }
    }

    fn merge_successor(
        state: &mut CleverImagePrefetchQueueState,
        request: CleverImagePrefetchQueueRequest,
        owner: CleverImagePrefetchQueueOwner,
    ) {
        state
            .successor_by_request
            .entry(request)
            .or_insert_with(|| CleverImagePrefetchQueuePendingJob { owners: HashSet::new() })
            .owners
            .insert(owner);
    }

    fn remove_directional_owners(state: &mut CleverImagePrefetchQueueState, scope: &str) {
        state.pending_by_request.retain(|_, pending| {
            pending.owners.retain(|owner| !Self::is_directional_owner(owner, scope));
            !pending.owners.is_empty()
        });
        state.successor_by_request.retain(|_, successor| {
            successor.owners.retain(|owner| !Self::is_directional_owner(owner, scope));
            !successor.owners.is_empty()
        });
        for active in state.active_by_request.values_mut() {
            active.owners.retain(|owner| !Self::is_directional_owner(owner, scope));
            if active.owners.is_empty() && !active.cancellation_requested {
                active.cancellation_requested = true;
                active.task.abort();
            }
        }
    }

    fn is_directional_owner(owner: &CleverImagePrefetchQueueOwner, scope: &str) -> bool {
        matches!(owner, CleverImagePrefetchQueueOwner::Directional { scope: owner_scope, .. } if owner_scope == scope)
    }

    fn pump(
        &self,
        state: &mut CleverImagePrefetchQueueState,
    ) -> Vec<(CleverImagePrefetchQueueRequest, u64, futures::future::AbortRegistration, Option<MediaPrefetchEnvironmentSnapshot>)> {
        let mut starts = Vec::new();
        while state.active_by_request.len() < self.maximum_concurrent && state.pending_head < state.pending_order.len() {
            let request = state.pending_order[state.pending_head].clone();
            state.pending_head += 1;
            let Some(pending) = state.pending_by_request.remove(&request) else {
                continue;
            };
            if pending.owners.is_empty() || state.active_by_request.contains_key(&request) {
                continue;
            }
            state.next_task_id = state.next_task_id.wrapping_add(1);
            let task_id = state.next_task_id;
            let (task, registration) = AbortHandle::new_pair();
            state.active_by_request.insert(
                request.clone(),
                CleverImagePrefetchQueueActiveJob {
                    owners: pending.owners,
                    task_id,
                    task,
                    cancellation_requested: false,
                },
            );
            starts.push((request, task_id, registration, state.environment_snapshot));
        }
        if state.pending_head == state.pending_order.len() {
            state.pending_order.clear();
            state.pending_head = 0;
        } else if state.pending_head >= 64 && state.pending_head * 2 >= state.pending_order.len() {
            state.pending_order.drain(..state.pending_head);
            state.pending_head = 0;
        }
        starts
    }

    fn start(
        self: &Arc<Self>,
        starts: Vec<(CleverImagePrefetchQueueRequest, u64, futures::future::AbortRegistration, Option<MediaPrefetchEnvironmentSnapshot>)>,
        cx: &AsyncApp,
    ) {
        for (request, task_id, registration, environment) in starts {
            let queue = Arc::clone(self);
            let operation = Arc::clone(&self.operation);
            cx.spawn(async move |cx| {
                if environment.is_none_or(|environment| environment.is_current()) {
                    let work = operation(request.clone(), cx.clone());
                    let _ = Abortable::new(work, registration).await;
                }
                queue.finish(request, task_id, cx);
            })
            .detach();
        }
    }

    fn finish(self: &Arc<Self>, request: CleverImagePrefetchQueueRequest, task_id: u64, cx: &AsyncApp) {
        let starts = self.state.with(|state| {
            if state.active_by_request.get(&request).map(|active| active.task_id) != Some(task_id) {
                return Vec::new();
            }
            state.active_by_request.remove(&request);
            if let Some(successor) = state.successor_by_request.remove(&request)
                && !successor.owners.is_empty()
            {
                state.pending_by_request.insert(request.clone(), successor);
                state.pending_order.push(request);
            }
            self.pump(state)
        });
        self.start(starts, cx);
    }
}

/// when speculative image prefetch is allowed, same conditions as video warm start
/// no speculative warmup in background, low power mode or serious thermal state
pub enum CleverImagePrefetchPolicy {}

impl CleverImagePrefetchPolicy {
    pub fn allows_prefetch(
        is_application_active: bool,
        is_low_power_mode_enabled: bool,
        thermal_level: AnimatedMediaThermalLevel,
    ) -> bool {
        if !is_application_active || is_low_power_mode_enabled {
            return false;
        }
        match thermal_level {
            AnimatedMediaThermalLevel::Nominal | AnimatedMediaThermalLevel::Fair => true,
            AnimatedMediaThermalLevel::Serious | AnimatedMediaThermalLevel::Critical => false,
        }
    }

    pub fn allows_prefetch_now(cx: &mut App) -> bool {
        let energy = RiseEnergySavingStore::shared(cx);
        let reduces_prefetch = energy.read(cx).reduces(RiseEnergySavingFeature::MediaPrefetch);
        let is_foreground = RiseMediaForegroundGate::shared(cx).read(cx).is_foreground();
        Self::allows_prefetch(
            is_foreground,
            system_low_power_mode() || reduces_prefetch,
            AnimatedMediaThermalLevel::current(),
        )
    }
}

/// a transition invalidates admissions as well as queued work
/// shared visible loads keep their own leases, only prefetch owners are cancelled here
pub enum MediaPrefetchEnvironment {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaPrefetchEnvironmentSnapshot {
    pub generation: u64,
    pub allows_prefetch: bool,
}

impl MediaPrefetchEnvironmentSnapshot {
    pub fn is_current(&self) -> bool {
        self.allows_prefetch && {
            let state = MEDIA_PREFETCH_ENVIRONMENT_STATE.lock();
            state.generation == self.generation && state.allows_prefetch
        }
    }
}

static MEDIA_PREFETCH_ENVIRONMENT_STATE: Mutex<MediaPrefetchEnvironmentSnapshot> = Mutex::new(MediaPrefetchEnvironmentSnapshot {
    generation: 0,
    allows_prefetch: false,
});

struct MediaPrefetchEnvironmentLifecycleObservers {
    _notifications: Vec<RiseSubscription>,
    _energy: Subscription,
    _delivery: Task<()>,
}

impl Global for MediaPrefetchEnvironmentLifecycleObservers {}

impl MediaPrefetchEnvironment {
    pub fn snapshot(cx: &mut App) -> MediaPrefetchEnvironmentSnapshot {
        Self::install_lifecycle_observers(cx);
        let allowed = CleverImagePrefetchPolicy::allows_prefetch_now(cx);
        let mut state = MEDIA_PREFETCH_ENVIRONMENT_STATE.lock();
        if state.allows_prefetch != allowed {
            *state = MediaPrefetchEnvironmentSnapshot {
                generation: state.generation.wrapping_add(1),
                allows_prefetch: allowed,
            };
        }
        *state
    }

    pub fn current_snapshot() -> MediaPrefetchEnvironmentSnapshot {
        *MEDIA_PREFETCH_ENVIRONMENT_STATE.lock()
    }

    fn install_lifecycle_observers(cx: &mut App) {
        if cx.has_global::<MediaPrefetchEnvironmentLifecycleObservers>() {
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let names = [
            RISE_MEDIA_FOREGROUND_DID_CHANGE,
            RiseNotificationName::new(POWER_STATE_DID_CHANGE),
            RiseNotificationName::new(THERMAL_STATE_DID_CHANGE),
            UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION,
        ];
        let notifications = names
            .into_iter()
            .map(|name| {
                let sender = sender.clone();
                RiseAppEvents::default_center().observe(name, move |_| {
                    let _ = sender.unbounded_send(());
                })
            })
            .collect();
        let energy_sender = sender.clone();
        let energy = RiseEnergySavingStore::shared(cx);
        let energy = cx.subscribe(&energy, move |_, _: &rise_energy::rise_energy_saving::RiseEnergySavingDidChange, _| {
            let _ = energy_sender.unbounded_send(());
        });
        let delivery = cx.spawn(async move |cx| {
            while receiver.next().await.is_some() {
                let next = cx.update(|cx| {
                    let allowed = CleverImagePrefetchPolicy::allows_prefetch_now(cx);
                    let mut state = MEDIA_PREFETCH_ENVIRONMENT_STATE.lock();
                    *state = MediaPrefetchEnvironmentSnapshot {
                        generation: state.generation.wrapping_add(1),
                        allows_prefetch: allowed,
                    };
                    *state
                });
                CleverImageDiskPrefetch::environment_did_change(next).await;
                LottieStickerPrefetch::environment_did_change(next).await;
            }
        });
        cx.set_global(MediaPrefetchEnvironmentLifecycleObservers {
            _notifications: notifications,
            _energy: energy,
            _delivery: delivery,
        });
    }
}

pub enum CleverImageDiskPrefetch {}

static PREFETCH_QUEUE: LazyLock<Arc<CleverImagePrefetchQueue>> = LazyLock::new(|| {
    CleverImagePrefetchQueue::new(
        RiseImagePrefetchLimits::queue_concurrency(),
        RiseImagePrefetchLimits::queue_pending(),
        Arc::new(|request: CleverImagePrefetchQueueRequest, cx: AsyncApp| {
            async move {
                let environment = MediaPrefetchEnvironment::current_snapshot();
                if !environment.is_current()
                    || !SaiAccountCacheScope::is_current_account_epoch(
                        request.account_user_id.as_deref(),
                        request.account_epoch.as_deref(),
                    )
                {
                    return;
                }
                let _ = CleverImageBitmapLoader::load(
                    &request.cache_key,
                    request.persist_to_disk,
                    true,
                    request.account_user_id.clone(),
                    request.account_epoch.clone(),
                    request.maximum_pixel_dimension,
                    request.cache_policy,
                    CleverImageWorkLane::Speculative,
                    &cx,
                )
                .await;
            }
            .boxed_local()
        }),
    )
});

impl CleverImageDiskPrefetch {
    const PREPARE_FIRST_IMAGES_LIMIT: usize = 12;
    const PREPARE_FIRST_IMAGES_BATCH: usize = 3;
    const PREPARE_FIRST_IMAGES_PIXEL_DIMENSION: i64 = 720;
    pub const PREPARE_FIRST_IMAGES_MAXIMUM_WAIT: Duration = Duration::from_millis(160);

    pub fn schedule_values_from(value: &Value, key_patterns: &[String], cx: &mut App) {
        let patterns: Vec<String> = key_patterns
            .iter()
            .map(|pattern| trimming_whitespaces_and_newlines(pattern).to_owned())
            .filter(|pattern| !pattern.is_empty())
            .collect();
        if patterns.is_empty() {
            return;
        }
        let urls = Self::extract_http_url_strings(value, &patterns);
        Self::schedule(&urls, None, true, CleverImageCachePolicy::Standard, cx);
    }

    pub fn extract_http_url_strings(value: &Value, key_patterns: &[String]) -> Vec<String> {
        let Some(root) = AnyDictionaryBridge::dictionary(Some(value)) else {
            return Vec::new();
        };
        let mut urls = Vec::new();
        for pattern in key_patterns {
            Self::collect_urls(&root, pattern, &mut urls);
        }
        let unique: HashSet<String> = urls.into_iter().collect();
        unique.into_iter().collect()
    }

    pub fn schedule(
        url_strings: &[String],
        maximum_pixel_dimension: Option<i64>,
        persist_to_disk: bool,
        cache_policy: CleverImageCachePolicy,
        cx: &mut App,
    ) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let environment = MediaPrefetchEnvironment::snapshot(cx);
        if !environment.is_current() {
            return;
        }
        let url_strings = url_strings.to_vec();
        cx.spawn(async move |cx| {
            PREFETCH_QUEUE
                .enqueue(
                    &url_strings,
                    account_user_id,
                    account_epoch,
                    maximum_pixel_dimension,
                    persist_to_disk,
                    cache_policy,
                    Some(environment),
                    cx,
                )
                .await;
        })
        .detach();
    }

    pub async fn prepare_first_images(url_strings: &[String], maximum_wait: Duration, cx: &AsyncApp) {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let mut seen = HashSet::new();
        let urls: Vec<String> = url_strings
            .iter()
            .filter_map(|raw| {
                let value = trimming_whitespaces_and_newlines(raw).to_owned();
                let url = Url::from_string(&value)?;
                let scheme = url.scheme()?.to_lowercase();
                if (scheme == "https" || scheme == "http") && seen.insert(value.clone()) {
                    Some(value)
                } else {
                    None
                }
            })
            .take(Self::PREPARE_FIRST_IMAGES_LIMIT)
            .collect();
        if urls.is_empty() {
            return;
        }
        let loads = async {
            for batch in urls.chunks(Self::PREPARE_FIRST_IMAGES_BATCH) {
                let workers = batch.iter().map(|url| {
                    CleverImageBitmapLoader::load(
                        url,
                        true,
                        false,
                        account_user_id.clone(),
                        account_epoch.clone(),
                        Some(Self::PREPARE_FIRST_IMAGES_PIXEL_DIMENSION),
                        CleverImageCachePolicy::Standard,
                        CleverImageWorkLane::Visible,
                        cx,
                    )
                });
                futures::future::join_all(workers).await;
            }
        };
        let timeout = cx.background_executor().timer(maximum_wait);
        let _ = futures::future::select(Box::pin(loads), timeout).await;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replace_directional_urls(
        url_strings: &[String],
        scope: &str,
        generation: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        maximum_pixel_dimension: Option<i64>,
        persist_to_disk: bool,
        cache_policy: CleverImageCachePolicy,
        cx: &mut App,
    ) {
        let items: Vec<CleverImagePrefetchItem> = url_strings
            .iter()
            .map(|url_string| CleverImagePrefetchItem {
                url_string: url_string.clone(),
                maximum_pixel_dimension,
                persist_to_disk,
                cache_policy,
            })
            .collect();
        Self::replace_directional(items, scope, generation, account_user_id, account_epoch, cx);
    }

    pub fn replace_directional(
        items: Vec<CleverImagePrefetchItem>,
        scope: &str,
        generation: u64,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut App,
    ) {
        let environment = MediaPrefetchEnvironment::snapshot(cx);
        if !environment.is_current() {
            Self::cancel_directional(scope, generation, cx);
            return;
        }
        let scope = scope.to_owned();
        cx.spawn(async move |cx| {
            PREFETCH_QUEUE
                .replace_directional(&items, &scope, generation, account_user_id, account_epoch, Some(environment), cx)
                .await;
        })
        .detach();
    }

    pub fn cancel_directional(scope: &str, generation: u64, cx: &mut App) {
        let scope = scope.to_owned();
        cx.spawn(async move |cx| {
            PREFETCH_QUEUE.cancel_directional(&scope, generation, cx).await;
        })
        .detach();
    }

    pub async fn environment_did_change(snapshot: MediaPrefetchEnvironmentSnapshot) {
        PREFETCH_QUEUE.update_environment(snapshot).await;
    }

    pub fn reset_account_state() {
        CleverImageDiskCache::invalidate_pending_operations(SaiAccountCacheScope::active_user_id().as_deref());
        CleverImageRenderMemoryCache::clear_all();
        CleverImageBitmapLoader::reset_shared_loads();
        let queue = Arc::clone(&PREFETCH_QUEUE);
        RiseAccountRuntime::current().spawn(async move {
            queue.reset(false).await;
        });
    }

    fn collect_urls(dict: &Map<String, Value>, pattern: &str, urls: &mut Vec<String>) {
        if let Some((array_key, nested_key)) = pattern.split_once(".*.") {
            let Some(array) = dict.get(array_key).and_then(Value::as_array) else {
                return;
            };
            for item in array {
                let Some(slot) = AnyDictionaryBridge::dictionary(Some(item)) else {
                    continue;
                };
                if let Some(url) = Self::http_url_string(slot.get(nested_key)) {
                    urls.push(url);
                }
            }
        } else if let Some(url) = Self::http_url_string(dict.get(pattern)) {
            urls.push(url);
        }
    }

    fn http_url_string(any: Option<&Value>) -> Option<String> {
        let string = any?.as_str()?;
        let trimmed = trimming_whitespaces_and_newlines(string);
        if !trimmed.to_lowercase().starts_with("http") || Url::from_string(trimmed).is_none() {
            return None;
        }
        Some(trimmed.to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CleverImageLoadRequest {
    url_string: String,
    persist_to_disk: bool,
    network_if_missing: bool,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    maximum_pixel_dimension: Option<i64>,
    cache_policy: CleverImageCachePolicy,
}

#[derive(Clone, Default)]
pub struct CleverImageLoadResult {
    pub image: Option<CleverImage>,
}

impl CleverImageLoadResult {
    pub fn new(image: Option<CleverImage>) -> Self {
        Self { image }
    }
}

struct CleverImageSharedLoadRegistryEntry<Value: Clone> {
    id: u64,
    task: Shared<BoxFuture<'static, Value>>,
    abort: AbortHandle,
    waiter_ids: HashSet<u64>,
}

struct CleverImageSharedLoadRegistryState<Key, Value: Clone> {
    entries: HashMap<Key, CleverImageSharedLoadRegistryEntry<Value>>,
    next_entry_id: u64,
    next_waiter_id: u64,
}

pub struct CleverImageSharedLoadRegistry<Key, Value: Clone> {
    state: RiseIsolated<CleverImageSharedLoadRegistryState<Key, Value>>,
}

struct CleverImageSharedLoadRegistryLease<'a, Key: Eq + Hash + Clone, Value: Clone> {
    registry: &'a CleverImageSharedLoadRegistry<Key, Value>,
    key: Key,
    entry_id: u64,
    waiter_id: u64,
    finished: bool,
}

impl<Key: Eq + Hash + Clone, Value: Clone> Drop for CleverImageSharedLoadRegistryLease<'_, Key, Value> {
    fn drop(&mut self) {
        self.registry
            .release(&self.key, self.entry_id, self.waiter_id, !self.finished);
    }
}

impl<Key, Value> Default for CleverImageSharedLoadRegistry<Key, Value>
where
    Key: Eq + Hash + Clone + Send + 'static,
    Value: Clone + Default + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<Key, Value> CleverImageSharedLoadRegistry<Key, Value>
where
    Key: Eq + Hash + Clone + Send + 'static,
    Value: Clone + Default + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            state: RiseIsolated::new(CleverImageSharedLoadRegistryState {
                entries: HashMap::new(),
                next_entry_id: 0,
                next_waiter_id: 0,
            }),
        }
    }

    pub async fn value<Operation>(&self, key: Key, operation: Operation, cx: &AsyncApp) -> Value
    where
        Operation: AsyncFnOnce(&mut AsyncApp) -> Value + 'static,
    {
        let (entry_id, waiter_id, task) = self.state.with(|state| {
            state.next_waiter_id = state.next_waiter_id.wrapping_add(1);
            let waiter_id = state.next_waiter_id;
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.waiter_ids.insert(waiter_id);
                return (entry.id, waiter_id, entry.task.clone());
            }
            state.next_entry_id = state.next_entry_id.wrapping_add(1);
            let entry_id = state.next_entry_id;
            let (abort, registration) = AbortHandle::new_pair();
            let spawned = cx.spawn(async move |cx| operation(cx).await);
            let task: BoxFuture<'static, Value> = Abortable::new(spawned, registration)
                .map(Result::unwrap_or_default)
                .boxed();
            let task = task.shared();
            state.entries.insert(
                key.clone(),
                CleverImageSharedLoadRegistryEntry {
                    id: entry_id,
                    task: task.clone(),
                    abort,
                    waiter_ids: HashSet::from([waiter_id]),
                },
            );
            (entry_id, waiter_id, task)
        });
        let mut lease = CleverImageSharedLoadRegistryLease {
            registry: self,
            key,
            entry_id,
            waiter_id,
            finished: false,
        };
        let value = task.await;
        lease.finished = true;
        value
    }

    pub async fn reset(&self) {
        self.state.with(|state| {
            for entry in state.entries.values() {
                entry.abort.abort();
            }
            state.entries.clear();
        });
    }

    pub async fn active_entry_count(&self) -> usize {
        self.state.with(|state| state.entries.len())
    }

    pub async fn waiter_count(&self, key: &Key) -> usize {
        self.state
            .with(|state| state.entries.get(key).map_or(0, |entry| entry.waiter_ids.len()))
    }
}

impl<Key: Eq + Hash + Clone, Value: Clone> CleverImageSharedLoadRegistry<Key, Value> {
    fn release(&self, key: &Key, entry_id: u64, waiter_id: u64, cancel_task_when_last: bool) {
        self.state.with(|state| {
            let Some(entry) = state.entries.get_mut(key) else {
                return;
            };
            if entry.id != entry_id || !entry.waiter_ids.remove(&waiter_id) {
                return;
            }
            if entry.waiter_ids.is_empty()
                && let Some(entry) = state.entries.remove(key)
                && cancel_task_when_last
            {
                entry.abort.abort();
            }
        });
    }
}

/// who asks for the image, what the reader looks at or what is warmed for later
/// visible and speculative work share one permit pool
/// without lanes a fifo queue would put the image under the finger behind the prefetch
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CleverImageWorkLane {
    Visible,
    Speculative,
}

struct CleverImageAsyncWorkLimiterWaiter {
    lane: CleverImageWorkLane,
    continuation: oneshot::Sender<bool>,
}

struct CleverImageAsyncWorkLimiterState {
    active_count: usize,
    active_speculative_count: usize,
    next_waiter_id: u64,
    waiters: HashMap<u64, CleverImageAsyncWorkLimiterWaiter>,
    visible_order: Vec<u64>,
    visible_head: usize,
    speculative_order: Vec<u64>,
    speculative_head: usize,
}

pub struct CleverImageAsyncWorkLimiter {
    maximum_concurrent: usize,
    maximum_speculative: usize,
    state: RiseIsolated<CleverImageAsyncWorkLimiterState>,
}

/// a granted permit, dropping it releases the lane even when the work was cancelled
pub struct CleverImageAsyncWorkPermit<'a> {
    limiter: &'a CleverImageAsyncWorkLimiter,
    lane: CleverImageWorkLane,
}

impl Drop for CleverImageAsyncWorkPermit<'_> {
    fn drop(&mut self) {
        self.limiter.release(self.lane);
    }
}

struct CleverImageAsyncWorkLimiterPendingWaiter<'a> {
    limiter: &'a CleverImageAsyncWorkLimiter,
    waiter_id: u64,
    lane: CleverImageWorkLane,
    receiver: Option<oneshot::Receiver<bool>>,
}

impl Drop for CleverImageAsyncWorkLimiterPendingWaiter<'_> {
    fn drop(&mut self) {
        let Some(mut receiver) = self.receiver.take() else {
            return;
        };
        if !self.limiter.cancel(self.waiter_id) {
            if let Ok(Some(true)) = receiver.try_recv() {
                self.limiter.release(self.lane);
            }
        }
    }
}

impl CleverImageAsyncWorkLimiter {
    /// one permit is always reserved for the visible lane
    /// otherwise a prefetch holding the whole pool with long downloads blocks the first frame until
    /// its slowest one ends
    pub fn new(maximum_concurrent: i64) -> Self {
        let bound = usize::try_from(maximum_concurrent.max(1)).unwrap_or(1);
        Self {
            maximum_concurrent: bound,
            maximum_speculative: bound.saturating_sub(1).max(1),
            state: RiseIsolated::new(CleverImageAsyncWorkLimiterState {
                active_count: 0,
                active_speculative_count: 0,
                next_waiter_id: 0,
                waiters: HashMap::new(),
                visible_order: Vec::new(),
                visible_head: 0,
                speculative_order: Vec::new(),
                speculative_head: 0,
            }),
        }
    }

    pub async fn acquire(&self, lane: CleverImageWorkLane) -> Option<CleverImageAsyncWorkPermit<'_>> {
        let queued = self.state.with(|state| {
            if self.can_start(state, lane) {
                Self::grant(state, lane);
                return None;
            }
            state.next_waiter_id = state.next_waiter_id.wrapping_add(1);
            let waiter_id = state.next_waiter_id;
            let (continuation, receiver) = oneshot::channel();
            state.waiters.insert(waiter_id, CleverImageAsyncWorkLimiterWaiter { lane, continuation });
            match lane {
                CleverImageWorkLane::Visible => state.visible_order.push(waiter_id),
                CleverImageWorkLane::Speculative => state.speculative_order.push(waiter_id),
            }
            Some((waiter_id, receiver))
        });
        let Some((waiter_id, receiver)) = queued else {
            return Some(CleverImageAsyncWorkPermit { limiter: self, lane });
        };
        let mut pending = CleverImageAsyncWorkLimiterPendingWaiter {
            limiter: self,
            waiter_id,
            lane,
            receiver: Some(receiver),
        };
        let granted = match pending.receiver.as_mut() {
            Some(receiver) => receiver.await.unwrap_or(false),
            None => false,
        };
        pending.receiver = None;
        granted.then_some(CleverImageAsyncWorkPermit { limiter: self, lane })
    }

    fn release(&self, lane: CleverImageWorkLane) {
        self.state.with(|state| {
            state.active_count = state.active_count.saturating_sub(1);
            if lane == CleverImageWorkLane::Speculative {
                state.active_speculative_count = state.active_speculative_count.saturating_sub(1);
            }
            self.pump_waiters(state);
        });
    }

    pub async fn snapshot(&self) -> (usize, usize) {
        self.state.with(|state| (state.active_count, state.waiters.len()))
    }

    fn can_start(&self, state: &CleverImageAsyncWorkLimiterState, lane: CleverImageWorkLane) -> bool {
        if state.active_count >= self.maximum_concurrent {
            return false;
        }
        if lane != CleverImageWorkLane::Speculative {
            return true;
        }
        state.active_speculative_count < self.maximum_speculative
    }

    fn grant(state: &mut CleverImageAsyncWorkLimiterState, lane: CleverImageWorkLane) {
        state.active_count += 1;
        if lane == CleverImageWorkLane::Speculative {
            state.active_speculative_count += 1;
        }
    }

    /// visible queue drains first and completely, only then a freed permit goes to prefetch
    fn pump_waiters(&self, state: &mut CleverImageAsyncWorkLimiterState) {
        while self.resume_next_waiter(state) {}
        Self::compact(&mut state.visible_order, &mut state.visible_head);
        Self::compact(&mut state.speculative_order, &mut state.speculative_head);
    }

    fn resume_next_waiter(&self, state: &mut CleverImageAsyncWorkLimiterState) -> bool {
        for lane in [CleverImageWorkLane::Visible, CleverImageWorkLane::Speculative] {
            if !self.can_start(state, lane) {
                continue;
            }
            let Some(waiter) = Self::take_waiter(state, lane) else {
                continue;
            };
            Self::grant(state, waiter.lane);
            if waiter.continuation.send(true).is_err() {
                state.active_count = state.active_count.saturating_sub(1);
                if waiter.lane == CleverImageWorkLane::Speculative {
                    state.active_speculative_count = state.active_speculative_count.saturating_sub(1);
                }
            }
            return true;
        }
        false
    }

    fn take_waiter(
        state: &mut CleverImageAsyncWorkLimiterState,
        lane: CleverImageWorkLane,
    ) -> Option<CleverImageAsyncWorkLimiterWaiter> {
        let (order, head) = match lane {
            CleverImageWorkLane::Visible => (&state.visible_order, &mut state.visible_head),
            CleverImageWorkLane::Speculative => (&state.speculative_order, &mut state.speculative_head),
        };
        while *head < order.len() {
            let waiter_id = order[*head];
            *head += 1;
            if let Some(waiter) = state.waiters.remove(&waiter_id) {
                return Some(waiter);
            }
        }
        None
    }

    fn cancel(&self, waiter_id: u64) -> bool {
        self.state.with(|state| {
            let Some(waiter) = state.waiters.remove(&waiter_id) else {
                return false;
            };
            let _ = waiter.continuation.send(false);
            self.pump_waiters(state);
            true
        })
    }

    fn compact(order: &mut Vec<u64>, head: &mut usize) {
        if *head == order.len() {
            order.clear();
            *head = 0;
        } else if *head >= 64 && *head * 2 >= order.len() {
            order.drain(..*head);
            *head = 0;
        }
    }
}

pub enum CleverImageBitmapLoader {}

static IN_FLIGHT_REGISTRY: LazyLock<CleverImageSharedLoadRegistry<CleverImageLoadRequest, CleverImageLoadResult>> =
    LazyLock::new(CleverImageSharedLoadRegistry::new);
static BITMAP_WORK_LIMITER: LazyLock<CleverImageAsyncWorkLimiter> =
    LazyLock::new(|| CleverImageAsyncWorkLimiter::new(RiseLimits::client_value("image_bitmap_work_concurrency")));
static NETWORK_WORK_LIMITER: LazyLock<CleverImageAsyncWorkLimiter> =
    LazyLock::new(|| CleverImageAsyncWorkLimiter::new(RiseLimits::client_value("image_network_work_concurrency")));
static PENDING_DISK_PROMOTIONS: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

struct CleverImageDiskPromotionGuard {
    promotion_key: String,
}

impl Drop for CleverImageDiskPromotionGuard {
    fn drop(&mut self) {
        PENDING_DISK_PROMOTIONS.lock().remove(&self.promotion_key);
    }
}

impl CleverImageBitmapLoader {
    fn maximum_network_image_bytes() -> i64 {
        RiseLimits::client_value("image_network_body_bytes")
    }

    fn log_secure_refusal(sent_token: Option<&str>, cx: &mut AsyncApp) {
        let Some(token) = sent_token else {
            tracing::warn!(target: "riseonly::X422", "secure 401 without token");
            return;
        };
        let (still_current, owner) = cx.update(|cx| {
            let current = sai_auth_access_token(cx);
            let owner = sai_auth_hooks().and_then(|hooks| (hooks.extract_user_id)(token));
            (current.as_deref() == Some(token), owner)
        });
        let device_now = Date::now();
        tracing::warn!(
            target: "riseonly::X422",
            "secure 401: owner {:?} active {:?} still_current {} expired_by_server_clock {} expired_by_device_clock {} server_minus_device_s {:.1}",
            owner,
            SaiAccountCacheScope::active_user_id(),
            still_current,
            sai_auth_is_expired(token, 0.0),
            AccessTokenUserId::expiration_date(token).is_some_and(|exp| exp.time_interval_since(&device_now) < 0.0),
            RiseServerClock::now().time_interval_since(&device_now)
        );
    }

    /// blocking, may read and decode from disk so keep it off the main thread
    pub fn image_from_memory_or_disk(
        url_string: &str,
        persist_to_disk: bool,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        cache_policy: CleverImageCachePolicy,
        expected_invalidation_token: Option<u64>,
    ) -> Option<CleverImage> {
        let key = CleverImageDiskCache::normalized_cache_key(url_string);
        if cache_policy == CleverImageCachePolicy::PrivateEphemeral {
            return None;
        }
        if cache_policy == CleverImageCachePolicy::MemoryOnly {
            return MediaViewerImageMemoryCache::memory_only(&key, account_user_id);
        }
        if let Some(memory) = MediaViewerImageMemoryCache::image(&key, account_user_id, CleverImageCachePolicy::Standard) {
            if persist_to_disk {
                Self::schedule_disk_promotion(
                    Arc::clone(&memory),
                    &key,
                    account_user_id,
                    account_epoch,
                    expected_invalidation_token,
                );
            }
            return Some(memory);
        }
        if persist_to_disk
            && let Some(data) = CleverImageDiskCache::image_data(&key, account_user_id, expected_invalidation_token)
            && let Some(disk) = CleverImageBitmapDecoder::decode(&data, None)
        {
            MediaViewerImageMemoryCache::store(Arc::clone(&disk), &key, account_user_id);
            return Some(disk);
        }
        if let Some(url_cache) = MediaViewerBitmapLoader::ui_image_from_url_cache_only(&key) {
            MediaViewerImageMemoryCache::store(Arc::clone(&url_cache), &key, account_user_id);
            if persist_to_disk {
                CleverImageDiskCache::store_image(&url_cache, &key, account_user_id, expected_invalidation_token);
            }
            return Some(url_cache);
        }
        None
    }

    fn schedule_disk_promotion(
        image: CleverImage,
        cache_key: &str,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        expected_invalidation_token: Option<u64>,
    ) {
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
            return;
        }
        let invalidation_token =
            expected_invalidation_token.unwrap_or_else(|| CleverImageDiskCache::invalidation_token(account_user_id));
        let promotion_key = format!("{}|{}", account_user_id.unwrap_or("anonymous"), cache_key);
        if !PENDING_DISK_PROMOTIONS.lock().insert(promotion_key.clone()) {
            return;
        }
        let guard = CleverImageDiskPromotionGuard { promotion_key };
        let cache_key = cache_key.to_owned();
        let account_user_id = account_user_id.map(str::to_owned);
        let account_epoch = account_epoch.map(str::to_owned);
        RiseAccountRuntime::current().spawn_blocking(move || {
            let _guard = guard;
            if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
                || CleverImageDiskCache::contains(&cache_key, account_user_id.as_deref(), Some(invalidation_token))
            {
                return;
            }
            CleverImageDiskCache::store_image(&image, &cache_key, account_user_id.as_deref(), Some(invalidation_token));
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn load(
        url_string: &str,
        persist_to_disk: bool,
        network_if_missing: bool,
        expected_account_user_id: Option<String>,
        expected_account_epoch: Option<String>,
        maximum_pixel_dimension: Option<i64>,
        cache_policy: CleverImageCachePolicy,
        lane: CleverImageWorkLane,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        let request = CleverImageLoadRequest {
            url_string: CleverImageDiskCache::normalized_cache_key(url_string),
            persist_to_disk,
            network_if_missing,
            account_user_id: expected_account_user_id,
            account_epoch: expected_account_epoch,
            maximum_pixel_dimension: maximum_pixel_dimension.map(|value| value.max(1)),
            cache_policy,
        };
        let operation_request = request.clone();
        let result = IN_FLIGHT_REGISTRY
            .value(
                request,
                async move |cx: &mut AsyncApp| {
                    CleverImageLoadResult::new(
                        Self::perform_load(
                            &operation_request.url_string,
                            operation_request.persist_to_disk,
                            operation_request.network_if_missing,
                            operation_request.account_user_id.clone(),
                            operation_request.account_epoch.clone(),
                            operation_request.maximum_pixel_dimension,
                            operation_request.cache_policy,
                            lane,
                            cx,
                        )
                        .await,
                    )
                },
                cx,
            )
            .await;
        result.image
    }

    /// local bytes and derived bitmaps share the image decode budget too
    pub async fn prepare_bitmap(
        operation: impl FnOnce() -> Option<CleverImage> + Send + 'static,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        let _permit = BITMAP_WORK_LIMITER.acquire(CleverImageWorkLane::Visible).await?;
        cx.background_executor().spawn(async move { operation() }).await
    }

    pub fn reset_shared_loads() {
        RiseAccountRuntime::current().spawn(async {
            IN_FLIGHT_REGISTRY.reset().await;
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn perform_load(
        url_string: &str,
        persist_to_disk: bool,
        network_if_missing: bool,
        expected_account_user_id: Option<String>,
        expected_account_epoch: Option<String>,
        maximum_pixel_dimension: Option<i64>,
        cache_policy: CleverImageCachePolicy,
        lane: CleverImageWorkLane,
        cx: &mut AsyncApp,
    ) -> Option<CleverImage> {
        if !SaiAccountCacheScope::is_current_account_epoch(
            expected_account_user_id.as_deref(),
            expected_account_epoch.as_deref(),
        ) {
            return None;
        }
        let key = CleverImageDiskCache::normalized_cache_key(url_string);
        let maximum_pixel_dimension = maximum_pixel_dimension.map(|value| value.max(1));
        let invalidation_token = CleverImageDiskCache::invalidation_token(expected_account_user_id.as_deref());

        if let Some(memory_hit) = Self::memory_image(
            &key,
            persist_to_disk,
            expected_account_user_id.as_deref(),
            expected_account_epoch.as_deref(),
            maximum_pixel_dimension,
            cache_policy,
            Some(invalidation_token),
            true,
        ) {
            return Some(memory_hit);
        }

        let cached = Self::load_cached_image_bounded(
            key.clone(),
            persist_to_disk,
            expected_account_user_id.clone(),
            expected_account_epoch.clone(),
            maximum_pixel_dimension,
            cache_policy,
            Some(invalidation_token),
            lane,
        )
        .await;
        if !SaiAccountCacheScope::is_current_account_epoch(
            expected_account_user_id.as_deref(),
            expected_account_epoch.as_deref(),
        ) {
            return None;
        }
        if let Some(hit) = cached {
            return Some(hit);
        }

        if !key.to_lowercase().starts_with("http") {
            return None;
        }
        let url = Url::from_string(&key)?;
        if !network_if_missing {
            return None;
        }

        let _network_permit = NETWORK_WORK_LIMITER.acquire(lane).await?;
        Self::load_network_image(
            url,
            key,
            persist_to_disk,
            expected_account_user_id,
            expected_account_epoch,
            maximum_pixel_dimension,
            cache_policy,
            invalidation_token,
            lane,
            cx,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn load_network_image(
        url: Url,
        key: String,
        persist_to_disk: bool,
        expected_account_user_id: Option<String>,
        expected_account_epoch: Option<String>,
        maximum_pixel_dimension: Option<i64>,
        cache_policy: CleverImageCachePolicy,
        invalidation_token: u64,
        work_lane: CleverImageWorkLane,
        cx: &mut AsyncApp,
    ) -> Option<CleverImage> {
        let transfer_key = RiseMediaCacheIdentity::key(&url);
        let mut abandon = CleverImageTransferAbandonGuard {
            transfer_key: Some(transfer_key.clone()),
        };
        let mut unsigned_request = URLRequest::new(url.clone());
        if cache_policy != CleverImageCachePolicy::Standard {
            unsigned_request.set_value("no-store", reqwest::header::CACHE_CONTROL.as_str());
            unsigned_request.set_value("no-cache", reqwest::header::PRAGMA.as_str());
        }
        let mut sent_token = RemoteMediaDiskCache::bearer_token(&url, None, cx).await;
        let maximum_network_image_bytes = Self::maximum_network_image_bytes();
        let fetch_lane = if cache_policy == CleverImageCachePolicy::Standard && !persist_to_disk {
            RiseProgressiveDataFetcherLane::Standard
        } else {
            RiseProgressiveDataFetcherLane::PrivateEphemeral
        };
        let task_priority = if work_lane == CleverImageWorkLane::Speculative {
            URL_SESSION_TASK_LOW_PRIORITY
        } else {
            RiseProgressiveDataFetcher::DEFAULT_PRIORITY
        };
        let progress_key = transfer_key.clone();
        let on_progress: RiseProgressiveDataFetcherOnProgress = Arc::new(move |progress: RemoteMediaDownloadProgress| {
            let cache_key = progress_key.clone();
            RiseAccountRuntime::current().spawn(async move {
                RiseMediaTransferHub::shared()
                    .ingest(cache_key, progress.downloaded_bytes, progress.total_bytes.unwrap_or(0), false)
                    .await;
            });
        });
        let mut retried_after_refusal = false;
        let (data, response, request) = loop {
            let mut request = unsigned_request.clone();
            if let Some(token) = &sent_token {
                request.set_value(&format!("Bearer {token}"), reqwest::header::AUTHORIZATION.as_str());
            }
            let fetched = RemoteMediaDownloadRetryPolicy::retrying(|| {
                let request = request.clone();
                let on_progress = Arc::clone(&on_progress);
                let account_user_id = expected_account_user_id.clone();
                let account_epoch = expected_account_epoch.clone();
                let mut cx = cx.clone();
                async move {
                    RiseProgressiveDataFetcher::data(
                        request,
                        maximum_network_image_bytes,
                        fetch_lane,
                        account_user_id,
                        account_epoch,
                        task_priority,
                        on_progress,
                        &mut cx,
                    )
                    .await
                }
            })
            .await;
            let Ok((data, response)) = fetched else {
                tracing::warn!(target: "riseonly::clever_image", "fetch failed: {}", url.absolute_string());
                return None;
            };
            if response.status_code == 401
                && !retried_after_refusal
                && let Some(refused) = sent_token.as_deref()
            {
                Self::log_secure_refusal(Some(refused), cx);
                retried_after_refusal = true;
                if let Some(renewed) = RemoteMediaDiskCache::bearer_token_after_refusal(&url, refused, cx).await {
                    sent_token = Some(renewed);
                    continue;
                }
            }
            break (data, response, request);
        };
        abandon.transfer_key = None;
        let byte_count = i64::try_from(data.len()).unwrap_or(i64::MAX);
        let finished_key = transfer_key;
        RiseAccountRuntime::current().spawn(async move {
            RiseMediaTransferHub::shared()
                .ingest(finished_key, byte_count, byte_count, true)
                .await;
        });
        if !(200..300).contains(&response.status_code)
            || (response.expected_content_length() > maximum_network_image_bytes && response.expected_content_length() >= 0)
            || byte_count > maximum_network_image_bytes
        {
            tracing::warn!(
                target: "riseonly::clever_image",
                "fetch refused: status {} bytes {} {}",
                response.status_code,
                byte_count,
                url.absolute_string()
            );
            if response.status_code == 401 {
                Self::log_secure_refusal(sent_token.as_deref(), cx);
            }
            return None;
        }
        if !SaiAccountCacheScope::is_current_account_epoch(
            expected_account_user_id.as_deref(),
            expected_account_epoch.as_deref(),
        ) {
            return None;
        }

        let data = Arc::new(data);
        let image = {
            let _bitmap_permit = BITMAP_WORK_LIMITER.acquire(work_lane).await?;
            let decode_data = Arc::clone(&data);
            cx.background_executor()
                .spawn(async move { CleverImageBitmapDecoder::decode(&decode_data, maximum_pixel_dimension) })
                .await
        };
        if !SaiAccountCacheScope::is_current_account_epoch(
            expected_account_user_id.as_deref(),
            expected_account_epoch.as_deref(),
        ) {
            return None;
        }
        let Some(image) = image else {
            tracing::warn!(target: "riseonly::clever_image", "decode failed: bytes {} {}", byte_count, url.absolute_string());
            return None;
        };
        if cache_policy == CleverImageCachePolicy::PrivateEphemeral {
            return Some(image);
        }
        if let Some(maximum_pixel_dimension) = maximum_pixel_dimension {
            CleverImageRenderMemoryCache::store(
                Arc::clone(&image),
                &key,
                maximum_pixel_dimension,
                expected_account_user_id.as_deref(),
                expected_account_epoch.as_deref(),
            );
        } else {
            MediaViewerImageMemoryCache::store(Arc::clone(&image), &key, expected_account_user_id.as_deref());
        }
        if cache_policy == CleverImageCachePolicy::MemoryOnly {
            return Some(image);
        }
        if persist_to_disk {
            let account_user_id = expected_account_user_id.clone();
            let account_epoch = expected_account_epoch.clone();
            let store_key = key.clone();
            let _ = RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                        return;
                    }
                    CleverImageDiskCache::store_data(&data, &store_key, account_user_id.as_deref(), Some(invalidation_token));
                })
                .await;
        } else {
            let account_user_id = expected_account_user_id.clone();
            let account_epoch = expected_account_epoch.clone();
            let _ = RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                        return;
                    }
                    CleverImageDiskCache::perform_if_token_is_current(account_user_id.as_deref(), invalidation_token, || {
                        URLCache::shared().store_cached_response(CachedURLResponse::new(response, data.to_vec()), &request);
                    });
                })
                .await;
        }
        Some(image)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn memory_image(
        url_string: &str,
        persist_to_disk: bool,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        maximum_pixel_dimension: Option<i64>,
        cache_policy: CleverImageCachePolicy,
        expected_invalidation_token: Option<u64>,
        schedules_promotion: bool,
    ) -> Option<CleverImage> {
        if cache_policy == CleverImageCachePolicy::PrivateEphemeral {
            return None;
        }
        let key = CleverImageDiskCache::normalized_cache_key(url_string);

        if let Some(maximum_pixel_dimension) = maximum_pixel_dimension {
            if let Some(render_hit) =
                CleverImageRenderMemoryCache::image(&key, maximum_pixel_dimension, account_user_id, account_epoch)
            {
                return Some(render_hit);
            }
            let full_size = MediaViewerImageMemoryCache::memory_only(&key, account_user_id)?;
            if cache_policy == CleverImageCachePolicy::MemoryOnly {
                return Some(full_size);
            }
            if Self::pixel_maximum(&full_size) > maximum_pixel_dimension {
                return None;
            }
            CleverImageRenderMemoryCache::store(
                Arc::clone(&full_size),
                &key,
                maximum_pixel_dimension,
                account_user_id,
                account_epoch,
            );
            return Some(full_size);
        }

        let memory_hit = MediaViewerImageMemoryCache::memory_only(&key, account_user_id)?;
        if schedules_promotion && persist_to_disk && cache_policy == CleverImageCachePolicy::Standard {
            Self::schedule_disk_promotion(
                Arc::clone(&memory_hit),
                &key,
                account_user_id,
                account_epoch,
                expected_invalidation_token,
            );
        }
        Some(memory_hit)
    }

    #[allow(clippy::too_many_arguments)]
    async fn load_cached_image_bounded(
        url_string: String,
        persist_to_disk: bool,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        maximum_pixel_dimension: Option<i64>,
        cache_policy: CleverImageCachePolicy,
        expected_invalidation_token: Option<u64>,
        lane: CleverImageWorkLane,
    ) -> Option<CleverImage> {
        if cache_policy == CleverImageCachePolicy::PrivateEphemeral {
            return None;
        }
        let _permit = BITMAP_WORK_LIMITER.acquire(lane).await?;
        RiseAccountRuntime::current()
            .spawn_blocking(move || {
                if let Some(maximum_pixel_dimension) = maximum_pixel_dimension {
                    return Self::image_for_render(
                        &url_string,
                        persist_to_disk,
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                        maximum_pixel_dimension,
                        cache_policy == CleverImageCachePolicy::Standard,
                        expected_invalidation_token,
                    );
                }
                Self::image_from_memory_or_disk(
                    &url_string,
                    persist_to_disk,
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                    cache_policy,
                    expected_invalidation_token,
                )
            })
            .await
            .ok()
            .flatten()
    }

    fn image_for_render(
        url_string: &str,
        persist_to_disk: bool,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        maximum_pixel_dimension: i64,
        allow_persistent_reads: bool,
        expected_invalidation_token: Option<u64>,
    ) -> Option<CleverImage> {
        let key = CleverImageDiskCache::normalized_cache_key(url_string);
        if let Some(cached) = CleverImageRenderMemoryCache::image(&key, maximum_pixel_dimension, account_user_id, account_epoch) {
            return Some(cached);
        }
        let full_size = MediaViewerImageMemoryCache::memory_only(&key, account_user_id);
        if let Some(full_size) = &full_size
            && Self::pixel_maximum(full_size) <= maximum_pixel_dimension
        {
            CleverImageRenderMemoryCache::store(
                Arc::clone(full_size),
                &key,
                maximum_pixel_dimension,
                account_user_id,
                account_epoch,
            );
            return Some(Arc::clone(full_size));
        }

        let data = if allow_persistent_reads && persist_to_disk {
            CleverImageDiskCache::image_data(&key, account_user_id, expected_invalidation_token)
                .or_else(|| MediaViewerBitmapLoader::data_from_url_cache_only(&key))
        } else if allow_persistent_reads {
            MediaViewerBitmapLoader::data_from_url_cache_only(&key)
        } else {
            None
        };
        let Some(data) = data else {
            return full_size;
        };
        let Some(image) = CleverImageBitmapDecoder::decode(&data, Some(maximum_pixel_dimension)) else {
            return full_size;
        };
        CleverImageRenderMemoryCache::store(
            Arc::clone(&image),
            &key,
            maximum_pixel_dimension,
            account_user_id,
            account_epoch,
        );
        Some(image)
    }

    fn pixel_maximum(image: &RenderImage) -> i64 {
        let size = image.size(0);
        i64::from(size.width.0.max(size.height.0))
    }
}

struct CleverImageTransferAbandonGuard {
    transfer_key: Option<String>,
}

impl Drop for CleverImageTransferAbandonGuard {
    fn drop(&mut self) {
        let Some(cache_key) = self.transfer_key.take() else {
            return;
        };
        RiseAccountRuntime::current().spawn(async move {
            RiseMediaTransferHub::shared().abandon(cache_key).await;
        });
    }
}
