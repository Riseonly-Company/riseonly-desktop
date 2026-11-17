use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::{Arc, LazyLock};

use gpui::{App, Global, Pixels, RenderImage, Size};
use image::imageops::{self, FilterType};
use image::{Frame, RgbaImage};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_cache::NSCache;
use rise_energy::rise_energy_saving::RiseEnergySavingStore;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_ui::swiftui::blur::gaussian_blur_render_image;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::renderer::shared_element_cover_hooks::SharedElementGridPreviewHooks;
use crate::core::shared_element_transition::renderer::shared_element_cover_rasterizer::SharedElementCoverRasterizer;
use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementCover, SharedElementCoverContent, SharedElementFlightContent,
};
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapLoader, CleverImageDiskCache, CleverImageRenderMemoryCache,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::shorts_media_prefetch::ShortsMediaPrefetchPolicy;

/// bitmap handed out by the bitmap resolver
#[derive(Clone)]
pub struct SharedElementBitmapResolverBitmap {
    pub image: CleverImage,
    pub aspect: f32,
    pub source: String,
}

impl SharedElementBitmapResolverBitmap {
    pub fn flight_content(&self) -> SharedElementFlightContent {
        SharedElementFlightContent::Bitmap(Arc::clone(&self.image), self.aspect)
    }
}

struct BackdropWaiter {
    is_current: Box<dyn Fn(&App) -> bool>,
    on_ready: Box<dyn FnOnce(CleverImage, &mut App)>,
}

/// waiters hold main thread closures so they live in an app global, not a static
#[derive(Default)]
struct SharedElementBitmapResolverWaiters {
    backdrop_waiters: HashMap<String, Vec<BackdropWaiter>>,
}

impl Global for SharedElementBitmapResolverWaiters {}

static BACKDROP_CACHE: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(24);
    cache
});

pub enum SharedElementBitmapResolver {}

impl SharedElementBitmapResolver {
    pub fn reset_account_state(cx: &mut App) {
        BACKDROP_CACHE.remove_all_objects();
        if cx.has_global::<SharedElementBitmapResolverWaiters>() {
            cx.global_mut::<SharedElementBitmapResolverWaiters>().backdrop_waiters.clear();
        }
    }

    /// content chain steps 1 to 3, memory image for each bucket and source
    /// then the full size viewer cache, then any live decode
    pub fn bitmap(sources: &[String], is_own: bool, buckets: &[Option<i64>]) -> Option<SharedElementBitmapResolverBitmap> {
        let candidates = Self::unique(
            sources
                .iter()
                .map(|source| trimming_whitespaces_and_newlines(source).to_owned())
                .filter(|source| !source.is_empty())
                .collect(),
        );
        if candidates.is_empty() {
            return None;
        }
        let user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let policy = PostMediaCachePolicy::image_policy(is_own);

        for bucket in Self::unique(buckets.to_vec()) {
            for source in &candidates {
                if let Some(image) = CleverImageBitmapLoader::memory_image(
                    source,
                    is_own,
                    user_id.as_deref(),
                    account_epoch.as_deref(),
                    bucket,
                    policy,
                    None,
                    false,
                ) && let Some(resolved) = Self::make_bitmap(image, source)
                {
                    return Some(resolved);
                }
            }
        }
        for source in &candidates {
            if let Some(image) = MediaViewerImageMemoryCache::memory_only(
                &RemoteMediaURL::display_source(source, None),
                user_id.as_deref(),
            ) && let Some(resolved) = Self::make_bitmap(image, source)
            {
                return Some(resolved);
            }
        }
        for source in &candidates {
            if let Some(image) = CleverImageRenderMemoryCache::latest_image(
                &CleverImageDiskCache::normalized_cache_key(source),
                user_id.as_deref(),
                account_epoch.as_deref(),
            ) && let Some(resolved) = Self::make_bitmap(image, source)
            {
                return Some(resolved);
            }
        }
        None
    }

    /// full size bitmap of one image url, same as the list row draws
    pub fn full_size(url_string: &str, is_own: bool) -> Option<SharedElementBitmapResolverBitmap> {
        Self::bitmap(&[url_string.to_owned()], is_own, &[None])
    }

