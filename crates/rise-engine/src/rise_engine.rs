use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use parking_lot::{Mutex, RwLock};
use rise_canon::RiseWireRequestMetadata;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::FileManager;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::oneshot;

use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_core::rise_identifiers::{RiseRequestID, RiseRequestIDAllocator};
use crate::rise_core::rise_secure_key_store::{
    RiseEngineKeyStore, RiseEngineKeyStoreFuture, RiseKeychainKeyStore,
};
use crate::rise_engine_observability::{RiseEngineCallContext, RiseEngineObservability};
use crate::rise_store::rise_store::{RiseStore, RiseStoreConfiguration, RiseSystemClock};
use crate::rise_store::rise_store_cipher::RiseAESGCMStoreCipher;
use crate::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityViewPosition, RiseEntityViewState, RiseEntityWindowQuery,
    RiseEntityWindowResult, RiseStoreEntity,
};
use crate::rise_store::rise_store_models::{
    RiseCollectionAnchor, RiseDurableOperation, RiseDurableTerminalResolution, RiseOrderedItem,
    RiseStoreCommit, RiseStoreError, RiseStoreLimits, RiseStoreNamespace, RiseStoreObservation,
    RiseStoreSnapshot, RiseStoreSnapshotRequest, RiseStoreWriteBatch, RiseStoredValue,
};
use crate::rise_sync::rise_sync::{
    RiseDurableLifecycleEvents, RiseDurableSubmissionReceipt, RiseSync, RiseSyncDurableEndpoint,
    RiseSyncDurableHandler, RiseSyncHealthEvents, RiseSyncPushHandler, RiseSyncSubmitOptions,
};
use crate::rise_wire::rise_json_value::RiseJSONValue;
use crate::rise_wire::rise_wire::{
    RiseWire, RiseWireAccountContext, RiseWireConfiguration, RiseWireError, RiseWireFrameSender,
    RiseWireHttpSender, RiseWireInboundDisposition, RiseWireSocketSession,
};
use crate::rise_wire::rise_wire_contracts::{RiseWireMethod, RiseWireMethodDescriptor};

const ENGINE_DATABASE_FILE_NAME: &str = "engine.sqlite";
const ENGINE_DIRECTORY_NAME: &str = "RiseEngine";

pub struct RiseEngineConfiguration {
    pub base_directory_url: PathBuf,
    pub account: RiseWireAccountContext,
    pub sender: Arc<dyn RiseWireFrameSender>,
    pub http_sender: Option<Arc<dyn RiseWireHttpSender>>,
    pub key_store: Arc<dyn RiseEngineKeyStore>,
    pub store_limits: RiseStoreLimits,
    pub wire_configuration: RiseWireConfiguration,
}

pub struct RiseEngineConfigurationArgs {
    pub http_sender: Option<Arc<dyn RiseWireHttpSender>>,
    pub key_store: Arc<dyn RiseEngineKeyStore>,
    pub store_limits: RiseStoreLimits,
    pub wire_configuration: RiseWireConfiguration,
}

impl Default for RiseEngineConfigurationArgs {
    fn default() -> Self {
        Self {
            http_sender: None,
            key_store: Arc::new(RiseKeychainKeyStore::shared()),
            store_limits: RiseStoreLimits::production(),
            wire_configuration: RiseWireConfiguration::production(),
        }
    }
}

impl RiseEngineConfiguration {
    pub fn new(
        base_directory_url: PathBuf,
        account: RiseWireAccountContext,
        sender: Arc<dyn RiseWireFrameSender>,
        args: RiseEngineConfigurationArgs,
    ) -> Self {
        Self {
            base_directory_url,
            account,
            sender,
            http_sender: args.http_sender,
            key_store: args.key_store,
            store_limits: args.store_limits,
            wire_configuration: args.wire_configuration,
        }
    }
}

