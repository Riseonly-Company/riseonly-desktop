use std::fmt::Display;
use std::fs::File;
use std::io::Write as _;
use std::sync::mpsc;
use std::time::Duration;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_device_v1::{
    self, ExtDataControlDeviceV1,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1::ExtDataControlManagerV1;
use wayland_protocols::ext::data_control::v1::client::ext_data_control_offer_v1::ExtDataControlOfferV1;
use wayland_protocols::ext::data_control::v1::client::ext_data_control_source_v1::{
    self, ExtDataControlSourceV1,
};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_device_v1::{
    self, ZwlrDataControlDeviceV1,
};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1::ZwlrDataControlManagerV1;
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_offer_v1::ZwlrDataControlOfferV1;
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_source_v1::{
    self, ZwlrDataControlSourceV1,
};
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Time, Window,
    WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::clipboard_offer::*;
use super::pasteboard_write::*;

const OWNERSHIP_TIMEOUT: Duration = Duration::from_millis(250);

pub(super) struct NativePasteboard;

impl NativePasteboardWriter for NativePasteboard {
    fn write_host_only_text(&self, text: &str) -> Result<(), PasteboardWriteUnavailable> {
        own_clipboard(ClipboardOffer::host_only_text(text))
    }

    fn write_url(&self, absolute_string: &str) -> Result<(), PasteboardWriteUnavailable> {
        own_clipboard(ClipboardOffer::url(absolute_string))
    }
}

fn own_clipboard(offer: ClipboardOffer) -> Result<(), PasteboardWriteUnavailable> {
    let session = clipboard_session(
        std::env::var_os("ZED_HEADLESS").is_some(),
        std::env::var_os("WAYLAND_DISPLAY").is_some_and(|display| !display.is_empty()),
        std::env::var_os("DISPLAY").is_some_and(|display| !display.is_empty()),
    );
    if session == ClipboardSession::Unreachable {
        return Err(PasteboardWriteUnavailable::HostCannotWrite(NO_DISPLAY.to_owned()));
    }
    let (owned, ownership) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("rise-clipboard-owner".into())
        .spawn(move || match session {
            ClipboardSession::Wayland => serve_wayland(offer, owned),
            ClipboardSession::X11 => serve_x11(offer, owned),
            ClipboardSession::Unreachable => {}
        })
        .map_err(refused)?;
    ownership
        .recv_timeout(OWNERSHIP_TIMEOUT)
        .unwrap_or_else(|waited| Err(refused(waited)))
}

fn refused(error: impl Display) -> PasteboardWriteUnavailable {
    PasteboardWriteUnavailable::SystemRefused(error.to_string())
}

type Ownership = mpsc::SyncSender<Result<(), PasteboardWriteUnavailable>>;

struct DataControl {
    offer: ClipboardOffer,
    finished: bool,
}

impl DataControl {
    fn send(&self, mime_type: &str, fd: std::os::fd::OwnedFd) {
        let Some(bytes) = self.offer.payload(mime_type) else {
            return;
        };
        if let Err(error) = File::from(fd).write_all(bytes) {
            tracing::debug!(target: "riseonly", %error, mime_type, "a clipboard reader closed early");
        }
    }
}

enum DataControlSource {
    Ext(ExtDataControlSourceV1),
    Wlr(ZwlrDataControlSourceV1),
}

fn serve_wayland(offer: ClipboardOffer, owned: Ownership) {
    let connection = match Connection::connect_to_env() {
        Ok(connection) => connection,
        Err(error) => {
            let _ = owned.send(Err(refused(error)));
            return;
        }
    };
    let (globals, mut queue) = match registry_queue_init::<DataControl>(&connection) {
        Ok(initialized) => initialized,
        Err(error) => {
            let _ = owned.send(Err(refused(error)));
            return;
        }
    };
    let qh = queue.handle();
    let seat = match globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()) {
        Ok(seat) => seat,
        Err(error) => {
            let _ = owned.send(Err(refused(error)));
            return;
        }
    };
    let mut state = DataControl {
        offer,
        finished: false,
    };
    let source = if let Ok(manager) = globals.bind::<ExtDataControlManagerV1, _, _>(&qh, 1..=1, ())
    {
        let device = manager.get_data_device(&seat, &qh, ());
        let source = manager.create_data_source(&qh, ());
        for mime in state.offer.mime_types() {
            source.offer(mime.to_owned());
        }
        device.set_selection(Some(&source));
        DataControlSource::Ext(source)
    } else if let Ok(manager) = globals.bind::<ZwlrDataControlManagerV1, _, _>(&qh, 1..=1, ()) {
        let device = manager.get_data_device(&seat, &qh, ());
        let source = manager.create_data_source(&qh, ());
        for mime in state.offer.mime_types() {
            source.offer(mime.to_owned());
        }
        device.set_selection(Some(&source));
        DataControlSource::Wlr(source)
    } else {
        let _ = owned.send(Err(PasteboardWriteUnavailable::HostCannotWrite(
            NO_DATA_CONTROL.to_owned(),
        )));
        return;
    };
    if let Err(error) = queue.roundtrip(&mut state) {
        let _ = owned.send(Err(refused(error)));
        return;
    }
    if state.finished {
        let _ = owned.send(Err(refused("the compositor cancelled the data control source")));
        return;
    }
    let _ = owned.send(Ok(()));
    while !state.finished {
        if let Err(error) = queue.blocking_dispatch(&mut state) {
            tracing::debug!(target: "riseonly", %error, "the clipboard owner lost the compositor");
            break;
        }
    }
    match source {
        DataControlSource::Ext(source) => source.destroy(),
        DataControlSource::Wlr(source) => source.destroy(),
    }
    let _ = connection.flush();
}

