use gpui::TestAppContext;
use rise_i18n::{FormatArg, tr, tr_args};
use serde_json::Value;

use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::chat::components::chat_item::chat_list_preview_row::ChatListPreviewFactory;
use crate::modules::chat::pages::chats::chat::chat_screen::chat_screen_support::ChatMessagePresentationRecoveryPolicy;
use crate::modules::chat::pages::chats::chat::chat_topics::topic_editor_sheet::TopicEditorSheetPresentationContract;
use crate::modules::chat::shared::utils::message_preview::MessagePreview;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatTopic;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    RiseStoredChatPreview, RiseStoredChatTopic, RiseStoredChatTopicList,
};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind,
};

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("test payload is valid JSON")
}

#[test]
fn message_presentation_recovers_canonical_rows_and_waits_for_refresh() {
    assert!(ChatMessagePresentationRecoveryPolicy::should_use_canonical_fallback(0, 100));
    assert!(!ChatMessagePresentationRecoveryPolicy::should_use_canonical_fallback(12, 100));
    assert!(!ChatMessagePresentationRecoveryPolicy::initial_data_resolved(
        false,
        0,
        0,
        true,
        SaiWsSliceStatus::Fulfilled,
        true,
        false,
        false,
    ));
    assert!(ChatMessagePresentationRecoveryPolicy::initial_data_resolved(
        false,
        0,
        0,
        false,
        SaiWsSliceStatus::Fulfilled,
        true,
        false,
        false,
    ));
    assert!(!ChatMessagePresentationRecoveryPolicy::initial_data_resolved(
        false,
        0,
        100,
        false,
        SaiWsSliceStatus::Fulfilled,
        false,
        false,
        false,
    ));
    assert!(ChatMessagePresentationRecoveryPolicy::initial_data_resolved(
        false,
        30,
        30,
        true,
        SaiWsSliceStatus::Fulfilled,
        false,
        true,
        false,
    ));
    assert!(ChatMessagePresentationRecoveryPolicy::initial_data_resolved(
        false,
        30,
        30,
        false,
        SaiWsSliceStatus::Fulfilled,
        false,
        true,
        false,
    ));
    assert!(!ChatMessagePresentationRecoveryPolicy::initial_data_resolved(
        false,
        30,
        30,
        false,
        SaiWsSliceStatus::Fulfilled,
        false,
        true,
        true,
    ));
    assert!(ChatMessagePresentationRecoveryPolicy::initial_data_resolved(
        false,
        30,
        30,
        false,
        SaiWsSliceStatus::Fulfilled,
        false,
        false,
        true,
    ));
}

#[test]
fn editor_uses_full_height_detent_and_keyboard_spacing() {
    assert_eq!(TopicEditorSheetPresentationContract::FULL_HEIGHT_FRACTION, 1.0);
    assert_eq!(TopicEditorSheetPresentationContract::KEYBOARD_FOOTER_SPACING, 12.0);
}

#[test]
fn legacy_blank_icon_uses_default_emoji() {
    let payload = value(r#"{"topic_id": "topic-1", "chat_id": "chat-1", "title": "News", "icon": ""}"#);
    let wire = SaiJSONCodec::decode::<RiseStoredChatTopic>(Some(&payload)).expect("topic decodes");

    assert_eq!(wire.to_topic().icon, ChatTopic::DEFAULT_ICON);
}

#[test]
fn topic_preview_normalizes_milliseconds_and_unread_state() {
    let payload = value(
        r#"{
            "topic_id": "topic-2",
            "chat_id": "chat-1",
            "title": "Reports",
            "icon": "🎬",
            "unread_count": 7,
            "is_muted": true,
            "updated_at": 1720000100000,
            "last_message": {
                "id": "message-1",
                "chat_id": "chat-1",
                "sender_id": "user-2",
                "sender_name": "Aruzhan",
                "content": "Weekly report",
                "content_type": "text",
                "created_at": 1720000000000
            }
        }"#,
    );
    let topic = SaiJSONCodec::decode::<RiseStoredChatTopic>(Some(&payload)).expect("topic decodes").to_topic();
    let last_message = topic.last_message.as_ref().expect("topic keeps its last message");

    assert_eq!(last_message.text, "Weekly report");
    assert_eq!(last_message.created_at, 1_720_000_000.0);
    assert_eq!(topic.unread_count, 7);
    assert!(topic.is_muted);
    assert_eq!(topic.updated_at, 1_720_000_100.0);
}

