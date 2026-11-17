use std::collections::HashMap;
use std::future::Future;
use std::hash::{BuildHasher, RandomState};
use std::sync::{Arc, OnceLock};

use futures::FutureExt as _;
use gpui::{App, AsyncApp, Global};
use parking_lot::Mutex;
use rise_canon::rise_key::map;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_core::rise_identifiers::RiseRequestID;
use rise_engine::rise_wire::rise_wire_contracts::RiseWireMethodDescriptor;
use rise_engine::{
    CancellationError, RiseEngineError, RiseJSONValue, RiseWireContractError, RiseWireError, RiseWireRemoteError,
};
use serde_json::{Map, Value};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_entry::{
    SaiHttpTraceEntry, SaiTraceTag, SaiTraceTransport,
};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::SaiHttpTraceStore;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsRequestOptions;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationCacheWriteSource, RiseConversationResolutionSource,
};
use crate::modules::chat::engine::core::rise_conversation_resource_repository::RiseConversationResourceReadSource;
use crate::modules::chat::engine::core::rise_conversation_rpc::{
    RiseConversationEndpoint, RiseConversationEndpointKind, RiseConversationEndpoints,
};
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;

const TRACING_ENABLED: bool = cfg!(any(debug_assertions, feature = "sai_debug_tools"));
const MAXIMUM_TRACKED_SCOPES: usize = 64;
const TRACE_ID_LENGTH: usize = 8;
const MILLISECONDS_PER_SECOND: f64 = 1_000.0;

/// debug only tracing of conversation work
/// a flow is one user request, nested rpc calls update it not add rows
/// payloads omitted, message text redacted
pub enum RiseConversationDebugTrace {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseConversationDebugTraceRepositoryPolicy {
    CacheFirst,
    OptimisticMutation,
    DurableOutbox,
    DurableReplay,
}

impl RiseConversationDebugTraceRepositoryPolicy {
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::CacheFirst => "cache-first",
            Self::OptimisticMutation => "optimistic-mutation",
            Self::DurableOutbox => "durable-outbox",
            Self::DurableReplay => "durable-replay",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RiseConversationDebugTraceContextNetworkAttempt {
    pub route: String,
    pub started_at: Date,
    pub finished_at: Option<Date>,
    pub error: Option<String>,
    pub response_preview: Option<String>,
}

struct RiseConversationDebugTraceContextState {
    network_attempts: HashMap<Uuid, RiseConversationDebugTraceContextNetworkAttempt>,
    latest_cache_write: RiseConversationCacheWriteSource,
    recovered_lifecycle_gap: bool,
}

pub struct RiseConversationDebugTraceContext {
    state: Mutex<RiseConversationDebugTraceContextState>,
}

impl RiseConversationDebugTraceContext {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RiseConversationDebugTraceContextState {
                network_attempts: HashMap::new(),
                latest_cache_write: RiseConversationCacheWriteSource::None,
                recovered_lifecycle_gap: false,
            }),
        })
    }

    pub fn begin_network(&self, route: &str, date: Date) -> Uuid {
        let id = Uuid::new_v4();
        self.state.lock().network_attempts.insert(
            id,
            RiseConversationDebugTraceContextNetworkAttempt {
                route: route.to_owned(),
                started_at: date,
                finished_at: None,
                error: None,
                response_preview: None,
            },
        );
        id
    }

    pub fn finish_network(&self, id: Uuid, date: Date, error: Option<String>, response_preview: Option<String>) {
        if let Some(attempt) = self.state.lock().network_attempts.get_mut(&id) {
            attempt.finished_at = Some(date);
            attempt.error = error;
            attempt.response_preview = response_preview;
        }
    }

    pub fn snapshot(&self) -> Vec<RiseConversationDebugTraceContextNetworkAttempt> {
        let mut value: Vec<_> = self.state.lock().network_attempts.values().cloned().collect();
        value.sort_by_key(|attempt| attempt.started_at);
        value
    }

    pub fn note_cache_write(&self, source: RiseConversationCacheWriteSource) {
        if source == RiseConversationCacheWriteSource::None {
            return;
        }
        self.state.lock().latest_cache_write = source;
    }

    pub fn cache_write_snapshot(&self) -> RiseConversationCacheWriteSource {
        self.state.lock().latest_cache_write
    }

    pub fn note_lifecycle_gap(&self) {
        self.state.lock().recovered_lifecycle_gap = true;
    }

    pub fn has_lifecycle_gap(&self) -> bool {
        self.state.lock().recovered_lifecycle_gap
    }
}

#[derive(Clone)]
pub struct RiseConversationDebugTraceFlowToken {
    pub id: Uuid,
    pub created_at: Date,
    pub route: String,
    pub context: Arc<RiseConversationDebugTraceContext>,
    pub initial_cache_lookup: Option<RiseConversationResourceReadSource>,
}

#[derive(Clone)]
enum RiseConversationDebugTraceNetworkSpanStorage {
    Scoped { context: Arc<RiseConversationDebugTraceContext>, id: Uuid },
    Standalone(RiseConversationDebugTraceFlowToken, Uuid),
    Suppressed,
}

