use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    AnyElement, AnyWindowHandle, App, AsyncApp, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, Size, Window, point, px,
};
use rise_canon::{RiseGuideAnchor, RiseGuideCanon};
use rise_core::foundation::foundation_date::RiseClock;

use crate::guide_anchor_ui::guide_anchor_modifier::{GuideAnchorBackingInner, GuideAnchorBackingView};
use crate::guide_anchor_ui::guide_anchor_scroller_hooks::guide_anchor_scroller_hooks;
use crate::guide_anchor_ui::guide_anchor_tap_tracker::GuideAnchorTapTracker;

/// whether anything is currently interested in anchor geometry
/// read from every mounted anchor prepaint so its one thread local counter and not observed state
/// layout written state must never feed the render of the view that touched it
pub enum GuideAnchorCapture {}

thread_local! {
    static CAPTURE_DEPTH: Cell<usize> = const { Cell::new(0) };
    static CAPTURE_DID_BEGIN: RefCell<Vec<Weak<dyn Fn()>>> = const { RefCell::new(Vec::new()) };
}

impl GuideAnchorCapture {
    pub fn is_active() -> bool {
        CAPTURE_DEPTH.with(Cell::get) > 0
    }

    pub fn did_begin(listener: &Rc<dyn Fn()>) {
        CAPTURE_DID_BEGIN.with(|listeners| {
            let mut listeners = listeners.borrow_mut();
            listeners.retain(|weak| weak.strong_count() > 0);
            listeners.push(Rc::downgrade(listener));
        });
    }

    /// balanced and not a plain flag
    /// a hint shown over a running tour would otherwise switch capture off while the tour needs it
    pub fn begin() {
        let depth = CAPTURE_DEPTH.with(|depth| {
            let next = depth.get() + 1;
            depth.set(next);
            next
        });
        if depth == 1 {
            let listeners: Vec<Rc<dyn Fn()>> = CAPTURE_DID_BEGIN
                .with(|listeners| listeners.borrow().iter().filter_map(Weak::upgrade).collect());
            for listener in listeners {
                listener();
            }
        }
    }

    pub fn end() {
        CAPTURE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// what the guide can see of an anchor, the three states a spotlight has to tell apart
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GuideAnchorSighting {
    Visible(Bounds<Pixels>),
    Covered,
    Absent,
}

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

/// paint order of covers, a cover prepainted later is drawn above an earlier one
pub(crate) fn next_paint_order() -> u64 {
    thread_local! {
        static ORDER: Cell<u64> = const { Cell::new(0) };
    }
    ORDER.with(|order| {
        let next = order.get() + 1;
        order.set(next);
        next
    })
}

thread_local! {
    static LAYER_STACK: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// layer an anchor is drawn in, the innermost open cover or the window own screen (0)
pub(crate) fn current_layer() -> u64 {
    LAYER_STACK.with(|stack| stack.borrow().last().copied().unwrap_or(0))
}

/// kind of layer a cover stands for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideAnchorCoverKind {
    Layer,
    Alert,
    Menu,
    Chrome,
}

struct GuideAnchorCoverInner {
    serial: u64,
    kind: Cell<GuideAnchorCoverKind>,
    window: Cell<Option<AnyWindowHandle>>,
    viewport: Cell<Size<Pixels>>,
    frame: Cell<Option<Bounds<Pixels>>>,
    order: Cell<u64>,
}

/// element state of one mounted cover
#[derive(Clone)]
struct GuideAnchorCoverView {
    inner: Rc<GuideAnchorCoverInner>,
}

/// snapshot of one live cover, for the menu probe
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GuideAnchorCoverFrame {
    pub kind: GuideAnchorCoverKind,
    pub viewport: Size<Pixels>,
    pub frame: Bounds<Pixels>,
    pub order: u64,
}

#[derive(Default)]
struct RegistryState {
    registrations: HashMap<RiseGuideAnchor, Vec<Weak<GuideAnchorBackingInner>>>,
    last_frames: HashMap<RiseGuideAnchor, Bounds<Pixels>>,
    ever_registered: HashSet<RiseGuideAnchor>,
    generation: i64,
    covers: Vec<Weak<GuideAnchorCoverInner>>,
}

thread_local! {
    static REGISTRY: RefCell<RegistryState> = RefCell::new(RegistryState::default());
}

pub enum GuideAnchorRegistry {}

impl GuideAnchorRegistry {
    const STABILITY_TOLERANCE: f32 = 0.5;

    const POLL_INTERVAL_MS: i64 = 33;

    pub fn attach(anchor: RiseGuideAnchor, view: &GuideAnchorBackingView) {
        REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            let boxes = registry.registrations.entry(anchor).or_default();
            boxes.retain(|weak| weak.strong_count() > 0);
            if !boxes.iter().any(|weak| weak.as_ptr() == Rc::as_ptr(&view.inner)) {
                boxes.push(Rc::downgrade(&view.inner));
            }
            registry.ever_registered.insert(anchor);
            Self::invalidate(&mut registry, anchor);
        });
        if GuideAnchorCapture::is_active()
            && let Some(hooks) = guide_anchor_scroller_hooks()
        {
            (hooks.note_mounted)(anchor, view);
        }
    }

