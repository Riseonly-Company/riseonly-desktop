use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_canon::rise_key::session as session_key;
use rise_i18n::LanguageStore;

use super::session_wire_types::{DeleteUserSessionsGatewayDTO, GetUserSessionsGatewayDTO};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::session::stores::session_services::session_services::SessionServicesStore;

pub struct SessionActionsStore {
    pub get_sessions_sai: Rc<SaiTypedSlice<GetUserSessionsGatewayDTO>>,
    pub delete_sessions_sai: Rc<SaiTypedSlice<DeleteUserSessionsGatewayDTO>>,
}

impl SessionActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            get_sessions_sai: SaiTypedSlice::new(cx),
            delete_sessions_sai: SaiTypedSlice::new(cx),
        }
    }

    pub fn get_sessions_action(
        &self,
        relative_id: Option<String>,
        limit: i64,
        fetch_if_have_data: bool,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::signed_in_user_id(cx) else {
            return;
        };

        let cache_key = SessionServicesStore::sessions_cache_key(&user_id);
        let body = rise_canon::rise_request::session::GetUserSessions {
            limit: Some(limit),
            locale: Some(LanguageStore::shared(cx).code().to_owned()),
            relative_id: relative_id.clone(),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::session::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::session::GET_USER_SESSIONS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: fetch_if_have_data,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: relative_id.is_none(),
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(session_key::SESSIONS.to_owned()),
            path_to_id: Some(session_key::ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id,
                has_more_path: Some(session_key::IS_HAVE_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(|_payload: Option<Box<dyn Any>>, cx: &mut App| {
            SessionServicesStore::shared(cx)
                .update(cx, |service, cx| service.get_sessions_success_handler(cx));
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            SessionServicesStore::shared(cx)
                .update(cx, |service, cx| service.get_sessions_error_handler(error, cx));
        });

        SaiWs::request_slice(
            &self.get_sessions_sai,
            body,
            options,
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn delete_sessions_action(
        &self,
        session_id_to_terminate: Option<String>,
        is_all: bool,
        cx: &mut App,
    ) {
        let Some(user_id) = Self::signed_in_user_id(cx) else {
            return;
        };

        let body = rise_canon::rise_request::session::DeleteUserSessionsAdvanced {
            is_all: Some(is_all),
            locale: Some(LanguageStore::shared(cx).code().to_owned()),
            session_id_to_terminate: Some(session_id_to_terminate.clone().unwrap_or_default()),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::session::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::session::DELETE_USER_SESSIONS_ADVANCED.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: true,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            SessionServicesStore::delete_sessions_success_handler(
                payload,
                &user_id,
                session_id_to_terminate.as_deref(),
                is_all,
                cx,
            );
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            SessionServicesStore::shared(cx)
                .update(cx, |service, cx| service.delete_sessions_error_handler(error, cx));
        });

        SaiWs::request_slice(
            &self.delete_sessions_sai,
            body,
            options,
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    fn signed_in_user_id(cx: &mut App) -> Option<String> {
        let auth = AuthServicesStore::shared(cx);
        let user_id = auth.read(cx).profile()?.id.clone();
        (!user_id.is_empty()).then_some(user_id)
    }
}
