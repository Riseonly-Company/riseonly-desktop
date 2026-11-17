use std::f32::consts::PI;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use image::RgbaImage;
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;

use super::camera_capture::{
    CameraCaptureError, CameraDevice, CameraMode, CameraPosition, CameraSession, CameraSource, FrameSink,
    PixelEncoding, bgra_frame,
};
use crate::video_export::FrameVideoWriter;

pub const TESTER_FRONT_ID: &str = "tester-camera:front";
pub const TESTER_BACK_ID: &str = "tester-camera:back";
pub const TESTER_FRAME_SIZE: (u32, u32) = (640, 360);
pub const TESTER_FRAMES_PER_SECOND: u32 = 15;
const TESTER_BIT_RATE: u32 = 1_500_000;
const TESTER_LOOP_SECONDS: f32 = 3.0;

static TESTER_CAMERA: AtomicBool = AtomicBool::new(false);

pub fn set_tester_camera(on: bool) {
    TESTER_CAMERA.store(on, Ordering::Release);
}

pub fn uses_tester_camera() -> bool {
    TESTER_CAMERA.load(Ordering::Acquire)
}

pub fn tester_colors(front: bool) -> ([f32; 3], [f32; 3]) {
    if front {
        ([0.95, 0.45, 0.25], [0.55, 0.15, 0.6])
    } else {
        ([0.15, 0.55, 0.95], [0.1, 0.8, 0.55])
    }
}

/// one frame of the test picture at phase 0..1 of the loop, gradient plus a white disc that circles
pub fn tester_frame(size: (u32, u32), phase: f32, front: bool) -> RgbaImage {
    let (top, bottom) = tester_colors(front);
    let (width, height) = (size.0 as f32, size.1 as f32);
    let radius = width.min(height) * 0.18;
    let center_x = width / 2.0 + (phase * PI * 2.0).sin() * width * 0.25;
    let center_y = height / 2.0 + (phase * PI * 2.0).cos() * height * 0.2;
    let diagonal = width * width + height * height;
    RgbaImage::from_fn(size.0, size.1, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let t = ((fx * width + fy * height) / diagonal).clamp(0.0, 1.0);
        let mut channels = [0.0_f32; 3];
        for (index, channel) in channels.iter_mut().enumerate() {
            *channel = top[index] + (bottom[index] - top[index]) * t;
        }
        let (dx, dy) = (fx - center_x, fy - center_y);
        if dx * dx + dy * dy <= radius * radius {
            for channel in &mut channels {
                *channel = *channel * 0.15 + 0.85;
            }
        }
        let [r, g, b] = channels.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
        image::Rgba([r, g, b, 255])
    })
}

pub(crate) fn rgba_to_bgra(rgba: &RgbaImage) -> Vec<u8> {
    let mut bgra = Vec::with_capacity(rgba.as_raw().len());
    for pixel in rgba.as_raw().chunks_exact(4) {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    bgra
}

pub struct TesterCamera;

impl CameraSource for TesterCamera {
    fn support(&self) -> Result<(), CameraCaptureError> {
        Ok(())
    }

    fn recording_support(&self) -> Result<(), CameraCaptureError> {
        Ok(())
    }

    fn devices(&self) -> Result<Vec<CameraDevice>, CameraCaptureError> {
        Ok(vec![
            CameraDevice {
                id: TESTER_FRONT_ID.to_owned(),
                name: "Tester camera (front)".to_owned(),
                position: CameraPosition::Front,
                built_in: true,
            },
            CameraDevice {
                id: TESTER_BACK_ID.to_owned(),
                name: "Tester camera (back)".to_owned(),
                position: CameraPosition::Back,
                built_in: true,
            },
        ])
    }

    fn open(&self, device: &CameraDevice, frames: FrameSink) -> Result<Box<dyn CameraSession>, CameraCaptureError> {
        Ok(Box::new(TesterCameraSession::start(device.clone(), frames)?))
    }
}

/// segment being written, the writer and its frame count
struct OpenSegment {
    writer: FrameVideoWriter,
    frames: i64,
}

struct TesterCameraSession {
    device: CameraDevice,
    segment: Arc<Mutex<Option<OpenSegment>>>,
    failure: Arc<Mutex<Option<CameraCaptureError>>>,
    stop: Arc<AtomicBool>,
}

impl TesterCameraSession {
    fn start(device: CameraDevice, frames: FrameSink) -> Result<Self, CameraCaptureError> {
        let segment: Arc<Mutex<Option<OpenSegment>>> = Arc::new(Mutex::new(None));
        let failure: Arc<Mutex<Option<CameraCaptureError>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let front = device.position == CameraPosition::Front;
        let (thread_segment, thread_failure, thread_stop) = (Arc::clone(&segment), Arc::clone(&failure), Arc::clone(&stop));
        std::thread::Builder::new()
            .name("rise-tester-camera".to_owned())
            .spawn(move || {
                let interval = Duration::from_secs(1) / TESTER_FRAMES_PER_SECOND;
                while !thread_stop.load(Ordering::Acquire) {
                    let now = RiseClock::monotonic();
                    let phase = (now.as_secs_f32() % TESTER_LOOP_SECONDS) / TESTER_LOOP_SECONDS;
                    let picture = tester_frame(TESTER_FRAME_SIZE, phase, front);
                    if let Some(open) = thread_segment.lock().as_mut() {
                        if open.writer.append(&picture, open.frames).is_ok() {
                            open.frames += 1;
                        }
                    }
                    match bgra_frame(rgba_to_bgra(&picture), TESTER_FRAME_SIZE.0, TESTER_FRAME_SIZE.1, now, front) {
                        Ok(frame) => frames(frame),
                        Err(error) => {
                            *thread_failure.lock() = Some(error);
                            return;
                        }
                    }
                    std::thread::sleep(interval.saturating_sub(RiseClock::monotonic().saturating_sub(now)));
                }
            })
            .map_err(|error| CameraCaptureError::OpenFailed(error.to_string()))?;
        Ok(Self { device, segment, failure, stop })
    }
}

impl Drop for TesterCameraSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl CameraSession for TesterCameraSession {
    fn device(&self) -> &CameraDevice {
        &self.device
    }

    fn mode(&self) -> CameraMode {
        CameraMode {
            encoding: PixelEncoding::Bgra,
            width: TESTER_FRAME_SIZE.0,
            height: TESTER_FRAME_SIZE.1,
            interval_numerator: 1,
            interval_denominator: TESTER_FRAMES_PER_SECOND,
        }
    }

    fn start_segment(&mut self, path: &Path) -> Result<(), CameraCaptureError> {
        let writer = FrameVideoWriter::create(
            path,
            TESTER_FRAME_SIZE.0,
            TESTER_FRAME_SIZE.1,
            TESTER_FRAMES_PER_SECOND,
            TESTER_BIT_RATE,
        )
        .map_err(|error| CameraCaptureError::EncoderUnavailable(error.to_string()))?;
        *self.segment.lock() = Some(OpenSegment { writer, frames: 0 });
        Ok(())
    }

    fn stop_segment(&mut self) -> Result<(), CameraCaptureError> {
        let open = self.segment.lock().take().ok_or(CameraCaptureError::NotRecording)?;
        let duration = open.frames.max(1) as f64 / f64::from(TESTER_FRAMES_PER_SECOND);
        open.writer
            .finish(duration)
            .map_err(|error| CameraCaptureError::StreamFailed(error.to_string()))
    }

    fn failure(&self) -> Option<CameraCaptureError> {
        self.failure.lock().clone()
    }
}

