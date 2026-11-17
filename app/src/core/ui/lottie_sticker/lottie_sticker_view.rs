use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{
    AnyElement, AnyWindowHandle, App, BackgroundExecutor, Bounds, Context, Element, ElementId, Entity, EntityId,
    GlobalElementId, Hsla, ImageSource, InspectorElementId, IntoElement, LayoutId, ObjectFit, ParentElement,
    Pixels, RenderImage, RenderOnce, Rgba, Size, Styled, StyledImage, Subscription, Task, WeakEntity, Window,
    canvas, div, img, point, px, size,
};
use image::{Delay, Frame, RgbaImage};
use rise_canon::rise_key::otp;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_theme::ui_color_system::UIColor;
use rise_ui::IconUi;
use rise_ui::animated_media_playback_scope::{
    AnimatedMediaPlaybackScopeProviding, AnimatedMediaViewportPlaybackPolicy,
    AnimatedMediaViewportScopeProviding, is_animated_media_playback_active,
    nearest_animated_media_playback_scope_view, nearest_animated_media_viewport_scope_view,
};
use rise_ui::swiftui::alpha_shadow::AlphaShadowModifiers;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use smallvec::SmallVec;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai_remote_media_url_hooks::sai_remote_media_url_optional_normalized;
use crate::core::media::remote_media_disk_cache::RemoteMediaDownloadRetryPolicy;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::ui::animated_media_playback_coordinator::{
    AnimatedMediaParticipantId, AnimatedMediaPlaybackKind,
};
use crate::core::ui::lottie_sticker::chat_performance_trace_hooks::{
    chat_performance_trace_begin_operation, chat_performance_trace_end_operation,
    chat_performance_trace_record,
};
use crate::core::ui::lottie_sticker::lottie_sticker_loader::{
    LottieStickerContainer, LottieStickerLoader, LottieStickerLoaderError, LottieStickerPayload,
    LottieStickerRenderSource,
};
use crate::core::ui::lottie_sticker::lottie_sticker_playback_hub::LottieStickerPlaybackHub;
use crate::core::ui::lottie_sticker::rise_lottie_display_driver::{
    RiseLottieDisplayDriver, RiseLottiePlaybackTarget,
};
use crate::core::ui::lottie_sticker::rise_lottie_frame_cache::{
    RiseLottieFrameCache, RiseLottieFrameSequence, RiseLottieRasterPolicy, RiseLottieSequenceKey,
};
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieFirstFrameCache;
use crate::core::ui::lottie_sticker::sticker_premium_effect::STICKER_PREMIUM_EFFECT_DID_START;
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;

const DEFAULT_SIZE: f32 = 144.0;
const RLOTTIE_PREPARE_EVENT: &str = "animated_media.rlottie_prepare";
const LOTTIE_INSTALL_OPERATION: &str = "animated_media.lottie_install";
const LOTTIE_INSTALL_THRESHOLD_MS: f64 = 4.0;
const FIELD_RESULT: &str = "result";
const FIELD_FRAMES: &str = "frames";
const FIELD_FPS: &str = "fps";
const FIELD_ENGINE: &str = "engine";
const RESULT_READY: &str = "ready";
const RESULT_TRANSPARENT_FALLBACK: &str = "transparent-fallback";
const ENGINE_RLOTTIE: &str = "rlottie";
const RESIDENCY_RELEASE_DELAY: Duration = Duration::from_millis(500);
const PLACEHOLDER_CORNER_RADIUS: f32 = 12.0;
const PLACEHOLDER_ALPHA: f32 = 0.35;
const FAILED_PLACEHOLDER_ALPHA: f32 = 0.2;
const FAILURE_ICON_SYMBOL: &str = "exclamationmark.triangle";
const FAILURE_ICON_MAXIMUM_SIDE: f32 = 20.0;

/// called once per source after the frame is on screen
pub type LottieStickerFirstFrame = Rc<dyn Fn(&mut App)>;

/// called when a one shot player shows its last frame
pub type LottieStickerCompletion = Rc<dyn Fn(&mut App)>;

#[derive(IntoElement)]
pub struct LottieStickerView {
    id: ElementId,
    pub url: String,
    pub size: f32,
    pub r#loop: bool,
    pub is_playing: bool,
    pub playback_id: Option<String>,
    pub playback_kind: AnimatedMediaPlaybackKind,
    pub on_first_frame: Option<LottieStickerFirstFrame>,
    pub on_complete: Option<LottieStickerCompletion>,
    pub mirrored: bool,
    pub casts_shadow: bool,
}

impl LottieStickerView {
    pub fn new(id: impl Into<ElementId>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
            size: DEFAULT_SIZE,
            r#loop: true,
            is_playing: true,
            playback_id: None,
            playback_kind: AnimatedMediaPlaybackKind::Lottie,
            on_first_frame: None,
            on_complete: None,
            mirrored: false,
            casts_shadow: false,
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn r#loop(mut self, r#loop: bool) -> Self {
        self.r#loop = r#loop;
        self
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn playback_id(mut self, playback_id: Option<String>) -> Self {
        self.playback_id = playback_id;
        self
    }

    pub fn playback_kind(mut self, playback_kind: AnimatedMediaPlaybackKind) -> Self {
        self.playback_kind = playback_kind;
        self
    }

    pub fn on_first_frame(mut self, on_first_frame: impl Fn(&mut App) + 'static) -> Self {
        self.on_first_frame = Some(Rc::new(on_first_frame));
        self
    }

    pub fn on_complete(mut self, on_complete: impl Fn(&mut App) + 'static) -> Self {
        self.on_complete = Some(Rc::new(on_complete));
        self
    }

    pub fn mirrored(mut self, mirrored: bool) -> Self {
        self.mirrored = mirrored;
        self
    }

    /// the shadow source reads the image only in prepaint so under a shadow the first frame replaces
    /// the placeholder with a render
    pub fn casts_shadow(mut self, casts_shadow: bool) -> Self {
        self.casts_shadow = casts_shadow;
        self
    }
}

impl RenderOnce for LottieStickerView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let side = px(self.size);
        if is_message_layout_measurement(cx) {
            return div().w(side).h(side).into_any_element();
        }
        let is_playback_scope_active = is_animated_media_playback_active(cx);
        let (loops_forever, is_frozen) = {
            let store = RiseEnergySavingStore::shared(cx);
            let store = store.read(cx);
            (
                self.r#loop && !store.reduces(RiseEnergySavingFeature::LoopAnimations),
                self.playback_kind
                    .energy_feature()
                    .is_some_and(|feature| store.reduces(feature)),
            )
        };
        let playback_id = self.playback_id.unwrap_or_else(|| self.url.clone());
        div()
            .w(side)
            .h(side)
            .child(LottieStickerRepresentable {
                id: self.id,
                url: self.url,
                size: self.size,
                r#loop: loops_forever,
                is_playing: self.is_playing && is_playback_scope_active,
                is_frozen,
                playback_id,
                playback_kind: self.playback_kind,
                on_first_frame: self.on_first_frame,
                on_complete: self.on_complete,
                mirrored: self.mirrored,
                casts_shadow: self.casts_shadow,
            })
            .into_any_element()
    }
}

