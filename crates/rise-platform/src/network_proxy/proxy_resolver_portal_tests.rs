use super::network_proxy::*;
use super::proxy_resolver_portal::*;

fn https_target() -> ProxyTarget {
    ProxyTarget::new(TargetScheme::Https, "api.riseonly.net", None)
}

fn ws_target() -> ProxyTarget {
    ProxyTarget::new(TargetScheme::Ws, "media.riseonly.net", Some(8080))
}

fn answer(uris: &[&str]) -> Vec<String> {
    uris.iter().map(|uri| (*uri).to_owned()).collect()
}

fn environment(pairs: &[(&str, &str)]) -> ManualProxies {
    ManualProxies::from_environment(|name| {
        pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
    })
}

#[test]
fn direct_uri_is_direct() {
    assert_eq!(portal_entry("direct://"), Some(SystemProxyEntry::Direct));
    assert_eq!(portal_entry(" DIRECT:// "), Some(SystemProxyEntry::Direct));
}

#[test]
fn http_uri_keeps_host_port_and_credentials() {
    let Some(SystemProxyEntry::Proxy(endpoint)) = portal_entry("http://user:p%40ss@proxy.corp:3128")
    else {
        panic!("an http proxy entry");
    };
    assert_eq!(endpoint.protocol, ProxyProtocol::Http);
    assert_eq!(endpoint.host, "proxy.corp");
    assert_eq!(endpoint.port, 3128);
    let credentials = endpoint.credentials.expect("credentials");
    assert_eq!(credentials.user, "user");
    assert_eq!(credentials.password, "p@ss");
}

#[test]
fn missing_ports_take_the_protocol_default() {
    assert_eq!(
        portal_entry("http://proxy.corp"),
        Some(SystemProxyEntry::Proxy(ProxyEndpoint::new(ProxyProtocol::Http, "proxy.corp", 80)))
    );
    assert_eq!(
        portal_entry("socks://proxy.corp"),
        Some(SystemProxyEntry::Proxy(ProxyEndpoint::new(ProxyProtocol::Socks5, "proxy.corp", 1080)))
    );
}

#[test]
fn socks_versions_that_resolve_names_are_socks5() {
    for uri in ["socks://[::1]:9050", "socks5://[::1]:9050", "socks4a://[::1]:9050"] {
        assert_eq!(
            portal_entry(uri),
            Some(SystemProxyEntry::Proxy(ProxyEndpoint::new(ProxyProtocol::Socks5, "::1", 9050))),
            "{uri}"
        );
    }
}

#[test]
fn unusable_uris_are_skipped() {
    for uri in ["socks4://proxy.corp:1080", "rtsp://proxy.corp:554", "ftp://proxy.corp", "", "proxy.corp:3128"] {
        assert_eq!(portal_entry(uri), None, "{uri}");
    }
}

#[test]
fn portal_order_is_kept() {
    let uris = answer(&["socks4://old:1080", "http://first:3128", "direct://"]);
    assert_eq!(
        portal_entries(&uris),
        vec![
            SystemProxyEntry::Proxy(ProxyEndpoint::new(ProxyProtocol::Http, "first", 3128)),
            SystemProxyEntry::Direct,
        ]
    );
}

#[test]
fn portal_answer_wins_over_the_variables() {
    let variables = environment(&[("https_proxy", "http://env:8080")]);
    let uris = answer(&["direct://"]);
    assert_eq!(
        linux_entries(Some(uris.as_slice()), &variables, &https_target()),
        vec![SystemProxyEntry::Direct]
    );
}

#[test]
fn without_a_portal_the_variables_answer() {
    let variables = environment(&[("http_proxy", "http://env:8080"), ("no_proxy", "riseonly.net")]);
    assert_eq!(
        linux_entries(None, &variables, &ProxyTarget::new(TargetScheme::Ws, "example.org", None)),
        vec![SystemProxyEntry::Proxy(ProxyEndpoint::new(ProxyProtocol::Http, "env", 8080))]
    );
    assert_eq!(linux_entries(None, &variables, &ws_target()), vec![SystemProxyEntry::Direct]);
}

#[test]
fn an_answer_without_usable_entries_falls_back_to_the_variables() {
    let variables = environment(&[("all_proxy", "socks5://env:1080")]);
    let uris = answer(&["socks4://old:1080"]);
    assert_eq!(
        linux_entries(Some(uris.as_slice()), &variables, &https_target()),
        vec![SystemProxyEntry::Proxy(ProxyEndpoint::new(ProxyProtocol::Socks5, "env", 1080))]
    );
    assert_eq!(
        linux_entries(Some(Vec::<String>::new().as_slice()), &ManualProxies::default(), &https_target()),
        vec![SystemProxyEntry::Direct]
    );
}

#[test]
fn the_portal_is_asked_for_the_lookup_url() {
    assert_eq!(https_target().lookup_url(), "https://api.riseonly.net:443/");
    assert_eq!(ws_target().lookup_url(), "http://media.riseonly.net:8080/");
}

fn changes(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}

#[test]
fn dconf_commit_of_a_proxy_key_is_a_change() {
    assert!(dconf_commit_touches_proxy("/system/proxy/mode", &changes(&[""])));
    assert!(dconf_commit_touches_proxy("/system/proxy/http/host", &changes(&[""])));
    assert!(dconf_commit_touches_proxy("/system/proxy/", &changes(&["mode", "http/port"])));
}

#[test]
fn dconf_commit_under_a_common_prefix_is_read_whole() {
    assert!(dconf_commit_touches_proxy("/", &changes(&["org/gnome/a", "system/proxy/mode"])));
    assert!(!dconf_commit_touches_proxy("/", &changes(&["org/gnome/a", "system/locale/region"])));
}

#[test]
fn dconf_reset_of_a_parent_directory_is_a_change() {
    assert!(dconf_commit_touches_proxy("/", &changes(&[""])));
    assert!(dconf_commit_touches_proxy("/system/", &changes(&["proxy/"])));
}

#[test]
fn dconf_commit_elsewhere_is_not_a_change() {
    assert!(!dconf_commit_touches_proxy("/org/gnome/desktop/interface/color-scheme", &changes(&[""])));
    assert!(!dconf_commit_touches_proxy("/system/proxyish/mode", &changes(&[""])));
    assert!(!dconf_commit_touches_proxy("/sys", &changes(&[""])));
    assert!(!dconf_commit_touches_proxy("/system/proxy/", &[]));
}
