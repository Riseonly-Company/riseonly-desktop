use rise_engine::RiseWireRemoteError;
use rise_i18n::tr;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_failure_bridge::CanonFailure;
use crate::modules::moderation::engine::rise_moderation_rpc::RiseModerationError;

/// turns a moderation refusal into something a person can read
/// wire error is opaque, it prints only the code
/// so the repeat report branch never fired and already reported looked like a transport failure
pub enum RiseModerationFailure {}

impl RiseModerationFailure {
    pub fn message(error: &anyhow::Error) -> String {
        if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            let failure = remote.canon_failure();
            if let Some(localized) = CanonErrors::text_for(&failure) {
                return localized;
            }
            return if failure.message.is_empty() {
                tr("moderation_error_connection")
            } else {
                failure.message
            };
        }
        if let Some(localized) = error
            .downcast_ref::<RiseModerationError>()
            .and_then(RiseModerationError::error_description)
            .filter(|localized| !localized.is_empty())
        {
            return localized;
        }
        tr("moderation_error_connection")
    }

    /// the servers own sentence for branches that classify a refusal
    /// not translated, its matched and never shown
    pub fn prose(error: &anyhow::Error) -> String {
        error
            .downcast_ref::<RiseWireRemoteError>()
            .map(|remote| remote.message.clone())
            .unwrap_or_default()
    }

    /// code and prose together, for the log
    pub fn diagnostic(error: &anyhow::Error) -> String {
        match error.downcast_ref::<RiseWireRemoteError>() {
            Some(remote) => format!("{}: {}", remote.code, remote.message),
            None => format!("{error:#}"),
        }
    }

    /// true if the object already has a report from this person
    /// report-service says you already reported this or already have a pending appeal
    /// neither has a canon code, gateway stamps GRPC_ERROR so this matches the prose
    pub fn is_duplicate(error: &anyhow::Error) -> bool {
        let lower = Self::prose(error).to_lowercase();
        lower.contains("already reported") || lower.contains("already have a pending")
    }

    pub fn is_access_denied(error: &anyhow::Error) -> bool {
        Self::prose(error).to_lowercase().contains("access denied")
    }

    /// true if the sanction isnt one this person may appeal
    pub fn is_not_appealable(error: &anyhow::Error) -> bool {
        let lower = Self::prose(error).to_lowercase();
        lower.contains("can only appeal")
            || lower.contains("not eligible")
            || (lower.contains("appeal") && lower.contains("already been resolved"))
    }
}
