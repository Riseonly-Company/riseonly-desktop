use std::collections::HashMap;

use gpui::AsyncApp;
use rise_canon::rise_wire_canon;
use rise_core::foundation::foundation_url::Url;
use rise_engine::rise_upload::rise_chunked_upload::RiseChunkedUploadError;
use rise_engine::rise_upload::rise_url_session::URLRequest;
use rise_engine::{RiseJSONValue, RiseWireError, RiseWireRemoteError};
use rise_i18n::tr;
use rise_limits::service_limit::API_GATEWAY_HTTP_JSON_BODY_MAX_BYTES;
use serde_json::Value;

use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::lib::canon_failure_relay::RiseServerRefusalRelay;
use crate::core::rise_engine_bridge::rise_canon_fields::RiseCanonFields;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_ws_response_debt_coordinator::SaiWsAckTimeoutError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;

/// canon failure for the engine refusal, a type from another crate
pub trait CanonFailure {
    fn canon_failure(&self) -> RiseFailure;
}

impl SaiWsResponseError {
    /// legacy transport refusal in canon form
    /// a dropped connection has no code so is_transport_drop is set and no code is invented
    pub fn canon_failure(&self) -> RiseFailure {
        RiseFailure::new(
            if self.is_transport_drop { None } else { self.code.clone() },
            self.message.clone(),
            self.details.clone(),
        )
    }
}

impl CanonFailure for RiseWireRemoteError {
    fn canon_failure(&self) -> RiseFailure {
        RiseFailure::new(
            Some(self.code.clone()),
            self.message.clone(),
            flat_details(self.details.as_ref()),
        )
    }
}

fn flat_details(value: Option<&RiseJSONValue>) -> HashMap<String, String> {
    let Some(RiseJSONValue::Object(fields)) = value else {
        return HashMap::new();
    };
    let mut result = HashMap::with_capacity(fields.len());
    for (key, item) in fields {
        let text = match item {
            RiseJSONValue::String(text) => text.clone(),
            RiseJSONValue::Integer(number) => number.to_string(),
            RiseJSONValue::UnsignedInteger(number) => number.to_string(),
            RiseJSONValue::Number(number) => integral_text(*number).unwrap_or_else(|| number.to_string()),
            RiseJSONValue::Bool(flag) => flag.to_string(),
            RiseJSONValue::Null | RiseJSONValue::Array(_) | RiseJSONValue::Object(_) => continue,
        };
        result.insert(key.clone(), text);
    }
    result
}

fn integral_text(number: f64) -> Option<String> {
    let in_range = number >= i64::MIN as f64 && number < i64::MAX as f64;
    (number.is_finite() && number.fract() == 0.0 && in_range).then(|| (number as i64).to_string())
}

impl CanonErrors {
    /// screen text for a failed load
    /// never the error description, its a type name and backend prose isnt in the users language
    pub fn load_failure_text(error: &anyhow::Error) -> String {
        if Self::is_maintenance_refusal(error) {
            return tr("maintenance_action_blocked");
        }
        Self::failure(error)
            .and_then(|failure| Self::text_for(&failure))
            .unwrap_or_else(|| tr("error_fetching_data"))
    }

    /// structured refusal from either transport, none for a local or connection failure
    pub fn failure(error: &anyhow::Error) -> Option<RiseFailure> {
        if let Some(failure) = error.downcast_ref::<RiseFailure>() {
            return Some(failure.clone());
        }
        if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            return Some(remote.canon_failure());
        }
        if let Some(RiseChunkedUploadError::HttpStatus(_, body)) = error.downcast_ref::<RiseChunkedUploadError>() {
            return RiseMediaRefusal::parse(body.as_bytes());
        }
        error
            .downcast_ref::<SaiWsResponseError>()
            .map(SaiWsResponseError::canon_failure)
    }

    /// only canon phrases and local fallback text may reach a request slice
    pub fn refusal_text(error: &anyhow::Error) -> String {
        if Self::is_maintenance_refusal(error) {
            return tr("maintenance_action_blocked");
        }
        if Self::is_connection_failure(error) || Self::is_legacy_socket_drop(error) {
            return tr("chat_delete_failed_text");
        }
        Self::failure(error)
            .and_then(|failure| Self::text_for(&failure))
            .unwrap_or_else(|| tr("retry_later_error"))
    }

    /// the app itself refused the request because the product is in maintenance
    /// not a server refusal so failure stays none
    pub fn is_maintenance_refusal(error: &anyhow::Error) -> bool {
        matches!(error.downcast_ref::<SaiWsSendError>(), Some(SaiWsSendError::Maintenance))
    }

    /// no answer came, the socket was down or dropped or went silent before the reply
    pub fn is_connection_loss(error: &anyhow::Error) -> bool {
        Self::is_connection_failure(error)
            || Self::is_legacy_socket_drop(error)
            || error.downcast_ref::<SaiWsAckTimeoutError>().is_some()
    }

    /// the legacy socket was down or dropped before the answer
    /// its own description is english log text so dont show it
    fn is_legacy_socket_drop(error: &anyhow::Error) -> bool {
        match error.downcast_ref::<SaiWsSendError>() {
            Some(SaiWsSendError::NotConnected | SaiWsSendError::SendFailed(_)) => true,
            _ => error
                .downcast_ref::<SaiWsResponseError>()
                .is_some_and(|response| response.is_transport_drop),
        }
    }

    /// engine had no socket or lost it before the answer
    /// the request may never have reached the server so the reader is pointed at the connection
    pub fn is_connection_failure(error: &anyhow::Error) -> bool {
        matches!(
            error.downcast_ref::<RiseWireError>(),
            Some(
                RiseWireError::NoSocketSession
                    | RiseWireError::NotConnected
                    | RiseWireError::Suspended
                    | RiseWireError::Timeout(_)
                    | RiseWireError::Disconnected(_)
            )
        )
    }
}

