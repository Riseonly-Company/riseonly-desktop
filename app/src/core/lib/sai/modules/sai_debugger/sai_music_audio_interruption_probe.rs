use crate::core::desktop_platform::av_audio_session::{
    AVAudioSession, AVAudioSessionInterruption, AVAudioSessionInterruptionOptions,
    AVAudioSessionInterruptionType,
};
use crate::core::desktop_platform::rise_app_events::RiseAppEvents;

#[derive(Clone, Debug, PartialEq, Eq)]
struct SaiMusicAudioInterruptionProbeContext {
    user_id: String,
    epoch: String,
    track_id: String,
}

/// paired local fixture for the real music interruption observer
/// account changes discard it and it must never get an old end event
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaiMusicAudioInterruptionProbe {
    context: Option<SaiMusicAudioInterruptionProbeContext>,
}

impl SaiMusicAudioInterruptionProbe {
    pub fn is_active(&self) -> bool {
        self.context.is_some()
    }

    pub fn begin(
        &mut self,
        user_id: Option<&str>,
        epoch: Option<&str>,
        track_id: Option<&str>,
        isolated_playback: bool,
    ) -> bool {
        if !cfg!(debug_assertions) || self.context.is_some() || !isolated_playback {
            return false;
        }
        let (Some(user_id), Some(epoch), Some(track_id)) = (user_id, epoch, track_id) else {
            return false;
        };
        if user_id.is_empty() || epoch.is_empty() || track_id.is_empty() {
            return false;
        }
        self.context = Some(SaiMusicAudioInterruptionProbeContext {
            user_id: user_id.to_owned(),
            epoch: epoch.to_owned(),
            track_id: track_id.to_owned(),
        });
        RiseAppEvents::default_center().post_with(
            AVAudioSession::INTERRUPTION_NOTIFICATION,
            AVAudioSessionInterruption {
                kind: AVAudioSessionInterruptionType::Began,
                options: AVAudioSessionInterruptionOptions::empty(),
            },
        );
        true
    }

    pub fn end(
        &mut self,
        user_id: Option<&str>,
        epoch: Option<&str>,
        track_id: Option<&str>,
        allow_resume: bool,
    ) -> bool {
        let Some(previous) = self.context.take() else {
            return false;
        };
        if user_id != Some(previous.user_id.as_str()) || epoch != Some(previous.epoch.as_str()) {
            return false;
        }
        let options = if allow_resume && track_id == Some(previous.track_id.as_str()) {
            AVAudioSessionInterruptionOptions::SHOULD_RESUME
        } else {
            AVAudioSessionInterruptionOptions::empty()
        };
        RiseAppEvents::default_center().post_with(
            AVAudioSession::INTERRUPTION_NOTIFICATION,
            AVAudioSessionInterruption {
                kind: AVAudioSessionInterruptionType::Ended,
                options,
            },
        );
        true
    }
}
