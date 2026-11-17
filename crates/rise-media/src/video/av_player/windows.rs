use std::any::Any;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;

use ::windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, E_NOTIMPL, E_POINTER, HMODULE, RECT, RPC_E_CHANGED_MODE, S_OK};
use ::windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use ::windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
};
use ::windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use ::windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAsyncCallback, IMFAsyncCallback_Impl, IMFAsyncResult, IMFAttributes,
    IMFByteStream, IMFByteStream_Impl, IMFDXGIDeviceManager, IMFMediaEngine, IMFMediaEngineClassFactory,
    IMFMediaEngineEx, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl, MF_BYTESTREAM_CONTENT_TYPE, MF_E_SHUTDOWN,
    MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_DXGI_MANAGER, MF_MEDIA_ENGINE_EVENT_ENDED, MF_MEDIA_ENGINE_EVENT_ERROR,
    MF_MEDIA_ENGINE_EVENT_FORMATCHANGE, MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA, MF_MEDIA_ENGINE_EVENT_SEEKED,
    MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC, MF_MEDIA_ENGINE_PRELOAD_METADATA, MF_MEDIA_ENGINE_READY_HAVE_FUTURE_DATA,
    MF_MEDIA_ENGINE_SEEK_MODE_APPROXIMATE, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_VERSION, MFASYNC_CALLBACK_QUEUE_LONG_FUNCTION,
    MFBYTESTREAM_IS_READABLE, MFBYTESTREAM_IS_SEEKABLE, MFBYTESTREAM_SEEK_ORIGIN, MFSTARTUP_FULL, msoBegin, msoCurrent,
};
use ::windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use ::windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use ::windows::core::{BOOL, BSTR, ComObject, HRESULT, IUnknown, Interface, PCSTR, PCWSTR, Ref, implement, s, w};
use gpui::{AnyElement, IntoElement as _, ObjectFit, OwnedSurfaceFrame, Size, Styled as _, size};
use parking_lot::Mutex;
use rise_core::foundation::foundation_url::Url;
use rise_platform::HostOs;
use rise_platform::uniform_type::UTType;

use super::av_player::*;
use super::media_engine_frames::*;
use crate::texture::color::{ColorRange, ColorSpace, ConversionParams};
use crate::texture::frame::FrameFormat;
use crate::texture::native_surface::ycbcr_to_rgb_columns;
use crate::video::av_asset::{AVAssetByteSource, AVAssetReadError, ContentInformation};
use crate::video::av_player::pixel_buffer::Nv12PixelBuffer;

const NO_MEDIA_FOUNDATION: &str =
    "Media Foundation is not installed (a Windows N edition without the Media Feature Pack)";

const STREAM_ID_SPACE: u64 = 1 << 62;
static NEXT_STREAM: AtomicU64 = AtomicU64::new(0);

type MfStartupFn = unsafe extern "system" fn(u32, u32) -> HRESULT;
type MfShutdownFn = unsafe extern "system" fn() -> HRESULT;
type MfCreateAttributesFn = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type MfCreateDxgiDeviceManagerFn = unsafe extern "system" fn(*mut u32, *mut *mut c_void) -> HRESULT;
type MfCreateAsyncResultFn = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;
type MfPutWorkItemFn = unsafe extern "system" fn(u32, *mut c_void, *mut c_void) -> HRESULT;
type MfInvokeCallbackFn = unsafe extern "system" fn(*mut c_void) -> HRESULT;
type MfCreateByteStreamWrapperFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;

struct MediaFoundation {
    startup: MfStartupFn,
    shutdown: MfShutdownFn,
    create_attributes: MfCreateAttributesFn,
    create_dxgi_device_manager: MfCreateDxgiDeviceManagerFn,
    create_async_result: MfCreateAsyncResultFn,
    put_work_item: MfPutWorkItemFn,
    invoke_callback: MfInvokeCallbackFn,
    create_byte_stream_wrapper: MfCreateByteStreamWrapperFn,
}

fn media_foundation() -> Option<&'static MediaFoundation> {
    static RESOLVED: OnceLock<Option<MediaFoundation>> = OnceLock::new();
    RESOLVED.get_or_init(resolve_media_foundation).as_ref()
}

fn resolve_media_foundation() -> Option<MediaFoundation> {
    let platform = unsafe { LoadLibraryExW(w!("mfplat.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
    let export = |name: PCSTR| {
        unsafe { GetProcAddress(platform, name) }
    };
    let startup = export(s!("MFStartup"))?;
    let shutdown = export(s!("MFShutdown"))?;
    let create_attributes = export(s!("MFCreateAttributes"))?;
    let create_dxgi_device_manager = export(s!("MFCreateDXGIDeviceManager"))?;
    let create_async_result = export(s!("MFCreateAsyncResult"))?;
    let put_work_item = export(s!("MFPutWorkItem"))?;
    let invoke_callback = export(s!("MFInvokeCallback"))?;
    let create_byte_stream_wrapper = export(s!("MFCreateMFByteStreamWrapper"))?;
    type Exported = unsafe extern "system" fn() -> isize;
    unsafe {
        Some(MediaFoundation {
            startup: std::mem::transmute::<Exported, MfStartupFn>(startup),
            shutdown: std::mem::transmute::<Exported, MfShutdownFn>(shutdown),
            create_attributes: std::mem::transmute::<Exported, MfCreateAttributesFn>(create_attributes),
            create_dxgi_device_manager: std::mem::transmute::<Exported, MfCreateDxgiDeviceManagerFn>(
                create_dxgi_device_manager,
            ),
            create_async_result: std::mem::transmute::<Exported, MfCreateAsyncResultFn>(create_async_result),
            put_work_item: std::mem::transmute::<Exported, MfPutWorkItemFn>(put_work_item),
            invoke_callback: std::mem::transmute::<Exported, MfInvokeCallbackFn>(invoke_callback),
            create_byte_stream_wrapper: std::mem::transmute::<Exported, MfCreateByteStreamWrapperFn>(
                create_byte_stream_wrapper,
            ),
        })
    }
}

pub(super) fn player_available() -> bool {
    media_foundation().is_some()
}

/// wraps the interface that a creation call wrote into its out pointer
fn created<T: Interface>(make: impl FnOnce(*mut *mut c_void) -> HRESULT) -> ::windows::core::Result<T> {
    let mut raw: *mut c_void = ptr::null_mut();
    make(&mut raw).ok()?;
    if raw.is_null() {
        return Err(E_POINTER.into());
    }
    Ok(unsafe { T::from_raw(raw) })
}

/// what the player thread is asked to do, by gpui or by the engine events
enum Command {
    Muted(bool),
    Preload(bool),
    Play,
    Pause,
    Loop { source: Option<ClipSource> },
    Unloop,
    Event { event: u32, code: usize, status: u32 },
    Stop,
}

#[implement(IMFMediaEngineNotify)]
struct EngineEvents {
    commands: Sender<Command>,
}

impl IMFMediaEngineNotify_Impl for EngineEvents_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> ::windows::core::Result<()> {
        let _ = self.commands.send(Command::Event { event, code: param1, status: param2 });
        Ok(())
    }
}

struct WinPlayer {
    commands: Sender<Command>,
    picture: Arc<Mutex<Option<Nv12Picture>>>,
    source: String,
    stream_id: u64,
    waits: bool,
    buffers_ahead: bool,
}

struct LoopHolder {
    commands: Sender<Command>,
}

impl Drop for LoopHolder {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Unloop);
    }
}

