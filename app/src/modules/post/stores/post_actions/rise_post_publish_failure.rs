use std::collections::{BTreeMap, HashMap};

use rise_engine::rise_upload::rise_chunked_upload::RiseChunkedUploadError;
use rise_engine::rise_upload::rise_url_session::URLError;
use rise_engine::{CancellationError, RiseJSONValue, RiseWireError, RiseWireRemoteError};
use rise_i18n::tr;

use crate::core::lib::canon_errors::CanonErrors;
use crate::modules::post::engine::rise_post_repository::RisePostRepositoryError;

pub enum RisePostPublishFailure {}

impl RisePostPublishFailure {
    const DRAFT_RULE_CODES: [&'static str; 3] =
        ["thread_cannot_contain_media", "publication_requires_media", "thread_requires_description"];

    const CONNECTION_FALLBACK: &'static str = "create_post_error_connection";

    fn is_draft_rule_code(message: &str) -> bool {
        Self::DRAFT_RULE_CODES.contains(&message)
    }

    pub fn display_text(message: &str) -> String {
        if Self::is_draft_rule_code(message) || message == Self::CONNECTION_FALLBACK {
            return tr(message);
        }
        message.to_owned()
    }

    pub fn is_retryable(message: &str) -> bool {
        !Self::is_draft_rule_code(message)
            && message != "achievement_cannot_contain_media"
            && message != tr("create_post_short_too_long")
    }

    pub fn message(error: &anyhow::Error) -> String {
        if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            return Self::remote_message(remote);
        }
        let described = error.to_string();
        if Self::is_draft_rule_code(&described) {
            return described;
        }
        if error.is::<CancellationError>()
            || error.is::<RiseWireError>()
            || error.is::<RiseChunkedUploadError>()
            || error.is::<RisePostRepositoryError>()
            || error.is::<URLError>()
        {
            return Self::CONNECTION_FALLBACK.to_owned();
        }
        if described.is_empty() {
            Self::CONNECTION_FALLBACK.to_owned()
        } else {
            described
        }
    }

    fn remote_message(remote: &RiseWireRemoteError) -> String {
        if Self::is_draft_rule_code(&remote.code) {
            return remote.code.clone();
        }
        if let Some(localized) = CanonErrors::text(Some(&remote.code), &Self::string_details(remote.details.as_ref())) {
            return localized;
        }
        if remote.message.is_empty() {
            Self::CONNECTION_FALLBACK.to_owned()
        } else {
            remote.message.clone()
        }
    }

    fn string_details(value: Option<&RiseJSONValue>) -> HashMap<String, String> {
        let Some(object) = value.and_then(RiseJSONValue::object_value) else {
            return HashMap::new();
        };
        Self::flatten(object)
    }

    fn flatten(object: &BTreeMap<String, RiseJSONValue>) -> HashMap<String, String> {
        let mut result = HashMap::with_capacity(object.len());
        for (key, value) in object {
            let text = match value {
                RiseJSONValue::String(text) => text.clone(),
                RiseJSONValue::Integer(number) => number.to_string(),
                RiseJSONValue::UnsignedInteger(number) => number.to_string(),
                RiseJSONValue::Number(number) => format!("{number:?}"),
                RiseJSONValue::Bool(flag) => if *flag { "true" } else { "false" }.to_owned(),
                RiseJSONValue::Null | RiseJSONValue::Array(_) | RiseJSONValue::Object(_) => continue,
            };
            result.insert(key.clone(), text);
        }
        result
    }
}