#[derive(Clone)]
pub struct RiseConversationDebugTraceNetworkSpan {
    storage: RiseConversationDebugTraceNetworkSpanStorage,
}

impl RiseConversationDebugTraceNetworkSpan {
    fn suppressed() -> Self {
        Self { storage: RiseConversationDebugTraceNetworkSpanStorage::Suppressed }
    }
}

#[derive(Clone)]
enum RiseConversationDebugTraceScope {
    Flow(Arc<RiseConversationDebugTraceContext>),
    Suppressed,
}

tokio::task_local! {
    static SCOPE: RiseConversationDebugTraceScope;
}

impl RiseConversationDebugTrace {
    pub fn begin_flow(
        body: &RiseJSONValue,
        options: &SaiWsRequestOptions,
        repository_owned: bool,
        cx: &mut App,
    ) -> Option<RiseConversationDebugTraceFlowToken> {
        if !TRACING_ENABLED || options.exclude_from_debugger {
            return None;
        }

        let created_at = Date::now();
        let route = format!("{}.{}", options.service, options.method);
        let context = RiseConversationDebugTraceContext::new();
        let id = Uuid::new_v4();
        let cache_key = options.cache_key.as_deref().filter(|cache_key| !cache_key.is_empty());
        let presentation_cache = match cache_key {
            Some(cache_key) => SaiWsResponseCache::debug_presentation_source(cache_key, cx),
            None => "not-keyed".to_owned(),
        };
        let pagination = match &options.pagination {
            None => "none",
            Some(pagination) if pagination.is_initial_page() => "initial",
            Some(_) => "continuation",
        };
        let marks_pending_ui = options.reflect_pending_in_ui
            && options.pagination.as_ref().map(|pagination| pagination.is_initial_page()) != Some(false)
            && options.sai_slice.is_some();
        let mut headers = Self::headers(&[
            ("x-rise-owner", if repository_owned { "normalized-repository" } else { "auxiliary-resource" }),
            ("x-rise-policy", if options.reload_if_cached { "reload" } else { "cache-first" }),
            ("x-rise-presentation-cache", presentation_cache.as_str()),
            ("x-rise-pending-ui", if marks_pending_ui { "state-marked" } else { "not-marked" }),
            ("x-rise-storage-policy", options.disk_cache_policy.raw_value()),
            ("x-rise-store-response", if options.store_response_in_cache { "true" } else { "false" }),
            ("x-rise-pagination", pagination),
        ]);
        if let Some(cache_key) = cache_key {
            headers.insert("x-rise-cache-key".to_owned(), Self::cache_key_fingerprint(cache_key));
        }

        let mut tags = vec![SaiTraceTag::RISE_ENGINE.to_owned()];
        if options.is_screen_refresh_wire {
            tags.extend(SaiTraceTag::screen_refresh_wire());
        }
        Self::upsert(
            SaiHttpTraceEntry {
                id,
                transport: SaiTraceTransport::Websocket,
                created_at,
                is_pending: true,
                method: Self::operation_label(&options.service, &options.method),
                url: "RiseEngine".to_owned(),
                fetch_id: Some(route.clone()),
                duration_ms: 0,
                request_headers: headers,
                request_body_wire: Self::request_summary(&options.service, &options.method, body),
                response_status: 0,
                response_headers: HashMap::new(),
                response_body_preview: String::new(),
                transport_error: None,
                trace_tags: tags,
            },
            cx,
        );
        Some(RiseConversationDebugTraceFlowToken { id, created_at, route, context, initial_cache_lookup: None })
    }

    pub async fn with_flow<T>(
        token: Option<&RiseConversationDebugTraceFlowToken>,
        operation: impl Future<Output = T>,
    ) -> T {
        Self::with_scope(token, operation).await
    }

