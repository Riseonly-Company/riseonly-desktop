use gpui::TestAppContext;
use rise_core::foundation::foundation_uuid::Uuid;
use serde::{Deserialize, Serialize};

use crate::core::lib::sai::core::utilities::sai_cache_cleaner::{SaiCacheCleanLayer, SaiCacheCleaner};
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_types::{
    MusicListeningCommandPipelinePolicy, MusicListeningParticipant, MusicListeningParticipantPermissions,
    MusicListeningSessionCommand, MusicListeningSessionStateVector, MusicListeningSessionTransitionDecision,
    MusicListeningSessionTransitionPolicy, MusicPlaylistListeningSession, MusicUserReference,
};
use crate::modules::music::stores::music_actions::music_wire_types::{
    WireListeningParticipantDTO, WireListeningParticipantPermissionsDTO, WirePlaylistListenerDTO,
    WirePlaylistListeningSessionDTO, WirePlaylistListeningSessionResponseDTO, WirePlaylistListeningSnapshotDTO,
    WireUserRefDTO,
};
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct MusicPlaylistRealtimeTypedFixture {
    value: String,
}

struct MusicPlaylistRealtimeStoreTests;

impl MusicPlaylistRealtimeStoreTests {
    fn new_id() -> String {
        Uuid::new_v4().uuid_string().to_lowercase()
    }

    fn transition_session(host_user_id: &str, participant_user_ids: &[String]) -> MusicPlaylistListeningSession {
        MusicPlaylistListeningSession {
            session_id: Self::new_id(),
            playlist_id: Self::new_id(),
            host_user_id: host_user_id.to_owned(),
            host_user: MusicUserReference {
                id: host_user_id.to_owned(),
                name: "Host".to_owned(),
                tag: "host".to_owned(),
                logo: String::new(),
                is_premium: false,
                premium_badge_emoji: None,
                name_color: None,
            },
            track_id: Self::new_id(),
            anchor_position_ms: 0,
            is_playing: true,
            anchor_server_time_ms: 1,
            revision: 1,
            generation: 1,
            shuffle_enabled: false,
            shuffle_seed: 0,
            repeat_mode: "off".to_owned(),
            participants: participant_user_ids
                .iter()
                .map(|user_id| MusicListeningParticipant {
                    user_id: user_id.clone(),
                    user: MusicUserReference {
                        id: user_id.clone(),
                        name: "Participant".to_owned(),
                        tag: "participant".to_owned(),
                        logo: String::new(),
                        is_premium: false,
                        premium_badge_emoji: None,
                        name_color: None,
                    },
                    permissions: MusicListeningParticipantPermissions::NONE,
                    joined_at_ms: 1,
                })
                .collect(),
            viewer_permissions: MusicListeningParticipantPermissions::HOST,
            lease_until_ms: 10_000,
            audio_effect_enabled: false,
            audio_effect_rate_milli: 1_000,
            audio_effects: Vec::new(),
        }
    }

    fn state_vector(track_id: &str, position_ms: i64) -> MusicListeningSessionStateVector {
        MusicListeningSessionStateVector {
            track_id: track_id.to_owned(),
            position_ms,
            is_playing: true,
            shuffle_enabled: false,
            shuffle_seed: 0,
            repeat_mode: "off".to_owned(),
            audio_effect_enabled: false,
            audio_effect_rate_milli: 1_000,
            audio_effects_canonical: String::new(),
        }
    }

    fn late_write_response(
        playlist_id: &str,
        session_id: &str,
        host_user_id: &str,
        track_id: &str,
        revision: i64,
    ) -> WirePlaylistListeningSessionResponseDTO {
        WirePlaylistListeningSessionResponseDTO {
            server_time_ms: 100_000 + revision,
            session: Some(WirePlaylistListeningSessionDTO {
                anchor_position_ms: 0,
                anchor_server_time_ms: 100_000 + revision,
                generation: 1,
                host_user: None,
                host_user_id: host_user_id.to_owned(),
                is_playing: true,
                lease_until_ms: 150_000,
                participants: Vec::new(),
                playlist_id: playlist_id.to_owned(),
                repeat_mode: "off".to_owned(),
                revision,
                session_id: session_id.to_owned(),
                shuffle_enabled: false,
                shuffle_seed: 0,
                track_id: track_id.to_owned(),
                viewer_permissions: Some(WireListeningParticipantPermissionsDTO {
                    can_change_tracks: true,
                    can_control_playback: true,
                    can_seek: true,
                    ..WireListeningParticipantPermissionsDTO::default()
                }),
                ..WirePlaylistListeningSessionDTO::default()
            }),
        }
    }
}