    pub fn detach(anchor: RiseGuideAnchor, view: &GuideAnchorBackingView) {
        Self::detach_serial(anchor, Rc::as_ptr(&view.inner));
    }

    fn detach_serial(anchor: RiseGuideAnchor, view: *const GuideAnchorBackingInner) {
        REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            let Some(boxes) = registry.registrations.get_mut(&anchor) else { return };
            boxes.retain(|weak| weak.strong_count() > 0 && weak.as_ptr() != view);
            if boxes.is_empty() {
                registry.registrations.remove(&anchor);
            }
            Self::invalidate(&mut registry, anchor);
        });
    }

    pub(crate) fn detach_dropped(anchor: RiseGuideAnchor) {
        let _ = REGISTRY.try_with(|registry| {
            let Ok(mut registry) = registry.try_borrow_mut() else { return };
            if let Some(boxes) = registry.registrations.get_mut(&anchor) {
                boxes.retain(|weak| weak.strong_count() > 0);
                if boxes.is_empty() {
                    registry.registrations.remove(&anchor);
                }
            }
            Self::invalidate(&mut registry, anchor);
        });
    }

    pub fn note_layout_changed(anchor: RiseGuideAnchor) {
        Self::sample(anchor);
    }

    /// every anchor mounted at least once since launch, read by GuideAnchorInventory
    /// runtime uses is_registered since existed once and on screen now differ
    pub fn ever_registered() -> HashSet<RiseGuideAnchor> {
        REGISTRY.with(|registry| registry.borrow().ever_registered.clone())
    }

    /// bumped whenever a sampled anchor frame changes
    /// a watcher with no poll loop compares it between reads, settled_frame supplies the loop
    pub fn generation() -> i64 {
        REGISTRY.with(|registry| registry.borrow().generation)
    }

    /// whether the anchor is drawn in a window right now
    /// separates anchor_missing from anchor_offscreen
    pub fn is_registered(anchor: RiseGuideAnchor) -> bool {
        Self::live_view(anchor).is_some()
    }

    /// element that answers for the anchor, for the one caller that needs more than a rect
    /// finds what scrolls it for GuideAnchorScroller, keep it the only reader
    pub fn anchor_view(anchor: RiseGuideAnchor) -> Option<GuideAnchorBackingView> {
        Self::live_view(anchor)
    }

    pub fn visible_neighbours(anchor: RiseGuideAnchor) -> Vec<GuideAnchorBackingView> {
        let others: Vec<RiseGuideAnchor> = REGISTRY.with(|registry| {
            registry
                .borrow()
                .registrations
                .keys()
                .copied()
                .filter(|other| *other != anchor && other.screen() == anchor.screen())
                .collect()
        });
        others
            .into_iter()
            .filter_map(|other| match Self::sighting(other) {
                GuideAnchorSighting::Visible(_) => Self::live_view(other),
                GuideAnchorSighting::Covered | GuideAnchorSighting::Absent => None,
            })
            .collect()
    }

    /// anchor frame in window coordinates as of the last prepaint
    /// read on demand and not pushed into state, so an anchor costs nothing while no guide runs
    pub fn frame(anchor: RiseGuideAnchor) -> Option<Bounds<Pixels>> {
        Self::sample(anchor)
    }

    /// anchor frame once it held still for anchor_settle_ms, None if it didnt settle in time
    /// tell the two canon failures apart with is_registered
    /// absent is anchor_missing, present with no stable on screen rect is anchor_offscreen
    pub async fn settled_frame(
        anchor: RiseGuideAnchor,
        within: Option<i64>,
        cx: &AsyncApp,
    ) -> Option<Bounds<Pixels>> {
        let timeout_ms = within.unwrap_or(RiseGuideCanon::TIMINGS_ANCHOR_REVEAL_MS);
        let settle_ms = RiseGuideCanon::TIMINGS_ANCHOR_SETTLE_MS;
        let deadline = monotonic_ms() + timeout_ms.max(0);
        let mut candidate: Option<Bounds<Pixels>> = None;
        let mut stable_since = monotonic_ms();
        let mut traced: Option<std::mem::Discriminant<GuideAnchorSighting>> = None;

        loop {
            let sighting = Self::sighting(anchor);
            let kind = std::mem::discriminant(&sighting);
            if traced != Some(kind) {
                traced = Some(kind);
                tracing::warn!(
                    target: "riseonly::X1738",
                    anchor = anchor.raw_value(),
                    sighting = ?sighting,
                    detail = %Self::describe_sighting(anchor),
                    "settle sighting"
                );
            }
            if sighting == GuideAnchorSighting::Absent {
                cx.update(|cx| Self::refresh_hidden(anchor, cx));
            }
            let measured = match sighting {
                GuideAnchorSighting::Visible(frame) => Some(frame),
                GuideAnchorSighting::Covered | GuideAnchorSighting::Absent => None,
            };
            if let Some(measured) = measured {
                if is_stable(candidate, Some(measured)) {
                    if monotonic_ms() - stable_since >= settle_ms {
                        return Some(measured);
                    }
                } else {
                    candidate = Some(measured);
                    stable_since = monotonic_ms();
                }
            } else if candidate.is_some() {
                candidate = None;
                stable_since = monotonic_ms();
            }

            if monotonic_ms() >= deadline {
                return None;
            }
            cx.background_executor()
                .timer(Duration::from_millis(Self::POLL_INTERVAL_MS.unsigned_abs()))
                .await;
        }
    }

    /// re lays out a drawn element the sighting calls absent, throttled per element
    /// its clip may predate a cached view sliding in and only a render of that view replaces it
    /// one render per interval, only while a guide waits
    pub fn refresh_hidden(anchor: RiseGuideAnchor, cx: &mut App) {
        let Some(view) = Self::live_view(anchor) else { return };
        if view.window_frame().is_none() {
            return;
        }
        if view.request_fresh_layout(monotonic_ms(), cx) {
            tracing::debug!(target: "riseonly::X1738", anchor = anchor.raw_value(), "anchor relayout");
        }
    }

    /// what the guide can see of the anchor right now
    /// three answers since the guide owes each a different behaviour and a frame alone cant tell them apart
    /// an element behind a sheet reports the same rect as a visible one
    pub fn sighting(anchor: RiseGuideAnchor) -> GuideAnchorSighting {
        let Some(view) = Self::live_view(anchor) else { return GuideAnchorSighting::Absent };
        let Some(window) = view.inner.window.get() else { return GuideAnchorSighting::Absent };
        let Some(frame) = Self::sample(anchor) else { return GuideAnchorSighting::Absent };
        let window_bounds = Bounds { origin: Point::default(), size: view.inner.viewport.get() };
        let clip = view.inner.clip.get().unwrap_or(frame);
        if !window_bounds.intersects(&frame) || !has_area(clip) {
            return GuideAnchorSighting::Absent;
        }
        if Self::is_reachable(&view, window, window_bounds, clip) {
            GuideAnchorSighting::Visible(frame)
        } else {
            GuideAnchorSighting::Covered
        }
    }

    /// whether the anchor own layer is still frontmost there or the app put a layer over it
    /// covers know whats on top for every sheet, modal or menu
    /// asymmetric on purpose, a miss keeps the spotlight and a false alarm takes it away
    fn is_reachable(
        view: &GuideAnchorBackingView,
        window: AnyWindowHandle,
        window_bounds: Bounds<Pixels>,
        frame: Bounds<Pixels>,
    ) -> bool {
        let own_layer = view.inner.layer.get();
        let own_order = view.inner.order.get();
        let covers: Vec<LiveCover> = Self::live_covers(Some(window))
            .into_iter()
            .filter(|cover| cover.serial == own_layer || cover.order > own_order)
            .collect();
        let mut saw_foreign_view = false;
        for probe in probe_points(frame) {
            if !window_bounds.contains(&probe) {
                continue;
            }
            let top = covers
                .iter()
                .filter(|cover| cover.frame.contains(&probe))
                .max_by_key(|cover| cover.order)
                .map_or(0, |cover| cover.serial);
            if top == own_layer {
                return true;
            }
            saw_foreign_view = true;
        }
        !saw_foreign_view
    }

    /// which window the anchor is drawn in
    /// an anchor can be a row in a menu window above the guide own
    /// the overlay uses this to climb over it only while the step points there
    pub fn window_level(anchor: RiseGuideAnchor) -> Option<AnyWindowHandle> {
        Self::live_view(anchor)?.inner.window.get()
    }

    /// whether two frames are the same frame, for callers following an anchor over time
    /// shares the registry tolerance so a follower and the settle loop agree about movement
    pub fn are_same_frame(lhs: Option<Bounds<Pixels>>, rhs: Option<Bounds<Pixels>>) -> bool {
        is_stable(lhs, rhs)
    }

    /// how often a follower with no frame loop should look, same cadence as the settle loop
    pub fn follow_interval_ms() -> i64 {
        Self::POLL_INTERVAL_MS
    }

    fn register_cover(cover: &GuideAnchorCoverView) {
        REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            registry.covers.retain(|weak| weak.strong_count() > 0);
            if !registry.covers.iter().any(|weak| weak.as_ptr() == Rc::as_ptr(&cover.inner)) {
                registry.covers.push(Rc::downgrade(&cover.inner));
            }
        });
    }

    /// live covers of one window or of all, each with its own serial
    fn live_covers(window: Option<AnyWindowHandle>) -> Vec<LiveCover> {
        REGISTRY.with(|registry| {
            registry
                .borrow()
                .covers
                .iter()
                .filter_map(Weak::upgrade)
                .filter_map(|cover| {
                    let cover_window = cover.window.get()?;
                    if window.is_some_and(|window| window != cover_window) {
                        return None;
                    }
                    Some(LiveCover {
                        serial: cover.serial,
                        kind: cover.kind.get(),
                        viewport: cover.viewport.get(),
                        frame: cover.frame.get()?,
                        order: cover.order.get(),
                    })
                })
                .collect()
        })
    }

    pub(crate) fn layer_at(window: AnyWindowHandle, at: Point<Pixels>) -> u64 {
        Self::live_covers(Some(window))
            .iter()
            .filter(|cover| cover.frame.contains(&at))
            .max_by_key(|cover| cover.order)
            .map_or(0, |cover| cover.serial)
    }

    pub(crate) fn layer_over(window: AnyWindowHandle, at: Point<Pixels>, own_layer: u64, own_order: u64) -> u64 {
        Self::live_covers(Some(window))
            .iter()
            .filter(|cover| cover.serial == own_layer || cover.order > own_order)
            .filter(|cover| cover.frame.contains(&at))
            .max_by_key(|cover| cover.order)
            .map_or(0, |cover| cover.serial)
    }

    pub(crate) fn trimmed_by_chrome(
        window: AnyWindowHandle,
        layer: u64,
        anchor_order: u64,
        clip: Bounds<Pixels>,
    ) -> Bounds<Pixels> {
        let covers = Self::live_covers(Some(window));
        let own_order =
            covers.iter().find(|cover| cover.serial == layer).map_or(0, |cover| cover.order).max(anchor_order);
        let mut top = f32::from(clip.origin.y);
        let mut bottom = top + f32::from(clip.size.height);
        let left = f32::from(clip.origin.x);
        let right = left + f32::from(clip.size.width);
        for cover in covers
            .iter()
            .filter(|cover| cover.kind == GuideAnchorCoverKind::Chrome && cover.serial != layer && cover.order > own_order)
        {
            let cover_top = f32::from(cover.frame.origin.y);
            let cover_bottom = cover_top + f32::from(cover.frame.size.height);
            let cover_left = f32::from(cover.frame.origin.x);
            let cover_right = cover_left + f32::from(cover.frame.size.width);
            let spans = cover_left <= left + 1.0 && cover_right >= right - 1.0;
            if !spans || cover_bottom <= top || cover_top >= bottom {
                continue;
            }
            if cover_top <= top {
                top = cover_bottom.min(bottom);
            } else if cover_bottom >= bottom {
                bottom = cover_top.max(top);
            }
        }
        Bounds { origin: point(clip.origin.x, px(top)), size: Size { width: clip.size.width, height: px((bottom - top).max(0.0)) } }
    }

    pub(crate) fn menu_covers() -> Vec<GuideAnchorCoverFrame> {
        let mut covers: Vec<GuideAnchorCoverFrame> = Self::live_covers(None)
            .into_iter()
            .filter(|cover| matches!(cover.kind, GuideAnchorCoverKind::Alert | GuideAnchorCoverKind::Menu))
            .map(|cover| GuideAnchorCoverFrame {
                kind: cover.kind,
                viewport: cover.viewport,
                frame: cover.frame,
                order: cover.order,
            })
            .collect();
        covers.sort_by(|lhs, rhs| rhs.order.cmp(&lhs.order));
        covers
    }

    fn sample(anchor: RiseGuideAnchor) -> Option<Bounds<Pixels>> {
        let measured = Self::live_view(anchor).and_then(|view| view.visible_window_frame());
        REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            let previous = registry.last_frames.get(&anchor).copied();
            if is_stable(previous, measured) {
                return;
            }
            match measured {
                Some(measured) => {
                    registry.last_frames.insert(anchor, measured);
                }
                None => {
                    registry.last_frames.remove(&anchor);
                }
            }
            registry.generation = registry.generation.wrapping_add(1);
        });
        measured
    }

    fn invalidate(registry: &mut RegistryState, anchor: RiseGuideAnchor) {
        registry.last_frames.remove(&anchor);
        registry.generation = registry.generation.wrapping_add(1);
    }

    /// element that answers for the anchor right now
    /// several live elements may share a name, eg both screens of a push hold a composer
    /// among drawn ones the latest registration wins, the first lit the screen that was leaving
    fn live_view(anchor: RiseGuideAnchor) -> Option<GuideAnchorBackingView> {
        let mounted: Vec<GuideAnchorBackingView> = REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            let boxes = registry.registrations.get_mut(&anchor)?;
            boxes.retain(|weak| weak.strong_count() > 0);
            let alive: Vec<GuideAnchorBackingView> = boxes
                .iter()
                .filter_map(Weak::upgrade)
                .map(|inner| GuideAnchorBackingView { inner })
                .collect();
            if alive.is_empty() {
                registry.registrations.remove(&anchor);
                return None;
            }
            Some(alive.into_iter().filter(|view| view.inner.window.get().is_some()).collect())
        })?;
        let displayed: Vec<&GuideAnchorBackingView> =
            mounted.iter().filter(|view| view.window_frame().is_some()).collect();
        if displayed.len() > 1
            && let Some(reachable) = displayed.iter().rev().find(|view| Self::is_on_top(view))
        {
            return Some((*reachable).clone());
        }
        if let Some(displayed) = displayed.last() {
            return Some((*displayed).clone());
        }
        mounted.last().cloned()
    }

    /// whether sighting would call this element visible, inside its window and not under another layer
    /// only asked when several drawn elements share a name
    fn is_on_top(view: &GuideAnchorBackingView) -> bool {
        let Some(window) = view.inner.window.get() else { return false };
        let Some(frame) = view.visible_window_frame() else { return false };
        let window_bounds = Bounds { origin: Point::default(), size: view.inner.viewport.get() };
        let clip = view.inner.clip.get().unwrap_or(frame);
        window_bounds.intersects(&frame) && has_area(clip) && Self::is_reachable(view, window, window_bounds, clip)
    }

    /// one line of what the sighting saw for the trace
    /// which elements answer for the anchor and their layers
    /// plus live covers and the layer each probe point lands on
    pub(crate) fn describe_sighting(anchor: RiseGuideAnchor) -> String {
        let registrations: Vec<GuideAnchorBackingView> = REGISTRY.with(|registry| {
            registry
                .borrow()
                .registrations
                .get(&anchor)
                .map(|boxes| boxes.iter().filter_map(Weak::upgrade).map(|inner| GuideAnchorBackingView { inner }).collect())
                .unwrap_or_default()
        });
        let elements: Vec<String> = registrations
            .iter()
            .map(|view| {
                format!(
                    "(window={:?} layer={} frame={:?} clip={:?})",
                    view.inner.window.get().map(|window| window.window_id()),
                    view.inner.layer.get(),
                    view.window_frame(),
                    view.inner.clip.get()
                )
            })
            .collect();
        let chosen = Self::live_view(anchor);
        let covers: Vec<String> = Self::live_covers(None)
            .iter()
            .map(|cover| format!("(serial={} kind={:?} order={} frame={:?})", cover.serial, cover.kind, cover.order, cover.frame))
            .collect();
        let probes: Vec<u64> = chosen
            .as_ref()
            .and_then(|view| Some((view.inner.window.get()?, view.visible_window_frame()?)))
            .map(|(window, clip)| probe_points(clip).iter().map(|probe| Self::layer_at(window, *probe)).collect())
            .unwrap_or_default();
        format!(
            "elements={} chosen_layer={:?} covers={} probe_layers={:?}",
            elements.join(","),
            chosen.as_ref().map(|view| view.inner.layer.get()),
            covers.join(","),
            probes
        )
    }
}

