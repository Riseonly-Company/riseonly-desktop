use std::any::Any;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_ulong, c_void, CStr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_video::pixel_buffer::{
    kCVPixelBufferLock_ReadOnly, kCVPixelFormatType_32BGRA, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange, CVPixelBuffer,
    CVPixelBufferKeys, CVPixelBufferRef,
};
use core_video::r#return::kCVReturnSuccess;
use gpui::{size, AnyElement, IntoElement as _, ObjectFit, Size, Styled as _};
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{define_class, msg_send, sel, AnyThread as _, DefinedClass as _};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSNumber, NSSize, NSString, NSURL};
use parking_lot::Mutex;

use super::av_player::*;
use super::media_engine_frames::Nv12Picture;
use super::pixel_buffer::{clockwise_quarter_turns_of_display_rotation, Nv12PixelBuffer};
use crate::texture::color::{ColorRange, ColorSpace, ConversionParams};
use crate::texture::frame::FrameFormat;
use crate::texture::native_surface::ycbcr_to_rgb_columns;
use crate::video::av_asset::{
    AVAssetResourceLoader, AVAssetResourceLoadingRequest, AVURLAsset, ContentInformation, DataRange,
    LoadingRequestBackend,
};

const DRAWABLE_PIXEL_FORMAT: u32 = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;

const ALPHA_PIXEL_FORMAT: u32 = kCVPixelFormatType_32BGRA;

fn is_drawable_format(format: u32) -> bool {
    format == DRAWABLE_PIXEL_FORMAT || format == ALPHA_PIXEL_FORMAT
}

#[repr(C)]
struct CvBuffer {
    _private: [u8; 0],
}

unsafe impl RefEncode for CvBuffer {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CVBuffer", &[]));
}

/// coremedia cmtime layout
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CmTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

unsafe impl Encode for CmTime {
    const ENCODING: Encoding =
        Encoding::Struct("?", &[Encoding::LongLong, Encoding::Int, Encoding::UInt, Encoding::LongLong]);
}

unsafe impl RefEncode for CmTime {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

/// coremedia cmtimerange layout, two cmtimes
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CmTimeRange {
    start: CmTime,
    duration: CmTime,
}

unsafe impl Encode for CmTimeRange {
    const ENCODING: Encoding = Encoding::Struct("?", &[CmTime::ENCODING, CmTime::ENCODING]);
}

const CM_TIME_VALID: u32 = 1;
const CM_TIME_POSITIVE_INFINITY: u32 = 4;
const MICROSECOND_TIMESCALE: i32 = 1_000_000;

/// coregraphics affine transform layout, a b c d tx ty
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CgAffineTransform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

unsafe impl Encode for CgAffineTransform {
    const ENCODING: Encoding = Encoding::Struct(
        "CGAffineTransform",
        &[Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double, Encoding::Double],
    );
}

impl CgAffineTransform {
    /// clockwise quarter turns the transform shows an encoded picture with
    fn clockwise_quarter_turns(&self) -> u8 {
        let clockwise_degrees = self.b.atan2(self.a).to_degrees();
        clockwise_quarter_turns_of_display_rotation(-clockwise_degrees)
    }
}

fn cm_time_microseconds(us: i64) -> CmTime {
    CmTime { value: us, timescale: MICROSECOND_TIMESCALE, flags: CM_TIME_VALID, epoch: 0 }
}

fn cm_time_positive_infinity() -> CmTime {
    CmTime { value: 0, timescale: 0, flags: CM_TIME_VALID | CM_TIME_POSITIVE_INFINITY, epoch: 0 }
}

fn seek_times(window: &SeekWindow) -> (CmTime, CmTime, CmTime) {
    let before = window.target_us.saturating_sub(window.min_us).max(0);
    let after = if window.max_us == i64::MAX {
        cm_time_positive_infinity()
    } else {
        cm_time_microseconds(window.max_us.saturating_sub(window.target_us).max(0))
    };
    (cm_time_microseconds(window.target_us), cm_time_microseconds(before), after)
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVPlayerItemDidPlayToEndTimeNotification: &'static NSString;
    static AVMediaTypeVideo: &'static NSString;
    static AVMediaCharacteristicContainsAlphaChannel: &'static NSString;
    static AVVideoColorPropertiesKey: &'static NSString;
    static AVVideoColorPrimariesKey: &'static NSString;
    static AVVideoColorPrimaries_ITU_R_709_2: &'static NSString;
    static AVVideoTransferFunctionKey: &'static NSString;
    static AVVideoTransferFunction_ITU_R_709_2: &'static NSString;
    static AVVideoYCbCrMatrixKey: &'static NSString;
    static AVVideoYCbCrMatrix_ITU_R_601_4: &'static NSString;
}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    fn CACurrentMediaTime() -> f64;
}

unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut AnyObject;
    static _NSConcreteStackBlock: [u8; 0];
}

const LOADER_ERROR_DOMAIN: &str = "net.riseonly.Riseonly.AVAssetResourceLoader";

pub(super) fn player_available() -> bool {
    AnyClass::get(c"AVQueuePlayer").is_some()
        && AnyClass::get(c"AVPlayer").is_some()
        && AnyClass::get(c"AVPlayerItemVideoOutput").is_some()
}

