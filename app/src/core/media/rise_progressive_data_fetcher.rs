use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::channel::oneshot;
use gpui::AsyncApp;
use parking_lot::Mutex;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::core::engine_bridge::network_transport_config::NetworkClientSlot;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode, URLRequest};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription};
use crate::core::lib::canon_failure_bridge::RiseMediaRefusal;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SAI_WS_DID_CONNECT;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCacheError, RemoteMediaDownloadProgress, RemoteMediaDownloadProgressCoalescer,
    RemoteMediaDownloadRetryPolicy, RemoteMediaHttpResponse, RemoteMediaRefusalReport, RemoteMediaTransferError,
    RiseMediaRedirectSessionDelegate,
};
use crate::core::media::rise_media_range_policy::RiseMediaRangePolicy;

/// retry for a media request whose connection cant be made
/// waiting for a network change may never end so retry on a backoff and on socket reconnect
pub struct RiseMediaConnectivityRetry {
    handlers: Mutex<HashMap<Uuid, Arc<dyn Fn() + Send + Sync>>>,
    observer: Mutex<Option<RiseSubscription>>,
}

static SHARED: LazyLock<RiseMediaConnectivityRetry> = LazyLock::new(|| RiseMediaConnectivityRetry {
    handlers: Mutex::new(HashMap::new()),
    observer: Mutex::new(None),
});

struct RiseMediaConnectivityRegistration(Uuid);

impl Drop for RiseMediaConnectivityRegistration {
    fn drop(&mut self) {
        RiseMediaConnectivityRetry::shared().unregister(self.0);
    }
}

impl RiseMediaConnectivityRetry {
    const MAXIMUM_DELAY: Duration = Duration::from_secs(8);
    const MAXIMUM_DOUBLINGS: i64 = 10;

