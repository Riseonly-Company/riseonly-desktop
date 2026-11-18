use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Cursor;
use std::mem::size_of;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use livekit::webrtc::desktop_capturer::{
    CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions, DesktopFrame,
};
use parking_lot::Mutex;
use pipewire::context::Context;
use pipewire::link::Link;
use pipewire::main_loop::MainLoop;
use pipewire::node::{Node, NodeListener};
use pipewire::properties::properties;
use pipewire::registry::{GlobalObject, Registry};
use pipewire::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::serialize::PodSerializer;
use pipewire::spa::pod::{Object, Pod, Value};
use pipewire::spa::utils::dict::DictRef;
use pipewire::spa::utils::{Direction, SpaTypes};
use pipewire::stream::{Stream, StreamFlags, StreamState};
use pipewire::types::ObjectType;
use rise_core::foundation::foundation_date::RiseClock;
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use x11rb::connection::Connection as _;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::desktop_session::*;
use super::screen_capture::*;
use super::system_audio_routing::*;

const PICK_POLL: Duration = Duration::from_millis(100);
const HELD_POLL: Duration = Duration::from_millis(500);
const X11_START_TIMEOUT: Duration = Duration::from_secs(5);
const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREEN_CAST_INTERFACE: &str = "org.freedesktop.portal.ScreenCast";
const SCREEN_CAST_VERSION: &str = "version";

static NEXT_SOURCE: AtomicU64 = AtomicU64::new(1);
static PENDING: Mutex<Option<(String, Pump)>> = Mutex::new(None);

type PickReply = Result<(u32, u32), ScreenCaptureError>;

enum Route {
    Picking(mpsc::Sender<PickReply>),
    Held,
    Streaming { frames: ScreenFrameSink, ended: ScreenEndSink, options: CaptureOptions },
    Done,
}

/// one capturer on its own thread
struct Pump {
    route: Arc<Mutex<Route>>,
    stop: Arc<AtomicBool>,
    interval_nanos: Arc<AtomicU64>,
}

impl Pump {
    fn spawn(answer: mpsc::Sender<PickReply>) -> Result<Self, ScreenCaptureError> {
        let route = Arc::new(Mutex::new(Route::Picking(answer)));
        let stop = Arc::new(AtomicBool::new(false));
        let interval_nanos = Arc::new(AtomicU64::new(nanos(PICK_POLL)));
        let (thread_route, thread_stop, thread_interval) = (route.clone(), stop.clone(), interval_nanos.clone());
        thread::Builder::new()
            .name("rise-screen-capture".into())
            .spawn(move || run_pump(&thread_route, &thread_stop, &thread_interval))
            .map_err(|error| ScreenCaptureError::StartFailed(error.to_string()))?;
        Ok(Self { route, stop, interval_nanos })
    }

    fn set_interval(&self, interval: Duration) {
        self.interval_nanos.store(nanos(interval), Ordering::Release);
    }

    /// never waits for the thread since it may be the thread calling
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        *self.route.lock() = Route::Done;
    }
}

fn nanos(interval: Duration) -> u64 {
    u64::try_from(interval.as_nanos()).unwrap_or(u64::MAX).max(1)
}

fn run_pump(route: &Arc<Mutex<Route>>, stop: &AtomicBool, interval_nanos: &AtomicU64) {
    let mut options = DesktopCapturerOptions::new(DesktopCaptureSourceType::Generic);
    options.set_include_cursor(true);
    let Some(mut capturer) = DesktopCapturer::new(options) else {
        fail(route, ScreenCaptureError::StartFailed("libwebrtc made no portal capturer".into()));
        return;
    };
    let callback_route = route.clone();
    capturer.start_capture(None, move |result| deliver(&callback_route, result));
    while !stop.load(Ordering::Acquire) {
        capturer.capture_frame();
        thread::sleep(Duration::from_nanos(interval_nanos.load(Ordering::Acquire)));
    }
}

