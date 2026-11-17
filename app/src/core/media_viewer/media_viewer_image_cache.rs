use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use futures::channel::oneshot;
use gpui::{
    AnyElement, App, AsyncApp, ElementId, Entity, ImageSource, IntoElement, RenderImage, RenderOnce, Size,
    Styled, StyledImage, Window, div, img, rgba,
};
use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::URLRequest;
use rise_platform::photo_library::{self, ImageContentMode, ImageDeliveryMode, ImageRequestId};
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::url_cache::URLCache;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::media_picker::library::photos_image_request::PhotosImageRequest;

const ACCOUNT_KEY_PREFIX: &str = "account:";

static CACHE: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(
        usize::try_from(RiseLimits::client_value("media_viewer_image_memory_cache_entries")).unwrap_or(0),
    );
    cache.set_total_cost_limit(
        usize::try_from(RiseLimits::client_value("media_viewer_image_memory_cache_bytes")).unwrap_or(0),
    );
    cache
});

pub enum MediaViewerImageMemoryCache {}

impl MediaViewerImageMemoryCache {
    pub fn image(
        url_string: &str,
        account_user_id: Option<&str>,
        cache_policy: CleverImageCachePolicy,
    ) -> Option<CleverImage> {
        if cache_policy == CleverImageCachePolicy::PrivateEphemeral {
            return None;
        }
        if cache_policy == CleverImageCachePolicy::MemoryOnly {
            return Self::memory_only(url_string, account_user_id);
        }
        let raw_key = Self::normalized(url_string);
        let key = Self::scoped_key(raw_key, account_user_id);
        if let Some(hit) = CACHE.object(&key) {
            return Some(hit);
        }
        let disk = MediaViewerBitmapLoader::data_from_url_cache_only(raw_key)
            .and_then(|data| CleverImageBitmapDecoder::decode(&data, None))?;
        CACHE.set_object_cost(Arc::clone(&disk), key, Self::image_cost(&disk));
        Some(disk)
    }

    pub fn memory_only(url_string: &str, account_user_id: Option<&str>) -> Option<CleverImage> {
        CACHE.object(&Self::scoped_key(Self::normalized(url_string), account_user_id))
    }

    pub fn store(image: CleverImage, url_string: &str, account_user_id: Option<&str>) {
        let cost = Self::image_cost(&image);
        CACHE.set_object_cost(image, Self::scoped_key(Self::normalized(url_string), account_user_id), cost);
    }

    pub fn remove(url_string: &str, account_user_id: Option<&str>) {
        CACHE.remove_object(&Self::scoped_key(Self::normalized(url_string), account_user_id));
    }

    pub fn clear_all() {
        CACHE.remove_all_objects();
    }

    fn normalized(url_string: &str) -> &str {
        trimming_whitespaces_and_newlines(url_string)
    }

    fn scoped_key(key: &str, account_user_id: Option<&str>) -> String {
        if key.is_empty() || key.starts_with(ACCOUNT_KEY_PREFIX) {
            return key.to_owned();
        }
        let user_id = account_user_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if user_id.is_empty() {
            return key.to_owned();
        }
        SaiAccountCacheScope::prefix(user_id) + key
    }

    fn image_cost(image: &RenderImage) -> usize {
        image.as_bytes(0).map_or(0, <[u8]>::len)
    }
}

pub type ChatMediaImageMemoryCache = MediaViewerImageMemoryCache;

pub enum MediaViewerBitmapLoader {}

impl MediaViewerBitmapLoader {
    pub fn data_from_url_cache_only(url_string: &str) -> Option<Vec<u8>> {
        let t = trimming_whitespaces_and_newlines(url_string);
        if !t.to_lowercase().starts_with("http") {
            return None;
        }
        let url = Url::from_string(t)?;
        URLCache::shared()
            .cached_response(&URLRequest::new(url))
            .map(|cached| cached.data)
    }

    pub fn ui_image_from_url_cache_only(url_string: &str) -> Option<CleverImage> {
        let data = Self::data_from_url_cache_only(url_string)?;
        CleverImageBitmapDecoder::decode(&data, None)
    }

    pub async fn load_into_memory(
        url_string: &str,
        network_if_missing: bool,
        persist_to_disk: bool,
        cache_policy: CleverImageCachePolicy,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return None;
        }
        let t = trimming_whitespaces_and_newlines(url_string);

        if let Some(local_id) = t.strip_prefix("ph://") {
            if let Some(hit) = MediaViewerImageMemoryCache::memory_only(t, account_user_id.as_deref()) {
                return Some(hit);
            }
            return Self::load_ph_asset_image(local_id, t, account_user_id, account_epoch).await;
        }

