use crate::{
    AnyElement, AnyEntity, AnyWeakEntity, App, Bounds, ContentMask, Context, Element, ElementId,
    Entity, EntityId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, PaintIndex,
    Pixels, Point, PrepaintStateIndex, Render, RenderOnce, Style, StyleRefinement, TextStyle, WeakEntity,
};
use crate::window::{HoleRecord, HoleStacks, PaintContext, SpliceMap};
use crate::{Empty, Window};
use anyhow::Result;
use collections::FxHashSet;
use refineable::Refineable;
use std::mem;
use std::rc::Rc;
use std::{any::TypeId, fmt, ops::Range};

/// A dynamically-typed view handle that can be downcast to a specific `Entity<V>`.
///
/// This is the type-erased counterpart to [`ViewElement`]: it holds an entity plus
/// a function pointer to its render, and is itself a [`View`], so embedding it as an
/// element goes through the same [`ViewElement`] machinery as any other view.
#[derive(Clone, Debug)]
pub struct AnyView {
    entity: AnyEntity,
    render: fn(&AnyView, &mut Window, &mut App) -> AnyElement,
}

impl<V: Render> From<Entity<V>> for AnyView {
    fn from(value: Entity<V>) -> Self {
        AnyView {
            entity: value.into_any(),
            render: any_view::render::<V>,
        }
    }
}

impl AnyView {
    /// Embed this view as a cached [`ViewElement`] laid out at `style`.
    ///
    /// The rendered subtree is recycled from the previous frame unless
    /// [Context::notify] was called on the backing entity since it was rendered
    /// (or [Window::refresh] is called, which ignores caching).
    pub fn cached(self, style: StyleRefinement) -> ViewElement<AnyView> {
        let mut element = ViewElement::new(self).cached(style);
        element.to_any = Some(|view: &AnyView| view.clone());
        element
    }

    /// Convert this to a weak handle.
    pub fn downgrade(&self) -> AnyWeakView {
        AnyWeakView {
            entity: self.entity.downgrade(),
            render: self.render,
        }
    }

    /// Convert this to a [Entity] of a specific type.
    /// If this handle does not contain a view of the specified type, returns itself in an `Err` variant.
    pub fn downcast<T: 'static>(self) -> Result<Entity<T>, Self> {
        match self.entity.downcast() {
            Ok(entity) => Ok(entity),
            Err(entity) => Err(Self {
                entity,
                render: self.render,
            }),
        }
    }

    /// Gets the [TypeId] of the underlying view.
    pub fn entity_type(&self) -> TypeId {
        self.entity.entity_type
    }

    /// The [`EntityId`] of this view.
    pub fn entity_id(&self) -> EntityId {
        self.entity.entity_id()
    }
}

impl PartialEq for AnyView {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl Eq for AnyView {}

/// `AnyView` is the type-erased [`View`]: its `render` is a function pointer rather
/// than a concrete type, but it participates in the reactive graph exactly like any
/// other view via [`ViewElement`].
impl View for AnyView {
    fn entity_id(&self) -> Option<EntityId> {
        Some(self.entity.entity_id())
    }

    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        (self.render)(&self, window, cx)
    }
}

impl<V: 'static + Render> IntoElement for Entity<V> {
    type Element = ViewElement<Entity<V>>;

    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }
}

impl IntoElement for AnyView {
    type Element = ViewElement<AnyView>;

    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }
}

/// A weak, dynamically-typed view handle.
pub struct AnyWeakView {
    entity: AnyWeakEntity,
    render: fn(&AnyView, &mut Window, &mut App) -> AnyElement,
}

impl AnyWeakView {
    /// Upgrade to a strong `AnyView` handle, if the view is still alive.
    pub fn upgrade(&self) -> Option<AnyView> {
        let entity = self.entity.upgrade()?;
        Some(AnyView {
            entity,
            render: self.render,
        })
    }
}

impl<V: 'static + Render> From<WeakEntity<V>> for AnyWeakView {
    fn from(view: WeakEntity<V>) -> Self {
        AnyWeakView {
            entity: view.into(),
            render: any_view::render::<V>,
        }
    }
}

impl PartialEq for AnyWeakView {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl std::fmt::Debug for AnyWeakView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyWeakView")
            .field("entity_id", &self.entity.entity_id)
            .finish_non_exhaustive()
    }
}

mod any_view {
    use crate::{AnyElement, AnyView, App, IntoElement, Render, Window};

    pub(crate) fn render<V: 'static + Render>(
        view: &AnyView,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let view = view.clone().downcast::<V>().unwrap();
        // Record the view's Render type name so the accessibility debug dump can
        // attribute nodes to the view that produced them.
        #[cfg(debug_assertions)]
        window
            .a11y
            .view_type_names
            .insert(view.entity_id(), std::any::type_name::<V>());
        crate::profiler::note_view_rendered(std::any::type_name::<V>());
        crate::profiler::note_view_type(view.entity_id(), std::any::type_name::<V>());
        view.update(cx, |view, cx| view.render(window, cx).into_any_element())
    }
}

/// A renderable that participates in GPUI's reactive graph — the unifying model
/// behind [`Render`] and [`RenderOnce`].
///
/// When `entity_id()` returns `Some`, that id becomes the view's identity: it gets
/// a unique element-id space (so internal `use_state` / `.id(..)` never collide
/// across siblings) and `cx.notify()` on that entity re-renders only this view's
/// subtree. `None` behaves like a stateless component.
///
/// You rarely implement `View` directly. `Entity<T: Render>` and any `T: RenderOnce`
/// get a blanket impl below; implement it by hand only when a component needs both
/// parent-supplied props *and* a backing entity for identity.
pub trait View: 'static + Sized {
    /// This view's identity, if it has one. A view typically holds the backing
    /// entity as a field and returns its [`EntityId`] here.
    ///
    /// The id becomes this view's [`ElementId`], so two views keyed on the same
    /// entity must not be rendered at the same position in the element tree
    /// (e.g. as siblings under the same parent): their internal element state
    /// (`use_state`, scroll offsets, etc.) would silently collide. Nesting is
    /// fine — the id is scoped by the parent path.
    fn entity_id(&self) -> Option<EntityId>;

    /// Render this view into an element tree, consuming `self`.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement;
}

/// A stateless component (`RenderOnce`) is a `View` with no identity.
impl<T: RenderOnce> View for T {
    fn entity_id(&self) -> Option<EntityId> {
        None
    }

    #[inline]
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        RenderOnce::render(self, window, cx)
    }
}

/// An entity that renders itself (`Render`) is a `View` keyed on its own id.
impl<T: Render> View for Entity<T> {
    fn entity_id(&self) -> Option<EntityId> {
        Some(Entity::entity_id(self))
    }

    #[inline]
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        crate::profiler::note_view_rendered(std::any::type_name::<T>());
        crate::profiler::note_view_type(Entity::entity_id(&self), std::any::type_name::<T>());
        self.update(cx, |this, cx| {
            Render::render(this, window, cx).into_any_element()
        })
    }
}

