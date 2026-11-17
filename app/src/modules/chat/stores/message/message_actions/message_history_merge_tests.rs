use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui::{App, Entity, TestAppContext};
use rise_canon::RiseWireKey;
use rise_canon::rise_key::message as key;
use rise_canon::rise_wire::{KEY_CODE, KEY_DATA, KEY_ERROR, KEY_REQUEST_ID, KEY_STATUS, STATUS_ERROR};
use rise_canon::rise_wire_canon::key as wire_key;
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_core::rise_identifiers::RiseRequestID;
use rise_engine::rise_core::rise_secure_key_store::{RiseEngineKeyStore, RiseEngineKeyStoreFuture};
use rise_engine::rise_store::rise_store_entities::{RiseEntityWritePolicy, RiseStoreEntity};
use rise_engine::rise_store::rise_store_models::{RiseStoreMutation, RiseStoreWriteBatch};
use rise_engine::rise_upload::rise_upload_progress::RiseUploadRealtimeProgressHub;
use rise_engine::rise_wire::rise_wire_contracts::RiseWireRemoteError;
use rise_engine::{
    RiseEngine, RiseEngineConfiguration, RiseEngineConfigurationArgs, RiseJSONValue, RiseWireAccountContext,
    RiseWireFrameSender, RiseWireInboundDisposition, RiseWireSocketSession,
};
use rise_platform::file_manager::FileManager;
use serde_json::{Map, Value};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Mutex, MutexGuard};

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::SaiHttpTraceStore;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseConversationEntitySource, RiseMessageEntityRecord, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationPresentation, RiseMessageWireSnapshot,
};
use crate::modules::chat::engine::core::rise_conversation_store_layout::RiseConversationStoreLayout;
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseAuthoritativeMessage, RiseMessageAuxiliaryInvalidationBatchHook, RiseMessageChatPreviewBatchHook,
    RiseMessageChatReadOptimisticHook, RiseMessageDraft, RiseMessageReadReceiptHintHook, RiseMessageSubmissionMode,
};
use crate::modules::chat::engine::messages::rise_message_durable_outbox::RiseCreateMessageDurableEndpoint;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::engine::messages::rise_message_scope::{
    RiseMessageAnchor, RiseMessagePageLoadOutcome, RiseMessageScope, RiseMessageWindowConfiguration,
};
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomain;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageHistoryDirection, MessageHistoryPageState, MessageSystemKind, MessageThreadOrder,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    GetMessagesAroundGatewayDTO, GetMessagesGatewayDTO, MessagePayloadCodec, MessageWireDTO,
    RiseStoredOutgoingMessage,
};
use crate::modules::chat::stores::message::message_services::message_presentation_window_projector::MessagePresentationWindowProjector;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

const HISTORY_PAGINATION_KEY: &str = "history_pagination";
const SCOPE_STORAGE_KEY_FIELD: &str = "scopeStorageKey";

static SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

struct MessageHistoryRiseKeyStore;

impl RiseEngineKeyStore for MessageHistoryRiseKeyStore {
    fn key<'a>(&'a self, _account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(Some(vec![0x51; 32])) })
    }

    fn create_key_if_absent<'a>(&'a self, _account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Vec<u8>> {
        Box::pin(async move { Ok(vec![0x51; 32]) })
    }

    fn delete_key<'a>(&'a self, _account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, ()> {
        Box::pin(async move { Ok(()) })
    }
}

#[derive(Debug, thiserror::Error)]
enum MessageHistoryRiseTestError {
    #[error("invalid request")]
    InvalidRequest,
}

struct MessageHistoryRiseSender {
    outgoing: UnboundedSender<(Vec<u8>, RiseWireSocketSession)>,
    incoming: Mutex<UnboundedReceiver<(Vec<u8>, RiseWireSocketSession)>>,
}

impl MessageHistoryRiseSender {
    fn new() -> Arc<Self> {
        let (outgoing, incoming) = unbounded_channel();
        Arc::new(Self { outgoing, incoming: Mutex::new(incoming) })
    }

    async fn next(&self) -> (Vec<u8>, RiseWireSocketSession) {
        self.incoming.lock().await.recv().await.expect("the sender owns its own channel")
    }
}

impl RiseWireFrameSender for MessageHistoryRiseSender {
    fn send(&self, frame: Vec<u8>, session: RiseWireSocketSession) -> BoxFuture<'static, anyhow::Result<()>> {
        let sent = self.outgoing.send((frame, session)).map_err(|_| anyhow::anyhow!("sender channel closed"));
        async move { sent }.boxed()
    }
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = FileManager::default().remove_item(&self.0);
    }
}

struct RestoreAccountScope(Option<String>);

impl Drop for RestoreAccountScope {
    fn drop(&mut self) {
        match self.0.take() {
            Some(user_id) => SaiAccountCacheScope::activate(&user_id),
            None => SaiAccountCacheScope::clear_active_account(),
        }
    }
}

fn unique_token() -> String {
    Uuid::new_v4().uuid_string()
}

fn temporary_root(label: &str) -> RemoveOnDrop {
    RemoveOnDrop(std::env::temp_dir().join(format!("{label}-{}", unique_token())))
}

fn object(entries: Vec<(&str, Value)>) -> Map<String, Value> {
    entries.into_iter().map(|(entry_key, value)| (entry_key.to_owned(), value)).collect()
}

fn row(id: &str, created_at: f64) -> Map<String, Value> {
    object(vec![(key::ID, Value::from(id)), (key::CREATED_AT, Value::from(created_at))])
}

fn content_row(id: &str, content: &str, created_at: f64) -> Map<String, Value> {
    object(vec![
        (key::ID, Value::from(id)),
        (key::CONTENT, Value::from(content)),
        (key::CREATED_AT, Value::from(created_at)),
    ])
}

fn temp_row(id: &str, content: Option<&str>, created_at: f64) -> Map<String, Value> {
    let mut temp = row(id, created_at);
    if let Some(content) = content {
        temp.insert(key::CONTENT.to_owned(), Value::from(content));
    }
    temp.insert(RiseWireKey::camel(RiseLocalKey::IS_TEMP), Value::Bool(true));
    temp
}

fn message_values(rows: Vec<Map<String, Value>>) -> Value {
    Value::Array(rows.into_iter().map(Value::Object).collect())
}

fn page_root(
    chat_id: &str,
    relative_id: Option<&str>,
    is_have_more: Option<bool>,
    rows: Vec<Map<String, Value>>,
) -> Map<String, Value> {
    let mut root = object(vec![(key::CHAT_ID, Value::from(chat_id))]);
    if let Some(relative_id) = relative_id {
        root.insert(key::RELATIVE_ID.to_owned(), Value::from(relative_id));
    }
    if let Some(is_have_more) = is_have_more {
        root.insert(key::IS_HAVE_MORE.to_owned(), Value::Bool(is_have_more));
    }
    root.insert(key::MESSAGES.to_owned(), message_values(rows));
    root
}

fn gateway_page(
    chat_id: &str,
    relative_id: Option<&str>,
    is_have_more: bool,
    rows: Vec<Map<String, Value>>,
) -> Value {
    let page = Value::Object(page_root(chat_id, relative_id, Some(is_have_more), rows));
    assert!(MessagePayloadCodec::decode::<GetMessagesGatewayDTO>(Some(&page)).is_some());
    page
}

fn history(lanes: &[(MessageHistoryDirection, &str, bool)]) -> HashMap<MessageHistoryDirection, MessageHistoryPageState> {
    lanes
        .iter()
        .map(|(direction, relative_id, has_more)| (*direction, page_state(relative_id, *has_more)))
        .collect()
}

fn page_state(relative_id: &str, has_more: bool) -> MessageHistoryPageState {
    MessageHistoryPageState { relative_id: Some(relative_id.to_owned()), has_more }
}

fn string_of<'a>(row: &'a Map<String, Value>, row_key: &str) -> Option<&'a str> {
    row.get(row_key).and_then(Value::as_str)
}

fn canonical_root(cache_key: &str, cx: &mut App) -> Map<String, Value> {
    let payload = SaiWsResponseCache::current_payload(
        cache_key,
        SaiWsCachePreferredLayer::MemoryFirst,
        SaiWsDiskCachePolicy::PreserveExisting,
        cx,
    );
    MessagePayloadCodec::dictionary(payload.as_ref()).expect("the cache holds a root")
}

fn memory_root(cache_key: &str, cx: &mut App) -> Map<String, Value> {
    let payload = SaiWsResponseCache::memory_only(cache_key, cx);
    MessagePayloadCodec::dictionary(payload.as_ref()).expect("the memory layer holds a root")
}

fn rows_of(root: &Map<String, Value>) -> Vec<Map<String, Value>> {
    MessagePayloadCodec::array_of_dictionaries(root.get(key::MESSAGES))
}

fn ids_of(root: &Map<String, Value>) -> Vec<String> {
    rows_of(root).iter().filter_map(|row| string_of(row, key::ID).map(str::to_owned)).collect()
}

fn lane_state(
    service: &Entity<MessageServicesStore>,
    chat_id: &str,
    direction: MessageHistoryDirection,
    cx: &mut App,
) -> MessageHistoryPageState {
    service.update(cx, |service, cx| service.message_history_page_state(chat_id, direction, cx))
}

fn patch_root(
    service: &Entity<MessageServicesStore>,
    chat_id: &str,
    relative_id: Option<&str>,
    is_have_more: Option<bool>,
    lanes: &[(MessageHistoryDirection, &str, bool)],
    rows: Vec<Map<String, Value>>,
    cx: &mut App,
) {
    service.update(cx, |service, cx| {
        service.patch_messages_root_with_history(chat_id, relative_id, is_have_more, history(lanes), rows, cx);
    });
}

