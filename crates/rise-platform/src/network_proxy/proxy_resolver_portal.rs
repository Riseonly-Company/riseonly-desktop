use std::time::Duration;

use super::network_proxy::*;

pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const PROXY_RESOLVER_INTERFACE: &str = "org.freedesktop.portal.ProxyResolver";
pub const LOOKUP_METHOD: &str = "Lookup";
pub const DIRECT_URI: &str = "direct://";

pub const HTTP_PROXY_DEFAULT_PORT: u16 = 80;
pub const SOCKS_PROXY_DEFAULT_PORT: u16 = 1080;

/// one uri of the portal answer, direct or http or socks
/// none for entries the transports cant use, eg socks4 since the client resolves names itself
pub fn portal_entry(uri: &str) -> Option<SystemProxyEntry> {
    let uri = uri.trim_ascii();
    if uri.eq_ignore_ascii_case(DIRECT_URI) {
        return Some(SystemProxyEntry::Direct);
    }
    let scheme = uri.split_once("://")?.0.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" | "https" => HTTP_PROXY_DEFAULT_PORT,
        "socks" | "socks5" | "socks5h" | "socks4a" => SOCKS_PROXY_DEFAULT_PORT,
        _ => return None,
    };
    parse_proxy_url(uri, default_port).map(SystemProxyEntry::Proxy)
}

/// usable entries of the portal answer in its order
pub fn portal_entries(uris: &[String]) -> Vec<SystemProxyEntry> {
    uris.iter().filter_map(|uri| portal_entry(uri)).collect()
}

/// ordered answer for one destination, the portal one if it gave a usable entry
/// else the session variables, which mean direct when none is set
pub fn linux_entries(
    portal: Option<&[String]>,
    environment: &ManualProxies,
    target: &ProxyTarget,
) -> Vec<SystemProxyEntry> {
    let answered = portal.map(portal_entries).unwrap_or_default();
    if answered.is_empty() {
        return environment.entries_for(target);
    }
    answered
}

pub const SETTINGS_QUIET: Duration = Duration::from_millis(500);

pub const DCONF_WRITER_PATH_NAMESPACE: &str = "/ca/desrt/dconf/Writer";
pub const DCONF_WRITER_INTERFACE: &str = "ca.desrt.dconf.Writer";
pub const DCONF_NOTIFY_SIGNAL: &str = "Notify";
pub const GNOME_PROXY_DCONF_DIR: &str = "/system/proxy/";

pub const KIO_SCHEDULER_PATH: &str = "/KIO/Scheduler";
pub const KIO_SCHEDULER_INTERFACE: &str = "org.kde.KIO.Scheduler";
pub const KIO_REPARSE_SIGNAL: &str = "reparseSlaveConfiguration";

/// true when a dconf commit touches the proxy settings
/// a key or dir under them or a parent dir reset as a whole
pub fn dconf_commit_touches_proxy(prefix: &str, changes: &[String]) -> bool {
    changes.iter().any(|change| {
        let path = format!("{prefix}{change}");
        path.starts_with(GNOME_PROXY_DCONF_DIR)
            || (path.ends_with('/') && GNOME_PROXY_DCONF_DIR.starts_with(path.as_str()))
    })
}
