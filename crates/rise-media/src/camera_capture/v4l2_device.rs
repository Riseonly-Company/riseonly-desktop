use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::camera_capture::camera_capture::{
    CameraMode, CameraPosition, PixelEncoding, TARGET_HEIGHT, TARGET_WIDTH,
};

const IOC_WRITE: u64 = 1;
const IOC_READ: u64 = 2;

pub const fn ioc(read: bool, write: bool, number: u8, size: usize) -> u64 {
    let direction = (if read { IOC_READ } else { 0 }) | (if write { IOC_WRITE } else { 0 });
    (direction << 30) | ((size as u64) << 16) | ((b'V' as u64) << 8) | number as u64
}

pub const fn fourcc(code: &[u8; 4]) -> u32 {
    (code[0] as u32) | ((code[1] as u32) << 8) | ((code[2] as u32) << 16) | ((code[3] as u32) << 24)
}

pub const PIX_FMT_YUYV: u32 = fourcc(b"YUYV");
pub const PIX_FMT_NV12: u32 = fourcc(b"NV12");
pub const PIX_FMT_MJPEG: u32 = fourcc(b"MJPG");
pub const PIX_FMT_JPEG: u32 = fourcc(b"JPEG");

pub const CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
pub const CAP_STREAMING: u32 = 0x0400_0000;
pub const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
pub const CAP_TIMEPERFRAME: u32 = 0x1000;
pub const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
pub const MEMORY_MMAP: u32 = 1;
pub const FIELD_NONE: u32 = 1;
pub const FRMSIZE_TYPE_DISCRETE: u32 = 1;
pub const FRMIVAL_TYPE_DISCRETE: u32 = 1;
pub const CID_CAMERA_ORIENTATION: u32 = 0x009a_0922;

/// encoding the preview converts for a v4l2 pixel format, none if it skips it
pub fn encoding_of(pixel_format: u32) -> Option<PixelEncoding> {
    match pixel_format {
        PIX_FMT_YUYV => Some(PixelEncoding::Yuyv),
        PIX_FMT_NV12 => Some(PixelEncoding::Nv12),
        PIX_FMT_MJPEG | PIX_FMT_JPEG => Some(PixelEncoding::Mjpeg),
        _ => None,
    }
}

pub fn pixel_format_of(encoding: PixelEncoding) -> Option<u32> {
    match encoding {
        PixelEncoding::Yuyv => Some(PIX_FMT_YUYV),
        PixelEncoding::Nv12 => Some(PIX_FMT_NV12),
        PixelEncoding::Mjpeg => Some(PIX_FMT_MJPEG),
        PixelEncoding::Bgra => None,
    }
}

pub fn node_capabilities(capabilities: u32, device_caps: u32) -> u32 {
    if capabilities & CAP_DEVICE_CAPS != 0 { device_caps } else { capabilities }
}

/// true if the node can stream with mmap buffers
pub fn is_streaming_camera(capabilities: u32, device_caps: u32) -> bool {
    let caps = node_capabilities(capabilities, device_caps);
    caps & CAP_VIDEO_CAPTURE != 0 && caps & CAP_STREAMING != 0
}

/// sizes to try on a camera that gives a range
/// target and common webcam sizes, snapped to range and step
pub fn stepwise_sizes(
    min: (u32, u32),
    max: (u32, u32),
    step: (u32, u32),
) -> Vec<(u32, u32)> {
    let snap = |value: u32, low: u32, high: u32, step: u32| -> u32 {
        let clamped = value.clamp(low, high.max(low));
        let step = step.max(1);
        low + (clamped - low) / step * step
    };
    let mut sizes = Vec::new();
    for (width, height) in [(TARGET_WIDTH, TARGET_HEIGHT), (640, 480), (1920, 1080), max] {
        let size = (snap(width, min.0, max.0, step.0), snap(height, min.1, max.1, step.1));
        if size.0 > 0 && size.1 > 0 && !sizes.contains(&size) {
            sizes.push(size);
        }
    }
    sizes
}

/// builds a mode from one enumerated size and interval
pub fn mode_of(
    encoding: PixelEncoding,
    width: u32,
    height: u32,
    numerator: u32,
    denominator: u32,
) -> CameraMode {
    CameraMode {
        encoding,
        width,
        height,
        interval_numerator: numerator,
        interval_denominator: denominator,
    }
}

/// interval for a stepwise camera, the min rate frame if it reaches wanted fps
/// else its fastest
pub fn stepwise_interval(min: (u32, u32), max: (u32, u32), wanted_fps: u32) -> (u32, u32) {
    let rate = |(numerator, denominator): (u32, u32)| {
        if numerator == 0 { 0 } else { denominator / numerator }
    };
    if rate(min) >= wanted_fps && rate(max) <= wanted_fps {
        (1, wanted_fps)
    } else {
        min
    }
}

/// where the camera looks, orientation control first (0 front 1 back 2 external)
/// else the usb port, fixed is built in so faces the user
/// removable is on a cable
pub fn position_of(orientation: Option<i32>, removable: Option<&str>) -> (CameraPosition, bool) {
    match orientation {
        Some(0) => return (CameraPosition::Front, true),
        Some(1) => return (CameraPosition::Back, true),
        Some(2) => return (CameraPosition::External, false),
        _ => {}
    }
    match removable.map(trimming_whitespaces_and_newlines) {
        Some("fixed") => (CameraPosition::Front, true),
        Some("removable") => (CameraPosition::External, false),
        _ => (CameraPosition::Unspecified, false),
    }
}

/// picker name from the sysfs name, else the card name from querycap
pub fn display_name(sysfs_name: Option<&str>, card: &[u8]) -> String {
    if let Some(name) = sysfs_name.map(trimming_whitespaces_and_newlines).filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    let end = card.iter().position(|byte| *byte == 0).unwrap_or(card.len());
    let card = String::from_utf8_lossy(&card[..end]);
    let card = trimming_whitespaces_and_newlines(&card);
    if card.is_empty() { "Camera".to_owned() } else { card.to_owned() }
}

/// number at the end of a videoN node name, used for a stable list order
pub fn node_number(name: &str) -> Option<u32> {
    name.strip_prefix("video")?.parse().ok()
}
