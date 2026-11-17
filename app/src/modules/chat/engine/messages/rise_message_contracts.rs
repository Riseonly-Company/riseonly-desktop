use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use futures::FutureExt;
use futures::future::{BoxFuture, ready};
use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_request::message::VotePoll;
use rise_canon::rise_response::message as response;
use rise_canon::rise_view::message as view;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::rise_core::rise_identifiers::RiseRequestID;
use rise_engine::rise_store::rise_store_models::RiseStoreWriteBatch;
use rise_engine::rise_upload::rise_upload_progress::RiseUploadCancellationHub;
use rise_engine::{RiseEngine, RiseJSONValue};
use serde::{Deserialize, Serialize};

use crate::modules::chat::engine::chats::rise_chat_repository_models::RiseChatOptimisticToken;
use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpoints, RiseConversationRPC};
use crate::modules::chat::engine::messages::rise_message_scope::{RiseMessagePageDirection, RiseMessageScope};
use crate::modules::chat::engine::messages::rise_message_sparks::{GetMessageSparksBody, SendMessageSparksBody};
use crate::modules::chat::engine::messages::rise_message_upload_retry::RiseMessageUploadRetry;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    AddReactionBody, AddReactionGatewayDTO, DeleteAllMessagesFromUserBody, DeleteAllMessagesFromUserGatewayDTO,
    DeleteMessagesBody, DeleteMessagesGatewayDTO, EditMessageBody, EditMessageGatewayDTO, ForwardMessagesBody,
    ForwardMessagesGatewayDTO, GetChatCalendarBody, GetChatCalendarGatewayDTO, GetMessagesAroundBody,
    GetMessagesAroundGatewayDTO, GetMessagesBody, GetMessagesGatewayDTO, MarkChatAsReadBody, MarkChatAsReadGatewayDTO,
    MarkMediaPlayedBody, MarkMessageViewedBody, MessageTypingBody, MessageWireDTO, PinMessageBody, RemoveReactionBody,
    RemoveReactionGatewayDTO, RiseStoredOutgoingMessage, UnpinAllMessagesBody, UnpinAllMessagesGatewayDTO,
};
use crate::modules::sparks::engine::rise_sparks_engine_models::{RiseStoredSparksSendResult, RiseStoredSparksTop};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseMessageRepositoryError {
    #[error("The message scope is invalid")]
    InvalidScope,
    #[error("The {} message cursor is unavailable", .0.raw_value())]
    MissingCursor(RiseMessagePageDirection),
    #[error("Message anchor is not cached: {0}")]
    AnchorNotCached(String),
    #[error("The around response did not contain its anchor: {0}")]
    AroundResponseMissingAnchor(String),
    #[error("Message was not found: {0}")]
    MessageNotFound(String),
    #[error("Message has no server identity yet: {0}")]
    MissingServerID(String),
    #[error("Message request failed: {0}")]
    RemoteFailure(String),
    #[error("An upload pipeline is required for local attachments")]
    UploadPipelineRequired,
    #[error("The message payload could not be decoded")]
    InvalidWirePayload,
}

pub enum RiseMessagePageContract {}

impl RiseMessagePageContract {
    pub fn require_messages(messages: Option<Vec<MessageWireDTO>>) -> anyhow::Result<Vec<MessageWireDTO>> {
        let Some(messages) = messages else {
            return Err(RiseMessageRepositoryError::InvalidWirePayload.into());
        };
        Ok(messages)
    }
}

