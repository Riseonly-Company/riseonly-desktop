use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{
    App, ElementId, Entity, Hsla, ImageSource, IntoElement, ParentElement, RenderImage, RenderOnce,
    SharedString, Size, Styled, StyledImage, Window, div, img, px,
};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::swiftui::alpha_shadow::AlphaShadowModifiers;
use rise_ui::swiftui::blur::blur_aware_image_source;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::progress_view::progress_view;

pub use rise_ui::swiftui::content_mode::ContentMode;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::ui::clever_image_disk_cache::{CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageWorkLane};
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;

const FAILURE_ICON_SIDE: f32 = 17.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CleverImageCachePolicy {
    #[default]
    Standard,
    MemoryOnly,
    PrivateEphemeral,
}

impl CleverImageCachePolicy {
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::MemoryOnly => "memoryOnly",
            Self::PrivateEphemeral => "privateEphemeral",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        match raw {
            "standard" => Some(Self::Standard),
            "memoryOnly" => Some(Self::MemoryOnly),
            "privateEphemeral" => Some(Self::PrivateEphemeral),
            _ => None,
        }
    }
}

/// radius per corner, leading and trailing as laid out left to right
/// radius goes on the picture and every placeholder fill since gpui clips a parent by its rect only
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CleverImageCorners {
    pub top_leading: f32,
    pub top_trailing: f32,
    pub bottom_leading: f32,
    pub bottom_trailing: f32,
}

impl CleverImageCorners {
    pub const fn uniform(radius: f32) -> Self {
        Self {
            top_leading: radius,
            top_trailing: radius,
            bottom_leading: radius,
            bottom_trailing: radius,
        }
    }

    pub const fn bottom(radius: f32) -> Self {
        Self {
            top_leading: 0.0,
            top_trailing: 0.0,
            bottom_leading: radius,
            bottom_trailing: radius,
        }
    }
}

trait CleverCornersStyled: Styled + Sized {
    fn clever_corners(self, corners: CleverImageCorners) -> Self {
        self.rounded_tl(px(corners.top_leading))
            .rounded_tr(px(corners.top_trailing))
            .rounded_bl(px(corners.bottom_leading))
            .rounded_br(px(corners.bottom_trailing))
    }
}

impl<E: Styled> CleverCornersStyled for E {}

