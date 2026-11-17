use gpui::TestAppContext;
use rise_canon::rise_request;
use rise_canon::rise_view::chat::ChatInfo as CanonChatInfo;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::modules::chat::stores::chats::chat_actions::chat_actions::DeleteChatGatewayDTO;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatMuteScopes;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::MessageSystemKind;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    GetMessagesGatewayDTO, MessagePayloadCodec, MessageWireDTO,
};
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::post::stores::post_actions::post_wire_types::{GetFeedPostItemDTOExt, GetViewHistoryGatewayDTO};
use crate::modules::search::stores::search_actions::search_wire_types::{
    GlobalSearchGatewayDTO, GlobalSearchGatewayDTOExt,
};

enum WireDecodeSupport {}

impl WireDecodeSupport {
    fn decode<T: DeserializeOwned>(json: &str) -> T {
        match serde_json::from_str(json) {
            Ok(value) => value,
            Err(error) => panic!("wire payload decodes: {error}"),
        }
    }

    fn chat(json: &str) -> RiseStoredChat {
        RiseStoredChat::from_canon(&Self::decode::<CanonChatInfo>(json))
    }

    fn encode_object<T: Serialize>(value: &T) -> Value {
        match serde_json::to_value(value) {
            Ok(value) => value,
            Err(error) => panic!("wire body encodes: {error}"),
        }
    }
}

#[test]
fn chat_info_decodes_unread_info_structured() {
    let dto = WireDecodeSupport::chat(
        r#"{
          "id": "chat-1",
          "type": "GROUP",
          "unread_info": {
            "reactions": { "reactions": ["x", "y"], "count": 2 },
            "messages": { "count": 5 },
            "mentions": { "message_ids": ["m1", "m2"], "count": 2 }
          }
        }"#,
    );
    let Some(unread) = dto.unread_info.as_ref() else {
        panic!("unread_info decodes");
    };
    assert_eq!(unread.reactions.as_ref().map(|reactions| reactions.count), Some(2));
    assert_eq!(
        unread.reactions.as_ref().map(|reactions| reactions.reactions.clone()),
        Some(vec!["x".to_owned(), "y".to_owned()])
    );
    assert_eq!(unread.messages.as_ref().and_then(|messages| messages.count), Some(5));
    assert_eq!(unread.mentions.as_ref().map(|mentions| mentions.count), Some(2));
    assert_eq!(
        unread.mentions.as_ref().map(|mentions| mentions.message_ids.clone()),
        Some(vec!["m1".to_owned(), "m2".to_owned()])
    );
    assert_eq!(dto.resolved_unread_count(), 5);
    assert_eq!(dto.to_chat_info().unread_count, 5);
}

#[test]
fn chat_info_root_unread_count_is_authoritative_over_nested_duplicate() {
    let cleared = WireDecodeSupport::chat(
        r#"{"id":"chat-cleared","type":"GROUP","unread_count":0,"unread_info":{"messages":{"count":6}}}"#,
    );
    let decremented = WireDecodeSupport::chat(
        r#"{"id":"chat-decremented","type":"GROUP","unread_count":1,"unread_info":{"messages":{"count":6}}}"#,
    );
    let root_wins = WireDecodeSupport::chat(
        r#"{"id":"chat-root","type":"GROUP","unread_count":8,"unread_info":{"messages":{"count":3}}}"#,
    );

    assert_eq!(cleared.resolved_unread_count(), 0);
    assert_eq!(cleared.to_chat_info().unread_count, 0);
    assert_eq!(decremented.resolved_unread_count(), 1);
    assert_eq!(root_wins.resolved_unread_count(), 8);
}