#[derive(IntoElement)]
struct LottieStickerRepresentable {
    id: ElementId,
    url: String,
    size: f32,
    r#loop: bool,
    is_playing: bool,
    is_frozen: bool,
    playback_id: String,
    playback_kind: AnimatedMediaPlaybackKind,
    on_first_frame: Option<LottieStickerFirstFrame>,
    on_complete: Option<LottieStickerCompletion>,
    mirrored: bool,
    casts_shadow: bool,
}

impl LottieStickerRepresentable {
    fn make_coordinator(&self, window: &mut Window, cx: &mut App) -> Entity<LottieStickerRepresentableCoordinator> {
        window.use_keyed_state((self.id.clone(), "coordinator"), cx, |_, cx| {
            LottieStickerRepresentableCoordinator::new()
        })
    }

    fn make_ui_view(&self, window: &mut Window, cx: &mut App) -> Entity<StickerHostView> {
        window.use_keyed_state((self.id.clone(), "host"), cx, |window, cx| {
            let mut host = StickerHostView::new();
            host.did_move_to_window(window, cx);
            host
        })
    }

    fn update_ui_view(&self, coordinator: &Entity<LottieStickerRepresentableCoordinator>, cx: &mut App) {
        let point_size = size(px(self.size), px(self.size));
        coordinator.update(cx, |coordinator, cx| {
            coordinator.update(
                &self.url,
                &self.playback_id,
                self.r#loop,
                self.is_playing,
                self.is_frozen,
                point_size,
                self.playback_kind,
                self.on_first_frame.clone(),
                self.on_complete.clone(),
                self.mirrored,
                cx,
            );
        });
    }

    fn dismantle_ui_view(coordinator: &Entity<LottieStickerRepresentableCoordinator>, cx: &mut App) {
        coordinator.update(cx, |coordinator, cx| coordinator.teardown(cx));
    }
}

impl RenderOnce for LottieStickerRepresentable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let coordinator = self.make_coordinator(window, cx);
        let host = self.make_ui_view(window, cx);
        let swap_window = window.window_handle();
        let swap_key = (coordinator.entity_id().as_u64() as u32).wrapping_add(1);
        let casts_shadow = self.casts_shadow;
        let paint_view = window.current_view();
        coordinator.update(cx, |coordinator, _| {
            coordinator.bind_swap(swap_window, swap_key);
            coordinator.casts_shadow = casts_shadow;
            coordinator.paint_view = Some(paint_view);
        });
        if coordinator.read(cx).host.is_none() {
            coordinator.update(cx, |coordinator, cx| coordinator.bind(&host, cx));
        }
        let playback_scope = nearest_animated_media_playback_scope_view(cx);
        let viewport_scope = nearest_animated_media_viewport_scope_view(cx);
        let is_host_playback_scope_active = playback_scope
            .as_deref()
            .map(AnimatedMediaPlaybackScopeProviding::is_animated_media_playback_active)
            != Some(false);
        host.update(cx, |host, cx| host.set_playback_scope_active(is_host_playback_scope_active, cx));
        self.update_ui_view(&coordinator, cx);

        let (contents, frame_painted) = {
            let coordinator = coordinator.read(cx);
            (coordinator.frame_view.clone(), coordinator.frame_painted.clone())
        };
        let (is_placeholder_hidden, placeholder_alpha, is_failure_icon_hidden) = {
            let host = host.read(cx);
            (host.is_placeholder_hidden, host.placeholder_alpha, host.is_failure_icon_hidden)
        };
        let appearance = rise_ui::theme(cx).appearance;
        let side = px(self.size);
        let placeholder_image = (contents.is_none() && !is_placeholder_hidden && is_failure_icon_hidden)
            .then(|| {
                placeholder_frame(
                    self.size,
                    UIColor::secondary_system_fill().resolved_color(appearance),
                    placeholder_alpha,
                )
            })
            .flatten();
        let painted_placeholder = placeholder_image.clone();
        coordinator.update(cx, |coordinator, _| coordinator.painted_placeholder = painted_placeholder);

        let probe_host = host.clone();
        let replay_host = host.clone();
        let visibility = canvas(
            move |bounds, window, _cx| {
                let is_in_viewport_scope = StickerVisibilityProbe::is_in_viewport_scope(viewport_scope.as_deref());
                let mask = window.content_mask().bounds;
                let moved_host = replay_host.clone();
                window.on_replayed_move(move |offset, mask_offset, window, cx| {
                    let moved = StickerVisibilityProbe::measure_at(
                        bounds + offset,
                        mask + mask_offset,
                        window.viewport_size(),
                        is_in_viewport_scope,
                    );
                    let host = moved_host.clone();
                    window.defer(cx, move |_window, cx| host.update(cx, |host, cx| host.layout_subviews(moved, cx)));
                });
                StickerVisibilityProbe::measure_at(bounds, mask, window.viewport_size(), is_in_viewport_scope)
            },
            move |_bounds, measured, window, cx| {
                window.defer(cx, move |_window, cx| {
                    probe_host.update(cx, |host, cx| host.layout_subviews(measured, cx));
                });
            },
        )
        .absolute()
        .size_full();

        let placeholder = (!is_placeholder_hidden && placeholder_image.is_none()).then(|| {
            div()
                .absolute()
                .size_full()
                .rounded(px(PLACEHOLDER_CORNER_RADIUS))
                .bg(UIColor::secondary_system_fill().resolved_color(appearance))
                .opacity(placeholder_alpha)
        });
        let placeholder_painted = frame_painted.clone();
        let frame_view = contents.map(|image| LottieFrameSwapKeyed {
            key: swap_key,
            painted: frame_painted,
            child: img(ImageSource::Render(image.clone()))
                .absolute()
                .size_full()
                .object_fit(ObjectFit::Contain)
                .alpha_shadow_source(image, ObjectFit::Contain)
                .into_any_element(),
        });
        let frame_view = frame_view.or_else(|| {
            placeholder_image.map(|image| LottieFrameSwapKeyed {
                key: swap_key,
                painted: placeholder_painted,
                child: img(ImageSource::Render(image)).absolute().size_full().object_fit(ObjectFit::Contain).into_any_element(),
            })
        });
        let failure_icon = (!is_failure_icon_hidden).then(|| {
            div()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized(
                    FAILURE_ICON_SYMBOL,
                    px(self.size.min(FAILURE_ICON_MAXIMUM_SIDE)),
                    UIColor::secondary_label().resolved_color(appearance),
                    false,
                ))
        });

        div()
            .relative()
            .overflow_hidden()
            .w(side)
            .h(side)
            .children(placeholder)
            .children(frame_view)
            .children(failure_icon)
            .child(visibility)
            .on_disappear((self.id, "lottie-sticker-view"), move |_window, cx| {
                Self::dismantle_ui_view(&coordinator, cx);
            })
    }
}

thread_local! {
    static PLACEHOLDER_FRAMES: RefCell<HashMap<(i64, [u8; 4]), Arc<RenderImage>>> = RefCell::new(HashMap::new());
}

