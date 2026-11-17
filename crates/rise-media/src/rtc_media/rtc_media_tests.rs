use std::collections::HashMap;
use std::time::Duration;

use crate::rtc_media::*;
use rise_platform::HostOs;

const HOSTS: [HostOs; 3] = [HostOs::MacOs, HostOs::Linux, HostOs::Windows];

fn request() -> ConnectRequest {
    ConnectRequest {
        room_id: "room-1".to_owned(),
        connection_id: "conn-1".to_owned(),
        server_url: "wss://user:secret@sfu.example.com:7880/rtc?access_token=abc".to_owned(),
        access_token: "very-secret-token".to_owned(),
        participant_identity: "ident-1".to_owned(),
        video_ceiling: VideoCeiling::FREE,
        audio_bitrate_kbps: Some(32),
        end_to_end_encrypted: true,
        expires_at_ms: 4_102_444_800_000,
    }
}

fn key(connection: &str) -> MediaSessionKey {
    MediaSessionKey {
        room_id: "room-1".to_owned(),
        connection_id: connection.to_owned(),
        participant_identity: "ident-1".to_owned(),
    }
}

fn person(identity: &str, metadata: &str) -> RemoteParticipant {
    RemoteParticipant::new(identity.to_owned(), String::new(), metadata.to_owned(), [], None)
}

fn track(audio: bool, source: TrackSource, muted: bool) -> PublishedTrack {
    PublishedTrack { audio, source, muted }
}

#[test]
fn microphone_ceiling_follows_the_room_bitrate() {
    assert_eq!(microphone_max_bitrate(Some(32)), Some(32_000));
    assert_eq!(microphone_max_bitrate(Some(0)), Some(0));
    assert_eq!(microphone_max_bitrate(None), None);
    assert_eq!(microphone_max_bitrate(Some(-1)), None);
    assert_eq!(microphone_max_bitrate(Some(i64::MAX)), Some(u64::MAX));
}

#[test]
fn track_flags_read_live_publications_only() {
    let quiet = person("a", "");
    assert!(!quiet.microphone_enabled && !quiet.camera_enabled && !quiet.screen_share_enabled);
    assert_eq!(quiet.joined_at_ms, None);

    let live = RemoteParticipant::new(
        "b".to_owned(),
        String::new(),
        String::new(),
        [
            track(true, TrackSource::ScreenShareAudio, false),
            track(false, TrackSource::Camera, true),
            track(false, TrackSource::ScreenShare, false),
        ],
        Some(1_700_000_000_000),
    );
    assert!(live.microphone_enabled);
    assert!(!live.camera_enabled);
    assert!(live.screen_share_enabled);
    assert_eq!(live.joined_at_ms, Some(1_700_000_000_000));

    let camera = RemoteParticipant::new(
        "c".to_owned(),
        String::new(),
        String::new(),
        [track(true, TrackSource::Microphone, true), track(false, TrackSource::Camera, false)],
        None,
    );
    assert!(!camera.microphone_enabled);
    assert!(camera.camera_enabled);
    assert!(!camera.screen_share_enabled);
}

#[test]
fn debug_shows_the_audio_ceiling() {
    assert!(format!("{:?}", request()).contains("audio_bitrate_kbps: Some(32)"));
}

fn connected_session() -> MediaSession {
    let mut session = MediaSession::new(&request());
    let out = session.receive(RoomCallback::Connected {
        local_identity: "ident-1".to_owned(),
    });
    assert_eq!(out, vec![SessionOutput::Connected]);
    session
}

#[test]
fn endpoint_keeps_scheme_host_and_port_only() {
    assert_eq!(sanitized_endpoint(&request().server_url), "wss://sfu.example.com:7880");
    assert_eq!(sanitized_endpoint("wss://sfu.example.com/rtc"), "wss://sfu.example.com");
    assert_eq!(sanitized_endpoint("ws://[::1]:7880/x"), "ws://[::1]:7880");
    assert_eq!(sanitized_endpoint("sfu.example.com"), "invalid");
    assert_eq!(sanitized_endpoint("wss://"), "invalid");
    assert_eq!(sanitized_endpoint("wss://host:notaport"), "invalid");
}

