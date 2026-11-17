use std::path::PathBuf;

use gpui::{App, AppContext, Task};
use rise_media::audio::decoder::AudioDecoder as _;
use rise_media::audio::symphonia_backend::SymphoniaDecoder;

use crate::core::assets::RiseAssets;

/// tracks a sandbox scene can put on the computer, shipped under audio/ with one square cover
/// small files so two lessons fit under two megabytes
pub enum GuideSandboxAudio {}

/// picked by the sandbox from the guide so back to back lessons dont reuse a track
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuideSandboxAudioTrack {
    RiseUp,
    Chillhop,
}

impl GuideSandboxAudioTrack {
    pub fn resource(self) -> &'static str {
        match self {
            Self::RiseUp => "guide-rise-up",
            Self::Chillhop => "guide-chillhop",
        }
    }

    pub fn file_name_key(self) -> &'static str {
        match self {
            Self::RiseUp => "guide_scene_track_file",
            Self::Chillhop => "guide_scene_track_file_chillhop",
        }
    }

    pub fn shipped_duration_ms(self) -> i64 {
        match self {
            Self::RiseUp => 117_081,
            Self::Chillhop => 111_647,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuideSandboxAudioPrepared {
    pub track: GuideSandboxAudioTrack,
    pub url: PathBuf,
    pub cover_url: Option<PathBuf>,
    pub byte_count: i64,
    pub duration_ms: i64,
}

impl GuideSandboxAudio {
    const COVER_RESOURCE: &'static str = "guide-rise-up-cover";

    const AUDIO_DIRECTORY: &'static str = "audio";

    /// off the main thread, reading the duration opens the file
    pub fn prepare(track: GuideSandboxAudioTrack, cx: &App) -> Task<Option<GuideSandboxAudioPrepared>> {
        let Some(url) = Self::bundled(track.resource(), "mp3") else {
            return Task::ready(None);
        };
        let cover_url = Self::bundled(Self::COVER_RESOURCE, "jpg");
        cx.background_spawn(async move {
            let byte_count = std::fs::metadata(&url)
                .ok()
                .and_then(|metadata| i64::try_from(metadata.len()).ok())
                .unwrap_or(0);
            let duration_ms = Self::decoded_duration_ms(&url)
                .filter(|milliseconds| *milliseconds > 1_000)
                .unwrap_or_else(|| track.shipped_duration_ms());
            Some(GuideSandboxAudioPrepared {
                track,
                url,
                cover_url,
                byte_count,
                duration_ms,
            })
        })
    }

    /// bundle file lookup, none if the bundle doesnt carry it
    fn bundled(resource: &str, extension: &str) -> Option<PathBuf> {
        let path = RiseAssets::discover()
            .root()?
            .join(Self::AUDIO_DIRECTORY)
            .join(format!("{resource}.{extension}"));
        path.is_file().then_some(path)
    }

    fn decoded_duration_ms(url: &PathBuf) -> Option<i64> {
        let bytes = std::fs::read(url).ok()?;
        let decoder = SymphoniaDecoder::open(bytes).ok()?;
        i64::try_from(decoder.duration_millis()?).ok()
    }
}