struct LiveCover {
    serial: u64,
    kind: GuideAnchorCoverKind,
    viewport: Size<Pixels>,
    frame: Bounds<Pixels>,
    order: u64,
}

/// five points and not one, a single center probe fails twice
/// a wide row middle can fall in a gap between children
/// and a partial sheet leaves part of the element usable
fn probe_points(rect: Bounds<Pixels>) -> [Point<Pixels>; 5] {
    let min_x = f32::from(rect.origin.x);
    let min_y = f32::from(rect.origin.y);
    let width = f32::from(rect.size.width);
    let height = f32::from(rect.size.height);
    let inset_x = width / 4.0;
    let inset_y = height / 4.0;
    let max_x = min_x + width;
    let max_y = min_y + height;
    [
        point(px(min_x + width / 2.0), px(min_y + height / 2.0)),
        point(px(min_x + inset_x), px(min_y + inset_y)),
        point(px(max_x - inset_x), px(min_y + inset_y)),
        point(px(min_x + inset_x), px(max_y - inset_y)),
        point(px(max_x - inset_x), px(max_y - inset_y)),
    ]
}

fn has_area(rect: Bounds<Pixels>) -> bool {
    f32::from(rect.size.width) > 1.0 && f32::from(rect.size.height) > 1.0
}

