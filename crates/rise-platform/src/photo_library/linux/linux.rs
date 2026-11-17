use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_int, c_short, c_ulong, CString};
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::ops::Range;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;

use futures::StreamExt as _;
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;

use crate::photo_library::asset_details::*;
use crate::photo_library::folder_library::*;
use crate::photo_library::photo_library::*;
use crate::photo_library::thumbnailer_queue::*;
use crate::photo_library::xdg_library::*;

pub(crate) struct NativePhotoLibrary;

type Job = Box<dyn FnOnce() + Send>;

const IMAGE_WORKERS: usize = 2;
const HEADER_FIRST_READ: usize = 64 * 1024;
const WATCH_TICK: Duration = Duration::from_millis(250);
const THUMBNAILER_WAIT: Duration = Duration::from_secs(4);
const INOTIFY_BUFFER: usize = 64 * 1024;

const IN_NONBLOCK: c_int = 0o4000;
const IN_CLOEXEC: c_int = 0o2_000_000;
const POLLIN: c_short = 0x001;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

unsafe extern "C" {
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, pathname: *const c_char, mask: u32) -> c_int;
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
}

static SNAPSHOT: FolderSnapshotCache = FolderSnapshotCache::new();
static REQUESTS: PendingImageRequests = PendingImageRequests::new();
static IMAGE_QUEUE: OnceLock<Option<Sender<Job>>> = OnceLock::new();
static THUMBNAILER: Mutex<Option<Arc<ThumbnailerLink>>> = Mutex::new(None);
static THUMBNAILER_GAVE_UP: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

impl PhotoLibrarySource for NativePhotoLibrary {
    fn access_level(&self) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
        Ok(PhotoLibraryAccessLevel::Authorized)
    }

    fn save_to_library(&self, request: LibrarySaveRequest, done: SaveSink) {
        let (pictures, videos) = save_folders();
        spawn_folder_save(request, pictures, videos, done);
    }

    fn count(&self, kind: LibraryMediaKind) -> Result<usize, PhotoLibraryUnavailable> {
        Ok(count_of_kind(&snapshot(), kind))
    }

    fn assets(
        &self,
        kind: LibraryMediaKind,
        range: Range<usize>,
    ) -> Result<Vec<LibraryAsset>, PhotoLibraryUnavailable> {
        let mut page = slice_of_kind(&snapshot(), kind, range);
        for asset in &mut page {
            fill_from_header(asset);
        }
        Ok(page)
    }

    fn assets_created_after(
        &self,
        kind: LibraryMediaKind,
        after_ms: i64,
        limit: usize,
    ) -> Result<Vec<LibraryAssetDetails>, PhotoLibraryUnavailable> {
        let screenshots = screenshot_folder();
        Ok(created_after_of_kind(&snapshot(), kind, after_ms, limit)
            .into_iter()
            .map(|mut asset| {
                fill_from_header(&mut asset);
                let path = Path::new(&asset.id);
                let place = match asset.kind {
                    LibraryAssetKind::Image => picture_place(path),
                    LibraryAssetKind::Video => None,
                };
                let is_screenshot = screenshots
                    .as_ref()
                    .is_some_and(|folder| is_inside_roots(path, std::slice::from_ref(folder)));
                LibraryAssetDetails {
                    asset,
                    place,
                    is_screenshot,
                }
            })
            .collect())
    }

    fn request_image(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        spec: ImageSpec,
        sink: ImageSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let (path, kind) = owned_asset(asset_id)?;
        let queue = image_queue()?;
        REQUESTS.start(id);
        let sent = queue.send(Box::new(move || image_job(id, &path, kind, spec, sink)));
        if sent.is_err() {
            REQUESTS.cancel(id);
            return Err(image_threads_stopped());
        }
        Ok(())
    }

    fn cancel_image(&self, id: ImageRequestId) {
        REQUESTS.cancel(id);
    }

    fn export_video(
        &self,
        asset_id: &str,
        directory: &Path,
        done: ExportSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let (path, kind) = owned_asset(asset_id)?;
        if kind != LibraryAssetKind::Video {
            return Err(PhotoLibraryUnavailable::NotAVideo(asset_id.to_owned()));
        }
        let original = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        let target = directory.join(export_file_name(asset_id, &original));
        let directory = directory.to_path_buf();
        std::thread::Builder::new()
            .name("rise-photo-export".to_owned())
            .spawn(move || {
                let copied = std::fs::create_dir_all(&directory)
                    .and_then(|()| std::fs::copy(&path, &target))
                    .map(|_| target)
                    .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()));
                done(copied);
            })
            .map(drop)
            .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))
    }

    fn watch_changes(&self, sink: ChangeSink) -> Result<ChangeWatch, PhotoLibraryUnavailable> {
        let roots = library_roots();
        if roots.is_empty() {
            return Err(PhotoLibraryUnavailable::SystemRefused(
                "neither the Pictures nor the Videos folder exists".to_owned(),
            ));
        }
        let mut watcher = FolderWatcher::open()?;
        for root in &roots {
            watcher.add_tree(root, 0);
        }
        if watcher.dirs.is_empty() {
            return Err(PhotoLibraryUnavailable::SystemRefused(format!(
                "inotify_add_watch: {}",
                std::io::Error::last_os_error()
            )));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("rise-photo-watch".to_owned())
            .spawn(move || watch_loop(watcher, &thread_stop, &sink))
            .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))?;
        Ok(ChangeWatch::new(StopOnDrop(stop)))
    }

    fn asset_resources(
        &self,
        asset_id: &str,
    ) -> Result<Vec<LibraryAssetResource>, PhotoLibraryUnavailable> {
        let (path, kind) = owned_asset(asset_id)?;
        Ok(folder_asset_resources(&path, kind))
    }

    fn is_animated_image(&self, asset_id: &str) -> Result<bool, PhotoLibraryUnavailable> {
        let (path, kind) = owned_asset(asset_id)?;
        Ok(folder_is_animated_image(&path, kind))
    }

    fn request_resource_data(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        resource: &LibraryAssetResource,
        request: Arc<ResourceDataRequest>,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let (path, _) = owned_asset(asset_id)?;
        folder_resource_exists(asset_id, resource)?;
        std::thread::Builder::new()
            .name(format!("rise-photo-resource-{}", id.0))
            .spawn(move || read_folder_resource(&path, &request))
            .map(drop)
            .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))
    }

    fn request_image_data(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        done: ImageDataSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let (path, kind) = owned_asset(asset_id)?;
        std::thread::Builder::new()
            .name(format!("rise-photo-image-data-{}", id.0))
            .spawn(move || done(read_folder_image_data(&path, kind)))
            .map(drop)
            .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))
    }
}

