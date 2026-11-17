use std::cell::Cell;
use std::rc::{Rc, Weak};

use gpui::{
    App, Bounds, ElementId, IntoElement, ParentElement, Pixels, RenderOnce, Styled, Window, canvas,
};

/// serial that tells apart the backing elements of one probe and the probes of one parent
fn next_serial() -> u64 {
    thread_local! {
        static SERIAL: Cell<u64> = const { Cell::new(0) };
    }
    SERIAL.with(|serial| {
        let next = serial.get() + 1;
        serial.set(next);
        next
    })
}

struct ViewAnchorProbeHolder {
    serial: u64,
    frame: Bounds<Pixels>,
}

struct ViewAnchorProbeInner {
    serial: u64,
    view: Cell<Option<ViewAnchorProbeHolder>>,
}

/// one per subtree to locate, cloning shares the same probe
#[derive(Clone)]
pub struct ViewAnchorProbe {
    inner: Rc<ViewAnchorProbeInner>,
}

impl Default for ViewAnchorProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewAnchorProbe {
    pub fn new() -> Self {
        Self { inner: Rc::new(ViewAnchorProbeInner { serial: next_serial(), view: Cell::new(None) }) }
    }

    fn attach(&self, view: &ViewAnchorProbeBackingView, frame: Bounds<Pixels>) {
        self.inner.view.set(Some(ViewAnchorProbeHolder { serial: view.serial, frame }));
    }

    /// the outgoing branch leaves after the incoming one already claimed the probe
    /// only the current holder may release it
    fn detach(inner: &ViewAnchorProbeInner, serial: u64) {
        let holder = inner.view.take();
        if holder.as_ref().is_some_and(|holder| holder.serial != serial) {
            inner.view.set(holder);
        }
    }

    /// where the subtree was painted last in window coords
    /// none when not in a window or collapsed to a line
    pub fn window_frame(&self) -> Option<Bounds<Pixels>> {
        let holder = self.inner.view.take();
        let frame = holder.as_ref().map(|holder| holder.frame);
        self.inner.view.set(holder);
        let frame = frame?;
        (f32::from(frame.size.width) > 1.0 && f32::from(frame.size.height) > 1.0).then_some(frame)
    }
}

/// element state behind the reader, alive while the reader is drawn
pub struct ViewAnchorProbeBackingView {
    serial: u64,
    probe: Weak<ViewAnchorProbeInner>,
}

impl ViewAnchorProbeBackingView {
    fn did_move_to_window(&self, frame: Option<Bounds<Pixels>>) {
        let Some(probe) = self.probe.upgrade() else { return };
        match frame {
            Some(frame) => ViewAnchorProbe { inner: probe }.attach(self, frame),
            None => ViewAnchorProbe::detach(&probe, self.serial),
        }
    }
}

impl Drop for ViewAnchorProbeBackingView {
    fn drop(&mut self) {
        self.did_move_to_window(None);
    }
}

/// transparent layer over its parent that reports the parent painted frame to the probe
/// no hitbox so no clicks
#[derive(IntoElement)]
pub struct ViewAnchorProbeReader {
    probe: ViewAnchorProbe,
}

impl ViewAnchorProbeReader {
    pub fn new(probe: ViewAnchorProbe) -> Self {
        Self { probe }
    }

    fn make_ui_view(&self) -> ViewAnchorProbeBackingView {
        ViewAnchorProbeBackingView { serial: next_serial(), probe: Rc::downgrade(&self.probe.inner) }
    }

    fn update_ui_view(&self, ui_view: &mut ViewAnchorProbeBackingView, frame: Bounds<Pixels>) {
        if !std::ptr::eq(ui_view.probe.as_ptr(), Rc::as_ptr(&self.probe.inner)) {
            *ui_view = self.make_ui_view();
        }
        ui_view.did_move_to_window(Some(frame));
    }
}

impl RenderOnce for ViewAnchorProbeReader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = ElementId::NamedInteger("view-anchor-probe".into(), self.probe.inner.serial);
        let moved_probe = Rc::downgrade(&self.probe.inner);
        canvas(
            move |bounds, window, _cx| {
                let serial = window.with_global_id(id, |global_id, window| {
                    window.with_element_state::<ViewAnchorProbeBackingView, _>(global_id, |backing, _| {
                        let mut backing = backing.unwrap_or_else(|| self.make_ui_view());
                        self.update_ui_view(&mut backing, bounds);
                        (backing.serial, backing)
                    })
                });
                window.on_replayed_move(move |offset, _, _, _| {
                    let Some(inner) = moved_probe.upgrade() else { return };
                    let holder = inner.view.take();
                    let holder = match holder {
                        Some(holder) if holder.serial == serial => Some(ViewAnchorProbeHolder {
                            serial,
                            frame: Bounds::new(bounds.origin + offset, bounds.size),
                        }),
                        other => other,
                    };
                    inner.view.set(holder);
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}

/// adds the reader behind the element children
pub trait ViewAnchorProbeExt: ParentElement + Sized {
    fn view_anchor_probe(mut self, probe: &ViewAnchorProbe) -> Self {
        self.extend(std::iter::once(ViewAnchorProbeReader::new(probe.clone()).into_any_element()));
        self
    }
}

impl<T: ParentElement> ViewAnchorProbeExt for T {}
