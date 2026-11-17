use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rise_platform::{HostOs, PlatformSupport};
use thiserror::Error;

pub use super::still_frame::still_rgba_of;
use crate::texture::frame::{FrameFormat, FrameGeometry};
use super::tester_camera::{TesterCamera, uses_tester_camera};

pub const MINIMUM_FRAME_RATE: u32 = 30;
pub const TARGET_WIDTH: u32 = 1280;
pub const TARGET_HEIGHT: u32 = 720;

/// where a camera looks, plus the webcam on a cable
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CameraPosition {
    Front,
    Back,
    External,
    Unspecified,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CameraDevice {
    pub id: String,
    pub name: String,
    pub position: CameraPosition,
    pub built_in: bool,
}

/// how a camera hands over its pixels before they become a preview frame
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PixelEncoding {
    Yuyv,
    Nv12,
    Mjpeg,
    Bgra,
}

impl PixelEncoding {
    pub fn is_compressed(self) -> bool {
        matches!(self, Self::Mjpeg)
    }
}

/// one mode a camera offers, encoding and size and frame interval
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CameraMode {
    pub encoding: PixelEncoding,
    pub width: u32,
    pub height: u32,
    pub interval_numerator: u32,
    pub interval_denominator: u32,
}

impl CameraMode {
    /// whole frames per second rounded down, 0 for a mode without an interval
    pub fn frames_per_second(&self) -> u32 {
        if self.interval_numerator == 0 {
            return 0;
        }
        self.interval_denominator / self.interval_numerator
    }

    fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// the mode the session opens, at least MINIMUM_FRAME_RATE if any mode reaches it
/// then the smallest size that covers the target or the largest below it
pub fn choose_mode(modes: &[CameraMode]) -> Option<CameraMode> {
    let target = u64::from(TARGET_WIDTH) * u64::from(TARGET_HEIGHT);
    let fast_enough = modes.iter().any(|mode| mode.frames_per_second() >= MINIMUM_FRAME_RATE);
    let candidates = modes
        .iter()
        .filter(|mode| mode.width > 0 && mode.height > 0)
        .filter(|mode| !fast_enough || mode.frames_per_second() >= MINIMUM_FRAME_RATE);
    candidates
        .min_by_key(|mode| {
            let size_rank = if mode.area() >= target {
                (0, mode.area())
            } else {
                (1, u64::MAX - mode.area())
            };
            let rate_rank = if fast_enough {
                0
            } else {
                u32::MAX - mode.frames_per_second()
            };
            (rate_rank, size_rank, mode.encoding.is_compressed(), u32::MAX - mode.frames_per_second())
        })
        .copied()
}

/// the camera a take opens, front or back wide angle
/// rarely a back one, so front falls back to built in then first
/// one camera hides the flip button
pub fn pick_device(devices: &[CameraDevice], front: bool) -> Option<&CameraDevice> {
    if front {
        return devices
            .iter()
            .find(|device| device.position == CameraPosition::Front)
            .or_else(|| devices.iter().find(|device| device.built_in))
            .or_else(|| devices.first());
    }
    let front_choice = pick_device(devices, true);
    devices
        .iter()
        .find(|device| device.position == CameraPosition::Back)
        .or_else(|| devices.iter().find(|device| Some(*device) != front_choice))
}

/// true when the flip button shows, ie there is a second camera
pub fn can_flip(devices: &[CameraDevice]) -> bool {
    match (pick_device(devices, true), pick_device(devices, false)) {
        (Some(front), Some(back)) => front != back,
        _ => false,
    }
}

/// file kind of a segment, macos writes quicktime and the other two hosts mpeg-4
/// both hold h264 and the composition reads either
pub fn segment_extension(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => "mov",
        HostOs::Linux | HostOs::Windows => "mp4",
    }
}

/// one preview frame ready for a stream texture
#[derive(Clone, Debug)]
pub struct CameraFrame {
    pub geometry: FrameGeometry,
    pub pixels: Arc<[u8]>,
    pub captured_at: Duration,
    pub mirrored: bool,
}

/// called on the capture thread for every frame, keep it short and hand the frame over
pub type FrameSink = Arc<dyn Fn(CameraFrame) + Send + Sync>;

/// bt.601 limited range yuyv (Y0 U Y1 V) to bgra, what every webcam 4:2:2 needs
/// uvc cameras state bt.601 for uncompressed formats
pub fn yuyv_to_bgra(
    source: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    out: &mut Vec<u8>,
) -> Result<(), CameraCaptureError> {
    let (width, height, stride) = (width as usize, height as usize, stride as usize);
    if width == 0 || height == 0 || width % 2 != 0 || stride < width * 2 {
        return Err(CameraCaptureError::BadFrame("YUYV geometry"));
    }
    if source.len() < stride * (height - 1) + width * 2 {
        return Err(CameraCaptureError::BadFrame("YUYV frame is shorter than its geometry"));
    }
    out.clear();
    out.reserve(width * height * 4);
    for row in 0..height {
        let line = &source[row * stride..row * stride + width * 2];
        for pair in line.chunks_exact(4) {
            let (u, v) = (i32::from(pair[1]) - 128, i32::from(pair[3]) - 128);
            for y in [pair[0], pair[2]] {
                let c = (i32::from(y) - 16).max(0) * 298;
                let r = (c + 409 * v + 128) >> 8;
                let g = (c - 100 * u - 208 * v + 128) >> 8;
                let b = (c + 516 * u + 128) >> 8;
                out.extend_from_slice(&[clamp(b), clamp(g), clamp(r), 255]);
            }
        }
    }
    Ok(())
}

fn clamp(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

pub fn bgra_frame(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    captured_at: Duration,
    mirrored: bool,
) -> Result<CameraFrame, CameraCaptureError> {
    let geometry = FrameGeometry::packed(FrameFormat::Bgra8, width, height)
        .map_err(|_| CameraCaptureError::BadFrame("BGRA geometry"))?;
    if (pixels.len() as u64) < geometry.byte_len() {
        return Err(CameraCaptureError::BadFrame("BGRA frame is shorter than its geometry"));
    }
    Ok(CameraFrame { geometry, pixels: pixels.into(), captured_at, mirrored })
}

#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum CameraCaptureError {
    #[error("camera capture is not written for {0:?} yet")]
    BranchNotWritten(HostOs),
    #[error("no camera is connected")]
    NoCamera,
    #[error("no H.264 encoder: {0}")]
    EncoderUnavailable(String),
    #[error("the camera is unavailable: {0}")]
    Unavailable(String),
    #[error("the camera is in use by another program")]
    Busy,
    #[error("camera access was refused")]
    AccessDenied,
    #[error("the camera could not be opened: {0}")]
    OpenFailed(String),
    #[error("the camera stopped delivering frames: {0}")]
    StreamFailed(String),
    #[error("the camera delivered a bad frame: {0}")]
    BadFrame(&'static str),
    #[error("no webcam has a torch")]
    TorchUnsupported,
    #[error("this camera has no zoom")]
    ZoomUnsupported,
    #[error("this camera takes no point of interest")]
    PointOfInterestUnsupported,
    #[error("this camera records no sound")]
    MicrophoneUnsupported,
    #[error("this camera cannot record mirrored")]
    MirroringUnsupported,
    #[error("the segment file failed: {0}")]
    FileFailed(String),
    #[error("a segment is already being written")]
    AlreadyRecording,
    #[error("no segment is being written")]
    NotRecording,
}

impl CameraCaptureError {
    /// true if the host cant do this at all, the screen hides the button instead of failing
    pub fn is_unsupported(&self) -> bool {
        matches!(
            self,
            Self::BranchNotWritten(_)
                | Self::NoCamera
                | Self::EncoderUnavailable(_)
                | Self::Unavailable(_)
                | Self::TorchUnsupported
                | Self::ZoomUnsupported
                | Self::PointOfInterestUnsupported
                | Self::MicrophoneUnsupported
                | Self::MirroringUnsupported
        )
    }
}

/// one open camera, frames flow to the sink from open to close
/// a segment is written between start_segment and stop_segment
pub trait CameraSession: Send {
    fn device(&self) -> &CameraDevice;
    fn mode(&self) -> CameraMode;
    fn start_segment(&mut self, _path: &Path) -> Result<(), CameraCaptureError> {
        Err(CameraCaptureError::BranchNotWritten(HostOs::current()))
    }
    fn stop_segment(&mut self) -> Result<(), CameraCaptureError> {
        Err(CameraCaptureError::BranchNotWritten(HostOs::current()))
    }
    fn set_torch(&mut self, _on: bool) -> Result<(), CameraCaptureError> {
        Err(CameraCaptureError::TorchUnsupported)
    }
    /// zoom factors the open device takes, lowest and highest
    /// none if no zoom
    fn zoom_range(&self) -> Option<(f32, f32)> {
        None
    }
    fn set_zoom_factor(&mut self, _factor: f32) -> Result<(), CameraCaptureError> {
        Err(CameraCaptureError::ZoomUnsupported)
    }
    fn set_point_of_interest(&mut self, _x: f32, _y: f32) -> Result<(), CameraCaptureError> {
        Err(CameraCaptureError::PointOfInterestUnsupported)
    }
    /// mic sound in segments starting after this call, off removes the mic
    /// blocking
    fn set_microphone(&mut self, on: bool) -> Result<(), CameraCaptureError> {
        if on {
            return Err(CameraCaptureError::MicrophoneUnsupported);
        }
        Ok(())
    }
    fn set_mirrors_recording(&mut self, on: bool) -> Result<(), CameraCaptureError> {
        if on {
            return Err(CameraCaptureError::MirroringUnsupported);
        }
        Ok(())
    }
    /// why frames stopped (camera unplugged or driver error) once they have, none while they flow
    /// the overlay shows the error and ends the take
    fn failure(&self) -> Option<CameraCaptureError> {
        None
    }
}

/// the os camera, default bodies are the honest answer of a branch not written yet
pub trait CameraSource {
    fn support(&self) -> Result<(), CameraCaptureError> {
        Err(CameraCaptureError::BranchNotWritten(HostOs::current()))
    }
    fn recording_support(&self) -> Result<(), CameraCaptureError> {
        Err(CameraCaptureError::BranchNotWritten(HostOs::current()))
    }
    fn devices(&self) -> Result<Vec<CameraDevice>, CameraCaptureError> {
        Err(CameraCaptureError::BranchNotWritten(HostOs::current()))
    }
    fn open(
        &self,
        _device: &CameraDevice,
        _frames: FrameSink,
    ) -> Result<Box<dyn CameraSession>, CameraCaptureError> {
        Err(CameraCaptureError::BranchNotWritten(HostOs::current()))
    }
}

pub fn video_note_support() -> PlatformSupport {
    match video_note_unavailable_reason() {
        None => PlatformSupport::Performed,
        Some(_) => PlatformSupport::Unsupported,
    }
}

/// why video note support answers Unsupported
pub fn video_note_unavailable_reason() -> Option<CameraCaptureError> {
    let source = camera_source();
    source.support().and_then(|()| source.recording_support()).err()
}

/// cameras of this host, for the picker and the flip button
pub fn devices() -> Result<Vec<CameraDevice>, CameraCaptureError> {
    camera_source().devices()
}

static NATIVE_CAMERA: super::os::NativeCamera = super::os::NativeCamera;
static TESTER_CAMERA_SOURCE: TesterCamera = TesterCamera;

fn camera_source() -> &'static dyn CameraSource {
    if uses_tester_camera() { &TESTER_CAMERA_SOURCE } else { &NATIVE_CAMERA }
}

/// one finished segment of a take in recording order
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecordedSegment {
    pub path: PathBuf,
    pub duration: Duration,
    pub front: bool,
}

/// segment bookkeeping of one take, names and the open segment and recorded time
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SegmentPlan {
    directory: PathBuf,
    stem: String,
    extension: &'static str,
    next_index: u32,
    open: Option<(PathBuf, Duration, bool)>,
    finished: Vec<RecordedSegment>,
}

impl SegmentPlan {
    pub fn new(directory: PathBuf, stem: String, host: HostOs) -> Self {
        Self {
            directory,
            stem,
            extension: segment_extension(host),
            next_index: 0,
            open: None,
            finished: Vec::new(),
        }
    }

    /// path of the next segment (stem-n.ext)
    /// marks it open from now
    pub fn begin(&mut self, now: Duration, front: bool) -> Result<PathBuf, CameraCaptureError> {
        if self.open.is_some() {
            return Err(CameraCaptureError::AlreadyRecording);
        }
        let name = format!("{}-{}.{}", self.stem, self.next_index, self.extension);
        self.next_index += 1;
        let path = self.directory.join(name);
        self.open = Some((path.clone(), now, front));
        Ok(path)
    }

    /// closes the open segment at now
    pub fn end(&mut self, now: Duration) -> Result<&RecordedSegment, CameraCaptureError> {
        let (path, started, front) = self.open.take().ok_or(CameraCaptureError::NotRecording)?;
        self.finished.push(RecordedSegment { path, duration: now.saturating_sub(started), front });
        Ok(&self.finished[self.finished.len() - 1])
    }

    /// forgets the open segment after its writer failed to start, its file is the caller to drop
    pub fn abandon(&mut self) -> Option<PathBuf> {
        self.open.take().map(|(path, _, _)| path)
    }

    pub fn is_recording(&self) -> bool {
        self.open.is_some()
    }

    /// recorded time so far, the open segment counted up to now
    pub fn elapsed(&self, now: Duration) -> Duration {
        let finished: Duration = self.finished.iter().map(|segment| segment.duration).sum();
        let open = self.open.as_ref().map_or(Duration::ZERO, |(_, started, _)| now.saturating_sub(*started));
        finished + open
    }

    pub fn segments(&self) -> &[RecordedSegment] {
        &self.finished
    }

    /// every file of the take finished and open, for cleanup after a cancel
    pub fn all_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.finished.iter().map(|segment| segment.path.clone()).collect();
        if let Some((path, _, _)) = &self.open {
            paths.push(path.clone());
        }
        paths
    }
}

