use crate::modules::chat::engine::messages::rise_call_history_message::RiseCallHistoryMessage;

#[test]
fn message_id_matches_the_server_uuid_v5() {
    assert_eq!(
        RiseCallHistoryMessage::message_id("abc-1").as_deref(),
        Some("286db27b-9083-5b5c-96e0-84ea1072d27d")
    );
}

#[test]
fn message_id_normalizes_the_call_id_before_hashing() {
    assert_eq!(RiseCallHistoryMessage::message_id("  ABC-1\n"), RiseCallHistoryMessage::message_id("abc-1"));
}

#[test]
fn message_id_refuses_a_blank_call_id() {
    assert_eq!(RiseCallHistoryMessage::message_id(" \n"), None);
}
