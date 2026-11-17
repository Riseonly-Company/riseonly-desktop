use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity};
use rise_canon::{RiseRequestBody, RiseTimingCanon, rise_key, rise_request};
use rise_core::foundation::foundation_character::{Character, characters};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{LanguageStore, tr};
use rise_platform::HostOs;
use tokio::sync::oneshot;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::push_notifications::push_notification_manager_hooks::PushNotificationManagerHooks;
use crate::core::rise_engine_bridge::rise_canon_fields::RiseCanonFields;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierStore, NotifierType,
};
use crate::modules::auth::services::account_state_reset_coordinator::AccountStateResetReason;
use crate::modules::auth::services::app_attest_client::AppAttestClient;
use crate::modules::auth::shared::device_identity::DeviceIdentity;
use crate::modules::auth::stores::auth_actions::auth_actions::AuthActionsStore;
use crate::modules::auth::stores::auth_actions::auth_types::{AuthCodeChannel, AuthCodeDelivery};
use crate::modules::auth::stores::auth_actions::auth_wire_types::{
    AuthCheckTagBody, AuthCheckTagResponse, AuthLoginBody, AuthLoginResponse, AuthRegisterBody,
    AuthRegisterResponse, AuthResetPasswordResponse, AuthSendCodeResponse,
};
use crate::modules::auth::stores::auth_interactions::auth_interactions_moderation_hooks::AuthInteractionsModerationHooks;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

struct LogoutCompletionGate {
    continuation: RefCell<Option<oneshot::Sender<()>>>,
}

impl LogoutCompletionGate {
    fn new(continuation: oneshot::Sender<()>) -> Self {
        Self {
            continuation: RefCell::new(Some(continuation)),
        }
    }

    fn finish(&self) {
        if let Some(continuation) = self.continuation.borrow_mut().take() {
            let _ = continuation.send(());
        }
    }
}

pub enum AuthLogoutWorkflow {}

impl AuthLogoutWorkflow {
    pub const DEFAULT_TIMEOUT_NANOSECONDS: u64 = 2_000_000_000;

    pub async fn run(
        is_connected: bool,
        timeout_nanoseconds: u64,
        send: impl FnOnce(Rc<dyn Fn(bool, &mut App)>, &mut App),
        local_reset: impl FnOnce(&mut App),
        cx: &mut AsyncApp,
    ) {
        Self::await_transport(is_connected, timeout_nanoseconds, send, cx).await;
        cx.update(local_reset);
    }

    async fn await_transport(
        is_connected: bool,
        timeout_nanoseconds: u64,
        send: impl FnOnce(Rc<dyn Fn(bool, &mut App)>, &mut App),
        cx: &mut AsyncApp,
    ) {
        if !is_connected {
            return;
        }

        let (continuation, finished) = oneshot::channel::<()>();
        let gate = Rc::new(LogoutCompletionGate::new(continuation));
        let send_gate = Rc::clone(&gate);
        cx.update(|cx| send(Rc::new(move |_: bool, _: &mut App| send_gate.finish()), cx));
        let timer = cx
            .background_executor()
            .timer(Duration::from_nanos(timeout_nanoseconds));
        cx.spawn(async move |_| {
            timer.await;
            gate.finish();
        })
        .detach();
        let _ = finished.await;
    }
}

pub struct AuthInteractionsStore {
    is_logging_out: bool,
}

