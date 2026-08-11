use rise_validate::{Charset, Check, TextSchema, Violation, ViolationKind};

pub const PHONE: TextSchema = TextSchema::new("phone")
    .digits_only()
    .missing_message("auth_phone_invalid")
    .checks(&[
        Check::min_digits(7).message("auth_phone_invalid"),
        Check::max_digits(15).message("auth_phone_invalid"),
    ]);

pub const NAME: TextSchema = TextSchema::new("name")
    .trim()
    .missing_message("auth_name_too_short")
    .checks(&[
        Check::min_chars(2).message("auth_name_too_short"),
        Check::max_chars(64).message("auth_name_invalid"),
        Check::predicate("printable", is_printable).message("auth_name_invalid"),
    ]);

pub const PASSWORD: TextSchema = TextSchema::new("password")
    .missing_message("auth_password_too_short")
    .checks(&[
        Check::min_chars(8).message("auth_password_too_short"),
        Check::max_chars(128).message("auth_password_too_long"),
    ]);

// Empty by design: PASSWORD's 8 is a product rule the server (6) never had, so it would lock out older accounts.
pub const LOGIN_PASSWORD: TextSchema = TextSchema::new("password")
    .missing_message("auth_password_required")
    .checks(&[]);

pub const TAG: TextSchema = TextSchema::new("tag")
    .trim()
    .lowercase()
    .missing_message("tag_too_short")
    .checks(&[
        Check::min_bytes(3).message("tag_too_short"),
        Check::max_bytes(32).message("auth_tag_too_long"),
        Check::charset(Charset::TAG).message("tag_invalid_characters"),
        Check::not_edged_with('_').message("auth_tag_edge_underscore"),
        Check::no_run_of("__").message("auth_tag_double_underscore"),
    ]);

pub const CODE_LENGTH: usize = 4;

pub const CODE: TextSchema = TextSchema::new("code")
    .digits_only()
    .missing_message("send_code_invalidcode_error")
    .checks(&[
        Check::min_chars(CODE_LENGTH).message("send_code_invalidcode_error"),
        Check::max_chars(CODE_LENGTH).message("send_code_invalidcode_error"),
    ]);

fn is_printable(value: &str) -> bool {
    !value.chars().any(|character| {
        character.is_control()
            || matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
    })
}

pub fn normalize_tag(raw: &str) -> String {
    TAG.normalize(raw.trim().trim_start_matches('@'))
}

pub const TAG_INPUT_LIMIT: usize = 40;

pub fn tag_input(raw: &str) -> String {
    raw.trim_start()
        .trim_start_matches('@')
        .to_lowercase()
        .chars()
        .take(TAG_INPUT_LIMIT)
        .collect()
}

pub fn code_input(raw: &str) -> String {
    raw.chars()
        .filter(char::is_ascii_digit)
        .take(CODE_LENGTH)
        .collect()
}