fn fail(route: &Mutex<Route>, error: ScreenCaptureError) {
    let previous = std::mem::replace(&mut *route.lock(), Route::Done);
    match previous {
        Route::Picking(answer) => {
            let _ = answer.send(Err(error));
        }
        Route::Streaming { ended, .. } => ended(ScreenCaptureEnd::Failed(error)),
        Route::Held | Route::Done => {}
    }
}

fn deliver(route: &Mutex<Route>, result: Result<DesktopFrame, CaptureError>) {
    let mut guard = route.lock();
    let (frames, ended, options) = match &*guard {
        Route::Picking(answer) => {
            let answer = answer.clone();
            let reply = match result {
                Ok(frame) => frame_size(&frame).map(Ok),
                Err(CaptureError::Permanent) => Some(Err(ScreenCaptureError::Cancelled)),
                Err(CaptureError::Temporary) => None,
            };
            if let Some(reply) = reply {
                *guard = if reply.is_ok() { Route::Held } else { Route::Done };
                drop(guard);
                let _ = answer.send(reply);
            }
            return;
        }
        Route::Held => {
            if matches!(result, Err(CaptureError::Permanent)) {
                *guard = Route::Done;
            }
            return;
        }
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
            ended(ScreenCaptureEnd::StoppedBySystem);
        }
    }
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

/// session from the variables libwebrtc reads
fn session() -> DesktopSession {
    let session_type = std::env::var_os("XDG_SESSION_TYPE").unwrap_or_default();
    let x_display = std::env::var_os("DISPLAY").unwrap_or_default();
    desktop_session(
        &session_type.to_string_lossy(),
        std::env::var_os("WAYLAND_DISPLAY").is_some(),
        &x_display.to_string_lossy(),
    )
}

/// arm of this session, probed once
/// libwebrtc loads libX11 lazily and makes no x11 capturer
/// when the display doesnt open or a library is missing
fn route() -> Result<DesktopSession, ScreenCaptureError> {
    static ROUTE: OnceLock<DesktopSession> = OnceLock::new();
    let route = *ROUTE.get_or_init(|| match session() {
        DesktopSession::X11 if x11_capturer(CaptureSourceKind::Display).is_none() => DesktopSession::NoDisplay,
        other => other,
    });
    match route {
        DesktopSession::NoDisplay => Err(ScreenCaptureError::Unsupported(match session() {
            DesktopSession::X11 => "libwebrtc could not open X11 capture on this display (libX11, Xfixes, Xdamage, \
                Xrandr, Xcomposite)"
                .into(),
            _ => "screen sharing on Linux needs a Wayland session with the ScreenCast portal or an X11 display".into(),
        })),
        live => Ok(live),
    }
}

fn x11_source_type(kind: CaptureSourceKind) -> DesktopCaptureSourceType {
    match kind {
        CaptureSourceKind::Display => DesktopCaptureSourceType::Screen,
        CaptureSourceKind::Window => DesktopCaptureSourceType::Window,
    }
}

/// outside wayland libwebrtc create screen and window capturer calls give the x11 capturers
fn x11_capturer(kind: CaptureSourceKind) -> Option<DesktopCapturer> {
    let mut options = DesktopCapturerOptions::new(x11_source_type(kind));
    options.set_include_cursor(true);
    DesktopCapturer::new(options)
}

/// sizes of what libwebrtc lists, read from the x server on the app own connection
struct X11Geometry {
    connection: Option<RustConnection>,
    root: (u32, u32),
    monitors: Vec<X11Monitor>,
}

