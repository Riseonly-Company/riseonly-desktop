//! Video decode, and the choice of who does it.
//!
//! The decoder itself is FFmpeg (through rsmpeg, which exposes
//! `AVHWDeviceContext` where ffmpeg-next does not — without it there is no way
//! to ask for a hardware frame rather than a CPU one). That dependency is
//! behind the `ffmpeg` feature so the rest of the crate, and the whole test
//! suite, builds on a machine with no FFmpeg prefix.
//!
//! Everything in this file that makes a DECISION is pure and always compiled:
//! which accelerator a host uses, which codecs are accepted, what happens when
//! hardware decode is refused. Only the binding to libavcodec is optional.

use super::super::texture::frame::{FrameFormat, FrameGeometry};
use rise_platform::HostOs;

/// The codecs the client will decode.
///
/// Deliberately short. Each entry is a C parser fed bytes off a socket, and the
/// ones not listed here are the cheapest attack-surface reduction available.
/// This list must stay a subset of what `scripts/build-ffmpeg-lgpl.sh` enables.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum VideoCodec {
    H264,
    Hevc,
    Vp8,
    Vp9,
    Av1,
}

impl VideoCodec {
    pub const ALL: [Self; 5] = [Self::H264, Self::Hevc, Self::Vp8, Self::Vp9, Self::Av1];

    /// The name libavcodec knows it by.
    pub const fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::Vp8 => "vp8",
            Self::Vp9 => "vp9",
            Self::Av1 => "av1",
        }
    }
}

/// The hardware decode API used on a host.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HwAccel {
    VideoToolbox,
    D3D11Va,
    Vaapi,
    /// libavcodec's own software decoders. Correct everywhere, and the only
    /// option for a codec the GPU does not implement.
    None,
}

impl HwAccel {
    /// The `AVHWDeviceType` name rsmpeg is given.
    pub const fn ffmpeg_device_name(self) -> Option<&'static str> {
        match self {
            Self::VideoToolbox => Some("videotoolbox"),
            Self::D3D11Va => Some("d3d11va"),
            Self::Vaapi => Some("vaapi"),
            Self::None => None,
        }
    }

    /// The accelerator a host uses when the codec supports it.
    pub const fn for_host(host: HostOs) -> Self {
        match host {
            HostOs::MacOs => Self::VideoToolbox,
            HostOs::Windows => Self::D3D11Va,
            HostOs::Linux => Self::Vaapi,
        }
    }

    /// The frame format this accelerator hands back.
    ///
    /// All three return biplanar YUV, which is why NV12 is the format the
    /// texture path is built around rather than an intermediate everything is
    /// converted to.
    pub const fn output_format(self, bit_depth: u8) -> FrameFormat {
        match self {
            // Software decoders keep YUV planar at every bit depth. The
            // shader's triplanar path reads it directly.
            Self::None => FrameFormat::I420,
            _ => {
                if bit_depth > 8 {
                    FrameFormat::P010
                } else {
                    FrameFormat::Nv12
                }
            }
        }
    }
}

/// Whether a codec at a bit depth is decoded in hardware on a host.
///
/// The pessimistic entries are deliberate. Claiming hardware where it does not
/// exist means the decoder opens, fails on the first packet, and the video is
/// black — worse than starting in software and being smooth.
pub const fn hardware_support(host: HostOs, codec: VideoCodec, bit_depth: u8) -> HwAccel {
    let accel = HwAccel::for_host(host);

    match (host, codec) {
        // VP8 has no hardware decoder on any Apple silicon, and Intel Macs are
        // out of scope.
        (HostOs::MacOs, VideoCodec::Vp8) => HwAccel::None,
        // AV1 hardware decode arrived with M3; older Macs must decode it in
        // software, and this cannot be known until the device is opened, so the
        // conservative answer is the one that always works.
        (HostOs::MacOs, VideoCodec::Av1) => HwAccel::None,
        // VP8 predates the D3D11 video decode profiles that matter.
        (HostOs::Windows, VideoCodec::Vp8) => HwAccel::None,
        _ => {
            if bit_depth > 10 {
                // 12-bit profiles are software everywhere in practice.
                HwAccel::None
            } else {
                accel
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DecoderError {
    UnsupportedCodec(String),
    HardwareUnavailable {
        requested: HwAccel,
        reason: String,
    },
    InvalidGeometry(String),
    /// The `ffmpeg` feature is off. Returned rather than panicking so a build
    /// without FFmpeg still runs the whole app and simply shows poster frames.
    NotCompiledIn,
    Backend(String),
}

/// A decoded frame's description, plus a handle to where its pixels live.
///
/// The pixels themselves never pass through this type. On the zero-copy paths
/// they are already in GPU memory and `handle` identifies them; that is the
/// entire point.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DecodedFrame {
    pub geometry: FrameGeometry,
    pub presentation_timestamp_us: i64,
    pub handle: FrameHandle,
}

/// An opaque reference to decoder-owned GPU memory.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FrameHandle {
    /// A CVPixelBuffer's IOSurface id.
    IoSurface(u32),
    /// A DMA-BUF file descriptor exported by VAAPI.
    DmaBufFd(i32),
    /// A DXGI shared NT handle, as a pointer-sized value.
    DxgiHandle(usize),
    /// The frame is in CPU memory at this index into the decoder's own pool.
    HostBuffer(usize),
}

impl FrameHandle {
    pub const fn is_gpu_resident(self) -> bool {
        !matches!(self, Self::HostBuffer(_))
    }
}

/// What a decode session was asked for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DecoderRequest {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
}

/// The decision made before any FFmpeg call, so it can be tested without one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DecoderPlan {
    pub accel: HwAccel,
    pub format: FrameFormat,
    pub geometry: FrameGeometry,
}

