use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;

use super::rise_wire::WireError;
use super::rise_wire_contracts::{RemoteError, ReplayPolicy};

/// Verb for an [`HttpDescriptor`].
///
/// HTTP carries only what the socket's action gate refuses to an unauthenticated
/// caller (`auth.refresh_token`, tag availability). A method the socket can carry
/// must never appear here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HttpVerb {
    Get,
    Post,
}

impl HttpVerb {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HttpDescriptor {
    pub path: &'static str,
    pub verb: HttpVerb,
    pub timeout: Duration,
    pub replay: ReplayPolicy,
}

impl HttpDescriptor {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    pub const fn read(path: &'static str) -> Self {
        Self {
            path,
            verb: HttpVerb::Post,
            timeout: Self::DEFAULT_TIMEOUT,
            replay: ReplayPolicy::ReadOnly,
        }
    }

    pub const fn mutation(path: &'static str) -> Self {
        Self {
            path,
            verb: HttpVerb::Post,
            timeout: Self::DEFAULT_TIMEOUT,
            replay: ReplayPolicy::Never,
        }
    }

    pub const fn idempotent_mutation(path: &'static str, field: &'static str) -> Self {
        Self {
            path,
            verb: HttpVerb::Post,
            timeout: Self::DEFAULT_TIMEOUT,
            replay: ReplayPolicy::IdempotencyKey { field },
        }
    }

    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub const fn with_verb(mut self, verb: HttpVerb) -> Self {
        self.verb = verb;
        self
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HttpCall {
    pub path: &'static str,
    pub verb: HttpVerb,
    pub body: String,
    pub timeout: Duration,
    pub authorization: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
}

/// Performs the request. The host supplies it, so TLS, proxy and cookie policy
/// stay outside the engine.
pub trait HttpSender: Send + Sync {
    fn send(&self, call: HttpCall) -> BoxFuture<'static, Result<HttpReply, WireError>>;
}

pub struct RiseHttp {
    sender: Arc<dyn HttpSender>,
}

impl RiseHttp {
    pub fn new(sender: Arc<dyn HttpSender>) -> Self {
        Self { sender }
    }

    pub async fn call<T: Serialize>(
        &self,
        descriptor: &HttpDescriptor,
        request: T,
        authorization: Option<String>,
    ) -> Result<Value, WireError> {
        let body = serde_json::to_string(&request)
            .map_err(|error| WireError::Encode(error.to_string()))?;

        let reply = self
            .sender
            .send(HttpCall {
                path: descriptor.path,
                verb: descriptor.verb,
                body,
                timeout: descriptor.timeout,
                authorization,
            })
            .await?;

        decode_reply(reply)
    }
}

// A non-2xx keeps the gateway's own message: 401-expired and 401-reused recover differently.
fn decode_reply(reply: HttpReply) -> Result<Value, WireError> {
    let payload: Value = if reply.body.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&reply.body).map_err(|error| WireError::Decode(error.to_string()))?
    };

    if (200..300).contains(&reply.status) {
        return Ok(payload);
    }

    let message = payload
        .get("error")
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("HTTP {}", reply.status));

    Err(WireError::Remote(RemoteError {
        code: Some(reply.status.to_string()),
        message,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSender {
        calls: Mutex<Vec<HttpCall>>,
        reply: Mutex<Option<Result<HttpReply, WireError>>>,
    }

    impl RecordingSender {
        fn answering(status: u16, body: &str) -> Arc<Self> {
            let sender = Arc::new(Self::default());
            *sender.reply.lock().unwrap() = Some(Ok(HttpReply {
                status,
                body: body.to_owned(),
            }));
            sender
        }

        fn last(&self) -> HttpCall {
            self.calls.lock().unwrap().last().cloned().expect("no call")
        }
    }

    impl HttpSender for RecordingSender {
        fn send(&self, call: HttpCall) -> BoxFuture<'static, Result<HttpReply, WireError>> {
            self.calls.lock().unwrap().push(call);
            let reply = self.reply.lock().unwrap().take().unwrap_or(Ok(HttpReply {
                status: 200,
                body: "{}".into(),
            }));
            Box::pin(async move { reply })
        }
    }

    const REFRESH: HttpDescriptor = HttpDescriptor::idempotent_mutation("/auth/refresh", "id");

    #[tokio::test]
    async fn a_call_carries_the_descriptor_and_returns_the_decoded_payload() {
        let sender = RecordingSender::answering(200, r#"{"accessToken":"a"}"#);
        let http = RiseHttp::new(Arc::clone(&sender) as Arc<dyn HttpSender>);

        let payload = http
            .call(&REFRESH, json!({"refresh_token": "r"}), None)
            .await
            .unwrap();

        assert_eq!(payload["accessToken"], "a");
        let call = sender.last();
        assert_eq!(call.path, "/auth/refresh");
        assert_eq!(call.verb, HttpVerb::Post);
        assert_eq!(call.timeout, HttpDescriptor::DEFAULT_TIMEOUT);
        assert!(call.body.contains("refresh_token"));
        assert_eq!(call.authorization, None);
    }

    #[tokio::test]
    async fn an_authorization_travels_on_the_call_rather_than_in_the_body() {
        let sender = RecordingSender::answering(200, "{}");
        let http = RiseHttp::new(Arc::clone(&sender) as Arc<dyn HttpSender>);

        http.call(&REFRESH, json!({}), Some("Bearer t".into()))
            .await
            .unwrap();

        assert_eq!(sender.last().authorization.as_deref(), Some("Bearer t"));
        assert!(
            !sender.last().body.contains("Bearer"),
            "a credential in the body would end up in a request log"
        );
    }

    #[tokio::test]
    async fn a_refusal_keeps_the_server_message_so_the_caller_can_tell_them_apart() {
        let sender = RecordingSender::answering(401, r#"{"error":"Refresh token reused"}"#);
        let http = RiseHttp::new(sender as Arc<dyn HttpSender>);

        match http.call(&REFRESH, json!({}), None).await {
            Err(WireError::Remote(remote)) => {
                assert_eq!(remote.code.as_deref(), Some("401"));
                assert_eq!(remote.message, "Refresh token reused");
            }
            other => panic!("expected a remote error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_refusal_with_no_body_still_names_its_status() {
        let sender = RecordingSender::answering(503, "");
        let http = RiseHttp::new(sender as Arc<dyn HttpSender>);

        match http.call(&REFRESH, json!({}), None).await {
            Err(WireError::Remote(remote)) => assert_eq!(remote.message, "HTTP 503"),
            other => panic!("expected a remote error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_success_with_an_unparseable_body_is_a_decode_error_not_a_silent_null() {
        let sender = RecordingSender::answering(200, "<html>gateway</html>");
        let http = RiseHttp::new(sender as Arc<dyn HttpSender>);

        assert!(matches!(
            http.call(&REFRESH, json!({}), None).await,
            Err(WireError::Decode(_))
        ));
    }

    #[test]
    fn a_plain_mutation_defaults_to_no_replay() {
        assert_eq!(
            HttpDescriptor::mutation("/auth/logout").replay,
            ReplayPolicy::Never
        );
        assert!(
            HttpDescriptor::read("/auth/check-tag")
                .replay
                .permits_replay()
        );
    }
}
