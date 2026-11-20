use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, MessageReceiver as _, NSObject, Sel};
use objc2::{define_class, msg_send, AnyThread as _, ClassType as _, DefinedClass as _, MainThreadMarker};
use objc2_foundation::{NSArray, NSError, NSNumber, NSRect, NSString};
use parking_lot::{Condvar, Mutex};
use rise_core::foundation::foundation_date::RiseClock;

use super::screen_capture::*;
use crate::texture::native_surface::retain_decoder_surface;

const OUTPUT_TYPE_SCREEN: isize = 0;
const OUTPUT_TYPE_AUDIO: isize = 1;
const FRAME_STATUS_COMPLETE: isize = 0;
const FRAME_STATUS_STOPPED: isize = 5;
const ERROR_USER_DECLINED: isize = -3801;
const ERROR_NO_CAPTURE_SOURCE: isize = -3815;
const ERROR_USER_STOPPED: isize = -3817;
const STYLE_WINDOW: isize = 1;
const STYLE_APPLICATION: isize = 3;
const PIXEL_FORMAT_420V: u32 = u32::from_be_bytes(*b"420v");
const AUDIO_FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
const AUDIO_FORMAT_FLAG_IS_FLOAT: u32 = 1;
const AUDIO_BUFFER_LIST_ASSURE_ALIGNMENT: u32 = 1;
const FLOAT_SAMPLE_BITS: u32 = 32;
const CMTIME_FLAGS_VALID: u32 = 1;
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const QUEUE_DEPTH: isize = 6;

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
struct CmSampleBuffer {
    _private: [u8; 0],
}

unsafe impl RefEncode for CmSampleBuffer {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("opaqueCMSampleBuffer", &[]));
}

#[repr(C)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    _bytes_per_packet: u32,
    _frames_per_packet: u32,
    _bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    _reserved: u32,
}

#[repr(C)]
struct AudioBuffer {
    _number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 1],
}

#[repr(C)]
struct CfString {
    _private: [u8; 0],
}

unsafe impl RefEncode for CfString {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CFString", &[]));
}

#[link(name = "ScreenCaptureKit", kind = "framework")]
unsafe extern "C" {
    static SCStreamFrameInfoStatus: &'static NSString;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
    fn CGDisplayModeGetPixelWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeGetPixelHeight(mode: *mut c_void) -> usize;
    fn CGDisplayModeRelease(mode: *mut c_void);
    static kCGDisplayStreamYCbCrMatrix_ITU_R_601_4: &'static CfString;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferGetImageBuffer(buffer: *mut CmSampleBuffer) -> *mut c_void;
    fn CMSampleBufferGetSampleAttachmentsArray(buffer: *mut CmSampleBuffer, create: u8) -> *const AnyObject;
    fn CMSampleBufferGetFormatDescription(buffer: *mut CmSampleBuffer) -> *const c_void;
    fn CMAudioFormatDescriptionGetStreamBasicDescription(description: *const c_void) -> *const AudioStreamBasicDescription;
    /// block buffer comes back retained and owns the memory the list points into
    fn CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        buffer: *mut CmSampleBuffer,
        size_needed: *mut usize,
        list: *mut AudioBufferList,
        list_size: usize,
        structure_allocator: *const c_void,
        block_allocator: *const c_void,
        flags: u32,
        block_buffer: *mut *mut c_void,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: *const c_void);
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferGetWidth(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetPixelFormatType(buffer: *mut c_void) -> u32;
}

unsafe extern "C" {
    static _NSConcreteStackBlock: [u8; 0];
    static _dispatch_main_q: [u8; 0];
    fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut AnyObject;
}

#[repr(C)]
struct BlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
}

#[repr(C)]
struct ContentBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*const ContentBlock, *mut AnyObject, *mut NSError),
    descriptor: *const BlockDescriptor,
    request: u64,
}

#[repr(C)]
struct ErrorBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*const ErrorBlock, *mut NSError),
    descriptor: *const BlockDescriptor,
    request: u64,
}

unsafe impl RefEncode for ContentBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

unsafe impl RefEncode for ErrorBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

static CONTENT_DESCRIPTOR: BlockDescriptor =
    BlockDescriptor { reserved: 0, size: std::mem::size_of::<ContentBlock>() as c_ulong };
static ERROR_DESCRIPTOR: BlockDescriptor =
    BlockDescriptor { reserved: 0, size: std::mem::size_of::<ErrorBlock>() as c_ulong };

