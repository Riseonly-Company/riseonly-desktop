use std::collections::HashMap;

use gpui::TestAppContext;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode};

use crate::app::router::navigation_types::RootRoute;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_errors::RiseFailure;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiActiveScreen;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::{RiseonlyUniversalLink, UniversalLinkParser};
use crate::core::stores::http::http_service::HttpService;
use crate::core::widgets::sheets::share_sheet::{
    MusicSharePlaylistContext, MusicTrackSharePolicy, SharePayload, ShareSheetMediaItem,
};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::shared::utils::music::music_media_url::MusicMediaURL;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_playback_url::MusicPlaybackUrl;
use crate::modules::music::stores::music_actions::music_types::{MusicPlaylistTrackLinkError, MusicTrackLinkError};
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

const TRACK_ID: &str = "11111111-2222-4333-8444-555555555555";
const PLAYLIST_ID: &str = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

fn url(string: &str) -> Url {
    Url::from_string(string).expect("test url parses")
}

fn local_playlist(title: &str) -> MusicSharePlaylistContext {
    MusicSharePlaylistContext { id: PLAYLIST_ID.to_owned(), title: title.to_owned(), visibility: "local".to_owned() }
}

fn track(status: &str, title: &str) -> TrackModel {
    TrackModel {
        id: TRACK_ID.to_owned(),
        title: title.to_owned(),
        artist: "Rise".to_owned(),
        artist_id: String::new(),
        album_title: String::new(),
        album_id: String::new(),
        duration_seconds: 180,
        cover_colors: vec!["#000000".to_owned()],
        cover_image_url: format!("https://staging.riseonly.net/api/file/secure/music/{PLAYLIST_ID}/cover.jpg"),
        audio_url: format!("https://staging.riseonly.net/api/file/secure/music/{PLAYLIST_ID}/audio.mp3"),
        is_exclusive: false,
        status: status.to_owned(),
        description: None,
        source_chat_id: None,
        source_message_id: None,
        artist_is_registered: true,
        remote_audio_url: Some(format!("https://staging.riseonly.net/api/file/secure/music/{PLAYLIST_ID}/audio.mp3")),
        transfer_phase: None,
        transferred_bytes: 0,
        transfer_total_bytes: 0,
        transfer_error: None,
        library_owner_user_id: None,
        playback_cache_id: None,
        is_liked: false,
    }
}

fn published_track() -> TrackModel {
    track("published", "Midnight")
}

fn wire_track(status: &str, cover_url: &str, audio_url: &str) -> RiseStoredMusicTrack {
    RiseStoredMusicTrack {
        id: Some(TRACK_ID.to_owned()),
        title: Some("Midnight".to_owned()),
        description: None,
        duration_ms: Some(180_000),
        cover_url: Some(cover_url.to_owned()),
        audio_url: Some(audio_url.to_owned()),
        play_count: Some(0),
        like_count: Some(0),
        is_liked: Some(false),
        album: None,
        artist: None,
        created_at_ms: Some(1),
        status: Some(status.to_owned()),
        genre: None,
        rejection_reason: None,
        scheduled_at_ms: None,
        is_exclusive: Some(false),
        library_owner_user_id: None,
        ..RiseStoredMusicTrack::default()
    }
}

fn wire_track_with_status(status: &str) -> RiseStoredMusicTrack {
    wire_track(status, "https://cdn.riseonly.net/cover.jpg", "https://cdn.riseonly.net/audio.mp3")
}

fn failure(message: &str) -> anyhow::Error {
    RiseFailure::new(Some("test".to_owned()), message.to_owned(), HashMap::new()).into()
}

fn api_origin_url(path: &str) -> Option<String> {
    let mut expected = UrlComponents::from_url(&HttpService::resolved_api_base_url(), false)?;
    expected.set_path(path);
    expected.set_query(None);
    expected.set_fragment(None);
    expected.url().map(|url| url.absolute_string().to_owned())
}

