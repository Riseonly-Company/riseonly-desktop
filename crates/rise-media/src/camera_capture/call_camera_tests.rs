use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use super::call_camera::*;
use crate::camera_capture::{
    CameraCaptureError, CameraDevice, CameraFrame, CameraMode, CameraPosition, CameraSession, CameraSource, FrameSink,
    PixelEncoding,
};
use crate::rtc_media::VideoShelf;
use crate::texture::frame::{FrameFormat, FrameGeometry};

fn device(id: &str, position: CameraPosition) -> CameraDevice {
    CameraDevice { id: id.into(), name: id.into(), position, built_in: position == CameraPosition::Front }
}

fn shelf(fps: u32) -> VideoShelf {
    VideoShelf { width: 1280, height: 720, fps, encoding: None }
}

fn bgra(width: u32, height: u32, at: Duration) -> CameraFrame {
    let geometry = FrameGeometry::packed(FrameFormat::Bgra8, width, height).expect("geometry");
    CameraFrame { geometry, pixels: vec![255; (width * height * 4) as usize].into(), captured_at: at, mirrored: false }
}

#[test]
fn the_room_device_wins_and_an_unknown_one_falls_back_to_the_front_camera() {
    let devices = [device("usb", CameraPosition::External), device("lid", CameraPosition::Front)];
    assert_eq!(call_device(&devices, Some("usb")).map(|d| d.id.as_str()), Some("usb"));
    assert_eq!(call_device(&devices, Some("gone")).map(|d| d.id.as_str()), Some("lid"));
    assert_eq!(call_device(&devices, None).map(|d| d.id.as_str()), Some("lid"));
    assert!(call_device(&[], None).is_none());
}

#[test]
fn frames_are_paced_to_the_shelf_rate_with_some_jitter_allowed() {
    let at = Duration::from_millis;
    assert!(admits_frame(None, at(0), 15));
    assert!(!admits_frame(Some(at(0)), at(33), 15));
    assert!(admits_frame(Some(at(0)), at(60), 15));
    assert!(admits_frame(Some(at(0)), at(67), 15));
    assert!(admits_frame(Some(at(0)), at(1), 0));
}

#[test]
fn a_white_bgra_frame_becomes_limited_range_nv12() {
    let frame = host_nv12_of(&bgra(4, 2, Duration::ZERO), 7, 1).expect("nv12");
    assert_eq!((frame.width, frame.height, frame.luma_stride, frame.chroma_stride), (4, 2, 4, 4));
    assert!(frame.luma.iter().all(|&y| y == 235));
    assert!(frame.chroma.iter().all(|&c| c == 128));
    assert_eq!((frame.stream_id, frame.generation), (7, 1));
}

#[test]
fn an_nv12_frame_is_copied_plane_by_plane() {
    let geometry = FrameGeometry::with_strides(FrameFormat::Nv12, 4, 2, &[4, 4]).expect("geometry");
    let mut pixels = vec![16_u8; 8];
    pixels.extend_from_slice(&[100, 200, 101, 201]);
    let frame = CameraFrame { geometry, pixels: pixels.into(), captured_at: Duration::ZERO, mirrored: true };
    let host = host_nv12_of(&frame, 1, 2).expect("nv12");
    assert_eq!(&host.luma[..], &[16; 8]);
    assert_eq!(&host.chroma[..], &[100, 200, 101, 201]);
}

#[test]
fn a_short_frame_is_refused() {
    let geometry = FrameGeometry::packed(FrameFormat::Bgra8, 4, 2).expect("geometry");
    let frame = CameraFrame { geometry, pixels: vec![0; 8].into(), captured_at: Duration::ZERO, mirrored: false };
    assert!(matches!(host_nv12_of(&frame, 1, 1), Err(CameraCaptureError::BadFrame(_))));
}

struct FakeSession {
    device: CameraDevice,
    failure: Arc<Mutex<Option<CameraCaptureError>>>,
}

impl CameraSession for FakeSession {
    fn device(&self) -> &CameraDevice {
        &self.device
    }
    fn mode(&self) -> CameraMode {
        CameraMode {
            encoding: PixelEncoding::Bgra,
            width: 4,
            height: 2,
            interval_numerator: 1,
            interval_denominator: 30,
        }
    }
    fn failure(&self) -> Option<CameraCaptureError> {
        self.failure.lock().clone()
    }
}

#[derive(Default)]
struct FakeCamera {
    opened: AtomicUsize,
    sink: Mutex<Option<FrameSink>>,
    failure: Arc<Mutex<Option<CameraCaptureError>>>,
}

impl CameraSource for FakeCamera {
    fn support(&self) -> Result<(), CameraCaptureError> {
        Ok(())
    }
    fn devices(&self) -> Result<Vec<CameraDevice>, CameraCaptureError> {
        Ok(vec![device("lid", CameraPosition::Front)])
    }
    fn open(&self, device: &CameraDevice, frames: FrameSink) -> Result<Box<dyn CameraSession>, CameraCaptureError> {
        self.opened.fetch_add(1, Ordering::SeqCst);
        *self.sink.lock() = Some(frames);
        Ok(Box::new(FakeSession { device: device.clone(), failure: self.failure.clone() }))
    }
}

impl FakeCamera {
    fn deliver(&self, frame: CameraFrame) {
        let sink = self.sink.lock().clone().expect("open");
        sink(frame);
    }
}

#[test]
fn frames_reach_the_track_and_set_the_dimensions_until_a_restart() {
    let camera = FakeCamera::default();
    let pushed = Arc::new(AtomicUsize::new(0));
    let counter = pushed.clone();
    let capture = CallCameraCapture::start_with(
        &camera,
        None,
        &shelf(30),
        Arc::new(move |_frame| {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(|_error| {}),
    )
    .expect("start");
    assert_eq!(capture.dimensions(), None);
    camera.deliver(bgra(4, 2, Duration::from_millis(0)));
    camera.deliver(bgra(4, 2, Duration::from_millis(5)));
    camera.deliver(bgra(4, 2, Duration::from_millis(40)));
    assert_eq!(pushed.load(Ordering::SeqCst), 2);
    assert_eq!(capture.dimensions(), Some((4, 2)));

    capture.restart_with(&camera).expect("restart");
    assert_eq!(camera.opened.load(Ordering::SeqCst), 2);
    assert_eq!(capture.dimensions(), None);
}

#[test]
fn a_stopped_capture_drops_late_frames() {
    let camera = FakeCamera::default();
    let pushed = Arc::new(AtomicUsize::new(0));
    let counter = pushed.clone();
    let mut capture = CallCameraCapture::start_with(
        &camera,
        Some("lid"),
        &shelf(30),
        Arc::new(move |_frame| {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(|_error| {}),
    )
    .expect("start");
    capture.stop();
    camera.deliver(bgra(4, 2, Duration::ZERO));
    assert_eq!(pushed.load(Ordering::SeqCst), 0);
}

#[test]
fn a_failed_session_ends_the_capture_once() {
    let camera = FakeCamera::default();
    let ends = Arc::new(Mutex::new(Vec::new()));
    let seen = ends.clone();
    let _capture = CallCameraCapture::start_with(
        &camera,
        None,
        &shelf(30),
        Arc::new(|_frame| {}),
        Arc::new(move |error| seen.lock().push(error)),
    )
    .expect("start");
    *camera.failure.lock() = Some(CameraCaptureError::StreamFailed("unplugged".into()));
    for _ in 0..40 {
        if !ends.lock().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(ends.lock().as_slice(), &[CameraCaptureError::StreamFailed("unplugged".into())]);
}
