use super::camera_capture::{CameraMode, CameraPosition, PixelEncoding};

pub const HNS_PER_SECOND: i64 = 10_000_000;
const BITS_PER_PIXEL_FRAME: f64 = 0.15;
pub const MIN_H264_BITRATE: u32 = 1_000_000;
pub const MAX_H264_BITRATE: u32 = 12_000_000;
pub const FALLBACK_FRAME_HNS: i64 = HNS_PER_SECOND / 30;
pub const EMPTY_SEGMENT_GRACE_HNS: i64 = HNS_PER_SECOND;

/// two u32 values in one u64 attribute, first in the high half
pub fn pack_pair(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

/// the two halves of a packed u64 attribute, high first
pub fn unpack_pair(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

/// native media type of the camera as a hub mode
/// frame_size and frame_rate are the packed MF_MT_FRAME_SIZE and MF_MT_FRAME_RATE values
pub fn mode_from_native(encoding: PixelEncoding, frame_size: u64, frame_rate: u64) -> Option<CameraMode> {
    let (width, height) = unpack_pair(frame_size);
    let (frames, seconds) = unpack_pair(frame_rate);
    if width == 0 || height == 0 {
        return None;
    }
    Some(CameraMode {
        encoding,
        width,
        height,
        interval_numerator: seconds,
        interval_denominator: frames,
    })
}

/// frame rate of a mode packed as (frames, seconds)
/// 30/1 for a mode without an interval
pub fn packed_frame_rate(mode: &CameraMode) -> u64 {
    if mode.interval_numerator == 0 || mode.interval_denominator == 0 {
        return pack_pair(30, 1);
    }
    pack_pair(mode.interval_denominator, mode.interval_numerator)
}

/// frame size of a mode packed as (width, height)
pub fn packed_frame_size(mode: &CameraMode) -> u64 {
    pack_pair(mode.width, mode.height)
}

/// one frame of the mode in 100 ns units
pub fn frame_duration_hns(mode: &CameraMode) -> i64 {
    if mode.interval_numerator == 0 || mode.interval_denominator == 0 {
        return FALLBACK_FRAME_HNS;
    }
    let duration = HNS_PER_SECOND * i64::from(mode.interval_numerator) / i64::from(mode.interval_denominator);
    if duration > 0 { duration } else { FALLBACK_FRAME_HNS }
}

/// average bitrate of the h264 output type for a mode
pub fn h264_bitrate(mode: &CameraMode) -> u32 {
    let frames = match mode.frames_per_second() {
        0 => 30,
        rate => rate,
    };
    let bits = f64::from(mode.width) * f64::from(mode.height) * f64::from(frames) * BITS_PER_PIXEL_FRAME;
    (bits.round() as u64).clamp(u64::from(MIN_H264_BITRATE), u64::from(MAX_H264_BITRATE)) as u32
}

/// windows enclosure panel, by numeric value
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnclosurePanel {
    Unknown,
    Front,
    Back,
    Top,
    Bottom,
    Left,
    Right,
}

impl EnclosurePanel {
    pub fn from_raw(value: i32) -> Self {
        match value {
            1 => Self::Front,
            2 => Self::Back,
            3 => Self::Top,
            4 => Self::Bottom,
            5 => Self::Left,
            6 => Self::Right,
            _ => Self::Unknown,
        }
    }
}

/// where a camera looks and if it is built in, from its enclosure location
/// no enclosure location (usb webcam usually) means not built in and direction unknown
/// docking station ones arent built in either
pub fn placement(enclosure: Option<(EnclosurePanel, bool)>) -> (CameraPosition, bool) {
    let Some((panel, in_dock)) = enclosure else {
        return (CameraPosition::Unspecified, false);
    };
    if in_dock {
        return (CameraPosition::External, false);
    }
    match panel {
        EnclosurePanel::Front => (CameraPosition::Front, true),
        EnclosurePanel::Back => (CameraPosition::Back, true),
        EnclosurePanel::Top | EnclosurePanel::Bottom | EnclosurePanel::Left | EnclosurePanel::Right => {
            (CameraPosition::Unspecified, true)
        }
        EnclosurePanel::Unknown => (CameraPosition::Unspecified, false),
    }
}

/// sample times of one segment, first frame at zero and times never step back
/// a frame without a duration lasts one frame of the mode
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SegmentClock {
    origin: Option<i64>,
    last: Option<i64>,
    frame: i64,
}

impl SegmentClock {
    pub fn new(frame: i64) -> Self {
        Self { origin: None, last: None, frame: if frame > 0 { frame } else { FALLBACK_FRAME_HNS } }
    }

    /// (time, duration) the sink writer gets for a frame the source reader stamped
    /// duration 0 when the sample states none
    pub fn place(&mut self, timestamp: i64, duration: i64) -> (i64, i64) {
        let origin = *self.origin.get_or_insert(timestamp);
        let mut time = timestamp.saturating_sub(origin).max(0);
        if let Some(last) = self.last
            && time <= last
        {
            time = last + 1;
        }
        self.last = Some(time);
        (time, if duration > 0 { duration } else { self.frame })
    }

    /// true if a frame went into the segment
    pub fn has_frames(&self) -> bool {
        self.last.is_some()
    }
}