    pub fn shared() -> &'static RiseMediaConnectivityRetry {
        &SHARED
    }

    /// first retry waits the canon media delay and each next one doubles it
    pub fn delay(attempt: i64) -> Duration {
        let base = RemoteMediaDownloadRetryPolicy::delay(1);
        let doublings = (attempt - 1).clamp(0, Self::MAXIMUM_DOUBLINGS);
        let doubled = base.saturating_mul(1u32 << doublings);
        doubled.min(Self::MAXIMUM_DELAY)
    }

    pub fn register(&self, handler: impl Fn() + Send + Sync + 'static) -> Uuid {
        let identifier = Uuid::new_v4();
        self.handlers.lock().insert(identifier, Arc::new(handler));
        let mut observer = self.observer.lock();
        if observer.is_none() {
            *observer = Some(
                RiseAppEvents::default_center()
                    .observe(SAI_WS_DID_CONNECT, |_| RiseMediaConnectivityRetry::shared().fire()),
            );
        }
        identifier
    }

    pub fn unregister(&self, identifier: Uuid) {
        self.handlers.lock().remove(&identifier);
    }

    fn fire(&self) {
        let current: Vec<Arc<dyn Fn() + Send + Sync>> = self.handlers.lock().values().cloned().collect();
        for handler in current {
            handler();
        }
    }

    /// parks a transfer without connection until its backoff ends or the socket reconnects
    pub async fn wait_for_retry(&self, attempt: i64) {
        let (sender, receiver) = oneshot::channel::<()>();
        let sender = Mutex::new(Some(sender));
        let _registration = RiseMediaConnectivityRegistration(self.register(move || {
            if let Some(sender) = sender.lock().take() {
                sender.send(()).ok();
            }
        }));
        let backoff = Box::pin(RiseRuntimeTimer::sleep(Self::delay(attempt)));
        futures::future::select(backoff, receiver).await;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseProgressiveDataFetcherLane {
    Standard,
    PrivateEphemeral,
}

pub type RiseProgressiveDataFetcherResult = (Vec<u8>, RemoteMediaHttpResponse);

pub type RiseProgressiveDataFetcherOnProgress = Arc<dyn Fn(RemoteMediaDownloadProgress) + Send + Sync>;
pub type RiseProgressiveDataFetcherOnResponse = Arc<dyn Fn(&RemoteMediaHttpResponse) -> bool + Send + Sync>;
pub type RiseProgressiveDataFetcherOnChunk = Arc<dyn Fn(&[u8]) + Send + Sync>;

struct RiseProgressiveDataFetcherPending {
    request: URLRequest,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    maximum_byte_count: i64,
    on_progress: RiseProgressiveDataFetcherOnProgress,
    on_response: Option<RiseProgressiveDataFetcherOnResponse>,
    on_chunk: Option<RiseProgressiveDataFetcherOnChunk>,
}

/// how a transfer ended on the account runtime, a refusal is published on main
enum RiseProgressiveDataFetcherOutcome {
    Completed(RiseProgressiveDataFetcherResult),
    Refusal(RemoteMediaRefusalReport),
}

struct RiseProgressiveDataFetcherAbort(tokio::task::AbortHandle);

impl Drop for RiseProgressiveDataFetcherAbort {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// data fetch that reports bytes as they arrive, so a ring follows a 12 MB photo
/// one shared client per lane, per request breaks connection reuse
/// buffers in memory, aborts mid flight at the caller ceiling
pub struct RiseProgressiveDataFetcher {
    lane: RiseProgressiveDataFetcherLane,
    session: NetworkClientSlot,
}

static STANDARD_FETCHER: LazyLock<RiseProgressiveDataFetcher> =
    LazyLock::new(|| RiseProgressiveDataFetcher::new(RiseProgressiveDataFetcherLane::Standard));
static PRIVATE_FETCHER: LazyLock<RiseProgressiveDataFetcher> =
    LazyLock::new(|| RiseProgressiveDataFetcher::new(RiseProgressiveDataFetcherLane::PrivateEphemeral));

impl RiseProgressiveDataFetcher {
    pub const DEFAULT_PRIORITY: f32 = 0.5;

    fn new(lane: RiseProgressiveDataFetcherLane) -> Self {
        Self { lane, session: NetworkClientSlot::without_redirects() }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn data(
        request: URLRequest,
        maximum_byte_count: i64,
        lane: RiseProgressiveDataFetcherLane,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        task_priority: f32,
        on_progress: RiseProgressiveDataFetcherOnProgress,
        cx: &mut AsyncApp,
    ) -> Result<RiseProgressiveDataFetcherResult, RemoteMediaTransferError> {
        Self::fetcher(lane)
            .perform(
                RiseProgressiveDataFetcherPending {
                    request,
                    account_user_id,
                    account_epoch,
                    maximum_byte_count: maximum_byte_count.max(1),
                    on_progress,
                    on_response: None,
                    on_chunk: None,
                },
                task_priority,
                cx,
            )
            .await
    }

    /// hands over every chunk as it lands instead of buffering
    /// a 300 MB video cant sit in memory, bytes go to the partial file first
    /// on_response false rejects and cancels
    pub async fn stream(
        request: URLRequest,
        lane: RiseProgressiveDataFetcherLane,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        on_response: RiseProgressiveDataFetcherOnResponse,
        on_chunk: RiseProgressiveDataFetcherOnChunk,
        cx: &mut AsyncApp,
    ) -> Result<(), RemoteMediaTransferError> {
        Self::fetcher(lane)
            .perform(
                RiseProgressiveDataFetcherPending {
                    request,
                    account_user_id,
                    account_epoch,
                    maximum_byte_count: i64::MAX,
                    on_progress: Arc::new(|_| {}),
                    on_response: Some(on_response),
                    on_chunk: Some(on_chunk),
                },
                Self::DEFAULT_PRIORITY,
                cx,
            )
            .await
            .map(|_| ())
    }

    fn fetcher(lane: RiseProgressiveDataFetcherLane) -> &'static RiseProgressiveDataFetcher {
        match lane {
            RiseProgressiveDataFetcherLane::Standard => &STANDARD_FETCHER,
            RiseProgressiveDataFetcherLane::PrivateEphemeral => &PRIVATE_FETCHER,
        }
    }

    async fn perform(
        &'static self,
        entry: RiseProgressiveDataFetcherPending,
        _task_priority: f32,
        cx: &mut AsyncApp,
    ) -> Result<RiseProgressiveDataFetcherResult, RemoteMediaTransferError> {
        let is_streaming = entry.on_chunk.is_some();
        let handle = RiseAccountRuntime::current().spawn(self.transfer(entry));
        let _abort = RiseProgressiveDataFetcherAbort(handle.abort_handle());
        let outcome = handle
            .await
            .map_err(|_| RemoteMediaTransferError::from(CancellationError))??;
        match outcome {
            RiseProgressiveDataFetcherOutcome::Completed(result) => Ok(result),
            RiseProgressiveDataFetcherOutcome::Refusal(report) => {
                let payload = report.body.clone();
                let response = report.response.clone();
                let status_code = response.status_code;
                report.publish(cx).await;
                if is_streaming {
                    return Err(RemoteMediaDiskCacheError::InvalidResponse {
                        status_code: Some(status_code),
                    }
                    .into());
                }
                Ok((payload, response))
            }
        }
    }

    async fn transfer(
        &self,
        entry: RiseProgressiveDataFetcherPending,
    ) -> Result<RiseProgressiveDataFetcherOutcome, RemoteMediaTransferError> {
        let Some(session) = self.session.current() else {
            return Err(URLError::new(URLErrorCode::SecureConnectionFailed).into());
        };
        let session = &session;
        let original_url = entry.request.url.clone();
        let mut connectivity_retries: i64 = 0;
        let (mut response, url) = loop {
            let sent = RiseMediaRedirectSessionDelegate::send(session, &entry.request, None, |redirect, _| {
                RiseMediaRangePolicy::record(redirect, Some(&original_url));
            })
            .await;
            match sent {
                Ok(sent) => break sent,
                Err(error) if RemoteMediaTransferError::is_waiting_for_connectivity(&error) => {
                    connectivity_retries += 1;
                    RiseMediaConnectivityRetry::shared()
                        .wait_for_retry(connectivity_retries)
                        .await;
                    if entry.account_user_id.is_some()
                        && !SaiAccountCacheScope::is_current_account_epoch(
                            entry.account_user_id.as_deref(),
                            entry.account_epoch.as_deref(),
                        )
                    {
                        return Err(RemoteMediaDiskCacheError::AccountChanged.into());
                    }
                }
                Err(error) => return Err(error.into()),
            }
        };
        let head = RemoteMediaHttpResponse::from_reqwest(&response, url);
        let is_media_refusal =
            RiseMediaRefusal::is_protected_refusal(Some(head.status_code), Some(&head.url), &entry.request);
        if !is_media_refusal
            && let Some(veto) = &entry.on_response
            && !veto(&head)
        {
            return Err(RemoteMediaDiskCacheError::InvalidResponse {
                status_code: Some(head.status_code),
            }
            .into());
        }
        let expected = head.expected_content_length().max(0);
        let ceiling = if is_media_refusal {
            entry
                .maximum_byte_count
                .min(i64::try_from(RiseMediaRefusal::MAXIMUM_BODY_BYTES).unwrap_or(i64::MAX))
        } else {
            entry.maximum_byte_count
        };
        if expected > ceiling {
            return Err(RemoteMediaDiskCacheError::PayloadTooLarge.into());
        }
        let sink = entry.on_chunk.as_ref().filter(|_| !is_media_refusal);
        let mut buffer: Vec<u8> = Vec::new();
        if expected > 0 && entry.on_chunk.is_none() {
            buffer.reserve(usize::try_from(expected.min(entry.maximum_byte_count)).unwrap_or(0));
        }
        let mut coalescer = RemoteMediaDownloadProgressCoalescer::default();
        loop {
            let chunk = tokio::time::timeout(entry.request.timeout_interval, response.chunk())
                .await
                .map_err(|_| URLError::new(URLErrorCode::TimedOut))?
                .map_err(|error| RemoteMediaTransferError::url_error(&error))?;
            let Some(data) = chunk else {
                break;
            };
            if let Some(sink) = sink {
                sink(&data);
                continue;
            }
            buffer.extend_from_slice(&data);
            let received = i64::try_from(buffer.len()).unwrap_or(i64::MAX);
            if received > ceiling {
                return Err(RemoteMediaDiskCacheError::PayloadTooLarge.into());
            }
            let progress = RemoteMediaDownloadProgress::new(received, (expected > 0).then_some(expected));
            if coalescer.should_publish(progress, RemoteMediaDownloadProgressCoalescer::uptime()) {
                (entry.on_progress)(progress);
            }
        }

        if is_media_refusal {
            return Ok(RiseProgressiveDataFetcherOutcome::Refusal(RemoteMediaRefusalReport {
                body: buffer,
                response: head,
                request: entry.request,
                account_user_id: entry.account_user_id,
                account_epoch: entry.account_epoch,
            }));
        }
        if entry.on_chunk.is_none() {
            let count = i64::try_from(buffer.len()).unwrap_or(i64::MAX);
            (entry.on_progress)(RemoteMediaDownloadProgress::new(count, Some(count)));
        }
        Ok(RiseProgressiveDataFetcherOutcome::Completed((buffer, head)))
    }

    pub fn lane(&self) -> RiseProgressiveDataFetcherLane {
        self.lane
    }
}
