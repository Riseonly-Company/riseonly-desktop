use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread::Thread;
use std::time::Duration;

use parking_lot::Mutex;

use super::camera_capture::{
    pick_device, CameraCaptureError, CameraDevice, CameraFrame, CameraSession, CameraSource, FrameSink,
};
use super::tester_camera::{uses_tester_camera, TesterCamera};
use crate::rtc_media::VideoShelf;
use crate::screen_capture::bgra_to_nv12;
use crate::texture::color::{ColorDescription, ColorRange, ColorSpace};
use crate::texture::frame::FrameFormat;
use crate::texture::native_surface::HostNv12Frame;

/// called on the capture thread for every picture handed to the track, keep it short
pub type CallFrameSink = Arc<dyn Fn(HostNv12Frame) + Send + Sync>;

/// called once on the capture monitor thread when the camera stopped delivering frames
pub type CallCameraEndSink = Arc<dyn Fn(CameraCaptureError) + Send + Sync>;

const FAILURE_POLL: Duration = Duration::from_millis(500);

const PACING_TOLERANCE_PERCENT: u64 = 85;

const NO_FRAME: u64 = u64::MAX;

static NEXT_CALL_CAMERA_STREAM: AtomicU64 = AtomicU64::new(0x00ca_0000_0000_0000);

static NATIVE_CALL_CAMERA: super::os::NativeCamera = super::os::NativeCamera;
static TESTER_CALL_CAMERA: TesterCamera = TesterCamera;

fn call_camera_source() -> &'static dyn CameraSource {
    if uses_tester_camera() { &TESTER_CALL_CAMERA } else { &NATIVE_CALL_CAMERA }
}

/// the camera a call opens, the one the room names if connected
/// otherwise the front one
pub fn call_device<'a>(devices: &'a [CameraDevice], device_id: Option<&str>) -> Option<&'a CameraDevice> {
    device_id
        .and_then(|wanted| devices.iter().find(|device| device.id == wanted))
        .or_else(|| pick_device(devices, true))
}

/// true if a frame captured now goes to the track, no more than fps frames a second
/// the rate of the shelf the track was published with
pub fn admits_frame(last: Option<Duration>, now: Duration, fps: u32) -> bool {
    let Some(last) = last else {
        return true;
    };
    if fps == 0 {
        return true;
    }
    let interval_micros = 1_000_000 / u64::from(fps);
    let elapsed = now.saturating_sub(last).as_micros();
    elapsed >= u128::from(interval_micros * PACING_TOLERANCE_PERCENT / 100)
}

/// one camera picture as the published track takes it
/// bgra is converted here on the capture thread and nv12 is copied plane by plane
pub fn host_nv12_of(frame: &CameraFrame, stream_id: u64, generation: u64) -> Result<HostNv12Frame, CameraCaptureError> {
    let geometry = &frame.geometry;
    let (width, height) = (geometry.width(), geometry.height());
    if width == 0 || height == 0 {
        return Err(CameraCaptureError::BadFrame("camera frame geometry"));
    }
    let planes = geometry.planes();
    let (luma, luma_stride, chroma, chroma_stride): (Arc<[u8]>, u32, Arc<[u8]>, u32) = match geometry.format() {
        FrameFormat::Bgra8 => {
            let stride = planes.first().map_or(width * 4, |plane| plane.stride);
            let (luma, chroma) = bgra_to_nv12(&frame.pixels, width, height, stride)
                .map_err(|_| CameraCaptureError::BadFrame("BGRA camera frame"))?;
            (luma.into(), width, chroma.into(), width.div_ceil(2) * 2)
        }
        FrameFormat::Nv12 => {
            let [luma_plane, chroma_plane] = planes else {
                return Err(CameraCaptureError::BadFrame("NV12 camera frame planes"));
            };
            let chroma_rows = height.div_ceil(2);
            let luma_needed = u64::from(luma_plane.stride) * u64::from(height - 1) + u64::from(width);
            let chroma_needed =
                u64::from(chroma_plane.stride) * u64::from(chroma_rows - 1) + u64::from(width.div_ceil(2) * 2);
            if luma_plane.stride < width || chroma_plane.stride < width.div_ceil(2) * 2 {
                return Err(CameraCaptureError::BadFrame("NV12 camera frame strides"));
            }
            let luma = plane_bytes(&frame.pixels, luma_plane.offset, luma_needed)?;
            let chroma = plane_bytes(&frame.pixels, chroma_plane.offset, chroma_needed)?;
            (luma, luma_plane.stride, chroma, chroma_plane.stride)
        }
        _ => return Err(CameraCaptureError::BadFrame("camera frame format")),
    };
    let description = ColorDescription { space: ColorSpace::Bt601, range: ColorRange::Limited };
    Ok(HostNv12Frame {
        stream_id,
        generation,
        width,
        height,
        luma,
        luma_stride,
        chroma,
        chroma_stride,
        conversion: description.params(FrameFormat::Nv12),
    })
}

fn plane_bytes(pixels: &[u8], offset: u64, needed: u64) -> Result<Arc<[u8]>, CameraCaptureError> {
    let start = usize::try_from(offset).map_err(|_| CameraCaptureError::BadFrame("NV12 plane offset"))?;
    let length = usize::try_from(needed).map_err(|_| CameraCaptureError::BadFrame("NV12 plane length"))?;
    let end = start.checked_add(length).ok_or(CameraCaptureError::BadFrame("NV12 plane length"))?;
    pixels
        .get(start..end)
        .map(Arc::from)
        .ok_or(CameraCaptureError::BadFrame("NV12 camera frame is shorter than its geometry"))
}

fn pack_dimensions(width: u32, height: u32) -> u64 {
    (u64::from(width) << 32) | u64::from(height)
}

