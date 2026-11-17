use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use futures::lock::Mutex as AsyncMutex;
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, Context, Element, ElementId, Entity, GlobalElementId,
    ImageSource, InspectorElementId, IntoElement, LayoutId, ObjectFit, ParentElement, Pixels, RenderImage,
    RenderOnce, Size, Styled, StyledImage, Subscription, Task, WeakEntity, Window, canvas, div, img, point, px,
    size,
};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, Frame, Frames, ImageDecoder, ImageFormat, RgbaImage};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::rise_upload::rise_url_session::URLRequest;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_ui::animated_media_playback_scope::{
    ANIMATED_MEDIA_PLAYBACK_SCOPE_DID_CHANGE, AnimatedMediaPausesUnderMediaViewerKey, AnimatedMediaViewportPlaybackPolicy,
    is_animated_media_playback_active,
};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::alpha_shadow::AlphaShadowModifiers;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use smallvec::SmallVec;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDownloadProgress, RemoteMediaDownloadRetryPolicy,
    RemoteMediaHttpResponse,
};
use crate::core::media::rise_progressive_data_fetcher::{
    RiseProgressiveDataFetcher, RiseProgressiveDataFetcherLane,
};
use crate::core::ui::animated_gif_view_hooks::animated_gif_view_hooks;
use crate::core::ui::animated_media_playback_coordinator::{
    AnimatedMediaParticipantId, AnimatedMediaPlaybackCoordinator, AnimatedMediaPlaybackKind,
    AnimatedMediaPlaybackPolicy,
};
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;

const GIF_FRAME_APPLY_OPERATION: &str = "animated_media.gif_frame_apply";
const GIF_FRAME_APPLY_THRESHOLD_MS: f64 = 4.0;
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const REMOTE_FAILURE_RETRY_DELAY: f64 = 3.0;
const ACCEPT_IMAGES: &str = "image/*";
const NO_STORE: &str = "no-store";
const FRAME_DECODE_QUEUE_LABEL: &str = "riseonly.animated-image.frames";
const REJECTED_CONTENT_TYPES: [&str; 4] =
    ["text/html", "application/json", "application/xml", "text/xml"];

pub enum AnimatedImagePlaybackPolicy {}

impl AnimatedImagePlaybackPolicy {
    pub const MINIMUM_FRAME_DURATION: f64 = 1.0 / 30.0;
    pub const DEFAULT_FRAME_DURATION: f64 = 0.10;

    pub fn maximum_visible_players() -> i64 {
        AnimatedMediaPlaybackPolicy::nominal_budget()
    }

    pub fn normalized_frame_duration(value: Option<f64>) -> f64 {
        let Some(value) = value.filter(|value| value.is_finite() && *value > 0.0) else {
            return Self::DEFAULT_FRAME_DURATION;
        };
        value.clamp(Self::MINIMUM_FRAME_DURATION, 1.0)
    }
}

/// where the bytes of an animation come from
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimatedImageSourcePolicySource {
    LocalFile(PathBuf),
    Remote,
}

pub enum AnimatedImageSourcePolicy {}

impl AnimatedImageSourcePolicy {
    const PAYLOAD_KEY_PREFIX: &'static str = "animated-image-payload:";

    pub fn payload_cache_key(url: &str) -> String {
        format!(
            "{}{}",
            Self::PAYLOAD_KEY_PREFIX,
            CleverImageDiskCache::normalized_cache_key(url)
        )
    }

    /// source key an animation payload belongs to, none for any other cache key
    pub fn source_key(of_payload_key: &str) -> Option<String> {
        of_payload_key
            .strip_prefix(Self::PAYLOAD_KEY_PREFIX)
            .map(str::to_owned)
    }

    pub fn is_complete_animation(data: &[u8]) -> bool {
        let Some(mut source) = AnimatedFrameSource::open(Arc::from(data)) else {
            return false;
        };
        source
            .frame_durations(&FrameDecodeToken::new())
            .is_some_and(|durations| durations.len() > 1)
    }

    pub fn resolve(raw_value: &str) -> Option<AnimatedImageSourcePolicySource> {
        let value = trimming_whitespaces_and_newlines(raw_value);
        if value.is_empty() {
            return None;
        }
        if is_local_absolute_path(value) {
            return Some(AnimatedImageSourcePolicySource::LocalFile(standardized_path(Path::new(
                value,
            ))));
        }
        let url = Url::from_string(value)?;
        let scheme = url.scheme()?.to_lowercase();
        match scheme.as_str() {
            "file" => {
                let path = url.to_file_path()?;
                Some(AnimatedImageSourcePolicySource::LocalFile(standardized_path(&path)))
            }
            "http" | "https" => Some(AnimatedImageSourcePolicySource::Remote),
            _ => None,
        }
    }

    pub fn mapped_local_data(file_url: &Path, maximum_payload_bytes: i64) -> Option<Vec<u8>> {
        if maximum_payload_bytes <= 0 {
            return None;
        }
        let metadata = std::fs::metadata(file_url).ok()?;
        let file_size = i64::try_from(metadata.len()).ok()?;
        if !metadata.is_file() || file_size <= 0 || file_size > maximum_payload_bytes {
            return None;
        }
        let data = std::fs::read(file_url).ok()?;
        let length = i64::try_from(data.len()).ok()?;
        (!data.is_empty() && length <= maximum_payload_bytes).then_some(data)
    }
}

/// resolves . and .. in a path by text only, never touches the disk
fn standardized_path(path: &Path) -> PathBuf {
    let mut standardized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                standardized.pop();
            }
            other => standardized.push(other.as_os_str()),
        }
    }
    standardized
}

/// content mode as the host passes it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AnimatedGifContentMode {
    #[default]
    ScaleAspectFill,
    ScaleAspectFit,
    ScaleToFill,
}

impl AnimatedGifContentMode {
    fn object_fit(self) -> ObjectFit {
        match self {
            Self::ScaleAspectFill => ObjectFit::Cover,
            Self::ScaleAspectFit => ObjectFit::Contain,
            Self::ScaleToFill => ObjectFit::Fill,
        }
    }

    fn fills_box(self) -> bool {
        matches!(self, Self::ScaleAspectFill | Self::ScaleToFill)
    }
}

pub type AnimatedGifBackdropHandler = Rc<dyn Fn(Arc<RenderImage>, &mut App)>;

#[derive(IntoElement)]
pub struct AnimatedGifView {
    id: ElementId,
    pub url: String,
    pub content_mode: AnimatedGifContentMode,
    pub is_playing: bool,
    pub playback_kind: AnimatedMediaPlaybackKind,
    pub persist_to_disk: bool,
    pub cache_policy: CleverImageCachePolicy,
    pub on_backdrop_image: Option<AnimatedGifBackdropHandler>,
    pub releases_hidden_frames: bool,
}

impl AnimatedGifView {
    const MAXIMUM_PAYLOAD_BYTES: i64 = 64 * 1_024 * 1_024;

