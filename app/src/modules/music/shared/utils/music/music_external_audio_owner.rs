use std::borrow::Cow;

use rise_core::foundation::foundation_uuid::Uuid;

/// who holds the audio output outside the music player
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MusicExternalAudioOwner {
    pub raw_value: Cow<'static, str>,
}

impl MusicExternalAudioOwner {
    pub const LEGACY: Self = Self::fixed("legacy");
    pub const CHAT_VOICE_MESSAGE: Self = Self::fixed("chat.voice-message");
    pub const CHAT_VIDEO_NOTE: Self = Self::fixed("chat.video-note");
    pub const CHAT_VOICE_RECORDER: Self = Self::fixed("chat.voice-recorder");
    pub const CHAT_VIDEO_NOTE_RECORDER: Self = Self::fixed("chat.video-note-recorder");
    pub const POST_AMBIENT: Self = Self::fixed("post.ambient");
    pub const RTC: Self = Self::fixed("rtc");

    const fn fixed(raw_value: &'static str) -> Self {
        Self { raw_value: Cow::Borrowed(raw_value) }
    }

    pub fn unique(prefix: &str) -> Self {
        Self { raw_value: Cow::Owned(format!("{prefix}.{}", Uuid::new_v4().uuid_string())) }
    }
}
