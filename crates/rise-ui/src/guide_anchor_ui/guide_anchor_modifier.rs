use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, Element, ElementId, EntityId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Size, Window,
};
use rise_canon::RiseGuideAnchor;

use crate::guide_anchor_ui::guide_anchor_registry::{
    GuideAnchorCapture, GuideAnchorRegistry, current_layer, next_paint_order,
};
use crate::guide_anchor_ui::guide_anchor_tap_tracker::GuideAnchorTapTracker;
use crate::guide_anchor_ui::guide_scroll_host::{
    GuideScrollHost, WeakGuideScrollHost, current_scroll_hosts, is_current_scroll_hosts,
};

pub trait GuideAnchorModifierModifiers: IntoElement + Sized {
    /// declares this element as the canon anchor
    /// pass None where its the anchor only in one role
    /// eg ProfileTop draws own and stranger profile but only the first has profile_avatar
    fn guide_anchor(self, anchor: impl Into<Option<RiseGuideAnchor>>) -> GuideAnchorReader {
        GuideAnchorReader { anchor: anchor.into(), child: self.into_any_element() }
    }
}

impl<E: IntoElement> GuideAnchorModifierModifiers for E {}

/// where the element is drawn as of one prepaint
#[derive(Clone, Copy)]
struct GuideAnchorPlacement {
    window: AnyWindowHandle,
    viewport: Size<Pixels>,
    bounds: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    layer: u64,
    view: EntityId,
}

pub(crate) struct GuideAnchorBackingInner {
    anchor: Cell<Option<RiseGuideAnchor>>,
    pub(crate) window: Cell<Option<AnyWindowHandle>>,
    pub(crate) viewport: Cell<Size<Pixels>>,
    frame: Cell<Option<Bounds<Pixels>>>,
    pub(crate) clip: Cell<Option<Bounds<Pixels>>>,
    pub(crate) layer: Cell<u64>,
    pub(crate) order: Cell<u64>,
    view: Cell<Option<EntityId>>,
    relaid_at: Cell<i64>,
    relaid_frame: Cell<Option<Bounds<Pixels>>>,
    relaid_redraws: Cell<u32>,
    scroll_hosts: RefCell<Vec<WeakGuideScrollHost>>,
}

impl Drop for GuideAnchorBackingInner {
    fn drop(&mut self) {
        if let Some(anchor) = self.anchor.take() {
            GuideAnchorReader::dismantle_ui_view(anchor);
        }
    }
}

/// the element whose window rect answers for the anchor
/// bounds are the decorated element, draws nothing and takes no clicks
/// cloning shares it, registry only holds weak refs
#[derive(Clone)]
pub struct GuideAnchorBackingView {
    pub(crate) inner: Rc<GuideAnchorBackingInner>,
}

impl GuideAnchorBackingView {
    const RELAYOUT_INTERVAL_MS: i64 = 250;
    const RELAYOUT_REDRAW_LIMIT: u32 = 4;

    fn new() -> Self {
        Self {
            inner: Rc::new(GuideAnchorBackingInner {
                anchor: Cell::new(None),
                window: Cell::new(None),
                viewport: Cell::new(Size::default()),
                frame: Cell::new(None),
                clip: Cell::new(None),
                layer: Cell::new(0),
                order: Cell::new(0),
                view: Cell::new(None),
                relaid_at: Cell::new(i64::MIN),
                relaid_frame: Cell::new(None),
                relaid_redraws: Cell::new(0),
                scroll_hosts: RefCell::new(Vec::new()),
            }),
        }
    }

    /// the window the element is drawn in
    pub fn window(&self) -> Option<AnyWindowHandle> {
        self.inner.window.get()
    }

    /// element frame in window coordinates as last laid out
    /// a mounted but not laid out subtree gives a degenerate rect and a cutout would be a dot in the corner
    pub fn window_frame(&self) -> Option<Bounds<Pixels>> {
        let frame = self.inner.frame.get()?;
        (f32::from(frame.size.width) > 1.0 && f32::from(frame.size.height) > 1.0).then_some(frame)
    }

    /// part of window_frame its clips leave on screen
    /// a tile taller than its scroll viewport is cut at the edge
    /// a cutout around the whole tile framed the header above, fully clipped keeps its frame
    pub fn visible_window_frame(&self) -> Option<Bounds<Pixels>> {
        let frame = self.window_frame()?;
        let clip = self.inner.clip.get().map(|clip| match self.inner.window.get() {
            Some(window) => GuideAnchorRegistry::trimmed_by_chrome(window, self.inner.layer.get(), self.inner.order.get(), clip),
            None => clip,
        });
        match clip {
            Some(clip) if f32::from(clip.size.width) > 1.0 && f32::from(clip.size.height) > 1.0 => Some(clip),
            _ => Some(frame),
        }
    }

    /// kit scroll views that carry the element, innermost first
    /// as of the last prepaint
    /// for GuideAnchorScroller which walks them up
    pub fn scroll_hosts(&self) -> Vec<GuideScrollHost> {
        self.inner.scroll_hosts.borrow().iter().filter_map(WeakGuideScrollHost::upgrade).collect()
    }

