use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::StreamExt;
use rise_canon::rise_upload;
use rise_core::foundation::foundation_url::Url;
use tokio::runtime::Handle;
use tokio::task::AbortHandle;

use super::rise_url_session_hooks::rise_url_session_hooks;
use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

/// error codes the engine reads or produces
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum URLErrorCode {
    Unknown,
    Cancelled,
    BadURL,
    TimedOut,
    CannotFindHost,
    CannotConnectToHost,
    NetworkConnectionLost,
    DnsLookupFailed,
    NotConnectedToInternet,
    SecureConnectionFailed,
    DataNotAllowed,
}

/// url error
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
#[error("URLError({code:?})")]
pub struct URLError {
    pub code: URLErrorCode,
}

impl URLError {
    pub const fn new(code: URLErrorCode) -> Self {
        Self { code }
    }

    fn from_reqwest(error: &reqwest::Error) -> Self {
        let code = if error.is_timeout() {
            URLErrorCode::TimedOut
        } else if error.is_builder() {
            URLErrorCode::BadURL
        } else if error.is_connect() {
            URLErrorCode::CannotConnectToHost
        } else {
            URLErrorCode::NetworkConnectionLost
        };
        Self::new(code)
    }
}

/// url request
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct URLRequest {
    pub url: Url,
    pub http_method: String,
    pub http_body: Option<Vec<u8>>,
    pub timeout_interval: Duration,
    header_fields: Vec<(String, String)>,
}

impl URLRequest {
    pub const DEFAULT_TIMEOUT_INTERVAL: Duration = Duration::from_secs(60);

    /// new request, GET with no body and the default timeout
    pub fn new(url: Url) -> Self {
        Self {
            url,
            http_method: "GET".to_owned(),
            http_body: None,
            timeout_interval: Self::DEFAULT_TIMEOUT_INTERVAL,
            header_fields: Vec::new(),
        }
    }

    /// sets a header, replaces one with the same name in any case
    pub fn set_value(&mut self, value: &str, for_http_header_field: &str) {
        self.header_fields
            .retain(|(name, _)| !name.eq_ignore_ascii_case(for_http_header_field));
        self.header_fields
            .push((for_http_header_field.to_owned(), value.to_owned()));
    }

    pub fn value(&self, for_http_header_field: &str) -> Option<&str> {
        self.header_fields
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(for_http_header_field))
            .map(|(_, value)| value.as_str())
    }

    pub fn all_http_header_fields(&self) -> &[(String, String)] {
        &self.header_fields
    }
}

/// http response, the status is all the engine reads
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HTTPURLResponse {
    pub status_code: i64,
}

/// delegate told about body bytes sent
pub trait URLSessionTaskDelegate: Send + Sync {
    fn url_session(
        &self,
        bytes_sent: i64,
        total_bytes_sent: i64,
        total_bytes_expected_to_send: i64,
    );
}

/// url session
pub struct URLSession {
    client: Option<reqwest::Client>,
}

struct URLSessionTransferAbortOnDrop(AbortHandle);

impl Drop for URLSessionTransferAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl URLSession {
    /// the shared session
    pub fn shared() -> &'static URLSession {
        static SHARED: OnceLock<URLSession> = OnceLock::new();
        SHARED.get_or_init(|| URLSession {
            client: reqwest::Client::builder().build().ok(),
        })
    }

    pub async fn data_for(
        &self,
        request: URLRequest,
    ) -> Result<(Vec<u8>, HTTPURLResponse), URLError> {
        self.send(request, None).await
    }

    pub async fn data_for_with_delegate(
        &self,
        request: URLRequest,
        delegate: Arc<dyn URLSessionTaskDelegate>,
    ) -> Result<(Vec<u8>, HTTPURLResponse), URLError> {
        self.send(request, Some(delegate)).await
    }

    async fn send(
        &self,
        request: URLRequest,
        delegate: Option<Arc<dyn URLSessionTaskDelegate>>,
    ) -> Result<(Vec<u8>, HTTPURLResponse), URLError> {
        let client = match rise_url_session_hooks() {
            Some(hooks) => (hooks.client)(),
            None => self.client.clone(),
        };
        let Some(client) = client else {
            return Err(URLError::new(URLErrorCode::SecureConnectionFailed));
        };
        let transfer = Self::transfer(client, request, delegate);
        if Handle::try_current().is_ok() {
            return transfer.await;
        }
        let task = RiseAccountRuntime::current().spawn(transfer);
        let _abort_on_drop = URLSessionTransferAbortOnDrop(task.abort_handle());
        task.await
            .unwrap_or(Err(URLError::new(URLErrorCode::Cancelled)))
    }

    async fn transfer(
        client: reqwest::Client,
        request: URLRequest,
        delegate: Option<Arc<dyn URLSessionTaskDelegate>>,
    ) -> Result<(Vec<u8>, HTTPURLResponse), URLError> {
        let method = reqwest::Method::from_bytes(request.http_method.as_bytes())
            .map_err(|_| URLError::new(URLErrorCode::BadURL))?;
        let mut builder = client
            .request(method, request.url.absolute_string())
            .timeout(request.timeout_interval);
        for (name, value) in &request.header_fields {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let Some(body) = request.http_body {
            let expected = i64::try_from(body.len()).unwrap_or(i64::MAX);
            builder = match delegate {
                Some(delegate) => {
                    let piece_bytes =
                        usize::try_from(rise_upload::PROGRESS_SOCKET_SEND_BUFFER_BYTES)
                            .unwrap_or(1)
                            .max(1);
                    let sized = builder.header("Content-Length", expected.to_string());
                    let pieces: Vec<Vec<u8>> = body
                        .chunks(piece_bytes)
                        .map(<[u8]>::to_vec)
                        .collect();
                    let mut total_sent: i64 = 0;
                    let stream = futures::stream::iter(pieces).map(move |piece| {
                        let sent = i64::try_from(piece.len()).unwrap_or(i64::MAX);
                        total_sent = total_sent.saturating_add(sent);
                        delegate.url_session(sent, total_sent, expected);
                        Ok::<Vec<u8>, std::io::Error>(piece)
                    });
                    sized.body(reqwest::Body::wrap_stream(stream))
                }
                None => builder.body(body),
            };
        }
        let response = builder
            .send()
            .await
            .map_err(|error| URLError::from_reqwest(&error))?;
        let status_code = i64::from(response.status().as_u16());
        let data = response
            .bytes()
            .await
            .map_err(|error| URLError::from_reqwest(&error))?;
        Ok((data.to_vec(), HTTPURLResponse { status_code }))
    }
}

#[cfg(test)]
#[path = "rise_url_session_tests.rs"]
mod tests;
