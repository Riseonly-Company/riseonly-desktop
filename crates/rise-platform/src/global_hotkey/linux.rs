use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use parking_lot::Mutex;
use x11rb::connection::Connection as _;
use x11rb::errors::{ConnectionError, ReplyError};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, Mapping, ModMask, Window};
use x11rb::rust_connection::RustConnection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::global_hotkey::{DisplayServer, Hotkey, HotkeyError, Modifiers};
use super::shortcut_routes::*;
use crate::host_os::HostOs;

const MAX_PENDING: usize = 64;
const PORTAL_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const REBIND_SETTLE: Duration = Duration::from_millis(150);

static PRESSED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static NEXT_SLOT: AtomicU32 = AtomicU32::new(1);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

static X11: OnceLock<Option<X11Grabber>> = OnceLock::new();
static X11_LISTENER: OnceLock<()> = OnceLock::new();
static PORTAL: OnceLock<Option<PortalWorker>> = OnceLock::new();
static WANTED: Mutex<Vec<(u32, PortalShortcut)>> = Mutex::new(Vec::new());

fn queue_press(action: String) {
    push_bounded(&mut PRESSED.lock(), action, MAX_PENDING);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    X11,
    Portal,
}

pub(super) struct Registration {
    action: String,
    slot: u32,
    route: Route,
}

impl Registration {
    pub(super) fn action(&self) -> &str {
        &self.action
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        match self.route {
            Route::X11 => {
                if let Some(Some(grabber)) = X11.get() {
                    grabber.release(self.slot);
                }
            }
            Route::Portal => {
                if let Some(Some(worker)) = PORTAL.get() {
                    worker.release(self.slot);
                }
            }
        }
    }
}

pub(super) fn register(action: &str, hotkey: Hotkey) -> Result<Option<Registration>, HotkeyError> {
    let route = match DisplayServer::detect() {
        DisplayServer::X11 => Route::X11,
        DisplayServer::Wayland => Route::Portal,
        DisplayServer::Unknown => return Ok(None),
    };
    let slot = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);

    let placed = match route {
        Route::X11 => match X11.get_or_init(X11Grabber::connect) {
            Some(grabber) => {
                grabber.grab(slot, action, hotkey)?;
                X11_LISTENER.get_or_init(|| grabber.start_listener());
                true
            }
            None => false,
        },
        Route::Portal => match PORTAL.get_or_init(PortalWorker::start) {
            Some(worker) => {
                worker.want(slot, PortalShortcut::new(action, hotkey));
                true
            }
            None => false,
        },
    };

    Ok(placed.then(|| Registration {
        action: action.to_owned(),
        slot,
        route,
    }))
}

pub(super) fn take_pressed() -> Vec<String> {
    std::mem::take(&mut *PRESSED.lock())
}

struct X11Grab {
    slot: u32,
    action: String,
    keysym: u32,
    modifiers: Modifiers,
    placed: Option<(u8, u16)>,
}

struct X11Keyboard {
    map: KeyboardMap,
    layout: ModifierLayout,
    grabs: Vec<X11Grab>,
    repeats: RepeatFilter,
}

struct X11Grabber {
    connection: RustConnection,
    root: Window,
    keyboard: Mutex<X11Keyboard>,
}

fn reply_code(error: ReplyError) -> i32 {
    match error {
        ReplyError::X11Error(error) => i32::from(error.error_code),
        ReplyError::ConnectionError(_) => CONNECTION_LOST,
    }
}

fn connection_code(_error: ConnectionError) -> i32 {
    CONNECTION_LOST
}

fn read_keyboard(connection: &RustConnection) -> Result<(KeyboardMap, ModifierLayout), ReplyError> {
    let setup = connection.setup();
    let first = setup.min_keycode;
    let count = setup.max_keycode.saturating_sub(first).saturating_add(1);

    let keyboard = connection.get_keyboard_mapping(first, count)?.reply()?;
    let map = KeyboardMap::new(first, keyboard.keysyms_per_keycode, keyboard.keysyms);
    let modifiers = connection.get_modifier_mapping()?.reply()?;
    let per_modifier = usize::from(modifiers.keycodes_per_modifier());
    let layout = ModifierLayout::from_mapping(per_modifier, &modifiers.keycodes, &map);
    Ok((map, layout))
}

