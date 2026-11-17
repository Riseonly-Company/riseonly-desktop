use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{Either, select};
use gpui::http_client::http::{HeaderName, HeaderValue, Method, Request, Uri};
use gpui::{App, AsyncApp, Task};
use rise_canon::rise_key::{bot, file, upload as upload_key};
use rise_canon::rise_upload::{self, ErrorCode as RiseUploadErrorCode};
use rise_canon::rise_wire_canon::key as wire_key;
use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_upload_byte_estimator::RiseUploadByteEstimator;
use rise_engine::rise_upload::rise_url_session::{
    HTTPURLResponse, URLError, URLErrorCode, URLRequest, URLSession, URLSessionTaskDelegate,
};
use rise_i18n::tr;
use rise_platform::system_activity::{
    SystemActivityEvent, SystemActivityKind, SystemActivityUnavailable, watch,
};
use serde_json::{Map, Value};

use crate::core::desktop_platform::rise_app_quit_drain::QuitDrain;
use crate::core::lib::canon_errors::RiseFailure;
use crate::core::lib::canon_failure_relay::RiseServerRefusalRelay;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_entry::{SaiHttpTraceEntry, SaiTraceTransport};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_recorder::SaiHttpTraceRecorder;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::SaiHttpTraceStore;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::{
    SaiFetchHttpConfiguration, SaiFetchRequestInterceptor,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_outbound_cancellation::SaiWsOutboundCancelledError;
use crate::core::lib::sai::sai_auth_hooks::{sai_auth_hooks, sai_auth_refresh_access_token};
use crate::modules::share_extension::shared::rise_upload_context_rules::RiseLimitsUploadContextRulesExt;

const HEADER_ACCEPT: &str = "Accept";
const HEADER_CONTENT_TYPE: &str = "Content-Type";
const HEADER_UPLOAD_ID: &str = "Upload-Id";
const HEADER_LAST_SLAB: &str = "Last-Slab";
const HEADER_SLAB_OFFSET: &str = "Slab-Offset";
const ACCEPT_JSON: &str = "application/json";
const CONTENT_TYPE_JSON: &str = "application/json; charset=utf-8";
const CONTENT_TYPE_OCTET_STREAM: &str = "application/octet-stream";
const METHOD_POST: &str = "POST";
const METHOD_PUT: &str = "PUT";
const PATH_INITIATE: &str = "file/upload/initiate";
const PATH_SLAB: &str = "file/upload/slab";
const PATH_COMPLETE: &str = "file/upload/complete";
const PATH_ABORT: &str = "file/upload/abort";
const FETCH_ID_INITIATE: &str = "file.upload.initiate";
const FETCH_ID_COMPLETE: &str = "file.upload.complete";
const FETCH_ID_ABORT: &str = "file.upload.abort";
const UPLOAD_SESSION_TTL: &str = "upload_session_ttl";
const HTTP_UNAUTHORIZED: i64 = 401;
const FETCH_ID_SLAB: &str = "file.upload.slab";
const TRACE_TAG_FILE_UPLOAD: &str = "FileUpload";
const TRACE_METHOD_FALLBACK: &str = "HTTP";
const TRACE_RESPONSE_PREVIEW_MAX_CHARS: usize = 32_000;

/// overall upload fraction, on the main thread
pub type SaiWsUploadOnProgress = Rc<dyn Fn(f64, &mut App)>;
/// done and total bytes, on the main thread
pub type SaiWsUploadOnByteProgress = Rc<dyn Fn(i64, i64, &mut App)>;

struct SaiWsUploadProgressReporter {
    last_fraction: f64,
    last_completed_bytes: i64,
    sent_bytes: i64,
    total_bytes: i64,
    estimator: RiseUploadByteEstimator,
    callback: Option<SaiWsUploadOnProgress>,
    byte_callback: Option<SaiWsUploadOnByteProgress>,
}

impl SaiWsUploadProgressReporter {
    fn new(callback: Option<SaiWsUploadOnProgress>, byte_callback: Option<SaiWsUploadOnByteProgress>) -> Self {
        Self {
            last_fraction: 0.0,
            last_completed_bytes: -1,
            sent_bytes: 0,
            total_bytes: 0,
            estimator: RiseUploadByteEstimator::new(RiseClock::now().time_interval_since_reference_date()),
            callback,
            byte_callback,
        }
    }

    /// socket takes a whole slab at once so sent bytes jump in slabs
    /// the estimate fills the gap at the measured rate and
    /// stays under the ceiling until the server confirms
    fn sent(&mut self, bytes: i64, total_bytes: i64, cx: &mut App) {
        self.sent_bytes = self.sent_bytes.max(bytes);
        self.total_bytes = total_bytes;
        self.emit_estimate(cx);
    }

    fn acknowledge(&mut self, bytes: i64, total_bytes: i64, cx: &mut App) {
        self.estimator
            .acknowledge(bytes, RiseClock::now().time_interval_since_reference_date());
        self.sent(bytes, total_bytes, cx);
    }

    fn tick(&mut self, cx: &mut App) {
        if self.total_bytes <= 0 {
            return;
        }
        self.emit_estimate(cx);
    }

    fn emit_estimate(&mut self, cx: &mut App) {
        let shown = self
            .estimator
            .estimate(self.sent_bytes, RiseClock::now().time_interval_since_reference_date());
        let total_bytes = self.total_bytes;
        self.report(
            SaiWsChunkedFileUpload::overall_progress(shown, 0, total_bytes),
            shown,
            total_bytes,
            cx,
        );
    }

    fn report(&mut self, fraction: f64, completed_bytes: i64, total_bytes: i64, cx: &mut App) {
        let normalized = fraction.clamp(0.0, 1.0);
        let completed = completed_bytes.max(0).min(total_bytes.max(0));
        let fraction_advanced = normalized > self.last_fraction;
        let bytes_advanced = completed > self.last_completed_bytes;
        if !(fraction_advanced || bytes_advanced) {
            return;
        }
        if fraction_advanced {
            self.last_fraction = normalized;
            if let Some(callback) = &self.callback {
                callback(normalized, cx);
            }
        }
        if bytes_advanced {
            self.last_completed_bytes = completed;
            if let Some(byte_callback) = &self.byte_callback {
                byte_callback(completed, total_bytes, cx);
            }
        }
    }
}

struct SaiWsSlabProgressState {
    last_reported_fraction: f64,
    last_reported_at: Duration,
    last_body_activity: Duration,
    body_finished: bool,
}

/// progress delegate on the engine runtime, reports hop to main through the sink
struct SaiWsSlabProgressDelegate {
    completed_bytes: i64,
    total_bytes: i64,
    sink: mpsc::UnboundedSender<(f64, i64, i64)>,
    state: Mutex<SaiWsSlabProgressState>,
}

impl SaiWsSlabProgressDelegate {
    fn minimum_fraction_delta() -> f64 {
        rise_upload::PROGRESS_EPSILON_FRACTION
    }

    fn maximum_silence() -> Duration {
        Duration::from_millis(rise_upload::PROGRESS_MINIMUM_INTERVAL_MS)
    }

    fn new(completed_bytes: i64, total_bytes: i64, sink: mpsc::UnboundedSender<(f64, i64, i64)>) -> Self {
        let now = RiseClock::monotonic();
        Self {
            completed_bytes,
            total_bytes,
            sink,
            state: Mutex::new(SaiWsSlabProgressState {
                last_reported_fraction: SaiWsChunkedFileUpload::overall_progress(completed_bytes, 0, total_bytes),
                last_reported_at: now,
                last_body_activity: now,
                body_finished: false,
            }),
        }
    }

    fn send_stalled(&self, longer_than: Duration) -> bool {
        let now = RiseClock::monotonic();
        match self.state.lock() {
            Ok(state) => !state.body_finished && now.saturating_sub(state.last_body_activity) > longer_than,
            Err(_) => false,
        }
    }
}

impl URLSessionTaskDelegate for SaiWsSlabProgressDelegate {
    fn url_session(&self, _bytes_sent: i64, total_bytes_sent: i64, total_bytes_expected_to_send: i64) {
        let fraction = SaiWsChunkedFileUpload::overall_progress(self.completed_bytes, total_bytes_sent, self.total_bytes);
        let uploaded_bytes = self
            .total_bytes
            .min(self.completed_bytes.max(0).saturating_add(total_bytes_sent.max(0)));
        let now = RiseClock::monotonic();

        let should_report = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            state.last_body_activity = now;
            if total_bytes_expected_to_send > 0 && total_bytes_sent >= total_bytes_expected_to_send {
                state.body_finished = true;
            }
            let ceiling = rise_upload::PROGRESS_CEILING_FRACTION;
            let reached_upload_ceiling = fraction >= ceiling && state.last_reported_fraction < ceiling;
            let should_report = reached_upload_ceiling
                || fraction - state.last_reported_fraction >= Self::minimum_fraction_delta()
                || now.saturating_sub(state.last_reported_at) >= Self::maximum_silence();
            if should_report {
                state.last_reported_fraction = fraction;
                state.last_reported_at = now;
            }
            should_report
        };

        if should_report {
            let _ = self.sink.unbounded_send((fraction, uploaded_bytes, self.total_bytes));
        }
    }
}

