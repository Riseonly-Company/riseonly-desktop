use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{
    App, AsyncApp, Context, ElementId, IntoElement, ObjectFit, ParentElement, Render, RenderImage, RenderOnce,
    Styled, StyledImage, StyleRefinement, Task, WeakEntity, Window, black, img, px,
};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_ui::swiftui::blur::gaussian_blur_render_image;
use rise_ui::swiftui::shapes::pass_through;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperRender;

/// photo wallpaper under the bubbles with blur and dim
/// own host crops to offered size so wide photos dont stretch
/// blur baked once off main
#[derive(IntoElement)]
pub struct ChatWallpaperImageLayer {
    pub render: ChatWallpaperRender,
    id: ElementId,
    maximum_pixel_dimension: i64,
    corner_radius: f32,
}

impl ChatWallpaperImageLayer {
    pub fn new(render: ChatWallpaperRender) -> Self {
        Self {
            render,
            id: ElementId::from("chat-wallpaper-image-layer"),
            maximum_pixel_dimension: ChatWallpaperBitmapCache::MAXIMUM_PIXEL_DIMENSION,
            corner_radius: 0.0,
        }
    }

    /// corner radius for the picture, gpui only clips a parent by its rect so tiles need this
    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    /// caps decode size, settings tiles are small and decoding at window size made the page load forever
    pub fn maximum_pixel_dimension(mut self, maximum_pixel_dimension: i64) -> Self {
        self.maximum_pixel_dimension = maximum_pixel_dimension.clamp(1, ChatWallpaperBitmapCache::MAXIMUM_PIXEL_DIMENSION);
        self
    }

    /// element id for this layer, sibling layers need their own key or the host dies
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }
}

impl RenderOnce for ChatWallpaperImageLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let dimension = self.maximum_pixel_dimension;
        let first_render = self.render.clone();
        let host = window.use_keyed_state(self.id, cx, move |_, cx| {
            let mut host = ChatWallpaperImageHostView::new();
            host.apply_sized(first_render, dimension, cx);
            host
        });
        let corner_radius = self.corner_radius;
        host.update(cx, |host, cx| {
            if host.corner_radius != corner_radius {
                host.corner_radius = corner_radius;
                cx.notify();
            }
            host.apply_sized(self.render, dimension, cx);
        });
        pass_through().size_full().child(host.cached(StyleRefinement::default().size_full()))
    }
}

pub struct ChatWallpaperImageHostView {
    image: Option<CleverImage>,
    dim: f32,
    applied_identity: Option<String>,
    load_task: Option<Task<()>>,
    corner_radius: f32,
}

impl ChatWallpaperImageHostView {
    fn new() -> Self {
        Self {
            image: None,
            dim: 0.0,
            applied_identity: None,
            load_task: None,
            corner_radius: 0.0,
        }
    }

    pub fn apply(&mut self, next: ChatWallpaperRender, cx: &mut Context<Self>) {
        self.apply_sized(next, ChatWallpaperBitmapCache::MAXIMUM_PIXEL_DIMENSION, cx);
    }

    pub fn apply_sized(&mut self, next: ChatWallpaperRender, maximum_pixel_dimension: i64, cx: &mut Context<Self>) {
        let dim = next.look.dim().clamp(0.0, 1.0) as f32;
        if dim != self.dim {
            self.dim = dim;
            cx.notify();
        }
        let identity = ChatWallpaperBitmapCache::key(&next.image_url, next.look.blur_percent, maximum_pixel_dimension);
        if self.applied_identity.as_deref() == Some(identity.as_str()) {
            return;
        }
        self.applied_identity = Some(identity.clone());
        let blur = next.look.blur_percent;
        let source = next.image_url;
        if let Some(cached) = ChatWallpaperBitmapCache::cached_image_sized(&source, blur, maximum_pixel_dimension) {
            self.load_task = None;
            self.image = Some(cached);
            cx.notify();
            return;
        }
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let image = ChatWallpaperBitmapCache::image_sized(&source, blur, maximum_pixel_dimension, cx).await;
            let Some(image) = image else { return };
            let _ = this.update(cx, |this, cx| {
                if this.applied_identity.as_deref() != Some(identity.as_str()) {
                    return;
                }
                this.image = Some(image);
                Self::notify_batched(cx);
            });
        }));
    }

    const NOTIFY_BATCH: Duration = Duration::from_millis(120);

    fn notify_batched(cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        PENDING_NOTIFIES.with(|pending| pending.borrow_mut().push(weak));
        if NOTIFY_FLUSH.with(|flush| flush.borrow().is_some()) {
            return;
        }
        let task = cx.spawn(async move |_, cx| {
            cx.background_executor().timer(Self::NOTIFY_BATCH).await;
            NOTIFY_FLUSH.with(|flush| flush.borrow_mut().take());
            let hosts = PENDING_NOTIFIES.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
            let _ = cx.update(|cx| {
                for host in hosts {
                    let _ = host.update(cx, |_, cx| cx.notify());
                }
            });
        });
        NOTIFY_FLUSH.with(|flush| *flush.borrow_mut() = Some(task));
    }

    pub fn stop(&mut self) {
        self.load_task = None;
    }
}

