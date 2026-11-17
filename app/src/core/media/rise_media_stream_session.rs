use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use futures::future::LocalBoxFuture;
use gpui::{AsyncApp, Task};
use parking_lot::Mutex;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::URLRequest;
use rise_platform::file_manager::{CocoaError, CocoaErrorCode, FileManager};
use serde::{Deserialize, Serialize};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDownloadProgress, RemoteMediaDownloadProgressCoalescer, RemoteMediaHttpResponse,
    RemoteMediaTransferError,
};
use crate::core::media::rise_media_transfer_hub::RiseMediaTransferHub;
use crate::core::media::rise_progressive_data_fetcher::{
    RiseProgressiveDataFetcher, RiseProgressiveDataFetcherLane, RiseProgressiveDataFetcherOnChunk,
    RiseProgressiveDataFetcherOnResponse,
};

/// half open byte interval start to end that is on disk
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiseMediaByteRange {
    pub start: i64,
    pub end: i64,
}

impl RiseMediaByteRange {
    pub fn length(&self) -> i64 {
        (self.end - self.start).max(0)
    }

    pub fn contains(&self, offset: i64) -> bool {
        offset >= self.start && offset < self.end
    }
}

/// what survives a relaunch about a half downloaded object
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseMediaPartialManifest {
    source_digest: String,
    content_length: i64,
    content_type: Option<String>,
    ranges: Vec<RiseMediaByteRange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMediaStreamError {
    AccountChanged,
    Cancelled,
    UnusableSource,
    RangeUnsupported,
    TooLarge,
}

impl RiseMediaStreamError {
    pub fn error_description(&self) -> Option<String> {
        Some(
            match self {
                Self::AccountChanged => "Account changed while media was loading",
                Self::Cancelled => "Media transfer was cancelled",
                Self::UnusableSource => "Invalid media URL",
                Self::RangeUnsupported => "Server cannot serve media ranges",
                Self::TooLarge => "Media is too large to cache",
            }
            .to_owned(),
        )
    }
}

impl fmt::Display for RiseMediaStreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for RiseMediaStreamError {}

impl From<RiseMediaStreamError> for RemoteMediaTransferError {
    fn from(error: RiseMediaStreamError) -> Self {
        Self::Other(Arc::new(anyhow::Error::new(error)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMediaStreamSessionContentInformation {
    pub content_length: i64,
    pub content_type: Option<String>,
}

/// injectable transport so range, hole and commit logic can be tested without network
/// on_response returns false to reject
pub type RiseMediaStreamSessionRangeFetcher = Arc<
    dyn Fn(
            URLRequest,
            RiseProgressiveDataFetcherOnResponse,
            RiseProgressiveDataFetcherOnChunk,
            AsyncApp,
        ) -> LocalBoxFuture<'static, Result<(), RemoteMediaTransferError>>
        + Send
        + Sync,
>;

enum RiseMediaStreamSessionFetchEvent {
    Response(RemoteMediaHttpResponse),
    Chunk(Vec<u8>),
}

struct RiseMediaStreamSessionByteWaiter {
    offset: i64,
    continuation: oneshot::Sender<Result<(), RemoteMediaTransferError>>,
}

#[derive(Default)]
struct RiseMediaStreamSessionFiles {
    partial_url: Option<PathBuf>,
    manifest_url: Option<PathBuf>,
    write_handle: Option<File>,
    read_handle: Option<File>,
}

#[derive(Default)]
struct RiseMediaStreamSessionState {
    covered: Vec<RiseMediaByteRange>,
    content_length: Option<i64>,
    content_type: Option<String>,
    fetch_task: Option<Task<()>>,
    fetch_generation: u64,
    fetch_head: i64,
    completion_requested: bool,
    failure: Option<RemoteMediaTransferError>,
    committed_url: Option<PathBuf>,
    is_committing: bool,
    last_manifest_write: f64,
    consecutive_failures: i64,
    progress_coalescer: RemoteMediaDownloadProgressCoalescer,
    information_waiters:
        HashMap<Uuid, oneshot::Sender<Result<RiseMediaStreamSessionContentInformation, RemoteMediaTransferError>>>,
    byte_waiters: HashMap<Uuid, RiseMediaStreamSessionByteWaiter>,
    completion_waiters: HashMap<Uuid, oneshot::Sender<Result<PathBuf, RemoteMediaTransferError>>>,
}

#[derive(Clone, Copy)]
enum RiseMediaStreamSessionWaiterKind {
    Information,
    Bytes,
    Completion,
}

/// cancel guard of a waiting consumer, walking away drops only its own waiter
struct RiseMediaStreamSessionWaiterGuard {
    session: Weak<RiseMediaStreamSession>,
    identifier: Uuid,
    kind: RiseMediaStreamSessionWaiterKind,
    armed: bool,
}

impl RiseMediaStreamSessionWaiterGuard {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for RiseMediaStreamSessionWaiterGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(session) = self.session.upgrade() else {
            return;
        };
        match self.kind {
            RiseMediaStreamSessionWaiterKind::Information => session.drop_information_waiter(self.identifier),
            RiseMediaStreamSessionWaiterKind::Bytes => session.drop_byte_waiter(self.identifier),
            RiseMediaStreamSessionWaiterKind::Completion => session.drop_completion_waiter(self.identifier),
        }
    }
}

/// one growing file and one http stream with two kinds of consumer
/// player asks for byte ranges, the download chip for the whole object
/// both read the same partial file so one fetch, seek past the write head re aims it
pub struct RiseMediaStreamSession {
    pub cache_key: String,
    remote: Url,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    maximum_byte_count: i64,
    suggested_filename: Option<String>,
    fetcher: RiseMediaStreamSessionRangeFetcher,
    this: Weak<RiseMediaStreamSession>,
    files: Mutex<RiseMediaStreamSessionFiles>,
    state: RiseIsolated<RiseMediaStreamSessionState>,
}

impl RiseMediaStreamSession {
    const MAXIMUM_READ_CHUNK: i64 = 256 * 1_024;
    const SEEK_TOLERANCE: i64 = 4 * 1_024 * 1_024;
    const MANIFEST_WRITE_INTERVAL: f64 = 1.0;
    const MAXIMUM_CONSECUTIVE_FAILURES: i64 = 3;
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

    pub fn new(
        cache_key: &str,
        remote: Url,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        maximum_byte_count: i64,
        suggested_filename: Option<String>,
        fetcher: Option<RiseMediaStreamSessionRangeFetcher>,
    ) -> Arc<Self> {
        let fetcher = fetcher.unwrap_or_else(|| -> RiseMediaStreamSessionRangeFetcher {
            let user_id = account_user_id.clone();
            let epoch = account_epoch.clone();
            Arc::new(
                move |request: URLRequest,
                      on_response: RiseProgressiveDataFetcherOnResponse,
                      on_chunk: RiseProgressiveDataFetcherOnChunk,
                      mut cx: AsyncApp| {
                    let user_id = user_id.clone();
                    let epoch = epoch.clone();
                    async move {
                        RiseProgressiveDataFetcher::stream(
                            request,
                            RiseProgressiveDataFetcherLane::PrivateEphemeral,
                            user_id,
                            epoch,
                            on_response,
                            on_chunk,
                            &mut cx,
                        )
                        .await
                    }
                    .boxed_local()
                },
            )
        });
        Arc::new_cyclic(|this| Self {
            cache_key: cache_key.to_owned(),
            remote,
            account_user_id,
            account_epoch,
            maximum_byte_count,
            suggested_filename,
            fetcher,
            this: this.clone(),
            files: Mutex::new(RiseMediaStreamSessionFiles::default()),
            state: RiseIsolated::new(RiseMediaStreamSessionState::default()),
        })
    }

    /// length and type from the first response, the player blocks on this before asking for bytes
    pub async fn content_information(&self, cx: &AsyncApp) -> anyhow::Result<RiseMediaStreamSessionContentInformation> {
        self.await_content_information(cx).await.map_err(Self::thrown)
    }

    async fn await_content_information(
        &self,
        cx: &AsyncApp,
    ) -> Result<RiseMediaStreamSessionContentInformation, RemoteMediaTransferError> {
        self.prepare_if_needed()?;
        let identifier = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel();
        let known = self.state.with(|state| {
            if let Some(failure) = &state.failure {
                return Some(Err(failure.clone()));
            }
            if let Some(content_length) = state.content_length {
                return Some(Ok(RiseMediaStreamSessionContentInformation {
                    content_length,
                    content_type: state.content_type.clone(),
                }));
            }
            state.information_waiters.insert(identifier, sender);
            None
        });
        if let Some(known) = known {
            return known;
        }
        self.ensure_fetch(cx);
        let guard = self.waiter_guard(identifier, RiseMediaStreamSessionWaiterKind::Information);
        let resumed = receiver.await;
        guard.disarm();
        resumed.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    /// whatever is contiguously available from offset, waits only if nothing is there yet
    /// caller loops so one slow range never pins a big buffer
    pub async fn read(&self, offset: i64, max_length: i64, cx: &AsyncApp) -> anyhow::Result<Vec<u8>> {
        self.await_read(offset, max_length, cx).await.map_err(Self::thrown)
    }

    async fn await_read(&self, offset: i64, max_length: i64, cx: &AsyncApp) -> Result<Vec<u8>, RemoteMediaTransferError> {
        self.prepare_if_needed()?;
        if let Some(early) = self.state.with(|state| Self::read_boundary(state, offset)) {
            return early;
        }
        if offset < 0 {
            return Err(RiseMediaStreamError::UnusableSource.into());
        }

        if let Some(chunk) = self.read_covered(offset, max_length) {
            return Ok(chunk);
        }

        let identifier = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel();
        let registered = self.state.with(|state| {
            if state.covered.iter().any(|range| range.contains(offset)) {
                return false;
            }
            state.byte_waiters.insert(
                identifier,
                RiseMediaStreamSessionByteWaiter {
                    offset,
                    continuation: sender,
                },
            );
            true
        });
        if registered {
            self.ensure_fetch(cx);
            let guard = self.waiter_guard(identifier, RiseMediaStreamSessionWaiterKind::Bytes);
            let resumed = receiver.await;
            guard.disarm();
            resumed.unwrap_or_else(|_| Err(CancellationError.into()))?;
        }

        if let Some(early) = self.state.with(|state| Self::read_boundary(state, offset)) {
            return early;
        }
        Ok(self.read_covered(offset, max_length).unwrap_or_default())
    }

    /// asks for the whole object and returns the committed cache url
    /// also keeps the stream running after the player stops asking
    pub async fn complete(&self, cx: &AsyncApp) -> anyhow::Result<PathBuf> {
        self.await_complete(cx).await.map_err(Self::thrown)
    }

    async fn await_complete(&self, cx: &AsyncApp) -> Result<PathBuf, RemoteMediaTransferError> {
        if let Some(committed_url) = self.state.with(|state| state.committed_url.clone()) {
            return Ok(committed_url);
        }
        let remote = self.remote.clone();
        let cache_key = self.cache_key.clone();
        let user_id = self.account_user_id.clone();
        let epoch = self.account_epoch.clone();
        let existing = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                RemoteMediaDiskCache::shared().existing_local_url(
                    &remote,
                    Some(&cache_key),
                    user_id.as_deref(),
                    epoch.as_deref(),
                )
            })
            .await
            .map_err(|_| RemoteMediaTransferError::from(CancellationError))?;
        if let Some(existing) = existing {
            self.state.with(|state| state.committed_url = Some(existing.clone()));
            return Ok(existing);
        }
        self.prepare_if_needed()?;

        let identifier = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel();
        let settled = self.state.with(|state| {
            if let Some(failure) = &state.failure {
                return Some(Err(failure.clone()));
            }
            if let Some(committed_url) = &state.committed_url {
                return Some(Ok(committed_url.clone()));
            }
            state.completion_requested = true;
            state.completion_waiters.insert(identifier, sender);
            None
        });
        if let Some(settled) = settled {
            return settled;
        }
        self.ensure_fetch(cx);

        let guard = self.waiter_guard(identifier, RiseMediaStreamSessionWaiterKind::Completion);
        let resumed = receiver.await;
        guard.disarm();
        resumed.unwrap_or_else(|_| Err(CancellationError.into()))
    }

    pub async fn downloaded_byte_count(&self) -> i64 {
        self.state.with(|state| state.downloaded_byte_count())
    }

    pub async fn declared_byte_count(&self) -> i64 {
        self.state.with(|state| state.content_length.unwrap_or(0))
    }

    /// stops the transfer for everyone but keeps partial bytes and manifest so the next tap resumes
    /// a consumer walking away drops only its own waiter
    pub async fn cancel(&self) {
        self.fail(RiseMediaStreamError::Cancelled.into(), false);
    }

    pub async fn invalidate_for_account_change(&self) {
        self.fail(RiseMediaStreamError::AccountChanged.into(), true);
    }

    /// error a consumer sees, stream errors come back as
    /// stream error and other cases keep their own type
    fn thrown(error: RemoteMediaTransferError) -> anyhow::Error {
        if let RemoteMediaTransferError::Other(other) = &error
            && let Some(stream_error) = other.downcast_ref::<RiseMediaStreamError>()
        {
            return anyhow::Error::new(*stream_error);
        }
        error.into_anyhow()
    }

    fn waiter_guard(
        &self,
        identifier: Uuid,
        kind: RiseMediaStreamSessionWaiterKind,
    ) -> RiseMediaStreamSessionWaiterGuard {
        RiseMediaStreamSessionWaiterGuard {
            session: self.this.clone(),
            identifier,
            kind,
            armed: true,
        }
    }

    /// failure and end of object answers read gives before and after it waits
    fn read_boundary(
        state: &RiseMediaStreamSessionState,
        offset: i64,
    ) -> Option<Result<Vec<u8>, RemoteMediaTransferError>> {
        if let Some(failure) = &state.failure {
            return Some(Err(failure.clone()));
        }
        if offset < 0 {
            return None;
        }
        match state.content_length {
            Some(content_length) if offset >= content_length => Some(Ok(Vec::new())),
            _ => None,
        }
    }

    fn prepare_if_needed(&self) -> Result<(), RemoteMediaTransferError> {
        let mut files = self.files.lock();
        if files.partial_url.is_some() {
            return Ok(());
        }
        if !SaiAccountCacheScope::is_current_account_epoch(
            self.account_user_id.as_deref(),
            self.account_epoch.as_deref(),
        ) {
            return Err(RiseMediaStreamError::AccountChanged.into());
        }
        let directory = RemoteMediaDiskCache::partials_directory(self.account_user_id.as_deref())?;
        let digest = RemoteMediaDiskCache::key_digest(&self.cache_key);
        let partial = directory.join(format!("{digest}.part"));
        let manifest = directory.join(format!("{digest}.part.json"));
        files.partial_url = Some(partial.clone());
        files.manifest_url = Some(manifest.clone());
        if let Some(stored) = self.restore_manifest(&partial, &manifest) {
            let covered = Self::normalized(stored.ranges);
            self.state.with(|state| {
                state.covered = covered;
                state.content_length = Some(stored.content_length);
                state.content_type = stored.content_type;
            });
        }

        if !FileManager.file_exists(&partial) {
            FileManager.create_file(&partial, None);
        }
        let write_handle = OpenOptions::new()
            .write(true)
            .open(&partial)
            .map_err(|error| Self::file_error(error, &partial, CocoaErrorCode::FileWriteUnknown))?;
        let read_handle = File::open(&partial)
            .map_err(|error| Self::file_error(error, &partial, CocoaErrorCode::FileReadUnknown))?;
        files.write_handle = Some(write_handle);
        files.read_handle = Some(read_handle);
        Ok(())
    }

    fn restore_manifest(&self, partial: &Path, manifest: &Path) -> Option<RiseMediaPartialManifest> {
        let restored = std::fs::read(manifest)
            .ok()
            .and_then(|data| serde_json::from_slice::<RiseMediaPartialManifest>(&data).ok())
            .filter(|stored| {
                stored.source_digest == RemoteMediaDiskCache::key_digest(&self.cache_key) && stored.content_length > 0
            })
            .filter(|stored| {
                let Ok(attributes) = FileManager.attributes_of_item(partial) else {
                    return false;
                };
                let Some(highest) = stored.ranges.iter().map(|range| range.end).max() else {
                    return false;
                };
                attributes.size >= highest
            });
        if restored.is_none() {
            FileManager.remove_item(manifest).ok();
            FileManager.remove_item(partial).ok();
        }
        restored
    }

    fn persist_manifest(&self, force: bool) {
        let Some(manifest_url) = self.files.lock().manifest_url.clone() else {
            return;
        };
        let source_digest = RemoteMediaDiskCache::key_digest(&self.cache_key);
        let now = RiseClock::monotonic().as_secs_f64();
        let manifest = self.state.with(|state| {
            let content_length = state.content_length.filter(|length| *length > 0)?;
            if !force && now - state.last_manifest_write < Self::MANIFEST_WRITE_INTERVAL {
                return None;
            }
            state.last_manifest_write = now;
            Some(RiseMediaPartialManifest {
                source_digest,
                content_length,
                content_type: state.content_type.clone(),
                ranges: state.covered.clone(),
            })
        });
        let Some(manifest) = manifest else {
            return;
        };
        let Ok(data) = serde_json::to_vec(&manifest) else {
            return;
        };
        let staging = manifest_url.with_extension(format!("json-{}", Uuid::new_v4().uuid_string().to_lowercase()));
        if std::fs::write(&staging, &data).is_err() {
            FileManager.remove_item(&staging).ok();
            return;
        }
        if std::fs::rename(&staging, &manifest_url).is_err() {
            FileManager.remove_item(&staging).ok();
        }
    }

    fn read_covered(&self, offset: i64, max_length: i64) -> Option<Vec<u8>> {
        let range = self
            .state
            .with(|state| state.covered.iter().find(|range| range.contains(offset)).copied())?;
        let available = range.end - offset;
        let length = available.min(max_length).min(Self::MAXIMUM_READ_CHUNK);
        if length <= 0 {
            return None;
        }
        let position = u64::try_from(offset).ok()?;
        let limit = u64::try_from(length).ok()?;
        let mut files = self.files.lock();
        let read_handle = files.read_handle.as_mut()?;
        read_handle.seek(SeekFrom::Start(position)).ok()?;
        let mut data = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
        std::io::Read::by_ref(read_handle).take(limit).read_to_end(&mut data).ok()?;
        if data.is_empty() {
            return None;
        }
        Some(data)
    }

    fn append(&self, data: &[u8], offset: i64) {
        if data.is_empty() {
            return;
        }
        let Ok(position) = u64::try_from(offset) else {
            return;
        };
        let written = {
            let mut files = self.files.lock();
            let partial = files.partial_url.clone().unwrap_or_default();
            let Some(write_handle) = files.write_handle.as_mut() else {
                return;
            };
            write_handle
                .seek(SeekFrom::Start(position))
                .and_then(|_| write_handle.write_all(data))
                .map_err(|error| Self::file_error(error, &partial, CocoaErrorCode::FileWriteUnknown))
        };
        if let Err(error) = written {
            self.fail(error, false);
            return;
        }
        let end = offset + i64::try_from(data.len()).unwrap_or(i64::MAX);
        self.state.with(|state| state.merge(RiseMediaByteRange { start: offset, end }));
    }

    fn normalized(ranges: Vec<RiseMediaByteRange>) -> Vec<RiseMediaByteRange> {
        let mut sorted: Vec<RiseMediaByteRange> = ranges.into_iter().filter(|range| range.length() > 0).collect();
        sorted.sort_by_key(|range| range.start);
        let mut merged: Vec<RiseMediaByteRange> = Vec::with_capacity(sorted.len());
        for range in sorted {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        merged
    }

    fn file_error(error: std::io::Error, path: &Path, code: CocoaErrorCode) -> RemoteMediaTransferError {
        CocoaError {
            code,
            path: Some(path.to_path_buf()),
            underlying: Some(Arc::new(error)),
        }
        .into()
    }

    fn ensure_fetch(&self, cx: &AsyncApp) {
        enum Decision {
            Keep,
            FinishIfComplete,
            Start(i64, Option<Task<()>>),
        }
        let decision = self.state.with(|state| {
            if state.failure.is_some() || state.committed_url.is_some() {
                return Decision::Keep;
            }
            let Some(target) = state.next_target() else {
                if state.fetch_task.is_none() {
                    return Decision::FinishIfComplete;
                }
                return Decision::Keep;
            };
            if state.fetch_task.is_some() {
                if target >= state.fetch_head && target - state.fetch_head <= Self::SEEK_TOLERANCE {
                    return Decision::Keep;
                }
                return Decision::Start(target, state.fetch_task.take());
            }
            Decision::Start(target, None)
        });
        match decision {
            Decision::Keep => {}
            Decision::FinishIfComplete => self.finish_if_complete(),
            Decision::Start(target, superseded) => {
                drop(superseded);
                self.start_fetch(target, cx);
            }
        }
    }

    fn start_fetch(&self, offset: i64, cx: &AsyncApp) {
        if !SaiAccountCacheScope::is_current_account_epoch(
            self.account_user_id.as_deref(),
            self.account_epoch.as_deref(),
        ) {
            self.fail(RiseMediaStreamError::AccountChanged.into(), true);
            return;
        }
        let (generation, expected_length) = self.state.with(|state| {
            state.fetch_generation = state.fetch_generation.wrapping_add(1);
            state.fetch_head = offset;
            (state.fetch_generation, state.content_length)
        });
        let this = self.this.clone();
        let remote = self.remote.clone();
        let fetcher = Arc::clone(&self.fetcher);
        let task = cx.spawn(async move |cx| {
            let mut request = URLRequest::new(remote.clone());
            request.timeout_interval = Self::REQUEST_TIMEOUT;
            request.http_method = reqwest::Method::GET.as_str().to_owned();
            request.set_value("identity", reqwest::header::ACCEPT_ENCODING.as_str());
            request.set_value(&format!("bytes={offset}-"), reqwest::header::RANGE.as_str());
            if let Some(bearer) = RemoteMediaDiskCache::bearer_token(&remote, None, cx).await {
                request.set_value(&format!("Bearer {bearer}"), reqwest::header::AUTHORIZATION.as_str());
            }
            let Some(session) = this.upgrade() else {
                return;
            };
            if !session.is_current_fetch(generation) {
                return;
            }

            let (sender, mut events) = mpsc::unbounded::<RiseMediaStreamSessionFetchEvent>();
            let sink = Arc::new(Mutex::new(Some(sender)));
            let response_sink = Arc::clone(&sink);
            let chunk_sink = Arc::clone(&sink);
            let on_response: RiseProgressiveDataFetcherOnResponse =
                Arc::new(move |response: &RemoteMediaHttpResponse| {
                    let accepted = Self::acceptable_status(response.status_code);
                    if accepted && let Some(sender) = response_sink.lock().as_ref() {
                        sender
                            .unbounded_send(RiseMediaStreamSessionFetchEvent::Response(response.clone()))
                            .ok();
                    }
                    accepted
                });
            let on_chunk: RiseProgressiveDataFetcherOnChunk = Arc::new(move |chunk: &[u8]| {
                if let Some(sender) = chunk_sink.lock().as_ref() {
                    sender
                        .unbounded_send(RiseMediaStreamSessionFetchEvent::Chunk(chunk.to_vec()))
                        .ok();
                }
            });

            let fetch_cx = cx.clone();
            let cx: &AsyncApp = cx;
            let fetch = async {
                let result = fetcher(request, on_response, on_chunk, fetch_cx).await;
                sink.lock().take();
                result
            };
            let consumer = async {
                while let Some(event) = events.next().await {
                    match event {
                        RiseMediaStreamSessionFetchEvent::Response(response) => {
                            session.adopt_response(&response, offset, expected_length, generation, cx);
                        }
                        RiseMediaStreamSessionFetchEvent::Chunk(data) => {
                            session.consume(&data, generation, cx);
                        }
                    }
                }
            };
            let (result, ()) = futures::join!(fetch, consumer);
            session.finish_fetch(result.err(), generation, cx);
        });
        let replaced = self.state.with(|state| state.fetch_task.replace(task));
        drop(replaced);
    }

    fn is_current_fetch(&self, generation: u64) -> bool {
        self.state.with(|state| {
            generation == state.fetch_generation && state.failure.is_none() && state.committed_url.is_none()
        })
    }

    fn acceptable_status(status_code: i64) -> bool {
        status_code == 200 || status_code == 206
    }

    fn adopt_response(
        &self,
        response: &RemoteMediaHttpResponse,
        requested_offset: i64,
        previous_length: Option<i64>,
        generation: u64,
        cx: &AsyncApp,
    ) {
        enum Adoption {
            Stale,
            ObjectChanged(Option<Task<()>>),
            TooLarge,
            Adopted,
        }
        let total = Self::total_length(response).or(previous_length);
        let header_type = response
            .value(reqwest::header::CONTENT_TYPE.as_str())
            .and_then(|value| value.splitn(2, ';').find(|part| !part.is_empty()))
            .map(|value| trimming_whitespaces(value).to_owned());
        let adoption = self.state.with(|state| {
            if generation != state.fetch_generation {
                return Adoption::Stale;
            }
            if response.status_code == 200 && requested_offset > 0 {
                state.fetch_head = 0;
            }
            if let Some(total) = total
                && total > 0
            {
                if let Some(known) = state.content_length
                    && known != total
                {
                    state.content_length = Some(total);
                    state.fetch_generation = state.fetch_generation.wrapping_add(1);
                    return Adoption::ObjectChanged(state.fetch_task.take());
                }
                state.content_length = Some(total);
                if total > self.maximum_byte_count {
                    return Adoption::TooLarge;
                }
            }
            if state.content_type.is_none() {
                state.content_type = header_type;
            }
            Adoption::Adopted
        });
        match adoption {
            Adoption::Stale => {}
            Adoption::ObjectChanged(superseded) => {
                drop(superseded);
                self.truncate_partial();
                self.resume_information_waiters_if_possible();
                self.persist_manifest(true);
                self.ensure_fetch(cx);
            }
            Adoption::TooLarge => self.fail(RiseMediaStreamError::TooLarge.into(), true),
            Adoption::Adopted => {
                self.resume_information_waiters_if_possible();
                self.persist_manifest(true);
            }
        }
    }

    fn total_length(response: &RemoteMediaHttpResponse) -> Option<i64> {
        if let Some(range) = response.value(reqwest::header::CONTENT_RANGE.as_str())
            && let Some((_, tail)) = range.rsplit_once('/')
            && let Ok(total) = trimming_whitespaces(tail).parse::<i64>()
            && total > 0
        {
            return Some(total);
        }
        if response.status_code == 200 && response.expected_content_length() > 0 {
            return Some(response.expected_content_length());
        }
        None
    }

    fn consume(&self, chunk: &[u8], generation: u64, cx: &AsyncApp) {
        let Some(head) = self.state.with(|state| {
            (generation == state.fetch_generation && state.failure.is_none()).then_some(state.fetch_head)
        }) else {
            return;
        };
        self.append(chunk, head);
        let complete = self.state.with(|state| {
            state.fetch_head += i64::try_from(chunk.len()).unwrap_or(i64::MAX);
            state.consecutive_failures = 0;
            state.is_complete()
        });
        self.persist_manifest(false);
        self.publish_progress(false);
        self.resume_byte_waiters(None);
        if complete {
            self.publish_progress(true);
            self.finish_if_complete();
            return;
        }
        let superseded = self.state.with(|state| {
            if state.lowest_uncovered(state.fetch_head) == Some(state.fetch_head) {
                return None;
            }
            state.fetch_generation = state.fetch_generation.wrapping_add(1);
            Some(state.fetch_task.take())
        });
        if let Some(superseded) = superseded {
            drop(superseded);
            self.ensure_fetch(cx);
        }
    }

    fn finish_fetch(&self, error: Option<RemoteMediaTransferError>, generation: u64, cx: &AsyncApp) {
        enum Outcome {
            Stale,
            Finished,
            Retry(Duration),
            GiveUp(RemoteMediaTransferError),
        }
        let (finished_task, outcome) = self.state.with(|state| {
            if generation != state.fetch_generation {
                return (None, Outcome::Stale);
            }
            let finished_task = state.fetch_task.take();
            let error = match error {
                None | Some(RemoteMediaTransferError::Cancelled(_)) => {
                    state.consecutive_failures = 0;
                    return (finished_task, Outcome::Finished);
                }
                Some(error) => error,
            };
            state.consecutive_failures += 1;
            if state.consecutive_failures <= Self::MAXIMUM_CONSECUTIVE_FAILURES {
                let delay = Duration::from_millis(300 * u64::try_from(state.consecutive_failures).unwrap_or(1));
                return (finished_task, Outcome::Retry(delay));
            }
            (finished_task, Outcome::GiveUp(error))
        });
        drop(finished_task);
        match outcome {
            Outcome::Stale => {}
            Outcome::Finished => {
                self.finish_if_complete();
                self.ensure_fetch(cx);
            }
            Outcome::Retry(delay) => {
                let this = self.this.clone();
                cx.spawn(async move |cx| {
                    RiseAccountRuntime::current()
                        .spawn(async move { tokio::time::sleep(delay).await })
                        .await
                        .ok();
                    if let Some(session) = this.upgrade() {
                        session.ensure_fetch(cx);
                    }
                })
                .detach();
            }
            Outcome::GiveUp(error) => {
                self.resume_byte_waiters(Some(error.clone()));
                self.fail(error, false);
            }
        }
    }

    fn truncate_partial(&self) {
        if let Some(write_handle) = self.files.lock().write_handle.as_ref() {
            write_handle.set_len(0).ok();
        }
        self.state.with(|state| state.covered.clear());
    }

    fn finish_if_complete(&self) {
        let Some(partial_url) = self.files.lock().partial_url.clone() else {
            return;
        };
        let begins = self.state.with(|state| {
            if state.is_committing || state.committed_url.is_some() || !state.is_complete() {
                return None;
            }
            state.is_committing = true;
            Some(state.content_type.clone())
        });
        let Some(content_type) = begins else {
            return;
        };
        if !SaiAccountCacheScope::is_current_account_epoch(
            self.account_user_id.as_deref(),
            self.account_epoch.as_deref(),
        ) {
            self.fail(RiseMediaStreamError::AccountChanged.into(), true);
            return;
        }
        if let Some(write_handle) = self.files.lock().write_handle.as_ref() {
            write_handle.sync_all().ok();
        }
        let this = self.this.clone();
        let remote = self.remote.clone();
        let cache_key = self.cache_key.clone();
        let filename = self.suggested_filename.clone();
        let user_id = self.account_user_id.clone();
        let epoch = self.account_epoch.clone();
        RiseAccountRuntime::current().spawn_blocking(move || {
            let committed = RemoteMediaDiskCache::shared().adopt(
                &partial_url,
                &remote,
                Some(&cache_key),
                filename.as_deref(),
                content_type,
                user_id.as_deref(),
                epoch.as_deref(),
            );
            let Some(session) = this.upgrade() else {
                return;
            };
            match committed {
                Ok(url) => session.did_commit(url),
                Err(error) => session.did_fail_commit(error),
            }
        });
    }

    fn did_commit(&self, url: PathBuf) {
        let (completions, byte_count) = self.state.with(|state| {
            state.committed_url = Some(url.clone());
            (std::mem::take(&mut state.completion_waiters), state.content_length.unwrap_or(0))
        });
        let manifest_url = {
            let mut files = self.files.lock();
            files.write_handle = None;
            files.manifest_url.clone()
        };
        if let Some(manifest_url) = manifest_url {
            FileManager.remove_item(&manifest_url).ok();
        }
        self.retire_from_registry();
        for (_, waiter) in completions {
            waiter.send(Ok(url.clone())).ok();
        }
        self.resume_byte_waiters(None);
        let cache_key = self.cache_key.clone();
        let user_id = self.account_user_id.clone();
        let epoch = self.account_epoch.clone();
        RiseAccountRuntime::current().spawn(async move {
            RiseMediaTransferHub::shared()
                .note(cache_key, byte_count, user_id, epoch)
                .await;
        });
    }

    fn did_fail_commit(&self, error: RemoteMediaTransferError) {
        self.state.with(|state| state.is_committing = false);
        self.fail(error, false);
    }

    /// chunks arrive every few ms so waking the hub per chunk would be a hundred hops a second
    /// the same coalescer as plain download gates this
    fn publish_progress(&self, force: bool) {
        let now = RemoteMediaDownloadProgressCoalescer::uptime();
        let published = self.state.with(|state| {
            let downloaded = state.downloaded_byte_count();
            let total = state.content_length.unwrap_or(0);
            let candidate = RemoteMediaDownloadProgress::new(downloaded, (total > 0).then_some(total));
            if !force && !state.progress_coalescer.should_publish(candidate, now) {
                return None;
            }
            Some((downloaded, total))
        });
        let Some((downloaded, total)) = published else {
            return;
        };
        let cache_key = self.cache_key.clone();
        let user_id = self.account_user_id.clone();
        let epoch = self.account_epoch.clone();
        RiseAccountRuntime::current().spawn(async move {
            RiseMediaTransferHub::shared()
                .ingest_for_account(cache_key, downloaded, total, false, user_id, epoch)
                .await;
        });
    }

    fn resume_information_waiters_if_possible(&self) {
        let resumed = self.state.with(|state| {
            let content_length = state.content_length.filter(|length| *length > 0)?;
            let information = RiseMediaStreamSessionContentInformation {
                content_length,
                content_type: state.content_type.clone(),
            };
            Some((information, std::mem::take(&mut state.information_waiters)))
        });
        let Some((information, waiters)) = resumed else {
            return;
        };
        for (_, waiter) in waiters {
            waiter.send(Ok(information.clone())).ok();
        }
    }

    fn resume_byte_waiters(&self, error: Option<RemoteMediaTransferError>) {
        let resumed = self.state.with(|state| {
            let mut resumed = Vec::new();
            if state.byte_waiters.is_empty() {
                return resumed;
            }
            let waiters = std::mem::take(&mut state.byte_waiters);
            for (identifier, waiter) in waiters {
                let servable = state.covered.iter().any(|range| range.contains(waiter.offset))
                    || state.content_length.is_some_and(|length| waiter.offset >= length);
                if servable {
                    resumed.push((waiter.continuation, Ok(())));
                } else if let Some(error) = &error {
                    resumed.push((waiter.continuation, Err(error.clone())));
                } else {
                    state.byte_waiters.insert(identifier, waiter);
                }
            }
            resumed
        });
        for (continuation, result) in resumed {
            continuation.send(result).ok();
        }
    }

    fn drop_byte_waiter(&self, identifier: Uuid) {
        let Some(waiter) = self.state.with(|state| state.byte_waiters.remove(&identifier)) else {
            return;
        };
        waiter.continuation.send(Err(CancellationError.into())).ok();
        self.stop_fetch_if_unwanted();
    }

    fn drop_information_waiter(&self, identifier: Uuid) {
        if let Some(waiter) = self.state.with(|state| state.information_waiters.remove(&identifier)) {
            waiter.send(Err(CancellationError.into())).ok();
        }
        self.stop_fetch_if_unwanted();
    }

    fn drop_completion_waiter(&self, identifier: Uuid) {
        if let Some(waiter) = self.state.with(|state| state.completion_waiters.remove(&identifier)) {
            waiter.send(Err(CancellationError.into())).ok();
        }
    }

    /// nobody watches and nobody wants the whole object so stop fetching, partial stays on disk
    fn stop_fetch_if_unwanted(&self) {
        let stopped = self.state.with(|state| {
            if state.completion_requested || !state.byte_waiters.is_empty() || !state.information_waiters.is_empty() {
                return None;
            }
            Some(state.fetch_task.take())
        });
        let Some(stopped) = stopped else {
            return;
        };
        drop(stopped);
        self.persist_manifest(true);
    }

    fn fail(&self, error: RemoteMediaTransferError, discarding_partial: bool) {
        let taken = self.state.with(|state| {
            if state.failure.is_some() || state.committed_url.is_some() {
                return None;
            }
            state.failure = Some(error.clone());
            if discarding_partial {
                state.covered.clear();
            }
            Some((
                state.fetch_task.take(),
                std::mem::take(&mut state.information_waiters),
                std::mem::take(&mut state.byte_waiters),
                std::mem::take(&mut state.completion_waiters),
            ))
        });
        let Some((fetch_task, information, bytes, completions)) = taken else {
            return;
        };
        drop(fetch_task);
        let (partial_url, manifest_url) = {
            let mut files = self.files.lock();
            files.write_handle = None;
            files.read_handle = None;
            (files.partial_url.clone(), files.manifest_url.clone())
        };
        if discarding_partial {
            if let Some(partial_url) = partial_url {
                FileManager.remove_item(&partial_url).ok();
            }
            if let Some(manifest_url) = manifest_url {
                FileManager.remove_item(&manifest_url).ok();
            }
        } else {
            self.persist_manifest(true);
        }

        for (_, waiter) in information {
            waiter.send(Err(error.clone())).ok();
        }
        for (_, waiter) in bytes {
            waiter.continuation.send(Err(error.clone())).ok();
        }
        for (_, waiter) in completions {
            waiter.send(Err(error.clone())).ok();
        }
        self.retire_from_registry();
    }

    /// a finished session must not stay discoverable, the next consumer should find the cache entry
    fn retire_from_registry(&self) {
        let cache_key = self.cache_key.clone();
        let epoch = self.account_epoch.clone();
        RiseAccountRuntime::current().spawn(async move {
            RiseMediaStreamRegistry::shared()
                .remove(&cache_key, epoch.as_deref())
                .await;
        });
    }
}

impl RiseMediaStreamSessionState {
    fn downloaded_byte_count(&self) -> i64 {
        self.covered.iter().map(RiseMediaByteRange::length).sum()
    }

    fn merge(&mut self, range: RiseMediaByteRange) {
        let mut ranges = std::mem::take(&mut self.covered);
        ranges.push(range);
        self.covered = RiseMediaStreamSession::normalized(ranges);
    }

    fn is_complete(&self) -> bool {
        let Some(content_length) = self.content_length.filter(|length| *length > 0) else {
            return false;
        };
        let Some(first) = self.covered.first() else {
            return false;
        };
        self.covered.len() == 1 && first.start == 0 && first.end >= content_length
    }

    fn lowest_uncovered(&self, offset: i64) -> Option<i64> {
        let Some(content_length) = self.content_length else {
            return Some(offset);
        };
        if offset >= content_length {
            return None;
        }
        let mut cursor = offset;
        for range in self.covered.iter() {
            if range.end <= cursor {
                continue;
            }
            if range.start > cursor {
                return Some(cursor);
            }
            cursor = range.end;
        }
        (cursor < content_length).then_some(cursor)
    }

    fn next_target(&self) -> Option<i64> {
        let blocked = self
            .byte_waiters
            .values()
            .filter_map(|waiter| self.lowest_uncovered(waiter.offset))
            .min();
        if blocked.is_some() {
            return blocked;
        }
        if self.content_length.is_none() && !self.information_waiters.is_empty() {
            return Some(self.lowest_uncovered(0).unwrap_or(0));
        }
        if !self.completion_requested {
            return None;
        }
        self.lowest_uncovered(0)
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct RiseMediaStreamRegistryKey {
    cache_key: String,
    account_epoch: Option<String>,
}

/// one session per cached object so the player and download chip never get two
pub struct RiseMediaStreamRegistry {
    sessions: RiseIsolated<HashMap<RiseMediaStreamRegistryKey, Arc<RiseMediaStreamSession>>>,
}

static SHARED: LazyLock<RiseMediaStreamRegistry> = LazyLock::new(|| RiseMediaStreamRegistry {
    sessions: RiseIsolated::new(HashMap::new()),
});

impl RiseMediaStreamRegistry {
    pub fn shared() -> &'static RiseMediaStreamRegistry {
        &SHARED
    }

    pub async fn session(
        &self,
        cache_key: String,
        remote: Url,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        maximum_byte_count: i64,
        suggested_filename: Option<String>,
    ) -> Arc<RiseMediaStreamSession> {
        let key = RiseMediaStreamRegistryKey {
            cache_key: cache_key.clone(),
            account_epoch: account_epoch.clone(),
        };
        self.sessions.with(|sessions| {
            Arc::clone(sessions.entry(key).or_insert_with(|| {
                RiseMediaStreamSession::new(
                    &cache_key,
                    remote,
                    account_user_id,
                    account_epoch,
                    maximum_byte_count,
                    suggested_filename,
                    None,
                )
            }))
        })
    }

    /// drops the session without touching it, for one that finished and waits to be forgotten
    pub async fn remove(&self, cache_key: &str, account_epoch: Option<&str>) {
        let key = RiseMediaStreamRegistryKey {
            cache_key: cache_key.to_owned(),
            account_epoch: account_epoch.map(str::to_owned),
        };
        let removed = self.sessions.with(|sessions| sessions.remove(&key));
        drop(removed);
    }

    pub async fn cancel(&self, cache_key: &str, account_epoch: Option<&str>) {
        let key = RiseMediaStreamRegistryKey {
            cache_key: cache_key.to_owned(),
            account_epoch: account_epoch.map(str::to_owned),
        };
        let Some(session) = self.sessions.with(|sessions| sessions.remove(&key)) else {
            return;
        };
        session.cancel().await;
    }

    pub async fn reset_account_state(&self) {
        let open: Vec<Arc<RiseMediaStreamSession>> =
            self.sessions.with(|sessions| std::mem::take(sessions).into_values().collect());
        for session in open {
            session.invalidate_for_account_change().await;
        }
    }
}
