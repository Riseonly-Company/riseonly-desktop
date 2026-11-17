use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::LazyLock;

use gpui::{App, Context, Entity, EventEmitter, Task};
use parking_lot::RwLock;
use rise_canon::rise_wire_canon::key as wire_key;
use rise_canon::{RiseGuideSignal, RiseLimits, RiseTimingCanon, rise_http_path, rise_key, rise_request};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::CancellationError;
use rise_engine::rise_upload::rise_url_session::URLError;
use rise_errors::ErrorCode;
use rise_i18n::tr;
use rise_moderation::PlatformRole;
use rise_validate::VERIFICATION_CODE;
use serde_json::{Map, Value};

use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::lib::canon_rules::{RiseLimitsCanonRules, ValidationSchemaCanonRules};
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use crate::core::lib::sai::modules::sai_debugger::sai_dev_host_panel::SaiDevHostSwitcher;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::SaiFetchHttpConfiguration;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_engine::SaiFetchHttpEngine;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_error::SaiFetchHttpError;
use crate::core::lib::sai::sai::Sai;
use crate::core::maintenance::maintenance_gate::MaintenanceGate;
use crate::core::push_notifications::push_notification_manager_hooks::PushNotificationManagerHooks;
use crate::core::rise_engine_bridge::rise_canon_fields::RiseCanonFields;
use crate::core::stores::http::http_service::HttpService;
use crate::core::ui::clever_image_disk_cache::{CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::auth::services::access_token_user_id::AccessTokenUserId;
use crate::modules::auth::services::account_state_reset_coordinator::{
    AccountStateResetCoordinator, AccountStateResetReason,
};
use crate::modules::auth::services::app_attest_client::AppAttestClient;
use crate::modules::auth::services::auth_entry_order::AuthEntryOrder;
use crate::modules::auth::services::auth_local_storage::{AuthLocalStorage, AuthPersistedAccount};
use crate::modules::auth::services::auth_token_refresh::{
    AuthTokenRefresh, AuthTokenRefreshError, AuthTokenRefreshRequest,
};
use crate::modules::auth::stores::auth_actions::auth_types::{
    AuthAccountError, AuthAccountLimitPolicy, AuthAccountSummary, AuthCodeChannel, AuthCodeDelivery,
    AuthFlowFailure, SignInFailure, StoredAccessTokens, TokensAndOtherData,
};
use crate::modules::auth::stores::auth_actions::auth_wire_types::{
    AuthLoginResponse, AuthManagedSessionResponse, AuthRefreshResponse, AuthRegisterResponse,
    AuthResetPasswordResponse, AuthSendCodeResponse, AuthSendCodeResponseExt, AuthUserDTO,
    RefreshTokenResponseExt,
};
use crate::modules::auth::stores::auth_actions::profile_model_hooks::ProfileModel;
use crate::modules::auth::stores::auth_services::auth_services_guides_hooks::AuthServicesGuidesHooks;
use crate::modules::auth::stores::auth_services::auth_services_notify_hooks::AuthServicesNotifyHooks;
use crate::modules::auth::stores::auth_services::auth_services_share_hooks::auth_services_share_hooks;
use crate::modules::auth::stores::auth_services::auth_services_user_hooks::AuthServicesUserHooks;
use crate::modules::onboarding::pages::onboarding::onboarding_progress::OnboardingProgress;

const DEFAULT_REGISTRATION_BOT_USERNAME: &str = "riseonly_bot";
const ACCESS_TOKEN_LEEWAY_SECONDS: f64 = 30.0;
const REFRESH_TIMEOUT_SECONDS: f64 = 10.0;

const OUTAGE_CODES: [&str; 4] = [
    ErrorCode::GwTimeout.wire(),
    ErrorCode::GwUpstreamUnavailable.wire(),
    ErrorCode::CommonTimeout.wire(),
    ErrorCode::CommonUpstreamUnavailable.wire(),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthServicesViewerMirror {
    pub effective_user_id_for_wire: Option<String>,
    pub profile_name: Option<String>,
    pub profile_logo: Option<String>,
}

static VIEWER_MIRROR: LazyLock<RwLock<AuthServicesViewerMirror>> =
    LazyLock::new(|| RwLock::new(AuthServicesViewerMirror::default()));

#[derive(Clone, Debug, PartialEq, Eq)]
struct RefreshAttempt {
    account_id: String,
    account_epoch: Option<String>,
    access_token: String,
    refresh_token: String,
    session_id: String,
}

pub struct AuthFlowChanged;

pub struct AuthServicesStore {
    tokens_and_other_data: TokensAndOtherData,
    profile: Option<ProfileModel>,
    is_authenticated: bool,
    has_restored_persisted_session: bool,
    is_account_blocked: bool,
    lands_on_sign_in: bool,
    accounts: Vec<AuthAccountSummary>,
    active_account_id: Option<String>,
    is_adding_account: bool,
    account_switcher_error: Option<String>,
    account_switch_identity: Option<(String, Option<String>)>,
    active_user_dto: Option<AuthUserDTO>,
    cached_access_token_identity: RefCell<Option<(String, Option<String>)>>,
    telegram_redirect_active: bool,
    registration_bot_username: String,
    registration_phone_payload: String,
    code_entry_active: bool,
    code_delivery: Option<AuthCodeDelivery>,
    auth_flow_error: Option<String>,
    auth_flow_error_code: Option<String>,
    password_reset_active: bool,
}

impl EventEmitter<AuthFlowChanged> for AuthServicesStore {}

impl AuthServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn try_read(cx: &App) -> Option<&Self> {
        rise_ui::shared::try_read::<Self>(cx)
    }

    pub fn viewer_mirror() -> AuthServicesViewerMirror {
        VIEWER_MIRROR.read().clone()
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            let restore = cx.update(|cx| {
                this.upgrade().map(|this| Self::restore_persisted_session(this, None, cx))
            });
            if let Some(restore) = restore {
                restore.await;
            }
        })
        .detach();
        Self::empty()
    }

    fn empty() -> Self {
        Self {
            tokens_and_other_data: TokensAndOtherData::default(),
            profile: None,
            is_authenticated: false,
            has_restored_persisted_session: false,
            is_account_blocked: false,
            lands_on_sign_in: false,
            accounts: Vec::new(),
            active_account_id: None,
            is_adding_account: false,
            account_switcher_error: None,
            account_switch_identity: None,
            active_user_dto: None,
            cached_access_token_identity: RefCell::new(None),
            telegram_redirect_active: false,
            registration_bot_username: DEFAULT_REGISTRATION_BOT_USERNAME.to_owned(),
            registration_phone_payload: String::new(),
            code_entry_active: false,
            code_delivery: None,
            auth_flow_error: None,
            auth_flow_error_code: None,
            password_reset_active: false,
        }
    }

    fn flow_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(AuthFlowChanged);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let mirror = AuthServicesViewerMirror {
            effective_user_id_for_wire: self.effective_user_id_for_wire(),
            profile_name: self.profile.as_ref().map(|profile| profile.name.clone()),
            profile_logo: self
                .profile
                .as_ref()
                .map(|profile| profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone())),
        };
        *VIEWER_MIRROR.write() = mirror;
        cx.notify();
    }

    pub fn tokens_and_other_data(&self) -> &TokensAndOtherData {
        &self.tokens_and_other_data
    }

    pub fn profile(&self) -> Option<&ProfileModel> {
        self.profile.as_ref()
    }

    pub fn is_authenticated(&self) -> bool {
        self.is_authenticated
    }

    pub fn has_restored_persisted_session(&self) -> bool {
        self.has_restored_persisted_session
    }

    pub fn is_account_blocked(&self) -> bool {
        self.is_account_blocked
    }

    pub fn lands_on_sign_in(&self) -> bool {
        self.lands_on_sign_in
    }

    pub fn accounts(&self) -> &[AuthAccountSummary] {
        &self.accounts
    }

    pub fn active_account_id(&self) -> Option<&str> {
        self.active_account_id.as_deref()
    }

    pub fn is_adding_account(&self) -> bool {
        self.is_adding_account
    }

    fn set_is_adding_account(&mut self, value: bool, cx: &mut Context<Self>) {
        self.is_adding_account = value;
        cx.defer(move |cx| GuideFacts::set(RiseGuideSignal::AddAccountFlowOpened, value, cx));
    }

    pub fn account_switcher_error(&self) -> Option<&str> {
        self.account_switcher_error.as_deref()
    }

    pub fn set_account_switcher_error(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        self.account_switcher_error = value;
        self.changed(cx);
    }

    pub fn telegram_redirect_active(&self) -> bool {
        self.telegram_redirect_active
    }

    pub fn registration_bot_username(&self) -> &str {
        &self.registration_bot_username
    }

    pub fn registration_phone_payload(&self) -> &str {
        &self.registration_phone_payload
    }

    pub fn code_entry_active(&self) -> bool {
        self.code_entry_active
    }

    pub fn code_delivery(&self) -> Option<&AuthCodeDelivery> {
        self.code_delivery.as_ref()
    }

    pub fn auth_flow_error(&self) -> Option<&str> {
        self.auth_flow_error.as_deref()
    }

    pub fn auth_flow_error_code(&self) -> Option<&str> {
        self.auth_flow_error_code.as_deref()
    }

    pub fn password_reset_active(&self) -> bool {
        self.password_reset_active
    }

    fn is_premium(&self) -> bool {
        self.profile.as_ref().is_some_and(|profile| profile.more.is_premium)
    }

    pub fn account_limit(&self) -> i64 {
        AuthAccountLimitPolicy::limit(self.is_premium())
    }

    pub fn can_add_account(&self) -> bool {
        AuthAccountLimitPolicy::can_add_account(self.accounts.len() as i64, self.is_premium())
    }

    pub fn sorted_accounts(&self) -> Vec<AuthAccountSummary> {
        let active = self.active_account_id.as_deref();
        let mut sorted: Vec<AuthAccountSummary> =
            self.accounts.iter().filter(|account| Some(account.user_id.as_str()) == active).cloned().collect();
        let mut others: Vec<AuthAccountSummary> =
            self.accounts.iter().filter(|account| Some(account.user_id.as_str()) != active).cloned().collect();
        others.sort_by(|lhs, rhs| {
            rhs.last_used_at.cmp(&lhs.last_used_at).then_with(|| lhs.user_id.cmp(&rhs.user_id))
        });
        sorted.extend(others);
        sorted
    }

    pub fn effective_user_id_for_wire(&self) -> Option<String> {
        RiseIdentity::viewer(
            || self.access_token_user_id(&self.tokens_and_other_data.access_token),
            || self.profile.as_ref().map(|profile| profile.id.clone()),
        )
    }

    pub fn is_current_user(&self, sender_id: &str) -> bool {
        RiseIdentity::is_viewer(Some(sender_id), self.effective_user_id_for_wire().as_deref())
    }

    pub fn viewer_acts_as_author(&self) -> bool {
        let Some(profile) = &self.profile else {
            return false;
        };
        if profile.more.role.is_empty() {
            return false;
        }
        let wire = trimming_whitespaces_and_newlines(&profile.more.role).to_uppercase();
        PlatformRole::from_wire(&wire).is_some_and(PlatformRole::acts_as_author)
    }

    pub fn viewer_bypasses_maintenance(&self) -> bool {
        let profile_role = self.profile.as_ref().map(|profile| profile.more.role.as_str()).unwrap_or_default();
        let role = if profile_role.is_empty() {
            self.active_user_dto.as_ref().and_then(|dto| dto.role.as_deref()).unwrap_or_default()
        } else {
            profile_role
        };
        let wire = trimming_whitespaces_and_newlines(role).to_uppercase();
        PlatformRole::from_wire(&wire).is_some_and(PlatformRole::bypasses_maintenance)
    }

    pub fn message_author_user_id_for_outgoing(&self) -> String {
        self.effective_user_id_for_wire().unwrap_or_default()
    }

    fn access_token_user_id(&self, token: &str) -> Option<String> {
        if let Some((cached_token, user_id)) = self.cached_access_token_identity.borrow().as_ref()
            && cached_token == token
        {
            return user_id.clone();
        }
        let user_id = AccessTokenUserId::extract(token);
        *self.cached_access_token_identity.borrow_mut() = Some((token.to_owned(), user_id.clone()));
        user_id
    }

    pub fn get_tokens_and_other_data(&self) -> TokensAndOtherData {
        self.tokens_and_other_data.clone()
    }

    pub fn set_tokens_and_other_data(&mut self, data: TokensAndOtherData, cx: &mut Context<Self>) {
        self.tokens_and_other_data = data;
        self.changed(cx);
        if let Some(hooks) = auth_services_share_hooks() {
            (hooks.synchronize_credential)(true);
        }
        self.connect_web_socket_if_has_access_token(cx);
    }

    pub fn apply_updated_session_profile(&mut self, model: ProfileModel, cx: &mut Context<Self>) {
        let Some(active_account_id) = self.active_account_id.clone() else {
            tracing::warn!(target: "riseonly::X453", "session profile dropped: no active account");
            return;
        };
        if !self.is_authenticated
            || model.id != active_account_id
            || self.active_user_dto.as_ref().map(|dto| dto.id.as_str()) != Some(active_account_id.as_str())
        {
            tracing::warn!(
                target: "riseonly::X453",
                "session profile dropped: authenticated={} model={} active={} dto={:?}",
                self.is_authenticated,
                model.id,
                active_account_id,
                self.active_user_dto.as_ref().map(|dto| dto.id.as_str())
            );
            return;
        }
        tracing::warn!(target: "riseonly::X453", "session profile applied badge={:?}", model.more.premium_badge_emoji);

        let mut m = model;
        if let Some(prev) = &self.profile {
            if m.level.is_none() {
                m.level = prev.level;
            }
            if m.streak.is_none() {
                m.streak = prev.streak;
            }
            if m.rating.is_none() {
                m.rating = prev.rating;
            }
        }
        let avatar_source = m.avatar_url.clone().unwrap_or_else(|| m.more.logo.clone());
        if let Some(mut dto) = self.active_user_dto.clone() {
            dto.role = Some(m.more.role.clone());
            dto.name = Some(m.name.clone());
            dto.tag = Some(m.tag.clone());
            dto.avatar_url = m.avatar_url.clone();
            dto.cover_image_url = m.cover_image_url.clone();
            dto.is_premium = Some(m.more.is_premium);
            dto.is_official = Some(m.more.is_official);
            dto.quick_reaction = Some(m.quick_reaction.clone());
            dto.premium_badge_emoji = m.more.premium_badge_emoji.clone();
            dto.name_color = m.more.name_color.clone();
            AuthLocalStorage::update_user(&dto);
            self.active_user_dto = Some(dto);
            self.profile = Some(m);
            self.reload_accounts();
        } else {
            self.profile = Some(m);
        }
        self.changed(cx);
        cx.defer(move |cx| {
            MaintenanceGate::shared(cx).update(cx, |gate, cx| gate.viewer_did_change(true, cx));
            Self::warm_account_avatar(Some(avatar_source), cx);
            if let Some(hooks) = auth_services_share_hooks() {
                (hooks.synchronize_credential)(false);
                (hooks.write_snapshot_keeping_persisted_targets)(cx);
            }
        });
    }

    pub fn restore_persisted_session(
        this: Entity<Self>,
        refresh_request: Option<AuthTokenRefreshRequest>,
        cx: &mut App,
    ) -> Task<()> {
        let task = Self::restore_persisted_session_steps(&this, refresh_request, cx);
        this.update(cx, |store, cx| {
            store.has_restored_persisted_session = true;
            store.changed(cx);
        });
        task
    }

    fn restore_persisted_session_steps(
        this: &Entity<Self>,
        refresh_request: Option<AuthTokenRefreshRequest>,
        cx: &mut App,
    ) -> Task<()> {
        if let Some(hooks) = auth_services_share_hooks() {
            (hooks.reconcile_persisted_credential)();
        }
        let persisted = AuthLocalStorage::load_accounts();
        this.update(cx, |store, cx| {
            store.accounts = persisted.iter().map(Self::summary).collect();
            store.changed(cx);
        });
        let remembered_id = AuthLocalStorage::active_account_id();
        #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
        if let Some(remembered_id) = &remembered_id {
            SaiDevHostSwitcher::repoint_if_needed(remembered_id, cx);
        }
        let usable: Vec<AuthPersistedAccount> =
            persisted.into_iter().filter(Self::belongs_to_current_backend).collect();
        let selected = remembered_id
            .as_deref()
            .and_then(|active_id| usable.iter().find(|account| account.user.id == active_id))
            .or_else(|| usable.first())
            .cloned();
        let Some(selected) = selected else {
            return Task::ready(());
        };

        Self::activate_persisted_account(this, &selected, false, false, cx);

        while this.read(cx).is_authenticated {
            let tokens = this.read(cx).tokens_and_other_data.clone();
            if !AccessTokenUserId::is_expired(&tokens.access_token, ACCESS_TOKEN_LEEWAY_SECONDS) {
                this.update(cx, |store, cx| store.connect_web_socket_if_has_access_token(cx));
                return Task::ready(());
            }

            if AccessTokenUserId::is_expired(&tokens.refresh_token, 0.0) {
                Self::end_refused_session_in_app(this, cx);
                continue;
            }

            this.update(cx, |store, cx| {
                store.has_restored_persisted_session = true;
                store.changed(cx);
            });
            let refresh = AuthTokenRefresh::refresh_access_token(refresh_request, cx);
            return cx.spawn(async move |cx| {
                if let Err(error) = refresh.await {
                    cx.update(|cx| {
                        Sai::log(
                            "Auth",
                            || format!("restore session refresh failed: {error}"),
                            SaiLogKind::Network,
                            cx,
                        );
                    });
                }
            });
        }
        Task::ready(())
    }

    pub fn perform_token_refresh(
        &mut self,
        request: Option<AuthTokenRefreshRequest>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let Some(refresh_account_id) = self.active_account_id.clone() else {
            Self::defer_end_refused_session(cx);
            return Task::ready(Err(AuthTokenRefreshError::InvalidResponse.into()));
        };
        let refresh_account_epoch = SaiAccountCacheScope::active_epoch();

        let refresh = trimming_whitespaces_and_newlines(&self.tokens_and_other_data.refresh_token).to_owned();
        if refresh.is_empty() {
            Self::defer_end_refused_session(cx);
            return Task::ready(Err(AuthTokenRefreshError::MissingRefreshToken.into()));
        }

        if AccessTokenUserId::is_expired(&refresh, 0.0) {
            Self::defer_end_refused_session(cx);
            return Task::ready(Err(AuthTokenRefreshError::RefreshTokenExpired.into()));
        }

        let attempt = RefreshAttempt {
            account_id: refresh_account_id,
            account_epoch: refresh_account_epoch,
            access_token: trimming_whitespaces_and_newlines(&self.tokens_and_other_data.access_token).to_owned(),
            refresh_token: refresh,
            session_id: trimming_whitespaces_and_newlines(&self.tokens_and_other_data.session_id).to_owned(),
        };
        cx.spawn(async move |this, cx| {
            let refresh_token = attempt.refresh_token.clone();
            let pending = cx.update(|cx| match &request {
                Some(request) => request(refresh_token, cx),
                None => Self::request_token_refresh(refresh_token, cx),
            });
            let decoded: AuthRefreshResponse = match pending.await {
                Ok(decoded) => decoded,
                Err(error) => {
                    let current = this.read_with(cx, |store, _| store.is_current_refresh_attempt(&attempt))?;
                    if !current {
                        let account_id = attempt.account_id.clone();
                        tracing::warn!(target: "riseonly::X288", "refresh discarded: stale failure");
                        cx.update(|cx| {
                            Sai::log(
                                "Auth",
                                || format!("discarded stale refresh failure user={account_id}"),
                                SaiLogKind::Network,
                                cx,
                            );
                        });
                        return Ok(());
                    }
                    return Err(error);
                }
            };

            let Some((access, refresh, session_id)) = decoded.resolved_tokens() else {
                let current = this.read_with(cx, |store, _| store.is_current_refresh_attempt(&attempt))?;
                if current {
                    return Err(AuthTokenRefreshError::InvalidResponse.into());
                }
                let account_id = attempt.account_id.clone();
                tracing::warn!(target: "riseonly::X288", "refresh discarded: stale invalid response");
                cx.update(|cx| {
                    Sai::log(
                        "Auth",
                        || format!("discarded invalid stale refresh response user={account_id}"),
                        SaiLogKind::Network,
                        cx,
                    );
                });
                return Ok(());
            };

            let session_id = trimming_whitespaces_and_newlines(session_id.as_deref().unwrap_or(&attempt.session_id))
                .to_owned();
            let resolved_session_id = if session_id.is_empty() { attempt.session_id.clone() } else { session_id };
            let stored = StoredAccessTokens {
                access_token: access.clone(),
                refresh_token: refresh.clone(),
            };
            let persisted = AuthLocalStorage::store_rotated_tokens(
                &attempt.account_id,
                &attempt.session_id,
                &stored,
                &resolved_session_id,
            )?;

            let current = this.read_with(cx, |store, _| store.is_current_refresh_attempt(&attempt))?;
            if !current {
                tracing::warn!(target: "riseonly::X288", "refresh discarded: stale after rotation persisted {persisted}");
                let account_id = attempt.account_id.clone();
                cx.update(|cx| {
                    Sai::log(
                        "Auth",
                        || {
                            if persisted {
                                format!("persisted rotated tokens for inactive account user={account_id}")
                            } else {
                                format!(
                                    "discarded stale refresh response for removed or changed account user={account_id}"
                                )
                            }
                        },
                        SaiLogKind::Network,
                        cx,
                    );
                });
                return Ok(());
            }

            if !persisted {
                return Err(AuthTokenRefreshError::InvalidResponse.into());
            }

            this.update(cx, |store, cx| {
                store.tokens_and_other_data = TokensAndOtherData {
                    access_token: access.clone(),
                    refresh_token: refresh,
                    session_id: resolved_session_id,
                };
                store.is_authenticated = true;
                store.account_switch_identity = None;
                store.changed(cx);
            })?;
            let store_holds_new = this.read_with(cx, |store, _| store.tokens_and_other_data.access_token == access)?;
            let handle_holds_new = cx.update(|cx| {
                Self::try_read(cx).map(|store| store.tokens_and_other_data.access_token == access)
            });
            let expires_in = |token: &str| {
                AccessTokenUserId::expiration_date(token)
                    .map(|exp| exp.time_interval_since(&crate::core::maintenance::maintenance_gate::RiseServerClock::now()) as i64)
            };
            tracing::warn!(
                target: "riseonly::X555",
                "refresh applied: new_differs {} store_holds_new {} handle_holds_new {:?} new_exp_in_s {:?} old_exp_in_s {:?}",
                access != attempt.access_token,
                store_holds_new,
                handle_holds_new,
                expires_in(&access),
                expires_in(&attempt.access_token)
            );
            if let Some(hooks) = auth_services_share_hooks() {
                (hooks.synchronize_credential)(true);
            }
            cx.update(|cx| {
                SaiWebSocketManager::shared(cx).update(cx, |manager, cx| manager.reconnect_with_access_token(&access, cx));
                Sai::log("Auth", || "access token refreshed", SaiLogKind::Network, cx);
            });
            Ok(())
        })
    }

    fn request_token_refresh(refresh_token: String, cx: &mut App) -> Task<anyhow::Result<AuthRefreshResponse>> {
        let mut body = Map::new();
        body.insert(rise_key::auth::REFRESH_TOKEN.to_owned(), Value::String(refresh_token));
        let fetch = SaiFetchHttpEngine::fetch_response_body(
            &HttpService::api_route(rise_http_path::auth::REFRESH),
            "POST",
            Some(&body),
            None,
            Some(REFRESH_TIMEOUT_SECONDS),
            Some("refreshToken"),
            cx,
        );
        cx.spawn(async move |cx| {
            let data = fetch.await?;
            let decoded = cx.update(|cx| {
                SaiFetchHttpConfiguration::shared(cx).read(cx).json_decoder.decode::<AuthRefreshResponse>(&data)
            })?;
            Ok(decoded)
        })
    }

    fn is_current_refresh_attempt(&self, attempt: &RefreshAttempt) -> bool {
        if self.active_account_id.as_deref() != Some(attempt.account_id.as_str())
            || !SaiAccountCacheScope::is_current_account_epoch(
                Some(&attempt.account_id),
                attempt.account_epoch.as_deref(),
            )
        {
            return false;
        }

        let current_access = trimming_whitespaces_and_newlines(&self.tokens_and_other_data.access_token);
        let current_refresh = trimming_whitespaces_and_newlines(&self.tokens_and_other_data.refresh_token);
        let current_session = trimming_whitespaces_and_newlines(&self.tokens_and_other_data.session_id);
        current_access == attempt.access_token
            && current_refresh == attempt.refresh_token
            && current_session == attempt.session_id
    }

    fn persist_and_activate_session(
        this: &Entity<Self>,
        user: AuthUserDTO,
        access: String,
        refresh: String,
        session_id: String,
        is_mirror: bool,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        if is_mirror {
            for stale in AuthLocalStorage::load_accounts() {
                if stale.is_mirror && stale.user.id != user.id {
                    AuthLocalStorage::remove_account(&stale.user.id);
                    AccountStateResetCoordinator::remove_persisted_cache(&stale.user.id, cx);
                }
            }
        }
        let persisted_accounts = AuthLocalStorage::load_accounts();
        let is_existing = persisted_accounts.iter().any(|account| account.user.id == user.id);
        let (active_account_id, profile_is_premium) = {
            let store = this.read(cx);
            (store.active_account_id.clone(), store.is_premium())
        };
        let current_persisted_is_premium = persisted_accounts
            .iter()
            .find(|account| Some(account.user.id.as_str()) == active_account_id.as_deref())
            .is_some_and(|account| account.user.is_premium == Some(true));
        let is_premium = profile_is_premium || current_persisted_is_premium;
        let can_persist_account = is_mirror
            || AuthAccountLimitPolicy::can_add_account(
                persisted_accounts
                    .iter()
                    .filter(|account| Self::belongs_to_current_backend(account) && !account.is_mirror)
                    .count() as i64,
                is_premium,
            );
        if !(is_existing || persisted_accounts.is_empty() || can_persist_account) {
            this.update(cx, |store, cx| store.set_account_switcher_error(Some(tr("account_limit_reached")), cx));
            return Err(AuthAccountError::LimitReached.into());
        }
        let tokens = StoredAccessTokens {
            access_token: access,
            refresh_token: refresh,
        };
        let now = Self::now_milliseconds();
        let backend_id = HttpService::resolved_backend_id();
        let reenters_active_account = active_account_id.as_deref() == Some(user.id.as_str());
        AuthLocalStorage::save_account(&user, &tokens, &session_id, now, Some(backend_id.as_str()), is_mirror)?;
        let account = AuthPersistedAccount {
            user,
            tokens,
            session_id,
            last_used_at: now,
            backend_id: Some(backend_id),
            is_mirror,
        };
        Self::activate_persisted_account(this, &account, false, false, cx);
        this.update(cx, |store, cx| {
            store.set_is_adding_account(false, cx);
            store.account_switcher_error = None;
            store.changed(cx);
        });
        if reenters_active_account {
            AuthServicesUserHooks::refresh_my_profile(cx);
        }
        Ok(())
    }

    pub fn handle_sign_in_success(&mut self, response: AuthLoginResponse, cx: &mut Context<Self>) {
        let this = cx.entity();
        cx.defer(move |cx| Self::sign_in_success(&this, response, cx));
    }

    fn sign_in_success(this: &Entity<Self>, response: AuthLoginResponse, cx: &mut App) {
        let access = response.access_token;
        let refresh = response.refresh_token;
        let session_id = response.session_id;
        let Some(user) = response.user.as_ref().map(AuthUserDTO::from_canon) else {
            let error = anyhow::Error::new(SignInFailure::MissingPayload);
            this.update(cx, |store, cx| store.handle_sign_in_error(&error, cx));
            return;
        };
        if access.is_empty() || refresh.is_empty() || session_id.is_empty() {
            let error = anyhow::Error::new(SignInFailure::MissingPayload);
            this.update(cx, |store, cx| store.handle_sign_in_error(&error, cx));
            return;
        }

        let user_id = user.id.clone();
        match Self::persist_and_activate_session(this, user, access, refresh, session_id, false, cx) {
            Ok(()) => {
                this.update(cx, |store, cx| {
                    store.auth_flow_error = None;
                    store.changed(cx);
                });
                Sai::log("Auth", || format!("sign-in success user={user_id}"), SaiLogKind::Network, cx);
            }
            Err(error) => this.update(cx, |store, cx| store.handle_sign_in_error(&error, cx)),
        }
    }

    pub fn handle_managed_login_success(&mut self, response: AuthManagedSessionResponse, cx: &mut Context<Self>) {
        let this = cx.entity();
        cx.defer(move |cx| Self::managed_login_success(&this, response, cx));
    }

    fn managed_login_success(this: &Entity<Self>, response: AuthManagedSessionResponse, cx: &mut App) {
        let access = response.access_token;
        let refresh = response.refresh_token;
        let session_id = response.session_id;
        let user = response.user.as_ref().map(AuthUserDTO::from_managed);
        let Some(user) = user.filter(|_| !access.is_empty() && !refresh.is_empty() && !session_id.is_empty())
        else {
            let error = anyhow::Error::new(SignInFailure::MissingPayload);
            this.update(cx, |store, cx| store.handle_managed_login_error(&error, cx));
            return;
        };
        let user_id = user.id.clone();
        match Self::persist_and_activate_session(this, user, access, refresh, session_id, true, cx) {
            Ok(()) => {
                this.update(cx, |store, cx| store.set_account_switcher_error(None, cx));
                Sai::log("Auth", || format!("managed login success user={user_id}"), SaiLogKind::Network, cx);
            }
            Err(error) => this.update(cx, |store, cx| store.handle_managed_login_error(&error, cx)),
        }
    }

    pub fn handle_managed_login_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.account_switcher_error = Some(
            Self::sign_in_outage_message(error)
                .unwrap_or_else(|| Self::auth_error_message(error, "retry_later_error")),
        );
        self.changed(cx);
        let description = error.to_string();
        Sai::log("Auth", || format!("managed login failed: {description}"), SaiLogKind::Network, cx);
    }

    pub fn handle_sign_in_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let description = error.to_string();
        Sai::log("Auth", || format!("sign-in error: {description}"), SaiLogKind::Network, cx);
        self.auth_flow_error_code = Self::auth_error_code(error);
        self.auth_flow_error = Some(
            Self::sign_in_outage_message(error)
                .unwrap_or_else(|| Self::auth_error_message(error, "auth_signin_error")),
        );
        self.flow_changed(cx);
    }

    fn sign_in_outage_message(error: &anyhow::Error) -> Option<String> {
        if Self::is_url_error(error) {
            return Some(tr("moderation_error_connection"));
        }
        let Some((status, body)) = Self::bad_status(error) else {
            return Some(tr("retry_later_error"));
        };
        let object = Self::json_object(body);
        let code = object
            .as_ref()
            .and_then(|object| object.get(wire_key::CODE))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if OUTAGE_CODES.contains(&code) {
            return Some(CanonErrors::text(Some(code), &HashMap::new()).unwrap_or_else(|| tr("server_error_unavailable")));
        }
        (500..=599).contains(&status).then(|| tr("server_error_unavailable"))
    }

    pub fn handle_send_code_success(
        &mut self,
        response: AuthSendCodeResponse,
        phone: &str,
        purpose: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if !response.code_sent() {
            self.auth_flow_error = Some(Self::localized_auth_message(Some(&response.error), "send_code_error"));
            self.telegram_redirect_active = false;
            self.code_entry_active = false;
            self.flow_changed(cx);
            return;
        }
        let wire_channel = trimming_whitespaces_and_newlines(&response.channel).to_owned();
        let channel = if wire_channel.is_empty() {
            Some(AuthCodeChannel::TelegramBot)
        } else {
            AuthCodeChannel::from_raw(&wire_channel)
        };
        let logged_channel = if wire_channel.is_empty() { "legacy".to_owned() } else { wire_channel.clone() };
        Sai::log("Auth", || format!("send-code success channel={logged_channel}"), SaiLogKind::Network, cx);
        self.auth_flow_error = None;
        self.auth_flow_error_code = None;
        self.registration_phone_payload = Self::digits(phone);
        let offered: Vec<AuthCodeChannel> =
            response.alternatives.iter().filter_map(|raw| AuthCodeChannel::from_raw(raw)).collect();
        self.code_delivery = Some(AuthCodeDelivery {
            phone_digits: self.registration_phone_payload.clone(),
            purpose: purpose.map(ToOwned::to_owned),
            channel,
            destination_masked: trimming_whitespaces_and_newlines(&response.destination_masked).to_owned(),
            code_length: Self::code_length(Some(response.code_length)),
            resend_available_at: RiseClock::now().adding_time_interval(response.resend_after_ms.max(0) as f64 / 1_000.0),
            alternatives: offered
                .iter()
                .copied()
                .filter(|offer| Some(*offer) != channel && offer.switch_title_key().is_some())
                .collect(),
            offers_telegram: channel != Some(AuthCodeChannel::TelegramBot)
                && offered.contains(&AuthCodeChannel::TelegramBot),
            named_by_server: !wire_channel.is_empty(),
        });

        if channel != Some(AuthCodeChannel::TelegramBot) {
            self.telegram_redirect_active = false;
            self.code_entry_active = true;
            self.flow_changed(cx);
            return;
        }
        self.code_entry_active = false;
        self.telegram_redirect_active = false;
        let bot_username = trimming_whitespaces_and_newlines(&response.bot_username);
        if !bot_username.is_empty() {
            self.registration_bot_username = bot_username.to_owned();
        }

        self.telegram_redirect_active = true;
        self.flow_changed(cx);
        let bot = self.registration_bot_username.clone();
        let payload = self.registration_phone_payload.clone();
        Sai::log(
            "Auth",
            || format!("Telegram step: https://t.me/{bot}?start={payload}"),
            SaiLogKind::Network,
            cx,
        );
    }

    fn code_length(announced: Option<i64>) -> i64 {
        let ceiling = RiseLimits::service_value("auth_verification_code_ceiling");
        match announced {
            Some(announced) if (1..=ceiling).contains(&announced) => announced,
            _ => VERIFICATION_CODE.maximum_value() as i64,
        }
    }

    pub fn handle_password_reset_success(&mut self, response: AuthResetPasswordResponse, cx: &mut Context<Self>) {
        let message = trimming_whitespaces_and_newlines(&response.error).to_owned();
        if !message.is_empty() {
            let logged = message.clone();
            Sai::log("Auth", || format!("password reset refused: {logged}"), SaiLogKind::Network, cx);
            self.present_auth_flow_error(&message, cx);
            return;
        }
        self.reset_auth_flow_presentation(cx);
        Sai::log("Auth", || "password reset success", SaiLogKind::Network, cx);
    }

    pub fn handle_password_reset_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let description = error.to_string();
        Sai::log("Auth", || format!("password reset error: {description}"), SaiLogKind::Network, cx);
        self.auth_flow_error_code = Self::auth_error_code(error);
        self.auth_flow_error = Some(Self::auth_error_message(error, "auth_reset_password_error"));
        self.flow_changed(cx);
    }

    pub fn begin_password_reset(&mut self, cx: &mut Context<Self>) {
        self.password_reset_active = true;
        self.flow_changed(cx);
    }

    pub fn cancel_password_reset(&mut self, cx: &mut Context<Self>) {
        self.password_reset_active = false;
        self.telegram_redirect_active = false;
        self.code_entry_active = false;
        self.code_delivery = None;
        self.flow_changed(cx);
    }

    pub fn confirm_bot_registration_completed(&mut self, cx: &mut Context<Self>) {
        self.telegram_redirect_active = false;
        self.code_entry_active = true;
        self.flow_changed(cx);
    }

    pub fn reopen_telegram_redirect(&mut self, cx: &mut Context<Self>) {
        if self.code_delivery.as_ref().and_then(|delivery| delivery.channel) != Some(AuthCodeChannel::TelegramBot) {
            return;
        }
        self.telegram_redirect_active = true;
        self.flow_changed(cx);
    }

    pub fn cancel_verification_code_entry(&mut self, cx: &mut Context<Self>) {
        self.code_entry_active = false;
        self.flow_changed(cx);
    }

    pub fn present_auth_flow_error(&mut self, message: &str, cx: &mut Context<Self>) {
        self.auth_flow_error = Some(Self::localized_auth_message(Some(message), "send_code_error"));
        self.flow_changed(cx);
    }

    pub fn present_localized_auth_flow_error(&mut self, text: &str, cx: &mut Context<Self>) {
        self.auth_flow_error = Some(text.to_owned());
        self.flow_changed(cx);
    }

    pub fn clear_auth_flow_error(&mut self, cx: &mut Context<Self>) {
        self.auth_flow_error = None;
        self.auth_flow_error_code = None;
        self.flow_changed(cx);
    }

    pub fn reset_auth_flow_presentation(&mut self, cx: &mut Context<Self>) {
        self.telegram_redirect_active = false;
        self.code_entry_active = false;
        self.password_reset_active = false;
        self.code_delivery = None;
        self.auth_flow_error = None;
        self.flow_changed(cx);
    }

    pub fn handle_send_code_error(
        &mut self,
        error: &anyhow::Error,
        phone: &str,
        purpose: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let description = error.to_string();
        Sai::log("Auth", || format!("send-code error: {description}"), SaiLogKind::Network, cx);
        let failure = Self::auth_failure(error);
        self.telegram_redirect_active = false;

        if let Some(failure) = &failure
            && failure.code.as_deref() == Some(ErrorCode::VerifyResendTooEarly.wire())
            && let Some(mut delivery) = self.code_delivery.clone()
            && delivery.phone_digits == Self::digits(phone)
            && delivery.purpose.as_deref() == purpose
        {
            if let Some(seconds) = CanonErrors::retry_after_seconds(&failure.details) {
                delivery.resend_available_at = RiseClock::now().adding_time_interval(seconds as f64);
                self.code_delivery = Some(delivery);
            }
            let resent_from_code_step = self.code_entry_active;
            self.auth_flow_error_code = None;
            self.auth_flow_error = if resent_from_code_step { CanonErrors::text_for(failure) } else { None };
            self.code_entry_active = true;
            self.flow_changed(cx);
            return;
        }

        self.code_entry_active = false;
        self.auth_flow_error_code = failure.as_ref().and_then(|failure| failure.code.clone());
        if self.password_reset_active
            && failure.as_ref().and_then(|failure| failure.code.as_deref())
                == Some(ErrorCode::UserAccountNotFound.wire())
        {
            self.auth_flow_error = Some(tr("auth_reset_password_no_account"));
            self.flow_changed(cx);
            return;
        }
        self.auth_flow_error = Some(
            failure
                .as_ref()
                .and_then(CanonErrors::text_for)
                .unwrap_or_else(|| Self::auth_error_message(error, "send_code_error")),
        );
        self.flow_changed(cx);
    }

    pub fn assumed_sms_delivery(
        body: &rise_request::auth::SendCode,
        error: &anyhow::Error,
        previous: Option<&AuthCodeDelivery>,
    ) -> Option<AuthSendCodeResponse> {
        if body.channel_preference.as_deref() == Some(AuthCodeChannel::TelegramBot.raw_value()) {
            return None;
        }
        let failure = Self::auth_failure(error)?;
        let code = failure.code.as_deref();
        if code == Some(ErrorCode::VerifyTemporarilyUnavailable.wire()) {
            if CanonErrors::retry_after_seconds(&failure.details).is_some() {
                return None;
            }
        } else if code != Some(ErrorCode::VerifyCountryNotSupported.wire()) {
            return None;
        }
        let mut offered: Vec<String> = Vec::new();
        if let Some(previous) = previous
            && previous.phone_digits == Self::digits(body.phone_number.as_deref().unwrap_or_default())
            && previous.purpose == body.purpose
        {
            offered = previous
                .channel
                .into_iter()
                .chain(previous.alternatives.iter().copied())
                .map(|channel| channel.raw_value().to_owned())
                .collect();
        }
        if Self::can_ask_telegram_bot() {
            offered.push(AuthCodeChannel::TelegramBot.raw_value().to_owned());
        }
        Some(AuthSendCodeResponse {
            alternatives: offered,
            channel: AuthCodeChannel::Sms.raw_value().to_owned(),
            code_length: VERIFICATION_CODE.maximum_value() as i64,
            resend_after_ms: RiseTimingCanon::server_window("otp_resend_ladder").window_ms as i64,
            success: true,
            ..Default::default()
        })
    }

    fn can_ask_telegram_bot() -> bool {
        RiseCanonFields::field("auth_send_code", "/channel_preference")
            .is_some_and(|field| field.allowed_values.contains(&AuthCodeChannel::TelegramBot.raw_value()))
    }

    pub fn auth_failure(error: &anyhow::Error) -> Option<RiseFailure> {
        let (_, body) = Self::bad_status(error)?;
        let object = Self::json_object(body)?;
        let code = object.get(wire_key::CODE).and_then(Value::as_str).filter(|code| !code.is_empty())?;
        Some(RiseFailure::new(
            Some(code.to_owned()),
            object.get(wire_key::ERROR).and_then(Value::as_str).unwrap_or_default().to_owned(),
            Self::string_details(&object),
        ))
    }

    fn string_details(object: &Map<String, Value>) -> HashMap<String, String> {
        let Some(details) = object.get(wire_key::DETAILS).and_then(Value::as_object) else {
            return HashMap::new();
        };
        details
            .iter()
            .filter_map(|(key, value)| {
                let text = match value {
                    Value::String(text) => text.clone(),
                    Value::Number(number) => match number.as_i64() {
                        Some(integer) => integer.to_string(),
                        None => number.to_string(),
                    },
                    Value::Bool(flag) => (if *flag { "1" } else { "0" }).to_owned(),
                    _ => return None,
                };
                Some((key.clone(), text))
            })
            .collect()
    }

    pub fn handle_register_success(&mut self, response: AuthRegisterResponse, cx: &mut Context<Self>) {
        let message = trimming_whitespaces_and_newlines(&response.error).to_owned();
        if !message.is_empty() {
            self.auth_flow_error = Some(Self::localized_auth_message(Some(&message), "register_error"));
            self.flow_changed(cx);
            return;
        }
        let this = cx.entity();
        cx.defer(move |cx| Self::register_success(&this, response, cx));
    }

    fn register_success(this: &Entity<Self>, response: AuthRegisterResponse, cx: &mut App) {
        let access = response.access_token;
        let refresh = response.refresh_token;
        let session_id = response.session_id;
        let has_tokens = !access.is_empty() && !refresh.is_empty() && !session_id.is_empty();
        let has_user = response.user.is_some();
        let user = response.user.as_ref().map(AuthUserDTO::from_canon);
        let Some(user) = user.filter(|_| has_tokens) else {
            Sai::log(
                "Auth",
                || format!("register parse failed user={has_user} tokens={has_tokens}"),
                SaiLogKind::Network,
                cx,
            );
            let error = anyhow::Error::new(AuthFlowFailure::MissingPayload);
            this.update(cx, |store, cx| store.handle_register_error(&error, cx));
            return;
        };

        let user_id = user.id.clone();
        match Self::persist_and_activate_session(this, user, access, refresh, session_id, false, cx) {
            Ok(()) => {
                this.update(cx, |store, cx| {
                    store.code_entry_active = false;
                    store.telegram_redirect_active = false;
                    store.auth_flow_error = None;
                    store.changed(cx);
                });
                Sai::log("Auth", || format!("register success user={user_id}"), SaiLogKind::Network, cx);
                let steps_shown = OnboardingProgress::slides_seen();
                EngagementInteractionsStore::shared(cx)
                    .update(cx, |engagement, cx| engagement.onboarding_complete(steps_shown, cx));
                AuthServicesNotifyHooks::note_account_created(cx);
                AuthServicesUserHooks::prepare_contact_sync_after_registration(cx);
                AuthServicesGuidesHooks::note_account_created(&user_id, cx);
            }
            Err(error) => this.update(cx, |store, cx| store.handle_register_error(&error, cx)),
        }
    }

    pub fn handle_register_error(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        let description = error.to_string();
        Sai::log("Auth", || format!("register error: {description}"), SaiLogKind::Network, cx);
        self.auth_flow_error_code = Self::auth_error_code(error);
        self.auth_flow_error = Some(Self::auth_error_message(error, "register_error"));
        self.flow_changed(cx);
    }

    pub fn auth_error_code(error: &anyhow::Error) -> Option<String> {
        let (_, body) = Self::bad_status(error)?;
        let object = Self::json_object(body)?;
        let code = object.get(wire_key::CODE).and_then(Value::as_str).filter(|code| !code.is_empty())?;
        if code == ErrorCode::AttestationInvalid.wire() {
            AppAttestClient::forget_key();
        }
        Some(code.to_owned())
    }

    fn auth_error_message(error: &anyhow::Error, fallback_key: &str) -> String {
        let Some(object) = Self::bad_status(error).and_then(|(_, body)| Self::json_object(body)) else {
            return tr(fallback_key);
        };

        if let Some(code) = object.get(wire_key::CODE).and_then(Value::as_str)
            && CanonErrors::entry(Some(code)).is_some()
        {
            return CanonErrors::text(Some(code), &Self::string_details(&object)).unwrap_or_else(|| tr(fallback_key));
        }
        for key in [wire_key::ERROR, wire_key::MESSAGE] {
            if let Some(value) = object.get(key).and_then(Value::as_str) {
                return Self::localized_auth_message(Some(value), fallback_key);
            }
        }
        tr(fallback_key)
    }

    fn localized_auth_message(message: Option<&str>, fallback_key: &str) -> String {
        let normalized = message.map(|message| trimming_whitespaces_and_newlines(message).to_lowercase()).unwrap_or_default();
        if normalized.is_empty() {
            return tr(fallback_key);
        }
        let contains_any = |needles: &[&str]| needles.iter().any(|needle| normalized.contains(needle));

        if fallback_key == "register_error" && normalized.ends_with("user already exists") {
            return tr("auth_phone_already_registered");
        }
        if normalized.contains("tag") && contains_any(&["exist", "reserved", "taken"]) {
            return tr("tag_already_exists");
        }
        if normalized.contains("phone") && contains_any(&["exist", "registered"]) {
            return tr("auth_phone_already_registered");
        }
        if normalized.contains("user") && normalized.contains("not found") {
            return tr("auth_reset_password_no_account");
        }
        if normalized.contains("code") {
            if normalized.contains("expired") {
                return tr("auth_code_expired");
            }
            if contains_any(&["invalid", "incorrect", "wrong"]) {
                return tr("send_code_invalidcode_error");
            }
        }
        if contains_any(&["too many", "rate limit", "60 seconds"]) {
            return tr("auth_rate_limited");
        }
        if normalized.contains("phone") {
            return tr("auth_phone_invalid");
        }
        if normalized.contains("name") {
            return tr("auth_name_invalid");
        }
        if normalized.contains("blocked") {
            return tr("block_type_full_desc");
        }
        if normalized.contains("password") {
            return if normalized.contains("128") {
                tr("auth_password_too_long")
            } else {
                tr("auth_password_too_short")
            };
        }
        if contains_any(&["credential", "unauthorized", "authentication"]) {
            return tr("auth_signin_error");
        }
        tr(fallback_key)
    }

    pub fn full_clear(&mut self, reason: AccountStateResetReason, cx: &mut Context<Self>) {
        let this = cx.entity();
        cx.defer(move |cx| Self::remove_current_account(&this, reason, cx));
    }

    pub(crate) fn confirm_account_switch(cx: &mut App) {
        if Self::try_read(cx).is_none_or(|store| store.account_switch_identity.is_none()) {
            return;
        }
        Self::shared(cx).update(cx, |store, _| store.account_switch_identity = None);
    }

    fn defer_end_refused_session(cx: &mut Context<Self>) {
        let this = cx.entity();
        cx.defer(move |cx| Self::end_refused_session_in_app(&this, cx));
    }

    pub fn end_refused_session(&mut self, cx: &mut Context<Self>) {
        Self::defer_end_refused_session(cx);
    }

    fn end_refused_session_in_app(this: &Entity<Self>, cx: &mut App) {
        let refused_account_switch = {
            let store = this.read(cx);
            store.account_switch_identity.as_ref().is_some_and(|(user_id, epoch)| {
                store.active_account_id.as_deref() == Some(user_id.as_str())
                    && SaiAccountCacheScope::is_current_account_epoch(Some(user_id), epoch.as_deref())
            })
        };
        Self::remove_current_account(this, AccountStateResetReason::ForcedLogout, cx);
        this.update(cx, |store, cx| {
            store.lands_on_sign_in = !store.is_authenticated;
            if refused_account_switch {
                store.account_switcher_error = Some(tr("account_session_expired"));
            }
            store.changed(cx);
        });
    }

    pub fn begin_adding_account(&mut self, cx: &mut Context<Self>) {
        if !self.can_add_account() {
            self.account_switcher_error = Some(tr("account_limit_reached"));
            self.changed(cx);
            return;
        }
        self.account_switcher_error = None;
        self.set_is_adding_account(true, cx);
        self.changed(cx);
    }

    pub fn cancel_adding_account(&mut self, cx: &mut Context<Self>) {
        self.set_is_adding_account(false, cx);
        self.account_switcher_error = None;
        self.changed(cx);
    }

    pub fn switch_account(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if self.active_account_id.as_deref() == Some(user_id) {
            tracing::warn!(target: "riseonly::X1529", "switch to the active account ignored");
            return;
        }
        let this = cx.entity();
        let user_id = user_id.to_owned();
        cx.defer(move |cx| Self::switch_account_to(&this, &user_id, cx));
    }

    fn switch_account_to(this: &Entity<Self>, user_id: &str, cx: &mut App) {
        if this.read(cx).active_account_id.as_deref() == Some(user_id) {
            return;
        }
        #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
        SaiDevHostSwitcher::repoint_if_needed(user_id, cx);
        let Some(account) = AuthLocalStorage::load_account(user_id) else {
            this.update(cx, |store, cx| {
                store.account_switcher_error = Some(tr("account_session_unavailable"));
                store.reload_accounts();
                store.changed(cx);
            });
            return;
        };
        if AccessTokenUserId::is_expired(&account.tokens.refresh_token, 0.0) {
            AuthLocalStorage::remove_account(user_id);
            AccountStateResetCoordinator::remove_persisted_cache(user_id, cx);
            this.update(cx, |store, cx| {
                store.reload_accounts();
                store.account_switcher_error = Some(tr("account_session_expired"));
                store.changed(cx);
            });
            return;
        }

        Self::activate_persisted_account(this, &account, true, true, cx);
        this.update(cx, |store, cx| store.set_account_switcher_error(None, cx));
        Self::refresh_taken_over_account_if_expired(this, &account, cx);
    }

    fn refresh_taken_over_account_if_expired(this: &Entity<Self>, account: &AuthPersistedAccount, cx: &mut App) {
        if !AccessTokenUserId::is_expired(&account.tokens.access_token, ACCESS_TOKEN_LEEWAY_SECONDS) {
            return;
        }
        let user_id = account.user.id.clone();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let weak = this.downgrade();
        cx.spawn(async move |cx| {
            let refresh = cx.update(|cx| {
                if !SaiAccountCacheScope::is_current_account_epoch(Some(&user_id), account_epoch.as_deref()) {
                    return None;
                }
                Some(AuthTokenRefresh::refresh_access_token(None, cx))
            });
            let Some(refresh) = refresh else {
                return;
            };
            let Err(error) = refresh.await else {
                return;
            };
            if !SaiAccountCacheScope::is_current_account_epoch(Some(&user_id), account_epoch.as_deref())
                || Self::is_refresh_transport_failure(&error)
            {
                return;
            }
            let message = Self::sign_in_outage_message(&error)
                .unwrap_or_else(|| Self::auth_error_message(&error, "retry_later_error"));
            let _ = weak.update(cx, |store, cx| store.set_account_switcher_error(Some(message), cx));
        })
        .detach();
    }

    fn is_refresh_transport_failure(error: &anyhow::Error) -> bool {
        if error.chain().any(|cause| cause.is::<CancellationError>()) || Self::is_url_error(error) {
            return true;
        }
        if let Some((code, _)) = Self::bad_status(error) {
            return code >= 500;
        }
        false
    }

    fn remove_current_account(this: &Entity<Self>, reason: AccountStateResetReason, cx: &mut App) {
        let Some(user_id) = this.read(cx).active_account_id.clone() else {
            AccountStateResetCoordinator::hard_reset(reason, true, false, cx);
            Self::clear_active_state(this, cx);
            return;
        };

        AccountStateResetCoordinator::hard_reset(reason, true, true, cx);
        AccountStateResetCoordinator::remove_persisted_cache(&user_id, cx);
        AuthLocalStorage::remove_account(&user_id);
        let remaining = AuthLocalStorage::load_accounts();
        this.update(cx, |store, cx| {
            store.accounts = remaining.iter().map(Self::summary).collect();
            store.changed(cx);
        });

        if let Some(next) = remaining.iter().find(|account| Self::belongs_to_current_backend(account)) {
            Self::activate_persisted_account(this, next, true, false, cx);
            Self::refresh_taken_over_account_if_expired(this, next, cx);
        } else {
            Self::clear_active_state(this, cx);
        }
    }

    fn belongs_to_current_backend(account: &AuthPersistedAccount) -> bool {
        account.backend_id.clone().unwrap_or_else(HttpService::unstamped_backend_id) == HttpService::resolved_backend_id()
    }

    pub fn debug_first_account(on_backend: &str) -> Option<AuthPersistedAccount> {
        if !cfg!(any(debug_assertions, feature = "sai_debug_tools")) {
            return None;
        }
        AuthLocalStorage::load_accounts()
            .into_iter()
            .find(|account| account.backend_id.clone().unwrap_or_else(HttpService::unstamped_backend_id) == on_backend)
    }

    pub fn debug_detach_active_account(this: &Entity<Self>, cx: &mut App) {
        if !cfg!(any(debug_assertions, feature = "sai_debug_tools")) {
            return;
        }
        if this.read(cx).active_account_id.is_none() {
            return;
        }
        AccountStateResetCoordinator::hard_reset(AccountStateResetReason::AccountChanged, true, true, cx);
        AuthLocalStorage::clear_active_account_pointer();
        Self::clear_active_state(this, cx);
    }

    fn activate_persisted_account(
        this: &Entity<Self>,
        account: &AuthPersistedAccount,
        update_last_used: bool,
        account_switch: bool,
        cx: &mut App,
    ) {
        let now = if update_last_used { Self::now_milliseconds() } else { account.last_used_at };
        let (was_authenticated, active_account_id) = {
            let store = this.read(cx);
            (store.is_authenticated, store.active_account_id.clone())
        };
        let switches_identity = active_account_id.as_deref().is_some_and(|active| active != account.user.id);
        if switches_identity {
            AccountStateResetCoordinator::hard_reset(AccountStateResetReason::AccountChanged, false, true, cx);
        }
        AccountStateResetCoordinator::prepare_for_activation(&account.user.id, switches_identity, cx);
        AuthLocalStorage::set_active_account(&account.user.id, now);
        AuthLocalStorage::stamp_backend_id_if_missing(&account.user.id, &HttpService::resolved_backend_id());

        let account_epoch = SaiAccountCacheScope::active_epoch();
        this.update(cx, |store, cx| {
            store.active_user_dto = Some(account.user.clone());
            store.active_account_id = Some(account.user.id.clone());
            store.account_switch_identity = account_switch.then(|| (account.user.id.clone(), account_epoch));
            store.is_account_blocked = account.user.is_blocked == Some(true);
            tracing::warn!(target: "riseonly::X453", "session profile from stored account badge={:?}", account.user.premium_badge_emoji);
            store.profile = Some(account.user.as_profile_model());
            store.tokens_and_other_data = TokensAndOtherData {
                access_token: account.tokens.access_token.clone(),
                refresh_token: account.tokens.refresh_token.clone(),
                session_id: account.session_id.clone(),
            };
            store.is_authenticated = true;
            store.lands_on_sign_in = false;
            store.reload_accounts();
            store.changed(cx);
        });
        MaintenanceGate::shared(cx).update(cx, |gate, cx| gate.viewer_did_change(false, cx));
        AuthEntryOrder::record_signed_in();
        Self::warm_account_avatar(
            account.user.avatar_url.clone().or_else(|| account.user.more.as_ref().and_then(|more| more.logo.clone())),
            cx,
        );
        if let Some(hooks) = auth_services_share_hooks() {
            (hooks.synchronize_credential)(false);
        }

        if was_authenticated {
            PushNotificationManagerHooks::prepare_for_account_switch(cx);
        } else {
            PushNotificationManagerHooks::activate_for_authenticated_account(cx);
        }
        let access_token = account.tokens.access_token.clone();
        SaiWebSocketManager::shared(cx).update(cx, |manager, cx| {
            if AccessTokenUserId::is_expired(&access_token, ACCESS_TOKEN_LEEWAY_SECONDS) {
                manager.connect(&access_token, cx);
            } else {
                manager.reconnect_with_access_token(&access_token, cx);
            }
        });
    }

    fn clear_active_state(this: &Entity<Self>, cx: &mut App) {
        AuthLocalStorage::clear_auth_payload();
        SaiAccountCacheScope::clear_active_account();
        PushNotificationManagerHooks::handle_account_cleared(cx);
        this.update(cx, |store, cx| {
            store.tokens_and_other_data = TokensAndOtherData::default();
            store.active_user_dto = None;
            store.active_account_id = None;
            store.account_switch_identity = None;
            store.is_account_blocked = false;
            store.profile = None;
            store.is_authenticated = false;
            store.accounts = Vec::new();
            store.changed(cx);
        });
        MaintenanceGate::shared(cx).update(cx, |gate, cx| gate.viewer_did_change(false, cx));
        if let Some(hooks) = auth_services_share_hooks() {
            (hooks.sign_out)(cx);
        }
    }

    pub fn mark_account_blocked(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(account_id) = self.active_account_id.clone() else {
            return false;
        };
        let Some(mut user) = self.active_user_dto.clone() else {
            return false;
        };
        if !self.is_authenticated || self.is_account_blocked || user.id != account_id {
            return false;
        }

        user.is_blocked = Some(true);
        AuthLocalStorage::update_user(&user);
        self.active_user_dto = Some(user);
        self.is_account_blocked = true;
        self.changed(cx);
        true
    }

    pub fn mark_account_block_lifted(&mut self, cx: &mut Context<Self>) {
        if !self.is_account_blocked {
            return;
        }
        let Some(mut user) = self.active_user_dto.clone() else {
            return;
        };
        user.is_blocked = Some(false);
        AuthLocalStorage::update_user(&user);
        self.active_user_dto = Some(user);
        self.is_account_blocked = false;
        self.changed(cx);
    }

    pub fn adopt_share_extension_credential_if_needed(&mut self, _cx: &mut Context<Self>) -> bool {
        false
    }

    fn reload_accounts(&mut self) {
        self.accounts = AuthLocalStorage::load_accounts().iter().map(Self::summary).collect();
    }

    fn summary(account: &AuthPersistedAccount) -> AuthAccountSummary {
        AuthAccountSummary {
            user_id: account.user.id.clone(),
            name: account.user.name.clone().unwrap_or_default(),
            tag: account.user.tag.clone().unwrap_or_default(),
            avatar_url: account
                .user
                .avatar_url
                .clone()
                .or_else(|| account.user.more.as_ref().and_then(|more| more.logo.clone())),
            is_premium: account.user.is_premium.unwrap_or(false),
            last_used_at: account.last_used_at,
            is_mirror: account.is_mirror,
        }
    }

    fn now_milliseconds() -> i64 {
        (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64
    }

    fn warm_account_avatar(source: Option<String>, cx: &mut App) {
        let Some(source) = source.map(|source| trimming_whitespaces_and_newlines(&source).to_owned()) else {
            return;
        };
        if source.is_empty() {
            return;
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |cx| {
            let _ = CleverImageBitmapLoader::load(
                &source,
                true,
                true,
                account_user_id,
                account_epoch,
                None,
                CleverImageCachePolicy::Standard,
                CleverImageWorkLane::Visible,
                cx,
            )
            .await;
        })
        .detach();
    }

    fn connect_web_socket_if_has_access_token(&self, cx: &mut Context<Self>) {
        let access_token = self.tokens_and_other_data.access_token.clone();
        cx.defer(move |cx| {
            SaiWebSocketManager::shared(cx)
                .update(cx, |manager, cx| manager.connect_if_needed(Some(access_token.as_str()), cx));
        });
    }

    fn digits(phone: &str) -> String {
        phone.chars().filter(|character| character.is_numeric()).collect()
    }

    fn bad_status(error: &anyhow::Error) -> Option<(i64, &[u8])> {
        error.chain().find_map(|cause| match cause.downcast_ref::<SaiFetchHttpError>() {
            Some(SaiFetchHttpError::BadStatus { code, body }) => Some((*code, body.as_slice())),
            _ => None,
        })
    }

    fn json_object(body: &[u8]) -> Option<Map<String, Value>> {
        match serde_json::from_slice::<Value>(body) {
            Ok(Value::Object(object)) => Some(object),
            _ => None,
        }
    }

    fn is_url_error(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| cause.is::<URLError>() || cause.is::<std::io::Error>())
    }
}