impl<T: Render> Entity<T> {
    /// Embed this entity as a cached [`ViewElement`] laid out at `style`.
    ///
    /// The rendered subtree is reused until the entity is notified (or the
    /// cached bounds / text style change). Caching requires a definite size:
    /// a cached view is laid out from `style` and is *not* measured from its
    /// contents. Use [`ViewElement::new`] (or `.child(entity)`) for the
    /// uncached case.
    #[track_caller]
    pub fn cached(self, style: StyleRefinement) -> ViewElement<Entity<T>> {
        let mut element = ViewElement::new(self).cached(style);
        element.to_any = Some(|entity: &Entity<T>| AnyView::from(entity.clone()));
        element
    }
}

/// The element type for [`View`] implementations. Wraps a `View` and hooks it
/// into layout, prepaint, and paint. Constructed via [`ViewElement::new`].
#[doc(hidden)]
pub struct ViewElement<V: View> {
    view: Option<V>,
    entity_id: Option<EntityId>,
    cached_style: Option<StyleRefinement>,
    replays_moved: bool,
    /// Riseonly 0088: how to name this view to draw it again alone, set by the cached
    /// constructors, and what its prepaint captured for that.
    to_any: Option<fn(&V) -> AnyView>,
    hole_view: Option<AnyView>,
    hole_prepaint: Option<HoleStacks>,
    #[cfg(debug_assertions)]
    source: &'static core::panic::Location<'static>,
}

impl<V: View> ViewElement<V> {
    /// Wrap a [`View`] as an element.
    #[track_caller]
    pub fn new(view: V) -> Self {
        let entity_id = view.entity_id();
        ViewElement {
            entity_id,
            cached_style: None,
            replays_moved: false,
            to_any: None,
            hole_view: None,
            hole_prepaint: None,
            view: Some(view),
            #[cfg(debug_assertions)]
            source: core::panic::Location::caller(),
        }
    }

    /// Enable caching of this view's rendered subtree, laid out at `style`.
    /// The composer supplies the layout style because caching skips rendering
    /// the contents to measure them.
    ///
    /// Crate-private on purpose: caching is only sound for entity-backed views,
    /// where [`Context::notify`] is the contract that busts the cache. A stateless
    /// view has no such contract, so a frozen subtree could never be invalidated.
    /// Reach this through [`Entity::cached`] or [`AnyView::cached`], which are
    /// entity-backed by construction.
    pub(crate) fn cached(mut self, style: StyleRefinement) -> Self {
        self.cached_style = Some(style);
        self
    }

    /// Lets a cached view that only changed its origin (a cell scrolled within its clip) replay its
    /// last frame moved instead of rendering (Riseonly 0041). Only for views whose prepaint keeps no
    /// window position outside [`Window::on_replayed_move`] that anything but the mouse reads: input
    /// handlers, tooltips and deferred draws already make it render. Mouse listeners may replay
    /// moved (Riseonly 0044): a view with them renders while the mouse is over it, and the mouse
    /// entering one replayed moved since it last rendered makes it render in the next frame.
    pub fn replay_when_moved(mut self) -> Self {
        self.replays_moved = true;
        self
    }
}

/// Riseonly 0077: a cached view laid out wholly outside the clip it rendered under (its primitives
/// were dropped by the scene, not recorded) that the new clip reaches. Its replay would draw nothing.
/// Riseonly 0089: also a view the clip cut in part, when the new clip shows a part of it (taken
/// from its own origin) the old clip hid. The scene dropped every primitive cut away whole, so the
/// hidden part recorded nothing, and a view whose shown strip held only padding recorded nothing at
/// all: every range check passed on nothing and the replay stayed blank (a report queue row built
/// with its top padding peeking above the viewport bottom, X1876).
fn culled_then_revealed(
    old_bounds: Bounds<Pixels>,
    old_mask: Bounds<Pixels>,
    new_bounds: Bounds<Pixels>,
    new_mask: Bounds<Pixels>,
) -> bool {
    if old_bounds.is_empty() || !new_bounds.intersects(&new_mask) {
        return false;
    }
    if !old_bounds.intersects(&old_mask) {
        return true;
    }
    let old_shown = old_bounds.intersect(&old_mask);
    let new_shown = new_bounds.intersect(&new_mask);
    let old_left = f32::from(old_shown.left() - old_bounds.left());
    let old_top = f32::from(old_shown.top() - old_bounds.top());
    let old_right = f32::from(old_shown.right() - old_bounds.left());
    let old_bottom = f32::from(old_shown.bottom() - old_bounds.top());
    let new_left = f32::from(new_shown.left() - new_bounds.left());
    let new_top = f32::from(new_shown.top() - new_bounds.top());
    let new_right = f32::from(new_shown.right() - new_bounds.left());
    let new_bottom = f32::from(new_shown.bottom() - new_bounds.top());
    let slack = 0.001;
    new_left < old_left - slack
        || new_top < old_top - slack
        || new_right > old_right + slack
        || new_bottom > old_bottom + slack
}

impl<V: View> IntoElement for ViewElement<V> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Type of the element state a cached view keeps, so a rolled back transaction can drop the
/// states it recorded (Riseonly 0031).
pub(crate) fn view_element_state_type_id() -> TypeId {
    TypeId::of::<ViewElementState>()
}

struct ViewElementState {
    prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Range<PaintIndex>,
    /// `Window::draw_count` of the frame the ranges above were recorded in. They index the
    /// previous frame only; a view that was not prepainted or painted last frame (under a hidden
    /// div, or inside a cached ancestor that was replayed) holds ranges of an older frame.
    prepainted_frame: u64,
    painted_frame: u64,
    cache_key: ViewElementCacheKey,
    accessed_entities: Rc<FxHashSet<EntityId>>,
    /// Riseonly 0088: the part of `accessed_entities` the view read itself.
    own_entities: Rc<FxHashSet<EntityId>>,
    /// The last paint, taken out of the frames while a hidden div skips this view (Riseonly 0034).
    parked: Option<crate::window::ParkedPaint>,
    /// The render that recorded the prepaint ranges dropped a deferred draw under a hidden div.
    dropped_deferred: bool,
    /// Turned dirty while a hidden div held it: the replayed frame is old, so the frame that shows
    /// it again renders it fresh (Riseonly 0039).
    stale_while_hidden: bool,
    /// The prepaint replayed moved by this offset; the paint that follows replays moved too.
    moved_by: Option<Point<Pixels>>,
    /// What the primitives of the paint range were painted under (Riseonly 0053).
    painted_under: PaintContext,
    /// The recorded ranges hold mouse listeners and element states of an earlier origin: this view
    /// or a cached view inside it replayed moved since it last rendered (Riseonly 0044).
    positions_moved: bool,
    /// Riseonly 0091: the frame a nested replay (0045, 0080) last looked at this view, and why it
    /// left the view on its old frame (empty when it rebased it), for the `prepaint_gap` reason.
    rebase_seen: u64,
    rebase_refused: &'static str,
    /// Riseonly 0099: the paint range was recorded inside the view's own clip boundary, so its
    /// primitives keep only the clips inside the view and a replay anywhere is cut by the mask
    /// it lands under.
    relative_clips: bool,
}

