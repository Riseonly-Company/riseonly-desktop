use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use gpui::App;
use rise_canon::rise_key::{chat, stickers};
use rise_canon::rise_wire_canon::key;
use rise_core::foundation::foundation_crypto::{Sha256, hex_string};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{character_count, character_prefix};
use rise_engine::{
    CancellationError, RiseEngineCallContext, RiseEngineCallObserver, RiseEngineError,
    RiseEngineObservability, RiseWireContractError, RiseWireError, RiseWireRemoteError,
    RiseWireReplayPolicy,
};
use serde_json::{Map, Value};

use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use crate::core::lib::canon_failure_bridge::CanonFailure;
use crate::core::lib::canon_failure_relay::RiseServerRefusalRelay;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_entry::{
    SaiHttpTraceEntry, SaiTraceTag, SaiTraceTransport,
};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::SaiHttpTraceStore;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;

const SUMMARY_FIELDS: [&str; 7] = [
    stickers::PACK_KIND,
    stickers::KIND,
    stickers::LIMIT,
    stickers::PAGE,
    chat::HAS_MEDIA,
    chat::HAS_LINK,
    chat::HAS_DOCUMENTS,
];

const SUMMARY_VALUE_MAXIMUM_CHARACTERS: usize = 32;
const FINGERPRINT_BYTES: usize = 12;
const TRANSPORT_ERROR_MAXIMUM_CHARACTERS: usize = 256;

/// feeds every engine rpc into the in app network debugger, one row per call
/// it becomes a network request only at the platform
/// send boundary, repo and cache spans stay diagnostic
pub struct RiseEngineTraceBridge {
    main: RiseEngineMainHop,
}

static SHARED: OnceLock<Arc<RiseEngineTraceBridge>> = OnceLock::new();

impl RiseEngineTraceBridge {
    pub const DEBUG_TOOLS: bool = cfg!(any(debug_assertions, feature = "sai_debug_tools"));

    pub fn shared(cx: &mut App) -> Arc<RiseEngineTraceBridge> {
        SHARED
            .get_or_init(|| Arc::new(RiseEngineTraceBridge { main: RiseEngineMainHop::shared(cx) }))
            .clone()
    }

    pub fn install(cx: &mut App) {
        let observer: Arc<dyn RiseEngineCallObserver> = Self::shared(cx);
        RiseEngineObservability::install(observer);
    }

