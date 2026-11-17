use std::sync::LazyLock;

use parking_lot::Mutex;
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{
    SAI_DEV_HOST_CUSTOM_API, SAI_DEV_HOST_CUSTOM_WS, SAI_DEV_HOST_LOCAL_SLOT, SAI_DEV_HOST_SLOT,
};

use crate::core::stores::http::http_service::AppEnvironment;

const SCHEME_SEPARATOR: &str = "://";
const PATH_DELIMITER: char = '/';
const HTTPS_SCHEME: &str = "https";
const HTTP_SCHEME: &str = "http";
const WSS_SCHEME: &str = "wss";
const WS_SCHEME: &str = "ws";
const DEFAULT_API_PATH: &str = "/api";
const WEB_SOCKET_PATH: &str = "/ws";

/// which backend a debug build talks to
/// config is the last set-dev-socket-url.sh write, fallback is the compiled in one
/// custom is what the host panel stored
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiDevHostSlot {
    Config,
    Fallback,
    Staging,
    Production,
    Custom,
}

impl SaiDevHostSlot {
    pub const ALL_CASES: [Self; 5] = [Self::Config, Self::Fallback, Self::Staging, Self::Production, Self::Custom];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Fallback => "fallback",
            Self::Staging => "staging",
            Self::Production => "production",
            Self::Custom => "custom",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|slot| slot.raw_value() == raw)
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Config => "Local dev \u{b7} Config JSON",
            Self::Fallback => "Local dev \u{b7} Code fallback",
            Self::Staging => "Staging",
            Self::Production => "Production",
            Self::Custom => "Custom URL",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self {
            Self::Config => "ApiLocal.json / SocketLocal.json \u{2014} set-dev-socket-url.sh",
            Self::Fallback => "Hardcoded in http-service.swift / SaiWebSocketEnvironment.swift",
            Self::Staging => "staging.riseonly.net, the backend the staging TestFlight build talks to",
            Self::Production => "api.riseonly.net, the live backend real accounts live on",
            Self::Custom => "Typed in the Host panel; WS is derived from API when left empty",
        }
    }

    /// true if writes reach real people
    /// the switch asks before applying even when theres no session to lose
    pub fn is_live(&self) -> bool {
        *self == Self::Production
    }

    /// coarse bucket the header selector shows
    /// both local slots are dev and a custom address belongs to none
    pub fn environment(&self) -> Option<SaiDevHostEnvironment> {
        match self {
            Self::Config | Self::Fallback => Some(SaiDevHostEnvironment::Dev),
            Self::Staging => Some(SaiDevHostEnvironment::Staging),
            Self::Production => Some(SaiDevHostEnvironment::Production),
            Self::Custom => None,
        }
    }
}

/// three way selector in the debugger header
/// dev resolves to the local slot used last so dev to staging and back lands on the same address
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiDevHostEnvironment {
    Dev,
    Staging,
    Production,
}

impl SaiDevHostEnvironment {
    pub const ALL_CASES: [Self; 3] = [Self::Dev, Self::Staging, Self::Production];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Staging => "staging",
            Self::Production => "production",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|environment| environment.raw_value() == raw)
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Dev => "Dev",
            Self::Staging => "Staging",
            Self::Production => "Prod",
        }
    }

    pub fn slot(&self) -> SaiDevHostSlot {
        match self {
            Self::Dev => SaiDevHostOverride::last_local_slot(),
            Self::Staging => SaiDevHostSlot::Staging,
            Self::Production => SaiDevHostSlot::Production,
        }
    }
}

/// custom slot keeps both addresses as typed text so the panel shows back what was entered
/// even if it doesnt parse, the ws address is optional for a host serving both from one origin
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaiDevHostCustomEndpoint {
    pub api_text: String,
    pub web_socket_text: String,
}

impl SaiDevHostCustomEndpoint {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn api_url(&self) -> Option<Url> {
        Self::normalized(&self.api_text, HTTPS_SCHEME, Some(DEFAULT_API_PATH))
    }

    /// a typed ws address keeps its path exactly as entered
    /// the local gateway listens on a bare ws://host:8085 with no path
    /// so a default path would break local slots
    pub fn web_socket_url(&self) -> Option<Url> {
        if let Some(explicit) = Self::normalized(&self.web_socket_text, WSS_SCHEME, None) {
            return Some(explicit);
        }
        Self::derived_web_socket_url(self.api_url().as_ref())
    }

    fn normalized(raw: &str, default_scheme: &str, default_path: Option<&str>) -> Option<Url> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        if trimmed.is_empty() {
            return None;
        }

        let with_scheme = if trimmed.contains(SCHEME_SEPARATOR) {
            trimmed.to_owned()
        } else {
            format!("{default_scheme}{SCHEME_SEPARATOR}{trimmed}")
        };
        let mut components = UrlComponents::from_string(&with_scheme)?;
        if components.host().is_none_or(|host| host.is_empty()) {
            return None;
        }

