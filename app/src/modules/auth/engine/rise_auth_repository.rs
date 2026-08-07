use std::sync::Arc;

use rise_engine::WireError;
use rise_platform::{DeviceLocale, HostOs, SecureStore};
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use crate::core::engine_bridge::SocketCredential;
use crate::modules::auth::services::access_token_user_id::AccessTokenUserId;
use crate::modules::auth::services::auth_session_guard::{AuthRecovery, recovery_for};
use crate::modules::auth::shared::auth_error_message;
use crate::modules::auth::shared::auth_validation;
use crate::modules::auth::shared::device_identity::DeviceIdentity;

use super::core::rise_auth_credential_store::AuthCredentialStore;
use super::core::rise_auth_rpc as rpc;
use super::core::rise_auth_transport::AuthTransport;
use super::rise_auth_debug_trace::{AuthTrace, TraceEvent};
use super::rise_auth_engine_models::{
    AccountLimitPolicy, AuthUser, PersistedAccount, ResetReason, StoredTokens, ordered_accounts,
};
use super::rise_auth_presentation::{
    AuthFlowState, AuthSnapshot, RiseAuthPresentation, TagAvailability,
};

/// Refresh this long before the access token actually dies.
///
/// The reference's thirty seconds. Spending a round trip on a token that expires
/// mid-flight costs a retry and, on the socket, a whole reconnect.
const ACCESS_TOKEN_LEEWAY_SECONDS: i64 = 30;

/// How long a sign-out waits for the server before clearing local state anyway.
///
/// The local reset is unconditional: a user who pressed "log out" must not still
/// be signed in because the network was gone. The wait exists only so that the
/// server usually gets to revoke the session too.
const LOGOUT_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

pub enum AuthCommand {
    Restore,
    SendCode {
        phone: String,
    },
    Register {
        name: String,
        tag: String,
        phone: String,
        password: String,
        code: String,
    },
    SignIn {
        phone: String,
        password: String,
    },
    CheckTag {
        tag: String,
    },
    TagChecked {
        generation: u64,
        tag: String,
        availability: TagAvailability,
    },
    ConfirmTelegramCompleted,
    CancelCodeEntry,
    ClearError,
    ResetFlow,
    SignOut,
    SwitchAccount {
        user_id: String,
    },
    NoteRemoteFailure {
        code: Option<String>,
        message: Option<String>,
    },
    Shutdown,
}

/// The repository, as the canon means it: a task that owns the state and a
/// channel that serialises every mutation.
///
/// A mutex would give mutual exclusion without ordering, and every rule here
/// depends on ordering — a refresh that lands after a sign-out must find the
/// account already gone, not race it.
pub struct AuthRepository {
    commands: mpsc::Sender<AuthCommand>,
    snapshot: watch::Receiver<Arc<AuthSnapshot>>,
}