fn replace_latest(cache_key: &str, root: Map<String, Value>, write_to: SaiUpdaterWriteScope, cx: &mut App) {
    SaiUpdater::replace_root(cache_key, root, write_to, cx).expect("the root replaces");
}

fn flush_and_wait(chat_id: &str, cx: &mut TestAppContext) {
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        service.update(cx, |service, cx| service.flush_messages_disk_persistence(chat_id, cx));
    });
    cx.run_until_parked();
    SaiWsDiskCache::wait_for_asynchronous_writes();
    cx.run_until_parked();
}

fn clean_message_caches(chat_id: &str, cx: &mut TestAppContext) {
    flush_and_wait(chat_id, cx);
    cx.update(|cx| {
        SaiWsResponseCache::remove(
            &MessageServicesStore::messages_cache_key(chat_id),
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
        SaiWsResponseCache::remove(
            &MessageServicesStore::latest_messages_cache_key(chat_id),
            SaiCacheCleanLayer::Everywhere,
            cx,
        );
    });
}

fn request_id_of(request: &[u8]) -> anyhow::Result<RiseRequestID> {
    let root = RiseJSONValue::decode(request)?;
    let raw_value = match root.subscript_key(wire_key::ID) {
        Some(RiseJSONValue::Integer(value)) if *value > 0 => *value as u64,
        Some(RiseJSONValue::UnsignedInteger(value)) if *value > 0 => *value,
        _ => return Err(MessageHistoryRiseTestError::InvalidRequest.into()),
    };
    RiseRequestID::new(raw_value).ok_or_else(|| MessageHistoryRiseTestError::InvalidRequest.into())
}

fn rise_response(request: &[u8], payload: Map<String, Value>) -> anyhow::Result<Vec<u8>> {
    let request_id = request_id_of(request)?;
    Ok(serde_json::to_vec(&Value::Object(object(vec![
        (KEY_REQUEST_ID, Value::from(request_id.raw_value().to_string())),
        (KEY_DATA, Value::Object(payload)),
    ])))?)
}

fn rise_error_response(request: &[u8], code: &str) -> anyhow::Result<Vec<u8>> {
    let request_id = request_id_of(request)?;
    Ok(serde_json::to_vec(&Value::Object(object(vec![
        (KEY_REQUEST_ID, Value::from(request_id.raw_value().to_string())),
        (KEY_STATUS, Value::from(STATUS_ERROR)),
        (KEY_ERROR, Value::from("saved anchor unavailable")),
        (KEY_CODE, Value::from(code)),
    ])))?)
}

fn create_acknowledgement(chat_id: &str, message_id: &str, temp_id: &str) -> Map<String, Value> {
    object(vec![
        (key::SUCCESS, Value::Bool(true)),
        (key::MESSAGE_ID, Value::from(message_id)),
        (key::CHAT_ID, Value::from(chat_id)),
        (key::TEMP_ID, Value::from(temp_id)),
        (key::CREATED_AT, Value::from(3.0)),
    ])
}

fn method_of(frame: &[u8]) -> anyhow::Result<RiseJSONValue> {
    let root = RiseJSONValue::decode(frame)?;
    Ok(root.subscript_key(wire_key::METHOD).cloned().unwrap_or(RiseJSONValue::Null))
}

fn method_value(method: &str) -> RiseJSONValue {
    RiseJSONValue::String(method.to_owned())
}

fn outgoing_message(chat_id: &str, topic_id: Option<&str>, content: &str, temp_id: &str) -> RiseStoredOutgoingMessage {
    RiseStoredOutgoingMessage {
        chat_id: Some(chat_id.to_owned()),
        topic_id: topic_id.map(str::to_owned),
        content: content.to_owned(),
        original_content: content.to_owned(),
        content_type: "text".to_owned(),
        temp_id: Some(temp_id.to_owned()),
        ..RiseStoredOutgoingMessage::default()
    }
}

fn wire_ids(snapshot: &RiseMessageWireSnapshot) -> Vec<String> {
    snapshot.rows.iter().filter_map(|row| row.wire.id.clone()).collect()
}

async fn open_engine(
    root: PathBuf,
    account_id: &str,
    epoch: &str,
    sender: Arc<MessageHistoryRiseSender>,
) -> anyhow::Result<Arc<RiseEngine>> {
    RiseEngine::open(RiseEngineConfiguration::new(
        root,
        RiseWireAccountContext::new(account_id.to_owned(), epoch.to_owned()),
        sender,
        RiseEngineConfigurationArgs {
            key_store: Arc::new(MessageHistoryRiseKeyStore),
            ..RiseEngineConfigurationArgs::default()
        },
    ))
    .await
}

async fn start_session(engine: &RiseEngine, transport_epoch: u64) -> anyhow::Result<RiseWireSocketSession> {
    let session = RiseWireSocketSession::new(engine.account.clone(), transport_epoch);
    engine.socket_session_started(session.clone()).await?;
    engine.socket_connected(session.clone()).await?;
    Ok(session)
}

fn repository_for(engine: &Arc<RiseEngine>, account_id: &str) -> Arc<RiseMessageRepository> {
    RiseMessageRepository::new(
        engine.clone(),
        account_id,
        Arc::new(RiseUploadRealtimeProgressHub::default()),
        RiseMessageChatPreviewBatchHook::empty(),
        RiseMessageChatReadOptimisticHook::empty(),
        RiseMessageAuxiliaryInvalidationBatchHook::empty(),
        RiseMessageReadReceiptHintHook::empty(),
    )
}

async fn finish_engine(engine: &RiseEngine, result: anyhow::Result<()>) -> anyhow::Result<()> {
    match result {
        Ok(()) => engine.close().await,
        Err(error) => {
            let _ = engine.close().await;
            Err(error)
        }
    }
}

struct MessageHistoryHarness {
    _serial: MutexGuard<'static, ()>,
    _root: RemoveOnDrop,
    sender: Arc<MessageHistoryRiseSender>,
    engine: Arc<RiseEngine>,
    session: RiseWireSocketSession,
    repository: Arc<RiseMessageRepository>,
}

impl MessageHistoryHarness {
    async fn open(label: &str) -> anyhow::Result<Self> {
        let serial = SERIAL.lock().await;
        let root = temporary_root(&format!("rise-{label}"));
        let sender = MessageHistoryRiseSender::new();
        let account_id = format!("{label}-{}", unique_token());
        let engine = open_engine(root.0.clone(), &account_id, "test", sender.clone()).await?;
        let session = match start_session(&engine, 1).await {
            Ok(session) => session,
            Err(error) => {
                let _ = engine.close().await;
                return Err(error);
            }
        };
        let repository = repository_for(&engine, &account_id);
        Ok(Self { _serial: serial, _root: root, sender, engine, session, repository })
    }

    async fn answer(&self, frame: &[u8], payload: Map<String, Value>) -> anyhow::Result<()> {
        let response = rise_response(frame, payload)?;
        let disposition = self.engine.accept(&response, &self.session).await?;
        assert!(matches!(disposition, RiseWireInboundDisposition::ConsumedResponse));
        Ok(())
    }

    async fn seed(
        &self,
        scope: &RiseMessageScope,
        payload: Map<String, Value>,
    ) -> anyhow::Result<RiseMessagePageLoadOutcome> {
        let request = tokio::spawn({
            let repository = self.repository.clone();
            let scope = scope.clone();
            async move { repository.load_initial(&scope).await }
        });
        let (frame, frame_session) = self.sender.next().await;
        assert_eq!(frame_session, self.session);
        self.answer(&frame, payload).await?;
        request.await?
    }

    async fn snapshot(&self, scope: &RiseMessageScope) -> anyhow::Result<RiseMessageWireSnapshot> {
        self.repository
            .snapshot(scope, RiseMessageAnchor::Latest, RiseMessageWindowConfiguration::presentation())
            .await
    }

    async fn message_entity(&self, local_id: &str) -> anyhow::Result<RiseStoreEntity> {
        self.engine
            .entity_local_id(RiseConversationStoreLayout::messages(), local_id.to_owned())
            .await?
            .ok_or_else(|| anyhow::anyhow!("the entity is stored"))
    }
}

async fn on_runtime<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    RiseAccountRuntime::current().spawn(future).await.expect("the account runtime stays alive")
}

async fn eventually(cx: &mut TestAppContext, condition: impl Fn(&mut App) -> bool) {
    for _ in 0..100 {
        cx.run_until_parked();
        if cx.update(|cx| condition(cx)) {
            return;
        }
        on_runtime(tokio::time::sleep(Duration::from_millis(10))).await;
    }
}

async fn accept_on_runtime(
    engine: &Arc<RiseEngine>,
    frame: Vec<u8>,
    session: &RiseWireSocketSession,
) -> anyhow::Result<RiseWireInboundDisposition> {
    let engine = engine.clone();
    let session = session.clone();
    on_runtime(async move { engine.accept(&frame, &session).await }).await
}

