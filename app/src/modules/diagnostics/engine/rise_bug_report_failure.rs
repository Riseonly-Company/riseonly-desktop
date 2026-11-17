use rise_engine::rise_upload::rise_chunked_upload::RiseChunkedUploadError;
use rise_engine::rise_upload::rise_url_session::URLError;
use rise_engine::{CancellationError, RiseWireError, RiseWireRemoteError};
use rise_i18n::tr;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_failure_bridge::CanonFailure;
use crate::modules::diagnostics::engine::core::rise_bug_report_upload::BugReportUploadError;
use crate::modules::diagnostics::engine::rise_bug_report_submitter::BugReportSubmitError;

/// turns a report refusal into readable text
/// same funnel as moderation failures: canon phrase then backend sentence then the feature
/// fallback for transport and decode errors
pub enum RiseBugReportFailure {}

impl RiseBugReportFailure {
    /// fallback_key is the translated feature sentence used when nothing better exists
    pub fn message(error: &anyhow::Error, fallback_key: &str) -> String {
        if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            let failure = remote.canon_failure();
            if let Some(localized) = CanonErrors::text_for(&failure) {
                return localized;
            }
            return if failure.message.is_empty() { tr(fallback_key) } else { failure.message };
        }
        if error.is::<CancellationError>()
            || error.is::<RiseWireError>()
            || error.is::<RiseChunkedUploadError>()
            || error.is::<URLError>()
            || error.is::<serde_json::Error>()
        {
            return tr(fallback_key);
        }
        let localized = error
            .downcast_ref::<BugReportUploadError>()
            .and_then(BugReportUploadError::error_description)
            .or_else(|| error.downcast_ref::<BugReportSubmitError>().and_then(BugReportSubmitError::error_description));
        match localized {
            Some(localized) if !localized.is_empty() => localized,
            _ => tr(fallback_key),
        }
    }

    /// code and prose together for the breadcrumb trail, never shown
    pub fn diagnostic(error: &anyhow::Error) -> String {
        match error.downcast_ref::<RiseWireRemoteError>() {
            Some(remote) => format!("{}: {}", remote.code, remote.message),
            None => format!("{error:#}"),
        }
    }
}
