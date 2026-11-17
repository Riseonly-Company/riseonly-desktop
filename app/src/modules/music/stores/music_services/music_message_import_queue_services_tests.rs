use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, TestAppContext};
use rise_canon::rise_key::message as key;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::URLRequest;
use serde_json::{Map, Value};

use crate::modules::chat::components::chat::bubble::bot_message_inline_keyboard::BotInlineKeyboardActivationPolicy;
use crate::modules::chat::components::chat::bubble::chat_audio_track_adapter::ChatAudioTrackAdapter;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{MessagePayloadCodec, MessageWireDTO};
use crate::modules::music::shared::utils::music::music_models::MusicLocalTrackTransferPhase;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::music::shared::utils::music::music_playback_url::MusicPlaybackUrl;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_message_import_types::{
    MusicMessageMP3ImportRequest, MusicMessageMP3ImportSource,
};
use crate::modules::music::stores::music_actions::music_types::MusicLocalPlaylistDestination;
use crate::modules::music::stores::music_actions::music_wire_types::{RiseStoredMusicPlaylist, RiseStoredMusicTrack};
use crate::modules::music::stores::music_services::music_message_import_queue_services::{
    MusicMessageMP3AuthPolicy, MusicMessageMP3LimitedDownloader, MusicMessageMP3ProgressPolicy,
};
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::playlists::message_mp3_import_sheet::MessageMusicImportPresenter;

fn media(id: &str, kind: MessageMediaKind, url: &str) -> MessageMediaItem {
    MessageMediaItem::new(id.to_owned(), kind, url.to_owned())
}

fn named_media(id: &str, kind: MessageMediaKind, url: &str, file_name: &str) -> MessageMediaItem {
    let mut item = media(id, kind, url);
    item.file_name = Some(file_name.to_owned());
    item
}

fn typed_media(id: &str, kind: MessageMediaKind, url: &str, mime_type: &str) -> MessageMediaItem {
    let mut item = media(id, kind, url);
    item.mime_type = Some(mime_type.to_owned());
    item
}

fn message(id: &str, content_type: MessageContentType, media_items: Vec<MessageMediaItem>) -> GetMessageMessage {
    let mut message = GetMessageMessage::new(id.to_owned());
    message.content_type = content_type;
    message.media_items = media_items;
    message
}

fn wire_message(message_id: &str, file: Vec<(&str, Value)>) -> GetMessageMessage {
    let media_item: Map<String, Value> = file.into_iter().map(|(name, value)| (name.to_owned(), value)).collect();
    let mut row = Map::new();
    row.insert(key::ID.to_owned(), Value::from(message_id));
    row.insert(key::CHAT_ID.to_owned(), Value::from("chat-1"));
    row.insert(key::CONTENT_TYPE.to_owned(), Value::from("audio"));
    row.insert(key::MEDIA_ITEMS.to_owned(), Value::Array(vec![Value::Object(media_item)]));
    let payload = Value::Object(row);
    let Some(wire) = MessagePayloadCodec::decode::<MessageWireDTO>(Some(&payload)) else {
        panic!("the row decodes");
    };
    wire.to_message()
}

fn url(raw: &str) -> Url {
    Url::from_string(raw).expect("the test URL parses")
}

fn file_url(path: &Path) -> Url {
    Url::from_file_path(path).expect("the temporary path is absolute")
}

fn temporary_path(prefix: &str, extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}.{extension}", Uuid::new_v4().uuid_string()))
}

/// removes the file on drop so a failed assertion leaves nothing behind
struct RemoveFileOnDrop(PathBuf);

impl Drop for RemoveFileOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn written_file(prefix: &str, extension: &str, bytes: &[u8]) -> RemoveFileOnDrop {
    let path = temporary_path(prefix, extension);
    std::fs::write(&path, bytes).expect("the temporary directory is writable");
    RemoveFileOnDrop(path)
}

fn plain_draft(temp_id: &str, title: &str, artist: &str, duration_ms: i64, audio_url: &Url, cx: &mut App) -> RiseStoredMusicTrack {
    transfer_draft(temp_id, title, artist, duration_ms, audio_url, None, cx)
}

