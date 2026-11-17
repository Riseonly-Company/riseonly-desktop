use std::any::Any;
use std::rc::Rc;

use gpui::App;
use rise_canon::rise_key::chat as key;
use rise_canon::rise_proto::chat::{ProtoInviteLink, ProtoInviteVoiceTarget};
use rise_canon::rise_request;
use rise_canon::rise_wire_methods;
use rise_canon::rise_response::chat::JoinChat as JoinChatResponse;
use rise_engine::CancellationError;
use rise_errors::{ErrorCategory, ErrorCode};
use rise_i18n::tr;

use super::chat_actions::{ChatActionsStore, effective_user_id_for_wire};
use super::chat_folders_actions::chat_call_options;
use super::chat_types::{ChatInviteActionError, ChatVoiceInviteError};
use super::chat_wire_types::{
    CreateInviteLinkResultDTO, GetChatPreviewGatewayDTO, GetInviteLinksGatewayDTO, GetVoiceInviteLinkResultDTO,
    JoinByLinkGatewayDTO, RequestJoinGatewayDTO,
};
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationCursorFallback, SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

pub type ChatInviteActionResult<T> = Rc<dyn Fn(Result<T, &anyhow::Error>, &mut App)>;
pub type ChatInviteLinkCreated = Rc<dyn Fn(ProtoInviteLink, &mut App)>;
pub type ChatVoiceInviteLinksResult = Rc<dyn Fn(Option<Vec<ProtoInviteLink>>, &mut App)>;

fn typed_result_callbacks<T: 'static>(
    on_result: ChatInviteActionResult<T>,
    log_label: Option<&'static str>,
) -> (SaiWsOnSuccess, SaiWsOnError) {
    let on_success: SaiWsOnSuccess = {
        let on_result = Rc::clone(&on_result);
        Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            match payload.and_then(|payload| payload.downcast::<T>().ok()) {
                Some(dto) => on_result(Ok(*dto), cx),
                None => {
                    let error = anyhow::Error::new(ChatInviteActionError::InvalidResponse);
                    on_result(Err(&error), cx);
                }
            }
        })
    };
    let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
        if let Some(label) = log_label {
            Sai::log("Chat", || format!("{label} error: {error}"), SaiLogKind::Network, cx);
        }
        on_result(Err(error), cx);
    });
    (on_success, on_error)
}

fn fail_invalid_request<T>(on_result: &ChatInviteActionResult<T>, cx: &mut App) {
    let error = anyhow::Error::new(ChatInviteActionError::InvalidRequest);
    on_result(Err(&error), cx);
}

