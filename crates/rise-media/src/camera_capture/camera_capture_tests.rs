use std::path::PathBuf;
use std::time::Duration;

use crate::camera_capture::camera_capture::*;
use rise_platform::HostOs;

fn mode(encoding: PixelEncoding, width: u32, height: u32, fps: u32) -> CameraMode {
    CameraMode { encoding, width, height, interval_numerator: 1, interval_denominator: fps }
}

fn camera(id: &str, position: CameraPosition, built_in: bool) -> CameraDevice {
    CameraDevice { id: id.to_owned(), name: id.to_owned(), position, built_in }
}

#[test]
fn a_fast_mode_wins_over_a_larger_slow_one() {
    let modes = [
        mode(PixelEncoding::Yuyv, 1280, 720, 10),
        mode(PixelEncoding::Mjpeg, 1280, 720, 30),
        mode(PixelEncoding::Yuyv, 640, 480, 30),
    ];
    assert_eq!(choose_mode(&modes), Some(mode(PixelEncoding::Mjpeg, 1280, 720, 30)));
}

#[test]
fn uncompressed_wins_at_the_same_size_and_rate() {
    let modes = [mode(PixelEncoding::Mjpeg, 1280, 720, 30), mode(PixelEncoding::Yuyv, 1280, 720, 30)];
    assert_eq!(choose_mode(&modes).map(|m| m.encoding), Some(PixelEncoding::Yuyv));
}

#[test]
fn the_smallest_size_covering_the_target_is_chosen() {
    let modes = [
        mode(PixelEncoding::Mjpeg, 1920, 1080, 30),
        mode(PixelEncoding::Mjpeg, 1280, 720, 30),
        mode(PixelEncoding::Mjpeg, 640, 480, 30),
    ];
    assert_eq!(choose_mode(&modes).map(|m| m.width), Some(1280));
}

#[test]
fn below_the_target_the_largest_size_is_chosen() {
    let modes = [mode(PixelEncoding::Yuyv, 320, 240, 30), mode(PixelEncoding::Yuyv, 640, 480, 30)];
    assert_eq!(choose_mode(&modes).map(|m| m.width), Some(640));
}

#[test]
fn without_a_fast_mode_the_fastest_is_chosen() {
    let modes = [mode(PixelEncoding::Yuyv, 1280, 720, 10), mode(PixelEncoding::Yuyv, 640, 480, 15)];
    assert_eq!(choose_mode(&modes).map(|m| m.frames_per_second()), Some(15));
    assert_eq!(choose_mode(&[]), None);
}

#[test]
fn frame_rate_reads_the_interval_fraction() {
    let ntsc = CameraMode {
        encoding: PixelEncoding::Nv12,
        width: 1280,
        height: 720,
        interval_numerator: 1001,
        interval_denominator: 30000,
    };
    assert_eq!(ntsc.frames_per_second(), 29);
    let none = CameraMode { interval_numerator: 0, ..ntsc };
    assert_eq!(none.frames_per_second(), 0);
}

#[test]
fn front_prefers_the_front_then_built_in_then_first() {
    let devices = [camera("usb", CameraPosition::External, false), camera("lid", CameraPosition::Front, true)];
    assert_eq!(pick_device(&devices, true).map(|d| d.id.as_str()), Some("lid"));
    let devices = [camera("usb", CameraPosition::External, false), camera("lid", CameraPosition::Unspecified, true)];
    assert_eq!(pick_device(&devices, true).map(|d| d.id.as_str()), Some("lid"));
    let devices = [camera("usb", CameraPosition::External, false)];
    assert_eq!(pick_device(&devices, true).map(|d| d.id.as_str()), Some("usb"));
    assert_eq!(pick_device(&[], true), None);
}

#[test]
fn back_is_another_camera_and_one_camera_cannot_flip() {
    let devices = [camera("lid", CameraPosition::Front, true), camera("usb", CameraPosition::External, false)];
    assert_eq!(pick_device(&devices, false).map(|d| d.id.as_str()), Some("usb"));
    assert!(can_flip(&devices));
    let single = [camera("lid", CameraPosition::Front, true)];
    assert_eq!(pick_device(&single, false), None);
    assert!(!can_flip(&single));
}

#[test]
fn segments_are_mov_on_macos_and_mp4_elsewhere() {
    assert_eq!(segment_extension(HostOs::MacOs), "mov");
    assert_eq!(segment_extension(HostOs::Linux), "mp4");
    assert_eq!(segment_extension(HostOs::Windows), "mp4");
}