impl X11Grabber {
    fn connect() -> Option<Self> {
        let (connection, screen) = match x11rb::connect(None) {
            Ok(opened) => opened,
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "global hotkeys: no X connection, chords stay in-window");
                return None;
            }
        };
        let root = connection.setup().roots.get(screen)?.root;
        let (map, layout) = match read_keyboard(&connection) {
            Ok(keyboard) => keyboard,
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "global hotkeys: the X keymap could not be read");
                return None;
            }
        };

        Some(Self {
            connection,
            root,
            keyboard: Mutex::new(X11Keyboard {
                map,
                layout,
                grabs: Vec::new(),
                repeats: RepeatFilter::default(),
            }),
        })
    }

    fn start_listener(&'static self) {
        let started = std::thread::Builder::new()
            .name("rise-hotkey-x11".into())
            .spawn(move || self.listen());
        if let Err(error) = started {
            tracing::warn!(target: "riseonly", %error, "global hotkeys: the X event thread did not start");
        }
    }

    fn grab(&self, slot: u32, action: &str, hotkey: Hotkey) -> Result<(), HotkeyError> {
        let keysym = keysym_of(hotkey);
        let mut keyboard = self.keyboard.lock();
        let refused = |code| HotkeyError::Refused {
            chord: hotkey.display(HostOs::Linux),
            code,
        };

        let keycode = keyboard.map.keycode_for(keysym).ok_or_else(|| refused(KEYSYM_NOT_ON_KEYBOARD))?;
        let base = keyboard.layout.mask(hotkey.modifiers);
        if let Err(code) = self.grab_all(keycode, base, keyboard.layout) {
            self.ungrab_all(keycode, base, keyboard.layout);
            return Err(refused(code));
        }

        keyboard.grabs.push(X11Grab {
            slot,
            action: action.to_owned(),
            keysym,
            modifiers: hotkey.modifiers,
            placed: Some((keycode, base)),
        });
        Ok(())
    }

    fn grab_all(&self, keycode: u8, base: u16, layout: ModifierLayout) -> Result<(), i32> {
        for mask in layout.grab_masks(base) {
            self.connection
                .grab_key(false, self.root, ModMask::from(mask), keycode, GrabMode::ASYNC, GrabMode::ASYNC)
                .map_err(connection_code)?
                .check()
                .map_err(reply_code)?;
        }
        Ok(())
    }

    fn ungrab_all(&self, keycode: u8, base: u16, layout: ModifierLayout) {
        for mask in layout.grab_masks(base) {
            if let Err(error) = self.connection.ungrab_key(keycode, self.root, ModMask::from(mask)) {
                tracing::warn!(target: "riseonly", %error, "global hotkeys: an X ungrab was not sent");
                return;
            }
        }
        if let Err(error) = self.connection.flush() {
            tracing::warn!(target: "riseonly", %error, "global hotkeys: the X ungrab was not flushed");
        }
    }

    fn release(&self, slot: u32) {
        let mut keyboard = self.keyboard.lock();
        let Some(index) = keyboard.grabs.iter().position(|grab| grab.slot == slot) else {
            return;
        };
        let grab = keyboard.grabs.remove(index);
        if let Some((keycode, base)) = grab.placed {
            self.ungrab_all(keycode, base, keyboard.layout);
        }
    }

    fn listen(&self) {
        loop {
            let event = match self.connection.wait_for_event() {
                Ok(event) => event,
                Err(error) => {
                    tracing::warn!(target: "riseonly", %error, "global hotkeys: the X connection closed");
                    return;
                }
            };

            match event {
                Event::KeyPress(press) => {
                    let mut keyboard = self.keyboard.lock();
                    if !keyboard.repeats.pressed(press.detail, press.time) {
                        continue;
                    }
                    let state = u16::from(press.state);
                    let layout = keyboard.layout;
                    let action = keyboard
                        .grabs
                        .iter()
                        .find(|grab| {
                            grab.placed.is_some_and(|(keycode, base)| {
                                keycode == press.detail && layout.matches(state, base)
                            })
                        })
                        .map(|grab| grab.action.clone());
                    drop(keyboard);
                    if let Some(action) = action {
                        queue_press(action);
                    }
                }
                Event::KeyRelease(release) => {
                    self.keyboard.lock().repeats.released(release.detail, release.time);
                }
                Event::MappingNotify(notify) if notify.request != Mapping::POINTER => self.reload(),
                _ => {}
            }
        }
    }

    fn reload(&self) {
        let (map, layout) = match read_keyboard(&self.connection) {
            Ok(keyboard) => keyboard,
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "global hotkeys: the new X keymap could not be read");
                return;
            }
        };

        let mut keyboard = self.keyboard.lock();
        let old_layout = keyboard.layout;
        for placed in keyboard.grabs.iter_mut().filter_map(|grab| grab.placed.take()) {
            self.ungrab_all(placed.0, placed.1, old_layout);
        }
        keyboard.map = map;
        keyboard.layout = layout;

        let mut moved = Vec::with_capacity(keyboard.grabs.len());
        for grab in &keyboard.grabs {
            let base = layout.mask(grab.modifiers);
            let placed = match keyboard.map.keycode_for(grab.keysym) {
                Some(keycode) => match self.grab_all(keycode, base, layout) {
                    Ok(()) => Some((keycode, base)),
                    Err(code) => {
                        self.ungrab_all(keycode, base, layout);
                        tracing::warn!(target: "riseonly", action = %grab.action, code, "global hotkeys: the chord was not grabbed again after a keymap change");
                        None
                    }
                },
                None => None,
            };
            moved.push(placed);
        }
        for (grab, placed) in keyboard.grabs.iter_mut().zip(moved) {
            grab.placed = placed;
        }
    }
}