fn class(name: &'static CStr) -> Result<&'static AnyClass, AVPlayerError> {
    AnyClass::get(name).ok_or(AVPlayerError::Platform { call: "AVFoundation class lookup" })
}

fn ns_url(url: &rise_core::foundation::foundation_url::Url) -> Result<Retained<NSURL>, AVPlayerError> {
    match SystemUrl::of(url)? {
        SystemUrl::File(path) => Ok(NSURL::fileURLWithPath(&NSString::from_str(&path))),
        SystemUrl::Remote(text) => NSURL::URLWithString(&NSString::from_str(&text))
            .ok_or_else(|| AVPlayerError::BadUrl(text)),
    }
}

/// system item opened for an item, with the resource loader delegate of its asset when it has one
/// the loader holds its delegate weakly so whoever plays the item keeps this alive
struct SystemItem {
    item: Retained<AnyObject>,
    loader_delegate: Option<Retained<LoaderDelegate>>,
}

/// system item for an item made from a url or an asset, item settings applied
fn player_item(item: &AVPlayerItem) -> Result<SystemItem, AVPlayerError> {
    let item_class = class(c"AVPlayerItem")?;
    let (made, loader_delegate) = match item.asset() {
        Some(asset) => {
            let (system_asset, delegate) = url_asset(asset)?;
            let made: Option<Retained<AnyObject>> =
                unsafe { msg_send![item_class, playerItemWithAsset: &*system_asset] };
            (made.ok_or(AVPlayerError::Platform { call: "AVPlayerItem playerItemWithAsset:" })?, Some(delegate))
        }
        None => {
            let url = ns_url(item.url())?;
            let made: Option<Retained<AnyObject>> = unsafe { msg_send![item_class, playerItemWithURL: &*url] };
            (made.ok_or(AVPlayerError::Platform { call: "AVPlayerItem playerItemWithURL:" })?, None)
        }
    };
    apply_item_settings(&made, item);
    Ok(SystemItem { item: made, loader_delegate })
}

/// url asset whose resource loader asks a delegate that forwards to the asset loader
/// on a serial queue of its own
fn url_asset(asset: &AVURLAsset) -> Result<(Retained<AnyObject>, Retained<LoaderDelegate>), AVPlayerError> {
    let url = ns_url(asset.url())?;
    let asset_class = class(c"AVURLAsset")?;
    let system_asset: Option<Retained<AnyObject>> =
        unsafe { msg_send![asset_class, URLAssetWithURL: &*url, options: std::ptr::null::<AnyObject>()] };
    let system_asset = system_asset.ok_or(AVPlayerError::Platform { call: "AVURLAsset URLAssetWithURL:options:" })?;
    let loader: Option<Retained<AnyObject>> = unsafe { msg_send![&*system_asset, resourceLoader] };
    let loader = loader.ok_or(AVPlayerError::Platform { call: "AVURLAsset resourceLoader" })?;
    let queue = loader_queue()?;
    let delegate = LoaderDelegate::new(asset.resource_loader());
    unsafe {
        let _: () = msg_send![&*loader, setDelegate: &*delegate, queue: &*queue];
    }
    Ok((system_asset, delegate))
}

fn loader_queue() -> Result<Retained<AnyObject>, AVPlayerError> {
    unsafe {
        let queue = dispatch_queue_create(c"net.riseonly.media.resource-loader".as_ptr(), std::ptr::null_mut());
        Retained::from_raw(queue).ok_or(AVPlayerError::Platform { call: "dispatch_queue_create" })
    }
}

/// system requests a delegate forwarded and has not seen finished or cancelled
/// keyed by the address of the system request, the backend keeps that object alive
type ForwardedRequests = Mutex<HashMap<usize, AVAssetResourceLoadingRequest>>;

/// what the delegate object owns, the asset loader and the requests in flight
struct LoaderBridge {
    loader: AVAssetResourceLoader,
    requests: Arc<ForwardedRequests>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseAVAssetResourceLoaderDelegate"]
    #[ivars = LoaderBridge]
    struct LoaderDelegate;

    impl LoaderDelegate {
        #[unsafe(method(resourceLoader:shouldWaitForLoadingOfRequestedResource:))]
        fn should_wait_for_loading(&self, _resource_loader: &AnyObject, request: &AnyObject) -> Bool {
            let accepted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.forward(request)));
            Bool::new(accepted.unwrap_or_else(|_| {
                tracing::error!(target: "riseonly", "resource loader delegate panicked");
                false
            }))
        }

        #[unsafe(method(resourceLoader:didCancelLoadingRequest:))]
        fn did_cancel_loading(&self, _resource_loader: &AnyObject, request: &AnyObject) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.cancel(request))).is_err() {
                tracing::error!(target: "riseonly", "resource loader cancel panicked");
            }
        }
    }
);

impl LoaderDelegate {
    fn new(loader: AVAssetResourceLoader) -> Retained<Self> {
        let this = Self::alloc().set_ivars(LoaderBridge { loader, requests: Arc::new(Mutex::new(HashMap::new())) });
        unsafe { msg_send![super(this), init] }
    }

    /// returns true because the app delegate answer comes later on the main thread
    /// a request it declines is finished with an error
    fn forward(&self, request: &AnyObject) -> bool {
        let Some(system) = (unsafe { Retained::retain(std::ptr::from_ref(request).cast_mut()) }) else {
            return false;
        };
        let (asks_content_information, data_range) = unsafe {
            let information: Option<Retained<AnyObject>> = msg_send![&*system, contentInformationRequest];
            let data: Option<Retained<AnyObject>> = msg_send![&*system, dataRequest];
            let data_range = match data {
                Some(data) => {
                    let requested_offset: i64 = msg_send![&*data, requestedOffset];
                    let requested_length: isize = msg_send![&*data, requestedLength];
                    let to_end: Bool = msg_send![&*data, requestsAllDataToEndOfResource];
                    Some(DataRange {
                        requested_offset,
                        requested_length: i64::try_from(requested_length).unwrap_or(i64::MAX),
                        requests_all_data_to_end_of_resource: to_end.as_bool(),
                    })
                }
                None => None,
            };
            (information.is_some(), data_range)
        };
        let key = Retained::as_ptr(&system) as usize;
        let backend = SystemLoadingRequest {
            request: SystemRequest(system),
            key,
            requests: Arc::downgrade(&self.ivars().requests),
        };
        let forwarded = AVAssetResourceLoadingRequest::new(asks_content_information, data_range, Box::new(backend));
        self.ivars().requests.lock().insert(key, forwarded.clone());
        self.ivars().loader.submit(forwarded);
        true
    }

