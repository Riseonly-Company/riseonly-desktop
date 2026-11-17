use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context, Entity, Task};
use rise_canon::{rise_http_path, rise_request, rise_wire_methods};
use rise_engine::CancellationError;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch::SaiFetch;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_options::SaiFetchOptions;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::stores::http::http_service::HttpService;
use crate::modules::auth::engine::rise_web_login_approval_rpc::RiseWebLoginApprovalRPC;
use crate::modules::auth::stores::auth_actions::auth_types::SignInFailure;
use crate::modules::auth::stores::auth_actions::auth_wire_types::{
    AuthCheckTagBody, AuthCheckTagResponse, AuthLoginBody, AuthLoginResponse, AuthLogoutResponse,
    AuthManagedSessionResponse, AuthRegisterBody, AuthRegisterResponse, AuthResetPasswordResponse,
    AuthSendCodeResponse,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

pub struct AuthActionsStore {
    managed_login_sai: Rc<SaiTypedSlice<AuthManagedSessionResponse>>,
    pub logout_sai: Rc<SaiTypedSlice<AuthLogoutResponse>>,
}

impl AuthActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            managed_login_sai: SaiTypedSlice::new(cx),
            logout_sai: SaiTypedSlice::new(cx),
        }
    }

    pub fn login_action(
        &self,
        body: AuthLoginBody,
        should_apply_response: Rc<dyn Fn() -> bool>,
        on_finish: Option<Rc<dyn Fn(Result<AuthLoginResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !should_apply_response() {
            return;
        }
        let requested = SaiFetch::request_encodable_decoding::<AuthLoginResponse, AuthLoginBody>(
            &HttpService::api_route(rise_http_path::auth::LOGIN),
            &body,
            None,
            SaiFetchOptions::with_cache_key_cache(&SaiCache::new(["loginAction"]), true, false),
            cx,
        );
        match requested {
            Ok(handle) => {
                let success_should_apply = Rc::clone(&should_apply_response);
                let success_on_finish = on_finish.clone();
                SaiFetch::handler(
                    &handle,
                    move |response: AuthLoginResponse, cx: &mut App| {
                        if !success_should_apply() {
                            return;
                        }
                        AuthServicesStore::shared(cx)
                            .update(cx, |service, cx| service.handle_sign_in_success(response.clone(), cx));
                        if let Some(on_finish) = &success_on_finish {
                            on_finish(Ok(response), cx);
                        }
                    },
                    move |error: &anyhow::Error, cx: &mut App| {
                        if !should_apply_response() {
                            return;
                        }
                        AuthServicesStore::shared(cx)
                            .update(cx, |service, cx| service.handle_sign_in_error(error, cx));
                        if let Some(on_finish) = &on_finish {
                            on_finish(Err(error), cx);
                        }
                    },
                    cx,
                );
            }
            Err(error) => {
                if !should_apply_response() {
                    return;
                }
                AuthServicesStore::shared(cx)
                    .update(cx, |service, cx| service.handle_sign_in_error(&error, cx));
                if let Some(on_finish) = &on_finish {
                    on_finish(Err(&error), cx);
                }
            }
        }
    }

    pub fn exchange_managed_login_action(
        &self,
        code: &str,
        on_finish: Option<Rc<dyn Fn(Result<AuthManagedSessionResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let managed_login_sai = Rc::clone(&self.managed_login_sai);
        let success_on_finish = on_finish.clone();
        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(response) = managed_login_sai.data() else {
                let error = anyhow::Error::new(SignInFailure::MissingPayload);
                AuthServicesStore::shared(cx)
                    .update(cx, |service, cx| service.handle_managed_login_error(&error, cx));
                if let Some(on_finish) = &success_on_finish {
                    on_finish(Err(&error), cx);
                }
                return;
            };
            AuthServicesStore::shared(cx)
                .update(cx, |service, cx| service.handle_managed_login_success(response.clone(), cx));
            if let Some(on_finish) = &success_on_finish {
                on_finish(Ok(response), cx);
            }
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            AuthServicesStore::shared(cx)
                .update(cx, |service, cx| service.handle_managed_login_error(error, cx));
            if let Some(on_finish) = &on_finish {
                on_finish(Err(error), cx);
            }
        });
        SaiWs::request_slice(
            &self.managed_login_sai,
            rise_request::auth::ExchangeManagedLogin {
                code: Some(code.to_owned()),
                ..rise_request::auth::ExchangeManagedLogin::default()
            },
            SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
                service: rise_wire_methods::auth::SERVICE.to_owned(),
                method: rise_wire_methods::auth::EXCHANGE_MANAGED_LOGIN.to_owned(),
                ..SaiWsRequestOptionsInit::default()
            }),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn reset_password_action(
        &self,
        body: rise_request::auth::ResetPassword,
        should_apply_response: Rc<dyn Fn() -> bool>,
        on_finish: Option<Rc<dyn Fn(Result<AuthResetPasswordResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if !should_apply_response() {
            return;
        }
        let requested = SaiFetch::request_encodable_decoding::<
            AuthResetPasswordResponse,
            rise_request::auth::ResetPassword,
        >(
            &HttpService::api_route(rise_http_path::auth::RESET_PASSWORD),
            &body,
            None,
            SaiFetchOptions::with_cache_key_cache(&SaiCache::new(["resetPasswordAction"]), true, false),
            cx,
        );
        match requested {
            Ok(handle) => {
                let success_should_apply = Rc::clone(&should_apply_response);
                let success_on_finish = on_finish.clone();
                SaiFetch::handler(
                    &handle,
                    move |response: AuthResetPasswordResponse, cx: &mut App| {
                        if !success_should_apply() {
                            return;
                        }
                        AuthServicesStore::shared(cx)
                            .update(cx, |service, cx| service.handle_password_reset_success(response.clone(), cx));
                        if let Some(on_finish) = &success_on_finish {
                            on_finish(Ok(response), cx);
                        }
                    },
                    move |error: &anyhow::Error, cx: &mut App| {
                        if !should_apply_response() {
                            return;
                        }
                        AuthServicesStore::shared(cx)
                            .update(cx, |service, cx| service.handle_password_reset_error(error, cx));
                        if let Some(on_finish) = &on_finish {
                            on_finish(Err(error), cx);
                        }
                    },
                    cx,
                );
            }
            Err(error) => {
                if !should_apply_response() {
                    return;
                }
                AuthServicesStore::shared(cx)
                    .update(cx, |service, cx| service.handle_password_reset_error(&error, cx));
                if let Some(on_finish) = &on_finish {
                    on_finish(Err(&error), cx);
                }
            }
        }
    }

    pub fn send_code_action(
        &self,
        body: rise_request::auth::SendCode,
        assumes_delivery_on_failure: bool,
        on_finish: Option<Rc<dyn Fn(Result<AuthSendCodeResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let phone = body.phone_number.clone().unwrap_or_default();
        let purpose = body.purpose.clone();
        let requested = SaiFetch::request_encodable_decoding::<AuthSendCodeResponse, rise_request::auth::SendCode>(
            &HttpService::api_route(rise_http_path::auth::SEND_CODE),
            &body,
            None,
            SaiFetchOptions::with_cache_key_cache(&SaiCache::new(["sendCodeAction"]), true, false),
            cx,
        );
        match requested {
            Ok(handle) => {
                let success_phone = phone.clone();
                let success_purpose = purpose.clone();
                let success_on_finish = on_finish.clone();
                SaiFetch::handler(
                    &handle,
                    move |response: AuthSendCodeResponse, cx: &mut App| {
                        AuthServicesStore::shared(cx).update(cx, |service, cx| {
                            service.handle_send_code_success(
                                response.clone(),
                                &success_phone,
                                success_purpose.as_deref(),
                                cx,
                            );
                        });
                        if let Some(on_finish) = &success_on_finish {
                            on_finish(Ok(response), cx);
                        }
                    },
                    move |error: &anyhow::Error, cx: &mut App| {
                        if assumes_delivery_on_failure {
                            let services = AuthServicesStore::shared(cx);
                            let previous = services.read(cx).code_delivery().cloned();
                            if let Some(assumed) =
                                AuthServicesStore::assumed_sms_delivery(&body, error, previous.as_ref())
                            {
                                Sai::log(
                                    "Auth",
                                    || "send-code: not delivered, opening the code step as sent",
                                    SaiLogKind::Network,
                                    cx,
                                );
                                services.update(cx, |service, cx| {
                                    service.handle_send_code_success(assumed.clone(), &phone, purpose.as_deref(), cx);
                                });
                                if let Some(on_finish) = &on_finish {
                                    on_finish(Ok(assumed), cx);
                                }
                                return;
                            }
                        }
                        AuthServicesStore::shared(cx).update(cx, |service, cx| {
                            service.handle_send_code_error(error, &phone, purpose.as_deref(), cx);
                        });
                        if let Some(on_finish) = &on_finish {
                            on_finish(Err(error), cx);
                        }
                    },
                    cx,
                );
            }
            Err(error) => {
                AuthServicesStore::shared(cx).update(cx, |service, cx| {
                    service.handle_send_code_error(&error, &phone, purpose.as_deref(), cx);
                });
                if let Some(on_finish) = &on_finish {
                    on_finish(Err(&error), cx);
                }
            }
        }
    }

    pub fn check_registration_tag_action(
        &self,
        body: AuthCheckTagBody,
        on_finish: Rc<dyn Fn(Result<AuthCheckTagResponse, &anyhow::Error>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let cache = SaiCache::new(["authCheckRegistrationTag", body.tag.as_deref().unwrap_or("")]);
        let requested = SaiFetch::request_encodable_decoding::<AuthCheckTagResponse, AuthCheckTagBody>(
            &HttpService::api_route(rise_http_path::auth::CHECK_TAG),
            &body,
            None,
            SaiFetchOptions::with_cache_key_cache(&cache, true, true),
            cx,
        );
        match requested {
            Ok(handle) => {
                let success_on_finish = Rc::clone(&on_finish);
                SaiFetch::handler(
                    &handle,
                    move |response: AuthCheckTagResponse, cx: &mut App| success_on_finish(Ok(response), cx),
                    move |error: &anyhow::Error, cx: &mut App| on_finish(Err(error), cx),
                    cx,
                );
            }
            Err(error) => on_finish(Err(&error), cx),
        }
    }

    pub fn register_action(
        &self,
        body: AuthRegisterBody,
        on_finish: Option<Rc<dyn Fn(Result<AuthRegisterResponse, &anyhow::Error>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let requested = SaiFetch::request_encodable_decoding::<AuthRegisterResponse, AuthRegisterBody>(
            &HttpService::api_route(rise_http_path::auth::REGISTER),
            &body,
            None,
            SaiFetchOptions::with_cache_key_cache(&SaiCache::new(["registerAction"]), true, false),
            cx,
        );
        match requested {
            Ok(handle) => {
                let success_on_finish = on_finish.clone();
                SaiFetch::handler(
                    &handle,
                    move |response: AuthRegisterResponse, cx: &mut App| {
                        AuthServicesStore::shared(cx)
                            .update(cx, |service, cx| service.handle_register_success(response.clone(), cx));
                        if let Some(on_finish) = &success_on_finish {
                            on_finish(Ok(response), cx);
                        }
                    },
                    move |error: &anyhow::Error, cx: &mut App| {
                        AuthServicesStore::shared(cx)
                            .update(cx, |service, cx| service.handle_register_error(error, cx));
                        if let Some(on_finish) = &on_finish {
                            on_finish(Err(error), cx);
                        }
                    },
                    cx,
                );
            }
            Err(error) => {
                AuthServicesStore::shared(cx)
                    .update(cx, |service, cx| service.handle_register_error(&error, cx));
                if let Some(on_finish) = &on_finish {
                    on_finish(Err(&error), cx);
                }
            }
        }
    }

    pub fn approve_web_login_action(
        &self,
        login_token: &str,
        approve: bool,
        account_id: &str,
        account_epoch: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        if !SaiAccountCacheScope::is_current_account_epoch(Some(account_id), account_epoch) {
            return Task::ready(Err(CancellationError.into()));
        }

        let prepared = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        let login_token = login_token.to_owned();
        let account_id = account_id.to_owned();
        let account_epoch = account_epoch.map(str::to_owned);
        cx.spawn(async move |_, _| {
            let engine = prepared.await?;
            if engine.account.account_id != account_id
                || account_epoch.as_deref() != Some(engine.account.epoch.as_str())
                || !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), account_epoch.as_deref())
            {
                return Err(CancellationError.into());
            }

            RiseWebLoginApprovalRPC { engine }.send(&login_token, approve).await
        })
    }

    pub fn logout_action(
        &self,
        body: rise_request::auth::Logout,
        on_finish: Rc<dyn Fn(bool, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::auth::SERVICE.to_owned(),
            method: rise_wire_methods::auth::LOGOUT.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            persist_cache_to_disk: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let success_on_finish = Rc::clone(&on_finish);
        let on_success: SaiWsOnSuccess =
            Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| success_on_finish(true, cx));
        let on_error: SaiWsOnError = Rc::new(move |_: &anyhow::Error, cx: &mut App| on_finish(false, cx));
        SaiWs::request_slice(&self.logout_sai, body, options, Some(on_success), Some(on_error), cx);
    }
}
