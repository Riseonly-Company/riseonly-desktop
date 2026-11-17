use std::time::Duration;

use super::network_proxy::*;
use super::winhttp_proxy::*;

fn https_target() -> ProxyTarget {
    ProxyTarget::new(TargetScheme::Https, "api.riseonly.net", None)
}

fn http_target() -> ProxyTarget {
    ProxyTarget::new(TargetScheme::Ws, "media.riseonly.net", Some(8080))
}

fn http_proxy(host: &str, port: u16) -> SystemProxyEntry {
    SystemProxyEntry::Proxy(ProxyEndpoint::new(ProxyProtocol::Http, host, port))
}

#[test]
fn blank_settings_are_absent() {
    let settings = IeProxySettings::new(false, Some(String::from("  ")), Some(String::new()), None);
    assert_eq!(settings, IeProxySettings::default());
    assert_eq!(settings.configured_entries(&https_target()), vec![SystemProxyEntry::Direct]);
}

#[test]
fn setup_script_comes_before_manual_servers() {
    let settings = IeProxySettings::new(
        true,
        Some(String::from(" http://intranet/proxy.pac ")),
        Some(String::from("corp:3128")),
        Some(String::from("<local>;*.lan")),
    );
    assert!(settings.auto_detect);
    assert_eq!(
        settings.configured_entries(&https_target()),
        vec![
            SystemProxyEntry::AutoConfig(AutoConfig::Url(String::from("http://intranet/proxy.pac"))),
            http_proxy("corp", 3128),
        ]
    );
}

#[test]
fn script_without_manual_servers_falls_back_to_direct() {
    let settings = IeProxySettings::new(false, Some(String::from("http://wpad/wpad.dat")), None, None);
    assert_eq!(
        settings.configured_entries(&https_target()),
        vec![
            SystemProxyEntry::AutoConfig(AutoConfig::Url(String::from("http://wpad/wpad.dat"))),
            SystemProxyEntry::Direct,
        ]
    );
}

#[test]
fn manual_exception_list_applies() {
    let settings = IeProxySettings::new(
        false,
        None,
        Some(String::from("http=web:80;https=secure:443")),
        Some(String::from("<local>;*.riseonly.net")),
    );
    assert_eq!(settings.configured_entries(&https_target()), vec![SystemProxyEntry::Direct]);
    let intranet = ProxyTarget::new(TargetScheme::Https, "portal.corp", None);
    assert_eq!(settings.configured_entries(&intranet), vec![http_proxy("secure", 443)]);
    let plain = ProxyTarget::new(TargetScheme::Http, "portal.corp", None);
    assert_eq!(settings.configured_entries(&plain), vec![http_proxy("web", 80)]);
}

#[test]
fn only_a_named_answer_carries_proxies() {
    for access in [ACCESS_TYPE_DEFAULT_PROXY, ACCESS_TYPE_NO_PROXY, ACCESS_TYPE_AUTOMATIC_PROXY, 9] {
        assert_eq!(
            entries_from_answer(access, Some("corp:3128"), &https_target()),
            vec![SystemProxyEntry::Direct]
        );
    }
}

#[test]
fn named_answer_keeps_the_pac_order() {
    assert_eq!(
        entries_from_answer(ACCESS_TYPE_NAMED_PROXY, Some("first:8080; second\tthird:3128"), &https_target()),
        vec![http_proxy("first", 8080), http_proxy("second", 80), http_proxy("third", 3128)]
    );
}

#[test]
fn named_answer_takes_the_scheme_of_the_destination() {
    let list = Some("http=web:81;https=secure:444;ftp=files:21;socks=tunnel");
    assert_eq!(
        entries_from_answer(ACCESS_TYPE_NAMED_PROXY, list, &https_target()),
        vec![
            http_proxy("secure", 444),
            SystemProxyEntry::Proxy(ProxyEndpoint::new(ProxyProtocol::Socks5, "tunnel", 1080)),
        ]
    );
    assert_eq!(
        entries_from_answer(ACCESS_TYPE_NAMED_PROXY, list, &http_target()),
        vec![
            http_proxy("web", 81),
            SystemProxyEntry::Proxy(ProxyEndpoint::new(ProxyProtocol::Socks5, "tunnel", 1080)),
        ]
    );
}

#[test]
fn named_answer_without_servers_is_direct() {
    assert_eq!(entries_from_answer(ACCESS_TYPE_NAMED_PROXY, None, &https_target()), vec![SystemProxyEntry::Direct]);
    assert_eq!(
        entries_from_answer(ACCESS_TYPE_NAMED_PROXY, Some(" ; ftp=files:21 "), &https_target()),
        vec![SystemProxyEntry::Direct]
    );
}

#[test]
fn ipv6_server_in_a_named_answer() {
    assert_eq!(
        entries_from_answer(ACCESS_TYPE_NAMED_PROXY, Some("[fd00::1]:3128"), &https_target()),
        vec![http_proxy("fd00::1", 3128)]
    );
}

#[test]
fn script_text_cannot_run() {
    assert_eq!(
        AutoProxyRequest::for_config(&AutoConfig::Url(String::from("http://intranet/proxy.pac"))),
        Ok(AutoProxyRequest::ConfigUrl(String::from("http://intranet/proxy.pac")))
    );
    assert_eq!(
        AutoProxyRequest::for_config(&AutoConfig::Script(String::from("function FindProxyForURL() {}"))),
        Err(ProxyUnavailable::AutoConfigFailed(SCRIPT_NOT_SUPPORTED.to_owned()))
    );
}

#[test]
fn win32_code_reads_facility_seven_only() {
    let from_win32 = |code: u32| (0x8007_0000_u32 | code) as i32;
    assert_eq!(win32_code(from_win32(ERROR_AUTODETECTION_FAILED)), Some(12180));
    assert_eq!(win32_code(from_win32(ERROR_TIMEOUT)), Some(12002));
    assert_eq!(win32_code(0x8000_4005_u32 as i32), None);
    assert_eq!(win32_code(0), None);
}

#[test]
fn failures_map_to_seam_reasons() {
    let timeout = Duration::from_secs(10);
    assert_eq!(
        failure_from_code(Some(ERROR_TIMEOUT), "timed out", timeout),
        ProxyUnavailable::AutoConfigTimedOut(timeout)
    );
    for code in [
        ERROR_AUTODETECTION_FAILED,
        ERROR_UNABLE_TO_DOWNLOAD_SCRIPT,
        ERROR_BAD_AUTO_PROXY_SCRIPT,
        ERROR_AUTO_PROXY_SERVICE_ERROR,
        ERROR_LOGIN_FAILURE,
    ] {
        assert_eq!(
            failure_from_code(Some(code), "failed", timeout),
            ProxyUnavailable::AutoConfigFailed(format!("WinHTTP error {code}: failed"))
        );
    }
    assert_eq!(
        failure_from_code(Some(5), "denied", timeout),
        ProxyUnavailable::SystemRefused(String::from("WinHTTP error 5: denied"))
    );
    assert_eq!(
        failure_from_code(None, "broken", timeout),
        ProxyUnavailable::SystemRefused(String::from("broken"))
    );
}

#[test]
fn timeouts_never_reach_winhttp_as_zero() {
    assert_eq!(timeout_millis(Duration::ZERO), 1);
    assert_eq!(timeout_millis(Duration::from_millis(1500)), 1500);
    assert_eq!(timeout_millis(AUTO_CONFIG_TIMEOUT), 10_000);
    assert_eq!(timeout_millis(Duration::from_secs(u64::MAX)), i32::MAX);
}
