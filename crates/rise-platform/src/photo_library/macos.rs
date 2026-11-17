use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{define_class, msg_send, AnyThread as _};
use objc2_foundation::{NSArray, NSData, NSError, NSNumber, NSPoint, NSRect, NSSize, NSString, NSURL};
use parking_lot::Mutex;

use super::photo_library::*;

const PH_ACCESS_LEVEL_READ_WRITE: isize = 2;
const PH_ACCESS_LEVEL_ADD_ONLY: isize = 1;
const PH_INVALID_IMAGE_REQUEST_ID: i32 = 0;
const PH_PLAYBACK_STYLE_IMAGE_ANIMATED: isize = 2;
const PH_IMAGE_VERSION_CURRENT: isize = 0;

#[link(name = "Photos", kind = "framework")]
unsafe extern "C" {
    static PHImageCancelledKey: &'static NSString;
    static PHImageErrorKey: &'static NSString;
    static PHImageResultIsDegradedKey: &'static NSString;
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVAssetExportPresetHighestQuality: &'static NSString;
    static AVFileTypeMPEG4: &'static NSString;
}

const PH_VIDEO_DELIVERY_HIGH_QUALITY: isize = 1;
const PH_VIDEO_VERSION_CURRENT: isize = 0;
const AV_EXPORT_COMPLETED: isize = 3;
const AV_EXPORT_CANCELLED: isize = 5;
const EXPORT_NAME: &str = "video.mp4";

#[repr(C)]
struct CGImage {
    _private: [u8; 0],
}

unsafe impl RefEncode for CGImage {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[]));
}

const BGRA_PREMULTIPLIED: u32 = 2 | (2 << 12);

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(image: *const CGImage) -> usize;
    fn CGImageGetHeight(image: *const CGImage) -> usize;
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut c_void;
    fn CGContextDrawImage(context: *mut c_void, rect: NSRect, image: *const CGImage);
    fn CGContextRelease(context: *mut c_void);
    fn CGImageRelease(image: *const CGImage);
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    static kCGImageSourceCreateThumbnailFromImageAlways: &'static NSString;
    static kCGImageSourceCreateThumbnailWithTransform: &'static NSString;
    static kCGImageSourceThumbnailMaxPixelSize: &'static NSString;
    static kCGImagePropertyPixelWidth: &'static NSString;
    static kCGImagePropertyPixelHeight: &'static NSString;
    static kCGImagePropertyOrientation: &'static NSString;
    fn CGImageSourceCreateWithURL(url: *const AnyObject, options: *const AnyObject) -> *const c_void;
    fn CGImageSourceCreateWithData(data: *const AnyObject, options: *const AnyObject) -> *const c_void;
    fn CGImageSourceCreateThumbnailAtIndex(source: *const c_void, index: usize, options: *const AnyObject) -> *const CGImage;
    fn CGImageSourceCopyPropertiesAtIndex(source: *const c_void, index: usize, options: *const AnyObject) -> *const AnyObject;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *const c_void);
}

unsafe extern "C" {
    static _NSConcreteStackBlock: [u8; 0];
}

const BLOCK_HAS_SIGNATURE: c_int = 1 << 30;

#[repr(C)]
struct BlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
    signature: *const c_char,
}

unsafe impl Sync for BlockDescriptor {}

#[repr(C)]
struct IdBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: *const c_void,
    descriptor: *const BlockDescriptor,
    id: u64,
}

unsafe impl RefEncode for IdBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

const fn descriptor(signature: &'static std::ffi::CStr) -> BlockDescriptor {
    BlockDescriptor {
        reserved: 0,
        size: std::mem::size_of::<IdBlock>() as c_ulong,
        signature: signature.as_ptr(),
    }
}

static ACCESS_DESCRIPTOR: BlockDescriptor = descriptor(c"v16@?0q8");
static IMAGE_DESCRIPTOR: BlockDescriptor = descriptor(c"v24@?0@8@16");
static VIDEO_DESCRIPTOR: BlockDescriptor = descriptor(c"v32@?0@8@16@24");
static DATA_DESCRIPTOR: BlockDescriptor = descriptor(c"v16@?0@8");
static IMAGE_DATA_DESCRIPTOR: BlockDescriptor = descriptor(c"v40@?0@8@16I24@32");
#[cfg(target_arch = "aarch64")]
static PROGRESS_DESCRIPTOR: BlockDescriptor = descriptor(c"v40@?0d8@16^B24@32");
#[cfg(not(target_arch = "aarch64"))]
static PROGRESS_DESCRIPTOR: BlockDescriptor = descriptor(c"v40@?0d8@16^c24@32");
static ENCODE_DESCRIPTOR: BlockDescriptor = descriptor(c"v8@?0");

fn id_block(id: u64, invoke: *const c_void, descriptor: &'static BlockDescriptor) -> IdBlock {
    IdBlock {
        isa: (&raw const _NSConcreteStackBlock).cast(),
        flags: BLOCK_HAS_SIGNATURE,
        reserved: 0,
        invoke,
        descriptor,
        id,
    }
}

/// what is waiting for a handler, by request number
struct Pending {
    access: HashMap<u64, AccessSink>,
    images: HashMap<u64, ImageRequest>,
    exports: HashMap<u64, Export>,
    resources: HashMap<u64, ResourceLoad>,
    image_data: HashMap<u64, ImageDataSink>,
    next: u64,
}

/// a resource data request, the hub request answers the feature
struct ResourceLoad {
    request: Arc<ResourceDataRequest>,
    ph_id: i32,
}

struct ImageRequest {
    sink: Option<ImageSink>,
    queue: VecDeque<ImageDelivery>,
    ph_id: i32,
    cancel_asked: bool,
    progress: Option<LibraryProgress>,
}

struct Export {
    asset_id: String,
    directory: PathBuf,
    done: Option<ExportSink>,
    stage: ExportStage,
    cancel_asked: bool,
    progress: Option<ExportProgressSink>,
}

enum ExportStage {
    Loading { ph_id: i32 },
    Copying(Arc<AtomicBool>),
    Encoding(ExportSession),
}

struct ExportSession(Retained<AnyObject>);

unsafe impl Send for ExportSession {}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn with_pending<T>(f: impl FnOnce(&mut Pending) -> T) -> T {
    let mut guard = PENDING.lock();
    let pending = guard.get_or_insert_with(|| Pending {
        access: HashMap::new(),
        images: HashMap::new(),
        exports: HashMap::new(),
        resources: HashMap::new(),
        image_data: HashMap::new(),
        next: 1,
    });
    f(pending)
}

/// runs a sink and stops a panic from unwinding into photokit (that aborts the process)
fn guarded(what: &'static str, f: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
        tracing::error!(target: "riseonly", what, "photo library sink panicked");
    }
}

pub(super) struct NativePhotoLibrary;

impl PhotoLibrarySource for NativePhotoLibrary {
    fn asset_collection(&self, id: &str) -> Result<Option<LibraryAssetCollection>, PhotoLibraryUnavailable> {
        Ok(collection_by_id(id)?.map(|collection| library_collection(&collection)))
    }

    fn smart_albums(
        &self,
        subtype: LibrarySmartAlbum,
    ) -> Result<Vec<LibraryAssetCollection>, PhotoLibraryUnavailable> {
        collections_of(PH_COLLECTION_TYPE_SMART_ALBUM, subtype.ph_subtype())
    }

    fn user_albums(&self) -> Result<Vec<LibraryAssetCollection>, PhotoLibraryUnavailable> {
        collections_of(PH_COLLECTION_TYPE_ALBUM, PH_COLLECTION_SUBTYPE_ANY)
    }

    fn collection_page(
        &self,
        collection: &LibraryAssetCollection,
        kind: LibraryMediaKind,
        offset: usize,
        limit: usize,
    ) -> Result<AssetPage, PhotoLibraryUnavailable> {
        let result = cached_collection_fetch(&collection.id, kind)?;
        let total = fetch_count(&result);
        let range = page_range(total, offset, limit);
        Ok(page_of(assets_at(&result, range.clone()), total, &range))
    }

