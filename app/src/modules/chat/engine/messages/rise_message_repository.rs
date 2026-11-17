use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Weak};

use futures::future::{BoxFuture, Shared, try_join};
use rise_canon::RiseLimits;
use rise_canon::rise_request;
use rise_canon::rise_upload::{self, ErrorCode as RiseUploadErrorCode};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_core::rise_identifiers::RiseRequestID;
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityAliasLookup, RiseEntityKey, RiseEntityViewKey,
    RiseEntityViewLoadState, RiseEntityViewMembership, RiseEntityViewRetention, RiseEntityViewState, RiseEntityWindow,
    RiseEntityWindowQuery, RiseEntityWindowResult, RiseEntityWritePolicy, RisePositionKey, RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::{
    RiseStoreCommit, RiseStoreMutation, RiseStoreSnapshot, RiseStoreSnapshotRequest, RiseStoreWriteBatch,
};
use rise_engine::rise_upload::rise_chunked_upload::RiseChunkedUploadError;
use rise_engine::rise_upload::rise_upload_progress::{
    RiseUploadCancellationHub, RiseUploadKey, RiseUploadProgress, RiseUploadProgressHub, RiseUploadProgressStream,
    RiseUploadRealtimeProgressHub,
};
use rise_engine::rise_upload::rise_url_session::URLError;
use rise_engine::{CancellationError, RiseEngine, RiseJSONValue, RiseWireRemoteError};
use tokio::sync::Notify;
use tokio::task::{AbortHandle, JoinHandle};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseColdOpenTrace;
use crate::modules::chat::engine::core::rise_conversation_codec::RiseConversationCodec;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseConversationEntitySource, RiseMessageEntityRecord, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationEntityStamp, RiseMessageWireSnapshot,
};
use crate::modules::chat::engine::core::rise_conversation_store_layout::{
    RiseConversationStoreLayout, RiseEntityAliasKindExt,
};
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseMessageAuxiliaryInvalidation, RiseMessageAuxiliaryInvalidationBatchHook,
    RiseMessageAuxiliaryInvalidationPinnedMessagePatch, RiseMessageAuxiliaryProjection,
    RiseMessageChatPreviewBatchHook, RiseMessageChatReadOptimisticHook, RiseMessageEngineRemoteAPI,
    RiseMessageHistoryClearReceipt, RiseMessagePageContract, RiseMessagePreviewChange, RiseMessageReadReceiptHintHook,
    RiseMessageRemoteAPI, RiseMessageRepositoryError, RiseMessageUploadPipeline,
};
use crate::modules::chat::engine::messages::rise_message_durable_outbox::{
    RiseAddReactionDurableEndpoint, RiseCreateMessageDurableEndpoint, RiseRemoveReactionDurableEndpoint,
};
use crate::modules::chat::engine::messages::rise_message_ephemeral_state::{
    RiseMessageTypingHub, RiseMessageTypingStream,
};
use crate::modules::chat::engine::messages::rise_message_scope::{
    RiseMessageAnchor, RiseMessagePageDirection, RiseMessagePageLoadOutcome, RiseMessagePageLoadReceipt,
    RiseMessageScope, RiseMessageWindowConfiguration, RiseMessageWindowShift,
};
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::engine::messages::rise_message_mutations::SendOrderGate;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageContentType, MessageSystemKind};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    GetMessagesAroundGatewayDTO, GetMessagesGatewayDTO, MessageWireDTO,
};

#[derive(Clone, Debug)]
pub struct RiseMessageDurableTelemetryRecord {
    pub request_id: RiseRequestID,
    pub scope: RiseMessageScope,
    pub local_id: String,
    pub local_phase: RiseMessageLocalPhase,
    pub last_error_code: Option<String>,
    pub entity_version: i64,
    pub entity_source: RiseConversationEntitySource,
    pub source_revision: i64,
}

struct RiseMessageWireStreamBuffer {
    latest: Option<RiseMessageWireSnapshot>,
    terminal: Option<Option<anyhow::Error>>,
}

struct RiseMessageWireStreamShared {
    buffer: RiseIsolated<RiseMessageWireStreamBuffer>,
    notify: Notify,
}

impl RiseMessageWireStreamShared {
    fn yield_snapshot(&self, snapshot: RiseMessageWireSnapshot) {
        self.buffer.with(|buffer| buffer.latest = Some(snapshot));
        self.notify.notify_one();
    }

    fn finish(&self, error: Option<anyhow::Error>) {
        self.buffer.with(|buffer| buffer.terminal = Some(error));
        self.notify.notify_one();
    }
}

/// what an observed window shows, rows at their entity versions and the view state
/// two equal windows present the same timeline whatever the store revision
#[derive(PartialEq)]
struct RiseMessageWindowShown {
    local_ids: Vec<String>,
    entity_stamps: Vec<RiseConversationEntityStamp>,
    anchor_local_id: Option<String>,
    has_cached_before: bool,
    has_cached_after: bool,
    before_cursor: Option<String>,
    after_cursor: Option<String>,
    has_more_before: bool,
    has_more_after: bool,
    load_state: RiseEntityViewLoadState,
    error_code: Option<String>,
}

impl RiseMessageWindowShown {
    fn of(snapshot: &RiseMessageWireSnapshot) -> Self {
        Self {
            local_ids: snapshot.local_ids.clone(),
            entity_stamps: snapshot.entity_stamps.clone(),
            anchor_local_id: snapshot.anchor_local_id.clone(),
            has_cached_before: snapshot.has_cached_before,
            has_cached_after: snapshot.has_cached_after,
            before_cursor: snapshot.before_cursor.clone(),
            after_cursor: snapshot.after_cursor.clone(),
            has_more_before: snapshot.has_more_before,
            has_more_after: snapshot.has_more_after,
            load_state: snapshot.load_state,
            error_code: snapshot.error_code.clone(),
        }
    }

    /// rows whose id or entity version is new compared to previous
    fn changed_rows(&self, previous: &Self) -> usize {
        let before: HashSet<(&str, &RiseConversationEntityStamp)> =
            previous.local_ids.iter().map(String::as_str).zip(previous.entity_stamps.iter()).collect();
        self.local_ids.iter().map(String::as_str).zip(self.entity_stamps.iter()).filter(|row| !before.contains(row)).count()
    }
}

/// stream of the newest only, dropping it cancels the producer
pub struct RiseMessageWireStream {
    shared: Arc<RiseMessageWireStreamShared>,
    task: JoinHandle<()>,
}

impl RiseMessageWireStream {
    pub async fn next(&mut self) -> Option<anyhow::Result<RiseMessageWireSnapshot>> {
        loop {
            let step = self.shared.buffer.with(|buffer| {
                if let Some(snapshot) = buffer.latest.take() {
                    return Some(Some(Ok(snapshot)));
                }
                buffer.terminal.as_mut().map(|terminal| terminal.take().map(Err))
            });
            if let Some(element) = step {
                return element;
            }
            self.shared.notify.notified().await;
        }
    }
}

impl Drop for RiseMessageWireStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct RiseMessageRepositoryObservation {
    pub seed: RiseMessageWireSnapshot,
    pub updates: RiseMessageWireStream,
}

#[derive(Clone)]
pub struct RiseMessageRepositoryEncodedRow {
    pub local_id: String,
    pub data: Vec<u8>,
    pub position: RisePositionKey,
    pub aliases: Vec<RiseEntityAlias>,
    pub duplicate_local_ids: Vec<String>,
    pub entity_version: i64,
    pub is_unchanged: bool,
}

#[derive(Clone)]
pub struct RiseMessageRepositoryPreservedMembership {
    pub local_id: String,
    pub position: RisePositionKey,
    pub version: i64,
    pub entity_version: i64,
    pub is_local: bool,
    pub server_id: Option<String>,
    pub written_at_milliseconds: i64,
}

#[derive(Clone)]
pub struct RiseMessageRepositoryResolvedRow {
    pub wire: MessageWireDTO,
    pub local_id: String,
    pub aliases: Vec<RiseEntityAlias>,
    pub duplicate_local_ids: Vec<String>,
    pub entity_version: i64,
    pub existing_data: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RiseMessageRepositoryDecodedCacheKey {
    pub local_id: String,
    pub version: i64,
    pub updated_at_milliseconds: i64,
}

#[derive(Clone)]
pub struct RiseMessageRepositoryDecodedCacheEntry {
    pub record: RiseMessageEntityRecord,
    pub access: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RiseMessageRepositoryRequestLane {
    Seed,
    Older,
    Newer,
}

impl RiseMessageRepositoryRequestLane {
    const ALL: [Self; 3] = [Self::Seed, Self::Older, Self::Newer];
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RiseMessageRepositoryRequestKey {
    pub scope: RiseMessageScope,
    pub lane: RiseMessageRepositoryRequestLane,
}

impl RiseMessageRepositoryRequestKey {
    pub fn new(scope: &RiseMessageScope, lane: RiseMessageRepositoryRequestLane) -> Self {
        Self { scope: scope.clone(), lane }
    }
}

#[derive(Clone)]
pub struct RiseMessageRepositoryRequestContext {
    pub key: RiseMessageRepositoryRequestKey,
    pub token: u64,
    pub seed_generation: u64,
    pub requested_cursor: Option<String>,
    pub version: i64,
    pub started_state: Option<RiseEntityViewState>,
}

struct RiseMessageRepositoryPagePrefetch {
    key: RiseMessageRepositoryRequestKey,
    cursor: String,
    seed_generation: u64,
    version: i64,
    task: JoinHandle<anyhow::Result<GetMessagesGatewayDTO>>,
}

/// shared future several later sends await, the send gate of one chat
pub type RiseMessageSendOrderGate = Shared<BoxFuture<'static, ()>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMessageRepositoryValidatedCursor {
    pub cursor: Option<String>,
    pub has_more: bool,
    pub stalled: bool,
}

#[derive(Default)]
pub struct RiseMessageRepositoryState {
    page_prefetch: Option<RiseMessageRepositoryPagePrefetch>,
    prefetch_intent_scope: Option<RiseMessageScope>,
    prefetch_intent_generation: u64,
    pub(crate) version_clock: i64,
    pub(crate) request_tokens: HashMap<RiseMessageRepositoryRequestKey, u64>,
    pub(crate) seed_generations: HashMap<RiseMessageScope, u64>,
    pub(crate) loading: HashSet<RiseMessageRepositoryRequestKey>,
    pub(crate) known_scopes: HashSet<RiseMessageScope>,
    pub(crate) live_upload_owner_ids: HashSet<String>,
    pub(crate) settled_orphan_upload_ids: HashSet<String>,
    pub(crate) live_fanout_local_ids: HashSet<String>,
    pub(crate) settled_orphan_fanout_ids: HashSet<String>,
    pub(crate) send_order_turns: HashMap<String, Vec<SendOrderGate>>,
    pub(crate) read_flight_chat_ids: HashSet<String>,
    pub(crate) queued_reads: HashMap<String, HashMap<RiseMessageScope, bool>>,
    pub(crate) queued_read_up_to: HashMap<String, HashMap<RiseMessageScope, String>>,
    pub(crate) decoded_record_cache: HashMap<RiseMessageRepositoryDecodedCacheKey, RiseMessageRepositoryDecodedCacheEntry>,
    pub(crate) decoded_cache_access: u64,
    pub(crate) recent_created_aliases: HashMap<RiseEntityAlias, u64>,
    pub(crate) recent_created_alias_access: u64,
    pub(crate) early_link_previews: HashMap<String, BTreeMap<String, RiseJSONValue>>,
    pub(crate) early_link_preview_order: Vec<String>,
    pub(crate) reduced_created_rows: HashMap<String, RiseStoreEntity>,
    pub(crate) reduced_created_order: Vec<String>,
}

/// cleanup of a load plus the cancelled case
/// a dropped future never errors so the loading state it wrote must be cleared
struct RiseMessageRequestGuard<'a> {
    repository: &'a RiseMessageRepository,
    context: RiseMessageRepositoryRequestContext,
    writes_cancellation: bool,
    finishes_request: bool,
}

impl<'a> RiseMessageRequestGuard<'a> {
    fn new(repository: &'a RiseMessageRepository, context: &RiseMessageRepositoryRequestContext) -> Self {
        Self { repository, context: context.clone(), writes_cancellation: true, finishes_request: true }
    }
}

impl Drop for RiseMessageRequestGuard<'_> {
    fn drop(&mut self) {
        if self.writes_cancellation
            && self.repository.is_current(&self.context)
            && let Some(repository) = self.repository.this.upgrade()
        {
            let scope = self.context.key.scope.clone();
            let previous = self.context.started_state.clone();
            RiseAccountRuntime::current().spawn(async move {
                let cancelled: anyhow::Error = CancellationError.into();
                let _ = repository.write_failure(&scope, previous.as_ref(), &cancelled).await;
            });
        }
        if self.finishes_request {
            self.repository.finish_request(&self.context);
        }
    }
}

struct RiseMessageLoadingInsertGuard<'a> {
    repository: &'a RiseMessageRepository,
    key: RiseMessageRepositoryRequestKey,
    is_armed: bool,
}

impl Drop for RiseMessageLoadingInsertGuard<'_> {
    fn drop(&mut self) {
        if self.is_armed {
            self.repository.state.with(|state| state.loading.remove(&self.key));
        }
    }
}

struct RiseMessagePrefetchAbortOnDrop(AbortHandle);