/// objective c object moved between screencapturekit queues and the waiting thread
struct Shared(Retained<AnyObject>);

unsafe impl Send for Shared {}

/// failure as screencapturekit reports it, code and description
type Failure = (isize, String);

enum Reply {
    Content(Result<Shared, Failure>),
    Done(Option<Failure>),
}

struct Replies {
    answers: HashMap<u64, Reply>,
    next: u64,
}

static REPLIES: Mutex<Option<Replies>> = Mutex::new(None);
static REPLY_ARRIVED: Condvar = Condvar::new();

fn next_request() -> u64 {
    let mut guard = REPLIES.lock();
    let replies = guard.get_or_insert_with(|| Replies { answers: HashMap::new(), next: 1 });
    let request = replies.next;
    replies.next += 1;
    request
}

fn answer(request: u64, reply: Reply) {
    REPLIES
        .lock()
        .get_or_insert_with(|| Replies { answers: HashMap::new(), next: 1 })
        .answers
        .insert(request, reply);
    REPLY_ARRIVED.notify_all();
}

fn wait_reply(request: u64, timeout: Duration) -> Option<Reply> {
    let deadline = RiseClock::monotonic() + timeout;
    let mut replies = REPLIES.lock();
    loop {
        if let Some(reply) = replies.as_mut().and_then(|replies| replies.answers.remove(&request)) {
            return Some(reply);
        }
        let remaining = deadline.saturating_sub(RiseClock::monotonic());
        if remaining.is_zero() || REPLY_ARRIVED.wait_for(&mut replies, remaining).timed_out() {
            return replies.as_mut().and_then(|replies| replies.answers.remove(&request));
        }
    }
}

fn describe(error: &NSError) -> Failure {
    (error.code(), error.localizedDescription().to_string())
}

unsafe extern "C" fn content_answered(block: *const ContentBlock, content: *mut AnyObject, error: *mut NSError) {
    let (request, content, error) = unsafe { ((*block).request, Retained::retain(content), error.as_ref()) };
    let reply = match (content, error) {
        (Some(content), _) => Ok(Shared(content)),
        (None, Some(error)) => Err(describe(error)),
        (None, None) => Err((0, "no shareable content".into())),
    };
    answer(request, Reply::Content(reply));
}

unsafe extern "C" fn error_answered(block: *const ErrorBlock, error: *mut NSError) {
    let (request, error) = unsafe { ((*block).request, error.as_ref()) };
    answer(request, Reply::Done(error.map(describe)));
}

fn class(name: &'static std::ffi::CStr) -> Result<&'static AnyClass, ScreenCaptureError> {
    AnyClass::get(name).ok_or_else(|| {
        ScreenCaptureError::Unsupported(format!("ScreenCaptureKit has no {} (macOS 12.3 or later)", name.to_string_lossy()))
    })
}

fn start_failure((code, description): Failure) -> ScreenCaptureError {
    match code {
        ERROR_USER_DECLINED => ScreenCaptureError::AccessDenied,
        ERROR_NO_CAPTURE_SOURCE => ScreenCaptureError::SourceGone,
        _ => ScreenCaptureError::StartFailed(description),
    }
}

fn stop_end((code, description): Failure) -> ScreenCaptureEnd {
    match code {
        ERROR_USER_STOPPED => ScreenCaptureEnd::StoppedBySystem,
        ERROR_NO_CAPTURE_SOURCE => ScreenCaptureEnd::SourceLost,
        _ => ScreenCaptureEnd::Failed(ScreenCaptureError::StreamFailed(description)),
    }
}

/// current SCShareableContent with every on screen window included
fn shareable_content() -> Result<Retained<AnyObject>, ScreenCaptureError> {
    let content_class = class(c"SCShareableContent")?;
    let request = next_request();
    let block = ContentBlock {
        isa: (&raw const _NSConcreteStackBlock).cast(),
        flags: 0,
        reserved: 0,
        invoke: content_answered,
        descriptor: &raw const CONTENT_DESCRIPTOR,
        request,
    };
    unsafe {
        let _: () = msg_send![
            content_class,
            getShareableContentExcludingDesktopWindows: Bool::YES,
            onScreenWindowsOnly: Bool::YES,
            completionHandler: &raw const block
        ];
    }
    match wait_reply(request, REPLY_TIMEOUT) {
        Some(Reply::Content(Ok(Shared(content)))) => Ok(content),
        Some(Reply::Content(Err(failure))) => Err(start_failure(failure)),
        Some(Reply::Done(_)) | None => Err(ScreenCaptureError::StartFailed("no answer from ScreenCaptureKit".into())),
    }
}

fn displays(content: &AnyObject) -> Vec<Retained<AnyObject>> {
    let array: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![content, displays] };
    array.map(|array| array.to_vec()).unwrap_or_default()
}

