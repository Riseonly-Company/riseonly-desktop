use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::rc::{Rc, Weak};

use gpui::{
    AnyWindowHandle, App, Bounds, ElementId, EntityId, IntoElement, Pixels, Point, RenderOnce, Styled, Window,
    canvas,
};

use crate::animated_media_playback_scope::AnimatedMediaPlaybackActiveKey;
use crate::message_bubble_geometry::MessageBubbleRadii;
use crate::message_media_tail_presentation::MessageMediaTailPresentation;
use crate::swiftui::environment::env;

pub type WindowScreenFrameChange = Rc<dyn Fn(Bounds<Pixels>, &mut Window, &mut App)>;

pub enum WindowScreenFrameRefreshPolicy {}

impl WindowScreenFrameRefreshPolicy {
    pub fn requires_scroll_subscription(has_change_consumer: bool) -> bool {
        has_change_consumer
    }
}

fn is_drawable(frame: &Bounds<Pixels>) -> bool {
    f32::from(frame.size.width) > 1.0 && f32::from(frame.size.height) > 1.0
}

/// owner token compared by identity
fn same_owner(stored: &Weak<dyn Any>, owner: &Rc<dyn Any>) -> bool {
    std::ptr::addr_eq(stored.as_ptr(), Rc::as_ptr(owner))
}

struct Record {
    view: Option<Weak<FrameViewInner>>,
    screen_frame: Bounds<Pixels>,
}

struct MediaTailRecord {
    owner: Weak<dyn Any>,
    presentation: MessageMediaTailPresentation,
}

struct SourceMediaRadiiRecord {
    owner: Weak<dyn Any>,
    radii: MessageBubbleRadii,
}

#[derive(Default)]
struct MediaCellScreenFrameRegistryState {
    records: HashMap<String, Record>,
    media_tail_presentations: HashMap<String, MediaTailRecord>,
    source_media_radii_records: HashMap<String, SourceMediaRadiiRecord>,
}

thread_local! {
    static REGISTRY: RefCell<MediaCellScreenFrameRegistryState> =
        RefCell::new(MediaCellScreenFrameRegistryState::default());
}

fn with_registry<R>(f: impl FnOnce(&mut MediaCellScreenFrameRegistryState) -> R) -> R {
    REGISTRY.with(|registry| f(&mut registry.borrow_mut()))
}

pub enum MediaCellScreenFrameRegistry {}

impl MediaCellScreenFrameRegistry {
    pub fn set(id: &str, screen_frame: Bounds<Pixels>, view: Option<&WindowScreenFrameReaderFrameView>) {
        let view = view.map(|view| Rc::downgrade(&view.inner));
        with_registry(|registry| {
            registry.records.insert(id.to_owned(), Record { view, screen_frame });
        });
    }

    pub fn frame(id: &str) -> Option<Bounds<Pixels>> {
        with_registry(|registry| {
            let record = registry.records.get_mut(id)?;
            if let Some(live) = record.view.as_ref().and_then(Weak::upgrade).and_then(|view| view.screen_frame()) {
                record.screen_frame = live;
                return Some(live);
            }
            Some(record.screen_frame)
        })
    }

    /// hit testing variant, a view backed record answers only while its view is attached
    /// cached rect says where a recycled tile was, a click there belongs to the row now in that spot
    pub fn attached_frame(id: &str) -> Option<Bounds<Pixels>> {
        with_registry(|registry| {
            let record = registry.records.get_mut(id)?;
            match &record.view {
                Some(view) => {
                    let live = view.upgrade().and_then(|view| view.screen_frame())?;
                    record.screen_frame = live;
                    Some(live)
                }
                None => Some(record.screen_frame),
            }
        })
    }

    pub fn set_media_tail_presentation(
        presentation: Option<MessageMediaTailPresentation>,
        id: &str,
        owner: &Rc<dyn Any>,
    ) {
        with_registry(|registry| match presentation {
            Some(presentation) => {
                registry
                    .media_tail_presentations
                    .insert(id.to_owned(), MediaTailRecord { owner: Rc::downgrade(owner), presentation });
            }
            None => {
                if registry.media_tail_presentations.get(id).is_some_and(|record| same_owner(&record.owner, owner)) {
                    registry.media_tail_presentations.remove(id);
                }
            }
        });
    }