impl RiseEngineKeyStore for &'static RiseKeychainKeyStore {
    fn key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, Option<Vec<u8>>> {
        RiseEngineKeyStore::key(*self, account_id)
    }

    fn create_key_if_absent<'a>(
        &'a self,
        account_id: &'a str,
    ) -> RiseEngineKeyStoreFuture<'a, Vec<u8>> {
        RiseEngineKeyStore::create_key_if_absent(*self, account_id)
    }

    fn delete_key<'a>(&'a self, account_id: &'a str) -> RiseEngineKeyStoreFuture<'a, ()> {
        RiseEngineKeyStore::delete_key(*self, account_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum RiseEngineError {
    #[error("missingKeyForExistingDatabase")]
    MissingKeyForExistingDatabase,
    #[error("accountAlreadyOpen")]
    AccountAlreadyOpen,
    #[error("accountInUse")]
    AccountInUse,
    #[error("alreadyClosed")]
    AlreadyClosed,
}

/// checked before every outbound rpc on whatever task calls
/// an error refuses the call unsent and the caller sees it
/// installed only when a body must never reach the wire
pub type RiseEngineOutboundGuard = Arc<
    dyn Fn(&RiseWireMethodDescriptor, &RiseJSONValue) -> Option<anyhow::Error> + Send + Sync,
>;

/// asked when a socket read finds no ready socket, host wakes its transport
/// and says whether a handshake is coming, false fails the read at once
/// none means reads keep waiting
pub type RiseEngineSocketDemand = Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>;

static OUTBOUND_GUARD: LazyLock<RwLock<Option<RiseEngineOutboundGuard>>> =
    LazyLock::new(|| RwLock::new(None));
static SOCKET_DEMAND: LazyLock<RwLock<Option<RiseEngineSocketDemand>>> =
    LazyLock::new(|| RwLock::new(None));

pub struct RiseEngine {
    pub account: RiseWireAccountContext,
    wire: Arc<RiseWire>,
    sync: Arc<RiseSync>,
    store: Arc<RiseStore>,
    ownership: Arc<RiseEngineStoreOwnership>,
    lifecycle: Arc<RiseEngineLifecycle>,
}

impl Drop for RiseEngine {
    fn drop(&mut self) {
        let engine_sync = self.sync.clone();
        let engine_wire = self.wire.clone();
        let engine_store = self.store.clone();
        let engine_ownership = self.ownership.clone();
        RiseAccountRuntime::current().spawn(async move {
            engine_sync.shutdown().await;
            engine_wire.shutdown().await;
            let _ = engine_store.close().await;
            engine_ownership.release();
        });
    }
}

impl RiseEngine {
    pub fn outbound_guard() -> Option<RiseEngineOutboundGuard> {
        OUTBOUND_GUARD.read().clone()
    }

    pub fn set_outbound_guard(guard: Option<RiseEngineOutboundGuard>) {
        *OUTBOUND_GUARD.write() = guard;
    }

    pub fn socket_demand() -> Option<RiseEngineSocketDemand> {
        SOCKET_DEMAND.read().clone()
    }

    pub fn set_socket_demand(demand: Option<RiseEngineSocketDemand>) {
        *SOCKET_DEMAND.write() = demand;
    }

    pub async fn open(configuration: RiseEngineConfiguration) -> anyhow::Result<Arc<RiseEngine>> {
        let account_id = configuration.account.account_id.clone();
        let directory = Self::account_directory(&configuration.base_directory_url, &account_id);
        let database_url = directory.join(ENGINE_DATABASE_FILE_NAME);
        let Some(ownership) = RiseEngineStoreRegistry::shared().claim(&database_url, &account_id)
        else {
            return Err(RiseEngineError::AccountAlreadyOpen.into());
        };
        match Self::open_claimed(configuration, database_url, ownership.clone()).await {
            Ok(engine) => Ok(engine),
            Err(error) => {
                ownership.release();
                Err(error)
            }
        }
    }

    async fn open_claimed(
        configuration: RiseEngineConfiguration,
        database_url: PathBuf,
        ownership: Arc<RiseEngineStoreOwnership>,
    ) -> anyhow::Result<Arc<RiseEngine>> {
        let account_id = configuration.account.account_id.clone();
        let database_exists = FileManager::default().file_exists(&database_url);
        let existing_key = configuration.key_store.key(&account_id).await?;
        if database_exists && existing_key.is_none() {
            return Err(RiseEngineError::MissingKeyForExistingDatabase.into());
        }
        let key = match existing_key {
            Some(existing_key) => existing_key,
            None => configuration.key_store.create_key_if_absent(&account_id).await?,
        };
        let cipher = RiseAESGCMStoreCipher::new(&key)?;
        let store_configuration = RiseStoreConfiguration::new(
            database_url,
            account_id.clone(),
            Arc::new(cipher),
            Arc::new(RiseSystemClock),
            configuration.store_limits,
            true,
        );
        let store = RiseStore::open(store_configuration).await?;
        let allocator = RiseRequestIDAllocator::shared();
        let wire = RiseWire::new(
            configuration.account.clone(),
            configuration.sender,
            configuration.http_sender,
            allocator,
            configuration.wire_configuration,
        );
        let sync = RiseSync::new(store.clone(), wire.clone(), allocator);
        Ok(Arc::new(RiseEngine {
            account: configuration.account,
            wire,
            sync,
            store,
            ownership,
            lifecycle: Arc::new(RiseEngineLifecycle::new()),
        }))
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        self.with_open_operation(self.sync.start()).await
    }

    pub async fn close(&self) -> anyhow::Result<()> {
        let sync = self.sync.clone();
        let wire = self.wire.clone();
        let store = self.store.clone();
        let ownership = self.ownership.clone();
        self.lifecycle
            .close(
                async move {
                    sync.shutdown().await;
                    wire.shutdown().await;
                }
                .boxed(),
                async move {
                    store.close().await?;
                    ownership.release();
                    Ok::<(), anyhow::Error>(())
                }
                .boxed(),
            )
            .await
    }

    pub async fn call<Method: RiseWireMethod>(
        &self,
        request: &Method::Request,
        request_id: Option<RiseRequestID>,
        metadata: RiseWireRequestMetadata,
    ) -> anyhow::Result<Method::Response> {
        let descriptor = Method::descriptor();
        let context = self.call_context(&descriptor);
        RiseEngineObservability::begin(&context);
        let mut trace = RiseEngineCallTrace { context: Some(context.clone()) };
        let result = self
            .with_open_operation(RiseEngineObservability::with_call_context(
                context,
                self.wire.call::<Method>(request, request_id, metadata),
            ))
            .await;
        trace.finish(result.as_ref().err());
        result
    }

    pub async fn call_descriptor(
        &self,
        descriptor: &RiseWireMethodDescriptor,
        request: &RiseJSONValue,
        request_id: Option<RiseRequestID>,
        metadata: RiseWireRequestMetadata,
    ) -> anyhow::Result<RiseJSONValue> {
        let context = self.call_context(descriptor);
        RiseEngineObservability::begin(&context);
        let mut trace = RiseEngineCallTrace { context: Some(context.clone()) };
        let result = self
            .guarded_call_descriptor(context, descriptor, request, request_id, metadata)
            .await;
        trace.finish(result.as_ref().err());
        result
    }

    async fn guarded_call_descriptor(
        &self,
        context: RiseEngineCallContext,
        descriptor: &RiseWireMethodDescriptor,
        request: &RiseJSONValue,
        request_id: Option<RiseRequestID>,
        metadata: RiseWireRequestMetadata,
    ) -> anyhow::Result<RiseJSONValue> {
        if let Some(guard) = Self::outbound_guard()
            && let Some(refusal) = guard(descriptor, request)
        {
            return Err(refusal);
        }
        self.with_open_operation(RiseEngineObservability::with_call_context(
            context,
            self.wire.call_descriptor(descriptor, request, request_id, metadata),
        ))
        .await
    }

    pub async fn call_descriptor_response<Request, Response>(
        &self,
        descriptor: &RiseWireMethodDescriptor,
        request: &Request,
        request_id: Option<RiseRequestID>,
        metadata: RiseWireRequestMetadata,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Sync,
        Response: DeserializeOwned,
    {
        let body = RiseJSONValue::encode(request)?;
        let payload = self.call_descriptor(descriptor, &body, request_id, metadata).await?;
        payload
            .decode_as::<Response>()
            .map_err(|_| RiseWireError::ResponseDecodingFailed.into())
    }

    pub async fn register_push_handler(&self, push_handler: RiseSyncPushHandler) -> anyhow::Result<()> {
        self.with_open_operation(async { self.sync.register_push_handler(push_handler).await.map_err(anyhow::Error::from) })
            .await
    }

    pub async fn register_durable_handler(
        &self,
        durable_handler: RiseSyncDurableHandler,
    ) -> anyhow::Result<()> {
        self.with_open_operation(async {
            self.sync.register_durable_handler(durable_handler).await.map_err(anyhow::Error::from)
        })
        .await
    }

    pub async fn register_durable_endpoint<Endpoint: RiseSyncDurableEndpoint>(
        &self,
    ) -> anyhow::Result<()> {
        self.with_open_operation(async {
            self.sync.register_durable_endpoint::<Endpoint>().await.map_err(anyhow::Error::from)
        })
        .await
    }

    pub async fn on_resync_required<Handler, Handled>(&self, handler: Handler) -> anyhow::Result<()>
    where
        Handler: Fn(String) -> Handled + Send + Sync + 'static,
        Handled: Future<Output = ()> + Send + 'static,
    {
        self.with_open_operation(async {
            self.sync.on_resync_required(handler).await;
            Ok::<(), anyhow::Error>(())
        })
        .await
    }

    pub async fn health_events(&self) -> anyhow::Result<RiseSyncHealthEvents> {
        self.with_open_operation(async { Ok(self.sync.health_events().await) }).await
    }

    pub async fn durable_lifecycle_events(&self) -> anyhow::Result<RiseDurableLifecycleEvents> {
        self.with_open_operation(async { Ok(self.sync.durable_lifecycle_events().await) }).await
    }

    pub async fn apply<Input, Reducer>(
        &self,
        input: Input,
        reducer: Reducer,
    ) -> anyhow::Result<RiseStoreCommit>
    where
        Input: Send + 'static,
        Reducer: FnOnce(Input) -> anyhow::Result<RiseStoreWriteBatch> + Send + 'static,
    {
        self.with_open_operation(self.sync.apply(input, reducer)).await
    }

    pub async fn submit_durable<Endpoint: RiseSyncDurableEndpoint>(
        &self,
        payload: Endpoint::Payload,
        options: RiseSyncSubmitOptions,
    ) -> anyhow::Result<RiseRequestID> {
        let receipt = self.submit_durable_receipt::<Endpoint>(payload, options).await?;
        Ok(receipt.request_id)
    }

    pub async fn submit_durable_receipt<Endpoint: RiseSyncDurableEndpoint>(
        &self,
        payload: Endpoint::Payload,
        options: RiseSyncSubmitOptions,
    ) -> anyhow::Result<RiseDurableSubmissionReceipt> {
        self.with_open_operation(self.sync.submit_durable_receipt::<Endpoint>(payload, options))
            .await
    }

    pub async fn terminal_operations(&self, limit: i64) -> anyhow::Result<Vec<RiseDurableOperation>> {
        self.with_open_operation(self.sync.terminal_operations(limit)).await
    }

    pub async fn durable_operation(
        &self,
        request_id: RiseRequestID,
    ) -> anyhow::Result<Option<RiseDurableOperation>> {
        self.with_open_operation(async { self.store.operation(request_id).await.map_err(anyhow::Error::from) }).await
    }

    pub async fn resolve_terminal_operation(
        &self,
        request_id: RiseRequestID,
        resolution: RiseDurableTerminalResolution,
    ) -> anyhow::Result<()> {
        self.with_open_operation(self.sync.resolve_terminal_operation(request_id, resolution))
            .await
    }

    pub async fn socket_session_started(&self, session: RiseWireSocketSession) -> anyhow::Result<()> {
        self.with_open_operation(async {
            self.wire.socket_session_started(session).await;
            Ok::<(), anyhow::Error>(())
        })
        .await
    }

    pub async fn socket_connected(&self, session: RiseWireSocketSession) -> anyhow::Result<()> {
        self.with_open_operation(async {
            self.wire.socket_connected(session).await;
            Ok::<(), anyhow::Error>(())
        })
        .await
    }

    pub async fn socket_disconnected(
        &self,
        session: RiseWireSocketSession,
        reason: String,
    ) -> anyhow::Result<()> {
        self.with_open_operation(async {
            self.wire.socket_disconnected(session, reason).await;
            Ok::<(), anyhow::Error>(())
        })
        .await
    }

    pub async fn socket_handshake_failed(&self) -> anyhow::Result<()> {
        self.with_open_operation(async {
            self.wire.socket_handshake_failed().await;
            Ok::<(), anyhow::Error>(())
        })
        .await
    }

    pub async fn set_application_active(
        &self,
        active: bool,
        session: Option<RiseWireSocketSession>,
    ) -> anyhow::Result<()> {
        self.with_open_operation(async {
            self.wire.set_application_active(active, session).await;
            Ok::<(), anyhow::Error>(())
        })
        .await
    }

    pub async fn accept(
        &self,
        data: &[u8],
        session: &RiseWireSocketSession,
    ) -> anyhow::Result<RiseWireInboundDisposition> {
        self.with_open_operation(async { Ok(self.wire.accept(data, session).await) }).await
    }

    pub async fn is_ready_for_calls(&self, survives_suspension: bool) -> anyhow::Result<bool> {
        self.with_open_operation(async { Ok(self.wire.is_ready_for_calls(survives_suspension).await) })
            .await
    }

    /// true if a call with this descriptor can go out now, an http call needs no socket
    pub async fn can_call(&self, descriptor: &RiseWireMethodDescriptor) -> anyhow::Result<bool> {
        self.with_open_operation(async { Ok(self.wire.can_call(descriptor).await) }).await
    }

    pub async fn write(&self, batch: RiseStoreWriteBatch) -> anyhow::Result<RiseStoreCommit> {
        self.with_open_operation(async { self.store.write(batch).await.map_err(anyhow::Error::from) }).await
    }

    pub async fn snapshot(&self, request: RiseStoreSnapshotRequest) -> anyhow::Result<RiseStoreSnapshot> {
        self.with_open_operation(async { self.store.snapshot(request).await.map_err(anyhow::Error::from) }).await
    }

    pub async fn observe(
        &self,
        request: RiseStoreSnapshotRequest,
    ) -> anyhow::Result<RiseStoreObservation> {
        self.with_open_operation(async { self.store.observe(request).await.map_err(anyhow::Error::from) }).await
    }

    pub async fn value(
        &self,
        namespace: RiseStoreNamespace,
        key: String,
    ) -> anyhow::Result<Option<RiseStoredValue>> {
        self.with_open_operation(async { self.store.value(namespace, key).await.map_err(anyhow::Error::from) }).await
    }

    pub async fn ordered_items(
        &self,
        namespace: RiseStoreNamespace,
        collection: String,
        limit: i64,
        ascending: bool,
        anchor: Option<RiseCollectionAnchor>,
    ) -> anyhow::Result<Vec<RiseOrderedItem>> {
        self.with_open_operation(async {
            self.store
                .ordered_items(namespace, collection, limit, ascending, anchor)
                .await
                .map_err(anyhow::Error::from)
        })
        .await
    }

    /// o(1) lookup by local primary key
    pub async fn entity_local_id(
        &self,
        namespace: RiseStoreNamespace,
        local_id: String,
    ) -> anyhow::Result<Option<RiseStoreEntity>> {
        self.with_open_operation(async { self.store.entity_local_id(namespace, local_id).await.map_err(anyhow::Error::from) })
            .await
    }

    /// o(1) lookup by a server or client or temporary identity
    pub async fn entity_alias(
        &self,
        namespace: RiseStoreNamespace,
        alias: RiseEntityAlias,
    ) -> anyhow::Result<Option<RiseStoreEntity>> {
        self.with_open_operation(async { self.store.entity_alias(namespace, alias).await.map_err(anyhow::Error::from) })
            .await
    }

    pub async fn entity_window(
        &self,
        query: RiseEntityWindowQuery,
    ) -> anyhow::Result<RiseEntityWindowResult> {
        self.with_open_operation(async { self.store.entity_window(query).await.map_err(anyhow::Error::from) }).await
    }

    pub async fn entity_view_state(
        &self,
        namespace: RiseStoreNamespace,
        view: String,
    ) -> anyhow::Result<Option<RiseEntityViewState>> {
        self.with_open_operation(async { self.store.entity_view_state(namespace, view).await.map_err(anyhow::Error::from) })
            .await
    }

    pub async fn entity_view_payload_bytes(
        &self,
        namespace: RiseStoreNamespace,
    ) -> anyhow::Result<HashMap<String, i64>> {
        self.with_open_operation(async { self.store.entity_view_payload_bytes(namespace).await.map_err(anyhow::Error::from) })
            .await
    }

    pub async fn retained_entity_view_positions(
        &self,
        namespace: RiseStoreNamespace,
        view: String,
    ) -> anyhow::Result<Vec<RiseEntityViewPosition>> {
        self.with_open_operation(async {
            self.store.retained_entity_view_positions(namespace, view).await.map_err(anyhow::Error::from)
        })
        .await
    }

    /// o(1) lookup of the saved anchor for deep around windows
    pub async fn entity_view_position(
        &self,
        namespace: RiseStoreNamespace,
        view: String,
        local_id: String,
    ) -> anyhow::Result<Option<RiseEntityViewPosition>> {
        self.with_open_operation(async {
            self.store.entity_view_position(namespace, view, local_id).await.map_err(anyhow::Error::from)
        })
        .await
    }

    pub async fn remove_local_account_data(
        base_directory_url: &Path,
        account_id: &str,
        key_store: Arc<dyn RiseEngineKeyStore>,
    ) -> anyhow::Result<()> {
        let directory = Self::account_directory(base_directory_url, account_id);
        let database_url = directory.join(ENGINE_DATABASE_FILE_NAME);
        let Some(ownership) = RiseEngineStoreRegistry::shared().claim(&database_url, account_id)
        else {
            return Err(RiseEngineError::AccountInUse.into());
        };
        let result = Self::remove_claimed(&directory, account_id, key_store).await;
        ownership.release();
        result
    }

    async fn remove_claimed(
        directory: &Path,
        account_id: &str,
        key_store: Arc<dyn RiseEngineKeyStore>,
    ) -> anyhow::Result<()> {
        key_store.delete_key(account_id).await?;
        let file_manager = FileManager::default();
        if file_manager.file_exists(directory) {
            file_manager.remove_item(directory)?;
        }
        Ok(())
    }

    pub fn local_account_directory_url(base_directory_url: &Path, account_id: &str) -> PathBuf {
        Self::account_directory(base_directory_url, account_id)
    }

    pub(crate) fn account_directory(base: &Path, account_id: &str) -> PathBuf {
        let name = sha256_hex(account_id.as_bytes());
        base.join(ENGINE_DIRECTORY_NAME).join(name)
    }

    fn call_context(&self, descriptor: &RiseWireMethodDescriptor) -> RiseEngineCallContext {
        RiseEngineCallContext::new(
            Uuid::new_v4(),
            descriptor.service.clone(),
            descriptor.method.clone(),
            descriptor.replay_policy.clone(),
            self.account.account_id.clone(),
            Date::now(),
        )
    }

    async fn with_open_operation<Value>(
        &self,
        operation: impl Future<Output = anyhow::Result<Value>>,
    ) -> anyhow::Result<Value> {
        self.lifecycle.begin_operation().await?;
        let _admitted = RiseEngineAdmittedOperation { lifecycle: &self.lifecycle };
        operation.await
    }
}

/// trace of one call, finishes it with the error on failure
struct RiseEngineCallTrace {
    context: Option<RiseEngineCallContext>,
}

impl RiseEngineCallTrace {
    fn finish(&mut self, error: Option<&anyhow::Error>) {
        if let Some(context) = self.context.take() {
            RiseEngineObservability::finish(&context, error);
        }
    }
}

impl Drop for RiseEngineCallTrace {
    fn drop(&mut self) {
        if let Some(context) = self.context.take() {
            let cancelled =
                anyhow::Error::new(crate::rise_wire::rise_wire::CancellationError);
            RiseEngineObservability::finish(&context, Some(&cancelled));
        }
    }
}

struct RiseEngineAdmittedOperation<'a> {
    lifecycle: &'a RiseEngineLifecycle,
}

impl Drop for RiseEngineAdmittedOperation<'_> {
    fn drop(&mut self) {
        self.lifecycle.end_operation();
    }
}

struct RiseEngineStoreRegistry {
    claims: Mutex<RiseEngineStoreRegistryClaims>,
}

#[derive(Default)]
struct RiseEngineStoreRegistryClaims {
    claimed_database_paths: HashSet<PathBuf>,
    claimed_account_ids: HashSet<String>,
}

static SHARED_REGISTRY: LazyLock<RiseEngineStoreRegistry> = LazyLock::new(|| {
    RiseEngineStoreRegistry { claims: Mutex::new(RiseEngineStoreRegistryClaims::default()) }
});

impl RiseEngineStoreRegistry {
    fn shared() -> &'static RiseEngineStoreRegistry {
        &SHARED_REGISTRY
    }

    fn claim(
        &'static self,
        database_url: &Path,
        account_id: &str,
    ) -> Option<Arc<RiseEngineStoreOwnership>> {
        let path = resolving_symlinks_in_path(database_url);
        let mut claims = self.claims.lock();
        if claims.claimed_database_paths.contains(&path)
            || claims.claimed_account_ids.contains(account_id)
        {
            return None;
        }
        claims.claimed_database_paths.insert(path.clone());
        claims.claimed_account_ids.insert(account_id.to_owned());
        Some(Arc::new(RiseEngineStoreOwnership {
            registry: self,
            database_path: path,
            account_id: account_id.to_owned(),
            is_released: Mutex::new(false),
        }))
    }

    fn release(&self, database_path: &Path, account_id: &str) {
        let mut claims = self.claims.lock();
        claims.claimed_database_paths.remove(database_path);
        claims.claimed_account_ids.remove(account_id);
    }
}

