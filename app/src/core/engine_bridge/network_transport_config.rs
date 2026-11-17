use std::collections::HashMap;
use std::io;
use std::net::Ipv6Addr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Once, OnceLock};
use std::time::Duration;

use base64::Engine;
use reqwest::{Client, ClientBuilder};
use rise_platform::network_proxy::{
    ProxyCredentials, ProxyEndpoint, ProxyProtocol, ProxyRoute, ProxyTarget, SystemProxy,
    TargetScheme, system_proxy,
};
use rise_platform::system_activity::system_activity::{SystemActivityEvent, watch};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio_tungstenite::tungstenite::error::UrlError;
use tokio_tungstenite::tungstenite::handshake::client::{Request, Response};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

const PROXY_TUNNEL_TIMEOUT: Duration = Duration::from_secs(30);

const MAXIMUM_CONNECT_RESPONSE_BYTES: usize = 16 * 1024;

const SOCKS_VERSION: u8 = 5;
const SOCKS_AUTH_VERSION: u8 = 1;
const SOCKS_METHOD_NONE: u8 = 0;
const SOCKS_METHOD_PASSWORD: u8 = 2;
const SOCKS_COMMAND_CONNECT: u8 = 1;
const SOCKS_ADDRESS_V4: u8 = 1;
const SOCKS_ADDRESS_DOMAIN: u8 = 3;
const SOCKS_ADDRESS_V6: u8 = 4;

static TRANSPORT_GENERATION: AtomicU64 = AtomicU64::new(0);

static SOCKET_ROUTES: LazyLock<Mutex<SocketRoutes>> = LazyLock::new(|| Mutex::new(SocketRoutes::default()));

static TESTER_NETWORK_DOWN: AtomicBool = AtomicBool::new(false);

const TESTER_DEAD_PROXY: &str = "http://127.0.0.1:9";

/// true while the driven session network is switched off
/// constant false outside a debug build with dev-drive
pub fn tester_network_down() -> bool {
    crate::app::dev_drive::ENABLED && TESTER_NETWORK_DOWN.load(Ordering::Acquire)
}

pub fn set_tester_network_down(down: bool) -> bool {
    if !crate::app::dev_drive::ENABLED {
        return false;
    }
    TESTER_NETWORK_DOWN.store(down, Ordering::Release);
    network_changed();
    true
}

/// value built for one transport generation and kept until the generation moves
pub struct GenerationCell<T> {
    current: Mutex<Option<(u64, T)>>,
}

impl<T> GenerationCell<T> {
    pub const fn new() -> Self {
        Self { current: Mutex::new(None) }
    }
}

impl<T> Default for GenerationCell<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> GenerationCell<T> {
    /// value of the generation, a failed build is tried again by the next caller
    pub fn get_or_build(&self, generation: u64, build: impl FnOnce() -> Option<T>) -> Option<T> {
        let mut current = self.current.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((built_for, value)) = current.as_ref()
            && *built_for == generation
        {
            return Some(value.clone());
        }
        let value = build()?;
        *current = Some((generation, value.clone()));
        Some(value)
    }
}

/// one reqwest client with its own connection pool, always on the current proxy and trust settings
/// use in a static or field instead of a client, current is a cheap clone
pub struct NetworkClientSlot {
    configure: fn(ClientBuilder) -> ClientBuilder,
    cell: GenerationCell<Client>,
}

impl NetworkClientSlot {
    pub const fn new(configure: fn(ClientBuilder) -> ClientBuilder) -> Self {
        Self { configure, cell: GenerationCell::new() }
    }

    /// redirects followed by reqwest, default of a session without a delegate
    pub const fn standard() -> Self {
        Self::new(unchanged)
    }

    /// redirects returned to the caller, for sessions whose delegate decides each hop
    pub const fn without_redirects() -> Self {
        Self::new(no_redirects)
    }

    /// none only when the client cant be built, the transfer then fails as a broken connection
    pub fn current(&self) -> Option<Client> {
        self.cell.get_or_build(TRANSPORT_GENERATION.load(Ordering::Acquire), || {
            match (self.configure)(client_builder()).build() {
                Ok(client) => Some(client),
                Err(error) => {
                    tracing::error!(target: "riseonly", "the HTTP client could not be built: {error}");
                    None
                }
            }
        })
    }
}

fn unchanged(builder: ClientBuilder) -> ClientBuilder {
    builder
}

