use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::time::Duration;

use rise_canon::RiseWireRequestMetadata;
use rise_canon::rise_rpc::{ReplayPolicy, TIMEOUTS_DEFAULT_MS};
use rise_canon::rise_wire::{
    KEY_DATA, KEY_FRAME_TYPE, KEY_ID, KEY_METADATA, KEY_METHOD, KEY_SERVICE, KEY_TIMESTAMP,
    OUTBOUND_FRAME_TYPE, STATUS_ERROR, STATUS_FAILURE_VALUES, STATUS_SUCCESS,
};
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::rise_core::rise_identifiers::RiseRequestID;
use crate::rise_wire::rise_json_value::RiseJSONValue;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseWireReplayPolicy {
    Never,
    ReadOnly,
    RequestID,
    IdempotencyKey { field: String },
}

impl RiseWireReplayPolicy {
    pub fn permits_replay(&self) -> bool {
        self.canon_policy().permits_replay()
    }

    pub fn canon_policy(&self) -> ReplayPolicy {
        match self {
            RiseWireReplayPolicy::Never => ReplayPolicy::Never,
            RiseWireReplayPolicy::ReadOnly => ReplayPolicy::ReadOnly,
            RiseWireReplayPolicy::RequestID => ReplayPolicy::RequestId,
            RiseWireReplayPolicy::IdempotencyKey { .. } => ReplayPolicy::IdempotencyKey,
        }
    }
}

