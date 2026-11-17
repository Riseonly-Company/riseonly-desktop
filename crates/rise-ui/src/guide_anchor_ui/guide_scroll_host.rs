use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, ListState, Pixels, Point, ScrollHandle, ScrollWheelEvent,
    TouchPhase, Window, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::guide_anchor_ui::guide_anchor_registry::{GuideAnchorRegistry, current_layer, monotonic_ms, next_paint_order};
use crate::swiftui::animated::Animated;
use crate::swiftui::motion::ease_in_out;

const SET_CONTENT_OFFSET_ANIMATION_SECONDS: f64 = 0.3;

const WHEEL_QUIET_MS: i64 = 100;

thread_local! {
    static HOST_STACK: RefCell<Vec<GuideScrollHost>> = const { RefCell::new(Vec::new()) };
    static LIVE_HOSTS: RefCell<Vec<Weak<GuideScrollHostInner>>> = const { RefCell::new(Vec::new()) };
}

/// what holds the scroll offset
/// a list only knows content it already measured so max offset grows while paging
enum GuideScrollSource {
    Scroll(ScrollHandle),
    List(ListState),
}

struct GuideScrollHostInner {
    source: GuideScrollSource,
    window: Cell<Option<AnyWindowHandle>>,
    frame: Cell<Option<Bounds<Pixels>>>,
    layer: Cell<u64>,
    order: Cell<u64>,
    scroll_enabled: Cell<bool>,
    tracking: Cell<bool>,
    last_wheel_ms: Cell<Option<i64>>,
    offset_animation: RefCell<Option<Animated<Point<Pixels>>>>,
}

/// one scroll view of the kit as the guide scroller sees it, clones share it
#[derive(Clone)]
pub struct GuideScrollHost {
    inner: Rc<GuideScrollHostInner>,
}

#[derive(Clone)]
pub struct WeakGuideScrollHost {
    inner: Weak<GuideScrollHostInner>,
}

impl WeakGuideScrollHost {
    pub fn upgrade(&self) -> Option<GuideScrollHost> {
        self.inner.upgrade().map(|inner| GuideScrollHost { inner })
    }

    fn is(&self, host: &GuideScrollHost) -> bool {
        self.inner.as_ptr() == Rc::as_ptr(&host.inner)
    }
}

/// owning end kept in the scroll view state, dropping it removes the host from the window
pub struct GuideScrollHostMount {
    host: GuideScrollHost,
}

impl GuideScrollHostMount {
    pub fn new(handle: ScrollHandle) -> Self {
        Self::mounted(GuideScrollSource::Scroll(handle))
    }

    /// host for a list standing in for a scroll view
    pub fn for_list(list: ListState) -> Self {
        Self::mounted(GuideScrollSource::List(list))
    }

    fn mounted(source: GuideScrollSource) -> Self {
        let host = GuideScrollHost {
            inner: Rc::new(GuideScrollHostInner {
                source,
                window: Cell::new(None),
                frame: Cell::new(None),
                layer: Cell::new(0),
                order: Cell::new(0),
                scroll_enabled: Cell::new(true),
                tracking: Cell::new(false),
                last_wheel_ms: Cell::new(None),
                offset_animation: RefCell::new(None),
            }),
        };
        LIVE_HOSTS.with(|hosts| {
            let mut hosts = hosts.borrow_mut();
            hosts.retain(|weak| weak.strong_count() > 0);
            hosts.push(Rc::downgrade(&host.inner));
        });
        Self { host }
    }

    pub fn host(&self) -> GuideScrollHost {
        self.host.clone()
    }
}

impl Drop for GuideScrollHostMount {
    fn drop(&mut self) {
        self.host.inner.window.set(None);
        self.host.inner.frame.set(None);
        self.host.inner.tracking.set(false);
        self.host.inner.offset_animation.borrow_mut().take();
    }
}

impl GuideScrollHost {
    /// every kit scroll view drawn right now in any window
    pub fn live() -> Vec<GuideScrollHost> {
        LIVE_HOSTS.with(|hosts| {
            let mut hosts = hosts.borrow_mut();
            hosts.retain(|weak| weak.strong_count() > 0);
            hosts
                .iter()
                .filter_map(Weak::upgrade)
                .map(|inner| GuideScrollHost { inner })
                .filter(GuideScrollHost::is_in_window)
                .collect()
        })
    }

    /// innermost scroll view around the element being prepainted
    /// none outside prepaint or outside any scroll view
    pub fn current() -> Option<GuideScrollHost> {
        HOST_STACK.with(|stack| stack.borrow().last().cloned())
    }

    pub fn downgrade(&self) -> WeakGuideScrollHost {
        WeakGuideScrollHost { inner: Rc::downgrade(&self.inner) }
    }