impl X11Geometry {
    fn read() -> Self {
        let Ok((connection, screen)) = x11rb::connect(None) else {
            return Self { connection: None, root: (0, 0), monitors: Vec::new() };
        };
        let Some((root_window, root)) = connection
            .setup()
            .roots
            .get(screen)
            .map(|root| (root.root, (u32::from(root.width_in_pixels), u32::from(root.height_in_pixels))))
        else {
            return Self { connection: None, root: (0, 0), monitors: Vec::new() };
        };
        let monitors = connection
            .randr_get_monitors(root_window, true)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| {
                reply
                    .monitors
                    .iter()
                    .map(|monitor| X11Monitor {
                        name: u64::from(monitor.name),
                        width: u32::from(monitor.width),
                        height: u32::from(monitor.height),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { connection: Some(connection), root, monitors }
    }

    fn screen(&self, id: u64) -> Option<(u32, u32)> {
        self.connection.as_ref()?;
        x11_screen_pixels(id, &self.monitors, self.root)
    }

    fn window(&self, id: u64) -> Option<(u32, u32)> {
        let connection = self.connection.as_ref()?;
        let window = u32::try_from(id).ok()?;
        let geometry = connection.get_geometry(window).ok()?.reply().ok()?;
        x11_window_pixels(geometry.width, geometry.height)
    }
}

fn x11_list(kind: CaptureSourceKind, geometry: &X11Geometry) -> Vec<CaptureSource> {
    let Some(capturer) = x11_capturer(kind) else {
        return Vec::new();
    };
    capturer
        .get_source_list()
        .into_iter()
        .filter_map(|listed| {
            let native = listed.id();
            let (width, height) = match kind {
                CaptureSourceKind::Display => geometry.screen(native),
                CaptureSourceKind::Window => geometry.window(native),
            }?;
            Some(CaptureSource { id: x11_source_id(kind, native), kind, name: listed.title(), width, height })
        })
        .collect()
}

enum X11Route {
    Streaming { frames: ScreenFrameSink, ended: ScreenEndSink, options: CaptureOptions },
    Done,
}

fn x11_deliver(route: &Mutex<X11Route>, result: Result<DesktopFrame, CaptureError>) {
    let mut guard = route.lock();
    let (frames, ended, options) = match &*guard {
        X11Route::Done => return,
        X11Route::Streaming { frames, ended, options } => (frames.clone(), ended.clone(), *options),
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
            *guard = X11Route::Done;
            drop(guard);
            ended(ScreenCaptureEnd::SourceLost);
        }
    }
}

fn run_x11_pump(
    kind: CaptureSourceKind,
    native: u64,
    interval: Duration,
    route: Arc<Mutex<X11Route>>,
    stop: Arc<AtomicBool>,
    started: mpsc::Sender<Result<(), ScreenCaptureError>>,
) {
    let Some(mut capturer) = x11_capturer(kind) else {
        let _ = started.send(Err(ScreenCaptureError::StartFailed("libwebrtc made no X11 capturer".into())));
        return;
    };
    let Some(listed) = capturer.get_source_list().into_iter().find(|listed| listed.id() == native) else {
        let _ = started.send(Err(ScreenCaptureError::SourceGone));
        return;
    };
    let callback_route = route.clone();
    capturer.start_capture(Some(listed), move |result| x11_deliver(&callback_route, result));
    let _ = started.send(Ok(()));
    while !stop.load(Ordering::Acquire) {
        capturer.capture_frame();
        thread::sleep(interval);
    }
}

struct X11Session {
    source: CaptureSource,
    route: Arc<Mutex<X11Route>>,
    stop: Arc<AtomicBool>,
}

impl ScreenCaptureSession for X11Session {
    fn source(&self) -> &CaptureSource {
        &self.source
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        *self.route.lock() = X11Route::Done;
    }
}

impl Drop for X11Session {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start_x11(
    source: &CaptureSource,
    kind: CaptureSourceKind,
    native: u64,
    options: CaptureOptions,
    frames: ScreenFrameSink,
    ended: ScreenEndSink,
) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
    let route = Arc::new(Mutex::new(X11Route::Streaming { frames, ended, options }));
    let stop = Arc::new(AtomicBool::new(false));
    let (started, answer) = mpsc::channel();
    let (thread_route, thread_stop, interval) = (route.clone(), stop.clone(), options.frame_interval());
    thread::Builder::new()
        .name("rise-screen-capture".into())
        .spawn(move || run_x11_pump(kind, native, interval, thread_route, thread_stop, started))
        .map_err(|error| ScreenCaptureError::StartFailed(error.to_string()))?;
    let session = X11Session { source: source.clone(), route, stop };
    match answer.recv_timeout(X11_START_TIMEOUT) {
        Ok(Ok(())) => Ok(Box::new(session)),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(ScreenCaptureError::StartFailed("the capture thread did not start".into())),
    }
}

fn screen_cast_portal() -> Result<(), ScreenCaptureError> {
    let missing = |error: zbus::Error| ScreenCaptureError::PortalMissing(error.to_string());
    let connection = zbus::blocking::Connection::session().map_err(missing)?;
    zbus::blocking::proxy::Builder::<zbus::blocking::Proxy<'_>>::new(&connection)
        .destination(PORTAL_BUS_NAME)
        .and_then(|builder| builder.path(PORTAL_OBJECT_PATH))
        .and_then(|builder| builder.interface(SCREEN_CAST_INTERFACE))
        .map(|builder| builder.cache_properties(zbus::proxy::CacheProperties::No))
        .and_then(|builder| builder.build())
        .and_then(|proxy| proxy.get_property::<u32>(SCREEN_CAST_VERSION))
        .map(|_| ())
        .map_err(missing)
}

fn on_main_thread() -> bool {
    thread::current().name() == Some("main")
}

struct LinuxSession {
    source: CaptureSource,
    pump: Option<Pump>,
}

impl ScreenCaptureSession for LinuxSession {
    fn source(&self) -> &CaptureSource {
        &self.source
    }

    fn stop(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.stop();
        }
    }
}

impl Drop for LinuxSession {
    fn drop(&mut self) {
        self.stop();
    }
}

const SYSTEM_AUDIO_RATE: u32 = 48_000;
const SYSTEM_AUDIO_CHANNELS: u16 = 2;

/// the sound of every other program without this one
/// a sink monitor would send call voices back to speakers and pipewire cant skip a client
/// so the share owns an unlisted null sink and records its monitor
struct AudioRoutes {
    graph: AudioGraph,
    sink_node: Option<u32>,
    links: HashMap<AudioLink, Link>,
    players: HashMap<u32, (Node, NodeListener)>,
}

/// brings the links to what the graph asks for, on the pipewire thread
fn reconcile_audio_links(routes: &RefCell<AudioRoutes>, core: &pipewire::core::Core) {
    let mut routes = routes.borrow_mut();
    let Some(sink_node) = routes.sink_node else {
        return;
    };
    let wanted = routes.graph.wanted_links(sink_node, std::process::id());
    routes.links.retain(|link, _| wanted.contains(link));
    for link in wanted {
        if routes.links.contains_key(&link) {
            continue;
        }
        let created = core.create_object::<Link>(
            "link-factory",
            &properties! {
                "link.output.node" => link.output_node.to_string(),
                "link.output.port" => link.output_port.to_string(),
                "link.input.node" => link.input_node.to_string(),
                "link.input.port" => link.input_port.to_string(),
                "object.linger" => "false",
            },
        );
        match created {
            Ok(proxy) => {
                routes.links.insert(link, proxy);
            }
            Err(error) => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: sound link failed: {error}"));
            }
        }
    }
}