#[test]
fn chat_info_unread_info_null_decodes_to_nil() {
    let dto = WireDecodeSupport::chat(r#"{ "id": "chat-2", "type": "PRIVATE", "unread_info": null }"#);
    assert!(dto.unread_info.is_none());
}

#[test]
fn get_message_requests_body_encodes_authenticated_user_and_snake_case_cursor() {
    let body = rise_request::chat::GetMessageRequests {
        limit: Some(50),
        relative_id: Some("1720000000000:chat-1".to_owned()),
    };

    let json = WireDecodeSupport::encode_object(&body);
    assert!(json.get("user_id").is_none());
    assert_eq!(json.get("limit").and_then(Value::as_i64), Some(50));
    assert_eq!(json.get("relative_id").and_then(Value::as_str), Some("1720000000000:chat-1"));
    assert!(json.get("userId").is_none());
    assert!(json.get("relativeId").is_none());
}

#[test]
fn resolve_message_request_body_encodes_authenticated_user_and_snake_case_chat_id() {
    let body = rise_request::chat::ResolveMessageRequest {
        action: "DELETE_AND_BLOCK".to_owned(),
        chat_id: "chat-1".to_owned(),
    };

    let json = WireDecodeSupport::encode_object(&body);
    assert!(json.get("user_id").is_none());
    assert_eq!(json.get("chat_id").and_then(Value::as_str), Some("chat-1"));
    assert_eq!(json.get("action").and_then(Value::as_str), Some("DELETE_AND_BLOCK"));
    assert!(json.get("userId").is_none());
    assert!(json.get("chatId").is_none());
}

#[test]
fn delete_chat_body_encodes_for_both_scope() {
    let body = rise_request::chat::DeleteChat {
        chat_id: "chat-1".to_owned(),
        clear_history: None,
        delete_for_both: Some(true),
    };

    let json = WireDecodeSupport::encode_object(&body);
    assert!(json.get("user_id").is_none());
    assert_eq!(json.get("chat_id").and_then(Value::as_str), Some("chat-1"));
    assert_eq!(json.get("delete_for_both").and_then(Value::as_bool), Some(true));
    assert!(json.get("deleteForBoth").is_none());
}

#[test]
fn delete_chat_response_decodes_server_failure_for_rollback() {
    let dto: DeleteChatGatewayDTO =
        WireDecodeSupport::decode(r#"{"success":false,"error":"not allowed","chat_id":""}"#);
    assert!(!dto.success);
    assert_eq!(dto.error, "not allowed");
    assert_eq!(dto.chat_id, "");
}

#[test]
fn chat_info_decodes_pending_message_request_state() {
    let dto = WireDecodeSupport::chat(
        r#"{"id":"request-chat-1","type":"PRIVATE","is_message_request":true,"message_request_state":"PENDING"}"#,
    );
    assert_eq!(dto.is_message_request, Some(true));
    assert_eq!(dto.message_request_state.as_deref(), Some("PENDING"));

    let chat = dto.to_chat_info();
    assert!(chat.is_message_request);
    assert_eq!(chat.message_request_state.as_deref(), Some("PENDING"));
}

#[test]
fn chat_info_decodes_typed_creator_from_json_string() {
    let dto = WireDecodeSupport::chat(
        r#"{ "id": "chat-3", "type": "CHANNEL", "creator_info": "{\"id\":\"user-7\",\"name\":\"Alice\",\"tag\":\"alice\",\"is_premium\":true,\"user_chat_id\":\"uc-7\",\"role\":\"OWNER\",\"more\":{\"logo\":\"logo.png\",\"is_online\":true,\"description\":\"hi\"}}" }"#,
    );
    let Some(typed) = dto.creator() else {
        panic!("creator_info decodes to a typed user");
    };
    assert_eq!(typed.id.as_deref(), Some("user-7"));
    assert_eq!(typed.name.as_deref(), Some("Alice"));
    assert_eq!(typed.tag.as_deref(), Some("alice"));
    assert_eq!(typed.is_premium, Some(true));
    assert_eq!(typed.user_chat_id.as_deref(), Some("uc-7"));
    assert_eq!(typed.role.as_deref(), Some("OWNER"));
    assert_eq!(typed.more.as_ref().and_then(|more| more.logo.as_deref()), Some("logo.png"));
    assert_eq!(typed.more.as_ref().and_then(|more| more.is_online), Some(true));
    assert_eq!(dto.creator_info.as_ref().and_then(|creator| creator.as_string()).as_deref(), Some("user-7"));
}

#[test]
fn chat_info_creator_info_plain_string_is_an_id() {
    let dto = WireDecodeSupport::chat(r#"{ "id": "chat-4", "type": "GROUP", "creator_info": "user-42" }"#);
    assert_eq!(dto.creator_info.as_ref().and_then(|creator| creator.as_string()).as_deref(), Some("user-42"));
    assert!(dto.creator().is_none());
}

#[test]
fn message_decodes_reactions_unreaded_bool() {
    let dto: MessageWireDTO =
        WireDecodeSupport::decode(r#"{"id":"msg-1","chat_id":"chat-1","content":"hello","reactions_unreaded":true}"#);
    assert_eq!(dto.reactions_unreaded, Some(true));
}

#[test]
fn chat_info_decodes_bot_participant_identity() {
    let dto = WireDecodeSupport::chat(
        r#"{
          "id": "chat-bot",
          "type": "PRIVATE",
          "participant": {
            "id": "bot-user",
            "name": "Example Bot",
            "tag": "examplebot",
            "is_bot": true,
            "bot_is_active": true,
            "bot_owner_id": "owner-user"
          }
        }"#,
    );
    let chat = dto.to_chat_info();
    assert!(chat.is_bot_chat());
    assert!(chat.participant.is_bot);
    assert!(chat.participant.bot_is_active);
    assert_eq!(chat.participant.bot_owner_id.as_deref(), Some("owner-user"));
}

#[test]
fn global_search_decodes_bot_participant_in_results() {
    let dto: GlobalSearchGatewayDTO = WireDecodeSupport::decode(
        r#"{
          "query": "orphan",
          "chats": [],
          "chats_has_more": false,
          "global_results": [{
            "entity_type": 0,
            "entity_id": "bot-user",
            "user": {
              "id": "bot-user",
              "name": "Orphan Bot",
              "tag": "orphanbot",
              "is_bot": true,
              "bot_is_active": true,
              "chat_id": "chat-1"
            }
          }],
          "global_has_more": false,
          "global_next_cursor": null
        }"#,
    );
    let rows = dto.global_entity_rows();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_bot_user());
    assert_eq!(rows[0].display_name, "Orphan Bot");
}

#[test]
fn message_reactions_unreaded_absent_decodes_to_nil() {
    let dto: MessageWireDTO = WireDecodeSupport::decode(r#"{ "id": "msg-2", "chat_id": "chat-1", "content": "no flag" }"#);
    assert!(dto.reactions_unreaded.is_none());
}

#[test]
fn message_history_keeps_row_when_optional_gateway_field_is_malformed() {
    let page: GetMessagesGatewayDTO = WireDecodeSupport::decode(
        r#"{
          "chat_id": "chat-1",
          "is_have_more": false,
          "messages": [{
            "id": "msg-1",
            "chat_id": "chat-1",
            "content": "visible",
            "created_at": "1784919481000",
            "timestamp": "1784919481000",
            "reactions": "malformed optional field",
            "recent_commenters": "[{\"logo\":\"a.png\",\"name\":\"Alice\"}]",
            "shared_post_info": "{\"post_id\":42,\"title\":\"Post\"}"
          }]
        }"#,
    );
    let Some(message) = page.messages.as_ref().and_then(|messages| messages.first()) else {
        panic!("the row survives");
    };

    assert_eq!(message.id.as_deref(), Some("msg-1"));
    assert_eq!(message.created_at, Some(1_784_919_481_000.0));
    assert!(message.reactions.is_none());
    assert_eq!(
        message.recent_commenters.as_ref().and_then(|list| list.first()).and_then(|first| first.name.as_deref()),
        Some("Alice")
    );
    assert_eq!(
        message.shared_post.as_ref().and_then(|post| post.post_id.as_ref()).map(|id| id.as_string()).as_deref(),
        Some("42")
    );
}

#[test]
fn message_history_decodes_gateway_commenter_array_and_shared_post_object() {
    let message: MessageWireDTO = WireDecodeSupport::decode(
        r#"{
          "id": "msg-2",
          "chat_id": "chat-1",
          "recent_commenters": [{ "logo": "b.png", "name": "Bob" }],
          "shared_post": { "post_id": "post-2", "title": "Shared" }
        }"#,
    );

    assert_eq!(
        message.recent_commenters.as_ref().and_then(|list| list.first()).and_then(|first| first.name.as_deref()),
        Some("Bob")
    );
    assert_eq!(
        message.shared_post.as_ref().and_then(|post| post.post_id.as_ref()).map(|id| id.as_string()).as_deref(),
        Some("post-2")
    );
}