    /// true when both handles are the same scroll view
    pub fn same_host(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn is_in_window(&self) -> bool {
        self.inner.window.get().is_some() && self.inner.frame.get().is_some()
    }

    pub fn window(&self) -> Option<AnyWindowHandle> {
        self.inner.window.get()
    }

    pub fn forget_frame(&self) {
        self.inner.frame.set(None);
    }

    /// viewport in window coordinates
    pub fn frame(&self) -> Option<Bounds<Pixels>> {
        self.inner.frame.get()
    }

    pub fn content_offset(&self) -> Point<Pixels> {
        let offset = match &self.inner.source {
            GuideScrollSource::Scroll(handle) => handle.offset(),
            GuideScrollSource::List(list) => list.scroll_px_offset_for_scrollbar(),
        };
        point(px(-f32::from(offset.x)), px(-f32::from(offset.y)))
    }

    /// largest content offset, never negative
    pub fn max_content_offset(&self) -> Point<Pixels> {
        let max = match &self.inner.source {
            GuideScrollSource::Scroll(handle) => handle.max_offset(),
            GuideScrollSource::List(list) => list.max_offset_for_scrollbar(),
        };
        point(px(f32::from(max.x).max(0.0)), px(f32::from(max.y).max(0.0)))
    }

    pub fn is_scroll_enabled(&self) -> bool {
        self.inner.scroll_enabled.get()
    }

    /// turns scrolling on or off for the owning scroll view, written on each render
    pub fn set_scroll_enabled(&self, enabled: bool) {
        self.inner.scroll_enabled.set(enabled);
    }

    /// true while a trackpad gesture is on the content
    pub fn is_tracking(&self) -> bool {
        self.inner.tracking.get()
    }

    /// true while the content still rides a fling or a wheel run
    pub fn is_decelerating(&self) -> bool {
        !self.inner.tracking.get()
            && self
                .inner
                .last_wheel_ms
                .get()
                .is_some_and(|at| monotonic_ms() - at < WHEEL_QUIET_MS)
    }

    /// called by the owning scroll view for every wheel step
    /// reader input wins over an offset the guide is still animating
    pub fn note_wheel(&self, event: &ScrollWheelEvent) {
        match event.touch_phase {
            TouchPhase::Started => self.inner.tracking.set(true),
            TouchPhase::Ended | TouchPhase::Cancelled => self.inner.tracking.set(false),
            TouchPhase::Moved => {}
        }
        self.inner.last_wheel_ms.set(Some(monotonic_ms()));
        self.inner.offset_animation.borrow_mut().take();
    }

    /// sets the content offset, optionally animated
    /// the scroll view clamps it itself on its next layout
    pub fn set_content_offset(&self, offset: Point<Pixels>, animated: bool, cx: &mut App) {
        if animated {
            let now = RiseClock::monotonic();
            let mut motion = self
                .inner
                .offset_animation
                .borrow_mut()
                .take()
                .unwrap_or_else(|| Animated::new(self.content_offset()));
            motion.set(offset, ease_in_out(SET_CONTENT_OFFSET_ANIMATION_SECONDS), now, false);
            *self.inner.offset_animation.borrow_mut() = Some(motion);
        } else {
            self.inner.offset_animation.borrow_mut().take();
            self.apply_offset(offset);
        }
        if let Some(window) = self.inner.window.get() {
            let _ = window.update(cx, |_, window, _| window.refresh());
        }
    }

    /// true when this window point belongs to this scroll view and not a sheet, menu or dialog over it
    pub fn answers_for(&self, at: Point<Pixels>) -> bool {
        let Some(window) = self.inner.window.get() else { return false };
        let layer = self.inner.layer.get();
        GuideAnchorRegistry::layer_over(window, at, layer, self.inner.order.get()) == layer
    }

    fn apply_offset(&self, offset: Point<Pixels>) {
        let negated = point(px(-f32::from(offset.x)), px(-f32::from(offset.y)));
        match &self.inner.source {
            GuideScrollSource::Scroll(handle) => handle.set_offset(negated),
            GuideScrollSource::List(list) => list.set_offset_from_scrollbar(negated),
        }
    }

    fn did_lay_out(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        self.inner.window.set(Some(window.window_handle()));
        self.inner.frame.set(Some(bounds));
        self.inner.layer.set(current_layer());
        self.inner.order.set(next_paint_order());
        let now = RiseClock::monotonic();
        let frame_offset = {
            let motion = self.inner.offset_animation.borrow();
            motion.as_ref().map(|motion| (motion.frame_value(now, window), motion.is_running(now)))
        };
        if let Some((offset, running)) = frame_offset {
            self.apply_offset(offset);
            if !running {
                self.inner.offset_animation.borrow_mut().take();
            }
        }
    }
}

/// scroll hosts around the element being prepainted, innermost first
pub(crate) fn current_scroll_hosts() -> Vec<WeakGuideScrollHost> {
    HOST_STACK.with(|stack| stack.borrow().iter().rev().map(GuideScrollHost::downgrade).collect())
}

/// true when stored names the same hosts in the same order as the stack now
/// usual answer, so anchor prepaint allocates nothing while it stays put
pub(crate) fn is_current_scroll_hosts(stored: &[WeakGuideScrollHost]) -> bool {
    HOST_STACK.with(|stack| {
        let stack = stack.borrow();
        stack.len() == stored.len()
            && stack.iter().rev().zip(stored.iter()).all(|(host, weak)| weak.is(host))
    })
}

/// wraps the scrolling element of a kit scroll view
/// its bounds are the viewport and the host sits on the stack while children prepaint
pub struct GuideScrollHostScope {
    host: GuideScrollHost,
    child: AnyElement,
}

impl GuideScrollHostScope {
    pub fn new(host: GuideScrollHost, child: impl IntoElement) -> Self {
        Self { host, child: child.into_any_element() }
    }
}

impl IntoElement for GuideScrollHostScope {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GuideScrollHostScope {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.host.did_lay_out(bounds, window);
        HOST_STACK.with(|stack| stack.borrow_mut().push(self.host.clone()));
        self.child.prepaint(window, cx);
        HOST_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