    /// did cancel, the hub marks the request cancelled and tells the app delegate on the main thread
    fn cancel(&self, request: &AnyObject) {
        let key = std::ptr::from_ref(request) as usize;
        let forwarded = self.ivars().requests.lock().remove(&key);
        if let Some(forwarded) = forwarded {
            self.ivars().loader.cancel(&forwarded);
        }
    }
}

/// system resource loading request, moved to the main thread where the hub answers it
struct SystemRequest(Retained<AnyObject>);

unsafe impl Send for SystemRequest {}
unsafe impl Sync for SystemRequest {}

struct SystemLoadingRequest {
    request: SystemRequest,
    key: usize,
    requests: Weak<ForwardedRequests>,
}

impl LoadingRequestBackend for SystemLoadingRequest {
    fn set_content_information(&self, information: &ContentInformation) {
        let content_type = information.content_type.as_deref().map(NSString::from_str);
        unsafe {
            let part: Option<Retained<AnyObject>> = msg_send![&*self.request.0, contentInformationRequest];
            let Some(part) = part else {
                return;
            };
            let _: () = msg_send![&*part, setByteRangeAccessSupported: Bool::new(information.is_byte_range_access_supported)];
            let _: () = msg_send![&*part, setContentLength: information.content_length];
            let _: () = msg_send![&*part, setContentType: content_type.as_deref()];
        }
    }

    fn respond(&self, data: &[u8]) {
        let bytes = NSData::with_bytes(data);
        unsafe {
            let part: Option<Retained<AnyObject>> = msg_send![&*self.request.0, dataRequest];
            if let Some(part) = part {
                let _: () = msg_send![&*part, respondWithData: &*bytes];
            }
        }
    }

    fn finish(&self, outcome: Result<(), anyhow::Error>) {
        if let Some(requests) = self.requests.upgrade() {
            requests.lock().remove(&self.key);
        }
        match outcome {
            Ok(()) => unsafe {
                let _: () = msg_send![&*self.request.0, finishLoading];
            },
            Err(error) => {
                let description = NSString::from_str(&format!("{error:#}"));
                let domain = NSString::from_str(LOADER_ERROR_DOMAIN);
                unsafe {
                    let user_info = NSDictionary::<NSString, AnyObject>::from_retained_objects(
                        &[objc2_foundation::NSLocalizedDescriptionKey],
                        &[description.into()],
                    );
                    let ns_error = NSError::errorWithDomain_code_userInfo(&domain, -1, Some(&user_info));
                    let _: () = msg_send![&*self.request.0, finishLoadingWithError: &*ns_error];
                }
            }
        }
    }
}

/// settings applied to the system item opened for item
fn apply_item_settings(system_item: &AnyObject, item: &AVPlayerItem) {
    let seconds: f64 = item.preferred_forward_buffer_duration().as_secs_f64();
    unsafe {
        let _: () = msg_send![system_item, setPreferredForwardBufferDuration: seconds];
    }
}

fn output_settings(alpha: bool) -> Retained<NSDictionary<NSString, AnyObject>> {
    let format_key = CFString::from(CVPixelBufferKeys::PixelFormatType);
    let surface_key = CFString::from(CVPixelBufferKeys::IOSurfaceProperties);
    let metal_key = CFString::from(CVPixelBufferKeys::MetalCompatibility);
    let (format_key, surface_key, metal_key): (&NSString, &NSString, &NSString) = unsafe {
        (
            &*(format_key.as_concrete_TypeRef() as *const NSString),
            &*(surface_key.as_concrete_TypeRef() as *const NSString),
            &*(metal_key.as_concrete_TypeRef() as *const NSString),
        )
    };
    let (color_key, primaries_key, primaries, transfer_key, transfer, matrix_key, matrix) = unsafe {
        (
            AVVideoColorPropertiesKey,
            AVVideoColorPrimariesKey,
            AVVideoColorPrimaries_ITU_R_709_2,
            AVVideoTransferFunctionKey,
            AVVideoTransferFunction_ITU_R_709_2,
            AVVideoYCbCrMatrixKey,
            AVVideoYCbCrMatrix_ITU_R_601_4,
        )
    };
    let color: Retained<NSDictionary<NSString, NSString>> =
        NSDictionary::from_slices(&[primaries_key, transfer_key, matrix_key], &[primaries, transfer, matrix]);
    let io_surface = NSDictionary::<NSString, AnyObject>::from_slices::<NSString>(&[], &[]);
    if alpha {
        return NSDictionary::from_retained_objects(
            &[format_key, surface_key, metal_key],
            &[
                NSNumber::numberWithUnsignedInt(ALPHA_PIXEL_FORMAT).into(),
                io_surface.into(),
                NSNumber::numberWithBool(true).into(),
            ],
        );
    }
    NSDictionary::from_retained_objects(
        &[format_key, surface_key, metal_key, color_key],
        &[
            NSNumber::numberWithUnsignedInt(DRAWABLE_PIXEL_FORMAT).into(),
            io_surface.into(),
            NSNumber::numberWithBool(true).into(),
            color.into(),
        ],
    )
}

fn attach_output(item: &AnyObject, alpha: bool) -> Result<Retained<AnyObject>, AVPlayerError> {
    let output_class = class(c"AVPlayerItemVideoOutput")?;
    let settings = output_settings(alpha);
    unsafe {
        let allocated: Allocated<AnyObject> = msg_send![output_class, alloc];
        let output: Option<Retained<AnyObject>> = msg_send![allocated, initWithOutputSettings: &*settings];
        let output = output.ok_or(AVPlayerError::Platform { call: "AVPlayerItemVideoOutput initWithOutputSettings:" })?;
        let _: () = msg_send![item, addOutput: &*output];
        Ok(output)
    }
}

