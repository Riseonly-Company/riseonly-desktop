use futures::FutureExt as _;
use gpui::App;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::sai_auth_hooks::{SaiAuthHooks, set_sai_auth_hooks};
use crate::modules::auth::services::access_token_user_id::AccessTokenUserId;
use crate::modules::auth::services::auth_local_storage::AuthLocalStorage;
use crate::modules::auth::services::auth_session_guard::AuthSessionGuard;
use crate::modules::auth::services::auth_token_refresh::AuthTokenRefresh;
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

pub(crate) fn install_auth_services_hooks() {
    set_sai_auth_hooks(SaiAuthHooks {
        access_token: Box::new(|cx: &App| {
            let store = AuthServicesStore::try_read(cx)?;
            let token = trimming_whitespaces_and_newlines(&store.tokens_and_other_data().access_token);
            (!token.is_empty()).then(|| token.to_owned())
        }),
        is_expired: Box::new(AccessTokenUserId::is_expired),
        extract_user_id: Box::new(AccessTokenUserId::extract),
        refresh_access_token: Box::new(|cx: &mut App| {
            let refresh = AuthTokenRefresh::refresh_access_token(None, cx);
            async move { refresh.await }.boxed_local()
        }),
        ensure_valid_access_token: Box::new(|cx: &mut App| {
            let ensure = AuthTokenRefresh::ensure_valid_access_token(cx);
            async move { ensure.await }.boxed_local()
        }),
        handle_auth_error: Box::new(|cx: &mut App| {
            if let Some(recovery) = AuthSessionGuard::handle_auth_error(None, cx) {
                cx.spawn(async move |_| recovery.await).detach();
            }
        }),
        is_auth_error: Box::new(AuthSessionGuard::is_auth_error),
        note_authorized_reply: Box::new(AuthServicesStore::confirm_account_switch),
        handle_account_blocked: Box::new(|cx: &mut App| {
            AuthInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.handle_account_blocked(cx));
        }),
        is_premium: Box::new(|cx: &App| {
            AuthServicesStore::try_read(cx)
                .and_then(|store| store.profile())
                .is_some_and(|profile| profile.more.is_premium)
        }),
        effective_user_id_for_wire: Box::new(|cx: &App| {
            AuthServicesStore::try_read(cx).and_then(AuthServicesStore::effective_user_id_for_wire)
        }),
        debug_first_account: Box::new(|backend_id: &str, _: &App| {
            AuthServicesStore::debug_first_account(backend_id).map(|account| account.user.id)
        }),
        debug_detach_active_account: Box::new(|cx: &mut App| {
            let auth = AuthServicesStore::shared(cx);
            AuthServicesStore::debug_detach_active_account(&auth, cx);
        }),
        active_account_id: Box::new(|cx: &App| {
            AuthServicesStore::try_read(cx).and_then(|store| store.active_account_id().map(ToOwned::to_owned))
        }),
        switch_account: Box::new(|user_id: &str, cx: &mut App| {
            AuthServicesStore::shared(cx).update(cx, |auth, cx| auth.switch_account(user_id, cx));
        }),
        load_account_backend_id: Box::new(|user_id: &str| {
            AuthLocalStorage::load_account(user_id).and_then(|account| account.backend_id)
        }),
    });
}
