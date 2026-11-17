use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::http_client::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use gpui::http_client::http::{HeaderValue, Request};
use gpui::{App, AssetSource, Context, Entity, Task};
use parking_lot::Mutex;
use rise_canon::rise_http_path;
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_platform::process_info::environment;
use serde::Deserialize;

pub use crate::core::config::app_environment::AppEnvironment;
use crate::core::assets::RiseAssets;
use crate::core::config::endpoints::Endpoints;
use crate::core::lib::sai::core::sai_dev_host_override::{
    SaiDevHostEnvironment, SaiDevHostOverride, SaiDevHostSlot,
};
use crate::core::lib::sai::core::websocket::sai_web_socket_environment::SaiWebSocketEnvironment;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch::SaiFetch;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::{
    SaiFetchJsonDecoder, SaiFetchJsonEncoder, SaiFetchKeyEncodingStrategy,
    SaiFetchRequestInterceptor,
};
use crate::core::lib::sai::sai_auth_hooks::sai_auth_access_token;

const API_LOCAL_RESOURCE: &str = "ApiLocal.json";
const SAI_API_URL_ENVIRONMENT_KEY: &str = "SAI_API_URL";
const ACCEPT_VALUE: &str = "application/json";
const CONTENT_TYPE_VALUE: &str = "application/json; charset=utf-8";

fn literal_url(string: &str) -> Url {
    Url::from_string(string).expect("compiled-in API address parses")
}

static PRODUCTION_API_BASE_URL: LazyLock<Url> =
    LazyLock::new(|| literal_url("https://api.riseonly.net/api"));

static STAGING_API_BASE_URL: LazyLock<Url> =
    LazyLock::new(|| literal_url("https://staging.riseonly.net/api"));

static DEFAULT_DEBUG_API_BASE_URL: LazyLock<Url> =
    LazyLock::new(|| literal_url("http://192.168.31.177:8080/api"));

static CURRENT_APP_ENVIRONMENT: LazyLock<AppEnvironment> = LazyLock::new(AppEnvironment::compiled);

static DEBUG_CACHED_API_BASE_URL: Mutex<Option<Url>> = Mutex::new(None);

static RELEASE_CACHED_API_BASE_URL: LazyLock<Url> = LazyLock::new(HttpService::compute_api_base_url);

static BUNDLED_API_BASE_URL: LazyLock<Option<Url>> = LazyLock::new(HttpService::load_api_local_json);

static LOCAL_BACKEND_ORIGINS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    HashSet::from([
        HttpService::origin(&HttpService::debug_api_base_url(SaiDevHostSlot::Config)),
        HttpService::origin(&HttpService::debug_api_base_url(SaiDevHostSlot::Fallback)),
    ])
});

impl AppEnvironment {
    pub fn current() -> AppEnvironment {
        *CURRENT_APP_ENVIRONMENT
    }

    pub fn api_base_url(&self) -> Url {
        Self::info_url(&Endpoints::compiled_for(*self).api_base_url)
            .unwrap_or_else(|| PRODUCTION_API_BASE_URL.clone())
    }

    pub fn web_socket_url(&self) -> Url {
        Self::info_url(&Endpoints::compiled_for(*self).ws_url)
            .unwrap_or_else(SaiWebSocketEnvironment::production_web_socket_url)
    }

    fn info_url(key: &str) -> Option<Url> {
        let raw = trimming_whitespaces_and_newlines(key);
        if raw.is_empty() {
            return None;
        }
        Url::from_string(raw)
    }
}

#[derive(Deserialize)]
struct ApiLocalConfig {
    #[serde(rename = "apiBaseURL")]
    api_base_url: String,
}

pub struct HttpService {}

impl HttpService {
    pub const LOCAL_BACKEND_ID: &'static str = "local";

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn production_api_base_url() -> Url {
        PRODUCTION_API_BASE_URL.clone()
    }

    pub fn staging_api_base_url() -> Url {
        STAGING_API_BASE_URL.clone()
    }

    pub fn default_debug_api_base_url() -> Url {
        DEFAULT_DEBUG_API_BASE_URL.clone()
    }

    pub fn configure(&mut self, cx: &mut App) {
        let decoder = SaiFetchJsonDecoder;

        let encoder = SaiFetchJsonEncoder {
            key_encoding_strategy: SaiFetchKeyEncodingStrategy::ConvertToSnakeCase,
        };

        let attach_bearer: SaiFetchRequestInterceptor =
            Rc::new(|mut req: Request<Vec<u8>>, cx: &mut App| -> Task<anyhow::Result<Request<Vec<u8>>>> {
                let url = Url::from_string(&req.uri().to_string());
                if !Self::request_should_attach_bearer(url.as_ref()) {
                    return Task::ready(Ok(req));
                }
                if let Some(token) = sai_auth_access_token(cx)
                    && let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}"))
                {
                    req.headers_mut().insert(AUTHORIZATION, value);
                }
                Task::ready(Ok(req))
            });

