use std::time::Duration;

use super::network_proxy::*;

pub const ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
pub const ACCESS_TYPE_NO_PROXY: u32 = 1;
pub const ACCESS_TYPE_NAMED_PROXY: u32 = 3;
pub const ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;

pub const ERROR_TIMEOUT: u32 = 12002;
pub const ERROR_LOGIN_FAILURE: u32 = 12015;
pub const ERROR_BAD_AUTO_PROXY_SCRIPT: u32 = 12166;
pub const ERROR_UNABLE_TO_DOWNLOAD_SCRIPT: u32 = 12167;
pub const ERROR_AUTO_PROXY_SERVICE_ERROR: u32 = 12178;
pub const ERROR_AUTODETECTION_FAILED: u32 = 12180;

pub const DEADLINE_GRACE: Duration = Duration::from_secs(2);

pub const SCRIPT_NOT_SUPPORTED: &str = "WinHTTP runs a proxy auto-configuration only by URL";

/// what one auto proxy lookup asks winhttp to evaluate
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AutoProxyRequest {
    Discover,
    ConfigUrl(String),
}

impl AutoProxyRequest {
    /// request for an auto config entry, a script given as text cant run
    pub fn for_config(config: &AutoConfig) -> Result<Self, ProxyUnavailable> {
        match config {
            AutoConfig::Url(url) => Ok(Self::ConfigUrl(url.clone())),
            AutoConfig::Script(_) => Err(ProxyUnavailable::AutoConfigFailed(SCRIPT_NOT_SUPPORTED.to_owned())),
        }
    }
}

/// settings winhttp reads for the signed in user
/// internet options and the settings proxy page write the same values
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct IeProxySettings {
    pub auto_detect: bool,
    pub auto_config_url: Option<String>,
    pub proxy_server: Option<String>,
    pub proxy_bypass: Option<String>,
}

impl IeProxySettings {
    /// the four values as winhttp returns them, blank strings count as absent
    pub fn new(
        auto_detect: bool,
        auto_config_url: Option<String>,
        proxy_server: Option<String>,
        proxy_bypass: Option<String>,
    ) -> Self {
        let present = |value: Option<String>| {
            value
                .map(|text| text.trim_ascii().to_owned())
                .filter(|text| !text.is_empty())
        };
        Self {
            auto_detect,
            auto_config_url: present(auto_config_url),
            proxy_server: present(proxy_server),
            proxy_bypass: present(proxy_bypass),
        }
    }

    /// manual servers with the exception list
    pub fn manual(&self) -> ManualProxies {
        ManualProxies::from_windows_settings(
            self.proxy_server.as_deref().unwrap_or_default(),
            self.proxy_bypass.as_deref().unwrap_or_default(),
        )
    }

    /// answer after automatic detection, the setup script then manual servers (or direct)
    /// wpad itself is run by the caller before these entries
    pub fn configured_entries(&self, target: &ProxyTarget) -> Vec<SystemProxyEntry> {
        let mut entries = Vec::new();
        if let Some(url) = &self.auto_config_url {
            entries.push(SystemProxyEntry::AutoConfig(AutoConfig::Url(url.clone())));
        }
        let manual = self.manual();
        if manual.is_empty() {
            entries.push(SystemProxyEntry::Direct);
        } else {
            entries.extend(manual.entries_for(target));
        }
        entries
    }
}

/// ordered answer of one get proxy for url call
/// named answer is the pac proxy list converted by winhttp
/// any other type or an unusable server means direct
pub fn entries_from_answer(
    access_type: u32,
    proxy_list: Option<&str>,
    target: &ProxyTarget,
) -> Vec<SystemProxyEntry> {
    if access_type != ACCESS_TYPE_NAMED_PROXY {
        return vec![SystemProxyEntry::Direct];
    }
    let own_scheme = target.scheme.lookup_scheme();
    let entries: Vec<SystemProxyEntry> = proxy_list
        .unwrap_or_default()
        .split(|character: char| character == ';' || character.is_whitespace())
        .map(str::trim_ascii)
        .filter(|part| !part.is_empty())
        .filter_map(|part| match part.split_once('=') {
            Some((scheme, server)) => {
                let scheme = scheme.trim_ascii().to_ascii_lowercase();
                if scheme == "socks" {
                    parse_proxy_url(&format!("socks5://{}", server.trim_ascii()), 1080)
                } else if scheme == own_scheme {
                    parse_proxy_url(server, 80)
                } else {
                    None
                }
            }
            None => parse_proxy_url(part, 80),
        })
        .map(SystemProxyEntry::Proxy)
        .collect();
    if entries.is_empty() {
        vec![SystemProxyEntry::Direct]
    } else {
        entries
    }
}

/// win32 code inside an hresult made by the win32 macro (facility 7)
pub const fn win32_code(hresult: i32) -> Option<u32> {
    let bits = hresult as u32;
    if bits & 0xFFFF_0000 == 0x8007_0000 {
        Some(bits & 0xFFFF)
    } else {
        None
    }
}

/// the seam failure for a failed auto proxy lookup
pub fn failure_from_code(code: Option<u32>, message: &str, timeout: Duration) -> ProxyUnavailable {
    match code {
        Some(ERROR_TIMEOUT) => ProxyUnavailable::AutoConfigTimedOut(timeout),
        Some(
            code @ (ERROR_AUTODETECTION_FAILED
            | ERROR_UNABLE_TO_DOWNLOAD_SCRIPT
            | ERROR_BAD_AUTO_PROXY_SCRIPT
            | ERROR_AUTO_PROXY_SERVICE_ERROR
            | ERROR_LOGIN_FAILURE),
        ) => ProxyUnavailable::AutoConfigFailed(format!("WinHTTP error {code}: {message}")),
        Some(code) => ProxyUnavailable::SystemRefused(format!("WinHTTP error {code}: {message}")),
        None => ProxyUnavailable::SystemRefused(message.to_owned()),
    }
}

/// timeout in whole ms for winhttp, at least 1 and at most i32 max
pub fn timeout_millis(timeout: Duration) -> i32 {
    i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX).max(1)
}