    pub async fn begin_repository_flow(
        endpoint: &RiseConversationEndpoint,
        policy: RiseConversationDebugTraceRepositoryPolicy,
        has_presentation_cache: bool,
        request_body_fields: &[String],
        scope_key: Option<&str>,
    ) -> Option<RiseConversationDebugTraceFlowToken> {
        if !TRACING_ENABLED {
            return None;
        }
        let created_at = Date::now();
        let route = format!("{}.{}", endpoint.service, endpoint.method);
        let token = RiseConversationDebugTraceFlowToken {
            id: Uuid::new_v4(),
            created_at,
            route: route.clone(),
            context: RiseConversationDebugTraceContext::new(),
            initial_cache_lookup: Some(if has_presentation_cache {
                RiseConversationResourceReadSource::PersistentStore
            } else {
                RiseConversationResourceReadSource::Miss
            }),
        };
        let mut request_headers = Self::headers(&[
            ("x-rise-owner", "normalized-repository"),
            ("x-rise-policy", policy.raw_value()),
            ("x-rise-presentation-cache", if has_presentation_cache { "normalized-store" } else { "miss" }),
            ("x-rise-pending-ui", "not-marked"),
            ("x-rise-storage-policy", "encrypted-rise-store"),
            ("x-rise-store-response", "true"),
        ]);
        if let Some(scope_key) = scope_key {
            request_headers.insert("x-rise-scope".to_owned(), scope_key.to_owned());
        }
        let mut sorted_fields = request_body_fields.to_vec();
        sorted_fields.sort();
        let mut request_body = vec![
            ("service", Value::from(endpoint.service.as_str())),
            ("method", Value::from(endpoint.method.as_str())),
            ("body_fields", Value::from(sorted_fields)),
            ("payload", Value::from("omitted")),
        ];
        if let Some(scope_key) = scope_key {
            request_body.push(("scope", Value::from(scope_key)));
        }
        let entry = SaiHttpTraceEntry {
            id: token.id,
            transport: SaiTraceTransport::Websocket,
            created_at,
            is_pending: true,
            method: Self::operation_label(&endpoint.service, &endpoint.method),
            url: "RiseEngine · normalized repository".to_owned(),
            fetch_id: Some(route),
            duration_ms: 0,
            request_headers,
            request_body_wire: Self::json_string(Self::object(request_body)),
            response_status: 0,
            response_headers: HashMap::new(),
            response_body_preview: String::new(),
            transport_error: None,
            trace_tags: vec![SaiTraceTag::RISE_ENGINE.to_owned()],
        };
        Self::on_main(move |cx| Self::upsert(entry, cx)).await;
        Some(token)
    }

    pub async fn with_repository_flow<T>(
        token: Option<&RiseConversationDebugTraceFlowToken>,
        operation: impl Future<Output = T>,
    ) -> T {
        Self::with_scope(token, operation).await
    }

    pub async fn begin_durable_replay_flow(
        endpoint: &RiseConversationEndpoint,
        request_id: RiseRequestID,
    ) -> Option<RiseConversationDebugTraceFlowToken> {
        if !TRACING_ENABLED {
            return None;
        }
        let created_at = Date::now();
        let route = format!("{}.{}", endpoint.service, endpoint.method);
        let token = RiseConversationDebugTraceFlowToken {
            id: Uuid::new_v4(),
            created_at,
            route: route.clone(),
            context: RiseConversationDebugTraceContext::new(),
            initial_cache_lookup: None,
        };
        let entry = SaiHttpTraceEntry {
            id: token.id,
            transport: SaiTraceTransport::Websocket,
            created_at,
            is_pending: true,
            method: Self::operation_label(&endpoint.service, &endpoint.method),
            url: "RiseEngine · durable replay".to_owned(),
            fetch_id: Some(route),
            duration_ms: 0,
            request_headers: Self::headers(&[
                ("x-rise-owner", "rise-sync"),
                ("x-rise-policy", RiseConversationDebugTraceRepositoryPolicy::DurableReplay.raw_value()),
                ("x-rise-presentation-cache", "not-applicable"),
                ("x-rise-pending-ui", "not-marked"),
                ("x-rise-storage-policy", "encrypted-rise-store"),
                ("x-rise-request-id", Self::cache_key_fingerprint(&request_id.raw_value().to_string()).as_str()),
            ]),
            request_body_wire: Self::json_string(Self::object(vec![
                ("service", Value::from(endpoint.service.as_str())),
                ("method", Value::from(endpoint.method.as_str())),
                (map::REQUEST_ID, Value::from("redacted")),
                ("payload", Value::from("omitted")),
            ])),
            response_status: 0,
            response_headers: HashMap::new(),
            response_body_preview: String::new(),
            transport_error: None,
            trace_tags: vec![SaiTraceTag::RISE_ENGINE.to_owned()],
        };
        Self::on_main(move |cx| Self::upsert(entry, cx)).await;
        Some(token)
    }

    pub fn begin_network_for(
        token: Option<&RiseConversationDebugTraceFlowToken>,
        endpoint: &RiseConversationEndpoint,
    ) -> RiseConversationDebugTraceNetworkSpan {
        let Some(token) = token.filter(|_| TRACING_ENABLED) else {
            return RiseConversationDebugTraceNetworkSpan::suppressed();
        };
        let started_at = Date::now();
        let route = format!("{}.{}", endpoint.service, endpoint.method);
        RiseConversationDebugTraceNetworkSpan {
            storage: RiseConversationDebugTraceNetworkSpanStorage::Scoped {
                context: token.context.clone(),
                id: token.context.begin_network(&route, started_at),
            },
        }
    }

    pub async fn complete_repository_flow(
        token: Option<&RiseConversationDebugTraceFlowToken>,
        resolution: Option<RiseConversationResolutionSource>,
        cache_write: RiseConversationCacheWriteSource,
        coalesced: bool,
        stale: bool,
        published_to_ui: bool,
        error_message: Option<&str>,
    ) {
        if !TRACING_ENABLED {
            return;
        }
        let Some(token) = token else {
            return;
        };
        let id = token.id;
        let Some(previous) = Self::on_main(move |cx| Self::trace_entry(id, cx)).await.flatten() else {
            return;
        };
        let completed = Self::completed_entry(
            &previous,
            token,
            token.initial_cache_lookup,
            resolution,
            cache_write,
            coalesced,
            stale,
            published_to_ui,
            error_message,
        );
        Self::on_main(move |cx| Self::upsert(completed, cx)).await;
    }