pub(super) fn open_engine(item: &AVPlayerItem, ends: EndSink) -> Result<Box<dyn PlayerEngine>, AVPlayerError> {
    let api = media_foundation()
        .ok_or(AVPlayerError::NotOnThisHost { host: HostOs::Windows, reason: NO_MEDIA_FOUNDATION })?;
    let clip = ClipSource::of(item)?;
    let source = clip.url.clone();
    let (commands, receiver) = mpsc::channel();
    let picture = Arc::new(Mutex::new(None));
    let stream_id = STREAM_ID_SPACE | NEXT_STREAM.fetch_add(1, Ordering::Relaxed);
    let worker = PlayerThread {
        api,
        source: clip,
        ends,
        events: commands.clone(),
        receiver,
        picture: Arc::clone(&picture),
    };
    thread::Builder::new()
        .name("riseonly-clip-player".to_owned())
        .spawn(move || worker.run())
        .map_err(|_| AVPlayerError::Platform { call: "spawn the Media Engine thread" })?;
    Ok(Box::new(WinPlayer {
        commands,
        picture,
        source,
        stream_id,
        waits: true,
        buffers_ahead: buffers_ahead(item),
    }))
}

/// true when the item wants media buffered past the play head
fn buffers_ahead(item: &AVPlayerItem) -> bool {
    !item.preferred_forward_buffer_duration().is_zero()
}

fn engine_source(url: &Url) -> Result<String, AVPlayerError> {
    SystemUrl::of(url)?;
    Ok(url.absolute_string().to_owned())
}

/// what the engine opens for an item
/// the url alone or the url plus the asset loader that serves the bytes
#[derive(Clone, Debug)]
struct ClipSource {
    url: String,
    bytes: Option<AVAssetByteSource>,
}

impl ClipSource {
    fn of(item: &AVPlayerItem) -> Result<Self, AVPlayerError> {
        Ok(Self {
            url: engine_source(item.url())?,
            bytes: item.asset().map(|asset| asset.resource_loader().byte_source()),
        })
    }
}

impl Drop for WinPlayer {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
    }
}

impl WinPlayer {
    fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            tracing::debug!(target: "riseonly", "clip player thread already left");
        }
    }

    /// sends the preload hint for current settings
    /// automatic if waiting to minimize stalling or a forward buffer is wanted
    /// metadata otherwise
    fn send_preload(&self) {
        self.send(Command::Preload(self.waits || self.buffers_ahead));
    }
}

impl PlayerEngine for WinPlayer {
    fn set_is_muted(&mut self, muted: bool) {
        self.send(Command::Muted(muted));
    }

    fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd) {
        tracing::trace!(target: "riseonly", ?action, "clip action at item end");
    }

    fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool) {
        self.waits = waits;
        self.send_preload();
    }

    fn play(&mut self) {
        self.send(Command::Play);
    }

    fn pause(&mut self) {
        self.send(Command::Pause);
    }

    fn start_looping(&mut self, template: &AVPlayerItem) -> Result<Box<dyn Any>, AVPlayerError> {
        let template_source = ClipSource::of(template)?;
        let replaced = template_source.url != self.source;
        self.source = template_source.url.clone();
        let source = replaced.then_some(template_source);
        let template_buffers_ahead = buffers_ahead(template);
        if template_buffers_ahead != self.buffers_ahead {
            self.buffers_ahead = template_buffers_ahead;
            self.send_preload();
        }
        self.commands
            .send(Command::Loop { source })
            .map_err(|_| AVPlayerError::Platform { call: "IMFMediaEngine::SetLoop" })?;
        Ok(Box::new(LoopHolder { commands: self.commands.clone() }))
    }

    fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement> {
        let picture = self.picture.lock().clone()?;
        nv12_surface(self.stream_id, picture, fit)
    }
}

/// the picture as the owned nv12 surface of this stream, drawn into the callers bounds with fit
fn nv12_surface(stream_id: u64, picture: Nv12Picture, fit: ObjectFit) -> Option<AnyElement> {
    let owned = OwnedSurfaceFrame::nv12(
        stream_id,
        picture.generation,
        picture.width,
        picture.height,
        picture.luma,
        picture.luma_stride,
        picture.chroma,
        picture.chroma_stride,
    )?;
    Some(gpui::surface(owned).object_fit(fit).size_full().into_any_element())
}

/// com and media foundation started on the player thread, dropped after every media foundation object
struct MediaFoundationSession {
    api: &'static MediaFoundation,
    com_joined: bool,
    started: bool,
}

impl MediaFoundationSession {
    fn start(api: &'static MediaFoundation) -> ::windows::core::Result<Self> {
        let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if joined.is_err() && joined != RPC_E_CHANGED_MODE {
            return Err(joined.into());
        }
        let mut session = Self { api, com_joined: joined.is_ok(), started: false };
        unsafe { (api.startup)(MF_VERSION, MFSTARTUP_FULL) }.ok()?;
        session.started = true;
        Ok(session)
    }
}

impl Drop for MediaFoundationSession {
    fn drop(&mut self) {
        if self.started {
            let stopped = unsafe { (self.api.shutdown)() };
            if stopped.is_err() {
                tracing::debug!(target: "riseonly", ?stopped, "MFShutdown");
            }
        }
        if self.com_joined {
            unsafe { CoUninitialize() };
        }
    }
}

struct PlayerThread {
    api: &'static MediaFoundation,
    source: ClipSource,
    ends: EndSink,
    events: Sender<Command>,
    receiver: Receiver<Command>,
    picture: Arc<Mutex<Option<Nv12Picture>>>,
}

