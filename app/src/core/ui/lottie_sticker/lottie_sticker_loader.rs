use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_json_serialization::JSONSerialization;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::foundation::ns_cache::NSCache;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::core::engine_bridge::network_transport_config::NetworkClientSlot;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode, URLRequest};
use rise_media::lottie::lottie_package::{DotLottieCache, DotLottieFile, LottieAnimation};
use rise_media::decode_guard::decode_guard::inflate_lottie;
use rise_media::lottie::zlib::{crc32, inflate_raw};
use rise_platform::AppPaths;
use rise_platform::file_manager::{CocoaError, CocoaErrorCode, FileAttributeType, FileManager};
use serde_json::{Map, Value};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::sai_remote_media_url_hooks::sai_remote_media_url_optional_normalized;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDownloadRetryPolicy, RemoteMediaHttpResponse, RemoteMediaTransferError,
    RiseMediaRedirectSessionDelegate,
};

const KEY_LAYERS: &str = "layers";
const KEY_FONTS: &str = "fonts";
const KEY_CHARS: &str = "chars";
const KEY_ASSETS: &str = "assets";
const KEY_LAYER_TYPE: &str = "ty";
const REQUIRED_NUMBER_KEYS: [&str; 5] = ["fr", "ip", "op", "w", "h"];

const ACCEPT_HEADER: &str = "Accept";
const CACHE_CONTROL_HEADER: &str = "Cache-Control";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const ACCEPTED_CONTENT_TYPES: &str = "application/x-rosticker, application/x-tgsticker, application/json, application/zip, application/gzip, application/octet-stream";
const NO_STORE: &str = "no-store";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

const STORAGE_DIRECTORY_NAME: &str = "LottieStickers";
const INCOMING_PREFIX: &str = ".incoming-";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LottieStickerContainer {
    Json,
    DotLottie,
    Ros,
    TgsLegacy,
}

impl LottieStickerContainer {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::DotLottie => "lottie",
            Self::Ros => "ros",
            Self::TgsLegacy => "tgs",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LottieStickerRenderSource {
    pub json_data: Arc<[u8]>,
    pub cache_key: String,
    pub container: LottieStickerContainer,
    pub animation: LottieAnimation,
}

#[derive(Clone, Debug)]
pub enum LottieStickerPayload {
    DotLottie(DotLottieFile),
    Animation(LottieStickerRenderSource),
}

#[derive(Clone)]
struct LottieStickerRenderSourceCacheEntry {
    json_data: Arc<[u8]>,
    container: LottieStickerContainer,
    animation: LottieAnimation,
}

struct LottieStickerRenderSourceCache {
    values: NSCache<String, LottieStickerRenderSourceCacheEntry>,
}

impl LottieStickerRenderSourceCache {
    const PARSED_GRAPH_COST_MULTIPLIER: usize = 10;

    fn shared() -> &'static LottieStickerRenderSourceCache {
        static SHARED: LazyLock<LottieStickerRenderSourceCache> = LazyLock::new(|| {
            let values = NSCache::new();
            values.set_count_limit(
                usize::try_from(RiseLimits::client_value("lottie_render_source_cache_entries")).unwrap_or(0),
            );
            values.set_total_cost_limit(
                usize::try_from(RiseLimits::client_value("lottie_render_source_cache_bytes")).unwrap_or(0),
            );
            LottieStickerRenderSourceCache { values }
        });
        &SHARED
    }

    fn source(&self, cache_key: &str) -> Option<LottieStickerRenderSource> {
        let value = self.values.object(&cache_key.to_owned())?;
        Some(LottieStickerRenderSource {
            json_data: value.json_data,
            cache_key: cache_key.to_owned(),
            container: value.container,
            animation: value.animation,
        })
    }

    fn insert(
        &self,
        data: Arc<[u8]>,
        cache_key: &str,
        container: LottieStickerContainer,
        animation: LottieAnimation,
    ) {
        let cost = data.len().saturating_mul(Self::PARSED_GRAPH_COST_MULTIPLIER);
        self.values.set_object_cost(
            LottieStickerRenderSourceCacheEntry {
                json_data: data,
                container,
                animation,
            },
            cache_key.to_owned(),
            cost,
        );
    }

