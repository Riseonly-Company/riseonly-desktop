use std::sync::Arc;

use rise_core::RequestIdAllocator;
use rise_engine::{RiseHttp, RiseWire};
use tokio::runtime::{Handle, Runtime};

use crate::core::config::Endpoints;

use super::http_host::ReqwestHttpSender;
use super::socket_host::SocketHost;
use super::socket_policy::{ConnectionState, SocketCredential, socket_url};

/// The one seam between the modules and rise-engine.
///
/// Process-scoped on purpose, unlike the engine's database. The socket has to
/// exist BEFORE anyone is signed in — the gateway admits send_code, register and
/// login on an anonymous connection, and that is the whole sign-in path — so
/// transport cannot be a child of an account. What is account-scoped is the
/// store, and that lives on `AccountScope`, which is dropped whole on a switch.
pub struct EngineBridge {
    runtime: Runtime,
    wire: Arc<RiseWire>,
    http: Arc<RiseHttp>,
    socket: SocketHost,
}

impl EngineBridge {
    pub fn new(endpoints: &Endpoints) -> std::io::Result<Self> {
        // Its own runtime rather than gpui's executor: the engine's work is
        // socket and disk I/O, and putting it on the executor that also drives
        // the frame loop is how a slow response becomes a dropped frame.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("rise-engine")
            .enable_all()
            .build()?;

        // One allocator for the wire and the heartbeat both. The engine has its
        // own, but it is account-scoped and the socket outlives an account, so a
        // second counter here would eventually hand a heartbeat the correlation
        // id of a request that is still in flight.
        let requests = Arc::new(RequestIdAllocator::new());
        let (socket, wire) = SocketHost::spawn(
            runtime.handle(),
            socket_url(&endpoints.ws_url),
            SocketCredential::Anonymous,
            Arc::clone(&requests),
            process_seed(),
            |sender| Arc::new(RiseWire::new(sender, Arc::clone(&requests))),
        );

        let http = Arc::new(RiseHttp::new(Arc::new(ReqwestHttpSender::new(
            endpoints.api_base_url.clone(),
        ))));

        Ok(Self {
            runtime,
            wire,
            http,
            socket,
        })
    }

    pub fn handle(&self) -> Handle {
        self.runtime.handle().clone()
    }

    pub fn wire(&self) -> &Arc<RiseWire> {
        &self.wire
    }

    pub fn http(&self) -> &Arc<RiseHttp> {
        &self.http
    }

    pub fn subscribe_connection(&self) -> tokio::sync::watch::Receiver<ConnectionState> {
        self.socket.subscribe()
    }

    /// Points the socket at a new identity, or at none.
    ///
    /// Signing in, refreshing a token and signing out are all the same call: the
    /// gateway decides the connection's user once, at handshake, so a credential
    /// change is a reconnect and never an in-band message.
    /// A callable that re-handshakes, without a borrow of the bridge.
    ///
    /// The auth repository lives on the runtime and outlives any borrow a
    /// caller could hold, so it takes the ability to re-authenticate as a
    /// value rather than a reference to the whole transport.
    pub fn authenticator(&self) -> impl Fn(SocketCredential) + Send + Sync + 'static {
        let socket = self.socket.commands();
        move |credential| SocketHost::authenticate_through(&socket, credential)
    }

    pub fn shutdown(&self) {
        self.socket.shutdown();
    }
}

/// A per-process backoff seed, so two clients do not retry in lockstep.
///
/// Derived from the process id rather than a random number generator: it is
/// stable within a run, which keeps a reconnect schedule reproducible while
/// still differing between machines and between two launches on one machine.
fn process_seed() -> u64 {
    u64::from(std::process::id()).wrapping_mul(2_654_435_761)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seed_is_stable_within_a_process() {
        assert_eq!(process_seed(), process_seed());
    }

    #[test]
    fn the_seed_is_not_zero_which_would_disable_the_jitter_entirely() {
        assert_ne!(process_seed(), 0);
    }
}