fn unpack_dimensions(packed: u64) -> Option<(u32, u32)> {
    (packed != NO_FRAME).then(|| ((packed >> 32) as u32, packed as u32))
}

struct CallCameraState {
    device: CameraDevice,
    session: Mutex<Option<Box<dyn CameraSession>>>,
    frames: FrameSink,
    stopped: AtomicBool,
    ended: AtomicBool,
    dimensions: AtomicU64,
    last_sent_micros: AtomicU64,
    generation: AtomicU64,
}

impl CallCameraState {
    fn take_session(&self) -> Option<Box<dyn CameraSession>> {
        self.session.lock().take()
    }
}

/// camera publishing to the call from start to stop (or drop)
pub struct CallCameraCapture {
    state: Arc<CallCameraState>,
    monitor: Option<Thread>,
}

impl CallCameraCapture {
    /// opens the camera the room asked for and starts handing its frames over, blocking
    pub fn start(
        device_id: Option<&str>,
        shelf: &VideoShelf,
        frames: CallFrameSink,
        ended: CallCameraEndSink,
    ) -> Result<Self, CameraCaptureError> {
        Self::start_with(call_camera_source(), device_id, shelf, frames, ended)
    }

    /// start over any camera source, tests drive a fake one
    pub fn start_with(
        source: &dyn CameraSource,
        device_id: Option<&str>,
        shelf: &VideoShelf,
        frames: CallFrameSink,
        ended: CallCameraEndSink,
    ) -> Result<Self, CameraCaptureError> {
        source.support()?;
        let devices = source.devices()?;
        let device = call_device(&devices, device_id).ok_or(CameraCaptureError::NoCamera)?.clone();
        let stream_id = NEXT_CALL_CAMERA_STREAM.fetch_add(1, Ordering::Relaxed);
        let fps = shelf.fps;
        let state = Arc::new_cyclic(|weak: &Weak<CallCameraState>| {
            let weak = weak.clone();
            let sink: FrameSink = Arc::new(move |frame: CameraFrame| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                if state.stopped.load(Ordering::Acquire) {
                    return;
                }
                let now_micros = u64::try_from(frame.captured_at.as_micros()).unwrap_or(u64::MAX - 1);
                let last = state.last_sent_micros.load(Ordering::Acquire);
                let last = (last != NO_FRAME).then(|| Duration::from_micros(last));
                if !admits_frame(last, frame.captured_at, fps) {
                    return;
                }
                state.last_sent_micros.store(now_micros, Ordering::Release);
                let generation = state.generation.fetch_add(1, Ordering::AcqRel) + 1;
                match host_nv12_of(&frame, stream_id, generation) {
                    Ok(host_frame) => {
                        state.dimensions.store(pack_dimensions(host_frame.width, host_frame.height), Ordering::Release);
                        frames(host_frame);
                    }
                    Err(error) => tracing::debug!(%error, "call camera frame unreadable"),
                }
            });
            CallCameraState {
                device: device.clone(),
                session: Mutex::new(None),
                frames: sink,
                stopped: AtomicBool::new(false),
                ended: AtomicBool::new(false),
                dimensions: AtomicU64::new(NO_FRAME),
                last_sent_micros: AtomicU64::new(NO_FRAME),
                generation: AtomicU64::new(0),
            }
        });
        let session = source.open(&device, state.frames.clone())?;
        *state.session.lock() = Some(session);

        let monitor_state = state.clone();
        let monitor = std::thread::Builder::new()
            .name("rise-call-camera".into())
            .spawn(move || monitor_failures(&monitor_state, &ended))
            .map_err(|error| CameraCaptureError::OpenFailed(error.to_string()))?;
        Ok(Self { state, monitor: Some(monitor.thread().clone()) })
    }

    pub fn device(&self) -> &CameraDevice {
        &self.state.device
    }

    /// size of the last frame handed to the track, none until one arrived
    pub fn dimensions(&self) -> Option<(u32, u32)> {
        unpack_dimensions(self.state.dimensions.load(Ordering::Acquire))
    }

    /// closes the camera and opens it again, blocking
    pub fn restart(&self) -> Result<(), CameraCaptureError> {
        self.restart_with(call_camera_source())
    }

    /// restart over any camera source, tests drive a fake one
    pub fn restart_with(&self, source: &dyn CameraSource) -> Result<(), CameraCaptureError> {
        if self.state.stopped.load(Ordering::Acquire) || self.state.ended.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut held = self.state.session.lock();
        drop(held.take());
        self.state.dimensions.store(NO_FRAME, Ordering::Release);
        self.state.last_sent_micros.store(NO_FRAME, Ordering::Release);
        let session = source.open(&self.state.device, self.state.frames.clone())?;
        *held = Some(session);
        Ok(())
    }

    /// stops the frames and closes the camera, ended is not called
    /// safe from any thread, the end sink too
    pub fn stop(&mut self) {
        if self.state.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        drop(self.state.take_session());
        if let Some(monitor) = self.monitor.take() {
            monitor.unpark();
        }
    }
}

impl Drop for CallCameraCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn monitor_failures(state: &CallCameraState, ended: &CallCameraEndSink) {
    loop {
        std::thread::park_timeout(FAILURE_POLL);
        if state.stopped.load(Ordering::Acquire) {
            return;
        }
        let failure = state.session.lock().as_ref().and_then(|session| session.failure());
        let Some(failure) = failure else {
            continue;
        };
        if state.ended.swap(true, Ordering::AcqRel) {
            return;
        }
        tracing::warn!(error = %failure, "call camera stopped");
        drop(state.take_session());
        ended(failure);
        return;
    }
}
