use std::ops::Range;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PhotoLibraryAccessLevel {
    Authorized,
    Limited,
    Denied,
    NotDetermined,
    Restricted,
}

impl PhotoLibraryAccessLevel {
    /// raw photos authorization values, an unknown future value is Denied
    pub const fn from_ph_status(status: i64) -> Self {
        match status {
            0 => Self::NotDetermined,
            1 => Self::Restricted,
            2 => Self::Denied,
            3 => Self::Authorized,
            4 => Self::Limited,
            _ => Self::Denied,
        }
    }

    pub const fn can_browse(self) -> bool {
        matches!(self, Self::Authorized | Self::Limited)
    }

    /// true when asking shows the system prompt, otherwise the feature opens system settings
    pub const fn can_request(self) -> bool {
        matches!(self, Self::NotDetermined)
    }
}

/// which assets the grid lists
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LibraryMediaKind {
    Images,
    Videos,
    All,
}

impl LibraryMediaKind {
    pub const fn admits(self, kind: LibraryAssetKind) -> bool {
        match self {
            Self::Images => matches!(kind, LibraryAssetKind::Image),
            Self::Videos => matches!(kind, LibraryAssetKind::Video),
            Self::All => true,
        }
    }

    /// media type values the fetch predicate matches, image 1 and video 2
    pub const fn ph_media_types(self) -> &'static [i64] {
        match self {
            Self::Images => &[PH_MEDIA_TYPE_IMAGE],
            Self::Videos => &[PH_MEDIA_TYPE_VIDEO],
            Self::All => &[PH_MEDIA_TYPE_IMAGE, PH_MEDIA_TYPE_VIDEO],
        }
    }
}

pub const PH_MEDIA_TYPE_IMAGE: i64 = 1;
pub const PH_MEDIA_TYPE_VIDEO: i64 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LibraryAssetKind {
    Image,
    Video,
}

impl LibraryAssetKind {
    /// video is video, anything else is shown as an image
    pub const fn from_ph_media_type(media_type: i64) -> Self {
        if media_type == PH_MEDIA_TYPE_VIDEO {
            Self::Video
        } else {
            Self::Image
        }
    }
}

/// one entry of the grid
#[derive(Clone, PartialEq, Debug)]
pub struct LibraryAsset {
    pub id: String,
    pub kind: LibraryAssetKind,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub duration_secs: f64,
    pub created_at_ms: Option<i64>,
}

/// where the shot was taken in degrees, south and west negative
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LibraryAssetPlace {
    pub latitude: f64,
    pub longitude: f64,
}

/// one asset of assets_created_after with what the map gallery scout reads beside the grid fields
#[derive(Clone, PartialEq, Debug)]
pub struct LibraryAssetDetails {
    pub asset: LibraryAsset,
    pub place: Option<LibraryAssetPlace>,
    pub is_screenshot: bool,
}

/// seconds since 1970 (or a file time in seconds) as whole ms
pub fn epoch_ms(seconds_since_1970: f64) -> Option<i64> {
    if !seconds_since_1970.is_finite() {
        return None;
    }
    let ms = (seconds_since_1970 * 1000.0).round();
    (ms >= i64::MIN as f64 && ms <= i64::MAX as f64).then_some(ms as i64)
}

