use livekit::webrtc::video_source::native::NativeVideoSource;

use super::rtc_media::*;
use crate::texture::native_surface::RetainedSurface;

pub(super) struct NativeRtcEngine;

impl RtcMediaEngine for NativeRtcEngine {
    fn audio_host_id(&self) -> cpal::HostId {
        cpal::HostId::Alsa
    }
}

pub(super) fn capture_surface(_source: &NativeVideoSource, _surface: &RetainedSurface) -> bool {
    false
}
