use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Weak;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures::StreamExt as _;
use gpui::App;
use parking_lot::{Condvar, Mutex};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use thiserror::Error;

/// why a byte range could not be read through the loader
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum AVAssetReadError {
    #[error("no resource loader delegate accepted the loading request")]
    Declined,
    #[error("the resource loader did not answer within {0:?}")]
    TimedOut(Duration),
    #[error("the loading request was cancelled")]
    Cancelled,
    #[error("the resource loader failed the loading request: {0}")]
    Failed(String),
    #[error("the asset is gone: its resource loader no longer takes requests")]
    LoaderGone,
}

/// fields of the resource loading content information request a delegate fills
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ContentInformation {
    pub is_byte_range_access_supported: bool,
    pub content_length: i64,
    pub content_type: Option<String>,
}

/// range of a resource loading data request
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DataRange {
    pub requested_offset: i64,
    pub requested_length: i64,
    pub requests_all_data_to_end_of_resource: bool,
}

impl DataRange {
    /// closed range of requested_length bytes from requested_offset
    pub const fn bytes(requested_offset: i64, requested_length: i64) -> Self {
        Self { requested_offset, requested_length, requests_all_data_to_end_of_resource: false }
    }

    /// everything from requested_offset to the end of the resource
    pub const fn to_end(requested_offset: i64) -> Self {
        Self { requested_offset, requested_length: i64::MAX, requests_all_data_to_end_of_resource: true }
    }
}

/// backend side of one loading request, where the answers the delegate gives go
/// macos forwards them to the system request, byte source collects them for a waiting decoder thread
/// called from the main thread, never with a request lock held
pub trait LoadingRequestBackend: Send + Sync {
    fn set_content_information(&self, information: &ContentInformation);
    fn respond(&self, data: &[u8]);
    /// finishes the request with ok or an error
    /// called at most once
    fn finish(&self, outcome: Result<(), anyhow::Error>);
}

struct RequestState {
    content_information: ContentInformation,
    current_offset: i64,
    is_finished: bool,
    is_cancelled: bool,
}

struct RequestInner {
    identifier: u64,
    asks_content_information: bool,
    data_range: Option<DataRange>,
    state: Mutex<RequestState>,
    backend: Box<dyn LoadingRequestBackend>,
}

static NEXT_REQUEST_IDENTIFIER: AtomicU64 = AtomicU64::new(1);
static NEXT_LOADER_IDENTIFIER: AtomicU64 = AtomicU64::new(1);

/// one request a backend makes of the loader
/// clones share the request
#[derive(Clone)]
pub struct AVAssetResourceLoadingRequest {
    inner: Arc<RequestInner>,
}

impl fmt::Debug for AVAssetResourceLoadingRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AVAssetResourceLoadingRequest")
            .field("identifier", &self.inner.identifier)
            .field("asks_content_information", &self.inner.asks_content_information)
            .field("data_range", &self.inner.data_range)
            .finish()
    }
}

impl AVAssetResourceLoadingRequest {
    /// request a backend is about to hand to submit
    /// asks for content info or a data range or both, like the first avfoundation request
    pub fn new(
        asks_content_information: bool,
        data_range: Option<DataRange>,
        backend: Box<dyn LoadingRequestBackend>,
    ) -> Self {
        let current_offset = data_range.map_or(0, |range| range.requested_offset);
        Self {
            inner: Arc::new(RequestInner {
                identifier: NEXT_REQUEST_IDENTIFIER.fetch_add(1, Ordering::Relaxed),
                asks_content_information,
                data_range,
                state: Mutex::new(RequestState {
                    content_information: ContentInformation::default(),
                    current_offset,
                    is_finished: false,
                    is_cancelled: false,
                }),
                backend,
            }),
        }
    }

    /// stable for the life of the request
    pub fn identifier(&self) -> u64 {
        self.inner.identifier
    }

    /// some when the backend asked for the content information
    pub fn content_information_request(&self) -> Option<AVAssetResourceLoadingContentInformationRequest> {
        self.inner
            .asks_content_information
            .then(|| AVAssetResourceLoadingContentInformationRequest { request: self.clone() })
    }

    /// some when the backend asked for bytes
    pub fn data_request(&self) -> Option<AVAssetResourceLoadingDataRequest> {
        self.inner.data_range.map(|range| AVAssetResourceLoadingDataRequest { request: self.clone(), range })
    }

    /// true once the request is finished
    pub fn is_finished(&self) -> bool {
        self.inner.state.lock().is_finished
    }