/// item that output is attached to and the output itself
/// replaced when the queue moves on, the looper plays a fresh copy of the template every pass
struct ItemOutput {
    item: Retained<AnyObject>,
    output: Retained<AnyObject>,
    alpha: Option<bool>,
}

impl Drop for ItemOutput {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.item, removeOutput: &*self.output];
        }
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseAVPlayerEndObserver"]
    #[ivars = EndSink]
    struct EndObserver;

    impl EndObserver {
        #[unsafe(method(itemDidPlayToEndTime:))]
        fn item_did_play_to_end_time(&self, notification: &AnyObject) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Some(url) = ended_asset_url(notification) {
                    self.ivars().deliver(&url);
                }
            }))
            .is_err()
            {
                tracing::error!(target: "riseonly", "clip end observer panicked");
            }
        }
    }
);

impl EndObserver {
    fn new(sink: EndSink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(sink);
        unsafe { msg_send![super(this), init] }
    }
}

fn ended_asset_url(notification: &AnyObject) -> Option<String> {
    let item: Option<Retained<AnyObject>> = unsafe { msg_send![notification, object] };
    let item = item?;
    item_asset_url(&item)
}

fn notification_center() -> Result<Retained<AnyObject>, AVPlayerError> {
    let center_class = class(c"NSNotificationCenter")?;
    let center: Option<Retained<AnyObject>> = unsafe { msg_send![center_class, defaultCenter] };
    center.ok_or(AVPlayerError::Platform { call: "NSNotificationCenter defaultCenter" })
}

struct SystemLooper {
    looper: Retained<AnyObject>,
    _loader_delegate: Option<Retained<LoaderDelegate>>,
}

impl Drop for SystemLooper {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.looper, disableLooping];
        }
    }
}

struct MacPlayer {
    player: Retained<AnyObject>,
    observer: Retained<EndObserver>,
    tap: PictureTap,
    _loader_delegate: Option<Retained<LoaderDelegate>>,
}

pub(super) fn open_engine(item: &AVPlayerItem, ends: EndSink) -> Result<Box<dyn PlayerEngine>, AVPlayerError> {
    let SystemItem { item: system_item, loader_delegate } = player_item(item)?;
    let player_class = class(c"AVQueuePlayer")?;
    let player: Option<Retained<AnyObject>> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![player_class, alloc];
        msg_send![allocated, initWithPlayerItem: &*system_item]
    };
    let player = player.ok_or(AVPlayerError::Platform { call: "AVQueuePlayer initWithPlayerItem:" })?;

    let observer = EndObserver::new(ends);
    let center = notification_center()?;
    unsafe {
        let _: () = msg_send![
            &*center,
            addObserver: &*observer,
            selector: sel!(itemDidPlayToEndTime:),
            name: AVPlayerItemDidPlayToEndTimeNotification,
            object: std::ptr::null::<AnyObject>()
        ];
    }

    Ok(Box::new(MacPlayer { player, observer, tap: PictureTap::default(), _loader_delegate: loader_delegate }))
}

impl Drop for MacPlayer {
    fn drop(&mut self) {
        if let Ok(center) = notification_center() {
            unsafe {
                let _: () = msg_send![&*center, removeObserver: &*self.observer];
            }
        }
        self.tap = PictureTap::default();
        unsafe {
            let _: () = msg_send![&*self.player, pause];
        }
    }
}

/// video output on the item a player plays now and the last picture it handed out
struct PictureTap {
    output: Option<ItemOutput>,
    last: Option<CVPixelBuffer>,
    last_uncopied: bool,
    last_unswapped: bool,
    swap_key: u64,
}

static NEXT_SWAP_KEY: AtomicU64 = AtomicU64::new(1);