/// size of the decoded picture in pixels
pub type CleverImageNaturalSize = Rc<dyn Fn(Size<f32>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct CleverImageUi {
    id: Option<ElementId>,
    source: SharedString,
    content_mode: ContentMode,
    corners: CleverImageCorners,
    persist_to_disk: bool,
    shows_progress_indicator: bool,
    maximum_decode_pixel_dimension: Option<i64>,
    cache_policy: CleverImageCachePolicy,
    shows_failure_icon: bool,
    placeholder_color: Option<Hsla>,
    on_natural_size: Option<CleverImageNaturalSize>,
}

impl CleverImageUi {
    pub fn new(source: impl Into<SharedString>) -> Self {
        Self {
            id: None,
            source: source.into(),
            content_mode: ContentMode::Fill,
            corners: CleverImageCorners::uniform(0.0),
            persist_to_disk: false,
            shows_progress_indicator: true,
            maximum_decode_pixel_dimension: Some(3_072),
            cache_policy: CleverImageCachePolicy::Standard,
            shows_failure_icon: true,
            placeholder_color: None,
            on_natural_size: None,
        }
    }

    /// element id the load state is kept under, the source when not set
    /// two images of one source under the same parent need their own ids
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn content_mode(mut self, content_mode: ContentMode) -> Self {
        self.content_mode = content_mode;
        self
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corners = CleverImageCorners::uniform(corner_radius);
        self
    }

    /// clips the image with per corner radii, a profile banner rounds its foot only
    pub fn corners(mut self, corners: CleverImageCorners) -> Self {
        self.corners = corners;
        self
    }

    pub fn persist_to_disk(mut self, persist_to_disk: bool) -> Self {
        self.persist_to_disk = persist_to_disk;
        self
    }

    pub fn shows_progress_indicator(mut self, shows_progress_indicator: bool) -> Self {
        self.shows_progress_indicator = shows_progress_indicator;
        self
    }

    pub fn maximum_decode_pixel_dimension(mut self, maximum: impl Into<Option<i64>>) -> Self {
        self.maximum_decode_pixel_dimension = maximum.into();
        self
    }

    pub fn cache_policy(mut self, cache_policy: CleverImageCachePolicy) -> Self {
        self.cache_policy = cache_policy;
        self
    }

    pub fn shows_failure_icon(mut self, shows_failure_icon: bool) -> Self {
        self.shows_failure_icon = shows_failure_icon;
        self
    }

    pub fn placeholder_color(mut self, placeholder_color: impl Into<Option<Hsla>>) -> Self {
        self.placeholder_color = placeholder_color.into();
        self
    }

    pub fn on_natural_size(mut self, on_natural_size: impl Fn(Size<f32>, &mut Window, &mut App) + 'static) -> Self {
        self.on_natural_size = Some(Rc::new(on_natural_size));
        self
    }

    fn local_file_url(source: &str) -> Option<PathBuf> {
        if source.starts_with("file://")
            && let Some(url) = Url::from_string(source)
        {
            return url.standardized().to_file_path();
        }
        if is_local_absolute_path(source) {
            return Some(PathBuf::from(source));
        }
        None
    }
}

impl RenderOnce for CleverImageUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (placeholder, failure_icon_color) = {
            let store = ThemeStore::shared(cx);
            let theme = store.read(cx).current_theme();
            (
                self.placeholder_color.unwrap_or(theme.bg._300),
                theme.text.secondary.opacity(0.6),
            )
        };
        let source = trimming_whitespaces_and_newlines(&self.source).to_owned();
        let id = self.id.clone().unwrap_or_else(|| ElementId::Name(self.source.clone()));
        let is_layout_measurement = is_message_layout_measurement(cx);

        let content = if is_layout_measurement {
            div().size_full().bg(placeholder).clever_corners(self.corners).into_any_element()
        } else if source.to_lowercase().starts_with("http") {
            CleverRemoteCachedImage {
                id,
                source,
                content_mode: self.content_mode,
                corners: self.corners,
                persist_to_disk: self.persist_to_disk,
                cache_policy: self.cache_policy,
                maximum_decode_pixel_dimension: self.maximum_decode_pixel_dimension,
                placeholder,
                failure_icon_color,
                shows_progress_indicator: self.shows_progress_indicator,
                shows_failure_icon: self.shows_failure_icon,
                on_natural_size: self.on_natural_size,
            }
            .render(window, cx)
        } else if let Some(file_url) = Self::local_file_url(&source) {
            LocalFileImageView {
                id,
                url: file_url,
                content_mode: self.content_mode,
                corners: self.corners,
                maximum_decode_pixel_dimension: self.maximum_decode_pixel_dimension,
                placeholder,
                on_natural_size: self.on_natural_size,
            }
            .render(window, cx)
        } else if source.starts_with("ph://") {
            PhotoAssetImageView {
                id,
                source,
                content_mode: self.content_mode,
                corners: self.corners,
                placeholder,
                on_natural_size: self.on_natural_size,
            }
            .render(window, cx)
        } else {
            div().size_full().bg(placeholder).clever_corners(self.corners).into_any_element()
        };

        div()
            .size_full()
            .flex()
            .clever_corners(self.corners)
            .overflow_hidden()
            .child(content)
    }
}

