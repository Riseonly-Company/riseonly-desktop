use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

// Claims are read unverified: the client has no signing key, and the server re-checks every call.
pub struct AccessTokenUserId;

impl AccessTokenUserId {
    pub fn extract(jwt: &str) -> Option<String> {
        let claims = payload(jwt)?;
        for key in ["id", "sub"] {
            if let Some(value) = claims.get(key).and_then(Value::as_str)
                && !value.is_empty()
            {
                return Some(value.to_owned());
            }
        }
        None
    }

    // auth-service has issued `exp` in both units; a value past year 33658 is milliseconds.
    pub fn expiry_seconds(jwt: &str) -> Option<i64> {
        let claims = payload(jwt)?;
        let raw = claims.get("exp").and_then(number)?;
        Some(if raw > 1_000_000_000_000 {
            raw / 1000
        } else {
            raw
        })
    }

    pub fn is_expired(jwt: &str, now_seconds: i64, leeway_seconds: i64) -> bool {
        if jwt.trim().is_empty() {
            return true;
        }
        let Some(expiry) = Self::expiry_seconds(jwt) else {
            return false;
        };
        expiry - now_seconds < leeway_seconds
    }
}

fn payload(jwt: &str) -> Option<Value> {
    let mut parts = jwt.trim().split('.');
    let _header = parts.next()?;
    let claims = parts.next()?;
    // Rejects a two-segment unsigned token; the discarded segment is the signature.
    parts.next()?;

    let decoded = URL_SAFE_NO_PAD.decode(claims.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn number(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|seconds| seconds as i64))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn token(claims: Value) -> String {
        let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        format!("header.{body}.signature")
    }

    #[test]
    fn the_user_id_comes_from_id_and_falls_back_to_sub() {
        assert_eq!(
            AccessTokenUserId::extract(&token(json!({"id": "u1", "sub": "u2"}))),
            Some("u1".to_owned())
        );
        assert_eq!(
            AccessTokenUserId::extract(&token(json!({"sub": "u2"}))),
            Some("u2".to_owned())
        );
        assert_eq!(
            AccessTokenUserId::extract(&token(json!({"id": ""}))),
            None,
            "an empty claim is not an identity"
        );
    }

    #[test]
    fn a_malformed_token_yields_nothing_rather_than_panicking() {
        assert_eq!(AccessTokenUserId::extract(""), None);
        assert_eq!(AccessTokenUserId::extract("not-a-jwt"), None);
        assert_eq!(AccessTokenUserId::extract("only.two"), None);
        assert_eq!(AccessTokenUserId::extract("a.!!!!.c"), None);
    }

    #[test]
    fn an_expiry_in_milliseconds_is_normalised_to_seconds() {
        assert_eq!(
            AccessTokenUserId::expiry_seconds(&token(json!({"exp": 1_700_000_000_000i64}))),
            Some(1_700_000_000)
        );
        assert_eq!(
            AccessTokenUserId::expiry_seconds(&token(json!({"exp": 1_700_000_000i64}))),
            Some(1_700_000_000)
        );
    }

    #[test]
    fn the_leeway_expires_a_token_before_the_server_would() {
        let jwt = token(json!({"exp": 1_000}));

        assert!(!AccessTokenUserId::is_expired(&jwt, 900, 30));
        assert!(
            AccessTokenUserId::is_expired(&jwt, 980, 30),
            "a token with twenty seconds left must be refreshed, not spent on a request \
             that arrives after it dies"
        );
        assert!(AccessTokenUserId::is_expired(&jwt, 1_001, 0));
    }

    #[test]
    fn an_absent_token_is_expired_and_an_unreadable_one_is_not() {
        assert!(AccessTokenUserId::is_expired("", 0, 0));
        assert!(AccessTokenUserId::is_expired("   ", 0, 0));
        assert!(
            !AccessTokenUserId::is_expired(&token(json!({"id": "u1"})), 0, 0),
            "a token with no exp claim must not lock the account out; the server decides"
        );
    }

    #[test]
    fn an_expiry_sent_as_a_string_or_a_float_still_reads() {
        assert_eq!(
            AccessTokenUserId::expiry_seconds(&token(json!({"exp": "1700000000"}))),
            Some(1_700_000_000)
        );
        assert_eq!(
            AccessTokenUserId::expiry_seconds(&token(json!({"exp": 1_700_000_000.9}))),
            Some(1_700_000_000)
        );
    }
}