#[test]
fn json_codec_accepts_typed_sai_callback_payload() {
    let payload = MusicPlaylistRealtimeTypedFixture { value: "wire".to_owned() };
    let encoded = SaiJSONCodec::encode(&payload).map(serde_json::Value::Object);
    let decoded = SaiJSONCodec::decode::<MusicPlaylistRealtimeTypedFixture>(encoded.as_ref())
        .expect("typed payload decodes");

    assert_eq!(decoded, payload);
}

#[gpui::test]
fn typed_snapshot_callback_installs_live_listeners(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let playlist_id = MusicPlaylistRealtimeStoreTests::new_id();
        let track_id = MusicPlaylistRealtimeStoreTests::new_id();
        let user_id = MusicPlaylistRealtimeStoreTests::new_id();
        let cache_key = MusicServicesStore::playlist_listening_snapshot_cache_key(&playlist_id, None);

        let payload = WirePlaylistListeningSnapshotDTO {
            active_session: None,
            listeners: vec![WirePlaylistListenerDTO {
                anchor_position_ms: 4_200,
                anchor_server_time_ms: 50_000,
                audio_effects: Vec::new(),
                client_sequence: 7,
                is_playing: true,
                session_id: None,
                track_id: track_id.clone(),
                user: Some(WireUserRefDTO {
                    id: user_id.clone(),
                    is_premium: false,
                    logo: "listener.png".to_owned(),
                    name: "Listener".to_owned(),
                    premium_badge_emoji: None,
                    tag: "listener".to_owned(),
                    ..WireUserRefDTO::default()
                }),
                user_id: user_id.clone(),
            }],
            presence_sequence: 9,
            server_time_ms: 50_000,
            total_listener_count: 1,
            unchanged: false,
            watcher_lease_until_ms: 80_000,
        };

        let services = MusicServicesStore::shared(cx);
        services.update(cx, |services, cx| {
            services.handle_playlist_listening_snapshot_success(
                Some(Box::new(payload)),
                &playlist_id,
                None,
                49_990,
                cx,
            );
        });

        let snapshot = services
            .update(cx, |services, cx| services.playlist_listening_snapshot(&playlist_id, None, cx))
            .expect("snapshot installed");
        let listener = snapshot.listeners.first().expect("listener installed");
        assert_eq!(snapshot.total_listener_count, 1);
        assert_eq!(snapshot.presence_sequence, 9);
        assert_eq!(listener.user_id, user_id);
        assert_eq!(listener.track_id, track_id);
        assert!(listener.is_playing);

        SaiCacheCleaner::clean(&[cache_key], SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[gpui::test]
fn typed_join_callback_installs_listening_session(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let playlist_id = MusicPlaylistRealtimeStoreTests::new_id();
        let track_id = MusicPlaylistRealtimeStoreTests::new_id();
        let session_id = MusicPlaylistRealtimeStoreTests::new_id();
        let host_user_id = MusicPlaylistRealtimeStoreTests::new_id();
        let participant_user_id = MusicPlaylistRealtimeStoreTests::new_id();
        let cache_key = MusicServicesStore::playlist_listening_session_cache_key(&session_id);

        let restricted = WireListeningParticipantPermissionsDTO {
            can_change_tracks: false,
            can_control_playback: false,
            can_seek: false,
            ..WireListeningParticipantPermissionsDTO::default()
        };
        let session = WirePlaylistListeningSessionDTO {
            anchor_position_ms: 7_500,
            anchor_server_time_ms: 100_000,
            generation: 1,
            host_user: Some(WireUserRefDTO {
                id: host_user_id.clone(),
                is_premium: true,
                logo: "host.png".to_owned(),
                name: "Host".to_owned(),
                premium_badge_emoji: None,
                tag: "host".to_owned(),
                ..WireUserRefDTO::default()
            }),
            host_user_id: host_user_id.clone(),
            is_playing: true,
            lease_until_ms: 130_000,
            participants: vec![WireListeningParticipantDTO {
                joined_at_ms: 99_000,
                permissions: Some(restricted.clone()),
                user: None,
                user_id: participant_user_id.clone(),
            }],
            playlist_id: playlist_id.clone(),
            repeat_mode: "all".to_owned(),
            revision: 2,
            session_id: session_id.clone(),
            shuffle_enabled: true,
            shuffle_seed: 0,
            track_id: track_id.clone(),
            viewer_permissions: Some(restricted),
            ..WirePlaylistListeningSessionDTO::default()
        };
        let payload = WirePlaylistListeningSessionResponseDTO { server_time_ms: 100_000, session: Some(session) };

        let services = MusicServicesStore::shared(cx);
        let result = services
            .update(cx, |services, cx| {
                services.handle_listening_session_response(Some(Box::new(payload)), None, Some(99_990), true, false, cx)
            })
            .expect("session installed");

        assert_eq!(result.session_id, session_id);
        assert_eq!(result.host_user_id, host_user_id);
        assert_eq!(result.track_id, track_id);
        let participant_ids: Vec<&str> =
            result.participants.iter().map(|participant| participant.user_id.as_str()).collect();
        assert_eq!(participant_ids, vec![participant_user_id.as_str()]);
        let cached = services.update(cx, |services, cx| services.playlist_listening_session(&session_id, cx));
        assert_eq!(cached, Some(result));

        SaiCacheCleaner::clean(&[cache_key], SaiCacheCleanLayer::Everywhere, cx);
    });
}