/// zoom range a device reports, kept only if it really zooms
/// both ends finite and lower at least 1 and upper above it
pub fn usable_zoom_range(lower: f32, upper: f32) -> Option<(f32, f32)> {
    let finite = lower.is_finite() && upper.is_finite();
    (finite && lower >= 1.0 && upper > lower).then_some((lower, upper))
}

/// factor held inside range, a non finite factor stays at the lower end
pub fn clamp_zoom_factor(factor: f32, range: (f32, f32)) -> f32 {
    let (lower, upper) = range;
    if !factor.is_finite() {
        return lower;
    }
    factor.clamp(lower, upper)
}

/// point of interest held in the unit square, a non finite coordinate is the center
pub fn clamp_point_of_interest(x: f32, y: f32) -> (f32, f32) {
    let unit = |value: f32| if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.5 };
    (unit(x), unit(y))
}

/// a video note take, the open camera and its segments and the flip between cameras
pub struct VideoNoteCamera {
    session: Option<Box<dyn CameraSession>>,
    device: CameraDevice,
    devices: Vec<CameraDevice>,
    front: bool,
    frames: FrameSink,
    plan: SegmentPlan,
    microphone: bool,
    mirrors_recording: bool,
}

impl VideoNoteCamera {
    /// opens the front or back camera and starts the preview, blocking
    pub fn open(
        front: bool,
        directory: PathBuf,
        stem: String,
        frames: FrameSink,
    ) -> Result<Self, CameraCaptureError> {
        let source = camera_source();
        source.support()?;
        let devices = source.devices()?;
        let device = pick_device(&devices, front).ok_or(CameraCaptureError::NoCamera)?.clone();
        let session = source.open(&device, frames.clone())?;
        Ok(Self {
            session: Some(session),
            device,
            devices,
            front,
            frames,
            plan: SegmentPlan::new(directory, stem, HostOs::current()),
            microphone: false,
            mirrors_recording: false,
        })
    }

