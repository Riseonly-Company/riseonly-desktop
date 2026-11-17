use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Weak};

use futures::future::BoxFuture;
use rise_canon::{RiseLimits, RiseTimingCanon, rise_key, rise_request, rise_response, rise_view};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityAliasLookup, RiseEntityKey, RiseEntityViewItem,
    RiseEntityViewKey, RiseEntityViewLoadState, RiseEntityViewMembership, RiseEntityViewPosition,
    RiseEntityViewRetention, RiseEntityViewState, RiseEntityWindow, RiseEntityWindowQuery,
    RiseEntityWindowResult, RiseEntityWritePolicy, RisePositionKey, RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::{
    RiseStoreError, RiseStoreMutation, RiseStoreSnapshot, RiseStoreSnapshotRequest, RiseStoreValueKey,
    RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue, RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine, RiseJSONValue, RiseWireError, RiseWireRemoteError};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::modules::chat::engine::chat_engine_hooks::chat_engine_hooks;
use crate::modules::chat::engine::chats::rise_chat_decoded_record_cache::RiseChatDecodedRecordCache;
use crate::modules::chat::engine::chats::rise_chat_repository_models::{
    RiseChatAggregateDelta, RiseChatListAggregates, RiseChatPendingMutation, RiseChatStoredEntity,
    RiseChatTombstone, RiseChatWireCodec, RiseStoreWriteBatchConversationExt,
};
use crate::modules::chat::engine::chats::rise_chat_repository_realtime::RiseChatOrphanPreview;
use crate::modules::chat::engine::chats::rise_normalized_key::RiseNormalizedKey;
use crate::modules::chat::engine::core::rise_conversation_codec::RiseConversationCodec;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseChatEntityRecord, RiseConversationEntitySource,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseChatListWireSnapshot, RiseConversationEntityStamp,
};
use crate::modules::chat::engine::core::rise_conversation_rpc::{
    RiseConversationEndpoints, RiseConversationRPC,
};
use crate::modules::chat::engine::core::rise_conversation_store_layout::{
    RiseConversationStoreLayout, RiseConversationStoreLayoutChatView,
};
use crate::modules::chat::stores::chats::chat_actions::chat_archive_wire_types::RiseStoredArchivedChatPage;
use crate::modules::chat::stores::chats::chat_actions::chat_message_requests_wire_types::RiseStoredMessageRequestPage;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    RiseStoredChat, RiseStoredChatPage, RiseStoredChatPreview,
};
use crate::modules::chat::stores::message::message_actions::message_types::{MessageContentType, MessageSystemKind};

#[derive(Clone)]
pub struct RiseChatLoadedWirePage {
    pub account_id: String,
    pub kind: RiseChatListKind,
    pub revision: i64,
    pub rows: Vec<RiseChatEntityRecord>,
    pub local_ids: Vec<String>,
    pub entity_stamps: Vec<RiseConversationEntityStamp>,
    pub cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Clone)]
pub struct RiseChatRepositoryProjectedChat {
    pub stored: RiseChatStoredEntity,
    pub access: u64,
}

#[derive(Clone)]
pub struct RiseChatRepositoryEncodedRow {
    pub local_id: String,
    pub data: Vec<u8>,
    pub position: RisePositionKey,
    pub aliases: Vec<RiseEntityAlias>,
    pub duplicate_local_ids: Vec<String>,
    pub is_unchanged: bool,
    pub revives_tombstone: bool,
    pub entity_version: Option<i64>,
}

#[derive(Clone)]
pub struct RiseChatRepositoryPreservedMembership {
    pub local_id: String,
    pub position: RisePositionKey,
    pub version: i64,
    pub is_local: bool,
}

#[derive(Clone)]
pub struct RiseChatRepositoryPage {
    pub rows: Vec<RiseStoredChat>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub aggregates: Option<RiseChatListAggregates>,
}

/// async handler for an unknown chat id
pub type RiseChatUnknownChatHandler = Arc<dyn Fn(String) -> BoxFuture<'static, ()> + Send + Sync>;

pub struct RiseChatRepositoryState {
    pub(crate) version_clock: i64,
    pub(crate) request_generation: HashMap<RiseChatListKind, u64>,
    pub(crate) loading_kinds: HashSet<RiseChatListKind>,
    pub(crate) pending_mutations_by_id: HashMap<Uuid, RiseChatPendingMutation>,
    pub(crate) pending_mutation_by_chat: HashMap<String, Uuid>,
    pub(crate) projected_chats: HashMap<String, RiseChatRepositoryProjectedChat>,
    pub(crate) projected_aliases: HashMap<RiseEntityAlias, String>,
    pub(crate) projection_access: u64,
    pub(crate) projected_aggregates: HashMap<RiseChatListKind, (i64, RiseChatListAggregates)>,
    pub(crate) badge_refreshed_at_ms: i64,
    pub(crate) badge_refresh_generation: u64,
    pub(crate) server_inbox_badge_generation: u64,
    pub(crate) loaded_pages: HashMap<RiseChatListKind, RiseChatLoadedWirePage>,
    pub(crate) unknown_chat_handler: Option<RiseChatUnknownChatHandler>,
    pub(crate) orphan_created_previews: HashMap<String, RiseChatOrphanPreview>,
    pub(crate) pending_preview_refetch_kinds: HashSet<RiseChatListKind>,
    pub(crate) preview_refetch_after_load_kinds: HashSet<RiseChatListKind>,
    pub(crate) preview_refetch_task: Option<JoinHandle<()>>,
    pub(crate) request_preview_retries: HashMap<String, u32>,
    pub(crate) displaced_previews: HashMap<String, HashMap<String, RiseChatDisplacedPreview>>,
}

/// what an own optimistic send replaced in its chat row, so a refused send puts the row back
#[derive(Clone)]
pub(crate) struct RiseChatDisplacedPreview {
    pub(crate) last_message: Option<crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChatPreview>,
    pub(crate) updated_at: Option<f64>,
    pub(crate) last_activity: Option<f64>,
}

impl RiseChatRepositoryState {
    fn new() -> Self {
        Self {
            version_clock: 0,
            request_generation: HashMap::new(),
            loading_kinds: HashSet::new(),
            pending_mutations_by_id: HashMap::new(),
            pending_mutation_by_chat: HashMap::new(),
            projected_chats: HashMap::new(),
            projected_aliases: HashMap::new(),
            projection_access: 0,
            projected_aggregates: HashMap::new(),
            badge_refreshed_at_ms: 0,
            badge_refresh_generation: 0,
            server_inbox_badge_generation: 0,
            loaded_pages: HashMap::new(),
            unknown_chat_handler: None,
            orphan_created_previews: HashMap::new(),
            pending_preview_refetch_kinds: HashSet::new(),
            preview_refetch_after_load_kinds: HashSet::new(),
            preview_refetch_task: None,
            request_preview_retries: HashMap::new(),
            displaced_previews: HashMap::new(),
        }
    }

    fn remove_projection(&mut self, local_id: &str) {
        self.projected_chats.remove(local_id);
        self.projected_aliases.retain(|_, value| value != local_id);
    }

    fn evict_projections(&mut self, projection_limit: i64) {
        while self.projected_chats.len() as i64 > projection_limit {
            let Some(victim) = self
                .projected_chats
                .iter()
                .min_by_key(|(_, entry)| entry.access)
                .map(|(key, _)| key.clone())
            else {
                return;
            };
            self.remove_projection(&victim);
        }
    }
}

/// buffer of the newest one, a yield replaces the unread element
/// what is buffered at the end is still delivered
struct RiseChatListWireStreamBuffer {
    latest: Option<RiseChatListWireSnapshot>,
    terminal: Option<Option<anyhow::Error>>,
}

struct RiseChatListWireStreamShared {
    buffer: RiseIsolated<RiseChatListWireStreamBuffer>,
    notify: Notify,
}

impl RiseChatListWireStreamShared {
    fn yield_snapshot(&self, snapshot: RiseChatListWireSnapshot) {
        self.buffer.with(|buffer| buffer.latest = Some(snapshot));
        self.notify.notify_one();
    }

    fn finish(&self, error: Option<anyhow::Error>) {
        self.buffer.with(|buffer| buffer.terminal = Some(error));
        self.notify.notify_one();
    }
}

/// stream that observe returns, dropping it cancels the producer
pub struct RiseChatListWireStream {
    shared: Arc<RiseChatListWireStreamShared>,
    task: JoinHandle<()>,
}

impl RiseChatListWireStream {
    pub async fn next(&mut self) -> Option<anyhow::Result<RiseChatListWireSnapshot>> {
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

impl Drop for RiseChatListWireStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// what a dropped load still owes the view
/// a dropped future never errors so the loading state must be cleared
struct RiseChatLoadCancellation {
    current_state: Option<RiseEntityViewState>,
    keeps_committed_list: bool,
}

struct RiseChatLoadGuard<'a> {
    repository: &'a RiseChatRepository,
    kind: RiseChatListKind,
    request_token: u64,
    cancellation: Option<RiseChatLoadCancellation>,
}

impl Drop for RiseChatLoadGuard<'_> {
    fn drop(&mut self) {
        let kind = self.kind;
        if let Some(cancellation) = self.cancellation.take() {
            let request_token = self.request_token;
            let is_current = self.repository.state.with(|state| {
                let is_current = state.request_generation.get(&kind).copied() == Some(request_token);
                if is_current {
                    state.projected_aggregates.remove(&kind);
                }
                is_current
            });
            if is_current && let Some(repository) = self.repository.this.upgrade() {
                let version = repository.next_version();
                RiseAccountRuntime::current().spawn(async move {
                    let _ = repository
                        .write_state_keeping_cursors(
                            kind,
                            version,
                            cancellation.current_state.as_ref(),
                            cancellation.current_state.as_ref().is_none_or(|state| state.has_more_after),
                            if cancellation.keeps_committed_list {
                                RiseEntityViewLoadState::Loaded
                            } else {
                                RiseEntityViewLoadState::Failed
                            },
                            Some(RiseChatRepository::CANCELLED_ERROR_CODE.to_owned()),
                        )
                        .await;
                });
            }
        }
        let refetches = self.repository.state.with(|state| {
            state.loading_kinds.remove(&kind);
            state.preview_refetch_after_load_kinds.remove(&kind)
        });
        if refetches {
            self.repository.schedule_preview_refetch(kind);
        }
    }
}