impl Dispatch<WlRegistry, GlobalListContents> for DataControl {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for DataControl {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlManagerV1, ()> for DataControl {
    fn event(
        _: &mut Self,
        _: &ExtDataControlManagerV1,
        _: <ExtDataControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for DataControl {
    fn event(
        state: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::Selection { id: Some(offer) }
            | ext_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                offer.destroy();
            }
            ext_data_control_device_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(DataControl, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, ()> for DataControl {
    fn event(
        _: &mut Self,
        _: &ExtDataControlOfferV1,
        _: <ExtDataControlOfferV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlSourceV1, ()> for DataControl {
    fn event(
        state: &mut Self,
        _: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => state.send(&mime_type, fd),
            ext_data_control_source_v1::Event::Cancelled => state.finished = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for DataControl {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: <ZwlrDataControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for DataControl {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::Selection { id: Some(offer) }
            | zwlr_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                offer.destroy();
            }
            zwlr_data_control_device_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(DataControl, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for DataControl {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlOfferV1,
        _: <ZwlrDataControlOfferV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for DataControl {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => state.send(&mime_type, fd),
            zwlr_data_control_source_v1::Event::Cancelled => state.finished = true,
            _ => {}
        }
    }
}

struct SelectionOwner {
    connection: RustConnection,
    window: Window,
    clipboard: Atom,
    targets: Atom,
    offered: Vec<(Atom, &'static str)>,
    offer: ClipboardOffer,
}

fn serve_x11(offer: ClipboardOffer, owned: Ownership) {
    let owner = match take_selection(offer) {
        Ok(owner) => owner,
        Err(unavailable) => {
            let _ = owned.send(Err(unavailable));
            return;
        }
    };
    let _ = owned.send(Ok(()));
    loop {
        match owner.connection.wait_for_event() {
            Ok(Event::SelectionRequest(request)) => owner.answer(&request),
            Ok(Event::SelectionClear(clear)) if clear.selection == owner.clipboard => break,
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(target: "riseonly", %error, "the clipboard owner lost the X server");
                return;
            }
        }
    }
    let _ = owner.connection.destroy_window(owner.window);
    let _ = owner.connection.flush();
}

fn take_selection(offer: ClipboardOffer) -> Result<SelectionOwner, PasteboardWriteUnavailable> {
    let (connection, screen) = x11rb::connect(None).map_err(refused)?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .map(|screen| screen.root)
        .ok_or_else(|| refused("the X server has no such screen"))?;
    let window = connection.generate_id().map_err(refused)?;
    connection
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new(),
        )
        .map_err(refused)?;
    let clipboard = intern(&connection, "CLIPBOARD")?;
    let targets = intern(&connection, "TARGETS")?;
    let mut offered = Vec::new();
    for mime in offer.mime_types() {
        offered.push((intern(&connection, mime)?, mime));
    }
    connection
        .set_selection_owner(window, clipboard, Time::CURRENT_TIME)
        .map_err(refused)?;
    let owner = connection
        .get_selection_owner(clipboard)
        .map_err(refused)?
        .reply()
        .map_err(refused)?
        .owner;
    if owner != window {
        return Err(refused("another client kept the CLIPBOARD selection"));
    }
    Ok(SelectionOwner {
        connection,
        window,
        clipboard,
        targets,
        offered,
        offer,
    })
}

fn intern(connection: &RustConnection, name: &str) -> Result<Atom, PasteboardWriteUnavailable> {
    Ok(connection
        .intern_atom(false, name.as_bytes())
        .map_err(refused)?
        .reply()
        .map_err(refused)?
        .atom)
}

impl SelectionOwner {
    /// icccm 2.2, store the converted value on the requestor property then send SelectionNotify
    /// a refused conversion answers with property none
    fn answer(&self, request: &SelectionRequestEvent) {
        let property = if request.property == x11rb::NONE {
            request.target
        } else {
            request.property
        };
        let stored = if request.selection != self.clipboard {
            false
        } else if request.target == self.targets {
            let mut atoms = vec![self.targets];
            atoms.extend(self.offered.iter().map(|(atom, _)| *atom));
            self.connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &atoms,
                )
                .is_ok()
        } else {
            self.offered
                .iter()
                .find(|(atom, _)| *atom == request.target)
                .and_then(|(_, mime)| self.offer.payload(mime))
                .is_some_and(|bytes| {
                    self.connection
                        .change_property8(
                            PropMode::REPLACE,
                            request.requestor,
                            property,
                            request.target,
                            bytes,
                        )
                        .is_ok()
                })
        };
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if stored { property } else { x11rb::NONE },
        };
        let sent = self
            .connection
            .send_event(false, request.requestor, EventMask::NO_EVENT, notify)
            .is_ok();
        if !sent || self.connection.flush().is_err() {
            tracing::debug!(target: "riseonly", "the clipboard owner could not answer a reader");
        }
    }
}
