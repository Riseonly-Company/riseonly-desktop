use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, SystemTime};

use futures::FutureExt;
use futures::channel::oneshot;
use gpui::{AsyncApp, Pixels, RenderImage, Size};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_cache::NSCache;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::AppPaths;
use rise_platform::file_manager::{DirectoryEnumerationOptions, FileAttributeType, FileManager};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai_remote_media_url_hooks::sai_remote_media_url_optional_normalized;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, MediaPrefetchEnvironmentSnapshot,
};
use crate::core::ui::lottie_sticker::lottie_sticker_loader::{LottieStickerContainer, LottieStickerRenderSource};
use crate::core::ui::lottie_sticker::rise_lottie_frame_cache::{
    RiseLottieFrameCache, RiseLottieFrameSequence, RiseLottieRasterPolicy, RiseLottieSequenceKey,
};

pub const RISE_LOTTIE_FIRST_FRAME_CACHE_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("riseonly.lottie.first-frame-cache-did-change");

/// callback the preparation re checks between every expensive step
type RiseLottieIsNeeded = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Debug, Default)]
pub struct RiseLottiePreparationCancellation {
    cancelled: AtomicBool,
}

impl RiseLottiePreparationCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

struct RiseLottiePreparationCancelOnDrop {
    cancellation: Arc<RiseLottiePreparationCancellation>,
    finished: bool,
}

impl Drop for RiseLottiePreparationCancelOnDrop {
    fn drop(&mut self) {
        if !self.finished {
            self.cancellation.cancel();
        }
    }
}

pub enum RiseLottieImageVisibility {}

impl RiseLottieImageVisibility {
    /// true when any pixel is more than barely transparent, an empty frame is never a poster
    pub fn has_visible_alpha(image: &RenderImage) -> bool {
        let size = image.size(0);
        if size.width.0 <= 0 || size.height.0 <= 0 {
            return false;
        }
        let Some(pixels) = image.as_bytes(0) else {
            return false;
        };
        pixels.iter().skip(3).step_by(4).any(|alpha| *alpha > 1)
    }
}

enum RiseLottieAirbnbPosterRenderer {}

impl RiseLottieAirbnbPosterRenderer {
    async fn render(
        json_data: Arc<[u8]>,
        cache_key: String,
        point_size: Size<Pixels>,
        is_needed: RiseLottieIsNeeded,
    ) -> Option<CleverImage> {
        if !is_needed() {
            return None;
        }
        let dimension = RiseLottieRasterPolicy::dimension(Self::point_side(point_size));
        RiseAccountRuntime::current()
            .spawn_blocking(move || Self::render_blocking(&json_data, &cache_key, dimension, &is_needed))
            .await
            .ok()
            .flatten()
    }

    fn point_side(point_size: Size<Pixels>) -> f32 {
        f32::from(point_size.width).max(f32::from(point_size.height))
    }

    #[cfg(feature = "rlottie")]
    fn render_blocking(
        json_data: &[u8],
        cache_key: &str,
        dimension: i64,
        is_needed: &RiseLottieIsNeeded,
    ) -> Option<CleverImage> {
        use rise_media::lottie::rlottie_backend::RlottieRasterizer;

        if !is_needed() {
            return None;
        }
        let side = usize::try_from(dimension).ok().filter(|side| *side > 0)?;
        let side_u32 = u32::try_from(side).ok()?;
        let mut rasterizer = RlottieRasterizer::open(json_data, cache_key).ok()?;
        let frame_count = rasterizer.frame_count();
        let span = frame_count.saturating_sub(1) as f64;
        let candidates = [0.0, span * 0.25, span * 0.5, span * 0.75];
        let mut pixels = vec![0u32; side * side];
        for candidate in candidates {
            if !is_needed() {
                return None;
            }
            rasterizer.render(candidate.floor() as usize, side_u32, &mut pixels);
            let image = Self::image(&pixels, side_u32)?;
            if RiseLottieImageVisibility::has_visible_alpha(&image) {
                return Some(image);
            }
        }
        None
    }

