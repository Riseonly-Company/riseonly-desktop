use std::collections::{BTreeMap, HashSet};

use rise_canon::rise_key::rtc::ANSWERED_ELSEWHERE;
use rise_canon::rise_key::voip_push;
use rise_core::foundation::foundation_crypto::Sha256;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_rtc::{CallEventType, CallMediaMode, RtcWireEnum, VoipPushType};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::modules::rtc::engine::rise_rtc_engine_models::{RiseRTCCallState, RiseRTCConnectionState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiseRTCSystemMediaMode {
    Audio,
    Video,
}

impl RiseRTCSystemMediaMode {
    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        if raw_value == CallMediaMode::Audio.wire() {
            Some(Self::Audio)
        } else if raw_value == CallMediaMode::Video.wire() {
            Some(Self::Video)
        } else {
            None
        }
    }

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Audio => CallMediaMode::Audio.wire(),
            Self::Video => CallMediaMode::Video.wire(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseRTCSystemVoIPEvent {
    Incoming,
    Cancelled,
    Ended,
    Answered,
    AnsweredElsewhere,
    Declined,
    TimedOut,
    Unknown(String),
}

const LEGACY_INCOMING: &str = "incoming";
const LEGACY_CANCELED: &str = "canceled";
const LEGACY_CALL_CANCELED: &str = "call_canceled";
const LEGACY_ANSWERED: &str = "answered";
const LEGACY_ANSWERED_ELSEWHERE_FOLDED: &str = "answeredelsewhere";
const LEGACY_TIMEOUT: &str = "timeout";

impl RiseRTCSystemVoIPEvent {
    pub fn from_raw_value(raw_value: Option<&str>) -> Self {
        let value = raw_value.map(|raw| trimming_whitespaces_and_newlines(raw).to_lowercase()).unwrap_or_default();
        let token = value.as_str();
        if token == LEGACY_INCOMING || token == CallEventType::IncomingCall.wire() {
            Self::Incoming
        } else if token == RiseRTCCallState::Cancelled.wire()
            || token == LEGACY_CANCELED
            || token == CallEventType::CallCancelled.wire()
            || token == LEGACY_CALL_CANCELED
        {
            Self::Cancelled
        } else if token == RiseRTCCallState::Ended.wire() || token == CallEventType::CallEnded.wire() {
            Self::Ended
        } else if token == LEGACY_ANSWERED || token == CallEventType::CallAnswered.wire() {
            Self::Answered
        } else if token == ANSWERED_ELSEWHERE || token == LEGACY_ANSWERED_ELSEWHERE_FOLDED {
            Self::AnsweredElsewhere
        } else if token == RiseRTCCallState::Declined.wire() || token == CallEventType::CallDeclined.wire() {
            Self::Declined
        } else if token == RiseRTCCallState::TimedOut.wire()
            || token == LEGACY_TIMEOUT
            || token == CallEventType::CallTimedOut.wire()
        {
            Self::TimedOut
        } else {
            Self::Unknown(value)
        }
    }

    pub fn raw_value(&self) -> &str {
        match self {
            Self::Incoming => LEGACY_INCOMING,
            Self::Cancelled => RiseRTCCallState::Cancelled.wire(),
            Self::Ended => RiseRTCCallState::Ended.wire(),
            Self::Answered => LEGACY_ANSWERED,
            Self::AnsweredElsewhere => ANSWERED_ELSEWHERE,
            Self::Declined => RiseRTCCallState::Declined.wire(),
            Self::TimedOut => RiseRTCCallState::TimedOut.wire(),
            Self::Unknown(value) => value,
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Incoming | Self::Answered)
    }
}

pub enum RiseRTCCallSystemEventMapper {}

impl RiseRTCCallSystemEventMapper {
    pub fn event(state: RiseRTCCallState) -> Option<RiseRTCSystemVoIPEvent> {
        match state {
            RiseRTCCallState::Accepted => Some(RiseRTCSystemVoIPEvent::Answered),
            RiseRTCCallState::Declined => Some(RiseRTCSystemVoIPEvent::Declined),
            RiseRTCCallState::Cancelled => Some(RiseRTCSystemVoIPEvent::Cancelled),
            RiseRTCCallState::TimedOut => Some(RiseRTCSystemVoIPEvent::TimedOut),
            RiseRTCCallState::Ended => Some(RiseRTCSystemVoIPEvent::Ended),
            RiseRTCCallState::Unspecified | RiseRTCCallState::Ringing => None,
        }
    }
}

/// connected needs backend acceptance and media both, early sfu link alone isnt answered
pub enum RiseRTCCallActivationGate {}

impl RiseRTCCallActivationGate {
    pub fn is_ready(call_state: RiseRTCCallState, media_connection_state: RiseRTCConnectionState) -> bool {
        call_state == RiseRTCCallState::Accepted && media_connection_state == RiseRTCConnectionState::Connected
    }
}

const CALLER_DISPLAY_NAME_FALLBACK: &str = "Riseonly";
const CALLER_HANDLE_FALLBACK: &str = "riseonly";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCSystemCaller {
    pub id: Option<String>,
    pub tag: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

impl RiseRTCSystemCaller {
    pub fn display_name(&self) -> String {
        for candidate in [&self.name, &self.tag] {
            if let Some(value) = candidate.as_deref().map(trimming_whitespaces_and_newlines)
                && !value.is_empty()
            {
                return value.to_owned();
            }
        }
        CALLER_DISPLAY_NAME_FALLBACK.to_owned()
    }

    pub fn handle(&self) -> String {
        for candidate in [&self.tag, &self.id, &self.name] {
            if let Some(value) = candidate.as_deref().map(trimming_whitespaces_and_newlines)
                && !value.is_empty()
            {
                return value.to_owned();
            }
        }
        CALLER_HANDLE_FALLBACK.to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCSystemIncomingEnvelope {
    pub call_uuid: Uuid,
    pub server_call_id: Option<String>,
    pub room_id: Option<String>,
    pub chat_id: Option<String>,
    pub recipient_user_id: Option<String>,
    pub media_mode: RiseRTCSystemMediaMode,
    pub event: RiseRTCSystemVoIPEvent,
    pub caller: Option<RiseRTCSystemCaller>,
    pub created_at_milliseconds: Option<i64>,
    pub expires_at_milliseconds: Option<i64>,
    pub occurred_at_milliseconds: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseRTCSystemVoIPPayloadIssue {
    UnexpectedType,
    UnknownEvent,
    InvalidCallID,
    MissingRecipient,
    MissingExpiry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCSystemParsedVoIPPayload {
    pub envelope: RiseRTCSystemIncomingEnvelope,
    pub issues: HashSet<RiseRTCSystemVoIPPayloadIssue>,
}

impl RiseRTCSystemParsedVoIPPayload {
    pub fn is_valid_incoming_invitation(&self) -> bool {
        self.envelope.event == RiseRTCSystemVoIPEvent::Incoming && self.issues.is_empty()
    }
}

pub enum RiseRTCSystemVoIPPayloadParser {}

impl RiseRTCSystemVoIPPayloadParser {
    pub fn parse(payload: &Map<String, Value>) -> RiseRTCSystemParsedVoIPPayload {
        let reader = RiseRTCSystemPayloadReader::new(payload);
        let payload_type = reader.string(&[voip_push::TYPE]).map(|value| value.to_lowercase());
        let event = RiseRTCSystemVoIPEvent::from_raw_value(reader.string(&[voip_push::EVENT]).as_deref());
        let raw_call_id = reader.string(&[voip_push::CALL_ID]);
        let valid_call_id = raw_call_id.as_deref().and_then(Uuid::from_uuid_string);
        let call_uuid = valid_call_id.unwrap_or_else(|| RiseRTCSystemFallbackCallUUID::make(payload));
        let recipient = reader.string(&[voip_push::RECIPIENT_USER_ID]);
        let expires_at = reader.int64(&[voip_push::EXPIRES_AT_MS]);

        let mut issues = HashSet::new();
        if payload_type.as_deref() != Some(VoipPushType::VoipCall.wire()) {
            issues.insert(RiseRTCSystemVoIPPayloadIssue::UnexpectedType);
        }
        if matches!(event, RiseRTCSystemVoIPEvent::Unknown(_)) {
            issues.insert(RiseRTCSystemVoIPPayloadIssue::UnknownEvent);
        }
        if valid_call_id.is_none() {
            issues.insert(RiseRTCSystemVoIPPayloadIssue::InvalidCallID);
        }
        if recipient.is_none() {
            issues.insert(RiseRTCSystemVoIPPayloadIssue::MissingRecipient);
        }
        if event == RiseRTCSystemVoIPEvent::Incoming && expires_at.is_none() {
            issues.insert(RiseRTCSystemVoIPPayloadIssue::MissingExpiry);
        }

        let mode_raw = reader.string(&[voip_push::MODE]).map(|value| value.to_lowercase()).unwrap_or_default();
        let mode = RiseRTCSystemMediaMode::from_raw_value(&mode_raw).unwrap_or(RiseRTCSystemMediaMode::Audio);
        let caller = reader.dictionary(&[voip_push::CALLER]).map(|caller_payload| {
            let caller_reader = RiseRTCSystemPayloadReader::new(caller_payload);
            RiseRTCSystemCaller {
                id: caller_reader.string(&[voip_push::ID]),
                tag: caller_reader.string(&[voip_push::TAG]),
                name: caller_reader.string(&[voip_push::NAME]),
                avatar_url: caller_reader.string(&[voip_push::AVATAR_URL]),
            }
        });

        RiseRTCSystemParsedVoIPPayload {
            envelope: RiseRTCSystemIncomingEnvelope {
                call_uuid,
                server_call_id: if valid_call_id.is_none() { None } else { raw_call_id },
                room_id: reader.string(&[voip_push::ROOM_ID]),
                chat_id: reader.string(&[voip_push::CHAT_ID]),
                recipient_user_id: recipient,
                media_mode: mode,
                event,
                caller,
                created_at_milliseconds: reader.int64(&[voip_push::CREATED_AT_MS]),
                expires_at_milliseconds: expires_at,
                occurred_at_milliseconds: reader.int64(&[voip_push::OCCURRED_AT_MS]),
                reason: reader.string(&[voip_push::END_REASON]),
            },
            issues,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseRTCSystemAccountFence {
    pub account_id: String,
    pub account_epoch: Option<String>,
    pub generation: u64,
}

impl RiseRTCSystemAccountFence {
    pub fn matches(&self, account_id: Option<&str>, account_epoch: Option<&str>, generation: u64) -> bool {
        if Some(self.account_id.as_str()) != account_id || self.generation != generation {
            return false;
        }
        let (Some(own_epoch), Some(candidate_epoch)) = (self.account_epoch.as_deref(), account_epoch) else {
            return true;
        };
        own_epoch == candidate_epoch
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCSystemVoIPEndReason {
    Failed,
    Unanswered,
    RemoteEnded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseRTCSystemVoIPHandlingDecision {
    Ring(RiseRTCSystemAccountFence),
    ReportAndEnd(RiseRTCSystemVoIPEndReason),
}

pub enum RiseRTCSystemVoIPHandlingPolicy {}

impl RiseRTCSystemVoIPHandlingPolicy {
    pub fn decision(
        payload: &RiseRTCSystemParsedVoIPPayload,
        active_account_id: Option<&str>,
        active_account_epoch: Option<&str>,
        account_generation: u64,
        now_milliseconds: i64,
    ) -> RiseRTCSystemVoIPHandlingDecision {
        if payload.envelope.event != RiseRTCSystemVoIPEvent::Incoming {
            return RiseRTCSystemVoIPHandlingDecision::ReportAndEnd(RiseRTCSystemVoIPEndReason::RemoteEnded);
        }
        let active_account_id = match Self::normalized(active_account_id) {
            Some(account_id) if payload.issues.is_empty() && payload.envelope.recipient_user_id.as_deref() == Some(account_id) => {
                account_id
            }
            _ => return RiseRTCSystemVoIPHandlingDecision::ReportAndEnd(RiseRTCSystemVoIPEndReason::Failed),
        };
        let unexpired = payload.envelope.expires_at_milliseconds.is_some_and(|expires_at| expires_at > now_milliseconds);
        if !unexpired {
            return RiseRTCSystemVoIPHandlingDecision::ReportAndEnd(RiseRTCSystemVoIPEndReason::Unanswered);
        }
        RiseRTCSystemVoIPHandlingDecision::Ring(RiseRTCSystemAccountFence {
            account_id: active_account_id.to_owned(),
            account_epoch: Self::normalized(active_account_epoch).map(str::to_owned),
            generation: account_generation,
        })
    }

    fn normalized(value: Option<&str>) -> Option<&str> {
        let value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() {
            return None;
        }
        Some(value)
    }
}

/// push carries invites and urgent cancels, a known end event ends the system call
pub enum RiseRTCSystemVoIPServerPolicy {}

impl RiseRTCSystemVoIPServerPolicy {
    pub fn may_use_vo_ip_transport(event: &RiseRTCSystemVoIPEvent) -> bool {
        match event {
            RiseRTCSystemVoIPEvent::Incoming
            | RiseRTCSystemVoIPEvent::Cancelled
            | RiseRTCSystemVoIPEvent::Ended
            | RiseRTCSystemVoIPEvent::AnsweredElsewhere
            | RiseRTCSystemVoIPEvent::Declined
            | RiseRTCSystemVoIPEvent::TimedOut => true,
            RiseRTCSystemVoIPEvent::Answered | RiseRTCSystemVoIPEvent::Unknown(_) => false,
        }
    }

    pub fn legacy_client_must_report_to_call_kit(_event: &RiseRTCSystemVoIPEvent) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCSystemCallDirection {
    Incoming,
    Outgoing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCSystemCallPhase {
    IncomingRinging,
    OutgoingRequested,
    OutgoingStarting,
    OutgoingRinging,
    Answering,
    Active,
    Ending,
    Ended,
}

impl RiseRTCSystemCallPhase {
    /// phases that still talk to the server, ending counts since the end action goes out after hangup
    pub fn keeps_transport_live(self) -> bool {
        self != Self::Ended
    }
}

/// final result shown to the user after the system call ended
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCSystemCallTerminalOutcome {
    Ended,
    Failed,
    Cancelled,
    Declined,
    TimedOut,
    AnsweredElsewhere,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseRTCSystemBackendEndAction {
    Decline,
    Cancel,
    End,
    EndThenDecline,
    WaitForStartThenCancel,
}

pub enum RiseRTCSystemCallRacePolicy {}

impl RiseRTCSystemCallRacePolicy {
    pub fn backend_end_action(
        direction: RiseRTCSystemCallDirection,
        phase: RiseRTCSystemCallPhase,
        has_server_call_id: bool,
    ) -> RiseRTCSystemBackendEndAction {
        use RiseRTCSystemCallPhase as Phase;
        match direction {
            RiseRTCSystemCallDirection::Outgoing => match phase {
                Phase::OutgoingRequested | Phase::OutgoingStarting | Phase::OutgoingRinging => {
                    if has_server_call_id {
                        RiseRTCSystemBackendEndAction::Cancel
                    } else {
                        RiseRTCSystemBackendEndAction::WaitForStartThenCancel
                    }
                }
                Phase::Active | Phase::Ending | Phase::Ended => RiseRTCSystemBackendEndAction::End,
                Phase::IncomingRinging | Phase::Answering => RiseRTCSystemBackendEndAction::End,
            },
            RiseRTCSystemCallDirection::Incoming => match phase {
                Phase::IncomingRinging => RiseRTCSystemBackendEndAction::Decline,
                Phase::Answering => RiseRTCSystemBackendEndAction::EndThenDecline,
                Phase::Active => RiseRTCSystemBackendEndAction::End,
                Phase::Ending | Phase::Ended => RiseRTCSystemBackendEndAction::End,
                Phase::OutgoingRequested | Phase::OutgoingStarting | Phase::OutgoingRinging => RiseRTCSystemBackendEndAction::End,
            },
        }
    }

    pub fn accepts_async_completion(expected_operation_id: Uuid, current_operation_id: Option<Uuid>, phase: RiseRTCSystemCallPhase) -> bool {
        Some(expected_operation_id) == current_operation_id
            && phase != RiseRTCSystemCallPhase::Ending
            && phase != RiseRTCSystemCallPhase::Ended
    }

    pub fn should_finish_managed_call(phase: RiseRTCSystemCallPhase) -> bool {
        phase != RiseRTCSystemCallPhase::Ended
    }
}

struct RiseRTCSystemPayloadReader<'a> {
    values: &'a Map<String, Value>,
}

impl<'a> RiseRTCSystemPayloadReader<'a> {
    fn new(payload: &'a Map<String, Value>) -> Self {
        Self { values: payload }
    }

    fn string(&self, keys: &[&str]) -> Option<String> {
        for key in keys {
            let raw = match self.values.get(*key) {
                Some(Value::String(value)) => Some(value.clone()),
                Some(Value::Number(value)) => Some(value.to_string()),
                _ => None,
            };
            if let Some(normalized) = raw.as_deref().map(trimming_whitespaces_and_newlines)
                && !normalized.is_empty()
            {
                return Some(normalized.to_owned());
            }
        }
        None
    }

    fn int64(&self, keys: &[&str]) -> Option<i64> {
        for key in keys {
            match self.values.get(*key) {
                Some(Value::Number(value)) => {
                    if let Some(integer) = value.as_i64() {
                        return Some(integer);
                    }
                    if let Some(float) = value.as_f64() {
                        return Some(float as i64);
                    }
                }
                Some(Value::String(value)) => {
                    if let Ok(parsed) = value.parse::<i64>() {
                        return Some(parsed);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn dictionary(&self, keys: &[&str]) -> Option<&'a Map<String, Value>> {
        for key in keys {
            if let Some(Value::Object(value)) = self.values.get(*key) {
                return Some(value);
            }
        }
        None
    }
}

enum RiseRTCSystemFallbackCallUUID {}

impl RiseRTCSystemFallbackCallUUID {
    fn make(payload: &Map<String, Value>) -> Uuid {
        let canonical = Self::canonical_object(payload);
        let digest = Sha256::hash(canonical.as_bytes());
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x50;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid::from_bytes(bytes)
    }

    fn canonical_object(object: &Map<String, Value>) -> String {
        let sorted: BTreeMap<&str, String> = object.iter().map(|(key, value)| (key.as_str(), Self::canonical_value(value))).collect();
        sorted.into_iter().map(|(key, value)| format!("{key}:{value}")).collect::<Vec<_>>().join("|")
    }

    fn canonical_value(value: &Value) -> String {
        match value {
            Value::Object(object) => Self::canonical_object(object),
            Value::Array(array) => array.iter().map(Self::canonical_value).collect::<Vec<_>>().join(","),
            Value::String(string) => string.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            Value::Null => String::new(),
        }
    }
}
