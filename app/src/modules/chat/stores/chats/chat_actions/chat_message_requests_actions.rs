use std::any::Any;
use std::rc::Rc;

use anyhow::anyhow;
use gpui::App;
use rise_canon::ChatMessageRequestAction;
use rise_canon::rise_key::chat as key;
use rise_canon::rise_key::message as message_key;
use rise_canon::rise_request;
use rise_canon::rise_wire_methods;

use super::chat_actions::{ChatActionsStore, effective_user_id_for_wire};
use super::chat_folders_actions::chat_call_options;
use super::chat_message_requests_wire_types::{DeleteAllMessageRequestsGatewayDTO, ResolveMessageRequestGatewayDTO};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationCursorFallback, SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiWsScreenRefresh;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

const SCREEN_MESSAGE_REQUESTS: &str = "MessageRequests";

impl ChatActionsStore {
    pub fn get_message_requests_action(
        &self,
        relative_id: Option<&str>,
        reload_if_cached: bool,
        reflect_pending_in_ui: bool,
        cx: &mut App,
    ) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };

        let is_first_page = relative_id.is_none();
        let body = rise_request::chat::GetMessageRequests {
            limit: Some(ChatServicesStore::message_requests_limit()),
            relative_id: relative_id.map(str::to_owned),
        };
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::GET_MESSAGE_REQUESTS.to_owned(),
            cache_key: Some(ChatServicesStore::message_requests_cache_key(&user_id)),
            reload_if_cached: !is_first_page || reload_if_cached,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: is_first_page && reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::Persistent),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(key::CHATS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: relative_id.map(str::to_owned),
                response_relative_id_fallback: Some(SaiWsPaginationCursorFallback::LastItemId),
                has_more_path: Some(message_key::HAS_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            screen_refresh: is_first_page
                .then(|| SaiWsScreenRefresh::new(true, vec![SCREEN_MESSAGE_REQUESTS.to_owned()], None)),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(|_payload: Option<Box<dyn Any>>, _cx: &mut App| {});
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("get_message_requests error: {error}"), SaiLogKind::Network, cx);
        });
        SaiWs::request_slice(&self.get_message_requests_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn resolve_message_request_action(
        &self,
        chat_id: &str,
        action: ChatMessageRequestAction,
        on_success: Option<Rc<dyn Fn(ResolveMessageRequestGatewayDTO, &mut App)>>,
        on_error: Option<SaiWsOnError>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() {
            return;
        }
        let body = rise_request::chat::ResolveMessageRequest::new(action.wire().to_owned(), chat_id.to_owned());

        let success: SaiWsOnSuccess = {
            let on_error = on_error.clone();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let Some(dto) = payload.and_then(|payload| payload.downcast::<ResolveMessageRequestGatewayDTO>().ok())
                else {
                    if let Some(on_error) = on_error.as_ref() {
                        on_error(&anyhow!("Invalid message request response"), cx);
                    }
                    return;
                };
                if let Some(on_success) = on_success.as_ref() {
                    on_success(*dto, cx);
                }
            })
        };
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("resolve_message_request error: {error}"), SaiLogKind::Network, cx);
            if let Some(on_error) = on_error.as_ref() {
                on_error(error, cx);
            }
        });
        SaiWs::request_slice(
            &self.resolve_message_request_sai,
            body,
            chat_call_options(rise_wire_methods::chat::RESOLVE_MESSAGE_REQUEST, false, true),
            Some(success),
            Some(failure),
            cx,
        );
    }

    pub fn delete_all_message_requests_action(&self, on_error: Option<SaiWsOnError>, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() {
            return;
        }

        let success: SaiWsOnSuccess = {
            let on_error = on_error.clone();
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let is_valid =
                    payload.is_some_and(|payload| payload.is::<DeleteAllMessageRequestsGatewayDTO>());
                if !is_valid && let Some(on_error) = on_error.as_ref() {
                    on_error(&anyhow!("Invalid delete-all response"), cx);
                }
            })
        };
        let failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("delete_all_message_requests error: {error}"), SaiLogKind::Network, cx);
            if let Some(on_error) = on_error.as_ref() {
                on_error(error, cx);
            }
        });
        SaiWs::request_slice(
            &self.delete_all_message_requests_sai,
            rise_request::chat::DeleteAllMessageRequests::default(),
            chat_call_options(rise_wire_methods::chat::DELETE_ALL_MESSAGE_REQUESTS, false, true),
            Some(success),
            Some(failure),
            cx,
        );
    }
}