impl Default for PictureTap {
    fn default() -> Self {
        Self {
            output: None,
            last: None,
            last_uncopied: false,
            last_unswapped: false,
            swap_key: NEXT_SWAP_KEY.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl PictureTap {
    /// newest picture of the player current item drawn with fit, or the last one
    fn frame(&mut self, player: &AnyObject, fit: ObjectFit) -> Option<AnyElement> {
        self.take_new_picture(player);
        self.last_unswapped = false;
        let swap_key = self.swap_key;
        self.last
            .clone()
            .map(|buffer| gpui::surface(buffer).object_fit(fit).swap_key(swap_key).size_full().into_any_element())
    }

    fn take_swapped_picture(&mut self, player: &AnyObject) -> Option<gpui::SurfaceSource> {
        self.take_new_picture(player);
        if !self.last_unswapped {
            return None;
        }
        self.last_unswapped = false;
        self.last.clone().map(gpui::SurfaceSource::from)
    }

    /// moves the output newest picture into last if there is one
    fn take_new_picture(&mut self, player: &AnyObject) {
        self.follow_current_item(player);
        if let Some(buffer) = self.copy_new_picture() {
            if is_drawable_format(buffer.get_pixel_format()) {
                self.last = Some(buffer);
                self.last_uncopied = true;
                self.last_unswapped = true;
            } else {
                tracing::warn!(target: "riseonly", "clip output is neither full-range NV12 nor 32BGRA; picture skipped");
            }
        }
    }

    fn copy_uncopied_picture(&mut self, player: &AnyObject) -> Option<CVPixelBuffer> {
        self.take_new_picture(player);
        if !self.last_uncopied {
            return None;
        }
        self.last_uncopied = false;
        self.last.clone()
    }

    fn follow_current_item(&mut self, player: &AnyObject) {
        let current: Option<Retained<AnyObject>> = unsafe { msg_send![player, currentItem] };
        let Some(current) = current else {
            self.output = None;
            return;
        };
        if let Some(attached) = self.output.as_mut() {
            if Retained::as_ptr(&attached.item) == Retained::as_ptr(&current) {
                if attached.alpha.is_some() {
                    return;
                }
                match item_has_alpha(&current) {
                    None => return,
                    Some(false) => {
                        attached.alpha = Some(false);
                        compose_upright(&current);
                        return;
                    }
                    Some(true) => self.last = None,
                }
            }
        }
        self.output = None;
        let alpha = item_has_alpha(&current);
        if alpha == Some(false) {
            compose_upright(&current);
        }
        match attach_output(&current, alpha == Some(true)) {
            Ok(output) => self.output = Some(ItemOutput { item: current, output, alpha }),
            Err(error) => tracing::warn!(target: "riseonly", %error, "clip video output unavailable"),
        }
    }

    /// true when a picture was taken
    /// or the output holds one for the current host time that the next frame draws
    fn has_picture(&self) -> bool {
        if self.last.is_some() {
            return true;
        }
        let Some(attached) = self.output.as_ref() else {
            return false;
        };
        unsafe {
            let item_time: CmTime = msg_send![&*attached.output, itemTimeForHostTime: CACurrentMediaTime()];
            let has_new: Bool = msg_send![&*attached.output, hasNewPixelBufferForItemTime: item_time];
            has_new.as_bool()
        }
    }

    fn copy_new_picture(&self) -> Option<CVPixelBuffer> {
        let attached = self.output.as_ref()?;
        unsafe {
            let host_time = CACurrentMediaTime();
            let item_time: CmTime = msg_send![&*attached.output, itemTimeForHostTime: host_time];
            let has_new: Bool = msg_send![&*attached.output, hasNewPixelBufferForItemTime: item_time];
            if !has_new.as_bool() {
                return None;
            }
            let buffer: *mut CvBuffer = msg_send![
                &*attached.output,
                copyPixelBufferForItemTime: item_time,
                itemTimeForDisplay: std::ptr::null_mut::<CmTime>()
            ];
            if buffer.is_null() {
                return None;
            }
            Some(CVPixelBuffer::wrap_under_create_rule(buffer as CVPixelBufferRef))
        }
    }
}

impl PlayerEngine for MacPlayer {
    fn set_is_muted(&mut self, muted: bool) {
        unsafe {
            let _: () = msg_send![&*self.player, setMuted: Bool::new(muted)];
        }
    }

    fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd) {
        unsafe {
            let _: () = msg_send![&*self.player, setActionAtItemEnd: action.code()];
        }
    }

    fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool) {
        unsafe {
            let _: () = msg_send![&*self.player, setAutomaticallyWaitsToMinimizeStalling: Bool::new(waits)];
        }
    }

    fn play(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.player, play];
        }
    }

    fn pause(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.player, pause];
        }
    }

    fn start_looping(&mut self, template: &AVPlayerItem) -> Result<Box<dyn Any>, AVPlayerError> {
        let current: Option<Retained<AnyObject>> = unsafe { msg_send![&*self.player, currentItem] };
        let (template_item, loader_delegate) = match current {
            Some(current) if item_asset_url(&current).is_some_and(|url| is_end_of(template.url(), &url)) => {
                apply_item_settings(&current, template);
                (current, None)
            }
            _ => {
                let SystemItem { item, loader_delegate } = player_item(template)?;
                (item, loader_delegate)
            }
        };
        let looper_class = class(c"AVPlayerLooper")?;
        let looper: Option<Retained<AnyObject>> = unsafe {
            let allocated: Allocated<AnyObject> = msg_send![looper_class, alloc];
            msg_send![allocated, initWithPlayer: &*self.player, templateItem: &*template_item]
        };
        let looper = looper.ok_or(AVPlayerError::Platform { call: "AVPlayerLooper initWithPlayer:templateItem:" })?;
        Ok(Box::new(SystemLooper { looper, _loader_delegate: loader_delegate }))
    }

    fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement> {
        self.tap.frame(&self.player, fit)
    }

    fn swap_key(&self) -> Option<u64> {
        Some(self.tap.swap_key)
    }

    fn take_swapped_picture(&mut self) -> Option<gpui::SurfaceSource> {
        self.tap.take_swapped_picture(&self.player)
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseAVPlayerItemEndObserver"]
    #[ivars = ItemEndSink]
    struct ItemEndObserver;

    impl ItemEndObserver {
        #[unsafe(method(itemDidPlayToEndTime:))]
        fn item_did_play_to_end_time(&self, _notification: &AnyObject) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.ivars().deliver())).is_err() {
                tracing::error!(target: "riseonly", "media item end observer panicked");
            }
        }
    }
);

impl ItemEndObserver {
    fn new(sink: ItemEndSink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(sink);
        unsafe { msg_send![super(this), init] }
    }
}

/// system item a player plays, observed for its own end
struct PlaybackItem {
    item: Retained<AnyObject>,
    observer: Retained<ItemEndObserver>,
    _loader_delegate: Option<Retained<LoaderDelegate>>,
}

impl PlaybackItem {
    fn open(item: &AVPlayerItem, ends: ItemEndSink) -> Result<Self, AVPlayerError> {
        let SystemItem { item: system_item, loader_delegate } = player_item(item)?;
        let observer = ItemEndObserver::new(ends);
        let center = notification_center()?;
        unsafe {
            let _: () = msg_send![
                &*center,
                addObserver: &*observer,
                selector: sel!(itemDidPlayToEndTime:),
                name: AVPlayerItemDidPlayToEndTimeNotification,
                object: &*system_item
            ];
        }
        Ok(Self { item: system_item, observer, _loader_delegate: loader_delegate })
    }
}

