use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::network_proxy::*;

fn http(host: &str, port: u16) -> ProxyEndpoint {
    ProxyEndpoint::new(ProxyProtocol::Http, host, port)
}

fn target(scheme: TargetScheme, host: &str) -> ProxyTarget {
    ProxyTarget::new(scheme, host, None)
}

#[test]
fn a_websocket_is_looked_up_as_its_http_twin() {
    assert_eq!(TargetScheme::Wss.lookup_scheme(), "https");
    assert_eq!(TargetScheme::Ws.lookup_scheme(), "http");
    assert_eq!(target(TargetScheme::Wss, "api.riseonly.net").lookup_url(), "https://api.riseonly.net:443/");
    assert_eq!(target(TargetScheme::Ws, "Gate.Example").lookup_url(), "http://gate.example:80/");
    assert_eq!(
        ProxyTarget::new(TargetScheme::Https, "[::1]", Some(8443)).lookup_url(),
        "https://[::1]:8443/"
    );
}

#[test]
fn the_scheme_parser_knows_only_web_schemes() {
    assert_eq!(TargetScheme::parse("WSS"), Some(TargetScheme::Wss));
    assert_eq!(TargetScheme::parse("ftp"), None);
}

#[test]
fn a_proxy_url_never_carries_its_credentials() {
    let mut endpoint = ProxyEndpoint::new(ProxyProtocol::Socks5, "::1", 1080);
    endpoint.credentials = Some(ProxyCredentials {
        user: String::from("me"),
        password: String::from("secret"),
    });
    assert_eq!(endpoint.url(), "socks5h://[::1]:1080");
    assert!(!format!("{endpoint:?}").contains("secret"));
}

#[test]
fn bypass_rules_read_the_way_the_three_systems_write_them() {
    assert_eq!(BypassRule::parse("*"), Some(BypassRule::Everything));
    assert_eq!(BypassRule::parse("<local>"), Some(BypassRule::SimpleHostnames));
    assert_eq!(BypassRule::parse(".Example.com"), Some(BypassRule::Domain(String::from("example.com"))));
    assert_eq!(BypassRule::parse("*.local"), Some(BypassRule::Pattern(String::from("*.local"))));
    assert_eq!(BypassRule::parse("  "), None);
    assert!(matches!(BypassRule::parse("169.254/16"), Some(BypassRule::Network { prefix: 16, .. })));
    assert!(matches!(BypassRule::parse("10.1.2.3"), Some(BypassRule::Network { prefix: 32, .. })));
    assert!(matches!(BypassRule::parse("fe80::/10"), Some(BypassRule::Network { prefix: 10, .. })));
    assert_eq!(BypassRule::parse("10.1"), Some(BypassRule::Domain(String::from("10.1"))));
}

#[test]
fn a_domain_rule_covers_the_domain_and_its_subdomains_only() {
    let rule = BypassRule::parse("example.com").unwrap();
    assert!(rule.matches("example.com"));
    assert!(rule.matches("api.EXAMPLE.com"));
    assert!(!rule.matches("badexample.com"));
    assert!(!rule.matches("example.com.evil"));
}

#[test]
fn network_and_simple_hostname_rules_match_addresses_and_dotless_names() {
    let link_local = BypassRule::parse("169.254/16").unwrap();
    assert!(link_local.matches("169.254.10.1"));
    assert!(!link_local.matches("169.255.0.1"));
    assert!(BypassRule::parse("fe80::/10").unwrap().matches("[fe80::1]"));
    let simple = BypassRule::SimpleHostnames;
    assert!(simple.matches("intranet"));
    assert!(!simple.matches("intranet.corp"));
    assert!(!simple.matches("::1"));
    assert!(BypassRule::parse("*.local").unwrap().matches("printer.local"));
    assert!(!BypassRule::parse("*.local").unwrap().matches("local"));
    assert!(BypassRule::parse("10.*").unwrap().matches("10.0.0.7"));
}

#[test]
fn proxy_urls_parse_with_scheme_credentials_and_default_port() {
    let endpoint = parse_proxy_url("http://user:p%40ss@proxy.corp:3128/", 1080).unwrap();
    assert_eq!(endpoint.protocol, ProxyProtocol::Http);
    assert_eq!((endpoint.host.as_str(), endpoint.port), ("proxy.corp", 3128));
    let credentials = endpoint.credentials.unwrap();
    assert_eq!((credentials.user.as_str(), credentials.password.as_str()), ("user", "p@ss"));
    assert_eq!(parse_proxy_url("proxy.corp", 1080).unwrap().port, 1080);
    assert_eq!(parse_proxy_url("socks5://[::1]:9050", 1080).unwrap().protocol, ProxyProtocol::Socks5);
    assert_eq!(parse_proxy_url("[::1]:9050", 80).unwrap().host, "::1");
    assert_eq!(parse_proxy_url("ftp://proxy:21", 80), None);
    assert_eq!(parse_proxy_url("proxy:notaport", 80), None);
}