/// normalized chat list hot path
/// one payload per chat with aliases for lookup, lists only store membership
pub struct RiseChatRepository {
    pub(crate) this: Weak<RiseChatRepository>,
    pub(crate) engine: Arc<RiseEngine>,
    pub(crate) rpc: RiseConversationRPC,
    pub(crate) account_id: String,
    pub(crate) page_limit: i64,
    pub(crate) presentation_limit: i64,
    pub(crate) decoded_record_cache: Arc<RiseChatDecodedRecordCache>,
    pub(crate) projection_limit: i64,
    pub(crate) state: RiseIsolated<RiseChatRepositoryState>,
}

impl RiseChatRepository {
    pub const TOMBSTONE_VIEW: &'static str = "chat.tombstones";
    pub const ARCHIVE_AGGREGATES_KEY: &'static str = "chat.archive.aggregates";
    pub const REQUESTS_AGGREGATES_KEY: &'static str = "chat.requests.aggregates";
    pub const INBOX_BADGE_KEY: &'static str = "chat.inbox.badge";
    const JOINED_FALLBACK_ROLE: &'static str = "MEMBER";
    const CANCELLED_ERROR_CODE: &'static str = "cancelled";
    const DIRECT_PEER_LOGO_ATTEMPTS: usize = 3;

    pub fn default_page_limit() -> i64 {
        RiseLimits::client_value("chat_repository_page_rows")
    }

    pub fn default_presentation_limit() -> i64 {
        RiseLimits::client_value("chat_repository_presentation_rows")
    }

    pub fn badge_refresh_interval_ms() -> i64 {
        RiseTimingCanon::freshness_ms("chat_unread_badge")
    }

    /// page_limit and presentation_limit default to the canon rows when none
    pub fn new(
        engine: Arc<RiseEngine>,
        account_id: impl Into<String>,
        page_limit: Option<i64>,
        presentation_limit: Option<i64>,
    ) -> Arc<Self> {
        let account_id = account_id.into();
        let page_limit = page_limit.unwrap_or_else(Self::default_page_limit);
        let presentation_limit = presentation_limit.unwrap_or_else(Self::default_presentation_limit);
        assert!(!account_id.is_empty());
        assert!(page_limit > 0);
        assert!(presentation_limit >= page_limit);
        Arc::new_cyclic(|this| Self {
            this: this.clone(),
            rpc: RiseConversationRPC::new(engine.clone()),
            engine,
            account_id,
            page_limit,
            presentation_limit,
            decoded_record_cache: Arc::new(RiseChatDecodedRecordCache::new(
                RiseChatDecodedRecordCache::default_maximum_entries(),
                RiseChatDecodedRecordCache::default_maximum_bytes(),
            )),
            projection_limit: RiseLimits::client_value("chat_repository_projection_entries"),
            state: RiseIsolated::new(RiseChatRepositoryState::new()),
        })
    }

    pub fn on_unknown_chat(&self, handler: RiseChatUnknownChatHandler) {
        self.state.with(|state| state.unknown_chat_handler = Some(handler));
    }

    fn snapshot_request(query: &RiseEntityWindowQuery, kind: RiseChatListKind) -> RiseStoreSnapshotRequest {
        let namespace = RiseConversationStoreLayout::chats();
        RiseStoreSnapshotRequest {
            values: Self::aggregate_store_key(kind)
                .map(|key| RiseStoreValueKey::new(namespace.clone(), key))
                .into_iter()
                .collect(),
            entity_windows: HashSet::from([query.clone()]),
            entity_view_states: HashSet::from([RiseEntityViewKey::new(
                namespace,
                kind.store_view().raw_value(),
            )]),
            ..RiseStoreSnapshotRequest::default()
        }
    }

    pub async fn observe(&self, kind: RiseChatListKind) -> anyhow::Result<RiseChatListWireStream> {
        let query = self.window_query(kind);
        let request = Self::snapshot_request(&query, kind);
        let observation = self.engine.observe(request.clone()).await?;
        let account_id = self.account_id.clone();
        let engine = self.engine.clone();
        let decoded_record_cache = self.decoded_record_cache.clone();
        let shared = Arc::new(RiseChatListWireStreamShared {
            buffer: RiseIsolated::new(RiseChatListWireStreamBuffer { latest: None, terminal: None }),
            notify: Notify::new(),
        });
        let continuation = shared.clone();
        let task = RiseAccountRuntime::current().spawn(async move {
            let outcome: anyhow::Result<()> = async {
                let seed = Self::presentation_snapshot(
                    &observation.seed,
                    &query,
                    kind,
                    &account_id,
                    &decoded_record_cache,
                )
                .await?;
                continuation.yield_snapshot(seed);
                let mut updates = observation.updates.make_async_iterator();
                while updates.next().await.is_some() {
                    let snapshot = engine.snapshot(request.clone()).await?;
                    let prepared = Self::presentation_snapshot(
                        &snapshot,
                        &query,
                        kind,
                        &account_id,
                        &decoded_record_cache,
                    )
                    .await?;
                    continuation.yield_snapshot(prepared);
                }
                Ok(())
            }
            .await;
            continuation.finish(outcome.err().filter(|error| !error.is::<CancellationError>()));
        });
        Ok(RiseChatListWireStream { shared, task })
    }

    pub async fn snapshot(&self, kind: RiseChatListKind) -> anyhow::Result<RiseChatListWireSnapshot> {
        let query = self.window_query(kind);
        let value = self.engine.snapshot(Self::snapshot_request(&query, kind)).await?;
        Self::presentation_snapshot(&value, &query, kind, &self.account_id, &self.decoded_record_cache).await
    }

    /// view state is written even for an empty page so this checks the cache better than the row count
    /// offline empty list stays valid
    pub async fn has_materialized_view(&self, kind: RiseChatListKind) -> anyhow::Result<bool> {
        Ok(self
            .engine
            .entity_view_state(RiseConversationStoreLayout::chats(), kind.store_view().raw_value().to_owned())
            .await?
            .is_some())
    }

    /// lookup by canonical or temp or client or local id
    pub async fn chat(&self, id: &str) -> anyhow::Result<Option<RiseChatEntityRecord>> {
        Ok(self.stored_chat_id(id, false).await?.map(|stored| stored.record))
    }

    /// permanent renderer id for a server, temp or client chat id
    pub async fn local_id(&self, id: &str) -> anyhow::Result<Option<String>> {
        Ok(self.stored_chat_id(id, false).await?.map(|stored| stored.entity.local_id))
    }

    fn alias_kinds(id: &str) -> [RiseEntityAliasKind; 3] {
        if GetSaiServerId::is_temp_id(Some(id)) {
            [RiseEntityAliasKind::temporary_id(), RiseEntityAliasKind::client_id(), RiseEntityAliasKind::server_id()]
        } else {
            [RiseEntityAliasKind::server_id(), RiseEntityAliasKind::client_id(), RiseEntityAliasKind::temporary_id()]
        }
    }

    /// stored chat by id, durable skips the pre commit projection and reads the committed row
    pub async fn stored_chat_id(&self, raw_id: &str, durable: bool) -> anyhow::Result<Option<RiseChatStoredEntity>> {
        let id = trimming_whitespaces_and_newlines(raw_id);
        if id.is_empty() {
            return Ok(None);
        }
        if !durable && let Some(projected) = self.projected_chat_id(id) {
            return Ok(Some(projected));
        }
        let namespace = RiseConversationStoreLayout::chats();
        for kind in Self::alias_kinds(id) {
            if let Some(entity) =
                self.engine.entity_alias(namespace.clone(), RiseEntityAlias::new(kind, id)).await?
            {
                return Ok(Some(self.decode_stored(entity)?));
            }
        }
        let Some(entity) = self.engine.entity_local_id(namespace, id.to_owned()).await? else {
            return Ok(None);
        };
        Ok(Some(self.decode_stored(entity)?))
    }

    pub async fn stored_chat_local_id(&self, local_id: &str) -> anyhow::Result<Option<RiseChatStoredEntity>> {
        if let Some(projected) = self.projected_chat_local_id(local_id) {
            return Ok(Some(projected));
        }
        let Some(entity) =
            self.engine.entity_local_id(RiseConversationStoreLayout::chats(), local_id.to_owned()).await?
        else {
            return Ok(None);
        };
        Ok(Some(self.decode_stored(entity)?))
    }

    /// pair of local ids and confirmed ids
    pub async fn removed_chats(&self) -> anyhow::Result<(HashSet<String>, HashSet<String>)> {
        let result = self
            .engine
            .entity_window(RiseEntityWindowQuery::new(
                RiseConversationStoreLayout::chats(),
                Self::TOMBSTONE_VIEW,
                RiseEntityWindow::Latest { limit: self.presentation_limit as usize },
            ))
            .await?;
        let records = self.decoded_record_cache.records(&result.items).await?;
        let mut local_ids = HashSet::new();
        let mut confirmed_ids = HashSet::new();
        for (item, record) in result.items.iter().zip(records.iter()) {
            let Some(tombstone) = record.tombstone else {
                continue;
            };
            local_ids.insert(item.entity.local_id.clone());
            if tombstone.is_confirmed() {
                let id = RiseConversationStoreLayout::canonical_chat_server_id(&record.wire);
                if !id.is_empty() {
                    confirmed_ids.insert(id);
                }
            }
        }
        Ok((local_ids, confirmed_ids))
    }

