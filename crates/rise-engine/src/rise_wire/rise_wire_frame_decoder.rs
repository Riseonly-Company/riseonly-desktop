use std::collections::BTreeMap;

use rise_canon::rise_wire::{
    ERROR_CODE_GATEWAY_DEFAULT, ERROR_CODE_REMOTE_DEFAULT, KEY_CODE,
    KEY_DATA, KEY_DETAILS, KEY_ERROR, KEY_FRAME_TYPE, KEY_MESSAGE, KEY_REQUEST_ID, KEY_STATUS,
    KEY_TIMESTAMP, PAYLOAD_KEYS, STATUS_SUCCESS, parse_request_id as canon_parse_request_id,
};

use crate::rise_core::rise_identifiers::RiseRequestID;
use crate::rise_wire::rise_json_value::RiseJSONValue;
use crate::rise_wire::rise_wire_contracts::{
    RiseWireContractError, RiseWireIncomingFrame, RiseWirePushEnvelope, RiseWireRemoteError,
    RiseWireResponseEnvelope, RiseWireResponseStatus, RiseWireTimestamp,
};

pub(crate) const DEFAULT_REMOTE_MESSAGE: &str = "Remote request failed";

#[derive(Clone, Debug, Default)]
pub struct RiseWireFrameDecoder {
    frame_extras: BTreeMap<String, Vec<String>>,
}

impl RiseWireFrameDecoder {
    pub fn new(frame_extras: BTreeMap<String, Vec<String>>) -> Self {
        Self { frame_extras }
    }

    pub fn decode(&self, data: &[u8]) -> Result<RiseWireIncomingFrame, RiseWireContractError> {
        let root =
            RiseJSONValue::decode(data).map_err(|_| RiseWireContractError::InvalidFrame)?;
        let Some(object) = root.object_value() else {
            return Err(RiseWireContractError::InvalidFrame);
        };

        let error = self.remote_error(object);
        if let Some(raw_request_id) = object.get(KEY_REQUEST_ID) {
            let request_id = self.parse_request_id(raw_request_id)?;
            let status_text = object.get(KEY_STATUS).and_then(RiseJSONValue::string_value);
            let status = if error.is_some()
                || status_text.is_some_and(|text| RiseWireResponseStatus::new(text).is_error())
            {
                RiseWireResponseStatus::ERROR
            } else {
                RiseWireResponseStatus::new(status_text.unwrap_or(STATUS_SUCCESS))
            };
            let error = error.or_else(|| {
                status.is_error().then(|| {
                    RiseWireRemoteError::new(
                        self.scalar_string(object.get(KEY_CODE))
                            .unwrap_or_else(|| ERROR_CODE_REMOTE_DEFAULT.to_owned()),
                        self.scalar_string(object.get(KEY_MESSAGE))
                            .unwrap_or_else(|| DEFAULT_REMOTE_MESSAGE.to_owned()),
                        None,
                    )
                })
            });
            return Ok(RiseWireIncomingFrame::Response(RiseWireResponseEnvelope {
                request_id,
                status,
                data: self.response_payload(object).cloned(),
                timestamp: self.timestamp(object.get(KEY_TIMESTAMP)),
                error,
                r#type: object
                    .get(KEY_FRAME_TYPE)
                    .and_then(RiseJSONValue::string_value)
                    .map(str::to_owned),
            }));
        }

        let top_level_type = object.get(KEY_FRAME_TYPE).and_then(RiseJSONValue::string_value);
        let nested_type = object
            .get(KEY_DATA)
            .and_then(RiseJSONValue::object_value)
            .and_then(|data| data.get(KEY_FRAME_TYPE))
            .and_then(RiseJSONValue::string_value);
        if let Some(event_type) =
            [top_level_type, nested_type].into_iter().flatten().find(|value| !value.is_empty())
        {
            return Ok(RiseWireIncomingFrame::Push(RiseWirePushEnvelope::new(
                event_type.to_owned(),
                self.response_payload(object).cloned().unwrap_or_else(|| root.clone()),
                self.timestamp(object.get(KEY_TIMESTAMP)),
                self.extras(event_type, object),
            )));
        }
        if let Some(error) = error {
            return Ok(RiseWireIncomingFrame::GatewayError(error));
        }
        Err(RiseWireContractError::InvalidPushFrame)
    }

    /// lifts exactly the root keys declared for this event
    /// folding the whole root into data would change the parse shape of every push
    /// the key list comes from canon and not from a transport guess
    fn extras(
        &self,
        event_type: &str,
        object: &BTreeMap<String, RiseJSONValue>,
    ) -> BTreeMap<String, RiseJSONValue> {
        let Some(keys) = self.frame_extras.get(event_type).filter(|keys| !keys.is_empty()) else {
            return BTreeMap::new();
        };

        let mut lifted = BTreeMap::new();
        for key in keys {
            let Some(value) = object.get(key).filter(|value| **value != RiseJSONValue::Null)
            else {
                continue;
            };
            lifted.insert(key.clone(), value.clone());
        }
        lifted
    }

