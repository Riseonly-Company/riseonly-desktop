use super::asset_details::{created_after_of_kind, picture_place};
use super::folder_library::*;
use super::photo_library::*;

use std::ffi::{c_void, OsString};
use std::mem::size_of;
use std::ops::Range;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use windows::core::{Interface, GUID, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, PROPERTYKEY, SIZE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
    FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Threading::{
    CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, INFINITE,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::Shell::{
    FOLDERID_Pictures, FOLDERID_Screenshots, FOLDERID_Videos, IShellItem2, IShellItemImageFactory,
    SHCreateItemFromParsingName, SHGetKnownFolderPath, KF_FLAG_DEFAULT, SIIGBF,
    SIIGBF_CROPTOSQUARE, SIIGBF_INCACHEONLY, SIIGBF_RESIZETOFIT,
};

pub(super) struct NativePhotoLibrary;

const PKEY_IMAGE_HORIZONTAL_SIZE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x6444048f_4c8b_11d1_8b70_080036b11a03),
    pid: 3,
};
const PKEY_IMAGE_VERTICAL_SIZE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x6444048f_4c8b_11d1_8b70_080036b11a03),
    pid: 4,
};
const PKEY_VIDEO_FRAME_WIDTH: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440491_4c8b_11d1_8b70_080036b11a03),
    pid: 3,
};
const PKEY_VIDEO_FRAME_HEIGHT: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440491_4c8b_11d1_8b70_080036b11a03),
    pid: 4,
};
const PKEY_MEDIA_DURATION: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440490_4c8b_11d1_8b70_080036b11a03),
    pid: 3,
};

const SHELL_WORKERS: usize = 2;
const NOTIFY_BUFFER_WORDS: usize = 64 * 1024 / 4;

static SNAPSHOT: FolderSnapshotCache = FolderSnapshotCache::new();
static REQUESTS: PendingImageRequests = PendingImageRequests::new();

type Job = Box<dyn FnOnce() + Send>;

static SHELL_QUEUE: OnceLock<Option<Sender<Job>>> = OnceLock::new();

impl PhotoLibrarySource for NativePhotoLibrary {
    fn access_level(&self) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
        Ok(PhotoLibraryAccessLevel::Authorized)
    }

    fn save_to_library(&self, request: LibrarySaveRequest, done: SaveSink) {
        spawn_folder_save(
            request,
            known_folder(&FOLDERID_Pictures),
            known_folder(&FOLDERID_Videos),
            done,
        );
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
        on_shell_thread(move || {
            for asset in &mut page {
                fill_from_properties(asset);
            }
            page
        })
    }

    fn assets_created_after(
        &self,
        kind: LibraryMediaKind,
        after_ms: i64,
        limit: usize,
    ) -> Result<Vec<LibraryAssetDetails>, PhotoLibraryUnavailable> {
        let mut picked = created_after_of_kind(&snapshot(), kind, after_ms, limit);
        let filled = on_shell_thread(move || {
            for asset in &mut picked {
                fill_from_properties(asset);
            }
            picked
        })?;
        let screenshots = known_folder(&FOLDERID_Screenshots);
        Ok(filled
            .into_iter()
            .map(|asset| {
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
        let queue = shell_queue()?;
        REQUESTS.start(id);
        let sent = queue.send(Box::new(move || image_job(id, &path, kind, spec, sink)));
        if sent.is_err() {
            REQUESTS.cancel(id);
            return Err(shell_thread_stopped());
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
        let mut folders = Vec::with_capacity(roots.len());
        for root in &roots {
            folders.push(WatchedFolder::open(root)?);
        }
        let stop = Arc::new(new_event()?);
        let thread_stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("rise-photo-watch".to_owned())
            .spawn(move || watch_loop(folders, &thread_stop, &sink))
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
        let _ = id;
        let (path, _) = owned_asset(asset_id)?;
        folder_resource_exists(asset_id, resource)?;
        spawn_file_read("rise-photo-resource", move || read_folder_resource(&path, &request))
    }

    fn request_image_data(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        done: ImageDataSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let _ = id;
        let (path, kind) = owned_asset(asset_id)?;
        spawn_file_read("rise-photo-image-data", move || done(read_folder_image_data(&path, kind)))
    }
}

/// runs a blocking file read on its own thread like the export copy does
fn spawn_file_read(
    name: &str,
    work: impl FnOnce() + Send + 'static,
) -> Result<(), PhotoLibraryUnavailable> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
        .map(drop)
        .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))
}

fn refused(error: windows::core::Error) -> PhotoLibraryUnavailable {
    PhotoLibraryUnavailable::SystemRefused(error.message())
}

fn shell_thread_stopped() -> PhotoLibraryUnavailable {
    PhotoLibraryUnavailable::SystemRefused("the shell thumbnail threads stopped".to_owned())
}

fn wide_nul(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    let path = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    let wide = unsafe { path.as_wide() }.to_vec();
    unsafe { CoTaskMemFree(Some(path.0 as *const c_void)) };
    let folder = PathBuf::from(OsString::from_wide(&wide));
    folder.is_dir().then_some(folder)
}

