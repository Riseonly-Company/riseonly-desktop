use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    App, Bounds, Context, Div, Entity, FontWeight, Hsla, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent,
    PathBuilder, PinchEvent, Pixels, Point, Render, RenderImage, SharedString, Subscription, Task, TouchPhase,
    WeakEntity, Window, black, canvas, div, img, point, prelude::*, px, red, white, yellow,
};
use futures::channel::oneshot;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::capture_permission::{CaptureKind, open_settings};
use rise_platform::photo_library::{self, ImageContentMode, ImageDeliveryMode, LibraryMediaKind};
use rise_platform::haptics::haptics::{ImpactStyle, NotificationKind, impact, notification, selection_changed};
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureElement, DragGestureModifiers, DragGestureValue, GestureMask};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{ease_in, ease_out, linear, spring};
use rise_ui::swiftui::shapes::{Shape, SwiftUiShapes, centred_stroke};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{CreatePostIntent, RootRoute};
use crate::modules::camera::capture::rise_camera_capture::{
    RiseCameraCapture, RiseCameraCaptureAccess, RiseCameraCaptureFlashMode, RiseCameraShot,
};
use crate::modules::camera::capture::rise_camera_mode::RiseCameraMode;
use crate::modules::camera::capture::rise_camera_presenter::RiseCameraPresenter;
#[cfg(debug_assertions)]
use crate::modules::camera::capture::rise_camera_test_source::RiseCameraTestPreview;
use crate::modules::camera::components::rise_camera_preview::{RiseCameraPreview, RiseCameraPreviewBridge};
use crate::modules::media_picker::api::media_picker_result::MediaPickerResult;
use crate::modules::media_picker::library::photos_image_request::{PhotoImageDelivery, PhotosImageRequest};
use crate::modules::user::pages::profile::create_post::create_post_media_page::CreatePostMediaHandoff;

const HOLD_DELAY: Duration = Duration::from_millis(280);
const MODE_SLOT_WIDTH: f32 = 124.0;
const MODE_STRIP_HEIGHT: f32 = 64.0;
const PREVIEW_CORNER_RADIUS: f32 = 22.0;
const DRAG_MINIMUM_DISTANCE: f32 = 14.0;
const GALLERY_THUMBNAIL_SIDE: f32 = 132.0;
const MODE_DRAG_MINIMUM_DISTANCE: f32 = 8.0;
const RELEASE_VELOCITY_FACTOR: f32 = 4.0;
const FOCUS_SIDE: f32 = 72.0;
const FOCUS_EXIT_SCALE: f32 = 1.3;
const FOCUS_HOLD: Duration = Duration::from_millis(900);
const HINT_HOLD: Duration = Duration::from_secs(2);
const FLASH_FADE_DELAY: Duration = Duration::from_millis(60);
const ROUND_BUTTON_SIDE: f32 = 44.0;
const SHUTTER_SIDE: f32 = 100.0;
const SHUTTER_RING_IDLE: f32 = 78.0;
const SHUTTER_RING_RECORDING: f32 = 96.0;
const SHUTTER_LINE_WIDTH: f32 = 5.0;
const SHUTTER_INNER_IDLE: f32 = 64.0;
const SHUTTER_INNER_PRESSED: f32 = 58.0;
const SHUTTER_INNER_RECORDING: f32 = 34.0;
const RING_SEGMENTS: usize = 96;
const CHROME_FADE_START: f32 = 0.5;

/// feed camera, rounded preview with controls over it and the mode strip under
/// click shutter for photo and hold for video, pinch zooms and click focuses
/// double click flips, left drag slides it away
pub struct RiseCameraScreen {
    presenter: Entity<RiseCameraPresenter>,
    screen_width: f32,