    pub async fn begin_network_endpoint(
        endpoint: &RiseConversationEndpoint,
        request_body_fields: &[String],
    ) -> RiseConversationDebugTraceNetworkSpan {
        if !TRACING_ENABLED {
            return RiseConversationDebugTraceNetworkSpan::suppressed();
        }
        Self::begin_network(&endpoint.service, &endpoint.method, request_body_fields).await
    }

    pub async fn begin_network_descriptor(
        descriptor: &RiseWireMethodDescriptor,
        request_body_fields: &[String],
    ) -> RiseConversationDebugTraceNetworkSpan {
        if !TRACING_ENABLED {
            return RiseConversationDebugTraceNetworkSpan::suppressed();
        }
        Self::begin_network(&descriptor.service, &descriptor.method, request_body_fields).await
    }

    pub async fn finish_network(
        span: RiseConversationDebugTraceNetworkSpan,
        response_preview: Option<String>,
        error_message: Option<String>,
    ) {
        if !TRACING_ENABLED {
            return;
        }
        let finished_at = Date::now();
        match span.storage {
            RiseConversationDebugTraceNetworkSpanStorage::Scoped { context, id } => {
                context.finish_network(id, finished_at, error_message, response_preview);
            }
            RiseConversationDebugTraceNetworkSpanStorage::Standalone(token, id) => {
                token.context.finish_network(id, finished_at, error_message.clone(), response_preview);
                Self::on_main(move |cx| {
                    Self::complete_flow(
                        Some(&token),
                        None,
                        error_message.is_none().then_some(RiseConversationResolutionSource::Network),
                        RiseConversationCacheWriteSource::None,
                        false,
                        false,
                        false,
                        error_message.as_deref(),
                        cx,
                    );
                })
                .await;
            }
            RiseConversationDebugTraceNetworkSpanStorage::Suppressed => {}
        }
    }

    pub fn note_cache_write(source: RiseConversationCacheWriteSource) {
        if !TRACING_ENABLED {
            return;
        }
        if let Some(RiseConversationDebugTraceScope::Flow(context)) = Self::scope() {
            context.note_cache_write(source);
        }
    }

    pub fn note_lifecycle_gap(token: Option<&RiseConversationDebugTraceFlowToken>) {
        if !TRACING_ENABLED {
            return;
        }
        if let Some(token) = token {
            token.context.note_lifecycle_gap();
        }
    }

    pub async fn record_resource_cache_hit(
        service: &str,
        method: &str,
        source: RiseConversationResourceReadSource,
    ) {
        if !TRACING_ENABLED || source == RiseConversationResourceReadSource::Miss {
            return;
        }
        let created_at = Date::now();
        let route = format!("{service}.{method}");
        let is_memory = source == RiseConversationResourceReadSource::Memory;
        let resolution = if is_memory {
            RiseConversationResolutionSource::ResourceMemory
        } else {
            RiseConversationResolutionSource::PersistentStore
        };
        let served_from = if is_memory { "memory" } else { "disk" };
        let presentation_cache = if is_memory { "memory-hit" } else { "persistent-store-hit" };
        let entry = SaiHttpTraceEntry {
            id: Uuid::new_v4(),
            transport: SaiTraceTransport::Websocket,
            created_at,
            is_pending: false,
            method: Self::operation_label(service, method),
            url: "RiseEngine · resource cache".to_owned(),
            fetch_id: Some(route.clone()),
            duration_ms: 0,
            request_headers: Self::headers(&[
                ("x-rise-owner", "resource-repository"),
                ("x-rise-policy", "cache-first"),
                ("x-rise-presentation-cache", presentation_cache),
                ("x-rise-pending-ui", "not-marked"),
            ]),
            request_body_wire: Self::json_string(Self::object(vec![
                ("service", Value::from(service)),
                ("method", Value::from(method)),
                ("payload", Value::from("omitted")),
            ])),
            response_status: 200,
            response_headers: Self::headers(&[
                ("x-rise-cache-lookup", source.raw_value()),
                ("x-rise-source", resolution.raw_value()),
                ("x-rise-coalesced", "false"),
                ("x-rise-cache-write", "none"),
                ("x-rise-rpc-attempts", "0"),
                ("x-rise-rpc-failures", "0"),
                ("x-rise-ui-result", "not-published"),
                ("x-sai-served-from", served_from),
                ("x-sai-resolve", format!("rise-{}", resolution.raw_value()).as_str()),
            ]),
            response_body_preview: Self::json_string(Self::object(vec![
                ("route", Value::from(route)),
                ("source", Value::from(resolution.raw_value())),
                (
                    "stages",
                    Value::Array(vec![
                        Self::stage("resource-cache-lookup", source.raw_value()),
                        Self::stage("presentation", "not-published"),
                    ]),
                ),
                ("payload", Value::from("omitted")),
            ])),
            transport_error: None,
            trace_tags: vec![SaiTraceTag::RISE_ENGINE.to_owned()],
        };
        Self::on_main(move |cx| Self::upsert(entry, cx)).await;
    }

