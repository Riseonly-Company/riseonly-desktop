/// Why a value was refused, as data — never a rendered sentence.
///
/// Every variant carries the numbers the message needs, so a caller interpolates
/// rather than hard-coding them a second time and letting the two drift.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ViolationKind {
    /// Nothing was typed, or only whitespace that normalisation removed.
    Missing,
    TooShort {
        minimum: usize,
        actual: usize,
        unit: Unit,
    },
    TooLong {
        maximum: usize,
        actual: usize,
        unit: Unit,
    },
    /// A character the field does not accept; `offending` is the FIRST one.
    Charset {
        charset: &'static str,
        offending: char,
    },
    /// A character legal inside the value but not at its edge — the tag underscore rule.
    Edge { character: char, edge: Edge },
    /// A run that is legal once but not twice, such as `__`.
    Run { run: &'static str },
    /// Two fields that had to agree did not.
    Mismatch,
    /// A named rule with no general shape. `name` is a stable slug, not prose.
    Failed { rule: &'static str },
}

/// What a length was counted in. Load-bearing: auth-service counts a name in
/// `chars()` while tag-service bounds a tag in BYTES.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Characters,
    Bytes,
    /// ASCII digits only, ignoring spaces, brackets and dashes.
    Digits,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Leading,
    Trailing,
}

/// One refused value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Violation {
    /// The schema that refused it — a stable slug like `tag`.
    pub field: &'static str,
    pub kind: ViolationKind,
    /// The i18n key the app should render. `None` on purpose: a schema may leave
    /// the wording to the screen, and a defaulted English string would render.
    pub message_key: Option<&'static str>,
}

impl Violation {
    pub fn new(field: &'static str, kind: ViolationKind) -> Self {
        Self {
            field,
            kind,
            message_key: None,
        }
    }

    pub fn with_message(mut self, key: Option<&'static str>) -> Self {
        self.message_key = key;
        self
    }

    /// The character that caused a [`ViolationKind::Charset`] refusal.
    pub fn offending_char(&self) -> Option<char> {
        match self.kind {
            ViolationKind::Charset { offending, .. } => Some(offending),
            ViolationKind::Edge { character, .. } => Some(character),
            _ => None,
        }
    }

    /// Whether the offending character is outside ASCII: "latin letters only" is a
    /// different message from "not this character".
    pub fn offending_is_non_ascii(&self) -> bool {
        self.offending_char()
            .is_some_and(|character| !character.is_ascii())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_charset_refusal_names_the_character_that_caused_it() {
        let violation = Violation::new(
            "tag",
            ViolationKind::Charset {
                charset: "a-z0-9_",
                offending: 'й',
            },
        );
        assert_eq!(violation.offending_char(), Some('й'));
        assert!(violation.offending_is_non_ascii());
    }

    #[test]
    fn an_ascii_offender_is_reported_as_ascii() {
        let violation = Violation::new(
            "tag",
            ViolationKind::Charset {
                charset: "a-z0-9_",
                offending: '-',
            },
        );
        assert!(!violation.offending_is_non_ascii());
    }

    #[test]
    fn a_length_refusal_has_no_offending_character() {
        let violation = Violation::new(
            "tag",
            ViolationKind::TooShort {
                minimum: 3,
                actual: 2,
                unit: Unit::Characters,
            },
        );
        assert_eq!(violation.offending_char(), None);
        assert!(!violation.offending_is_non_ascii());
    }
}