fn image_threads_stopped() -> PhotoLibraryUnavailable {
    PhotoLibraryUnavailable::SystemRefused("the photo image threads stopped".to_owned())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

fn library_roots() -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    let config = xdg_base_dir(std::env::var_os("XDG_CONFIG_HOME").as_deref(), &home, ".config");
    let contents = std::fs::read_to_string(config.join("user-dirs.dirs")).ok();
    let candidates = [
        user_dir_or_default(contents.as_deref(), XDG_PICTURES_KEY, &home, XDG_PICTURES_DEFAULT),
        user_dir_or_default(contents.as_deref(), XDG_VIDEOS_KEY, &home, XDG_VIDEOS_DEFAULT),
    ];
    folder_roots(candidates.into_iter().flatten().filter(|dir| dir.is_dir()))
}

/// where a save lands, the pictures and videos xdg dirs
/// created when missing
fn save_folders() -> (Option<PathBuf>, Option<PathBuf>) {
    let Some(home) = home_dir() else {
        return (None, None);
    };
    let config = xdg_base_dir(std::env::var_os("XDG_CONFIG_HOME").as_deref(), &home, ".config");
    let contents = std::fs::read_to_string(config.join("user-dirs.dirs")).ok();
    (
        user_dir_or_default(contents.as_deref(), XDG_PICTURES_KEY, &home, XDG_PICTURES_DEFAULT),
        user_dir_or_default(contents.as_deref(), XDG_VIDEOS_KEY, &home, XDG_VIDEOS_DEFAULT),
    )
}

fn screenshot_folder() -> Option<PathBuf> {
    let (pictures, _) = save_folders();
    Some(pictures?.join(SCREENSHOT_FOLDER_NAME))
}

const SCREENSHOT_FOLDER_NAME: &str = "Screenshots";

fn thumbnail_root() -> Option<PathBuf> {
    let home = home_dir()?;
    Some(xdg_base_dir(std::env::var_os("XDG_CACHE_HOME").as_deref(), &home, ".cache").join("thumbnails"))
}

