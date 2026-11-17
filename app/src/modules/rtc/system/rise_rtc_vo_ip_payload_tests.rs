use rise_canon::rise_key::voip_push;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value, json};

use crate::modules::rtc::system::rise_rtc_system_models::{
    RiseRTCSystemMediaMode, RiseRTCSystemParsedVoIPPayload, RiseRTCSystemVoIPEndReason, RiseRTCSystemVoIPEvent,
    RiseRTCSystemVoIPHandlingDecision, RiseRTCSystemVoIPHandlingPolicy, RiseRTCSystemVoIPPayloadIssue, RiseRTCSystemVoIPPayloadParser,
    RiseRTCSystemVoIPServerPolicy,
};

const CALL_ID: &str = "2b132f78-e539-4a55-9ac2-c928405f2328";

const ALIAS_EVENT_TYPE: &str = "eventType";
const ALIAS_RECIPIENT_USER_ID: &str = "recipientUserId";
const ALIAS_CALL_ID: &str = "callId";
const ALIAS_EXPIRES_AT_MS: &str = "expiresAtMs";

fn payload(entries: Vec<(&str, Value)>) -> Map<String, Value> {
    entries.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

fn room_id() -> String {
    rise_rtc::call_room_id(CALL_ID)
}

#[test]
fn incoming_payload_decodes_backend_snake_case_contract() {
    let caller = payload(vec![
        (voip_push::ID, json!("user-a")),
        (voip_push::TAG, json!("aian")),
        (voip_push::NAME, json!("Aian")),
        (voip_push::AVATAR_URL, json!("https://cdn.example/a.jpg")),
    ]);
    let parsed = RiseRTCSystemVoIPPayloadParser::parse(&payload(vec![
        (voip_push::TYPE, json!("voip_call")),
        (voip_push::EVENT, json!("incoming")),
        (voip_push::RECIPIENT_USER_ID, json!("user-b")),
        (voip_push::CALL_ID, json!(CALL_ID)),
        (voip_push::ROOM_ID, json!(room_id())),
        (voip_push::CHAT_ID, json!("chat-1")),
        (voip_push::MODE, json!("video")),
        (voip_push::CREATED_AT_MS, json!(1_000)),
        (voip_push::EXPIRES_AT_MS, json!(31_000)),
        (voip_push::CALLER, Value::Object(caller)),
    ]));

    assert!(parsed.issues.is_empty());
    assert!(parsed.is_valid_incoming_invitation());
    assert_eq!(Some(parsed.envelope.call_uuid), Uuid::from_uuid_string(CALL_ID));
    assert_eq!(parsed.envelope.server_call_id.as_deref(), Some(CALL_ID));
    assert_eq!(parsed.envelope.recipient_user_id.as_deref(), Some("user-b"));
    assert_eq!(parsed.envelope.media_mode, RiseRTCSystemMediaMode::Video);
    assert_eq!(parsed.envelope.caller.as_ref().map(|caller| caller.display_name()).as_deref(), Some("Aian"));
    assert_eq!(parsed.envelope.caller.as_ref().map(|caller| caller.handle()).as_deref(), Some("aian"));
    assert_eq!(parsed.envelope.expires_at_milliseconds, Some(31_000));
}

#[test]
fn parser_tolerates_uppercase_mode_and_numeric_strings() {
    let caller = payload(vec![(voip_push::ID, json!("user-a")), (voip_push::NAME, json!("Caller"))]);
    let parsed = RiseRTCSystemVoIPPayloadParser::parse(&payload(vec![
        (voip_push::TYPE, json!("voip_call")),
        (voip_push::EVENT, json!("incoming")),
        (voip_push::RECIPIENT_USER_ID, json!("user-b")),
        (voip_push::CALL_ID, json!(CALL_ID)),
        (voip_push::ROOM_ID, json!(room_id())),
        (voip_push::MODE, json!("AUDIO")),
        (voip_push::EXPIRES_AT_MS, json!("42000")),
        (voip_push::CALLER, Value::Object(caller)),
    ]));

    assert!(parsed.issues.is_empty());
    assert_eq!(parsed.envelope.media_mode, RiseRTCSystemMediaMode::Audio);
    assert_eq!(parsed.envelope.expires_at_milliseconds, Some(42_000));
    assert_eq!(parsed.envelope.caller.as_ref().and_then(|caller| caller.id.as_deref()), Some("user-a"));
}

#[test]
fn parser_reads_only_canon_keys_and_rejects_camel_case_aliases() {
    let parsed = RiseRTCSystemVoIPPayloadParser::parse(&payload(vec![
        (voip_push::TYPE, json!("voip_call")),
        (ALIAS_EVENT_TYPE, json!("incoming_call")),
        (ALIAS_RECIPIENT_USER_ID, json!("user-b")),
        (ALIAS_CALL_ID, json!(CALL_ID)),
        (ALIAS_EXPIRES_AT_MS, json!("42000")),
    ]));

    assert!(parsed.issues.contains(&RiseRTCSystemVoIPPayloadIssue::UnknownEvent));
    assert!(parsed.issues.contains(&RiseRTCSystemVoIPPayloadIssue::MissingRecipient));
    assert!(parsed.issues.contains(&RiseRTCSystemVoIPPayloadIssue::InvalidCallID));
    assert_eq!(parsed.envelope.server_call_id, None);
    assert_eq!(parsed.envelope.expires_at_milliseconds, None);
}

#[test]
fn invalid_call_identifier_gets_stable_compliance_uuid_but_never_becomes_server_id() {
    let source = payload(vec![
        (voip_push::TYPE, json!("voip_call")),
        (voip_push::EVENT, json!("incoming")),
        (voip_push::RECIPIENT_USER_ID, json!("user-b")),
        (voip_push::CALL_ID, json!("not-a-uuid")),
        (voip_push::EXPIRES_AT_MS, json!(42_000)),
    ]);
    let first = RiseRTCSystemVoIPPayloadParser::parse(&source);
    let second = RiseRTCSystemVoIPPayloadParser::parse(&source);

    assert_eq!(first.envelope.call_uuid, second.envelope.call_uuid);
    assert_eq!(first.envelope.server_call_id, None);
    assert!(first.issues.contains(&RiseRTCSystemVoIPPayloadIssue::InvalidCallID));
    assert!(!first.is_valid_incoming_invitation());
}

#[test]
fn missing_recipient_fails_closed_even_for_single_account() {
    let parsed = RiseRTCSystemVoIPPayloadParser::parse(&payload(vec![
        (voip_push::TYPE, json!("voip_call")),
        (voip_push::EVENT, json!("incoming")),
        (voip_push::CALL_ID, json!(CALL_ID)),
        (voip_push::EXPIRES_AT_MS, json!(42_000)),
    ]));

    let decision = RiseRTCSystemVoIPHandlingPolicy::decision(&parsed, Some("user-b"), Some("epoch-b"), 7, 1_000);

    assert!(parsed.issues.contains(&RiseRTCSystemVoIPPayloadIssue::MissingRecipient));
    assert_eq!(decision, RiseRTCSystemVoIPHandlingDecision::ReportAndEnd(RiseRTCSystemVoIPEndReason::Failed));
}

#[test]
fn wrong_account_is_reported_then_ended_without_cross_account_mutation() {
    let parsed = valid_incoming("user-a", 42_000);
    let decision = RiseRTCSystemVoIPHandlingPolicy::decision(&parsed, Some("user-b"), Some("epoch-b"), 7, 1_000);

    assert_eq!(decision, RiseRTCSystemVoIPHandlingDecision::ReportAndEnd(RiseRTCSystemVoIPEndReason::Failed));
}

#[test]
fn expired_invitation_is_reported_then_marked_unanswered() {
    let parsed = valid_incoming("user-b", 999);
    let decision = RiseRTCSystemVoIPHandlingPolicy::decision(&parsed, Some("user-b"), Some("epoch-b"), 7, 1_000);

    assert_eq!(decision, RiseRTCSystemVoIPHandlingDecision::ReportAndEnd(RiseRTCSystemVoIPEndReason::Unanswered));
}

#[test]
fn valid_invitation_captures_account_epoch_and_process_generation() {
    let parsed = valid_incoming("user-b", 42_000);
    let decision = RiseRTCSystemVoIPHandlingPolicy::decision(&parsed, Some("user-b"), Some("epoch-b"), 7, 1_000);
    let RiseRTCSystemVoIPHandlingDecision::Ring(fence) = decision else {
        panic!("expected a ring decision, got {decision:?}");
    };

    assert_eq!(fence.account_id, "user-b");
    assert_eq!(fence.account_epoch.as_deref(), Some("epoch-b"));
    assert_eq!(fence.generation, 7);
}

#[test]
fn only_incoming_invitations_belong_on_vo_ip_transport() {
    assert!(RiseRTCSystemVoIPServerPolicy::may_use_vo_ip_transport(&RiseRTCSystemVoIPEvent::Incoming));
    assert!(!RiseRTCSystemVoIPServerPolicy::may_use_vo_ip_transport(&RiseRTCSystemVoIPEvent::Answered));
    assert!(RiseRTCSystemVoIPServerPolicy::may_use_vo_ip_transport(&RiseRTCSystemVoIPEvent::Ended));

    assert!(RiseRTCSystemVoIPServerPolicy::legacy_client_must_report_to_call_kit(&RiseRTCSystemVoIPEvent::Ended));
}

fn valid_incoming(recipient: &str, expires_at: i64) -> RiseRTCSystemParsedVoIPPayload {
    RiseRTCSystemVoIPPayloadParser::parse(&payload(vec![
        (voip_push::TYPE, json!("voip_call")),
        (voip_push::EVENT, json!("incoming")),
        (voip_push::RECIPIENT_USER_ID, json!(recipient)),
        (voip_push::CALL_ID, json!(CALL_ID)),
        (voip_push::ROOM_ID, json!(room_id())),
        (voip_push::MODE, json!("audio")),
        (voip_push::EXPIRES_AT_MS, json!(expires_at)),
    ]))
}
