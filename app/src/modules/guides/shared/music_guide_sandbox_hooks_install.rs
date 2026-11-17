use gpui::App;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::stores::music_actions::music_types::{
    MusicListeningParticipantPermissions, MusicPlaylistInviteLink, MusicPlaylistJoinRequest,
    MusicPlaylistListeningSession, MusicPlaylistListeningSnapshot, MusicPlaylistMember, MusicPlaylistMemberPermissions,
    MusicPlaylistSettings,
};
use crate::modules::music::stores::music_actions::music_wire_types::{
    RiseStoredMusicPlaylist, RiseStoredMusicTrack, WireLocalPlaylistDestinationDTO,
};
use crate::modules::music::stores::music_services::music_guide_sandbox_hooks::{
    MusicGuideSandboxHooks, MusicGuideSandboxReaderTrack, MusicGuideSandboxUpdateSessionState,
    set_music_guide_sandbox_hooks,
};

pub fn install_music_guide_sandbox_hooks() {
    set_music_guide_sandbox_hooks(MusicGuideSandboxHooks {
        playlist_id: GuideSandbox::playlist_id(),
        owns_playlist_id: Box::new(owns_playlist_id),
        owns_session_id: Box::new(owns_session_id),
        owns_track_id: Box::new(owns_track_id),
        owns_sound: Box::new(owns_sound),
        is_armed: Box::new(is_armed),
        playlist: Box::new(playlist),
        playlist_destination: Box::new(playlist_destination),
        playlist_settings: Box::new(playlist_settings),
        members: Box::new(members),
        join_requests: Box::new(join_requests),
        invite_link: Box::new(invite_link),
        listening_snapshot: Box::new(listening_snapshot),
        create_playlist: Box::new(create_playlist),
        add_track_to_playlist: Box::new(add_track_to_playlist),
        add_reader_track: Box::new(add_reader_track),
        remove_panda_from_playlist: Box::new(remove_panda_from_playlist),
        update_panda_permissions: Box::new(update_panda_permissions),
        update_participant_permissions: Box::new(update_participant_permissions),
        review_join_request: Box::new(review_join_request),
        join_listening: Box::new(join_listening),
        update_session_state: Box::new(update_session_state),
        session_ended: Box::new(session_ended),
        sound_track: Box::new(sound_track),
        track_playback_started: Box::new(track_playback_started),
        note_invite_link_copied: Box::new(note_invite_link_copied),
        composer_suggested_tracks: Box::new(composer_suggested_tracks),
    });
}

fn owns_playlist_id(id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_playlist_id(id))
}

fn owns_session_id(id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_session_id(id))
}

fn owns_track_id(id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_track_id(id))
}

fn owns_sound(id: &str, cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_sound(id))
}

fn is_armed(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).is_armed())
}

fn playlist(id: &str, cx: &mut App) -> Option<RiseStoredMusicPlaylist> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.playlist(id, cx))
}

fn playlist_destination(cx: &mut App) -> Option<WireLocalPlaylistDestinationDTO> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.playlist_destination(cx))
}

fn playlist_settings(id: &str, cx: &mut App) -> Option<MusicPlaylistSettings> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.playlist_settings(id, cx))
}

fn members(id: &str, cx: &mut App) -> Option<Vec<MusicPlaylistMember>> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.members(id, cx))
}

fn join_requests(id: &str, cx: &App) -> Option<Vec<MusicPlaylistJoinRequest>> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).join_requests(id))
}

fn invite_link(id: &str, cx: &App) -> Option<MusicPlaylistInviteLink> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).invite_link(id))
}

fn listening_snapshot(id: &str, cx: &mut App) -> Option<MusicPlaylistListeningSnapshot> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.listening_snapshot(id, cx))
}

fn create_playlist(title: &str, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.create_playlist(title, cx));
}

fn add_track_to_playlist(cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.add_track_to_playlist(cx));
}

fn add_reader_track(track: MusicGuideSandboxReaderTrack, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| {
        sandbox.add_reader_track(
            &track.title,
            &track.artist_name,
            track.duration_ms,
            track.audio_file,
            track.cover_file,
            cx,
        );
    });
}

fn remove_panda_from_playlist(cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.remove_panda_from_playlist(cx));
}

fn update_panda_permissions(permissions: MusicPlaylistMemberPermissions, cx: &mut App) -> Option<MusicPlaylistMember> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.update_panda_permissions(permissions, cx))
}

fn update_participant_permissions(
    user_id: &str,
    permissions: MusicListeningParticipantPermissions,
    cx: &mut App,
) -> Option<MusicPlaylistListeningSession> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.update_participant_permissions(user_id, permissions, cx))
}

fn review_join_request(approve: bool, cx: &mut App) -> Option<MusicPlaylistMember> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.review_join_request(approve, cx))
}

fn join_listening(id: &str, cx: &mut App) -> Option<MusicPlaylistListeningSession> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.join_listening(id, cx))
}

fn update_session_state(state: MusicGuideSandboxUpdateSessionState, cx: &mut App) -> Option<MusicPlaylistListeningSession> {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| {
        sandbox.update_session_state(
            &state.track_id,
            state.position_ms,
            state.is_playing,
            state.shuffle_enabled,
            state.shuffle_seed,
            &state.repeat_mode,
            state.audio_effect_enabled,
            state.audio_effect_rate_milli,
            state.audio_effects,
            cx,
        )
    })
}

fn session_ended(cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.session_ended(cx));
}

fn sound_track(id: &str, cx: &App) -> Option<RiseStoredMusicTrack> {
    GuideSandbox::existing(cx).and_then(|sandbox| sandbox.read(cx).sound_track(id))
}

fn track_playback_started(cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.track_playback_started(cx));
}

fn note_invite_link_copied(cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.note_invite_link_copied(cx));
}

fn composer_suggested_tracks(cx: &mut App) -> Vec<RiseStoredMusicTrack> {
    GuideSandbox::existing(cx)
        .map(|sandbox| sandbox.update(cx, |sandbox, cx| sandbox.composer_suggested_tracks(cx)))
        .unwrap_or_default()
}
