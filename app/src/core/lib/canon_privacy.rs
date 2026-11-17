use rise_canon::{RisePrivacyAudience, RisePrivacyRule};

/// outcome of parsing a wire audience value
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RisePrivacyParsedAudience {
    Missing,
    Audience(RisePrivacyAudience),
    Unknown,
}

/// audience canon helpers for this bridge
pub trait RisePrivacyAudienceCanon: Sized {
    fn unknown_value() -> Self;

    /// wire value parsing, trim then case then exact match then aliases
    fn parse(raw: Option<&str>) -> RisePrivacyParsedAudience;
}

impl RisePrivacyAudienceCanon for RisePrivacyAudience {
    fn unknown_value() -> Self {
        RisePrivacyAudience::None
    }

    fn parse(raw: Option<&str>) -> RisePrivacyParsedAudience {
        let Some(raw) = raw else {
            return RisePrivacyParsedAudience::Missing;
        };

        let value = normalized_for_parsing(raw);
        if value.is_empty() {
            return RisePrivacyParsedAudience::Missing;
        }

        if let Some(exact) = RisePrivacyAudience::from_raw(&value) {
            return RisePrivacyParsedAudience::Audience(exact);
        }

        if let Some(aliased) = RisePrivacyAudience::ALL
            .into_iter()
            .find(|audience| audience.aliases().iter().any(|alias| *alias == value))
        {
            return RisePrivacyParsedAudience::Audience(aliased);
        }

        RisePrivacyParsedAudience::Unknown
    }
}

fn normalized_for_parsing(raw: &str) -> String {
    raw.trim_matches(is_ascii_whitespace)
        .chars()
        .map(|character| character.to_ascii_uppercase())
        .collect()
}

fn is_ascii_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{09}' | '\u{0A}' | '\u{0B}' | '\u{0C}' | '\u{0D}' | '\u{20}'
    )
}

/// privacy rule canon helpers for this bridge
pub trait RisePrivacyRuleCanon {
    /// rule audience from the wire value, the only function screens may use to ask who can see this
    fn audience(&self, raw: Option<&str>) -> RisePrivacyAudience;
}

impl RisePrivacyRuleCanon for RisePrivacyRule {
    fn audience(&self, raw: Option<&str>) -> RisePrivacyAudience {
        match RisePrivacyAudience::parse(raw) {
            RisePrivacyParsedAudience::Audience(audience) => audience,
            RisePrivacyParsedAudience::Missing => self.default_audience(),
            RisePrivacyParsedAudience::Unknown => RisePrivacyAudience::unknown_value(),
        }
    }
}
