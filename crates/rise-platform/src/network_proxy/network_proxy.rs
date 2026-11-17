use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;
use std::time::Duration;

use parking_lot::Mutex;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::system_activity::ActivityWatch;

pub const AUTO_CONFIG_TIMEOUT: Duration = Duration::from_secs(10);

/// scheme of the connection about to open
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TargetScheme {
    Http,
    Https,
    Ws,
    Wss,
}

impl TargetScheme {
    /// parses http https ws or wss ignoring case, anything else has no proxy
    pub fn parse(scheme: &str) -> Option<Self> {
        match scheme.to_ascii_lowercase().as_str() {
            "http" => Some(Self::Http),
            "https" => Some(Self::Https),
            "ws" => Some(Self::Ws),
            "wss" => Some(Self::Wss),
            _ => None,
        }
    }

    /// tls end to end, such a connection only goes through a proxy as a connect tunnel
    pub const fn is_secure(self) -> bool {
        matches!(self, Self::Https | Self::Wss)
    }

    pub const fn default_port(self) -> u16 {
        if self.is_secure() { 443 } else { 80 }
    }

    /// scheme the system lookup is made for
    pub const fn lookup_scheme(self) -> &'static str {
        if self.is_secure() { "https" } else { "http" }
    }
}

/// one destination, what the system lookup is keyed by
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ProxyTarget {
    pub scheme: TargetScheme,
    pub host: String,
    pub port: u16,
}

impl ProxyTarget {
    /// port none means the default port of the scheme
    pub fn new(scheme: TargetScheme, host: &str, port: Option<u16>) -> Self {
        let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        Self {
            scheme,
            host,
            port: port.unwrap_or_else(|| scheme.default_port()),
        }
    }

    /// url given to the system lookup, scheme host and port with an empty path
    /// ipv6 hosts are bracketed
    pub fn lookup_url(&self) -> String {
        let host = if self.host.parse::<Ipv6Addr>().is_ok() {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("{}://{}:{}/", self.scheme.lookup_scheme(), host, self.port)
    }
}

/// what a proxy server speaks
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ProxyProtocol {
    Http,
    Socks5,
}

/// user and password the system keeps for a proxy
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProxyCredentials {
    pub user: String,
    pub password: String,
}

impl std::fmt::Debug for ProxyCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProxyCredentials")
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ProxyEndpoint {
    pub protocol: ProxyProtocol,
    pub host: String,
    pub port: u16,
    pub credentials: Option<ProxyCredentials>,
}

impl ProxyEndpoint {
    pub fn new(protocol: ProxyProtocol, host: &str, port: u16) -> Self {
        Self {
            protocol,
            host: host.trim_start_matches('[').trim_end_matches(']').to_owned(),
            port,
            credentials: None,
        }
    }

    /// proxy url for a client library, eg http://host:port or socks5h://host:port
    /// credentials are left out so they never hit a log, the transport sets them itself
    pub fn url(&self) -> String {
        let scheme = match self.protocol {
            ProxyProtocol::Http => "http",
            ProxyProtocol::Socks5 => "socks5h",
        };
        let host = if self.host.parse::<Ipv6Addr>().is_ok() {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("{scheme}://{host}:{}", self.port)
    }
}

/// a proxy auto config the system points at
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum AutoConfig {
    Url(String),
    Script(String),
}

/// one entry of the ordered system answer for a destination
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SystemProxyEntry {
    Direct,
    Proxy(ProxyEndpoint),
    AutoConfig(AutoConfig),
}

/// the decision for one destination
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ProxyRoute {
    Direct,
    Via(ProxyEndpoint),
}

/// why the system answer couldnt be read, the connection then opens directly
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum ProxyUnavailable {
    #[error("reading the system proxy is not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("no system service resolves proxies: {0}")]
    NoService(String),
    #[error("the proxy auto-configuration failed: {0}")]
    AutoConfigFailed(String),
    #[error("the proxy auto-configuration did not answer within {0:?}")]
    AutoConfigTimedOut(Duration),
    #[error("the system refused the proxy lookup: {0}")]
    SystemRefused(String),
}

/// default answer of an os layer that doesnt override a method of the source
pub const fn unavailable_by_default(host: HostOs) -> ProxyUnavailable {
    ProxyUnavailable::BranchNotWritten(host)
}

/// where an os layer reports that the system proxy settings changed
pub type ProxySettingsSink = Box<dyn Fn() + Send>;

/// the os layer
/// both calls block (settings read, pac download and run) so only call them off the main thread
pub trait SystemProxySource: Send + Sync {
    fn watch_settings(&self, sink: ProxySettingsSink) -> Result<ActivityWatch, ProxyUnavailable> {
        let _ = sink;
        Err(unavailable_by_default(HostOs::current()))
    }

