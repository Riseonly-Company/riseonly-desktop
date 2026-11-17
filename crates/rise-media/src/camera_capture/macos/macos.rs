use std::ffi::{c_char, c_void, CStr};
use std::path::Path;
use std::time::Duration;

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{define_class, msg_send, sel, AnyThread as _, DefinedClass as _};
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSString, NSURL};
use parking_lot::{Condvar, Mutex};
use rise_core::foundation::foundation_date::RiseClock;

use crate::camera_capture::camera_capture::*;

pub(crate) const POSITION_BACK: isize = 1;
pub(crate) const POSITION_FRONT: isize = 2;
const AUTHORIZATION_RESTRICTED: isize = 1;
const AUTHORIZATION_DENIED: isize = 2;
const AUTHORIZATION_AUTHORIZED: isize = 3;
pub(crate) const ERROR_DEVICE_IN_USE: isize = -11804;
pub(crate) const ERROR_DEVICE_NOT_CONNECTED: isize = -11814;
pub(crate) const ERROR_NOT_AUTHORIZED: isize = -11852;
pub(crate) const SUBTYPE_420V: u32 = u32::from_be_bytes(*b"420v");
pub(crate) const SUBTYPE_420F: u32 = u32::from_be_bytes(*b"420f");
pub(crate) const SUBTYPE_YUVS: u32 = u32::from_be_bytes(*b"yuvs");
pub(crate) const SUBTYPE_BGRA: u32 = u32::from_be_bytes(*b"BGRA");
pub(crate) const SUBTYPE_DMB1: u32 = u32::from_be_bytes(*b"dmb1");
pub(crate) const SUBTYPE_JPEG: u32 = u32::from_be_bytes(*b"jpeg");
const LOCK_READ_ONLY: u64 = 1;
const FINISH_TIMEOUT: Duration = Duration::from_secs(10);
const CMTIME_FLAGS_VALID: u32 = 1;
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

const FOCUS_MODE_AUTO: isize = 1;
const EXPOSURE_MODE_AUTO: isize = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CgPoint {
    x: f64,
    y: f64,
}

unsafe impl Encode for CgPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[Encoding::Double, Encoding::Double]);
}

/// core media time struct as laid out in CMTime.h
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

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CmVideoDimensions {
    width: i32,
    height: i32,
}

/// the struct behind CMSampleBufferRef
#[repr(C)]
struct CmSampleBuffer {
    _private: [u8; 0],
}

unsafe impl RefEncode for CmSampleBuffer {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("opaqueCMSampleBuffer", &[]));
}

#[repr(C)]
struct CmFormatDescription {
    _private: [u8; 0],
}

