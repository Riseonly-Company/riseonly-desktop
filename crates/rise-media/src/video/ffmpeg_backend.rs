use std::ffi::{CStr, CString};

use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avutil::AVHWDeviceContext;
use rsmpeg::ffi;

use super::decoder::{
    DecodedFrame, DecoderError, DecoderPlan, DecoderRequest, FrameHandle, HostI420Planes, HwAccel,
    VideoDecoder,
};
use super::d3d11_frame::D3d11FrameRef;
use super::drm_prime::{DmaBufFrame, DmaBufLayout, DrmFrameDescriptor};
use crate::texture::color::ColorDescription;
use crate::texture::frame::{FrameFormat, FrameGeometry};
use crate::texture::native_surface::{RetainedSurface, retain_decoder_surface, ycbcr_to_rgb_columns};

pub fn open(
    plan: DecoderPlan,
    request: &DecoderRequest,
) -> Result<Box<dyn VideoDecoder>, DecoderError> {
    if !plan.is_hardware() {
        return open_software_plan(plan, request);
    }
    match FfmpegDecoder::open(plan.clone(), request) {
        Ok(decoder) => Ok(Box::new(decoder)),
        Err(DecoderError::HardwareUnavailable { .. } | DecoderError::UnsupportedCodec(_)) => {
            let software = plan.degraded_to_software(request)?;
            open_software_plan(software, request)
        }
        Err(error) => Err(error),
    }
}

/// a software plan decodes in the shipped libavcodec itself where the host can reach it apart
/// from the ffmpeg copy linked into libwebrtc, which only knows h264
fn open_software_plan(plan: DecoderPlan, request: &DecoderRequest) -> Result<Box<dyn VideoDecoder>, DecoderError> {
    #[cfg(target_os = "macos")]
    if let Some(decoder) = isolated::IsolatedDecoder::open(plan.clone(), request) {
        return decoder.map(|decoder| Box::new(decoder) as Box<dyn VideoDecoder>);
    }
    Ok(Box::new(FfmpegDecoder::open(plan, request)?))
}

struct FfmpegDecoder {
    plan: DecoderPlan,
    context: AVCodecContext,
    _hw_device: Option<AVHWDeviceContext>,
    frame_index: usize,
    last_host_frame: Option<rsmpeg::avutil::AVFrame>,
}

impl FfmpegDecoder {
    fn open(plan: DecoderPlan, request: &DecoderRequest) -> Result<Self, DecoderError> {
        let name = CString::new(request.codec.ffmpeg_name())
            .map_err(|_| DecoderError::UnsupportedCodec(request.codec.ffmpeg_name().into()))?;

        let codec = AVCodec::find_decoder_by_name(&name)
            .ok_or_else(|| DecoderError::UnsupportedCodec(request.codec.ffmpeg_name().into()))?;

        let mut context = AVCodecContext::new(&codec);
        context.set_width(request.width as i32);
        context.set_height(request.height as i32);
        crate::decode_guard::decode_guard::limit_codec_pixels(&mut context);

        let hw_device = match plan.accel.ffmpeg_device_name() {
            None => None,
            Some(device_name) => {
                let mut device = open_hw_device(plan.accel, device_name)?;
                unsafe {
                    (*context.as_mut_ptr()).hw_device_ctx = ffi::av_buffer_ref(device.as_mut_ptr());
                }
                Some(device)
            }
        };

        context
            .open(None)
            .map_err(|error| DecoderError::Backend(format!("avcodec_open2: {error}")))?;

        Ok(Self {
            plan,
            context,
            _hw_device: hw_device,
            frame_index: 0,
            last_host_frame: None,
        })
    }
}

fn open_hw_device(accel: HwAccel, device_name: &str) -> Result<AVHWDeviceContext, DecoderError> {
    let name = CString::new(device_name).map_err(|_| DecoderError::HardwareUnavailable {
        requested: accel,
        reason: "device name contained a nul".into(),
    })?;

    let device_type = unsafe { ffi::av_hwdevice_find_type_by_name(name.as_ptr()) };
    if device_type == ffi::AV_HWDEVICE_TYPE_NONE {
        return Err(DecoderError::HardwareUnavailable {
            requested: accel,
            reason: format!("this FFmpeg build has no {device_name} support"),
        });
    }

    AVHWDeviceContext::create(device_type, None, None, 0).map_err(|error| {
        DecoderError::HardwareUnavailable {
            requested: accel,
            reason: format!("{error}"),
        }
    })
}

