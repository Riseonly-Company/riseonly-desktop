use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use rise_platform::{HostOs, PlatformSupport};
use thiserror::Error;

use crate::rtc_media::{screen_share_liveness, ScreenShareLiveness, VideoShelf};
use crate::texture::color::{ColorDescription, ColorRange, ColorSpace};
use crate::texture::frame::FrameFormat;
use crate::texture::native_surface::HostNv12Frame;

pub const PICKER_WINDOW: Duration = Duration::from_secs(60);
pub const DEFAULT_MAX_FPS: u32 = 30;
pub const MACOS_SCREEN_CAPTURE_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CaptureSourceKind {
    Display,
    Window,
}

/// one display or window the user chose
/// id is opaque outside the os file, eg display id or window id or pipewire node or picker token
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CaptureSource {
    pub id: String,
    pub kind: CaptureSourceKind,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

impl CaptureSource {
    /// size rtc_media screen_share_shelf takes as display
    pub fn pixel_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// window a system picker is shown over
/// the portal names it as a string, windows needs the hwnd
/// macos presents its picker on its own
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum PickerParent {
    #[default]
    None,
    Win32(isize),
    X11(u32),
    Wayland(String),
}

pub fn portal_parent_window(parent: &PickerParent) -> String {
    match parent {
        PickerParent::X11(id) => format!("x11:{id:x}"),
        PickerParent::Wayland(handle) if !handle.is_empty() => format!("wayland:{handle}"),
        PickerParent::None | PickerParent::Win32(_) | PickerParent::Wayland(_) => String::new(),
    }
}

/// what a capture asks of the system, the shelf size and rate
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CaptureOptions {
    pub max_fps: u32,
    pub max_long_side: u32,
    pub max_short_side: u32,
}

impl CaptureOptions {
    pub fn from_shelf(shelf: &VideoShelf) -> Self {
        let fps = if shelf.fps == 0 { DEFAULT_MAX_FPS } else { shelf.fps };
        Self {
            max_fps: fps,
            max_long_side: shelf.width.max(shelf.height),
            max_short_side: shelf.width.min(shelf.height),
        }
    }

    /// minimum time between two frames the system should deliver
    pub fn frame_interval(&self) -> Duration {
        Duration::from_secs(1) / self.max_fps.max(1)
    }

    /// size frames are delivered at for a source of width by height
    /// scaled down to fit the shelf with aspect kept, never up
    /// both sides even (nv12 chroma is 2x2) and at least 2
    pub fn output_size(&self, width: u32, height: u32) -> (u32, u32) {
        if width == 0 || height == 0 {
            return (2, 2);
        }
        let portrait = height > width;
        let (long, short) = if portrait { (height, width) } else { (width, height) };
        let max_long = if self.max_long_side == 0 { long } else { self.max_long_side };
        let max_short = if self.max_short_side == 0 { short } else { self.max_short_side };
        let (long, short) = if long <= max_long && short <= max_short {
            (long, short)
        } else if u64::from(max_short) * u64::from(long) < u64::from(max_long) * u64::from(short) {
            (scaled(long, max_short, short), max_short)
        } else {
            (max_long, scaled(short, max_long, long))
        };
        let even = |side: u32| (side & !1).max(2);
        let (long, short) = (even(long), even(short));
        if portrait { (short, long) } else { (long, short) }
    }
}

/// side * numerator / denominator rounded down, no overflow
fn scaled(side: u32, numerator: u32, denominator: u32) -> u32 {
    let value = u64::from(side) * u64::from(numerator) / u64::from(denominator.max(1));
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// pixels of one captured picture
#[derive(Clone)]
pub enum ScreenPixels {
    Nv12 { luma: Arc<[u8]>, luma_stride: u32, chroma: Arc<[u8]>, chroma_stride: u32 },
    Bgra { pixels: Arc<[u8]>, stride: u32 },
}

impl std::fmt::Debug for ScreenPixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nv12 { luma, chroma, .. } => f
                .debug_struct("Nv12")
                .field("luma_bytes", &luma.len())
                .field("chroma_bytes", &chroma.len())
                .finish(),
            Self::Bgra { pixels, stride } => f
                .debug_struct("Bgra")
                .field("bytes", &pixels.len())
                .field("stride", stride)
                .finish(),
        }
    }
}