/// how a call reaches the gateway, http posts the same frame to POST /api/rpc
/// no socket, for calls a suspended app still makes where a socket costs a heartbeat
/// canon picks which methods go this way
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseWireTransport {
    Socket,
    Http,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseWireMethodDescriptor {
    pub service: String,
    pub method: String,
    pub timeout: Duration,
    pub replay_policy: RiseWireReplayPolicy,
    pub survives_suspension: bool,
    pub transport: RiseWireTransport,
}

impl RiseWireMethodDescriptor {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(TIMEOUTS_DEFAULT_MS);

    /// descriptor with all defaults, with_* builders set the rest
    pub fn new(service: impl Into<String>, method: impl Into<String>) -> Self {
        let service = service.into();
        let method = method.into();
        assert!(!trimming_whitespaces_and_newlines(&service).is_empty());
        assert!(!trimming_whitespaces_and_newlines(&method).is_empty());
        Self {
            service,
            method,
            timeout: Self::DEFAULT_TIMEOUT,
            replay_policy: RiseWireReplayPolicy::Never,
            survives_suspension: false,
            transport: RiseWireTransport::Socket,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_replay_policy(mut self, replay_policy: RiseWireReplayPolicy) -> Self {
        if let RiseWireReplayPolicy::IdempotencyKey { field } = &replay_policy {
            assert!(!trimming_whitespaces_and_newlines(field).is_empty());
        }
        self.replay_policy = replay_policy;
        self
    }

    pub fn with_survives_suspension(mut self, survives_suspension: bool) -> Self {
        self.survives_suspension = survives_suspension;
        self
    }

    pub fn with_transport(mut self, transport: RiseWireTransport) -> Self {
        self.transport = transport;
        self
    }
}

pub trait RiseWireMethod: Send + Sync + 'static {
    type Request: Serialize + Send + Sync;
    type Response: DeserializeOwned + Send;
    fn descriptor() -> RiseWireMethodDescriptor;
}

/// outbound request frame, keys and frame type come from the wire canon
#[derive(Clone, Debug)]
pub struct RiseWireRequestEnvelope<Body: Serialize + Send + Sync> {
    pub id: RiseRequestID,
    pub service: String,
    pub method: String,
    pub data: Body,
    pub timestamp: i64,
    pub metadata: RiseWireRequestMetadata,
}

impl<Body: Serialize + Send + Sync> RiseWireRequestEnvelope<Body> {
    pub fn new(
        id: RiseRequestID,
        service: String,
        method: String,
        data: Body,
        timestamp: i64,
        metadata: RiseWireRequestMetadata,
    ) -> Self {
        Self { id, service, method, data, timestamp, metadata }
    }
}

impl<Body: Serialize + Send + Sync> Serialize for RiseWireRequestEnvelope<Body> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let data = RiseJSONValue::encode(&self.data).map_err(S::Error::custom)?;
        let metadata = RiseJSONValue::encode(&self.metadata).map_err(S::Error::custom)?;
        let mut frame = BTreeMap::new();
        frame.insert(KEY_ID.to_owned(), RiseJSONValue::UnsignedInteger(self.id.raw_value()));
        frame.insert(
            KEY_FRAME_TYPE.to_owned(),
            RiseJSONValue::String(OUTBOUND_FRAME_TYPE.to_owned()),
        );
        frame.insert(KEY_SERVICE.to_owned(), RiseJSONValue::String(self.service.clone()));
        frame.insert(KEY_METHOD.to_owned(), RiseJSONValue::String(self.method.clone()));
        frame.insert(KEY_DATA.to_owned(), data);
        frame.insert(KEY_TIMESTAMP.to_owned(), RiseJSONValue::Integer(self.timestamp));
        frame.insert(KEY_METADATA.to_owned(), metadata);
        frame.serialize(serializer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RiseWireResponseStatus {
    pub raw_value: Cow<'static, str>,
}

impl RiseWireResponseStatus {
    pub const SUCCESS: Self = Self { raw_value: Cow::Borrowed(STATUS_SUCCESS) };
    pub const ERROR: Self = Self { raw_value: Cow::Borrowed(STATUS_ERROR) };

    pub const FAILURE_VALUES: [&'static str; 5] = STATUS_FAILURE_VALUES;

    pub fn new(raw_value: impl Into<String>) -> Self {
        Self { raw_value: Cow::Owned(raw_value.into()) }
    }

    pub fn is_success(&self) -> bool {
        compare_case_insensitive(&self.raw_value, STATUS_SUCCESS) == Ordering::Equal
    }

    pub fn is_error(&self) -> bool {
        Self::FAILURE_VALUES
            .iter()
            .any(|value| compare_case_insensitive(&self.raw_value, value) == Ordering::Equal)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RiseWireTimestamp {
    UnixSeconds(f64),
    Text(String),
}

impl Serialize for RiseWireTimestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            RiseWireTimestamp::UnixSeconds(value) => serializer.serialize_f64(*value),
            RiseWireTimestamp::Text(value) => serializer.serialize_str(value),
        }
    }
}

impl<'de> Deserialize<'de> for RiseWireTimestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match RiseJSONValue::deserialize(deserializer)? {
            RiseJSONValue::Integer(value) => Ok(RiseWireTimestamp::UnixSeconds(value as f64)),
            RiseJSONValue::UnsignedInteger(value) => {
                Ok(RiseWireTimestamp::UnixSeconds(value as f64))
            }
            RiseJSONValue::Number(value) => Ok(RiseWireTimestamp::UnixSeconds(value)),
            RiseJSONValue::String(value) => Ok(RiseWireTimestamp::Text(value)),
            _ => Err(D::Error::custom("expected a number or a string")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("RiseWireRemoteError(code: {code})")]
pub struct RiseWireRemoteError {
    pub code: String,
    pub message: String,
    pub details: Option<RiseJSONValue>,
}

impl RiseWireRemoteError {
    pub fn new(code: String, message: String, details: Option<RiseJSONValue>) -> Self {
        Self { code, message, details }
    }

    /// description is the code only, never the server message
    pub fn description(&self) -> String {
        self.to_string()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseWireResponseEnvelope {
    pub request_id: RiseRequestID,
    pub status: RiseWireResponseStatus,
    pub data: Option<RiseJSONValue>,
    pub timestamp: Option<RiseWireTimestamp>,
    pub error: Option<RiseWireRemoteError>,
    pub r#type: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseWirePushEnvelope {
    pub r#type: String,
    pub data: RiseJSONValue,
    pub timestamp: Option<RiseWireTimestamp>,
    pub extras: BTreeMap<String, RiseJSONValue>,
}

impl RiseWirePushEnvelope {
    pub fn new(
        r#type: String,
        data: RiseJSONValue,
        timestamp: Option<RiseWireTimestamp>,
        extras: BTreeMap<String, RiseJSONValue>,
    ) -> Self {
        assert!(!r#type.is_empty());
        Self { r#type, data, timestamp, extras }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RiseWireIncomingFrame {
    Response(RiseWireResponseEnvelope),
    Push(RiseWirePushEnvelope),
    GatewayError(RiseWireRemoteError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum RiseWireContractError {
    #[error("requestDataMustBeObject")]
    RequestDataMustBeObject,
    #[error("missingResponseData")]
    MissingResponseData,
    #[error("invalidFrame")]
    InvalidFrame,
    #[error("invalidPushFrame")]
    InvalidPushFrame,
    #[error("invalidRequestID")]
    InvalidRequestID,
}

pub struct RiseWireRequestEncoder;

impl RiseWireRequestEncoder {
    pub fn carries_object(request: &RiseJSONValue) -> bool {
        if request.object_value().is_some() {
            return true;
        }
        let Some(text) = request.string_value() else { return false };
        let Ok(decoded) = RiseJSONValue::decode(text.as_bytes()) else { return false };
        decoded.object_value().is_some()
    }

    pub fn encode(
        descriptor: &RiseWireMethodDescriptor,
        request: &RiseJSONValue,
        id: RiseRequestID,
        timestamp: i64,
        metadata: RiseWireRequestMetadata,
    ) -> anyhow::Result<Vec<u8>> {
        if !Self::carries_object(request) {
            return Err(RiseWireContractError::RequestDataMustBeObject.into());
        }
        let envelope = RiseWireRequestEnvelope::new(
            id,
            descriptor.service.clone(),
            descriptor.method.clone(),
            request,
            timestamp,
            metadata,
        );
        Ok(serde_json::to_vec(&envelope)?)
    }

    pub fn encode_method<Method: RiseWireMethod>(
        request: &Method::Request,
        id: RiseRequestID,
        timestamp: i64,
        metadata: RiseWireRequestMetadata,
    ) -> anyhow::Result<Vec<u8>> {
        let encoded_request = RiseJSONValue::encode(request)?;
        if encoded_request.object_value().is_none() {
            return Err(RiseWireContractError::RequestDataMustBeObject.into());
        }
        let descriptor = Method::descriptor();
        let envelope = RiseWireRequestEnvelope::new(
            id,
            descriptor.service,
            descriptor.method,
            encoded_request,
            timestamp,
            metadata,
        );
        Ok(serde_json::to_vec(&envelope)?)
    }
}