fn placeholder_frame(point_side: f32, color: Hsla, alpha: f32) -> Option<Arc<RenderImage>> {
    if point_side <= 0.0 {
        return None;
    }
    let side = RiseLottieRasterPolicy::dimension(point_side);
    let rgba = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    let opacity = (rgba.a * alpha).clamp(0.0, 1.0);
    let key = (side, [channel(rgba.r), channel(rgba.g), channel(rgba.b), channel(opacity)]);
    if let Some(image) = PLACEHOLDER_FRAMES.with(|frames| frames.borrow().get(&key).cloned()) {
        return Some(image);
    }
    let pixels = u32::try_from(side).ok()?;
    let extent = side as f32;
    let radius = (PLACEHOLDER_CORNER_RADIUS * extent / point_side).min(extent / 2.0);
    let mut buffer = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let center_x = x as f32 + 0.5;
            let center_y = y as f32 + 0.5;
            let dx = (radius - center_x).max(center_x - (extent - radius)).max(0.0);
            let dy = (radius - center_y).max(center_y - (extent - radius)).max(0.0);
            let coverage = if dx > 0.0 && dy > 0.0 {
                (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let alpha = opacity * coverage;
            buffer.extend_from_slice(&[channel(rgba.b * alpha), channel(rgba.g * alpha), channel(rgba.r * alpha), channel(alpha)]);
        }
    }
    let bitmap = RgbaImage::from_raw(pixels, pixels, buffer)?;
    let frame = Frame::from_parts(bitmap, 0, 0, Delay::from_numer_denom_ms(0, 1));
    let frames: SmallVec<[Frame; 1]> = SmallVec::from_buf([frame]);
    let image = Arc::new(RenderImage::new(frames));
    PLACEHOLDER_FRAMES.with(|frames| frames.borrow_mut().insert(key, image.clone()));
    Some(image)
}

/// frame image of a sticker painted under the coordinator image swap key
/// a new frame replaces the picture in the frame on screen instead of a notify
/// that would render every view above the sticker
struct LottieFrameSwapKeyed {
    key: u32,
    child: AnyElement,
    painted: Rc<Cell<bool>>,
}

impl IntoElement for LottieFrameSwapKeyed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LottieFrameSwapKeyed {
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
        self.painted.set(true);
        window.with_image_swap_key(self.key, |window| self.child.paint(window, cx));
    }
}

pub struct LottieStickerRepresentableCoordinator {
    frame_view: Option<Arc<RenderImage>>,
    swap_window: Option<AnyWindowHandle>,
    swap_key: u32,
    frame_painted: Rc<Cell<bool>>,
    notified_unpainted: bool,
    paint_view: Option<EntityId>,
    painted_placeholder: Option<Arc<RenderImage>>,
    swapped_placeholder: bool,
    casts_shadow: bool,
    host: Option<WeakEntity<StickerHostView>>,
    requested_url: String,
    requested_playback_id: String,
    requested_point_size: Size<Pixels>,
    loaded_url: String,
    desired_playing: bool,
    desired_loop: bool,
    playback_kind: AnimatedMediaPlaybackKind,
    is_visible: bool,
    is_playback_scope_active: bool,
    is_loaded: bool,
    has_completed_one_shot: bool,
    is_frozen: bool,
    owns_playback_slot: bool,
    is_waiting_for_playback_slot: bool,
    load_task: Option<Task<()>>,
    reconnect_retry: Option<RiseTransportDropWatch>,
    used_link_retry: bool,
    frame_preparation_task: Option<Task<()>>,
    residency_release_task: Option<Task<()>>,
    resting_frame_task: Option<Task<()>>,
    frame_sequence: Option<Arc<RiseLottieFrameSequence>>,
    retained_sequence_key: Option<RiseLottieSequenceKey>,
    frame_renderer_generation: u64,
    current_frame_index: i64,
    has_presented_first_frame: bool,
    has_delivered_first_frame: bool,
    is_first_frame_delivery_scheduled: bool,
    first_frame_delivery_generation: u64,
    first_frame_callback: Option<LottieStickerFirstFrame>,
    completion_callback: Option<LottieStickerCompletion>,
    requested_mirrored: bool,
    playback_token: AnimatedMediaParticipantId,
    premium_effect_observer: Option<(RiseSubscription, Task<()>)>,
}

impl LottieStickerRepresentableCoordinator {
    fn new() -> Self {
        Self {
            frame_view: None,
            swap_window: None,
            swap_key: 0,
            frame_painted: Rc::new(Cell::new(true)),
            paint_view: None,
            notified_unpainted: false,
            painted_placeholder: None,
            swapped_placeholder: false,
            casts_shadow: false,
            host: None,
            requested_url: String::new(),
            requested_playback_id: String::new(),
            requested_point_size: size(px(DEFAULT_SIZE), px(DEFAULT_SIZE)),
            loaded_url: String::new(),
            desired_playing: true,
            desired_loop: true,
            playback_kind: AnimatedMediaPlaybackKind::Lottie,
            is_visible: false,
            is_playback_scope_active: true,
            is_loaded: false,
            has_completed_one_shot: false,
            is_frozen: false,
            owns_playback_slot: false,
            is_waiting_for_playback_slot: false,
            load_task: None,
            reconnect_retry: None,
            used_link_retry: false,
            frame_preparation_task: None,
            residency_release_task: None,
            resting_frame_task: None,
            frame_sequence: None,
            retained_sequence_key: None,
            frame_renderer_generation: 0,
            current_frame_index: 0,
            has_presented_first_frame: false,
            has_delivered_first_frame: false,
            is_first_frame_delivery_scheduled: false,
            first_frame_delivery_generation: 0,
            first_frame_callback: None,
            completion_callback: None,
            requested_mirrored: false,
            playback_token: AnimatedMediaParticipantId::next(),
            premium_effect_observer: None,
        }
    }

    /// removes this target from the shared display driver
    fn remove_from_display_driver(cx: &mut Context<Self>) {
        let target = cx.entity();
        RiseLottieDisplayDriver::shared(cx, |driver, _| driver.remove(&target));
    }

    fn bound_host(&self) -> Option<Entity<StickerHostView>> {
        self.host.as_ref().and_then(WeakEntity::upgrade)
    }

