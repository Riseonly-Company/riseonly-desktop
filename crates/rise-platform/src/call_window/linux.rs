use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{Atom, ClientMessageEvent, ConnectionExt as _, EventMask};
use x11rb::xcb_ffi::XCBConnection;

use super::call_window::*;

const NET_WM_STATE_REMOVE: u32 = 0;
const NET_WM_STATE_ADD: u32 = 1;
const SOURCE_APPLICATION: u32 = 1;

const WAYLAND_ABOVE_LIMIT: &str =
    "Wayland lets no client keep a window above the others without layer-shell";

pub(super) struct NativeCallWindow;

impl CallWindowLayer for NativeCallWindow {
    fn keep_above(&self, window: &gpui::Window) -> Result<(), CallWindowUnavailable> {
        match surface_of(window)? {
            Surface::X11 { conn, x_window } => {
                change_state(
                    conn,
                    x_window,
                    NET_WM_STATE_ADD,
                    b"_NET_WM_STATE_ABOVE",
                    Some(b"_NET_WM_STATE_SKIP_TASKBAR"),
                )?;
                change_state(conn, x_window, NET_WM_STATE_ADD, b"_NET_WM_STATE_STICKY", None)
            }
            Surface::Wayland => Err(CallWindowUnavailable::WindowSystemLimit(
                WAYLAND_ABOVE_LIMIT,
            )),
        }
    }

    fn request_attention(
        &self,
        window: &gpui::Window,
        level: AttentionLevel,
    ) -> Result<AttentionRequest, CallWindowUnavailable> {
        let _ = level;
        match surface_of(window)? {
            Surface::X11 { conn, x_window } => {
                change_state(
                    conn,
                    x_window,
                    NET_WM_STATE_ADD,
                    b"_NET_WM_STATE_DEMANDS_ATTENTION",
                    None,
                )?;
                Ok(AttentionRequest::new(move || {
                    if let Err(failure) = change_state(
                        conn,
                        x_window,
                        NET_WM_STATE_REMOVE,
                        b"_NET_WM_STATE_DEMANDS_ATTENTION",
                        None,
                    ) {
                        tracing::debug!(target: "riseonly", %failure, "the X11 attention state stays until focus");
                    }
                }))
            }
            Surface::Wayland => {
                window.activate_window();
                Ok(AttentionRequest::new(|| {}))
            }
        }
    }
}

enum Surface {
    X11 {
        conn: &'static XCBConnection,
        x_window: u32,
    },
    Wayland,
}

fn surface_of(window: &gpui::Window) -> Result<Surface, CallWindowUnavailable> {
    let display = HasDisplayHandle::display_handle(window)
        .map_err(|error| CallWindowUnavailable::NoNativeWindow(error.to_string()))?;
    let surface = HasWindowHandle::window_handle(window)
        .map_err(|error| CallWindowUnavailable::NoNativeWindow(error.to_string()))?;
    match (display.as_raw(), surface.as_raw()) {
        (RawDisplayHandle::Xcb(xcb), RawWindowHandle::Xcb(x_window)) => {
            let connection = xcb.connection.ok_or_else(|| {
                CallWindowUnavailable::NoNativeWindow("no xcb connection".to_owned())
            })?;
            Ok(Surface::X11 {
                conn: xcb_guest(connection)?,
                x_window: x_window.window.get(),
            })
        }
        (RawDisplayHandle::Wayland(_), RawWindowHandle::Wayland(_)) => Ok(Surface::Wayland),
        _ => Err(CallWindowUnavailable::NoNativeWindow(
            "neither an X11 nor a Wayland window".to_owned(),
        )),
    }
}

thread_local! {
    static XCB_GUEST: Cell<Option<(usize, &'static XCBConnection)>> = const { Cell::new(None) };
}

fn xcb_guest(connection: NonNull<c_void>) -> Result<&'static XCBConnection, CallWindowUnavailable> {
    let key = connection.as_ptr() as usize;
    if let Some((known, guest)) = XCB_GUEST.with(Cell::get) {
        if known == key {
            return Ok(guest);
        }
    }
    let wrapped = unsafe { XCBConnection::from_raw_xcb_connection(connection.as_ptr(), false) }
        .map_err(|error| CallWindowUnavailable::SystemRefused(error.to_string()))?;
    let guest: &'static XCBConnection = Box::leak(Box::new(wrapped));
    XCB_GUEST.with(|slot| slot.set(Some((key, guest))));
    Ok(guest)
}

fn atom(conn: &XCBConnection, name: &[u8]) -> Result<Atom, CallWindowUnavailable> {
    conn.intern_atom(false, name)
        .map_err(|error| CallWindowUnavailable::SystemRefused(error.to_string()))?
        .reply()
        .map(|reply| reply.atom)
        .map_err(|error| CallWindowUnavailable::SystemRefused(error.to_string()))
}

/// sends one net_wm_state change for the window to its root, as ewmh asks of a mapped window
fn change_state(
    conn: &XCBConnection,
    window: u32,
    action: u32,
    first: &[u8],
    second: Option<&[u8]>,
) -> Result<(), CallWindowUnavailable> {
    let refused =
        |error: &dyn std::fmt::Display| CallWindowUnavailable::SystemRefused(error.to_string());
    let root = conn
        .get_geometry(window)
        .map_err(|error| refused(&error))?
        .reply()
        .map_err(|error| refused(&error))?
        .root;
    let state = atom(conn, b"_NET_WM_STATE")?;
    let first = atom(conn, first)?;
    let second = match second {
        Some(name) => atom(conn, name)?,
        None => 0,
    };
    let event = ClientMessageEvent::new(
        32,
        window,
        state,
        [action, first, second, SOURCE_APPLICATION, 0],
    );
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    )
    .map_err(|error| refused(&error))?
    .check()
    .map_err(|error| refused(&error))?;
    conn.flush().map_err(|error| refused(&error))
}