/// one captured picture with its monotonic capture time
#[derive(Clone, Debug)]
pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: ScreenPixels,
    pub captured_at: Duration,
}

impl ScreenFrame {
    /// frame as the published track takes it
    /// a bgra picture is converted here on the capture thread
    pub fn into_host_frame(self, stream_id: u64, generation: u64) -> Result<HostNv12Frame, ScreenCaptureError> {
        let (luma, luma_stride, chroma, chroma_stride) = match self.pixels {
            ScreenPixels::Nv12 { luma, luma_stride, chroma, chroma_stride } => {
                check_nv12(&luma, luma_stride, &chroma, chroma_stride, self.width, self.height)?;
                (luma, luma_stride, chroma, chroma_stride)
            }
            ScreenPixels::Bgra { pixels, stride } => {
                let (luma, chroma) = bgra_to_nv12(&pixels, self.width, self.height, stride)?;
                let chroma_stride = self.width.div_ceil(2) * 2;
                (luma.into(), self.width, chroma.into(), chroma_stride)
            }
        };
        let description = ColorDescription { space: ColorSpace::Bt601, range: ColorRange::Limited };
        Ok(HostNv12Frame {
            stream_id,
            generation,
            width: self.width,
            height: self.height,
            luma,
            luma_stride,
            chroma,
            chroma_stride,
            conversion: description.params(FrameFormat::Nv12),
        })
    }
}

fn check_nv12(
    luma: &[u8],
    luma_stride: u32,
    chroma: &[u8],
    chroma_stride: u32,
    width: u32,
    height: u32,
) -> Result<(), ScreenCaptureError> {
    if width == 0 || height == 0 || luma_stride < width || chroma_stride < width.div_ceil(2) * 2 {
        return Err(ScreenCaptureError::BadFrame("NV12 geometry"));
    }
    let luma_needed = u64::from(luma_stride) * u64::from(height - 1) + u64::from(width);
    let chroma_rows = height.div_ceil(2);
    let chroma_needed = u64::from(chroma_stride) * u64::from(chroma_rows - 1) + u64::from(width.div_ceil(2) * 2);
    if (luma.len() as u64) < luma_needed || (chroma.len() as u64) < chroma_needed {
        return Err(ScreenCaptureError::BadFrame("NV12 frame is shorter than its geometry"));
    }
    Ok(())
}

/// packed bgra to tight nv12, bt601 limited range
/// chroma averaged over each 2x2 block, same matrix webrtc assumes for untagged nv12
pub fn bgra_to_nv12(source: &[u8], width: u32, height: u32, stride: u32) -> Result<(Vec<u8>, Vec<u8>), ScreenCaptureError> {
    let (w, h, stride) = (width as usize, height as usize, stride as usize);
    if w == 0 || h == 0 || stride < w * 4 {
        return Err(ScreenCaptureError::BadFrame("BGRA geometry"));
    }
    if source.len() < stride * (h - 1) + w * 4 {
        return Err(ScreenCaptureError::BadFrame("BGRA frame is shorter than its geometry"));
    }
    let chroma_width = w.div_ceil(2);
    let mut luma = vec![0u8; w * h];
    let mut chroma = vec![0u8; chroma_width * 2 * h.div_ceil(2)];
    let pixel = |x: usize, y: usize| -> (i32, i32, i32) {
        let at = y * stride + x * 4;
        (i32::from(source[at + 2]), i32::from(source[at + 1]), i32::from(source[at]))
    };
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = pixel(x, y);
            luma[y * w + x] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(0, 255) as u8;
        }
    }
    for cy in 0..h.div_ceil(2) {
        for cx in 0..chroma_width {
            let (mut r, mut g, mut b, mut count) = (0, 0, 0, 0);
            for (x, y) in [(cx * 2, cy * 2), (cx * 2 + 1, cy * 2), (cx * 2, cy * 2 + 1), (cx * 2 + 1, cy * 2 + 1)] {
                if x < w && y < h {
                    let (pr, pg, pb) = pixel(x, y);
                    r += pr;
                    g += pg;
                    b += pb;
                    count += 1;
                }
            }
            let (r, g, b) = (r / count, g / count, b / count);
            let u = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            let v = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            let at = cy * chroma_width * 2 + cx * 2;
            chroma[at] = u;
            chroma[at + 1] = v;
        }
    }
    Ok((luma, chroma))
}

