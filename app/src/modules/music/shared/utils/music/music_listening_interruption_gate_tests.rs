use crate::modules::music::shared::utils::music::music_listening_interruption_gate::{
    MusicListeningAutoInterruptionAction, MusicListeningInterruptionDecision, MusicListeningInterruptionPolicy,
    MusicListeningResyncAction,
};
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

type MusicListeningInterruptionPolicyTests = MusicListeningInterruptionPolicy;

#[test]
fn no_session_always_proceeds() {
    assert_eq!(
        MusicListeningInterruptionPolicyTests::decision(false, false, false, true, true, false),
        MusicListeningInterruptionDecision::Proceed
    );
}

#[test]
fn rights_holder_confirms_only_while_something_is_playing() {
    assert_eq!(
        MusicListeningInterruptionPolicyTests::decision(true, false, true, true, false, false),
        MusicListeningInterruptionDecision::ConfirmPauseAll
    );
    assert_eq!(
        MusicListeningInterruptionPolicyTests::decision(true, true, false, false, true, false),
        MusicListeningInterruptionDecision::ConfirmPauseAll
    );
    assert_eq!(
        MusicListeningInterruptionPolicyTests::decision(true, false, true, false, false, false),
        MusicListeningInterruptionDecision::Proceed
    );
}

#[test]
fn consent_episode_suppresses_repeat_confirmations() {
    assert_eq!(
        MusicListeningInterruptionPolicyTests::decision(true, false, true, true, true, true),
        MusicListeningInterruptionDecision::Proceed
    );
}

#[test]
fn without_playback_control_leaving_always_needs_a_confirmation() {
    for playing in [true, false] {
        assert_eq!(
            MusicListeningInterruptionPolicyTests::decision(true, false, false, playing, playing, true),
            MusicListeningInterruptionDecision::ConfirmLeave
        );
    }
}

#[test]
fn auto_interruption_pauses_the_room_for_rights_holders_and_removes_the_rest() {
    assert_eq!(
        MusicListeningInterruptionPolicyTests::auto_action(true, true, false, true, true, false),
        MusicListeningAutoInterruptionAction::PauseAll
    );
    assert_eq!(
        MusicListeningInterruptionPolicyTests::auto_action(true, false, false, true, true, false),
        MusicListeningAutoInterruptionAction::Leave
    );
    assert_eq!(
        MusicListeningInterruptionPolicyTests::auto_action(false, false, false, true, true, false),
        MusicListeningAutoInterruptionAction::None
    );
}

#[test]
fn auto_interruption_is_idempotent_and_skips_an_idle_room() {
    assert_eq!(
        MusicListeningInterruptionPolicyTests::auto_action(true, true, true, false, true, true),
        MusicListeningAutoInterruptionAction::None
    );
    assert_eq!(
        MusicListeningInterruptionPolicyTests::auto_action(true, false, true, false, false, false),
        MusicListeningAutoInterruptionAction::None
    );
}

#[test]
fn resync_resumes_the_room_this_device_paused_but_adopts_a_resumed_room() {
    assert_eq!(
        MusicListeningInterruptionPolicyTests::resync_action(true, true, false),
        MusicListeningResyncAction::Republish
    );
    assert_eq!(
        MusicListeningInterruptionPolicyTests::resync_action(false, true, false),
        MusicListeningResyncAction::Republish
    );
    assert_eq!(
        MusicListeningInterruptionPolicyTests::resync_action(true, true, true),
        MusicListeningResyncAction::Align
    );
    assert_eq!(
        MusicListeningInterruptionPolicyTests::resync_action(true, false, true),
        MusicListeningResyncAction::Republish
    );
    assert_eq!(
        MusicListeningInterruptionPolicyTests::resync_action(false, false, true),
        MusicListeningResyncAction::Align
    );
}

struct MusicAdHocPlaybackScopeTests;

impl MusicAdHocPlaybackScopeTests {
    fn track(id: &str, source_chat_id: Option<&str>) -> TrackModel {
        TrackModel {
            id: id.to_owned(),
            title: "Title".to_owned(),
            artist: "Artist".to_owned(),
            artist_id: String::new(),
            album_title: String::new(),
            album_id: String::new(),
            duration_seconds: 60,
            cover_colors: Vec::new(),
            cover_image_url: String::new(),
            audio_url: "https://example.com/a.mp3".to_owned(),
            is_exclusive: false,
            status: String::new(),
            description: None,
            source_chat_id: source_chat_id.map(str::to_owned),
            source_message_id: None,
            artist_is_registered: false,
            remote_audio_url: None,
            transfer_phase: None,
            transferred_bytes: 0,
            transfer_total_bytes: 0,
            transfer_error: None,
            library_owner_user_id: None,
            playback_cache_id: None,
            is_liked: false,
        }
    }
}

#[test]
fn chat_and_profile_scopes_are_ad_hoc() {
    assert!(MusicPlayerState::is_ad_hoc_playback_context(Some("chat:abc"), &MusicAdHocPlaybackScopeTests::track("t1", None)));
    assert!(MusicPlayerState::is_ad_hoc_playback_context(Some("profile-featured"), &MusicAdHocPlaybackScopeTests::track("t1", None)));
    assert!(MusicPlayerState::is_ad_hoc_playback_context(Some("profile-featured:u1"), &MusicAdHocPlaybackScopeTests::track("t1", None)));
}

#[test]
fn library_scopes_are_not_ad_hoc() {
    for scope in ["playlist:p1", "search", "liked_songs", "album:a1", "artist:x", "local_tracks", "track:t1"] {
        assert!(!MusicPlayerState::is_ad_hoc_playback_context(Some(scope), &MusicAdHocPlaybackScopeTests::track("t1", None)));
    }
}

#[test]
fn scopeless_playback_falls_back_to_the_track_origin() {
    assert!(MusicPlayerState::is_ad_hoc_playback_context(None, &MusicAdHocPlaybackScopeTests::track("t1", Some("c1"))));
    assert!(!MusicPlayerState::is_ad_hoc_playback_context(None, &MusicAdHocPlaybackScopeTests::track("t1", None)));
    assert!(!MusicPlayerState::is_ad_hoc_playback_context(Some("playlist:p1"), &MusicAdHocPlaybackScopeTests::track("t1", Some("c1"))));
}
