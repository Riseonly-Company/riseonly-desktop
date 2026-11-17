use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use futures::future::join_all;
use gpui::{
    App, AppContext, AsyncApp, Bounds, ElementId, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Size, Styled,
    Window, black, div, white,
};
use image::metadata::Orientation;
use image::{ImageDecoder, ImageReader};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::ns_cache::NSCache;
use rise_limits::MediaClass;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize};
use rise_ui::swiftui::blur::BlurModifiers;
use rise_ui::swiftui::lifecycle::on_change;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, pass_through};
use rise_ui::swiftui::transition::TransitionModifiers;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaDiskCacheMaterializeOptions};
use crate::core::ui::clever_image_disk_cache::{CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::modules::story::stores::story::story_actions::story_types::StoryPlaybackPolicy;
use crate::modules::story::stores::story::story_actions::story_wire_types::{StoryDTO, StoryDTOExt};

pub type StoryMediaGeometry = Rc<dyn Fn(Option<f32>, bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct StoryMediaView {
    id: ElementId,
    source: SharedString,
    corner_radius: f32,
    blur_radius: f32,
    show_blurred_fill: bool,
    pins_aspect_fit: bool,
    on_media_geometry: Option<StoryMediaGeometry>,
    geometry_key: i64,
    shows_loader: bool,
}

/// state of the view plus the size its content gets
struct StoryMediaViewState {
    aspect: Option<f32>,
    aspect_source: String,
    size: Size<Pixels>,
}

impl StoryMediaView {
    pub fn new(id: impl Into<ElementId>, source: impl Into<SharedString>, corner_radius: f32) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            corner_radius,
            blur_radius: 28.0,
            show_blurred_fill: true,
            pins_aspect_fit: false,
            on_media_geometry: None,
            geometry_key: 0,
            shows_loader: false,
        }
    }

    pub fn blur_radius(mut self, blur_radius: f32) -> Self {
        self.blur_radius = blur_radius;
        self
    }

    pub fn show_blurred_fill(mut self, show_blurred_fill: bool) -> Self {
        self.show_blurred_fill = show_blurred_fill;
        self
    }

    pub fn pins_aspect_fit(mut self, pins_aspect_fit: bool) -> Self {
        self.pins_aspect_fit = pins_aspect_fit;
        self
    }

    pub fn on_media_geometry(
        mut self,
        on_media_geometry: impl Fn(Option<f32>, bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_media_geometry = Some(Rc::new(on_media_geometry));
        self
    }

    pub fn geometry_key(mut self, geometry_key: i64) -> Self {
        self.geometry_key = geometry_key;
        self
    }

    pub fn shows_loader(mut self, shows_loader: bool) -> Self {
        self.shows_loader = shows_loader;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn resolve_aspect(key: String, state: gpui::Entity<StoryMediaViewState>, cx: &mut App) {
        if key.is_empty() {
            return;
        }
        state.update(cx, |state, _| state.aspect_source = key.clone());
        let probe_key = key.clone();
        StoryImageAspectProbe::aspect(
            &probe_key,
            move |value, cx| {
                state.update(cx, |state, cx| {
                    if state.aspect_source != key {
                        return;
                    }
                    if let Some(value) = value.filter(|value| *value > 0.0) {
                        state.aspect = Some(value);
                        cx.notify();
                    }
                });
            },
            cx,
        );
    }
}

impl RenderOnce for StoryMediaView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let clean = trimming_whitespaces_and_newlines(&self.source).to_owned();
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| StoryMediaViewState {
            aspect: None,
            aspect_source: String::new(),
            size: Size::default(),
        });
        let (aspect, size) = {
            let state = state.read(cx);
            (state.aspect, state.size)
        };
        let width = f32::from(size.width);
        let height = f32::from(size.height);
        let rect_aspect = width / height.max(1.0);
        let fills = !self.pins_aspect_fit
            && aspect.is_some_and(|aspect| {
                aspect > 0.0 && !StoryFrameAspect::is_story_frame(aspect) && aspect < rect_aspect
            });

        {
            let state = state.clone();
            on_change(self.child_id("source"), clean.clone(), true, window, cx, move |old, new, _, cx| {
                if old != new {
                    state.update(cx, |state, _| {
                        state.aspect = None;
                        state.aspect_source = String::new();
                    });
                }
                Self::resolve_aspect(new.clone(), state, cx);
            });
        }
        if let Some(on_media_geometry) = self.on_media_geometry.clone() {
            let report = on_media_geometry.clone();
            on_change(self.child_id("aspect"), aspect, true, window, cx, move |_, value, window, cx| {
                report(*value, fills, window, cx);
            });
            on_change(self.child_id("geometry-key"), self.geometry_key, false, window, cx, move |_, _, window, cx| {
                on_media_geometry(aspect, fills, window, cx);
            });
        }

        let mut stack = div().relative().size_full().bg(black());
        if !clean.is_empty() {
            if self.show_blurred_fill && !fills {
                stack = stack.child(
                    pass_through().absolute().inset_0().child(
                        div()
                            .size_full()
                            .child(
                                CleverImageUi::new(clean.clone())
                                    .id(self.child_id("blurred-fill"))
                                    .content_mode(ContentMode::Fill)
                                    .corner_radius(0.0)
                                    .persist_to_disk(true)
                                    .maximum_decode_pixel_dimension(StoryPlaybackPolicy::IMAGE_MAXIMUM_PIXEL_DIMENSION)
                                    .blur(self.blur_radius),
                            )
                            .scale_effect(1.18),
                    ),
                );
            }
            let seed_state = state.clone();
            let seed_source = clean.clone();
            let image_corner_radius = if fills { self.corner_radius } else { 0.0 };
            stack = stack.child(
                pass_through().absolute().inset_0().child(
                    CleverImageUi::new(clean.clone())
                        .id(self.child_id("image"))
                        .content_mode(if fills { ContentMode::Fill } else { ContentMode::Fit })
                        .corner_radius(image_corner_radius)
                        .persist_to_disk(true)
                        .maximum_decode_pixel_dimension(StoryPlaybackPolicy::IMAGE_MAXIMUM_PIXEL_DIMENSION)
                        .on_natural_size(move |size, _, cx| {
                            if size.height <= 0.0 {
                                return;
                            }
                            let value = size.width / size.height;
                            StoryImageAspectProbe::seed(value, &seed_source);
                            seed_state.update(cx, |state, cx| {
                                if state.aspect.is_none() && state.aspect_source == seed_source {
                                    state.aspect = Some(value);
                                    cx.notify();
                                }
                            });
                        }),
                ),
            );
            if self.shows_loader && aspect.is_none() {
                stack = stack.child(
                    pass_through()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(LoaderUi::new().size(LoaderUiSize::Large).color(Some(white()))),
                );
            }
        }
        let measured = state.clone();
        stack
            .clip_shape(Shape::continuous(self.corner_radius))
            .child(measure(self.child_id("geometry"), move |bounds: Bounds<Pixels>, _, cx| {
                measured.update(cx, |state, cx| {
                    if state.size != bounds.size {
                        state.size = bounds.size;
                        cx.notify();
                    }
                });
            }))
    }
}