impl DecoderPlan {
    pub fn resolve(host: HostOs, request: &DecoderRequest) -> Result<Self, DecoderError> {
        let accel = hardware_support(host, request.codec, request.bit_depth);
        let format = accel.output_format(request.bit_depth);

        let geometry = FrameGeometry::packed(format, request.width, request.height)
            .map_err(|error| DecoderError::InvalidGeometry(format!("{error:?}")))?;

        Ok(Self {
            accel,
            format,
            geometry,
        })
    }

    /// The same plan with hardware given up on, used when opening the hardware
    /// device fails at runtime — an old driver, a VM, a headless CI runner.
    pub fn degraded_to_software(&self, request: &DecoderRequest) -> Result<Self, DecoderError> {
        let format = HwAccel::None.output_format(request.bit_depth);
        let geometry = FrameGeometry::packed(format, request.width, request.height)
            .map_err(|error| DecoderError::InvalidGeometry(format!("{error:?}")))?;

        Ok(Self {
            accel: HwAccel::None,
            format,
            geometry,
        })
    }

    pub const fn is_hardware(&self) -> bool {
        !matches!(self.accel, HwAccel::None)
    }
}

/// A decode session.
///
/// A trait so the scheduler, the budget and the ceiling test can all run with
/// no FFmpeg present, and so rlottie or a future ThorVG-style backend can be
/// substituted without touching the callers.
pub trait VideoDecoder: Send {
    fn plan(&self) -> &DecoderPlan;

    /// Feed one compressed packet.
    fn send_packet(&mut self, packet: &[u8], timestamp_us: i64) -> Result<(), DecoderError>;

    /// Take the next decoded frame, if one is ready.
    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, DecoderError>;

    fn flush(&mut self) -> Result<(), DecoderError>;
}

