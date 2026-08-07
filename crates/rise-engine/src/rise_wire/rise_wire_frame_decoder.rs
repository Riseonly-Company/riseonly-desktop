use serde_json::Value;

use rise_core::RequestId;

use super::rise_wire_contracts::{RemoteError, ResponseStatus};

#[derive(Clone, PartialEq, Debug)]
pub enum IncomingFrame {
    Response {
        request_id: RequestId,
        status: ResponseStatus,
        payload: Value,
        error: Option<RemoteError>,
    },
    Push {
        event_type: String,
        payload: Value,
    },
    GatewayError(RemoteError),
    Unrecognized,
}

/// Classifies an inbound frame the way the deployed gateway actually behaves,
/// not the way the proto suggests it should.
///
/// Three deviations are load-bearing and each is covered by a test below:
/// `status` is omitted on success, the payload lives under `data` or `result`
/// depending on the service, and `request_id` arrives as a number or a string.
pub fn decode_frame(root: &Value) -> IncomingFrame {
    if let Some(request_id) = root.get("request_id").and_then(parse_request_id) {
        let error = root.get("error").and_then(parse_error);
        let status = if error.is_some() || says_error(root) {
            ResponseStatus::Error
        } else {
            ResponseStatus::Success
        };

        return IncomingFrame::Response {
            request_id,
            status,
            payload: payload_of(root),
            error,
        };
    }

    if let Some(error) = root.get("error").and_then(parse_error) {
        return IncomingFrame::GatewayError(error);
    }

    if let Some(event_type) = push_type(root) {
        return IncomingFrame::Push {
            event_type,
            payload: push_payload(root),
        };
    }

    IncomingFrame::Unrecognized
}

fn says_error(root: &Value) -> bool {
    root.get("status")
        .and_then(Value::as_str)
        .is_some_and(|s| s.eq_ignore_ascii_case("error"))
}

fn payload_of(root: &Value) -> Value {
    root.get("data")
        .or_else(|| root.get("result"))
        .cloned()
        .unwrap_or(Value::Null)
}

fn push_type(root: &Value) -> Option<String> {
    root.get("type")
        .and_then(Value::as_str)
        .or_else(|| {
            root.get("data")
                .and_then(|d| d.get("type"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
}

fn push_payload(root: &Value) -> Value {
    match root.get("data").or_else(|| root.get("result")) {
        Some(value) => value.clone(),
        None => root.clone(),
    }
}

fn parse_request_id(value: &Value) -> Option<RequestId> {
    let raw = match value {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|v| u64::try_from(v).ok()))
            .or_else(|| n.as_f64().map(|v| v as u64))?,
        Value::String(s) => s.parse::<u64>().ok()?,
        _ => return None,
    };
    RequestId::new(raw)
}

fn parse_error(value: &Value) -> Option<RemoteError> {
    match value {
        Value::Null => None,
        Value::String(message) => Some(RemoteError {
            code: None,
            message: message.clone(),
        }),
        Value::Object(map) => Some(RemoteError {
            code: map.get("code").and_then(Value::as_str).map(str::to_owned),
            message: map
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string()),
        }),
        other => Some(RemoteError {
            code: None,
            message: other.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(value: Value) -> (RequestId, ResponseStatus, Value, Option<RemoteError>) {
        match decode_frame(&value) {
            IncomingFrame::Response {
                request_id,
                status,
                payload,
                error,
            } => (request_id, status, payload, error),
            other => panic!("expected a response, got {other:?}"),
        }
    }

    #[test]
    fn presence_of_request_id_makes_it_a_response() {
        let (id, status, payload, error) = response(json!({"request_id": 7, "data": {"ok": true}}));
        assert_eq!(id.get(), 7);
        assert_eq!(status, ResponseStatus::Success);
        assert_eq!(payload["ok"], true);
        assert!(error.is_none());
    }

    #[test]
    fn a_missing_status_field_still_means_success() {
        let (_, status, _, _) = response(json!({"request_id": 1, "data": {}}));
        assert_eq!(
            status,
            ResponseStatus::Success,
            "the deployed gateway omits status on success"
        );
    }

    #[test]
    fn payload_falls_back_from_data_to_result() {
        let (_, _, payload, _) = response(json!({"request_id": 2, "result": {"v": 9}}));
        assert_eq!(payload["v"], 9);
    }

    #[test]
    fn request_id_parses_from_number_float_and_string() {
        for raw in [json!(11), json!(11.0), json!("11")] {
            let (id, _, _, _) = response(json!({"request_id": raw, "data": {}}));
            assert_eq!(id.get(), 11);
        }
    }

    #[test]
    fn an_error_object_marks_the_response_as_failed() {
        let (_, status, _, error) = response(json!({
            "request_id": 3,
            "error": {"code": "AUTH_ERROR", "message": "token expired"}
        }));
        assert_eq!(status, ResponseStatus::Error);
        let error = error.unwrap();
        assert_eq!(error.code.as_deref(), Some("AUTH_ERROR"));
        assert_eq!(error.message, "token expired");
    }

    #[test]
    fn a_bare_string_error_is_accepted() {
        let (_, status, _, error) = response(json!({"request_id": 4, "error": "something broke"}));
        assert_eq!(status, ResponseStatus::Error);
        assert_eq!(error.unwrap().message, "something broke");
    }

    #[test]
    fn an_explicit_error_status_without_an_error_object_still_fails() {
        let (_, status, _, error) = response(json!({"request_id": 5, "status": "error"}));
        assert_eq!(status, ResponseStatus::Error);
        assert!(error.is_none());
    }

    #[test]
    fn a_frame_without_request_id_but_with_type_is_a_push() {
        match decode_frame(&json!({"type": "message_created", "data": {"id": "m1"}})) {
            IncomingFrame::Push {
                event_type,
                payload,
            } => {
                assert_eq!(event_type, "message_created");
                assert_eq!(payload["id"], "m1");
            }
            other => panic!("expected a push, got {other:?}"),
        }
    }

    #[test]
    fn a_push_type_nested_under_data_is_found() {
        match decode_frame(&json!({"data": {"type": "presence_changed", "user_id": "u9"}})) {
            IncomingFrame::Push {
                event_type,
                payload,
            } => {
                assert_eq!(event_type, "presence_changed");
                assert_eq!(payload["user_id"], "u9");
            }
            other => panic!("expected a push, got {other:?}"),
        }
    }

    #[test]
    fn an_error_without_request_id_is_a_gateway_error_not_a_response() {
        match decode_frame(&json!({"error": {"code": "UNAUTHORIZED", "message": "no"}})) {
            IncomingFrame::GatewayError(error) => {
                assert_eq!(error.code.as_deref(), Some("UNAUTHORIZED"));
            }
            other => panic!("expected a gateway error, got {other:?}"),
        }
    }

    #[test]
    fn a_zero_request_id_is_not_a_valid_response() {
        assert_eq!(
            decode_frame(&json!({"request_id": 0, "data": {}})),
            IncomingFrame::Unrecognized
        );
    }

    #[test]
    fn an_unclassifiable_frame_is_reported_rather_than_guessed() {
        assert_eq!(
            decode_frame(&json!({"hello": "world"})),
            IncomingFrame::Unrecognized
        );
    }
}
