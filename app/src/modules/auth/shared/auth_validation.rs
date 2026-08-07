/// The client-side half of the sign-up contract.
///
/// `VALIDATES_SUMMARY.md` records 712 fields with no validation on any server
/// layer, and auth is among them: the gateway reads a missing `name` as `""` and
/// never checks a tag at all — the real tag rules live three hops away in
/// tag-service, and its refusal arrives as "Failed to reserve tag". So the
/// client's validation is load-bearing rather than decorative: it is the only
/// thing between a typo and an error the user cannot act on.
///
/// Every bound below mirrors a server bound rather than inventing one. Being
/// stricter than the server would refuse accounts the product accepts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldProblem {
    Empty,
    TooShort,
    TooLong,
    Malformed,
}

impl FieldProblem {
    /// The reference's own strings, so the desktop says what the phone says in
    /// all 41 locales rather than adding keys that need translating.
    pub fn message_key(self, field: Field) -> &'static str {
        match (field, self) {
            (Field::Phone, _) => "auth_phone_invalid",
            (Field::Name, _) => "auth_name_invalid",
            (Field::Password, Self::TooLong) => "auth_password_too_long",
            (Field::Password, _) => "auth_password_too_short",
            (Field::Tag, _) => "auth_tag_step_subtitle",
            (Field::Code, _) => "send_code_invalidcode_error",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Phone,
    Name,
    Tag,
    Password,
    Code,
}

/// The number the server is sent: digits only, no `+`.
///
/// user-service prefixes `+` itself and then runs a REAL E.164 parse
/// (`canonicalize_account_phone`), so a country code that arrives twice is not
/// a cosmetic problem — `+7` and `77075803272` concatenated give
/// `+777075803272`, whose country code is `77`, which does not exist, and
/// registration fails with "Registration phone is not a valid E.164 number".
///
/// Until there is a country picker (it belongs to `user`, which owns the
/// catalogue) the field takes a whole number, so this has to work out whether
/// the user already typed their country code:
///
/// - a leading `+` or `00` means they said so explicitly, and nothing is added;
/// - digits that already begin with the calling code AND are long enough to be
///   a full international number are left alone;
/// - anything else is a local number and gets the calling code.
pub fn compose_phone(calling_code: &str, number: &str) -> String {
    let code: String = calling_code.chars().filter(char::is_ascii_digit).collect();
    let typed = number.trim();
    let digits: String = typed.chars().filter(char::is_ascii_digit).collect();

    if digits.is_empty() {
        return String::new();
    }

    // "+7707…" or "007707…": the user has named their country code.
    if typed.starts_with('+') {
        return digits;
    }
    if let Some(rest) = digits.strip_prefix("00")
        && !rest.is_empty()
    {
        return rest.to_owned();
    }

    if code.is_empty() {
        return digits;
    }

    // The shortest national number worth calling is nine digits, so a value
    // that already starts with the calling code and is at least that much
    // longer is a complete international number rather than a local one that
    // happens to begin with the same digit.
    let already_international =
        digits.starts_with(&code) && digits.len() >= code.len() + MINIMUM_NATIONAL_DIGITS;

    if already_international {
        digits
    } else {
        format!("{code}{digits}")
    }
}

/// How many digits a national number has at minimum, for the purpose of telling
/// "7707…" (a Kazakh number in full) from "7123…" (a local one).
const MINIMUM_NATIONAL_DIGITS: usize = 9;

pub fn validate_phone(composed: &str) -> Result<(), FieldProblem> {
    let digits = composed.chars().filter(char::is_ascii_digit).count();
    if digits == 0 {
        return Err(FieldProblem::Empty);
    }
    if digits < 7 {
        return Err(FieldProblem::TooShort);
    }
    if digits > 15 {
        return Err(FieldProblem::TooLong);
    }
    Ok(())
}

/// 2 to 64 characters after trimming, counted in characters and not bytes —
/// auth-service uses `chars().count()`, so a 40-character Cyrillic name is
/// valid even though it is 80 bytes.
pub fn validate_name(raw: &str) -> Result<(), FieldProblem> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FieldProblem::Empty);
    }
    match trimmed.chars().count() {
        0..=1 => Err(FieldProblem::TooShort),
        2..=64 => Ok(()),
        _ => Err(FieldProblem::TooLong),
    }
}