fn snapshot() -> Arc<Vec<LibraryAsset>> {
    SNAPSHOT.get_or_scan(RiseClock::monotonic(), || scan_folder_library(&library_roots()))
}

/// asset behind an id this layer handed out, a media file under Pictures or Videos
fn owned_asset(asset_id: &str) -> Result<(PathBuf, LibraryAssetKind), PhotoLibraryUnavailable> {
    let path = PathBuf::from(asset_id);
    match folder_asset_kind(&path) {
        Some(kind) if is_inside_roots(&path, &library_roots()) && path.is_file() => Ok((path, kind)),
        _ => Err(PhotoLibraryUnavailable::AssetNotFound(asset_id.to_owned())),
    }
}

fn read_prefix(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(limit as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

fn image_size(path: &Path) -> Option<(u32, u32)> {
    let first = read_prefix(path, HEADER_FIRST_READ)?;
    if let Some(size) = image_header_size(&first) {
        return Some(size);
    }
    if first.len() < HEADER_FIRST_READ {
        return None;
    }
    image_header_size(&read_prefix(path, IMAGE_HEADER_READ)?)
}

fn video_header(path: &Path) -> Option<VideoHeader> {
    if !is_bmff_extension(path) {
        return None;
    }
    bmff_video_header(&mut File::open(path).ok()?)
}

fn fill_from_header(asset: &mut LibraryAsset) {
    let path = Path::new(&asset.id);
    match asset.kind {
        LibraryAssetKind::Image => {
            if let Some((width, height)) = image_size(path) {
                asset.pixel_width = width;
                asset.pixel_height = height;
            }
        }
        LibraryAssetKind::Video => {
            if let Some(header) = video_header(path) {
                asset.pixel_width = header.width;
                asset.pixel_height = header.height;
                asset.duration_secs = header.duration_secs;
            }
        }
    }
}

fn image_queue() -> Result<&'static Sender<Job>, PhotoLibraryUnavailable> {
    IMAGE_QUEUE
        .get_or_init(|| {
            let (sender, receiver) = channel::<Job>();
            let receiver = Arc::new(Mutex::new(receiver));
            let mut started = 0;
            for index in 0..IMAGE_WORKERS {
                let receiver = Arc::clone(&receiver);
                let spawned = std::thread::Builder::new()
                    .name(format!("rise-photo-image-{index}"))
                    .spawn(move || image_worker(&receiver));
                if spawned.is_ok() {
                    started += 1;
                }
            }
            (started > 0).then_some(sender)
        })
        .as_ref()
        .ok_or_else(image_threads_stopped)
}

fn image_worker(receiver: &Mutex<Receiver<Job>>) {
    loop {
        let job = receiver.lock().recv();
        let Ok(job) = job else {
            return;
        };
        if catch_unwind(AssertUnwindSafe(job)).is_err() {
            tracing::error!("photo library image job panicked");
        }
    }
}

/// gpui decoder for a file extension, none for formats its image build cant read
pub(crate) fn decoder_format(path: &Path) -> Option<gpui::ImageFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "jpg" | "jpeg" => gpui::ImageFormat::Jpeg,
        "png" => gpui::ImageFormat::Png,
        "gif" => gpui::ImageFormat::Gif,
        "webp" => gpui::ImageFormat::Webp,
        "bmp" => gpui::ImageFormat::Bmp,
        "tif" | "tiff" => gpui::ImageFormat::Tiff,
        _ => return None,
    })
}

/// decodes with gpui, exif orientation applied and bgra straight alpha
/// only the first frame of a gif
fn decode(format: gpui::ImageFormat, bytes: Vec<u8>) -> Result<LibraryImage, String> {
    let image = gpui::Image::from_bytes(format, bytes);
    let render = image
        .to_image_data(gpui::SvgRenderer::new(Arc::new(())))
        .map_err(|error| error.to_string())?;
    let size = render.size(0);
    let pixels = render.as_bytes(0).ok_or_else(|| "the image has no frame".to_owned())?;
    Ok(LibraryImage {
        width: u32::try_from(size.width.0).unwrap_or(0),
        height: u32::try_from(size.height.0).unwrap_or(0),
        bgra: pixels.to_vec(),
    })
}

