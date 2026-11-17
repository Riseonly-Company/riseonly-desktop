use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rise_core::foundation::foundation_date::RiseClock;

use crate::photo_library::asset_details::*;
use crate::photo_library::folder_library::*;
use crate::photo_library::photo_library::*;
use crate::photo_library::xdg_library::*;

pub const TESTER_FAVORITES_FOLDER: &str = "Favorites";
pub const TESTER_VIDEOS_COLLECTION_ID: &str = "tester-library:videos";
pub const TESTER_WATCH_INTERVAL: Duration = Duration::from_secs(2);
const HEADER_READ: usize = 512 * 1024;

/// first frame of a video file, at most max_side on the longer side
/// the platform crate has no video decoder so the app hands over the media poster reader
pub type TesterVideoPoster = fn(&Path, u32) -> Option<LibraryImage>;

pub struct TesterFolderLibrary {
    root: PathBuf,
    video_poster: TesterVideoPoster,
    snapshot: FolderSnapshotCache,
    requests: PendingImageRequests,
}

static TESTER_LIBRARY: OnceLock<TesterFolderLibrary> = OnceLock::new();

pub fn install_tester_library(root: PathBuf, video_poster: TesterVideoPoster) -> bool {
    if let Err(error) = std::fs::create_dir_all(&root) {
        tracing::warn!(target: "riseonly", %error, root = %root.display(), "tester photo folder not created");
    }
    TESTER_LIBRARY
        .set(TesterFolderLibrary {
            root,
            video_poster,
            snapshot: FolderSnapshotCache::new(),
            requests: PendingImageRequests::new(),
        })
        .is_ok()
}

pub(crate) fn tester_library() -> Option<&'static TesterFolderLibrary> {
    TESTER_LIBRARY.get()
}

pub fn is_tester_library_installed() -> bool {
    TESTER_LIBRARY.get().is_some()
}

/// pixel size a decoder reads from the first bytes of an image file
pub fn tester_image_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(HEADER_READ as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    image_header_size(&bytes).or_else(|| super::os::image_file_size(path))
}

/// size and length of an mp4 or quicktime file from its moov box
pub fn tester_video_header(path: &Path) -> Option<VideoHeader> {
    if !is_bmff_extension(path) {
        return None;
    }
    bmff_video_header(&mut std::fs::File::open(path).ok()?)
}

