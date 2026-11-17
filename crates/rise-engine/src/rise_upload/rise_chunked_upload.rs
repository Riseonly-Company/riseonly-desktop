use crate::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use parking_lot::Mutex;
use rise_canon::rise_key;
use rise_canon::{RiseLimits, RiseTimingCanon, rise_upload};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_data::base64_encoded_string;
use rise_core::foundation::foundation_date::RiseClock;
use super::rise_upload_byte_estimator::RiseUploadByteEstimator;
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_platform::system_activity::{
    SystemActivityEvent, SystemActivityKind, SystemActivityUnavailable, watch,
};

use super::rise_upload_progress::RiseUploadCancellationSignal;
use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use super::rise_url_session::{
    HTTPURLResponse, URLError, URLErrorCode, URLRequest, URLSession, URLSessionTaskDelegate,
};
use crate::rise_wire::rise_json_value::RiseJSONValue;
use crate::rise_wire::rise_wire::CancellationError;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseChunkedUploadError {
    #[error("The upload endpoint is invalid")]
    InvalidEndpoint,
    #[error("Cannot upload an empty file")]
    EmptyFile,
    #[error("{0}")]
    InvalidResponse(String),
    #[error("File upload HTTP {0}: {1}")]
    HttpStatus(i64, String),
}

