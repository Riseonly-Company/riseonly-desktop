use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{App, Context, Entity};
use rise_canon::rise_wire_canon::key;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_entry::{
    SaiHttpTraceEntry, SaiTraceTag, SaiTraceTransport,
};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_recorder::SaiHttpTraceRecorder;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh_debug::SaiWsScreenRefreshDebug;

/// defaulted tail of the completed ws trace, covers duration and status and transport error and tags
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiCompletedWsTraceOptions {
    pub duration_ms: i64,
    pub response_status: i64,
    pub transport_error: Option<String>,
    pub trace_tags: Vec<String>,
}

impl Default for SaiCompletedWsTraceOptions {
    fn default() -> Self {
        Self { duration_ms: 0, response_status: 200, transport_error: None, trace_tags: Vec::new() }
    }
}

const NOTIFY_COALESCE: Duration = Duration::from_millis(16);

static HAS_READER: AtomicBool = AtomicBool::new(false);

pub struct SaiHttpTraceStore {
    entries: Vec<SaiHttpTraceEntry>,
    max_entries: usize,
    ws_awaiting_by_request_key: HashMap<String, Uuid>,
    notify_scheduled: bool,
}

impl SaiHttpTraceStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            entries: Vec::new(),
            max_entries: 200,
            ws_awaiting_by_request_key: HashMap::new(),
            notify_scheduled: false,
        }
    }

    pub fn set_has_reader(has_reader: bool) {
        HAS_READER.store(has_reader, Ordering::Relaxed);
    }

    fn notify_readers(&mut self, cx: &mut Context<Self>) {
        if self.notify_scheduled || !HAS_READER.load(Ordering::Relaxed) {
            return;
        }
        self.notify_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTIFY_COALESCE).await;
            let _ = this.update(cx, |this, cx| {
                this.notify_scheduled = false;
                cx.notify();
            });
        })
        .detach();
    }

    const fn debug_tools() -> bool {
        cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools")
    }

    pub fn entries(&self) -> &[SaiHttpTraceEntry] {
        &self.entries
    }

    pub fn upsert(&mut self, entry: SaiHttpTraceEntry, cx: &mut Context<Self>) {
        if !Self::debug_tools() {
            SaiWsScreenRefreshDebug::log(
                || "SaiHttpTraceStore.upsert SKIPPED — not DEBUG build".to_owned(),
                cx,
            );
            return;
        }
        if entry
            .trace_tags
            .iter()
            .any(|tag| tag == SaiTraceTag::SCREEN_REFRESH)
        {
            let fetch_id = entry.fetch_id.clone().unwrap_or_else(|| "?".to_owned());
            let is_pending = entry.is_pending;
            let trace_tags = entry.trace_tags.clone();
            let total = self.entries.len() + 1;
            SaiWsScreenRefreshDebug::log(
                move || {
                    format!(
                        "SaiHttpTraceStore.upsert {fetch_id} pending={is_pending} tags={trace_tags:?} total={total}"
                    )
                },
                cx,
            );
        }
        if let Some(i) = self.entries.iter().position(|existing| existing.id == entry.id) {
            if !self.entries[i].is_pending && entry.is_pending {
                return;
            }
            if self.entries[i].is_network_request() && !entry.is_network_request() {
                return;
            }
            self.entries[i] = entry;
        } else {
            self.entries.insert(0, entry);
            self.trim_if_needed();
        }
        self.notify_readers(cx);
    }

    pub fn register_ws_awaiting_response(&mut self, request_key: String, trace_id: Uuid) {
        self.ws_awaiting_by_request_key.insert(request_key, trace_id);
    }

    pub fn cancel_ws_awaiting(&mut self, request_key: &str) {
        self.ws_awaiting_by_request_key.remove(request_key);
    }

    pub fn complete_ws_trace_if_linked(
        &mut self,
        request_key: &str,
        incoming: &Map<String, Value>,
        server_error: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(trace_id) = self.ws_awaiting_by_request_key.get(request_key).copied() else {
            return;
        };
        let Some(idx) = self.entries.iter().position(|entry| entry.id == trace_id) else {
            self.ws_awaiting_by_request_key.remove(request_key);
            return;
        };
        self.ws_awaiting_by_request_key.remove(request_key);
        let prev = &self.entries[idx];
        let ms = (Date::now().time_interval_since(&prev.created_at) * 1000.0) as i64;
        let body = SaiHttpTraceRecorder::pretty_json_string_from_json_object(incoming);
        let status = if server_error { 502 } else { 200 };
        let transport_err = if server_error {
            Some(Self::ws_server_error_text(incoming))
        } else {
            None
        };
        let completed = prev.with_completed(
            ms,
            status,
            HashMap::new(),
            body,
            transport_err,
            None,
        );
        self.entries[idx] = completed;
        self.notify_readers(cx);
    }

    pub fn fail_ws_trace_if_linked(
        &mut self,
        request_key: &str,
        message: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(trace_id) = self.ws_awaiting_by_request_key.remove(request_key) else {
            return;
        };
        let Some(idx) = self.entries.iter().position(|entry| entry.id == trace_id) else {
            return;
        };
        let prev = &self.entries[idx];
        let ms = (Date::now().time_interval_since(&prev.created_at) * 1000.0) as i64;
        let completed = prev.with_completed(
            ms,
            0,
            HashMap::new(),
            String::new(),
            Some(message.to_owned()),
            None,
        );
        self.entries[idx] = completed;
        self.notify_readers(cx);
    }

    pub fn fail_all_awaiting_ws(&mut self, message: &str, cx: &mut Context<Self>) {
        let keys: Vec<String> = self.ws_awaiting_by_request_key.keys().cloned().collect();
        for key in keys {
            self.fail_ws_trace_if_linked(&key, message, cx);
        }
    }

    pub fn append_completed_ws_trace(
        &mut self,
        fetch_id: String,
        url: String,
        request_body_wire: String,
        response_headers: HashMap<String, String>,
        response_body_preview: String,
        options: SaiCompletedWsTraceOptions,
        cx: &mut Context<Self>,
    ) {
        let SaiCompletedWsTraceOptions { duration_ms, response_status, transport_error, trace_tags } = options;
        self.upsert(
            SaiHttpTraceEntry {
                id: Uuid::new_v4(),
                transport: SaiTraceTransport::Websocket,
                created_at: Date::now(),
                is_pending: false,
                method: "WS".to_owned(),
                url,
                fetch_id: Some(fetch_id),
                duration_ms,
                request_headers: HashMap::new(),
                request_body_wire,
                response_status,
                response_headers,
                response_body_preview,
                transport_error,
                trace_tags,
            },
            cx,
        );
    }

    pub fn append_memory_cache_hit_http_trace(
        &mut self,
        method: &str,
        url: &str,
        fetch_id: Option<&str>,
        request_body_wire: &str,
        response_body_preview: &str,
        cx: &mut Context<Self>,
    ) {
        let mut headers: HashMap<String, String> = HashMap::from([
            ("x-sai-resolve".to_owned(), "cache-hit-no-reload".to_owned()),
            ("x-sai-served-from".to_owned(), "memory".to_owned()),
        ]);
        if let Some(fetch_id) = fetch_id {
            if !fetch_id.is_empty() {
                headers.insert("x-sai-cache-key".to_owned(), fetch_id.to_owned());
            }
        }
        self.upsert(
            SaiHttpTraceEntry {
                id: Uuid::new_v4(),
                transport: SaiTraceTransport::Http,
                created_at: Date::now(),
                is_pending: false,
                method: method.to_owned(),
                url: url.to_owned(),
                fetch_id: fetch_id.map(str::to_owned),
                duration_ms: 0,
                request_headers: HashMap::new(),
                request_body_wire: request_body_wire.to_owned(),
                response_status: 200,
                response_headers: headers,
                response_body_preview: response_body_preview.to_owned(),
                transport_error: None,
                trace_tags: Vec::new(),
            },
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn merge_http_completion(
        &mut self,
        id: Uuid,
        duration_ms: i64,
        response_status: i64,
        response_headers: HashMap<String, String>,
        response_body_preview: String,
        transport_error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !Self::debug_tools() {
            return;
        }
        let Some(i) = self.entries.iter().position(|entry| entry.id == id) else {
            return;
        };
        let completed = self.entries[i].with_completed(
            duration_ms,
            response_status,
            response_headers,
            response_body_preview,
            transport_error,
            None,
        );
        self.entries[i] = completed;
        self.notify_readers(cx);
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if !Self::debug_tools() {
            return;
        }
        self.entries.clear();
        self.ws_awaiting_by_request_key.clear();
        cx.notify();
    }

    pub fn remove_entry(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.entries.retain(|entry| entry.id != id);
        self.ws_awaiting_by_request_key
            .retain(|_, trace_id| *trace_id != id);
        cx.notify();
    }

    fn trim_if_needed(&mut self) {
        if self.entries.len() > self.max_entries {
            self.entries.truncate(self.max_entries);
        }
    }

    fn ws_server_error_text(incoming: &Map<String, Value>) -> String {
        match incoming.get(key::ERROR) {
            Some(Value::String(e)) if !e.is_empty() => e.clone(),
            Some(Value::Object(d)) => {
                if let Some(Value::String(m)) = d.get(key::MESSAGE) {
                    if !m.is_empty() {
                        return m.clone();
                    }
                }
                if let Some(Value::String(c)) = d.get(key::CODE) {
                    if !c.is_empty() {
                        return c.clone();
                    }
                }
                Value::Object(d.clone()).to_string()
            }
            _ => "WebSocket response error".to_owned(),
        }
    }
}
