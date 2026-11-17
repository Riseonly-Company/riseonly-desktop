use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui::TestAppContext;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_core::rise_identifiers::RiseRequestID;
use rise_engine::rise_core::rise_secure_key_store::{RiseEngineKeyStore, RiseEngineKeyStoreFuture};
use rise_engine::rise_wire::rise_wire::{RiseWireAccountContext, RiseWireFrameSender};
use rise_engine::{
    RiseEngine, RiseEngineConfiguration, RiseEngineConfigurationArgs, RiseJSONValue, RiseWireInboundDisposition,
    RiseWireSocketSession,
};
use rise_platform::file_manager::FileManager;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::Mutex;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepositoryPage;
use crate::modules::chat::engine::chats::rise_chat_repository_models::RiseChatWireCodec;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationPresentation,
};
use crate::modules::chat::engine::messages::rise_message_scope::{
    RiseMessageAnchor, RiseMessageScope, RiseMessageWindowConfiguration,
};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomain;

const WAIT_ATTEMPTS: usize = 200;
const WAIT_STEP: Duration = Duration::from_millis(10);
const OWN_PROCESS_MARKER: &str = "RISE_REALTIME_SEED_OWN_PROCESS";

struct RealtimeIngestKeyStore;

impl RiseEngineKeyStore for RealtimeIngestKeyStore {
    fn key<'a>(&'a self, _account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Option<Vec<u8>>> {
        Box::pin(async move { Ok(Some(vec![0x63; 32])) })
    }

    fn create_key_if_absent<'a>(&'a self, _account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Vec<u8>> {
        Box::pin(async move { Ok(vec![0x63; 32]) })
    }

    fn delete_key<'a>(&'a self, _account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, ()> {
        Box::pin(async move { Ok(()) })
    }
}

#[derive(Debug, thiserror::Error)]
enum RealtimeIngestError {
    #[error("the request frame has no usable id")]
    InvalidRequest,
}

struct RealtimeIngestSender {
    outgoing: UnboundedSender<(Vec<u8>, RiseWireSocketSession)>,
    incoming: Mutex<UnboundedReceiver<(Vec<u8>, RiseWireSocketSession)>>,
}

impl RealtimeIngestSender {
    fn new() -> Arc<Self> {
        let (outgoing, incoming) = unbounded_channel();
        Arc::new(Self { outgoing, incoming: Mutex::new(incoming) })
    }

    async fn next(&self) -> (Vec<u8>, RiseWireSocketSession) {
        self.incoming.lock().await.recv().await.expect("the sender owns its own channel")
    }
}

impl RiseWireFrameSender for RealtimeIngestSender {
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

fn json_string(value: &str) -> RiseJSONValue {
    RiseJSONValue::String(value.to_owned())
}

fn json_map<const N: usize>(entries: [(&str, RiseJSONValue); N]) -> BTreeMap<String, RiseJSONValue> {
    entries.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

fn realtime_ingest_request_id(request: &[u8]) -> anyhow::Result<RiseRequestID> {
    let root = RiseJSONValue::decode(request)?;
    let raw_value = match root.subscript_key("id") {
        Some(RiseJSONValue::Integer(value)) if *value > 0 => *value as u64,
        Some(RiseJSONValue::UnsignedInteger(value)) if *value > 0 => *value,
        _ => return Err(RealtimeIngestError::InvalidRequest.into()),
    };
    Ok(RiseRequestID::new(raw_value).ok_or(RealtimeIngestError::InvalidRequest)?)
}

fn realtime_ingest_response(request: &[u8], payload: RiseJSONValue) -> anyhow::Result<Vec<u8>> {
    let request_id = realtime_ingest_request_id(request)?;
    Ok(RiseJSONValue::Object(json_map([
        ("request_id", json_string(&request_id.raw_value().to_string())),
        ("data", payload),
    ]))
    .data(false)?)
}

fn realtime_ingest_message(id: &str, chat_id: &str, sender_id: &str, created_at: f64) -> BTreeMap<String, RiseJSONValue> {
    json_map([
        ("id", json_string(id)),
        ("server_id", json_string(id)),
        ("chat_id", json_string(chat_id)),
        ("sender_id", json_string(sender_id)),
        ("content", json_string(&format!("body-{id}"))),
        ("content_type", json_string("text")),
        ("created_at", RiseJSONValue::Number(created_at)),
    ])
}

fn peer_messages(chat_id: &str, ids: &[(&str, f64)]) -> RiseJSONValue {
    RiseJSONValue::Array(
        ids.iter()
            .map(|(id, created_at)| RiseJSONValue::Object(realtime_ingest_message(id, chat_id, "peer", *created_at)))
            .collect(),
    )
}

fn seed_payload(chat_id: &str, ids: &[(&str, f64)]) -> RiseJSONValue {
    RiseJSONValue::Object(json_map([
        ("chat_id", json_string(chat_id)),
        ("is_have_more", RiseJSONValue::Bool(false)),
        ("messages", peer_messages(chat_id, ids)),
    ]))
}

fn created_push(id: &str, chat_id: &str, created_at: f64, timestamp_ms: i64) -> anyhow::Result<Vec<u8>> {
    let mut data = realtime_ingest_message(id, chat_id, "peer", created_at);
    data.insert("message_id".to_owned(), json_string(id));
    Ok(RiseJSONValue::Object(json_map([
        ("type", json_string("message_created")),
        ("data", RiseJSONValue::Object(data)),
        ("timestamp_ms", RiseJSONValue::Integer(timestamp_ms)),
    ]))
    .data(false)?)
}

fn method_of(request: &[u8]) -> anyhow::Result<String> {
    let root = RiseJSONValue::decode(request)?;
    Ok(root.subscript_key("method").and_then(|method| method.string_value()).unwrap_or("none").to_owned())
}

async fn on_runtime<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    RiseAccountRuntime::current().spawn(future).await.expect("the account runtime stays alive")
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

fn presented_ids(scope: &RiseMessageScope, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        let presentation = RiseConversationPresentation::shared(cx);
        presentation.read(cx).messages(scope).iter().map(|message| message.id.clone()).collect()
    })
}