fn transfer_draft(
    temp_id: &str,
    title: &str,
    artist: &str,
    duration_ms: i64,
    audio_url: &Url,
    transfer_phase: Option<MusicLocalTrackTransferPhase>,
    cx: &mut App,
) -> RiseStoredMusicTrack {
    let service = MusicServicesStore::shared(cx);
    service.read(cx).make_local_track_draft(
        temp_id,
        title,
        None,
        artist,
        duration_ms,
        "owner",
        audio_url,
        "",
        None,
        None,
        None,
        transfer_phase,
        0,
        0,
        None,
    )
}

fn local_playlist(tracks: Vec<RiseStoredMusicTrack>) -> RiseStoredMusicPlaylist {
    RiseStoredMusicPlaylist {
        id: Some("playlist".to_owned()),
        owner_user_id: Some("owner".to_owned()),
        title: Some("Server playlist".to_owned()),
        visibility: Some("local".to_owned()),
        tracks: Some(tracks),
        created_at_ms: Some(1),
        updated_at_ms: Some(2),
        ..RiseStoredMusicPlaylist::default()
    }
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn bot_inline_keyboard_requires_deliberate_stationary_press() {
    assert!(BotInlineKeyboardActivationPolicy::MINIMUM_PRESS_DURATION >= 0.25);
    assert!(BotInlineKeyboardActivationPolicy::MAXIMUM_MOVEMENT <= 8.0);
}

#[test]
fn importable_mp3_detector_accepts_audio_and_document_signals() {
    let audio_by_url = typed_media(
        "audio-url",
        MessageMediaKind::Audio,
        "https://cdn.example/music/track.MP3?token=1",
        "application/octet-stream",
    );
    let mut document_by_name = named_media(
        "document-name",
        MessageMediaKind::Document,
        "https://cdn.example/file/opaque",
        "Artist \u{2014} Track.mp3",
    );
    document_by_name.mime_type = Some("application/octet-stream".to_owned());
    let document_by_mime = typed_media(
        "document-mime",
        MessageMediaKind::Document,
        "https://cdn.example/file/opaque",
        "audio/mpeg; charset=binary",
    );

    assert!(audio_by_url.is_importable_mp3());
    assert!(document_by_name.is_importable_mp3());
    assert!(document_by_mime.is_importable_mp3());
}

#[test]
fn importable_mp3_detector_rejects_voice_and_other_audio_formats() {
    let voice = typed_media("voice", MessageMediaKind::Voice, "https://cdn.example/voice.mp3", "audio/mpeg");
    let mut m4a = named_media("m4a", MessageMediaKind::Audio, "https://cdn.example/track.m4a", "track.m4a");
    m4a.mime_type = Some("audio/mp4".to_owned());
    let image = typed_media("image", MessageMediaKind::Image, "https://cdn.example/not-really.mp3", "audio/mpeg");
    let voice_message = message(
        "voice-message",
        MessageContentType::Voice,
        vec![typed_media("legacy-voice", MessageMediaKind::Audio, "https://cdn.example/voice.mp3", "audio/mpeg")],
    );

    assert!(!voice.is_importable_mp3());
    assert!(!m4a.is_importable_mp3());
    assert!(!image.is_importable_mp3());
    assert_eq!(voice_message.first_importable_mp3_item(), None);
}

#[test]
fn message_finds_mp3_document_even_when_content_type_is_document() {
    let document =
        named_media("document-mp3", MessageMediaKind::Document, "https://cdn.example/file/123", "Bot Track.mp3");
    let message = message("document-message", MessageContentType::Document, vec![document.clone()]);

    assert_eq!(message.first_importable_mp3_item(), Some(&document));
}

#[test]
fn message_owner_type_recovers_legacy_bot_mp3_media_kind() {
    let legacy = named_media("legacy-file", MessageMediaKind::Image, "https://cdn.example/file/opaque", "Bot Track.mp3");
    let audio_message = message("legacy-audio-message", MessageContentType::Audio, vec![legacy.clone()]);
    let document_message = message("legacy-document-message", MessageContentType::Document, vec![legacy.clone()]);

    assert_eq!(audio_message.first_importable_mp3_item(), Some(&legacy));
    assert_eq!(document_message.first_importable_mp3_item(), Some(&legacy));
}

#[test]
fn wire_file_kind_normalizes_from_owning_audio_content_type() {
    let message = wire_message(
        "legacy-bot-message",
        vec![
            (key::FILE_ID, Value::from("file-1")),
            (key::FILE_URL, Value::from("https://cdn.example/file/opaque")),
            (key::FILE_NAME, Value::from("Legacy Bot Track.mp3")),
            (key::MEDIA_TYPE, Value::from("file")),
        ],
    );

    assert_eq!(message.media_items.first().map(|item| &item.kind), Some(&MessageMediaKind::Audio));
    assert_eq!(message.first_importable_mp3_item().map(|item| item.id.as_str()), Some("file-1"));
}

#[gpui::test]
fn message_presenter_opens_for_message_validated_legacy_mp3(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let presenter = MessageMusicImportPresenter::shared(cx);
        presenter.update(cx, |presenter, cx| presenter.dismiss(cx));
        let legacy =
            named_media("legacy-file", MessageMediaKind::Image, "https://cdn.example/file/opaque", "Bot Track.mp3");
        let mut message = message("legacy-message", MessageContentType::Audio, vec![legacy.clone()]);
        message.chat_id = "chat".to_owned();

        presenter.update(cx, |presenter, cx| presenter.present(&message, cx));

        assert!(presenter.read(cx).is_presented());
        assert_eq!(presenter.read(cx).source().map(|source| source.media_id.as_str()), Some(legacy.id.as_str()));
        presenter.update(cx, |presenter, cx| presenter.dismiss(cx));
    });
}

