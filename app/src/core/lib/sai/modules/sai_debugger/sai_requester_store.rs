#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use futures::AsyncReadExt;
use futures::future::{Either, select};
use gpui::http_client::http::header::{AUTHORIZATION, CONTENT_TYPE};
use gpui::http_client::http::{HeaderName, HeaderValue, Method, Request, Response, Uri};
use gpui::http_client::{AsyncBody, HttpClient};
use gpui::{App, AsyncApp, Context, Entity};
use rise_canon::rise_wire_canon::{OUTBOUND_FRAME_TYPE, key};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::{
    is_newline, trimming_whitespaces, trimming_whitespaces_and_newlines,
};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use super::sai_api_doc_types::{SaiApiEndpointDoc, SaiApiVariant};
use super::sai_http_trace_entry::{SaiHttpTraceEntry, SaiTraceTransport};
use super::sai_http_trace_recorder::SaiHttpTraceRecorder;
use super::sai_http_trace_store::SaiHttpTraceStore;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

const TRANSPORT_WS: &str = "WS";
const CONTENT_TYPE_JSON: &str = "application/json";
const REQUEST_TIMED_OUT: &str = "The request timed out.";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const RESPONSE_PREVIEW_MAX_CHARS: usize = 64_000;
const REQUESTER_FETCH_ID: &str = "requester";
const REQUESTER_TRACE_TAG: &str = "Requester";
const DEFAULT_HTTP_BODY_JSON: &str = "{\n  \n}";

const WS_ENVELOPE_KEYS: [&str; 7] = [
    key::SERVICE,
    key::METHOD,
    key::FRAME_TYPE,
    key::REQUEST_ID,
    key::ID,
    key::TIMESTAMP,
    key::METADATA,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiRequesterMode {
    Websocket,
    Http,
}

impl SaiRequesterMode {
    pub const ALL: [Self; 2] = [Self::Websocket, Self::Http];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Websocket => "WS",
            Self::Http => "HTTP",
        }
    }

    pub const fn id(self) -> &'static str {
        self.raw_value()
    }
}

