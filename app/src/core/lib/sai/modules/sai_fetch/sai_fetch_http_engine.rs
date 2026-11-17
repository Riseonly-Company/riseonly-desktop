use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use futures::AsyncReadExt;
use futures::future::{Either, select};
use gpui::http_client::http::header::{ACCEPT, CONTENT_TYPE};
use gpui::http_client::http::{HeaderName, HeaderValue, Method, Request, Response, Uri};
use gpui::http_client::{AsyncBody, HttpClient};
use gpui::{App, AsyncApp, Task};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::{Url, UrlComponents, UrlQueryItem};
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_entry::{SaiHttpTraceEntry, SaiTraceTransport};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_recorder::SaiHttpTraceRecorder;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::SaiHttpTraceStore;

use super::sai_fetch_http_configuration::{SaiFetchHttpConfiguration, SaiFetchResponseInterceptor};
use super::sai_fetch_http_error::SaiFetchHttpError;

const METHOD_GET: &str = "GET";
const METHOD_HEAD: &str = "HEAD";
const ACCEPT_JSON: &str = "application/json";
const CONTENT_TYPE_JSON: &str = "application/json; charset=utf-8";
const RESPONSE_PREVIEW_MAX_CHARS: usize = 32_000;
const REQUEST_TIMED_OUT: &str = "The request timed out.";
const NULL_DESCRIPTION: &str = "<null>";
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// runs one request over the shared process http client
pub enum SaiFetchHttpEngine {}

