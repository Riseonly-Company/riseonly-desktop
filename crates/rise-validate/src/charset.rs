/// Which characters a field accepts: a named predicate, so a violation can say
/// WHAT was expected rather than echo a pattern.
#[derive(Clone, Copy)]
pub struct Charset {
    /// Shown to nobody; carried in the violation for traces and tests.
    pub name: &'static str,
    allows: fn(char) -> bool,
}

impl Charset {
    /// tag-service's alphabet: lowercase ASCII letters, digits and underscore.
    pub const TAG: Self = Self {
        name: "a-z0-9_",
        allows: |character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        },
    };

    pub const DIGITS: Self = Self {
        name: "0-9",
        allows: |character| character.is_ascii_digit(),
    };

    /// What a phone field tolerates WHILE BEING TYPED: digits plus the punctuation
    /// people paste around them, which is formatting rather than an error.
    pub const PHONE_INPUT: Self = Self {
        name: "0-9 +()- ",
        allows: |character| {
            character.is_ascii_digit() || matches!(character, '+' | '(' | ')' | '-' | ' ' | '.')
        },
    };

    pub const fn custom(name: &'static str, allows: fn(char) -> bool) -> Self {
        Self { name, allows }
    }

    pub fn allows(&self, character: char) -> bool {
        (self.allows)(character)
    }

    /// The first character the set refuses, which is the one worth naming.
    pub fn first_offender(&self, value: &str) -> Option<char> {
        value.chars().find(|character| !self.allows(*character))
    }
}

impl std::fmt::Debug for Charset {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_tuple("Charset").field(&self.name).finish()
    }
}

impl PartialEq for Charset {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Charset {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_alphabet_is_tag_services_own() {
        assert!(Charset::TAG.first_offender("riseonly_1").is_none());
        assert_eq!(Charset::TAG.first_offender("RiseOnly"), Some('R'));
        assert_eq!(Charset::TAG.first_offender("with-dash"), Some('-'));
        assert_eq!(Charset::TAG.first_offender("тег"), Some('т'));
        assert_eq!(Charset::TAG.first_offender("emoji🙂"), Some('🙂'));
    }

    /// The offender is the FIRST one, so a message can name a character the user can find.
    #[test]
    fn the_first_offender_is_the_one_reported() {
        assert_eq!(Charset::TAG.first_offender("ok-then-Bad"), Some('-'));
    }

    #[test]
    fn a_phone_field_tolerates_the_punctuation_people_paste() {
        assert!(
            Charset::PHONE_INPUT
                .first_offender("+7 (707) 580-32-72")
                .is_none()
        );
        assert_eq!(Charset::PHONE_INPUT.first_offender("7о7"), Some('о'));
    }

    #[test]
    fn an_empty_value_offends_nothing() {
        assert_eq!(Charset::TAG.first_offender(""), None);
    }

    #[test]
    fn a_custom_charset_carries_its_own_name() {
        let hex = Charset::custom("0-9a-f", |c| {
            c.is_ascii_hexdigit() && !c.is_ascii_uppercase()
        });
        assert_eq!(hex.name, "0-9a-f");
        assert_eq!(hex.first_offender("deadbeefG"), Some('G'));
    }
}