        SaiFetch::configure_http(
            Some(Self::resolved_api_base_url()),
            HashMap::from([
                (ACCEPT.as_str().to_owned(), ACCEPT_VALUE.to_owned()),
                (CONTENT_TYPE.as_str().to_owned(), CONTENT_TYPE_VALUE.to_owned()),
            ]),
            Some(decoder),
            Some(encoder),
            vec![attach_bearer],
            Vec::new(),
            cx,
        );
    }

    const fn debug_tools() -> bool {
        cfg!(any(debug_assertions, feature = "sai_debug_tools"))
    }

    pub fn resolved_api_base_url() -> Url {
        if !Self::debug_tools() {
            return RELEASE_CACHED_API_BASE_URL.clone();
        }
        let mut cached = DEBUG_CACHED_API_BASE_URL.lock();
        if let Some(cached) = cached.as_ref() {
            return cached.clone();
        }
        let resolved = Self::compute_api_base_url();
        *cached = Some(resolved.clone());
        resolved
    }

    pub fn invalidate_resolved_api_base_url() {
        *DEBUG_CACHED_API_BASE_URL.lock() = None;
    }

    pub fn debug_api_base_url(slot: SaiDevHostSlot) -> Url {
        if !Self::debug_tools() {
            return AppEnvironment::current().api_base_url();
        }
        if !SaiDevHostOverride::is_effective() {
            return AppEnvironment::current().api_base_url();
        }
        match slot {
            SaiDevHostSlot::Fallback => Self::default_debug_api_base_url(),
            SaiDevHostSlot::Staging => Self::staging_api_base_url(),
            SaiDevHostSlot::Production => Self::production_api_base_url(),
            SaiDevHostSlot::Custom => SaiDevHostOverride::custom_endpoint()
                .api_url()
                .unwrap_or_else(Self::default_debug_api_base_url),
            SaiDevHostSlot::Config => {
                let variables = environment();
                let env = variables
                    .get(SAI_API_URL_ENVIRONMENT_KEY)
                    .map(|env| trimming_whitespaces_and_newlines(env))
                    .filter(|env| !env.is_empty());
                if let Some(url) = env.and_then(Url::from_string) {
                    return url;
                }
                if let Some(url) = BUNDLED_API_BASE_URL.as_ref() {
                    return url.clone();
                }
                Self::default_debug_api_base_url()
            }
        }
    }

    fn compute_api_base_url() -> Url {
        if Self::debug_tools() {
            Self::debug_api_base_url(SaiDevHostOverride::slot())
        } else {
            AppEnvironment::current().api_base_url()
        }
    }

    pub fn resolved_api_origin() -> String {
        Self::origin(&Self::resolved_api_base_url())
    }

    pub fn resolved_backend_id() -> String {
        if Self::debug_tools()
            && SaiDevHostOverride::is_effective()
            && SaiDevHostOverride::slot().environment() == Some(SaiDevHostEnvironment::Dev)
        {
            return Self::LOCAL_BACKEND_ID.to_owned();
        }
        Self::canonical_backend_id(&Self::resolved_api_origin())
    }

    pub fn canonical_backend_id(backend_id: &str) -> String {
        if Self::debug_tools()
            && SaiDevHostOverride::is_effective()
            && LOCAL_BACKEND_ORIGINS.contains(backend_id)
        {
            return Self::LOCAL_BACKEND_ID.to_owned();
        }
        backend_id.to_owned()
    }

    pub fn unstamped_backend_id() -> String {
        if Self::debug_tools() && SaiDevHostOverride::is_effective() {
            return Self::LOCAL_BACKEND_ID.to_owned();
        }
        Self::resolved_api_origin()
    }

    pub fn origin(of: &Url) -> String {
        let scheme = of.scheme().map(str::to_lowercase).unwrap_or_default();
        let host = of.host().map(|host| host.to_lowercase()).unwrap_or_default();
        let port = of.port().map(|port| port.to_string()).unwrap_or_default();
        format!("{scheme}://{host}:{port}")
    }

    pub fn api_route(canon_path: &str) -> String {
        if canon_path.starts_with("/api/") {
            canon_path["/api".len()..].to_owned()
        } else {
            canon_path.to_owned()
        }
    }

    pub fn absolute_url(path: &str) -> Url {
        let base = Self::resolved_api_base_url();
        let normalized = format!("/{}", trimming_characters(path, |character| character == '/'));
        let Some(mut components) = UrlComponents::from_url(&base, false) else {
            return base.appending_path_component(&normalized);
        };
        components.set_path(&normalized);
        components.set_query(None);
        components.set_fragment(None);
        components
            .url()
            .unwrap_or_else(|| base.appending_path_component(&normalized))
    }

    pub fn request_should_attach_bearer(url: Option<&Url>) -> bool {
        let Some(path) = url.map(|url| url.path().to_lowercase()) else {
            return true;
        };
        let public_markers = [
            rise_http_path::auth::LOGIN,
            rise_http_path::auth::REGISTER,
            rise_http_path::auth::REFRESH,
            rise_http_path::auth::SEND_CODE,
            rise_http_path::auth::CHECK_TAG,
        ]
        .map(Self::api_route);
        for m in &public_markers {
            if path.contains(m.as_str()) {
                return false;
            }
        }
        true
    }

    fn load_api_local_json() -> Option<Url> {
        let data = RiseAssets::discover().load(API_LOCAL_RESOURCE).ok()??;
        let payload: ApiLocalConfig = serde_json::from_slice(&data).ok()?;
        let trimmed = trimming_whitespaces_and_newlines(&payload.api_base_url);
        if trimmed.is_empty() {
            return None;
        }
        Url::from_string(trimmed)
    }
}
