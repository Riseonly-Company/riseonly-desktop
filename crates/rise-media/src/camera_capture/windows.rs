use std::ffi::c_void;
use std::io::ErrorKind;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;

use ::windows::Devices::Enumeration::DeviceInformation;
use ::windows::Win32::Foundation::{E_ACCESSDENIED, E_POINTER, RPC_E_CHANGED_MODE};
use ::windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFActivate, IMFAttributes, IMFMediaBuffer, IMFMediaSource, IMFMediaType, IMFMediaTypeHandler,
    IMFSample, IMFSinkWriter, IMFSourceReader, IMFStreamDescriptor, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_E_HW_MFT_FAILED_START_STREAMING, MF_E_INVALIDMEDIATYPE,
    MF_E_TOPO_CODEC_NOT_FOUND, MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED, MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED,
    MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR,
    MF_TRANSCODE_CONTAINERTYPE, MF_VERSION, MFMediaType_Video, MFSTARTUP_LITE, MFTranscodeContainerType_MPEG4,
    MFVideoFormat_H264, MFVideoFormat_MJPG, MFVideoFormat_NV12, MFVideoFormat_YUY2, MFVideoInterlace_Progressive,
};
use ::windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize};
use ::windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use ::windows::core::{GUID, HRESULT, HSTRING, Interface, PCSTR, PCWSTR, PWSTR, s, w};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;

use super::camera_capture::*;
use super::media_foundation_video::*;
use crate::texture::frame::{FrameFormat, FrameGeometry};

pub(super) struct NativeCamera;

type MfStartupFn = unsafe extern "system" fn(u32, u32) -> HRESULT;
type MfShutdownFn = unsafe extern "system" fn() -> HRESULT;
type MfCreateAttributesFn = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type MfCreateMediaTypeFn = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type MfEnumDeviceSourcesFn = unsafe extern "system" fn(*mut c_void, *mut *mut *mut c_void, *mut u32) -> HRESULT;
type MfCreateDeviceSourceFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
type MfCreateSourceReaderFromMediaSourceFn =
    unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;
type MfCreateSinkWriterFromUrlFn =
    unsafe extern "system" fn(PCWSTR, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;

struct MediaFoundation {
    startup: MfStartupFn,
    shutdown: MfShutdownFn,
    create_attributes: MfCreateAttributesFn,
    create_media_type: MfCreateMediaTypeFn,
    enum_device_sources: MfEnumDeviceSourcesFn,
    create_device_source: MfCreateDeviceSourceFn,
    create_source_reader_from_media_source: MfCreateSourceReaderFromMediaSourceFn,
    create_sink_writer_from_url: MfCreateSinkWriterFromUrlFn,
}

fn media_foundation() -> Result<&'static MediaFoundation, CameraCaptureError> {
    static RESOLVED: OnceLock<Result<MediaFoundation, String>> = OnceLock::new();
    RESOLVED
        .get_or_init(resolve_media_foundation)
        .as_ref()
        .map_err(|reason| CameraCaptureError::Unavailable(reason.clone()))
}