impl PlayerThread {
    fn run(self) {
        let session = match MediaFoundationSession::start(self.api) {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "clip player could not start Media Foundation");
                return;
            }
        };
        match Engine::open(self.api, &self.source, self.events.clone()) {
            Ok(engine) => self.serve(engine),
            Err(error) => tracing::warn!(target: "riseonly", %error, "clip player could not open the Media Engine"),
        }
        drop(session);
    }

    fn serve(self, mut engine: Engine) {
        let mut source = self.source;
        let mut playing = false;
        let mut looping = false;
        let mut loaded = false;
        let mut has_picture = false;
        let mut clock = PictureClock::default();
        let mut generation = 0_u64;
        loop {
            let polling = loaded && (playing || !has_picture);
            let command = if polling {
                match self.receiver.recv_timeout(PICTURE_POLL) {
                    Ok(command) => Some(command),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match self.receiver.recv() {
                    Ok(command) => Some(command),
                    Err(_) => break,
                }
            };
            match command {
                Some(Command::Stop) => break,
                Some(Command::Muted(muted)) => engine.apply(Setter::Muted(muted)),
                Some(Command::Preload(ahead)) => engine.apply(Setter::Preload(ahead)),
                Some(Command::Play) => {
                    playing = true;
                    engine.apply(Setter::Play);
                }
                Some(Command::Pause) => {
                    playing = false;
                    engine.apply(Setter::Pause);
                }
                Some(Command::Loop { source: replacement }) => {
                    if let Some(replacement) = replacement {
                        engine.load(&replacement);
                        source = replacement;
                        loaded = false;
                        clock.reset();
                        if playing {
                            engine.apply(Setter::Play);
                        }
                    }
                    looping = true;
                    engine.apply(Setter::Loop(true));
                }
                Some(Command::Unloop) => {
                    looping = false;
                    engine.apply(Setter::Loop(false));
                }
                Some(Command::Event { event, code, status }) => {
                    if event == engine_event(MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA)
                        || event == engine_event(MF_MEDIA_ENGINE_EVENT_FORMATCHANGE)
                    {
                        loaded = true;
                    } else if event == engine_event(MF_MEDIA_ENGINE_EVENT_ENDED) {
                        playing = false;
                        self.ends.deliver(&source.url);
                    } else if event == engine_event(MF_MEDIA_ENGINE_EVENT_ERROR) {
                        tracing::warn!(
                            target: "riseonly",
                            media_engine_error = code,
                            status = %format!("{status:#010x}"),
                            "clip player error"
                        );
                    }
                }
                None => {}
            }
            if !polling {
                continue;
            }
            let Some(presentation_time) = engine.new_picture_time() else {
                continue;
            };
            match clock.tick(presentation_time) {
                StreamTick::Same => continue,
                StreamTick::Wrapped if looping => self.ends.deliver(&source.url),
                StreamTick::Wrapped | StreamTick::New => {}
            }
            generation = generation.wrapping_add(1);
            match engine.transfer(generation) {
                Ok(Some(picture)) => {
                    *self.picture.lock() = Some(picture);
                    has_picture = true;
                }
                Ok(None) => {}
                Err(error) => tracing::debug!(target: "riseonly", %error, "clip picture transfer"),
            }
        }
    }
}

/// one engine call the player thread makes for a command
enum Setter {
    Muted(bool),
    Preload(bool),
    Play,
    Pause,
    Loop(bool),
    Rate(f64),
    Volume(f64),
}

/// seconds from the engine, none for nan (no resource) and infinity or a negative value
fn engine_seconds(value: f64) -> Option<f64> {
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// the event code as the notify callback gets it
fn engine_event(event: ::windows::Win32::Media::MediaFoundation::MF_MEDIA_ENGINE_EVENT) -> u32 {
    event.0.unsigned_abs()
}

/// the bgra render target the engine draws into and the staging copy the cpu reads
struct Targets {
    width: u32,
    height: u32,
    render: ID3D11Texture2D,
    staging: ID3D11Texture2D,
}

/// one media engine in frame server mode on its own d3d11 device, lives on the player thread
struct Engine {
    api: &'static MediaFoundation,
    media: IMFMediaEngine,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    _manager: IMFDXGIDeviceManager,
    targets: Option<Targets>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Err(error) = unsafe { self.media.Shutdown() } {
            tracing::debug!(target: "riseonly", %error, "IMFMediaEngine::Shutdown");
        }
    }
}

impl Engine {
    fn open(api: &'static MediaFoundation, source: &ClipSource, events: Sender<Command>) -> ::windows::core::Result<Self> {
        let engine = Self::create(api, EngineEvents { commands: events }.into())?;
        engine.set_source(source)?;
        Ok(engine)
    }

