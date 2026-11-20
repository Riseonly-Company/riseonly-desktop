use std::sync::{Arc, LazyLock};
use std::time::Duration;

use rise_canon::rise_key::rtc::{CALL_ID, IDEMPOTENCY_KEY};
use rise_canon::{RiseWireRequestMetadata, rise_wire_methods};
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::{RiseEngine, RiseJSONValue, RiseWireMethodDescriptor, RiseWireReplayPolicy};
use serde::de::DeserializeOwned;
use serde::ser::{Serialize, SerializeMap, Serializer};

use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout};

/// replay policy of an rtc endpoint
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseRTCEndpointReplay {
    ReadOnly,
    Never,
    IdempotencyKey(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCEndpoint {
    pub method: String,
    pub timeout: Duration,
    pub replay: RiseRTCEndpointReplay,
    pub survives_suspension: bool,
}

impl RiseRTCEndpoint {
    /// endpoint descriptor, ios defaults are 15 s
    /// never replay and no survive on suspension
    pub fn new(method: &str, timeout: Duration, replay: RiseRTCEndpointReplay, survives_suspension: bool) -> Self {
        Self { method: method.to_owned(), timeout, replay, survives_suspension }
    }

    /// descriptor from the canon catalogue, timeout and replay policy
    /// come from there and expected only checks the declared intent
    #[track_caller]
    pub fn canonical(method: &str, expected: RiseEndpointKind, survives_suspension: bool) -> Self {
        let entry = RiseCanonEndpoints::required(rise_wire_methods::rtc::SERVICE, method);
        debug_assert!(
            entry.kind == expected,
            "rtc.{method}: canon declares {}, not {}",
            entry.kind.name(),
            expected.name()
        );
        let replay = match entry.kind {
            RiseEndpointKind::Read => RiseRTCEndpointReplay::ReadOnly,
            RiseEndpointKind::Mutation | RiseEndpointKind::RequestIdMutation => RiseRTCEndpointReplay::Never,
            RiseEndpointKind::IdempotentMutation => {
                debug_assert!(
                    entry.idempotency_field.is_some(),
                    "{}: idempotent mutation without a deduplication field",
                    entry.name
                );
                RiseRTCEndpointReplay::IdempotencyKey(entry.idempotency_field.unwrap_or(IDEMPOTENCY_KEY).to_owned())
            }
        };
        Self { method: entry.method.to_owned(), timeout: entry.client_timeout(), replay, survives_suspension }
    }

    pub fn descriptor(&self) -> RiseWireMethodDescriptor {
        let policy = match &self.replay {
            RiseRTCEndpointReplay::ReadOnly => RiseWireReplayPolicy::ReadOnly,
            RiseRTCEndpointReplay::Never => RiseWireReplayPolicy::Never,
            RiseRTCEndpointReplay::IdempotencyKey(field) => RiseWireReplayPolicy::IdempotencyKey { field: field.clone() },
        };
        RiseWireMethodDescriptor::new(rise_wire_methods::rtc::SERVICE, self.method.clone())
            .with_timeout(self.timeout)
            .with_replay_policy(policy)
            .with_survives_suspension(self.survives_suspension)
    }
}

static ALL: LazyLock<Vec<RiseRTCEndpoint>> = LazyLock::new(|| {
    use rise_wire_methods::rtc;
    vec![
        RiseRTCEndpoints::mutation(rtc::JOIN_ROOM),
        RiseRTCEndpoints::live_call_mutation(rtc::LEAVE_ROOM),
        RiseRTCEndpoints::live_call_mutation(rtc::HEARTBEAT),
        RiseRTCEndpoints::live_call_read(rtc::GET_ROOM_SNAPSHOT),
        RiseRTCEndpoints::read(rtc::LIST_PARTICIPANTS),
        RiseRTCEndpoints::live_call_mutation(rtc::UPDATE_SELF_MEDIA),
        RiseRTCEndpoints::mutation(rtc::REQUEST_TO_SPEAK),
        RiseRTCEndpoints::mutation(rtc::CANCEL_REQUEST_TO_SPEAK),
        RiseRTCEndpoints::read(rtc::LIST_RAISED_HANDS),
        RiseRTCEndpoints::mutation(rtc::PROMOTE_TO_SPEAKER),
        RiseRTCEndpoints::mutation(rtc::MOVE_TO_AUDIENCE),
        RiseRTCEndpoints::mutation(rtc::MODERATE_PARTICIPANT),
        RiseRTCEndpoints::idempotent(rtc::SEND_REACTION),
        RiseRTCEndpoints::live_call_idempotent(rtc::START_DIRECT_CALL),
        RiseRTCEndpoints::live_call_mutation(rtc::ANSWER_CALL),
        RiseRTCEndpoints::live_call_mutation(rtc::REJOIN_CALL),
        RiseRTCEndpoints::live_call_mutation(rtc::DECLINE_CALL),
        RiseRTCEndpoints::live_call_mutation(rtc::CANCEL_CALL),
        RiseRTCEndpoints::live_call_mutation(rtc::END_CALL),
        RiseRTCEndpoints::live_call_mutation(rtc::START_CALL_RECORDING),
        RiseRTCEndpoints::live_call_mutation(rtc::STOP_CALL_RECORDING),
        RiseRTCEndpoints::live_call_read(rtc::GET_CALL),
        RiseRTCEndpoints::live_call_mutation(rtc::SET_STREAM_WATCH),
    ]
});

pub enum RiseRTCEndpoints {}

impl RiseRTCEndpoints {
    pub fn join_room() -> &'static RiseRTCEndpoint {
        &ALL[0]
    }

    pub fn leave_room() -> &'static RiseRTCEndpoint {
        &ALL[1]
    }

    pub fn heartbeat() -> &'static RiseRTCEndpoint {
        &ALL[2]
    }

    pub fn get_room_snapshot() -> &'static RiseRTCEndpoint {
        &ALL[3]
    }

    pub fn list_participants() -> &'static RiseRTCEndpoint {
        &ALL[4]
    }

    pub fn update_self_media() -> &'static RiseRTCEndpoint {
        &ALL[5]
    }

    pub fn request_to_speak() -> &'static RiseRTCEndpoint {
        &ALL[6]
    }

    pub fn cancel_request_to_speak() -> &'static RiseRTCEndpoint {
        &ALL[7]
    }

    pub fn list_raised_hands() -> &'static RiseRTCEndpoint {
        &ALL[8]
    }

    pub fn promote_to_speaker() -> &'static RiseRTCEndpoint {
        &ALL[9]
    }

    pub fn move_to_audience() -> &'static RiseRTCEndpoint {
        &ALL[10]
    }

    pub fn moderate_participant() -> &'static RiseRTCEndpoint {
        &ALL[11]
    }

    pub fn send_reaction() -> &'static RiseRTCEndpoint {
        &ALL[12]
    }

    pub fn start_direct_call() -> &'static RiseRTCEndpoint {
        &ALL[13]
    }

    pub fn answer_call() -> &'static RiseRTCEndpoint {
        &ALL[14]
    }

    pub fn rejoin_call() -> &'static RiseRTCEndpoint {
        &ALL[15]
    }

    pub fn decline_call() -> &'static RiseRTCEndpoint {
        &ALL[16]
    }

    pub fn cancel_call() -> &'static RiseRTCEndpoint {
        &ALL[17]
    }

    pub fn end_call() -> &'static RiseRTCEndpoint {
        &ALL[18]
    }

    pub fn start_call_recording() -> &'static RiseRTCEndpoint {
        &ALL[19]
    }

    pub fn stop_call_recording() -> &'static RiseRTCEndpoint {
        &ALL[20]
    }

    pub fn get_call() -> &'static RiseRTCEndpoint {
        &ALL[21]
    }

    pub fn set_stream_watch() -> &'static RiseRTCEndpoint {
        &ALL[22]
    }

    pub fn all() -> &'static [RiseRTCEndpoint] {
        &ALL
    }

    pub fn endpoint(method: &str) -> Option<&'static RiseRTCEndpoint> {
        let method = rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines(method);
        ALL.iter().find(|endpoint| endpoint.method == method)
    }

    #[track_caller]
    fn read(method: &str) -> RiseRTCEndpoint {
        RiseRTCEndpoint::canonical(method, RiseEndpointKind::Read, false)
    }

    #[track_caller]
    fn live_call_read(method: &str) -> RiseRTCEndpoint {
        RiseRTCEndpoint::canonical(method, RiseEndpointKind::Read, true)
    }

    #[track_caller]
    fn live_call_mutation(method: &str) -> RiseRTCEndpoint {
        RiseRTCEndpoint::canonical(method, RiseEndpointKind::Mutation, true)
    }

    #[track_caller]
    fn live_call_idempotent(method: &str) -> RiseRTCEndpoint {
        RiseRTCEndpoint::canonical(method, RiseEndpointKind::IdempotentMutation, true)
    }

    #[track_caller]
    fn mutation(method: &str) -> RiseRTCEndpoint {
        RiseRTCEndpoint::canonical(method, RiseEndpointKind::Mutation, false)
    }

    #[track_caller]
    fn idempotent(method: &str) -> RiseRTCEndpoint {
        RiseRTCEndpoint::canonical(method, RiseEndpointKind::IdempotentMutation, false)
    }
}