        let raw_path = components.path();
        let path = trimming_characters(&raw_path, |c| c == PATH_DELIMITER);
        let normalized_path = if path.is_empty() {
            default_path.unwrap_or("").to_owned()
        } else {
            format!("{PATH_DELIMITER}{path}")
        };
        components.set_path(&normalized_path);
        components.set_query(None);
        components.set_fragment(None);
        components.url()
    }

    fn derived_web_socket_url(api: Option<&Url>) -> Option<Url> {
        let api = api?;
        let mut components = UrlComponents::from_url(api, false)?;
        let scheme = if api.scheme().is_some_and(|scheme| scheme.to_lowercase() == HTTP_SCHEME) {
            WS_SCHEME
        } else {
            WSS_SCHEME
        };
        let socket_path = WEB_SOCKET_PATH;
        components.set_scheme(Some(scheme));
        components.set_path(socket_path);
        components.set_query(None);
        components.set_fragment(None);
        components.url()
    }
}

static CACHED_SLOT: LazyLock<Mutex<Option<SaiDevHostSlot>>> = LazyLock::new(|| Mutex::new(None));
static CACHED_CUSTOM: LazyLock<Mutex<Option<SaiDevHostCustomEndpoint>>> = LazyLock::new(|| Mutex::new(None));

pub enum SaiDevHostOverride {}

impl SaiDevHostOverride {
    pub const DEFAULTS_KEY: &'static str = SAI_DEV_HOST_SLOT;
    pub const CUSTOM_API_DEFAULTS_KEY: &'static str = SAI_DEV_HOST_CUSTOM_API;
    pub const CUSTOM_WEB_SOCKET_DEFAULTS_KEY: &'static str = SAI_DEV_HOST_CUSTOM_WS;
    pub const LOCAL_SLOT_DEFAULTS_KEY: &'static str = SAI_DEV_HOST_LOCAL_SLOT;

    /// the switch exists only for the local dev environment in debug or debug tools builds
    /// anywhere else resolvers behave as before
    pub fn is_effective() -> bool {
        cfg!(any(debug_assertions, feature = "sai_debug_tools"))
            && AppEnvironment::current() == AppEnvironment::Dev
    }

    pub fn slot() -> SaiDevHostSlot {
        if !Self::is_effective() {
            return SaiDevHostSlot::Config;
        }

        let mut cached = CACHED_SLOT.lock();
        if let Some(cached) = *cached {
            return cached;
        }
        let stored = UserDefaults::standard()
            .string(Self::DEFAULTS_KEY)
            .and_then(|raw| SaiDevHostSlot::from_raw(&raw))
            .unwrap_or(SaiDevHostSlot::Config);
        *cached = Some(stored);
        stored
    }

    pub fn custom_endpoint() -> SaiDevHostCustomEndpoint {
        let mut cached = CACHED_CUSTOM.lock();
        if let Some(cached) = cached.as_ref() {
            return cached.clone();
        }
        let defaults = UserDefaults::standard();
        let stored = SaiDevHostCustomEndpoint {
            api_text: defaults.string(Self::CUSTOM_API_DEFAULTS_KEY).unwrap_or_default(),
            web_socket_text: defaults.string(Self::CUSTOM_WEB_SOCKET_DEFAULTS_KEY).unwrap_or_default(),
        };
        *cached = Some(stored.clone());
        stored
    }

    pub fn select(slot: SaiDevHostSlot) {
        *CACHED_SLOT.lock() = Some(slot);
        let defaults = UserDefaults::standard();
        defaults.set(Self::DEFAULTS_KEY, slot.raw_value());
        if slot.environment() == Some(SaiDevHostEnvironment::Dev) {
            defaults.set(Self::LOCAL_SLOT_DEFAULTS_KEY, slot.raw_value());
        }
    }

    /// local slot the header dev segment goes back to, read once per tap
    pub fn last_local_slot() -> SaiDevHostSlot {
        UserDefaults::standard()
            .string(Self::LOCAL_SLOT_DEFAULTS_KEY)
            .and_then(|raw| SaiDevHostSlot::from_raw(&raw))
            .filter(|stored| stored.environment() == Some(SaiDevHostEnvironment::Dev))
            .unwrap_or(SaiDevHostSlot::Config)
    }

    pub fn store_custom_endpoint(endpoint: SaiDevHostCustomEndpoint) {
        let defaults = UserDefaults::standard();
        defaults.set(Self::CUSTOM_API_DEFAULTS_KEY, endpoint.api_text.as_str());
        defaults.set(Self::CUSTOM_WEB_SOCKET_DEFAULTS_KEY, endpoint.web_socket_text.as_str());
        *CACHED_CUSTOM.lock() = Some(endpoint);
    }
}
