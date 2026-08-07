use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures::future::BoxFuture;
use rise_engine::{HttpDescriptor, MethodDescriptor, RemoteError};
use rise_platform::InMemorySecureStore;
use serde_json::json;

use super::*;

/// A scripted gateway.
///
/// Every rule the repository owns — which failure rotates a token, which one
/// ends the session, what two concurrent refreshes do — is unreachable against a
/// real server, so the transport is the seam the tests drive.
#[derive(Default)]
struct ScriptedTransport {
    replies: Mutex<VecDeque<(String, Result<Value, WireError>)>>,
    calls: Mutex<Vec<String>>,
    credentials: Mutex<Vec<SocketCredential>>,
}

impl ScriptedTransport {
    fn answering(script: Vec<(&str, Result<Value, WireError>)>) -> Arc<Self> {
        let transport = Arc::new(Self::default());
        *transport.replies.lock().unwrap() = script
            .into_iter()
            .map(|(name, reply)| (name.to_owned(), reply))
            .collect();
        transport
    }

    fn next(&self, name: &str) -> Result<Value, WireError> {
        self.calls.lock().unwrap().push(name.to_owned());

        let mut replies = self.replies.lock().unwrap();
        match replies.front() {
            Some((expected, _)) if expected == name => replies.pop_front().unwrap().1,
            _ => Ok(json!({})),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn credentials(&self) -> Vec<SocketCredential> {
        self.credentials.lock().unwrap().clone()
    }
}

impl AuthTransport for ScriptedTransport {
    fn call(
        &self,
        descriptor: &'static MethodDescriptor,
        _body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        let reply = self.next(&descriptor.qualified_name());
        Box::pin(async move { reply })
    }

    fn http(
        &self,
        descriptor: &'static HttpDescriptor,
        _body: Value,
        _authorization: Option<String>,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        let reply = self.next(descriptor.path);
        Box::pin(async move { reply })
    }

    fn authenticate(&self, credential: SocketCredential) {
        self.credentials.lock().unwrap().push(credential);
    }
}

const NOW_MS: i64 = 1_700_000_000_000;

fn jwt(user_id: &str, expiry_seconds: i64) -> String {
    let body = URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&json!({"id": user_id, "exp": expiry_seconds})).unwrap());
    format!("header.{body}.signature")
}

fn live_token(user_id: &str) -> String {
    jwt(user_id, NOW_MS / 1_000 + 3_600)
}

fn dead_token(user_id: &str) -> String {
    jwt(user_id, NOW_MS / 1_000 - 10)
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rise-auth-repo-{name}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Fixture {
    state: AuthState,
    transport: Arc<ScriptedTransport>,
    credentials: Arc<AuthCredentialStore>,
    secrets: Arc<InMemorySecureStore>,
    directory: PathBuf,
    _inbox: mpsc::Receiver<AuthCommand>,
}

impl Fixture {
    fn new(name: &str, transport: Arc<ScriptedTransport>) -> Self {
        let directory = temp_dir(name);
        let credentials = Arc::new(AuthCredentialStore::new(&directory));
        let secrets = Arc::new(InMemorySecureStore::new());
        let (commands, inbox) = mpsc::channel(64);
        let (presentation, _snapshot) = RiseAuthPresentation::new();

        let state = AuthState::new(
            AuthRepositoryConfig {
                transport: Arc::clone(&transport) as Arc<dyn AuthTransport>,
                credentials: Arc::clone(&credentials),
                secrets: Arc::clone(&secrets) as Arc<dyn SecureStore>,
                host: HostOs::MacOs,
                locale: DeviceLocale::parse(&["ru-KZ"]),
                now_ms: Arc::new(|| NOW_MS),
            },
            presentation,
            commands,
        );

        Self {
            state,
            transport,
            credentials,
            secrets,
            directory,
            _inbox: inbox,
        }
    }

    fn seed_account(&self, user_id: &str, tokens: StoredTokens) {
        self.credentials
            .save(
                PersistedAccount {
                    user: AuthUser {
                        id: user_id.to_owned(),
                        name: user_id.to_owned(),
                        ..AuthUser::default()
                    },
                    last_used_ms: 1,
                },
                &tokens,
                self.secrets.as_ref(),
            )
            .unwrap();
    }

    fn snapshot(&self) -> Arc<AuthSnapshot> {
        self.state.presentation.current()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).ok();
    }
}

