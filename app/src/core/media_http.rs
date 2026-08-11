use std::sync::Arc;

use futures::AsyncReadExt;
use futures::future::BoxFuture;
use gpui::http_client::http::{HeaderValue, Request, Response};
use gpui::http_client::{AsyncBody, HttpClient, Url};

pub struct MediaHttpClient {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
    user_agent: HeaderValue,
    proxy: Option<Url>,
}

impl MediaHttpClient {
    const WORKER_THREADS: usize = 2;

    pub fn new(user_agent: &str) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(Self::WORKER_THREADS)
            .thread_name("rise-media-http")
            .enable_all()
            .build()?;

        Ok(Self {
            runtime,
            client: reqwest::Client::new(),
            user_agent: HeaderValue::from_str(user_agent)
                .unwrap_or_else(|_| HeaderValue::from_static("Riseonly")),
            proxy: None,
        })
    }

    pub fn optional(user_agent: &str) -> Option<Arc<dyn HttpClient>> {
        match Self::new(user_agent) {
            Ok(client) => Some(Arc::new(client)),
            Err(error) => {
                tracing::error!(
                    target: "riseonly",
                    "no media HTTP client ({error}); every remote image will draw its fallback"
                );
                None
            }
        }
    }
}

impl HttpClient for MediaHttpClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        self.proxy.as_ref()
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let handle = self.runtime.handle().clone();
        let client = self.client.clone();
        let user_agent = self.user_agent.clone();

        Box::pin(async move {
            let (parts, mut body) = request.into_parts();

            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes).await?;

            let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())?;
            let url = parts.uri.to_string();

            let mut outbound = client.request(method, url).body(bytes);
            for (name, value) in parts.headers.iter() {
                outbound = outbound.header(name.as_str(), value.as_bytes());
            }
            if !parts
                .headers
                .contains_key(gpui::http_client::http::header::USER_AGENT)
            {
                outbound = outbound.header("User-Agent", user_agent.as_bytes());
            }

            // reqwest must be polled on its own runtime; only the join handle may be awaited here.
            let response = handle.spawn(async move { outbound.send().await }).await??;

            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let payload = response.bytes().await?.to_vec();

            let mut builder = Response::builder().status(status);
            for (name, value) in headers.iter() {
                builder = builder.header(name.as_str(), value.as_bytes());
            }

            Ok(builder.body(AsyncBody::from(payload))?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_reports_the_product_rather_than_nothing() {
        let client = MediaHttpClient::new("Riseonly/0.1").expect("a runtime opens");
        assert_eq!(
            client.user_agent().map(|value| value.to_str().unwrap()),
            Some("Riseonly/0.1"),
            "a CDN that refuses agentless requests would fail every image"
        );
    }

    #[test]
    fn a_user_agent_that_cannot_be_a_header_falls_back_rather_than_panicking() {
        let client = MediaHttpClient::new("Rise\nonly").expect("a runtime opens");
        assert!(client.user_agent().is_some());
    }

    #[test]
    fn a_client_that_opens_is_handed_back_ready_to_install() {
        let client = MediaHttpClient::optional("Riseonly/0.1").expect("a runtime opens");
        assert_eq!(
            client.user_agent().map(|value| value.to_str().unwrap()),
            Some("Riseonly/0.1")
        );
        assert!(client.proxy().is_none());
    }
}