async fn domain_flow(
    domain: &Arc<RiseConversationDomain>,
    engine: &Arc<RiseEngine>,
    sender: &Arc<MessageHistoryRiseSender>,
    account_id: &str,
    cx: &mut TestAppContext,
) -> anyhow::Result<()> {
    let session = on_runtime({
        let engine = engine.clone();
        async move { start_session(&engine, 1).await }
    })
    .await?;
    on_runtime({
        let domain = domain.clone();
        async move { domain.activate().await }
    })
    .await?;

    let chat_id = format!("chat-{}", unique_token());
    let scope = RiseMessageScope::new(chat_id.clone(), None, None);
    let owner_id = Uuid::new_v4();
    let load = RiseAccountRuntime::current().spawn({
        let domain = domain.clone();
        let scope = scope.clone();
        async move { domain.load_initial_messages(&scope, None, None, None, Some(owner_id), Some(1)).await }
    });
    let (request, request_session) = sender.next().await;
    assert_eq!(request_session, session);
    let request_root = RiseJSONValue::decode(&request)?;
    assert_eq!(method_of(&request)?, method_value(rise_wire_methods::message::GET_MESSAGES));
    assert_eq!(
        request_root.subscript_key(wire_key::DATA).and_then(|data| data.subscript_key(key::CHAT_ID)),
        Some(&RiseJSONValue::String(chat_id.clone()))
    );
    let message_row = |id: &str, content: &str, created_at: Value| {
        object(vec![
            (key::ID, Value::from(id)),
            (key::CHAT_ID, Value::from(chat_id.as_str())),
            (key::SENDER_ID, Value::from(account_id)),
            (key::CONTENT, Value::from(content)),
            (key::CONTENT_TYPE, Value::from("text")),
            (key::CREATED_AT, created_at),
        ])
    };
    let response = rise_response(
        &request,
        page_root(
            &chat_id,
            None,
            Some(false),
            vec![message_row("message-1", "one", Value::from(1)), message_row("message-2", "two", Value::from(2.0))],
        ),
    )?;
    let disposition = accept_on_runtime(engine, response, &session).await?;
    assert!(matches!(disposition, RiseWireInboundDisposition::ConsumedResponse));
    load.await?;

    let published_scope = scope.clone();
    eventually(cx, move |cx| {
        RiseConversationPresentation::shared(cx).read(cx).messages(&published_scope).len() == 2
    })
    .await;
    let published_ids: Vec<String> = cx.update(|cx| {
        RiseConversationPresentation::shared(cx).read(cx).messages(&scope).iter().map(|row| row.id.clone()).collect()
    });
    assert_eq!(published_ids, vec!["message-1".to_owned(), "message-2".to_owned()]);

    on_runtime({
        let domain = domain.clone();
        let scope = scope.clone();
        async move { domain.load_initial_messages(&scope, None, None, None, Some(owner_id), Some(2)).await }
    })
    .await;
    let scope_key = scope.storage_key();
    let find_cache_trace = move |cx: &mut App| {
        SaiHttpTraceStore::shared(cx)
            .read(cx)
            .entries()
            .iter()
            .find(|entry| {
                entry.fetch_id.as_deref() == Some("message.get_messages")
                    && entry.request_headers.get("x-rise-scope") == Some(&scope_key)
                    && entry.response_headers.get("x-rise-rpc-attempts").map(String::as_str) == Some("0")
            })
            .cloned()
    };
    let probe = find_cache_trace.clone();
    eventually(cx, move |cx| probe(cx).is_some()).await;
    let cache_trace = cx.update(|cx| find_cache_trace(cx));
    assert_eq!(
        cache_trace.as_ref().and_then(|entry| entry.response_headers.get("x-rise-source")).map(String::as_str),
        Some("normalized-store")
    );
    assert_eq!(
        cache_trace.as_ref().and_then(|entry| entry.response_headers.get("x-rise-ui-result")).map(String::as_str),
        Some("published")
    );

    let temp_id = format!("temp-{}", unique_token());
    let create = RiseAccountRuntime::current().spawn({
        let domain = domain.clone();
        let scope = scope.clone();
        let draft = RiseMessageDraft::new(outgoing_message(&chat_id, None, "optimistic", &temp_id));
        async move { domain.submit_message(draft, &scope, RiseMessageSubmissionMode::Immediate).await }
    });
    let (create_request, create_session) = sender.next().await;
    assert_eq!(create_session, session);
    assert_eq!(method_of(&create_request)?, method_value(rise_wire_methods::message::CREATE_MESSAGE));

    let optimistic_scope = scope.clone();
    let optimistic_id = temp_id.clone();
    eventually(cx, move |cx| {
        RiseConversationPresentation::shared(cx)
            .read(cx)
            .messages(&optimistic_scope)
            .iter()
            .any(|row| row.id == optimistic_id)
    })
    .await;
    let has_optimistic_row = cx.update(|cx| {
        RiseConversationPresentation::shared(cx)
            .read(cx)
            .messages(&scope)
            .iter()
            .any(|row| row.id == temp_id && row.text == "optimistic")
    });
    assert!(has_optimistic_row);

    let create_response = rise_response(&create_request, create_acknowledgement(&chat_id, "message-3", &temp_id))?;
    let create_disposition = accept_on_runtime(engine, create_response, &session).await?;
    assert!(matches!(create_disposition, RiseWireInboundDisposition::ConsumedResponse));
    create.await??;
    Ok(())
}

#[gpui::test]
async fn domain_publishes_initial_history_after_repository_commit(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _serial = SERIAL.lock().await;
    cx.update(|cx| {
        RiseEngineMainHop::shared(cx);
    });
    let initial_trace_ids: HashSet<Uuid> =
        cx.update(|cx| SaiHttpTraceStore::shared(cx).read(cx).entries().iter().map(|entry| entry.id).collect());
    let _restore_scope = RestoreAccountScope(SaiAccountCacheScope::active_user_id());
    let account_id = format!("message-domain-publish-{}", unique_token());
    SaiAccountCacheScope::activate(&account_id);
    let account_epoch = SaiAccountCacheScope::active_epoch().expect("activation sets an epoch");
    let sender = MessageHistoryRiseSender::new();
    let root = temporary_root("rise-message-domain-publish");

    let engine = on_runtime({
        let root = root.0.clone();
        let account_id = account_id.clone();
        let sender = sender.clone();
        async move { open_engine(root, &account_id, &account_epoch, sender).await }
    })
    .await
    .expect("the engine opens");
    let domain = RiseConversationDomain::new(engine.clone());

    let result = domain_flow(&domain, &engine, &sender, &account_id, cx).await;
    on_runtime({
        let domain = domain.clone();
        async move { domain.shutdown().await }
    })
    .await;
    let closed = on_runtime({
        let engine = engine.clone();
        async move { finish_engine(&engine, result).await }
    })
    .await;
    cx.run_until_parked();
    cx.update(|cx| {
        let store = SaiHttpTraceStore::shared(cx);
        let added: Vec<Uuid> = store
            .read(cx)
            .entries()
            .iter()
            .map(|entry| entry.id)
            .filter(|id| !initial_trace_ids.contains(id))
            .collect();
        store.update(cx, |store, cx| {
            for id in added {
                store.remove_entry(id, cx);
            }
        });
    });
    closed.expect("the domain flow finishes");
}

#[test]
fn message_record_decodes_without_persisted_scope_key() -> anyhow::Result<()> {
    let wire_root = Value::Object(object(vec![
        (key::ID, Value::from("legacy-message")),
        (key::CHAT_ID, Value::from("legacy-chat")),
        (key::CONTENT_TYPE, Value::from("text")),
    ]));
    let wire: MessageWireDTO = MessagePayloadCodec::decode(Some(&wire_root)).expect("the wire decodes");
    let mut record = RiseMessageEntityRecord::new(wire, RiseConversationEntitySource::Optimistic);
    record.local_phase = RiseMessageLocalPhase::Queued;
    record.client_request_id = Some("123".to_owned());
    record.scope_storage_key = Some("legacy-chat::th::thread-root".to_owned());
    let encoded = RiseMessageWireCodec::record_data(&record)?;
    let mut encoded_object: Map<String, Value> = serde_json::from_slice(&encoded)?;
    encoded_object.remove(SCOPE_STORAGE_KEY_FIELD);
    let legacy = serde_json::to_vec(&Value::Object(encoded_object))?;

    let decoded = RiseMessageWireCodec::record(&legacy)?;
    assert_eq!(decoded.scope_storage_key, None);
    assert_eq!(decoded.client_request_id.as_deref(), Some("123"));
    assert_eq!(decoded.local_phase, RiseMessageLocalPhase::Queued);
    Ok(())
}

#[gpui::test]
fn newest_seed_request_supersedes_older_initial_or_around_response(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    cx.update(|cx| {
        let scope = format!("test-message-seed-token-{}", unique_token());
        let service = MessageServicesStore::shared(cx);
        let older = service.update(cx, |service, cx| service.begin_message_seed_request(&scope, cx));
        let newer = service.update(cx, |service, cx| service.begin_message_seed_request(&scope, cx));

        assert!(!service.read(cx).is_current_message_seed_request(&scope, older));
        assert!(service.read(cx).is_current_message_seed_request(&scope, newer));

        service.update(cx, |service, cx| service.finish_message_seed_request(&scope, older, cx));
        assert!(service.read(cx).is_current_message_seed_request(&scope, newer));

        service.update(cx, |service, cx| service.finish_message_seed_request(&scope, newer, cx));
        assert!(!service.read(cx).is_current_message_seed_request(&scope, newer));
    });
}

#[tokio::test]
async fn reconnect_refresh_uses_fresh_seed_and_commits_missed_message() -> anyhow::Result<()> {
    let harness = MessageHistoryHarness::open("message-reconnect").await?;
    let chat_id = format!("test-reconnect-refresh-seed-{}", unique_token());
    let scope = RiseMessageScope::new(chat_id.clone(), None, None);
    let result = async {
        let initial = harness
            .seed(&scope, page_root(&chat_id, None, Some(false), vec![row("message-before-drop", 1.0)]))
            .await?;
        assert_eq!(initial, RiseMessagePageLoadOutcome::Loaded { count: 1 });
        let initial_snapshot = harness.snapshot(&scope).await?;
        assert_eq!(wire_ids(&initial_snapshot), vec!["message-before-drop".to_owned()]);

        harness.engine.socket_disconnected(harness.session.clone(), "test reconnect".to_owned()).await?;
        let session = start_session(&harness.engine, 2).await?;

        let reconnect_request = tokio::spawn({
            let repository = harness.repository.clone();
            let scope = scope.clone();
            async move { repository.load_initial(&scope).await }
        });
        let (reconnect_frame, reconnect_session) = harness.sender.next().await;
        assert_eq!(reconnect_session, session);
        let reconnect_response = rise_response(
            &reconnect_frame,
            page_root(
                &chat_id,
                None,
                Some(false),
                vec![row("message-before-drop", 1.0), row("message-missed-offline", 2.0)],
            ),
        )?;
        let disposition = harness.engine.accept(&reconnect_response, &session).await?;
        assert!(matches!(disposition, RiseWireInboundDisposition::ConsumedResponse));
        assert_eq!(reconnect_request.await??, RiseMessagePageLoadOutcome::Loaded { count: 2 });
        let refreshed = harness.snapshot(&scope).await?;
        assert_eq!(wire_ids(&refreshed), vec!["message-before-drop".to_owned(), "message-missed-offline".to_owned()]);
        Ok(())
    }
    .await;
    finish_engine(&harness.engine, result).await
}