    fn remove_all(&self) {
        self.values.remove_all_objects();
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum LottieStickerLoaderError {
    #[error("invalid sticker URL")]
    InvalidURL,
    #[error("invalid sticker payload")]
    InvalidPayload,
    #[error(transparent)]
    Transfer(RemoteMediaTransferError),
}

impl LottieStickerLoaderError {
    fn cancelled() -> Self {
        Self::Transfer(RemoteMediaTransferError::Cancelled(CancellationError))
    }

    fn file(error: std::io::Error, path: &Path) -> Self {
        Self::Transfer(RemoteMediaTransferError::File(CocoaError {
            code: CocoaErrorCode::FileReadUnknown,
            path: Some(path.to_path_buf()),
            underlying: Some(Arc::new(error)),
        }))
    }

    fn is_cancellation(&self) -> bool {
        matches!(self, Self::Transfer(RemoteMediaTransferError::Cancelled(_)))
    }
}

type LottieStickerPayloadResult = Result<LottieStickerPayload, LottieStickerLoaderError>;
type LottieStickerLoadTask = Shared<BoxFuture<'static, LottieStickerPayloadResult>>;

struct InflightEntry {
    id: u64,
    task: LottieStickerLoadTask,
    abort: tokio::task::AbortHandle,
    waiter_ids: HashSet<u64>,
}

#[derive(Default)]
struct LottieStickerLoaderState {
    inflight: HashMap<String, InflightEntry>,
    next_entry_id: u64,
    next_waiter_id: u64,
}

struct InflightLease<'a> {
    loader: &'a LottieStickerLoader,
    key: String,
    entry_id: u64,
    waiter_id: u64,
    finished: bool,
}

impl Drop for InflightLease<'_> {
    fn drop(&mut self) {
        self.loader
            .release(&self.key, self.entry_id, self.waiter_id, !self.finished);
    }
}

static DISK_LOCK: Mutex<()> = Mutex::new(());

static DOWNLOAD_SESSION: NetworkClientSlot = NetworkClientSlot::without_redirects();

/// temp file of the download, removed once the load is done with it
struct LottieStickerTemporaryFile {
    path: PathBuf,
}

impl Drop for LottieStickerTemporaryFile {
    fn drop(&mut self) {
        FileManager.remove_item(&self.path).ok();
    }
}

pub struct LottieStickerLoader {
    state: RiseIsolated<LottieStickerLoaderState>,
    disk_root: Option<PathBuf>,
}