/// resolves symlinks in a path, the db file often doesnt exist yet
/// so the deepest existing ancestor is resolved and the rest stays as written
fn resolving_symlinks_in_path(url: &Path) -> PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(url) {
        return resolved;
    }
    match (url.parent(), url.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
            resolving_symlinks_in_path(parent).join(name)
        }
        _ => url.to_path_buf(),
    }
}

struct RiseEngineStoreOwnership {
    registry: &'static RiseEngineStoreRegistry,
    database_path: PathBuf,
    account_id: String,
    is_released: Mutex<bool>,
}

impl RiseEngineStoreOwnership {
    fn release(&self) {
        {
            let mut is_released = self.is_released.lock();
            if *is_released {
                return;
            }
            *is_released = true;
        }
        self.registry.release(&self.database_path, &self.account_id);
    }
}

impl Drop for RiseEngineStoreOwnership {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RiseEngineLifecycleState {
    Open,
    Closing,
    Closed,
}

type RiseEngineLifecycleCloseFlight = Shared<BoxFuture<'static, Result<(), Arc<anyhow::Error>>>>;

struct RiseEngineLifecycleIsolated {
    state: RiseEngineLifecycleState,
    active_operations: usize,
    drain_waiter: Option<oneshot::Sender<()>>,
    close_flight: Option<RiseEngineLifecycleCloseFlight>,
}

pub(crate) struct RiseEngineLifecycle {
    isolated: Arc<RiseIsolated<RiseEngineLifecycleIsolated>>,
}

impl RiseEngineLifecycle {
    pub(crate) fn new() -> Self {
        Self {
            isolated: Arc::new(RiseIsolated::new(RiseEngineLifecycleIsolated {
                state: RiseEngineLifecycleState::Open,
                active_operations: 0,
                drain_waiter: None,
                close_flight: None,
            })),
        }
    }

