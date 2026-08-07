use std::sync::Arc;

use futures::future::BoxFuture;
use rise_engine::{HttpDescriptor, MethodDescriptor, RiseHttp, RiseWire, WireError};
use serde_json::Value;

use crate::core::engine_bridge::SocketCredential;

/// Everything the auth repository is allowed to do to the outside world.
///
/// A trait rather than the concrete wire, for one reason: the repository owns
/// the rules that matter — which failure rotates a token, which one signs the
/// user out, what happens when two refreshes race — and none of them can be
/// tested against a real gateway. With this seam the whole repository runs in a
/// unit test with a scripted server.
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

    /// Re-handshakes the socket under a new identity.
    ///
    /// The gateway resolves the connection's user once, at upgrade, so this is
    /// the only way a rotated token takes effect — there is no in-band way to
    /// re-authenticate an open socket.
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

/// The envelope's timestamp. Wall clock, because the server reads it as one.
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