    pub fn new(id: impl Into<ElementId>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            content_mode: AnimatedGifContentMode::default(),
            is_playing: true,
            playback_kind: AnimatedMediaPlaybackKind::AnimatedImage,
            persist_to_disk: true,
            cache_policy: CleverImageCachePolicy::Standard,
            on_backdrop_image: None,
            releases_hidden_frames: false,
        }
    }

    pub fn content_mode(mut self, content_mode: AnimatedGifContentMode) -> Self {
        self.content_mode = content_mode;
        self
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn playback_kind(mut self, playback_kind: AnimatedMediaPlaybackKind) -> Self {
        self.playback_kind = playback_kind;
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

    pub fn on_backdrop_image(mut self, handler: impl Fn(Arc<RenderImage>, &mut App) + 'static) -> Self {
        self.on_backdrop_image = Some(Rc::new(handler));
        self
    }

    pub fn releases_hidden_frames(mut self, releases_hidden_frames: bool) -> Self {
        self.releases_hidden_frames = releases_hidden_frames;
        self
    }
}

impl AnimatedGifView {
    fn make_ui_view(&self, window: &mut Window, cx: &mut App) -> Entity<AnimatedGifImageView> {
        let content_mode = self.content_mode;
        window.use_keyed_state((self.id.clone(), "image-view"), cx, move |window, cx| {
            let mut view = AnimatedGifImageView::new(content_mode);
            view.did_move_to_window(window, cx);
            view
        })
    }

    fn make_coordinator(&self, window: &mut Window, cx: &mut App) -> Entity<AnimatedGifViewCoordinator> {
        window.use_keyed_state((self.id.clone(), "coordinator"), cx, |_, _| {
            AnimatedGifViewCoordinator::new()
        })
    }

    fn update_ui_view(
        &self,
        ui_view: &Entity<AnimatedGifImageView>,
        coordinator: &Entity<AnimatedGifViewCoordinator>,
        is_playback_scope_active: bool,
        cx: &mut App,
    ) {
        let content_mode = self.content_mode;
        ui_view.update(cx, |view, _| view.content_mode = content_mode);
        coordinator.update(cx, |coordinator, cx| {
            coordinator.playback_kind = self.playback_kind;
            coordinator.set_releases_hidden_frames(self.releases_hidden_frames, cx);
            coordinator.update(
                &self.url,
                self.is_playing && is_playback_scope_active,
                self.persist_to_disk,
                self.cache_policy,
                self.on_backdrop_image.clone(),
                cx,
            );
        });
    }

    pub fn size_that_fits(proposal: Size<Option<Pixels>>, ui_view: &AnimatedGifImageView) -> Size<Pixels> {
        let intrinsic = ui_view
            .image
            .as_ref()
            .map(|image| {
                let pixels = image.size(0);
                size(px(pixels.width.0 as f32), px(pixels.height.0 as f32))
            })
            .unwrap_or_else(|| size(px(10.), px(10.)));
        size(
            proposal.width.unwrap_or(intrinsic.width),
            proposal.height.unwrap_or(intrinsic.height),
        )
    }

    fn dismantle_ui_view(
        ui_view: &Entity<AnimatedGifImageView>,
        coordinator: &Entity<AnimatedGifViewCoordinator>,
        cx: &mut App,
    ) {
        coordinator.update(cx, |coordinator, cx| coordinator.teardown(ui_view, cx));
        ui_view.update(cx, |view, cx| view.set_image(None, cx));
    }
}

impl RenderOnce for AnimatedGifView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let pauses_under_media_viewer = env::<AnimatedMediaPausesUnderMediaViewerKey>(cx);
        let coordinator = self.make_coordinator(window, cx);
        let ui_view = self.make_ui_view(window, cx);
        let is_covered_by_media_viewer = ui_view
            .update(cx, |view, cx| view.follow_media_viewer_cover(pauses_under_media_viewer && self.is_playing, cx));
        let is_playback_scope_active = is_animated_media_playback_active(cx) && !is_covered_by_media_viewer;
        if coordinator.read(cx).image_view.is_none() {
            coordinator.update(cx, |coordinator, cx| coordinator.bind(&ui_view, cx));
        }
        self.update_ui_view(&ui_view, &coordinator, is_playback_scope_active, cx);
        let waits_for_scope = self.is_playing && !is_playback_scope_active;
        ui_view.update(cx, |view, cx| view.set_waits_for_playback_scope(waits_for_scope, cx));

        let swap_window = window.window_handle();
        let swap_key = (ui_view.entity_id().as_u64() as u32).wrapping_add(1);
        ui_view.update(cx, |view, _| view.bind_swap(swap_window, swap_key));
        let draw = window.draw_count();
        if ui_view.update(cx, |view, _| view.note_render(draw)) {
            let unpainted = ui_view.clone();
            window.defer(cx, move |_window, cx| {
                unpainted.update(cx, |view, cx| view.hide_if_unpainted(draw, cx));
            });
        }
        let image = ui_view.read(cx).image.clone();
        let probe = ui_view.clone();
        let replay_probe = ui_view.clone();
        let visibility = canvas(
            move |bounds, window, _cx| {
                let mask = window.content_mask().bounds;
                let moved_probe = replay_probe.clone();
                window.on_replayed_move(move |offset, mask_offset, window, cx| {
                    let draw = window.draw_count();
                    moved_probe.update(cx, |view, _| view.painted_draw = draw);
                    let measured = VisibilityProbe::measure_at(
                        bounds + offset,
                        mask + mask_offset,
                        window.viewport_size(),
                        is_playback_scope_active,
                    );
                    let scale = window.scale_factor();
                    let probe = moved_probe.clone();
                    window.defer(cx, move |_window, cx| {
                        probe.update(cx, |view, cx| {
                            view.layout_subviews(bounds.size, scale, measured, cx);
                            view.render_clipped_frame_if_pending(cx);
                        });
                    });
                });
                VisibilityProbe::measure_at(bounds, mask, window.viewport_size(), is_playback_scope_active)
            },
            move |bounds, measured, window, cx| {
                let scale = window.scale_factor();
                let draw = window.draw_count();
                probe.update(cx, |view, _| {
                    view.painted_draw = draw;
                    view.clipped_frame_pending = false;
                });
                window.defer(cx, move |_window, cx| {
                    probe.update(cx, |view, cx| view.layout_subviews(bounds.size, scale, measured, cx));
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        div()
            .relative()
            .overflow_hidden()
            .size_full()
            .children(image.map(|image| GifFrameSwapKeyed {
                key: swap_key,
                child: img(ImageSource::Render(image.clone()))
                    .size_full()
                    .object_fit(self.content_mode.object_fit())
                    .alpha_shadow_source(image, self.content_mode.object_fit())
                    .into_any_element(),
            }))
            .child(visibility)
            .on_disappear((self.id, "animated-gif-view"), move |_window, cx| {
                Self::dismantle_ui_view(&ui_view, &coordinator, cx);
            })
    }
}

pub type VisibilityChanged = Rc<dyn Fn(bool, &mut App)>;
pub type LeftViewport = Rc<dyn Fn(&mut App)>;

struct GifFrameSwapKeyed {
    key: u32,
    child: AnyElement,
}

impl IntoElement for GifFrameSwapKeyed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GifFrameSwapKeyed {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_image_swap_key(self.key, |window| self.child.paint(window, cx));
    }
}

/// image view the frames are drawn into, with visibility tracking
/// every paint reports the clipped frame
pub struct AnimatedGifImageView {
    swap_window: Option<AnyWindowHandle>,
    swap_key: u32,
    pub image: Option<Arc<RenderImage>>,
    pub content_mode: AnimatedGifContentMode,
    pub visibility_changed: Option<VisibilityChanged>,
    pub left_viewport: Option<LeftViewport>,
    reports_viewport_exit: bool,
    last_reported_visibility: Option<bool>,
    is_outside_window: bool,
    last_probe: Option<VisibilityProbe>,
    frame_probe: Option<VisibilityProbe>,
    rendered_draw: u64,
    painted_draw: u64,
    bounds_size: Size<Pixels>,
    display_scale: f32,
    scene_observers: Vec<Subscription>,
    unpainted_miss_draw: Option<u64>,
    offscreen_poll: Option<Task<()>>,
    clipped_frame_pending: bool,
    scope_resume_observer: Option<(RiseSubscription, Task<()>)>,
    media_viewer_cover_observer: Option<Subscription>,
    is_covered_by_media_viewer: bool,
}

impl AnimatedGifImageView {
    fn new(content_mode: AnimatedGifContentMode) -> Self {
        Self {
            swap_window: None,
            swap_key: 0,
            image: None,
            content_mode,
            visibility_changed: None,
            left_viewport: None,
            reports_viewport_exit: false,
            last_reported_visibility: None,
            is_outside_window: false,
            last_probe: None,
            frame_probe: None,
            rendered_draw: 0,
            painted_draw: 0,
            bounds_size: Size::default(),
            display_scale: 1.0,
            scene_observers: Vec::new(),
            unpainted_miss_draw: None,
            offscreen_poll: None,
            clipped_frame_pending: false,
            scope_resume_observer: None,
            media_viewer_cover_observer: None,
            is_covered_by_media_viewer: false,
        }
    }

    fn follow_media_viewer_cover(&mut self, follows: bool, cx: &mut Context<Self>) -> bool {
        if !follows {
            self.media_viewer_cover_observer = None;
            self.is_covered_by_media_viewer = false;
            return false;
        }
        if self.media_viewer_cover_observer.is_none() {
            let Some(hooks) = animated_gif_view_hooks() else {
                return self.is_covered_by_media_viewer;
            };
            let observer = cx.entity().downgrade();
            self.media_viewer_cover_observer = Some((hooks.observe_media_viewer_cover)(
                Box::new(move |covers: bool, cx: &mut App| {
                    if let Some(observer) = observer.upgrade() {
                        observer.update(cx, |view, cx| view.apply_media_viewer_cover(covers, cx));
                    }
                }),
                cx,
            ));
            let this = cx.entity().downgrade();
            cx.defer(move |cx| {
                let covers = (hooks.media_viewer_covers_page)(cx);
                if let Some(this) = this.upgrade() {
                    this.update(cx, |view, cx| view.apply_media_viewer_cover(covers, cx));
                }
            });
        }
        self.is_covered_by_media_viewer
    }

    fn apply_media_viewer_cover(&mut self, covers: bool, cx: &mut Context<Self>) {
        if self.media_viewer_cover_observer.is_some() && covers != self.is_covered_by_media_viewer {
            self.is_covered_by_media_viewer = covers;
            cx.notify();
        }
    }

    fn set_waits_for_playback_scope(&mut self, waits: bool, cx: &mut Context<Self>) {
        if waits == self.scope_resume_observer.is_some() {
            return;
        }
        if !waits {
            self.scope_resume_observer = None;
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let subscription = RiseAppEvents::default_center().observe(ANIMATED_MEDIA_PLAYBACK_SCOPE_DID_CHANGE, move |_| {
            let _ = sender.unbounded_send(());
        });
        let task = cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        });
        self.scope_resume_observer = Some((subscription, task));
    }

    pub fn reports_viewport_exit(&self) -> bool {
        self.reports_viewport_exit
    }

    pub fn set_reports_viewport_exit(&mut self, reports_viewport_exit: bool) {
        self.reports_viewport_exit = reports_viewport_exit;
        if !reports_viewport_exit {
            self.is_outside_window = false;
        }
    }

    /// true when drawn in a window, in an active playback scope
    /// and enough of its height shows inside every clipping parent, as of the last paint
    pub fn is_effectively_visible(&self) -> bool {
        self.last_probe.is_some_and(|probe| probe.is_visible)
    }

    pub fn bounds_size(&self) -> Size<Pixels> {
        self.bounds_size
    }

    pub fn display_scale(&self) -> f32 {
        self.display_scale
    }

    fn did_move_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_scene_subscription(window, cx);
        self.report_visibility_if_changed(cx);
    }

    fn did_move_to_superview(&mut self, cx: &mut Context<Self>) {
        self.report_visibility_if_changed(cx);
    }

    fn layout_subviews(
        &mut self,
        bounds_size: Size<Pixels>,
        display_scale: f32,
        probe: VisibilityProbe,
        cx: &mut Context<Self>,
    ) {
        self.bounds_size = bounds_size;
        self.display_scale = display_scale;
        let pending = self.frame_probe.is_some();
        self.frame_probe = Some(match self.frame_probe {
            Some(merged) => merged.merged(probe),
            None => probe,
        });
        if pending {
            return;
        }
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            this.update(cx, |this, cx| this.flush_frame_probe(cx)).ok();
        });
    }

    fn flush_frame_probe(&mut self, cx: &mut Context<Self>) {
        let Some(probe) = self.frame_probe.take() else {
            return;
        };
        if !probe.is_visible && self.last_reported_visibility == Some(true) {
            tracing::warn!(target: "riseonly::X960", bounds = ?probe.bounds, clip = ?probe.clip, window = ?probe.window, scope = probe.is_playback_scope_active, outside = probe.is_outside_window, "gif probe hidden");
        }
        let first_report = self.last_probe.is_none();
        self.unpainted_miss_draw = None;
        self.offscreen_poll = None;
        self.last_probe = Some(probe);
        if first_report {
            self.did_move_to_superview(cx);
        } else {
            self.report_visibility_if_changed(cx);
        }
    }

    fn note_render(&mut self, draw: u64) -> bool {
        if self.rendered_draw == draw {
            return false;
        }
        self.rendered_draw = draw;
        self.last_reported_visibility == Some(true)
    }

    fn hide_if_unpainted(&mut self, draw: u64, cx: &mut Context<Self>) {
        if self.painted_draw == draw || self.last_reported_visibility != Some(true) {
            return;
        }
        let Some(last) = self.last_probe else {
            return;
        };
        if let Some(image) = self.image.clone()
            && self.swap_frame_in_place(&image, &image, cx)
        {
            return;
        }
        self.last_probe = Some(VisibilityProbe { is_visible: false, is_outside_window: false, ..last });
        self.report_visibility_if_changed(cx);
    }

    pub fn stop_visibility_tracking(&mut self) {
        self.scene_observers.clear();
        self.last_reported_visibility = None;
        self.frame_probe = None;
    }

    fn refresh_scene_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.scene_observers.clear();
        self.scene_observers.push(cx.observe_window_activation(window, |this, _window, cx| {
            this.report_visibility_if_changed(cx);
            cx.notify();
        }));
    }

    fn report_visibility_if_changed(&mut self, cx: &mut Context<Self>) {
        let Some(probe) = self.last_probe else {
            return;
        };
        let visible = probe.is_visible;
        if Some(visible) != self.last_reported_visibility {
            self.last_reported_visibility = Some(visible);
            if let Some(visibility_changed) = self.visibility_changed.clone() {
                visibility_changed(visible, cx);
            }
        }
        self.report_viewport_exit_if_needed(probe, cx);
    }

    fn report_viewport_exit_if_needed(&mut self, probe: VisibilityProbe, cx: &mut Context<Self>) {
        if !self.reports_viewport_exit {
            return;
        }
        let outside = !probe.is_visible && probe.is_outside_window;
        if outside == self.is_outside_window {
            return;
        }
        self.is_outside_window = outside;
        if outside && let Some(left_viewport) = self.left_viewport.clone() {
            left_viewport(cx);
        }
    }

    fn set_image(&mut self, image: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        let unchanged = match (&self.image, &image) {
            (None, None) => true,
            (Some(current), Some(next)) => Arc::ptr_eq(current, next),
            _ => false,
        };
        if unchanged {
            return;
        }
        let replaces_frame = self.image.is_some() && image.is_some();
        let swapped = match (&self.image, &image) {
            (Some(previous), Some(next)) => self.swap_frame_in_place(previous, next, cx),
            _ => false,
        };
        let offscreen = !swapped && replaces_frame && self.missed_frame_is_offscreen(cx);
        let clipped = !swapped && replaces_frame && !offscreen && self.painted_without_sprite(cx);
        if let Some(previous) = std::mem::replace(&mut self.image, image) {
            cx.drop_image(previous, None);
        }
        if offscreen {
            let this = cx.weak_entity();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| this.hide_offscreen(cx)).ok();
            });
            return;
        }
        if clipped {
            if !self.clipped_frame_pending {
                tracing::warn!(target: "riseonly::X1817", image_view = ?cx.entity_id(), key = self.swap_key, "gif frame held: painted with no sprite");
            }
            self.clipped_frame_pending = true;
            return;
        }
        if !swapped {
            if replaces_frame {
                let carries_key = self.frame_on_screen_carries_key(cx).map(|(carries_key, _)| carries_key);
                tracing::warn!(target: "riseonly::X960", image_view = ?cx.entity_id(), key = self.swap_key, ?carries_key, painted = self.painted_draw, "gif frame swap missed");
            }
            self.clipped_frame_pending = false;
            cx.notify();
        }
    }

    fn painted_without_sprite(&self, cx: &mut Context<Self>) -> bool {
        if self.last_reported_visibility != Some(true) {
            return false;
        }
        matches!(self.frame_on_screen_carries_key(cx), Some((false, draw)) if self.painted_draw >= draw)
    }

    fn render_clipped_frame_if_pending(&mut self, cx: &mut Context<Self>) {
        if self.clipped_frame_pending {
            self.clipped_frame_pending = false;
            cx.notify();
        }
    }

    fn missed_frame_is_offscreen(&mut self, cx: &mut Context<Self>) -> bool {
        if self.last_reported_visibility != Some(true) {
            return false;
        }
        let Some((carries_key, draw)) = self.frame_on_screen_carries_key(cx) else {
            return false;
        };
        if carries_key || self.painted_draw >= draw {
            self.unpainted_miss_draw = None;
            return false;
        }
        match self.unpainted_miss_draw {
            Some(missed) => draw > missed,
            None => {
                self.unpainted_miss_draw = Some(draw);
                false
            }
        }
    }

    /// says if an image of the frame on screen has the swap key, plus that frame draw count
    fn frame_on_screen_carries_key(&self, cx: &mut Context<Self>) -> Option<(bool, u64)> {
        let window = self.swap_window?;
        let key = self.swap_key;
        window
            .update(cx, |_, window, _| (window.fade_swapped_image(key, 1.0), window.draw_count()))
            .ok()
    }

    fn hide_offscreen(&mut self, cx: &mut Context<Self>) {
        self.unpainted_miss_draw = None;
        if self.frame_on_screen_carries_key(cx).is_none_or(|(carries_key, _)| carries_key) {
            return;
        }
        if let Some(last) = self.last_probe {
            self.last_probe = Some(VisibilityProbe { is_visible: false, is_outside_window: false, ..last });
        }
        tracing::warn!(target: "riseonly::X1652", image_view = ?cx.entity_id(), key = self.swap_key, "gif hidden after unpainted swap miss");
        self.report_visibility_if_changed(cx);
        self.start_offscreen_poll(cx);
    }

    fn start_offscreen_poll(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let shown = this.update(cx, |view, cx| {
                    let Some((carries_key, _)) = view.frame_on_screen_carries_key(cx) else {
                        return false;
                    };
                    if !carries_key {
                        return false;
                    }
                    if let Some(last) = view.last_probe {
                        view.last_probe = Some(VisibilityProbe { is_visible: true, ..last });
                    }
                    view.report_visibility_if_changed(cx);
                    true
                });
                match shown {
                    Ok(false) => continue,
                    _ => return,
                }
            }
        });
        self.offscreen_poll = Some(task);
    }

    fn bind_swap(&mut self, window: AnyWindowHandle, key: u32) {
        self.swap_window = Some(window);
        self.swap_key = key;
    }

    fn swap_frame_in_place(&self, previous: &Arc<RenderImage>, next: &Arc<RenderImage>, cx: &mut Context<Self>) -> bool {
        let Some(window) = self.swap_window else {
            return false;
        };
        let key = self.swap_key;
        window
            .update(cx, |_, window, _| window.swap_image(key, previous, next, 0))
            .unwrap_or(false)
    }
}