fn is_stable(lhs: Option<Bounds<Pixels>>, rhs: Option<Bounds<Pixels>>) -> bool {
    match (lhs, rhs) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            let tolerance = GuideAnchorRegistry::STABILITY_TOLERANCE;
            (f32::from(left.origin.x) - f32::from(right.origin.x)).abs() <= tolerance
                && (f32::from(left.origin.y) - f32::from(right.origin.y)).abs() <= tolerance
                && (f32::from(left.size.width) - f32::from(right.size.width)).abs() <= tolerance
                && (f32::from(left.size.height) - f32::from(right.size.height)).abs() <= tolerance
        }
        _ => false,
    }
}

/// monotonic, so a clock correction mid reveal cant make a frame look settled for a day or never settle
pub(crate) fn monotonic_ms() -> i64 {
    i64::try_from(RiseClock::monotonic().as_millis()).unwrap_or(i64::MAX)
}

pub struct GuideAnchorCover {
    id: ElementId,
    kind: GuideAnchorCoverKind,
    child: AnyElement,
}

impl GuideAnchorCover {
    /// id must be unique among the layers of one window (the overlay id of the layer)
    pub fn new(id: impl Into<ElementId>, kind: GuideAnchorCoverKind, child: AnyElement) -> Self {
        Self { id: id.into(), kind, child }
    }
}