    /// makes an engine with no source that reports its events to notify
    fn create(api: &'static MediaFoundation, notify: IMFMediaEngineNotify) -> ::windows::core::Result<Self> {
        let (device, context) = create_device()?;
        let protection: ID3D11Multithread = device.cast()?;
        let _ = unsafe { protection.SetMultithreadProtected(true) };

        let mut reset_token = 0_u32;
        let manager: IMFDXGIDeviceManager = created(|out| {
            unsafe { (api.create_dxgi_device_manager)(&mut reset_token, out) }
        })?;
        unsafe { manager.ResetDevice(&device, reset_token) }?;

        let attributes: IMFAttributes = created(|out| {
            unsafe { (api.create_attributes)(out, 3) }
        })?;
        unsafe {
            attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
            attributes.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0.unsigned_abs())?;
        }
        let factory: IMFMediaEngineClassFactory =
            unsafe { CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER) }?;
        let media = unsafe { factory.CreateInstance(0, &attributes) }?;
        unsafe {
            media.SetAutoPlay(false)?;
            media.SetPreload(MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC)?;
        }
        Ok(Self { api, media, device, context, _manager: manager, targets: None })
    }

    /// current time and duration in seconds, none when the engine says nan or infinity
    fn times(&self) -> (Option<f64>, Option<f64>) {
        let (now, duration) = unsafe { (self.media.GetCurrentTime(), self.media.GetDuration()) };
        (engine_seconds(now), engine_seconds(duration))
    }

    fn is_ended(&self) -> bool {
        unsafe { self.media.IsEnded() }.as_bool()
    }

    /// moves the play head to seconds, nearest key frame if approximate else exact
    /// the engine raises the seeked event when done
    fn seek(&self, seconds: f64, approximate: bool) -> ::windows::core::Result<()> {
        if approximate {
            let extended: IMFMediaEngineEx = self.media.cast()?;
            unsafe { extended.SetCurrentTimeEx(seconds, MF_MEDIA_ENGINE_SEEK_MODE_APPROXIMATE) }
        } else {
            unsafe { self.media.SetCurrentTime(seconds) }
        }
    }

    fn is_starved(&self) -> bool {
        let (ready, seeking) = unsafe { (self.media.GetReadyState(), self.media.IsSeeking()) };
        i32::from(ready) < MF_MEDIA_ENGINE_READY_HAVE_FUTURE_DATA.0 || seeking.as_bool()
    }

    /// buffered ranges in seconds, empty when the engine cant tell
    fn buffered(&self) -> Vec<CMTimeRange> {
        let ranges = match unsafe { self.media.GetBuffered() } {
            Ok(ranges) => ranges,
            Err(error) => {
                tracing::debug!(target: "riseonly", %error, "IMFMediaEngine::GetBuffered");
                return Vec::new();
            }
        };
        let count = unsafe { ranges.GetLength() };
        (0..count)
            .filter_map(|index| {
                let (start, end) = unsafe { (ranges.GetStart(index), ranges.GetEnd(index)) };
                let (Ok(start), Ok(end)) = (start, end) else {
                    return None;
                };
                let range = CMTimeRange::new(start, end - start);
                range.is_valid().then_some(range)
            })
            .collect()
    }

    /// native picture size widened to the declared aspect ratio
    /// zero for no video, none while the engine cant tell
    fn presentation_size(&self) -> Option<Size<f32>> {
        let (mut width, mut height) = (0_u32, 0_u32);
        unsafe { self.media.GetNativeVideoSize(Some(&mut width), Some(&mut height)) }.ok()?;
        if width == 0 || height == 0 {
            return Some(size(0.0, 0.0));
        }
        let (mut aspect_x, mut aspect_y) = (0_u32, 0_u32);
        let aspect = unsafe { self.media.GetVideoAspectRatio(Some(&mut aspect_x), Some(&mut aspect_y)) };
        if aspect.is_err() || aspect_x == 0 || aspect_y == 0 {
            return Some(presentation_size_of(width, height, (1, 1), 0.0));
        }
        let shown_width = f64::from(height) * f64::from(aspect_x) / f64::from(aspect_y);
        Some(size(shown_width as f32, height as f32))
    }

    /// swaps the source, a failure is logged and the engine keeps the old one
    fn load(&self, source: &ClipSource) {
        if let Err(error) = self.set_source(source) {
            tracing::debug!(target: "riseonly", %error, "IMFMediaEngine source");
        }
    }

    /// opens the clip by url, or through the asset loader if the item has one
    fn set_source(&self, source: &ClipSource) -> ::windows::core::Result<()> {
        let url = BSTR::from(source.url.as_str());
        let Some(bytes) = &source.bytes else {
            return unsafe { self.media.SetSource(&url) };
        };
        let information = match bytes.content_information() {
            Ok(information) => Some(information),
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "clip stream content information");
                None
            }
        };
        let own: IMFByteStream = AssetByteStream {
            api: self.api,
            cursor: Arc::new(StreamCursor::new(bytes.clone(), information.as_ref())),
        }
        .into();
        let wrapped: IMFByteStream = created(|out| {
            unsafe { (self.api.create_byte_stream_wrapper)(own.as_raw(), out) }
        })?;
        if let Some(mime) = information.as_ref().and_then(content_mime_type) {
            match wrapped.cast::<IMFAttributes>() {
                Ok(attributes) => {
                    let wide: Vec<u16> = mime.encode_utf16().chain(std::iter::once(0)).collect();
                    if let Err(error) = unsafe { attributes.SetString(&MF_BYTESTREAM_CONTENT_TYPE, PCWSTR(wide.as_ptr())) } {
                        tracing::debug!(target: "riseonly", %error, "MF_BYTESTREAM_CONTENT_TYPE");
                    }
                }
                Err(error) => tracing::debug!(target: "riseonly", %error, "byte stream wrapper has no attribute store"),
            }
        }
        let extended: IMFMediaEngineEx = self.media.cast()?;
        unsafe { extended.SetSourceFromByteStream(&wrapped, &url) }
    }

    /// applies one setting, a failure is logged and the hub keeps the callers value
    fn apply(&self, setter: Setter) {
        let media = &self.media;
        let (call, applied) = unsafe {
            match &setter {
                Setter::Muted(muted) => ("SetMuted", media.SetMuted(*muted)),
                Setter::Preload(ahead) => (
                    "SetPreload",
                    media.SetPreload(if *ahead { MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC } else { MF_MEDIA_ENGINE_PRELOAD_METADATA }),
                ),
                Setter::Play => ("Play", media.Play()),
                Setter::Pause => ("Pause", media.Pause()),
                Setter::Loop(looping) => ("SetLoop", media.SetLoop(*looping)),
                Setter::Rate(rate) => ("SetPlaybackRate", media.SetDefaultPlaybackRate(*rate).and(media.SetPlaybackRate(*rate))),
                Setter::Volume(volume) => ("SetVolume", media.SetVolume(*volume)),
            }
        };
        if let Err(error) = applied {
            tracing::debug!(target: "riseonly", %error, call, "IMFMediaEngine");
        }
    }

    /// presentation time of the picture due now if the engine has one
    /// no picture yet or a failure means nothing to transfer
    fn new_picture_time(&self) -> Option<i64> {
        let mut presentation_time = 0_i64;
        let status = unsafe {
            (Interface::vtable(&self.media).OnVideoStreamTick)(Interface::as_raw(&self.media), &mut presentation_time)
        };
        (status == S_OK).then_some(presentation_time)
    }

    /// transfers the picture due now into nv12 planes, none before the size is known
    fn transfer(&mut self, generation: u64) -> ::windows::core::Result<Option<Nv12Picture>> {
        let (mut width, mut height) = (0_u32, 0_u32);
        unsafe { self.media.GetNativeVideoSize(Some(&mut width), Some(&mut height)) }?;
        if width == 0 || height == 0 {
            return Ok(None);
        }
        let (Ok(right), Ok(bottom)) = (i32::try_from(width), i32::try_from(height)) else {
            return Ok(None);
        };
        if !self.targets.as_ref().is_some_and(|targets| targets.width == width && targets.height == height) {
            self.targets = Some(Targets {
                width,
                height,
                render: texture(&self.device, width, height, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET.0.unsigned_abs(), 0)?,
                staging: texture(&self.device, width, height, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0.unsigned_abs())?,
            });
        }
        let Some(targets) = self.targets.as_ref() else {
            return Ok(None);
        };
        let whole = RECT { left: 0, top: 0, right, bottom };
        unsafe { self.media.TransferVideoFrame(&targets.render, None, &whole, None) }?;
        unsafe { self.context.CopyResource(&targets.staging, &targets.render) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.context.Map(&targets.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }?;
        let row_pitch = usize::try_from(mapped.RowPitch).unwrap_or(0);
        let rows = usize::try_from(height).unwrap_or(0);
        let row_bytes = usize::try_from(width).unwrap_or(0) * 4;
        let picture = if mapped.pData.is_null() || row_pitch < row_bytes {
            None
        } else {
            let length = row_pitch * (rows - 1) + row_bytes;
            let bytes = unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), length) };
            bgra_to_nv12(bytes, row_pitch, width, height, generation)
        };
        unsafe { self.context.Unmap(&targets.staging, 0) };
        Ok(picture)
    }
}

fn content_mime_type(information: &ContentInformation) -> Option<String> {
    information
        .content_type
        .as_deref()
        .and_then(UTType::from_identifier)
        .and_then(|uniform| uniform.preferred_mime_type())
}