    fn access_level(&self) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
        level_of(PH_ACCESS_LEVEL_READ_WRITE)
    }

    fn request_access(&self, done: AccessSink) -> Result<(), PhotoLibraryUnavailable> {
        request_level(PH_ACCESS_LEVEL_READ_WRITE, done)
    }

    fn add_access_level(&self) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
        level_of(PH_ACCESS_LEVEL_ADD_ONLY)
    }

    fn request_add_access(&self, done: AccessSink) -> Result<(), PhotoLibraryUnavailable> {
        request_level(PH_ACCESS_LEVEL_ADD_ONLY, done)
    }

    fn save_to_library(&self, request: LibrarySaveRequest, done: SaveSink) {
        start_save(request, done);
    }

    fn count(&self, kind: LibraryMediaKind) -> Result<usize, PhotoLibraryUnavailable> {
        let result = cached_fetch(kind)?;
        Ok(fetch_count(&result))
    }

    fn assets(
        &self,
        kind: LibraryMediaKind,
        range: Range<usize>,
    ) -> Result<Vec<LibraryAsset>, PhotoLibraryUnavailable> {
        let result = cached_fetch(kind)?;
        let total = fetch_count(&result);
        Ok(assets_at(&result, range.start.min(total)..range.end.min(total)))
    }

    fn page(
        &self,
        kind: LibraryMediaKind,
        offset: usize,
        limit: usize,
    ) -> Result<AssetPage, PhotoLibraryUnavailable> {
        let result = cached_fetch(kind)?;
        let total = fetch_count(&result);
        let range = page_range(total, offset, limit);
        Ok(page_of(assets_at(&result, range.clone()), total, &range))
    }

    fn assets_created_after(
        &self,
        kind: LibraryMediaKind,
        after_ms: i64,
        limit: usize,
    ) -> Result<Vec<LibraryAssetDetails>, PhotoLibraryUnavailable> {
        let result = fetch_created_after(kind, after_ms, limit)?;
        let total = fetch_count(&result);
        Ok((0..total)
            .map(|index| {
                let asset: Retained<AnyObject> = unsafe { msg_send![&*result, objectAtIndex: index] };
                asset_details(&asset)
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
        self.request_image_with_progress(id, asset_id, spec, sink, None)
    }

    fn request_image_with_progress(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        spec: ImageSpec,
        sink: ImageSink,
        progress: Option<LibraryProgress>,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let asset = asset_by_id(asset_id)?;
        let manager = image_manager()?;
        let options = new_object(c"PHImageRequestOptions")?;
        let delivery_mode: isize = match spec.delivery {
            ImageDeliveryMode::Opportunistic => 0,
            ImageDeliveryMode::HighQuality => 1,
        };
        let content_mode: isize = match spec.mode {
            ImageContentMode::AspectFit => 0,
            ImageContentMode::AspectFill => 1,
        };
        unsafe {
            let _: () = msg_send![&*options, setDeliveryMode: delivery_mode];
            let _: () = msg_send![&*options, setResizeMode: 1isize];
            let _: () = msg_send![&*options, setNetworkAccessAllowed: spec.allow_network];
            let _: () = msg_send![&*options, setSynchronous: false];
        }
        if progress.is_some() {
            set_progress_handler(&options, id.0, image_progress as *const c_void);
        }
        with_pending(|p| {
            p.images.insert(
                id.0,
                ImageRequest {
                    sink: Some(sink),
                    queue: VecDeque::new(),
                    ph_id: PH_INVALID_IMAGE_REQUEST_ID,
                    cancel_asked: false,
                    progress,
                },
            );
        });
        let block = id_block(id.0, image_answered as *const c_void, &IMAGE_DESCRIPTOR);
        let target = NSSize::new(f64::from(spec.width), f64::from(spec.height));
        let ph_id: i32 = unsafe {
            msg_send![
                &*manager.0,
                requestImageForAsset: &*asset,
                targetSize: target,
                contentMode: content_mode,
                options: &*options,
                resultHandler: &raw const block
            ]
        };
        let cancel_now = with_pending(|p| {
            p.images.get_mut(&id.0).is_some_and(|request| {
                request.ph_id = ph_id;
                request.cancel_asked
            })
        });
        if cancel_now {
            cancel_ph_request(&manager.0, ph_id);
        }
        Ok(())
    }

    fn cancel_image(&self, id: ImageRequestId) {
        let ph_id = with_pending(|p| {
            let request = p.images.get_mut(&id.0)?;
            request.cancel_asked = true;
            Some(request.ph_id).filter(|ph_id| *ph_id != PH_INVALID_IMAGE_REQUEST_ID)
        });
        let (Some(ph_id), Ok(manager)) = (ph_id, image_manager()) else {
            return;
        };
        cancel_ph_request(&manager.0, ph_id);
    }

    fn export_video(
        &self,
        asset_id: &str,
        directory: &Path,
        done: ExportSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        self.start_export(ImageRequestId::next(), asset_id, directory, done)
    }

    fn start_export(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        directory: &Path,
        done: ExportSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        self.start_export_with_progress(id, asset_id, directory, done, None)
    }

    fn start_export_with_progress(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        directory: &Path,
        done: ExportSink,
        progress: Option<ExportProgressSink>,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let asset = asset_by_id(asset_id)?;
        let media_type: isize = unsafe { msg_send![&*asset, mediaType] };
        if LibraryAssetKind::from_ph_media_type(media_type as i64) != LibraryAssetKind::Video {
            return Err(PhotoLibraryUnavailable::NotAVideo(asset_id.to_owned()));
        }
        let manager = default_manager()?;
        let options = new_object(c"PHVideoRequestOptions")?;
        unsafe {
            let _: () = msg_send![&*options, setDeliveryMode: PH_VIDEO_DELIVERY_HIGH_QUALITY];
            let _: () = msg_send![&*options, setNetworkAccessAllowed: true];
            let _: () = msg_send![&*options, setVersion: PH_VIDEO_VERSION_CURRENT];
        }
        if progress.is_some() {
            set_progress_handler(&options, id.0, video_progress as *const c_void);
        }
        with_pending(|p| {
            p.exports.insert(
                id.0,
                Export {
                    asset_id: asset_id.to_owned(),
                    directory: directory.to_path_buf(),
                    done: Some(done),
                    stage: ExportStage::Loading {
                        ph_id: PH_INVALID_IMAGE_REQUEST_ID,
                    },
                    cancel_asked: false,
                    progress,
                },
            );
        });
        let block = id_block(id.0, video_answered as *const c_void, &VIDEO_DESCRIPTOR);
        let ph_id: i32 = unsafe {
            msg_send![
                &*manager,
                requestAVAssetForVideo: &*asset,
                options: &*options,
                resultHandler: &raw const block
            ]
        };
        let cancel_now = with_pending(|p| {
            p.exports.get_mut(&id.0).is_some_and(|export| {
                if let ExportStage::Loading { ph_id: slot } = &mut export.stage {
                    *slot = ph_id;
                    export.cancel_asked
                } else {
                    false
                }
            })
        });
        if cancel_now {
            cancel_ph_request(&manager, ph_id);
        }
        Ok(())
    }

    fn cancel_export(&self, id: ImageRequestId) {
        enum Stop {
            Request(i32),
            Copy(Arc<AtomicBool>),
            Session(Retained<AnyObject>),
        }
        let stop = with_pending(|p| {
            let export = p.exports.get_mut(&id.0)?;
            export.cancel_asked = true;
            match &export.stage {
                ExportStage::Loading { ph_id } if *ph_id != PH_INVALID_IMAGE_REQUEST_ID => {
                    Some(Stop::Request(*ph_id))
                }
                ExportStage::Loading { .. } => None,
                ExportStage::Copying(flag) => Some(Stop::Copy(flag.clone())),
                ExportStage::Encoding(session) => Some(Stop::Session(session.0.clone())),
            }
        });
        match stop {
            None => {}
            Some(Stop::Request(ph_id)) => {
                if let Ok(manager) = default_manager() {
                    cancel_ph_request(&manager, ph_id);
                }
            }
            Some(Stop::Copy(flag)) => flag.store(true, Ordering::SeqCst),
            Some(Stop::Session(session)) => unsafe {
                let _: () = msg_send![&*session, cancelExport];
            },
        }
    }

    fn asset_resources(
        &self,
        asset_id: &str,
    ) -> Result<Vec<LibraryAssetResource>, PhotoLibraryUnavailable> {
        let asset = asset_by_id(asset_id)?;
        Ok(resources_of(&asset)?
            .iter()
            .enumerate()
            .map(|(index, resource)| library_resource(index, resource))
            .collect())
    }

    fn is_animated_image(&self, asset_id: &str) -> Result<bool, PhotoLibraryUnavailable> {
        let asset = asset_by_id(asset_id)?;
        let style: isize = unsafe { msg_send![&*asset, playbackStyle] };
        Ok(style == PH_PLAYBACK_STYLE_IMAGE_ANIMATED)
    }

    fn request_resource_data(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        resource: &LibraryAssetResource,
        request: Arc<ResourceDataRequest>,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let asset = asset_by_id(asset_id)?;
        let resources = resources_of(&asset)?;
        let target = resources.get(resource.index).ok_or_else(|| {
            PhotoLibraryUnavailable::SystemRefused(format!(
                "asset {asset_id} has no resource {}",
                resource.index
            ))
        })?;
        let manager = resource_manager()?;
        let options = new_object(c"PHAssetResourceRequestOptions")?;
        unsafe {
            let _: () = msg_send![&*options, setNetworkAccessAllowed: true];
        }
        with_pending(|p| {
            p.resources.insert(
                id.0,
                ResourceLoad {
                    request,
                    ph_id: PH_INVALID_IMAGE_REQUEST_ID,
                },
            );
        });
        let chunk_block = id_block(id.0, resource_chunk as *const c_void, &DATA_DESCRIPTOR);
        let done_block = id_block(id.0, resource_done as *const c_void, &DATA_DESCRIPTOR);
        let ph_id: i32 = unsafe {
            msg_send![
                &*manager,
                requestDataForAssetResource: &**target,
                options: &*options,
                dataReceivedHandler: &raw const chunk_block,
                completionHandler: &raw const done_block
            ]
        };
        let cancel_now = with_pending(|p| {
            p.resources.get_mut(&id.0).is_some_and(|load| {
                load.ph_id = ph_id;
                load.request.is_finished()
            })
        });
        if cancel_now {
            cancel_data_request(&manager, ph_id);
        }
        Ok(())
    }

    fn cancel_resource_data(&self, id: ImageRequestId) {
        let ph_id = with_pending(|p| {
            p.resources
                .get(&id.0)
                .map(|load| load.ph_id)
                .filter(|ph_id| *ph_id != PH_INVALID_IMAGE_REQUEST_ID)
        });
        let (Some(ph_id), Ok(manager)) = (ph_id, resource_manager()) else {
            return;
        };
        cancel_data_request(&manager, ph_id);
    }

    fn request_image_data(
        &self,
        id: ImageRequestId,
        asset_id: &str,
        done: ImageDataSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        let asset = asset_by_id(asset_id)?;
        let manager = default_manager()?;
        let options = new_object(c"PHImageRequestOptions")?;
        unsafe {
            let _: () = msg_send![&*options, setDeliveryMode: 1isize];
            let _: () = msg_send![&*options, setNetworkAccessAllowed: true];
            let _: () = msg_send![&*options, setVersion: PH_IMAGE_VERSION_CURRENT];
            let _: () = msg_send![&*options, setSynchronous: false];
        }
        with_pending(|p| {
            p.image_data.insert(id.0, done);
        });
        let block = id_block(id.0, image_data_answered as *const c_void, &IMAGE_DATA_DESCRIPTOR);
        let _ph_id: i32 = unsafe {
            msg_send![
                &*manager,
                requestImageDataAndOrientationForAsset: &*asset,
                options: &*options,
                resultHandler: &raw const block
            ]
        };
        Ok(())
    }

    fn watch_changes(&self, sink: ChangeSink) -> Result<ChangeWatch, PhotoLibraryUnavailable> {
        let subscription = {
            let mut subscribers = SUBSCRIBERS.lock();
            subscribers.next += 1;
            let id = subscribers.next;
            subscribers.sinks.push((id, sink));
            Subscription(id)
        };
        if self.access_level()?.can_browse() {
            ensure_library_observer()?;
        }
        Ok(ChangeWatch::new(subscription))
    }
}

fn cancel_ph_request(manager: &AnyObject, ph_id: i32) {
    unsafe {
        let _: () = msg_send![manager, cancelImageRequest: ph_id];
    }
}

fn default_manager() -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    let manager = class(c"PHImageManager")?;
    let shared: Option<Retained<AnyObject>> = unsafe { msg_send![manager, defaultManager] };
    shared.ok_or_else(|| PhotoLibraryUnavailable::SystemRefused("defaultManager is nil".into()))
}

/// the default asset resource manager
fn resource_manager() -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    let manager = class(c"PHAssetResourceManager")?;
    let shared: Option<Retained<AnyObject>> = unsafe { msg_send![manager, defaultManager] };
    shared.ok_or_else(|| PhotoLibraryUnavailable::SystemRefused("resource manager is nil".into()))
}