    /// concrete engine error type name, never the error text since it can carry private content
    pub fn error_label(error: &anyhow::Error) -> String {
        if !TRACING_ENABLED {
            return String::new();
        }
        let label = if error.is::<CancellationError>() {
            "CancellationError"
        } else if error.is::<RiseWireRemoteError>() {
            "RiseWireRemoteError"
        } else if error.is::<RiseWireError>() {
            "RiseWireError"
        } else if error.is::<RiseWireContractError>() {
            "RiseWireContractError"
        } else if error.is::<RiseEngineError>() {
            "RiseEngineError"
        } else {
            "Error"
        };
        label.to_owned()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_flow(
        token: Option<&RiseConversationDebugTraceFlowToken>,
        cache_lookup: Option<RiseConversationResourceReadSource>,
        resolution: Option<RiseConversationResolutionSource>,
        cache_write: RiseConversationCacheWriteSource,
        coalesced: bool,
        stale: bool,
        published_to_ui: bool,
        error_message: Option<&str>,
        cx: &mut App,
    ) {
        if !TRACING_ENABLED {
            return;
        }
        let Some(token) = token else {
            return;
        };
        let Some(previous) = Self::trace_entry(token.id, cx) else {
            return;
        };
        let completed = Self::completed_entry(
            &previous,
            token,
            cache_lookup,
            resolution,
            cache_write,
            coalesced,
            stale,
            published_to_ui,
            error_message,
        );
        Self::upsert(completed, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn completed_entry(
        previous: &SaiHttpTraceEntry,
        token: &RiseConversationDebugTraceFlowToken,
        cache_lookup: Option<RiseConversationResourceReadSource>,
        resolution: Option<RiseConversationResolutionSource>,
        cache_write: RiseConversationCacheWriteSource,
        coalesced: bool,
        stale: bool,
        published_to_ui: bool,
        error_message: Option<&str>,
    ) -> SaiHttpTraceEntry {
        let attempts = token.context.snapshot();
        let effective_resolution = resolution
            .unwrap_or_else(|| Self::inferred_resolution(&attempts, cache_lookup, stale, error_message));
        let effective_cache_write = if cache_write == RiseConversationCacheWriteSource::None {
            token.context.cache_write_snapshot()
        } else {
            cache_write
        };
        let source_label = if stale {
            "stale-dropped"
        } else if error_message.is_none() {
            effective_resolution.raw_value()
        } else {
            "failure"
        };
        let ui_result = if stale {
            "stale-dropped"
        } else if published_to_ui {
            "published"
        } else {
            "not-published"
        };
        let failures = attempts.iter().filter(|attempt| attempt.error.is_some()).count();
        let mut headers = Self::headers(&[
            ("x-rise-cache-lookup", cache_lookup.map_or("not-checked", |lookup| lookup.raw_value())),
            ("x-rise-source", source_label),
            ("x-rise-coalesced", if coalesced { "true" } else { "false" }),
            ("x-rise-cache-write", effective_cache_write.raw_value()),
            ("x-rise-rpc-attempts", attempts.len().to_string().as_str()),
            ("x-rise-rpc-failures", failures.to_string().as_str()),
            ("x-rise-ui-result", ui_result),
        ]);
        if token.context.has_lifecycle_gap() {
            headers.insert("x-rise-lifecycle-gap".to_owned(), "recovered".to_owned());
        }
        if attempts.iter().any(|attempt| attempt.response_preview.is_some()) {
            headers.insert("x-rise-backend-response".to_owned(), "captured-debug-projection".to_owned());
        }
        if let (Some(served_from), None) = (Self::served_from_header(effective_resolution), error_message) {
            headers.insert("x-sai-served-from".to_owned(), served_from.to_owned());
            headers.insert("x-sai-resolve".to_owned(), format!("rise-{}", effective_resolution.raw_value()));
        }

        let mut tags = previous.trace_tags.clone();
        if coalesced {
            tags.push(SaiTraceTag::COALESCED.to_owned());
        }
        if stale {
            tags.push(SaiTraceTag::STALE.to_owned());
        }
        if error_message.is_none() {
            match effective_resolution {
                RiseConversationResolutionSource::NetworkFallbackMemory
                | RiseConversationResolutionSource::NetworkFallbackPersistentStore => {
                    tags.push(SaiTraceTag::FALLBACK.to_owned());
                }
                RiseConversationResolutionSource::NormalizedStore => {
                    tags.push(SaiTraceTag::NORMALIZED_STORE.to_owned());
                }
                _ => {}
            }
        }

        let response_status = match (error_message, stale) {
            (None, true) => 204,
            (None, false) => 200,
            (Some(_), _) => 0,
        };
        previous.with_completed(
            Self::offset_milliseconds(Date::now(), token.created_at),
            response_status,
            headers,
            Self::response_summary(
                token,
                &attempts,
                cache_lookup,
                effective_resolution,
                effective_cache_write,
                coalesced,
                stale,
                published_to_ui,
                error_message,
            ),
            error_message.map(str::to_owned),
            Some(Self::unique_tags(tags)),
        )
    }

    async fn begin_network(
        service: &str,
        method: &str,
        request_body_fields: &[String],
    ) -> RiseConversationDebugTraceNetworkSpan {
        let route = format!("{service}.{method}");
        let started_at = Date::now();
        match Self::scope() {
            Some(RiseConversationDebugTraceScope::Flow(context)) => {
                let id = context.begin_network(&route, started_at);
                RiseConversationDebugTraceNetworkSpan {
                    storage: RiseConversationDebugTraceNetworkSpanStorage::Scoped { context, id },
                }
            }
            Some(RiseConversationDebugTraceScope::Suppressed) => RiseConversationDebugTraceNetworkSpan::suppressed(),
            None => {
                let service = service.to_owned();
                let method = method.to_owned();
                let request_body_fields = request_body_fields.to_vec();
                let token = Self::on_main(move |cx| {
                    Self::begin_standalone_repository_network_flow(
                        &service,
                        &method,
                        &request_body_fields,
                        started_at,
                        cx,
                    )
                })
                .await;
                let Some(token) = token else {
                    return RiseConversationDebugTraceNetworkSpan::suppressed();
                };
                let id = token.context.begin_network(&route, started_at);
                RiseConversationDebugTraceNetworkSpan {
                    storage: RiseConversationDebugTraceNetworkSpanStorage::Standalone(token, id),
                }
            }
        }
    }

    fn begin_standalone_repository_network_flow(
        service: &str,
        method: &str,
        request_body_fields: &[String],
        created_at: Date,
        cx: &mut App,
    ) -> RiseConversationDebugTraceFlowToken {
        let route = format!("{service}.{method}");
        let token = RiseConversationDebugTraceFlowToken {
            id: Uuid::new_v4(),
            created_at,
            route: route.clone(),
            context: RiseConversationDebugTraceContext::new(),
            initial_cache_lookup: None,
        };
        Self::upsert(
            SaiHttpTraceEntry {
                id: token.id,
                transport: SaiTraceTransport::Websocket,
                created_at,
                is_pending: true,
                method: Self::operation_label(service, method),
                url: "RiseEngine · repository RPC".to_owned(),
                fetch_id: Some(route),
                duration_ms: 0,
                request_headers: Self::headers(&[
                    ("x-rise-owner", "normalized-repository"),
                    ("x-rise-policy", "network"),
                    ("x-rise-presentation-cache", "not-applicable"),
                    ("x-rise-pending-ui", "not-applicable"),
                ]),
                request_body_wire: Self::json_string(Self::object(vec![
                    ("service", Value::from(service)),
                    ("method", Value::from(method)),
                    ("body_fields", Value::from(request_body_fields.to_vec())),
                    ("payload", Value::from("omitted")),
                ])),
                response_status: 0,
                response_headers: HashMap::new(),
                response_body_preview: String::new(),
                transport_error: None,
                trace_tags: vec![SaiTraceTag::RISE_ENGINE.to_owned()],
            },
            cx,
        );
        token
    }

    fn inferred_resolution(
        attempts: &[RiseConversationDebugTraceContextNetworkAttempt],
        cache_lookup: Option<RiseConversationResourceReadSource>,
        _stale: bool,
        error_message: Option<&str>,
    ) -> RiseConversationResolutionSource {
        if error_message.is_none() && attempts.iter().any(|attempt| attempt.error.is_none()) {
            return RiseConversationResolutionSource::Network;
        }
        if error_message.is_none() && attempts.iter().any(|attempt| attempt.error.is_some()) {
            match cache_lookup {
                Some(RiseConversationResourceReadSource::Memory) => {
                    return RiseConversationResolutionSource::NetworkFallbackMemory;
                }
                Some(RiseConversationResourceReadSource::PersistentStore) => {
                    return RiseConversationResolutionSource::NetworkFallbackPersistentStore;
                }
                Some(RiseConversationResourceReadSource::Miss) | None => {}
            }
        }
        match cache_lookup {
            Some(RiseConversationResourceReadSource::Memory) => RiseConversationResolutionSource::ResourceMemory,
            Some(RiseConversationResourceReadSource::PersistentStore) => {
                RiseConversationResolutionSource::PersistentStore
            }
            Some(RiseConversationResourceReadSource::Miss) | None => RiseConversationResolutionSource::NormalizedStore,
        }
    }

    fn served_from_header(resolution: RiseConversationResolutionSource) -> Option<&'static str> {
        match resolution {
            RiseConversationResolutionSource::ResourceMemory
            | RiseConversationResolutionSource::NetworkFallbackMemory => Some("memory"),
            RiseConversationResolutionSource::PersistentStore
            | RiseConversationResolutionSource::NetworkFallbackPersistentStore => Some("disk"),
            RiseConversationResolutionSource::Network | RiseConversationResolutionSource::NormalizedStore => None,
        }
    }

    fn operation_label(service: &str, method: &str) -> String {
        let Some(endpoint) = RiseConversationEndpoints::endpoint(service, method) else {
            return "RISE".to_owned();
        };
        let label = if matches!(endpoint.kind, RiseConversationEndpointKind::Read) { "READ" } else { "WRITE" };
        label.to_owned()
    }

    fn request_summary(service: &str, method: &str, body: &RiseJSONValue) -> String {
        let body_fields: Vec<String> =
            body.object_value().map(|object| object.keys().cloned().collect()).unwrap_or_default();
        Self::json_string(Self::object(vec![
            ("service", Value::from(service)),
            ("method", Value::from(method)),
            ("body_fields", Value::from(body_fields)),
            ("payload", Value::from("omitted")),
        ]))
    }

    #[allow(clippy::too_many_arguments)]
    fn response_summary(
        token: &RiseConversationDebugTraceFlowToken,
        attempts: &[RiseConversationDebugTraceContextNetworkAttempt],
        cache_lookup: Option<RiseConversationResourceReadSource>,
        resolution: RiseConversationResolutionSource,
        cache_write: RiseConversationCacheWriteSource,
        coalesced: bool,
        stale: bool,
        published_to_ui: bool,
        error_message: Option<&str>,
    ) -> String {
        let mut stages =
            vec![Self::stage("resource-cache-lookup", cache_lookup.map_or("not-checked", |lookup| lookup.raw_value()))];
        for attempt in attempts {
            let mut stage = vec![
                ("stage", Value::from("rpc")),
                ("route", Value::from(attempt.route.as_str())),
                ("started_ms", Value::from(Self::offset_milliseconds(attempt.started_at, token.created_at))),
            ];
            if let Some(finished_at) = attempt.finished_at {
                stage.push((map::DURATION_MS, Value::from(Self::offset_milliseconds(finished_at, attempt.started_at))));
            }
            stage.push(("result", Value::from(if attempt.error.is_none() { "success" } else { "failure" })));
            if let Some(error) = &attempt.error {
                stage.push(("error", Value::from(error.as_str())));
            }
            stages.push(Self::object(stage));
        }
        if coalesced {
            stages.push(Self::stage("single-flight", "joined"));
        }
        if token.context.has_lifecycle_gap() {
            stages.push(Self::stage("lifecycle-events", "recovered-after-gap"));
        }
        stages.push(Self::stage("resolution", if error_message.is_none() { resolution.raw_value() } else { "failure" }));
        if cache_write != RiseConversationCacheWriteSource::None {
            stages.push(Self::stage("cache-write", cache_write.raw_value()));
        }
        let presentation = if stale {
            "stale-dropped"
        } else if published_to_ui {
            "published"
        } else {
            "not-published"
        };
        stages.push(Self::stage("presentation", presentation));

        let source = if stale {
            "stale-dropped"
        } else if error_message.is_none() {
            resolution.raw_value()
        } else {
            "failure"
        };
        let mut root = vec![
            ("route", Value::from(token.route.as_str())),
            ("source", Value::from(source)),
            ("stages", Value::Array(stages)),
            ("payload", Value::from("omitted")),
        ];
        let backend_response = attempts
            .iter()
            .rev()
            .find_map(|attempt| attempt.response_preview.as_deref())
            .and_then(|preview| serde_json::from_str::<Value>(preview).ok());
        if let Some(backend_response) = backend_response {
            root.push(("backend_response", backend_response));
        }
        if let Some(error_message) = error_message {
            root.push(("error", Value::from(error_message)));
        }
        Self::json_string(Self::object(root))
    }

    fn offset_milliseconds(date: Date, start: Date) -> i64 {
        ((date.time_interval_since(&start) * MILLISECONDS_PER_SECOND) as i64).max(0)
    }

    fn unique_tags(tags: Vec<String>) -> Vec<String> {
        let mut result: Vec<String> = Vec::with_capacity(tags.len());
        for tag in tags {
            if !result.contains(&tag) {
                result.push(tag);
            }
        }
        result
    }

    /// hash seeded per process so a fingerprint means nothing across launches
    fn cache_key_fingerprint(cache_key: &str) -> String {
        static SEED: OnceLock<RandomState> = OnceLock::new();
        let hash = SEED.get_or_init(RandomState::new).hash_one(cache_key);
        format!("cache#{hash:x}")
    }

    fn json_string(object: Value) -> String {
        serde_json::to_string_pretty(&Self::sorted_keys(object)).unwrap_or_else(|_| "{}".to_owned())
    }

    fn sorted_keys(value: Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut entries: Vec<(String, Value)> =
                    object.into_iter().map(|(key, field)| (key, Self::sorted_keys(field))).collect();
                entries.sort_by(|left, right| left.0.cmp(&right.0));
                Value::Object(entries.into_iter().collect())
            }
            Value::Array(items) => Value::Array(items.into_iter().map(Self::sorted_keys).collect()),
            other => other,
        }
    }

