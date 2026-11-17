use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, Bounds, Corners, Edges, ElementId, Entity, ImageSource, IntoElement, ParentElement, Pixels, RenderOnce,
    SharedString, Size, Styled, StyledImage, Task, Window, black, div, hsla, img, point, px, size,
};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::swiftui::blur::{BlurModifiers, blur_aware_image_source};
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::environment::layout_direction;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::transition::TransitionModifiers;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::components::chat::bubble::bubble_corners::BubbleCorners;
use crate::modules::chat::components::chat::bubble::rise_native_message_image_grid::{
    RISE_NATIVE_MEDIA_BACKDROP_APPLIED, RiseNativeMediaThumbnailPipeline,
};

const FAILURE_ICON_SIDE: f32 = 17.0;

pub enum MediaAspectFit {}

impl MediaAspectFit {
    pub fn fitted_image_size(bounds: Size<Pixels>, aspect: f32) -> Size<Pixels> {
        let rw = f32::from(bounds.width).max(1.0);
        let rh = f32::from(bounds.height).max(1.0);
        let ar = aspect.max(0.2);
        let img_h_if_full_w = rw / ar;
        if img_h_if_full_w > rh {
            return size(px(rh * ar), px(rh));
        }
        size(px(rw), px(img_h_if_full_w))
    }

    pub fn centered_fit_rect_aspect(aspect: f32, in_size: Size<Pixels>) -> Bounds<Pixels> {
        Self::centered_fit_rect(size(px(aspect), px(1.0)), in_size, Edges::default())
    }

    /// centered fit rect for an image size inside a container with insets
    pub fn centered_fit_rect(image_size: Size<Pixels>, container: Size<Pixels>, insets: Edges<Pixels>) -> Bounds<Pixels> {
        let avail_w = (f32::from(container.width) - f32::from(insets.left) - f32::from(insets.right)).max(1.0);
        let avail_h = (f32::from(container.height) - f32::from(insets.top) - f32::from(insets.bottom)).max(1.0);
        let fit = Self::aspect_fit_size(image_size, size(px(avail_w), px(avail_h)));
        Bounds::new(
            point(
                px(f32::from(insets.left) + (avail_w - f32::from(fit.width)) / 2.0),
                px(f32::from(insets.top) + (avail_h - f32::from(fit.height)) / 2.0),
            ),
            fit,
        )
    }

    pub fn aspect_fit_size(image: Size<Pixels>, fitting: Size<Pixels>) -> Size<Pixels> {
        let (image_w, image_h) = (f32::from(image.width), f32::from(image.height));
        let (box_w, box_h) = (f32::from(fitting.width), f32::from(fitting.height));
        if !(image_w > 0.0 && image_h > 0.0 && box_w > 0.0 && box_h > 0.0) {
            return Size::default();
        }
        let scale = (box_w / image_w).min(box_h / image_h);
        size(px(image_w * scale), px(image_h * scale))
    }
}

/// where the letterbox photo gets its source from
#[derive(Clone, Debug, PartialEq)]
enum SourceKind {
    Remote,
    Local(PathBuf),
    Invalid,
}

struct MediaLetterboxPhotoState {
    loaded_image: Option<CleverImage>,
    loaded_identity: String,
    did_fail: bool,
}

/// photo fitted into a box with the letterbox bars filled by a blurred copy of itself
#[derive(IntoElement)]
pub struct MediaLetterboxPhoto {
    id: ElementId,
    source: SharedString,
    cached_image: Option<CleverImage>,
    r#box: Size<Pixels>,
    blur_opacity: f32,
    persist_to_disk: bool,
    cache_policy: CleverImageCachePolicy,
    shows_progress_indicator: bool,
}