#[gpui::test]
fn message_system_actor_decodes_and_survives_cache_round_trip(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let dto: MessageWireDTO = WireDecodeSupport::decode(
            r#"{
              "id": "system-pin-1",
              "chat_id": "bot-chat-1",
              "sender_id": "bot-user",
              "sender_name": "Example Bot",
              "content": "Example Bot pinned a message",
              "content_type": "system",
              "is_system_message": true,
              "system_message_type": "message_pinned",
              "system_actor_info": { "id": "human-user", "name": "Alice" }
            }"#,
        );
        let message = dto.to_message();
        let system = message.system.as_ref();
        assert_eq!(system.map(|system| system.kind), Some(MessageSystemKind::MessagePinned));
        assert_eq!(system.and_then(|system| system.actor_id.as_deref()), Some("human-user"));
        assert_eq!(system.and_then(|system| system.actor_name.as_deref()), Some("Alice"));

        let cached = MessageServicesStore::shared(cx).read(cx).wire_to_dict_from(&message);
        let cached = Value::Object(cached);
        let Some(round_trip) = MessagePayloadCodec::decode::<MessageWireDTO>(Some(&cached)) else {
            panic!("the cached row decodes");
        };
        let actor = round_trip.system_actor_info.as_ref();
        assert_eq!(actor.and_then(|actor| actor.id.as_deref()), Some("human-user"));
        assert_eq!(actor.and_then(|actor| actor.name.as_deref()), Some("Alice"));
        assert_eq!(round_trip.to_message().system.and_then(|system| system.actor_id).as_deref(), Some("human-user"));
    });
}

