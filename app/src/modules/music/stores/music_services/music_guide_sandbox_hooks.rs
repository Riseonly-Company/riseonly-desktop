use gpui::App;
use rise_core::foundation::foundation_url::Url;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::music::stores::music_actions::music_types::{
    MusicListeningAudioEffect, MusicListeningParticipantPermissions, MusicPlaylistInviteLink,
    MusicPlaylistJoinRequest, MusicPlaylistListeningSession, MusicPlaylistListeningSnapshot, MusicPlaylistMember,
    MusicPlaylistMemberPermissions, MusicPlaylistSettings,
};
use crate::modules::music::stores::music_actions::music_wire_types::{
    RiseStoredMusicPlaylist, RiseStoredMusicTrack, WireLocalPlaylistDestinationDTO,
};

pub struct MusicGuideSandboxUpdateSessionState {
    pub track_id: String,
    pub position_ms: i64,
    pub is_playing: bool,
    pub shuffle_enabled: bool,
    pub shuffle_seed: i64,
    pub repeat_mode: String,
    pub audio_effect_enabled: bool,
    pub audio_effect_rate_milli: i64,
    pub audio_effects: Vec<MusicListeningAudioEffect>,
}

pub struct MusicGuideSandboxReaderTrack {
    pub title: String,
    pub artist_name: String,
    pub duration_ms: i64,
    pub audio_file: Url,
    pub cover_file: Option<Url>,
}

pub struct MusicGuideSandboxHooks {
    pub playlist_id: String,
    pub owns_playlist_id: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub owns_session_id: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub owns_track_id: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub owns_sound: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub is_armed: Box<dyn Fn(&App) -> bool + Send + Sync>,
    pub playlist: Box<dyn Fn(&str, &mut App) -> Option<RiseStoredMusicPlaylist> + Send + Sync>,
    pub playlist_destination: Box<dyn Fn(&mut App) -> Option<WireLocalPlaylistDestinationDTO> + Send + Sync>,
    pub playlist_settings: Box<dyn Fn(&str, &mut App) -> Option<MusicPlaylistSettings> + Send + Sync>,
    pub members: Box<dyn Fn(&str, &mut App) -> Option<Vec<MusicPlaylistMember>> + Send + Sync>,
    pub join_requests: Box<dyn Fn(&str, &App) -> Option<Vec<MusicPlaylistJoinRequest>> + Send + Sync>,
    pub invite_link: Box<dyn Fn(&str, &App) -> Option<MusicPlaylistInviteLink> + Send + Sync>,
    pub listening_snapshot: Box<dyn Fn(&str, &mut App) -> Option<MusicPlaylistListeningSnapshot> + Send + Sync>,
    pub create_playlist: Box<dyn Fn(&str, &mut App) + Send + Sync>,
    pub add_track_to_playlist: Box<dyn Fn(&mut App) + Send + Sync>,
    pub add_reader_track: Box<dyn Fn(MusicGuideSandboxReaderTrack, &mut App) + Send + Sync>,
    pub remove_panda_from_playlist: Box<dyn Fn(&mut App) + Send + Sync>,
    pub update_panda_permissions:
        Box<dyn Fn(MusicPlaylistMemberPermissions, &mut App) -> Option<MusicPlaylistMember> + Send + Sync>,
    pub update_participant_permissions: Box<
        dyn Fn(&str, MusicListeningParticipantPermissions, &mut App) -> Option<MusicPlaylistListeningSession>
            + Send
            + Sync,
    >,
    pub review_join_request: Box<dyn Fn(bool, &mut App) -> Option<MusicPlaylistMember> + Send + Sync>,
    pub join_listening: Box<dyn Fn(&str, &mut App) -> Option<MusicPlaylistListeningSession> + Send + Sync>,
    pub update_session_state: Box<
        dyn Fn(MusicGuideSandboxUpdateSessionState, &mut App) -> Option<MusicPlaylistListeningSession> + Send + Sync,
    >,
    pub session_ended: Box<dyn Fn(&mut App) + Send + Sync>,
    pub sound_track: Box<dyn Fn(&str, &App) -> Option<RiseStoredMusicTrack> + Send + Sync>,
    pub track_playback_started: Box<dyn Fn(&mut App) + Send + Sync>,
    pub note_invite_link_copied: Box<dyn Fn(&mut App) + Send + Sync>,
    pub composer_suggested_tracks: Box<dyn Fn(&mut App) -> Vec<RiseStoredMusicTrack> + Send + Sync>,
}

pub(crate) static MUSIC_GUIDE_SANDBOX_HOOKS: FeatureHookSlot<MusicGuideSandboxHooks> = FeatureHookSlot::new("MUSIC_GUIDE_SANDBOX_HOOKS");

pub fn set_music_guide_sandbox_hooks(hooks: MusicGuideSandboxHooks) {
    let _ = MUSIC_GUIDE_SANDBOX_HOOKS.set(hooks);
}

pub fn music_guide_sandbox_hooks() -> Option<&'static MusicGuideSandboxHooks> {
    MUSIC_GUIDE_SANDBOX_HOOKS.get()
}