/// requester http method
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiRequesterHTTPMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl SaiRequesterHTTPMethod {
    pub const ALL: [Self; 5] = [Self::Get, Self::Post, Self::Put, Self::Patch, Self::Delete];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    pub const fn id(self) -> &'static str {
        self.raw_value()
    }

    /// parses the method from its raw value
    pub fn from_raw(raw_value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|method| method.raw_value() == raw_value)
    }

    fn wire_method(self) -> Method {
        match self {
            Self::Get => Method::GET,
            Self::Post => Method::POST,
            Self::Put => Method::PUT,
            Self::Patch => Method::PATCH,
            Self::Delete => Method::DELETE,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaiRequesterState {
    Idle,
    Loading,
    Success(String),
    Failure(String),
}

pub struct SaiRequesterStore {
    ws_service: String,
    ws_method: String,
    ws_body_json: String,
    http_method: SaiRequesterHTTPMethod,
    http_url: String,
    http_body_json: String,
    http_headers: String,
    mode: SaiRequesterMode,
    state: SaiRequesterState,
    last_response_body: String,
    tab_jump_tick: i64,
}

impl SaiRequesterStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            ws_service: String::new(),
            ws_method: String::new(),
            ws_body_json: Self::make_empty_ws_body("", ""),
            http_method: SaiRequesterHTTPMethod::Post,
            http_url: String::new(),
            http_body_json: DEFAULT_HTTP_BODY_JSON.to_owned(),
            http_headers: String::new(),
            mode: SaiRequesterMode::Websocket,
            state: SaiRequesterState::Idle,
            last_response_body: String::new(),
            tab_jump_tick: 0,
        }
    }

    pub fn ws_service(&self) -> &str {
        &self.ws_service
    }

    pub fn ws_method(&self) -> &str {
        &self.ws_method
    }

    pub fn ws_body_json(&self) -> &str {
        &self.ws_body_json
    }

    pub fn http_method(&self) -> SaiRequesterHTTPMethod {
        self.http_method
    }

    pub fn http_url(&self) -> &str {
        &self.http_url
    }

    pub fn http_body_json(&self) -> &str {
        &self.http_body_json
    }

    pub fn http_headers(&self) -> &str {
        &self.http_headers
    }

    pub fn mode(&self) -> SaiRequesterMode {
        self.mode
    }

    pub fn state(&self) -> &SaiRequesterState {
        &self.state
    }

    pub fn last_response_body(&self) -> &str {
        &self.last_response_body
    }

    pub fn tab_jump_tick(&self) -> i64 {
        self.tab_jump_tick
    }

    /// sets the ws service, an unchanged value leaves the body alone
    pub fn set_ws_service(&mut self, value: String, cx: &mut Context<Self>) {
        if self.ws_service == value {
            return;
        }
        self.ws_service = value.clone();
        self.patch_ws_body_field(key::SERVICE, &value);
        cx.notify();
    }

    /// sets the ws method, an unchanged value leaves the body alone
    pub fn set_ws_method(&mut self, value: String, cx: &mut Context<Self>) {
        if self.ws_method == value {
            return;
        }
        self.ws_method = value.clone();
        self.patch_ws_body_field(key::METHOD, &value);
        cx.notify();
    }

    pub fn set_ws_body_json(&mut self, value: String, cx: &mut Context<Self>) {
        self.ws_body_json = value;
        cx.notify();
    }

    pub fn set_http_method(&mut self, value: SaiRequesterHTTPMethod, cx: &mut Context<Self>) {
        self.http_method = value;
        cx.notify();
    }

    pub fn set_http_url(&mut self, value: String, cx: &mut Context<Self>) {
        self.http_url = value;
        cx.notify();
    }

    pub fn set_http_body_json(&mut self, value: String, cx: &mut Context<Self>) {
        self.http_body_json = value;
        cx.notify();
    }

    pub fn set_http_headers(&mut self, value: String, cx: &mut Context<Self>) {
        self.http_headers = value;
        cx.notify();
    }

    pub fn set_mode(&mut self, value: SaiRequesterMode, cx: &mut Context<Self>) {
        self.mode = value;
        cx.notify();
    }

    pub fn load_from_variant(&mut self, variant: &SaiApiVariant, doc: &SaiApiEndpointDoc, cx: &mut Context<Self>) {
        if doc.transport == TRANSPORT_WS {
            let service = doc.service.clone().unwrap_or_default();
            let method = doc.ws_method.clone().unwrap_or_default();
            let data_obj = Self::extract_actual_data(&variant.request_body_example);

            let mut body_dict = Map::new();
            body_dict.insert(key::SERVICE.to_owned(), Value::String(service.clone()));
            body_dict.insert(key::METHOD.to_owned(), Value::String(method.clone()));
            body_dict.insert(key::DATA.to_owned(), Value::Object(data_obj));
            self.ws_body_json =
                Self::to_json(&body_dict).unwrap_or_else(|| Self::make_empty_ws_body(&service, &method));
            self.mode = SaiRequesterMode::Websocket;
            self.set_ws_service(service, cx);
            self.set_ws_method(method, cx);
        } else {
            self.mode = SaiRequesterMode::Http;
            let raw_method = doc.http_method.as_deref().unwrap_or(SaiRequesterHTTPMethod::Post.raw_value());
            self.http_method =
                SaiRequesterHTTPMethod::from_raw(&raw_method.to_uppercase()).unwrap_or(SaiRequesterHTTPMethod::Post);
            self.http_url = doc.path.clone();
            self.http_body_json =
                Self::pretty_json(&variant.request_body_example).unwrap_or_else(|| variant.request_body_example.clone());
        }

        self.state = SaiRequesterState::Idle;
        self.last_response_body = String::new();
        self.tab_jump_tick += 1;
        cx.notify();
    }

    pub fn send(&mut self, cx: &mut Context<Self>) {
        if self.state == SaiRequesterState::Loading {
            return;
        }
        self.state = SaiRequesterState::Loading;
        self.last_response_body = String::new();
        cx.notify();
        match self.mode {
            SaiRequesterMode::Websocket => self.send_ws(cx),
            SaiRequesterMode::Http => self.send_http(cx),
        }
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.state = SaiRequesterState::Idle;
        self.last_response_body = String::new();
        cx.notify();
    }

    pub fn format_ws_body(&mut self, cx: &mut Context<Self>) {
        let Some(pretty) = Self::parse_json_object(&self.ws_body_json).and_then(|obj| Self::to_json(&obj)) else {
            return;
        };
        self.ws_body_json = pretty;
        cx.notify();
    }

    pub fn format_http_body(&mut self, cx: &mut Context<Self>) {
        let Some(pretty) = Self::parse_json_object(&self.http_body_json).and_then(|obj| Self::to_json(&obj)) else {
            return;
        };
        self.http_body_json = pretty;
        cx.notify();
    }

    fn patch_ws_body_field(&mut self, field: &str, value: &str) {
        if let Some(mut dict) = Self::parse_json_object(&self.ws_body_json) {
            dict.insert(field.to_owned(), Value::String(value.to_owned()));
            if let Some(updated) = Self::to_json(&dict) {
                self.ws_body_json = updated;
            }
        } else {
            self.ws_body_json = Self::make_empty_ws_body(&self.ws_service, &self.ws_method);
        }
    }

    fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.state = SaiRequesterState::Failure(message);
        cx.notify();
    }

    fn send_ws(&mut self, cx: &mut Context<Self>) {
        let Some(mut body_dict) = Self::parse_json_object(&self.ws_body_json) else {
            self.fail("Invalid JSON in body".to_owned(), cx);
            return;
        };

        let service = Self::string_field(&body_dict, key::SERVICE, &self.ws_service);
        let method = Self::string_field(&body_dict, key::METHOD, &self.ws_method);
        if service.is_empty() || method.is_empty() {
            self.fail("service and method are required".to_owned(), cx);
            return;
        }

        let nested = body_dict.get(key::DATA).and_then(Value::as_object).cloned();
        let data_payload = match nested {
            Some(nested) => Self::unwrap_service_call_envelope(nested),
            None => {
                for envelope_key in WS_ENVELOPE_KEYS {
                    body_dict.remove(envelope_key);
                }
                body_dict
            }
        };

        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service,
            method,
            reflect_pending_in_ui: false,
            ..Default::default()
        });

        let on_success: SaiWsOnSuccess = {
            let store = cx.weak_entity();
            Rc::new(move |any: Option<Box<dyn std::any::Any>>, cx: &mut App| {
                let value = any.and_then(|any| any.downcast::<Value>().ok());
                let body = SaiHttpTraceRecorder::pretty_json_string_from_any(value.as_deref());
                let _ = store.update(cx, |store, cx| {
                    store.last_response_body = body.clone();
                    store.state = SaiRequesterState::Success(body);
                    cx.notify();
                });
            })
        };
        let on_error: SaiWsOnError = {
            let store = cx.weak_entity();
            Rc::new(move |error: &anyhow::Error, cx: &mut App| {
                let message = error.to_string();
                let _ = store.update(cx, |store, cx| store.fail(message, cx));
            })
        };

        let manager = SaiWs::manager(cx);
        cx.spawn(async move |this, cx| {
            let result = SaiWebSocketManager::send_service_call_raw_body(
                manager,
                data_payload,
                options,
                Some(on_success),
                Some(on_error),
                cx.clone(),
            )
            .await;
            if let Err(error) = result {
                let _ = this.update(cx, |store, cx| store.fail(error.to_string(), cx));
            }
        })
        .detach();
    }

    fn send_http(&mut self, cx: &mut Context<Self>) {
        let url_string = trimming_whitespaces_and_newlines(&self.http_url).to_owned();
        let uri = Url::from_string(&url_string)
            .filter(|_| !url_string.is_empty())
            .and_then(|url| url.absolute_string().parse::<Uri>().ok());
        let Some(uri) = uri else {
            self.fail("Invalid URL".to_owned(), cx);
            return;
        };

        let mut request = Request::new(Vec::new());
        *request.method_mut() = self.http_method.wire_method();
        *request.uri_mut() = uri;

        for line in self.http_headers.split(is_newline) {
            let Some((name, value)) = line.split_once(':').filter(|(name, value)| !name.is_empty() && !value.is_empty())
            else {
                continue;
            };
            let name = trimming_whitespaces(name);
            if !name.is_empty() {
                Self::set_value(&mut request, name, trimming_whitespaces(value));
            }
        }

        let auth = AuthServicesStore::shared(cx);
        let token = auth.read(cx).tokens_and_other_data().access_token.clone();
        if !token.is_empty() {
            Self::set_value(&mut request, AUTHORIZATION.as_str(), &format!("Bearer {token}"));
        }

        let body_text = trimming_whitespaces_and_newlines(&self.http_body_json);
        if self.http_method != SaiRequesterHTTPMethod::Get && !body_text.is_empty() && body_text != "{}" {
            if !request.headers().contains_key(CONTENT_TYPE) {
                request.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_JSON));
            }
            *request.body_mut() = body_text.as_bytes().to_vec();
        }

        let trace_id = Uuid::new_v4();
        let wire_body = SaiHttpTraceRecorder::wire_json_string(
            (!request.body().is_empty()).then_some(request.body().as_slice()),
        );
        let req_headers = SaiHttpTraceRecorder::request_headers(&request);
        let start_date = RiseClock::now();
        let entry = SaiHttpTraceEntry {
            id: trace_id,
            transport: SaiTraceTransport::Http,
            created_at: start_date,
            is_pending: true,
            method: self.http_method.raw_value().to_owned(),
            url: url_string,
            fetch_id: Some(REQUESTER_FETCH_ID.to_owned()),
            duration_ms: 0,
            request_headers: req_headers,
            request_body_wire: wire_body,
            response_status: 0,
            response_headers: Default::default(),
            response_body_preview: String::new(),
            transport_error: None,
            trace_tags: vec![REQUESTER_TRACE_TAG.to_owned()],
        };
        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| store.upsert(entry, cx));

        let mut outbound = Request::new(AsyncBody::from(request.body().clone()));
        *outbound.method_mut() = request.method().clone();
        *outbound.uri_mut() = request.uri().clone();
        *outbound.headers_mut() = request.headers().clone();
        let client = cx.http_client();

        cx.spawn(async move |this, cx| {
            let result = Self::exchange(outbound, client, cx).await;
            let ms = (RiseClock::now().time_interval_since(&start_date) * 1000.0) as i64;
            let _ = this.update(cx, |store, cx| match result {
                Ok((data, response)) => {
                    let status = i64::from(response.status().as_u16());
                    let r_headers = SaiHttpTraceRecorder::response_headers(&response);
                    let preview = SaiHttpTraceRecorder::pretty_json_string_from_raw_utf8(
                        &SaiHttpTraceRecorder::truncate_utf8(&data, RESPONSE_PREVIEW_MAX_CHARS),
                    );
                    SaiHttpTraceStore::shared(cx).update(cx, |trace, cx| {
                        trace.merge_http_completion(trace_id, ms, status, r_headers, preview.clone(), None, cx);
                    });
                    store.last_response_body = preview.clone();
                    store.state = SaiRequesterState::Success(preview);
                    cx.notify();
                }
                Err(error) => {
                    let message = error.to_string();
                    SaiHttpTraceStore::shared(cx).update(cx, |trace, cx| {
                        trace.merge_http_completion(
                            trace_id,
                            ms,
                            0,
                            Default::default(),
                            String::new(),
                            Some(message.clone()),
                            cx,
                        );
                    });
                    store.fail(message, cx);
                }
            });
        })
        .detach();
    }

    async fn exchange(
        outbound: Request<AsyncBody>,
        client: Arc<dyn HttpClient>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<(Vec<u8>, Response<()>)> {
        let sending = client.send(outbound);
        let timer = cx.background_executor().timer(REQUEST_TIMEOUT);
        let response = match select(sending, timer).await {
            Either::Left((response, _)) => response?,
            Either::Right(_) => return Err(anyhow!(REQUEST_TIMED_OUT)),
        };
        let (parts, mut body) = response.into_parts();
        let mut data = Vec::new();
        body.read_to_end(&mut data).await?;
        Ok((data, Response::from_parts(parts, ())))
    }

    fn set_value(request: &mut Request<Vec<u8>>, field: &str, value: &str) {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(field.as_bytes()), HeaderValue::from_str(value)) {
            request.headers_mut().insert(name, value);
        }
    }

    fn string_field(dict: &Map<String, Value>, field: &str, fallback: &str) -> String {
        let raw = dict.get(field).and_then(Value::as_str).unwrap_or(fallback);
        trimming_whitespaces_and_newlines(raw).to_owned()
    }

    fn unwrap_service_call_envelope(dict: Map<String, Value>) -> Map<String, Value> {
        let is_envelope = dict.get(key::FRAME_TYPE).and_then(Value::as_str) == Some(OUTBOUND_FRAME_TYPE)
            || (dict.contains_key(key::SERVICE) && dict.contains_key(key::METHOD) && dict.contains_key(key::DATA));
        if !is_envelope {
            return dict;
        }
        match dict.get(key::DATA).and_then(Value::as_object).cloned() {
            Some(inner) => Self::unwrap_service_call_envelope(inner),
            None => dict,
        }
    }

    fn extract_actual_data(wire_json: &str) -> Map<String, Value> {
        let Some(mut stripped) = Self::parse_json_object(wire_json) else {
            return Map::new();
        };
        for envelope_key in WS_ENVELOPE_KEYS {
            stripped.remove(envelope_key);
        }
        if stripped.len() == 1
            && let Some(data) = stripped.get(key::DATA).and_then(Value::as_object).cloned()
        {
            return Self::unwrap_service_call_envelope(data);
        }
        stripped
    }

    fn parse_json_object(text: &str) -> Option<Map<String, Value>> {
        let trimmed = trimming_whitespaces_and_newlines(text);
        if trimmed.is_empty() {
            return None;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(Value::Object(obj)) => Some(obj),
            _ => None,
        }
    }

    fn to_json(dict: &Map<String, Value>) -> Option<String> {
        serde_json::to_string_pretty(dict).ok()
    }

    fn pretty_json(raw: &str) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() {
            return None;
        }
        let value = serde_json::from_str::<Value>(trimmed).ok()?;
        serde_json::to_string_pretty(&value).ok()
    }

    fn make_empty_ws_body(service: &str, method: &str) -> String {
        format!(
            "{{\n  \"{}\" : \"{service}\",\n  \"{}\" : \"{method}\",\n  \"{}\" : {{}}\n}}",
            key::SERVICE,
            key::METHOD,
            key::DATA,
        )
    }
}
