use gpui::{App, TestAppContext};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use serde_json::{Map, Value};

use crate::modules::chat::components::chat::bubble::message_sticker_view::MessageStickerFavourite;
use crate::modules::chat::shared::message_permissions::{ChatMessageAction, MessagePermissions};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageDeliveryStatus, MessageMediaItem, MessageMediaKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{MessagePayloadCodec, MessageWireDTO};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const FEED_MENU_INCLUDES_COMPOSER_ACTIONS: bool = true;

fn chat(id: &str, chat_type: ChatType) -> ChatInfo {
    ChatInfo { id: id.to_owned(), r#type: chat_type, ..ChatInfo::default() }
}

fn member_chat(id: &str, chat_type: ChatType) -> ChatInfo {
    ChatInfo { role: Some("MEMBER".to_owned()), ..chat(id, chat_type) }
}

fn confirmed_message(id: &str, content_type: MessageContentType) -> GetMessageMessage {
    let mut message = GetMessageMessage::new(id.to_owned());
    message.server_id = Some(id.to_owned());
    message.content_type = content_type;
    message
}

fn actions(permissions: &MessagePermissions<'_>, includes_selection: bool, cx: &mut App) -> Vec<HoldContextMenuAction> {
    permissions.context_menu_actions(includes_selection, FEED_MENU_INCLUDES_COMPOSER_ACTIONS, cx)
}

fn row(text: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(row)) => row,
        _ => panic!("test row is a JSON object"),
    }
}

#[gpui::test]
fn chat_menu_places_selection_after_delete(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut message = confirmed_message("message-1", MessageContentType::Text);
        message.text = "hello".to_owned();
        let chat = chat("chat-1", ChatType::Direct);

        let actions = actions(&MessagePermissions::new(&message, &chat, true), true, cx);
        let delete_index = actions.iter().position(|action| action.id == ChatMessageAction::DELETE);
        let select_index = actions.iter().position(|action| action.id == ChatMessageAction::SELECT);

        let (Some(delete_index), Some(select_index)) = (delete_index, select_index) else {
            panic!("the menu has delete and select");
        };
        assert_eq!(select_index, delete_index + 1);
        assert!(actions[select_index].section_break);
    });
}

#[gpui::test]
fn shared_lists_do_not_expose_selection_unless_enabled(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut message = confirmed_message("message-2", MessageContentType::Text);
        message.text = "hello".to_owned();
        let chat = chat("chat-2", ChatType::Direct);
        let permissions = MessagePermissions::new(&message, &chat, true);

        assert!(actions(&permissions, false, cx).iter().all(|action| action.id != ChatMessageAction::SELECT));
        assert!(actions(&permissions, true, cx).iter().any(|action| action.id == ChatMessageAction::SELECT));
    });
}

#[gpui::test]
fn pending_and_system_messages_cannot_be_selected(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut pending = GetMessageMessage::new("temp_1".to_owned());
        pending.content_type = MessageContentType::Text;
        pending.is_temp = true;
        let system = confirmed_message("system-1", MessageContentType::System);
        let chat = chat("chat-3", ChatType::Direct);

        assert!(!MessagePermissions::new(&pending, &chat, true).can_select(cx));
        assert!(!MessagePermissions::new(&pending, &chat, true).can_delete_for_me(cx));
        assert!(!MessagePermissions::new(&system, &chat, false).can_select(cx));
    });
}

#[gpui::test]
fn protected_messages_cannot_be_forwarded_or_shared(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut message = confirmed_message("message-protected", MessageContentType::Image);
        message.has_protected_content = true;
        let chat = chat("chat-protected", ChatType::Direct);
        let permissions = MessagePermissions::new(&message, &chat, false);

        assert!(!permissions.can_forward(cx));
        assert!(!permissions.can_share());
        assert!(!permissions.can_copy());
        assert!(!permissions.can_save_to_gallery());
        assert!(!permissions.can_save_to_files());
        assert!(!permissions.can_add_mp3_to_local_playlist());
        assert!(permissions.can_select(cx));
        assert!(permissions.can_delete_for_me(cx));
        let actions = actions(&permissions, false, cx);
        let forward = actions.iter().find(|action| action.id == ChatMessageAction::FORWARD);
        assert!(forward.is_some_and(|forward| forward.disabled));
    });
}