/// position, size and state shared by the byte stream and its pending reads
struct StreamCursor {
    source: AVAssetByteSource,
    position: AtomicU64,
    length: Mutex<Option<Option<u64>>>,
    closed: AtomicBool,
}

impl StreamCursor {
    fn new(source: AVAssetByteSource, information: Option<&ContentInformation>) -> Self {
        let length = information.map(|information| u64::try_from(information.content_length).ok().filter(|length| *length > 0));
        Self { source, position: AtomicU64::new(0), length: Mutex::new(length), closed: AtomicBool::new(false) }
    }

    /// size of the resource, asked from the loader once
    /// runs on a media foundation thread, never the main one
    fn length(&self) -> Option<u64> {
        let mut known = self.length.lock();
        if let Some(length) = *known {
            return length;
        }
        let length = match self.source.content_information() {
            Ok(information) => u64::try_from(information.content_length).ok().filter(|length| *length > 0),
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "clip stream size unavailable");
                None
            }
        };
        *known = Some(length);
        length
    }

    /// reads up to capacity bytes at the current position and moves past them
    /// zero bytes means end of resource
    fn read_into(&self, buffer: *mut u8, capacity: u32) -> ::windows::core::Result<u32> {
        if self.closed.load(Ordering::Acquire) {
            return Err(MF_E_SHUTDOWN.into());
        }
        if buffer.is_null() {
            return Err(E_POINTER.into());
        }
        let position = self.position.load(Ordering::Acquire);
        let offset = i64::try_from(position).map_err(|_| ::windows::core::Error::from(E_INVALIDARG))?;
        let wanted = usize::try_from(capacity).unwrap_or(usize::MAX);
        let bytes = match self.source.read_at(offset, wanted) {
            Ok(bytes) => bytes,
            Err(AVAssetReadError::Cancelled | AVAssetReadError::LoaderGone) => Vec::new(),
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, position, "clip stream read failed");
                return Err(E_FAIL.into());
            }
        };
        let count = bytes.len().min(wanted);
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, count) };
        let advanced = u64::try_from(count).unwrap_or(u64::MAX);
        self.position.store(position.saturating_add(advanced), Ordering::Release);
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}

#[implement(IMFByteStream)]
struct AssetByteStream {
    api: &'static MediaFoundation,
    cursor: Arc<StreamCursor>,
}

impl IMFByteStream_Impl for AssetByteStream_Impl {
    fn GetCapabilities(&self) -> ::windows::core::Result<u32> {
        Ok(MFBYTESTREAM_IS_READABLE | MFBYTESTREAM_IS_SEEKABLE)
    }

    fn GetLength(&self) -> ::windows::core::Result<u64> {
        Ok(self.cursor.length().unwrap_or(u64::MAX))
    }

    fn SetLength(&self, _length: u64) -> ::windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetCurrentPosition(&self) -> ::windows::core::Result<u64> {
        Ok(self.cursor.position.load(Ordering::Acquire))
    }

    fn SetCurrentPosition(&self, position: u64) -> ::windows::core::Result<()> {
        self.cursor.position.store(position, Ordering::Release);
        Ok(())
    }

    fn IsEndOfStream(&self) -> ::windows::core::Result<BOOL> {
        let position = self.cursor.position.load(Ordering::Acquire);
        Ok(self.cursor.length().is_some_and(|length| position >= length).into())
    }

    fn Read(&self, buffer: *mut u8, capacity: u32, read: *mut u32) -> ::windows::core::Result<()> {
        if read.is_null() {
            return Err(E_POINTER.into());
        }
        let count = self.cursor.read_into(buffer, capacity)?;
        unsafe { read.write(count) };
        Ok(())
    }

    fn BeginRead(
        &self,
        buffer: *mut u8,
        capacity: u32,
        callback: Ref<IMFAsyncCallback>,
        state: Ref<IUnknown>,
    ) -> ::windows::core::Result<()> {
        let callback = callback.ok()?;
        if buffer.is_null() {
            return Err(E_POINTER.into());
        }
        let pending = ComObject::new(PendingRead {
            api: self.api,
            cursor: Arc::clone(&self.cursor),
            buffer,
            capacity,
            read: Mutex::new(0),
            caller: Mutex::new(None),
        });
        let work: IMFAsyncCallback = pending.to_interface();
        let object: IUnknown = work.cast()?;
        let state = state.as_ref().map_or(ptr::null_mut(), Interface::as_raw);
        let caller: IMFAsyncResult = created(|out| {
            unsafe { (self.api.create_async_result)(object.as_raw(), callback.as_raw(), state, out) }
        })?;
        *pending.caller.lock() = Some(caller);
        let queued = unsafe { (self.api.put_work_item)(MFASYNC_CALLBACK_QUEUE_LONG_FUNCTION, work.as_raw(), ptr::null_mut()) };
        if queued.is_err() {
            pending.caller.lock().take();
        }
        queued.ok()
    }

    fn EndRead(&self, result: Ref<IMFAsyncResult>) -> ::windows::core::Result<u32> {
        let result = result.ok()?;
        unsafe { result.GetStatus() }?;
        let object = unsafe { result.GetObject() }?;
        let pending = object.cast_object::<PendingRead>()?;
        let count = *pending.read.lock();
        Ok(count)
    }

    fn Write(&self, _buffer: *const u8, _length: u32) -> ::windows::core::Result<u32> {
        Err(E_NOTIMPL.into())
    }

    fn BeginWrite(
        &self,
        _buffer: *const u8,
        _length: u32,
        _callback: Ref<IMFAsyncCallback>,
        _state: Ref<IUnknown>,
    ) -> ::windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn EndWrite(&self, _result: Ref<IMFAsyncResult>) -> ::windows::core::Result<u32> {
        Err(E_NOTIMPL.into())
    }

    fn Seek(&self, origin: MFBYTESTREAM_SEEK_ORIGIN, offset: i64, _flags: u32) -> ::windows::core::Result<u64> {
        let base = if origin == msoBegin {
            0
        } else if origin == msoCurrent {
            i64::try_from(self.cursor.position.load(Ordering::Acquire)).map_err(|_| ::windows::core::Error::from(E_INVALIDARG))?
        } else {
            return Err(E_INVALIDARG.into());
        };
        let target = base
            .checked_add(offset)
            .and_then(|target| u64::try_from(target).ok())
            .ok_or_else(|| ::windows::core::Error::from(E_INVALIDARG))?;
        self.cursor.position.store(target, Ordering::Release);
        Ok(target)
    }

    fn Flush(&self) -> ::windows::core::Result<()> {
        Ok(())
    }

    fn Close(&self) -> ::windows::core::Result<()> {
        self.cursor.closed.store(true, Ordering::Release);
        Ok(())
    }
}

