use crate::camera_capture::camera_capture::{CameraMode, CameraPosition, PixelEncoding, choose_mode};
use crate::camera_capture::media_foundation_video::*;

fn mode(width: u32, height: u32, frames: u32) -> CameraMode {
    CameraMode {
        encoding: PixelEncoding::Nv12,
        width,
        height,
        interval_numerator: 1,
        interval_denominator: frames,
    }
}

#[test]
fn a_packed_pair_keeps_the_first_value_in_the_high_half() {
    assert_eq!(pack_pair(1280, 720), 0x0000_0500_0000_02D0);
    assert_eq!(unpack_pair(pack_pair(1280, 720)), (1280, 720));
    assert_eq!(unpack_pair(pack_pair(u32::MAX, 0)), (u32::MAX, 0));
}

#[test]
fn a_native_type_becomes_seconds_per_frame() {
    let native = mode_from_native(PixelEncoding::Yuyv, pack_pair(640, 480), pack_pair(30_000, 1001))
        .expect("a sized native type");
    assert_eq!(native.width, 640);
    assert_eq!(native.height, 480);
    assert_eq!((native.interval_numerator, native.interval_denominator), (1001, 30_000));
    assert_eq!(native.frames_per_second(), 29);
    assert_eq!(packed_frame_rate(&native), pack_pair(30_000, 1001));
    assert_eq!(packed_frame_size(&native), pack_pair(640, 480));
}

#[test]
fn a_native_type_without_a_size_is_skipped() {
    assert_eq!(mode_from_native(PixelEncoding::Mjpeg, pack_pair(0, 720), pack_pair(30, 1)), None);
    assert_eq!(mode_from_native(PixelEncoding::Mjpeg, pack_pair(1280, 0), pack_pair(30, 1)), None);
}

#[test]
fn native_types_feed_the_shared_mode_choice() {
    let modes = [
        mode_from_native(PixelEncoding::Yuyv, pack_pair(1280, 720), pack_pair(10, 1)).unwrap(),
        mode_from_native(PixelEncoding::Mjpeg, pack_pair(1280, 720), pack_pair(30, 1)).unwrap(),
        mode_from_native(PixelEncoding::Yuyv, pack_pair(640, 480), pack_pair(30, 1)).unwrap(),
    ];
    let chosen = choose_mode(&modes).expect("a mode");
    assert_eq!(chosen, modes[1]);
}

#[test]
fn a_mode_without_an_interval_runs_at_thirty_frames() {
    let still = CameraMode { interval_numerator: 0, interval_denominator: 0, ..mode(1280, 720, 30) };
    assert_eq!(packed_frame_rate(&still), pack_pair(30, 1));
    assert_eq!(frame_duration_hns(&still), FALLBACK_FRAME_HNS);
    assert_eq!(frame_duration_hns(&mode(1280, 720, 30)), 333_333);
    assert_eq!(frame_duration_hns(&mode(1280, 720, 60)), 166_666);
}

#[test]
fn the_bitrate_follows_the_picture_within_bounds() {
    assert_eq!(h264_bitrate(&mode(1280, 720, 30)), 4_147_200);
    assert_eq!(h264_bitrate(&mode(320, 240, 15)), MIN_H264_BITRATE);
    assert_eq!(h264_bitrate(&mode(3840, 2160, 60)), MAX_H264_BITRATE);
}

#[test]
fn the_panel_names_the_direction_of_a_built_in_camera() {
    assert_eq!(placement(Some((EnclosurePanel::Front, false))), (CameraPosition::Front, true));
    assert_eq!(placement(Some((EnclosurePanel::Back, false))), (CameraPosition::Back, true));
    assert_eq!(placement(Some((EnclosurePanel::Top, false))), (CameraPosition::Unspecified, true));
    assert_eq!(placement(Some((EnclosurePanel::Unknown, false))), (CameraPosition::Unspecified, false));
    assert_eq!(placement(Some((EnclosurePanel::Front, true))), (CameraPosition::External, false));
    assert_eq!(placement(None), (CameraPosition::Unspecified, false));
}

#[test]
fn panel_values_follow_the_winrt_enumeration() {
    let panels: Vec<EnclosurePanel> = (0..8).map(EnclosurePanel::from_raw).collect();
    assert_eq!(
        panels,
        [
            EnclosurePanel::Unknown,
            EnclosurePanel::Front,
            EnclosurePanel::Back,
            EnclosurePanel::Top,
            EnclosurePanel::Bottom,
            EnclosurePanel::Left,
            EnclosurePanel::Right,
            EnclosurePanel::Unknown,
        ]
    );
}

#[test]
fn a_segment_starts_at_zero_and_never_steps_back() {
    let mut clock = SegmentClock::new(333_333);
    assert!(!clock.has_frames());
    assert_eq!(clock.place(50_000_000, 333_333), (0, 333_333));
    assert!(clock.has_frames());
    assert_eq!(clock.place(50_333_333, 0), (333_333, 333_333));
    assert_eq!(clock.place(50_333_333, 333_333), (333_334, 333_333));
    assert_eq!(clock.place(49_000_000, 333_333), (333_335, 333_333));
}

#[test]
fn a_clock_without_a_frame_length_falls_back_to_thirty_frames() {
    let mut clock = SegmentClock::new(0);
    assert_eq!(clock.place(7, 0), (0, FALLBACK_FRAME_HNS));
}