unsafe impl RefEncode for CmFormatDescription {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("opaqueCMFormatDescription", &[]));
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: &'static NSString;
    static AVMediaTypeAudio: &'static NSString;
    static AVCaptureDeviceTypeBuiltInWideAngleCamera: &'static NSString;
    static AVVideoCodecKey: &'static NSString;
    static AVVideoCodecTypeH264: &'static NSString;
    static AVCaptureSessionRuntimeErrorNotification: &'static NSString;
    static AVCaptureSessionErrorKey: &'static NSString;
    static AVCaptureDeviceWasDisconnectedNotification: &'static NSString;
    static AVErrorRecordingSuccessfullyFinishedKey: &'static NSString;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetImageBuffer(buffer: *mut CmSampleBuffer) -> *mut c_void;
    fn CMFormatDescriptionGetMediaSubType(description: *const CmFormatDescription) -> u32;
    fn CMVideoFormatDescriptionGetDimensions(description: *const CmFormatDescription) -> CmVideoDimensions;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: &'static NSString;
    fn CVPixelBufferLockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: *mut c_void, flags: u64) -> i32;
    fn CVPixelBufferGetBaseAddress(buffer: *mut c_void) -> *mut c_void;
    fn CVPixelBufferGetBytesPerRow(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetWidth(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetPixelFormatType(buffer: *mut c_void) -> u32;
}

unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut AnyObject;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// encoding a capture format hands over
/// none for one the hub has no name for (2vuy or 10 bit or prores) so its not offered as a mode
pub(crate) fn encoding_of(subtype: u32) -> Option<PixelEncoding> {
    match subtype {
        SUBTYPE_420V | SUBTYPE_420F => Some(PixelEncoding::Nv12),
        SUBTYPE_YUVS => Some(PixelEncoding::Yuyv),
        SUBTYPE_BGRA => Some(PixelEncoding::Bgra),
        SUBTYPE_DMB1 | SUBTYPE_JPEG => Some(PixelEncoding::Mjpeg),
        _ => None,
    }
}

/// hub position of a device, the facetime camera of a mac reports unspecified on older models
/// it looks at the user so a built in camera without a position is the front one
pub(crate) fn position_of(raw: isize, built_in: bool) -> CameraPosition {
    match raw {
        POSITION_FRONT => CameraPosition::Front,
        POSITION_BACK => CameraPosition::Back,
        _ if built_in => CameraPosition::Front,
        _ => CameraPosition::External,
    }
}

/// one mode from a format dimensions and the shortest frame duration of its frame rate ranges
pub(crate) fn mode_of(
    encoding: PixelEncoding,
    width: i32,
    height: i32,
    duration_value: i64,
    duration_timescale: i32,
) -> Option<CameraMode> {
    let width = u32::try_from(width).ok().filter(|width| *width > 0)?;
    let height = u32::try_from(height).ok().filter(|height| *height > 0)?;
    let interval_numerator = u32::try_from(duration_value).ok().filter(|value| *value > 0)?;
    let interval_denominator = u32::try_from(duration_timescale).ok().filter(|scale| *scale > 0)?;
    Some(CameraMode { encoding, width, height, interval_numerator, interval_denominator })
}

/// hub error for an AVError raised while the device input opens
pub(crate) fn open_error(code: isize, description: String) -> CameraCaptureError {
    match code {
        ERROR_DEVICE_IN_USE => CameraCaptureError::Busy,
        ERROR_NOT_AUTHORIZED => CameraCaptureError::AccessDenied,
        ERROR_DEVICE_NOT_CONNECTED => CameraCaptureError::NoCamera,
        _ => CameraCaptureError::OpenFailed(description),
    }
}

/// hub error for a capture session runtime error, an unplugged camera is NoCamera like on other hosts
pub(crate) fn runtime_failure(code: isize, description: String) -> CameraCaptureError {
    match code {
        ERROR_DEVICE_IN_USE => CameraCaptureError::Busy,
        ERROR_DEVICE_NOT_CONNECTED => CameraCaptureError::NoCamera,
        _ => CameraCaptureError::StreamFailed(description),
    }
}

/// end of one segment
/// movie output may report an error yet have written a playable file, eg full disk
/// then the segment counts
pub(crate) fn segment_outcome(error: Option<(String, bool)>) -> Result<(), CameraCaptureError> {
    match error {
        None | Some((_, true)) => Ok(()),
        Some((description, false)) => Err(CameraCaptureError::FileFailed(description)),
    }
}

/// packed bgra rows from a pixel buffer whose rows are stride bytes apart
pub(crate) fn pack_rows(
    bytes: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) -> Result<Vec<u8>, CameraCaptureError> {
    let row = width * 4;
    if width == 0 || height == 0 || stride < row {
        return Err(CameraCaptureError::BadFrame("BGRA geometry"));
    }
    if bytes.len() < stride * (height - 1) + row {
        return Err(CameraCaptureError::BadFrame("the pixel buffer is shorter than its geometry"));
    }
    let mut packed = Vec::with_capacity(row * height);
    for line in 0..height {
        packed.extend_from_slice(&bytes[line * stride..line * stride + row]);
    }
    Ok(packed)
}

fn class(name: &'static CStr) -> Result<&'static AnyClass, CameraCaptureError> {
    AnyClass::get(name).ok_or_else(|| {
        CameraCaptureError::Unavailable(format!("AVFoundation has no {}", name.to_string_lossy()))
    })
}

fn describe(error: &NSError) -> (isize, String) {
    (error.code(), error.localizedDescription().to_string())
}

/// device type by its symbol, looked up at run time
fn device_type(symbol: &CStr) -> Option<&'static NSString> {
    unsafe {
        let address = dlsym(RTLD_DEFAULT, symbol.as_ptr());
        if address.is_null() {
            return None;
        }
        address.cast::<*const NSString>().read().as_ref()
    }
}

fn device_types() -> Retained<NSArray<NSString>> {
    let external = device_type(c"AVCaptureDeviceTypeExternal")
        .or_else(|| device_type(c"AVCaptureDeviceTypeExternalUnknown"));
    let built_in = unsafe { AVCaptureDeviceTypeBuiltInWideAngleCamera };
    match external {
        Some(external) => NSArray::from_slice(&[built_in, external]),
        None => NSArray::from_slice(&[built_in]),
    }
}

/// every camera with its capture device, built in ones first
fn discover() -> Result<Vec<(CameraDevice, Retained<AnyObject>)>, CameraCaptureError> {
    let discovery = class(c"AVCaptureDeviceDiscoverySession")?;
    let types = device_types();
    let session: Option<Retained<AnyObject>> = unsafe {
        msg_send![
            discovery,
            discoverySessionWithDeviceTypes: &*types,
            mediaType: AVMediaTypeVideo,
            position: 0isize
        ]
    };
    let session = session
        .ok_or_else(|| CameraCaptureError::Unavailable("the camera discovery session is nil".into()))?;
    let devices: Retained<NSArray<AnyObject>> = unsafe { msg_send![&*session, devices] };
    let mut found: Vec<(CameraDevice, Retained<AnyObject>)> = (0..devices.count())
        .map(|index| devices.objectAtIndex(index))
        .map(|device| (read_device(&device), device))
        .collect();
    found.sort_by_key(|(device, _)| !device.built_in);
    Ok(found)
}

fn read_device(device: &AnyObject) -> CameraDevice {
    let (id, name, kind, position, built_in_type): (
        Retained<NSString>,
        Retained<NSString>,
        Retained<NSString>,
        isize,
        &NSString,
    ) = unsafe {
        (
            msg_send![device, uniqueID],
            msg_send![device, localizedName],
            msg_send![device, deviceType],
            msg_send![device, position],
            AVCaptureDeviceTypeBuiltInWideAngleCamera,
        )
    };
    let built_in = kind.isEqualToString(built_in_type);
    CameraDevice {
        id: id.to_string(),
        name: name.to_string(),
        position: position_of(position, built_in),
        built_in,
    }
}

fn device_with_id(id: &str) -> Result<Retained<AnyObject>, CameraCaptureError> {
    let device_class = class(c"AVCaptureDevice")?;
    let device: Option<Retained<AnyObject>> =
        unsafe { msg_send![device_class, deviceWithUniqueID: &*NSString::from_str(id)] };
    device.ok_or(CameraCaptureError::NoCamera)
}

/// one mode with the format and frame duration that select it on the device
struct FormatChoice {
    mode: CameraMode,
    format: Retained<AnyObject>,
    min_frame_duration: CmTime,
}

fn formats_of(device: &AnyObject) -> Vec<FormatChoice> {
    let formats: Retained<NSArray<AnyObject>> = unsafe { msg_send![device, formats] };
    let mut choices = Vec::new();
    for index in 0..formats.count() {
        let format = formats.objectAtIndex(index);
        let (subtype, dimensions) = unsafe {
            let description: *const CmFormatDescription = msg_send![&*format, formatDescription];
            if description.is_null() {
                continue;
            }
            (CMFormatDescriptionGetMediaSubType(description), CMVideoFormatDescriptionGetDimensions(description))
        };
        let Some(encoding) = encoding_of(subtype) else { continue };
        let ranges: Retained<NSArray<AnyObject>> = unsafe { msg_send![&*format, videoSupportedFrameRateRanges] };
        for range_index in 0..ranges.count() {
            let range = ranges.objectAtIndex(range_index);
            let min_frame_duration: CmTime = unsafe { msg_send![&*range, minFrameDuration] };
            if !usable(&min_frame_duration) {
                continue;
            }
            if let Some(mode) = mode_of(
                encoding,
                dimensions.width,
                dimensions.height,
                min_frame_duration.value,
                min_frame_duration.timescale,
            ) {
                choices.push(FormatChoice { mode, format: format.clone(), min_frame_duration });
            }
        }
    }
    choices
}

/// true if the device can be set to this frame duration, a valid time on the default epoch
fn usable(time: &CmTime) -> bool {
    time.flags & CMTIME_FLAGS_VALID != 0 && time.epoch == 0
}

fn authorization_refused() -> Result<bool, CameraCaptureError> {
    let device_class = class(c"AVCaptureDevice")?;
    let status: isize = unsafe { msg_send![device_class, authorizationStatusForMediaType: AVMediaTypeVideo] };
    Ok(matches!(status, AUTHORIZATION_RESTRICTED | AUTHORIZATION_DENIED))
}

fn new_object(name: &'static CStr) -> Result<Retained<AnyObject>, CameraCaptureError> {
    let object_class = class(name)?;
    let object: Option<Retained<AnyObject>> = unsafe { msg_send![object_class, new] };
    object.ok_or_else(|| CameraCaptureError::OpenFailed(format!("{} new returned nil", name.to_string_lossy())))
}

fn video_connection(output: &AnyObject) -> Option<Retained<AnyObject>> {
    unsafe { msg_send![output, connectionWithMediaType: AVMediaTypeVideo] }
}

/// the picture of a connection is never mirrored, the file is recorded as is
/// and the overlay flips the preview instead
fn unmirror(connection: &AnyObject) {
    unsafe {
        let supported: bool = msg_send![connection, isVideoMirroringSupported];
        if supported {
            let _: () = msg_send![connection, setAutomaticallyAdjustsVideoMirroring: Bool::NO];
            let _: () = msg_send![connection, setVideoMirrored: Bool::NO];
        }
    }
}

fn set_mirrored(connection: &AnyObject, on: bool) -> bool {
    unsafe {
        let supported: bool = msg_send![connection, isVideoMirroringSupported];
        if supported {
            let _: () = msg_send![connection, setAutomaticallyAdjustsVideoMirroring: Bool::NO];
            let _: () = msg_send![connection, setVideoMirrored: Bool::new(on)];
        }
        supported
    }
}

/// what the capture queue and movie output and notification center report to the session
struct Route {
    sink: FrameSink,
    mirrored: bool,
    failure: Mutex<Option<CameraCaptureError>>,
    segment: Mutex<Segment>,
    segment_done: Condvar,
}

enum Segment {
    Idle,
    Writing,
    Finished(Result<(), CameraCaptureError>),
}

impl Route {
    fn deliver(&self, buffer: *mut CmSampleBuffer) {
        let captured_at = RiseClock::monotonic();
        match read_frame(buffer, captured_at, self.mirrored) {
            Ok(frame) => (self.sink)(frame),
            Err(error) => tracing::debug!(target: "riseonly", %error, "camera frame skipped"),
        }
    }

    fn fail(&self, error: CameraCaptureError) {
        let mut failure = self.failure.lock();
        if failure.is_none() {
            tracing::warn!(target: "riseonly", %error, "camera stopped");
            *failure = Some(error);
        }
    }

    fn finish(&self, outcome: Result<(), CameraCaptureError>) {
        *self.segment.lock() = Segment::Finished(outcome);
        self.segment_done.notify_all();
    }

    fn wait_finished(&self) -> Result<(), CameraCaptureError> {
        let mut segment = self.segment.lock();
        let waited =
            self.segment_done.wait_while_for(&mut segment, |segment| matches!(segment, Segment::Writing), FINISH_TIMEOUT);
        if waited.timed_out() {
            *segment = Segment::Idle;
            return Err(CameraCaptureError::FileFailed("the movie file output did not close the segment".into()));
        }
        match std::mem::replace(&mut *segment, Segment::Idle) {
            Segment::Finished(outcome) => outcome,
            Segment::Idle | Segment::Writing => Err(CameraCaptureError::NotRecording),
        }
    }
}

fn read_frame(buffer: *mut CmSampleBuffer, captured_at: Duration, mirrored: bool) -> Result<CameraFrame, CameraCaptureError> {
    let (packed, width, height) = unsafe {
        let pixels = CMSampleBufferGetImageBuffer(buffer);
        if pixels.is_null() {
            return Err(CameraCaptureError::BadFrame("the sample buffer holds no image"));
        }
        if CVPixelBufferGetPixelFormatType(pixels) != SUBTYPE_BGRA {
            return Err(CameraCaptureError::BadFrame("the frame is not BGRA"));
        }
        if CVPixelBufferLockBaseAddress(pixels, LOCK_READ_ONLY) != 0 {
            return Err(CameraCaptureError::BadFrame("the pixel buffer could not be locked"));
        }
        let base = CVPixelBufferGetBaseAddress(pixels);
        let stride = CVPixelBufferGetBytesPerRow(pixels);
        let width = CVPixelBufferGetWidth(pixels);
        let height = CVPixelBufferGetHeight(pixels);
        let packed = if base.is_null() {
            Err(CameraCaptureError::BadFrame("the pixel buffer has no base address"))
        } else {
            pack_rows(std::slice::from_raw_parts(base.cast::<u8>(), stride * height), width, height, stride)
        };
        CVPixelBufferUnlockBaseAddress(pixels, LOCK_READ_ONLY);
        (packed?, width, height)
    };
    let width = u32::try_from(width).map_err(|_| CameraCaptureError::BadFrame("BGRA width"))?;
    let height = u32::try_from(height).map_err(|_| CameraCaptureError::BadFrame("BGRA height"))?;
    bgra_frame(packed, width, height, captured_at, mirrored)
}

fn guarded(what: &'static str, f: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
        tracing::error!(target: "riseonly", what, "camera callback panicked");
    }
}