#[test]
fn canonical_track_links_never_expose_media_hosts() {
    let payload = SharePayload::Track { track: published_track(), playlist: Some(local_playlist("Night Drive")) };

    assert_eq!(
        payload.share_url_for(AppEnvironment::Prod).map(|url| url.absolute_string().to_owned()),
        Some(format!("https://riseonly.net/t/{TRACK_ID}?playlist={PLAYLIST_ID}"))
    );
    assert_eq!(Some(payload.share_text()), payload.share_url().map(|url| url.absolute_string().to_owned()));
    assert!(!payload.share_text().contains("staging.riseonly.net"));
    assert_eq!(payload.link_preview_type(), Some("BIG"));
    assert!(payload.requires_media_permission());
    assert_eq!(payload.in_app_content_type(), "audio");
    assert_eq!(
        payload.in_app_media_items(),
        vec![ShareSheetMediaItem::new(
            "audio".to_owned(),
            format!("/api/file/secure/music/{PLAYLIST_ID}/audio.mp3"),
            Some(format!("/api/file/secure/music/{PLAYLIST_ID}/cover.jpg")),
            "Rise \u{2014} Midnight.mp3".to_owned(),
            "audio/mpeg".to_owned(),
            Some(180),
        )]
    );
}

#[test]
fn canonical_track_url_is_first_for_rich_preview_extraction() {
    let payload = SharePayload::Track {
        track: track("published", "Listen at https://evil.example/track"),
        playlist: Some(local_playlist("https://evil.example/playlist")),
    };
    let canonical = payload.share_url().expect("canonical track url").absolute_string().to_owned();

    assert_eq!(payload.share_text(), canonical);
    assert!(!payload.share_text().contains("https://evil.example/track"));
}

#[test]
fn shared_chat_music_paths_resolve_to_authenticated_api_origin() {
    let asset_id = "99999999-8888-4777-8666-555555555555";
    let path = format!("/api/file/secure/chat/{PLAYLIST_ID}/shared-music/{asset_id}.mp3");
    let expected = api_origin_url(&path);
    assert!(expected.is_some());

    assert_eq!(RemoteMediaURL::optional_normalized(Some(&path)), expected);
}

#[test]
fn chat_attachment_paths_resolve_to_authenticated_api_origin() {
    let asset_id = "99999999-8888-4777-8666-555555555555";

    for path in [
        format!("/api/file/secure/chat/{PLAYLIST_ID}/{asset_id}.jpg"),
        format!("/api/file/secure/chat/{PLAYLIST_ID}/{asset_id}.pdf"),
        format!("/api/file/secure/chat/{PLAYLIST_ID}/thumb_{asset_id}.jpg"),
    ] {
        let expected = api_origin_url(&path);
        assert!(expected.is_some());
        assert_eq!(RemoteMediaURL::optional_normalized(Some(&path)), expected);
    }

    for rejected in [
        format!("/api/file/secure/chat/not-a-uuid/{asset_id}.jpg"),
        format!("/api/file/secure/chat/{PLAYLIST_ID}/{asset_id}"),
        format!("/api/file/secure/chat/{PLAYLIST_ID}/../{asset_id}.jpg"),
        format!("/api/file/secure/chat/{PLAYLIST_ID}/nested/{asset_id}.jpg"),
    ] {
        assert_eq!(RemoteMediaURL::optional_normalized(Some(&rejected)), Some(rejected.clone()));
    }
}