impl LottieStickerLoader {
    pub fn shared() -> &'static LottieStickerLoader {
        static SHARED: LazyLock<LottieStickerLoader> = LazyLock::new(|| LottieStickerLoader {
            state: RiseIsolated::new(LottieStickerLoaderState::default()),
            disk_root: LottieStickerLoader::storage_directory_url(),
        });
        &SHARED
    }

    fn maximum_entry_count() -> usize {
        usize::try_from(RiseLimits::client_value("lottie_source_disk_cache_entries")).unwrap_or(0)
    }

    fn maximum_total_bytes() -> i64 {
        RiseLimits::client_value("lottie_source_disk_cache_bytes")
    }

    fn maximum_file_bytes() -> i64 {
        RiseLimits::value("sticker_image_source")
    }

    fn maximum_file_byte_count() -> usize {
        usize::try_from(Self::maximum_file_bytes()).unwrap_or(0)
    }

    pub fn cached_payload(url_string: &str) -> Option<LottieStickerPayload> {
        let key = Self::normalized(url_string);
        if key.is_empty() {
            return None;
        }
        LottieStickerRenderSourceCache::shared()
            .source(&key)
            .map(LottieStickerPayload::Animation)
    }

    pub async fn payload(&self, url_string: &str) -> LottieStickerPayloadResult {
        let key = Self::normalized(url_string);
        let url = (!key.is_empty())
            .then(|| Url::from_string(&key))
            .flatten()
            .ok_or(LottieStickerLoaderError::InvalidURL)?;
        let scheme = url.scheme().map(str::to_lowercase);
        let is_remote = matches!(scheme.as_deref(), Some("http" | "https"));
        let is_local = if url.is_file_url() {
            let path = url.to_file_path().ok_or(LottieStickerLoaderError::InvalidURL)?;
            let maximum = Self::maximum_file_bytes();
            Self::blocking(move || {
                Ok(FileManager.attributes_of_item(&path).is_ok_and(|attributes| {
                    attributes.r#type == FileAttributeType::TypeRegular
                        && attributes.size > 0
                        && attributes.size <= maximum
                }))
            })
            .await?
        } else {
            false
        };
        if !is_remote && !is_local {
            return Err(LottieStickerLoaderError::InvalidURL);
        }

        let (entry_id, waiter_id, task) = self.state.with(|state| {
            state.next_waiter_id = state.next_waiter_id.wrapping_add(1);
            let waiter_id = state.next_waiter_id;
            if let Some(existing) = state.inflight.get_mut(&key) {
                existing.waiter_ids.insert(waiter_id);
                return (existing.id, waiter_id, existing.task.clone());
            }
            state.next_entry_id = state.next_entry_id.wrapping_add(1);
            let entry_id = state.next_entry_id;
            let disk_root = self.disk_root.clone();
            let load_url = url.clone();
            let load_key = key.clone();
            let handle = RiseAccountRuntime::current().spawn(async move {
                if is_local {
                    let Some(path) = load_url.to_file_path() else {
                        return Err(LottieStickerLoaderError::InvalidURL);
                    };
                    return Self::blocking(move || Self::decode_payload(&path, &load_key)).await;
                }
                Self::load_uncached(load_url, load_key, disk_root).await
            });
            let abort = handle.abort_handle();
            let task: BoxFuture<'static, LottieStickerPayloadResult> = handle
                .map(|joined| joined.unwrap_or_else(|_| Err(LottieStickerLoaderError::cancelled())))
                .boxed();
            let task = task.shared();
            state.inflight.insert(
                key.clone(),
                InflightEntry {
                    id: entry_id,
                    task: task.clone(),
                    abort,
                    waiter_ids: HashSet::from([waiter_id]),
                },
            );
            (entry_id, waiter_id, task)
        });

        let mut lease = InflightLease {
            loader: self,
            key,
            entry_id,
            waiter_id,
            finished: false,
        };
        let payload = task.await;
        lease.finished = true;
        payload
    }

    pub async fn reset_active_loads(&self) {
        self.reset_active_loads_now();
    }

    fn reset_active_loads_now(&self) {
        self.state.with(|state| {
            for entry in state.inflight.values() {
                entry.abort.abort();
            }
            state.inflight.clear();
        });
        LottieStickerRenderSourceCache::shared().remove_all();
    }

    pub async fn release_memory_caches(&self) {
        LottieStickerRenderSourceCache::shared().remove_all();
    }

    pub async fn cache_uploaded_sticker(&self, local_url: &Url, canonical_url: &str) {
        let key = Self::normalized(canonical_url);
        let Some(remote_url) = Url::from_string(&key) else {
            return;
        };
        let scheme = remote_url.scheme().map(str::to_lowercase).unwrap_or_default();
        if !local_url.is_file_url()
            || !matches!(scheme.as_str(), "http" | "https")
            || remote_url.path_extension().to_lowercase() != LottieStickerContainer::Ros.raw_value()
        {
            return;
        }
        let Some(disk_root) = self.disk_root.clone() else {
            return;
        };
        let Some(local_path) = local_url.to_file_path() else {
            return;
        };
        let local_key = Self::normalized(local_url.absolute_string());
        let _ = Self::blocking(move || {
            Self::seed_uploaded_sticker(&local_path, &local_key, &key, &disk_root);
            Ok(())
        })
        .await;
    }

    fn seed_uploaded_sticker(local_path: &Path, local_key: &str, key: &str, disk_root: &Path) {
        let Some(container) = Self::validated_container(local_path) else {
            return;
        };
        if container == LottieStickerContainer::DotLottie {
            return;
        }
        let (json, animation) = match LottieStickerRenderSourceCache::shared().source(local_key) {
            Some(cached) => (cached.json_data, cached.animation),
            None => {
                let Ok(source) = std::fs::read(local_path) else {
                    return;
                };
                let Ok(json) = Self::normalized_json_data(&source, container) else {
                    return;
                };
                let Ok(animation) = LottieAnimation::from_data(&json) else {
                    return;
                };
                (Arc::clone(animation.json_data()), animation)
            }
        };
        LottieStickerRenderSourceCache::shared().insert(
            Arc::clone(&json),
            key,
            LottieStickerContainer::Json,
            animation,
        );
        let _guard = DISK_LOCK.lock();
        if Self::prepare_directory_locked(disk_root).is_err() {
            return;
        }
        let destination = Self::disk_file_url_at(disk_root, key, LottieStickerContainer::Json);
        if Self::write_atomically(&destination, &json).is_err() {
            return;
        }
        Self::prune_locked(disk_root, Some(&destination));
    }

    pub fn storage_directory_url() -> Option<PathBuf> {
        AppPaths::resolve(AppEnvironment::current().canon())
            .ok()
            .map(|paths| paths.cache_dir().join(STORAGE_DIRECTORY_NAME))
    }

    pub async fn clear_disk_cache(&self) {
        self.reset_active_loads_now();
        let Some(directory) = Self::storage_directory_url() else {
            return;
        };
        let _ = Self::blocking(move || {
            let _guard = DISK_LOCK.lock();
            let items = FileManager.contents_of_directory_at_path(&directory).unwrap_or_default();
            for name in items {
                FileManager.remove_item(&directory.join(name)).ok();
            }
            Ok(())
        })
        .await;
    }

    pub async fn disk_file_url(&self, url_string: &str) -> Option<PathBuf> {
        let key = Self::normalized(url_string);
        if key.is_empty() {
            return None;
        }
        let disk_root = self.disk_root.clone()?;
        Self::blocking(move || Ok(Self::existing_disk_file_url(&disk_root, &key)))
            .await
            .ok()
            .flatten()
    }

    pub async fn cached_payload_from_disk(&self, url_string: &str) -> Option<LottieStickerPayload> {
        let key = Self::normalized(url_string);
        if key.is_empty() {
            return None;
        }
        if let Some(source) = LottieStickerRenderSourceCache::shared().source(&key) {
            return Some(LottieStickerPayload::Animation(source));
        }
        let disk_root = self.disk_root.clone();
        Self::blocking(move || {
            if let Some(dot) = DotLottieCache::shared_cache().file(&key) {
                let archive = disk_root
                    .as_deref()
                    .and_then(|root| Self::existing_disk_file_url(root, &key));
                return Ok(Self::normalized_dot_lottie_payload(dot, &key, archive.as_deref()).ok());
            }
            let Some(disk_url) = disk_root
                .as_deref()
                .and_then(|root| Self::existing_disk_file_url(root, &key))
            else {
                return Ok(None);
            };
            let Ok(payload) = Self::decode_payload(&disk_url, &key) else {
                return Ok(None);
            };
            Self::touch(&disk_url);
            Ok(Some(payload))
        })
        .await
        .ok()
        .flatten()
    }

    async fn blocking<R: Send + 'static>(
        work: impl FnOnce() -> Result<R, LottieStickerLoaderError> + Send + 'static,
    ) -> Result<R, LottieStickerLoaderError> {
        RiseAccountRuntime::current()
            .spawn_blocking(work)
            .await
            .unwrap_or_else(|_| Err(LottieStickerLoaderError::cancelled()))
    }

    async fn load_uncached(
        url: Url,
        cache_key: String,
        disk_root: Option<PathBuf>,
    ) -> LottieStickerPayloadResult {
        if let Some(render_source) = LottieStickerRenderSourceCache::shared().source(&cache_key) {
            return Ok(LottieStickerPayload::Animation(render_source));
        }
        let cached_key = cache_key.clone();
        let cached_root = disk_root.clone();
        let cached = Self::blocking(move || {
            if let Some(dot) = DotLottieCache::shared_cache().file(&cached_key) {
                let archive = cached_root
                    .as_deref()
                    .and_then(|root| Self::existing_disk_file_url(root, &cached_key));
                return Self::normalized_dot_lottie_payload(dot, &cached_key, archive.as_deref())
                    .map(Some);
            }
            let Some(disk_url) = cached_root
                .as_deref()
                .and_then(|root| Self::existing_disk_file_url(root, &cached_key))
            else {
                return Ok(None);
            };
            match Self::decode_payload(&disk_url, &cached_key) {
                Ok(payload) => {
                    Self::touch(&disk_url);
                    Ok(Some(payload))
                }
                Err(error) if error.is_cancellation() => Err(error),
                Err(_) => {
                    FileManager.remove_item(&disk_url).ok();
                    Ok(None)
                }
            }
        })
        .await?;
        if let Some(payload) = cached {
            return Ok(payload);
        }

        let mut request = URLRequest::new(url);
        request.timeout_interval = DOWNLOAD_TIMEOUT;
        request.set_value(ACCEPTED_CONTENT_TYPES, ACCEPT_HEADER);
        request.set_value(NO_STORE, CACHE_CONTROL_HEADER);
        let maximum_file_bytes = Self::maximum_file_bytes();
        let (temporary, response) = RemoteMediaDownloadRetryPolicy::retrying(|| {
            Self::download(request.clone(), maximum_file_bytes)
        })
        .await
        .map_err(LottieStickerLoaderError::Transfer)?;
        let expected_content_length = response.expected_content_length();
        let is_acceptable = (200..300).contains(&response.status_code)
            && (expected_content_length < 0 || expected_content_length <= maximum_file_bytes)
            && Self::is_acceptable_content_type(&response);

        Self::blocking(move || {
            let Some(container) = is_acceptable
                .then(|| Self::validated_container(&temporary.path))
                .flatten()
            else {
                return Err(LottieStickerLoaderError::InvalidPayload);
            };
            let payload = Self::decode_payload(&temporary.path, &cache_key)?;
            if let Some(disk_root) = disk_root {
                Self::commit_download(&disk_root, &cache_key, container, &temporary.path);
            }
            drop(temporary);
            Ok(payload)
        })
        .await
    }

    async fn download(
        request: URLRequest,
        maximum_file_bytes: i64,
    ) -> Result<(LottieStickerTemporaryFile, RemoteMediaHttpResponse), RemoteMediaTransferError> {
        let Some(session) = DOWNLOAD_SESSION.current() else {
            return Err(URLError::new(URLErrorCode::SecureConnectionFailed).into());
        };
        let session = &session;
        let (mut response, url) =
            RiseMediaRedirectSessionDelegate::send(session, &request, None, |_, _| {}).await?;
        let head = RemoteMediaHttpResponse::from_reqwest(&response, url);
        let temporary = LottieStickerTemporaryFile {
            path: rise_platform::paths::temporary_directory().join(format!(
                "riseonly-lottie-{}.download",
                Uuid::new_v4().uuid_string().to_lowercase()
            )),
        };
        let write_error = |error: std::io::Error, path: &Path| {
            RemoteMediaTransferError::File(CocoaError {
                code: CocoaErrorCode::FileWriteUnknown,
                path: Some(path.to_path_buf()),
                underlying: Some(Arc::new(error)),
            })
        };
        let mut file = std::fs::File::create(&temporary.path)
            .map_err(|error| write_error(error, &temporary.path))?;
        let mut written: i64 = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| RemoteMediaTransferError::url_error(&error))?
        {
            file.write_all(&chunk)
                .map_err(|error| write_error(error, &temporary.path))?;
            written = written.saturating_add(i64::try_from(chunk.len()).unwrap_or(i64::MAX));
            if written > maximum_file_bytes {
                break;
            }
        }
        file.flush().map_err(|error| write_error(error, &temporary.path))?;
        Ok((temporary, head))
    }

    fn commit_download(
        disk_root: &Path,
        cache_key: &str,
        container: LottieStickerContainer,
        temporary: &Path,
    ) {
        let _guard = DISK_LOCK.lock();
        let disk_url = Self::disk_file_url_at(disk_root, cache_key, container);
        let staging = disk_root.join(format!(
            "{INCOMING_PREFIX}{}",
            Uuid::new_v4().uuid_string().to_lowercase()
        ));
        FileManager.remove_item(&staging).ok();
        let committed = Self::prepare_directory_locked(disk_root).is_ok()
            && FileManager.copy_item(temporary, &staging).is_ok()
            && if FileManager.file_exists(&disk_url) {
                std::fs::rename(&staging, &disk_url).is_ok()
            } else {
                FileManager.move_item(&staging, &disk_url).is_ok()
            };
        if !committed {
            FileManager.remove_item(&staging).ok();
            return;
        }
        Self::remove_alternate_files(disk_root, cache_key, &disk_url);
        Self::touch(&disk_url);
        Self::prune_locked(disk_root, Some(&disk_url));
    }

    fn decode_payload(disk_url: &Path, cache_key: &str) -> LottieStickerPayloadResult {
        let container =
            Self::validated_container(disk_url).ok_or(LottieStickerLoaderError::InvalidPayload)?;
        if container == LottieStickerContainer::DotLottie {
            let dot = DotLottieFile::loaded_from(disk_url)
                .map_err(|_| LottieStickerLoaderError::InvalidPayload)?;
            DotLottieCache::shared_cache().set_file(dot.clone(), cache_key);
            return Self::normalized_dot_lottie_payload(dot, cache_key, Some(disk_url));
        }

        let source =
            std::fs::read(disk_url).map_err(|error| LottieStickerLoaderError::file(error, disk_url))?;
        let json_data = Self::normalized_json_data(&source, container)?;
        let animation = LottieAnimation::from_data(&json_data)
            .map_err(|_| LottieStickerLoaderError::InvalidPayload)?;
        let json_data = Arc::clone(animation.json_data());
        LottieStickerRenderSourceCache::shared().insert(
            Arc::clone(&json_data),
            cache_key,
            container,
            animation.clone(),
        );
        Ok(LottieStickerPayload::Animation(LottieStickerRenderSource {
            json_data,
            cache_key: cache_key.to_owned(),
            container,
            animation,
        }))
    }

    /// old server archives often wrap one plain vector animation
    /// normalize those once off the main thread so they share frames with json and ros stickers
    /// archives with assets, text compositions or speed changes are left alone
    fn normalized_dot_lottie_payload(
        file: DotLottieFile,
        cache_key: &str,
        archive_url: Option<&Path>,
    ) -> LottieStickerPayloadResult {
        let [item] = &*file.animations else {
            return Ok(LottieStickerPayload::DotLottie(file));
        };
        let normalized = (item.configuration.speed == 1.0)
            .then_some(archive_url)
            .flatten()
            .and_then(|archive_url| std::fs::read(archive_url).ok())
            .and_then(|archive| {
                RiseLottieArchiveJSON::read(
                    &item.configuration.id,
                    &archive,
                    Self::maximum_file_byte_count(),
                )
            })
            .filter(|json_data| {
                JSONSerialization::json_object(json_data)
                    .as_ref()
                    .and_then(Value::as_object)
                    .is_some_and(Self::supports_standalone_vector_rasterization)
            });
        let Some(json_data) = normalized else {
            return Ok(LottieStickerPayload::DotLottie(file));
        };
        let json_data: Arc<[u8]> = Arc::from(json_data);
        let animation = item.animation.clone();
        LottieStickerRenderSourceCache::shared().insert(
            Arc::clone(&json_data),
            cache_key,
            LottieStickerContainer::Json,
            animation.clone(),
        );
        Ok(LottieStickerPayload::Animation(LottieStickerRenderSource {
            json_data,
            cache_key: cache_key.to_owned(),
            container: LottieStickerContainer::Json,
            animation,
        }))
    }

    pub fn supports_standalone_vector_rasterization(root: &Map<String, Value>) -> bool {
        let Some(layers) = root.get(KEY_LAYERS).and_then(Value::as_array) else {
            return false;
        };
        if layers.is_empty()
            || !layers.iter().all(Value::is_object)
            || root.contains_key(KEY_FONTS)
            || root.contains_key(KEY_CHARS)
        {
            return false;
        }
        if let Some(assets) = root.get(KEY_ASSETS)
            && !assets.as_array().is_some_and(Vec::is_empty)
        {
            return false;
        }
        layers.iter().all(|layer| {
            matches!(
                layer.get(KEY_LAYER_TYPE).and_then(Self::json_int),
                Some(3 | 4)
            )
        })
    }

    fn json_int(value: &Value) -> Option<i64> {
        value.as_i64().or_else(|| {
            value
                .as_f64()
                .filter(|number| number.fract() == 0.0 && number.abs() <= i64::MAX as f64)
                .map(|number| number as i64)
        })
    }

    fn existing_disk_file_url(disk_root: &Path, cache_key: &str) -> Option<PathBuf> {
        let _guard = DISK_LOCK.lock();
        for candidate in Self::disk_file_candidates(disk_root, cache_key) {
            let Some(container) = Self::validated_container(&candidate) else {
                if FileManager.file_exists(&candidate) {
                    FileManager.remove_item(&candidate).ok();
                }
                continue;
            };
            let canonical = Self::disk_file_url_at(disk_root, cache_key, container);
            if candidate != canonical {
                if Self::validated_container(&canonical) == Some(container) {
                    FileManager.remove_item(&candidate).ok();
                    Self::touch(&canonical);
                    return Some(canonical);
                }
                FileManager.remove_item(&canonical).ok();
                if FileManager.move_item(&candidate, &canonical).is_ok() {
                    Self::touch(&canonical);
                    return Some(canonical);
                }
                Self::touch(&candidate);
                return Some(candidate);
            }
            Self::touch(&candidate);
            return Some(candidate);
        }
        None
    }

    fn release(&self, key: &str, entry_id: u64, waiter_id: u64, cancel_task_when_last: bool) {
        self.state.with(|state| {
            let Some(entry) = state.inflight.get_mut(key) else {
                return;
            };
            if entry.id != entry_id || !entry.waiter_ids.remove(&waiter_id) {
                return;
            }
            if entry.waiter_ids.is_empty()
                && let Some(entry) = state.inflight.remove(key)
                && cancel_task_when_last
            {
                entry.abort.abort();
            }
        });
    }

    fn prepare_directory_locked(directory: &Path) -> Result<(), CocoaError> {
        FileManager.create_directory(directory, true)?;
        Self::prune_locked(directory, None);
        Ok(())
    }

    pub fn detected_container(data: &[u8]) -> Option<LottieStickerContainer> {
        if data.is_empty() {
            return None;
        }
        if data.starts_with(&[0x50, 0x4B]) {
            return Some(LottieStickerContainer::DotLottie);
        }
        if data.starts_with(&[0x1F, 0x8B]) {
            return Some(LottieStickerContainer::Ros);
        }
        let start = if data.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
        for byte in &data[start..] {
            match byte {
                0x09 | 0x0A | 0x0D | 0x20 => continue,
                0x7B => return Some(LottieStickerContainer::Json),
                _ => return None,
            }
        }
        None
    }

    pub fn normalized_json_data(
        data: &[u8],
        container: LottieStickerContainer,
    ) -> Result<Vec<u8>, LottieStickerLoaderError> {
        let result = match container {
            LottieStickerContainer::Json => data.to_vec(),
            LottieStickerContainer::Ros | LottieStickerContainer::TgsLegacy => {
                Self::inflated_gzip_data(data)?
            }
            LottieStickerContainer::DotLottie => {
                return Err(LottieStickerLoaderError::InvalidPayload);
            }
        };
        let fits = i64::try_from(result.len()).is_ok_and(|count| count <= Self::maximum_file_bytes());
        let is_animation = !result.is_empty()
            && fits
            && JSONSerialization::json_object(&result)
                .as_ref()
                .and_then(Value::as_object)
                .is_some_and(|root| {
                    root.get(KEY_LAYERS).is_some_and(Value::is_array)
                        && REQUIRED_NUMBER_KEYS
                            .iter()
                            .all(|key| root.get(*key).is_some_and(Self::is_ns_number))
                });
        if !is_animation {
            return Err(LottieStickerLoaderError::InvalidPayload);
        }
        Ok(result)
    }

    fn is_ns_number(value: &Value) -> bool {
        value.is_number() || value.is_boolean()
    }

    fn validated_container(url: &Path) -> Option<LottieStickerContainer> {
        let attributes = FileManager.attributes_of_item(url).ok()?;
        if attributes.r#type != FileAttributeType::TypeRegular
            || attributes.size <= 0
            || attributes.size > Self::maximum_file_bytes()
        {
            return None;
        }
        let data = std::fs::read(url).ok()?;
        let container = Self::detected_container(&data)?;
        if container == LottieStickerContainer::DotLottie {
            return Some(container);
        }
        Self::normalized_json_data(&data, container).ok()?;
        Some(container)
    }

    fn inflated_gzip_data(data: &[u8]) -> Result<Vec<u8>, LottieStickerLoaderError> {
        inflate_lottie(data).map_err(|_| LottieStickerLoaderError::InvalidPayload)
    }

    fn is_acceptable_content_type(response: &RemoteMediaHttpResponse) -> bool {
        let value = response
            .value(CONTENT_TYPE_HEADER)
            .map(str::to_lowercase)
            .unwrap_or_default();
        !value.contains("text/html") && !value.contains("application/xml") && !value.contains("text/xml")
    }

    fn prune_locked(directory: &Path, preserving: Option<&Path>) {
        Self::remove_stale_incoming_files(directory);
        let names = FileManager.contents_of_directory_at_path(directory).unwrap_or_default();
        let maximum_file_bytes = Self::maximum_file_bytes();
        let mut entries: Vec<(PathBuf, i64, SystemTime)> = names
            .into_iter()
            .filter_map(|name| {
                let url = directory.join(&name);
                if name.starts_with(INCOMING_PREFIX) {
                    FileManager.remove_item(&url).ok();
                    return None;
                }
                if name.starts_with('.') || preserving == Some(url.as_path()) {
                    return None;
                }
                let attributes = FileManager.attributes_of_item(&url).ok()?;
                if attributes.r#type != FileAttributeType::TypeRegular {
                    return None;
                }
                let bytes = attributes.size;
                if bytes <= 0 || bytes > maximum_file_bytes {
                    FileManager.remove_item(&url).ok();
                    return None;
                }
                Some((url, bytes, attributes.modification_date.unwrap_or(UNIX_EPOCH)))
            })
            .collect();
        let preserved_bytes = preserving
            .and_then(|path| FileManager.attributes_of_item(path).ok())
            .map_or(0, |attributes| attributes.size);
        let mut total_bytes = entries
            .iter()
            .fold(preserved_bytes, |total, entry| total.saturating_add(entry.1));
        let mut total_count = entries.len() + usize::from(preserving.is_some());
        entries.sort_by(|left, right| left.2.cmp(&right.2));
        let mut oldest_first = entries.into_iter();
        while total_count > Self::maximum_entry_count() || total_bytes > Self::maximum_total_bytes() {
            let Some(entry) = oldest_first.next() else {
                break;
            };
            if FileManager.remove_item(&entry.0).is_ok() {
                total_bytes = (total_bytes - entry.1).max(0);
                total_count -= 1;
            }
        }
    }

    pub fn remove_stale_incoming_files(directory: &Path) {
        let items = FileManager.contents_of_directory_at_path(directory).unwrap_or_default();
        for name in items.iter().filter(|name| name.starts_with(INCOMING_PREFIX)) {
            FileManager.remove_item(&directory.join(name)).ok();
        }
    }

    fn touch(url: &Path) {
        let seconds = RiseClock::now().time_interval_since_1970().max(0.0);
        let modified = UNIX_EPOCH + Duration::from_secs_f64(seconds);
        if let Ok(file) = std::fs::File::options().write(true).open(url) {
            file.set_modified(modified).ok();
        }
    }

    fn write_atomically(destination: &Path, data: &[u8]) -> std::io::Result<()> {
        let directory = destination.parent().unwrap_or(destination);
        let staging = directory.join(format!(
            "{INCOMING_PREFIX}{}",
            Uuid::new_v4().uuid_string().to_lowercase()
        ));
        let written = std::fs::write(&staging, data).and_then(|()| std::fs::rename(&staging, destination));
        if written.is_err() {
            FileManager.remove_item(&staging).ok();
        }
        written
    }

    fn normalized(url_string: &str) -> String {
        sai_remote_media_url_optional_normalized(Some(url_string))
            .unwrap_or_else(|| trimming_whitespaces_and_newlines(url_string).to_owned())
    }

    fn cache_digest(cache_key: &str) -> String {
        sha256_hex(cache_key.as_bytes())
    }

    fn disk_file_url_at(root: &Path, cache_key: &str, container: LottieStickerContainer) -> PathBuf {
        root.join(format!(
            "{}.{}",
            Self::cache_digest(cache_key),
            container.raw_value()
        ))
    }

    fn disk_file_candidates(root: &Path, cache_key: &str) -> [PathBuf; 4] {
        [
            LottieStickerContainer::Json,
            LottieStickerContainer::DotLottie,
            LottieStickerContainer::Ros,
            LottieStickerContainer::TgsLegacy,
        ]
        .map(|container| Self::disk_file_url_at(root, cache_key, container))
    }

    fn remove_alternate_files(root: &Path, cache_key: &str, preserving: &Path) {
        for candidate in Self::disk_file_candidates(root, cache_key) {
            if candidate != preserving {
                FileManager.remove_item(&candidate).ok();
            }
        }
    }
}