    fn proxies_for(&self, target: &ProxyTarget) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        let _ = target;
        Err(unavailable_by_default(HostOs::current()))
    }

    fn run_auto_config(
        &self,
        config: &AutoConfig,
        target: &ProxyTarget,
        timeout: Duration,
    ) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        let _ = (config, target, timeout);
        Err(unavailable_by_default(HostOs::current()))
    }
}

/// walks the ordered system answer, first usable entry wins
/// an auto config entry is run and its first usable entry wins, a failed one is skipped
pub fn resolve(
    source: &dyn SystemProxySource,
    target: &ProxyTarget,
) -> Result<ProxyRoute, ProxyUnavailable> {
    let entries = source.proxies_for(target)?;
    for entry in entries {
        match entry {
            SystemProxyEntry::Direct => return Ok(ProxyRoute::Direct),
            SystemProxyEntry::Proxy(endpoint) => return Ok(ProxyRoute::Via(endpoint)),
            SystemProxyEntry::AutoConfig(config) => {
                match source.run_auto_config(&config, target, AUTO_CONFIG_TIMEOUT) {
                    Ok(answer) => {
                        if let Some(route) = first_concrete(answer) {
                            return Ok(route);
                        }
                    }
                    Err(failure) => {
                        tracing::warn!(target: "riseonly", %failure, host = %target.host, "proxy auto-configuration skipped");
                    }
                }
            }
        }
    }
    Ok(ProxyRoute::Direct)
}

pub fn first_concrete(answer: Vec<SystemProxyEntry>) -> Option<ProxyRoute> {
    answer.into_iter().find_map(|entry| match entry {
        SystemProxyEntry::Direct => Some(ProxyRoute::Direct),
        SystemProxyEntry::Proxy(endpoint) => Some(ProxyRoute::Via(endpoint)),
        SystemProxyEntry::AutoConfig(_) => None,
    })
}

/// one rule of an exception list (bypass hosts, ProxyOverride or no_proxy)
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum BypassRule {
    Everything,
    SimpleHostnames,
    Pattern(String),
    Domain(String),
    Network { address: IpAddr, prefix: u8 },
}

impl BypassRule {
    /// parses one rule, none for an empty or unreadable entry
    pub fn parse(rule: &str) -> Option<Self> {
        let rule = rule.trim_ascii().to_ascii_lowercase();
        if rule.is_empty() {
            return None;
        }
        if rule == "*" {
            return Some(Self::Everything);
        }
        if rule == "<local>" {
            return Some(Self::SimpleHostnames);
        }
        if let Some(network) = parse_network(&rule) {
            return Some(network);
        }
        if rule.contains('*') {
            return Some(Self::Pattern(rule));
        }
        let domain = rule.trim_start_matches('.').trim_end_matches('.');
        if domain.is_empty() {
            return None;
        }
        Some(Self::Domain(domain.to_owned()))
    }

    pub fn matches(&self, host: &str) -> bool {
        let host = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        let address = host.parse::<IpAddr>().ok();
        match self {
            Self::Everything => true,
            Self::SimpleHostnames => address.is_none() && !host.contains('.') && !host.is_empty(),
            Self::Pattern(pattern) => wildcard_matches(pattern, &host),
            Self::Domain(domain) => {
                host == *domain
                    || host
                        .strip_suffix(domain.as_str())
                        .is_some_and(|head| head.ends_with('.'))
            }
            Self::Network { address: network, prefix } => {
                address.is_some_and(|address| in_network(address, *network, *prefix))
            }
        }
    }
}

fn parse_network(rule: &str) -> Option<BypassRule> {
    let (address, prefix) = match rule.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix.parse::<u8>().ok()?)),
        None => (rule, None),
    };
    let address = address.trim_start_matches('[').trim_end_matches(']');
    if let Ok(v6) = address.parse::<Ipv6Addr>() {
        let prefix = prefix.unwrap_or(128);
        return (prefix <= 128).then_some(BypassRule::Network {
            address: IpAddr::V6(v6),
            prefix,
        });
    }
    let octets: Vec<&str> = address.split('.').collect();
    if octets.is_empty() || octets.len() > 4 {
        return None;
    }
    let mut bytes = [0_u8; 4];
    for (slot, octet) in bytes.iter_mut().zip(&octets) {
        *slot = octet.parse::<u8>().ok()?;
    }
    if octets.len() < 4 && prefix.is_none() {
        return None;
    }
    let prefix = prefix.unwrap_or(32);
    (prefix <= 32).then_some(BypassRule::Network {
        address: IpAddr::V4(Ipv4Addr::from(bytes)),
        prefix,
    })
}