/// scales a packed bgra picture to out_width by out_height with tight rows, nearest sample
/// for branches whose system hands over full size pictures
pub fn scale_bgra(
    source: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    out_width: u32,
    out_height: u32,
) -> Result<Vec<u8>, ScreenCaptureError> {
    let (w, h, stride) = (width as usize, height as usize, stride as usize);
    let (ow, oh) = (out_width as usize, out_height as usize);
    if w == 0 || h == 0 || ow == 0 || oh == 0 || stride < w * 4 {
        return Err(ScreenCaptureError::BadFrame("BGRA geometry"));
    }
    if source.len() < stride * (h - 1) + w * 4 {
        return Err(ScreenCaptureError::BadFrame("BGRA frame is shorter than its geometry"));
    }
    let mut out = vec![0u8; ow * oh * 4];
    if ow == w && oh == h {
        for y in 0..h {
            out[y * w * 4..(y + 1) * w * 4].copy_from_slice(&source[y * stride..y * stride + w * 4]);
        }
        return Ok(out);
    }
    for y in 0..oh {
        let sy = (y * h / oh).min(h - 1);
        for x in 0..ow {
            let sx = (x * w / ow).min(w - 1);
            let from = sy * stride + sx * 4;
            let to = (y * ow + x) * 4;
            out[to..to + 4].copy_from_slice(&source[from..from + 4]);
        }
    }
    Ok(out)
}

#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum ScreenCaptureError {
    #[error("screen capture is not written for {0:?} yet")]
    BranchNotWritten(HostOs),
    #[error("screen capture is unavailable: {0}")]
    Unsupported(String),
    #[error("this system has no screen sharing picker")]
    PickerUnavailable,
    #[error("screen recording access was refused")]
    AccessDenied,
    #[error("no screen or window was chosen")]
    Cancelled,
    #[error("the screen sharing picker timed out")]
    PickerTimedOut,
    #[error("the screen sharing picker cannot wait on the main thread")]
    MainThread,
    #[error("the chosen screen or window is gone")]
    SourceGone,
    #[error("screen capture could not start: {0}")]
    StartFailed(String),
    #[error("screen capture stopped: {0}")]
    StreamFailed(String),
    #[error("screen capture delivered a bad frame: {0}")]
    BadFrame(&'static str),
}

