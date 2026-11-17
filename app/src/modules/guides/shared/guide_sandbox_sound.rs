use rise_catalog::MusicTrackStatus;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::stores::music_actions::music_wire_types::{RiseStoredMusicTrack, WireArtistDTO};
use crate::modules::post::stores::post_services::post_audio_services::PostServicesStoreAudioPostsState;

impl GuideSandbox {
    pub fn owns_sound_scene(&self) -> bool {
        self.is_armed && self.scenes.contains(&*Self::sound_scene_id())
    }

    pub fn owns_sound(&self, id: &str) -> bool {
        self.owns_sound_scene() && id == &*Self::sound_track_id()
    }

    /// the sound as the page reads it, none for any real track
    pub fn sound_track(&self, id: &str) -> Option<RiseStoredMusicTrack> {
        if !self.owns_sound(id) {
            return None;
        }
        let stamp = (Self::scene_stamp() * 1000.0) as i64;
        let cover_url = self
            .audio
            .as_ref()
            .and_then(|audio| audio.cover_url.as_deref())
            .and_then(Url::from_file_path)
            .map(|url| url.absolute_string().to_owned());
        let audio_url = self
            .audio
            .as_ref()
            .and_then(|audio| Url::from_file_path(&audio.url))
            .map(|url| url.absolute_string().to_owned())
            .unwrap_or_default();
        Some(RiseStoredMusicTrack {
            id: Some(Self::sound_track_id().to_owned()),
            title: Some(tr("original_audio_title")),
            description: None,
            duration_ms: Some(self.track_duration_ms()),
            cover_url,
            audio_url: Some(audio_url),
            play_count: Some(0),
            like_count: Some(0),
            is_liked: Some(false),
            album: None,
            artist: Some(WireArtistDTO {
                created_at_ms: stamp,
                stage_name: tr("guide_scene_sender"),
                user_id: Self::panda_user_id().to_owned(),
                ..WireArtistDTO::default()
            }),
            created_at_ms: Some(stamp),
            status: Some(MusicTrackStatus::OriginalAudio.wire().to_owned()),
            genre: None,
            rejection_reason: None,
            scheduled_at_ms: None,
            is_exclusive: Some(false),
            library_owner_user_id: None,
            ..RiseStoredMusicTrack::default()
        })
    }

    /// loaded and empty, the sound is new
    pub fn sound_posts(&self, id: &str) -> Option<PostServicesStoreAudioPostsState> {
        if !self.owns_sound(id) {
            return None;
        }
        Some(PostServicesStoreAudioPostsState {
            has_more: false,
            did_load: true,
            total_count: 0,
            ..PostServicesStoreAudioPostsState::default()
        })
    }
}