/// 6 to 128 characters, the auth-service bound.
///
/// The server does not trim, so six spaces are a valid password there. This does
/// not trim either: silently turning the user's password into something else is
/// how they end up unable to sign in from another client.
pub fn validate_password(raw: &str) -> Result<(), FieldProblem> {
    match raw.chars().count() {
        0 => Err(FieldProblem::Empty),
        1..=5 => Err(FieldProblem::TooShort),
        6..=128 => Ok(()),
        _ => Err(FieldProblem::TooLong),
    }
}

/// tag-service's rules: 3 to 32 bytes, `[a-z0-9_]`, no leading or trailing
/// underscore and no doubled one.
pub fn validate_tag(raw: &str) -> Result<(), FieldProblem> {
    let tag = normalize_tag(raw);
    if tag.is_empty() {
        return Err(FieldProblem::Empty);
    }
    if tag.len() < 3 {
        return Err(FieldProblem::TooShort);
    }
    if tag.len() > 32 {
        return Err(FieldProblem::TooLong);
    }
    if !tag
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(FieldProblem::Malformed);
    }
    if tag.starts_with('_') || tag.ends_with('_') || tag.contains("__") {
        return Err(FieldProblem::Malformed);
    }
    Ok(())
}

pub fn normalize_tag(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Four digits. auth-service formats the code with `{:04}` and checks nothing on
/// the way in, so this is entirely the client's rule — and it is the difference
/// between "that is not a code" and burning one of the five attempts the server
/// allows before it deletes the code outright.
pub fn validate_code(raw: &str) -> Result<(), FieldProblem> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FieldProblem::Empty);
    }
    if !trimmed.chars().all(|character| character.is_ascii_digit()) {
        return Err(FieldProblem::Malformed);
    }
    match trimmed.len() {
        1..=3 => Err(FieldProblem::TooShort),
        4 => Ok(()),
        _ => Err(FieldProblem::TooLong),
    }
}