impl ScreenCaptureError {
    /// host cant do this at all, the call screen hides the share button
    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::BranchNotWritten(_) | Self::Unsupported(_))
    }

    /// user decided and nothing failed, no error is shown
    pub fn is_user_choice(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

/// screen recording access as each host answers it
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScreenCaptureAccess {
    Granted,
    Denied,
    AskedOnPick,
}

impl From<ScreenCaptureAccess> for rise_platform::privacy_access::AccessStatus {
    fn from(access: ScreenCaptureAccess) -> Self {
        match access {
            ScreenCaptureAccess::Granted | ScreenCaptureAccess::AskedOnPick => Self::Authorized,
            ScreenCaptureAccess::Denied => Self::Denied,
        }
    }
}

/// why a running capture ended without the app asking
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ScreenCaptureEnd {
    SourceLost,
    StoppedBySystem,
    Failed(ScreenCaptureError),
}

/// called on the branch capture thread for every frame, keep it short and hand the frame over
pub type ScreenFrameSink = Arc<dyn Fn(ScreenFrame) + Send + Sync>;
/// called once on any thread when the capture ends without the app asking
pub type ScreenEndSink = Arc<dyn Fn(ScreenCaptureEnd) + Send + Sync>;

/// one block of the machine own sound during a share
/// interleaved 16 bit pcm, rate and channels are what the branch system delivers
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ScreenAudioBlock {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// called on the branch audio thread for every block, keep it short and hand the block over
pub type ScreenAudioSink = Arc<dyn Fn(ScreenAudioBlock) + Send + Sync>;

/// float samples in -1..1 as pcm, out of range clips
pub fn pcm16_from_float(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).collect()
}

/// separate channel planes of floats as one interleaved run
/// frames past the shortest plane lack a full channel set and go
pub fn interleaved_from_planes(planes: &[&[f32]]) -> Vec<f32> {
    let frames = planes.iter().map(|plane| plane.len()).min().unwrap_or(0);
    let mut interleaved = Vec::with_capacity(frames * planes.len());
    for frame in 0..frames {
        interleaved.extend(planes.iter().map(|plane| plane[frame]));
    }
    interleaved
}

/// one running capture, frames flow from start to stop
pub trait ScreenCaptureSession: Send {
    fn source(&self) -> &CaptureSource;
    /// ends the capture
    /// never waits for the system to confirm so the end sink thread may call it, a second call does nothing
    fn stop(&mut self);
}

/// the os capture, default bodies are the honest answer of a branch not written yet
pub trait ScreenCaptureBackend {
    fn support(&self) -> Result<(), ScreenCaptureError> {
        Err(ScreenCaptureError::BranchNotWritten(HostOs::current()))
    }
    fn access(&self) -> ScreenCaptureAccess {
        ScreenCaptureAccess::Denied
    }
    fn request_access(&self) -> ScreenCaptureAccess {
        self.access()
    }
    fn pick_source(&self, _parent: &PickerParent) -> Result<CaptureSource, ScreenCaptureError> {
        Err(ScreenCaptureError::BranchNotWritten(HostOs::current()))
    }
    /// displays and windows the app may list itself, where the host has no picker
    fn sources(&self) -> Result<Vec<CaptureSource>, ScreenCaptureError> {
        Err(ScreenCaptureError::PickerUnavailable)
    }
    fn start(
        &self,
        _source: &CaptureSource,
        _options: CaptureOptions,
        _frames: ScreenFrameSink,
        _ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        Err(ScreenCaptureError::BranchNotWritten(HostOs::current()))
    }
    fn system_audio_support(&self) -> Result<(), ScreenCaptureError> {
        Err(ScreenCaptureError::BranchNotWritten(HostOs::current()))
    }
    fn start_with_system_audio(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        _audio: ScreenAudioSink,
        ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        self.start(source, options, frames, ended)
    }
}

/// privacy pane where a refused access is given back, where the host has one
pub const fn settings_url(host: HostOs) -> Option<&'static str> {
    match host {
        HostOs::MacOs => Some(MACOS_SCREEN_CAPTURE_SETTINGS_URL),
        HostOs::Linux | HostOs::Windows => None,
    }
}

pub fn open_settings(app: &mut gpui::App) -> PlatformSupport {
    match settings_url(HostOs::current()) {
        Some(url) => {
            app.open_url(url);
            PlatformSupport::Performed
        }
        None => PlatformSupport::Unsupported,
    }
}

/// capture of this host, or the test picture in a driven tester session
pub fn native() -> impl ScreenCaptureBackend {
    HostScreenCapture
}

struct HostScreenCapture;