/// one begin read served on a work queue thread, also what end read takes the count from
#[implement(IMFAsyncCallback)]
struct PendingRead {
    api: &'static MediaFoundation,
    cursor: Arc<StreamCursor>,
    buffer: *mut u8,
    capacity: u32,
    read: Mutex<u32>,
    caller: Mutex<Option<IMFAsyncResult>>,
}

impl IMFAsyncCallback_Impl for PendingRead_Impl {
    fn GetParameters(&self, _flags: *mut u32, _queue: *mut u32) -> ::windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Invoke(&self, _result: Ref<IMFAsyncResult>) -> ::windows::core::Result<()> {
        let status = match self.cursor.read_into(self.buffer, self.capacity) {
            Ok(count) => {
                *self.read.lock() = count;
                S_OK
            }
            Err(error) => error.code(),
        };
        let Some(caller) = self.caller.lock().take() else {
            return Ok(());
        };
        unsafe { caller.SetStatus(status) }?;
        unsafe { (self.api.invoke_callback)(caller.as_raw()) }.ok()
    }
}

fn create_device() -> ::windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let flags = D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT;
    let mut failure: ::windows::core::Error = E_POINTER.into();
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        let mut context = None;
        let made = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        };
        match (made, device, context) {
            (Ok(()), Some(device), Some(context)) => return Ok((device, context)),
            (Err(error), _, _) => failure = error,
            _ => {}
        }
    }
    Err(failure)
}

fn texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    usage: D3D11_USAGE,
    bind: u32,
    cpu_access: u32,
) -> ::windows::core::Result<ID3D11Texture2D> {
    let description = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu_access,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&description, None, Some(&mut texture)) }?;
    texture.ok_or_else(|| E_POINTER.into())
}

/// what the media player thread is asked to do
enum PlaybackCommand {
    Replace { item: u64, source: Option<ClipSource>, ends: ItemEndSink },
    Play { rate: f64 },
    Pause,
    Seek { item: u64, window: SeekWindow, done: SeekSink },
    Configure(PlaybackSetting),
    Event { engine: u64, event: u32, code: usize, status: u32 },
    Stop,
}

/// player properties the thread keeps across items
#[derive(Clone, Copy, Debug)]
enum PlaybackSetting {
    Muted(bool),
    Volume(f32),
    ActionAtItemEnd(AVPlayerActionAtItemEnd),
    WaitsToMinimizeStalling(bool),
}

/// settings a new engine starts from, unmuted at full volume
/// pauses at item end and waits to minimize stalling
#[derive(Clone, Copy, Debug)]
struct PlaybackSettings {
    muted: bool,
    volume: f32,
    action_at_item_end: AVPlayerActionAtItemEnd,
    waits_to_minimize_stalling: bool,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self { muted: false, volume: 1.0, action_at_item_end: AVPlayerActionAtItemEnd::Pause, waits_to_minimize_stalling: true }
    }
}

impl PlaybackSettings {
    fn set(&mut self, setting: PlaybackSetting) {
        match setting {
            PlaybackSetting::Muted(muted) => self.muted = muted,
            PlaybackSetting::Volume(volume) => self.volume = volume,
            PlaybackSetting::ActionAtItemEnd(action) => self.action_at_item_end = action,
            PlaybackSetting::WaitsToMinimizeStalling(waits) => self.waits_to_minimize_stalling = waits,
        }
    }
}

#[implement(IMFMediaEngineNotify)]
struct PlaybackEvents {
    commands: Sender<PlaybackCommand>,
    engine: u64,
}

impl IMFMediaEngineNotify_Impl for PlaybackEvents_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> ::windows::core::Result<()> {
        let _ = self.commands.send(PlaybackCommand::Event { engine: self.engine, event, code: param1, status: param2 });
        Ok(())
    }
}

/// what the player thread last read for the numbered item
#[derive(Clone, Default)]
struct PlaybackTimes {
    item: u64,
    current: Option<f64>,
    duration: Option<f64>,
    status: Option<AVPlayerItemStatus>,
    waiting: bool,
    buffered: Vec<CMTimeRange>,
    presentation_size: Option<Size<f32>>,
}

/// newest picture with the number of the item it belongs to
type ItemPicture = Arc<Mutex<Option<(u64, Nv12Picture)>>>;

struct WinMediaPlayer {
    commands: Sender<PlaybackCommand>,
    picture: ItemPicture,
    times: Arc<Mutex<PlaybackTimes>>,
    stream_id: u64,
    item: u64,
    copied: Option<(u64, u64)>,
}

impl Drop for WinMediaPlayer {
    fn drop(&mut self) {
        let _ = self.commands.send(PlaybackCommand::Stop);
    }
}

impl WinMediaPlayer {
    fn send(&self, command: PlaybackCommand) {
        if self.commands.send(command).is_err() {
            tracing::debug!(target: "riseonly", "media player thread already left");
        }
    }

    /// reads what the player thread knows about the current item
    /// none while it still reports a replaced one
    fn current<T>(&self, read: impl FnOnce(&PlaybackTimes) -> T) -> Option<T> {
        let times = self.times.lock();
        (times.item == self.item).then(|| read(&times))
    }
}

impl PlaybackEngine for WinMediaPlayer {
    fn replace_current_item(&mut self, item: Option<&AVPlayerItem>, ends: ItemEndSink) -> Result<(), AVPlayerError> {
        self.item = self.item.wrapping_add(1);
        let (source, failure) = match item.map(ClipSource::of).transpose() {
            Ok(source) => (source, None),
            Err(error) => (None, Some(error)),
        };
        self.send(PlaybackCommand::Replace { item: self.item, source, ends });
        failure.map_or(Ok(()), Err)
    }

    fn play_immediately(&mut self, rate: f32) {
        self.set_rate(rate);
    }

    fn set_rate(&mut self, rate: f32) {
        let rate = f64::from(rate);
        if rate.is_nan() || rate <= 0.0 {
            self.send(PlaybackCommand::Pause);
        } else {
            self.send(PlaybackCommand::Play { rate });
        }
    }

    fn pause(&mut self) {
        self.send(PlaybackCommand::Pause);
    }

    fn current_time(&self) -> Option<f64> {
        self.current(|times| times.current).flatten()
    }

    fn current_item_duration(&self) -> Option<f64> {
        self.current(|times| times.duration).flatten()
    }

    fn seek(&mut self, window: SeekWindow, done: SeekSink) {
        if let Err(refused) = self.commands.send(PlaybackCommand::Seek { item: self.item, window, done }) {
            tracing::debug!(target: "riseonly", "media player thread already left");
            if let PlaybackCommand::Seek { done, .. } = refused.0 {
                done.deliver(false);
            }
        }
    }

    fn set_is_muted(&mut self, muted: bool) -> Result<(), AVPlayerError> {
        self.send(PlaybackCommand::Configure(PlaybackSetting::Muted(muted)));
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), AVPlayerError> {
        self.send(PlaybackCommand::Configure(PlaybackSetting::Volume(volume)));
        Ok(())
    }