impl VideoDecoder for FfmpegDecoder {
    fn plan(&self) -> &DecoderPlan {
        &self.plan
    }

    fn send_packet(&mut self, packet: &[u8], timestamp_us: i64) -> Result<(), DecoderError> {
        if packet.len() > i32::MAX as usize {
            return Err(DecoderError::Backend(
                "packet larger than AVPacket can hold".into(),
            ));
        }

        let mut av_packet = rsmpeg::avcodec::AVPacket::new();

        unsafe {
            if ffi::av_new_packet(av_packet.as_mut_ptr(), packet.len() as i32) < 0 {
                return Err(DecoderError::Backend("av_new_packet failed".into()));
            }

            let destination = (*av_packet.as_mut_ptr()).data;
            std::ptr::copy_nonoverlapping(packet.as_ptr(), destination, packet.len());
        }
        av_packet.set_pts(timestamp_us);
        av_packet.set_dts(timestamp_us);

        self.context
            .send_packet(Some(&av_packet))
            .map_err(|error| DecoderError::Backend(format!("avcodec_send_packet: {error}")))?;

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, DecoderError> {
        let frame = match self.context.receive_frame() {
            Ok(frame) => frame,
            Err(rsmpeg::error::RsmpegError::DecoderDrainError)
            | Err(rsmpeg::error::RsmpegError::DecoderFlushedError) => return Ok(None),
            Err(error) => {
                return Err(DecoderError::Backend(format!(
                    "avcodec_receive_frame: {error}"
                )));
            }
        };

        let (frame, handle, format) = match self.frame_handle(&frame) {
            Ok(handle) => (frame, handle, self.plan.format),
            Err(error) if takes_cpu_path(&error) => match transfer_to_host_i420(&frame) {
                Ok(host) => (host, FrameHandle::HostBuffer(self.frame_index), FrameFormat::I420),
                Err(copy_failure) => return Err(with_copy_failure(error, &copy_failure)),
            },
            Err(error) => return Err(error),
        };

        let width = frame.width as u32;
        let height = frame.height as u32;

        let strides: Vec<u32> = frame
            .linesize
            .iter()
            .take(format.plane_count())
            .map(|stride| (*stride).unsigned_abs())
            .collect();

        let geometry = FrameGeometry::with_strides(format, width, height, &strides)
            .map_err(|error| DecoderError::InvalidGeometry(format!("{error:?}")))?;

        self.frame_index = self.frame_index.wrapping_add(1);
        let presentation_timestamp_us = frame.pts;
        self.last_host_frame = (!handle.is_gpu_resident()).then_some(frame);

        Ok(Some(DecodedFrame {
            geometry,
            presentation_timestamp_us,
            handle,
        }))
    }

    fn flush(&mut self) -> Result<(), DecoderError> {
        self.context
            .send_packet(None)
            .map_err(|error| DecoderError::Backend(format!("flush: {error}")))
    }

    fn host_planes(&self) -> Option<HostI420Planes<'_>> {
        let frame = self.last_host_frame.as_ref()?;
        if frame.format != ffi::AV_PIX_FMT_YUV420P {
            return None;
        }

        let width = u32::try_from(frame.width).ok().filter(|width| *width > 0)?;
        let height = u32::try_from(frame.height).ok().filter(|height| *height > 0)?;
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));

        let (y, stride_y) = host_plane(frame, 0, width, height)?;
        let (u, stride_u) = host_plane(frame, 1, chroma_width, chroma_height)?;
        let (v, stride_v) = host_plane(frame, 2, chroma_width, chroma_height)?;

        Some(HostI420Planes {
            width,
            height,
            y,
            u,
            v,
            stride_y,
            stride_u,
            stride_v,
        })
    }
}

