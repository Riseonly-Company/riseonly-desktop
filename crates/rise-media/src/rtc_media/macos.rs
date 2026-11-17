use livekit::webrtc::video_frame::native::NativeBuffer;
use livekit::webrtc::video_frame::{VideoFrame, VideoRotation};
use livekit::webrtc::video_source::native::NativeVideoSource;

use super::rtc_media::*;
use crate::texture::native_surface::macos::pixel_buffer_with_extra_reference;
use crate::texture::native_surface::RetainedSurface;

pub(super) struct NativeRtcEngine;

impl RtcMediaEngine for NativeRtcEngine {
    fn audio_host_id(&self) -> cpal::HostId {
        cpal::HostId::CoreAudio
    }

    fn screen_share_codec(&self) -> PublishedVideoCodec {
        PublishedVideoCodec::H264
    }
}

pub(super) fn capture_surface(source: &NativeVideoSource, surface: &RetainedSurface) -> bool {
    let Some(pixels) = pixel_buffer_with_extra_reference(surface) else {
        return false;
    };
    let buffer = unsafe { NativeBuffer::from_cv_pixel_buffer(pixels) };
    source.capture_frame(&VideoFrame {
        rotation: VideoRotation::VideoRotation0,
        timestamp_us: 0,
        buffer,
    });
    true
}