    /// true once the request is cancelled
    pub fn is_cancelled(&self) -> bool {
        self.inner.state.lock().is_cancelled
    }

    /// content information the delegate has set so far
    pub fn content_information(&self) -> ContentInformation {
        self.inner.state.lock().content_information.clone()
    }

    /// finishes loading, a finished or cancelled request ignores it
    pub fn finish_loading(&self) {
        if self.close() {
            self.inner.backend.finish(Ok(()));
        }
    }

    pub fn finish_loading_with_error(&self, error: anyhow::Error) {
        if self.close() {
            self.inner.backend.finish(Err(error));
        }
    }

    /// marks the request finished, false if it already was finished or cancelled
    fn close(&self) -> bool {
        let mut state = self.inner.state.lock();
        if state.is_finished || state.is_cancelled {
            return false;
        }
        state.is_finished = true;
        true
    }

    fn mark_cancelled(&self) -> bool {
        let mut state = self.inner.state.lock();
        if state.is_finished || state.is_cancelled {
            return false;
        }
        state.is_cancelled = true;
        true
    }

    fn update_content_information(&self, update: impl FnOnce(&mut ContentInformation)) {
        let information = {
            let mut state = self.inner.state.lock();
            if state.is_finished || state.is_cancelled {
                return;
            }
            update(&mut state.content_information);
            state.content_information.clone()
        };
        self.inner.backend.set_content_information(&information);
    }
}

/// part of a request the delegate fills with what the resource is
#[derive(Clone, Debug)]
pub struct AVAssetResourceLoadingContentInformationRequest {
    request: AVAssetResourceLoadingRequest,
}

impl AVAssetResourceLoadingContentInformationRequest {
    pub fn set_is_byte_range_access_supported(&self, supported: bool) {
        self.request
            .update_content_information(|information| information.is_byte_range_access_supported = supported);
    }

    pub fn set_content_length(&self, content_length: i64) {
        self.request.update_content_information(|information| information.content_length = content_length);
    }

    /// uti of the resource, eg public.mpeg-4
    pub fn set_content_type(&self, content_type: Option<String>) {
        self.request.update_content_information(|information| information.content_type = content_type);
    }

    pub fn is_byte_range_access_supported(&self) -> bool {
        self.request.content_information().is_byte_range_access_supported
    }

    pub fn content_length(&self) -> i64 {
        self.request.content_information().content_length
    }

    pub fn content_type(&self) -> Option<String> {
        self.request.content_information().content_type
    }
}

/// byte range a request wants and the bytes given so far
#[derive(Clone, Debug)]
pub struct AVAssetResourceLoadingDataRequest {
    request: AVAssetResourceLoadingRequest,
    range: DataRange,
}

impl AVAssetResourceLoadingDataRequest {
    pub fn requested_offset(&self) -> i64 {
        self.range.requested_offset
    }

    pub fn requested_length(&self) -> i64 {
        self.range.requested_length
    }

    pub fn requests_all_data_to_end_of_resource(&self) -> bool {
        self.range.requests_all_data_to_end_of_resource
    }

    /// offset of the next byte respond gives, the requested offset plus every byte given so far
    pub fn current_offset(&self) -> i64 {
        self.request.inner.state.lock().current_offset
    }

    pub fn respond(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        {
            let mut state = self.request.inner.state.lock();
            if state.is_finished || state.is_cancelled {
                return;
            }
            let given = i64::try_from(data.len()).unwrap_or(i64::MAX);
            state.current_offset = state.current_offset.saturating_add(given);
        }
        self.request.inner.backend.respond(data);
    }
}

/// not Send, the loader calls it only on the gpui main thread
/// carrying every request over from the backend thread that made it
pub trait AVAssetResourceLoaderDelegate {
    fn should_wait_for_loading_of_requested_resource(
        &self,
        loading_request: AVAssetResourceLoadingRequest,
        cx: &mut App,
    ) -> bool;

    fn did_cancel(&self, loading_request: AVAssetResourceLoadingRequest, cx: &mut App);
}

/// what a backend hands the main thread
pub(crate) enum LoaderEvent {
    Load(AVAssetResourceLoadingRequest),
    Cancel(AVAssetResourceLoadingRequest),
}

struct LoaderShared {
    identifier: u64,
    events: UnboundedSender<LoaderEvent>,
    pending_events: Mutex<Option<UnboundedReceiver<LoaderEvent>>>,
}