    /// saves a point lookup without putting it in any list window, existing membership stays
    pub async fn store_chat_detail(&self, wire: RiseStoredChat) -> anyhow::Result<RiseChatStoredEntity> {
        let existing = self.stored_chat_id(&wire.id, false).await?;
        let mut wire = wire;
        let live_stored = existing.as_ref().map(|stored| &stored.record).filter(|record| record.tombstone.is_none());
        if let Some(stored) = live_stored {
            if wire.last_message.is_none() && stored.wire.last_message.is_some() {
                wire = Self::keeping_stored_preview(&wire, &stored.wire)?;
            } else if Self::keeps_unsent_preview(stored.wire.last_message.as_ref(), wire.last_message.as_ref()) {
                wire = Self::patched_last_message(&wire, stored.wire.last_message.as_ref())?;
            }
            wire = Self::keeping_stored_call_metadata(wire, &stored.wire)?;
            wire = Self::keeping_stored_unread_count(wire, &stored.wire)?;
        }
        let local_id = existing
            .as_ref()
            .map(|stored| stored.entity.local_id.clone())
            .unwrap_or_else(|| RiseConversationStoreLayout::local_chat_id(&wire));
        let kind = existing.as_ref().and_then(|stored| stored.record.list_kind);
        let version = self.next_version();
        let aliases = RiseConversationStoreLayout::chat_aliases(&wire);
        let record = RiseChatEntityRecord {
            wire,
            source: RiseConversationEntitySource::ServerPage,
            locally_created: existing.as_ref().is_some_and(|stored| stored.record.locally_created),
            list_kind: kind,
            tombstone: None,
        };
        let duplicates = existing
            .as_ref()
            .filter(|stored| stored.entity.local_id != local_id)
            .map(|stored| stored.entity.local_id.clone())
            .into_iter()
            .collect();
        let batch = self.make_upsert_batch(record.clone(), &local_id, aliases, kind, version, true, duplicates)?;
        self.engine.write(batch).await?;
        if let Some(stored) = self.stored_chat_local_id(&local_id).await? {
            return Ok(stored);
        }
        Ok(RiseChatStoredEntity {
            entity: RiseStoreEntity::new(
                local_id,
                RiseChatWireCodec::encode_record(&record)?,
                version,
                Self::now_milliseconds(),
            ),
            record,
        })
    }