impl FfmpegDecoder {
    fn frame_handle(&self, frame: &rsmpeg::avutil::AVFrame) -> Result<FrameHandle, DecoderError> {
        if !self.plan.is_hardware() {
            return Ok(FrameHandle::HostBuffer(self.frame_index));
        }

        match self.plan.accel {
            HwAccel::VideoToolbox => {
                let held = unsafe { retain_decoder_surface(frame.data[3].cast()) }.map_err(|error| {
                    DecoderError::HardwareUnavailable {
                        requested: HwAccel::VideoToolbox,
                        reason: error.to_string(),
                    }
                })?;
                Ok(FrameHandle::PixelBuffer(held))
            }
            HwAccel::D3D11Va => retain_d3d11_frame(frame).map(FrameHandle::D3d11Texture),
            HwAccel::Vaapi => map_vaapi_to_drm_prime(frame).map(FrameHandle::DmaBuf),
            HwAccel::None => Ok(FrameHandle::HostBuffer(self.frame_index)),
        }
    }
}

/// one plane of a planar software frame, rows of at least row bytes and their stride
fn host_plane(
    frame: &rsmpeg::avutil::AVFrame,
    index: usize,
    row: u32,
    rows: u32,
) -> Option<(&[u8], u32)> {
    let stride = u32::try_from(frame.linesize[index]).ok().filter(|stride| *stride >= row)?;
    let pointer = frame.data[index];
    if pointer.is_null() || rows == 0 {
        return None;
    }
    let len = plane_len(stride, row, rows)?;
    Some((unsafe { std::slice::from_raw_parts(pointer, len) }, stride))
}

/// true when a frame_handle error is a hw picture the platform branch wouldnt hold
/// cpu path can still copy it
fn takes_cpu_path(error: &DecoderError) -> bool {
    matches!(error, DecoderError::HardwareUnavailable { requested, .. } if *requested != HwAccel::None)
}

/// the hold error plus the reason the cpu copy failed too, so no cause is lost
fn with_copy_failure(error: DecoderError, copy_failure: &str) -> DecoderError {
    match error {
        DecoderError::HardwareUnavailable { requested, reason } => DecoderError::HardwareUnavailable {
            requested,
            reason: format!("{reason}; the CPU copy failed too: {copy_failure}"),
        },
        other => other,
    }
}

fn transfer_to_host_i420(
    frame: &rsmpeg::avutil::AVFrame,
) -> Result<rsmpeg::avutil::AVFrame, String> {
    if frame.hw_frames_ctx.is_null() {
        return Err("the picture carries no hw_frames_ctx to copy from".into());
    }

    let mut host = rsmpeg::avutil::AVFrame::new();
    host.hwframe_transfer_data(frame)
        .map_err(|error| format!("av_hwframe_transfer_data: {error}"))?;
    host.set_pts(frame.pts);

    if host.format == ffi::AV_PIX_FMT_YUV420P {
        Ok(host)
    } else if host.format == ffi::AV_PIX_FMT_NV12 {
        i420_from_nv12(&host)
    } else {
        Err(format!(
            "the surface's software format {} is not 8-bit 4:2:0",
            host.format
        ))
    }
}