impl ChatActionsStore {
    pub fn get_invite_links_action(&self, chat_id: &str, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let links_are_empty =
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.invite_links(chat_id, cx).is_empty());
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::GET_INVITE_LINKS.to_owned(),
            cache_key: Some(ChatServicesStore::invite_links_cache_key(chat_id)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: links_are_empty,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: false,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::MemoryOnly),
            preferred_cache_layer: SaiWsCachePreferredLayer::MemoryFirst,
            path_to_array: Some(key::INVITE_LINKS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = Rc::new(|_payload: Option<Box<dyn Any>>, _cx: &mut App| {});
        let log_chat_id = chat_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("get_invite_links({log_chat_id}) error: {error}"), SaiLogKind::Network, cx);
        });
        SaiWs::request_slice(
            &self.get_invite_links_sai,
            rise_request::chat::GetInviteLinks::new(chat_id.to_owned()),
            options,
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_invite_link_action(
        &self,
        chat_id: &str,
        name: Option<&str>,
        expires_at_ms: Option<i64>,
        usage_limit: Option<i64>,
        creates_join_request: bool,
        voice: Option<ProtoInviteVoiceTarget>,
        on_rights_revoked: Option<Rc<dyn Fn(&mut App)>>,
        on_created: Option<ChatInviteLinkCreated>,
        on_result: Rc<dyn Fn(bool, &mut App)>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            on_result(false, cx);
            return;
        }

        let is_voice = voice.is_some();
        let mut body = rise_request::chat::CreateInviteLink::new(chat_id.to_owned());
        body.creates_join_request = Some(!is_voice && creates_join_request);
        if let Some(voice) = voice {
            body.voice = Some(true);
            body.voice_topic_id = voice.topic_id;
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            body.name = Some(name.to_owned());
        }
        if let Some(expires_at_ms) = expires_at_ms.filter(|expires_at_ms| *expires_at_ms > 0) {
            body.expires_at = Some(expires_at_ms);
        }
        if let Some(usage_limit) = usage_limit.filter(|usage_limit| *usage_limit > 0) {
            body.usage_limit = Some(usage_limit);
        }

        let on_success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let on_result = Rc::clone(&on_result);
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let wire = payload
                    .and_then(|payload| payload.downcast::<CreateInviteLinkResultDTO>().ok())
                    .and_then(|response| response.invite_link);
                if let Some(wire) = wire {
                    if !is_voice {
                        ChatServicesStore::shared(cx)
                            .update(cx, |store, cx| store.insert_invite_link(wire.clone(), &chat_id, cx));
                    }
                    if let Some(on_created) = on_created.as_ref() {
                        on_created(wire, cx);
                    }
                }
                on_result(true, cx);
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log("Chat", || format!("create_invite_link({chat_id}) error: {error}"), SaiLogKind::Network, cx);
                let failure = CanonErrors::failure(error);
                let text = failure
                    .as_ref()
                    .and_then(CanonErrors::text_for)
                    .unwrap_or_else(|| tr("retry_later_error"));
                Notifier::send(
                    text,
                    NotifierSendOptions {
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Bottom,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
                if let Some(failure) = failure
                    && matches!(CanonErrors::category(failure.code.as_deref()), ErrorCategory::Permission)
                    && let Some(on_rights_revoked) = on_rights_revoked.as_ref()
                {
                    on_rights_revoked(cx);
                }
                on_result(false, cx);
            })
        };
        SaiWs::request_slice(
            &self.create_invite_link_sai,
            body,
            chat_call_options(rise_wire_methods::chat::CREATE_INVITE_LINK, true, true),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn get_voice_invite_link_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        on_result: ChatInviteActionResult<ProtoInviteLink>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            fail_invalid_request(&on_result, cx);
            return;
        }

        let mut body = rise_request::chat::GetVoiceInviteLink::new(chat_id.to_owned());
        body.topic_id = topic_id.map(str::to_owned);
        let on_success: SaiWsOnSuccess = {
            let on_result = Rc::clone(&on_result);
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let dto = payload.and_then(|payload| payload.downcast::<GetVoiceInviteLinkResultDTO>().ok());
                match dto {
                    Some(dto) if dto.success && dto.invite_link.is_some() => {
                        if let Some(link) = dto.invite_link {
                            on_result(Ok(link), cx);
                        }
                    }
                    dto => {
                        let raw = dto.map(|dto| dto.error).unwrap_or_default();
                        let error = anyhow::Error::new(ChatVoiceInviteError::Refused(raw));
                        on_result(Err(&error), cx);
                    }
                }
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log("Chat", || format!("get_voice_invite_link({chat_id}) error: {error}"), SaiLogKind::Network, cx);
                on_result(Err(error), cx);
            })
        };
        SaiWs::request_slice(
            &self.get_voice_invite_link_sai,
            body,
            chat_call_options(rise_wire_methods::chat::GET_VOICE_INVITE_LINK, false, true),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn get_voice_invite_links_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        on_result: ChatVoiceInviteLinksResult,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            on_result(None, cx);
            return;
        }

        let mut body = rise_request::chat::GetInviteLinks::new(chat_id.to_owned());
        body.voice = Some(true);
        body.voice_topic_id = topic_id.map(str::to_owned);
        let on_success: SaiWsOnSuccess = {
            let on_result = Rc::clone(&on_result);
            Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
                let dto = payload.and_then(|payload| payload.downcast::<GetInviteLinksGatewayDTO>().ok());
                let links: Option<Vec<ProtoInviteLink>> = dto
                    .filter(|dto| dto.success)
                    .map(|dto| dto.invite_links.into_iter().filter(|link| !link.is_revoked).collect());
                on_result(links, cx);
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log(
                    "Chat",
                    || format!("get_invite_links voice({chat_id}) error: {error}"),
                    SaiLogKind::Network,
                    cx,
                );
                on_result(None, cx);
            })
        };
        SaiWs::request_slice(
            &self.get_voice_invite_links_sai,
            body,
            chat_call_options(rise_wire_methods::chat::GET_INVITE_LINKS, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn revoke_voice_invite_link_action(&self, link_id: &str, on_result: Rc<dyn Fn(bool, &mut App)>, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() || link_id.is_empty() {
            on_result(false, cx);
            return;
        }

        let on_success: SaiWsOnSuccess = {
            let on_result = Rc::clone(&on_result);
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| on_result(true, cx))
        };
        let on_error: SaiWsOnError = {
            let link_id = link_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log("Chat", || format!("revoke_invite_link voice({link_id}) error: {error}"), SaiLogKind::Network, cx);
                let text = CanonErrors::failure(error)
                    .as_ref()
                    .and_then(CanonErrors::text_for)
                    .unwrap_or_else(|| tr("retry_later_error"));
                Notifier::send(
                    text,
                    NotifierSendOptions {
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Bottom,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
                on_result(false, cx);
            })
        };
        SaiWs::request_slice(
            &self.revoke_voice_invite_link_sai,
            rise_request::chat::RevokeInviteLink::new(link_id.to_owned()),
            chat_call_options(rise_wire_methods::chat::REVOKE_INVITE_LINK, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn revoke_invite_link_action(&self, chat_id: &str, link_id: &str, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() || link_id.is_empty() {
            return;
        }

        let services = ChatServicesStore::shared(cx);
        let Some(removed) = services.update(cx, |store, cx| store.invite_link_raw_row(chat_id, link_id, cx)) else {
            return;
        };
        services.update(cx, |store, cx| store.remove_invite_link(chat_id, link_id, cx));

        let on_success: SaiWsOnSuccess = Rc::new(|_payload: Option<Box<dyn Any>>, _cx: &mut App| {});
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let link_id = link_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                Sai::log("Chat", || format!("revoke_invite_link({link_id}) error: {error}"), SaiLogKind::Network, cx);
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.restore_invite_link(removed.clone(), &chat_id, cx));
            })
        };
        SaiWs::request_slice(
            &self.revoke_invite_link_sai,
            rise_request::chat::RevokeInviteLink::new(link_id.to_owned()),
            chat_call_options(rise_wire_methods::chat::REVOKE_INVITE_LINK, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn get_join_requests_action(&self, chat_id: &str, relative_id: Option<&str>, cx: &mut App) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            return;
        }

        let is_first_page = relative_id.is_none();
        let services = ChatServicesStore::shared(cx);
        let show_pending =
            is_first_page && services.update(cx, |store, cx| store.join_requests(chat_id, cx).is_empty());
        let total_mark = services.update(cx, |store, cx| store.join_requests_total_mark(chat_id, cx));

        let mut body = rise_request::chat::GetJoinRequests::new(chat_id.to_owned());
        body.limit = Some(ChatServicesStore::join_requests_page_limit());
        body.relative_id = relative_id.map(str::to_owned);

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::chat::SERVICE.to_owned(),
            method: rise_wire_methods::chat::GET_JOIN_REQUESTS.to_owned(),
            cache_key: Some(ChatServicesStore::join_requests_cache_key(chat_id)),
            reload_if_cached: true,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: show_pending,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            disk_cache_policy: Some(SaiWsDiskCachePolicy::Persistent),
            reload_when_only_disk_cached: true,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(key::REQUESTS.to_owned()),
            path_to_id: Some(key::ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: relative_id.map(str::to_owned),
                response_relative_id_fallback: Some(SaiWsPaginationCursorFallback::LastItemId),
                has_more_path: Some(key::HAVE_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });

        let on_success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.reconcile_join_requests_total(&chat_id, &total_mark, cx));
            })
        };
        let log_chat_id = chat_id.to_owned();
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Sai::log("Chat", || format!("get_join_requests({log_chat_id}) error: {error}"), SaiLogKind::Network, cx);
        });
        SaiWs::request_slice(&self.get_join_requests_sai, body, options, Some(on_success), Some(on_error), cx);
    }

    pub fn get_chat_preview_by_link_action(
        &self,
        token: &str,
        on_result: ChatInviteActionResult<GetChatPreviewGatewayDTO>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || token.is_empty() {
            fail_invalid_request(&on_result, cx);
            return;
        }

        let body = rise_request::chat::GetChatPreviewByInviteLink { invite_link: Some(token.to_owned()) };
        let (on_success, on_error) = typed_result_callbacks(on_result, Some("get_chat_preview_by_invite_link"));
        SaiWs::request_slice(
            &self.get_chat_preview_sai,
            body,
            chat_call_options(rise_wire_methods::chat::GET_CHAT_PREVIEW_BY_INVITE_LINK, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn join_by_link_action(
        &self,
        chat_id: &str,
        token: &str,
        on_result: ChatInviteActionResult<JoinByLinkGatewayDTO>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() || token.is_empty() {
            fail_invalid_request(&on_result, cx);
            return;
        }

        let body = rise_request::chat::JoinChatByLink::new(chat_id.to_owned(), token.to_owned());
        let (on_success, on_error) = typed_result_callbacks(on_result, Some("join_chat_by_link"));
        SaiWs::request_slice(
            &self.join_by_link_sai,
            body,
            chat_call_options(rise_wire_methods::chat::JOIN_CHAT_BY_LINK, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn join_public_chat_action(
        &self,
        chat_id: &str,
        on_result: ChatInviteActionResult<JoinChatResponse>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            fail_invalid_request(&on_result, cx);
            return;
        }

        let (on_success, on_error) = typed_result_callbacks(on_result, None);
        SaiWs::request_slice(
            &self.join_chat_sai,
            rise_request::chat::JoinChat::new(chat_id.to_owned()),
            chat_call_options(rise_wire_methods::chat::JOIN_CHAT, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn request_public_join_action(
        &self,
        chat_id: &str,
        on_result: ChatInviteActionResult<RequestJoinGatewayDTO>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() {
            fail_invalid_request(&on_result, cx);
            return;
        }

        let (on_success, on_error) = typed_result_callbacks(on_result, None);
        SaiWs::request_slice(
            &self.request_join_sai,
            rise_request::chat::RequestJoinChat::new(chat_id.to_owned()),
            chat_call_options(rise_wire_methods::chat::REQUEST_JOIN_CHAT, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn request_join_action(
        &self,
        chat_id: &str,
        token: &str,
        on_result: ChatInviteActionResult<RequestJoinGatewayDTO>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() || token.is_empty() {
            fail_invalid_request(&on_result, cx);
            return;
        }

        let mut body = rise_request::chat::RequestJoinChat::new(chat_id.to_owned());
        body.invite_link = Some(token.to_owned());
        let (on_success, on_error) = typed_result_callbacks(on_result, Some("request_join_chat"));
        SaiWs::request_slice(
            &self.request_join_sai,
            body,
            chat_call_options(rise_wire_methods::chat::REQUEST_JOIN_CHAT, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn approve_join_request_action(
        &self,
        chat_id: &str,
        request_id: &str,
        on_result: Rc<dyn Fn(bool, &mut App)>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() || request_id.is_empty() {
            on_result(false, cx);
            return;
        }

        let (on_success, on_error) = Self::join_request_decision_callbacks(
            chat_id,
            request_id,
            "approve_join_request",
            on_result,
            cx,
        );
        SaiWs::request_slice(
            &self.approve_join_request_sai,
            rise_request::chat::ApproveJoinRequest::new(chat_id.to_owned(), request_id.to_owned()),
            chat_call_options(rise_wire_methods::chat::APPROVE_JOIN_REQUEST, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    pub fn reject_join_request_action(
        &self,
        chat_id: &str,
        request_id: &str,
        on_result: Rc<dyn Fn(bool, &mut App)>,
        cx: &mut App,
    ) {
        if effective_user_id_for_wire(cx).is_none() || chat_id.is_empty() || request_id.is_empty() {
            on_result(false, cx);
            return;
        }

        let (on_success, on_error) = Self::join_request_decision_callbacks(
            chat_id,
            request_id,
            "reject_join_request",
            on_result,
            cx,
        );
        SaiWs::request_slice(
            &self.reject_join_request_sai,
            rise_request::chat::RejectJoinRequest::new(chat_id.to_owned(), request_id.to_owned()),
            chat_call_options(rise_wire_methods::chat::REJECT_JOIN_REQUEST, true, false),
            Some(on_success),
            Some(on_error),
            cx,
        );
    }

    fn join_request_decision_callbacks(
        chat_id: &str,
        request_id: &str,
        log_label: &'static str,
        on_result: Rc<dyn Fn(bool, &mut App)>,
        cx: &mut App,
    ) -> (SaiWsOnSuccess, SaiWsOnError) {
        let services = ChatServicesStore::shared(cx);
        let removed = services.update(cx, |store, cx| store.join_request_raw_row(chat_id, request_id, cx));
        services.update(cx, |store, cx| {
            store.remove_join_request(chat_id, request_id, cx);
            store.begin_join_request_decision(chat_id, cx);
        });

        let on_success: SaiWsOnSuccess = {
            let chat_id = chat_id.to_owned();
            let on_result = Rc::clone(&on_result);
            Rc::new(move |_payload: Option<Box<dyn Any>>, cx: &mut App| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.end_join_request_decision(&chat_id, cx));
                on_result(true, cx);
            })
        };
        let on_error: SaiWsOnError = {
            let chat_id = chat_id.to_owned();
            let request_id = request_id.to_owned();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                let services = ChatServicesStore::shared(cx);
                services.update(cx, |store, cx| store.end_join_request_decision(&chat_id, cx));
                Sai::log("Chat", || format!("{log_label}({request_id}) error: {error}"), SaiLogKind::Network, cx);
                if let Some(removed) = removed.as_ref() {
                    services.update(cx, |store, cx| store.restore_join_request(removed.clone(), &chat_id, cx));
                }
                Self::report_join_request_failure(error, &chat_id, cx);
                on_result(false, cx);
            })
        };
        (on_success, on_error)
    }

    pub fn report_join_request_failure(error: &anyhow::Error, chat_id: &str, cx: &mut App) {
        if error.is::<CancellationError>() {
            return;
        }
        let is_rights_refusal = CanonErrors::failure(error)
            .is_some_and(|failure| failure.code.as_deref() == Some(ErrorCode::ChatPermissionDenied.wire()));
        if is_rights_refusal {
            ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_chat_by_id_action(chat_id, true, cx));
        }
        Self::report_member_mutation_failure(error, cx);
    }
}