fn in_network(address: IpAddr, network: IpAddr, prefix: u8) -> bool {
    match (address, network) {
        (IpAddr::V4(address), IpAddr::V4(network)) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            u32::from(address) & mask == u32::from(network) & mask
        }
        (IpAddr::V6(address), IpAddr::V6(network)) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            u128::from(address) & mask == u128::from(network) & mask
        }
        _ => false,
    }
}

fn wildcard_matches(pattern: &str, host: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let (Some(first), Some(last)) = (parts.first(), parts.last()) else {
        return false;
    };
    if parts.len() == 1 {
        return pattern == host;
    }
    if !host.starts_with(first) {
        return false;
    }
    let mut rest = &host[first.len()..];
    for middle in &parts[1..parts.len() - 1] {
        match rest.find(middle) {
            Some(index) => rest = &rest[index + middle.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

/// proxy config as servers plus an exception list, linux variables and windows ie settings
/// mac answers through cfnetwork and never builds one
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ManualProxies {
    pub http: Option<ProxyEndpoint>,
    pub https: Option<ProxyEndpoint>,
    pub fallback: Option<ProxyEndpoint>,
    pub bypass: Vec<BypassRule>,
}

impl ManualProxies {
    pub fn is_empty(&self) -> bool {
        self.http.is_none() && self.https.is_none() && self.fallback.is_none()
    }

    pub fn bypasses(&self, host: &str) -> bool {
        self.bypass.iter().any(|rule| rule.matches(host))
    }

    /// answer for one destination in the shape the system lookups give
    pub fn entries_for(&self, target: &ProxyTarget) -> Vec<SystemProxyEntry> {
        if self.bypasses(&target.host) {
            return vec![SystemProxyEntry::Direct];
        }
        let own = if target.scheme.is_secure() { &self.https } else { &self.http };
        match own.as_ref().or(self.fallback.as_ref()) {
            Some(endpoint) => vec![SystemProxyEntry::Proxy(endpoint.clone())],
            None => vec![SystemProxyEntry::Direct],
        }
    }

    /// reads proxy variables of a unix session like curl does
    /// lowercase wins over uppercase and http_proxy is lowercase only
    pub fn from_environment(variable: impl Fn(&str) -> Option<String>) -> Self {
        let read = |names: &[&str]| {
            names
                .iter()
                .find_map(|name| variable(name).filter(|value| !value.trim_ascii().is_empty()))
        };
        let server = |names: &[&str]| read(names).and_then(|value| parse_proxy_url(&value, 1080));
        Self {
            http: server(&["http_proxy"]),
            https: server(&["https_proxy", "HTTPS_PROXY"]),
            fallback: server(&["all_proxy", "ALL_PROXY"]),
            bypass: read(&["no_proxy", "NO_PROXY"])
                .map(|list| parse_bypass_list(&list, ','))
                .unwrap_or_default(),
        }
    }

    /// reads the ie settings winhttp uses, host:port for all schemes or per scheme pairs
    /// override list is split on semicolons and may hold local
    pub fn from_windows_settings(proxy_server: &str, proxy_override: &str) -> Self {
        let mut proxies = Self {
            bypass: parse_bypass_list(proxy_override, ';'),
            ..Self::default()
        };
        for part in proxy_server.split(';').map(str::trim_ascii).filter(|part| !part.is_empty()) {
            match part.split_once('=') {
                Some((scheme, server)) => {
                    let scheme = scheme.trim_ascii().to_ascii_lowercase();
                    let endpoint = if scheme == "socks" {
                        parse_proxy_url(&format!("socks5://{}", server.trim_ascii()), 1080)
                    } else {
                        parse_proxy_url(server, 80)
                    };
                    match scheme.as_str() {
                        "http" => proxies.http = endpoint,
                        "https" => proxies.https = endpoint,
                        "socks" if proxies.fallback.is_none() => proxies.fallback = endpoint,
                        _ => {}
                    }
                }
                None => proxies.fallback = parse_proxy_url(part, 80),
            }
        }
        proxies
    }
}

/// parses exception rules split by the separator, whitespace splits too
pub fn parse_bypass_list(list: &str, separator: char) -> Vec<BypassRule> {
    list.split(|character: char| character == separator || character.is_whitespace())
        .filter_map(BypassRule::parse)
        .collect()
}

/// parses scheme://user:password@host:port, default_port if the port is missing
/// socks variants are socks, http https or no scheme is an http proxy
pub fn parse_proxy_url(value: &str, default_port: u16) -> Option<ProxyEndpoint> {
    let value = value.trim_ascii();
    let (scheme, rest) = match value.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => (String::from("http"), value),
    };
    let protocol = match scheme.as_str() {
        "http" | "https" => ProxyProtocol::Http,
        "socks" | "socks5" | "socks5h" | "socks4a" => ProxyProtocol::Socks5,
        _ => return None,
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let (credentials, server) = match authority.rsplit_once('@') {
        Some((userinfo, server)) => {
            let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
            (
                Some(ProxyCredentials {
                    user: percent_decode(user),
                    password: percent_decode(password),
                }),
                server,
            )
        }
        None => (None, authority),
    };
    let (host, port) = split_host_port(server)?;
    let mut endpoint = ProxyEndpoint::new(protocol, host, port.unwrap_or(default_port));
    endpoint.credentials = credentials;
    Some(endpoint)
}

fn split_host_port(server: &str) -> Option<(&str, Option<u16>)> {
    if let Some(rest) = server.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(port) => Some(port.parse::<u16>().ok()?),
            None if tail.is_empty() => None,
            None => return None,
        };
        return (!host.is_empty()).then_some((host, port));
    }
    match server.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            if host.is_empty() {
                return None;
            }
            Some((host, Some(port.parse::<u16>().ok()?)))
        }
        Some(_) => server.parse::<Ipv6Addr>().ok().map(|_| (server, None)),
        None => (!server.is_empty()).then_some((server, None)),
    }
}

