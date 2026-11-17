use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rise_platform::network_proxy::{
    BypassRule, ManualProxies, ProxyCredentials, ProxyEndpoint, ProxyProtocol, ProxyRoute, ProxyTarget,
    ProxyUnavailable, SystemProxy, SystemProxyEntry, SystemProxySource, TargetScheme,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::network_transport_config::{
    GenerationCell, SocketRoutes, check_connect_response, connect_request, endpoint_url,
    establish_http_tunnel, establish_socks5_tunnel, proxy_url_for, reconnect_route, socks5_address,
    tunnel_authority,
};

struct ManualSource(ManualProxies);

impl SystemProxySource for ManualSource {
    fn proxies_for(&self, target: &ProxyTarget) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        Ok(self.0.entries_for(target))
    }
}

struct SwitchableSource(Arc<Mutex<Vec<SystemProxyEntry>>>);

impl SystemProxySource for SwitchableSource {
    fn proxies_for(&self, _target: &ProxyTarget) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        Ok(self.0.lock().unwrap().clone())
    }
}

fn credentials() -> ProxyCredentials {
    ProxyCredentials { user: "aladdin".to_owned(), password: "open sesame".to_owned() }
}

fn manual_system() -> SystemProxy {
    SystemProxy::new(ManualSource(ManualProxies {
        http: Some(ProxyEndpoint::new(ProxyProtocol::Http, "plain.proxy", 3128)),
        https: Some(ProxyEndpoint::new(ProxyProtocol::Http, "secure.proxy", 3129)),
        fallback: None,
        bypass: vec![BypassRule::Domain("internal.example".to_owned())],
    }))
}

fn url(text: &str) -> reqwest::Url {
    reqwest::Url::parse(text).unwrap()
}

#[test]
fn each_scheme_takes_its_own_proxy() {
    let system = manual_system();
    assert_eq!(
        proxy_url_for(&system, &url("https://api.example.com/")).unwrap().as_str(),
        "http://secure.proxy:3129/"
    );
    assert_eq!(
        proxy_url_for(&system, &url("http://api.example.com/")).unwrap().as_str(),
        "http://plain.proxy:3128/"
    );
}

#[test]
fn the_exception_list_connects_directly() {
    let system = manual_system();
    assert_eq!(proxy_url_for(&system, &url("https://internal.example/")), None);
    assert_eq!(proxy_url_for(&system, &url("https://cdn.internal.example/")), None);
}

#[test]
fn no_system_proxy_connects_directly() {
    let system = SystemProxy::new(ManualSource(ManualProxies::default()));
    assert_eq!(proxy_url_for(&system, &url("https://api.example.com/")), None);
}

#[test]
fn a_scheme_without_a_proxy_lookup_connects_directly() {
    let system = manual_system();
    assert_eq!(proxy_url_for(&system, &url("file:///tmp/a")), None);
}

#[test]
fn credentials_travel_in_the_userinfo_escaped() {
    let mut endpoint = ProxyEndpoint::new(ProxyProtocol::Http, "proxy.example", 8080);
    endpoint.credentials = Some(credentials());
    let built = endpoint_url(&endpoint).unwrap();
    assert_eq!(built.username(), "aladdin");
    assert_eq!(built.password(), Some("open%20sesame"));
}

#[test]
fn a_socks_endpoint_keeps_name_resolution_on_the_proxy() {
    let endpoint = ProxyEndpoint::new(ProxyProtocol::Socks5, "proxy.example", 1080);
    assert_eq!(endpoint_url(&endpoint).unwrap().scheme(), "socks5h");
}

#[test]
fn the_connect_request_names_the_destination_twice() {
    let target = ProxyTarget::new(TargetScheme::Wss, "gateway.example.com", None);
    assert_eq!(
        connect_request(&target, None),
        "CONNECT gateway.example.com:443 HTTP/1.1\r\nHost: gateway.example.com:443\r\n\r\n"
    );
}

#[test]
fn the_connect_request_carries_basic_credentials() {
    let target = ProxyTarget::new(TargetScheme::Wss, "gateway.example.com", Some(8443));
    let request = connect_request(&target, Some(&credentials()));
    assert!(request.contains("Proxy-Authorization: Basic YWxhZGRpbjpvcGVuIHNlc2FtZQ==\r\n"));
    assert!(request.ends_with("\r\n\r\n"));
}

#[test]
fn an_ipv6_destination_is_bracketed_in_the_authority() {
    let target = ProxyTarget::new(TargetScheme::Wss, "[2001:db8::1]", None);
    assert_eq!(tunnel_authority(&target), "[2001:db8::1]:443");
}

