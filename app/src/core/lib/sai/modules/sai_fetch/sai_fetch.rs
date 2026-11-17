use std::any::Any;
use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::rc::Rc;

use gpui::{App, AppContext, Entity};
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_recorder::SaiHttpTraceRecorder;
use crate::core::lib::sai::sai::Sai;

use super::sai_fetch_handle::{SaiFetchHandle, SaiFetchMemoryCacheTraceContext, SaiFetchOperation};
use super::sai_fetch_http_configuration::{
    SaiFetchHttpConfiguration, SaiFetchJsonDecoder, SaiFetchJsonEncoder, SaiFetchRequestInterceptor,
    SaiFetchResponseInterceptor,
};
use super::sai_fetch_http_engine::SaiFetchHttpEngine;
use super::sai_fetch_memory_cache::SaiFetchMemoryCache;
use super::sai_fetch_options::{SaiFetchOptions, SaiFetchOptionsParseError};

const RESPONSE_PREVIEW_MAX_CHARS: usize = 32_000;
const EMPTY_BODY_WIRE: &str = "{}";

/// the axios-ish http entry, every member runs on the main thread
/// overloads carry their distinguishing label in the name
pub enum SaiFetch {}

impl SaiFetch {
    pub fn configure_http(
        base_url: Option<Url>,
        default_headers: HashMap<String, String>,
        json_decoder: Option<SaiFetchJsonDecoder>,
        json_encoder: Option<SaiFetchJsonEncoder>,
        request_interceptors: Vec<SaiFetchRequestInterceptor>,
        response_interceptors: Vec<SaiFetchResponseInterceptor>,
        cx: &mut App,
    ) {
        let base_description = base_url
            .as_ref()
            .map_or_else(|| "nil".to_owned(), |url| url.absolute_string().to_owned());
        SaiFetchHttpConfiguration::shared(cx).update(cx, |cfg, _| {
            cfg.base_url = base_url;
            cfg.default_headers = default_headers;
            if let Some(json_decoder) = json_decoder {
                cfg.json_decoder = json_decoder;
            }
            if let Some(json_encoder) = json_encoder {
                cfg.json_encoder = json_encoder;
            }
            cfg.request_interceptors = request_interceptors;
            cfg.response_interceptors = response_interceptors;
        });
        Sai::log(
            "SaiFetch",
            move || format!("configureHttp baseURL={base_description}"),
            SaiLogKind::Network,
            cx,
        );
    }

    pub fn request(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: SaiFetchOptions,
        cx: &mut App,
    ) -> Entity<SaiFetchHandle<Vec<u8>>> {
        Self::request_data_handle(route, body, headers, options, cx)
    }