fn read_recording_error(error: &NSError) -> (String, bool) {
    let finished = unsafe {
        error
            .userInfo()
            .objectForKey(AVErrorRecordingSuccessfullyFinishedKey)
            .is_some_and(|value| msg_send![&*value, boolValue])
    };
    (error.localizedDescription().to_string(), finished)
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseCameraCaptureDelegate"]
    #[ivars = Route]
    struct CameraDelegate;

    impl CameraDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn did_output_sample_buffer(&self, _output: &AnyObject, buffer: *mut CmSampleBuffer, _connection: &AnyObject) {
            guarded("frame", || self.ivars().deliver(buffer));
        }

        #[unsafe(method(captureOutput:didFinishRecordingToOutputFileAtURL:fromConnections:error:))]
        fn did_finish_recording(
            &self,
            _output: &AnyObject,
            _url: &AnyObject,
            _connections: &AnyObject,
            error: Option<&NSError>,
        ) {
            guarded("segment", || self.ivars().finish(segment_outcome(error.map(read_recording_error))));
        }

        #[unsafe(method(sessionRuntimeError:))]
        fn session_runtime_error(&self, notification: &AnyObject) {
            let error: Option<Retained<NSError>> = unsafe {
                let info: Option<Retained<NSDictionary<NSString, AnyObject>>> = msg_send![notification, userInfo];
                info.and_then(|info| info.objectForKey(AVCaptureSessionErrorKey))
                    .and_then(|value| value.downcast::<NSError>().ok())
            };
            let failure = match error {
                Some(error) => {
                    let (code, description) = describe(&error);
                    runtime_failure(code, description)
                }
                None => CameraCaptureError::StreamFailed("the capture session stopped".into()),
            };
            guarded("runtime error", || self.ivars().fail(failure));
        }

        #[unsafe(method(deviceWasDisconnected:))]
        fn device_was_disconnected(&self, _notification: &AnyObject) {
            guarded("disconnect", || self.ivars().fail(CameraCaptureError::NoCamera));
        }
    }
);