thread_local! {
    static DELEGATES: RefCell<HashMap<u64, Weak<dyn AVAssetResourceLoaderDelegate>>> =
        RefCell::new(HashMap::new());
}

/// takes requests from the player backend on any thread
/// hands them to the delegate on the main thread, clones share the loader
#[derive(Clone)]
pub struct AVAssetResourceLoader {
    shared: Arc<LoaderShared>,
}

impl fmt::Debug for AVAssetResourceLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("AVAssetResourceLoader").field("identifier", &self.shared.identifier).finish()
    }
}

impl AVAssetResourceLoader {
    fn new() -> Self {
        let (events, pending_events) = mpsc::unbounded();
        Self {
            shared: Arc::new(LoaderShared {
                identifier: NEXT_LOADER_IDENTIFIER.fetch_add(1, Ordering::Relaxed),
                events,
                pending_events: Mutex::new(Some(pending_events)),
            }),
        }
    }

    pub fn set_delegate(&self, delegate: Weak<dyn AVAssetResourceLoaderDelegate>) {
        DELEGATES.with(|delegates| {
            let mut delegates = delegates.borrow_mut();
            delegates.retain(|_, held| held.strong_count() > 0);
            delegates.insert(self.shared.identifier, delegate);
        });
    }

    /// starts handing requests to the delegate on the main thread
    /// the player calls it when it opens an item of this asset, a second call does nothing
    /// requests made before it wait
    pub fn start(&self, cx: &mut App) {
        let Some(mut events) = self.take_events() else {
            return;
        };
        let loader = self.shared.identifier;
        cx.spawn(async move |cx| {
            while let Some(event) = events.next().await {
                cx.update(|cx| dispatch(loader, event, cx));
            }
            DELEGATES.with(|delegates| {
                delegates.borrow_mut().remove(&loader);
            });
        })
        .detach();
    }

    pub(crate) fn take_events(&self) -> Option<UnboundedReceiver<LoaderEvent>> {
        self.shared.pending_events.lock().take()
    }

    /// hands a backend request to the delegate, any thread
    pub fn submit(&self, request: AVAssetResourceLoadingRequest) {
        if self.shared.events.unbounded_send(LoaderEvent::Load(request.clone())).is_err() {
            request.finish_loading_with_error(anyhow::Error::new(AVAssetReadError::LoaderGone));
        }
    }

    /// backend no longer wants the request
    /// it is marked cancelled at once so later answers are dropped
    /// delegate hears did_cancel on the main thread, any thread
    pub fn cancel(&self, request: &AVAssetResourceLoadingRequest) {
        if request.mark_cancelled() {
            let _ = self.shared.events.unbounded_send(LoaderEvent::Cancel(request.clone()));
        }
    }

    /// blocking reader over this loader for a decoder thread
    pub fn byte_source(&self) -> AVAssetByteSource {
        AVAssetByteSource { loader: self.clone(), timeout: AVAssetByteSource::DEFAULT_TIMEOUT }
    }
}

/// one event on the main thread, a request goes to the delegate or fails when no delegate takes it
fn dispatch(loader: u64, event: LoaderEvent, cx: &mut App) {
    let delegate = DELEGATES.with(|delegates| delegates.borrow().get(&loader).and_then(Weak::upgrade));
    match event {
        LoaderEvent::Load(request) => {
            if request.is_finished() || request.is_cancelled() {
                return;
            }
            let accepted = delegate
                .is_some_and(|delegate| delegate.should_wait_for_loading_of_requested_resource(request.clone(), cx));
            if !accepted {
                request.finish_loading_with_error(anyhow::Error::new(AVAssetReadError::Declined));
            }
        }
        LoaderEvent::Cancel(request) => {
            if let Some(delegate) = delegate {
                delegate.did_cancel(request, cx);
            }
        }
    }
}

struct AssetInner {
    url: Url,
    resource_loader: AVAssetResourceLoader,
}

/// a url plus the loader the player asks for bytes when the scheme is the app own
/// clones are the same asset, equality and hashing are by identity
#[derive(Clone)]
pub struct AVURLAsset {
    inner: Arc<AssetInner>,
}

impl AVURLAsset {
    pub fn new(url: Url) -> Self {
        Self { inner: Arc::new(AssetInner { url, resource_loader: AVAssetResourceLoader::new() }) }
    }

    pub fn url(&self) -> &Url {
        &self.inner.url
    }

    pub fn resource_loader(&self) -> AVAssetResourceLoader {
        self.inner.resource_loader.clone()
    }
}