impl Drop for ChatWallpaperImageHostView {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Render for ChatWallpaperImageHostView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let radius = px(self.corner_radius);
        pass_through()
            .relative()
            .size_full()
            .overflow_hidden()
            .rounded(radius)
            .bg(black())
            .children(self.image.clone().map(|image| {
                img(image)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .rounded(radius)
                    .object_fit(ObjectFit::Cover)
            }))
            .child(pass_through().absolute().inset_0().rounded(radius).bg(black()).opacity(self.dim))
    }
}

thread_local! {
    static PENDING_NOTIFIES: RefCell<Vec<WeakEntity<ChatWallpaperImageHostView>>> = const { RefCell::new(Vec::new()) };
    static NOTIFY_FLUSH: RefCell<Option<Task<()>>> = const { RefCell::new(None) };
}

/// bounded cache of wallpaper bitmaps, downsampled and blurred once
/// same wallpaper in ten chats decodes once
pub enum ChatWallpaperBitmapCache {}

static IMAGES: LazyLock<NSCache<String, Arc<RenderImage>>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(64);
    cache.set_total_cost_limit(96 * 1024 * 1024);
    cache
});

impl ChatWallpaperBitmapCache {
    pub const MAXIMUM_PIXEL_DIMENSION: i64 = 1_536;

    /// cache key from source, blur and decode size
    pub fn key(source: &str, blur_percent: i64, maximum_pixel_dimension: i64) -> String {
        if maximum_pixel_dimension >= Self::MAXIMUM_PIXEL_DIMENSION {
            format!("{source}|blur:{blur_percent}")
        } else {
            format!("{source}|blur:{blur_percent}|px:{maximum_pixel_dimension}")
        }
    }

    pub fn cached_image(source: &str, blur_percent: i64) -> Option<CleverImage> {
        Self::cached_image_sized(source, blur_percent, Self::MAXIMUM_PIXEL_DIMENSION)
    }

    pub fn cached_image_sized(source: &str, blur_percent: i64, maximum_pixel_dimension: i64) -> Option<CleverImage> {
        IMAGES.object(&Self::key(source, blur_percent, maximum_pixel_dimension))
    }

    pub async fn image(source: &str, blur_percent: i64, cx: &mut AsyncApp) -> Option<CleverImage> {
        Self::image_sized(source, blur_percent, Self::MAXIMUM_PIXEL_DIMENSION, cx).await
    }

    pub async fn image_sized(
        source: &str,
        blur_percent: i64,
        maximum_pixel_dimension: i64,
        cx: &mut AsyncApp,
    ) -> Option<CleverImage> {
        let maximum_pixel_dimension = maximum_pixel_dimension.clamp(1, Self::MAXIMUM_PIXEL_DIMENSION);
        let key = Self::key(source, blur_percent, maximum_pixel_dimension);
        if let Some(hit) = IMAGES.object(&key) {
            return Some(hit);
        }
        let base = Self::base_image(source, maximum_pixel_dimension, cx).await?;
        let blurred = if blur_percent > 0 {
            let radius = Self::blur_radius(blur_percent, &base);
            let input = Arc::clone(&base);
            RiseAccountRuntime::current()
                .spawn_blocking(move || Self::blur(&input, radius))
                .await
                .ok()
                .flatten()
                .unwrap_or(base)
        } else {
            base
        };
        let size = blurred.size(0);
        let width = usize::try_from(size.width.0).unwrap_or(0);
        let height = usize::try_from(size.height.0).unwrap_or(0);
        IMAGES.set_object_cost(Arc::clone(&blurred), key, width * height * 4);
        Some(blurred)
    }

    /// decoded picture from the account disk cache, a pending upload is decoded from the local file
    async fn base_image(source: &str, maximum_pixel_dimension: i64, cx: &mut AsyncApp) -> Option<CleverImage> {
        if let Some(file_path) = Self::local_file_path(source) {
            return RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    let data = std::fs::read(&file_path).ok()?;
                    CleverImageBitmapDecoder::decode(&data, Some(maximum_pixel_dimension))
                })
                .await
                .ok()
                .flatten();
        }
        CleverImageBitmapLoader::load(
            source,
            true,
            true,
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
            Some(maximum_pixel_dimension),
            CleverImageCachePolicy::Standard,
            CleverImageWorkLane::Visible,
            cx,
        )
        .await
    }

    fn local_file_path(source: &str) -> Option<PathBuf> {
        if Path::new(source).is_absolute() {
            return Some(PathBuf::from(source));
        }
        let url = Url::from_string(source)?;
        if url.is_file_url() {
            return url.to_file_path();
        }
        None
    }

    /// blur radius scaled with the bitmap so it looks the same at any decode size
    fn blur_radius(blur_percent: i64, image: &RenderImage) -> f64 {
        let size = image.size(0);
        let longest = size.width.0.max(size.height.0).max(0);
        let fraction = blur_percent.clamp(0, 100) as f64 / 100.0;
        fraction * f64::from(longest) * 0.045
    }

    fn blur(image: &CleverImage, radius: f64) -> Option<CleverImage> {
        if radius <= 0.5 || image.frame_count() == 0 {
            return Some(Arc::clone(image));
        }
        Some(Arc::new(gaussian_blur_render_image(image, radius as f32, true)))
    }
}
