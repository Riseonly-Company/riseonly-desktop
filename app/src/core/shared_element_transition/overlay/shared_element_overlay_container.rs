use std::cell::RefCell;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, Context, Entity, EntityId, IntoElement, ParentElement, Pixels,
    Render, Size, Styled, Subscription, WeakEntity, Window, div, rgb,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::ease_out;

use crate::core::shared_element_transition::overlay::shared_element_overlay_driver::SharedElementOverlayDriver;
use crate::core::shared_element_transition::renderer::shared_element_flight_runner::SharedElementFlightContainer;

struct InstalledContainer {
    window: AnyWindowHandle,
    container: WeakEntity<SharedElementOverlayContainerView>,
}

thread_local! {
    static INSTALLED: RefCell<Vec<InstalledContainer>> = const { RefCell::new(Vec::new()) };
}

/// window level host of an overlay flight, a black curtain under the flight view
pub struct SharedElementOverlayContainerView {
    window: Option<AnyWindowHandle>,
    curtain: Animated<f32>,
    subviews: Vec<AnyView>,
    bounds_size: Size<Pixels>,
    is_removed: bool,
}

impl SharedElementOverlayContainerView {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            window: None,
            curtain: Animated::new(0.0),
            subviews: Vec::new(),
            bounds_size: Size::default(),
            is_removed: false,
        }
    }

    pub fn bounds_size(&self) -> Size<Pixels> {
        self.bounds_size
    }

    pub fn window(&self) -> Option<AnyWindowHandle> {
        self.window
    }

    pub fn install(&mut self, window: AnyWindowHandle, curtain_alpha: f32, cx: &mut Context<Self>) {
        self.window = Some(window);
        self.curtain.jump(curtain_alpha);
        let container = cx.weak_entity();
        INSTALLED.with(|installed| {
            let mut installed = installed.borrow_mut();
            installed.retain(|entry| entry.container.upgrade().is_some());
            installed.push(InstalledContainer { window, container });
        });
        Self::refresh(window, cx);
        cx.notify();
    }

    pub fn set_curtain_alpha(&mut self, alpha: f32, cx: &mut Context<Self>) {
        self.curtain.jump(alpha);
        cx.notify();
    }

    /// fades the curtain, the fade starts from the alpha on screen
    pub fn fade_curtain(&mut self, to: f32, duration: f64, cx: &mut Context<Self>) {
        self.curtain.set_in(to, ease_out(duration.max(0.0)), RiseClock::monotonic(), cx);
        cx.notify();
    }

    pub fn presented_curtain_alpha(&self, now: Duration) -> f32 {
        self.curtain.value(now)
    }

    pub fn add_subview(&mut self, view: AnyView, cx: &mut Context<Self>) {
        let id = view.entity_id();
        self.subviews.retain(|subview| subview.entity_id() != id);
        self.subviews.push(view);
        cx.notify();
    }

    pub fn remove_subview(&mut self, id: EntityId, cx: &mut Context<Self>) {
        self.subviews.retain(|subview| subview.entity_id() != id);
        cx.notify();
    }

    fn layout_subviews(&mut self, window: &Window) {
        self.bounds_size = window.viewport_size();
    }

    pub fn remove_now(&mut self, cx: &mut Context<Self>) {
        if self.is_removed {
            return;
        }
        self.is_removed = true;
        let target = *self.curtain.target();
        self.curtain.jump(target);
        let id = cx.entity_id();
        INSTALLED.with(|installed| {
            installed.borrow_mut().retain(|entry| {
                entry.container.upgrade().is_some_and(|container| container.entity_id() != id)
            });
        });
        if let Some(window) = self.window.take() {
            Self::refresh(window, cx);
        }
        cx.notify();
    }

    /// the layer draws on the next frame of the window
    /// deferred since install and removal may run inside an update of that same window
    fn refresh(window: AnyWindowHandle, cx: &mut App) {
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, _| window.refresh());
        });
    }

    fn installed_in(window: AnyWindowHandle) -> Vec<Entity<SharedElementOverlayContainerView>> {
        INSTALLED.with(|installed| {
            installed
                .borrow()
                .iter()
                .filter(|entry| entry.window == window)
                .filter_map(|entry| entry.container.upgrade())
                .collect()
        })
    }
}

impl Render for SharedElementOverlayContainerView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.layout_subviews(window);
        let alpha = self.curtain.frame_value(RiseClock::monotonic(), window);
        div()
            .absolute()
            .top_0()
            .left_0()
            .w(self.bounds_size.width)
            .h(self.bounds_size.height)
            .child(div().absolute().top_0().left_0().size_full().bg(rgb(0x000000)).opacity(alpha))
            .children(self.subviews.iter().cloned())
    }
}

impl SharedElementFlightContainer for WeakEntity<SharedElementOverlayContainerView> {
    fn add_flight_view(&self, view: AnyView, cx: &mut App) {
        let _ = self.update(cx, |container, cx| container.add_subview(view, cx));
    }

    fn remove_flight_view(&self, view: EntityId, cx: &mut App) {
        let _ = self.update(cx, |container, cx| container.remove_subview(view, cx));
    }
}

pub struct SharedElementOverlayContainerHost {
    window: AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
}

impl SharedElementOverlayContainerHost {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe_window_bounds(window, |_, _, cx| {
            cx.defer(SharedElementOverlayDriver::window_did_resize);
        })];
        Self { window: window.window_handle(), _subscriptions: subscriptions }
    }
}

impl Render for SharedElementOverlayContainerHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .children(SharedElementOverlayContainerView::installed_in(self.window))
    }
}

pub trait SharedElementOverlayContainerHostModifiers: IntoElement + Sized {
    fn mounts_shared_element_overlay_container(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let host = window.use_keyed_state("shared-element-overlay-container-host", cx, SharedElementOverlayContainerHost::new);
        div().relative().size_full().child(self).child(host).into_any_element()
    }
}

impl<E: IntoElement> SharedElementOverlayContainerHostModifiers for E {}
