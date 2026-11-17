use std::time::Duration;

use futures::AsyncReadExt;
use futures::future::{Either, select};
use gpui::http_client::http::header::ACCEPT_LANGUAGE;
use gpui::http_client::http::{HeaderValue, StatusCode, Uri};
use gpui::http_client::{AsyncBody, Request};
use gpui::{App, AsyncApp};
use rise_canon::RiseTimingCanon;
use rise_canon::rise_blob::bootstrap::Locale;
use rise_canon::rise_http_path::bootstrap::LOCALE;
use rise_canon::rise_key::auth::{DEVICE_LANGUAGES, DEVICE_REGION, TIMEZONE};
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::{UrlComponents, UrlQueryItem};
use rise_i18n::AppLanguageCatalog;
use rise_i18n::language_store::LanguageStore;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{BOOTSTRAP_LOCALE_FETCHED_AT, BOOTSTRAP_LOCALE_PAYLOAD};
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::stores::http::http_service::HttpService;

/// pre auth region and language negotiation so a new users first screen is in their language
/// no account yet so plain http not the engine, hard timeout and user choice always wins
pub enum LocaleBootstrap {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocaleBootstrapResult {
    pub region: String,
    pub suggested_language: String,
    pub device_languages: Option<Vec<String>>,
}

impl LocaleBootstrap {
    const TIMEOUT: Duration = Duration::from_millis(800);

    fn cache_ttl() -> f64 {
        RiseTimingCanon::freshness_seconds("locale_bootstrap")
    }

    pub fn resolve_and_apply(cx: &mut App) {
        if let Some(cached) = Self::cached_result()
            && Self::was_negotiated_for_current_device(&cached)
        {
            Self::apply(&cached, cx);
            return;
        }

        let Some(request) = Self::request() else {
            return;
        };
        let client = cx.http_client();
        let timer = cx.background_executor().timer(Self::TIMEOUT);
        let preferences = Self::device_preferences();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let response = match select(client.send(request), timer).await {
                Either::Left((Ok(response), _)) => response,
                Either::Left((Err(_), _)) | Either::Right(_) => return,
            };
            if response.status() != StatusCode::OK {
                return;
            }
            let mut data = Vec::new();
            let mut body = response.into_body();
            if body.read_to_end(&mut data).await.is_err() {
                return;
            }
            let Some(result) = Self::result_from(&data, preferences) else {
                return;
            };
            cx.update(|cx| Self::apply(&result, cx));
            Self::cache(&result);
        })
        .detach();
    }

    fn apply(result: &LocaleBootstrapResult, cx: &mut App) {
        LanguageStore::update(cx, |store, cx| {
            store.apply_bootstrap(Some(result.region.as_str()), Some(result.suggested_language.as_str()), cx);
        });
    }

    fn device_preferences() -> Vec<String> {
        AppLanguageCatalog::device_preferences().into_iter().map(str::to_owned).collect()
    }

    fn was_negotiated_for_current_device(cached: &LocaleBootstrapResult) -> bool {
        cached.device_languages.as_deref() == Some(Self::device_preferences().as_slice())
    }

    fn cached_result() -> Option<LocaleBootstrapResult> {
        let defaults = UserDefaults::standard();
        let stamp = defaults.double(BOOTSTRAP_LOCALE_FETCHED_AT);
        if stamp <= 0.0 || RiseClock::now().time_interval_since_1970() - stamp >= Self::cache_ttl() {
            return None;
        }
        let data = defaults.data(BOOTSTRAP_LOCALE_PAYLOAD)?;
        serde_json::from_slice(&data).ok()
    }

    fn cache(result: &LocaleBootstrapResult) {
        let Ok(data) = serde_json::to_vec(result) else {
            return;
        };
        let defaults = UserDefaults::standard();
        defaults.set(BOOTSTRAP_LOCALE_PAYLOAD, data);
        defaults.set(BOOTSTRAP_LOCALE_FETCHED_AT, RiseClock::now().time_interval_since_1970());
    }

    fn request() -> Option<Request<AsyncBody>> {
        let endpoint = HttpService::absolute_url(LOCALE);
        let mut components = UrlComponents::from_url(&endpoint, false)?;

        let preferences = Self::device_preferences().join(",");
        let mut items = Vec::new();
        if !preferences.is_empty() {
            items.push(UrlQueryItem::new(DEVICE_LANGUAGES, Some(preferences.clone())));
        }
        if let Some(region) = rise_platform::device_locale::current().region {
            items.push(UrlQueryItem::new(DEVICE_REGION, Some(region)));
        }
        items.push(UrlQueryItem::new(TIMEZONE, Some(TimeZone::current().identifier())));
        components.set_query_items(Some(items));

        let uri = components.url()?.absolute_string().parse::<Uri>().ok()?;
        let mut request = Request::new(AsyncBody::empty());
        *request.uri_mut() = uri;
        if !preferences.is_empty() {
            let accept_language = HeaderValue::from_str(&preferences).ok()?;
            request.headers_mut().insert(ACCEPT_LANGUAGE, accept_language);
        }
        Some(request)
    }

    fn result_from(data: &[u8], preferences: Vec<String>) -> Option<LocaleBootstrapResult> {
        let reply: Locale = serde_json::from_slice(data).ok()?;
        Some(LocaleBootstrapResult {
            region: reply.region?,
            suggested_language: reply.suggested_language?,
            device_languages: Some(preferences),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_without_a_region_or_a_language_answers_nothing() {
        assert_eq!(LocaleBootstrap::result_from(br#"{"region":"KZ"}"#, Vec::new()), None);
        assert_eq!(LocaleBootstrap::result_from(br#"{"suggested_language":"kk"}"#, Vec::new()), None);
        assert_eq!(LocaleBootstrap::result_from(b"not json", Vec::new()), None);
    }

    #[test]
    fn a_reply_keeps_the_language_list_it_was_negotiated_for() {
        let result = LocaleBootstrap::result_from(
            br#"{"region":"KZ","suggested_language":"kk"}"#,
            vec!["kk".to_owned(), "ru".to_owned()],
        );
        assert_eq!(
            result,
            Some(LocaleBootstrapResult {
                region: "KZ".to_owned(),
                suggested_language: "kk".to_owned(),
                device_languages: Some(vec!["kk".to_owned(), "ru".to_owned()]),
            })
        );
    }

    #[test]
    fn the_cached_payload_keeps_the_keys_ios_writes() {
        let result = LocaleBootstrapResult {
            region: "KZ".to_owned(),
            suggested_language: "kk".to_owned(),
            device_languages: None,
        };
        let encoded = serde_json::to_string(&result).expect("the result encodes");
        assert!(encoded.contains("\"suggestedLanguage\""));
        assert!(encoded.contains("\"deviceLanguages\""));
        assert_eq!(serde_json::from_str::<LocaleBootstrapResult>(&encoded).expect("the result decodes"), result);
    }
}
