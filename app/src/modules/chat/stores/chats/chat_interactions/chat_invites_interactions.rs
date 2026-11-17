use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Context, Global, Task};
use rise_canon::rise_proto::chat::{ProtoInviteLink, ProtoInviteVoiceTarget};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_engine::RiseWireError;
use rise_errors::ErrorCategory;
use rise_i18n::tr;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SAI_WS_DID_CONNECT;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::{
    RISE_ENGINE_CONNECTION_READY, RiseEngineAppRuntimeError,
};
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutorError;
use crate::modules::chat::engine::core::rise_conversation_request_state::RiseConversationStateError;
use crate::modules::chat::stores::chats::chat_actions::chat_invites_actions::{
    ChatInviteActionResult, ChatInviteLinkCreated,
};
use crate::modules::chat::stores::chats::chat_actions::chat_roles_actions::ChatRolesResultCallback;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatInfo, ChatInviteActionError, ChatPublicJoinOutcome, ChatType, JoinLinkSheetPhase,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    GetChatPreviewGatewayDTO, GetChatPreviewGatewayDTOExt, JoinByLinkGatewayDTO, RequestJoinGatewayDTO, RiseStoredChat,
};
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::{
    ChatInteractionsBoolResult, ChatInteractionsDone, ChatInteractionsStore,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

const JOIN_PREVIEW_OFFLINE_GRACE_SECONDS: u64 = 8;

pub type ChatInteractionsJoinedResult = Box<dyn FnOnce(Option<String>, &mut App)>;
pub type ChatInteractionsPublicJoinResult = Box<dyn FnOnce(ChatPublicJoinOutcome, &mut App)>;

fn load_initial_messages(chat_id: &str, cx: &mut App) {
    MessageInteractionsStore::shared(cx).update(cx, |messages, cx| {
        messages.load_initial_messages(chat_id, None, None, None, None, None, None, cx);
    });
}

impl ChatInteractionsStore {
    pub fn load_invite_links(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_invite_links_action(chat_id, cx));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_invite_link(
        &mut self,
        chat_id: &str,
        name: Option<&str>,
        expires_at_ms: Option<i64>,
        usage_limit: Option<i64>,
        creates_join_request: bool,
        voice: Option<ProtoInviteVoiceTarget>,
        on_rights_revoked: Option<ChatInteractionsDone>,
        on_created: Option<ChatInviteLinkCreated>,
        on_result: ChatInteractionsBoolResult,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.create_invite_link_action(
                chat_id,
                name,
                expires_at_ms,
                usage_limit,
                creates_join_request,
                voice,
                on_rights_revoked.map(Self::once_callback),
                on_created,
                Self::once_result(on_result),
                cx,
            );
        });
    }

    pub fn load_voice_invite_link(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        on_result: Box<dyn FnOnce(Option<ProtoInviteLink>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let on_result = once_invite_result(move |result: Result<ProtoInviteLink, &anyhow::Error>, cx: &mut App| {
            match result {
                Ok(link) => on_result(Some(link), cx),
                Err(error) => {
                    let failure = CanonErrors::failure(error);
                    let text = match failure.as_ref() {
                        Some(failure) => CanonErrors::text_for(failure).unwrap_or_else(|| {
                            if matches!(CanonErrors::category(failure.code.as_deref()), ErrorCategory::Permission) {
                                tr("voice_invite_create_forbidden")
                            } else {
                                tr("retry_later_error")
                            }
                        }),
                        None => tr("retry_later_error"),
                    };
                    Notifier::send(
                        text,
                        NotifierSendOptions {
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                    on_result(None, cx);
                }
            }
        });
        Self::actions(cx)
            .update(cx, |actions, cx| actions.get_voice_invite_link_action(chat_id, topic_id, on_result, cx));
    }

    pub fn load_voice_invite_links(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        on_result: Box<dyn FnOnce(Option<Vec<ProtoInviteLink>>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let on_result = RefCell::new(Some(on_result));
        let on_result = Rc::new(move |links: Option<Vec<ProtoInviteLink>>, cx: &mut App| {
            let on_result = on_result.borrow_mut().take();
            if let Some(on_result) = on_result {
                on_result(links, cx);
            }
        });
        Self::actions(cx)
            .update(cx, |actions, cx| actions.get_voice_invite_links_action(chat_id, topic_id, on_result, cx));
    }

    pub fn revoke_voice_invite_link(&mut self, link_id: &str, on_result: ChatInteractionsBoolResult, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.revoke_voice_invite_link_action(link_id, Self::once_result(on_result), cx);
        });
    }

    pub fn revoke_invite_link(&mut self, chat_id: &str, link_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.revoke_invite_link_action(chat_id, link_id, cx));
    }

    pub fn open_invite_link(&mut self, token: &str, cx: &mut Context<Self>) {
        let clean = trimming_whitespaces_and_newlines(token).to_owned();
        if clean.is_empty() {
            return;
        }
        let Some(request_id) =
            Self::service(cx).update(cx, |service, cx| service.begin_join_link_preview(&clean, false, cx))
        else {
            return;
        };

        Self::load_join_link_preview(clean, request_id, cx);
    }

    pub fn retry_invite_link_preview(&mut self, cx: &mut Context<Self>) {
        Self::retry_join_link_preview(cx);
    }

    fn retry_join_link_preview(cx: &mut App) {
        let Some((token, request_id)) = Self::service(cx).update(cx, |service, cx| service.retry_join_link_preview(cx))
        else {
            return;
        };

        Self::load_join_link_preview(token, request_id, cx);
    }

    fn load_join_link_preview(token: String, request_id: Uuid, cx: &mut App) {
        JoinLinkPreviewReconnectRetry::will_send(request_id, cx);
        let request_token = token.clone();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_chat_preview_by_link_action(
                &request_token,
                once_invite_result(move |dto: Result<GetChatPreviewGatewayDTO, &anyhow::Error>, cx: &mut App| {
                    Self::finish_join_link_preview(token, request_id, dto, cx);
                }),
                cx,
            );
        });
    }

    fn finish_join_link_preview(
        token: String,
        request_id: Uuid,
        dto: Result<GetChatPreviewGatewayDTO, &anyhow::Error>,
        cx: &mut App,
    ) {
        let service = Self::service(cx);
        match dto {
            Ok(response) => {
                let preview = if response.success { response.to_chat_join_preview() } else { None };
                let refusal = response
                    .join_status
                    .as_deref()
                    .map(trimming_whitespaces_and_newlines)
                    .unwrap_or_default();
                if response.success && (preview.is_some() || refusal.is_empty()) {
                    service.update(cx, |service, cx| {
                        let _ = service.resolve_join_link_preview(&token, request_id, preview, cx);
                    });
                } else {
                    let (message, retryable) = service
                        .read(cx)
                        .join_preview_failure(response.join_status.as_deref(), response.error.as_deref());
                    service.update(cx, |service, cx| {
                        let _ = service.fail_join_link_preview(&token, request_id, message, retryable, cx);
                    });
                }
            }
            Err(error) => {
                let retry_token = token.clone();
                let waits = JoinLinkPreviewReconnectRetry::wait(
                    error,
                    request_id,
                    Box::new(move |cx: &mut App| {
                        Self::reload_join_link_preview_after_reconnect(retry_token, request_id, cx);
                    }),
                    cx,
                );
                if waits {
                    Self::offer_retry_if_still_offline(token, request_id, cx);
                    return;
                }
                let (message, retryable) = service.read(cx).join_preview_failure(None, Some(&error.to_string()));
                service.update(cx, |service, cx| {
                    let _ = service.fail_join_link_preview(&token, request_id, message, retryable, cx);
                });
            }
        }
    }

    fn reload_join_link_preview_after_reconnect(token: String, request_id: Uuid, cx: &mut App) {
        let service = Self::service(cx);
        if service.read(cx).is_loading_join_link_preview(&token, request_id) {
            Self::load_join_link_preview(token, request_id, cx);
            return;
        }
        let waits_for_reconnect = service.read(cx).join_link_sheet.as_ref().is_some_and(|state| {
            state.token == token
                && state.preview_request_id == request_id
                && matches!(state.phase, JoinLinkSheetPhase::Failed(_, true))
        });
        if !waits_for_reconnect {
            return;
        }
        Self::retry_join_link_preview(cx);
    }

    fn offer_retry_if_still_offline(token: String, request_id: Uuid, cx: &mut App) {
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_secs(JOIN_PREVIEW_OFFLINE_GRACE_SECONDS)).await;
            cx.update(|cx| {
                let service = Self::service(cx);
                if RiseTransportStatus::is_connected(cx)
                    || !service.read(cx).is_loading_join_link_preview(&token, request_id)
                {
                    return;
                }
                service.update(cx, |service, cx| {
                    let _ =
                        service.fail_join_link_preview(&token, request_id, tr("join_preview_load_failed"), true, cx);
                });
            });
        })
        .detach();
    }

    pub fn join_via_sheet(
        &mut self,
        chat_id: &str,
        token: &str,
        on_joined: ChatInteractionsJoinedResult,
        cx: &mut Context<Self>,
    ) {
        let Some(submission_id) =
            Self::service(cx).update(cx, |service, cx| service.begin_join_link_submission(chat_id, token, cx))
        else {
            return;
        };

        let owned_chat_id = chat_id.to_owned();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.join_by_link_action(
                chat_id,
                token,
                once_invite_result(move |result: Result<JoinByLinkGatewayDTO, &anyhow::Error>, cx: &mut App| {
                    Self::finish_join_via_sheet(owned_chat_id, submission_id, result, on_joined, cx);
                }),
                cx,
            );
        });
    }

    fn finish_join_via_sheet(
        chat_id: String,
        submission_id: Uuid,
        result: Result<JoinByLinkGatewayDTO, &anyhow::Error>,
        on_joined: ChatInteractionsJoinedResult,
        cx: &mut App,
    ) {
        let service = Self::service(cx);
        match result {
            Ok(dto) if dto.success => {
                let chat = dto.chat.as_ref().map(RiseStoredChat::from_canon);
                let joined_id = service.update(cx, |service, cx| service.reconcile_joined_chat(&chat_id, chat.as_ref(), cx));
                if dto.chat.is_none() {
                    Self::actions(cx).update(cx, |actions, cx| actions.get_chat_by_id_action(&joined_id, false, cx));
                }
                load_initial_messages(&joined_id, cx);
                let finished =
                    service.update(cx, |service, cx| service.finish_join_link_submission(submission_id, None, cx));
                if !finished {
                    return;
                }
                on_joined(Some(joined_id), cx);
            }
            Ok(dto) => {
                let raw = Some(dto.message.as_str());
                if service.read(cx).is_already_member_join_failure(raw) {
                    Self::complete_already_member_join(chat_id, submission_id, on_joined, cx);
                    return;
                }
                let message = service.read(cx).localized_join_submission_failure(raw, false);
                let finished = service
                    .update(cx, |service, cx| service.finish_join_link_submission(submission_id, Some(message), cx));
                if !finished {
                    return;
                }
                on_joined(None, cx);
            }
            Err(error) => {
                if service.read(cx).is_already_member_join_error(error) {
                    Self::complete_already_member_join(chat_id, submission_id, on_joined, cx);
                    return;
                }
                let message = service.read(cx).localized_join_submission_error(error, false);
                let finished = service
                    .update(cx, |service, cx| service.finish_join_link_submission(submission_id, Some(message), cx));
                if !finished {
                    return;
                }
                on_joined(None, cx);
            }
        }
    }

    pub fn join_public_chat(
        &mut self,
        chat: &ChatInfo,
        requires_approval: bool,
        on_result: ChatInteractionsPublicJoinResult,
        cx: &mut Context<Self>,
    ) {
        let chat_id = chat.id.clone();
        let is_channel = chat.r#type == ChatType::Channel;

        if requires_approval {
            Self::actions(cx).update(cx, |actions, cx| {
                let owned_chat_id = chat_id.clone();
                actions.request_public_join_action(
                    &chat_id,
                    once_invite_result(move |result: Result<RequestJoinGatewayDTO, &anyhow::Error>, cx: &mut App| match result {
                        Ok(dto) if dto.success => {
                            Self::public_chat_joined(owned_chat_id, requires_approval, on_result, cx);
                        }
                        Ok(dto) => {
                            Self::public_join_failed(
                                owned_chat_id,
                                is_channel,
                                requires_approval,
                                Some(&dto.message),
                                None,
                                on_result,
                                cx,
                            );
                        }
                        Err(error) => {
                            Self::public_join_failed(
                                owned_chat_id,
                                is_channel,
                                requires_approval,
                                None,
                                Some(error),
                                on_result,
                                cx,
                            );
                        }
                    }),
                    cx,
                );
            });
        } else {
            Self::actions(cx).update(cx, |actions, cx| {
                let owned_chat_id = chat_id.clone();
                actions.join_public_chat_action(
                    &chat_id,
                    once_invite_result(move |result: Result<rise_canon::rise_response::chat::JoinChat, &anyhow::Error>, cx: &mut App| {
                        match result {
                            Ok(dto) if dto.success => {
                                Self::public_chat_joined(owned_chat_id, requires_approval, on_result, cx);
                            }
                            Ok(dto) => {
                                Self::public_join_failed(
                                    owned_chat_id,
                                    is_channel,
                                    requires_approval,
                                    Some(&dto.message),
                                    None,
                                    on_result,
                                    cx,
                                );
                            }
                            Err(error) => {
                                Self::public_join_failed(
                                    owned_chat_id,
                                    is_channel,
                                    requires_approval,
                                    None,
                                    Some(error),
                                    on_result,
                                    cx,
                                );
                            }
                        }
                    }),
                    cx,
                );
            });
        }
    }

    fn public_chat_joined(
        chat_id: String,
        requires_approval: bool,
        on_result: ChatInteractionsPublicJoinResult,
        cx: &mut App,
    ) {
        if !requires_approval {
            Self::service(cx).update(cx, |service, cx| service.promote_community_preview_to_member(&chat_id, cx));
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_chat_by_id_action(&chat_id, true, cx));
        load_initial_messages(&chat_id, cx);
        on_result(ChatPublicJoinOutcome::Joined, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn public_join_failed(
        chat_id: String,
        is_channel: bool,
        requires_approval: bool,
        raw: Option<&str>,
        error: Option<&anyhow::Error>,
        on_result: ChatInteractionsPublicJoinResult,
        cx: &mut App,
    ) {
        let service = Self::service(cx);
        let service = service.read(cx);
        let banned = match error {
            Some(error) => service.is_banned_join_error(error),
            None => service.is_banned_join_failure(raw),
        };
        if banned {
            on_result(ChatPublicJoinOutcome::Banned, cx);
            return;
        }
        let already_there = match (requires_approval, error) {
            (true, Some(error)) => service.is_pending_join_request_error(error),
            (true, None) => service.is_pending_join_request_failure(raw),
            (false, Some(error)) => service.is_already_member_join_error(error),
            (false, None) => service.is_already_member_join_failure(raw),
        };
        if already_there {
            Self::public_chat_joined(chat_id, requires_approval, on_result, cx);
            return;
        }
        if requires_approval {
            Notifier::send(
                tr("join_request_failed"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        } else {
            let title = if is_channel {
                tr("chat_public_preview_subscribe_failed")
            } else {
                tr("chat_public_preview_join_failed")
            };
            Notifier::send(
                title,
                NotifierSendOptions {
                    content: Some(tr("chat_public_preview_join_failed_hint")),
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Bottom,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        }
        on_result(ChatPublicJoinOutcome::Failed, cx);
    }

    pub fn request_join_via_sheet(
        &mut self,
        chat_id: &str,
        token: &str,
        on_done: ChatInteractionsBoolResult,
        cx: &mut Context<Self>,
    ) {
        let Some(submission_id) =
            Self::service(cx).update(cx, |service, cx| service.begin_join_link_submission(chat_id, token, cx))
        else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| {
            actions.request_join_action(
                chat_id,
                token,
                once_invite_result(move |result: Result<RequestJoinGatewayDTO, &anyhow::Error>, cx: &mut App| {
                    Self::finish_request_join_via_sheet(submission_id, result, on_done, cx);
                }),
                cx,
            );
        });
    }

    fn finish_request_join_via_sheet(
        submission_id: Uuid,
        result: Result<RequestJoinGatewayDTO, &anyhow::Error>,
        on_done: ChatInteractionsBoolResult,
        cx: &mut App,
    ) {
        let service = Self::service(cx);
        let (raw, message) = match result {
            Ok(dto) if dto.success => {
                let finished =
                    service.update(cx, |service, cx| service.finish_join_link_submission(submission_id, None, cx));
                if finished {
                    on_done(true, cx);
                }
                return;
            }
            Ok(dto) => {
                let message = service.read(cx).localized_join_submission_failure(Some(&dto.message), true);
                (dto.message, message)
            }
            Err(error) => {
                let message = service.read(cx).localized_join_submission_error(error, true);
                (error.to_string(), message)
            }
        };
        if service.read(cx).is_pending_join_request_failure(Some(&raw)) {
            let finished =
                service.update(cx, |service, cx| service.finish_join_link_submission(submission_id, None, cx));
            if finished {
                on_done(true, cx);
            }
            return;
        }
        let finished =
            service.update(cx, |service, cx| service.finish_join_link_submission(submission_id, Some(message), cx));
        if !finished {
            return;
        }
        on_done(false, cx);
    }

    fn complete_already_member_join(
        chat_id: String,
        submission_id: Uuid,
        on_joined: ChatInteractionsJoinedResult,
        cx: &mut App,
    ) {
        let service = Self::service(cx);
        let joined_id = service.update(cx, |service, cx| service.reconcile_joined_chat(&chat_id, None, cx));
        Self::actions(cx).update(cx, |actions, cx| actions.get_chat_by_id_action(&joined_id, false, cx));
        load_initial_messages(&joined_id, cx);
        let finished = service.update(cx, |service, cx| service.finish_join_link_submission(submission_id, None, cx));
        if !finished {
            return;
        }
        on_joined(Some(joined_id), cx);
    }

    pub fn dismiss_join_link_sheet(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| {
            let _ = service.dismiss_join_link_sheet(cx);
        });
    }

    pub fn load_join_requests(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_join_requests_action(chat_id, None, cx));
    }

    pub fn load_more_join_requests(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let service = Self::service(cx);
        if !service.update(cx, |service, cx| service.join_requests_have_more(chat_id, cx)) {
            return;
        }
        let Some(rid) = service.update(cx, |service, cx| service.join_requests_next_relative_id(chat_id, cx)) else {
            return;
        };
        let cache_key = ChatServicesStore::join_requests_cache_key(chat_id);
        if !SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().can_request_automatically() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.get_join_requests_action(chat_id, Some(&rid), cx));
    }

    pub fn approve_join_request(
        &mut self,
        chat_id: &str,
        request_id: &str,
        on_result: Option<ChatInteractionsBoolResult>,
        cx: &mut Context<Self>,
    ) {
        let on_result: ChatRolesResultCallback = match on_result {
            Some(on_result) => Self::once_result(on_result),
            None => Rc::new(|_: bool, _: &mut App| {}),
        };
        Self::actions(cx)
            .update(cx, |actions, cx| actions.approve_join_request_action(chat_id, request_id, on_result, cx));
    }

    pub fn reject_join_request(
        &mut self,
        chat_id: &str,
        request_id: &str,
        on_result: Option<ChatInteractionsBoolResult>,
        cx: &mut Context<Self>,
    ) {
        let on_result: ChatRolesResultCallback = match on_result {
            Some(on_result) => Self::once_result(on_result),
            None => Rc::new(|_: bool, _: &mut App| {}),
        };
        Self::actions(cx)
            .update(cx, |actions, cx| actions.reject_join_request_action(chat_id, request_id, on_result, cx));
    }
}

fn once_invite_result<T: 'static>(
    body: impl for<'a> FnOnce(Result<T, &'a anyhow::Error>, &mut App) + 'static,
) -> ChatInviteActionResult<T> {
    let body = RefCell::new(Some(body));
    Rc::new(move |result: Result<T, &anyhow::Error>, cx: &mut App| {
        let body = body.borrow_mut().take();
        if let Some(body) = body {
            body(result, cx);
        }
    })
}

