use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use livekit::webrtc::desktop_capturer::{
    CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions, DesktopFrame,
};
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use windows::core::{implement, IUnknown, Interface, Ref, HRESULT, PCWSTR};
use windows::Wdk::System::SystemServices::RtlGetVersion;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ACTIVE, ENUM_CURRENT_SETTINGS,
};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
    WAVE_FORMAT_PCM,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, BLOB, COINIT_MULTITHREADED};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindow};

use super::screen_capture::*;

const START_TIMEOUT: Duration = Duration::from_secs(5);
const SCREEN_PREFIX: &str = "screen:";
const WINDOW_PREFIX: &str = "window:";
const PROCESS_LOOPBACK_MIN_BUILD: u32 = 20348;
const SYSTEM_AUDIO_ACTIVATE_TIMEOUT: Duration = Duration::from_secs(5);
const SYSTEM_AUDIO_RATE: u32 = 48_000;
const SYSTEM_AUDIO_CHANNELS: u16 = 2;
const SYSTEM_AUDIO_BITS: u16 = 16;
const SYSTEM_AUDIO_BUFFER: i64 = 200_000;
const SYSTEM_AUDIO_WAIT_MILLIS: u32 = 100;

enum Route {
    Streaming { frames: ScreenFrameSink, ended: ScreenEndSink, options: CaptureOptions },
    Done,
}

fn source_type(kind: CaptureSourceKind) -> DesktopCaptureSourceType {
    match kind {
        CaptureSourceKind::Display => DesktopCaptureSourceType::Screen,
        CaptureSourceKind::Window => DesktopCaptureSourceType::Window,
    }
}

fn capturer(kind: CaptureSourceKind) -> Option<DesktopCapturer> {
    let mut options = DesktopCapturerOptions::new(source_type(kind));
    options.set_include_cursor(true);
    DesktopCapturer::new(options)
}

fn source_id(kind: CaptureSourceKind, native: u64) -> String {
    match kind {
        CaptureSourceKind::Display => format!("{SCREEN_PREFIX}{native}"),
        CaptureSourceKind::Window => format!("{WINDOW_PREFIX}{native}"),
    }
}

fn parse_id(id: &str) -> Option<(CaptureSourceKind, u64)> {
    if let Some(native) = id.strip_prefix(SCREEN_PREFIX) {
        return native.parse().ok().map(|native| (CaptureSourceKind::Display, native));
    }
    id.strip_prefix(WINDOW_PREFIX)?.parse().ok().map(|native| (CaptureSourceKind::Window, native))
}

fn display_pixels(index: u64) -> Option<(u32, u32)> {
    let index = u32::try_from(index).ok()?;
    let mut device = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
    if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), index, &raw mut device, 0) }.as_bool() {
        return None;
    }
    if !device.StateFlags.contains(DISPLAY_DEVICE_ACTIVE) {
        return None;
    }
    let mut mode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
    if !unsafe { EnumDisplaySettingsW(PCWSTR(device.DeviceName.as_ptr()), ENUM_CURRENT_SETTINGS, &raw mut mode) }
        .as_bool()
    {
        return None;
    }
    Some((mode.dmPelsWidth, mode.dmPelsHeight))
}

fn window_pixels(native: u64) -> Option<(u32, u32)> {
    let hwnd = HWND(native as usize as *mut core::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return None;
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut rect) }.ok()?;
    let width = u32::try_from(rect.right - rect.left).ok()?;
    let height = u32::try_from(rect.bottom - rect.top).ok()?;
    Some((width, height))
}

fn list(kind: CaptureSourceKind) -> Vec<CaptureSource> {
    let _apartment = MtaApartment::enter().ok();
    let Some(capturer) = capturer(kind) else {
        return Vec::new();
    };
    capturer
        .get_source_list()
        .into_iter()
        .filter_map(|listed| {
            let native = listed.id();
            let (width, height) = match kind {
                CaptureSourceKind::Display => display_pixels(native),
                CaptureSourceKind::Window => window_pixels(native),
            }?;
            Some(CaptureSource { id: source_id(kind, native), kind, name: listed.title(), width, height })
        })
        .collect()
}

fn frame_size(frame: &DesktopFrame) -> Option<(u32, u32)> {
    let width = u32::try_from(frame.width()).ok().filter(|side| *side > 0)?;
    let height = u32::try_from(frame.height()).ok().filter(|side| *side > 0)?;
    Some((width, height))
}

fn screen_frame(frame: &DesktopFrame, options: CaptureOptions) -> Option<ScreenFrame> {
    let (width, height) = frame_size(frame)?;
    let (out_width, out_height) = options.output_size(width, height);
    let pixels = scale_bgra(frame.data(), width, height, frame.stride(), out_width, out_height).ok()?;
    Some(ScreenFrame {
        width: out_width,
        height: out_height,
        pixels: ScreenPixels::Bgra { pixels: pixels.into(), stride: out_width * 4 },
        captured_at: RiseClock::monotonic(),
    })
}