fn cancel_data_request(manager: &AnyObject, ph_id: i32) {
    unsafe {
        let _: () = msg_send![manager, cancelDataRequest: ph_id];
    }
}

/// resources of an asset in their order
fn resources_of(asset: &AnyObject) -> Result<Vec<Retained<AnyObject>>, PhotoLibraryUnavailable> {
    let resource_class = class(c"PHAssetResource")?;
    unsafe {
        let list: Retained<AnyObject> = msg_send![resource_class, assetResourcesForAsset: asset];
        let count: usize = msg_send![&*list, count];
        Ok((0..count)
            .map(|index| {
                let resource: Retained<AnyObject> = msg_send![&*list, objectAtIndex: index];
                resource
            })
            .collect())
    }
}

fn library_resource(index: usize, resource: &AnyObject) -> LibraryAssetResource {
    let key = NSString::from_str("type");
    unsafe {
        let raw: Option<Retained<NSNumber>> = msg_send![resource, valueForKey: &*key];
        let raw_type = raw.map_or(0, |number| number.integerValue());
        let uti: Retained<NSString> = msg_send![resource, uniformTypeIdentifier];
        let name: Retained<NSString> = msg_send![resource, originalFilename];
        LibraryAssetResource {
            resource_type: LibraryAssetResourceType::from_ph_resource_type(raw_type as i64),
            uniform_type_identifier: uti.to_string(),
            original_filename: name.to_string(),
            index,
        }
    }
}

/// copies the bytes of an NSData
fn bytes_of(data: &AnyObject) -> Vec<u8> {
    unsafe {
        let length: usize = msg_send![data, length];
        let bytes: *const c_void = msg_send![data, bytes];
        if length == 0 || bytes.is_null() {
            return Vec::new();
        }
        std::slice::from_raw_parts(bytes.cast::<u8>(), length).to_vec()
    }
}

unsafe extern "C" fn resource_chunk(block: *const IdBlock, data: *mut AnyObject) {
    let (id, data) = unsafe { ((*block).id, data.as_ref()) };
    let Some(data) = data else {
        return;
    };
    let Some(request) = with_pending(|p| p.resources.get(&id).map(|load| load.request.clone())) else {
        return;
    };
    let chunk = bytes_of(data);
    let mut keep = true;
    guarded("resource chunk", || keep = request.receive(&chunk));
    if keep {
        return;
    }
    let ph_id = with_pending(|p| p.resources.get(&id).map(|load| load.ph_id));
    if let (Some(ph_id), Ok(manager)) = (ph_id, resource_manager()) {
        if ph_id != PH_INVALID_IMAGE_REQUEST_ID {
            cancel_data_request(&manager, ph_id);
        }
    }
}