impl CameraDelegate {
    fn new(route: Route) -> Retained<Self> {
        let this = Self::alloc().set_ivars(route);
        unsafe { msg_send![super(this), init] }
    }
}

fn notification_center() -> Result<Retained<AnyObject>, CameraCaptureError> {
    let center_class = class(c"NSNotificationCenter")?;
    let center: Option<Retained<AnyObject>> = unsafe { msg_send![center_class, defaultCenter] };
    center.ok_or_else(|| CameraCaptureError::Unavailable("no notification center".into()))
}

pub(crate) struct NativeCamera;

impl CameraSource for NativeCamera {
    fn support(&self) -> Result<(), CameraCaptureError> {
        if discover()?.is_empty() {
            return Err(CameraCaptureError::NoCamera);
        }
        Ok(())
    }

    fn recording_support(&self) -> Result<(), CameraCaptureError> {
        class(c"AVCaptureMovieFileOutput").map(|_| ())
    }

    fn devices(&self) -> Result<Vec<CameraDevice>, CameraCaptureError> {
        Ok(discover()?.into_iter().map(|(device, _)| device).collect())
    }

    fn open(&self, device: &CameraDevice, frames: FrameSink) -> Result<Box<dyn CameraSession>, CameraCaptureError> {
        if authorization_refused()? {
            return Err(CameraCaptureError::AccessDenied);
        }
        let capture_device = device_with_id(&device.id)?;
        let choices = formats_of(&capture_device);
        let modes: Vec<CameraMode> = choices.iter().map(|choice| choice.mode).collect();
        let choice = choose_mode(&modes)
            .and_then(|mode| choices.into_iter().find(|choice| choice.mode == mode))
            .ok_or_else(|| CameraCaptureError::OpenFailed("the camera offers no video format".into()))?;
        Ok(Box::new(MacSession::open(device.clone(), capture_device, choice, frames)?))
    }
}