    #[cfg(not(feature = "rlottie"))]
    fn render_blocking(
        _json_data: &[u8],
        _cache_key: &str,
        _dimension: i64,
        _is_needed: &RiseLottieIsNeeded,
    ) -> Option<CleverImage> {
        None
    }

    #[cfg_attr(not(feature = "rlottie"), allow(dead_code))]
    fn image(pixels: &[u32], side: u32) -> Option<CleverImage> {
        let mut bytes = Vec::with_capacity(pixels.len() * 4);
        for pixel in pixels {
            let [blue, green, red, alpha] = pixel.to_le_bytes();
            bytes.extend_from_slice(&[red, green, blue, alpha]);
        }
        let rgba = RgbaImage::from_raw(side, side, bytes)?;
        Some(CleverImageBitmapDecoder::render_image(rgba))
    }
}

pub enum RiseLottieFirstFrameCache {}

static MEMORY: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(usize::try_from(RiseLottieFirstFrameCache::memory_count_limit()).unwrap_or(0));
    cache.set_total_cost_limit(usize::try_from(RiseLottieFirstFrameCache::memory_cost_limit()).unwrap_or(0));
    cache
});
static LATEST_MEMORY: LazyLock<Mutex<HashMap<String, Weak<RenderImage>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static DISK_QUEUE: Mutex<Option<Duration>> = Mutex::new(None);
static DISK_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::compiled().canon())
        .ok()
        .map(|paths| paths.cache_dir().join("LottieStickerPosters"))
});
static HAS_PENDING_NOTIFICATION: AtomicBool = AtomicBool::new(false);
static NORMALIZED_URLS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

impl RiseLottieFirstFrameCache {
    const LATEST_MEMORY_LIMIT: usize = 384;
    const NORMALIZED_URLS_LIMIT: usize = 2_048;
    const PRUNE_INTERVAL: Duration = Duration::from_secs(60);

    fn maximum_disk_bytes() -> i64 {
        RiseLimits::client_value("lottie_first_frame_disk_cache_bytes")
    }

    fn maximum_disk_entries() -> i64 {
        RiseLimits::client_value("lottie_first_frame_disk_cache_entries")
    }

    fn memory_count_limit() -> i64 {
        RiseLimits::client_value("lottie_first_frame_memory_cache_entries")
    }

    fn memory_cost_limit() -> i64 {
        RiseLimits::client_value("lottie_first_frame_memory_cache_bytes")
    }

    pub fn memory_image(
        url_string: &str,
        point_size: Size<Pixels>,
        account_user_id: Option<&str>,
    ) -> Option<CleverImage> {
        MEMORY.object(&Self::memory_key(url_string, point_size, account_user_id))
    }

    pub fn latest_memory_image(url_string: &str, account_user_id: Option<&str>) -> Option<CleverImage> {
        let key = Self::latest_memory_key(url_string, account_user_id);
        let mut latest = LATEST_MEMORY.lock();
        let image = latest.get(&key)?.upgrade();
        if image.is_none() {
            latest.remove(&key);
        }
        image
    }

    pub async fn prepare(
        source: LottieStickerRenderSource,
        point_size: Size<Pixels>,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        speculative_environment: Option<MediaPrefetchEnvironmentSnapshot>,
    ) {
        let cancellation = Arc::new(RiseLottiePreparationCancellation::new());
        let mut on_cancel = RiseLottiePreparationCancelOnDrop {
            cancellation: Arc::clone(&cancellation),
            finished: false,
        };
        let is_needed: RiseLottieIsNeeded = {
            let account_user_id = account_user_id.clone();
            Arc::new(move || {
                !cancellation.is_cancelled()
                    && speculative_environment.is_none_or(|environment| environment.is_current())
                    && SaiAccountCacheScope::is_current_account_epoch(
                        account_user_id.as_deref(),
                        account_epoch.as_deref(),
                    )
            })
        };
        if is_needed() {
            Self::prepare_when_needed(source, point_size, account_user_id, is_needed).await;
        }
        on_cancel.finished = true;
    }