unsafe extern "C" fn resource_done(block: *const IdBlock, error: *mut AnyObject) {
    let (id, error) = unsafe { ((*block).id, error.cast::<NSError>().as_ref()) };
    let Some(load) = with_pending(|p| p.resources.remove(&id)) else {
        return;
    };
    let reason = error.map(|error| {
        PhotoLibraryUnavailable::SystemRefused(error.localizedDescription().to_string())
    });
    guarded("resource done", move || load.request.complete(reason));
}

unsafe extern "C" fn image_data_answered(
    block: *const IdBlock,
    data: *mut AnyObject,
    uti: *mut AnyObject,
    _orientation: u32,
    info: *mut AnyObject,
) {
    let (id, data, uti, info) = unsafe {
        (
            (*block).id,
            data.as_ref(),
            uti.cast::<NSString>().as_ref(),
            info.as_ref(),
        )
    };
    let Some(done) = with_pending(|p| p.image_data.remove(&id)) else {
        return;
    };
    let (cancelled_key, error_key) = unsafe { (PHImageCancelledKey, PHImageErrorKey) };
    let result = match data {
        Some(data) => Ok((bytes_of(data), uti.map(|uti| uti.to_string()))),
        None if info.is_some_and(|info| flag(info, cancelled_key)) => {
            Err(PhotoLibraryUnavailable::Cancelled)
        }
        None => Err(PhotoLibraryUnavailable::SystemRefused(
            info.and_then(|info| error_text(info, error_key))
                .unwrap_or_else(|| "no image data for the asset".to_owned()),
        )),
    };
    guarded("image data", move || done(result));
}

struct FetchResult(Retained<AnyObject>);

unsafe impl Send for FetchResult {}

static FETCHES: Mutex<Option<HashMap<LibraryMediaKind, FetchResult>>> = Mutex::new(None);

fn cached_fetch(kind: LibraryMediaKind) -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    ensure_library_observer()?;
    let mut fetches = FETCHES.lock();
    let fetches = fetches.get_or_insert_with(HashMap::new);
    if let Some(result) = fetches.get(&kind) {
        return Ok(result.0.clone());
    }
    let result = fetch(kind)?;
    fetches.insert(kind, FetchResult(result.clone()));
    Ok(result)
}

fn fetch_count(result: &AnyObject) -> usize {
    unsafe { msg_send![result, count] }
}

fn assets_at(result: &AnyObject, range: Range<usize>) -> Vec<LibraryAsset> {
    range
        .map(|index| {
            let asset: Retained<AnyObject> = unsafe { msg_send![result, objectAtIndex: index] };
            library_asset(&asset)
        })
        .collect()
}

/// library change handler, each kept result moves to its after changes value
fn apply_change(change: &AnyObject) {
    if let Some(fetches) = FETCHES.lock().as_mut() {
        advance_results(change, fetches.values_mut());
    }
    if let Some(fetches) = COLLECTION_FETCHES.lock().as_mut() {
        advance_results(change, fetches.values_mut());
    }
}

fn advance_results<'a>(change: &AnyObject, results: impl Iterator<Item = &'a mut FetchResult>) {
    for result in results {
        let after: Option<Retained<AnyObject>> = unsafe {
            let details: Option<Retained<AnyObject>> =
                msg_send![change, changeDetailsForFetchResult: &*result.0];
            match details {
                Some(details) => msg_send![&*details, fetchResultAfterChanges],
                None => None,
            }
        };
        if let Some(after) = after {
            result.0 = after;
        }
    }
}

struct Subscribers {
    sinks: Vec<(u64, ChangeSink)>,
    next: u64,
}

static SUBSCRIBERS: Mutex<Subscribers> = Mutex::new(Subscribers {
    sinks: Vec::new(),
    next: 0,
});

/// removes its sink from the fan out when the watch drops
struct Subscription(u64);

impl Drop for Subscription {
    fn drop(&mut self) {
        SUBSCRIBERS.lock().sinks.retain(|(id, _)| *id != self.0);
    }
}

struct LibraryObserver(Retained<ChangeObserver>);

unsafe impl Send for LibraryObserver {}

static LIBRARY_OBSERVER: Mutex<Option<LibraryObserver>> = Mutex::new(None);

fn ensure_library_observer() -> Result<(), PhotoLibraryUnavailable> {
    let mut slot = LIBRARY_OBSERVER.lock();
    if slot.is_some() {
        return Ok(());
    }
    let library = shared_library()?;
    let observer = ChangeObserver::new();
    unsafe {
        let _: () = msg_send![&*library, registerChangeObserver: &*observer];
    }
    *slot = Some(LibraryObserver(observer));
    Ok(())
}

fn class(name: &'static std::ffi::CStr) -> Result<&'static AnyClass, PhotoLibraryUnavailable> {
    AnyClass::get(name).ok_or(PhotoLibraryUnavailable::FrameworkMissing("Photos.framework"))
}

fn new_object(name: &'static std::ffi::CStr) -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    let class = class(name)?;
    let object: Option<Retained<AnyObject>> = unsafe { msg_send![class, new] };
    object.ok_or_else(|| PhotoLibraryUnavailable::SystemRefused(format!("{name:?} new is nil")))
}

fn shared_library() -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    let library = class(c"PHPhotoLibrary")?;
    let shared: Option<Retained<AnyObject>> = unsafe { msg_send![library, sharedPhotoLibrary] };
    shared.ok_or_else(|| PhotoLibraryUnavailable::SystemRefused("sharedPhotoLibrary is nil".into()))
}

/// newest first fetch for the kind
fn fetch(kind: LibraryMediaKind) -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    let options = fetch_options(kind)?;
    let asset_class = class(c"PHAsset")?;
    let result: Option<Retained<AnyObject>> = unsafe { msg_send![asset_class, fetchAssetsWithOptions: &*options] };
    result.ok_or_else(|| PhotoLibraryUnavailable::SystemRefused("fetchAssets returned nil".into()))
}

fn fetch_options(kind: LibraryMediaKind) -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    let options = new_object(c"PHFetchOptions")?;
    let sort_class = class(c"NSSortDescriptor")?;
    let predicate_class = class(c"NSPredicate")?;
    let types: Vec<Retained<NSNumber>> = kind
        .ph_media_types()
        .iter()
        .map(|media_type| NSNumber::new_i64(*media_type))
        .collect();
    let format = vec!["mediaType == %@"; types.len()].join(" OR ");
    let arguments = NSArray::from_retained_slice(&types);
    unsafe {
        let sort: Retained<AnyObject> = msg_send![
            sort_class,
            sortDescriptorWithKey: &*NSString::from_str("creationDate"),
            ascending: false
        ];
        let predicate: Retained<AnyObject> = msg_send![
            predicate_class,
            predicateWithFormat: &*NSString::from_str(&format),
            argumentArray: &*arguments
        ];
        let sorts = NSArray::from_retained_slice(&[sort]);
        let _: () = msg_send![&*options, setSortDescriptors: &*sorts];
        let _: () = msg_send![&*options, setPredicate: &*predicate];
    }
    Ok(options)
}

const PH_COLLECTION_TYPE_ALBUM: isize = 1;
const PH_COLLECTION_TYPE_SMART_ALBUM: isize = 2;
const PH_COLLECTION_SUBTYPE_ANY: isize = isize::MAX;

static COLLECTION_FETCHES: Mutex<Option<HashMap<(String, LibraryMediaKind), FetchResult>>> = Mutex::new(None);

fn cached_collection_fetch(
    id: &str,
    kind: LibraryMediaKind,
) -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    ensure_library_observer()?;
    let key = (id.to_owned(), kind);
    if let Some(result) = COLLECTION_FETCHES.lock().as_ref().and_then(|fetches| fetches.get(&key)) {
        return Ok(result.0.clone());
    }
    let collection =
        collection_by_id(id)?.ok_or_else(|| PhotoLibraryUnavailable::AssetNotFound(id.to_owned()))?;
    let options = fetch_options(kind)?;
    let asset_class = class(c"PHAsset")?;
    let result: Option<Retained<AnyObject>> =
        unsafe { msg_send![asset_class, fetchAssetsInAssetCollection: &*collection, options: &*options] };
    let result = result.ok_or_else(|| PhotoLibraryUnavailable::SystemRefused("fetchAssetsInAssetCollection returned nil".into()))?;
    COLLECTION_FETCHES
        .lock()
        .get_or_insert_with(HashMap::new)
        .insert(key, FetchResult(result.clone()));
    Ok(result)
}