struct SaiWsUploadFileSource {
    total_bytes: i64,
    handle: Mutex<Option<File>>,
}

impl SaiWsUploadFileSource {
    fn new(file_url: &Path) -> std::io::Result<Self> {
        let attributes = std::fs::metadata(file_url)?;
        let total_bytes = i64::try_from(attributes.len()).unwrap_or(i64::MAX);
        let handle = File::open(file_url)?;
        Ok(Self {
            total_bytes,
            handle: Mutex::new(Some(handle)),
        })
    }

    fn read(&self, offset: i64, count: usize) -> std::io::Result<Vec<u8>> {
        let mut guard = self
            .handle
            .lock()
            .map_err(|_| std::io::Error::other("upload file source lock poisoned"))?;
        let Some(handle) = guard.as_mut() else {
            return Ok(Vec::new());
        };
        handle.seek(SeekFrom::Start(u64::try_from(offset).unwrap_or(0)))?;
        let mut data = Vec::with_capacity(count);
        handle
            .by_ref()
            .take(u64::try_from(count).unwrap_or(u64::MAX))
            .read_to_end(&mut data)?;
        Ok(data)
    }

    fn close(&self) {
        if let Ok(mut guard) = self.handle.lock() {
            guard.take();
        }
    }
}

struct SaiWsUploadFileSourceCloser(Arc<SaiWsUploadFileSource>);

impl Drop for SaiWsUploadFileSourceCloser {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// debugger row for one upload request, kept only while debug tools are on
struct SaiWsUploadHTTPTrace {
    id: Uuid,
    started_at: Date,
    did_finish: bool,
}

impl SaiWsUploadHTTPTrace {
    fn new(request: &Request<Vec<u8>>, fetch_id: &str, request_body_wire: Option<String>, cx: &mut App) -> Self {
        let trace = Self {
            id: Uuid::new_v4(),
            started_at: RiseClock::now(),
            did_finish: false,
        };
        let method = request.method().as_str();
        let body = request.body();
        let entry = SaiHttpTraceEntry {
            id: trace.id,
            transport: SaiTraceTransport::Http,
            created_at: trace.started_at,
            is_pending: true,
            method: if method.is_empty() { TRACE_METHOD_FALLBACK } else { method }.to_owned(),
            url: request.uri().to_string(),
            fetch_id: Some(fetch_id.to_owned()),
            duration_ms: 0,
            request_headers: SaiHttpTraceRecorder::request_headers(request),
            request_body_wire: request_body_wire.unwrap_or_else(|| {
                SaiHttpTraceRecorder::wire_json_string((!body.is_empty()).then_some(body.as_slice()))
            }),
            response_status: 0,
            response_headers: HashMap::new(),
            response_body_preview: String::new(),
            transport_error: None,
            trace_tags: vec![TRACE_TAG_FILE_UPLOAD.to_owned()],
        };
        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| store.upsert(entry, cx));
        trace
    }

