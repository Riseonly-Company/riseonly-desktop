use std::sync::Arc;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11rb::connection::Connection as _;
use x11rb::errors::{ConnectionError, ReplyError, ReplyOrIdError};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use super::window_minimise::*;
use super::x11_window_state::*;
use crate::host_os::HostOs;

pub(super) struct NativeWindowMinimise;

impl WindowMinimiseSource for NativeWindowMinimise {
    fn watch(
        &self,
        window: &dyn HasWindowHandle,
        sink: MinimisedSink,
    ) -> Result<MinimiseWatch, WindowMinimiseUnavailable> {
        let handle = window.window_handle().map_err(|error| no_window(&error.to_string()))?;
        let target: Window = match handle.as_raw() {
            RawWindowHandle::Xcb(xcb) => xcb.window.get(),
            RawWindowHandle::Xlib(xlib) => {
                Window::try_from(xlib.window).map_err(|_| no_window("the Xlib window id does not fit an X11 resource id"))?
            }
            RawWindowHandle::Wayland(_) => {
                return Err(no_window(
                    "wayland: xdg_toplevel tells clients no minimised state, the window counts as visible until it loses focus",
                ));
            }
            _ => return Err(no_window("the handle is neither an X11 nor a Wayland window")),
        };

        let watcher = Arc::new(Watcher::open(target).map_err(|detail| no_window(&detail))?);
        report(&sink, watcher.read_state());
        let thread_watcher = Arc::clone(&watcher);
        let thread_sink = Arc::clone(&sink);
        let started = std::thread::Builder::new()
            .name("rise-window-minimise-x11".into())
            .spawn(move || thread_watcher.listen(&thread_sink));
        if let Err(error) = started {
            watcher.stop();
            return Err(no_window(&format!("the X event thread did not start: {error}")));
        }
        Ok(MinimiseWatch::new(StopOnDrop(watcher)))
    }
}

fn no_window(detail: &str) -> WindowMinimiseUnavailable {
    WindowMinimiseUnavailable::NoNativeWindow {
        host: HostOs::Linux,
        detail: detail.to_owned(),
    }
}

/// hands one state to the sink, a panic in it is logged and never unwound into the x thread
fn report(sink: &MinimisedSink, minimised: bool) {
    let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(minimised)));
    if delivered.is_err() {
        tracing::error!(target: "riseonly", minimised, "window minimise handler panicked");
    }
}

struct Atoms {
    net_wm_state: u32,
    net_wm_state_hidden: u32,
    wm_state: u32,
    net_supported: u32,
}

impl Atoms {
    fn intern(connection: &RustConnection) -> Result<Self, ReplyError> {
        let net_wm_state = connection.intern_atom(false, b"_NET_WM_STATE")?;
        let net_wm_state_hidden = connection.intern_atom(false, b"_NET_WM_STATE_HIDDEN")?;
        let wm_state = connection.intern_atom(false, b"WM_STATE")?;
        let net_supported = connection.intern_atom(false, b"_NET_SUPPORTED")?;
        Ok(Self {
            net_wm_state: net_wm_state.reply()?.atom,
            net_wm_state_hidden: net_wm_state_hidden.reply()?.atom,
            wm_state: wm_state.reply()?.atom,
            net_supported: net_supported.reply()?.atom,
        })
    }
}

struct Watcher {
    connection: RustConnection,
    root: Window,
    target: Window,
    stopper: Window,
    atoms: Atoms,
}

impl Watcher {
    fn open(target: Window) -> Result<Self, String> {
        let (connection, screen) = x11rb::connect(None).map_err(|error| format!("no X connection: {error}"))?;
        let root = connection
            .setup()
            .roots
            .get(screen)
            .map(|screen| screen.root)
            .ok_or_else(|| "the X display has no such screen".to_owned())?;
        let atoms = Atoms::intern(&connection).map_err(|error| format!("the X atoms could not be interned: {error}"))?;

        let watched = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY);
        connection
            .change_window_attributes(target, &watched)
            .map_err(|error: ConnectionError| error.to_string())?
            .check()
            .map_err(|error| format!("the window does not accept our event mask: {error}"))?;

        let stopper = create_stopper(&connection, root).map_err(|error| format!("the stop window failed: {error}"))?;
        Ok(Self {
            connection,
            root,
            target,
            stopper,
            atoms,
        })
    }

    /// rereads the window state from the server, a failed read counts as visible
    fn read_state(&self) -> bool {
        let atoms = &self.atoms;
        let net_wm_state = self.atom_list(self.target, atoms.net_wm_state);
        let supported = self.atom_list(self.root, atoms.net_supported);
        let wm_state = self
            .connection
            .get_property(false, self.target, atoms.wm_state, AtomEnum::ANY, 0, 2)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut cards| cards.next()));
        X11WindowState {
            hidden_atom: atoms.net_wm_state_hidden,
            manager_supports_hidden: supported.contains(&atoms.net_wm_state_hidden),
            net_wm_state,
            wm_state,
        }
        .minimised()
    }

    fn atom_list(&self, window: Window, property: u32) -> Vec<u32> {
        self.connection
            .get_property(false, window, property, AtomEnum::ATOM, 0, 1024)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().map(|cards| cards.collect::<Vec<u32>>()))
            .unwrap_or_default()
    }

    fn listen(&self, sink: &MinimisedSink) {
        loop {
            let event = match self.connection.wait_for_event() {
                Ok(event) => event,
                Err(error) => {
                    tracing::warn!(target: "riseonly", %error, "window minimise: the X connection closed");
                    return;
                }
            };
            match event {
                Event::PropertyNotify(change)
                    if change.window == self.target
                        && is_state_property(change.atom, self.atoms.net_wm_state, self.atoms.wm_state) =>
                {
                    report(sink, self.read_state());
                }
                Event::DestroyNotify(gone) if gone.window == self.stopper || gone.window == self.target => return,
                Event::Error(error) => {
                    tracing::debug!(target: "riseonly", ?error, "window minimise: X error");
                }
                _ => {}
            }
        }
    }

    fn stop(&self) {
        let destroyed = self.connection.destroy_window(self.stopper).map(|_| ()).and_then(|()| self.connection.flush());
        if let Err(error) = destroyed {
            tracing::debug!(target: "riseonly", %error, "window minimise: the stop window was not destroyed");
        }
    }
}

fn create_stopper(connection: &RustConnection, root: Window) -> Result<Window, ReplyOrIdError> {
    let stopper = connection.generate_id()?;
    let aux = CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY);
    connection
        .create_window(0, stopper, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &aux)?
        .check()?;
    Ok(stopper)
}

struct StopOnDrop(Arc<Watcher>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}