    pub async fn patch_direct_peer_logo<ValidateSession, Validation>(
        &self,
        chat_id: &str,
        peer_user_id: &str,
        logo: &str,
        validate_session: ValidateSession,
    ) -> anyhow::Result<()>
    where
        ValidateSession: Fn() -> Validation,
        Validation: Future<Output = anyhow::Result<()>>,
    {
        validate_session().await?;
        let Some(original) = self.stored_chat_id(chat_id, false).await? else {
            return Ok(());
        };
        for _ in 0..Self::DIRECT_PEER_LOGO_ATTEMPTS {
            validate_session().await?;
            let Some(stored) = self.stored_chat_id(chat_id, false).await? else {
                return Ok(());
            };
            if stored.entity.local_id != original.entity.local_id || stored.record.tombstone.is_some() {
                return Ok(());
            }
            let Some(participant) = stored.record.wire.participant.as_ref() else {
                return Ok(());
            };
            if participant.id != peer_user_id || participant.resolved_logo() == logo {
                return Ok(());
            }
            let mut object = RiseChatWireCodec::object(&stored.record.wire)?;
            let Some(mut peer) =
                object.get(rise_key::chat::PARTICIPANT).and_then(RiseJSONValue::object_value).cloned()
            else {
                return Ok(());
            };
            let mut more = peer
                .get(rise_key::chat::MORE)
                .and_then(RiseJSONValue::object_value)
                .cloned()
                .unwrap_or_default();
            more.insert(rise_key::chat::LOGO.to_owned(), RiseJSONValue::String(logo.to_owned()));
            peer.insert(rise_key::chat::MORE.to_owned(), RiseJSONValue::Object(more));
            peer.insert(rise_key::chat::LOGO.to_owned(), RiseJSONValue::String(logo.to_owned()));
            peer.insert(RiseNormalizedKey::avatar_url().to_owned(), RiseJSONValue::String(logo.to_owned()));
            object.insert(rise_key::chat::PARTICIPANT.to_owned(), RiseJSONValue::Object(peer));
            let record = RiseChatEntityRecord {
                wire: RiseChatWireCodec::decode_wire(object)?,
                source: stored.record.source,
                locally_created: stored.record.locally_created,
                list_kind: stored.record.list_kind,
                tombstone: stored.record.tombstone,
            };
            let entity = RiseStoreEntity::new(
                stored.entity.local_id.clone(),
                RiseChatWireCodec::encode_record(&record)?,
                self.next_version(),
                Self::now_milliseconds(),
            );
            validate_session().await?;
            self.engine
                .write(Self::write_batch(vec![RiseStoreMutation::UpsertEntity {
                    namespace: RiseConversationStoreLayout::chats(),
                    entity: entity.clone(),
                    policy: RiseEntityWritePolicy::CompareAndSwap { expected: stored.entity.clone() },
                }]))
                .await?;
            validate_session().await?;
            let Some(committed) = self
                .engine
                .entity_local_id(RiseConversationStoreLayout::chats(), entity.local_id.clone())
                .await?
            else {
                return Ok(());
            };
            if committed == entity {
                self.state.with(|state| {
                    let is_older = state
                        .projected_chats
                        .get(&entity.local_id)
                        .is_some_and(|projected| projected.stored.entity.version <= entity.version);
                    if is_older {
                        state.remove_projection(&entity.local_id);
                    }
                });
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
        validate_session().await?;
        Err(RiseStoreError::EntityVersionConflict {
            namespace: RiseConversationStoreLayout::chats(),
            local_id: original.entity.local_id,
        }
        .into())
    }

    /// stored rows every alias resolves to, undecodable ones are skipped
    async fn resolved_aliases(&self, aliases: &[RiseEntityAlias]) -> anyhow::Result<Vec<RiseChatStoredEntity>> {
        let mut resolved = Vec::new();
        for alias in aliases {
            let Some(entity) =
                self.engine.entity_alias(RiseConversationStoreLayout::chats(), alias.clone()).await?
            else {
                continue;
            };
            if let Ok(stored) = self.decode_stored(entity) {
                resolved.push(stored);
            }
        }
        Ok(resolved)
    }

    fn duplicate_ids<'a>(local_ids: impl Iterator<Item = &'a str>, survivor: &str) -> Vec<String> {
        local_ids
            .filter(|local_id| *local_id != survivor)
            .collect::<HashSet<_>>()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// puts a created chat in the inbox from the mutation ack
    /// no get_chats after create_chat, the returned row is the truth
    pub async fn store_created_chat(&self, wire: RiseStoredChat) -> anyhow::Result<()> {
        let aliases = RiseConversationStoreLayout::chat_aliases(&wire);
        let resolved = self.resolved_aliases(&aliases).await?;
        let preferred = resolved
            .iter()
            .find(|stored| GetSaiServerId::is_temp_id(Some(stored.entity.local_id.as_str())))
            .or_else(|| resolved.first());
        let local_id = preferred
            .map(|stored| stored.entity.local_id.clone())
            .unwrap_or_else(|| RiseConversationStoreLayout::local_chat_id(&wire));
        let duplicates =
            Self::duplicate_ids(resolved.iter().map(|stored| stored.entity.local_id.as_str()), &local_id);
        let version = self.next_version();
        let record = RiseChatEntityRecord {
            wire,
            source: RiseConversationEntitySource::Acknowledgement,
            locally_created: false,
            list_kind: Some(RiseChatListKind::Inbox),
            tombstone: None,
        };
        let batch = self.make_upsert_batch(
            record,
            &local_id,
            aliases,
            Some(RiseChatListKind::Inbox),
            version,
            true,
            duplicates,
        )?;
        self.engine.write(batch).await?;
        Ok(())
    }

    pub async fn adopt_joined_chat(
        &self,
        chat_id: &str,
        authoritative: Option<RiseStoredChat>,
    ) -> anyhow::Result<Option<RiseStoredChat>> {
        let existing = self.stored_chat_id(chat_id, false).await?;
        let Some(mut base) = authoritative.or_else(|| existing.as_ref().map(|stored| stored.record.wire.clone()))
        else {
            return Ok(None);
        };
        if base.last_message.is_none()
            && let Some(previous) = existing.as_ref().and_then(|stored| stored.record.wire.last_message.as_ref())
        {
            base = Self::patched_last_message(&base, Some(previous))?;
        }
        let wire = Self::granting_membership(&base)?;
        let aliases = RiseConversationStoreLayout::chat_aliases(&wire);
        let mut resolved: Vec<RiseChatStoredEntity> = existing.iter().cloned().collect();
        resolved.extend(self.resolved_aliases(&aliases).await?);
        let local_id = resolved
            .first()
            .map(|stored| stored.entity.local_id.clone())
            .unwrap_or_else(|| RiseConversationStoreLayout::local_chat_id(&wire));
        let duplicates =
            Self::duplicate_ids(resolved.iter().map(|stored| stored.entity.local_id.as_str()), &local_id);
        let destination =
            if existing.as_ref().and_then(|stored| stored.record.list_kind) == Some(RiseChatListKind::Archive) {
                RiseChatListKind::Archive
            } else {
                RiseChatListKind::Inbox
            };
        let version = self.next_version();
        let record = RiseChatEntityRecord {
            wire: wire.clone(),
            source: RiseConversationEntitySource::Acknowledgement,
            locally_created: existing.as_ref().is_some_and(|stored| stored.record.locally_created),
            list_kind: Some(destination),
            tombstone: None,
        };
        let batch =
            self.make_upsert_batch(record, &local_id, aliases, Some(destination), version, true, duplicates)?;
        self.engine.write(batch).await?;
        if wire.last_message.is_none() {
            self.schedule_preview_refetch(destination);
        }
        Ok(Some(wire))
    }

    /// fills in the membership a join grants when the row omits it
    /// else a preview role survives the join
    pub fn granting_membership(wire: &RiseStoredChat) -> anyhow::Result<RiseStoredChat> {
        let is_joined =
            chat_engine_hooks().is_some_and(|hooks| (hooks.is_joined_member_role)(wire.role.as_deref()));
        if is_joined {
            return Ok(wire.clone());
        }
        let mut updates = BTreeMap::from([(
            rise_key::chat::ROLE.to_owned(),
            RiseJSONValue::String(Self::JOINED_FALLBACK_ROLE.to_owned()),
        )]);
        if wire.joined_at.is_none() {
            updates.insert(
                RiseNormalizedKey::joined_at().to_owned(),
                RiseJSONValue::Integer(Self::now_milliseconds()),
            );
        }
        RiseChatWireCodec::patch(wire, updates)
    }

    pub async fn refresh(&self, kind: RiseChatListKind) -> anyhow::Result<()> {
        self.load(kind, None, true).await
    }

    /// false when nothing was asked because the same list is already loading
    pub async fn load_more(&self, kind: RiseChatListKind) -> anyhow::Result<bool> {
        let state = self
            .engine
            .entity_view_state(RiseConversationStoreLayout::chats(), kind.store_view().raw_value().to_owned())
            .await?;
        if state.as_ref().is_some_and(|state| !state.has_more_after) {
            return Ok(true);
        }
        let cursor =
            RiseConversationCodec::decode_cursor(state.as_ref().and_then(|state| state.after_cursor.as_deref()));
        let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) else {
            return Ok(true);
        };
        if self.state.with(|state| state.loading_kinds.contains(&kind)) {
            return Ok(false);
        }
        self.load(kind, Some(&cursor), false).await?;
        Ok(true)
    }

    pub fn take_loaded_page(&self, kind: RiseChatListKind) -> Option<RiseChatLoadedWirePage> {
        self.state.with(|state| state.loaded_pages.remove(&kind))
    }

    pub async fn load(
        &self,
        kind: RiseChatListKind,
        cursor: Option<&str>,
        replaces_loaded_view: bool,
    ) -> anyhow::Result<()> {
        if !self.state.with(|state| state.loading_kinds.insert(kind)) {
            return Ok(());
        }
        let request_token = self.state.with(|state| {
            let generation = state.request_generation.entry(kind).or_insert(0);
            *generation = generation.wrapping_add(1);
            *generation
        });
        let mut guard = RiseChatLoadGuard { repository: self, kind, request_token, cancellation: None };

        let request_version = self.next_version();
        let current_state = self
            .engine
            .entity_view_state(RiseConversationStoreLayout::chats(), kind.store_view().raw_value().to_owned())
            .await?;
        let cached_memberships = self.preserved_memberships(kind).await?;
        let keeps_committed_list =
            current_state.as_ref().is_some_and(|state| state.load_state == RiseEntityViewLoadState::Loaded)
                || cached_memberships.iter().any(|membership| !membership.is_local);
        guard.cancellation =
            Some(RiseChatLoadCancellation { current_state: current_state.clone(), keeps_committed_list });
        let has_more_after = current_state.as_ref().is_none_or(|state| state.has_more_after);
        let loading_written = self
            .write_state_keeping_cursors(
                kind,
                self.next_version(),
                current_state.as_ref(),
                has_more_after,
                RiseEntityViewLoadState::Loading,
                None,
            )
            .await;
        if let Err(error) = loading_written {
            guard.cancellation = None;
            return Err(error);
        }

        let outcome = self
            .load_page(
                kind,
                cursor,
                replaces_loaded_view,
                request_token,
                request_version,
                current_state.as_ref(),
                &cached_memberships,
            )
            .await;
        guard.cancellation = None;
        let Err(error) = outcome else {
            return Ok(());
        };

        let is_current = self.state.with(|state| {
            let is_current = state.request_generation.get(&kind).copied() == Some(request_token);
            if is_current {
                state.projected_aggregates.remove(&kind);
            }
            is_current
        });
        if !is_current {
            return Ok(());
        }
        let _ = self
            .write_state_keeping_cursors(
                kind,
                self.next_version(),
                current_state.as_ref(),
                has_more_after,
                if keeps_committed_list { RiseEntityViewLoadState::Loaded } else { RiseEntityViewLoadState::Failed },
                Some(Self::error_code(&error)),
            )
            .await;
        Err(error)
    }

    /// the do block of load
    #[allow(clippy::too_many_arguments)]
    async fn load_page(
        &self,
        kind: RiseChatListKind,
        cursor: Option<&str>,
        replaces_loaded_view: bool,
        request_token: u64,
        request_version: i64,
        current_state: Option<&RiseEntityViewState>,
        cached_memberships: &[RiseChatRepositoryPreservedMembership],
    ) -> anyhow::Result<()> {
        let mut page = self.fetch_page(kind, cursor).await?;
        if self.state.with(|state| state.request_generation.get(&kind).copied()) != Some(request_token) {
            return Ok(());
        }
        let rows = self.encode_rows(&page.rows, kind, request_version).await?;
        let cursor_stalled =
            page.has_more && (page.rows.is_empty() || (cursor.is_some() && page.cursor.as_deref() == cursor));
        if page.rows.is_empty() && cursor_stalled {
            self.write_state_keeping_cursors(
                kind,
                self.next_version(),
                current_state,
                false,
                RiseEntityViewLoadState::Loaded,
                Some("cursor_stalled".to_owned()),
            )
            .await?;
            return Ok(());
        }
        let mut preserved: Vec<RiseChatRepositoryPreservedMembership> =
            cached_memberships.iter().filter(|membership| membership.is_local).cloned().collect();
        if replaces_loaded_view
            && page.has_more
            && let Some(fresh_boundary) = rows.last().map(|row| &row.position)
        {
            let fresh: HashSet<&str> = rows.iter().map(|row| row.local_id.as_str()).collect();
            preserved.extend(
                cached_memberships
                    .iter()
                    .filter(|membership| {
                        !membership.is_local
                            && !fresh.contains(membership.local_id.as_str())
                            && membership.position > *fresh_boundary
                    })
                    .cloned(),
            );
        }
        if cursor_stalled {
            page.has_more = false;
        }
        let batch = self.make_page_batch(
            &rows,
            &preserved,
            kind,
            &page,
            request_version,
            self.next_version(),
            replaces_loaded_view,
        )?;
        self.engine.write(batch).await?;
        self.reconcile_page_projection(&rows, request_version);
        if replaces_loaded_view {
            self.state.with(|state| state.loaded_pages.remove(&kind));
        } else {
            let namespace = RiseConversationStoreLayout::chats();
            let view_key = RiseEntityViewKey::new(namespace.clone(), kind.store_view().raw_value());
            let committed = self
                .engine
                .snapshot(RiseStoreSnapshotRequest {
                    entities: rows
                        .iter()
                        .map(|row| RiseEntityKey::new(namespace.clone(), row.local_id.clone()))
                        .collect(),
                    entity_view_states: HashSet::from([view_key.clone()]),
                    ..RiseStoreSnapshotRequest::default()
                })
                .await?;
            let mut committed_rows = Vec::new();
            let mut local_ids = Vec::new();
            let mut stamps = Vec::new();
            for row in &rows {
                let Some(entity) =
                    committed.entities.get(&RiseEntityKey::new(namespace.clone(), row.local_id.clone()))
                else {
                    continue;
                };
                let record = RiseChatWireCodec::decode_record(&entity.data)?;
                if record.tombstone.is_some() || record.list_kind != Some(kind) {
                    continue;
                }
                committed_rows.push(record);
                local_ids.push(entity.local_id.clone());
                stamps.push(RiseConversationEntityStamp {
                    version: entity.version,
                    updated_at_milliseconds: entity.updated_at_milliseconds,
                });
            }
            let view_state = committed.entity_view_states.get(&view_key);
            let loaded = RiseChatLoadedWirePage {
                account_id: self.account_id.clone(),
                kind,
                revision: committed.revision,
                rows: committed_rows,
                local_ids,
                entity_stamps: stamps,
                cursor: RiseConversationCodec::decode_cursor(
                    view_state.and_then(|state| state.after_cursor.as_deref()),
                ),
                has_more: view_state.map_or(page.has_more, |state| state.has_more_after),
            };
            self.state.with(|state| state.loaded_pages.insert(kind, loaded));
        }
        if let Some(aggregates) = page.aggregates {
            self.install_aggregate_projection(kind, aggregates, request_version);
            if kind == RiseChatListKind::Inbox {
                self.bump_server_inbox_badge_generation();
            }
        }
        Ok(())
    }

    pub async fn search_inbox_page(&self, query: &str, cursor: Option<&str>) -> anyhow::Result<RiseStoredChatPage> {
        let reply: rise_view::chat::ChatPage = self
            .rpc
            .call(
                RiseConversationEndpoints::get_chats(),
                rise_request::chat::GetChats {
                    limit: Some(self.page_limit),
                    q: Some(query.to_owned()),
                    relative_id: cursor.map(str::to_owned),
                    up: Some(false),
                },
            )
            .await?;
        let mut page = RiseStoredChatPage::from_canon(&reply);
        if page.relative_id.is_none() {
            page.relative_id = page.chats.as_ref().and_then(|rows| rows.last()).map(|row| row.id.clone());
        }
        Ok(page)
    }

    pub async fn fetch_page(
        &self,
        kind: RiseChatListKind,
        cursor: Option<&str>,
    ) -> anyhow::Result<RiseChatRepositoryPage> {
        match kind {
            RiseChatListKind::Inbox => {
                let reply: rise_view::chat::ChatPage = self
                    .rpc
                    .call(
                        RiseConversationEndpoints::get_chats(),
                        rise_request::chat::GetChats {
                            limit: Some(self.page_limit),
                            q: None,
                            relative_id: cursor.map(str::to_owned),
                            up: Some(false),
                        },
                    )
                    .await?;
                let response = RiseStoredChatPage::from_canon(&reply);
                let rows = response.chats.unwrap_or_default();
                Ok(RiseChatRepositoryPage {
                    cursor: response.relative_id.or_else(|| rows.last().map(|row| row.id.clone())),
                    has_more: response.is_have_more.unwrap_or(false),
                    aggregates: None,
                    rows,
                })
            }
            RiseChatListKind::Archive => {
                let reply: rise_view::chat::ArchivedChats = self
                    .rpc
                    .call(
                        RiseConversationEndpoints::get_archived_chats(),
                        rise_request::chat::GetArchivedChats {
                            limit: Some(self.page_limit),
                            q: None,
                            relative_id: cursor.map(str::to_owned),
                            up: Some(false),
                        },
                    )
                    .await?;
                let response = RiseStoredArchivedChatPage::from_canon(&reply);
                let rows = response.chats.unwrap_or_default();
                Ok(RiseChatRepositoryPage {
                    cursor: response.relative_id.or_else(|| rows.last().map(|row| row.id.clone())),
                    has_more: response.is_have_more.unwrap_or(false),
                    aggregates: Some(RiseChatListAggregates::new(
                        response.total_count,
                        response.total_unread_count,
                        response.unread_chats_count,
                        None,
                        None,
                    )),
                    rows,
                })
            }
            RiseChatListKind::Requests => {
                let reply: rise_view::chat::MessageRequests = self
                    .rpc
                    .call(
                        RiseConversationEndpoints::get_message_requests(),
                        rise_request::chat::GetMessageRequests {
                            limit: Some(self.page_limit),
                            relative_id: cursor.map(str::to_owned),
                        },
                    )
                    .await?;
                let response = RiseStoredMessageRequestPage::from_canon(&reply);
                let rows = response.chats.unwrap_or_default();
                let aggregates = cursor
                    .is_none()
                    .then(|| RiseChatListAggregates::new(response.total_count, None, None, None, None));
                Ok(RiseChatRepositoryPage {
                    cursor: response.relative_id.or_else(|| rows.last().map(|row| row.id.clone())),
                    has_more: response.has_more.unwrap_or(false),
                    aggregates,
                    rows,
                })
            }
        }
    }

    pub async fn encode_rows(
        &self,
        rows: &[RiseStoredChat],
        kind: RiseChatListKind,
        request_version: i64,
    ) -> anyhow::Result<Vec<RiseChatRepositoryEncodedRow>> {
        let namespace = RiseConversationStoreLayout::chats();
        let aliases_by_index: Vec<Vec<RiseEntityAlias>> =
            rows.iter().map(RiseConversationStoreLayout::chat_aliases).collect();
        let lookups: HashSet<RiseEntityAliasLookup> = aliases_by_index
            .iter()
            .flatten()
            .map(|alias| RiseEntityAliasLookup::new(namespace.clone(), alias.clone()))
            .collect();
        let snapshot = self
            .engine
            .snapshot(RiseStoreSnapshotRequest { entity_aliases: lookups, ..RiseStoreSnapshotRequest::default() })
            .await?;

        let page_lacks_previews = rows.iter().all(|row| row.last_message.is_none());

        let mut encoded = Vec::with_capacity(rows.len());
        let mut selected_by_alias: HashMap<RiseEntityAlias, String> = HashMap::new();
        for (aliases, server_wire) in aliases_by_index.into_iter().zip(rows.iter()) {
            let resolved: Vec<String> = aliases
                .iter()
                .filter_map(|alias| {
                    selected_by_alias.get(alias).cloned().or_else(|| {
                        snapshot
                            .entity_aliases
                            .get(&RiseEntityAliasLookup::new(namespace.clone(), alias.clone()))
                            .map(|entity| entity.local_id.clone())
                    })
                })
                .collect();
            let preferred = resolved
                .iter()
                .find(|local_id| GetSaiServerId::is_temp_id(Some(local_id.as_str())))
                .or_else(|| resolved.first());
            let local_id =
                preferred.cloned().unwrap_or_else(|| RiseConversationStoreLayout::local_chat_id(server_wire));
            let duplicate_ids = Self::duplicate_ids(resolved.iter().map(String::as_str), &local_id);

            let existing = snapshot.entity_aliases.values().find(|entity| entity.local_id == local_id);
            let existing_record = existing.and_then(|entity| RiseChatWireCodec::decode_record(&entity.data).ok());
            let mut revives_tombstone = false;
            if let Some(entity) = existing
                && let Some(tombstone) = existing_record.as_ref().and_then(|record| record.tombstone)
            {
                if !tombstone.is_confirmed() || entity.version >= request_version {
                    continue;
                }
                revives_tombstone = true;
            }
            let mut wire = server_wire.clone();
            if let Some(stored) = existing_record.as_ref().filter(|record| record.tombstone.is_none()) {
                if page_lacks_previews && stored.wire.last_message.is_some() {
                    wire = Self::keeping_stored_preview(server_wire, &stored.wire)?;
                } else if Self::keeps_unsent_preview(
                    stored.wire.last_message.as_ref(),
                    server_wire.last_message.as_ref(),
                ) {
                    wire = Self::patched_last_message(server_wire, stored.wire.last_message.as_ref())?;
                }
                wire = Self::keeping_stored_call_metadata(wire, &stored.wire)?;
            }
            let mut late_version = None;
            if let Some(entity) = existing
                && entity.version >= request_version
                && let Some(stored) = existing_record.as_ref().filter(|record| record.tombstone.is_none())
                && Self::server_preview_is_newer(stored.wire.last_message.as_ref(), wire.last_message.as_ref())
            {
                wire = Self::patched_last_message(&stored.wire, wire.last_message.as_ref())?;
                late_version = Some(self.next_version().max(entity.version + 1));
                tracing::warn!(target: "riseonly::X1336", "page preview carried onto later row chat={}", server_wire.id);
            }
            if kind == RiseChatListKind::Requests && server_wire.last_message.is_none() {
                if let Some(stored) = existing_record.as_ref().filter(|record| record.tombstone.is_none())
                    && stored.wire.last_message.is_some()
                {
                    wire = Self::keeping_stored_preview(&wire, &stored.wire)?;
                }
                if wire.last_message.is_none() {
                    wire = self.applying_orphan_preview(wire, &server_wire.id)?;
                }
                let stored_unread = existing_record
                    .as_ref()
                    .filter(|record| record.tombstone.is_none())
                    .map_or(0, |record| record.wire.resolved_unread_count());
                if wire.last_message.is_some() {
                    self.state.with(|state| state.request_preview_retries.remove(&server_wire.id));
                } else if stored_unread > 0 {
                    if stored_unread > wire.resolved_unread_count() {
                        wire = RiseChatWireCodec::patch(
                            &wire,
                            BTreeMap::from([(
                                RiseNormalizedKey::unread_count().to_owned(),
                                RiseJSONValue::Integer(stored_unread),
                            )]),
                        )?;
                    }
                    self.schedule_request_preview_retry(&server_wire.id);
                }
                tracing::warn!(
                    target: "riseonly::X567",
                    "requests page chat={} preview={} unread={}",
                    server_wire.id,
                    wire.last_message.is_some(),
                    wire.resolved_unread_count()
                );
            }

            for alias in &aliases {
                selected_by_alias.insert(alias.clone(), local_id.clone());
            }
            let position = RiseConversationStoreLayout::chat_position(&wire);
            let record = RiseChatEntityRecord {
                wire,
                source: RiseConversationEntitySource::ServerPage,
                locally_created: false,
                list_kind: Some(kind),
                tombstone: None,
            };
            let data = RiseChatWireCodec::encode_record(&record)?;
            let is_unchanged = duplicate_ids.is_empty()
                && existing.is_some_and(|entity| Self::is_equivalent_server_page_data(&entity.data, &data));
            encoded.push(RiseChatRepositoryEncodedRow {
                local_id,
                data,
                position,
                aliases,
                duplicate_local_ids: duplicate_ids,
                is_unchanged,
                revives_tombstone,
                entity_version: late_version,
            });
        }
        Ok(encoded)
    }

    /// an own message the server hasnt taken yet is newer than the server last one
    /// a page fetched meanwhile must not hide it
    pub fn keeps_unsent_preview(
        stored: Option<&RiseStoredChatPreview>,
        server: Option<&RiseStoredChatPreview>,
    ) -> bool {
        let Some(stored) = stored else {
            return false;
        };
        let device_only =
            stored.system_message_type.as_deref() == Some(MessageSystemKind::ContactJoined.raw_value());
        if !device_only && !GetSaiServerId::is_temp_id(stored.id.as_deref()) {
            return false;
        }
        let Some(server) = server else {
            return true;
        };
        Self::preview_seconds(stored) > Self::preview_seconds(server)
    }

    /// list preview has no call metadata so a page turned incoming call into plain
    /// stored preview of same message keeps it, also the member a kick or ban names
    pub fn keeping_stored_call_metadata(
        wire: RiseStoredChat,
        stored: &RiseStoredChat,
    ) -> anyhow::Result<RiseStoredChat> {
        let (Some(server), Some(stored_message)) = (wire.last_message.as_ref(), stored.last_message.as_ref()) else {
            return Ok(wire);
        };
        let Some(id) = server.id.as_deref().filter(|id| !id.is_empty()) else {
            return Ok(wire);
        };
        if Some(id) != stored_message.id.as_deref() {
            return Ok(wire);
        }
        let mut patched = server.clone();
        let mut changed = false;
        if server.metadata.is_none()
            && MessageContentType::from_raw_value(server.content_type.as_deref().unwrap_or(""))
                == MessageContentType::Call
            && let Some(metadata) = stored_message.metadata.as_ref()
            && let RiseJSONValue::Object(mut object) = RiseJSONValue::encode(server)?
        {
            object.insert(rise_key::message::METADATA.to_owned(), RiseJSONValue::encode(metadata)?);
            patched = RiseJSONValue::Object(object).decode_as::<RiseStoredChatPreview>()?;
            changed = true;
        }
        if patched.system_target.is_none()
            && let Some(target) = stored_message.system_target.as_ref()
        {
            patched.system_target = Some(target.clone());
            changed = true;
        }
        if !changed {
            return Ok(wire);
        }
        Self::patched_last_message(&wire, Some(&patched))
    }

    fn server_preview_is_newer(stored: Option<&RiseStoredChatPreview>, server: Option<&RiseStoredChatPreview>) -> bool {
        let Some(server) = server else {
            return false;
        };
        let Some(stored) = stored else {
            return true;
        };
        if server.id.is_some() && server.id == stored.id {
            return false;
        }
        Self::preview_seconds(server) > Self::preview_seconds(stored)
    }

    fn preview_seconds(message: &RiseStoredChatPreview) -> TimeInterval {
        let raw = message.timestamp.or(message.created_at).unwrap_or(0.0);
        if raw > 1_000_000_000_000.0 { raw / 1000.0 } else { raw }
    }

    fn patched_last_message(
        wire: &RiseStoredChat,
        last_message: Option<&RiseStoredChatPreview>,
    ) -> anyhow::Result<RiseStoredChat> {
        RiseChatWireCodec::patch(
            wire,
            BTreeMap::from([(
                RiseNormalizedKey::last_message().to_owned(),
                RiseChatWireCodec::last_message_value(last_message)?,
            )]),
        )
    }

    /// unread_count on the card is only ever reset to 0 by chat-service, message-service owns the count
    /// keeps the stored one so leaving a chat doesnt zero it
    pub fn keeping_stored_unread_count(wire: RiseStoredChat, stored: &RiseStoredChat) -> anyhow::Result<RiseStoredChat> {
        let Some(stored_unread) = RiseChatWireCodec::object(stored)?.get(RiseNormalizedKey::unread_count()).cloned()
        else {
            return Ok(wire);
        };
        if matches!(stored_unread, RiseJSONValue::Null) {
            return Ok(wire);
        }
        RiseChatWireCodec::patch(&wire, BTreeMap::from([(RiseNormalizedKey::unread_count().to_owned(), stored_unread)]))
    }

    pub fn keeping_stored_preview(
        wire: &RiseStoredChat,
        stored: &RiseStoredChat,
    ) -> anyhow::Result<RiseStoredChat> {
        let stored_object = RiseChatWireCodec::object(stored)?;
        let mut updates = BTreeMap::new();
        for key in [
            RiseNormalizedKey::last_message(),
            RiseNormalizedKey::unread_info(),
            RiseNormalizedKey::unread_mentions_count(),
            RiseNormalizedKey::unread_replies_count(),
            RiseNormalizedKey::unread_reactions_count(),
            RiseNormalizedKey::unread_media_count(),
        ] {
            updates.insert(key.to_owned(), stored_object.get(key).cloned().unwrap_or(RiseJSONValue::Null));
        }
        RiseChatWireCodec::patch(wire, updates)
    }

    pub fn is_equivalent_server_page_data(existing: &[u8], fresh: &[u8]) -> bool {
        if existing == fresh {
            return true;
        }
        let Ok(record) = RiseChatWireCodec::decode_record(existing) else {
            return false;
        };
        if record.source != RiseConversationEntitySource::Realtime
            || record.locally_created
            || record.tombstone.is_some()
        {
            return false;
        }
        let relabelled = RiseChatEntityRecord {
            wire: record.wire,
            source: RiseConversationEntitySource::ServerPage,
            locally_created: false,
            list_kind: record.list_kind,
            tombstone: None,
        };
        RiseChatWireCodec::encode_record(&relabelled).is_ok_and(|encoded| encoded == fresh)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn make_page_batch(
        &self,
        rows: &[RiseChatRepositoryEncodedRow],
        preserved: &[RiseChatRepositoryPreservedMembership],
        kind: RiseChatListKind,
        page: &RiseChatRepositoryPage,
        entity_version: i64,
        state_version: i64,
        replaces_loaded_view: bool,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let namespace = RiseConversationStoreLayout::chats();
        let destination = kind.store_view().raw_value();
        let mut mutations = Vec::with_capacity(rows.len() * 8 + 6);
        if replaces_loaded_view {
            mutations.push(RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace: namespace.clone(),
                view: destination.to_owned(),
                if_version_at_most: Some(entity_version),
            });
            let row_ids: HashSet<&str> = rows.iter().map(|row| row.local_id.as_str()).collect();
            for item in preserved.iter().filter(|item| !row_ids.contains(item.local_id.as_str())) {
                mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                    namespace: namespace.clone(),
                    view: destination.to_owned(),
                    membership: RiseEntityViewMembership::new(
                        item.local_id.clone(),
                        item.position.clone(),
                        item.version,
                    ),
                });
            }
        }
        let mut already_merged: HashSet<&str> = HashSet::new();
        for row in rows {
            for duplicate in &row.duplicate_local_ids {
                if already_merged.insert(duplicate.as_str()) {
                    mutations.push(RiseStoreMutation::MergeEntities {
                        namespace: namespace.clone(),
                        survivor_local_id: row.local_id.clone(),
                        duplicate_local_id: duplicate.clone(),
                    });
                }
            }
            let row_version = row.entity_version.unwrap_or(entity_version);
            if !row.is_unchanged {
                mutations.push(RiseStoreMutation::UpsertEntity {
                    namespace: namespace.clone(),
                    entity: RiseStoreEntity::new(
                        row.local_id.clone(),
                        row.data.clone(),
                        row_version,
                        Self::now_milliseconds(),
                    ),
                    policy: RiseEntityWritePolicy::IgnoreIfNotNewer,
                });
            }
            for alias in &row.aliases {
                mutations.push(RiseStoreMutation::SetEntityAlias {
                    namespace: namespace.clone(),
                    alias: alias.clone(),
                    local_id: row.local_id.clone(),
                });
            }
            mutations.extend(self.exclusive_membership_mutations(
                &row.local_id,
                kind,
                row.position.clone(),
                row_version,
                row.revives_tombstone,
            ));
        }
        mutations.push(RiseStoreMutation::SetEntityViewState {
            namespace: namespace.clone(),
            view: destination.to_owned(),
            state: RiseEntityViewState::new(
                state_version,
                None,
                RiseConversationCodec::encode_cursor(page.cursor.as_deref()),
                false,
                page.has_more,
                RiseEntityViewLoadState::Loaded,
                None,
                Self::now_milliseconds(),
            ),
        });
        mutations.push(RiseStoreMutation::PruneUnreferencedEntities {
            namespace: namespace.clone(),
            limit: self.presentation_limit,
        });
        if let Some(aggregates) = page.aggregates.as_ref()
            && let Some(aggregate_key) = Self::aggregate_store_key(kind)
        {
            mutations.push(RiseStoreMutation::SetValue {
                namespace,
                key: aggregate_key.to_owned(),
                value: RiseStoredValue::new(
                    serde_json::to_vec(aggregates)?,
                    entity_version,
                    Self::now_milliseconds(),
                ),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::IgnoreIfNotNewer,
            });
        }
        Ok(Self::write_batch(mutations))
    }

    /// reads the whole durable membership and not just the head
    /// else a refresh dropped chats scrolled past it
    pub async fn preserved_memberships(
        &self,
        kind: RiseChatListKind,
    ) -> anyhow::Result<Vec<RiseChatRepositoryPreservedMembership>> {
        let limit = self.presentation_limit as usize;
        let mut memberships = Vec::new();
        let mut window = RiseEntityWindow::First { limit };
        loop {
            let result = self
                .engine
                .entity_window(RiseEntityWindowQuery::new(
                    RiseConversationStoreLayout::chats(),
                    kind.store_view().raw_value(),
                    window,
                ))
                .await?;
            memberships.extend(result.items.iter().filter_map(|item| {
                let record = RiseChatWireCodec::decode_record(&item.entity.data).ok()?;
                if record.tombstone.is_some() {
                    return None;
                }
                Some(RiseChatRepositoryPreservedMembership {
                    local_id: item.entity.local_id.clone(),
                    position: item.position_key.clone(),
                    version: item.membership_version,
                    is_local: record.locally_created || record.source == RiseConversationEntitySource::Optimistic,
                })
            }));
            let Some(last) = result.items.last().filter(|_| result.has_cached_after) else {
                break;
            };
            window = RiseEntityWindow::After {
                anchor: RiseEntityViewPosition::new(last.position_key.clone(), last.entity.local_id.clone()),
                limit,
            };
        }
        Ok(memberships)
    }

    fn write_batch(mutations: Vec<RiseStoreMutation>) -> RiseStoreWriteBatch {
        RiseStoreWriteBatch::new(mutations, HashSet::new())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn make_upsert_batch(
        &self,
        record: RiseChatEntityRecord,
        local_id: &str,
        aliases: Vec<RiseEntityAlias>,
        destination: Option<RiseChatListKind>,
        version: i64,
        clears_tombstone: bool,
        duplicate_local_ids: Vec<String>,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let namespace = RiseConversationStoreLayout::chats();
        let projected_entity = RiseStoreEntity::new(
            local_id,
            RiseChatWireCodec::encode_record(&record)?,
            version,
            Self::now_milliseconds(),
        );
        let position = RiseConversationStoreLayout::chat_position(&record.wire);
        self.install_projection(
            &RiseChatStoredEntity { entity: projected_entity.clone(), record },
            aliases.clone(),
            false,
        );
        let mut mutations: Vec<RiseStoreMutation> = duplicate_local_ids
            .into_iter()
            .map(|duplicate_local_id| RiseStoreMutation::MergeEntities {
                namespace: namespace.clone(),
                survivor_local_id: local_id.to_owned(),
                duplicate_local_id,
            })
            .collect();
        mutations.push(RiseStoreMutation::UpsertEntity {
            namespace: namespace.clone(),
            entity: projected_entity,
            policy: RiseEntityWritePolicy::ServerAuthoritative,
        });
        mutations.extend(aliases.into_iter().map(|alias| RiseStoreMutation::SetEntityAlias {
            namespace: namespace.clone(),
            alias,
            local_id: local_id.to_owned(),
        }));
        if let Some(destination) = destination {
            mutations.extend(self.exclusive_membership_mutations(
                local_id,
                destination,
                position,
                version,
                clears_tombstone,
            ));
        } else {
            for view in RiseConversationStoreLayoutChatView::ALL_CASES {
                mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                    namespace: namespace.clone(),
                    view: view.raw_value().to_owned(),
                    local_id: local_id.to_owned(),
                    if_version_at_most: Some(version),
                });
            }
        }
        Ok(Self::write_batch(mutations))
    }

    pub fn make_tombstone_batch(
        &self,
        stored: &RiseChatStoredEntity,
        tombstone: RiseChatTombstone,
        version: i64,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let record = RiseChatEntityRecord {
            wire: stored.record.wire.clone(),
            source: if tombstone.is_confirmed() {
                RiseConversationEntitySource::Realtime
            } else {
                RiseConversationEntitySource::Optimistic
            },
            locally_created: stored.record.locally_created,
            list_kind: None,
            tombstone: Some(tombstone),
        };
        let namespace = RiseConversationStoreLayout::chats();
        let local_id = stored.entity.local_id.as_str();
        let aliases = RiseConversationStoreLayout::chat_aliases(&record.wire);
        let batch = self.make_upsert_batch(
            record,
            local_id,
            aliases,
            None,
            version,
            false,
            Vec::new(),
        )?;
        let mut mutations = batch.mutations;
        mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
            namespace: namespace.clone(),
            view: Self::TOMBSTONE_VIEW.to_owned(),
            membership: RiseEntityViewMembership::new(
                local_id,
                Self::tombstone_position(version, local_id),
                version,
            ),
        });
        mutations.push(RiseStoreMutation::TrimEntityView {
            namespace: namespace.clone(),
            view: Self::TOMBSTONE_VIEW.to_owned(),
            keeping: RiseEntityViewRetention::Latest(self.presentation_limit as usize),
            if_version_at_most: None,
        });
        mutations.push(RiseStoreMutation::PruneUnreferencedEntities { namespace, limit: self.presentation_limit });
        Ok(Self::write_batch(mutations))
    }

    pub fn exclusive_membership_mutations(
        &self,
        local_id: &str,
        destination: RiseChatListKind,
        position: RisePositionKey,
        version: i64,
        clears_tombstone: bool,
    ) -> Vec<RiseStoreMutation> {
        let namespace = RiseConversationStoreLayout::chats();
        let mut mutations = Vec::new();
        for view in RiseConversationStoreLayoutChatView::ALL_CASES {
            if view == destination.store_view() {
                continue;
            }
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: namespace.clone(),
                view: view.raw_value().to_owned(),
                local_id: local_id.to_owned(),
                if_version_at_most: Some(version),
            });
        }
        if clears_tombstone {
            mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                namespace: namespace.clone(),
                view: Self::TOMBSTONE_VIEW.to_owned(),
                local_id: local_id.to_owned(),
                if_version_at_most: Some(version),
            });
        }
        mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
            namespace,
            view: destination.store_view().raw_value().to_owned(),
            membership: RiseEntityViewMembership::new(local_id, position, version),
        });
        mutations
    }

    pub async fn aggregate_mutation(
        &self,
        kind: RiseChatListKind,
        version: i64,
        delta: &RiseChatAggregateDelta,
    ) -> anyhow::Result<Option<RiseStoreMutation>> {
        let Some(key) = Self::aggregate_store_key(kind).filter(|_| !delta.is_zero()) else {
            return Ok(None);
        };
        let projection = self.state.with(|state| state.projected_aggregates.get(&kind).cloned());
        let (current_version, current) = if let Some(projection) = projection {
            projection
        } else {
            let stored =
                self.engine.value(RiseConversationStoreLayout::chats(), key.to_owned()).await?;
            let stored_version = stored.as_ref().map(|stored| stored.version);
            let newer_projection = self.state.with(|state| {
                if let Some(stored_version) = stored_version {
                    state.version_clock = state.version_clock.max(stored_version);
                }
                state
                    .projected_aggregates
                    .get(&kind)
                    .filter(|projection| projection.0 >= stored_version.unwrap_or(i64::MIN))
                    .cloned()
            });
            if let Some(projection) = newer_projection {
                projection
            } else {
                match stored {
                    Some(stored) => (stored.version, serde_json::from_slice(&stored.data)?),
                    None => (i64::MIN, RiseChatListAggregates::default()),
                }
            }
        };
        let updated = current.applying(delta);
        let aggregate_version =
            if current_version == i64::MAX { i64::MAX } else { version.max(current_version + 1) };
        self.install_aggregate_projection(kind, updated, aggregate_version);
        Ok(Some(RiseStoreMutation::SetValue {
            namespace: RiseConversationStoreLayout::chats(),
            key: key.to_owned(),
            value: RiseStoredValue::new(
                serde_json::to_vec(&updated)?,
                aggregate_version,
                Self::now_milliseconds(),
            ),
            precondition: RiseStoreVersionPrecondition::Any,
            policy: RiseStoredValueWritePolicy::ServerAuthoritative,
        }))
    }

    /// puts the aggregate changes of every affected list into the batch
    pub async fn merging_aggregates(
        &self,
        batch: RiseStoreWriteBatch,
        version: i64,
        deltas: Vec<(RiseChatListKind, RiseChatAggregateDelta)>,
    ) -> anyhow::Result<RiseStoreWriteBatch> {
        let mut mutations = Vec::new();
        for (kind, delta) in deltas {
            if let Some(mutation) = self.aggregate_mutation(kind, version, &delta).await? {
                mutations.push(mutation);
            }
        }
        if mutations.is_empty() {
            return Ok(batch);
        }
        Ok(RiseStoreWriteBatch::conversation_merging(vec![batch, Self::write_batch(mutations)]))
    }

    pub async fn refresh_unread_badge(&self, force: bool) -> Option<RiseChatListAggregates> {
        let now = Self::now_milliseconds();
        let interval = Self::badge_refresh_interval_ms();
        let generation = self.state.with(|state| {
            if !force && state.badge_refreshed_at_ms > 0 && now - state.badge_refreshed_at_ms < interval {
                return None;
            }
            state.badge_refresh_generation = state.badge_refresh_generation.wrapping_add(1);
            Some(state.badge_refresh_generation)
        })?;
        let response: rise_response::chat::GetUnreadSummary = self
            .rpc
            .call(RiseConversationEndpoints::get_unread_summary(), rise_request::chat::GetUnreadSummary {})
            .await
            .ok()?;
        if self.state.with(|state| state.badge_refresh_generation) != generation {
            return None;
        }
        let aggregates = RiseChatListAggregates::new(
            None,
            Some(response.total_unread_count),
            Some(response.unread_chats_count),
            Some(response.total_unread_media_count),
            Some(response.total_unread_replies_count),
        );
        let version = self.next_version();
        self.install_aggregate_projection(RiseChatListKind::Inbox, aggregates, version);
        self.bump_server_inbox_badge_generation();
        let written: anyhow::Result<()> = async {
            self.engine
                .write(Self::write_batch(vec![RiseStoreMutation::SetValue {
                    namespace: RiseConversationStoreLayout::chats(),
                    key: Self::INBOX_BADGE_KEY.to_owned(),
                    value: RiseStoredValue::new(
                        serde_json::to_vec(&aggregates)?,
                        version,
                        Self::now_milliseconds(),
                    ),
                    precondition: RiseStoreVersionPrecondition::Any,
                    policy: RiseStoredValueWritePolicy::ServerAuthoritative,
                }]))
                .await?;
            Ok(())
        }
        .await;
        let refreshed_at = Self::now_milliseconds();
        self.state.with(|state| {
            if written.is_ok() {
                state.badge_refreshed_at_ms = refreshed_at;
            } else {
                state.projected_aggregates.remove(&RiseChatListKind::Inbox);
            }
        });
        Some(aggregates)
    }

    pub fn decode_stored(&self, entity: RiseStoreEntity) -> anyhow::Result<RiseChatStoredEntity> {
        if let Some(projected) = self.projected_chat_local_id(&entity.local_id)
            && projected.entity.version >= entity.version
        {
            return Ok(projected);
        }
        self.state.with(|state| state.version_clock = state.version_clock.max(entity.version));
        let record = RiseChatWireCodec::decode_record(&entity.data)?;
        Ok(RiseChatStoredEntity { entity, record })
    }

    pub fn next_version(&self) -> i64 {
        let now = Self::now_milliseconds();
        self.state.with(|state| {
            state.version_clock = (state.version_clock + 1).max(now);
            state.version_clock
        })
    }

    pub fn projected_chat_id(&self, id: &str) -> Option<RiseChatStoredEntity> {
        for kind in Self::alias_kinds(id) {
            let alias = RiseEntityAlias::new(kind, id);
            let local_id = self.state.with(|state| state.projected_aliases.get(&alias).cloned());
            if let Some(projected) = local_id.and_then(|local_id| self.projected_chat_local_id(&local_id)) {
                return Some(projected);
            }
        }
        self.projected_chat_local_id(id)
    }

    pub fn projected_chat_local_id(&self, local_id: &str) -> Option<RiseChatStoredEntity> {
        self.state.with(|state| {
            state.projected_chats.get(local_id)?;
            state.projection_access = state.projection_access.wrapping_add(1);
            let access = state.projection_access;
            let entry = state.projected_chats.get_mut(local_id)?;
            entry.access = access;
            Some(entry.stored.clone())
        })
    }

    pub fn install_projection(&self, stored: &RiseChatStoredEntity, aliases: Vec<RiseEntityAlias>, force: bool) {
        let projection_limit = self.projection_limit;
        self.state.with(|state| {
            if !force
                && state
                    .projected_chats
                    .get(&stored.entity.local_id)
                    .is_some_and(|current| current.stored.entity.version > stored.entity.version)
            {
                return;
            }
            state.projection_access = state.projection_access.wrapping_add(1);
            let local_id = stored.entity.local_id.clone();
            for alias in aliases {
                state.projected_aliases.insert(alias, local_id.clone());
            }
            let access = state.projection_access;
            state
                .projected_chats
                .insert(local_id, RiseChatRepositoryProjectedChat { stored: stored.clone(), access });
            state.evict_projections(projection_limit);
        });
    }

    pub fn restore_projection(&self, stored: &RiseChatStoredEntity) {
        self.state.with(|state| state.projected_chats.remove(&stored.entity.local_id));
        let aliases = RiseConversationStoreLayout::chat_aliases(&stored.record.wire);
        self.install_projection(stored, aliases, true);
    }

    /// page batch is already durable here so drop an older or equal projection
    /// newer one stays until its commit lands
    pub fn drop_projection(&self, local_id: &str) {
        self.state.with(|state| {
            if state.projected_chats.contains_key(local_id) {
                state.remove_projection(local_id);
            }
        });
    }

    pub fn reconcile_page_projection(&self, rows: &[RiseChatRepositoryEncodedRow], committed_version: i64) {
        self.state.with(|state| {
            for row in rows {
                let committed = row.entity_version.unwrap_or(committed_version);
                let is_older = state
                    .projected_chats
                    .get(&row.local_id)
                    .is_some_and(|projected| projected.stored.entity.version <= committed);
                if is_older {
                    state.remove_projection(&row.local_id);
                }
            }
        });
    }

    /// clears only the pre commit reducer state, durable data and optimistic bookkeeping stay
    /// call around a full resync or after a failed external write
    pub fn reset_reducer_projection(&self) {
        self.state.with(|state| {
            state.projected_chats.clear();
            state.projected_aliases.clear();
            state.projected_aggregates.clear();
        });
    }

    /// counts every server answer that replaced the inbox badge
    /// a read in flight across one may be counted by the server after it
    pub fn server_inbox_badge_generation(&self) -> u64 {
        self.state.with(|state| state.server_inbox_badge_generation)
    }

    fn bump_server_inbox_badge_generation(&self) {
        self.state.with(|state| state.server_inbox_badge_generation = state.server_inbox_badge_generation.wrapping_add(1));
    }

    pub fn install_aggregate_projection(
        &self,
        kind: RiseChatListKind,
        aggregates: RiseChatListAggregates,
        version: i64,
    ) {
        self.state.with(|state| {
            let current = state.projected_aggregates.get(&kind).map_or(i64::MIN, |projection| projection.0);
            if current <= version {
                state.projected_aggregates.insert(kind, (version, aggregates));
            }
        });
    }

    pub fn evict_projections_if_needed(&self) {
        let projection_limit = self.projection_limit;
        self.state.with(|state| state.evict_projections(projection_limit));
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn write_state(
        &self,
        kind: RiseChatListKind,
        version: i64,
        before_cursor: Option<Vec<u8>>,
        after_cursor: Option<Vec<u8>>,
        has_more_before: bool,
        has_more_after: bool,
        load_state: RiseEntityViewLoadState,
        error_code: Option<String>,
    ) -> anyhow::Result<()> {
        self.engine
            .write(Self::write_batch(vec![RiseStoreMutation::SetEntityViewState {
                namespace: RiseConversationStoreLayout::chats(),
                view: kind.store_view().raw_value().to_owned(),
                state: RiseEntityViewState::new(
                    version,
                    before_cursor,
                    after_cursor,
                    has_more_before,
                    has_more_after,
                    load_state,
                    error_code,
                    Self::now_milliseconds(),
                ),
            }]))
            .await?;
        Ok(())
    }

    /// write_state that keeps the cursors of the state read before the request
    async fn write_state_keeping_cursors(
        &self,
        kind: RiseChatListKind,
        version: i64,
        current_state: Option<&RiseEntityViewState>,
        has_more_after: bool,
        load_state: RiseEntityViewLoadState,
        error_code: Option<String>,
    ) -> anyhow::Result<()> {
        self.write_state(
            kind,
            version,
            current_state.and_then(|state| state.before_cursor.clone()),
            current_state.and_then(|state| state.after_cursor.clone()),
            current_state.is_some_and(|state| state.has_more_before),
            has_more_after,
            load_state,
            error_code,
        )
        .await
    }

    pub fn window_query(&self, kind: RiseChatListKind) -> RiseEntityWindowQuery {
        RiseEntityWindowQuery::new(
            RiseConversationStoreLayout::chats(),
            kind.store_view().raw_value(),
            RiseEntityWindow::First { limit: self.presentation_limit as usize },
        )
    }

    pub async fn presentation_snapshot(
        snapshot: &RiseStoreSnapshot,
        query: &RiseEntityWindowQuery,
        kind: RiseChatListKind,
        account_id: &str,
        decoded_record_cache: &RiseChatDecodedRecordCache,
    ) -> anyhow::Result<RiseChatListWireSnapshot> {
        let empty = RiseEntityWindowResult::new(Vec::new(), None, 0, false, false, None, None);
        let result = snapshot.entity_windows.get(query).unwrap_or(&empty);
        let records = decoded_record_cache.records(&result.items).await?;
        let aggregates: Option<RiseChatListAggregates> = match Self::aggregate_store_key(kind).and_then(|key| {
            snapshot.values.get(&RiseStoreValueKey::new(RiseConversationStoreLayout::chats(), key))
        }) {
            Some(stored) => Some(serde_json::from_slice(&stored.data)?),
            None => None,
        };
        let items: &[RiseEntityViewItem] = &result.items;
        Ok(RiseChatListWireSnapshot {
            account_id: account_id.to_owned(),
            kind,
            revision: snapshot.revision,
            rows: records,
            local_ids: items.iter().map(|item| item.entity.local_id.clone()).collect(),
            entity_stamps: items
                .iter()
                .map(|item| RiseConversationEntityStamp {
                    version: item.entity.version,
                    updated_at_milliseconds: item.entity.updated_at_milliseconds,
                })
                .collect(),
            loaded_row_count: result.total_count as i64,
            server_total_count: aggregates.as_ref().and_then(|value| value.total_count),
            server_total_unread_count: aggregates.as_ref().and_then(|value| value.total_unread_count),
            server_unread_chats_count: aggregates.as_ref().and_then(|value| value.unread_chats_count),
            server_total_unread_media_count: aggregates.as_ref().and_then(|value| value.total_unread_media_count),
            cursor: RiseConversationCodec::decode_cursor(
                result.state.as_ref().and_then(|state| state.after_cursor.as_deref()),
            ),
            has_more: result.state.as_ref().is_some_and(|state| state.has_more_after),
            load_state: result.state.as_ref().map_or(RiseEntityViewLoadState::Idle, |state| state.load_state),
            error_code: result.state.as_ref().and_then(|state| state.last_error_code.clone()),
        })
    }

    pub fn aggregate_store_key(kind: RiseChatListKind) -> Option<&'static str> {
        match kind {
            RiseChatListKind::Archive => Some(Self::ARCHIVE_AGGREGATES_KEY),
            RiseChatListKind::Inbox => Some(Self::INBOX_BADGE_KEY),
            RiseChatListKind::Requests => Some(Self::REQUESTS_AGGREGATES_KEY),
        }
    }

    pub fn tombstone_position(version: i64, local_id: &str) -> RisePositionKey {
        RisePositionKey::make(|builder| {
            builder.append_signed_int64(version);
            builder.append_string(local_id);
        })
    }

    pub fn error_code(error: &anyhow::Error) -> String {
        if error.is::<CancellationError>() {
            return Self::CANCELLED_ERROR_CODE.to_owned();
        }
        if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            return remote.code.clone();
        }
        if let Some(wire) = error.downcast_ref::<RiseWireError>() {
            return wire.to_string();
        }
        "conversation_request_failed".to_owned()
    }

    pub fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
