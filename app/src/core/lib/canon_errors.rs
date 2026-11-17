use std::collections::HashMap;

use rise_errors::{ErrorCategory, ErrorEntry, ErrorParam, ErrorParamKind};
use rise_i18n::{FormatArg, format_printf, tr};

use crate::core::lib::canon_format::RiseFormat;

/// server refusal in a transport independent form
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("RiseFailure(code: {code:?})")]
pub struct RiseFailure {
    pub code: Option<String>,

    pub message: String,

    pub details: HashMap<String, String>,
}

impl RiseFailure {
    pub fn new(code: Option<String>, message: String, details: HashMap<String, String>) -> Self {
        Self { code, message, details }
    }
}

pub enum CanonErrors {}

impl CanonErrors {
    /// canon entry by wire code, resolves legacy spellings like RATE_LIMITED to GW_RATE_LIMITED
    /// none means only that the code isnt in the dictionary
    pub fn entry(code: Option<&str>) -> Option<&'static ErrorEntry> {
        let code = code?;
        if rise_errors::is_absent_code(code) {
            return None;
        }
        rise_errors::entry(code)
    }

    /// category that drives presentation and retry policy
    /// for a code outside the dictionary it is the canon fallback
    pub fn category(code: Option<&str>) -> ErrorCategory {
        Self::entry(code).map_or(rise_errors::UNKNOWN_CATEGORY, |entry| entry.category)
    }

    /// phrase in the users language
    /// none means nothing to show so the caller decides
    /// never print the code or the key
    pub fn text(code: Option<&str>, details: &HashMap<String, String>) -> Option<String> {
        let entry = Self::entry(code)?;
        let key = entry.i18n_key?;
        if let Some(clock) = Self::clock_twin(key, details) {
            return Some(clock);
        }
        if let Some(attempts) = Self::attempts_twin(key, details) {
            return Self::format(&tr(&attempts), &attempts, entry, details);
        }
        let minutes = Self::minutes_twin(key, details).unwrap_or_else(|| key.to_owned());
        let shown = Self::hours_twin(&minutes, details).unwrap_or(minutes);
        Self::format(&tr(&shown), &shown, entry, details)
    }

    fn attempts_twin(key: &str, details: &HashMap<String, String>) -> Option<String> {
        details
            .get(ErrorParam::AttemptsLeft.wire())
            .and_then(|raw| raw.parse::<i64>().ok())?;
        let twin = format!("{key}{}", Self::ATTEMPTS_SUFFIX);
        if tr(&twin) == twin { None } else { Some(twin) }
    }

    fn clock_twin(key: &str, details: &HashMap<String, String>) -> Option<String> {
        let seconds = Self::retry_after_seconds(details).filter(|seconds| *seconds > 0)?;
        let twin = format!("{key}{}", Self::CLOCK_SUFFIX);
        let pattern = tr(&twin);
        if pattern == twin || pattern.split(Self::OBJECT_SPECIFIER).count() != 2 {
            return None;
        }
        let clock = RiseFormat::duration(seconds);
        Some(format_printf(&pattern, &[FormatArg::from(clock)]))
    }

    fn minutes_twin(key: &str, details: &HashMap<String, String>) -> Option<String> {
        if key.ends_with(Self::MINUTES_SUFFIX)
            || key.ends_with(Self::SECONDS_SUFFIX)
            || key.ends_with(Self::HOURS_SUFFIX)
        {
            return None;
        }
        Self::retry_after_seconds(details).filter(|seconds| *seconds >= Self::SECONDS_PER_MINUTE)?;
        let twin = format!("{key}{}", Self::MINUTES_SUFFIX);
        if tr(&twin) == twin { None } else { Some(twin) }
    }

    fn hours_twin(key: &str, details: &HashMap<String, String>) -> Option<String> {
        let stem = key.strip_suffix(Self::MINUTES_SUFFIX)?;
        Self::retry_after_seconds(details)
            .filter(|seconds| *seconds >= Self::HOURS_TWIN_FROM_HOURS * Self::SECONDS_PER_HOUR)?;
        let twin = format!("{stem}{}", Self::HOURS_SUFFIX);
        if tr(&twin) == twin { None } else { Some(twin) }
    }

    pub fn text_for(failure: &RiseFailure) -> Option<String> {
        Self::text(failure.code.as_deref(), &failure.details)
    }

    pub fn is_retryable(code: Option<&str>) -> bool {
        Self::entry(code).map_or(rise_errors::UNKNOWN_RETRYABLE, |entry| entry.retryable)
    }

    /// seconds after which a retry makes sense, none when the server didnt say
    pub fn retry_after_seconds(details: &HashMap<String, String>) -> Option<i64> {
        details
            .get(ErrorParam::RetryAfterS.wire())
            .and_then(|raw| raw.parse::<i64>().ok())
    }

    /// input field to focus, for some codes pinned to the code and for others it comes as a parameter
    /// name matches the rules field in canon validation
    pub fn field(code: Option<&str>, details: &HashMap<String, String>) -> Option<String> {
        let entry = Self::entry(code)?;
        entry
            .field
            .map(str::to_owned)
            .or_else(|| details.get(ErrorParam::Field.wire()).cloned())
    }

    pub fn field_pointer(details: &HashMap<String, String>) -> Option<String> {
        details.get(ErrorParam::FieldPath.wire()).cloned()
    }

    pub fn field_pointer_for(failure: &RiseFailure) -> Option<String> {
        Self::field_pointer(&failure.details)
    }

    pub fn upgrade_tier(details: &HashMap<String, String>) -> Option<String> {
        details
            .get(ErrorParam::UpgradeTier.wire())
            .filter(|tier| !tier.is_empty())
            .cloned()
    }

    /// canon phrases carry at most one %d and it is always an integer
    /// with more specifiers blind substitution is worse than showing nothing
    fn format(
        pattern: &str,
        key: &str,
        entry: &ErrorEntry,
        details: &HashMap<String, String>,
    ) -> Option<String> {
        let specifiers = Self::integer_specifier_count(pattern);
        if specifiers == 0 {
            return Some(pattern.to_owned());
        }
        if specifiers != 1 {
            return None;
        }
        let value = Self::integer_argument(key, entry, details)?;
        Some(format_printf(pattern, &[FormatArg::Int(value)]))
    }

    fn integer_argument(key: &str, entry: &ErrorEntry, details: &HashMap<String, String>) -> Option<i64> {
        if key.ends_with(Self::MINUTES_SUFFIX) || key.ends_with(Self::SECONDS_SUFFIX) || key.ends_with(Self::HOURS_SUFFIX) {
            let seconds = Self::retry_after_seconds(details)?;
            if key.ends_with(Self::HOURS_SUFFIX) {
                return Some(Self::rounded_up(seconds, Self::SECONDS_PER_HOUR).max(1));
            }
            if !key.ends_with(Self::MINUTES_SUFFIX) {
                return Some(seconds);
            }
            return Some(Self::rounded_up(seconds, Self::SECONDS_PER_MINUTE).max(1));
        }

        let mut present = entry
            .params
            .iter()
            .filter(|param| param.kind() == ErrorParamKind::Int && details.contains_key(param.wire()));
        let only = present.next()?;
        if present.next().is_some() {
            return None;
        }
        details.get(only.wire())?.parse::<i64>().ok()
    }

    fn rounded_up(seconds: i64, unit: i64) -> i64 {
        (seconds as f64 / unit as f64).ceil() as i64
    }

    fn integer_specifier_count(pattern: &str) -> usize {
        let characters: Vec<char> = pattern.chars().collect();
        let mut count = 0;
        let mut index = 0;
        while index + 1 < characters.len() {
            if characters[index] != '%' {
                index += 1;
                continue;
            }
            let next = characters[index + 1];
            if next == '%' {
                index += 2;
                continue;
            }
            if next == 'd' || next == 'i' || next == 'u' {
                count += 1;
            }
            index += 2;
        }
        count
    }

    const ATTEMPTS_SUFFIX: &'static str = "_attempts";
    const CLOCK_SUFFIX: &'static str = "_clock";
    const MINUTES_SUFFIX: &'static str = "_minutes";
    const SECONDS_SUFFIX: &'static str = "_seconds";
    const HOURS_SUFFIX: &'static str = "_hours";
    const OBJECT_SPECIFIER: &'static str = "%@";
    const SECONDS_PER_MINUTE: i64 = 60;
    const SECONDS_PER_HOUR: i64 = 3_600;
    const HOURS_TWIN_FROM_HOURS: i64 = 2;
}