    pub fn device(&self) -> &CameraDevice {
        &self.device
    }

    fn session_mut(&mut self) -> Result<&mut Box<dyn CameraSession>, CameraCaptureError> {
        self.session.as_mut().ok_or(CameraCaptureError::NoCamera)
    }

    pub fn suspend(&mut self, now: Duration) -> Result<(), CameraCaptureError> {
        let paused = if self.plan.is_recording() { self.pause(now) } else { Ok(()) };
        self.session = None;
        paused
    }

    pub fn is_suspended(&self) -> bool {
        self.session.is_none()
    }

    /// reopens the device a suspend closed and the preview starts again, blocking
    pub fn reactivate(&mut self) -> Result<(), CameraCaptureError> {
        if self.session.is_some() {
            return Ok(());
        }
        let device = pick_device(&self.devices, self.front).ok_or(CameraCaptureError::NoCamera)?.clone();
        let mut session = camera_source().open(&device, self.frames.clone())?;
        if self.microphone {
            session.set_microphone(true)?;
        }
        self.device = session.device().clone();
        self.session = Some(session);
        Ok(())
    }

    pub fn can_flip(&self) -> bool {
        can_flip(&self.devices)
    }

    pub fn is_front(&self) -> bool {
        self.front
    }

    /// starts recording (or resumes after a pause) into a new segment, blocking
    pub fn record(&mut self, now: Duration) -> Result<(), CameraCaptureError> {
        let mirrors_recording = self.mirrors_recording;
        match self.session_mut()?.set_mirrors_recording(mirrors_recording) {
            Ok(()) => {}
            Err(error) if error.is_unsupported() => {
                tracing::debug!(target: "riseonly", %error, "segment recorded unmirrored");
            }
            Err(error) => return Err(error),
        }
        let path = self.plan.begin(now, self.front)?;
        if let Err(error) = self.session_mut().and_then(|session| session.start_segment(&path)) {
            self.plan.abandon();
            return Err(error);
        }
        Ok(())
    }