#[test]
fn message_mp3_import_request_clamps_backend_bounded_metadata() {
    let source = MusicMessageMP3ImportSource {
        message_id: "message".to_owned(),
        chat_id: "chat".to_owned(),
        media_id: "media".to_owned(),
        remote_url: "https://cdn.example/track.mp3".to_owned(),
        local_preview_url: None,
        thumbnail_url: None,
        file_name: Some("track.mp3".to_owned()),
        file_size: None,
        mime_type: Some("audio/mpeg".to_owned()),
        duration_seconds: Some(30.0),
        caption: String::new(),
        suggested_title: String::new(),
        suggested_artist: String::new(),
    };
    let destination = MusicLocalPlaylistDestination {
        id: "playlist".to_owned(),
        title: "Playlist".to_owned(),
        description: String::new(),
        cover_url: String::new(),
        owner_user_id: "user".to_owned(),
        viewer_role: "owner".to_owned(),
        can_add_tracks: true,
        updated_at_ms: 0,
        revision: 1,
    };
    let request = MusicMessageMP3ImportRequest::new(
        source,
        destination,
        &"\u{1F3B5}".repeat(150),
        Some("d".repeat(1_100).as_str()),
        &"a".repeat(250),
        None,
    );

    assert!(request.title.encode_utf16().count() <= 200);
    assert_eq!(request.artist_name.encode_utf16().count(), 200);
    assert_eq!(request.description.as_deref().map(|text| text.encode_utf16().count()), Some(1_000));
}

#[test]
fn message_mp3_auth_uses_exact_https_origin() {
    let base = url("https://api.riseonly.example:443/v1");

    assert!(MusicMessageMP3AuthPolicy::is_trusted_origin(
        Some(&url("https://api.riseonly.example/music/file.mp3")),
        Some(&base),
    ));
    assert!(!MusicMessageMP3AuthPolicy::is_trusted_origin(
        Some(&url("https://api.riseonly.example:8443/music/file.mp3")),
        Some(&base),
    ));
    assert!(!MusicMessageMP3AuthPolicy::is_trusted_origin(
        Some(&url("http://api.riseonly.example/music/file.mp3")),
        Some(&base),
    ));
    assert!(!MusicMessageMP3AuthPolicy::is_trusted_origin(
        Some(&url("https://cdn.riseonly.example/music/file.mp3")),
        Some(&base),
    ));
}

#[test]
fn already_cancelled_bounded_download_completes_with_cancellation() {
    let request = URLRequest::new(url("https://127.0.0.1/never"));
    let downloader = Arc::new(MusicMessageMP3LimitedDownloader::new(&request, 1, "mp3", None, None));
    let task = RiseAccountRuntime::current().spawn(async move { downloader.download(request).await });
    task.abort();

    let result = futures::executor::block_on(task);
    assert!(!matches!(result, Ok(Ok(_))), "A pre-cancelled download must not succeed");
}