/// freshest cached thumbnail scaled to the box, and whether it is sharp enough to be final
fn cached_thumbnail(path: &Path, target: ThumbnailBox) -> Option<(LibraryImage, bool)> {
    let uri = file_uri(path.to_str()?);
    let name = thumbnail_file_name(&uri);
    let mtime = std::fs::metadata(path).ok()?.mtime();
    let root = thumbnail_root()?;
    for (folder, _) in thumbnail_candidates(target) {
        let Ok(bytes) = std::fs::read(root.join(folder).join(&name)) else {
            continue;
        };
        if !thumbnail_is_fresh(&png_text_chunks(&bytes), &uri, mtime) {
            continue;
        }
        let Ok(decoded) = decode(gpui::ImageFormat::Png, bytes) else {
            continue;
        };
        let covers = image_covers_box(decoded.width, decoded.height, target);
        let scaled = scale_bgra_to_box(decoded.width, decoded.height, &decoded.bgra, target)?;
        return Some((scaled, covers));
    }
    None
}

fn decode_original(path: &Path, target: ThumbnailBox) -> Result<LibraryImage, String> {
    let format = decoder_format(path).ok_or_else(|| format!("no decoder for {}", path.display()))?;
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let decoded = decode(format, bytes)?;
    scale_bgra_to_box(decoded.width, decoded.height, &decoded.bgra, target)
        .ok_or_else(|| "the decoded image is empty".to_owned())
}

fn image_job(
    id: ImageRequestId,
    path: &Path,
    kind: LibraryAssetKind,
    spec: ImageSpec,
    mut sink: ImageSink,
) {
    if !REQUESTS.is_live(id) {
        sink(cancelled_delivery(id));
        return;
    }
    let source = match kind {
        LibraryAssetKind::Image => image_size(path),
        LibraryAssetKind::Video => video_header(path)
            .filter(|header| header.width > 0 && header.height > 0)
            .map(|header| (header.width, header.height)),
    };
    let target = thumbnail_box(spec, source);
    if let Some((image, covers)) = cached_thumbnail(path, target) {
        if covers || kind == LibraryAssetKind::Video {
            finish(id, &mut sink, Ok(image));
            return;
        }
        if spec.delivery == ImageDeliveryMode::Opportunistic && REQUESTS.is_live(id) {
            sink(ImageDelivery {
                request: id,
                image: Some(image),
                degraded: true,
                cancelled: false,
                error: None,
            });
        }
    }
    if kind == LibraryAssetKind::Video {
        if REQUESTS.is_live(id) && system_video_thumbnail(path, target) {
            if let Some((image, _)) = cached_thumbnail(path, target) {
                finish(id, &mut sink, Ok(image));
                return;
            }
        }
        finish(
            id,
            &mut sink,
            Err("no cached thumbnail for this video and the desktop thumbnailer made none".to_owned()),
        );
        return;
    }
    if !REQUESTS.is_live(id) {
        finish(id, &mut sink, Err(String::new()));
        return;
    }
    let image = decode_original(path, target);
    finish(id, &mut sink, image);
}

/// the desktop thumbnailer service or its absence, found once per run
enum ThumbnailerLink {
    Absent,
    Present { connection: zbus::Connection, flavors: Vec<String>, mime_types: Vec<String> },
}

fn thumbnailer_link() -> Arc<ThumbnailerLink> {
    let mut slot = THUMBNAILER.lock();
    if let Some(link) = slot.as_ref() {
        return link.clone();
    }
    let opened = zbus::block_on(async { tokio::time::timeout(THUMBNAILER_WAIT, open_thumbnailer()).await });
    let link = match opened {
        Ok(Ok(link)) => link,
        Ok(Err(error)) => {
            tracing::debug!(target: "riseonly", %error, "photo library: no desktop thumbnailer");
            ThumbnailerLink::Absent
        }
        Err(_) => {
            tracing::debug!(target: "riseonly", timeout = ?THUMBNAILER_WAIT, "photo library: the thumbnailer did not answer");
            ThumbnailerLink::Absent
        }
    };
    let link = Arc::new(link);
    *slot = Some(link.clone());
    link
}

async fn thumbnailer_proxy(connection: &zbus::Connection) -> zbus::Result<zbus::Proxy<'static>> {
    zbus::Proxy::new(connection, THUMBNAILER_BUS_NAME, THUMBNAILER_OBJECT_PATH, THUMBNAILER_INTERFACE).await
}

async fn open_thumbnailer() -> zbus::Result<ThumbnailerLink> {
    let connection = zbus::Connection::session().await?;
    let proxy = thumbnailer_proxy(&connection).await?;
    let flavors: Vec<String> = proxy.call(THUMBNAILER_GET_FLAVORS, &()).await?;
    let (_schemes, mime_types): (Vec<String>, Vec<String>) = proxy.call(THUMBNAILER_GET_SUPPORTED, &()).await?;
    Ok(ThumbnailerLink::Present { connection, flavors, mime_types })
}