impl HostScreenCapture {
    fn backend(&self) -> &'static dyn ScreenCaptureBackend {
        static NATIVE: super::os::NativeScreenCapture = super::os::NativeScreenCapture;
        static TESTER: TesterScreenCapture = TesterScreenCapture;
        if rise_platform::capture_permission::uses_tester_sources() { &TESTER } else { &NATIVE }
    }
}

impl ScreenCaptureBackend for HostScreenCapture {
    fn support(&self) -> Result<(), ScreenCaptureError> {
        self.backend().support()
    }
    fn access(&self) -> ScreenCaptureAccess {
        self.backend().access()
    }
    fn request_access(&self) -> ScreenCaptureAccess {
        self.backend().request_access()
    }
    fn pick_source(&self, parent: &PickerParent) -> Result<CaptureSource, ScreenCaptureError> {
        self.backend().pick_source(parent)
    }
    fn sources(&self) -> Result<Vec<CaptureSource>, ScreenCaptureError> {
        self.backend().sources()
    }
    fn start(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        self.backend().start(source, options, frames, ended)
    }
    fn system_audio_support(&self) -> Result<(), ScreenCaptureError> {
        self.backend().system_audio_support()
    }
    fn start_with_system_audio(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        audio: ScreenAudioSink,
        ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        self.backend().start_with_system_audio(source, options, frames, audio, ended)
    }
}

pub const TESTER_SCREEN_ID: &str = "tester-screen";
pub const TESTER_SCREEN_SIZE: (u32, u32) = (1280, 720);
pub const TESTER_SCREEN_MAX_FPS: u32 = 15;
const TESTER_SCREEN_LOOP_SECONDS: f32 = 3.0;
const TESTER_SCREEN_AUDIO_TICK: Duration = Duration::from_millis(10);

struct TesterScreenCapture;

fn tester_screen_source() -> CaptureSource {
    CaptureSource {
        id: TESTER_SCREEN_ID.to_owned(),
        kind: CaptureSourceKind::Display,
        name: "Tester screen".to_owned(),
        width: TESTER_SCREEN_SIZE.0,
        height: TESTER_SCREEN_SIZE.1,
    }
}

impl ScreenCaptureBackend for TesterScreenCapture {
    fn support(&self) -> Result<(), ScreenCaptureError> {
        Ok(())
    }
    fn access(&self) -> ScreenCaptureAccess {
        ScreenCaptureAccess::Granted
    }
    fn pick_source(&self, _parent: &PickerParent) -> Result<CaptureSource, ScreenCaptureError> {
        Ok(tester_screen_source())
    }
    fn sources(&self) -> Result<Vec<CaptureSource>, ScreenCaptureError> {
        Ok(vec![tester_screen_source()])
    }
    fn start(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        _ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        TesterScreenSession::start(source, options, frames, None)
    }
    fn system_audio_support(&self) -> Result<(), ScreenCaptureError> {
        Ok(())
    }
    fn start_with_system_audio(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        audio: ScreenAudioSink,
        _ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        TesterScreenSession::start(source, options, frames, Some(audio))
    }
}

struct TesterScreenSession {
    source: CaptureSource,
    stop: Arc<AtomicBool>,
}