    async fn prepare_when_needed(
        source: LottieStickerRenderSource,
        point_size: Size<Pixels>,
        account_user_id: Option<String>,
        is_needed: RiseLottieIsNeeded,
    ) {
        if !is_needed() {
            return;
        }
        let key = Self::memory_key(&source.cache_key, point_size, account_user_id.as_deref());
        if MEMORY.object(&key).is_some() {
            return;
        }
        if let Some(disk_image) = Self::disk_image(key.clone(), account_user_id.clone(), Arc::clone(&is_needed)).await
            && is_needed()
            && RiseLottieImageVisibility::has_visible_alpha(&disk_image)
        {
            if !is_needed() {
                return;
            }
            Self::insert(disk_image, &key, &source.cache_key, account_user_id.as_deref());
            return;
        }
        if !is_needed() {
            return;
        }
        let airbnb_poster = || {
            RiseLottieAirbnbPosterRenderer::render(
                Arc::from(&source.json_data[..]),
                source.cache_key.clone(),
                point_size,
                Arc::clone(&is_needed),
            )
        };
        let prepared_image = match source.container {
            LottieStickerContainer::DotLottie => airbnb_poster().await,
            LottieStickerContainer::Ros | LottieStickerContainer::TgsLegacy | LottieStickerContainer::Json => {
                match Self::rlottie_poster(&source, point_size, Arc::clone(&is_needed)).await {
                    Some(image) => Some(image),
                    None => airbnb_poster().await,
                }
            }
        };
        let Some(prepared_image) = prepared_image.filter(|_| is_needed()) else {
            return;
        };
        Self::insert(Arc::clone(&prepared_image), &key, &source.cache_key, account_user_id.as_deref());
        Self::persist(prepared_image, key, account_user_id, is_needed).await;
    }