/// what one paint saw of the view frame
#[derive(Clone, Copy, Debug, PartialEq)]
struct VisibilityProbe {
    is_visible: bool,
    is_outside_window: bool,
    bounds: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    window: Size<Pixels>,
    is_playback_scope_active: bool,
}

impl VisibilityProbe {
    fn measure_at(bounds: Bounds<Pixels>, clip: Bounds<Pixels>, viewport: Size<Pixels>, is_playback_scope_active: bool) -> Self {
        let window_bounds = Bounds::new(point(px(0.), px(0.)), viewport);
        let is_outside_window = !bounds.intersects(&window_bounds);
        let drawable = bounds.size.width > px(0.) && bounds.size.height > px(0.);
        let is_visible = drawable
            && is_playback_scope_active
            && bounds.intersects(&clip)
            && {
                let visible = bounds.intersect(&clip);
                let fraction = f32::from(visible.size.height) / f32::from(bounds.size.height);
                f32::from(visible.size.width) > 1.0
                    && fraction > AnimatedMediaViewportPlaybackPolicy::MINIMUM_VISIBLE_HEIGHT_FRACTION
            };
        Self {
            is_visible,
            is_outside_window,
            bounds,
            clip,
            window: window_bounds.size,
            is_playback_scope_active,
        }
    }