fn deliver(route: &Mutex<Route>, result: Result<DesktopFrame, CaptureError>) {
    let mut guard = route.lock();
    let (frames, ended, options) = match &*guard {
        Route::Done => return,
        Route::Streaming { frames, ended, options } => (frames.clone(), ended.clone(), *options),
    };
    match result {
        Ok(frame) => {
            drop(guard);
            if let Some(frame) = screen_frame(&frame, options) {
                frames(frame);
            }
        }
        Err(CaptureError::Temporary) => {}
        Err(CaptureError::Permanent) => {
            *guard = Route::Done;
            drop(guard);
            ended(ScreenCaptureEnd::SourceLost);
        }
    }
}

fn run_pump(
    kind: CaptureSourceKind,
    native: u64,
    interval: Duration,
    route: Arc<Mutex<Route>>,
    stop: Arc<AtomicBool>,
    started: mpsc::Sender<Result<(), ScreenCaptureError>>,
) {
    let _apartment = match MtaApartment::enter() {
        Ok(apartment) => Some(apartment),
        Err(error) => {
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: no COM apartment: {error}"));
            None
        }
    };
    let Some(mut capturer) = capturer(kind) else {
        let _ = started.send(Err(ScreenCaptureError::StartFailed("libwebrtc made no capturer".into())));
        return;
    };
    let Some(listed) = capturer.get_source_list().into_iter().find(|listed| listed.id() == native) else {
        let _ = started.send(Err(ScreenCaptureError::SourceGone));
        return;
    };
    let callback_route = route.clone();
    capturer.start_capture(Some(listed), move |result| deliver(&callback_route, result));
    let _ = started.send(Ok(()));
    while !stop.load(Ordering::Acquire) {
        capturer.capture_frame();
        thread::sleep(interval);
    }
}

fn on_main_thread() -> bool {
    thread::current().name() == Some("main")
}

struct WindowsSession {
    source: CaptureSource,
    route: Arc<Mutex<Route>>,
    stop: Arc<AtomicBool>,
}

impl ScreenCaptureSession for WindowsSession {
    fn source(&self) -> &CaptureSource {
        &self.source
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        *self.route.lock() = Route::Done;
    }
}

impl Drop for WindowsSession {
    fn drop(&mut self) {
        self.stop();
    }
}

/// build RtlGetVersion answers, GetVersionExW lies to a process whose manifest doesnt cover it
/// zero when the system refused, reads as too old
fn windows_build() -> u32 {
    let mut info = OSVERSIONINFOW { dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32, ..Default::default() };
    let status = unsafe { RtlGetVersion(&raw mut info) };
    if status.0 < 0 { 0 } else { info.dwBuildNumber }
}

fn system_audio_format() -> WAVEFORMATEX {
    let block = SYSTEM_AUDIO_CHANNELS * SYSTEM_AUDIO_BITS / 8;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: SYSTEM_AUDIO_CHANNELS,
        nSamplesPerSec: SYSTEM_AUDIO_RATE,
        nAvgBytesPerSec: SYSTEM_AUDIO_RATE * u32::from(block),
        nBlockAlign: block,
        wBitsPerSample: SYSTEM_AUDIO_BITS,
        cbSize: 0,
    }
}

fn pcm16_from_bytes(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect()
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct LoopbackActivated {
    done: mpsc::Sender<()>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for LoopbackActivated_Impl {
    fn ActivateCompleted(&self, _operation: Ref<IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        let _ = self.done.send(());
        Ok(())
    }
}

struct MtaApartment;

impl MtaApartment {
    fn enter() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().map_err(|error| error.to_string())?;
        Ok(Self)
    }
}

impl Drop for MtaApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct AudioEvent(HANDLE);

impl AudioEvent {
    fn new() -> Result<Self, String> {
        unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.map(Self).map_err(|error| error.to_string())
    }
}

impl Drop for AudioEvent {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn activate_loopback_client() -> Result<IAudioClient, String> {
    let text = |error: windows::core::Error| error.to_string();
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: std::process::id(),
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let blob = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: (&raw mut params).cast(),
                    },
                },
            }),
        },
    });
    let (done, activated) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = LoopbackActivated { done }.into();
    let operation = unsafe {
        ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&raw const *blob), &handler)
    }
    .map_err(text)?;
    activated.recv_timeout(SYSTEM_AUDIO_ACTIVATE_TIMEOUT).map_err(|_| "the loopback activation did not answer".to_owned())?;
    let mut result = HRESULT(0);
    let mut client: Option<IUnknown> = None;
    unsafe { operation.GetActivateResult(&raw mut result, &raw mut client) }.map_err(text)?;
    result.ok().map_err(text)?;
    client.ok_or_else(|| "the loopback activation gave no client".to_owned())?.cast().map_err(text)
}