/// first asset collection with this local id, none if missing
fn collection_by_id(id: &str) -> Result<Option<Retained<AnyObject>>, PhotoLibraryUnavailable> {
    let collection_class = class(c"PHAssetCollection")?;
    let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
    Ok(unsafe {
        let none: Option<&AnyObject> = None;
        let result: Retained<AnyObject> =
            msg_send![collection_class, fetchAssetCollectionsWithLocalIdentifiers: &*ids, options: none];
        msg_send![&*result, firstObject]
    })
}

/// every collection of this type and subtype in order
fn collections_of(
    collection_type: isize,
    subtype: isize,
) -> Result<Vec<LibraryAssetCollection>, PhotoLibraryUnavailable> {
    let collection_class = class(c"PHAssetCollection")?;
    unsafe {
        let none: Option<&AnyObject> = None;
        let result: Option<Retained<AnyObject>> = msg_send![
            collection_class,
            fetchAssetCollectionsWithType: collection_type,
            subtype: subtype,
            options: none
        ];
        let result = result.ok_or_else(|| PhotoLibraryUnavailable::SystemRefused("fetchAssetCollections returned nil".into()))?;
        Ok((0..fetch_count(&result))
            .map(|index| {
                let collection: Retained<AnyObject> = msg_send![&*result, objectAtIndex: index];
                library_collection(&collection)
            })
            .collect())
    }
}

fn library_collection(collection: &AnyObject) -> LibraryAssetCollection {
    unsafe {
        let id: Retained<NSString> = msg_send![collection, localIdentifier];
        let title: Option<Retained<NSString>> = msg_send![collection, localizedTitle];
        LibraryAssetCollection {
            id: id.to_string(),
            localized_title: title.map(|title| title.to_string()),
        }
    }
}

const PH_ASSET_SOURCE_USER_LIBRARY: usize = 1;
const PH_MEDIA_SUBTYPE_PHOTO_SCREENSHOT: usize = 1 << 2;

/// plain coordinate as the location answers it
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Coordinate {
    latitude: f64,
    longitude: f64,
}

unsafe impl Encode for Coordinate {
    const ENCODING: Encoding = Encoding::Struct("CLLocationCoordinate2D", &[f64::ENCODING, f64::ENCODING]);
}

fn fetch_created_after(
    kind: LibraryMediaKind,
    after_ms: i64,
    limit: usize,
) -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    let options = new_object(c"PHFetchOptions")?;
    let sort_class = class(c"NSSortDescriptor")?;
    let predicate_class = class(c"NSPredicate")?;
    let asset_class = class(c"PHAsset")?;
    let date_class = class(c"NSDate")?;
    let number_class = class(c"NSNumber")?;
    let types = kind.ph_media_types();
    let format = format!(
        "creationDate > %@ AND ({})",
        vec!["mediaType == %@"; types.len()].join(" OR ")
    );
    unsafe {
        let since: Retained<AnyObject> =
            msg_send![date_class, dateWithTimeIntervalSince1970: after_ms as f64 / 1000.0];
        let mut arguments: Vec<Retained<AnyObject>> = vec![since];
        for media_type in types {
            let number: Retained<AnyObject> = msg_send![number_class, numberWithLongLong: *media_type];
            arguments.push(number);
        }
        let arguments = NSArray::from_retained_slice(&arguments);
        let sort: Retained<AnyObject> = msg_send![
            sort_class,
            sortDescriptorWithKey: &*NSString::from_str("creationDate"),
            ascending: false
        ];
        let predicate: Retained<AnyObject> = msg_send![
            predicate_class,
            predicateWithFormat: &*NSString::from_str(&format),
            argumentArray: &*arguments
        ];
        let sorts = NSArray::from_retained_slice(&[sort]);
        let _: () = msg_send![&*options, setSortDescriptors: &*sorts];
        let _: () = msg_send![&*options, setPredicate: &*predicate];
        let _: () = msg_send![&*options, setFetchLimit: limit];
        let _: () = msg_send![&*options, setIncludeAssetSourceTypes: PH_ASSET_SOURCE_USER_LIBRARY];
        let result: Option<Retained<AnyObject>> =
            msg_send![asset_class, fetchAssetsWithOptions: &*options];
        result.ok_or_else(|| PhotoLibraryUnavailable::SystemRefused("fetchAssets returned nil".into()))
    }
}

/// reads the asset location coordinate and whether its a screenshot
/// the scout checks the coordinate itself (valid and not 0 0)
fn asset_details(asset: &AnyObject) -> LibraryAssetDetails {
    let (place, subtypes) = unsafe {
        let location: Option<Retained<AnyObject>> = msg_send![asset, location];
        let place = location.map(|location| {
            let coordinate: Coordinate = msg_send![&*location, coordinate];
            LibraryAssetPlace {
                latitude: coordinate.latitude,
                longitude: coordinate.longitude,
            }
        });
        let subtypes: usize = msg_send![asset, mediaSubtypes];
        (place, subtypes)
    };
    LibraryAssetDetails {
        asset: library_asset(asset),
        place,
        is_screenshot: subtypes & PH_MEDIA_SUBTYPE_PHOTO_SCREENSHOT != 0,
    }
}

fn asset_by_id(asset_id: &str) -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    let asset_class = class(c"PHAsset")?;
    let ids = NSArray::from_retained_slice(&[NSString::from_str(asset_id)]);
    let asset: Option<Retained<AnyObject>> = unsafe {
        let none: Option<&AnyObject> = None;
        let result: Retained<AnyObject> =
            msg_send![asset_class, fetchAssetsWithLocalIdentifiers: &*ids, options: none];
        msg_send![&*result, firstObject]
    };
    asset.ok_or_else(|| PhotoLibraryUnavailable::AssetNotFound(asset_id.to_owned()))
}

/// library asset made from a photokit asset
fn library_asset(asset: &AnyObject) -> LibraryAsset {
    unsafe {
        let id: Retained<NSString> = msg_send![asset, localIdentifier];
        let media_type: isize = msg_send![asset, mediaType];
        let width: usize = msg_send![asset, pixelWidth];
        let height: usize = msg_send![asset, pixelHeight];
        let duration: f64 = msg_send![asset, duration];
        let created: Option<Retained<AnyObject>> = msg_send![asset, creationDate];
        let created_at_ms = created.and_then(|date| {
            let seconds: f64 = msg_send![&*date, timeIntervalSince1970];
            epoch_ms(seconds)
        });
        LibraryAsset {
            id: id.to_string(),
            kind: LibraryAssetKind::from_ph_media_type(media_type as i64),
            pixel_width: u32::try_from(width).unwrap_or(u32::MAX),
            pixel_height: u32::try_from(height).unwrap_or(u32::MAX),
            duration_secs: duration,
            created_at_ms,
        }
    }
}

struct SharedManager(Retained<AnyObject>);

unsafe impl Send for SharedManager {}
unsafe impl Sync for SharedManager {}

static IMAGE_MANAGER: OnceLock<Option<SharedManager>> = OnceLock::new();

/// the caching image manager of the fetcher, high quality caching off
fn image_manager() -> Result<&'static SharedManager, PhotoLibraryUnavailable> {
    IMAGE_MANAGER
        .get_or_init(|| {
            let manager = new_object(c"PHCachingImageManager").ok()?;
            unsafe {
                let _: () = msg_send![&*manager, setAllowsCachingHighQualityImages: false];
            }
            Some(SharedManager(manager))
        })
        .as_ref()
        .ok_or(PhotoLibraryUnavailable::FrameworkMissing("PHCachingImageManager"))
}

unsafe extern "C" fn access_answered(block: *const IdBlock, status: isize) {
    let id = unsafe { (*block).id };
    let Some(done) = with_pending(|p| p.access.remove(&id)) else {
        return;
    };
    guarded("access", move || done(PhotoLibraryAccessLevel::from_ph_status(status as i64)));
}

