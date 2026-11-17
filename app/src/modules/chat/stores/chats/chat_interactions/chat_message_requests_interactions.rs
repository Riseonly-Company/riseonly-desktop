use std::rc::Rc;

use gpui::{App, Context};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde_json::{Map, Value};

use super::chat_interactions::ChatInteractionsStore;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_presentation::{RiseChatListKind, RiseConversationPresentation};
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_message_requests_wire_types::ResolveMessageRequestGatewayDTO;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use rise_canon::ChatMessageRequestAction;

fn effective_user_id(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
}

fn conversation_is_active(cx: &mut App) -> bool {
    RiseConversationPresentation::shared(cx).read(cx).is_active()
}

fn active_domain(cx: &mut App) -> Option<std::sync::Arc<RiseConversationDomain>> {
    RiseConversationDomainRegistry::shared(cx).read(cx).active()
}

fn report(
    on_complete: &Option<Rc<dyn Fn(bool, Option<ChatInfo>, &mut App)>>,
    success: bool,
    chat: Option<ChatInfo>,
    cx: &mut App,
) {
    if let Some(on_complete) = on_complete.clone() {
        cx.defer(move |cx| on_complete(success, chat, cx));
    }
}

impl ChatInteractionsStore {
    pub fn load_message_requests(&mut self, cx: &mut Context<Self>) {
        if conversation_is_active(cx) {
            if let Some(domain) = active_domain(cx) {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain.load_chats(RiseChatListKind::Requests).await;
                }));
            }
            return;
        }
        let actions = ChatActionsStore::shared(cx);
        if actions.read(cx).get_message_requests_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_message_requests_action(None, false, true, cx));
    }

    pub fn refresh_message_requests(&mut self, cx: &mut Context<Self>) {
        if conversation_is_active(cx) {
            if let Some(domain) = active_domain(cx) {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain.refresh_chats(RiseChatListKind::Requests).await;
                }));
            }
            return;
        }
        let actions = ChatActionsStore::shared(cx);
        if actions.read(cx).get_message_requests_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_message_requests_action(None, true, false, cx));
    }

    pub fn load_more_message_requests(&mut self, cx: &mut Context<Self>) {
        let services = ChatServicesStore::shared(cx);
        if conversation_is_active(cx) {
            if !services.update(cx, |store, cx| store.message_requests_has_more(cx)) {
                return;
            }
            if let Some(domain) = active_domain(cx) {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain.load_more_chats(RiseChatListKind::Requests).await;
                }));
            }
            return;
        }
        let Some(user_id) = effective_user_id(cx) else {
            return;
        };
        if !services.update(cx, |store, cx| store.message_requests_has_more(cx)) {
            return;
        }
        let Some(cursor) = services
            .update(cx, |store, cx| store.message_requests_relative_id(cx))
            .filter(|cursor| !cursor.is_empty())
        else {
            return;
        };
        let cache_key = ChatServicesStore::message_requests_cache_key(&user_id);
        let can_request = SaiWs::pagination_state(&cache_key, cx)
            .read(cx)
            .state(SaiWsPaginationEdge::Trailing)
            .can_request_automatically();
        if !can_request {
            return;
        }
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_message_requests_action(Some(&cursor), false, false, cx));
    }

    pub fn resolve_message_request(
        &mut self,
        chat_id: &str,
        action: ChatMessageRequestAction,
        on_complete: Option<Rc<dyn Fn(bool, Option<ChatInfo>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id(cx).is_none_or(|user_id| user_id.is_empty()) {
            report(&on_complete, false, None, cx);
            return;
        }
        let actions = ChatActionsStore::shared(cx);
        if actions.read(cx).resolve_message_request_sai.is_pending() {
            return;
        }
        let is_accept = matches!(action, ChatMessageRequestAction::Accept);
        let services = ChatServicesStore::shared(cx);
        let request_chat = services
            .update(cx, |store, cx| store.message_requests(cx))
            .into_iter()
            .find(|chat| chat.id == chat_id);
        let removed: Option<Map<String, Value>> = services.update(cx, |store, cx| store.message_request_raw_row(chat_id, cx));
        if !is_accept {
            services.update(cx, |store, cx| store.remove_message_request(chat_id, cx));
        }

        let on_success: Rc<dyn Fn(ResolveMessageRequestGatewayDTO, &mut App)> = {
            let chat_id = chat_id.to_owned();
            let on_complete = on_complete.clone();
            Rc::new(move |dto: ResolveMessageRequestGatewayDTO, cx: &mut App| {
                let chat = dto.chat.as_ref().map(|chat| RiseStoredChat::from_canon(chat).to_chat_info());
                let services = ChatServicesStore::shared(cx);
                if is_accept {
                    services.update(cx, |store, cx| store.remove_message_request(&chat_id, cx));
                    if let Some(accepted) = chat.clone().or_else(|| request_chat.clone()) {
                        services.update(cx, |store, cx| store.accept_message_request(accepted, cx));
                    }
                } else {
                    services.update(cx, |store, cx| {
                        store.confirmed_removed_chat_ids.insert(chat_id.clone());
                        cx.notify();
                    });
                }
                report(&on_complete, true, chat.or_else(|| request_chat.clone()), cx);
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |_: &anyhow::Error, cx: &mut App| {
                let services = ChatServicesStore::shared(cx);
                let confirmed_removed = services.read(cx).confirmed_removed_chat_ids.contains(&chat_id);
                if !is_accept && !confirmed_removed {
                    if let Some(removed) = removed.clone() {
                        services.update(cx, |store, cx| store.restore_message_request(removed, cx));
                    }
                }
                report(&on_complete, false, None, cx);
            })
        };
        actions.update(cx, |actions, cx| {
            actions.resolve_message_request_action(chat_id, action, Some(on_success), Some(on_error), cx);
        });
    }

    pub fn delete_all_message_requests(&mut self, cx: &mut Context<Self>) {
        let services = ChatServicesStore::shared(cx);
        let actions = ChatActionsStore::shared(cx);
        if conversation_is_active(cx) {
            if services.update(cx, |store, cx| store.message_requests(cx)).is_empty() {
                return;
            }
            actions.update(cx, |actions, cx| actions.delete_all_message_requests_action(None, cx));
            return;
        }
        if effective_user_id(cx).is_none_or(|user_id| user_id.is_empty()) {
            return;
        }
        let Some(snapshot) = services.update(cx, |store, cx| store.message_requests_root_snapshot(cx)) else {
            return;
        };
        if actions.read(cx).delete_all_message_requests_sai.is_pending() {
            return;
        }
        services.update(cx, |store, cx| store.clear_message_requests(cx));
        let on_error: SaiWsOnError = Rc::new(move |_: &anyhow::Error, cx: &mut App| {
            let snapshot = snapshot.clone();
            ChatServicesStore::shared(cx)
                .update(cx, |store, cx| store.restore_message_requests_root_merging_current(snapshot, cx));
        });
        actions.update(cx, |actions, cx| actions.delete_all_message_requests_action(Some(on_error), cx));
    }
}
