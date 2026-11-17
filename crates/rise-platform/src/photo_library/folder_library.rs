use std::io::{ErrorKind, Read};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use super::photo_library::{
    epoch_ms, sort_newest_first, unpremultiply_bgra, ImageContentMode, ImageDelivery,
    ImageRequestId, ImageSpec, LibraryAsset, LibraryAssetKind, LibraryAssetResource,
    LibraryAssetResourceType, LibraryMediaKind, PhotoLibraryUnavailable, ResourceDataRequest,
};
use crate::uniform_type::UTType;

pub const FOLDER_IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "bmp", "tif", "tiff", "avif",
];
pub const FOLDER_VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "m4v", "avi", "mkv", "webm", "wmv", "3gp"];
pub const FOLDER_WALK_MAX_DEPTH: usize = 8;
pub const FOLDER_WALK_MAX_FILES: usize = 50_000;
pub const FOLDER_SNAPSHOT_TTL: Duration = Duration::from_secs(3);

/// asset kind of a file by extension, none for anything the grid doesnt show
pub fn folder_asset_kind(path: &Path) -> Option<LibraryAssetKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if FOLDER_IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        Some(LibraryAssetKind::Image)
    } else if FOLDER_VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        Some(LibraryAssetKind::Video)
    } else {
        None
    }
}

/// file time as ms since the unix epoch, negative before 1970
pub fn system_time_ms(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => epoch_ms(after.as_secs_f64()),
        Err(before) => epoch_ms(-before.duration().as_secs_f64()),
    }
}

/// date a folder asset sorts by, the earlier of created and modified
/// because a copy gets a new creation time but keeps the modification time of the shot
pub fn folder_file_date_ms(created: Option<SystemTime>, modified: Option<SystemTime>) -> Option<i64> {
    let created = created.and_then(system_time_ms);
    let modified = modified.and_then(system_time_ms);
    match (created, modified) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// media durations from the windows property system and gstreamer come in 100 ns units
pub fn duration_secs_from_100ns(units: u64) -> f64 {
    units as f64 / 10_000_000.0
}

/// roots to walk, existing ones without duplicates and without a root inside another
pub fn folder_roots(candidates: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if candidate.as_os_str().is_empty() {
            continue;
        }
        if roots.iter().any(|root| candidate.starts_with(root)) {
            continue;
        }
        roots.retain(|root| !root.starts_with(&candidate));
        roots.push(candidate);
    }
    roots
}

/// true when path lies under one of the roots, folder layers only answer for their own files
pub fn is_inside_roots(path: &Path, roots: &[PathBuf]) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        && roots.iter().any(|root| path.starts_with(root))
}