fn windows(content: &AnyObject) -> Vec<Retained<AnyObject>> {
    let array: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![content, windows] };
    array.map(|array| array.to_vec()).unwrap_or_default()
}

fn display_pixels(display_id: u32) -> Option<(u32, u32)> {
    unsafe {
        let mode = CGDisplayCopyDisplayMode(display_id);
        if mode.is_null() {
            return None;
        }
        let size = (CGDisplayModeGetPixelWidth(mode), CGDisplayModeGetPixelHeight(mode));
        CGDisplayModeRelease(mode);
        Some((u32::try_from(size.0).ok()?, u32::try_from(size.1).ok()?))
    }
}

/// backing scale of the main screen, for a window whose size screencapturekit gives in points
fn main_backing_scale() -> f64 {
    let Some(screen_class) = AnyClass::get(c"NSScreen") else {
        return 1.0;
    };
    let screen: Option<Retained<AnyObject>> = unsafe { msg_send![screen_class, mainScreen] };
    screen.map(|screen| unsafe { msg_send![&*screen, backingScaleFactor] }).unwrap_or(1.0)
}

fn pixels_of(points: f64, scale: f64) -> u32 {
    (points * scale).round().clamp(0.0, f64::from(u32::MAX)) as u32
}

fn display_source(display: &AnyObject) -> CaptureSource {
    let (display_id, width, height): (u32, isize, isize) =
        unsafe { (msg_send![display, displayID], msg_send![display, width], msg_send![display, height]) };
    let (width, height) = display_pixels(display_id).unwrap_or_else(|| {
        let scale = main_backing_scale();
        (pixels_of(width as f64, scale), pixels_of(height as f64, scale))
    });
    CaptureSource { id: format!("display:{display_id}"), kind: CaptureSourceKind::Display, name: String::new(), width, height }
}

fn window_source(window: &AnyObject, scale: f64) -> Option<CaptureSource> {
    unsafe {
        let layer: isize = msg_send![window, windowLayer];
        if layer != 0 {
            return None;
        }
        let window_id: u32 = msg_send![window, windowID];
        let frame: NSRect = msg_send![window, frame];
        if frame.size.width < 1.0 || frame.size.height < 1.0 {
            return None;
        }
        let title: Option<Retained<NSString>> = msg_send![window, title];
        let app: Option<Retained<AnyObject>> = msg_send![window, owningApplication];
        let app_name: Option<Retained<NSString>> = app.and_then(|app| msg_send![&*app, applicationName]);
        let name = match (app_name.map(|name| name.to_string()), title.map(|title| title.to_string())) {
            (Some(app), Some(title)) if !title.is_empty() => format!("{app}: {title}"),
            (Some(app), _) => app,
            (None, Some(title)) => title,
            (None, None) => String::new(),
        };
        Some(CaptureSource {
            id: format!("window:{window_id}"),
            kind: CaptureSourceKind::Window,
            name,
            width: pixels_of(frame.size.width, scale),
            height: pixels_of(frame.size.height, scale),
        })
    }
}

fn filter_for_listed(id: &str) -> Result<Retained<AnyObject>, ScreenCaptureError> {
    let content = shareable_content()?;
    let filter_class = class(c"SCContentFilter")?;
    let (kind, raw) = id.split_once(':').ok_or(ScreenCaptureError::SourceGone)?;
    let wanted: u32 = raw.parse().map_err(|_| ScreenCaptureError::SourceGone)?;
    let filter: Option<Retained<AnyObject>> = match kind {
        "display" => {
            let display = displays(&content)
                .into_iter()
                .find(|display| {
                    let display_id: u32 = unsafe { msg_send![&**display, displayID] };
                    display_id == wanted
                })
                .ok_or(ScreenCaptureError::SourceGone)?;
            let none = NSArray::<AnyObject>::new();
            unsafe { msg_send![alloc(filter_class), initWithDisplay: &*display, excludingWindows: &*none] }
        }
        "window" => {
            let window = windows(&content)
                .into_iter()
                .find(|window| {
                    let window_id: u32 = unsafe { msg_send![&**window, windowID] };
                    window_id == wanted
                })
                .ok_or(ScreenCaptureError::SourceGone)?;
            unsafe { msg_send![alloc(filter_class), initWithDesktopIndependentWindow: &*window] }
        }
        _ => return Err(ScreenCaptureError::SourceGone),
    };
    filter.ok_or_else(|| ScreenCaptureError::StartFailed("no content filter".into()))
}