struct PortalWorker {
    changes: UnboundedSender<()>,
}

impl PortalWorker {
    fn start() -> Option<Self> {
        let (changes, receiver) = unbounded();
        let (ready, answer) = mpsc::sync_channel(1);
        let started = std::thread::Builder::new().name("rise-hotkey-portal".into()).spawn(move || {
            zbus::block_on(run_portal(receiver, ready));
        });
        if let Err(error) = started {
            tracing::warn!(target: "riseonly", %error, "global hotkeys: the portal thread did not start");
            return None;
        }

        match answer.recv_timeout(PORTAL_PROBE_TIMEOUT) {
            Ok(Ok(())) => Some(Self { changes }),
            Ok(Err(detail)) => {
                tracing::info!(target: "riseonly", %detail, "global hotkeys: no GlobalShortcuts portal, chords stay in-window");
                None
            }
            Err(_) => {
                tracing::warn!(target: "riseonly", "global hotkeys: the GlobalShortcuts portal did not answer in time");
                None
            }
        }
    }

    fn want(&self, slot: u32, shortcut: PortalShortcut) {
        WANTED.lock().push((slot, shortcut));
        self.changed();
    }

    fn release(&self, slot: u32) {
        WANTED.lock().retain(|(existing, _)| *existing != slot);
        self.changed();
    }

    fn changed(&self) {
        if self.changes.unbounded_send(()).is_err() {
            tracing::warn!(target: "riseonly", "global hotkeys: the portal thread has stopped");
        }
    }
}

fn wanted_shortcuts() -> Vec<PortalShortcut> {
    WANTED.lock().iter().map(|(_, shortcut)| shortcut.clone()).collect()
}

async fn run_portal(mut changes: UnboundedReceiver<()>, ready: mpsc::SyncSender<Result<(), String>>) {
    let opened = open_portal().await;
    let (connection, shortcuts) = match opened {
        Ok(parts) => parts,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    let mut activated = match shortcuts.receive_signal(ACTIVATED_SIGNAL).await {
        Ok(stream) => stream,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }

    let mut session: Option<String> = None;
    loop {
        tokio::select! {
            change = changes.next() => {
                if change.is_none() {
                    break;
                }
                tokio::time::sleep(REBIND_SETTLE).await;
                while changes.try_recv().is_ok() {}
                session = rebind(&connection, &shortcuts, session.take(), &wanted_shortcuts()).await;
            }
            signal = activated.next() => {
                let Some(message) = signal else {
                    tracing::warn!(target: "riseonly", "global hotkeys: the portal stopped sending Activated");
                    break;
                };
                type Activated = (OwnedObjectPath, String, u64, HashMap<String, OwnedValue>);
                match message.body().deserialize::<Activated>() {
                    Ok((path, shortcut_id, _timestamp, _options)) => {
                        let action =
                            activated_action(session.as_deref(), path.as_str(), &shortcut_id, &wanted_shortcuts());
                        if let Some(action) = action {
                            queue_press(action);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(target: "riseonly", %error, "global hotkeys: an Activated signal did not parse");
                    }
                }
            }
        }
    }

    if let Some(path) = session {
        close_session(&connection, &path).await;
    }
}

async fn open_portal() -> zbus::Result<(zbus::Connection, zbus::Proxy<'static>)> {
    let connection = zbus::Connection::session().await?;
    let shortcuts =
        zbus::Proxy::new(&connection, PORTAL_BUS_NAME, PORTAL_OBJECT_PATH, GLOBAL_SHORTCUTS_INTERFACE)
            .await?;
    let version: u32 = shortcuts.get_property(VERSION_PROPERTY).await?;
    tracing::debug!(target: "riseonly", version, "global hotkeys: GlobalShortcuts portal found");
    Ok((connection, shortcuts))
}

async fn rebind(
    connection: &zbus::Connection,
    shortcuts: &zbus::Proxy<'static>,
    previous: Option<String>,
    wanted: &[PortalShortcut],
) -> Option<String> {
    if let Some(path) = previous {
        close_session(connection, &path).await;
    }
    if wanted.is_empty() {
        return None;
    }

    let session = match create_session(connection, shortcuts).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(target: "riseonly", %error, "global hotkeys: the portal session was not created");
            return None;
        }
    };
    match bind_shortcuts(connection, shortcuts, &session, wanted).await {
        Ok(PortalAnswer::Accepted) => {
            tracing::debug!(target: "riseonly", count = wanted.len(), "global hotkeys: the portal bound the chords");
        }
        Ok(answer) => {
            tracing::info!(target: "riseonly", ?answer, "global hotkeys: the portal did not bind the chords");
        }
        Err(error) => {
            tracing::warn!(target: "riseonly", %error, "global hotkeys: BindShortcuts failed");
        }
    }
    Some(session)
}