/// walks the roots for media files, newest first
/// hidden entries and links are skipped and sizes and durations are zero, the layer fills them per page
pub fn scan_folder_library(roots: &[PathBuf]) -> Vec<LibraryAsset> {
    let mut assets = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = roots.iter().map(|root| (root.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if assets.len() >= FOLDER_WALK_MAX_FILES {
                sort_newest_first(&mut assets);
                return assets;
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if depth < FOLDER_WALK_MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let Some(kind) = folder_asset_kind(&path) else {
                continue;
            };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Some(id) = path.to_str().map(str::to_owned) else {
                continue;
            };
            assets.push(LibraryAsset {
                id,
                kind,
                pixel_width: 0,
                pixel_height: 0,
                duration_secs: 0.0,
                created_at_ms: folder_file_date_ms(metadata.created().ok(), metadata.modified().ok()),
            });
        }
    }
    sort_newest_first(&mut assets);
    assets
}

/// how many assets of this kind a newest first snapshot holds
pub fn count_of_kind(all: &[LibraryAsset], kind: LibraryMediaKind) -> usize {
    all.iter().filter(|asset| kind.admits(asset.kind)).count()
}

/// assets at range of the newest first list of this kind
pub fn slice_of_kind(all: &[LibraryAsset], kind: LibraryMediaKind, range: Range<usize>) -> Vec<LibraryAsset> {
    all.iter()
        .filter(|asset| kind.admits(asset.kind))
        .skip(range.start)
        .take(range.len())
        .cloned()
        .collect()
}

/// last walk, reused while fresh so count and assets of one page agree
/// times are monotonic clock readings so a test can drive them
pub struct FolderSnapshotCache {
    inner: Mutex<Option<(Duration, Arc<Vec<LibraryAsset>>)>>,
}

impl FolderSnapshotCache {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    pub fn get_or_scan(&self, now: Duration, scan: impl FnOnce() -> Vec<LibraryAsset>) -> Arc<Vec<LibraryAsset>> {
        let mut inner = self.inner.lock();
        if let Some((taken, assets)) = inner.as_ref() {
            if now.saturating_sub(*taken) < FOLDER_SNAPSHOT_TTL {
                return Arc::clone(assets);
            }
        }
        let assets = Arc::new(scan());
        *inner = Some((now, Arc::clone(&assets)));
        assets
    }

    /// a change event makes the next fetch walk again
    pub fn invalidate(&self) {
        *self.inner.lock() = None;
    }
}

impl Default for FolderSnapshotCache {
    fn default() -> Self {
        Self::new()
    }
}

/// box a fit inside thumbnailer is asked for, and whether it crops to a square
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ThumbnailBox {
    pub width: u32,
    pub height: u32,
    pub crop_to_square: bool,
}

pub const THUMBNAIL_BOX_MAX_EDGE: u32 = 8192;

/// thumbnail box for a fit inside thumbnailer, fit asks for the target itself
/// fill asks for the source scaled to cover the target and never cropped
/// only a square fill of an unknown size source falls back to crop
pub fn thumbnail_box(spec: ImageSpec, source: Option<(u32, u32)>) -> ThumbnailBox {
    let width = spec.width.max(1);
    let height = spec.height.max(1);
    match spec.mode {
        ImageContentMode::AspectFit => ThumbnailBox {
            width,
            height,
            crop_to_square: false,
        },
        ImageContentMode::AspectFill if width == height && !source.is_some_and(|(w, h)| w > 0 && h > 0) => {
            ThumbnailBox {
                width,
                height,
                crop_to_square: true,
            }
        }
        ImageContentMode::AspectFill => match source {
            Some((source_width, source_height)) if source_width > 0 && source_height > 0 => {
                let scale = (f64::from(width) / f64::from(source_width))
                    .max(f64::from(height) / f64::from(source_height));
                let edge = |side: u32| {
                    ((f64::from(side) * scale).ceil() as u32).clamp(1, THUMBNAIL_BOX_MAX_EDGE)
                };
                ThumbnailBox {
                    width: edge(source_width).max(width),
                    height: edge(source_height).max(height),
                    crop_to_square: false,
                }
            }
            _ => {
                let edge = width.max(height);
                ThumbnailBox {
                    width: edge,
                    height: edge,
                    crop_to_square: false,
                }
            }
        },
    }
}

/// fixes 32 bit shell and gdi bitmaps, all zero alpha means the source had none so its opaque
/// otherwise colour is premultiplied and gpui wants it straight
pub fn straighten_shell_bgra(pixels: &mut [u8]) {
    if pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    } else {
        unpremultiply_bgra(pixels);
    }
}

/// book of image requests a background layer still works on
/// cancel strikes an id and the worker asks before every delivery, answers cancelled once
pub struct PendingImageRequests {
    live: Mutex<Vec<u64>>,
}

impl PendingImageRequests {
    pub const fn new() -> Self {
        Self {
            live: Mutex::new(Vec::new()),
        }
    }

    pub fn start(&self, id: ImageRequestId) {
        let mut live = self.live.lock();
        if !live.contains(&id.0) {
            live.push(id.0);
        }
    }

    pub fn is_live(&self, id: ImageRequestId) -> bool {
        self.live.lock().contains(&id.0)
    }

    /// strikes the id out, false when it was unknown or already finished
    pub fn cancel(&self, id: ImageRequestId) -> bool {
        let mut live = self.live.lock();
        let before = live.len();
        live.retain(|known| *known != id.0);
        live.len() != before
    }

    /// the workers last word, true when the request was still live so it delivers the final image
    /// false when cancelled meanwhile so it delivers the cancelled one
    pub fn finish(&self, id: ImageRequestId) -> bool {
        self.cancel(id)
    }
}

impl Default for PendingImageRequests {
    fn default() -> Self {
        Self::new()
    }
}

pub fn cancelled_delivery(request: ImageRequestId) -> ImageDelivery {
    ImageDelivery {
        request,
        image: None,
        degraded: false,
        cancelled: true,
        error: None,
    }
}

pub fn failed_delivery(request: ImageRequestId, reason: String) -> ImageDelivery {
    ImageDelivery {
        request,
        image: None,
        degraded: false,
        cancelled: false,
        error: Some(reason),
    }
}

pub const FOLDER_CHANGE_QUIET: Duration = Duration::from_millis(400);
pub const FOLDER_CHANGE_MAX_DELAY: Duration = Duration::from_secs(2);

/// folds a burst of file events (a camera import writes hundreds) into one change call
/// times are monotonic clock readings
#[derive(Clone, Copy, Default, Debug)]
pub struct ChangeCoalescer {
    first: Option<Duration>,
    last: Option<Duration>,
}

