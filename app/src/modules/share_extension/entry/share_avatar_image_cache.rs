use std::io::Cursor;
use std::sync::LazyLock;
use std::time::Duration;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageReader};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::{URLRequest, URLSession};

use crate::core::media::remote_media_disk_cache::RemoteMediaHttpResponse;
use crate::core::media::url_cache::{CachedURLResponse, URLCache};
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::modules::share_extension::shared::rise_share_container::RiseShareContainer;

const MEMORY_COUNT_LIMIT: usize = 180;
const MEMORY_TOTAL_COST_LIMIT: usize = 32 * 1_024 * 1_024;
const MAXIMUM_PIXEL_SIZE: u32 = 240;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(18);
const MAXIMUM_DOWNLOAD_BYTES: usize = 16 * 1_024 * 1_024;
const PERSISTED_JPEG_QUALITY: u8 = 84;
const ACCEPT_HEADER: &str = "image/avif,image/webp,image/*,*/*;q=0.6";
const USER_AGENT: &str = "RiseOnlyShare/1";

static MEMORY: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(MEMORY_COUNT_LIMIT);
    cache.set_total_cost_limit(MEMORY_TOTAL_COST_LIMIT);
    cache
});

pub enum ShareAvatarImageCache {}

impl ShareAvatarImageCache {
    /// reads the disk copy synchronously, so keep it off the main thread
    pub fn cached_image(url: &Url) -> Option<CleverImage> {
        let key = url.absolute_string().to_owned();
        if let Some(image) = MEMORY.object(&key) {
            return Some(image);
        }
        let data = RiseShareContainer::avatar_image_data(url)?;
        let image = Self::downsample(&data, MAXIMUM_PIXEL_SIZE)?;
        let image = Self::store_in_memory(image, key);
        Some(image)
    }

    pub async fn load_image(url: Url) -> Option<CleverImage> {
        let lookup_url = url.clone();
        let cached = RiseAccountRuntime::current()
            .spawn_blocking(move || Self::cached_image(&lookup_url))
            .await
            .ok()
            .flatten();
        if cached.is_some() {
            return cached;
        }

        let mut request = URLRequest::new(url.clone());
        request.timeout_interval = REQUEST_TIMEOUT;
        request.set_value(ACCEPT_HEADER, "Accept");
        request.set_value(USER_AGENT, "User-Agent");

        let (data, status_code) = match URLCache::shared().cached_response(&request) {
            Some(cached) => (cached.data, cached.response.status_code),
            None => {
                let handle = RiseAccountRuntime::current().spawn({
                    let request = request.clone();
                    async move { URLSession::shared().data_for(request).await }
                });
                let _abort_on_drop = AbortOnDrop(handle.abort_handle());
                let (data, response) = handle.await.ok()?.ok()?;
                if (200..=299).contains(&response.status_code) && !data.is_empty() {
                    URLCache::shared().store_cached_response(
                        CachedURLResponse::new(
                            RemoteMediaHttpResponse::new(url.clone(), response.status_code, Vec::new()),
                            data.clone(),
                        ),
                        &request,
                    );
                }
                (data, response.status_code)
            }
        };
        if !(200..=299).contains(&status_code) || data.is_empty() || data.len() > MAXIMUM_DOWNLOAD_BYTES {
            return None;
        }

        let store_url = url.clone();
        RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let image = Self::downsample(&data, MAXIMUM_PIXEL_SIZE)?;
                let persisted = Self::jpeg_data(&image, PERSISTED_JPEG_QUALITY).unwrap_or(data);
                let _ = RiseShareContainer::store_avatar_image_data(&persisted, &store_url);
                Some(Self::store_in_memory(image, store_url.absolute_string().to_owned()))
            })
            .await
            .ok()
            .flatten()
    }

    /// decodes a thumbnail at most this size and applies the exif rotation
    fn downsample(data: &[u8], maximum_pixel_size: u32) -> Option<DynamicImage> {
        let reader = ImageReader::new(Cursor::new(data)).with_guessed_format().ok()?;
        let mut decoder = reader.into_decoder().ok()?;
        let orientation = image::ImageDecoder::orientation(&mut decoder).ok();
        let mut image = DynamicImage::from_decoder(decoder).ok()?;
        if image.width() == 0 || image.height() == 0 {
            return None;
        }
        if let Some(orientation) = orientation {
            image.apply_orientation(orientation);
        }
        if image.width().max(image.height()) > maximum_pixel_size {
            image = image.thumbnail(maximum_pixel_size, maximum_pixel_size);
        }
        Some(image)
    }

    fn jpeg_data(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, quality)
            .encode_image(&image.to_rgb8())
            .ok()?;
        Some(data)
    }

    /// puts the image in memory, cost is the bitmap size
    fn store_in_memory(image: DynamicImage, key: String) -> CleverImage {
        let rgba = image.into_rgba8();
        let cost = usize::try_from(rgba.width())
            .unwrap_or(0)
            .saturating_mul(4)
            .saturating_mul(usize::try_from(rgba.height()).unwrap_or(0));
        let image = CleverImageBitmapDecoder::render_image(rgba);
        MEMORY.set_object_cost(image.clone(), key, cost);
        image
    }
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