#[test]
fn host_and_followers_apply_accepted_state_from_another_controller() {
    let host_user_id = MusicPlaylistRealtimeStoreTests::new_id();
    let follower_user_id = MusicPlaylistRealtimeStoreTests::new_id();
    let delegated_controller_id = MusicPlaylistRealtimeStoreTests::new_id();

    assert!(MusicPlayerState::should_apply_accepted_listening_state(Some(&delegated_controller_id), &host_user_id));
    assert!(MusicPlayerState::should_apply_accepted_listening_state(Some(&host_user_id), &follower_user_id));
    assert!(!MusicPlayerState::should_apply_accepted_listening_state(
        Some(&delegated_controller_id),
        &delegated_controller_id
    ));
}

#[test]
fn material_commands_dominate_queued_heartbeats_and_resolve_track_changes() {
    assert_eq!(
        MusicListeningCommandPipelinePolicy::dominant(
            MusicListeningSessionCommand::Heartbeat,
            MusicListeningSessionCommand::ChangeTrack
        ),
        MusicListeningSessionCommand::ChangeTrack
    );
    assert_eq!(
        MusicListeningCommandPipelinePolicy::dominant(
            MusicListeningSessionCommand::Seek,
            MusicListeningSessionCommand::Heartbeat
        ),
        MusicListeningSessionCommand::Seek
    );

    let authoritative = MusicPlaylistRealtimeStoreTests::state_vector("track-a", 5_000);
    let delegated_change = MusicPlaylistRealtimeStoreTests::state_vector("track-b", 0);
    assert_eq!(
        MusicListeningCommandPipelinePolicy::resolved_command(
            MusicListeningSessionCommand::Playback,
            Some(&authoritative),
            &delegated_change
        ),
        MusicListeningSessionCommand::ChangeTrack
    );
}

#[test]
fn stale_revision_recovery_is_bounded() {
    assert!(MusicListeningCommandPipelinePolicy::should_retry_stale_revision(
        "Listening session changed; refresh its snapshot",
        None,
        0
    ));
    assert!(!MusicListeningCommandPipelinePolicy::should_retry_stale_revision(
        "Listening session changed; refresh its snapshot",
        None,
        2
    ));
    assert!(!MusicListeningCommandPipelinePolicy::should_retry_stale_revision(
        "The host has not granted this control",
        None,
        0
    ));
}

#[test]
fn listening_session_transition_skips_repeated_join() {
    let host_id = MusicPlaylistRealtimeStoreTests::new_id();
    let participant_id = MusicPlaylistRealtimeStoreTests::new_id();
    let session = MusicPlaylistRealtimeStoreTests::transition_session(&host_id, std::slice::from_ref(&participant_id));

    assert_eq!(
        MusicListeningSessionTransitionPolicy::decision(Some(&session), &participant_id, &host_id, None),
        MusicListeningSessionTransitionDecision::AlreadyJoined
    );
    assert_eq!(
        MusicListeningSessionTransitionPolicy::decision(
            Some(&session),
            &participant_id,
            &MusicPlaylistRealtimeStoreTests::new_id(),
            Some(&session.session_id)
        ),
        MusicListeningSessionTransitionDecision::AlreadyJoined
    );
    assert_eq!(
        MusicListeningSessionTransitionPolicy::decision(
            Some(&session),
            &participant_id,
            &host_id,
            Some(&MusicPlaylistRealtimeStoreTests::new_id())
        ),
        MusicListeningSessionTransitionDecision::LeaveThenJoin
    );
}