fn no_redirects(builder: ClientBuilder) -> ClientBuilder {
    builder.redirect(reqwest::redirect::Policy::none())
}

pub fn client_builder() -> ClientBuilder {
    let builder = Client::builder().proxy(reqwest::Proxy::custom(|url| {
        if tester_network_down() {
            return reqwest::Url::parse(TESTER_DEAD_PROXY).ok();
        }
        proxy_url_for(system_proxy(), url)
    }));
    match platform_tls_config() {
        Some(config) => builder.use_preconfigured_tls(config.as_ref().clone()),
        None => builder,
    }
}

/// proxy of one destination as reqwest takes it, none is a direct connection
pub fn proxy_url_for(system: &SystemProxy, url: &reqwest::Url) -> Option<reqwest::Url> {
    let scheme = TargetScheme::parse(url.scheme())?;
    let target = ProxyTarget::new(scheme, url.host_str()?, url.port());
    match without_stalling(|| system.route_for(&target)) {
        ProxyRoute::Direct => None,
        ProxyRoute::Via(endpoint) => endpoint_url(&endpoint),
    }
}

/// route lookup on a runtime thread
/// reqwest asks synchronously inside its connector
/// and the first lookup may run a pac file for seconds
fn without_stalling<T>(lookup: impl FnOnce() -> T) -> T {
    match Handle::try_current() {
        Ok(handle) if matches!(handle.runtime_flavor(), RuntimeFlavor::MultiThread) => {
            tokio::task::block_in_place(lookup)
        }
        _ => lookup(),
    }
}

/// url of a proxy with credentials in the userinfo
/// thats where reqwest reads them for basic auth and socks password
pub fn endpoint_url(endpoint: &ProxyEndpoint) -> Option<reqwest::Url> {
    let mut url = reqwest::Url::parse(&endpoint.url()).ok()?;
    if let Some(credentials) = &endpoint.credentials {
        url.set_username(&credentials.user).ok()?;
        url.set_password(Some(&credentials.password)).ok()?;
    }
    Some(url)
}

/// rustls config with the os verifier, shared by http and sockets
/// none if the verifier cant start, then transports keep their own roots
pub fn platform_tls_config() -> Option<Arc<rustls::ClientConfig>> {
    static SHARED: OnceLock<Option<Arc<rustls::ClientConfig>>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .and_then(|builder| builder.with_platform_verifier())
                .map(|builder| builder.with_no_client_auth());
            match config {
                Ok(config) => Some(Arc::new(config)),
                Err(error) => {
                    tracing::error!(target: "riseonly", "the system certificate verifier did not start: {error}");
                    None
                }
            }
        })
        .clone()
}

/// the next client out of every slot is a new one and the next route lookup asks the system
/// transfers in flight finish on the client they hold
pub fn network_changed() {
    system_proxy().forget();
    advance_generation(&TRANSPORT_GENERATION);
}

pub fn advance_generation(generation: &AtomicU64) {
    generation.fetch_add(1, Ordering::AcqRel);
}

/// starts following network path and system proxy changes for the life of the process
/// safe to call again
pub fn watch_network_changes() {
    static WATCHING: Once = Once::new();
    WATCHING.call_once(|| {
        let watches = watch(|event| {
            if matches!(event, SystemActivityEvent::NetworkPath { .. }) {
                network_changed();
            }
        });
        for gap in watches.gaps() {
            tracing::warn!(target: "riseonly", "the transport does not hear network changes: {gap}");
        }
        std::mem::forget(watches);
        match system_proxy().watch_settings(network_changed) {
            Ok(settings_watch) => std::mem::forget(settings_watch),
            Err(error) => {
                tracing::warn!(target: "riseonly", "the transport does not hear proxy settings changes: {error}");
            }
        }
    });
}

/// the last route each socket destination used
#[derive(Default)]
pub struct SocketRoutes {
    routes: HashMap<ProxyTarget, ProxyRoute>,
}

impl SocketRoutes {
    /// true when the destination had another route before
    pub fn record(&mut self, target: ProxyTarget, route: ProxyRoute) -> bool {
        self.routes.insert(target, route.clone()).is_some_and(|previous| previous != route)
    }
}

