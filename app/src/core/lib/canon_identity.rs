use std::collections::HashSet;
use std::sync::LazyLock;

use rise_canon::rise_key;
use rise_canon::{
    RiseIdentityCanon, RiseIdentityContentSignal, RiseIdentityRealtimeLevel,
    RiseIdentitySelfSource, RiseIdentityTimestampUnit,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

/// canon identity rules, every answer is a pure function of its arguments
pub enum RiseIdentity {}

impl RiseIdentity {
    /// resolves who am I, source order comes from canon
    /// access token subject first since the server attributes the request to it, then the profile
    pub fn viewer(
        access_token_subject: impl FnOnce() -> Option<String>,
        profile_id: impl FnOnce() -> Option<String>,
    ) -> Option<String> {
        let mut access_token_subject = Some(access_token_subject);
        let mut profile_id = Some(profile_id);
        for source in RiseIdentitySelfSource::ALL {
            let raw = match source {
                RiseIdentitySelfSource::AccessTokenSubject => {
                    access_token_subject.take().and_then(|read| read())
                }
                RiseIdentitySelfSource::ProfileId => profile_id.take().and_then(|read| read()),
            };
            if let Some(value) = Self::present_viewer(raw.as_deref()) {
                return Some(value);
            }
        }
        None
    }

    /// is this me, the candidate is compared only with the resolved value
    /// matching a stale profile once showed Edit on someone elses post
    pub fn is_viewer(candidate: Option<&str>, resolved: Option<&str>) -> bool {
        if !*VIEWER_COMPARES_RESOLVED_ONLY {
            return false;
        }
        let (Some(candidate), Some(resolved)) =
            (Self::present_viewer(candidate), Self::present_viewer(resolved))
        else {
            return false;
        };
        candidate == resolved
    }

    /// row author in canon key order
    /// the flat field goes first, its the rows own foreign key and always there
    /// the nested object is a join the gateway may omit
    pub fn author(
        flat: Option<&str>,
        camel: Option<&str>,
        nested: Option<&str>,
    ) -> RiseAuthorIdentity {
        for key in AUTHOR_KEYS.iter() {
            let raw = match key {
                AuthorKey::Flat => flat,
                AuthorKey::CamelCase => camel,
                AuthorKey::Nested => nested,
            };
            let Some(value) = Self::present_author(raw) else {
                continue;
            };
            if Self::is_self_sentinel(&value) {
                return RiseAuthorIdentity::SelfSentinel;
            }
            return RiseAuthorIdentity::Known(value);
        }
        RiseAuthorIdentity::Unknown
    }

    /// true if the record is mine
    /// the me sentinel means whoever was current at write time
    /// so it resolves only when the record account matches
    pub fn is_own(
        author: &RiseAuthorIdentity,
        viewer: Option<&str>,
        record_account_id: Option<&str>,
    ) -> bool {
        let Some(viewer) = Self::present_viewer(viewer) else {
            return false;
        };

        match author {
            RiseAuthorIdentity::Unknown => false,
            RiseAuthorIdentity::SelfSentinel => {
                if !RiseIdentityCanon::AUTHOR_SENTINEL_REQUIRES_ACCOUNT_MATCH {
                    return true;
                }
                let Some(account) = Self::present_viewer(record_account_id) else {
                    return false;
                };
                account == viewer
            }
            RiseAuthorIdentity::Known(id) => *id == viewer,
        }
    }

    /// unit the timestamp arrived in, threshold is strict so exactly 1_000_000_000_000 is still seconds
    pub fn unit(raw: i64) -> RiseIdentityTimestampUnit {
        let compared = if *TIMESTAMP_COMPARES_MAGNITUDE {
            i64::try_from(raw.unsigned_abs()).unwrap_or(i64::MAX)
        } else {
            raw
        };
        if compared > RiseIdentityCanon::TIMESTAMP_MILLISECOND_FLOOR {
            RiseIdentityTimestampUnit::Millisecond
        } else {
            RiseIdentityTimestampUnit::Second
        }
    }

    /// timestamp in milliseconds, what normalization outputs
    pub fn milliseconds(raw: i64) -> i64 {
        if Self::unit(raw) != RiseIdentityTimestampUnit::Second {
            return raw;
        }

        let (product, overflow) =
            raw.overflowing_mul(RiseIdentityCanon::TIMESTAMP_MILLISECONDS_PER_SECOND);
        if !overflow {
            return product;
        }

        if *TIMESTAMP_SATURATES_ON_OVERFLOW {
            if raw < 0 { i64::MIN } else { i64::MAX }
        } else {
            product
        }
    }

    pub fn milliseconds_from_double(raw: f64) -> i64 {
        Self::milliseconds(Self::clamped_to_int64(raw))
    }

    /// one composite key component
    /// length is counted in utf-8 bytes since languages measure strings differently but the key must match
    pub fn component(value: &str) -> String {
        if !(*KEY_COMPONENTS_ARE_LENGTH_PREFIXED && *KEY_LENGTH_COUNTS_UTF8_BYTES) {
            panic!("Canon key encoding is unmapped");
        }
        format!(
            "{}{}{}",
            value.len(),
            RiseIdentityCanon::KEY_ENCODING_LENGTH_SEPARATOR,
            value
        )
    }

    /// joins components, a separator inside a value isnt escaped
    /// the length prefix protects against it so 3:a|b parses unambiguously
    pub fn composite_key(parts: &[String]) -> String {
        parts.join(RiseIdentityCanon::KEY_ENCODING_COMPONENT_SEPARATOR)
    }

    /// free text before key encoding, trimmed and lowercased
    /// trim alone gave two keys for the same tag
    pub fn scope_text(value: &str) -> String {
        Self::normalized(value, &RiseIdentityCanon::SCOPE_KEY_TEXT_NORMALIZATIONS)
    }

    /// identifier before key encoding, left untouched since its case matters
    pub fn scope_identifier(value: &str) -> String {
        Self::normalized(value, &RiseIdentityCanon::SCOPE_KEY_IDENTIFIER_NORMALIZATIONS)
    }

    /// list inside one key component
    pub fn list(values: &[String]) -> String {
        values.join(RiseIdentityCanon::KEY_ENCODING_LIST_SEPARATOR)
    }

    /// flag inside a composite key
    pub fn flag(value: bool) -> &'static str {
        if value {
            RiseIdentityCanon::KEY_ENCODING_BOOLEAN_TRUE
        } else {
            RiseIdentityCanon::KEY_ENCODING_BOOLEAN_FALSE
        }
    }

    pub fn has_publication_signal(
        images: &[String],
        video: Option<&str>,
        track_id: Option<&str>,
        is_short: bool,
        is_image_reel: bool,
    ) -> bool {
        PUBLICATION_SIGNALS.iter().any(|signal| match signal {
            PublicationSignal::Images => !images.is_empty(),
            PublicationSignal::Video => Self::is_present_text(video),
            PublicationSignal::TrackId => Self::is_present_text(track_id),
            PublicationSignal::IsShort => is_short,
            PublicationSignal::IsImageReel => is_image_reel,
        })
    }

    pub fn content_kind_authoritative(value: &str) -> String {
        Self::normalized(value, &RiseIdentityCanon::CONTENT_KIND_AUTHORITATIVE_NORMALIZATIONS)
    }

    pub fn content_kind_fallback() -> &'static str {
        RiseIdentityCanon::CONTENT_KIND_FALLBACK
    }

    pub fn publish_signature(values: &[(&str, RiseIdentitySignatureValue)]) -> String {
        let unexpected: Vec<&str> = values
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| !PUBLISH_SIGNATURE_FIELD_NAMES.contains(*name))
            .collect();
        if !unexpected.is_empty() {
            panic!("Canon publish signature refuses {}", unexpected.join(", "));
        }

        let mut parts: Vec<String> =
            Vec::with_capacity(RiseIdentityCanon::PUBLISH_SIGNATURE_FIELDS.len());

        for field in RiseIdentityCanon::PUBLISH_SIGNATURE_FIELDS.iter() {
            let Some((_, supplied)) = values.iter().find(|(name, _)| *name == field.name) else {
                panic!("Canon publish signature field {} is missing", field.name);
            };
            parts.push(Self::component(&Self::encoded(supplied, field.value_kind, field.name)));
        }

        Self::composite_key(&parts)
    }

    pub fn realtime_applies(
        current: Option<&RiseRealtimeStamp>,
        incoming: &RiseRealtimeStamp,
    ) -> bool {
        let Some(current) = current else {
            return true;
        };
        if current.session_id != incoming.session_id {
            return RiseIdentityCanon::REALTIME_ORDER_SESSION_CHANGE_APPLIES;
        }

        for level in REALTIME_LEVELS.iter() {
            let (current_value, incoming_value) = match level {
                RealtimeLevel::Generation => (current.generation, incoming.generation),
                RealtimeLevel::Revision => (current.revision, incoming.revision),
            };
            if incoming_value != current_value {
                return incoming_value > current_value;
            }
        }

        false
    }

    fn present_viewer(raw: Option<&str>) -> Option<String> {
        Self::present(raw, RiseIdentityCanon::VIEWER_EMPTY_IS_ABSENT)
    }

    fn present_author(raw: Option<&str>) -> Option<String> {
        Self::present(raw, true)
    }

    fn present(raw: Option<&str>, empty_is_absent: bool) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(raw?);
        if trimmed.is_empty() && empty_is_absent {
            return None;
        }
        Some(trimmed.to_owned())
    }

    fn is_present_text(raw: Option<&str>) -> bool {
        Self::present(raw, true).is_some()
    }

    fn is_self_sentinel(trimmed: &str) -> bool {
        if RiseIdentityCanon::AUTHOR_SENTINEL_CASE_INSENSITIVE {
            SELF_SENTINELS.contains(trimmed.to_lowercase().as_str())
        } else {
            SELF_SENTINELS.contains(trimmed)
        }
    }

    fn normalized(value: &str, steps: &[&str]) -> String {
        let mut out = value.to_owned();
        for step in steps {
            out = match *step {
                "trim" => trimming_whitespaces_and_newlines(&out).to_owned(),
                "lowercase" => out.to_lowercase(),
                other => panic!("Canon normalization {other} is unmapped"),
            };
        }
        out
    }

    fn encoded(value: &RiseIdentitySignatureValue, value_kind: &str, field: &str) -> String {
        let (expected, encoded) = match value {
            RiseIdentitySignatureValue::Text(text) => ("text", text.clone()),
            RiseIdentitySignatureValue::Flag(state) => ("boolean", Self::flag(*state).to_owned()),
            RiseIdentitySignatureValue::Integer(number) => ("integer", number.to_string()),
            RiseIdentitySignatureValue::TextList(items) => ("text_list", Self::list(items)),
        };

        if expected != value_kind {
            panic!("Canon publish signature field {field} got {expected}, not {value_kind}");
        }
        encoded
    }

    fn clamped_to_int64(value: f64) -> i64 {
        if value.is_nan() {
            return 0;
        }
        if value >= 9_223_372_036_854_775_808.0 {
            return i64::MAX;
        }
        if value <= -9_223_372_036_854_775_808.0 {
            return i64::MIN;
        }
        value as i64
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AuthorKey {
    Flat,
    CamelCase,
    Nested,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PublicationSignal {
    Images,
    Video,
    TrackId,
    IsShort,
    IsImageReel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RealtimeLevel {
    Generation,
    Revision,
}

static AUTHOR_KEYS: LazyLock<Vec<AuthorKey>> = LazyLock::new(|| {
    RiseIdentityCanon::AUTHOR_ID_KEYS
        .iter()
        .map(|key: &&'static str| match *key {
            "author_id" => AuthorKey::Flat,
            "authorId" => AuthorKey::CamelCase,
            "author.id" => AuthorKey::Nested,
            other => panic!("Canon author key {other} is unmapped"),
        })
        .collect()
});

static PUBLICATION_SIGNALS: LazyLock<Vec<PublicationSignal>> = LazyLock::new(|| {
    RiseIdentityCanon::CONTENT_KIND_PUBLICATION_SIGNALS
        .iter()
        .map(|signal: &RiseIdentityContentSignal| match (signal.name, signal.emptiness) {
            ("images", "empty_collection") => PublicationSignal::Images,
            ("video", "blank_string") => PublicationSignal::Video,
            (rise_key::events::TRACK_ID, "blank_string") => PublicationSignal::TrackId,
            (rise_key::post::IS_SHORT, "false_flag") => PublicationSignal::IsShort,
            (rise_key::post::IS_IMAGE_REEL, "false_flag") => PublicationSignal::IsImageReel,
            (name, _) => panic!("Canon content signal {name} is unmapped"),
        })
        .collect()
});

static REALTIME_LEVELS: LazyLock<Vec<RealtimeLevel>> = LazyLock::new(|| {
    RiseIdentityCanon::REALTIME_ORDER_LEVELS
        .iter()
        .map(|level: &RiseIdentityRealtimeLevel| {
            if level.comparison != "strictly_greater" {
                panic!("Canon realtime comparison {} is unmapped", level.comparison);
            }
            match level.name {
                "generation" => RealtimeLevel::Generation,
                "revision" => RealtimeLevel::Revision,
                other => panic!("Canon realtime level {other} is unmapped"),
            }
        })
        .collect()
});

static VIEWER_COMPARES_RESOLVED_ONLY: LazyLock<bool> = LazyLock::new(|| {
    if RiseIdentityCanon::VIEWER_COMPARISON != "resolved_only" {
        panic!(
            "Canon viewer comparison {} is unmapped",
            RiseIdentityCanon::VIEWER_COMPARISON
        );
    }
    true
});

static TIMESTAMP_COMPARES_MAGNITUDE: LazyLock<bool> =
    LazyLock::new(|| RiseIdentityCanon::TIMESTAMP_ABSOLUTE_BEFORE_COMPARE);

static TIMESTAMP_SATURATES_ON_OVERFLOW: LazyLock<bool> = LazyLock::new(|| {
    if RiseIdentityCanon::TIMESTAMP_FLOOR_COMPARISON != "strictly_greater" {
        panic!("Canon timestamp comparison is unmapped");
    }
    RiseIdentityCanon::TIMESTAMP_OVERFLOW == "saturate"
});

static KEY_COMPONENTS_ARE_LENGTH_PREFIXED: LazyLock<bool> =
    LazyLock::new(|| RiseIdentityCanon::KEY_ENCODING_COMPONENT_FORMAT == "length_prefixed");

static KEY_LENGTH_COUNTS_UTF8_BYTES: LazyLock<bool> =
    LazyLock::new(|| RiseIdentityCanon::KEY_ENCODING_LENGTH_UNIT == "utf8_byte");

static PUBLISH_SIGNATURE_FIELD_NAMES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    RiseIdentityCanon::PUBLISH_SIGNATURE_FIELDS
        .iter()
        .map(|field| field.name)
        .collect()
});

static SELF_SENTINELS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    RiseIdentityCanon::AUTHOR_SELF_SENTINELS
        .iter()
        .map(|sentinel: &&'static str| {
            if RiseIdentityCanon::AUTHOR_SENTINEL_CASE_INSENSITIVE {
                sentinel.to_lowercase()
            } else {
                (*sentinel).to_owned()
            }
        })
        .collect()
});

/// result of resolving a rows author key
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RiseAuthorIdentity {
    Unknown,
    SelfSentinel,
    Known(String),
}

impl RiseAuthorIdentity {
    /// identifier fit for the wire and navigation
    /// the me sentinel isnt one, the server cant look a record up by it
    pub fn wire_id(&self) -> Option<&str> {
        match self {
            Self::Known(id) => Some(id),
            Self::Unknown | Self::SelfSentinel => None,
        }
    }
}

/// value of one publish signature field
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RiseIdentitySignatureValue {
    Text(String),
    Flag(bool),
    Integer(i64),
    TextList(Vec<String>),
}

/// payload position in realtime order, session then generation then revision
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseRealtimeStamp {
    pub session_id: String,
    pub generation: i64,
    pub revision: i64,
}