fn resolve_media_foundation() -> Result<MediaFoundation, String> {
    let missing = |library: &str| {
        format!("{library} is not installed (a Windows N edition without the Media Feature Pack)")
    };
    let platform = unsafe { LoadLibraryExW(w!("mfplat.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(|_| missing("mfplat.dll"))?;
    let devices = unsafe { LoadLibraryExW(w!("mf.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(|_| missing("mf.dll"))?;
    let read_write = unsafe { LoadLibraryExW(w!("mfreadwrite.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(|_| missing("mfreadwrite.dll"))?;
    let export = |module, name: PCSTR, label: &str| {
        unsafe { GetProcAddress(module, name) }.ok_or_else(|| format!("Media Foundation exports no {label}"))
    };
    let startup = export(platform, s!("MFStartup"), "MFStartup")?;
    let shutdown = export(platform, s!("MFShutdown"), "MFShutdown")?;
    let create_attributes = export(platform, s!("MFCreateAttributes"), "MFCreateAttributes")?;
    let create_media_type = export(platform, s!("MFCreateMediaType"), "MFCreateMediaType")?;
    let enum_device_sources = export(devices, s!("MFEnumDeviceSources"), "MFEnumDeviceSources")?;
    let create_device_source = export(devices, s!("MFCreateDeviceSource"), "MFCreateDeviceSource")?;
    let create_source_reader_from_media_source = export(
        read_write,
        s!("MFCreateSourceReaderFromMediaSource"),
        "MFCreateSourceReaderFromMediaSource",
    )?;
    let create_sink_writer_from_url =
        export(read_write, s!("MFCreateSinkWriterFromURL"), "MFCreateSinkWriterFromURL")?;
    type Exported = unsafe extern "system" fn() -> isize;
    unsafe {
        Ok(MediaFoundation {
            startup: std::mem::transmute::<Exported, MfStartupFn>(startup),
            shutdown: std::mem::transmute::<Exported, MfShutdownFn>(shutdown),
            create_attributes: std::mem::transmute::<Exported, MfCreateAttributesFn>(create_attributes),
            create_media_type: std::mem::transmute::<Exported, MfCreateMediaTypeFn>(create_media_type),
            enum_device_sources: std::mem::transmute::<Exported, MfEnumDeviceSourcesFn>(enum_device_sources),
            create_device_source: std::mem::transmute::<Exported, MfCreateDeviceSourceFn>(create_device_source),
            create_source_reader_from_media_source: std::mem::transmute::<
                Exported,
                MfCreateSourceReaderFromMediaSourceFn,
            >(create_source_reader_from_media_source),
            create_sink_writer_from_url: std::mem::transmute::<Exported, MfCreateSinkWriterFromUrlFn>(
                create_sink_writer_from_url,
            ),
        })
    }
}

/// wraps the interface a creation call wrote into its out pointer
fn created<T: Interface>(
    call: &str,
    make: impl FnOnce(*mut *mut c_void) -> HRESULT,
) -> Result<T, ::windows::core::Error> {
    let mut raw: *mut c_void = ptr::null_mut();
    make(&mut raw).ok()?;
    if raw.is_null() {
        tracing::debug!(call, "Media Foundation returned no object");
        return Err(E_POINTER.into());
    }
    Ok(unsafe { T::from_raw(raw) })
}

/// maps a failed call while opening or reading the camera to a screen error
fn camera_error(call: &str, error: ::windows::core::Error) -> CameraCaptureError {
    let code = error.code();
    if code == E_ACCESSDENIED {
        CameraCaptureError::AccessDenied
    } else if code == MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED || code == MF_E_HW_MFT_FAILED_START_STREAMING {
        CameraCaptureError::Busy
    } else if code == MF_E_VIDEO_RECORDING_DEVICE_INVALIDATED {
        CameraCaptureError::NoCamera
    } else {
        CameraCaptureError::OpenFailed(format!("{call} failed: {error}"))
    }
}

/// maps a failed call while writing a segment to an error
fn writer_error(call: &str, error: ::windows::core::Error) -> CameraCaptureError {
    if error.code() == MF_E_TOPO_CODEC_NOT_FOUND {
        CameraCaptureError::EncoderUnavailable(format!("no Media Foundation H.264 encoder is registered ({call}: {error})"))
    } else {
        CameraCaptureError::FileFailed(format!("{call} failed: {error}"))
    }
}

fn opening(call: &'static str) -> impl Fn(::windows::core::Error) -> CameraCaptureError {
    move |error| camera_error(call, error)
}

fn writing(call: &'static str) -> impl Fn(::windows::core::Error) -> CameraCaptureError {
    move |error| writer_error(call, error)
}

/// com and media foundation started on this thread, dropped after every interface
struct MediaFoundationSession {
    api: &'static MediaFoundation,
    com_joined: bool,
    started: bool,
}

impl MediaFoundationSession {
    fn start(api: &'static MediaFoundation) -> Result<Self, CameraCaptureError> {
        let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if joined.is_err() && joined != RPC_E_CHANGED_MODE {
            return Err(CameraCaptureError::Unavailable(format!("CoInitializeEx failed: {joined:?}")));
        }
        let mut session = Self { api, com_joined: joined.is_ok(), started: false };
        let started = unsafe { (api.startup)(MF_VERSION, MFSTARTUP_LITE) };
        started
            .ok()
            .map_err(|error| CameraCaptureError::Unavailable(format!("MFStartup failed: {error}")))?;
        session.started = true;
        Ok(session)
    }
}

impl Drop for MediaFoundationSession {
    fn drop(&mut self) {
        if self.started {
            let stopped = unsafe { (self.api.shutdown)() };
            if stopped.is_err() {
                tracing::debug!(?stopped, "MFShutdown");
            }
        }
        if self.com_joined {
            unsafe { CoUninitialize() };
        }
    }
}

fn new_attributes(api: &'static MediaFoundation, slots: u32) -> Result<IMFAttributes, ::windows::core::Error> {
    created("MFCreateAttributes", |out| {
        unsafe { (api.create_attributes)(out, slots) }
    })
}

/// new video media type with the major type set, fill does the rest
fn video_type(
    api: &'static MediaFoundation,
    fill: impl FnOnce(&IMFMediaType) -> ::windows::core::Result<()>,
) -> Result<IMFMediaType, ::windows::core::Error> {
    let media_type: IMFMediaType = created("MFCreateMediaType", |out| {
        unsafe { (api.create_media_type)(out) }
    })?;
    unsafe { media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }?;
    fill(&media_type)?;
    Ok(media_type)
}

/// string attribute that media foundation allocated, copied out and freed
fn allocated_string(attributes: &IMFAttributes, key: &GUID) -> Option<String> {
    let mut value = PWSTR::null();
    let mut length = 0u32;
    unsafe { attributes.GetAllocatedString(key, &mut value, &mut length) }.ok()?;
    if value.is_null() {
        return None;
    }
    let text = unsafe { value.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(value.0 as *const c_void)) };
    text
}

/// enclosure location of a device interface if the system knows it
fn enclosure(symbolic_link: &str) -> Option<(EnclosurePanel, bool)> {
    let info = DeviceInformation::CreateFromIdAsync(&HSTRING::from(symbolic_link)).ok()?.join().ok()?;
    let location = info.EnclosureLocation().ok()?;
    let panel = location.Panel().ok()?;
    Some((EnclosurePanel::from_raw(panel.0), location.InDock().unwrap_or(false)))
}

fn enumerate() -> Result<Vec<CameraDevice>, CameraCaptureError> {
    let api = media_foundation()?;
    let _session = MediaFoundationSession::start(api)?;
    let listing = |error: ::windows::core::Error| CameraCaptureError::Unavailable(format!("MFEnumDeviceSources failed: {error}"));
    let attributes = new_attributes(api, 1).map_err(listing)?;
    unsafe { attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID) }
        .map_err(listing)?;
    let mut array: *mut *mut c_void = ptr::null_mut();
    let mut count = 0u32;
    unsafe { (api.enum_device_sources)(attributes.as_raw(), &mut array, &mut count) }
        .ok()
        .map_err(listing)?;
    let mut activates: Vec<IMFActivate> = Vec::with_capacity(count as usize);
    if !array.is_null() {
        for index in 0..count as usize {
            let raw = unsafe { *array.add(index) };
            if !raw.is_null() {
                activates.push(unsafe { IMFActivate::from_raw(raw) });
            }
        }
        unsafe { CoTaskMemFree(Some(array as *const c_void)) };
    }
    let mut devices = Vec::with_capacity(activates.len());
    for activate in &activates {
        let Some(id) = allocated_string(activate, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK) else {
            continue;
        };
        let name = allocated_string(activate, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME).unwrap_or_else(|| id.clone());
        let (position, built_in) = placement(enclosure(&id));
        devices.push(CameraDevice { id, name, position, built_in });
    }
    drop(activates);
    Ok(devices)
}

impl CameraSource for NativeCamera {
    fn support(&self) -> Result<(), CameraCaptureError> {
        if enumerate()?.is_empty() { Err(CameraCaptureError::NoCamera) } else { Ok(()) }
    }

    fn recording_support(&self) -> Result<(), CameraCaptureError> {
        media_foundation().map(|_| ())
    }

    fn devices(&self) -> Result<Vec<CameraDevice>, CameraCaptureError> {
        enumerate()
    }

    fn open(&self, device: &CameraDevice, frames: FrameSink) -> Result<Box<dyn CameraSession>, CameraCaptureError> {
        Ok(Box::new(WindowsCameraSession::start(device.clone(), frames)?))
    }
}

/// request from the session to the capture thread, answered on its own channel
enum Command {
    Start(PathBuf, SyncSender<Result<(), CameraCaptureError>>),
    Stop(SyncSender<Result<(), CameraCaptureError>>),
}

struct WindowsCameraSession {
    device: CameraDevice,
    mode: CameraMode,
    commands: Sender<Command>,
    stop: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<CameraCaptureError>>>,
    thread: Option<JoinHandle<()>>,
}

impl WindowsCameraSession {
    fn start(device: CameraDevice, frames: FrameSink) -> Result<Self, CameraCaptureError> {
        let api = media_foundation()?;
        let (commands, inbox) = mpsc::channel();
        let (opened, opened_answer) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        let mirrored = device.position == CameraPosition::Front || device.built_in;
        let worker_device = device.clone();
        let (worker_stop, worker_failure) = (stop.clone(), failure.clone());
        let wiring = Wiring { frames, mirrored, opened, inbox, stop: worker_stop, failure: worker_failure };
        let thread = std::thread::Builder::new()
            .name("camera-capture".to_owned())
            .spawn(move || run(api, worker_device, wiring))
            .map_err(|error| CameraCaptureError::OpenFailed(error.to_string()))?;
        let mode = match opened_answer.recv() {
            Ok(Ok(mode)) => mode,
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(CameraCaptureError::OpenFailed("the capture thread ended while opening".to_owned()));
            }
        };
        Ok(Self { device, mode, commands, stop, failure, thread: Some(thread) })
    }

    fn ask(&self, make: impl FnOnce(SyncSender<Result<(), CameraCaptureError>>) -> Command) -> Result<(), CameraCaptureError> {
        let (reply, answer) = mpsc::sync_channel(1);
        let gone = || self.failure.lock().clone().unwrap_or_else(|| CameraCaptureError::StreamFailed("the capture thread ended".to_owned()));
        self.commands.send(make(reply)).map_err(|_| gone())?;
        answer.recv().map_err(|_| gone())?
    }
}

impl CameraSession for WindowsCameraSession {
    fn device(&self) -> &CameraDevice {
        &self.device
    }

    fn mode(&self) -> CameraMode {
        self.mode
    }

    fn start_segment(&mut self, path: &Path) -> Result<(), CameraCaptureError> {
        let path = path.to_path_buf();
        self.ask(|reply| Command::Start(path, reply))
    }

    fn stop_segment(&mut self) -> Result<(), CameraCaptureError> {
        self.ask(Command::Stop)
    }

    fn failure(&self) -> Option<CameraCaptureError> {
        self.failure.lock().clone()
    }
}

impl Drop for WindowsCameraSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// what the capture thread gets from the session
struct Wiring {
    frames: FrameSink,
    mirrored: bool,
    opened: SyncSender<Result<CameraMode, CameraCaptureError>>,
    inbox: Receiver<Command>,
    stop: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<CameraCaptureError>>>,
}

/// capture thread, opens the camera and reports the mode then reads frames until told to stop
fn run(api: &'static MediaFoundation, device: CameraDevice, wiring: Wiring) {
    let Wiring { frames, mirrored, opened, inbox, stop, failure } = wiring;
    let session = match MediaFoundationSession::start(api) {
        Ok(session) => session,
        Err(error) => {
            let _ = opened.send(Err(error));
            return;
        }
    };
    let mut worker = match Capture::open(api, &device) {
        Ok(capture) => Worker {
            api,
            capture,
            frames,
            mirrored,
            inbox,
            stop,
            segment: None,
            pending_stop: None,
        },
        Err(error) => {
            let _ = opened.send(Err(error));
            drop(session);
            return;
        }
    };
    match worker.read_one() {
        Ok(()) => {
            let _ = opened.send(Ok(worker.capture.mode));
        }
        Err(error) => {
            let _ = opened.send(Err(error));
            drop(worker);
            drop(session);
            return;
        }
    }
    let result = worker.pump();
    let closed = worker.close_segment();
    if let Some((reply, _)) = worker.pending_stop.take() {
        let _ = reply.send(closed);
    }
    if let Err(error) = result {
        tracing::warn!(%error, "camera capture stopped");
        *failure.lock() = Some(error);
    }
    drop(worker);
    drop(session);
}

/// open camera, its reader and the geometry of the nv12 frames it gives out
struct Capture {
    reader: IMFSourceReader,
    mode: CameraMode,
    width: u32,
    height: u32,
    stride: u32,
    frame_hns: i64,
    _source: IMFMediaSource,
}

fn native_encoding(subtype: &GUID) -> Option<PixelEncoding> {
    if *subtype == MFVideoFormat_YUY2 {
        Some(PixelEncoding::Yuyv)
    } else if *subtype == MFVideoFormat_NV12 {
        Some(PixelEncoding::Nv12)
    } else if *subtype == MFVideoFormat_MJPG {
        Some(PixelEncoding::Mjpeg)
    } else {
        None
    }
}

fn video_handler(source: &IMFMediaSource) -> Result<IMFMediaTypeHandler, CameraCaptureError> {
    let presentation = unsafe { source.CreatePresentationDescriptor() }
        .map_err(|error| camera_error("IMFMediaSource::CreatePresentationDescriptor", error))?;
    let count = unsafe { presentation.GetStreamDescriptorCount() }
        .map_err(|error| camera_error("IMFPresentationDescriptor::GetStreamDescriptorCount", error))?;
    for index in 0..count {
        let mut selected = ::windows::core::BOOL::default();
        let mut stream: Option<IMFStreamDescriptor> = None;
        if unsafe { presentation.GetStreamDescriptorByIndex(index, &mut selected, &mut stream) }.is_err() {
            continue;
        }
        let Some(stream) = stream else { continue };
        let Ok(handler) = (unsafe { stream.GetMediaTypeHandler() }) else { continue };
        if unsafe { handler.GetMajorType() }.is_ok_and(|major| major == MFMediaType_Video) {
            return Ok(handler);
        }
    }
    Err(CameraCaptureError::OpenFailed("the camera offers no video stream".to_owned()))
}

impl Capture {
    fn open(api: &'static MediaFoundation, device: &CameraDevice) -> Result<Self, CameraCaptureError> {
        let attributes = new_attributes(api, 2).map_err(opening("MFCreateAttributes"))?;
        let link: Vec<u16> = device.id.encode_utf16().chain(iter::once(0)).collect();
        unsafe {
            attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)
        }
        .map_err(opening("IMFAttributes::SetGUID"))?;
        unsafe { attributes.SetString(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, PCWSTR(link.as_ptr())) }
            .map_err(opening("IMFAttributes::SetString"))?;
        let source: IMFMediaSource = created("MFCreateDeviceSource", |out| {
            unsafe { (api.create_device_source)(attributes.as_raw(), out) }
        })
        .map_err(opening("MFCreateDeviceSource"))?;

        let handler = video_handler(&source)?;
        let count = unsafe { handler.GetMediaTypeCount() }.map_err(opening("IMFMediaTypeHandler::GetMediaTypeCount"))?;
        let mut natives: Vec<(CameraMode, IMFMediaType)> = Vec::new();
        for index in 0..count {
            let Ok(native) = (unsafe { handler.GetMediaTypeByIndex(index) }) else { continue };
            let described = unsafe {
                (native.GetGUID(&MF_MT_SUBTYPE), native.GetUINT64(&MF_MT_FRAME_SIZE), native.GetUINT64(&MF_MT_FRAME_RATE))
            };
            let (Ok(subtype), Ok(size), Ok(rate)) = described else { continue };
            let Some(encoding) = native_encoding(&subtype) else { continue };
            if let Some(mode) = mode_from_native(encoding, size, rate) {
                natives.push((mode, native));
            }
        }
        let modes: Vec<CameraMode> = natives.iter().map(|(mode, _)| *mode).collect();
        let mode = choose_mode(&modes)
            .ok_or_else(|| CameraCaptureError::OpenFailed("the camera offers no YUY2, NV12 or MJPG mode".to_owned()))?;
        let native = natives
            .iter()
            .find(|(candidate, _)| *candidate == mode)
            .map(|(_, native)| native)
            .ok_or_else(|| CameraCaptureError::OpenFailed("the chosen mode vanished".to_owned()))?;
        unsafe { handler.SetCurrentMediaType(native) }.map_err(opening("IMFMediaTypeHandler::SetCurrentMediaType"))?;

        let reader_attributes = new_attributes(api, 1).map_err(opening("MFCreateAttributes"))?;
        unsafe { reader_attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1) }
            .map_err(opening("IMFAttributes::SetUINT32"))?;
        let reader: IMFSourceReader = created("MFCreateSourceReaderFromMediaSource", |out| {
            unsafe { (api.create_source_reader_from_media_source)(source.as_raw(), reader_attributes.as_raw(), out) }
        })
        .map_err(opening("MFCreateSourceReaderFromMediaSource"))?;
        let all = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        unsafe {
            reader.SetStreamSelection(all, false).map_err(opening("IMFSourceReader::SetStreamSelection"))?;
            reader.SetStreamSelection(video, true).map_err(opening("IMFSourceReader::SetStreamSelection"))?;
        }
        let output = video_type(api, |media_type| {
            unsafe {
                media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                media_type.SetUINT64(&MF_MT_FRAME_SIZE, packed_frame_size(&mode))?;
                media_type.SetUINT64(&MF_MT_FRAME_RATE, packed_frame_rate(&mode))
            }
        })
        .map_err(opening("MFCreateMediaType"))?;
        unsafe { reader.SetCurrentMediaType(video, None, &output) }.map_err(|error| {
            if error.code() == MF_E_INVALIDMEDIATYPE {
                CameraCaptureError::OpenFailed(format!("the camera format cannot become NV12: {error}"))
            } else {
                camera_error("IMFSourceReader::SetCurrentMediaType", error)
            }
        })?;
        let current = unsafe { reader.GetCurrentMediaType(video) }.map_err(opening("IMFSourceReader::GetCurrentMediaType"))?;
        let (width, height) = unsafe { current.GetUINT64(&MF_MT_FRAME_SIZE) }
            .map(unpack_pair)
            .unwrap_or((mode.width, mode.height));
        let stride = unsafe { current.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
            .map(|raw| (raw as i32).unsigned_abs())
            .ok()
            .filter(|stride| *stride >= width)
            .unwrap_or(width);
        Ok(Self {
            reader,
            mode,
            width,
            height,
            stride,
            frame_hns: frame_duration_hns(&mode),
            _source: source,
        })
    }

    /// nv12 picture of a sample, copied out for the preview
    fn frame(&self, sample: &IMFSample) -> Result<(FrameGeometry, Vec<u8>), CameraCaptureError> {
        let buffer: IMFMediaBuffer =
            unsafe { sample.ConvertToContiguousBuffer() }.map_err(|_| CameraCaptureError::BadFrame("NV12 buffer"))?;
        let (pixels, stride) = if let Ok(planes) = buffer.cast::<IMF2DBuffer>() {
            let length = unsafe { planes.GetContiguousLength() }.map_err(|_| CameraCaptureError::BadFrame("NV12 length"))?;
            let mut pixels = vec![0u8; length as usize];
            unsafe { planes.ContiguousCopyTo(&mut pixels) }.map_err(|_| CameraCaptureError::BadFrame("NV12 copy"))?;
            (pixels, self.width)
        } else {
            let mut data: *mut u8 = ptr::null_mut();
            let mut current = 0u32;
            unsafe { buffer.Lock(&mut data, None, Some(&raw mut current)) }
                .map_err(|_| CameraCaptureError::BadFrame("NV12 lock"))?;
            let pixels = if data.is_null() {
                Vec::new()
            } else {
                unsafe { std::slice::from_raw_parts(data, current as usize) }.to_vec()
            };
            let _ = unsafe { buffer.Unlock() };
            (pixels, self.stride)
        };
        let geometry = FrameGeometry::with_strides(FrameFormat::Nv12, self.width, self.height, &[stride, stride])
            .map_err(|_| CameraCaptureError::BadFrame("NV12 geometry"))?;
        if (pixels.len() as u64) < geometry.byte_len() {
            return Err(CameraCaptureError::BadFrame("NV12 frame is shorter than its geometry"));
        }
        Ok((geometry, pixels))
    }
}

/// one open mp4 segment, the writer is released before the capture thread session
struct SegmentWriter {
    writer: Option<IMFSinkWriter>,
    stream: u32,
    clock: SegmentClock,
}

impl SegmentWriter {
    fn create(api: &'static MediaFoundation, path: &Path, capture: &Capture) -> Result<Self, CameraCaptureError> {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(CameraCaptureError::FileFailed(format!("{}: {error}", path.display()))),
        }
        let mode = CameraMode { width: capture.width, height: capture.height, ..capture.mode };
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(iter::once(0)).collect();
        let attributes = new_attributes(api, 2).map_err(writing("MFCreateAttributes"))?;
        unsafe {
            attributes
                .SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4)
                .map_err(writing("MF_TRANSCODE_CONTAINERTYPE"))?;
            attributes
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                .map_err(writing("MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS"))?;
        }
        let writer: IMFSinkWriter = created("MFCreateSinkWriterFromURL", |out| {
            unsafe { (api.create_sink_writer_from_url)(PCWSTR(wide.as_ptr()), ptr::null_mut(), attributes.as_raw(), out) }
        })
        .map_err(|error| CameraCaptureError::FileFailed(format!("{}: {error}", path.display())))?;
        let picture = |media_type: &IMFMediaType| -> ::windows::core::Result<()> {
            unsafe {
                media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                media_type.SetUINT64(&MF_MT_FRAME_SIZE, packed_frame_size(&mode))?;
                media_type.SetUINT64(&MF_MT_FRAME_RATE, packed_frame_rate(&mode))?;
                media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_pair(1, 1))
            }
        };
        let output = video_type(api, |media_type| {
            unsafe {
                media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                media_type.SetUINT32(&MF_MT_AVG_BITRATE, h264_bitrate(&mode))?;
            }
            picture(media_type)
        })
        .map_err(writing("H.264 output type"))?;
        let stream = unsafe { writer.AddStream(&output) }.map_err(writing("IMFSinkWriter::AddStream"))?;
        let input = video_type(api, |media_type| {
            unsafe { media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)? };
            picture(media_type)
        })
        .map_err(writing("NV12 input type"))?;
        unsafe { writer.SetInputMediaType(stream, &input, None::<&IMFAttributes>) }
            .map_err(writing("IMFSinkWriter::SetInputMediaType"))?;
        unsafe { writer.BeginWriting() }.map_err(writing("IMFSinkWriter::BeginWriting"))?;
        Ok(Self { writer: Some(writer), stream, clock: SegmentClock::new(capture.frame_hns) })
    }

    fn write(&mut self, sample: &IMFSample, timestamp: i64) -> Result<(), CameraCaptureError> {
        let Some(writer) = self.writer.as_ref() else {
            return Err(CameraCaptureError::NotRecording);
        };
        let duration = unsafe { sample.GetSampleDuration() }.unwrap_or(0);
        let (time, duration) = self.clock.place(timestamp, duration);
        unsafe {
            sample.SetSampleTime(time).map_err(|error| writer_error("IMFSample::SetSampleTime", error))?;
            sample.SetSampleDuration(duration).map_err(|error| writer_error("IMFSample::SetSampleDuration", error))?;
        }
        unsafe { writer.WriteSample(self.stream, sample) }.map_err(|error| writer_error("IMFSinkWriter::WriteSample", error))
    }

    fn finish(&mut self) -> Result<(), CameraCaptureError> {
        let Some(writer) = self.writer.take() else {
            return Ok(());
        };
        if !self.clock.has_frames() {
            return Err(CameraCaptureError::FileFailed("no frame reached the segment".to_owned()));
        }
        unsafe { writer.Finalize() }.map_err(|error| writer_error("IMFSinkWriter::Finalize", error))
    }
}

impl Drop for SegmentWriter {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            tracing::warn!(%error, "the camera segment did not close cleanly");
        }
    }
}