fn library_roots() -> Vec<PathBuf> {
    folder_roots(
        [known_folder(&FOLDERID_Pictures), known_folder(&FOLDERID_Videos)]
            .into_iter()
            .flatten(),
    )
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

fn shell_queue() -> Result<&'static Sender<Job>, PhotoLibraryUnavailable> {
    SHELL_QUEUE
        .get_or_init(|| {
            let (sender, receiver) = channel::<Job>();
            let receiver = Arc::new(Mutex::new(receiver));
            let mut started = 0;
            for index in 0..SHELL_WORKERS {
                let receiver = Arc::clone(&receiver);
                let spawned = std::thread::Builder::new()
                    .name(format!("rise-photo-shell-{index}"))
                    .spawn(move || shell_worker(&receiver));
                if spawned.is_ok() {
                    started += 1;
                }
            }
            (started > 0).then_some(sender)
        })
        .as_ref()
        .ok_or_else(shell_thread_stopped)
}

fn shell_worker(receiver: &Mutex<Receiver<Job>>) {
    let joined = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    if joined.is_err() {
        tracing::warn!(?joined, "photo library shell thread runs without COM");
    }
    loop {
        let job = receiver.lock().recv();
        let Ok(job) = job else {
            return;
        };
        if catch_unwind(AssertUnwindSafe(job)).is_err() {
            tracing::error!("photo library shell job panicked");
        }
    }
}

/// runs work on a shell thread and waits for its answer
fn on_shell_thread<R: Send + 'static>(
    work: impl FnOnce() -> R + Send + 'static,
) -> Result<R, PhotoLibraryUnavailable> {
    let (reply, answer) = channel();
    shell_queue()?
        .send(Box::new(move || {
            let _ = reply.send(work());
        }))
        .map_err(|_| shell_thread_stopped())?;
    answer.recv().map_err(|_| shell_thread_stopped())
}

fn shell_item(path: &Path) -> windows::core::Result<IShellItem2> {
    let wide = wide_nul(path);
    unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }
}

fn size_keys(kind: LibraryAssetKind) -> (PROPERTYKEY, PROPERTYKEY) {
    match kind {
        LibraryAssetKind::Image => (PKEY_IMAGE_HORIZONTAL_SIZE, PKEY_IMAGE_VERTICAL_SIZE),
        LibraryAssetKind::Video => (PKEY_VIDEO_FRAME_WIDTH, PKEY_VIDEO_FRAME_HEIGHT),
    }
}

/// pixel size the property system knows for the file, none when it knows none
fn source_size(item: &IShellItem2, kind: LibraryAssetKind) -> Option<(u32, u32)> {
    let (width_key, height_key) = size_keys(kind);
    let width = unsafe { item.GetUInt32(&width_key) }.ok()?;
    let height = unsafe { item.GetUInt32(&height_key) }.ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

fn fill_from_properties(asset: &mut LibraryAsset) {
    let Ok(item) = shell_item(Path::new(&asset.id)) else {
        return;
    };
    if let Some((width, height)) = source_size(&item, asset.kind) {
        asset.pixel_width = width;
        asset.pixel_height = height;
    }
    if asset.kind == LibraryAssetKind::Video {
        if let Ok(units) = unsafe { item.GetUInt64(&PKEY_MEDIA_DURATION) } {
            asset.duration_secs = duration_secs_from_100ns(units);
        }
    }
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
    let made = shell_item(path).and_then(|item| {
        let factory: IShellItemImageFactory = item.cast()?;
        Ok((item, factory))
    });
    let (item, factory) = match made {
        Ok(pair) => pair,
        Err(error) => {
            finish(id, &mut sink, Err(error.message()));
            return;
        }
    };
    let target = thumbnail_box(spec, source_size(&item, kind));
    let size = SIZE {
        cx: target.width as i32,
        cy: target.height as i32,
    };
    let flags = if target.crop_to_square {
        SIIGBF_RESIZETOFIT | SIIGBF_CROPTOSQUARE
    } else {
        SIIGBF_RESIZETOFIT
    };
    if spec.delivery == ImageDeliveryMode::Opportunistic {
        if let Ok(image) = shell_image(&factory, size, flags | SIIGBF_INCACHEONLY) {
            if REQUESTS.is_live(id) {
                sink(ImageDelivery {
                    request: id,
                    image: Some(image),
                    degraded: true,
                    cancelled: false,
                    error: None,
                });
            }
        }
    }
    if !REQUESTS.is_live(id) {
        finish(id, &mut sink, Err(String::new()));
        return;
    }
    let image = shell_image(&factory, size, flags);
    finish(id, &mut sink, image);
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

fn shell_image(
    factory: &IShellItemImageFactory,
    size: SIZE,
    flags: SIIGBF,
) -> Result<LibraryImage, String> {
    let bitmap = unsafe { factory.GetImage(size, flags) }.map_err(|error| error.message())?;
    bitmap_pixels(OwnedBitmap(bitmap))
}

struct OwnedBitmap(HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.0.into()) };
    }
}

