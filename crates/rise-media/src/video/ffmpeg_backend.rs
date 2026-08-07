//! The libavcodec binding, through rsmpeg.
//!
//! rsmpeg rather than ffmpeg-next because it exposes `AVHWDeviceContext` and
//! `AVCodecContext::hw_device_ctx`. Without those there is no way to ask
//! libavcodec for a frame that stays in GPU memory, and every frame comes back
//! on the CPU — which is the entire failure this crate exists to avoid.
//!
//! Compiled only with the `ffmpeg` feature. The FFmpeg it links against must be
//! the LGPL prefix from `scripts/build-ffmpeg-lgpl.sh`; `build.rs` points
//! rsmpeg at it and `scripts/check-ffmpeg-license.py` re-derives the licence
//! from the artefact rather than trusting either.

use std::ffi::{CStr, CString};

use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avutil::AVHWDeviceContext;
use rsmpeg::ffi;

use super::decoder::{
    DecodedFrame, DecoderError, DecoderPlan, DecoderRequest, FrameHandle, HwAccel, VideoDecoder,
};
use crate::texture::frame::FrameGeometry;

pub fn open(
    plan: DecoderPlan,
    request: &DecoderRequest,
) -> Result<Box<dyn VideoDecoder>, DecoderError> {
    match FfmpegDecoder::open(plan.clone(), request) {
        Ok(decoder) => Ok(Box::new(decoder)),
        // A hardware device that will not open is the common case on a VM, an
        // old driver or a CI runner. Degrading is correct; failing the whole
        // stream because the GPU declined is not.
        Err(DecoderError::HardwareUnavailable { .. }) if plan.is_hardware() => {
            let software = plan.degraded_to_software(request)?;
            Ok(Box::new(FfmpegDecoder::open(software, request)?))
        }
        Err(error) => Err(error),
    }
}

struct FfmpegDecoder {
    plan: DecoderPlan,
    context: AVCodecContext,
    /// Held for the lifetime of the session. Dropping it invalidates every
    /// frame libavcodec has handed out, including ones still being presented.
    _hw_device: Option<AVHWDeviceContext>,
    frame_index: usize,
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

        let hw_device = match plan.accel.ffmpeg_device_name() {
            None => None,
            Some(device_name) => {
                let mut device = open_hw_device(plan.accel, device_name)?;
                // rsmpeg exposes set_hw_frames_ctx but not set_hw_device_ctx,
                // and a decoder needs the device: the frames context is
                // allocated by libavcodec from it, after the first packet has
                // told it the real dimensions. So this is set directly.
                //
                // av_buffer_ref because avcodec_free_context unrefs this field,
                // and `device` is still owned by the session.
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

        // av_new_packet rather than pointing AVPacket at a Rust buffer: it
        // allocates AV_INPUT_BUFFER_PADDING_SIZE beyond the payload, and
        // libavcodec's bitstream readers rely on that padding existing. Without
        // it a truncated packet reads past the allocation, with a length the
        // sender controls.
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

        let width = frame.width as u32;
        let height = frame.height as u32;

        let strides: Vec<u32> = frame
            .linesize
            .iter()
            .take(self.plan.format.plane_count())
            .map(|stride| (*stride).unsigned_abs())
            .collect();

        let geometry = FrameGeometry::with_strides(self.plan.format, width, height, &strides)
            .map_err(|error| DecoderError::InvalidGeometry(format!("{error:?}")))?;

        let handle = self.frame_handle(&frame);
        self.frame_index = self.frame_index.wrapping_add(1);

        Ok(Some(DecodedFrame {
            geometry,
            presentation_timestamp_us: frame.pts,
            handle,
        }))
    }

    fn flush(&mut self) -> Result<(), DecoderError> {
        self.context
            .send_packet(None)
            .map_err(|error| DecoderError::Backend(format!("flush: {error}")))
    }
}

impl FfmpegDecoder {
    fn frame_handle(&self, frame: &rsmpeg::avutil::AVFrame) -> FrameHandle {
        if !self.plan.is_hardware() {
            return FrameHandle::HostBuffer(self.frame_index);
        }

        // On a hardware frame, data[3] carries the platform surface: a
        // CVPixelBufferRef under VideoToolbox, an ID3D11Texture2D under
        // D3D11VA, a VASurfaceID under VAAPI. data[0..2] are null, which is why
        // reading a hw frame as if it were planar yields a null-pointer deref
        // rather than a wrong picture.
        let surface = frame.data[3] as usize;

        match self.plan.accel {
            HwAccel::VideoToolbox => FrameHandle::IoSurface(surface as u32),
            HwAccel::D3D11Va => FrameHandle::DxgiHandle(surface),
            HwAccel::Vaapi => FrameHandle::DmaBufFd(surface as i32),
            HwAccel::None => FrameHandle::HostBuffer(self.frame_index),
        }
    }
}

/// The FFmpeg build this binary is linked against, for the About box and for
/// the licence check to read back.
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