fn alloc(class: &AnyClass) -> Allocated<AnyObject> {
    unsafe { msg_send![class, alloc] }
}

enum PickerAnswer {
    Picked(Shared),
    Cancelled,
    Failed(Failure),
}

#[derive(Default)]
struct PickerState {
    answer: Mutex<Option<PickerAnswer>>,
    arrived: Condvar,
}

impl PickerState {
    fn settle(&self, answer: PickerAnswer) {
        let mut slot = self.answer.lock();
        if slot.is_none() {
            *slot = Some(answer);
            self.arrived.notify_all();
        }
    }
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseScreenSharingPickerObserver"]
    #[ivars = Arc<PickerState>]
    struct PickerObserver;

    impl PickerObserver {
        #[unsafe(method(contentSharingPicker:didCancelForStream:))]
        fn did_cancel(&self, _picker: &AnyObject, _stream: Option<&AnyObject>) {
            guarded("picker cancel", || self.ivars().settle(PickerAnswer::Cancelled));
        }

        #[unsafe(method(contentSharingPicker:didUpdateWithFilter:forStream:))]
        fn did_update(&self, _picker: &AnyObject, filter: &AnyObject, _stream: Option<&AnyObject>) {
            guarded("picker choice", || {
                let filter = unsafe { Retained::retain(std::ptr::from_ref(filter).cast_mut()) };
                match filter {
                    Some(filter) => self.ivars().settle(PickerAnswer::Picked(Shared(filter))),
                    None => self.ivars().settle(PickerAnswer::Cancelled),
                }
            });
        }

        #[unsafe(method(contentSharingPickerStartDidFailWithError:))]
        fn did_fail(&self, error: &NSError) {
            guarded("picker failure", || self.ivars().settle(PickerAnswer::Failed(describe(error))));
        }
    }
);

impl PickerObserver {
    fn new(state: Arc<PickerState>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(state);
        unsafe { msg_send![super(this), init] }
    }
}

static PICKED: Mutex<Vec<(String, Shared)>> = Mutex::new(Vec::new());
const PICKED_KEPT: usize = 4;
static PICKER_LOCK: Mutex<()> = Mutex::new(());

fn add_protocols(class: &AnyClass, names: &[&'static std::ffi::CStr]) {
    for name in names {
        let Some(protocol) = AnyProtocol::get(name) else {
            tracing::warn!(target: "riseonly", protocol = ?name, "screen capture protocol is not registered");
            continue;
        };
        let class: *const AnyClass = class;
        unsafe {
            objc2::ffi::class_addProtocol(class.cast_mut(), protocol);
        }
    }
}

fn declare_conformance() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        add_protocols(PickerObserver::class(), &[c"SCContentSharingPickerObserver"]);
        add_protocols(StreamDelegate::class(), &[c"SCStreamDelegate", c"SCStreamOutput"]);
    });
}

/// work for the main queue, present or dismiss the picker
struct MainWork {
    picker: Shared,
    observer: Option<Retained<PickerObserver>>,
    present: bool,
}

extern "C" fn run_on_main(context: *mut c_void) {
    let work = unsafe { Box::from_raw(context.cast::<MainWork>()) };
    guarded("picker main", || {
        let picker = &*work.picker.0;
        unsafe {
            if work.present {
                if let Some(observer) = &work.observer {
                    let _: () = msg_send![picker, addObserver: &**observer];
                }
                let _: () = msg_send![picker, setActive: Bool::YES];
                let _: () = msg_send![picker, present];
            } else {
                if let Some(observer) = &work.observer {
                    let _: () = msg_send![picker, removeObserver: &**observer];
                }
                let _: () = msg_send![picker, setActive: Bool::NO];
            }
        }
    });
}