pub struct AuthRepositoryConfig {
    pub transport: Arc<dyn AuthTransport>,
    pub credentials: Arc<AuthCredentialStore>,
    pub secrets: Arc<dyn SecureStore>,
    pub host: HostOs,
    pub locale: DeviceLocale,
    pub now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl AuthRepository {
    pub fn spawn(runtime: &tokio::runtime::Handle, config: AuthRepositoryConfig) -> Self {
        let (commands, inbox) = mpsc::channel(64);
        let (presentation, snapshot) = RiseAuthPresentation::new();

        let state = AuthState::new(config, presentation, commands.clone());
        runtime.spawn(state.run(inbox));

        Self { commands, snapshot }
    }

    pub fn snapshot(&self) -> Arc<AuthSnapshot> {
        Arc::clone(&self.snapshot.borrow())
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<AuthSnapshot>> {
        self.snapshot.clone()
    }

    /// Commands are dropped rather than queued when the actor is gone. Failing
    /// loudly here would mean a shutting-down app panicking on its last frame.
    pub fn dispatch(&self, command: AuthCommand) {
        let _ = self.commands.try_send(command);
    }
}

struct AuthState {
    transport: Arc<dyn AuthTransport>,
    credentials: Arc<AuthCredentialStore>,
    secrets: Arc<dyn SecureStore>,
    host: HostOs,
    locale: DeviceLocale,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    presentation: RiseAuthPresentation,
    commands: mpsc::Sender<AuthCommand>,
    trace: AuthTrace,

    accounts: Vec<PersistedAccount>,
    active: Option<String>,
    tokens: Option<StoredTokens>,
    flow: AuthFlowState,
    is_restoring: bool,
    /// Bumped on every keystroke that starts a tag check, so an answer for a tag
    /// the user has already edited past cannot land on the newer one.
    tag_generation: u64,
}

impl AuthState {
    fn new(
        config: AuthRepositoryConfig,
        presentation: RiseAuthPresentation,
        commands: mpsc::Sender<AuthCommand>,
    ) -> Self {
        Self {
            transport: config.transport,
            credentials: config.credentials,
            secrets: config.secrets,
            host: config.host,
            locale: config.locale,
            now_ms: config.now_ms,
            presentation,
            commands,
            trace: AuthTrace::new(),
            accounts: Vec::new(),
            active: None,
            tokens: None,
            flow: AuthFlowState::default(),
            is_restoring: false,
            tag_generation: 0,
        }
    }

    async fn run(mut self, mut inbox: mpsc::Receiver<AuthCommand>) {
        while let Some(command) = inbox.recv().await {
            match command {
                AuthCommand::Restore => self.restore().await,
                AuthCommand::SendCode { phone } => self.send_code(phone).await,
                AuthCommand::Register {
                    name,
                    tag,
                    phone,
                    password,
                    code,
                } => self.register(name, tag, phone, password, code).await,
                AuthCommand::SignIn { phone, password } => self.sign_in(phone, password).await,
                AuthCommand::CheckTag { tag } => self.start_tag_check(tag),
                AuthCommand::TagChecked {
                    generation,
                    tag,
                    availability,
                } => self.finish_tag_check(generation, tag, availability),
                AuthCommand::ConfirmTelegramCompleted => {
                    self.flow.telegram_redirect_active = false;
                    self.flow.code_entry_active = true;
                    self.publish();
                }
                AuthCommand::CancelCodeEntry => {
                    self.flow.code_entry_active = false;
                    self.publish();
                }
                AuthCommand::ClearError => {
                    self.flow.error_key = None;
                    self.publish();
                }
                AuthCommand::ResetFlow => {
                    self.flow = AuthFlowState::default();
                    self.publish();
                }
                AuthCommand::SignOut => self.sign_out(ResetReason::UserLogout).await,
                AuthCommand::SwitchAccount { user_id } => self.switch_account(user_id).await,
                AuthCommand::NoteRemoteFailure { code, message } => {
                    self.note_remote_failure(code.as_deref(), message.as_deref())
                        .await;
                }
                AuthCommand::Shutdown => break,
            }
        }
    }

    // ── lifecycle ───────────────────────────────────────────────────────────

    async fn restore(&mut self) {
        self.is_restoring = true;
        self.accounts = self.credentials.accounts();
        self.publish();

        let selected = self
            .credentials
            .active_account_id()
            .filter(|id| self.accounts.iter().any(|account| &account.user.id == id))
            .or_else(|| self.accounts.first().map(|account| account.user.id.clone()));

        let Some(user_id) = selected else {
            self.is_restoring = false;
            self.publish();
            return;
        };

        self.activate(&user_id, false);

        // A refresh token that is already dead cannot recover anything, so the
        // account is dropped rather than left on screen as a session that fails
        // every request it makes.
        if self.refresh_token_is_dead() {
            self.trace.record(TraceEvent::RestoreFoundExpiredSession);
            self.clear_active_account(ResetReason::ForcedLogout);
            self.is_restoring = false;
            self.publish();
            return;
        }

        if self.access_token_needs_refresh() {
            self.refresh_tokens().await;
        }

        self.is_restoring = false;
        self.publish();
    }

    /// Puts an account on screen and points the socket at it.
    fn activate(&mut self, user_id: &str, update_last_used: bool) {
        let Ok(Some(tokens)) = self.credentials.tokens(user_id, self.secrets.as_ref()) else {
            self.trace.record(TraceEvent::MissingCredential);
            return;
        };

        self.active = Some(user_id.to_owned());
        self.tokens = Some(tokens.clone());

        if update_last_used {
            let now = (self.now_ms)();
            let _ = self.credentials.set_active(user_id, now);
            if let Some(account) = self
                .accounts
                .iter_mut()
                .find(|account| account.user.id == user_id)
            {
                account.last_used_ms = now;
            }
        }

        self.transport
            .authenticate(SocketCredential::from_access_token(&tokens.access));
        self.trace.record(TraceEvent::Activated);
    }

    async fn switch_account(&mut self, user_id: String) {
        if self.active.as_deref() == Some(user_id.as_str()) {
            return;
        }
        if !self
            .accounts
            .iter()
            .any(|account| account.user.id == user_id)
        {
            self.flow.error_key = Some("account_session_unavailable");
            self.publish();
            return;
        }

        self.activate(&user_id, true);

        if self.refresh_token_is_dead() {
            self.flow.error_key = Some("account_session_expired");
            self.clear_active_account(ResetReason::ForcedLogout);
            self.publish();
            return;
        }

        if self.access_token_needs_refresh() {
            self.refresh_tokens().await;
        }
        self.publish();
    }

    async fn sign_out(&mut self, reason: ResetReason) {
        // Best effort, and bounded: the local reset happens whether or not the
        // server answers, so a dead network cannot leave somebody signed in
        // against their wishes.
        if let (Some(user_id), Some(tokens)) = (self.active.clone(), self.tokens.clone()) {
            let body = serde_json::to_value(rpc::LogoutRequest {
                user_id: self.wire_user_id().unwrap_or(user_id),
                session_id: tokens.session.clone(),
                refresh_token: tokens.refresh.clone(),
                device_token: None,
                platform: DeviceIdentity::platform(self.host).to_owned(),
            })
            .unwrap_or(Value::Null);

            let call = self.transport.call(&rpc::LOGOUT, body);
            if tokio::time::timeout(LOGOUT_GRACE, call).await.is_err() {
                self.trace.record(TraceEvent::LogoutTimedOut);
            }
        }

        self.clear_active_account(reason);
        self.publish();
    }

    /// Removes the current account and promotes whatever is left.
    fn clear_active_account(&mut self, reason: ResetReason) {
        let Some(user_id) = self.active.clone() else {
            self.tokens = None;
            self.transport.authenticate(SocketCredential::Anonymous);
            return;
        };

        let _ = self.credentials.remove(&user_id, self.secrets.as_ref());
        self.accounts = self.credentials.accounts();
        self.active = None;
        self.tokens = None;
        self.flow = AuthFlowState {
            error_key: self.flow.error_key,
            ..AuthFlowState::default()
        };
        self.trace.record(TraceEvent::Cleared(reason));

        match self.accounts.first().map(|account| account.user.id.clone()) {
            Some(next) => self.activate(&next, true),
            None => self.transport.authenticate(SocketCredential::Anonymous),
        }
    }

    // ── sign in and sign up ─────────────────────────────────────────────────

    async fn sign_in(&mut self, phone: String, password: String) {
        self.begin_busy();

        let body = serde_json::to_value(rpc::LoginRequest {
            phone_number: phone,
            password,
            device: DeviceIdentity::envelope(self.host, None),
        })
        .unwrap_or(Value::Null);

        let outcome = self.transport.call(&rpc::LOGIN, body).await;
        self.consume_grant(outcome, "auth_signin_error");
    }

    async fn send_code(&mut self, phone: String) {
        self.begin_busy();

        let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
        let body = serde_json::to_value(rpc::SendCodeRequest {
            phone_number: digits.clone(),
        })
        .unwrap_or(Value::Null);

        match self.transport.call(&rpc::SEND_CODE, body).await {
            Ok(payload) => {
                let response: rpc::SendCodeResponse =
                    serde_json::from_value(payload).unwrap_or_default();

                if !response.code_sent() {
                    self.fail_flow(response.error.as_deref(), "send_code_error");
                    return;
                }

                self.flow = AuthFlowState {
                    bot_username: Some(
                        response
                            .bot_username
                            .filter(|name| !name.trim().is_empty())
                            .unwrap_or_else(|| "riseonly_bot".to_owned()),
                    ),
                    phone_payload: Some(digits),
                    telegram_redirect_active: true,
                    tag_availability: self.flow.tag_availability,
                    checked_tag: self.flow.checked_tag.clone(),
                    ..AuthFlowState::default()
                };
                self.trace.record(TraceEvent::CodeSent);
                self.publish();
            }
            Err(error) => self.fail_wire(error, "send_code_error"),
        }
    }

    async fn register(
        &mut self,
        name: String,
        tag: String,
        phone: String,
        password: String,
        code: String,
    ) {
        self.begin_busy();

        let (region, languages) = DeviceIdentity::locale_hints(&self.locale);
        let body = serde_json::to_value(rpc::RegisterRequest {
            phone_number: phone,
            name: name.trim().to_owned(),
            tag: auth_validation::normalize_tag(&tag),
            code,
            password,
            device: DeviceIdentity::envelope(self.host, None),
            device_region: region,
            device_languages: languages,
            timezone: None,
        })
        .unwrap_or(Value::Null);

        let outcome = self.transport.call(&rpc::REGISTER, body).await;
        self.consume_grant(outcome, "register_error");
    }

    /// The one place a login, a registration or a restore becomes a session.
    fn consume_grant(&mut self, outcome: Result<Value, WireError>, fallback_key: &'static str) {
        let payload = match outcome {
            Ok(payload) => payload,
            Err(error) => {
                self.fail_wire(error, fallback_key);
                return;
            }
        };

        if let Some(message) = rpc::refusal_message(&payload) {
            self.fail_flow(Some(message.as_str()), fallback_key);
            return;
        }

        let grant: rpc::SessionGrant = serde_json::from_value(payload).unwrap_or_default();
        let (Some(user), Some(tokens)) = (grant.user.clone(), grant.resolved()) else {
            self.trace.record(TraceEvent::IncompleteGrant);
            self.fail_flow(None, fallback_key);
            return;
        };

        let user: AuthUser = user.into();
        let already_known = self
            .accounts
            .iter()
            .any(|account| account.user.id == user.id);
        let is_premium = self.active_user().map(|active| active.is_premium) == Some(true);

        if !already_known
            && !self.accounts.is_empty()
            && !AccountLimitPolicy::can_add(self.accounts.len(), is_premium)
        {
            self.fail_flow(None, "account_limit_reached");
            return;
        }

        let now = (self.now_ms)();
        let account = PersistedAccount {
            user: user.clone(),
            last_used_ms: now,
        };
        let stored = StoredTokens {
            access: tokens.access,
            refresh: tokens.refresh,
            session: tokens.session,
        };

        if self
            .credentials
            .save(account, &stored, self.secrets.as_ref())
            .is_err()
        {
            self.fail_flow(None, fallback_key);
            return;
        }

        self.accounts = self.credentials.accounts();
        self.active = Some(user.id.clone());
        self.tokens = Some(stored.clone());
        self.flow = AuthFlowState::default();
        self.transport
            .authenticate(SocketCredential::from_access_token(&stored.access));
        self.trace.record(TraceEvent::SignedIn);
        self.publish();
    }

    // ── tokens ──────────────────────────────────────────────────────────────

    fn now_seconds(&self) -> i64 {
        (self.now_ms)() / 1_000
    }

    fn access_token_needs_refresh(&self) -> bool {
        self.tokens.as_ref().is_some_and(|tokens| {
            AccessTokenUserId::is_expired(
                &tokens.access,
                self.now_seconds(),
                ACCESS_TOKEN_LEEWAY_SECONDS,
            )
        })
    }

    fn refresh_token_is_dead(&self) -> bool {
        self.tokens.as_ref().is_none_or(|tokens| {
            AccessTokenUserId::is_expired(&tokens.refresh, self.now_seconds(), 0)
        })
    }

    async fn refresh_tokens(&mut self) {
        let (Some(user_id), Some(current)) = (self.active.clone(), self.tokens.clone()) else {
            return;
        };

        let body = serde_json::to_value(rpc::RefreshRequest {
            refresh_token: current.refresh.clone(),
        })
        .unwrap_or(Value::Null);

        let outcome = self.transport.http(&rpc::REFRESH, body, None).await;
        let payload = match outcome {
            Ok(payload) => payload,
            Err(error) => {
                self.trace.record(TraceEvent::RefreshFailed);
                // A refused refresh is the end of the session; a network failure
                // is not. Signing somebody out because their wifi dropped would
                // lose the cache along with the session.
                if matches!(error, WireError::Remote(_)) {
                    self.clear_active_account(ResetReason::ForcedLogout);
                }
                return;
            }
        };

        let response: rpc::RefreshResponse = serde_json::from_value(payload).unwrap_or_default();
        let Some((access, refresh, session)) = response.resolved() else {
            self.trace.record(TraceEvent::RefreshFailed);
            self.clear_active_account(ResetReason::ForcedLogout);
            return;
        };

        let next = StoredTokens {
            access,
            refresh,
            session: session.unwrap_or(current.session.clone()),
        };

        // The compare-and-set is what makes two concurrent refreshes safe: the
        // loser sees that the pair on disk is no longer the one it started from
        // and abandons its own answer rather than reinstating an older token.
        match self
            .credentials
            .rotate_if_matching(&user_id, &current, &next, self.secrets.as_ref())
        {
            Ok(true) => {
                self.tokens = Some(next.clone());
                self.transport
                    .authenticate(SocketCredential::from_access_token(&next.access));
                self.trace.record(TraceEvent::Refreshed);
            }
            Ok(false) => self.trace.record(TraceEvent::RefreshRaceLost),
            Err(_) => self.trace.record(TraceEvent::RefreshFailed),
        }
    }

    /// What to do when any request anywhere came back as an auth failure.
    async fn note_remote_failure(&mut self, code: Option<&str>, message: Option<&str>) {
        let refresh = self
            .tokens
            .as_ref()
            .map(|tokens| tokens.refresh.clone())
            .unwrap_or_default();

        match recovery_for(code, message, &refresh, self.now_seconds()) {
            AuthRecovery::NotAuthRelated => {}
            AuthRecovery::Refresh => {
                self.refresh_tokens().await;
                self.publish();
            }
            AuthRecovery::SignOut => {
                self.clear_active_account(ResetReason::ForcedLogout);
                self.publish();
            }
        }
    }

    // ── tag availability ────────────────────────────────────────────────────

    fn start_tag_check(&mut self, tag: String) {
        let normalized = auth_validation::normalize_tag(&tag);
        self.tag_generation = self.tag_generation.wrapping_add(1);
        let generation = self.tag_generation;

        if auth_validation::validate_tag(&normalized).is_err() {
            self.flow.tag_availability = TagAvailability::Invalid;
            self.flow.checked_tag = Some(normalized);
            self.publish();
            return;
        }

        self.flow.tag_availability = TagAvailability::Checking;
        self.flow.checked_tag = Some(normalized.clone());
        self.publish();

        // Spawned rather than awaited: the actor stays responsive to the next
        // keystroke, and the generation on the way back is what keeps a slow
        // answer from landing on a tag the user has already edited past.
        let transport = Arc::clone(&self.transport);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let body = serde_json::to_value(rpc::CheckTagRequest {
                tag: normalized.clone(),
            })
            .unwrap_or(Value::Null);

            let availability = match transport.http(&rpc::CHECK_TAG, body, None).await {
                Ok(payload) => {
                    let response: rpc::CheckTagResponse =
                        serde_json::from_value(payload).unwrap_or_default();
                    tag_availability(&response)
                }
                Err(_) => TagAvailability::Unknown,
            };

            let _ = commands
                .send(AuthCommand::TagChecked {
                    generation,
                    tag: normalized,
                    availability,
                })
                .await;
        });
    }

