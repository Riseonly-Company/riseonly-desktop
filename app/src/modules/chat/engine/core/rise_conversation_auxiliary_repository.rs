use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, LazyLock};

use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AsyncApp};
use rise_canon::{ChatMessageListTab, RiseLimits, rise_key, rise_request, rise_response, rise_view};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::RiseStoreWriteBatch;
use rise_engine::{CancellationError, RiseEngine, RiseJSONValue, RiseWireError, RiseWireRemoteError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::task::AbortHandle;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_coordinator::SaiWsScreenRefreshCoordinator;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::components::chat_search::chat_search_overlay_store::ChatSearchMessageKind;
use crate::modules::chat::engine::chat_engine_hooks::chat_engine_hooks;
use crate::modules::chat::engine::core::rise_chat_overlay_search_presentation::{
    RiseChatSearchRecentItem, RiseChatSearchRecentsPayload,
};
use crate::modules::chat::engine::core::rise_conversation_debug_trace::RiseConversationDebugTrace;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::core::rise_conversation_resource_keys::RiseConversationResourceKeys;
use crate::modules::chat::engine::core::rise_conversation_resource_repository::{
    RiseConversationResourceKey, RiseConversationResourceKeyPersistence, RiseConversationResourceReadSource,
    RiseConversationResourceRepository,
};
use crate::modules::chat::engine::core::rise_conversation_rpc::{
    RiseConversationEndpoint, RiseConversationEndpointKind, RiseConversationEndpoints, RiseConversationRPC,
};
use crate::modules::chat::engine::messages::rise_chat_calendar::RiseChatCalendarWindow;
use crate::modules::chat::engine::messages::rise_message_contracts::{
    RiseMessageAuxiliaryInvalidation, RiseMessageAuxiliaryInvalidationPinnedMessagePatch, RiseMessageAuxiliaryProjection,
    RiseMessageReadReceiptHint,
};
use crate::modules::chat::engine::messages::rise_message_read_receipt_wire_types::{
    RiseMessageReadReceiptStoredResource, RiseStoredReadReceipts,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChat;
use crate::modules::chat::stores::message::message_actions::message_stored_canon::RiseCanonMessageList;
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    GetChatCalendarGatewayDTO, LinkMessageWireDTO, MediaItemWireDTO, MessageLinkMetadataWireDTO, MessageWireDTO,
};
use crate::modules::post::stores::post_actions::post_wire_types::GetFeedPostItemDTO;
use crate::modules::search::stores::search_actions::search_wire_types::{GlobalSearchGatewayDTO, SearchResultItemDTO};

const SERVER_ID_SNAKE_KEY: &str = "server_id";
const SERVER_ID_CAMEL_KEY: &str = "serverId";
const SEARCH_BACKEND_UNAVAILABLE_MARKER: &str = "elasticsearch_unavailable";

const DEFAULT_MAXIMUM_MEMORY_ENTRIES: usize = 128;
const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 16 * 1_024 * 1_024;

static LINK_PREVIEW_CACHE_LIFETIME: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("cache_ttl_secs"));
static READ_RECEIPT_HINT_LIMIT: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("rise_conversation_auxiliary_repository_read_receipt_hint_limit"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseConversationAuxiliaryLoadState {
    Idle,
    Loading,
    Loaded,
    Failed,
}

impl RiseConversationAuxiliaryLoadState {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RiseConversationAuxiliarySnapshot<Value> {
    pub value: Option<Value>,
    pub revision: i64,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
    pub is_persisted: bool,
}

#[derive(Clone, Debug)]
pub struct RiseConversationAuxiliaryPageSnapshot<Item> {
    pub items: Vec<Item>,
    pub cursor: Option<String>,
    pub has_more: bool,
    pub revision: i64,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
    pub is_persisted: bool,
}

pub type RiseConversationMessageListTabKind = ChatMessageListTab;

/// request bodies the tab kind picks between, each encodes as its own flat object
#[derive(Serialize)]
#[serde(untagged)]
enum RiseConversationMessageListTabRequest {
    Files(rise_request::message::GetFileMessages),
    Music(rise_request::message::GetMusicMessages),
    Voice(rise_request::message::GetAudioMessages),
    Gif(rise_request::message::GetGifMessages),
    Favorites(rise_request::message::GetFavMessages),
}

trait RiseConversationMessageListTabKindExt {
    fn endpoint(self) -> RiseConversationEndpoint;
    fn request(
        self,
        chat_id: &str,
        topic_id: Option<&str>,
        source_chat_id: Option<&str>,
        relative_id: Option<&str>,
        limit: i64,
    ) -> RiseConversationMessageListTabRequest;
}

impl RiseConversationMessageListTabKindExt for RiseConversationMessageListTabKind {
    fn endpoint(self) -> RiseConversationEndpoint {
        match self {
            Self::Files => RiseConversationEndpoints::get_file_messages(),
            Self::Music => RiseConversationEndpoints::get_music_messages(),
            Self::Voice => RiseConversationEndpoints::get_audio_messages(),
            Self::Gif => RiseConversationEndpoints::get_gif_messages(),
            Self::Favorites => RiseConversationEndpoints::get_favorite_messages(),
        }
    }

    fn request(
        self,
        chat_id: &str,
        topic_id: Option<&str>,
        source_chat_id: Option<&str>,
        relative_id: Option<&str>,
        limit: i64,
    ) -> RiseConversationMessageListTabRequest {
        let chat_id = chat_id.to_owned();
        let limit = Some(limit);
        let relative_id = relative_id.map(str::to_owned);
        let topic_id = topic_id.map(str::to_owned);
        let up = Some(true);
        match self {
            Self::Files => RiseConversationMessageListTabRequest::Files(rise_request::message::GetFileMessages {
                chat_id,
                limit,
                relative_id,
                topic_id,
                up,
            }),
            Self::Music => RiseConversationMessageListTabRequest::Music(rise_request::message::GetMusicMessages {
                chat_id,
                limit,
                relative_id,
                topic_id,
                up,
            }),
            Self::Voice => RiseConversationMessageListTabRequest::Voice(rise_request::message::GetAudioMessages {
                chat_id,
                limit,
                relative_id,
                topic_id,
                up,
            }),
            Self::Gif => RiseConversationMessageListTabRequest::Gif(rise_request::message::GetGifMessages {
                chat_id,
                limit,
                relative_id,
                topic_id,
                up,
            }),
            Self::Favorites => RiseConversationMessageListTabRequest::Favorites(rise_request::message::GetFavMessages {
                chat_id,
                limit,
                relative_id,
                source_chat_id: source_chat_id.map(str::to_owned),
                up,
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseConversationAuxiliaryRepositoryError {
    #[error("Conversation resource request is already running: {0}")]
    RequestAlreadyInFlight(String),
    #[error("Expected a non-replayable mutation endpoint: {0}")]
    EndpointIsReplayable(String),
    #[error("Unknown conversation endpoint: {service}.{method}")]
    UnknownEndpoint { service: String, method: String },
    #[error("A newer conversation compatibility read superseded this result")]
    StaleCompatibilityResult,
    #[error("Message search is temporarily unavailable")]
    SearchBackendUnavailable,
}

impl RiseConversationAuxiliaryRepositoryError {
    pub fn error_description(&self) -> String {
        self.to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseConversationCompatibilityRequestReceipt {
    resource: RiseConversationCompatibilityRequestCoordinatorResource,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseConversationResolutionSource {
    ResourceMemory,
    PersistentStore,
    Network,
    NetworkFallbackMemory,
    NetworkFallbackPersistentStore,
    NormalizedStore,
}

impl RiseConversationResolutionSource {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::ResourceMemory => "resource-memory",
            Self::PersistentStore => "persistent-store",
            Self::Network => "network",
            Self::NetworkFallbackMemory => "network-fallback-memory",
            Self::NetworkFallbackPersistentStore => "network-fallback-persistent-store",
            Self::NormalizedStore => "normalized-store",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseConversationCacheWriteSource {
    None,
    Memory,
    PersistentStore,
}

impl RiseConversationCacheWriteSource {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Memory => "memory",
            Self::PersistentStore => "persistent-store",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RiseConversationCompatibilityCallResult {
    pub value: RiseJSONValue,
    pub receipt: Option<RiseConversationCompatibilityRequestReceipt>,
    pub cache_lookup_source: Option<RiseConversationResourceReadSource>,
    pub resolution_source: RiseConversationResolutionSource,
    pub was_coalesced: bool,
    pub cache_write_source: RiseConversationCacheWriteSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseConversationCompatibilityRequestCoordinatorResource {
    pub account_id: String,
    pub logical_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseConversationCompatibilityRequestCoordinatorSignature {
    pub body_digest: String,
    pub prefer_cached_value: bool,
    pub persistence: RiseConversationResourceKeyPersistence,
    pub allow_persistent_creation: bool,
}

#[derive(Clone, Debug)]
pub struct RiseConversationCompatibilityRequestCoordinatorOutput {
    pub value: RiseJSONValue,
    pub receipt: RiseConversationCompatibilityRequestReceipt,
    pub cache_lookup_source: Option<RiseConversationResourceReadSource>,
    pub resolution_source: RiseConversationResolutionSource,
    pub joined_existing_flight: bool,
}

#[derive(Clone, Debug)]
pub struct RiseConversationCompatibilityRequestCoordinatorResolution {
    pub value: RiseJSONValue,
    pub cache_lookup_source: Option<RiseConversationResourceReadSource>,
    pub source: RiseConversationResolutionSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RiseConversationCompatibilityRequestCoordinatorRequestKey {
    resource: RiseConversationCompatibilityRequestCoordinatorResource,
    signature: RiseConversationCompatibilityRequestCoordinatorSignature,
}

type RiseConversationCompatibilityFlightResult =
    Result<RiseConversationCompatibilityRequestCoordinatorResolution, Arc<anyhow::Error>>;

struct RiseConversationCompatibilityRequestCoordinatorInFlight {
    identifier: Uuid,
    receipt: RiseConversationCompatibilityRequestReceipt,
    task: Shared<BoxFuture<'static, RiseConversationCompatibilityFlightResult>>,
    abort: AbortHandle,
    waiter_count: usize,
}

#[derive(Default)]
struct RiseConversationCompatibilityRequestCoordinatorState {
    generation_clock: u64,
    latest_generation: HashMap<RiseConversationCompatibilityRequestCoordinatorResource, u64>,
    in_flight: HashMap<
        RiseConversationCompatibilityRequestCoordinatorRequestKey,
        RiseConversationCompatibilityRequestCoordinatorInFlight,
    >,
}

/// account scoped single flight and latest wins gate for compat reads
/// signature covers source so a cache first read never satisfies a reload
/// identity is broader, newer body for the same resource blocks older
pub struct RiseConversationCompatibilityRequestCoordinator {
    state: RiseIsolated<RiseConversationCompatibilityRequestCoordinatorState>,
}

impl Default for RiseConversationCompatibilityRequestCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

struct RiseConversationCompatibilityFlightRelease<'a> {
    coordinator: &'a RiseConversationCompatibilityRequestCoordinator,
    key: RiseConversationCompatibilityRequestCoordinatorRequestKey,
    identifier: Uuid,
}

impl Drop for RiseConversationCompatibilityFlightRelease<'_> {
    fn drop(&mut self) {
        self.coordinator.release_flight(&self.key, self.identifier);
    }
}

impl RiseConversationCompatibilityRequestCoordinator {
    pub fn new() -> Self {
        Self { state: RiseIsolated::new(RiseConversationCompatibilityRequestCoordinatorState::default()) }
    }

    pub async fn perform_read<Operation>(
        &self,
        resource: RiseConversationCompatibilityRequestCoordinatorResource,
        signature: RiseConversationCompatibilityRequestCoordinatorSignature,
        operation: impl FnOnce() -> Operation + Send + 'static,
    ) -> anyhow::Result<RiseConversationCompatibilityRequestCoordinatorOutput>
    where
        Operation: Future<Output = anyhow::Result<RiseJSONValue>> + Send + 'static,
    {
        self.perform_resolved_read(resource, signature, move || async move {
            Ok::<_, anyhow::Error>(RiseConversationCompatibilityRequestCoordinatorResolution {
                value: operation().await?,
                cache_lookup_source: None,
                source: RiseConversationResolutionSource::Network,
            })
        })
        .await
    }

    pub async fn perform_resolved_read<Operation>(
        &self,
        resource: RiseConversationCompatibilityRequestCoordinatorResource,
        signature: RiseConversationCompatibilityRequestCoordinatorSignature,
        operation: impl FnOnce() -> Operation + Send + 'static,
    ) -> anyhow::Result<RiseConversationCompatibilityRequestCoordinatorOutput>
    where
        Operation: Future<Output = anyhow::Result<RiseConversationCompatibilityRequestCoordinatorResolution>>
            + Send
            + 'static,
    {
        let key = RiseConversationCompatibilityRequestCoordinatorRequestKey { resource, signature };
        let (task, receipt, identifier, joined_existing_flight) = self.state.with(|state| {
            let latest = state.latest_generation.get(&key.resource).copied();
            if let Some(existing) = state.in_flight.get_mut(&key)
                && latest == Some(existing.receipt.generation)
            {
                existing.waiter_count += 1;
                return (existing.task.clone(), existing.receipt.clone(), existing.identifier, true);
            }
            if let Some(stale) = state.in_flight.remove(&key) {
                stale.abort.abort();
            }
            state.generation_clock = state.generation_clock.wrapping_add(1);
            let receipt = RiseConversationCompatibilityRequestReceipt {
                resource: key.resource.clone(),
                generation: state.generation_clock,
            };
            state.latest_generation.insert(key.resource.clone(), receipt.generation);
            let handle = RiseAccountRuntime::current().spawn(async move { operation().await });
            let abort = handle.abort_handle();
            let task: Shared<BoxFuture<'static, RiseConversationCompatibilityFlightResult>> = async move {
                match handle.await {
                    Ok(Ok(resolution)) => Ok(resolution),
                    Ok(Err(error)) => Err(Arc::new(error)),
                    Err(_) => Err(Arc::new(anyhow::Error::new(CancellationError))),
                }
            }
            .boxed()
            .shared();
            let identifier = Uuid::new_v4();
            state.in_flight.insert(
                key.clone(),
                RiseConversationCompatibilityRequestCoordinatorInFlight {
                    identifier,
                    receipt: receipt.clone(),
                    task: task.clone(),
                    abort,
                    waiter_count: 1,
                },
            );
            (task, receipt, identifier, false)
        });

        let release = RiseConversationCompatibilityFlightRelease { coordinator: self, key, identifier };
        let result = task.await;
        drop(release);
        if !self.is_current(&receipt).await {
            return Err(RiseConversationAuxiliaryRepositoryError::StaleCompatibilityResult.into());
        }
        let resolution = result.map_err(|error| Self::shared_failure(&error))?;
        Ok(RiseConversationCompatibilityRequestCoordinatorOutput {
            value: resolution.value,
            receipt,
            cache_lookup_source: resolution.cache_lookup_source,
            resolution_source: resolution.source,
            joined_existing_flight,
        })
    }

    fn shared_failure(error: &anyhow::Error) -> anyhow::Error {
        if let Some(failure) = error.downcast_ref::<CancellationError>() {
            return anyhow::Error::new(*failure);
        }
        if let Some(failure) = error.downcast_ref::<RiseWireRemoteError>() {
            return anyhow::Error::new(failure.clone());
        }
        if let Some(failure) = error.downcast_ref::<RiseWireError>() {
            return anyhow::Error::new(failure.clone());
        }
        if let Some(failure) = error.downcast_ref::<RiseFailure>() {
            return anyhow::Error::new(failure.clone());
        }
        if let Some(failure) = error.downcast_ref::<RiseConversationAuxiliaryRepositoryError>() {
            return anyhow::Error::new(failure.clone());
        }
        anyhow::anyhow!("{error:#}")
    }

    pub async fn is_current(&self, receipt: &RiseConversationCompatibilityRequestReceipt) -> bool {
        self.state.with(|state| state.latest_generation.get(&receipt.resource) == Some(&receipt.generation))
    }

    pub fn invalidate(&self) {
        let resources: HashSet<RiseConversationCompatibilityRequestCoordinatorResource> = self.state.with(|state| {
            state
                .latest_generation
                .keys()
                .cloned()
                .chain(state.in_flight.keys().map(|key| key.resource.clone()))
                .collect()
        });
        self.invalidate_resources(&resources);
    }

    pub fn invalidate_resources(&self, resources: &HashSet<RiseConversationCompatibilityRequestCoordinatorResource>) {
        self.state.with(|state| {
            let tracked: HashSet<&RiseConversationCompatibilityRequestCoordinatorResource> = resources
                .iter()
                .filter(|resource| {
                    state.latest_generation.contains_key(*resource)
                        || state.in_flight.keys().any(|key| &key.resource == *resource)
                })
                .collect();
            if tracked.is_empty() {
                return;
            }
            for resource in &tracked {
                state.generation_clock = state.generation_clock.wrapping_add(1);
                state.latest_generation.insert((*resource).clone(), state.generation_clock);
            }
            let request_keys: Vec<RiseConversationCompatibilityRequestCoordinatorRequestKey> =
                state.in_flight.keys().filter(|key| tracked.contains(&key.resource)).cloned().collect();
            for key in request_keys {
                if let Some(flight) = state.in_flight.remove(&key) {
                    flight.abort.abort();
                }
            }
        });
    }

    pub fn clear(&self) {
        self.invalidate();
        self.state.with(|state| {
            state.latest_generation = HashMap::new();
        });
    }

    pub async fn in_flight_waiter_count(
        &self,
        resource: &RiseConversationCompatibilityRequestCoordinatorResource,
        signature: &RiseConversationCompatibilityRequestCoordinatorSignature,
    ) -> usize {
        let key = RiseConversationCompatibilityRequestCoordinatorRequestKey {
            resource: resource.clone(),
            signature: signature.clone(),
        };
        self.state.with(|state| state.in_flight.get(&key).map_or(0, |flight| flight.waiter_count))
    }

    fn release_flight(&self, key: &RiseConversationCompatibilityRequestCoordinatorRequestKey, identifier: Uuid) {
        self.state.with(|state| {
            let Some(flight) = state.in_flight.get_mut(key) else {
                return;
            };
            if flight.identifier != identifier {
                return;
            }
            flight.waiter_count -= 1;
            if flight.waiter_count == 0 {
                state.in_flight.remove(key);
            }
        });
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseConversationAuxiliaryStoredPage<Item> {
    items: Vec<Item>,
    cursor: Option<String>,
    has_more: bool,
}

#[derive(Clone, Default)]
struct RiseConversationAuxiliaryRequestStatus {
    generation: u64,
    is_loading: bool,
    did_resolve: bool,
    error: Option<String>,
}

#[derive(Default)]
struct RiseConversationAuxiliaryRepositoryState {
    request_status: HashMap<RiseConversationResourceKey, RiseConversationAuxiliaryRequestStatus>,
    compatibility_replacement_generations: HashMap<String, u64>,
}

struct RiseConversationAuxiliaryRepositoryCompatibilityCachedValue {
    value: Option<RiseJSONValue>,
    source: RiseConversationResourceReadSource,
}

struct RiseConversationAuxiliaryInvalidationTargets {
    resource_keys: HashSet<RiseConversationResourceKey>,
    compatibility_logical_keys: HashSet<String>,
    presentation_cache_identities: HashSet<String>,
}

/// guard for a dropped request
/// a dropped call never reaches its error branch and is_loading would stay set
struct RiseConversationAuxiliaryRequestGuard<'a> {
    repository: &'a RiseConversationAuxiliaryRepository,
    key: &'a RiseConversationResourceKey,
    generation: u64,
    is_armed: bool,
}

impl Drop for RiseConversationAuxiliaryRequestGuard<'_> {
    fn drop(&mut self) {
        if self.is_armed {
            self.repository.fail_request(self.key, self.generation, &anyhow::Error::new(CancellationError));
        }
    }
}

/// typed cold path repository for bounded conversation resources
/// owns rarer pages with encrypted disk cache and stale request guard
pub struct RiseConversationAuxiliaryRepository {
    account_id: String,
    account_epoch: String,
    resources: Arc<RiseConversationResourceRepository>,
    rpc: RiseConversationRPC,
    compatibility_requests: RiseConversationCompatibilityRequestCoordinator,
    state: RiseIsolated<RiseConversationAuxiliaryRepositoryState>,
}

impl RiseConversationAuxiliaryRepository {
    pub async fn link_preview(&self, url: &str) -> anyhow::Result<MessageLinkMetadataWireDTO> {
        let key = RiseConversationResourceKey::new(&["link-preview", url]);
        let now = Self::now_milliseconds();
        if !Self::is_invite_link(url)
            && let Ok(snapshot) = self.resources.cached::<rise_response::link_preview::GetLinkPreview>(&key).await
            && let Some(cached) = snapshot.value.as_ref()
            && now >= snapshot.revision
            && now - snapshot.revision < *LINK_PREVIEW_CACHE_LIFETIME
        {
            return Ok(MessageLinkMetadataWireDTO::from_canon(cached));
        }
        let fresh: rise_response::link_preview::GetLinkPreview = self
            .resources
            .refresh(
                &key,
                RiseConversationEndpoints::get_link_preview(),
                rise_request::link_preview::GetLinkPreview { url: Some(url.to_owned()) },
            )
            .await?;
        Ok(MessageLinkMetadataWireDTO::from_canon(&fresh))
    }

    fn is_invite_link(url: &str) -> bool {
        let raw = if url.contains("://") { url.to_owned() } else { format!("https://{url}") };
        let Some(link) = Url::from_string(&raw) else {
            return false;
        };
        matches!(UniversalLinkParser::parse(&link, AppEnvironment::current()), Some(PushDeepLink::JoinChat { .. }))
    }

    /// new repo with both memory bounds at their defaults
    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Arc<Self> {
        Self::with_memory_limits(engine, account_id, DEFAULT_MAXIMUM_MEMORY_ENTRIES, DEFAULT_MAXIMUM_MEMORY_BYTES)
    }

    pub fn with_memory_limits(
        engine: Arc<RiseEngine>,
        account_id: String,
        maximum_memory_entries: usize,
        maximum_memory_bytes: usize,
    ) -> Arc<Self> {
        assert!(!account_id.is_empty());
        assert!(engine.account.account_id == account_id);
        let account_epoch = engine.account.epoch.clone();
        let resources = Arc::new(RiseConversationResourceRepository::with_memory_limits(
            engine.clone(),
            maximum_memory_entries,
            maximum_memory_bytes,
        ));
        let rpc = RiseConversationRPC::new(engine);
        Arc::new(Self {
            account_id,
            account_epoch,
            resources,
            rpc,
            compatibility_requests: RiseConversationCompatibilityRequestCoordinator::new(),
            state: RiseIsolated::new(RiseConversationAuxiliaryRepositoryState::default()),
        })
    }

    pub async fn invalidate_pending_requests(&self) {
        self.state.with(|state| {
            for status in state.request_status.values_mut() {
                status.generation = status.generation.wrapping_add(1);
                status.is_loading = false;
            }
        });
        self.compatibility_requests.invalidate();
        self.resources.invalidate_requests().await;
    }

    pub async fn clear_memory(&self) {
        self.state.with(|state| {
            state.request_status = HashMap::new();
            state.compatibility_replacement_generations = HashMap::new();
        });
        self.compatibility_requests.clear();
        self.resources.clear_memory().await;
    }

    pub async fn remove_cached_resource(&self, key: &RiseConversationResourceKey) -> anyhow::Result<()> {
        self.state.with(|state| {
            state.request_status.remove(key);
        });
        self.resources.remove(key).await
    }

    /// mirrors an already reduced realtime or optimistic projection into the store
    /// keeps the cache authoritative with no extra read after an event
    pub async fn replace_compatibility_cached_response(
        &self,
        service: &str,
        method: &str,
        cache_identity: &str,
        encoded_value: &[u8],
    ) -> anyhow::Result<()> {
        let endpoint = RiseConversationEndpoints::endpoint(service, method)
            .filter(|endpoint| matches!(endpoint.kind, RiseConversationEndpointKind::Read))
            .ok_or_else(|| RiseConversationAuxiliaryRepositoryError::UnknownEndpoint {
                service: service.to_owned(),
                method: method.to_owned(),
            })?;
        let key = Self::compatibility_resource_key(
            endpoint,
            &SaiAccountCacheScope::logical_key(cache_identity),
            RiseConversationResourceKeyPersistence::Persistent,
        );
        let resource = self.compatibility_resource(&key);
        let generation = self.state.with(|state| {
            let generation = state.compatibility_replacement_generations.entry(key.key.clone()).or_insert(0);
            *generation = generation.wrapping_add(1);
            *generation
        });
        self.compatibility_requests.invalidate_resources(&HashSet::from([resource]));
        let is_latest = self
            .state
            .with(|state| state.compatibility_replacement_generations.get(&key.key) == Some(&generation));
        if !is_latest {
            return Ok(());
        }
        self.store_compatibility_value(&RiseJSONValue::decode(encoded_value)?, &key, true).await?;
        Ok(())
    }

    pub async fn make_message_projection_invalidation_batch(
        &self,
        invalidation: &RiseMessageAuxiliaryInvalidation,
    ) -> RiseStoreWriteBatch {
        let mut invalidation_only_projections = invalidation.projections.clone();
        let mut pinned_patch_batch = RiseStoreWriteBatch::empty();
        let has_pinned = invalidation.projections.contains(&RiseMessageAuxiliaryProjection::Pinned);
        if has_pinned && let Some(pinned_message_patch) = invalidation.pinned_message_patch.as_ref() {
            pinned_patch_batch = self.make_pinned_message_patch_batch(pinned_message_patch, &invalidation.chat_id).await;
            invalidation_only_projections.remove(&RiseMessageAuxiliaryProjection::Pinned);
        }
        let targets = Self::message_projection_invalidation_targets_for(
            &invalidation.chat_id,
            &invalidation.topic_ids,
            &invalidation_only_projections,
        );
        self.state.with(|state| {
            for key in &targets.resource_keys {
                state.request_status.remove(key);
            }
        });
        let compatibility_resources: HashSet<RiseConversationCompatibilityRequestCoordinatorResource> = targets
            .compatibility_logical_keys
            .iter()
            .map(|logical_key| RiseConversationCompatibilityRequestCoordinatorResource {
                account_id: self.account_id.clone(),
                logical_key: logical_key.clone(),
            })
            .collect();
        self.compatibility_requests.invalidate_resources(&compatibility_resources);
        let removal_batch = self.resources.removal_batch(&targets.resource_keys).await;
        let prefix = SaiAccountCacheScope::prefix(&self.account_id);
        let presentation_cache_keys: HashSet<String> =
            targets.presentation_cache_identities.iter().map(|identity| format!("{prefix}{identity}")).collect();

        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let chat_id = invalidation.chat_id.clone();
        let removed_message_ids = invalidation.removed_message_ids.clone();
        Self::run_on_main(move |cx| {
            if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch)) {
                return;
            }
            if !removed_message_ids.is_empty()
                && let Some(hooks) = chat_engine_hooks()
            {
                (hooks.record_confirmed_message_deletions)(
                    &chat_id,
                    removed_message_ids.iter().cloned().collect(),
                    cx,
                );
            }
            for cache_key in &presentation_cache_keys {
                SaiWsResponseCache::remove(cache_key, SaiCacheCleanLayer::Memory, cx);
            }
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.remove_message_tab_entries(&chat_id, &removed_message_ids, cx);
                if has_pinned {
                    presentation.remove_pinned_entries(&chat_id, &removed_message_ids, cx);
                }
            });
            SaiWsScreenRefreshCoordinator::schedule_targeted_revalidation(
                &presentation_cache_keys,
                &account_id,
                &account_epoch,
                cx,
            );
        })
        .await;
        Self::combining(pinned_patch_batch, removal_batch)
    }

    async fn make_pinned_message_patch_batch(
        &self,
        patch: &RiseMessageAuxiliaryInvalidationPinnedMessagePatch,
        chat_id: &str,
    ) -> RiseStoreWriteBatch {
        let typed_key = RiseConversationResourceKeys::pinned_messages(chat_id);
        self.state.with(|state| {
            state.request_status.remove(&typed_key);
        });

        let mut batch = RiseStoreWriteBatch::empty();
        let cached_page = self
            .resources
            .cached::<RiseConversationAuxiliaryStoredPage<MessageWireDTO>>(&typed_key)
            .await
            .ok()
            .and_then(|cached| cached.value);
        if let Some(page) = cached_page {
            let items = Self::applying_pinned_message_patch_to_items(patch, page.items);
            let replacement = RiseConversationAuxiliaryStoredPage {
                items,
                cursor: if patch.removes_all { None } else { page.cursor },
                has_more: if patch.removes_all { false } else { page.has_more },
            };
            if let Ok(next) = self.resources.replacement_batch(&replacement, &typed_key).await {
                batch = Self::combining(batch, next);
            }
        }

        let logical_cache_identity = format!("getPinnedMessages-{chat_id}");
        let compatibility_key = Self::compatibility_resource_key(
            &RiseConversationEndpoints::get_pinned_messages(),
            &logical_cache_identity,
            RiseConversationResourceKeyPersistence::Persistent,
        );
        let compatibility_resource = self.compatibility_resource(&compatibility_key);
        self.compatibility_requests.invalidate_resources(&HashSet::from([compatibility_resource]));

        let cached_value = self
            .resources
            .cached::<RiseJSONValue>(&compatibility_key)
            .await
            .ok()
            .and_then(|cached| cached.value);
        let Some(patched_value) = cached_value
            .and_then(|value| Self::applying_pinned_message_patch_to_value(patch, &value).ok().flatten())
        else {
            return batch;
        };
        if let Ok(next) = self.resources.replacement_batch(&patched_value, &compatibility_key).await {
            batch = Self::combining(batch, next);
        }

        let presentation_value = Value::from(patched_value);
        let presentation_cache_key =
            format!("{}{logical_cache_identity}", SaiAccountCacheScope::prefix(&self.account_id));
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        Self::run_on_main(move |cx| {
            if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch)) {
                return;
            }
            SaiWsResponseCache::hydrate_memory(&presentation_cache_key, presentation_value, false, cx);
        })
        .await;
        batch
    }

    fn applying_pinned_message_patch_to_items(
        patch: &RiseMessageAuxiliaryInvalidationPinnedMessagePatch,
        items: Vec<MessageWireDTO>,
    ) -> Vec<MessageWireDTO> {
        if patch.removes_all {
            return Vec::new();
        }
        let identities = &patch.message_identifiers;
        let position = items.iter().position(|item| !Self::message_identities(item).is_disjoint(identities));
        let mut result: Vec<MessageWireDTO> = items
            .into_iter()
            .filter(|item| Self::message_identities(item).is_disjoint(identities))
            .collect();
        if patch.is_pinned
            && let Some(message) = patch.message.as_ref()
        {
            let index = position.unwrap_or(result.len()).min(result.len());
            result.insert(index, message.clone());
        }
        result
    }

    fn applying_pinned_message_patch_to_value(
        patch: &RiseMessageAuxiliaryInvalidationPinnedMessagePatch,
        value: &RiseJSONValue,
    ) -> anyhow::Result<Option<RiseJSONValue>> {
        let Some(mut root) = value.object_value().cloned() else {
            return Ok(None);
        };
        let Some(RiseJSONValue::Array(rows)) = root.get(rise_key::message::MESSAGES).cloned() else {
            return Ok(None);
        };
        if patch.removes_all {
            root.insert(rise_key::message::MESSAGES.to_owned(), RiseJSONValue::Array(Vec::new()));
            root.insert(rise_key::chat::IS_HAVE_MORE.to_owned(), RiseJSONValue::Bool(false));
            root.remove(rise_key::message::RELATIVE_ID);
            return Ok(Some(RiseJSONValue::Object(root)));
        }
        let identities = &patch.message_identifiers;
        let position = rows.iter().position(|row| !Self::message_identities_of_value(row).is_disjoint(identities));
        let mut next_rows: Vec<RiseJSONValue> = rows
            .into_iter()
            .filter(|row| Self::message_identities_of_value(row).is_disjoint(identities))
            .collect();
        if patch.is_pinned
            && let Some(message) = patch.message.as_ref()
        {
            let index = position.unwrap_or(next_rows.len()).min(next_rows.len());
            next_rows.insert(index, RiseJSONValue::encode(message)?);
        }
        root.insert(rise_key::message::MESSAGES.to_owned(), RiseJSONValue::Array(next_rows));
        Ok(Some(RiseJSONValue::Object(root)))
    }

    fn message_identities(message: &MessageWireDTO) -> HashSet<String> {
        [message.server_id.as_deref(), message.id.as_deref(), message.temp_id.as_deref()]
            .into_iter()
            .filter_map(Self::normalized)
            .collect()
    }

    fn message_identities_of_value(value: &RiseJSONValue) -> HashSet<String> {
        let Some(object) = value.object_value() else {
            return HashSet::new();
        };
        [SERVER_ID_SNAKE_KEY, SERVER_ID_CAMEL_KEY, rise_key::message::ID, rise_key::message::TEMP_ID]
            .into_iter()
            .filter_map(|key| Self::normalized(object.get(key).and_then(RiseJSONValue::string_value)))
            .collect()
    }

    fn combining(first: RiseStoreWriteBatch, second: RiseStoreWriteBatch) -> RiseStoreWriteBatch {
        let mut mutations = first.mutations;
        mutations.extend(second.mutations);
        let mut invalidated_views = first.invalidated_views;
        invalidated_views.extend(second.invalidated_views);
        RiseStoreWriteBatch::new(mutations, invalidated_views)
    }

    async fn run_on_main(work: impl FnOnce(&mut App) + Send + 'static) {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(work);
                    Ok::<(), anyhow::Error>(())
                }
                .boxed_local()
            })
            .await;
    }

    fn now_milliseconds() -> i64 {
        (Date::now().time_interval_since_1970() * 1_000.0) as i64
    }

    fn compatibility_resource(
        &self,
        key: &RiseConversationResourceKey,
    ) -> RiseConversationCompatibilityRequestCoordinatorResource {
        RiseConversationCompatibilityRequestCoordinatorResource {
            account_id: self.account_id.clone(),
            logical_key: key.key.clone(),
        }
    }

    /// temporary entry point while old screens still use typed slices
    /// transport, encryption and disk stay in the engine
    pub async fn compatibility_call(
        &self,
        service: &str,
        method: &str,
        body: RiseJSONValue,
        cache_identity: Option<&str>,
        persistence: RiseConversationResourceKeyPersistence,
        prefer_cached_value: bool,
    ) -> anyhow::Result<RiseJSONValue> {
        self.compatibility_call_with_options(
            service,
            method,
            body,
            cache_identity,
            persistence,
            prefer_cached_value,
            true,
            true,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn compatibility_call_with_options(
        &self,
        service: &str,
        method: &str,
        body: RiseJSONValue,
        cache_identity: Option<&str>,
        persistence: RiseConversationResourceKeyPersistence,
        prefer_cached_value: bool,
        store_response_in_cache: bool,
        allow_persistent_creation: bool,
    ) -> anyhow::Result<RiseJSONValue> {
        Ok(self
            .compatibility_call_result(
                service,
                method,
                body,
                cache_identity,
                persistence,
                prefer_cached_value,
                store_response_in_cache,
                allow_persistent_creation,
            )
            .await?
            .value)
    }

    pub async fn has_compatibility_cached_response(
        &self,
        service: &str,
        method: &str,
        body: &RiseJSONValue,
        cache_identity: Option<&str>,
        persistence: RiseConversationResourceKeyPersistence,
        allow_persistent_creation: bool,
    ) -> anyhow::Result<bool> {
        Ok(self
            .compatibility_cached_response(
                service,
                method,
                body,
                cache_identity,
                persistence,
                allow_persistent_creation,
            )
            .await?
            .is_some())
    }

    /// stored response of a read without touching the network, none when nothing is stored
    pub async fn compatibility_cached_response(
        &self,
        service: &str,
        method: &str,
        body: &RiseJSONValue,
        cache_identity: Option<&str>,
        persistence: RiseConversationResourceKeyPersistence,
        allow_persistent_creation: bool,
    ) -> anyhow::Result<Option<RiseJSONValue>> {
        let Some(endpoint) = RiseConversationEndpoints::endpoint(service, method)
            .filter(|endpoint| matches!(endpoint.kind, RiseConversationEndpointKind::Read))
        else {
            return Ok(None);
        };
        let key = Self::compatibility_resource_key_for_body(endpoint, body, cache_identity, persistence)?;
        Ok(Self::cached_compatibility_value(&self.resources, &key, allow_persistent_creation).await?.value)
    }

    /// variant that returns a receipt for the compatibility bridge
    /// bridge revalidates it after every suspension before touching legacy state
    #[allow(clippy::too_many_arguments)]
    pub async fn compatibility_call_result(
        &self,
        service: &str,
        method: &str,
        body: RiseJSONValue,
        cache_identity: Option<&str>,
        persistence: RiseConversationResourceKeyPersistence,
        prefer_cached_value: bool,
        store_response_in_cache: bool,
        allow_persistent_creation: bool,
    ) -> anyhow::Result<RiseConversationCompatibilityCallResult> {
        let Some(endpoint) = RiseConversationEndpoints::endpoint(service, method) else {
            return Err(RiseConversationAuxiliaryRepositoryError::UnknownEndpoint {
                service: service.to_owned(),
                method: method.to_owned(),
            }
            .into());
        };

        if !Self::uses_compatibility_read_coordinator(endpoint) {
            return Ok(RiseConversationCompatibilityCallResult {
                value: self.rpc.call(endpoint, body).await?,
                receipt: None,
                cache_lookup_source: None,
                resolution_source: RiseConversationResolutionSource::Network,
                was_coalesced: false,
                cache_write_source: RiseConversationCacheWriteSource::None,
            });
        }

        let key = Self::compatibility_resource_key_for_body(endpoint, &body, cache_identity, persistence)?;
        let resource = self.compatibility_resource(&key);
        let signature = RiseConversationCompatibilityRequestCoordinatorSignature {
            body_digest: Self::canonical_body_digest(&body)?,
            prefer_cached_value,
            persistence,
            allow_persistent_creation,
        };
        let resources = self.resources.clone();
        let rpc = self.rpc.clone();
        let output = self
            .compatibility_requests
            .perform_resolved_read(resource, signature, {
                let key = key.clone();
                move || {
                    Self::resolve_compatibility_read(
                        resources,
                        rpc,
                        key,
                        endpoint,
                        body,
                        prefer_cached_value,
                        allow_persistent_creation,
                    )
                }
            })
            .await?;
        if !self.compatibility_requests.is_current(&output.receipt).await {
            return Err(RiseConversationAuxiliaryRepositoryError::StaleCompatibilityResult.into());
        }
        let mut cache_write_source = RiseConversationCacheWriteSource::None;
        if store_response_in_cache {
            cache_write_source = self.store_compatibility_value(&output.value, &key, allow_persistent_creation).await?;
            if !self.compatibility_requests.is_current(&output.receipt).await {
                return Err(RiseConversationAuxiliaryRepositoryError::StaleCompatibilityResult.into());
            }
        }
        Ok(RiseConversationCompatibilityCallResult {
            value: output.value,
            receipt: Some(output.receipt),
            cache_lookup_source: output.cache_lookup_source,
            resolution_source: output.resolution_source,
            was_coalesced: output.joined_existing_flight,
            cache_write_source,
        })
    }

    async fn resolve_compatibility_read(
        resources: Arc<RiseConversationResourceRepository>,
        rpc: RiseConversationRPC,
        key: RiseConversationResourceKey,
        endpoint: &'static RiseConversationEndpoint,
        body: RiseJSONValue,
        prefer_cached_value: bool,
        allow_persistent_creation: bool,
    ) -> anyhow::Result<RiseConversationCompatibilityRequestCoordinatorResolution> {
        let cached = Self::cached_compatibility_value(&resources, &key, allow_persistent_creation).await?;
        if prefer_cached_value && let Some(value) = cached.value {
            return Ok(RiseConversationCompatibilityRequestCoordinatorResolution {
                value,
                cache_lookup_source: Some(cached.source),
                source: Self::cache_resolution_source(cached.source),
            });
        }

        match rpc.call(endpoint, body).await {
            Ok(value) => Ok(RiseConversationCompatibilityRequestCoordinatorResolution {
                value,
                cache_lookup_source: Some(cached.source),
                source: RiseConversationResolutionSource::Network,
            }),
            Err(error) => match cached.value {
                Some(value) => Ok(RiseConversationCompatibilityRequestCoordinatorResolution {
                    value,
                    cache_lookup_source: Some(cached.source),
                    source: Self::cache_fallback_resolution_source(cached.source),
                }),
                None => Err(error),
            },
        }
    }

    pub fn uses_compatibility_read_coordinator(endpoint: &RiseConversationEndpoint) -> bool {
        matches!(endpoint.kind, RiseConversationEndpointKind::Read)
    }

    pub async fn is_compatibility_result_current(
        &self,
        receipt: &RiseConversationCompatibilityRequestReceipt,
    ) -> bool {
        self.compatibility_requests.is_current(receipt).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn store_compatibility_response(
        &self,
        value: &RiseJSONValue,
        receipt: &RiseConversationCompatibilityRequestReceipt,
        service: &str,
        method: &str,
        body: &RiseJSONValue,
        cache_identity: Option<&str>,
        persistence: RiseConversationResourceKeyPersistence,
        allow_persistent_creation: bool,
    ) -> anyhow::Result<RiseConversationCacheWriteSource> {
        let Some(endpoint) = RiseConversationEndpoints::endpoint(service, method)
            .filter(|endpoint| matches!(endpoint.kind, RiseConversationEndpointKind::Read))
        else {
            return Ok(RiseConversationCacheWriteSource::None);
        };
        let key = Self::compatibility_resource_key_for_body(endpoint, body, cache_identity, persistence)?;
        let expected_resource = self.compatibility_resource(&key);
        if receipt.resource != expected_resource || !self.compatibility_requests.is_current(receipt).await {
            return Err(RiseConversationAuxiliaryRepositoryError::StaleCompatibilityResult.into());
        }
        let cache_write_source = self.store_compatibility_value(value, &key, allow_persistent_creation).await?;
        if !self.compatibility_requests.is_current(receipt).await {
            return Err(RiseConversationAuxiliaryRepositoryError::StaleCompatibilityResult.into());
        }
        Ok(cache_write_source)
    }

    async fn cached_compatibility_value(
        resources: &RiseConversationResourceRepository,
        key: &RiseConversationResourceKey,
        allow_persistent_creation: bool,
    ) -> anyhow::Result<RiseConversationAuxiliaryRepositoryCompatibilityCachedValue> {
        if key.persistence != RiseConversationResourceKeyPersistence::Persistent || allow_persistent_creation {
            let snapshot = resources.cached::<RiseJSONValue>(key).await?;
            return Ok(RiseConversationAuxiliaryRepositoryCompatibilityCachedValue {
                value: snapshot.value,
                source: snapshot.read_source,
            });
        }

        let memory_key = Self::memory_companion_key(key);
        let memory = resources.cached::<RiseJSONValue>(&memory_key).await?;
        let persisted = resources.cached::<RiseJSONValue>(key).await?;
        if let Some(persisted_value) = persisted.value
            && persisted.revision >= memory.revision
        {
            return Ok(RiseConversationAuxiliaryRepositoryCompatibilityCachedValue {
                value: Some(persisted_value),
                source: persisted.read_source,
            });
        }
        if let Some(memory_value) = memory.value {
            return Ok(RiseConversationAuxiliaryRepositoryCompatibilityCachedValue {
                value: Some(memory_value),
                source: memory.read_source,
            });
        }
        Ok(RiseConversationAuxiliaryRepositoryCompatibilityCachedValue {
            value: None,
            source: RiseConversationResourceReadSource::Miss,
        })
    }

    fn cache_resolution_source(source: RiseConversationResourceReadSource) -> RiseConversationResolutionSource {
        match source {
            RiseConversationResourceReadSource::Memory => RiseConversationResolutionSource::ResourceMemory,
            RiseConversationResourceReadSource::PersistentStore => RiseConversationResolutionSource::PersistentStore,
            RiseConversationResourceReadSource::Miss => RiseConversationResolutionSource::NormalizedStore,
        }
    }

    fn cache_fallback_resolution_source(source: RiseConversationResourceReadSource) -> RiseConversationResolutionSource {
        match source {
            RiseConversationResourceReadSource::Memory => RiseConversationResolutionSource::NetworkFallbackMemory,
            RiseConversationResourceReadSource::PersistentStore => {
                RiseConversationResolutionSource::NetworkFallbackPersistentStore
            }
            RiseConversationResourceReadSource::Miss => RiseConversationResolutionSource::NormalizedStore,
        }
    }

    async fn store_compatibility_value(
        &self,
        value: &RiseJSONValue,
        key: &RiseConversationResourceKey,
        allow_persistent_creation: bool,
    ) -> anyhow::Result<RiseConversationCacheWriteSource> {
        if key.persistence != RiseConversationResourceKeyPersistence::Persistent {
            self.resources.store(value, key).await?;
            RiseConversationDebugTrace::note_cache_write(RiseConversationCacheWriteSource::Memory);
            return Ok(RiseConversationCacheWriteSource::Memory);
        }
        if allow_persistent_creation {
            self.resources.store(value, key).await?;
            RiseConversationDebugTrace::note_cache_write(RiseConversationCacheWriteSource::PersistentStore);
            let _ = self.resources.remove(&Self::memory_companion_key(key)).await;
            return Ok(RiseConversationCacheWriteSource::PersistentStore);
        }

        let existing = self.resources.cached::<RiseJSONValue>(key).await?;
        if existing.is_persisted {
            self.resources.store(value, key).await?;
            RiseConversationDebugTrace::note_cache_write(RiseConversationCacheWriteSource::PersistentStore);
            let _ = self.resources.remove(&Self::memory_companion_key(key)).await;
            Ok(RiseConversationCacheWriteSource::PersistentStore)
        } else {
            self.resources.store(value, &Self::memory_companion_key(key)).await?;
            RiseConversationDebugTrace::note_cache_write(RiseConversationCacheWriteSource::Memory);
            Ok(RiseConversationCacheWriteSource::Memory)
        }
    }

    fn memory_companion_key(persistent_key: &RiseConversationResourceKey) -> RiseConversationResourceKey {
        RiseConversationResourceKey::with_persistence(
            &["compatibility-memory", persistent_key.key.as_str()],
            RiseConversationResourceKeyPersistence::MemoryOnly,
        )
    }

    pub async fn call_non_replayable_mutation<Request, Response>(
        &self,
        endpoint: RiseConversationEndpoint,
        request: Request,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Send + Sync,
        Response: DeserializeOwned + Send,
    {
        Self::require_non_replayable_mutation(&endpoint)?;
        self.rpc.call(endpoint, request).await
    }

    async fn cached_value<Stored>(
        &self,
        key: &RiseConversationResourceKey,
    ) -> anyhow::Result<RiseConversationAuxiliarySnapshot<Stored>>
    where
        Stored: DeserializeOwned + Send + 'static,
    {
        let cached = self.resources.cached::<Stored>(key).await?;
        let status = self.status(key);
        let load_state = Self::load_state(&status, cached.value.is_some());
        Ok(RiseConversationAuxiliarySnapshot {
            value: cached.value,
            revision: cached.revision,
            load_state,
            error: status.error,
            is_persisted: cached.is_persisted,
        })
    }

    async fn refresh_value<Request, Stored>(
        &self,
        key: &RiseConversationResourceKey,
        endpoint: RiseConversationEndpoint,
        request: Request,
    ) -> anyhow::Result<RiseConversationAuxiliarySnapshot<Stored>>
    where
        Request: Serialize + Send + Sync,
        Stored: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    {
        let generation = self.begin_request(key)?;
        let mut guard =
            RiseConversationAuxiliaryRequestGuard { repository: self, key, generation, is_armed: true };
        let refreshed = self.resources.refresh::<Request, Stored>(key, endpoint, request).await;
        guard.is_armed = false;
        if let Err(error) = refreshed {
            self.fail_request(key, generation, &error);
            return Err(error);
        }
        self.finish_request(key, generation);
        let cached = self.cached_value::<Stored>(key).await;
        if let Err(error) = cached.as_ref() {
            self.fail_request(key, generation, error);
        }
        cached
    }

    async fn cached_page<Item>(
        &self,
        key: &RiseConversationResourceKey,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<Item>>
    where
        Item: DeserializeOwned + Send + 'static,
    {
        let cached = self.resources.cached::<RiseConversationAuxiliaryStoredPage<Item>>(key).await?;
        let status = self.status(key);
        let load_state = Self::load_state(&status, cached.value.is_some());
        let (items, cursor, has_more) = match cached.value {
            Some(page) => (page.items, page.cursor, page.has_more),
            None => (Vec::new(), None, true),
        };
        Ok(RiseConversationAuxiliaryPageSnapshot {
            items,
            cursor,
            has_more,
            revision: cached.revision,
            load_state,
            error: status.error,
            is_persisted: cached.is_persisted,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn refresh_page<Request, Response, Item>(
        &self,
        key: &RiseConversationResourceKey,
        endpoint: RiseConversationEndpoint,
        request: Request,
        relative_id: Option<&str>,
        validate: Option<fn(&Response) -> anyhow::Result<()>>,
        page: impl FnOnce(Response) -> (Vec<Item>, Option<String>, bool) + Send,
        stable_id: impl Fn(&Item) -> String + Send + Sync,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<Item>>
    where
        Request: Serialize + Send + Sync,
        Response: DeserializeOwned + Send,
        Item: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    {
        let generation = self.begin_request(key)?;
        let mut guard =
            RiseConversationAuxiliaryRequestGuard { repository: self, key, generation, is_armed: true };
        let outcome: anyhow::Result<RiseConversationAuxiliaryPageSnapshot<Item>> = async {
            let response_value: Response = self.rpc.call(endpoint, request).await?;
            if let Some(validate) = validate {
                validate(&response_value)?;
            }
            if !self.is_current_request(key, generation) {
                return self.cached_page::<Item>(key).await;
            }

            let (incoming_items, incoming_cursor, incoming_has_more) = page(response_value);
            let previous = self.resources.cached::<RiseConversationAuxiliaryStoredPage<Item>>(key).await?.value;
            if !self.is_current_request(key, generation) {
                return self.cached_page::<Item>(key).await;
            }

            let base = match relative_id {
                None => Vec::new(),
                Some(_) => previous.map(|page| page.items).unwrap_or_default(),
            };
            let fallback_cursor = incoming_items.last().map(&stable_id);
            let cursor = Self::normalized(incoming_cursor.as_deref())
                .or_else(|| Self::normalized(fallback_cursor.as_deref()));
            let cursor_advanced = cursor.is_some() && cursor != Self::normalized(relative_id);
            let has_more = incoming_has_more && !incoming_items.is_empty() && cursor_advanced;
            let merged = Self::stable_merge(base, incoming_items, &stable_id);
            let stored = RiseConversationAuxiliaryStoredPage { items: merged, cursor, has_more };
            let revision = self.resources.store(&stored, key).await?;
            self.finish_request(key, generation);
            let status = self.status(key);
            Ok(RiseConversationAuxiliaryPageSnapshot {
                items: stored.items,
                cursor: stored.cursor,
                has_more,
                revision,
                load_state: Self::load_state(&status, true),
                error: status.error,
                is_persisted: key.persistence == RiseConversationResourceKeyPersistence::Persistent,
            })
        }
        .await;
        guard.is_armed = false;
        if let Err(error) = outcome.as_ref() {
            self.fail_request(key, generation, error);
        }
        outcome
    }

    fn status(&self, key: &RiseConversationResourceKey) -> RiseConversationAuxiliaryRequestStatus {
        self.state.with(|state| state.request_status.get(key).cloned().unwrap_or_default())
    }

    fn begin_request(&self, key: &RiseConversationResourceKey) -> anyhow::Result<u64> {
        self.state.with(|state| {
            let status = state.request_status.entry(key.clone()).or_default();
            if status.is_loading {
                return Err(RiseConversationAuxiliaryRepositoryError::RequestAlreadyInFlight(key.key.clone()).into());
            }
            status.generation = status.generation.wrapping_add(1);
            status.is_loading = true;
            status.error = None;
            Ok(status.generation)
        })
    }

    fn finish_request(&self, key: &RiseConversationResourceKey, generation: u64) {
        self.state.with(|state| {
            let Some(status) = state.request_status.get_mut(key) else {
                return;
            };
            if status.generation != generation {
                return;
            }
            status.is_loading = false;
            status.did_resolve = true;
            status.error = None;
        });
    }

    fn fail_request(&self, key: &RiseConversationResourceKey, generation: u64, error: &anyhow::Error) {
        let text = CanonErrors::load_failure_text(error);
        self.state.with(|state| {
            let Some(status) = state.request_status.get_mut(key) else {
                return;
            };
            if status.generation != generation {
                return;
            }
            status.is_loading = false;
            status.error = Some(text);
        });
    }

    fn is_current_request(&self, key: &RiseConversationResourceKey, generation: u64) -> bool {
        self.state.with(|state| state.request_status.get(key).is_some_and(|status| status.generation == generation))
    }

    fn load_state(
        status: &RiseConversationAuxiliaryRequestStatus,
        has_value: bool,
    ) -> RiseConversationAuxiliaryLoadState {
        if status.is_loading {
            return RiseConversationAuxiliaryLoadState::Loading;
        }
        if status.error.is_some() {
            return RiseConversationAuxiliaryLoadState::Failed;
        }
        if has_value || status.did_resolve {
            return RiseConversationAuxiliaryLoadState::Loaded;
        }
        RiseConversationAuxiliaryLoadState::Idle
    }

    fn require_non_replayable_mutation(endpoint: &RiseConversationEndpoint) -> anyhow::Result<()> {
        if matches!(endpoint.kind, RiseConversationEndpointKind::Mutation) {
            return Ok(());
        }
        Err(RiseConversationAuxiliaryRepositoryError::EndpointIsReplayable(format!(
            "{}.{}",
            endpoint.service, endpoint.method
        ))
        .into())
    }

    fn stable_merge<Item>(current: Vec<Item>, incoming: Vec<Item>, stable_id: impl Fn(&Item) -> String) -> Vec<Item> {
        let mut result = current;
        let mut index_by_id: HashMap<String, usize> = HashMap::with_capacity(result.len() + incoming.len());
        for (index, item) in result.iter().enumerate() {
            let identity = stable_id(item);
            if !identity.is_empty() {
                index_by_id.entry(identity).or_insert(index);
            }
        }
        for item in incoming {
            let identity = stable_id(&item);
            if !identity.is_empty()
                && let Some(index) = index_by_id.get(&identity)
            {
                result[*index] = item;
            } else {
                if !identity.is_empty() {
                    index_by_id.insert(identity, result.len());
                }
                result.push(item);
            }
        }
        result
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(value?);
        if value.is_empty() {
            return None;
        }
        Some(value.to_owned())
    }

    fn compatibility_resource_key_for_body(
        endpoint: &RiseConversationEndpoint,
        body: &RiseJSONValue,
        cache_identity: Option<&str>,
        persistence: RiseConversationResourceKeyPersistence,
    ) -> anyhow::Result<RiseConversationResourceKey> {
        let identity = match Self::normalized(cache_identity) {
            Some(cache_identity) => SaiAccountCacheScope::logical_key(&cache_identity),
            None => Self::canonical_body_digest(body)?,
        };
        Ok(Self::compatibility_resource_key(endpoint, &identity, persistence))
    }

    fn compatibility_resource_key(
        endpoint: &RiseConversationEndpoint,
        logical_cache_identity: &str,
        persistence: RiseConversationResourceKeyPersistence,
    ) -> RiseConversationResourceKey {
        RiseConversationResourceKey::with_persistence(
            &["compatibility", endpoint.service.as_str(), endpoint.method.as_str(), logical_cache_identity],
            persistence,
        )
    }

    fn canonical_body_digest(body: &RiseJSONValue) -> anyhow::Result<String> {
        let digest = Sha256::digest(body.data(true)?);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    /// public so tests can check chat and topic targeting without a network request
    pub fn message_projection_resource_keys(
        invalidation: &RiseMessageAuxiliaryInvalidation,
    ) -> HashSet<RiseConversationResourceKey> {
        Self::message_projection_invalidation_targets(invalidation).resource_keys
    }

    pub fn message_projection_presentation_cache_keys(
        invalidation: &RiseMessageAuxiliaryInvalidation,
        account_id: &str,
    ) -> HashSet<String> {
        let account_id = trimming_whitespaces_and_newlines(account_id);
        if account_id.is_empty() {
            return HashSet::new();
        }
        let prefix = SaiAccountCacheScope::prefix(account_id);
        Self::message_projection_invalidation_targets(invalidation)
            .presentation_cache_identities
            .iter()
            .map(|identity| format!("{prefix}{identity}"))
            .collect()
    }

    fn message_projection_invalidation_targets(
        invalidation: &RiseMessageAuxiliaryInvalidation,
    ) -> RiseConversationAuxiliaryInvalidationTargets {
        Self::message_projection_invalidation_targets_for(
            &invalidation.chat_id,
            &invalidation.topic_ids,
            &invalidation.projections,
        )
    }

    fn message_projection_invalidation_targets_for(
        chat_id: &str,
        topic_ids: &HashSet<String>,
        projections: &HashSet<RiseMessageAuxiliaryProjection>,
    ) -> RiseConversationAuxiliaryInvalidationTargets {
        let chat_id = trimming_whitespaces_and_newlines(chat_id);
        let mut keys = HashSet::new();
        let mut compatibility_logical_keys = HashSet::new();
        let mut presentation_cache_identities = HashSet::new();
        if chat_id.is_empty() || projections.is_empty() {
            return RiseConversationAuxiliaryInvalidationTargets {
                resource_keys: keys,
                compatibility_logical_keys,
                presentation_cache_identities,
            };
        }

        if projections.contains(&RiseMessageAuxiliaryProjection::Pinned) {
            keys.insert(RiseConversationResourceKeys::pinned_messages(chat_id));
            Self::insert_compatibility_keys(
                &RiseConversationEndpoints::get_pinned_messages(),
                &format!("getPinnedMessages-{chat_id}"),
                &mut keys,
                &mut compatibility_logical_keys,
                &mut presentation_cache_identities,
            );
        }

        let mut sorted_topic_ids: Vec<&String> = topic_ids.iter().collect();
        sorted_topic_ids.sort();
        let scoped_topic_ids: Vec<Option<&str>> =
            std::iter::once(None).chain(sorted_topic_ids.into_iter().map(|topic| Some(topic.as_str()))).collect();
        for projection in projections {
            if projection == &RiseMessageAuxiliaryProjection::Pinned {
                continue;
            }
            let Some((typed_kind, endpoint, cache_prefix)) = Self::projection_descriptor(projection) else {
                continue;
            };
            for topic_id in &scoped_topic_ids {
                keys.insert(RiseConversationResourceKeys::message_tab(typed_kind, chat_id, *topic_id));
                let scope = Self::compatibility_message_scope(chat_id, *topic_id);
                Self::insert_compatibility_keys(
                    &endpoint,
                    &format!("{cache_prefix}-{scope}"),
                    &mut keys,
                    &mut compatibility_logical_keys,
                    &mut presentation_cache_identities,
                );
            }
        }
        RiseConversationAuxiliaryInvalidationTargets {
            resource_keys: keys,
            compatibility_logical_keys,
            presentation_cache_identities,
        }
    }

    fn insert_compatibility_keys(
        endpoint: &RiseConversationEndpoint,
        logical_cache_identity: &str,
        keys: &mut HashSet<RiseConversationResourceKey>,
        logical_keys: &mut HashSet<String>,
        presentation_cache_identities: &mut HashSet<String>,
    ) {
        let persistent = Self::compatibility_resource_key(
            endpoint,
            logical_cache_identity,
            RiseConversationResourceKeyPersistence::Persistent,
        );
        keys.insert(Self::memory_companion_key(&persistent));
        keys.insert(Self::compatibility_resource_key(
            endpoint,
            logical_cache_identity,
            RiseConversationResourceKeyPersistence::MemoryOnly,
        ));
        logical_keys.insert(persistent.key.clone());
        keys.insert(persistent);
        presentation_cache_identities.insert(logical_cache_identity.to_owned());
    }

    fn projection_descriptor(
        projection: &RiseMessageAuxiliaryProjection,
    ) -> Option<(&'static str, RiseConversationEndpoint, &'static str)> {
        match projection {
            RiseMessageAuxiliaryProjection::Pinned => None,
            RiseMessageAuxiliaryProjection::Media => {
                Some(("media", RiseConversationEndpoints::get_media_messages(), "getMediaMessages"))
            }
            RiseMessageAuxiliaryProjection::Files => {
                Some(("files", RiseConversationEndpoints::get_file_messages(), "get_file_messages"))
            }
            RiseMessageAuxiliaryProjection::Music => {
                Some(("music", RiseConversationEndpoints::get_music_messages(), "get_music_messages"))
            }
            RiseMessageAuxiliaryProjection::Voice => {
                Some(("voice", RiseConversationEndpoints::get_audio_messages(), "get_audio_messages"))
            }
            RiseMessageAuxiliaryProjection::Gif => {
                Some(("gif", RiseConversationEndpoints::get_gif_messages(), "get_gif_messages"))
            }
            RiseMessageAuxiliaryProjection::Favorites => {
                Some(("favorites", RiseConversationEndpoints::get_favorite_messages(), "get_fav_messages"))
            }
            RiseMessageAuxiliaryProjection::Links => {
                Some(("links", RiseConversationEndpoints::get_link_messages(), "getLinkMessages"))
            }
        }
    }

    fn compatibility_message_scope(chat_id: &str, topic_id: Option<&str>) -> String {
        match Self::normalized(topic_id) {
            Some(topic_id) => format!("{chat_id}::t::{topic_id}"),
            None => chat_id.to_owned(),
        }
    }
}

impl RiseConversationAuxiliaryRepository {
    pub async fn cached_chat_calendar(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        window: &RiseChatCalendarWindow,
    ) -> anyhow::Result<RiseConversationAuxiliarySnapshot<GetChatCalendarGatewayDTO>> {
        let key = RiseConversationResourceKeys::chat_calendar(
            chat_id,
            topic_id,
            thread_root_id,
            window.start_ms(),
            window.end_ms,
        );
        self.cached_value::<GetChatCalendarGatewayDTO>(&key).await
    }

    pub async fn refresh_chat_calendar(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        window: &RiseChatCalendarWindow,
        include_previews: bool,
    ) -> anyhow::Result<RiseConversationAuxiliarySnapshot<GetChatCalendarGatewayDTO>> {
        let key = RiseConversationResourceKeys::chat_calendar(
            chat_id,
            topic_id,
            thread_root_id,
            window.start_ms(),
            window.end_ms,
        );
        self.refresh_value::<_, GetChatCalendarGatewayDTO>(
            &key,
            RiseConversationEndpoints::get_chat_calendar(),
            rise_request::message::GetChatCalendar {
                chat_id: chat_id.to_owned(),
                day_starts: window.day_starts.clone(),
                include_previews: Some(include_previews),
                thread_root_id: thread_root_id.map(str::to_owned),
                topic_id: topic_id.map(str::to_owned),
                user_chat_id: None,
                window_end_ms: window.end_ms,
            },
        )
        .await
    }
}

impl RiseConversationAuxiliaryRepository {
    pub async fn record_message_read_receipt_hint(&self, hint: RiseMessageReadReceiptHint) {
        let chat_id = trimming_whitespaces_and_newlines(&hint.chat_id);
        if chat_id.is_empty() || hint.read_at_milliseconds <= 0 {
            return;
        }
        let limit = usize::try_from(*READ_RECEIPT_HINT_LIMIT).unwrap_or(0);
        let message_ids = hint
            .message_ids
            .iter()
            .map(|message_id| trimming_whitespaces_and_newlines(message_id))
            .filter(|message_id| !message_id.is_empty())
            .take(limit);
        for message_id in message_ids {
            let key = RiseConversationResourceKeys::message_read_receipts(chat_id, message_id);
            let cached = self.resources.cached::<RiseMessageReadReceiptStoredResource>(&key).await.ok();
            if let Some(value) = cached.and_then(|cached| cached.value) {
                if value.is_local_hint != Some(true) {
                    continue;
                }
                let stored = value.response.representative_read_at.unwrap_or(0);
                if stored <= hint.read_at_milliseconds {
                    continue;
                }
            }
            let _ = self
                .resources
                .store(
                    &RiseMessageReadReceiptStoredResource::local_hint(message_id, hint.read_at_milliseconds),
                    &key,
                )
                .await;
        }
    }

    pub async fn cached_message_read_receipts(
        &self,
        chat_id: &str,
        message_id: &str,
    ) -> anyhow::Result<RiseConversationAuxiliarySnapshot<RiseMessageReadReceiptStoredResource>> {
        let key = RiseConversationResourceKeys::message_read_receipts(chat_id, message_id);
        self.cached_value::<RiseMessageReadReceiptStoredResource>(&key).await
    }

    pub async fn refresh_message_read_receipts(
        &self,
        _user_id: &str,
        chat_id: &str,
        message_id: &str,
    ) -> anyhow::Result<RiseConversationAuxiliarySnapshot<RiseMessageReadReceiptStoredResource>> {
        let key = RiseConversationResourceKeys::message_read_receipts(chat_id, message_id);
        let generation = self.begin_request(&key)?;
        let mut guard =
            RiseConversationAuxiliaryRequestGuard { repository: self, key: &key, generation, is_armed: true };
        let outcome: anyhow::Result<RiseConversationAuxiliarySnapshot<RiseMessageReadReceiptStoredResource>> = async {
            let readers: rise_view::message::Readers = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_message_read_users(),
                    rise_request::message::GetMessageReadUsers {
                        chat_id: chat_id.to_owned(),
                        limit: Some(100),
                        message_id: message_id.to_owned(),
                        relative_id: None,
                        up: Some(false),
                    },
                )
                .await?;
            let response = RiseStoredReadReceipts::from_canon(&readers);
            if !self.is_current_request(&key, generation) {
                return self.cached_value::<RiseMessageReadReceiptStoredResource>(&key).await;
            }
            let stored = RiseMessageReadReceiptStoredResource {
                response,
                refreshed_at_milliseconds: Self::now_milliseconds(),
                is_local_hint: None,
            };
            self.resources.store(&stored, &key).await?;
            self.finish_request(&key, generation);
            self.cached_value::<RiseMessageReadReceiptStoredResource>(&key).await
        }
        .await;
        guard.is_armed = false;
        if let Err(error) = outcome.as_ref() {
            self.fail_request(&key, generation, error);
        }
        outcome
    }
}

impl RiseConversationAuxiliaryRepository {
    const DEFAULT_MESSAGE_PAGE_LIMIT: i64 = 50;
    const DEFAULT_GLOBAL_SEARCH_LIMIT: i64 = 30;
    const DEFAULT_OVERLAY_ENTITY_LIMIT: i64 = 15;
    const DEFAULT_OVERLAY_CHAT_LIMIT: i64 = 30;
    const DEFAULT_OVERLAY_POST_LIMIT: i64 = 21;

    pub async fn cached_pinned_messages(
        &self,
        chat_id: &str,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>> {
        self.cached_page::<MessageWireDTO>(&RiseConversationResourceKeys::pinned_messages(chat_id)).await
    }

    pub async fn refresh_pinned_messages(
        &self,
        _user_id: &str,
        chat_id: &str,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>> {
        let limit = limit.unwrap_or(Self::DEFAULT_MESSAGE_PAGE_LIMIT);
        self.refresh_page::<_, rise_view::message::Pinned, MessageWireDTO>(
            &RiseConversationResourceKeys::pinned_messages(chat_id),
            RiseConversationEndpoints::get_pinned_messages(),
            rise_request::message::GetPinnedMessages {
                chat_id: chat_id.to_owned(),
                limit: Some(limit),
                relative_id: relative_id.map(str::to_owned),
                up: Some(true),
            },
            relative_id,
            None,
            move |response| {
                let items: Vec<MessageWireDTO> =
                    response.messages.iter().filter_map(|message| MessageWireDTO::from_canon(message).ok()).collect();
                let has_more = response.is_have_more.unwrap_or(items.len() as i64 >= limit);
                (items, response.relative_id, has_more)
            },
            Self::message_stable_id,
        )
        .await
    }

    pub async fn cached_media_messages(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<MediaItemWireDTO>> {
        self.cached_page::<MediaItemWireDTO>(&RiseConversationResourceKeys::message_tab("media", chat_id, topic_id))
            .await
    }

    pub async fn refresh_media_messages(
        &self,
        _user_id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<MediaItemWireDTO>> {
        let limit = limit.unwrap_or(Self::DEFAULT_MESSAGE_PAGE_LIMIT);
        self.refresh_page::<_, rise_response::message::GetMediaMessages, MediaItemWireDTO>(
            &RiseConversationResourceKeys::message_tab("media", chat_id, topic_id),
            RiseConversationEndpoints::get_media_messages(),
            rise_request::message::GetMediaMessages {
                chat_id: chat_id.to_owned(),
                limit: Some(limit),
                relative_id: relative_id.map(str::to_owned),
                topic_id: topic_id.map(str::to_owned),
                up: Some(true),
            },
            relative_id,
            None,
            move |response| {
                let items: Vec<MediaItemWireDTO> = response.media_items.iter().map(MediaItemWireDTO::from_canon).collect();
                let has_more = response.is_have_more.unwrap_or(items.len() as i64 >= limit);
                (items, response.next_cursor, has_more)
            },
            Self::media_stable_id,
        )
        .await
    }

    pub async fn cached_message_list_tab(
        &self,
        kind: RiseConversationMessageListTabKind,
        chat_id: &str,
        topic_id: Option<&str>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>> {
        self.cached_page::<MessageWireDTO>(&RiseConversationResourceKeys::message_tab(
            kind.raw_value(),
            chat_id,
            topic_id,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn refresh_message_list_tab(
        &self,
        kind: RiseConversationMessageListTabKind,
        user_id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        relative_id: Option<&str>,
        limit: Option<i64>,
        request_chat_id: Option<&str>,
        source_chat_id: Option<&str>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>> {
        let limit = limit.unwrap_or(Self::DEFAULT_MESSAGE_PAGE_LIMIT);
        match kind {
            RiseConversationMessageListTabKind::Files => {
                self.refresh_message_list_tab_list::<rise_view::message::Files>(
                    kind, user_id, chat_id, topic_id, relative_id, limit, request_chat_id, source_chat_id,
                )
                .await
            }
            RiseConversationMessageListTabKind::Music => {
                self.refresh_message_list_tab_list::<rise_view::message::Music>(
                    kind, user_id, chat_id, topic_id, relative_id, limit, request_chat_id, source_chat_id,
                )
                .await
            }
            RiseConversationMessageListTabKind::Voice => {
                self.refresh_message_list_tab_list::<rise_view::message::Audio>(
                    kind, user_id, chat_id, topic_id, relative_id, limit, request_chat_id, source_chat_id,
                )
                .await
            }
            RiseConversationMessageListTabKind::Gif => {
                self.refresh_message_list_tab_list::<rise_view::message::Gifs>(
                    kind, user_id, chat_id, topic_id, relative_id, limit, request_chat_id, source_chat_id,
                )
                .await
            }
            RiseConversationMessageListTabKind::Favorites => {
                self.refresh_message_list_tab_list::<rise_view::message::Favorites>(
                    kind, user_id, chat_id, topic_id, relative_id, limit, request_chat_id, source_chat_id,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn refresh_message_list_tab_list<List>(
        &self,
        kind: RiseConversationMessageListTabKind,
        _user_id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        relative_id: Option<&str>,
        limit: i64,
        request_chat_id: Option<&str>,
        source_chat_id: Option<&str>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>>
    where
        List: RiseCanonMessageList + DeserializeOwned + Send + 'static,
    {
        let traces_gif_page = matches!(kind, RiseConversationMessageListTabKind::Gif);
        let page = self
            .refresh_page::<_, List, MessageWireDTO>(
                &RiseConversationResourceKeys::message_tab(kind.raw_value(), chat_id, topic_id),
                kind.endpoint(),
                kind.request(request_chat_id.unwrap_or(chat_id), topic_id, source_chat_id, relative_id, limit),
                relative_id,
                None,
                move |response| {
                    let items: Vec<MessageWireDTO> = response
                        .messages()
                        .iter()
                        .filter_map(|message| MessageWireDTO::from_canon(message).ok())
                        .collect();
                    if traces_gif_page {
                        tracing::warn!(
                            target: "riseonly::X1567",
                            received = response.messages().len(),
                            decoded = items.len(),
                            "gif tab page answered"
                        );
                    }
                    let has_more = response.is_have_more().unwrap_or(items.len() as i64 >= limit);
                    (items, response.list_cursor(), has_more)
                },
                Self::message_stable_id,
            )
            .await;
        if traces_gif_page && let Err(error) = &page {
            tracing::warn!(target: "riseonly::X1567", chat_id, %error, "gif tab page failed");
        }
        page
    }

    pub async fn cached_link_messages(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<LinkMessageWireDTO>> {
        self.cached_page::<LinkMessageWireDTO>(&RiseConversationResourceKeys::message_tab("links", chat_id, topic_id))
            .await
    }

    pub async fn refresh_link_messages(
        &self,
        _user_id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<LinkMessageWireDTO>> {
        let limit = limit.unwrap_or(Self::DEFAULT_MESSAGE_PAGE_LIMIT);
        self.refresh_page::<_, rise_response::message::GetLinkMessages, LinkMessageWireDTO>(
            &RiseConversationResourceKeys::message_tab("links", chat_id, topic_id),
            RiseConversationEndpoints::get_link_messages(),
            rise_request::message::GetLinkMessages {
                chat_id: chat_id.to_owned(),
                limit: Some(limit),
                relative_id: relative_id.map(str::to_owned),
                topic_id: topic_id.map(str::to_owned),
                up: Some(true),
            },
            relative_id,
            None,
            move |response| {
                let items: Vec<LinkMessageWireDTO> =
                    response.link_messages.iter().map(LinkMessageWireDTO::from_canon).collect();
                let has_more = response.is_have_more.unwrap_or(items.len() as i64 >= limit);
                (items, None, has_more)
            },
            Self::link_stable_id,
        )
        .await
    }
}

impl RiseConversationAuxiliaryRepository {
    pub async fn cached_global_message_search(
        &self,
        kind: ChatSearchMessageKind,
        query: &str,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>> {
        self.cached_page::<MessageWireDTO>(&RiseConversationResourceKeys::global_message_search(kind.raw_value(), query))
            .await
    }

    pub async fn refresh_global_message_search(
        &self,
        kind: ChatSearchMessageKind,
        _user_id: &str,
        query: &str,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<MessageWireDTO>> {
        let limit = limit.unwrap_or(Self::DEFAULT_GLOBAL_SEARCH_LIMIT);
        let normalized_query = trimming_whitespaces_and_newlines(query);
        self.refresh_page::<_, rise_response::message::SearchMessagesGlobal, MessageWireDTO>(
            &RiseConversationResourceKeys::global_message_search(kind.raw_value(), normalized_query),
            RiseConversationEndpoints::search_messages(),
            rise_request::message::SearchMessagesGlobal {
                chat_type: None,
                content_type: Some(kind.wire_content_type().to_owned()),
                limit: Some(limit),
                query: normalized_query.to_owned(),
                relative_id: relative_id.map(str::to_owned),
                up: Some(false),
            },
            relative_id,
            Some(Self::validate_global_message_search),
            move |response| {
                let items: Vec<MessageWireDTO> =
                    response.messages.iter().filter_map(|message| MessageWireDTO::from_canon(message).ok()).collect();
                let has_more = response.is_have_more.unwrap_or(items.len() as i64 >= limit);
                (items, response.relative_id, has_more)
            },
            Self::message_stable_id,
        )
        .await
    }

    fn validate_global_message_search(response: &rise_response::message::SearchMessagesGlobal) -> anyhow::Result<()> {
        if response.error.as_deref() == Some(SEARCH_BACKEND_UNAVAILABLE_MARKER) {
            return Err(RiseConversationAuxiliaryRepositoryError::SearchBackendUnavailable.into());
        }
        Ok(())
    }

    fn message_stable_id(message: &MessageWireDTO) -> String {
        for value in [message.server_id.as_deref(), message.id.as_deref(), message.temp_id.as_deref()] {
            if let Some(normalized) = Self::normalized(value) {
                return normalized;
            }
        }
        let timestamp = message.created_at.or(message.timestamp).unwrap_or(0.0);
        [
            message.chat_id.clone().unwrap_or_default(),
            message.sender_id.clone().unwrap_or_default(),
            format!("{timestamp:?}"),
            message.content_type.clone().unwrap_or_default(),
            message.content.clone().or_else(|| message.caption.clone()).unwrap_or_default(),
        ]
        .join("|")
    }

    fn media_stable_id(media: &MediaItemWireDTO) -> String {
        if let Some(id) = Self::normalized(media.id.as_deref()) {
            return id;
        }
        let file_identity = media
            .media_info
            .as_ref()
            .and_then(|info| info.file_id.clone().or_else(|| info.file_url.clone()))
            .unwrap_or_default();
        let timestamp = media.created_at.or(media.timestamp).unwrap_or(0.0);
        [
            media.message_id.clone().unwrap_or_default(),
            media.media_index.unwrap_or(0).to_string(),
            file_identity,
            format!("{timestamp:?}"),
        ]
        .join("|")
    }

    fn link_stable_id(link: &LinkMessageWireDTO) -> String {
        if let Some(message_id) = Self::normalized(link.message_id.as_deref()) {
            return message_id;
        }
        [
            link.link.clone().unwrap_or_default(),
            format!("{:?}", link.created_at.unwrap_or(0.0)),
            link.sender.as_ref().and_then(|sender| sender.id.clone()).unwrap_or_default(),
        ]
        .join("|")
    }
}

impl RiseConversationAuxiliaryRepository {
    pub async fn cached_chat_search_recents(&self) -> anyhow::Result<Vec<RiseChatSearchRecentItem>> {
        let snapshot = self
            .resources
            .cached::<RiseChatSearchRecentsPayload>(&RiseConversationResourceKeys::chat_search_recents(&self.account_id))
            .await?;
        Ok(snapshot.value.map(|payload| payload.items).unwrap_or_default())
    }

    pub async fn store_chat_search_recents(&self, items: Vec<RiseChatSearchRecentItem>) -> anyhow::Result<()> {
        self.resources
            .store(
                &RiseChatSearchRecentsPayload { items },
                &RiseConversationResourceKeys::chat_search_recents(&self.account_id),
            )
            .await?;
        Ok(())
    }

    pub async fn clear_chat_search_recents(&self) -> anyhow::Result<()> {
        self.resources.remove(&RiseConversationResourceKeys::chat_search_recents(&self.account_id)).await
    }

    pub async fn cached_chat_overlay_entities(
        &self,
        query: &str,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<SearchResultItemDTO>> {
        self.cached_page::<SearchResultItemDTO>(&RiseConversationResourceKeys::chat_overlay_entities(query)).await
    }

    pub async fn refresh_chat_overlay_entities(
        &self,
        _user_id: &str,
        query: &str,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<SearchResultItemDTO>> {
        let limit = limit.unwrap_or(Self::DEFAULT_OVERLAY_ENTITY_LIMIT);
        let normalized_query = trimming_whitespaces_and_newlines(query).trim_matches('@');
        self.refresh_page::<_, GlobalSearchGatewayDTO, SearchResultItemDTO>(
            &RiseConversationResourceKeys::chat_overlay_entities(normalized_query),
            RiseConversationEndpoints::global_search(),
            rise_request::search::GlobalSearch {
                category: None,
                chats_limit: None,
                chats_only: None,
                chats_relative_id: None,
                include_chats: Some(false),
                limit: Some(limit),
                query: Some(normalized_query.to_owned()),
                relative_id: relative_id.map(str::to_owned),
                up: Some(false),
            },
            relative_id,
            None,
            |response| (response.global_results, response.global_next_cursor, response.global_has_more),
            |item| item.entity_id.clone(),
        )
        .await
    }

    pub async fn cached_chat_overlay_chats(
        &self,
        query: &str,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<RiseStoredChat>> {
        self.cached_page::<RiseStoredChat>(&RiseConversationResourceKeys::chat_overlay_chats(query)).await
    }

    pub async fn refresh_chat_overlay_chats(
        &self,
        _user_id: &str,
        query: &str,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<RiseStoredChat>> {
        let limit = limit.unwrap_or(Self::DEFAULT_OVERLAY_CHAT_LIMIT);
        let normalized_query = trimming_whitespaces_and_newlines(query).trim_matches('@');
        self.refresh_page::<_, GlobalSearchGatewayDTO, RiseStoredChat>(
            &RiseConversationResourceKeys::chat_overlay_chats(normalized_query),
            RiseConversationEndpoints::global_search(),
            rise_request::search::GlobalSearch {
                category: None,
                chats_limit: Some(limit),
                chats_only: Some(true),
                chats_relative_id: relative_id.map(str::to_owned),
                include_chats: None,
                limit: None,
                query: Some(normalized_query.to_owned()),
                relative_id: None,
                up: Some(false),
            },
            relative_id,
            None,
            |response| {
                (
                    response.chats.iter().map(RiseStoredChat::from_canon).collect(),
                    response.chats_next_cursor,
                    response.chats_has_more,
                )
            },
            |chat| chat.server_id.clone().unwrap_or_else(|| chat.id.clone()),
        )
        .await
    }

    pub async fn cached_chat_overlay_posts(
        &self,
        query: &str,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<GetFeedPostItemDTO>> {
        self.cached_page::<GetFeedPostItemDTO>(&RiseConversationResourceKeys::chat_overlay_posts(query)).await
    }

    pub async fn refresh_chat_overlay_posts(
        &self,
        query: &str,
        relative_id: Option<&str>,
        limit: Option<i64>,
    ) -> anyhow::Result<RiseConversationAuxiliaryPageSnapshot<GetFeedPostItemDTO>> {
        let limit = limit.unwrap_or(Self::DEFAULT_OVERLAY_POST_LIMIT);
        let normalized_query = trimming_whitespaces_and_newlines(query);
        self.refresh_page::<_, rise_response::post::SearchPosts, GetFeedPostItemDTO>(
            &RiseConversationResourceKeys::chat_overlay_posts(normalized_query),
            RiseConversationEndpoints::search_posts(),
            rise_request::post::SearchPosts {
                content_kind: None,
                limit: Some(limit),
                q: Some(normalized_query.to_owned()),
                relative_id: relative_id.and_then(|relative_id| relative_id.parse::<i64>().ok()),
                tags: None,
                up: Some(false),
            },
            relative_id,
            None,
            |response| (response.list, response.relative_id.map(|relative_id| relative_id.to_string()), response.is_have_more),
            |post| post.id.to_string(),
        )
        .await
    }
}