#[test]
fn general_preview_is_separate_from_real_topics() {
    let payload = value(
        r##"{
            "topics": [],
            "general_topic": {
                "topic_id": "",
                "chat_id": "chat-1",
                "title": "General",
                "icon": "#⃣",
                "is_general": true,
                "unread_count": 3
            }
        }"##,
    );
    let response = SaiJSONCodec::decode::<RiseStoredChatTopicList>(Some(&payload)).expect("topic list decodes");

    assert!(response.topics.as_ref().is_some_and(|topics| topics.is_empty()));
    let general = response.general_topic.as_ref().expect("general topic decodes").to_topic();
    assert!(general.is_general);
    assert_eq!(general.unread_count, 3);
}

#[gpui::test]
fn cold_summary_media_previews_fall_back_to_content_type(cx: &mut TestAppContext) {
    let expected = [
        ("image", tr("msg_preview_photo").to_string()),
        ("video", tr("msg_preview_video").to_string()),
        ("voice", tr("msg_preview_voice").to_string()),
        ("document", format!("\u{1F4C4} {}", tr("msg_preview_document_fallback"))),
        ("sticker", format!("\u{1F642} {}", tr("msg_preview_sticker"))),
        ("video_note", tr("msg_preview_video_note").to_string()),
    ];

    for (content_type, preview) in expected {
        let payload = value(&format!(
            r#"{{
                "id": "message-{content_type}",
                "chat_id": "chat-1",
                "sender_id": "user-2",
                "content": "",
                "content_type": "{content_type}",
                "created_at": 1720000000000
            }}"#
        ));
        let wire = SaiJSONCodec::decode::<RiseStoredChatPreview>(Some(&payload)).expect("summary decodes");

        let snippet = cx.update(|cx| MessagePreview::snippet(&wire.to_message(), false, cx));
        assert_eq!(snippet, preview);
    }
}

#[gpui::test]
fn chat_list_preview_shows_empty_copy_and_media_thumbnail(cx: &mut TestAppContext) {
    let empty = cx.update(|cx| ChatListPreviewFactory::synthetic_topic("Empty", None, 0, 0, 0, 0, false, 0.0, None, cx));
    assert_eq!(empty.snippet, Some(tr("chat_preview_empty").to_string()));
    assert_eq!(empty.media_thumbnail_source, None);

    let media = GetMessageMessage {
        id: "media-message".to_owned(),
        content_type: MessageContentType::Image,
        media_items: vec![MessageMediaItem {
            thumbnail_url: Some("https://cdn.example/thumb.jpg".to_owned()),
            ..MessageMediaItem::new(
                "image-1".to_owned(),
                MessageMediaKind::Image,
                "https://cdn.example/full.jpg".to_owned(),
            )
        }],
        ..GetMessageMessage::default()
    };
    let media_preview =
        cx.update(|cx| ChatListPreviewFactory::synthetic_topic("Media", Some(&media), 0, 0, 0, 0, false, 0.0, None, cx));
    assert_eq!(media_preview.snippet, Some(tr("msg_preview_photo_plain").to_string()));
    assert_eq!(media_preview.media_thumbnail_source.as_deref(), Some("https://cdn.example/thumb.jpg"));
}

#[gpui::test]
fn chat_list_summary_media_draws_first_photo_and_album_size(cx: &mut TestAppContext) {
    let payload = value(
        r#"{
            "id": "album",
            "chat_id": "chat-1",
            "sender_id": "user-2",
            "content": "",
            "content_type": "image",
            "created_at": 1720000000000,
            "media_info": "{\"type\":\"image\",\"media_type\":\"image\",\"file_url\":\"https://cdn.example/first.jpg\",\"media_count\":3}"
        }"#,
    );
    let wire = SaiJSONCodec::decode::<RiseStoredChatPreview>(Some(&payload)).expect("summary decodes");
    let message = wire.to_message();
    assert_eq!(message.grid_media_items().len(), 3);

    let preview =
        cx.update(|cx| ChatListPreviewFactory::synthetic_topic("Album", Some(&message), 0, 0, 0, 0, false, 0.0, None, cx));
    assert_eq!(preview.media_thumbnail_source.as_deref(), Some("https://cdn.example/first.jpg"));
    assert_eq!(
        preview.snippet,
        Some(tr_args("msg_preview_media_count_plain", &[FormatArg::Int(3)]).to_string())
    );
    assert_eq!(
        cx.update(|cx| MessagePreview::snippet(&message, false, cx)),
        tr_args("msg_preview_media_count", &[FormatArg::Int(3)]).to_string()
    );

    let legacy_payload = value(
        r#"{
            "id": "voice",
            "content": "",
            "content_type": "voice",
            "media_info": "{\"type\":\"voice\",\"duration\":3}"
        }"#,
    );
    let legacy = SaiJSONCodec::decode::<RiseStoredChatPreview>(Some(&legacy_payload)).expect("summary decodes");
    assert!(legacy.to_message().media_items.is_empty());
}
