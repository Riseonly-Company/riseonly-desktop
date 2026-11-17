use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::Shared;
use gpui::{App, Global, Task};
use rise_canon::rise_response;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_errors::ErrorCode;
use rise_i18n::tr;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_error::SaiFetchHttpError;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineSharedError;
use crate::modules::auth::services::access_token_user_id::AccessTokenUserId;
use crate::modules::auth::services::auth_local_storage::AuthLocalStorage;
use crate::modules::auth::stores::auth_actions::auth_types::TokensAndOtherData;
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthTokenRefreshError {
    MissingRefreshToken,
    RefreshTokenExpired,
    InvalidResponse,
}

impl AuthTokenRefreshError {
    pub fn error_description(&self) -> Option<String> {
        Some(tr("error_auth_token_invalid"))
    }
}

impl fmt::Display for AuthTokenRefreshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for AuthTokenRefreshError {}

/// request closure of refresh_access_token, sends the refresh token and answers the new pair
pub type AuthTokenRefreshRequest = Rc<dyn Fn(String, &mut App) -> Task<anyhow::Result<rise_response::auth::RefreshToken>>>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RefreshIdentity {
    account_id: String,
    account_epoch: Option<String>,
    access_token: String,
    refresh_token: String,
    session_id: String,
}

impl RefreshIdentity {
    fn has_same_credential(&self, other: &RefreshIdentity) -> bool {
        self.account_id == other.account_id
            && self.access_token == other.access_token
            && self.refresh_token == other.refresh_token
            && self.session_id == other.session_id
    }
}

type AuthTokenRefreshFlight = Shared<Task<Result<(), RiseAppEngineSharedError>>>;

struct InFlightRefresh {
    generation: Uuid,
    task: AuthTokenRefreshFlight,
}

#[derive(Default)]
struct AuthTokenRefreshInFlight(HashMap<RefreshIdentity, InFlightRefresh>);

impl Global for AuthTokenRefreshInFlight {}

pub enum AuthTokenRefresh {}

impl AuthTokenRefresh {
    pub fn ensure_valid_access_token(cx: &mut App) -> Task<anyhow::Result<()>> {
        let access = AuthServicesStore::shared(cx).read(cx).tokens_and_other_data().access_token.clone();
        if !AccessTokenUserId::is_expired(&access, 30.0) {
            return Task::ready(Ok(()));
        }
        Self::refresh_access_token(None, cx)
    }