impl MusicGuideSandboxHooks {
    pub fn playlist_id() -> Option<&'static str> {
        music_guide_sandbox_hooks().map(|hooks| hooks.playlist_id.as_str())
    }

    pub fn owns_playlist_id(playlist_id: &str, cx: &App) -> bool {
        music_guide_sandbox_hooks().is_some_and(|hooks| (hooks.owns_playlist_id)(playlist_id, cx))
    }

    pub fn owns_session_id(session_id: &str, cx: &App) -> bool {
        music_guide_sandbox_hooks().is_some_and(|hooks| (hooks.owns_session_id)(session_id, cx))
    }

    pub fn owns_track_id(track_id: &str, cx: &App) -> bool {
        music_guide_sandbox_hooks().is_some_and(|hooks| (hooks.owns_track_id)(track_id, cx))
    }

    pub fn owns_sound(track_id: &str, cx: &App) -> bool {
        music_guide_sandbox_hooks().is_some_and(|hooks| (hooks.owns_sound)(track_id, cx))
    }

    pub fn is_armed(cx: &App) -> bool {
        music_guide_sandbox_hooks().is_some_and(|hooks| (hooks.is_armed)(cx))
    }

    pub fn playlist(playlist_id: &str, cx: &mut App) -> Option<RiseStoredMusicPlaylist> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.playlist)(playlist_id, cx))
    }

    pub fn playlist_destination(cx: &mut App) -> Option<WireLocalPlaylistDestinationDTO> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.playlist_destination)(cx))
    }

    pub fn playlist_settings(playlist_id: &str, cx: &mut App) -> Option<MusicPlaylistSettings> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.playlist_settings)(playlist_id, cx))
    }

    pub fn members(playlist_id: &str, cx: &mut App) -> Option<Vec<MusicPlaylistMember>> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.members)(playlist_id, cx))
    }

    pub fn join_requests(playlist_id: &str, cx: &App) -> Option<Vec<MusicPlaylistJoinRequest>> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.join_requests)(playlist_id, cx))
    }

    pub fn invite_link(playlist_id: &str, cx: &App) -> Option<MusicPlaylistInviteLink> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.invite_link)(playlist_id, cx))
    }

    pub fn listening_snapshot(playlist_id: &str, cx: &mut App) -> Option<MusicPlaylistListeningSnapshot> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.listening_snapshot)(playlist_id, cx))
    }

    pub fn create_playlist(named: &str, cx: &mut App) {
        if let Some(hooks) = music_guide_sandbox_hooks() {
            (hooks.create_playlist)(named, cx);
        }
    }

    pub fn add_track_to_playlist(cx: &mut App) {
        if let Some(hooks) = music_guide_sandbox_hooks() {
            (hooks.add_track_to_playlist)(cx);
        }
    }

    pub fn add_reader_track(track: MusicGuideSandboxReaderTrack, cx: &mut App) {
        if let Some(hooks) = music_guide_sandbox_hooks() {
            (hooks.add_reader_track)(track, cx);
        }
    }

    pub fn remove_panda_from_playlist(cx: &mut App) {
        if let Some(hooks) = music_guide_sandbox_hooks() {
            (hooks.remove_panda_from_playlist)(cx);
        }
    }

    pub fn update_panda_permissions(
        permissions: MusicPlaylistMemberPermissions,
        cx: &mut App,
    ) -> Option<MusicPlaylistMember> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.update_panda_permissions)(permissions, cx))
    }

    pub fn update_participant_permissions(
        user_id: &str,
        permissions: MusicListeningParticipantPermissions,
        cx: &mut App,
    ) -> Option<MusicPlaylistListeningSession> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.update_participant_permissions)(user_id, permissions, cx))
    }

    pub fn review_join_request(approve: bool, cx: &mut App) -> Option<MusicPlaylistMember> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.review_join_request)(approve, cx))
    }

    pub fn join_listening(playlist_id: &str, cx: &mut App) -> Option<MusicPlaylistListeningSession> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.join_listening)(playlist_id, cx))
    }

    pub fn update_session_state(
        state: MusicGuideSandboxUpdateSessionState,
        cx: &mut App,
    ) -> Option<MusicPlaylistListeningSession> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.update_session_state)(state, cx))
    }

    pub fn session_ended(cx: &mut App) {
        if let Some(hooks) = music_guide_sandbox_hooks() {
            (hooks.session_ended)(cx);
        }
    }

    pub fn sound_track(track_id: &str, cx: &App) -> Option<RiseStoredMusicTrack> {
        music_guide_sandbox_hooks().and_then(|hooks| (hooks.sound_track)(track_id, cx))
    }

    pub fn track_playback_started(cx: &mut App) {
        if let Some(hooks) = music_guide_sandbox_hooks() {
            (hooks.track_playback_started)(cx);
        }
    }

    pub fn note_invite_link_copied(cx: &mut App) {
        if let Some(hooks) = music_guide_sandbox_hooks() {
            (hooks.note_invite_link_copied)(cx);
        }
    }

    pub fn composer_suggested_tracks(cx: &mut App) -> Vec<RiseStoredMusicTrack> {
        music_guide_sandbox_hooks().map(|hooks| (hooks.composer_suggested_tracks)(cx)).unwrap_or_default()
    }
}
