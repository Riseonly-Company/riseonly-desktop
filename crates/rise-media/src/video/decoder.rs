use super::super::texture::frame::{FrameFormat, FrameGeometry};
use super::super::texture::native_surface::RetainedSurface;
use rise_platform::HostOs;

use crate::decode_guard::decode_guard::{
    DecodeGuardError, check_frame_bytes, check_video_frame, check_video_packet, native_call,
};

/// codecs the client decodes, must stay a subset of scripts build-ffmpeg-lgpl.sh
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

    /// the name libavcodec knows it by
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

/// hardware decode api used on a host
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HwAccel {
    VideoToolbox,
    D3D11Va,
    Vaapi,
    None,
}

impl HwAccel {
    /// the hw device type name rsmpeg gets
    pub const fn ffmpeg_device_name(self) -> Option<&'static str> {
        match self {
            Self::VideoToolbox => Some("videotoolbox"),
            Self::D3D11Va => Some("d3d11va"),
            Self::Vaapi => Some("vaapi"),
            Self::None => None,
        }
    }

    /// the accelerator a host uses when the codec supports it
    pub const fn for_host(host: HostOs) -> Self {
        match host {
            HostOs::MacOs => Self::VideoToolbox,
            HostOs::Windows => Self::D3D11Va,
            HostOs::Linux => Self::Vaapi,
        }
    }

    /// frame format this accelerator hands back, biplanar in hardware and planar in software
    pub const fn output_format(self, bit_depth: u8) -> FrameFormat {
        match self {
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

/// true if a codec at a bit depth is decoded in hardware on a host, pessimistic when unknown
pub const fn hardware_support(host: HostOs, codec: VideoCodec, bit_depth: u8) -> HwAccel {
    let accel = HwAccel::for_host(host);

    match (host, codec) {
        (HostOs::MacOs, VideoCodec::Vp8) => HwAccel::None,
        (HostOs::MacOs, VideoCodec::Av1) => HwAccel::None,
        (HostOs::Windows, VideoCodec::Vp8) => HwAccel::None,
        _ => {
            if bit_depth > 10 {
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
    NotCompiledIn,
    Backend(String),
}

/// description of a decoded frame plus a handle to its pixels, pixels never go through this type
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DecodedFrame {
    pub geometry: FrameGeometry,
    pub presentation_timestamp_us: i64,
    pub handle: FrameHandle,
}

/// opaque reference to decoder owned gpu memory
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum FrameHandle {
    PixelBuffer(RetainedSurface),
    DmaBuf(RetainedSurface),
    D3d11Texture(RetainedSurface),
    HostBuffer(usize),
}

impl FrameHandle {
    pub const fn is_gpu_resident(&self) -> bool {
        !matches!(self, Self::HostBuffer(_))
    }
}

/// planes of the last software decoded 8 bit 4:2:0 picture
/// y is height rows of width bytes, u and v are half size rounded up
/// borrowed from the decoder so they live until it is fed again
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HostI420Planes<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a [u8],
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub stride_y: u32,
    pub stride_u: u32,
    pub stride_v: u32,
}

/// what a decode session was asked for
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DecoderRequest {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
}

/// the decision made before any ffmpeg call, so tests can run without one
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

    /// same plan with hardware given up, for when opening the device fails at runtime
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

pub trait VideoDecoder: Send {
    fn plan(&self) -> &DecoderPlan;

    /// feeds one compressed packet
    fn send_packet(&mut self, packet: &[u8], timestamp_us: i64) -> Result<(), DecoderError>;

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, DecoderError>;

    fn flush(&mut self) -> Result<(), DecoderError>;

    fn host_planes(&self) -> Option<HostI420Planes<'_>> {
        None
    }
}


fn backend_refusal(refusal: DecodeGuardError) -> DecoderError {
    DecoderError::Backend(refusal.to_string())
}

/// backend behind the decode guard, size limits on packets and frames
/// a panic in a native call counts as a failed decode, not a dead process
pub struct GuardedDecoder {
    inner: Box<dyn VideoDecoder>,
}

impl GuardedDecoder {
    pub fn new(inner: Box<dyn VideoDecoder>) -> Self {
        Self { inner }
    }
}

impl VideoDecoder for GuardedDecoder {
    fn plan(&self) -> &DecoderPlan {
        self.inner.plan()
    }

    fn send_packet(&mut self, packet: &[u8], timestamp_us: i64) -> Result<(), DecoderError> {
        check_video_packet(packet.len() as u64).map_err(backend_refusal)?;
        native_call(|| self.inner.send_packet(packet, timestamp_us)).map_err(backend_refusal)?
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, DecoderError> {
        let frame = native_call(|| self.inner.receive_frame()).map_err(backend_refusal)??;
        if let Some(frame) = &frame {
            check_video_frame(frame.geometry.width(), frame.geometry.height())
                .and_then(|()| check_frame_bytes(frame.geometry.byte_len()))
                .map_err(backend_refusal)?;
        }
        Ok(frame)
    }

    fn flush(&mut self) -> Result<(), DecoderError> {
        native_call(|| self.inner.flush()).map_err(backend_refusal)?
    }

    fn host_planes(&self) -> Option<HostI420Planes<'_>> {
        self.inner.host_planes()
    }
}

/// opens a decoder for a request, without the ffmpeg feature it says not compiled in
pub fn open(request: &DecoderRequest) -> Result<Box<dyn VideoDecoder>, DecoderError> {
    let _plan = DecoderPlan::resolve(HostOs::current(), request)?;
    check_video_frame(request.width, request.height)
        .map_err(|refusal| DecoderError::InvalidGeometry(refusal.to_string()))?;

    #[cfg(feature = "ffmpeg")]
    {
        let decoder = native_call(|| super::ffmpeg_backend::open(_plan, request))
            .map_err(backend_refusal)??;
        Ok(Box::new(GuardedDecoder::new(decoder)))
    }
    #[cfg(not(feature = "ffmpeg"))]
    {
        Err(DecoderError::NotCompiledIn)
    }
}

/// opens a software decoder whatever the hardware can do
/// caller reads every frame through host_planes, eg webm stickers joining color and alpha streams
/// without the ffmpeg feature it says not compiled in
pub fn open_software(request: &DecoderRequest) -> Result<Box<dyn VideoDecoder>, DecoderError> {
    let _plan = DecoderPlan::resolve(HostOs::current(), request)?.degraded_to_software(request)?;
    check_video_frame(request.width, request.height)
        .map_err(|refusal| DecoderError::InvalidGeometry(refusal.to_string()))?;

    #[cfg(feature = "ffmpeg")]
    {
        let decoder = native_call(|| super::ffmpeg_backend::open(_plan, request))
            .map_err(backend_refusal)??;
        Ok(Box::new(GuardedDecoder::new(decoder)))
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
        assert!(FrameHandle::PixelBuffer(RetainedSurface::new(7, ())).is_gpu_resident());
        assert!(FrameHandle::DmaBuf(RetainedSurface::new(9, ())).is_gpu_resident());
        assert!(FrameHandle::D3d11Texture(RetainedSurface::new(0x1234, ())).is_gpu_resident());
        assert!(!FrameHandle::HostBuffer(0).is_gpu_resident());
    }

    #[test]
    fn opening_without_the_ffmpeg_feature_reports_it_rather_than_panicking() {
        if cfg!(not(feature = "ffmpeg")) {
            let opened = open(&request(VideoCodec::H264, 8));
            assert!(matches!(opened, Err(DecoderError::NotCompiledIn)));
        }
    }

    #[test]
    fn a_software_open_without_the_ffmpeg_feature_reports_it_and_still_checks_the_geometry() {
        if cfg!(not(feature = "ffmpeg")) {
            let opened = open_software(&request(VideoCodec::Vp9, 8));
            assert!(matches!(opened, Err(DecoderError::NotCompiledIn)));
        }
        let odd = DecoderRequest {
            codec: VideoCodec::Vp9,
            width: 511,
            height: 512,
            bit_depth: 8,
        };
        assert!(matches!(
            open_software(&odd),
            Err(DecoderError::InvalidGeometry(_))
        ));
    }
}