impl TesterScreenSession {
    fn start(
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        audio: Option<ScreenAudioSink>,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let size = options.output_size(TESTER_SCREEN_SIZE.0, TESTER_SCREEN_SIZE.1);
        let frame_interval = Duration::from_secs(1) / options.max_fps.clamp(1, TESTER_SCREEN_MAX_FPS);
        std::thread::Builder::new()
            .name("rise-tester-screen".to_owned())
            .spawn(move || {
                let format = crate::audio_capture::TESTER_SIGNAL_FORMAT;
                let started = RiseClock::monotonic();
                let mut next_frame = Duration::ZERO;
                let mut written: u64 = 0;
                while !thread_stop.load(Ordering::Acquire) {
                    let now = RiseClock::monotonic();
                    let elapsed = now.saturating_sub(started);
                    if elapsed >= next_frame {
                        next_frame = elapsed + frame_interval;
                        let phase = (elapsed.as_secs_f32() % TESTER_SCREEN_LOOP_SECONDS) / TESTER_SCREEN_LOOP_SECONDS;
                        let picture = crate::camera_capture::tester_camera::tester_frame(size, phase, false);
                        let bgra = crate::camera_capture::tester_camera::rgba_to_bgra(&picture);
                        frames(ScreenFrame {
                            width: size.0,
                            height: size.1,
                            pixels: ScreenPixels::Bgra { pixels: bgra.into(), stride: size.0 * 4 },
                            captured_at: now,
                        });
                    }
                    if let Some(audio) = audio.as_ref() {
                        let due = (elapsed.as_secs_f64() * f64::from(format.sample_rate)) as u64;
                        if due > written {
                            let samples: Vec<f32> =
                                (written..due).map(crate::audio_capture::tester_signal_sample).collect();
                            written = due;
                            audio(ScreenAudioBlock {
                                samples: pcm16_from_float(&samples),
                                sample_rate: format.sample_rate,
                                channels: format.channels,
                            });
                        }
                    }
                    std::thread::sleep(TESTER_SCREEN_AUDIO_TICK);
                }
            })
            .map_err(|error| ScreenCaptureError::StartFailed(error.to_string()))?;
        Ok(Box::new(Self { source: source.clone(), stop }))
    }
}

impl ScreenCaptureSession for TesterScreenSession {
    fn source(&self) -> &CaptureSource {
        &self.source
    }
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl Drop for TesterScreenSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

pub fn screen_share_support() -> PlatformSupport {
    match native().support() {
        Ok(()) => PlatformSupport::Performed,
        Err(_) => PlatformSupport::Unsupported,
    }
}

pub fn screen_share_unavailable_reason() -> Option<ScreenCaptureError> {
    native().support().err()
}

pub fn system_audio_support() -> PlatformSupport {
    match native().system_audio_support() {
        Ok(()) => PlatformSupport::Performed,
        Err(_) => PlatformSupport::Unsupported,
    }
}

pub fn system_audio_unavailable_reason() -> Option<ScreenCaptureError> {
    native().system_audio_support().err()
}

fn breadcrumb(message: impl FnOnce() -> String) {
    RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Media, message);
}

fn kind_label(kind: CaptureSourceKind) -> &'static str {
    match kind {
        CaptureSourceKind::Display => "display",
        CaptureSourceKind::Window => "window",
    }
}

/// what one share keeps across the capture thread and the caller
struct ShareState {
    session: Mutex<Option<Box<dyn ScreenCaptureSession>>>,
    frames: AtomicU64,
    audio_seen: AtomicBool,
    ended: AtomicBool,
}

impl ShareState {
    /// takes the session out and stops it once, later calls find it gone
    fn stop_session(&self) {
        let session = self.session.lock().take();
        if let Some(mut session) = session {
            session.stop();
        }
    }
}

/// one screen share from start to stop
/// counts frames for the first frame probe and logs diagnostics
/// stops the capture when the source goes away before the end reaches the caller
pub struct ScreenShareCapture {
    state: Arc<ShareState>,
    source: CaptureSource,
    started_at: Duration,
}

impl ScreenShareCapture {
    pub fn start(
        backend: &dyn ScreenCaptureBackend,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        ended: ScreenEndSink,
    ) -> Result<Self, ScreenCaptureError> {
        Self::begin(backend, source, options, frames, None, ended)
    }

    pub fn start_with_system_audio(
        backend: &dyn ScreenCaptureBackend,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        audio: ScreenAudioSink,
        ended: ScreenEndSink,
    ) -> Result<Self, ScreenCaptureError> {
        let audio = match backend.system_audio_support() {
            Ok(()) => Some(audio),
            Err(error) => {
                breadcrumb(|| format!("screen share: no system sound: {error}"));
                None
            }
        };
        Self::begin(backend, source, options, frames, audio, ended)
    }