impl ViewElementState {
    /// The offset a moved replay applies in this frame, if the view may take one (Riseonly 0041):
    /// shown, clean, the same size, text style and environment, painted in the frame just before,
    /// no position kept by a listener, moved by whole device pixels so glyphs and images keep their
    /// pixel snapping, and a mask that either moved along or cut nothing of it before and after.
    fn moved_offset(
        &self,
        entity_id: EntityId,
        bounds: Bounds<Pixels>,
        content_mask: &ContentMask<Pixels>,
        text_style: &TextStyle,
        env_fingerprint: u64,
        window: &Window,
    ) -> Option<(Point<Pixels>, Point<Pixels>)> {
        self.moved_replay(entity_id, bounds, content_mask, text_style, env_fingerprint, window).ok()
    }

    /// [`Self::moved_offset`], or the first check that refused the moved replay, for the frame
    /// trace (Riseonly 0076): an "origin" miss alone does not say whether a clip, the mouse, a
    /// fractional step or the environment made the view render.
    fn moved_replay(
        &self,
        entity_id: EntityId,
        bounds: Bounds<Pixels>,
        content_mask: &ContentMask<Pixels>,
        text_style: &TextStyle,
        env_fingerprint: u64,
        window: &Window,
    ) -> Result<(Point<Pixels>, Point<Pixels>), &'static str> {
        let key = &self.cache_key;
        let near = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() < 0.001;
        if window.hidden_depth > 0
            || window.refreshing
            || window.dirty_views.contains(&entity_id)
            || self.dropped_deferred
            || self.stale_while_hidden
        {
            return Err("origin:state");
        }
        if self.prepainted_frame + 1 != window.draw_count || self.painted_frame + 1 != window.draw_count {
            return Err("origin:gap");
        }
        if key.bounds.size != bounds.size || key.bounds.origin == bounds.origin {
            return Err("origin:bounds");
        }
        // Riseonly 0086: the text style and the environment are told apart, with the field or key
        // that changed, while the frame trace runs.
        if key.text_style != *text_style {
            return Err(if crate::profiler::frame_trace_enabled() {
                crate::profiler::detailed_reason("origin:text_style", text_style_change(&key.text_style, text_style))
            } else {
                "origin:text_style"
            });
        }
        if key.env_fingerprint != env_fingerprint {
            return Err(if crate::profiler::frame_trace_enabled() {
                crate::profiler::detailed_reason("origin:env", env_change(key.env_trace.as_deref(), &window.env_trace))
            } else {
                "origin:env"
            });
        }
        if !window.ranges_replay_moved(&self.prepaint_range, &self.paint_range) {
            return Err("origin:pinned");
        }
        // Riseonly 0077: the scene keeps no primitive its clip cuts away whole, so a view rendered
        // wholly outside its clip may have recorded nothing, and every range check below passes on
        // nothing: replayed into view it stayed blank (settings switches laid out below the viewport,
        // X1741). Its own bounds say what the empty ranges cannot; it renders when revealed.
        // Riseonly 0099: under its own clip boundary the view recorded what the mask cut away too.
        if !self.relative_clips
            && culled_then_revealed(key.bounds, key.content_mask.bounds, bounds, content_mask.bounds)
        {
            return Err("origin:culled");
        }
        // Riseonly 0067: shown nothing before and nothing after (a pager neighbour outside its clip
        // on a page sliding in): the replay draws the same nothing, so neither pixel snapping nor
        // the mouse matter, and the recorded clips move along with it.
        let offset = bounds.origin - key.bounds.origin;
        if window.ranges_hidden_by_mask(&self.prepaint_range, &self.paint_range, content_mask.bounds, offset) {
            return Ok((offset, offset));
        }
        // Riseonly 0074: shown nothing before, but maybe something now. A moved replay moves every
        // recorded clip with the view (Primitive::translate), and the clip that hid it was the outer
        // mask: replayed into the mask it would keep that clip and stay blank for good (gift grid
        // cells built below the viewport came up black, X1277). It renders instead.
        if !self.relative_clips && window.ranges_wholly_hidden(&self.prepaint_range, &self.paint_range) {
            return Err("origin:wholly_hidden");
        }
        // Riseonly 0044: listeners and element states keep the old origin, so a view the mouse is
        // over before or after the move renders: hover, clicks and drags read fresh positions.
        let mouse = window.mouse_position();
        if window.paint_range_listens(&self.paint_range)
            && (key.bounds.contains(&mouse) || bounds.contains(&mouse))
        {
            return Err("origin:mouse");
        }
        let device = offset.scale(window.scale_factor());
        let whole = |value: f32| (value - value.round()).abs() < 0.001;
        if !whole(device.x.0) || !whole(device.y.0) {
            return Err("origin:fraction");
        }
        // The mask moved along (a clipped cell scrolled whole): every recorded clip moves with it.
        let old_mask = key.content_mask.bounds;
        let new_mask = content_mask.bounds;
        let mask_moved_along = near(new_mask.origin.x, old_mask.origin.x + offset.x)
            && near(new_mask.origin.y, old_mask.origin.y + offset.y)
            && new_mask.size == old_mask.size;
        // The mask stayed (the scroll viewport) and cut nothing of the view before or after.
        let mask_stays_clear = key.content_mask == *content_mask
            && window.ranges_clear_of_mask(&self.prepaint_range, &self.paint_range, new_mask, offset);
        // Riseonly 0061: the mask changed shape (a page sliding into a clipped column: its own clip
        // moves along, the column's stays, so their intersection grows each frame) but cut nothing of
        // the view before nor cuts it after. The replay moves the recorded clips with the view, and
        // they still cut nothing, so the pixels and the hit test are the ones a render would give.
        let mask_reshaped_clear = !mask_moved_along
            && key.content_mask != *content_mask
            && window.ranges_clear_of_mask(&self.prepaint_range, &self.paint_range, old_mask, Point::default())
            && window.ranges_clear_of_mask(&self.prepaint_range, &self.paint_range, new_mask, offset);
        // Riseonly 0043: the second offset is how far the mask moved, for the callbacks that read it.
        if mask_moved_along || mask_reshaped_clear {
            Ok((offset, offset))
        } else if mask_stays_clear || self.relative_clips {
            // Riseonly 0099: under its own clip boundary the recorded clips are the view's own and
            // the scene cuts the replay by the mask it lands under, so a row on the scroll edge
            // replays too; its hitbox masks are cut again (reuse_moved_prepaint).
            Ok((offset, Point::default()))
        } else {
            Err("origin:mask")
        }
    }

    /// Riseonly 0054: the mask around the view moved (a reveal growing its clip, a sheet edge) but
    /// cut nothing the view drew or hit-tests in the frame just drawn, neither under the mask it
    /// rendered with nor under the new one. The replay then draws the same pixels and the recorded
    /// clips stay valid, so the clip alone does not render the view. The key keeps the mask the
    /// view rendered under, which its recorded primitives carry.
    fn mask_change_clear(&self, content_mask: &ContentMask<Pixels>, window: &Window) -> bool {
        if self.painted_frame + 1 != window.draw_count || self.prepainted_frame + 1 != window.draw_count {
            return false;
        }
        // Riseonly 0077: culled whole under the old mask it may have recorded nothing to replay.
        let bounds = self.cache_key.bounds;
        if culled_then_revealed(bounds, self.cache_key.content_mask.bounds, bounds, content_mask.bounds) {
            return false;
        }
        // Riseonly 0074: a view the old mask hid whole replays only while the new one hides it too;
        // a mask growing over it (a page sliding in) renders it, its recorded clips are the old mask.
        if window.ranges_wholly_hidden(&self.prepaint_range, &self.paint_range) {
            return window.ranges_hidden_by_mask(
                &self.prepaint_range,
                &self.paint_range,
                content_mask.bounds,
                Point::default(),
            );
        }
        window.ranges_clear_of_mask(
            &self.prepaint_range,
            &self.paint_range,
            self.cache_key.content_mask.bounds,
            Point::default(),
        )
        && window.ranges_clear_of_mask(
            &self.prepaint_range,
            &self.paint_range,
            content_mask.bounds,
            Point::default(),
        )
    }

    /// Whether the recorded prepaint may replay in this frame (Riseonly 0034). Under a hidden div it
    /// may unless it carries deferred draws, which would paint over what is shown. Shown, the paint
    /// that follows must replay too: from the frame just before or from a valid parked paint;
    /// otherwise the view renders now instead of drawing nothing for a frame.
    /// Riseonly 0070: why a shown cached view renders instead of replaying, the first failed check
    /// of the in-place replay, for the frame trace.
    fn miss_reason(
        &self,
        entity_id: EntityId,
        bounds: Bounds<Pixels>,
        content_mask: &ContentMask<Pixels>,
        text_style: &TextStyle,
        env_fingerprint: u64,
        replays_moved: bool,
        window: &Window,
    ) -> &'static str {
        let key = &self.cache_key;
        if window.dirty_views.contains(&entity_id) {
            "dirty"
        } else if window.refreshing {
            "refresh"
        } else if key.bounds.size != bounds.size {
            "size"
        } else if key.bounds.origin != bounds.origin {
            // Riseonly 0076: which check of the moved replay refused it.
            if replays_moved {
                self.moved_replay(entity_id, bounds, content_mask, text_style, env_fingerprint, window)
                    .err()
                    .unwrap_or("origin")
            } else {
                "origin:not_replayable"
            }
        } else if key.content_mask != *content_mask && !self.mask_change_clear(content_mask, window) {
            "mask"
        } else if key.text_style != *text_style {
            crate::profiler::detailed_reason("text_style", text_style_change(&key.text_style, text_style))
        } else if key.env_fingerprint != env_fingerprint {
            crate::profiler::detailed_reason("env", env_change(key.env_trace.as_deref(), &window.env_trace))
        } else if self.prepainted_frame + 1 != window.draw_count {
            // Riseonly 0091: whether a nested replay of the frame just before saw this view.
            if self.rebase_seen + 1 != window.draw_count {
                "prepaint_gap:unvisited"
            } else if self.rebase_refused.is_empty() {
                "prepaint_gap"
            } else {
                crate::profiler::detailed_reason("prepaint_gap", self.rebase_refused)
            }
        } else if self.dropped_deferred {
            "dropped_deferred"
        } else if self.stale_while_hidden {
            "stale_hidden"
        } else {
            "paint_gap"
        }
    }

    fn can_replay(&self, window: &Window) -> bool {
        if window.hidden_depth > 0 {
            return !window.prepaint_range_defers(&self.prepaint_range);
        }
        if self.dropped_deferred || self.stale_while_hidden {
            return false;
        }
        (self.painted_frame + 1 == window.draw_count && window.paint_range_in_frame(&self.paint_range))
            || self
                .parked
                .as_ref()
                .is_some_and(|parked| window.parked_paint_valid(parked))
    }
}

