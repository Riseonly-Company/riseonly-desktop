use std::any::Any;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::LocalBoxFuture;
use gpui::{App, AsyncApp};
use rise_canon::{rise_key, rise_wire_methods};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseJSONValue};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_payload_path::{SaiJSONCodec, SaiPayloadPath};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWsOnSuccess;
use crate::core::lib::sai::modules::sai_ws::sai_ws_envelope::SaiWsEnvelopeBuilder;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationStateStore;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsDiskCachePolicy, SaiWsPendingCacheBinding, SaiWsRequestOptions,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiWsScreenRefresh;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_coordinator::SaiWsScreenRefreshCoordinator;
use crate::modules::chat::engine::chats::rise_chat_repository_models::RiseChatWireCodec;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryRepositoryError, RiseConversationCacheWriteSource,
    RiseConversationCompatibilityRequestReceipt, RiseConversationResolutionSource,
};
use crate::modules::chat::engine::core::rise_conversation_compatibility_preparer::{
    RiseConversationCompatibilityJSON, RiseConversationCompatibilityPreparer,
};
use crate::modules::chat::engine::core::rise_conversation_debug_trace::{
    RiseConversationDebugTrace, RiseConversationDebugTraceFlowToken,
};
use crate::modules::chat::engine::core::rise_conversation_file_upload_pipeline::RiseConversationFileUploadPipeline;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::core::rise_conversation_request_state::RiseConversationStateError;
use crate::modules::chat::engine::core::rise_conversation_resource_repository::RiseConversationResourceKeyPersistence;
use crate::modules::chat::engine::core::rise_conversation_resource_repository::RiseConversationResourceReadSource;
use crate::modules::chat::engine::rise_conversation_domain::{RiseConversationDomain, RiseConversationDomainRegistry};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::{
    CreateChatResultDTO, CreateChatResultDTOExt, RiseStoredChat,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseConversationCompatibilityBridgeError {
    RepositoryOnlyEndpoint { service: String, method: String },
    InvalidRequestBody,
    InvalidResponseBody,
}

impl fmt::Display for RiseConversationCompatibilityBridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RepositoryOnlyEndpoint { service, method } => {
                write!(formatter, "{service}.{method} must be dispatched by its Rise repository")
            }
            Self::InvalidRequestBody => formatter.write_str("Conversation request body is not valid JSON"),
            Self::InvalidResponseBody => formatter.write_str("Conversation response body is not valid JSON"),
        }
    }
}