/// newest first, undated assets go last
/// ties keep the id order so pages stay stable between fetches
pub fn sort_newest_first(assets: &mut [LibraryAsset]) {
    assets.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// one page of the newest first list
#[derive(Clone, PartialEq, Debug)]
pub struct AssetPage {
    pub assets: Vec<LibraryAsset>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

/// an asset collection of the picker album menu, its local id and the system title
/// the picker swaps in its own title for smart albums
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LibraryAssetCollection {
    pub id: String,
    pub localized_title: Option<String>,
}

/// smart albums in the menu order
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LibrarySmartAlbum {
    Favorites,
    Videos,
    SelfPortraits,
    LivePhotos,
    DepthEffect,
    Panoramas,
    Timelapses,
    SlomoVideos,
    Bursts,
    Screenshots,
}

impl LibrarySmartAlbum {
    /// raw collection subtype value
    pub const fn ph_subtype(self) -> isize {
        match self {
            Self::Panoramas => 201,
            Self::Videos => 202,
            Self::Favorites => 203,
            Self::Timelapses => 204,
            Self::Bursts => 207,
            Self::SlomoVideos => 208,
            Self::SelfPortraits => 210,
            Self::Screenshots => 211,
            Self::DepthEffect => 212,
            Self::LivePhotos => 213,
        }
    }
}

/// slice of total a page covers, an offset past the end is an empty last page
pub fn page_range(total: usize, offset: usize, limit: usize) -> Range<usize> {
    let start = offset.min(total);
    start..start.saturating_add(limit).min(total)
}

pub fn next_offset(total: usize, range: &Range<usize>) -> Option<usize> {
    (range.end < total && !range.is_empty()).then_some(range.end)
}

pub fn page_of(assets: Vec<LibraryAsset>, total: usize, range: &Range<usize>) -> AssetPage {
    AssetPage {
        assets,
        total,
        next_offset: next_offset(total, range),
    }
}

pub const GRID_THUMBNAIL_PIXELS: u32 = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageContentMode {
    AspectFit,
    AspectFill,
}

/// image delivery mode as the picker uses it
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageDeliveryMode {
    Opportunistic,
    HighQuality,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageSpec {
    pub width: u32,
    pub height: u32,
    pub mode: ImageContentMode,
    pub delivery: ImageDeliveryMode,
    pub allow_network: bool,
}

impl ImageSpec {
    /// spec for a grid thumbnail
    pub const fn grid() -> Self {
        Self {
            width: GRID_THUMBNAIL_PIXELS,
            height: GRID_THUMBNAIL_PIXELS,
            mode: ImageContentMode::AspectFill,
            delivery: ImageDeliveryMode::Opportunistic,
            allow_network: true,
        }
    }

    /// spec for a hero thumbnail, the window size in device pixels
    pub fn hero(width_px: u32, height_px: u32) -> Self {
        Self {
            width: width_px.max(1),
            height: height_px.max(1),
            mode: ImageContentMode::AspectFill,
            delivery: ImageDeliveryMode::HighQuality,
            allow_network: true,
        }
    }
}

/// progress fraction for a bar, a report with an error is dropped
/// the fraction is held to 0..1 and nan has no place on a bar
pub fn library_progress_fraction(progress: f64, has_error: bool) -> Option<f64> {
    if has_error || progress.is_nan() {
        return None;
    }
    Some(progress.clamp(0.0, 1.0))
}

pub const ENCODE_PROGRESS_POLL: Duration = Duration::from_millis(80);
const ENCODE_PROGRESS_CEILING: f64 = 0.99;
const ENCODE_PROGRESS_STEP: f64 = 0.005;

/// progress monitor of the export, stays below the ceiling until the session really ends
/// reports only after a step so polling doesnt wake the bar for nothing
#[derive(Clone, Copy, Debug)]
pub struct EncodeProgressGate {
    last: f64,
}

impl Default for EncodeProgressGate {
    fn default() -> Self {
        Self { last: -1.0 }
    }
}

impl EncodeProgressGate {
    pub fn report(&mut self, session_progress: f64) -> Option<f64> {
        let progress = session_progress.clamp(0.0, ENCODE_PROGRESS_CEILING);
        if progress - self.last >= ENCODE_PROGRESS_STEP {
            self.last = progress;
            Some(progress)
        } else {
            None
        }
    }
}

/// decoded pixels in the layout of gpui RenderImage, bgra 8 bit straight alpha
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LibraryImage {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub fn decode_system_image_data(bytes: &[u8], maximum_pixel_dimension: Option<u32>) -> Option<LibraryImage> {
    if bytes.is_empty() {
        return None;
    }
    super::os::decode_image_data(bytes, maximum_pixel_dimension)
}

/// core graphics contexts only draw premultiplied alpha but gpui wants straight
pub fn unpremultiply_bgra(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            let straight = (u32::from(*channel) * 255 + alpha / 2) / alpha;
            *channel = straight.min(255) as u8;
        }
    }
}

/// id the feature cancels a request by
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageRequestId(pub u64);

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

impl ImageRequestId {
    pub fn next() -> Self {
        Self(NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// one answer to an image request
/// opportunistic requests answer degraded first and final last
/// a cancelled one answers once with cancelled
#[derive(Clone, PartialEq, Debug)]
pub struct ImageDelivery {
    pub request: ImageRequestId,
    pub image: Option<LibraryImage>,
    pub degraded: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

impl ImageDelivery {
    /// true when no more answers follow for this request
    pub fn is_final(&self) -> bool {
        self.cancelled || !self.degraded || self.image.is_none()
    }
}

pub type ImageSink = Box<dyn FnMut(ImageDelivery) + Send>;
pub type AccessSink = Box<dyn FnOnce(PhotoLibraryAccessLevel) + Send>;
/// what an os layer answers an export with, Err(Cancelled) after cancel_export
pub type ExportSink = Box<dyn FnOnce(Result<PathBuf, PhotoLibraryUnavailable>) + Send>;
/// what the feature gets for an export, once
pub type ExportDone = Box<dyn FnOnce(ExportOutcome) + Send>;
/// runs on a library queue after any change, the feature refetches its pages
pub type ChangeSink = Arc<dyn Fn() + Send + Sync>;
/// fraction of the original fetched so far, on any thread
/// only a library that downloads reports it, local files never do
pub type LibraryProgress = Arc<dyn Fn(f64) + Send + Sync>;
/// one report of a video export, the two stages weigh 0.1 and 0.9 on the feature bar
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ExportProgress {
    Fetching(f64),
    Encoding(f64),
}
/// takes the export reports on any thread, local files and plain copies report nothing
pub type ExportProgressSink = Arc<dyn Fn(ExportProgress) + Send + Sync>;
/// what an os layer answers a save with, exactly once on any thread
pub type SaveSink = Box<dyn FnOnce(Result<(), PhotoLibraryUnavailable>) + Send>;

/// resource type of a file added as it is, photo or video
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LibraryResourceKind {
    Photo,
    Video,
}

impl LibraryResourceKind {
    pub const fn ph_resource_type(self) -> isize {
        match self {
            Self::Photo => 1,
            Self::Video => 2,
        }
    }
}

/// one file to save, its bytes go into the library unchanged
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LibrarySaveFile {
    pub path: PathBuf,
    pub kind: LibraryResourceKind,
    pub content_type: Option<String>,
}

/// what one performChanges block of the save writes
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LibrarySaveRequest {
    Image(PathBuf),
    Video(PathBuf),
    Originals(Vec<LibrarySaveFile>),
}

impl LibrarySaveRequest {
    /// every file the request reads, with the kind of resource it becomes
    pub fn files(&self) -> Vec<(&Path, LibraryResourceKind)> {
        match self {
            Self::Image(path) => vec![(path.as_path(), LibraryResourceKind::Photo)],
            Self::Video(path) => vec![(path.as_path(), LibraryResourceKind::Video)],
            Self::Originals(files) => files
                .iter()
                .map(|file| (file.path.as_path(), file.kind))
                .collect(),
        }
    }

    /// reason the request cant reach the library, checked before access is asked
    pub fn precheck(&self) -> Result<(), PhotoLibraryUnavailable> {
        let files = self.files();
        if files.is_empty() {
            return Err(PhotoLibraryUnavailable::NothingToSave);
        }
        match files.iter().find(|(path, _)| !path.is_file()) {
            Some((path, _)) => Err(PhotoLibraryUnavailable::SourceMissing(path.to_path_buf())),
            None => Ok(()),
        }
    }
}

/// save result, saved or failed or denied (add access not granted)
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LibrarySaveOutcome {
    Saved,
    Denied,
    Failed(PhotoLibraryUnavailable),
}

impl LibrarySaveOutcome {
    pub fn from_result(result: Result<(), PhotoLibraryUnavailable>) -> Self {
        match result {
            Ok(()) => Self::Saved,
            Err(PhotoLibraryUnavailable::NotAuthorized(_)) => Self::Denied,
            Err(reason) => Self::Failed(reason),
        }
    }
}

/// name a saved file takes, its own or stem (n).ext if taken
/// like a file manager names a copy, taken is the existence check so its testable
pub fn unique_save_name(file_name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(file_name) {
        return file_name.to_owned();
    }
    let (stem, extension) = match file_name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            (stem, Some(extension))
        }
        _ => (file_name, None),
    };
    (2..)
        .map(|n| match extension {
            Some(extension) => format!("{stem} ({n}).{extension}"),
            None => format!("{stem} ({n})"),
        })
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| file_name.to_owned())
}

/// folder library save, images to pictures and videos to videos
/// all or nothing, a failed copy removes the earlier ones
/// blocking so run it off main
pub fn save_into_folders(
    request: &LibrarySaveRequest,
    pictures: Option<&Path>,
    videos: Option<&Path>,
) -> Result<Vec<PathBuf>, PhotoLibraryUnavailable> {
    request.precheck()?;
    let mut written: Vec<PathBuf> = Vec::new();
    for (source, kind) in request.files() {
        match copy_into_folder(source, kind, pictures, videos) {
            Ok(target) => written.push(target),
            Err(reason) => {
                for target in &written {
                    let _ = std::fs::remove_file(target);
                }
                return Err(reason);
            }
        }
    }
    Ok(written)
}

fn copy_into_folder(
    source: &Path,
    kind: LibraryResourceKind,
    pictures: Option<&Path>,
    videos: Option<&Path>,
) -> Result<PathBuf, PhotoLibraryUnavailable> {
    let directory = match kind {
        LibraryResourceKind::Photo => pictures,
        LibraryResourceKind::Video => videos,
    }
    .ok_or_else(|| {
        PhotoLibraryUnavailable::SystemRefused(format!("the user has no {kind:?} folder"))
    })?;
    let refused = |error: std::io::Error| PhotoLibraryUnavailable::SystemRefused(error.to_string());
    std::fs::create_dir_all(directory).map_err(refused)?;
    let file_name = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| PhotoLibraryUnavailable::SourceMissing(source.to_path_buf()))?;
    let name = unique_save_name(&file_name, |candidate| directory.join(candidate).exists());
    let target = directory.join(name);
    std::fs::copy(source, &target).map_err(refused)?;
    Ok(target)
}

/// runs save_into_folders on its own thread, done gets the answer once
pub fn spawn_folder_save(
    request: LibrarySaveRequest,
    pictures: Option<PathBuf>,
    videos: Option<PathBuf>,
    done: SaveSink,
) {
    let slot = Arc::new(Mutex::new(Some(done)));
    let thread_slot = Arc::clone(&slot);
    let spawned = std::thread::Builder::new()
        .name("rise-photo-save".to_owned())
        .spawn(move || {
            let result = save_into_folders(&request, pictures.as_deref(), videos.as_deref());
            if let Some(done) = take_sink(&thread_slot) {
                done(result.map(drop));
            }
        });
    if let Err(error) = spawned {
        if let Some(done) = take_sink(&slot) {
            done(Err(PhotoLibraryUnavailable::SystemRefused(error.to_string())));
        }
    }
}

fn take_sink(slot: &Mutex<Option<SaveSink>>) -> Option<SaveSink> {
    slot.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// file name for an exported video, the original extension
/// and a stem made of the safe chars of the asset id (it carries slashes)
pub fn export_file_name(asset_id: &str, original_filename: &str) -> String {
    let stem: String = asset_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .take(96)
        .collect();
    let stem = if stem.is_empty() { "video".to_owned() } else { stem };
    let extension = Path::new(original_filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "mov".to_owned());
    format!("{stem}.{extension}")
}

/// why the library didnt answer
/// the grid shows the reason and the choose files button stays
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum PhotoLibraryUnavailable {
    #[error("the photo library is not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("photo library access is {0:?}")]
    NotAuthorized(PhotoLibraryAccessLevel),
    #[error("the system photo framework is not available: {0}")]
    FrameworkMissing(&'static str),
    #[error("no asset with id {0}")]
    AssetNotFound(String),
    #[error("asset {0} has no video resource")]
    NotAVideo(String),
    #[error("the library refused: {0}")]
    SystemRefused(String),
    #[error("the request was cancelled")]
    Cancelled,
    #[error("the video of asset {0} is unavailable")]
    VideoUnavailable(String),
    #[error("nothing to save at {}", .0.display())]
    SourceMissing(PathBuf),
    #[error("the save names no file")]
    NothingToSave,
}

/// three ends of a video load, the file or a cancellation or a failure
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ExportOutcome {
    Exported(PathBuf),
    Cancelled,
    Failed(PhotoLibraryUnavailable),
}

impl ExportOutcome {
    pub fn from_result(result: Result<PathBuf, PhotoLibraryUnavailable>) -> Self {
        match result {
            Ok(path) => Self::Exported(path),
            Err(PhotoLibraryUnavailable::Cancelled) => Self::Cancelled,
            Err(reason) => Self::Failed(reason),
        }
    }
}

/// what became of an os answer that reached the ledger
#[derive(Debug)]
pub enum ExportFinish {
    Delivered,
    Discarded(Option<PathBuf>),
}

/// exports still running by request id
/// a cancel answers the feature at once and the late os answer is dropped
pub struct ExportLedger {
    live: Mutex<Option<HashMap<u64, ExportDone>>>,
}

impl ExportLedger {
    pub const fn new() -> Self {
        Self {
            live: Mutex::new(None),
        }
    }

    fn with_live<T>(&self, f: impl FnOnce(&mut HashMap<u64, ExportDone>) -> T) -> T {
        let mut guard = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        f(guard.get_or_insert_with(HashMap::new))
    }

    pub fn start(&self, id: ImageRequestId, done: ExportDone) {
        self.with_live(|live| live.insert(id.0, done));
    }

    /// drops an export that never started, without answering it
    pub fn forget(&self, id: ImageRequestId) {
        self.with_live(|live| live.remove(&id.0));
    }

    pub fn is_live(&self, id: ImageRequestId) -> bool {
        self.with_live(|live| live.contains_key(&id.0))
    }

    /// answers the feature with Cancelled, false when the export already ended
    pub fn cancel(&self, id: ImageRequestId) -> bool {
        let Some(done) = self.with_live(|live| live.remove(&id.0)) else {
            return false;
        };
        done(ExportOutcome::Cancelled);
        true
    }

    pub fn finish(
        &self,
        id: ImageRequestId,
        result: Result<PathBuf, PhotoLibraryUnavailable>,
    ) -> ExportFinish {
        match self.with_live(|live| live.remove(&id.0)) {
            Some(done) => {
                done(ExportOutcome::from_result(result));
                ExportFinish::Delivered
            }
            None => ExportFinish::Discarded(result.ok()),
        }
    }
}

impl Default for ExportLedger {
    fn default() -> Self {
        Self::new()
    }
}

/// default answer of an os layer that doesnt override a method
pub const fn unavailable_by_default(host: HostOs) -> PhotoLibraryUnavailable {
    PhotoLibraryUnavailable::BranchNotWritten(host)
}

/// keeps a change subscription alive, dropping it unsubscribes
pub struct ChangeWatch {
    _guard: Box<dyn Send>,
}

impl ChangeWatch {
    pub fn new(guard: impl Send + 'static) -> Self {
        Self {
            _guard: Box::new(guard),
        }
    }
}

/// resource type that tells the resources of an asset apart
/// linux and windows have one resource per asset, the file as photo or video and never full size
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LibraryAssetResourceType {
    Photo,
    Video,
    FullSizePhoto,
    FullSizeVideo,
    Other,
}

impl LibraryAssetResourceType {
    /// photo 1 and video 2 and full size photo 5 and full size video 6
    /// everything else is Other
    pub fn from_ph_resource_type(raw: i64) -> Self {
        match raw {
            1 => Self::Photo,
            2 => Self::Video,
            5 => Self::FullSizePhoto,
            6 => Self::FullSizeVideo,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LibraryAssetResource {
    pub resource_type: LibraryAssetResourceType,
    pub uniform_type_identifier: String,
    pub original_filename: String,
    pub index: usize,
}

/// what the feature gets for a resource data request, once
pub type ResourceDataDone = Box<dyn FnOnce(Result<Vec<u8>, PhotoLibraryUnavailable>) + Send>;
/// what the feature gets for an image data request, once
/// the bytes and their type id
pub type ImageDataSink =
    Box<dyn FnOnce(Result<(Vec<u8>, Option<String>), PhotoLibraryUnavailable>) + Send>;

struct ResourceDataState {
    done: Option<ResourceDataDone>,
    data: Vec<u8>,
}

/// state of a resource request, chunks pile up under a ceiling
/// the feature is answered once by the first of completion or cancel or a chunk over the ceiling
/// the os layer feeds it from its handlers on any thread
pub struct ResourceDataRequest {
    maximum_bytes: usize,
    state: Mutex<ResourceDataState>,
}

impl ResourceDataRequest {
    pub fn new(maximum_bytes: usize, done: ResourceDataDone) -> Arc<Self> {
        Arc::new(Self {
            maximum_bytes,
            state: Mutex::new(ResourceDataState {
                done: Some(done),
                data: Vec::new(),
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ResourceDataState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn is_finished(&self) -> bool {
        self.lock().done.is_none()
    }

    /// adds a chunk, false when the request is over
    /// either it already ended or this chunk went over the ceiling and the feature got SystemRefused
    /// the os layer then cancels its own request
    pub fn receive(&self, chunk: &[u8]) -> bool {
        let done = {
            let mut state = self.lock();
            if state.done.is_none() {
                return false;
            }
            if chunk.len() <= self.maximum_bytes.saturating_sub(state.data.len()) {
                state.data.extend_from_slice(chunk);
                return true;
            }
            state.data = Vec::new();
            state.done.take()
        };
        if let Some(done) = done {
            done(Err(PhotoLibraryUnavailable::SystemRefused(format!(
                "the resource is larger than {} bytes",
                self.maximum_bytes
            ))));
        }
        false
    }

    /// the os completion handler, the collected bytes or its error
    pub fn complete(&self, error: Option<PhotoLibraryUnavailable>) {
        let (done, data) = {
            let mut state = self.lock();
            let data = std::mem::take(&mut state.data);
            (state.done.take(), data)
        };
        let Some(done) = done else {
            return;
        };
        match error {
            Some(reason) => done(Err(reason)),
            None => done(Ok(data)),
        }
    }

    /// answers the feature with Cancelled, false when the request had already ended
    pub fn cancel(&self) -> bool {
        let done = {
            let mut state = self.lock();
            state.data = Vec::new();
            state.done.take()
        };
        let Some(done) = done else {
            return false;
        };
        done(Err(PhotoLibraryUnavailable::Cancelled));
        true
    }

    /// drops a request that never started, without answering it
    pub fn forget(&self) {
        let mut state = self.lock();
        state.data = Vec::new();
        drop(state.done.take());
    }
}

/// the os layer, a host that doesnt override a method answers unavailable_by_default
pub trait PhotoLibrarySource {
    fn access_level(&self) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
        Err(unavailable_by_default(HostOs::current()))
    }

    fn request_access(&self, done: AccessSink) -> Result<(), PhotoLibraryUnavailable> {
        drop(done);
        Err(unavailable_by_default(HostOs::current()))
    }

    /// how many assets of this kind the library holds, blocking so call off the main thread
    fn count(&self, kind: LibraryMediaKind) -> Result<usize, PhotoLibraryUnavailable> {
        let _ = kind;
        Err(unavailable_by_default(HostOs::current()))
    }

    /// assets at range of the newest first list, blocking so call off the main thread
    fn assets(
        &self,
        kind: LibraryMediaKind,
        range: Range<usize>,
    ) -> Result<Vec<LibraryAsset>, PhotoLibraryUnavailable> {
        let _ = (kind, range);
        Err(unavailable_by_default(HostOs::current()))
    }

    /// one page from a single view of the library so total and assets agree
    /// default asks count and assets, a layer that holds a fetch result answers from it
    fn page(
        &self,
        kind: LibraryMediaKind,
        offset: usize,
        limit: usize,
    ) -> Result<AssetPage, PhotoLibraryUnavailable> {
        let total = self.count(kind)?;
        let range = page_range(total, offset, limit);
        let assets = if range.is_empty() {
            Vec::new()
        } else {
            self.assets(kind, range.clone())?
        };
        Ok(page_of(assets, total, &range))
    }

    fn assets_created_after(
        &self,
        kind: LibraryMediaKind,
        after_ms: i64,
        limit: usize,
    ) -> Result<Vec<LibraryAssetDetails>, PhotoLibraryUnavailable> {
        let _ = (kind, after_ms, limit);
        Err(unavailable_by_default(HostOs::current()))
    }

    fn request_image(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        spec: ImageSpec,
        sink: ImageSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let _ = (id, asset_id, spec);
        drop(sink);
        Err(unavailable_by_default(HostOs::current()))
    }

    fn request_image_with_progress(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        spec: ImageSpec,
        sink: ImageSink,
        progress: Option<LibraryProgress>,
    ) -> Result<(), PhotoLibraryUnavailable> {
        drop(progress);
        self.request_image(id, asset_id, spec, sink)
    }

    fn cancel_image(&self, id: ImageRequestId) {
        let _ = id;
    }

    fn export_video(
        &self,
        asset_id: &str,
        directory: &Path,
        done: ExportSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let _ = (asset_id, directory);
        drop(done);
        Err(unavailable_by_default(HostOs::current()))
    }

    fn start_export(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        directory: &Path,
        done: ExportSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let _ = id;
        self.export_video(asset_id, directory, done)
    }

    fn start_export_with_progress(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        directory: &Path,
        done: ExportSink,
        progress: Option<ExportProgressSink>,
    ) -> Result<(), PhotoLibraryUnavailable> {
        drop(progress);
        self.start_export(id, asset_id, directory, done)
    }

    fn cancel_export(&self, id: ImageRequestId) {
        let _ = id;
    }

    fn watch_changes(&self, sink: ChangeSink) -> Result<ChangeWatch, PhotoLibraryUnavailable> {
        drop(sink);
        Err(unavailable_by_default(HostOs::current()))
    }

    fn add_access_level(&self) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
        self.access_level()
    }

    fn request_add_access(&self, done: AccessSink) -> Result<(), PhotoLibraryUnavailable> {
        self.request_access(done)
    }

    fn save_to_library(&self, request: LibrarySaveRequest, done: SaveSink) {
        drop(request);
        done(Err(unavailable_by_default(HostOs::current())));
    }

    fn asset_resources(
        &self,
        asset_id: &str,
    ) -> Result<Vec<LibraryAssetResource>, PhotoLibraryUnavailable> {
        let _ = asset_id;
        Err(unavailable_by_default(HostOs::current()))
    }

    fn is_animated_image(&self, asset_id: &str) -> Result<bool, PhotoLibraryUnavailable> {
        let _ = asset_id;
        Err(unavailable_by_default(HostOs::current()))
    }

    fn request_resource_data(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        resource: &LibraryAssetResource,
        request: Arc<ResourceDataRequest>,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let _ = (id, asset_id, resource);
        drop(request);
        Err(unavailable_by_default(HostOs::current()))
    }

    fn cancel_resource_data(&self, id: ImageRequestId) {
        let _ = id;
    }

    fn request_image_data(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        done: ImageDataSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let _ = (id, asset_id);
        drop(done);
        Err(unavailable_by_default(HostOs::current()))
    }

    fn asset_collection(&self, id: &str) -> Result<Option<LibraryAssetCollection>, PhotoLibraryUnavailable> {
        let _ = id;
        Err(unavailable_by_default(HostOs::current()))
    }

    fn smart_albums(
        &self,
        subtype: LibrarySmartAlbum,
    ) -> Result<Vec<LibraryAssetCollection>, PhotoLibraryUnavailable> {
        let _ = subtype;
        Err(unavailable_by_default(HostOs::current()))
    }

    fn user_albums(&self) -> Result<Vec<LibraryAssetCollection>, PhotoLibraryUnavailable> {
        Err(unavailable_by_default(HostOs::current()))
    }

    fn collection_page(
        &self,
        collection: &LibraryAssetCollection,
        kind: LibraryMediaKind,
        offset: usize,
        limit: usize,
    ) -> Result<AssetPage, PhotoLibraryUnavailable> {
        let _ = (collection, kind, offset, limit);
        Err(unavailable_by_default(HostOs::current()))
    }
}

static NATIVE_LIBRARY: super::os::NativePhotoLibrary = super::os::NativePhotoLibrary;

fn source() -> &'static dyn PhotoLibrarySource {
    match super::tester_folder_library::tester_library() {
        Some(library) => library,
        None => &NATIVE_LIBRARY,
    }
}

pub fn access_level() -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
    source().access_level()
}

pub fn request_access(
    done: impl FnOnce(PhotoLibraryAccessLevel) + Send + 'static,
) -> Result<(), PhotoLibraryUnavailable> {
    let current = access_level()?;
    if !current.can_request() {
        done(current);
        return Ok(());
    }
    source().request_access(Box::new(done))
}

pub fn browse_support() -> PlatformSupport {
    match access_level() {
        Ok(level) if level.can_browse() => PlatformSupport::Performed,
        _ => PlatformSupport::Unsupported,
    }
}

fn source_for(asset_id: &str) -> &'static dyn PhotoLibrarySource {
    if super::picked_files_library::is_picked_asset(asset_id) {
        super::picked_files_library::picked_library()
    } else {
        source()
    }
}

fn require_browse_for(asset_id: &str) -> Result<(), PhotoLibraryUnavailable> {
    if super::picked_files_library::is_picked_asset(asset_id) {
        Ok(())
    } else {
        require_browse()
    }
}

fn require_browse() -> Result<(), PhotoLibraryUnavailable> {
    let level = access_level()?;
    if level.can_browse() {
        Ok(())
    } else {
        Err(PhotoLibraryUnavailable::NotAuthorized(level))
    }
}

/// one page of the newest first list of this kind, blocking so call off the main thread
pub fn page(
    kind: LibraryMediaKind,
    offset: usize,
    limit: usize,
) -> Result<AssetPage, PhotoLibraryUnavailable> {
    require_browse()?;
    source().page(kind, offset, limit)
}

pub fn assets_created_after(
    kind: LibraryMediaKind,
    after_ms: i64,
    limit: usize,
) -> Result<Vec<LibraryAssetDetails>, PhotoLibraryUnavailable> {
    require_browse()?;
    source().assets_created_after(kind, after_ms, limit)
}

pub fn request_image(
    asset_id: &str,
    spec: ImageSpec,
    sink: impl FnMut(ImageDelivery) + Send + 'static,
) -> Result<ImageRequestId, PhotoLibraryUnavailable> {
    require_browse_for(asset_id)?;
    let id = ImageRequestId::next();
    source_for(asset_id).request_image(id, asset_id, spec, Box::new(sink))?;
    Ok(id)
}

pub fn request_image_with_progress(
    asset_id: &str,
    spec: ImageSpec,
    on_progress: impl Fn(f64) + Send + Sync + 'static,
    sink: impl FnMut(ImageDelivery) + Send + 'static,
) -> Result<ImageRequestId, PhotoLibraryUnavailable> {
    require_browse_for(asset_id)?;
    let id = ImageRequestId::next();
    source_for(asset_id).request_image_with_progress(id, asset_id, spec, Box::new(sink), Some(Arc::new(on_progress)))?;
    Ok(id)
}

pub fn cancel_image(id: ImageRequestId) {
    super::picked_files_library::picked_library().cancel_image(id);
    source().cancel_image(id);
}

static EXPORTS: ExportLedger = ExportLedger::new();

pub fn export_video(
    asset_id: &str,
    directory: &Path,
    done: impl FnOnce(ExportOutcome) + Send + 'static,
) -> Result<ImageRequestId, PhotoLibraryUnavailable> {
    require_browse_for(asset_id)?;
    start_tracked_export(asset_id, directory, None, Box::new(done))
}

pub fn export_video_with_progress(
    asset_id: &str,
    directory: &Path,
    on_progress: impl Fn(ExportProgress) + Send + Sync + 'static,
    done: impl FnOnce(ExportOutcome) + Send + 'static,
) -> Result<ImageRequestId, PhotoLibraryUnavailable> {
    require_browse_for(asset_id)?;
    start_tracked_export(asset_id, directory, Some(Arc::new(on_progress)), Box::new(done))
}

fn start_tracked_export(
    asset_id: &str,
    directory: &Path,
    progress: Option<ExportProgressSink>,
    done: ExportDone,
) -> Result<ImageRequestId, PhotoLibraryUnavailable> {
    let id = ImageRequestId::next();
    EXPORTS.start(id, done);
    let sink: ExportSink = Box::new(move |result| finish_export(id, result));
    if let Err(reason) = source_for(asset_id).start_export_with_progress(id, asset_id, directory, sink, progress) {
        EXPORTS.forget(id);
        return Err(reason);
    }
    Ok(id)
}

pub fn cancel_export(id: ImageRequestId) {
    if EXPORTS.is_live(id) {
        source().cancel_export(id);
        EXPORTS.cancel(id);
    }
}

fn finish_export(id: ImageRequestId, result: Result<PathBuf, PhotoLibraryUnavailable>) {
    let ExportFinish::Discarded(Some(path)) = EXPORTS.finish(id, result) else {
        return;
    };
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(target: "riseonly", %error, path = %path.display(), "cancelled export not removed");
        }
    }
}

pub fn watch_changes(
    on_change: impl Fn() + Send + Sync + 'static,
) -> Result<ChangeWatch, PhotoLibraryUnavailable> {
    source().watch_changes(Arc::new(on_change))
}

pub fn add_access_level() -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
    source().add_access_level()
}

pub fn request_add_access(
    done: impl FnOnce(PhotoLibraryAccessLevel) + Send + 'static,
) -> Result<(), PhotoLibraryUnavailable> {
    let current = add_access_level()?;
    if !current.can_request() {
        done(current);
        return Ok(());
    }
    source().request_add_access(Box::new(done))
}

pub fn save_to_library(
    request: LibrarySaveRequest,
    done: impl FnOnce(LibrarySaveOutcome) + Send + 'static,
) {
    if let Err(reason) = request.precheck() {
        done(LibrarySaveOutcome::Failed(reason));
        return;
    }
    match add_access_level() {
        Ok(level) if level.can_browse() => {}
        Ok(level) => {
            done(LibrarySaveOutcome::from_result(Err(
                PhotoLibraryUnavailable::NotAuthorized(level),
            )));
            return;
        }
        Err(reason) => {
            done(LibrarySaveOutcome::Failed(reason));
            return;
        }
    }
    source().save_to_library(
        request,
        Box::new(move |result| done(LibrarySaveOutcome::from_result(result))),
    );
}

pub fn save_image(path: &Path, done: impl FnOnce(LibrarySaveOutcome) + Send + 'static) {
    save_to_library(LibrarySaveRequest::Image(path.to_path_buf()), done);
}

pub fn save_video(path: &Path, done: impl FnOnce(LibrarySaveOutcome) + Send + 'static) {
    save_to_library(LibrarySaveRequest::Video(path.to_path_buf()), done);
}

pub fn save_originals(
    files: Vec<LibrarySaveFile>,
    done: impl FnOnce(LibrarySaveOutcome) + Send + 'static,
) {
    save_to_library(LibrarySaveRequest::Originals(files), done);
}

pub fn asset_resources(asset_id: &str) -> Vec<LibraryAssetResource> {
    match require_browse_for(asset_id).and_then(|()| source_for(asset_id).asset_resources(asset_id)) {
        Ok(resources) => resources,
        Err(reason) => {
            tracing::debug!(target: "riseonly", %reason, "asset resources unavailable");
            Vec::new()
        }
    }
}

pub fn is_animated_image(asset_id: &str) -> bool {
    match require_browse_for(asset_id).and_then(|()| source_for(asset_id).is_animated_image(asset_id)) {
        Ok(animated) => animated,
        Err(reason) => {
            tracing::debug!(target: "riseonly", %reason, "playback style unavailable");
            false
        }
    }
}

static RESOURCE_REQUESTS: Mutex<Option<HashMap<u64, Arc<ResourceDataRequest>>>> = Mutex::new(None);

fn with_resource_requests<T>(f: impl FnOnce(&mut HashMap<u64, Arc<ResourceDataRequest>>) -> T) -> T {
    let mut guard = RESOURCE_REQUESTS.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

pub fn request_resource_data(
    asset_id: &str,
    resource: &LibraryAssetResource,
    maximum_bytes: usize,
    done: impl FnOnce(Result<Vec<u8>, PhotoLibraryUnavailable>) + Send + 'static,
) -> Result<ImageRequestId, PhotoLibraryUnavailable> {
    require_browse_for(asset_id)?;
    let id = ImageRequestId::next();
    let request = ResourceDataRequest::new(
        maximum_bytes,
        Box::new(move |result| {
            with_resource_requests(|live| live.remove(&id.0));
            done(result);
        }),
    );
    with_resource_requests(|live| live.insert(id.0, request.clone()));
    if let Err(reason) = source_for(asset_id).request_resource_data(id, asset_id, resource, request.clone()) {
        with_resource_requests(|live| live.remove(&id.0));
        request.forget();
        return Err(reason);
    }
    Ok(id)
}

pub fn cancel_resource_data(id: ImageRequestId) {
    let Some(request) = with_resource_requests(|live| live.get(&id.0).cloned()) else {
        return;
    };
    super::picked_files_library::picked_library().cancel_resource_data(id);
    source().cancel_resource_data(id);
    request.cancel();
}

pub fn request_image_data(
    asset_id: &str,
    done: impl FnOnce(Result<(Vec<u8>, Option<String>), PhotoLibraryUnavailable>) + Send + 'static,
) -> Result<ImageRequestId, PhotoLibraryUnavailable> {
    require_browse_for(asset_id)?;
    let id = ImageRequestId::next();
    source_for(asset_id).request_image_data(id, asset_id, Box::new(done))?;
    Ok(id)
}

pub fn asset_collection(id: &str) -> Option<LibraryAssetCollection> {
    match require_browse().and_then(|()| source().asset_collection(id)) {
        Ok(collection) => collection,
        Err(reason) => {
            tracing::debug!(target: "riseonly", %reason, "asset collection unavailable");
            None
        }
    }
}

pub fn smart_albums(subtype: LibrarySmartAlbum) -> Vec<LibraryAssetCollection> {
    match require_browse().and_then(|()| source().smart_albums(subtype)) {
        Ok(collections) => collections,
        Err(reason) => {
            tracing::debug!(target: "riseonly", %reason, "smart albums unavailable");
            Vec::new()
        }
    }
}

pub fn user_albums() -> Vec<LibraryAssetCollection> {
    match require_browse().and_then(|()| source().user_albums()) {
        Ok(collections) => collections,
        Err(reason) => {
            tracing::debug!(target: "riseonly", %reason, "user albums unavailable");
            Vec::new()
        }
    }
}

pub fn collection_page(
    collection: &LibraryAssetCollection,
    kind: LibraryMediaKind,
    offset: usize,
    limit: usize,
) -> Result<AssetPage, PhotoLibraryUnavailable> {
    require_browse()?;
    source().collection_page(collection, kind, offset, limit)
}