    nav_store: Option<Entity<NavigationStore>>,
    bridge: RiseCameraPreviewBridge,
    focus_point: Option<Point<Pixels>>,
    focus_token: u64,
    focus_exit: Animated<f32>,
    focus_task: Option<Task<()>>,
    shows_hint: bool,
    hint_opacity: Animated<f32>,
    hint_token: u64,
    hint_task: Option<Task<()>>,
    shutter_pressed: bool,
    hold_task: Option<Task<()>>,
    limit_task: Option<Task<()>>,
    video_phase: VideoPhase,
    stop_requested: bool,
    torch_for_take: bool,
    pinching: bool,
    magnification: f32,
    close_drag_axis: Rc<Cell<Option<bool>>>,
    mode_drag: Animated<f32>,
    mode_index: Animated<f32>,
    shutter_flash: Animated<f32>,
    flash_task: Option<Task<()>>,
    shutter_ring: Animated<f32>,
    shutter_inner: Animated<f32>,
    gallery_thumbnail: Option<Arc<RenderImage>>,
    _gallery_thumbnail_task: Option<Task<()>>,
    press_origin: Option<Point<Pixels>>,
    preview_bounds: Rc<Cell<Bounds<Pixels>>>,
    mode_strip_width: Rc<Cell<f32>>,
    _observations: Vec<Subscription>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VideoPhase {
    Idle,
    Starting,
    Recording,
}

impl RiseCameraScreen {
    pub fn new(presenter: Entity<RiseCameraPresenter>, screen_width: f32, cx: &mut Context<Self>) -> Self {
        let capture = presenter.read(cx).capture.clone();
        let mode_index = Self::index_of(presenter.read(cx).mode());
        let observations =
            vec![cx.observe(&presenter, |_, _, cx| cx.notify()), cx.observe(&capture, |_, _, cx| cx.notify())];
        Self {
            presenter,
            screen_width,
            nav_store: None,
            bridge: RiseCameraPreviewBridge::new(),
            focus_point: None,
            focus_token: 0,
            focus_exit: Animated::new(0.0),
            focus_task: None,
            shows_hint: false,
            hint_opacity: Animated::new(0.0),
            hint_token: 0,
            hint_task: None,
            shutter_pressed: false,
            hold_task: None,
            limit_task: None,
            video_phase: VideoPhase::Idle,
            stop_requested: false,
            torch_for_take: false,
            pinching: false,
            magnification: 1.0,
            close_drag_axis: Rc::new(Cell::new(None)),
            mode_drag: Animated::new(0.0),
            mode_index: Animated::new(mode_index),
            shutter_flash: Animated::new(0.0),
            flash_task: None,
            shutter_ring: Animated::new(SHUTTER_RING_IDLE),
            shutter_inner: Animated::new(SHUTTER_INNER_IDLE),
            gallery_thumbnail: None,
            _gallery_thumbnail_task: Some(Self::load_gallery_thumbnail(cx)),
            press_origin: None,
            preview_bounds: Rc::new(Cell::new(Bounds::default())),
            mode_strip_width: Rc::new(Cell::new(0.0)),
            _observations: observations,
        }
    }

    pub fn set_screen_width(&mut self, screen_width: f32, cx: &mut Context<Self>) {
        if self.screen_width != screen_width {
            self.screen_width = screen_width;
            cx.notify();
        }
    }

    fn capture(&self, cx: &App) -> Entity<RiseCameraCapture> {
        self.presenter.read(cx).capture.clone()
    }

    fn index_of(mode: RiseCameraMode) -> f32 {
        RiseCameraMode::ALL_CASES.iter().position(|m| *m == mode).unwrap_or(0) as f32
    }

    fn close_drag(&self, this: WeakEntity<Self>) -> DragGesture {
        let axis = self.close_drag_axis.clone();
        let ended_axis = self.close_drag_axis.clone();
        let ended_this = this.clone();
        DragGesture::new(DRAG_MINIMUM_DISTANCE)
            .on_changed(move |value: &DragGestureValue, _, cx| {
                let axis = axis.clone();
                let value = *value;
                let _ = this.update(cx, |screen, cx| {
                    if screen.pinching || screen.shutter_pressed || screen.video_phase != VideoPhase::Idle {
                        return;
                    }
                    let (dx, dy) = (f32::from(value.translation.width), f32::from(value.translation.height));
                    if axis.get().is_none() {
                        axis.set(Some(dx.abs() > dy.abs() && dx < 0.0));
                    }
                    if axis.get() != Some(true) {
                        return;
                    }
                    let fraction = -dx / screen.screen_width.max(1.0);
                    screen.presenter.update(cx, |presenter, cx| presenter.close_drag_changed(fraction, cx));
                });
            })
            .on_ended(move |value: &DragGestureValue, _, cx| {
                if ended_axis.take() != Some(true) {
                    return;
                }
                let value = *value;
                let _ = ended_this.update(cx, |screen, cx| {
                    let dx = f32::from(value.translation.width);
                    let velocity = (f32::from(value.predicted_end_translation.width) - dx) * RELEASE_VELOCITY_FACTOR;
                    let fraction = -dx / screen.screen_width.max(1.0);
                    screen
                        .presenter
                        .update(cx, |presenter, cx| presenter.close_drag_ended(fraction, velocity, cx));
                });
            })
    }

    fn pointer_down(&mut self, event: &MouseDownEvent) {
        self.press_origin = Some(event.position);
    }

    fn pointer_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some(origin) = self.press_origin.take() else {
            return;
        };
        let moved = event.position - origin;
        if f32::from(moved.x).hypot(f32::from(moved.y)) >= DRAG_MINIMUM_DISTANCE {
            return;
        }
        let capture = self.capture(cx);
        if event.click_count >= 2 {
            capture.update(cx, |capture, cx| capture.switch_camera(cx));
        } else {
            let local = event.position - self.preview_bounds.get().origin;
            self.focus(local, cx);
        }
    }

    fn pinch(&mut self, event: &PinchEvent, cx: &mut Context<Self>) {
        let capture = self.capture(cx);
        match event.phase {
            TouchPhase::Started => {
                self.pinching = true;
                self.magnification = 1.0;
                capture.update(cx, |capture, _| capture.begin_zoom());
            }
            TouchPhase::Moved => {
                if !self.pinching {
                    self.pinching = true;
                    self.magnification = 1.0;
                    capture.update(cx, |capture, _| capture.begin_zoom());
                }
                self.magnification *= 1.0 + event.delta;
                let magnification = self.magnification;
                capture.update(cx, |capture, cx| capture.zoom(magnification, cx));
            }
            TouchPhase::Ended | TouchPhase::Cancelled => self.pinching = false,
        }
    }

