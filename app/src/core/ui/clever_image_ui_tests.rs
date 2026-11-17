use std::io::Cursor;

use gpui::TestAppContext;
use image::codecs::jpeg::JpegEncoder;
use image::{Rgb, RgbImage};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_upload::rise_url_session::URLRequest;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::RemoteMediaHttpResponse;
use crate::core::media::url_cache::{CachedURLResponse, URLCache};
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::ui::clever_image_disk_cache::{
    CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageDiskCache, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;

struct CleverImageCachePolicyCleanup {
    user_id: String,
    source: String,
    request: URLRequest,
}

impl Drop for CleverImageCachePolicyCleanup {
    fn drop(&mut self) {
        CleverImageDiskCache::remove_persisted_account(&self.user_id);
        MediaViewerImageMemoryCache::remove(&self.source, Some(&self.user_id));
        URLCache::shared().remove_cached_response(&self.request);
        SaiAccountCacheScope::clear_active_account();
    }
}

#[gpui::test]
async fn memory_only_ignores_persistent_copies_without_destroying_shared_cache(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let user_id = format!("image-memory-only-{}", Uuid::new_v4().uuid_string());
    let source = format!("https://example.invalid/post-{}.jpg", Uuid::new_v4().uuid_string());
    let url = Url::from_string(&source).expect("test url");
    let request = URLRequest::new(url.clone());
    let response = RemoteMediaHttpResponse::new(url, 200, vec![("Content-Type".to_owned(), "image/jpeg".to_owned())]);
    let bitmap = RgbImage::from_pixel(8, 8, Rgb([255, 149, 0]));
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(Cursor::new(&mut bytes), 90)
        .encode_image(&bitmap)
        .expect("jpeg encode of a solid bitmap");
    let image = CleverImageBitmapDecoder::decode(&bytes, None).expect("decoded bitmap");

    SaiAccountCacheScope::activate(&user_id);
    let _cleanup = CleverImageCachePolicyCleanup {
        user_id: user_id.clone(),
        source: source.clone(),
        request: request.clone(),
    };

    CleverImageDiskCache::store_data(&bytes, &source, Some(&user_id), None);
    URLCache::shared().store_cached_response(CachedURLResponse::new(response, bytes.clone()), &request);

    let loaded = CleverImageBitmapLoader::load(
        &source,
        false,
        false,
        SaiAccountCacheScope::active_user_id(),
        SaiAccountCacheScope::active_epoch(),
        None,
        CleverImageCachePolicy::MemoryOnly,
        CleverImageWorkLane::Visible,
        &async_cx,
    )
    .await;

    assert!(loaded.is_none());
    assert_eq!(
        CleverImageDiskCache::image_data(&source, Some(&user_id), None).as_deref(),
        Some(bytes.as_slice())
    );
    assert_eq!(
        URLCache::shared().cached_response(&request).map(|cached| cached.data).as_deref(),
        Some(bytes.as_slice())
    );

    MediaViewerImageMemoryCache::store(image, &source, Some(&user_id));
    assert!(
        CleverImageBitmapLoader::image_from_memory_or_disk(
            &source,
            false,
            Some(&user_id),
            SaiAccountCacheScope::active_epoch().as_deref(),
            CleverImageCachePolicy::MemoryOnly,
            None,
        )
        .is_some()
    );
}
