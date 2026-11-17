use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::BoxFuture;
use rise_canon::rise_decode;
use rise_canon::rise_key::bot as rise_key_bot;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityAliasKind, RiseEntityViewLoadState, RiseEntityViewMembership, RiseEntityViewState,
    RiseEntityWritePolicy, RisePositionKey, RiseStoreEntity,
};
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch, RiseStoredValue,
    RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine, RiseJSONValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineImporter;
use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepository;
use crate::modules::chat::engine::core::rise_conversation_codec::RiseConversationCodec;
use crate::modules::chat::engine::core::rise_conversation_entity_records::{
    RiseChatEntityRecord, RiseConversationEntitySource, RiseMessageEntityRecord, RiseMessageLocalPhase,
};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseChatListKind;
use crate::modules::chat::engine::core::rise_conversation_store_layout::{
    RiseConversationStoreLayout, RiseConversationStoreLayoutChatView,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::message::message_actions::message_wire_types::MessageWireDTO;

const MARKER_SCHEMA_VERSION: i64 = 4;
const MAXIMUM_IDENTIFIER_BYTES: usize = 4_096;
const MAXIMUM_SCOPE_STORAGE_KEY_BYTES: usize = 3_900;
const VERSION_MODULUS: u64 = 999_999_999;
const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
const FNV_PRIME: u64 = 1_099_511_628_211;
const CANCELLATION_CHECK_INTERVAL: usize = 32;

/// defaulted params of the legacy importer config
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseLegacyConversationImporterConfigurationArgs {
    pub maximum_rows_per_batch: usize,
    pub maximum_mutations_per_batch: usize,
    pub maximum_batch_value_bytes: usize,
    pub maximum_entity_bytes: usize,
}

impl Default for RiseLegacyConversationImporterConfigurationArgs {
    fn default() -> Self {
        Self {
            maximum_rows_per_batch: 96,
            maximum_mutations_per_batch: 1_800,
            maximum_batch_value_bytes: 4 * 1_024 * 1_024,
            maximum_entity_bytes: 1_024 * 1_024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseLegacyConversationImporterConfiguration {
    pub maximum_rows_per_batch: usize,
    pub maximum_mutations_per_batch: usize,
    pub maximum_batch_value_bytes: usize,
    pub maximum_entity_bytes: usize,
}

impl RiseLegacyConversationImporterConfiguration {
    pub fn new(args: RiseLegacyConversationImporterConfigurationArgs) -> Self {
        assert!(args.maximum_rows_per_batch > 0 && args.maximum_rows_per_batch <= 256);
        assert!(args.maximum_mutations_per_batch > 0 && args.maximum_mutations_per_batch <= 1_800);
        assert!(args.maximum_entity_bytes > 0);
        assert!(args.maximum_batch_value_bytes >= args.maximum_entity_bytes);
        Self {
            maximum_rows_per_batch: args.maximum_rows_per_batch,
            maximum_mutations_per_batch: args.maximum_mutations_per_batch,
            maximum_batch_value_bytes: args.maximum_batch_value_bytes,
            maximum_entity_bytes: args.maximum_entity_bytes,
        }
    }
}

impl Default for RiseLegacyConversationImporterConfiguration {
    fn default() -> Self {
        Self::new(RiseLegacyConversationImporterConfigurationArgs::default())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseLegacyConversationImporterSummary {
    pub source_payload_count: usize,
    pub chat_count: usize,
    pub message_count: usize,
    pub chat_view_count: usize,
    pub message_scope_count: usize,
    pub skipped_row_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseLegacyConversationImporterOutcome {
    AlreadyImported,
    Imported(RiseLegacyConversationImporterSummary),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseLegacyConversationImporterError {
    #[error("The legacy cache payload {0} is invalid")]
    InvalidCachePayload(String),
    #[error("The legacy message payload is invalid")]
    InvalidMessagePayload,
    #[error("The legacy entity {local_id} takes {byte_count} bytes, over the limit of {limit}")]
    EntityTooLarge { local_id: String, byte_count: usize, limit: usize },
    #[error("The legacy row {local_id} needs {mutation_count} mutations, over the limit of {limit}")]
    RowMutationLimitExceeded { local_id: String, mutation_count: usize, limit: usize },
}

#[derive(Serialize, Deserialize)]
struct Marker {
    #[serde(rename = "schemaVersion")]
    schema_version: i64,
    #[serde(rename = "accountID")]
    account_id: String,
    #[serde(rename = "completedAtMilliseconds")]
    completed_at_milliseconds: i64,
    summary: RiseLegacyConversationImporterSummary,
}

#[derive(Default)]
struct SourceIndex {
    chat_keys: HashMap<RiseChatListKind, String>,
    message_keys: HashMap<RiseMessageScope, MessageSourceKeys>,
}

#[derive(Clone, Default)]
struct MessageSourceKeys {
    canonical: Option<String>,
    latest: Option<String>,
}

struct ChatEnvelope {
    chats: Option<Vec<RiseStoredChat>>,
    relative_id: Option<String>,
    is_have_more: Option<bool>,
    has_more: Option<bool>,
    total_count: Option<i64>,
    total_unread_count: Option<i64>,
    unread_chats_count: Option<i64>,
}

impl<'de> Deserialize<'de> for ChatEnvelope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                chats: container.rise("chats")?,
                relative_id: container.rise(rise_key_bot::RELATIVE_ID)?,
                is_have_more: container.rise("is_have_more")?,
                has_more: container.rise(rise_key_bot::HAS_MORE)?,
                total_count: container.rise("total_count")?,
                total_unread_count: container.rise("total_unread_count")?,
                unread_chats_count: container.rise("unread_chats_count")?,
            })
        })
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveAggregates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    total_unread_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unread_chats_count: Option<i64>,
}

struct ChatPage {
    kind: RiseChatListKind,
    rows: Vec<RiseStoredChat>,
    cursor: Option<String>,
    has_more: bool,
    archive_aggregates: Option<ArchiveAggregates>,
}

struct HistoryLaneEnvelope {
    relative_id: Option<String>,
    has_more: Option<bool>,
}

impl<'de> Deserialize<'de> for HistoryLaneEnvelope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                relative_id: container.rise(rise_key_bot::RELATIVE_ID)?,
                has_more: container.rise(rise_key_bot::HAS_MORE)?,
            })
        })
    }
}