    /// pauses, the segment is finalized and the preview keeps running
    /// blocking
    pub fn pause(&mut self, now: Duration) -> Result<(), CameraCaptureError> {
        if !self.plan.is_recording() {
            return Err(CameraCaptureError::NotRecording);
        }
        let stopped = self.session_mut().and_then(|session| session.stop_segment());
        self.plan.end(now)?;
        stopped
    }

    /// switches cameras
    /// while recording the segment ends and the next starts on the other camera, blocking
    pub fn flip(&mut self, now: Duration) -> Result<(), CameraCaptureError> {
        let target = pick_device(&self.devices, !self.front)
            .filter(|device| **device != self.device)
            .ok_or(CameraCaptureError::NoCamera)?
            .clone();
        let was_recording = self.plan.is_recording();
        if was_recording {
            self.pause(now)?;
        }
        let source = camera_source();
        let mut session = source.open(&target, self.frames.clone())?;
        if self.microphone {
            session.set_microphone(true)?;
        }
        self.device = session.device().clone();
        self.session = Some(session);
        self.front = !self.front;
        if was_recording {
            self.record(now)?;
        }
        Ok(())
    }

    /// torch, no webcam has one
    pub fn set_torch(&mut self, on: bool) -> Result<(), CameraCaptureError> {
        self.session_mut()?.set_torch(on)
    }