fn system_video_thumbnail(path: &Path, target: ThumbnailBox) -> bool {
    let (Some(text), Some(mime)) = (path.to_str(), video_mime_type(path)) else {
        return false;
    };
    let uri = file_uri(text);
    if THUMBNAILER_GAVE_UP.lock().contains(&uri) {
        return false;
    }
    let link = thumbnailer_link();
    let ThumbnailerLink::Present { connection, flavors, mime_types } = link.as_ref() else {
        return false;
    };
    if !mime_types.iter().any(|known| known == mime) {
        return false;
    }
    let Some(flavor) = thumbnailer_flavor(target, flavors) else {
        return false;
    };
    let answer = zbus::block_on(async {
        tokio::time::timeout(THUMBNAILER_WAIT, queue_thumbnail(connection, &uri, mime, flavor)).await
    });
    let step = match answer {
        Ok(Ok(step)) => step,
        Ok(Err(error)) => {
            tracing::debug!(target: "riseonly", %error, "photo library: the thumbnailer refused a video");
            ThumbnailerStep::GiveUp
        }
        Err(_) => ThumbnailerStep::LookInCache,
    };
    if step == ThumbnailerStep::GiveUp {
        THUMBNAILER_GAVE_UP.lock().insert(uri);
        return false;
    }
    true
}

async fn queue_thumbnail(
    connection: &zbus::Connection,
    uri: &str,
    mime: &str,
    flavor: &str,
) -> zbus::Result<ThumbnailerStep> {
    let proxy = thumbnailer_proxy(connection).await?;
    let ready = proxy.receive_signal(THUMBNAILER_READY).await?.map(|message| {
        message
            .body()
            .deserialize::<(u32, Vec<String>)>()
            .map_or(ThumbnailerSignal::Unreadable, |(handle, uris)| ThumbnailerSignal::Ready { handle, uris })
    });
    let failed = proxy.receive_signal(THUMBNAILER_ERROR).await?.map(|message| {
        message
            .body()
            .deserialize::<(u32, Vec<String>, i32, String)>()
            .map_or(ThumbnailerSignal::Unreadable, |(handle, uris, _, _)| ThumbnailerSignal::Error { handle, uris })
    });
    let finished = proxy.receive_signal(THUMBNAILER_FINISHED).await?.map(|message| {
        message
            .body()
            .deserialize::<u32>()
            .map_or(ThumbnailerSignal::Unreadable, |handle| ThumbnailerSignal::Finished { handle })
    });
    let mut signals = std::pin::pin!(futures::stream::select(ready, futures::stream::select(failed, finished)));
    let handle: u32 = proxy
        .call(
            THUMBNAILER_QUEUE,
            &(vec![uri], vec![mime], flavor, THUMBNAILER_SCHEDULER, THUMBNAILER_NO_UNQUEUE),
        )
        .await?;
    while let Some(signal) = signals.next().await {
        match thumbnailer_step(handle, uri, &signal) {
            ThumbnailerStep::Wait => continue,
            step => return Ok(step),
        }
    }
    Ok(ThumbnailerStep::LookInCache)
}

/// the one last delivery of a request, cancelled if cancel_image came first
fn finish(id: ImageRequestId, sink: &mut ImageSink, result: Result<LibraryImage, String>) {
    if !REQUESTS.finish(id) {
        sink(cancelled_delivery(id));
        return;
    }
    sink(match result {
        Ok(image) => ImageDelivery {
            request: id,
            image: Some(image),
            degraded: false,
            cancelled: false,
            error: None,
        },
        Err(reason) => failed_delivery(id, reason),
    });
}

struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// one inotify descriptor watching every folder of the walk, wd to folder and depth under its root
pub(crate) struct FolderWatcher {
    file: File,
    pub(crate) dirs: HashMap<i32, (PathBuf, usize)>,
}