#[test]
fn audio_metadata_parses_telegram_style_dash() {
    let mut item = named_media(
        "audio-1",
        MessageMediaKind::Audio,
        "https://cdn.example/track.mp3",
        "\u{D6}sterreich \u{2014} sayonara.mp3",
    );
    item.duration = Some(91.0);

    let (title, artist) = ChatAudioTrackAdapter::title_and_artist(&item, "");

    assert_eq!(title, "sayonara");
    assert_eq!(artist, "\u{D6}sterreich");
}

#[test]
fn audio_metadata_parses_ascii_dash_and_keeps_remaining_title() {
    let item = named_media(
        "audio-2",
        MessageMediaKind::Audio,
        "https://cdn.example/track.m4a",
        "maebi - the lobotomy - slowed.m4a",
    );

    let (title, artist) = ChatAudioTrackAdapter::title_and_artist(&item, "");

    assert_eq!(title, "the lobotomy - slowed");
    assert_eq!(artist, "maebi");
}

#[test]
fn chat_track_carries_origin_and_does_not_pretend_artist_is_registered() {
    let mut item = named_media(
        "audio-3",
        MessageMediaKind::Audio,
        "https://cdn.example/mock.mp3",
        "Music Bot \u{2014} Mock Track.mp3",
    );
    item.thumbnail_url = Some("https://cdn.example/cover.jpg".to_owned());
    item.duration = Some(1.0);

    let track = ChatAudioTrackAdapter::track("message-7", None, "chat-3", &item, "");

    assert_eq!(track.source_chat_id.as_deref(), Some("chat-3"));
    assert_eq!(track.source_message_id.as_deref(), Some("message-7"));
    assert_eq!(track.audio_url, item.url);
    assert_eq!(Some(track.cover_image_url.as_str()), item.thumbnail_url.as_deref());
    assert_eq!(track.duration_seconds, 1);
    assert!(!track.artist_is_registered);
}

#[test]
fn audio_content_normalizes_legacy_voice_media_and_keeps_remote_playback_url() {
    let message = wire_message(
        "message-audio",
        vec![
            (key::FILE_ID, Value::from("file-1")),
            (key::FILE_URL, Value::from("https://cdn.riseonly.net/public/chat/song.mp3")),
            (key::FILE_NAME, Value::from("\u{D6}sterreich \u{2014} sayonara.mp3")),
            (key::MEDIA_TYPE, Value::from("voice")),
            (key::MIME_TYPE, Value::from("audio/mpeg")),
            (key::DURATION, Value::from(258)),
        ],
    );
    let item = message.first_audio_item().expect("the audio row keeps its item");

    assert_eq!(message.first_voice_item(), None);
    assert_eq!(item.kind, MessageMediaKind::Audio);
    assert_eq!(item.url, "https://cdn.riseonly.net/public/chat/song.mp3");
}

#[test]
fn uuid_filename_falls_back_to_caption_metadata() {
    let item = named_media(
        "audio-4",
        MessageMediaKind::Audio,
        "https://cdn.example/mock.mp3",
        "60c426b3-d81d-4686-afc2-b220c2cc524c",
    );

    let (title, artist) =
        ChatAudioTrackAdapter::title_and_artist(&item, "\u{1F3B5} sayonara\n\u{1F464} \u{D6}sterreich");

    assert_eq!(title, "sayonara");
    assert_eq!(artist, "\u{D6}sterreich");
}

#[test]
fn legacy_cdn_filename_falls_back_to_music_bot_caption() {
    let item = named_media(
        "audio-cdn-name",
        MessageMediaKind::Audio,
        "https://cdn.riseonly.net/public/music/song.mp3",
        "https://cdn.riseonly.net/public/music/ca19cf9ba7f81878edd0555cd9d7993dfd4.mp3",
    );

    let (title, artist) =
        ChatAudioTrackAdapter::title_and_artist(&item, "\u{1F3B5} lovepoison (Slowed)\n\u{1F464} HRXSTAL");

    assert_eq!(title, "lovepoison (Slowed)");
    assert_eq!(artist, "HRXSTAL");
}