/// decodes percent escapes in userinfo, an invalid escape stays as written
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(value) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// cache key, one answer per scheme host and port
/// kept until the system settings or the network change
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct RouteKey {
    secure: bool,
    host: String,
    port: u16,
}

/// process wide resolver, one lookup per destination then the cached route until forget
pub struct SystemProxy {
    source: Box<dyn SystemProxySource>,
    routes: Mutex<HashMap<RouteKey, ProxyRoute>>,
    last_failure: Mutex<Option<ProxyUnavailable>>,
}

impl SystemProxy {
    pub fn new(source: impl SystemProxySource + 'static) -> Self {
        Self {
            source: Box::new(source),
            routes: Mutex::new(HashMap::new()),
            last_failure: Mutex::new(None),
        }
    }

    /// route for one destination, blocks on the first lookup (a pac run can take a while)
    /// so call it off the main thread, a failed lookup means a direct connection
    pub fn route_for(&self, target: &ProxyTarget) -> ProxyRoute {
        let key = RouteKey {
            secure: target.scheme.is_secure(),
            host: target.host.clone(),
            port: target.port,
        };
        if let Some(route) = self.routes.lock().get(&key) {
            return route.clone();
        }
        let route = match resolve(self.source.as_ref(), target) {
            Ok(route) => route,
            Err(failure) => {
                let mut last = self.last_failure.lock();
                if last.as_ref() != Some(&failure) {
                    tracing::warn!(target: "riseonly", %failure, "connecting without the system proxy");
                }
                *last = Some(failure);
                ProxyRoute::Direct
            }
        };
        self.routes.lock().insert(key, route.clone());
        route
    }

    /// drops every cached route since settings or network changed, next connection asks again
    pub fn forget(&self) {
        self.routes.lock().clear();
        *self.last_failure.lock() = None;
    }

    pub fn watch_settings(
        &self,
        on_change: impl Fn() + Send + 'static,
    ) -> Result<ActivityWatch, ProxyUnavailable> {
        self.source.watch_settings(Box::new(on_change))
    }

    /// why the last lookup fell back to direct, if it did
    pub fn last_failure(&self) -> Option<ProxyUnavailable> {
        self.last_failure.lock().clone()
    }

    pub fn support(&self) -> PlatformSupport {
        match self.last_failure.lock().as_ref() {
            Some(
                ProxyUnavailable::BranchNotWritten(_)
                | ProxyUnavailable::NoService(_)
                | ProxyUnavailable::SystemRefused(_),
            ) => PlatformSupport::Unsupported,
            _ => PlatformSupport::Performed,
        }
    }
}

/// the resolver for the os layer of this build target
pub fn system_proxy() -> &'static SystemProxy {
    static SHARED: OnceLock<SystemProxy> = OnceLock::new();
    SHARED.get_or_init(|| SystemProxy::new(super::os::NativeProxySource))
}