pub fn message_key(violation: &Violation) -> &'static str {
    violation.message_key.unwrap_or(match violation.kind {
        ViolationKind::Missing => "field_required",
        _ => "retry_later_error",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_matches_the_seven_to_fifteen_digit_server_bound() {
        assert!(PHONE.check("").is_err());
        assert!(PHONE.check("123456").is_err());
        assert!(PHONE.check("1234567").is_ok());
        assert!(PHONE.check("123456789012345").is_ok());
        assert!(PHONE.check("1234567890123456").is_err());
    }

    #[test]
    fn a_phone_keeps_only_digits_so_pasted_punctuation_is_not_an_error() {
        assert_eq!(PHONE.check("+7 (707) 580-32-72").unwrap(), "77075803272");
    }

    #[test]
    fn a_name_is_counted_in_characters_so_cyrillic_is_not_penalised() {
        assert!(NAME.check("Ян").is_ok());
        assert!(NAME.check(" Я ").is_err());
        assert!(NAME.check("   ").is_err());

        let sixty_four: String = "я".repeat(64);
        assert!(
            NAME.check(&sixty_four).is_ok(),
            "the server counts chars; counting bytes would refuse a name it accepts"
        );
        assert!(NAME.check(&"я".repeat(65)).is_err());
    }

    #[test]
    fn a_name_may_not_smuggle_control_characters_or_bidi_overrides() {
        assert!(NAME.check("Ян\nПетров").is_err());
        assert!(NAME.check("Ян\u{202E}вортеП").is_err());
        assert!(NAME.check("\u{FEFF}Ян").is_err());
        assert!(NAME.check("Ян Петров").is_ok());
    }

    #[test]
    fn a_registration_password_is_the_eight_the_message_promises() {
        assert!(PASSWORD.check("1234567").is_err());
        assert!(PASSWORD.check("12345678").is_ok());
        assert_eq!(
            PASSWORD.check("1234567").unwrap_err().message_key,
            Some("auth_password_too_short")
        );
        assert!(PASSWORD.check(&"a".repeat(129)).is_err());
    }

    #[test]
    fn a_password_is_never_trimmed_because_the_server_does_not_trim_either() {
        assert!(
            PASSWORD.check("        ").is_ok(),
            "altering the user's password silently is worse than accepting spaces"
        );
    }

    #[test]
    fn signing_in_accepts_a_password_that_predates_the_eight_character_rule() {
        assert!(LOGIN_PASSWORD.check("old123").is_ok());
        assert_eq!(
            LOGIN_PASSWORD.check("").unwrap_err().message_key,
            Some("auth_password_required"),
            "an empty field is not a short password"
        );
    }

    #[test]
    fn a_tag_follows_the_rules_that_live_three_hops_away_in_tag_service() {
        assert!(TAG.check("riseonly_1").is_ok());
        assert_eq!(TAG.check("  RiseOnly  ").unwrap(), "riseonly");
        assert!(TAG.check("ab").is_err());
        assert!(TAG.check(&"a".repeat(33)).is_err());
        assert!(TAG.check("with-dash").is_err());
        assert!(TAG.check("тег123").is_err());
        assert!(TAG.check("_lead").is_err());
        assert!(TAG.check("trail_").is_err());
        assert!(TAG.check("do__uble").is_err());
    }

    #[test]
    fn every_tag_problem_gets_its_own_sentence_rather_than_the_step_subtitle() {
        for (tag, expected) in [
            ("ab", "tag_too_short"),
            ("a".repeat(33).as_str(), "auth_tag_too_long"),
            ("тег", "tag_invalid_characters"),
            ("_lead", "auth_tag_edge_underscore"),
            ("do__uble", "auth_tag_double_underscore"),
        ] {
            let violation = TAG.check(tag).unwrap_err();
            assert_eq!(message_key(&violation), expected, "{tag}");
            assert_ne!(
                message_key(&violation),
                "auth_tag_step_subtitle",
                "{tag} rendered as the step's own subtitle"
            );
        }
    }

    #[test]
    fn a_cyrillic_tag_is_reported_as_a_character_problem_and_names_the_letter() {
        let violation = TAG.check("тег").unwrap_err();
        assert_eq!(violation.offending_char(), Some('т'));
        assert!(violation.offending_is_non_ascii());
    }

    #[test]
    fn the_at_sign_the_subtitle_advertises_is_stripped_rather_than_refused() {
        assert_eq!(normalize_tag("@riseonly"), "riseonly");
        assert_eq!(normalize_tag("  @RiseOnly "), "riseonly");
        assert!(TAG.check(&normalize_tag("@riseonly")).is_ok());
    }

    #[test]
    fn the_tag_field_shows_what_will_actually_be_sent() {
        assert_eq!(tag_input("@RiseOnly"), "riseonly");
        assert_eq!(tag_input(&"a".repeat(60)).len(), TAG_INPUT_LIMIT);
    }

    #[test]
    fn a_code_is_four_digits_and_pasted_whitespace_never_reaches_the_server() {
        assert!(CODE.check("1234").is_ok());
        assert_eq!(CODE.check(" 1234 ").unwrap(), "1234");
        assert!(CODE.check("").is_err());
        assert!(CODE.check("12").is_err());
        assert!(CODE.check("12345").is_err());
        assert!(CODE.check("12a4").is_err());
    }

    #[test]
    fn the_code_field_accepts_nothing_but_four_digits_as_it_is_typed() {
        assert_eq!(code_input("1a2b3c4d5"), "1234");
        assert_eq!(code_input(" 12 34 "), "1234");
    }

    #[test]
    fn every_schema_names_a_key_on_every_rule_it_can_break() {
        let broken = [
            TAG.check("_"),
            TAG.check("тег"),
            TAG.check(&"a".repeat(40)),
            NAME.check(""),
            NAME.check(&"я".repeat(65)),
            PASSWORD.check("1"),
            PASSWORD.check(&"a".repeat(200)),
            PHONE.check("1"),
            CODE.check("1"),
        ];
        for outcome in broken {
            let violation = outcome.unwrap_err();
            assert!(
                violation.message_key.is_some(),
                "{violation:?} would render the last-resort default"
            );
        }
    }
}
