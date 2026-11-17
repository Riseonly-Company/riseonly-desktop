use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Global, IntoElement, Render, Size, Task, WeakEntity, Window,
    black, div, point, prelude::*, px, size, white,
};
use rise_i18n::tr;
use rise_media::camera_capture::call_camera::host_nv12_of;
use rise_media::camera_capture::{CameraFrame, FrameSink, VideoNoteCamera};
use rise_media::texture::native_surface::{
    PreparedHostFrame, mirror_host_nv12_frame, prepare_host_frame, present_host_frame,
};
use rise_platform::capture_permission::{self, CaptureAccess, CaptureKind};
use rise_platform::paths::temporary_directory;
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

static NEXT_PICKER_CAMERA_STREAM: AtomicU64 = AtomicU64::new(0x00cc_0000_0000_0000);

/// live camera tile that leads the picker grid
/// back camera runs while its on screen, a tap opens the capture screen
/// why: tile spans two grid slots, the grid layout gives it its frame
pub struct MediaPickerCameraCell {
    pub preview_view: Entity<MediaPickerCameraPreviewView>,
    pub on_tap: Option<Box<dyn Fn(&mut Window, &mut App)>>,
    focus_handle: FocusHandle,
}

impl MediaPickerCameraCell {
    pub const REUSE_IDENTIFIER: &'static str = "MediaPickerCameraCell";
    const ICON_SIDE: f32 = 28.0;
    const ICON_POINT_SIZE: f32 = 22.0;

    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            preview_view: cx.new(MediaPickerCameraPreviewView::new),
            on_tap: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn handle_tap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_tap) = self.on_tap.as_ref() {
            on_tap(window, cx);
        }
    }
}

impl Render for MediaPickerCameraCell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = tr("rtc_camera");
        div()
            .id("media-picker-camera-cell")
            .relative()
            .size_full()
            .bg(black())
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .tab_stop(true)
            .aria_label(label.clone())
            .tooltip_label(label)
            .on_click(cx.listener(|this, _, window, cx| this.handle_tap(window, cx)))
            .child(div().absolute().inset_0().child(self.preview_view.clone()))
            .child(
                div().absolute().inset_0().flex().items_center().justify_center().child(
                    div()
                        .size(px(Self::ICON_SIDE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized("camera.fill", px(Self::ICON_POINT_SIZE), white(), true)),
                ),
            )
    }
}

#[derive(Default)]
struct MediaPickerCameraPreviewSlot {
    frame: Option<CameraFrame>,
    generation: u64,
}

/// preview surface of the tile
pub struct MediaPickerCameraPreviewView {
    attached_session: Option<Arc<MediaPickerCameraSession>>,
    is_in_window: bool,
    is_displayed: bool,
    stream_id: u64,
    shown_generation: u64,
    picture: Option<PreparedHostFrame>,
    picture_size: Size<f32>,
    box_size: Size<f32>,
    prepare_task: Option<Task<()>>,
}

