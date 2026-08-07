use rise_engine::MethodDescriptor;
use serde::{Deserialize, Serialize};

/// session-service, through the gateway's `session` service.
///
/// `delete_user_sessions_advanced` is a plain mutation with no replay: ending a
/// session is not idempotent from the client's side — a retry after an
/// inconclusive failure could end a session the user has since started from
/// another device, and the backend contract promises nothing that would make it
/// safe.
pub const GET_USER_SESSIONS: MethodDescriptor =
    MethodDescriptor::read("session", "get_user_sessions");
pub const DELETE_USER_SESSIONS: MethodDescriptor =
    MethodDescriptor::mutation("session", "delete_user_sessions_advanced");

#[derive(Clone, Debug, Serialize)]
pub struct GetUserSessionsRequest {
    pub user_id: String,
    pub current_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_id: Option<String>,
    pub limit: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeleteUserSessionsRequest {
    pub user_id: String,
    pub session_id_to_terminate: String,
    pub is_all: bool,
    pub current_session_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct UserSessionDto {
    pub id: String,
    #[serde(alias = "userId")]
    pub user_id: Option<String>,
    #[serde(alias = "deviceInfo")]
    pub device_info: Option<String>,
    #[serde(alias = "ipAddress")]
    pub ip_address: Option<String>,
    pub location: Option<String>,
    #[serde(alias = "createdAt")]
    pub created_at: Option<String>,
    #[serde(alias = "lastAccessedAt")]
    pub last_accessed_at: Option<String>,
    #[serde(alias = "isCurrent")]
    pub is_current: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct GetUserSessionsResponse {
    pub success: Option<bool>,
    pub error: Option<String>,
    #[serde(default)]
    pub sessions: Vec<UserSessionDto>,
    #[serde(alias = "relativeId")]
    pub relative_id: Option<String>,
    #[serde(alias = "isHaveMore", alias = "is_have_more")]
    pub is_have_more: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DeleteUserSessionsResponse {
    pub success: Option<bool>,
    pub error: Option<String>,
    #[serde(alias = "remainingSessions", default)]
    pub remaining_sessions: Option<Vec<UserSessionDto>>,
    /// The server ending the session this client is running on. The only correct
    /// answer is a local sign-out: every subsequent request would be refused.
    #[serde(alias = "shouldLogout")]
    pub should_logout: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_engine::ReplayPolicy;
    use serde_json::json;

    #[test]
    fn ending_a_session_never_replays_itself() {
        assert_eq!(DELETE_USER_SESSIONS.replay, ReplayPolicy::Never);
        assert_eq!(GET_USER_SESSIONS.replay, ReplayPolicy::ReadOnly);
    }

    #[test]
    fn a_page_decodes_both_key_styles_the_gateway_has_used() {
        let snake: GetUserSessionsResponse = serde_json::from_value(json!({
            "sessions": [{"id": "s1", "device_info": "iPhone", "is_current": true}],
            "relative_id": "s1",
            "is_have_more": true
        }))
        .unwrap();
        assert_eq!(snake.sessions.len(), 1);
        assert_eq!(snake.sessions[0].device_info.as_deref(), Some("iPhone"));
        assert_eq!(snake.is_have_more, Some(true));
        assert_eq!(snake.relative_id.as_deref(), Some("s1"));

        let camel: GetUserSessionsResponse = serde_json::from_value(json!({
            "sessions": [{"id": "s1", "deviceInfo": "Mac", "isCurrent": false}],
            "relativeId": "s1",
            "isHaveMore": false
        }))
        .unwrap();
        assert_eq!(camel.sessions[0].device_info.as_deref(), Some("Mac"));
        assert_eq!(camel.is_have_more, Some(false));
    }

    #[test]
    fn an_answer_with_no_sessions_key_is_an_empty_page_rather_than_a_decode_error() {
        let empty: GetUserSessionsResponse = serde_json::from_value(json!({"success": true}))
            .expect("an absent list is a valid empty page");
        assert!(empty.sessions.is_empty());
        assert_eq!(empty.is_have_more, None);
    }

    #[test]
    fn a_delete_can_report_that_this_client_just_ended_its_own_session() {
        let response: DeleteUserSessionsResponse =
            serde_json::from_value(json!({"success": true, "shouldLogout": true})).unwrap();
        assert_eq!(response.should_logout, Some(true));
    }

    #[test]
    fn a_get_request_omits_the_cursor_on_the_first_page() {
        let body = serde_json::to_value(GetUserSessionsRequest {
            user_id: "u1".into(),
            current_session_id: "s1".into(),
            relative_id: None,
            limit: 20,
        })
        .unwrap();

        assert!(
            body.get("relative_id").is_none(),
            "a null cursor would read as a page anchored at nothing"
        );
        assert_eq!(body["limit"], 20);
    }
}