impl MediaLetterboxPhoto {
    pub fn new(id: impl Into<ElementId>, source: impl Into<SharedString>, r#box: Size<Pixels>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            cached_image: None,
            r#box,
            blur_opacity: 1.0,
            persist_to_disk: false,
            cache_policy: CleverImageCachePolicy::Standard,
            shows_progress_indicator: true,
        }
    }

    pub fn cached_image(mut self, cached_image: Option<CleverImage>) -> Self {
        self.cached_image = cached_image;
        self
    }

    pub fn blur_opacity(mut self, blur_opacity: f32) -> Self {
        self.blur_opacity = blur_opacity;
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

    pub fn shows_progress_indicator(mut self, shows_progress_indicator: bool) -> Self {
        self.shows_progress_indicator = shows_progress_indicator;
        self
    }

    fn normalized_source(&self) -> String {
        trimming_whitespaces_and_newlines(&self.source).to_owned()
    }

    fn source_kind(&self) -> SourceKind {
        let value = self.normalized_source();
        if value.to_lowercase().starts_with("http") && Url::from_string(&value).is_some() {
            return SourceKind::Remote;
        }
        if value.starts_with("file://")
            && let Some(path) = Url::from_string(&value).and_then(|url| url.to_file_path())
        {
            return SourceKind::Local(path);
        }
        if is_local_absolute_path(&value) {
            return SourceKind::Local(PathBuf::from(value));
        }
        SourceKind::Invalid
    }

    fn backdrop_blur_radius(&self) -> f32 {
        let side = f32::from(self.r#box.width).min(f32::from(self.r#box.height));
        if !(side.is_finite() && side > 0.0) {
            return 16.0;
        }
        (side * 0.13).max(10.0).min(34.0)
    }

    /// max decode size, display scale is the scale factor of the window the photo is drawn in
    fn maximum_decode_pixel_dimension(&self, display_scale: f32) -> i64 {
        let largest_point_dimension = f32::from(self.r#box.width).max(f32::from(self.r#box.height));
        if !(largest_point_dimension.is_finite() && largest_point_dimension > 0.0) {
            return 1;
        }
        let display_scale = if display_scale.is_finite() { display_scale } else { 1.0 };
        let scale = display_scale.max(1.0).min(3.0);
        let pixels = (f64::from(largest_point_dimension) * f64::from(scale)).ceil();
        (pixels.min(f64::from(i32::MAX)) as i64).max(1)
    }

    fn image_identity(&self, display_scale: f32) -> String {
        let account = SaiAccountCacheScope::active_user_id().unwrap_or_else(|| "anonymous".to_owned());
        let epoch = SaiAccountCacheScope::active_epoch().unwrap_or_else(|| "none".to_owned());
        format!(
            "{}|disk:{}|pixels:{}|account:{}|epoch:{}",
            self.normalized_source(),
            self.persist_to_disk,
            self.maximum_decode_pixel_dimension(display_scale),
            account,
            epoch,
        )
    }

    fn task_identity(&self, display_scale: f32, is_layout_measurement: bool) -> String {
        format!(
            "{}|measurement:{}|policy:{}",
            self.image_identity(display_scale),
            is_layout_measurement,
            self.cache_policy.raw_value(),
        )
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

/// clips one element with per corner radii
fn rounded<E: Styled>(element: E, radii: Corners<Pixels>) -> E {
    element
        .rounded_tl(radii.top_left)
        .rounded_tr(radii.top_right)
        .rounded_bl(radii.bottom_left)
        .rounded_br(radii.bottom_right)
}

/// image element resized to the box with the given content mode
fn rendered_image(image: CleverImage, content_mode: ContentMode) -> gpui::AnyElement {
    img(blur_aware_image_source(ImageSource::Render(image)))
        .size_full()
        .object_fit(content_mode.object_fit())
        .into_any_element()
}

impl RenderOnce for MediaLetterboxPhoto {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let display_scale = window.scale_factor();
        let is_layout_measurement = is_message_layout_measurement(cx);
        let identity = self.image_identity(display_scale);
        let maximum_pixel_dimension = self.maximum_decode_pixel_dimension(display_scale);
        let normalized_source = self.normalized_source();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let has_cached_image = self.cached_image.is_some();
        let immediate_image = if has_cached_image {
            None
        } else {
            CleverImageBitmapLoader::memory_image(
                &normalized_source,
                self.persist_to_disk,
                account_user_id.as_deref(),
                account_epoch.as_deref(),
                Some(maximum_pixel_dimension),
                self.cache_policy,
                None,
                true,
            )
        };

        let initial_image = self.cached_image.clone();
        let state: Entity<MediaLetterboxPhotoState> =
            window.use_keyed_state(self.child_id("letterbox-state"), cx, move |_, _| MediaLetterboxPhotoState {
                loaded_image: initial_image,
                loaded_identity: String::new(),
                did_fail: false,
            });
        let (loaded_image, is_loaded_identity, did_fail) = {
            let current = state.read(cx);
            (current.loaded_image.clone(), current.loaded_identity == identity, current.did_fail)
        };
        let image = self
            .cached_image
            .clone()
            .or(immediate_image)
            .or(if is_loaded_identity { loaded_image } else { None });
        let (placeholder, failure_icon_color) = {
            let store = ThemeStore::shared(cx);
            let theme = store.read(cx).current_theme();
            (theme.bg._300, theme.text.secondary.opacity(0.6))
        };

        let task_state = state.clone();
        let task_identity = identity.clone();
        let task_cached_image = self.cached_image.clone();
        let task_source_kind = self.source_kind();
        let task_source = normalized_source.clone();
        let persist_to_disk = self.persist_to_disk;
        let cache_policy = self.cache_policy;
        if !has_cached_image {
            use_task(
                self.child_id("letterbox-task"),
                self.task_identity(display_scale, is_layout_measurement),
                window,
                cx,
                async move |cx| {
                    if is_layout_measurement {
                        return;
                    }
                    resolve_image(
                        task_state,
                        task_identity,
                        task_cached_image,
                        task_source_kind,
                        task_source,
                        persist_to_disk,
                        maximum_pixel_dimension,
                        cache_policy,
                        cx,
                    )
                    .await;
                },
            );
        }

        let r#box = self.r#box;
        let mut root = div()
            .relative()
            .w(r#box.width)
            .h(r#box.height)
            .overflow_hidden()
            .bg(black());

        if !is_layout_measurement
            && self.blur_opacity > 0.01
            && let Some(image) = image.clone()
        {
            let backdrop = div()
                .size_full()
                .child(rendered_image(image, ContentMode::Fill))
                .scale_effect(1.24)
                .blur_opaque(self.backdrop_blur_radius(), true);
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .overflow_hidden()
                    .opacity(0.55 * self.blur_opacity)
                    .child(backdrop),
            );
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(hsla(0.0, 0.0, 0.0, 0.2 * self.blur_opacity)),
            );
        }

        let foreground = match (is_layout_measurement, image) {
            (false, Some(image)) => rendered_image(image, ContentMode::Fit),
            _ => {
                let mut placeholder_view = div()
                    .size_full()
                    .bg(placeholder)
                    .flex()
                    .items_center()
                    .justify_center();
                if !is_layout_measurement && self.source_kind() == SourceKind::Remote {
                    if is_loaded_identity && did_fail {
                        if let Some(icon) = IconUi::sized("photo", px(FAILURE_ICON_SIDE), failure_icon_color, false) {
                            placeholder_view = placeholder_view.child(icon);
                        }
                    } else if self.shows_progress_indicator {
                        placeholder_view = placeholder_view.child(div().child(progress_view().scale_effect(0.85)));
                    }
                }
                placeholder_view.into_any_element()
            }
        };

        root.child(
            div()
                .absolute()
                .inset_0()
                .w(r#box.width)
                .h(r#box.height)
                .child(foreground),
        )
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_image(
    state: Entity<MediaLetterboxPhotoState>,
    identity: String,
    cached_image: Option<CleverImage>,
    source_kind: SourceKind,
    source: String,
    persist_to_disk: bool,
    maximum_pixel_dimension: i64,
    cache_policy: CleverImageCachePolicy,
    cx: &mut gpui::AsyncWindowContext,
) {
    if let Some(cached_image) = cached_image {
        state.update(cx, |state, cx| {
            state.loaded_image = Some(cached_image);
            state.loaded_identity = identity;
            state.did_fail = false;
            cx.notify();
        });
        return;
    }
    let should_load = state.update(cx, |state, cx| {
        if state.loaded_identity == identity && !(state.loaded_image.is_none() && !state.did_fail) {
            return false;
        }
        state.loaded_image = None;
        state.loaded_identity = String::new();
        state.did_fail = false;
        cx.notify();
        true
    });
    if !should_load {
        return;
    }

    let image = match source_kind {
        SourceKind::Remote => {
            CleverImageBitmapLoader::load(
                &source,
                persist_to_disk,
                true,
                SaiAccountCacheScope::active_user_id(),
                SaiAccountCacheScope::active_epoch(),
                Some(maximum_pixel_dimension),
                cache_policy,
                CleverImageWorkLane::Visible,
                cx,
            )
            .await
        }
        SourceKind::Local(path) => {
            let decode: Task<Option<CleverImage>> = cx.background_executor().spawn(async move {
                let data = std::fs::read(&path).ok()?;
                CleverImageBitmapDecoder::decode(&data, Some(maximum_pixel_dimension))
            });
            decode.await
        }
        SourceKind::Invalid => None,
    };

    state.update(cx, |state, cx| {
        state.did_fail = image.is_none();
        state.loaded_image = image;
        state.loaded_identity = identity;
        cx.notify();
    });
}

/// state of the media side fill plus the observer of the tile backdrop event
struct MessageMediaSideFillState {
    source: String,
    video_source: String,
    image: Option<CleverImage>,
    image_source: Option<String>,
    _observer: Option<RiseSubscription>,
    _listener: Option<Task<()>>,
}

impl MessageMediaSideFillState {
    /// one cache read, a miss keeps the previous fill
    /// so the bubble doesnt flash flat when the upload replaces the local preview
    fn resolve(&mut self) -> bool {
        let Some(cached) = MessageMediaSideFill::lookup(&self.source, &self.video_source) else {
            return false;
        };
        self.image = Some(cached);
        self.image_source = Some(self.source.clone());
        true
    }
}

/// blurred copy filling a bubble beside media too narrow to reach its edges
/// reuses the tile fill, loads nothing
/// until the tile has it the bubble keeps its own color
#[derive(IntoElement)]
pub struct MessageMediaSideFill {
    id: ElementId,
    source: String,
    video_source: String,
    corners: BubbleCorners,
}

impl MessageMediaSideFill {
    pub fn new(id: impl Into<ElementId>, source: impl Into<String>, corners: BubbleCorners) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            video_source: String::new(),
            corners,
        }
    }

    pub fn video_source(mut self, video_source: impl Into<String>) -> Self {
        self.video_source = video_source.into();
        self
    }

    fn lookup(source: &str, video_source: &str) -> Option<CleverImage> {
        RiseNativeMediaThumbnailPipeline::cached_backdrop(
            source,
            video_source,
            SaiAccountCacheScope::active_user_id().as_deref(),
            SaiAccountCacheScope::active_epoch().as_deref(),
        )
    }
}

impl RenderOnce for MessageMediaSideFill {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = self.source.clone();
        let video_source = self.video_source.clone();
        let state: Entity<MessageMediaSideFillState> = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("side-fill-state")),
            cx,
            move |_, cx| {
                let mut state = MessageMediaSideFillState {
                    source,
                    video_source,
                    image: None,
                    image_source: None,
                    _observer: None,
                    _listener: None,
                };
                state.resolve();
                let (sender, mut receiver) = mpsc::unbounded::<()>();
                state._observer = Some(RiseAppEvents::default_center().observe(
                    RISE_NATIVE_MEDIA_BACKDROP_APPLIED,
                    move |_| {
                        let _ = sender.unbounded_send(());
                    },
                ));
                state._listener = Some(cx.spawn(async move |this, cx| {
                    while receiver.next().await.is_some() {
                        let updated = this.update(cx, |state: &mut MessageMediaSideFillState, cx| {
                            if state.image_source.as_deref() == Some(state.source.as_str()) {
                                return;
                            }
                            if state.resolve() {
                                cx.notify();
                            }
                        });
                        if updated.is_err() {
                            break;
                        }
                    }
                }));
                state
            },
        );

        state.update(cx, |state, _| {
            state.video_source = self.video_source.clone();
            if state.source != self.source {
                state.source = self.source.clone();
                state.resolve();
            }
        });

        let radii = self.corners.uneven_shape(layout_direction(cx));
        let mut fill = rounded(div().relative().size_full().overflow_hidden(), radii);
        if let Some(image) = state.read(cx).image.clone() {
            let picture = rounded(
                img(blur_aware_image_source(ImageSource::Render(image)))
                    .size_full()
                    .object_fit(ContentMode::Fill.object_fit()),
                radii,
            );
            fill = fill
                .child(div().absolute().inset_0().child(picture.blur_opaque(22.0, true)))
                .child(
                    rounded(div().absolute().inset_0(), radii).bg(hsla(0.0, 0.0, 0.0, 0.24)),
                );
        }
        fill
    }
}