#[test]
fn canonical_resource_links_round_trip_to_app_routes() {
    let track_url = RiseonlyUniversalLink::track(TRACK_ID, Some(PLAYLIST_ID), AppEnvironment::Prod)
        .expect("canonical playlist track link");
    let alias_url = url(&format!("https://riseonly.net/track/{TRACK_ID}?playlist_id={PLAYLIST_ID}"));
    let playlist_url =
        RiseonlyUniversalLink::playlist(PLAYLIST_ID, AppEnvironment::Prod).expect("canonical playlist link");
    let custom_track_url = url(&format!("riseonly://track/{TRACK_ID}?playlist={PLAYLIST_ID}"));
    let playlist_track =
        PushDeepLink::PlaylistTrack { playlist_id: PLAYLIST_ID.to_owned(), track_id: TRACK_ID.to_owned() };
    let music_track = PushDeepLink::MusicTrack { track_id: TRACK_ID.to_owned() };

    assert_eq!(UniversalLinkParser::parse(&track_url, AppEnvironment::Prod), Some(playlist_track.clone()));
    assert_eq!(UniversalLinkParser::parse(&alias_url, AppEnvironment::Prod), Some(playlist_track.clone()));
    let bare_track_url =
        RiseonlyUniversalLink::track(TRACK_ID, None, AppEnvironment::Prod).expect("canonical track link");
    assert_eq!(UniversalLinkParser::parse(&bare_track_url, AppEnvironment::Prod), Some(music_track.clone()));
    assert_eq!(
        UniversalLinkParser::parse(&playlist_url, AppEnvironment::Prod),
        Some(PushDeepLink::Playlist { playlist_id: PLAYLIST_ID.to_owned() })
    );
    assert_eq!(UniversalLinkParser::parse(&custom_track_url, AppEnvironment::Prod), Some(playlist_track));
    assert_eq!(
        UniversalLinkParser::parse(&url(&format!("https://www.riseonly.net/track/{TRACK_ID}")), AppEnvironment::Prod),
        Some(music_track)
    );
}

#[test]
fn trusted_relative_music_media_uses_configured_api_origin() {
    let api_base_url = url("http://192.168.31.177:8080/api");
    let audio_path = format!("/api/file/secure/music/{PLAYLIST_ID}/track.mp3");
    let cover_path = format!("/api/file/secure/music/{PLAYLIST_ID}/cover.webp");

    assert_eq!(
        MusicMediaURL::resolved_url_string_api_base_url(Some(&audio_path), &api_base_url),
        Some(format!("http://192.168.31.177:8080/api/file/secure/music/{PLAYLIST_ID}/track.mp3"))
    );
    assert_eq!(
        MusicMediaURL::cover_url_strings_for_fetch_api_base_url(Some(&cover_path), &api_base_url),
        vec![format!("http://192.168.31.177:8080/api/file/secure/music/{PLAYLIST_ID}/cover.webp")]
    );
}

#[test]
fn music_media_resolver_preserves_absolute_urls() {
    let api_base_url = url("https://api.riseonly.net/api");
    let remote = "https://cdn.riseonly.net/music/track.mp3";
    let local = "file:///tmp/riseonly-track.mp3";

    assert_eq!(MusicMediaURL::resolved_url_string_api_base_url(Some(remote), &api_base_url).as_deref(), Some(remote));
    assert_eq!(MusicMediaURL::resolved_url_string_api_base_url(Some(local), &api_base_url).as_deref(), Some(local));
}

#[test]
fn music_media_resolver_rejects_untrusted_relative_paths() {
    let api_base_url = url("https://api.riseonly.net/api");
    let rejected = [
        "track.mp3".to_owned(),
        "//evil.example/track.mp3".to_owned(),
        "/api/file/secure/music/not-a-uuid/track.mp3".to_owned(),
        format!("/api/file/secure/music/{PLAYLIST_ID}/nested/track.mp3"),
        format!("/api/file/secure/music/{PLAYLIST_ID}/../track.mp3"),
        format!("/api/file/secure/music/{PLAYLIST_ID}/%2e%2e.mp3"),
        format!("/api/file/secure/music/{PLAYLIST_ID}/track.mp3?token=secret"),
        format!("/api/file/secure/music/{PLAYLIST_ID}/track.m4a"),
        format!("/api/files/secure/music/{PLAYLIST_ID}/track.mp3"),
        "javascript:alert(1)".to_owned(),
    ];

    for raw in rejected {
        assert_eq!(MusicMediaURL::resolved_url_string_api_base_url(Some(&raw), &api_base_url), None, "{raw}");
    }
}

