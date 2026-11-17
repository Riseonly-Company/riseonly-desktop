use rise_engine::RiseWireReplayPolicy;

use crate::modules::music::engine::core::rise_music_rpc::RiseMusicEndpoints;
use crate::modules::music::pages::track::track_context_menu_row::MusicLocalTrackEditPolicy;
use crate::modules::music::stores::music_actions::music_wire_types::{RiseStoredMusicTrack, WireArtistDTO};

struct RiseMusicLocalTrackEditTests;

impl RiseMusicLocalTrackEditTests {
    fn track_wire(status: &str) -> RiseStoredMusicTrack {
        RiseStoredMusicTrack {
            id: Some("track-id".to_owned()),
            title: Some("Old title".to_owned()),
            description: None,
            duration_ms: Some(123_000),
            cover_url: Some("/api/file/secure/music/owner/old.jpg".to_owned()),
            audio_url: Some("/api/file/secure/music/owner/audio.mp3".to_owned()),
            play_count: Some(0),
            like_count: Some(0),
            is_liked: Some(false),
            album: None,
            artist: Some(WireArtistDTO { stage_name: "Old composer".to_owned(), ..WireArtistDTO::default() }),
            created_at_ms: Some(1),
            status: Some(status.to_owned()),
            genre: Some("local_upload".to_owned()),
            rejection_reason: None,
            scheduled_at_ms: None,
            is_exclusive: Some(false),
            library_owner_user_id: Some("owner".to_owned()),
            ..RiseStoredMusicTrack::default()
        }
    }
}

#[test]
fn update_endpoint_is_never_replayed() {
    assert_eq!(RiseMusicEndpoints::update_local_track().method, "update_local_track");
    assert_eq!(RiseMusicEndpoints::update_local_track().descriptor().replay_policy, RiseWireReplayPolicy::Never);
}

#[test]
fn optimistic_metadata_preserves_audio_identity() {
    let source = RiseMusicLocalTrackEditTests::track_wire("local_library");
    let updated = source.with_local_metadata("New title", "New composer", Some("file:///tmp/cover.jpg".to_owned()));

    assert_eq!(updated.id, source.id);
    assert_eq!(updated.audio_url, source.audio_url);
    assert_eq!(updated.duration_ms, source.duration_ms);
    assert_eq!(updated.library_owner_user_id, source.library_owner_user_id);
    assert_eq!(updated.title.as_deref(), Some("New title"));
    assert_eq!(updated.artist.as_ref().map(|artist| artist.stage_name.as_str()), Some("New composer"));
}

#[test]
fn only_local_upload_owner_can_edit() {
    let track = RiseMusicLocalTrackEditTests::track_wire("local_library").as_track_model().expect("fixture has an id");

    assert!(MusicLocalTrackEditPolicy::can_edit(&track, Some("owner")));
    assert!(!MusicLocalTrackEditPolicy::can_edit(&track, Some("collaborator")));

    let published = RiseMusicLocalTrackEditTests::track_wire("published").as_track_model().expect("fixture has an id");
    assert!(!MusicLocalTrackEditPolicy::can_edit(&published, Some("owner")));
}