impl Drop for RiseMessagePrefetchAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct RiseMessageRepository {
    pub(crate) this: Weak<RiseMessageRepository>,
    pub(crate) engine: Arc<RiseEngine>,
    pub(crate) remote: Arc<dyn RiseMessageRemoteAPI>,
    pub(crate) account_id: String,
    pub(crate) page_limit: i64,
    pub(crate) retention_limit: i64,
    pub(crate) upload_pipeline: Option<Arc<dyn RiseMessageUploadPipeline>>,
    pub(crate) preview_hook: RiseMessageChatPreviewBatchHook,
    pub(crate) chat_read_optimistic_hook: RiseMessageChatReadOptimisticHook,
    pub(crate) auxiliary_invalidation_hook: RiseMessageAuxiliaryInvalidationBatchHook,
    pub(crate) read_receipt_hint_hook: RiseMessageReadReceiptHintHook,
    pub(crate) typing_hub: Arc<RiseMessageTypingHub>,
    pub(crate) upload_progress_hub: Arc<RiseUploadProgressHub>,
    pub(crate) upload_cancellation_hub: Arc<RiseUploadCancellationHub>,
    pub(crate) upload_realtime_progress: Arc<RiseUploadRealtimeProgressHub>,
    pub(crate) decoded_cache_limit: i64,
    pub(crate) recent_created_alias_limit: i64,
    pub(crate) state: RiseIsolated<RiseMessageRepositoryState>,
}

impl RiseMessageRepository {
    const CANCELLED_ERROR_CODE: &'static str = "cancelled";
    const CREATE_MESSAGE_FAILED_ERROR_CODE: &'static str = "create_message_failed";
    const MESSAGE_REQUEST_FAILED_ERROR_CODE: &'static str = "message_request_failed";
    const TEMPORARY_ID_PREFIX: &'static str = "temp_";
    const MAXIMUM_PAGE_ROWS: i64 = 320;
    const MINIMUM_RETENTION_ROWS: i64 = 320;
    const ALIAS_LOOKUP_CHUNK: usize = 128;

    pub fn default_page_limit() -> i64 {
        RiseLimits::client_value("message_repository_page_rows")
    }

    pub fn default_retention_limit() -> i64 {
        RiseLimits::client_value("message_repository_retention_rows")
    }

    pub fn page_replica_grace_milliseconds() -> i64 {
        RiseLimits::client_value("message_page_replica_grace_ms")
    }

    fn around_side_limit() -> i64 {
        RiseLimits::service_value("get_messages_around_limit_clamp")
    }