/// handler runs only while photokit downloads from icloud, on an arbitrary serial queue
fn set_progress_handler(options: &AnyObject, id: u64, invoke: *const c_void) {
    let block = id_block(id, invoke, &PROGRESS_DESCRIPTOR);
    unsafe {
        let _: () = msg_send![options, setProgressHandler: &raw const block];
    }
}

unsafe extern "C" fn image_progress(
    block: *const IdBlock,
    progress: f64,
    error: *mut AnyObject,
    _stop: *mut Bool,
    _info: *mut AnyObject,
) {
    let id = unsafe { (*block).id };
    let Some(fraction) = library_progress_fraction(progress, !error.is_null()) else {
        return;
    };
    let callback = with_pending(|p| {
        let request = p.images.get(&id).filter(|request| !request.cancel_asked)?;
        request.progress.clone()
    });
    if let Some(callback) = callback {
        guarded("progress", move || callback(fraction));
    }
}

unsafe extern "C" fn video_progress(
    block: *const IdBlock,
    progress: f64,
    error: *mut AnyObject,
    _stop: *mut Bool,
    _info: *mut AnyObject,
) {
    let id = unsafe { (*block).id };
    let Some(fraction) = library_progress_fraction(progress, !error.is_null()) else {
        return;
    };
    let callback = with_pending(|p| {
        let export = p.exports.get(&id).filter(|export| !export.cancel_asked)?;
        export.progress.clone()
    });
    if let Some(callback) = callback {
        guarded("progress", move || callback(ExportProgress::Fetching(fraction)));
    }
}

unsafe extern "C" fn image_answered(block: *const IdBlock, image: *mut AnyObject, info: *mut AnyObject) {
    let (id, image, info) = unsafe { ((*block).id, image.as_ref(), info.as_ref()) };
    let (cancelled_key, degraded_key, error_key) =
        unsafe { (PHImageCancelledKey, PHImageResultIsDegradedKey, PHImageErrorKey) };
    let cancelled = info.is_some_and(|info| flag(info, cancelled_key));
    let degraded = info.is_some_and(|info| flag(info, degraded_key));
    let error = info.and_then(|info| error_text(info, error_key));
    let pixels = if cancelled { None } else { image.and_then(bgra_of) };
    let delivery = ImageDelivery {
        request: ImageRequestId(id),
        error: if pixels.is_none() && !cancelled {
            Some(error.unwrap_or_else(|| "no image for the asset".to_owned()))
        } else {
            None
        },
        image: pixels,
        degraded,
        cancelled,
    };
    let mut held = with_pending(|p| {
        let request = p.images.get_mut(&id)?;
        request.queue.push_back(delivery);
        request.sink.take()
    });
    while held.is_some() {
        let next = with_pending(|p| match p.images.get_mut(&id) {
            None => None,
            Some(request) => {
                let next = request.queue.pop_front();
                if next.is_none() {
                    request.sink = held.take();
                }
                next
            }
        });
        let Some(delivery) = next else {
            return;
        };
        let finished = delivery.is_final();
        if let Some(sink) = held.as_mut() {
            guarded("image", || sink(delivery));
        }
        if finished {
            with_pending(|p| p.images.remove(&id));
            return;
        }
    }
}

/// ends an export, the entry leaves the table and its sink runs once
fn finish_video(id: u64, result: Result<PathBuf, PhotoLibraryUnavailable>) {
    let Some(done) = with_pending(|p| p.exports.remove(&id).and_then(|mut export| export.done.take()))
    else {
        return;
    };
    guarded("export", move || done(result));
}

unsafe extern "C" fn video_answered(
    block: *const IdBlock,
    asset: *mut AnyObject,
    _audio_mix: *mut AnyObject,
    info: *mut AnyObject,
) {
    let (id, asset, info) = unsafe { ((*block).id, asset.as_ref(), info.as_ref()) };
    let (cancelled_key, error_key) = unsafe { (PHImageCancelledKey, PHImageErrorKey) };
    let target = with_pending(|p| {
        p.exports
            .get(&id)
            .map(|export| (export.asset_id.clone(), export.directory.clone(), export.cancel_asked))
    });
    let Some((asset_id, directory, cancel_asked)) = target else {
        return;
    };
    let Some(asset) = asset else {
        let result = if info.is_some_and(|info| flag(info, cancelled_key)) {
            Err(PhotoLibraryUnavailable::Cancelled)
        } else if let Some(reason) = info.and_then(|info| error_text(info, error_key)) {
            Err(PhotoLibraryUnavailable::SystemRefused(reason))
        } else {
            Err(PhotoLibraryUnavailable::VideoUnavailable(asset_id))
        };
        finish_video(id, result);
        return;
    };
    if cancel_asked {
        finish_video(id, Err(PhotoLibraryUnavailable::Cancelled));
        return;
    }
    let result = match url_asset_path(asset) {
        Some(source) => copy_video(id, &asset_id, &directory, source),
        None => encode_video(id, &asset_id, &directory, asset),
    };
    if let Err(reason) = result {
        finish_video(id, Err(reason));
    }
}

fn url_asset_path(asset: &AnyObject) -> Option<PathBuf> {
    let url_asset = AnyClass::get(c"AVURLAsset")?;
    unsafe {
        let is_url: bool = msg_send![asset, isKindOfClass: url_asset];
        if !is_url {
            return None;
        }
        let url: Retained<NSURL> = msg_send![asset, URL];
        let path: Option<Retained<NSString>> = msg_send![&*url, path];
        path.map(|path| PathBuf::from(path.to_string()))
    }
}

fn prepared_target(directory: &Path, name: &str) -> Result<PathBuf, PhotoLibraryUnavailable> {
    std::fs::create_dir_all(directory)
        .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))?;
    let path = directory.join(name);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(PhotoLibraryUnavailable::SystemRefused(error.to_string())),
    }
}

fn copy_video(
    id: u64,
    asset_id: &str,
    directory: &Path,
    source: PathBuf,
) -> Result<(), PhotoLibraryUnavailable> {
    let original = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let target = prepared_target(directory, &export_file_name(asset_id, &original))?;
    let stop = Arc::new(AtomicBool::new(false));
    let started = with_pending(|p| match p.exports.get_mut(&id) {
        Some(export) if !export.cancel_asked => {
            export.stage = ExportStage::Copying(stop.clone());
            true
        }
        _ => false,
    });
    if !started {
        return Err(PhotoLibraryUnavailable::Cancelled);
    }
    std::thread::Builder::new()
        .name("rise-photo-export".to_owned())
        .spawn(move || {
            let copied = std::fs::copy(&source, &target)
                .map(|_| ())
                .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()));
            let result = match copied {
                Ok(()) if stop.load(Ordering::SeqCst) => {
                    let _ = std::fs::remove_file(&target);
                    Err(PhotoLibraryUnavailable::Cancelled)
                }
                Ok(()) => Ok(target),
                Err(reason) => {
                    let _ = std::fs::remove_file(&target);
                    Err(reason)
                }
            };
            finish_video(id, result);
        })
        .map(drop)
        .map_err(|error| PhotoLibraryUnavailable::SystemRefused(error.to_string()))
}