/// resizable image with the given content mode
/// radius sits on the image itself, gpui rounds a sprite by its own radii and never by a rounded parent
fn resizable_image(image: Arc<RenderImage>, content_mode: ContentMode, corners: CleverImageCorners) -> gpui::AnyElement {
    let natural = natural_size(&image);
    let sprite = img(blur_aware_image_source(ImageSource::Render(image.clone())))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .object_fit(content_mode.object_fit())
        .clever_corners(corners)
        .alpha_shadow_source(image, content_mode.object_fit());
    let mut spacer = div().size_full();
    if natural.width > 0.0 && natural.height > 0.0 {
        spacer = spacer.aspect_ratio(natural.width / natural.height);
    }
    div()
        .relative()
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .child(spacer)
        .child(sprite)
        .into_any_element()
}

fn natural_size(image: &RenderImage) -> Size<f32> {
    let size = image.size(0);
    Size {
        width: size.width.0 as f32,
        height: size.height.0 as f32,
    }
}

struct CleverRemoteCachedImage {
    id: ElementId,
    source: String,
    content_mode: ContentMode,
    corners: CleverImageCorners,
    persist_to_disk: bool,
    cache_policy: CleverImageCachePolicy,
    maximum_decode_pixel_dimension: Option<i64>,
    placeholder: Hsla,
    failure_icon_color: Hsla,
    shows_progress_indicator: bool,
    shows_failure_icon: bool,
    on_natural_size: Option<CleverImageNaturalSize>,
}

#[derive(Default)]
struct CleverRemoteCachedImageState {
    ui_image: Option<Arc<RenderImage>>,
    did_fail: bool,
    loaded_identity: String,
}

