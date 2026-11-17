use std::cell::Cell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{
    AnyWindowHandle, App, Bounds, Context, ElementId, Entity, IntoElement, Pixels, Point, RenderOnce, Styled,
    Window, canvas,
};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementContentMode, SharedElementSurface,
};
use crate::core::shared_element_transition::runtime::shared_element_containers::SharedElementOriginContainer;

/// origin a flight over an overlay presentation lands back on
/// registered through the host probe and found with the coordinator origin lookup
pub trait SharedElementOverlayOrigin: SharedElementOriginContainer {
    fn overlay_window(&self, cx: &App) -> Option<AnyWindowHandle>;
    fn overlay_corner_radius(&self, item_id: &str, cx: &App) -> f32;
}

/// discrete presentation writes the overlay driver asks of its destination
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SharedElementOverlayPhase {
    OpenLanded,
    OpenFallback,
    OpenAborted,
    ExitBegan,
    ExitCancelled,
    ExitFinished,
}

/// an overlay presentation, eg the story player of one request
pub trait SharedElementOverlayDestination {
    fn surface(&self) -> SharedElementSurface;
    /// window rect of the media box at rest, none while its not in a window
    fn media_frame(&self, cx: &App) -> Option<Bounds<Pixels>>;
    fn open_landing_mode(&self, content_aspect: f32, clip: Bounds<Pixels>, cx: &App) -> SharedElementContentMode;
    fn current_item_id(&self, cx: &App) -> Option<String>;
    fn media_is_ready(&self, item_id: &str, cx: &App) -> bool;
    fn apply_overlay_phase(&self, phase: SharedElementOverlayPhase, epoch: u64, cx: &mut App);
}

struct SharedElementOverlayRegistryEntry {
    destination: Weak<dyn SharedElementOverlayDestination>,
}

/// weak destinations by surface token, overlays have no stack host for the probe registry
pub struct SharedElementOverlayRegistry {
    entries: HashMap<String, SharedElementOverlayRegistryEntry>,
}

impl SharedElementOverlayRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { entries: HashMap::new() }
    }

    pub fn register(&mut self, destination: &Rc<dyn SharedElementOverlayDestination>) {
        self.entries.retain(|_, entry| entry.destination.strong_count() > 0);
        self.entries.insert(
            destination.surface().token(),
            SharedElementOverlayRegistryEntry { destination: Rc::downgrade(destination) },
        );
    }

    pub fn unregister(&mut self, destination: &Rc<dyn SharedElementOverlayDestination>) {
        let token = destination.surface().token();
        let registered = self
            .entries
            .get(&token)
            .and_then(|entry| entry.destination.upgrade())
            .is_some_and(|current| std::ptr::addr_eq(Rc::as_ptr(&current), Rc::as_ptr(destination)));
        if !registered {
            return;
        }
        self.entries.remove(&token);
    }

    pub fn destination(&self, surface: &SharedElementSurface) -> Option<Rc<dyn SharedElementOverlayDestination>> {
        self.entries.get(&surface.token()).and_then(|entry| entry.destination.upgrade())
    }
}

struct SharedElementOverlayProbeHolder {
    serial: u64,
    origin: Point<Pixels>,
    window: AnyWindowHandle,
}

struct SharedElementOverlayProbeAnchorInner {
    serial: u64,
    view: Cell<Option<SharedElementOverlayProbeHolder>>,
}

fn next_probe_serial() -> u64 {
    thread_local! {
        static SERIAL: Cell<u64> = const { Cell::new(0) };
    }
    SERIAL.with(|serial| {
        let next = serial.get() + 1;
        serial.set(next);
        next
    })
}

/// weak ref to a mounted 0x0 probe, cloning shares the same anchor
#[derive(Clone)]
pub struct SharedElementOverlayProbeAnchor {
    inner: Rc<SharedElementOverlayProbeAnchorInner>,
}

impl Default for SharedElementOverlayProbeAnchor {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedElementOverlayProbeAnchor {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(SharedElementOverlayProbeAnchorInner {
                serial: next_probe_serial(),
                view: Cell::new(None),
            }),
        }
    }

    fn read<R>(&self, read: impl FnOnce(Option<&SharedElementOverlayProbeHolder>) -> R) -> R {
        let holder = self.inner.view.take();
        let result = read(holder.as_ref());
        self.inner.view.set(holder);
        result
    }

    /// window point of the probe, none while its not in a window
    pub fn screen_origin(&self) -> Option<Point<Pixels>> {
        self.read(|holder| holder.map(|holder| holder.origin))
    }

    pub fn window(&self) -> Option<AnyWindowHandle> {
        self.read(|holder| holder.map(|holder| holder.window))
    }

    /// only the current holder may release
    /// a remount claims the anchor in the same frame the old probe is dropped
    fn detach(inner: &SharedElementOverlayProbeAnchorInner, serial: u64) {
        let holder = inner.view.take();
        if holder.as_ref().is_some_and(|holder| holder.serial != serial) {
            inner.view.set(holder);
        }
    }
}

/// element state behind a mounted probe, alive while the probe is drawn
pub struct SharedElementOverlayProbeView {
    serial: u64,
    anchor: Weak<SharedElementOverlayProbeAnchorInner>,
}

impl SharedElementOverlayProbeView {
    fn new(anchor: &SharedElementOverlayProbeAnchor) -> Self {
        Self { serial: next_probe_serial(), anchor: Rc::downgrade(&anchor.inner) }
    }

    fn did_move_to_window(&self, placed: Option<(Point<Pixels>, AnyWindowHandle)>) {
        let Some(anchor) = self.anchor.upgrade() else { return };
        match placed {
            Some((origin, window)) => {
                anchor.view.set(Some(SharedElementOverlayProbeHolder { serial: self.serial, origin, window }));
            }
            None => SharedElementOverlayProbeAnchor::detach(&anchor, self.serial),
        }
    }
}

impl Drop for SharedElementOverlayProbeView {
    fn drop(&mut self) {
        self.did_move_to_window(None);
    }
}

/// zero size probe, mount it as a background child of the box it locates
/// it has no hitbox so it never takes clicks
#[derive(IntoElement)]
pub struct SharedElementOverlayProbe {
    pub anchor: SharedElementOverlayProbeAnchor,
}

impl SharedElementOverlayProbe {
    pub fn new(anchor: SharedElementOverlayProbeAnchor) -> Self {
        Self { anchor }
    }

    fn make_ui_view(&self) -> SharedElementOverlayProbeView {
        SharedElementOverlayProbeView::new(&self.anchor)
    }

    fn update_ui_view(&self, ui_view: &mut SharedElementOverlayProbeView, origin: Point<Pixels>, window: AnyWindowHandle) {
        if !std::ptr::eq(ui_view.anchor.as_ptr(), Rc::as_ptr(&self.anchor.inner)) {
            *ui_view = self.make_ui_view();
        }
        ui_view.did_move_to_window(Some((origin, window)));
    }
}

impl RenderOnce for SharedElementOverlayProbe {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = ElementId::NamedInteger("shared-element-overlay-probe".into(), self.anchor.inner.serial);
        canvas(
            move |bounds, window, _cx| {
                let handle = window.window_handle();
                window.with_global_id(id, |global_id, window| {
                    window.with_element_state::<SharedElementOverlayProbeView, _>(global_id, |view, _| {
                        let mut view = view.unwrap_or_else(|| self.make_ui_view());
                        self.update_ui_view(&mut view, bounds.origin, handle);
                        ((), view)
                    })
                })
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_0()
    }
}
