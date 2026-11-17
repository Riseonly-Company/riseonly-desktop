use futures::executor::block_on;
use futures::future::{Aborted, abortable};
use serde_json::Value;

use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{
    SaiWsInboundFrame, SaiWsInboundFrameDecoder,
};

#[test]
fn decodes_string_and_data_frames() {
    let source = r#"{"type":"event","payload":{"message":"hello"},"count":2}"#;
    let decoded_string =
        block_on(SaiWsInboundFrameDecoder::decode(SaiWsInboundFrame::String(source.to_owned())));
    let decoded_data =
        block_on(SaiWsInboundFrameDecoder::decode(SaiWsInboundFrame::Data(source.as_bytes().to_vec())));
    let string_result = decoded_string.ok().flatten().expect("string frame decodes");
    let data_result = decoded_data.ok().flatten().expect("data frame decodes");

    assert_eq!(string_result.value.get("type").and_then(Value::as_str), Some("event"));
    assert_eq!(
        string_result
            .value
            .get("payload")
            .and_then(|payload| payload.get("message"))
            .and_then(Value::as_str),
        Some("hello")
    );
    assert_eq!(data_result.value.get("count").and_then(Value::as_i64), Some(2));
}

#[test]
fn ignores_malformed_non_object_and_invalid_utf8_frames() {
    let malformed = block_on(SaiWsInboundFrameDecoder::decode(SaiWsInboundFrame::String(
        r#"{"type":"event""#.to_owned(),
    )));
    let array = block_on(SaiWsInboundFrameDecoder::decode(SaiWsInboundFrame::String(
        "[1,2,3]".to_owned(),
    )));
    let invalid_utf8 =
        block_on(SaiWsInboundFrameDecoder::decode(SaiWsInboundFrame::Data(vec![0xFF, 0xFE])));

    assert_eq!(malformed, Ok(None));
    assert_eq!(array, Ok(None));
    assert_eq!(invalid_utf8, Ok(None));
}

#[test]
fn propagates_cancellation_before_decode_starts() {
    let (operation, handle) = abortable(SaiWsInboundFrameDecoder::decode(SaiWsInboundFrame::String(
        r#"{"type":"event"}"#.to_owned(),
    )));
    handle.abort();

    let outcome = block_on(operation);

    assert!(
        matches!(outcome, Err(Aborted)),
        "A cancelled inbound decode must not succeed"
    );
}