impl PartialEq for AVURLAsset {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for AVURLAsset {}

impl Hash for AVURLAsset {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.inner).hash(state);
    }
}

impl fmt::Debug for AVURLAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("AVURLAsset").field("url", &self.inner.url.absolute_string()).finish()
    }
}

#[derive(Default)]
struct Collected {
    information: Option<ContentInformation>,
    bytes: Vec<u8>,
    outcome: Option<Result<(), String>>,
}

/// collects the answers to one request for the thread waiting on it
#[derive(Default)]
struct Collector {
    collected: Mutex<Collected>,
    changed: Condvar,
}

struct CollectorBackend(Arc<Collector>);

impl LoadingRequestBackend for CollectorBackend {
    fn set_content_information(&self, information: &ContentInformation) {
        self.0.collected.lock().information = Some(information.clone());
    }

    fn respond(&self, data: &[u8]) {
        self.0.collected.lock().bytes.extend_from_slice(data);
    }

    fn finish(&self, outcome: Result<(), anyhow::Error>) {
        self.0.collected.lock().outcome = Some(outcome.map_err(|error| format!("{error:#}")));
        self.0.changed.notify_all();
    }
}

/// blocking reader over an asset loader for an ffmpeg custom io context or a media foundation stream
/// never call it on the main thread
/// the delegate answers there so waiting would deadlock
#[derive(Clone, Debug)]
pub struct AVAssetByteSource {
    loader: AVAssetResourceLoader,
    timeout: Duration,
}

impl AVAssetByteSource {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn loader(&self) -> &AVAssetResourceLoader {
        &self.loader
    }

    /// content information alone, the first thing a backend asks
    pub fn content_information(&self) -> Result<ContentInformation, AVAssetReadError> {
        let (information, _) = self.request(true, None)?;
        Ok(information.unwrap_or_default())
    }

    /// up to length bytes from offset
    /// fewer at the end of the resource and none past it
    pub fn read_at(&self, offset: i64, length: usize) -> Result<Vec<u8>, AVAssetReadError> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let requested_length = i64::try_from(length).unwrap_or(i64::MAX);
        let (_, mut bytes) = self.request(false, Some(DataRange::bytes(offset, requested_length)))?;
        bytes.truncate(length);
        Ok(bytes)
    }

    fn request(
        &self,
        asks_content_information: bool,
        data_range: Option<DataRange>,
    ) -> Result<(Option<ContentInformation>, Vec<u8>), AVAssetReadError> {
        let collector = Arc::new(Collector::default());
        let request = AVAssetResourceLoadingRequest::new(
            asks_content_information,
            data_range,
            Box::new(CollectorBackend(Arc::clone(&collector))),
        );
        self.loader.submit(request.clone());
        let deadline = RiseClock::monotonic().saturating_add(self.timeout);
        let mut collected = collector.collected.lock();
        loop {
            if let Some(outcome) = collected.outcome.take() {
                return match outcome {
                    Ok(()) => Ok((collected.information.take(), std::mem::take(&mut collected.bytes))),
                    Err(message) => Err(read_error(message)),
                };
            }
            if request.is_cancelled() {
                return Err(AVAssetReadError::Cancelled);
            }
            let remaining = deadline.saturating_sub(RiseClock::monotonic());
            if remaining.is_zero() {
                drop(collected);
                self.loader.cancel(&request);
                return Err(AVAssetReadError::TimedOut(self.timeout));
            }
            let _ = collector.changed.wait_for(&mut collected, remaining);
        }
    }
}

fn read_error(message: String) -> AVAssetReadError {
    for known in [AVAssetReadError::Declined, AVAssetReadError::LoaderGone] {
        if message == known.to_string() {
            return known;
        }
    }
    AVAssetReadError::Failed(message)
}

pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub const SEEK_END: i32 = 2;
pub const AVSEEK_SIZE: i32 = 0x10000;
pub const AVSEEK_FORCE: i32 = 0x20000;

/// what an ffmpeg seek callback answers, the new position or the size for AVSEEK_SIZE
/// none for a seek it cant make (unknown size or before the start)
pub fn avio_seek_target(offset: i64, whence: i32, position: i64, content_length: Option<i64>) -> Option<i64> {
    let whence = whence & !AVSEEK_FORCE;
    let target = match whence {
        AVSEEK_SIZE => return content_length,
        SEEK_SET => offset,
        SEEK_CUR => position.checked_add(offset)?,
        SEEK_END => content_length?.checked_add(offset)?,
        _ => return None,
    };
    (target >= 0).then_some(target)
}