impl SaiFetchHttpEngine {
    pub fn fetch_response_body(
        route: &str,
        method: &str,
        body: Option<&Map<String, Value>>,
        headers: Option<&HashMap<String, String>>,
        timeout: Option<TimeInterval>,
        fetch_id: Option<&str>,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<u8>>> {
        let configuration = SaiFetchHttpConfiguration::shared(cx);
        let cfg = configuration.read(cx);
        let base_url = cfg.base_url.clone();
        let default_headers = cfg.default_headers.clone();
        let request_interceptors = cfg.request_interceptors.clone();
        let response_interceptors = cfg.response_interceptors.clone();

        let mut request = match Self::build_request(route, method, body, headers, base_url.as_ref(), &default_headers) {
            Ok(request) => request,
            Err(error) => return Task::ready(Err(error)),
        };
        let fetch_id = fetch_id.map(str::to_owned);
        let client = cx.http_client();

        cx.spawn(async move |cx| {
            for hook in &request_interceptors {
                let pending = cx.update(|cx| hook(request, cx));
                request = pending.await?;
            }
            Self::exchange(request, timeout, fetch_id, &response_interceptors, client, cx).await
        })
    }

    fn build_request(
        route: &str,
        method: &str,
        body: Option<&Map<String, Value>>,
        headers: Option<&HashMap<String, String>>,
        base_url: Option<&Url>,
        default_headers: &HashMap<String, String>,
    ) -> anyhow::Result<Request<Vec<u8>>> {
        let url = Self::resolve_url(route, method, body, base_url)?;
        let upper = method.to_uppercase();

        let mut request = Request::new(Vec::new());
        *request.method_mut() = Method::from_bytes(upper.as_bytes())?;
        *request.uri_mut() = url
            .absolute_string()
            .parse::<Uri>()
            .map_err(|_| SaiFetchHttpError::InvalidUrl(url.absolute_string().to_owned()))?;

        for (k, v) in default_headers {
            Self::set_value(&mut request, k, v)?;
        }
        if let Some(headers) = headers {
            for (k, v) in headers {
                Self::set_value(&mut request, k, v)?;
            }
        }
        if !request.headers().contains_key(ACCEPT) {
            request.headers_mut().insert(ACCEPT, HeaderValue::from_static(ACCEPT_JSON));
        }

        if upper != METHOD_GET && upper != METHOD_HEAD {
            if let Some(body) = body.filter(|body| !body.is_empty()) {
                *request.body_mut() = serde_json::to_vec(body)?;
                if !request.headers().contains_key(CONTENT_TYPE) {
                    request.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_JSON));
                }
            }
        }
        Ok(request)
    }

    /// sets a header and replaces any old value under the same name, case insensitive
    fn set_value(request: &mut Request<Vec<u8>>, field: &str, value: &str) -> anyhow::Result<()> {
        let name = HeaderName::from_bytes(field.as_bytes())?;
        let value = HeaderValue::from_str(value)?;
        request.headers_mut().insert(name, value);
        Ok(())
    }

    async fn exchange(
        request: Request<Vec<u8>>,
        timeout: Option<TimeInterval>,
        fetch_id: Option<String>,
        response_interceptors: &[SaiFetchResponseInterceptor],
        client: Arc<dyn HttpClient>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Vec<u8>> {
        let t0 = RiseClock::now();
        let method_final = request.method().as_str().to_uppercase();
        let url_final = request.uri().to_string();

        let http_debug_trace_id =
            (cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools")).then(Uuid::new_v4);
        if let Some(id) = http_debug_trace_id {
            let http_body = request.body();
            let entry = SaiHttpTraceEntry {
                id,
                transport: SaiTraceTransport::Http,
                created_at: t0,
                is_pending: true,
                method: method_final,
                url: url_final,
                fetch_id,
                duration_ms: 0,
                request_headers: SaiHttpTraceRecorder::request_headers(&request),
                request_body_wire: SaiHttpTraceRecorder::wire_json_string(
                    (!http_body.is_empty()).then_some(http_body.as_slice()),
                ),
                response_status: 0,
                response_headers: HashMap::new(),
                response_body_preview: String::new(),
                transport_error: None,
                trace_tags: Vec::new(),
            };
            cx.update(|cx| SaiHttpTraceStore::shared(cx).update(cx, |store, cx| store.upsert(entry, cx)));
        }

        let request = Rc::new(request);
        let mut http_trace_done = false;
        let result = Self::send_and_read(
            &request,
            timeout,
            response_interceptors,
            client,
            http_debug_trace_id,
            t0,
            &mut http_trace_done,
            cx,
        )
        .await;

        if let Err(error) = &result
            && !http_trace_done
        {
            Self::finish_http_trace(
                http_debug_trace_id,
                t0,
                0,
                HashMap::new(),
                String::new(),
                Some(error.to_string()),
                cx,
            );
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_and_read(
        request: &Rc<Request<Vec<u8>>>,
        timeout: Option<TimeInterval>,
        response_interceptors: &[SaiFetchResponseInterceptor],
        client: Arc<dyn HttpClient>,
        http_debug_trace_id: Option<Uuid>,
        t0: Date,
        http_trace_done: &mut bool,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Vec<u8>> {
        let mut outbound = Request::new(AsyncBody::from(request.body().clone()));
        *outbound.method_mut() = request.method().clone();
        *outbound.uri_mut() = request.uri().clone();
        *outbound.headers_mut() = request.headers().clone();

        let limit = Self::request_limit(timeout);
        let response = Self::limited(client.send(outbound), limit, cx).await?;

        let (parts, mut body) = response.into_parts();
        let mut payload = Vec::new();
        Self::limited(
            async { body.read_to_end(&mut payload).await.map_err(anyhow::Error::from) },
            limit,
            cx,
        )
        .await?;
        let http = Rc::new(Response::from_parts(parts, ()));

        for hook in response_interceptors {
            let request = Rc::clone(request);
            let response = Rc::clone(&http);
            let pending = cx.update(|cx| hook(request, response, payload, cx));
            payload = pending.await?;
        }

        let status_code = i64::from(http.status().as_u16());
        if http_debug_trace_id.is_some() {
            *http_trace_done = true;
            Self::finish_http_trace(
                http_debug_trace_id,
                t0,
                status_code,
                SaiHttpTraceRecorder::response_headers(&http),
                SaiHttpTraceRecorder::truncate_utf8(&payload, RESPONSE_PREVIEW_MAX_CHARS),
                None,
                cx,
            );
        }

        if !(200..300).contains(&status_code) {
            return Err(SaiFetchHttpError::BadStatus {
                code: status_code,
                body: payload,
            }
            .into());
        }
        Ok(payload)
    }

    fn request_limit(timeout: Option<TimeInterval>) -> Duration {
        timeout
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT)
    }

    async fn limited<T>(
        work: impl Future<Output = anyhow::Result<T>>,
        limit: Duration,
        cx: &AsyncApp,
    ) -> anyhow::Result<T> {
        let timer = cx.background_executor().timer(limit);
        match select(Box::pin(work), timer).await {
            Either::Left((result, _)) => result,
            Either::Right(_) => Err(anyhow!(REQUEST_TIMED_OUT)),
        }
    }

    fn finish_http_trace(
        http_debug_trace_id: Option<Uuid>,
        t0: Date,
        response_status: i64,
        response_headers: HashMap<String, String>,
        response_body_preview: String,
        transport_error: Option<String>,
        cx: &mut AsyncApp,
    ) {
        let Some(id) = http_debug_trace_id else {
            return;
        };
        let duration_ms = (RiseClock::now().time_interval_since(&t0) * 1000.0) as i64;
        cx.update(|cx| {
            SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
                store.merge_http_completion(
                    id,
                    duration_ms,
                    response_status,
                    response_headers,
                    response_body_preview,
                    transport_error,
                    cx,
                );
            });
        });
    }

    fn resolve_url(
        route: &str,
        method: &str,
        body: Option<&Map<String, Value>>,
        base_url: Option<&Url>,
    ) -> Result<Url, SaiFetchHttpError> {
        let trimmed = trimming_whitespaces_and_newlines(route);
        if let Some(absolute) = Url::from_string(trimmed).filter(|url| url.scheme().is_some()) {
            return Self::append_query_if_get(absolute, method, body);
        }

        let Some(base) = base_url else {
            return Err(SaiFetchHttpError::MissingBaseUrl(trimmed.to_owned()));
        };

        let base_str = trimming_characters(base.absolute_string(), |c| c == '/');
        let path = if trimmed.starts_with('/') {
            trimmed.to_owned()
        } else {
            format!("/{trimmed}")
        };
        let Some(url) = Url::from_string(&format!("{base_str}{path}")) else {
            return Err(SaiFetchHttpError::InvalidUrl(trimmed.to_owned()));
        };
        Self::append_query_if_get(url, method, body)
    }

    fn append_query_if_get(url: Url, method: &str, body: Option<&Map<String, Value>>) -> Result<Url, SaiFetchHttpError> {
        let Some(body) = body.filter(|body| method.to_uppercase() == METHOD_GET && !body.is_empty()) else {
            return Ok(url);
        };
        let Some(mut components) = UrlComponents::from_url(&url, false) else {
            return Err(SaiFetchHttpError::InvalidUrl(url.absolute_string().to_owned()));
        };
        let mut items = components.query_items().unwrap_or_default();
        for (k, v) in body {
            items.push(UrlQueryItem::new(k.clone(), Some(query_value_description(v))));
        }
        components.set_query_items(Some(items));
        components
            .url()
            .ok_or_else(|| SaiFetchHttpError::InvalidUrl(url.absolute_string().to_owned()))
    }
}

fn query_value_description(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => if *b { "1" } else { "0" }.to_owned(),
        Value::Null => NULL_DESCRIPTION.to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "sai_fetch_http_engine_tests.rs"]
mod tests;