/// Riseonly 0045: a cached view that replays its prepaint copies the prepaint of every cached view
/// nested in it, but their states kept the ranges and frame of the render that recorded them, so
/// the next frame that renders the outer view rendered and laid out each of them too (0030 check),
/// the hidden feed under a pushed page included. Each nested view prepainted in the frame just
/// before now takes the range its prepaint was copied to and this frame, and keeps its parked paint
/// alive, as if it had replayed itself; its paint range is left as it was, so a shown nested view
/// whose paint was not replayed by itself still renders.
pub(crate) fn rebase_nested_views(
    old: &Range<PrepaintStateIndex>,
    replayed: &Range<PrepaintStateIndex>,
    window: &mut Window,
) {
    let frame = window.draw_count;
    window.update_replayed_states(replayed, view_element_state_type_id(), |key, state, window| {
        let Some(Some(state)) = state.downcast_mut::<Option<ViewElementState>>() else {
            return;
        };
        state.rebase_seen = frame;
        if state.prepainted_frame + 1 != frame {
            state.rebase_refused = if state.prepainted_frame == frame { "" } else { "stale" };
            return;
        }
        let Some(range) = PrepaintStateIndex::rebase_within(&state.prepaint_range, old, &replayed.start)
        else {
            state.rebase_refused = PrepaintStateIndex::outside_field(&state.prepaint_range, old);
            return;
        };
        state.rebase_refused = "";
        state.prepaint_range = range;
        state.prepainted_frame = frame;
        state.moved_by = None;
        if let Some(parked) = state.parked.as_ref() {
            window.keep_parked_states(parked);
        }
        register_nested_reader(key, state, window);
    });
}

/// Riseonly 0109: a cached view replayed inside its replayed ancestor names itself as a reader
/// of what it read, as its own replay does. Left out of `cached_views`, a notify of a store it
/// reads dirtied only the ancestor, which rendered and replayed the nested view on its stale
/// frame (X2223: the post composer hero kept its loading spinner after the preview landed,
/// until a page switch notified the page again).
fn register_nested_reader(key: &GlobalElementId, state: &ViewElementState, window: &mut Window) {
    let Some(ElementId::View(entity_id)) = key.0.last() else {
        return;
    };
    window
        .next_frame
        .cached_views
        .push((*entity_id, state.accessed_entities.clone()));
    window
        .next_frame
        .own_reads
        .insert(*entity_id, state.own_entities.clone());
}

/// Riseonly 0088: a spliced frame copied the frame just drawn around its holes, so every cached
/// view of the copied spans that was drawn in that frame now lies where its span went, as if it
/// had replayed itself; the holes rendered fresh and keep their own ranges.
pub(crate) fn rebase_spliced_views(map: &SpliceMap, window: &mut Window) {
    let frame = window.draw_count;
    let whole = window.spliced_state_keys();
    window.update_replayed_states(&whole, view_element_state_type_id(), |_, state, _| {
        let Some(Some(state)) = state.downcast_mut::<Option<ViewElementState>>() else {
            return;
        };
        if state.prepainted_frame + 1 == frame
            && let Some(range) = map.remap_prepaint(&state.prepaint_range)
        {
            state.prepaint_range = range;
            state.prepainted_frame = frame;
        }
        if state.prepainted_frame == frame
            && state.painted_frame + 1 == frame
            && state.parked.is_none()
            && let Some(range) = map.remap_paint(&state.paint_range)
        {
            state.paint_range = range;
            state.painted_frame = frame;
        }
    });
}