#[test]
fn post_view_history_decodes_keyset_pagination_and_post_shape() {
    let dto: GetViewHistoryGatewayDTO = WireDecodeSupport::decode(
        r#"{
          "list": [{
            "id": 42,
            "title": "Viewed post",
            "content": "Body",
            "author_id": "author-1",
            "created_at": "2026-07-14T10:00:00Z",
            "author": {
              "id": "author-1",
              "name": "Alice",
              "tag": "alice",
              "more": { "logo": "https://cdn.example/alice.jpg" }
            }
          }],
          "is_have_more": true,
          "next_cursor": 1784023200123,
          "next_cursor_post_id": 42
        }"#,
    );
    let Some(post) = dto.list.first().and_then(|item| item.as_feed_response()) else {
        panic!("the post maps to a feed response");
    };

    assert_eq!(post.id, "42");
    assert_eq!(post.author.id.as_deref(), Some("author-1"));
    assert_eq!(post.author.name, "Alice");
    assert!(dto.is_have_more);
    assert_eq!(dto.next_cursor, Some(1_784_023_200_123));
    assert_eq!(dto.next_cursor_post_id, Some(42));
}

#[test]
fn post_view_history_body_encodes_snake_case_cursor_pair() {
    let body = rise_request::post::GetViewHistory {
        cursor: Some(1_784_023_200_123),
        cursor_post_id: Some(42),
        limit: Some(20),
    };

    let json = WireDecodeSupport::encode_object(&body);
    assert!(json.get("user_id").is_none());
    assert_eq!(json.get("cursor").and_then(Value::as_i64), Some(1_784_023_200_123));
    assert_eq!(json.get("cursor_post_id").and_then(Value::as_i64), Some(42));
    assert!(json.get("cursorPostId").is_none());
}

#[test]
fn global_mute_effective_scopes_preserve_granular_exceptions() {
    let stored = ChatMuteScopes { messages: true, mentions: false, replies: false, reactions: true, events: false };

    assert_eq!(ChatServicesStore::effective_notification_scopes(stored.clone(), true, false), ChatMuteScopes::MUTED);
    assert_eq!(ChatServicesStore::effective_notification_scopes(stored.clone(), true, true), stored);
    assert_eq!(ChatServicesStore::effective_notification_scopes(stored.clone(), false, true), stored);
}
