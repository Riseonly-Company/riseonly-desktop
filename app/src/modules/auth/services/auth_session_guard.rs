use super::access_token_user_id::AccessTokenUserId;

/// What the app does when the server says the credential is no longer good.
///
/// A pure decision so the whole matrix is testable: the recovery itself belongs
/// to the repository, which is the only thing that may touch tokens.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthRecovery {
    /// Not an auth failure at all — the caller handles its own error.
    NotAuthRelated,
    /// Rotate the pair and retry once the new access token is in hand.
    Refresh,
    /// The refresh token is gone or expired. Nothing local can recover this.
    SignOut,
}

/// Whether the server's refusal means the credential rather than the request.
///
/// Matched on both code and message because the gateway is not consistent: a
/// rejected JWT arrives as `AUTH_ERROR` from one path and as a plain message
/// from another, and treating the second as an ordinary failure leaves the user
/// staring at a screen that silently refuses to load.
pub fn is_auth_error(code: Option<&str>, message: Option<&str>) -> bool {
    let code = code.unwrap_or_default().to_uppercase();
    if ["AUTH_ERROR", "UNAUTHORIZED", "UNAUTHENTICATED"]
        .iter()
        .any(|needle| code.contains(needle))
        || code == "401"
    {
        return true;
    }

    let message = message.unwrap_or_default().to_lowercase();
    [
        "authentication token",
        "invalid token",
        "token expired",
        "expired token",
        "invalid or expired",
        "auth_error",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

pub fn recovery_for(
    code: Option<&str>,
    message: Option<&str>,
    refresh_token: &str,
    now_seconds: i64,
) -> AuthRecovery {
    if !is_auth_error(code, message) {
        return AuthRecovery::NotAuthRelated;
    }

    // No leeway on the refresh token, unlike the access token: refreshing early
    // is cheap and correct, but signing somebody out thirty seconds early is a
    // lost session for nothing.
    if AccessTokenUserId::is_expired(refresh_token, now_seconds, 0) {
        return AuthRecovery::SignOut;
    }

    AuthRecovery::Refresh
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn token(expiry: i64) -> String {
        let body = URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&serde_json::json!({"exp": expiry})).unwrap());
        format!("header.{body}.signature")
    }

    #[test]
    fn every_shape_the_gateway_refuses_a_credential_in_is_recognised() {
        assert!(is_auth_error(Some("AUTH_ERROR"), None));
        assert!(is_auth_error(Some("401"), None));
        assert!(is_auth_error(Some("unauthorized"), None));
        assert!(is_auth_error(None, Some("Invalid or expired token")));
        assert!(is_auth_error(None, Some("Missing authentication token")));
    }

    #[test]
    fn an_ordinary_failure_is_not_mistaken_for_an_expired_session() {
        assert!(!is_auth_error(Some("NOT_FOUND"), Some("Chat not found")));
        assert!(!is_auth_error(None, None));
        assert!(
            !is_auth_error(Some("RATE_LIMITED"), Some("Too many requests")),
            "signing the user out on a rate limit would turn a wait into a logout"
        );
    }

    #[test]
    fn a_live_refresh_token_means_rotate_rather_than_sign_out() {
        assert_eq!(
            recovery_for(Some("AUTH_ERROR"), None, &token(10_000), 1_000),
            AuthRecovery::Refresh
        );
    }

    #[test]
    fn a_dead_or_missing_refresh_token_is_the_end_of_the_session() {
        assert_eq!(
            recovery_for(Some("AUTH_ERROR"), None, &token(500), 1_000),
            AuthRecovery::SignOut
        );
        assert_eq!(
            recovery_for(Some("AUTH_ERROR"), None, "", 1_000),
            AuthRecovery::SignOut
        );
    }

    #[test]
    fn a_non_auth_failure_never_touches_the_credential() {
        assert_eq!(
            recovery_for(Some("INTERNAL"), Some("boom"), "", 0),
            AuthRecovery::NotAuthRelated,
            "an empty refresh token must not turn a server error into a logout"
        );
    }
}