    fn finish_tag_check(&mut self, generation: u64, tag: String, availability: TagAvailability) {
        if generation != self.tag_generation {
            return;
        }
        self.flow.tag_availability = availability;
        self.flow.checked_tag = Some(tag);
        self.publish();
    }

    // ── shared plumbing ─────────────────────────────────────────────────────

    fn begin_busy(&mut self) {
        self.flow.is_busy = true;
        self.flow.error_key = None;
        self.publish();
    }

    fn fail_flow(&mut self, server_message: Option<&str>, fallback: &'static str) {
        self.flow.is_busy = false;
        self.flow.telegram_redirect_active = false;
        self.flow.error_key = Some(auth_error_message::message_key(server_message, fallback));
        self.trace.record(TraceEvent::FlowRefused);
        self.publish();
    }

    /// A transport failure shows the step's own fallback string, not a network
    /// message of its own.
    ///
    /// That is the reference's behaviour and it is deliberate here rather than
    /// inherited: inventing "check your connection" would mean adding a key to
    /// 41 locale files, and the reference ships no generic one. What the server
    /// said, when it said anything, always wins over the fallback.
    fn fail_wire(&mut self, error: WireError, fallback: &'static str) {
        match &error {
            WireError::Remote(remote) => {
                let message = remote.message.clone();
                self.fail_flow(Some(message.as_str()), fallback);
            }
            _ => {
                self.trace.record(TraceEvent::TransportFailed);
                self.fail_flow(None, fallback);
            }
        }
    }

