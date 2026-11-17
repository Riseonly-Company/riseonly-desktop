use rise_canon::{RiseLimits, RiseValidation};
use rise_i18n::{FormatArg, tr, tr_args};
use rise_limits::MediaPickerPreset as RiseMediaPickerPreset;
use rise_validate::{CheckRule, CollectionLimit, TextSchema, Unit, UrlPolicy, Violation, ViolationKind};

/// validation schema helpers over the linked canon schema
/// the for_tier forms rebuild the schema with the tier ceilings
pub trait ValidationSchemaCanonRules {
    /// rule ceiling in its own unit
    /// not input_limit, that headroom exists for the too long message
    /// a field with nowhere to show it would clamp to an invalid value
    fn maximum(&self) -> Option<(usize, Unit)>;

    /// same ceiling for one tier, a tiered rule gives the subscriber bound when premium
    fn maximum_for_tier(&self, premium: bool) -> Option<(usize, Unit)>;

    /// same ceiling as a bare number, for counters like 17/50 and clamping
    /// a schema without a ceiling is a typo in its name and not data
    fn maximum_value(&self) -> usize;

    /// rule lower bound in its own unit, none for fields canon gave none so no too short
    fn minimum(&self) -> Option<(usize, Unit)>;

    fn minimum_value(&self) -> usize;

    /// required value suffix, only for the bot tag
    /// its part of the rule since the server refuses a tag without it
    fn required_suffix(&self) -> &'static str;

    fn charset_message_key(&self) -> Option<&'static str>;

    /// first violated rule as an i18n key, only one message fits under a field
    fn error_key(&self, raw: &str) -> Option<&'static str>;

    fn error_key_for_raw_value(&self, raw: &str) -> Option<&'static str>;

    /// first violated rule as a ready string in the users language
    /// ask for this and not error_key because min and max messages carry %d that tr would print raw
    fn error_text(&self, raw: &str) -> Option<String>;

    fn error_text_for_raw_value(&self, raw: &str) -> Option<String>;

    fn error_text_for_raw_value_for_tier(&self, raw: &str, premium: bool) -> Option<String>;

    /// formatting applies only to bounds
    /// other violations have nothing to substitute and a stray printf over a foreign locale gives garbage
    fn text(violation: &Violation) -> Option<String>
    where
        Self: Sized;

    /// what a field keeps while typing when theres nowhere to say too long
    /// the trailing space stays since its about to be erased or continued by a word
    fn clamped(&self, raw: &str) -> String;

    fn clamped_for_tier(&self, raw: &str, premium: bool) -> String;

    /// same for a value already heading to the wire
    /// whitespace is trimmed on both sides since nobody types into it anymore
    fn normalized_clamped(&self, raw: &str) -> String;
}

impl ValidationSchemaCanonRules for TextSchema {
    fn maximum(&self) -> Option<(usize, Unit)> {
        self.maximum_for_tier(false)
    }

    fn maximum_for_tier(&self, premium: bool) -> Option<(usize, Unit)> {
        self.rules().iter().find_map(|check| match check.rule() {
            CheckRule::Max { value, premium: tiered, unit } => match (premium, tiered) {
                (true, Some(tiered)) => Some((tiered, unit)),
                _ => Some((value, unit)),
            },
            _ => None,
        })
    }

    fn maximum_value(&self) -> usize {
        match self.maximum() {
            Some((value, _)) => value,
            None => panic!("Canon schema {} has no maximum", self.field()),
        }
    }

    fn minimum(&self) -> Option<(usize, Unit)> {
        self.rules().iter().find_map(|check| match check.rule() {
            CheckRule::Min { value, unit } => Some((value, unit)),
            _ => None,
        })
    }

    fn minimum_value(&self) -> usize {
        match self.minimum() {
            Some((value, _)) => value,
            None => panic!("Canon schema {} has no minimum", self.field()),
        }
    }

    fn required_suffix(&self) -> &'static str {
        let suffix = self.rules().iter().find_map(|check| match check.rule() {
            CheckRule::EndsWith(suffix) => Some(suffix),
            _ => None,
        });
        match suffix {
            Some(suffix) => suffix,
            None => panic!("Canon schema {} has no required suffix", self.field()),
        }
    }

    fn charset_message_key(&self) -> Option<&'static str> {
        self.rules()
            .iter()
            .find(|check| matches!(check.rule(), CheckRule::Charset(_)))
            .and_then(|check| check.message_key())
    }

    fn error_key(&self, raw: &str) -> Option<&'static str> {
        self.check(raw).err().and_then(|violation| violation.message_key)
    }

    fn error_key_for_raw_value(&self, raw: &str) -> Option<&'static str> {
        if self.normalize(raw) != raw
            && let Some(key) = self.charset_message_key()
        {
            return Some(key);
        }
        self.error_key(raw)
    }

    fn error_text(&self, raw: &str) -> Option<String> {
        let violation = self.check(raw).err()?;
        Self::text(&violation)
    }

    fn error_text_for_raw_value(&self, raw: &str) -> Option<String> {
        self.error_text_for_raw_value_for_tier(raw, false)
    }

    fn error_text_for_raw_value_for_tier(&self, raw: &str, premium: bool) -> Option<String> {
        if self.normalize(raw) != raw
            && let Some(key) = self.charset_message_key()
        {
            return Some(tr(key));
        }
        let violation = self.check_for(raw, premium).err()?;
        Self::text(&violation)
    }

    fn text(violation: &Violation) -> Option<String> {
        let key = violation.message_key?;

        match violation.kind {
            ViolationKind::TooLong { maximum, .. } => Some(tr_args(key, &[bound(maximum)])),
            ViolationKind::TooShort { minimum, .. } => Some(tr_args(key, &[bound(minimum)])),
            _ => Some(tr(key)),
        }
    }

    fn clamped(&self, raw: &str) -> String {
        self.clamped_for_tier(raw, false)
    }

    fn clamped_for_tier(&self, raw: &str, premium: bool) -> String {
        truncated_to_maximum(self.maximum_for_tier(premium), self.input(raw))
    }

    fn normalized_clamped(&self, raw: &str) -> String {
        truncated_to_maximum(self.maximum(), self.normalize(raw))
    }
}