impl MediaPickerCameraPreviewView {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            attached_session: None,
            is_in_window: false,
            is_displayed: false,
            stream_id: NEXT_PICKER_CAMERA_STREAM.fetch_add(1, Ordering::Relaxed),
            shown_generation: 0,
            picture: None,
            picture_size: Size::default(),
            box_size: Size::default(),
            prepare_task: None,
        }
    }

    pub fn is_displayed(&self) -> bool {
        self.is_displayed
    }

    pub fn set_is_displayed(&mut self, is_displayed: bool, cx: &mut Context<Self>) {
        if self.is_displayed != is_displayed {
            self.is_displayed = is_displayed;
            self.sync_feed(cx);
        }
    }

    pub fn did_move_to_window(&mut self, is_in_window: bool, cx: &mut Context<Self>) {
        if self.is_in_window != is_in_window {
            self.is_in_window = is_in_window;
            self.sync_feed(cx);
        }
    }

    /// feed runs only for a displayed tile inside a window
    /// closing the picker or covering it with a full screen controller stops the camera
    fn sync_feed(&mut self, cx: &mut Context<Self>) {
        let this = cx.entity();
        let runs = self.is_displayed && self.is_in_window;
        cx.defer(move |cx| {
            if runs {
                MediaPickerCameraFeed::attach(this, cx);
            } else {
                MediaPickerCameraFeed::detach(&this, cx);
            }
        });
    }

    pub fn attach(&mut self, session: Option<Arc<MediaPickerCameraSession>>, cx: &mut Context<Self>) {
        let same = match (&self.attached_session, &session) {
            (Some(current), Some(next)) => Arc::ptr_eq(current, next),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.attached_session = session;
        if self.attached_session.is_none() {
            self.picture = None;
            self.prepare_task = None;
        }
        cx.notify();
    }

    fn pull_latest_frame(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.attached_session.as_ref() else {
            return;
        };
        let (generation, frame) = session.latest();
        if generation == self.shown_generation || self.prepare_task.is_some() {
            return;
        }
        let Some(frame) = frame else {
            return;
        };
        self.shown_generation = generation;
        let stream_id = self.stream_id;
        let prepared = cx.background_spawn(async move {
            let host = host_nv12_of(&frame, stream_id, generation).ok()?;
            let (width, height) = (host.width, host.height);
            let host = if frame.mirrored { mirror_host_nv12_frame(&host)? } else { host };
            let prepared = prepare_host_frame(host).ok()?;
            Some((prepared, size(width as f32, height as f32)))
        });
        self.prepare_task = Some(cx.spawn(async move |this, cx| {
            let prepared = prepared.await;
            let _ = this.update(cx, |view, cx| {
                view.prepare_task = None;
                if let Some((picture, picture_size)) = prepared
                    && view.attached_session.is_some()
                {
                    view.picture = Some(picture);
                    view.picture_size = picture_size;
                    cx.notify();
                }
            });
        }));
    }

    fn picture_frame(&self) -> Bounds<f32> {
        if self.picture_size.width <= 0.0 || self.picture_size.height <= 0.0 {
            return Bounds { origin: point(0.0, 0.0), size: self.box_size };
        }
        let scale =
            (self.box_size.width / self.picture_size.width).max(self.box_size.height / self.picture_size.height);
        let drawn = size(self.picture_size.width * scale, self.picture_size.height * scale);
        Bounds {
            origin: point((self.box_size.width - drawn.width) / 2.0, (self.box_size.height - drawn.height) / 2.0),
            size: drawn,
        }
    }
}

impl Render for MediaPickerCameraPreviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.pull_latest_frame(cx);
        if self.attached_session.is_some() {
            window.request_animation_frame();
        }
        let picture = self.picture.as_ref().and_then(|picture| present_host_frame(picture).ok());
        let drawn = self.picture_frame();
        let this = cx.weak_entity();
        div()
            .relative()
            .size_full()
            .bg(black())
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left(px(drawn.origin.x))
                    .top(px(drawn.origin.y))
                    .w(px(drawn.size.width))
                    .h(px(drawn.size.height))
                    .grid()
                    .grid_cols(1)
                    .grid_rows(1)
                    .children(picture),
            )
            .child(measure("media-picker-camera-preview-size", move |bounds, _, cx| {
                let box_size = size(f32::from(bounds.size.width), f32::from(bounds.size.height));
                let _ = this.update(cx, |view, cx| {
                    if view.box_size != box_size {
                        view.box_size = box_size;
                        cx.notify();
                    }
                });
            }))
    }
}

/// one running preview camera, frames land in a slot the attached view polls
pub struct MediaPickerCameraSession {
    camera: Mutex<Option<VideoNoteCamera>>,
    is_stopped: AtomicBool,
    slot: Arc<Mutex<MediaPickerCameraPreviewSlot>>,
}

impl MediaPickerCameraSession {
    fn latest(&self) -> (u64, Option<CameraFrame>) {
        let slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        (slot.generation, slot.frame.clone())
    }

    fn frame_sink(&self) -> FrameSink {
        let slot = Arc::clone(&self.slot);
        Arc::new(move |frame: CameraFrame| {
            let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
            slot.frame = Some(frame);
            slot.generation = slot.generation.wrapping_add(1);
        })
    }

    pub fn is_running(&self) -> bool {
        self.camera.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }

    fn stop(&self) {
        self.is_stopped.store(true, Ordering::Release);
        let camera = self.camera.lock().unwrap_or_else(PoisonError::into_inner).take();
        drop(camera);
    }