struct HistoryPaginationEnvelope {
    older: Option<HistoryLaneEnvelope>,
    newer: Option<HistoryLaneEnvelope>,
}

impl<'de> Deserialize<'de> for HistoryPaginationEnvelope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self { older: container.rise("older")?, newer: container.rise("newer")? })
        })
    }
}

struct MessageEnvelope {
    messages: Option<Vec<MessageWireDTO>>,
    relative_id: Option<String>,
    is_have_more: Option<bool>,
    history_pagination: Option<HistoryPaginationEnvelope>,
}

impl<'de> Deserialize<'de> for MessageEnvelope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                messages: container.rise("messages")?,
                relative_id: container.rise(rise_key_bot::RELATIVE_ID)?,
                is_have_more: container.rise("is_have_more")?,
                history_pagination: container.rise("history_pagination")?,
            })
        })
    }
}

#[derive(Clone)]
struct HistoryLane {
    cursor: Option<String>,
    has_more: bool,
}

struct MessagePage {
    rows: Vec<MessageWireDTO>,
    older: HistoryLane,
    newer: HistoryLane,
}

#[derive(Clone)]
struct ChatCandidate {
    wire: RiseStoredChat,
    local_id: String,
    local_id_is_temporary: bool,
    aliases: HashSet<RiseEntityAlias>,
    tokens: HashSet<String>,
    kind: RiseChatListKind,
}

#[derive(Clone)]
struct MessageCandidate {
    wire: MessageWireDTO,
    local_id: String,
    local_id_is_temporary: bool,
    aliases: HashSet<RiseEntityAlias>,
    tokens: HashSet<String>,
}

struct PreparedChatRow {
    local_id: String,
    data: Vec<u8>,
    position: RisePositionKey,
    aliases: Vec<RiseEntityAlias>,
    kind: RiseChatListKind,
    version: i64,
}

struct PreparedMessageRow {
    local_id: String,
    data: Vec<u8>,
    position: RisePositionKey,
    aliases: Vec<RiseEntityAlias>,
    version: i64,
}

struct PreparedChatImport {
    rows: Vec<PreparedChatRow>,
    skipped_row_count: usize,
}

struct PreparedMessageImport {
    rows: Vec<PreparedMessageRow>,
    skipped_row_count: usize,
}

struct ViewStateSeed {
    namespace: RiseStoreNamespace,
    view: String,
    before_cursor: Option<String>,
    after_cursor: Option<String>,
    has_more_before: bool,
    has_more_after: bool,
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

async fn detached<Output, Work>(work: Work) -> anyhow::Result<Output>
where
    Output: Send + 'static,
    Work: FnOnce(&AtomicBool) -> anyhow::Result<Output> + Send + 'static,
{
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel = CancelOnDrop(cancelled.clone());
    RiseAccountRuntime::current()
        .spawn_blocking(move || work(&cancelled))
        .await
        .map_err(|_| CancellationError)?
}

fn check_cancellation(cancelled: &AtomicBool) -> anyhow::Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(CancellationError.into());
    }
    Ok(())
}

fn encode_sorted<Value: Serialize + ?Sized>(value: &Value) -> anyhow::Result<Vec<u8>> {
    Ok(RiseJSONValue::encode(value)?.data(true)?)
}

/// single pass of the importer over the old disk cache
/// an aborted task is dropped at its next await, only blocking steps are not cancellable
#[derive(Clone, Copy, Debug, Default)]
pub struct RiseLegacyConversationImporter {
    configuration: RiseLegacyConversationImporterConfiguration,
}

impl RiseAppEngineImporter for RiseLegacyConversationImporter {
    fn import_data<'a>(&'a self, engine: &'a RiseEngine) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.import_if_needed(engine).await?;
            Ok(())
        })
    }
}