/// published story frame is 9:16 so a canvas renders 1080 x 1920, such media is never cropped
pub enum StoryFrameAspect {}

impl StoryFrameAspect {
    const FRAME: f32 = 9.0 / 16.0;
    const TOLERANCE: f32 = 0.005;

    pub fn is_story_frame(aspect: f32) -> bool {
        (aspect - Self::FRAME).abs() < Self::TOLERANCE
    }
}

pub enum StoryMediaPrefetcher {}

const MAXIMUM_CONCURRENT_LOADS: usize = 2;

static PREFETCHED_VIDEO_URLS: LazyLock<NSCache<String, Url>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(12);
    cache
});

impl StoryMediaPrefetcher {
    pub fn reset_account_state() {
        PREFETCHED_VIDEO_URLS.remove_all_objects();
    }

    /// lane decides priority, the current story must not wait and the next few are a guess
    pub async fn prefetch(stories: &[StoryDTO], lane: CleverImageWorkLane, cx: &AsyncApp) {
        let count = usize::try_from(StoryPlaybackPolicy::PREFETCH_COUNT).unwrap_or(0);
        let candidates: Vec<&StoryDTO> = stories.iter().take(count).collect();
        if candidates.is_empty() {
            return;
        }
        for chunk in candidates.chunks(MAXIMUM_CONCURRENT_LOADS) {
            join_all(chunk.iter().map(|story| Self::prefetch_story(story, lane, cx))).await;
        }
    }

    pub fn video_cache_key(url: &Url) -> String {
        let components = UrlComponents::from_url(url, false).and_then(|mut components| {
            components.set_query(None);
            components.set_fragment(None);
            components.string()
        });
        format!("story-video-{}", components.unwrap_or_else(|| url.absolute_string().to_owned()))
    }

    pub fn prefetched_video_url(remote: &Url) -> Option<Url> {
        let key = Self::scoped_video_memory_key(
            remote,
            SaiAccountCacheScope::active_user_id().as_deref(),
            SaiAccountCacheScope::active_epoch().as_deref(),
        );
        let url = PREFETCHED_VIDEO_URLS.object(&key)?;
        if !url.to_file_path().is_some_and(|path| path.exists()) {
            PREFETCHED_VIDEO_URLS.remove_object(&key);
            return None;
        }
        Some(url)
    }

    fn scoped_video_memory_key(remote: &Url, account_user_id: Option<&str>, account_epoch: Option<&str>) -> String {
        format!(
            "{}|{}|{}",
            account_user_id.unwrap_or("anonymous"),
            account_epoch.unwrap_or("none"),
            remote.absolute_string()
        )
    }

