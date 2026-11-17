use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use gpui::{App, AsyncApp};
use rise_canon::{ChatMessageRequestAction, rise_key};
use rise_canon::rise_view::chat::{ChatById, ChatInfo};
use rise_canon::rise_wire_methods;
use rise_canon::rise_response::message::SharePayload as SharePayloadResponse;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewLoadState;
use rise_engine::{CancellationError, RiseJSONValue, RiseWireError};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::task::AbortHandle;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_canon_endpoints::{RiseCanonEndpoints, RiseEndpointClientTimeout as _};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::chat_engine_hooks::chat_engine_hooks;
use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepository;
use crate::modules::chat::engine::chats::rise_chat_repository_models::{
    RiseChatNotificationPreference, RiseChatOptimisticOperation, RiseChatOptimisticToken,
};
use crate::modules::chat::engine::core::rise_conversation_entity_records::RiseMessageLocalPhase;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationPresentation,
};
use crate::modules::chat::engine::core::rise_conversation_resource_repository::RiseConversationResourceKeyPersistence;
use crate::modules::chat::engine::messages::rise_message_contracts::RiseMessageAuxiliaryProjection;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::messages::rise_message_scope::{
    RiseMessageAnchor, RiseMessagePageDirection, RiseMessageScope, RiseMessageWindowConfiguration,
};
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::engine::rise_conversation_domain::{
    RiseConversationDomain, RiseConversationDomainRegistry,
};
use crate::modules::chat::stores::chats::chat_actions::chat_archive_wire_types::{
    ChatArchiveMutationGatewayDTO, RiseStoredArchivedChatPage,
};
use crate::modules::chat::stores::chats::chat_actions::chat_message_requests_wire_types::RiseStoredMessageRequestPage;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    RiseStoredChat, RiseStoredChatDetail, RiseStoredChatPage,
};
use crate::modules::chat::stores::message::message_actions::message_types::MessageDeleteBatchPlanner;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    AddReactionBody, DeleteAllMessagesFromUserBody, DeleteMessagesBody, EditMessageBody, ForwardMessagesBody,
    GetMessagesAroundGatewayDTO, GetMessagesGatewayDTO, MarkChatAsReadBody, MarkMediaPlayedBody, MessageTypingBody,
    MessageWireDTO, PinMessageBody, RemoveReactionBody, UnpinAllMessagesBody,
};
use crate::modules::share_extension::send_handoff::share_send_handoff_models::RiseStoredSharePayloadReply;

const GENERAL_TOPIC_UUID: &str = "00000000-0000-0000-0000-000000000000";
const READY_FOR_CALLS_POLL_INTERVAL: Duration = Duration::from_millis(200);
const AROUND_WINDOW_DEFAULT_LIMIT: i64 = 100;
const AROUND_WINDOW_MAX_LIMIT: i64 = 159;
const ORDERED_CHAT_IDS_KEY: &str = "ordered_chat_ids";
const PINNED_CHAT_IDS_KEY: &str = "pinned_chat_ids";
const SHARE_FAILED_CODE: &str = "share_failed";
const ARCHIVE_STATE_CONFLICT_CODE: &str = "archive_state_conflict";
const REMOTE_REJECTED_CODE: &str = "remote_rejected";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseConversationRepositoryFacadeError {
    UnsupportedRoute(String),
    InvalidBody(String),
    RemoteRejected(String),
}

impl RiseConversationRepositoryFacadeError {
    pub fn error_description(&self) -> String {
        match self {
            Self::UnsupportedRoute(route) => format!("Rise conversation repository does not implement {route}"),
            Self::InvalidBody(route) => format!("Invalid request body for {route}"),
            Self::RemoteRejected(code) => format!("Conversation request was rejected: {code}"),
        }
    }
}

impl fmt::Display for RiseConversationRepositoryFacadeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.error_description())
    }
}

impl std::error::Error for RiseConversationRepositoryFacadeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseConversationActionExecutorError {
    EngineUnavailable,
    InvalidRequestBody,
}

impl RiseConversationActionExecutorError {
    pub fn error_description(&self) -> &'static str {
        match self {
            Self::EngineUnavailable => "Rise conversation engine is unavailable",
            Self::InvalidRequestBody => "Rise conversation request body is not valid JSON",
        }
    }
}

impl fmt::Display for RiseConversationActionExecutorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.error_description())
    }
}

impl std::error::Error for RiseConversationActionExecutorError {}

/// runs work on the main thread, none only before the main hop is installed
async fn on_main<R, W>(work: W) -> Option<R>
where
    R: Send + 'static,
    W: FnOnce(&mut App) -> R + Send + 'static,
{
    let main = RiseEngineMainHop::installed()?;
    main.run(move |cx: AsyncApp| async move { Ok(cx.update(work)) }.boxed_local()).await.ok()
}

struct RiseChatOptimisticRollbackGuard {
    chats: Arc<RiseChatRepository>,
    tokens: Vec<RiseChatOptimisticToken>,
}

impl RiseChatOptimisticRollbackGuard {
    fn new(chats: Arc<RiseChatRepository>) -> Self {
        Self { chats, tokens: Vec::new() }
    }

    fn disarm(&mut self) -> Vec<RiseChatOptimisticToken> {
        std::mem::take(&mut self.tokens)
    }
}

impl Drop for RiseChatOptimisticRollbackGuard {
    fn drop(&mut self) {
        let tokens = self.disarm();
        if tokens.is_empty() {
            return;
        }
        let chats = self.chats.clone();
        drop(RiseAccountRuntime::current().spawn(async move {
            for token in tokens.iter().rev() {
                let _ = chats.rollback_optimistic(token).await;
            }
        }));
    }
}

struct RiseFacadeCancellationGuard {
    on_cancel: Option<Box<dyn FnOnce() + Send>>,
}

impl RiseFacadeCancellationGuard {
    fn new(on_cancel: impl FnOnce() + Send + 'static) -> Self {
        Self {
            on_cancel: Some(Box::new(on_cancel)),
        }
    }

    fn disarm(&mut self) {
        self.on_cancel = None;
    }
}

impl Drop for RiseFacadeCancellationGuard {
    fn drop(&mut self) {
        if let Some(on_cancel) = self.on_cancel.take() {
            on_cancel();
        }
    }
}

struct RiseFacadeAbortOnDrop(AbortHandle);

impl Drop for RiseFacadeAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// entry point for ui action facades, never falls back to the old ws
/// if account startup is in flight it waits for the engine then dispatches to the normalized repos
pub enum RiseConversationActionExecutor {}