/// futures are boxed because the repo holds this behind dyn and tests swap it
pub trait RiseMessageRemoteAPI: Send + Sync {
    fn messages(&self, body: GetMessagesBody) -> BoxFuture<'_, anyhow::Result<GetMessagesGatewayDTO>>;
    fn messages_around(&self, body: GetMessagesAroundBody) -> BoxFuture<'_, anyhow::Result<GetMessagesAroundGatewayDTO>>;
    fn message_by_id(&self, message_id: String) -> BoxFuture<'_, anyhow::Result<RiseJSONValue>>;
    fn message_by_id_surviving_suspension(&self, message_id: String) -> BoxFuture<'_, anyhow::Result<RiseJSONValue>> {
        self.message_by_id(message_id)
    }
    fn chat_calendar(&self, body: GetChatCalendarBody) -> BoxFuture<'_, anyhow::Result<GetChatCalendarGatewayDTO>>;
    fn edit(&self, body: EditMessageBody) -> BoxFuture<'_, anyhow::Result<EditMessageGatewayDTO>>;
    fn delete(&self, body: DeleteMessagesBody) -> BoxFuture<'_, anyhow::Result<DeleteMessagesGatewayDTO>>;
    fn pin(&self, body: PinMessageBody) -> BoxFuture<'_, anyhow::Result<response::PinMessage>>;
    fn unpin(&self, body: PinMessageBody) -> BoxFuture<'_, anyhow::Result<response::UnpinMessage>>;
    fn unpin_all(&self, body: UnpinAllMessagesBody) -> BoxFuture<'_, anyhow::Result<UnpinAllMessagesGatewayDTO>>;
    fn delete_all_from_user(
        &self,
        body: DeleteAllMessagesFromUserBody,
    ) -> BoxFuture<'_, anyhow::Result<DeleteAllMessagesFromUserGatewayDTO>>;
    fn forward(&self, body: ForwardMessagesBody) -> BoxFuture<'_, anyhow::Result<ForwardMessagesGatewayDTO>>;
    fn add_reaction(&self, body: AddReactionBody) -> BoxFuture<'_, anyhow::Result<AddReactionGatewayDTO>>;
    fn remove_reaction(&self, body: RemoveReactionBody) -> BoxFuture<'_, anyhow::Result<RemoveReactionGatewayDTO>>;
    fn send_sparks(&self, body: SendMessageSparksBody) -> BoxFuture<'_, anyhow::Result<RiseStoredSparksSendResult>>;
    fn sparks_top(&self, body: GetMessageSparksBody) -> BoxFuture<'_, anyhow::Result<RiseStoredSparksTop>>;
    fn mark_read(&self, body: MarkChatAsReadBody) -> BoxFuture<'_, anyhow::Result<MarkChatAsReadGatewayDTO>>;
    fn mark_media_played(&self, body: MarkMediaPlayedBody) -> BoxFuture<'_, anyhow::Result<response::MarkMediaAsPlayed>>;
    fn vote_poll(&self, body: VotePoll) -> BoxFuture<'_, anyhow::Result<response::VotePoll>>;
    fn mark_viewed(&self, body: MarkMessageViewedBody) -> BoxFuture<'_, anyhow::Result<response::MarkMessageAsViewed>>;
    fn send_typing(&self, body: MessageTypingBody) -> BoxFuture<'_, anyhow::Result<response::MessageTyping>>;
}

pub struct RiseMessageEngineRemoteAPI {
    rpc: RiseConversationRPC,
}

impl RiseMessageEngineRemoteAPI {
    pub fn new(engine: Arc<RiseEngine>) -> Self {
        Self { rpc: RiseConversationRPC::new(engine) }
    }
}

