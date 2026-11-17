use std::any::Any;
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::chat as chat_key;
use rise_canon::{RiseRequestBody, rise_request, rise_wire_methods};
use rise_engine::RiseJSONValue;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiWsScreenRefresh;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::{
    ChatActionCallback, ChatActionsStore, chat_page_pagination, effective_user_id_for_wire, log_chat_failure,
};
use crate::modules::chat::stores::chats::chat_actions::chat_archive_wire_types::ChatArchiveMutationGatewayDTO;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

impl ChatActionsStore {
    pub fn get_archived_chats_action(
        &mut self,
        relative_id: Option<&str>,
        reload_if_cached: bool,
        reflect_pending_in_ui: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };

        let body = rise_request::chat::GetArchivedChats {
            limit: Some(ChatServicesStore::archived_chats_limit()),
            q: None,
            relative_id: relative_id.map(str::to_owned),
            up: Some(false),
        };
        let cache_key = ChatServicesStore::archived_chats_cache_key_for_user(&user_id);
        let is_first_page = relative_id.is_none();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::GET_ARCHIVED_CHATS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached: !is_first_page || reload_if_cached,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: is_first_page && reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::Persistent),
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(chat_key::CHATS.to_owned()),
            path_to_id: Some(chat_key::ID.to_owned()),
            pagination: Some(chat_page_pagination(relative_id.map(str::to_owned))),
            screen_refresh: Some(SaiWsScreenRefresh::new(
                true,
                vec!["Chats".to_owned(), "ArchivedChats".to_owned(), "ChatProfile".to_owned(), "Chat".to_owned()],
                None,
            )),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(move |_: Option<Box<dyn Any>>, cx: &mut App| {
            ChatServicesStore::shared(cx).update(cx, |services, cx| services.reconcile_chat_archives_after_fetch(cx));
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            log_chat_failure(format!("get_archived_chats error: {error}"), cx);
        });

        SaiWs::request_slice(&self.get_archived_chats_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn archive_chat_action(&mut self, chat: &ChatInfo, on_error: Option<ChatActionCallback>, cx: &mut Context<Self>) {
        let slice = self.archive_chat_sai.clone();
        let body = rise_request::chat::ArchiveChat { chat_id: chat.id.clone() };
        self.perform_archive_mutation_action(chat.clone(), true, slice, rise_wire_methods::chat::ARCHIVE_CHAT, body, on_error, cx);
    }

    pub fn unarchive_chat_action(&mut self, chat: &ChatInfo, on_error: Option<ChatActionCallback>, cx: &mut Context<Self>) {
        let slice = self.unarchive_chat_sai.clone();
        let body = rise_request::chat::UnarchiveChat { chat_id: chat.id.clone() };
        self.perform_archive_mutation_action(
            chat.clone(),
            false,
            slice,
            rise_wire_methods::chat::UNARCHIVE_CHAT,
            body,
            on_error,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn perform_archive_mutation_action<Body: RiseRequestBody + 'static>(
        &mut self,
        chat: ChatInfo,
        moves_to_archive: bool,
        slice: Rc<SaiTypedSlice<ChatArchiveMutationGatewayDTO>>,
        method: &'static str,
        body: Body,
        on_error: Option<ChatActionCallback>,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat.id.is_empty() {
            if let Some(on_error) = on_error {
                cx.defer(move |cx| on_error(cx));
            }
            return;
        }

        let domain = RiseConversationDomainRegistry::shared(cx).read(cx).active();
        if let Some(domain) = domain {
            let chat_id = chat.id;
            cx.spawn(async move |_, cx| {
                let result = match RiseJSONValue::encode(&body) {
                    Ok(encoded) => {
                        domain.repository_call(rise_wire_methods::chat::SERVICE, method, &encoded).await.map(|_| ())
                    }
                    Err(error) => Err(error.into()),
                };
                if let Err(error) = result {
                    cx.update(|cx| {
                        log_chat_failure(format!("{method}({chat_id}) error: {error}"), cx);
                        if let Some(on_error) = &on_error {
                            on_error(cx);
                        }
                    });
                }
            })
            .detach();
            return;
        }

        let services = ChatServicesStore::shared(cx);
        let Some(token) =
            services.update(cx, |services, cx| services.apply_archive_mutation_optimistic(&chat, moves_to_archive, cx))
        else {
            return;
        };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: method.to_owned(),
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            track_until_acknowledged: true,
            ..SaiWsRequestOptionsInit::default()
        });

        let success_id = chat.id.clone();
        let success_error = on_error.clone();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = payload
                .and_then(|payload| payload.downcast::<ChatArchiveMutationGatewayDTO>().ok())
                .filter(|response| response.success && response.is_archived == moves_to_archive);
            let services = ChatServicesStore::shared(cx);
            let Some(response) = response else {
                let did_revert = services.update(cx, |services, cx| services.revert_archive_mutation(&success_id, token, cx));
                if did_revert && let Some(on_error) = &success_error {
                    on_error(cx);
                }
                return;
            };

            let stored = response.chat.as_ref().map(RiseStoredChat::from_canon);
            services.update(cx, |services, cx| {
                services.complete_archive_mutation(&success_id, token, moves_to_archive, stored.as_ref(), cx);
            });
        });
        let failed_id = chat.id;
        let on_failure: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let did_revert =
                ChatServicesStore::shared(cx).update(cx, |services, cx| services.revert_archive_mutation(&failed_id, token, cx));
            log_chat_failure(format!("{method}({failed_id}) error: {error}"), cx);
            if did_revert && let Some(on_error) = &on_error {
                on_error(cx);
            }
        });

        SaiWs::request_slice(&slice, body, options, Some(on_success), Some(on_failure), cx);
    }
}
