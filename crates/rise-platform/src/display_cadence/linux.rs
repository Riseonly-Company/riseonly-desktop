use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::NonNull;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_backend::sys::client::Backend;
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use x11rb::protocol::randr::{ConnectionExt as _, ModeFlag};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::xcb_ffi::XCBConnection;

use super::display_cadence::frames_per_second_from_millihertz;
use super::refresh_mode::*;

const WL_OUTPUT_VERSION: u32 = 4;
const WL_OUTPUT_RELEASE_SINCE: u32 = 3;

pub(super) fn reported_refresh_rate(window: &gpui::Window) -> Option<u32> {
    let display = HasDisplayHandle::display_handle(window).ok()?;
    let surface = HasWindowHandle::window_handle(window).ok()?;
    match (display.as_raw(), surface.as_raw()) {
        (RawDisplayHandle::Xcb(xcb), RawWindowHandle::Xcb(x_window)) => {
            x11_refresh(xcb.connection?, x_window.window.get())
        }
        (RawDisplayHandle::Wayland(wayland), RawWindowHandle::Wayland(_)) => {
            wayland_refresh(wayland.display)
        }
        _ => None,
    }
}

thread_local! {
    static XCB_GUEST: Cell<Option<(usize, &'static XCBConnection)>> = const { Cell::new(None) };
    static WAYLAND_GUEST: RefCell<Option<(usize, &'static mut WaylandGuest)>> =
        const { RefCell::new(None) };
}

fn xcb_guest(connection: NonNull<c_void>) -> Option<&'static XCBConnection> {
    let key = connection.as_ptr() as usize;
    if let Some((known, guest)) = XCB_GUEST.with(Cell::get) {
        if known == key {
            return Some(guest);
        }
    }
    let wrapped = unsafe { XCBConnection::from_raw_xcb_connection(connection.as_ptr(), false) }
        .ok()?;
    let guest: &'static XCBConnection = Box::leak(Box::new(wrapped));
    XCB_GUEST.with(|slot| slot.set(Some((key, guest))));
    Some(guest)
}

fn x11_refresh(connection: NonNull<c_void>, x_window: u32) -> Option<u32> {
    let conn = xcb_guest(connection)?;
    let geometry = conn.get_geometry(x_window).ok()?.reply().ok()?;
    let origin = conn
        .translate_coordinates(x_window, geometry.root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    let window = ScreenRect {
        x: i32::from(origin.dst_x),
        y: i32::from(origin.dst_y),
        width: u32::from(geometry.width),
        height: u32::from(geometry.height),
    };

    let resources = conn
        .randr_get_screen_resources_current(geometry.root)
        .ok()?
        .reply()
        .ok()?;
    let cookies: Vec<_> = resources
        .crtcs
        .iter()
        .filter_map(|crtc| conn.randr_get_crtc_info(*crtc, resources.config_timestamp).ok())
        .collect();
    let crtcs: Vec<CrtcArea> = cookies
        .into_iter()
        .filter_map(|cookie| cookie.reply().ok())
        .map(|info| CrtcArea {
            rect: ScreenRect {
                x: i32::from(info.x),
                y: i32::from(info.y),
                width: u32::from(info.width),
                height: u32::from(info.height),
            },
            mode: info.mode,
        })
        .collect();

    let holder = crtcs[crtc_holding_most(window, &crtcs)?];
    let line = resources.modes.iter().find(|mode| mode.id == holder.mode)?;
    let flags = u32::from(line.mode_flags);
    let millihertz = mode_millihertz(ModeTiming {
        dot_clock: line.dot_clock,
        h_total: line.htotal,
        v_total: line.vtotal,
        interlace: flags & u32::from(ModeFlag::INTERLACE) != 0,
        double_scan: flags & u32::from(ModeFlag::DOUBLE_SCAN) != 0,
    })?;
    Some(frames_per_second_from_millihertz(millihertz))
}

#[derive(Default)]
struct OutputState {
    refreshes: OutputRefreshes,
    bound: Vec<(u32, WlOutput)>,
}

struct WaylandGuest {
    queue: EventQueue<OutputState>,
    state: OutputState,
    _connection: Connection,
    _registry: WlRegistry,
}

impl Dispatch<WlRegistry, ()> for OutputState {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == WlOutput::interface().name => {
                let version = version.min(WL_OUTPUT_VERSION);
                let output = registry.bind::<WlOutput, u32, Self>(name, version, qh, name);
                state.refreshes.add(name);
                state.bound.push((name, output));
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.bound.iter().position(|(known, _)| *known == name) {
                    let (_, output) = state.bound.remove(index);
                    if output.version() >= WL_OUTPUT_RELEASE_SINCE {
                        output.release();
                    }
                    state.refreshes.remove(name);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for OutputState {
    fn event(
        state: &mut Self,
        _output: &WlOutput,
        event: wl_output::Event,
        name: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode { flags, refresh, .. } = event {
            let current =
                matches!(flags, WEnum::Value(mode) if mode.contains(wl_output::Mode::Current));
            state.refreshes.mode(*name, current, refresh);
        }
    }
}

fn wayland_guest(display: NonNull<c_void>) -> Option<&'static mut WaylandGuest> {
    let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
    let connection = Connection::from_backend(backend);
    let mut queue = connection.new_event_queue::<OutputState>();
    let registry = connection.display().get_registry(&queue.handle(), ());
    let mut state = OutputState::default();
    queue.roundtrip(&mut state).ok()?;
    queue.roundtrip(&mut state).ok()?;
    Some(Box::leak(Box::new(WaylandGuest {
        queue,
        state,
        _connection: connection,
        _registry: registry,
    })))
}

fn wayland_refresh(display: NonNull<c_void>) -> Option<u32> {
    let key = display.as_ptr() as usize;
    WAYLAND_GUEST.with(|slot| {
        let mut slot = slot.borrow_mut();
        let fresh = !matches!(&*slot, Some((known, _)) if *known == key);
        if fresh {
            *slot = Some((key, wayland_guest(display)?));
        }
        let (_, guest) = slot.as_mut()?;
        if !fresh {
            guest.queue.dispatch_pending(&mut guest.state).ok()?;
        }
        guest
            .state
            .refreshes
            .fastest_millihertz()
            .map(frames_per_second_from_millihertz)
    })
}