#[tokio::test]
async fn seed_keeps_row_whose_entity_moved_ahead_of_its_membership() -> anyhow::Result<()> {
    let harness = MessageHistoryHarness::open("message-seed-entity-version").await?;
    let chat_id = format!("test-seed-entity-version-{}", unique_token());
    let scope = RiseMessageScope::new(chat_id.clone(), None, None);
    let payload = page_root(&chat_id, None, Some(false), vec![row("message-b", 2.0), row("message-a", 1.0)]);
    let result = async {
        let first = harness.seed(&scope, payload.clone()).await?;
        assert_eq!(first, RiseMessagePageLoadOutcome::Loaded { count: 2 });
        let seeded = harness.snapshot(&scope).await?;
        assert_eq!(wire_ids(&seeded), vec!["message-a".to_owned(), "message-b".to_owned()]);

        let entity = harness.message_entity("message-a").await?;
        harness
            .engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::UpsertEntity {
                    namespace: RiseConversationStoreLayout::messages(),
                    entity: RiseStoreEntity::new(
                        entity.local_id.clone(),
                        entity.data.clone(),
                        entity.version + 100_000,
                        entity.updated_at_milliseconds + 1,
                    ),
                    policy: RiseEntityWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await?;

        let second = harness.seed(&scope, payload.clone()).await?;
        assert_eq!(second, RiseMessagePageLoadOutcome::Loaded { count: 2 });
        let reseeded = harness.snapshot(&scope).await?;
        assert_eq!(wire_ids(&reseeded), vec!["message-a".to_owned(), "message-b".to_owned()]);
        Ok(())
    }
    .await;
    finish_engine(&harness.engine, result).await
}

#[tokio::test]
async fn seed_leaves_unchanged_rows_untouched() -> anyhow::Result<()> {
    let harness = MessageHistoryHarness::open("message-seed-unchanged").await?;
    let chat_id = format!("test-seed-unchanged-{}", unique_token());
    let scope = RiseMessageScope::new(chat_id.clone(), None, None);
    let payload = page_root(
        &chat_id,
        None,
        Some(false),
        vec![content_row("message-b", "b", 2.0), content_row("message-a", "a", 1.0)],
    );
    let edited_payload = page_root(
        &chat_id,
        None,
        Some(false),
        vec![content_row("message-b", "b edited", 2.0), content_row("message-a", "a", 1.0)],
    );
    let loaded = RiseMessagePageLoadOutcome::Loaded { count: 2 };
    let result = async {
        assert_eq!(harness.seed(&scope, payload.clone()).await?, loaded);
        let first_a = harness.message_entity("message-a").await?;
        let first_b = harness.message_entity("message-b").await?;

        assert_eq!(harness.seed(&scope, payload.clone()).await?, loaded);
        let second_a = harness.message_entity("message-a").await?;
        let second_b = harness.message_entity("message-b").await?;
        assert_eq!(second_a.version, first_a.version);
        assert_eq!(second_a.updated_at_milliseconds, first_a.updated_at_milliseconds);
        assert_eq!(second_b.version, first_b.version);
        let reseeded = harness.snapshot(&scope).await?;
        assert_eq!(wire_ids(&reseeded), vec!["message-a".to_owned(), "message-b".to_owned()]);

        assert_eq!(harness.seed(&scope, edited_payload.clone()).await?, loaded);
        let third_a = harness.message_entity("message-a").await?;
        let third_b = harness.message_entity("message-b").await?;
        assert_eq!(third_a.version, first_a.version);
        assert!(third_b.version > first_b.version);
        let edited = harness.snapshot(&scope).await?;
        assert_eq!(wire_ids(&edited), vec!["message-a".to_owned(), "message-b".to_owned()]);
        assert_eq!(edited.rows.last().and_then(|row| row.wire.content.as_deref()), Some("b edited"));
        Ok(())
    }
    .await;
    finish_engine(&harness.engine, result).await
}

#[tokio::test]
async fn seed_reads_newest_first_page_as_timeline_when_dropping_contradicted_rows() -> anyhow::Result<()> {
    let harness = MessageHistoryHarness::open("message-seed-page-order").await?;
    let chat_id = format!("test-seed-page-order-{}", unique_token());
    let scope = RiseMessageScope::new(chat_id.clone(), None, None);
    let result = async {
        let first = harness
            .seed(
                &scope,
                page_root(
                    &chat_id,
                    None,
                    Some(false),
                    vec![row("message-c", 3.0), row("message-b", 2.0), row("message-a", 1.0)],
                ),
            )
            .await?;
        assert_eq!(first, RiseMessagePageLoadOutcome::Loaded { count: 3 });

        let second = harness
            .seed(&scope, page_root(&chat_id, None, Some(false), vec![row("message-c", 3.0), row("message-a", 1.0)]))
            .await?;
        assert_eq!(second, RiseMessagePageLoadOutcome::Loaded { count: 2 });
        let reseeded = harness.snapshot(&scope).await?;
        assert_eq!(wire_ids(&reseeded), vec!["message-a".to_owned(), "message-c".to_owned()]);
        Ok(())
    }
    .await;
    finish_engine(&harness.engine, result).await
}

#[tokio::test]
async fn failed_saved_anchor_falls_back_to_latest_messages_request() -> anyhow::Result<()> {
    let harness = MessageHistoryHarness::open("message-anchor-fallback").await?;
    let chat_id = format!("test-anchor-fallback-{}", unique_token());
    let scope = RiseMessageScope::new(chat_id.clone(), None, None);
    let result = async {
        let request = tokio::spawn({
            let repository = harness.repository.clone();
            let scope = scope.clone();
            async move {
                repository
                    .load_initial_receipt(
                        &scope,
                        Some(RiseMessageAnchor::ServerID("stale-saved-anchor".to_owned())),
                        RiseMessageWindowConfiguration::presentation(),
                    )
                    .await
            }
        });

        let (around_frame, around_session) = harness.sender.next().await;
        assert_eq!(around_session, harness.session);
        assert_eq!(method_of(&around_frame)?, method_value(rise_wire_methods::message::GET_MESSAGES_AROUND));
        let around_failure = rise_error_response(&around_frame, "message_not_found")?;
        let around_disposition = harness.engine.accept(&around_failure, &harness.session).await?;
        assert!(matches!(around_disposition, RiseWireInboundDisposition::ConsumedResponse));

        let (latest_frame, latest_session) = harness.sender.next().await;
        assert_eq!(latest_session, harness.session);
        assert_eq!(method_of(&latest_frame)?, method_value(rise_wire_methods::message::GET_MESSAGES));
        let mut latest_row = row("latest-message", 2.0);
        latest_row.insert(key::CHAT_ID.to_owned(), Value::from(chat_id.as_str()));
        harness.answer(&latest_frame, page_root(&chat_id, None, Some(false), vec![latest_row])).await?;

        let receipt = request.await??;
        assert_eq!(receipt.outcome, RiseMessagePageLoadOutcome::Loaded { count: 1 });
        assert!(receipt.used_latest_fallback);
        let snapshot = harness.snapshot(&scope).await?;
        assert_eq!(wire_ids(&snapshot), vec!["latest-message".to_owned()]);
        Ok(())
    }
    .await;
    finish_engine(&harness.engine, result).await
}

#[tokio::test]
async fn typed_optimistic_message_commits_before_create_acknowledgement() -> anyhow::Result<()> {
    let harness = MessageHistoryHarness::open("message-typed-optimistic").await?;
    let chat_id = format!("test-typed-optimistic-{}", unique_token());
    let thread_root_id = format!("thread-{}", unique_token());
    let temp_id = format!("temp-{}", unique_token());
    let scope = RiseMessageScope::new(chat_id.clone(), None, Some(thread_root_id));
    let result = async {
        let request = tokio::spawn({
            let repository = harness.repository.clone();
            let scope = scope.clone();
            let draft = RiseMessageDraft::new(outgoing_message(&chat_id, None, "hello", &temp_id));
            async move { repository.create(draft, &scope, RiseMessageSubmissionMode::Immediate).await }
        });

        let (create_frame, create_session) = harness.sender.next().await;
        assert_eq!(create_session, harness.session);
        assert_eq!(method_of(&create_frame)?, method_value(rise_wire_methods::message::CREATE_MESSAGE));

        let optimistic = harness.snapshot(&scope).await?;
        assert_eq!(optimistic.local_ids, vec![temp_id.clone()]);
        assert_eq!(optimistic.rows.first().map(|row| row.local_phase), Some(RiseMessageLocalPhase::Queued));
        assert_eq!(optimistic.rows.first().and_then(|row| row.wire.content.as_deref()), Some("hello"));

        harness.answer(&create_frame, create_acknowledgement(&chat_id, "server-message", &temp_id)).await?;
        let submission = request.await??;
        assert_eq!(submission.local_id, temp_id);
        let acknowledgement = harness
            .repository
            .durable_telemetry_record(submission.request_id)
            .await?
            .expect("the acknowledgement resolves");
        assert_eq!(acknowledgement.request_id, submission.request_id);
        assert_eq!(acknowledgement.scope, scope);
        assert_eq!(acknowledgement.local_id, temp_id);
        assert_eq!(acknowledgement.local_phase, RiseMessageLocalPhase::Remote);
        assert_eq!(acknowledgement.last_error_code, None);
        assert!(acknowledgement.entity_version > 0);
        assert_eq!(acknowledgement.entity_source, RiseConversationEntitySource::Acknowledgement);
        assert!(acknowledgement.source_revision >= submission.store_revision);

        let acknowledged = harness.snapshot(&scope).await?;
        assert_eq!(acknowledged.local_ids, vec![temp_id.clone()]);
        assert_eq!(acknowledged.rows.first().map(|row| row.local_phase), Some(RiseMessageLocalPhase::Remote));
        assert_eq!(
            acknowledged.rows.first().and_then(|row| row.wire.server_id.as_deref()),
            Some("server-message")
        );

        let realtime_root = Value::Object(object(vec![
            (key::ID, Value::from("server-message")),
            (RiseLocalKey::SERVER_ID, Value::from("server-message")),
            (key::CHAT_ID, Value::from(chat_id.as_str())),
            (key::TEMP_ID, Value::from(temp_id.as_str())),
            (key::CONTENT, Value::from("hello from realtime")),
            (key::CONTENT_TYPE, Value::from("text")),
            (key::CREATED_AT, Value::from(3.0)),
        ]));
        let realtime_wire: MessageWireDTO =
            MessagePayloadCodec::decode(Some(&realtime_root)).expect("the realtime wire decodes");
        harness
            .repository
            .ingest_authoritative(vec![RiseAuthoritativeMessage::new(realtime_wire, scope.clone(), None)])
            .await?;
        let realtime = harness
            .repository
            .durable_telemetry_record(submission.request_id)
            .await?
            .expect("the realtime row resolves");
        assert_eq!(realtime.scope, scope);
        assert_eq!(realtime.local_id, temp_id);
        assert_eq!(realtime.local_phase, RiseMessageLocalPhase::Remote);
        assert_eq!(realtime.entity_source, RiseConversationEntitySource::Realtime);
        assert!(realtime.source_revision > acknowledgement.source_revision);
        Ok(())
    }
    .await;
    finish_engine(&harness.engine, result).await
}

#[tokio::test]
async fn durable_optimistic_thread_scope_resolves_after_relaunch() -> anyhow::Result<()> {
    let _serial = SERIAL.lock().await;
    let chat_id = format!("test-durable-optimistic-{}", unique_token());
    let thread_root_id = format!("thread-{}", unique_token());
    let temp_id = format!("temp-{}", unique_token());
    let root = temporary_root("rise-message-durable-optimistic");
    let account_id = format!("message-durable-optimistic-{}", unique_token());
    let scope = RiseMessageScope::new(chat_id.clone(), None, Some(thread_root_id.clone()));
    let engine = open_engine(root.0.clone(), &account_id, "test", MessageHistoryRiseSender::new()).await?;
    let first_run = async {
        engine.register_durable_endpoint::<RiseCreateMessageDurableEndpoint>().await?;
        let repository = repository_for(&engine, &account_id);
        let submission = repository
            .create(
                RiseMessageDraft::new(outgoing_message(&chat_id, None, "offline hello", &temp_id)),
                &scope,
                RiseMessageSubmissionMode::Durable,
            )
            .await?;

        let snapshot = repository
            .snapshot(&scope, RiseMessageAnchor::Latest, RiseMessageWindowConfiguration::presentation())
            .await?;
        let operation = engine.durable_operation(submission.request_id).await?;
        assert_eq!(submission.local_id, temp_id);
        assert_eq!(snapshot.local_ids, vec![temp_id.clone()]);
        assert_eq!(snapshot.rows.first().map(|row| row.local_phase), Some(RiseMessageLocalPhase::Queued));
        assert_eq!(snapshot.rows.first().and_then(|row| row.wire.content.as_deref()), Some("offline hello"));
        assert!(operation.is_some());
        assert_eq!(operation.as_ref().map(|operation| operation.request_id), Some(submission.request_id));
        assert_eq!(
            operation.as_ref().map(|operation| operation.kind.as_str()),
            Some(RiseCreateMessageDurableEndpoint::KIND)
        );
        Ok(submission)
    }
    .await;
    let submission = match first_run {
        Ok(submission) => {
            engine.close().await?;
            submission
        }
        Err(error) => {
            let _ = engine.close().await;
            return Err(error);
        }
    };

    let reopened_engine = open_engine(root.0.clone(), &account_id, "test", MessageHistoryRiseSender::new()).await?;
    let reopened = async {
        let reopened_repository = repository_for(&reopened_engine, &account_id);
        let record = reopened_repository
            .durable_telemetry_record(submission.request_id)
            .await?
            .expect("the durable row resolves after relaunch");
        assert_eq!(record.request_id, submission.request_id);
        assert_eq!(record.scope, scope);
        assert_eq!(record.scope.thread_root_id.as_deref(), Some(thread_root_id.as_str()));
        assert_eq!(record.local_id, temp_id);
        assert_eq!(record.local_phase, RiseMessageLocalPhase::Queued);
        assert_eq!(record.last_error_code, None);
        assert!(record.entity_version > 0);
        assert_eq!(record.entity_source, RiseConversationEntitySource::Optimistic);
        assert!(record.source_revision >= submission.store_revision);
        Ok(())
    }
    .await;
    finish_engine(&reopened_engine, reopened).await
}

#[tokio::test]
async fn failed_message_resolves_from_durable_request_alias() -> anyhow::Result<()> {
    let harness = MessageHistoryHarness::open("message-failed-telemetry").await?;
    let chat_id = format!("test-failed-message-telemetry-{}", unique_token());
    let topic_id = format!("topic-{}", unique_token());
    let temp_id = format!("temp-{}", unique_token());
    let scope = RiseMessageScope::new(chat_id.clone(), Some(topic_id.clone()), None);
    let result = async {
        let request = tokio::spawn({
            let repository = harness.repository.clone();
            let scope = scope.clone();
            let draft = RiseMessageDraft::new(outgoing_message(&chat_id, Some(&topic_id), "will fail", &temp_id));
            async move { repository.create(draft, &scope, RiseMessageSubmissionMode::Immediate).await }
        });

        let (create_frame, create_session) = harness.sender.next().await;
        assert_eq!(create_session, harness.session);
        let request_id = request_id_of(&create_frame)?;
        let failure_code = "terminal_create_failure";
        let response = rise_error_response(&create_frame, failure_code)?;
        let disposition = harness.engine.accept(&response, &harness.session).await?;
        assert!(matches!(disposition, RiseWireInboundDisposition::ConsumedResponse));
        let failure = request.await?.expect_err("the create request fails");
        assert!(failure.downcast_ref::<RiseWireRemoteError>().is_some());

        let record = harness
            .repository
            .durable_telemetry_record(request_id)
            .await?
            .expect("the failed row resolves");
        assert_eq!(record.request_id, request_id);
        assert_eq!(record.scope, scope);
        assert_eq!(record.scope.thread_root_id, None);
        assert_eq!(record.local_id, temp_id);
        assert_eq!(record.local_phase, RiseMessageLocalPhase::Failed);
        assert_eq!(record.last_error_code.as_deref(), Some(failure_code));
        assert!(record.entity_version > 0);
        assert_eq!(record.entity_source, RiseConversationEntitySource::Optimistic);
        assert!(record.source_revision > 0);
        Ok(())
    }
    .await;
    finish_engine(&harness.engine, result).await
}

#[gpui::test]
fn new_seed_invalidates_directional_request_token(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    cx.update(|cx| {
        let scope = format!("test-message-page-token-{}", unique_token());
        let older = MessageHistoryDirection::Older;
        let service = MessageServicesStore::shared(cx);
        let token = service.update(cx, |service, cx| service.begin_message_history_request(&scope, older, cx));
        assert!(token.is_some());
        assert!(service.read(cx).is_message_history_request_loading(&scope, older));

        service.update(cx, |service, cx| service.begin_initial_messages_request(&scope, true, cx));
        assert!(!service.read(cx).is_message_history_request_loading(&scope, older));
        if let Some(token) = token {
            assert!(!service.read(cx).is_current_message_history_request(&scope, older, token));
        }

        service.update(cx, |service, cx| service.finish_message_history_request(&scope, older, token, true, cx));
        assert!(!service.read(cx).is_message_history_request_loading(&scope, older));
        let replacement = service.update(cx, |service, cx| service.begin_message_history_request(&scope, older, cx));
        assert!(replacement.is_some());
        service.update(cx, |service, cx| service.finish_message_history_request(&scope, older, replacement, false, cx));
    });
}

#[test]
fn equal_millisecond_timestamps_use_descending_id_order() {
    let timestamp = MessageThreadOrder::normalize(1_725_000_000_123.0);
    let message = |id: &str| {
        let mut value = GetMessageMessage::new(id.to_owned());
        value.created_at = timestamp;
        value
    };
    let first_wire_order = vec![message("message-c"), message("message-a"), message("message-b")];
    let second_wire_order: Vec<GetMessageMessage> = first_wire_order.iter().rev().cloned().collect();
    let ids = |rows: Vec<GetMessageMessage>| -> Vec<String> {
        MessageThreadOrder::chronological(rows).into_iter().map(|row| row.id).collect()
    };
    let expected = vec!["message-c".to_owned(), "message-b".to_owned(), "message-a".to_owned()];

    assert_eq!(ids(first_wire_order), expected);
    assert_eq!(ids(second_wire_order), expected);
}

#[gpui::test]
fn older_page_cut_inside_same_millisecond_prepends_in_cassandra_reverse_order(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-same-millisecond-page-cut-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
        let timestamp = 1_725_000_000.123;

        let initial_page = gateway_page(
            &chat_id,
            Some("same-ms-cursor"),
            true,
            vec![row("message-a", timestamp), row("message-b", timestamp)],
        );
        service.update(cx, |service, cx| service.install_initial_messages_page(Some(&initial_page), &chat_id, cx));

        let older_page = gateway_page(
            &chat_id,
            Some("same-ms-cursor-2"),
            false,
            vec![row("message-c", timestamp), row("message-d", timestamp)],
        );
        service.update(cx, |service, cx| {
            service.merge_messages_page(
                Some(&older_page),
                &chat_id,
                MessageHistoryDirection::Older,
                Some("same-ms-cursor"),
                None,
                cx,
            );
        });

        let root = canonical_root(&cache_key, cx);
        assert_eq!(ids_of(&root), ["message-d", "message-c", "message-b", "message-a"]);
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn deep_history_checkpoint_restores_every_loaded_row_from_disk(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-deep-checkpoint-{}", unique_token());
    let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let rows: Vec<Map<String, Value>> = (0..750)
            .map(|index| content_row(&format!("message-{index}"), &format!("body-{index}"), f64::from(index)))
            .collect();
        patch_root(&service, &chat_id, Some("older-cursor"), Some(true), &[], rows, cx);
    });
    flush_and_wait(&chat_id, cx);
    cx.update(|cx| {
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Memory, cx);
        let service = MessageServicesStore::shared(cx);
        service.update(cx, |service, cx| service.preload_messages_slice_from_disk_if_needed(&chat_id, cx));

        let restored_root = memory_root(&cache_key, cx);
        let restored_rows = rows_of(&restored_root);
        assert_eq!(restored_rows.len(), 750);
        assert_eq!(restored_rows.first().and_then(|row| string_of(row, key::ID)), Some("message-0"));
        assert_eq!(restored_rows.last().and_then(|row| string_of(row, key::ID)), Some("message-749"));
        assert_eq!(string_of(&restored_root, key::RELATIVE_ID), Some("older-cursor"));
        assert_eq!(restored_root.get(key::IS_HAVE_MORE).and_then(Value::as_bool), Some(true));
    });
    cx.update(|cx| SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx));
}