#[test]
fn relative_realtime_media_maps_like_absolute_create_ack() {
    let audio_path = format!("/api/file/secure/music/{PLAYLIST_ID}/track.mp3");
    let cover_path = format!("/api/file/secure/music/{PLAYLIST_ID}/cover.webp");
    let api_base_url = HttpService::resolved_api_base_url();
    let audio_url = MusicMediaURL::trusted_private_music_url(&audio_path, &api_base_url)
        .expect("trusted audio path")
        .absolute_string()
        .to_owned();
    let cover_url = MusicMediaURL::trusted_private_music_url(&cover_path, &api_base_url)
        .expect("trusted cover path")
        .absolute_string()
        .to_owned();
    let ack = wire_track("local_library", &cover_url, &audio_url).as_track_model().expect("ack track");
    let realtime = wire_track("local_library", &cover_path, &audio_path).as_track_model().expect("realtime track");

    assert_eq!(realtime.audio_url, ack.audio_url);
    assert_eq!(realtime.cover_image_url, ack.cover_image_url);
}

#[gpui::test]
fn trusted_relative_audio_is_immediately_playable(cx: &mut TestAppContext) {
    let path = format!("/api/file/secure/music/{PLAYLIST_ID}/track.mp3");
    let expected = MusicMediaURL::trusted_private_music_url(&path, &HttpService::resolved_api_base_url())
        .expect("trusted audio path");

    cx.update(|_| {
        assert_eq!(MusicPlaybackUrl::playback_url_candidates_from(&path), vec![expected]);
    });
}

#[test]
fn signed_playlist_invites_remain_distinct_from_playlist_ids() {
    let signature = "a".repeat(64);
    let token = format!("{PLAYLIST_ID}.3.{signature}");
    let invite_url = url(&format!("https://riseonly.net/playlist/{token}"));

    assert_eq!(
        UniversalLinkParser::parse(&invite_url, AppEnvironment::Prod),
        Some(PushDeepLink::PlaylistInvite { token })
    );
}

#[test]
fn malformed_music_links_are_rejected() {
    let lower_signature = "a".repeat(64);
    let upper_signature = "A".repeat(64);
    let urls = [
        "https://riseonly.net/track/not-a-uuid".to_owned(),
        format!("https://riseonly.net/track/{}", PLAYLIST_ID.to_uppercase()),
        format!("https://riseonly.net//track/{TRACK_ID}"),
        format!("https://riseonly.net/track//{TRACK_ID}"),
        format!("https://riseonly.net/track/{TRACK_ID}/"),
        format!("https://riseonly.net/track/{TRACK_ID}/extra"),
        format!("https://riseonly.net/track/{TRACK_ID}?"),
        format!("https://riseonly.net/track/{TRACK_ID}?playlist=not-a-uuid"),
        format!("https://riseonly.net/track/{TRACK_ID}?playlist=%20{PLAYLIST_ID}"),
        format!("https://riseonly.net/track/{TRACK_ID}?playlist=%61aaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"),
        format!("https://riseonly.net/track/{TRACK_ID}?playlist={PLAYLIST_ID}&playlist_id={PLAYLIST_ID}"),
        format!("https://riseonly.net/track/{TRACK_ID}?playlist={PLAYLIST_ID}&playlist={PLAYLIST_ID}"),
        format!("https://riseonly.net/track/{TRACK_ID}?foo=bar"),
        format!("https://riseonly.net/track/{TRACK_ID}?Playlist={PLAYLIST_ID}"),
        format!("https://riseonly.net/track/{TRACK_ID}?PLAYLIST_ID={PLAYLIST_ID}"),
        format!("https://riseonly.net/track/{TRACK_ID}?playlist[]={PLAYLIST_ID}"),
        format!("https://riseonly.net/track/{TRACK_ID}?playlistId={PLAYLIST_ID}"),
        format!("https://user:pass@riseonly.net/track/{TRACK_ID}"),
        format!("https://riseonly.net:443/track/{TRACK_ID}"),
        format!("riseonly://track//{TRACK_ID}"),
        format!("riseonly://track/{TRACK_ID}/"),
        "https://riseonly.net/playlist/not-an-invite".to_owned(),
        format!("https://riseonly.net//playlist/{PLAYLIST_ID}"),
        format!("https://riseonly.net/playlist//{PLAYLIST_ID}"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}/"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}?"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}?foo=bar"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}?playlist={PLAYLIST_ID}"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}.0.{lower_signature}"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}.01.{lower_signature}"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}.1.{upper_signature}"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}.1.short"),
        format!("https://riseonly.net/playlist/{PLAYLIST_ID}/extra"),
    ];

    for raw in urls {
        assert_eq!(UniversalLinkParser::parse(&url(&raw), AppEnvironment::Prod), None, "{raw}");
    }
}