#[test]
fn listening_session_transition_closes_empty_host_and_confirms_occupied_host() {
    let host_id = MusicPlaylistRealtimeStoreTests::new_id();
    let target_id = MusicPlaylistRealtimeStoreTests::new_id();
    let empty = MusicPlaylistRealtimeStoreTests::transition_session(&host_id, &[]);
    let occupied =
        MusicPlaylistRealtimeStoreTests::transition_session(&host_id, &[MusicPlaylistRealtimeStoreTests::new_id()]);

    assert_eq!(
        MusicListeningSessionTransitionPolicy::decision(
            Some(&empty),
            &host_id,
            &target_id,
            Some(&MusicPlaylistRealtimeStoreTests::new_id())
        ),
        MusicListeningSessionTransitionDecision::EndThenJoin
    );
    assert_eq!(
        MusicListeningSessionTransitionPolicy::decision(
            Some(&occupied),
            &host_id,
            &target_id,
            Some(&MusicPlaylistRealtimeStoreTests::new_id())
        ),
        MusicListeningSessionTransitionDecision::ConfirmEndThenJoin
    );
}

#[test]
fn listening_session_transition_leaves_follower_and_tolerates_ended_session() {
    let host_id = MusicPlaylistRealtimeStoreTests::new_id();
    let participant_id = MusicPlaylistRealtimeStoreTests::new_id();
    let session = MusicPlaylistRealtimeStoreTests::transition_session(&host_id, std::slice::from_ref(&participant_id));

    assert_eq!(
        MusicListeningSessionTransitionPolicy::decision(
            Some(&session),
            &participant_id,
            &MusicPlaylistRealtimeStoreTests::new_id(),
            Some(&MusicPlaylistRealtimeStoreTests::new_id())
        ),
        MusicListeningSessionTransitionDecision::LeaveThenJoin
    );
    assert!(MusicListeningSessionTransitionPolicy::can_continue_after_close_error("Listening session not found", None));
    assert!(!MusicListeningSessionTransitionPolicy::can_continue_after_close_error(
        "Realtime listening is temporarily unavailable",
        None
    ));
}

#[gpui::test]
fn late_write_response_cannot_roll_back_newer_fanout_revision(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let playlist_id = MusicPlaylistRealtimeStoreTests::new_id();
        let session_id = MusicPlaylistRealtimeStoreTests::new_id();
        let host_user_id = MusicPlaylistRealtimeStoreTests::new_id();
        let cache_key = MusicServicesStore::playlist_listening_session_cache_key(&session_id);

        let services = MusicServicesStore::shared(cx);
        let newer = services
            .update(cx, |services, cx| {
                services.handle_listening_session_response(
                    Some(Box::new(MusicPlaylistRealtimeStoreTests::late_write_response(
                        &playlist_id,
                        &session_id,
                        &host_user_id,
                        "track-new",
                        9,
                    ))),
                    None,
                    None,
                    false,
                    false,
                    cx,
                )
            })
            .expect("newer session installed");
        let late = services
            .update(cx, |services, cx| {
                services.handle_listening_session_response(
                    Some(Box::new(MusicPlaylistRealtimeStoreTests::late_write_response(
                        &playlist_id,
                        &session_id,
                        &host_user_id,
                        "track-old",
                        8,
                    ))),
                    None,
                    None,
                    false,
                    false,
                    cx,
                )
            })
            .expect("cached session returned");

        assert_eq!(newer.revision, 9);
        assert_eq!(late.revision, 9);
        assert_eq!(late.track_id, "track-new");
        let cached = services.update(cx, |services, cx| services.playlist_listening_session(&session_id, cx));
        assert_eq!(cached.map(|session| session.track_id), Some("track-new".to_owned()));

        SaiCacheCleaner::clean(&[cache_key], SaiCacheCleanLayer::Everywhere, cx);
    });
}