impl ChangeCoalescer {
    pub fn note(&mut self, now: Duration) {
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// how long the watcher may sleep before due, none with nothing pending
    pub fn wait(&self, now: Duration) -> Option<Duration> {
        let (first, last) = (self.first?, self.last?);
        let quiet_end = last + FOLDER_CHANGE_QUIET;
        let hard_end = first + FOLDER_CHANGE_MAX_DELAY;
        Some(quiet_end.min(hard_end).saturating_sub(now))
    }

    /// true once when the pending burst should be reported, then it starts empty
    pub fn take_due(&mut self, now: Duration) -> bool {
        match self.wait(now) {
            Some(left) if left.is_zero() => {
                *self = Self::default();
                true
            }
            _ => false,
        }
    }
}

/// file names from a ReadDirectoryChangesW buffer, relative to the watched folder
/// a malformed record ends the list
pub fn notify_record_names(buffer: &[u8]) -> Vec<String> {
    let read_u32 = |at: usize| -> Option<u32> {
        let bytes = buffer.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    let mut names = Vec::new();
    let mut at = 0usize;
    loop {
        let (Some(next), Some(name_len)) = (read_u32(at), read_u32(at + 8)) else {
            break;
        };
        let start = at + 12;
        let Some(name_bytes) = start
            .checked_add(name_len as usize)
            .and_then(|end| buffer.get(start..end))
        else {
            break;
        };
        let units: Vec<u16> = name_bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        names.push(String::from_utf16_lossy(&units));
        if next == 0 {
            break;
        }
        match at.checked_add(next as usize) {
            Some(following) if following > at => at = following,
            _ => break,
        }
    }
    names
}

/// true when a batch of folder events can change the grid
/// a media file or a name without extension (maybe a folder) or an overflowed buffer
pub fn change_touches_media(names: &[String], overflowed: bool) -> bool {
    overflowed
        || names.iter().any(|name| {
            let path = Path::new(name);
            path.extension().is_none() || folder_asset_kind(path).is_some()
        })
}

pub const FOLDER_RESOURCE_CHUNK_BYTES: usize = 256 * 1024;

/// uniform type of a folder file, none for an extension no declaration claims
pub fn folder_uniform_type(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?;
    UTType::from_filename_extension(extension).map(|uniform| uniform.identifier())
}

/// resources of a folder asset, just the file itself at index 0
/// an unknown extension still names the kind (image or movie)
pub fn folder_asset_resources(path: &Path, kind: LibraryAssetKind) -> Vec<LibraryAssetResource> {
    let (resource_type, fallback) = match kind {
        LibraryAssetKind::Image => (LibraryAssetResourceType::Photo, UTType::image()),
        LibraryAssetKind::Video => (LibraryAssetResourceType::Video, UTType::movie()),
    };
    let original_filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    vec![LibraryAssetResource {
        resource_type,
        uniform_type_identifier: folder_uniform_type(path).unwrap_or_else(|| fallback.identifier()),
        original_filename,
        index: 0,
    }]
}

/// true when a folder asset is an animated image, ie an image file that is a gif
pub fn folder_is_animated_image(path: &Path, kind: LibraryAssetKind) -> bool {
    kind == LibraryAssetKind::Image
        && folder_uniform_type(path).as_deref() == Some("com.compuserve.gif")
}

/// the resource a folder asset was asked for, only the file at index 0 exists
pub fn folder_resource_exists(
    asset_id: &str,
    resource: &LibraryAssetResource,
) -> Result<(), PhotoLibraryUnavailable> {
    if resource.index == 0 {
        Ok(())
    } else {
        Err(PhotoLibraryUnavailable::AssetNotFound(asset_id.to_owned()))
    }
}

/// feeds the request from the reader chunk by chunk until the end or an error
/// or until the request is over (cancelled or past its ceiling, receive answers false)
pub fn feed_resource_request(reader: &mut impl Read, request: &ResourceDataRequest) {
    let mut buffer = vec![0u8; FOLDER_RESOURCE_CHUNK_BYTES];
    loop {
        if request.is_finished() {
            return;
        }
        match reader.read(&mut buffer) {
            Ok(0) => {
                request.complete(None);
                return;
            }
            Ok(read) => {
                if !request.receive(&buffer[..read]) {
                    return;
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => {
                request.complete(Some(PhotoLibraryUnavailable::SystemRefused(error.to_string())));
                return;
            }
        }
    }
}

/// reads a folder file under the request ceiling, blocking so run it off the main thread
pub fn read_folder_resource(path: &Path, request: &ResourceDataRequest) {
    match std::fs::File::open(path) {
        Ok(mut file) => feed_resource_request(&mut file, request),
        Err(error) => {
            request.complete(Some(PhotoLibraryUnavailable::SystemRefused(error.to_string())));
        }
    }
}

/// image file bytes and their type for a folder asset
/// a video has no image data so the photo export fails, blocking so run it off the main thread
pub fn read_folder_image_data(
    path: &Path,
    kind: LibraryAssetKind,
) -> Result<(Vec<u8>, Option<String>), PhotoLibraryUnavailable> {
    if kind != LibraryAssetKind::Image {
        return Err(PhotoLibraryUnavailable::SystemRefused(format!(
            "{} has no image data",
            path.display()
        )));
    }
    let bytes = std::fs::read(path)
        .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))?;
    Ok((bytes, folder_uniform_type(path)))
}