struct Worker {
    api: &'static MediaFoundation,
    capture: Capture,
    frames: FrameSink,
    mirrored: bool,
    inbox: Receiver<Command>,
    stop: Arc<AtomicBool>,
    segment: Option<SegmentWriter>,
    pending_stop: Option<(SyncSender<Result<(), CameraCaptureError>>, i64)>,
}

impl Worker {
    fn pump(&mut self) -> Result<(), CameraCaptureError> {
        while !self.stop.load(Ordering::Acquire) {
            self.serve_commands();
            self.read_one()?;
        }
        Ok(())
    }

    fn serve_commands(&mut self) {
        loop {
            match self.inbox.try_recv() {
                Ok(Command::Start(path, reply)) => {
                    let answer = if self.segment.is_some() {
                        Err(CameraCaptureError::AlreadyRecording)
                    } else {
                        SegmentWriter::create(self.api, &path, &self.capture).map(|writer| {
                            self.segment = Some(writer);
                        })
                    };
                    let _ = reply.send(answer);
                }
                Ok(Command::Stop(reply)) => match self.segment.as_ref() {
                    None => {
                        let _ = reply.send(Err(CameraCaptureError::NotRecording));
                    }
                    Some(segment) if !segment.clock.has_frames() => {
                        let deadline = RiseClock::monotonic().as_nanos() as i64 / 100 + EMPTY_SEGMENT_GRACE_HNS;
                        self.pending_stop = Some((reply, deadline));
                    }
                    Some(_) => {
                        let answer = self.close_segment();
                        let _ = reply.send(answer);
                    }
                },
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return,
            }
        }
    }