    fn set_action_at_item_end(&mut self, action: AVPlayerActionAtItemEnd) -> Result<(), AVPlayerError> {
        self.send(PlaybackCommand::Configure(PlaybackSetting::ActionAtItemEnd(action)));
        Ok(())
    }

    fn set_automatically_waits_to_minimize_stalling(&mut self, waits: bool) -> Result<(), AVPlayerError> {
        self.send(PlaybackCommand::Configure(PlaybackSetting::WaitsToMinimizeStalling(waits)));
        Ok(())
    }

    fn time_control_status(&self) -> Option<AVPlayerTimeControlStatus> {
        self.current(|times| {
            if times.waiting {
                AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate
            } else {
                AVPlayerTimeControlStatus::Playing
            }
        })
    }

    fn item_status(&self) -> Option<AVPlayerItemStatus> {
        self.current(|times| times.status).flatten()
    }

    fn loaded_time_ranges(&self) -> Vec<CMTimeRange> {
        self.current(|times| times.buffered.clone()).unwrap_or_default()
    }

    fn presentation_size(&self) -> Option<Size<f32>> {
        self.current(|times| times.presentation_size).flatten()
    }

    fn is_ready_for_display(&self) -> Option<bool> {
        let slot = self.picture.lock();
        Some(matches!(slot.as_ref(), Some((item, _)) if *item == self.item))
    }

    fn frame(&mut self, fit: ObjectFit) -> Option<AnyElement> {
        let picture = {
            let slot = self.picture.lock();
            match slot.as_ref() {
                Some((item, picture)) if *item == self.item => picture.clone(),
                _ => return None,
            }
        };
        nv12_surface(self.stream_id, picture, fit)
    }

    fn copy_pixel_buffer(&mut self) -> Option<AVPlayerPixelBuffer> {
        let picture = {
            let slot = self.picture.lock();
            let (item, picture) = slot.as_ref()?;
            let key = (*item, picture.generation);
            if *item != self.item || self.copied == Some(key) {
                return None;
            }
            self.copied = Some(key);
            picture.clone()
        };
        let ycbcr_to_rgb =
            ycbcr_to_rgb_columns(&ConversionParams::new(FrameFormat::Nv12, ColorSpace::Bt601, ColorRange::Full));
        Some(AVPlayerPixelBuffer::new(Box::new(Nv12PixelBuffer::new(picture, ycbcr_to_rgb, 0))))
    }
}

/// a seek the engine was asked for or one waiting for the item to load
struct PendingSeek {
    window: SeekWindow,
    done: SeekSink,
    approximate: bool,
    started: bool,
}

/// engine of the current item plus what the player thread knows about its playback
struct PlaybackItem {
    engine: Engine,
    engine_id: u64,
    item: u64,
    ends: ItemEndSink,
    playing: bool,
    wants_play: bool,
    loaded: bool,
    failed: bool,
    has_picture: bool,
    clock: PictureClock,
    action_at_item_end: AVPlayerActionAtItemEnd,
    seek: Option<PendingSeek>,
}

impl Drop for PlaybackItem {
    fn drop(&mut self) {
        if let Some(seek) = self.seek.take() {
            seek.done.deliver(false);
        }
    }
}

impl PlaybackItem {
    /// true when the thread polls for pictures, ie a loaded item that plays or has none to show yet
    fn polls(&self) -> bool {
        self.loaded && (self.playing || !self.has_picture)
    }

    fn status(&self) -> AVPlayerItemStatus {
        if self.failed {
            AVPlayerItemStatus::Failed
        } else if self.loaded {
            AVPlayerItemStatus::ReadyToPlay
        } else {
            AVPlayerItemStatus::Unknown
        }
    }

    /// item should play but the engine cant advance it
    /// an item held at its end with action none is not waiting
    fn waiting(&self) -> bool {
        self.wants_play && !self.failed && !self.engine.is_ended() && (self.seek.is_some() || self.engine.is_starved())
    }

    fn play(&mut self, rate: f64) {
        self.wants_play = true;
        self.engine.apply(Setter::Rate(rate));
        if self.engine.is_ended() {
            return;
        }
        self.playing = true;
        self.engine.apply(Setter::Play);
    }

    fn pause(&mut self) {
        self.wants_play = false;
        self.playing = false;
        self.engine.apply(Setter::Pause);
    }

    /// takes over a seek, an earlier pending one finishes as unfinished
    fn seek(&mut self, window: SeekWindow, done: SeekSink) {
        if let Some(earlier) = self.seek.take() {
            earlier.done.deliver(false);
        }
        if self.failed {
            done.deliver(false);
            return;
        }
        self.seek = Some(PendingSeek { window, done, approximate: !window.is_exact(), started: false });
        self.start_seek();
    }

    /// asks the engine for the pending seek once the item is loaded
    /// the engine stores no seek without metadata
    fn start_seek(&mut self) {
        if !self.loaded {
            return;
        }
        let Some(seek) = self.seek.as_mut().filter(|seek| !seek.started) else {
            return;
        };
        seek.started = true;
        let seconds = seek.window.target_us as f64 / 1_000_000.0;
        if let Err(error) = self.engine.seek(seconds, seek.approximate) {
            tracing::debug!(target: "riseonly", %error, "IMFMediaEngine seek");
            if let Some(seek) = self.seek.take() {
                seek.done.deliver(false);
            }
        }
    }

    /// seeked event, the pending seek is done when the play head is inside its window
    /// an approximate landing outside it seeks again exactly
    fn seeked(&mut self) {
        if !self.seek.as_ref().is_some_and(|seek| seek.started) {
            return;
        }
        let Some(mut seek) = self.seek.take() else {
            return;
        };
        let (now, _) = self.engine.times();
        let landed = now.is_some_and(|now| {
            let now_us = (now * 1_000_000.0).round() as i64;
            !seek.window.is_before(now_us) && now_us <= seek.window.max_us
        });
        if !landed && seek.approximate {
            seek.approximate = false;
            seek.started = false;
            self.seek = Some(seek);
            self.start_seek();
            return;
        }
        seek.done.deliver(true);
        if self.wants_play && !self.playing {
            self.playing = true;
            self.engine.apply(Setter::Play);
        }
    }

    fn configure(&mut self, setting: PlaybackSetting) {
        match setting {
            PlaybackSetting::Muted(muted) => self.engine.apply(Setter::Muted(muted)),
            PlaybackSetting::Volume(volume) => self.engine.apply(Setter::Volume(f64::from(volume))),
            PlaybackSetting::ActionAtItemEnd(action) => self.action_at_item_end = action,
            PlaybackSetting::WaitsToMinimizeStalling(waits) => self.engine.apply(Setter::Preload(waits)),
        }
    }