fn i420_from_nv12(nv12: &rsmpeg::avutil::AVFrame) -> Result<rsmpeg::avutil::AVFrame, String> {
    let width = u32::try_from(nv12.width).ok().filter(|width| *width > 0);
    let height = u32::try_from(nv12.height).ok().filter(|height| *height > 0);
    let (Some(width), Some(height)) = (width, height) else {
        return Err(format!("the copied picture is {}x{}", nv12.width, nv12.height));
    };
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));

    let unreadable = |plane: &str| format!("the copied NV12 picture has no readable {plane} plane");
    let (y, stride_y) = host_plane(nv12, 0, width, height).ok_or_else(|| unreadable("Y"))?;
    let (uv, stride_uv) =
        host_plane(nv12, 1, chroma_width * 2, chroma_height).ok_or_else(|| unreadable("UV"))?;

    let mut i420 = rsmpeg::avutil::AVFrame::new();
    i420.set_format(ffi::AV_PIX_FMT_YUV420P);
    i420.set_width(nv12.width);
    i420.set_height(nv12.height);
    i420.set_pts(nv12.pts);
    i420.alloc_buffer()
        .map_err(|error| format!("av_frame_get_buffer: {error}"))?;

    let planes = [(width, height), (chroma_width, chroma_height), (chroma_width, chroma_height)];
    let mut strides = [0u32; 3];
    let mut lens = [0usize; 3];
    for (index, (row, rows)) in planes.into_iter().enumerate() {
        let stride = u32::try_from(i420.linesize[index])
            .ok()
            .filter(|stride| *stride >= row)
            .ok_or_else(|| format!("av_frame_get_buffer gave plane {index} a short stride"))?;
        if i420.data[index].is_null() {
            return Err(format!("av_frame_get_buffer left plane {index} empty"));
        }
        strides[index] = stride;
        lens[index] = plane_len(stride, row, rows)
            .ok_or_else(|| format!("plane {index} does not fit in memory"))?;
    }

    let (out_y, out_u, out_v) = unsafe {
        (
            std::slice::from_raw_parts_mut(i420.data[0], lens[0]),
            std::slice::from_raw_parts_mut(i420.data[1], lens[1]),
            std::slice::from_raw_parts_mut(i420.data[2], lens[2]),
        )
    };
    copy_rows(y, stride_y, out_y, strides[0], width, height);
    split_chroma(
        uv,
        stride_uv,
        (out_u, strides[1]),
        (out_v, strides[2]),
        chroma_width,
        chroma_height,
    );

    Ok(i420)
}

/// bytes a plane takes with rows apart by stride when the last row is row bytes
fn plane_len(stride: u32, row: u32, rows: u32) -> Option<usize> {
    if rows == 0 {
        return Some(0);
    }
    usize::try_from(u64::from(stride) * u64::from(rows - 1) + u64::from(row)).ok()
}

/// copies rows between planes that have their own strides
fn copy_rows(
    source: &[u8],
    source_stride: u32,
    target: &mut [u8],
    target_stride: u32,
    row: u32,
    rows: u32,
) {
    let row = row as usize;
    for line in 0..rows as usize {
        let from = line * source_stride as usize;
        let to = line * target_stride as usize;
        target[to..to + row].copy_from_slice(&source[from..from + row]);
    }
}

/// splits interleaved nv12 chroma into separate u and v planes
fn split_chroma(
    uv: &[u8],
    uv_stride: u32,
    (u, u_stride): (&mut [u8], u32),
    (v, v_stride): (&mut [u8], u32),
    width: u32,
    rows: u32,
) {
    let width = width as usize;
    for line in 0..rows as usize {
        let source = &uv[line * uv_stride as usize..][..width * 2];
        let u_row = &mut u[line * u_stride as usize..][..width];
        let v_row = &mut v[line * v_stride as usize..][..width];
        for (sample, pair) in source.chunks_exact(2).enumerate() {
            u_row[sample] = pair[0];
            v_row[sample] = pair[1];
        }
    }
}

fn retain_d3d11_frame(
    frame: &rsmpeg::avutil::AVFrame,
) -> Result<RetainedSurface, DecoderError> {
    let unavailable = |reason: String| DecoderError::HardwareUnavailable {
        requested: HwAccel::D3D11Va,
        reason,
    };
    if frame.format != ffi::AV_PIX_FMT_D3D11 {
        return Err(unavailable(format!(
            "the decoder returned pixel format {} instead of a D3D11 texture",
            frame.format
        )));
    }
    let height = u32::try_from(frame.height).unwrap_or(0);
    let conversion = ColorDescription::resolve(None, None, height).params(FrameFormat::Nv12);
    let reference = D3d11FrameRef::from_frame_planes(
        frame.data[0],
        frame.data[1],
        frame.width,
        frame.height,
        ycbcr_to_rgb_columns(&conversion),
    )
    .map_err(|error| unavailable(error.to_string()))?;
    unsafe { retain_decoder_surface((&reference as *const D3d11FrameRef).cast_mut().cast()) }
        .map_err(|error| unavailable(error.to_string()))
}