async fn wait_for_presented(scope: &RiseMessageScope, expected: &[&str], cx: &mut TestAppContext) -> Vec<String> {
    let mut ids = Vec::new();
    for _ in 0..WAIT_ATTEMPTS {
        cx.run_until_parked();
        ids = presented_ids(scope, cx);
        if ids == expected {
            break;
        }
        on_runtime(tokio::time::sleep(WAIT_STEP)).await;
    }
    ids
}

async fn stored_ids(domain: &Arc<RiseConversationDomain>, scope: &RiseMessageScope) -> anyhow::Result<Vec<String>> {
    let domain = domain.clone();
    let scope = scope.clone();
    let snapshot = on_runtime(async move {
        domain.messages.snapshot(&scope, RiseMessageAnchor::Latest, RiseMessageWindowConfiguration::presentation()).await
    })
    .await?;
    Ok(snapshot.rows.into_iter().filter_map(|row| row.wire.id).collect())
}

async fn wait_for_stored(
    domain: &Arc<RiseConversationDomain>,
    scope: &RiseMessageScope,
    id: &str,
) -> anyhow::Result<Vec<String>> {
    let mut ids = Vec::new();
    for _ in 0..WAIT_ATTEMPTS {
        ids = stored_ids(domain, scope).await?;
        if ids.iter().any(|candidate| candidate == id) {
            break;
        }
        on_runtime(tokio::time::sleep(WAIT_STEP)).await;
    }
    Ok(ids)
}