    pub fn memory_bitmap(
        url_string: &str,
        persist_to_disk: bool,
        cache_policy: CleverImageCachePolicy,
        buckets: &[Option<i64>],
    ) -> Option<SharedElementBitmapResolverBitmap> {
        let source = trimming_whitespaces_and_newlines(url_string);
        if source.is_empty() {
            return None;
        }
        let user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        for bucket in Self::unique(buckets.to_vec()) {
            if let Some(image) = CleverImageBitmapLoader::memory_image(
                source,
                persist_to_disk,
                user_id.as_deref(),
                account_epoch.as_deref(),
                bucket,
                cache_policy,
                None,
                false,
            ) && let Some(resolved) = Self::make_bitmap(image, source)
            {
                return Some(resolved);
            }
        }
        if let Some(image) = CleverImageRenderMemoryCache::latest_image(
            &CleverImageDiskCache::normalized_cache_key(source),
            user_id.as_deref(),
            account_epoch.as_deref(),
        ) && let Some(resolved) = Self::make_bitmap(image, source)
        {
            return Some(resolved);
        }
        None
    }

    /// reel poster, looked up under the player cover key and the raw thumbnail key
    pub fn poster(post: &GetPostFeedResponse, cx: &mut App) -> Option<SharedElementBitmapResolverBitmap> {
        let thumbnail = post
            .video_thumbnail
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or_default()
            .to_owned();
        if thumbnail.is_empty() {
            return None;
        }
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        Self::bitmap(
            &[RemoteMediaURL::display_source(&thumbnail, Some(thumbnail.as_str())), thumbnail],
            is_own,
            &[Some(ShortsMediaPrefetchPolicy::POSTER_PIXEL_DIMENSION), Some(3_072)],
        )
    }

    /// decode bucket for a grid cell
    /// none until the profile grid installs its slot, then covers start at full size
    pub fn cell_bucket(cell_size: Size<Pixels>, display_scale: f32) -> Option<i64> {
        SharedElementGridPreviewHooks::profile_thumbnail_maximum_pixel_dimension(cell_size, display_scale)
    }

    /// source the grid cell draws, none for a text cell
    pub fn grid_cover_source(post: &GetPostFeedResponse) -> Option<String> {
        SharedElementGridPreviewHooks::profile_grid_preview_source(
            post.local_preview_url.as_deref(),
            post.images.first().map(String::as_str),
            post.video_thumbnail.as_deref(),
        )
    }

    pub fn grid_cover_identity(post: &GetPostFeedResponse) -> String {
        Self::grid_cover_source(post).unwrap_or_else(|| SharedElementCoverRasterizer::text_identity(&post.id))
    }

    /// grid cell picture, tries cell bucket then full size then any live decode
    /// text cells are rasterized
    pub fn grid_cover(
        post: &GetPostFeedResponse,
        cell_size: Size<Pixels>,
        display_scale: f32,
        epoch: u64,
        cx: &mut App,
    ) -> Option<SharedElementCover> {
        let Some(source) = Self::grid_cover_source(post) else {
            return SharedElementCoverRasterizer::text_cover(post, cell_size, display_scale, epoch);
        };
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        Self::cover(&source, is_own, cell_size, display_scale)
    }

    /// grid cell picture from source, tries cell bucket then full size then any live decode
    pub fn cover(source: &str, is_own: bool, cell_size: Size<Pixels>, display_scale: f32) -> Option<SharedElementCover> {
        let resolved = Self::bitmap(
            &[source.to_owned()],
            is_own,
            &[Self::cell_bucket(cell_size, display_scale), None],
        )?;
        Some(SharedElementCover {
            image: SharedElementCoverContent::Image(resolved.image),
            aspect: resolved.aspect,
            identity: source.to_owned(),
        })
    }