impl CleverRemoteCachedImage {
    fn render(self, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let identity = self.task_identity(account_user_id.as_deref(), account_epoch.as_deref());
        let immediate_image = CleverImageBitmapLoader::memory_image(
            &self.source,
            self.persist_to_disk,
            account_user_id.as_deref(),
            account_epoch.as_deref(),
            self.maximum_decode_pixel_dimension,
            self.cache_policy,
            None,
            false,
        );
        let state: Entity<CleverRemoteCachedImageState> = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), "clever-remote-state".into()),
            cx,
            |_, _| CleverRemoteCachedImageState::default(),
        );
        let (ui_image, did_fail, is_loaded_identity) = {
            let current = state.read(cx);
            (current.ui_image.clone(), current.did_fail, current.loaded_identity == identity)
        };

        let source = self.source.clone();
        let persist_to_disk = self.persist_to_disk;
        let maximum_pixel_dimension = self.maximum_decode_pixel_dimension;
        let cache_policy = self.cache_policy;
        let on_natural_size = self.on_natural_size.clone();
        let task_state = state.clone();
        let task_identity = identity.clone();
        let drawn_memory_image = immediate_image.clone().filter(|_| !(is_loaded_identity && ui_image.is_some()));
        use_task(
            ElementId::NamedChild(Arc::new(self.id.clone()), "clever-remote-task".into()),
            identity,
            window,
            cx,
            async move |cx| {
                let should_load = task_state.update(cx, |state, cx| {
                    if state.loaded_identity == task_identity && state.ui_image.is_some() {
                        return false;
                    }
                    if state.loaded_identity == task_identity {
                        return true;
                    }
                    let had_content = state.ui_image.is_some() || state.did_fail || !state.loaded_identity.is_empty();
                    if had_content {
                        tracing::warn!(
                            target: "riseonly::X1827",
                            "clever image identity changed old={} new={}",
                            state.loaded_identity,
                            task_identity
                        );
                    }
                    state.ui_image = None;
                    state.did_fail = false;
                    state.loaded_identity = String::new();
                    if had_content {
                        cx.notify();
                    }
                    true
                });
                if !should_load {
                    return;
                }
                let loaded = CleverImageBitmapLoader::load(
                    &source,
                    persist_to_disk,
                    true,
                    SaiAccountCacheScope::active_user_id(),
                    SaiAccountCacheScope::active_epoch(),
                    maximum_pixel_dimension,
                    cache_policy,
                    CleverImageWorkLane::Visible,
                    cx,
                )
                .await;
                let size = loaded.as_deref().map(natural_size);
                task_state.update(cx, |state, cx| {
                    let is_drawn = matches!(
                        (&loaded, &drawn_memory_image),
                        (Some(loaded), Some(drawn)) if Arc::ptr_eq(loaded, drawn) || loaded.id == drawn.id
                    );
                    let repeats_failure =
                        loaded.is_none() && state.did_fail && state.ui_image.is_none() && state.loaded_identity == task_identity;
                    state.did_fail = loaded.is_none();
                    state.ui_image = loaded;
                    state.loaded_identity = task_identity;
                    if !is_drawn && !repeats_failure {
                        cx.notify();
                    }
                });
                if let (Some(size), Some(on_natural_size)) = (size, on_natural_size) {
                    let _ = cx.update(|window, cx| on_natural_size(size, window, cx));
                }
            },
        );

        if is_loaded_identity && let Some(ui_image) = ui_image {
            resizable_image(ui_image, self.content_mode, self.corners)
        } else if let Some(immediate_image) = immediate_image {
            resizable_image(immediate_image, self.content_mode, self.corners)
        } else if is_loaded_identity && did_fail {
            let mut placeholder = div()
                .size_full()
                .bg(self.placeholder)
                .clever_corners(self.corners)
                .flex()
                .items_center()
                .justify_center();
            if self.shows_failure_icon
                && let Some(icon) = IconUi::sized("photo", px(FAILURE_ICON_SIDE), self.failure_icon_color, false)
            {
                placeholder = placeholder.child(icon);
            }
            placeholder.into_any_element()
        } else {
            let mut placeholder = div()
                .size_full()
                .bg(self.placeholder)
                .clever_corners(self.corners)
                .flex()
                .items_center()
                .justify_center();
            if self.shows_progress_indicator && !RiseTransportStatus::shown(cx).is_waiting_for_network() {
                placeholder = placeholder.child(div().child(progress_view().scale_effect(0.85)));
            }
            placeholder.into_any_element()
        }
    }

    fn task_identity(&self, account_user_id: Option<&str>, account_epoch: Option<&str>) -> String {
        format!(
            "{}|disk:{}|pixels:{}|policy:{}|account:{}|epoch:{}",
            self.source,
            self.persist_to_disk,
            self.maximum_decode_pixel_dimension.unwrap_or(0),
            self.cache_policy.raw_value(),
            account_user_id.unwrap_or("anonymous"),
            account_epoch.unwrap_or("none"),
        )
    }
}

static LOCAL_FILE_IMAGE_CACHE: LazyLock<NSCache<String, Arc<RenderImage>>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(80);
    cache.set_total_cost_limit(64 * 1024 * 1024);
    cache
});

enum LocalFileImageCache {}

impl LocalFileImageCache {
    fn image(url: &std::path::Path, maximum_pixel_dimension: Option<i64>) -> Option<Arc<RenderImage>> {
        let key = format!("{}|pixels:{}", url.display(), maximum_pixel_dimension.unwrap_or(0));
        if let Some(hit) = LOCAL_FILE_IMAGE_CACHE.object(&key) {
            return Some(hit);
        }
        let data = std::fs::read(url).ok()?;
        let loaded = CleverImageBitmapDecoder::decode(&data, maximum_pixel_dimension)?;
        let size = loaded.size(0);
        let cost = usize::try_from(size.width.0).unwrap_or(0)
            * usize::try_from(size.height.0).unwrap_or(0)
            * 4;
        LOCAL_FILE_IMAGE_CACHE.set_object_cost(loaded.clone(), key, cost);
        Some(loaded)
    }
}

struct LocalFileImageView {
    id: ElementId,
    url: PathBuf,
    content_mode: ContentMode,
    corners: CleverImageCorners,
    maximum_decode_pixel_dimension: Option<i64>,
    placeholder: Hsla,
    on_natural_size: Option<CleverImageNaturalSize>,
}