/// route of a socket about to connect, read from the system and not from the cache
pub fn reconnect_route(
    system: &SystemProxy,
    routes: &Mutex<SocketRoutes>,
    generation: &AtomicU64,
    target: &ProxyTarget,
) -> ProxyRoute {
    system.forget();
    let route = system.route_for(target);
    let changed = routes
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .record(target.clone(), route.clone());
    if changed {
        advance_generation(generation);
    }
    route
}

/// connect_async_with_config on the system proxy and system trust roots
/// same stream and errors so a caller reads a refused upgrade as before
pub async fn connect_websocket(
    request: Request,
    config: Option<WebSocketConfig>,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), tokio_tungstenite::tungstenite::Error> {
    use tokio_tungstenite::tungstenite::Error;

    let scheme = request
        .uri()
        .scheme_str()
        .and_then(TargetScheme::parse)
        .ok_or(Error::Url(UrlError::UnsupportedUrlScheme))?;
    let host = request.uri().host().ok_or(Error::Url(UrlError::NoHostName))?;
    let target = ProxyTarget::new(scheme, host, request.uri().port_u16());

    let route = without_stalling(|| {
        reconnect_route(system_proxy(), &SOCKET_ROUTES, &TRANSPORT_GENERATION, &target)
    });

    let stream = open_stream(&route, &target).await.map_err(Error::Io)?;
    let connector = if scheme.is_secure() {
        platform_tls_config().map(Connector::Rustls)
    } else {
        None
    };
    tokio_tungstenite::client_async_tls_with_config(request, stream, config, connector).await
}

/// tcp stream of a socket whose tls and upgrade live in a crate with no connector hook (livekit signal)
/// same route per destination as connect_websocket
pub async fn connect_routed_stream(host: String, port: u16, secure: bool) -> io::Result<TcpStream> {
    let scheme = if secure { TargetScheme::Wss } else { TargetScheme::Ws };
    let target = ProxyTarget::new(scheme, &host, Some(port));
    let route = without_stalling(|| {
        reconnect_route(system_proxy(), &SOCKET_ROUTES, &TRANSPORT_GENERATION, &target)
    });
    open_stream(&route, &target).await
}

async fn open_stream(route: &ProxyRoute, target: &ProxyTarget) -> io::Result<TcpStream> {
    if tester_network_down() {
        return Err(io::Error::new(
            io::ErrorKind::NetworkUnreachable,
            "the network is switched off (dev-drive)",
        ));
    }
    let endpoint = match route {
        ProxyRoute::Direct => return TcpStream::connect((target.host.as_str(), target.port)).await,
        ProxyRoute::Via(endpoint) => endpoint,
    };
    let tunnel = async {
        let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port)).await?;
        match endpoint.protocol {
            ProxyProtocol::Http => {
                establish_http_tunnel(&mut stream, target, endpoint.credentials.as_ref()).await?;
            }
            ProxyProtocol::Socks5 => {
                establish_socks5_tunnel(&mut stream, target, endpoint.credentials.as_ref()).await?;
            }
        }
        Ok::<TcpStream, io::Error>(stream)
    };
    tokio::time::timeout(PROXY_TUNNEL_TIMEOUT, tunnel)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the proxy did not open the tunnel in time"))?
}

/// host:port of a connect line, an ipv6 literal in brackets
pub fn tunnel_authority(target: &ProxyTarget) -> String {
    if target.host.parse::<Ipv6Addr>().is_ok() {
        format!("[{}]:{}", target.host, target.port)
    } else {
        format!("{}:{}", target.host, target.port)
    }
}

/// the request that opens an http tunnel
pub fn connect_request(target: &ProxyTarget, credentials: Option<&ProxyCredentials>) -> String {
    let authority = tunnel_authority(target);
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some(credentials) = credentials {
        let pair = format!("{}:{}", credentials.user, credentials.password);
        let encoded = base64::engine::general_purpose::STANDARD.encode(pair);
        request.push_str(&format!("Proxy-Authorization: Basic {encoded}\r\n"));
    }
    request.push_str("\r\n");
    request
}

/// sends connect and reads the answer up to its blank line
pub async fn establish_http_tunnel<S>(
    stream: &mut S,
    target: &ProxyTarget,
    credentials: Option<&ProxyCredentials>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    stream.write_all(connect_request(target, credentials).as_bytes()).await?;
    stream.flush().await?;

    let mut response = Vec::with_capacity(256);
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() >= MAXIMUM_CONNECT_RESPONSE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the proxy answer has no end"));
        }
        response.push(stream.read_u8().await?);
    }
    check_connect_response(&response)
}

