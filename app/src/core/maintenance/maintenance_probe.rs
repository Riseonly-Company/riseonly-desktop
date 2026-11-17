use std::time::Duration;

use futures::AsyncReadExt;
use futures::future::{Either, select};
use gpui::http_client::http::header::ACCEPT_LANGUAGE;
use gpui::http_client::http::{HeaderValue, Method, StatusCode, Uri};
use gpui::http_client::{AsyncBody, Request};
use gpui::{App, Task};
use rise_canon::rise_key::auth::DEVICE_LANGUAGES;
use rise_canon::rise_maintenance::{
    MediaKind, POLL_PROBE_TIMEOUT_MS, STATUS_METHOD, STATUS_PATH, media_kind_from_wire,
    state_from_wire, status_field,
};
use rise_core::foundation::foundation_url::{UrlComponents, UrlQueryItem};
use serde_json::{Map, Value};

use crate::core::stores::http::http_service::HttpService;

/// the one call an app refused everything else may still make
/// plain http outside the engine, runs without an account or socket
/// no answer means app stays as it was, no cache means open
pub enum MaintenanceProbe {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaintenanceProbeReading {
    pub is_closed: bool,
    pub revision: i64,
    pub started_at_ms: i64,
    pub ends_at_ms: i64,
    pub server_time_ms: i64,
    pub retry_after_ms: i64,
    pub locale: String,
    pub title: String,
    pub body: String,
    pub media_kind: MediaKind,
    pub media_ref: String,
}

impl MaintenanceProbeReading {
    pub const OPEN: MaintenanceProbeReading = MaintenanceProbeReading {
        is_closed: false,
        revision: 0,
        started_at_ms: 0,
        ends_at_ms: 0,
        server_time_ms: 0,
        retry_after_ms: 0,
        locale: String::new(),
        title: String::new(),
        body: String::new(),
        media_kind: MediaKind::None,
        media_ref: String::new(),
    };
}

impl MaintenanceProbe {
    /// reads the status, none if no answer came (timeout or non 200 or bad json)
    /// the caller keeps what it believed
    pub fn read(languages: &[&str], cx: &mut App) -> Task<Option<MaintenanceProbeReading>> {
        let endpoint = HttpService::absolute_url(STATUS_PATH);
        let Some(mut components) = UrlComponents::from_url(&endpoint, false) else {
            return Task::ready(None);
        };
        let joined_languages = languages.join(",");
        if !languages.is_empty() {
            components.set_query_items(Some(vec![UrlQueryItem::new(
                DEVICE_LANGUAGES,
                Some(joined_languages.clone()),
            )]));
        }
        let Some(url) = components.url() else {
            return Task::ready(None);
        };
        let Ok(uri) = url.absolute_string().parse::<Uri>() else {
            return Task::ready(None);
        };
        let Ok(method) = Method::from_bytes(STATUS_METHOD.as_bytes()) else {
            return Task::ready(None);
        };

        let mut request = Request::new(AsyncBody::empty());
        *request.method_mut() = method;
        *request.uri_mut() = uri;
        if !languages.is_empty() {
            let Ok(accept_language) = HeaderValue::from_str(&joined_languages) else {
                return Task::ready(None);
            };
            request.headers_mut().insert(ACCEPT_LANGUAGE, accept_language);
        }

        let client = cx.http_client();
        let timer = cx.background_executor().timer(Duration::from_millis(POLL_PROBE_TIMEOUT_MS));
        cx.spawn(async move |_cx| {
            let response = match select(client.send(request), timer).await {
                Either::Left((Ok(response), _)) => response,
                Either::Left((Err(_), _)) | Either::Right(_) => return None,
            };
            if response.status() != StatusCode::OK {
                return None;
            }
            let mut data = Vec::new();
            let mut body = response.into_body();
            body.read_to_end(&mut data).await.ok()?;
            Self::reading_from_json(&data)
        })
    }

    fn reading_from(payload: &Map<String, Value>) -> MaintenanceProbeReading {
        let state = Self::string_field(payload, status_field::STATE).map(state_from_wire);
        let media_kind = Self::string_field(payload, status_field::MEDIA_KIND)
            .map_or(MediaKind::None, media_kind_from_wire);

        MaintenanceProbeReading {
            is_closed: state.is_some_and(|state| state.blocks_traffic()),
            revision: Self::integer_field(payload, status_field::REVISION).unwrap_or(0),
            started_at_ms: Self::integer_field(payload, status_field::STARTED_AT_MS).unwrap_or(0),
            ends_at_ms: Self::integer_field(payload, status_field::ENDS_AT_MS).unwrap_or(0),
            server_time_ms: Self::integer_field(payload, status_field::SERVER_TIME_MS).unwrap_or(0),
            retry_after_ms: Self::integer_field(payload, status_field::RETRY_AFTER_MS).unwrap_or(0),
            locale: Self::string_field(payload, status_field::LOCALE).unwrap_or_default().to_owned(),
            title: Self::string_field(payload, status_field::TITLE).unwrap_or_default().to_owned(),
            body: Self::string_field(payload, status_field::BODY).unwrap_or_default().to_owned(),
            media_kind,
            media_ref: Self::string_field(payload, status_field::MEDIA_REF)
                .unwrap_or_default()
                .to_owned(),
        }
    }

    /// maps json bytes to a reading, split out so it can be tested without a network
    pub fn reading_from_json(data: &[u8]) -> Option<MaintenanceProbeReading> {
        let payload = serde_json::from_slice::<Value>(data).ok()?;
        payload.as_object().map(Self::reading_from)
    }

    /// string field, a value of the wrong type reads as absent instead of failing the whole status
    fn string_field<'a>(payload: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
        payload.get(key).and_then(Value::as_str)
    }

    /// int64 field, an integral double like 7.0 is accepted too
    fn integer_field(payload: &Map<String, Value>, key: &str) -> Option<i64> {
        let value = payload.get(key)?;
        if let Some(integer) = value.as_i64() {
            return Some(integer);
        }
        let double = value.as_f64()?;
        let in_range = double >= i64::MIN as f64 && double < i64::MAX as f64;
        (double.fract() == 0.0 && in_range).then_some(double as i64)
    }
}