    fn read_one(&mut self) -> Result<(), CameraCaptureError> {
        let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut sample: Option<IMFSample> = None;
        unsafe { self.capture.reader.ReadSample(video, 0, None, Some(&raw mut flags), Some(&raw mut timestamp), Some(&raw mut sample)) }
            .map_err(|error| match camera_error("IMFSourceReader::ReadSample", error) {
                CameraCaptureError::OpenFailed(reason) => CameraCaptureError::StreamFailed(reason),
                other => other,
            })?;
        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            return Err(CameraCaptureError::StreamFailed("the source reader reported a stream error".to_owned()));
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Err(CameraCaptureError::NoCamera);
        }
        if let Some(sample) = sample {
            let captured_at = RiseClock::monotonic();
            match self.capture.frame(&sample) {
                Ok((geometry, pixels)) => (self.frames)(CameraFrame {
                    geometry,
                    pixels: pixels.into(),
                    captured_at,
                    mirrored: self.mirrored,
                }),
                Err(error) => tracing::debug!(%error, "camera frame dropped"),
            }
            if let Some(segment) = self.segment.as_mut() {
                if let Err(error) = segment.write(&sample, timestamp) {
                    tracing::warn!(%error, "the camera segment stopped taking frames");
                    self.segment = None;
                    if let Some((reply, _)) = self.pending_stop.take() {
                        let _ = reply.send(Err(error));
                    }
                    return Ok(());
                }
            }
        }
        if let Some((_, deadline)) = &self.pending_stop {
            let has_frames = self.segment.as_ref().is_some_and(|segment| segment.clock.has_frames());
            let expired = RiseClock::monotonic().as_nanos() as i64 / 100 >= *deadline;
            if has_frames || expired || self.segment.is_none() {
                if let Some((reply, _)) = self.pending_stop.take() {
                    let _ = reply.send(self.close_segment());
                }
            }
        }
        Ok(())
    }

    /// finalizes the open segment if there is one
    fn close_segment(&mut self) -> Result<(), CameraCaptureError> {
        match self.segment.take() {
            Some(mut segment) => segment.finish(),
            None => Err(CameraCaptureError::NotRecording),
        }
    }
}