/// the capture objects of one open camera
struct CaptureObjects {
    capture_device: Retained<AnyObject>,
    session: Retained<AnyObject>,
    data_output: Retained<AnyObject>,
    movie_output: Retained<AnyObject>,
    delegate: Retained<CameraDelegate>,
    _queue: Retained<AnyObject>,
    center: Retained<AnyObject>,
    audio_input: Option<Retained<AnyObject>>,
}

unsafe impl Send for CaptureObjects {}

pub(crate) struct MacSession {
    device: CameraDevice,
    mode: CameraMode,
    objects: CaptureObjects,
    recording: bool,
    running: bool,
    microphone_wanted: bool,
    mirrors_recording: bool,
}

impl MacSession {
    fn open(
        device: CameraDevice,
        capture_device: Retained<AnyObject>,
        choice: FormatChoice,
        frames: FrameSink,
    ) -> Result<Self, CameraCaptureError> {
        let route = Route {
            sink: frames,
            mirrored: device.position == CameraPosition::Front,
            failure: Mutex::new(None),
            segment: Mutex::new(Segment::Idle),
            segment_done: Condvar::new(),
        };
        let input_class = class(c"AVCaptureDeviceInput")?;
        let opened: Result<Retained<AnyObject>, Retained<NSError>> =
            unsafe { msg_send![input_class, deviceInputWithDevice: &*capture_device, error: _] };
        let input = opened.map_err(|error| {
            let (code, description) = describe(&error);
            open_error(code, description)
        })?;
        let queue = unsafe {
            Retained::from_raw(dispatch_queue_create(c"rise.camera.capture".as_ptr(), std::ptr::null_mut()))
        }
        .ok_or_else(|| CameraCaptureError::OpenFailed("the capture queue was not created".into()))?;
        let objects = CaptureObjects {
            capture_device,
            session: new_object(c"AVCaptureSession")?,
            data_output: new_object(c"AVCaptureVideoDataOutput")?,
            movie_output: new_object(c"AVCaptureMovieFileOutput")?,
            delegate: CameraDelegate::new(route),
            _queue: queue,
            center: notification_center()?,
            audio_input: None,
        };
        let mut this = Self {
            device,
            mode: choice.mode,
            objects,
            recording: false,
            running: false,
            microphone_wanted: false,
            mirrors_recording: false,
        };
        this.configure(&input, &choice)?;
        this.start_running()?;
        Ok(this)
    }

