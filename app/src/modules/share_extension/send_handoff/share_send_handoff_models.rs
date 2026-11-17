use std::fmt;

use rise_canon::rise_key::payload as payload_key;
use rise_canon::rise_view::message as view;
use rise_engine::RiseJSONValue;
use rise_i18n::{FormatArg, format_printf, tr};

use crate::core::lib::canon_errors::RiseFailure;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    FanoutRefusalDetailsWireDTO, MessageWireDTO,
};

#[derive(Clone, Debug, PartialEq)]
pub enum ShareSendHandoffPhase {
    Idle,
    Preparing { session_id: String },
    Uploading {
        session_id: String,
        filename: String,
        item: i64,
        total: i64,
        fraction: f64,
        sent_bytes: i64,
        file_bytes: i64,
    },
    Sending { session_id: String },
    Failed { session_id: String, message: String, commit_started: bool },
    Partial { session_id: String, message: String, delivered: i64, failed: i64 },
    Succeeded { session_id: String, destination_count: i64 },
}

impl ShareSendHandoffPhase {
    pub fn active_session_id(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Preparing { session_id }
            | Self::Uploading { session_id, .. }
            | Self::Sending { session_id }
            | Self::Failed { session_id, .. }
            | Self::Partial { session_id, .. }
            | Self::Succeeded { session_id, .. } => Some(session_id.as_str()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareSendAcknowledgement {
    Complete,
    Partial { message: String, delivered: i64, failed: i64 },
}

/// answer of one destination to message.share_payload, read from the canon view
#[derive(Clone, Debug)]
pub struct RiseStoredSharePayloadResult {
    pub chat_id: String,
    pub topic_id: Option<String>,
    pub success: bool,
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub error_details: Option<FanoutRefusalDetailsWireDTO>,
    pub retryable: bool,
    pub message: Option<MessageWireDTO>,
}

impl RiseStoredSharePayloadResult {
    pub fn refusal(&self) -> Option<RiseFailure> {
        RiseFailure::from_fanout_code(self.error_code.as_deref(), self.error.as_deref(), self.error_details.as_ref())
    }

    pub fn from_canon(result: &view::ShareResult) -> Self {
        let retry_after_text = result.error_details.as_ref().and_then(|details| {
            let details = RiseJSONValue::from(details.clone());
            match details.object_value()?.get(payload_key::RETRY_AFTER_S)? {
                RiseJSONValue::String(text) => Some(text.clone()),
                RiseJSONValue::Integer(number) => Some(number.to_string()),
                _ => None,
            }
        });
        Self {
            chat_id: result.chat_id.clone(),
            topic_id: result.topic_id.clone(),
            success: result.success,
            error: if result.error.is_empty() { None } else { Some(result.error.clone()) },
            error_code: result.error_code.clone(),
            error_details: result
                .error_details
                .as_ref()
                .map(|_| FanoutRefusalDetailsWireDTO { retry_after_s: retry_after_text }),
            retryable: result.retryable,
            message: result.message.as_ref().and_then(|message| MessageWireDTO::from_canon(message).ok()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RiseStoredSharePayloadReply {
    pub idempotency_key: String,
    pub all_succeeded: bool,
    pub results: Vec<RiseStoredSharePayloadResult>,
}

impl RiseStoredSharePayloadReply {
    pub fn from_canon(shared: &view::Shared) -> Self {
        Self {
            idempotency_key: shared.idempotency_key.clone(),
            all_succeeded: shared.all_succeeded,
            results: shared.results.iter().map(RiseStoredSharePayloadResult::from_canon).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareSendHandoffError {
    InvalidResponse,
    AccountChanged,
    PartialDelivery { delivered: i64, failed: Vec<String> },
}

impl ShareSendHandoffError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            Self::InvalidResponse => Some(tr("share_send_handoff_invalid")),
            Self::AccountChanged => Some(tr("share_send_handoff_account_changed")),
            Self::PartialDelivery { delivered, failed } => {
                let summary = format_printf(
                    &tr("share_send_handoff_partial"),
                    &[FormatArg::from(*delivered), FormatArg::from(failed.len())],
                );
                if failed.is_empty() {
                    return Some(summary);
                }
                Some(summary + "\n" + &failed.join("\n"))
            }
        }
    }
}

impl fmt::Display for ShareSendHandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for ShareSendHandoffError {}
