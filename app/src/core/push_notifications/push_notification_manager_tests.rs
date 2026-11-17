use serde_json::{Map, Value, json};

use crate::core::push_notifications::push_notification_manager::PushRecipientAccountGate;

fn payload(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("test payload is an object")
}

#[test]
fn matching_recipient_is_accepted_with_multiple_accounts() {
    let result = PushRecipientAccountGate::should_handle(
        &payload(json!({ "recipient_user_id": "user-b" })),
        Some("user-b"),
        3,
    );

    assert!(result);
}

#[test]
fn different_recipient_is_rejected_even_with_one_account() {
    let result = PushRecipientAccountGate::should_handle(
        &payload(json!({ "recipient_user_id": "user-a" })),
        Some("user-b"),
        1,
    );

    assert!(!result);
}

#[test]
fn legacy_payload_is_accepted_with_exactly_one_account() {
    let result = PushRecipientAccountGate::should_handle(
        &payload(json!({ "type": "new_message" })),
        Some("user-a"),
        1,
    );

    assert!(result);
}

#[test]
fn legacy_payload_is_rejected_with_multiple_accounts() {
    let result = PushRecipientAccountGate::should_handle(
        &payload(json!({ "type": "new_message" })),
        Some("user-a"),
        2,
    );

    assert!(!result);
}

#[test]
fn malformed_recipient_is_rejected_instead_of_falling_back_to_legacy() {
    let result = PushRecipientAccountGate::should_handle(
        &payload(json!({ "recipient_user_id": "   " })),
        Some("user-a"),
        1,
    );

    assert!(!result);
}

#[test]
fn camel_case_recipient_alias_is_not_a_contract_key() {
    let result = PushRecipientAccountGate::should_handle(
        &payload(json!({ "recipientUserId": "user-a" })),
        Some(" user-a "),
        2,
    );

    assert!(!result);
}

#[test]
fn notification_is_rejected_without_an_active_account() {
    let result = PushRecipientAccountGate::should_handle(
        &payload(json!({ "recipient_user_id": "user-a" })),
        None,
        1,
    );

    assert!(!result);
}
