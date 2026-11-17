use std::collections::HashMap;

use futures::FutureExt as _;
use futures::future::Shared;
use gpui::{App, Global, Task};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::services::access_token_user_id::AccessTokenUserId;
use crate::modules::auth::services::auth_token_refresh::{AuthRefreshFailure, AuthTokenRefresh};
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

/// recovery closure of handle_auth_error, started only while the failed credential is still
/// the active one
pub type AuthSessionGuardRecovery = Box<dyn FnOnce(&mut App) -> Task<anyhow::Result<()>>>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RecoveryIdentity {
    account_id: String,
    account_epoch: Option<String>,
    access_token: String,
    refresh_token: String,
    session_id: String,
}

#[derive(Default)]
struct AuthSessionGuardHandling(HashMap<RecoveryIdentity, Uuid>);

impl Global for AuthSessionGuardHandling {}

pub enum AuthSessionGuard {}

impl AuthSessionGuard {
    pub fn is_auth_error(code: Option<&str>, message: Option<&str>) -> bool {
        let c = code.unwrap_or_default().to_uppercase();
        if c.contains("AUTH_ERROR") || c.contains("UNAUTHORIZED") || c.contains("UNAUTHENTICATED") || c == "401" {
            return true;
        }
        let m = message.unwrap_or_default().to_lowercase();
        if m.contains("authentication token") {
            return true;
        }
        if m.contains("invalid token") || m.contains("token expired") || m.contains("expired token") {
            return true;
        }
        if m.contains("invalid or expired") {
            return true;
        }
        if m.contains("auth_error") {
            return true;
        }
        false
    }

    pub fn handle_auth_error(recovery: Option<AuthSessionGuardRecovery>, cx: &mut App) -> Option<Shared<Task<()>>> {
        let auth = AuthServicesStore::shared(cx);
        if !auth.read(cx).is_authenticated() {
            return None;
        }
        let account_id = auth.read(cx).active_account_id().map(ToOwned::to_owned)?;
        if auth.update(cx, |auth, cx| auth.adopt_share_extension_credential_if_needed(cx))
            && !AccessTokenUserId::is_expired(&auth.read(cx).tokens_and_other_data().access_token, 30.0)
        {
            return None;
        }

        let identity = {
            let tokens = auth.read(cx).tokens_and_other_data();
            RecoveryIdentity {
                account_id,
                account_epoch: SaiAccountCacheScope::active_epoch(),
                access_token: trimming_whitespaces_and_newlines(&tokens.access_token).to_owned(),
                refresh_token: trimming_whitespaces_and_newlines(&tokens.refresh_token).to_owned(),
                session_id: trimming_whitespaces_and_newlines(&tokens.session_id).to_owned(),
            }
        };
        if cx.default_global::<AuthSessionGuardHandling>().0.contains_key(&identity) {
            return None;
        }

        let generation = Uuid::new_v4();
        cx.default_global::<AuthSessionGuardHandling>().0.insert(identity.clone(), generation);

        if identity.refresh_token.is_empty() || AccessTokenUserId::is_expired(&identity.refresh_token, 0.0) {
            Self::force_logout(&identity, cx);
            Self::finish(&identity, generation, cx);
            return None;
        }

        let task = cx
            .spawn(async move |cx| {
                let recovery = cx.update(|cx| {
                    if !Self::is_current(&identity, cx) {
                        return None;
                    }
                    Some(match recovery {
                        Some(recovery) => recovery(cx),
                        None => AuthTokenRefresh::refresh_access_token(None, cx),
                    })
                });
                if let Some(recovery) = recovery
                    && let Err(error) = recovery.await
                {
                    cx.update(|cx| {
                        if AuthRefreshFailure::ends_session(&error) {
                            Self::force_logout(&identity, cx);
                        } else {
                            Sai::log(
                                "Auth",
                                || "session recovery deferred; keeping selected account",
                                SaiLogKind::Network,
                                cx,
                            );
                        }
                    });
                }
                cx.update(|cx| Self::finish(&identity, generation, cx));
            })
            .shared();
        cx.foreground_executor().spawn(task.clone()).detach();
        Some(task)
    }

    fn force_logout(identity: &RecoveryIdentity, cx: &mut App) {
        if !Self::is_current(identity, cx) {
            return;
        }
        Sai::log("Auth", || "auth error \u{2192} forcing logout to login screen", SaiLogKind::Network, cx);
        AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.handle_session_refused(cx));
    }

    fn is_current(identity: &RecoveryIdentity, cx: &mut App) -> bool {
        let auth = AuthServicesStore::shared(cx);
        let auth = auth.read(cx);
        let tokens = auth.tokens_and_other_data();
        auth.is_authenticated()
            && auth.active_account_id().map(ToOwned::to_owned).as_deref() == Some(identity.account_id.as_str())
            && SaiAccountCacheScope::is_current_account_epoch(
                Some(&identity.account_id),
                identity.account_epoch.as_deref(),
            )
            && trimming_whitespaces_and_newlines(&tokens.access_token) == identity.access_token
            && trimming_whitespaces_and_newlines(&tokens.refresh_token) == identity.refresh_token
            && trimming_whitespaces_and_newlines(&tokens.session_id) == identity.session_id
    }

    fn finish(identity: &RecoveryIdentity, generation: Uuid, cx: &mut App) {
        let handling = &mut cx.default_global::<AuthSessionGuardHandling>().0;
        if handling.get(identity) != Some(&generation) {
            return;
        }
        handling.remove(identity);
    }
}