impl IntoElement for GuideAnchorCover {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GuideAnchorCover {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
        let serial = id.map(|id| {
            let kind = self.kind;
            window.with_element_state::<GuideAnchorCoverView, _>(id, |cover, window| {
                let cover = cover.unwrap_or_else(|| GuideAnchorCoverView {
                    inner: Rc::new(GuideAnchorCoverInner {
                        serial: next_serial(),
                        kind: Cell::new(kind),
                        window: Cell::new(None),
                        viewport: Cell::new(Size::default()),
                        frame: Cell::new(None),
                        order: Cell::new(0),
                    }),
                });
                cover.inner.kind.set(kind);
                cover.inner.window.set(Some(window.window_handle()));
                cover.inner.viewport.set(window.viewport_size());
                cover.inner.frame.set(Some(bounds.intersect(&window.content_mask().bounds)));
                cover.inner.order.set(next_paint_order());
                GuideAnchorRegistry::register_cover(&cover);
                (cover.inner.serial, cover)
            })
        });
        if let Some(serial) = serial {
            LAYER_STACK.with(|stack| stack.borrow_mut().push(serial));
        }
        self.child.prepaint(window, cx);
        if serial.is_some() {
            LAYER_STACK.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
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

/// what an overlay layer of the kit puts around itself
pub trait GuideAnchorCoverModifiers: IntoElement + Sized {
    fn guide_anchor_cover(self, id: impl Into<ElementId>, kind: GuideAnchorCoverKind) -> GuideAnchorCover {
        GuideAnchorCover::new(id, kind, self.into_any_element())
    }
}

impl<E: IntoElement> GuideAnchorCoverModifiers for E {}