fn tokens(access: &str, refresh: &str) -> StoredTokens {
    StoredTokens {
        access: access.to_owned(),
        refresh: refresh.to_owned(),
        session: "session".to_owned(),
    }
}

fn grant(user_id: &str) -> Value {
    json!({
        "success": true,
        "user": {"id": user_id, "name": "Name", "tag": "tag"},
        "access_token": live_token(user_id),
        "refresh_token": live_token(user_id),
        "session_id": "session"
    })
}

#[tokio::test]
async fn a_successful_sign_in_persists_the_account_and_re_handshakes_the_socket() {
    let transport = ScriptedTransport::answering(vec![("auth.login", Ok(grant("u1")))]);
    let mut fixture = Fixture::new("signin", transport);

    fixture
        .state
        .sign_in("70000000000".into(), "password".into())
        .await;

    let snapshot = fixture.snapshot();
    assert!(snapshot.is_authenticated);
    assert_eq!(snapshot.active.as_ref().unwrap().id, "u1");
    assert_eq!(snapshot.accounts.len(), 1);
    assert!(!snapshot.flow.is_busy);
    assert_eq!(snapshot.flow.error_key, None);

    assert_eq!(
        fixture.credentials.active_account_id().as_deref(),
        Some("u1")
    );
    assert!(
        matches!(
            fixture.transport.credentials().last(),
            Some(SocketCredential::Bearer(_))
        ),
        "the socket must re-handshake, because the gateway reads the user once at upgrade"
    );
}

#[tokio::test]
async fn a_refused_sign_in_shows_the_servers_own_reason() {
    let transport = ScriptedTransport::answering(vec![(
        "auth.login",
        Err(WireError::Remote(RemoteError {
            code: Some("AUTH_ERROR".into()),
            message: "Invalid credentials".into(),
        })),
    )]);
    let mut fixture = Fixture::new("signin-refused", transport);

    fixture
        .state
        .sign_in("70000000000".into(), "wrong".into())
        .await;

    let snapshot = fixture.snapshot();
    assert!(!snapshot.is_authenticated);
    assert_eq!(snapshot.flow.error_key, Some("auth_signin_error"));
    assert!(!snapshot.flow.is_busy, "a refusal must clear the spinner");
}

#[tokio::test]
async fn a_two_hundred_that_says_success_false_is_not_a_sign_in() {
    let transport = ScriptedTransport::answering(vec![(
        "auth.login",
        Ok(json!({"success": false, "error": "Phone already registered"})),
    )]);
    let mut fixture = Fixture::new("signin-soft-failure", transport);

    fixture.state.sign_in("7000".into(), "p".into()).await;

    let snapshot = fixture.snapshot();
    assert!(!snapshot.is_authenticated);
    assert_eq!(
        snapshot.flow.error_key,
        Some("auth_phone_already_registered")
    );
}

#[tokio::test]
async fn a_grant_with_no_refresh_token_is_refused_rather_than_signed_in() {
    let transport = ScriptedTransport::answering(vec![(
        "auth.login",
        Ok(json!({
            "success": true,
            "user": {"id": "u1"},
            "access_token": live_token("u1"),
            "session_id": "session"
        })),
    )]);
    let mut fixture = Fixture::new("signin-half", transport);

    fixture.state.sign_in("7000".into(), "p".into()).await;

    assert!(
        !fixture.snapshot().is_authenticated,
        "signing in with no way to refresh drops the user at the login screen in fifteen minutes"
    );
    assert!(fixture.snapshot().flow.error_key.is_some());
}

#[tokio::test]
async fn send_code_moves_the_flow_to_the_telegram_hop() {
    let transport = ScriptedTransport::answering(vec![(
        "auth.send_code",
        Ok(json!({"success": true, "bot_username": "riseonly_bot"})),
    )]);
    let mut fixture = Fixture::new("send-code", transport);

    fixture.state.send_code("+7 (999) 123-45-67".into()).await;

    let flow = fixture.snapshot().flow.clone();
    assert!(flow.telegram_redirect_active);
    assert!(!flow.code_entry_active);
    assert_eq!(flow.bot_username.as_deref(), Some("riseonly_bot"));
    assert_eq!(
        flow.phone_payload.as_deref(),
        Some("79991234567"),
        "the bot deep link carries digits only"
    );
}