impl Drop for PlaybackItem {
    fn drop(&mut self) {
        if let Ok(center) = notification_center() {
            unsafe {
                let _: () = msg_send![
                    &*center,
                    removeObserver: &*self.observer,
                    name: AVPlayerItemDidPlayToEndTimeNotification,
                    object: &*self.item
                ];
            }
        }
    }
}

struct MacPlayback {
    player: Retained<AnyObject>,
    current: Option<PlaybackItem>,
    tap: PictureTap,
}

pub(super) fn open_playback_engine() -> Result<Box<dyn PlaybackEngine>, AVPlayerError> {
    let player_class = class(c"AVPlayer")?;
    let player: Option<Retained<AnyObject>> = unsafe {
        let allocated: Allocated<AnyObject> = msg_send![player_class, alloc];
        msg_send![allocated, init]
    };
    let player = player.ok_or(AVPlayerError::Platform { call: "AVPlayer init" })?;
    Ok(Box::new(MacPlayback { player, current: None, tap: PictureTap::default() }))
}

impl MacPlayback {
    fn set_system_item(&mut self, item: Option<&AnyObject>) {
        unsafe {
            let _: () = msg_send![&*self.player, replaceCurrentItemWithPlayerItem: item];
        }
    }
}

impl Drop for MacPlayback {
    fn drop(&mut self) {
        self.tap = PictureTap::default();
        unsafe {
            let _: () = msg_send![&*self.player, pause];
        }
        self.set_system_item(None);
        self.current = None;
    }
}

impl PlaybackEngine for MacPlayback {
    fn replace_current_item(&mut self, item: Option<&AVPlayerItem>, ends: ItemEndSink) -> Result<(), AVPlayerError> {
        self.tap = PictureTap::default();
        let opened = item.map(|item| PlaybackItem::open(item, ends)).transpose();
        let next = match opened {
            Ok(next) => next,
            Err(error) => {
                self.set_system_item(None);
                self.current = None;
                return Err(error);
            }
        };
        self.set_system_item(next.as_ref().map(|next| &*next.item));
        self.current = next;
        Ok(())
    }

    fn play_immediately(&mut self, rate: f32) {
        unsafe {
            let _: () = msg_send![&*self.player, playImmediatelyAtRate: rate];
        }
    }

    fn set_rate(&mut self, rate: f32) {
        unsafe {
            let _: () = msg_send![&*self.player, setRate: rate];
        }
    }

    fn pause(&mut self) {
        unsafe {
            let _: () = msg_send![&*self.player, pause];
        }
    }

    fn current_time(&self) -> Option<f64> {
        self.current.as_ref()?;
        let time: CmTime = unsafe { msg_send![&*self.player, currentTime] };
        cm_time_seconds(time.value, time.timescale, time.flags)
    }

    fn current_item_duration(&self) -> Option<f64> {
        let current = self.current.as_ref()?;
        let duration: CmTime = unsafe { msg_send![&*current.item, duration] };
        cm_time_seconds(duration.value, duration.timescale, duration.flags)
    }

    fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement> {
        self.tap.frame(&self.player, fit)
    }

    fn swap_key(&self) -> Option<u64> {
        Some(self.tap.swap_key)
    }

    fn take_swapped_picture(&mut self) -> Option<gpui::SurfaceSource> {
        self.tap.take_swapped_picture(&self.player)
    }

    fn is_ready_for_display(&self) -> Option<bool> {
        Some(self.current.is_some() && self.tap.has_picture())
    }

    fn copy_pixel_buffer(&mut self) -> Option<AVPlayerPixelBuffer> {
        let current = self.current.as_ref()?;
        let buffer = self.tap.copy_uncopied_picture(&self.player)?;
        let clockwise_quarter_turns = output_quarter_turns(&current.item);
        Some(AVPlayerPixelBuffer::new(Box::new(MacPixelBuffer { buffer, clockwise_quarter_turns })))
    }

    fn seek(&mut self, window: SeekWindow, done: SeekSink) {
        if self.current.is_none() {
            done.deliver(false);
            return;
        }
        let (to, before, after) = seek_times(&window);
        let id = NEXT_SEEK.fetch_add(1, Ordering::Relaxed);
        PENDING_SEEKS.lock().get_or_insert_with(HashMap::new).insert(id, done);
        let block = SeekBlock {
            isa: (&raw const _NSConcreteStackBlock).cast(),
            flags: BLOCK_HAS_SIGNATURE,
            reserved: 0,
            invoke: seek_answered as *const c_void,
            descriptor: &SEEK_DESCRIPTOR,
            id,
        };
        unsafe {
            let _: () = msg_send![
                &*self.player,
                seekToTime: to,
                toleranceBefore: before,
                toleranceAfter: after,
                completionHandler: &raw const block
            ];
        }
    }

    fn set_is_muted(&mut self, muted: bool) -> Result<(), AVPlayerError> {
        unsafe {
            let _: () = msg_send![&*self.player, setMuted: Bool::new(muted)];
        }
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), AVPlayerError> {
        unsafe {
            let _: () = msg_send![&*self.player, setVolume: volume];
        }
        Ok(())
    }

    fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd) -> Result<(), AVPlayerError> {
        unsafe {
            let _: () = msg_send![&*self.player, setActionAtItemEnd: action.code()];
        }
        Ok(())
    }

    fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool) -> Result<(), AVPlayerError> {
        unsafe {
            let _: () = msg_send![&*self.player, setAutomaticallyWaitsToMinimizeStalling: Bool::new(waits)];
        }
        Ok(())
    }

    fn time_control_status(&self) -> Option<AVPlayerTimeControlStatus> {
        let status: isize = unsafe { msg_send![&*self.player, timeControlStatus] };
        match status {
            AV_TIME_CONTROL_PAUSED => Some(AVPlayerTimeControlStatus::Paused),
            AV_TIME_CONTROL_WAITING => Some(AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate),
            AV_TIME_CONTROL_PLAYING => Some(AVPlayerTimeControlStatus::Playing),
            _ => None,
        }
    }

    fn item_status(&self) -> Option<AVPlayerItemStatus> {
        let current = self.current.as_ref()?;
        let status: isize = unsafe { msg_send![&*current.item, status] };
        match status {
            AV_ITEM_STATUS_UNKNOWN => Some(AVPlayerItemStatus::Unknown),
            AV_ITEM_STATUS_READY => Some(AVPlayerItemStatus::ReadyToPlay),
            AV_ITEM_STATUS_FAILED => Some(AVPlayerItemStatus::Failed),
            _ => None,
        }
    }

    fn loaded_time_ranges(&self) -> Vec<CMTimeRange> {
        let Some(current) = self.current.as_ref() else {
            return Vec::new();
        };
        let ranges: Retained<NSArray<AnyObject>> = unsafe { msg_send![&*current.item, loadedTimeRanges] };
        ranges
            .to_vec()
            .into_iter()
            .filter_map(|value| {
                let range: CmTimeRange = unsafe { msg_send![&*value, CMTimeRangeValue] };
                let start = cm_time_seconds(range.start.value, range.start.timescale, range.start.flags)?;
                let duration = cm_time_seconds(range.duration.value, range.duration.timescale, range.duration.flags)?;
                Some(CMTimeRange::new(start, duration))
            })
            .collect()
    }

    fn presentation_size(&self) -> Option<Size<f32>> {
        let current = self.current.as_ref()?;
        let presented: NSSize = unsafe { msg_send![&*current.item, presentationSize] };
        if !(presented.width > 0.0 && presented.height > 0.0) {
            return None;
        }
        Some(size(presented.width as f32, presented.height as f32))
    }
}

/// quarter turns left in the output pictures, none once a composition turns them upright
fn output_quarter_turns(item: &AnyObject) -> u8 {
    let composed: Option<Retained<AnyObject>> = unsafe { msg_send![item, videoComposition] };
    if composed.is_some() {
        return 0;
    }
    video_track_transform(item).map_or(0, |transform| transform.clockwise_quarter_turns())
}

/// turns a rotated track upright with a composition so the surface draws its pictures as they are
fn compose_upright(item: &AnyObject) {
    if output_quarter_turns(item) == 0 {
        return;
    }
    let Ok(composition_class) = class(c"AVVideoComposition") else {
        return;
    };
    unsafe {
        let asset: Option<Retained<AnyObject>> = msg_send![item, asset];
        let Some(asset) = asset else {
            return;
        };
        let composition: Option<Retained<AnyObject>> =
            msg_send![composition_class, videoCompositionWithPropertiesOfAsset: &*asset];
        if let Some(composition) = composition {
            let _: () = msg_send![item, setVideoComposition: &*composition];
        }
    }
}

fn video_track_transform(item: &AnyObject) -> Option<CgAffineTransform> {
    let tracks: Retained<NSArray<AnyObject>> = unsafe { msg_send![item, tracks] };
    tracks.to_vec().into_iter().find_map(|track| {
        unsafe {
            let asset_track: Option<Retained<AnyObject>> = msg_send![&*track, assetTrack];
            let asset_track = asset_track?;
            let media_type: Retained<NSString> = msg_send![&*asset_track, mediaType];
            if !media_type.isEqualToString(AVMediaTypeVideo) {
                return None;
            }
            let transform: CgAffineTransform = msg_send![&*asset_track, preferredTransform];
            Some(transform)
        }
    })
}

/// whether the item video track has an alpha channel (hevc with alpha)
/// none until the item knows its tracks
fn item_has_alpha(item: &AnyObject) -> Option<bool> {
    let tracks: Retained<NSArray<AnyObject>> = unsafe { msg_send![item, tracks] };
    let mut video_tracks = 0_usize;
    for track in tracks.to_vec() {
        unsafe {
            let asset_track: Option<Retained<AnyObject>> = msg_send![&*track, assetTrack];
            let Some(asset_track) = asset_track else { continue };
            let media_type: Retained<NSString> = msg_send![&*asset_track, mediaType];
            if !media_type.isEqualToString(AVMediaTypeVideo) {
                continue;
            }
            video_tracks += 1;
            let alpha: Bool = msg_send![&*asset_track, hasMediaCharacteristic: AVMediaCharacteristicContainsAlphaChannel];
            if alpha.as_bool() {
                return Some(true);
            }
        }
    }
    (video_tracks > 0).then_some(false)
}

fn turned_bgra(width: usize, height: usize, bgra: &[u8], clockwise_quarter_turns: u8) -> Option<(u32, u32, Vec<u8>)> {
    if width == 0 || height == 0 || bgra.len() < width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }
    let turns = clockwise_quarter_turns % 4;
    let (out_width, out_height) = if turns % 2 == 1 { (height, width) } else { (width, height) };
    let mut turned = vec![0_u8; out_width * out_height * 4];
    for row in 0..height {
        for column in 0..width {
            let (to_column, to_row) = match turns {
                1 => (height - 1 - row, column),
                2 => (width - 1 - column, height - 1 - row),
                3 => (row, width - 1 - column),
                _ => (column, row),
            };
            let from = (row * width + column) * 4;
            let to = (to_row * out_width + to_column) * 4;
            turned[to..to + 4].copy_from_slice(&bgra[from..from + 4]);
        }
    }
    Some((u32::try_from(out_width).ok()?, u32::try_from(out_height).ok()?, turned))
}

struct MacPixelBuffer {
    buffer: CVPixelBuffer,
    clockwise_quarter_turns: u8,
}

unsafe impl Send for MacPixelBuffer {}