    async fn prefetch_story(story: &StoryDTO, lane: CleverImageWorkLane, cx: &AsyncApp) {
        let source = story.media_source();
        if source.is_empty() {
            return;
        }

        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if story.media_type.to_lowercase() == MediaClass::Video.slug()
            && let Some(url) = Url::from_string(&source)
        {
            let options = RemoteMediaDiskCacheMaterializeOptions {
                cache_key: Some(Self::video_cache_key(&url)),
                suggested_filename: Some(url.last_path_component()),
                expected_account_user_id: account_user_id.clone(),
                expected_account_epoch: account_epoch.clone(),
                ..RemoteMediaDiskCacheMaterializeOptions::default()
            };
            let local = RemoteMediaDiskCache::shared().ensure_cached(&url, options, cx).await;
            if let Some(local_url) = local.as_deref().and_then(Url::from_file_path) {
                let key = Self::scoped_video_memory_key(&url, account_user_id.as_deref(), account_epoch.as_deref());
                PREFETCHED_VIDEO_URLS.set_object(local_url, key);
            }
            return;
        }

        let _ = CleverImageBitmapLoader::load(
            &source,
            true,
            true,
            account_user_id,
            account_epoch,
            Some(StoryPlaybackPolicy::IMAGE_MAXIMUM_PIXEL_DIMENSION),
            CleverImageCachePolicy::Standard,
            lane,
            cx,
        )
        .await;
    }
}

pub enum StoryImageAspectProbe {}

thread_local! {
    static ASPECT_CACHE: RefCell<HashMap<String, f32>> = RefCell::new(HashMap::new());
}

impl StoryImageAspectProbe {
    /// bitmap loader only resolves http so a local story cant probe itself
    /// seeding the picker size lays the pending story out like the committed one
    /// no reflow at commit
    pub fn seed(aspect: f32, source: &str) {
        let key = trimming_whitespaces_and_newlines(source).to_owned();
        if key.is_empty() || aspect <= 0.0 {
            return;
        }
        ASPECT_CACHE.with_borrow_mut(|cache| cache.insert(key, aspect));
    }

    /// true once the picture was decoded so the player may start its clock
    pub fn is_loaded(source: &str) -> bool {
        let key = trimming_whitespaces_and_newlines(source);
        ASPECT_CACHE.with_borrow(|cache| cache.contains_key(key))
    }

    pub fn copy_seed(source: &str, destination: &str) {
        let source_key = trimming_whitespaces_and_newlines(source);
        let destination_key = trimming_whitespaces_and_newlines(destination).to_owned();
        ASPECT_CACHE.with_borrow_mut(|cache| {
            let Some(value) = cache.get(source_key).copied() else {
                return;
            };
            if destination_key.is_empty() {
                return;
            }
            cache.insert(destination_key, value);
        });
    }

    pub fn aspect(source: &str, completion: impl FnOnce(Option<f32>, &mut App) + 'static, cx: &mut App) {
        let key = trimming_whitespaces_and_newlines(source).to_owned();
        if let Some(cached) = ASPECT_CACHE.with_borrow(|cache| cache.get(&key).copied()) {
            completion(Some(cached), cx);
            return;
        }
        if let Some(file_path) = Url::from_string(&key).filter(Url::is_file_url).and_then(|url| url.to_file_path()) {
            let read = cx.background_spawn(async move { Self::local_file_aspect(&file_path) });
            cx.spawn(async move |cx| {
                let value = read.await;
                cx.update(|cx| {
                    if let Some(value) = value {
                        ASPECT_CACHE.with_borrow_mut(|cache| cache.insert(key, value));
                    }
                    completion(value, cx);
                });
            })
            .detach();
            return;
        }
        cx.spawn(async move |cx| {
            let image = CleverImageBitmapLoader::load(
                &key,
                true,
                true,
                SaiAccountCacheScope::active_user_id(),
                SaiAccountCacheScope::active_epoch(),
                Some(StoryPlaybackPolicy::IMAGE_MAXIMUM_PIXEL_DIMENSION),
                CleverImageCachePolicy::Standard,
                CleverImageWorkLane::Visible,
                cx,
            )
            .await;
            let value = image.map(|image| {
                let size = image.size(0);
                size.width.0 as f32 / (size.height.0 as f32).max(1.0)
            });
            cx.update(|cx| {
                if let Some(value) = value {
                    ASPECT_CACHE.with_borrow_mut(|cache| cache.insert(key, value));
                }
                completion(value, cx);
            });
        })
        .detach();
    }

    fn local_file_aspect(path: &Path) -> Option<f32> {
        let mut decoder = ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
        let (width, height) = decoder.dimensions();
        if width == 0 || height == 0 {
            return None;
        }
        let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
        let is_quarter_turn = matches!(
            orientation,
            Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH
        );
        let (width, height) = (f64::from(width), f64::from(height));
        Some(if is_quarter_turn { (height / width) as f32 } else { (width / height) as f32 })
    }
}