#[tokio::test]
async fn a_rate_limited_send_code_never_opens_the_telegram_step() {
    let transport = ScriptedTransport::answering(vec![(
        "auth.send_code",
        Ok(json!({"success": false, "error": "Too many requests"})),
    )]);
    let mut fixture = Fixture::new("send-code-limited", transport);

    fixture.state.send_code("79991234567".into()).await;

    let flow = fixture.snapshot().flow.clone();
    assert!(!flow.telegram_redirect_active);
    assert_eq!(flow.error_key, Some("auth_rate_limited"));
}

#[tokio::test]
async fn registration_signs_the_new_account_in() {
    let transport = ScriptedTransport::answering(vec![("auth.register", Ok(grant("new")))]);
    let mut fixture = Fixture::new("register", transport);

    fixture
        .state
        .register(
            " Name ".into(),
            "  NewTag ".into(),
            "79991234567".into(),
            "password".into(),
            "1234".into(),
        )
        .await;

    assert!(fixture.snapshot().is_authenticated);
    assert_eq!(fixture.snapshot().active.as_ref().unwrap().id, "new");
    assert!(
        !fixture.snapshot().flow.telegram_redirect_active
            && !fixture.snapshot().flow.code_entry_active,
        "a completed registration leaves no step of its own flow open"
    );
}

#[tokio::test]
async fn a_restore_with_a_live_token_signs_straight_back_in_without_a_round_trip() {
    let transport = ScriptedTransport::answering(vec![]);
    let mut fixture = Fixture::new("restore-live", transport);
    fixture.seed_account("u1", tokens(&live_token("u1"), &live_token("u1")));

    fixture.state.restore().await;

    let snapshot = fixture.snapshot();
    assert!(snapshot.is_authenticated);
    assert!(!snapshot.is_restoring);
    assert!(
        fixture.transport.calls().is_empty(),
        "a valid session must open the app from cache, not from the network"
    );
}

#[tokio::test]
async fn a_restore_with_a_stale_access_token_refreshes_before_it_finishes() {
    let transport = ScriptedTransport::answering(vec![(
        "/auth/refresh",
        Ok(json!({
            "access_token": live_token("u1"),
            "refresh_token": live_token("u1"),
            "session_id": "session2"
        })),
    )]);
    let mut fixture = Fixture::new("restore-stale", transport);
    fixture.seed_account("u1", tokens(&dead_token("u1"), &live_token("u1")));

    fixture.state.restore().await;

    assert!(fixture.snapshot().is_authenticated);
    assert_eq!(fixture.transport.calls(), vec!["/auth/refresh"]);
    assert_eq!(
        fixture
            .credentials
            .tokens("u1", fixture.secrets.as_ref())
            .unwrap()
            .unwrap()
            .session,
        "session2"
    );
}

#[tokio::test]
async fn a_restore_whose_refresh_token_has_expired_drops_the_account() {
    let transport = ScriptedTransport::answering(vec![]);
    let mut fixture = Fixture::new("restore-dead", transport);
    fixture.seed_account("u1", tokens(&dead_token("u1"), &dead_token("u1")));

    fixture.state.restore().await;

    let snapshot = fixture.snapshot();
    assert!(!snapshot.is_authenticated);
    assert!(!snapshot.is_restoring);
    assert!(
        fixture.credentials.accounts().is_empty(),
        "a session that can never recover must not sit on screen failing every request"
    );
    assert!(
        fixture.transport.calls().is_empty(),
        "there is nothing to ask the server about"
    );
}

#[tokio::test]
async fn a_refused_refresh_ends_the_session_but_a_dropped_socket_does_not() {
    let refused = ScriptedTransport::answering(vec![(
        "/auth/refresh",
        Err(WireError::Remote(RemoteError {
            code: Some("401".into()),
            message: "Refresh token reused".into(),
        })),
    )]);
    let mut fixture = Fixture::new("refresh-refused", refused);
    fixture.seed_account("u1", tokens(&dead_token("u1"), &live_token("u1")));
    fixture.state.restore().await;

    assert!(!fixture.snapshot().is_authenticated);
    assert!(fixture.credentials.accounts().is_empty());

    let offline =
        ScriptedTransport::answering(vec![("/auth/refresh", Err(WireError::NotConnected))]);
    let mut fixture = Fixture::new("refresh-offline", offline);
    fixture.seed_account("u1", tokens(&dead_token("u1"), &live_token("u1")));
    fixture.state.restore().await;

    assert!(
        !fixture.credentials.accounts().is_empty(),
        "losing the network must not throw the account and its cache away"
    );
}

