use std::time::Duration;

use crate::camera_capture::camera_capture::{bgra_frame, CameraCaptureError, CameraFrame};
use crate::camera_capture::still_frame::still_rgba_of;
use crate::texture::frame::{FrameFormat, FrameGeometry};

fn yuv_frame(format: FrameFormat, pixels: Vec<u8>, mirrored: bool) -> CameraFrame {
    let geometry = FrameGeometry::packed(format, 2, 2).expect("2x2 geometry");
    CameraFrame { geometry, pixels: pixels.into(), captured_at: Duration::ZERO, mirrored }
}

#[test]
fn bgra_becomes_rgba() {
    let frame = bgra_frame(vec![10, 20, 30, 255, 40, 50, 60, 128], 2, 1, Duration::ZERO, false).expect("frame");
    let image = still_rgba_of(&frame).expect("still");
    assert_eq!(image.dimensions(), (2, 1));
    assert_eq!(image.get_pixel(0, 0).0, [30, 20, 10, 255]);
    assert_eq!(image.get_pixel(1, 0).0, [60, 50, 40, 128]);
}

#[test]
fn mirrored_frame_gives_mirrored_still() {
    let frame = bgra_frame(vec![10, 20, 30, 255, 40, 50, 60, 255], 2, 1, Duration::ZERO, true).expect("frame");
    let image = still_rgba_of(&frame).expect("still");
    assert_eq!(image.get_pixel(0, 0).0, [60, 50, 40, 255]);
    assert_eq!(image.get_pixel(1, 0).0, [30, 20, 10, 255]);
}

#[test]
fn nv12_limited_range_black_and_white() {
    let frame = yuv_frame(FrameFormat::Nv12, vec![16, 235, 16, 235, 128, 128], false);
    let image = still_rgba_of(&frame).expect("still");
    assert_eq!(image.get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(image.get_pixel(1, 1).0, [255, 255, 255, 255]);
}

#[test]
fn i420_reads_three_planes() {
    let frame = yuv_frame(FrameFormat::I420, vec![235, 235, 235, 235, 128, 128], false);
    let image = still_rgba_of(&frame).expect("still");
    assert!(image.pixels().all(|pixel| pixel.0 == [255, 255, 255, 255]));
}

#[test]
fn p010_uses_the_high_byte() {
    let luma = [0x00, 235, 0x00, 235, 0x00, 16, 0x00, 16];
    let chroma = [0x00, 128, 0x00, 128];
    let frame = yuv_frame(FrameFormat::P010, [&luma[..], &chroma[..]].concat(), false);
    let image = still_rgba_of(&frame).expect("still");
    assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255, 255]);
    assert_eq!(image.get_pixel(0, 1).0, [0, 0, 0, 255]);
}

#[test]
fn short_frame_is_refused() {
    let frame = yuv_frame(FrameFormat::Nv12, vec![16, 16], false);
    assert!(matches!(still_rgba_of(&frame), Err(CameraCaptureError::BadFrame(_))));
}