#[test]
fn only_a_success_status_opens_the_tunnel() {
    assert!(check_connect_response(b"HTTP/1.1 200 Connection established\r\n\r\n").is_ok());
    assert!(check_connect_response(b"HTTP/1.0 204 No Content\r\n\r\n").is_ok());
    assert_eq!(
        check_connect_response(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        check_connect_response(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").unwrap_err().kind(),
        std::io::ErrorKind::ConnectionRefused
    );
    assert_eq!(
        check_connect_response(b"not http at all\r\n\r\n").unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[tokio::test]
async fn the_tunnel_leaves_the_bytes_after_the_answer_on_the_stream() {
    let (mut client, mut proxy) = tokio::io::duplex(1024);
    let target = ProxyTarget::new(TargetScheme::Wss, "gateway.example.com", None);
    let expected = connect_request(&target, None);

    let server = tokio::spawn(async move {
        let mut received = vec![0_u8; expected.len()];
        proxy.read_exact(&mut received).await.unwrap();
        assert_eq!(received, expected.as_bytes());
        proxy.write_all(b"HTTP/1.1 200 OK\r\nVia: test\r\n\r\nTLS").await.unwrap();
        proxy
    });

    establish_http_tunnel(&mut client, &target, None).await.unwrap();
    let mut first_tunnelled = [0_u8; 3];
    client.read_exact(&mut first_tunnelled).await.unwrap();
    assert_eq!(&first_tunnelled, b"TLS");
    drop(server.await.unwrap());
}

#[tokio::test]
async fn a_closed_proxy_fails_the_tunnel() {
    let (mut client, proxy) = tokio::io::duplex(1024);
    drop(proxy);
    let target = ProxyTarget::new(TargetScheme::Wss, "gateway.example.com", None);
    assert!(establish_http_tunnel(&mut client, &target, None).await.is_err());
}

#[test]
fn socks_addresses_follow_the_rfc() {
    let name = ProxyTarget::new(TargetScheme::Wss, "a.io", Some(443));
    assert_eq!(socks5_address(&name).unwrap(), vec![3, 4, b'a', b'.', b'i', b'o', 1, 187]);
    let v4 = ProxyTarget::new(TargetScheme::Ws, "10.0.0.1", Some(80));
    assert_eq!(socks5_address(&v4).unwrap(), vec![1, 10, 0, 0, 1, 0, 80]);
    let too_long = ProxyTarget::new(TargetScheme::Ws, &"a".repeat(256), None);
    assert!(socks5_address(&too_long).is_err());
}

#[tokio::test]
async fn the_socks_tunnel_exchanges_the_password_and_skips_the_bound_address() {
    let (mut client, mut proxy) = tokio::io::duplex(1024);
    let target = ProxyTarget::new(TargetScheme::Wss, "a.io", Some(443));

    let server = tokio::spawn(async move {
        let mut greeting = [0_u8; 4];
        proxy.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 2, 0, 2]);
        proxy.write_all(&[5, 2]).await.unwrap();

        let mut auth = vec![0_u8; 3 + "aladdin".len() + "open sesame".len()];
        proxy.read_exact(&mut auth).await.unwrap();
        assert_eq!(auth[0], 1);
        assert_eq!(auth[1], 7);
        assert_eq!(&auth[2..9], b"aladdin");
        proxy.write_all(&[1, 0]).await.unwrap();

        let mut request = [0_u8; 3 + 8];
        proxy.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[..3], &[5, 1, 0]);
        assert_eq!(&request[3..], &[3, 4, b'a', b'.', b'i', b'o', 1, 187]);
        proxy.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0, b'X']).await.unwrap();
        proxy
    });

    establish_socks5_tunnel(&mut client, &target, Some(&credentials())).await.unwrap();
    assert_eq!(client.read_u8().await.unwrap(), b'X');
    drop(server.await.unwrap());
}

#[tokio::test]
async fn a_socks_refusal_fails_the_tunnel() {
    let (mut client, mut proxy) = tokio::io::duplex(1024);
    let target = ProxyTarget::new(TargetScheme::Wss, "a.io", Some(443));
    let server = tokio::spawn(async move {
        let mut greeting = [0_u8; 3];
        proxy.read_exact(&mut greeting).await.unwrap();
        proxy.write_all(&[5, 0]).await.unwrap();
        let mut request = [0_u8; 3 + 8];
        proxy.read_exact(&mut request).await.unwrap();
        proxy.write_all(&[5, 5, 0, 1]).await.unwrap();
        proxy
    });
    let refused = establish_socks5_tunnel(&mut client, &target, None).await.unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::ConnectionRefused);
    drop(server.await.unwrap());
}

#[test]
fn a_cell_keeps_its_value_within_a_generation_and_rebuilds_after() {
    let cell = GenerationCell::<u32>::new();
    let builds = AtomicU64::new(0);
    let build = || {
        builds.fetch_add(1, Ordering::SeqCst);
        Some(7)
    };
    assert_eq!(cell.get_or_build(0, build), Some(7));
    assert_eq!(cell.get_or_build(0, build), Some(7));
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert_eq!(cell.get_or_build(1, build), Some(7));
    assert_eq!(builds.load(Ordering::SeqCst), 2);
}

#[test]
fn a_failed_build_is_tried_again() {
    let cell = GenerationCell::<u32>::new();
    assert_eq!(cell.get_or_build(0, || None), None);
    assert_eq!(cell.get_or_build(0, || Some(1)), Some(1));
}

#[test]
fn a_reconnect_reads_the_new_proxy_and_moves_the_generation() {
    let answer = Arc::new(Mutex::new(vec![SystemProxyEntry::Direct]));
    let system = SystemProxy::new(SwitchableSource(Arc::clone(&answer)));
    let routes = Mutex::new(SocketRoutes::default());
    let generation = AtomicU64::new(0);
    let target = ProxyTarget::new(TargetScheme::Wss, "gateway.example.com", None);

    assert_eq!(reconnect_route(&system, &routes, &generation, &target), ProxyRoute::Direct);
    assert_eq!(generation.load(Ordering::SeqCst), 0, "the first route is not a change");

    assert_eq!(reconnect_route(&system, &routes, &generation, &target), ProxyRoute::Direct);
    assert_eq!(generation.load(Ordering::SeqCst), 0, "the same route is not a change");

    let endpoint = ProxyEndpoint::new(ProxyProtocol::Http, "proxy.example", 8080);
    *answer.lock().unwrap() = vec![SystemProxyEntry::Proxy(endpoint.clone())];
    assert_eq!(reconnect_route(&system, &routes, &generation, &target), ProxyRoute::Via(endpoint));
    assert_eq!(generation.load(Ordering::SeqCst), 1);
}