impl RiseMessageRemoteAPI for RiseMessageEngineRemoteAPI {
    fn messages(&self, body: GetMessagesBody) -> BoxFuture<'_, anyhow::Result<GetMessagesGatewayDTO>> {
        async move {
            let page: view::Page = self.rpc.call(RiseConversationEndpoints::get_messages(), body).await?;
            GetMessagesGatewayDTO::from_canon(&page)
        }
        .boxed()
    }

    fn messages_around(&self, body: GetMessagesAroundBody) -> BoxFuture<'_, anyhow::Result<GetMessagesAroundGatewayDTO>> {
        async move {
            let page: view::Around = self.rpc.call(RiseConversationEndpoints::get_messages_around(), body).await?;
            GetMessagesAroundGatewayDTO::from_canon(&page)
        }
        .boxed()
    }

    fn message_by_id(&self, message_id: String) -> BoxFuture<'_, anyhow::Result<RiseJSONValue>> {
        async move {
            let body = RiseJSONValue::Object(BTreeMap::from([(
                bot_key::MESSAGE_ID.to_string(),
                RiseJSONValue::String(message_id),
            )]));
            self.rpc.call_body(RiseConversationEndpoints::get_message_by_id(), &body, None).await
        }
        .boxed()
    }

    fn message_by_id_surviving_suspension(&self, message_id: String) -> BoxFuture<'_, anyhow::Result<RiseJSONValue>> {
        async move {
            let body = RiseJSONValue::Object(BTreeMap::from([(
                bot_key::MESSAGE_ID.to_string(),
                RiseJSONValue::String(message_id),
            )]));
            let endpoint = RiseConversationEndpoints::get_message_by_id().with_survives_suspension(true);
            self.rpc.call_body(endpoint, &body, None).await
        }
        .boxed()
    }

    fn chat_calendar(&self, body: GetChatCalendarBody) -> BoxFuture<'_, anyhow::Result<GetChatCalendarGatewayDTO>> {
        self.rpc.call(RiseConversationEndpoints::get_chat_calendar(), body).boxed()
    }

    fn edit(&self, body: EditMessageBody) -> BoxFuture<'_, anyhow::Result<EditMessageGatewayDTO>> {
        self.rpc.call(RiseConversationEndpoints::edit_message(), body).boxed()
    }

    fn delete(&self, body: DeleteMessagesBody) -> BoxFuture<'_, anyhow::Result<DeleteMessagesGatewayDTO>> {
        self.rpc.call(RiseConversationEndpoints::delete_messages(), body).boxed()
    }

    fn pin(&self, body: PinMessageBody) -> BoxFuture<'_, anyhow::Result<response::PinMessage>> {
        self.rpc.call(RiseConversationEndpoints::pin_message(), body).boxed()
    }

    fn unpin(&self, body: PinMessageBody) -> BoxFuture<'_, anyhow::Result<response::UnpinMessage>> {
        self.rpc.call(RiseConversationEndpoints::unpin_message(), body).boxed()
    }

    fn unpin_all(&self, body: UnpinAllMessagesBody) -> BoxFuture<'_, anyhow::Result<UnpinAllMessagesGatewayDTO>> {
        self.rpc.call(RiseConversationEndpoints::unpin_all_messages(), body).boxed()
    }

    fn delete_all_from_user(
        &self,
        body: DeleteAllMessagesFromUserBody,
    ) -> BoxFuture<'_, anyhow::Result<DeleteAllMessagesFromUserGatewayDTO>> {
        self.rpc.call(RiseConversationEndpoints::delete_all_messages_from_user(), body).boxed()
    }

    fn forward(&self, body: ForwardMessagesBody) -> BoxFuture<'_, anyhow::Result<ForwardMessagesGatewayDTO>> {
        async move {
            let forwarded: view::Forwarded = self.rpc.call(RiseConversationEndpoints::forward_messages(), body).await?;
            Ok(ForwardMessagesGatewayDTO::from_canon(&forwarded))
        }
        .boxed()
    }

    fn add_reaction(&self, body: AddReactionBody) -> BoxFuture<'_, anyhow::Result<AddReactionGatewayDTO>> {
        self.rpc.call(RiseConversationEndpoints::add_reaction(), body).boxed()
    }

    fn remove_reaction(&self, body: RemoveReactionBody) -> BoxFuture<'_, anyhow::Result<RemoveReactionGatewayDTO>> {
        self.rpc.call(RiseConversationEndpoints::remove_reaction(), body).boxed()
    }

    fn send_sparks(&self, body: SendMessageSparksBody) -> BoxFuture<'_, anyhow::Result<RiseStoredSparksSendResult>> {
        async move {
            let result: response::SendMessageSparks =
                self.rpc.attested_call(RiseConversationEndpoints::send_message_sparks(), body).await?;
            Ok(RiseStoredSparksSendResult::from_message_canon(&result))
        }
        .boxed()
    }

    fn sparks_top(&self, body: GetMessageSparksBody) -> BoxFuture<'_, anyhow::Result<RiseStoredSparksTop>> {
        async move {
            let top: response::GetMessageSparks =
                self.rpc.call(RiseConversationEndpoints::get_message_sparks(), body).await?;
            Ok(RiseStoredSparksTop::from_message_canon(&top))
        }
        .boxed()
    }

    fn mark_read(&self, body: MarkChatAsReadBody) -> BoxFuture<'_, anyhow::Result<MarkChatAsReadGatewayDTO>> {
        self.rpc.call(RiseConversationEndpoints::mark_chat_as_read(), body).boxed()
    }

    fn mark_media_played(&self, body: MarkMediaPlayedBody) -> BoxFuture<'_, anyhow::Result<response::MarkMediaAsPlayed>> {
        self.rpc.call(RiseConversationEndpoints::mark_media_played(), body).boxed()
    }

    fn vote_poll(&self, body: VotePoll) -> BoxFuture<'_, anyhow::Result<response::VotePoll>> {
        self.rpc.call(RiseConversationEndpoints::vote_poll(), body).boxed()
    }

    fn mark_viewed(&self, body: MarkMessageViewedBody) -> BoxFuture<'_, anyhow::Result<response::MarkMessageAsViewed>> {
        self.rpc.call(RiseConversationEndpoints::mark_message_viewed(), body).boxed()
    }

    fn send_typing(&self, body: MessageTypingBody) -> BoxFuture<'_, anyhow::Result<response::MessageTyping>> {
        self.rpc.call(RiseConversationEndpoints::typing(), body).boxed()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseMessageLocalUpload {
    pub identifier: String,
    #[serde(rename = "fileURL")]
    pub file_url: Url,
    pub media_type: String,
}