impl AuthInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { is_logging_out: false }
    }

    fn actions(cx: &mut App) -> Entity<AuthActionsStore> {
        AuthActionsStore::shared(cx)
    }

    fn service(cx: &mut App) -> Entity<AuthServicesStore> {
        AuthServicesStore::shared(cx)
    }

    pub fn is_logging_out(&self) -> bool {
        self.is_logging_out
    }

    pub fn logout(&mut self, cx: &mut Context<Self>) {
        if self.is_logging_out {
            return;
        }
        self.is_logging_out = true;
        cx.notify();

        let service = Self::service(cx);
        let tokens = service.read(cx).tokens_and_other_data().clone();
        let body = rise_request::auth::Logout {
            device_token: PushNotificationManagerHooks::device_token(cx),
            platform: Some(DeviceIdentity::platform(HostOs::current()).to_owned()),
            refresh_token: Some(tokens.refresh_token),
            session_id: Some(tokens.session_id),
        };

        cx.spawn(async move |this, cx| {
            let is_connected = cx.update(|cx| SaiWebSocketManager::shared(cx).read(cx).is_connected());
            AuthLogoutWorkflow::run(
                is_connected,
                AuthLogoutWorkflow::DEFAULT_TIMEOUT_NANOSECONDS,
                move |finish: Rc<dyn Fn(bool, &mut App)>, cx: &mut App| {
                    Self::actions(cx).update(cx, |actions, cx| actions.logout_action(body, finish, cx));
                },
                move |cx: &mut App| {
                    Self::service(cx).update(cx, |service, cx| {
                        service.full_clear(AccountStateResetReason::UserLogout, cx);
                    });
                    let _ = this.update(cx, |store, cx| {
                        store.is_logging_out = false;
                        cx.notify();
                    });
                },
                cx,
            )
            .await;
        })
        .detach();
    }

    pub fn handle_account_blocked(&mut self, cx: &mut Context<Self>) {
        if !Self::service(cx).update(cx, |service, cx| service.mark_account_blocked(cx)) {
            return;
        }
        Notifier::send(
            tr("blocked_account_title"),
            NotifierSendOptions {
                content: Some(tr("block_type_full_desc")),
                r#type: NotifierType::Warning,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("nosign".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn handle_session_refused(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if !service.read(cx).is_authenticated() {
            return;
        }
        service.update(cx, |service, cx| service.end_refused_session(cx));
        cx.defer(|cx| {
            Notifier::send(
                tr("rpc_error_shutdown"),
                NotifierSendOptions {
                    r#type: NotifierType::Warning,
                    position: NotifierPosition::Bottom,
                    icon: NotifierIcon::System("person.crop.circle.badge.xmark".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
    }

    pub fn reconcile_account_block(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        let state = service.read(cx);
        if !state.is_account_blocked() {
            return;
        }
        let Some(account_id) = state.active_account_id().map(str::to_owned) else {
            return;
        };
        let account_epoch = SaiAccountCacheScope::active_epoch();
        cx.spawn(async move |this, cx| {
            let still_blocked = cx.update(|cx| {
                let state = Self::service(cx).read(cx);
                state.is_account_blocked()
                    && state.active_account_id() == Some(account_id.as_str())
                    && SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), account_epoch.as_deref())
            });
            if !still_blocked {
                return;
            }
            let refreshed = cx.update(AuthInteractionsModerationHooks::refresh_blocks_awaiting_reply);
            let Some(blocks) = refreshed.await else {
                return;
            };
            let still_blocked = cx.update(|cx| {
                let state = Self::service(cx).read(cx);
                state.is_account_blocked()
                    && state.active_account_id() == Some(account_id.as_str())
                    && SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), account_epoch.as_deref())
            });
            if !still_blocked {
                return;
            }
            let now = Date::now().ms();
            let full_block_in_force = blocks.iter().any(|block| {
                block.is_full && (block.block_end_timestamp == 0 || block.block_end_timestamp > now)
            });
            if !full_block_in_force {
                let _ = this.update(cx, |store, cx| store.account_block_lifted(cx));
            }
        })
        .detach();
    }

    pub fn account_block_lifted(&mut self, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if !service.read(cx).is_account_blocked() {
            return;
        }
        service.update(cx, |service, cx| service.mark_account_block_lifted(cx));
        let token = service.read(cx).tokens_and_other_data().access_token.clone();
        if !token.is_empty() {
            SaiWebSocketManager::shared(cx).update(cx, |manager, cx| manager.reconnect_with_access_token(&token, cx));
        }
    }

    pub fn submit_sign_in(
        &mut self,
        number: &str,
        password: &str,
        calling_code: &str,
        should_apply_response: Rc<dyn Fn() -> bool>,
        on_finish: Option<Rc<dyn Fn(Result<AuthLoginResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !should_apply_response() {
            return;
        }
        Self::service(cx).update(cx, |service, cx| service.clear_auth_flow_error(cx));
        let phone = Self::compose_phone_number(calling_code, number);
        let password = password.to_owned();
        cx.spawn(async move |_, cx| {
            let attestation = AppAttestClient::envelope().await;
            if !should_apply_response() {
                return;
            }
            cx.update(|cx| {
                let body = AuthLoginBody::build(
                    phone,
                    password,
                    PushNotificationManagerHooks::device_token(cx),
                    attestation,
                );
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.login_action(body, should_apply_response, on_finish, cx);
                });
            });
        })
        .detach();
    }

    pub fn enter_managed_account(&mut self, code: &str, cx: &mut Context<Self>) {
        let code = trimming_whitespaces_and_newlines(code);
        if code.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.exchange_managed_login_action(code, None, cx));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_send_code(
        &mut self,
        number: &str,
        calling_code: &str,
        purpose: Option<&str>,
        channel: Option<AuthCodeChannel>,
        assumes_delivery_on_failure: bool,
        on_finish: Option<Rc<dyn Fn(Result<AuthSendCodeResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        Self::service(cx).update(cx, |service, cx| service.clear_auth_flow_error(cx));
        let body = Self::send_code_body(number, calling_code, purpose, channel, cx);
        Self::actions(cx).update(cx, |actions, cx| {
            actions.send_code_action(body, assumes_delivery_on_failure, on_finish, cx);
        });
    }

    pub fn send_code_body(
        number: &str,
        calling_code: &str,
        purpose: Option<&str>,
        channel: Option<AuthCodeChannel>,
        cx: &mut App,
    ) -> rise_request::auth::SendCode {
        rise_request::auth::SendCode {
            channel_preference: channel.and_then(|channel| {
                Self::send_code_value(
                    channel.raw_value(),
                    &format!("/{}", rise_key::auth::CHANNEL_PREFERENCE),
                )
            }),
            idempotency_key: Some(Uuid::new_v4().uuid_string().to_lowercase()),
            language: Some(LanguageStore::shared(cx).code().to_owned()),
            phone_number: Some(Self::compose_phone_number(calling_code, number)),
            purpose: purpose
                .and_then(|purpose| Self::send_code_value(purpose, &format!("/{}", rise_key::auth::PURPOSE))),
        }
    }

    pub fn submit_registration_code(
        &mut self,
        number: &str,
        calling_code: &str,
        on_finish: Option<Rc<dyn Fn(Result<AuthSendCodeResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        self.submit_send_code(number, calling_code, Some("registration"), None, true, on_finish, cx);
    }

    pub fn submit_password_reset_code(
        &mut self,
        number: &str,
        calling_code: &str,
        on_finish: Option<Rc<dyn Fn(Result<AuthSendCodeResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        Self::service(cx).update(cx, |service, cx| service.begin_password_reset(cx));
        self.submit_send_code(number, calling_code, Some("password_reset"), None, true, on_finish, cx);
    }

    pub fn resend_code(
        &mut self,
        number: &str,
        calling_code: &str,
        channel: Option<AuthCodeChannel>,
        on_finish: Option<Rc<dyn Fn(Result<AuthSendCodeResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(delivery) = Self::service(cx).read(cx).code_delivery().cloned() else {
            return false;
        };
        if !delivery.offers_resend() || Date::now() < delivery.resend_available_at {
            return false;
        }
        self.submit_send_code(
            number,
            calling_code,
            delivery.purpose.as_deref(),
            channel.or(delivery.channel),
            true,
            on_finish,
            cx,
        );
        true
    }

    pub fn switch_to_telegram(
        &mut self,
        number: &str,
        calling_code: &str,
        on_finish: Option<Rc<dyn Fn(Result<AuthSendCodeResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(delivery) = Self::service(cx).read(cx).code_delivery().cloned() else {
            return false;
        };
        let Some(body) = Self::telegram_code_body(&delivery, number, calling_code, cx) else {
            return false;
        };
        Self::service(cx).update(cx, |service, cx| service.clear_auth_flow_error(cx));
        Self::actions(cx).update(cx, |actions, cx| actions.send_code_action(body, false, on_finish, cx));
        true
    }

    pub fn telegram_code_body(
        delivery: &AuthCodeDelivery,
        number: &str,
        calling_code: &str,
        cx: &mut App,
    ) -> Option<rise_request::auth::SendCode> {
        if !delivery.offers_telegram {
            return None;
        }
        Some(Self::send_code_body(
            number,
            calling_code,
            delivery.purpose.as_deref(),
            Some(AuthCodeChannel::TelegramBot),
            cx,
        ))
    }

    pub fn reopen_telegram_bot(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.reopen_telegram_redirect(cx));
    }

    fn send_code_value(value: &str, pointer: &str) -> Option<String> {
        let allowed = RiseCanonFields::field(<rise_request::auth::SendCode as RiseRequestBody>::CONTRACT, pointer)
            .map_or(&[] as &[&str], |field| field.allowed_values);
        allowed.iter().any(|candidate| *candidate == value).then(|| value.to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_password_reset(
        &mut self,
        number: &str,
        calling_code: &str,
        code: &str,
        password: &str,
        should_apply_response: Rc<dyn Fn() -> bool>,
        on_finish: Option<Rc<dyn Fn(Result<AuthResetPasswordResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !should_apply_response() {
            return;
        }
        Self::service(cx).update(cx, |service, cx| service.clear_auth_flow_error(cx));
        let phone = Self::compose_phone_number(calling_code, number);
        let body = rise_request::auth::ResetPassword {
            code: Some(code.to_owned()),
            password: Some(password.to_owned()),
            phone_number: Some(phone),
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.reset_password_action(body, should_apply_response, on_finish, cx);
        });
    }

    pub fn announce_password_reset_succeeded(&mut self, cx: &mut Context<Self>) {
        NotifierStore::shared(cx).update(cx, |notifier, cx| {
            notifier.send(
                tr("auth_reset_password_done"),
                NotifierSendOptions {
                    r#type: NotifierType::Success,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
    }

    pub fn cancel_password_reset(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.cancel_password_reset(cx));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_register(
        &mut self,
        name: &str,
        tag: &str,
        number: &str,
        password: &str,
        code: &str,
        calling_code: &str,
        birth_date: Option<&str>,
        invite_code: Option<&str>,
        on_finish: Option<Rc<dyn Fn(Result<AuthRegisterResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        Self::service(cx).update(cx, |service, cx| service.clear_auth_flow_error(cx));
        let phone = Self::compose_phone_number(calling_code, number);
        let name = trimming_whitespaces_and_newlines(name).to_owned();
        let tag = trimming_whitespaces_and_newlines(tag).to_lowercase();
        let code = code.to_owned();
        let password = password.to_owned();
        let birth_date = birth_date.map(str::to_owned);
        let invite_code = invite_code.map(|invite_code| trimming_whitespaces_and_newlines(invite_code).to_owned());
        cx.spawn(async move |_, cx| {
            let attestation = AppAttestClient::envelope().await;
            cx.update(|cx| {
                let body = AuthRegisterBody::build(
                    phone,
                    name,
                    tag,
                    code,
                    password,
                    birth_date,
                    PushNotificationManagerHooks::device_token(cx),
                    invite_code,
                    attestation,
                );
                Self::actions(cx).update(cx, |actions, cx| actions.register_action(body, on_finish, cx));
            });
        })
        .detach();
    }

    pub fn schedule_registration_tag_check(
        &mut self,
        tag: &str,
        on_finish: Rc<dyn Fn(Result<AuthCheckTagResponse, &anyhow::Error>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let normalized = trimming_whitespaces_and_newlines(tag).to_lowercase();
        SaiDebouncer::debounced_action_delay_milliseconds(
            "availability",
            Box::new(move |cx: &mut App| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.check_registration_tag_action(
                        AuthCheckTagBody {
                            tag: Some(normalized),
                        },
                        on_finish,
                        cx,
                    );
                });
            }),
            RiseTimingCanon::debounce("registration_tag_check").client_debounce_ms,
            "authRegistrationTag",
            1,
            None,
            cx,
        );
    }

    pub fn cancel_registration_tag_check(&mut self, cx: &mut Context<Self>) {
        SaiDebouncer::cancel_all_in_group("authRegistrationTag", cx);
    }

    pub fn complete_telegram_redirect(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.confirm_bot_registration_completed(cx));
    }

    pub fn cancel_verification_code_entry(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.cancel_verification_code_entry(cx));
    }

    pub fn present_auth_flow_error(&mut self, message: &str, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.present_auth_flow_error(message, cx));
    }

    pub fn present_localized_auth_flow_error(&mut self, text: &str, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.present_localized_auth_flow_error(text, cx));
    }

    pub fn clear_auth_flow_error(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.clear_auth_flow_error(cx));
    }

    pub fn reset_auth_flow_presentation(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.reset_auth_flow_presentation(cx));
    }

    pub fn compose_phone_number(calling_code: &str, number: &str) -> String {
        let cc: String = characters(calling_code)
            .filter(Character::is_number)
            .map(|character| character.as_str())
            .collect();
        let digits: String = characters(number)
            .filter(Character::is_number)
            .map(|character| character.as_str())
            .collect();
        cc + &digits
    }
}