    fn object(fields: Vec<(&str, Value)>) -> Value {
        let mut object = Map::with_capacity(fields.len());
        for (key, field) in fields {
            object.insert(key.to_owned(), field);
        }
        Value::Object(object)
    }

    fn stage(stage: &str, result: &str) -> Value {
        Self::object(vec![("stage", Value::from(stage)), ("result", Value::from(result))])
    }

    fn headers(fields: &[(&str, &str)]) -> HashMap<String, String> {
        fields.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect()
    }

    fn scope() -> Option<RiseConversationDebugTraceScope> {
        SCOPE.try_with(RiseConversationDebugTraceScope::clone).ok()
    }

    async fn with_scope<T>(
        token: Option<&RiseConversationDebugTraceFlowToken>,
        operation: impl Future<Output = T>,
    ) -> T {
        if !TRACING_ENABLED {
            return operation.await;
        }
        let installed_scope = match token {
            Some(token) => RiseConversationDebugTraceScope::Flow(token.context.clone()),
            None => RiseConversationDebugTraceScope::Suppressed,
        };
        SCOPE.scope(installed_scope, operation).await
    }

    fn upsert(entry: SaiHttpTraceEntry, cx: &mut App) {
        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| store.upsert(entry, cx));
    }

    fn trace_entry(id: Uuid, cx: &mut App) -> Option<SaiHttpTraceEntry> {
        SaiHttpTraceStore::shared(cx).read(cx).entries().iter().find(|entry| entry.id == id).cloned()
    }

    /// runs work on the main thread, none only before the hop starts or after the app is gone
    async fn on_main<R: Send + 'static>(work: impl FnOnce(&mut App) -> R + Send + 'static) -> Option<R> {
        let hop = RiseEngineMainHop::installed()?;
        hop.run(move |cx: AsyncApp| async move { Ok(cx.update(work)) }.boxed_local()).await.ok()
    }
}