    /// zoom factors of the open device (the new one after a flip), none means no zoom
    pub fn zoom_range(&self) -> Option<(f32, f32)> {
        self.session.as_ref().and_then(|session| session.zoom_range())
    }

    pub fn set_zoom_factor(&mut self, factor: f32) -> Result<(), CameraCaptureError> {
        self.session_mut()?.set_zoom_factor(factor)
    }

    pub fn set_point_of_interest(&mut self, x: f32, y: f32) -> Result<(), CameraCaptureError> {
        let (x, y) = clamp_point_of_interest(x, y);
        self.session_mut()?.set_point_of_interest(x, y)
    }

    /// mic sound in the next segment and on (after a flip too)
    /// off takes it out, blocking
    pub fn set_microphone(&mut self, on: bool) -> Result<(), CameraCaptureError> {
        self.session_mut()?.set_microphone(on)?;
        self.microphone = on;
        Ok(())
    }

    pub fn set_mirrors_recording(&mut self, on: bool) {
        self.mirrors_recording = on;
    }

    pub fn elapsed(&self, now: Duration) -> Duration {
        self.plan.elapsed(now)
    }

    /// why the camera stopped delivering frames, once it has
    pub fn failure(&self) -> Option<CameraCaptureError> {
        self.session.as_ref().and_then(|session| session.failure())
    }

    /// ends the take and hands the segments to the composition, blocking
    pub fn finish(mut self, now: Duration) -> Result<Vec<RecordedSegment>, CameraCaptureError> {
        if self.plan.is_recording() {
            self.pause(now)?;
        }
        Ok(self.plan.segments().to_vec())
    }

    /// every file the take wrote, for the cancel path to remove
    pub fn files(&self) -> Vec<PathBuf> {
        self.plan.all_paths()
    }
}