#[tokio::test]
async fn the_loser_of_two_concurrent_refreshes_never_reinstates_an_older_pair() {
    let transport = ScriptedTransport::answering(vec![(
        "/auth/refresh",
        Ok(json!({
            "access_token": live_token("u1"),
            "refresh_token": live_token("u1")
        })),
    )]);
    let mut fixture = Fixture::new("refresh-race", transport);

    let original = tokens(&dead_token("u1"), &live_token("u1"));
    fixture.seed_account("u1", original.clone());
    fixture.state.accounts = fixture.credentials.accounts();
    fixture.state.active = Some("u1".into());
    fixture.state.tokens = Some(original.clone());

    // Somebody else already rotated while this refresh was in flight.
    let winner = tokens("winner-access", "winner-refresh");
    fixture
        .credentials
        .save(
            PersistedAccount {
                user: AuthUser {
                    id: "u1".into(),
                    ..AuthUser::default()
                },
                last_used_ms: 1,
            },
            &winner,
            fixture.secrets.as_ref(),
        )
        .unwrap();

    fixture.state.refresh_tokens().await;

    assert_eq!(
        fixture
            .credentials
            .tokens("u1", fixture.secrets.as_ref())
            .unwrap(),
        Some(winner),
        "the loser must abandon its answer rather than overwrite a newer pair"
    );
    assert!(
        fixture
            .state
            .trace
            .recent()
            .contains(&TraceEvent::RefreshRaceLost)
    );
}

#[tokio::test]
async fn signing_out_clears_the_account_even_when_the_server_never_answers() {
    let transport =
        ScriptedTransport::answering(vec![("auth.logout", Err(WireError::NotConnected))]);
    let mut fixture = Fixture::new("signout-offline", transport);
    fixture.seed_account("u1", tokens(&live_token("u1"), &live_token("u1")));
    fixture.state.restore().await;
    assert!(fixture.snapshot().is_authenticated);

    fixture.state.sign_out(ResetReason::UserLogout).await;

    assert!(!fixture.snapshot().is_authenticated);
    assert!(fixture.credentials.accounts().is_empty());
    assert_eq!(
        fixture.transport.credentials().last(),
        Some(&SocketCredential::Anonymous),
        "the socket must stop being authenticated as a session that was just revoked"
    );
}

#[tokio::test]
async fn signing_out_of_one_of_two_accounts_promotes_the_other() {
    let transport = ScriptedTransport::answering(vec![("auth.logout", Ok(json!({})))]);
    let mut fixture = Fixture::new("signout-multi", transport);
    fixture.seed_account("u1", tokens(&live_token("u1"), &live_token("u1")));
    fixture.seed_account("u2", tokens(&live_token("u2"), &live_token("u2")));
    fixture.state.restore().await;
    assert_eq!(fixture.snapshot().active_account_id.as_deref(), Some("u2"));

    fixture.state.sign_out(ResetReason::UserLogout).await;

    let snapshot = fixture.snapshot();
    assert!(snapshot.is_authenticated);
    assert_eq!(snapshot.active_account_id.as_deref(), Some("u1"));
    assert_eq!(snapshot.accounts.len(), 1);
}

#[tokio::test]
async fn an_auth_failure_from_any_request_rotates_the_token_when_it_still_can() {
    let transport = ScriptedTransport::answering(vec![(
        "/auth/refresh",
        Ok(json!({
            "access_token": live_token("u1"),
            "refresh_token": live_token("u1")
        })),
    )]);
    let mut fixture = Fixture::new("note-failure-refresh", transport);
    fixture.seed_account("u1", tokens(&live_token("u1"), &live_token("u1")));
    fixture.state.restore().await;

    fixture
        .state
        .note_remote_failure(Some("AUTH_ERROR"), Some("Invalid token"))
        .await;

    assert!(fixture.snapshot().is_authenticated);
    assert_eq!(fixture.transport.calls(), vec!["/auth/refresh"]);
}

#[tokio::test]
async fn an_auth_failure_with_a_dead_refresh_token_signs_out_without_asking() {
    let transport = ScriptedTransport::answering(vec![]);
    let mut fixture = Fixture::new("note-failure-signout", transport);
    fixture.seed_account("u1", tokens(&live_token("u1"), &dead_token("u1")));
    fixture.state.accounts = fixture.credentials.accounts();
    fixture.state.active = Some("u1".into());
    fixture.state.tokens = fixture
        .credentials
        .tokens("u1", fixture.secrets.as_ref())
        .unwrap();

    fixture
        .state
        .note_remote_failure(Some("AUTH_ERROR"), None)
        .await;

    assert!(!fixture.snapshot().is_authenticated);
    assert!(fixture.transport.calls().is_empty());
}