fn encode_video(
    id: u64,
    asset_id: &str,
    directory: &Path,
    asset: &AnyObject,
) -> Result<(), PhotoLibraryUnavailable> {
    let target = prepared_target(directory, &export_file_name(asset_id, EXPORT_NAME))?;
    let target_text = target
        .to_str()
        .ok_or_else(|| PhotoLibraryUnavailable::SystemRefused("export path is not UTF-8".into()))?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(target_text));
    let session_class = class(c"AVAssetExportSession")?;
    let session: Retained<AnyObject> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![session_class, alloc];
        let session: Option<Retained<AnyObject>> = msg_send![
            allocated,
            initWithAsset: asset,
            presetName: AVAssetExportPresetHighestQuality
        ];
        let session = session.ok_or_else(|| {
            PhotoLibraryUnavailable::SystemRefused("no export session for this video".into())
        })?;
        let supported: Retained<NSArray<NSString>> = msg_send![&*session, supportedFileTypes];
        if supported.count() > 0 && !supported.containsObject(AVFileTypeMPEG4) {
            return Err(PhotoLibraryUnavailable::SystemRefused(
                "this video cannot be exported as MPEG-4".into(),
            ));
        }
        let _: () = msg_send![&*session, setOutputURL: &*url];
        let _: () = msg_send![&*session, setOutputFileType: AVFileTypeMPEG4];
        let _: () = msg_send![&*session, setShouldOptimizeForNetworkUse: Bool::YES];
        session
    };
    let started = with_pending(|p| match p.exports.get_mut(&id) {
        Some(export) if !export.cancel_asked => {
            export.stage = ExportStage::Encoding(ExportSession(session.clone()));
            true
        }
        _ => false,
    });
    if !started {
        return Err(PhotoLibraryUnavailable::Cancelled);
    }
    let block = id_block(id, encode_answered as *const c_void, &ENCODE_DESCRIPTOR);
    unsafe {
        let _: () = msg_send![&*session, exportAsynchronouslyWithCompletionHandler: &raw const block];
    }
    if let Some(progress) = with_pending(|p| p.exports.get(&id).and_then(|export| export.progress.clone())) {
        watch_encoding(id, ExportSession(session), progress);
    }
    Ok(())
}

fn watch_encoding(id: u64, session: ExportSession, progress: ExportProgressSink) {
    let spawned = std::thread::Builder::new().name("riseonly-export-progress".into()).spawn(move || {
        let session = session;
        let mut gate = EncodeProgressGate::default();
        loop {
            let encoding = with_pending(|p| {
                p.exports
                    .get(&id)
                    .is_some_and(|export| !export.cancel_asked && matches!(export.stage, ExportStage::Encoding(_)))
            });
            if !encoding {
                return;
            }
            let fraction: f32 = unsafe { msg_send![&*session.0, progress] };
            if let Some(fraction) = gate.report(f64::from(fraction)) {
                guarded("progress", || progress(ExportProgress::Encoding(fraction)));
            }
            std::thread::sleep(ENCODE_PROGRESS_POLL);
        }
    });
    if let Err(error) = spawned {
        tracing::warn!(target: "riseonly", %error, "export progress watcher not started");
    }
}

unsafe extern "C" fn encode_answered(block: *const IdBlock) {
    let id = unsafe { (*block).id };
    let state = with_pending(|p| {
        let export = p.exports.get(&id)?;
        let ExportStage::Encoding(session) = &export.stage else {
            return None;
        };
        Some((session.0.clone(), export.directory.join(export_file_name(&export.asset_id, EXPORT_NAME))))
    });
    let Some((session, target)) = state else {
        return;
    };
    let (status, error): (isize, Option<Retained<NSError>>) =
        unsafe { (msg_send![&*session, status], msg_send![&*session, error]) };
    let result = match status {
        AV_EXPORT_COMPLETED => Ok(target),
        AV_EXPORT_CANCELLED => {
            let _ = std::fs::remove_file(&target);
            Err(PhotoLibraryUnavailable::Cancelled)
        }
        _ => {
            let _ = std::fs::remove_file(&target);
            Err(PhotoLibraryUnavailable::SystemRefused(error.map_or_else(
                || format!("export session ended with status {status}"),
                |error| error.localizedDescription().to_string(),
            )))
        }
    };
    finish_video(id, result);
}

fn flag(info: &AnyObject, key: &NSString) -> bool {
    unsafe {
        let value: Option<Retained<AnyObject>> = msg_send![info, objectForKey: key];
        value.is_some_and(|number| {
            let on: bool = msg_send![&*number, boolValue];
            on
        })
    }
}

fn error_text(info: &AnyObject, key: &NSString) -> Option<String> {
    unsafe {
        let value: Option<Retained<NSError>> = msg_send![info, objectForKey: key];
        value.map(|error| error.localizedDescription().to_string())
    }
}

fn bgra_of(image: &AnyObject) -> Option<LibraryImage> {
    unsafe {
        let none: Option<&AnyObject> = None;
        let cg: *const CGImage = msg_send![
            image,
            CGImageForProposedRect: std::ptr::null_mut::<NSRect>(),
            context: none,
            hints: none
        ];
        bgra_of_cg_image(cg)
    }
}

/// draws a cgimage into a bgra bitmap of its pixel size
/// # Safety
/// cg is null or a cgimage alive for the whole call
unsafe fn bgra_of_cg_image(cg: *const CGImage) -> Option<LibraryImage> {
    if cg.is_null() {
        return None;
    }
    unsafe {
        let width = CGImageGetWidth(cg);
        let height = CGImageGetHeight(cg);
        if width == 0 || height == 0 {
            return None;
        }
        let bytes_per_row = width.checked_mul(4)?;
        let mut bgra = vec![0u8; bytes_per_row.checked_mul(height)?];
        let space = CGColorSpaceCreateDeviceRGB();
        let context = CGBitmapContextCreate(
            bgra.as_mut_ptr().cast(),
            width,
            height,
            8,
            bytes_per_row,
            space,
            BGRA_PREMULTIPLIED,
        );
        CGColorSpaceRelease(space);
        if context.is_null() {
            return None;
        }
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width as f64, height as f64));
        CGContextDrawImage(context, rect, cg);
        CGContextRelease(context);
        unpremultiply_bgra(&mut bgra);
        Some(LibraryImage {
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
            bgra,
        })
    }
}

/// imageio source over the file at path, released on drop
struct ImageSource(*const c_void);

impl ImageSource {
    fn open(path: &Path) -> Option<Self> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        let source = unsafe { CGImageSourceCreateWithURL(Retained::as_ptr(&url).cast(), std::ptr::null()) };
        (!source.is_null()).then_some(Self(source))
    }

    fn from_data(bytes: &[u8]) -> Option<Self> {
        let data = NSData::with_bytes(bytes);
        let source = unsafe { CGImageSourceCreateWithData(Retained::as_ptr(&data).cast(), std::ptr::null()) };
        (!source.is_null()).then_some(Self(source))
    }
}