    /// two paints of one frame, visible if either is and outside only if both are
    fn merged(self, other: Self) -> Self {
        let (shown, hidden) = if other.is_visible || !self.is_visible { (other, self) } else { (self, other) };
        Self {
            is_visible: shown.is_visible || hidden.is_visible,
            is_outside_window: shown.is_outside_window && hidden.is_outside_window,
            ..shown
        }
    }
}

/// shared cancel flag, compared by identity
#[derive(Clone)]
struct FrameDecodeToken(Arc<AtomicBool>);

impl FrameDecodeToken {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

fn same_token(slot: Option<&FrameDecodeToken>, token: &FrameDecodeToken) -> bool {
    slot.is_some_and(|slot| slot.same(token))
}

#[derive(Clone)]
struct AnimationAsset {
    data: Arc<[u8]>,
    frame_durations: Vec<f64>,
    maximum_pixel_dimension: u32,
}

impl AnimationAsset {
    fn frame_count(&self) -> usize {
        self.frame_durations.len()
    }
}

/// how many pixels one frame may take
/// aspect fill scales by the short edge of the box
/// so a budget from the longest side would under decode some pairs
#[derive(Clone, Copy, Debug, PartialEq)]
struct DecodeBudget {
    box_point_size: Size<Pixels>,
    display_scale: f32,
    fills_box: bool,
    produces_backdrop: bool,
}

impl DecodeBudget {
    const CEILING: u32 = 1_024;
    const MINIMUM: u32 = 64;
    const BACKDROP_DIMENSION: u32 = 48;

    fn maximum_pixel_dimension(&self, source_pixel_size: (u32, u32)) -> u32 {
        let box_width = f64::from(f32::from(self.box_point_size.width));
        let box_height = f64::from(f32::from(self.box_point_size.height));
        let (source_width, source_height) =
            (f64::from(source_pixel_size.0), f64::from(source_pixel_size.1));
        if box_width <= 0.0
            || box_height <= 0.0
            || source_width <= 0.0
            || source_height <= 0.0
            || self.display_scale <= 0.0
        {
            return Self::CEILING;
        }
        let width_scale = box_width / source_width;
        let height_scale = box_height / source_height;
        let drawn_scale = if self.fills_box {
            width_scale.max(height_scale)
        } else {
            width_scale.min(height_scale)
        };
        let longest_source_edge = source_width.max(source_height);
        let pixels = (longest_source_edge * drawn_scale * f64::from(self.display_scale)).ceil();
        if !pixels.is_finite() {
            return Self::CEILING;
        }
        (pixels as u32).clamp(Self::MINIMUM, Self::CEILING)
    }
}

/// frame decoder over one payload, read in order
/// kept between frames so the payload isnt reparsed for every shown frame
struct AnimatedFrameSource {
    data: Arc<[u8]>,
    format: ImageFormat,
    frames: Option<Frames<'static>>,
    next_index: usize,
}

impl AnimatedFrameSource {
    fn open(data: Arc<[u8]>) -> Option<Self> {
        if data.is_empty() {
            return None;
        }
        let format = image::guess_format(&data).ok()?;
        let source = Self {
            data,
            format,
            frames: None,
            next_index: 0,
        };
        source.pixel_size()?;
        Some(source)
    }

    /// frame size from the container header
    fn pixel_size(&self) -> Option<(u32, u32)> {
        let cursor = Cursor::new(self.data.clone());
        match self.format {
            ImageFormat::Gif => GifDecoder::new(cursor).ok().map(|decoder| decoder.dimensions()),
            ImageFormat::Png => PngDecoder::new(cursor).ok().map(|decoder| decoder.dimensions()),
            ImageFormat::WebP => WebPDecoder::new(cursor).ok().map(|decoder| decoder.dimensions()),
            _ => image::ImageReader::with_format(cursor, self.format)
                .into_dimensions()
                .ok(),
        }
    }