    fn begin(
        backend: &dyn ScreenCaptureBackend,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        audio: Option<ScreenAudioSink>,
        ended: ScreenEndSink,
    ) -> Result<Self, ScreenCaptureError> {
        let state = Arc::new(ShareState {
            session: Mutex::new(None),
            frames: AtomicU64::new(0),
            audio_seen: AtomicBool::new(false),
            ended: AtomicBool::new(false),
        });
        let frame_state = Arc::downgrade(&state);
        let counted: ScreenFrameSink = Arc::new(move |frame: ScreenFrame| {
            let Some(state) = frame_state.upgrade() else {
                return;
            };
            if state.ended.load(Ordering::Acquire) {
                return;
            }
            if state.frames.fetch_add(1, Ordering::AcqRel) == 0 {
                breadcrumb(|| format!("screen share: first frame {}x{}", frame.width, frame.height));
            }
            frames(frame);
        });
        let end_state = Arc::downgrade(&state);
        let on_end: ScreenEndSink = Arc::new(move |end: ScreenCaptureEnd| {
            let Some(state) = end_state.upgrade() else {
                return;
            };
            if state.ended.swap(true, Ordering::AcqRel) {
                return;
            }
            match &end {
                ScreenCaptureEnd::SourceLost => breadcrumb(|| "screen share: source lost".into()),
                ScreenCaptureEnd::StoppedBySystem => breadcrumb(|| "screen share: stopped by the system".into()),
                ScreenCaptureEnd::Failed(error) => {
                    RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: {error}"))
                }
            }
            state.stop_session();
            ended(end);
        });
        breadcrumb(|| {
            format!("screen share: start {} {}x{} at {} fps", kind_label(source.kind), source.width, source.height, options.max_fps)
        });
        let started = match audio {
            Some(audio) => {
                let audio_state = Arc::downgrade(&state);
                let gated: ScreenAudioSink = Arc::new(move |block: ScreenAudioBlock| {
                    let Some(state) = audio_state.upgrade() else {
                        return;
                    };
                    if state.ended.load(Ordering::Acquire) {
                        return;
                    }
                    if !state.audio_seen.swap(true, Ordering::AcqRel) {
                        breadcrumb(|| format!("screen share: first sound {} Hz {} ch", block.sample_rate, block.channels));
                    }
                    audio(block);
                });
                backend.start_with_system_audio(source, options, counted, gated, on_end)
            }
            None => backend.start(source, options, counted, on_end),
        };
        let session = match started {
            Ok(session) => session,
            Err(error) => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: start failed: {error}"));
                return Err(error);
            }
        };
        if state.ended.load(Ordering::Acquire) {
            let mut session = session;
            session.stop();
        } else {
            *state.session.lock() = Some(session);
        }
        Ok(Self { state, source: source.clone(), started_at: RiseClock::monotonic() })
    }

    pub fn source(&self) -> &CaptureSource {
        &self.source
    }

    pub fn frames_seen(&self) -> u64 {
        self.state.frames.load(Ordering::Acquire)
    }

    /// whether the capture still runs, false after stop or an end from the system
    pub fn is_running(&self) -> bool {
        !self.state.ended.load(Ordering::Acquire)
    }

    /// first frame probe at now
    /// a still screen sends nothing after its first picture so only no frame at all counts
    pub fn liveness(&self, now: Duration) -> ScreenShareLiveness {
        screen_share_liveness(self.frames_seen(), now.saturating_sub(self.started_at))
    }

    /// ends the capture at the app request, the end sink is not called
    pub fn stop(&mut self) {
        if self.state.ended.swap(true, Ordering::AcqRel) {
            return;
        }
        breadcrumb(|| "screen share: stop".into());
        self.state.stop_session();
    }
}

impl Drop for ScreenShareCapture {
    fn drop(&mut self) {
        self.stop();
    }
}