fn on_main(work: MainWork) {
    let context = Box::into_raw(Box::new(work)).cast::<c_void>();
    unsafe { dispatch_async_f((&raw const _dispatch_main_q).cast(), context, run_on_main) };
}

fn picked_source(filter: Shared) -> CaptureSource {
    let (rect, scale, style): (NSRect, f32, isize) = unsafe {
        (msg_send![&*filter.0, contentRect], msg_send![&*filter.0, pointPixelScale], msg_send![&*filter.0, style])
    };
    let scale = if scale > 0.0 { f64::from(scale) } else { main_backing_scale() };
    let kind = if style == STYLE_WINDOW || style == STYLE_APPLICATION {
        CaptureSourceKind::Window
    } else {
        CaptureSourceKind::Display
    };
    static NEXT: Mutex<u64> = Mutex::new(1);
    let id = {
        let mut next = NEXT.lock();
        let id = format!("picked:{}", *next);
        *next += 1;
        id
    };
    let mut picked = PICKED.lock();
    if picked.len() >= PICKED_KEPT {
        picked.remove(0);
    }
    picked.push((id.clone(), filter));
    CaptureSource {
        id,
        kind,
        name: String::new(),
        width: pixels_of(rect.size.width, scale),
        height: pixels_of(rect.size.height, scale),
    }
}

fn picked_filter(id: &str) -> Option<Retained<AnyObject>> {
    PICKED.lock().iter().find(|(picked, _)| picked == id).map(|(_, filter)| filter.0.clone())
}

struct Route {
    frames: ScreenFrameSink,
    audio: Option<ScreenAudioSink>,
    ended: ScreenEndSink,
    unread_audio_reported: std::sync::atomic::AtomicBool,
}

impl Route {
    fn deliver(&self, buffer: *mut CmSampleBuffer) {
        match frame_status(buffer) {
            Some(FRAME_STATUS_STOPPED) => {
                (self.ended)(ScreenCaptureEnd::SourceLost);
                return;
            }
            Some(status) if status != FRAME_STATUS_COMPLETE => return,
            _ => {}
        }
        if let Some(frame) = held_frame(buffer) {
            (self.frames)(frame);
        }
    }

    fn deliver_audio(&self, buffer: *mut CmSampleBuffer) {
        let Some(audio) = &self.audio else {
            return;
        };
        match copy_audio(buffer) {
            Some(block) => audio(block),
            None => {
                if !self.unread_audio_reported.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!(target: "riseonly", "screen share sound buffer was not linear float pcm, it is skipped");
                }
            }
        }
    }
}

fn guarded(what: &'static str, f: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
        tracing::error!(target: "riseonly", what, "screen capture callback panicked");
    }
}

fn frame_status(buffer: *mut CmSampleBuffer) -> Option<isize> {
    unsafe {
        let attachments = CMSampleBufferGetSampleAttachmentsArray(buffer, 0);
        let attachments = attachments.as_ref()?;
        let first: Option<Retained<AnyObject>> = msg_send![attachments, firstObject];
        let status: Option<Retained<AnyObject>> = msg_send![&*first?, objectForKey: SCStreamFrameInfoStatus];
        let status = status?.downcast::<NSNumber>().ok()?;
        Some(status.integerValue())
    }
}

fn held_frame(buffer: *mut CmSampleBuffer) -> Option<ScreenFrame> {
    unsafe {
        let pixels = CMSampleBufferGetImageBuffer(buffer);
        if pixels.is_null() || CVPixelBufferGetPixelFormatType(pixels) != PIXEL_FORMAT_420V {
            return None;
        }
        let width = u32::try_from(CVPixelBufferGetWidth(pixels)).ok()?;
        let height = u32::try_from(CVPixelBufferGetHeight(pixels)).ok()?;
        let surface = retain_decoder_surface(pixels).ok()?;
        Some(ScreenFrame {
            width,
            height,
            pixels: ScreenPixels::Surface(surface),
            captured_at: RiseClock::monotonic(),
        })
    }
}