/// Riseonly 0063: a cached view that replays its paint in place copies the paint of every cached
/// view nested in it, but their states kept the paint range and frame of the render that recorded
/// them, so the next frame that renders the outer view (a settings page after a hover frame) found
/// each nested view unable to replay (painted_frame) and rendered it too: every wallpaper tile and
/// switch of the page on every tap (X1284). Each nested view painted in the frame just before and
/// prepainted in this one (0045 rebased it) now takes the range its paint was copied to and this
/// frame, as if it had replayed itself. A moved or transformed replay leaves them as they were.
///
/// Riseonly 0113: an in-place replay (`carry_holes`) also keeps the hole record (0088) of each
/// nested view it rebased, at its new ranges. Holes were recorded only by views that drew
/// themselves, so after any full frame in which a page replayed, a notify of a cell inside it
/// (a poster landing in an archive tile, X1937) found no hole and laid out the whole window
/// (no_hole, 3-5 ms layout for 3 views) instead of drawing that cell alone.
pub(crate) fn rebase_nested_paints(
    old: &Range<PaintIndex>,
    replayed: &Range<PaintIndex>,
    carry_holes: bool,
    window: &mut Window,
) {
    let frame = window.draw_count;
    let carry_holes =
        carry_holes && window.hidden_depth == 0 && window.transformation() == crate::TransformationMatrix::default();
    let mut carried: Vec<(EntityId, Range<PrepaintStateIndex>, Range<PaintIndex>)> = Vec::new();
    window.update_replayed_paint_states(replayed, view_element_state_type_id(), |key, state, _| {
        let Some(Some(state)) = state.downcast_mut::<Option<ViewElementState>>() else {
            return;
        };
        if state.painted_frame + 1 != frame
            || state.prepainted_frame != frame
            || state.moved_by.is_some()
            || state.parked.is_some()
        {
            return;
        }
        let Some(range) = PaintIndex::rebase_within(&state.paint_range, old, &replayed.start) else {
            return;
        };
        if carry_holes && let Some(ElementId::View(entity_id)) = key.0.last() {
            carried.push((*entity_id, state.prepaint_range.clone(), range.clone()));
        }
        state.paint_range = range;
        state.painted_frame = frame;
    });
    for (entity_id, prepaint_range, paint_range) in carried {
        if window.next_frame.holes.contains_key(&entity_id) {
            continue;
        }
        let Some(record) = window.rendered_frame.holes.get(&entity_id) else {
            continue;
        };
        let mut record = record.clone();
        record.prepaint_range = prepaint_range;
        record.paint_range = paint_range;
        window.next_frame.holes.insert(entity_id, record);
    }
}

/// Riseonly 0073: a cached view that replays at a new origin (a list cell on a wheel step, 0041)
/// copies the prepaint of every cached view nested in it, moved by the same offset, but their
/// states kept the ranges, frame and origin of the render that recorded them. The next frame that
/// renders the outer view (one notify of a preview in the cell) found each nested view a frame
/// behind (prepaint_gap) and rendered it too: preview, backdrop and every gift surface of the cell
/// on each wheel step that touched it (X1343). Each nested view prepainted and painted in the frame
/// just before now takes the range its prepaint was copied to, this frame, and its old key moved
/// by the offset, as if it had replayed moved itself; its paint follows in the paint pass (0063).
/// The new mask of a nested view is known only when the outer mask moved along with the view (the
/// nested one moved the same) or stayed and was the nested one too; otherwise it renders as before.
/// A nested view that listens to the mouse keeps the old origin in its element states, so it is
/// marked moved and renders when the mouse is over it (0044).
fn rebase_moved_nested_views(
    old: &Range<PrepaintStateIndex>,
    replayed: &Range<PrepaintStateIndex>,
    offset: Point<Pixels>,
    old_mask: &ContentMask<Pixels>,
    new_mask: &ContentMask<Pixels>,
    window: &mut Window,
) {
    let frame = window.draw_count;
    let near = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() < 0.001;
    let mask_moved_along = near(new_mask.bounds.origin.x, old_mask.bounds.origin.x + offset.x)
        && near(new_mask.bounds.origin.y, old_mask.bounds.origin.y + offset.y)
        && new_mask.bounds.size == old_mask.bounds.size;
    let mask_stayed = new_mask == old_mask;
    window.update_replayed_states(replayed, view_element_state_type_id(), |key, state, window| {
        let Some(Some(state)) = state.downcast_mut::<Option<ViewElementState>>() else {
            return;
        };
        if state.prepainted_frame + 1 != frame
            || state.painted_frame + 1 != frame
            || state.dropped_deferred
            || state.stale_while_hidden
            || state.parked.is_some()
            || state.moved_by.is_some()
        {
            return;
        }
        let moved_own_mask = ContentMask {
            bounds: Bounds {
                origin: state.cache_key.content_mask.bounds.origin + offset,
                size: state.cache_key.content_mask.bounds.size,
            },
        };
        let content_mask = if mask_moved_along {
            moved_own_mask
        } else if mask_stayed && state.cache_key.content_mask == *old_mask {
            new_mask.clone()
        } else if let Some(bounds) = mask_stayed
            .then(|| clip_moved_inside(&state.cache_key.content_mask, old_mask, new_mask, offset))
            .flatten()
        {
            ContentMask { bounds }
        } else if window.ranges_hidden_by_mask(&state.prepaint_range, &state.paint_range, new_mask.bounds, offset) {
            // Riseonly 0106: shown nothing before and nothing after (a grid cell of a pager page
            // off screen, under its own scroll clip): its mask moves along, as the 0067 replay of
            // the view itself would take it. Left behind, every cell of the hidden Stories page
            // was a frame behind (origin:gap) and rendered on the next render of its page, 30
            // cells and 26-30 ms on a Comments/Reposts switch (X1370).
            moved_own_mask
        } else {
            return;
        };
        let Some(range) = PrepaintStateIndex::rebase_within(&state.prepaint_range, old, &replayed.start)
        else {
            return;
        };
        if window.paint_range_listens(&state.paint_range) {
            state.positions_moved = true;
        }
        state.prepaint_range = range;
        state.prepainted_frame = frame;
        state.cache_key.bounds.origin = state.cache_key.bounds.origin + offset;
        state.cache_key.content_mask = content_mask;
        register_nested_reader(key, state, window);
    });
}

/// Riseonly 0082: the mask of a view nested under a clip of its own (a rounded cell with
/// overflow hidden) inside an outer mask that stayed (the scroll viewport), after the outer view
/// replayed moved by `offset`. A nested mask that touched no edge of the old outer mask was its own
/// clips alone, which moved along with the view, so the new one is that clip moved, cut by the new
/// outer mask: what a render would compute. A nested mask on an outer edge may have been cut by the
/// outer mask, so it gives None and the nested view renders as before (X1343: preview, backdrop and
/// gift surfaces of every wardrobe cell rendered on the next notify, miss prepaint_gap).
fn clip_moved_inside(
    nested: &ContentMask<Pixels>,
    old_mask: &ContentMask<Pixels>,
    new_mask: &ContentMask<Pixels>,
    offset: Point<Pixels>,
) -> Option<Bounds<Pixels>> {
    let inner = nested.bounds;
    let outer = old_mask.bounds;
    let inside = inner.origin.x > outer.origin.x
        && inner.origin.y > outer.origin.y
        && inner.right() < outer.right()
        && inner.bottom() < outer.bottom();
    if !inside {
        return None;
    }
    let moved = Bounds { origin: inner.origin + offset, size: inner.size };
    Some(moved.intersect(&new_mask.bounds))
}

