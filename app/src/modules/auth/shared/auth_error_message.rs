pub fn message_key(server_message: Option<&str>, fallback: &'static str) -> &'static str {
    let normalized = readable(server_message.unwrap_or_default());
    if normalized.is_empty() {
        return fallback;
    }

    let has = |needles: &[&str]| needles.iter().any(|needle| normalized.contains(needle));

    if normalized.starts_with("missing ") {
        return fallback;
    }

    if has(&["или пароль", "or password", "неверный номер телефона"])
    {
        return "auth_signin_error";
    }

    if has(&["too many attempts. request", "request a new code"]) {
        return "auth_code_expired";
    }
    if has(&[
        "too many requests",
        "слишком много",
        "please wait",
        "подождите",
        "already in progress",
        "попробуйте через",
    ]) {
        return "auth_rate_limited";
    }

    if has(&["code expired", "code not found", "код истек", "код истёк"]) {
        return "auth_code_expired";
    }
    if has(&["invalid code", "неверный код"]) {
        return "send_code_invalidcode_error";
    }

    if has(&[
        "tag service unavailable",
        "failed to reserve tag",
        "tag reservation",
        "tag availability",
    ]) {
        return "tag_check_failed";
    }
    if has(&["tag", "тег"]) {
        if has(&[
            "already taken",
            "already owns",
            "protected",
            "занят",
            "существ",
        ]) {
            return "tag_already_exists";
        }
        if has(&["between 3 and 32", "3 and 32"]) {
            return "tag_too_short";
        }
        if has(&["start or end", "начина", "заканчива"]) {
            return "auth_tag_edge_underscore";
        }
        if has(&["consecutive", "подряд"]) {
            return "auth_tag_double_underscore";
        }
        return "tag_invalid_characters";
    }

    if has(&[
        "user already exists",
        "already registered",
        "уже зарегистр",
        "уже существ",
    ]) {
        return "auth_phone_already_registered";
    }
    if has(&["phone", "номер телефона"]) {
        return "auth_phone_invalid";
    }

    if has(&["password must", "парол"]) {
        return if has(&["больше", "too long", "128"]) {
            "auth_password_too_long"
        } else {
            "auth_password_too_short"
        };
    }

    if has(&["name must", "имя"]) {
        return "auth_name_too_short";
    }

    if has(&["unauthenticated", "refresh token", "session"]) {
        return "account_session_expired";
    }

    fallback
}

// api-gateway wraps auth prose in a stringified `tonic::Status` whose code name collides with the needles above.
fn readable(raw: &str) -> String {
    let mut text = raw.to_lowercase();

    for prefix in ["status: ", "details: ", "metadata: "] {
        while let Some(start) = text.find(prefix) {
            let end = section_end(&text, start + prefix.len());
            text.replace_range(start..end, " ");
        }
    }

    text.replace("message: ", " ")
        .replace(['"', '{', '}', '[', ']'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn section_end(text: &str, from: usize) -> usize {
    let mut depth = 0i32;
    for (offset, character) in text[from..].char_indices() {
        match character {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ',' if depth <= 0 => return from + offset,
            _ => {}
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrapped(operation: &str, code: &str, message: &str) -> String {
        format!(
            "{operation} failed: status: {code}, message: {message:?}, details: [], \
             metadata: MetadataMap {{ headers: {{}} }}"
        )
    }

    #[test]
    fn a_missing_or_blank_message_falls_back() {
        assert_eq!(message_key(None, "register_error"), "register_error");
        assert_eq!(
            message_key(Some("   "), "send_code_error"),
            "send_code_error"
        );
    }

    #[test]
    fn the_tonic_scaffolding_is_removed_and_every_human_word_survives() {
        let cleaned = readable(&wrapped(
            "Register",
            "InvalidArgument",
            "User already exists",
        ));
        assert!(cleaned.contains("user already exists"), "{cleaned}");
        assert!(
            !cleaned.contains("invalidargument"),
            "the code name collides with the 'invalid code' rule: {cleaned}"
        );
        assert!(!cleaned.contains("metadatamap"), "{cleaned}");

        let nested =
            readable("Failed to reserve tag: status: Internal, message: \"db\", details: []");
        assert!(nested.contains("failed to reserve tag"), "{nested}");

        assert_eq!(readable("Code expired"), "code expired");
    }

    #[test]
    fn a_taken_number_is_recognised_through_the_wrapper() {
        assert_eq!(
            message_key(
                Some(&wrapped(
                    "Register",
                    "InvalidArgument",
                    "User already exists"
                )),
                "register_error"
            ),
            "auth_phone_already_registered"
        );
    }

    #[test]
    fn a_wrong_password_says_wrong_credentials_rather_than_bad_phone() {
        let refusal = "Неверный номер телефона или пароль. Осталось попыток: 2";
        assert_eq!(
            message_key(Some(refusal), "auth_signin_error"),
            "auth_signin_error"
        );
        assert_eq!(
            message_key(
                Some(&wrapped("Login", "InvalidArgument", refusal)),
                "register_error"
            ),
            "auth_signin_error"
        );
    }

    #[test]
    fn a_lockout_and_a_cooldown_both_say_wait() {
        for message in [
            "Слишком много неудачных попыток входа. Попробуйте через 5 минут",
            "Слишком много неудачных попыток. Попробуйте через 4 минут 12 секунд",
            "Too many requests",
            "Please wait 47 seconds",
            "Registration is already in progress. Please retry in 47 seconds",
        ] {
            assert_eq!(
                message_key(Some(message), "send_code_error"),
                "auth_rate_limited",
                "{message}"
            );
        }
    }

    #[test]
    fn a_spent_code_budget_asks_for_a_new_code_rather_than_for_patience() {
        assert_eq!(
            message_key(
                Some("Too many attempts. Request a new code"),
                "register_error"
            ),
            "auth_code_expired"
        );
    }

    #[test]
    fn an_expired_code_is_not_confused_with_a_wrong_one() {
        assert_eq!(
            message_key(Some("Code expired"), "register_error"),
            "auth_code_expired"
        );
        assert_eq!(
            message_key(Some("Code not found"), "register_error"),
            "auth_code_expired"
        );
        assert_eq!(
            message_key(Some("Invalid code. 2 attempts remaining"), "register_error"),
            "send_code_invalidcode_error"
        );
    }

    #[test]
    fn every_tag_format_complaint_gets_its_own_sentence() {
        for (message, expected) in [
            ("Tag must be between 3 and 32 characters", "tag_too_short"),
            (
                "Tag must contain only letters, digits, and underscores",
                "tag_invalid_characters",
            ),
            (
                "Tag cannot start or end with an underscore",
                "auth_tag_edge_underscore",
            ),
            (
                "Tag cannot contain consecutive underscores",
                "auth_tag_double_underscore",
            ),
        ] {
            assert_eq!(
                message_key(Some(message), "register_error"),
                expected,
                "{message}"
            );
        }
    }

    #[test]
    fn a_taken_tag_and_the_protected_system_tag_both_read_as_occupied() {
        assert_eq!(
            message_key(
                Some("Tag is already taken or the entity already owns a tag"),
                "register_error"
            ),
            "tag_already_exists"
        );
        assert_eq!(
            message_key(
                Some("The Riseonly system tag is protected"),
                "register_error"
            ),
            "tag_already_exists"
        );
    }

    #[test]
    fn a_tag_backend_failure_is_never_reported_as_a_bad_tag() {
        for message in [
            "Tag service unavailable: transport error",
            "Failed to reserve tag: status: Internal, message: \"db\", details: []",
            "Tag reservation timeout",
            "Tag availability is temporarily unavailable",
        ] {
            assert_eq!(
                message_key(Some(message), "register_error"),
                "tag_check_failed",
                "{message}"
            );
        }
    }

    #[test]
    fn every_phone_complaint_reads_as_a_bad_number() {
        for message in [
            "Phone number is required",
            "Phone number has invalid formatting",
            "Phone number contains unsupported characters",
            "Phone number must contain 7 to 15 digits",
            "Registration phone is not a valid E.164 number",
        ] {
            assert_eq!(
                message_key(Some(message), "register_error"),
                "auth_phone_invalid",
                "{message}"
            );
        }
    }

    #[test]
    fn the_password_and_name_bounds_map_to_their_own_strings() {
        assert_eq!(
            message_key(
                Some("Password must contain 6 to 128 characters"),
                "register_error"
            ),
            "auth_password_too_long"
        );
        assert_eq!(
            message_key(
                Some("Name must contain 2 to 64 characters"),
                "register_error"
            ),
            "auth_name_too_short"
        );
    }

    #[test]
    fn a_rejected_refresh_reads_as_an_expired_session() {
        assert_eq!(
            message_key(
                Some(&wrapped(
                    "Refresh token",
                    "Unauthenticated",
                    "Invalid refresh token"
                )),
                "retry_later_error"
            ),
            "account_session_expired"
        );
    }

    #[test]
    fn an_unrecognised_message_never_leaks_the_servers_prose() {
        for message in [
            "Register timeout",
            "Client error: transport error: tcp connect error: 127.0.0.1:50051",
            "thread 'main' panicked at src/lib.rs:1",
            "Missing phone_number",
        ] {
            assert_eq!(
                message_key(Some(message), "register_error"),
                "register_error",
                "{message}"
            );
        }
    }
}