#[gpui::test]
fn local_optimistic_audio_url_is_immediately_playable(cx: &mut TestAppContext) {
    let file = written_file("riseonly-audio", "mp3", &[0x49, 0x44, 0x33, 0x04]);
    let local_url = file_url(&file.0);

    let draft = cx.update(|cx| plain_draft("temp-track", "Track", "Artist", 1_000, &local_url, cx));
    let track = draft.as_track_model().expect("the draft has an id");

    assert_eq!(track.audio_url, local_url.absolute_string());
    assert_eq!(MusicPlaybackUrl::playback_url_candidates_for(&track), vec![local_url]);
}

#[gpui::test]
fn local_optimistic_track_keeps_edited_metadata_cover_and_exact_progress(cx: &mut TestAppContext) {
    let audio_url = file_url(Path::new("/tmp/riseonly-edited-track.mp3"));
    let cover_url = file_url(Path::new("/tmp/riseonly-edited-cover.jpg"));
    let draft = cx.update(|cx| {
        let service = MusicServicesStore::shared(cx);
        service.read(cx).make_local_track_draft(
            "temp-edited-track",
            "Edited title",
            Some("Edited description"),
            "Edited artist",
            93_000,
            "owner",
            &audio_url,
            cover_url.absolute_string(),
            Some("chat-42"),
            Some("message-7"),
            Some("https://cdn.example/original.mp3"),
            Some(MusicLocalTrackTransferPhase::Uploading),
            524_288,
            1_048_576,
            None,
        )
    });
    let track = draft.as_track_model().expect("the draft has an id");

    assert_eq!(track.id, "temp-edited-track");
    assert_eq!(track.title, "Edited title");
    assert_eq!(track.description.as_deref(), Some("Edited description"));
    assert_eq!(track.artist, "Edited artist");
    assert_eq!(track.cover_image_url, cover_url.absolute_string());
    assert_eq!(track.source_chat_id.as_deref(), Some("chat-42"));
    assert_eq!(track.source_message_id.as_deref(), Some("message-7"));
    assert_eq!(track.remote_audio_url.as_deref(), Some("https://cdn.example/original.mp3"));
    assert_eq!(track.transfer_phase, Some(MusicLocalTrackTransferPhase::Uploading));
    assert_eq!(track.transferred_bytes, 524_288);
    assert_eq!(track.transfer_total_bytes, 1_048_576);
    assert_eq!(track.normalized_transfer_progress(), 0.5);
}

#[gpui::test]
fn server_playlist_refresh_cannot_erase_pending_local_track(cx: &mut TestAppContext) {
    let (server_track, pending_track) = cx.update(|cx| {
        let server_track = plain_draft(
            "server-track",
            "Server track",
            "Server artist",
            10_000,
            &url("https://cdn.example/server.mp3"),
            cx,
        );
        let service = MusicServicesStore::shared(cx);
        let pending_track = service.read(cx).make_local_track_draft(
            "temp-pending",
            "Edited pending title",
            Some("Edited pending description"),
            "Edited pending artist",
            20_000,
            "owner",
            &url("https://cdn.example/pending.mp3"),
            "https://cdn.example/pending.jpg",
            None,
            None,
            None,
            Some(MusicLocalTrackTransferPhase::Downloading),
            65_536,
            1_048_576,
            None,
        );
        (server_track, pending_track)
    });
    let base = RiseStoredMusicPlaylist { new_tracks_on_top: Some(true), ..local_playlist(vec![server_track]) };
    let pending = local_playlist(vec![pending_track]);

    let merged = MusicServicesStore::merging_local_track_optimistic_playlist(Some(base), Some(pending))
        .expect("both playlists are present");
    let tracks = merged.tracks.unwrap_or_default();
    let restored_pending = tracks
        .iter()
        .find(|track| track.id.as_deref() == Some("temp-pending"))
        .and_then(RiseStoredMusicTrack::as_track_model)
        .expect("the pending track survives the refresh");

    assert_eq!(
        tracks.iter().map(|track| track.id.as_deref()).collect::<Vec<_>>(),
        vec![Some("temp-pending"), Some("server-track")]
    );
    assert_eq!(restored_pending.title, "Edited pending title");
    assert_eq!(restored_pending.description.as_deref(), Some("Edited pending description"));
    assert_eq!(restored_pending.cover_image_url, "https://cdn.example/pending.jpg");
    assert_eq!(restored_pending.transfer_phase, Some(MusicLocalTrackTransferPhase::Downloading));
    assert_eq!(restored_pending.transferred_bytes, 65_536);
}