    pub fn store(
        image: CleverImage,
        url_string: &str,
        point_size: Size<Pixels>,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) {
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch)
            || !RiseLottieImageVisibility::has_visible_alpha(&image)
        {
            return;
        }
        let key = Self::memory_key(url_string, point_size, account_user_id);
        Self::insert(Arc::clone(&image), &key, url_string, account_user_id);
        let account_user_id = account_user_id.map(str::to_owned);
        RiseAccountRuntime::current().spawn(async move {
            Self::persist(image, key, account_user_id, Arc::new(|| true)).await;
        });
    }

    pub fn reset_memory() {
        MEMORY.remove_all_objects();
        let mut latest = LATEST_MEMORY.lock();
        latest.clear();
        latest.shrink_to_fit();
    }

    pub fn storage_root_url() -> Option<PathBuf> {
        DISK_ROOT.clone()
    }

    pub fn clear_disk_cache() {
        Self::reset_memory();
        let _queue = DISK_QUEUE.lock();
        let Some(disk_root) = DISK_ROOT.as_deref() else {
            return;
        };
        let Ok(items) = FileManager.contents_of_directory_at(disk_root, DirectoryEnumerationOptions::default()) else {
            return;
        };
        for url in items {
            let _ = FileManager.remove_item(&url);
        }
    }

    fn insert(image: CleverImage, key: &str, url_string: &str, account_user_id: Option<&str>) {
        let size = image.size(0);
        let pixels = i64::from(size.width.0).saturating_mul(i64::from(size.height.0));
        let cost = usize::try_from(pixels.saturating_mul(4).max(1)).unwrap_or(usize::MAX);
        let weak = Arc::downgrade(&image);
        MEMORY.set_object_cost(image, key.to_owned(), cost);
        let latest_key = Self::latest_memory_key(url_string, account_user_id);
        {
            let mut latest = LATEST_MEMORY.lock();
            latest.insert(latest_key, weak);
            if latest.len() > Self::LATEST_MEMORY_LIMIT {
                latest.retain(|_, image| image.strong_count() > 0);
                let overflow = latest.len().saturating_sub(Self::LATEST_MEMORY_LIMIT);
                let dropped: Vec<String> = latest.keys().take(overflow).cloned().collect();
                for key in dropped {
                    latest.remove(&key);
                }
            }
        }
        Self::schedule_cache_did_change_notification();
    }

    fn schedule_cache_did_change_notification() {
        if HAS_PENDING_NOTIFICATION.swap(true, Ordering::AcqRel) {
            return;
        }
        let post = || {
            HAS_PENDING_NOTIFICATION.store(false, Ordering::Release);
            RiseAppEvents::default_center().post(RISE_LOTTIE_FIRST_FRAME_CACHE_DID_CHANGE);
        };
        match RiseEngineMainHop::installed() {
            Some(hop) => hop.post(move |_| post()),
            None => post(),
        }
    }

    /// normalizing parses the url twice and poster keys are recomputed on every visibility change
    /// so memoize per raw string
    fn normalized_url(url_string: &str) -> String {
        if let Some(hit) = NORMALIZED_URLS.lock().get(url_string) {
            return hit.clone();
        }
        let value = sai_remote_media_url_optional_normalized(Some(url_string))
            .unwrap_or_else(|| trimming_whitespaces_and_newlines(url_string).to_owned());
        let mut normalized_urls = NORMALIZED_URLS.lock();
        if normalized_urls.len() >= Self::NORMALIZED_URLS_LIMIT {
            normalized_urls.clear();
        }
        normalized_urls.insert(url_string.to_owned(), value.clone());
        value
    }

    fn memory_key(url_string: &str, point_size: Size<Pixels>, account_user_id: Option<&str>) -> String {
        let dimension = RiseLottieRasterPolicy::dimension(RiseLottieAirbnbPosterRenderer::point_side(point_size));
        format!(
            "aspect-fit-v2|{}|{}|{}",
            account_user_id.unwrap_or("anonymous"),
            Self::normalized_url(url_string),
            dimension
        )
    }

    fn latest_memory_key(url_string: &str, account_user_id: Option<&str>) -> String {
        format!("{}|{}", account_user_id.unwrap_or("anonymous"), Self::normalized_url(url_string))
    }

    async fn rlottie_poster(
        source: &LottieStickerRenderSource,
        point_size: Size<Pixels>,
        is_needed: RiseLottieIsNeeded,
    ) -> Option<CleverImage> {
        if !is_needed() {
            return None;
        }
        let key = RiseLottieSequenceKey {
            cache_key: source.cache_key.clone(),
            dimension: RiseLottieRasterPolicy::dimension(RiseLottieAirbnbPosterRenderer::point_side(point_size)),
            mirrored: false,
        };
        let cache = RiseLottieFrameCache::shared();
        cache.retain(&key);
        let _release = RiseLottieSequenceRelease { key: key.clone() };
        let sequence = Self::main_sequence(key.clone(), source.json_data.clone()).await?;
        let last = (sequence.frame_count - 1).max(0);
        let mut seen = HashSet::new();
        for index in [0, last / 4, last / 2] {
            if !seen.insert(index) {
                continue;
            }
            if !is_needed() {
                return None;
            }
            let (continuation, frame) = oneshot::channel::<Option<CleverImage>>();
            sequence.request_frame(
                index,
                Arc::clone(&is_needed),
                Some(Box::new(move |image| {
                    let _ = continuation.send(image);
                })),
            );
            let Some(candidate) = frame.await.ok().flatten() else {
                continue;
            };
            if RiseLottieImageVisibility::has_visible_alpha(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    async fn main_sequence(
        key: RiseLottieSequenceKey,
        json_data: Arc<[u8]>,
    ) -> Option<Arc<RiseLottieFrameSequence>> {
        if let Some(cached) = RiseLottieFrameCache::shared().cached_sequence(&key) {
            return Some(cached);
        }
        let hop = RiseEngineMainHop::installed()?;
        hop.run(move |cx: AsyncApp| {
            async move { Ok::<_, anyhow::Error>(RiseLottieFrameCache::shared().sequence(&key, json_data, &cx).await) }
                .boxed_local()
        })
        .await
        .ok()
        .flatten()
    }

    fn png_data(image: &RenderImage) -> Option<Vec<u8>> {
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        let mut bytes = image.as_bytes(0)?.to_vec();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let mut data = Vec::new();
        PngEncoder::new(&mut data)
            .write_image(&bytes, width, height, ExtendedColorType::Rgba8)
            .ok()?;
        Some(data)
    }

    async fn disk_image(
        key: String,
        account_user_id: Option<String>,
        is_needed: RiseLottieIsNeeded,
    ) -> Option<CleverImage> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let _queue = DISK_QUEUE.lock();
                if !is_needed() {
                    return None;
                }
                let url = Self::disk_url(&key, account_user_id.as_deref())?;
                let data = std::fs::read(url).ok()?;
                CleverImageBitmapDecoder::decode(&data, None)
            })
            .await
            .ok()
            .flatten()
    }

    async fn persist(
        image: CleverImage,
        key: String,
        account_user_id: Option<String>,
        is_needed: RiseLottieIsNeeded,
    ) {
        let _ = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let mut last_prune_at = DISK_QUEUE.lock();
                if !is_needed() {
                    return;
                }
                let Some(data) = Self::png_data(&image) else {
                    return;
                };
                if !is_needed() {
                    return;
                }
                let (Some(directory), Some(url)) = (
                    Self::account_directory(account_user_id.as_deref()),
                    Self::disk_url(&key, account_user_id.as_deref()),
                ) else {
                    return;
                };
                if FileManager.create_directory(&directory, true).is_err() || Self::write_atomically(&url, &data).is_err() {
                    return;
                }
                Self::prune_if_due(&directory, &mut last_prune_at);
            })
            .await;
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

    fn account_directory(account_user_id: Option<&str>) -> Option<PathBuf> {
        let account = account_user_id.unwrap_or("anonymous");
        Some(DISK_ROOT.as_deref()?.join(sha256_hex(account.as_bytes())))
    }

    fn disk_url(key: &str, account_user_id: Option<&str>) -> Option<PathBuf> {
        let digest = sha256_hex(key.as_bytes());
        Some(Self::account_directory(account_user_id)?.join(format!("{digest}.png")))
    }

    fn prune_if_due(directory: &Path, last_prune_at: &mut Option<Duration>) {
        let now = RiseClock::monotonic();
        if last_prune_at.is_some_and(|last| now.saturating_sub(last) <= Self::PRUNE_INTERVAL) {
            return;
        }
        *last_prune_at = Some(now);
        Self::prune(directory);
    }

    fn prune(directory: &Path) {
        let mut entries: Vec<(PathBuf, i64, Option<SystemTime>)> = FileManager
            .contents_of_directory_at(directory, DirectoryEnumerationOptions::default())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|url| {
                let values = FileManager.attributes_of_item(&url).ok()?;
                if values.r#type != FileAttributeType::TypeRegular {
                    return None;
                }
                Some((url, values.size, values.modification_date))
            })
            .collect();
        entries.sort_by(|left, right| left.2.cmp(&right.2));
        let mut total_bytes: i64 = entries.iter().map(|entry| entry.1).sum();
        let maximum_disk_entries = usize::try_from(Self::maximum_disk_entries()).unwrap_or(0);
        let maximum_disk_bytes = Self::maximum_disk_bytes();
        let mut remaining = entries.into_iter();
        let mut count = remaining.len();
        while count > maximum_disk_entries || total_bytes > maximum_disk_bytes {
            let Some(entry) = remaining.next() else {
                break;
            };
            let _ = FileManager.remove_item(&entry.0);
            total_bytes -= entry.1;
            count -= 1;
        }
    }
}

struct RiseLottieSequenceRelease {
    key: RiseLottieSequenceKey,
}

impl Drop for RiseLottieSequenceRelease {
    fn drop(&mut self) {
        RiseLottieFrameCache::shared().release(&self.key);
    }
}