    fn configure(&mut self, input: &AnyObject, choice: &FormatChoice) -> Result<(), CameraCaptureError> {
        let objects = &self.objects;
        let settings = unsafe {
            NSDictionary::<NSString, NSNumber>::from_slices(
                &[kCVPixelBufferPixelFormatTypeKey],
                &[&*NSNumber::numberWithUnsignedInt(SUBTYPE_BGRA)],
            )
        };
        let configured = unsafe {
            let _: () = msg_send![&*objects.session, beginConfiguration];
            let configured = (|| {
                let can_add: bool = msg_send![&*objects.session, canAddInput: input];
                if !can_add {
                    return Err(CameraCaptureError::OpenFailed("the session refused the camera input".into()));
                }
                let _: () = msg_send![&*objects.session, addInput: input];
                let _: () = msg_send![&*objects.data_output, setVideoSettings: &*settings];
                let _: () = msg_send![&*objects.data_output, setAlwaysDiscardsLateVideoFrames: Bool::YES];
                let _: () = msg_send![
                    &*objects.data_output,
                    setSampleBufferDelegate: &*objects.delegate,
                    queue: &*objects._queue
                ];
                for output in [&objects.data_output, &objects.movie_output] {
                    let can_add: bool = msg_send![&*objects.session, canAddOutput: &**output];
                    if !can_add {
                        return Err(CameraCaptureError::OpenFailed("the session refused a capture output".into()));
                    }
                    let _: () = msg_send![&*objects.session, addOutput: &**output];
                }
                let locked: Result<(), Retained<NSError>> =
                    msg_send![&*objects.capture_device, lockForConfiguration: _];
                locked.map_err(|error| CameraCaptureError::OpenFailed(describe(&error).1))?;
                let _: () = msg_send![&*objects.capture_device, setActiveFormat: &*choice.format];
                let _: () = msg_send![&*objects.capture_device, setActiveVideoMinFrameDuration: choice.min_frame_duration];
                let _: () = msg_send![&*objects.capture_device, unlockForConfiguration];
                Ok(())
            })();
            let _: () = msg_send![&*objects.session, commitConfiguration];
            configured
        };
        configured?;
        if let Some(connection) = video_connection(&objects.data_output) {
            unmirror(&connection);
        }
        let connection = video_connection(&objects.movie_output)
            .ok_or_else(|| CameraCaptureError::OpenFailed("the movie output has no video connection".into()))?;
        unmirror(&connection);
        self.choose_h264(&connection)?;
        self.observe()
    }

    fn choose_h264(&self, connection: &AnyObject) -> Result<(), CameraCaptureError> {
        let movie_output = &self.objects.movie_output;
        unsafe {
            let codecs: Retained<NSArray<NSString>> = msg_send![&**movie_output, availableVideoCodecTypes];
            let h264 = AVVideoCodecTypeH264;
            if !(0..codecs.count()).any(|index| codecs.objectAtIndex(index).isEqualToString(h264)) {
                return Err(CameraCaptureError::EncoderUnavailable(
                    "the movie file output offers no H.264 codec for this camera".into(),
                ));
            }
            let settings = NSDictionary::<NSString, NSString>::from_slices(&[AVVideoCodecKey], &[h264]);
            let _: () = msg_send![&**movie_output, setOutputSettings: &*settings, forConnection: connection];
        }
        Ok(())
    }

    /// runtime errors of the session and unplugging the camera end the take (see failure)
    fn observe(&self) -> Result<(), CameraCaptureError> {
        let objects = &self.objects;
        unsafe {
            let _: () = msg_send![
                &*objects.center,
                addObserver: &*objects.delegate,
                selector: sel!(sessionRuntimeError:),
                name: AVCaptureSessionRuntimeErrorNotification,
                object: &*objects.session
            ];
            let _: () = msg_send![
                &*objects.center,
                addObserver: &*objects.delegate,
                selector: sel!(deviceWasDisconnected:),
                name: AVCaptureDeviceWasDisconnectedNotification,
                object: &*objects.capture_device
            ];
        }
        Ok(())
    }

    fn route(&self) -> &Route {
        self.objects.delegate.ivars()
    }

    /// true if the capture device answers selector, zoom properties came in macos 14
    /// and subject area monitoring is ios only so each is asked first
    fn device_responds(&self, selector: objc2::runtime::Sel) -> bool {
        unsafe { msg_send![&*self.objects.capture_device, respondsToSelector: selector] }
    }