    fn event(&mut self, event: u32, code: usize, status: u32) {
        if event == engine_event(MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA)
            || event == engine_event(MF_MEDIA_ENGINE_EVENT_FORMATCHANGE)
        {
            self.loaded = true;
            self.start_seek();
        } else if event == engine_event(MF_MEDIA_ENGINE_EVENT_SEEKED) {
            self.seeked();
        } else if event == engine_event(MF_MEDIA_ENGINE_EVENT_ENDED) {
            self.playing = false;
            if self.action_at_item_end != AVPlayerActionAtItemEnd::None {
                self.wants_play = false;
            }
            self.ends.deliver();
        } else if event == engine_event(MF_MEDIA_ENGINE_EVENT_ERROR) {
            self.playing = false;
            self.failed = true;
            if let Some(seek) = self.seek.take() {
                seek.done.deliver(false);
            }
            tracing::warn!(
                target: "riseonly",
                media_engine_error = code,
                status = %format!("{status:#010x}"),
                "media player error"
            );
        }
    }
}

struct PlaybackThread {
    api: &'static MediaFoundation,
    receiver: Receiver<PlaybackCommand>,
    events: Sender<PlaybackCommand>,
    picture: ItemPicture,
    times: Arc<Mutex<PlaybackTimes>>,
}

impl PlaybackThread {
    fn run(self) {
        let session = match MediaFoundationSession::start(self.api) {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "media player could not start Media Foundation");
                return;
            }
        };
        self.serve();
        drop(session);
    }

    /// engine for source whose events carry engine_id, with the player settings
    fn open(&self, source: &ClipSource, engine_id: u64, settings: PlaybackSettings) -> ::windows::core::Result<Engine> {
        let notify: IMFMediaEngineNotify = PlaybackEvents { commands: self.events.clone(), engine: engine_id }.into();
        let engine = Engine::create(self.api, notify)?;
        engine.apply(Setter::Muted(settings.muted));
        engine.apply(Setter::Volume(f64::from(settings.volume)));
        engine.apply(Setter::Preload(settings.waits_to_minimize_stalling));
        engine.set_source(source)?;
        Ok(engine)
    }

    fn serve(self) {
        let mut current: Option<PlaybackItem> = None;
        let mut settings = PlaybackSettings::default();
        let mut engines = 0_u64;
        let mut generation = 0_u64;
        loop {
            let polling = current.as_ref().is_some_and(PlaybackItem::polls);
            let command = if polling {
                match self.receiver.recv_timeout(PICTURE_POLL) {
                    Ok(command) => Some(command),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match self.receiver.recv() {
                    Ok(command) => Some(command),
                    Err(_) => break,
                }
            };
            let command_seen = command.is_some();
            match command {
                Some(PlaybackCommand::Stop) => break,
                Some(PlaybackCommand::Replace { item, source, ends }) => {
                    current = None;
                    let mut status = None;
                    if let Some(source) = source {
                        engines = engines.wrapping_add(1);
                        match self.open(&source, engines, settings) {
                            Ok(engine) => {
                                current = Some(PlaybackItem {
                                    engine,
                                    engine_id: engines,
                                    item,
                                    ends,
                                    playing: false,
                                    wants_play: false,
                                    loaded: false,
                                    failed: false,
                                    has_picture: false,
                                    clock: PictureClock::default(),
                                    action_at_item_end: settings.action_at_item_end,
                                    seek: None,
                                });
                                status = Some(AVPlayerItemStatus::Unknown);
                            }
                            Err(error) => {
                                tracing::warn!(target: "riseonly", %error, "media player could not open the item");
                                status = Some(AVPlayerItemStatus::Failed);
                            }
                        }
                    }
                    *self.times.lock() = PlaybackTimes { item, status, ..PlaybackTimes::default() };
                }
                Some(PlaybackCommand::Seek { item, window, done }) => {
                    match current.as_mut().filter(|playback| playback.item == item) {
                        Some(playback) => playback.seek(window, done),
                        None => done.deliver(false),
                    }
                }
                Some(PlaybackCommand::Configure(setting)) => {
                    settings.set(setting);
                    if let Some(playback) = current.as_mut() {
                        playback.configure(setting);
                    }
                }
                Some(PlaybackCommand::Play { rate }) => {
                    if let Some(playback) = current.as_mut() {
                        playback.play(rate);
                    }
                }
                Some(PlaybackCommand::Pause) => {
                    if let Some(playback) = current.as_mut() {
                        playback.pause();
                    }
                }
                Some(PlaybackCommand::Event { engine, event, code, status }) => {
                    if let Some(playback) = current.as_mut().filter(|playback| playback.engine_id == engine) {
                        playback.event(event, code, status);
                    }
                }
                None => {}
            }
            let Some(playback) = current.as_mut() else {
                continue;
            };
            let (now, duration) = playback.engine.times();
            let waiting = playback.waiting();
            let details = command_seen.then(|| (playback.status(), playback.engine.buffered(), playback.engine.presentation_size()));
            {
                let mut times = self.times.lock();
                times.item = playback.item;
                times.current = now;
                times.duration = duration;
                times.waiting = waiting;
                if let Some((status, buffered, presentation_size)) = details {
                    times.status = Some(status);
                    times.buffered = buffered;
                    times.presentation_size = presentation_size;
                }
            }
            if !polling {
                continue;
            }
            let Some(presentation_time) = playback.engine.new_picture_time() else {
                continue;
            };
            if playback.clock.tick(presentation_time) == StreamTick::Same {
                continue;
            }
            generation = generation.wrapping_add(1);
            match playback.engine.transfer(generation) {
                Ok(Some(picture)) => {
                    *self.picture.lock() = Some((playback.item, picture));
                    playback.has_picture = true;
                }
                Ok(None) => {}
                Err(error) => tracing::debug!(target: "riseonly", %error, "media player picture transfer"),
            }
        }
    }
}

pub(super) fn open_playback_engine() -> Result<Box<dyn PlaybackEngine>, AVPlayerError> {
    let api = media_foundation()
        .ok_or(AVPlayerError::NotOnThisHost { host: HostOs::Windows, reason: NO_MEDIA_FOUNDATION })?;
    let (commands, receiver) = mpsc::channel();
    let picture: ItemPicture = Arc::new(Mutex::new(None));
    let times = Arc::new(Mutex::new(PlaybackTimes::default()));
    let stream_id = STREAM_ID_SPACE | NEXT_STREAM.fetch_add(1, Ordering::Relaxed);
    let worker = PlaybackThread {
        api,
        receiver,
        events: commands.clone(),
        picture: Arc::clone(&picture),
        times: Arc::clone(&times),
    };
    thread::Builder::new()
        .name("riseonly-media-player".to_owned())
        .spawn(move || worker.run())
        .map_err(|_| AVPlayerError::Platform { call: "spawn the Media Engine thread" })?;
    Ok(Box::new(WinMediaPlayer { commands, picture, times, stream_id, item: 0, copied: None }))
}