async fn response_stream(
    connection: &zbus::Connection,
    path: String,
) -> zbus::Result<zbus::proxy::SignalStream<'static>> {
    zbus::Proxy::new(connection, PORTAL_BUS_NAME, path, REQUEST_INTERFACE)
        .await?
        .receive_signal(RESPONSE_SIGNAL)
        .await
}

fn unique_name(connection: &zbus::Connection) -> String {
    connection.unique_name().map(|name| name.as_str().to_owned()).unwrap_or_default()
}

async fn settle_request(
    connection: &zbus::Connection,
    expected: &str,
    handle: OwnedObjectPath,
    responses: zbus::proxy::SignalStream<'static>,
) -> zbus::Result<(u32, HashMap<String, OwnedValue>)> {
    let mut responses = responses;
    if handle.as_str() != expected {
        responses = response_stream(connection, handle.as_str().to_owned()).await?;
    }
    let message = responses.next().await.ok_or_else(|| {
        zbus::Error::Failure("the GlobalShortcuts request closed without a response".to_owned())
    })?;
    message.body().deserialize()
}

async fn create_session(
    connection: &zbus::Connection,
    shortcuts: &zbus::Proxy<'static>,
) -> zbus::Result<String> {
    let serial = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    let token = request_token(serial);
    let expected = request_path(&unique_name(connection), &token);
    let responses = response_stream(connection, expected.clone()).await?;

    let options: HashMap<&str, Value<'_>> = HashMap::from([
        (HANDLE_TOKEN_OPTION, Value::from(token.as_str())),
        (SESSION_HANDLE_TOKEN_OPTION, Value::from(session_token(serial))),
    ]);
    let handle: OwnedObjectPath = shortcuts.call(CREATE_SESSION_METHOD, &(options,)).await?;
    let (code, results) = settle_request(connection, &expected, handle, responses).await?;

    if portal_answer(code) != PortalAnswer::Accepted {
        return Err(zbus::Error::Failure(format!("CreateSession answered {code}")));
    }
    match results.get(SESSION_HANDLE_RESULT).map(|value| &**value) {
        Some(Value::Str(text)) => Ok(text.as_str().to_owned()),
        Some(Value::ObjectPath(path)) => Ok(path.as_str().to_owned()),
        _ => Err(zbus::Error::Failure("CreateSession returned no session_handle".to_owned())),
    }
}

async fn bind_shortcuts(
    connection: &zbus::Connection,
    shortcuts: &zbus::Proxy<'static>,
    session: &str,
    wanted: &[PortalShortcut],
) -> zbus::Result<PortalAnswer> {
    let token = request_token(NEXT_TOKEN.fetch_add(1, Ordering::Relaxed));
    let expected = request_path(&unique_name(connection), &token);
    let responses = response_stream(connection, expected.clone()).await?;

    let session_path = ObjectPath::try_from(session)?;
    let list: Vec<(&str, HashMap<&str, Value<'_>>)> = wanted
        .iter()
        .map(|shortcut| {
            let options = HashMap::from([
                (DESCRIPTION_OPTION, Value::from(shortcut.description.as_str())),
                (PREFERRED_TRIGGER_OPTION, Value::from(shortcut.preferred_trigger.as_str())),
            ]);
            (shortcut.id.as_str(), options)
        })
        .collect();
    let options: HashMap<&str, Value<'_>> =
        HashMap::from([(HANDLE_TOKEN_OPTION, Value::from(token.as_str()))]);

    let handle: OwnedObjectPath =
        shortcuts.call(BIND_SHORTCUTS_METHOD, &(session_path, list, "", options)).await?;
    let (code, _results) = settle_request(connection, &expected, handle, responses).await?;
    Ok(portal_answer(code))
}

async fn close_session(connection: &zbus::Connection, path: &str) {
    let closed = async {
        let session = zbus::Proxy::new(connection, PORTAL_BUS_NAME, path, SESSION_INTERFACE).await?;
        let () = session.call(CLOSE_METHOD, &()).await?;
        Ok::<(), zbus::Error>(())
    };
    if let Err(error) = closed.await {
        tracing::debug!(target: "riseonly", %error, "global hotkeys: the old portal session did not close");
    }
}
