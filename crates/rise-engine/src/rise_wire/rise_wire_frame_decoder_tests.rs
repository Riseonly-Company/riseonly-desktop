use std::collections::BTreeMap;

use rise_canon::rise_wire::{ERROR_CODE_GATEWAY_DEFAULT, ERROR_CODE_REMOTE_DEFAULT};

use crate::rise_core::rise_identifiers::RiseRequestID;
use crate::rise_wire::rise_json_value::RiseJSONValue;
use crate::rise_wire::rise_wire_contracts::{
    RiseWireContractError, RiseWireIncomingFrame, RiseWireResponseStatus, RiseWireTimestamp,
};
use crate::rise_wire::rise_wire_frame_decoder::RiseWireFrameDecoder;

fn decode(frame: &str) -> Result<RiseWireIncomingFrame, RiseWireContractError> {
    RiseWireFrameDecoder::default().decode(frame.as_bytes())
}

#[test]
fn a_reply_without_status_is_a_success() {
    let Ok(RiseWireIncomingFrame::Response(response)) =
        decode(r#"{"request_id":"42","result":{"ok":true},"error":""}"#)
    else {
        panic!("expected a response");
    };
    assert_eq!(Some(response.request_id), RiseRequestID::new(42));
    assert!(response.status.is_success());
    assert!(response.error.is_none());
    assert_eq!(
        response.data.and_then(|data| data.subscript_key("ok").cloned()),
        Some(RiseJSONValue::Bool(true))
    );
}

#[test]
fn a_failure_status_without_error_gets_the_remote_default_code() {
    let Ok(RiseWireIncomingFrame::Response(response)) =
        decode(r#"{"request_id":7,"status":"unauthorized"}"#)
    else {
        panic!("expected a response");
    };
    assert_eq!(response.status, RiseWireResponseStatus::ERROR);
    assert_eq!(response.error.map(|error| error.code), Some(ERROR_CODE_REMOTE_DEFAULT.to_owned()));
}

#[test]
fn a_present_null_data_stays_null() {
    let Ok(RiseWireIncomingFrame::Response(response)) = decode(r#"{"request_id":3,"data":null}"#)
    else {
        panic!("expected a response");
    };
    assert_eq!(response.data, Some(RiseJSONValue::Null));
}

#[test]
fn a_push_wins_over_a_root_error_and_keeps_its_timestamp() {
    let Ok(RiseWireIncomingFrame::Push(push)) =
        decode(r#"{"type":"new_message","data":{"id":1},"error":"x","timestamp":1.5}"#)
    else {
        panic!("expected a push");
    };
    assert_eq!(push.r#type, "new_message");
    assert_eq!(push.timestamp, Some(RiseWireTimestamp::UnixSeconds(1.5)));
}

#[test]
fn only_declared_extras_are_lifted() {
    let mut frame_extras = BTreeMap::new();
    frame_extras.insert("typing".to_owned(), vec!["chat_id".to_owned(), "absent".to_owned()]);
    let decoder = RiseWireFrameDecoder::new(frame_extras);
    let Ok(RiseWireIncomingFrame::Push(push)) =
        decoder.decode(br#"{"type":"typing","data":{},"chat_id":"c1","other":"o","absent":null}"#)
    else {
        panic!("expected a push");
    };
    assert_eq!(push.extras.len(), 1);
    assert_eq!(push.extras.get("chat_id"), Some(&RiseJSONValue::String("c1".to_owned())));
}

#[test]
fn a_bare_error_is_a_gateway_error() {
    let Ok(RiseWireIncomingFrame::GatewayError(error)) = decode(r#"{"error":{"message":"m"}}"#)
    else {
        panic!("expected a gateway error");
    };
    assert_eq!(error.code, ERROR_CODE_GATEWAY_DEFAULT);
    assert_eq!(error.message, "m");
}

#[test]
fn malformed_frames_are_refused() {
    assert_eq!(decode("[1]"), Err(RiseWireContractError::InvalidFrame));
    assert_eq!(decode(r#"{"request_id":0}"#), Err(RiseWireContractError::InvalidRequestID));
    assert_eq!(decode(r#"{"error":""}"#), Err(RiseWireContractError::InvalidPushFrame));
}

#[test]
fn whole_doubles_decode_as_integers() {
    assert_eq!(RiseJSONValue::decode(b"5.0").ok(), Some(RiseJSONValue::Integer(5)));
    assert_eq!(RiseJSONValue::decode(b"5.5").ok(), Some(RiseJSONValue::Number(5.5)));
    assert_eq!(
        RiseJSONValue::decode(b"18446744073709551615").ok(),
        Some(RiseJSONValue::UnsignedInteger(u64::MAX))
    );
}
