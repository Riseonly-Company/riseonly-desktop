/// Turns a server message into a string the user can act on.
///
/// auth-service answers in prose, in two languages, with no error codes: "Tag
/// already exists", "Код истёк", "Too many requests". The reference matches on
/// substrings for exactly that reason, and this is that table ported rather than
/// redesigned — a desktop that classified errors differently from the phone
/// would show a different sentence for the same refusal.
///
/// Returns a translation KEY, never a rendered string: the caller owns the
/// locale, and a pure function is what makes the whole table testable.
pub fn message_key(server_message: Option<&str>, fallback: &'static str) -> &'static str {
    let normalized = server_message.unwrap_or_default().trim().to_lowercase();
    if normalized.is_empty() {
        return fallback;
    }

    let has = |needles: &[&str]| needles.iter().any(|needle| normalized.contains(needle));

    if has(&["tag", "тег"]) && has(&["exist", "reserved", "taken", "существ", "занят"])
    {
        return "tag_already_exists";
    }
    if has(&["phone", "телефон"]) && has(&["exist", "registered", "существ", "зарегистр"])
    {
        return "auth_phone_already_registered";
    }
    if has(&["code", "код"]) {
        if has(&["expired", "истек", "истёк"]) {
            return "auth_code_expired";
        }
        if has(&["invalid", "incorrect", "wrong", "невер"]) {
            return "send_code_invalidcode_error";
        }
    }
    if has(&[
        "too many",
        "rate limit",
        "60 seconds",
        "слишком много",
        "подождите",
    ]) {
        return "auth_rate_limited";
    }
    if has(&["phone", "номер"]) {
        return "auth_phone_invalid";
    }
    if has(&["name", "имя"]) {
        return "auth_name_invalid";
    }
    if has(&["password", "парол"]) {
        return if normalized.contains("128") {
            "auth_password_too_long"
        } else {
            "auth_password_too_short"
        };
    }
    if has(&[
        "credential",
        "unauthorized",
        "authentication",
        "учетн",
        "учётн",
    ]) {
        return "auth_signin_error";
    }

    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_blank_message_falls_back() {
        assert_eq!(message_key(None, "register_error"), "register_error");
        assert_eq!(
            message_key(Some("   "), "send_code_error"),
            "send_code_error"
        );
    }

    #[test]
    fn a_taken_tag_is_recognised_in_both_languages() {
        assert_eq!(
            message_key(Some("Tag already exists"), "register_error"),
            "tag_already_exists"
        );
        assert_eq!(
            message_key(Some("Тег уже занят"), "register_error"),
            "tag_already_exists"
        );
    }

    #[test]
    fn an_expired_code_is_not_confused_with_a_wrong_one() {
        assert_eq!(
            message_key(Some("Code expired"), "register_error"),
            "auth_code_expired"
        );
        assert_eq!(
            message_key(Some("Invalid code"), "register_error"),
            "send_code_invalidcode_error"
        );
    }

    #[test]
    fn a_rate_limit_says_wait_rather_than_naming_a_field() {
        assert_eq!(
            message_key(Some("Too many requests"), "send_code_error"),
            "auth_rate_limited"
        );
        assert_eq!(
            message_key(Some("Подождите 60 seconds"), "send_code_error"),
            "auth_rate_limited"
        );
    }

    #[test]
    fn the_password_length_message_picks_the_side_the_server_complained_about() {
        assert_eq!(
            message_key(Some("Password must be 6..128 characters"), "register_error"),
            "auth_password_too_long"
        );
        assert_eq!(
            message_key(Some("Password too short"), "register_error"),
            "auth_password_too_short"
        );
    }

    #[test]
    fn an_unrecognised_message_never_leaks_the_servers_prose_to_the_user() {
        assert_eq!(
            message_key(
                Some("thread 'main' panicked at src/lib.rs:1"),
                "register_error"
            ),
            "register_error"
        );
    }

    #[test]
    fn a_registered_phone_outranks_the_generic_phone_message() {
        assert_eq!(
            message_key(Some("Phone already registered"), "register_error"),
            "auth_phone_already_registered"
        );
        assert_eq!(
            message_key(Some("Invalid phone number"), "register_error"),
            "auth_phone_invalid"
        );
    }
}