pub enum RiseLottieArchiveJSON {}

impl RiseLottieArchiveJSON {
    const LOCAL_FILE_HEADER_SIGNATURE: usize = 0x0403_4b50;
    const LOCAL_FILE_HEADER_BYTES: usize = 30;

    pub fn read(animation_id: &str, archive: &[u8], maximum_bytes: usize) -> Option<Vec<u8>> {
        if maximum_bytes == 0 || archive.len() > maximum_bytes {
            return None;
        }
        let bytes = archive;
        let uint16 = |offset: usize| usize::from(bytes[offset]) | usize::from(bytes[offset + 1]) << 8;
        let uint32 = |offset: usize| uint16(offset) | uint16(offset + 2) << 16;
        let animation_names = [
            format!("animations/{animation_id}.json"),
            format!("a/{animation_id}.json"),
        ];
        let mut offset = 0;
        while bytes.len() - offset >= Self::LOCAL_FILE_HEADER_BYTES {
            if uint32(offset) != Self::LOCAL_FILE_HEADER_SIGNATURE {
                return None;
            }
            let flags = uint16(offset + 6);
            let method = uint16(offset + 8);
            let checksum = u32::try_from(uint32(offset + 14)).ok()?;
            let compressed_size = uint32(offset + 18);
            let expanded_size = uint32(offset + 22);
            let name_size = uint16(offset + 26);
            let extra_size = uint16(offset + 28);
            let header_end = offset + Self::LOCAL_FILE_HEADER_BYTES;
            if flags & !0x0806 != 0
                || (method != 0 && method != 8)
                || compressed_size > maximum_bytes
                || expanded_size > maximum_bytes
                || name_size > bytes.len() - header_end
                || extra_size > bytes.len() - header_end - name_size
            {
                return None;
            }
            let payload_offset = header_end + name_size + extra_size;
            if compressed_size > bytes.len() - payload_offset {
                return None;
            }
            let name = std::str::from_utf8(&bytes[header_end..header_end + name_size]).ok();
            if name.is_some_and(|name| animation_names.iter().any(|candidate| candidate == name)) {
                if expanded_size == 0 {
                    return None;
                }
                let compressed = &bytes[payload_offset..payload_offset + compressed_size];
                let output = if method == 0 {
                    if compressed_size != expanded_size {
                        return None;
                    }
                    compressed.to_vec()
                } else {
                    inflate_raw(compressed, expanded_size)?
                };
                return (crc32(&output) == checksum).then_some(output);
            }
            offset = payload_offset + compressed_size;
        }
        None
    }
}
