use std::path::Path;

use gpui::TestAppContext;

use crate::core::media::media_save_coordinator::{MediaSaveTargetKind, MessageMediaSaveClassifier};
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind,
};
use crate::modules::music::shared::utils::music::music_models::{MusicLocalTrackTransferPhase, TrackModel};
use crate::modules::music::shared::utils::music::music_track_file_export::{
    MusicTrackFileMaterializer, MusicTrackSavePolicy,
};

const DEFAULT_URL: &str = "https://cdn.example.com/object.bin";

fn item_at(kind: MessageMediaKind, file_name: Option<&str>, mime_type: Option<&str>, url: &str) -> MessageMediaItem {
    let mut item = MessageMediaItem::new(
        format!("media-{}-{}", kind.raw_value(), file_name.unwrap_or("none")),
        kind,
        url.to_owned(),
    );
    item.file_name = file_name.map(str::to_owned);
    item.mime_type = mime_type.map(str::to_owned);
    item
}

fn item(kind: MessageMediaKind, file_name: Option<&str>, mime_type: Option<&str>) -> MessageMediaItem {
    item_at(kind, file_name, mime_type, DEFAULT_URL)
}

fn chat(id: &str) -> ChatInfo {
    let mut chat = ChatInfo::new(id.to_owned());
    chat.r#type = ChatType::Direct;
    chat
}