#[gpui::test]
fn initial_page_is_installed_chronologically_without_duplicates(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-initial-install-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
        let page = gateway_page(
            &chat_id,
            Some("initial-cursor"),
            true,
            vec![
                content_row("message-3", "third", 3.0),
                content_row("message-2", "stale", 2.0),
                content_row("message-1", "first", 1.0),
                content_row("message-2", "second", 2.0),
            ],
        );

        service.update(cx, |service, cx| service.install_initial_messages_page(Some(&page), &chat_id, cx));

        let root = canonical_root(&cache_key, cx);
        let rows = rows_of(&root);
        assert_eq!(ids_of(&root), ["message-1", "message-2", "message-3"]);
        assert_eq!(
            rows.iter()
                .find(|row| string_of(row, key::ID) == Some("message-2"))
                .and_then(|row| string_of(row, key::CONTENT)),
            Some("second")
        );
        assert_eq!(string_of(&root, key::RELATIVE_ID), Some("initial-cursor"));
        assert_eq!(root.get(key::IS_HAVE_MORE).and_then(Value::as_bool), Some(true));
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn typed_refresh_preserves_older_history_and_reconciles_newest_window(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-refresh-merge-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
        let old_rows = vec![
            content_row("old-1", "old", 1.0),
            content_row("server-2", "before edit", 2.0),
            content_row("deleted-3", "deleted", 3.0),
            temp_row("temp_4000_local", Some("pending"), 4.0),
        ];
        SaiWsResponseCache::install_optimistic_root(
            Value::Object(page_root(&chat_id, Some("cached-cursor"), Some(true), old_rows)),
            &cache_key,
            true,
            true,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            cx,
        )
        .expect("the root installs");

        let page = gateway_page(
            &chat_id,
            Some("fresh-cursor"),
            true,
            vec![content_row("server-2", "after edit", 2.0), content_row("server-4", "new", 4.0)],
        );

        service.update(cx, |service, cx| service.merge_messages_refresh(Some(&page), &chat_id, None, cx));

        let root = canonical_root(&cache_key, cx);
        let rows = rows_of(&root);
        let ids = ids_of(&root);
        assert!(ids.iter().any(|id| id == "old-1"));
        assert!(!ids.iter().any(|id| id == "deleted-3"));
        assert_eq!(
            rows.iter()
                .find(|row| string_of(row, key::ID) == Some("server-2"))
                .and_then(|row| string_of(row, key::CONTENT)),
            Some("after edit")
        );
        assert!(ids.iter().any(|id| id == "server-4"));
        assert_eq!(ids.iter().filter(|id| id.as_str() == "temp_4000_local").count(), 1);
        assert_eq!(string_of(&root, key::RELATIVE_ID), Some("cached-cursor"));
        assert_eq!(root.get(key::IS_HAVE_MORE).and_then(Value::as_bool), Some(true));
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn latest_snapshot_never_installs_a_stale_disk_only_tail(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-latest-disk-tail-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        patch_root(
            &service,
            &chat_id,
            None,
            None,
            &[(MessageHistoryDirection::Newer, "canonical-2", false)],
            vec![row("canonical-1", 10.0), row("canonical-2", 20.0)],
            cx,
        );
        replace_latest(
            &MessageServicesStore::latest_messages_cache_key(&chat_id),
            page_root(&chat_id, None, None, vec![row("stale-1", 1.0)]),
            SaiUpdaterWriteScope::DiskCache,
            cx,
        );

        let _ = service.update(cx, |service, cx| service.install_latest_messages_snapshot(&chat_id, cx));

        let root = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        assert_eq!(ids_of(&root), ["canonical-1", "canonical-2"]);
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn stale_empty_latest_snapshot_never_erases_nonempty_canonical(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-empty-latest-tail-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        patch_root(
            &service,
            &chat_id,
            None,
            None,
            &[(MessageHistoryDirection::Newer, "canonical-2", false)],
            vec![row("canonical-1", 10.0), row("canonical-2", 20.0)],
            cx,
        );
        replace_latest(
            &MessageServicesStore::latest_messages_cache_key(&chat_id),
            page_root(&chat_id, None, Some(false), Vec::new()),
            SaiUpdaterWriteScope::DiskCache,
            cx,
        );

        assert!(service.update(cx, |service, cx| service.install_latest_messages_snapshot(&chat_id, cx)));

        let root = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        assert_eq!(ids_of(&root), ["canonical-1", "canonical-2"]);
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn latest_snapshot_projects_without_discarding_continuous_deep_history(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-latest-project-tail-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let rows: Vec<Map<String, Value>> =
            (0..500).map(|index| row(&format!("message-{index:03}"), f64::from(index + 1))).collect();
        patch_root(
            &service,
            &chat_id,
            Some("older-cursor"),
            Some(true),
            &[
                (MessageHistoryDirection::Older, "older-cursor", true),
                (MessageHistoryDirection::Newer, "message-499", false),
            ],
            rows.clone(),
            cx,
        );
        replace_latest(
            &MessageServicesStore::latest_messages_cache_key(&chat_id),
            page_root(&chat_id, None, None, rows[400..].to_vec()),
            SaiUpdaterWriteScope::Cache,
            cx,
        );

        assert!(service.update(cx, |service, cx| service.install_latest_messages_snapshot(&chat_id, cx)));

        let root = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        let installed = rows_of(&root);
        assert_eq!(installed.len(), 500);
        assert_eq!(installed.first().and_then(|row| string_of(row, key::ID)), Some("message-000"));
        assert_eq!(installed.last().and_then(|row| string_of(row, key::ID)), Some("message-499"));
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn latest_tail_trim_moves_older_cursor_to_oldest_retained_server_row(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-latest-trim-cursor-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let limit = MessagePresentationWindowProjector::limit();
        let rows: Vec<Map<String, Value>> = (0..limit)
            .map(|index| {
                if index == 1 {
                    return object(vec![
                        (key::ID, Value::from("contact-join-event:local")),
                        (key::CONTENT_TYPE, Value::from("system")),
                        (key::SYSTEM_MESSAGE_TYPE, Value::from(MessageSystemKind::ContactJoined.raw_value())),
                        (key::CREATED_AT, Value::from((index + 1) as f64)),
                    ]);
                }
                row(&format!("message-{index:03}"), (index + 1) as f64)
            })
            .collect();
        let latest_cache_key = MessageServicesStore::latest_messages_cache_key(&chat_id);
        replace_latest(
            &MessageServicesStore::messages_cache_key(&chat_id),
            page_root(&chat_id, None, None, rows.clone()),
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        let mut latest = page_root(&chat_id, Some("message-000"), Some(false), rows);
        let older_lane = object(vec![
            (key::RELATIVE_ID, Value::from("message-000")),
            (key::HAS_MORE, Value::Bool(false)),
        ]);
        latest.insert(
            HISTORY_PAGINATION_KEY.to_owned(),
            Value::Object(object(vec![(MessageHistoryDirection::Older.raw_value(), Value::Object(older_lane))])),
        );
        replace_latest(&latest_cache_key, latest, SaiUpdaterWriteScope::Cache, cx);

        service.update(cx, |service, cx| {
            service.mutate_messages_list(
                &chat_id,
                SaiUpdaterWriteScope::Cache,
                |messages| {
                    let mut next = GetMessageMessage::new("message-320".to_owned());
                    next.created_at = 321.0;
                    messages.push(next);
                    MessageThreadOrder::chronological(messages.clone())
                },
                cx,
            );
        });

        let latest_root = memory_root(&latest_cache_key, cx);
        let latest_rows = rows_of(&latest_root);
        assert_eq!(latest_rows.len() as i64, limit);
        assert_eq!(latest_rows.first().and_then(|row| string_of(row, key::ID)), Some("contact-join-event:local"));
        assert_eq!(string_of(&latest_root, key::RELATIVE_ID), Some("message-002"));
        assert_eq!(latest_root.get(key::IS_HAVE_MORE).and_then(Value::as_bool), Some(true));
        let history =
            MessagePayloadCodec::dictionary(latest_root.get(HISTORY_PAGINATION_KEY)).expect("the root has lanes");
        let older = MessagePayloadCodec::dictionary(history.get(MessageHistoryDirection::Older.raw_value()))
            .expect("the older lane exists");
        assert_eq!(string_of(&older, key::RELATIVE_ID), Some("message-002"));
        assert_eq!(older.get(key::HAS_MORE).and_then(Value::as_bool), Some(true));
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn latest_snapshot_keeps_pending_local_message_when_replacing_distant_window(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-latest-pending-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        patch_root(
            &service,
            &chat_id,
            Some("distant-older"),
            Some(true),
            &[
                (MessageHistoryDirection::Older, "distant-older", true),
                (MessageHistoryDirection::Newer, "distant-newer", true),
            ],
            vec![row("distant-1", 1.0), temp_row("temp_3000_pending", None, 30.0)],
            cx,
        );
        replace_latest(
            &MessageServicesStore::latest_messages_cache_key(&chat_id),
            page_root(&chat_id, None, None, vec![row("latest-10", 10.0), row("latest-20", 20.0)]),
            SaiUpdaterWriteScope::Cache,
            cx,
        );

        assert!(service.update(cx, |service, cx| service.install_latest_messages_snapshot(&chat_id, cx)));

        let root = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        assert_eq!(ids_of(&root), ["latest-10", "latest-20", "temp_3000_pending"]);
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn initial_refresh_fence_preserves_realtime_rows_that_arrive_after_dispatch(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-refresh-fence-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let latest_cache_key = MessageServicesStore::latest_messages_cache_key(&chat_id);
        let initial_rows = vec![content_row("message-1", "first", 1.0), content_row("message-2", "second", 2.0)];
        patch_root(
            &service,
            &chat_id,
            None,
            None,
            &[(MessageHistoryDirection::Newer, "message-2", false)],
            initial_rows.clone(),
            cx,
        );
        replace_latest(
            &latest_cache_key,
            page_root(&chat_id, None, None, initial_rows.clone()),
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        let token = service.update(cx, |service, cx| service.begin_message_seed_request(&chat_id, cx));
        let fence = service.read(cx).message_seed_refresh_fence(&chat_id, token).expect("the seed has a fence");

        service.update(cx, |service, cx| {
            service.mutate_messages_list(
                &chat_id,
                SaiUpdaterWriteScope::Cache,
                |messages| {
                    let mut incoming = GetMessageMessage::new("message-3".to_owned());
                    incoming.text = "realtime".to_owned();
                    incoming.created_at = 3.0;
                    messages.push(incoming);
                    MessageThreadOrder::chronological(messages.clone())
                },
                cx,
            );
        });

        let response = gateway_page(&chat_id, None, false, initial_rows);
        service.update(cx, |service, cx| {
            service.snapshot_latest_messages_page(Some(&response), &chat_id, Some(&fence), cx);
            service.merge_messages_refresh(Some(&response), &chat_id, Some(&fence), cx);
            service.finish_message_seed_request(&chat_id, token, cx);
        });

        let canonical = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        let latest = memory_root(&latest_cache_key, cx);
        assert_eq!(ids_of(&canonical), ["message-1", "message-2", "message-3"]);
        assert_eq!(ids_of(&latest), ["message-1", "message-2", "message-3"]);
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn seed_fence_uses_latest_mirror_boundary_instead_of_treating_it_as_realtime(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-latest-refresh-fence-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let latest_cache_key = MessageServicesStore::latest_messages_cache_key(&chat_id);
        patch_root(
            &service,
            &chat_id,
            None,
            None,
            &[(MessageHistoryDirection::Newer, "around-20", true)],
            vec![row("around-10", 10.0), row("around-20", 20.0)],
            cx,
        );
        replace_latest(
            &latest_cache_key,
            page_root(&chat_id, None, None, vec![row("old-latest-100", 100.0), row("old-latest-200", 200.0)]),
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        let token = service.update(cx, |service, cx| service.begin_message_seed_request(&chat_id, cx));
        let fresh_page =
            gateway_page(&chat_id, None, true, vec![row("fresh-300", 300.0), row("fresh-400", 400.0)]);
        let fence = service.read(cx).message_seed_refresh_fence(&chat_id, token);

        service.update(cx, |service, cx| {
            service.snapshot_latest_messages_page(Some(&fresh_page), &chat_id, fence.as_ref(), cx);
        });

        let latest = memory_root(&latest_cache_key, cx);
        assert_eq!(ids_of(&latest), ["fresh-300", "fresh-400"]);
        service.update(cx, |service, cx| service.finish_message_seed_request(&chat_id, token, cx));
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn persisted_latest_tail_restores_true_bottom_from_disk_after_around_relaunch(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-persisted-latest-tail-{}", unique_token());
    let latest_cache_key = MessageServicesStore::latest_messages_cache_key(&chat_id);
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        patch_root(
            &service,
            &chat_id,
            Some("around-older"),
            Some(true),
            &[
                (MessageHistoryDirection::Older, "around-older", true),
                (MessageHistoryDirection::Newer, "around-newer", true),
            ],
            vec![row("around-10", 10.0), row("around-20", 20.0)],
            cx,
        );
        let latest_page = gateway_page(
            &chat_id,
            Some("latest-older"),
            true,
            vec![row("latest-100", 100.0), row("latest-200", 200.0)],
        );
        service.update(cx, |service, cx| {
            service.snapshot_latest_messages_page(Some(&latest_page), &chat_id, None, cx);
        });
    });
    flush_and_wait(&chat_id, cx);
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        SaiWsResponseCache::remove(&latest_cache_key, SaiCacheCleanLayer::Memory, cx);

        assert_eq!(SaiWsResponseCache::memory_only(&latest_cache_key, cx), None);
        assert!(service.update(cx, |service, cx| service.install_latest_messages_snapshot(&chat_id, cx)));

        let installed = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        assert_eq!(ids_of(&installed), ["latest-100", "latest-200"]);
        assert!(!lane_state(&service, &chat_id, MessageHistoryDirection::Newer, cx).has_more);
    });
    clean_message_caches(&chat_id, cx);
}

fn seed_around_window(service: &Entity<MessageServicesStore>, chat_id: &str, rows: Vec<Map<String, Value>>, cx: &mut App) {
    patch_root(service, chat_id, None, None, &[(MessageHistoryDirection::Newer, "around-newer", true)], rows, cx);
}

fn deliver_latest_page(service: &Entity<MessageServicesStore>, chat_id: &str, page: &Value, cx: &mut App) {
    service.update(cx, |service, cx| {
        service.snapshot_latest_messages_page(Some(page), chat_id, None, cx);
        service.merge_messages_refresh(Some(page), chat_id, None, cx);
        service.finish_pending_latest_messages_jump(chat_id, cx);
    });
}

fn cancel_jump(service: &Entity<MessageServicesStore>, chat_id: &str, cx: &mut App) {
    service.update(cx, |service, cx| service.cancel_pending_latest_messages_jump(chat_id, cx));
}

#[gpui::test]
fn pending_latest_jump_completes_only_after_fresh_latest_response(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-pending-latest-jump-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        seed_around_window(&service, &chat_id, vec![row("around-10", 10.0), row("around-20", 20.0)], cx);
        assert!(!service.update(cx, |service, cx| service.request_latest_messages_jump(&chat_id, cx)));
        assert_eq!(service.read(cx).latest_jump_completion_revision(&chat_id), 0);

        let latest_page =
            gateway_page(&chat_id, None, false, vec![row("latest-100", 100.0), row("latest-200", 200.0)]);
        deliver_latest_page(&service, &chat_id, &latest_page, cx);

        assert_eq!(service.read(cx).latest_jump_completion_revision(&chat_id), 1);
        let installed = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        assert_eq!(ids_of(&installed), ["latest-100", "latest-200"]);
        cancel_jump(&service, &chat_id, cx);
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn pending_latest_jump_completes_for_fresh_empty_chat(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-empty-latest-jump-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        seed_around_window(&service, &chat_id, vec![row("around-1", 1.0)], cx);
        assert!(!service.update(cx, |service, cx| service.request_latest_messages_jump(&chat_id, cx)));

        let empty_page = gateway_page(&chat_id, None, false, Vec::new());
        deliver_latest_page(&service, &chat_id, &empty_page, cx);

        assert_eq!(service.read(cx).latest_jump_completion_revision(&chat_id), 1);
        assert_eq!(service.update(cx, |service, cx| service.cached_message_count(&chat_id, cx)), 0);
        assert!(!lane_state(&service, &chat_id, MessageHistoryDirection::Newer, cx).has_more);
        cancel_jump(&service, &chat_id, cx);
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn cancelled_latest_jump_does_not_complete_on_late_response(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-cancel-latest-jump-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        seed_around_window(&service, &chat_id, vec![row("around-1", 1.0)], cx);
        assert!(!service.update(cx, |service, cx| service.request_latest_messages_jump(&chat_id, cx)));
        cancel_jump(&service, &chat_id, cx);

        let page = gateway_page(&chat_id, None, false, vec![row("latest-2", 2.0)]);
        deliver_latest_page(&service, &chat_id, &page, cx);

        assert_eq!(service.read(cx).latest_jump_completion_revision(&chat_id), 0);
        cancel_jump(&service, &chat_id, cx);
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn refresh_without_overlap_replaces_server_history_without_creating_false_gap(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-refresh-no-overlap-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        patch_root(
            &service,
            &chat_id,
            Some("cached-older-cursor"),
            Some(true),
            &[
                (MessageHistoryDirection::Older, "cached-older-cursor", true),
                (MessageHistoryDirection::Newer, "cached-newer-cursor", true),
            ],
            vec![row("cached-1", 1.0), row("cached-2", 2.0), temp_row("temp_3000_local", Some("pending"), 102.0)],
            cx,
        );

        let page = gateway_page(
            &chat_id,
            Some("fresh-older-cursor"),
            true,
            vec![row("fresh-100", 100.0), row("fresh-101", 101.0)],
        );

        service.update(cx, |service, cx| service.merge_messages_refresh(Some(&page), &chat_id, None, cx));

        let root = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        let ids = ids_of(&root);
        assert_eq!(ids, ["fresh-100", "fresh-101", "temp_3000_local"]);
        assert!(!ids.iter().any(|id| id == "cached-1"));
        assert!(!ids.iter().any(|id| id == "cached-2"));
        assert_eq!(
            lane_state(&service, &chat_id, MessageHistoryDirection::Older, cx),
            page_state("fresh-older-cursor", true)
        );
        assert_eq!(lane_state(&service, &chat_id, MessageHistoryDirection::Newer, cx), page_state("fresh-101", false));
        assert_eq!(string_of(&root, key::RELATIVE_ID), Some("fresh-older-cursor"));
        assert_eq!(root.get(key::IS_HAVE_MORE).and_then(Value::as_bool), Some(true));
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn typed_pagination_appends_without_replacing_loaded_history(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-page-merge-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let cache_key = MessageServicesStore::messages_cache_key(&chat_id);
        SaiWsResponseCache::install_optimistic_root(
            Value::Object(page_root(
                &chat_id,
                Some("cursor-1"),
                Some(true),
                vec![content_row("message-2", "newer", 2.0)],
            )),
            &cache_key,
            true,
            true,
            SaiWsDiskCachePolicy::PreserveExisting,
            false,
            cx,
        )
        .expect("the root installs");

        let page = gateway_page(&chat_id, Some("cursor-2"), false, vec![content_row("message-1", "older", 1.0)]);

        service.update(cx, |service, cx| {
            service.merge_messages_page(Some(&page), &chat_id, MessageHistoryDirection::Older, None, None, cx);
        });

        let root = canonical_root(&cache_key, cx);
        assert_eq!(ids_of(&root), ["message-1", "message-2"]);
        assert_eq!(string_of(&root, key::RELATIVE_ID), Some("cursor-2"));
        assert_eq!(root.get(key::IS_HAVE_MORE).and_then(Value::as_bool), Some(false));
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn newer_page_advances_only_its_lane_and_keeps_history_chronological(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-newer-page-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        patch_root(
            &service,
            &chat_id,
            Some("older-cursor"),
            Some(true),
            &[
                (MessageHistoryDirection::Older, "older-cursor", true),
                (MessageHistoryDirection::Newer, "newer-cursor", true),
            ],
            vec![content_row("message-2", "second", 2.0), content_row("message-3", "stale", 3.0)],
            cx,
        );

        let page = gateway_page(
            &chat_id,
            Some("newer-cursor-2"),
            true,
            vec![content_row("message-3", "fresh", 3.0), content_row("message-4", "fourth", 4.0)],
        );

        service.update(cx, |service, cx| {
            service.merge_messages_page(
                Some(&page),
                &chat_id,
                MessageHistoryDirection::Newer,
                Some("newer-cursor"),
                None,
                cx,
            );
        });

        let root = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        let rows = rows_of(&root);
        assert_eq!(ids_of(&root), ["message-2", "message-3", "message-4"]);
        assert_eq!(
            rows.iter()
                .find(|row| string_of(row, key::ID) == Some("message-3"))
                .and_then(|row| string_of(row, key::CONTENT)),
            Some("fresh")
        );
        assert_eq!(lane_state(&service, &chat_id, MessageHistoryDirection::Older, cx), page_state("older-cursor", true));
        assert_eq!(
            lane_state(&service, &chat_id, MessageHistoryDirection::Newer, cx),
            page_state("newer-cursor-2", true)
        );
        assert_eq!(string_of(&root, key::RELATIVE_ID), Some("older-cursor"));
        assert_eq!(root.get(key::IS_HAVE_MORE).and_then(Value::as_bool), Some(true));
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn non_advancing_message_cursor_closes_only_requested_lane(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-stale-message-cursor-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        patch_root(
            &service,
            &chat_id,
            Some("older-cursor"),
            Some(true),
            &[
                (MessageHistoryDirection::Older, "older-cursor", true),
                (MessageHistoryDirection::Newer, "newer-cursor", true),
            ],
            vec![row("message-2", 2.0)],
            cx,
        );
        let page = gateway_page(&chat_id, Some("newer-cursor"), true, Vec::new());

        service.update(cx, |service, cx| {
            service.merge_messages_page(
                Some(&page),
                &chat_id,
                MessageHistoryDirection::Newer,
                Some("newer-cursor"),
                None,
                cx,
            );
        });

        assert!(lane_state(&service, &chat_id, MessageHistoryDirection::Older, cx).has_more);
        assert!(!lane_state(&service, &chat_id, MessageHistoryDirection::Newer, cx).has_more);
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn stale_directional_response_cannot_overwrite_advanced_lane(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-stale-message-page-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        patch_root(
            &service,
            &chat_id,
            Some("current-cursor"),
            Some(true),
            &[(MessageHistoryDirection::Older, "current-cursor", true)],
            vec![row("message-2", 2.0)],
            cx,
        );
        let stale_page = gateway_page(&chat_id, Some("stale-next-cursor"), true, vec![row("message-1", 1.0)]);

        service.update(cx, |service, cx| {
            service.merge_messages_page(
                Some(&stale_page),
                &chat_id,
                MessageHistoryDirection::Older,
                Some("stale-request-cursor"),
                None,
                cx,
            );
        });

        let root = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        assert_eq!(ids_of(&root), ["message-2"]);
        assert_eq!(
            lane_state(&service, &chat_id, MessageHistoryDirection::Older, cx),
            page_state("current-cursor", true)
        );
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn large_directional_page_merges_in_stable_order_and_replaces_overlap(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-large-message-page-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let rows: Vec<Map<String, Value>> = (0..10_000)
            .map(|index| content_row(&format!("message-{index:05}"), "cached", f64::from(index / 2 + 1)))
            .collect();
        patch_root(&service, &chat_id, Some("large-cursor"), Some(true), &[], rows, cx);
        let page = gateway_page(
            &chat_id,
            Some("large-cursor-2"),
            false,
            vec![
                content_row("message-05000", "fresh", 2_501.0),
                content_row("message-before-all", "older", 0.5),
                content_row("message-10000", "newer", 5_001.0),
            ],
        );

        service.update(cx, |service, cx| {
            service.merge_messages_page(Some(&page), &chat_id, MessageHistoryDirection::Older, None, None, cx);
        });

        let root = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        let merged = rows_of(&root);
        assert_eq!(merged.len(), 10_002);
        assert_eq!(merged.first().and_then(|row| string_of(row, key::ID)), Some("message-before-all"));
        assert_eq!(merged.last().and_then(|row| string_of(row, key::ID)), Some("message-10000"));
        assert_eq!(string_of(&merged[1], key::ID), Some("message-00001"));
        assert_eq!(string_of(&merged[2], key::ID), Some("message-00000"));
        assert_eq!(
            merged
                .iter()
                .find(|row| string_of(row, key::ID) == Some("message-05000"))
                .and_then(|row| string_of(row, key::CONTENT)),
            Some("fresh")
        );
    });
    clean_message_caches(&chat_id, cx);
}

#[gpui::test]
fn around_page_seeds_both_lanes_and_presentation_anchor(cx: &mut TestAppContext) {
    let _serial = SERIAL.blocking_lock();
    let chat_id = format!("test-message-around-{}", unique_token());
    cx.update(|cx| {
        let service = MessageServicesStore::shared(cx);
        let page = Value::Object(object(vec![
            (key::SUCCESS, Value::Bool(true)),
            (key::CHAT_ID, Value::from(chat_id.as_str())),
            (key::ANCHOR_MESSAGE_ID, Value::from("message-2")),
            (key::ANCHOR_RELATIVE_ID, Value::from("anchor-cursor")),
            (key::OLDER_RELATIVE_ID, Value::from("older-cursor")),
            (key::HAS_OLDER, Value::Bool(true)),
            (key::NEWER_RELATIVE_ID, Value::from("newer-cursor")),
            (key::HAS_NEWER, Value::Bool(true)),
            (key::MESSAGES, message_values(vec![row("message-3", 3.0), row("message-2", 2.0), row("message-1", 1.0)])),
        ]));
        assert!(MessagePayloadCodec::decode::<GetMessagesAroundGatewayDTO>(Some(&page)).is_some());

        assert!(service.update(cx, |service, cx| {
            service.install_messages_around_page(Some(&page), &chat_id, "message-2", cx)
        }));

        let root = canonical_root(&MessageServicesStore::messages_cache_key(&chat_id), cx);
        assert_eq!(ids_of(&root), ["message-1", "message-2", "message-3"]);
        assert_eq!(lane_state(&service, &chat_id, MessageHistoryDirection::Older, cx), page_state("older-cursor", true));
        assert_eq!(lane_state(&service, &chat_id, MessageHistoryDirection::Newer, cx), page_state("newer-cursor", true));
        let slice = service.update(cx, |service, cx| service.message_presentation_slice(&chat_id, cx));
        assert_eq!(slice.state.anchor_id.as_deref(), Some("message-2"));
        ChatScrollPositionStore::clear(&chat_id, cx);
    });
    clean_message_caches(&chat_id, cx);
}