#[test]
fn temporary_track_ids_cannot_produce_share_links() {
    assert_eq!(RiseonlyUniversalLink::track("temp_message_mp3_123", None, AppEnvironment::current()), None);
    assert_eq!(RiseonlyUniversalLink::playlist("local-playlist", AppEnvironment::current()), None);
}

#[test]
fn local_tracks_require_playlist_context_before_sharing() {
    assert!(MusicTrackSharePolicy::can_share(&track("published", "Midnight"), None));
    assert!(!MusicTrackSharePolicy::can_share(&track("local_library", "Midnight"), None));
    assert!(MusicTrackSharePolicy::can_share(&track("local_library", "Midnight"), Some(PLAYLIST_ID)));
    assert!(!MusicTrackSharePolicy::can_share(&track("local_library_pending", "Midnight"), Some(PLAYLIST_ID)));
}

#[test]
fn music_link_routes_expose_stable_navigation_context() {
    assert_eq!(
        RootRoute::MusicTrackLink { track_id: TRACK_ID.to_owned() }.sai_active_screen(),
        SaiActiveScreen::new(
            "MusicTrackLink".to_owned(),
            HashMap::from([("trackId".to_owned(), TRACK_ID.to_owned())]),
        )
    );
    assert_eq!(
        RootRoute::MusicPlaylistTrackLink { playlist_id: PLAYLIST_ID.to_owned(), track_id: TRACK_ID.to_owned() }
            .sai_active_screen(),
        SaiActiveScreen::new(
            "MusicPlaylistTrackLink".to_owned(),
            HashMap::from([
                ("playlistId".to_owned(), PLAYLIST_ID.to_owned()),
                ("trackId".to_owned(), TRACK_ID.to_owned()),
            ]),
        )
    );
}

#[gpui::test]
fn playlist_access_errors_map_to_stable_user_facing_states(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let service = MusicServicesStore::shared(cx);
        let service = service.read(cx);

        assert_eq!(
            service.playlist_track_link_error(&failure("Playlist not found")),
            MusicPlaylistTrackLinkError::PlaylistNotFound
        );
        assert_eq!(
            service.playlist_track_link_error(&failure("This playlist is private")),
            MusicPlaylistTrackLinkError::AccessDenied
        );
        assert_eq!(
            service.playlist_track_link_error(&URLError::new(URLErrorCode::NotConnectedToInternet).into()),
            MusicPlaylistTrackLinkError::LoadFailed
        );
        assert!(service.is_public_link_track(&wire_track_with_status("published")));
        assert!(service.is_public_link_track(&wire_track_with_status("original_audio")));
        assert!(!service.is_public_link_track(&wire_track_with_status("local_library")));
        assert!(!service.is_public_link_track(&wire_track_with_status("approved")));
        assert_eq!(service.track_link_error(&failure("Track not found")), MusicTrackLinkError::NotFound);
        assert_eq!(service.track_link_error(&failure("Permission denied")), MusicTrackLinkError::AccessDenied);
    });
}