fn bound(value: usize) -> FormatArg<'static> {
    FormatArg::Int(i64::try_from(value).unwrap_or(i64::MAX))
}

/// a character is never split, steps down to the last one that fits whole
/// first pass only counts so nothing is rewritten on every keystroke with nothing to cut
fn truncated_to_maximum(limit: Option<(usize, Unit)>, value: String) -> String {
    let Some((maximum, unit)) = limit else {
        return value;
    };
    if unit.count(&value) <= maximum {
        return value;
    }
    let mut out = String::with_capacity(value.len());
    let mut used = 0usize;
    for character in value.chars() {
        let cost = match unit {
            Unit::Bytes => character.len_utf8(),
            Unit::Chars => 1,
            Unit::Utf16 => character.len_utf16(),
            Unit::Digits => usize::from(character.is_ascii_digit()),
        };
        if used + cost > maximum {
            break;
        }
        used += cost;
        out.push(character);
    }
    out
}

/// validation canon helpers for this bridge
pub trait RiseValidationCanonRules {
    /// collection limit by canon name
    /// a miss is a typo in code so it panics instead of guessing a number
    fn collection_limit(name: &str) -> CollectionLimit;

    fn url_policy(name: &str) -> UrlPolicy;
}

impl RiseValidationCanonRules for RiseValidation {
    fn collection_limit(name: &str) -> CollectionLimit {
        match rise_validate::collection_limit(name) {
            Some(limit) => limit,
            None => panic!("Canon collection limit {name} is missing"),
        }
    }

    fn url_policy(name: &str) -> UrlPolicy {
        match rise_validate::url_policy(name) {
            Some(policy) => policy,
            None => panic!("Canon url policy {name} is missing"),
        }
    }
}

/// limits canon helpers for this bridge
pub trait RiseLimitsCanonRules {
    /// product limit by canon name
    fn value(name: &str) -> i64;

    /// cache budget by canon name, numbers are shared and eviction stays platform specific
    fn cache_budget(name: &str) -> i64;

    /// silent backend ceiling by canon name
    /// separate from value since a product limit is one the user sees
    /// server applies this one silently, ask for more and you get less with no sign
    fn service_value(name: &str) -> i64;

    /// ceiling set by the client itself
    /// added after ios kept its own numbers that duplicated or missed what canon described
    fn client_value(name: &str) -> i64;

    /// media picker preset by canon name, a miss is a typo in code
    fn media_picker_preset(name: &str) -> &'static RiseMediaPickerPreset;
}

impl RiseLimitsCanonRules for RiseLimits {
    fn value(name: &str) -> i64 {
        match rise_limits::spec(name) {
            Some(spec) => rise_limits::signed(spec.value),
            None => panic!("Canon limit {name} is missing"),
        }
    }

    fn cache_budget(name: &str) -> i64 {
        match rise_limits::budget(name) {
            Some(spec) => rise_limits::signed(spec.value),
            None => panic!("Canon cache budget {name} is missing"),
        }
    }

    fn service_value(name: &str) -> i64 {
        match rise_limits::SERVICE_LIMITS.iter().find(|spec| spec.name == name) {
            Some(spec) => rise_limits::signed(spec.value),
            None => panic!("Canon service limit {name} is missing"),
        }
    }

    fn client_value(name: &str) -> i64 {
        match rise_limits::CLIENT_LIMITS.iter().find(|spec| spec.name == name) {
            Some(spec) => rise_limits::signed(spec.value),
            None => panic!("Canon client limit {name} is missing"),
        }
    }

    fn media_picker_preset(name: &str) -> &'static RiseMediaPickerPreset {
        match rise_limits::preset(name) {
            Some(preset) => preset,
            None => panic!("Canon media picker preset {name} is missing"),
        }
    }
}

pub trait RiseMediaPickerPresetCanonRules {
    /// canon measures video in milliseconds and the picker and trimming use seconds
    fn max_video_duration_seconds(&self) -> Option<i64>;
}

impl RiseMediaPickerPresetCanonRules for RiseMediaPickerPreset {
    fn max_video_duration_seconds(&self) -> Option<i64> {
        self.max_video_duration_ms
            .map(|milliseconds| rise_limits::signed(milliseconds) / 1_000)
    }
}
