use rise_chat::ChatType;

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::components::chat::async_render::rise_async_text_message_row::RiseAsyncTextMessageEligibility;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageCustomEmojiEntity, MessageNormalizedEntities,
};

fn message(id: &str) -> GetMessageMessage {
    GetMessageMessage {
        id: id.to_string(),
        ..Default::default()
    }
}

fn chat(id: &str, r#type: ChatType) -> ChatInfo {
    ChatInfo {
        id: id.to_string(),
        r#type,
        ..Default::default()
    }
}

#[test]
fn empty_wire_metadata_keeps_private_plain_text_on_exact_geometry_path() {
    let mut message = message("message-911");
    message.content_type = MessageContentType::Text;
    message.text = "911".to_string();
    message.created_at = 1_721_500_000.0;
    message.view_count = Some(0);
    message.comments_count = Some(0);
    message.entities = Some(" [ ] ".to_string());

    let seed = RiseAsyncTextMessageEligibility::seed(
        &message,
        &chat("private-chat", ChatType::Direct),
        false,
        false,
        None,
        Vec::new(),
        None,
    );

    assert!(seed.is_some());
}

#[test]
fn non_empty_entities_stay_on_the_full_renderer() {
    let mut message = message("message-entity");
    message.content_type = MessageContentType::Text;
    message.text = "Hello".to_string();
    message.normalized_entities = Some(MessageNormalizedEntities {
        custom_emojis: vec![MessageCustomEmojiEntity {
            offset: 0,
            length: 5,
            custom_emoji_id: "emoji-1".to_string(),
            file_url: String::new(),
            asset_type: "static".to_string(),
            width: 0,
            height: 0,
            pack_id: String::new(),
        }],
        role_mentions: Vec::new(),
        topic_mentions: Vec::new(),
    });

    let seed = RiseAsyncTextMessageEligibility::seed(
        &message,
        &chat("private-chat", ChatType::Direct),
        false,
        false,
        None,
        Vec::new(),
        None,
    );

    assert!(seed.is_none());
}

#[test]
fn channel_view_count_rides_the_fast_path_with_its_counter() {
    let mut message = message("channel-message");
    message.content_type = MessageContentType::Text;
    message.text = "Announcement".to_string();
    message.view_count = Some(12);

    let seed = RiseAsyncTextMessageEligibility::seed(
        &message,
        &chat("channel", ChatType::Channel),
        false,
        false,
        None,
        Vec::new(),
        None,
    );

    assert!(seed.is_some());
    assert_eq!(
        seed.and_then(|seed| seed.view_count_text),
        Some(RiseFormat::compact_count(12))
    );
}