fn copy_audio(buffer: *mut CmSampleBuffer) -> Option<ScreenAudioBlock> {
    let (sample_rate, channels) = unsafe {
        let format = CMSampleBufferGetFormatDescription(buffer);
        if format.is_null() {
            return None;
        }
        let description = CMAudioFormatDescriptionGetStreamBasicDescription(format).as_ref()?;
        if description.format_id != AUDIO_FORMAT_LINEAR_PCM
            || description.format_flags & AUDIO_FORMAT_FLAG_IS_FLOAT == 0
            || description.bits_per_channel != FLOAT_SAMPLE_BITS
        {
            return None;
        }
        (description.sample_rate as u32, u16::try_from(description.channels_per_frame).ok()?)
    };
    let mut size_needed = 0_usize;
    let sized = unsafe {
        CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            buffer,
            &raw mut size_needed,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            std::ptr::null(),
            AUDIO_BUFFER_LIST_ASSURE_ALIGNMENT,
            std::ptr::null_mut(),
        )
    };
    if sized != 0 || size_needed < size_of::<AudioBufferList>() {
        return None;
    }
    let mut storage = vec![0_u64; size_needed.div_ceil(size_of::<u64>())];
    let list = storage.as_mut_ptr().cast::<AudioBufferList>();
    let mut block_buffer: *mut c_void = std::ptr::null_mut();
    let filled = unsafe {
        CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            buffer,
            std::ptr::null_mut(),
            list,
            size_needed,
            std::ptr::null(),
            std::ptr::null(),
            AUDIO_BUFFER_LIST_ASSURE_ALIGNMENT,
            &raw mut block_buffer,
        )
    };
    if filled != 0 {
        return None;
    }
    let samples = unsafe {
        let count = (*list).number_buffers as usize;
        let entries = std::slice::from_raw_parts((&raw const (*list).buffers).cast::<AudioBuffer>(), count);
        let planes: Vec<&[f32]> = entries
            .iter()
            .filter(|entry| !entry.data.is_null())
            .map(|entry| {
                let samples = entry.data_byte_size as usize / size_of::<f32>();
                std::slice::from_raw_parts(entry.data.cast::<f32>().cast_const(), samples)
            })
            .collect();
        let samples = match planes.as_slice() {
            [interleaved] => pcm16_from_float(interleaved),
            planes => pcm16_from_float(&interleaved_from_planes(planes)),
        };
        if !block_buffer.is_null() {
            CFRelease(block_buffer.cast_const());
        }
        samples
    };
    if samples.is_empty() || channels == 0 {
        return None;
    }
    Some(ScreenAudioBlock { samples, sample_rate, channels })
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseScreenCaptureStreamDelegate"]
    #[ivars = Route]
    struct StreamDelegate;

    impl StreamDelegate {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &AnyObject, buffer: *mut CmSampleBuffer, kind: isize) {
            if kind == OUTPUT_TYPE_SCREEN {
                guarded("frame", || self.ivars().deliver(buffer));
            } else if kind == OUTPUT_TYPE_AUDIO {
                guarded("sound", || self.ivars().deliver_audio(buffer));
            }
        }

        #[unsafe(method(stream:didStopWithError:))]
        fn did_stop(&self, _stream: &AnyObject, error: &NSError) {
            guarded("stop", || (self.ivars().ended)(stop_end(describe(error))));
        }
    }
);

impl StreamDelegate {
    fn new(route: Route) -> Retained<Self> {
        let this = Self::alloc().set_ivars(route);
        unsafe { msg_send![super(this), init] }
    }
}

struct StreamObjects {
    stream: Retained<AnyObject>,
    _delegate: Retained<StreamDelegate>,
    _queue: Retained<AnyObject>,
    _audio_queue: Option<Retained<AnyObject>>,
}

unsafe impl Send for StreamObjects {}

struct MacSession {
    source: CaptureSource,
    objects: Option<StreamObjects>,
}

impl ScreenCaptureSession for MacSession {
    fn source(&self) -> &CaptureSource {
        &self.source
    }

    fn stop(&mut self) {
        let Some(objects) = self.objects.take() else {
            return;
        };
        unsafe {
            let _: () = msg_send![&*objects.stream, stopCaptureWithCompletionHandler: std::ptr::null::<ErrorBlock>()];
        }
    }
}