    pub fn media_tail_presentation(id: &str) -> Option<MessageMediaTailPresentation> {
        with_registry(|registry| {
            let live = registry.media_tail_presentations.get(id).is_some_and(|record| record.owner.strong_count() > 0);
            if !live {
                registry.media_tail_presentations.remove(id);
                return None;
            }
            registry.media_tail_presentations.get(id).map(|record| record.presentation)
        })
    }

    pub fn set_source_media_radii(radii: Option<MessageBubbleRadii>, id: &str, owner: &Rc<dyn Any>) {
        with_registry(|registry| match radii {
            Some(radii) => {
                registry
                    .source_media_radii_records
                    .insert(id.to_owned(), SourceMediaRadiiRecord { owner: Rc::downgrade(owner), radii });
            }
            None => {
                if registry.source_media_radii_records.get(id).is_some_and(|record| same_owner(&record.owner, owner)) {
                    registry.source_media_radii_records.remove(id);
                }
            }
        });
    }

    pub fn source_media_radii(id: &str) -> Option<MessageBubbleRadii> {
        with_registry(|registry| {
            let live = registry.source_media_radii_records.get(id).is_some_and(|record| record.owner.strong_count() > 0);
            if !live {
                registry.source_media_radii_records.remove(id);
                return None;
            }
            registry.source_media_radii_records.get(id).map(|record| record.radii)
        })
    }

    pub fn remove(id: &str) {
        with_registry(|registry| {
            registry.records.remove(id);
        });
    }

    /// removes the record only while view still owns it or its view is gone
    /// so a dismantled reader never drops a live reader frame for the same id
    pub fn remove_if_owned_by(id: &str, view: &WindowScreenFrameReaderFrameView) {
        Self::remove_if_owned_by_inner(id, &view.inner);
    }

    fn remove_if_owned_by_inner(id: &str, view: &Rc<FrameViewInner>) {
        with_registry(|registry| {
            let Some(record) = registry.records.get(id) else { return };
            let owned = match record.view.as_ref().and_then(Weak::upgrade) {
                None => true,
                Some(owner) => Rc::ptr_eq(&owner, view),
            };
            if owned {
                registry.records.remove(id);
            }
        });
    }

    /// view behind a record, only while attached to a window
    pub fn attached_view(id: &str) -> Option<WindowScreenFrameReaderFrameView> {
        with_registry(|registry| {
            let inner = registry.records.get(id)?.view.as_ref()?.upgrade()?;
            inner.attached.get().then_some(WindowScreenFrameReaderFrameView { inner })
        })
    }
}

pub enum WindowScreenCoordinates {}

impl WindowScreenCoordinates {
    /// a window point is already in the space every in window overlay uses
    pub fn screen_point(from_window_point: Point<Pixels>) -> Point<Pixels> {
        from_window_point
    }

    pub fn screen_frame(view: &WindowScreenFrameReaderFrameView) -> Option<Bounds<Pixels>> {
        view.inner.screen_frame()
    }
}

/// what the reader saw when it was painted last
#[derive(Clone, Copy, Debug, PartialEq)]
struct FramePaint {
    window: AnyWindowHandle,
    frame: Bounds<Pixels>,
    viewport_clip: Bounds<Pixels>,
    is_animated_media_playback_active: bool,
}

struct FrameViewInner {
    cell_id: RefCell<Option<String>>,
    source_media_radii: Cell<Option<MessageBubbleRadii>>,
    on_change: RefCell<Option<WindowScreenFrameChange>>,
    painted: Cell<Option<FramePaint>>,
    attached: Cell<bool>,
    is_subscribed_to_scroll: Cell<bool>,
    drawn_by: Cell<Option<EntityId>>,
}

impl FrameViewInner {
    fn screen_frame(&self) -> Option<Bounds<Pixels>> {
        if !self.attached.get() {
            return None;
        }
        let frame = self.painted.get()?.frame;
        is_drawable(&frame).then_some(frame)
    }
}

/// backing view of the reader, a shared handle to the frame it reports
/// the registry keeps it weakly
#[derive(Clone)]
pub struct WindowScreenFrameReaderFrameView {
    inner: Rc<FrameViewInner>,
}