    /// dimmed backdrop behind fit mode media, at most BACKDROP_PIXEL_BUCKET pixels
    /// cached blurred bitmap, else blurs once off main and gives a dimmed copy if small
    /// on_ready gets the blurred one on main, low power mode skips the blur
    pub fn backdrop(
        url_string: &str,
        is_own: bool,
        display_size: Size<Pixels>,
        is_current: impl Fn(&App) -> bool + 'static,
        on_ready: impl FnOnce(CleverImage, &mut App) + 'static,
        cx: &mut App,
    ) -> Option<CleverImage> {
        let bucket = SharedElementMotion::BACKDROP_PIXEL_BUCKET;
        let source = Self::bitmap(&[url_string.to_owned()], is_own, &[Some(bucket), None])?;
        let user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let longest = Self::pixel_longest(&source.image);
        let blurs = {
            let energy = RiseEnergySavingStore::shared(cx);
            !energy.read(cx).system_low_power_mode()
        };
        let radius = if blurs {
            Self::backdrop_blur_radius(longest.min(bucket), display_size)
        } else {
            0.0
        };
        let key = [
            user_id.clone().unwrap_or_else(|| "anonymous".to_owned()),
            account_epoch.clone().unwrap_or_else(|| "none".to_owned()),
            CleverImageDiskCache::normalized_cache_key(&source.source),
            format!("{radius}"),
        ]
        .join("|");
        if let Some(cached) = BACKDROP_CACHE.object(&key) {
            return Some(cached);
        }

        let dim = SharedElementMotion::BACKDROP_DIM;
        let is_small = longest <= bucket * 2;
        if radius == 0.0 && is_small {
            let flat = SharedElementBackdropRenderer::render(&source.image, bucket, 0.0, dim)?;
            BACKDROP_CACHE.set_object(Arc::clone(&flat), key);
            return Some(flat);
        }

        Self::enqueue_backdrop(
            key,
            Arc::clone(&source.image),
            radius,
            user_id,
            account_epoch,
            BackdropWaiter { is_current: Box::new(is_current), on_ready: Box::new(on_ready) },
            cx,
        );
        let shows_flat_copy = radius > 0.0 && is_small;
        if !shows_flat_copy {
            return None;
        }
        SharedElementBackdropRenderer::render(&source.image, bucket, 0.0, dim)
    }

    fn enqueue_backdrop(
        key: String,
        image: CleverImage,
        radius: f32,
        user_id: Option<String>,
        account_epoch: Option<String>,
        waiter: BackdropWaiter,
        cx: &mut App,
    ) {
        let waiters = &mut cx.default_global::<SharedElementBitmapResolverWaiters>().backdrop_waiters;
        if let Some(pending) = waiters.get_mut(&key) {
            pending.push(waiter);
            return;
        }
        waiters.insert(key.clone(), vec![waiter]);
        let bucket = SharedElementMotion::BACKDROP_PIXEL_BUCKET;
        let dim = SharedElementMotion::BACKDROP_DIM;
        let work = RiseAccountRuntime::current()
            .spawn_blocking(move || SharedElementBackdropRenderer::render(&image, bucket, radius, dim));
        cx.spawn(async move |cx| {
            let rendered = work.await.ok().flatten();
            cx.update(|cx| Self::finish_backdrop(&key, rendered, user_id.as_deref(), account_epoch.as_deref(), cx));
        })
        .detach();
    }

    fn finish_backdrop(
        key: &str,
        image: Option<CleverImage>,
        user_id: Option<&str>,
        account_epoch: Option<&str>,
        cx: &mut App,
    ) {
        let waiters = cx
            .default_global::<SharedElementBitmapResolverWaiters>()
            .backdrop_waiters
            .remove(key)
            .unwrap_or_default();
        let Some(image) = image else {
            return;
        };
        if !SaiAccountCacheScope::is_current_account_epoch(user_id, account_epoch) {
            return;
        }
        BACKDROP_CACHE.set_object(Arc::clone(&image), key.to_owned());
        for waiter in waiters {
            if (waiter.is_current)(cx) {
                (waiter.on_ready)(Arc::clone(&image), cx);
            }
        }
    }