#[test]
fn the_environment_prefers_lowercase_and_ignores_uppercase_http_proxy() {
    let variables: HashMap<&str, &str> = HashMap::from([
        ("HTTP_PROXY", "http://injected:1"),
        ("https_proxy", "http://secure.corp:8443"),
        ("HTTPS_PROXY", "http://ignored:1"),
        ("ALL_PROXY", "socks5://socks.corp"),
        ("no_proxy", "localhost,.internal 10.0.0.0/8"),
    ]);
    let proxies = ManualProxies::from_environment(|name| variables.get(name).map(|value| (*value).to_owned()));
    assert_eq!(proxies.http, None);
    assert_eq!(proxies.https, Some(http("secure.corp", 8443)));
    assert_eq!(proxies.fallback.as_ref().map(|socks| socks.port), Some(1080));
    assert_eq!(proxies.bypass.len(), 3);

    assert_eq!(
        proxies.entries_for(&target(TargetScheme::Wss, "api.riseonly.net")),
        vec![SystemProxyEntry::Proxy(http("secure.corp", 8443))]
    );
    assert!(matches!(
        proxies.entries_for(&target(TargetScheme::Http, "cdn.riseonly.net")).as_slice(),
        [SystemProxyEntry::Proxy(ProxyEndpoint { protocol: ProxyProtocol::Socks5, .. })]
    ));
    assert_eq!(
        proxies.entries_for(&target(TargetScheme::Https, "db.internal")),
        vec![SystemProxyEntry::Direct]
    );
    assert_eq!(
        proxies.entries_for(&target(TargetScheme::Https, "10.3.4.5")),
        vec![SystemProxyEntry::Direct]
    );
}

#[test]
fn no_variables_mean_a_direct_connection() {
    let proxies = ManualProxies::from_environment(|_| None);
    assert!(proxies.is_empty());
    assert_eq!(
        proxies.entries_for(&target(TargetScheme::Https, "api.riseonly.net")),
        vec![SystemProxyEntry::Direct]
    );
}

#[test]
fn the_windows_strings_split_per_scheme_or_apply_to_all() {
    let single = ManualProxies::from_windows_settings("proxy.corp:8080", "<local>;*.corp.local");
    assert_eq!(single.fallback, Some(http("proxy.corp", 8080)));
    assert_eq!(
        single.entries_for(&target(TargetScheme::Https, "intranet")),
        vec![SystemProxyEntry::Direct]
    );
    assert_eq!(
        single.entries_for(&target(TargetScheme::Https, "wiki.corp.local")),
        vec![SystemProxyEntry::Direct]
    );

    let split = ManualProxies::from_windows_settings("http=web.corp:80;https=tls.corp;socks=s.corp:1081", "");
    assert_eq!(split.http, Some(http("web.corp", 80)));
    assert_eq!(split.https, Some(http("tls.corp", 80)));
    assert_eq!(
        split.fallback,
        Some(ProxyEndpoint::new(ProxyProtocol::Socks5, "s.corp", 1081))
    );
}

struct FakeSystem {
    answer: Result<Vec<SystemProxyEntry>, ProxyUnavailable>,
    auto_config: Result<Vec<SystemProxyEntry>, ProxyUnavailable>,
    lookups: Arc<AtomicUsize>,
}

impl SystemProxySource for FakeSystem {
    fn proxies_for(&self, _target: &ProxyTarget) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.answer.clone()
    }

    fn run_auto_config(
        &self,
        _config: &AutoConfig,
        _target: &ProxyTarget,
        timeout: Duration,
    ) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        assert_eq!(timeout, AUTO_CONFIG_TIMEOUT);
        self.auto_config.clone()
    }
}

fn fake(
    answer: Result<Vec<SystemProxyEntry>, ProxyUnavailable>,
    auto_config: Result<Vec<SystemProxyEntry>, ProxyUnavailable>,
) -> (FakeSystem, Arc<AtomicUsize>) {
    let lookups = Arc::new(AtomicUsize::new(0));
    (
        FakeSystem {
            answer,
            auto_config,
            lookups: Arc::clone(&lookups),
        },
        lookups,
    )
}