async fn realtime_flow(
    domain: &Arc<RiseConversationDomain>,
    engine: &Arc<RiseEngine>,
    sender: &Arc<RealtimeIngestSender>,
    cx: &mut TestAppContext,
) -> anyhow::Result<()> {
    let session = on_runtime({
        let domain = domain.clone();
        let engine = engine.clone();
        async move {
            for handler in domain.messages.realtime_push_handlers() {
                engine.register_push_handler(handler).await?;
            }
            engine.start().await?;
            let session = RiseWireSocketSession::new(engine.account.clone(), 1);
            engine.socket_session_started(session.clone()).await?;
            engine.socket_connected(session.clone()).await?;
            domain.activate().await?;
            Ok::<_, anyhow::Error>(session)
        }
    })
    .await?;

    let chat_id = format!("chat-{}", Uuid::new_v4().uuid_string());
    let scope = RiseMessageScope::new(chat_id.clone(), None, None);
    let owner_id = Uuid::new_v4();
    on_runtime({
        let domain = domain.clone();
        let engine = engine.clone();
        let chat_id = chat_id.clone();
        async move {
            let chat = RiseChatWireCodec::minimal_wire(&chat_id)?;
            let chat_version = domain.chats.next_version();
            let chat_rows =
                domain.chats.encode_rows(std::slice::from_ref(&chat), RiseChatListKind::Inbox, chat_version).await?;
            let chat_batch = domain.chats.make_page_batch(
                &chat_rows,
                &[],
                RiseChatListKind::Inbox,
                &RiseChatRepositoryPage { rows: vec![chat], cursor: None, has_more: false, aggregates: None },
                chat_version,
                domain.chats.next_version(),
                true,
            )?;
            engine.write(chat_batch).await?;
            Ok::<_, anyhow::Error>(())
        }
    })
    .await?;

    let open = RiseAccountRuntime::current().spawn({
        let domain = domain.clone();
        let scope = scope.clone();
        async move { domain.load_initial_messages(&scope, None, None, None, Some(owner_id), Some(1)).await }
    });
    let (seed_request, seed_session) = sender.next().await;
    assert_eq!(seed_session, session);
    assert_eq!(method_of(&seed_request)?, "get_messages");
    let seed_response = realtime_ingest_response(
        &seed_request,
        seed_payload(&chat_id, &[("message-1", 1.0), ("message-2", 2.0)]),
    )?;
    assert_eq!(
        accept_on_runtime(engine, seed_response, &session).await?,
        RiseWireInboundDisposition::ConsumedResponse
    );
    open.await?;

    assert_eq!(wait_for_presented(&scope, &["message-1", "message-2"], cx).await, vec!["message-1", "message-2"]);

    on_runtime({
        let domain = domain.clone();
        let storage_key = scope.storage_key();
        async move { domain.release_messages_storage_key(&storage_key, Some(owner_id), Some(2)).await }
    })
    .await;

    let disposition = accept_on_runtime(engine, created_push("message-3", &chat_id, 3.0, 3_000)?, &session).await?;
    assert_eq!(disposition, RiseWireInboundDisposition::Event, "push was not routed to RiseSync");

    let stored = wait_for_stored(domain, &scope, "message-3").await?;
    assert_eq!(
        stored,
        vec!["message-1", "message-2", "message-3"],
        "STAGE 2 store timeline after realtime push: {stored:?}"
    );
    let (last_message_id, general) = on_runtime({
        let domain = domain.clone();
        let chat_id = chat_id.clone();
        async move {
            let last_message_id = domain
                .chats
                .chat(&chat_id)
                .await?
                .and_then(|record| record.wire.last_message)
                .and_then(|preview| preview.id);
            let general = domain
                .messages
                .snapshot_anchor(
                    &RiseMessageScope::new(chat_id.clone(), Some(String::new()), None),
                    RiseMessageAnchor::Latest,
                    RiseMessageWindowConfiguration::presentation(),
                    false,
                )
                .await?;
            Ok::<_, anyhow::Error>((last_message_id, general))
        }
    })
    .await?;
    assert_eq!(last_message_id.as_deref(), Some("message-3"));
    assert!(
        general.local_ids.iter().any(|id| id == "message-3"),
        "General must receive the event even when its history has never been opened"
    );

    let reopen = RiseAccountRuntime::current().spawn({
        let domain = domain.clone();
        let scope = scope.clone();
        async move {
            let storage_key = scope.storage_key();
            domain
                .load_initial_messages(
                    &scope,
                    Some("message-2"),
                    None,
                    Some(&storage_key),
                    Some(owner_id),
                    Some(3),
                )
                .await
        }
    });
    let (reopen_request, _) = sender.next().await;
    let reopen_method = method_of(&reopen_request)?;
    assert_eq!(reopen_method, "get_messages_around", "STAGE 3 reopen route: {reopen_method}");
    assert_eq!(
        wait_for_presented(&scope, &["message-1", "message-2", "message-3"], cx).await,
        vec!["message-1", "message-2", "message-3"],
        "The cached event must be visible BEFORE the reopen RPC is answered"
    );
    let reopen_payload = if reopen_method == "get_messages_around" {
        RiseJSONValue::Object(json_map([
            ("chat_id", json_string(&chat_id)),
            ("anchor_message_id", json_string("message-2")),
            ("has_older", RiseJSONValue::Bool(false)),
            ("has_newer", RiseJSONValue::Bool(false)),
            ("messages", peer_messages(&chat_id, &[("message-1", 1.0), ("message-2", 2.0)])),
        ]))
    } else {
        seed_payload(&chat_id, &[])
    };
    accept_on_runtime(engine, realtime_ingest_response(&reopen_request, reopen_payload)?, &session).await?;
    reopen.await?;

    let reopened = wait_for_presented(&scope, &["message-1", "message-2", "message-3"], cx).await;
    assert_eq!(
        reopened,
        vec!["message-1", "message-2", "message-3"],
        "STAGE 3 presentation after reopen: {reopened:?}"
    );

    on_runtime({
        let domain = domain.clone();
        let storage_key = scope.storage_key();
        async move { domain.release_messages_storage_key(&storage_key, Some(owner_id), Some(4)).await }
    })
    .await;
    accept_on_runtime(engine, created_push("message-4", &chat_id, 4.0, 4_000)?, &session).await?;
    wait_for_stored(domain, &scope, "message-4").await?;
    let storage_key = scope.storage_key();
    cx.update(|cx| ChatScrollPositionStore::clear(&storage_key, cx));
    let stale_reopen = RiseAccountRuntime::current().spawn({
        let domain = domain.clone();
        let scope = scope.clone();
        async move { domain.load_initial_messages(&scope, None, None, None, Some(owner_id), Some(5)).await }
    });
    let (stale_request, _) = sender.next().await;
    assert_eq!(method_of(&stale_request)?, "get_messages");
    assert_eq!(
        wait_for_presented(&scope, &["message-1", "message-2", "message-3", "message-4"], cx).await,
        vec!["message-1", "message-2", "message-3", "message-4"],
        "Opening at latest must publish realtime cache before the network response"
    );
    let stale_response = realtime_ingest_response(
        &stale_request,
        seed_payload(&chat_id, &[("message-1", 1.0), ("message-2", 2.0), ("message-3", 3.0)]),
    )?;
    accept_on_runtime(engine, stale_response, &session).await?;
    stale_reopen.await?;

    let after_stale_seed = wait_for_stored(domain, &scope, "message-4").await?;
    assert!(
        after_stale_seed.iter().any(|id| id == "message-4"),
        "STAGE 4 timeline after a seed page that omits it: {after_stale_seed:?}"
    );
    Ok(())
}