impl RiseLegacyConversationImporter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_configuration(configuration: RiseLegacyConversationImporterConfiguration) -> Self {
        Self { configuration }
    }

    pub fn marker_namespace() -> RiseStoreNamespace {
        RiseStoreNamespace::new("conversation.legacy-migrations.v1")
    }

    pub fn marker_key(account_id: &str) -> String {
        format!("legacy-sai.normalized-v4.{account_id}")
    }

    pub async fn import_if_needed(&self, engine: &RiseEngine) -> anyhow::Result<RiseLegacyConversationImporterOutcome> {
        let account_id = engine.account.account_id.clone();
        let marker_key = Self::marker_key(&account_id);
        if engine.value(Self::marker_namespace(), marker_key.clone()).await?.is_some() {
            return Ok(RiseLegacyConversationImporterOutcome::AlreadyImported);
        }

        let source_index = Self::load_source_index(account_id.clone()).await?;
        let mut source_payload_count = 0;
        let mut skipped_row_count = 0;

        let mut chat_pages = Vec::new();
        for kind in RiseChatListKind::ALL_CASES {
            let Some(cache_key) = source_index.chat_keys.get(&kind) else { continue };
            let Some(page) = Self::load_chat_page(cache_key.clone(), kind).await? else { continue };
            chat_pages.push(page);
            source_payload_count += 1;
        }
        let chat_pages = Arc::new(chat_pages);

        let chat_import = Self::prepare_chats(chat_pages.clone()).await?;
        skipped_row_count += chat_import.skipped_row_count;
        self.write_chats(&chat_import.rows, engine).await?;
        self.write_archive_aggregates(&chat_pages, engine).await?;

        let mut state_seeds: Vec<ViewStateSeed> = chat_pages.iter().map(Self::chat_state_seed).collect();
        let mut message_count = 0;
        let mut message_scope_count = 0;

        let mut scopes: Vec<&RiseMessageScope> = source_index.message_keys.keys().collect();
        scopes.sort_by_cached_key(|scope| scope.storage_key());
        for scope in scopes {
            let Some(keys) = source_index.message_keys.get(scope) else { continue };
            let (canonical_page, latest_page) = futures::try_join!(
                Self::load_message_page(keys.canonical.clone()),
                Self::load_message_page(keys.latest.clone()),
            )?;
            let loaded_payload_count = usize::from(canonical_page.is_some()) + usize::from(latest_page.is_some());
            if loaded_payload_count == 0 {
                continue;
            }

            source_payload_count += loaded_payload_count;
            message_scope_count += 1;
            let merged_rows: Vec<MessageWireDTO> = canonical_page
                .iter()
                .chain(latest_page.iter())
                .flat_map(|page| page.rows.iter().cloned())
                .collect();
            let message_import = Self::prepare_messages(merged_rows, scope.clone()).await?;
            skipped_row_count += message_import.skipped_row_count;
            message_count += message_import.rows.len();
            self.write_messages(&message_import.rows, scope, engine).await?;
            state_seeds.push(Self::message_state_seed(scope, canonical_page.as_ref(), latest_page.as_ref()));
        }

        self.write_missing_states(state_seeds, engine).await?;

        let summary = RiseLegacyConversationImporterSummary {
            source_payload_count,
            chat_count: chat_import.rows.len(),
            message_count,
            chat_view_count: chat_pages.len(),
            message_scope_count,
            skipped_row_count,
        };
        let completed_at = Self::now_milliseconds();
        let marker = Marker {
            schema_version: MARKER_SCHEMA_VERSION,
            account_id,
            completed_at_milliseconds: completed_at,
            summary,
        };
        let marker_data = RiseConversationCodec::encode(marker).await?;

        let written = engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: Self::marker_namespace(),
                    key: marker_key.clone(),
                    value: RiseStoredValue::new(marker_data, MARKER_SCHEMA_VERSION, completed_at),
                    precondition: RiseStoreVersionPrecondition::Missing,
                    policy: RiseStoredValueWritePolicy::RequireIdenticalReplay,
                }],
                HashSet::new(),
            ))
            .await;
        if let Err(error) = written
            && engine.value(Self::marker_namespace(), marker_key).await?.is_none()
        {
            return Err(error);
        }

        Ok(RiseLegacyConversationImporterOutcome::Imported(summary))
    }

    async fn load_source_index(account_id: String) -> anyhow::Result<SourceIndex> {
        detached(move |cancelled| {
            check_cancellation(cancelled)?;
            SaiWsDiskCache::wait_for_asynchronous_writes();
            check_cancellation(cancelled)?;
            let account_prefix = format!("account:{account_id}:");
            let inbox_key = format!("getChats-{account_id}");
            let archive_key = format!("getArchivedChats-{account_id}");
            let requests_key = format!("getMessageRequests-{account_id}");
            let mut result = SourceIndex::default();

            for cache_key in SaiWsDiskCache::all_indexed_cache_keys() {
                let Some(logical_key) = cache_key.strip_prefix(&account_prefix) else { continue };
                check_cancellation(cancelled)?;
                if logical_key == inbox_key {
                    result.chat_keys.insert(RiseChatListKind::Inbox, cache_key);
                } else if logical_key == archive_key {
                    result.chat_keys.insert(RiseChatListKind::Archive, cache_key);
                } else if logical_key == requests_key {
                    result.chat_keys.insert(RiseChatListKind::Requests, cache_key);
                } else if let Some(storage_key) = logical_key.strip_prefix("getMessagesLatest-") {
                    let Some(scope) = Self::valid_scope(storage_key) else { continue };
                    result.message_keys.entry(scope).or_default().latest = Some(cache_key);
                } else if let Some(storage_key) = logical_key.strip_prefix("getMessages-") {
                    let Some(scope) = Self::valid_scope(storage_key) else { continue };
                    result.message_keys.entry(scope).or_default().canonical = Some(cache_key);
                }
            }
            Ok(result)
        })
        .await
    }

    async fn load_chat_page(cache_key: String, kind: RiseChatListKind) -> anyhow::Result<Option<ChatPage>> {
        let failed_key = cache_key.clone();
        let loaded = detached(move |cancelled| {
            let Some(envelope) = Self::decode_disk_payload::<ChatEnvelope>(&cache_key, cancelled)? else {
                return Ok(None);
            };
            let rows = envelope.chats.unwrap_or_default();
            let fallback = rows.iter().rev().find_map(|chat| Self::normalized_identifier(Some(&chat.id)));
            let cursor = Self::normalized_cursor(envelope.relative_id.as_deref()).or(fallback);
            let raw_has_more = envelope.is_have_more.or(envelope.has_more).unwrap_or(false);
            let has_more = raw_has_more && cursor.is_some();
            Ok(Some(ChatPage {
                kind,
                rows,
                cursor,
                has_more,
                archive_aggregates: (kind == RiseChatListKind::Archive).then_some(ArchiveAggregates {
                    total_count: envelope.total_count,
                    total_unread_count: envelope.total_unread_count,
                    unread_chats_count: envelope.unread_chats_count,
                }),
            }))
        })
        .await;
        Self::cache_payload_result(loaded, failed_key)
    }

    async fn load_message_page(cache_key: Option<String>) -> anyhow::Result<Option<MessagePage>> {
        let Some(cache_key) = cache_key else { return Ok(None) };
        let failed_key = cache_key.clone();
        let loaded = detached(move |cancelled| {
            let Some(envelope) = Self::decode_disk_payload::<MessageEnvelope>(&cache_key, cancelled)? else {
                return Ok(None);
            };
            let rows = envelope.messages.unwrap_or_default();
            let older_fallback = Self::boundary_message_id(&rows, true);
            let newer_fallback = Self::boundary_message_id(&rows, false);
            let (older_envelope, newer_envelope) = match envelope.history_pagination {
                Some(pagination) => (pagination.older, pagination.newer),
                None => (None, None),
            };
            let (older_cursor, older_has_more) = match older_envelope {
                Some(lane) => (lane.relative_id, lane.has_more),
                None => (None, None),
            };
            let older = Self::resolved_lane(
                older_cursor.or(envelope.relative_id).as_deref(),
                older_has_more.or(envelope.is_have_more).unwrap_or(false),
                older_fallback,
            );
            let (newer_cursor, newer_has_more) = match newer_envelope {
                Some(lane) => (lane.relative_id, lane.has_more),
                None => (None, None),
            };
            let newer =
                Self::resolved_lane(newer_cursor.as_deref(), newer_has_more.unwrap_or(false), newer_fallback);
            Ok(Some(MessagePage { rows, older, newer }))
        })
        .await;
        Self::cache_payload_result(loaded, failed_key)
    }

    /// cancellation stays cancellation, any other error names the key
    fn cache_payload_result<Page>(
        loaded: anyhow::Result<Option<Page>>,
        cache_key: String,
    ) -> anyhow::Result<Option<Page>> {
        match loaded {
            Err(error) if !error.is::<CancellationError>() => {
                Err(RiseLegacyConversationImporterError::InvalidCachePayload(cache_key).into())
            }
            other => other,
        }
    }

    fn decode_disk_payload<Value: DeserializeOwned>(
        cache_key: &str,
        cancelled: &AtomicBool,
    ) -> anyhow::Result<Option<Value>> {
        check_cancellation(cancelled)?;
        let Some(payload) = SaiWsDiskCache::load(cache_key) else { return Ok(None) };
        Ok(Some(serde_json::from_value(payload)?))
    }

    async fn prepare_chats(pages: Arc<Vec<ChatPage>>) -> anyhow::Result<PreparedChatImport> {
        detached(move |cancelled| {
            let mut groups: Vec<Option<ChatCandidate>> = Vec::new();
            let mut group_by_token: HashMap<String, usize> = HashMap::new();
            let mut skipped = 0;

            for page in pages.iter() {
                for (index, wire) in page.rows.iter().enumerate() {
                    if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                        check_cancellation(cancelled)?;
                    }
                    let Some(candidate) = Self::chat_candidate(wire, page.kind) else {
                        skipped += 1;
                        continue;
                    };
                    Self::merge_chat_candidate(candidate, &mut groups, &mut group_by_token);
                }
            }

            let mut rows = Vec::with_capacity(groups.len());
            for (index, candidate) in groups.into_iter().flatten().enumerate() {
                if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                    check_cancellation(cancelled)?;
                }
                let position = RiseConversationStoreLayout::chat_position(&candidate.wire);
                let mut record = RiseChatEntityRecord::new(candidate.wire, RiseConversationEntitySource::Migration);
                record.list_kind = Some(candidate.kind);
                let data = encode_sorted(&record)?;
                let version = Self::migration_version(&data, &position);
                rows.push(PreparedChatRow {
                    local_id: candidate.local_id,
                    data,
                    position,
                    aliases: Self::sorted_aliases(&candidate.aliases),
                    kind: candidate.kind,
                    version,
                });
            }
            Ok(PreparedChatImport { rows, skipped_row_count: skipped })
        })
        .await
    }

    async fn prepare_messages(
        messages: Vec<MessageWireDTO>,
        scope: RiseMessageScope,
    ) -> anyhow::Result<PreparedMessageImport> {
        detached(move |cancelled| {
            let mut groups: Vec<Option<MessageCandidate>> = Vec::new();
            let mut group_by_token: HashMap<String, usize> = HashMap::new();
            let mut skipped = 0;

            for (index, wire) in messages.into_iter().enumerate() {
                if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                    check_cancellation(cancelled)?;
                }
                let Some(candidate) = Self::message_candidate(wire, &scope)? else {
                    skipped += 1;
                    continue;
                };
                Self::merge_message_candidate(candidate, &mut groups, &mut group_by_token);
            }

            let mut rows = Vec::with_capacity(groups.len());
            for (index, candidate) in groups.into_iter().flatten().enumerate() {
                if index.is_multiple_of(CANCELLATION_CHECK_INTERVAL) {
                    check_cancellation(cancelled)?;
                }
                let server_id_kind = RiseEntityAliasKind::server_id();
                let has_server_alias = candidate.aliases.iter().any(|alias| alias.kind == server_id_kind);
                let is_unresolved_temporary =
                    candidate.wire.is_temp == Some(true) || (candidate.local_id_is_temporary && !has_server_alias);
                let local_phase = if is_unresolved_temporary {
                    RiseMessageLocalPhase::Queued
                } else {
                    RiseMessageLocalPhase::Remote
                };
                let position = RiseConversationStoreLayout::message_position(
                    &candidate.wire,
                    &candidate.local_id,
                    local_phase,
                );
                let mut record = RiseMessageEntityRecord::new(candidate.wire, RiseConversationEntitySource::Migration);
                record.local_phase = local_phase;
                let data = encode_sorted(&record)?;
                let version = Self::migration_version(&data, &position);
                rows.push(PreparedMessageRow {
                    local_id: candidate.local_id,
                    data,
                    position,
                    aliases: Self::sorted_aliases(&candidate.aliases),
                    version,
                });
            }
            Ok(PreparedMessageImport { rows, skipped_row_count: skipped })
        })
        .await
    }

    async fn write_chats(&self, rows: &[PreparedChatRow], engine: &RiseEngine) -> anyhow::Result<()> {
        let namespace = RiseConversationStoreLayout::chats();
        let mut mutations = Vec::new();
        let mut row_count = 0;
        let mut value_bytes = 0;

        for row in rows {
            self.validate_entity_size(row.data.len(), &row.local_id)?;
            let row_mutation_count = row.aliases.len() + 4;
            self.validate_mutation_count(row_mutation_count, &row.local_id)?;
            if self.batch_is_full(row_count, mutations.len() + row_mutation_count, value_bytes + row.data.len()) {
                Self::commit(std::mem::take(&mut mutations), engine).await?;
                row_count = 0;
                value_bytes = 0;
            }

            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: namespace.clone(),
                entity: RiseStoreEntity::new(row.local_id.clone(), row.data.clone(), row.version, row.version),
                policy: RiseEntityWritePolicy::IgnoreIfNotNewer,
            });
            for alias in &row.aliases {
                mutations.push(RiseStoreMutation::SetEntityAlias {
                    namespace: namespace.clone(),
                    alias: alias.clone(),
                    local_id: row.local_id.clone(),
                });
            }
            let store_view = row.kind.store_view();
            for view in RiseConversationStoreLayoutChatView::ALL_CASES {
                if view == store_view {
                    continue;
                }
                mutations.push(RiseStoreMutation::RemoveEntityViewMembership {
                    namespace: namespace.clone(),
                    view: view.raw_value().to_owned(),
                    local_id: row.local_id.clone(),
                    if_version_at_most: Some(row.version),
                });
            }
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: store_view.raw_value().to_owned(),
                membership: RiseEntityViewMembership::new(row.local_id.clone(), row.position.clone(), row.version),
            });
            row_count += 1;
            value_bytes += row.data.len();
        }
        Self::commit(mutations, engine).await
    }

    async fn write_messages(
        &self,
        rows: &[PreparedMessageRow],
        scope: &RiseMessageScope,
        engine: &RiseEngine,
    ) -> anyhow::Result<()> {
        let namespace = RiseConversationStoreLayout::messages();
        let view = RiseConversationStoreLayout::message_view(scope);
        let mut mutations = Vec::new();
        let mut row_count = 0;
        let mut value_bytes = 0;

        for row in rows {
            self.validate_entity_size(row.data.len(), &row.local_id)?;
            let row_mutation_count = row.aliases.len() + 2;
            self.validate_mutation_count(row_mutation_count, &row.local_id)?;
            if self.batch_is_full(row_count, mutations.len() + row_mutation_count, value_bytes + row.data.len()) {
                Self::commit(std::mem::take(&mut mutations), engine).await?;
                row_count = 0;
                value_bytes = 0;
            }

            mutations.push(RiseStoreMutation::UpsertEntity {
                namespace: namespace.clone(),
                entity: RiseStoreEntity::new(row.local_id.clone(), row.data.clone(), row.version, row.version),
                policy: RiseEntityWritePolicy::IgnoreIfNotNewer,
            });
            for alias in &row.aliases {
                mutations.push(RiseStoreMutation::SetEntityAlias {
                    namespace: namespace.clone(),
                    alias: alias.clone(),
                    local_id: row.local_id.clone(),
                });
            }
            mutations.push(RiseStoreMutation::UpsertEntityViewMembership {
                namespace: namespace.clone(),
                view: view.clone(),
                membership: RiseEntityViewMembership::new(row.local_id.clone(), row.position.clone(), row.version),
            });
            row_count += 1;
            value_bytes += row.data.len();
        }
        Self::commit(mutations, engine).await
    }

    async fn write_archive_aggregates(&self, pages: &[ChatPage], engine: &RiseEngine) -> anyhow::Result<()> {
        let Some(aggregates) = pages
            .iter()
            .find(|page| page.kind == RiseChatListKind::Archive)
            .and_then(|page| page.archive_aggregates)
        else {
            return Ok(());
        };
        let data = RiseConversationCodec::encode(aggregates).await?;
        self.validate_entity_size(data.len(), RiseChatRepository::ARCHIVE_AGGREGATES_KEY)?;
        let version = Self::migration_value_version(&data);
        engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: RiseConversationStoreLayout::chats(),
                    key: RiseChatRepository::ARCHIVE_AGGREGATES_KEY.to_owned(),
                    value: RiseStoredValue::new(data, version, version),
                    precondition: RiseStoreVersionPrecondition::Any,
                    policy: RiseStoredValueWritePolicy::IgnoreIfNotNewer,
                }],
                HashSet::new(),
            ))
            .await?;
        Ok(())
    }

    async fn write_missing_states(&self, seeds: Vec<ViewStateSeed>, engine: &RiseEngine) -> anyhow::Result<()> {
        let mut mutations = Vec::new();
        for seed in seeds {
            if engine.entity_view_state(seed.namespace.clone(), seed.view.clone()).await?.is_some() {
                continue;
            }
            mutations.push(RiseStoreMutation::SetEntityViewState {
                namespace: seed.namespace,
                view: seed.view,
                state: RiseEntityViewState::new(
                    1,
                    RiseConversationCodec::encode_cursor(seed.before_cursor.as_deref()),
                    RiseConversationCodec::encode_cursor(seed.after_cursor.as_deref()),
                    seed.has_more_before,
                    seed.has_more_after,
                    RiseEntityViewLoadState::Loaded,
                    None,
                    1,
                ),
            });
            if mutations.len() >= self.configuration.maximum_rows_per_batch
                || mutations.len() >= self.configuration.maximum_mutations_per_batch
            {
                Self::commit(std::mem::take(&mut mutations), engine).await?;
            }
        }
        Self::commit(mutations, engine).await
    }

    fn batch_is_full(&self, row_count: usize, mutation_count: usize, value_bytes: usize) -> bool {
        row_count >= self.configuration.maximum_rows_per_batch
            || mutation_count > self.configuration.maximum_mutations_per_batch
            || value_bytes > self.configuration.maximum_batch_value_bytes
    }

    fn validate_entity_size(&self, count: usize, local_id: &str) -> anyhow::Result<()> {
        if count > self.configuration.maximum_entity_bytes {
            return Err(RiseLegacyConversationImporterError::EntityTooLarge {
                local_id: local_id.to_owned(),
                byte_count: count,
                limit: self.configuration.maximum_entity_bytes,
            }
            .into());
        }
        Ok(())
    }

    fn validate_mutation_count(&self, count: usize, local_id: &str) -> anyhow::Result<()> {
        if count > self.configuration.maximum_mutations_per_batch {
            return Err(RiseLegacyConversationImporterError::RowMutationLimitExceeded {
                local_id: local_id.to_owned(),
                mutation_count: count,
                limit: self.configuration.maximum_mutations_per_batch,
            }
            .into());
        }
        Ok(())
    }

    async fn commit(mutations: Vec<RiseStoreMutation>, engine: &RiseEngine) -> anyhow::Result<()> {
        if mutations.is_empty() {
            return Ok(());
        }
        engine.write(RiseStoreWriteBatch::new(mutations, HashSet::new())).await?;
        Ok(())
    }

    fn chat_candidate(wire: &RiseStoredChat, kind: RiseChatListKind) -> Option<ChatCandidate> {
        let id = Self::normalized_identifier(Some(&wire.id))?;
        let server_id = Self::normalized_identifier(wire.server_id.as_deref());
        let local_id_is_temporary = GetSaiServerId::is_temp_id(Some(&id));
        let mut aliases = HashSet::new();
        aliases.insert(RiseEntityAlias::new(
            if local_id_is_temporary { RiseEntityAliasKind::temporary_id() } else { RiseEntityAliasKind::server_id() },
            id.clone(),
        ));
        if let Some(server_id) = server_id {
            let alias_kind = if GetSaiServerId::is_temp_id(Some(&server_id)) {
                RiseEntityAliasKind::temporary_id()
            } else {
                RiseEntityAliasKind::server_id()
            };
            aliases.insert(RiseEntityAlias::new(alias_kind, server_id));
        }
        let mut tokens: HashSet<String> = aliases.iter().map(Self::alias_token).collect();
        tokens.insert(format!("local\u{0}{id}"));
        Some(ChatCandidate { wire: wire.clone(), local_id: id, local_id_is_temporary, aliases, tokens, kind })
    }

    fn message_candidate(
        mut wire: MessageWireDTO,
        scope: &RiseMessageScope,
    ) -> anyhow::Result<Option<MessageCandidate>> {
        if let Some(raw_chat_id) = wire.chat_id.as_deref()
            && trimming_whitespaces_and_newlines(raw_chat_id) != scope.chat_id
        {
            return Ok(None);
        }
        wire.chat_id = Some(scope.chat_id.clone());

        if let Some(topic_id) = scope.topic_id.as_ref() {
            if let Some(raw_topic_id) = wire.topic_id.as_deref()
                && trimming_whitespaces_and_newlines(raw_topic_id) != topic_id
            {
                return Ok(None);
            }
            wire.topic_id = Some(topic_id.clone());
        }

        for field in [&mut wire.id, &mut wire.server_id, &mut wire.temp_id] {
            if Self::normalized_identifier(field.as_deref()).is_none() {
                *field = None;
            }
        }

        let id = Self::normalized_identifier(wire.id.as_deref());
        let server_id = Self::server_identifier(&wire);
        let temp_id = Self::normalized_identifier(wire.temp_id.as_deref());
        let temporary_id =
            temp_id.clone().or_else(|| id.clone().filter(|id| GetSaiServerId::is_temp_id(Some(id))));
        let fallback_data = encode_sorted(&wire)?;
        let fallback_id = format!("legacy_{}", Self::hex(Self::fnv1a64(&fallback_data)));
        let local_id_is_temporary = temporary_id.is_some();
        let local_id = temporary_id.or_else(|| server_id.clone()).unwrap_or(fallback_id);
        let mut aliases = HashSet::new();
        if let Some(server_id) = server_id {
            aliases.insert(RiseEntityAlias::new(RiseEntityAliasKind::server_id(), server_id));
        }
        if let Some(id) = id {
            let alias_kind = if GetSaiServerId::is_temp_id(Some(&id)) {
                RiseEntityAliasKind::temporary_id()
            } else {
                RiseEntityAliasKind::server_id()
            };
            aliases.insert(RiseEntityAlias::new(alias_kind, id));
        }
        if let Some(temp_id) = temp_id {
            aliases.insert(RiseEntityAlias::new(RiseEntityAliasKind::temporary_id(), temp_id));
        }
        let mut tokens: HashSet<String> = aliases.iter().map(Self::alias_token).collect();
        tokens.insert(format!("local\u{0}{local_id}"));
        Ok(Some(MessageCandidate { wire, local_id, local_id_is_temporary, aliases, tokens }))
    }

    fn merge_chat_candidate(
        incoming: ChatCandidate,
        groups: &mut Vec<Option<ChatCandidate>>,
        group_by_token: &mut HashMap<String, usize>,
    ) {
        let matches: BTreeSet<usize> =
            incoming.tokens.iter().filter_map(|token| group_by_token.get(token).copied()).collect();
        let mut matches = matches.into_iter();
        let Some(target) = matches.next() else {
            let index = groups.len();
            for token in &incoming.tokens {
                group_by_token.insert(token.clone(), index);
            }
            groups.push(Some(incoming));
            return;
        };

        let mut merged = groups[target].take().unwrap_or_else(|| incoming.clone());
        for index in matches {
            let Some(candidate) = groups[index].take() else { continue };
            merged.aliases.extend(candidate.aliases);
            merged.tokens.extend(candidate.tokens);
            if candidate.local_id_is_temporary && !merged.local_id_is_temporary {
                merged.local_id = candidate.local_id;
                merged.local_id_is_temporary = true;
            }
        }
        merged.wire = incoming.wire;
        merged.kind = incoming.kind;
        merged.aliases.extend(incoming.aliases);
        merged.tokens.extend(incoming.tokens);
        if incoming.local_id_is_temporary || !merged.local_id_is_temporary {
            merged.local_id = incoming.local_id;
            merged.local_id_is_temporary = incoming.local_id_is_temporary;
        }
        for token in &merged.tokens {
            group_by_token.insert(token.clone(), target);
        }
        groups[target] = Some(merged);
    }

    fn merge_message_candidate(
        incoming: MessageCandidate,
        groups: &mut Vec<Option<MessageCandidate>>,
        group_by_token: &mut HashMap<String, usize>,
    ) {
        let matches: BTreeSet<usize> =
            incoming.tokens.iter().filter_map(|token| group_by_token.get(token).copied()).collect();
        let mut matches = matches.into_iter();
        let Some(target) = matches.next() else {
            let index = groups.len();
            for token in &incoming.tokens {
                group_by_token.insert(token.clone(), index);
            }
            groups.push(Some(incoming));
            return;
        };

        let mut merged = groups[target].take().unwrap_or_else(|| incoming.clone());
        for index in matches {
            let Some(candidate) = groups[index].take() else { continue };
            merged.aliases.extend(candidate.aliases);
            merged.tokens.extend(candidate.tokens);
            if candidate.local_id_is_temporary && !merged.local_id_is_temporary {
                merged.local_id = candidate.local_id;
                merged.local_id_is_temporary = true;
            }
        }
        merged.wire = incoming.wire;
        merged.aliases.extend(incoming.aliases);
        merged.tokens.extend(incoming.tokens);
        if incoming.local_id_is_temporary || !merged.local_id_is_temporary {
            merged.local_id = incoming.local_id;
            merged.local_id_is_temporary = incoming.local_id_is_temporary;
        }
        for token in &merged.tokens {
            group_by_token.insert(token.clone(), target);
        }
        groups[target] = Some(merged);
    }

    fn chat_state_seed(page: &ChatPage) -> ViewStateSeed {
        ViewStateSeed {
            namespace: RiseConversationStoreLayout::chats(),
            view: page.kind.store_view().raw_value().to_owned(),
            before_cursor: None,
            after_cursor: page.cursor.clone(),
            has_more_before: false,
            has_more_after: page.has_more,
        }
    }

    fn message_state_seed(
        scope: &RiseMessageScope,
        canonical: Option<&MessagePage>,
        latest: Option<&MessagePage>,
    ) -> ViewStateSeed {
        let empty_lane = || HistoryLane { cursor: None, has_more: false };
        let older = match canonical {
            Some(canonical) if !canonical.rows.is_empty() || latest.is_none() => canonical.older.clone(),
            _ => latest.map_or_else(empty_lane, |latest| latest.older.clone()),
        };
        let newer = latest
            .or(canonical)
            .map_or_else(empty_lane, |page| page.newer.clone());
        ViewStateSeed {
            namespace: RiseConversationStoreLayout::messages(),
            view: RiseConversationStoreLayout::message_view(scope),
            before_cursor: older.cursor,
            after_cursor: newer.cursor,
            has_more_before: older.has_more,
            has_more_after: newer.has_more,
        }
    }

    fn resolved_lane(cursor: Option<&str>, has_more: bool, fallback: Option<String>) -> HistoryLane {
        let resolved_cursor = Self::normalized_cursor(cursor).or(fallback);
        let has_more = has_more && resolved_cursor.is_some();
        HistoryLane { cursor: resolved_cursor, has_more }
    }

    fn boundary_message_id(rows: &[MessageWireDTO], from_start: bool) -> Option<String> {
        if from_start {
            rows.iter().find_map(Self::server_identifier)
        } else {
            rows.iter().rev().find_map(Self::server_identifier)
        }
    }

    /// first of server id and id that is a real server id
    fn server_identifier(wire: &MessageWireDTO) -> Option<String> {
        [wire.server_id.as_deref(), wire.id.as_deref()]
            .into_iter()
            .filter_map(Self::normalized_identifier)
            .find(|id| !GetSaiServerId::is_temp_id(Some(id)))
    }

    fn valid_scope(storage_key: &str) -> Option<RiseMessageScope> {
        if storage_key.is_empty() || storage_key.len() > MAXIMUM_SCOPE_STORAGE_KEY_BYTES {
            return None;
        }
        let scope = RiseMessageScope::parse(storage_key)?;
        Self::normalized_identifier(Some(&scope.chat_id))?;
        Some(scope)
    }

    fn normalized_identifier(raw: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw?);
        if value.is_empty() || value.len() > MAXIMUM_IDENTIFIER_BYTES {
            return None;
        }
        Some(value.to_owned())
    }

    fn normalized_cursor(raw: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw?);
        if value.is_empty() || value.len() > MAXIMUM_IDENTIFIER_BYTES {
            return None;
        }
        Some(value.to_owned())
    }

    fn alias_token(alias: &RiseEntityAlias) -> String {
        format!("{}\u{0}{}", alias.kind.raw_value(), alias.value)
    }

    fn sorted_aliases(aliases: &HashSet<RiseEntityAlias>) -> Vec<RiseEntityAlias> {
        let mut sorted: Vec<RiseEntityAlias> = aliases.iter().cloned().collect();
        sorted.sort_by(|first, second| {
            first
                .kind
                .raw_value()
                .cmp(second.kind.raw_value())
                .then_with(|| first.value.cmp(&second.value))
        });
        sorted
    }

    fn migration_version(data: &[u8], position: &RisePositionKey) -> i64 {
        let mut hash = Self::fnv1a64(data);
        for byte in position.raw_value() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        (hash % VERSION_MODULUS) as i64 + 1
    }

    fn migration_value_version(data: &[u8]) -> i64 {
        (Self::fnv1a64(data) % VERSION_MODULUS) as i64 + 1
    }

    fn fnv1a64(data: &[u8]) -> u64 {
        let mut hash = FNV_OFFSET_BASIS;
        for byte in data {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash
    }

    fn hex(value: u64) -> String {
        format!("{value:x}")
    }

    fn now_milliseconds() -> i64 {
        (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64
    }
}