        if let Some(local_url) = Self::local_file_url(t) {
            if cache_policy != CleverImageCachePolicy::PrivateEphemeral
                && let Some(hit) = MediaViewerImageMemoryCache::memory_only(t, account_user_id.as_deref())
            {
                return Some(hit);
            }
            return Self::load_local_file_image(local_url, t, account_user_id, account_epoch, cache_policy).await;
        }

        if !t.to_lowercase().starts_with("http") {
            return None;
        }
        CleverImageBitmapLoader::load(
            t,
            persist_to_disk,
            network_if_missing,
            account_user_id,
            account_epoch,
            None,
            cache_policy,
            CleverImageWorkLane::Visible,
            cx,
        )
        .await
    }

    async fn load_local_file_image(
        url: PathBuf,
        cache_key: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cache_policy: CleverImageCachePolicy,
    ) -> Option<CleverImage> {
        let image = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let data = std::fs::read(&url).ok()?;
                CleverImageBitmapDecoder::decode(&data, None)
            })
            .await
            .ok()
            .flatten()?;
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return None;
        }
        if cache_policy != CleverImageCachePolicy::PrivateEphemeral {
            MediaViewerImageMemoryCache::store(Arc::clone(&image), cache_key, account_user_id.as_deref());
        }
        Some(image)
    }

    fn local_file_url(source: &str) -> Option<PathBuf> {
        if source.to_lowercase().starts_with("file://")
            && let Some(url) = Url::from_string(source)
            && url.is_file_url()
        {
            return url.to_file_path();
        }
        if is_local_absolute_path(source) {
            return Some(PathBuf::from(source));
        }
        None
    }

    async fn load_ph_asset_image(
        local_identifier: &str,
        cache_key: &str,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
    ) -> Option<CleverImage> {
        let image = MediaViewerPhotosImageRequest::load(
            local_identifier,
            Size {
                width: MEDIA_VIEWER_PHOTOS_TARGET_SIDE,
                height: MEDIA_VIEWER_PHOTOS_TARGET_SIDE,
            },
        )
        .await?;
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return None;
        }
        MediaViewerImageMemoryCache::store(Arc::clone(&image), cache_key, account_user_id.as_deref());
        Some(image)
    }
}

const MEDIA_VIEWER_PHOTOS_TARGET_SIDE: f32 = 1920.0;

type MediaViewerPhotosContinuation = Arc<Mutex<Option<oneshot::Sender<Option<CleverImage>>>>>;

struct MediaViewerPhotosImageRequest {
    continuation: MediaViewerPhotosContinuation,
    request_id: Option<ImageRequestId>,
}

impl MediaViewerPhotosImageRequest {
    async fn load(local_identifier: &str, target_size: Size<f32>) -> Option<CleverImage> {
        let (sender, receiver) = oneshot::channel();
        let continuation: MediaViewerPhotosContinuation = Arc::new(Mutex::new(Some(sender)));
        let delivery_continuation = Arc::clone(&continuation);
        let request_id = PhotosImageRequest::request_image(
            local_identifier,
            target_size,
            ImageContentMode::AspectFit,
            ImageDeliveryMode::HighQuality,
            true,
            move |delivery| {
                if delivery.is_degraded {
                    return;
                }
                Self::finish(
                    &delivery_continuation,
                    if delivery.is_cancelled { None } else { delivery.image },
                );
            },
        );
        let request = Self {
            continuation,
            request_id,
        };
        let image = receiver.await.ok().flatten();
        drop(request);
        image
    }

    fn finish(continuation: &MediaViewerPhotosContinuation, image: Option<CleverImage>) {
        if let Some(sender) = continuation.lock().take() {
            let _ = sender.send(image);
        }
    }
}

impl Drop for MediaViewerPhotosImageRequest {
    fn drop(&mut self) {
        let unfinished = self.continuation.lock().take().is_some();
        if unfinished && let Some(request_id) = self.request_id {
            photo_library::cancel_image(request_id);
        }
    }
}

pub type ChatMediaHeroBitmap = MediaViewerBitmapLoader;

#[derive(IntoElement)]
pub struct MediaViewerCachedImageUi {
    id: ElementId,
    source: String,
    content_mode: ContentMode,
    seed_image: Option<CleverImage>,
    persist_to_disk: bool,
    cache_policy: CleverImageCachePolicy,
}

#[derive(Default)]
struct MediaViewerCachedImageState {
    ui_image: Option<CleverImage>,
}