pub fn tester_decoder_format(path: &Path) -> Option<gpui::ImageFormat> {
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

pub(crate) fn decode_image(path: &Path) -> Result<LibraryImage, String> {
    let Some(format) = tester_decoder_format(path) else {
        return super::os::decode_image_file(path).ok_or_else(|| format!("no decoder for {}", path.display()));
    };
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
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

pub(crate) fn fill_from_header(asset: &mut LibraryAsset) {
    let path = Path::new(&asset.id);
    match asset.kind {
        LibraryAssetKind::Image => {
            if let Some((width, height)) = tester_image_size(path) {
                asset.pixel_width = width;
                asset.pixel_height = height;
            }
        }
        LibraryAssetKind::Video => {
            if let Some(header) = tester_video_header(path) {
                asset.pixel_width = header.width;
                asset.pixel_height = header.height;
                asset.duration_secs = header.duration_secs;
            }
        }
    }
}

fn filled(mut assets: Vec<LibraryAsset>) -> Vec<LibraryAsset> {
    for asset in &mut assets {
        fill_from_header(asset);
    }
    assets
}

struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// what a walk saw, cheap to compare
/// the ids and dates in order
fn fingerprint(assets: &[LibraryAsset]) -> Vec<(String, Option<i64>)> {
    assets
        .iter()
        .map(|asset| (asset.id.clone(), asset.created_at_ms))
        .collect()
}

impl TesterFolderLibrary {
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn snapshot(&self) -> Arc<Vec<LibraryAsset>> {
        self.snapshot
            .get_or_scan(RiseClock::monotonic(), || scan_folder_library(std::slice::from_ref(&self.root)))
    }

    fn owned_asset(&self, asset_id: &str) -> Result<(PathBuf, LibraryAssetKind), PhotoLibraryUnavailable> {
        let path = PathBuf::from(asset_id);
        match folder_asset_kind(&path) {
            Some(kind) if is_inside_roots(&path, std::slice::from_ref(&self.root)) && path.is_file() => {
                Ok((path, kind))
            }
            _ => Err(PhotoLibraryUnavailable::AssetNotFound(asset_id.to_owned())),
        }
    }

    fn favorites_folder(&self) -> PathBuf {
        self.root.join(TESTER_FAVORITES_FOLDER)
    }

    fn album_folders(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut folders: Vec<PathBuf> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                !name.starts_with('.') && name != TESTER_FAVORITES_FOLDER
            })
            .map(|entry| entry.path())
            .collect();
        folders.sort();
        folders
    }

    fn folder_collection(path: &Path) -> Option<LibraryAssetCollection> {
        Some(LibraryAssetCollection {
            id: path.to_str()?.to_owned(),
            localized_title: path.file_name().map(|name| name.to_string_lossy().into_owned()),
        })
    }

    fn collection_assets(&self, collection: &LibraryAssetCollection) -> Result<Vec<LibraryAsset>, PhotoLibraryUnavailable> {
        if collection.id == TESTER_VIDEOS_COLLECTION_ID {
            return Ok(self
                .snapshot()
                .iter()
                .filter(|asset| asset.kind == LibraryAssetKind::Video)
                .cloned()
                .collect());
        }
        let folder = PathBuf::from(&collection.id);
        if !is_inside_roots(&folder, std::slice::from_ref(&self.root)) || !folder.is_dir() {
            return Err(PhotoLibraryUnavailable::AssetNotFound(collection.id.clone()));
        }
        Ok(scan_folder_library(&[folder]))
    }

    fn image_job(&self, id: ImageRequestId, path: &Path, kind: LibraryAssetKind, spec: ImageSpec, mut sink: ImageSink) {
        if !self.requests.is_live(id) {
            sink(cancelled_delivery(id));
            return;
        }
        let source = match kind {
            LibraryAssetKind::Image => tester_image_size(path),
            LibraryAssetKind::Video => tester_video_header(path)
                .filter(|header| header.width > 0 && header.height > 0)
                .map(|header| (header.width, header.height)),
        };
        let target = thumbnail_box(spec, source);
        let decoded = match kind {
            LibraryAssetKind::Image => decode_image(path),
            LibraryAssetKind::Video => (self.video_poster)(path, target.width.max(target.height))
                .ok_or_else(|| format!("no poster frame for {}", path.display())),
        };
        let result = decoded.and_then(|image| {
            scale_bgra_to_box(image.width, image.height, &image.bgra, target)
                .ok_or_else(|| "the decoded image is empty".to_owned())
        });
        if !self.requests.finish(id) {
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
}

impl PhotoLibrarySource for TesterFolderLibrary {
    fn access_level(&self) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
        Ok(PhotoLibraryAccessLevel::Authorized)
    }

    fn add_access_level(&self) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
        Ok(PhotoLibraryAccessLevel::Authorized)
    }

    fn request_add_access(&self, done: AccessSink) -> Result<(), PhotoLibraryUnavailable> {
        done(PhotoLibraryAccessLevel::Authorized);
        Ok(())
    }

    fn save_to_library(&self, request: LibrarySaveRequest, done: SaveSink) {
        spawn_folder_save(request, Some(self.root.clone()), Some(self.root.clone()), done);
    }

    fn count(&self, kind: LibraryMediaKind) -> Result<usize, PhotoLibraryUnavailable> {
        Ok(count_of_kind(&self.snapshot(), kind))
    }

    fn assets(&self, kind: LibraryMediaKind, range: Range<usize>) -> Result<Vec<LibraryAsset>, PhotoLibraryUnavailable> {
        Ok(filled(slice_of_kind(&self.snapshot(), kind, range)))
    }

    fn assets_created_after(
        &self,
        kind: LibraryMediaKind,
        after_ms: i64,
        limit: usize,
    ) -> Result<Vec<LibraryAssetDetails>, PhotoLibraryUnavailable> {
        Ok(filled(created_after_of_kind(&self.snapshot(), kind, after_ms, limit))
            .into_iter()
            .map(|asset| {
                let place = match asset.kind {
                    LibraryAssetKind::Image => picture_place(Path::new(&asset.id)),
                    LibraryAssetKind::Video => None,
                };
                LibraryAssetDetails {
                    asset,
                    place,
                    is_screenshot: false,
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
        let (path, kind) = self.owned_asset(asset_id)?;
        self.requests.start(id);
        let spawned = std::thread::Builder::new()
            .name(format!("rise-tester-photo-{}", id.0))
            .spawn(move || {
                if let Some(library) = tester_library() {
                    library.image_job(id, &path, kind, spec, sink);
                }
            });
        if let Err(error) = spawned {
            self.requests.cancel(id);
            return Err(PhotoLibraryUnavailable::SystemRefused(error.to_string()));
        }
        Ok(())
    }

    fn cancel_image(&self, id: ImageRequestId) {
        self.requests.cancel(id);
    }

    fn export_video(&self, asset_id: &str, directory: &Path, done: ExportSink) -> Result<(), PhotoLibraryUnavailable> {
        let (path, kind) = self.owned_asset(asset_id)?;
        if kind != LibraryAssetKind::Video {
            return Err(PhotoLibraryUnavailable::NotAVideo(asset_id.to_owned()));
        }
        let original = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let target = directory.join(export_file_name(asset_id, &original));
        let directory = directory.to_path_buf();
        std::thread::Builder::new()
            .name("rise-tester-photo-export".to_owned())
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
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let root = self.root.clone();
        std::thread::Builder::new()
            .name("rise-tester-photo-watch".to_owned())
            .spawn(move || {
                let mut seen = fingerprint(&scan_folder_library(std::slice::from_ref(&root)));
                while !thread_stop.load(Ordering::Acquire) {
                    std::thread::sleep(TESTER_WATCH_INTERVAL);
                    if thread_stop.load(Ordering::Acquire) {
                        return;
                    }
                    let now = fingerprint(&scan_folder_library(std::slice::from_ref(&root)));
                    if now != seen {
                        seen = now;
                        if let Some(library) = tester_library() {
                            library.snapshot.invalidate();
                        }
                        sink();
                    }
                }
            })
            .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))?;
        Ok(ChangeWatch::new(StopOnDrop(stop)))
    }

    fn asset_resources(&self, asset_id: &str) -> Result<Vec<LibraryAssetResource>, PhotoLibraryUnavailable> {
        let (path, kind) = self.owned_asset(asset_id)?;
        Ok(folder_asset_resources(&path, kind))
    }

    fn is_animated_image(&self, asset_id: &str) -> Result<bool, PhotoLibraryUnavailable> {
        let (path, kind) = self.owned_asset(asset_id)?;
        Ok(folder_is_animated_image(&path, kind))
    }

    fn request_resource_data(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        resource: &LibraryAssetResource,
        request: Arc<ResourceDataRequest>,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let (path, _) = self.owned_asset(asset_id)?;
        folder_resource_exists(asset_id, resource)?;
        std::thread::Builder::new()
            .name(format!("rise-tester-photo-resource-{}", id.0))
            .spawn(move || read_folder_resource(&path, &request))
            .map(drop)
            .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))
    }

    fn request_image_data(&self, id: ImageRequestId, asset_id: &str, done: ImageDataSink) -> Result<(), PhotoLibraryUnavailable> {
        let (path, kind) = self.owned_asset(asset_id)?;
        std::thread::Builder::new()
            .name(format!("rise-tester-photo-image-data-{}", id.0))
            .spawn(move || done(read_folder_image_data(&path, kind)))
            .map(drop)
            .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))
    }

    fn asset_collection(&self, id: &str) -> Result<Option<LibraryAssetCollection>, PhotoLibraryUnavailable> {
        if id == TESTER_VIDEOS_COLLECTION_ID {
            return Ok(Some(LibraryAssetCollection {
                id: id.to_owned(),
                localized_title: None,
            }));
        }
        let folder = PathBuf::from(id);
        if !is_inside_roots(&folder, std::slice::from_ref(&self.root)) || !folder.is_dir() {
            return Ok(None);
        }
        Ok(Self::folder_collection(&folder))
    }

    fn smart_albums(&self, subtype: LibrarySmartAlbum) -> Result<Vec<LibraryAssetCollection>, PhotoLibraryUnavailable> {
        Ok(match subtype {
            LibrarySmartAlbum::Videos => vec![LibraryAssetCollection {
                id: TESTER_VIDEOS_COLLECTION_ID.to_owned(),
                localized_title: None,
            }],
            LibrarySmartAlbum::Favorites => {
                let folder = self.favorites_folder();
                if folder.is_dir() {
                    Self::folder_collection(&folder)
                        .map(|collection| LibraryAssetCollection {
                            localized_title: None,
                            ..collection
                        })
                        .into_iter()
                        .collect()
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        })
    }

    fn user_albums(&self) -> Result<Vec<LibraryAssetCollection>, PhotoLibraryUnavailable> {
        Ok(self
            .album_folders()
            .iter()
            .filter_map(|folder| Self::folder_collection(folder))
            .collect())
    }

    fn collection_page(
        &self,
        collection: &LibraryAssetCollection,
        kind: LibraryMediaKind,
        offset: usize,
        limit: usize,
    ) -> Result<AssetPage, PhotoLibraryUnavailable> {
        let all = self.collection_assets(collection)?;
        let total = count_of_kind(&all, kind);
        let range = page_range(total, offset, limit);
        let assets = if range.is_empty() {
            Vec::new()
        } else {
            filled(slice_of_kind(&all, kind, range.clone()))
        };
        Ok(page_of(assets, total, &range))
    }
}