#[test]
fn the_plan_numbers_segments_and_sums_time() {
    let mut plan = SegmentPlan::new(PathBuf::from("take"), "note".to_owned(), HostOs::Linux);
    let first = plan.begin(Duration::from_secs(1), true).unwrap();
    assert_eq!(first.file_name().and_then(|n| n.to_str()), Some("note-0.mp4"));
    assert_eq!(plan.begin(Duration::from_secs(1), true), Err(CameraCaptureError::AlreadyRecording));
    assert_eq!(plan.elapsed(Duration::from_secs(3)), Duration::from_secs(2));
    plan.end(Duration::from_secs(4)).unwrap();
    assert_eq!(plan.end(Duration::from_secs(4)), Err(CameraCaptureError::NotRecording));
    let second = plan.begin(Duration::from_secs(10), false).unwrap();
    assert_eq!(second.file_name().and_then(|n| n.to_str()), Some("note-1.mp4"));
    assert_eq!(plan.elapsed(Duration::from_secs(11)), Duration::from_secs(4));
    assert_eq!(plan.all_paths().len(), 2);
    plan.end(Duration::from_secs(12)).unwrap();
    let segments = plan.segments();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].duration, Duration::from_secs(3));
    assert!(segments[0].front);
    assert!(!segments[1].front);
}

#[test]
fn an_abandoned_segment_is_not_counted() {
    let mut plan = SegmentPlan::new(PathBuf::from("take"), "note".to_owned(), HostOs::MacOs);
    let path = plan.begin(Duration::ZERO, true).unwrap();
    assert_eq!(plan.abandon(), Some(path));
    assert!(!plan.is_recording());
    assert!(plan.segments().is_empty());
    let next = plan.begin(Duration::ZERO, true).unwrap();
    assert_eq!(next.file_name().and_then(|n| n.to_str()), Some("note-1.mov"));
}

#[test]
fn yuyv_converts_black_white_and_respects_stride() {
    let source = [16, 128, 16, 128, 0, 0, 235, 128, 235, 128];
    let mut out = Vec::new();
    yuyv_to_bgra(&source, 2, 2, 6, &mut out).unwrap();
    assert_eq!(&out[0..8], &[0, 0, 0, 255, 0, 0, 0, 255]);
    assert_eq!(&out[8..16], &[255, 255, 255, 255, 255, 255, 255, 255]);
}

#[test]
fn yuyv_converts_red() {
    let mut out = Vec::new();
    yuyv_to_bgra(&[81, 90, 81, 240], 2, 1, 4, &mut out).unwrap();
    assert!(out[2] > 250 && out[1] < 5 && out[0] < 5, "{out:?}");
}

#[test]
fn yuyv_rejects_short_or_odd_frames() {
    let mut out = Vec::new();
    assert!(yuyv_to_bgra(&[0; 3], 2, 1, 4, &mut out).is_err());
    assert!(yuyv_to_bgra(&[0; 8], 3, 1, 6, &mut out).is_err());
    assert!(yuyv_to_bgra(&[0; 8], 2, 1, 2, &mut out).is_err());
}

#[test]
fn bgra_frame_checks_its_length() {
    assert!(bgra_frame(vec![0; 16], 2, 2, Duration::ZERO, true).is_ok());
    assert!(bgra_frame(vec![0; 15], 2, 2, Duration::ZERO, true).is_err());
}

#[test]
fn unsupported_answers_hide_the_button() {
    assert!(CameraCaptureError::NoCamera.is_unsupported());
    assert!(CameraCaptureError::TorchUnsupported.is_unsupported());
    assert!(CameraCaptureError::EncoderUnavailable("x".to_owned()).is_unsupported());
    assert!(!CameraCaptureError::Busy.is_unsupported());
    assert!(!CameraCaptureError::StreamFailed("x".to_owned()).is_unsupported());
}

#[test]
fn a_zoom_range_counts_only_when_the_device_zooms() {
    assert_eq!(usable_zoom_range(1.0, 4.0), Some((1.0, 4.0)));
    assert_eq!(usable_zoom_range(1.0, 1.0), None, "a webcam at one factor has no zoom");
    assert_eq!(usable_zoom_range(0.5, 2.0), None, "AVFoundation never goes below one factor");
    assert_eq!(usable_zoom_range(1.0, f32::INFINITY), None);
    assert_eq!(usable_zoom_range(f32::NAN, 2.0), None);
}

#[test]
fn a_zoom_factor_stays_inside_the_range() {
    assert_eq!(clamp_zoom_factor(0.2, (1.0, 4.0)), 1.0);
    assert_eq!(clamp_zoom_factor(2.5, (1.0, 4.0)), 2.5);
    assert_eq!(clamp_zoom_factor(9.0, (1.0, 4.0)), 4.0);
    assert_eq!(clamp_zoom_factor(f32::NAN, (1.0, 4.0)), 1.0);
}

#[test]
fn a_point_of_interest_stays_in_the_unit_square() {
    assert_eq!(clamp_point_of_interest(0.25, 0.75), (0.25, 0.75));
    assert_eq!(clamp_point_of_interest(-1.0, 2.0), (0.0, 1.0));
    assert_eq!(clamp_point_of_interest(f32::NAN, f32::INFINITY), (0.5, 0.5));
}

#[test]
fn missing_zoom_and_focus_are_unsupported_not_failures() {
    assert!(CameraCaptureError::ZoomUnsupported.is_unsupported());
    assert!(CameraCaptureError::PointOfInterestUnsupported.is_unsupported());
}

