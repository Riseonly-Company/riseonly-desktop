use crate::camera_capture::camera_capture::{CameraPosition, PixelEncoding};
use crate::camera_capture::v4l2_device::*;

#[test]
fn request_numbers_match_videodev2_on_64_bit() {
    assert_eq!(ioc(true, false, 0, 104), 0x8068_5600);
    assert_eq!(ioc(true, true, 2, 64), 0xc040_5602);
    assert_eq!(ioc(true, true, 5, 208), 0xc0d0_5605);
    assert_eq!(ioc(true, true, 8, 20), 0xc014_5608);
    assert_eq!(ioc(true, true, 15, 88), 0xc058_560f);
    assert_eq!(ioc(false, true, 18, 4), 0x4004_5612);
    assert_eq!(ioc(true, true, 22, 204), 0xc0cc_5616);
    assert_eq!(ioc(true, true, 74, 44), 0xc02c_564a);
    assert_eq!(ioc(true, true, 75, 52), 0xc034_564b);
}

#[test]
fn fourcc_is_little_endian() {
    assert_eq!(PIX_FMT_YUYV, 0x5659_5559);
    assert_eq!(PIX_FMT_MJPEG, 0x4750_4a4d);
}

#[test]
fn formats_map_both_ways() {
    assert_eq!(encoding_of(PIX_FMT_YUYV), Some(PixelEncoding::Yuyv));
    assert_eq!(encoding_of(PIX_FMT_JPEG), Some(PixelEncoding::Mjpeg));
    assert_eq!(encoding_of(fourcc(b"H264")), None);
    assert_eq!(pixel_format_of(PixelEncoding::Mjpeg), Some(PIX_FMT_MJPEG));
    assert_eq!(pixel_format_of(PixelEncoding::Bgra), None);
}

#[test]
fn metadata_nodes_are_not_cameras() {
    let device = CAP_VIDEO_CAPTURE | CAP_STREAMING | CAP_DEVICE_CAPS | 0x0080_0000;
    assert!(is_streaming_camera(device, CAP_VIDEO_CAPTURE | CAP_STREAMING));
    assert!(!is_streaming_camera(device, 0x0080_0000 | CAP_STREAMING));
    assert!(is_streaming_camera(CAP_VIDEO_CAPTURE | CAP_STREAMING, 0));
}

#[test]
fn stepwise_sizes_snap_into_the_range() {
    let sizes = stepwise_sizes((160, 120), (1280, 960), (16, 8));
    assert_eq!(sizes, vec![(1280, 720), (640, 480), (1280, 960)]);
    let tiny = stepwise_sizes((100, 100), (100, 100), (0, 0));
    assert_eq!(tiny, vec![(100, 100)]);
}

#[test]
fn stepwise_interval_prefers_the_minimum_rate() {
    assert_eq!(stepwise_interval((1, 60), (1, 5), 30), (1, 30));
    assert_eq!(stepwise_interval((1, 15), (1, 5), 30), (1, 15));
}

#[test]
fn position_reads_orientation_then_the_usb_port() {
    assert_eq!(position_of(Some(0), Some("removable")), (CameraPosition::Front, true));
    assert_eq!(position_of(Some(1), None), (CameraPosition::Back, true));
    assert_eq!(position_of(Some(2), Some("fixed")), (CameraPosition::External, false));
    assert_eq!(position_of(None, Some("fixed\n")), (CameraPosition::Front, true));
    assert_eq!(position_of(None, Some("removable\n")), (CameraPosition::External, false));
    assert_eq!(position_of(None, Some("unknown\n")), (CameraPosition::Unspecified, false));
    assert_eq!(position_of(None, None), (CameraPosition::Unspecified, false));
}

#[test]
fn names_come_from_sysfs_then_the_card() {
    assert_eq!(display_name(Some("Integrated Camera: Integrated C\n"), b"x\0"), "Integrated Camera: Integrated C");
    assert_eq!(display_name(Some("\n"), b"UVC Camera\0\0\0"), "UVC Camera");
    assert_eq!(display_name(None, b"\0"), "Camera");
}

#[test]
fn node_numbers_order_the_list() {
    assert_eq!(node_number("video2"), Some(2));
    assert_eq!(node_number("vbi0"), None);
}