impl Drop for MacSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn configuration(
    source: &CaptureSource,
    options: CaptureOptions,
    with_audio: bool,
) -> Result<Retained<AnyObject>, ScreenCaptureError> {
    let configuration_class = class(c"SCStreamConfiguration")?;
    let configuration: Option<Retained<AnyObject>> = unsafe { msg_send![alloc(configuration_class), init] };
    let configuration = configuration.ok_or_else(|| ScreenCaptureError::StartFailed("no stream configuration".into()))?;
    let (width, height) = options.output_size(source.width, source.height);
    let interval = options.frame_interval();
    let minimum_frame_interval = CmTime {
        value: i64::try_from(interval.as_micros()).unwrap_or(i64::MAX),
        timescale: 1_000_000,
        flags: CMTIME_FLAGS_VALID,
        epoch: 0,
    };
    unsafe {
        let _: () = msg_send![&*configuration, setWidth: width as usize];
        let _: () = msg_send![&*configuration, setHeight: height as usize];
        let _: () = msg_send![&*configuration, setMinimumFrameInterval: minimum_frame_interval];
        let _: () = msg_send![&*configuration, setPixelFormat: PIXEL_FORMAT_420V];
        let _: () = msg_send![&*configuration, setColorMatrix: kCGDisplayStreamYCbCrMatrix_ITU_R_601_4];
        let _: () = msg_send![&*configuration, setShowsCursor: Bool::YES];
        let _: () = msg_send![&*configuration, setQueueDepth: QUEUE_DEPTH];
    }
    if with_audio {
        unsafe {
            let _: () = msg_send![&*configuration, setCapturesAudio: Bool::YES];
            let _: () = msg_send![&*configuration, setExcludesCurrentProcessAudio: Bool::YES];
            let _: () = msg_send![&*configuration, setSampleRate: SYSTEM_AUDIO_SAMPLE_RATE as isize];
            let _: () = msg_send![&*configuration, setChannelCount: SYSTEM_AUDIO_CHANNEL_COUNT as isize];
        }
    }
    Ok(configuration)
}

fn sample_queue(label: &'static std::ffi::CStr) -> Result<Retained<AnyObject>, ScreenCaptureError> {
    unsafe { Retained::from_raw(dispatch_queue_create(label.as_ptr(), std::ptr::null_mut())) }
        .ok_or_else(|| ScreenCaptureError::StartFailed("no capture queue".into()))
}

fn add_stream_output(
    stream: &AnyObject,
    output: &AnyObject,
    kind: isize,
    queue: &AnyObject,
) -> Result<(), Failure> {
    let mut error: *mut NSError = std::ptr::null_mut();
    let add_output = Sel::register(c"addStreamOutput:type:sampleHandlerQueue:error:");
    let added: Bool = unsafe { stream.send_message(add_output, (output, kind, queue, &raw mut error)) };
    if added.as_bool() {
        return Ok(());
    }
    Err(unsafe { error.as_ref() }.map(describe).unwrap_or((0, "no stream output".into())))
}

pub(crate) struct NativeScreenCapture;

impl ScreenCaptureBackend for NativeScreenCapture {
    fn support(&self) -> Result<(), ScreenCaptureError> {
        class(c"SCStream").map(|_| ())
    }

    fn access(&self) -> ScreenCaptureAccess {
        if unsafe { CGPreflightScreenCaptureAccess() } {
            ScreenCaptureAccess::Granted
        } else {
            ScreenCaptureAccess::Denied
        }
    }

    fn request_access(&self) -> ScreenCaptureAccess {
        if unsafe { CGRequestScreenCaptureAccess() } {
            ScreenCaptureAccess::Granted
        } else {
            ScreenCaptureAccess::Denied
        }
    }

    fn pick_source(&self, _parent: &PickerParent) -> Result<CaptureSource, ScreenCaptureError> {
        self.support()?;
        let Some(picker_class) = AnyClass::get(c"SCContentSharingPicker") else {
            return Err(ScreenCaptureError::PickerUnavailable);
        };
        if MainThreadMarker::new().is_some() {
            return Err(ScreenCaptureError::MainThread);
        }
        let _one_picker = PICKER_LOCK.lock();
        declare_conformance();
        let picker: Option<Retained<AnyObject>> = unsafe { msg_send![picker_class, sharedPicker] };
        let picker = picker.ok_or(ScreenCaptureError::PickerUnavailable)?;
        let state = Arc::new(PickerState::default());
        let observer = PickerObserver::new(state.clone());
        on_main(MainWork { picker: Shared(picker.clone()), observer: Some(observer.clone()), present: true });
        let answer = {
            let deadline = RiseClock::monotonic() + PICKER_WINDOW;
            let mut slot = state.answer.lock();
            while slot.is_none() {
                let remaining = deadline.saturating_sub(RiseClock::monotonic());
                if remaining.is_zero() || state.arrived.wait_for(&mut slot, remaining).timed_out() {
                    break;
                }
            }
            slot.take()
        };
        on_main(MainWork { picker: Shared(picker), observer: Some(observer), present: false });
        match answer {
            Some(PickerAnswer::Picked(filter)) => Ok(picked_source(filter)),
            Some(PickerAnswer::Cancelled) => Err(ScreenCaptureError::Cancelled),
            Some(PickerAnswer::Failed(failure)) => Err(start_failure(failure)),
            None => Err(ScreenCaptureError::PickerTimedOut),
        }
    }