    fn backdrop_blur_radius(pixel_longest: i64, display_size: Size<Pixels>) -> f32 {
        let display_longest = f32::from(display_size.width).max(f32::from(display_size.height));
        if !display_longest.is_finite() || display_longest <= 0.0 || pixel_longest <= 0 {
            return SharedElementMotion::BACKDROP_BLUR;
        }
        let radius = SharedElementMotion::BACKDROP_BLUR * pixel_longest as f32 / display_longest;
        0.5_f32.max((radius * 2.0).round() / 2.0)
    }

    /// decoded frames are already upright, the decoder applies orientation
    fn make_bitmap(image: CleverImage, source: &str) -> Option<SharedElementBitmapResolverBitmap> {
        if image.frame_count() == 0 {
            return None;
        }
        let size = image.size(0);
        if size.width.0 <= 0 || size.height.0 <= 0 {
            return None;
        }
        Some(SharedElementBitmapResolverBitmap {
            aspect: size.width.0 as f32 / size.height.0 as f32,
            image,
            source: source.to_owned(),
        })
    }

    fn pixel_longest(image: &RenderImage) -> i64 {
        let size = image.size(0);
        i64::from(size.width.0.max(size.height.0))
    }

    fn unique<Element: Eq + Hash + Clone>(elements: Vec<Element>) -> Vec<Element> {
        let mut seen = HashSet::new();
        elements.into_iter().filter(|element| seen.insert(element.clone())).collect()
    }
}

enum SharedElementBackdropRenderer {}

impl SharedElementBackdropRenderer {
    fn render(image: &RenderImage, maximum_pixel_dimension: i64, blur_radius: f32, dim: f32) -> Option<CleverImage> {
        if blur_radius <= 0.0 {
            return Self::downscaled(image, maximum_pixel_dimension, dim);
        }
        let small = Self::downscaled(image, maximum_pixel_dimension, 0.0)?;
        let blurred = gaussian_blur_render_image(&small, blur_radius, true);
        let keep = (1.0 - dim).clamp(0.0, 1.0);
        let size = blurred.size(0);
        let mut pixels = blurred.as_bytes(0)?.to_vec();
        for pixel in pixels.chunks_exact_mut(4) {
            for channel in &mut pixel[..3] {
                *channel = (f32::from(*channel) * keep).round() as u8;
            }
        }
        Self::make_image(pixels, size.width.0, size.height.0)
    }

    fn downscaled(image: &RenderImage, maximum_pixel_dimension: i64, dim: f32) -> Option<CleverImage> {
        let size = image.size(0);
        let longest = i64::from(size.width.0.max(size.height.0));
        if longest <= 0 || maximum_pixel_dimension <= 0 {
            return None;
        }
        let scale = (maximum_pixel_dimension as f32 / longest as f32).min(1.0);
        let width = 1_u32.max((size.width.0 as f32 * scale).round() as u32);
        let height = 1_u32.max((size.height.0 as f32 * scale).round() as u32);
        let source = RgbaImage::from_raw(
            u32::try_from(size.width.0).ok()?,
            u32::try_from(size.height.0).ok()?,
            image.as_bytes(0)?.to_vec(),
        )?;
        let mut scaled = imageops::resize(&source, width, height, FilterType::Triangle);
        if dim > 0.0 {
            let overlay = dim.min(1.0);
            for pixel in scaled.pixels_mut() {
                let alpha = f32::from(pixel.0[3]) / 255.0;
                let out_alpha = overlay + alpha * (1.0 - overlay);
                if out_alpha > 0.0 {
                    let color_scale = alpha * (1.0 - overlay) / out_alpha;
                    for channel in &mut pixel.0[..3] {
                        *channel = (f32::from(*channel) * color_scale).round() as u8;
                    }
                }
                pixel.0[3] = (out_alpha * 255.0).round() as u8;
            }
        }
        let (width, height) = scaled.dimensions();
        Self::make_image(scaled.into_raw(), i32::try_from(width).ok()?, i32::try_from(height).ok()?)
    }

    fn make_image(pixels: Vec<u8>, width: i32, height: i32) -> Option<CleverImage> {
        let buffer = RgbaImage::from_raw(u32::try_from(width).ok()?, u32::try_from(height).ok()?, pixels)?;
        Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
    }
}
