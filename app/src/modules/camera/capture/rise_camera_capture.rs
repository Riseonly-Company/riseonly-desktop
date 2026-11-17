use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use gpui::{App, Context, Point, Size, Task, size};
use image::{DynamicImage, ImageFormat};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::camera_capture::{CameraFrame, FrameSink, VideoNoteCamera, still_rgba_of};
use rise_media::video_export::{VideoProbe, probe};
use rise_platform::capture_permission::{self, CaptureAccess, CaptureKind};
use rise_platform::haptics::{self, ImpactStyle};
use rise_platform::paths::temporary_directory;

use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
#[cfg(debug_assertions)]
use crate::modules::camera::capture::rise_camera_test_source::RiseCameraTestSource;
use crate::modules::media_picker::library::media_asset::{MediaAsset, MediaAssetMediaType};

/// photo or video the feed camera made, written to a temp file the export reads
#[derive(Clone)]
pub struct RiseCameraShot {
    pub id: String,
    pub url: Url,
    pub image: Option<CleverImage>,
    pub pixel_size: Size<f32>,
    pub duration: TimeInterval,
}

impl RiseCameraShot {
    pub fn new(url: Url, image: Option<CleverImage>, pixel_size: Size<f32>, duration: TimeInterval) -> Self {
        Self {
            id: Uuid::new_v4().uuid_string(),
            url,
            image,
            pixel_size,
            duration,
        }
    }

    pub fn is_video(&self) -> bool {
        self.image.is_none()
    }

    pub fn media_asset(&self) -> MediaAsset {
        MediaAsset::new(
            format!("camera:{}", self.id),
            if self.is_video() { MediaAssetMediaType::Video } else { MediaAssetMediaType::Image },
            self.pixel_size.width.max(1.0).round() as i64,
            self.pixel_size.height.max(1.0).round() as i64,
            self.duration,
            Some(Date::now()),
        )
    }

    pub fn discard(&self) {
        if let Some(path) = self.url.to_file_path() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseCameraCaptureAccess {
    Unknown,
    Granted,
    Denied,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum RiseCameraCaptureFlashMode {
    #[default]
    Off,
    On,
    Auto,
}

impl RiseCameraCaptureFlashMode {
    pub const ALL_CASES: [RiseCameraCaptureFlashMode; 3] = [Self::Off, Self::On, Self::Auto];

    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Auto,
            Self::Auto => Self::Off,
        }
    }
}

#[derive(Default)]
struct RiseCameraPreviewSlot {
    generation: u64,
    frame: Option<CameraFrame>,
}

/// the open camera and its latest preview frame, shared with the background work that opens
/// and records and closes it
struct RiseCameraSession {
    camera: Mutex<Option<VideoNoteCamera>>,
    run_generation: AtomicU64,
    zoom_range: Mutex<(f32, f32)>,
    zoom_target: AtomicU32,
    slot: Arc<Mutex<RiseCameraPreviewSlot>>,
}

impl RiseCameraSession {
    const CAPTURE_DIRECTORY_NAME: &'static str = "feed-camera";

    fn frame_sink(&self) -> FrameSink {
        let slot = Arc::clone(&self.slot);
        Arc::new(move |frame: CameraFrame| {
            let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
            slot.frame = Some(frame);
            slot.generation = slot.generation.wrapping_add(1);
        })
    }

    fn latest(&self) -> (u64, Option<CameraFrame>) {
        let slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        (slot.generation, slot.frame.clone())
    }

    fn is_running(&self) -> bool {
        self.camera.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }

    fn next_run_generation(&self) -> u64 {
        self.run_generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }

    fn is_latest_run(&self, generation: u64) -> bool {
        self.run_generation.load(Ordering::Acquire) == generation
    }