#[tokio::test]
async fn an_ordinary_failure_never_touches_the_session() {
    let transport = ScriptedTransport::answering(vec![]);
    let mut fixture = Fixture::new("note-failure-ignored", transport);
    fixture.seed_account("u1", tokens(&live_token("u1"), &live_token("u1")));
    fixture.state.restore().await;

    fixture
        .state
        .note_remote_failure(Some("NOT_FOUND"), Some("Chat not found"))
        .await;

    assert!(fixture.snapshot().is_authenticated);
    assert!(fixture.transport.calls().is_empty());
}

#[tokio::test]
async fn switching_to_an_account_whose_session_died_reports_it_and_removes_it() {
    let transport = ScriptedTransport::answering(vec![]);
    let mut fixture = Fixture::new("switch-dead", transport);
    fixture.seed_account("u1", tokens(&live_token("u1"), &live_token("u1")));
    fixture.seed_account("u2", tokens(&dead_token("u2"), &dead_token("u2")));
    fixture.state.accounts = fixture.credentials.accounts();
    fixture.state.active = Some("u1".into());
    fixture.state.tokens = fixture
        .credentials
        .tokens("u1", fixture.secrets.as_ref())
        .unwrap();

    fixture.state.switch_account("u2".into()).await;

    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.flow.error_key, Some("account_session_expired"));
    assert_eq!(
        snapshot.active_account_id.as_deref(),
        Some("u1"),
        "the account that could not be entered is dropped and the other one comes back"
    );
}

#[tokio::test]
async fn a_locally_invalid_tag_is_refused_without_a_request() {
    let transport = ScriptedTransport::answering(vec![]);
    let mut fixture = Fixture::new("tag-invalid", transport);

    fixture.state.start_tag_check("no".into());

    assert_eq!(
        fixture.snapshot().flow.tag_availability,
        TagAvailability::Invalid
    );
    assert!(fixture.transport.calls().is_empty());
}

#[tokio::test]
async fn an_answer_for_a_tag_the_user_has_already_edited_past_is_dropped() {
    let transport = ScriptedTransport::answering(vec![]);
    let mut fixture = Fixture::new("tag-stale", transport);

    fixture.state.tag_generation = 7;
    fixture.state.flow.tag_availability = TagAvailability::Checking;

    fixture
        .state
        .finish_tag_check(6, "older".into(), TagAvailability::Taken);
    assert_eq!(
        fixture.state.flow.tag_availability,
        TagAvailability::Checking,
        "an answer from two keystrokes ago must not label the tag on screen"
    );

    fixture
        .state
        .finish_tag_check(7, "current".into(), TagAvailability::Available);
    assert_eq!(
        fixture.state.flow.tag_availability,
        TagAvailability::Available
    );
}

#[test]
fn a_failed_availability_check_never_tells_somebody_their_tag_is_taken() {
    assert_eq!(
        tag_availability(&rpc::CheckTagResponse {
            error: Some("Tag availability is temporarily unavailable".into()),
            ..rpc::CheckTagResponse::default()
        }),
        TagAvailability::Unknown
    );
    assert_eq!(
        tag_availability(&rpc::CheckTagResponse::default()),
        TagAvailability::Unknown
    );
    assert_eq!(
        tag_availability(&rpc::CheckTagResponse {
            available: Some(true),
            ..rpc::CheckTagResponse::default()
        }),
        TagAvailability::Available
    );
    assert_eq!(
        tag_availability(&rpc::CheckTagResponse {
            exists: Some(true),
            ..rpc::CheckTagResponse::default()
        }),
        TagAvailability::Taken
    );
}

#[tokio::test]
async fn the_wire_user_id_comes_from_the_token_rather_than_the_stored_profile() {
    let transport = ScriptedTransport::answering(vec![]);
    let mut fixture = Fixture::new("wire-id", transport);

    fixture.state.tokens = Some(tokens(&live_token("from-token"), &live_token("x")));
    fixture.state.active = Some("from-profile".into());

    assert_eq!(
        fixture.state.wire_user_id().as_deref(),
        Some("from-token"),
        "after a switch the profile can lag; the token is what the gateway authenticates"
    );
}