impl Drop for ImageSource {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

pub(crate) fn decode_image_file(path: &Path) -> Option<LibraryImage> {
    first_image(&ImageSource::open(path)?, None)
}

pub(crate) fn decode_image_data(bytes: &[u8], maximum_pixel_dimension: Option<u32>) -> Option<LibraryImage> {
    first_image(&ImageSource::from_data(bytes)?, maximum_pixel_dimension)
}

fn first_image(source: &ImageSource, maximum_pixel_dimension: Option<u32>) -> Option<LibraryImage> {
    let dictionary_class = AnyClass::get(c"NSDictionary")?;
    let yes = NSNumber::new_bool(true);
    let maximum = maximum_pixel_dimension.map(|value| NSNumber::new_u32(value.max(1)));
    unsafe {
        let mut keys: Vec<*const AnyObject> = vec![
            (kCGImageSourceCreateThumbnailFromImageAlways as *const NSString).cast(),
            (kCGImageSourceCreateThumbnailWithTransform as *const NSString).cast(),
        ];
        let mut values: Vec<*const AnyObject> = vec![Retained::as_ptr(&yes).cast(), Retained::as_ptr(&yes).cast()];
        if let Some(maximum) = maximum.as_ref() {
            keys.push((kCGImageSourceThumbnailMaxPixelSize as *const NSString).cast());
            values.push(Retained::as_ptr(maximum).cast());
        }
        let options: Option<Retained<AnyObject>> = msg_send![
            dictionary_class,
            dictionaryWithObjects: values.as_ptr(),
            forKeys: keys.as_ptr(),
            count: keys.len()
        ];
        let options = options?;
        let cg = CGImageSourceCreateThumbnailAtIndex(source.0, 0, Retained::as_ptr(&options));
        if cg.is_null() {
            return None;
        }
        let image = bgra_of_cg_image(cg);
        CGImageRelease(cg);
        image
    }
}

pub(crate) fn image_file_size(path: &Path) -> Option<(u32, u32)> {
    let source = ImageSource::open(path)?;
    unsafe {
        let properties = CGImageSourceCopyPropertiesAtIndex(source.0, 0, std::ptr::null());
        if properties.is_null() {
            return None;
        }
        let read = |key: &NSString| -> Option<i64> {
            let value: Option<Retained<NSNumber>> = msg_send![properties, objectForKey: key];
            value.map(|number| number.as_i64())
        };
        let width = read(kCGImagePropertyPixelWidth);
        let height = read(kCGImagePropertyPixelHeight);
        let orientation = read(kCGImagePropertyOrientation).unwrap_or(1);
        CFRelease(properties.cast());
        let width = u32::try_from(width?).ok().filter(|width| *width > 0)?;
        let height = u32::try_from(height?).ok().filter(|height| *height > 0)?;
        Some(if (5..=8).contains(&orientation) { (height, width) } else { (width, height) })
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RisePhotoLibraryObserver"]
    struct ChangeObserver;

    impl ChangeObserver {
        #[unsafe(method(photoLibraryDidChange:))]
        fn photo_library_did_change(&self, change: &AnyObject) {
            guarded("change", || apply_change(change));
            let sinks: Vec<ChangeSink> =
                SUBSCRIBERS.lock().sinks.iter().map(|(_, sink)| sink.clone()).collect();
            for sink in sinks {
                guarded("change", move || sink());
            }
        }
    }
);

impl ChangeObserver {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(target_arch = "aarch64")]
static SAVED_DESCRIPTOR: BlockDescriptor = descriptor(c"v20@?0B8@12");
#[cfg(not(target_arch = "aarch64"))]
static SAVED_DESCRIPTOR: BlockDescriptor = descriptor(c"v20@?0c8@12");

/// a save between performChanges and its completion handler
struct PendingSave {
    request: LibrarySaveRequest,
    refused: Option<PhotoLibraryUnavailable>,
    done: SaveSink,
}

static SAVES: Mutex<Option<HashMap<u64, PendingSave>>> = Mutex::new(None);
static NEXT_SAVE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn with_saves<T>(f: impl FnOnce(&mut HashMap<u64, PendingSave>) -> T) -> T {
    let mut guard = SAVES.lock();
    f(guard.get_or_insert_with(HashMap::new))
}

fn level_of(level: isize) -> Result<PhotoLibraryAccessLevel, PhotoLibraryUnavailable> {
    let library = class(c"PHPhotoLibrary")?;
    let status: isize = unsafe { msg_send![library, authorizationStatusForAccessLevel: level] };
    Ok(PhotoLibraryAccessLevel::from_ph_status(status as i64))
}

fn request_level(level: isize, done: AccessSink) -> Result<(), PhotoLibraryUnavailable> {
    let library = class(c"PHPhotoLibrary")?;
    let id = with_pending(|p| {
        let id = p.next;
        p.next += 1;
        p.access.insert(id, done);
        id
    });
    let block = id_block(id, access_answered as *const c_void, &ACCESS_DESCRIPTOR);
    unsafe {
        let _: () = msg_send![
            library,
            requestAuthorizationForAccessLevel: level,
            handler: &raw const block
        ];
    }
    Ok(())
}

fn start_save(request: LibrarySaveRequest, done: SaveSink) {
    let library = match shared_library() {
        Ok(library) => library,
        Err(reason) => {
            guarded("save", move || done(Err(reason)));
            return;
        }
    };
    let id = NEXT_SAVE.fetch_add(1, Ordering::Relaxed);
    with_saves(|saves| {
        saves.insert(
            id,
            PendingSave {
                request,
                refused: None,
                done,
            },
        )
    });
    let changes = id_block(id, save_changes as *const c_void, &ENCODE_DESCRIPTOR);
    let completion = id_block(id, save_answered as *const c_void, &SAVED_DESCRIPTOR);
    unsafe {
        let _: () = msg_send![
            &*library,
            performChanges: &raw const changes,
            completionHandler: &raw const completion
        ];
    }
}

unsafe extern "C" fn save_changes(block: *const IdBlock) {
    let id = unsafe { (*block).id };
    let Some(request) = with_saves(|saves| saves.get(&id).map(|save| save.request.clone())) else {
        return;
    };
    guarded("save changes", move || {
        if let Err(reason) = create_assets(&request) {
            with_saves(|saves| {
                if let Some(save) = saves.get_mut(&id) {
                    save.refused = Some(reason);
                }
            });
        }
    });
}

unsafe extern "C" fn save_answered(block: *const IdBlock, success: Bool, error: *mut AnyObject) {
    let id = unsafe { (*block).id };
    let Some(save) = with_saves(|saves| saves.remove(&id)) else {
        return;
    };
    let result = match save.refused {
        Some(reason) => Err(reason),
        None if success.as_bool() => Ok(()),
        None => {
            let error = unsafe { error.cast::<NSError>().as_ref() };
            Err(PhotoLibraryUnavailable::SystemRefused(error.map_or_else(
                || "performChanges failed".to_owned(),
                |error| error.localizedDescription().to_string(),
            )))
        }
    };
    let done = save.done;
    guarded("save", move || done(result));
}

fn file_url(path: &Path) -> Result<Retained<NSURL>, PhotoLibraryUnavailable> {
    let text = path
        .to_str()
        .ok_or_else(|| PhotoLibraryUnavailable::SystemRefused("save path is not UTF-8".into()))?;
    Ok(NSURL::fileURLWithPath(&NSString::from_str(text)))
}

fn made(
    request: Option<Retained<AnyObject>>,
    what: &str,
) -> Result<Retained<AnyObject>, PhotoLibraryUnavailable> {
    request.ok_or_else(|| PhotoLibraryUnavailable::SystemRefused(format!("no {what} request")))
}

/// files a saved asset under the moment of the save
fn filed_now(request: &AnyObject) -> Result<(), PhotoLibraryUnavailable> {
    let date_class = class(c"NSDate")?;
    unsafe {
        let now: Option<Retained<AnyObject>> = msg_send![date_class, date];
        let now = made(now, "date")?;
        let _: () = msg_send![request, setCreationDate: &*now];
    }
    Ok(())
}

fn create_assets(request: &LibrarySaveRequest) -> Result<(), PhotoLibraryUnavailable> {
    match request {
        LibrarySaveRequest::Image(path) => {
            let change_class = class(c"PHAssetChangeRequest")?;
            let url = file_url(path)?;
            let created: Option<Retained<AnyObject>> =
                unsafe { msg_send![change_class, creationRequestForAssetFromImageAtFileURL: &*url] };
            filed_now(&*made(created, "image")?)
        }
        LibrarySaveRequest::Video(path) => {
            let change_class = class(c"PHAssetChangeRequest")?;
            let url = file_url(path)?;
            let created: Option<Retained<AnyObject>> =
                unsafe { msg_send![change_class, creationRequestForAssetFromVideoAtFileURL: &*url] };
            filed_now(&*made(created, "video")?)
        }
        LibrarySaveRequest::Originals(files) => {
            let creation_class = class(c"PHAssetCreationRequest")?;
            for file in files {
                let url = file_url(&file.path)?;
                let options = new_object(c"PHAssetResourceCreationOptions")?;
                let name = file
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let created: Option<Retained<AnyObject>> =
                    unsafe { msg_send![creation_class, creationRequestForAsset] };
                let creation = made(created, "asset")?;
                filed_now(&creation)?;
                unsafe {
                    let _: () = msg_send![&*options, setOriginalFilename: &*NSString::from_str(&name)];
                    let _: () = msg_send![&*options, setShouldMoveFile: Bool::NO];
                    if let Some(content_type) = &file.content_type {
                        let identifier = NSString::from_str(content_type);
                        let _: () = msg_send![&*options, setUniformTypeIdentifier: &*identifier];
                    }
                    let _: () = msg_send![
                        &*creation,
                        addResourceWithType: file.kind.ph_resource_type(),
                        fileURL: &*url,
                        options: &*options
                    ];
                }
            }
            Ok(())
        }
    }
}