fn on_audio_node(
    global: &GlobalObject<&DictRef>,
    props: &DictRef,
    sink_name: &str,
    routes: &Rc<RefCell<AudioRoutes>>,
    registry: &Registry,
    core: &pipewire::core::Core,
) {
    if props.get("node.name") == Some(sink_name) {
        routes.borrow_mut().sink_node = Some(global.id);
    } else if props.get("media.class") == Some(PLAYER_MEDIA_CLASS) {
        let Ok(node) = registry.bind::<Node, _>(global) else {
            return;
        };
        let id = global.id;
        let info_routes = routes.clone();
        let info_core = core.clone();
        let listener = node
            .add_listener_local()
            .info(move |info| {
                let Some(props) = info.props() else {
                    return;
                };
                let process = props.get("application.process.id").and_then(parse_graph_id);
                info_routes.borrow_mut().graph.set_player_process(id, process);
                reconcile_audio_links(&info_routes, &info_core);
            })
            .register();
        let mut routes = routes.borrow_mut();
        routes.graph.add_player(id);
        routes.players.insert(id, (node, listener));
    }
}

fn on_audio_port(global: &GlobalObject<&DictRef>, props: &DictRef, routes: &RefCell<AudioRoutes>) {
    let node = props.get("node.id").and_then(parse_graph_id);
    let direction = props.get("port.direction").and_then(parse_port_direction);
    let (Some(node), Some(direction), Some(channel)) = (node, direction, props.get("audio.channel")) else {
        return;
    };
    routes.borrow_mut().graph.add_port(global.id, AudioGraphPort { node, direction, channel: channel.to_owned() });
}