pub enum RiseMessageRuntimeTrace {}

#[derive(Default)]
struct RiseMessageRuntimeTraceState {
    latest_trace_ids: HashMap<String, String>,
    trace_order: Vec<String>,
    last_signatures: HashMap<String, String>,
    signature_order: Vec<String>,
}

impl Global for RiseMessageRuntimeTraceState {}

impl RiseMessageRuntimeTrace {
    pub fn make_id() -> Option<String> {
        if !TRACING_ENABLED {
            return None;
        }
        Some(Uuid::new_v4().uuid_string().chars().take(TRACE_ID_LENGTH).collect())
    }

    pub async fn record(stage: &str, trace_id: Option<&str>, scope: &RiseMessageScope, detail: &str) {
        if !TRACING_ENABLED {
            return;
        }
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let stage = stage.to_owned();
        let trace_id = trace_id.map(str::to_owned);
        let scope_key = scope.storage_key();
        let detail = detail.to_owned();
        hop.post(move |cx| {
            Self::record_on_main(&stage, trace_id.as_deref(), &scope_key, &detail, None, cx);
        });
    }

    pub fn record_on_main(
        stage: &str,
        trace_id: Option<&str>,
        scope: &str,
        detail: &str,
        signature: Option<&str>,
        cx: &mut App,
    ) {
        if !TRACING_ENABLED {
            return;
        }
        let state = cx.default_global::<RiseMessageRuntimeTraceState>();
        if let Some(trace_id) = trace_id {
            if !state.latest_trace_ids.contains_key(scope) {
                if state.latest_trace_ids.len() >= MAXIMUM_TRACKED_SCOPES && !state.trace_order.is_empty() {
                    let oldest = state.trace_order.remove(0);
                    state.latest_trace_ids.remove(&oldest);
                }
                state.trace_order.push(scope.to_owned());
            }
            state.latest_trace_ids.insert(scope.to_owned(), trace_id.to_owned());
        }
        if let Some(signature) = signature {
            let key = format!("{scope}|{stage}");
            match state.last_signatures.get(&key) {
                Some(last) if last == signature => return,
                Some(_) => {}
                None => {
                    if state.last_signatures.len() >= MAXIMUM_TRACKED_SCOPES && !state.signature_order.is_empty() {
                        let oldest = state.signature_order.remove(0);
                        state.last_signatures.remove(&oldest);
                    }
                    state.signature_order.push(key.clone());
                }
            }
            state.last_signatures.insert(key, signature.to_owned());
        }
        let trace = trace_id
            .or_else(|| state.latest_trace_ids.get(scope).map(String::as_str))
            .unwrap_or("none")
            .to_owned();
        let line = format!("trace={trace} scope={scope} stage={stage} {detail}");
        Sai::log("MessagePipeline", move || line, SaiLogKind::Network, cx);
    }
}
