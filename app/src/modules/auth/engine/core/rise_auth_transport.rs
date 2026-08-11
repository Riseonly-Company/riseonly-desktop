use std::sync::Arc;

use futures::future::BoxFuture;
use rise_engine::{HttpDescriptor, MethodDescriptor, RiseHttp, RiseWire, WireError};
use serde_json::Value;

use crate::core::engine_bridge::SocketCredential;

pub trait AuthTransport: Send + Sync {
    fn call(
        &self,
        descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>>;

    fn http(
        &self,
        descriptor: &'static HttpDescriptor,
        body: Value,
        authorization: Option<String>,
    ) -> BoxFuture<'static, Result<Value, WireError>>;

    // Gateway binds the socket's user at upgrade only; a rotated token needs a re-handshake.
    fn authenticate(&self, credential: SocketCredential);
}

pub struct LiveAuthTransport {
    wire: Arc<RiseWire>,
    http: Arc<RiseHttp>,
    reauthenticate: Arc<dyn Fn(SocketCredential) + Send + Sync>,
}

impl LiveAuthTransport {
    pub fn new(
        wire: Arc<RiseWire>,
        http: Arc<RiseHttp>,
        reauthenticate: Arc<dyn Fn(SocketCredential) + Send + Sync>,
    ) -> Self {
        Self {
            wire,
            http,
            reauthenticate,
        }
    }
}

impl AuthTransport for LiveAuthTransport {
    fn call(
        &self,
        descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        let wire = Arc::clone(&self.wire);
        Box::pin(async move { wire.call(descriptor, body, now_ms()).await })
    }

    fn http(
        &self,
        descriptor: &'static HttpDescriptor,
        body: Value,
        authorization: Option<String>,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        let http = Arc::clone(&self.http);
        Box::pin(async move { http.call(descriptor, body, authorization).await })
    }

    fn authenticate(&self, credential: SocketCredential) {
        (self.reauthenticate)(credential);
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_envelope_timestamp_is_a_wall_clock_in_milliseconds() {
        let stamp = now_ms();
        assert!(
            stamp > 1_700_000_000_000,
            "a seconds-resolution stamp would be rejected as decades in the past"
        );
    }
}