struct ViewElementCacheKey {
    bounds: Bounds<Pixels>,
    content_mask: ContentMask<Pixels>,
    text_style: TextStyle,
    env_fingerprint: u64,
    /// Riseonly 0086: the environment keys the view rendered under, kept only while the frame
    /// trace runs, so a miss on env names the key that changed.
    env_trace: Option<Rc<[(&'static str, u64)]>>,
}

/// Riseonly 0086: the first text style field that differs, for the frame trace.
fn text_style_change(old: &TextStyle, new: &TextStyle) -> &'static str {
    if old.color != new.color {
        "color"
    } else if old.font_family != new.font_family {
        "font_family"
    } else if old.font_features != new.font_features {
        "font_features"
    } else if old.font_fallbacks != new.font_fallbacks {
        "font_fallbacks"
    } else if old.font_size != new.font_size {
        "font_size"
    } else if old.line_height != new.line_height {
        "line_height"
    } else if old.font_weight != new.font_weight {
        "font_weight"
    } else if old.font_style != new.font_style {
        "font_style"
    } else if old.background_color != new.background_color {
        "background_color"
    } else if old.underline != new.underline {
        "underline"
    } else if old.strikethrough != new.strikethrough {
        "strikethrough"
    } else if old.white_space != new.white_space {
        "white_space"
    } else if old.text_overflow != new.text_overflow {
        "text_overflow"
    } else if old.text_align != new.text_align {
        "text_align"
    } else if old.line_clamp != new.line_clamp {
        "line_clamp"
    } else {
        "other"
    }
}

/// Riseonly 0086: the environment key whose value differs, or that is present on one side only,
/// first from the outside; "untraced" when the view rendered before the frame trace started.
fn env_change(old: Option<&[(&'static str, u64)]>, new: &[(&'static str, u64)]) -> &'static str {
    let Some(old) = old else {
        return "untraced";
    };
    for index in 0..old.len().max(new.len()) {
        match (old.get(index), new.get(index)) {
            (Some(before), Some(now)) if before == now => continue,
            (Some(before), Some(now)) if before.0 == now.0 => return before.0,
            (_, Some(now)) => return now.0,
            (Some(before), None) => return before.0,
            (None, None) => break,
        }
    }
    "unnamed"
}

impl<V: View> Element for ViewElement<V> {
    type RequestLayoutState = Option<AnyElement>;
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        self.entity_id.map(ElementId::View)
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        #[cfg(debug_assertions)]
        return Some(self.source);