impl FolderWatcher {
    pub(crate) fn open() -> Result<Self, PhotoLibraryUnavailable> {
        let fd = unsafe { inotify_init1(IN_NONBLOCK | IN_CLOEXEC) };
        if fd < 0 {
            return Err(PhotoLibraryUnavailable::SystemRefused(format!(
                "inotify_init1: {}",
                std::io::Error::last_os_error()
            )));
        }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self {
            file: File::from(owned),
            dirs: HashMap::new(),
        })
    }

    fn add(&mut self, dir: &Path, depth: usize) -> bool {
        let Ok(c_path) = CString::new(dir.as_os_str().as_bytes()) else {
            return false;
        };
        let wd = unsafe { inotify_add_watch(self.file.as_raw_fd(), c_path.as_ptr(), FOLDER_WATCH_MASK) };
        if wd < 0 {
            return false;
        }
        self.dirs.insert(wd, (dir.to_path_buf(), depth));
        true
    }

    /// watches root and the folders under it the walk would enter
    /// no hidden ones or links, up to the max depth and max dir count
    pub(crate) fn add_tree(&mut self, root: &Path, depth: usize) {
        let mut stack = vec![(root.to_path_buf(), depth)];
        while let Some((dir, depth)) = stack.pop() {
            if self.dirs.len() >= FOLDER_WATCH_MAX_DIRS {
                tracing::warn!("photo library watch stops at {FOLDER_WATCH_MAX_DIRS} folders");
                return;
            }
            if !self.add(&dir, depth) || depth >= FOLDER_WALK_MAX_DEPTH {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_name().as_bytes().starts_with(b".") {
                    continue;
                }
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    stack.push((entry.path(), depth + 1));
                }
            }
        }
    }

    /// waits up to timeout for events, none when the descriptor failed for good
    pub(crate) fn wait(&mut self, timeout: Duration, buffer: &mut [u8]) -> Option<Vec<InotifyRecord>> {
        let mut fds = PollFd {
            fd: self.file.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        };
        let millis = c_int::try_from(timeout.as_millis()).unwrap_or(c_int::MAX);
        let ready = unsafe { poll(&raw mut fds, 1, millis) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            return (error.kind() == ErrorKind::Interrupted).then(Vec::new);
        }
        let mut records = Vec::new();
        if ready == 0 {
            return Some(records);
        }
        loop {
            match self.file.read(buffer) {
                Ok(0) => break,
                Ok(read) => records.extend(inotify_records(&buffer[..read])),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => {
                    tracing::warn!(%error, "photo library watch read failed");
                    return None;
                }
            }
        }
        Some(records)
    }

    /// keeps the watch list in step with the records, true when the grid may have changed
    fn apply(&mut self, records: &[InotifyRecord]) -> bool {
        let overflowed = records.iter().any(|record| record.mask & IN_Q_OVERFLOW != 0);
        let mut names = Vec::new();
        let mut folder_gone = false;
        for record in records {
            if record.mask & IN_IGNORED != 0 {
                self.dirs.remove(&record.wd);
                continue;
            }
            if record.mask & (IN_DELETE_SELF | IN_MOVE_SELF) != 0 {
                folder_gone = true;
            }
            if record.is_new_dir() {
                if let Some((parent, depth)) = self.dirs.get(&record.wd).cloned() {
                    if depth < FOLDER_WALK_MAX_DEPTH && !record.name.starts_with('.') {
                        self.add_tree(&parent.join(&record.name), depth + 1);
                    }
                }
            }
            if !record.name.is_empty() {
                names.push(record.name.clone());
            }
        }
        folder_gone || change_touches_media(&names, overflowed)
    }
}

fn watch_loop(mut watcher: FolderWatcher, stop: &AtomicBool, sink: &ChangeSink) {
    let mut buffer = vec![0u8; INOTIFY_BUFFER];
    let mut coalescer = ChangeCoalescer::default();
    while !stop.load(Ordering::Acquire) {
        let timeout = coalescer
            .wait(RiseClock::monotonic())
            .map_or(WATCH_TICK, |left| left.min(WATCH_TICK));
        let Some(records) = watcher.wait(timeout, &mut buffer) else {
            return;
        };
        if watcher.apply(&records) {
            coalescer.note(RiseClock::monotonic());
        }
        if coalescer.take_due(RiseClock::monotonic()) {
            SNAPSHOT.invalidate();
            sink();
        }
    }
}

pub(crate) fn decode_image_file(_path: &Path) -> Option<LibraryImage> {
    None
}

pub(crate) fn decode_image_data(_bytes: &[u8], _maximum_pixel_dimension: Option<u32>) -> Option<LibraryImage> {
    None
}

pub(crate) fn image_file_size(_path: &Path) -> Option<(u32, u32)> {
    None
}