impl RiseFailure {
    pub fn canonical_field_pointer(&self) -> Option<String> {
        CanonErrors::field_pointer_for(self)
            .map(|pointer| RiseCanonFields::canonical_pointer(&pointer))
    }

    pub fn belongs_under_field(&self) -> bool {
        CanonErrors::category(self.code.as_deref()) == rise_errors::ErrorCategory::Validation
    }

    pub fn upgrade_tier(&self) -> Option<String> {
        CanonErrors::upgrade_tier(&self.details)
    }

    /// true if a subscription raises this ceiling, the only reason to send the user to the paywall
    /// quota category alone isnt one since most premium ceilings equal the free ones
    pub fn subscription_lifts_limit(&self) -> bool {
        self.upgrade_tier().is_some()
    }
}

/// transport adapter for authenticated media refusals, parsing stays off the main thread
pub enum RiseMediaRefusal {}

impl RiseMediaRefusal {
    pub const MAXIMUM_BODY_BYTES: u64 = API_GATEWAY_HTTP_JSON_BODY_MAX_BYTES;

    pub fn is_protected_refusal(status_code: Option<i64>, response_url: Option<&Url>, request: &URLRequest) -> bool {
        let Some(status_code) = status_code else {
            return false;
        };
        let Some(response_url) = response_url else {
            return false;
        };
        let source = &request.url;
        status_code == rise_wire_canon::HTTP_STATUS_TOO_MANY_REQUESTS
            && source.path().starts_with(rise_wire_canon::SECURE_MEDIA_PATH_PREFIX)
            && request.value(rise_wire_canon::AUTHORIZATION_HEADER).is_some()
            && response_url.host() == source.host()
            && response_url.scheme() == source.scheme()
            && response_url.port() == source.port()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn publish(
        body: Vec<u8>,
        status_code: Option<i64>,
        response_url: Option<Url>,
        request: URLRequest,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) {
        if !Self::is_protected_refusal(status_code, response_url.as_ref(), &request)
            || body.len() as u64 > Self::MAXIMUM_BODY_BYTES
        {
            return;
        }
        let Some(failure) = Self::parse(&body) else {
            return;
        };
        let _ = cx.update(|cx| {
            if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                return;
            }
            RiseServerRefusalRelay::publish(failure.clone(), cx);
        });
    }

    fn parse(body: &[u8]) -> Option<RiseFailure> {
        let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(body) else {
            return None;
        };
        let code = object.get(rise_wire_canon::key::CODE)?.as_str()?.to_owned();
        let mut details = HashMap::new();
        if let Some(Value::Object(raw)) = object.get(rise_wire_canon::key::DETAILS) {
            for (key, value) in raw {
                if let Some(text) = ns_number_or_string(value) {
                    details.insert(key.clone(), text);
                }
            }
        }
        let message = object
            .get(rise_wire_canon::key::MESSAGE)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Some(RiseFailure::new(Some(code), message, details))
    }
}

fn ns_number_or_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(if *flag { "1" } else { "0" }.to_owned()),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                return Some(integer.to_string());
            }
            if let Some(unsigned) = number.as_u64() {
                return Some(unsigned.to_string());
            }
            let float = number.as_f64()?;
            Some(integral_text(float).unwrap_or_else(|| float.to_string()))
        }
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}