fn ran_in_own_process() -> bool {
    if std::env::var_os(OWN_PROCESS_MARKER).is_some() {
        return false;
    }
    let module = module_path!().split_once("::").map_or(module_path!(), |(_, module)| module);
    let test_name = format!("{module}::realtime_created_reaches_store_and_reopened_presentation");
    let status = Command::new(std::env::current_exe().expect("the test binary has a path"))
        .args(["--exact", test_name.as_str(), "--test-threads=1", "--nocapture"])
        .env(OWN_PROCESS_MARKER, "1")
        .status()
        .expect("the test binary starts");
    assert!(status.success(), "the isolated run of {test_name} failed");
    true
}

#[gpui::test]
async fn realtime_created_reaches_store_and_reopened_presentation(cx: &mut TestAppContext) {
    if ran_in_own_process() {
        return;
    }
    cx.executor().allow_parking();
    cx.update(|cx| {
        RiseEngineMainHop::shared(cx);
    });
    let _restore_scope = RestoreAccountScope(SaiAccountCacheScope::active_user_id());
    let account_id = format!("realtime-ingest-{}", Uuid::new_v4().uuid_string());
    SaiAccountCacheScope::activate(&account_id);
    let account_epoch = SaiAccountCacheScope::active_epoch().expect("activation sets an epoch");
    let sender = RealtimeIngestSender::new();
    let root = std::env::temp_dir().join(format!("rise-realtime-ingest-{}", Uuid::new_v4().uuid_string()));
    let _remove_root = RemoveOnDrop(root.clone());

    let engine = on_runtime({
        let sender = sender.clone();
        async move {
            RiseEngine::open(RiseEngineConfiguration::new(
                root,
                RiseWireAccountContext::new(account_id, account_epoch),
                sender,
                RiseEngineConfigurationArgs {
                    key_store: Arc::new(RealtimeIngestKeyStore),
                    ..RiseEngineConfigurationArgs::default()
                },
            ))
            .await
        }
    })
    .await
    .expect("the engine opens");
    let domain = RiseConversationDomain::new(engine.clone());

    let result = realtime_flow(&domain, &engine, &sender, cx).await;

    on_runtime({
        let domain = domain.clone();
        async move { domain.shutdown().await }
    })
    .await;
    let closed = on_runtime({
        let engine = engine.clone();
        async move { engine.close().await }
    })
    .await;
    result.expect("the realtime flow finishes");
    closed.expect("the engine closes");
}
