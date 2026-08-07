use rise_engine::{HttpDescriptor, MethodDescriptor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The typed catalogue for auth-service.
///
/// Every method the module can reach is named here once, with its replay policy
/// beside it, so no store ever carries a service or method string. Two of them
/// are HTTP, and the comment on each says why the socket cannot carry it.
pub const SEND_CODE: MethodDescriptor = MethodDescriptor::mutation("auth", "send_code");
pub const REGISTER: MethodDescriptor = MethodDescriptor::mutation("auth", "register");
pub const LOGIN: MethodDescriptor = MethodDescriptor::mutation("auth", "login");
pub const LOGOUT: MethodDescriptor = MethodDescriptor::mutation("auth", "logout");

/// Refreshing happens exactly when the access token has expired, and the
/// socket's action gate admits only send_code, register, login, ping and the QR
/// verbs without a session — so a refresh over the socket is refused by
/// construction. `refresh_token` is the idempotency key by the backend's own
/// contract: auth-service answers a parallel retry with the same pair rather
/// than rotating twice, and `ws-handlers.test.ts` pins that.
pub const REFRESH: HttpDescriptor =
    HttpDescriptor::idempotent_mutation("/auth/refresh", "refresh_token");

/// Tag availability during sign-up, which happens before there is a session.
/// The gateway exposes it only over HTTP (`/api/auth/check-tag` → tag-service
/// `check_tag_exist`); the WS action gate would require the account that is
/// being created.
pub const CHECK_TAG: HttpDescriptor = HttpDescriptor::read("/auth/check-tag");

/// What the device tells the server about itself.
///
/// Registration is the ONE moment `signup_region` and `signup_language` are
/// written — they are frozen afterwards — so the locale hints have to travel
/// with the request rather than be guessed from the connection, where a VPN
/// would freeze the wrong market into the account permanently.
///
/// `ip_address` is deliberately absent. The gateway strips every client-supplied
/// address from an auth body before routing it (`strip_unverifiable_client_
/// network_claims`) and stamps the peer it resolved itself, so sending one is at
/// best ignored and at worst a forged value in somebody's session list.
#[derive(Clone, Debug, Serialize)]
pub struct DeviceEnvelope {
    pub device_info: String,
    pub device_type: String,
    pub platform: String,
    pub user_agent: String,
    pub browser: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_token: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SendCodeRequest {
    pub phone_number: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SendCodeResponse {
    pub success: Option<bool>,
    pub message: Option<String>,
    pub error: Option<String>,
    #[serde(alias = "botUsername")]
    pub bot_username: Option<String>,
}

impl SendCodeResponse {
    /// The gateway answers `success` from auth-service and always attaches a
    /// message, so a truthful "sent" is `success == true`. The reference also
    /// accepts the Russian message text, which was how it read the answer before
    /// the flag existed; keeping that is bug-compatibility, not belt and braces.
    pub fn code_sent(&self) -> bool {
        self.success == Some(true)
            || self
                .message
                .as_deref()
                .is_some_and(|message| message.contains("отправлен"))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LoginRequest {
    pub phone_number: String,
    pub password: String,
    #[serde(flatten)]
    pub device: DeviceEnvelope,
}

#[derive(Clone, Debug, Serialize)]
pub struct RegisterRequest {
    pub phone_number: String,
    pub name: String,
    pub tag: String,
    pub code: String,
    pub password: String,
    #[serde(flatten)]
    pub device: DeviceEnvelope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_region: Option<String>,
    pub device_languages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

/// Login and register answer with the same shape.
///
/// Snake case, because the gateway builds this JSON itself
/// (`grpc_routes/auth.rs`) rather than forwarding a proto. The camelCase aliases
/// exist because the reference's decoder converts from snake case and its DTOs
/// therefore read camel — a build pointed at an older gateway must not silently
/// decode every field as `None` and look like a login with no tokens.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SessionGrant {
    pub success: Option<bool>,
    pub error: Option<String>,
    pub user: Option<AuthUserDto>,
    #[serde(alias = "accessToken")]
    pub access_token: Option<String>,
    #[serde(alias = "refreshToken")]
    pub refresh_token: Option<String>,
    #[serde(alias = "sessionId")]
    pub session_id: Option<String>,
}

impl SessionGrant {
    /// A grant is only usable when all three parts arrived non-empty.
    ///
    /// Half a grant is worse than none: an access token with no refresh token
    /// signs the user in for fifteen minutes and then drops them at the sign-in
    /// screen with no way to recover.
    pub fn resolved(&self) -> Option<GrantedTokens> {
        let access = non_empty(self.access_token.as_deref())?;
        let refresh = non_empty(self.refresh_token.as_deref())?;
        let session = non_empty(self.session_id.as_deref())?;

        Some(GrantedTokens {
            access,
            refresh,
            session,
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GrantedTokens {
    pub access: String,
    pub refresh: String,
    pub session: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// The refresh answer, in both shapes the deployed backends produce.
///
/// The HTTP route returns the gateway's flat object; older builds nested it
/// under `data`. The reference decodes both, and a client that dropped the
/// nested form would sign the user out on a backend that still uses it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RefreshResponse {
    #[serde(alias = "accessToken")]
    pub access_token: Option<String>,
    #[serde(alias = "refreshToken")]
    pub refresh_token: Option<String>,
    #[serde(alias = "sessionId")]
    pub session_id: Option<String>,
    pub error: Option<String>,
    pub data: Option<RefreshPayload>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RefreshPayload {
    #[serde(alias = "accessToken")]
    pub access_token: Option<String>,
    #[serde(alias = "refreshToken")]
    pub refresh_token: Option<String>,
    #[serde(alias = "sessionId")]
    pub session_id: Option<String>,
}

impl RefreshResponse {
    /// The session id may legitimately be absent: a rotation that keeps the same
    /// session answers with only the token pair, and the caller keeps the one it
    /// already had.
    pub fn resolved(&self) -> Option<(String, String, Option<String>)> {
        let nested = self.data.as_ref();
        let access = non_empty(self.access_token.as_deref())
            .or_else(|| nested.and_then(|d| non_empty(d.access_token.as_deref())))?;
        let refresh = non_empty(self.refresh_token.as_deref())
            .or_else(|| nested.and_then(|d| non_empty(d.refresh_token.as_deref())))?;
        let session = non_empty(self.session_id.as_deref())
            .or_else(|| nested.and_then(|d| non_empty(d.session_id.as_deref())));

        Some((access, refresh, session))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LogoutRequest {
    pub user_id: String,
    pub session_id: String,
    pub refresh_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_token: Option<String>,
    pub platform: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckTagRequest {
    pub tag: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct CheckTagResponse {
    pub success: Option<bool>,
    pub available: Option<bool>,
    pub exists: Option<bool>,
    #[serde(alias = "normalizedTag")]
    pub normalized_tag: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AuthUserMoreDto {
    pub id: Option<String>,
    pub description: Option<String>,
    pub hb: Option<String>,
    pub streak: Option<i64>,
    #[serde(alias = "pLang", default)]
    pub p_lang: Vec<String>,
    pub subscribers: Option<i64>,
    pub friends: Option<i64>,
    pub status: Option<String>,
    #[serde(alias = "postsCount")]
    pub posts_count: Option<i64>,
    pub level: Option<i64>,
    pub logo: Option<String>,
    pub banner: Option<String>,
    pub who: Option<String>,
    pub rating: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AuthUserDto {
    pub id: String,
    pub name: Option<String>,
    pub tag: Option<String>,
    pub phone: Option<String>,
    #[serde(alias = "isPremium")]
    pub is_premium: Option<bool>,
    #[serde(alias = "isOfficial")]
    pub is_official: Option<bool>,
    #[serde(alias = "isBlocked")]
    pub is_blocked: Option<bool>,
    #[serde(alias = "userChatId")]
    pub user_chat_id: Option<String>,
    pub gender: Option<String>,
    pub role: Option<String>,
    #[serde(alias = "avatarUrl")]
    pub avatar_url: Option<String>,
    #[serde(alias = "coverImageUrl")]
    pub cover_image_url: Option<String>,
    #[serde(alias = "subscriptionStatus")]
    pub subscription_status: Option<String>,
    pub more: Option<AuthUserMoreDto>,
}

/// The gateway omits `status` on success and puts the payload under `data` for
/// some services and `result` for others; the frame decoder already normalises
/// both. What is left here is the one thing it cannot know: a 2xx frame whose
/// body says `success: false` is still a failure.
pub fn refusal_message(payload: &Value) -> Option<String> {
    if payload.get("success").and_then(Value::as_bool) == Some(false)
        || payload.get("error").is_some()
    {
        return payload
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| Some(String::new()));
    }
    None
}

fn non_empty(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_engine::ReplayPolicy;
    use serde_json::json;

    #[test]
    fn no_auth_mutation_may_replay_itself() {
        for descriptor in [SEND_CODE, REGISTER, LOGIN, LOGOUT] {
            assert_eq!(
                descriptor.replay,
                ReplayPolicy::Never,
                "{} would mint a second session on a retry",
                descriptor.qualified_name()
            );
        }
    }

    #[test]
    fn refresh_replays_only_under_the_key_the_backend_promised() {
        assert_eq!(
            REFRESH.replay,
            ReplayPolicy::IdempotencyKey {
                field: "refresh_token"
            }
        );
    }

    #[test]
    fn a_login_body_flattens_the_device_and_carries_no_client_address() {
        let body = serde_json::to_value(LoginRequest {
            phone_number: "70000000000".into(),
            password: "secret".into(),
            device: DeviceEnvelope {
                device_info: "MacBook".into(),
                device_type: "desktop".into(),
                platform: "macos".into(),
                user_agent: "Riseonly/0.1".into(),
                browser: false,
                device_token: None,
            },
        })
        .unwrap();

        assert_eq!(body["phone_number"], "70000000000");
        assert_eq!(body["platform"], "macos");
        assert_eq!(
            body["device_info"], "MacBook",
            "a nested device object would not match the gateway's flat reader"
        );
        assert!(
            body.get("ip_address").is_none(),
            "the gateway strips a client-supplied address; sending one is a forged session row"
        );
        assert!(body.get("device_token").is_none());
    }

    #[test]
    fn a_grant_decodes_the_gateways_snake_case() {
        let grant: SessionGrant = serde_json::from_value(json!({
            "success": true,
            "user": {"id": "u1", "name": "A", "tag": "a"},
            "access_token": "at",
            "refresh_token": "rt",
            "session_id": "sid"
        }))
        .unwrap();

        let tokens = grant.resolved().expect("a complete grant");
        assert_eq!(tokens.access, "at");
        assert_eq!(tokens.refresh, "rt");
        assert_eq!(tokens.session, "sid");
        assert_eq!(grant.user.unwrap().id, "u1");
    }

    #[test]
    fn a_grant_also_decodes_the_camel_case_an_older_gateway_sends() {
        let grant: SessionGrant = serde_json::from_value(json!({
            "accessToken": "at",
            "refreshToken": "rt",
            "sessionId": "sid"
        }))
        .unwrap();
        assert!(grant.resolved().is_some());
    }

    #[test]
    fn half_a_grant_is_refused_rather_than_signed_in() {
        let missing_refresh: SessionGrant = serde_json::from_value(json!({
            "access_token": "at",
            "session_id": "sid"
        }))
        .unwrap();
        assert_eq!(
            missing_refresh.resolved(),
            None,
            "an access token with no refresh token signs the user out fifteen minutes later"
        );

        let blank: SessionGrant = serde_json::from_value(json!({
            "access_token": "  ",
            "refresh_token": "rt",
            "session_id": "sid"
        }))
        .unwrap();
        assert_eq!(blank.resolved(), None);
    }

    #[test]
    fn a_refresh_reads_both_the_flat_and_the_nested_shape() {
        let flat: RefreshResponse = serde_json::from_value(json!({
            "access_token": "a", "refresh_token": "r", "session_id": "s"
        }))
        .unwrap();
        assert_eq!(
            flat.resolved(),
            Some(("a".into(), "r".into(), Some("s".into())))
        );

        let nested: RefreshResponse = serde_json::from_value(json!({
            "data": {"accessToken": "a", "refreshToken": "r"}
        }))
        .unwrap();
        assert_eq!(nested.resolved(), Some(("a".into(), "r".into(), None)));
    }

    #[test]
    fn a_rotation_that_keeps_its_session_is_not_a_failed_refresh() {
        let response: RefreshResponse =
            serde_json::from_value(json!({"access_token": "a", "refresh_token": "r"})).unwrap();
        let (_, _, session) = response
            .resolved()
            .expect("a pair with no session is valid");
        assert_eq!(session, None);
    }

    #[test]
    fn send_code_reads_the_flag_and_the_legacy_message() {
        let flagged: SendCodeResponse =
            serde_json::from_value(json!({"success": true, "bot_username": "riseonly_bot"}))
                .unwrap();
        assert!(flagged.code_sent());
        assert_eq!(flagged.bot_username.as_deref(), Some("riseonly_bot"));

        let legacy: SendCodeResponse =
            serde_json::from_value(json!({"message": "Код отправлен"})).unwrap();
        assert!(legacy.code_sent());

        let refused: SendCodeResponse =
            serde_json::from_value(json!({"success": false, "error": "Too many requests"}))
                .unwrap();
        assert!(!refused.code_sent());
    }

    #[test]
    fn a_two_hundred_that_says_success_false_is_still_a_refusal() {
        assert_eq!(
            refusal_message(&json!({"success": false, "error": "Tag already exists"})),
            Some("Tag already exists".to_owned())
        );
        assert_eq!(refusal_message(&json!({"success": true, "user": {}})), None);
        assert_eq!(
            refusal_message(&json!({"success": false})),
            Some(String::new()),
            "a refusal with no message is still a refusal"
        );
    }
}