/// a tunnel is open on any 2xx
pub fn check_connect_response(response: &[u8]) -> io::Result<()> {
    let text = String::from_utf8_lossy(response);
    let status = text
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the proxy answer has no status"))?;
    match status {
        200..=299 => Ok(()),
        407 => Err(io::Error::new(io::ErrorKind::PermissionDenied, "the proxy asks for credentials")),
        other => Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("the proxy refused the tunnel with status {other}"),
        )),
    }
}

/// address part of a socks5 request, the proxy resolves names
pub fn socks5_address(target: &ProxyTarget) -> io::Result<Vec<u8>> {
    let mut address = Vec::with_capacity(target.host.len() + 4);
    if let Ok(v4) = target.host.parse::<std::net::Ipv4Addr>() {
        address.push(SOCKS_ADDRESS_V4);
        address.extend_from_slice(&v4.octets());
    } else if let Ok(v6) = target.host.parse::<Ipv6Addr>() {
        address.push(SOCKS_ADDRESS_V6);
        address.extend_from_slice(&v6.octets());
    } else {
        let length = u8::try_from(target.host.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the host name is too long for SOCKS"))?;
        address.push(SOCKS_ADDRESS_DOMAIN);
        address.push(length);
        address.extend_from_slice(target.host.as_bytes());
    }
    address.extend_from_slice(&target.port.to_be_bytes());
    Ok(address)
}

/// socks5 tunnel with the password exchange when the system keeps credentials
pub async fn establish_socks5_tunnel<S>(
    stream: &mut S,
    target: &ProxyTarget,
    credentials: Option<&ProxyCredentials>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let greeting: &[u8] = if credentials.is_some() {
        &[SOCKS_VERSION, 2, SOCKS_METHOD_NONE, SOCKS_METHOD_PASSWORD]
    } else {
        &[SOCKS_VERSION, 1, SOCKS_METHOD_NONE]
    };
    stream.write_all(greeting).await?;
    stream.flush().await?;

    let mut choice = [0_u8; 2];
    stream.read_exact(&mut choice).await?;
    if choice[0] != SOCKS_VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the proxy does not speak SOCKS 5"));
    }
    match (choice[1], credentials) {
        (SOCKS_METHOD_NONE, _) => {}
        (SOCKS_METHOD_PASSWORD, Some(credentials)) => authenticate_socks5(stream, credentials).await?,
        _ => {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the proxy accepts no offered method"));
        }
    }

    let mut request = vec![SOCKS_VERSION, SOCKS_COMMAND_CONNECT, 0];
    request.extend_from_slice(&socks5_address(target)?);
    stream.write_all(&request).await?;
    stream.flush().await?;

    let mut head = [0_u8; 4];
    stream.read_exact(&mut head).await?;
    if head[1] != 0 {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("the proxy refused the tunnel with reply {}", head[1]),
        ));
    }
    let bound_address_bytes = match head[3] {
        SOCKS_ADDRESS_V4 => 4,
        SOCKS_ADDRESS_V6 => 16,
        SOCKS_ADDRESS_DOMAIN => usize::from(stream.read_u8().await?),
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "the proxy answered an unknown address type")),
    };
    let mut bound = vec![0_u8; bound_address_bytes + 2];
    stream.read_exact(&mut bound).await?;
    Ok(())
}

async fn authenticate_socks5<S>(stream: &mut S, credentials: &ProxyCredentials) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let too_long = || io::Error::new(io::ErrorKind::InvalidInput, "the proxy credentials are too long for SOCKS");
    let user = u8::try_from(credentials.user.len()).map_err(|_| too_long())?;
    let password = u8::try_from(credentials.password.len()).map_err(|_| too_long())?;

    let mut request = Vec::with_capacity(3 + credentials.user.len() + credentials.password.len());
    request.push(SOCKS_AUTH_VERSION);
    request.push(user);
    request.extend_from_slice(credentials.user.as_bytes());
    request.push(password);
    request.extend_from_slice(credentials.password.as_bytes());
    stream.write_all(&request).await?;
    stream.flush().await?;

    let mut reply = [0_u8; 2];
    stream.read_exact(&mut reply).await?;
    if reply[1] != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the proxy refused the credentials"));
    }
    Ok(())
}