    fn complete(&mut self, data: &[u8], response: Option<HTTPURLResponse>, transport_error: Option<String>, cx: &mut App) {
        if self.did_finish {
            return;
        }
        self.did_finish = true;
        let id = self.id;
        let duration_ms = (RiseClock::now().time_interval_since(&self.started_at) * 1_000.0) as i64;
        let response_status = response.map_or(0, |response| response.status_code);
        let preview = SaiHttpTraceRecorder::truncate_utf8(data, TRACE_RESPONSE_PREVIEW_MAX_CHARS);
        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
            store.merge_http_completion(id, duration_ms, response_status, HashMap::new(), preview, transport_error, cx);
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaiWsChunkedFileUploadError {
    MissingBaseURL,
    InvalidURL(String),
    EmptyFile,
    FileTooLarge { byte_count: i64, maximum_bytes: i64 },
    OffsetMismatch { expected: i64, acknowledged: Option<i64> },
    NotAcknowledged,
    InvalidResponse(String),
    HttpStatus(i64, String),
}

impl SaiWsChunkedFileUploadError {
    /// canon refusal behind this error
    /// exact http codes map directly, a 5xx without its own code becomes server
    /// the retry loop repeats server
    pub fn canon_code(&self) -> RiseUploadErrorCode {
        match self {
            Self::MissingBaseURL | Self::InvalidURL(_) => RiseUploadErrorCode::BadRequest,
            Self::EmptyFile => RiseUploadErrorCode::EmptyFile,
            Self::FileTooLarge { .. } => RiseUploadErrorCode::FileTooLarge,
            Self::OffsetMismatch { .. } => RiseUploadErrorCode::OffsetMismatch,
            Self::NotAcknowledged => RiseUploadErrorCode::NotAcknowledged,
            Self::InvalidResponse(_) => RiseUploadErrorCode::MalformedResponse,
            Self::HttpStatus(status, _) => {
                if let Some(exact) = u16::try_from(*status).ok().and_then(RiseUploadErrorCode::from_http_status) {
                    return exact;
                }
                if SaiWsChunkedFileUpload::is_retryable_http_status(*status) {
                    RiseUploadErrorCode::Server
                } else {
                    RiseUploadErrorCode::BadRequest
                }
            }
        }
    }

    pub fn error_description(&self) -> Option<String> {
        Some(tr(self.canon_code().i18n_key()))
    }

    /// debug detail for traces and logs, kept apart from the user message
    pub fn failure_reason(&self) -> Option<String> {
        Some(match self {
            Self::MissingBaseURL => "SaiFetch baseURL is not configured; cannot upload files.".to_owned(),
            Self::InvalidURL(value) => format!("Invalid upload URL: {value}"),
            Self::EmptyFile => "Cannot upload an empty file.".to_owned(),
            Self::FileTooLarge {
                byte_count,
                maximum_bytes,
            } => format!("File is {byte_count} bytes; the limit is {maximum_bytes}."),
            Self::OffsetMismatch { expected, acknowledged } => {
                let seen = acknowledged.map_or_else(|| "nothing".to_owned(), |value| value.to_string());
                format!("Slab acknowledged {seen}; the sent range ended at {expected}.")
            }
            Self::NotAcknowledged => "Upload completion was not acknowledged.".to_owned(),
            Self::InvalidResponse(message) => message.clone(),
            Self::HttpStatus(status, message) => format!("File upload HTTP {status}: {message}"),
        })
    }
}

impl fmt::Display for SaiWsChunkedFileUploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for SaiWsChunkedFileUploadError {}

struct SaiWsUploadAccount {
    user_id: Option<String>,
    epoch: Option<String>,
}

impl SaiWsUploadAccount {
    fn ensure_current(&self) -> anyhow::Result<()> {
        SaiWsChunkedFileUpload::ensure_current_account(self.user_id.as_deref(), self.epoch.as_deref())
    }
}

struct SaiWsUploadSessionAbortGuard {
    upload_id: String,
    base: Url,
    user_id: Option<String>,
    epoch: Option<String>,
    completed_successfully: bool,
    cx: AsyncApp,
}

impl Drop for SaiWsUploadSessionAbortGuard {
    fn drop(&mut self) {
        if self.completed_successfully {
            return;
        }
        SaiWsChunkedFileUpload::best_effort_abort(
            std::mem::take(&mut self.upload_id),
            &self.base,
            self.user_id.take(),
            self.epoch.take(),
            &self.cx,
        );
    }
}

struct SaiWsSlabAbortOnDrop(tokio::task::AbortHandle);

impl Drop for SaiWsSlabAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum SaiWsSlabEvent {
    Finished(Result<Result<(Vec<u8>, HTTPURLResponse), URLError>, tokio::task::JoinError>),
    Progress(Option<(f64, i64, i64)>),
    StallCheck,
}

pub enum SaiWsChunkedFileUpload {}

impl SaiWsChunkedFileUpload {
    fn minimum_slab_size() -> i64 {
        i64::try_from(rise_upload::SLAB_MINIMUM_BYTES).unwrap_or(i64::MAX)
    }

    fn maximum_slab_size() -> i64 {
        i64::try_from(rise_upload::SLAB_MAXIMUM_BYTES).unwrap_or(i64::MAX)
    }

    fn default_slab_size() -> i64 {
        i64::try_from(rise_upload::SLAB_DEFAULT_BYTES).unwrap_or(i64::MAX)
    }

    /// slab size from the server value, a number is cut to int and a bool is 1 or 0
    /// a string is parsed and anything else gives the canon default
    pub fn resolved_slab_size(server_value: Option<&Value>) -> i64 {
        let preferred = match server_value {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_u64().map(|value| i64::try_from(value).unwrap_or(i64::MAX)))
                .or_else(|| number.as_f64().map(|value| value as i64))
                .unwrap_or_else(Self::default_slab_size),
            Some(Value::Bool(flag)) => i64::from(*flag),
            Some(Value::String(text)) => int_from_string(text).unwrap_or_else(Self::default_slab_size),
            _ => Self::default_slab_size(),
        };
        Self::maximum_slab_size().min(Self::minimum_slab_size().max(preferred))
    }

    pub fn resolved_slab_size_initiate_data(initiate_data: &Map<String, Value>) -> i64 {
        Self::resolved_slab_size(
            initiate_data
                .get(upload_key::SLAB_SIZE)
                .or_else(|| initiate_data.get(upload_key::CHUNK_SIZE)),
        )
    }

    /// true if the server supports offset aware slabs, a json null counts as present
    pub fn supports_offset_aware_slabs(initiate_data: &Map<String, Value>) -> bool {
        initiate_data.contains_key(upload_key::SLAB_SIZE)
    }

    fn is_retryable_http_status(status: i64) -> bool {
        let first = i64::try_from(rise_upload::RETRY_RETRYABLE_HTTP_STATUS_FIRST).unwrap_or(i64::MAX);
        let last = i64::try_from(rise_upload::RETRY_RETRYABLE_HTTP_STATUS_LAST).unwrap_or(i64::MAX);
        (first..=last).contains(&status)
    }

    pub fn should_retry_slab(
        offset_aware: bool,
        attempt: i64,
        status_code: Option<i64>,
        url_error_code: Option<URLErrorCode>,
    ) -> bool {
        let maximum_attempts = i64::try_from(rise_upload::RETRY_SLAB_MAXIMUM_ATTEMPTS).unwrap_or(i64::MAX);
        if !offset_aware || attempt >= maximum_attempts - 1 {
            return false;
        }
        if let Some(status_code) = status_code {
            return Self::is_retryable_http_status(status_code);
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

    pub fn overall_progress(completed_bytes: i64, slab_bytes_sent: i64, total_bytes: i64) -> f64 {
        if total_bytes <= 0 {
            return 0.0;
        }
        let sent = total_bytes.min(completed_bytes.max(0).saturating_add(slab_bytes_sent.max(0)));
        rise_upload::PROGRESS_CEILING_FRACTION.min(sent as f64 / total_bytes as f64)
    }

    /// slab retry delay in ns, attempt is already incremented so first retry reads index 0
    pub fn retry_slab_delay_nanoseconds(attempt: i64) -> u64 {
        Self::canon_delay_nanoseconds(&rise_upload::RETRY_SLAB_DELAYS_MS, attempt - 1)
    }

    pub fn retry_complete_delay_nanoseconds(attempt: i64) -> u64 {
        Self::canon_delay_nanoseconds(&rise_upload::RETRY_COMPLETE_DELAYS_MS, attempt)
    }

    fn canon_delay_nanoseconds(table: &[u64], step: i64) -> u64 {
        if table.is_empty() {
            return 0;
        }
        let last = table.len() - 1;
        let index = usize::try_from(step.max(0)).unwrap_or(last).min(last);
        table[index].saturating_mul(1_000_000)
    }

    /// uploads a file in slabs, account defaults to the active user and epoch
    #[allow(clippy::too_many_arguments)]
    pub fn upload(
        file_url: PathBuf,
        filename: String,
        content_type: String,
        file_context: String,
        resource_id: Option<String>,
        expected_account_user_id: Option<String>,
        expected_account_epoch: Option<String>,
        on_progress: Option<SaiWsUploadOnProgress>,
        on_byte_progress: Option<SaiWsUploadOnByteProgress>,
        cx: &mut App,
    ) -> Task<anyhow::Result<Map<String, Value>>> {
        let account = SaiWsUploadAccount {
            user_id: expected_account_user_id,
            epoch: expected_account_epoch,
        };
        if let Err(error) = account.ensure_current() {
            return Task::ready(Err(error));
        }
        let quit_lease = QuitDrain::lease("sai_ws.file_upload", cx);
        cx.spawn(async move |cx| {
            let _quit_lease = quit_lease;
            Self::upload_once(
                file_url,
                filename,
                content_type,
                file_context,
                resource_id,
                account,
                on_progress,
                on_byte_progress,
                cx,
            )
            .await
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn upload_once(
        file_url: PathBuf,
        filename: String,
        content_type: String,
        file_context: String,
        resource_id: Option<String>,
        account: SaiWsUploadAccount,
        on_progress: Option<SaiWsUploadOnProgress>,
        on_byte_progress: Option<SaiWsUploadOnByteProgress>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Map<String, Value>> {
        account.ensure_current()?;
        let base = cx.update(|cx| SaiFetchHttpConfiguration::shared(cx).read(cx).base_url.clone());
        let Some(base) = base else {
            return Err(SaiWsChunkedFileUploadError::MissingBaseURL.into());
        };

        let file_source = cx
            .background_executor()
            .spawn(async move { SaiWsUploadFileSource::new(&file_url) })
            .await?;
        let file_source = Arc::new(file_source);
        let _file_source_closer = SaiWsUploadFileSourceCloser(Arc::clone(&file_source));
        let total = file_source.total_bytes;
        if total <= 0 {
            return Err(SaiWsChunkedFileUploadError::EmptyFile.into());
        }
        let premium = cx.update(|cx| sai_auth_hooks().is_some_and(|hooks| (hooks.is_premium)(cx)));
        let maximum_bytes = i64::try_from(rise_upload::PIPELINE_MAXIMUM_FILE_BYTES)
            .unwrap_or(i64::MAX)
            .min(RiseLimits::upload_maximum_bytes(&file_context, &content_type, premium));
        if total > maximum_bytes {
            return Err(SaiWsChunkedFileUploadError::FileTooLarge {
                byte_count: total,
                maximum_bytes,
            }
            .into());
        }

        let initiate_url = Self::resolve_api_url(&base, PATH_INITIATE)?;
        let mut initiate_body = Map::new();
        initiate_body.insert(upload_key::FILENAME.to_owned(), Value::String(filename));
        initiate_body.insert(upload_key::CONTENT_TYPE.to_owned(), Value::String(content_type));
        initiate_body.insert(upload_key::FILE_CONTEXT.to_owned(), Value::String(file_context));
        initiate_body.insert(upload_key::TOTAL_SIZE.to_owned(), Value::from(total));
        if let Some(resource_id) = resource_id {
            initiate_body.insert(upload_key::RESOURCE_ID.to_owned(), Value::String(resource_id));
        }

        let reconnect_deadline = Self::connection_loss_deadline();
        let mut offline_polls: u32 = 0;
        let init_data = loop {
            match Self::perform_json_request(
                &initiate_url,
                METHOD_POST,
                &initiate_body,
                FETCH_ID_INITIATE,
                &[],
                &account,
                cx,
            )
            .await
            {
                Ok(initiated) => break initiated,
                Err(error) => {
                    if !Self::await_reconnect_after_connection_loss(
                        &error,
                        reconnect_deadline,
                        &mut offline_polls,
                        true,
                        cx,
                    )
                    .await?
                    {
                        return Err(error);
                    }
                }
            }
        };
        let upload_id = Self::string(&init_data, &[upload_key::UPLOAD_ID])?;
        let mut abort_guard = SaiWsUploadSessionAbortGuard {
            upload_id: upload_id.clone(),
            base: base.clone(),
            user_id: account.user_id.clone(),
            epoch: account.epoch.clone(),
            completed_successfully: false,
            cx: cx.clone(),
        };
        let slab_size = Self::resolved_slab_size_initiate_data(&init_data);
        let offset_aware = Self::supports_offset_aware_slabs(&init_data);

        let mut offset: i64 = 0;
        let progress_reporter = Rc::new(RefCell::new(SaiWsUploadProgressReporter::new(on_progress, on_byte_progress)));
        cx.update(|cx| progress_reporter.borrow_mut().report(0.0, 0, total, cx));
        let ticker_reporter = Rc::clone(&progress_reporter);
        let ticker: Task<()> = cx.spawn(async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(rise_upload::PROGRESS_MINIMUM_INTERVAL_MS))
                    .await;
                let _ = cx.update(|cx| ticker_reporter.borrow_mut().tick(cx));
            }
        });

        while offset < total {
            let slab_len = usize::try_from(slab_size.min(total - offset)).unwrap_or(0);
            let read_offset = offset;
            let reader = Arc::clone(&file_source);
            let chunk = cx
                .background_executor()
                .spawn(async move { reader.read(read_offset, slab_len) })
                .await?;
            if chunk.is_empty() {
                return Err(SaiWsChunkedFileUploadError::InvalidResponse("File changed during upload.".to_owned()).into());
            }
            let end = offset + i64::try_from(chunk.len()).unwrap_or(i64::MAX);
            let is_last = end >= total;

            let slab_url = Self::resolve_api_url(&base, PATH_SLAB)?;
            Self::perform_slab_put(
                &slab_url,
                &upload_id,
                is_last,
                chunk,
                offset,
                total,
                offset_aware,
                &progress_reporter,
                &account,
                cx,
            )
            .await?;

            offset = end;
            cx.update(|cx| progress_reporter.borrow_mut().acknowledge(offset, total, cx));
        }
        drop(ticker);

        let complete_url = Self::resolve_api_url(&base, PATH_COMPLETE)?;
        let maximum_complete_attempts = i64::try_from(rise_upload::RETRY_COMPLETE_MAXIMUM_ATTEMPTS).unwrap_or(i64::MAX);
        let mut completed: Option<Map<String, Value>> = None;
        let mut last_complete_error: Option<anyhow::Error> = None;
        let mut attempt: i64 = 0;
        while attempt < maximum_complete_attempts {
            account.ensure_current()?;
            match Self::perform_json_request(
                &complete_url,
                METHOD_POST,
                &Map::new(),
                FETCH_ID_COMPLETE,
                &[(HEADER_UPLOAD_ID, upload_id.as_str())],
                &account,
                cx,
            )
            .await
            {
                Ok(done) => {
                    completed = Some(done);
                    break;
                }
                Err(error) => {
                    let reconnect = Self::await_reconnect_after_connection_loss(
                        &error,
                        reconnect_deadline,
                        &mut offline_polls,
                        false,
                        cx,
                    )
                    .await?;
                    last_complete_error = Some(error);
                    if reconnect {
                        continue;
                    }
                    if attempt >= maximum_complete_attempts - 1 {
                        break;
                    }
                    cx.background_executor()
                        .timer(Duration::from_nanos(Self::retry_complete_delay_nanoseconds(attempt)))
                        .await;
                    attempt += 1;
                }
            }
        }
        let Some(mut done) = completed else {
            return Err(last_complete_error.unwrap_or_else(|| SaiWsChunkedFileUploadError::NotAcknowledged.into()));
        };
        let server_asset_id_missing = done
            .get(upload_key::ASSET_ID)
            .and_then(Value::as_str)
            .is_none_or(|asset_id| trimming_whitespaces_and_newlines(asset_id).is_empty());
        if server_asset_id_missing {
            done.insert(upload_key::ASSET_ID.to_owned(), Value::String(upload_id));
        }
        cx.update(|cx| progress_reporter.borrow_mut().report(1.0, total, total, cx));
        abort_guard.completed_successfully = true;
        Ok(done)
    }

    fn best_effort_abort(
        upload_id: String,
        base: &Url,
        expected_account_user_id: Option<String>,
        expected_account_epoch: Option<String>,
        cx: &AsyncApp,
    ) {
        if upload_id.is_empty() {
            return;
        }
        let Ok(url) = Self::resolve_api_url(base, PATH_ABORT) else {
            return;
        };
        let account = SaiWsUploadAccount {
            user_id: expected_account_user_id,
            epoch: expected_account_epoch,
        };
        cx.spawn(async move |cx| {
            let _ = Self::perform_json_request(
                &url,
                METHOD_POST,
                &Map::new(),
                FETCH_ID_ABORT,
                &[(HEADER_UPLOAD_ID, upload_id.as_str())],
                &account,
                cx,
            )
            .await;
        })
        .detach();
    }

    /// http refusals never reached the canon bridge so this relays them
    /// no code means nothing published, byte quota refuses only here
    fn relay_server_refusal(body: &[u8], cx: &mut AsyncApp) {
        if body.is_empty() {
            return;
        }
        let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(body) else {
            return;
        };
        let Some(code) = object.get(wire_key::CODE).and_then(Value::as_str).filter(|code| !code.is_empty()) else {
            return;
        };
        let mut details: HashMap<String, String> = HashMap::new();
        if let Some(Value::Object(raw)) = object.get(wire_key::DETAILS) {
            for (key, value) in raw {
                let text = match value {
                    Value::String(text) => text.clone(),
                    Value::Number(number) => number.to_string(),
                    Value::Bool(flag) => if *flag { "1" } else { "0" }.to_owned(),
                    _ => continue,
                };
                details.insert(key.clone(), text);
            }
        }
        let message = object
            .get(wire_key::MESSAGE)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let failure = RiseFailure::new(Some(code.to_owned()), message, details);
        cx.update(|cx| RiseServerRefusalRelay::publish(failure, cx));
    }

    fn resolve_api_url(base: &Url, path: &str) -> anyhow::Result<Url> {
        let base_str = trimming_characters(base.absolute_string(), |c| c == '/');
        let p = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        };
        let joined = format!("{base_str}{p}");
        Url::from_string(&joined).ok_or_else(|| SaiWsChunkedFileUploadError::InvalidURL(joined).into())
    }

    /// sets a header and replaces any old value under the same name, case insensitive
    fn set_value(request: &mut Request<Vec<u8>>, value: &str, field: &str) -> anyhow::Result<()> {
        let name = HeaderName::from_bytes(field.as_bytes())?;
        let value = HeaderValue::from_str(value)?;
        request.headers_mut().insert(name, value);
        Ok(())
    }

    fn new_request(url: &Url, method: &str) -> anyhow::Result<Request<Vec<u8>>> {
        let mut request = Request::new(Vec::new());
        *request.method_mut() = Method::from_bytes(method.as_bytes())?;
        *request.uri_mut() = url
            .absolute_string()
            .parse::<Uri>()
            .map_err(|_| SaiWsChunkedFileUploadError::InvalidURL(url.absolute_string().to_owned()))?;
        Ok(request)
    }

    /// runs every request interceptor in order
    async fn intercept(
        mut request: Request<Vec<u8>>,
        interceptors: &[SaiFetchRequestInterceptor],
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Request<Vec<u8>>> {
        for hook in interceptors {
            let pending = cx.update(|cx| hook(request, cx));
            request = pending.await?;
        }
        Ok(request)
    }

    /// the intercepted request that goes to the engine
    fn url_request(request: Request<Vec<u8>>, timeout_interval: Option<Duration>) -> anyhow::Result<URLRequest> {
        let (parts, body) = request.into_parts();
        let uri = parts.uri.to_string();
        let url = Url::from_string(&uri).ok_or(SaiWsChunkedFileUploadError::InvalidURL(uri))?;
        let mut out = URLRequest::new(url);
        out.http_method = parts.method.as_str().to_owned();
        out.http_body = (!body.is_empty()).then_some(body);
        if let Some(timeout_interval) = timeout_interval {
            out.timeout_interval = timeout_interval;
        }
        for (name, value) in &parts.headers {
            if let Ok(value) = value.to_str() {
                out.set_value(value, name.as_str());
            }
        }
        Ok(out)
    }

    fn joined_url_error(joined: Result<Result<(Vec<u8>, HTTPURLResponse), URLError>, tokio::task::JoinError>) -> anyhow::Result<(Vec<u8>, HTTPURLResponse)> {
        match joined {
            Ok(Ok(exchange)) => Ok(exchange),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => Err(URLError::new(URLErrorCode::Cancelled).into()),
        }
    }

    fn snapshot_configuration(cx: &mut AsyncApp) -> (HashMap<String, String>, Vec<SaiFetchRequestInterceptor>) {
        cx.update(|cx| {
            let configuration = SaiFetchHttpConfiguration::shared(cx);
            let cfg = configuration.read(cx);
            (cfg.default_headers.clone(), cfg.request_interceptors.clone())
        })
    }

    async fn execute_json(
        url: &Url,
        method: &str,
        json_body: &Map<String, Value>,
        fetch_id: &str,
        extra_headers: &[(&str, &str)],
        account: &SaiWsUploadAccount,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<(Vec<u8>, HTTPURLResponse)> {
        account.ensure_current()?;
        let mut req = Self::new_request(url, method)?;
        Self::set_value(&mut req, ACCEPT_JSON, HEADER_ACCEPT)?;
        Self::set_value(&mut req, CONTENT_TYPE_JSON, HEADER_CONTENT_TYPE)?;
        let (default_headers, request_interceptors) = Self::snapshot_configuration(cx);
        for (k, v) in &default_headers {
            Self::set_value(&mut req, v, k)?;
        }
        for (k, v) in extra_headers {
            Self::set_value(&mut req, v, k)?;
        }
        if method.to_uppercase() == METHOD_POST || !json_body.is_empty() {
            *req.body_mut() = serde_json::to_vec(&Value::Object(json_body.clone()))?;
        }
        let req = Self::intercept(req, &request_interceptors, cx).await?;
        account.ensure_current()?;
        let mut trace = cx.update(|cx| SaiWsUploadHTTPTrace::new(&req, fetch_id, None, cx));
        let request = Self::url_request(req, None)?;
        let joined = RiseAccountRuntime::current()
            .spawn(async move { URLSession::shared().data_for(request).await })
            .await;
        Self::finish_trace(&mut trace, Self::joined_url_error(joined), account, cx)
    }

    /// completes the trace on success and on failure
    fn finish_trace(
        trace: &mut SaiWsUploadHTTPTrace,
        exchange: anyhow::Result<(Vec<u8>, HTTPURLResponse)>,
        account: &SaiWsUploadAccount,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<(Vec<u8>, HTTPURLResponse)> {
        let checked = exchange.and_then(|exchange| account.ensure_current().map(|()| exchange));
        match checked {
            Ok((data, http)) => {
                cx.update(|cx| trace.complete(&data, Some(http), None, cx));
                Ok((data, http))
            }
            Err(error) => {
                let transport_error = error.to_string();
                cx.update(|cx| trace.complete(&[], None, Some(transport_error), cx));
                Err(error)
            }
        }
    }

    async fn perform_json_request(
        url: &Url,
        method: &str,
        json_body: &Map<String, Value>,
        fetch_id: &str,
        extra_headers: &[(&str, &str)],
        account: &SaiWsUploadAccount,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Map<String, Value>> {
        let (mut data, mut http) = Self::execute_json(url, method, json_body, fetch_id, extra_headers, account, cx).await?;
        if http.status_code == HTTP_UNAUTHORIZED {
            account.ensure_current()?;
            cx.update(sai_auth_refresh_access_token).await?;
            account.ensure_current()?;
            (data, http) = Self::execute_json(url, method, json_body, fetch_id, extra_headers, account, cx).await?;
        }
        if !(200..300).contains(&http.status_code) {
            let msg = String::from_utf8(data.clone()).unwrap_or_default();
            Self::relay_server_refusal(&data, cx);
            return Err(SaiWsChunkedFileUploadError::HttpStatus(http.status_code, msg).into());
        }
        let obj: Value = serde_json::from_slice(&data)?;
        let Value::Object(mut dict) = obj else {
            return Err(SaiWsChunkedFileUploadError::InvalidResponse("Top-level JSON is not an object".to_owned()).into());
        };
        if let Some(Value::Bool(false)) = dict.get(file::SUCCESS) {
            let msg = dict
                .get(upload_key::MESSAGE)
                .and_then(Value::as_str)
                .unwrap_or("success=false")
                .to_owned();
            return Err(SaiWsChunkedFileUploadError::InvalidResponse(msg).into());
        }
        match dict.remove(bot::DATA) {
            Some(Value::Object(inner)) => Ok(inner),
            Some(other) => {
                dict.insert(bot::DATA.to_owned(), other);
                Ok(dict)
            }
            None => Ok(dict),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_slab(
        url: &Url,
        upload_id: &str,
        is_last: bool,
        body: &[u8],
        completed_bytes: i64,
        total_bytes: i64,
        offset_aware: bool,
        progress_reporter: &Rc<RefCell<SaiWsUploadProgressReporter>>,
        account: &SaiWsUploadAccount,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<(Vec<u8>, HTTPURLResponse)> {
        account.ensure_current()?;
        let mut req = Self::new_request(url, METHOD_PUT)?;
        *req.body_mut() = body.to_vec();
        let (default_headers, request_interceptors) = Self::snapshot_configuration(cx);
        for (k, v) in &default_headers {
            Self::set_value(&mut req, v, k)?;
        }
        Self::set_value(&mut req, CONTENT_TYPE_OCTET_STREAM, HEADER_CONTENT_TYPE)?;
        Self::set_value(&mut req, upload_id, HEADER_UPLOAD_ID)?;
        Self::set_value(&mut req, if is_last { "true" } else { "false" }, HEADER_LAST_SLAB)?;
        if offset_aware {
            Self::set_value(&mut req, &completed_bytes.to_string(), HEADER_SLAB_OFFSET)?;
        }
        let req = Self::intercept(req, &request_interceptors, cx).await?;
        account.ensure_current()?;
        let (sink, progress) = mpsc::unbounded();
        let delegate = Arc::new(SaiWsSlabProgressDelegate::new(completed_bytes, total_bytes, sink));
        let request_body_wire = format!("<binary {} bytes>", body.len());
        let mut trace = cx.update(|cx| SaiWsUploadHTTPTrace::new(&req, FETCH_ID_SLAB, Some(request_body_wire), cx));
        let request = Self::url_request(req, Some(Self::slab_request_timeout()))?;
        let exchange = Self::send_slab(request, delegate, progress, progress_reporter, cx).await;
        Self::finish_trace(&mut trace, exchange, account, cx)
    }

    #[allow(clippy::too_many_arguments)]
    async fn perform_slab_put(
        url: &Url,
        upload_id: &str,
        is_last: bool,
        body: Vec<u8>,
        completed_bytes: i64,
        total_bytes: i64,
        offset_aware: bool,
        progress_reporter: &Rc<RefCell<SaiWsUploadProgressReporter>>,
        account: &SaiWsUploadAccount,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Map<String, Value>> {
        let mut attempt: i64 = 0;
        let mut authorization_refreshes: u64 = 0;
        let reconnect_deadline = Self::connection_loss_deadline();
        let mut offline_polls: u32 = 0;

        loop {
            let error = match Self::execute_slab(
                url,
                upload_id,
                is_last,
                &body,
                completed_bytes,
                total_bytes,
                offset_aware,
                progress_reporter,
                account,
                cx,
            )
            .await
            {
                Ok((data, http)) => {
                    if http.status_code == HTTP_UNAUTHORIZED
                        && authorization_refreshes < rise_upload::RETRY_AUTHORIZATION_REFRESHES_PER_SLAB
                    {
                        authorization_refreshes += 1;
                        match Self::refresh_authorization(account, cx).await {
                            Ok(()) => continue,
                            Err(error) => error,
                        }
                    } else {
                        if Self::should_retry_slab(offset_aware, attempt, Some(http.status_code), None) {
                            attempt += 1;
                            cx.background_executor()
                                .timer(Duration::from_nanos(Self::retry_slab_delay_nanoseconds(attempt)))
                                .await;
                            continue;
                        }
                        match Self::acknowledge_slab(&data, http, completed_bytes, &body, offset_aware, cx) {
                            Ok(payload) => return Ok(payload),
                            Err(error) => error,
                        }
                    }
                }
                Err(error) => error,
            };
            let url_code = Self::url_error_code(&error);
            if offset_aware
                && Self::await_reconnect_after_connection_loss(
                    &error,
                    reconnect_deadline,
                    &mut offline_polls,
                    false,
                    cx,
                )
                .await?
            {
                continue;
            }
            if !Self::should_retry_slab(offset_aware, attempt, None, url_code) {
                return Err(error);
            }
            attempt += 1;
            cx.background_executor()
                .timer(Duration::from_nanos(Self::retry_slab_delay_nanoseconds(attempt)))
                .await;
        }
    }

    /// refreshes the token for the slab ladder, failure goes into the ladder error path
    async fn refresh_authorization(account: &SaiWsUploadAccount, cx: &mut AsyncApp) -> anyhow::Result<()> {
        account.ensure_current()?;
        cx.update(sai_auth_refresh_access_token).await?;
        account.ensure_current()
    }

    fn acknowledge_slab(
        data: &[u8],
        http: HTTPURLResponse,
        completed_bytes: i64,
        body: &[u8],
        offset_aware: bool,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Map<String, Value>> {
        if !(200..300).contains(&http.status_code) {
            let msg = String::from_utf8(data.to_vec()).unwrap_or_default();
            Self::relay_server_refusal(data, cx);
            return Err(SaiWsChunkedFileUploadError::HttpStatus(http.status_code, msg).into());
        }
        let obj: Value = serde_json::from_slice(data)?;
        let Value::Object(mut dict) = obj else {
            return Err(SaiWsChunkedFileUploadError::InvalidResponse("Slab response not object".to_owned()).into());
        };
        let payload = match dict.remove(bot::DATA) {
            Some(Value::Object(inner)) => inner,
            Some(other) => {
                dict.insert(bot::DATA.to_owned(), other);
                dict
            }
            None => dict,
        };
        if offset_aware {
            let expected_next_offset = completed_bytes + i64::try_from(body.len()).unwrap_or(i64::MAX);
            let acknowledged = Self::integer(payload.get(bot::NEXT_OFFSET));
            if acknowledged != Some(expected_next_offset) {
                return Err(SaiWsChunkedFileUploadError::OffsetMismatch {
                    expected: expected_next_offset,
                    acknowledged,
                }
                .into());
            }
        }
        Ok(payload)
    }

    const SLAB_SEND_STALL_TIMEOUT: Duration = Duration::from_secs(60);
    const SLAB_STALL_CHECK_INTERVAL: Duration = Duration::from_secs(5);

    fn upload_session_window() -> Duration {
        let window_ms = RiseTimingCanon::server_window(UPLOAD_SESSION_TTL).window_ms;
        Duration::from_millis(u64::try_from(window_ms).unwrap_or(0))
    }

    fn slab_request_timeout() -> Duration {
        Self::SLAB_SEND_STALL_TIMEOUT.max(Self::upload_session_window())
    }

    async fn send_slab(
        request: URLRequest,
        delegate: Arc<SaiWsSlabProgressDelegate>,
        mut progress: mpsc::UnboundedReceiver<(f64, i64, i64)>,
        progress_reporter: &Rc<RefCell<SaiWsUploadProgressReporter>>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<(Vec<u8>, HTTPURLResponse)> {
        let session_delegate: Arc<dyn URLSessionTaskDelegate> = delegate.clone();
        let mut sending = RiseAccountRuntime::current()
            .spawn(async move { URLSession::shared().data_for_with_delegate(request, session_delegate).await });
        let _cancel_with_caller = SaiWsSlabAbortOnDrop(sending.abort_handle());
        let mut stall_check = Box::pin(cx.background_executor().timer(Self::SLAB_STALL_CHECK_INTERVAL));
        loop {
            let event = match select(&mut sending, select(progress.next(), &mut stall_check)).await {
                Either::Left((joined, _)) => SaiWsSlabEvent::Finished(joined),
                Either::Right((Either::Left((update, _)), _)) => SaiWsSlabEvent::Progress(update),
                Either::Right((Either::Right(((), _)), _)) => SaiWsSlabEvent::StallCheck,
            };
            match event {
                SaiWsSlabEvent::Finished(joined) => {
                    while let Ok((_fraction, uploaded_bytes, total_bytes)) = progress.try_recv() {
                        cx.update(|cx| progress_reporter.borrow_mut().sent(uploaded_bytes, total_bytes, cx));
                    }
                    return Self::joined_url_error(joined);
                }
                SaiWsSlabEvent::Progress(Some((_fraction, uploaded_bytes, total_bytes))) => {
                    cx.update(|cx| progress_reporter.borrow_mut().sent(uploaded_bytes, total_bytes, cx));
                }
                SaiWsSlabEvent::Progress(None) => {
                    let joined = (&mut sending).await;
                    return Self::joined_url_error(joined);
                }
                SaiWsSlabEvent::StallCheck => {
                    if delegate.send_stalled(Self::SLAB_SEND_STALL_TIMEOUT) {
                        return Err(URLError::new(URLErrorCode::TimedOut).into());
                    }
                    stall_check = Box::pin(cx.background_executor().timer(Self::SLAB_STALL_CHECK_INTERVAL));
                }
            }
        }
    }

    fn connection_loss_deadline() -> Duration {
        RiseClock::monotonic() + Self::upload_session_window()
    }

    /// waits for the network after a lost connection, then upload resumes from same offset
    /// pause doubles from the slab delay once online, dont trust the socket here lol
    async fn await_reconnect_after_connection_loss(
        error: &anyhow::Error,
        deadline: Duration,
        polls: &mut u32,
        request_never_sent: bool,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<bool> {
        let codes: &[URLErrorCode] = if request_never_sent {
            &Self::UNSENT_REQUEST_CODES[..]
        } else {
            &Self::CONNECTION_LOSS_CODES[..]
        };
        let Some(url_code) = Self::url_error_code(error) else {
            return Ok(false);
        };
        if !codes.contains(&url_code) || RiseClock::monotonic() >= deadline {
            return Ok(false);
        }
        let slab_delays = i64::try_from(rise_upload::RETRY_SLAB_DELAYS_MS.len()).unwrap_or(0);
        let base = Self::retry_slab_delay_nanoseconds(slab_delays);
        let pause = base << (*polls).min(Self::MAXIMUM_OFFLINE_PAUSE_DOUBLINGS);
        *polls = polls.saturating_add(1);
        let wake = deadline.min(RiseClock::monotonic() + Duration::from_nanos(pause));
        Self::wait_for_usable_network_path(deadline, cx).await;
        let now = RiseClock::monotonic();
        if now < wake {
            cx.background_executor().timer(wake - now).await;
        }
        Ok(RiseClock::monotonic() < deadline)
    }

    const MAXIMUM_OFFLINE_PAUSE_DOUBLINGS: u32 = 5;

    async fn wait_for_usable_network_path(deadline: Duration, cx: &mut AsyncApp) {
        let (usable, mut updates) = mpsc::unbounded::<bool>();
        let watches = watch(move |event| {
            if let SystemActivityEvent::NetworkPath { satisfied } = event {
                let _ = usable.unbounded_send(satisfied);
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
        let satisfied = Box::pin(async move {
            while let Some(satisfied) = updates.next().await {
                if satisfied {
                    return;
                }
            }
        });
        let remaining = deadline.saturating_sub(RiseClock::monotonic());
        let expired = Box::pin(cx.background_executor().timer(remaining));
        select(satisfied, expired).await;
        drop(watches);
    }

    const UNSENT_REQUEST_CODES: [URLErrorCode; 5] = [
        URLErrorCode::CannotConnectToHost,
        URLErrorCode::NotConnectedToInternet,
        URLErrorCode::DnsLookupFailed,
        URLErrorCode::CannotFindHost,
        URLErrorCode::DataNotAllowed,
    ];

    const CONNECTION_LOSS_CODES: [URLErrorCode; 8] = [
        URLErrorCode::NetworkConnectionLost,
        URLErrorCode::TimedOut,
        URLErrorCode::CannotConnectToHost,
        URLErrorCode::NotConnectedToInternet,
        URLErrorCode::DnsLookupFailed,
        URLErrorCode::CannotFindHost,
        URLErrorCode::SecureConnectionFailed,
        URLErrorCode::DataNotAllowed,
    ];

    fn url_error_code(error: &anyhow::Error) -> Option<URLErrorCode> {
        error.downcast_ref::<URLError>().map(|error| error.code)
    }

    fn ensure_current_account(user_id: Option<&str>, epoch: Option<&str>) -> anyhow::Result<()> {
        if !SaiAccountCacheScope::is_current_account_epoch(user_id, epoch) {
            return Err(anyhow::Error::new(SaiWsOutboundCancelledError));
        }
        Ok(())
    }

    fn string(dict: &Map<String, Value>, keys: &[&str]) -> anyhow::Result<String> {
        for k in keys {
            match dict.get(*k) {
                Some(Value::String(s)) if !s.is_empty() => return Ok(s.clone()),
                Some(Value::Number(n)) => return Ok(n.to_string()),
                Some(Value::Bool(flag)) => return Ok(if *flag { "1" } else { "0" }.to_owned()),
                _ => {}
            }
        }
        Err(SaiWsChunkedFileUploadError::InvalidResponse(format!(
            "Missing keys {keys:?} in {}",
            Value::Object(dict.clone())
        ))
        .into())
    }

    fn integer(value: Option<&Value>) -> Option<i64> {
        match value? {
            Value::Number(number) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value as i64)),
            Value::Bool(flag) => Some(i64::from(*flag)),
            Value::String(text) => int_from_string(text),
            _ => None,
        }
    }
}
