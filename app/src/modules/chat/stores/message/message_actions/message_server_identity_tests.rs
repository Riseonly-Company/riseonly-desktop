use super::message_types::GetMessageMessage;

#[test]
fn server_id_wins_over_opaque_local_identity() {
    let mut message = GetMessageMessage::new("local-store-id".to_owned());
    message.server_id = Some("server-message-id".to_owned());

    assert_eq!(message.sai_server_id(), "server-message-id");
}

#[test]
fn local_identity_remains_fallback_before_server_confirmation() {
    let message = GetMessageMessage::new("local-store-id".to_owned());

    assert_eq!(message.sai_server_id(), "local-store-id");
}