impl WindowScreenFrameReaderFrameView {
    fn new() -> Self {
        Self {
            inner: Rc::new(FrameViewInner {
                cell_id: RefCell::new(None),
                source_media_radii: Cell::new(None),
                on_change: RefCell::new(None),
                painted: Cell::new(None),
                attached: Cell::new(false),
                is_subscribed_to_scroll: Cell::new(false),
                drawn_by: Cell::new(None),
            }),
        }
    }

    /// identity the registry compares owners by
    pub fn owner(&self) -> Rc<dyn Any> {
        self.inner.clone()
    }

    pub fn cell_id(&self) -> Option<String> {
        self.inner.cell_id.borrow().clone()
    }

    pub fn source_media_radii(&self) -> Option<MessageBubbleRadii> {
        self.inner.source_media_radii.get()
    }

    pub fn window(&self) -> Option<AnyWindowHandle> {
        self.inner.attached.get().then(|| self.inner.painted.get().map(|paint| paint.window)).flatten()
    }

    pub fn report_frame(&self, window: &mut Window, cx: &mut App) {
        let Some(id) = self.cell_id() else { return };
        let Some(screen) = WindowScreenCoordinates::screen_frame(self) else { return };
        MediaCellScreenFrameRegistry::set(&id, screen, Some(self));
        let on_change = self.inner.on_change.borrow().clone();
        if let Some(on_change) = on_change {
            on_change(screen, window, cx);
        }
    }

    fn did_move_to_window(&self, window: &mut Window, cx: &mut App) {
        self.update_scroll_subscription();
        let view = self.clone();
        window.defer(cx, move |window, cx| view.report_frame(window, cx));
    }

    fn layout_subviews(&self, window: &mut Window, cx: &mut App) {
        let view = self.clone();
        window.defer(cx, move |window, cx| view.report_frame(window, cx));
    }

    fn set_on_change(&self, on_change: Option<WindowScreenFrameChange>) {
        *self.inner.on_change.borrow_mut() = on_change;
        self.update_scroll_subscription();
    }

    fn update_scroll_subscription(&self) {
        let should_subscribe = self.inner.attached.get()
            && WindowScreenFrameRefreshPolicy::requires_scroll_subscription(self.inner.on_change.borrow().is_some());
        if should_subscribe && !self.inner.is_subscribed_to_scroll.get() {
            let weak = Rc::downgrade(&self.inner);
            ScrollFrameSync::shared().subscribe(
                self,
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    if let Some(inner) = weak.upgrade() {
                        WindowScreenFrameReaderFrameView { inner }.report_frame(window, cx);
                    }
                }),
            );
            self.inner.is_subscribed_to_scroll.set(true);
        } else if !should_subscribe && self.inner.is_subscribed_to_scroll.get() {
            self.stop_scroll_subscription();
        }
    }

    fn stop_scroll_subscription(&self) {
        if !self.inner.is_subscribed_to_scroll.get() {
            return;
        }
        ScrollFrameSync::shared().unsubscribe(self);
        self.inner.is_subscribed_to_scroll.set(false);
    }

    /// visibility for autoplaying chat media, frame clipped by every clipping ancestor and the window
    /// inside an active playback scope, a cell stops only after its frame leaves every clipping ancestor
    pub fn is_visible_in_active_clipped_viewport(&self) -> bool {
        self.is_visible_in_active_clipped_viewport_with(0.0, true)
    }

    pub fn is_visible_in_active_clipped_viewport_ignoring_playback_scope(&self) -> bool {
        self.is_visible_in_active_clipped_viewport_with(0.0, false)
    }

    /// gpui stops drawing a hidden or minimised window and that detaches its readers
    /// so drawn is the check here
    pub fn is_visible_in_active_clipped_viewport_with(
        &self,
        minimum_visible_height_fraction: f32,
        requires_active_playback_scope: bool,
    ) -> bool {
        if !self.inner.attached.get() {
            return false;
        }
        let Some(paint) = self.inner.painted.get() else { return false };
        if !is_drawable(&paint.frame) {
            return false;
        }
        if requires_active_playback_scope && !paint.is_animated_media_playback_active {
            return false;
        }
        if !paint.frame.intersects(&paint.viewport_clip) {
            return false;
        }
        let visible = paint.frame.intersect(&paint.viewport_clip);
        let threshold = minimum_visible_height_fraction.clamp(0.0, 1.0);
        f32::from(visible.size.width) > 1.0
            && f32::from(visible.size.height) / f32::from(paint.frame.size.height) > threshold
    }
}

