use std::sync::LazyLock;

use gpui::AssetSource;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_platform::process_info::environment;
use serde::Deserialize;

use crate::core::assets::RiseAssets;
use crate::core::lib::sai::core::sai_dev_host_override::{SaiDevHostOverride, SaiDevHostSlot};
use crate::core::stores::http::http_service::AppEnvironment;

const SOCKET_LOCAL_RESOURCE: &str = "SocketLocal.json";
const SAI_WS_URL_ENVIRONMENT_KEY: &str = "SAI_WS_URL";

#[derive(Deserialize)]
struct SocketLocalPayload {
    #[serde(rename = "webSocketURL")]
    web_socket_url: String,
}

fn literal_url(string: &str) -> Url {
    Url::from_string(string).expect("compiled-in WebSocket address parses")
}

static PRODUCTION_WEB_SOCKET_URL: LazyLock<Url> = LazyLock::new(|| literal_url("wss://api.riseonly.net/ws"));

static STAGING_WEB_SOCKET_URL: LazyLock<Url> = LazyLock::new(|| literal_url("wss://staging.riseonly.net/ws"));

static DEFAULT_DEBUG_WEB_SOCKET_URL: LazyLock<Url> = LazyLock::new(|| literal_url("ws://192.168.31.177:8085"));

static BUNDLED_WEB_SOCKET_URL: LazyLock<Option<Url>> = LazyLock::new(SaiWebSocketEnvironment::load_socket_local_json);

pub enum SaiWebSocketEnvironment {}

impl SaiWebSocketEnvironment {
    pub fn production_web_socket_url() -> Url {
        PRODUCTION_WEB_SOCKET_URL.clone()
    }

    pub fn staging_web_socket_url() -> Url {
        STAGING_WEB_SOCKET_URL.clone()
    }

    pub fn default_debug_web_socket_url() -> Url {
        DEFAULT_DEBUG_WEB_SOCKET_URL.clone()
    }

    /// debug and debug tools builds honour the host switch, release builds pin the environment socket
    pub fn resolved_web_socket_url() -> Url {
        if cfg!(any(debug_assertions, feature = "sai_debug_tools")) {
            Self::debug_web_socket_url(SaiDevHostOverride::slot())
        } else {
            AppEnvironment::current().web_socket_url()
        }
    }

    pub fn debug_web_socket_url(slot: SaiDevHostSlot) -> Url {
        if !SaiDevHostOverride::is_effective() {
            return AppEnvironment::current().web_socket_url();
        }
        match slot {
            SaiDevHostSlot::Fallback => Self::default_debug_web_socket_url(),
            SaiDevHostSlot::Staging => Self::staging_web_socket_url(),
            SaiDevHostSlot::Production => Self::production_web_socket_url(),
            SaiDevHostSlot::Custom => SaiDevHostOverride::custom_endpoint()
                .web_socket_url()
                .unwrap_or_else(Self::default_debug_web_socket_url),
            SaiDevHostSlot::Config => {
                let variables = environment();
                let env = variables
                    .get(SAI_WS_URL_ENVIRONMENT_KEY)
                    .map(|env| trimming_whitespaces_and_newlines(env))
                    .filter(|env| !env.is_empty());
                if let Some(url) = env.and_then(Url::from_string) {
                    return url;
                }
                if let Some(url) = BUNDLED_WEB_SOCKET_URL.as_ref() {
                    return url.clone();
                }
                Self::default_debug_web_socket_url()
            }
        }
    }

    pub fn resolved_web_socket_display_string() -> String {
        Self::resolved_web_socket_url().absolute_string().to_owned()
    }

    fn load_socket_local_json() -> Option<Url> {
        let data = RiseAssets::discover().load(SOCKET_LOCAL_RESOURCE).ok()??;
        let payload: SocketLocalPayload = serde_json::from_slice(&data).ok()?;
        let trimmed = trimming_whitespaces_and_newlines(&payload.web_socket_url);
        if trimmed.is_empty() {
            return None;
        }
        Url::from_string(trimmed)
    }
}