fn track(status: &str, audio_url: &str) -> TrackModel {
    TrackModel {
        id: format!("track-{status}"),
        title: "Title".to_owned(),
        artist: "Artist".to_owned(),
        artist_id: String::new(),
        album_title: String::new(),
        album_id: String::new(),
        duration_seconds: 100,
        cover_colors: Vec::new(),
        cover_image_url: String::new(),
        audio_url: audio_url.to_owned(),
        is_exclusive: false,
        status: status.to_owned(),
        description: None,
        source_chat_id: None,
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

#[test]
fn compressed_photo_and_video_go_to_the_gallery() {
    for kind in [
        MessageMediaKind::Image,
        MessageMediaKind::Video,
        MessageMediaKind::VideoNote,
        MessageMediaKind::Animation,
    ] {
        assert_eq!(MessageMediaSaveClassifier::target(&item(kind, None, None)), Some(MediaSaveTargetKind::Gallery));
    }
}

#[test]
fn uncompressed_photo_and_video_arrive_as_documents_but_still_go_to_the_gallery() {
    let photo = item(MessageMediaKind::Document, Some("IMG_0001.HEIC"), Some("image/heic"));
    let video = item(MessageMediaKind::Document, Some("IMG_0002.MOV"), Some("video/quicktime"));

    assert_eq!(MessageMediaSaveClassifier::target(&photo), Some(MediaSaveTargetKind::Gallery));
    assert_eq!(MessageMediaSaveClassifier::target(&video), Some(MediaSaveTargetKind::Gallery));
    assert!(!MessageMediaSaveClassifier::is_video(&photo));
    assert!(MessageMediaSaveClassifier::is_video(&video));
}

#[test]
fn document_without_mime_falls_back_to_its_extension() {
    let photo = item(MessageMediaKind::Document, Some("scan.png"), None);
    let archive = item(MessageMediaKind::Document, Some("logs.zip"), None);
    let octet_stream_video = item(MessageMediaKind::Document, Some("clip.mp4"), Some("application/octet-stream"));

    assert_eq!(MessageMediaSaveClassifier::target(&photo), Some(MediaSaveTargetKind::Gallery));
    assert_eq!(MessageMediaSaveClassifier::target(&archive), Some(MediaSaveTargetKind::Files));
    assert_eq!(MessageMediaSaveClassifier::target(&octet_stream_video), Some(MediaSaveTargetKind::Gallery));
}

#[test]
fn audio_voice_and_plain_files_go_to_files() {
    let mp3 = item(MessageMediaKind::Audio, Some("track.mp3"), Some("audio/mpeg"));
    let voice = item(MessageMediaKind::Voice, Some("voice.ogg"), None);
    let pdf = item(MessageMediaKind::Document, Some("contract.pdf"), Some("application/pdf"));

    assert_eq!(MessageMediaSaveClassifier::target(&mp3), Some(MediaSaveTargetKind::Files));
    assert_eq!(MessageMediaSaveClassifier::target(&voice), Some(MediaSaveTargetKind::Files));
    assert_eq!(MessageMediaSaveClassifier::target(&pdf), Some(MediaSaveTargetKind::Files));
}

#[test]
fn stickers_in_flight_uploads_and_sourceless_items_are_not_savable() {
    let mut uploading = item(MessageMediaKind::Image, None, None);
    uploading.is_uploading = true;
    let mut cancelled = item(MessageMediaKind::Document, Some("a.pdf"), None);
    cancelled.is_cancelled = true;

    assert_eq!(MessageMediaSaveClassifier::target(&item(MessageMediaKind::Sticker, None, None)), None);
    assert_eq!(MessageMediaSaveClassifier::target(&uploading), None);
    assert_eq!(MessageMediaSaveClassifier::target(&cancelled), None);
    assert_eq!(
        MessageMediaSaveClassifier::target(&item_at(MessageMediaKind::Document, Some("a.pdf"), None, "  ")),
        None
    );
}

#[gpui::test]
fn protected_content_offers_no_save_action(cx: &mut TestAppContext) {
    let mut message = GetMessageMessage::new("message-protected".to_owned());
    message.server_id = Some("message-protected".to_owned());
    message.content_type = MessageContentType::Document;
    message.has_protected_content = true;
    message.media_items = vec![item(MessageMediaKind::Document, Some("secret.pdf"), None)];
    let chat = chat("chat-protected");
    let permissions = MessagePermissions::new(&message, &chat, false);

    assert!(!permissions.can_save_to_files());
    assert!(!permissions.can_save_to_gallery());
    let actions = cx.update(|cx| permissions.context_menu_actions(false, true, cx));
    assert!(
        actions
            .iter()
            .all(|action| action.id != ChatMessageAction::SAVE && action.id != ChatMessageAction::SAVE_TO_GALLERY)
    );
}

#[gpui::test]
fn album_offers_both_destinations_and_routes_each_item_once(cx: &mut TestAppContext) {
    let mut message = GetMessageMessage::new("message-mixed".to_owned());
    message.server_id = Some("message-mixed".to_owned());
    message.content_type = MessageContentType::Image;
    message.media_items = vec![
        item(MessageMediaKind::Image, Some("a.jpg"), None),
        item(MessageMediaKind::Document, Some("b.pdf"), Some("application/pdf")),
        item(MessageMediaKind::Document, Some("c.png"), Some("image/png")),
        item(MessageMediaKind::Sticker, Some("d.webp"), None),
    ];
    let chat = chat("chat-mixed");
    let permissions = MessagePermissions::new(&message, &chat, true);

    assert_eq!(permissions.gallery_savable_items().len(), 2);
    assert_eq!(permissions.file_savable_items().len(), 1);
    assert_eq!(permissions.file_savable_items()[0].file_name.as_deref(), Some("b.pdf"));

    let actions = cx.update(|cx| permissions.context_menu_actions(false, true, cx));
    assert!(actions.iter().any(|action| action.id == ChatMessageAction::SAVE_TO_GALLERY));
    assert!(actions.iter().any(|action| action.id == ChatMessageAction::SAVE));
}

#[test]
fn only_user_uploaded_tracks_are_savable() {
    assert!(MusicTrackSavePolicy::can_save(&track("local_library", "https://cdn/x.mp3")));
    assert!(!MusicTrackSavePolicy::can_save(&track("published", "https://cdn/x.mp3")));
    assert!(!MusicTrackSavePolicy::can_save(&track("original_audio", "https://cdn/x.mp3")));
    assert!(!MusicTrackSavePolicy::can_save(&track("local_library", "")));

    let mut transferring = track("local_library", "https://cdn/x.mp3");
    transferring.transfer_phase = Some(MusicLocalTrackTransferPhase::Downloading);
    assert!(!MusicTrackSavePolicy::can_save(&transferring));
}

#[test]
fn exported_track_file_name_combines_artist_and_title() {
    let mut track = track("local_library", "https://cdn/x.mp3");
    track.id = "track-1".to_owned();
    track.title = "Song / Two".to_owned();
    track.artist = "The: Band".to_owned();

    let name = MusicTrackFileMaterializer::export_file_name(&track, Path::new("/tmp/music-abc.m4a"));

    assert_eq!(name, "The_ Band - Song _ Two.m4a");
}