impl RiseConversationActionExecutor {
    pub async fn require_domain(cx: &mut AsyncApp) -> anyhow::Result<Arc<RiseConversationDomain>> {
        if let Some(domain) = Self::active_domain(cx) {
            return Ok(domain);
        }
        let preparation = cx.update(|cx| {
            RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx))
        });
        preparation.await?;
        Self::active_domain(cx).ok_or_else(|| RiseConversationActionExecutorError::EngineUnavailable.into())
    }

    pub async fn require_cached_domain(cx: &mut AsyncApp) -> anyhow::Result<Arc<RiseConversationDomain>> {
        if let Some(domain) = Self::active_domain(cx) {
            return Ok(domain);
        }
        let preparation = Arc::new(PreparationFlag::default());
        let flag = preparation.clone();
        let prepare = cx.update(|cx| {
            RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx))
        });
        let task = cx.spawn(async move |_| {
            let result = prepare.await;
            flag.settled.store(true, std::sync::atomic::Ordering::Release);
            result
        });
        while !preparation.settled.load(std::sync::atomic::Ordering::Acquire) {
            if let Some(domain) = Self::active_domain(cx) {
                return Ok(domain);
            }
            cx.background_executor().timer(std::time::Duration::from_millis(16)).await;
        }
        task.await?;
        Self::active_domain(cx).ok_or_else(|| RiseConversationActionExecutorError::EngineUnavailable.into())
    }

    fn active_domain(cx: &mut AsyncApp) -> Option<Arc<RiseConversationDomain>> {
        cx.update(|cx| RiseConversationDomainRegistry::shared(cx).read(cx).active())
    }

    pub async fn call<Request, Response>(
        service: &str,
        method: &str,
        request: &Request,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize,
        Response: DeserializeOwned + Send + 'static,
    {
        let domain = Self::require_domain(cx).await?;
        let body = RiseJSONValue::encode(request)?;
        Self::call_domain(&domain, service, method, body).await
    }

    pub async fn call_domain<Response>(
        domain: &Arc<RiseConversationDomain>,
        service: &str,
        method: &str,
        body: RiseJSONValue,
    ) -> anyhow::Result<Response>
    where
        Response: DeserializeOwned + Send + 'static,
    {
        let domain = domain.clone();
        let service = service.to_owned();
        let method = method.to_owned();
        let task = RiseAccountRuntime::current().spawn(async move {
            let value = domain.repository_call(&service, &method, &body).await?;
            anyhow::Ok(value.decode_as::<Response>()?)
        });
        let _abort = RiseFacadeAbortOnDrop(task.abort_handle());
        task.await.map_err(|_| anyhow::Error::from(CancellationError))?
    }

    pub fn json_body(object: serde_json::Map<String, Value>) -> anyhow::Result<RiseJSONValue> {
        RiseJSONValue::encode(&object).map_err(|_| RiseConversationActionExecutorError::InvalidRequestBody.into())
    }
}