    /// fingerprint of the body and not the request id or timestamp
    /// equal fingerprints mean equal parameters
    /// runs on the adapter worker never while a frame renders
    pub fn request_summary(frame: &[u8]) -> String {
        let Ok(Value::Object(root)) = serde_json::from_slice::<Value>(frame) else {
            return String::new();
        };
        let Some(body_value) = root.get(key::DATA) else {
            return String::new();
        };
        let Value::Object(body) = body_value else {
            return String::new();
        };
        let Ok(data) = serde_json::to_vec(&Self::sorted_keys(body_value)) else {
            return String::new();
        };
        let digest = Sha256::hash(&data);
        let fingerprint = hex_string(&digest[..FINGERPRINT_BYTES]);
        let mut fields = vec![format!("body_sha256={fingerprint}")];
        for field in SUMMARY_FIELDS {
            match body.get(field) {
                Some(Value::Number(value)) => fields.push(format!("{field}={value}")),
                Some(Value::Bool(value)) => fields.push(format!("{field}={}", u8::from(*value))),
                Some(Value::String(value))
                    if character_count(value) <= SUMMARY_VALUE_MAXIMUM_CHARACTERS
                        && value
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '-') =>
                {
                    fields.push(format!("{field}={value}"));
                }
                _ => {}
            }
        }
        fields.join(" ")
    }

    /// sorts keys at every depth whatever order serde_json built the map with
    fn sorted_keys(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut names: Vec<&String> = object.keys().collect();
                names.sort();
                let mut sorted = Map::with_capacity(object.len());
                for name in names {
                    if let Some(child) = object.get(name) {
                        sorted.insert(name.clone(), Self::sorted_keys(child));
                    }
                }
                Value::Object(sorted)
            }
            Value::Array(items) => Value::Array(items.iter().map(Self::sorted_keys).collect()),
            other => other.clone(),
        }
    }

    /// name of the concrete engine error behind the erased one
    fn error_type_name(error: &anyhow::Error) -> &'static str {
        if error.downcast_ref::<RiseWireRemoteError>().is_some() {
            "RiseWireRemoteError"
        } else if error.downcast_ref::<RiseWireError>().is_some() {
            "RiseWireError"
        } else if error.downcast_ref::<RiseWireContractError>().is_some() {
            "RiseWireContractError"
        } else if error.downcast_ref::<RiseEngineError>().is_some() {
            "RiseEngineError"
        } else if error.downcast_ref::<CancellationError>().is_some() {
            "CancellationError"
        } else {
            "Error"
        }
    }

    fn publish(
        &self,
        context: &RiseEngineCallContext,
        duration_milliseconds: i64,
        is_pending: bool,
        error: Option<&anyhow::Error>,
    ) {
        let replay = match &context.replay_policy {
            RiseWireReplayPolicy::ReadOnly => "read-only".to_owned(),
            RiseWireReplayPolicy::Never => "never".to_owned(),
            RiseWireReplayPolicy::RequestID => "request-id".to_owned(),
            RiseWireReplayPolicy::IdempotencyKey { field } => format!("idempotency:{field}"),
        };

        let attempt = context.transport_attempt();
        let source = match (&attempt, is_pending, error.is_none()) {
            (None, true, _) => "waiting-for-transport",
            (None, false, _) => "local-error",
            (Some(_), _, true) => "network",
            (Some(_), _, false) => "network-error",
        };
        let mut request_headers = HashMap::from([
            ("x-rise-policy".to_owned(), replay),
            ("x-rise-account".to_owned(), context.account_id.clone()),
        ]);
        if let Some(attempt) = &attempt {
            request_headers.insert(
                "x-rise-request-id".to_owned(),
                attempt.request_id.raw_value().to_string(),
            );
        }
        let attempts = if attempt.is_none() { "0" } else { "1" };
        let duration_ms = match &attempt {
            Some(_) if is_pending => 0,
            Some(attempt) => {
                let elapsed = RiseClock::now().time_interval_since(&attempt.started_at) * 1_000.0;
                (elapsed as i64).max(0)
            }
            None => duration_milliseconds,
        };
        let entry = SaiHttpTraceEntry {
            id: context.id,
            transport: SaiTraceTransport::Websocket,
            created_at: attempt.as_ref().map_or(context.started_at, |attempt| attempt.started_at),
            is_pending,
            method: "WS".to_owned(),
            url: format!("{}/{}", context.service, context.method),
            fetch_id: Some(format!("rise.{}.{}", context.service, context.method)),
            duration_ms,
            request_headers,
            request_body_wire: attempt
                .as_ref()
                .map(|attempt| attempt.request_summary.clone())
                .unwrap_or_default(),
            response_status: match (is_pending, error.is_none()) {
                (true, _) => 0,
                (false, true) => 200,
                (false, false) => 0,
            },
            response_headers: HashMap::from([
                ("x-rise-rpc-attempts".to_owned(), attempts.to_owned()),
                ("x-rise-wire-attempts".to_owned(), attempts.to_owned()),
                ("x-rise-source".to_owned(), source.to_owned()),
            ]),
            response_body_preview: String::new(),
            transport_error: error.map(|error| {
                character_prefix(&format!("{error:#}"), TRANSPORT_ERROR_MAXIMUM_CHARACTERS)
                    .to_owned()
            }),
            trace_tags: vec![SaiTraceTag::RISE_ENGINE.to_owned(), SaiTraceTag::DIRECT_RPC.to_owned()],
        };

        self.main.post(move |cx| {
            SaiHttpTraceStore::shared(cx).update(cx, |store, cx| store.upsert(entry, cx));
        });
    }
}

impl RiseEngineCallObserver for RiseEngineTraceBridge {
    fn engine_call_did_begin(&self, context: &RiseEngineCallContext) {
        if Self::DEBUG_TOOLS {
            self.publish(context, 0, true, None);
        }
    }

    fn engine_call_will_send(&self, context: &RiseEngineCallContext) {
        if !Self::DEBUG_TOOLS {
            return;
        }
        if let Some(attempt) = context.transport_attempt() {
            let line = format!(
                "send {}.{} id={} {}",
                context.service,
                context.method,
                attempt.request_id.raw_value(),
                attempt.request_summary
            );
            self.main.post(move |cx| Sai::log("RiseWire", || line, SaiLogKind::Network, cx));
        }
        self.publish(context, 0, true, None);
    }

    fn engine_call_did_finish(
        &self,
        context: &RiseEngineCallContext,
        duration_milliseconds: i64,
        error: Option<&anyhow::Error>,
    ) {
        if let Some(error) = error {
            let type_name = Self::error_type_name(error);
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Rpc, || {
                format!(
                    "{}.{} failed after {}ms: {}",
                    context.service, context.method, duration_milliseconds, type_name
                )
            });
            if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
                let failure = remote.canon_failure();
                self.main.post(move |cx| RiseServerRefusalRelay::publish(failure, cx));
            }
        }
        if !Self::DEBUG_TOOLS {
            return;
        }
        let attempt = context.transport_attempt();
        let request_id = attempt
            .as_ref()
            .map_or_else(|| "none".to_owned(), |attempt| attempt.request_id.raw_value().to_string());
        let phase = if attempt.is_none() { "local" } else { "wire" };
        let result = if error.is_none() { "success" } else { "failure" };
        let line = format!(
            "finish {}.{} id={} phase={} result={} duration_ms={}",
            context.service, context.method, request_id, phase, result, duration_milliseconds
        );
        self.main.post(move |cx| Sai::log("RiseWire", || line, SaiLogKind::Network, cx));
        self.publish(context, duration_milliseconds, false, error);
    }
}