#[derive(Default)]
struct LocalFileImageState {
    ui_image: Option<Arc<RenderImage>>,
    loaded_path: String,
}

impl LocalFileImageView {
    fn render(self, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
        let path = format!("{}|pixels:{}", self.url.display(), self.maximum_decode_pixel_dimension.unwrap_or(0));
        let state: Entity<LocalFileImageState> = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), "clever-local-state".into()),
            cx,
            |_, _| LocalFileImageState::default(),
        );
        let ui_image = {
            let current = state.read(cx);
            if current.loaded_path == path { current.ui_image.clone() } else { None }
        };

        let url = self.url.clone();
        let maximum_pixel_dimension = self.maximum_decode_pixel_dimension;
        let on_natural_size = self.on_natural_size.clone();
        let task_path = path.clone();
        use_task(
            ElementId::NamedChild(Arc::new(self.id.clone()), "clever-local-task".into()),
            path,
            window,
            cx,
            async move |cx| {
                let loaded = cx
                    .background_executor()
                    .spawn(async move { LocalFileImageCache::image(&url, maximum_pixel_dimension) })
                    .await;
                let size = loaded.as_deref().map(natural_size);
                state.update(cx, |state, cx| {
                    state.ui_image = loaded;
                    state.loaded_path = task_path;
                    cx.notify();
                });
                if let (Some(size), Some(on_natural_size)) = (size, on_natural_size) {
                    let _ = cx.update(|window, cx| on_natural_size(size, window, cx));
                }
            },
        );

        match ui_image {
            Some(ui_image) => resizable_image(ui_image, self.content_mode, self.corners),
            None => div().size_full().bg(self.placeholder).clever_corners(self.corners).into_any_element(),
        }
    }
}

struct PhotoAssetImageView {
    id: ElementId,
    source: String,
    content_mode: ContentMode,
    corners: CleverImageCorners,
    placeholder: Hsla,
    on_natural_size: Option<CleverImageNaturalSize>,
}

#[derive(Default)]
struct PhotoAssetImageState {
    ui_image: Option<Arc<RenderImage>>,
    loaded_identity: String,
}

impl PhotoAssetImageView {
    fn render(self, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let identity = format!("{}|account:{}", self.source, account_user_id.as_deref().unwrap_or("anonymous"));
        let immediate_image = MediaViewerImageMemoryCache::memory_only(&self.source, account_user_id.as_deref());
        let state: Entity<PhotoAssetImageState> = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), "clever-asset-state".into()),
            cx,
            |_, _| PhotoAssetImageState::default(),
        );
        let ui_image = {
            let current = state.read(cx);
            if current.loaded_identity == identity { current.ui_image.clone() } else { None }
        };

        let source = self.source.clone();
        let on_natural_size = self.on_natural_size.clone();
        let task_identity = identity.clone();
        use_task(
            ElementId::NamedChild(Arc::new(self.id.clone()), "clever-asset-task".into()),
            identity,
            window,
            cx,
            async move |cx| {
                let loaded = MediaViewerBitmapLoader::load_into_memory(
                    &source,
                    false,
                    false,
                    CleverImageCachePolicy::MemoryOnly,
                    cx,
                )
                .await;
                let size = loaded.as_deref().map(natural_size);
                state.update(cx, |state, cx| {
                    state.ui_image = loaded;
                    state.loaded_identity = task_identity;
                    cx.notify();
                });
                if let (Some(size), Some(on_natural_size)) = (size, on_natural_size) {
                    let _ = cx.update(|window, cx| on_natural_size(size, window, cx));
                }
            },
        );

        match ui_image.or(immediate_image) {
            Some(ui_image) => resizable_image(ui_image, self.content_mode, self.corners),
            None => div().size_full().bg(self.placeholder).clever_corners(self.corners).into_any_element(),
        }
    }
}