fn system_audio_format() -> Result<Vec<u8>, String> {
    let mut position = [0u32; 64];
    position[0] = pipewire::spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = pipewire::spa::sys::SPA_AUDIO_CHANNEL_FR;
    let mut info = AudioInfoRaw::new();
    info.set_format(AudioFormat::F32LE);
    info.set_rate(SYSTEM_AUDIO_RATE);
    info.set_channels(u32::from(SYSTEM_AUDIO_CHANNELS));
    info.set_position(position);
    let object = Object { type_: SpaTypes::ObjectParamFormat.as_raw(), id: ParamType::EnumFormat.as_raw(), properties: info.into() };
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(object))
        .map(|(cursor, _)| cursor.into_inner())
        .map_err(|error| format!("{error:?}"))
}

fn connect_system_audio_stream(stream: &Stream, format: &[u8]) -> Result<(), String> {
    let format = Pod::from_bytes(format).ok_or("the sound format pod did not parse")?;
    stream
        .connect(Direction::Input, None, StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS, &mut [format])
        .map_err(|error| error.to_string())
}

fn run_system_audio(sink: ScreenAudioSink, terminate: pipewire::channel::Receiver<()>, stopped: Arc<AtomicBool>) -> Result<(), String> {
    pipewire::init();
    let text = |error: pipewire::Error| error.to_string();
    let main_loop = MainLoop::new(None).map_err(text)?;
    let context = Context::new(&main_loop).map_err(text)?;
    let core = context.connect(None).map_err(text)?;
    let registry = Rc::new(core.get_registry().map_err(text)?);

    let quit = main_loop.clone();
    let _terminate = terminate.attach(main_loop.loop_(), move |()| quit.quit());
    let quit = main_loop.clone();
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _, _, message| {
            if id == pipewire::sys::PW_ID_CORE {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: sound graph lost: {message}"));
                quit.quit();
            }
        })
        .register();

    let sink_name = format!("riseonly-screen-audio-{}", std::process::id());
    let _sink_node: Node = core
        .create_object(
            "adapter",
            &properties! {
                "factory.name" => "support.null-audio-sink",
                "node.name" => sink_name.as_str(),
                "media.class" => "Audio/Sink",
                "audio.position" => "FL,FR",
                "object.linger" => "false",
            },
        )
        .map_err(text)?;

    let stream = Rc::new(Stream::new(
        &core,
        "riseonly-screen-audio",
        properties! {
            "media.type" => "Audio",
            "media.category" => "Capture",
            "target.object" => sink_name.as_str(),
            "stream.capture.sink" => "true",
            "node.dont-reconnect" => "true",
            "node.dont-fallback" => "true",
        },
    )
    .map_err(text)?);
    let quit = main_loop.clone();
    let _stream_listener = stream
        .add_local_listener::<()>()
        .state_changed(move |_, _, _, state| {
            if let StreamState::Error(message) = state {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: sound stream failed: {message}"));
                quit.quit();
            }
        })
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            if stopped.load(Ordering::Acquire) {
                return;
            }
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let (offset, size) = (data.chunk().offset() as usize, data.chunk().size() as usize);
            let Some(mapped) = data.data() else {
                return;
            };
            let Some(bytes) = mapped.get(offset..offset.saturating_add(size)) else {
                return;
            };
            let floats: Vec<f32> = bytes
                .chunks_exact(size_of::<f32>())
                .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
                .collect();
            if floats.is_empty() {
                return;
            }
            sink(ScreenAudioBlock {
                samples: pcm16_from_float(&floats),
                sample_rate: SYSTEM_AUDIO_RATE,
                channels: SYSTEM_AUDIO_CHANNELS,
            });
        })
        .register()
        .map_err(text)?;
    let format = system_audio_format()?;
    let routes = Rc::new(RefCell::new(AudioRoutes {
        graph: AudioGraph::default(),
        sink_node: None,
        links: HashMap::new(),
        players: HashMap::new(),
    }));
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let (routes, registry, core, sink_name) = (routes.clone(), registry.clone(), core.clone(), sink_name.clone());
            let (stream, quit) = (stream.clone(), main_loop.clone());
            let connected = Cell::new(false);
            move |global| {
                let Some(props) = global.props else {
                    return;
                };
                match global.type_ {
                    ObjectType::Node => on_audio_node(global, props, &sink_name, &routes, &registry, &core),
                    ObjectType::Port => on_audio_port(global, props, &routes),
                    _ => return,
                }
                reconcile_audio_links(&routes, &core);
                if routes.borrow().sink_node.is_some() && !connected.replace(true) {
                    if let Err(reason) = connect_system_audio_stream(&stream, &format) {
                        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: no sound: {reason}"));
                        quit.quit();
                    }
                }
            }
        })
        .global_remove({
            let (routes, core) = (routes.clone(), core.clone());
            move |id| {
                {
                    let mut routes = routes.borrow_mut();
                    routes.graph.remove(id);
                    routes.players.remove(&id);
                }
                reconcile_audio_links(&routes, &core);
            }
        })
        .register();

    main_loop.run();
    Ok(())
}