    fn with_host(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut StickerHostView, &mut Context<StickerHostView>)) {
        if let Some(host) = self.bound_host() {
            host.update(cx, f);
        }
    }

    fn set_frame_view(&mut self, contents: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        let unchanged = match (&self.frame_view, &contents) {
            (None, None) => true,
            (Some(current), Some(next)) => Arc::ptr_eq(current, next),
            _ => false,
        };
        if unchanged {
            return;
        }
        let previous = self
            .frame_view
            .clone()
            .or_else(|| (!self.casts_shadow).then(|| self.painted_placeholder.clone()).flatten());
        let swapped = match (&previous, &contents) {
            (Some(previous), Some(next)) => self.swap_frame_in_place(previous, next, cx),
            _ => false,
        };
        if swapped && self.frame_view.is_none() {
            self.painted_placeholder = None;
            self.swapped_placeholder = true;
        }
        let advances_frame = previous.is_some() && contents.is_some();
        let had_previous = previous.is_some();
        let has_next = contents.is_some();
        self.frame_view = contents;
        if swapped {
            return;
        }
        if !swapped && (self.current_frame_index % 30 == 0 || !self.frame_painted.get()) {
            tracing::warn!(target: "riseonly::X2260", url = %self.requested_url, index = self.current_frame_index, had_previous, has_next, painted = self.frame_painted.get(), notified_unpainted = self.notified_unpainted, swap_bound = self.swap_window.is_some(), shadow = self.casts_shadow, visible = self.is_visible, "sticker frame not swapped");
        }
        if advances_frame && !self.frame_painted.get() {
            if self.notified_unpainted {
                return;
            }
            self.notified_unpainted = true;
        } else {
            self.notified_unpainted = false;
        }
        self.frame_painted.set(false);
        cx.notify();
        if let Some(view) = self.paint_view {
            let app: &mut App = cx;
            app.notify(view);
        }
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

    fn set_frame_sequence(&mut self, sequence: Option<Arc<RiseLottieFrameSequence>>) {
        self.frame_sequence = sequence;
    }

    /// the host reports visibility and scope changes here
    /// reports reach the coordinator on a later main loop turn since both cant be borrowed at once
    fn bind(&mut self, host: &Entity<StickerHostView>, cx: &mut Context<Self>) {
        self.host = Some(host.downgrade());
        self.observe_premium_effect_starts(cx);
        let coordinator = cx.weak_entity();
        let bound = host.downgrade();
        let visibility_changed: StickerHostChange = Rc::new(move |visible: bool, cx: &mut App| {
            let coordinator = coordinator.clone();
            let bound = bound.clone();
            cx.defer(move |cx| {
                coordinator
                    .update(cx, |this, cx| {
                        if this.host.as_ref().is_some_and(|host| *host == bound) {
                            this.set_visible(visible, cx);
                        }
                    })
                    .ok();
            });
        });
        let coordinator = cx.weak_entity();
        let bound = host.downgrade();
        let playback_scope_changed: StickerHostChange = Rc::new(move |is_active: bool, cx: &mut App| {
            let coordinator = coordinator.clone();
            let bound = bound.clone();
            cx.defer(move |cx| {
                coordinator
                    .update(cx, |this, cx| {
                        if this.host.as_ref().is_some_and(|host| *host == bound) {
                            this.set_playback_scope_active(is_active, cx);
                        }
                    })
                    .ok();
            });
        });
        let (is_playback_scope_active, is_effectively_visible) = host.update(cx, |host, _| {
            host.visibility_changed = Some(visibility_changed);
            host.playback_scope_changed = Some(playback_scope_changed);
            (host.is_playback_scope_active(), host.is_effectively_visible())
        });
        self.set_playback_scope_active(is_playback_scope_active, cx);
        self.set_visible(is_effectively_visible, cx);
    }

    /// gpui calls it on every render so every step below acts on a change only
    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        url: &str,
        playback_id: &str,
        r#loop: bool,
        is_playing: bool,
        is_frozen: bool,
        point_size: Size<Pixels>,
        kind: AnimatedMediaPlaybackKind,
        on_first_frame: Option<LottieStickerFirstFrame>,
        on_complete: Option<LottieStickerCompletion>,
        mirrored: bool,
        cx: &mut Context<Self>,
    ) {
        let normalized_url = sai_remote_media_url_optional_normalized(Some(url))
            .unwrap_or_else(|| trimming_whitespaces_and_newlines(url).to_owned());
        let source_changed = self.requested_url != normalized_url
            || self.requested_playback_id != playback_id
            || self.requested_mirrored != mirrored;
        self.requested_mirrored = mirrored;
        self.completion_callback = on_complete;
        let restarts_one_shot = !self.desired_playing && is_playing;
        let loop_changed = self.desired_loop != r#loop;
        self.desired_playing = is_playing;
        self.desired_loop = r#loop;
        self.playback_kind = kind;
        self.requested_point_size = point_size;
        self.first_frame_callback = on_first_frame;
        if restarts_one_shot {
            if self.has_completed_one_shot
                && let Some(sequence) = &self.frame_sequence
            {
                self.current_frame_index = sequence.cached_visible_frame_index().unwrap_or(0);
            }
            self.has_completed_one_shot = false;
        }
        if loop_changed && !source_changed {
            self.apply_loop_change(cx);
        }
        if source_changed {
            self.reset_source(normalized_url, playback_id.to_owned(), cx);
        }
        if is_frozen != self.is_frozen {
            self.is_frozen = is_frozen;
            if is_frozen && !source_changed {
                self.rest_on_poster(cx);
            }
        }
        if self.requested_url.is_empty() {
            self.pause_and_release_slot(cx);
            self.with_host(cx, |host, cx| host.show_failed(cx));
            return;
        }
        self.with_host(cx, |host, cx| host.resync_scopes(cx));
        self.begin_load_if_needed(cx);
        self.apply_playback_state(cx);
        self.schedule_first_frame_delivery_if_needed(cx);
    }

    fn apply_loop_change(&mut self, cx: &mut Context<Self>) {
        if self.desired_loop && self.has_completed_one_shot {
            self.has_completed_one_shot = false;
            self.apply_playback_state(cx);
        }
        if !self.owns_playback_slot {
            return;
        }
        if self.frame_sequence.is_some() {
            Self::remove_from_display_driver(cx);
            self.start_frame_playback(cx);
        }
    }

    fn teardown(&mut self, cx: &mut Context<Self>) {
        self.invalidate_first_frame_delivery();
        self.load_task = None;
        if let Some(retry) = self.reconnect_retry.take() {
            retry.stop();
        }
        self.frame_preparation_task = None;
        self.residency_release_task = None;
        self.frame_renderer_generation = self.frame_renderer_generation.wrapping_add(1);
        self.release_sequence(cx);
        self.pause_and_release_slot(cx);
        self.set_frame_view(None, cx);
        self.with_host(cx, |host, _| {
            host.visibility_changed = None;
            host.playback_scope_changed = None;
            host.stop_visibility_tracking();
        });
        self.host = None;
        self.requested_url.clear();
        self.requested_playback_id.clear();
        self.loaded_url.clear();
        self.desired_playing = false;
        self.is_visible = false;
        self.is_playback_scope_active = false;
        self.is_loaded = false;
        self.has_presented_first_frame = false;
        self.first_frame_callback = None;
    }

    fn reset_source(&mut self, url: String, playback_id: String, cx: &mut Context<Self>) {
        self.swapped_placeholder = false;
        self.invalidate_first_frame_delivery();
        self.load_task = None;
        self.frame_preparation_task = None;
        self.residency_release_task = None;
        self.frame_renderer_generation = self.frame_renderer_generation.wrapping_add(1);
        self.release_sequence(cx);
        self.pause_and_release_slot(cx);
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let cached_frame = if self.requested_mirrored {
            None
        } else {
            RiseLottieFirstFrameCache::memory_image(&url, self.requested_point_size, account_user_id.as_deref())
                .or_else(|| RiseLottieFirstFrameCache::latest_memory_image(&url, account_user_id.as_deref()))
        };
        let has_cached_frame = cached_frame.is_some();
        if has_cached_frame {
            self.set_frame_view(cached_frame, cx);
        } else if self.requested_playback_id != playback_id || self.requested_url != url {
            self.set_frame_view(None, cx);
        }
        self.requested_url = url.clone();
        self.requested_playback_id = playback_id;
        self.used_link_retry = false;
        self.loaded_url.clear();
        self.is_loaded = false;
        self.has_completed_one_shot = false;
        self.has_presented_first_frame = false;
        self.with_host(cx, |host, cx| host.show_placeholder(cx));
        if has_cached_frame {
            self.reveal_first_frame(cx);
        }
        if self.is_visible
            && let Some(payload) = LottieStickerLoader::cached_payload(&url)
        {
            self.loaded_url = url.clone();
            self.install(payload, url, cx);
        }
    }

    fn begin_load_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.is_visible
            || self.is_loaded
            || self.load_task.is_some()
            || self.frame_preparation_task.is_some()
            || self.requested_url.is_empty()
            || self.loaded_url == self.requested_url
        {
            return;
        }
        let url = self.requested_url.clone();
        self.loaded_url = url.clone();

        if let Some(payload) = LottieStickerLoader::cached_payload(&url) {
            self.install(payload, url, cx);
            return;
        }

        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = LottieStickerLoader::shared().payload(&url).await;
            this.update(cx, |this, cx| this.finish_load(url, result, cx)).ok();
        }));
    }

    /// tail of the load task, back on the main thread
    fn finish_load(
        &mut self,
        url: String,
        result: Result<LottieStickerPayload, LottieStickerLoaderError>,
        cx: &mut Context<Self>,
    ) {
        let error = match result {
            Ok(payload) => {
                if !self.is_visible || self.requested_url != url || self.loaded_url != url {
                    return;
                }
                self.install(payload, url, cx);
                return;
            }
            Err(error) => error,
        };
        if self.requested_url != url {
            return;
        }
        self.load_task = None;
        self.loaded_url.clear();
        self.is_loaded = false;
        self.pause_and_release_slot(cx);
        if let Some(retry) = self.reconnect_retry.take() {
            retry.stop();
        }
        if RiseTransportStatus::is_connected(cx) {
            let retries = matches!(
                &error,
                LottieStickerLoaderError::Transfer(transfer)
                    if RemoteMediaDownloadRetryPolicy::should_retry(transfer, 0)
            );
            if !self.used_link_retry && retries {
                self.used_link_retry = true;
                self.with_host(cx, |host, cx| host.show_placeholder(cx));
                self.begin_load_if_needed(cx);
            } else {
                self.with_host(cx, |host, cx| host.show_failed(cx));
            }
        } else {
            self.with_host(cx, |host, cx| host.show_placeholder(cx));
            let coordinator = cx.weak_entity();
            self.reconnect_retry = Some(RiseTransportStatus::on_next_connect(
                move |cx: &mut App| {
                    coordinator
                        .update(cx, |this, cx| {
                            this.reconnect_retry = None;
                            this.begin_load_if_needed(cx);
                        })
                        .ok();
                },
                cx,
            ));
        }
    }

    fn install(&mut self, payload: LottieStickerPayload, source_url: String, cx: &mut Context<Self>) {
        if self.requested_url != source_url || self.loaded_url != source_url || !self.is_visible {
            return;
        }
        self.load_task = None;
        match payload {
            LottieStickerPayload::Animation(render_source) => match render_source.container {
                LottieStickerContainer::Ros | LottieStickerContainer::TgsLegacy | LottieStickerContainer::Json => {
                    self.install_sequence(render_source, source_url, cx);
                }
                LottieStickerContainer::DotLottie => {
                    self.install_airbnb(LottieStickerPayload::Animation(render_source), source_url, None, cx);
                }
            },
            payload @ LottieStickerPayload::DotLottie(_) => self.install_airbnb(payload, source_url, None, cx),
        }
    }

    fn install_sequence(&mut self, source: LottieStickerRenderSource, source_url: String, cx: &mut Context<Self>) {
        self.frame_preparation_task = None;
        self.frame_renderer_generation = self.frame_renderer_generation.wrapping_add(1);
        let generation = self.frame_renderer_generation;
        let point_size = self.requested_point_size;
        let started_at = RiseClock::monotonic();
        if self.frame_view.is_none() && !self.requested_mirrored {
            let cached_frame = RiseLottieFirstFrameCache::memory_image(
                &source_url,
                point_size,
                SaiAccountCacheScope::active_user_id().as_deref(),
            );
            let has_cached_frame = cached_frame.is_some();
            self.set_frame_view(cached_frame, cx);
            if has_cached_frame {
                self.reveal_first_frame(cx);
            }
        }
        let key = RiseLottieSequenceKey {
            cache_key: source.cache_key.clone(),
            dimension: RiseLottieRasterPolicy::dimension(f32::from(point_size.width.max(point_size.height))),
            mirrored: self.requested_mirrored,
        };
        self.release_sequence(cx);
        self.retained_sequence_key = Some(key.clone());
        RiseLottieFrameCache::shared().retain(&key);

        if let Some(resident) = RiseLottieFrameCache::shared().cached_sequence(&key)
            && let Some(index) = resident.cached_visible_frame_index()
            && let Some(image) = resident.cached_frame(index)
        {
            self.set_frame_sequence(Some(resident));
            self.current_frame_index = index;
            self.set_frame_view(Some(image), cx);
            self.is_loaded = true;
            self.reveal_first_frame(cx);
            self.apply_playback_state(cx);
            return;
        }

        let json_data = source.json_data.clone();
        self.frame_preparation_task = Some(cx.spawn(async move |this, cx| {
            let sequence = RiseLottieFrameCache::shared().sequence(&key, json_data, cx).await;
            let prepared = this
                .update(cx, |this, cx| {
                    if generation != this.frame_renderer_generation
                        || this.requested_url != source_url
                        || this.loaded_url != source_url
                    {
                        return None;
                    }
                    if sequence.is_none() {
                        this.frame_preparation_task = None;
                        this.install_airbnb(LottieStickerPayload::Animation(source.clone()), source_url.clone(), None, cx);
                    }
                    sequence
                })
                .ok()
                .flatten();
            let Some(sequence) = prepared else {
                return;
            };
            let visible_index = sequence.first_visible_frame_index().await;
            this.update(cx, |this, cx| {
                this.finish_sequence_preparation(sequence, visible_index, source, source_url, generation, started_at, cx);
            })
            .ok();
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_sequence_preparation(
        &mut self,
        sequence: Arc<RiseLottieFrameSequence>,
        visible_index: Option<i64>,
        source: LottieStickerRenderSource,
        source_url: String,
        generation: u64,
        started_at: Duration,
        cx: &mut Context<Self>,
    ) {
        if generation != self.frame_renderer_generation
            || self.requested_url != source_url
            || self.loaded_url != source_url
        {
            return;
        }
        let Some(visible_index) = visible_index else {
            self.frame_preparation_task = None;
            self.install_airbnb(LottieStickerPayload::Animation(source), source_url, Some(sequence), cx);
            chat_performance_trace_record(RLOTTIE_PREPARE_EVENT, || {
                vec![(FIELD_RESULT.to_owned(), RESULT_TRANSPARENT_FALLBACK.to_owned())]
            });
            return;
        };
        self.set_frame_sequence(Some(sequence.clone()));
        self.current_frame_index = visible_index;
        self.frame_preparation_task = None;
        self.is_loaded = true;
        self.present_poster_frame(&sequence, visible_index, source_url, cx);
        let elapsed = RiseClock::monotonic().saturating_sub(started_at);
        chat_performance_trace_record(RLOTTIE_PREPARE_EVENT, || {
            vec![
                (otp::DURATION_MS.to_owned(), format!("{:.3}", elapsed.as_secs_f64() * 1_000.0)),
                (FIELD_FRAMES.to_owned(), sequence.frame_count.to_string()),
                (FIELD_FPS.to_owned(), format!("{:.2}", sequence.frame_rate)),
                (FIELD_RESULT.to_owned(), RESULT_READY.to_owned()),
            ]
        });
        self.apply_playback_state(cx);
    }

    fn present_poster_frame(
        &mut self,
        sequence: &Arc<RiseLottieFrameSequence>,
        index: i64,
        source_url: String,
        cx: &mut Context<Self>,
    ) {
        let point_size = self.requested_point_size;
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let stores_poster = !self.requested_mirrored;
        if let Some(image) = sequence.cached_frame(index) {
            self.set_frame_view(Some(image.clone()), cx);
            self.reveal_first_frame(cx);
            if stores_poster {
                Self::store_poster(
                    image,
                    source_url,
                    point_size,
                    account_user_id,
                    account_epoch,
                    cx.background_executor(),
                );
            }
            return;
        }
        let generation = self.frame_renderer_generation;
        let (sender, receiver) = oneshot::channel();
        sequence.request_frame(
            index,
            Arc::new(|| true),
            Some(Box::new(move |image: Option<Arc<RenderImage>>| {
                let _ = sender.send(image);
            })),
        );
        cx.spawn(async move |this, cx| {
            let Ok(Some(image)) = receiver.await else {
                return;
            };
            if stores_poster {
                Self::store_poster(
                    image.clone(),
                    source_url.clone(),
                    point_size,
                    account_user_id,
                    account_epoch,
                    cx.background_executor(),
                );
            }
            this.update(cx, |this, cx| {
                if generation != this.frame_renderer_generation || this.requested_url != source_url {
                    return;
                }
                if this.frame_view.is_none() || this.current_frame_index == index {
                    this.set_frame_view(Some(image), cx);
                }
                this.reveal_first_frame(cx);
            })
            .ok();
        })
        .detach();
    }

    /// neighbours playing in the same grid can evict the poster frame from the decoded cache
    /// the first frame cache keeps it so a reopened sheet shows it with no decode
    fn store_poster(
        image: Arc<RenderImage>,
        source_url: String,
        point_size: Size<Pixels>,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        executor: &BackgroundExecutor,
    ) {
        if RiseLottieFirstFrameCache::memory_image(&source_url, point_size, account_user_id.as_deref()).is_some() {
            return;
        }
        executor
            .spawn(async move {
                RiseLottieFirstFrameCache::store(
                    image,
                    &source_url,
                    point_size,
                    account_user_id.as_deref(),
                    account_epoch.as_deref(),
                );
            })
            .detach();
    }

    fn release_sequence(&mut self, cx: &mut Context<Self>) {
        self.resting_frame_task = None;
        Self::remove_from_display_driver(cx);
        if let Some(retained_sequence_key) = self.retained_sequence_key.take() {
            RiseLottieFrameCache::shared().release(&retained_sequence_key);
        }
        self.set_frame_sequence(None);
    }

    /// art the standalone rasterizer cannot take, or whose frames came out empty
    /// goes to the layer engine for live display
    fn install_airbnb(
        &mut self,
        _payload: LottieStickerPayload,
        source_url: String,
        live_sequence: Option<Arc<RiseLottieFrameSequence>>,
        cx: &mut Context<Self>,
    ) {
        if self.requested_url != source_url || self.loaded_url != source_url || self.bound_host().is_none() {
            return;
        }
        let install_token = chat_performance_trace_begin_operation(LOTTIE_INSTALL_OPERATION);
        self.frame_preparation_task = None;
        self.frame_renderer_generation = self.frame_renderer_generation.wrapping_add(1);
        self.release_sequence(cx);
        let Some(sequence) = live_sequence else {
            chat_performance_trace_end_operation(install_token, LOTTIE_INSTALL_THRESHOLD_MS, Vec::new);
            self.with_host(cx, |host, cx| host.show_failed(cx));
            return;
        };
        let key = sequence.key.clone();
        RiseLottieFrameCache::shared().retain(&key);
        self.retained_sequence_key = Some(key);
        self.set_frame_sequence(Some(sequence.clone()));
        self.show_still(0, &sequence, cx);
        chat_performance_trace_end_operation(install_token, LOTTIE_INSTALL_THRESHOLD_MS, || {
            vec![(FIELD_ENGINE.to_owned(), ENGINE_RLOTTIE.to_owned())]
        });
        self.is_loaded = true;
        self.reveal_first_frame(cx);
        self.apply_playback_state(cx);
    }

    fn reveal_first_frame(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.swapped_placeholder) {
            self.with_host(cx, |host, _| host.hide_placeholder_in_place());
        } else {
            self.with_host(cx, |host, cx| host.hide_placeholder(cx));
        }
        self.has_presented_first_frame = true;
        self.schedule_first_frame_delivery_if_needed(cx);
    }

    fn invalidate_first_frame_delivery(&mut self) {
        self.first_frame_delivery_generation = self.first_frame_delivery_generation.wrapping_add(1);
        self.has_delivered_first_frame = false;
        self.is_first_frame_delivery_scheduled = false;
    }

    fn schedule_first_frame_delivery_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.has_presented_first_frame
            || self.has_delivered_first_frame
            || self.is_first_frame_delivery_scheduled
            || self.first_frame_callback.is_none()
        {
            return;
        }
        self.is_first_frame_delivery_scheduled = true;
        let generation = self.first_frame_delivery_generation;
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| {
                if this.first_frame_delivery_generation != generation {
                    return;
                }
                this.is_first_frame_delivery_scheduled = false;
                if !this.has_presented_first_frame || this.has_delivered_first_frame {
                    return;
                }
                let Some(callback) = this.first_frame_callback.clone() else {
                    return;
                };
                this.has_delivered_first_frame = true;
                cx.defer(move |cx| callback(cx));
            })
            .ok();
        })
        .detach();
    }

    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.is_visible {
            return;
        }
        self.is_visible = visible;
        if visible {
            self.residency_release_task = None;
            self.begin_load_if_needed(cx);
            self.apply_playback_state(cx);
        } else {
            if !self.is_loaded {
                self.load_task = None;
                self.frame_preparation_task = None;
                self.frame_renderer_generation = self.frame_renderer_generation.wrapping_add(1);
                self.release_sequence(cx);
                self.loaded_url.clear();
            }
            self.pause_and_release_slot(cx);
            self.schedule_renderer_residency_release(cx);
        }
    }

    fn schedule_renderer_residency_release(&mut self, cx: &mut Context<Self>) {
        if !self.is_loaded {
            return;
        }
        self.residency_release_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RESIDENCY_RELEASE_DELAY).await;
            this.update(cx, |this, cx| this.release_renderer_residency(cx)).ok();
        }));
    }

    fn release_renderer_residency(&mut self, cx: &mut Context<Self>) {
        if self.is_visible {
            return;
        }
        self.residency_release_task = None;
        self.frame_preparation_task = None;
        self.frame_renderer_generation = self.frame_renderer_generation.wrapping_add(1);
        self.release_sequence(cx);
        if self.frame_view.is_none() && !self.requested_mirrored {
            let account_user_id = SaiAccountCacheScope::active_user_id();
            let poster = RiseLottieFirstFrameCache::memory_image(
                &self.requested_url,
                self.requested_point_size,
                account_user_id.as_deref(),
            )
            .or_else(|| RiseLottieFirstFrameCache::latest_memory_image(&self.requested_url, account_user_id.as_deref()));
            self.set_frame_view(poster, cx);
        }
        self.loaded_url.clear();
        self.is_loaded = false;
        self.current_frame_index = 0;
        self.has_presented_first_frame = self.frame_view.is_some();
        if !self.has_presented_first_frame {
            self.with_host(cx, |host, cx| host.show_placeholder(cx));
        }
    }

    fn set_playback_scope_active(&mut self, is_active: bool, cx: &mut Context<Self>) {
        if self.is_playback_scope_active == is_active {
            return;
        }
        self.is_playback_scope_active = is_active;
        if is_active {
            self.begin_load_if_needed(cx);
        }
        self.apply_playback_state(cx);
    }

    fn apply_playback_state(&mut self, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X2260", url = %self.requested_url, loaded = self.is_loaded, visible = self.is_visible, scope = self.is_playback_scope_active, playing = self.desired_playing, one_shot_done = self.has_completed_one_shot, owns = self.owns_playback_slot, waiting = self.is_waiting_for_playback_slot, "sticker playback state");
        if !self.is_loaded
            || !self.is_visible
            || !self.is_playback_scope_active
            || !self.desired_playing
            || self.has_completed_one_shot
        {
            self.pause_and_release_slot(cx);
            return;
        }
        self.request_playback_slot_if_needed(cx);
    }

    fn request_playback_slot_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.owns_playback_slot || self.is_waiting_for_playback_slot {
            return;
        }
        let token = self.playback_token;
        self.is_waiting_for_playback_slot = true;
        let granted = cx.weak_entity();
        let revoked = cx.weak_entity();
        LottieStickerPlaybackHub::shared().request(
            token,
            self.playback_kind,
            move |cx: &mut App| {
                let still_here = granted.update(cx, |this, cx| this.playback_slot_granted(cx)).is_ok();
                if !still_here {
                    LottieStickerPlaybackHub::shared().release(token, cx);
                }
            },
            move |cx: &mut App| {
                revoked.update(cx, |this, cx| this.playback_slot_revoked(cx)).ok();
            },
            cx,
        );
    }

    fn playback_slot_granted(&mut self, cx: &mut Context<Self>) {
        self.is_waiting_for_playback_slot = false;
        tracing::warn!(target: "riseonly::X2260", url = %self.requested_url, playing = self.desired_playing, visible = self.is_visible, scope = self.is_playback_scope_active, loaded = self.is_loaded, sequence = self.frame_sequence.is_some(), kind = ?self.playback_kind, "sticker playback slot granted");
        if !self.desired_playing || !self.is_visible || !self.is_playback_scope_active || !self.is_loaded {
            LottieStickerPlaybackHub::shared().release(self.playback_token, cx);
            return;
        }
        self.owns_playback_slot = true;
        if self.frame_sequence.is_some() {
            self.start_frame_playback(cx);
            return;
        }
        self.owns_playback_slot = false;
        LottieStickerPlaybackHub::shared().release(self.playback_token, cx);
    }

    fn start_frame_playback(&mut self, cx: &mut Context<Self>) {
        if self.frame_sequence.is_none() {
            return;
        }
        let target = cx.entity();
        cx.defer(move |cx| {
            let (loops, starting_at) = {
                let this = target.read(cx);
                if !this.owns_playback_slot || this.frame_sequence.is_none() {
                    return;
                }
                (this.desired_loop, this.current_frame_index)
            };
            tracing::warn!(target: "riseonly::X2260", loops, starting_at, "sticker registers with display driver");
            RiseLottieDisplayDriver::shared(cx, |driver, cx| driver.add(&target, loops, starting_at, cx));
        });
    }

    fn observe_premium_effect_starts(&mut self, cx: &mut Context<Self>) {
        if self.premium_effect_observer.is_some() {
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<String>();
        let subscription = RiseAppEvents::default_center().observe(STICKER_PREMIUM_EFFECT_DID_START, move |note| {
            if let Some(playback_id) = note.payload::<String>() {
                let _ = sender.unbounded_send(playback_id.clone());
            }
        });
        let task = cx.spawn(async move |this, cx| {
            while let Some(playback_id) = receiver.next().await {
                let alive = this.update(cx, |this, cx| {
                    if this.requested_playback_id == playback_id {
                        this.restart_from_first_frame(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        self.premium_effect_observer = Some((subscription, task));
    }

    fn restart_from_first_frame(&mut self, cx: &mut Context<Self>) {
        if !self.is_loaded || self.is_frozen {
            return;
        }
        self.resting_frame_task = None;
        self.has_completed_one_shot = false;
        self.current_frame_index = 0;
        if !self.owns_playback_slot {
            self.apply_playback_state(cx);
            return;
        }
        if self.frame_sequence.is_some() {
            Self::remove_from_display_driver(cx);
            self.start_frame_playback(cx);
        }
    }

    fn one_shot_did_complete(&mut self, cx: &mut Context<Self>) {
        if self.desired_loop {
            return;
        }
        self.has_completed_one_shot = true;
        self.owns_playback_slot = false;
        self.is_waiting_for_playback_slot = false;
        LottieStickerPlaybackHub::shared().release(self.playback_token, cx);
        self.settle_on_resting_frame(cx);
        if let Some(callback) = self.completion_callback.clone() {
            cx.defer(move |cx| callback(cx));
        }
    }

    fn settle_on_resting_frame(&mut self, cx: &mut Context<Self>) {
        let Some(sequence) = self.frame_sequence.clone() else {
            return;
        };
        let finished_index = self.current_frame_index;
        let generation = self.frame_renderer_generation;
        self.resting_frame_task = Some(cx.spawn(async move |this, cx| {
            let index = sequence.resting_frame_index_after_one_shot().await;
            this.update(cx, |this, cx| {
                if generation != this.frame_renderer_generation
                    || !this.has_completed_one_shot
                    || this.current_frame_index != finished_index
                {
                    return;
                }
                this.resting_frame_task = None;
                let Some(index) = index.filter(|index| *index != finished_index) else {
                    return;
                };
                this.show_still(index, &sequence, cx);
            })
            .ok();
        }));
    }

    fn show_still(&mut self, index: i64, sequence: &Arc<RiseLottieFrameSequence>, cx: &mut Context<Self>) {
        self.current_frame_index = index;
        if let Some(image) = sequence.cached_frame(index) {
            self.set_frame_view(Some(image), cx);
            return;
        }
        let generation = self.frame_renderer_generation;
        let source_url = self.requested_url.clone();
        let (sender, receiver) = oneshot::channel();
        sequence.request_frame(
            index,
            Arc::new(|| true),
            Some(Box::new(move |image: Option<Arc<RenderImage>>| {
                let _ = sender.send(image);
            })),
        );
        cx.spawn(async move |this, cx| {
            let Ok(Some(image)) = receiver.await else {
                return;
            };
            this.update(cx, |this, cx| {
                if generation != this.frame_renderer_generation
                    || this.requested_url != source_url
                    || this.current_frame_index != index
                {
                    return;
                }
                this.set_frame_view(Some(image), cx);
            })
            .ok();
        })
        .detach();
    }

    fn playback_slot_revoked(&mut self, cx: &mut Context<Self>) {
        self.owns_playback_slot = false;
        self.is_waiting_for_playback_slot = true;
        Self::remove_from_display_driver(cx);
        if let Some(feature) = self.playback_kind.energy_feature()
            && RiseEnergySavingStore::snapshot_reduces(feature)
        {
            self.rest_on_poster(cx);
        }
    }

    /// power saving freezes a sticker on its first visible frame and not where playback stopped
    /// a one shot that already ran plays again once the freeze lifts
    fn rest_on_poster(&mut self, cx: &mut Context<Self>) {
        self.has_completed_one_shot = false;
        if let Some(sequence) = self.frame_sequence.clone() {
            self.show_still(sequence.cached_visible_frame_index().unwrap_or(0), &sequence, cx);
        }
    }

    fn pause_and_release_slot(&mut self, cx: &mut Context<Self>) {
        Self::remove_from_display_driver(cx);
        if !self.owns_playback_slot && !self.is_waiting_for_playback_slot {
            return;
        }
        self.owns_playback_slot = false;
        self.is_waiting_for_playback_slot = false;
        LottieStickerPlaybackHub::shared().release(self.playback_token, cx);
    }
}

impl RiseLottiePlaybackTarget for LottieStickerRepresentableCoordinator {
    fn playback_sequence(&self) -> Option<Arc<RiseLottieFrameSequence>> {
        self.frame_sequence.clone()
    }

    fn present_frame(&mut self, image: Arc<RenderImage>, index: i64, cx: &mut Context<Self>) {
        self.current_frame_index = index;
        self.set_frame_view(Some(image), cx);
    }

    fn playback_did_finish_one_shot(&mut self, cx: &mut Context<Self>) {
        self.one_shot_did_complete(cx);
    }
}

pub type StickerHostChange = Rc<dyn Fn(bool, &mut App)>;

/// view hosting the sticker, placeholder
/// failure mark and visibility tracking
/// every paint reports the clipped frame
pub struct StickerHostView {
    is_placeholder_hidden: bool,
    placeholder_alpha: f32,
    is_failure_icon_hidden: bool,
    pub visibility_changed: Option<StickerHostChange>,
    pub playback_scope_changed: Option<StickerHostChange>,
    last_reported_visibility: Option<bool>,
    scene_observers: Vec<Subscription>,
    is_playback_scope_active: bool,
    last_probe: Option<StickerVisibilityProbe>,
}

impl StickerHostView {
    fn new() -> Self {
        Self {
            is_placeholder_hidden: false,
            placeholder_alpha: 1.0,
            is_failure_icon_hidden: true,
            visibility_changed: None,
            playback_scope_changed: None,
            last_reported_visibility: None,
            scene_observers: Vec::new(),
            is_playback_scope_active: true,
            last_probe: None,
        }
    }

    /// true when drawn in a window, inside its viewport scope if it has one
    /// and enough of its height shows inside every clipping parent, as of the last paint
    pub fn is_effectively_visible(&self) -> bool {
        self.last_probe.is_some_and(|probe| probe.is_visible)
    }

    pub fn is_playback_scope_active(&self) -> bool {
        self.is_playback_scope_active
    }

    fn did_move_to_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_scene_subscription(window, cx);
        self.resync_scopes(cx);
    }

    fn did_move_to_superview(&mut self, cx: &mut Context<Self>) {
        self.resync_scopes(cx);
    }

    fn layout_subviews(&mut self, probe: StickerVisibilityProbe, cx: &mut Context<Self>) {
        let first_report = self.last_probe.is_none();
        self.last_probe = Some(probe);
        if first_report {
            self.did_move_to_superview(cx);
        } else {
            self.report_visibility_if_changed(cx);
        }
    }

    fn set_playback_scope_active(&mut self, is_active: bool, cx: &mut Context<Self>) {
        if self.is_playback_scope_active == is_active {
            return;
        }
        self.is_playback_scope_active = is_active;
        self.playback_scope_did_change(cx);
    }

    fn show_placeholder(&mut self, cx: &mut Context<Self>) {
        if self.is_failure_icon_hidden && !self.is_placeholder_hidden && self.placeholder_alpha == PLACEHOLDER_ALPHA {
            return;
        }
        self.is_failure_icon_hidden = true;
        self.is_placeholder_hidden = false;
        self.placeholder_alpha = PLACEHOLDER_ALPHA;
        cx.notify();
    }

    fn hide_placeholder(&mut self, cx: &mut Context<Self>) {
        if self.is_failure_icon_hidden && self.is_placeholder_hidden {
            return;
        }
        self.hide_placeholder_in_place();
        cx.notify();
    }

    /// the first frame already took the placeholder place on screen, the next render reads it
    fn hide_placeholder_in_place(&mut self) {
        self.is_failure_icon_hidden = true;
        self.is_placeholder_hidden = true;
    }

    fn show_failed(&mut self, cx: &mut Context<Self>) {
        self.is_failure_icon_hidden = false;
        self.is_placeholder_hidden = false;
        self.placeholder_alpha = FAILED_PLACEHOLDER_ALPHA;
        cx.notify();
    }

    pub fn stop_visibility_tracking(&mut self) {
        self.remove_scene_observers();
        self.last_reported_visibility = None;
    }

    fn resync_scopes(&mut self, cx: &mut Context<Self>) {
        if let Some(playback_scope_changed) = self.playback_scope_changed.clone() {
            playback_scope_changed(self.is_playback_scope_active, cx);
        }
        self.report_visibility_if_changed(cx);
    }

    fn refresh_scene_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remove_scene_observers();
        self.scene_observers.push(cx.observe_window_activation(window, |this, _window, cx| {
            this.resync_scopes(cx);
            cx.notify();
        }));
    }

    fn playback_scope_did_change(&mut self, cx: &mut Context<Self>) {
        if let Some(playback_scope_changed) = self.playback_scope_changed.clone() {
            playback_scope_changed(self.is_playback_scope_active, cx);
        }
        self.report_visibility_if_changed(cx);
    }

    fn remove_scene_observers(&mut self) {
        self.scene_observers.clear();
    }

    fn report_visibility_if_changed(&mut self, cx: &mut Context<Self>) {
        let visible = self.is_effectively_visible();
        if Some(visible) == self.last_reported_visibility {
            return;
        }
        self.last_reported_visibility = Some(visible);
        if let Some(visibility_changed) = self.visibility_changed.clone() {
            visibility_changed(visible, cx);
        }
    }
}

/// what one paint saw of the host frame
#[derive(Clone, Copy, Debug, PartialEq)]
struct StickerVisibilityProbe {
    is_visible: bool,
}

impl StickerVisibilityProbe {
    /// visibility verdict, the viewport scope own verdict when there is one
    /// else the frame clipped by every clipping parent and the window, ignoring the playback scope
    fn measure_at(bounds: Bounds<Pixels>, mask: Bounds<Pixels>, viewport: Size<Pixels>, is_in_viewport_scope: bool) -> Self {
        let window_bounds = Bounds::new(point(px(0.), px(0.)), viewport);
        let clip = mask.intersect(&window_bounds);
        let drawable = bounds.size.width > px(0.) && bounds.size.height > px(0.);
        let is_visible = is_in_viewport_scope
            && drawable
            && bounds.intersects(&clip)
            && {
                let visible = bounds.intersect(&clip);
                let fraction = f32::from(visible.size.height) / f32::from(bounds.size.height);
                f32::from(visible.size.width) > 1.0
                    && fraction > AnimatedMediaViewportPlaybackPolicy::MINIMUM_VISIBLE_HEIGHT_FRACTION
            };
        Self { is_visible }
    }

    fn is_in_viewport_scope(viewport_scope: Option<&dyn AnimatedMediaViewportScopeProviding>) -> bool {
        viewport_scope.is_none_or(AnimatedMediaViewportScopeProviding::is_animated_media_visible_in_viewport)
    }
}