fn bitmap_pixels(bitmap: OwnedBitmap) -> Result<LibraryImage, String> {
    let mut header = BITMAP::default();
    let filled = unsafe {
        GetObjectW(
            bitmap.0.into(),
            size_of::<BITMAP>() as i32,
            Some((&mut header as *mut BITMAP).cast()),
        )
    };
    let width = header.bmWidth;
    let height = header.bmHeight.abs();
    if filled == 0 || width <= 0 || height <= 0 {
        return Err("the thumbnail bitmap has no size".to_owned());
    }
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.is_invalid() {
        return Err("CreateCompatibleDC failed".to_owned());
    }
    let lines = unsafe {
        GetDIBits(
            dc,
            bitmap.0,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    let _ = unsafe { DeleteDC(dc) };
    if lines != height {
        return Err("GetDIBits copied no thumbnail rows".to_owned());
    }
    straighten_shell_bgra(&mut pixels);
    Ok(LibraryImage {
        width: width as u32,
        height: height as u32,
        bgra: pixels,
    })
}

struct OwnedHandle(HANDLE);

unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn new_event() -> Result<OwnedHandle, PhotoLibraryUnavailable> {
    unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
        .map(OwnedHandle)
        .map_err(refused)
}

struct StopOnDrop(Arc<OwnedHandle>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let _ = unsafe { SetEvent(self.0 .0) };
    }
}

/// one watched folder with its overlapped read
/// boxed so the overlapped and the buffer never move while the read is pending
struct WatchedFolder {
    directory: OwnedHandle,
    event: OwnedHandle,
    overlapped: OVERLAPPED,
    buffer: Vec<u32>,
    armed: bool,
}

unsafe impl Send for WatchedFolder {}

impl WatchedFolder {
    fn open(root: &Path) -> Result<Box<Self>, PhotoLibraryUnavailable> {
        let wide = wide_nul(root);
        let directory = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .map_err(refused)?;
        let directory = OwnedHandle(directory);
        Ok(Box::new(Self {
            directory,
            event: new_event()?,
            overlapped: OVERLAPPED::default(),
            buffer: vec![0; NOTIFY_BUFFER_WORDS],
            armed: false,
        }))
    }

    fn arm(&mut self) {
        self.overlapped = OVERLAPPED {
            hEvent: self.event.0,
            ..Default::default()
        };
        let started = unsafe {
            ReadDirectoryChangesW(
                self.directory.0,
                self.buffer.as_mut_ptr().cast(),
                (self.buffer.len() * 4) as u32,
                true,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_LAST_WRITE
                    | FILE_NOTIFY_CHANGE_SIZE,
                None,
                Some(&mut self.overlapped),
                None,
            )
        };
        self.armed = started.is_ok();
    }

    /// reads the finished batch, true when it may change the grid
    fn collect(&mut self) -> bool {
        let mut bytes = 0u32;
        let done = unsafe {
            GetOverlappedResult(self.directory.0, &self.overlapped, &mut bytes, false)
        };
        self.armed = false;
        let _ = unsafe { ResetEvent(self.event.0) };
        match done {
            Ok(()) => {
                let raw: Vec<u8> = self
                    .buffer
                    .iter()
                    .flat_map(|word| word.to_le_bytes())
                    .take(bytes as usize)
                    .collect();
                change_touches_media(&notify_record_names(&raw), bytes == 0)
            }
            Err(_) => true,
        }
    }
}

impl Drop for WatchedFolder {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut bytes = 0u32;
        unsafe {
            let _ = CancelIoEx(self.directory.0, Some(&self.overlapped));
            let _ = GetOverlappedResult(self.directory.0, &self.overlapped, &mut bytes, true);
        }
    }
}

fn watch_loop(mut folders: Vec<Box<WatchedFolder>>, stop: &OwnedHandle, sink: &ChangeSink) {
    for folder in &mut folders {
        folder.arm();
    }
    let handles: Vec<HANDLE> = std::iter::once(stop.0)
        .chain(folders.iter().map(|folder| folder.event.0))
        .collect();
    let mut coalescer = ChangeCoalescer::default();
    loop {
        let timeout = coalescer
            .wait(RiseClock::monotonic())
            .map_or(INFINITE, |left| {
                left.as_micros().div_ceil(1000).min(u128::from(INFINITE - 1)) as u32
            });
        let woke = unsafe { WaitForMultipleObjects(&handles, false, timeout) };
        if woke == WAIT_OBJECT_0 {
            return;
        }
        if woke != WAIT_TIMEOUT {
            let index = woke.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
            let Some(folder) = index.checked_sub(1).and_then(|at| folders.get_mut(at)) else {
                tracing::warn!(?woke, "photo library watch stopped");
                return;
            };
            if folder.collect() {
                coalescer.note(RiseClock::monotonic());
            }
            folder.arm();
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