fn map_vaapi_to_drm_prime(frame: &rsmpeg::avutil::AVFrame) -> Result<RetainedSurface, DecoderError> {
    let unavailable = |reason: String| DecoderError::HardwareUnavailable {
        requested: HwAccel::Vaapi,
        reason,
    };

    if frame.format != ffi::AV_PIX_FMT_VAAPI {
        return Err(unavailable(format!(
            "the decoder returned pixel format {} instead of a VAAPI surface",
            frame.format
        )));
    }

    let mut mapped = rsmpeg::avutil::AVFrame::new();
    mapped.set_format(ffi::AV_PIX_FMT_DRM_PRIME);

    let status = unsafe {
        ffi::av_hwframe_map(
            mapped.as_mut_ptr(),
            frame.as_ptr(),
            ffi::AV_HWFRAME_MAP_READ as i32,
        )
    };
    if status < 0 {
        return Err(unavailable(format!(
            "av_hwframe_map to DRM_PRIME failed with {status} (no libdrm in this FFmpeg build, or the driver cannot export)"
        )));
    }

    let address = mapped.data[0] as *const DrmFrameDescriptor;
    if address.is_null() {
        return Err(unavailable("the DRM_PRIME map carried no descriptor".into()));
    }

    let descriptor = unsafe { &*address };
    let layout = DmaBufLayout::from_descriptor(descriptor, frame.width as u32, frame.height as u32)
        .map_err(|error| unavailable(error.to_string()))?;

    Ok(RetainedSurface::new(
        address as usize,
        DmaBufFrame::new(layout, mapped),
    ))
}

