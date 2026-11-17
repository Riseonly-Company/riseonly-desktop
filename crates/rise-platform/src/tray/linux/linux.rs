use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Poll, Waker};

use futures::channel::mpsc;
use futures::future::{select, Either};
use futures::StreamExt;
use parking_lot::Mutex;
use zbus::fdo::DBusProxy;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, StructureBuilder, Value};

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::tray::dbus_menu::*;
use crate::tray::*;

const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";

const ITEM_ID: &str = "riseonly";
const ICON_NAME: &str = "riseonly";
const CATEGORY: &str = "Communications";

/// pixmap row of the spec, width and height then ARGB32 in network byte order
type Pixmap = (i32, i32, Vec<u8>);

/// what the host should show plus which of it the service still has to announce
struct ItemState {
    title: String,
    base_tooltip: String,
    count: u32,
    visible: bool,
    tooltip_dirty: bool,
    status_dirty: bool,
    menu: Vec<TrayMenuItem>,
    revision: u32,
    layout_dirty: bool,
    closed: bool,
    waker: Option<Waker>,
}

impl ItemState {
    fn tooltip(&self) -> String {
        tray_tooltip(HostOs::Linux, &self.base_tooltip, self.count)
    }
}

struct Shared {
    state: Mutex<ItemState>,
    no_host: AtomicBool,
    events: mpsc::UnboundedSender<TrayEvent>,
    pending_events: Mutex<Option<mpsc::UnboundedReceiver<TrayEvent>>>,
}

impl Shared {
    fn new(config: &TrayConfig) -> Self {
        let (events, pending_events) = mpsc::unbounded();
        Self {
            state: Mutex::new(ItemState {
                title: config.label.clone(),
                base_tooltip: config.tooltip.clone(),
                count: 0,
                visible: true,
                tooltip_dirty: false,
                status_dirty: false,
                menu: config.menu_items.clone().unwrap_or_default(),
                revision: 1,
                layout_dirty: false,
                closed: false,
                waker: None,
            }),
            no_host: AtomicBool::new(false),
            events,
            pending_events: Mutex::new(Some(pending_events)),
        }
    }

    /// queue lives as long as this value, no reader yet means the app hasnt asked
    fn send_event(&self, event: TrayEvent) {
        self.events.unbounded_send(event).ok();
    }