    pub fn new(
        engine: Arc<RiseEngine>,
        account_id: impl Into<String>,
        upload_realtime_progress: Arc<RiseUploadRealtimeProgressHub>,
        preview_hook: RiseMessageChatPreviewBatchHook,
        chat_read_optimistic_hook: RiseMessageChatReadOptimisticHook,
        auxiliary_invalidation_hook: RiseMessageAuxiliaryInvalidationBatchHook,
        read_receipt_hint_hook: RiseMessageReadReceiptHintHook,
    ) -> Arc<Self> {
        Self::with_configuration(
            engine,
            account_id,
            None,
            None,
            upload_realtime_progress,
            preview_hook,
            chat_read_optimistic_hook,
            auxiliary_invalidation_hook,
            read_receipt_hint_hook,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_configuration(
        engine: Arc<RiseEngine>,
        account_id: impl Into<String>,
        remote: Option<Arc<dyn RiseMessageRemoteAPI>>,
        upload_pipeline: Option<Arc<dyn RiseMessageUploadPipeline>>,
        upload_realtime_progress: Arc<RiseUploadRealtimeProgressHub>,
        preview_hook: RiseMessageChatPreviewBatchHook,
        chat_read_optimistic_hook: RiseMessageChatReadOptimisticHook,
        auxiliary_invalidation_hook: RiseMessageAuxiliaryInvalidationBatchHook,
        read_receipt_hint_hook: RiseMessageReadReceiptHintHook,
        page_limit: Option<i64>,
        retention_limit: Option<i64>,
    ) -> Arc<Self> {
        let account_id = account_id.into();
        let page_limit = page_limit.unwrap_or_else(Self::default_page_limit);
        let retention_limit = retention_limit.unwrap_or_else(Self::default_retention_limit);
        assert!(!account_id.is_empty());
        assert!(page_limit > 0 && page_limit <= Self::MAXIMUM_PAGE_ROWS);
        assert!(retention_limit >= Self::MINIMUM_RETENTION_ROWS);
        let remote = remote.unwrap_or_else(|| Arc::new(RiseMessageEngineRemoteAPI::new(engine.clone())));
        Arc::new_cyclic(|this| Self {
            this: this.clone(),
            engine,
            remote,
            account_id,
            page_limit,
            retention_limit,
            upload_pipeline,
            preview_hook,
            chat_read_optimistic_hook,
            auxiliary_invalidation_hook,
            read_receipt_hint_hook,
            typing_hub: Arc::new(RiseMessageTypingHub::new()),
            upload_progress_hub: Arc::new(RiseUploadProgressHub::default()),
            upload_cancellation_hub: Arc::new(RiseUploadCancellationHub::default()),
            upload_realtime_progress,
            decoded_cache_limit: RiseLimits::client_value("message_decoded_record_cache_entries"),
            recent_created_alias_limit: RiseLimits::client_value("message_repository_recent_created_aliases"),
            state: RiseIsolated::new(RiseMessageRepositoryState::default()),
        })
    }

    pub async fn auxiliary_invalidation_batch_scope(
        &self,
        scope: &RiseMessageScope,
        projections: HashSet<RiseMessageAuxiliaryProjection>,
        pinned_message_patch: Option<RiseMessageAuxiliaryInvalidationPinnedMessagePatch>,
    ) -> RiseStoreWriteBatch {
        let mut invalidation = RiseMessageAuxiliaryInvalidation::with_scope(scope, projections);
        invalidation.pinned_message_patch = pinned_message_patch;
        self.auxiliary_invalidation_batch(invalidation).await
    }

    pub async fn auxiliary_invalidation_batch(&self, invalidation: RiseMessageAuxiliaryInvalidation) -> RiseStoreWriteBatch {
        if invalidation.chat_id.is_empty() || invalidation.projections.is_empty() {
            return RiseStoreWriteBatch::empty();
        }
        self.auxiliary_invalidation_hook.call(invalidation).await
    }

    /// used by optimistic mutations whose message write is already committed
    /// eviction is best effort, a secondary projection must never fail the real mutation
    pub async fn invalidate_auxiliary_scope(
        &self,
        scope: &RiseMessageScope,
        projections: &HashSet<RiseMessageAuxiliaryProjection>,
    ) {
        self.invalidate_auxiliary(RiseMessageAuxiliaryInvalidation::with_scope(scope, projections.clone())).await;
    }

    pub async fn invalidate_auxiliary(&self, invalidation: RiseMessageAuxiliaryInvalidation) {
        let batch = self.auxiliary_invalidation_batch(invalidation).await;
        if batch.mutations.is_empty() && batch.invalidated_views.is_empty() {
            return;
        }
        let _ = self.engine.write(batch).await;
    }

    pub async fn register(&self, context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        context.register_durable_endpoint::<RiseCreateMessageDurableEndpoint>().await?;
        context.register_durable_endpoint::<RiseAddReactionDurableEndpoint>().await?;
        context.register_durable_endpoint::<RiseRemoveReactionDurableEndpoint>().await?;
        for handler in self.realtime_push_handlers() {
            context.register_push_handler(handler).await?;
        }
        Ok(())
    }

    pub async fn observe(
        &self,
        scope: &RiseMessageScope,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseMessageWireStream> {
        Ok(self.observe_with_seed_including(scope, anchor, configuration, true).await?.updates)
    }

    pub async fn observe_with_seed(
        &self,
        scope: &RiseMessageScope,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseMessageRepositoryObservation> {
        self.observe_with_seed_including(scope, anchor, configuration, false).await
    }

    pub async fn observe_with_seed_including(
        &self,
        scope: &RiseMessageScope,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
        including_seed_in_updates: bool,
    ) -> anyhow::Result<RiseMessageRepositoryObservation> {
        self.state.with(|state| state.known_scopes.insert(scope.clone()));
        let query = self.window_query(scope, &anchor, configuration).await?;
        let request = Self::presentation_request(scope, &query);
        let observation = self.engine.observe(request.clone()).await?;
        let seed = self.presentation_snapshot(&observation.seed, &query, scope)?;
        let repository = self.this.upgrade().ok_or(CancellationError)?;
        let engine = self.engine.clone();
        let shared = Arc::new(RiseMessageWireStreamShared {
            buffer: RiseIsolated::new(RiseMessageWireStreamBuffer { latest: None, terminal: None }),
            notify: Notify::new(),
        });
        let continuation = shared.clone();
        let task_scope = scope.clone();
        let task_seed = including_seed_in_updates.then(|| seed.clone());
        let mut shown = RiseMessageWindowShown::of(&seed);
        let updates = observation.updates;
        let task = RiseAccountRuntime::current().spawn(async move {
            let outcome: anyhow::Result<()> = async {
                if let Some(task_seed) = task_seed {
                    continuation.yield_snapshot(task_seed);
                }
                let mut updates = updates.make_async_iterator();
                while updates.next().await.is_some() {
                    let snapshot = engine.snapshot(request.clone()).await?;
                    let prepared = repository.presentation_snapshot(&snapshot, &query, &task_scope)?;
                    let next = RiseMessageWindowShown::of(&prepared);
                    if next == shown {
                        continue;
                    }
                    tracing::warn!(
                        target: "riseonly::X1898",
                        chat = %task_scope.chat_id,
                        changed = next.changed_rows(&shown),
                        rows = next.local_ids.len(),
                        "message window update"
                    );
                    shown = next;
                    continuation.yield_snapshot(prepared);
                }
                Ok(())
            }
            .await;
            continuation.finish(outcome.err().filter(|error| !error.is::<CancellationError>()));
        });
        Ok(RiseMessageRepositoryObservation { seed, updates: RiseMessageWireStream { shared, task } })
    }

    fn presentation_request(scope: &RiseMessageScope, query: &RiseEntityWindowQuery) -> RiseStoreSnapshotRequest {
        RiseStoreSnapshotRequest {
            entity_windows: HashSet::from([query.clone()]),
            entity_view_states: HashSet::from([RiseEntityViewKey::new(
                RiseConversationStoreLayout::messages(),
                RiseConversationStoreLayout::message_view(scope),
            )]),
            ..RiseStoreSnapshotRequest::default()
        }
    }

    pub async fn snapshot(
        &self,
        scope: &RiseMessageScope,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseMessageWireSnapshot> {
        self.snapshot_anchor(scope, anchor, configuration, true).await
    }

    pub async fn snapshot_anchor(
        &self,
        scope: &RiseMessageScope,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
        registers_scope: bool,
    ) -> anyhow::Result<RiseMessageWireSnapshot> {
        if registers_scope {
            self.state.with(|state| state.known_scopes.insert(scope.clone()));
        }
        let query = self.window_query(scope, &anchor, configuration).await?;
        let value = self.engine.snapshot(Self::presentation_request(scope, &query)).await?;
        self.presentation_snapshot(&value, &query, scope)
    }

    pub async fn durable_telemetry_record(
        &self,
        request_id: RiseRequestID,
    ) -> anyhow::Result<Option<RiseMessageDurableTelemetryRecord>> {
        if self.engine.account.account_id != self.account_id {
            return Err(RiseMessageRepositoryError::InvalidScope.into());
        }
        let request_string = request_id.raw_value().to_string();
        let lookup = RiseEntityAliasLookup::new(
            RiseConversationStoreLayout::messages(),
            RiseEntityAlias::new(RiseEntityAliasKind::request_id(), request_string.clone()),
        );
        let snapshot = self
            .engine
            .snapshot(RiseStoreSnapshotRequest {
                entity_aliases: HashSet::from([lookup.clone()]),
                ..RiseStoreSnapshotRequest::default()
            })
            .await?;
        let Some(entity) = snapshot.entity_aliases.get(&lookup) else {
            return Ok(None);
        };
        let record = RiseMessageWireCodec::record(&entity.data)
            .map_err(|_| RiseMessageRepositoryError::InvalidWirePayload)?;
        if self.normalized(record.client_request_id.as_deref()).as_deref() != Some(request_string.as_str()) {
            return Err(RiseMessageRepositoryError::InvalidWirePayload.into());
        }
        let Some(chat_id) = self.normalized(record.wire.chat_id.as_deref()) else {
            return Err(RiseMessageRepositoryError::InvalidWirePayload.into());
        };
        let scope = if let Some(scope_storage_key) = record.scope_storage_key.as_deref() {
            match RiseMessageScope::parse(scope_storage_key) {
                Some(persisted_scope) if persisted_scope.chat_id == chat_id => persisted_scope,
                _ => return Err(RiseMessageRepositoryError::InvalidWirePayload.into()),
            }
        } else {
            RiseMessageScope::new(chat_id, record.wire.topic_id.clone(), None)
        };
        Ok(Some(RiseMessageDurableTelemetryRecord {
            request_id,
            scope,
            local_id: entity.local_id.clone(),
            local_phase: record.local_phase,
            last_error_code: record.last_error_code,
            entity_version: entity.version,
            entity_source: record.source,
            source_revision: snapshot.revision,
        }))
    }

    /// empty timelines are materialized too
    /// checking the persisted view state stops an empty chat from being a permanent cache miss
    pub async fn has_materialized_view(&self, scope: &RiseMessageScope) -> anyhow::Result<bool> {
        Ok(self.view_state(scope).await?.is_some())
    }

    pub async fn shifted_snapshot(
        &self,
        scope: &RiseMessageScope,
        around_local_id: &str,
        direction: RiseMessageWindowShift,
    ) -> anyhow::Result<RiseMessageWireSnapshot> {
        self.snapshot(
            scope,
            RiseMessageAnchor::LocalID(around_local_id.to_owned()),
            RiseMessageWindowConfiguration::shifted(direction),
        )
        .await
    }

    fn receipt(outcome: RiseMessagePageLoadOutcome, store_revision: Option<i64>) -> RiseMessagePageLoadReceipt {
        RiseMessagePageLoadReceipt {
            outcome,
            store_revision,
            entity_versions_by_local_id: HashMap::new(),
            used_latest_fallback: false,
            anchor_resolution_failed_authoritatively: false,
            resolved_anchor_id: None,
        }
    }

    /// shared failure handling of the loads
    /// the failure is written here so the guard only keeps its cleanup
    async fn settle_request<T>(
        &self,
        guard: &mut RiseMessageRequestGuard<'_>,
        previous: Option<&RiseEntityViewState>,
        outcome: anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        guard.writes_cancellation = false;
        match outcome {
            Ok(value) => Ok(value),
            Err(error) => {
                if self.is_current(&guard.context) {
                    let _ = self.write_failure(&guard.context.key.scope, previous, &error).await;
                }
                Err(error)
            }
        }
    }

    pub async fn load_initial(&self, scope: &RiseMessageScope) -> anyhow::Result<RiseMessagePageLoadOutcome> {
        Ok(self.load_initial_latest_receipt(scope).await?.outcome)
    }

    pub async fn load_initial_latest_receipt(
        &self,
        scope: &RiseMessageScope,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        let seed_key = RiseMessageRepositoryRequestKey::new(scope, RiseMessageRepositoryRequestLane::Seed);
        let is_loading = self.state.with(|state| {
            state.known_scopes.insert(scope.clone());
            state.loading.contains(&seed_key)
        });
        if is_loading {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::AlreadyLoading, None));
        }
        self.cancel_page_prefetch(Some(scope));
        self.state.with(|state| Self::bump_seed_generation(state, scope));
        let Some(context) = self.begin_request(scope, RiseMessageRepositoryRequestLane::Seed, None).await? else {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::AlreadyLoading, None));
        };
        let mut guard = RiseMessageRequestGuard::new(self, &context);
        self.preserved_memberships(scope).await?;
        let previous = self.view_state(scope).await?;
        let outcome = self.load_initial_latest_page(scope, &context, previous.as_ref()).await;
        self.settle_request(&mut guard, previous.as_ref(), outcome).await
    }

    async fn load_initial_latest_page(
        &self,
        scope: &RiseMessageScope,
        context: &RiseMessageRepositoryRequestContext,
        previous: Option<&RiseEntityViewState>,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        let mut response = self
            .first_nonempty_message_page(
                rise_request::message::GetMessages {
                    chat_id: Some(scope.chat_id.clone()),
                    limit: Some(self.page_limit),
                    relative_id: None,
                    thread_root_id: scope.thread_root_id.clone(),
                    topic_id: scope.topic_id.clone(),
                    up: Some(false),
                    user_chat_id: None,
                },
                context,
            )
            .await?;
        if !self.is_current(context) {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::StaleResponseDropped, None));
        }
        self.validate(response.success, response.error.as_deref())?;
        let messages = RiseMessagePageContract::require_messages(response.messages.take())?;
        let has_more = response.is_have_more.unwrap_or(false);
        if messages.is_empty() && has_more {
            let revision = self.next_version_floor(previous.map_or(context.version, |state| state.revision));
            let before_cursor = self.decode(previous.and_then(|state| state.before_cursor.as_deref()));
            let after_cursor = self.decode(previous.and_then(|state| state.after_cursor.as_deref()));
            let commit = self
                .write_state(
                    scope,
                    revision,
                    before_cursor.as_deref(),
                    after_cursor.as_deref(),
                    previous.is_none_or(|state| state.has_more_before),
                    previous.is_some_and(|state| state.has_more_after),
                    RiseEntityViewLoadState::Loaded,
                    None,
                )
                .await?;
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::CursorStalled, Some(commit.revision)));
        }
        let rows = self.encode_rows(&messages, context.version).await?;
        if !self.is_current(context) {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::StaleResponseDropped, None));
        }
        let raw_cursor = self
            .normalized(response.relative_id.as_deref())
            .or_else(|| self.boundary_cursor(&messages, RiseMessagePageDirection::Older));
        let lane = self.validated_cursor(raw_cursor.as_deref(), has_more, None);
        let cached = self.preserved_memberships(scope).await?;
        let mut preserved: Vec<_> = cached.iter().filter(|item| item.is_local).cloned().collect();
        if lane.has_more
            && let Some(fresh_boundary) = rows.first().map(|row| row.local_id.as_str())
            && let Some(boundary_index) =
                cached.iter().position(|item| !item.is_local && item.local_id == fresh_boundary)
        {
            preserved.extend(cached[..boundary_index].iter().filter(|item| !item.is_local).cloned());
        }
        preserved.extend(self.rows_newer_than_page(rows.last().map(|row| &row.position), &cached));
        let fresh_rows = self.with_fresh_entity_versions(&rows).await?;
        let batch = self.make_page_batch(
            scope,
            &fresh_rows,
            &preserved,
            &self.membership_versions(&cached),
            context.version,
            true,
            lane.cursor.as_deref(),
            None,
            lane.has_more,
            false,
            RiseEntityViewRetention::Latest(self.retention_limit as usize),
        );
        let written = self.engine.write(batch).await?;
        let commit = self.restore_refused_page_memberships(scope, &rows, written).await?;
        self.refresh_call_preview_from_latest_page(scope, &messages).await;
        RiseColdOpenTrace::mark(|| {
            format!(
                "a3a1 seed commit rows={} preserved={} newerKept={} cached={} v={} rev={} merged={} newestTag={}",
                rows.len(),
                preserved.len(),
                self.rows_newer_than_page(rows.last().map(|row| &row.position), &cached).len(),
                cached.len(),
                context.version,
                commit.revision,
                rows.iter().map(|row| row.duplicate_local_ids.len()).sum::<usize>(),
                rows.last().map_or_else(|| "none".to_owned(), |row| RiseColdOpenTrace::tag(&row.local_id))
            )
        });
        let outcome = if lane.stalled {
            RiseMessagePageLoadOutcome::CursorStalled
        } else {
            RiseMessagePageLoadOutcome::Loaded { count: rows.len() as i64 }
        };
        Ok(RiseMessagePageLoadReceipt {
            entity_versions_by_local_id: self.debug_entity_versions(&rows, context.version),
            ..Self::receipt(outcome, Some(commit.revision))
        })
    }

    async fn refresh_call_preview_from_latest_page(&self, scope: &RiseMessageScope, messages: &[MessageWireDTO]) {
        if scope.thread_root_id.is_some() {
            return;
        }
        let Some(newest) = messages.iter().max_by(|left, right| {
            left.created_at.or(left.timestamp).unwrap_or(0.0).total_cmp(&right.created_at.or(right.timestamp).unwrap_or(0.0))
        }) else {
            return;
        };
        if newest.metadata.is_none()
            || MessageContentType::from_raw_value(newest.content_type.as_deref().unwrap_or_default())
                != MessageContentType::Call
        {
            return;
        }
        let change = RiseMessagePreviewChange::Created {
            scope: scope.clone(),
            message: newest.clone(),
            was_already_stored: true,
        };
        let Ok(preview) = self.preview_hook.call(change, self.next_version()).await else {
            return;
        };
        if preview.mutations.is_empty() && preview.invalidated_views.is_empty() {
            return;
        }
        let _ = self.engine.write(preview).await;
    }

    pub async fn load_initial_restoring(
        &self,
        scope: &RiseMessageScope,
        anchor: Option<RiseMessageAnchor>,
        configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseMessagePageLoadOutcome> {
        Ok(self.load_initial_receipt(scope, anchor, configuration).await?.outcome)
    }

    pub async fn load_initial_receipt(
        &self,
        scope: &RiseMessageScope,
        anchor: Option<RiseMessageAnchor>,
        configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        let Some(anchor) = anchor else {
            return self.load_initial_latest_receipt(scope).await;
        };
        match self.jump_receipt_publishing(scope, anchor, configuration, false).await {
            Ok(receipt) => Ok(receipt),
            Err(error) if error.is::<CancellationError>() => Err(CancellationError.into()),
            Err(error) => {
                let latest = self.load_initial_latest_receipt(scope).await?;
                Ok(RiseMessagePageLoadReceipt {
                    used_latest_fallback: true,
                    anchor_resolution_failed_authoritatively: Self::reports_missing_anchor(&error),
                    resolved_anchor_id: None,
                    ..latest
                })
            }
        }
    }

    pub async fn prefetch_page(&self, scope: &RiseMessageScope, direction: Option<RiseMessagePageDirection>) {
        let Some(direction) = direction else {
            self.cancel_page_prefetch(Some(scope));
            return;
        };
        let lane = Self::lane_of(direction);
        let key = RiseMessageRepositoryRequestKey::new(scope, lane);
        let begun = self.state.with(|state| {
            if !state.known_scopes.contains(scope) || state.loading.contains(&key) {
                return None;
            }
            if state.page_prefetch.as_ref().is_none_or(|prefetch| prefetch.key != key)
                && let Some(prefetch) = state.page_prefetch.take()
            {
                prefetch.task.abort();
            }
            state.prefetch_intent_generation = state.prefetch_intent_generation.wrapping_add(1);
            state.prefetch_intent_scope = Some(scope.clone());
            Some((state.prefetch_intent_generation, state.seed_generations.get(scope).copied().unwrap_or(0)))
        });
        let Some((intent, seed_generation)) = begun else {
            return;
        };
        let Ok(Some(view_state)) = self.view_state(scope).await else {
            return;
        };
        let is_older = direction == RiseMessagePageDirection::Older;
        let has_more = if is_older { view_state.has_more_before } else { view_state.has_more_after };
        let cursor_data = if is_older { view_state.before_cursor.as_deref() } else { view_state.after_cursor.as_deref() };
        let Some(cursor) = self.decode(cursor_data).filter(|cursor| !cursor.is_empty()) else {
            return;
        };
        let version = self.next_version_floor(view_state.revision);
        let remote = self.remote.clone();
        let body = rise_request::message::GetMessages {
            chat_id: Some(scope.chat_id.clone()),
            limit: Some(self.page_limit),
            relative_id: Some(cursor.clone()),
            thread_root_id: scope.thread_root_id.clone(),
            topic_id: scope.topic_id.clone(),
            up: Some(direction.wire_up()),
            user_chat_id: None,
        };
        self.state.with(|state| {
            if state.prefetch_intent_generation != intent
                || state.seed_generations.get(scope).copied().unwrap_or(0) != seed_generation
                || !state.known_scopes.contains(scope)
                || state.loading.contains(&key)
                || !has_more
            {
                return;
            }
            if let Some(current) = state.page_prefetch.as_ref()
                && current.key == key
                && current.cursor == cursor
                && current.seed_generation == seed_generation
            {
                return;
            }
            if let Some(prefetch) = state.page_prefetch.take() {
                prefetch.task.abort();
            }
            let task = RiseAccountRuntime::current().spawn(async move { remote.messages(body).await });
            state.page_prefetch = Some(RiseMessageRepositoryPagePrefetch {
                key: key.clone(),
                cursor: cursor.clone(),
                seed_generation,
                version,
                task,
            });
        });
    }

    fn cancel_page_prefetch(&self, scope: Option<&RiseMessageScope>) {
        self.state.with(|state| {
            if scope.is_some() && state.prefetch_intent_scope.as_ref() != scope {
                return;
            }
            state.prefetch_intent_generation = state.prefetch_intent_generation.wrapping_add(1);
            state.prefetch_intent_scope = None;
            if let Some(prefetch) = state.page_prefetch.take() {
                prefetch.task.abort();
            }
        });
    }

    async fn page_response(
        &self,
        context: &RiseMessageRepositoryRequestContext,
        direction: RiseMessagePageDirection,
        cursor: &str,
    ) -> anyhow::Result<(GetMessagesGatewayDTO, i64)> {
        let claimed = self.state.with(|state| {
            let matches = state.page_prefetch.as_ref().is_some_and(|prefetch| {
                prefetch.key == context.key
                    && prefetch.cursor == cursor
                    && prefetch.seed_generation == context.seed_generation
            });
            if !matches {
                return None;
            }
            state.prefetch_intent_scope = None;
            state.prefetch_intent_generation = state.prefetch_intent_generation.wrapping_add(1);
            state.page_prefetch.take()
        });
        if let Some(prefetch) = claimed {
            let _abort = RiseMessagePrefetchAbortOnDrop(prefetch.task.abort_handle());
            if let Ok(Ok(response)) = prefetch.task.await {
                return Ok((response, prefetch.version));
            }
            if !self.is_current(context) {
                return Err(CancellationError.into());
            }
        }
        let scope = &context.key.scope;
        let response = self
            .remote
            .messages(rise_request::message::GetMessages {
                chat_id: Some(scope.chat_id.clone()),
                limit: Some(self.page_limit),
                relative_id: Some(cursor.to_owned()),
                thread_root_id: scope.thread_root_id.clone(),
                topic_id: scope.topic_id.clone(),
                up: Some(direction.wire_up()),
                user_chat_id: None,
            })
            .await?;
        Ok((response, context.version))
    }

    pub async fn load_page(
        &self,
        scope: &RiseMessageScope,
        direction: RiseMessagePageDirection,
    ) -> anyhow::Result<RiseMessagePageLoadOutcome> {
        Ok(self.load_page_receipt(scope, direction).await?.outcome)
    }

    pub async fn load_page_receipt(
        &self,
        scope: &RiseMessageScope,
        direction: RiseMessagePageDirection,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        self.state.with(|state| state.known_scopes.insert(scope.clone()));
        let view_state = self.view_state(scope).await?;
        let is_older = direction == RiseMessagePageDirection::Older;
        let has_more = view_state.as_ref().map(|state| if is_older { state.has_more_before } else { state.has_more_after });
        if has_more == Some(false) {
            tracing::warn!(target: "riseonly::X2021", chat = %scope.chat_id, older = is_older, "page end reached by view state");
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::EndReached, None));
        }
        let cursor_data = view_state
            .as_ref()
            .and_then(|state| if is_older { state.before_cursor.as_deref() } else { state.after_cursor.as_deref() });
        let Some(cursor) = RiseConversationCodec::decode_cursor(cursor_data).filter(|cursor| !cursor.is_empty()) else {
            tracing::warn!(target: "riseonly::X2021", chat = %scope.chat_id, older = is_older, has_more = ?has_more, "page without cursor");
            if has_more == Some(true) {
                return Err(RiseMessageRepositoryError::MissingCursor(direction).into());
            }
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::EndReached, None));
        };
        let Some(context) = self.begin_request(scope, Self::lane_of(direction), Some(&cursor)).await? else {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::AlreadyLoading, None));
        };
        let mut guard = RiseMessageRequestGuard::new(self, &context);
        let outcome = self.load_page_committed(scope, &context, direction, &cursor).await;
        self.settle_request(&mut guard, view_state.as_ref(), outcome).await
    }

    async fn load_page_committed(
        &self,
        scope: &RiseMessageScope,
        context: &RiseMessageRepositoryRequestContext,
        direction: RiseMessagePageDirection,
        cursor: &str,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        let (mut response, page_version) = self.page_response(context, direction, cursor).await?;
        if !self.cursor_still_matches(context, direction).await? || !self.is_current(context) {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::StaleResponseDropped, None));
        }
        self.validate(response.success, response.error.as_deref())?;
        let messages = RiseMessagePageContract::require_messages(response.messages.take())?;
        let rows = self.encode_rows(&messages, page_version).await?;
        let next_raw = self
            .normalized(response.relative_id.as_deref())
            .or_else(|| self.boundary_cursor(&messages, direction));
        let next = self.validated_cursor(next_raw.as_deref(), response.is_have_more.unwrap_or(false), Some(cursor));
        tracing::warn!(
            target: "riseonly::X2021",
            chat = %context.key.scope.chat_id,
            ?direction,
            requested = cursor,
            rows = messages.len(),
            wire_more = ?response.is_have_more,
            wire_cursor = ?response.relative_id,
            next_cursor = ?next.cursor,
            next_more = next.has_more,
            stalled = next.stalled,
            "message page response"
        );
        let current = self.view_state(scope).await?;
        let is_older = direction == RiseMessagePageDirection::Older;
        let mut before = if is_older {
            next.cursor.clone()
        } else {
            self.decode(current.as_ref().and_then(|state| state.before_cursor.as_deref()))
        };
        let mut after = if is_older {
            self.decode(current.as_ref().and_then(|state| state.after_cursor.as_deref()))
        } else {
            next.cursor.clone()
        };
        let mut has_before = if is_older { next.has_more } else { current.as_ref().is_some_and(|state| state.has_more_before) };
        let mut has_after = if is_older { current.as_ref().is_some_and(|state| state.has_more_after) } else { next.has_more };
        let cached = self.preserved_memberships(scope).await?;
        let mut merged: HashMap<&str, (&RisePositionKey, Option<&str>)> = HashMap::with_capacity(cached.len() + rows.len());
        for item in &cached {
            merged.insert(item.local_id.as_str(), (&item.position, item.server_id.as_deref()));
        }
        let server_id_kind = RiseEntityAliasKind::server_id();
        for row in &rows {
            merged.insert(
                row.local_id.as_str(),
                (
                    &row.position,
                    row.aliases.iter().find(|alias| alias.kind == server_id_kind).map(|alias| alias.value.as_str()),
                ),
            );
        }
        let mut ordered: Vec<_> = merged.into_iter().collect();
        ordered.sort_by(|left, right| left.1.0.cmp(right.1.0).then_with(|| left.0.cmp(right.0)));
        let retention = self.retention_limit as usize;
        if ordered.len() > retention {
            if is_older {
                let boundary = ordered[..retention].iter().rev().find_map(|(_, value)| value.1);
                if let Some(boundary) = boundary {
                    after = Some(boundary.to_owned());
                    has_after = true;
                }
            } else {
                let boundary = ordered[ordered.len() - retention..].iter().find_map(|(_, value)| value.1);
                if let Some(boundary) = boundary {
                    before = Some(boundary.to_owned());
                    has_before = true;
                }
            }
        }
        let trim = if is_older { RiseEntityViewRetention::First(retention) } else { RiseEntityViewRetention::Latest(retention) };
        if !self.cursor_still_matches(context, direction).await? || !self.is_current(context) {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::StaleResponseDropped, None));
        }
        let batch = self.make_page_batch(
            scope,
            &rows,
            &[],
            &self.membership_versions(&cached),
            page_version,
            false,
            before.as_deref(),
            after.as_deref(),
            has_before,
            has_after,
            trim,
        );
        let commit = self.engine.write(batch).await?;
        let outcome = if next.stalled {
            RiseMessagePageLoadOutcome::CursorStalled
        } else {
            RiseMessagePageLoadOutcome::Loaded { count: rows.len() as i64 }
        };
        Ok(RiseMessagePageLoadReceipt {
            entity_versions_by_local_id: self.debug_entity_versions(&rows, page_version),
            ..Self::receipt(outcome, Some(commit.revision))
        })
    }

    pub async fn jump(
        &self,
        scope: &RiseMessageScope,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseMessagePageLoadOutcome> {
        Ok(self.jump_receipt(scope, anchor, configuration).await?.outcome)
    }

    pub async fn jump_receipt(
        &self,
        scope: &RiseMessageScope,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        self.jump_receipt_publishing(scope, anchor, configuration, true).await
    }

    /// jump to a receipt, publishes_failure false keeps the failure out of the view state
    /// for a caller with its own fallback
    pub async fn jump_receipt_publishing(
        &self,
        scope: &RiseMessageScope,
        anchor: RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
        publishes_failure: bool,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        self.state.with(|state| state.known_scopes.insert(scope.clone()));
        let anchor_id = self.remote_anchor_id(&anchor).await?;
        self.jump_receipt_resolved(scope, Some(anchor_id), None, configuration, publishes_failure).await
    }

    pub async fn jump_receipt_at_date(
        &self,
        scope: &RiseMessageScope,
        day_start_ms: i64,
        configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        self.state.with(|state| state.known_scopes.insert(scope.clone()));
        self.jump_receipt_resolved(scope, None, Some(day_start_ms), configuration, true).await
    }

    async fn jump_receipt_resolved(
        &self,
        scope: &RiseMessageScope,
        anchor_id: Option<String>,
        anchor_at_ms: Option<i64>,
        configuration: RiseMessageWindowConfiguration,
        publishes_failure: bool,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        let request_key = anchor_id.clone().unwrap_or_else(|| format!("date:{}", anchor_at_ms.unwrap_or(0)));
        self.state.with(|state| {
            state.loading.remove(&RiseMessageRepositoryRequestKey::new(scope, RiseMessageRepositoryRequestLane::Seed))
        });
        self.cancel_page_prefetch(Some(scope));
        self.state.with(|state| Self::bump_seed_generation(state, scope));
        let Some(context) =
            self.begin_request(scope, RiseMessageRepositoryRequestLane::Seed, Some(&request_key)).await?
        else {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::AlreadyLoading, None));
        };
        let mut guard = RiseMessageRequestGuard::new(self, &context);
        let previous = self.view_state(scope).await?;
        let outcome = self
            .jump_window_committed(scope, &context, anchor_id, anchor_at_ms, configuration, &request_key)
            .await;
        if !publishes_failure && outcome.is_err() {
            guard.writes_cancellation = false;
            return outcome;
        }
        self.settle_request(&mut guard, previous.as_ref(), outcome).await
    }

    async fn jump_window_committed(
        &self,
        scope: &RiseMessageScope,
        context: &RiseMessageRepositoryRequestContext,
        anchor_id: Option<String>,
        anchor_at_ms: Option<i64>,
        configuration: RiseMessageWindowConfiguration,
        request_key: &str,
    ) -> anyhow::Result<RiseMessagePageLoadReceipt> {
        let side_limit = Self::around_side_limit();
        let mut response = self
            .message_window_response(
                rise_request::message::GetMessagesAround {
                    after_limit: Some(configuration.after.clamp(1, side_limit)),
                    anchor: anchor_id.clone(),
                    anchor_at_ms,
                    before_limit: Some(configuration.before.clamp(1, side_limit)),
                    chat_id: Some(scope.chat_id.clone()),
                    thread_root_id: scope.thread_root_id.clone(),
                    topic_id: scope.topic_id.clone(),
                    user_chat_id: None,
                },
                context,
            )
            .await?;
        if !self.is_current(context) {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::StaleResponseDropped, None));
        }
        self.validate(response.success, response.error.as_deref())?;
        let messages = RiseMessagePageContract::require_messages(response.messages.take())?;
        let Some(resolved_anchor_id) = anchor_id.or_else(|| self.normalized(response.anchor_message_id.as_deref())) else {
            return Err(RiseMessageRepositoryError::AroundResponseMissingAnchor(request_key.to_owned()).into());
        };
        if !self.page(&messages, &resolved_anchor_id) {
            return Err(RiseMessageRepositoryError::AroundResponseMissingAnchor(resolved_anchor_id).into());
        }
        let rows = self.encode_rows(&messages, context.version).await?;
        if !self.is_current(context) {
            return Ok(Self::receipt(RiseMessagePageLoadOutcome::StaleResponseDropped, None));
        }
        let older_raw = self.normalized(response.older_relative_id.as_deref());
        let older = self.validated_cursor(older_raw.as_deref(), response.has_older.unwrap_or(false), None);
        let newer_raw = self.normalized(response.newer_relative_id.as_deref());
        let newer = self.validated_cursor(newer_raw.as_deref(), response.has_newer.unwrap_or(false), None);
        tracing::warn!(
            target: "riseonly::X2021",
            chat = %scope.chat_id,
            anchor = %resolved_anchor_id,
            rows = rows.len(),
            wire_has_older = ?response.has_older,
            older_cursor = ?older.cursor,
            older_more = older.has_more,
            wire_has_newer = ?response.has_newer,
            newer_more = newer.has_more,
            "message jump window"
        );
        let cached = self.preserved_memberships(scope).await?;
        let mut preserved_rows: Vec<_> = cached.iter().filter(|item| item.is_local).cloned().collect();
        if !newer.has_more {
            preserved_rows.extend(self.rows_newer_than_page(rows.last().map(|row| &row.position), &cached));
        }
        let fresh_rows = self.with_fresh_entity_versions(&rows).await?;
        let batch = self.make_page_batch(
            scope,
            &fresh_rows,
            &preserved_rows,
            &self.membership_versions(&cached),
            context.version,
            true,
            older.cursor.as_deref(),
            newer.cursor.as_deref(),
            older.has_more,
            newer.has_more,
            RiseEntityViewRetention::Latest(self.retention_limit as usize),
        );
        let written = self.engine.write(batch).await?;
        let commit = self.restore_refused_page_memberships(scope, &rows, written).await?;
        let outcome = if older.stalled || newer.stalled {
            RiseMessagePageLoadOutcome::CursorStalled
        } else {
            RiseMessagePageLoadOutcome::Loaded { count: rows.len() as i64 }
        };
        Ok(RiseMessagePageLoadReceipt {
            entity_versions_by_local_id: self.debug_entity_versions(&rows, context.version),
            resolved_anchor_id: Some(resolved_anchor_id),
            ..Self::receipt(outcome, Some(commit.revision))
        })
    }

    /// filtered history can spend the server scan budget without finding a visible row
    /// continues the cursor under the same request, an empty seed has no edge to ask from
    async fn first_nonempty_message_page(
        &self,
        body: rise_request::message::GetMessages,
        context: &RiseMessageRepositoryRequestContext,
    ) -> anyhow::Result<GetMessagesGatewayDTO> {
        let mut request = body.clone();
        loop {
            if !self.is_current(context) {
                return Err(CancellationError.into());
            }
            let mut response = self.remote.messages(request.clone()).await?;
            if !self.is_current(context) {
                return Err(CancellationError.into());
            }
            self.validate(response.success, response.error.as_deref())?;
            let rows = RiseMessagePageContract::require_messages(response.messages.take())?;
            let is_empty = rows.is_empty();
            response.messages = Some(rows);
            let cursor = self.normalized(response.relative_id.as_deref());
            let Some(cursor) = cursor.filter(|cursor| {
                is_empty && response.is_have_more == Some(true) && request.relative_id.as_deref() != Some(cursor.as_str())
            }) else {
                return Ok(response);
            };
            request = rise_request::message::GetMessages {
                chat_id: body.chat_id.clone(),
                limit: body.limit,
                relative_id: Some(cursor),
                thread_root_id: body.thread_root_id.clone(),
                topic_id: body.topic_id.clone(),
                up: Some(body.relative_id.is_none() || body.up == Some(true)),
                user_chat_id: body.user_chat_id.clone(),
            };
        }
    }

    async fn message_window_response(
        &self,
        body: rise_request::message::GetMessagesAround,
        context: &RiseMessageRepositoryRequestContext,
    ) -> anyhow::Result<GetMessagesAroundGatewayDTO> {
        if body.topic_id.as_deref() != Some("") {
            return self.remote.messages_around(body).await;
        }
        let mut anchor_id = body.anchor.clone();
        if anchor_id.is_none()
            && let Some(date) = body.anchor_at_ms
        {
            let mut page = self
                .first_nonempty_message_page(
                    rise_request::message::GetMessages {
                        chat_id: body.chat_id.clone(),
                        limit: Some(1),
                        relative_id: Some(date.saturating_sub(1).to_string()),
                        thread_root_id: body.thread_root_id.clone(),
                        topic_id: Some(String::new()),
                        up: Some(false),
                        user_chat_id: body.user_chat_id.clone(),
                    },
                    context,
                )
                .await?;
            self.validate(page.success, page.error.as_deref())?;
            let rows = RiseMessagePageContract::require_messages(page.messages.take())?;
            anchor_id = self.boundary_cursor(&rows, RiseMessagePageDirection::Older);
        }
        if !self.is_current(context) {
            return Err(CancellationError.into());
        }
        let Some(anchor_id) = anchor_id else {
            return Err(RiseMessageRepositoryError::InvalidWirePayload.into());
        };
        let mut anchor_page = self
            .remote
            .messages_around(rise_request::message::GetMessagesAround {
                after_limit: Some(1),
                anchor: Some(anchor_id.clone()),
                anchor_at_ms: None,
                before_limit: Some(1),
                chat_id: body.chat_id.clone(),
                thread_root_id: body.thread_root_id.clone(),
                topic_id: None,
                user_chat_id: body.user_chat_id.clone(),
            })
            .await?;
        if !self.is_current(context) {
            return Err(CancellationError.into());
        }
        self.validate(anchor_page.success, anchor_page.error.as_deref())?;
        let anchor_rows = RiseMessagePageContract::require_messages(anchor_page.messages.take())?;
        let Some(anchor) = anchor_rows.into_iter().find(|row| self.page(std::slice::from_ref(row), &anchor_id)) else {
            return Err(RiseMessageRepositoryError::AroundResponseMissingAnchor(anchor_id).into());
        };
        if self.normalized(anchor.topic_id.as_deref()).is_some() {
            return Err(RiseMessageRepositoryError::AroundResponseMissingAnchor(anchor_id).into());
        }
        let cursor = self.normalized(anchor_page.anchor_relative_id.as_deref()).unwrap_or_else(|| anchor_id.clone());
        let side_body = |limit: Option<i64>, up: bool| rise_request::message::GetMessages {
            chat_id: body.chat_id.clone(),
            limit,
            relative_id: Some(cursor.clone()),
            thread_root_id: body.thread_root_id.clone(),
            topic_id: Some(String::new()),
            up: Some(up),
            user_chat_id: body.user_chat_id.clone(),
        };
        let (mut older, mut newer) = try_join(
            self.remote.messages(side_body(body.before_limit, true)),
            self.remote.messages(side_body(body.after_limit, false)),
        )
        .await?;
        if !self.is_current(context) {
            return Err(CancellationError.into());
        }
        self.validate(older.success, older.error.as_deref())?;
        self.validate(newer.success, newer.error.as_deref())?;
        let older_rows = RiseMessagePageContract::require_messages(older.messages.take())?;
        let newer_rows = RiseMessagePageContract::require_messages(newer.messages.take())?;
        let older_relative_id = self
            .normalized(older.relative_id.as_deref())
            .or_else(|| self.boundary_cursor(&older_rows, RiseMessagePageDirection::Older));
        let newer_relative_id = self
            .normalized(newer.relative_id.as_deref())
            .or_else(|| self.boundary_cursor(&newer_rows, RiseMessagePageDirection::Newer));
        let mut messages = older_rows;
        messages.push(anchor);
        messages.extend(newer_rows);
        Ok(GetMessagesAroundGatewayDTO {
            success: Some(true),
            error: None,
            chat_id: body.chat_id,
            anchor_message_id: Some(anchor_id),
            anchor_relative_id: Some(cursor),
            before_limit: body.before_limit,
            after_limit: body.after_limit,
            older_relative_id,
            has_older: older.is_have_more,
            newer_relative_id,
            has_newer: newer.is_have_more,
            messages: Some(messages),
        })
    }

    pub async fn typing_updates(&self, scope: &RiseMessageScope) -> RiseMessageTypingStream {
        self.state.with(|state| state.known_scopes.insert(scope.clone()));
        self.typing_hub.stream(scope, false).await
    }

    pub async fn upload_progress_updates(&self, key: RiseUploadKey) -> RiseUploadProgressStream<Option<RiseUploadProgress>> {
        self.upload_progress_hub.updates_for(key).await
    }

    pub async fn forget_topic_rows(
        &self,
        raw_chat_id: &str,
        raw_topic_ids: HashSet<String>,
    ) -> anyhow::Result<Option<MessageWireDTO>> {
        let chat_id = self.require_account_chat_id(raw_chat_id)?;
        let topic_ids: HashSet<String> =
            raw_topic_ids.iter().filter_map(|topic_id| self.normalized(Some(topic_id))).collect();
        if topic_ids.is_empty() {
            return Ok(None);
        }
        let namespace = RiseConversationStoreLayout::messages();
        let view = RiseConversationStoreLayout::message_view(&RiseMessageScope::new(chat_id, None, None));
        let result = self
            .engine
            .entity_window(RiseEntityWindowQuery::new(
                namespace.clone(),
                view.clone(),
                RiseEntityWindow::Latest { limit: self.retention_limit as usize },
            ))
            .await?;
        let version = self.next_version();
        let mut newest_visible: Option<(f64, MessageWireDTO)> = None;
        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        for item in &result.items {
            let Ok(record) = self.decoded_record(&item.entity) else {
                continue;
            };
            let hidden = self
                .normalized(record.wire.topic_id.as_deref())
                .is_some_and(|topic_id| topic_ids.contains(&topic_id));
            if hidden {
                mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                    namespace: namespace.clone(),
                    view: view.clone(),
                    local_id: item.entity.local_id.clone(),
                    if_version_at_most: Some(version),
                });
                continue;
            }
            if record.local_phase != RiseMessageLocalPhase::Remote || Self::is_device_only_row(&record.wire) {
                continue;
            }
            let timestamp = RiseConversationStoreLayout::normalized_timestamp(
                record.wire.created_at.or(record.wire.timestamp).unwrap_or(0.0),
            );
            if newest_visible.as_ref().is_none_or(|(newest, _)| timestamp > *newest) {
                newest_visible = Some((timestamp, record.wire.clone()));
            }
        }
        self.evict_decoded_cache_if_needed();
        if !mutations.is_empty() {
            self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        }
        Ok(newest_visible.map(|(_, wire)| wire))
    }

    /// deleting the last message left the row with no preview until the list read came back
    /// the previous message is already in the feed so the row takes it now
    pub(crate) async fn newest_preview_fallback(
        &self,
        raw_chat_id: &str,
        excluded: &HashSet<String>,
    ) -> Option<MessageWireDTO> {
        let chat_id = self.require_account_chat_id(raw_chat_id).ok()?;
        let limit = (excluded.len() + 32).min(self.retention_limit.max(1) as usize);
        let result = self
            .engine
            .entity_window(RiseEntityWindowQuery::new(
                RiseConversationStoreLayout::messages(),
                RiseConversationStoreLayout::message_view(&RiseMessageScope::new(chat_id, None, None)),
                RiseEntityWindow::Latest { limit },
            ))
            .await
            .ok()?;
        let mut newest: Option<(f64, MessageWireDTO)> = None;
        for item in &result.items {
            if excluded.contains(&item.entity.local_id) {
                continue;
            }
            let Ok(record) = self.decoded_record(&item.entity) else {
                continue;
            };
            if record.local_phase != RiseMessageLocalPhase::Remote || Self::is_device_only_row(&record.wire) {
                continue;
            }
            if RiseConversationStoreLayout::message_aliases(&record.wire)
                .iter()
                .any(|alias| excluded.contains(&alias.value))
            {
                continue;
            }
            let timestamp = RiseConversationStoreLayout::normalized_timestamp(
                record.wire.created_at.or(record.wire.timestamp).unwrap_or(0.0),
            );
            if newest.as_ref().is_none_or(|(time, _)| timestamp > *time) {
                newest = Some((timestamp, record.wire.clone()));
            }
        }
        self.evict_decoded_cache_if_needed();
        newest.map(|(_, wire)| wire)
    }

    fn require_account_chat_id(&self, raw_chat_id: &str) -> anyhow::Result<String> {
        if self.engine.account.account_id != self.account_id {
            return Err(RiseMessageRepositoryError::InvalidScope.into());
        }
        self.normalized(Some(raw_chat_id)).ok_or_else(|| RiseMessageRepositoryError::InvalidScope.into())
    }

    fn bump_seed_generation(state: &mut RiseMessageRepositoryState, scope: &RiseMessageScope) {
        let generation = state.seed_generations.entry(scope.clone()).or_insert(0);
        *generation = generation.wrapping_add(1);
    }

    fn bump_request_token(state: &mut RiseMessageRepositoryState, key: &RiseMessageRepositoryRequestKey) {
        let token = state.request_tokens.entry(key.clone()).or_insert(0);
        *token = token.wrapping_add(1);
    }

    /// every in flight request of the scope becomes stale
    fn retire_scope_requests(&self, scope: &RiseMessageScope) {
        self.cancel_page_prefetch(Some(scope));
        self.state.with(|state| {
            Self::bump_seed_generation(state, scope);
            for lane in RiseMessageRepositoryRequestLane::ALL {
                let key = RiseMessageRepositoryRequestKey::new(scope, lane);
                Self::bump_request_token(state, &key);
                state.loading.remove(&key);
            }
        });
    }

    fn evict_decoded_records_of_chat(&self, chat_id: &str) {
        self.state.with(|state| {
            state.decoded_record_cache.retain(|_, entry| {
                RiseMessageWireCodec::normalized(entry.record.wire.chat_id.as_deref()).as_deref() != Some(chat_id)
            });
        });
    }

    /// clears the local view of one chat history without tombstoning the chat
    /// timeline and cursors and preview and projections go in one store batch
    pub async fn clear_local_history(&self, raw_chat_id: &str) -> anyhow::Result<RiseMessageHistoryClearReceipt> {
        let chat_id = self.require_account_chat_id(raw_chat_id)?;
        let scopes = self.state.with(|state| {
            let mut scopes: HashSet<RiseMessageScope> =
                state.known_scopes.iter().filter(|scope| scope.chat_id == chat_id).cloned().collect();
            scopes.insert(RiseMessageScope::new(chat_id.clone(), None, None));
            scopes.insert(RiseMessageScope::new(chat_id.clone(), Some(String::new()), None));
            state.known_scopes.extend(scopes.iter().cloned());
            scopes
        });
        for scope in &scopes {
            self.retire_scope_requests(scope);
        }

        let version = self.next_version();
        let now = Self::now_milliseconds();
        let namespace = RiseConversationStoreLayout::messages();
        let mut timeline_mutations: Vec<RiseStoreMutation> = Vec::with_capacity(scopes.len() * 2 + 1);
        let mut ordered_scopes: Vec<&RiseMessageScope> = scopes.iter().collect();
        ordered_scopes.sort_by_key(|scope| scope.storage_key());
        for scope in ordered_scopes {
            let view = RiseConversationStoreLayout::message_view(scope);
            timeline_mutations.push(RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace: namespace.clone(),
                view: view.clone(),
                if_version_at_most: None,
            });
            timeline_mutations.push(RiseStoreMutation::SetEntityViewState {
                namespace: namespace.clone(),
                view,
                state: RiseEntityViewState::new(version, None, None, false, false, RiseEntityViewLoadState::Loaded, None, now),
            });
        }

        let topic_ids: HashSet<String> = scopes.iter().filter_map(|scope| scope.topic_id.clone()).collect();
        let projection_batch = self
            .auxiliary_invalidation_batch(RiseMessageAuxiliaryInvalidation::with_topic_ids(
                &chat_id,
                topic_ids,
                RiseMessageAuxiliaryProjection::all(),
            ))
            .await;
        let preview_batch = self
            .preview_hook
            .call(RiseMessagePreviewChange::HistoryCleared { chat_id: chat_id.clone() }, version)
            .await?;
        timeline_mutations.extend(preview_batch.mutations);
        timeline_mutations.extend(projection_batch.mutations);
        timeline_mutations.push(RiseStoreMutation::PruneUnreferencedEntities {
            namespace,
            limit: self.retention_limit,
        });
        let mut invalidated_views = preview_batch.invalidated_views;
        invalidated_views.extend(projection_batch.invalidated_views);
        let commit = self.engine.write(RiseStoreWriteBatch::new(timeline_mutations, invalidated_views)).await?;
        self.evict_decoded_records_of_chat(&chat_id);
        Ok(RiseMessageHistoryClearReceipt { scopes, store_revision: commit.revision })
    }

    pub async fn cached_history_bytes_by_chat(&self) -> anyhow::Result<HashMap<String, i64>> {
        if self.engine.account.account_id != self.account_id {
            return Err(RiseMessageRepositoryError::InvalidScope.into());
        }
        let by_view = self.cached_history_view_bytes().await?;
        let mut result: HashMap<String, i64> = HashMap::new();
        for (view, bytes) in by_view {
            if bytes <= 0 {
                continue;
            }
            let Some(scope) = RiseConversationStoreLayout::message_scope(&view) else {
                continue;
            };
            *result.entry(scope.chat_id).or_insert(0) += bytes;
        }
        Ok(result)
    }

    /// stored bytes per message view, one scan serves a whole batch of evictions
    pub async fn cached_history_view_bytes(&self) -> anyhow::Result<HashMap<String, i64>> {
        if self.engine.account.account_id != self.account_id {
            return Err(RiseMessageRepositoryError::InvalidScope.into());
        }
        self.engine.entity_view_payload_bytes(RiseConversationStoreLayout::messages()).await
    }

    pub async fn evict_cached_history_scanning(&self, raw_chat_id: &str) -> anyhow::Result<RiseMessageHistoryClearReceipt> {
        let chat_id = self.require_account_chat_id(raw_chat_id)?;
        let by_view = self.cached_history_view_bytes().await?;
        self.evict_cached_history(&chat_id, &by_view).await
    }

    /// storage screen eviction, drops cached pages of one chat but leaves the preview alone
    /// every view is marked as having older rows so the next open loads from the server
    pub async fn evict_cached_history(
        &self,
        raw_chat_id: &str,
        view_bytes: &HashMap<String, i64>,
    ) -> anyhow::Result<RiseMessageHistoryClearReceipt> {
        let chat_id = self.require_account_chat_id(raw_chat_id)?;
        let mut scopes: HashSet<RiseMessageScope> = self.state.with(|state| {
            state.known_scopes.iter().filter(|scope| scope.chat_id == chat_id).cloned().collect()
        });
        scopes.insert(RiseMessageScope::new(chat_id.clone(), None, None));
        for view in view_bytes.keys() {
            if let Some(scope) = RiseConversationStoreLayout::message_scope(view)
                && scope.chat_id == chat_id
            {
                scopes.insert(scope);
            }
        }
        for scope in &scopes {
            self.retire_scope_requests(scope);
        }

        let version = self.next_version();
        let now = Self::now_milliseconds();
        let namespace = RiseConversationStoreLayout::messages();
        let mut mutations: Vec<RiseStoreMutation> = Vec::with_capacity(scopes.len() * 2 + 1);
        let mut ordered_scopes: Vec<&RiseMessageScope> = scopes.iter().collect();
        ordered_scopes.sort_by_key(|scope| scope.storage_key());
        for scope in ordered_scopes {
            let view = RiseConversationStoreLayout::message_view(scope);
            let retained = self.engine.retained_entity_view_positions(namespace.clone(), view.clone()).await?;
            mutations.push(RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace: namespace.clone(),
                view: view.clone(),
                if_version_at_most: Some(version),
            });
            for position in retained {
                mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                    namespace: namespace.clone(),
                    view: view.clone(),
                    membership: RiseEntityViewMembership::new(position.local_id, position.position_key, version),
                });
            }
            mutations.push(RiseStoreMutation::SetEntityViewState {
                namespace: namespace.clone(),
                view,
                state: RiseEntityViewState::new(version, None, None, true, false, RiseEntityViewLoadState::Idle, None, now),
            });
        }
        mutations.push(RiseStoreMutation::PruneUnreferencedEntities { namespace, limit: self.retention_limit });
        let commit = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        self.evict_decoded_records_of_chat(&chat_id);
        Ok(RiseMessageHistoryClearReceipt { scopes, store_revision: commit.revision })
    }

    /// drops only the live presentation ownership for a closed screen
    /// rows stay in the store and reopen cache first
    pub async fn release(&self, scope: &RiseMessageScope) {
        self.state.with(|state| state.known_scopes.remove(scope));
        self.retire_scope_requests(scope);
        self.typing_hub.release(scope).await;
    }

    pub fn cancel_seed_request(&self, scope: &RiseMessageScope) {
        self.cancel_page_prefetch(Some(scope));
        let key = RiseMessageRepositoryRequestKey::new(scope, RiseMessageRepositoryRequestLane::Seed);
        self.state.with(|state| {
            Self::bump_seed_generation(state, scope);
            Self::bump_request_token(state, &key);
            state.loading.remove(&key);
        });
    }

    /// waits without polling until the durable create reaches an authoritative entity or a terminal failure
    /// keyed by the permanent local id so temp to server aliasing cant detach it
    pub async fn wait_for_submission_resolution(&self, local_id: &str) -> anyhow::Result<()> {
        let key = RiseEntityKey::new(RiseConversationStoreLayout::messages(), local_id);
        let request = RiseStoreSnapshotRequest {
            entities: HashSet::from([key.clone()]),
            ..RiseStoreSnapshotRequest::default()
        };
        let observation = self.engine.observe(request.clone()).await?;
        if let Some(result) = Self::submission_resolution(&observation.seed, &key) {
            return result;
        }
        let mut updates = observation.updates.make_async_iterator();
        while updates.next().await.is_some() {
            let snapshot = self.engine.snapshot(request.clone()).await?;
            if let Some(result) = Self::submission_resolution(&snapshot, &key) {
                return result;
            }
        }
        Err(CancellationError.into())
    }

    fn submission_resolution(snapshot: &RiseStoreSnapshot, key: &RiseEntityKey) -> Option<anyhow::Result<()>> {
        let entity = snapshot.entities.get(key)?;
        let record = RiseMessageWireCodec::record(&entity.data).ok()?;
        match record.local_phase {
            RiseMessageLocalPhase::Remote => Some(Ok(())),
            RiseMessageLocalPhase::Failed => Some(Err(RiseMessageRepositoryError::RemoteFailure(
                record.last_error_code.unwrap_or_else(|| Self::CREATE_MESSAGE_FAILED_ERROR_CODE.to_owned()),
            )
            .into())),
            RiseMessageLocalPhase::Queued | RiseMessageLocalPhase::Uploading | RiseMessageLocalPhase::Sending => None,
        }
    }

    pub async fn shutdown(&self) {
        self.cancel_page_prefetch(None);
        self.state.with(|state| {
            state.request_tokens.clear();
            state.seed_generations.clear();
            state.loading.clear();
            state.known_scopes.clear();
            state.decoded_record_cache = HashMap::new();
        });
        self.typing_hub.shutdown().await;
        self.upload_progress_hub.shutdown().await;
        self.upload_cancellation_hub.shutdown().await;
    }

    fn lane_of(direction: RiseMessagePageDirection) -> RiseMessageRepositoryRequestLane {
        if direction == RiseMessagePageDirection::Older {
            RiseMessageRepositoryRequestLane::Older
        } else {
            RiseMessageRepositoryRequestLane::Newer
        }
    }

    pub(crate) async fn begin_request(
        &self,
        scope: &RiseMessageScope,
        lane: RiseMessageRepositoryRequestLane,
        cursor: Option<&str>,
    ) -> anyhow::Result<Option<RiseMessageRepositoryRequestContext>> {
        let key = RiseMessageRepositoryRequestKey::new(scope, lane);
        if !self.state.with(|state| state.loading.insert(key.clone())) {
            return Ok(None);
        }
        let mut guard = RiseMessageLoadingInsertGuard { repository: self, key: key.clone(), is_armed: true };
        let context = self.begin_inserted_request(key, scope, cursor).await?;
        guard.is_armed = false;
        Ok(Some(context))
    }

    pub(crate) async fn begin_inserted_request(
        &self,
        key: RiseMessageRepositoryRequestKey,
        scope: &RiseMessageScope,
        cursor: Option<&str>,
    ) -> anyhow::Result<RiseMessageRepositoryRequestContext> {
        self.state.with(|state| Self::bump_request_token(state, &key));
        let view_state = self.view_state(scope).await?;
        let version = self.next_version_floor(view_state.as_ref().map_or(0, |state| state.revision));
        let (token, seed_generation) = self.state.with(|state| {
            (
                state.request_tokens.get(&key).copied().unwrap_or(0),
                state.seed_generations.get(scope).copied().unwrap_or(0),
            )
        });
        let context = RiseMessageRepositoryRequestContext {
            key,
            token,
            seed_generation,
            requested_cursor: cursor.map(str::to_owned),
            version,
            started_state: view_state.clone(),
        };
        let mut guard = RiseMessageRequestGuard::new(self, &context);
        guard.finishes_request = false;
        let before_cursor = self.decode(view_state.as_ref().and_then(|state| state.before_cursor.as_deref()));
        let after_cursor = self.decode(view_state.as_ref().and_then(|state| state.after_cursor.as_deref()));
        let error_code = view_state
            .as_ref()
            .filter(|state| {
                state.load_state == RiseEntityViewLoadState::Failed
                    && state.last_error_code.as_deref() != Some(Self::CANCELLED_ERROR_CODE)
            })
            .and_then(|state| state.last_error_code.clone());
        let written = self
            .write_state(
                scope,
                version,
                before_cursor.as_deref(),
                after_cursor.as_deref(),
                view_state.as_ref().is_some_and(|state| state.has_more_before),
                view_state.as_ref().is_some_and(|state| state.has_more_after),
                RiseEntityViewLoadState::Loading,
                error_code,
            )
            .await;
        guard.writes_cancellation = false;
        written?;
        Ok(context)
    }

    pub(crate) fn finish_request(&self, context: &RiseMessageRepositoryRequestContext) {
        self.state.with(|state| {
            if state.request_tokens.get(&context.key) == Some(&context.token) {
                state.loading.remove(&context.key);
            }
        });
    }

    pub(crate) fn is_current(&self, context: &RiseMessageRepositoryRequestContext) -> bool {
        self.state.with(|state| {
            state.request_tokens.get(&context.key) == Some(&context.token)
                && state.seed_generations.get(&context.key.scope).copied().unwrap_or(0) == context.seed_generation
        })
    }

    pub(crate) async fn cursor_still_matches(
        &self,
        context: &RiseMessageRepositoryRequestContext,
        direction: RiseMessagePageDirection,
    ) -> anyhow::Result<bool> {
        let view_state = self.view_state(&context.key.scope).await?;
        let data = view_state.as_ref().and_then(|state| {
            if direction == RiseMessagePageDirection::Older {
                state.before_cursor.as_deref()
            } else {
                state.after_cursor.as_deref()
            }
        });
        Ok(self.decode(data) == context.requested_cursor)
    }

    pub(crate) fn debug_entity_versions(
        &self,
        rows: &[RiseMessageRepositoryEncodedRow],
        version: i64,
    ) -> HashMap<String, i64> {
        if cfg!(debug_assertions) {
            rows.iter().map(|row| (row.local_id.clone(), version)).collect()
        } else {
            HashMap::new()
        }
    }

    pub(crate) async fn encode_rows(
        &self,
        messages: &[MessageWireDTO],
        _version: i64,
    ) -> anyhow::Result<Vec<RiseMessageRepositoryEncodedRow>> {
        let aliases_by_row: Vec<Vec<RiseEntityAlias>> =
            messages.iter().map(RiseConversationStoreLayout::message_aliases).collect();
        let unique_aliases: Vec<RiseEntityAlias> =
            aliases_by_row.iter().flatten().cloned().collect::<HashSet<_>>().into_iter().collect();
        let mut entity_by_alias: HashMap<RiseEntityAlias, RiseStoreEntity> = HashMap::with_capacity(unique_aliases.len());
        for chunk in unique_aliases.chunks(Self::ALIAS_LOOKUP_CHUNK) {
            let lookups: HashSet<RiseEntityAliasLookup> = chunk
                .iter()
                .map(|alias| RiseEntityAliasLookup::new(RiseConversationStoreLayout::messages(), alias.clone()))
                .collect();
            let snapshot = self
                .engine
                .snapshot(RiseStoreSnapshotRequest { entity_aliases: lookups, ..RiseStoreSnapshotRequest::default() })
                .await?;
            self.state.with(|state| {
                for entity in snapshot.entity_aliases.values() {
                    state.version_clock = state.version_clock.max(entity.version);
                }
            });
            for (lookup, entity) in snapshot.entity_aliases {
                entity_by_alias.insert(lookup.alias, entity);
            }
        }

        let mut resolved: Vec<RiseMessageRepositoryResolvedRow> = Vec::with_capacity(messages.len());
        for (index, wire) in messages.iter().enumerate() {
            let aliases = &aliases_by_row[index];
            let mut entities: Vec<&RiseStoreEntity> = Vec::new();
            for alias in aliases {
                if let Some(entity) = entity_by_alias.get(alias)
                    && !entities.iter().any(|known| known.local_id == entity.local_id)
                {
                    entities.push(entity);
                }
            }
            let local_id = self.preferred_local_id(&entities, wire);
            if entities.iter().any(|entity| RiseMessageWireCodec::is_tombstone(&entity.data)) {
                continue;
            }
            resolved.push(RiseMessageRepositoryResolvedRow {
                wire: wire.clone(),
                aliases: aliases.clone(),
                duplicate_local_ids: entities
                    .iter()
                    .filter(|entity| entity.local_id != local_id)
                    .map(|entity| entity.local_id.clone())
                    .collect(),
                entity_version: entities.iter().map(|entity| entity.version).max().unwrap_or(0),
                existing_data: entities.iter().find(|entity| entity.local_id == local_id).map(|entity| entity.data.clone()),
                local_id,
            });
        }
        let encoded: Vec<RiseMessageRepositoryEncodedRow> = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                resolved
                    .into_iter()
                    .map(|row| -> anyhow::Result<RiseMessageRepositoryEncodedRow> {
                        let position = RiseConversationStoreLayout::message_position(
                            &row.wire,
                            &row.local_id,
                            RiseMessageLocalPhase::Remote,
                        );
                        let record = RiseMessageEntityRecord::new(row.wire, RiseConversationEntitySource::ServerPage);
                        let data = RiseMessageWireCodec::record_data(&record)?;
                        let is_unchanged = row.duplicate_local_ids.is_empty()
                            && row
                                .existing_data
                                .as_deref()
                                .is_some_and(|existing| Self::is_equivalent_server_page_data(existing, &data));
                        Ok(RiseMessageRepositoryEncodedRow {
                            local_id: row.local_id,
                            data,
                            position,
                            aliases: row.aliases,
                            duplicate_local_ids: row.duplicate_local_ids,
                            entity_version: row.entity_version,
                            is_unchanged,
                        })
                    })
                    .collect::<anyhow::Result<Vec<_>>>()
            })
            .await??;
        let mut rows = self.deduplicated(encoded);
        rows.sort_by(|left, right| left.position.cmp(&right.position));
        Ok(rows)
    }

    pub fn is_equivalent_server_page_data(existing: &[u8], fresh: &[u8]) -> bool {
        if existing == fresh {
            return true;
        }
        let Ok(record) = RiseMessageWireCodec::record(existing) else {
            return false;
        };
        if record.local_phase != RiseMessageLocalPhase::Remote
            || !matches!(record.source, RiseConversationEntitySource::Realtime | RiseConversationEntitySource::Migration)
        {
            return false;
        }
        let relabelled = RiseMessageEntityRecord { source: RiseConversationEntitySource::ServerPage, ..record };
        RiseMessageWireCodec::record_data(&relabelled).is_ok_and(|data| data == fresh)
    }

    /// store silently refuses a membership with a version below the entity
    /// rewriting the entity without the timeline leaves it behind
    /// so we lift it to the entity version, only past one the store holds
    pub(crate) fn membership_version(
        &self,
        requested: i64,
        entity_version: i64,
        existing_membership_version: Option<i64>,
    ) -> i64 {
        if let Some(existing_membership_version) = existing_membership_version
            && entity_version <= existing_membership_version
        {
            return requested;
        }
        requested.max(entity_version)
    }

    pub(crate) fn membership_versions(&self, cached: &[RiseMessageRepositoryPreservedMembership]) -> HashMap<String, i64> {
        cached.iter().map(|item| (item.local_id.clone(), item.version)).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn make_page_batch(
        &self,
        scope: &RiseMessageScope,
        rows: &[RiseMessageRepositoryEncodedRow],
        preserved: &[RiseMessageRepositoryPreservedMembership],
        cached_membership_versions: &HashMap<String, i64>,
        version: i64,
        replaces_view: bool,
        before_cursor: Option<&str>,
        after_cursor: Option<&str>,
        has_more_before: bool,
        has_more_after: bool,
        trim: RiseEntityViewRetention,
    ) -> RiseStoreWriteBatch {
        let namespace = RiseConversationStoreLayout::messages();
        let view = RiseConversationStoreLayout::message_view(scope);
        let now = Self::now_milliseconds();
        let row_ids: HashSet<&str> = rows.iter().map(|row| row.local_id.as_str()).collect();
        let mut mutations: Vec<RiseStoreMutation> = Vec::with_capacity(rows.len() * 6 + preserved.len() + 6);
        if replaces_view {
            mutations.push(RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace: namespace.clone(),
                view: view.clone(),
                if_version_at_most: Some(version),
            });
            for item in preserved.iter().filter(|item| !row_ids.contains(item.local_id.as_str())) {
                mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                    namespace: namespace.clone(),
                    view: view.clone(),
                    membership: RiseEntityViewMembership::new(
                        item.local_id.clone(),
                        item.position.clone(),
                        self.membership_version(item.version, item.entity_version, Some(item.version)),
                    ),
                });
            }
        }
        for row in rows {
            if !row.is_unchanged {
                mutations.push(RiseStoreMutation::UpsertEntity {
                    namespace: namespace.clone(),
                    entity: RiseStoreEntity::new(row.local_id.clone(), row.data.clone(), version, now),
                    policy: RiseEntityWritePolicy::IgnoreIfNotNewer,
                });
            }
            for duplicate in &row.duplicate_local_ids {
                mutations.push(RiseStoreMutation::MergeEntities {
                    namespace: namespace.clone(),
                    survivor_local_id: row.local_id.clone(),
                    duplicate_local_id: duplicate.clone(),
                });
            }
            for alias in &row.aliases {
                mutations.push(RiseStoreMutation::SetEntityAlias {
                    namespace: namespace.clone(),
                    alias: alias.clone(),
                    local_id: row.local_id.clone(),
                });
            }
            let existing_membership_version = cached_membership_versions.get(&row.local_id).copied();
            let requested_membership_version =
                if row.is_unchanged { existing_membership_version.unwrap_or(version) } else { version };
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: view.clone(),
                membership: RiseEntityViewMembership::new(
                    row.local_id.clone(),
                    row.position.clone(),
                    self.membership_version(requested_membership_version, row.entity_version, existing_membership_version),
                ),
            });
        }
        mutations.push(RiseStoreMutation::SetEntityViewState {
            namespace: namespace.clone(),
            view: view.clone(),
            state: RiseEntityViewState::new(
                self.next_version_floor(version),
                RiseConversationCodec::encode_cursor(before_cursor),
                RiseConversationCodec::encode_cursor(after_cursor),
                has_more_before,
                has_more_after,
                RiseEntityViewLoadState::Loaded,
                None,
                now,
            ),
        });
        mutations.push(RiseStoreMutation::TrimEntityView {
            namespace: namespace.clone(),
            view,
            keeping: trim,
            if_version_at_most: Some(version),
        });
        mutations.push(RiseStoreMutation::PruneUnreferencedEntities { namespace, limit: self.retention_limit });
        RiseStoreWriteBatch::new(mutations, HashSet::new())
    }

    /// re reads entity versions right before a view replacing write
    /// a realtime update during the request raises the version and the old membership would be refused
    pub async fn with_fresh_entity_versions(
        &self,
        rows: &[RiseMessageRepositoryEncodedRow],
    ) -> anyhow::Result<Vec<RiseMessageRepositoryEncodedRow>> {
        let mut fresh = rows.to_vec();
        for row in &mut fresh {
            let Some(entity) =
                self.engine.entity_local_id(RiseConversationStoreLayout::messages(), row.local_id.clone()).await?
            else {
                continue;
            };
            if entity.version > row.entity_version {
                row.entity_version = entity.version;
            }
        }
        Ok(fresh)
    }

    /// a page replacing the view clears memberships at or below its version then re adds its rows
    /// rows with a newer entity get refused so they are re added at entity version
    /// tombstones and removed entities stay out
    pub async fn restore_refused_page_memberships(
        &self,
        scope: &RiseMessageScope,
        rows: &[RiseMessageRepositoryEncodedRow],
        commit: RiseStoreCommit,
    ) -> anyhow::Result<RiseStoreCommit> {
        if rows.is_empty() {
            return Ok(commit);
        }
        let namespace = RiseConversationStoreLayout::messages();
        let view = RiseConversationStoreLayout::message_view(scope);
        let window = self
            .engine
            .entity_window(RiseEntityWindowQuery::new(
                namespace.clone(),
                view.clone(),
                RiseEntityWindow::Latest { limit: usize::try_from(self.retention_limit).unwrap_or(0) },
            ))
            .await?;
        let present: HashSet<&str> = window.items.iter().map(|item| item.entity.local_id.as_str()).collect();
        let row_ids: HashSet<&str> = rows.iter().map(|row| row.local_id.as_str()).collect();
        let Some(oldest_kept) = window
            .items
            .iter()
            .filter(|item| row_ids.contains(item.entity.local_id.as_str()))
            .map(|item| &item.position_key)
            .min()
            .cloned()
        else {
            return Ok(commit);
        };
        let mut mutations: Vec<RiseStoreMutation> = Vec::new();
        for row in rows.iter().filter(|row| !present.contains(row.local_id.as_str()) && row.position > oldest_kept) {
            let Some(entity) = self.engine.entity_local_id(namespace.clone(), row.local_id.clone()).await? else {
                continue;
            };
            if RiseMessageWireCodec::is_tombstone(&entity.data) {
                continue;
            }
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: view.clone(),
                membership: RiseEntityViewMembership::new(row.local_id.clone(), row.position.clone(), entity.version),
            });
        }
        if mutations.is_empty() {
            return Ok(commit);
        }
        RiseColdOpenTrace::mark(|| format!("a3a1 restored refused memberships={}", mutations.len()));
        let Ok(repaired) = self.engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await else {
            return Ok(commit);
        };
        Ok(if repaired.revision >= commit.revision { repaired } else { commit })
    }

    /// confirmed rows held above the newest row of a page
    /// page cant contradict them, they sort past its upper edge
    /// only rows inside the replica lag window count
    pub(crate) fn rows_newer_than_page(
        &self,
        newest_page_position: Option<&RisePositionKey>,
        cached: &[RiseMessageRepositoryPreservedMembership],
    ) -> Vec<RiseMessageRepositoryPreservedMembership> {
        let Some(newest_page_position) = newest_page_position else {
            return Vec::new();
        };
        let now = Self::now_milliseconds();
        let grace = Self::page_replica_grace_milliseconds();
        cached
            .iter()
            .filter(|item| {
                !item.is_local && item.position > *newest_page_position && now - item.written_at_milliseconds <= grace
            })
            .cloned()
            .collect()
    }

    /// a row only this device holds, eg the contact joined notice
    /// no server page has it so a seed keeps it like an unsent row
    pub fn is_device_only_row(wire: &MessageWireDTO) -> bool {
        wire.system_message_type.as_deref() == Some(MessageSystemKind::ContactJoined.raw_value())
    }

    pub(crate) async fn preserved_memberships(
        &self,
        scope: &RiseMessageScope,
    ) -> anyhow::Result<Vec<RiseMessageRepositoryPreservedMembership>> {
        let namespace = RiseConversationStoreLayout::messages();
        let view = RiseConversationStoreLayout::message_view(scope);
        let result = self
            .engine
            .entity_window(RiseEntityWindowQuery::new(
                namespace.clone(),
                view.clone(),
                RiseEntityWindow::Latest { limit: self.retention_limit as usize },
            ))
            .await?;
        let in_window: HashSet<&str> = result.items.iter().map(|item| item.entity.local_id.as_str()).collect();
        let mut memberships: Vec<RiseMessageRepositoryPreservedMembership> = Vec::with_capacity(result.items.len());
        let retained = self.engine.retained_entity_view_positions(namespace.clone(), view).await?;
        for position in retained.into_iter().filter(|position| !in_window.contains(position.local_id.as_str())) {
            let Some(entity) = self.engine.entity_local_id(namespace.clone(), position.local_id.clone()).await? else {
                continue;
            };
            if RiseMessageWireCodec::is_tombstone(&entity.data) {
                continue;
            }
            let Ok(record) = self.decoded_record(&entity) else {
                continue;
            };
            if record.local_phase == RiseMessageLocalPhase::Remote && !Self::is_device_only_row(&record.wire) {
                continue;
            }
            memberships.push(RiseMessageRepositoryPreservedMembership {
                position: position.position_key,
                version: entity.version,
                entity_version: entity.version,
                is_local: true,
                server_id: RiseConversationStoreLayout::canonical_message_server_id(&record.wire),
                written_at_milliseconds: entity.updated_at_milliseconds,
                local_id: entity.local_id,
            });
        }
        for item in &result.items {
            if RiseMessageWireCodec::is_tombstone(&item.entity.data) {
                continue;
            }
            let Ok(record) = self.decoded_record(&item.entity) else {
                continue;
            };
            let position = if record.local_phase == RiseMessageLocalPhase::Failed {
                RiseConversationStoreLayout::message_position(
                    &record.wire,
                    &item.entity.local_id,
                    RiseMessageLocalPhase::Failed,
                )
            } else {
                item.position_key.clone()
            };
            let moved = position != item.position_key;
            memberships.push(RiseMessageRepositoryPreservedMembership {
                local_id: item.entity.local_id.clone(),
                position,
                version: if moved { item.membership_version.wrapping_add(1) } else { item.membership_version },
                entity_version: item.entity.version,
                is_local: record.local_phase != RiseMessageLocalPhase::Remote || Self::is_device_only_row(&record.wire),
                server_id: RiseConversationStoreLayout::canonical_message_server_id(&record.wire),
                written_at_milliseconds: item.entity.updated_at_milliseconds,
            });
        }
        self.evict_decoded_cache_if_needed();
        Ok(memberships)
    }

    pub(crate) async fn window_query(
        &self,
        scope: &RiseMessageScope,
        anchor: &RiseMessageAnchor,
        configuration: RiseMessageWindowConfiguration,
    ) -> anyhow::Result<RiseEntityWindowQuery> {
        let namespace = RiseConversationStoreLayout::messages();
        let view = RiseConversationStoreLayout::message_view(scope);
        let before_limit = configuration.before as usize;
        let after_limit = configuration.after as usize;
        let window = match anchor {
            RiseMessageAnchor::Latest => RiseEntityWindow::Latest { limit: before_limit + after_limit + 1 },
            RiseMessageAnchor::LocalID(local_id) => {
                let Some(position) =
                    self.engine.entity_view_position(namespace.clone(), view.clone(), local_id.clone()).await?
                else {
                    return Err(RiseMessageRepositoryError::AnchorNotCached(local_id.clone()).into());
                };
                RiseEntityWindow::Around { anchor: position, before_limit, after_limit }
            }
            RiseMessageAnchor::ServerID(server_id) | RiseMessageAnchor::SavedServerID(server_id) => {
                let entity = self
                    .engine
                    .entity_alias(namespace.clone(), RiseEntityAlias::new(RiseEntityAliasKind::server_id(), server_id.clone()))
                    .await?;
                let position = match entity {
                    Some(entity) => {
                        self.engine.entity_view_position(namespace.clone(), view.clone(), entity.local_id).await?
                    }
                    None => None,
                };
                let Some(position) = position else {
                    return Err(RiseMessageRepositoryError::AnchorNotCached(server_id.clone()).into());
                };
                RiseEntityWindow::Around { anchor: position, before_limit, after_limit }
            }
        };
        Ok(RiseEntityWindowQuery::new(namespace, view, window))
    }

    pub(crate) async fn remote_anchor_id(&self, anchor: &RiseMessageAnchor) -> anyhow::Result<String> {
        match anchor {
            RiseMessageAnchor::Latest => Err(RiseMessageRepositoryError::InvalidScope.into()),
            RiseMessageAnchor::ServerID(value) | RiseMessageAnchor::SavedServerID(value) => {
                self.normalized(Some(value)).ok_or_else(|| RiseMessageRepositoryError::InvalidScope.into())
            }
            RiseMessageAnchor::LocalID(local_id) => {
                let entity =
                    self.engine.entity_local_id(RiseConversationStoreLayout::messages(), local_id.clone()).await?;
                entity
                    .and_then(|entity| RiseMessageWireCodec::record(&entity.data).ok())
                    .and_then(|record| RiseConversationStoreLayout::canonical_message_server_id(&record.wire))
                    .ok_or_else(|| RiseMessageRepositoryError::MissingServerID(local_id.clone()).into())
            }
        }
    }

    pub async fn is_latest_window(&self, scope: &RiseMessageScope) -> anyhow::Result<bool> {
        Ok(self.view_state(scope).await?.is_some_and(|state| !state.has_more_after))
    }

    pub async fn view_state(&self, scope: &RiseMessageScope) -> anyhow::Result<Option<RiseEntityViewState>> {
        self.engine
            .entity_view_state(RiseConversationStoreLayout::messages(), RiseConversationStoreLayout::message_view(scope))
            .await
    }

    pub(crate) async fn write_failure(
        &self,
        scope: &RiseMessageScope,
        previous: Option<&RiseEntityViewState>,
        error: &anyhow::Error,
    ) -> anyhow::Result<()> {
        let revision = self.next_version_floor(previous.map_or(0, |state| state.revision));
        let before_cursor = self.decode(previous.and_then(|state| state.before_cursor.as_deref()));
        let after_cursor = self.decode(previous.and_then(|state| state.after_cursor.as_deref()));
        self.write_state(
            scope,
            revision,
            before_cursor.as_deref(),
            after_cursor.as_deref(),
            previous.is_some_and(|state| state.has_more_before),
            previous.is_some_and(|state| state.has_more_after),
            RiseEntityViewLoadState::Failed,
            Some(Self::error_code(error)),
        )
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn write_state(
        &self,
        scope: &RiseMessageScope,
        revision: i64,
        before_cursor: Option<&str>,
        after_cursor: Option<&str>,
        has_more_before: bool,
        has_more_after: bool,
        load_state: RiseEntityViewLoadState,
        error_code: Option<String>,
    ) -> anyhow::Result<RiseStoreCommit> {
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetEntityViewState {
                    namespace: RiseConversationStoreLayout::messages(),
                    view: RiseConversationStoreLayout::message_view(scope),
                    state: RiseEntityViewState::new(
                        revision,
                        RiseConversationCodec::encode_cursor(before_cursor),
                        RiseConversationCodec::encode_cursor(after_cursor),
                        has_more_before,
                        has_more_after,
                        load_state,
                        error_code,
                        Self::now_milliseconds(),
                    ),
                }],
                HashSet::new(),
            ))
            .await
    }

    pub fn next_version(&self) -> i64 {
        self.next_version_floor(0)
    }

    pub fn next_version_floor(&self, floor: i64) -> i64 {
        let now = Self::now_milliseconds();
        self.state.with(|state| {
            state.version_clock = (state.version_clock + 1).max(floor + 1).max(now);
            state.version_clock
        })
    }

    pub(crate) fn register_created_aliases(&self, aliases: &[RiseEntityAlias]) -> bool {
        let limit = self.recent_created_alias_limit;
        self.state.with(|state| {
            let was_already_reduced = aliases.iter().any(|alias| state.recent_created_aliases.contains_key(alias));
            for alias in aliases {
                state.recent_created_alias_access = state.recent_created_alias_access.wrapping_add(1);
                state.recent_created_aliases.insert(alias.clone(), state.recent_created_alias_access);
            }
            let count = state.recent_created_aliases.len() as i64;
            if count <= limit {
                return was_already_reduced;
            }
            let mut victims: Vec<(RiseEntityAlias, u64)> =
                state.recent_created_aliases.iter().map(|(alias, access)| (alias.clone(), *access)).collect();
            victims.sort_by_key(|(_, access)| *access);
            for (alias, _) in victims.into_iter().take((count - limit) as usize) {
                state.recent_created_aliases.remove(&alias);
            }
            was_already_reduced
        })
    }

    pub(crate) fn scopes_for_message(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
    ) -> Vec<RiseMessageScope> {
        let mut scopes = vec![RiseMessageScope::new(chat_id.to_owned(), None, None)];
        scopes.push(RiseMessageScope::new(
            chat_id.to_owned(),
            Some(self.normalized(topic_id).unwrap_or_default()),
            None,
        ));
        if let Some(thread_root_id) = self.normalized(thread_root_id) {
            scopes.push(RiseMessageScope::new(chat_id.to_owned(), None, Some(thread_root_id)));
        }
        scopes
    }

    pub(crate) fn presentation_snapshot(
        &self,
        snapshot: &RiseStoreSnapshot,
        query: &RiseEntityWindowQuery,
        scope: &RiseMessageScope,
    ) -> anyhow::Result<RiseMessageWireSnapshot> {
        let empty;
        let result = match snapshot.entity_windows.get(query) {
            Some(result) => result,
            None => {
                empty = RiseEntityWindowResult::new(Vec::new(), None, 0, false, false, None, None);
                &empty
            }
        };
        let visible_items: Vec<_> =
            result.items.iter().filter(|item| !RiseMessageWireCodec::is_tombstone(&item.entity.data)).collect();
        let records = visible_items
            .iter()
            .map(|item| self.decoded_record(&item.entity))
            .collect::<anyhow::Result<Vec<_>>>();
        self.evict_decoded_cache_if_needed();
        let records = records?;
        for (item, record) in visible_items.iter().zip(&records) {
            if record.local_phase == RiseMessageLocalPhase::Uploading {
                self.settle_orphaned_upload_if_needed(&item.entity.local_id);
            }
            if record.local_phase == RiseMessageLocalPhase::Sending && Self::is_fanout_local_id(&item.entity.local_id) {
                self.settle_orphaned_fanout_if_needed(&item.entity.local_id);
            }
        }
        let view_state = result.state.as_ref();
        Ok(RiseMessageWireSnapshot {
            account_id: self.account_id.clone(),
            scope: scope.clone(),
            revision: snapshot.revision,
            rows: records,
            local_ids: visible_items.iter().map(|item| item.entity.local_id.clone()).collect(),
            entity_stamps: visible_items
                .iter()
                .map(|item| RiseConversationEntityStamp {
                    version: item.entity.version,
                    updated_at_milliseconds: item.entity.updated_at_milliseconds,
                })
                .collect(),
            anchor_local_id: result
                .anchor_index
                .and_then(|index| result.items.get(index))
                .filter(|item| !RiseMessageWireCodec::is_tombstone(&item.entity.data))
                .map(|item| item.entity.local_id.clone()),
            has_cached_before: result.has_cached_before,
            has_cached_after: result.has_cached_after,
            before_cursor: self.decode(view_state.and_then(|state| state.before_cursor.as_deref())),
            after_cursor: self.decode(view_state.and_then(|state| state.after_cursor.as_deref())),
            has_more_before: view_state.is_some_and(|state| state.has_more_before),
            has_more_after: view_state.is_some_and(|state| state.has_more_after),
            load_state: view_state.map_or(RiseEntityViewLoadState::Idle, |state| state.load_state),
            error_code: view_state.and_then(|state| state.last_error_code.clone()),
        })
    }

    pub(crate) fn decoded_record(&self, entity: &RiseStoreEntity) -> anyhow::Result<RiseMessageEntityRecord> {
        let key = RiseMessageRepositoryDecodedCacheKey {
            local_id: entity.local_id.clone(),
            version: entity.version,
            updated_at_milliseconds: entity.updated_at_milliseconds,
        };
        let cached = self.state.with(|state| {
            state.decoded_cache_access = state.decoded_cache_access.wrapping_add(1);
            let access = state.decoded_cache_access;
            state.decoded_record_cache.get_mut(&key).map(|cached| {
                cached.access = access;
                cached.record.clone()
            })
        });
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let record = RiseMessageWireCodec::record(&entity.data)?;
        self.state.with(|state| {
            let access = state.decoded_cache_access;
            state
                .decoded_record_cache
                .insert(key, RiseMessageRepositoryDecodedCacheEntry { record: record.clone(), access });
        });
        Ok(record)
    }

    pub(crate) fn evict_decoded_cache_if_needed(&self) {
        let limit = self.decoded_cache_limit;
        self.state.with(|state| {
            let count = state.decoded_record_cache.len() as i64;
            if count <= limit {
                return;
            }
            let mut victims: Vec<(RiseMessageRepositoryDecodedCacheKey, u64)> =
                state.decoded_record_cache.iter().map(|(key, entry)| (key.clone(), entry.access)).collect();
            victims.sort_by_key(|(_, access)| *access);
            for (key, _) in victims.into_iter().take((count - limit) as usize) {
                state.decoded_record_cache.remove(&key);
            }
        });
    }

    pub fn reports_missing_anchor(error: &anyhow::Error) -> bool {
        let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() else {
            return false;
        };
        let code = remote.code.to_lowercase();
        code.contains("not_found") || code.contains("notfound") || code.contains("deleted") || code.contains("gone")
    }

    pub fn now_milliseconds() -> i64 {
        Date::now().ms()
    }

    fn repository_error_code(error: &RiseMessageRepositoryError) -> String {
        match error {
            RiseMessageRepositoryError::InvalidScope => "invalidScope".to_owned(),
            RiseMessageRepositoryError::MissingCursor(direction) => {
                format!("missingCursor(Riseonly.RiseMessagePageDirection.{})", direction.raw_value())
            }
            RiseMessageRepositoryError::AnchorNotCached(value) => format!("anchorNotCached(\"{value}\")"),
            RiseMessageRepositoryError::AroundResponseMissingAnchor(value) => {
                format!("aroundResponseMissingAnchor(\"{value}\")")
            }
            RiseMessageRepositoryError::MessageNotFound(value) => format!("messageNotFound(\"{value}\")"),
            RiseMessageRepositoryError::MissingServerID(value) => format!("missingServerID(\"{value}\")"),
            RiseMessageRepositoryError::RemoteFailure(value) => format!("remoteFailure(\"{value}\")"),
            RiseMessageRepositoryError::UploadPipelineRequired => "uploadPipelineRequired".to_owned(),
            RiseMessageRepositoryError::InvalidWirePayload => "invalidWirePayload".to_owned(),
        }
    }

    pub fn error_code(error: &anyhow::Error) -> String {
        if error.is::<CancellationError>() {
            return Self::CANCELLED_ERROR_CODE.to_owned();
        }
        if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            return remote.code.clone();
        }
        if let Some(message) = error.downcast_ref::<RiseMessageRepositoryError>() {
            return Self::repository_error_code(message);
        }
        if let Some(upload) = Self::upload_error_code(error) {
            return upload.wire().to_owned();
        }
        Self::MESSAGE_REQUEST_FAILED_ERROR_CODE.to_owned()
    }

    /// media send fails in transport far more than in the rpc
    /// canon names every upload failure and which are retryable so the transport error is translated
    pub fn upload_error_code(error: &anyhow::Error) -> Option<RiseUploadErrorCode> {
        if let Some(upload) = error.downcast_ref::<RiseChunkedUploadError>() {
            return Some(match upload {
                RiseChunkedUploadError::EmptyFile => RiseUploadErrorCode::EmptyFile,
                RiseChunkedUploadError::InvalidEndpoint => RiseUploadErrorCode::BadRequest,
                RiseChunkedUploadError::InvalidResponse(_) => RiseUploadErrorCode::MalformedResponse,
                RiseChunkedUploadError::HttpStatus(status, _) => {
                    let first = rise_upload::RETRY_RETRYABLE_HTTP_STATUS_FIRST as i64;
                    let last = rise_upload::RETRY_RETRYABLE_HTTP_STATUS_LAST as i64;
                    match *status {
                        400 => RiseUploadErrorCode::BadRequest,
                        401 => RiseUploadErrorCode::Unauthorized,
                        403 => RiseUploadErrorCode::Forbidden,
                        404 => RiseUploadErrorCode::SessionNotFound,
                        409 => RiseUploadErrorCode::StateConflict,
                        410 => RiseUploadErrorCode::SessionExpired,
                        413 => RiseUploadErrorCode::PayloadTooLarge,
                        429 => RiseUploadErrorCode::RateLimited,
                        503 => RiseUploadErrorCode::Unavailable,
                        status if (first..=last).contains(&status) => RiseUploadErrorCode::Server,
                        _ => RiseUploadErrorCode::BadRequest,
                    }
                }
            });
        }
        if error.is::<URLError>() {
            return Some(RiseUploadErrorCode::Network);
        }
        None
    }

    /// gateway strips success and error from payload replies and turns a refusal into an error frame
    /// so a false with no error text is an absent field
    pub(crate) fn validate(&self, success: Option<bool>, error: Option<&str>) -> anyhow::Result<()> {
        if success == Some(false)
            && let Some(error) = error
            && !error.is_empty()
        {
            return Err(RiseMessageRepositoryError::RemoteFailure(error.to_owned()).into());
        }
        Ok(())
    }

    pub(crate) fn normalized(&self, raw: Option<&str>) -> Option<String> {
        RiseMessageWireCodec::normalized(raw)
    }

    pub(crate) fn decode(&self, data: Option<&[u8]>) -> Option<String> {
        RiseConversationCodec::decode_cursor(data)
    }

    pub(crate) fn validated_cursor(
        &self,
        raw: Option<&str>,
        has_more: bool,
        requested: Option<&str>,
    ) -> RiseMessageRepositoryValidatedCursor {
        let cursor = self.normalized(raw);
        let stalled = has_more && (cursor.is_none() || cursor.as_deref() == requested);
        RiseMessageRepositoryValidatedCursor {
            cursor: if stalled { None } else { cursor },
            has_more: has_more && !stalled,
            stalled,
        }
    }

    /// edge of a page the next request continues from, oldest row paging older and newest paging newer
    /// decided by timestamp and not array order since the gateway sends newest first
    pub(crate) fn boundary_cursor(&self, rows: &[MessageWireDTO], direction: RiseMessagePageDirection) -> Option<String> {
        let timestamp = |row: &MessageWireDTO| {
            RiseConversationStoreLayout::normalized_timestamp(row.created_at.or(row.timestamp).unwrap_or(0.0))
        };
        let mut ordered: Vec<&MessageWireDTO> = rows.iter().collect();
        ordered.sort_by(|left, right| timestamp(left).total_cmp(&timestamp(right)));
        if direction != RiseMessagePageDirection::Older {
            ordered.reverse();
        }
        ordered.into_iter().find_map(RiseConversationStoreLayout::canonical_message_server_id)
    }

    pub(crate) fn page(&self, rows: &[MessageWireDTO], contains_anchor: &str) -> bool {
        rows.iter().any(|wire| {
            RiseConversationStoreLayout::message_aliases(wire).iter().any(|alias| alias.value == contains_anchor)
        })
    }

    pub(crate) fn preferred_local_id<Entity: Borrow<RiseStoreEntity>>(
        &self,
        entities: &[Entity],
        wire: &MessageWireDTO,
    ) -> String {
        let mut explicit_temporary_ids: HashSet<String> =
            RiseMessageWireCodec::normalized(wire.temp_id.as_deref()).into_iter().collect();
        if let Some(identifier) = RiseMessageWireCodec::normalized(wire.id.as_deref())
            && identifier.starts_with(Self::TEMPORARY_ID_PREFIX)
        {
            explicit_temporary_ids.insert(identifier);
        }
        let local_ids = || {
            entities.iter().map(|entity| {
                let entity: &RiseStoreEntity = entity.borrow();
                entity.local_id.as_str()
            })
        };
        if let Some(temp) = local_ids().find(|local_id| explicit_temporary_ids.contains(*local_id)) {
            return temp.to_owned();
        }
        local_ids().next().map_or_else(|| RiseConversationStoreLayout::local_message_id(wire), str::to_owned)
    }

    pub(crate) fn deduplicated(&self, rows: Vec<RiseMessageRepositoryEncodedRow>) -> Vec<RiseMessageRepositoryEncodedRow> {
        let mut index_by_local_id: HashMap<String, usize> = HashMap::with_capacity(rows.len());
        let mut result: Vec<RiseMessageRepositoryEncodedRow> = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(index) = index_by_local_id.get(&row.local_id) {
                result[*index] = row;
            } else {
                index_by_local_id.insert(row.local_id.clone(), result.len());
                result.push(row);
            }
        }
        result
    }
}
