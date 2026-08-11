use std::sync::Arc;

use rise_core::RequestIdAllocator;
use rise_engine::{RiseHttp, RiseWire};
use tokio::runtime::{Handle, Runtime};

use crate::core::config::Endpoints;

use super::http_host::ReqwestHttpSender;
use super::socket_host::SocketHost;
use super::socket_policy::{ConnectionState, SocketCredential, socket_url};

pub struct EngineBridge {
    runtime: Runtime,
    wire: Arc<RiseWire>,
    http: Arc<RiseHttp>,
    socket: SocketHost,
}

impl EngineBridge {
    pub fn new(endpoints: &Endpoints) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("rise-engine")
            .enable_all()
            .build()?;

        // One allocator for wire and heartbeat: a second counter would collide correlation ids.
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

    pub fn authenticator(&self) -> impl Fn(SocketCredential) + Send + Sync + 'static {
        let socket = self.socket.commands();
        move |credential| SocketHost::authenticate_through(&socket, credential)
    }

    pub fn shutdown(&self) {
        self.socket.shutdown();
    }
}

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