/// Open a decoder for a request on this host.
///
/// Without the `ffmpeg` feature this reports `NotCompiledIn` rather than
/// panicking: a build with no FFmpeg prefix is a legitimate configuration and
/// should degrade to poster frames, not crash the feed.
pub fn open(request: &DecoderRequest) -> Result<Box<dyn VideoDecoder>, DecoderError> {
    let _plan = DecoderPlan::resolve(HostOs::current(), request)?;

    #[cfg(feature = "ffmpeg")]
    {
        super::ffmpeg_backend::open(_plan, request)
    }
    #[cfg(not(feature = "ffmpeg"))]
    {
        Err(DecoderError::NotCompiledIn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(codec: VideoCodec, bit_depth: u8) -> DecoderRequest {
        DecoderRequest {
            codec,
            width: 1920,
            height: 1080,
            bit_depth,
        }
    }

    #[test]
    fn each_host_reaches_for_its_own_accelerator() {
        assert_eq!(HwAccel::for_host(HostOs::MacOs), HwAccel::VideoToolbox);
        assert_eq!(HwAccel::for_host(HostOs::Windows), HwAccel::D3D11Va);
        assert_eq!(HwAccel::for_host(HostOs::Linux), HwAccel::Vaapi);
    }

    #[test]
    fn h264_is_hardware_decoded_on_every_host() {
        for host in HostOs::ALL {
            let plan = DecoderPlan::resolve(host, &request(VideoCodec::H264, 8)).unwrap();
            assert!(plan.is_hardware(), "{host:?} would decode h264 in software");
            assert_eq!(plan.format, FrameFormat::Nv12);
        }
    }

    #[test]
    fn ten_bit_hevc_yields_p010_on_hardware() {
        let plan = DecoderPlan::resolve(HostOs::MacOs, &request(VideoCodec::Hevc, 10)).unwrap();

        assert!(plan.is_hardware());
        assert_eq!(plan.format, FrameFormat::P010);
        assert_eq!(plan.geometry.byte_len(), 1920 * 1080 * 3);
    }

    #[test]
    fn twelve_bit_falls_back_to_software_everywhere() {
        for host in HostOs::ALL {
            let plan = DecoderPlan::resolve(host, &request(VideoCodec::Hevc, 12)).unwrap();
            assert_eq!(plan.accel, HwAccel::None, "{host:?}");
        }
    }

    #[test]
    fn vp8_is_never_claimed_as_hardware_on_macos_or_windows() {
        for host in [HostOs::MacOs, HostOs::Windows] {
            assert_eq!(
                hardware_support(host, VideoCodec::Vp8, 8),
                HwAccel::None,
                "{host:?}"
            );
        }
        assert_eq!(
            hardware_support(HostOs::Linux, VideoCodec::Vp8, 8),
            HwAccel::Vaapi
        );
    }

    #[test]
    fn av1_on_macos_is_software_because_it_depends_on_the_chip_generation() {
        assert_eq!(
            hardware_support(HostOs::MacOs, VideoCodec::Av1, 8),
            HwAccel::None
        );
        assert_eq!(
            hardware_support(HostOs::Windows, VideoCodec::Av1, 8),
            HwAccel::D3D11Va
        );
    }

    #[test]
    fn a_software_plan_uses_a_planar_format_the_shader_can_read() {
        let plan = DecoderPlan::resolve(HostOs::MacOs, &request(VideoCodec::Av1, 8)).unwrap();

        assert_eq!(plan.format, FrameFormat::I420);
        assert!(plan.format.needs_color_conversion());
    }

    #[test]
    fn degrading_to_software_keeps_the_frame_dimensions() {
        let requested = request(VideoCodec::H264, 8);
        let hardware = DecoderPlan::resolve(HostOs::MacOs, &requested).unwrap();
        let software = hardware.degraded_to_software(&requested).unwrap();

        assert!(!software.is_hardware());
        assert_eq!(software.geometry.width(), hardware.geometry.width());
        assert_eq!(software.geometry.height(), hardware.geometry.height());
    }

    #[test]
    fn an_odd_width_is_rejected_before_any_ffmpeg_call() {
        let error = DecoderPlan::resolve(
            HostOs::MacOs,
            &DecoderRequest {
                codec: VideoCodec::H264,
                width: 1921,
                height: 1080,
                bit_depth: 8,
            },
        )
        .unwrap_err();

        assert!(matches!(error, DecoderError::InvalidGeometry(_)));
    }

    #[test]
    fn every_codec_has_an_ffmpeg_name_and_they_are_distinct() {
        let names: Vec<_> = VideoCodec::ALL.iter().map(|c| c.ffmpeg_name()).collect();

        for name in &names {
            assert!(!name.is_empty());
            assert_eq!(names.iter().filter(|other| *other == name).count(), 1);
        }
    }

    #[test]
    fn only_software_decode_lacks_a_hardware_device_name() {
        assert_eq!(HwAccel::None.ffmpeg_device_name(), None);
        for accel in [HwAccel::VideoToolbox, HwAccel::D3D11Va, HwAccel::Vaapi] {
            assert!(accel.ffmpeg_device_name().is_some(), "{accel:?}");
        }
    }

    #[test]
    fn hardware_frames_are_gpu_resident_and_host_buffers_are_not() {
        assert!(FrameHandle::IoSurface(7).is_gpu_resident());
        assert!(FrameHandle::DmaBufFd(9).is_gpu_resident());
        assert!(FrameHandle::DxgiHandle(0x1234).is_gpu_resident());
        assert!(!FrameHandle::HostBuffer(0).is_gpu_resident());
    }

    #[test]
    fn opening_without_the_ffmpeg_feature_reports_it_rather_than_panicking() {
        if cfg!(not(feature = "ffmpeg")) {
            let opened = open(&request(VideoCodec::H264, 8));
            assert!(matches!(opened, Err(DecoderError::NotCompiledIn)));
        }
    }
}