fn stored_track_with_id(id: &str) -> RiseStoredMusicTrack {
    RiseStoredMusicTrack { id: Some(id.to_owned()), ..RiseStoredMusicTrack::default() }
}

fn merged_track_ids(playlist: &RiseStoredMusicPlaylist, added: &[RiseStoredMusicTrack]) -> Option<Vec<String>> {
    let pending = playlist.with(None, None, None, None, Some(added.to_vec()));
    MusicServicesStore::merging_local_track_optimistic_playlist(Some(playlist.clone()), Some(pending))?
        .tracks
        .map(|tracks| tracks.into_iter().filter_map(|track| track.id).collect())
}

#[test]
fn new_tracks_go_where_the_playlist_setting_puts_them() {
    let rows = vec![stored_track_with_id("a"), stored_track_with_id("b")];
    let added = vec![stored_track_with_id("x"), stored_track_with_id("y")];
    let mut off = RiseStoredMusicPlaylist {
        id: Some("playlist".to_owned()),
        tracks: Some(rows),
        ..RiseStoredMusicPlaylist::default()
    };
    off.new_tracks_on_top = Some(false);
    let mut on = off.clone();
    on.new_tracks_on_top = Some(true);

    assert_eq!(
        merged_track_ids(&off, &added),
        Some(vec!["a".to_owned(), "b".to_owned(), "x".to_owned(), "y".to_owned()])
    );
    assert_eq!(
        merged_track_ids(&on, &added),
        Some(vec!["y".to_owned(), "x".to_owned(), "a".to_owned(), "b".to_owned()])
    );
}

#[gpui::test]
fn pending_playlist_bound_never_evicts_active_transfers(cx: &mut TestAppContext) {
    let (active, failed) = cx.update(|cx| {
        let mut draft = |index: usize, name: &str, phase: MusicLocalTrackTransferPhase| {
            transfer_draft(
                &format!("{}-{index}", name.to_lowercase()),
                &format!("{name} {index}"),
                "Artist",
                10_000,
                &url(&format!("https://cdn.example/{}-{index}.mp3", name.to_lowercase())),
                Some(phase),
                cx,
            )
        };
        let active: Vec<_> =
            (0..21).map(|index| draft(index, "Active", MusicLocalTrackTransferPhase::Uploading)).collect();
        let failed: Vec<_> =
            (0..4).map(|index| draft(index, "Failed", MusicLocalTrackTransferPhase::Failed)).collect();
        (active, failed)
    });

    let bounded = MusicServicesStore::bounded_message_mp3_pending_tracks(&[failed, active.clone()].concat(), 20);

    assert_eq!(bounded.len(), active.len());
    assert_eq!(
        bounded.iter().filter_map(|track| track.id.clone()).collect::<HashSet<_>>(),
        active.iter().filter_map(|track| track.id.clone()).collect::<HashSet<_>>()
    );
}

#[test]
fn message_mp3_progress_publishing_is_bounded_but_finishes_exactly() {
    assert!(!MusicMessageMP3ProgressPolicy::should_publish(0, 16_384, 1_048_576, 10.0, 10.05));
    assert!(MusicMessageMP3ProgressPolicy::should_publish(0, 32_768, 1_048_576, 10.0, 10.05));
    assert!(MusicMessageMP3ProgressPolicy::should_publish(32_768, 40_000, 1_048_576, 10.0, 10.11));
    assert!(MusicMessageMP3ProgressPolicy::should_publish(1_000_000, 1_048_576, 1_048_576, 10.0, 10.01));
}

#[test]
fn message_mp3_progress_rejects_late_phase_and_byte_regressions() {
    assert_eq!(
        MusicLocalTrackTransferPhase::reconciled_progress_update(
            Some(MusicLocalTrackTransferPhase::Uploading),
            MusicLocalTrackTransferPhase::Downloading,
            700_000,
            1_000_000,
            800_000,
            1_000_000,
        ),
        None
    );

    let same_phase = MusicLocalTrackTransferPhase::reconciled_progress_update(
        Some(MusicLocalTrackTransferPhase::Uploading),
        MusicLocalTrackTransferPhase::Uploading,
        700_000,
        1_000_000,
        600_000,
        900_000,
    );
    assert_eq!(same_phase, Some((700_000, 1_000_000)));

    let new_phase = MusicLocalTrackTransferPhase::reconciled_progress_update(
        Some(MusicLocalTrackTransferPhase::Preparing),
        MusicLocalTrackTransferPhase::Uploading,
        1_000_000,
        1_000_000,
        0,
        1_020_000,
    );
    assert_eq!(new_phase, Some((0, 1_020_000)));
}