/// sound half of a share, on its own thread
struct SystemAudioPump {
    terminate: pipewire::channel::Sender<()>,
    stopped: Arc<AtomicBool>,
}

impl SystemAudioPump {
    fn spawn(sink: ScreenAudioSink) -> Result<Self, ScreenCaptureError> {
        let (terminate, receiver) = pipewire::channel::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_stopped = stopped.clone();
        thread::Builder::new()
            .name("rise-screen-audio".into())
            .spawn(move || {
                if let Err(reason) = run_system_audio(sink, receiver, thread_stopped) {
                    RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: no sound: {reason}"));
                }
            })
            .map_err(|error| ScreenCaptureError::StartFailed(error.to_string()))?;
        Ok(Self { terminate, stopped })
    }

    /// never waits for the thread
    /// after a lost graph the thread is gone and the send reaches no one
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        let _ = self.terminate.send(());
    }
}

struct SessionWithAudio {
    video: Box<dyn ScreenCaptureSession>,
    audio: Option<SystemAudioPump>,
}

impl ScreenCaptureSession for SessionWithAudio {
    fn source(&self) -> &CaptureSource {
        self.video.source()
    }

    fn stop(&mut self) {
        if let Some(audio) = self.audio.take() {
            audio.stop();
        }
        self.video.stop();
    }
}

impl Drop for SessionWithAudio {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct NativeScreenCapture;

impl ScreenCaptureBackend for NativeScreenCapture {
    fn support(&self) -> Result<(), ScreenCaptureError> {
        route().map(|_| ())
    }