    fn active_user(&self) -> Option<&AuthUser> {
        let active = self.active.as_deref()?;
        self.accounts
            .iter()
            .find(|account| account.user.id == active)
            .map(|account| &account.user)
    }

    /// The id a request is stamped with.
    ///
    /// The token's own claim beats the stored profile: after a switch the
    /// profile can lag by a frame, and the token is what the gateway will
    /// actually authenticate the call as.
    fn wire_user_id(&self) -> Option<String> {
        self.tokens
            .as_ref()
            .and_then(|tokens| AccessTokenUserId::extract(&tokens.access))
            .or_else(|| self.active.clone())
    }

    fn publish(&mut self) {
        let snapshot = AuthSnapshot {
            revision: 0,
            is_authenticated: self.active.is_some()
                && self.tokens.as_ref().is_some_and(StoredTokens::is_complete),
            active: self.active_user().cloned(),
            active_account_id: self.active.clone(),
            accounts: ordered_accounts(&self.accounts, self.active.as_deref()),
            flow: self.flow.clone(),
            is_restoring: self.is_restoring,
        };

        self.presentation.publish(snapshot);
    }
}

/// A tag is available only when the server says so.
///
/// `Unknown` for anything ambiguous: telling somebody a tag is free and then
/// failing their registration is worse than asking them to try again.
fn tag_availability(response: &rpc::CheckTagResponse) -> TagAvailability {
    if response.error.is_some() {
        return TagAvailability::Unknown;
    }
    match (response.available, response.exists) {
        (Some(true), _) => TagAvailability::Available,
        (Some(false), _) | (_, Some(true)) => TagAvailability::Taken,
        (None, Some(false)) => TagAvailability::Available,
        (None, None) => TagAvailability::Unknown,
    }
}

#[cfg(test)]
#[path = "rise_auth_repository_tests.rs"]
mod tests;
