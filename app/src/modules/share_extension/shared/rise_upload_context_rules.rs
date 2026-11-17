use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_limits::{MediaClass as RiseUploadMediaClass, UPLOAD_CONTEXT_RULES, UploadContextRule as RiseUploadContextRule};

pub trait RiseLimitsUploadContextRulesExt {
    fn upload_context_rule(context: &str) -> &'static RiseUploadContextRule;

    fn upload_maximum_bytes(context: &str, content_type: &str, premium: bool) -> i64;
}

impl RiseLimitsUploadContextRulesExt for RiseLimits {
    fn upload_context_rule(context: &str) -> &'static RiseUploadContextRule {
        let normalized = trimming_whitespaces_and_newlines(context).to_lowercase();
        if let Some(named) = UPLOAD_CONTEXT_RULES
            .iter()
            .find(|rule| rule.contexts.contains(&normalized.as_str()))
        {
            return named;
        }
        match UPLOAD_CONTEXT_RULES.iter().find(|rule| rule.fallback) {
            Some(fallback) => fallback,
            None => panic!("Canon upload context rules have no fallback branch"),
        }
    }

    fn upload_maximum_bytes(context: &str, content_type: &str, premium: bool) -> i64 {
        let bytes = Self::upload_context_rule(context)
            .bytes_for(RiseUploadMediaClass::from_content_type(content_type), premium);
        i64::try_from(bytes).unwrap_or(i64::MAX)
    }
}

pub trait RiseUploadMediaClassContentTypeExt {
    fn from_content_type(content_type: &str) -> Self;
}

impl RiseUploadMediaClassContentTypeExt for RiseUploadMediaClass {
    fn from_content_type(content_type: &str) -> Self {
        let normalized = content_type
            .splitn(2, ';')
            .next()
            .map(|head| trimming_whitespaces(head).to_lowercase())
            .unwrap_or_default();
        match normalized.as_str() {
            "image/gif" | "image/apng" => RiseUploadMediaClass::AnimatedImage,
            _ if normalized.starts_with("image/") => RiseUploadMediaClass::Image,
            _ if normalized.starts_with("video/") => RiseUploadMediaClass::Video,
            _ if normalized.starts_with("audio/") => RiseUploadMediaClass::Audio,
            _ => RiseUploadMediaClass::Document,
        }
    }
}
