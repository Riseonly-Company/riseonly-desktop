use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::photo_library::folder_library::*;
use crate::photo_library::photo_library::*;
use crate::photo_library::tester_folder_library::{
    TesterVideoPoster, decode_image, fill_from_header, tester_image_size, tester_video_header,
};
use crate::photo_library::xdg_library::scale_bgra_to_box;

pub struct PickedFilesLibrary {
    requests: PendingImageRequests,
}

static PICKED_LIBRARY: PickedFilesLibrary = PickedFilesLibrary {
    requests: PendingImageRequests::new(),
};
static PICKED_ASSETS: Mutex<Option<HashMap<String, LibraryAsset>>> = Mutex::new(None);
static PICKED_VIDEO_POSTER: OnceLock<TesterVideoPoster> = OnceLock::new();

fn with_picked<T>(f: impl FnOnce(&mut HashMap<String, LibraryAsset>) -> T) -> T {
    let mut guard = PICKED_ASSETS.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

pub fn install_picked_video_poster(poster: TesterVideoPoster) -> bool {
    PICKED_VIDEO_POSTER.set(poster).is_ok()
}

/// registers the media files among paths in order and answers their assets
/// non files and unknown types are left out, a twice picked path answers its asset again
/// blocking header reads, call off main
pub fn register_picked_files(paths: &[PathBuf]) -> Vec<LibraryAsset> {
    let mut assets: Vec<LibraryAsset> = Vec::new();
    for path in paths {
        let Some(asset) = picked_asset_of(path) else {
            continue;
        };
        with_picked(|picked| picked.insert(asset.id.clone(), asset.clone()));
        if !assets.iter().any(|known| known.id == asset.id) {
            assets.push(asset);
        }
    }
    assets
}

fn picked_asset_of(path: &Path) -> Option<LibraryAsset> {
    let kind = folder_asset_kind(path)?;
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let id = path.to_str()?.to_owned();
    let mut asset = LibraryAsset {
        id,
        kind,
        pixel_width: 0,
        pixel_height: 0,
        duration_secs: 0.0,
        created_at_ms: folder_file_date_ms(metadata.created().ok(), metadata.modified().ok()),
    };
    fill_from_header(&mut asset);
    Some(asset)
}

pub fn picked_asset(id: &str) -> Option<LibraryAsset> {
    with_picked(|picked| picked.get(id).cloned())
}

/// true when the id names a file the user brought and not a system library asset
pub fn is_picked_asset(asset_id: &str) -> bool {
    if with_picked(|picked| picked.contains_key(asset_id)) {
        return true;
    }
    let path = Path::new(asset_id);
    if !path.is_absolute() {
        return false;
    }
    let Some(asset) = picked_asset_of(path) else {
        return false;
    };
    with_picked(|picked| picked.insert(asset.id.clone(), asset));
    true
}

pub(crate) fn picked_library() -> &'static PickedFilesLibrary {
    &PICKED_LIBRARY
}

impl PickedFilesLibrary {
    fn owned_asset(&self, asset_id: &str) -> Result<(PathBuf, LibraryAssetKind), PhotoLibraryUnavailable> {
        let path = PathBuf::from(asset_id);
        match picked_asset(asset_id) {
            Some(asset) if path.is_file() => Ok((path, asset.kind)),
            _ => Err(PhotoLibraryUnavailable::AssetNotFound(asset_id.to_owned())),
        }
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
            LibraryAssetKind::Video => PICKED_VIDEO_POSTER
                .get()
                .and_then(|poster| poster(path, target.width.max(target.height)))
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

impl PhotoLibrarySource for PickedFilesLibrary {
    fn access_level(&self) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
        Ok(PhotoLibraryAccessLevel::Authorized)
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
            .name(format!("rise-picked-photo-{}", id.0))
            .spawn(move || picked_library().image_job(id, &path, kind, spec, sink));
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
            .name("rise-picked-photo-export".to_owned())
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
            .name(format!("rise-picked-photo-resource-{}", id.0))
            .spawn(move || read_folder_resource(&path, &request))
            .map(drop)
            .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))
    }

    fn request_image_data(&self, id: ImageRequestId, asset_id: &str, done: ImageDataSink) -> Result<(), PhotoLibraryUnavailable> {
        let (path, kind) = self.owned_asset(asset_id)?;
        std::thread::Builder::new()
            .name(format!("rise-picked-photo-image-data-{}", id.0))
            .spawn(move || done(read_folder_image_data(&path, kind)))
            .map(drop)
            .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))
    }
}