impl MediaViewerCachedImageUi {
    pub fn new(id: impl Into<ElementId>, source: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            content_mode: ContentMode::Fit,
            seed_image: None,
            persist_to_disk: false,
            cache_policy: CleverImageCachePolicy::Standard,
        }
    }

    pub fn content_mode(mut self, content_mode: ContentMode) -> Self {
        self.content_mode = content_mode;
        self
    }

    pub fn seed_image(mut self, seed_image: Option<CleverImage>) -> Self {
        self.seed_image = seed_image;
        self
    }

    pub fn persist_to_disk(mut self, persist_to_disk: bool) -> Self {
        self.persist_to_disk = persist_to_disk;
        self
    }

    pub fn cache_policy(mut self, cache_policy: CleverImageCachePolicy) -> Self {
        self.cache_policy = cache_policy;
        self
    }

    fn adopt_seed_if_needed(
        state: &Entity<MediaViewerCachedImageState>,
        seed_image: Option<&CleverImage>,
        source: &str,
        cx: &mut App,
    ) {
        if let Some(seed_image) = seed_image {
            let seed = Arc::clone(seed_image);
            state.update(cx, |state, cx| {
                state.ui_image = Some(Arc::clone(&seed));
                cx.notify();
            });
            MediaViewerImageMemoryCache::store(seed, source, SaiAccountCacheScope::active_user_id().as_deref());
        }
    }

    fn resolve(
        state: &Entity<MediaViewerCachedImageState>,
        seed_image: Option<&CleverImage>,
        source: &str,
        cx: &mut App,
    ) {
        if state.read(cx).ui_image.is_some() {
            return;
        }
        let resolved = seed_image.cloned().or_else(|| {
            MediaViewerImageMemoryCache::memory_only(source, SaiAccountCacheScope::active_user_id().as_deref())
        });
        if let Some(resolved) = resolved {
            state.update(cx, |state, cx| {
                state.ui_image = Some(resolved);
                cx.notify();
            });
        }
    }
}

impl RenderOnce for MediaViewerCachedImageUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), "media-viewer-cached-image".into()),
            cx,
            |_, _| MediaViewerCachedImageState::default(),
        );

        {
            let state = state.clone();
            let seed_image = self.seed_image.clone();
            on_change(
                ElementId::NamedChild(Arc::new(self.id.clone()), "media-viewer-cached-image-source".into()),
                self.source.clone(),
                false,
                window,
                cx,
                move |_, source, _, cx| {
                    state.update(cx, |state, cx| {
                        state.ui_image = None;
                        cx.notify();
                    });
                    Self::adopt_seed_if_needed(&state, seed_image.as_ref(), source, cx);
                    Self::resolve(&state, seed_image.as_ref(), source, cx);
                },
            );
        }

        {
            let state = state.clone();
            let source = self.source.clone();
            let persist_to_disk = self.persist_to_disk;
            let cache_policy = self.cache_policy;
            use_task(
                ElementId::NamedChild(Arc::new(self.id.clone()), "media-viewer-cached-image-load".into()),
                self.source.clone(),
                window,
                cx,
                async move |cx| {
                    let has_image = state.read_with(&*cx, |state, _| state.ui_image.is_some());
                    if has_image {
                        return;
                    }
                    let loaded =
                        MediaViewerBitmapLoader::load_into_memory(&source, true, persist_to_disk, cache_policy, cx)
                            .await;
                    state.update(cx, |state, cx| {
                        if state.ui_image.is_none()
                            && let Some(image) = loaded.or_else(|| {
                                MediaViewerImageMemoryCache::memory_only(
                                    &source,
                                    SaiAccountCacheScope::active_user_id().as_deref(),
                                )
                            })
                        {
                            state.ui_image = Some(image);
                            cx.notify();
                        }
                    });
                },
            );
        }

        let ui_image = state.read(cx).ui_image.clone();
        let seed_image = self.seed_image;
        let source = self.source;
        Self::body(ui_image, self.content_mode).on_appear(
            ElementId::NamedChild(Arc::new(self.id), "media-viewer-cached-image-appear".into()),
            move |_, cx| {
                Self::adopt_seed_if_needed(&state, seed_image.as_ref(), &source, cx);
                Self::resolve(&state, seed_image.as_ref(), &source, cx);
            },
        )
    }
}

impl MediaViewerCachedImageUi {
    fn body(ui_image: Option<CleverImage>, content_mode: ContentMode) -> AnyElement {
        match ui_image {
            Some(ui_image) => img(ImageSource::Render(ui_image))
                .size_full()
                .object_fit(content_mode.object_fit())
                .into_any_element(),
            None => div().size_full().bg(rgba(0x0000_0005)).into_any_element(),
        }
    }
}

pub type ChatMediaCachedImageUi = MediaViewerCachedImageUi;