impl RiseChunkedUploadError {
    pub fn error_description(&self) -> Option<String> {
        Some(self.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseChunkedUploadRequest {
    pub file_url: PathBuf,
    pub filename: String,
    pub content_type: String,
    pub file_context: String,
    pub resource_id: Option<String>,
    pub video_width: Option<i64>,
    pub video_height: Option<i64>,
    pub video_duration: Option<i64>,
    pub video_thumbnail_file_url: Option<PathBuf>,
    pub awaits_network: bool,
}

impl RiseChunkedUploadRequest {
    /// upload source with every optional hint left out
    pub fn new(
        file_url: PathBuf,
        filename: String,
        content_type: String,
        file_context: String,
    ) -> Self {
        Self {
            file_url,
            filename,
            content_type,
            file_context,
            resource_id: None,
            video_width: None,
            video_height: None,
            video_duration: None,
            video_thumbnail_file_url: None,
            awaits_network: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiseChunkedUploadTransferProgress {
    pub fraction: f64,
    pub completed_bytes: i64,
    pub total_bytes: i64,
}

impl RiseChunkedUploadTransferProgress {
    pub fn new(fraction: f64, completed_bytes: i64, total_bytes: i64) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            total_bytes: total_bytes.max(0),
            completed_bytes: completed_bytes.max(0).min(total_bytes.max(0)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RiseChunkedUploadSessionEvent {
    Started { upload_id: String },
    Finished { upload_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseChunkedUploadResult {
    pub asset_id: String,
    pub file_url: String,
    pub file_key: String,
    pub access_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub file_id: Option<String>,
    pub mime_type: String,
    pub file_size: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
    pub metadata: BTreeMap<String, RiseJSONValue>,
}

/// async fn that authorizes a request
pub type RiseChunkedUploadAuthorize =
    Arc<dyn Fn(URLRequest) -> BoxFuture<'static, anyhow::Result<URLRequest>> + Send + Sync>;
pub type RiseChunkedUploadStep =
    Arc<dyn Fn() -> BoxFuture<'static, anyhow::Result<()>> + Send + Sync>;
pub type RiseChunkedUploadSessionEventHandler =
    Arc<dyn Fn(RiseChunkedUploadSessionEvent) -> BoxFuture<'static, ()> + Send + Sync>;
pub type RiseChunkedUploadProgressHandler =
    Arc<dyn Fn(RiseChunkedUploadTransferProgress) + Send + Sync>;

#[derive(Clone)]
pub struct RiseChunkedUploadConfiguration {
    pub base_url: Url,
    pub default_headers: HashMap<String, String>,
    pub authorize: RiseChunkedUploadAuthorize,
    pub authorize_cleanup: RiseChunkedUploadAuthorize,
    pub refresh_authorization: RiseChunkedUploadStep,
    pub validate_context: RiseChunkedUploadStep,
    pub premium: bool,
}

impl RiseChunkedUploadConfiguration {
    /// builds the uploader, none for an arg takes its default
    pub fn new(
        base_url: Url,
        default_headers: HashMap<String, String>,
        authorize: Option<RiseChunkedUploadAuthorize>,
        authorize_cleanup: Option<RiseChunkedUploadAuthorize>,
        refresh_authorization: Option<RiseChunkedUploadStep>,
        validate_context: Option<RiseChunkedUploadStep>,
        premium: bool,
    ) -> Self {
        let authorize: RiseChunkedUploadAuthorize = authorize.unwrap_or_else(|| {
            Arc::new(|request: URLRequest| -> BoxFuture<'static, anyhow::Result<URLRequest>> {
                Box::pin(async move { Ok(request) })
            })
        });
        let authorize_cleanup = authorize_cleanup.unwrap_or_else(|| Arc::clone(&authorize));
        Self {
            base_url,
            default_headers,
            authorize,
            authorize_cleanup,
            refresh_authorization: refresh_authorization.unwrap_or_else(Self::no_step),
            validate_context: validate_context.unwrap_or_else(Self::no_step),
            premium,
        }
    }

    fn no_step() -> RiseChunkedUploadStep {
        Arc::new(|| -> BoxFuture<'static, anyhow::Result<()>> { Box::pin(async { Ok(()) }) })
    }
}

struct RiseChunkedUploadAbortOnDrop(tokio::task::AbortHandle);

impl Drop for RiseChunkedUploadAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
pub struct RiseChunkedUploadClient {
    configuration: RiseChunkedUploadConfiguration,
}

impl RiseChunkedUploadClient {
    pub const MINIMUM_SLAB_SIZE: i64 = rise_upload::SLAB_MINIMUM_BYTES as i64;
    pub const DEFAULT_SLAB_SIZE: i64 = rise_upload::SLAB_DEFAULT_BYTES as i64;
    pub const MAXIMUM_SLAB_SIZE: i64 = rise_upload::SLAB_MAXIMUM_BYTES as i64;
    pub const PREFERRED_PIPELINE_DEPTH: i64 = rise_upload::PIPELINE_MAXIMUM_DEPTH as i64;

    const SLAB_SEND_STALL_TIMEOUT: Duration = Duration::from_secs(60);
    const SLAB_STALL_CHECK_INTERVAL: Duration = Duration::from_nanos(5_000_000_000);
    const MAXIMUM_OFFLINE_PAUSE_DOUBLINGS: u32 = 3;
    const SERVER_BOUNCE_STATUSES: [i64; 3] = [502, 503, 504];

    pub const CONNECTION_LOSS_CODES: [URLErrorCode; 8] = [
        URLErrorCode::NetworkConnectionLost,
        URLErrorCode::TimedOut,
        URLErrorCode::CannotConnectToHost,
        URLErrorCode::NotConnectedToInternet,
        URLErrorCode::DnsLookupFailed,
        URLErrorCode::CannotFindHost,
        URLErrorCode::SecureConnectionFailed,
        URLErrorCode::DataNotAllowed,
    ];

    pub fn preferred_pipeline_depth(premium: bool) -> i64 {
        if premium {
            rise_upload::PIPELINE_MAXIMUM_DEPTH_PREMIUM as i64
        } else {
            rise_upload::PIPELINE_MAXIMUM_DEPTH as i64
        }
    }

    fn slab_request_timeout() -> Duration {
        let window = Duration::from_millis(u64::try_from(Self::upload_session_window_ms()).unwrap_or(0));
        Self::SLAB_SEND_STALL_TIMEOUT.max(window)
    }

    fn upload_session_window_ms() -> i64 {
        RiseTimingCanon::SERVER_WINDOWS
            .iter()
            .find(|window| window.name == "upload_session_ttl")
            .map_or(0, |window| window.window_ms)
    }

    /// the final slab is the short one and comes first
    /// then waits on the server for the earlier ones and gets a 503 when that runs out
    /// so it leaves only after every earlier slab is acked
    fn holds_final_slab(next_offset: i64, slab_size: i64, total_bytes: i64, in_flight: i64) -> bool {
        in_flight > 0 && next_offset + slab_size >= total_bytes
    }

    pub fn maximum_inline_video_thumbnail_base64_chars() -> i64 {
        Self::canon_service_limit("api_gateway_request_max_string_chars")
    }

    pub fn maximum_inline_video_thumbnail_bytes() -> i64 {
        Self::canon_service_limit("max_client_video_thumbnail_bytes")
            .min(Self::maximum_inline_video_thumbnail_base64_chars() / 4 * 3)
    }

    fn canon_service_limit(name: &str) -> i64 {
        match RiseLimits::SERVICE_LIMITS.iter().find(|spec| spec.name == name) {
            Some(spec) => i64::try_from(spec.value).unwrap_or(i64::MAX),
            None => panic!("Canon service limit {name} is missing"),
        }
    }

    pub fn new(configuration: RiseChunkedUploadConfiguration) -> Self {
        Self { configuration }
    }

    /// uploads in chunks, none handlers do nothing
    /// any thread ok, needs the account runtime reactor
    /// dropping the future aborts it
    pub async fn upload(
        &self,
        request: RiseChunkedUploadRequest,
        cancellation_signal: Option<RiseUploadCancellationSignal>,
        on_session_event: Option<RiseChunkedUploadSessionEventHandler>,
        on_progress: Option<RiseChunkedUploadProgressHandler>,
    ) -> anyhow::Result<RiseChunkedUploadResult> {
        let client = self.clone();
        let upload = RiseAccountRuntime::current().spawn(async move {
            client.upload_on_runtime(request, cancellation_signal, on_session_event, on_progress).await
        });
        let _abort = RiseChunkedUploadAbortOnDrop(upload.abort_handle());
        upload.await.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    async fn upload_on_runtime(
        &self,
        request: RiseChunkedUploadRequest,
        cancellation_signal: Option<RiseUploadCancellationSignal>,
        on_session_event: Option<RiseChunkedUploadSessionEventHandler>,
        on_progress: Option<RiseChunkedUploadProgressHandler>,
    ) -> anyhow::Result<RiseChunkedUploadResult> {
        let on_session_event = on_session_event.unwrap_or_else(|| {
            Arc::new(|_: RiseChunkedUploadSessionEvent| -> BoxFuture<'static, ()> {
                Box::pin(async {})
            })
        });
        let on_progress: RiseChunkedUploadProgressHandler =
            on_progress.unwrap_or_else(|| Arc::new(|_: RiseChunkedUploadTransferProgress| {}));
        let Some(cancellation_signal) = cancellation_signal else {
            return self
                .perform_upload(request, on_session_event, on_progress)
                .await;
        };
        if cancellation_signal.is_cancelled() {
            return Err(CancellationError.into());
        }

        let (cancelled, mut cancellation) = tokio::sync::oneshot::channel::<()>();
        let cancelled = Mutex::new(Some(cancelled));
        let registration = cancellation_signal.on_cancel(move || {
            if let Some(cancelled) = cancelled.lock().take() {
                let _ = cancelled.send(());
            }
        });

        let operation = self.perform_upload(request, on_session_event, on_progress);
        let outcome = tokio::select! {
            result = operation => result.map_err(|error| {
                if cancellation_signal.is_cancelled() || error.is::<CancellationError>() {
                    CancellationError.into()
                } else {
                    error
                }
            }),
            Ok(()) = &mut cancellation => Err(CancellationError.into()),
        };
        registration.cancel();
        outcome
    }

    async fn perform_upload(
        &self,
        request: RiseChunkedUploadRequest,
        on_session_event: RiseChunkedUploadSessionEventHandler,
        on_progress: RiseChunkedUploadProgressHandler,
    ) -> anyhow::Result<RiseChunkedUploadResult> {
        (self.configuration.validate_context)().await?;
        let file_url = request.file_url.clone();
        let source = Arc::new(
            RiseAccountRuntime::current().spawn_blocking(move || RiseUploadFileSource::new(&file_url)).await??,
        );
        let _source_close = RiseUploadFileSourceClose(Arc::clone(&source));
        let total_bytes = source.total_bytes;
        if total_bytes <= 0 {
            return Err(RiseChunkedUploadError::EmptyFile.into());
        }

        let mut initiate_body: BTreeMap<String, RiseJSONValue> = BTreeMap::from([
            (
                rise_key::file::FILENAME.to_owned(),
                RiseJSONValue::String(request.filename.clone()),
            ),
            (
                rise_key::file::CONTENT_TYPE.to_owned(),
                RiseJSONValue::String(request.content_type.clone()),
            ),
            (
                rise_key::upload::FILE_CONTEXT.to_owned(),
                RiseJSONValue::String(request.file_context.clone()),
            ),
            (
                rise_key::upload::TOTAL_SIZE.to_owned(),
                RiseJSONValue::Integer(total_bytes),
            ),
        ]);
        if let Some(resource_id) = &request.resource_id {
            initiate_body.insert(
                rise_key::file::RESOURCE_ID.to_owned(),
                RiseJSONValue::String(resource_id.clone()),
            );
        }
        if request.content_type.to_lowercase().starts_with("video/") {
            if let Some(width) = request.video_width.filter(|width| *width > 0) {
                initiate_body.insert(
                    rise_key::message::VIDEO_WIDTH.to_owned(),
                    RiseJSONValue::Integer(width),
                );
            }
            if let Some(height) = request.video_height.filter(|height| *height > 0) {
                initiate_body.insert(
                    rise_key::message::VIDEO_HEIGHT.to_owned(),
                    RiseJSONValue::Integer(height),
                );
            }
            if let Some(duration) = request.video_duration.filter(|duration| *duration > 0) {
                initiate_body.insert(
                    rise_key::message::VIDEO_DURATION.to_owned(),
                    RiseJSONValue::Integer(duration),
                );
            }
        }
        if let Some(thumbnail_url) = &request.video_thumbnail_file_url
            && let Some(thumbnail_base64) = Self::video_thumbnail_base64(thumbnail_url).await
        {
            initiate_body.insert(
                rise_key::upload::VIDEO_THUMBNAIL_BASE64.to_owned(),
                RiseJSONValue::String(thumbnail_base64),
            );
        }
        let initiate = self
            .initiate_session(RiseJSONValue::Object(initiate_body), request.awaits_network)
            .await?;
        let upload_id = Self::required_string(&initiate, &[rise_key::file::UPLOAD_ID])?;
        on_session_event(RiseChunkedUploadSessionEvent::Started {
            upload_id: upload_id.clone(),
        })
        .await;
        let mut session = RiseChunkedUploadSessionGuard {
            upload_id: upload_id.clone(),
            on_session_event: Arc::clone(&on_session_event),
            configuration: self.configuration.clone(),
            completed_successfully: false,
        };
        let slab_size = Self::resolved_slab_size(&initiate);
        let offset_aware = initiate.contains_key(rise_key::upload::SLAB_SIZE);
        let reconnect_deadline = request.awaits_network.then(|| {
            RiseClock::monotonic()
                + Duration::from_millis(u64::try_from(Self::upload_session_window_ms()).unwrap_or(0))
        });

        on_progress(RiseChunkedUploadTransferProgress::new(0.0, 0, total_bytes));
        let pipeline_depth = if offset_aware {
            Self::preferred_pipeline_depth(self.configuration.premium)
        } else {
            rise_upload::PIPELINE_SERIAL_DEPTH as i64
        };
        let progress = Arc::new(RisePipelinedUploadProgress::new(
            total_bytes,
            Arc::clone(&on_progress),
        ));
        let mut ticker = RiseUploadProgressTicker::start(Arc::clone(&progress));
        let slab = RiseChunkedUploadSlabContext {
            upload_id: &upload_id,
            total_bytes,
            offset_aware,
            pipeline_depth,
            reconnect_deadline,
        };
        {
            let mut group = FuturesUnordered::new();
            let mut next_offset: i64 = 0;
            let mut in_flight: i64 = 0;

            while in_flight < pipeline_depth
                && next_offset < total_bytes
                && !Self::holds_final_slab(next_offset, slab_size, total_bytes, in_flight)
            {
                let (slab_offset, data) =
                    Self::read_slab(&source, next_offset, slab_size, total_bytes).await?;
                let end_offset = slab_offset + Self::byte_count(&data);
                progress.register(slab_offset, Self::byte_count(&data));
                group.push(self.send_registered_slab(&slab, &progress, slab_offset, data));
                next_offset = end_offset;
                in_flight += 1;
            }

            while let Some(finished) = group.next().await {
                finished?;
                in_flight -= 1;
                if next_offset >= total_bytes
                    || Self::holds_final_slab(next_offset, slab_size, total_bytes, in_flight)
                {
                    continue;
                }
                (self.configuration.validate_context)().await?;
                let (slab_offset, data) =
                    Self::read_slab(&source, next_offset, slab_size, total_bytes).await?;
                let end_offset = slab_offset + Self::byte_count(&data);
                progress.register(slab_offset, Self::byte_count(&data));
                group.push(self.send_registered_slab(&slab, &progress, slab_offset, data));
                next_offset = end_offset;
                in_flight += 1;
            }
        }

        ticker.cancel();
        let mut completion: Option<BTreeMap<String, RiseJSONValue>> = None;
        let mut completion_error: Option<anyhow::Error> = None;
        let mut attempt: i64 = 0;
        let mut offline_polls: u32 = 0;
        while attempt < (rise_upload::RETRY_COMPLETE_MAXIMUM_ATTEMPTS as i64) {
            match self
                .perform_json_request(
                    &["file", "upload", "complete"],
                    "POST",
                    &RiseJSONValue::Object(BTreeMap::new()),
                    &[("Upload-Id", upload_id.as_str())],
                )
                .await
            {
                Ok(object) => {
                    completion = Some(object);
                    break;
                }
                Err(error) => {
                    if let Some(reconnect_deadline) = reconnect_deadline
                        && Self::awaits_reconnect(&error, reconnect_deadline, &mut offline_polls)
                            .await?
                    {
                        completion_error = Some(error);
                        continue;
                    }
                    completion_error = Some(error);
                    if attempt >= (rise_upload::RETRY_COMPLETE_MAXIMUM_ATTEMPTS as i64) - 1 {
                        break;
                    }
                    RiseRuntimeTimer::sleep(Self::canon_delay(
                        &rise_upload::RETRY_COMPLETE_DELAYS_MS,
                        attempt,
                    ))
                    .await;
                    attempt += 1;
                }
            }
        }
        let Some(mut completion) = completion else {
            return Err(completion_error.unwrap_or_else(|| {
                RiseChunkedUploadError::InvalidResponse(
                    "Upload completion was not acknowledged".to_owned(),
                )
                .into()
            }));
        };
        if Self::normalized_string(completion.get(rise_key::file::ASSET_ID)).is_none() {
            completion.insert(
                rise_key::file::ASSET_ID.to_owned(),
                RiseJSONValue::String(upload_id.clone()),
            );
        }
        let result = Self::result(completion)?;
        on_progress(RiseChunkedUploadTransferProgress::new(1.0, total_bytes, total_bytes));
        session.completed_successfully = true;
        Ok(result)
    }

    async fn read_slab(
        source: &Arc<RiseUploadFileSource>,
        next_offset: i64,
        slab_size: i64,
        total_bytes: i64,
    ) -> anyhow::Result<(i64, Vec<u8>)> {
        let length = slab_size.min(total_bytes - next_offset);
        let slab_offset = next_offset;
        let reader = Arc::clone(source);
        let data =
            RiseAccountRuntime::current().spawn_blocking(move || reader.read(slab_offset, length)).await??;
        if data.is_empty() {
            return Err(RiseChunkedUploadError::InvalidResponse(
                "The file changed while it was being uploaded".to_owned(),
            )
            .into());
        }
        Ok((slab_offset, data))
    }

    async fn send_registered_slab(
        &self,
        slab: &RiseChunkedUploadSlabContext<'_>,
        progress: &Arc<RisePipelinedUploadProgress>,
        slab_offset: i64,
        data: Vec<u8>,
    ) -> anyhow::Result<()> {
        let end_offset = slab_offset + Self::byte_count(&data);
        let reporter = Arc::clone(progress);
        let on_progress: RiseChunkedUploadProgressHandler =
            Arc::new(move |transfer| reporter.report(slab_offset, transfer));
        self.perform_slab_request(
            slab,
            slab_offset,
            end_offset >= slab.total_bytes,
            data,
            on_progress,
        )
        .await?;
        progress.mark_acknowledged(slab_offset);
        Ok(())
    }

    fn byte_count(data: &[u8]) -> i64 {
        i64::try_from(data.len()).unwrap_or(i64::MAX)
    }

    async fn video_thumbnail_base64(url: &Path) -> Option<String> {
        let url = url.to_path_buf();
        RiseAccountRuntime::current().spawn_blocking(move || {
            let maximum = Self::maximum_inline_video_thumbnail_bytes();
            let byte_count = i64::try_from(std::fs::metadata(&url).ok()?.len()).ok()?;
            if byte_count <= 0 || byte_count > maximum {
                return None;
            }
            let data = std::fs::read(&url).ok()?;
            if data.is_empty() || Self::byte_count(&data) > maximum {
                return None;
            }
            let encoded = base64_encoded_string(&data);
            let chars = i64::try_from(character_count(&encoded)).unwrap_or(i64::MAX);
            if chars > Self::maximum_inline_video_thumbnail_base64_chars() {
                return None;
            }
            Some(encoded)
        })
        .await
        .ok()
        .flatten()
    }

    async fn best_effort_abort(upload_id: String, configuration: RiseChunkedUploadConfiguration) {
        if upload_id.is_empty() {
            return;
        }
        let mut url = configuration.base_url.clone();
        for component in ["file", "upload", "abort"] {
            url = url.appending_path_component(component);
        }
        if url.scheme().is_none() || url.host().is_none() {
            return;
        }

        let mut request = URLRequest::new(url);
        request.http_method = "POST".to_owned();
        for (key, value) in &configuration.default_headers {
            request.set_value(value, key);
        }
        request.set_value("application/json", "Accept");
        request.set_value(&upload_id, "Upload-Id");
        let Ok(request) = (configuration.authorize_cleanup)(request).await else {
            return;
        };
        let _ = URLSession::shared().data_for(request).await;
    }

    pub fn resolved_slab_size(metadata: &BTreeMap<String, RiseJSONValue>) -> i64 {
        Self::resolved_slab_size_value(
            metadata
                .get(rise_key::upload::SLAB_SIZE)
                .or_else(|| metadata.get(rise_key::upload::CHUNK_SIZE)),
        )
    }

    pub fn resolved_slab_size_value(value: Option<&RiseJSONValue>) -> i64 {
        let value = Self::integer(value).unwrap_or(Self::DEFAULT_SLAB_SIZE);
        value.clamp(Self::MINIMUM_SLAB_SIZE, Self::MAXIMUM_SLAB_SIZE)
    }

    pub fn overall_progress(completed_bytes: i64, slab_bytes_sent: i64, total_bytes: i64) -> f64 {
        if total_bytes <= 0 {
            return 0.0;
        }
        let sent = total_bytes.min(completed_bytes.max(0) + slab_bytes_sent.max(0));
        rise_upload::PROGRESS_CEILING_FRACTION.min(sent as f64 / total_bytes as f64)
    }

    pub fn should_retry_slab(
        offset_aware: bool,
        attempt: i64,
        status_code: Option<i64>,
        url_error_code: Option<URLErrorCode>,
    ) -> bool {
        if !offset_aware || attempt >= (rise_upload::RETRY_SLAB_MAXIMUM_ATTEMPTS as i64) - 1 {
            return false;
        }
        if let Some(status_code) = status_code {
            return ((rise_upload::RETRY_RETRYABLE_HTTP_STATUS_FIRST as i64)
                ..=(rise_upload::RETRY_RETRYABLE_HTTP_STATUS_LAST as i64))
                .contains(&status_code);
        }
        let Some(url_error_code) = url_error_code else {
            return false;
        };
        [
            URLErrorCode::NetworkConnectionLost,
            URLErrorCode::TimedOut,
            URLErrorCode::CannotConnectToHost,
            URLErrorCode::NotConnectedToInternet,
            URLErrorCode::DnsLookupFailed,
            URLErrorCode::CannotFindHost,
        ]
        .contains(&url_error_code)
    }

    async fn initiate_session(
        &self,
        body: RiseJSONValue,
        awaits_network: bool,
    ) -> anyhow::Result<BTreeMap<String, RiseJSONValue>> {
        let mut offline_polls: u32 = 0;
        let mut bounce_deadline: Option<Duration> = None;
        loop {
            let error = match self
                .perform_json_request(&["file", "upload", "initiate"], "POST", &body, &[])
                .await
            {
                Ok(object) => return Ok(object),
                Err(error) => error,
            };
            if awaits_network
                && Self::url_error_code(&error)
                    .is_some_and(|code| Self::CONNECTION_LOSS_CODES.contains(&code))
            {
                Self::wait_for_usable_network_path().await;
                RiseRuntimeTimer::sleep(Self::offline_pause(offline_polls)).await;
                offline_polls += 1;
                continue;
            }
            if awaits_network
                && let Some(RiseChunkedUploadError::HttpStatus(status, _)) =
                    error.downcast_ref::<RiseChunkedUploadError>()
                && Self::SERVER_BOUNCE_STATUSES.contains(status)
            {
                let deadline = *bounce_deadline.get_or_insert_with(|| {
                    RiseClock::monotonic()
                        + Duration::from_millis(
                            u64::try_from(Self::upload_session_window_ms()).unwrap_or(0),
                        )
                });
                if !Self::awaits_reconnect(&error, deadline, &mut offline_polls).await? {
                    return Err(error);
                }
                continue;
            }
            return Err(error);
        }
    }

    async fn wait_for_usable_network_path() {
        let (usable, mut updates) = tokio::sync::mpsc::unbounded_channel::<bool>();
        let watches = watch(move |event| {
            if let SystemActivityEvent::NetworkPath { satisfied } = event {
                let _ = usable.send(satisfied);
            }
        });
        let path_unwatched = watches.gaps().iter().any(|gap| {
            matches!(
                gap,
                SystemActivityUnavailable::BranchNotWritten {
                    what: SystemActivityKind::NetworkPath,
                    ..
                } | SystemActivityUnavailable::SystemRefused {
                    what: SystemActivityKind::NetworkPath,
                    ..
                }
            )
        });
        if path_unwatched {
            return;
        }
        while let Some(satisfied) = updates.recv().await {
            if satisfied {
                return;
            }
        }
    }

    /// true when a dead connection can wait for a usable path and retry from offset
    /// ladders last about a second, a dropped network outlasts it
    /// 502 to 504 while a service restarts waits
    async fn awaits_reconnect(
        error: &anyhow::Error,
        deadline: Duration,
        polls: &mut u32,
    ) -> anyhow::Result<bool> {
        if RiseClock::monotonic() >= deadline {
            return Ok(false);
        }
        if let Some(code) = Self::url_error_code(error)
            && Self::CONNECTION_LOSS_CODES.contains(&code)
        {
            Self::wait_for_usable_network_path().await;
        } else if !matches!(
            error.downcast_ref::<RiseChunkedUploadError>(),
            Some(RiseChunkedUploadError::HttpStatus(status, _))
                if Self::SERVER_BOUNCE_STATUSES.contains(status)
        ) {
            return Ok(false);
        }
        RiseRuntimeTimer::sleep(Self::offline_pause(*polls)).await;
        *polls += 1;
        Ok(RiseClock::monotonic() < deadline)
    }

    fn offline_pause(polls: u32) -> Duration {
        let delays = &rise_upload::RETRY_SLAB_DELAYS_MS;
        let last = i64::try_from(delays.len()).unwrap_or(1) - 1;
        let base = Self::canon_delay(delays, last);
        base * (1_u32 << polls.min(Self::MAXIMUM_OFFLINE_PAUSE_DOUBLINGS))
    }

    fn url_error_code(error: &anyhow::Error) -> Option<URLErrorCode> {
        error.downcast_ref::<URLError>().map(|error| error.code)
    }

    async fn perform_json_request(
        &self,
        path: &[&str],
        method: &str,
        body: &RiseJSONValue,
        extra_headers: &[(&str, &str)],
    ) -> anyhow::Result<BTreeMap<String, RiseJSONValue>> {
        let mut authorization_refreshes: i64 = 0;
        loop {
            let mut request = self.make_request(path, method).await?;
            for (key, value) in extra_headers {
                request.set_value(value, key);
            }
            request.set_value("application/json", "Accept");
            request.set_value("application/json; charset=utf-8", "Content-Type");
            request.http_body = Some(body.data(false)?);
            let request = (self.configuration.authorize)(request).await?;
            (self.configuration.validate_context)().await?;
            let (data, response) = URLSession::shared().data_for(request).await?;
            (self.configuration.validate_context)().await?;
            if response.status_code == 401
                && authorization_refreshes < (rise_upload::RETRY_AUTHORIZATION_REFRESHES_PER_SLAB as i64)
            {
                authorization_refreshes += 1;
                (self.configuration.refresh_authorization)().await?;
                continue;
            }
            if !(200..300).contains(&response.status_code) {
                return Err(RiseChunkedUploadError::HttpStatus(
                    response.status_code,
                    String::from_utf8(data).unwrap_or_default(),
                )
                .into());
            }
            return Self::response_object(&data);
        }
    }

    async fn send_slab(
        request: URLRequest,
        delegate: Arc<RiseUploadProgressDelegate>,
    ) -> Result<(Vec<u8>, HTTPURLResponse), URLError> {
        let session_delegate: Arc<dyn URLSessionTaskDelegate> = delegate.clone();
        let send = URLSession::shared().data_for_with_delegate(request, session_delegate);
        let stall = async {
            loop {
                RiseRuntimeTimer::sleep(Self::SLAB_STALL_CHECK_INTERVAL).await;
                if delegate.send_stalled(Self::SLAB_SEND_STALL_TIMEOUT) {
                    return URLError::new(URLErrorCode::TimedOut);
                }
            }
        };
        tokio::select! {
            result = send => result,
            error = stall => Err(error),
        }
    }

    async fn perform_slab_request(
        &self,
        slab: &RiseChunkedUploadSlabContext<'_>,
        offset: i64,
        is_last: bool,
        body: Vec<u8>,
        on_progress: RiseChunkedUploadProgressHandler,
    ) -> anyhow::Result<()> {
        let mut attempt: i64 = 0;
        let mut authorization_refreshes: i64 = 0;
        let mut offline_polls: u32 = 0;
        loop {
            let error = match self
                .send_slab_attempt(slab, offset, is_last, &body, &on_progress)
                .await
            {
                Ok((status, data)) => {
                    if status == 401
                        && authorization_refreshes
                            < (rise_upload::RETRY_AUTHORIZATION_REFRESHES_PER_SLAB as i64)
                    {
                        authorization_refreshes += 1;
                        match (self.configuration.refresh_authorization)().await {
                            Ok(()) => continue,
                            Err(error) => error,
                        }
                    } else if Self::should_retry_slab(slab.offset_aware, attempt, Some(status), None)
                    {
                        attempt += 1;
                        RiseRuntimeTimer::sleep(Self::retry_delay(attempt)).await;
                        continue;
                    } else {
                        match Self::acknowledge_slab(slab, offset, &body, status, data) {
                            Ok(()) => return Ok(()),
                            Err(error) => error,
                        }
                    }
                }
                Err(error) => error,
            };
            let url_code = Self::url_error_code(&error);
            if slab.offset_aware
                && let Some(reconnect_deadline) = slab.reconnect_deadline
                && Self::awaits_reconnect(&error, reconnect_deadline, &mut offline_polls).await?
            {
                continue;
            }
            if !Self::should_retry_slab(slab.offset_aware, attempt, None, url_code) {
                return Err(error);
            }
            attempt += 1;
            RiseRuntimeTimer::sleep(Self::retry_delay(attempt)).await;
        }
    }

    async fn send_slab_attempt(
        &self,
        slab: &RiseChunkedUploadSlabContext<'_>,
        offset: i64,
        is_last: bool,
        body: &[u8],
        on_progress: &RiseChunkedUploadProgressHandler,
    ) -> anyhow::Result<(i64, Vec<u8>)> {
        let mut request = self.make_request(&["file", "upload", "slab"], "PUT").await?;
        request.http_body = Some(body.to_vec());
        request.timeout_interval = Self::slab_request_timeout();
        request.set_value("application/octet-stream", "Content-Type");
        request.set_value(slab.upload_id, "Upload-Id");
        request.set_value(if is_last { "true" } else { "false" }, "Last-Slab");
        if slab.offset_aware {
            request.set_value(&offset.to_string(), "Slab-Offset");
        }
        if slab.pipeline_depth > 1 {
            request.set_value(&slab.pipeline_depth.to_string(), "Upload-Pipeline-Depth");
        }
        let request = (self.configuration.authorize)(request).await?;
        (self.configuration.validate_context)().await?;
        let delegate = Arc::new(RiseUploadProgressDelegate::new(
            offset,
            slab.total_bytes,
            Arc::clone(on_progress),
        ));
        let (data, response) = Self::send_slab(request, delegate).await?;
        (self.configuration.validate_context)().await?;
        Ok((response.status_code, data))
    }

    fn acknowledge_slab(
        slab: &RiseChunkedUploadSlabContext<'_>,
        offset: i64,
        body: &[u8],
        status: i64,
        data: Vec<u8>,
    ) -> anyhow::Result<()> {
        if !(200..300).contains(&status) {
            return Err(RiseChunkedUploadError::HttpStatus(
                status,
                String::from_utf8(data).unwrap_or_default(),
            )
            .into());
        }
        let payload = Self::response_object(&data)?;
        if slab.offset_aware {
            let expected = offset + Self::byte_count(body);
            if Self::integer(payload.get(rise_key::bot::NEXT_OFFSET)) != Some(expected) {
                return Err(RiseChunkedUploadError::InvalidResponse(
                    "Upload acknowledgement did not match the sent byte range".to_owned(),
                )
                .into());
            }
        }
        Ok(())
    }
    async fn make_request(&self, path: &[&str], method: &str) -> anyhow::Result<URLRequest> {
        (self.configuration.validate_context)().await?;
        let mut url = self.configuration.base_url.clone();
        for component in path {
            url = url.appending_path_component(component);
        }
        if url.scheme().is_none() || url.host().is_none() {
            return Err(RiseChunkedUploadError::InvalidEndpoint.into());
        }
        let mut request = URLRequest::new(url);
        request.http_method = method.to_owned();
        for (key, value) in &self.configuration.default_headers {
            request.set_value(value, key);
        }
        Ok(request)
    }

    fn retry_delay(attempt: i64) -> Duration {
        Self::canon_delay(&rise_upload::RETRY_SLAB_DELAYS_MS, attempt - 1)
    }

    fn canon_delay(table: &[u64], step: i64) -> Duration {
        let Some(last) = table.len().checked_sub(1) else {
            return Duration::ZERO;
        };
        let index = usize::try_from(step.max(0)).unwrap_or(0).min(last);
        Duration::from_millis(table[index])
    }

    fn response_object(data: &[u8]) -> anyhow::Result<BTreeMap<String, RiseJSONValue>> {
        let root = RiseJSONValue::decode(data)?;
        let Some(root) = root.object_value() else {
            return Err(RiseChunkedUploadError::InvalidResponse(
                "Upload response was not a JSON object".to_owned(),
            )
            .into());
        };
        if matches!(root.get(rise_key::file::SUCCESS), Some(RiseJSONValue::Bool(false))) {
            let message = Self::normalized_string(
                root.get(rise_key::file::MESSAGE)
                    .or_else(|| root.get(rise_key::file::ERROR)),
            );
            return Err(RiseChunkedUploadError::InvalidResponse(
                message.unwrap_or_else(|| "Upload failed".to_owned()),
            )
            .into());
        }
        Ok(root
            .get(rise_key::bot::DATA)
            .and_then(RiseJSONValue::object_value)
            .unwrap_or(root)
            .clone())
    }

    fn result(object: BTreeMap<String, RiseJSONValue>) -> anyhow::Result<RiseChunkedUploadResult> {
        Ok(RiseChunkedUploadResult {
            asset_id: Self::required_string(&object, &[rise_key::file::ASSET_ID])?,
            file_url: Self::required_string(&object, &[rise_key::file::FILE_URL, rise_key::file::URL])?,
            file_key: Self::required_string(&object, &[rise_key::file::FILE_KEY])?,
            access_url: Self::normalized_string(object.get(rise_key::file::ACCESS_URL)),
            thumbnail_url: Self::normalized_string(object.get(rise_key::file::THUMBNAIL_URL)),
            file_id: Self::normalized_string(object.get(rise_key::file::FILE_ID)),
            mime_type: Self::required_string(&object, &[rise_key::file::MIME_TYPE])?,
            file_size: Self::integer(object.get(rise_key::file::FILE_SIZE)),
            width: Self::integer(object.get(rise_key::file::WIDTH)),
            height: Self::integer(object.get(rise_key::file::HEIGHT)),
            duration: Self::integer(object.get(rise_key::file::DURATION)),
            metadata: object,
        })
    }

    fn required_string(
        object: &BTreeMap<String, RiseJSONValue>,
        keys: &[&str],
    ) -> anyhow::Result<String> {
        for key in keys {
            if let Some(value) = Self::normalized_string(object.get(*key)) {
                return Ok(value);
            }
        }
        Err(RiseChunkedUploadError::InvalidResponse(format!(
            "Upload response is missing {keys:?}"
        ))
        .into())
    }

    fn normalized_string(value: Option<&RiseJSONValue>) -> Option<String> {
        let Some(RiseJSONValue::String(raw)) = value else {
            return None;
        };
        let value = trimming_whitespaces_and_newlines(raw);
        (!value.is_empty()).then(|| value.to_owned())
    }

    fn integer(value: Option<&RiseJSONValue>) -> Option<i64> {
        match value {
            Some(RiseJSONValue::Integer(value)) => Some(*value),
            Some(RiseJSONValue::UnsignedInteger(value)) => i64::try_from(*value).ok(),
            Some(RiseJSONValue::Number(value)) => value.is_finite().then_some(*value as i64),
            Some(RiseJSONValue::String(value)) => int_from_string(value),
            _ => None,
        }
    }
}

/// args every slab of one upload shares, passed once
struct RiseChunkedUploadSlabContext<'upload> {
    upload_id: &'upload str,
    total_bytes: i64,
    offset_aware: bool,
    pipeline_depth: i64,
    reconnect_deadline: Option<Duration>,
}

struct RiseChunkedUploadSessionGuard {
    upload_id: String,
    on_session_event: RiseChunkedUploadSessionEventHandler,
    configuration: RiseChunkedUploadConfiguration,
    completed_successfully: bool,
}

impl Drop for RiseChunkedUploadSessionGuard {
    fn drop(&mut self) {
        let runtime = RiseAccountRuntime::current();
        runtime.spawn((self.on_session_event)(RiseChunkedUploadSessionEvent::Finished {
            upload_id: self.upload_id.clone(),
        }));
        if !self.completed_successfully {
            runtime.spawn(RiseChunkedUploadClient::best_effort_abort(
                self.upload_id.clone(),
                self.configuration.clone(),
            ));
        }
    }
}

struct RiseUploadFileSource {
    total_bytes: i64,
    handle: Mutex<Option<File>>,
}

impl RiseUploadFileSource {
    fn new(file_url: &Path) -> std::io::Result<Self> {
        let total_bytes = i64::try_from(std::fs::metadata(file_url)?.len()).unwrap_or(0);
        let handle = File::open(file_url)?;
        Ok(Self {
            total_bytes,
            handle: Mutex::new(Some(handle)),
        })
    }

    fn read(&self, offset: i64, count: i64) -> std::io::Result<Vec<u8>> {
        let mut handle = self.handle.lock();
        let Some(file) = handle.as_mut() else {
            return Ok(Vec::new());
        };
        file.seek(SeekFrom::Start(u64::try_from(offset).unwrap_or(0)))?;
        let mut data = Vec::new();
        file.take(u64::try_from(count).unwrap_or(0))
            .read_to_end(&mut data)?;
        Ok(data)
    }

    fn close(&self) {
        self.handle.lock().take();
    }
}

struct RiseUploadFileSourceClose(Arc<RiseUploadFileSource>);

impl Drop for RiseUploadFileSourceClose {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// merges progress from several http2 slab requests
/// doesnt count a retried byte range twice or jump to a later slab offset
struct RisePipelinedUploadProgress {
    total_bytes: i64,
    callback: RiseChunkedUploadProgressHandler,
    state: Mutex<RisePipelinedUploadProgressState>,
}

struct RisePipelinedUploadProgressState {
    lengths_by_offset: HashMap<i64, i64>,
    sent_by_offset: HashMap<i64, i64>,
    acknowledged_bytes: i64,
    in_flight_credit: i64,
    last_reported_bytes: i64,
    last_report: Duration,
    estimator: RiseUploadByteEstimator,
}

struct RiseUploadProgressTicker {
    task: Option<tokio::task::JoinHandle<()>>,
}

impl RiseUploadProgressTicker {
    fn start(progress: Arc<RisePipelinedUploadProgress>) -> Self {
        let task = RiseAccountRuntime::current().spawn(async move {
            loop {
                tokio::time::sleep(RisePipelinedUploadProgress::minimum_report_interval()).await;
                progress.tick();
            }
        });
        Self { task: Some(task) }
    }

    fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for RiseUploadProgressTicker {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl RisePipelinedUploadProgress {
    fn minimum_report_interval() -> Duration {
        Duration::from_millis(rise_upload::PROGRESS_MINIMUM_INTERVAL_MS)
    }

    fn new(total_bytes: i64, callback: RiseChunkedUploadProgressHandler) -> Self {
        Self {
            total_bytes: total_bytes.max(0),
            callback,
            state: Mutex::new(RisePipelinedUploadProgressState {
                lengths_by_offset: HashMap::new(),
                sent_by_offset: HashMap::new(),
                acknowledged_bytes: 0,
                in_flight_credit: 0,
                last_reported_bytes: 0,
                last_report: RiseClock::monotonic(),
                estimator: RiseUploadByteEstimator::new(
                    RiseClock::now().time_interval_since_reference_date(),
                ),
            }),
        }
    }

    /// moves the estimate on while a slab is on the wire and no socket callback comes
    fn tick(&self) {
        let update = {
            let mut state = self.state.lock();
            self.make_update_locked(&mut state, false)
        };
        if let Some(update) = update {
            (self.callback)(update);
        }
    }

    fn register(&self, offset: i64, length: i64) {
        let mut state = self.state.lock();
        state.lengths_by_offset.insert(offset, length.max(0));
        state.sent_by_offset.insert(offset, 0);
        state.in_flight_credit = state.in_flight_credit.max(length.max(0));
    }

    fn report(&self, offset: i64, transfer: RiseChunkedUploadTransferProgress) {
        let update = {
            let mut state = self.state.lock();
            let Some(&length) = state.lengths_by_offset.get(&offset) else {
                return;
            };
            let local_bytes = length.min((transfer.completed_bytes - offset).max(0));
            let sent = state.sent_by_offset.entry(offset).or_insert(0);
            *sent = (*sent).max(local_bytes);
            self.make_update_locked(&mut state, false)
        };
        if let Some(update) = update {
            (self.callback)(update);
        }
    }

    fn mark_acknowledged(&self, offset: i64) {
        let update = {
            let mut state = self.state.lock();
            let Some(length) = state.lengths_by_offset.remove(&offset) else {
                return;
            };
            state.sent_by_offset.remove(&offset);
            state.acknowledged_bytes = self.total_bytes.min(state.acknowledged_bytes + length);
            let total_acknowledged = state.acknowledged_bytes;
            state.estimator.acknowledge(
                total_acknowledged,
                RiseClock::now().time_interval_since_reference_date(),
            );
            self.make_update_locked(&mut state, true)
        };
        if let Some(update) = update {
            (self.callback)(update);
        }
    }

    fn make_update_locked(
        &self,
        state: &mut RisePipelinedUploadProgressState,
        force: bool,
    ) -> Option<RiseChunkedUploadTransferProgress> {
        let in_flight_bytes = state
            .in_flight_credit
            .min(state.sent_by_offset.values().sum::<i64>());
        let now = RiseClock::monotonic();
        let sent_bytes = state.acknowledged_bytes + in_flight_bytes;
        let completed_bytes = self.total_bytes.min(state.estimator.estimate(
            sent_bytes,
            RiseClock::now().time_interval_since_reference_date(),
        ));
        let minimum_byte_step =
            1_i64.max(self.total_bytes / (rise_upload::PROGRESS_MINIMUM_BYTE_STEP_DIVISOR as i64));
        let should_report = force
            || (completed_bytes >= self.total_bytes && state.last_reported_bytes < self.total_bytes)
            || completed_bytes - state.last_reported_bytes >= minimum_byte_step
            || now.saturating_sub(state.last_report) >= Self::minimum_report_interval();
        if !should_report || completed_bytes < state.last_reported_bytes {
            return None;
        }
        state.last_reported_bytes = completed_bytes;
        state.last_report = now;
        Some(RiseChunkedUploadTransferProgress::new(
            RiseChunkedUploadClient::overall_progress(completed_bytes, 0, self.total_bytes),
            completed_bytes,
            self.total_bytes,
        ))
    }
}

struct RiseUploadProgressDelegate {
    completed_bytes: i64,
    total_bytes: i64,
    callback: RiseChunkedUploadProgressHandler,
    state: Mutex<RiseUploadProgressDelegateState>,
}

struct RiseUploadProgressDelegateState {
    last_fraction: f64,
    last_completed_bytes: i64,
    last_report: Duration,
    last_body_activity: Duration,
    body_finished: bool,
}

impl RiseUploadProgressDelegate {
    fn new(
        completed_bytes: i64,
        total_bytes: i64,
        callback: RiseChunkedUploadProgressHandler,
    ) -> Self {
        let now = RiseClock::monotonic();
        Self {
            completed_bytes,
            total_bytes,
            callback,
            state: Mutex::new(RiseUploadProgressDelegateState {
                last_fraction: RiseChunkedUploadClient::overall_progress(
                    completed_bytes,
                    0,
                    total_bytes,
                ),
                last_completed_bytes: total_bytes.min(completed_bytes.max(0)),
                last_report: now,
                last_body_activity: now,
                body_finished: false,
            }),
        }
    }

    fn send_stalled(&self, longer_than: Duration) -> bool {
        let now = RiseClock::monotonic();
        let state = self.state.lock();
        !state.body_finished && now.saturating_sub(state.last_body_activity) > longer_than
    }
}

impl URLSessionTaskDelegate for RiseUploadProgressDelegate {
    fn url_session(
        &self,
        _bytes_sent: i64,
        total_bytes_sent: i64,
        total_bytes_expected_to_send: i64,
    ) {
        let fraction = RiseChunkedUploadClient::overall_progress(
            self.completed_bytes,
            total_bytes_sent,
            self.total_bytes,
        );
        let current_completed_bytes = self
            .total_bytes
            .min((self.completed_bytes + total_bytes_sent).max(0));
        let now = RiseClock::monotonic();
        let should_report = {
            let mut state = self.state.lock();
            state.last_body_activity = now;
            if total_bytes_expected_to_send > 0 && total_bytes_sent >= total_bytes_expected_to_send {
                state.body_finished = true;
            }
            let reached_end = current_completed_bytes >= self.total_bytes
                && state.last_completed_bytes < self.total_bytes;
            let ceiling = rise_upload::PROGRESS_CEILING_FRACTION;
            let should_report = reached_end
                || (fraction >= ceiling && state.last_fraction < ceiling)
                || fraction - state.last_fraction >= rise_upload::PROGRESS_EPSILON_FRACTION
                || now.saturating_sub(state.last_report)
                    >= RisePipelinedUploadProgress::minimum_report_interval();
            if should_report {
                state.last_fraction = fraction;
                state.last_completed_bytes = current_completed_bytes;
                state.last_report = now;
            }
            should_report
        };
        if !should_report {
            return;
        }
        (self.callback)(RiseChunkedUploadTransferProgress::new(
            fraction,
            current_completed_bytes,
            self.total_bytes,
        ));
    }
}
