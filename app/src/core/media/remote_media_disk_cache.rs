use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, UNIX_EPOCH};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::AsyncApp;
use parking_lot::Mutex;
use rise_canon::{RiseLimits, rise_wire_canon};
use rise_core::foundation::foundation_crypto::sha256_hex;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::core::engine_bridge::network_transport_config::NetworkClientSlot;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode, URLRequest};
use rise_platform::AppPaths;
use rise_platform::file_manager::{
    CocoaError, CocoaErrorCode, DirectoryEnumerationOptions, FileAttributeType, FileManager,
};
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_failure_bridge::RiseMediaRefusal;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::SaiFetchHttpConfiguration;
use crate::core::lib::sai::sai_auth_hooks::{sai_auth_access_token, sai_auth_hooks, sai_auth_is_expired};
use crate::core::media::rise_progressive_data_fetcher::RiseMediaConnectivityRetry;

pub enum RemoteMediaDownloadRetryPolicy {}

impl RemoteMediaDownloadRetryPolicy {
    pub fn maximum_retries() -> i64 {
        RiseLimits::client_value("media_download_retry_count")
    }

    fn delay_milliseconds() -> i64 {
        RiseLimits::client_value("media_download_retry_delay_ms")
    }

    pub fn should_retry(error: &RemoteMediaTransferError, attempt: i64) -> bool {
        if attempt >= Self::maximum_retries() {
            return false;
        }
        Self::is_transient(error)
    }

    pub fn is_transient(error: &RemoteMediaTransferError) -> bool {
        let RemoteMediaTransferError::Url(error) = error else {
            return false;
        };
        matches!(
            error.code,
            URLErrorCode::NetworkConnectionLost
                | URLErrorCode::TimedOut
                | URLErrorCode::CannotConnectToHost
                | URLErrorCode::CannotFindHost
                | URLErrorCode::NotConnectedToInternet
                | URLErrorCode::DnsLookupFailed
        )
    }

    pub fn delay(for_retry: i64) -> Duration {
        let milliseconds = Self::delay_milliseconds().saturating_mul(for_retry).max(0);
        Duration::from_millis(u64::try_from(milliseconds).unwrap_or(0))
    }

    pub async fn retrying<Value, Operation, Attempt>(
        mut operation: Operation,
    ) -> Result<Value, RemoteMediaTransferError>
    where
        Operation: FnMut() -> Attempt,
        Attempt: Future<Output = Result<Value, RemoteMediaTransferError>>,
    {
        let mut attempt = 0;
        loop {
            match operation().await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    if !Self::should_retry(&error, attempt) {
                        return Err(error);
                    }
                    attempt += 1;
                    if !RiseRuntimeTimer::sleep(Self::delay(attempt)).await {
                        return Err(error);
                    }
                }
            }
        }
    }
}

/// keeps a cached file on disk while something outside the cache still reads it
/// the cache hands out real paths so a purge must not unlink them
pub struct RemoteMediaFileLease {
    digest: String,
}

impl RemoteMediaFileLease {
    fn new(digest: String) -> Self {
        RemoteMediaDiskCache::retain_lease(&digest);
        Self { digest }
    }
}

impl Drop for RemoteMediaFileLease {
    fn drop(&mut self) {
        RemoteMediaDiskCache::release_lease(&self.digest);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RemoteMediaDiskCacheError {
    InvalidSource,
    AccountChanged,
    InvalidResponse { status_code: Option<i64> },
    EmptyResponse,
    PayloadTooLarge,
    UnsupportedPayload,
    CacheUnavailable,
}

impl RemoteMediaDiskCacheError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::InvalidSource => "Invalid media URL".to_owned(),
            Self::AccountChanged => "Account changed while media was loading".to_owned(),
            Self::InvalidResponse { status_code: Some(status_code) } => {
                format!("Media download failed (HTTP {status_code})")
            }
            Self::InvalidResponse { status_code: None } => "Invalid media response".to_owned(),
            Self::EmptyResponse => "Downloaded media is empty".to_owned(),
            Self::PayloadTooLarge => "Media is too large to cache".to_owned(),
            Self::UnsupportedPayload => "Downloaded response is not a media file".to_owned(),
            Self::CacheUnavailable => "Media cache is unavailable".to_owned(),
        })
    }
}

impl fmt::Display for RemoteMediaDiskCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for RemoteMediaDiskCacheError {}

/// what a protected media route refused with, passed to the caller that owns an AsyncApp
#[derive(Clone, Debug)]
pub struct RemoteMediaRefusalReport {
    pub body: Vec<u8>,
    pub response: RemoteMediaHttpResponse,
    pub request: URLRequest,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
}