    fn zoom_range(&self) -> (f32, f32) {
        *self.zoom_range.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_zoom_range(&self, range: Option<(f32, f32)>) {
        *self.zoom_range.lock().unwrap_or_else(PoisonError::into_inner) = range.unwrap_or((1.0, 1.0));
    }

    fn with_camera<R>(&self, work: impl FnOnce(&mut VideoNoteCamera) -> R) -> Option<R> {
        let mut camera = self.camera.lock().unwrap_or_else(PoisonError::into_inner);
        camera.as_mut().map(work)
    }

    fn start_running(&self, front: bool, generation: u64) {
        if !self.is_latest_run(generation) || self.is_running() {
            return;
        }
        let opened = VideoNoteCamera::open(
            front,
            temporary_directory().join(Self::CAPTURE_DIRECTORY_NAME),
            Uuid::new_v4().uuid_string(),
            self.frame_sink(),
        );
        let Ok(mut camera) = opened else {
            return;
        };
        camera.set_mirrors_recording(front);
        let zoom_range = camera.zoom_range();
        let mut slot = self.camera.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.is_latest_run(generation) || slot.is_some() {
            drop(slot);
            drop(camera);
            return;
        }
        *slot = Some(camera);
        self.set_zoom_range(zoom_range);
    }

    fn stop_running(&self, generation: u64) {
        let camera = {
            let mut slot = self.camera.lock().unwrap_or_else(PoisonError::into_inner);
            if !self.is_latest_run(generation) {
                return;
            }
            slot.take()
        };
        drop(camera);
        self.set_zoom_range(None);
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        slot.frame = None;
        slot.generation = slot.generation.wrapping_add(1);
    }
}

/// full screen camera of the feed, one capture session with a photo and a movie output
/// runs only while the camera is on screen and warms up when the feed drag starts
pub struct RiseCameraCapture {
    access: RiseCameraCaptureAccess,
    uses_front_camera: bool,
    is_recording: bool,
    recording_started_at: Option<Date>,
    is_capturing_photo: bool,
    torch_on: bool,
    zoom_factor: f32,
    flash_mode: RiseCameraCaptureFlashMode,

    session: Arc<RiseCameraSession>,
    is_configured: bool,
    wants_running: bool,
    zoom_base: f32,
    has_audio_input: bool,
}

impl RiseCameraCapture {
    const MAX_ZOOM_FACTOR: f32 = 10.0;

    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            access: RiseCameraCaptureAccess::Unknown,
            uses_front_camera: false,
            is_recording: false,
            recording_started_at: None,
            is_capturing_photo: false,
            torch_on: false,
            zoom_factor: 1.0,
            flash_mode: RiseCameraCaptureFlashMode::Off,
            session: Arc::new(RiseCameraSession {
                camera: Mutex::new(None),
                run_generation: AtomicU64::new(0),
                zoom_range: Mutex::new((1.0, 1.0)),
                zoom_target: AtomicU32::new(1.0_f32.to_bits()),
                slot: Arc::new(Mutex::new(RiseCameraPreviewSlot::default())),
            }),
            is_configured: false,
            wants_running: false,
            zoom_base: 1.0,
            has_audio_input: false,
        }
    }