#[derive(Default)]
struct JoinLinkPreviewReconnectRetry {
    observers: Vec<RiseSubscription>,
    delivery: Option<Task<()>>,
    pending: Option<ChatInteractionsDone>,
    retried_at_once: Option<Uuid>,
    ready_edges: u64,
    sent: Option<(Uuid, u64)>,
}

impl Global for JoinLinkPreviewReconnectRetry {}

impl JoinLinkPreviewReconnectRetry {
    fn will_send(request_id: Uuid, cx: &mut App) {
        Self::install_if_needed(cx);
        let state = cx.default_global::<Self>();
        state.sent = Some((request_id, state.ready_edges));
    }

    fn wait(error: &anyhow::Error, request_id: Uuid, retry: ChatInteractionsDone, cx: &mut App) -> bool {
        let refused = matches!(
            error.downcast_ref::<SaiWsSendError>(),
            Some(SaiWsSendError::NotConnected | SaiWsSendError::SendFailed(_))
        );
        let dropped = error.downcast_ref::<SaiWsResponseError>().is_some_and(|response| response.is_transport_drop);
        let not_ready = !refused && !dropped && Self::engine_not_ready(error);
        if !refused && !dropped && !not_ready {
            return false;
        }
        Self::install_if_needed(cx);
        let state = cx.default_global::<Self>();
        if state.sent.is_some_and(|(sent_id, ready_edge)| sent_id == request_id && ready_edge != state.ready_edges) {
            state.pending = None;
            retry(cx);
            return true;
        }
        if matches!(error.downcast_ref::<RiseWireError>(), Some(RiseWireError::Suspended)) {
            cx.default_global::<Self>().pending = Some(retry);
            return true;
        }
        let is_connected = SaiWs::manager(cx).read(cx).is_connected();
        if not_ready {
            if !is_connected {
                cx.default_global::<Self>().pending = Some(retry);
                return true;
            }
            let state = cx.default_global::<Self>();
            if state.retried_at_once == Some(request_id) {
                return false;
            }
            state.retried_at_once = Some(request_id);
            state.pending = None;
            retry(cx);
            return true;
        }
        let state = cx.default_global::<Self>();
        if refused && is_connected && state.retried_at_once != Some(request_id) {
            state.retried_at_once = Some(request_id);
            state.pending = None;
            retry(cx);
            return true;
        }
        state.pending = Some(retry);
        true
    }

