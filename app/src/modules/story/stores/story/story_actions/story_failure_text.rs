use std::collections::HashMap;

use gpui::App;
use rise_canon::rise_key::bot;
use rise_canon::rise_upload::ErrorCode as RiseUploadErrorCode;
use rise_canon::rise_wire_canon::key as wire_key;
use rise_engine::rise_upload::rise_chunked_upload::RiseChunkedUploadError;
use rise_engine::rise_upload::rise_url_session::URLError;
use rise_engine::{RiseWireError, RiseWireRemoteError};
use rise_errors::ErrorCode;
use rise_i18n::tr;
use serde_json::Value;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_failure_bridge::CanonFailure;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_error::SaiFetchHttpError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::SaiWsChunkedFileUploadError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::modules::share_extension::shared::rise_outgoing_media::RiseOutgoingVideoTranscoderFailure;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;

/// alert text for a failed story request
/// transport errors are raw for the log, a refusal gets the canon phrase
/// else a plain sentence
pub enum StoryFailureText {}

impl StoryFailureText {
    pub fn message(error: &anyhow::Error) -> String {
        if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            return CanonErrors::text_for(&remote.canon_failure()).unwrap_or_else(|| tr("retry_later_error"));
        }
        if let Some(response) = error.downcast_ref::<SaiWsResponseError>() {
            return if response.is_transport_drop { Self::connection_text() } else { response.error_description() };
        }
        if let Some(SaiFetchHttpError::BadStatus { code: status, body }) = error.downcast_ref::<SaiFetchHttpError>() {
            if let Some(text) = Self::gateway_code(body).and_then(|code| CanonErrors::text(Some(&code), &HashMap::new()))
            {
                return text;
            }
            return if *status >= 500 { Self::connection_text() } else { tr("retry_later_error") };
        }
        if let Some(send) = error.downcast_ref::<SaiWsSendError>() {
            return match send {
                SaiWsSendError::Maintenance | SaiWsSendError::GuideSandbox => send.error_description(),
                SaiWsSendError::NotConnected | SaiWsSendError::EncodingFailed(_) | SaiWsSendError::SendFailed(_) => {
                    Self::connection_text()
                }
            };
        }
        if error.is::<SaiWsChunkedFileUploadError>() {
            return error.to_string();
        }
        if error.is::<RiseOutgoingVideoTranscoderFailure>() {
            return tr("media_picker_video_export_failed");
        }
        if error.is::<URLError>()
            || error.is::<RiseWireError>()
            || error.is::<RiseChunkedUploadError>()
            || error.is::<SaiFetchHttpError>()
        {
            return Self::connection_text();
        }
        tr("retry_later_error")
    }

    /// a dropped link not a refusal, the publish parks and repeats when the network is back
    /// anything failing while the socket is down counts too, whichever layer saw it
    pub fn is_transport_failure(error: &anyhow::Error, cx: &mut App) -> bool {
        if error.is::<RiseWireRemoteError>() {
            return false;
        }
        if let Some(response) = error.downcast_ref::<SaiWsResponseError>() {
            return response.is_transport_drop;
        }
        if let Some(SaiFetchHttpError::BadStatus { code: status, .. }) = error.downcast_ref::<SaiFetchHttpError>() {
            return *status >= 500;
        }
        if let Some(send) = error.downcast_ref::<SaiWsSendError>() {
            return match send {
                SaiWsSendError::NotConnected | SaiWsSendError::SendFailed(_) => true,
                SaiWsSendError::Maintenance | SaiWsSendError::GuideSandbox | SaiWsSendError::EncodingFailed(_) => false,
            };
        }
        if let Some(upload) = error.downcast_ref::<SaiWsChunkedFileUploadError>() {
            return match upload.canon_code() {
                RiseUploadErrorCode::Network
                | RiseUploadErrorCode::Server
                | RiseUploadErrorCode::Unavailable
                | RiseUploadErrorCode::NotAcknowledged => true,
                _ => RiseTransportStatus::is_waiting_for_network(cx),
            };
        }
        if let Some(wire) = error.downcast_ref::<RiseWireError>() {
            return matches!(
                wire,
                RiseWireError::NoSocketSession
                    | RiseWireError::NotConnected
                    | RiseWireError::Suspended
                    | RiseWireError::Timeout(_)
                    | RiseWireError::Disconnected(_)
            );
        }
        if error.is::<URLError>() {
            return true;
        }
        RiseTransportStatus::is_waiting_for_network(cx)
    }

    pub fn is_story_gone(error: &anyhow::Error) -> bool {
        let code = if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            remote.canon_failure().code
        } else if let Some(response) = error.downcast_ref::<SaiWsResponseError>() {
            response.canon_failure().code
        } else if let Some(SaiFetchHttpError::BadStatus { body, .. }) = error.downcast_ref::<SaiFetchHttpError>() {
            Self::gateway_code(body)
        } else {
            None
        };
        code.as_deref() == Some(ErrorCode::StoryNotFound.wire())
    }

    pub fn connection_text() -> String {
        tr("moderation_error_connection")
    }

    fn gateway_code(body: &[u8]) -> Option<String> {
        let object = serde_json::from_slice::<Value>(body).ok()?;
        let object = object.as_object()?;
        let code = object
            .get(wire_key::CODE)
            .and_then(Value::as_str)
            .or_else(|| object.get(bot::ERROR_CODE).and_then(Value::as_str))?;
        if code.is_empty() {
            return None;
        }
        Some(code.to_owned())
    }
}