#[gpui::test]
fn every_regular_message_shows_enabled_forward_action(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let message = confirmed_message("message-forward", MessageContentType::Text);
        let chat = chat("chat-forward", ChatType::Direct);
        let permissions = MessagePermissions::new(&message, &chat, false);

        let actions = actions(&permissions, false, cx);
        let forward = actions.iter().find(|action| action.id == ChatMessageAction::FORWARD);
        assert!(forward.is_some_and(|forward| !forward.disabled));
    });
}

#[gpui::test]
fn confirmed_topic_message_with_stale_temp_flag_can_be_forwarded(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let rows = vec![row(
            r#"{
                "id": "temp_1784520000000_topic",
                "server_id": "message-topic-confirmed",
                "chat_id": "chat-topic",
                "topic_id": "topic-1",
                "content_type": "text",
                "content": "hello from topic",
                "created_at": 1784520000,
                "isTemp": true
            }"#,
        )];
        let mapped = MessageServicesStore::shared(cx)
            .update(cx, |service, cx| service.map_wire_rows(&rows, "chat-topic", cx));
        let Some(message) = mapped.first() else {
            panic!("the row maps to a message");
        };

        assert_eq!(message.topic_id.as_deref(), Some("topic-1"));
        assert_eq!(message.sai_server_id(), "message-topic-confirmed");
        assert!(!message.is_temp);
        assert!(!message.is_pending_outgoing());
        assert_eq!(message.delivery_status, MessageDeliveryStatus::Sent);

        let chat = member_chat("chat-topic", ChatType::Group);
        let permissions = MessagePermissions::new(message, &chat, true);
        let actions = actions(&permissions, false, cx);
        let forward = actions.iter().find(|action| action.id == ChatMessageAction::FORWARD);
        assert!(permissions.can_forward(cx));
        assert!(forward.is_some_and(|forward| !forward.disabled));
    });
}

#[gpui::test]
fn regular_group_member_can_delete_incoming_message_only_for_self(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let message = confirmed_message("message-incoming-group", MessageContentType::Text);
        let chat = member_chat("chat-group", ChatType::Group);
        let permissions = MessagePermissions::new(&message, &chat, false);

        assert!(permissions.can_delete_for_me(cx));
        assert!(!permissions.can_delete(cx));
        let actions = actions(&permissions, false, cx);
        let delete = actions.iter().find(|action| action.id == ChatMessageAction::DELETE_ME);
        assert!(delete.is_some_and(|delete| delete.submenu.is_none()));
    });
}

#[gpui::test]
fn public_channel_reader_cannot_invoke_member_only_delete_for_me(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let message = confirmed_message("message-public-channel", MessageContentType::Text);
        let chat = ChatInfo { is_public: true, role: None, joined_at: None, ..chat("chat-public-channel", ChatType::Channel) };
        let permissions = MessagePermissions::new(&message, &chat, false);

        assert!(!permissions.can_delete_for_me(cx));
        assert!(!actions(&permissions, false, cx).iter().any(|action| {
            action.id == ChatMessageAction::DELETE || action.id == ChatMessageAction::DELETE_ME
        }));
    });
}

#[gpui::test]
fn protected_mp3_cannot_be_imported_or_favourited(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut message = confirmed_message("protected-mp3", MessageContentType::Audio);
        message.has_protected_content = true;
        let mut item = MessageMediaItem::new(
            "audio".to_owned(),
            MessageMediaKind::Audio,
            "https://example.invalid/audio.mp3".to_owned(),
        );
        item.mime_type = Some("audio/mpeg".to_owned());
        message.media_items = vec![item];
        let chat = chat("chat", ChatType::Direct);
        let permissions = MessagePermissions::new(&message, &chat, false);

        assert!(!permissions.can_add_mp3_to_local_playlist());
        assert!(MessageStickerFavourite::menu_actions(&message, &chat, cx).is_empty());
    });
}

#[gpui::test]
fn protected_content_survives_wire_and_cache_mapping(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let payload = Value::Object(row(
            r#"{
                "id": "message-protected-wire",
                "chat_id": "chat-protected",
                "content_type": "text",
                "content": "secret",
                "has_protected_content": true
            }"#,
        ));
        let Some(wire) = MessagePayloadCodec::decode::<MessageWireDTO>(Some(&payload)) else {
            panic!("the row decodes");
        };
        let message = wire.to_message();

        assert!(message.has_protected_content);
        let cached = MessageServicesStore::shared(cx).read(cx).wire_to_dict_from(&message);
        assert_eq!(cached.get("has_protected_content").and_then(Value::as_bool), Some(true));
    });
}