    fn engine_not_ready(error: &anyhow::Error) -> bool {
        if CanonErrors::is_connection_failure(error) {
            return true;
        }
        if matches!(
            error.downcast_ref::<RiseWireError>(),
            Some(RiseWireError::Shutdown | RiseWireError::StaleSession | RiseWireError::StaleAccount)
        ) {
            return true;
        }
        if error.is::<RiseEngineAppRuntimeError>() {
            return true;
        }
        if matches!(
            error.downcast_ref::<RiseConversationActionExecutorError>(),
            Some(RiseConversationActionExecutorError::EngineUnavailable)
        ) {
            return true;
        }
        if matches!(
            error.downcast_ref::<RiseConversationStateError>(),
            Some(RiseConversationStateError::MissingActiveSession | RiseConversationStateError::StaleAccount)
        ) {
            return true;
        }
        matches!(error.downcast_ref::<ChatInviteActionError>(), Some(ChatInviteActionError::InvalidRequest))
    }

    fn install_if_needed(cx: &mut App) {
        if !cx.default_global::<Self>().observers.is_empty() {
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let observers: Vec<RiseSubscription> = [SAI_WS_DID_CONNECT, RISE_ENGINE_CONNECTION_READY]
            .map(|name| {
                let sender = sender.clone();
                RiseAppEvents::default_center().observe(name, move |_| {
                    let _ = sender.unbounded_send(());
                })
            })
            .into_iter()
            .collect();
        let delivery = cx.spawn(async move |cx: &mut AsyncApp| {
            while receiver.next().await.is_some() {
                cx.update(Self::connection_did_return);
            }
        });
        let state = cx.default_global::<Self>();
        state.observers = observers;
        state.delivery = Some(delivery);
    }

    fn connection_did_return(cx: &mut App) {
        let state = cx.default_global::<Self>();
        state.ready_edges = state.ready_edges.wrapping_add(1);
        let retry = state.pending.take();
        if let Some(retry) = retry {
            retry(cx);
        }
    }
}