    fn response_payload<'a>(
        &self,
        object: &'a BTreeMap<String, RiseJSONValue>,
    ) -> Option<&'a RiseJSONValue> {
        PAYLOAD_KEYS.iter().find_map(|key| object.get(*key))
    }

    fn parse_request_id(
        &self,
        value: &RiseJSONValue,
    ) -> Result<RiseRequestID, RiseWireContractError> {
        let wire = serde_json::Value::from(value.clone());
        let raw = canon_parse_request_id(&wire)
            .map_err(|_| RiseWireContractError::InvalidRequestID)?;
        RiseRequestID::try_from(raw).map_err(|_| RiseWireContractError::InvalidRequestID)
    }

    fn timestamp(&self, value: Option<&RiseJSONValue>) -> Option<RiseWireTimestamp> {
        match value? {
            RiseJSONValue::Integer(number) => Some(RiseWireTimestamp::UnixSeconds(*number as f64)),
            RiseJSONValue::UnsignedInteger(number) => {
                Some(RiseWireTimestamp::UnixSeconds(*number as f64))
            }
            RiseJSONValue::Number(number) => Some(RiseWireTimestamp::UnixSeconds(*number)),
            RiseJSONValue::String(text) => Some(RiseWireTimestamp::Text(text.clone())),
            RiseJSONValue::Null
            | RiseJSONValue::Bool(_)
            | RiseJSONValue::Array(_)
            | RiseJSONValue::Object(_) => None,
        }
    }

    fn remote_error(
        &self,
        object: &BTreeMap<String, RiseJSONValue>,
    ) -> Option<RiseWireRemoteError> {
        let value = object.get(KEY_ERROR).filter(|value| **value != RiseJSONValue::Null)?;
        match value {
            RiseJSONValue::String(message) => {
                if message.is_empty() {
                    return None;
                }
                Some(RiseWireRemoteError::new(
                    self.scalar_string(object.get(KEY_CODE))
                        .unwrap_or_else(|| ERROR_CODE_GATEWAY_DEFAULT.to_owned()),
                    message.clone(),
                    None,
                ))
            }
            RiseJSONValue::Object(error) => {
                if error.is_empty() {
                    return None;
                }
                Some(RiseWireRemoteError::new(
                    self.scalar_string(error.get(KEY_CODE))
                        .or_else(|| self.scalar_string(object.get(KEY_CODE)))
                        .unwrap_or_else(|| ERROR_CODE_GATEWAY_DEFAULT.to_owned()),
                    self.scalar_string(error.get(KEY_MESSAGE))
                        .or_else(|| self.scalar_string(error.get(KEY_ERROR)))
                        .or_else(|| self.scalar_string(object.get(KEY_MESSAGE)))
                        .unwrap_or_else(|| DEFAULT_REMOTE_MESSAGE.to_owned()),
                    error.get(KEY_DETAILS).cloned(),
                ))
            }
            RiseJSONValue::Null
            | RiseJSONValue::Bool(_)
            | RiseJSONValue::Integer(_)
            | RiseJSONValue::UnsignedInteger(_)
            | RiseJSONValue::Number(_)
            | RiseJSONValue::Array(_) => Some(RiseWireRemoteError::new(
                self.scalar_string(object.get(KEY_CODE))
                    .unwrap_or_else(|| ERROR_CODE_GATEWAY_DEFAULT.to_owned()),
                self.scalar_string(Some(value))
                    .unwrap_or_else(|| DEFAULT_REMOTE_MESSAGE.to_owned()),
                None,
            )),
        }
    }

    fn scalar_string(&self, value: Option<&RiseJSONValue>) -> Option<String> {
        match value? {
            RiseJSONValue::String(text) => Some(text.clone()),
            RiseJSONValue::Integer(number) => Some(number.to_string()),
            RiseJSONValue::UnsignedInteger(number) => Some(number.to_string()),
            RiseJSONValue::Number(number) => Some(swift_double_description(*number)),
            RiseJSONValue::Bool(value) => Some(value.to_string()),
            RiseJSONValue::Null | RiseJSONValue::Array(_) | RiseJSONValue::Object(_) => None,
        }
    }
}

fn swift_double_description(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e16 {
        format!("{value:.1}")
    } else if value.abs() >= 1e16 {
        let text = format!("{value:e}");
        match text.split_once('e') {
            Some((mantissa, exponent)) if !exponent.starts_with('-') => {
                format!("{mantissa}e+{exponent}")
            }
            _ => text,
        }
    } else {
        value.to_string()
    }
}