    fn frame_iterator(&self) -> Option<Frames<'static>> {
        let cursor = Cursor::new(self.data.clone());
        match self.format {
            ImageFormat::Gif => Some(GifDecoder::new(cursor).ok()?.into_frames()),
            ImageFormat::Png => {
                let decoder = PngDecoder::new(cursor).ok()?;
                if decoder.is_apng().ok()? {
                    Some(decoder.apng().ok()?.into_frames())
                } else {
                    None
                }
            }
            ImageFormat::WebP => {
                let decoder = WebPDecoder::new(cursor).ok()?;
                if decoder.has_animation() {
                    Some(decoder.into_frames())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// delay of every frame, one entry for a still image
    /// none if the payload is broken or the request was cancelled midway
    fn frame_durations(&mut self, token: &FrameDecodeToken) -> Option<Vec<f64>> {
        let Some(frames) = self.frame_iterator() else {
            return Some(vec![AnimatedImagePlaybackPolicy::DEFAULT_FRAME_DURATION]);
        };
        let mut durations = Vec::new();
        for frame in frames {
            if token.is_cancelled() {
                return None;
            }
            let frame = frame.ok()?;
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            let duration = (denominator > 0)
                .then(|| f64::from(numerator) / f64::from(denominator) / 1_000.0);
            durations.push(AnimatedImagePlaybackPolicy::normalized_frame_duration(duration));
        }
        (!durations.is_empty()).then_some(durations)
    }

    /// composed frame at index, scaled so the longest edge fits maximum_pixel_dimension
    fn decode_frame(&mut self, index: usize, maximum_pixel_dimension: u32) -> Option<Arc<RenderImage>> {
        let buffer = match self.frame_iterator_at(index) {
            Some(frames) => {
                let frame = frames.next()?.ok()?;
                self.next_index = index + 1;
                frame.into_buffer()
            }
            None => {
                if index != 0 {
                    return None;
                }
                image::load_from_memory_with_format(&self.data, self.format)
                    .ok()?
                    .to_rgba8()
            }
        };
        Some(render_image(buffer, maximum_pixel_dimension))
    }

    fn frame_iterator_at(&mut self, index: usize) -> Option<&mut Frames<'static>> {
        if self.frames.is_none() || index < self.next_index {
            self.frames = Some(self.frame_iterator()?);
            self.next_index = 0;
        }
        let frames = self.frames.as_mut()?;
        while self.next_index < index {
            frames.next()?.ok()?;
            self.next_index += 1;
        }
        Some(frames)
    }
}

/// wraps an rgba frame as a single frame gpui image bounded to maximum_pixel_dimension
fn render_image(buffer: RgbaImage, maximum_pixel_dimension: u32) -> Arc<RenderImage> {
    let (width, height) = buffer.dimensions();
    let longest = width.max(height).max(1);
    let mut buffer = if longest > maximum_pixel_dimension {
        let scale = f64::from(maximum_pixel_dimension) / f64::from(longest);
        let target_width = ((f64::from(width) * scale).round() as u32).max(1);
        let target_height = ((f64::from(height) * scale).round() as u32).max(1);
        image::imageops::resize(&buffer, target_width, target_height, FilterType::Triangle)
    } else {
        buffer
    };
    for pixel in buffer.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([Frame::new(buffer)]);
    Arc::new(RenderImage::new(frames))
}

/// what a decode on the preparation queue produced
struct DecodedAnimation {
    data: Arc<[u8]>,
    frame_durations: Vec<f64>,
    maximum_pixel_dimension: u32,
    poster: Arc<RenderImage>,
    backdrop: Option<Arc<RenderImage>>,
    frame_count: usize,
}

enum PreparationOutcome {
    Abandoned,
    Failed { retry_after: Option<f64> },
    Decoded(DecodedAnimation),
    FetchRemote,
}

/// who the cache entries belong to, the account when the load started
#[derive(Clone)]
struct AccountScope {
    user_id: Option<String>,
    epoch: Option<String>,
}

impl AccountScope {
    fn is_current(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(self.user_id.as_deref(), self.epoch.as_deref())
    }
}

static PREPARATION_QUEUE: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

enum FrameJob {
    Decode {
        data: Arc<[u8]>,
        index: usize,
        maximum_pixel_dimension: u32,
        token: FrameDecodeToken,
        reply: oneshot::Sender<Option<Arc<RenderImage>>>,
    },
    Release(FrameDecodeToken),
}

/// serial frame queue of one view
/// only its thread holds the open source so the decoder keeps its composition state
/// ends with the view
struct FrameDecodeQueue {
    jobs: std::sync::mpsc::Sender<FrameJob>,
}

impl FrameDecodeQueue {
    fn spawn() -> Option<Self> {
        let (jobs, inbox) = std::sync::mpsc::channel::<FrameJob>();
        std::thread::Builder::new()
            .name(FRAME_DECODE_QUEUE_LABEL.to_owned())
            .spawn(move || {
                let mut source: Option<AnimatedFrameSource> = None;
                let mut source_token: Option<FrameDecodeToken> = None;
                for job in inbox {
                    match job {
                        FrameJob::Decode {
                            data,
                            index,
                            maximum_pixel_dimension,
                            token,
                            reply,
                        } => {
                            if token.is_cancelled() {
                                continue;
                            }
                            if index <= 1 {
                                tracing::warn!(target: "riseonly::X960", index, "gif frame job started");
                            }
                            let reusable = same_token(source_token.as_ref(), &token) && source.is_some();
                            if !reusable {
                                let Some(created) = AnimatedFrameSource::open(data) else {
                                    tracing::warn!(target: "riseonly::X960", index, "gif frame source open failed");
                                    continue;
                                };
                                source = Some(created);
                                source_token = Some(token.clone());
                            }
                            let decoded = source
                                .as_mut()
                                .and_then(|source| source.decode_frame(index, maximum_pixel_dimension));
                            if token.is_cancelled() {
                                continue;
                            }
                            reply.send(decoded).ok();
                        }
                        FrameJob::Release(token) => {
                            if same_token(source_token.as_ref(), &token) {
                                source = None;
                                source_token = None;
                            }
                        }
                    }
                }
            })
            .ok()?;
        Some(Self { jobs })
    }
}

pub struct AnimatedGifViewCoordinator {
    backdrop_handler: Option<AnimatedGifBackdropHandler>,
    current_url: Option<String>,
    stop_flag: bool,
    generation: u64,
    playing: bool,
    task: Option<Task<()>>,
    animation_asset: Option<AnimationAsset>,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    persist_to_disk: bool,
    cache_policy: CleverImageCachePolicy,
    image_view: Option<WeakEntity<AnimatedGifImageView>>,
    is_visible: bool,
    pending_load: bool,
    owns_playback_slot: bool,
    is_waiting_for_playback_slot: bool,
    playback_grant_id: Option<u64>,
    preparation_token: Option<FrameDecodeToken>,
    preparation_retry_work_item: Option<Task<()>>,
    failed_load_attempts: i64,
    frame_decode_token: Option<FrameDecodeToken>,
    pending_frame_presentation: Option<Task<()>>,
    frame_decode_queue: Option<FrameDecodeQueue>,
    pub playback_kind: AnimatedMediaPlaybackKind,
    releases_hidden_frames: bool,
    playback_participant_id: AnimatedMediaParticipantId,
}

impl AnimatedGifViewCoordinator {
    fn new() -> Self {
        Self {
            backdrop_handler: None,
            current_url: None,
            stop_flag: false,
            generation: 0,
            playing: true,
            task: None,
            animation_asset: None,
            account_user_id: None,
            account_epoch: None,
            persist_to_disk: true,
            cache_policy: CleverImageCachePolicy::Standard,
            image_view: None,
            is_visible: false,
            pending_load: false,
            owns_playback_slot: false,
            is_waiting_for_playback_slot: false,
            playback_grant_id: None,
            preparation_token: None,
            preparation_retry_work_item: None,
            failed_load_attempts: 0,
            frame_decode_token: None,
            pending_frame_presentation: None,
            frame_decode_queue: None,
            playback_kind: AnimatedMediaPlaybackKind::AnimatedImage,
            releases_hidden_frames: false,
            playback_participant_id: AnimatedMediaParticipantId::next(),
        }
    }

    fn playback_id(&self) -> AnimatedMediaParticipantId {
        self.playback_participant_id
    }

    fn bound_image_view(&self) -> Option<Entity<AnimatedGifImageView>> {
        self.image_view.as_ref().and_then(WeakEntity::upgrade)
    }

    /// the view reports visibility and viewport exit here
    /// reports reach the coordinator on a later main loop turn since both cant be borrowed at once
    fn bind(&mut self, image_view: &Entity<AnimatedGifImageView>, cx: &mut Context<Self>) {
        self.image_view = Some(image_view.downgrade());
        let coordinator = cx.weak_entity();
        let bound = image_view.downgrade();
        let visibility_changed: VisibilityChanged = Rc::new(move |visible: bool, cx: &mut App| {
            let coordinator = coordinator.clone();
            let bound = bound.clone();
            cx.defer(move |cx| {
                coordinator
                    .update(cx, |this, cx| {
                        if this.image_view.as_ref().is_some_and(|view| *view == bound) {
                            this.set_visible(visible, cx);
                        }
                    })
                    .ok();
            });
        });
        let coordinator = cx.weak_entity();
        let bound = image_view.downgrade();
        let left_viewport: LeftViewport = Rc::new(move |cx: &mut App| {
            let coordinator = coordinator.clone();
            let bound = bound.clone();
            cx.defer(move |cx| {
                coordinator
                    .update(cx, |this, cx| {
                        if this.image_view.as_ref().is_some_and(|view| *view == bound) {
                            this.release_hidden_frames(cx);
                        }
                    })
                    .ok();
            });
        });
        let reports_viewport_exit = self.releases_hidden_frames;
        let is_effectively_visible = image_view.update(cx, |view, _| {
            view.visibility_changed = Some(visibility_changed);
            view.left_viewport = Some(left_viewport);
            view.set_reports_viewport_exit(reports_viewport_exit);
            view.is_effectively_visible()
        });
        self.set_visible(is_effectively_visible, cx);
    }

    fn set_releases_hidden_frames(&mut self, releases_hidden_frames: bool, cx: &mut Context<Self>) {
        if self.releases_hidden_frames == releases_hidden_frames {
            return;
        }
        self.releases_hidden_frames = releases_hidden_frames;
        if let Some(view) = self.bound_image_view() {
            view.update(cx, |view, _| view.set_reports_viewport_exit(releases_hidden_frames));
        }
    }

    fn set_image(&mut self, image: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        if let Some(view) = self.bound_image_view() {
            view.update(cx, |view, cx| view.set_image(image, cx));
        }
    }

    fn update(
        &mut self,
        url: &str,
        is_playing: bool,
        persist_to_disk: bool,
        cache_policy: CleverImageCachePolicy,
        on_backdrop_image: Option<AnimatedGifBackdropHandler>,
        cx: &mut Context<Self>,
    ) {
        self.backdrop_handler = on_backdrop_image;
        let trimmed = trimming_whitespaces_and_newlines(url);
        if trimmed.is_empty() {
            self.stop(cx);
            self.set_image(None, cx);
            self.current_url = None;
            self.animation_asset = None;
            self.account_user_id = None;
            self.account_epoch = None;
            return;
        }
        let active_user_id = SaiAccountCacheScope::active_user_id();
        let active_epoch = SaiAccountCacheScope::active_epoch();
        if self.current_url.as_deref() != Some(trimmed)
            || active_user_id != self.account_user_id
            || active_epoch != self.account_epoch
            || persist_to_disk != self.persist_to_disk
            || cache_policy != self.cache_policy
        {
            self.playing = is_playing;
            self.persist_to_disk = persist_to_disk;
            self.cache_policy = cache_policy;
            self.stop(cx);
            self.current_url = Some(trimmed.to_owned());
            self.account_user_id = active_user_id;
            self.account_epoch = active_epoch;
            self.failed_load_attempts = 0;
            self.pending_load = true;
            self.set_image(None, cx);
            self.animation_asset = None;
            self.begin_load_if_needed(cx);
            return;
        }
        self.begin_load_if_needed(cx);
        if is_playing == self.playing {
            return;
        }
        tracing::warn!(target: "riseonly::X960", url = ?self.current_url, is_playing, "gif playing changed");
        self.playing = is_playing;
        if is_playing {
            self.request_playback_if_possible(cx);
        } else {
            self.pause_playback(cx);
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.stop_flag = true;
        if let Some(token) = self.preparation_token.take() {
            token.cancel();
        }
        self.preparation_retry_work_item = None;
        self.cancel_frame_pipeline(cx);
        self.generation = self.generation.wrapping_add(1);
        self.task = None;
        self.pending_load = false;
        self.release_playback_slot(cx);
    }

    fn teardown(&mut self, ui_view: &Entity<AnimatedGifImageView>, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X960", url = ?self.current_url, owns = self.owns_playback_slot, "gif view dismantled");
        ui_view.update(cx, |view, _| {
            view.visibility_changed = None;
            view.left_viewport = None;
            view.stop_visibility_tracking();
        });
        if self.image_view.as_ref().is_some_and(|view| *view == ui_view.downgrade()) {
            self.image_view = None;
        }
        self.is_visible = false;
        self.pending_load = false;
        self.stop(cx);
    }

    fn begin_load_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.is_visible || !self.pending_load || self.bound_image_view().is_none() {
            return;
        }
        let Some(url) = self.current_url.clone() else {
            return;
        };
        self.pending_load = false;
        self.load(url, cx);
    }

    fn load(&mut self, url: String, cx: &mut Context<Self>) {
        let account = AccountScope {
            user_id: self.account_user_id.clone(),
            epoch: self.account_epoch.clone(),
        };
        self.current_url = Some(url.clone());
        self.stop(cx);
        self.account_user_id = account.user_id.clone();
        self.account_epoch = account.epoch.clone();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let token = FrameDecodeToken::new();
        self.preparation_token = Some(token.clone());
        self.set_image(None, cx);
        self.animation_asset = None;
        let Some(image_view) = self.bound_image_view() else {
            return;
        };
        let budget = {
            let view = image_view.read(cx);
            DecodeBudget {
                box_point_size: view.bounds_size(),
                display_scale: view.display_scale(),
                fills_box: view.content_mode.fills_box(),
                produces_backdrop: self.backdrop_handler.is_some(),
            }
        };
        let Some(source) = AnimatedImageSourcePolicy::resolve(&url) else {
            self.finish_preparation_failure(&token, generation, &url, None, cx);
            return;
        };
        let persist_to_disk = self.persist_to_disk;
        let cache_policy = self.cache_policy;

        let background_token = token.clone();
        let background_url = url.clone();
        let background_account = account.clone();
        let preparation = cx.background_spawn(async move {
            let _queue = PREPARATION_QUEUE.lock().await;
            prepare_local_or_cached(
                source,
                &background_url,
                &background_account,
                budget,
                &background_token,
                persist_to_disk,
                cache_policy,
            )
        });
        cx.spawn(async move |this, cx| {
            let outcome = preparation.await;
            this.update(cx, |this, cx| match outcome {
                PreparationOutcome::Abandoned => {}
                PreparationOutcome::Failed { retry_after } => {
                    this.finish_preparation_failure(&token, generation, &url, retry_after, cx);
                }
                PreparationOutcome::Decoded(decoded) => {
                    this.apply_decoded(decoded, &token, generation, &url, &account, cx);
                }
                PreparationOutcome::FetchRemote => {
                    let is_current = !token.is_cancelled()
                        && same_token(this.preparation_token.as_ref(), &token)
                        && generation == this.generation
                        && this.current_url.as_deref() == Some(url.as_str())
                        && account.is_current();
                    if is_current {
                        this.fetch_remote(url.clone(), generation, account.clone(), budget, token.clone(), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn fetch_remote(
        &mut self,
        url: String,
        generation: u64,
        account: AccountScope,
        budget: DecodeBudget,
        token: FrameDecodeToken,
        cx: &mut Context<Self>,
    ) {
        if token.is_cancelled() || !same_token(self.preparation_token.as_ref(), &token) {
            return;
        }
        let Some(remote) = Url::from_string(&url) else {
            self.finish_preparation_failure(&token, generation, &url, None, cx);
            return;
        };
        let stores_payload =
            self.persist_to_disk && self.cache_policy == CleverImageCachePolicy::Standard;
        let mut request = URLRequest::new(remote.clone());
        request.timeout_interval = REMOTE_REQUEST_TIMEOUT;
        request.set_value(ACCEPT_IMAGES, reqwest::header::ACCEPT.as_str());
        request.set_value(NO_STORE, reqwest::header::CACHE_CONTROL.as_str());
        self.task = Some(cx.spawn(async move |this, cx| {
            if let Some(bearer) = RemoteMediaDiskCache::bearer_token(&remote, None, cx).await {
                request.set_value(
                    &format!("Bearer {bearer}"),
                    reqwest::header::AUTHORIZATION.as_str(),
                );
            }
            if token.is_cancelled() || !account.is_current() {
                return;
            }
            let fetched = RiseProgressiveDataFetcher::data(
                request,
                AnimatedGifView::MAXIMUM_PAYLOAD_BYTES,
                RiseProgressiveDataFetcherLane::PrivateEphemeral,
                account.user_id.clone(),
                account.epoch.clone(),
                RiseProgressiveDataFetcher::DEFAULT_PRIORITY,
                Arc::new(|_: RemoteMediaDownloadProgress| {}),
                cx,
            )
            .await;
            if token.is_cancelled() {
                return;
            }
            let data = match fetched {
                Ok((data, response))
                    if !data.is_empty()
                        && (200..300).contains(&response.status_code)
                        && is_acceptable_content_type(&response) =>
                {
                    data
                }
                _ => {
                    this.update(cx, |this, cx| {
                        this.finish_preparation_failure(
                            &token,
                            generation,
                            &url,
                            Some(REMOTE_FAILURE_RETRY_DELAY),
                            cx,
                        );
                    })
                    .ok();
                    return;
                }
            };
            let decode_token = token.clone();
            let decode_url = url.clone();
            let decode_account = account.clone();
            let outcome = cx
                .background_spawn(async move {
                    let _queue = PREPARATION_QUEUE.lock().await;
                    if decode_token.is_cancelled() {
                        return PreparationOutcome::Abandoned;
                    }
                    decode_animation(
                        Arc::from(data),
                        &decode_url,
                        &decode_account,
                        budget,
                        &decode_token,
                        Some(REMOTE_FAILURE_RETRY_DELAY),
                        None,
                        stores_payload,
                    )
                })
                .await;
            this.update(cx, |this, cx| match outcome {
                PreparationOutcome::Decoded(decoded) => {
                    this.apply_decoded(decoded, &token, generation, &url, &account, cx);
                }
                PreparationOutcome::Failed { retry_after } => {
                    this.finish_preparation_failure(&token, generation, &url, retry_after, cx);
                }
                PreparationOutcome::Abandoned | PreparationOutcome::FetchRemote => {}
            })
            .ok();
        }));
    }

    fn finish_preparation_failure(
        &mut self,
        token: &FrameDecodeToken,
        failed_generation: u64,
        url: &str,
        retry_after: Option<f64>,
        cx: &mut Context<Self>,
    ) {
        if token.is_cancelled()
            || !same_token(self.preparation_token.as_ref(), token)
            || self.generation != failed_generation
            || self.current_url.as_deref() != Some(url)
        {
            return;
        }
        token.cancel();
        self.task = None;
        self.preparation_token = None;
        self.preparation_retry_work_item = None;

        let Some(delay) = retry_after.filter(|_| self.current_url.is_some()) else {
            self.pending_load = false;
            return;
        };
        if self.failed_load_attempts >= RemoteMediaDownloadRetryPolicy::maximum_retries() {
            self.pending_load = false;
            return;
        }
        self.failed_load_attempts += 1;
        if !self.is_visible {
            self.pending_load = true;
            return;
        }

        self.pending_load = false;
        let url = url.to_owned();
        self.preparation_retry_work_item = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(delay.max(0.0)))
                .await;
            this.update(cx, |this, cx| {
                if this.generation != failed_generation
                    || this.current_url.as_deref() != Some(url.as_str())
                    || !this.is_visible
                {
                    return;
                }
                this.preparation_retry_work_item = None;
                this.pending_load = true;
                this.begin_load_if_needed(cx);
            })
            .ok();
        }));
    }

    fn apply_decoded(
        &mut self,
        decoded: DecodedAnimation,
        token: &FrameDecodeToken,
        generation: u64,
        url: &str,
        account: &AccountScope,
        cx: &mut Context<Self>,
    ) {
        if self.bound_image_view().is_none()
            || token.is_cancelled()
            || !same_token(self.preparation_token.as_ref(), token)
            || generation != self.generation
            || self.current_url.as_deref() != Some(url)
            || !self.is_visible
            || !account.is_current()
        {
            return;
        }
        self.task = None;
        self.preparation_token = None;
        self.failed_load_attempts = 0;
        let frame_count = decoded.frame_count;
        self.animation_asset = (frame_count > 1).then(|| AnimationAsset {
            data: decoded.data,
            frame_durations: decoded.frame_durations,
            maximum_pixel_dimension: decoded.maximum_pixel_dimension,
        });
        self.set_image(Some(decoded.poster), cx);
        if let (Some(backdrop), Some(handler)) = (decoded.backdrop, self.backdrop_handler.clone()) {
            handler(backdrop, cx);
        }
        tracing::warn!(target: "riseonly::X960", url, frame_count, playing = self.playing, visible = self.is_visible, kind = ?self.playback_kind, "gif poster applied");
        if frame_count <= 1 || !self.playing {
            return;
        }
        self.request_playback_if_possible(cx);
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.is_visible {
            return;
        }
        tracing::warn!(target: "riseonly::X960", url = ?self.current_url, visible, "gif visibility changed");
        self.is_visible = visible;
        if visible {
            self.failed_load_attempts = 0;
            if self.animation_asset.is_none() && self.preparation_token.is_none() {
                self.pending_load = self.current_url.is_some();
            }
            self.begin_load_if_needed(cx);
            self.request_playback_if_possible(cx);
        } else {
            self.preparation_retry_work_item = None;
            if self.animation_asset.is_none() {
                if let Some(token) = self.preparation_token.take() {
                    token.cancel();
                }
                self.task = None;
                self.generation = self.generation.wrapping_add(1);
                self.pending_load = self.current_url.is_some();
            }
            self.pause_playback(cx);
        }
    }

    fn release_hidden_frames(&mut self, cx: &mut Context<Self>) {
        if !self.releases_hidden_frames
            || self.is_visible
            || self
                .bound_image_view()
                .is_none_or(|view| self.animation_asset.is_none() && view.read(cx).image.is_none())
        {
            return;
        }
        self.animation_asset = None;
        self.set_image(None, cx);
        self.pending_load = self.current_url.is_some();
    }

    fn pause_playback(&mut self, cx: &mut Context<Self>) {
        self.stop_flag = true;
        self.cancel_frame_pipeline(cx);
        if self.owns_playback_slot || self.is_waiting_for_playback_slot {
            self.generation = self.generation.wrapping_add(1);
        }
        self.release_playback_slot(cx);
    }

    fn request_playback_if_possible(&mut self, cx: &mut Context<Self>) {
        if !self.playing
            || !self.is_visible
            || self.animation_asset.is_none()
            || self.bound_image_view().is_none()
            || self.owns_playback_slot
            || self.is_waiting_for_playback_slot
        {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let requested_generation = self.generation;
        let requested_playback_id = self.playback_id();
        tracing::warn!(target: "riseonly::X960", url = ?self.current_url, generation = requested_generation, kind = ?self.playback_kind, "gif playback requested");
        self.is_waiting_for_playback_slot = true;
        let granted = cx.weak_entity();
        let revoked = cx.weak_entity();
        self.playback_grant_id = Some(AnimatedGifPlaybackHub::shared().request(
            requested_playback_id,
            self.playback_kind,
            Rc::new(move |grant_id: u64, cx: &mut App| {
                let still_here = granted
                    .update(cx, |this, cx| {
                        this.playback_slot_granted(requested_generation, grant_id, cx);
                    })
                    .is_ok();
                if !still_here {
                    AnimatedGifPlaybackHub::shared().release(requested_playback_id, grant_id, cx);
                }
            }),
            Rc::new(move |grant_id: u64, cx: &mut App| {
                revoked
                    .update(cx, |this, cx| this.playback_slot_revoked(grant_id, cx))
                    .ok();
            }),
            cx,
        ));
    }

    fn playback_slot_granted(&mut self, requested_generation: u64, grant_id: u64, cx: &mut Context<Self>) {
        if self.playback_grant_id != Some(grant_id) {
            AnimatedGifPlaybackHub::shared().release(self.playback_id(), grant_id, cx);
            return;
        }
        self.is_waiting_for_playback_slot = false;
        let asset = self.animation_asset.clone().filter(|_| {
            requested_generation == self.generation
                && self.playing
                && self.is_visible
                && self.bound_image_view().is_some()
        });
        tracing::warn!(target: "riseonly::X960", url = ?self.current_url, grant_id, requested_generation, generation = self.generation, playing = self.playing, visible = self.is_visible, has_asset = self.animation_asset.is_some(), bound = self.bound_image_view().is_some(), "gif playback granted");
        let Some(asset) = asset else {
            self.playback_grant_id = None;
            AnimatedGifPlaybackHub::shared().release(self.playback_id(), grant_id, cx);
            return;
        };
        self.owns_playback_slot = true;
        self.stop_flag = false;
        let token = FrameDecodeToken::new();
        self.frame_decode_token = Some(token.clone());
        self.schedule_frame(Arc::new(asset), 0, requested_generation, 0.0, token, cx);
    }

    fn playback_slot_revoked(&mut self, grant_id: u64, cx: &mut Context<Self>) {
        if self.playback_grant_id != Some(grant_id) {
            return;
        }
        tracing::warn!(target: "riseonly::X960", url = ?self.current_url, grant_id, "gif playback revoked");
        self.owns_playback_slot = false;
        self.is_waiting_for_playback_slot = true;
        self.stop_flag = true;
        self.cancel_frame_pipeline(cx);
    }

    fn may_present(&self, token: &FrameDecodeToken, requested_generation: u64) -> bool {
        self.bound_image_view().is_some()
            && !token.is_cancelled()
            && requested_generation == self.generation
            && !self.stop_flag
            && self.playing
            && self.is_visible
            && self.owns_playback_slot
            && SaiAccountCacheScope::is_current_account_epoch(
                self.account_user_id.as_deref(),
                self.account_epoch.as_deref(),
            )
    }

    fn schedule_frame(
        &mut self,
        asset: Arc<AnimationAsset>,
        index: usize,
        requested_generation: u64,
        display_after: f64,
        token: FrameDecodeToken,
        cx: &mut Context<Self>,
    ) {
        let display_delay = Duration::from_secs_f64(display_after.max(0.0));
        if self.frame_decode_queue.is_none() {
            self.frame_decode_queue = FrameDecodeQueue::spawn();
        }
        let (reply, decode) = oneshot::channel();
        let queued = self.frame_decode_queue.as_ref().is_some_and(|queue| {
            queue
                .jobs
                .send(FrameJob::Decode {
                    data: asset.data.clone(),
                    index,
                    maximum_pixel_dimension: asset.maximum_pixel_dimension,
                    token: token.clone(),
                    reply,
                })
                .is_ok()
        });
        if !queued {
            tracing::warn!(target: "riseonly::X960", index, has_queue = self.frame_decode_queue.is_some(), "gif frame job not queued");
            return;
        }
        let started = RiseClock::monotonic();
        self.pending_frame_presentation = Some(cx.spawn(async move |this, cx| {
            let Ok(decoded) = decode.await else {
                tracing::warn!(target: "riseonly::X960", index, cancelled = token.is_cancelled(), "gif frame reply dropped");
                return;
            };
            if token.is_cancelled() {
                tracing::warn!(target: "riseonly::X960", index, "gif frame cancelled after decode");
                return;
            }
            let Some(decoded) = decoded else {
                tracing::warn!(target: "riseonly::X960", index, "gif frame decode failed");
                this.update(cx, |this, cx| {
                    if !this.may_present(&token, requested_generation) {
                        return;
                    }
                    this.stop_flag = true;
                    this.cancel_frame_pipeline(cx);
                    this.generation = this.generation.wrapping_add(1);
                    this.release_playback_slot(cx);
                })
                .ok();
                return;
            };
            let remaining = display_delay.saturating_sub(RiseClock::monotonic().saturating_sub(started));
            if !remaining.is_zero() {
                cx.background_executor().timer(remaining).await;
            }
            this.update(cx, |this, cx| {
                if !this.may_present(&token, requested_generation) {
                    tracing::warn!(target: "riseonly::X960", url = ?this.current_url, index, requested_generation, generation = this.generation, stop = this.stop_flag, playing = this.playing, visible = this.is_visible, owns = this.owns_playback_slot, bound = this.bound_image_view().is_some(), "gif frame dropped");
                    return;
                }
                if index <= 1 || index % 30 == 0 {
                    tracing::warn!(target: "riseonly::X960", url = ?this.current_url, index, "gif frame presented");
                }
                let pixels = decoded.size(0);
                let fields = HashMap::from([
                    ("height".to_owned(), pixels.height.0.to_string()),
                    ("width".to_owned(), pixels.width.0.to_string()),
                ]);
                let trace = ChatPerformanceTrace::shared();
                let apply_token = trace.begin_operation(GIF_FRAME_APPLY_OPERATION, fields);
                this.set_image(Some(decoded), cx);
                trace.end_operation(apply_token, GIF_FRAME_APPLY_THRESHOLD_MS, HashMap::new());
                let next_index = (index + 1) % asset.frame_count();
                let display_after = asset.frame_durations[index];
                this.schedule_frame(asset.clone(), next_index, requested_generation, display_after, token.clone(), cx);
            })
            .ok();
        }));
    }

    fn cancel_frame_pipeline(&mut self, _cx: &mut Context<Self>) {
        if self.frame_decode_token.is_some() {
            tracing::warn!(target: "riseonly::X960", url = ?self.current_url, stop = self.stop_flag, playing = self.playing, visible = self.is_visible, "gif frame pipeline cancelled");
        }
        let token = self.frame_decode_token.take();
        self.pending_frame_presentation = None;
        let Some(token) = token else {
            return;
        };
        token.cancel();
        if let Some(queue) = self.frame_decode_queue.as_ref() {
            queue.jobs.send(FrameJob::Release(token)).ok();
        }
    }

    fn release_playback_slot(&mut self, cx: &mut Context<Self>) {
        if !self.owns_playback_slot && !self.is_waiting_for_playback_slot {
            return;
        }
        self.owns_playback_slot = false;
        self.is_waiting_for_playback_slot = false;
        let Some(grant_id) = self.playback_grant_id.take() else {
            return;
        };
        AnimatedGifPlaybackHub::shared().release(self.playback_id(), grant_id, cx);
    }
}

fn is_acceptable_content_type(response: &RemoteMediaHttpResponse) -> bool {
    let value = response
        .value(reqwest::header::CONTENT_TYPE.as_str())
        .unwrap_or_default()
        .to_lowercase();
    !REJECTED_CONTENT_TYPES
        .iter()
        .any(|rejected| value.contains(rejected))
}

fn prepare_local_or_cached(
    source: AnimatedImageSourcePolicySource,
    url: &str,
    account: &AccountScope,
    budget: DecodeBudget,
    token: &FrameDecodeToken,
    persist_to_disk: bool,
    cache_policy: CleverImageCachePolicy,
) -> PreparationOutcome {
    if token.is_cancelled() || !account.is_current() {
        return PreparationOutcome::Abandoned;
    }
    if let AnimatedImageSourcePolicySource::LocalFile(path) = source {
        let Some(data) =
            AnimatedImageSourcePolicy::mapped_local_data(&path, AnimatedGifView::MAXIMUM_PAYLOAD_BYTES)
        else {
            return PreparationOutcome::Failed { retry_after: None };
        };
        return decode_animation(Arc::from(data), url, account, budget, token, None, None, false);
    }
    if persist_to_disk && cache_policy == CleverImageCachePolicy::Standard {
        let user_id = account.user_id.as_deref();
        let payload_key = AnimatedImageSourcePolicy::payload_cache_key(url);
        let payload = CleverImageDiskCache::image_data(&payload_key, user_id, None);
        let legacy = if payload.is_none() {
            CleverImageDiskCache::image_data(url, user_id, None)
        } else {
            None
        };
        let stores_payload = payload.is_none();
        let cached = payload.or_else(|| {
            legacy.filter(|data| AnimatedImageSourcePolicy::is_complete_animation(data))
        });
        if let Some(cached) = cached {
            let cached_key = if stores_payload { url.to_owned() } else { payload_key };
            if token.is_cancelled() {
                return PreparationOutcome::Abandoned;
            }
            let cached: Arc<[u8]> = Arc::from(cached);
            if AnimatedFrameSource::open(cached.clone()).is_none() {
                CleverImageDiskCache::remove_cache_key(&cached_key, user_id, Some(&cached[..]));
                return PreparationOutcome::Failed { retry_after: Some(0.0) };
            }
            return decode_animation(
                cached,
                url,
                account,
                budget,
                token,
                Some(REMOTE_FAILURE_RETRY_DELAY),
                Some(&cached_key),
                stores_payload,
            );
        }
    }
    PreparationOutcome::FetchRemote
}

/// decode up to the main thread hop
/// frame delays, the poster in its budget
/// the optional backdrop and the payload store
#[allow(clippy::too_many_arguments)]
fn decode_animation(
    data: Arc<[u8]>,
    url: &str,
    account: &AccountScope,
    budget: DecodeBudget,
    token: &FrameDecodeToken,
    failure_retry_delay: Option<f64>,
    cached_payload_cache_key: Option<&str>,
    stores_payload: bool,
) -> PreparationOutcome {
    if token.is_cancelled() || !account.is_current() {
        return PreparationOutcome::Abandoned;
    }
    let fail_decoding = || {
        if let Some(key) = cached_payload_cache_key
            && !token.is_cancelled()
        {
            CleverImageDiskCache::remove_cache_key(key, account.user_id.as_deref(), Some(&data[..]));
        }
        let retry_after = if cached_payload_cache_key.is_some() {
            Some(0.0)
        } else {
            failure_retry_delay
        };
        PreparationOutcome::Failed { retry_after }
    };
    let Some(mut source) = AnimatedFrameSource::open(data.clone()) else {
        return fail_decoding();
    };
    let Some(frame_durations) = source.frame_durations(token) else {
        if token.is_cancelled() {
            return PreparationOutcome::Abandoned;
        }
        return fail_decoding();
    };
    let frame_count = frame_durations.len();
    if token.is_cancelled() {
        return PreparationOutcome::Abandoned;
    }
    let Some(pixel_size) = source.pixel_size() else {
        return fail_decoding();
    };
    let maximum_pixel_dimension = budget.maximum_pixel_dimension(pixel_size);
    let poster = source.decode_frame(0, maximum_pixel_dimension);
    if token.is_cancelled() {
        return PreparationOutcome::Abandoned;
    }
    let Some(poster) = poster else {
        return fail_decoding();
    };
    let backdrop = budget
        .produces_backdrop
        .then(|| source.decode_frame(0, DecodeBudget::BACKDROP_DIMENSION))
        .flatten();
    if token.is_cancelled() {
        return PreparationOutcome::Abandoned;
    }
    if stores_payload && account.is_current() {
        CleverImageDiskCache::store_data(
            &data[..],
            &AnimatedImageSourcePolicy::payload_cache_key(url),
            account.user_id.as_deref(),
            None,
        );
    }
    PreparationOutcome::Decoded(DecodedAnimation {
        data,
        frame_durations: if frame_count > 1 { frame_durations } else { Vec::new() },
        maximum_pixel_dimension,
        poster,
        backdrop,
        frame_count,
    })
}

type PlaybackGrantCallback = Rc<dyn Fn(u64, &mut App)>;

/// view lease on the shared animated media budget
/// grants and revokes reach the view on a later main loop turn, never inside the call that caused them
struct AnimatedGifPlaybackHub;

impl AnimatedGifPlaybackHub {
    fn shared() -> Self {
        Self
    }

    fn request(
        &self,
        id: AnimatedMediaParticipantId,
        kind: AnimatedMediaPlaybackKind,
        on_granted: PlaybackGrantCallback,
        on_revoked: PlaybackGrantCallback,
        cx: &mut App,
    ) -> u64 {
        AnimatedMediaPlaybackCoordinator::shared(cx).update(cx, |coordinator, cx| {
            coordinator.request(
                id,
                kind,
                move |grant_id: u64, cx: &mut App| on_granted(grant_id, cx),
                move |grant_id: u64, cx: &mut App| on_revoked(grant_id, cx),
                cx,
            )
        })
    }

    fn release(&self, id: AnimatedMediaParticipantId, grant_id: u64, cx: &mut App) {
        AnimatedMediaPlaybackCoordinator::shared(cx)
            .update(cx, |coordinator, cx| coordinator.release(id, Some(grant_id), cx));
    }
}

#[cfg(test)]
#[path = "animated_gif_view_tests.rs"]
mod tests;