    fn sources(&self) -> Result<Vec<CaptureSource>, ScreenCaptureError> {
        self.support()?;
        let content = shareable_content()?;
        let scale = main_backing_scale();
        let mut sources: Vec<CaptureSource> =
            displays(&content).iter().map(|display| display_source(display)).collect();
        sources.extend(windows(&content).iter().filter_map(|window| window_source(window, scale)));
        Ok(sources)
    }

    fn start(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        self.start_stream(source, options, frames, None, ended)
    }

    fn system_audio_support(&self) -> Result<(), ScreenCaptureError> {
        let configuration_class = class(c"SCStreamConfiguration")?;
        if configuration_class.responds_to(Sel::register(c"setExcludesCurrentProcessAudio:")) {
            Ok(())
        } else {
            Err(ScreenCaptureError::Unsupported("sharing the system sound needs macOS 13".into()))
        }
    }

    fn start_with_system_audio(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        audio: ScreenAudioSink,
        ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        self.start_stream(source, options, frames, Some(audio), ended)
    }
}

impl NativeScreenCapture {
    fn start_stream(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        audio: Option<ScreenAudioSink>,
        ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        self.support()?;
        declare_conformance();
        let filter = match picked_filter(&source.id) {
            Some(filter) => filter,
            None => {
                if self.access() != ScreenCaptureAccess::Granted {
                    return Err(ScreenCaptureError::AccessDenied);
                }
                filter_for_listed(&source.id)?
            }
        };
        let with_audio = audio.is_some();
        let configuration = configuration(source, options, with_audio)?;
        let delegate = StreamDelegate::new(Route { frames, audio, ended, unread_audio_reported: Default::default() });
        let queue = sample_queue(c"rise.screen.capture")?;
        let stream_class = class(c"SCStream")?;
        let stream: Option<Retained<AnyObject>> = unsafe {
            msg_send![
                alloc(stream_class),
                initWithFilter: &*filter,
                configuration: &*configuration,
                delegate: &*delegate
            ]
        };
        let stream = stream.ok_or_else(|| ScreenCaptureError::StartFailed("no stream".into()))?;
        let output: &AnyObject = &delegate;
        add_stream_output(&stream, output, OUTPUT_TYPE_SCREEN, &queue).map_err(start_failure)?;
        let audio_queue = if with_audio {
            match sample_queue(c"rise.screen.capture.audio").and_then(|audio_queue| {
                add_stream_output(&stream, output, OUTPUT_TYPE_AUDIO, &audio_queue)
                    .map_err(start_failure)
                    .map(|()| audio_queue)
            }) {
                Ok(audio_queue) => Some(audio_queue),
                Err(error) => {
                    tracing::error!(target: "riseonly", %error, "screen share goes on without sound");
                    None
                }
            }
        } else {
            None
        };
        let request = next_request();
        let block = ErrorBlock {
            isa: (&raw const _NSConcreteStackBlock).cast(),
            flags: 0,
            reserved: 0,
            invoke: error_answered,
            descriptor: &raw const ERROR_DESCRIPTOR,
            request,
        };
        unsafe {
            let _: () = msg_send![&*stream, startCaptureWithCompletionHandler: &raw const block];
        }
        let session = MacSession {
            source: source.clone(),
            objects: Some(StreamObjects { stream, _delegate: delegate, _queue: queue, _audio_queue: audio_queue }),
        };
        match wait_reply(request, REPLY_TIMEOUT) {
            Some(Reply::Done(None)) => Ok(Box::new(session)),
            Some(Reply::Done(Some(failure))) => Err(start_failure(failure)),
            Some(Reply::Content(_)) | None => {
                Err(ScreenCaptureError::StartFailed("ScreenCaptureKit did not confirm the start".into()))
            }
        }
    }
}
