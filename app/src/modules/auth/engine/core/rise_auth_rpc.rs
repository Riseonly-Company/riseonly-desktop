use rise_engine::{HttpDescriptor, MethodDescriptor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SEND_CODE: MethodDescriptor = MethodDescriptor::mutation("auth", "send_code");
pub const REGISTER: MethodDescriptor = MethodDescriptor::mutation("auth", "register");
pub const LOGIN: MethodDescriptor = MethodDescriptor::mutation("auth", "login");
pub const LOGOUT: MethodDescriptor = MethodDescriptor::mutation("auth", "logout");

// The WS action gate admits no session-less call beyond sign-in; hence the two HTTP verbs.
pub const REFRESH: HttpDescriptor =
    HttpDescriptor::idempotent_mutation("/auth/refresh", "refresh_token");

pub const CHECK_TAG: HttpDescriptor = HttpDescriptor::read("/auth/check-tag");

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

// A proto `string` has no absent form, so `error` is present and `""` on every successful reply.
pub fn refusal_message(payload: &Value) -> Option<String> {
    let message = rise_engine::remote_message(payload.get("error").and_then(Value::as_str));

    let refused =
        payload.get("success").and_then(Value::as_bool) == Some(false) || message.is_some();

    refused.then(|| message.unwrap_or_default().to_owned())
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

    #[test]
    fn a_successful_reply_still_carries_an_empty_error_and_is_not_a_refusal() {
        let granted = json!({
            "success": true,
            "error": "",
            "user": {"id": "773fb9c6-bcc6-4886-a85d-388c7cf933f8"},
            "session_id": "17df871b-945e-4f7d-a910-5d83d2e116f5",
            "access_token": "access",
            "refresh_token": "refresh"
        });

        assert_eq!(
            refusal_message(&granted),
            None,
            "an empty error is the proto's default, not a message"
        );

        let grant: SessionGrant = serde_json::from_value(granted).unwrap();
        assert!(
            grant.resolved().is_some(),
            "the tokens were there the whole time"
        );
    }

    #[test]
    fn a_blank_error_is_never_mistaken_for_a_message() {
        for blank in ["", "   ", "\n"] {
            assert_eq!(
                refusal_message(&json!({"success": true, "error": blank})),
                None
            );
        }
        assert_eq!(refusal_message(&json!({"error": null})), None);
    }

    #[test]
    fn a_message_with_no_success_flag_is_a_refusal() {
        assert_eq!(
            refusal_message(&json!({"error": "Failed to reserve tag"})),
            Some("Failed to reserve tag".to_owned())
        );
    }
}