impl MacPixelBuffer {
    fn nv12_picture(&self) -> Option<Nv12Picture> {
        let buffer = &self.buffer;
        if buffer.get_pixel_format() != DRAWABLE_PIXEL_FORMAT || buffer.get_plane_count() != 2 {
            return None;
        }
        if buffer.lock_base_address(kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess {
            return None;
        }
        let planes = (Self::packed_plane(buffer, 0, 1), Self::packed_plane(buffer, 1, 2));
        buffer.unlock_base_address(kCVPixelBufferLock_ReadOnly);
        let (Some((luma, width, height)), Some((chroma, chroma_width, _))) = planes else {
            return None;
        };
        Some(Nv12Picture {
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
            luma: luma.into(),
            luma_stride: u32::try_from(width).ok()?,
            chroma: chroma.into(),
            chroma_stride: u32::try_from(chroma_width * 2).ok()?,
            generation: 0,
        })
    }

    /// plane index with bytes_per_pixel byte pixels copied row by row without row padding
    /// gives bytes and width and height of the plane, base address must be locked
    fn packed_plane(buffer: &CVPixelBuffer, index: usize, bytes_per_pixel: usize) -> Option<(Vec<u8>, usize, usize)> {
        let (width, height) = (buffer.get_width_of_plane(index), buffer.get_height_of_plane(index));
        let stride = buffer.get_bytes_per_row_of_plane(index);
        let row_length = width.checked_mul(bytes_per_pixel)?;
        if width == 0 || height == 0 || stride < row_length {
            return None;
        }
        let base = unsafe { buffer.get_base_address_of_plane(index) } as *const u8;
        if base.is_null() {
            return None;
        }
        let mut packed = Vec::with_capacity(row_length.checked_mul(height)?);
        for row in 0..height {
            let bytes = unsafe { std::slice::from_raw_parts(base.add(row * stride), row_length) };
            packed.extend_from_slice(bytes);
        }
        Some((packed, width, height))
    }
}

impl MacPixelBuffer {
    fn bgra_rows(&self) -> Option<(Vec<u8>, usize, usize)> {
        let buffer = &self.buffer;
        if buffer.get_pixel_format() != ALPHA_PIXEL_FORMAT || buffer.is_planar() {
            return None;
        }
        let (width, height, stride) = (buffer.get_width(), buffer.get_height(), buffer.get_bytes_per_row());
        let row_length = width.checked_mul(4)?;
        if width == 0 || height == 0 || stride < row_length {
            return None;
        }
        if buffer.lock_base_address(kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess {
            return None;
        }
        let base = unsafe { buffer.get_base_address() } as *const u8;
        let mut packed = Vec::new();
        if !base.is_null() {
            packed.reserve(row_length * height);
            for row in 0..height {
                let bytes = unsafe { std::slice::from_raw_parts(base.add(row * stride), row_length) };
                packed.extend_from_slice(bytes);
            }
        }
        buffer.unlock_base_address(kCVPixelBufferLock_ReadOnly);
        (!packed.is_empty()).then_some((packed, width, height))
    }
}

impl PixelBufferImage for MacPixelBuffer {
    fn oriented_image(self: Box<Self>) -> Option<gpui::RenderImage> {
        if self.buffer.get_pixel_format() == ALPHA_PIXEL_FORMAT {
            let (rows, width, height) = self.bgra_rows()?;
            let (width, height, bgra) = turned_bgra(width, height, &rows, self.clockwise_quarter_turns)?;
            return super::pixel_buffer::render_image_of_bgra(width, height, bgra);
        }
        let picture = self.nv12_picture()?;
        let ycbcr_to_rgb =
            ycbcr_to_rgb_columns(&ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt601, ColorRange::Full));
        Box::new(Nv12PixelBuffer::new(picture, ycbcr_to_rgb, self.clockwise_quarter_turns)).oriented_image()
    }
}

const AV_TIME_CONTROL_PAUSED: isize = 0;
const AV_TIME_CONTROL_WAITING: isize = 1;
const AV_TIME_CONTROL_PLAYING: isize = 2;

const AV_ITEM_STATUS_UNKNOWN: isize = 0;
const AV_ITEM_STATUS_READY: isize = 1;
const AV_ITEM_STATUS_FAILED: isize = 2;

static NEXT_SEEK: AtomicU64 = AtomicU64::new(1);
static PENDING_SEEKS: Mutex<Option<HashMap<u64, SeekSink>>> = Mutex::new(None);

const BLOCK_HAS_SIGNATURE: c_int = 1 << 30;

#[repr(C)]
struct BlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
    signature: *const c_char,
}

unsafe impl Sync for BlockDescriptor {}

#[repr(C)]
struct SeekBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: *const c_void,
    descriptor: *const BlockDescriptor,
    id: u64,
}

unsafe impl RefEncode for SeekBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

static SEEK_DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<SeekBlock>() as c_ulong,
    signature: c"v12@?0B8".as_ptr(),
};

unsafe extern "C" fn seek_answered(block: *const SeekBlock, finished: Bool) {
    let id = unsafe { (*block).id };
    let Some(done) = PENDING_SEEKS.lock().as_mut().and_then(|pending| pending.remove(&id)) else {
        return;
    };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| done.deliver(finished.as_bool()))).is_err() {
        tracing::error!(target: "riseonly", "media seek completion panicked");
    }
}

/// asset url of a player item as an absolute string
fn item_asset_url(item: &AnyObject) -> Option<String> {
    let url_asset_class = AnyClass::get(c"AVURLAsset")?;
    unsafe {
        let asset: Option<Retained<AnyObject>> = msg_send![item, asset];
        let asset = asset?;
        let is_url_asset: Bool = msg_send![&*asset, isKindOfClass: url_asset_class];
        if !is_url_asset.as_bool() {
            return None;
        }
        let url: Option<Retained<NSURL>> = msg_send![&*asset, URL];
        url?.absoluteString().map(|text| text.to_string())
    }
}