impl std::error::Error for RiseConversationCompatibilityBridgeError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseConversationCompatibilityBridgeCachePlan {
    pub persistence: RiseConversationResourceKeyPersistence,
    pub allow_persistent_creation: bool,
    pub cache_wire_response: bool,
    pub cache_merged_response: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseConversationCompatibilityBridgeScreenRefreshDisposition {
    None,
    RevalidateCachedValue,
    MarkNetworkResolved,
}

static REPOSITORY_ONLY_ROUTES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    use rise_wire_methods::{chat, message};
    [
        (chat::SERVICE, chat::GET_CHATS),
        (chat::SERVICE, chat::GET_ARCHIVED_CHATS),
        (chat::SERVICE, chat::GET_MESSAGE_REQUESTS),
        (chat::SERVICE, chat::GET_CHAT_BY_ID),
        (chat::SERVICE, chat::JOIN_CHAT),
        (chat::SERVICE, chat::JOIN_CHAT_BY_LINK),
        (chat::SERVICE, chat::ARCHIVE_CHAT),
        (chat::SERVICE, chat::UNARCHIVE_CHAT),
        (chat::SERVICE, chat::RESOLVE_MESSAGE_REQUEST),
        (chat::SERVICE, chat::DELETE_ALL_MESSAGE_REQUESTS),
        (chat::SERVICE, chat::MUTE_CHAT),
        (chat::SERVICE, chat::UNMUTE_CHAT),
        (chat::SERVICE, chat::UPDATE_CHAT_NOTIFICATIONS),
        (chat::SERVICE, chat::LEAVE_CHAT),
        (chat::SERVICE, chat::DELETE_CHAT),
        (chat::SERVICE, chat::UNPIN_CHAT),
        (chat::SERVICE, chat::REORDER_PINNED_CHATS),
        (chat::SERVICE, chat::UPDATE_CHAT),
        (message::SERVICE, message::GET_MESSAGES),
        (message::SERVICE, message::GET_MESSAGES_AROUND),
        (message::SERVICE, message::CREATE_MESSAGE),
        (message::SERVICE, message::EDIT_MESSAGE),
        (message::SERVICE, message::DELETE_MESSAGES),
        (message::SERVICE, message::PIN_MESSAGE),
        (message::SERVICE, message::UNPIN_MESSAGE),
        (message::SERVICE, message::UNPIN_ALL_MESSAGES),
        (message::SERVICE, message::DELETE_ALL_MESSAGES_FROM_USER),
        (message::SERVICE, message::FORWARD_MESSAGES),
        (message::SERVICE, message::SHARE_PAYLOAD),
        (message::SERVICE, message::MARK_CHAT_AS_READ),
        (message::SERVICE, message::MARK_MEDIA_AS_PLAYED),
        (message::SERVICE, message::MESSAGE_TYPING),
        (message::SERVICE, message::ADD_REACTION),
        (message::SERVICE, message::REMOVE_REACTION),
    ]
    .into_iter()
    .map(|(service, method)| format!("{service}.{method}"))
    .collect()
});

struct ScreenRefreshRegistration {
    cache_key: String,
    refresh: SaiWsScreenRefresh,
    resend: Rc<dyn Fn(&mut App)>,
}

#[derive(Clone, Copy)]
struct FlowTrace {
    cache_lookup: Option<RiseConversationResourceReadSource>,
    resolution: Option<RiseConversationResolutionSource>,
    cache_write: RiseConversationCacheWriteSource,
    coalesced: bool,
}

impl FlowTrace {
    fn complete(
        &self,
        token: Option<&RiseConversationDebugTraceFlowToken>,
        stale: bool,
        published_to_ui: bool,
        error_message: Option<&str>,
        cx: &mut App,
    ) {
        RiseConversationDebugTrace::complete_flow(
            token,
            self.cache_lookup,
            self.resolution,
            self.cache_write,
            self.coalesced,
            stale,
            published_to_ui,
            error_message,
            cx,
        );
    }
}

/// keeps untouched cold path screens working without the old transport or disk cache
/// hot timelines and chat lists are refused on purpose
pub enum RiseConversationCompatibilityBridge {}