impl RiseConversationDomain {
    pub async fn delete_messages(
        self: &Arc<Self>,
        identifiers: &[String],
        scope: &RiseMessageScope,
        for_everyone: Option<bool>,
    ) -> anyhow::Result<()> {
        let this = self;
        let validate_session = move || async move {
            if this.is_active_account_session().await {
                Ok(())
            } else {
                Err(anyhow::Error::from(CancellationError))
            }
        };
        validate_session().await?;
        if identifiers.len() as i64 > MessageDeleteBatchPlanner::max_messages_per_request() {
            return Err(invalid_body(&route_name(rise_wire_methods::message::SERVICE, rise_wire_methods::message::DELETE_MESSAGES)));
        }
        let mut outgoing_local_ids: Vec<String> = Vec::new();
        let mut remote_ids: Vec<String> = Vec::new();
        for identifier in identifiers.iter().collect::<HashSet<_>>() {
            if let Some(entity) = self.messages.message_entity(identifier).await? {
                if RiseMessageWireCodec::is_tombstone(&entity.data) {
                    continue;
                }
                let record = RiseMessageWireCodec::record(&entity.data)?;
                RiseMessageRepository::validate_deletion_scope(&record.wire, scope)?;
                if record.local_phase != RiseMessageLocalPhase::Remote {
                    outgoing_local_ids.push(entity.local_id.clone());
                    continue;
                }
            }
            remote_ids.push(identifier.clone());
        }
        validate_session().await?;
        for local_id in &outgoing_local_ids {
            self.hide_outgoing_message_from_presentation(local_id).await;
            self.messages.cancel_outgoing(local_id, scope).await?;
        }
        if remote_ids.is_empty() {
            return Ok(());
        }
        let remote_message_ids: HashSet<String> = remote_ids.iter().cloned().collect();
        let expected_account_epoch = on_main(|_| SaiAccountCacheScope::active_epoch()).await.flatten();
        validate_session().await?;
        let main_domain = self.clone();
        let chat_id = scope.chat_id.clone();
        let deleted_ids = remote_message_ids.clone();
        let finish = on_main(move |cx| {
            if !main_domain.is_registry_active(cx)
                || !SaiAccountCacheScope::is_current_account_epoch(
                    Some(&main_domain.account_id),
                    expected_account_epoch.as_deref(),
                )
            {
                return Err(anyhow::Error::from(CancellationError));
            }
            Ok(RiseConversationPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.begin_message_tab_deletion(&chat_id, &deleted_ids, cx)))
        })
        .await
        .ok_or_else(|| anyhow::Error::from(CancellationError))??;
        let finish_guard = RiseFacadeCancellationGuard::new(move || {
            if let Some(main) = RiseEngineMainHop::installed() {
                main.post(finish);
            }
        });
        let outcome = async {
            self.messages
                .delete(&remote_ids, scope, for_everyone, Some(validate_session))
                .await?;
            validate_session().await?;
            self.drop_media_gallery_messages(&remote_message_ids, None, None).await;
            anyhow::Ok(())
        }
        .await;
        drop(finish_guard);
        on_main(|_| ()).await;
        outcome
    }

    /// closed boundary for action facades
    /// every route uses normalized repos, nothing falls through to the old transport or its cache
    pub async fn repository_call(
        self: &Arc<Self>,
        service: &str,
        method: &str,
        body: &RiseJSONValue,
    ) -> anyhow::Result<RiseJSONValue> {
        let route = route_name(service, method);
        let object = match body.object_value() {
            Some(object) if !service.is_empty() => object,
            _ => return Err(invalid_body(&route)),
        };
        match (service, method) {
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHATS) => {
                self.chat_list_response(RiseChatListKind::Inbox, object).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_ARCHIVED_CHATS) => {
                self.chat_list_response(RiseChatListKind::Archive, object).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_MESSAGE_REQUESTS) => {
                self.chat_list_response(RiseChatListKind::Requests, object).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_BY_ID) => self.chat_by_id_response(body, object).await,
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::JOIN_CHAT | rise_wire_methods::chat::JOIN_CHAT_BY_LINK) => {
                self.join_chat_mutation(method, body, object).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::ARCHIVE_CHAT) => {
                self.chat_mutation(method, body, object, RiseChatOptimisticOperation::Archive).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNARCHIVE_CHAT) => {
                self.chat_mutation(method, body, object, RiseChatOptimisticOperation::Unarchive).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::MUTE_CHAT) => {
                let operation = RiseChatOptimisticOperation::Mute {
                    is_muted: true,
                    until: number(object, rise_key::chat::MUTE_UNTIL),
                };
                self.chat_mutation(method, body, object, operation).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNMUTE_CHAT) => {
                let operation = RiseChatOptimisticOperation::Mute {
                    is_muted: false,
                    until: None,
                };
                self.chat_mutation(method, body, object, operation).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UPDATE_CHAT_NOTIFICATIONS) => {
                let operation = RiseChatOptimisticOperation::Notifications(RiseChatNotificationPreference::new(
                    bool_value(object, rise_key::chat::MESSAGES).unwrap_or(true),
                    bool_value(object, rise_key::chat::MENTIONS).unwrap_or(true),
                    bool_value(object, rise_key::chat::REPLIES),
                    bool_value(object, rise_key::chat::REACTIONS).unwrap_or(true),
                    bool_value(object, rise_key::chat::EVENTS).unwrap_or(true),
                ));
                self.chat_mutation(method, body, object, operation).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UPDATE_CHAT) => {
                let mut patch = object.clone();
                patch.remove(rise_key::bot::USER_ID);
                patch.remove(rise_key::bot::CHAT_ID);
                patch.remove(rise_key::bot::ID);
                self.chat_mutation(method, body, object, RiseChatOptimisticOperation::Patch(patch)).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::LEAVE_CHAT) => {
                self.chat_mutation(method, body, object, RiseChatOptimisticOperation::Leave).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::DELETE_CHAT) => self.delete_chat_mutation(body, object).await,
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNPIN_CHAT)
                if string(object, &[rise_key::chat::FOLDER_ID]).is_some() =>
            {
                self.folder_pin_mutation(rise_wire_methods::chat::UNPIN_CHAT, body).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::UNPIN_CHAT) => {
                self.chat_mutation(method, body, object, RiseChatOptimisticOperation::Pin(false)).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::REORDER_PINNED_CHATS) => {
                self.reorder_pinned_chats(body, object).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::RESOLVE_MESSAGE_REQUEST) => {
                self.resolve_message_request(body, object).await
            }
            (rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::DELETE_ALL_MESSAGE_REQUESTS) => {
                self.delete_all_message_requests(body).await
            }
            (
                rise_wire_methods::chat::SERVICE,
                rise_wire_methods::chat::CREATE_TOPIC
                | rise_wire_methods::chat::UPDATE_TOPIC
                | rise_wire_methods::chat::GET_GROUP_VOICE_CHANNEL
                | rise_wire_methods::chat::UPDATE_GROUP_VOICE_CHANNEL,
            ) => self.raw_mutation_when_connected(service, method, body).await,

            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MESSAGES) => self.message_list_response(object).await,
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MESSAGES_AROUND) => {
                self.message_around_response(object).await
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::EDIT_MESSAGE) => {
                self.edit_message_route(service, method, body, object, &route).await
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::DELETE_MESSAGES) => {
                let request: DeleteMessagesBody = decode(body, &route)?;
                let identifiers = request.message_ids.unwrap_or_default();
                let Some(scope) = self.message_scope_for(identifiers.first().map(String::as_str)).await? else {
                    return Err(invalid_body(&route));
                };
                self.delete_messages(&identifiers, &scope, request.for_everyone).await?;
                Ok(success_value())
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::PIN_MESSAGE | rise_wire_methods::message::UNPIN_MESSAGE) => {
                let request: PinMessageBody = decode(body, &route)?;
                self.messages
                    .set_pinned(
                        method == rise_wire_methods::message::PIN_MESSAGE,
                        &request.message_id,
                        &RiseMessageScope::new(request.chat_id, None, None),
                        request.for_everyone,
                        request.silent,
                    )
                    .await?;
                Ok(success_value())
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::UNPIN_ALL_MESSAGES) => {
                let request: UnpinAllMessagesBody = decode(body, &route)?;
                encode(&self.messages.unpin_all(&request.chat_id).await?)
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::DELETE_ALL_MESSAGES_FROM_USER) => {
                let request: DeleteAllMessagesFromUserBody = decode(body, &route)?;
                encode(
                    &self
                        .messages
                        .delete_all_from_user(&request.chat_id, &request.target_user_id)
                        .await?,
                )
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::FORWARD_MESSAGES) => {
                let request: ForwardMessagesBody = decode(body, &route)?;
                encode(&self.messages.forward(request).await?)
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::SHARE_PAYLOAD) => {
                self.share_payload_route(service, method, body, &route).await
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::ADD_REACTION) => {
                let request: AddReactionBody = decode(body, &route)?;
                self.messages.set_reaction(&request.reaction, true, &request.message_id).await?;
                Ok(success_value())
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::REMOVE_REACTION) => {
                let request: RemoveReactionBody = decode(body, &route)?;
                self.messages.set_reaction(&request.reaction, false, &request.message_id).await?;
                Ok(success_value())
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::MARK_CHAT_AS_READ) => {
                let request: MarkChatAsReadBody = decode(body, &route)?;
                let clear_marks = request.clear_marks.unwrap_or(false);
                let scope = RiseMessageScope::new(request.chat_id.clone(), request.topic_id, None);
                self.messages.mark_read(&scope, clear_marks, request.up_to_message_id.as_deref()).await?;
                if clear_marks {
                    self.drop_unread_marks(&request.chat_id).await;
                }
                Ok(success_value())
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::SET_CHAT_UNREAD_MARK) => {
                self.raw_mutation_when_connected(service, method, body).await
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::MARK_MEDIA_AS_PLAYED) => {
                let request: MarkMediaPlayedBody = decode(body, &route)?;
                let scope = RiseMessageScope::new(request.chat_id, None, None);
                self.messages.mark_media_played(&request.message_id, &scope, request.closed).await?;
                Ok(success_value())
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::MESSAGE_TYPING) => {
                let request: MessageTypingBody = decode(body, &route)?;
                let Some(chat_id) = request.chat_id.filter(|chat_id| !chat_id.is_empty()) else {
                    return Err(invalid_body(&route));
                };
                let scope = RiseMessageScope::new(chat_id, request.topic_id, None);
                self.messages
                    .send_typing(request.is_typing.unwrap_or(false), request.action.as_deref(), &scope)
                    .await?;
                Ok(success_value())
            }
            (rise_wire_methods::message::SERVICE, rise_wire_methods::message::CREATE_MESSAGE) => {
                Err(RiseConversationRepositoryFacadeError::UnsupportedRoute(route).into())
            }
            _ => Err(RiseConversationRepositoryFacadeError::UnsupportedRoute(route).into()),
        }
    }

    async fn edit_message_route(
        &self,
        service: &str,
        method: &str,
        body: &RiseJSONValue,
        object: &BTreeMap<String, RiseJSONValue>,
        route: &str,
    ) -> anyhow::Result<RiseJSONValue> {
        let request: EditMessageBody = decode(body, route)?;
        let Some(scope) = self.message_scope_for(Some(&request.message_id)).await? else {
            return Err(invalid_body(route));
        };
        if !object.contains_key(rise_key::bot::MEDIA_ITEMS) {
            self.messages
                .edit(
                    &request.message_id,
                    &scope,
                    request.content.as_deref().unwrap_or(""),
                    request.entities.as_deref(),
                    request.document.as_deref(),
                    request.link_preview_type.as_deref(),
                )
                .await?;
            return Ok(success_value());
        }
        let optional = |value: &Option<String>| value.clone().map_or(RiseJSONValue::Null, RiseJSONValue::String);
        let mut patch: BTreeMap<String, RiseJSONValue> = BTreeMap::from([
            (
                rise_key::bot::CONTENT.to_owned(),
                RiseJSONValue::String(request.content.clone().unwrap_or_default()),
            ),
            (rise_key::bot::ENTITIES.to_owned(), optional(&request.entities)),
            (rise_key::bot::DOCUMENT.to_owned(), optional(&request.document)),
        ]);
        for key in [rise_key::message::MEDIA_ITEMS, rise_key::chat::CAPTION, rise_key::message::ENTITIES] {
            if let Some(value) = object.get(key) {
                patch.insert(key.to_owned(), value.clone());
            }
        }
        let checkpoint = self
            .messages
            .begin_patch(&request.message_id, Some(&scope.chat_id), move |wire: &MessageWireDTO| {
                RiseMessageWireCodec::editing(wire, &patch, true)
            })
            .await?;
        let mut rollback_guard = {
            let messages = self.messages.clone();
            let checkpoint = checkpoint.clone();
            let chat_id = scope.chat_id.clone();
            RiseFacadeCancellationGuard::new(move || {
                drop(RiseAccountRuntime::current().spawn(async move {
                    let _ = messages.rollback(&checkpoint, Some(&chat_id)).await;
                }));
            })
        };
        let outcome = async {
            let response = self.raw_mutation(service, method, body).await?;
            validate_success(&response)?;
            self.messages.acknowledge(&checkpoint).await?;
            anyhow::Ok(response)
        }
        .await;
        rollback_guard.disarm();
        match outcome {
            Ok(response) => {
                self.messages
                    .invalidate_auxiliary_scope(
                        &RiseMessageScope::new(scope.chat_id.clone(), checkpoint.topic_id.clone(), None),
                        &RiseMessageAuxiliaryProjection::all(),
                    )
                    .await;
                Ok(response)
            }
            Err(error) => {
                let _ = self.messages.rollback(&checkpoint, Some(&scope.chat_id)).await;
                Err(error)
            }
        }
    }

    async fn share_payload_route(
        &self,
        service: &str,
        method: &str,
        body: &RiseJSONValue,
        route: &str,
    ) -> anyhow::Result<RiseJSONValue> {
        let rows = self.messages.install_share_payload_optimistic_rows(body).await;
        let mut fail_guard = {
            let messages = self.messages.clone();
            let rows = rows.clone();
            RiseFacadeCancellationGuard::new(move || {
                tracing::warn!(target: "riseonly::X1057", rows = rows.len(), "share route dropped before reply: rows failed");
                drop(RiseAccountRuntime::current().spawn(async move {
                    messages.fail_optimistic_fanout_rows(&rows, SHARE_FAILED_CODE).await;
                }));
            })
        };
        let outcome = async {
            let response = self.raw_mutation_when_connected(service, method, body).await?;
            let canon: SharePayloadResponse = decode(&response, route)?;
            anyhow::Ok((response, RiseStoredSharePayloadReply::from_canon(&canon)))
        }
        .await;
        fail_guard.disarm();
        match outcome {
            Ok((response, decoded)) => {
                self.messages.apply_share_payload_results(&decoded, &rows).await;
                Ok(response)
            }
            Err(error) => {
                tracing::warn!(target: "riseonly::X1057", rows = rows.len(), error = %error, "share route failed: rows failed");
                self.messages.fail_optimistic_fanout_rows(&rows, SHARE_FAILED_CODE).await;
                Err(error)
            }
        }
    }

    async fn chat_list_response(
        &self,
        kind: RiseChatListKind,
        body: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<RiseJSONValue> {
        if matches!(kind, RiseChatListKind::Inbox)
            && let Some(query) = string(body, &[rise_key::chat::Q]).filter(|query| !rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines(query).is_empty())
        {
            let cursor = string(body, &[rise_key::message::RELATIVE_ID]);
            let page = self.chats.search_inbox_page(&query, cursor.as_deref()).await?;
            return encode(&page);
        }
        if string(body, &[rise_key::message::RELATIVE_ID]).is_none() {
            self.chats.refresh(kind).await?;
        } else {
            self.chats.load_more(kind).await?;
        }
        let snapshot = self.chats.snapshot(kind).await?;
        self.accept(snapshot.clone()).await;
        let rows: Vec<RiseStoredChat> = snapshot.rows.iter().map(|row| row.wire.clone()).collect();
        match kind {
            RiseChatListKind::Inbox => encode(&RiseStoredChatPage::new(
                Some(rows),
                snapshot.cursor.clone(),
                Some(snapshot.loaded_row_count),
                Some(snapshot.has_more),
                Some(true),
                None,
            )),
            RiseChatListKind::Archive => encode(&RiseStoredArchivedChatPage::new(
                Some(rows),
                snapshot.cursor.clone(),
                Some(snapshot.loaded_row_count),
                Some(snapshot.has_more),
                snapshot.server_total_count,
                snapshot.server_total_unread_count,
                snapshot.server_unread_chats_count,
                Some(true),
                None,
            )),
            RiseChatListKind::Requests => encode(&RiseStoredMessageRequestPage {
                chats: Some(rows),
                has_more: Some(snapshot.has_more),
                relative_id: snapshot.cursor.clone(),
                total_count: snapshot.server_total_count,
            }),
        }
    }

    async fn chat_by_id_response(
        self: &Arc<Self>,
        body: &RiseJSONValue,
        object: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<RiseJSONValue> {
        let Some(chat_id) = string(object, &[rise_key::message::CHAT_ID, rise_key::chat::ID]) else {
            return Err(invalid_body(&route_name(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_BY_ID)));
        };
        if let Some(stored) = self.chats.stored_chat_id(&chat_id, false).await?
            && !stored.record.tombstone.as_ref().is_some_and(|tombstone| tombstone.is_confirmed())
        {
            let is_archived = stored.record.list_kind == Some(RiseChatListKind::Archive);
            self.accept_chat_detail(stored.record.wire.clone(), Some(is_archived)).await;
            let user_id = string(object, &[rise_key::message::USER_ID]).unwrap_or_else(|| self.account_id.clone());
            if self.insert_chat_detail_revalidation_in_flight(&chat_id) {
                let this = self.clone();
                let revalidated_chat_id = chat_id.clone();
                drop(RiseAccountRuntime::current().spawn(async move {
                    this.revalidate_chat_detail(&revalidated_chat_id, Some(&user_id)).await;
                    this.finish_chat_detail_revalidation(&revalidated_chat_id);
                }));
            }
            return encode(&stored_chat_detail(stored.record.wire.clone(), Some(is_archived)));
        }
        let _ = self.chats.refresh(RiseChatListKind::Inbox).await;
        let stored = self.chats.stored_chat_id(&chat_id, false).await?;
        let removed_here = stored
            .as_ref()
            .and_then(|stored| stored.record.tombstone.as_ref())
            .is_some_and(|tombstone| tombstone.is_confirmed());
        if !removed_here && let Some(stored) = stored {
            let is_archived = stored.record.list_kind == Some(RiseChatListKind::Archive);
            self.accept_chat_detail(stored.record.wire.clone(), Some(is_archived)).await;
            return encode(&stored_chat_detail(stored.record.wire, Some(is_archived)));
        }
        let response = self
            .auxiliary
            .compatibility_call(
                rise_wire_methods::chat::SERVICE,
                rise_wire_methods::chat::GET_CHAT_BY_ID,
                body.clone(),
                Some(&chat_by_id_cache_identity(&chat_id)),
                RiseConversationResourceKeyPersistence::Persistent,
                false,
            )
            .await?;
        let canon: ChatById = decode(
            &response,
            &route_name(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_BY_ID),
        )?;
        let detail = RiseStoredChatDetail::from_canon(&canon);
        if let Some(wire) = detail.chat {
            if removed_here {
                self.accept_chat_detail(wire, detail.is_archived).await;
            } else {
                let stored = self.chats.store_chat_detail(wire).await?;
                self.accept_chat_detail(stored.record.wire, detail.is_archived).await;
            }
        }
        Ok(response)
    }

    async fn join_chat_mutation(
        &self,
        method: &str,
        body: &RiseJSONValue,
        object: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<RiseJSONValue> {
        let Some(chat_id) = string(object, &[rise_key::message::CHAT_ID, rise_key::chat::ID]) else {
            return Err(invalid_body(&route_name(rise_wire_methods::chat::SERVICE, method)));
        };
        let response = self.raw_mutation(rise_wire_methods::chat::SERVICE, method, body).await?;
        validate_success(&response)?;
        let rights_chat_id = chat_id.clone();
        let _ = on_main(move |cx| {
            let Some(hooks) = chat_engine_hooks() else {
                return;
            };
            (hooks.invalidate_overwrite_caches)(&rights_chat_id, cx);
            (hooks.invalidate_effective_permissions)(&rights_chat_id, cx);
            (hooks.load_effective_permissions)(&rights_chat_id, None, true, cx);
        })
        .await;
        let authoritative = chat_from(&response).ok().flatten();
        let Some(wire) = self.chats.adopt_joined_chat(&chat_id, authoritative).await? else {
            return Ok(response);
        };
        self.accept_chat_detail(wire, None).await;
        Ok(response)
    }

    async fn delete_chat_mutation(
        &self,
        body: &RiseJSONValue,
        object: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<RiseJSONValue> {
        let Some(chat_id) = string(object, &[rise_key::message::CHAT_ID, rise_key::chat::ID]) else {
            return Err(invalid_body(&route_name(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::DELETE_CHAT)));
        };
        let clears_history = bool_value(object, rise_key::chat::CLEAR_HISTORY) == Some(true);
        let keeps_chat = self.chats.stored_chat_id(&chat_id, false).await?.is_some_and(|stored| {
            stored.record.tombstone.is_none()
                && (clears_history || stored.record.wire.to_chat_info().is_official_bot_chat())
        });
        if !keeps_chat {
            let response = self
                .chat_mutation(rise_wire_methods::chat::DELETE_CHAT, body, object, RiseChatOptimisticOperation::Delete)
                .await?;
            if let Err(error) = self.clear_deleted_chat_local_history(&chat_id).await {
                let message = format!("Rise clear deleted history {chat_id} failed: {error}");
                on_main(move |cx| Sai::log("Chat", move || message, SaiLogKind::Network, cx)).await;
            }
            return Ok(response);
        }

        let response = self
            .raw_mutation(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::DELETE_CHAT, body)
            .await?;
        validate_success(&response)?;
        self.clear_official_bot_local_history(&chat_id).await?;
        self.revalidate_chat_detail(&chat_id, string(object, &[rise_key::message::USER_ID]).as_deref())
            .await;
        Ok(response)
    }

    /// stored last message from a topic this account cant read keeps its text
    /// server preview only covers readable topics so this revalidates it
    /// reread skips the check, a new topic changes forum rows
    pub async fn revalidate_topic_scoped_preview(&self, chat_id: &str, topic_id: Option<&str>, reread: bool) {
        let Ok(Some(stored)) = self.chats.stored_chat_id(chat_id, false).await else {
            return;
        };
        if stored.record.tombstone.is_some() {
            return;
        }
        let wire = &stored.record.wire;
        let counts_unread = wire.topics_enabled == Some(true)
            && (wire.resolved_unread_count() > 0
                || wire.resolved_unread_mentions_count() > 0
                || wire.resolved_unread_replies_count() > 0
                || wire.resolved_unread_reactions_count() > 0);
        let preview_topic = wire.last_message.as_ref().and_then(|preview| preview.topic_id.as_deref());
        let preview_from_topic = preview_is_from_topic(preview_topic, topic_id);
        if !reread && !counts_unread && !preview_from_topic {
            return;
        }
        let account_id = self.account_id.clone();
        self.revalidate_chat_detail(chat_id, Some(&account_id)).await;
        if counts_unread || preview_from_topic {
            self.refresh_chat_row(chat_id).await;
        }
        if counts_unread {
            tracing::warn!(target: "riseonly::X1857", chat = chat_id, "topic scoped unread: badge summary refresh");
            self.refresh_unread_badge().await;
        }
    }

    fn finish_chat_detail_revalidation(&self, chat_id: &str) {
        self.remove_chat_detail_revalidation_in_flight(chat_id);
    }

    async fn revalidate_chat_detail(&self, chat_id: &str, user_id: Option<&str>) {
        let mut request: BTreeMap<String, RiseJSONValue> = BTreeMap::from([(
            rise_key::bot::CHAT_ID.to_owned(),
            RiseJSONValue::String(chat_id.to_owned()),
        )]);
        if let Some(user_id) = user_id.filter(|user_id| !user_id.is_empty()) {
            request.insert(rise_key::bot::USER_ID.to_owned(), RiseJSONValue::String(user_id.to_owned()));
        }
        let _ = self.revalidate_chat_detail_request(chat_id, RiseJSONValue::Object(request)).await;
    }

    async fn revalidate_chat_detail_request(&self, chat_id: &str, body: RiseJSONValue) -> anyhow::Result<()> {
        let response = self
            .auxiliary
            .compatibility_call(
                rise_wire_methods::chat::SERVICE,
                rise_wire_methods::chat::GET_CHAT_BY_ID,
                body,
                Some(&chat_by_id_cache_identity(chat_id)),
                RiseConversationResourceKeyPersistence::Persistent,
                false,
            )
            .await?;
        let canon: ChatById = decode(
            &response,
            &route_name(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::GET_CHAT_BY_ID),
        )?;
        let Some(wire) = RiseStoredChatDetail::from_canon(&canon).chat else {
            return Ok(());
        };
        let current = self.chats.stored_chat_id(chat_id, false).await?;
        if current.is_some_and(|stored| stored.record.tombstone.is_some()) {
            return Ok(());
        }
        let stored = self.chats.store_chat_detail(wire).await?;
        let is_archived = stored.record.list_kind == Some(RiseChatListKind::Archive);
        self.accept_chat_detail(stored.record.wire, Some(is_archived)).await;
        Ok(())
    }

    pub async fn apply_direct_peer_logo(self: &Arc<Self>, chat_id: &str, peer_user_id: &str, logo: &str) {
        let expected_epoch = on_main(|_| SaiAccountCacheScope::active_epoch()).await.flatten();
        let this = self.clone();
        let validate_session = move || {
            let this = this.clone();
            let expected_epoch = expected_epoch.clone();
            async move {
                let is_current = on_main(move |cx| {
                    this.is_registry_active(cx)
                        && SaiAccountCacheScope::is_current_account_epoch(
                            Some(&this.account_id),
                            expected_epoch.as_deref(),
                        )
                })
                .await;
                if is_current == Some(true) {
                    Ok(())
                } else {
                    Err(anyhow::Error::from(CancellationError))
                }
            }
        };
        let _ = self
            .chats
            .patch_direct_peer_logo(chat_id, peer_user_id, logo, validate_session)
            .await;
    }

    async fn chat_mutation(
        &self,
        method: &str,
        body: &RiseJSONValue,
        object: &BTreeMap<String, RiseJSONValue>,
        operation: RiseChatOptimisticOperation,
    ) -> anyhow::Result<RiseJSONValue> {
        let Some(chat_id) = string(object, &[rise_key::message::CHAT_ID, rise_key::chat::ID]) else {
            return Err(invalid_body(&route_name(rise_wire_methods::chat::SERVICE, method)));
        };
        let token = self.chats.apply_optimistic(&chat_id, operation).await?;
        let mut rollback_guard = RiseChatOptimisticRollbackGuard::new(self.chats.clone());
        rollback_guard.tokens.push(token.clone());
        let is_archive_mutation = method == rise_wire_methods::chat::ARCHIVE_CHAT || method == rise_wire_methods::chat::UNARCHIVE_CHAT;
        let outcome = async {
            let response = if is_archive_mutation {
                self.raw_mutation_when_connected(rise_wire_methods::chat::SERVICE, method, body).await?
            } else {
                self.raw_mutation(rise_wire_methods::chat::SERVICE, method, body).await?
            };
            let authoritative_chat = if is_archive_mutation {
                let decoded: ChatArchiveMutationGatewayDTO =
                    decode(&response, &route_name(rise_wire_methods::chat::SERVICE, method))?;
                let expected_archived = method == rise_wire_methods::chat::ARCHIVE_CHAT;
                if !decoded.success || decoded.is_archived != expected_archived {
                    return Err(RiseConversationRepositoryFacadeError::RemoteRejected(
                        ARCHIVE_STATE_CONFLICT_CODE.to_owned(),
                    )
                    .into());
                }
                decoded.chat.as_ref().map(RiseStoredChat::from_canon)
            } else {
                validate_success(&response)?;
                chat_from(&response).ok().flatten()
            };
            self.chats
                .confirm_optimistic_authoritative_chat(&token, authoritative_chat)
                .await?;
            anyhow::Ok(response)
        }
        .await;
        rollback_guard.disarm();
        match outcome {
            Ok(response) => Ok(response),
            Err(error) => {
                if is_archive_mutation {
                    let chats = self.chats.clone();
                    drop(RiseAccountRuntime::current().spawn(async move {
                        let _ = chats.rollback_optimistic(&token).await;
                        let _ = chats.refresh(RiseChatListKind::Inbox).await;
                        let _ = chats.refresh(RiseChatListKind::Archive).await;
                    }));
                    return Err(error);
                }
                let _ = self.chats.rollback_optimistic(&token).await;
                Err(error)
            }
        }
    }

    async fn folder_pin_mutation(&self, method: &str, body: &RiseJSONValue) -> anyhow::Result<RiseJSONValue> {
        let response = self.raw_mutation(rise_wire_methods::chat::SERVICE, method, body).await?;
        validate_success(&response)?;
        Ok(response)
    }

    async fn reorder_pinned_chats(
        &self,
        body: &RiseJSONValue,
        object: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<RiseJSONValue> {
        if string(object, &[rise_key::chat::FOLDER_ID]).is_some() {
            return self.folder_pin_mutation(rise_wire_methods::chat::REORDER_PINNED_CHATS, body).await;
        }
        let ids = strings(object, &[rise_key::message::CHAT_IDS, ORDERED_CHAT_IDS_KEY, PINNED_CHAT_IDS_KEY]);
        if ids.is_empty() {
            return Err(invalid_body(&route_name(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::REORDER_PINNED_CHATS)));
        }
        let mut rollback_guard = RiseChatOptimisticRollbackGuard::new(self.chats.clone());
        let outcome = async {
            for (position, id) in ids.iter().enumerate() {
                let token = self
                    .chats
                    .apply_optimistic(
                        id,
                        RiseChatOptimisticOperation::ReorderPinned {
                            position: Some(position as i64),
                        },
                    )
                    .await?;
                rollback_guard.tokens.push(token);
            }
            self.confirm_tokens_after_mutation(rise_wire_methods::chat::REORDER_PINNED_CHATS, body, &rollback_guard.tokens)
                .await
        }
        .await;
        self.finish_token_mutation(rollback_guard, outcome).await
    }

    async fn resolve_message_request(
        &self,
        body: &RiseJSONValue,
        object: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<RiseJSONValue> {
        let chat_id = string(object, &[rise_key::message::CHAT_ID]);
        let action = string(object, &[rise_key::chat::ACTION]).map(|action| action.to_uppercase());
        let (Some(chat_id), Some(action)) = (chat_id, action) else {
            return Err(invalid_body(&route_name(rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::RESOLVE_MESSAGE_REQUEST)));
        };
        let operation = if action == ChatMessageRequestAction::Accept.wire() {
            RiseChatOptimisticOperation::Unarchive
        } else {
            RiseChatOptimisticOperation::Delete
        };
        let chat_object = BTreeMap::from([(rise_key::bot::CHAT_ID.to_owned(), RiseJSONValue::String(chat_id))]);
        self.chat_mutation(rise_wire_methods::chat::RESOLVE_MESSAGE_REQUEST, body, &chat_object, operation).await
    }

    async fn delete_all_message_requests(&self, body: &RiseJSONValue) -> anyhow::Result<RiseJSONValue> {
        let snapshot = self.chats.snapshot(RiseChatListKind::Requests).await?;
        let mut rollback_guard = RiseChatOptimisticRollbackGuard::new(self.chats.clone());
        let outcome = async {
            for row in &snapshot.rows {
                let token = self
                    .chats
                    .apply_optimistic(&row.wire.id, RiseChatOptimisticOperation::Delete)
                    .await?;
                rollback_guard.tokens.push(token);
            }
            self.confirm_tokens_after_mutation(rise_wire_methods::chat::DELETE_ALL_MESSAGE_REQUESTS, body, &rollback_guard.tokens)
                .await
        }
        .await;
        self.finish_token_mutation(rollback_guard, outcome).await
    }

    async fn confirm_tokens_after_mutation(
        &self,
        method: &str,
        body: &RiseJSONValue,
        tokens: &[RiseChatOptimisticToken],
    ) -> anyhow::Result<RiseJSONValue> {
        let response = self.raw_mutation(rise_wire_methods::chat::SERVICE, method, body).await?;
        validate_success(&response)?;
        for token in tokens {
            self.chats.confirm_optimistic(token).await?;
        }
        Ok(response)
    }

    async fn finish_token_mutation(
        &self,
        mut rollback_guard: RiseChatOptimisticRollbackGuard,
        outcome: anyhow::Result<RiseJSONValue>,
    ) -> anyhow::Result<RiseJSONValue> {
        let tokens = rollback_guard.disarm();
        if outcome.is_err() {
            for token in tokens.iter().rev() {
                let _ = self.chats.rollback_optimistic(token).await;
            }
        }
        outcome
    }

    async fn message_list_response(&self, object: &BTreeMap<String, RiseJSONValue>) -> anyhow::Result<RiseJSONValue> {
        let scope = message_scope(object, &route_name(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MESSAGES))?;
        self.observe_messages(&scope).await?;
        if string(object, &[rise_key::message::RELATIVE_ID]).is_none() {
            self.messages.load_initial(&scope).await?;
        } else {
            let direction = if bool_value(object, rise_key::message::UP) == Some(true) {
                RiseMessagePageDirection::Older
            } else {
                RiseMessagePageDirection::Newer
            };
            self.messages.load_page(&scope, direction).await?;
        }
        let snapshot = self
            .messages
            .snapshot(&scope, RiseMessageAnchor::Latest, RiseMessageWindowConfiguration::presentation())
            .await?;
        let is_failed = snapshot.load_state == RiseEntityViewLoadState::Failed;
        encode(&GetMessagesGatewayDTO {
            chat_id: Some(scope.chat_id.clone()),
            limit: Some(snapshot.rows.len() as i64),
            relative_id: snapshot.before_cursor.clone(),
            is_have_more: Some(snapshot.has_more_before),
            messages: Some(snapshot.rows.iter().map(|row| row.wire.clone()).collect()),
            success: Some(!is_failed),
            error: snapshot.error_code.clone(),
        })
    }

    async fn message_around_response(
        &self,
        object: &BTreeMap<String, RiseJSONValue>,
    ) -> anyhow::Result<RiseJSONValue> {
        let route = route_name(rise_wire_methods::message::SERVICE, rise_wire_methods::message::GET_MESSAGES_AROUND);
        let scope = message_scope(object, &route)?;
        let Some(anchor) = string(object, &[rise_key::message::ANCHOR]) else {
            return Err(invalid_body(&route));
        };
        let limit = |key: &str| int(object, key).unwrap_or(AROUND_WINDOW_DEFAULT_LIMIT).clamp(0, AROUND_WINDOW_MAX_LIMIT);
        let configuration = RiseMessageWindowConfiguration::new(
            limit(rise_key::message::BEFORE_LIMIT),
            limit(rise_key::message::AFTER_LIMIT),
        );
        self.messages
            .jump(&scope, RiseMessageAnchor::ServerID(anchor.clone()), configuration)
            .await?;
        let observation_generation = self
            .observe_messages_full(
                &scope,
                RiseMessageAnchor::ServerID(anchor.clone()),
                configuration,
                true,
                false,
            )
            .await?;
        let snapshot = self
            .messages
            .snapshot_anchor(&scope, RiseMessageAnchor::ServerID(anchor.clone()), configuration, true)
            .await?;
        self.accept_observation_generation(snapshot.clone(), observation_generation).await;
        encode(&GetMessagesAroundGatewayDTO {
            success: Some(true),
            error: None,
            chat_id: Some(scope.chat_id.clone()),
            anchor_message_id: Some(anchor.clone()),
            anchor_relative_id: Some(anchor),
            before_limit: Some(configuration.before),
            after_limit: Some(configuration.after),
            older_relative_id: snapshot.before_cursor.clone(),
            has_older: Some(snapshot.has_more_before),
            newer_relative_id: snapshot.after_cursor.clone(),
            has_newer: Some(snapshot.has_more_after),
            messages: Some(snapshot.rows.iter().map(|row| row.wire.clone()).collect()),
        })
    }

    async fn message_scope_for(&self, identifier: Option<&str>) -> anyhow::Result<Option<RiseMessageScope>> {
        let Some(identifier) = identifier else {
            return Ok(None);
        };
        let Some(entity) = self.messages.message_entity(identifier).await? else {
            return Ok(None);
        };
        if RiseMessageWireCodec::is_tombstone(&entity.data) {
            return Ok(None);
        }
        let Ok(record) = RiseMessageWireCodec::record(&entity.data) else {
            return Ok(None);
        };
        let Some(chat_id) = RiseMessageWireCodec::normalized(record.wire.chat_id.as_deref()) else {
            return Ok(None);
        };
        Ok(Some(RiseMessageScope::new(chat_id, record.wire.topic_id.clone(), None)))
    }

    async fn raw_mutation(&self, service: &str, method: &str, body: &RiseJSONValue) -> anyhow::Result<RiseJSONValue> {
        self.auxiliary
            .compatibility_call(
                service,
                method,
                body.clone(),
                None,
                RiseConversationResourceKeyPersistence::MemoryOnly,
                false,
            )
            .await
    }

    async fn raw_mutation_when_connected(
        &self,
        service: &str,
        method: &str,
        body: &RiseJSONValue,
    ) -> anyhow::Result<RiseJSONValue> {
        let deadline = RiseClock::monotonic() + RiseCanonEndpoints::required(service, method).client_timeout();
        while !self.engine.is_ready_for_calls(false).await? {
            if RiseClock::monotonic() >= deadline {
                return Err(RiseWireError::NotConnected.into());
            }
            RiseAccountRuntime::current()
                .spawn(async move { tokio::time::sleep(READY_FOR_CALLS_POLL_INTERVAL).await })
                .await
                .map_err(|_| anyhow::Error::from(CancellationError))?;
        }
        self.raw_mutation(service, method, body).await
    }
}

fn route_name(service: &str, method: &str) -> String {
    format!("{service}.{method}")
}

fn invalid_body(route: &str) -> anyhow::Error {
    RiseConversationRepositoryFacadeError::InvalidBody(route.to_owned()).into()
}

fn chat_by_id_cache_identity(chat_id: &str) -> String {
    format!("chat-by-id-{chat_id}")
}

fn stored_chat_detail(chat: RiseStoredChat, is_archived: Option<bool>) -> RiseStoredChatDetail {
    RiseStoredChatDetail {
        chat: Some(chat),
        is_archived,
        success: None,
        error: None,
    }
}

fn preview_is_from_topic(preview_topic: Option<&str>, topic_id: Option<&str>) -> bool {
    let Some(preview_topic_uuid) = preview_topic.and_then(Uuid::from_uuid_string) else {
        return false;
    };
    if preview_topic_uuid.uuid_string() == GENERAL_TOPIC_UUID {
        return false;
    }
    topic_id.is_none_or(|topic_id| Uuid::from_uuid_string(topic_id) == Some(preview_topic_uuid))
}

fn message_scope(object: &BTreeMap<String, RiseJSONValue>, route: &str) -> anyhow::Result<RiseMessageScope> {
    let Some(chat_id) = string(object, &[rise_key::message::CHAT_ID, rise_key::message::USER_CHAT_ID]) else {
        return Err(invalid_body(route));
    };
    Ok(RiseMessageScope::new(
        chat_id,
        optional_string(object.get(rise_key::bot::TOPIC_ID)),
        optional_string(object.get(rise_key::bot::THREAD_ROOT_ID)),
    ))
}

fn decode<Decoded: DeserializeOwned>(body: &RiseJSONValue, route: &str) -> anyhow::Result<Decoded> {
    body.decode_as::<Decoded>().map_err(|_| invalid_body(route))
}

fn encode<Encoded: Serialize>(value: &Encoded) -> anyhow::Result<RiseJSONValue> {
    Ok(RiseJSONValue::encode(value)?)
}

fn success_value() -> RiseJSONValue {
    RiseJSONValue::Object(BTreeMap::from([(rise_key::bot::SUCCESS.to_owned(), RiseJSONValue::Bool(true))]))
}

fn validate_success(response: &RiseJSONValue) -> anyhow::Result<()> {
    let Some(object) = response.object_value() else {
        return Ok(());
    };
    if bool_value(object, rise_key::bot::SUCCESS) == Some(false) {
        let code = string(object, &[rise_key::bot::ERROR, rise_key::auth::CODE])
            .unwrap_or_else(|| REMOTE_REJECTED_CODE.to_owned());
        return Err(RiseConversationRepositoryFacadeError::RemoteRejected(code).into());
    }
    Ok(())
}

fn chat_from(response: &RiseJSONValue) -> anyhow::Result<Option<RiseStoredChat>> {
    let Some(root) = response.object_value() else {
        return Ok(None);
    };
    let data = root.get(rise_key::bot::DATA);
    let candidate = root
        .get(rise_key::events::CHAT)
        .or_else(|| data.and_then(RiseJSONValue::object_value).and_then(|data| data.get(rise_key::events::CHAT)))
        .or(data);
    let Some(candidate) = candidate.filter(|candidate| candidate.object_value().is_some()) else {
        return Ok(None);
    };
    let chat: ChatInfo = candidate.decode_as()?;
    Ok(if chat.id.is_empty() {
        None
    } else {
        Some(RiseStoredChat::from_canon(&chat))
    })
}

fn string(object: &BTreeMap<String, RiseJSONValue>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| optional_string(object.get(*key)).filter(|value| !value.is_empty()))
}

fn optional_string(value: Option<&RiseJSONValue>) -> Option<String> {
    match value? {
        RiseJSONValue::String(raw) => Some(raw.clone()),
        RiseJSONValue::Integer(raw) => Some(raw.to_string()),
        RiseJSONValue::UnsignedInteger(raw) => Some(raw.to_string()),
        _ => None,
    }
}

fn strings(object: &BTreeMap<String, RiseJSONValue>, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(RiseJSONValue::Array(values)) = object.get(*key) {
            return values.iter().filter_map(|value| optional_string(Some(value))).collect();
        }
    }
    Vec::new()
}

fn bool_value(object: &BTreeMap<String, RiseJSONValue>, key: &str) -> Option<bool> {
    match object.get(key)? {
        RiseJSONValue::Bool(raw) => Some(*raw),
        RiseJSONValue::Integer(raw) => Some(*raw != 0),
        RiseJSONValue::UnsignedInteger(raw) => Some(*raw != 0),
        RiseJSONValue::String(raw) => match raw.to_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn int(object: &BTreeMap<String, RiseJSONValue>, key: &str) -> Option<i64> {
    match object.get(key)? {
        RiseJSONValue::Integer(raw) => Some(*raw),
        RiseJSONValue::UnsignedInteger(raw) => i64::try_from(*raw).ok(),
        RiseJSONValue::Number(raw) => {
            let is_exact = raw.fract() == 0.0 && *raw >= i64::MIN as f64 && *raw < i64::MAX as f64;
            is_exact.then_some(*raw as i64)
        }
        RiseJSONValue::String(raw) => raw.parse().ok(),
        _ => None,
    }
}

fn number(object: &BTreeMap<String, RiseJSONValue>, key: &str) -> Option<f64> {
    match object.get(key)? {
        RiseJSONValue::Number(raw) => Some(*raw),
        RiseJSONValue::Integer(raw) => Some(*raw as f64),
        RiseJSONValue::UnsignedInteger(raw) => Some(*raw as f64),
        RiseJSONValue::String(raw) => raw.parse().ok(),
        _ => None,
    }
}

#[derive(Default)]
struct PreparationFlag {
    settled: std::sync::atomic::AtomicBool,
}