    fn focus(&mut self, point: Point<Pixels>, cx: &mut Context<Self>) {
        if !RiseCameraCapture::uses_test_source() {
            let Some(device_point) = self.bridge.device_point(point, cx) else {
                return;
            };
            let capture = self.capture(cx);
            capture.update(cx, |capture, cx| capture.focus(device_point, cx));
        }
        self.focus_token = self.focus_token.wrapping_add(1);
        let token = self.focus_token;
        self.focus_point = Some(point);
        self.focus_exit.jump(0.0);
        cx.notify();
        self.focus_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FOCUS_HOLD).await;
            let _ = this.update(cx, |screen, cx| {
                if screen.focus_token != token {
                    return;
                }
                screen.focus_exit.set_in(1.0, ease_out(0.2), RiseClock::monotonic(), cx);
                cx.notify();
            });
        }));
    }

    #[cfg(debug_assertions)]
    fn live_preview(&self, capture: Entity<RiseCameraCapture>, front: bool) -> gpui::AnyElement {
        if RiseCameraCapture::uses_test_source() {
            return RiseCameraTestPreview { front }.into_any_element();
        }
        RiseCameraPreview::new("rise-camera-preview", capture, self.bridge.clone()).into_any_element()
    }

    #[cfg(not(debug_assertions))]
    fn live_preview(&self, capture: Entity<RiseCameraCapture>, _front: bool) -> gpui::AnyElement {
        RiseCameraPreview::new("rise-camera-preview", capture, self.bridge.clone()).into_any_element()
    }

    fn load_gallery_thumbnail(cx: &mut Context<Self>) -> Task<()> {
        let newest = RiseAccountRuntime::current().spawn_blocking(|| {
            let level = photo_library::access_level().ok()?;
            if !level.can_browse() {
                return None;
            }
            photo_library::page(LibraryMediaKind::All, 0, 1).ok()?.assets.into_iter().next()
        });
        cx.spawn(async move |this, cx| {
            let Ok(Some(asset)) = newest.await else {
                return;
            };
            let (sender, receiver) = oneshot::channel::<Option<Arc<RenderImage>>>();
            let slot = Mutex::new(Some(sender));
            let request = PhotosImageRequest::request_image(
                &asset.id,
                gpui::size(GALLERY_THUMBNAIL_SIDE, GALLERY_THUMBNAIL_SIDE),
                ImageContentMode::AspectFill,
                ImageDeliveryMode::Opportunistic,
                true,
                move |delivery: PhotoImageDelivery| {
                    if delivery.is_degraded && delivery.image.is_some() {
                        return;
                    }
                    let sender = slot.lock().ok().and_then(|mut slot| slot.take());
                    if let Some(sender) = sender {
                        let _ = sender.send(delivery.image);
                    }
                },
            );
            if request.is_none() {
                return;
            }
            let Ok(Some(image)) = receiver.await else {
                return;
            };
            let _ = this.update(cx, |screen, cx| {
                screen.gallery_thumbnail = Some(image);
                cx.notify();
            });
        })
    }

    fn focus_indicator(&self, window: &Window) -> Option<Div> {
        let focus_point = self.focus_point?;
        let exit = self.focus_exit.frame_value(RiseClock::monotonic(), window);
        if exit >= 1.0 {
            return None;
        }
        let side = FOCUS_SIDE * (1.0 + (FOCUS_EXIT_SCALE - 1.0) * exit);
        Some(
            div()
                .absolute()
                .left(focus_point.x - px(side / 2.0))
                .top(focus_point.y - px(side / 2.0))
                .size(px(side))
                .opacity(1.0 - exit)
                .child(centred_stroke(yellow(), 1.5, Shape::continuous(6.0))),
        )
    }

    fn top_bar(&self, cx: &App) -> Div {
        let presenter = self.presenter.clone();
        let capture = self.capture(cx);
        let capture_state = capture.read(cx);
        let middle = if self.video_phase == VideoPhase::Recording
            && let Some(started) = capture_state.recording_started_at()
        {
            Some(Self::recording_clock(started).into_any_element())
        } else if capture_state.access() == RiseCameraCaptureAccess::Granted {
            Some(self.flash_button(cx).into_any_element())
        } else {
            None
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(14.0))
            .pt(px(10.0))
            .child(Self::round_button("rise-camera-close", "xmark", tr("camera_close"), false, move |_, cx| {
                presenter.update(cx, |presenter, cx| presenter.close(cx));
            }))
            .child(div().flex_1())
            .children(middle)
            .child(div().flex_1())
            .child(div().size(px(ROUND_BUTTON_SIDE)))
    }

    fn flash_button(&self, cx: &App) -> Div {
        let capture = self.capture(cx);
        let state = capture.read(cx);
        if self.presenter.read(cx).takes_photos() {
            let mode = state.flash_mode();
            div().child(Self::round_button(
                "rise-camera-flash",
                Self::flash_symbol(mode),
                Self::flash_label(mode),
                false,
                move |_, cx| {
                    capture.update(cx, |capture, cx| {
                        let next = capture.flash_mode().next();
                        capture.set_flash_mode(next, cx);
                    });
                    let _ = selection_changed();
                },
            ))
        } else {
            let torch_on = state.torch_on();
            div().child(Self::round_button(
                "rise-camera-torch",
                if torch_on { "flashlight.on.fill" } else { "flashlight.off.fill" },
                tr("camera_torch"),
                torch_on,
                move |_, cx| capture.update(cx, |capture, cx| capture.toggle_torch(cx)),
            ))
        }
    }

    fn flash_symbol(mode: RiseCameraCaptureFlashMode) -> &'static str {
        match mode {
            RiseCameraCaptureFlashMode::Off => "bolt.slash.fill",
            RiseCameraCaptureFlashMode::On => "bolt.fill",
            RiseCameraCaptureFlashMode::Auto => "bolt.badge.automatic.fill",
        }
    }

    fn flash_label(mode: RiseCameraCaptureFlashMode) -> String {
        match mode {
            RiseCameraCaptureFlashMode::Off => tr("camera_flash_off"),
            RiseCameraCaptureFlashMode::On => tr("camera_flash_on"),
            RiseCameraCaptureFlashMode::Auto => tr("camera_flash_auto"),
        }
    }

    fn recording_clock(start: Date) -> Div {
        let seconds = Date::now().time_interval_since(&start).max(0.0) as i64;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(12.0))
            .h(px(30.0))
            .bg(black().opacity(0.45))
            .capsule()
            .child(div().size(px(8.0)).bg(red()).circle())
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(white())
                    .child(SharedString::from(format!("{}:{:02}", seconds / 60, seconds % 60))),
            )
    }

    fn round_button(
        id: &'static str,
        system_name: &'static str,
        label: String,
        is_on: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> gpui::Stateful<Div> {
        let label = SharedString::from(label);
        let tint: Hsla = if is_on { yellow() } else { white() };
        div().id(SharedString::from(format!("{id}-wrap"))).aria_label(label.clone()).tooltip_label(label).child(
            plain(id).on_press(action).child(
                div()
                    .size(px(ROUND_BUTTON_SIDE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(black().opacity(0.28))
                    .circle()
                    .children(IconUi::sized(system_name, px(19.0), tint, system_name.ends_with(".fill"))),
            ),
        )
    }

    fn shutter_row(&self, this: WeakEntity<Self>, window: &mut Window, cx: &App) -> Div {
        let idle = self.video_phase == VideoPhase::Idle;
        let capture = self.capture(cx);
        div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(28.0))
            .child(self.gallery_button(idle))
            .child(div().flex_1())
            .child(self.shutter(this, window, cx))
            .child(div().flex_1())
            .child(
                div().opacity(if idle { 1.0 } else { 0.0 }).child(Self::round_button(
                    "rise-camera-switch",
                    "arrow.triangle.2.circlepath",
                    tr("camera_switch"),
                    false,
                    move |_, cx| {
                        if idle {
                            capture.update(cx, |capture, cx| capture.switch_camera(cx));
                        }
                    },
                )),
            )
    }

    fn gallery_button(&self, idle: bool) -> gpui::Stateful<Div> {
        let presenter = self.presenter.clone();
        let nav_store = self.nav_store.clone();
        let label = SharedString::from(tr("camera_gallery"));
        let face = match self.gallery_thumbnail.clone() {
            Some(thumbnail) => div().size_full().child(img(thumbnail).size_full().object_fit(gpui::ObjectFit::Cover)),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(black().opacity(0.28))
                .children(IconUi::sized("photo.on.rectangle", px(18.0), white(), false)),
        };
        div()
            .id("rise-camera-gallery-wrap")
            .opacity(if idle { 1.0 } else { 0.0 })
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(
                plain("rise-camera-gallery")
                    .disabled(!idle)
                    .on_press(move |_, cx| {
                        let nav_store = nav_store.clone();
                        presenter.update(cx, |presenter, cx| {
                            presenter.open_gallery(
                                move |intent: CreatePostIntent, cx: &mut App| {
                                    Self::push(
                                        nav_store.as_ref(),
                                        RootRoute::CreatePostMedia { intent, track_seed: None, journey_id: None },
                                        cx,
                                    );
                                },
                                cx,
                            );
                        });
                    })
                    .child(
                        div()
                            .relative()
                            .size(px(ROUND_BUTTON_SIDE))
                            .clip_shape(Shape::continuous(10.0))
                            .child(face)
                            .child(centred_stroke(white(), 2.0, Shape::continuous(10.0))),
                    ),
            )
    }

    fn shutter(&self, this: WeakEntity<Self>, window: &mut Window, cx: &App) -> gpui::Stateful<Div> {
        let recording = self.video_phase == VideoPhase::Recording;
        let now = RiseClock::monotonic();
        let ring = self.shutter_ring.frame_value(now, window);
        let inner = self.shutter_inner.frame_value(now, window);
        let presenter = self.presenter.read(cx);
        let limit = presenter.max_recording_seconds(cx).max(1.0);
        let inner_color = if presenter.takes_photos() && !recording { white() } else { red() };
        let share = if recording
            && let Some(started) = self.capture(cx).read(cx).recording_started_at()
        {
            window.request_animation_frame();
            Some((Date::now().time_interval_since(&started) / limit).clamp(0.0, 1.0) as f32)
        } else {
            None
        };
        let label = SharedString::from(tr("camera_shutter"));
        let down = this.clone();
        let up = this.clone();
        let up_out = this.clone();
        let key = this;
        div()
            .id("rise-camera-shutter")
            .relative()
            .size(px(SHUTTER_SIDE))
            .flex()
            .items_center()
            .justify_center()
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(
                div()
                    .absolute()
                    .size(px(ring))
                    .child(centred_stroke(white().opacity(if recording { 0.35 } else { 1.0 }), SHUTTER_LINE_WIDTH, Shape::Circle)),
            )
            .children(share.map(|share| {
                canvas(|_, _, _| (), move |bounds, (), window, _| paint_recording_ring(bounds, share, window))
                    .absolute()
                    .size(px(SHUTTER_RING_RECORDING))
            }))
            .child(div().size(px(inner)).bg(inner_color).circle())
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                let _ = down.update(cx, |screen, cx| screen.press_began(cx));
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                let _ = up.update(cx, |screen, cx| screen.press_ended(cx));
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                let _ = up_out.update(cx, |screen, cx| screen.press_ended(cx));
            })
            .on_click(move |event, _, cx| {
                if event.is_keyboard() {
                    let _ = key.update(cx, |screen, cx| screen.tap_shutter(cx));
                }
            })
    }

    fn hint_capsule(&self, window: &Window) -> Option<Div> {
        let opacity = self.hint_opacity.frame_value(RiseClock::monotonic(), window);
        if !self.shows_hint && opacity <= 0.0 {
            return None;
        }
        Some(
            div()
                .mb(px(14.0))
                .px(px(14.0))
                .py(px(8.0))
                .opacity(opacity)
                .bg(black().opacity(0.6))
                .capsule()
                .text_size(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(white())
                .text_center()
                .child(SharedString::from(tr("camera_riffs_hold_hint"))),
        )
    }

    /// post modes of the shot continue in the existing post composer with it attached
    fn open_post(nav_store: Option<&Entity<NavigationStore>>, intent: CreatePostIntent, result: MediaPickerResult, cx: &mut App) {
        CreatePostMediaHandoff::shared(cx).update(cx, |handoff, _| handoff.set(intent, result));
        Self::push(nav_store, RootRoute::CreatePost { intent, journey_id: None }, cx);
    }

    fn push(nav_store: Option<&Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn did_take(&mut self, shot: RiseCameraShot, cx: &mut Context<Self>) {
        let nav_store = self.nav_store.clone();
        self.presenter.update(cx, |presenter, cx| {
            presenter.did_take(
                shot,
                move |intent: CreatePostIntent, result: MediaPickerResult, cx: &mut App| {
                    Self::open_post(nav_store.as_ref(), intent, result, cx);
                },
                cx,
            );
        });
    }

    fn press_began(&mut self, cx: &mut Context<Self>) {
        let granted = self.capture(cx).read(cx).access() == RiseCameraCaptureAccess::Granted;
        if self.shutter_pressed || !granted {
            return;
        }
        self.set_shutter_pressed(true, cx);
        self.stop_requested = false;
        self.hold_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HOLD_DELAY).await;
            let _ = this.update(cx, |screen, cx| {
                let records = screen.presenter.read(cx).records_video();
                if !screen.shutter_pressed || screen.video_phase != VideoPhase::Idle || !records {
                    return;
                }
                screen.begin_video(cx);
            });
        }));
    }

    fn press_ended(&mut self, cx: &mut Context<Self>) {
        if !self.shutter_pressed {
            return;
        }
        self.set_shutter_pressed(false, cx);
        self.hold_task = None;
        match self.video_phase {
            VideoPhase::Idle => self.tap_shutter(cx),
            VideoPhase::Starting => self.stop_requested = true,
            VideoPhase::Recording => self.end_video(cx),
        }
    }

    fn set_shutter_pressed(&mut self, pressed: bool, cx: &mut Context<Self>) {
        self.shutter_pressed = pressed;
        let inner = self.shutter_inner_size();
        self.shutter_inner.set_in(inner, spring(0.2, 0.8), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn set_video_phase(&mut self, phase: VideoPhase, cx: &mut Context<Self>) {
        self.video_phase = phase;
        let recording = phase == VideoPhase::Recording;
        let now = RiseClock::monotonic();
        let ring = if recording { SHUTTER_RING_RECORDING } else { SHUTTER_RING_IDLE };
        self.shutter_ring.set_in(ring, spring(0.28, 0.8), now, cx);
        let inner = self.shutter_inner_size();
        self.shutter_inner.set_in(inner, spring(0.28, 0.8), now, cx);
        cx.notify();
    }

    fn shutter_inner_size(&self) -> f32 {
        if self.video_phase == VideoPhase::Recording {
            return SHUTTER_INNER_RECORDING;
        }
        if self.shutter_pressed { SHUTTER_INNER_PRESSED } else { SHUTTER_INNER_IDLE }
    }

    fn tap_shutter(&mut self, cx: &mut Context<Self>) {
        if !self.presenter.read(cx).takes_photos() {
            self.show_hint(cx);
            return;
        }
        let capture = self.capture(cx);
        if capture.read(cx).is_capturing_photo() {
            return;
        }
        let _ = impact(ImpactStyle::Light);
        self.shutter_flash.set_in(1.0, linear(0.05), RiseClock::monotonic(), cx);
        self.flash_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLASH_FADE_DELAY).await;
            let _ = this.update(cx, |screen, cx| {
                screen.shutter_flash.set_in(0.0, ease_out(0.25), RiseClock::monotonic(), cx);
                cx.notify();
            });
        }));
        cx.notify();
        let shot = capture.update(cx, |capture, cx| capture.capture_photo(cx));
        cx.spawn(async move |this, cx| {
            if let Some(shot) = shot.await {
                let _ = this.update(cx, |screen, cx| screen.did_take(shot, cx));
            }
        })
        .detach();
    }

    fn begin_video(&mut self, cx: &mut Context<Self>) {
        self.set_video_phase(VideoPhase::Starting, cx);
        self.hide_hint(ease_out(0.15), cx);
        let capture = self.capture(cx);
        let (flash_on, torch_on) = {
            let state = capture.read(cx);
            (state.flash_mode() == RiseCameraCaptureFlashMode::On, state.torch_on())
        };
        self.torch_for_take = self.presenter.read(cx).takes_photos() && flash_on && !torch_on;
        if self.torch_for_take {
            capture.update(cx, |capture, cx| capture.set_torch(true, cx));
        }
        let started = capture.update(cx, |capture, cx| capture.start_recording(cx));
        cx.spawn(async move |this, cx| {
            let started = started.await;
            let _ = this.update(cx, |screen, cx| screen.recording_started(started, cx));
        })
        .detach();
    }

    fn recording_started(&mut self, started: bool, cx: &mut Context<Self>) {
        let capture = self.capture(cx);
        if !started {
            self.set_video_phase(VideoPhase::Idle, cx);
            if self.torch_for_take {
                capture.update(cx, |capture, cx| capture.set_torch(false, cx));
            }
            return;
        }
        self.set_video_phase(VideoPhase::Recording, cx);
        let limit = Duration::from_secs_f64(self.presenter.read(cx).max_recording_seconds(cx).max(1.0));
        self.limit_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(limit).await;
            let _ = this.update(cx, |screen, cx| {
                if screen.video_phase == VideoPhase::Recording {
                    screen.end_video(cx);
                }
            });
        }));
        if self.stop_requested || !self.shutter_pressed {
            self.end_video(cx);
        }
    }

    fn end_video(&mut self, cx: &mut Context<Self>) {
        if self.video_phase != VideoPhase::Recording {
            return;
        }
        self.set_video_phase(VideoPhase::Idle, cx);
        self.stop_requested = false;
        self.limit_task = None;
        let capture = self.capture(cx);
        if self.torch_for_take {
            self.torch_for_take = false;
            capture.update(cx, |capture, cx| capture.set_torch(false, cx));
        }
        let shot = capture.update(cx, |capture, cx| capture.stop_recording(cx));
        cx.spawn(async move |this, cx| {
            if let Some(shot) = shot.await {
                let _ = this.update(cx, |screen, cx| screen.did_take(shot, cx));
            }
        })
        .detach();
    }

    fn reset_shutter(&mut self) {
        self.hold_task = None;
        self.limit_task = None;
        self.shutter_pressed = false;
        self.stop_requested = false;
        self.video_phase = VideoPhase::Idle;
        self.torch_for_take = false;
        self.shutter_ring.jump(SHUTTER_RING_IDLE);
        self.shutter_inner.jump(SHUTTER_INNER_IDLE);
    }

    fn show_hint(&mut self, cx: &mut Context<Self>) {
        self.hint_token = self.hint_token.wrapping_add(1);
        let token = self.hint_token;
        let _ = notification(NotificationKind::Warning);
        self.shows_hint = true;
        self.hint_opacity.set_in(1.0, ease_out(0.15), RiseClock::monotonic(), cx);
        cx.notify();
        self.hint_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HINT_HOLD).await;
            let _ = this.update(cx, |screen, cx| {
                if screen.hint_token == token {
                    screen.hide_hint(ease_in(0.2), cx);
                }
            });
        }));
    }

    fn hide_hint(&mut self, spec: rise_ui::swiftui::motion::MotionSpec, cx: &mut Context<Self>) {
        self.shows_hint = false;
        self.hint_opacity.set_in(0.0, spec, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn mode_strip(&self, this: WeakEntity<Self>, window: &Window, cx: &App) -> DragGestureElement {
        let modes = RiseCameraMode::ALL_CASES;
        let current = self.presenter.read(cx).mode();
        let now = RiseClock::monotonic();
        let index = self.mode_index.frame_value(now, window);
        let drag = self.mode_drag.frame_value(now, window);
        let strip_width = self.mode_strip_width.get();
        let offset = strip_width / 2.0 - (index + 0.5) * MODE_SLOT_WIDTH + drag;
        let idle = self.video_phase == VideoPhase::Idle;
        let measured = self.mode_strip_width.clone();
        let changed = this.clone();
        let ended = this.clone();
        let slots = modes.iter().copied().map(|mode| {
            let selected = mode == current;
            let select_this = this.clone();
            div().w(px(MODE_SLOT_WIDTH)).flex().justify_center().child(
                plain(SharedString::from(format!("rise-camera-mode-{}", mode.title_key())))
                    .disabled(!idle)
                    .on_press(move |_, cx| {
                        let _ = select_this.update(cx, |screen, cx| screen.select(mode, cx));
                    })
                    .child(
                        div()
                            .w(px(MODE_SLOT_WIDTH - 8.0))
                            .h(px(44.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white().opacity(if selected { 1.0 } else { 0.5 }))
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(SharedString::from(tr(mode.title_key()).to_uppercase())),
                    ),
            )
        });
        div()
            .id("rise-camera-mode-strip")
            .relative()
            .w_full()
            .h(px(MODE_STRIP_HEIGHT))
            .overflow_hidden()
            .opacity(if idle { 1.0 } else { 0.4 })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(offset))
                    .flex()
                    .flex_row()
                    .items_center()
                    .children(slots),
            )
            .child(measure("rise-camera-mode-strip-size", move |bounds, window, _| {
                let width = f32::from(bounds.size.width);
                if measured.get() != width {
                    measured.set(width);
                    window.refresh();
                }
            }))
            .gesture(
                "rise-camera-mode-drag",
                DragGesture::new(MODE_DRAG_MINIMUM_DISTANCE)
                    .on_changed(move |value: &DragGestureValue, _, cx| {
                        let width = f32::from(value.translation.width);
                        let _ = changed.update(cx, |screen, cx| {
                            if screen.video_phase != VideoPhase::Idle {
                                return;
                            }
                            screen.mode_drag.jump(width);
                            cx.notify();
                        });
                    })
                    .on_ended(move |value: &DragGestureValue, _, cx| {
                        let predicted = f32::from(value.predicted_end_translation.width);
                        let _ = ended.update(cx, |screen, cx| screen.mode_drag_ended(predicted, cx));
                    }),
            )
    }

    fn mode_drag_ended(&mut self, predicted_end_translation: f32, cx: &mut Context<Self>) {
        let modes = RiseCameraMode::ALL_CASES;
        let index = Self::index_of(self.presenter.read(cx).mode()) as i64;
        let shift = (-predicted_end_translation / MODE_SLOT_WIDTH).round() as i64;
        let target = index + shift.clamp(-1, 1);
        if self.video_phase == VideoPhase::Idle
            && let Some(mode) = usize::try_from(target).ok().and_then(|target| modes.get(target).copied())
        {
            self.select(mode, cx);
        } else {
            self.mode_drag.set_in(0.0, spring(0.3, 0.85), RiseClock::monotonic(), cx);
            cx.notify();
        }
    }

    fn select(&mut self, mode: RiseCameraMode, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        if self.presenter.read(cx).mode() != mode {
            self.presenter.update(cx, |presenter, cx| presenter.set_mode(mode, cx));
            let _ = selection_changed();
        }
        self.mode_index.set_in(Self::index_of(mode), spring(0.3, 0.85), now, cx);
        self.mode_drag.set_in(0.0, spring(0.3, 0.85), now, cx);
        let capture = self.capture(cx);
        let (flash_mode, torch_on) = {
            let state = capture.read(cx);
            (state.flash_mode(), state.torch_on())
        };
        if !mode.takes_photos() && flash_mode == RiseCameraCaptureFlashMode::On {
            capture.update(cx, |capture, cx| capture.set_flash_mode(RiseCameraCaptureFlashMode::Off, cx));
        }
        if mode.takes_photos() && torch_on {
            capture.update(cx, |capture, cx| capture.set_torch(false, cx));
        }
        cx.notify();
    }

    fn access_denied() -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.0))
            .px(px(32.0))
            .children(IconUi::sized("camera.fill", px(38.0), white().opacity(0.85), true))
            .child(
                div()
                    .text_size(px(20.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(white())
                    .text_center()
                    .child(SharedString::from(tr("camera_access_title"))),
            )
            .child(
                div()
                    .text_size(px(15.0))
                    .text_color(white().opacity(0.7))
                    .text_center()
                    .child(SharedString::from(tr("camera_access_message"))),
            )
            .child(
                div().pt(px(6.0)).child(
                    plain("rise-camera-open-settings")
                        .on_press(|_, cx| {
                            let _ = open_settings(cx, CaptureKind::Camera);
                        })
                        .child(
                            div()
                                .px(px(22.0))
                                .h(px(46.0))
                                .flex()
                                .items_center()
                                .bg(white())
                                .capsule()
                                .text_size(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(black())
                                .child(SharedString::from(tr("camera_open_settings"))),
                        ),
                ),
            )
    }
}

impl Drop for RiseCameraScreen {
    fn drop(&mut self) {
        self.reset_shutter();
    }
}

impl Render for RiseCameraScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.nav_store = env::<NavigationStoreKey>(cx);
        let this = cx.weak_entity();
        let (is_windowed, progress, from_tile) = {
            let presenter = self.presenter.read(cx);
            (presenter.is_windowed(), presenter.presented_progress(window), presenter.origin_rect().is_some())
        };
        let chrome_alpha = RiseCameraChromeFade { progress, from_tile }.alpha();
        let capture = self.capture(cx);
        let (access, front) = {
            let state = capture.read(cx);
            (state.access(), state.uses_front_camera())
        };
        let flash = self.shutter_flash.frame_value(RiseClock::monotonic(), window);
        let preview_bounds = self.preview_bounds.clone();

        let mut preview_area = div()
            .id("rise-camera-preview-area")
            .relative()
            .flex_1()
            .w_full()
            .bg(black())
            .clip_shape(Shape::continuous(PREVIEW_CORNER_RADIUS))
            .child(measure("rise-camera-preview-bounds", move |bounds, window, _| {
                if preview_bounds.get() != bounds {
                    preview_bounds.set(bounds);
                    window.refresh();
                }
            }));
        match access {
            RiseCameraCaptureAccess::Granted => {
                preview_area = preview_area
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .child(self.live_preview(capture.clone(), front)),
                    )
                    .child(
                        div()
                            .id("rise-camera-gesture-layer")
                            .absolute()
                            .inset_0()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|screen, event: &MouseDownEvent, _, _| screen.pointer_down(event)),
                            )
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|screen, event: &MouseUpEvent, _, cx| screen.pointer_up(event, cx)),
                            )
                            .on_pinch(cx.listener(|screen, event: &PinchEvent, _, cx| screen.pinch(event, cx))),
                    );
            }
            RiseCameraCaptureAccess::Denied => {
                preview_area = preview_area
                    .child(div().absolute().inset_0().flex().items_center().justify_center().child(Self::access_denied()));
            }
            RiseCameraCaptureAccess::Unknown => {}
        }
        if flash > 0.0 {
            preview_area = preview_area.child(div().absolute().inset_0().bg(white().opacity(flash)));
        }
        preview_area = preview_area.children(self.focus_indicator(window));
        let controls = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .opacity(chrome_alpha)
            .child(self.top_bar(cx))
            .child(div().flex_1())
            .children(self.hint_capsule(window).map(|hint| div().flex().justify_center().child(hint)))
            .children((access == RiseCameraCaptureAccess::Granted).then(|| {
                div().pb(px(22.0)).child(self.shutter_row(this.clone(), window, cx))
            }));
        let preview_area = preview_area.child(controls).simultaneous_gesture(
            "rise-camera-close-drag",
            self.close_drag(this.clone()),
            GestureMask::All,
        );

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(black())
            .child(preview_area)
            .children((!is_windowed).then(|| div().opacity(chrome_alpha).child(self.mode_strip(this, window, cx))))
    }
}