impl RemoteMediaRefusalReport {
    pub async fn publish(self, cx: &mut AsyncApp) {
        RiseMediaRefusal::publish(
            self.body,
            Some(self.response.status_code),
            Some(self.response.url),
            self.request,
            self.account_user_id,
            self.account_epoch,
            cx,
        )
        .await;
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum RemoteMediaTransferError {
    #[error(transparent)]
    Cache(#[from] RemoteMediaDiskCacheError),
    #[error(transparent)]
    Url(#[from] URLError),
    #[error(transparent)]
    File(#[from] CocoaError),
    #[error(transparent)]
    Cancelled(#[from] CancellationError),
    #[error("protected media refusal")]
    ProtectedRefusal(Arc<RemoteMediaRefusalReport>),
    #[error("{0}")]
    Other(Arc<anyhow::Error>),
}

impl RemoteMediaTransferError {
    /// error as anyhow, each case keeps its own type so callers can downcast
    pub fn into_anyhow(self) -> anyhow::Error {
        match self {
            Self::Cache(error) => anyhow::Error::new(error),
            Self::Url(error) => anyhow::Error::new(error),
            Self::File(error) => anyhow::Error::new(error),
            Self::Cancelled(error) => anyhow::Error::new(error),
            error @ (Self::ProtectedRefusal(_) | Self::Other(_)) => anyhow::Error::new(error),
        }
    }

    /// reads back the cases a downloader throws so the retry policy still sees the url error
    pub fn from_anyhow(error: anyhow::Error) -> Self {
        let error = match error.downcast::<Self>() {
            Ok(error) => return error,
            Err(error) => error,
        };
        let error = match error.downcast::<RemoteMediaDiskCacheError>() {
            Ok(error) => return Self::Cache(error),
            Err(error) => error,
        };
        let error = match error.downcast::<URLError>() {
            Ok(error) => return Self::Url(error),
            Err(error) => error,
        };
        let error = match error.downcast::<CocoaError>() {
            Ok(error) => return Self::File(error),
            Err(error) => error,
        };
        match error.downcast::<CancellationError>() {
            Ok(error) => Self::Cancelled(error),
            Err(error) => Self::Other(Arc::new(error)),
        }
    }

    fn file(error: std::io::Error, path: &Path) -> Self {
        Self::File(CocoaError {
            code: CocoaErrorCode::FileWriteUnknown,
            path: Some(path.to_path_buf()),
            underlying: Some(Arc::new(error)),
        })
    }

    pub fn url_error(error: &reqwest::Error) -> URLError {
        let code = if error.is_timeout() {
            URLErrorCode::TimedOut
        } else if error.is_builder() {
            URLErrorCode::BadURL
        } else if error.is_connect() {
            URLErrorCode::CannotConnectToHost
        } else {
            URLErrorCode::NetworkConnectionLost
        };
        URLError::new(code)
    }

    /// true for a connection that could not be made, the task is parked instead of failing
    pub(crate) fn is_waiting_for_connectivity(error: &URLError) -> bool {
        matches!(
            error.code,
            URLErrorCode::CannotConnectToHost
                | URLErrorCode::CannotFindHost
                | URLErrorCode::NotConnectedToInternet
                | URLErrorCode::DnsLookupFailed
        )
    }
}

/// http response of a media transfer, final url after redirects and status and header fields
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteMediaHttpResponse {
    pub url: Url,
    pub status_code: i64,
    header_fields: Vec<(String, String)>,
}

impl RemoteMediaHttpResponse {
    pub fn new(url: Url, status_code: i64, header_fields: Vec<(String, String)>) -> Self {
        Self {
            url,
            status_code,
            header_fields,
        }
    }

    pub fn from_reqwest(response: &reqwest::Response, url: Url) -> Self {
        let header_fields = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        Self::new(url, i64::from(response.status().as_u16()), header_fields)
    }

    pub fn value(&self, for_http_header_field: &str) -> Option<&str> {
        self.header_fields
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(for_http_header_field))
            .map(|(_, value)| value.as_str())
    }

    pub fn all_header_fields(&self) -> &[(String, String)] {
        &self.header_fields
    }

    /// expected content length, -1 when unknown
    pub fn expected_content_length(&self) -> i64 {
        self.value(reqwest::header::CONTENT_LENGTH.as_str())
            .and_then(|raw| trimming_whitespaces_and_newlines(raw).parse::<i64>().ok())
            .unwrap_or(-1)
    }

    /// whole object size from a 206 Content-Range header
    pub fn content_range_total(&self) -> Option<i64> {
        let raw = self.value(reqwest::header::CONTENT_RANGE.as_str())?;
        let (_, total) = raw.rsplit_once('/')?;
        trimming_whitespaces_and_newlines(total).parse::<i64>().ok()
    }

    /// file name from Content-Disposition, else the last path part
    pub fn suggested_filename(&self) -> Option<String> {
        if let Some(disposition) = self.value(reqwest::header::CONTENT_DISPOSITION.as_str()) {
            for part in disposition.split(';') {
                let part = trimming_whitespaces_and_newlines(part);
                let Some((name, value)) = part.split_once('=') else {
                    continue;
                };
                if !trimming_whitespaces_and_newlines(name).eq_ignore_ascii_case("filename") {
                    continue;
                }
                let value = trimming_whitespaces_and_newlines(value).trim_matches('"');
                if !value.is_empty() {
                    return Some(value.to_owned());
                }
            }
        }
        let last = self.url.last_path_component();
        (!last.is_empty() && last != "/").then_some(last)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemoteMediaDownloadProgress {
    pub downloaded_bytes: i64,
    pub total_bytes: Option<i64>,
}

impl RemoteMediaDownloadProgress {
    pub fn new(downloaded_bytes: i64, total_bytes: Option<i64>) -> Self {
        let downloaded = downloaded_bytes.max(0);
        let total_bytes = match total_bytes {
            Some(total) if total > 0 => Some(downloaded.max(total)),
            _ => None,
        };
        Self {
            downloaded_bytes: downloaded,
            total_bytes,
        }
    }

    pub fn fraction_completed(&self) -> Option<f64> {
        let total = self.total_bytes.filter(|total| *total > 0)?;
        Some((self.downloaded_bytes as f64 / total as f64).clamp(0.0, 1.0))
    }
}

impl Eq for RemoteMediaDownloadProgress {}

#[derive(Clone)]
pub struct RemoteMediaDownloadProgressSink {
    handler: Arc<dyn Fn(RemoteMediaDownloadProgress) + Send + Sync>,
}

impl RemoteMediaDownloadProgressSink {
    pub fn new(handler: impl Fn(RemoteMediaDownloadProgress) + Send + Sync + 'static) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }

    pub fn call_as_function(&self, progress: RemoteMediaDownloadProgress) {
        (self.handler)(progress);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemoteMediaDownloadProgressCoalescer {
    last_progress: Option<RemoteMediaDownloadProgress>,
    last_published_at: Option<f64>,
}

impl RemoteMediaDownloadProgressCoalescer {
    pub const MINIMUM_PUBLISH_INTERVAL: f64 = 0.1;
    pub const MINIMUM_BYTE_DELTA: i64 = 8 * 1_024;
    pub const MAXIMUM_SILENCE: f64 = 0.25;

    pub fn should_publish(&mut self, progress: RemoteMediaDownloadProgress, now: f64) -> bool {
        let (Some(last_progress), Some(last_published_at)) =
            (self.last_progress, self.last_published_at)
        else {
            self.record(progress, now);
            return true;
        };
        if progress == last_progress {
            return false;
        }
        let is_complete = progress
            .total_bytes
            .is_some_and(|total| progress.downloaded_bytes >= total);
        let elapsed = (now - last_published_at).max(0.0);
        if !(is_complete || elapsed >= Self::MINIMUM_PUBLISH_INTERVAL) {
            return false;
        }
        let byte_delta = (progress.downloaded_bytes - last_progress.downloaded_bytes).max(0);
        let total_became_known = last_progress.total_bytes.is_none() && progress.total_bytes.is_some();
        if !(is_complete
            || total_became_known
            || byte_delta >= Self::MINIMUM_BYTE_DELTA
            || elapsed >= Self::MAXIMUM_SILENCE)
        {
            return false;
        }
        self.record(progress, now);
        true
    }

    fn record(&mut self, progress: RemoteMediaDownloadProgress, now: f64) {
        self.last_progress = Some(progress);
        self.last_published_at = Some(now);
    }

    /// system uptime in seconds
    pub fn uptime() -> f64 {
        RiseClock::monotonic().as_secs_f64()
    }
}

type RemoteMediaSharedTask = Shared<BoxFuture<'static, Result<PathBuf, RemoteMediaTransferError>>>;

struct RemoteMediaSharedTaskWaiter;

impl RemoteMediaSharedTaskWaiter {
    async fn wait(task: RemoteMediaSharedTask) -> Result<PathBuf, RemoteMediaTransferError> {
        task.await
    }
}

/// removes a partial download unless it was handed over, a cancelled transfer drops it here
struct RemoteMediaTemporaryFile {
    path: PathBuf,
    is_owned: bool,
}

impl RemoteMediaTemporaryFile {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            is_owned: true,
        }
    }

    fn hand_over(mut self) -> PathBuf {
        self.is_owned = false;
        self.path.clone()
    }
}

impl Drop for RemoteMediaTemporaryFile {
    fn drop(&mut self) {
        if self.is_owned {
            FileManager.remove_item(&self.path).ok();
        }
    }
}

struct RemoteMediaDownloadOperation {
    maximum_byte_count: i64,
    progress_sink: RemoteMediaDownloadProgressSink,
    progress_coalescer: Mutex<RemoteMediaDownloadProgressCoalescer>,
}

pub type RemoteMediaDownloadOperationResult = (PathBuf, RemoteMediaHttpResponse);

static DOWNLOAD_SESSION: NetworkClientSlot = NetworkClientSlot::without_redirects();

impl RemoteMediaDownloadOperation {
    fn new(maximum_byte_count: i64, progress_sink: RemoteMediaDownloadProgressSink) -> Self {
        Self {
            maximum_byte_count: maximum_byte_count.max(1),
            progress_sink,
            progress_coalescer: Mutex::new(RemoteMediaDownloadProgressCoalescer::default()),
        }
    }

    async fn start(
        &self,
        request: URLRequest,
    ) -> Result<RemoteMediaDownloadOperationResult, RemoteMediaTransferError> {
        let owned = RemoteMediaTemporaryFile::new(rise_platform::paths::temporary_directory().join(format!(
            "riseonly-media-{}.download",
            Uuid::new_v4().uuid_string().to_lowercase()
        )));
        let response = self.transfer(&request, &owned.path).await?;
        Ok((owned.hand_over(), response))
    }

    async fn transfer(
        &self,
        request: &URLRequest,
        file_path: &Path,
    ) -> Result<RemoteMediaHttpResponse, RemoteMediaTransferError> {
        let Some(session) = DOWNLOAD_SESSION.current() else {
            return Err(URLError::new(URLErrorCode::SecureConnectionFailed).into());
        };
        let session = &session;
        let mut file = std::fs::File::create(file_path)
            .map_err(|error| RemoteMediaTransferError::file(error, file_path))?;
        let mut bytes_written: i64 = 0;
        let mut connectivity_retries: i64 = 0;
        let mut is_resumed_transfer = false;
        let mut resumes_without_progress: i64 = 0;
        let mut bytes_at_last_resume: i64 = 0;

        loop {
            let range_start = (is_resumed_transfer && bytes_written > 0).then_some(bytes_written);
            let sent =
                RiseMediaRedirectSessionDelegate::send(session, request, range_start, |_, _| {})
                    .await;
            let (mut response, url) = match sent {
                Ok(sent) => sent,
                Err(error) if RemoteMediaTransferError::is_waiting_for_connectivity(&error) => {
                    connectivity_retries += 1;
                    RiseMediaConnectivityRetry::shared()
                        .wait_for_retry(connectivity_retries)
                        .await;
                    continue;
                }
                Err(error) => {
                    let error = RemoteMediaTransferError::from(error);
                    if self.should_resume(
                        &error,
                        bytes_written,
                        &mut resumes_without_progress,
                        &mut bytes_at_last_resume,
                    ) {
                        is_resumed_transfer = true;
                        RiseRuntimeTimer::sleep(RemoteMediaDownloadRetryPolicy::delay(resumes_without_progress))
                            .await;
                        continue;
                    }
                    return Err(error);
                }
            };
            let head = RemoteMediaHttpResponse::from_reqwest(&response, url);
            if range_start.is_some() && head.status_code != 206 {
                file = std::fs::File::create(file_path)
                    .map_err(|error| RemoteMediaTransferError::file(error, file_path))?;
                bytes_written = 0;
                is_resumed_transfer = false;
            }
            let expected = if head.status_code == 206 {
                head.content_range_total().unwrap_or(-1)
            } else {
                head.expected_content_length()
            };
            if expected > self.maximum_byte_count {
                return Err(RemoteMediaDiskCacheError::PayloadTooLarge.into());
            }

            let cut = loop {
                let chunk = tokio::time::timeout(request.timeout_interval, response.chunk()).await;
                match chunk {
                    Ok(Ok(Some(bytes))) => {
                        file.write_all(&bytes)
                            .map_err(|error| RemoteMediaTransferError::file(error, file_path))?;
                        bytes_written =
                            bytes_written.saturating_add(i64::try_from(bytes.len()).unwrap_or(i64::MAX));
                        if bytes_written > self.maximum_byte_count {
                            return Err(RemoteMediaDiskCacheError::PayloadTooLarge.into());
                        }
                        let progress = RemoteMediaDownloadProgress::new(
                            bytes_written,
                            (expected > 0).then_some(expected),
                        );
                        let should_publish = self
                            .progress_coalescer
                            .lock()
                            .should_publish(progress, RemoteMediaDownloadProgressCoalescer::uptime());
                        if should_publish {
                            self.progress_sink.call_as_function(progress);
                        }
                    }
                    Ok(Ok(None)) => break None,
                    Ok(Err(error)) => break Some(RemoteMediaTransferError::url_error(&error)),
                    Err(_) => break Some(URLError::new(URLErrorCode::TimedOut)),
                }
            };

            if let Some(error) = cut {
                let error = RemoteMediaTransferError::from(error);
                if self.should_resume(
                    &error,
                    bytes_written,
                    &mut resumes_without_progress,
                    &mut bytes_at_last_resume,
                ) {
                    is_resumed_transfer = true;
                    RiseRuntimeTimer::sleep(RemoteMediaDownloadRetryPolicy::delay(resumes_without_progress))
                        .await;
                    continue;
                }
                return Err(error);
            }

            file.flush()
                .map_err(|error| RemoteMediaTransferError::file(error, file_path))?;
            let response = if is_resumed_transfer {
                Self::whole_response(head, file_path)
            } else {
                head
            };
            if response.expected_content_length() > self.maximum_byte_count {
                return Err(RemoteMediaDiskCacheError::PayloadTooLarge.into());
            }
            let bytes = RemoteMediaDiskCache::file_byte_count(file_path);
            let expected = response.expected_content_length();
            let progress =
                RemoteMediaDownloadProgress::new(bytes, Some(if expected > 0 { expected } else { bytes }));
            let should_publish = self
                .progress_coalescer
                .lock()
                .should_publish(progress, RemoteMediaDownloadProgressCoalescer::uptime());
            if should_publish {
                self.progress_sink.call_as_function(progress);
            }
            return Ok(response);
        }
    }

    /// a transfer cut mid body resumes from the partial file instead of zero
    /// a cut on a live path spends resumes_without_progress
    fn should_resume(
        &self,
        error: &RemoteMediaTransferError,
        bytes_written: i64,
        resumes_without_progress: &mut i64,
        bytes_at_last_resume: &mut i64,
    ) -> bool {
        if bytes_written <= 0 || !RemoteMediaDownloadRetryPolicy::is_transient(error) {
            return false;
        }
        if bytes_written > *bytes_at_last_resume {
            *resumes_without_progress = 0;
        }
        if *resumes_without_progress >= RemoteMediaDownloadRetryPolicy::maximum_retries() {
            return false;
        }
        *resumes_without_progress += 1;
        *bytes_at_last_resume = bytes_written;
        true
    }

    fn whole_response(response: RemoteMediaHttpResponse, file_path: &Path) -> RemoteMediaHttpResponse {
        if response.status_code != 206 {
            return response;
        }
        let mut headers: Vec<(String, String)> = response
            .all_header_fields()
            .iter()
            .filter(|(name, _)| {
                !name.eq_ignore_ascii_case(reqwest::header::CONTENT_RANGE.as_str())
                    && !name.eq_ignore_ascii_case(reqwest::header::CONTENT_LENGTH.as_str())
            })
            .cloned()
            .collect();
        headers.push((
            reqwest::header::CONTENT_LENGTH.as_str().to_owned(),
            RemoteMediaDiskCache::file_byte_count(file_path).to_string(),
        ));
        RemoteMediaHttpResponse::new(response.url, 200, headers)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteMediaDiskCacheLimits {
    pub maximum_entry_count: i64,
    pub maximum_total_bytes: i64,
    pub maximum_file_bytes: i64,
}

impl RemoteMediaDiskCacheLimits {
    pub fn new(maximum_entry_count: i64, maximum_total_bytes: i64, maximum_file_bytes: i64) -> Self {
        let maximum_total_bytes = maximum_total_bytes.max(1);
        Self {
            maximum_entry_count: maximum_entry_count.max(1),
            maximum_total_bytes,
            maximum_file_bytes: maximum_file_bytes.min(maximum_total_bytes).max(1),
        }
    }

    pub fn default_limits() -> Self {
        Self::new(1_200, 4_294_967_296, 1_610_612_736)
    }
}

impl Default for RemoteMediaDiskCacheLimits {
    fn default() -> Self {
        Self::default_limits()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMediaDiskCacheEntry {
    pub key_digest: String,
    pub file_name: String,
    pub byte_count: i64,
    pub created_at: f64,
    pub last_accessed_at: f64,
    pub source_digest: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteMediaDiskCacheStatistics {
    pub entry_count: i64,
    pub total_bytes: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteMediaDiskCacheListedEntry {
    pub id: String,
    pub file_name: String,
    pub byte_count: i64,
    pub content_type: Option<String>,
    pub last_accessed_at: Date,
}

pub type RemoteMediaDiskCacheDownloadResult = anyhow::Result<(PathBuf, RemoteMediaHttpResponse)>;
pub type RemoteMediaDiskCacheDownloader =
    Arc<dyn Fn(URLRequest) -> BoxFuture<'static, RemoteMediaDiskCacheDownloadResult> + Send + Sync>;
pub type RemoteMediaDiskCacheProgressiveDownloader = Arc<
    dyn Fn(URLRequest, i64, RemoteMediaDownloadProgressSink) -> BoxFuture<'static, RemoteMediaDiskCacheDownloadResult>
        + Send
        + Sync,
>;
pub type RemoteMediaDiskCacheProgressHandler = Arc<dyn Fn(RemoteMediaDownloadProgress) + Send + Sync>;

#[derive(Clone, Debug)]
struct RemoteMediaDiskCacheCacheIdentity {
    digest: String,
    source_digest: String,
    uses_explicit_key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RemoteMediaDiskCacheWorkKey {
    account_directory_name: String,
    key_digest: String,
    source_digest: String,
    account_epoch: Option<String>,
    invalidation_token: u64,
}

struct RemoteMediaDiskCacheInFlight {
    id: Uuid,
    invalidation_token: u64,
    task: RemoteMediaSharedTask,
    abort: tokio::task::AbortHandle,
    observers: HashMap<Uuid, RemoteMediaDiskCacheProgressObserver>,
    latest_progress: Option<RemoteMediaDownloadProgress>,
}

#[derive(Clone)]
struct RemoteMediaDiskCacheProgressObserver {
    handler: RemoteMediaDiskCacheProgressHandler,
    expected_account_user_id: Option<String>,
    expected_account_epoch: Option<String>,
}

struct RemoteMediaDiskCacheDownloadedPayload {
    temporary_url: PathBuf,
    response: RemoteMediaHttpResponse,
    byte_count: i64,
    content_type: Option<String>,
}

struct RemoteMediaDiskCacheObserverRegistration {
    cache: Weak<RemoteMediaDiskCache>,
    observer_id: Option<Uuid>,
    work_key: RemoteMediaDiskCacheWorkKey,
}

impl Drop for RemoteMediaDiskCacheObserverRegistration {
    fn drop(&mut self) {
        if let (Some(cache), Some(observer_id)) = (self.cache.upgrade(), self.observer_id) {
            cache.remove_observer(observer_id, &self.work_key);
        }
    }
}

static LEASED_DIGESTS: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static FILE_LOCK: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static SHARED: LazyLock<Arc<RemoteMediaDiskCache>> =
    LazyLock::new(|| Arc::new(RemoteMediaDiskCache::new(RemoteMediaDiskCacheConfiguration::default())));
static CACHE_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    AppPaths::resolve(AppEnvironment::current().canon())
        .ok()
        .map(|paths| paths.cache_dir().to_path_buf())
});

/// arguments of the disk cache init, with defaults
#[derive(Clone, Default)]
pub struct RemoteMediaDiskCacheConfiguration {
    pub root_directory: Option<PathBuf>,
    pub limits: RemoteMediaDiskCacheLimits,
    pub downloader: Option<RemoteMediaDiskCacheDownloader>,
    pub progressive_downloader: Option<RemoteMediaDiskCacheProgressiveDownloader>,
}

/// options for materialize and ensure cached
/// account defaults to the active one, read when the options are made
#[derive(Clone)]
pub struct RemoteMediaDiskCacheMaterializeOptions {
    pub cache_key: Option<String>,
    pub suggested_filename: Option<String>,
    pub maximum_byte_count: Option<i64>,
    pub authorization_token: Option<String>,
    pub expected_account_user_id: Option<String>,
    pub expected_account_epoch: Option<String>,
    pub on_progress: Option<RemoteMediaDiskCacheProgressHandler>,
}

impl Default for RemoteMediaDiskCacheMaterializeOptions {
    fn default() -> Self {
        Self {
            cache_key: None,
            suggested_filename: None,
            maximum_byte_count: None,
            authorization_token: None,
            expected_account_user_id: SaiAccountCacheScope::active_user_id(),
            expected_account_epoch: SaiAccountCacheScope::active_epoch(),
            on_progress: None,
        }
    }
}

pub struct RemoteMediaDiskCache {
    root_directory_override: Option<PathBuf>,
    limits: RemoteMediaDiskCacheLimits,
    downloader: RemoteMediaDiskCacheProgressiveDownloader,
    in_flight: Mutex<HashMap<RemoteMediaDiskCacheWorkKey, RemoteMediaDiskCacheInFlight>>,
}

impl RemoteMediaDiskCache {
    const SUBFOLDER: &'static str = "RemoteMediaCache";
    const ACCOUNTS_FOLDER: &'static str = "accounts";
    const FILES_FOLDER: &'static str = "files";
    const INDEX_FILE_NAME: &'static str = "_media_index.json";
    const PARTIALS_FOLDER: &'static str = "partials";
    const ANONYMOUS_ACCOUNT: &'static str = "anonymous";
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
    const ACCESS_TOKEN_LEEWAY_SECONDS: f64 = 30.0;
    const PAYLOAD_SNIFF_BYTES: u64 = 512;

    pub fn shared() -> Arc<RemoteMediaDiskCache> {
        Arc::clone(&SHARED)
    }

    pub fn new(configuration: RemoteMediaDiskCacheConfiguration) -> Self {
        let RemoteMediaDiskCacheConfiguration {
            root_directory,
            limits,
            downloader,
            progressive_downloader,
        } = configuration;
        let downloader: RemoteMediaDiskCacheProgressiveDownloader =
            match (progressive_downloader, downloader) {
                (Some(progressive_downloader), _) => progressive_downloader,
                (None, Some(downloader)) => Arc::new(
                    move |request: URLRequest, _: i64, _: RemoteMediaDownloadProgressSink| downloader(request),
                ),
                (None, None) => Arc::new(
                    |request: URLRequest, maximum_byte_count: i64, on_progress: RemoteMediaDownloadProgressSink| {
                    async move {
                        RemoteMediaDownloadOperation::new(maximum_byte_count, on_progress)
                            .start(request)
                            .await
                            .map_err(RemoteMediaTransferError::into_anyhow)
                    }
                    .boxed()
                    },
                ),
            };
        Self {
            root_directory_override: root_directory,
            limits,
            downloader,
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    /// leases the cached file a path points at, none for a path this cache didnt make
    pub fn lease(url: Option<&Path>) -> Option<RemoteMediaFileLease> {
        let name = url?.file_name()?.to_str()?;
        let digest = Self::digest_from_file_name(name)?;
        Some(RemoteMediaFileLease::new(digest))
    }

    fn retain_lease(digest: &str) {
        *LEASED_DIGESTS.lock().entry(digest.to_owned()).or_insert(0) += 1;
    }

    fn release_lease(digest: &str) {
        let mut leased = LEASED_DIGESTS.lock();
        if let Some(count) = leased.get(digest).copied() {
            if count <= 1 {
                leased.remove(digest);
            } else {
                leased.insert(digest.to_owned(), count - 1);
            }
        }
    }

    fn is_leased(digest: &str) -> bool {
        LEASED_DIGESTS.lock().contains_key(digest)
    }

    /// index id of an explicit cache key without opening the manifest
    /// same string list_entries reports as id
    pub fn key_digest(cache_key: &str) -> String {
        Self::digest_hex(&format!("key:{}", trimming_whitespaces_and_newlines(cache_key)))
    }

    pub fn cancel_transfer(
        &self,
        remote: &Url,
        cache_key: Option<&str>,
        expected_account_user_id: Option<&str>,
        expected_account_epoch: Option<&str>,
    ) {
        let identity = Self::cache_identity(remote, cache_key);
        let account_name = Self::account_directory_name(expected_account_user_id);
        let work_key = RemoteMediaDiskCacheWorkKey {
            invalidation_token: Self::invalidation_token(&account_name),
            account_directory_name: account_name,
            key_digest: identity.digest,
            source_digest: identity.source_digest,
            account_epoch: expected_account_epoch.map(str::to_owned),
        };
        if let Some(entry) = self.in_flight.lock().remove(&work_key) {
            entry.abort.abort();
        }
    }

    /// blocking file io, run it off the main thread
    pub fn existing_local_url(
        &self,
        remote: &Url,
        cache_key: Option<&str>,
        expected_account_user_id: Option<&str>,
        expected_account_epoch: Option<&str>,
    ) -> Option<PathBuf> {
        if !Self::is_http(remote)
            || !SaiAccountCacheScope::is_current_account_epoch(expected_account_user_id, expected_account_epoch)
        {
            return None;
        }
        let identity = Self::cache_identity(remote, cache_key);
        Self::with_file_lock(|_| {
            let root = self.resolved_root_directory().ok()?;
            let directory = Self::account_directory(&root, expected_account_user_id).ok()?;
            let files = Self::files_directory(&directory).ok()?;
            let mut index = Self::reconciled_index(&directory, &files).ok()?;
            let mut entry = index.get(&identity.digest).cloned()?;
            if !Self::is_valid(&entry, &files) {
                return None;
            }
            if !Self::source_matches(&entry, &identity) {
                FileManager.remove_item(&files.join(&entry.file_name)).ok();
                index.remove(&identity.digest);
                Self::write_index(&index, &directory).ok();
                return None;
            }
            entry.last_accessed_at = Date::now().time_interval_since_1970();
            index.insert(identity.digest.clone(), entry.clone());
            index = Self::trimmed_index(index.clone(), &files, &self.limits).unwrap_or(index);
            Self::write_index(&index, &directory).ok();
            index.get(&identity.digest)?;
            Some(files.join(&entry.file_name))
        })
    }

    pub async fn ensure_cached(
        self: &Arc<Self>,
        remote: &Url,
        options: RemoteMediaDiskCacheMaterializeOptions,
        cx: &AsyncApp,
    ) -> Option<PathBuf> {
        self.materialize(remote, options, cx).await.ok()
    }

    /// gets the file locally, downloading if needed
    /// bearer is resolved here on main before the transfer starts on the account runtime
    pub async fn materialize(
        self: &Arc<Self>,
        remote: &Url,
        options: RemoteMediaDiskCacheMaterializeOptions,
        cx: &AsyncApp,
    ) -> anyhow::Result<PathBuf> {
        self.materialize_transfer(remote.clone(), options, cx)
            .await
            .map_err(RemoteMediaTransferError::into_anyhow)
    }

    async fn materialize_transfer(
        self: &Arc<Self>,
        remote: Url,
        options: RemoteMediaDiskCacheMaterializeOptions,
        cx: &AsyncApp,
    ) -> Result<PathBuf, RemoteMediaTransferError> {
        let RemoteMediaDiskCacheMaterializeOptions {
            cache_key,
            suggested_filename,
            maximum_byte_count,
            authorization_token,
            expected_account_user_id,
            expected_account_epoch,
            on_progress,
        } = options;
        if !Self::is_http(&remote) {
            return Err(RemoteMediaDiskCacheError::InvalidSource.into());
        }
        if !SaiAccountCacheScope::is_current_account_epoch(
            expected_account_user_id.as_deref(),
            expected_account_epoch.as_deref(),
        ) {
            return Err(RemoteMediaDiskCacheError::AccountChanged.into());
        }
        let existing = {
            let this = Arc::clone(self);
            let remote = remote.clone();
            let cache_key = cache_key.clone();
            let user_id = expected_account_user_id.clone();
            let epoch = expected_account_epoch.clone();
            RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    this.existing_local_url(&remote, cache_key.as_deref(), user_id.as_deref(), epoch.as_deref())
                })
                .await
                .map_err(|_| RemoteMediaTransferError::from(CancellationError))?
        };
        if let Some(existing) = existing {
            return Ok(existing);
        }

        let identity = Self::cache_identity(&remote, cache_key.as_deref());
        let account_name = Self::account_directory_name(expected_account_user_id.as_deref());
        let invalidation_token = Self::invalidation_token(&account_name);
        let work_key = RemoteMediaDiskCacheWorkKey {
            account_directory_name: account_name.clone(),
            key_digest: identity.digest.clone(),
            source_digest: identity.source_digest.clone(),
            account_epoch: expected_account_epoch.clone(),
            invalidation_token,
        };
        if let Some((task, active_id)) = self.active_task(&work_key) {
            let observer_id = self.register_observer(
                on_progress,
                &work_key,
                active_id,
                expected_account_user_id,
                expected_account_epoch,
            );
            return Self::settle(self.await_shared_task(task, work_key, observer_id).await, false, cx).await;
        }

        let bearer = Self::bearer_token(&remote, authorization_token.as_deref(), cx).await;
        if let Some((task, active_id)) = self.active_task(&work_key) {
            let observer_id = self.register_observer(
                on_progress,
                &work_key,
                active_id,
                expected_account_user_id,
                expected_account_epoch,
            );
            return Self::settle(self.await_shared_task(task, work_key, observer_id).await, false, cx).await;
        }

        let id = Uuid::new_v4();
        let byte_limit = maximum_byte_count
            .unwrap_or(self.limits.maximum_file_bytes)
            .min(self.limits.maximum_file_bytes)
            .max(1);
        let weak = Arc::downgrade(self);
        let progress_target = Weak::clone(&weak);
        let progress_key = work_key.clone();
        let task_key = work_key.clone();
        let user_id = expected_account_user_id.clone();
        let epoch = expected_account_epoch.clone();
        let handle = RiseAccountRuntime::current().spawn(async move {
            let Some(this) = weak.upgrade() else {
                return Err(RemoteMediaTransferError::from(RemoteMediaDiskCacheError::CacheUnavailable));
            };
            let result = this
                .download_and_commit(
                    remote,
                    identity,
                    suggested_filename,
                    byte_limit,
                    bearer,
                    user_id,
                    epoch,
                    account_name,
                    invalidation_token,
                    RemoteMediaDownloadProgressSink::new(move |progress| {
                        if let Some(cache) = progress_target.upgrade() {
                            cache.publish_progress(progress, &progress_key, id);
                        }
                    }),
                )
                .await;
            this.finish_in_flight(&task_key, id);
            result
        });
        let abort = handle.abort_handle();
        let task: RemoteMediaSharedTask = async move {
            match handle.await {
                Ok(result) => result,
                Err(_) => Err(RemoteMediaTransferError::from(CancellationError)),
            }
        }
        .boxed()
        .shared();
        self.in_flight.lock().insert(
            work_key.clone(),
            RemoteMediaDiskCacheInFlight {
                id,
                invalidation_token,
                task: task.clone(),
                abort,
                observers: HashMap::new(),
                latest_progress: None,
            },
        );
        let observer_id = self.register_observer(
            on_progress,
            &work_key,
            id,
            expected_account_user_id,
            expected_account_epoch,
        );
        Self::settle(self.await_shared_task(task, work_key, observer_id).await, true, cx).await
    }

    fn active_task(&self, work_key: &RemoteMediaDiskCacheWorkKey) -> Option<(RemoteMediaSharedTask, Uuid)> {
        self.in_flight
            .lock()
            .get(work_key)
            .map(|active| (active.task.clone(), active.id))
    }

    /// only the caller that started the transfer publishes a refusal so its shown once
    async fn settle(
        result: Result<PathBuf, RemoteMediaTransferError>,
        publishes_refusal: bool,
        cx: &AsyncApp,
    ) -> Result<PathBuf, RemoteMediaTransferError> {
        match result {
            Err(RemoteMediaTransferError::ProtectedRefusal(report)) => {
                let status_code = report.response.status_code;
                if publishes_refusal {
                    let mut cx = cx.clone();
                    report.as_ref().clone().publish(&mut cx).await;
                }
                Err(RemoteMediaDiskCacheError::InvalidResponse {
                    status_code: Some(status_code),
                }
                .into())
            }
            other => other,
        }
    }

    fn register_observer(
        &self,
        observer: Option<RemoteMediaDiskCacheProgressHandler>,
        work_key: &RemoteMediaDiskCacheWorkKey,
        in_flight_id: Uuid,
        expected_account_user_id: Option<String>,
        expected_account_epoch: Option<String>,
    ) -> Option<Uuid> {
        let observer = observer?;
        let (progress_observer, latest, observer_id) = {
            let mut in_flight = self.in_flight.lock();
            let active = in_flight.get_mut(work_key)?;
            if active.id != in_flight_id {
                return None;
            }
            let observer_id = Uuid::new_v4();
            let progress_observer = RemoteMediaDiskCacheProgressObserver {
                handler: observer,
                expected_account_user_id,
                expected_account_epoch,
            };
            active.observers.insert(observer_id, progress_observer.clone());
            (progress_observer, active.latest_progress, observer_id)
        };
        if let Some(latest) = latest {
            Self::deliver(latest, &progress_observer);
        }
        Some(observer_id)
    }

    async fn await_shared_task(
        self: &Arc<Self>,
        task: RemoteMediaSharedTask,
        work_key: RemoteMediaDiskCacheWorkKey,
        observer_id: Option<Uuid>,
    ) -> Result<PathBuf, RemoteMediaTransferError> {
        let _registration = RemoteMediaDiskCacheObserverRegistration {
            cache: Arc::downgrade(self),
            observer_id,
            work_key,
        };
        RemoteMediaSharedTaskWaiter::wait(task).await
    }

    fn finish_in_flight(&self, work_key: &RemoteMediaDiskCacheWorkKey, in_flight_id: Uuid) {
        let mut in_flight = self.in_flight.lock();
        if in_flight.get(work_key).is_some_and(|active| active.id == in_flight_id) {
            in_flight.remove(work_key);
        }
    }

    fn remove_observer(&self, observer_id: Uuid, work_key: &RemoteMediaDiskCacheWorkKey) {
        if let Some(active) = self.in_flight.lock().get_mut(work_key) {
            active.observers.remove(&observer_id);
        }
    }

    fn publish_progress(
        &self,
        progress: RemoteMediaDownloadProgress,
        work_key: &RemoteMediaDiskCacheWorkKey,
        in_flight_id: Uuid,
    ) {
        let observers: Vec<RemoteMediaDiskCacheProgressObserver> = {
            let mut in_flight = self.in_flight.lock();
            let Some(active) = in_flight.get_mut(work_key) else {
                return;
            };
            if active.id != in_flight_id {
                return;
            }
            active.latest_progress = Some(progress);
            active.observers.values().cloned().collect()
        };
        for observer in &observers {
            Self::deliver(progress, observer);
        }
    }

    fn deliver(progress: RemoteMediaDownloadProgress, observer: &RemoteMediaDiskCacheProgressObserver) {
        if !SaiAccountCacheScope::is_current_account_epoch(
            observer.expected_account_user_id.as_deref(),
            observer.expected_account_epoch.as_deref(),
        ) {
            return;
        }
        (observer.handler)(progress);
    }

    pub fn statistics(
        &self,
        expected_account_user_id: Option<&str>,
        expected_account_epoch: Option<&str>,
    ) -> RemoteMediaDiskCacheStatistics {
        let empty = RemoteMediaDiskCacheStatistics {
            entry_count: 0,
            total_bytes: 0,
        };
        if !SaiAccountCacheScope::is_current_account_epoch(expected_account_user_id, expected_account_epoch) {
            return empty;
        }
        Self::with_file_lock(|_| {
            let Ok(root) = self.resolved_root_directory() else {
                return empty;
            };
            let Ok(account) = Self::account_directory(&root, expected_account_user_id) else {
                return empty;
            };
            let Ok(files) = Self::files_directory(&account) else {
                return empty;
            };
            let Ok(mut index) = Self::reconciled_index(&account, &files) else {
                return empty;
            };
            index = Self::trimmed_index(index.clone(), &files, &self.limits).unwrap_or(index);
            Self::write_index(&index, &account).ok();
            RemoteMediaDiskCacheStatistics {
                entry_count: i64::try_from(index.len()).unwrap_or(i64::MAX),
                total_bytes: index.values().map(|entry| entry.byte_count.max(0)).sum(),
            }
        })
    }

    pub fn folder_url_for_inspector(
        &self,
        expected_account_user_id: Option<&str>,
        expected_account_epoch: Option<&str>,
    ) -> Option<PathBuf> {
        if !SaiAccountCacheScope::is_current_account_epoch(expected_account_user_id, expected_account_epoch) {
            return None;
        }
        Self::with_file_lock(|_| {
            let root = self.resolved_root_directory().ok()?;
            Self::account_directory(&root, expected_account_user_id).ok()
        })
    }

    pub fn reset_account_state() {
        let account_name = Self::account_directory_name(SaiAccountCacheScope::active_user_id().as_deref());
        let token = Self::invalidate(&account_name);
        Self::shared().cancel_in_flight(&account_name, token);
    }

    pub fn clear_all(user_id: Option<&str>) {
        Self::clear_all_respecting_leases(user_id, true);
    }

    /// clears the cache, respecting_leases false is account removal and nothing may keep a file
    /// the user purge keeps leased files for the next pass
    fn clear_all_respecting_leases(user_id: Option<&str>, respecting_leases: bool) {
        let account_name = Self::account_directory_name(user_id);
        let token = Self::invalidate(&account_name);
        Self::with_file_lock(|_| {
            let Ok(root) = Self::production_root_directory(false) else {
                return;
            };
            let directory = root.join(Self::ACCOUNTS_FOLDER).join(&account_name);
            if !respecting_leases {
                FileManager.remove_item(&directory).ok();
                return;
            }
            FileManager.remove_item(&directory.join(Self::PARTIALS_FOLDER)).ok();
            let files = directory.join(Self::FILES_FOLDER);
            let names = FileManager.contents_of_directory_at_path(&files).unwrap_or_default();
            let mut survivors: HashSet<String> = HashSet::new();
            for name in names {
                if let Some(digest) = Self::digest_from_file_name(&name)
                    && Self::is_leased(&digest)
                {
                    survivors.insert(digest);
                    continue;
                }
                FileManager.remove_item(&files.join(&name)).ok();
            }
            if survivors.is_empty() {
                FileManager.remove_item(&directory).ok();
                return;
            }
            let index = Self::reconciled_index(&directory, &files).unwrap_or_default();
            let kept: HashMap<String, RemoteMediaDiskCacheEntry> = index
                .into_iter()
                .filter(|(digest, _)| survivors.contains(digest))
                .collect();
            Self::write_index(&kept, &directory).ok();
        });
        Self::shared().cancel_in_flight(&account_name, token);
    }

    pub fn remove_persisted_account(user_id: &str) {
        Self::clear_all_respecting_leases(Some(user_id), false);
    }

    /// removes the listed entries and their files, for one chats trash
    /// a leased file stays with its entry
    pub fn remove(key_digests: &HashSet<String>, user_id: Option<&str>) {
        if key_digests.is_empty() {
            return;
        }
        Self::with_file_lock(|_| {
            let Ok(root) = Self::production_root_directory(false) else {
                return;
            };
            let Ok(account) = Self::account_directory(&root, user_id) else {
                return;
            };
            let Ok(files) = Self::files_directory(&account) else {
                return;
            };
            let Some(mut index) = Self::read_index(&account) else {
                return;
            };
            let mut changed = false;
            for digest in key_digests {
                if Self::is_leased(digest) {
                    continue;
                }
                let Some(entry) = index.remove(digest) else {
                    continue;
                };
                FileManager.remove_item(&files.join(&entry.file_name)).ok();
                changed = true;
            }
            if changed {
                Self::write_index(&index, &account).ok();
            }
        });
    }

    pub fn folder_path_for_inspector(user_id: Option<&str>) -> String {
        Self::with_file_lock(|_| {
            Self::production_root_directory(true)
                .and_then(|root| Self::account_directory(&root, user_id))
                .map(|directory| directory.display().to_string())
                .unwrap_or_else(|_| "(unavailable)".to_owned())
        })
    }

    pub fn storage_directory_url(user_id: Option<&str>) -> Option<PathBuf> {
        Self::with_file_lock(|_| {
            let root = Self::production_root_directory(false).ok()?;
            Self::account_directory(&root, user_id).ok()
        })
    }

    /// read only view of the index
    /// statistics reconciles first and that deletes unmatched files, a size report must not do that
    pub fn list_entries(user_id: Option<&str>) -> Vec<RemoteMediaDiskCacheListedEntry> {
        Self::with_file_lock(|_| {
            let Ok(root) = Self::production_root_directory(false) else {
                return Vec::new();
            };
            let Ok(account) = Self::account_directory(&root, user_id) else {
                return Vec::new();
            };
            let Some(index) = Self::read_index(&account) else {
                return Vec::new();
            };
            let mut entries: Vec<RemoteMediaDiskCacheListedEntry> = index
                .into_values()
                .map(|entry| RemoteMediaDiskCacheListedEntry {
                    id: entry.key_digest,
                    file_name: entry.file_name,
                    byte_count: entry.byte_count.max(0),
                    content_type: entry.content_type,
                    last_accessed_at: Date::from_time_interval_since_1970(entry.last_accessed_at),
                })
                .collect();
            entries.sort_by(|lhs, rhs| {
                rhs.last_accessed_at
                    .time_interval_since_1970()
                    .total_cmp(&lhs.last_accessed_at.time_interval_since_1970())
            });
            entries
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_and_commit(
        &self,
        remote: Url,
        identity: RemoteMediaDiskCacheCacheIdentity,
        suggested_filename: Option<String>,
        maximum_byte_count: i64,
        bearer: Option<String>,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        account_directory_name: String,
        invalidation_token: u64,
        on_progress: RemoteMediaDownloadProgressSink,
    ) -> Result<PathBuf, RemoteMediaTransferError> {
        Self::validate_account(
            account_user_id.as_deref(),
            account_epoch.as_deref(),
            &account_directory_name,
            invalidation_token,
        )?;
        let mut request = URLRequest::new(remote.clone());
        request.timeout_interval = Self::REQUEST_TIMEOUT;
        request.http_method = reqwest::Method::GET.as_str().to_owned();
        request.set_value("*/*", reqwest::header::ACCEPT.as_str());
        request.set_value("no-store", reqwest::header::CACHE_CONTROL.as_str());
        if let Some(bearer) = bearer {
            request.set_value(&format!("Bearer {bearer}"), reqwest::header::AUTHORIZATION.as_str());
        }

        let payload = self
            .download_payload(
                request,
                maximum_byte_count,
                suggested_filename.as_deref(),
                &remote,
                account_user_id.as_deref(),
                account_epoch.as_deref(),
                &on_progress,
            )
            .await?;
        let temporary = RemoteMediaTemporaryFile::new(payload.temporary_url.clone());
        Self::validate_account(
            account_user_id.as_deref(),
            account_epoch.as_deref(),
            &account_directory_name,
            invalidation_token,
        )?;
        let result = self.commit(
            &temporary.path,
            payload.byte_count,
            payload.content_type,
            payload.response.suggested_filename(),
            &remote,
            &identity,
            suggested_filename.as_deref(),
            account_user_id.as_deref(),
            account_epoch.as_deref(),
            &account_directory_name,
            invalidation_token,
        );
        drop(temporary);
        result
    }

    /// commit half of a download, moves bytes into the account dir
    /// writes the index entry and trims to the budget
    /// split out because streaming builds its own file from range requests
    #[allow(clippy::too_many_arguments)]
    fn commit(
        &self,
        temporary_url: &Path,
        byte_count: i64,
        content_type: Option<String>,
        response_filename: Option<String>,
        remote: &Url,
        identity: &RemoteMediaDiskCacheCacheIdentity,
        suggested_filename: Option<&str>,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        account_directory_name: &str,
        invalidation_token: u64,
    ) -> Result<PathBuf, RemoteMediaTransferError> {
        let file_extension = Self::preferred_extension(
            suggested_filename,
            response_filename.as_deref(),
            remote,
            content_type.as_deref(),
        );
        Self::with_file_lock(|tokens| {
            Self::validate_account_locked(
                tokens,
                account_user_id,
                account_epoch,
                account_directory_name,
                invalidation_token,
            )?;
            let root = self.resolved_root_directory()?;
            let account = Self::account_directory(&root, account_user_id)?;
            let files = Self::files_directory(&account)?;
            let mut index = Self::reconciled_index(&account, &files)?;
            if let Some(mut existing) = index.get(&identity.digest).cloned()
                && Self::is_valid(&existing, &files)
            {
                if Self::source_matches(&existing, identity) {
                    existing.last_accessed_at = Date::now().time_interval_since_1970();
                    let file_name = existing.file_name.clone();
                    index.insert(identity.digest.clone(), existing);
                    Self::write_index(&index, &account)?;
                    return Ok(files.join(file_name));
                }
                FileManager.remove_item(&files.join(&existing.file_name)).ok();
                index.remove(&identity.digest);
            }

            let filename = format!("{}.{}", identity.digest, file_extension);
            let destination = files.join(&filename);
            let staging = files.join(format!(".incoming-{}", Uuid::new_v4().uuid_string().to_lowercase()));
            let staged = FileManager
                .move_item(temporary_url, &staging)
                .or_else(|_| FileManager.copy_item(temporary_url, &staging))
                .and_then(|()| {
                    Self::apply_storage_attributes(&staging);
                    std::fs::rename(&staging, &destination).map_err(|error| CocoaError {
                        code: CocoaErrorCode::FileWriteUnknown,
                        path: Some(destination.clone()),
                        underlying: Some(Arc::new(error)),
                    })
                });
            if let Err(error) = staged {
                FileManager.remove_item(&staging).ok();
                return Err(error.into());
            }
            Self::apply_storage_attributes(&destination);

            let now = Date::now().time_interval_since_1970();
            index.insert(
                identity.digest.clone(),
                RemoteMediaDiskCacheEntry {
                    key_digest: identity.digest.clone(),
                    file_name: filename,
                    byte_count,
                    created_at: now,
                    last_accessed_at: now,
                    source_digest: Some(identity.source_digest.clone()),
                    content_type,
                },
            );
            let index = Self::trimmed_index(index, &files, &self.limits)?;
            Self::write_index(&index, &account)?;
            if !index.contains_key(&identity.digest) || !FileManager.file_exists(&destination) {
                return Err(RemoteMediaDiskCacheError::CacheUnavailable.into());
            }
            Ok(destination)
        })
    }

    /// takes a file another transport produced and files it under this cache identity
    /// streaming loader uses it so the player video is the same object the bubble ring filled
    /// blocking file io, keep it off the main thread
    #[allow(clippy::too_many_arguments)]
    pub fn adopt(
        &self,
        temporary_url: &Path,
        remote: &Url,
        cache_key: Option<&str>,
        suggested_filename: Option<&str>,
        content_type: Option<String>,
        expected_account_user_id: Option<&str>,
        expected_account_epoch: Option<&str>,
    ) -> Result<PathBuf, RemoteMediaTransferError> {
        if !Self::is_http(remote) {
            return Err(RemoteMediaDiskCacheError::InvalidSource.into());
        }
        let byte_count = Self::file_byte_count(temporary_url);
        if byte_count <= 0 {
            return Err(RemoteMediaDiskCacheError::EmptyResponse.into());
        }
        if byte_count > self.limits.maximum_file_bytes {
            return Err(RemoteMediaDiskCacheError::PayloadTooLarge.into());
        }
        let identity = Self::cache_identity(remote, cache_key);
        let account_name = Self::account_directory_name(expected_account_user_id);
        self.commit(
            temporary_url,
            byte_count,
            content_type,
            None,
            remote,
            &identity,
            suggested_filename,
            expected_account_user_id,
            expected_account_epoch,
            &account_name,
            Self::invalidation_token(&account_name),
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_payload(
        &self,
        request: URLRequest,
        maximum_byte_count: i64,
        suggested_filename: Option<&str>,
        remote: &Url,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        on_progress: &RemoteMediaDownloadProgressSink,
    ) -> Result<RemoteMediaDiskCacheDownloadedPayload, RemoteMediaTransferError> {
        let mut attempt = 0;
        let (temporary_url, response) = loop {
            if !SaiAccountCacheScope::is_current_account_epoch(account_user_id, account_epoch) {
                return Err(RemoteMediaDiskCacheError::AccountChanged.into());
            }
            let downloaded = (self.downloader)(request.clone(), maximum_byte_count, on_progress.clone())
                .await
                .map_err(RemoteMediaTransferError::from_anyhow);
            match downloaded {
                Ok(downloaded) => break downloaded,
                Err(error) => {
                    if !RemoteMediaDownloadRetryPolicy::should_retry(&error, attempt) {
                        return Err(error);
                    }
                    attempt += 1;
                    on_progress.call_as_function(RemoteMediaDownloadProgress::new(0, None));
                    RiseRuntimeTimer::sleep(RemoteMediaDownloadRetryPolicy::delay(attempt)).await;
                }
            }
        };
        let temporary = RemoteMediaTemporaryFile::new(temporary_url);
        let maximum_body_bytes = i64::try_from(RiseMediaRefusal::MAXIMUM_BODY_BYTES).unwrap_or(i64::MAX);
        if RiseMediaRefusal::is_protected_refusal(Some(response.status_code), Some(&response.url), &request)
            && Self::file_byte_count(&temporary.path) <= maximum_body_bytes
            && let Ok(body) = std::fs::read(&temporary.path)
        {
            return Err(RemoteMediaTransferError::ProtectedRefusal(Arc::new(RemoteMediaRefusalReport {
                body,
                response,
                request,
                account_user_id: account_user_id.map(str::to_owned),
                account_epoch: account_epoch.map(str::to_owned),
            })));
        }
        let status_code = response.status_code;
        if !(200..300).contains(&status_code) || matches!(status_code, 204..=206) {
            return Err(RemoteMediaDiskCacheError::InvalidResponse {
                status_code: Some(status_code),
            }
            .into());
        }
        if response.expected_content_length() > maximum_byte_count {
            return Err(RemoteMediaDiskCacheError::PayloadTooLarge.into());
        }
        let byte_count = Self::file_byte_count(&temporary.path);
        if byte_count <= 0 {
            return Err(RemoteMediaDiskCacheError::EmptyResponse.into());
        }
        if byte_count > maximum_byte_count {
            return Err(RemoteMediaDiskCacheError::PayloadTooLarge.into());
        }
        let content_type = response
            .value(reqwest::header::CONTENT_TYPE.as_str())
            .map(str::to_lowercase);
        let response_filename = response.suggested_filename();
        let remote_last = remote.last_path_component();
        let allows_json = [suggested_filename, response_filename.as_deref(), Some(remote_last.as_str())]
            .into_iter()
            .flatten()
            .any(|name| Self::path_extension(name).eq_ignore_ascii_case("json"));
        if !Self::is_supported_payload(&temporary.path, content_type.as_deref(), allows_json) {
            return Err(RemoteMediaDiskCacheError::UnsupportedPayload.into());
        }
        Ok(RemoteMediaDiskCacheDownloadedPayload {
            temporary_url: temporary.hand_over(),
            response,
            byte_count,
            content_type,
        })
    }

    fn cancel_in_flight(&self, account_directory_name: &str, invalidated_before: u64) {
        self.in_flight.lock().retain(|key, value| {
            let cancels =
                key.account_directory_name == account_directory_name && value.invalidation_token != invalidated_before;
            if cancels {
                value.abort.abort();
            }
            !cancels
        });
    }

    fn resolved_root_directory(&self) -> Result<PathBuf, CocoaError> {
        if let Some(root) = &self.root_directory_override {
            FileManager.create_directory(root, true)?;
            Self::apply_storage_attributes(root);
            return Ok(root.clone());
        }
        Self::production_root_directory(true)
    }

    fn production_root_directory(create: bool) -> Result<PathBuf, CocoaError> {
        let Some(cache) = CACHE_ROOT.as_ref() else {
            return Err(CocoaError::new(CocoaErrorCode::FileNoSuchFile));
        };
        let root = cache.join(Self::SUBFOLDER);
        if create {
            FileManager.create_directory(&root, true)?;
            Self::apply_storage_attributes(&root);
        }
        Ok(root)
    }

    fn account_directory(root: &Path, user_id: Option<&str>) -> Result<PathBuf, CocoaError> {
        let accounts = root.join(Self::ACCOUNTS_FOLDER);
        FileManager.create_directory(&accounts, true)?;
        Self::apply_storage_attributes(&accounts);
        let directory = accounts.join(Self::account_directory_name(user_id));
        FileManager.create_directory(&directory, true)?;
        Self::apply_storage_attributes(&directory);
        Ok(directory)
    }

    pub fn partials_directory(user_id: Option<&str>) -> Result<PathBuf, CocoaError> {
        let root = Self::production_root_directory(true)?;
        let account = Self::account_directory(&root, user_id)?;
        let directory = account.join(Self::PARTIALS_FOLDER);
        FileManager.create_directory(&directory, true)?;
        Self::apply_storage_attributes(&directory);
        Ok(directory)
    }

    pub fn remove_partials(user_id: Option<&str>) {
        let Ok(root) = Self::production_root_directory(false) else {
            return;
        };
        let directory = root
            .join(Self::ACCOUNTS_FOLDER)
            .join(Self::account_directory_name(user_id))
            .join(Self::PARTIALS_FOLDER);
        FileManager.remove_item(&directory).ok();
    }

    fn files_directory(account: &Path) -> Result<PathBuf, CocoaError> {
        let directory = account.join(Self::FILES_FOLDER);
        FileManager.create_directory(&directory, true)?;
        Self::apply_storage_attributes(&directory);
        Ok(directory)
    }

    fn index_url(account: &Path) -> PathBuf {
        account.join(Self::INDEX_FILE_NAME)
    }

    fn read_index(account: &Path) -> Option<HashMap<String, RemoteMediaDiskCacheEntry>> {
        let data = std::fs::read(Self::index_url(account)).ok()?;
        serde_json::from_slice(&data).ok()
    }

    fn reconciled_index(
        account: &Path,
        files: &Path,
    ) -> Result<HashMap<String, RemoteMediaDiskCacheEntry>, CocoaError> {
        let index_file = Self::index_url(account);
        let mut changed = false;
        let mut index: HashMap<String, RemoteMediaDiskCacheEntry> = if FileManager.file_exists(&index_file) {
            match std::fs::read(&index_file)
                .ok()
                .and_then(|data| serde_json::from_slice(&data).ok())
            {
                Some(index) => index,
                None => {
                    FileManager.remove_item(&index_file).ok();
                    changed = true;
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        let disk_files = FileManager.contents_of_directory_at(files, DirectoryEnumerationOptions::default())?;
        let mut files_by_name: HashMap<String, PathBuf> = HashMap::new();
        for url in disk_files {
            let name = url
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Some(digest) = Self::digest_from_file_name(&name) else {
                FileManager.remove_item(&url).ok();
                changed = true;
                continue;
            };
            let is_sized_regular_file = FileManager
                .attributes_of_item(&url)
                .is_ok_and(|values| values.r#type == FileAttributeType::TypeRegular && values.size > 0);
            if !is_sized_regular_file {
                FileManager.remove_item(&url).ok();
                index.remove(&digest);
                changed = true;
                continue;
            }
            files_by_name.insert(name, url);
        }

        let digests: Vec<String> = index.keys().cloned().collect();
        for digest in digests {
            let Some(entry) = index.get(&digest).cloned() else {
                continue;
            };
            let url = files_by_name.get(&entry.file_name).cloned();
            let matches = entry.key_digest == digest
                && Self::safe_filename(&entry.file_name)
                && url.is_some()
                && Self::digest_from_file_name(&entry.file_name).as_deref() == Some(digest.as_str());
            let Some(url) = url.filter(|_| matches) else {
                index.remove(&digest);
                changed = true;
                continue;
            };
            let actual = Self::file_byte_count(&url);
            if actual <= 0 || actual != entry.byte_count {
                FileManager.remove_item(&url).ok();
                files_by_name.remove(&entry.file_name);
                index.remove(&digest);
                changed = true;
            }
        }

        let indexed_names: HashSet<String> = index.values().map(|entry| entry.file_name.clone()).collect();
        for (name, url) in files_by_name {
            if indexed_names.contains(&name) {
                continue;
            }
            let Some(digest) = Self::digest_from_file_name(&name) else {
                continue;
            };
            if index.contains_key(&digest) {
                FileManager.remove_item(&url).ok();
                changed = true;
                continue;
            }
            let values = FileManager.attributes_of_item(&url).ok();
            let byte_count = values.as_ref().map_or(0, |values| values.size);
            if byte_count <= 0 {
                FileManager.remove_item(&url).ok();
                changed = true;
                continue;
            }
            let modified = values
                .and_then(|values| values.modification_date)
                .and_then(|date| date.duration_since(UNIX_EPOCH).ok())
                .map_or_else(
                    || Date::distant_past().time_interval_since_1970(),
                    |since| since.as_secs_f64(),
                );
            index.insert(
                digest.clone(),
                RemoteMediaDiskCacheEntry {
                    key_digest: digest,
                    file_name: name,
                    byte_count,
                    created_at: modified,
                    last_accessed_at: modified,
                    source_digest: None,
                    content_type: None,
                },
            );
            changed = true;
        }

        if changed {
            Self::write_index(&index, account)?;
        }
        Ok(index)
    }

    fn write_index(index: &HashMap<String, RemoteMediaDiskCacheEntry>, account: &Path) -> Result<(), CocoaError> {
        let url = Self::index_url(account);
        let write_error = |error: std::io::Error| CocoaError {
            code: CocoaErrorCode::FileWriteUnknown,
            path: Some(url.clone()),
            underlying: Some(Arc::new(error)),
        };
        let sorted: std::collections::BTreeMap<&String, &RemoteMediaDiskCacheEntry> = index.iter().collect();
        let data = serde_json::to_vec(&sorted).map_err(|error| write_error(std::io::Error::other(error)))?;
        let staging = account.join(format!(".index-{}", Uuid::new_v4().uuid_string().to_lowercase()));
        std::fs::write(&staging, &data).map_err(write_error)?;
        if let Err(error) = std::fs::rename(&staging, &url) {
            FileManager.remove_item(&staging).ok();
            return Err(write_error(error));
        }
        Self::apply_storage_attributes(&url);
        Ok(())
    }

    fn trimmed_index(
        input: HashMap<String, RemoteMediaDiskCacheEntry>,
        files: &Path,
        limits: &RemoteMediaDiskCacheLimits,
    ) -> Result<HashMap<String, RemoteMediaDiskCacheEntry>, CocoaError> {
        let mut index = input;
        let mut total_bytes: i64 = index.values().map(|entry| entry.byte_count.max(0)).sum();
        let over_budget = |index: &HashMap<String, RemoteMediaDiskCacheEntry>, total_bytes: i64| {
            i64::try_from(index.len()).unwrap_or(i64::MAX) > limits.maximum_entry_count
                || total_bytes > limits.maximum_total_bytes
        };
        if !over_budget(&index, total_bytes) {
            return Ok(index);
        }
        let mut candidates: Vec<RemoteMediaDiskCacheEntry> = index.values().cloned().collect();
        candidates.sort_by(|lhs, rhs| {
            if lhs.last_accessed_at == rhs.last_accessed_at {
                return lhs.created_at.total_cmp(&rhs.created_at);
            }
            lhs.last_accessed_at.total_cmp(&rhs.last_accessed_at)
        });
        for entry in candidates {
            if !over_budget(&index, total_bytes) {
                break;
            }
            if Self::is_leased(&entry.key_digest) {
                continue;
            }
            let file = files.join(&entry.file_name);
            if FileManager.file_exists(&file) && FileManager.remove_item(&file).is_err() {
                continue;
            }
            index.remove(&entry.key_digest);
            total_bytes = (total_bytes - entry.byte_count.max(0)).max(0);
        }
        Ok(index)
    }

    fn is_valid(entry: &RemoteMediaDiskCacheEntry, files: &Path) -> bool {
        if entry.byte_count <= 0
            || !Self::safe_filename(&entry.file_name)
            || Self::digest_from_file_name(&entry.file_name).as_deref() != Some(entry.key_digest.as_str())
        {
            return false;
        }
        let file = files.join(&entry.file_name);
        Self::file_byte_count(&file) == entry.byte_count
            && Self::is_supported_payload(
                &file,
                entry.content_type.as_deref(),
                Self::path_extension(&entry.file_name).eq_ignore_ascii_case("json"),
            )
    }

    fn cache_identity(remote: &Url, explicit_key: Option<&str>) -> RemoteMediaDiskCacheCacheIdentity {
        let trimmed = explicit_key.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        let normalized_key = if trimmed.is_empty() {
            let without_fragment = UrlComponents::from_url(remote, false).and_then(|mut components| {
                components.set_fragment(None);
                components.url()
            });
            format!(
                "url:{}",
                without_fragment
                    .as_ref()
                    .map_or(remote.absolute_string(), Url::absolute_string)
            )
        } else {
            format!("key:{trimmed}")
        };
        RemoteMediaDiskCacheCacheIdentity {
            digest: Self::digest_hex(&normalized_key),
            source_digest: Self::digest_hex(&Self::canonical_source_identity(remote)),
            uses_explicit_key: !trimmed.is_empty(),
        }
    }

    fn source_matches(entry: &RemoteMediaDiskCacheEntry, identity: &RemoteMediaDiskCacheCacheIdentity) -> bool {
        !identity.uses_explicit_key || entry.source_digest.as_deref() == Some(identity.source_digest.as_str())
    }

    fn canonical_source_identity(remote: &Url) -> String {
        let scheme = remote.scheme().unwrap_or_default().to_lowercase();
        let host = remote.host().unwrap_or_default().to_lowercase();
        let port = Self::effective_port(remote).map(|port| port.to_string()).unwrap_or_default();
        let encoded_path = UrlComponents::from_url(remote, false)
            .map_or_else(|| remote.path(), |components| components.percent_encoded_path().to_owned());
        let path = if encoded_path.is_empty() { "/".to_owned() } else { encoded_path };
        format!("{scheme}|{host}|{port}|{path}")
    }

    fn account_directory_name(user_id: Option<&str>) -> String {
        let value = user_id.map(trimming_whitespaces_and_newlines).unwrap_or_default();
        if value.is_empty() {
            return Self::ANONYMOUS_ACCOUNT.to_owned();
        }
        Self::digest_hex(value)
    }

    fn digest_hex(value: &str) -> String {
        sha256_hex(value.as_bytes())
    }

    fn digest_from_file_name(name: &str) -> Option<String> {
        if name.starts_with('.') {
            return None;
        }
        let (digest, _) = name.split_once('.')?;
        if digest.len() != 64 || !digest.chars().all(|character| character.is_ascii_hexdigit()) {
            return None;
        }
        Some(digest.to_lowercase())
    }

    fn safe_filename(name: &str) -> bool {
        !name.is_empty()
            && Path::new(name).file_name().and_then(|last| last.to_str()) == Some(name)
            && !name.contains('/')
            && !name.contains('\\')
    }

    fn path_extension(name: &str) -> String {
        let last = name.rsplit('/').next().unwrap_or_default();
        match last.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => extension.to_owned(),
            _ => String::new(),
        }
    }

    fn preferred_extension(
        suggested_filename: Option<&str>,
        response_filename: Option<&str>,
        remote: &Url,
        content_type: Option<&str>,
    ) -> String {
        let remote_last = remote.last_path_component();
        for candidate in [suggested_filename, response_filename, Some(remote_last.as_str())]
            .into_iter()
            .flatten()
        {
            let normalized: String = Self::path_extension(candidate)
                .to_lowercase()
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .take(12)
                .collect();
            if !normalized.is_empty() {
                return normalized;
            }
        }
        let media_type = content_type.and_then(|value| value.split(';').next());
        match media_type {
            Some("video/mp4") => "mp4",
            Some("video/quicktime") => "mov",
            Some("audio/mpeg") => "mp3",
            Some("audio/mp4" | "audio/x-m4a") => "m4a",
            Some("audio/ogg" | "video/ogg") => "ogg",
            Some("audio/wav" | "audio/x-wav") => "wav",
            Some("application/pdf") => "pdf",
            Some("application/zip") => "zip",
            Some("text/plain") => "txt",
            _ => "bin",
        }
        .to_owned()
    }

    pub fn is_supported_payload(file_url: &Path, content_type: Option<&str>, allows_json: bool) -> bool {
        let media_type = content_type.and_then(|value| value.split(';').next()).unwrap_or_default();
        let is_json = media_type == "application/json" || media_type == "text/json" || media_type.ends_with("+json");
        if media_type == "text/html"
            || media_type == "application/xhtml+xml"
            || media_type == "application/xml"
            || media_type == "text/xml"
            || (is_json && !allows_json)
        {
            return false;
        }
        let Ok(handle) = std::fs::File::open(file_url) else {
            return false;
        };
        let mut head = Vec::new();
        if handle.take(Self::PAYLOAD_SNIFF_BYTES).read_to_end(&mut head).is_err() || head.is_empty() {
            return false;
        }
        let prefix = trimming_whitespaces_and_newlines(&String::from_utf8_lossy(&head)).to_lowercase();
        !(prefix.starts_with("<!doctype html")
            || prefix.starts_with("<html")
            || prefix.starts_with("<?xml")
            || (!allows_json && (prefix.starts_with('{') || prefix.starts_with('['))))
    }

    fn is_http(url: &Url) -> bool {
        let scheme = url.scheme().map(str::to_lowercase);
        matches!(scheme.as_deref(), Some("https" | "http"))
    }

    /// the one place that attaches the account token to a same origin secure media route
    /// streaming loader signs its own range requests and must not copy it
    /// also refreshes an expired token
    pub async fn bearer_token(remote: &Url, explicit_token: Option<&str>, cx: &AsyncApp) -> Option<String> {
        if !remote
            .path()
            .to_lowercase()
            .contains(rise_wire_canon::SECURE_MEDIA_PATH_PREFIX)
        {
            return None;
        }
        let (base, access_token, active_user_id) = cx.update(|cx| {
            let configuration = SaiFetchHttpConfiguration::shared(cx);
            (
                configuration.read(cx).base_url.clone(),
                sai_auth_access_token(cx)
                    .map(|token| trimming_whitespaces_and_newlines(&token).to_owned())
                    .unwrap_or_default(),
                SaiAccountCacheScope::active_user_id(),
            )
        });
        let Some(base) = base else {
            Self::trace_missing_bearer("no api base url", remote);
            return None;
        };
        if !Self::same_origin(remote, &base) {
            Self::trace_missing_bearer("foreign origin", remote);
            return None;
        }
        if explicit_token.is_none() && access_token.is_empty() {
            Self::trace_missing_bearer("empty access token in auth store", remote);
        }
        if let Some(explicit_token) = explicit_token {
            let token = trimming_whitespaces_and_newlines(explicit_token);
            if token.is_empty() {
                return None;
            }
            let owner = sai_auth_hooks().and_then(|hooks| (hooks.extract_user_id)(token));
            let is_active_account = matches!((&owner, &active_user_id), (Some(owner), Some(active)) if owner == active);
            if !is_active_account {
                return Some(token.to_owned());
            }
            return Self::fresh_active_account_token(token.to_owned(), cx).await;
        }
        Self::fresh_active_account_token(access_token, cx).await
    }

    /// tester diagnostics, why a secure media request left without a bearer
    fn trace_missing_bearer(reason: &'static str, remote: &Url) {
        static TRACED: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
        let mut traced = TRACED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if traced.contains(&reason) {
            return;
        }
        traced.push(reason);
        tracing::warn!(target: "riseonly::secure_media", "no bearer: {reason} for {}", remote.absolute_string());
    }

    async fn fresh_active_account_token(token: String, cx: &AsyncApp) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        if !sai_auth_is_expired(&token, Self::ACCESS_TOKEN_LEEWAY_SECONDS) {
            return Some(token);
        }
        if let Some(hooks) = sai_auth_hooks() {
            let refresh = cx.update(|cx| (hooks.ensure_valid_access_token)(cx));
            if let Err(error) = refresh.await {
                tracing::warn!(target: "riseonly::X657", %error, "ensure valid token before secure fetch failed");
            }
        }
        let refreshed = cx.update(|cx| sai_auth_access_token(cx));
        if refreshed.is_none() {
            tracing::warn!(target: "riseonly::secure_media", "no bearer: token expired and refresh left none");
        }
        refreshed
    }

    pub async fn bearer_token_after_refusal(remote: &Url, refused: &str, cx: &AsyncApp) -> Option<String> {
        static REFRESHED_AFTER_REFUSAL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
        if !remote
            .path()
            .to_lowercase()
            .contains(rise_wire_canon::SECURE_MEDIA_PATH_PREFIX)
        {
            return None;
        }
        let hooks = sai_auth_hooks()?;
        let current = cx.update(|cx| sai_auth_access_token(cx));
        if let Some(current) = current.filter(|current| current != refused) {
            return Some(current);
        }
        let already_refreshed = REFRESHED_AFTER_REFUSAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_deref()
            == Some(refused);
        if already_refreshed {
            return None;
        }
        tracing::warn!(target: "riseonly::X657", "refresh after a secure 401 starts");
        let refresh = cx.update(|cx| (hooks.refresh_access_token)(cx));
        if let Err(error) = refresh.await {
            tracing::warn!(target: "riseonly::secure_media", %error, "refresh after a secure 401 failed");
            return None;
        }
        let Some(refreshed) = cx.update(|cx| sai_auth_access_token(cx)).filter(|refreshed| refreshed != refused) else {
            tracing::warn!(
                target: "riseonly::X555",
                "refresh after a secure 401 left the refused token: refused_expired {} store_empty {}",
                sai_auth_is_expired(refused, 0.0),
                cx.update(|cx| sai_auth_access_token(cx)).is_none()
            );
            return None;
        };
        *REFRESHED_AFTER_REFUSAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(refreshed.clone());
        tracing::warn!(target: "riseonly::X422", "secure 401: token refreshed, retrying once");
        Some(refreshed)
    }

    /// request to continue a redirect with
    /// the secure route answers 307 to a presigned address
    /// and storage rejects the app Authorization header there
    pub fn redirect_request(original_request: &URLRequest, proposed: &URLRequest) -> URLRequest {
        let authorization = reqwest::header::AUTHORIZATION.as_str();
        if proposed.value(authorization).is_none() || Self::same_origin(&original_request.url, &proposed.url) {
            return proposed.clone();
        }
        let mut sanitized = URLRequest::new(proposed.url.clone());
        sanitized.http_method = proposed.http_method.clone();
        sanitized.http_body = proposed.http_body.clone();
        sanitized.timeout_interval = proposed.timeout_interval;
        for (name, value) in proposed.all_http_header_fields() {
            if !name.eq_ignore_ascii_case(authorization) {
                sanitized.set_value(value, name);
            }
        }
        sanitized
    }

    fn same_origin(lhs: &Url, rhs: &Url) -> bool {
        lhs.scheme().map(str::to_lowercase) == rhs.scheme().map(str::to_lowercase)
            && lhs.host().map(|host| host.to_lowercase()) == rhs.host().map(|host| host.to_lowercase())
            && Self::effective_port(lhs) == Self::effective_port(rhs)
    }

    fn effective_port(url: &Url) -> Option<i64> {
        if let Some(port) = url.port() {
            return Some(port);
        }
        match url.scheme().map(str::to_lowercase).as_deref() {
            Some("http") => Some(80),
            Some("https") => Some(443),
            _ => None,
        }
    }

    pub fn file_byte_count(url: &Path) -> i64 {
        FileManager.attributes_of_item(url).map_or(0, |attributes| attributes.size)
    }

    fn apply_storage_attributes(_url: &Path) {}

    fn validate_account(
        user_id: Option<&str>,
        epoch: Option<&str>,
        account_directory_name: &str,
        invalidation_token: u64,
    ) -> Result<(), RemoteMediaTransferError> {
        if !SaiAccountCacheScope::is_current_account_epoch(user_id, epoch)
            || Self::invalidation_token(account_directory_name) != invalidation_token
        {
            return Err(RemoteMediaDiskCacheError::AccountChanged.into());
        }
        Ok(())
    }

    fn validate_account_locked(
        invalidation_tokens: &HashMap<String, u64>,
        user_id: Option<&str>,
        epoch: Option<&str>,
        account_directory_name: &str,
        invalidation_token: u64,
    ) -> Result<(), RemoteMediaTransferError> {
        let current = invalidation_tokens.get(account_directory_name).copied().unwrap_or(0);
        if !SaiAccountCacheScope::is_current_account_epoch(user_id, epoch) || current != invalidation_token {
            return Err(RemoteMediaDiskCacheError::AccountChanged.into());
        }
        Ok(())
    }

    fn invalidation_token(account_directory_name: &str) -> u64 {
        Self::with_file_lock(|tokens| tokens.get(account_directory_name).copied().unwrap_or(0))
    }

    fn invalidate(account_directory_name: &str) -> u64 {
        Self::with_file_lock(|tokens| {
            let next = tokens.get(account_directory_name).copied().unwrap_or(0).wrapping_add(1);
            tokens.insert(account_directory_name.to_owned(), next);
            next
        })
    }

    fn with_file_lock<T>(body: impl FnOnce(&mut HashMap<String, u64>) -> T) -> T {
        let mut tokens = FILE_LOCK.lock();
        body(&mut tokens)
    }
}

/// one hop redirect rule applied to every media transfer
/// clients never follow redirects alone since that would carry the header onward
pub struct RiseMediaRedirectSessionDelegate;

impl RiseMediaRedirectSessionDelegate {
    const MAXIMUM_REDIRECTS: usize = 10;

    /// sends the request and follows redirects by the one hop rule
    /// will_perform_redirection sees each redirect with the proposed next request
    pub async fn send(
        session: &reqwest::Client,
        request: &URLRequest,
        range_start: Option<i64>,
        mut will_perform_redirection: impl FnMut(&RemoteMediaHttpResponse, &URLRequest),
    ) -> Result<(reqwest::Response, Url), URLError> {
        let mut current = request.clone();
        for _ in 0..=Self::MAXIMUM_REDIRECTS {
            let method = reqwest::Method::from_bytes(current.http_method.as_bytes())
                .map_err(|_| URLError::new(URLErrorCode::BadURL))?;
            let mut builder = session.request(method, current.url.absolute_string());
            for (name, value) in current.all_http_header_fields() {
                builder = builder.header(name.as_str(), value.as_str());
            }
            if let Some(start) = range_start {
                builder = builder.header(reqwest::header::RANGE, format!("bytes={start}-"));
            }
            if let Some(body) = &current.http_body {
                builder = builder.body(body.clone());
            }
            let response = tokio::time::timeout(current.timeout_interval, builder.send())
                .await
                .map_err(|_| URLError::new(URLErrorCode::TimedOut))?
                .map_err(|error| RemoteMediaTransferError::url_error(&error))?;
            if !response.status().is_redirection() {
                return Ok((response, current.url));
            }
            let head = RemoteMediaHttpResponse::from_reqwest(&response, current.url.clone());
            let Some(destination) = head
                .value(reqwest::header::LOCATION.as_str())
                .and_then(|location| Url::from_string_relative_to(location, Some(&current.url)))
                .and_then(|resolved| Url::from_string(resolved.absolute_string()))
            else {
                return Ok((response, current.url));
            };
            let mut proposed = current.clone();
            proposed.url = destination;
            if head.status_code == 303 {
                proposed.http_method = reqwest::Method::GET.as_str().to_owned();
                proposed.http_body = None;
            }
            will_perform_redirection(&head, &proposed);
            current = RemoteMediaDiskCache::redirect_request(request, &proposed);
        }
        Err(URLError::new(URLErrorCode::Unknown))
    }
}