fn pac() -> SystemProxyEntry {
    SystemProxyEntry::AutoConfig(AutoConfig::Url(String::from("http://wpad/wpad.dat")))
}

#[test]
fn the_first_usable_entry_wins_and_pac_answers_are_followed() {
    let wss = target(TargetScheme::Wss, "api.riseonly.net");
    let (system, _) = fake(Ok(vec![pac(), SystemProxyEntry::Direct]), Ok(vec![pac(), SystemProxyEntry::Proxy(http("pac.corp", 3128))]));
    assert_eq!(resolve(&system, &wss), Ok(ProxyRoute::Via(http("pac.corp", 3128))));

    let (system, _) = fake(
        Ok(vec![pac(), SystemProxyEntry::Proxy(http("backup.corp", 8080))]),
        Err(ProxyUnavailable::AutoConfigTimedOut(AUTO_CONFIG_TIMEOUT)),
    );
    assert_eq!(resolve(&system, &wss), Ok(ProxyRoute::Via(http("backup.corp", 8080))));

    let (system, _) = fake(Ok(vec![pac()]), Err(ProxyUnavailable::AutoConfigFailed(String::from("404"))));
    assert_eq!(resolve(&system, &wss), Ok(ProxyRoute::Direct));

    let (system, _) = fake(Ok(Vec::new()), Ok(Vec::new()));
    assert_eq!(resolve(&system, &wss), Ok(ProxyRoute::Direct));
}

#[test]
fn routes_are_cached_per_destination_until_forget() {
    let (system, lookups) = fake(Ok(vec![SystemProxyEntry::Proxy(http("proxy.corp", 3128))]), Ok(Vec::new()));
    let resolver = SystemProxy::new(system);
    let api = target(TargetScheme::Https, "api.riseonly.net");
    let socket = target(TargetScheme::Wss, "api.riseonly.net");
    assert_eq!(resolver.route_for(&api), ProxyRoute::Via(http("proxy.corp", 3128)));
    assert_eq!(resolver.route_for(&socket), ProxyRoute::Via(http("proxy.corp", 3128)));
    assert_eq!(lookups.load(Ordering::SeqCst), 1);
    resolver.forget();
    resolver.route_for(&api);
    assert_eq!(lookups.load(Ordering::SeqCst), 2);
}

#[test]
fn an_unreadable_system_connects_directly_and_reports_unsupported() {
    for host in HostOs::ALL {
        let (system, _) = fake(Err(unavailable_by_default(host)), Ok(Vec::new()));
        let resolver = SystemProxy::new(system);
        assert_eq!(resolver.support(), PlatformSupport::Performed);
        assert_eq!(resolver.route_for(&target(TargetScheme::Https, "a.b")), ProxyRoute::Direct);
        assert_eq!(resolver.last_failure(), Some(ProxyUnavailable::BranchNotWritten(host)));
        assert_eq!(resolver.support(), PlatformSupport::Unsupported);
        resolver.forget();
        assert_eq!(resolver.last_failure(), None);
    }
}

#[test]
fn the_default_trait_answer_names_the_host() {
    struct Unwritten;
    impl SystemProxySource for Unwritten {}
    assert_eq!(
        Unwritten.proxies_for(&target(TargetScheme::Https, "a.b")),
        Err(ProxyUnavailable::BranchNotWritten(HostOs::current()))
    );
}

#[test]
fn a_layer_without_a_settings_signal_says_so() {
    struct Unwritten;
    impl SystemProxySource for Unwritten {}
    let answer = SystemProxy::new(Unwritten).watch_settings(|| {});
    assert_eq!(answer.err(), Some(ProxyUnavailable::BranchNotWritten(HostOs::current())));
}

#[test]
fn the_settings_signal_reaches_the_caller() {
    struct Signalling;
    impl SystemProxySource for Signalling {
        fn watch_settings(&self, sink: ProxySettingsSink) -> Result<crate::system_activity::ActivityWatch, ProxyUnavailable> {
            sink();
            Ok(crate::system_activity::ActivityWatch::new(()))
        }
    }
    let heard = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&heard);
    let watch = SystemProxy::new(Signalling).watch_settings(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    assert!(watch.is_ok());
    assert_eq!(heard.load(Ordering::SeqCst), 1);
}
