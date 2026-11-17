use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::{App, Bounds, ElementId, IntoElement, Pixels, RenderOnce, Styled, Window, canvas};
use rise_ui::swiftui::environment::{EnvKey, env};

use crate::core::shared_element_transition::runtime::shared_element_containers::{
    SharedElementDestinationContainer, SharedElementOriginContainer,
};
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;

/// stack page a container is hosted by
/// ids are unique per pushed page for the life of the app
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SharedElementStackHost {
    pub id: u64,
}

/// set by the navigation host around each stack page, none outside a stack
pub struct SharedElementStackHostKey;

impl EnvKey for SharedElementStackHostKey {
    type Value = Option<SharedElementStackHost>;

    fn default_value() -> Self::Value {
        None
    }
}

pub type SharedElementHostProbeLayout = Rc<dyn Fn(Bounds<Pixels>, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct SharedElementHostProbe {
    origin: Option<Rc<dyn SharedElementOriginContainer>>,
    destination: Option<Rc<dyn SharedElementDestinationContainer>>,
    on_layout: Option<SharedElementHostProbeLayout>,
}

impl SharedElementHostProbe {
    /// origin side of the host probe
    pub fn origin(container: Rc<dyn SharedElementOriginContainer>) -> Self {
        Self { origin: Some(container), destination: None, on_layout: None }
    }

    /// destination side of the host probe
    pub fn destination(container: Rc<dyn SharedElementDestinationContainer>) -> Self {
        Self { origin: None, destination: Some(container), on_layout: None }
    }

    pub fn on_layout(mut self, on_layout: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.on_layout = Some(Rc::new(on_layout));
        self
    }

    fn element_id(&self) -> ElementId {
        let address = match (&self.origin, &self.destination) {
            (Some(origin), _) => Rc::as_ptr(origin) as *const () as usize,
            (None, Some(destination)) => Rc::as_ptr(destination) as *const () as usize,
            (None, None) => 0,
        };
        ElementId::NamedInteger("shared-element-host-probe".into(), address as u64)
    }

    fn make_ui_view(&self) -> SharedElementHostProbeView {
        SharedElementHostProbeView::new()
    }

    fn update_ui_view(self, ui_view: &SharedElementHostProbeView, cx: &mut App) {
        ui_view.configure(self.origin, self.destination, self.on_layout, cx);
    }
}

impl RenderOnce for SharedElementHostProbe {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let id = self.element_id();
        canvas(
            move |bounds, window, cx| {
                let host = env::<SharedElementStackHostKey>(cx);
                let probe = self;
                let view = window.with_global_id(id, |global_id, window| {
                    window.with_element_state::<SharedElementHostProbeView, _>(global_id, |view, _| {
                        let view = view.unwrap_or_else(|| probe.make_ui_view());
                        (view.clone(), view)
                    })
                });
                view.host.set(host);
                let was_attached = view.inner.attached.replace(true);
                probe.update_ui_view(&view, cx);
                if was_attached {
                    view.layout_subviews(bounds, window, cx);
                } else {
                    view.did_move_to_window(bounds, window, cx);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_0()
    }
}

struct SharedElementHostProbeViewInner {
    origin: RefCell<Option<Weak<dyn SharedElementOriginContainer>>>,
    destination: RefCell<Option<Weak<dyn SharedElementDestinationContainer>>>,
    on_layout: RefCell<Option<SharedElementHostProbeLayout>>,
    registered_generation: Cell<Option<u64>>,
    attached: Cell<bool>,
}

/// element state behind a drawn probe
/// its owner is the identity the coordinator keeps registrations under
#[derive(Clone)]
pub struct SharedElementHostProbeView {
    inner: Rc<SharedElementHostProbeViewInner>,
    owner: Rc<dyn Any>,
    host: Rc<Cell<Option<SharedElementStackHost>>>,
}

impl SharedElementHostProbeView {
    fn new() -> Self {
        Self {
            inner: Rc::new(SharedElementHostProbeViewInner {
                origin: RefCell::new(None),
                destination: RefCell::new(None),
                on_layout: RefCell::new(None),
                registered_generation: Cell::new(None),
                attached: Cell::new(false),
            }),
            owner: Rc::new(()),
            host: Rc::new(Cell::new(None)),
        }
    }

    fn origin(&self) -> Option<Rc<dyn SharedElementOriginContainer>> {
        self.inner.origin.borrow().as_ref().and_then(Weak::upgrade)
    }

    fn destination(&self) -> Option<Rc<dyn SharedElementDestinationContainer>> {
        self.inner.destination.borrow().as_ref().and_then(Weak::upgrade)
    }

    fn did_move_to_window(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        self.register_with_host(cx);
        self.call_on_layout(bounds, window, cx);
    }

    fn layout_subviews(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let generation = SharedElementTransitionCoordinator::shared(cx).read(cx).registration_generation();
        if self.inner.registered_generation.get() != Some(generation) {
            self.register_with_host(cx);
        }
        self.call_on_layout(bounds, window, cx);
    }

    fn call_on_layout(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let on_layout = self.inner.on_layout.borrow().clone();
        if let Some(on_layout) = on_layout {
            on_layout(bounds, window, cx);
        }
    }

    pub(crate) fn configure(
        &self,
        new_origin: Option<Rc<dyn SharedElementOriginContainer>>,
        new_destination: Option<Rc<dyn SharedElementDestinationContainer>>,
        new_on_layout: Option<SharedElementHostProbeLayout>,
        cx: &mut App,
    ) {
        *self.inner.on_layout.borrow_mut() = new_on_layout;
        let same_origin = match (&new_origin, self.origin()) {
            (Some(new), Some(old)) => std::ptr::addr_eq(Rc::as_ptr(new), Rc::as_ptr(&old)),
            (None, None) => true,
            _ => false,
        };
        let same_destination = match (&new_destination, self.destination()) {
            (Some(new), Some(old)) => std::ptr::addr_eq(Rc::as_ptr(new), Rc::as_ptr(&old)),
            (None, None) => true,
            _ => false,
        };
        if same_origin && same_destination {
            return;
        }
        self.unregister_owned(cx);
        *self.inner.origin.borrow_mut() = new_origin.as_ref().map(Rc::downgrade);
        *self.inner.destination.borrow_mut() = new_destination.as_ref().map(Rc::downgrade);
        self.inner.registered_generation.set(None);
        if self.inner.attached.get() {
            self.register_with_host(cx);
        }
    }

    pub fn tear_down(&self, cx: &mut App) {
        self.unregister_owned(cx);
        *self.inner.origin.borrow_mut() = None;
        *self.inner.destination.borrow_mut() = None;
        *self.inner.on_layout.borrow_mut() = None;
        self.inner.registered_generation.set(None);
    }

    fn register_with_host(&self, cx: &mut App) {
        let Some(host) = self.enclosing_stack_host_controller() else { return };
        let origin = self.origin();
        let destination = self.destination();
        let owner = self.owner.clone();
        let generation = SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| {
            if let Some(origin) = &origin {
                coordinator.register_origin(origin, host, &owner, cx);
            }
            if let Some(destination) = &destination {
                coordinator.register_destination(destination, host, &owner, cx);
            }
            coordinator.registration_generation()
        });
        self.inner.registered_generation.set(Some(generation));
    }

    fn unregister_owned(&self, cx: &mut App) {
        let origin = self.origin();
        let destination = self.destination();
        if origin.is_none() && destination.is_none() {
            return;
        }
        let owner = self.owner.clone();
        SharedElementTransitionCoordinator::shared(cx).update(cx, |coordinator, cx| {
            if let Some(origin) = &origin {
                coordinator.unregister_origin(origin, &owner, cx);
            }
            if let Some(destination) = &destination {
                coordinator.unregister_destination(destination, &owner, cx);
            }
        });
    }

    fn enclosing_stack_host_controller(&self) -> Option<SharedElementStackHost> {
        self.host.get()
    }
}