pub fn passwords_match(password: &str, repeated: &str) -> bool {
    password == repeated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_number_gets_the_calling_code_and_loses_every_separator() {
        assert_eq!(compose_phone("+7", "(999) 123-45-67"), "79991234567");
        assert_eq!(compose_phone("7", "9991234567"), "79991234567");
    }

    /// The bug this function exists for: the user types their whole number,
    /// including the country code, and the code must not be added a second time.
    #[test]
    fn a_number_that_already_carries_its_country_code_is_not_given_another() {
        assert_eq!(
            compose_phone("7", "77075803272"),
            "77075803272",
            "prefixing again gives +777075803272, whose country code 77 does not exist"
        );
        assert_eq!(compose_phone("7", "79991234567"), "79991234567");
    }

    #[test]
    fn an_explicit_plus_or_double_zero_is_taken_at_its_word() {
        assert_eq!(compose_phone("7", "+380501234567"), "380501234567");
        assert_eq!(compose_phone("7", "+7 707 580 32 72"), "77075803272");
        assert_eq!(compose_phone("7", "00380501234567"), "380501234567");
    }

    #[test]
    fn a_local_number_starting_with_the_calling_code_digit_still_gets_it() {
        // Ten digits beginning with 7 is a national number, not an
        // international one: it is one digit short of a complete +7 number.
        assert_eq!(compose_phone("7", "7123456789"), "77123456789");
    }

    #[test]
    fn an_empty_field_composes_to_nothing_rather_than_to_the_bare_code() {
        assert_eq!(compose_phone("7", ""), "");
        assert_eq!(compose_phone("7", "   "), "");
        assert_eq!(
            compose_phone("", "9991234567"),
            "9991234567",
            "with no calling code there is nothing to prepend"
        );
    }

    #[test]
    fn everything_it_produces_is_digits_only() {
        for (code, typed) in [
            ("+7", "(999) 123-45-67"),
            ("7", "+7 707 580 32 72"),
            ("7", "00380 50 123 45 67"),
        ] {
            let composed = compose_phone(code, typed);
            assert!(
                composed.chars().all(|c| c.is_ascii_digit()),
                "{composed} is not digits; user-service prefixes the + itself"
            );
        }
    }

    #[test]
    fn a_phone_matches_the_seven_to_fifteen_digit_server_bound() {
        assert_eq!(validate_phone(""), Err(FieldProblem::Empty));
        assert_eq!(validate_phone("123456"), Err(FieldProblem::TooShort));
        assert!(validate_phone("1234567").is_ok());
        assert!(validate_phone("123456789012345").is_ok());
        assert_eq!(
            validate_phone("1234567890123456"),
            Err(FieldProblem::TooLong)
        );
    }

    #[test]
    fn a_name_is_counted_in_characters_so_cyrillic_is_not_penalised() {
        assert!(validate_name("Ян").is_ok());
        assert_eq!(validate_name(" Я "), Err(FieldProblem::TooShort));
        assert_eq!(validate_name("   "), Err(FieldProblem::Empty));

        let sixty_four: String = "я".repeat(64);
        assert!(
            validate_name(&sixty_four).is_ok(),
            "the server counts chars; counting bytes would refuse a name it accepts"
        );
        assert_eq!(validate_name(&"я".repeat(65)), Err(FieldProblem::TooLong));
    }

    #[test]
    fn a_password_is_six_to_a_hundred_and_twenty_eight_and_is_never_trimmed() {
        assert_eq!(validate_password("12345"), Err(FieldProblem::TooShort));
        assert!(validate_password("123456").is_ok());
        assert!(
            validate_password("      ").is_ok(),
            "the server accepts it, and altering the user's password silently is worse"
        );
        assert_eq!(
            validate_password(&"a".repeat(129)),
            Err(FieldProblem::TooLong)
        );
    }

    #[test]
    fn a_tag_follows_the_rules_that_live_three_hops_away_in_tag_service() {
        assert!(validate_tag("riseonly_1").is_ok());
        assert!(validate_tag("  RiseOnly  ").is_ok());
        assert_eq!(validate_tag("ab"), Err(FieldProblem::TooShort));
        assert_eq!(validate_tag(&"a".repeat(33)), Err(FieldProblem::TooLong));
        assert_eq!(validate_tag("with-dash"), Err(FieldProblem::Malformed));
        assert_eq!(validate_tag("тег123"), Err(FieldProblem::Malformed));
        assert_eq!(validate_tag("_lead"), Err(FieldProblem::Malformed));
        assert_eq!(validate_tag("trail_"), Err(FieldProblem::Malformed));
        assert_eq!(validate_tag("do__uble"), Err(FieldProblem::Malformed));
    }

    #[test]
    fn a_tag_is_lowercased_before_it_travels() {
        assert_eq!(normalize_tag("  RiseOnly "), "riseonly");
    }

    #[test]
    fn a_code_is_four_digits_because_the_server_checks_nothing_and_counts_attempts() {
        assert!(validate_code("1234").is_ok());
        assert_eq!(validate_code(""), Err(FieldProblem::Empty));
        assert_eq!(validate_code("12"), Err(FieldProblem::TooShort));
        assert_eq!(validate_code("12345"), Err(FieldProblem::TooLong));
        assert_eq!(validate_code("12a4"), Err(FieldProblem::Malformed));
    }

    #[test]
    fn every_problem_names_a_string_the_reference_already_ships() {
        for field in [
            Field::Phone,
            Field::Name,
            Field::Tag,
            Field::Password,
            Field::Code,
        ] {
            for problem in [
                FieldProblem::Empty,
                FieldProblem::TooShort,
                FieldProblem::TooLong,
                FieldProblem::Malformed,
            ] {
                assert!(!problem.message_key(field).is_empty());
            }
        }
    }
}