impl RiseMessageLocalUpload {
    pub fn new(identifier: String, file_url: Url, media_type: String) -> Self {
        assert!(!identifier.is_empty());
        Self { identifier, file_url, media_type }
    }
}

/// asked once the attachments are ready and before the message goes out
/// the answer is a wire code
pub type RiseMessagePublishRefusal = Arc<dyn Fn() -> BoxFuture<'static, Option<String>> + Send + Sync>;

#[derive(Clone)]
pub struct RiseMessageDraft {
    pub body: RiseStoredOutgoingMessage,
    pub optimistic_wire: Option<MessageWireDTO>,
    pub local_uploads: Vec<RiseMessageLocalUpload>,
    pub upload_pipeline: Option<Arc<dyn RiseMessageUploadPipeline>>,
    pub publish_refusal: Option<RiseMessagePublishRefusal>,
}

impl RiseMessageDraft {
    pub fn new(body: RiseStoredOutgoingMessage) -> Self {
        Self { body, optimistic_wire: None, local_uploads: Vec::new(), upload_pipeline: None, publish_refusal: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RiseMessageUploadEvent {
    Preparing { index: i64, fraction: f64, total_bytes: i64 },
    Uploading { index: i64, fraction: f64, completed_bytes: i64, total_bytes: i64 },
    Processing { index: i64, fraction: f64, total_bytes: i64 },
    Completed { index: i64, total_bytes: i64 },
}

pub type RiseMessageUploadEvents = Arc<dyn Fn(RiseMessageUploadEvent) -> BoxFuture<'static, ()> + Send + Sync>;

pub trait RiseMessageUploadPipeline: Send + Sync {
    fn prepare(
        &self,
        draft: RiseMessageDraft,
        local_id: String,
        cancellation: Arc<RiseUploadCancellationHub>,
        events: RiseMessageUploadEvents,
    ) -> BoxFuture<'static, anyhow::Result<RiseStoredOutgoingMessage>>;

    /// serialized sources so a failed send can be sent again, none when they cant outlive the process
    fn retry(&self, _body: &RiseStoredOutgoingMessage, _scope: &RiseMessageScope) -> Option<RiseMessageUploadRetry> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMessageSubmission {
    pub request_id: RiseRequestID,
    pub local_id: String,
    pub entity_version: i64,
    pub store_revision: i64,
}

#[derive(Clone, Debug)]
pub struct RiseAuthoritativeMessage {
    pub wire: MessageWireDTO,
    pub scope: RiseMessageScope,
    pub local_id: Option<String>,
}

impl RiseAuthoritativeMessage {
    pub fn new(wire: MessageWireDTO, scope: RiseMessageScope, local_id: Option<String>) -> Self {
        Self { wire, scope, local_id }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseMessageSubmissionMode {
    Durable,
    Immediate,
    Undeliverable,
    AwaitingConversation,
}

#[derive(Clone, Debug)]
pub enum RiseMessagePreviewChange {
    Created { scope: RiseMessageScope, message: MessageWireDTO, was_already_stored: bool },
    Edited { scope: RiseMessageScope, message: MessageWireDTO },
    Deleted { chat_id: String, server_ids: Vec<String>, fallback: Option<MessageWireDTO> },
    HistoryCleared { chat_id: String },
    Read { chat_id: String, topic_id: Option<String>, authoritative_unread: Option<i64>, clears_marks: bool },
    Receipt { chat_id: String, server_ids: Vec<String> },
    SendFailed { chat_id: String, local_id: String },
}

#[derive(Clone, Debug)]
pub struct RiseMessageHistoryClearReceipt {
    pub scopes: HashSet<RiseMessageScope>,
    pub store_revision: i64,
}

type RiseMessageChatPreviewBatchOperation =
    Arc<dyn Fn(RiseMessagePreviewChange, i64) -> BoxFuture<'static, anyhow::Result<RiseStoreWriteBatch>> + Send + Sync>;

#[derive(Clone)]
pub struct RiseMessageChatPreviewBatchHook {
    operation: RiseMessageChatPreviewBatchOperation,
}

impl RiseMessageChatPreviewBatchHook {
    pub fn empty() -> Self {
        Self::new(|_, _| ready(Ok(RiseStoreWriteBatch::empty())).boxed())
    }

    pub fn new<Operation>(operation: Operation) -> Self
    where
        Operation:
            Fn(RiseMessagePreviewChange, i64) -> BoxFuture<'static, anyhow::Result<RiseStoreWriteBatch>> + Send + Sync + 'static,
    {
        Self { operation: Arc::new(operation) }
    }

    pub async fn call(&self, change: RiseMessagePreviewChange, version: i64) -> anyhow::Result<RiseStoreWriteBatch> {
        (self.operation)(change, version).await
    }
}

#[derive(Clone, Debug)]
pub struct RiseMessageChatReadOptimisticHookPrepared {
    pub token: RiseChatOptimisticToken,
    pub batch: RiseStoreWriteBatch,
}

type RiseMessageChatReadPrepareOperation = Arc<
    dyn Fn(String, Option<i64>) -> BoxFuture<'static, anyhow::Result<Option<RiseMessageChatReadOptimisticHookPrepared>>>
        + Send
        + Sync,
>;
type RiseMessageChatReadTokenOperation = Arc<dyn Fn(RiseChatOptimisticToken) -> BoxFuture<'static, ()> + Send + Sync>;

/// puts the chat side optimistic unread clear into the message reducer transaction
/// token stays opaque since only the chat repo can reconcile it
#[derive(Clone)]
pub struct RiseMessageChatReadOptimisticHook {
    prepare_operation: RiseMessageChatReadPrepareOperation,
    confirm_operation: RiseMessageChatReadTokenOperation,
    rollback_operation: RiseMessageChatReadTokenOperation,
    discard_operation: RiseMessageChatReadTokenOperation,
}

impl RiseMessageChatReadOptimisticHook {
    pub fn empty() -> Self {
        Self::new(
            |_, _| ready(Ok(None)).boxed(),
            |_| ready(()).boxed(),
            |_| ready(()).boxed(),
            |_| ready(()).boxed(),
        )
    }

    pub fn new<Prepare, Confirm, Rollback, Discard>(
        prepare: Prepare,
        confirm: Confirm,
        rollback: Rollback,
        discard: Discard,
    ) -> Self
    where
        Prepare: Fn(String, Option<i64>) -> BoxFuture<'static, anyhow::Result<Option<RiseMessageChatReadOptimisticHookPrepared>>>
            + Send
            + Sync
            + 'static,
        Confirm: Fn(RiseChatOptimisticToken) -> BoxFuture<'static, ()> + Send + Sync + 'static,
        Rollback: Fn(RiseChatOptimisticToken) -> BoxFuture<'static, ()> + Send + Sync + 'static,
        Discard: Fn(RiseChatOptimisticToken) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        Self {
            prepare_operation: Arc::new(prepare),
            confirm_operation: Arc::new(confirm),
            rollback_operation: Arc::new(rollback),
            discard_operation: Arc::new(discard),
        }
    }

    /// clearing_topic_unread none means the read covers the whole chat and the aggregate goes to zero
    /// a value means only that much is local and the ack supplies the rest
    pub async fn prepare(
        &self,
        chat_id: String,
        clearing_topic_unread: Option<i64>,
    ) -> anyhow::Result<Option<RiseMessageChatReadOptimisticHookPrepared>> {
        (self.prepare_operation)(chat_id, clearing_topic_unread).await
    }

    pub async fn confirm(&self, token: RiseChatOptimisticToken) {
        (self.confirm_operation)(token).await;
    }

    pub async fn rollback(&self, token: RiseChatOptimisticToken) {
        (self.rollback_operation)(token).await;
    }

    pub async fn discard(&self, token: RiseChatOptimisticToken) {
        (self.discard_operation)(token).await;
    }
}

/// bounded secondary indexes derived from the authoritative message stream
/// invalidated by kind and not by clearing the whole auxiliary cache
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMessageAuxiliaryProjection {
    Pinned,
    Media,
    Files,
    Music,
    Voice,
    Gif,
    Favorites,
    Links,
}

impl RiseMessageAuxiliaryProjection {
    pub const ALL_CASES: [Self; 8] =
        [Self::Pinned, Self::Media, Self::Files, Self::Music, Self::Voice, Self::Gif, Self::Favorites, Self::Links];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Pinned => "pinned",
            Self::Media => "media",
            Self::Files => "files",
            Self::Music => "music",
            Self::Voice => "voice",
            Self::Gif => "gif",
            Self::Favorites => "favorites",
            Self::Links => "links",
        }
    }

    pub fn content() -> HashSet<Self> {
        HashSet::from([Self::Media, Self::Files, Self::Music, Self::Voice, Self::Gif, Self::Links])
    }

    pub fn pin_state() -> HashSet<Self> {
        HashSet::from([Self::Pinned, Self::Favorites])
    }

    pub fn all() -> HashSet<Self> {
        Self::ALL_CASES.into_iter().collect()
    }
}

fn normalized_identifier(value: &str) -> Option<String> {
    let value = trimming_whitespaces_and_newlines(value);
    (!value.is_empty()).then(|| value.to_string())
}

#[derive(Clone, Debug)]
pub struct RiseMessageAuxiliaryInvalidationPinnedMessagePatch {
    pub message: Option<MessageWireDTO>,
    pub message_identifiers: HashSet<String>,
    pub is_pinned: bool,
    pub removes_all: bool,
}

impl RiseMessageAuxiliaryInvalidationPinnedMessagePatch {
    pub fn from_message(message: MessageWireDTO, is_pinned: bool) -> Self {
        let message_identifiers = [&message.server_id, &message.id, &message.temp_id]
            .into_iter()
            .filter_map(|value| value.as_deref().and_then(normalized_identifier))
            .collect();
        Self { message: Some(message), message_identifiers, is_pinned, removes_all: false }
    }

    pub fn from_message_identifier(message_identifier: &str, is_pinned: bool) -> Self {
        Self {
            message: None,
            message_identifiers: normalized_identifier(message_identifier).into_iter().collect(),
            is_pinned,
            removes_all: false,
        }
    }

    pub fn clear_all_pinned(clear_all_pinned: bool) -> Self {
        Self { message: None, message_identifiers: HashSet::new(), is_pinned: false, removes_all: clear_all_pinned }
    }
}

impl PartialEq for RiseMessageAuxiliaryInvalidationPinnedMessagePatch {
    fn eq(&self, other: &Self) -> bool {
        self.is_pinned == other.is_pinned
            && self.removes_all == other.removes_all
            && self.message_identifiers == other.message_identifiers
    }
}

impl Eq for RiseMessageAuxiliaryInvalidationPinnedMessagePatch {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMessageAuxiliaryInvalidation {
    pub chat_id: String,
    pub topic_ids: HashSet<String>,
    pub projections: HashSet<RiseMessageAuxiliaryProjection>,
    pub pinned_message_patch: Option<RiseMessageAuxiliaryInvalidationPinnedMessagePatch>,
    pub removed_message_ids: HashSet<String>,
}

impl RiseMessageAuxiliaryInvalidation {
    pub fn new(chat_id: &str, topic_id: Option<&str>, projections: HashSet<RiseMessageAuxiliaryProjection>) -> Self {
        Self {
            chat_id: trimming_whitespaces_and_newlines(chat_id).to_string(),
            topic_ids: topic_id.and_then(normalized_identifier).into_iter().collect(),
            projections,
            pinned_message_patch: None,
            removed_message_ids: HashSet::new(),
        }
    }

    pub fn with_topic_ids(
        chat_id: &str,
        topic_ids: HashSet<String>,
        projections: HashSet<RiseMessageAuxiliaryProjection>,
    ) -> Self {
        Self {
            chat_id: trimming_whitespaces_and_newlines(chat_id).to_string(),
            topic_ids: topic_ids.iter().filter_map(|value| normalized_identifier(value)).collect(),
            projections,
            pinned_message_patch: None,
            removed_message_ids: HashSet::new(),
        }
    }

    pub fn with_scope(scope: &RiseMessageScope, projections: HashSet<RiseMessageAuxiliaryProjection>) -> Self {
        Self::new(&scope.chat_id, scope.topic_id.as_deref(), projections)
    }

    pub fn with_pinned_message_patch(mut self, patch: RiseMessageAuxiliaryInvalidationPinnedMessagePatch) -> Self {
        self.pinned_message_patch = Some(patch);
        self
    }
}

/// builds store mutations that commit with a normalized message batch and drops process local copies
#[derive(Clone)]
pub struct RiseMessageAuxiliaryInvalidationBatchHook {
    operation: Arc<dyn Fn(RiseMessageAuxiliaryInvalidation) -> BoxFuture<'static, RiseStoreWriteBatch> + Send + Sync>,
}

impl RiseMessageAuxiliaryInvalidationBatchHook {
    pub fn empty() -> Self {
        Self::new(|_| ready(RiseStoreWriteBatch::empty()).boxed())
    }

    pub fn new<Operation>(operation: Operation) -> Self
    where
        Operation: Fn(RiseMessageAuxiliaryInvalidation) -> BoxFuture<'static, RiseStoreWriteBatch> + Send + Sync + 'static,
    {
        Self { operation: Arc::new(operation) }
    }

    pub async fn call(&self, invalidation: RiseMessageAuxiliaryInvalidation) -> RiseStoreWriteBatch {
        (self.operation)(invalidation).await
    }
}

/// a remote read event has no server read time so arrival time is stamped locally
/// lets the receipt menu open on a cached value, the real refresh replaces it
#[derive(Clone, Debug)]
pub struct RiseMessageReadReceiptHint {
    pub chat_id: String,
    pub message_ids: Vec<String>,
    pub read_at_milliseconds: i64,
}

#[derive(Clone)]
pub struct RiseMessageReadReceiptHintHook {
    operation: Arc<dyn Fn(RiseMessageReadReceiptHint) -> BoxFuture<'static, ()> + Send + Sync>,
}

impl RiseMessageReadReceiptHintHook {
    pub fn empty() -> Self {
        Self::new(|_| ready(()).boxed())
    }

    pub fn new<Operation>(operation: Operation) -> Self
    where
        Operation: Fn(RiseMessageReadReceiptHint) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        Self { operation: Arc::new(operation) }
    }

    pub async fn call(&self, hint: RiseMessageReadReceiptHint) {
        (self.operation)(hint).await;
    }
}