    /// applies change and wakes the service, waker runs after the lock is released
    fn update(&self, change: impl FnOnce(&mut ItemState)) {
        let waker = {
            let mut state = self.state.lock();
            change(&mut state);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// what the service has to announce after a wake up
#[derive(Debug, PartialEq, Eq)]
struct Announce {
    tooltip: bool,
    status: Option<&'static str>,
    layout: Option<u32>,
}

/// takes the pending announcements, None once the tray is gone and Pending when nothing changed
fn take_announce(state: &mut ItemState) -> Option<Option<Announce>> {
    if state.closed {
        return None;
    }
    if !state.tooltip_dirty && !state.status_dirty && !state.layout_dirty {
        return Some(None);
    }
    let tooltip = std::mem::take(&mut state.tooltip_dirty);
    let status = std::mem::take(&mut state.status_dirty).then(|| status_notifier_status(state.visible));
    let layout = std::mem::take(&mut state.layout_dirty).then_some(state.revision);
    Some(Some(Announce {
        tooltip,
        status,
        layout,
    }))
}

async fn next_announce(shared: &Shared) -> Option<Announce> {
    std::future::poll_fn(|cx| {
        let mut state = shared.state.lock();
        match take_announce(&mut state) {
            None => Poll::Ready(None),
            Some(Some(announce)) => Poll::Ready(Some(announce)),
            Some(None) => {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    })
    .await
}

/// the exported object, every getter reads the shared state so the host sees the latest
struct StatusNotifierItem {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItem {
    async fn activate(&self, _x: i32, _y: i32) {
        self.shared.send_event(TrayEvent::Activate);
    }

    async fn secondary_activate(&self, _x: i32, _y: i32) {}

    async fn context_menu(&self, _x: i32, _y: i32) {
        self.shared.send_event(TrayEvent::Activate);
    }

    async fn scroll(&self, _delta: i32, _orientation: String) {}

    #[zbus(property)]
    fn category(&self) -> String {
        CATEGORY.to_owned()
    }

    #[zbus(property)]
    fn id(&self) -> String {
        ITEM_ID.to_owned()
    }

    #[zbus(property)]
    fn title(&self) -> String {
        self.shared.state.lock().title.clone()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        status_notifier_status(self.shared.state.lock().visible).to_owned()
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        ICON_NAME.to_owned()
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<Pixmap> {
        Vec::new()
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<Pixmap>, String, String) {
        let state = self.shared.state.lock();
        (String::new(), Vec::new(), state.tooltip(), String::new())
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        ObjectPath::from_static_str_unchecked(DBUS_MENU_PATH).into()
    }

    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
}

/// a{sv} of one node, strings and numbers always make an owned value and only a descriptor cant
fn wire_properties(properties: &MenuProperties, names: &[String]) -> HashMap<String, OwnedValue> {
    properties
        .iter()
        .filter(|(name, _)| menu_property_wanted(name, names))
        .filter_map(|(name, property)| {
            let value = match property {
                MenuProperty::Text(text) => Value::from(text.as_str()),
                MenuProperty::Number(number) => Value::from(*number),
            };
            Some(((*name).to_owned(), OwnedValue::try_from(value).ok()?))
        })
        .collect()
}

fn wire_child(node: &MenuNode, names: &[String]) -> Option<OwnedValue> {
    let structure = StructureBuilder::new()
        .add_field(node.id)
        .add_field(wire_properties(&node.properties, names))
        .add_field(Vec::<OwnedValue>::new())
        .build()
        .ok()?;
    OwnedValue::try_from(Value::Structure(structure)).ok()
}

/// the exported menu, every call reads the shared state like the item does
struct DbusMenu {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))> {
        let state = self.shared.state.lock();
        let node = menu_layout(&state.menu, parent_id, recursion_depth)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no menu item {parent_id}")))?;
        let children = node
            .children
            .iter()
            .filter_map(|child| wire_child(child, &property_names))
            .collect();
        let properties = wire_properties(&node.properties, &property_names);
        Ok((state.revision, (node.id, properties, children)))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let state = self.shared.state.lock();
        menu_group_ids(&state.menu, &ids)
            .into_iter()
            .filter_map(|id| {
                let properties = menu_node_properties(&state.menu, id)?;
                Some((id, wire_properties(&properties, &property_names)))
            })
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
        let state = self.shared.state.lock();
        menu_node_properties(&state.menu, id)
            .map(|properties| wire_properties(&properties, std::slice::from_ref(&name)))
            .and_then(|mut properties| properties.remove(&name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name} on {id}")))
    }

    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        let event = menu_event(&self.shared.state.lock().menu, id, &event_id);
        if let Some(event) = event {
            self.shared.send_event(event);
        }
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, event_id, data, timestamp);
        }
        Vec::new()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        DBUS_MENU_VERSION
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        DBUS_MENU_TEXT_DIRECTION.to_owned()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        DBUS_MENU_STATUS.to_owned()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

async fn serve(shared: Arc<Shared>) {
    if let Err(error) = run(&shared).await {
        shared.no_host.store(true, Ordering::Release);
        tracing::warn!(%error, "tray: StatusNotifierItem service stopped");
    }
}

async fn run(shared: &Arc<Shared>) -> zbus::Result<()> {
    let name = status_notifier_bus_name(std::process::id(), 1);
    let item = StatusNotifierItem {
        shared: Arc::clone(shared),
    };
    let menu = DbusMenu {
        shared: Arc::clone(shared),
    };
    let connection = zbus::connection::Builder::session()?
        .serve_at(STATUS_NOTIFIER_ITEM_PATH, item)?
        .serve_at(DBUS_MENU_PATH, menu)?
        .name(name.as_str())?
        .build()
        .await?;
    let dbus = DBusProxy::new(&connection).await?;
    let mut owners = dbus
        .receive_name_owner_changed_with_args(&[(0, WATCHER_SERVICE)])
        .await?;
    let emitter = SignalEmitter::new(&connection, STATUS_NOTIFIER_ITEM_PATH)?;
    let menu_emitter = SignalEmitter::new(&connection, DBUS_MENU_PATH)?;
    on_watcher_event(shared, &connection, &name, StatusNotifierWatcherEvent::WatcherAppeared).await;

    loop {
        let announce = std::pin::pin!(next_announce(shared));
        let owner = std::pin::pin!(owners.next());
        match select(announce, owner).await {
            Either::Left((None, _)) => return Ok(()),
            Either::Left((Some(announce), _)) => {
                if announce.tooltip {
                    StatusNotifierItem::new_tool_tip(&emitter).await?;
                }
                if let Some(status) = announce.status {
                    StatusNotifierItem::new_status(&emitter, status).await?;
                }
                if let Some(revision) = announce.layout {
                    DbusMenu::layout_updated(&menu_emitter, revision, DBUS_MENU_ROOT_ID).await?;
                }
            }
            Either::Right((Some(changed), _)) => {
                let present = changed
                    .args()
                    .map(|args| args.new_owner().is_some())
                    .unwrap_or(false);
                let event = status_notifier_owner_event(present);
                on_watcher_event(shared, &connection, &name, event).await;
            }
            Either::Right((None, _)) => {
                return Err(zbus::Error::Failure("the session bus closed the signal stream".to_owned()));
            }
        }
    }
}

/// applies the seam decision for a watcher event
/// registers if told to then records if a host has the item
async fn on_watcher_event(
    shared: &Shared,
    connection: &zbus::Connection,
    name: &str,
    event: StatusNotifierWatcherEvent,
) {
    let mut step = status_notifier_step(event);
    if step.register_now {
        let outcome = connection
            .call_method(
                Some(WATCHER_SERVICE),
                WATCHER_PATH,
                Some(WATCHER_INTERFACE),
                "RegisterStatusNotifierItem",
                &(name,),
            )
            .await;
        let event = match outcome {
            Ok(_) => StatusNotifierWatcherEvent::Registered,
            Err(error) => {
                tracing::info!(%error, "tray: no StatusNotifierWatcher yet; waiting for one");
                StatusNotifierWatcherEvent::RegisterFailed
            }
        };
        step = status_notifier_step(event);
    }
    if let Some(registered) = step.registered {
        shared.no_host.store(!registered, Ordering::Release);
    }
}

pub(crate) struct LinuxTray {
    shared: Arc<Shared>,
    service: Option<TrayService>,
    handler: Rc<RefCell<Option<TrayEventHandler>>>,
}

impl LinuxTray {
    fn accepts(&self) -> bool {
        !self.shared.no_host.load(Ordering::Acquire)
    }

    /// change is kept either way so a watcher that shows up later gets the latest state
    fn answer(&self) -> PlatformSupport {
        if self.accepts() {
            PlatformSupport::Performed
        } else {
            PlatformSupport::Unsupported
        }
    }
}

impl Tray for LinuxTray {
    fn set_tooltip(&mut self, text: &str) -> PlatformSupport {
        self.shared.update(|state| {
            state.base_tooltip = text.to_owned();
            state.tooltip_dirty = true;
        });
        self.answer()
    }

    fn set_badge(&mut self, count: u32) -> PlatformSupport {
        self.shared.update(|state| {
            state.count = count;
            state.tooltip_dirty = true;
        });
        self.answer()
    }

    fn set_visible(&mut self, visible: bool) -> PlatformSupport {
        self.shared.update(|state| {
            state.visible = visible;
            state.status_dirty = true;
        });
        self.answer()
    }

    fn availability(&self) -> TrayAvailability {
        status_notifier_availability(self.accepts())
    }

    fn take_service(&mut self) -> Option<TrayService> {
        self.service.take()
    }

    fn set_menu(&mut self, items: Vec<TrayMenuItem>) -> PlatformSupport {
        if !tray_menu_is_valid(&items) {
            return PlatformSupport::Unsupported;
        }
        self.shared.update(|state| {
            state.menu = items;
            state.revision = state.revision.wrapping_add(1);
            state.layout_dirty = true;
        });
        self.answer()
    }

    fn set_event_handler(&mut self, handler: TrayEventHandler) -> Result<(), TrayError> {
        *self.handler.borrow_mut() = Some(handler);
        Ok(())
    }

    fn deliver_events(&mut self, app: &gpui::App) {
        let Some(mut events) = self.shared.pending_events.lock().take() else {
            return;
        };
        let handler = Rc::clone(&self.handler);
        app.spawn(async move |_| {
            while let Some(event) = events.next().await {
                let current = handler.borrow_mut().take();
                if let Some(mut current) = current {
                    current(event);
                    handler.borrow_mut().get_or_insert(current);
                }
            }
        })
        .detach();
    }
}

impl Drop for LinuxTray {
    fn drop(&mut self) {
        self.shared.update(|state| state.closed = true);
    }
}

/// builds the tray right away, icon shows once the caller spawns Tray take_service
pub(crate) fn open(config: TrayConfig) -> Result<Box<dyn Tray>, TrayError> {
    if !tray_menu_is_valid(config.menu_items.as_deref().unwrap_or_default()) {
        return Err(TrayError::Unsupported("the status icon menu has an invalid item id"));
    }
    let shared = Arc::new(Shared::new(&config));
    let service: TrayService = Box::pin(serve(Arc::clone(&shared)));
    Ok(Box::new(LinuxTray {
        shared,
        service: Some(service),
        handler: Rc::new(RefCell::new(None)),
    }))
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