    pub fn refresh_access_token(request: Option<AuthTokenRefreshRequest>, cx: &mut App) -> Task<anyhow::Result<()>> {
        let auth = AuthServicesStore::shared(cx);
        if auth.update(cx, |auth, cx| auth.adopt_share_extension_credential_if_needed(cx))
            && !AccessTokenUserId::is_expired(&auth.read(cx).tokens_and_other_data().access_token, 30.0)
        {
            tracing::warn!(target: "riseonly::X657", "refresh: share credential adopted");
            return Task::ready(Ok(()));
        }
        let identity = Self::current_identity(cx);
        let in_flight = &cx.default_global::<AuthTokenRefreshInFlight>().0;
        if let Some(refresh) = in_flight.get(&identity) {
            tracing::warn!(target: "riseonly::X657", "refresh: joins flight in progress");
            let flight = refresh.task.clone();
            return cx.spawn(async move |_| flight.await.map_err(anyhow::Error::from));
        }

        let previous = in_flight
            .iter()
            .find(|(key, _)| key.has_same_credential(&identity))
            .map(|(_, refresh)| refresh.task.clone());
        if let Some(previous) = previous {
            tracing::warn!(target: "riseonly::X657", "refresh: waits previous epoch flight");
            return cx.spawn(async move |cx| {
                previous.await.map_err(anyhow::Error::from)?;
                if cx.update(|cx| Self::current_identity(cx)) != identity {
                    return Ok(());
                }
                let account_id = identity.account_id.clone();
                let account = cx
                    .background_executor()
                    .spawn(async move { AuthLocalStorage::load_account(&account_id) })
                    .await
                    .ok_or(AuthTokenRefreshError::InvalidResponse)?;
                let next = cx.update(|cx| {
                    if account.tokens.access_token != identity.access_token
                        || account.tokens.refresh_token != identity.refresh_token
                        || account.session_id != identity.session_id
                    {
                        AuthServicesStore::shared(cx).update(cx, |auth, cx| {
                            auth.set_tokens_and_other_data(
                                TokensAndOtherData {
                                    access_token: account.tokens.access_token.clone(),
                                    refresh_token: account.tokens.refresh_token.clone(),
                                    session_id: account.session_id.clone(),
                                },
                                cx,
                            );
                        });
                        if !AccessTokenUserId::is_expired(&account.tokens.access_token, 30.0) {
                            return None;
                        }
                    }
                    Some(Self::refresh_access_token(request, cx))
                });
                match next {
                    Some(next) => next.await,
                    None => Ok(()),
                }
            });
        }

        let generation = Uuid::new_v4();
        let flight_identity = identity.clone();
        let flight: AuthTokenRefreshFlight = cx
            .spawn(async move |cx| {
                let identity = flight_identity;
                let started = RiseClock::monotonic();
                let mut attempt = 0usize;
                let result = 'refresh: {
                    if cx.update(|cx| Self::current_identity(cx)) != identity {
                        tracing::warn!(target: "riseonly::X288", "refresh skipped: credential changed before start");
                        break 'refresh Ok(());
                    }
                    loop {
                        let perform = cx.update(|cx| {
                            AuthServicesStore::shared(cx)
                                .update(cx, |auth, cx| auth.perform_token_refresh(request.clone(), cx))
                        });
                        tracing::warn!(target: "riseonly::X657", attempt, "refresh: request starts");
                        let deadline = cx
                            .background_executor()
                            .timer(Duration::from_secs_f64(AuthRefreshFailure::ATTEMPT_DEADLINE));
                        let outcome = match futures::future::select(Box::pin(perform), deadline).await {
                            futures::future::Either::Left((outcome, _)) => outcome,
                            futures::future::Either::Right(((), _)) => {
                                Err(anyhow::anyhow!("token refresh request did not settle in time"))
                            }
                        };
                        let error = match outcome {
                            Ok(()) => {
                                tracing::warn!(target: "riseonly::X657", attempt, "refresh: request done");
                                break 'refresh Ok(());
                            }
                            Err(error) => error,
                        };
                        tracing::warn!(target: "riseonly::X657", attempt, %error, "refresh: request failed");
                        if AuthRefreshFailure::ends_session(&error) {
                            cx.update(|cx| {
                                if Self::current_identity(cx) == identity {
                                    Sai::log(
                                        "Auth",
                                        || "refresh refused by server \u{2192} signing out",
                                        SaiLogKind::Network,
                                        cx,
                                    );
                                    AuthInteractionsStore::shared(cx)
                                        .update(cx, |interactions, cx| interactions.handle_session_refused(cx));
                                }
                            });
                            break 'refresh Err(RiseAppEngineSharedError::new(error));
                        }
                        let retries = AuthRefreshFailure::is_transient(&error)
                            && attempt < AuthRefreshFailure::RETRY_DELAYS.len()
                            && RiseClock::monotonic().saturating_sub(started)
                                < Duration::from_secs_f64(AuthRefreshFailure::RETRY_BUDGET)
                            && cx.update(|cx| Self::current_identity(cx)) == identity;
                        if !retries {
                            break 'refresh Err(RiseAppEngineSharedError::new(error));
                        }
                        cx.background_executor()
                            .timer(Duration::from_secs_f64(AuthRefreshFailure::RETRY_DELAYS[attempt]))
                            .await;
                        attempt += 1;
                        if cx.update(|cx| Self::current_identity(cx)) != identity {
                            break 'refresh Err(RiseAppEngineSharedError::new(error));
                        }
                    }
                };
                cx.update(|cx| Self::finish(&identity, generation, cx));
                result
            })
            .shared();
        cx.default_global::<AuthTokenRefreshInFlight>().0.insert(
            identity,
            InFlightRefresh {
                generation,
                task: flight.clone(),
            },
        );
        cx.spawn(async move |_| flight.await.map_err(anyhow::Error::from))
    }

    fn current_identity(cx: &mut App) -> RefreshIdentity {
        let auth = AuthServicesStore::shared(cx);
        let auth = auth.read(cx);
        let tokens = auth.tokens_and_other_data();
        RefreshIdentity {
            account_id: auth.active_account_id().map(ToOwned::to_owned).unwrap_or_default(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            access_token: trimming_whitespaces_and_newlines(&tokens.access_token).to_owned(),
            refresh_token: trimming_whitespaces_and_newlines(&tokens.refresh_token).to_owned(),
            session_id: trimming_whitespaces_and_newlines(&tokens.session_id).to_owned(),
        }
    }

    fn finish(identity: &RefreshIdentity, generation: Uuid, cx: &mut App) {
        let in_flight = &mut cx.default_global::<AuthTokenRefreshInFlight>().0;
        if in_flight.get(identity).map(|refresh| refresh.generation) != Some(generation) {
            return;
        }
        in_flight.remove(identity);
    }
}

/// which refresh failures end the session and which are worth the same request again
pub enum AuthRefreshFailure {}

impl AuthRefreshFailure {
    pub const RETRY_DELAYS: [f64; 4] = [0.5, 1.0, 2.0, 4.0];
    pub const RETRY_BUDGET: f64 = 22.0;
    pub const ATTEMPT_DEADLINE: f64 = 15.0;
    const REFUSAL_CODES: [ErrorCode; 4] = [
        ErrorCode::AuthSessionRevoked,
        ErrorCode::AuthTokenInvalid,
        ErrorCode::AuthRequired,
        ErrorCode::CommonFieldRequired,
    ];

    pub fn ends_session(error: &anyhow::Error) -> bool {
        let Some(body) = error.chain().find_map(|cause| match cause.downcast_ref::<SaiFetchHttpError>() {
            Some(SaiFetchHttpError::BadStatus { code: 401, body }) => Some(body),
            _ => None,
        }) else {
            return false;
        };
        let text = String::from_utf8_lossy(body);
        Self::REFUSAL_CODES.iter().any(|code| text.contains(code.wire()))
    }

    pub fn is_transient(error: &anyhow::Error) -> bool {
        if let Some(code) = error.chain().find_map(|cause| match cause.downcast_ref::<SaiFetchHttpError>() {
            Some(SaiFetchHttpError::BadStatus { code, .. }) => Some(*code),
            _ => None,
        }) {
            return code == 401 || code == 408 || code == 429 || code >= 500;
        }
        if error.chain().any(|cause| cause.is::<CancellationError>()) {
            return false;
        }
        true
    }
}