    /// runs change with the device locked for configuration, as every device change needs
    fn with_device_locked(&self, change: impl FnOnce(&AnyObject)) -> Result<(), CameraCaptureError> {
        let device = &*self.objects.capture_device;
        let locked: Result<(), Retained<NSError>> = unsafe { msg_send![device, lockForConfiguration: _] };
        locked.map_err(|error| CameraCaptureError::StreamFailed(describe(&error).1))?;
        change(device);
        unsafe {
            let _: () = msg_send![device, unlockForConfiguration];
        }
        Ok(())
    }

    /// mic input follows the wish between segments and never inside one
    /// adding an input while the movie output writes would change the running file
    fn sync_microphone(&mut self) -> Result<(), CameraCaptureError> {
        match (self.microphone_wanted, self.objects.audio_input.is_some()) {
            (true, false) => self.attach_microphone(),
            (false, true) => {
                self.detach_microphone();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn attach_microphone(&mut self) -> Result<(), CameraCaptureError> {
        let device_class = class(c"AVCaptureDevice")?;
        let (status, microphone): (isize, Option<Retained<AnyObject>>) = unsafe {
            (
                msg_send![device_class, authorizationStatusForMediaType: AVMediaTypeAudio],
                msg_send![device_class, defaultDeviceWithMediaType: AVMediaTypeAudio],
            )
        };
        if status != AUTHORIZATION_AUTHORIZED {
            return Err(CameraCaptureError::AccessDenied);
        }
        let microphone =
            microphone.ok_or_else(|| CameraCaptureError::Unavailable("no microphone is connected".into()))?;
        let input_class = class(c"AVCaptureDeviceInput")?;
        let opened: Result<Retained<AnyObject>, Retained<NSError>> =
            unsafe { msg_send![input_class, deviceInputWithDevice: &*microphone, error: _] };
        let input = opened.map_err(|error| CameraCaptureError::Unavailable(describe(&error).1))?;
        let session = &*self.objects.session;
        let added: bool = unsafe {
            let _: () = msg_send![session, beginConfiguration];
            let can_add: bool = msg_send![session, canAddInput: &*input];
            if can_add {
                let _: () = msg_send![session, addInput: &*input];
            }
            let _: () = msg_send![session, commitConfiguration];
            can_add
        };
        if !added {
            return Err(CameraCaptureError::Unavailable("the session refused the microphone input".into()));
        }
        self.objects.audio_input = Some(input);
        Ok(())
    }

    fn detach_microphone(&mut self) {
        let Some(input) = self.objects.audio_input.take() else {
            return;
        };
        let session = &*self.objects.session;
        unsafe {
            let _: () = msg_send![session, beginConfiguration];
            let _: () = msg_send![session, removeInput: &*input];
            let _: () = msg_send![session, commitConfiguration];
        }
    }

    fn start_running(&mut self) -> Result<(), CameraCaptureError> {
        let running: bool = unsafe {
            let _: () = msg_send![&*self.objects.session, startRunning];
            msg_send![&*self.objects.session, isRunning]
        };
        self.running = running;
        if running {
            return Ok(());
        }
        Err(self
            .route()
            .failure
            .lock()
            .clone()
            .unwrap_or_else(|| CameraCaptureError::OpenFailed("the capture session did not start".into())))
    }
}

impl CameraSession for MacSession {
    fn device(&self) -> &CameraDevice {
        &self.device
    }

    fn mode(&self) -> CameraMode {
        self.mode
    }

    fn start_segment(&mut self, path: &Path) -> Result<(), CameraCaptureError> {
        if self.recording {
            return Err(CameraCaptureError::AlreadyRecording);
        }
        if let Some(failure) = self.failure() {
            return Err(failure);
        }
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(CameraCaptureError::FileFailed(error.to_string())),
        }
        let text = path
            .to_str()
            .ok_or_else(|| CameraCaptureError::FileFailed("the segment path is not UTF-8".into()))?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(text));
        if let Err(error) = self.sync_microphone() {
            tracing::warn!(target: "riseonly", %error, "segment recorded without sound");
        }
        let connection = video_connection(&self.objects.movie_output)
            .ok_or_else(|| CameraCaptureError::StreamFailed("the movie output has no video connection".into()))?;
        let (enabled, active): (bool, bool) =
            unsafe { (msg_send![&*connection, isEnabled], msg_send![&*connection, isActive]) };
        if !enabled || !active {
            return Err(CameraCaptureError::StreamFailed("the movie output has no active video connection".into()));
        }
        set_mirrored(&connection, self.mirrors_recording);
        *self.route().segment.lock() = Segment::Writing;
        unsafe {
            let _: () = msg_send![
                &*self.objects.movie_output,
                startRecordingToOutputFileURL: &*url,
                recordingDelegate: &*self.objects.delegate
            ];
        }
        self.recording = true;
        Ok(())
    }

    fn stop_segment(&mut self) -> Result<(), CameraCaptureError> {
        if !self.recording {
            return Err(CameraCaptureError::NotRecording);
        }
        self.recording = false;
        let writing = matches!(*self.route().segment.lock(), Segment::Writing);
        if writing {
            unsafe {
                let _: () = msg_send![&*self.objects.movie_output, stopRecording];
            }
        }
        self.route().wait_finished()
    }

    fn failure(&self) -> Option<CameraCaptureError> {
        self.route().failure.lock().clone()
    }

    fn set_microphone(&mut self, on: bool) -> Result<(), CameraCaptureError> {
        let before = self.microphone_wanted;
        self.microphone_wanted = on;
        if self.recording {
            return Ok(());
        }
        let synced = self.sync_microphone();
        if synced.is_err() {
            self.microphone_wanted = before;
        }
        synced
    }

    fn set_mirrors_recording(&mut self, on: bool) -> Result<(), CameraCaptureError> {
        if on {
            let connection = video_connection(&self.objects.movie_output)
                .ok_or_else(|| CameraCaptureError::StreamFailed("the movie output has no video connection".into()))?;
            let supported: bool = unsafe { msg_send![&*connection, isVideoMirroringSupported] };
            if !supported {
                return Err(CameraCaptureError::MirroringUnsupported);
            }
        }
        self.mirrors_recording = on;
        Ok(())
    }

    fn zoom_range(&self) -> Option<(f32, f32)> {
        let known = self.device_responds(sel!(minAvailableVideoZoomFactor))
            && self.device_responds(sel!(maxAvailableVideoZoomFactor))
            && self.device_responds(sel!(setVideoZoomFactor:));
        if !known {
            return None;
        }
        let device = &*self.objects.capture_device;
        let (lower, upper): (f64, f64) = unsafe {
            (msg_send![device, minAvailableVideoZoomFactor], msg_send![device, maxAvailableVideoZoomFactor])
        };
        usable_zoom_range(lower as f32, upper as f32)
    }

    fn set_zoom_factor(&mut self, factor: f32) -> Result<(), CameraCaptureError> {
        let range = self.zoom_range().ok_or(CameraCaptureError::ZoomUnsupported)?;
        let factor = f64::from(clamp_zoom_factor(factor, range));
        self.with_device_locked(|device| {
            unsafe {
                let _: () = msg_send![device, setVideoZoomFactor: factor];
            }
        })
    }

    fn set_point_of_interest(&mut self, x: f32, y: f32) -> Result<(), CameraCaptureError> {
        let (x, y) = clamp_point_of_interest(x, y);
        let point = CgPoint { x: f64::from(x), y: f64::from(y) };
        let device = &*self.objects.capture_device;
        let (focus, expose): (bool, bool) = unsafe {
            let focus_point: bool = msg_send![device, isFocusPointOfInterestSupported];
            let focus_mode: bool = msg_send![device, isFocusModeSupported: FOCUS_MODE_AUTO];
            let expose_point: bool = msg_send![device, isExposurePointOfInterestSupported];
            let expose_mode: bool = msg_send![device, isExposureModeSupported: EXPOSURE_MODE_AUTO];
            (focus_point && focus_mode, expose_point && expose_mode)
        };
        if !focus && !expose {
            return Err(CameraCaptureError::PointOfInterestUnsupported);
        }
        let monitors = self.device_responds(sel!(setSubjectAreaChangeMonitoringEnabled:));
        self.with_device_locked(|device| {
            unsafe {
                if focus {
                    let _: () = msg_send![device, setFocusPointOfInterest: point];
                    let _: () = msg_send![device, setFocusMode: FOCUS_MODE_AUTO];
                }
                if expose {
                    let _: () = msg_send![device, setExposurePointOfInterest: point];
                    let _: () = msg_send![device, setExposureMode: EXPOSURE_MODE_AUTO];
                }
                if monitors {
                    let _: () = msg_send![device, setSubjectAreaChangeMonitoringEnabled: Bool::YES];
                }
            }
        })
    }
}

impl Drop for MacSession {
    fn drop(&mut self) {
        if self.recording {
            if let Err(error) = self.stop_segment() {
                tracing::debug!(target: "riseonly", %error, "camera segment closed on drop");
            }
        }
        let objects = &self.objects;
        unsafe {
            if self.running {
                let _: () = msg_send![&*objects.session, stopRunning];
            }
            let _: () = msg_send![
                &*objects.data_output,
                setSampleBufferDelegate: None::<&AnyObject>,
                queue: None::<&AnyObject>
            ];
            let _: () = msg_send![&*objects.center, removeObserver: &*objects.delegate];
        }
    }
}