#[test]
fn chat_metadata_url_change_restarts_only_pending_or_failed_playback() {
    let previous = "https://temporary.example/track.mp3";
    let updated = "https://cdn.example/track.mp3";
    assert!(MusicPlayerState::should_restart_after_chat_metadata_update(true, previous, updated, false, true, false));
    assert!(MusicPlayerState::should_restart_after_chat_metadata_update(true, previous, updated, false, false, true));
    assert!(!MusicPlayerState::should_restart_after_chat_metadata_update(true, previous, updated, true, false, false));
    assert!(!MusicPlayerState::should_restart_after_chat_metadata_update(false, previous, updated, false, true, false));
}

#[test]
fn chat_track_prefers_existing_local_audio_and_keeps_remote_fallback() {
    let file = written_file("riseonly-chat-audio", "mp3", &[0x49, 0x44, 0x33, 0x04]);
    let local_url = file_url(&file.0);
    let remote_url = "https://cdn.riseonly.net/public/chat/audio.mp3";
    let mut item = media("audio-local", MessageMediaKind::Audio, remote_url);
    item.local_preview_url = Some(local_url.absolute_string().to_owned());

    let track = ChatAudioTrackAdapter::track("message-local", None, "chat-local", &item, "");

    assert_eq!(track.audio_url, local_url.absolute_string());
    assert_eq!(track.remote_audio_url.as_deref(), Some(remote_url));
    assert_eq!(track.stable_audio_url(), remote_url);
    assert_eq!(
        MusicPlaybackUrl::playback_url_candidates_for(&track)
            .iter()
            .map(|candidate| candidate.absolute_string().to_owned())
            .collect::<Vec<_>>(),
        vec![local_url.absolute_string().to_owned(), remote_url.to_owned()]
    );
}

#[test]
fn chat_track_falls_back_to_remote_when_optimistic_file_was_removed() {
    let missing_url = file_url(&temporary_path("riseonly-missing-audio", "mp3"));
    let remote_url = "https://cdn.riseonly.net/public/chat/audio.mp3";
    let mut item = media("audio-remote", MessageMediaKind::Audio, remote_url);
    item.local_preview_url = Some(missing_url.absolute_string().to_owned());

    let track = ChatAudioTrackAdapter::track("message-remote", None, "chat-remote", &item, "");

    assert_eq!(track.audio_url, remote_url);
    assert_eq!(track.stable_audio_url(), remote_url);
}

#[test]
fn chat_track_never_treats_local_cover_as_audio() {
    let file = written_file("riseonly-chat-cover", "jpg", &[0xFF, 0xD8, 0xFF]);
    let cover_url = file_url(&file.0);
    let remote_url = "https://cdn.riseonly.net/public/chat/audio.mp3";
    let mut item = media(remote_url, MessageMediaKind::Audio, remote_url);
    item.thumbnail_url = Some(cover_url.absolute_string().to_owned());
    item.local_preview_url = Some(cover_url.absolute_string().to_owned());

    let track = ChatAudioTrackAdapter::track("message-cover", None, "chat-cover", &item, "");

    assert_eq!(track.audio_url, remote_url);
    assert_eq!(track.cover_image_url, cover_url.absolute_string());
}

#[test]
fn playback_cache_hashes_url_shaped_track_identifiers() {
    let track_id = "chat-audio:message:https://cdn.riseonly.net/public/chat/file.mp3";
    let path = MusicPlaybackDiskCache::file_url(track_id, "mp3", Some("music-test-user"))
        .expect("the cache directory resolves");
    MusicPlaybackDiskCache::remove_persisted_account("music-test-user");
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();

    assert!(path.iter().any(|component| component == "MusicPlaybackCache"));
    assert!(path.iter().any(|component| component == "accounts"));
    assert!(name.starts_with("music-"));
    assert_eq!(path.extension().and_then(|extension| extension.to_str()), Some("mp3"));
    assert!(!name.contains('/'));
}