#[derive(Clone)]
pub struct RiseRTCRPC {
    pub engine: Arc<RiseEngine>,
}

impl RiseRTCRPC {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { engine }
    }

    /// calls the endpoint, key casing is the serde attributes of the canon request and response types
    pub async fn call<Request, Response>(&self, endpoint: &RiseRTCEndpoint, request: Request) -> anyhow::Result<Response>
    where
        Request: Serialize + Send + Sync,
        Response: DeserializeOwned,
    {
        let body = RiseJSONValue::encode(&request)?;
        let payload = self
            .engine
            .call_descriptor(&endpoint.descriptor(), &body, None, RiseWireRequestMetadata::default())
            .await?;
        Ok(payload.decode_as::<Response>()?)
    }
}

pub trait RiseRTCDecodingErrorExt {
    fn rise_rtc_safe_summary(&self) -> String;
}

impl RiseRTCDecodingErrorExt for serde_json::Error {
    fn rise_rtc_safe_summary(&self) -> String {
        let message = self.to_string();
        let (kind, coding_path) = match self.classify() {
            serde_json::error::Category::Data => {
                if let Some(field) = missing_field(&message) {
                    ("key_not_found", field)
                } else if message.starts_with("invalid type") || message.starts_with("invalid value") {
                    ("type_mismatch", String::new())
                } else if message.starts_with("invalid length") {
                    ("value_not_found", String::new())
                } else {
                    ("data_corrupted", String::new())
                }
            }
            serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
                ("data_corrupted", String::new())
            }
            serde_json::error::Category::Io => ("unknown", String::new()),
        };
        format!(
            "decode_kind={kind} coding_path={}",
            if coding_path.is_empty() { "<root>" } else { coding_path.as_str() }
        )
    }
}

fn missing_field(message: &str) -> Option<String> {
    let rest = message.strip_prefix("missing field `")?;
    let end = rest.find('`')?;
    Some(rest[..end].to_owned())
}

pub trait RiseRTCErrorExt {
    fn rise_rtc_safe_decoding_summary(&self) -> Option<String>;
}

impl RiseRTCErrorExt for anyhow::Error {
    fn rise_rtc_safe_decoding_summary(&self) -> Option<String> {
        self.downcast_ref::<serde_json::Error>().map(RiseRTCDecodingErrorExt::rise_rtc_safe_summary)
    }
}

/// rtc.start_call_recording is in the canon catalogue but has no request contract
/// or gateway binding yet, so its body is the one rtc request still spelled here
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCStartCallRecordingBody {
    pub call_id: String,
    pub idempotency_key: String,
}

impl Serialize for RiseRTCStartCallRecordingBody {
    fn serialize<S: Serializer>(&self, encoder: S) -> Result<S::Ok, S::Error> {
        let mut container = encoder.serialize_map(Some(2))?;
        container.serialize_entry(CALL_ID, &self.call_id)?;
        container.serialize_entry(IDEMPOTENCY_KEY, &self.idempotency_key)?;
        container.end()
    }
}