    pub(crate) fn request_fresh_layout(&self, now: i64, cx: &mut App) -> bool {
        let Some(view) = self.inner.view.get() else { return false };
        if now.saturating_sub(self.inner.relaid_at.get()) < Self::RELAYOUT_INTERVAL_MS {
            return false;
        }
        self.inner.relaid_at.set(now);
        let frame = self.inner.frame.get();
        if frame != self.inner.relaid_frame.get() {
            self.inner.relaid_frame.set(frame);
            self.inner.relaid_redraws.set(0);
        }
        let redraws = self.inner.relaid_redraws.get();
        tracing::warn!(
            target: "riseonly::X1738",
            frame = ?frame,
            clip = ?self.inner.clip.get(),
            redraws,
            "anchor relayout request"
        );
        match self.inner.window.get() {
            Some(window) if redraws < Self::RELAYOUT_REDRAW_LIMIT => {
                self.inner.relaid_redraws.set(redraws + 1);
                cx.defer(move |cx| {
                    let refreshed = cx.update_window(window, |_, window, _| window.refresh()).is_ok();
                    tracing::warn!(target: "riseonly::X1738", refreshed, "anchor relayout refresh");
                });
            }
            _ => cx.notify(view),
        }
        true
    }

    /// whether both handles are the same element
    pub fn same_view(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn set_anchor(&self, next: Option<RiseGuideAnchor>) {
        if next == self.inner.anchor.get() {
            return;
        }
        if let Some(anchor) = self.inner.anchor.get() {
            GuideAnchorRegistry::detach(anchor, self);
        }
        self.inner.anchor.set(next);
        if let Some(next) = next
            && self.inner.window.get().is_some()
        {
            GuideAnchorRegistry::attach(next, self);
        }
    }

    fn note_scroll_hosts(&self) {
        if is_current_scroll_hosts(&self.inner.scroll_hosts.borrow()) {
            return;
        }
        *self.inner.scroll_hosts.borrow_mut() = current_scroll_hosts();
    }

    fn did_move_to_window(&self, placement: GuideAnchorPlacement) {
        let joined = self.inner.window.get() != Some(placement.window);
        self.inner.window.set(Some(placement.window));
        self.inner.viewport.set(placement.viewport);
        self.inner.frame.set(Some(placement.bounds));
        self.inner.clip.set(Some(placement.clip));
        self.inner.layer.set(placement.layer);
        self.inner.order.set(next_paint_order());
        self.inner.view.set(Some(placement.view));
        if joined && let Some(anchor) = self.inner.anchor.get() {
            GuideAnchorRegistry::attach(anchor, self);
        }
    }

    fn layout_subviews(&self) {
        if !GuideAnchorCapture::is_active() {
            return;
        }
        if let Some(anchor) = self.inner.anchor.get() {
            GuideAnchorRegistry::note_layout_changed(anchor);
        }
    }
}

/// the wrapper guide_anchor returns
/// takes the decorated element own layout so its bounds are that element bounds
pub struct GuideAnchorReader {
    anchor: Option<RiseGuideAnchor>,
    child: AnyElement,
}

impl GuideAnchorReader {
    fn make_ui_view(&self) -> GuideAnchorBackingView {
        let view = GuideAnchorBackingView::new();
        view.set_anchor(self.anchor);
        view
    }

    fn update_ui_view(&self, ui_view: &GuideAnchorBackingView, placement: GuideAnchorPlacement) {
        ui_view.note_scroll_hosts();
        ui_view.did_move_to_window(placement);
        ui_view.set_anchor(self.anchor);
        ui_view.layout_subviews();
    }

    fn dismantle_ui_view(anchor: RiseGuideAnchor) {
        GuideAnchorRegistry::detach_dropped(anchor);
    }
}

impl IntoElement for GuideAnchorReader {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GuideAnchorReader {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        self.anchor.map(|anchor| ElementId::Name(anchor.raw_value().into()))
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
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if let Some(id) = id {
            let mask = window.content_mask().bounds;
            let placement = GuideAnchorPlacement {
                window: window.window_handle(),
                viewport: window.viewport_size(),
                bounds,
                clip: bounds.intersect(&mask),
                layer: current_layer(),
                view: window.current_view(),
            };
            let backing = window.with_element_state::<GuideAnchorBackingView, _>(id, |backing, _window| {
                let backing = backing.unwrap_or_else(|| self.make_ui_view());
                self.update_ui_view(&backing, placement);
                (backing.clone(), backing)
            });
            window.on_replayed_move(move |offset, mask_offset, _window, _cx| {
                let bounds = placement.bounds + offset;
                let moved = GuideAnchorPlacement { bounds, clip: bounds.intersect(&(mask + mask_offset)), ..placement };
                backing.did_move_to_window(moved);
                backing.layout_subviews();
            });
        }
        self.child.prepaint(window, cx);
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
        GuideAnchorTapTracker::install(window);
        self.child.paint(window, cx);
    }
}
