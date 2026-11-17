use crate::camera_capture::camera_capture::*;
use crate::camera_capture::macos::*;

#[test]
fn capture_subtypes_map_to_the_hub_encodings() {
    assert_eq!(encoding_of(SUBTYPE_420V), Some(PixelEncoding::Nv12));
    assert_eq!(encoding_of(SUBTYPE_420F), Some(PixelEncoding::Nv12));
    assert_eq!(encoding_of(SUBTYPE_YUVS), Some(PixelEncoding::Yuyv));
    assert_eq!(encoding_of(SUBTYPE_BGRA), Some(PixelEncoding::Bgra));
    assert_eq!(encoding_of(SUBTYPE_DMB1), Some(PixelEncoding::Mjpeg));
    assert_eq!(encoding_of(SUBTYPE_JPEG), Some(PixelEncoding::Mjpeg));
    assert_eq!(encoding_of(u32::from_be_bytes(*b"2vuy")), None, "UYVY has no hub name and is not offered");
}

#[test]
fn four_character_codes_read_big_endian() {
    assert_eq!(SUBTYPE_BGRA, 0x4247_5241, "kCVPixelFormatType_32BGRA");
    assert_eq!(SUBTYPE_420V, 0x3432_3076, "kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange");
}

#[test]
fn a_built_in_camera_without_a_position_looks_at_the_user() {
    assert_eq!(position_of(POSITION_FRONT, false), CameraPosition::Front);
    assert_eq!(position_of(POSITION_BACK, false), CameraPosition::Back);
    assert_eq!(position_of(0, true), CameraPosition::Front);
    assert_eq!(position_of(0, false), CameraPosition::External);
}

#[test]
fn a_frame_rate_range_becomes_a_mode() {
    let mode = mode_of(PixelEncoding::Nv12, 1280, 720, 1, 30).unwrap();
    assert_eq!((mode.width, mode.height), (1280, 720));
    assert_eq!(mode.frames_per_second(), 30);

    let precise = mode_of(PixelEncoding::Yuyv, 640, 480, 1_000_000, 30_000_030).unwrap();
    assert_eq!(precise.frames_per_second(), 30);

    assert_eq!(mode_of(PixelEncoding::Nv12, 0, 720, 1, 30), None);
    assert_eq!(mode_of(PixelEncoding::Nv12, 1280, 720, 0, 30), None);
    assert_eq!(mode_of(PixelEncoding::Nv12, 1280, 720, 1, -30), None);
}

#[test]
fn the_hub_picks_a_mac_mode_at_thirty_frames_first() {
    let modes = [
        mode_of(PixelEncoding::Nv12, 1920, 1080, 1, 15).unwrap(),
        mode_of(PixelEncoding::Nv12, 1280, 720, 1, 30).unwrap(),
        mode_of(PixelEncoding::Yuyv, 1280, 720, 1, 30).unwrap(),
        mode_of(PixelEncoding::Nv12, 640, 480, 1, 30).unwrap(),
    ];
    let chosen = choose_mode(&modes).unwrap();
    assert_eq!((chosen.width, chosen.height, chosen.frames_per_second()), (1280, 720, 30));
}

#[test]
fn open_errors_tell_busy_refused_and_gone_apart() {
    assert_eq!(open_error(ERROR_DEVICE_IN_USE, "x".into()), CameraCaptureError::Busy);
    assert_eq!(open_error(ERROR_NOT_AUTHORIZED, "x".into()), CameraCaptureError::AccessDenied);
    assert_eq!(open_error(ERROR_DEVICE_NOT_CONNECTED, "x".into()), CameraCaptureError::NoCamera);
    assert_eq!(open_error(-11800, "unknown".into()), CameraCaptureError::OpenFailed("unknown".into()));
}

#[test]
fn an_unplugged_camera_is_no_camera_at_run_time() {
    assert_eq!(runtime_failure(ERROR_DEVICE_NOT_CONNECTED, "x".into()), CameraCaptureError::NoCamera);
    assert_eq!(runtime_failure(ERROR_DEVICE_IN_USE, "x".into()), CameraCaptureError::Busy);
    assert_eq!(
        runtime_failure(-11819, "media services were reset".into()),
        CameraCaptureError::StreamFailed("media services were reset".into())
    );
}

#[test]
fn a_segment_finished_with_an_error_still_counts_when_the_file_was_written() {
    assert_eq!(segment_outcome(None), Ok(()));
    assert_eq!(segment_outcome(Some(("disk full".into(), true))), Ok(()));
    assert_eq!(
        segment_outcome(Some(("write failed".into(), false))),
        Err(CameraCaptureError::FileFailed("write failed".into()))
    );
}

#[test]
fn padded_rows_are_packed() {
    let width = 2;
    let height = 2;
    let stride = 12;
    let mut bytes = vec![0u8; stride * height];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = index as u8;
    }
    let packed = pack_rows(&bytes, width, height, stride).unwrap();
    assert_eq!(packed.len(), width * 4 * height);
    assert_eq!(&packed[..8], &bytes[..8]);
    assert_eq!(&packed[8..], &bytes[12..20]);
}

#[test]
fn a_short_or_narrow_pixel_buffer_is_a_bad_frame() {
    assert!(matches!(pack_rows(&[0; 8], 2, 2, 8), Err(CameraCaptureError::BadFrame(_))));
    assert!(matches!(pack_rows(&[0; 64], 4, 2, 8), Err(CameraCaptureError::BadFrame(_))));
    assert!(matches!(pack_rows(&[0; 64], 0, 2, 8), Err(CameraCaptureError::BadFrame(_))));
}