    fn access(&self) -> ScreenCaptureAccess {
        match route() {
            Ok(DesktopSession::X11) => ScreenCaptureAccess::Granted,
            _ => ScreenCaptureAccess::AskedOnPick,
        }
    }

    fn pick_source(&self, _parent: &PickerParent) -> Result<CaptureSource, ScreenCaptureError> {
        if route()? != DesktopSession::Portal {
            return Err(ScreenCaptureError::PickerUnavailable);
        }
        if on_main_thread() {
            return Err(ScreenCaptureError::MainThread);
        }
        screen_cast_portal()?;
        if let Some((_, previous)) = PENDING.lock().take() {
            previous.stop();
        }
        let (answer, replies) = mpsc::channel();
        let pump = Pump::spawn(answer)?;
        let reply = replies.recv_timeout(PICKER_WINDOW);
        let (width, height) = match reply {
            Ok(Ok(size)) => size,
            Ok(Err(error)) => {
                pump.stop();
                return Err(error);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                pump.stop();
                return Err(ScreenCaptureError::PickerTimedOut);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                pump.stop();
                return Err(ScreenCaptureError::StartFailed("the portal capturer stopped".into()));
            }
        };
        pump.set_interval(HELD_POLL);
        let id = format!("portal-{}", NEXT_SOURCE.fetch_add(1, Ordering::Relaxed));
        let source = CaptureSource { id: id.clone(), kind: CaptureSourceKind::Display, name: String::new(), width, height };
        *PENDING.lock() = Some((id, pump));
        Ok(source)
    }

    fn sources(&self) -> Result<Vec<CaptureSource>, ScreenCaptureError> {
        if route()? != DesktopSession::X11 {
            return Err(ScreenCaptureError::PickerUnavailable);
        }
        let geometry = X11Geometry::read();
        let mut sources = x11_list(CaptureSourceKind::Display, &geometry);
        sources.extend(x11_list(CaptureSourceKind::Window, &geometry));
        Ok(sources)
    }

    fn start(
        &self,
        source: &CaptureSource,
        options: CaptureOptions,
        frames: ScreenFrameSink,
        ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        let session = route()?;
        if let Some((kind, native)) = parse_x11_source_id(&source.id) {
            if session != DesktopSession::X11 {
                return Err(ScreenCaptureError::SourceGone);
            }
            if on_main_thread() {
                return Err(ScreenCaptureError::MainThread);
            }
            return start_x11(source, kind, native, options, frames, ended);
        }
        let pump = {
            let mut pending = PENDING.lock();
            match pending.take() {
                Some((id, pump)) if id == source.id => pump,
                other => {
                    *pending = other;
                    return Err(ScreenCaptureError::SourceGone);
                }
            }
        };
        {
            let mut route = pump.route.lock();
            if matches!(*route, Route::Done) {
                drop(route);
                pump.stop();
                return Err(ScreenCaptureError::SourceGone);
            }
            *route = Route::Streaming { frames, ended, options };
        }
        pump.set_interval(options.frame_interval());
        Ok(Box::new(LinuxSession { source: source.clone(), pump: Some(pump) }))
    }

    fn system_audio_support(&self) -> Result<(), ScreenCaptureError> {
        route()?;
        let var = |name: &str| std::env::var(name).ok();
        let socket = pipewire_socket_path(var("PIPEWIRE_REMOTE").as_deref(), var("PIPEWIRE_RUNTIME_DIR").as_deref(), var("XDG_RUNTIME_DIR").as_deref());
        if socket.is_some_and(|socket| socket.exists()) {
            Ok(())
        } else {
            Err(ScreenCaptureError::Unsupported("sharing the system sound needs a PipeWire session".into()))
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
        let audio = match SystemAudioPump::spawn(audio) {
            Ok(pump) => Some(pump),
            Err(error) => {
                RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Failure, || format!("screen share: no sound: {error}"));
                None
            }
        };
        Ok(Box::new(SessionWithAudio { video, audio }))
    }
}