pub type ScrollFrameSyncRefresh = Rc<dyn Fn(&mut Window, &mut App)>;

struct ScrollFrameSyncEntry {
    view_key: *const FrameViewInner,
    view: Weak<FrameViewInner>,
    refresh: ScrollFrameSyncRefresh,
}

#[derive(Default)]
struct ScrollFrameSyncState {
    subscribers: Vec<ScrollFrameSyncEntry>,
    coalesced_notify: HashSet<AnyWindowHandle>,
}

thread_local! {
    static SCROLL_FRAME_SYNC: RefCell<ScrollFrameSyncState> = RefCell::new(ScrollFrameSyncState::default());
}

/// handle to the main thread sync, not Send so it never leaves its thread
#[derive(Clone, Copy)]
pub struct ScrollFrameSync {
    _main_thread: PhantomData<*const ()>,
}

impl ScrollFrameSync {
    pub fn shared() -> Self {
        Self { _main_thread: PhantomData }
    }

    /// subscribing twice is a no op, the refresh is the same every call
    pub fn subscribe(&self, view: &WindowScreenFrameReaderFrameView, refresh: ScrollFrameSyncRefresh) {
        let view_key = Rc::as_ptr(&view.inner);
        SCROLL_FRAME_SYNC.with(|sync| {
            let mut sync = sync.borrow_mut();
            sync.subscribers.retain(|entry| entry.view.strong_count() > 0);
            if !sync.subscribers.iter().any(|entry| std::ptr::eq(entry.view_key, view_key)) {
                sync.subscribers.push(ScrollFrameSyncEntry { view_key, view: Rc::downgrade(&view.inner), refresh });
            }
        });
    }

    pub fn unsubscribe(&self, view: &WindowScreenFrameReaderFrameView) {
        self.unsubscribe_inner(&view.inner);
    }

    fn unsubscribe_inner(&self, view: &Rc<FrameViewInner>) {
        SCROLL_FRAME_SYNC.with(|sync| {
            sync.borrow_mut().subscribers.retain(|entry| {
                entry.view.strong_count() > 0 && !std::ptr::eq(entry.view_key, Rc::as_ptr(view))
            });
        });
    }

    pub fn refresh_subscribers(&self, window: AnyWindowHandle, intersecting: Bounds<Pixels>, cx: &mut App) {
        let refreshes: Vec<(ScrollFrameSyncRefresh, Option<EntityId>)> = SCROLL_FRAME_SYNC.with(|sync| {
            let mut sync = sync.borrow_mut();
            sync.subscribers.retain(|entry| entry.view.strong_count() > 0);
            sync.subscribers
                .iter()
                .filter(|entry| {
                    entry.view.upgrade().is_some_and(|inner| {
                        inner.attached.get()
                            && inner
                                .painted
                                .get()
                                .is_some_and(|paint| paint.window == window && paint.frame.intersects(&intersecting))
                    })
                })
                .map(|entry| (entry.refresh.clone(), entry.view.upgrade().and_then(|inner| inner.drawn_by.get())))
                .collect()
        });
        if refreshes.is_empty() {
            return;
        }
        let first = SCROLL_FRAME_SYNC.with(|sync| sync.borrow_mut().coalesced_notify.insert(window));
        if !first {
            return;
        }
        cx.defer(move |cx| {
            SCROLL_FRAME_SYNC.with(|sync| sync.borrow_mut().coalesced_notify.remove(&window));
            let _ = window.update(cx, |_, window, cx| {
                let mut views: Vec<EntityId> = Vec::new();
                let mut unattributed = false;
                for (refresh, drawn_by) in &refreshes {
                    refresh(window, cx);
                    match drawn_by {
                        Some(view) if !views.contains(view) => views.push(*view),
                        Some(_) => {}
                        None => unattributed = true,
                    }
                }
                if unattributed {
                    window.refresh();
                    return;
                }
                for view in views {
                    cx.notify(view);
                }
            });
        });
    }
}

/// element state behind one reader, alive while the reader is drawn
/// cleanup on drop
struct FrameViewBacking {
    view: WindowScreenFrameReaderFrameView,
}

