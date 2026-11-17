use crate::modules::chat::components::chat::message_read_receipt_menu::{
    MessageReadReceiptMenuMetrics, MessageReadReceiptMenuPolicy,
};
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::RiseConversationAuxiliaryLoadState;
use crate::modules::chat::engine::messages::rise_message_read_receipt_presentation::{
    RiseMessageReadReceiptPresentationSnapshot, RiseMessageReadReceiptScope, RiseMessageReadReceiptSummaryKind,
};
use crate::modules::chat::engine::messages::rise_message_read_receipt_wire_types::RiseMessageReadReceiptStoredResource;
use crate::modules::chat::shared::message_permissions::ChatMessageAction;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

fn chat(id: &str, chat_type: ChatType) -> ChatInfo {
    ChatInfo { id: id.to_owned(), r#type: chat_type, ..ChatInfo::default() }
}

fn snapshot(
    is_local_hint: bool,
    read_at_text: Option<&str>,
    source_revision: i64,
    load_state: RiseConversationAuxiliaryLoadState,
) -> RiseMessageReadReceiptPresentationSnapshot {
    RiseMessageReadReceiptPresentationSnapshot {
        account_id: "account".to_owned(),
        scope: RiseMessageReadReceiptScope::new("chat", "server-message-1"),
        source_revision,
        readers: Vec::new(),
        total_count: 0,
        summary_kind: Some(RiseMessageReadReceiptSummaryKind::Timestamp),
        representative_read_at_milliseconds: read_at_text.map(|_| 1),
        representative_read_at_text: read_at_text.map(str::to_owned),
        is_local_hint,
        load_state,
        error: None,
    }
}

#[test]
fn policy_offers_only_stable_read_outgoing_messages_in_supported_chats() {
    let mut message = GetMessageMessage {
        id: "message-1".to_owned(),
        server_id: Some("server-message-1".to_owned()),
        is_read: true,
        ..GetMessageMessage::default()
    };
    let stable = |message: &GetMessageMessage, chat_type: ChatType, is_own: bool| {
        MessageReadReceiptMenuPolicy::stable_message_id(message, &chat("chat", chat_type), is_own)
    };

    assert_eq!(stable(&message, ChatType::Direct, true).as_deref(), Some("server-message-1"));
    assert_eq!(stable(&message, ChatType::Group, true).as_deref(), Some("server-message-1"));
    assert_eq!(stable(&message, ChatType::Channel, true), None);
    assert_eq!(stable(&message, ChatType::Direct, false), None);

    message.is_read = false;
    assert_eq!(stable(&message, ChatType::Direct, true), None);
}

#[test]
fn read_receipt_profile_action_round_trips_user_id() {
    let user_id = "f9311a53-b8ef-44f2-870d-b799e1d49472";
    let action = ChatMessageAction::read_receipt_user_action(user_id);

    assert_eq!(ChatMessageAction::read_receipt_user_id(&action).as_deref(), Some(user_id));
    assert_eq!(ChatMessageAction::read_receipt_user_id("reply"), None);
    assert_eq!(ChatMessageAction::read_receipt_user_id(&ChatMessageAction::read_receipt_user_action(" ")), None);
}

#[test]
fn local_hint_resolves_direct_summary_but_never_a_group_one() {
    let stored = RiseMessageReadReceiptStoredResource::local_hint("server-message-1", 1_760_000_000_000);

    assert_eq!(stored.is_local_hint, Some(true));
    assert_eq!(stored.refreshed_at_milliseconds, 0);
    assert_eq!(stored.response.representative_read_at, Some(1_760_000_000_000));
    assert_eq!(stored.response.summary_kind.as_deref(), Some("timestamp"));

    let hint = snapshot(true, Some("13:20"), 4, RiseConversationAuxiliaryLoadState::Loaded);
    assert!(hint.has_resolved_summary(true));
    assert!(!hint.has_resolved_summary(false));

    let authoritative = snapshot(false, Some("13:20"), 4, RiseConversationAuxiliaryLoadState::Loaded);
    assert!(authoritative.has_resolved_summary(true));
    assert!(authoritative.has_resolved_summary(false));

    let cold = snapshot(false, None, 0, RiseConversationAuxiliaryLoadState::Loading);
    assert!(!cold.has_resolved_summary(true));
}

#[test]
fn placeholder_geometry_matches_the_resolved_list() {
    assert_eq!(
        MessageReadReceiptMenuMetrics::details_list_height(3),
        MessageReadReceiptMenuMetrics::DETAILS_ROW_HEIGHT * 3.0
    );
    assert_eq!(
        MessageReadReceiptMenuMetrics::details_list_height(999),
        MessageReadReceiptMenuMetrics::DETAILS_MAX_HEIGHT
    );
    assert_eq!(MessageReadReceiptMenuMetrics::placeholder_row_count(0), 4);
    assert_eq!(MessageReadReceiptMenuMetrics::placeholder_row_count(2), 2);
    assert!(
        MessageReadReceiptMenuMetrics::placeholder_row_count(500) as f32
            * MessageReadReceiptMenuMetrics::DETAILS_ROW_HEIGHT
            <= MessageReadReceiptMenuMetrics::DETAILS_MAX_HEIGHT
    );
}
