use std::collections::HashMap;

use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiTraceTransport {
    Http,
    Websocket,
}

impl SaiTraceTransport {
    pub const ALL: [Self; 2] = [Self::Http, Self::Websocket];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Http => "HTTP",
            Self::Websocket => "WS",
        }
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|transport| transport.raw_value() == raw_value)
    }
}

pub enum SaiTraceTag {}

impl SaiTraceTag {
    pub const SCREEN_REFRESH: &'static str = "ScreenRefresh";
    pub const SYSTEM: &'static str = "System";
    pub const RISE_ENGINE: &'static str = "RiseEngine";
    pub const COALESCED: &'static str = "Coalesced";
    pub const FALLBACK: &'static str = "Fallback";
    pub const STALE: &'static str = "Stale";
    pub const NORMALIZED_STORE: &'static str = "NormalizedStore";
    pub const DIRECT_RPC: &'static str = "DirectRPC";

    pub const SCREEN_REFRESH_WIRE: [&'static str; 2] = [Self::SCREEN_REFRESH, Self::SYSTEM];

    /// the screen refresh wire tags as a string list for trace tags
    pub fn screen_refresh_wire() -> Vec<String> {
        Self::SCREEN_REFRESH_WIRE
            .iter()
            .map(|tag| (*tag).to_owned())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiTraceCacheSource {
    Memory,
    Disk,
}

impl SaiTraceCacheSource {
    pub fn from_response_headers(headers: &HashMap<String, String>) -> Option<Self> {
        let served = headers
            .get("x-sai-served-from")
            .map(|value| value.to_lowercase())
            .unwrap_or_default();
        if served == "memory" {
            return Some(Self::Memory);
        }
        if served == "disk" {
            return Some(Self::Disk);
        }
        let resolve = headers
            .get("x-sai-resolve")
            .map(|value| value.to_lowercase())
            .unwrap_or_default();
        if (resolve.contains("cache-hit") || resolve.contains("disk-only")) && served.is_empty() {
            return Some(if resolve.contains("disk-only") {
                Self::Disk
            } else {
                Self::Memory
            });
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiHttpTraceEntry {
    pub id: Uuid,
    pub transport: SaiTraceTransport,
    pub created_at: Date,
    pub is_pending: bool,
    pub method: String,
    pub url: String,
    pub fetch_id: Option<String>,
    pub duration_ms: i64,
    pub request_headers: HashMap<String, String>,
    pub request_body_wire: String,
    pub response_status: i64,
    pub response_headers: HashMap<String, String>,
    pub response_body_preview: String,
    pub transport_error: Option<String>,
    pub trace_tags: Vec<String>,
}

impl SaiHttpTraceEntry {
    pub fn cache_source(&self) -> Option<SaiTraceCacheSource> {
        SaiTraceCacheSource::from_response_headers(&self.response_headers)
    }

    pub fn is_cache_hit(&self) -> bool {
        self.cache_source().is_some()
    }

    pub fn is_rise_engine(&self) -> bool {
        self.has_tag(SaiTraceTag::RISE_ENGINE)
    }

    pub fn is_network_request(&self) -> bool {
        if self.transport == SaiTraceTransport::Http {
            return !self.is_cache_hit();
        }
        if self.is_rise_engine() {
            return self.has_tag(SaiTraceTag::DIRECT_RPC)
                && self
                    .response_headers
                    .get("x-rise-wire-attempts")
                    .is_some_and(|attempts| attempts == "1");
        }
        self.request_headers
            .get("x-sai-wire-attempts")
            .is_some_and(|attempts| attempts == "1")
    }

    pub fn rise_resolution_source(&self) -> Option<&str> {
        if !self.is_rise_engine() {
            return None;
        }
        self.response_headers.get("x-rise-source").map(String::as_str)
    }

    pub fn rise_path_line(&self) -> Option<String> {
        if !self.is_rise_engine() {
            return None;
        }
        if self.has_tag(SaiTraceTag::DIRECT_RPC) {
            if !self.is_network_request() {
                return Some(
                    if self.is_pending {
                        "WAITING · WIRE 0"
                    } else {
                        "LOCAL FAILURE · WIRE 0"
                    }
                    .to_owned(),
                );
            }
            let request_id = self
                .request_headers
                .get("x-rise-request-id")
                .map(String::as_str)
                .unwrap_or("?");
            return Some(format!("WIRE 1 · id={request_id}"));
        }
        if self.is_pending {
            let presentation = self
                .request_headers
                .get("x-rise-presentation-cache")
                .map(String::as_str)
                .unwrap_or("unknown");
            let policy = self
                .request_headers
                .get("x-rise-policy")
                .map(String::as_str)
                .unwrap_or("unknown");
            return Some(format!(
                "UI {} → {}…",
                Self::rise_label(presentation),
                Self::rise_label(policy)
            ));
        }

        let mut parts: Vec<String> = Vec::new();
        if let Some(presentation) = self.request_headers.get("x-rise-presentation-cache") {
            if presentation != "not-applicable" {
                parts.push(format!("UI {}", Self::rise_label(presentation)));
            }
        }
        if let Some(lookup) = self.response_headers.get("x-rise-cache-lookup") {
            if lookup != "not-checked" {
                parts.push(format!("LOOKUP {}", Self::rise_label(lookup)));
            }
        }
        if self
            .response_headers
            .get("x-rise-coalesced")
            .is_some_and(|coalesced| coalesced == "true")
        {
            parts.push("JOIN".to_owned());
        }
        if let Some(source) = self.rise_resolution_source() {
            let rpc_attempts = int_from_string(
                self.response_headers
                    .get("x-rise-rpc-attempts")
                    .map(String::as_str)
                    .unwrap_or("0"),
            )
            .unwrap_or(0);
            let rpc_failures = int_from_string(
                self.response_headers
                    .get("x-rise-rpc-failures")
                    .map(String::as_str)
                    .unwrap_or("0"),
            )
            .unwrap_or(0);
            if rpc_failures > 0 && rpc_failures < rpc_attempts {
                parts.push("RPC FAIL / RECOVERED".to_owned());
            } else if rpc_attempts > 0
                && source != "network"
                && !source.starts_with("network-fallback")
            {
                parts.push(
                    if rpc_failures > 0 {
                        "RPC ATTEMPT FAIL"
                    } else {
                        "NETWORK"
                    }
                    .to_owned(),
                );
            }
            parts.push(Self::rise_label(source));
        }
        if let Some(write) = self.response_headers.get("x-rise-cache-write") {
            if write != "none" {
                parts.push(format!("WRITE {}", Self::rise_label(write)));
            }
        }
        if self
            .response_headers
            .get("x-rise-ui-result")
            .is_some_and(|result| result == "published")
        {
            parts.push("PRESENTATION UPDATED".to_owned());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" → "))
        }
    }

    pub fn summary_line(&self) -> String {
        match self.transport {
            SaiTraceTransport::Http => format!("{} {}", self.method, self.url),
            SaiTraceTransport::Websocket => match self.fetch_id.as_deref() {
                Some(fetch_id) if !fetch_id.is_empty() => format!("{fetch_id} · {}", self.url),
                _ => self.url.clone(),
            },
        }
    }

    pub fn has_status_code(&self) -> bool {
        !self.is_pending && self.response_status > 0
    }

    pub fn is_success_status(&self) -> bool {
        (200..300).contains(&self.response_status)
    }

    pub fn copy_block_text(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("[{}] {}", self.transport.raw_value(), self.summary_line()));
        if let Some(cache_source) = self.cache_source() {
            if self.is_rise_engine() {
                let resolved = if cache_source == SaiTraceCacheSource::Memory {
                    "memory"
                } else {
                    "RiseStore"
                };
                lines.push(format!("resolved cache: {resolved}"));
            } else {
                let cache = if cache_source == SaiTraceCacheSource::Memory {
                    "memory (no network)"
                } else {
                    "disk (no network)"
                };
                lines.push(format!("cache: {cache}"));
            }
        }
        if let Some(rise_path_line) = self.rise_path_line() {
            lines.push(format!("path: {rise_path_line}"));
        }
        lines.push(format!("duration: {} ms", self.duration_ms));
        if let Some(fetch_id) = self.fetch_id.as_deref() {
            if !fetch_id.is_empty() && self.transport == SaiTraceTransport::Http {
                lines.push(format!("id: {fetch_id}"));
            }
        }
        if !self.trace_tags.is_empty() {
            lines.push(format!("tags: {}", self.trace_tags.join(", ")));
        }
        match self.transport_error.as_deref() {
            Some(transport_error) if !transport_error.is_empty() => {
                lines.push(format!("transport: {transport_error}"));
            }
            _ => {
                if self.has_status_code() {
                    lines.push(format!("status: {}", self.response_status));
                } else if self.is_pending {
                    lines.push("status: pending".to_owned());
                }
            }
        }
        lines.push(String::new());
        lines.push("— Request headers —".to_owned());
        lines.push(Self::headers_block(&self.request_headers));
        lines.push(String::new());
        lines.push("— Request body —".to_owned());
        lines.push(self.request_body_wire.clone());
        lines.push(String::new());
        lines.push("— Response headers —".to_owned());
        lines.push(Self::headers_block(&self.response_headers));
        lines.push(String::new());
        lines.push("— Response body —".to_owned());
        lines.push(self.response_body_preview.clone());
        lines.join("\n")
    }

    /// completed copy of the entry, trace_tags none keeps the entry own tags
    pub fn with_completed(
        &self,
        duration_ms: i64,
        response_status: i64,
        response_headers: HashMap<String, String>,
        response_body_preview: String,
        transport_error: Option<String>,
        trace_tags: Option<Vec<String>>,
    ) -> SaiHttpTraceEntry {
        SaiHttpTraceEntry {
            id: self.id,
            transport: self.transport,
            created_at: self.created_at,
            is_pending: false,
            method: self.method.clone(),
            url: self.url.clone(),
            fetch_id: self.fetch_id.clone(),
            duration_ms,
            request_headers: self.request_headers.clone(),
            request_body_wire: self.request_body_wire.clone(),
            response_status,
            response_headers,
            response_body_preview,
            transport_error,
            trace_tags: trace_tags.unwrap_or_else(|| self.trace_tags.clone()),
        }
    }

    fn has_tag(&self, tag: &str) -> bool {
        self.trace_tags.iter().any(|candidate| candidate == tag)
    }

    fn rise_label(raw: &str) -> String {
        match raw {
            "memory-hit" => "MEMORY HIT".to_owned(),
            "legacy-disk-hydrated" => "LEGACY DISK".to_owned(),
            "miss" => "MISS".to_owned(),
            "not-keyed" => "NO KEY".to_owned(),
            "cache-first" => "CACHE FIRST".to_owned(),
            "reload" => "NETWORK RELOAD".to_owned(),
            "memory" | "resource-memory" => "MEMORY".to_owned(),
            "persistent-store" => "RISESTORE".to_owned(),
            "network" => "NETWORK".to_owned(),
            "network-fallback-memory" => "RPC FAIL / MEMORY".to_owned(),
            "network-fallback-persistent-store" => "RPC FAIL / RISESTORE".to_owned(),
            "normalized-store" => "NORMALIZED STORE".to_owned(),
            "stale-dropped" => "STALE DROPPED".to_owned(),
            "failure" => "FAILED".to_owned(),
            _ => raw.to_uppercase(),
        }
    }

    fn headers_block(headers: &HashMap<String, String>) -> String {
        if headers.is_empty() {
            return "(none)".to_owned();
        }
        let mut keys: Vec<&String> = headers.keys().collect();
        keys.sort();
        keys.into_iter()
            .map(|key| {
                format!(
                    "{key}: {}",
                    headers.get(key).map(String::as_str).unwrap_or_default()
                )
            })
            .collect::<Vec<String>>()
            .join("\n")
    }
}