pub fn linked_configuration() -> String {
    unsafe { CStr::from_ptr(ffi::avcodec_configuration()) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linked_ffmpeg_is_not_gpl() {
        let configuration = linked_configuration();

        for forbidden in ["--enable-gpl", "--enable-nonfree", "libx264", "libx265"] {
            assert!(
                !configuration.contains(forbidden),
                "linked FFmpeg carries {forbidden}, which relicenses this product: {configuration}"
            );
        }
    }

    #[test]
    fn a_refused_hardware_hold_goes_down_the_cpu_path_and_nothing_else_does() {
        for accel in [HwAccel::VideoToolbox, HwAccel::D3D11Va, HwAccel::Vaapi] {
            let refused = DecoderError::HardwareUnavailable { requested: accel, reason: "retain".into() };
            assert!(takes_cpu_path(&refused), "{accel:?}");
        }
        let software = DecoderError::HardwareUnavailable { requested: HwAccel::None, reason: "x".into() };
        assert!(!takes_cpu_path(&software));
        assert!(!takes_cpu_path(&DecoderError::Backend("avcodec_receive_frame".into())));
        assert!(!takes_cpu_path(&DecoderError::InvalidGeometry("odd".into())));
    }

    #[test]
    fn a_failed_copy_keeps_both_reasons() {
        let refused = DecoderError::HardwareUnavailable {
            requested: HwAccel::Vaapi,
            reason: "no libdrm".into(),
        };
        assert_eq!(
            with_copy_failure(refused, "P010 surface"),
            DecoderError::HardwareUnavailable {
                requested: HwAccel::Vaapi,
                reason: "no libdrm; the CPU copy failed too: P010 surface".into(),
            }
        );
        let other = DecoderError::Backend("b".into());
        assert_eq!(with_copy_failure(other.clone(), "c"), other);
    }

    #[test]
    fn nv12_chroma_splits_into_u_and_v_rows_with_their_own_strides() {
        let uv = [1, 2, 3, 4, 5, 6, 0xEE, 0xEE, 7, 8, 9, 10, 11, 12];
        let mut u = [0u8; 4 + 3];
        let mut v = [0u8; 3 + 3];
        split_chroma(&uv, 8, (&mut u, 4), (&mut v, 3), 3, 2);
        assert_eq!(u, [1, 3, 5, 0, 7, 9, 11]);
        assert_eq!(v, [2, 4, 6, 8, 10, 12]);
    }

    #[test]
    fn luma_rows_copy_without_the_source_padding() {
        let source = [1, 2, 3, 0xEE, 4, 5, 6];
        let mut target = [0u8; 6];
        copy_rows(&source, 4, &mut target, 3, 3, 2);
        assert_eq!(target, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn a_plane_ends_at_its_last_row_not_at_a_full_stride() {
        assert_eq!(plane_len(8, 5, 3), Some(21));
        assert_eq!(plane_len(8, 5, 0), Some(0));
        assert_eq!(plane_len(4, 4, 1), Some(4));
    }

    #[test]
    fn every_codec_this_client_accepts_is_present_in_the_linked_build() {
        for codec in super::super::decoder::VideoCodec::ALL {
            let name = CString::new(codec.ffmpeg_name()).unwrap();
            assert!(
                AVCodec::find_decoder_by_name(&name).is_some(),
                "the LGPL build has no {} decoder",
                codec.ffmpeg_name()
            );
        }
    }
}

/// software decoding through the libavcodec dylib the app ships, called by address
/// invariant: every call of one decoder goes to that one image, libwebrtc links its own
/// h264 only ffmpeg under the same symbol names and static linking binds rsmpeg to it
#[cfg(target_os = "macos")]
mod isolated {
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::sync::OnceLock;

    use rsmpeg::ffi;

    use super::plane_len;
    use crate::decode_guard::decode_guard::{VIDEO_FRAME_BYTES_KEY, client_limit};
    use crate::texture::frame::{FrameFormat, FrameGeometry};
    use crate::video::decoder::{
        DecodedFrame, DecoderError, DecoderPlan, DecoderRequest, FrameHandle, HostI420Planes, VideoDecoder,
    };

    const RTLD_LAZY: c_int = 0x1;
    const RTLD_LOCAL: c_int = 0x4;
    const RTLD_FIRST: c_int = 0x100;
    const AVERROR_EAGAIN: c_int = -35;
    const AVERROR_EOF: c_int = -0x2046_4F45;

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn _dyld_image_count() -> u32;
        fn _dyld_get_image_name(index: u32) -> *const c_char;
    }

    struct Api {
        find_decoder_by_name: unsafe extern "C" fn(*const c_char) -> *const ffi::AVCodec,
        alloc_context3: unsafe extern "C" fn(*const ffi::AVCodec) -> *mut ffi::AVCodecContext,
        open2: unsafe extern "C" fn(*mut ffi::AVCodecContext, *const ffi::AVCodec, *mut *mut ffi::AVDictionary) -> c_int,
        free_context: unsafe extern "C" fn(*mut *mut ffi::AVCodecContext),
        packet_alloc: unsafe extern "C" fn() -> *mut ffi::AVPacket,
        new_packet: unsafe extern "C" fn(*mut ffi::AVPacket, c_int) -> c_int,
        packet_free: unsafe extern "C" fn(*mut *mut ffi::AVPacket),
        send_packet: unsafe extern "C" fn(*mut ffi::AVCodecContext, *const ffi::AVPacket) -> c_int,
        receive_frame: unsafe extern "C" fn(*mut ffi::AVCodecContext, *mut ffi::AVFrame) -> c_int,
        frame_alloc: unsafe extern "C" fn() -> *mut ffi::AVFrame,
        frame_free: unsafe extern "C" fn(*mut *mut ffi::AVFrame),
    }

    static API: OnceLock<Option<Api>> = OnceLock::new();

    /// handle of a loaded image whose file name starts with prefix, opened only for its own symbols
    fn loaded_image(prefix: &str) -> Option<*mut c_void> {
        let count = unsafe { _dyld_image_count() };
        (0..count).find_map(|index| {
            let name = unsafe { _dyld_get_image_name(index) };
            if name.is_null() {
                return None;
            }
            let path = unsafe { CStr::from_ptr(name) };
            let file = path.to_str().ok()?.rsplit('/').next()?;
            if !file.starts_with(prefix) || !file.ends_with(".dylib") {
                return None;
            }
            let handle = unsafe { dlopen(path.as_ptr(), RTLD_LAZY | RTLD_LOCAL | RTLD_FIRST) };
            (!handle.is_null()).then_some(handle)
        })
    }

    fn symbol<T: Copy>(handle: *mut c_void, name: &CStr) -> Option<T> {
        let address = unsafe { dlsym(handle, name.as_ptr()) };
        if address.is_null() {
            return None;
        }
        Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&address) })
    }

    fn api() -> Option<&'static Api> {
        API.get_or_init(|| {
            let codec = loaded_image("libavcodec.")?;
            let util = loaded_image("libavutil.")?;
            Some(Api {
                find_decoder_by_name: symbol(codec, c"avcodec_find_decoder_by_name")?,
                alloc_context3: symbol(codec, c"avcodec_alloc_context3")?,
                open2: symbol(codec, c"avcodec_open2")?,
                free_context: symbol(codec, c"avcodec_free_context")?,
                packet_alloc: symbol(codec, c"av_packet_alloc")?,
                new_packet: symbol(codec, c"av_new_packet")?,
                packet_free: symbol(codec, c"av_packet_free")?,
                send_packet: symbol(codec, c"avcodec_send_packet")?,
                receive_frame: symbol(codec, c"avcodec_receive_frame")?,
                frame_alloc: symbol(util, c"av_frame_alloc")?,
                frame_free: symbol(util, c"av_frame_free")?,
            })
        })
        .as_ref()
    }

    pub(super) struct IsolatedDecoder {
        api: &'static Api,
        plan: DecoderPlan,
        context: *mut ffi::AVCodecContext,
        frame: *mut ffi::AVFrame,
        has_frame: bool,
        frame_index: usize,
    }

    // invariant: the raw context and frame are owned by this value alone and used from one thread at a time
    unsafe impl Send for IsolatedDecoder {}

    impl IsolatedDecoder {
        /// none when the shipped libavcodec is not loaded in this process
        pub(super) fn open(plan: DecoderPlan, request: &DecoderRequest) -> Option<Result<Self, DecoderError>> {
            let api = api()?;
            let unsupported = || DecoderError::UnsupportedCodec(request.codec.ffmpeg_name().into());
            let Ok(name) = std::ffi::CString::new(request.codec.ffmpeg_name()) else {
                return Some(Err(unsupported()));
            };
            let codec = unsafe { (api.find_decoder_by_name)(name.as_ptr()) };
            if codec.is_null() {
                return Some(Err(unsupported()));
            }
            let mut context = unsafe { (api.alloc_context3)(codec) };
            if context.is_null() {
                return Some(Err(DecoderError::Backend("avcodec_alloc_context3 failed".into())));
            }
            let pixels = client_limit(VIDEO_FRAME_BYTES_KEY) / u64::from(FrameFormat::Bgra8.bytes_per_luma_sample());
            unsafe {
                (*context).width = request.width as c_int;
                (*context).height = request.height as c_int;
                (*context).max_pixels = i64::try_from(pixels).unwrap_or(i64::MAX);
            }
            let opened = unsafe { (api.open2)(context, codec, std::ptr::null_mut()) };
            if opened < 0 {
                unsafe { (api.free_context)(&mut context) };
                return Some(Err(DecoderError::Backend(format!("avcodec_open2: {opened}"))));
            }
            let frame = unsafe { (api.frame_alloc)() };
            if frame.is_null() {
                unsafe { (api.free_context)(&mut context) };
                return Some(Err(DecoderError::Backend("av_frame_alloc failed".into())));
            }
            Some(Ok(Self { api, plan, context, frame, has_frame: false, frame_index: 0 }))
        }

        fn plane(&self, index: usize, row: u32, rows: u32) -> Option<(&[u8], u32)> {
            let frame = unsafe { &*self.frame };
            let stride = u32::try_from(frame.linesize[index]).ok().filter(|stride| *stride >= row)?;
            let pointer = frame.data[index];
            if pointer.is_null() || rows == 0 {
                return None;
            }
            let len = plane_len(stride, row, rows)?;
            Some((unsafe { std::slice::from_raw_parts(pointer, len) }, stride))
        }
    }

    impl Drop for IsolatedDecoder {
        fn drop(&mut self) {
            unsafe {
                (self.api.frame_free)(&mut self.frame);
                (self.api.free_context)(&mut self.context);
            }
        }
    }

    impl VideoDecoder for IsolatedDecoder {
        fn plan(&self) -> &DecoderPlan {
            &self.plan
        }

        fn send_packet(&mut self, packet: &[u8], timestamp_us: i64) -> Result<(), DecoderError> {
            let size = c_int::try_from(packet.len())
                .map_err(|_| DecoderError::Backend("packet larger than AVPacket can hold".into()))?;
            let mut av_packet = unsafe { (self.api.packet_alloc)() };
            if av_packet.is_null() {
                return Err(DecoderError::Backend("av_packet_alloc failed".into()));
            }
            let sent = unsafe {
                if (self.api.new_packet)(av_packet, size) < 0 {
                    (self.api.packet_free)(&mut av_packet);
                    return Err(DecoderError::Backend("av_new_packet failed".into()));
                }
                std::ptr::copy_nonoverlapping(packet.as_ptr(), (*av_packet).data, packet.len());
                (*av_packet).pts = timestamp_us;
                (*av_packet).dts = timestamp_us;
                let sent = (self.api.send_packet)(self.context, av_packet);
                (self.api.packet_free)(&mut av_packet);
                sent
            };
            if sent < 0 && sent != AVERROR_EAGAIN {
                return Err(DecoderError::Backend(format!("avcodec_send_packet: {sent}")));
            }
            Ok(())
        }

        fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, DecoderError> {
            let received = unsafe { (self.api.receive_frame)(self.context, self.frame) };
            if received == AVERROR_EAGAIN || received == AVERROR_EOF {
                self.has_frame = false;
                return Ok(None);
            }
            if received < 0 {
                self.has_frame = false;
                return Err(DecoderError::Backend(format!("avcodec_receive_frame: {received}")));
            }
            self.has_frame = true;
            let frame = unsafe { &*self.frame };
            let width = u32::try_from(frame.width).unwrap_or(0);
            let height = u32::try_from(frame.height).unwrap_or(0);
            let strides: Vec<u32> = frame
                .linesize
                .iter()
                .take(FrameFormat::I420.plane_count())
                .map(|stride| (*stride).unsigned_abs())
                .collect();
            let geometry = FrameGeometry::with_strides(FrameFormat::I420, width, height, &strides)
                .map_err(|error| DecoderError::InvalidGeometry(format!("{error:?}")))?;
            self.frame_index = self.frame_index.wrapping_add(1);
            Ok(Some(DecodedFrame {
                geometry,
                presentation_timestamp_us: frame.pts,
                handle: FrameHandle::HostBuffer(self.frame_index),
            }))
        }

        fn flush(&mut self) -> Result<(), DecoderError> {
            let sent = unsafe { (self.api.send_packet)(self.context, std::ptr::null()) };
            if sent < 0 && sent != AVERROR_EOF {
                return Err(DecoderError::Backend(format!("flush: {sent}")));
            }
            Ok(())
        }

        fn host_planes(&self) -> Option<HostI420Planes<'_>> {
            if !self.has_frame {
                return None;
            }
            let frame = unsafe { &*self.frame };
            if frame.format != ffi::AV_PIX_FMT_YUV420P {
                return None;
            }
            let width = u32::try_from(frame.width).ok().filter(|width| *width > 0)?;
            let height = u32::try_from(frame.height).ok().filter(|height| *height > 0)?;
            let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
            let (y, stride_y) = self.plane(0, width, height)?;
            let (u, stride_u) = self.plane(1, chroma_width, chroma_height)?;
            let (v, stride_v) = self.plane(2, chroma_width, chroma_height)?;
            Some(HostI420Planes { width, height, y, u, v, stride_y, stride_u, stride_v })
        }
    }
}