    pub fn request_decoding<T: DeserializeOwned + Serialize + Clone + 'static>(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: SaiFetchOptions,
        cx: &mut App,
    ) -> Entity<SaiFetchHandle<T>> {
        Self::decoded_request_handle(route, body, headers, options, cx)
    }

    pub fn request_encodable_decoding<T: DeserializeOwned + Serialize + Clone + 'static, Body: Serialize>(
        route: &str,
        body: &Body,
        headers: Option<HashMap<String, String>>,
        options: SaiFetchOptions,
        cx: &mut App,
    ) -> anyhow::Result<Entity<SaiFetchHandle<T>>> {
        let dict = Self::encode_body_as_json_object(body, cx)?;
        Ok(Self::decoded_request_handle(route, Some(dict), headers, options, cx))
    }

    pub fn request_options_dictionary(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: &Map<String, Value>,
        cx: &mut App,
    ) -> Result<Entity<SaiFetchHandle<Vec<u8>>>, SaiFetchOptionsParseError> {
        let parsed = SaiFetchOptions::from_options_dictionary(options)?;
        Ok(Self::request_data_handle(route, body, headers, parsed, cx))
    }

    pub fn request_options_dictionary_decoding<T: DeserializeOwned + Serialize + Clone + 'static>(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: &Map<String, Value>,
        cx: &mut App,
    ) -> Result<Entity<SaiFetchHandle<T>>, SaiFetchOptionsParseError> {
        let parsed = SaiFetchOptions::from_options_dictionary(options)?;
        Ok(Self::decoded_request_handle(route, body, headers, parsed, cx))
    }

    pub fn handler<T: Clone + 'static>(
        handle: &Entity<SaiFetchHandle<T>>,
        on_success: impl FnOnce(T, &mut App) + 'static,
        on_error: impl FnOnce(&anyhow::Error, &mut App) + 'static,
        cx: &mut App,
    ) {
        handle.update(cx, |handle, cx| handle.start(on_success, on_error, cx));
    }

    pub fn request_on_success(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: SaiFetchOptions,
        on_success: impl FnOnce(Vec<u8>, &mut App) + 'static,
        on_error: impl FnOnce(&anyhow::Error, &mut App) + 'static,
        cx: &mut App,
    ) {
        let h = Self::request(route, body, headers, options, cx);
        Self::handler(&h, on_success, on_error, cx);
    }

    pub fn request_decoding_on_success<T: DeserializeOwned + Serialize + Clone + 'static>(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: SaiFetchOptions,
        on_success: impl FnOnce(T, &mut App) + 'static,
        on_error: impl FnOnce(&anyhow::Error, &mut App) + 'static,
        cx: &mut App,
    ) {
        let h = Self::request_decoding(route, body, headers, options, cx);
        Self::handler(&h, on_success, on_error, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn request_encodable_decoding_on_success<T: DeserializeOwned + Serialize + Clone + 'static, Body: Serialize>(
        route: &str,
        body: &Body,
        headers: Option<HashMap<String, String>>,
        options: SaiFetchOptions,
        on_success: impl FnOnce(T, &mut App) + 'static,
        on_error: impl FnOnce(&anyhow::Error, &mut App) + 'static,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        let h = Self::request_encodable_decoding(route, body, headers, options, cx)?;
        Self::handler(&h, on_success, on_error, cx);
        Ok(())
    }

    pub fn request_options_dictionary_on_success(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: &Map<String, Value>,
        on_success: impl FnOnce(Vec<u8>, &mut App) + 'static,
        on_error: impl FnOnce(&anyhow::Error, &mut App) + 'static,
        cx: &mut App,
    ) -> Result<(), SaiFetchOptionsParseError> {
        let h = Self::request_options_dictionary(route, body, headers, options, cx)?;
        Self::handler(&h, on_success, on_error, cx);
        Ok(())
    }

    pub fn request_options_dictionary_decoding_on_success<T: DeserializeOwned + Serialize + Clone + 'static>(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: &Map<String, Value>,
        on_success: impl FnOnce(T, &mut App) + 'static,
        on_error: impl FnOnce(&anyhow::Error, &mut App) + 'static,
        cx: &mut App,
    ) -> Result<(), SaiFetchOptionsParseError> {
        let h = Self::request_options_dictionary_decoding(route, body, headers, options, cx)?;
        Self::handler(&h, on_success, on_error, cx);
        Ok(())
    }

    pub fn request_operation<T: Clone + Debug + 'static>(
        operation: SaiFetchOperation<T>,
        options: SaiFetchOptions,
        cx: &mut App,
    ) -> Entity<SaiFetchHandle<T>> {
        if let Some(cached) = Self::cached_value::<T>(&options, cx) {
            let trace_context = Self::records_debug_traces().then(|| {
                let preview = Self::memory_cache_response_preview_any(&cached);
                let route = options.cache_key.clone().unwrap_or_default();
                Self::memory_cache_trace_context(&route, None, &options, preview, cx)
            });
            return cx.new(|_| SaiFetchHandle::cached(cached, options, trace_context));
        }
        cx.new(|_| SaiFetchHandle::new(operation, options))
    }

    pub fn request_operation_options_dictionary<T: Clone + Debug + 'static>(
        operation: SaiFetchOperation<T>,
        options: &Map<String, Value>,
        cx: &mut App,
    ) -> Result<Entity<SaiFetchHandle<T>>, SaiFetchOptionsParseError> {
        let parsed = SaiFetchOptions::from_options_dictionary(options)?;
        Ok(Self::request_operation(operation, parsed, cx))
    }

    fn encode_body_as_json_object<Body: Serialize>(body: &Body, cx: &mut App) -> anyhow::Result<Map<String, Value>> {
        let encoder = SaiFetchHttpConfiguration::shared(cx).read(cx).json_encoder;
        match encoder.encode(body)? {
            Value::Object(dict) => Ok(dict),
            _ => Err(SaiFetchBodyEncodingError::NotJsonObject.into()),
        }
    }

    /// cached value when reload_if_cached is off and the box under the key has the right type
    fn cached_value<T: Clone + 'static>(options: &SaiFetchOptions, cx: &mut App) -> Option<T> {
        let key = options.cache_key.as_deref()?;
        if options.reload_if_cached {
            return None;
        }
        let boxed = SaiFetchMemoryCache::value(key, cx)?;
        let any: &dyn Any = &*boxed;
        let cached = any.downcast_ref::<T>()?.clone();
        let key = key.to_owned();
        Sai::log("SaiFetch", move || format!("cache hit {key}"), SaiLogKind::Cache, cx);
        Some(cached)
    }

    fn request_data_handle(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: SaiFetchOptions,
        cx: &mut App,
    ) -> Entity<SaiFetchHandle<Vec<u8>>> {
        if let Some(cached) = Self::cached_value::<Vec<u8>>(&options, cx) {
            let trace_context = Self::records_debug_traces().then(|| {
                let preview = SaiHttpTraceRecorder::truncate_utf8(&cached, RESPONSE_PREVIEW_MAX_CHARS);
                Self::memory_cache_trace_context(route, body.as_ref(), &options, preview, cx)
            });
            return cx.new(|_| SaiFetchHandle::cached(cached, options, trace_context));
        }
        let route = route.to_owned();
        let http_method = options.http_method.clone();
        let timeout = options.timeout_seconds;
        let fetch_id = options.cache_key.clone();
        let operation: SaiFetchOperation<Vec<u8>> = Rc::new(move |cx: &mut App| {
            SaiFetchHttpEngine::fetch_response_body(
                &route,
                &http_method,
                body.as_ref(),
                headers.as_ref(),
                timeout,
                fetch_id.as_deref(),
                cx,
            )
        });
        cx.new(|_| SaiFetchHandle::new(operation, options))
    }

    fn decoded_request_handle<T: DeserializeOwned + Serialize + Clone + 'static>(
        route: &str,
        body: Option<Map<String, Value>>,
        headers: Option<HashMap<String, String>>,
        options: SaiFetchOptions,
        cx: &mut App,
    ) -> Entity<SaiFetchHandle<T>> {
        if let Some(cached) = Self::cached_value::<T>(&options, cx) {
            let trace_context = Self::records_debug_traces().then(|| {
                let preview = Self::memory_cache_response_preview_encodable(&cached, cx);
                Self::memory_cache_trace_context(route, body.as_ref(), &options, preview, cx)
            });
            return cx.new(|_| SaiFetchHandle::cached(cached, options, trace_context));
        }
        let decoder = SaiFetchHttpConfiguration::shared(cx).read(cx).json_decoder;
        let route = route.to_owned();
        let http_method = options.http_method.clone();
        let timeout = options.timeout_seconds;
        let fetch_id = options.cache_key.clone();
        let operation: SaiFetchOperation<T> = Rc::new(move |cx: &mut App| {
            let fetching = SaiFetchHttpEngine::fetch_response_body(
                &route,
                &http_method,
                body.as_ref(),
                headers.as_ref(),
                timeout,
                fetch_id.as_deref(),
                cx,
            );
            cx.spawn(async move |_| {
                let data = fetching.await?;
                Ok(decoder.decode::<T>(&data)?)
            })
        });
        cx.new(|_| SaiFetchHandle::new(operation, options))
    }

    fn records_debug_traces() -> bool {
        cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools")
    }

    fn memory_cache_trace_context(
        route: &str,
        body: Option<&Map<String, Value>>,
        options: &SaiFetchOptions,
        response_body_preview: String,
        cx: &mut App,
    ) -> SaiFetchMemoryCacheTraceContext {
        let base = SaiFetchHttpConfiguration::shared(cx).read(cx).base_url.clone();
        let trimmed = trimming_whitespaces_and_newlines(route);
        let url = match (Url::from_string(trimmed).filter(|url| url.scheme().is_some()), base) {
            (Some(absolute), _) => absolute.absolute_string().to_owned(),
            (None, None) => trimmed.to_owned(),
            (None, Some(base)) => {
                let base_str = trimming_characters(base.absolute_string(), |c| c == '/');
                if trimmed.starts_with('/') {
                    format!("{base_str}{trimmed}")
                } else {
                    format!("{base_str}/{trimmed}")
                }
            }
        };
        let req_body = match body {
            Some(body) if !body.is_empty() => SaiHttpTraceRecorder::pretty_json_string_from_json_object(body),
            _ => EMPTY_BODY_WIRE.to_owned(),
        };
        SaiFetchMemoryCacheTraceContext {
            method: options.http_method.to_uppercase(),
            url,
            fetch_id: options.cache_key.clone(),
            request_body_wire: req_body,
            response_body_preview,
        }
    }

    fn memory_cache_response_preview_encodable<T: Serialize + 'static>(value: &T, cx: &mut App) -> String {
        let any: &dyn Any = value;
        if let Some(data) = any.downcast_ref::<Vec<u8>>() {
            return SaiHttpTraceRecorder::truncate_utf8(data, RESPONSE_PREVIEW_MAX_CHARS);
        }
        let encoder = SaiFetchHttpConfiguration::shared(cx).read(cx).json_encoder;
        match encoder.encode(value) {
            Ok(obj) => SaiHttpTraceRecorder::pretty_json_string_from_any(Some(&obj)),
            Err(_) => std::any::type_name::<T>().to_owned(),
        }
    }

    fn memory_cache_response_preview_any<T: Debug + 'static>(value: &T) -> String {
        let any: &dyn Any = value;
        if let Some(data) = any.downcast_ref::<Vec<u8>>() {
            return SaiHttpTraceRecorder::truncate_utf8(data, RESPONSE_PREVIEW_MAX_CHARS);
        }
        format!("{value:?}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaiFetchBodyEncodingError {
    NotJsonObject,
}

impl fmt::Display for SaiFetchBodyEncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJsonObject => f.write_str("SaiFetch body is not a JSON object."),
        }
    }
}

impl std::error::Error for SaiFetchBodyEncodingError {}