impl Drop for FrameViewBacking {
    fn drop(&mut self) {
        WindowScreenFrameReader::dismantle_ui_view(&self.view.inner);
    }
}

/// transparent layer over its parent that registers the parent frame under id
/// calls on_change after every frame that moved it, no hitbox so no clicks
#[derive(IntoElement)]
pub struct WindowScreenFrameReader {
    id: String,
    source_media_radii: Option<MessageBubbleRadii>,
    on_change: Option<WindowScreenFrameChange>,
}

impl WindowScreenFrameReader {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), source_media_radii: None, on_change: None }
    }

    pub fn source_media_radii(mut self, radii: Option<MessageBubbleRadii>) -> Self {
        self.source_media_radii = radii;
        self
    }

    pub fn on_change(mut self, on_change: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(on_change));
        self
    }
}

impl WindowScreenFrameReader {
    fn make_ui_view(&self) -> WindowScreenFrameReaderFrameView {
        let view = WindowScreenFrameReaderFrameView::new();
        *view.inner.cell_id.borrow_mut() = Some(self.id.clone());
        view.inner.source_media_radii.set(self.source_media_radii);
        MediaCellScreenFrameRegistry::set_source_media_radii(self.source_media_radii, &self.id, &view.owner());
        view
    }

    fn update_ui_view(self, view: &WindowScreenFrameReaderFrameView) {
        let owner = view.owner();
        let previous = view.cell_id();
        if let Some(old) = previous.as_ref().filter(|old| **old != self.id) {
            MediaCellScreenFrameRegistry::set_source_media_radii(None, old, &owner);
            MediaCellScreenFrameRegistry::remove_if_owned_by(old, view);
        }
        let changed = previous.as_deref() != Some(self.id.as_str()) || view.source_media_radii() != self.source_media_radii;
        *view.inner.cell_id.borrow_mut() = Some(self.id.clone());
        view.inner.source_media_radii.set(self.source_media_radii);
        view.set_on_change(self.on_change);
        if changed {
            MediaCellScreenFrameRegistry::set_source_media_radii(self.source_media_radii, &self.id, &owner);
        }
    }

    fn dismantle_ui_view(view: &Rc<FrameViewInner>) {
        view.attached.set(false);
        *view.on_change.borrow_mut() = None;
        if view.is_subscribed_to_scroll.get() {
            ScrollFrameSync::shared().unsubscribe_inner(view);
            view.is_subscribed_to_scroll.set(false);
        }
        let id = view.cell_id.borrow().clone();
        if let Some(id) = id {
            let owner: Rc<dyn Any> = view.clone();
            MediaCellScreenFrameRegistry::set_source_media_radii(None, &id, &owner);
            MediaCellScreenFrameRegistry::remove_if_owned_by_inner(&id, view);
        }
    }
}

impl RenderOnce for WindowScreenFrameReader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let element_id = ElementId::Name(format!("window-screen-frame-reader-{}", self.id).into());
        canvas(
            move |bounds, window, cx| {
                let paint = FramePaint {
                    window: window.window_handle(),
                    frame: bounds,
                    viewport_clip: window.content_mask().bounds,
                    is_animated_media_playback_active: env::<AnimatedMediaPlaybackActiveKey>(cx),
                };
                let reader = self;
                let view = window.with_global_id(element_id, |global_id, window| {
                    window.with_element_state::<FrameViewBacking, _>(global_id, |backing, _| {
                        let backing = backing.unwrap_or_else(|| FrameViewBacking { view: reader.make_ui_view() });
                        (backing.view.clone(), backing)
                    })
                });
                view.inner.drawn_by.set(gpui::rendering_view());
                let was_attached = view.inner.attached.replace(true);
                let moved = view.inner.painted.replace(Some(paint)) != Some(paint);
                reader.update_ui_view(&view);
                if !was_attached {
                    view.did_move_to_window(window, cx);
                } else if moved {
                    view.layout_subviews(window, cx);
                }
                let moved_view = view.clone();
                window.on_replayed_move(move |offset, mask_offset, window, cx| {
                    let paint = FramePaint {
                        frame: paint.frame + offset,
                        viewport_clip: paint.viewport_clip + mask_offset,
                        ..paint
                    };
                    if moved_view.inner.painted.replace(Some(paint)) != Some(paint) {
                        moved_view.layout_subviews(window, cx);
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}