impl RiseConversationCompatibilityBridge {
    pub fn handle(
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        cx: &mut App,
    ) -> LocalBoxFuture<'static, anyhow::Result<Option<u64>>> {
        if !Self::is_conversation_service(&options.service) {
            return async { Ok(None) }.boxed_local();
        }
        let mut cx = cx.to_async();
        async move { Self::handle_conversation(data_value, options, on_success, &mut cx).await.map(Some) }.boxed_local()
    }

    fn is_conversation_service(service: &str) -> bool {
        service == rise_wire_methods::chat::SERVICE
            || service == rise_wire_methods::message::SERVICE
            || service == rise_wire_methods::tag::SERVICE
    }

    fn is_initial_page(options: &SaiWsRequestOptions) -> bool {
        options.pagination.as_ref().is_none_or(|pagination| pagination.is_initial_page())
    }

    async fn handle_conversation(
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<u64> {
        let domain = RiseConversationActionExecutor::require_domain(cx).await?;
        if options.account_user_id.as_deref() != Some(domain.account_id.as_str())
            || !SaiAccountCacheScope::is_current_account_epoch(
                options.account_user_id.as_deref(),
                options.account_epoch.as_deref(),
            )
        {
            return Err(RiseConversationStateError::StaleAccount.into());
        }

        let route = format!("{}.{}", options.service, options.method);
        let prepared_data_value = if RiseConversationFileUploadPipeline::has_files(&options) {
            RiseConversationFileUploadPipeline::prepare(data_value, &options, cx).await?
        } else {
            data_value
        };

        let body = Self::request_body(&prepared_data_value)?;
        Self::validate_publication(&domain, &options, None, cx).await?;
        let repository_owned = REPOSITORY_ONLY_ROUTES.contains(&route);
        let cache_plan =
            Self::cache_plan(options.disk_cache_policy, options.store_response_in_cache, options.pagination.is_some());
        let has_immediate_cached_response = if !repository_owned && !options.reload_if_cached && Self::is_initial_page(&options)
        {
            domain
                .auxiliary
                .has_compatibility_cached_response(
                    &options.service,
                    &options.method,
                    &body,
                    options.cache_key.as_deref(),
                    cache_plan.persistence,
                    cache_plan.allow_persistent_creation,
                )
                .await
                .unwrap_or(false)
        } else {
            false
        };
        let debug_trace = cx.update(|cx| RiseConversationDebugTrace::begin_flow(&body, &options, repository_owned, cx));
        let screen_refresh_registration = Self::register_screen_refresh(
            repository_owned,
            &prepared_data_value,
            &options,
            on_success.as_ref(),
            cx,
        );
        RiseConversationDebugTrace::with_flow(
            debug_trace.as_ref(),
            Self::run_flow(
                domain,
                route,
                body,
                options,
                on_success,
                cx,
                repository_owned,
                cache_plan,
                has_immediate_cached_response,
                debug_trace.as_ref(),
                screen_refresh_registration,
            ),
        )
        .await
    }

    fn register_screen_refresh(
        repository_owned: bool,
        prepared_data_value: &Value,
        options: &SaiWsRequestOptions,
        on_success: Option<&SaiWsOnSuccess>,
        cx: &mut AsyncApp,
    ) -> Option<ScreenRefreshRegistration> {
        if repository_owned || options.is_screen_refresh_wire || !Self::is_initial_page(options) {
            return None;
        }
        let refresh = options.screen_refresh.clone().filter(|refresh| refresh.enabled)?;
        let cache_key = options.cache_key.clone().filter(|cache_key| !cache_key.is_empty())?;
        let mut refresh_options = options.clone();
        refresh_options.reload_if_cached = true;
        refresh_options.force_bypass_cache_read = true;
        refresh_options.reflect_pending_in_ui = false;
        refresh_options.screen_refresh = None;
        refresh_options.is_screen_refresh_wire = true;
        let resend_data_value = prepared_data_value.clone();
        let resend_on_success = on_success.cloned();
        let resend_cache_key = cache_key.clone();
        let resend: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            SaiWsScreenRefreshCoordinator::mark_revalidated(&resend_cache_key, cx);
            let request =
                Self::handle(resend_data_value.clone(), refresh_options.clone(), resend_on_success.clone(), cx);
            cx.spawn(async move |_| {
                let _ = request.await;
            })
            .detach();
        });
        let registered_resend = resend.clone();
        cx.update(|cx| {
            SaiWsScreenRefreshCoordinator::install_lifecycle_observers_if_needed(cx);
            SaiWsScreenRefreshCoordinator::register(
                &cache_key,
                &refresh,
                Box::new(move |cx: &mut App| registered_resend(cx)),
                cx,
            );
        });
        Some(ScreenRefreshRegistration { cache_key, refresh, resend })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_flow(
        domain: Arc<RiseConversationDomain>,
        route: String,
        body: RiseJSONValue,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        cx: &mut AsyncApp,
        repository_owned: bool,
        cache_plan: RiseConversationCompatibilityBridgeCachePlan,
        has_immediate_cached_response: bool,
        debug_trace: Option<&RiseConversationDebugTraceFlowToken>,
        screen_refresh_registration: Option<ScreenRefreshRegistration>,
    ) -> anyhow::Result<u64> {
        Self::validate_publication(&domain, &options, None, cx).await?;
        let binding = SaiWsPendingCacheBinding::new(&options);
        cx.update(|cx| {
            SaiWsResponseCache::prepare_for_request(&options, cx);
            SaiWsResponseCache::note_active_slice(
                options.sai_slice.as_ref(),
                options.cache_key.as_deref(),
                options.take_path.as_deref(),
                cx,
            );
            if options.reflect_pending_in_ui && !has_immediate_cached_response && Self::is_initial_page(&options)
                && let Some(slice) = &options.sai_slice
            {
                slice.mark_pending();
            }
            SaiWsPaginationStateStore::begin(Some(&binding), cx);
        });

        let mut trace = FlowTrace {
            cache_lookup: None,
            resolution: None,
            cache_write: RiseConversationCacheWriteSource::None,
            coalesced: false,
        };

        let attempt: anyhow::Result<u64> = async {
            let response: RiseJSONValue;
            let receipt: Option<RiseConversationCompatibilityRequestReceipt>;
            if repository_owned {
                response = domain.repository_call(&options.service, &options.method, &body).await?;
                receipt = None;
            } else {
                let result = domain
                    .auxiliary
                    .compatibility_call_result(
                        &options.service,
                        &options.method,
                        body.clone(),
                        options.cache_key.as_deref(),
                        cache_plan.persistence,
                        !options.reload_if_cached && Self::is_initial_page(&options),
                        cache_plan.cache_wire_response,
                        cache_plan.allow_persistent_creation,
                    )
                    .await?;
                response = result.value;
                receipt = result.receipt;
                trace.cache_lookup = result.cache_lookup_source;
                trace.resolution = Some(result.resolution_source);
                trace.cache_write = result.cache_write_source;
                trace.coalesced = result.was_coalesced;
            }
            Self::validate_publication(&domain, &options, receipt.as_ref(), cx).await?;
            let existing = match (&options.pagination, &options.cache_key) {
                (Some(_), Some(cache_key)) => cx.update(|cx| SaiWsResponseCache::memory_only(cache_key, cx)),
                _ => None,
            };
            let existing_value = existing.as_ref().map(RiseConversationCompatibilityJSON::value).transpose()?;
            let pagination = options.pagination.clone();
            let items_path = options.path_to_array.clone().unwrap_or_default();
            let item_id_path = options.path_to_id.clone();
            let created_chat_request = (route == Self::create_chat_route()).then(|| body.clone());
            let prepared = RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    RiseConversationCompatibilityPreparer::shared().prepare(
                        &response,
                        existing_value.as_ref(),
                        pagination.as_ref(),
                        &items_path,
                        item_id_path.as_deref(),
                        created_chat_request.as_ref(),
                    )
                })
                .await
                .map_err(|_| CancellationError)??;
            Self::validate_publication(&domain, &options, receipt.as_ref(), cx).await?;
            if let Some(created) = prepared.created_chat {
                domain.chats.store_created_chat(created).await?;
                Self::validate_publication(&domain, &options, receipt.as_ref(), cx).await?;
            }
            let raw = prepared.raw;
            let fulfilled = prepared.fulfilled;
            if let Some(result) = &prepared.pagination {
                if cache_plan.cache_merged_response
                    && !repository_owned
                    && let (Some(receipt), Some(merged_value)) = (&receipt, &prepared.merged_value)
                {
                    trace.cache_write = domain
                        .auxiliary
                        .store_compatibility_response(
                            merged_value,
                            receipt,
                            &options.service,
                            &options.method,
                            &body,
                            options.cache_key.as_deref(),
                            cache_plan.persistence,
                            cache_plan.allow_persistent_creation,
                        )
                        .await?;
                }
                Self::validate_publication(&domain, &options, receipt.as_ref(), cx).await?;
                cx.update(|cx| {
                    if let Some(cache_key) = &options.cache_key
                        && options.store_response_in_cache
                    {
                        SaiWsResponseCache::hydrate_memory(cache_key, &fulfilled, false, cx);
                    }
                    SaiWsPaginationStateStore::finish(Some(&binding), result, cx);
                });
            } else {
                cx.update(|cx| {
                    if let Some(cache_key) = &options.cache_key
                        && options.store_response_in_cache
                    {
                        SaiWsResponseCache::hydrate_memory(cache_key, &raw, false, cx);
                    }
                });
            }

            let completed_trace = trace;
            cx.update(|cx| -> anyhow::Result<()> {
                if !Self::is_publication_current(&domain, &options, cx) {
                    return Err(RiseConversationStateError::StaleAccount.into());
                }
                if let Some(slice) = &options.sai_slice {
                    slice.mark_fulfilled(Some(&fulfilled), options.take_path.as_deref());
                }
                if let Some(on_success) = &on_success {
                    on_success(Self::callback_payload(&options, &raw), cx);
                }
                completed_trace.complete(debug_trace, false, options.sai_slice.is_some(), None, cx);
                if let Some(registration) = &screen_refresh_registration {
                    match Self::screen_refresh_disposition(completed_trace.resolution) {
                        RiseConversationCompatibilityBridgeScreenRefreshDisposition::None => {}
                        RiseConversationCompatibilityBridgeScreenRefreshDisposition::RevalidateCachedValue => {
                            if SaiWsScreenRefreshCoordinator::should_background_revalidate(
                                &registration.cache_key,
                                &registration.refresh,
                                cx,
                            ) {
                                (registration.resend)(cx);
                            }
                        }
                        RiseConversationCompatibilityBridgeScreenRefreshDisposition::MarkNetworkResolved => {
                            SaiWsScreenRefreshCoordinator::mark_revalidated(&registration.cache_key, cx);
                        }
                    }
                }
                Ok(())
            })?;
            Ok(SaiWsEnvelopeBuilder::make_outbound_id())
        }
        .await;

        let error = match attempt {
            Ok(request_id) => return Ok(request_id),
            Err(error) => error,
        };
        let is_current = error.downcast_ref::<CancellationError>().is_none()
            && cx.update(|cx| Self::is_publication_current(&domain, &options, cx));
        if matches!(
            error.downcast_ref::<RiseConversationAuxiliaryRepositoryError>(),
            Some(RiseConversationAuxiliaryRepositoryError::StaleCompatibilityResult)
        ) {
            if !is_current {
                let stale_account = anyhow::Error::new(RiseConversationStateError::StaleAccount);
                let label = RiseConversationDebugTrace::error_label(&stale_account);
                cx.update(|cx| trace.complete(debug_trace, false, false, Some(label.as_str()), cx));
                return Err(stale_account);
            }
            cx.update(|cx| trace.complete(debug_trace, true, false, None, cx));
            return Ok(SaiWsEnvelopeBuilder::make_outbound_id());
        }
        if !is_current {
            let label = RiseConversationDebugTrace::error_label(&error);
            cx.update(|cx| trace.complete(debug_trace, false, false, Some(label.as_str()), cx));
            return Err(error);
        }
        cx.update(|cx| SaiWsPaginationStateStore::fail(Some(&binding), &error, cx));
        if Self::is_initial_page(&options)
            && let Some(slice) = &options.sai_slice
        {
            slice.mark_rejected(&error);
        }
        let label = RiseConversationDebugTrace::error_label(&error);
        cx.update(|cx| trace.complete(debug_trace, false, false, Some(label.as_str()), cx));
        Err(error)
    }

    fn create_chat_route() -> String {
        format!("{}.{}", rise_wire_methods::chat::SERVICE, rise_wire_methods::chat::CREATE_CHAT)
    }

    fn callback_payload(options: &SaiWsRequestOptions, raw: &Value) -> Option<Box<dyn Any>> {
        options
            .sai_slice
            .as_ref()
            .and_then(|slice| slice.callback_payload(Some(raw), options.take_path.as_deref()))
            .or_else(|| {
                SaiPayloadPath::extract(Some(raw), options.take_path.as_deref())
                    .map(|extracted| Box::new(extracted.clone()) as Box<dyn Any>)
            })
    }

    /// stored engine response of a read routed through handle, decoded off the main thread
    /// none when nothing is stored or the account changed
    pub async fn cached_response<Body: Serialize, Reply: DeserializeOwned + Send + 'static>(
        body: &Body,
        options: &SaiWsRequestOptions,
        cx: &mut AsyncApp,
    ) -> Option<Reply> {
        let value = Self::cached_value(body, options, cx).await?;
        Self::decode_cached_value(value).await
    }

    async fn decode_cached_value<Reply: DeserializeOwned + Send + 'static>(value: RiseJSONValue) -> Option<Reply> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || {
                let data = value.data(false).ok()?;
                let object: Value = serde_json::from_slice(&data).ok()?;
                SaiJSONCodec::decode(Some(&object))
            })
            .await
            .ok()
            .flatten()
    }

    async fn cached_value<Body: Serialize>(
        body: &Body,
        options: &SaiWsRequestOptions,
        cx: &mut AsyncApp,
    ) -> Option<RiseJSONValue> {
        if !Self::is_conversation_service(&options.service) {
            return None;
        }
        let data_value = SaiWsEnvelopeBuilder::json_object_from_encodable(body).ok()?;
        let request_body = Self::request_body(&data_value).ok()?;
        let domain = RiseConversationActionExecutor::require_domain(cx).await.ok()?;
        if options.account_user_id.as_deref() != Some(domain.account_id.as_str()) {
            return None;
        }
        let plan =
            Self::cache_plan(options.disk_cache_policy, options.store_response_in_cache, options.pagination.is_some());
        let value = domain
            .auxiliary
            .compatibility_cached_response(
                &options.service,
                &options.method,
                &request_body,
                options.cache_key.as_deref(),
                plan.persistence,
                plan.allow_persistent_creation,
            )
            .await
            .ok()
            .flatten()?;
        cx.update(|cx| Self::is_publication_current(&domain, options, cx)).then_some(value)
    }

    pub fn cache_plan(
        disk_policy: SaiWsDiskCachePolicy,
        store_response_in_cache: bool,
        is_paginated: bool,
    ) -> RiseConversationCompatibilityBridgeCachePlan {
        let (persistence, allow_persistent_creation) = match disk_policy {
            SaiWsDiskCachePolicy::Persistent => (RiseConversationResourceKeyPersistence::Persistent, true),
            SaiWsDiskCachePolicy::PreserveExisting => (RiseConversationResourceKeyPersistence::Persistent, false),
            SaiWsDiskCachePolicy::MemoryOnly => (RiseConversationResourceKeyPersistence::MemoryOnly, true),
        };
        RiseConversationCompatibilityBridgeCachePlan {
            persistence,
            allow_persistent_creation,
            cache_wire_response: store_response_in_cache && !is_paginated,
            cache_merged_response: store_response_in_cache && is_paginated,
        }
    }

    pub fn screen_refresh_disposition(
        source: Option<RiseConversationResolutionSource>,
    ) -> RiseConversationCompatibilityBridgeScreenRefreshDisposition {
        match source {
            Some(RiseConversationResolutionSource::ResourceMemory | RiseConversationResolutionSource::PersistentStore) => {
                RiseConversationCompatibilityBridgeScreenRefreshDisposition::RevalidateCachedValue
            }
            Some(
                RiseConversationResolutionSource::Network
                | RiseConversationResolutionSource::NetworkFallbackMemory
                | RiseConversationResolutionSource::NetworkFallbackPersistentStore,
            ) => RiseConversationCompatibilityBridgeScreenRefreshDisposition::MarkNetworkResolved,
            Some(RiseConversationResolutionSource::NormalizedStore) | None => {
                RiseConversationCompatibilityBridgeScreenRefreshDisposition::None
            }
        }
    }

    pub fn created_chat_wire(response: &RiseJSONValue, request_body: &RiseJSONValue) -> Option<RiseStoredChat> {
        if let Ok(result) = response.decode_as::<CreateChatResultDTO>()
            && let Some(full_chat) = result.full_chat()
        {
            return Some(full_chat);
        }

        let request = request_body.object_value()?;
        let empty = BTreeMap::new();
        let response_object = response.object_value().unwrap_or(&empty);
        let nested = response_object
            .get(rise_key::tag::CHAT)
            .and_then(RiseJSONValue::object_value)
            .or_else(|| response_object.get(rise_key::events::DATA).and_then(RiseJSONValue::object_value))
            .unwrap_or(response_object);
        let chat_id = nested
            .get(rise_key::tag::ID)
            .and_then(RiseJSONValue::string_value)
            .or_else(|| nested.get(rise_key::tag::CHAT_ID).and_then(RiseJSONValue::string_value))
            .or_else(|| response_object.get(rise_key::tag::CHAT_ID).and_then(RiseJSONValue::string_value))
            .or_else(|| response_object.get(rise_key::tag::ID).and_then(RiseJSONValue::string_value))
            .filter(|chat_id| !chat_id.is_empty())?;

        let mut object = request.clone();
        for (key, value) in nested {
            object.insert(key.clone(), value.clone());
        }
        object.insert(rise_key::tag::ID.to_owned(), RiseJSONValue::String(chat_id.to_owned()));
        object.insert("server_id".to_owned(), RiseJSONValue::String(chat_id.to_owned()));
        object.insert("is_temp".to_owned(), RiseJSONValue::Bool(false));
        object.entry(rise_key::tag::ROLE.to_owned()).or_insert_with(|| RiseJSONValue::String("OWNER".to_owned()));
        if !object.contains_key(rise_key::tag::MEMBER_COUNT)
            && let Some(RiseJSONValue::Array(participants)) = request.get(rise_key::chat::PARTICIPANTS)
        {
            let member_count = i64::try_from(participants.len()).unwrap_or(i64::MAX).max(1);
            object.insert(rise_key::tag::MEMBER_COUNT.to_owned(), RiseJSONValue::Integer(member_count));
        }
        RiseChatWireCodec::decode_wire(object).ok()
    }

    fn request_body(value: &Value) -> anyhow::Result<RiseJSONValue> {
        let body = RiseConversationCompatibilityJSON::value(value)?;
        if body.object_value().is_none() {
            return Err(RiseConversationCompatibilityBridgeError::InvalidRequestBody.into());
        }
        Ok(body)
    }

    fn is_publication_current(domain: &Arc<RiseConversationDomain>, options: &SaiWsRequestOptions, cx: &mut App) -> bool {
        RiseConversationDomainRegistry::shared(cx).read(cx).active().is_some_and(|active| Arc::ptr_eq(&active, domain))
            && SaiAccountCacheScope::is_current_account_epoch(
                options.account_user_id.as_deref(),
                options.account_epoch.as_deref(),
            )
    }

    async fn validate_publication(
        domain: &Arc<RiseConversationDomain>,
        options: &SaiWsRequestOptions,
        receipt: Option<&RiseConversationCompatibilityRequestReceipt>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        if !cx.update(|cx| Self::is_publication_current(domain, options, cx)) {
            return Err(RiseConversationStateError::StaleAccount.into());
        }
        if let Some(receipt) = receipt
            && !domain.auxiliary.is_compatibility_result_current(receipt).await
        {
            return Err(RiseConversationAuxiliaryRepositoryError::StaleCompatibilityResult.into());
        }
        if !cx.update(|cx| Self::is_publication_current(domain, options, cx)) {
            return Err(RiseConversationStateError::StaleAccount.into());
        }
        Ok(())
    }
}