    fn keep(&self, camera: VideoNoteCamera) {
        let mut slot = self.camera.lock().unwrap_or_else(PoisonError::into_inner);
        if self.is_stopped.load(Ordering::Acquire) {
            drop(slot);
            drop(camera);
            return;
        }
        *slot = Some(camera);
    }
}

/// main thread singleton of the camera feed as a gpui global
#[derive(Default)]
struct MediaPickerCameraFeedState {
    session: Option<Arc<MediaPickerCameraSession>>,
    preview: Option<WeakEntity<MediaPickerCameraPreviewView>>,
    is_suspended: bool,
}

impl Global for MediaPickerCameraFeedState {}

/// preview only camera of the picker tile
/// runs while a tile is on screen and the camera is already allowed
/// steps aside while the capture screen owns it, picker never asks for camera permission
pub enum MediaPickerCameraFeed {}

impl MediaPickerCameraFeed {
    const CAPTURE_DIRECTORY_NAME: &'static str = "media-picker-camera-feed";
    const CAPTURE_STEM: &'static str = "preview";

    pub fn attach(view: Entity<MediaPickerCameraPreviewView>, cx: &mut App) {
        let state = cx.default_global::<MediaPickerCameraFeedState>();
        if state.preview.as_ref().is_some_and(|preview| preview.entity_id() == view.entity_id()) {
            return;
        }
        let previous = state.preview.replace(view.downgrade());
        if let Some(previous) = previous.and_then(|preview| preview.upgrade()) {
            previous.update(cx, |preview, cx| preview.attach(None, cx));
        }
        Self::start_if_possible(cx);
    }

    pub fn detach(view: &Entity<MediaPickerCameraPreviewView>, cx: &mut App) {
        let state = cx.default_global::<MediaPickerCameraFeedState>();
        if !state.preview.as_ref().is_some_and(|preview| preview.entity_id() == view.entity_id()) {
            return;
        }
        state.preview = None;
        view.update(cx, |preview, cx| preview.attach(None, cx));
        Self::stop_session(cx);
    }

    pub fn suspend(cx: &mut App) {
        let state = cx.default_global::<MediaPickerCameraFeedState>();
        state.is_suspended = true;
        let preview = state.preview.clone();
        if let Some(preview) = preview.and_then(|preview| preview.upgrade()) {
            preview.update(cx, |preview, cx| preview.attach(None, cx));
        }
        Self::stop_session(cx);
    }

    /// capture screen let the camera go, a tile still on screen comes back to life
    pub fn resume(cx: &mut App) {
        cx.default_global::<MediaPickerCameraFeedState>().is_suspended = false;
        Self::start_if_possible(cx);
    }

    fn start_if_possible(cx: &mut App) {
        let state = cx.default_global::<MediaPickerCameraFeedState>();
        if state.is_suspended {
            return;
        }
        let Some(preview) = state.preview.as_ref().and_then(|preview| preview.upgrade()) else {
            return;
        };
        if !matches!(capture_permission::status(CaptureKind::Camera), Ok(CaptureAccess::Granted)) {
            return;
        }
        let existing = cx.default_global::<MediaPickerCameraFeedState>().session.clone();
        let running = match existing {
            Some(running) => running,
            None => Self::make_session(cx),
        };
        cx.default_global::<MediaPickerCameraFeedState>().session = Some(Arc::clone(&running));
        preview.update(cx, |preview, cx| preview.attach(Some(running), cx));
    }

    fn stop_session(cx: &mut App) {
        let Some(running) = cx.default_global::<MediaPickerCameraFeedState>().session.take() else {
            return;
        };
        cx.background_executor().spawn(async move { running.stop() }).detach();
    }

    fn make_session(cx: &App) -> Arc<MediaPickerCameraSession> {
        let session = Arc::new(MediaPickerCameraSession {
            camera: Mutex::new(None),
            is_stopped: AtomicBool::new(false),
            slot: Arc::new(Mutex::new(MediaPickerCameraPreviewSlot::default())),
        });
        let opening = Arc::clone(&session);
        cx.background_executor()
            .spawn(async move {
                let opened = VideoNoteCamera::open(
                    false,
                    temporary_directory().join(Self::CAPTURE_DIRECTORY_NAME),
                    Self::CAPTURE_STEM.to_owned(),
                    opening.frame_sink(),
                );
                if let Ok(camera) = opened {
                    opening.keep(camera);
                }
            })
            .detach();
        session
    }
}