        #[cfg(not(debug_assertions))]
        return None;
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        if let Some(entity_id) = self.entity_id {
            // Stateful path: create a reactive boundary.
            window.with_rendered_view(entity_id, |window| {
                let caching_disabled = window.is_inspector_picking(cx);
                match self.cached_style.as_ref() {
                    Some(style) if !caching_disabled => {
                        let mut root_style = Style::default();
                        root_style.refine(style);
                        let layout_id = window.request_layout(root_style, None, cx);
                        (layout_id, None)
                    }
                    _ => {
                        let mut element = self
                            .view
                            .take()
                            .unwrap()
                            .render(window, cx)
                            .into_any_element();
                        let layout_id = element.request_layout(window, cx);
                        (layout_id, Some(element))
                    }
                }
            })
        } else {
            // Stateless path: isolate subtree via type name (no entity identity).
            window.with_id(
                ElementId::Name(std::any::type_name::<V>().into()),
                |window| {
                    let mut element = self
                        .view
                        .take()
                        .unwrap()
                        .render(window, cx)
                        .into_any_element();
                    let layout_id = element.request_layout(window, cx);
                    (layout_id, Some(element))
                },
            )
        }
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if let Some(entity_id) = self.entity_id {
            // Stateful path.
            window.set_view_id(entity_id);
            window.with_rendered_view(entity_id, |window| {
                if let Some(mut element) = element.take() {
                    element.prepaint(window, cx);
                    return Some(element);
                }

                window.with_element_state::<ViewElementState, _>(
                    global_id.unwrap(),
                    |mut element_state, window| {
                        let content_mask = window.content_mask();
                        let text_style = window.text_style();
                        let env_fingerprint = window.env_fingerprint;
                        let miss = (crate::profiler::frame_trace_enabled() && window.hidden_depth == 0)
                            .then(|| match element_state.as_ref() {
                                Some(state) => state.miss_reason(
                                    entity_id,
                                    bounds,
                                    &content_mask,
                                    &text_style,
                                    env_fingerprint,
                                    self.replays_moved,
                                    window,
                                ),
                                None => "new",
                            });

                        // Riseonly 0041: a scroll moves every visible cached cell; one that only
                        // moved replays its last frame at the new origin instead of rendering, and
                        // hands the move to the prepaint callbacks that read window positions.
                        if self.replays_moved
                            && let Some(state) = element_state.as_mut()
                            && let Some((offset, mask_offset)) = state.moved_offset(
                                entity_id,
                                bounds,
                                &content_mask,
                                &text_style,
                                env_fingerprint,
                                window,
                            )
                        {
                            if window.paint_range_listens(&state.paint_range) {
                                state.positions_moved = true;
                            }
                            if state.positions_moved {
                                window.next_frame.moved_views.push((entity_id, bounds));
                            }
                            let prepaint_start = window.prepaint_index();
                            let reclip = state
                                .relative_clips
                                .then(|| (state.cache_key.content_mask.bounds, content_mask.bounds));
                            let move_listeners = window.reuse_moved_prepaint(
                                state.prepaint_range.clone(),
                                offset,
                                mask_offset,
                                reclip,
                            );
                            cx.extend_nested_accessed(&state.accessed_entities);
                            window
                                .next_frame
                                .cached_views
                                .push((entity_id, state.accessed_entities.clone()));
                            window.next_frame.own_reads.insert(entity_id, state.own_entities.clone());
                            let prepaint_end = window.prepaint_index();
                            // Riseonly 0073: the cached views inside moved with it.
                            rebase_moved_nested_views(
                                &state.prepaint_range,
                                &(prepaint_start.clone()..prepaint_end.clone()),
                                offset,
                                &state.cache_key.content_mask,
                                &content_mask,
                                window,
                            );
                            state.prepaint_range = prepaint_start..prepaint_end;
                            state.prepainted_frame = window.draw_count;
                            state.cache_key.bounds = bounds;
                            state.cache_key.content_mask = content_mask;
                            state.moved_by = Some(offset);
                            for listener in &move_listeners {
                                listener.call(window, cx);
                            }
                            return (None, element_state.unwrap());
                        }

                        // Riseonly 0088: drawn shown and in place, a later frame may draw this view
                        // alone here while it copies the rest of the window.
                        self.hole_view = None;
                        self.hole_prepaint = None;
                        if window.hidden_depth == 0
                            && let Some(to_any) = self.to_any
                            && let Some(view) = self.view.as_ref()
                        {
                            self.hole_view = Some(to_any(view));
                            self.hole_prepaint = window.capture_hole_stacks(entity_id, cx);
                        }

                        // Riseonly 0037: a hidden view paints nothing, so a moved key (the centre
                        // narrowed by a panel, a new mask or environment) does not render it while
                        // hidden. It replays its last frame and keeps the old key, so the frame that
                        // shows it again renders it fresh when the key still differs.
                        if let Some(mut element_state) = element_state
                            && (window.hidden_depth > 0
                                || (element_state.cache_key.bounds == bounds
                                    && (element_state.cache_key.content_mask == content_mask
                                        || element_state.mask_change_clear(&content_mask, window))
                                    && element_state.cache_key.text_style == text_style
                                    && element_state.cache_key.env_fingerprint == env_fingerprint))
                            // Riseonly 0039: a dirty view under a hidden div paints nothing, so it
                            // waits for the frame that shows it instead of rendering and laying
                            // out every frame a store it read notifies (the feed under Settings).
                            && (window.hidden_depth > 0 || !window.dirty_views.contains(&entity_id))
                            // Riseonly 0042: a window refresh does not render a hidden view either;
                            // it waits, stale, for the frame that shows it (the feed under Settings).
                            && (window.hidden_depth > 0 || !window.refreshing)
                            && element_state.prepainted_frame + 1 == window.draw_count
                            && element_state.can_replay(window)
                        {
                            if window.hidden_depth > 0 {
                                if window.dirty_views.contains(&entity_id) || window.refreshing {
                                    element_state.stale_while_hidden = true;
                                }
                                // Riseonly 0034: the paint of the frame just drawn is skipped from
                                // now on; keep it for the frame that shows the view again.
                                if element_state.parked.is_none()
                                    && element_state.painted_frame + 1 == window.draw_count
                                    && window.paint_range_in_frame(&element_state.paint_range)
                                {
                                    element_state.parked =
                                        Some(window.park_paint(element_state.paint_range.clone()));
                                }
                                if let Some(parked) = element_state.parked.as_ref() {
                                    window.keep_parked_states(parked);
                                }
                            }
                            // Riseonly 0044: still holding an earlier origin, so the mouse entering
                            // it must make it render.
                            if element_state.positions_moved && window.hidden_depth == 0 {
                                window.next_frame.moved_views.push((entity_id, bounds));
                            }
                            let prepaint_start = window.prepaint_index();
                            window.reuse_prepaint(element_state.prepaint_range.clone());
                            cx.extend_nested_accessed(&element_state.accessed_entities);
                            window.next_frame.cached_views.push((
                                entity_id,
                                element_state.accessed_entities.clone(),
                            ));
                            window
                                .next_frame
                                .own_reads
                                .insert(entity_id, element_state.own_entities.clone());
                            let prepaint_end = window.prepaint_index();
                            // Riseonly 0045: the cached views inside replayed with it; a stale
                            // replay leaves them on their old frame so they render when shown.
                            if !element_state.stale_while_hidden {
                                rebase_nested_views(
                                    &element_state.prepaint_range,
                                    &(prepaint_start.clone()..prepaint_end.clone()),
                                    window,
                                );
                            }
                            element_state.prepaint_range = prepaint_start..prepaint_end;
                            element_state.prepainted_frame = window.draw_count;
                            element_state.moved_by = None;

                            return (None, element_state);
                        }

                        // Riseonly 0035: a view rendering fresh no longer forces its cached
                        // descendants to render too. Each one still renders when it is dirty, its
                        // cache key moved, the window refreshes, or its ranges are not of the frame
                        // just before (0030), so an animated leaf re-renders its ancestors only.
                        let dropped_before = window.hidden_dropped_deferred;
                        let moved_views_before = window.next_frame.moved_views.len();
                        let prepaint_start = window.prepaint_index();
                        if let Some(reason) = miss {
                            crate::profiler::note_cache_miss(reason);
                        }
                        let (mut element, own_entities, accessed_entities) = cx.detect_own_accessed_entities(|cx| {
                            let mut element = self
                                .view
                                .take()
                                .unwrap()
                                .render(window, cx)
                                .into_any_element();
                            element.layout_as_root(bounds.size.into(), window, cx);
                            element.prepaint_at(bounds.origin, window, cx);
                            element
                        });

                        let prepaint_end = window.prepaint_index();
                        // Riseonly 0044: a cached view inside replayed moved and listed itself;
                        // a later replay of this view skips it, so that replay lists this view.
                        let positions_moved = window.next_frame.moved_views.len() != moved_views_before;
                        let accessed_entities = Rc::new(accessed_entities);
                        let own_entities = Rc::new(own_entities);
                        window
                            .next_frame
                            .cached_views
                            .push((entity_id, accessed_entities.clone()));
                        window.next_frame.own_reads.insert(entity_id, own_entities.clone());

                        (
                            Some(element),
                            ViewElementState {
                                accessed_entities,
                                own_entities,
                                prepaint_range: prepaint_start..prepaint_end,
                                paint_range: PaintIndex::default()..PaintIndex::default(),
                                prepainted_frame: window.draw_count,
                                painted_frame: 0,
                                cache_key: ViewElementCacheKey {
                                    bounds,
                                    content_mask,
                                    text_style,
                                    env_fingerprint,
                                    env_trace: crate::profiler::frame_trace_enabled()
                                        .then(|| Rc::from(window.env_trace.as_slice())),
                                },
                                parked: None,
                                dropped_deferred: window.hidden_dropped_deferred != dropped_before,
                                stale_while_hidden: false,
                                moved_by: None,
                                painted_under: PaintContext::default(),
                                positions_moved,
                                rebase_seen: 0,
                                rebase_refused: "",
                                relative_clips: false,
                            },
                        )
                    },
                )
            })
        } else {
            // Stateless path: just prepaint the element.
            window.with_id(
                ElementId::Name(std::any::type_name::<V>().into()),
                |window| {
                    element.as_mut().unwrap().prepaint(window, cx);
                },
            );
            Some(element.take().unwrap())
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        element: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Riseonly 0099: a spliced hole paints inside the clip boundary its stacks already hold.
        let reopened = mem::take(&mut window.hole_clip_boundary);
        if let Some(entity_id) = self.entity_id {
            // Stateful path.
            window.with_rendered_view(entity_id, |window| {
                let caching_disabled = window.is_inspector_picking(cx);
                if self.cached_style.is_some() && !caching_disabled {
                    window.with_element_state::<ViewElementState, _>(
                        global_id.unwrap(),
                        |element_state, window| {
                            let mut element_state = element_state.unwrap();

                            // Riseonly 0099: a cell that replays moved paints inside its own clip
                            // boundary, outside its paint range, so each replay opens it anew.
                            let boundary = reopened || (self.replays_moved && window.begin_clip_boundary());
                            let fresh = element.is_some();
                            // Riseonly 0101: primitives painted inside a boundary keep masks
                            // relative to it. Replayed where no boundary opens (under a
                            // transformation), the scene cuts them by the mask around the view, so a
                            // row does not spill out of the scroll clip.
                            let replay_clip = !boundary
                                && !fresh
                                && element_state.relative_clips
                                && window.begin_replay_clip();
                            let paint_start = window.paint_index();
                            // Riseonly 0088: the state this paint runs under, for a hole record.
                            let mut hole_paint = if self.hole_prepaint.is_some() {
                                window.capture_hole_stacks(entity_id, cx)
                            } else {
                                None
                            };
                            if let Some(stacks) = hole_paint.as_mut() {
                                stacks.own_clip_boundary = boundary;
                            }
                            let mut in_place = false;

                            if let Some(element) = element {
                                in_place = true;
                                let refreshing = mem::replace(&mut window.refreshing, true);
                                element.paint(window, cx);
                                window.refreshing = refreshing;
                                element_state.painted_frame = window.draw_count;
                                element_state.painted_under = window.paint_context();
                            } else if element_state.painted_frame + 1 == window.draw_count
                                && window.paint_range_in_frame(&element_state.paint_range)
                            {
                                let range = element_state.paint_range.clone();
                                let now = window.paint_context();
                                match element_state.moved_by.take() {
                                    // Riseonly 0041: the prepaint replayed at a new origin.
                                    // Riseonly 0085: moved inside a fade (rows sliding while the
                                    // list fades a refresh in): the replay takes the offset and the
                                    // fade at once. Nested cached views are not rebased, as after
                                    // the in-place faded replay, so they render once.
                                    Some(offset)
                                        if !Window::same_opacity(&element_state.painted_under, &now)
                                            && window.reuse_moved_faded_paint(
                                                range.clone(),
                                                offset,
                                                &element_state.painted_under,
                                                &now,
                                            ) =>
                                    {
                                        element_state.painted_under = now;
                                    }
                                    Some(offset) => {
                                        window.reuse_moved_paint(range.clone(), offset);
                                        // Riseonly 0073: the cached views inside moved with it.
                                        let replayed = paint_start.clone()..window.paint_index();
                                        rebase_nested_paints(&range, &replayed, false, window);
                                        // Riseonly 0065: a moved replay keeps the opacity it was
                                        // painted with, so a view moving inside a fade shows the
                                        // old opacity once and renders in the next frame.
                                        if !Window::same_opacity(&element_state.painted_under, &now) {
                                            cx.notify(entity_id);
                                        }
                                    }
                                    None if element_state.painted_under.places_alike(&now)
                                        && Window::same_opacity(&element_state.painted_under, &now) =>
                                    {
                                        in_place = true;
                                        window.reuse_paint(range.clone());
                                        // Riseonly 0063: the cached views inside replayed with it.
                                        let replayed = paint_start.clone()..window.paint_index();
                                        rebase_nested_paints(&range, &replayed, true, window);
                                    }
                                    // Riseonly 0065: same place, another opacity. A view first
                                    // painted inside a fade-in kept that faint frame forever
                                    // (X1624: the switches of rows revealed by an opacity
                                    // transition stayed invisible until some click dirtied them).
                                    // The replay takes the fade; nested cached views are not
                                    // rebased, so they render once, as after a moved replay.
                                    None if element_state.painted_under.places_alike(&now)
                                        && window.reuse_faded_paint(
                                            range.clone(),
                                            &element_state.painted_under,
                                            &now,
                                        ) =>
                                    {
                                        element_state.painted_under = now;
                                    }
                                    None if element_state.painted_under.places_alike(&now) => {
                                        window.reuse_paint(range);
                                        cx.notify(entity_id);
                                    }
                                    // Riseonly 0053: the transformation around the view moved (a
                                    // zoom flight): its frame follows the matrix and the clip.
                                    None if window.reuse_transformed_paint(
                                        range.clone(),
                                        &element_state.painted_under,
                                        &now,
                                    ) =>
                                    {
                                        element_state.painted_under = now;
                                    }
                                    // Riseonly 0053: the old frame cannot follow, so it shows once
                                    // more where it was and the view renders in the next frame,
                                    // instead of staying frozen while the transformation moves.
                                    None => {
                                        window.reuse_paint(range);
                                        cx.notify(entity_id);
                                    }
                                }
                                element_state.painted_frame = window.draw_count;
                            } else if let Some(parked) = element_state.parked.take() {
                                // Riseonly 0034: shown again after hidden frames, same cache key.
                                window.replay_parked_paint(parked);
                                element_state.painted_frame = window.draw_count;
                                // Riseonly 0083: the parked paint keeps the opacity and the
                                // transformation it was painted under. One painted inside a fade
                                // (at zero opacity its primitives were dropped) or another flight
                                // came back blank and stayed so until notified (X1769: a feed row
                                // left a hole after Settings and back). It shows once and renders
                                // in the next frame, as the in-place replay does (0053, 0065).
                                let now = window.paint_context();
                                if !element_state.painted_under.places_alike(&now)
                                    || !Window::same_opacity(&element_state.painted_under, &now)
                                {
                                    cx.notify(entity_id);
                                }
                            } else {
                                // Prepaint was replayed but the last paint is older than the
                                // previous frame (the view sat under a hidden div): its range
                                // no longer indexes the rendered frame. Skip this frame and
                                // render the view fresh in the next one.
                                if element_state.painted_frame + 1 == window.draw_count {
                                    window.note_stale_paint_range("view", entity_id, &element_state.paint_range);
                                }
                                cx.notify(entity_id);
                            }

                            let paint_end = window.paint_index();
                            element_state.paint_range = paint_start..paint_end;
                            element_state.relative_clips =
                                (boundary || replay_clip) && (fresh || element_state.relative_clips);
                            if boundary && !reopened {
                                window.end_clip_boundary();
                            }
                            if replay_clip {
                                window.end_replay_clip();
                            }

                            let hole_prepaint = self.hole_prepaint.take();
                            let hole_view = self.hole_view.take();
                            // Riseonly 0104: a view painted under a transformation (a modal
                            // scaling in) keeps no hole: drawn alone later it would paint under that
                            // stale matrix and its masks, the rows spilling out of the sheet at rest.
                            if in_place
                                && window.transformation() == crate::TransformationMatrix::default()
                                && let Some(paint) = hole_paint
                                && let Some(prepaint) = hole_prepaint
                                && let Some(view) = hole_view
                                && let Some(style) = self.cached_style.clone()
                            {
                                window.next_frame.holes.insert(
                                    entity_id,
                                    HoleRecord {
                                        view,
                                        style,
                                        replays_moved: self.replays_moved,
                                        bounds: element_state.cache_key.bounds,
                                        prepaint: Rc::new(prepaint),
                                        paint: Rc::new(paint),
                                        prepaint_range: element_state.prepaint_range.clone(),
                                        paint_range: element_state.paint_range.clone(),
                                    },
                                );
                            }

                            ((), element_state)
                        },
                    )
                } else {
                    element.as_mut().unwrap().paint(window, cx);
                }
            });
        } else {
            // Stateless path: just paint the element.
            window.with_id(
                ElementId::Name(std::any::type_name::<V>().into()),
                |window| {
                    element.as_mut().unwrap().paint(window, cx);
                },
            );
        }
    }
}

/// A view that renders nothing
pub struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}