fn drain_system_audio(capture: &IAudioCaptureClient, sink: &ScreenAudioSink) -> Result<(), String> {
    let text = |error: windows::core::Error| error.to_string();
    let channels = usize::from(SYSTEM_AUDIO_CHANNELS);
    loop {
        if unsafe { capture.GetNextPacketSize() }.map_err(text)? == 0 {
            return Ok(());
        }
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;
        unsafe { capture.GetBuffer(&raw mut data, &raw mut frames, &raw mut flags, None, None) }.map_err(text)?;
        if frames == 0 {
            return Ok(());
        }
        let count = frames as usize * channels;
        let samples = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
            vec![0; count]
        } else {
            pcm16_from_bytes(unsafe { std::slice::from_raw_parts(data, count * std::mem::size_of::<i16>()) })
        };
        unsafe { capture.ReleaseBuffer(frames) }.map_err(text)?;
        sink(ScreenAudioBlock { samples, sample_rate: SYSTEM_AUDIO_RATE, channels: SYSTEM_AUDIO_CHANNELS });
    }
}

fn run_system_audio(sink: &ScreenAudioSink, stop: &AtomicBool) -> Result<(), String> {
    let text = |error: windows::core::Error| error.to_string();
    let _apartment = MtaApartment::enter()?;
    let client = activate_loopback_client()?;
    let event = AudioEvent::new()?;
    let format = system_audio_format();
    let flags = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM;
    unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, SYSTEM_AUDIO_BUFFER, 0, &raw const format, None) }.map_err(text)?;
    unsafe { client.SetEventHandle(event.0) }.map_err(text)?;
    let capture: IAudioCaptureClient = unsafe { client.GetService() }.map_err(text)?;
    unsafe { client.Start() }.map_err(text)?;
    let mut outcome = Ok(());
    while outcome.is_ok() && !stop.load(Ordering::Acquire) {
        let _ = unsafe { WaitForSingleObject(event.0, SYSTEM_AUDIO_WAIT_MILLIS) };
        outcome = drain_system_audio(&capture, sink);
    }
    let _ = unsafe { client.Stop() };
    outcome
}

struct WindowsSessionWithAudio {
    video: Box<dyn ScreenCaptureSession>,
    stop: Arc<AtomicBool>,
}

impl ScreenCaptureSession for WindowsSessionWithAudio {
    fn source(&self) -> &CaptureSource {
        self.video.source()
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.video.stop();
    }
}

impl Drop for WindowsSessionWithAudio {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct NativeScreenCapture;

impl ScreenCaptureBackend for NativeScreenCapture {
    fn support(&self) -> Result<(), ScreenCaptureError> {
        static SUPPORTED: OnceLock<bool> = OnceLock::new();
        if *SUPPORTED.get_or_init(|| {
            let _apartment = MtaApartment::enter().ok();
            capturer(CaptureSourceKind::Display).is_some()
        }) {
            Ok(())
        } else {
            Err(ScreenCaptureError::Unsupported(
                "this Windows has no desktop capture (Windows 10 1903 or later)".into(),
            ))
        }
    }

    fn access(&self) -> ScreenCaptureAccess {
        ScreenCaptureAccess::Granted
    }

    fn pick_source(&self, _parent: &PickerParent) -> Result<CaptureSource, ScreenCaptureError> {
        self.support()?;
        Err(ScreenCaptureError::PickerUnavailable)
    }

    fn sources(&self) -> Result<Vec<CaptureSource>, ScreenCaptureError> {
        self.support()?;
        let mut sources = list(CaptureSourceKind::Display);
        sources.extend(list(CaptureSourceKind::Window));
        Ok(sources)
    }

    fn start(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        self.support()?;
        if on_main_thread() {
            return Err(ScreenCaptureError::MainThread);
        }
        let (kind, native) = parse_id(&source.id).ok_or(ScreenCaptureError::SourceGone)?;
        let route = Arc::new(Mutex::new(Route::Streaming { frames, ended, options }));
        let stop = Arc::new(AtomicBool::new(false));
        let (started, answer) = mpsc::channel();
        let (thread_route, thread_stop, interval) = (route.clone(), stop.clone(), options.frame_interval());
        thread::Builder::new()
            .name("rise-screen-capture".into())
            .spawn(move || run_pump(kind, native, interval, thread_route, thread_stop, started))
            .map_err(|error| ScreenCaptureError::StartFailed(error.to_string()))?;
        let session = WindowsSession { source: source.clone(), route, stop };
        match answer.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Box::new(session)),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(ScreenCaptureError::StartFailed("the capture thread did not start".into())),
        }
    }

    fn system_audio_support(&self) -> Result<(), ScreenCaptureError> {
        self.support()?;
        static BUILD: OnceLock<u32> = OnceLock::new();
        if *BUILD.get_or_init(windows_build) >= PROCESS_LOOPBACK_MIN_BUILD {
            Ok(())
        } else {
            Err(ScreenCaptureError::Unsupported(format!(
                "sharing the system sound needs Windows build {PROCESS_LOOPBACK_MIN_BUILD} or later"
            )))
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
        let video = self.start(source, options, frames, ended)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let spawned = thread::Builder::new().name("rise-screen-audio".into()).spawn(move || {
            if let Err(error) = run_system_audio(&audio, &thread_stop) {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: no sound: {error}"));
            }
        });
        if let Err(error) = spawned {
            RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: no sound: {error}"));
            return Ok(video);
        }
        Ok(Box::new(WindowsSessionWithAudio { video, stop }))
    }
}