    /// a machine without a camera has nothing to preview, a debug build draws a test picture and
    /// writes test files
    /// so gestures and modes can still be tried
    #[cfg(debug_assertions)]
    pub fn uses_test_source() -> bool {
        static USES_TEST_SOURCE: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
            crate::app::dev_drive::DevDrive::stealth_from_launch()
                || rise_media::camera_capture::video_note_unavailable_reason().is_some()
        });
        *USES_TEST_SOURCE
    }

    #[cfg(not(debug_assertions))]
    pub fn uses_test_source() -> bool {
        false
    }

    pub fn access(&self) -> RiseCameraCaptureAccess {
        self.access
    }

    pub fn uses_front_camera(&self) -> bool {
        self.uses_front_camera
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording
    }

    pub fn recording_started_at(&self) -> Option<Date> {
        self.recording_started_at
    }

    pub fn is_capturing_photo(&self) -> bool {
        self.is_capturing_photo
    }

    pub fn torch_on(&self) -> bool {
        self.torch_on
    }

    pub fn zoom_factor(&self) -> f32 {
        self.zoom_factor
    }

    pub fn flash_mode(&self) -> RiseCameraCaptureFlashMode {
        self.flash_mode
    }

    pub fn set_flash_mode(&mut self, flash_mode: RiseCameraCaptureFlashMode, cx: &mut Context<Self>) {
        self.flash_mode = flash_mode;
        cx.notify();
    }

    /// preview picture, the counter moves with every new frame
    /// polled by the preview on each display refresh while the session runs
    pub fn latest_frame(&self) -> (u64, Option<CameraFrame>) {
        self.session.latest()
    }

    pub fn is_session_running(&self) -> bool {
        self.session.is_running()
    }

    /// feed drag started, brings the preview up if access is already settled
    /// never asks for access mid gesture, that waits for did_open
    pub fn warm_up(&mut self, cx: &mut Context<Self>) {
        self.wants_running = true;
        self.refresh_access(cx);
        if self.access != RiseCameraCaptureAccess::Granted {
            return;
        }
        self.start_if_wanted(cx).detach();
    }

    pub fn did_open(&mut self, cx: &mut Context<Self>) {
        self.wants_running = true;
        cx.spawn(async move |this, cx| {
            Self::request_access_if_needed(this.clone(), cx).await;
            let Ok(start) = this.update(cx, |this, cx| this.start_if_wanted(cx)) else {
                return;
            };
            start.await;
        })
        .detach();
    }

    /// camera left the screen, drops the take in progress and stops the session
    pub fn shut_down(&mut self, cx: &mut Context<Self>) {
        self.wants_running = false;
        self.cancel_recording(cx);
        self.stop_session(cx);
        self.zoom_factor = 1.0;
        self.zoom_base = 1.0;
        cx.notify();
    }

    pub fn suspend(&mut self, cx: &mut Context<Self>) {
        self.cancel_recording(cx);
        self.stop_session(cx);
    }

    pub fn resume(&mut self, cx: &mut Context<Self>) {
        if !self.wants_running {
            return;
        }
        self.refresh_access(cx);
        self.start_if_wanted(cx).detach();
    }

    fn refresh_access(&mut self, cx: &mut Context<Self>) {
        let access = if Self::uses_test_source() {
            RiseCameraCaptureAccess::Granted
        } else {
            match capture_permission::status(CaptureKind::Camera) {
                Ok(CaptureAccess::Granted) => RiseCameraCaptureAccess::Granted,
                Ok(CaptureAccess::NotDetermined) => RiseCameraCaptureAccess::Unknown,
                _ => RiseCameraCaptureAccess::Denied,
            }
        };
        if self.access != access {
            self.access = access;
            cx.notify();
        }
    }

    async fn request_access_if_needed(this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp) {
        if matches!(capture_permission::status(CaptureKind::Camera), Ok(CaptureAccess::NotDetermined)) {
            let _ = capture_permission::ensure_access(CaptureKind::Camera).await;
        }
        let Ok(granted) = this.update(cx, |this, cx| {
            this.refresh_access(cx);
            this.access == RiseCameraCaptureAccess::Granted
        }) else {
            return;
        };
        if granted
            && !Self::uses_test_source()
            && matches!(capture_permission::status(CaptureKind::Microphone), Ok(CaptureAccess::NotDetermined))
        {
            let _ = capture_permission::ensure_access(CaptureKind::Microphone).await;
        }
    }

    fn start_if_wanted(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if !self.wants_running || self.access != RiseCameraCaptureAccess::Granted || Self::uses_test_source() {
            return Task::ready(());
        }
        self.configure_if_needed();
        let session = Arc::clone(&self.session);
        let front = self.uses_front_camera;
        let generation = self.session.next_run_generation();
        let opening = self.on_queue(cx, move || session.start_running(front, generation));
        cx.spawn(async move |this, cx| {
            opening.await;
            let _ = this.update(cx, |this, cx| {
                if !this.wants_running {
                    this.stop_session(cx);
                }
                cx.notify();
            });
        })
    }

    fn stop_session(&mut self, cx: &mut Context<Self>) {
        self.set_torch(false, cx);
        if Self::uses_test_source() {
            return;
        }
        let session = Arc::clone(&self.session);
        let generation = self.session.next_run_generation();
        self.on_queue(cx, move || session.stop_running(generation)).detach();
    }

    fn configure_if_needed(&mut self) {
        if self.is_configured {
            return;
        }
        self.is_configured = true;
    }

    fn apply_connections(&self, cx: &App) -> Task<()> {
        let session = Arc::clone(&self.session);
        let front = self.uses_front_camera;
        self.on_queue(cx, move || {
            session.with_camera(|camera| camera.set_mirrors_recording(front));
        })
    }

    fn on_queue(&self, cx: &App, work: impl FnOnce() + Send + 'static) -> Task<()> {
        cx.background_executor().spawn(async move { work() })
    }

    pub fn switch_camera(&mut self, cx: &mut Context<Self>) {
        if self.is_recording || self.is_capturing_photo {
            return;
        }
        self.set_torch(false, cx);
        self.uses_front_camera = !self.uses_front_camera;
        self.zoom_factor = 1.0;
        self.zoom_base = 1.0;
        let _ = haptics::impact(ImpactStyle::Light);
        cx.notify();
        if !self.is_configured || Self::uses_test_source() || !self.session.is_running() {
            return;
        }
        let session = Arc::clone(&self.session);
        let flipping = self.on_queue(cx, move || {
            let zoom_range = session
                .with_camera(|camera| {
                    let _ = camera.flip(RiseClock::monotonic());
                    camera.zoom_range()
                })
                .flatten();
            session.set_zoom_range(zoom_range);
        });
        cx.spawn(async move |this, cx| {
            flipping.await;
            let Ok(applying) = this.update(cx, |this, cx| this.apply_connections(cx)) else {
                return;
            };
            applying.await;
        })
        .detach();
    }

    pub fn toggle_torch(&mut self, cx: &mut Context<Self>) {
        let on = !self.torch_on;
        self.set_torch(on, cx);
    }

    /// no webcam has a torch so this answers TorchUnsupported and the light stays off
    pub fn set_torch(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.torch_on != on {
            self.torch_on = on;
            cx.notify();
        }
        if !self.session.is_running() {
            return;
        }
        let session = Arc::clone(&self.session);
        self.on_queue(cx, move || {
            session.with_camera(|camera| {
                let _ = camera.set_torch(on);
            });
        })
        .detach();
    }

    pub fn begin_zoom(&mut self) {
        self.zoom_base = self.zoom_factor;
    }

    /// range comes from the open device through the camera seam
    /// a camera without zoom answers none and the range is 1 to 1
    pub fn zoom(&mut self, by: f32, cx: &mut Context<Self>) {
        let (lower, device_upper) = self.session.zoom_range();
        let upper = device_upper.min(Self::MAX_ZOOM_FACTOR);
        let factor = lower.max((self.zoom_base * by).min(upper));
        if (factor - self.zoom_factor).abs() <= 0.001 {
            return;
        }
        self.zoom_factor = factor;
        cx.notify();
        if !self.session.is_running() {
            return;
        }
        self.session.zoom_target.store(factor.to_bits(), Ordering::Release);
        let session = Arc::clone(&self.session);
        self.on_queue(cx, move || {
            let target = f32::from_bits(session.zoom_target.load(Ordering::Acquire));
            session.with_camera(|camera| {
                let _ = camera.set_zoom_factor(target);
            });
        })
        .detach();
    }

    /// point is in capture device space, 0 to 1 landscape
    /// focus and exposure settle on it where the camera can
    /// a camera that cant answers unsupported and keeps its continuous focus
    pub fn focus(&mut self, at_device_point: Point<f32>, cx: &mut Context<Self>) {
        if !self.session.is_running() {
            return;
        }
        let session = Arc::clone(&self.session);
        self.on_queue(cx, move || {
            session.with_camera(|camera| {
                let _ = camera.set_point_of_interest(at_device_point.x, at_device_point.y);
            });
        })
        .detach();
    }

    pub fn capture_photo(&mut self, cx: &mut Context<Self>) -> Task<Option<RiseCameraShot>> {
        if self.is_capturing_photo || self.is_recording || self.access != RiseCameraCaptureAccess::Granted {
            return Task::ready(None);
        }
        self.is_capturing_photo = true;
        cx.notify();

        #[cfg(debug_assertions)]
        if Self::uses_test_source() {
            let front = self.uses_front_camera;
            let taking = cx.background_executor().spawn(async move { RiseCameraTestSource::photo(front) });
            return Self::finish_photo(taking, cx);
        }

        let session = Arc::clone(&self.session);
        let taking = cx.background_executor().spawn(async move { Self::photo_from_preview(&session) });
        Self::finish_photo(taking, cx)
    }

    fn finish_photo(taking: Task<Option<RiseCameraShot>>, cx: &mut Context<Self>) -> Task<Option<RiseCameraShot>> {
        cx.spawn(async move |this, cx| {
            let shot = taking.await;
            let _ = this.update(cx, |this, cx| {
                this.is_capturing_photo = false;
                cx.notify();
            });
            shot
        })
    }

    fn photo_from_preview(session: &RiseCameraSession) -> Option<RiseCameraShot> {
        if !session.is_running() {
            return None;
        }
        let frame = session.latest().1?;
        let rgba = still_rgba_of(&frame).ok()?;
        let pixel_size = size(rgba.width() as f32, rgba.height() as f32);
        let mut data = Vec::new();
        DynamicImage::ImageRgba8(rgba.clone())
            .to_rgb8()
            .write_to(&mut Cursor::new(&mut data), ImageFormat::Jpeg)
            .ok()?;
        let path = Self::temporary_url("jpg");
        Self::write_atomically(&path, &data).ok()?;
        let url = Url::from_file_path(&path)?;
        Some(RiseCameraShot::new(url, Some(CleverImageBitmapDecoder::render_image(rgba)), pixel_size, 0.0))
    }

    pub fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut partial = path.as_os_str().to_owned();
        partial.push(".partial");
        let partial = PathBuf::from(partial);
        std::fs::write(&partial, data)?;
        std::fs::rename(&partial, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&partial);
        })
    }

    pub fn start_recording(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        if self.is_recording || self.is_capturing_photo || self.access != RiseCameraCaptureAccess::Granted {
            return Task::ready(false);
        }
        if Self::uses_test_source() {
            self.is_recording = true;
            self.recording_started_at = Some(Date::now());
            cx.notify();
            return Task::ready(true);
        }
        if !self.session.is_running() {
            return Task::ready(false);
        }
        let attaching = self.attach_microphone_if_allowed(cx);
        cx.spawn(async move |this, cx| {
            attaching.await;
            let Ok(Some(starting)) = this.update(cx, |this, cx| {
                if !this.wants_running || !this.session.is_running() {
                    return None;
                }
                let session = Arc::clone(&this.session);
                Some(cx.background_executor().spawn(async move {
                    session.with_camera(|camera| camera.record(RiseClock::monotonic()).is_ok()).unwrap_or(false)
                }))
            }) else {
                return false;
            };
            let started = starting.await;
            this.update(cx, |this, cx| {
                if !started {
                    return false;
                }
                this.is_recording = true;
                this.recording_started_at = Some(Date::now());
                let _ = haptics::impact(ImpactStyle::Medium);
                cx.notify();
                true
            })
            .unwrap_or(false)
        })
    }

    pub fn stop_recording(&mut self, cx: &mut Context<Self>) -> Task<Option<RiseCameraShot>> {
        if !self.is_recording {
            return Task::ready(None);
        }

        #[cfg(debug_assertions)]
        if Self::uses_test_source() {
            let elapsed = Date::now().time_interval_since(&self.recording_started_at.unwrap_or_else(Date::now));
            self.is_recording = false;
            self.recording_started_at = None;
            cx.notify();
            let front = self.uses_front_camera;
            return cx.spawn(async move |_, _| RiseCameraTestSource::video(elapsed.max(1.0), front).await);
        }

        let session = Arc::clone(&self.session);
        let stopping = cx.background_executor().spawn(async move { Self::end_take(&session) });
        cx.spawn(async move |this, cx| {
            let path = stopping.await;
            let detaching = this.update(cx, |this, cx| {
                this.is_recording = false;
                this.recording_started_at = None;
                cx.notify();
                this.detach_microphone(cx)
            });
            if let Ok(detaching) = detaching {
                detaching.await;
            }
            let path = path?;
            let url = Url::from_file_path(&path)?;
            match Self::video_shot(url).await {
                Some(shot) => Some(shot),
                None => {
                    let _ = std::fs::remove_file(&path);
                    None
                }
            }
        })
    }

    /// closes the segment of the take, a segment the writer failed to finish is removed
    /// blocking
    fn end_take(session: &RiseCameraSession) -> Option<PathBuf> {
        session
            .with_camera(|camera| {
                let path = camera.files().last().cloned();
                match camera.pause(RiseClock::monotonic()) {
                    Ok(()) => path,
                    Err(_) => {
                        if let Some(path) = path {
                            let _ = std::fs::remove_file(path);
                        }
                        None
                    }
                }
            })
            .flatten()
    }

    fn cancel_recording(&mut self, cx: &mut Context<Self>) {
        if !self.is_recording {
            return;
        }
        self.is_recording = false;
        self.recording_started_at = None;
        cx.notify();
        if Self::uses_test_source() {
            return;
        }
        let session = Arc::clone(&self.session);
        self.on_queue(cx, move || {
            if let Some(path) = Self::end_take(&session) {
                let _ = std::fs::remove_file(path);
            }
        })
        .detach();
        self.detach_microphone(cx).detach();
    }

    fn attach_microphone_if_allowed(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if self.has_audio_input
            || !matches!(capture_permission::status(CaptureKind::Microphone), Ok(CaptureAccess::Granted))
        {
            return Task::ready(());
        }
        let session = Arc::clone(&self.session);
        let attaching = cx.background_executor().spawn(async move {
            session.with_camera(|camera| camera.set_microphone(true).is_ok()).unwrap_or(false)
        });
        cx.spawn(async move |this, cx| {
            let attached = attaching.await;
            let _ = this.update(cx, |this, _| this.has_audio_input = attached);
        })
    }

    fn detach_microphone(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if !self.has_audio_input {
            return Task::ready(());
        }
        self.has_audio_input = false;
        let session = Arc::clone(&self.session);
        self.on_queue(cx, move || {
            session.with_camera(|camera| {
                let _ = camera.set_microphone(false);
            });
        })
    }

    pub async fn video_shot(url: Url) -> Option<RiseCameraShot> {
        let path = url.to_file_path()?;
        let probed = RiseAccountRuntime::current().spawn_blocking(move || probe(&path)).await.ok()?.ok()?;
        if !(probed.duration_seconds.is_finite() && probed.duration_seconds > 0.0) {
            return None;
        }
        let pixel_size = Self::display_size(&probed);
        Some(RiseCameraShot::new(url, None, pixel_size, probed.duration_seconds))
    }

    /// stored frame turned by the track transform
    fn display_size(probed: &VideoProbe) -> Size<f32> {
        if !probed.has_video {
            return size(1.0, 1.0);
        }
        let (width, height) = (probed.natural_size.width, probed.natural_size.height);
        let corners = [(0.0, 0.0), (width, 0.0), (0.0, height), (width, height)]
            .map(|(x, y)| probed.preferred_transform.apply(x, y));
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for (x, y) in corners {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        size((max_x - min_x).abs().max(1.0) as f32, (max_y - min_y).abs().max(1.0) as f32)
    }

    pub fn temporary_url(path_extension: &str) -> PathBuf {
        let directory = temporary_directory().join(RiseCameraSession::CAPTURE_DIRECTORY_NAME);
        let _ = std::fs::create_dir_all(&directory);
        directory.join(format!("{}.{}", Uuid::new_v4().uuid_string(), path_extension))
    }
}