#[test]
fn debug_never_shows_the_token() {
    let text = format!("{:?}", request());
    assert!(!text.contains("very-secret-token"));
    assert!(!text.contains("secret@"));
    assert!(text.contains("<redacted>"));
}

#[test]
fn failure_codes_match_ios() {
    assert_eq!(failure_code(SdkFailure::TimedOut), "rtc_livekit_timeout");
    assert_eq!(failure_code(SdkFailure::TokenRejected), "rtc_livekit_token_validation_failed");
    assert_eq!(failure_code(SdkFailure::RoomDeleted), "rtc_livekit_room_deleted");
    assert_eq!(failure_code(SdkFailure::Audio), "rtc_livekit_audio_failed");
    assert_eq!(failure_code(SdkFailure::Other), "rtc_livekit_connect_failed");
    assert_eq!(disconnect_code(None), "rtc_media_disconnected");
    assert_eq!(disconnect_code(Some(SdkFailure::ParticipantRemoved)), "rtc_livekit_participant_removed");
}

#[test]
fn user_id_prefers_metadata_then_identity() {
    assert_eq!(user_id_for(r#"{"userId":"u1"}"#, "ident"), Some("u1".to_owned()));
    assert_eq!(user_id_for(r#"{"user_id":"u2"}"#, "ident"), Some("u2".to_owned()));
    assert_eq!(user_id_for(r#"{"userId":"","user_id":"u3"}"#, "ident"), Some("u3".to_owned()));
    assert_eq!(user_id_for("not json", "ident"), Some("ident".to_owned()));
    assert_eq!(user_id_for("", ""), None);
}

#[test]
fn connected_is_told_once_and_identity_is_checked() {
    let mut session = connected_session();
    assert!(session
        .receive(RoomCallback::Connected {
            local_identity: "ident-1".to_owned()
        })
        .is_empty());

    let mut other = MediaSession::new(&request());
    let out = other.receive(RoomCallback::Connected {
        local_identity: "someone-else".to_owned(),
    });
    assert_eq!(
        out,
        vec![SessionOutput::Ended {
            code: CODE_IDENTITY_MISMATCH,
            established: false
        }]
    );
    assert!(other.receive(RoomCallback::TracksChanged).is_empty());
}

#[test]
fn reconnect_edges_are_told_once_each() {
    let mut session = connected_session();
    assert!(session.receive(RoomCallback::Reconnected).is_empty());
    assert_eq!(session.receive(RoomCallback::Reconnecting), vec![SessionOutput::Reconnecting]);
    assert!(session.receive(RoomCallback::Reconnecting).is_empty());
    assert_eq!(session.receive(RoomCallback::Reconnected), vec![SessionOutput::Reconnected]);
    assert_eq!(session.phase, SessionPhase::Connected);
}

#[test]
fn early_connect_failure_is_noted_not_ended() {
    let mut session = MediaSession::new(&request());
    let out = session.receive(RoomCallback::ConnectFailed(Some(SdkFailure::Network)));
    assert_eq!(out, vec![SessionOutput::ConnectFailureNoted(CODE_NETWORK_FAILED)]);
    assert_eq!(session.phase, SessionPhase::Connecting);

    let mut live = connected_session();
    let out = live.receive(RoomCallback::Disconnected(None));
    assert_eq!(
        out,
        vec![SessionOutput::Ended {
            code: CODE_LINK_LOST,
            established: true
        }]
    );
}

#[test]
fn late_departure_of_an_old_identity_is_ignored() {
    let mut session = connected_session();
    let old = person("ident-old", r#"{"userId":"u1"}"#);
    let new = person("ident-new", r#"{"userId":"u1"}"#);
    let out = session.receive(RoomCallback::ParticipantLeft {
        left: old.clone(),
        remaining: vec![new],
    });
    assert!(out.is_empty());
    let out = session.receive(RoomCallback::ParticipantLeft {
        left: old,
        remaining: Vec::new(),
    });
    assert_eq!(out, vec![SessionOutput::ParticipantLeft { user_id: "u1".to_owned() }]);
}

#[test]
fn joined_without_any_identity_is_a_failure() {
    let mut session = connected_session();
    let out = session.receive(RoomCallback::ParticipantJoined(person("", "")));
    assert_eq!(out, vec![SessionOutput::Failure(CODE_PARTICIPANT_IDENTITY_MISSING)]);
}

#[test]
fn speakers_are_capped_and_clamped() {
    let mut session = connected_session();
    let speakers = (0..20).map(|index| (person(&format!("p{index}"), ""), 1.5)).collect();
    let out = session.receive(RoomCallback::SpeakersChanged(speakers));
    let SessionOutput::Speakers(samples) = &out[0] else {
        panic!("speakers expected, got {out:?}");
    };
    assert_eq!(samples.len(), SPEAKER_SAMPLE_LIMIT);
    assert!(samples.iter().all(|(_, level)| *level == 1.0));
    assert_eq!(speaker_flush_interval(true), Duration::from_millis(140));
}

#[test]
fn only_screen_share_unpublish_is_checked() {
    let mut session = connected_session();
    assert_eq!(
        session.receive(RoomCallback::LocalTrackUnpublished(TrackSource::ScreenShare)),
        vec![SessionOutput::CheckScreenShareTeardown]
    );
    assert!(session.receive(RoomCallback::LocalTrackUnpublished(TrackSource::Camera)).is_empty());
}

#[test]
fn deafen_outranks_every_gain() {
    let mut playout = RemotePlayout {
        deafened: false,
        gains: HashMap::from([("u1".to_owned(), 2.0), ("bad".to_owned(), f64::NAN)]),
    };
    assert_eq!(playout.volume_for(Some("u1")), 2.0);
    assert_eq!(playout.volume_for(Some("u2")), 1.0);
    assert_eq!(playout.volume_for(Some("bad")), 1.0);
    assert_eq!(playout.volume_for(None), 1.0);
    assert!(playout.resolves_per_listener());
    playout.deafened = true;
    assert_eq!(playout.volume_for(Some("u1")), 0.0);
    assert!(!playout.resolves_per_listener());
}

#[test]
fn ceiling_from_wire_and_pressure() {
    assert_eq!(VideoCeiling::from_wire(None, Some(60)), VideoCeiling::FREE);
    assert_eq!(VideoCeiling::from_wire(Some(0), Some(60)), VideoCeiling::FREE);
    let premium = VideoCeiling::from_wire(Some(1080), Some(60));
    assert_eq!(premium.under_pressure(true), VideoCeiling { max_height: 1080, max_fps: 30 });
    assert_eq!(premium.under_pressure(false), premium);
}

#[test]
fn camera_shelves_follow_the_tier() {
    let free = camera_shelf(VideoCeiling::FREE);
    assert_eq!((free.height, free.fps, free.encoding), (720, 30, None));

    let premium = camera_shelf(VideoCeiling { max_height: 1080, max_fps: 60 });
    assert_eq!((premium.height, premium.fps), (1080, 60));
    assert_eq!(
        premium.encoding,
        Some(VideoEncoding {
            max_bitrate: 4_500_000,
            max_fps: 60
        })
    );

    let low = camera_shelf(VideoCeiling { max_height: 200, max_fps: 15 });
    assert_eq!((low.height, low.fps), (360, 15));
}

#[test]
fn screen_share_shelves_follow_quality_ceiling_and_display() {
    let premium = VideoCeiling { max_height: 1440, max_fps: 30 };
    let wide = screen_share_shelf(ScreenShareQuality::High, premium, (2560, 1600));
    assert_eq!((wide.height, wide.fps), (1440, 30));
    assert_eq!(wide.encoding.map(|e| e.max_bitrate), Some(6_000_000));

    let small_display = screen_share_shelf(ScreenShareQuality::High, premium, (1920, 1080));
    assert_eq!(small_display.height, 1080);
    assert_eq!(small_display.encoding.map(|e| e.max_bitrate), Some(4_500_000));

    let free = screen_share_shelf(ScreenShareQuality::High, VideoCeiling::FREE, (3840, 2160));
    assert_eq!((free.height, free.fps), (720, 30));
    assert_eq!(free.encoding.map(|e| e.max_bitrate), Some(1_700_000));

    let standard = screen_share_shelf(ScreenShareQuality::Standard, premium, (3840, 2160));
    assert_eq!(standard.height, 720);
}

#[test]
fn screen_share_liveness_waits_for_the_first_frame_only() {
    assert_eq!(screen_share_liveness(0, Duration::from_secs(3)), ScreenShareLiveness::Waiting);
    assert_eq!(screen_share_liveness(0, Duration::from_secs(10)), ScreenShareLiveness::DeadPipe);
    assert_eq!(screen_share_liveness(1, Duration::from_secs(60)), ScreenShareLiveness::Alive);
}

#[test]
fn voice_chain_is_software_on_every_desktop() {
    for host in HOSTS {
        let off = AudioProcessingSettings::default().options_for(host);
        assert_eq!(off.echo_cancellation_mode, ProcessingMode::Software);
        assert!(off.echo_cancellation && off.auto_gain_control && off.noise_suppression);
        assert!(!off.highpass_filter);
        let on = AudioProcessingSettings {
            software_noise_suppression: true,
            input_sensitivity: 0.0,
        }
        .options_for(host);
        assert!(on.highpass_filter);
        assert_eq!(on.noise_suppression_mode, ProcessingMode::Software);
    }
}

#[test]
fn sensitivity_maps_to_threshold() {
    assert_eq!(normalize_sensitivity(f64::NAN), 0.0);
    assert_eq!(normalize_sensitivity(3.0), 1.0);
    assert_eq!(gate_threshold_db(0.005), None);
    assert_eq!(gate_threshold_db(1.0), Some(-15.0));
    let half = gate_threshold_db(0.5).expect("engaged");
    assert!((half - -38.5).abs() < 1e-4);
}

fn block(amplitude: f32, frames: usize) -> Vec<f32> {
    vec![amplitude; frames]
}

#[test]
fn gate_opens_holds_and_releases() {
    let mut gate = InputNoiseGate::new(0.5);
    gate.initialize(48_000);
    let mut loud = block(3_000.0, 480);
    gate.process(&mut loud, 1);
    assert!(gate.is_open());
    assert_eq!(loud[0], 0.0);
    assert!(loud[1..].iter().all(|sample| *sample > 0.0));
    assert!(loud[479] > loud[1]);

    let mut quiet_blocks = 0;
    let mut quiet = block(30.0, 480);
    while gate.is_open() && quiet_blocks < 40 {
        quiet = block(30.0, 480);
        gate.process(&mut quiet, 1);
        quiet_blocks += 1;
    }
    assert!((25..=26).contains(&quiet_blocks), "closed after {quiet_blocks} blocks");
    assert!(quiet[479] < quiet[0]);

    for _ in 0..20 {
        let mut quiet = block(30.0, 480);
        gate.process(&mut quiet, 1);
    }
    let mut silent = block(30.0, 480);
    gate.process(&mut silent, 1);
    assert!(silent.iter().all(|sample| *sample == 0.0));
}

#[test]
fn open_gate_passes_audio_untouched() {
    let mut gate = InputNoiseGate::new(0.0);
    assert!(!gate.is_engaged());
    let mut samples = block(10.0, 960);
    gate.process(&mut samples, 2);
    assert!(samples.iter().all(|sample| *sample == 10.0));
}

#[test]
fn indicator_level_spans_minus_sixty_to_zero() {
    assert_eq!(indicator_level(&[]), 0.0);
    assert!(indicator_level(&block(32_767.0, 10)) > 0.999);
    assert!(indicator_level(&block(10.0, 10)) < 0.05);
}

fn device(id: &str, is_default: bool) -> AudioDevice {
    AudioDevice {
        id: id.to_owned(),
        name: id.to_owned(),
        direction: DeviceDirection::Input,
        is_default,
    }
}

#[test]
fn chosen_device_falls_back_to_default_then_first() {
    let devices = vec![device("usb", false), device("built-in", true)];
    assert_eq!(resolve_device(&devices, Some("usb")).map(|d| d.id.as_str()), Some("usb"));
    assert_eq!(resolve_device(&devices, Some("gone")).map(|d| d.id.as_str()), Some("built-in"));
    assert_eq!(resolve_device(&devices, None).map(|d| d.id.as_str()), Some("built-in"));
    let no_default = vec![device("a", false)];
    assert_eq!(resolve_device(&no_default, None).map(|d| d.id.as_str()), Some("a"));
    assert_eq!(resolve_device(&[], None), None);
}

#[test]
fn pickable_devices_hide_the_names_of_the_system_choice() {
    let devices = [device("default", true), device("pulse", false), device("pipewire", false), device("HDA Intel PCH", false)];
    let pickable: Vec<&str> = pickable_devices(&devices).iter().map(|device| device.id.as_str()).collect();
    assert_eq!(pickable, ["HDA Intel PCH"]);
}

#[test]
fn unplugging_the_chosen_device_switches_to_default() {
    let before = vec![device("usb", false), device("built-in", true)];
    assert_eq!(device_switch(&before, Some("usb"), Some("usb")), DeviceSwitch::Keep);
    let after = vec![device("built-in", true)];
    assert_eq!(
        device_switch(&after, Some("usb"), Some("usb")),
        DeviceSwitch::SwitchTo("built-in".to_owned())
    );
    assert_eq!(device_switch(&[], Some("usb"), Some("usb")), DeviceSwitch::NoDevice);
}

struct DefaultHost;

impl RtcMediaEngine for DefaultHost {}

#[test]
fn the_default_branch_opens_the_default_audio_host() {
    assert_eq!(DefaultHost.audio_host_id(), cpal::default_host().id());
    assert_eq!(intent_name(&MediaIntent::Connect(request())), "connect");
    assert_eq!(intent_name(&MediaIntent::Disconnect(key("conn-1"))), "disconnect");
}

#[test]
fn the_session_key_comes_from_the_join_credentials() {
    assert_eq!(request().session_key(), key("conn-1"));
    let debug = format!("{:?}", request());
    assert!(!debug.contains("very-secret-token"));
    assert!(!debug.contains("secret@"));
}

#[test]
fn an_intent_acts_only_on_the_intended_connected_session() {
    let ours = key("conn-1");
    let other = key("conn-2");
    let live = |phase| Some(ActiveSession { key: &ours, phase });
    assert_eq!(require_session(Some(&ours), live(SessionPhase::Connected), &ours), Ok(()));
    assert_eq!(require_session(Some(&ours), live(SessionPhase::Reconnecting), &ours), Ok(()));
    for (intended, active, asked) in [
        (Some(&ours), live(SessionPhase::Connecting), &ours),
        (Some(&ours), live(SessionPhase::Ended), &ours),
        (Some(&ours), live(SessionPhase::Connected), &other),
        (Some(&other), live(SessionPhase::Connected), &ours),
        (None, live(SessionPhase::Connected), &ours),
        (Some(&ours), None, &ours),
    ] {
        let answer = require_session(intended, active, asked);
        assert_eq!(answer, Err(RtcUnavailable::NotConnected));
        assert_eq!(answer.unwrap_err().code(), Some(CODE_MISSING_ACTIVE_ROOM));
    }
}

#[test]
fn a_repeated_connect_of_the_same_session_is_not_a_new_room() {
    let ours = key("conn-1");
    let other = key("conn-2");
    let live = |phase| Some(ActiveSession { key: &ours, phase });
    assert_eq!(connect_step(Some(&ours), live(SessionPhase::Connected), &ours), ConnectStep::AlreadyConnected);
    assert_eq!(connect_step(Some(&ours), live(SessionPhase::Reconnecting), &ours), ConnectStep::AlreadyConnected);
    assert_eq!(connect_step(Some(&ours), live(SessionPhase::Connecting), &ours), ConnectStep::InProgress);
    assert_eq!(connect_step(Some(&ours), None, &ours), ConnectStep::InProgress);
    assert_eq!(connect_step(Some(&ours), live(SessionPhase::Ended), &ours), ConnectStep::Start);
    assert_eq!(connect_step(Some(&ours), live(SessionPhase::Connected), &other), ConnectStep::Start);
    assert_eq!(connect_step(None, None, &ours), ConnectStep::Start);
    assert_eq!(RtcUnavailable::Refused(CODE_CONNECT_IN_PROGRESS).code(), Some("rtc_media_connect_in_progress"));
}

#[test]
fn a_late_disconnect_of_an_ended_call_leaves_the_next_one_alone() {
    let old = key("conn-1");
    let new = key("conn-2");
    assert!(!owns_session(Some(&new), Some(&new), &old));
    assert!(owns_session(Some(&new), None, &new));
    assert!(owns_session(None, Some(&old), &old));
    assert!(!owns_session(None, None, &old));
}

#[test]
fn data_packets_are_refused_with_the_ios_code() {
    assert_eq!(CODE_DATA_PUBLISH_DISABLED, "rtc_livekit_data_publish_disabled");
    assert_eq!(RtcUnavailable::Refused(CODE_DATA_PUBLISH_DISABLED).code(), Some(CODE_DATA_PUBLISH_DISABLED));
    assert_eq!(RtcUnavailable::NoRuntime(NO_RUNTIME).code(), None);
}

#[test]
fn sdk_error_text_maps_to_the_ios_failure_kinds() {
    let cases = [
        ("engine: signal failure: failed to parse the url: relative URL without a base", SdkFailure::InvalidUrl),
        ("engine: signal failure: access token has invalid characters", SdkFailure::TokenRejected),
        ("engine: signal failure: client error: 401 Unauthorized - invalid token", SdkFailure::TokenRejected),
        ("engine: signal failure: client error: 403 Forbidden - no grant", SdkFailure::TokenRejected),
        (
            "engine: signal failure: server sent leave during reconnect: reason=DuplicateIdentity, action=Disconnect",
            SdkFailure::DuplicateIdentity,
        ),
        ("request error: NotAllowed - publishing is not permitted", SdkFailure::InsufficientPermissions),
        ("engine: signal failure: validate request timeout", SdkFailure::TimedOut),
        ("engine: signal failure: ws failure: IO error: connection refused", SdkFailure::Network),
        ("engine: connection error: could not establish pc connection", SdkFailure::Network),
        ("already closed", SdkFailure::Cancelled),
        ("this track or a track of the same source is already published", SdkFailure::Other),
    ];
    for (text, expected) in cases {
        assert_eq!(classify_sdk_error(text), expected, "{text}");
    }
}

#[test]
fn server_disconnect_reasons_keep_their_codes() {
    assert_eq!(disconnect_failure("DUPLICATE_IDENTITY"), Some(SdkFailure::DuplicateIdentity));
    assert_eq!(disconnect_failure("ParticipantRemoved"), Some(SdkFailure::ParticipantRemoved));
    assert_eq!(disconnect_failure("ROOM_DELETED"), Some(SdkFailure::RoomDeleted));
    assert_eq!(disconnect_failure("ROOM_CLOSED"), Some(SdkFailure::RoomDeleted));
    assert_eq!(disconnect_failure("JOIN_FAILURE"), Some(SdkFailure::JoinFailure));
    assert_eq!(disconnect_failure("CLIENT_INITIATED"), Some(SdkFailure::Cancelled));
    assert_eq!(disconnect_failure("SIGNAL_CLOSE"), None);
    assert_eq!(disconnect_code(disconnect_failure("UNKNOWN_REASON")), CODE_LINK_LOST);
}

#[test]
fn the_mixer_plays_each_person_at_their_volume() {
    let mut mixer = PlayoutMixer::new(PlayoutMixer::capacity_for(48_000, 2, 200));
    mixer.add_track("a", Some("alice".to_owned()));
    mixer.add_track("b", Some("bob".to_owned()));
    mixer.push_i16("a", &[16_384, 16_384]);
    mixer.push_i16("b", &[8_192]);
    mixer.push_i16("unknown", &[32_767]);
    let mut playout = RemotePlayout::default();
    playout.gains.insert("bob".to_owned(), 2.0);
    let mut out = [1.0f32; 3];
    mixer.mix_into(&mut out, &playout);
    assert!((out[0] - 1.0).abs() < 1e-6, "0.5 + 0.25 * 2 = 1.0, got {}", out[0]);
    assert!((out[1] - 0.5).abs() < 1e-6);
    assert_eq!(out[2], 0.0, "an empty queue plays silence");
    assert_eq!(mixer.queued("a"), 0);
}

#[test]
fn deafen_silences_the_mix_and_the_queue_is_bounded() {
    let mut mixer = PlayoutMixer::new(4);
    mixer.add_track("a", None);
    mixer.push_i16("a", &[1, 2, 3, 4, 5, 6]);
    assert_eq!(mixer.queued("a"), 4, "the oldest samples go first");
    let playout = RemotePlayout {
        deafened: true,
        gains: HashMap::new(),
    };
    let mut out = [0.5f32; 4];
    mixer.mix_into(&mut out, &playout);
    assert_eq!(out, [0.0; 4]);
    mixer.add_track("b", None);
    mixer.push_i16("b", &[32_767, 32_767]);
    mixer.push_i16("a", &[32_767, 32_767]);
    let mut loud = [0.0f32; 2];
    mixer.mix_into(&mut loud, &RemotePlayout::default());
    assert_eq!(loud, [1.0, 1.0], "the sum is clipped");
    mixer.remove_track("a");
    assert_eq!(mixer.track_count(), 1);
}

#[test]
fn connect_refuses_missing_credentials_as_a_validation() {
    let now = 1_700_000_000_000;
    assert_eq!(validate_connect(&request(), now), Ok(()));
    let blanks: [fn(&mut ConnectRequest); 5] = [
        |r| r.room_id.clear(),
        |r| r.connection_id.clear(),
        |r| r.participant_identity.clear(),
        |r| r.server_url.clear(),
        |r| r.access_token.clear(),
    ];
    for blank in blanks {
        let mut broken = request();
        blank(&mut broken);
        let refused = validate_connect(&broken, now);
        assert_eq!(refused, Err(RtcUnavailable::Invalid(CODE_INVALID_JOIN_CREDENTIALS)));
        assert_eq!(refused.unwrap_err().code(), Some("rtc_invalid_join_credentials"));
    }
}

#[test]
fn connect_refuses_expired_credentials_with_the_ios_code() {
    let mut expiring = request();
    expiring.expires_at_ms = 1_700_000_000_000;
    assert_eq!(validate_connect(&expiring, 1_699_999_999_999), Ok(()));
    for now in [1_700_000_000_000, 1_700_000_000_001] {
        let refused = validate_connect(&expiring, now);
        assert_eq!(refused, Err(RtcUnavailable::Refused(CODE_JOIN_CREDENTIALS_EXPIRED)));
        assert_eq!(refused.unwrap_err().code(), Some("rtc_join_credentials_expired"));
    }
    let mut both = expiring.clone();
    both.access_token.clear();
    assert_eq!(validate_connect(&both, 1_800_000_000_000), Err(RtcUnavailable::Invalid(CODE_INVALID_JOIN_CREDENTIALS)));
}

#[test]
fn connect_debug_keeps_the_token_out_and_the_expiry_in() {
    let text = format!("{:?}", request());
    assert!(text.contains("4102444800000"));
    assert!(!text.contains("very-secret-token"));
}

#[test]
fn microphone_monitor_resamples_to_ten_millisecond_frames() {
    let mut monitor = MicrophoneMonitor::new(24_000, 2);
    let block: Vec<f32> = (0..480).flat_map(|index| [index as f32, index as f32]).collect();
    monitor.push(&block);
    let frame = monitor.next_frame().expect("a 20 ms block at 24 kHz fills one 48 kHz frame minus the tail");
    assert_eq!(frame.len(), MicrophoneMonitor::FRAME_SAMPLES);
    assert_eq!(frame[0], 0.0);
    assert!((frame[1] - 0.5).abs() < 1e-4);
    assert!((frame[2] - 1.0).abs() < 1e-4);
    assert!(monitor.next_frame().is_none());
}

#[test]
fn microphone_monitor_keeps_the_rate_at_forty_eight_kilohertz() {
    let mut monitor = MicrophoneMonitor::new(MICROPHONE_MONITOR_RATE, 1);
    let block: Vec<f32> = (0..MicrophoneMonitor::FRAME_SAMPLES + 1).map(|index| index as f32).collect();
    monitor.push(&block);
    let frame = monitor.next_frame().expect("one frame");
    assert_eq!(frame[..3], [0.0, 1.0, 2.0]);
    assert_eq!(MicrophoneMonitor::to_stereo(&[3, -4]), vec![3, 3, -4, -4]);
}

#[test]
fn microphone_test_state_round_trips() {
    for state in [MicrophoneTestState::Off, MicrophoneTestState::Starting, MicrophoneTestState::Running, MicrophoneTestState::Failed] {
        assert_eq!(MicrophoneTestState::from_raw(state.raw()), state);
    }
    assert!(MicrophoneTestState::Starting.is_active());
    assert!(!MicrophoneTestState::Failed.is_active());
}
