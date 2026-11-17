use super::rtc_media::*;

pub(super) struct NativeRtcEngine;

impl RtcMediaEngine for NativeRtcEngine {
    fn audio_host_id(&self) -> cpal::HostId {
        cpal::HostId::Alsa
    }
}