    pub(crate) async fn begin_operation(&self) -> Result<(), RiseEngineError> {
        self.isolated.with(|isolated| {
            if isolated.state != RiseEngineLifecycleState::Open {
                return Err(RiseEngineError::AlreadyClosed);
            }
            isolated.active_operations += 1;
            Ok(())
        })
    }

    pub(crate) fn end_operation(&self) {
        self.isolated.with(|isolated| {
            debug_assert!(isolated.active_operations > 0);
            isolated.active_operations = isolated.active_operations.saturating_sub(1);
            if isolated.active_operations == 0
                && let Some(drain_waiter) = isolated.drain_waiter.take()
            {
                let _ = drain_waiter.send(());
            }
        });
    }

    #[cfg(test)]
    pub(crate) async fn accepts_operations(&self) -> bool {
        self.isolated.with(|isolated| isolated.state == RiseEngineLifecycleState::Open)
    }

    pub(crate) async fn close(
        &self,
        prepare: BoxFuture<'static, ()>,
        finish: BoxFuture<'static, anyhow::Result<()>>,
    ) -> anyhow::Result<()> {
        let flight = self.isolated.with(|isolated| {
            if let Some(close_flight) = &isolated.close_flight {
                return Some(close_flight.clone());
            }
            if isolated.state == RiseEngineLifecycleState::Closed {
                return None;
            }
            isolated.state = RiseEngineLifecycleState::Closing;
            let lifecycle = self.isolated.clone();
            let task = RiseAccountRuntime::current().spawn(async move {
                prepare.await;
                Self::wait_for_active_operations(&lifecycle).await;
                let result = finish.await;
                lifecycle.with(|isolated| match &result {
                    Ok(()) => isolated.state = RiseEngineLifecycleState::Closed,
                    Err(_) => isolated.close_flight = None,
                });
                result.map_err(Arc::new)
            });
            let close_flight: RiseEngineLifecycleCloseFlight = async move {
                match task.await {
                    Ok(result) => result,
                    Err(join_error) => Err(Arc::new(anyhow::Error::new(join_error))),
                }
            }
            .boxed()
            .shared();
            isolated.close_flight = Some(close_flight.clone());
            Some(close_flight)
        });
        match flight {
            Some(flight) => flight.await.map_err(|error| shared_close_error(&error)),
            None => Ok(()),
        }
    }

    async fn wait_for_active_operations(isolated: &RiseIsolated<RiseEngineLifecycleIsolated>) {
        let waiter = isolated.with(|isolated| {
            if isolated.active_operations == 0 {
                return None;
            }
            debug_assert!(isolated.drain_waiter.is_none());
            let (drain_waiter, drained) = oneshot::channel();
            isolated.drain_waiter = Some(drain_waiter);
            Some(drained)
        });
        if let Some(drained) = waiter {
            let _ = drained.await;
        }
    }
}

fn shared_close_error(error: &anyhow::Error) -> anyhow::Error {
    if let Some(store_error) = error.downcast_ref::<RiseStoreError>() {
        return store_error.clone().into();
    }
    if let Some(engine_error) = error.downcast_ref::<RiseEngineError>() {
        return (*engine_error).into();
    }
    anyhow::anyhow!("{error:#}")
}