struct RiseCameraChromeFade {
    progress: f32,
    from_tile: bool,
}

impl RiseCameraChromeFade {
    fn alpha(&self) -> f32 {
        if self.from_tile {
            ((self.progress - CHROME_FADE_START) / (1.0 - CHROME_FADE_START)).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

fn paint_recording_ring(bounds: Bounds<Pixels>, share: f32, window: &mut Window) {
    if share <= 0.0 {
        return;
    }
    let center = bounds.center();
    let radius = f32::from(bounds.size.width.min(bounds.size.height)) / 2.0;
    let at = |fraction: f32| {
        let angle = fraction * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
    };
    let steps = ((RING_SEGMENTS as f32) * share).ceil().max(1.0) as usize;
    let mut builder = PathBuilder::stroke(px(SHUTTER_LINE_WIDTH));
    for step in 0..=steps {
        let position = at(share * step as f32 / steps as f32);
        if step == 0 {
            builder.move_to(position);
        } else {
            builder.line_to(position);
        }
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, red());
    }
    for end in [at(0.0), at(share)] {
        let mut cap = PathBuilder::fill();
        let cap_radius = SHUTTER_LINE_WIDTH / 2.0;
        for step in 0..12 {
            let angle = step as f32 / 12.0 * std::f32::consts::TAU;
            let position = point(end.x + px(cap_radius * angle.cos()), end.y + px(cap_radius * angle.sin()));
            if step == 0 {
                cap.move_to(position);
            } else {
                cap.line_to(position);
            }
        }
        cap.close();
        if let Ok(path) = cap.build() {
            window.paint_path(path, red());
        }
    }
}
