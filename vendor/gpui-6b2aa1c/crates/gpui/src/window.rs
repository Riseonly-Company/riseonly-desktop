#[cfg(any(feature = "inspector", debug_assertions))]
use crate::Inspector;
use crate::scene::TransformedReplay;
use crate::{SpliceNodes, SpliceSpans};
use crate::{
    Action, AnyDrag, AnyElement, AnyImageCache, AnyTooltip, AnyView, App, AppContext, Arena, Asset,
    AsyncWindowContext, AtlasTile, AvailableSpace, Background, BorderStyle, Bounds, BoxShadow,
    Capslock, Context, Corners, CursorHideMode, CursorStyle, Decorations, DevicePixels,
    DispatchActionListener, DispatchNodeId, DispatchTree, DisplayId, Edges, Effect, Entity,
    EntityId, EventEmitter, FileDropEvent, FontId, Global, GlobalElementId, GlyphId, GpuSpecs,
    Hsla, InputHandler, IsZero, KeyBinding, KeyContext, KeyDownEvent, KeyEvent, Keystroke,
    KeystrokeEvent, LayoutId, LineLayoutIndex, Modifiers, ModifiersChangedEvent, MonochromeSprite,
    AtlasKey, MouseButton, MouseEvent, MouseMoveEvent, MouseUpEvent, Path, Pixels, PlatformAtlas, PlatformImageUploader,
    PlatformDisplay, PlatformInput, PlatformInputHandler, PlatformWindow, Point, PolychromeSprite,
    Priority, PromptButton, PromptLevel, Quad, Render, RenderGlyphParams, RenderImage,
    RenderImageParams, RenderSvgParams, Replay, ResizeEdge, SMOOTH_SVG_SCALE_FACTOR,
    SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y, ScaledPixels, Scene, Shadow, SharedString, Size,
    StrikethroughStyle, Style, SubpixelSprite, SubscriberSet, Subscription, SystemWindowTab,
    SystemWindowTabController, TabStopMap, TaffyLayoutEngine, Task, TextRenderingMode, TextStyle,
    TextStyleRefinement, ThermalState, TransformationMatrix, Underline, UnderlineStyle,
    WindowAppearance, WindowBackgroundAppearance, WindowBounds, WindowControls, WindowDecorations,
    WindowOptions, WindowParams, WindowTextSystem, point, prelude::*, profiler, px, rems, size,
    transparent_black,
};

use anyhow::{Context as _, Result, anyhow};
use collections::{FxHashMap, FxHashSet};
#[cfg(target_os = "macos")]
use core_video::pixel_buffer::CVPixelBuffer;
use derive_more::{Deref, DerefMut};
use futures::FutureExt;
use futures::channel::oneshot;
use gpui_util::post_inc;
use gpui_util::{ResultExt, measure};
#[cfg(feature = "input-latency-histogram")]
use hdrhistogram::Histogram;
use itertools::FoldWhile::{Continue, Done};
use itertools::Itertools;
use parking_lot::RwLock;
use raw_window_handle::{HandleError, HasDisplayHandle, HasWindowHandle};
use refineable::Refineable;
use scheduler::Instant;
use slotmap::SlotMap;
use smallvec::SmallVec;
use std::{
    any::{Any, TypeId},
    borrow::Cow,
    cell::{Cell, RefCell},
    cmp,
    fmt::{Debug, Display},
    hash::{Hash, Hasher},
    marker::PhantomData,
    mem,
    ops::{DerefMut, Range},
    rc::Rc,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
    },
    time::Duration,
};
use uuid::Uuid;

pub(crate) mod a11y;
mod prompts;

pub use a11y::A11ySubtreeBuilder;

use self::a11y::A11y;
#[cfg(not(target_family = "wasm"))]
use self::a11y::ROOT_NODE_ID;
use crate::util::{
    atomic_incr_if_not_zero, ceil_to_device_pixel, floor_to_device_pixel, round_half_toward_zero,
    round_half_toward_zero_f64, round_stroke_to_device_pixel, round_to_device_pixel,
};
pub use prompts::*;

/// Default window size used when no explicit size is provided.
pub const DEFAULT_WINDOW_SIZE: Size<Pixels> = size(px(1536.), px(1095.));

/// A 6:5 aspect ratio minimum window size to be used for functional,
/// additional-to-main-Zed windows, like the settings and rules library windows.
pub const DEFAULT_ADDITIONAL_WINDOW_SIZE: Size<Pixels> = Size {
    width: Pixels(900.),
    height: Pixels(750.),
};

/// Represents the two different phases when dispatching events.
#[derive(Default, Copy, Clone, Debug, Eq, PartialEq)]
pub enum DispatchPhase {
    /// After the capture phase comes the bubble phase, in which mouse event listeners are
    /// invoked front to back and keyboard event listeners are invoked from the focused element
    /// to the root of the element tree. This is the phase you'll most commonly want to use when
    /// registering event listeners.
    #[default]
    Bubble,
    /// During the initial capture phase, mouse event listeners are invoked back to front, and keyboard
    /// listeners are invoked from the root of the tree downward toward the focused element. This phase
    /// is used for special purposes such as clearing the "pressed" state for click events. If
    /// you stop event propagation during this phase, you need to know what you're doing. Handlers
    /// outside of the immediate region may rely on detecting non-local events during this phase.
    Capture,
}

impl DispatchPhase {
    /// Returns true if this represents the "bubble" phase.
    #[inline]
    pub fn bubble(self) -> bool {
        self == DispatchPhase::Bubble
    }

    /// Returns true if this represents the "capture" phase.
    #[inline]
    pub fn capture(self) -> bool {
        self == DispatchPhase::Capture
    }
}

struct WindowInvalidatorInner {
    pub dirty: bool,
    pub draw_phase: DrawPhase,
    pub dirty_views: FxHashSet<EntityId>,
    pub update_count: usize,
    pub frame_dirty: FrameDirtyAccumulator,
    /// Riseonly 0105: a view notified while the frame paints (a cached view replayed under another
    /// opacity or transformation, or a range of an older frame) asks for the next frame.
    pub notified_in_paint: bool,
}

/// Per-frame invalidation bookkeeping, drained at draw time and emitted to the
/// frame profiler. Tracks when the current frame first became dirty and how
/// many invalidations were coalesced into it. Only populated while
/// `profiler::frame_trace_enabled()` is set.
#[derive(Default)]
struct FrameDirtyAccumulator {
    dirty_at: Option<Instant>,
    invalidations: u64,
}

#[derive(Clone)]
pub(crate) struct WindowInvalidator {
    inner: Rc<RefCell<WindowInvalidatorInner>>,
}

impl WindowInvalidator {
    pub fn new() -> Self {
        WindowInvalidator {
            inner: Rc::new(RefCell::new(WindowInvalidatorInner {
                dirty: true,
                draw_phase: DrawPhase::None,
                dirty_views: FxHashSet::default(),
                update_count: 0,
                frame_dirty: FrameDirtyAccumulator::default(),
                notified_in_paint: false,
            })),
        }
    }

    pub fn invalidate_view(&self, entity: EntityId, cx: &mut App) -> bool {
        let mut inner = self.inner.borrow_mut();
        inner.update_count += 1;
        inner.dirty_views.insert(entity);
        if inner.draw_phase == DrawPhase::None {
            Self::record_frame_dirty(&mut inner);
            inner.dirty = true;
            cx.push_effect(Effect::Notify { emitter: entity });
            true
        } else {
            // why: Riseonly 0105, a replay that shows a stale frame once notifies its view during
            // paint to render it in the next frame (0053, 0065, 0083, 0096); without a scheduled
            // frame a quiet window kept the stale one (X1603, X2006: a cached root host frozen on
            // its first open frame). A notify while rendering stays unscheduled, as upstream.
            if inner.draw_phase == DrawPhase::Paint {
                inner.notified_in_paint = true;
            }
            false
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.inner.borrow().dirty
    }

    pub fn set_dirty(&self, dirty: bool) {
        let mut inner = self.inner.borrow_mut();
        inner.dirty = dirty;
        if dirty {
            inner.update_count += 1;
            Self::record_frame_dirty(&mut inner);
        }
    }

    pub fn set_phase(&self, phase: DrawPhase) {
        let mut inner = self.inner.borrow_mut();
        inner.draw_phase = phase;
        // invariant: Riseonly 0105, the frame just drawn is done, so a view notified during its
        // paint (still in dirty_views) draws in the next frame.
        if phase == DrawPhase::None && mem::take(&mut inner.notified_in_paint) {
            inner.dirty = true;
            inner.update_count += 1;
            Self::record_frame_dirty(&mut inner);
        }
    }

    pub fn update_count(&self) -> usize {
        self.inner.borrow().update_count
    }

    fn record_frame_dirty(inner: &mut WindowInvalidatorInner) {
        if profiler::frame_trace_enabled() {
            inner.frame_dirty.dirty_at.get_or_insert_with(Instant::now);
            inner.frame_dirty.invalidations += 1;
        }
    }

    fn take_frame_dirty(&self) -> FrameDirtyAccumulator {
        mem::take(&mut self.inner.borrow_mut().frame_dirty)
    }

    /// Dirties `views` and schedules a frame without the notify effect of `invalidate_view`: a
    /// focus move is no change of a view's data, so its observers must not hear one (Riseonly 0040).
    pub fn mark_views_dirty(&self, views: impl IntoIterator<Item = EntityId>) {
        let mut inner = self.inner.borrow_mut();
        inner.update_count += 1;
        inner.dirty_views.extend(views);
        Self::record_frame_dirty(&mut inner);
        inner.dirty = true;
    }

    pub fn take_views(&self) -> FxHashSet<EntityId> {
        mem::take(&mut self.inner.borrow_mut().dirty_views)
    }

    pub fn replace_views(&self, views: FxHashSet<EntityId>) {
        self.inner.borrow_mut().dirty_views = views;
    }

    pub fn not_drawing(&self) -> bool {
        self.inner.borrow().draw_phase == DrawPhase::None
    }

    #[track_caller]
    pub fn debug_assert_paint(&self) {
        debug_assert!(
            matches!(self.inner.borrow().draw_phase, DrawPhase::Paint),
            "this method can only be called during paint"
        );
    }

    #[track_caller]
    pub fn debug_assert_prepaint(&self) {
        debug_assert!(
            matches!(self.inner.borrow().draw_phase, DrawPhase::Prepaint),
            "this method can only be called during request_layout, or prepaint"
        );
    }

    #[track_caller]
    pub fn debug_assert_paint_or_prepaint(&self) {
        debug_assert!(
            matches!(
                self.inner.borrow().draw_phase,
                DrawPhase::Paint | DrawPhase::Prepaint
            ),
            "this method can only be called during request_layout, prepaint, or paint"
        );
    }
}

type AnyObserver = Box<dyn FnMut(&mut Window, &mut App) -> bool + 'static>;

pub(crate) type AnyWindowFocusListener =
    Box<dyn FnMut(&WindowFocusEvent, &mut Window, &mut App) -> bool + 'static>;

pub(crate) struct WindowFocusEvent {
    pub(crate) previous_focus_path: SmallVec<[FocusId; 8]>,
    pub(crate) current_focus_path: SmallVec<[FocusId; 8]>,
}

impl WindowFocusEvent {
    pub fn is_focus_in(&self, focus_id: FocusId) -> bool {
        !self.previous_focus_path.contains(&focus_id) && self.current_focus_path.contains(&focus_id)
    }

    pub fn is_focus_out(&self, focus_id: FocusId) -> bool {
        self.previous_focus_path.contains(&focus_id) && !self.current_focus_path.contains(&focus_id)
    }
}

/// This is provided when subscribing for `Context::on_focus_out` events.
pub struct FocusOutEvent {
    /// A weak focus handle representing what was blurred.
    pub blurred: WeakFocusHandle,
}

slotmap::new_key_type! {
    /// A globally unique identifier for a focusable element.
    pub struct FocusId;
}

thread_local! {
    /// Fallback arena used when no app-specific arena is active.
    /// In production, each window draw sets CURRENT_ELEMENT_ARENA to the app's arena.
    pub(crate) static ELEMENT_ARENA: RefCell<Arena> = RefCell::new(Arena::new(1024 * 1024));

    /// Points to the current App's element arena during draw operations.
    /// This allows multiple test Apps to have isolated arenas, preventing
    /// cross-session corruption when the scheduler interleaves their tasks.
    static CURRENT_ELEMENT_ARENA: Cell<Option<*const RefCell<Arena>>> = const { Cell::new(None) };

    /// The view whose render, prepaint or paint runs now on this thread (Riseonly 0062).
    static RENDERING_VIEW: Cell<Option<EntityId>> = const { Cell::new(None) };
}

/// The view whose render, prepaint or paint runs now, None outside a draw (Riseonly 0062). A store
/// that has no `Context` at hand records it on a read and notifies only the views that read it,
/// instead of refreshing every window (which renders every cached view again).
pub fn rendering_view() -> Option<EntityId> {
    RENDERING_VIEW.with(Cell::get)
}

/// Whether a window draw is currently in progress on this thread.
///
/// This holds exactly while an `ElementArenaScope` is active: nested scopes
/// restore the previous (still set) arena pointer, so `CURRENT_ELEMENT_ARENA`
/// is `Some` from the outermost draw's start to its end.
///
/// The `on_request_frame` callback uses this to defer draw requests that
/// arrive re-entrantly while a draw is already on the stack (e.g. via nested
/// message pumping in the Windows window procedure), instead of running a
/// nested draw or panicking on the already-borrowed App.
fn draw_in_progress() -> bool {
    CURRENT_ELEMENT_ARENA.with(|current| current.get().is_some())
}

/// Allocates an element in the current arena. Uses the app-specific arena if one
/// is active (during draw), otherwise falls back to the thread-local ELEMENT_ARENA.
pub(crate) fn with_element_arena<R>(f: impl FnOnce(&mut Arena) -> R) -> R {
    CURRENT_ELEMENT_ARENA.with(|current| {
        if let Some(arena_ptr) = current.get() {
            // SAFETY: The pointer is valid for the duration of the draw operation
            // that set it, and we're being called during that same draw.
            let arena_cell = unsafe { &*arena_ptr };
            f(&mut arena_cell.borrow_mut())
        } else {
            ELEMENT_ARENA.with_borrow_mut(f)
        }
    })
}

/// Scope guard that sets CURRENT_ELEMENT_ARENA for the duration of a draw
/// operation and tracks the arena's scope depth, so that a nested draw's
/// `ArenaClearNeeded::clear` is deferred rather than freeing memory the outer
/// draw still references (see `Arena::clear`).
///
/// Call [`ElementArenaScope::exit`] with the same arena that was entered to
/// obtain the [`ArenaClearNeeded`] token the draw now owes; requiring `exit`
/// makes it impossible to request a clear before the scope has ended. The
/// scope's teardown — restoring the thread-local and balancing `begin_scope`
/// with `end_scope` — happens in `Drop`, so the arena's scope depth stays
/// balanced on every path, including when a panic unwinds a draw before `exit`
/// is reached. (If teardown lived only in `exit`, such a panic would leave the
/// scope depth permanently elevated and defer every future clear, leaking
/// memory unboundedly.)
pub(crate) struct ElementArenaScope {
    /// The entered arena: compared against the argument in `exit`, and
    /// dereferenced in `Drop` to end its scope (see the SAFETY note there).
    entered: *const RefCell<Arena>,
    previous: Option<*const RefCell<Arena>>,
    exited: bool,
}

impl ElementArenaScope {
    /// Enter a scope where element allocations use the given arena.
    pub(crate) fn enter(arena: &RefCell<Arena>) -> Self {
        arena.borrow_mut().begin_scope();
        let previous = CURRENT_ELEMENT_ARENA.with(|current| {
            let prev = current.get();
            current.set(Some(arena as *const RefCell<Arena>));
            prev
        });
        Self {
            entered: arena as *const RefCell<Arena>,
            previous,
            exited: false,
        }
    }

    /// End the scope: restores the previously-current arena and ends the
    /// arena's clear-deferral scope. Returns the token for the arena clear the
    /// draw now owes; producing it here makes it impossible to request a clear
    /// before the scope has ended (which would be silently deferred forever).
    ///
    /// Panics if passed a different arena than was entered: ending the scope
    /// of the wrong arena would unbalance two arenas' scope depths, allowing
    /// one of them to clear while a draw still references its memory.
    pub(crate) fn exit(mut self, arena: &RefCell<Arena>) -> ArenaClearNeeded {
        assert!(
            std::ptr::eq(self.entered, arena),
            "ElementArenaScope::exit called with a different arena than was entered"
        );
        self.exited = true;
        // Teardown (restoring the thread-local and ending the arena's
        // clear-deferral scope) runs in `Drop`, which fires both here — `self`
        // is dropped as `exit` returns, before the token reaches the caller —
        // and when a panic unwinds the draw before `exit` is reached.
        ArenaClearNeeded::new(arena)
    }
}

impl Drop for ElementArenaScope {
    fn drop(&mut self) {
        // Teardown lives here (rather than in `exit`) so it runs exactly once on
        // every path: `exit` consumes and drops the guard on the normal path,
        // and unwinding drops it on the panic path. Balancing `begin_scope` here
        // keeps the arena's scope depth correct even when a draw panics; if this
        // only happened in `exit`, a panic between `enter` and `exit` would leave
        // the depth elevated and defer every future clear.
        CURRENT_ELEMENT_ARENA.with(|current| {
            current.set(self.previous);
        });
        // SAFETY: `entered` came from a `&RefCell<Arena>` in `enter`, and the
        // arena (owned by the `App` being drawn) outlives this guard on both the
        // normal and unwinding paths, since the guard is a local of the draw.
        unsafe { &*self.entered }.borrow_mut().end_scope();
        if !self.exited && !std::thread::panicking() {
            debug_assert!(false, "ElementArenaScope dropped without calling exit()");
            log::error!(
                "ElementArenaScope dropped without calling exit(); \
                 the arena clear for this draw was never requested"
            );
        }
    }
}

/// Returned when the element arena has been used and so must be cleared before the next draw.
#[must_use]
pub struct ArenaClearNeeded {
    /// Identity of the arena that was drawn into. Only ever compared against
    /// another pointer in `clear`; never dereferenced.
    arena: *const RefCell<Arena>,
}

impl ArenaClearNeeded {
    /// Create a new ArenaClearNeeded token for the App whose arena was drawn
    /// into. Private: the only way to obtain one is [`ElementArenaScope::exit`].
    fn new(arena: &RefCell<Arena>) -> Self {
        Self {
            arena: arena as *const RefCell<Arena>,
        }
    }

    /// Clear the element arena of the App the draw ran against. If an enclosing
    /// draw is still in progress (this draw was nested inside it), the clear is
    /// deferred to the enclosing draw's own `ArenaClearNeeded` so that its live
    /// allocations aren't freed.
    ///
    /// Panics if passed a different App than the draw ran against, since
    /// clearing another App's arena could free memory its draws still
    /// reference.
    pub fn clear(self, cx: &mut App) {
        assert!(
            std::ptr::eq(self.arena, &cx.element_arena),
            "ArenaClearNeeded::clear called with a different App than the draw ran against"
        );
        cx.element_arena.borrow_mut().clear();
    }
}

pub(crate) type FocusMap = RwLock<SlotMap<FocusId, FocusRef>>;
pub(crate) struct FocusRef {
    pub(crate) ref_count: AtomicUsize,
    pub(crate) tab_index: isize,
    pub(crate) tab_stop: bool,
}

impl FocusId {
    /// Obtains whether the element associated with this handle is currently focused.
    pub fn is_focused(&self, window: &Window) -> bool {
        window.focus == Some(*self)
    }

    /// Obtains whether the element associated with this handle contains the focused
    /// element or is itself focused.
    pub fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        window
            .focused(cx)
            .is_some_and(|focused| self.contains(focused.id, window))
    }

    /// Obtains whether the element associated with this handle is contained within the
    /// focused element or is itself focused.
    pub fn within_focused(&self, window: &Window, cx: &App) -> bool {
        let focused = window.focused(cx);
        focused.is_some_and(|focused| focused.id.contains(*self, window))
    }

    /// Obtains whether this handle contains the given handle in the most recently rendered frame.
    pub(crate) fn contains(&self, other: Self, window: &Window) -> bool {
        window
            .rendered_frame
            .dispatch_tree
            .focus_contains(*self, other)
    }
}

/// A handle which can be used to track and manipulate the focused element in a window.
pub struct FocusHandle {
    pub(crate) id: FocusId,
    handles: Arc<FocusMap>,
    /// The index of this element in the tab order.
    pub tab_index: isize,
    /// Whether this element can be focused by tab navigation.
    pub tab_stop: bool,
}

impl std::fmt::Debug for FocusHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("FocusHandle({:?})", self.id))
    }
}

impl FocusHandle {
    pub(crate) fn new(handles: &Arc<FocusMap>) -> Self {
        let id = handles.write().insert(FocusRef {
            ref_count: AtomicUsize::new(1),
            tab_index: 0,
            tab_stop: false,
        });

        Self {
            id,
            tab_index: 0,
            tab_stop: false,
            handles: handles.clone(),
        }
    }

    pub(crate) fn for_id(id: FocusId, handles: &Arc<FocusMap>) -> Option<Self> {
        let lock = handles.read();
        let focus = lock.get(id)?;
        if atomic_incr_if_not_zero(&focus.ref_count) == 0 {
            return None;
        }
        Some(Self {
            id,
            tab_index: focus.tab_index,
            tab_stop: focus.tab_stop,
            handles: handles.clone(),
        })
    }

    /// Sets the tab index of the element associated with this handle.
    pub fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = index;
        if let Some(focus) = self.handles.write().get_mut(self.id) {
            focus.tab_index = index;
        }
        self
    }

    /// Sets whether the element associated with this handle is a tab stop.
    ///
    /// When `false`, the element will not be included in the tab order.
    pub fn tab_stop(mut self, tab_stop: bool) -> Self {
        self.tab_stop = tab_stop;
        if let Some(focus) = self.handles.write().get_mut(self.id) {
            focus.tab_stop = tab_stop;
        }
        self
    }

    /// Converts this focus handle into a weak variant, which does not prevent it from being released.
    pub fn downgrade(&self) -> WeakFocusHandle {
        WeakFocusHandle {
            id: self.id,
            handles: Arc::downgrade(&self.handles),
        }
    }

    /// Moves the focus to the element associated with this handle.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(self, cx)
    }

    /// Obtains whether the element associated with this handle is currently focused.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.id.is_focused(window)
    }

    /// Obtains whether the element associated with this handle contains the focused
    /// element or is itself focused.
    pub fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        self.id.contains_focused(window, cx)
    }

    /// Obtains whether the element associated with this handle is contained within the
    /// focused element or is itself focused.
    pub fn within_focused(&self, window: &Window, cx: &mut App) -> bool {
        self.id.within_focused(window, cx)
    }

    /// Obtains whether this handle contains the given handle in the most recently rendered frame.
    pub fn contains(&self, other: &Self, window: &Window) -> bool {
        self.id.contains(other.id, window)
    }

    /// Obtains whether an element of the most recently rendered frame tracks this handle. A live
    /// handle without one receives no key events: they dispatch from the window root instead.
    pub fn is_in_rendered_frame(&self, window: &Window) -> bool {
        window
            .rendered_frame
            .dispatch_tree
            .focusable_node_id(self.id)
            .is_some()
    }

    /// Dispatch an action on the element that rendered this focus handle
    pub fn dispatch_action(&self, action: &dyn Action, window: &mut Window, cx: &mut App) {
        if let Some(node_id) = window
            .rendered_frame
            .dispatch_tree
            .focusable_node_id(self.id)
        {
            window.dispatch_action_on_node(node_id, action, cx)
        }
    }
}

impl Clone for FocusHandle {
    fn clone(&self) -> Self {
        Self::for_id(self.id, &self.handles).unwrap()
    }
}

impl PartialEq for FocusHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for FocusHandle {}

impl Drop for FocusHandle {
    fn drop(&mut self) {
        self.handles
            .read()
            .get(self.id)
            .unwrap()
            .ref_count
            .fetch_sub(1, SeqCst);
    }
}

/// A weak reference to a focus handle.
#[derive(Clone, Debug)]
pub struct WeakFocusHandle {
    pub(crate) id: FocusId,
    pub(crate) handles: Weak<FocusMap>,
}

impl WeakFocusHandle {
    /// Attempts to upgrade the [WeakFocusHandle] to a [FocusHandle].
    pub fn upgrade(&self) -> Option<FocusHandle> {
        let handles = self.handles.upgrade()?;
        FocusHandle::for_id(self.id, &handles)
    }
}

impl PartialEq for WeakFocusHandle {
    fn eq(&self, other: &WeakFocusHandle) -> bool {
        self.id == other.id
    }
}

impl Eq for WeakFocusHandle {}

impl PartialEq<FocusHandle> for WeakFocusHandle {
    fn eq(&self, other: &FocusHandle) -> bool {
        self.id == other.id
    }
}

impl PartialEq<WeakFocusHandle> for FocusHandle {
    fn eq(&self, other: &WeakFocusHandle) -> bool {
        self.id == other.id
    }
}

/// Focusable allows users of your view to easily
/// focus it (using window.focus_view(cx, view))
pub trait Focusable: 'static {
    /// Returns the focus handle associated with this view.
    fn focus_handle(&self, cx: &App) -> FocusHandle;
}

impl<V: Focusable> Focusable for Entity<V> {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }
}

/// ManagedView is a view (like a Modal, Popover, Menu, etc.)
/// where the lifecycle of the view is handled by another view.
pub trait ManagedView: Focusable + EventEmitter<DismissEvent> + Render {}

impl<M: Focusable + EventEmitter<DismissEvent> + Render> ManagedView for M {}

/// Emitted by implementers of [`ManagedView`] to indicate the view should be dismissed, such as when a view is presented as a modal.
pub struct DismissEvent;

type FrameCallback = Box<dyn FnOnce(&mut Window, &mut App)>;

pub(crate) type AnyMouseListener =
    Box<dyn FnMut(&dyn Any, DispatchPhase, &mut Window, &mut App) + 'static>;

#[derive(Clone)]
pub(crate) struct CursorStyleRequest {
    pub(crate) hitbox_id: Option<HitboxId>,
    pub(crate) style: CursorStyle,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub(crate) struct HitTest {
    pub(crate) ids: SmallVec<[HitboxId; 8]>,
    pub(crate) hover_hitbox_count: usize,
}

/// A type of window control area that corresponds to the platform window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowControlArea {
    /// An area that allows dragging of the platform window.
    Drag,
    /// An area that allows closing of the platform window.
    Close,
    /// An area that allows maximizing of the platform window.
    Max,
    /// An area that allows minimizing of the platform window.
    Min,
}

/// An identifier for a [Hitbox] which also includes [HitboxBehavior].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct HitboxId(u64);

#[cfg(feature = "test-support")]
impl HitboxId {
    /// A placeholder HitboxId exclusively for integration testing API's that
    /// need a hitbox but where the value of the hitbox does not matter. The
    /// alternative is to make the Hitbox optional but that complicates the
    /// implementation.
    pub const fn placeholder() -> Self {
        Self(0)
    }
}

impl HitboxId {
    /// Checks if the hitbox with this ID is currently hovered. Returns `false` during keyboard
    /// input modality so that keyboard navigation suppresses hover highlights. Except when handling
    /// `ScrollWheelEvent`, this is typically what you want when determining whether to handle mouse
    /// events or paint hover styles.
    ///
    /// See [`Hitbox::is_hovered`] for details.
    pub fn is_hovered(self, window: &Window) -> bool {
        // If this hitbox has captured the pointer, it's always considered hovered
        if window.captured_hitbox == Some(self) {
            return true;
        }
        // Riseonly 0069: a view that draws a hitbox under the mouse depends on the input
        // modality (keyboard hides hover), so it is notified when the modality changes.
        let hit = self.hit_test(window);
        if hit && let Some(view) = window.rendered_entity_stack.last() {
            // Riseonly 0111: kept with the hitbox, so a switch notifies only the views whose
            // hitbox is under the mouse then (X2331).
            window.hover_modality_readers.borrow_mut().insert((*view, self));
        }
        hit && window.last_input_modality != InputModality::Keyboard
    }

    /// Checks if the hitbox with this ID is currently hovered, regardless of the last
    /// input modality used.
    ///
    /// See [`HitboxId::is_hovered`] for more details.
    pub(crate) fn is_hovered_ignoring_last_input(self, window: &Window) -> bool {
        // If this hitbox has captured the pointer, it's always considered hovered
        if window.captured_hitbox == Some(self) {
            return true;
        }
        self.hit_test(window)
    }

    fn hit_test(self, window: &Window) -> bool {
        let hit_test = &window.mouse_hit_test;
        for id in hit_test.ids.iter().take(hit_test.hover_hitbox_count) {
            if self == *id {
                return true;
            }
        }
        false
    }

    /// Checks if the hitbox with this ID contains the mouse and should handle scroll events.
    /// Typically this should only be used when handling `ScrollWheelEvent`, and otherwise
    /// `is_hovered` should be used. See the documentation of `Hitbox::is_hovered` for details about
    /// this distinction.
    pub fn should_handle_scroll(self, window: &Window) -> bool {
        window.mouse_hit_test.ids.contains(&self)
    }

    fn next(mut self) -> HitboxId {
        HitboxId(self.0.wrapping_add(1))
    }
}

/// A rectangular region that potentially blocks hitboxes inserted prior.
/// See [Window::insert_hitbox] for more details.
#[derive(Clone, Debug, Deref)]
pub struct Hitbox {
    /// A unique identifier for the hitbox.
    pub id: HitboxId,
    /// The bounds of the hitbox.
    #[deref]
    pub bounds: Bounds<Pixels>,
    /// The content mask when the hitbox was inserted.
    pub content_mask: ContentMask<Pixels>,
    /// Flags that specify hitbox behavior.
    pub behavior: HitboxBehavior,
    /// Paint layer the hitbox was inserted on: 0 for the root pass, deferred priority + 1 inside a
    /// deferred draw, `usize::MAX` for prompts, drags and tooltips (Riseonly 0032).
    pub(crate) layer: usize,
    /// Inserted or replayed under a hidden element (Riseonly 0050): it keeps its bounds for the
    /// views that read them, but the mouse never finds it.
    pub(crate) hidden: bool,
    /// Riseonly 0129: the area of the element's children relative to `bounds.origin`, set by
    /// `InteractiveElement::wheel_into_children`; a wheel over the rest of the hitbox scrolls there.
    pub(crate) wheel_area: Option<Bounds<Pixels>>,
}

impl Hitbox {
    /// Checks if the hitbox is currently hovered. Returns `false` during keyboard input modality
    /// so that keyboard navigation suppresses hover highlights. Except when handling
    /// `ScrollWheelEvent`, this is typically what you want when determining whether to handle mouse
    /// events or paint hover styles.
    ///
    /// This can return `false` even when the hitbox contains the mouse, if a hitbox in front of
    /// this sets `HitboxBehavior::BlockMouse` (`InteractiveElement::occlude`) or
    /// `HitboxBehavior::BlockMouseExceptScroll` (`InteractiveElement::block_mouse_except_scroll`),
    /// or if the current input modality is keyboard (see [`Window::last_input_was_keyboard`]).
    ///
    /// Handling of `ScrollWheelEvent` should typically use `should_handle_scroll` instead.
    /// Concretely, this is due to use-cases like overlays that cause the elements under to be
    /// non-interactive while still allowing scrolling. More abstractly, this is because
    /// `is_hovered` is about element interactions directly under the mouse - mouse moves, clicks,
    /// hover styling, etc. In contrast, scrolling is about finding the current outer scrollable
    /// container.
    pub fn is_hovered(&self, window: &Window) -> bool {
        self.id.is_hovered(window)
    }

    /// Checks if the hitbox contains the mouse and should handle scroll events. Typically this
    /// should only be used when handling `ScrollWheelEvent`, and otherwise `is_hovered` should be
    /// used. See the documentation of `Hitbox::is_hovered` for details about this distinction.
    ///
    /// This can return `false` even when the hitbox contains the mouse, if a hitbox in front of
    /// this sets `HitboxBehavior::BlockMouse` (`InteractiveElement::occlude`).
    pub fn should_handle_scroll(&self, window: &Window) -> bool {
        self.id.should_handle_scroll(window)
    }
}

/// How the hitbox affects mouse behavior.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum HitboxBehavior {
    /// Normal hitbox mouse behavior, doesn't affect mouse handling for other hitboxes.
    #[default]
    Normal,

    /// All hitboxes behind this hitbox will be ignored and so will have `hitbox.is_hovered() ==
    /// false` and `hitbox.should_handle_scroll() == false`. Typically for elements this causes
    /// skipping of all mouse events, hover styles, and tooltips. This flag is set by
    /// [`InteractiveElement::occlude`].
    ///
    /// For mouse handlers that check those hitboxes, this behaves the same as registering a
    /// bubble-phase handler for every mouse event type:
    ///
    /// ```ignore
    /// window.on_mouse_event(move |_: &EveryMouseEventTypeHere, phase, window, cx| {
    ///     if phase == DispatchPhase::Capture && hitbox.is_hovered(window) {
    ///         cx.stop_propagation();
    ///     }
    /// })
    /// ```
    ///
    /// This has effects beyond event handling - any use of hitbox checking, such as hover
    /// styles and tooltips. These other behaviors are the main point of this mechanism. An
    /// alternative might be to not affect mouse event handling - but this would allow
    /// inconsistent UI where clicks and moves interact with elements that are not considered to
    /// be hovered.
    BlockMouse,

    /// All hitboxes behind this hitbox will have `hitbox.is_hovered() == false`, even when
    /// `hitbox.should_handle_scroll() == true`. Typically for elements this causes all mouse
    /// interaction except scroll events to be ignored - see the documentation of
    /// [`Hitbox::is_hovered`] for details. This flag is set by
    /// [`InteractiveElement::block_mouse_except_scroll`].
    ///
    /// For mouse handlers that check those hitboxes, this behaves the same as registering a
    /// bubble-phase handler for every mouse event type **except** `ScrollWheelEvent`:
    ///
    /// ```ignore
    /// window.on_mouse_event(move |_: &EveryMouseEventTypeExceptScroll, phase, window, cx| {
    ///     if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
    ///         cx.stop_propagation();
    ///     }
    /// })
    /// ```
    ///
    /// See the documentation of [`Hitbox::is_hovered`] for details of why `ScrollWheelEvent` is
    /// handled differently than other mouse events. If also blocking these scroll events is
    /// desired, then a `cx.stop_propagation()` handler like the one above can be used.
    ///
    /// This has effects beyond event handling - this affects any use of `is_hovered`, such as
    /// hover styles and tooltips. These other behaviors are the main point of this mechanism.
    /// An alternative might be to not affect mouse event handling - but this would allow
    /// inconsistent UI where clicks and moves interact with elements that are not considered to
    /// be hovered.
    BlockMouseExceptScroll,
}

/// An identifier for a tooltip.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct TooltipId(usize);

impl TooltipId {
    /// Checks if the tooltip is currently hovered.
    pub fn is_hovered(&self, window: &Window) -> bool {
        window
            .tooltip_bounds
            .as_ref()
            .is_some_and(|tooltip_bounds| {
                tooltip_bounds.id == *self
                    && tooltip_bounds.bounds.contains(&window.mouse_position())
            })
    }
}

pub(crate) struct TooltipBounds {
    id: TooltipId,
    bounds: Bounds<Pixels>,
}

#[derive(Clone)]
pub(crate) struct TooltipRequest {
    id: TooltipId,
    tooltip: AnyTooltip,
}

pub(crate) struct DeferredDraw {
    current_view: EntityId,
    priority: usize,
    parent_node: DispatchNodeId,
    element_id_stack: SmallVec<[ElementId; 32]>,
    text_style_stack: Vec<TextStyleRefinement>,
    content_mask: Option<ContentMask<Pixels>>,
    rem_size: Pixels,
    element: Option<AnyElement>,
    absolute_offset: Point<Pixels>,
    prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Range<PaintIndex>,
    /// Riseonly 0098: the index in the frame just drawn of the draw this one replays, so a spliced
    /// frame finds the holes it holds.
    copied_from: Option<usize>,
    /// Riseonly 0108: what the element read while it laid out, outside the views inside it
    /// (own) and with them (full), kept so a replayed draw still names its readers.
    reads: Option<DeferredReads>,
}

/// Riseonly 0108: the reads of a deferred draw, owned by the view that deferred it.
#[derive(Clone)]
pub(crate) struct DeferredReads {
    own: Rc<FxHashSet<EntityId>>,
    full: Rc<FxHashSet<EntityId>>,
}

pub(crate) struct Frame {
    pub(crate) focus: Option<FocusId>,
    pub(crate) window_active: bool,
    pub(crate) element_states: FxHashMap<(GlobalElementId, TypeId), ElementStateBox>,
    accessed_element_states: Vec<(GlobalElementId, TypeId)>,
    pub(crate) mouse_listeners: Vec<Option<AnyMouseListener>>,
    pub(crate) dispatch_tree: DispatchTree,
    pub(crate) scene: Scene,
    pub(crate) hitboxes: Vec<Hitbox>,
    pub(crate) window_control_hitboxes: Vec<(WindowControlArea, Hitbox)>,
    pub(crate) deferred_draws: Vec<DeferredDraw>,
    pub(crate) input_handlers: Vec<Option<PlatformInputHandler>>,
    pub(crate) tooltip_requests: Vec<Option<TooltipRequest>>,
    pub(crate) cursor_styles: Vec<CursorStyleRequest>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) debug_bounds: FxHashMap<String, Bounds<Pixels>>,
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub(crate) next_inspector_instance_ids: FxHashMap<Rc<crate::InspectorElementPath>, usize>,
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub(crate) inspector_hitboxes: FxHashMap<HitboxId, crate::InspectorElementId>,
    pub(crate) tab_stops: TabStopMap,
    /// Every cached view drawn in this frame with the entities its render read (Riseonly 0029).
    pub(crate) cached_views: Vec<(EntityId, Rc<FxHashSet<EntityId>>)>,
    /// Prepaint callbacks a cached view replayed at a new origin calls again (Riseonly 0041).
    pub(crate) move_listeners: Vec<MoveListener>,
    /// Cached views drawn with mouse listeners of an earlier origin and their bounds now: the
    /// mouse entering one makes it render (Riseonly 0044).
    pub(crate) moved_views: Vec<(EntityId, Bounds<Pixels>)>,
    /// Cached views this frame drew shown and in place, which the next frame may draw alone while
    /// it copies the rest of this one (Riseonly 0088).
    pub(crate) holes: FxHashMap<EntityId, HoleRecord>,
    /// What each view of `cached_views` read itself, outside the cached views in it (Riseonly 0088).
    pub(crate) own_reads: FxHashMap<EntityId, Rc<FxHashSet<EntityId>>>,
    /// Where the root pass of this frame started and ended (Riseonly 0088).
    pub(crate) prepaint_start: PrepaintStateIndex,
    pub(crate) prepaint_end: PrepaintStateIndex,
    pub(crate) paint_start: PaintIndex,
    pub(crate) paint_end: PaintIndex,
    /// Drawn by the root pass and its deferred draws alone (no prompt, drag or inspector; a tooltip
    /// is drawn again by every frame, 0102) at `viewport`, so the next frame may splice it
    /// (Riseonly 0088, deferred draws 0098).
    pub(crate) spliceable: bool,
    pub(crate) viewport: (Size<Pixels>, f32),
}

/// Riseonly 0099: the base of the paint masks inside a clip boundary, no clip at all.
fn unbounded_paint_mask() -> ContentMask<Pixels> {
    ContentMask {
        bounds: Bounds {
            origin: point(px(-1.0e6), px(-1.0e6)),
            size: size(px(2.0e6), px(2.0e6)),
        },
    }
}

/// Riseonly 0099: a hitbox mask recorded under `outer` with every edge `outer` drew opened up and
/// the rest moved by `offset`. An inner clip on the very edge of `outer` opens too; a hit test may
/// then reach a little more of the hitbox until the view renders again.
fn reopened_mask(mask: Bounds<Pixels>, outer: Bounds<Pixels>, offset: Point<Pixels>) -> Bounds<Pixels> {
    let near = |a: Pixels, b: Pixels| (f32::from(a) - f32::from(b)).abs() < 0.001;
    let far = 1.0e6;
    let left = if near(mask.left(), outer.left()) { px(-far) } else { mask.left() + offset.x };
    let top = if near(mask.top(), outer.top()) { px(-far) } else { mask.top() + offset.y };
    let right = if near(mask.right(), outer.right()) { px(far) } else { mask.right() + offset.x };
    let bottom = if near(mask.bottom(), outer.bottom()) { px(far) } else { mask.bottom() + offset.y };
    Bounds::from_corners(point(left, top), point(right, bottom))
}

/// Riseonly 0088: the window state a cached view was drawn under, to draw it again alone in its
/// place: the stacks its ancestors pushed, the pass state and the environment.
#[derive(Clone)]
pub(crate) struct HoleStacks {
    element_id_stack: SmallVec<[ElementId; 32]>,
    text_style_stack: Vec<TextStyleRefinement>,
    rendered_entity_stack: Vec<EntityId>,
    element_offset_stack: Vec<Point<Pixels>>,
    content_mask_stack: Vec<ContentMask<Pixels>>,
    paint_mask_stack: Vec<ContentMask<Pixels>>,
    scene_clip_stack: Vec<Bounds<ScaledPixels>>,
    /// Riseonly 0099: taken inside the clip boundary of the view (its paint stacks).
    pub(crate) own_clip_boundary: bool,
    surface_corner_clip_stack: Vec<(Bounds<Pixels>, Corners<Pixels>)>,
    transformation_stack: Vec<(TransformationMatrix, ContentMask<Pixels>, ContentMask<Pixels>)>,
    image_cache_stack: Vec<crate::AnyImageCache>,
    rem_size_override_stack: SmallVec<[Pixels; 8]>,
    element_opacity: f32,
    glyph_swap_key: u32,
    image_swap_key: u32,
    hitbox_layer: usize,
    env_fingerprint: u64,
    env_trace: Vec<(&'static str, u64)>,
    env: Option<Rc<dyn Any>>,
}

/// Riseonly 0088: a cached view drawn shown and in place, and how to draw it again alone there.
#[derive(Clone)]
pub(crate) struct HoleRecord {
    pub(crate) view: AnyView,
    pub(crate) style: crate::StyleRefinement,
    pub(crate) replays_moved: bool,
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) prepaint: Rc<HoleStacks>,
    pub(crate) paint: Rc<HoleStacks>,
    pub(crate) prepaint_range: Range<PrepaintStateIndex>,
    pub(crate) paint_range: Range<PaintIndex>,
}

/// Riseonly 0088: how a framework that keeps an environment in an App global (rise-ui) hands the
/// environment a cached view is drawn under to a frame that draws the view alone.
#[derive(Clone, Copy)]
pub struct EnvSnapshotHooks {
    /// The environment now.
    pub capture: fn(&mut App) -> Rc<dyn Any>,
    /// Puts `snapshot` in place and returns the environment it replaced.
    pub swap: fn(&mut App, Rc<dyn Any>) -> Rc<dyn Any>,
}

thread_local! {
    static ENV_SNAPSHOT_HOOKS: Cell<Option<EnvSnapshotHooks>> = const { Cell::new(None) };
}

/// Riseonly 0088: installs [`EnvSnapshotHooks`]. Until then a cached view drawn under an
/// environment is never drawn alone.
pub fn set_env_snapshot_hooks(hooks: EnvSnapshotHooks) {
    ENV_SNAPSHOT_HOOKS.with(|cell| cell.set(Some(hooks)));
}

fn env_snapshot_hooks() -> Option<EnvSnapshotHooks> {
    ENV_SNAPSHOT_HOOKS.with(|cell| cell.get())
}

/// Riseonly 0098: one pass of a spliced frame (the root pass, or one deferred draw) and where its
/// copied spans went. Segment k is the span before hole k (the last one after the last hole), as
/// (old range, new start); a pass replayed whole is one segment and no hole.
pub(crate) struct SplicePass<I> {
    segments: Vec<(Range<I>, I)>,
    holes: Vec<Range<I>>,
}

impl<I: Clone> SplicePass<I> {
    fn whole(old: Range<I>, new_start: I) -> Self {
        Self {
            segments: vec![(old, new_start)],
            holes: Vec::new(),
        }
    }

    /// Where `range` lies in the spliced frame: None when it lies outside this pass, inside a
    /// hole (drawn fresh) or only partly in one.
    fn remap(
        &self,
        range: &Range<I>,
        le: fn(&I, &I) -> bool,
        rebase: fn(&Range<I>, &Range<I>, &I) -> Option<Range<I>>,
    ) -> Option<Range<I>> {
        let (first, _) = self.segments.first()?;
        let (last, _) = self.segments.last()?;
        if !(le(&first.start, &range.start) && le(&range.end, &last.end)) {
            return None;
        }
        let mut start_segment = 0;
        let mut end_segment = 0;
        for hole in &self.holes {
            if le(&hole.start, &range.start) && le(&range.end, &hole.end) {
                return None;
            }
            if le(&hole.end, &range.start) {
                start_segment += 1;
            } else if le(&hole.start, &range.start) {
                return None;
            }
            if !le(&range.end, &hole.start) {
                end_segment += 1;
            }
        }
        let point = |index: &I, segment: usize| {
            let (old, new_start) = self.segments.get(segment)?;
            rebase(&(index.clone()..index.clone()), old, new_start).map(|range| range.start)
        };
        Some(point(&range.start, start_segment)?..point(&range.end, end_segment)?)
    }
}

/// Riseonly 0088: where the copied spans of a spliced frame went, per pass (0098).
#[derive(Default)]
pub(crate) struct SpliceMap {
    prepaint: Vec<SplicePass<PrepaintStateIndex>>,
    paint: Vec<SplicePass<PaintIndex>>,
}

impl SpliceMap {
    /// Where a prepaint range of the frame just drawn lies in the spliced one: None when it lies
    /// inside a hole (drawn fresh), only partly in one, or in a pass the frame did not copy.
    pub(crate) fn remap_prepaint(&self, range: &Range<PrepaintStateIndex>) -> Option<Range<PrepaintStateIndex>> {
        self.prepaint
            .iter()
            .find_map(|pass| pass.remap(range, PrepaintStateIndex::le, PrepaintStateIndex::rebase_within))
    }

    /// [`Self::remap_prepaint`] for a paint range.
    pub(crate) fn remap_paint(&self, range: &Range<PaintIndex>) -> Option<Range<PaintIndex>> {
        self.paint
            .iter()
            .find_map(|pass| pass.remap(range, PaintIndex::le, PaintIndex::rebase_within))
    }
}

/// Riseonly 0098: which pass of the frame just drawn a hole lies in: the root pass, or the
/// deferred draw with this index.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HolePass {
    Root,
    Deferred(usize),
}

impl HolePass {
    fn order(self) -> usize {
        match self {
            HolePass::Root => 0,
            HolePass::Deferred(index) => index + 1,
        }
    }
}

/// Riseonly 0098: a hole of a spliced frame, laid out before anything is copied.
struct SpliceHole {
    view_id: EntityId,
    pass: HolePass,
    prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Range<PaintIndex>,
    paint_stacks: Rc<HoleStacks>,
    element: AnyElement,
    stacks: HoleStacks,
    bounds: Bounds<Pixels>,
    drawn: bool,
}

/// Riseonly 0098: the holes of a spliced frame and where the spans of every pass went.
struct SpliceDraw {
    holes: Vec<SpliceHole>,
    map: SpliceMap,
    nodes: SpliceNodes,
}

impl SpliceDraw {
    fn holds(&self, pass: HolePass) -> bool {
        self.holes.iter().any(|hole| hole.pass == pass)
    }
}

/// What a paint ran under that a replay of it must match or carry over (Riseonly 0053): the
/// transformation and the mask [`Window::with_transformation`] clips with, and the element opacity
/// baked into the colours.
#[derive(Clone, PartialEq)]
pub(crate) struct PaintContext {
    transformation: TransformationMatrix,
    transformation_mask: Option<ContentMask<Pixels>>,
    opacity: f32,
}

impl PaintContext {
    /// Whether a paint under `self` lands where a paint under `other` would, the opacity aside.
    pub(crate) fn places_alike(&self, other: &PaintContext) -> bool {
        self.transformation == other.transformation
            && self.transformation_mask == other.transformation_mask
    }
}

impl Default for PaintContext {
    fn default() -> Self {
        Self {
            transformation: TransformationMatrix::unit(),
            transformation_mask: None,
            opacity: 1.0,
        }
    }
}

/// How a paint range of the frame just drawn goes into the next one.
enum PaintReplay {
    Same,
    /// Moved by an offset in scaled pixels (Riseonly 0041).
    Moved(Point<ScaledPixels>),
    /// Under another transformation (Riseonly 0053).
    Transformed(TransformedReplay),
    /// In place under another element opacity, colours times the factor (Riseonly 0065).
    Faded(f32),
    /// Moved by an offset in scaled pixels under another element opacity (Riseonly 0085).
    MovedFaded(Point<ScaledPixels>, f32),
}

/// A prepaint callback that reads window positions, recorded so a cached view replayed at a new
/// origin instead of rendering can hand it the move (Riseonly 0041).
#[derive(Clone)]
pub(crate) struct MoveListener {
    callback: Rc<dyn Fn(Point<Pixels>, Point<Pixels>, &mut Window, &mut App)>,
    /// How far the view moved since the frame that recorded the callback.
    offset: Point<Pixels>,
    /// How far the content mask around the callback moved with it (Riseonly 0043): the offset when
    /// a clipped cell scrolled whole with its clip, zero when the clip is the viewport that stayed.
    mask_offset: Point<Pixels>,
}

impl MoveListener {
    pub(crate) fn call(&self, window: &mut Window, cx: &mut App) {
        (self.callback)(self.offset, self.mask_offset, window, cx);
    }
}

#[derive(Clone, Default)]
pub(crate) struct PrepaintStateIndex {
    hitboxes_index: usize,
    tooltips_index: usize,
    deferred_draws_index: usize,
    dispatch_tree_index: usize,
    accessed_element_states_index: usize,
    line_layout_index: LineLayoutIndex,
    move_listeners_index: usize,
    moved_views_index: usize,
}

impl PrepaintStateIndex {
    /// Riseonly 0088: every index of `self` is at most the same index of `other`.
    pub(crate) fn le(&self, other: &Self) -> bool {
        self.hitboxes_index <= other.hitboxes_index
            && self.tooltips_index <= other.tooltips_index
            && self.deferred_draws_index <= other.deferred_draws_index
            && self.dispatch_tree_index <= other.dispatch_tree_index
            && self.accessed_element_states_index <= other.accessed_element_states_index
            && self.move_listeners_index <= other.move_listeners_index
            && self
                .line_layout_index
                .within(&(LineLayoutIndex::default()..other.line_layout_index.clone()))
    }

    /// Riseonly 0091: the first index of `inner` that lies outside `old`, why
    /// [`Self::rebase_within`] refused it, for the frame trace (`none` when it lies inside).
    pub(crate) fn outside_field(inner: &Range<Self>, old: &Range<Self>) -> &'static str {
        let outside = |field: fn(&Self) -> usize| {
            !(field(&old.start) <= field(&inner.start)
                && field(&inner.start) <= field(&inner.end)
                && field(&inner.end) <= field(&old.end))
        };
        let layouts = old.start.line_layout_index.clone()..old.end.line_layout_index.clone();
        if outside(|index| index.hitboxes_index) {
            "hitboxes"
        } else if outside(|index| index.tooltips_index) {
            "tooltips"
        } else if outside(|index| index.deferred_draws_index) {
            "deferred_draws"
        } else if outside(|index| index.dispatch_tree_index) {
            "dispatch_tree"
        } else if outside(|index| index.accessed_element_states_index) {
            "element_states"
        } else if outside(|index| index.move_listeners_index) {
            "move_listeners"
        } else if !(inner.start.line_layout_index.within(&layouts)
            && inner.end.line_layout_index.within(&layouts))
        {
            "line_layouts"
        } else {
            "none"
        }
    }

    /// Where the prepaint range `inner`, recorded inside `old`, lies after `old` was replayed
    /// starting at `new_start` (Riseonly 0045), or None when `inner` is not inside `old`. A replay
    /// copies hitboxes, tooltips, deferred draws, dispatch nodes, element state keys, line layouts
    /// and move listeners in order, so every index keeps its distance from the start; moved views
    /// are not copied by a replay, so the rebased range holds none.
    pub(crate) fn rebase_within(
        inner: &Range<Self>,
        old: &Range<Self>,
        new_start: &Self,
    ) -> Option<Range<Self>> {
        let within = |field: fn(&Self) -> usize| {
            field(&old.start) <= field(&inner.start)
                && field(&inner.start) <= field(&inner.end)
                && field(&inner.end) <= field(&old.end)
        };
        let layouts = old.start.line_layout_index.clone()..old.end.line_layout_index.clone();
        if !(within(|index| index.hitboxes_index)
            && within(|index| index.tooltips_index)
            && within(|index| index.deferred_draws_index)
            && within(|index| index.dispatch_tree_index)
            && within(|index| index.accessed_element_states_index)
            && within(|index| index.move_listeners_index)
            && inner.start.line_layout_index.within(&layouts)
            && inner.end.line_layout_index.within(&layouts))
        {
            return None;
        }
        let rebase = |index: &Self| Self {
            hitboxes_index: index.hitboxes_index - old.start.hitboxes_index
                + new_start.hitboxes_index,
            tooltips_index: index.tooltips_index - old.start.tooltips_index
                + new_start.tooltips_index,
            deferred_draws_index: index.deferred_draws_index - old.start.deferred_draws_index
                + new_start.deferred_draws_index,
            dispatch_tree_index: index.dispatch_tree_index - old.start.dispatch_tree_index
                + new_start.dispatch_tree_index,
            accessed_element_states_index: index.accessed_element_states_index
                - old.start.accessed_element_states_index
                + new_start.accessed_element_states_index,
            line_layout_index: index
                .line_layout_index
                .rebased(&old.start.line_layout_index, &new_start.line_layout_index),
            move_listeners_index: index.move_listeners_index - old.start.move_listeners_index
                + new_start.move_listeners_index,
            moved_views_index: new_start.moved_views_index,
        };
        Some(rebase(&inner.start)..rebase(&inner.end))
    }
}

/// The paint of a cached view kept out of the frames in which a hidden div skips it, replayed when
/// the view shows again with the same cache key, so a tab coming back does not render (Riseonly 0034).
pub(crate) struct ParkedPaint {
    scene: Vec<crate::scene::PaintOperation>,
    mouse_listeners: Vec<Option<AnyMouseListener>>,
    cursor_styles: Vec<CursorStyleRequest>,
    accessed_element_states: Vec<(GlobalElementId, TypeId)>,
    tab_stops: Vec<crate::tab_stop::TabStopOperation>,
    image_drops: u64,
}

#[derive(Clone, Default)]
pub(crate) struct PaintIndex {
    scene_index: usize,
    mouse_listeners_index: usize,
    input_handlers_index: usize,
    cursor_styles_index: usize,
    accessed_element_states_index: usize,
    tab_handle_index: usize,
    line_layout_index: LineLayoutIndex,
}

impl PaintIndex {
    /// Riseonly 0088: every index of `self` is at most the same index of `other`.
    pub(crate) fn le(&self, other: &Self) -> bool {
        self.scene_index <= other.scene_index
            && self.mouse_listeners_index <= other.mouse_listeners_index
            && self.input_handlers_index <= other.input_handlers_index
            && self.cursor_styles_index <= other.cursor_styles_index
            && self.accessed_element_states_index <= other.accessed_element_states_index
            && self.tab_handle_index <= other.tab_handle_index
            && self
                .line_layout_index
                .within(&(LineLayoutIndex::default()..other.line_layout_index.clone()))
    }

    /// Where the paint range `inner`, recorded inside `old`, lies after `old` was replayed unmoved
    /// starting at `new_start` (Riseonly 0063), or None when `inner` is not inside `old`. A same
    /// place replay copies scene operations, mouse listeners, input handlers, cursor styles,
    /// element state keys, tab stops and line layouts in order, so every index keeps its distance
    /// from the start.
    pub(crate) fn rebase_within(
        inner: &Range<Self>,
        old: &Range<Self>,
        new_start: &Self,
    ) -> Option<Range<Self>> {
        let within = |field: fn(&Self) -> usize| {
            field(&old.start) <= field(&inner.start)
                && field(&inner.start) <= field(&inner.end)
                && field(&inner.end) <= field(&old.end)
        };
        let layouts = old.start.line_layout_index.clone()..old.end.line_layout_index.clone();
        if !(within(|index| index.scene_index)
            && within(|index| index.mouse_listeners_index)
            && within(|index| index.input_handlers_index)
            && within(|index| index.cursor_styles_index)
            && within(|index| index.accessed_element_states_index)
            && within(|index| index.tab_handle_index)
            && inner.start.line_layout_index.within(&layouts)
            && inner.end.line_layout_index.within(&layouts))
        {
            return None;
        }
        let rebase = |index: &Self| Self {
            scene_index: index.scene_index - old.start.scene_index + new_start.scene_index,
            mouse_listeners_index: index.mouse_listeners_index - old.start.mouse_listeners_index
                + new_start.mouse_listeners_index,
            input_handlers_index: index.input_handlers_index - old.start.input_handlers_index
                + new_start.input_handlers_index,
            cursor_styles_index: index.cursor_styles_index - old.start.cursor_styles_index
                + new_start.cursor_styles_index,
            accessed_element_states_index: index.accessed_element_states_index
                - old.start.accessed_element_states_index
                + new_start.accessed_element_states_index,
            tab_handle_index: index.tab_handle_index - old.start.tab_handle_index
                + new_start.tab_handle_index,
            line_layout_index: index
                .line_layout_index
                .rebased(&old.start.line_layout_index, &new_start.line_layout_index),
        };
        Some(rebase(&inner.start)..rebase(&inner.end))
    }
}

impl Frame {
    pub(crate) fn new(dispatch_tree: DispatchTree) -> Self {
        Frame {
            focus: None,
            window_active: false,
            element_states: FxHashMap::default(),
            accessed_element_states: Vec::new(),
            mouse_listeners: Vec::new(),
            dispatch_tree,
            scene: Scene::default(),
            hitboxes: Vec::new(),
            window_control_hitboxes: Vec::new(),
            deferred_draws: Vec::new(),
            input_handlers: Vec::new(),
            tooltip_requests: Vec::new(),
            cursor_styles: Vec::new(),

            #[cfg(any(test, feature = "test-support"))]
            debug_bounds: FxHashMap::default(),

            #[cfg(any(feature = "inspector", debug_assertions))]
            next_inspector_instance_ids: FxHashMap::default(),

            #[cfg(any(feature = "inspector", debug_assertions))]
            inspector_hitboxes: FxHashMap::default(),
            tab_stops: TabStopMap::default(),
            cached_views: Vec::new(),
            move_listeners: Vec::new(),
            moved_views: Vec::new(),
            holes: FxHashMap::default(),
            own_reads: FxHashMap::default(),
            prepaint_start: PrepaintStateIndex::default(),
            prepaint_end: PrepaintStateIndex::default(),
            paint_start: PaintIndex::default(),
            paint_end: PaintIndex::default(),
            spliceable: false,
            viewport: (Size::default(), 0.0),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.element_states.clear();
        self.accessed_element_states.clear();
        self.mouse_listeners.clear();
        self.dispatch_tree.clear();
        self.scene.clear();
        self.input_handlers.clear();
        self.tooltip_requests.clear();
        self.cursor_styles.clear();
        self.hitboxes.clear();
        self.window_control_hitboxes.clear();
        self.deferred_draws.clear();
        self.tab_stops.clear();
        self.cached_views.clear();
        self.move_listeners.clear();
        self.moved_views.clear();
        self.holes.clear();
        self.own_reads.clear();
        self.spliceable = false;
        self.focus = None;

        #[cfg(any(test, feature = "test-support"))]
        {
            self.debug_bounds.clear();
        }

        #[cfg(any(feature = "inspector", debug_assertions))]
        {
            self.next_inspector_instance_ids.clear();
            self.inspector_hitboxes.clear();
        }
    }

    pub(crate) fn cursor_style(&self, window: &Window) -> Option<CursorStyle> {
        self.cursor_styles
            .iter()
            .rev()
            .fold_while(None, |style, request| match request.hitbox_id {
                None => Done(Some(request.style)),
                Some(hitbox_id) => Continue(style.or_else(|| {
                    hitbox_id
                        .is_hovered_ignoring_last_input(window)
                        .then_some(request.style)
                })),
            })
            .into_inner()
    }

    pub(crate) fn hit_test(&self, position: Point<Pixels>) -> HitTest {
        let mut set_hover_hitbox_count = false;
        let mut hit_test = HitTest::default();
        // Riseonly 0032: deferred draws paint in priority order across nesting rounds, but their
        // hitboxes are inserted round by round, so a nested low priority layer used to sit above
        // a root level high priority one for the mouse. Hit test in paint order instead.
        let sorted = self
            .hitboxes
            .windows(2)
            .all(|pair| pair[0].layer <= pair[1].layer);
        let order = (!sorted).then(|| {
            let mut order: Vec<usize> = (0..self.hitboxes.len()).collect();
            order.sort_by_key(|ix| self.hitboxes[*ix].layer);
            order
        });
        for position_ix in (0..self.hitboxes.len()).rev() {
            let hitbox = &self.hitboxes[order.as_ref().map_or(position_ix, |order| order[position_ix])];
            // Riseonly 0050: a hidden subtree still prepaints, and a parked cached page replays its
            // hitboxes at the bounds it had when shown; an occluding one swallowed clicks on what
            // is drawn there now (X1418).
            if hitbox.hidden {
                continue;
            }
            let bounds = hitbox.bounds.intersect(&hitbox.content_mask.bounds);
            if bounds.contains(&position) {
                hit_test.ids.push(hitbox.id);
                if !set_hover_hitbox_count
                    && hitbox.behavior == HitboxBehavior::BlockMouseExceptScroll
                {
                    hit_test.hover_hitbox_count = hit_test.ids.len();
                    set_hover_hitbox_count = true;
                }
                if hitbox.behavior == HitboxBehavior::BlockMouse {
                    break;
                }
            }
        }
        if !set_hover_hitbox_count {
            hit_test.hover_hitbox_count = hit_test.ids.len();
        }
        hit_test
    }

    /// Riseonly 0129: where a wheel at `position` goes when the front-most hitbox under it with a
    /// wheel area (`InteractiveElement::wheel_into_children`) does not have the pointer over its
    /// children: the nearest point inside them, kept clear of their side edges. None otherwise.
    pub(crate) fn wheel_redirect(
        &self,
        position: Point<Pixels>,
        hit_test: &HitTest,
    ) -> Option<Point<Pixels>> {
        let areas: SmallVec<[(HitboxId, Bounds<Pixels>); 4]> = self
            .hitboxes
            .iter()
            .filter(|hitbox| !hitbox.hidden)
            .filter_map(|hitbox| {
                hitbox.wheel_area.map(|area| {
                    (
                        hitbox.id,
                        Bounds {
                            origin: hitbox.bounds.origin + area.origin,
                            size: area.size,
                        },
                    )
                })
            })
            .collect();
        if areas.is_empty() {
            return None;
        }
        let area = hit_test.ids.iter().find_map(|id| {
            areas
                .iter()
                .find(|(area_id, _)| area_id == id)
                .map(|(_, area)| *area)
        })?;
        if area.contains(&position)
            || area.size.width <= px(0.)
            || area.size.height <= px(1.)
        {
            return None;
        }
        let inset = (area.size.width / 2.).min(px(24.));
        let min = point(area.left() + inset, area.top());
        let max = point(area.right() - inset, area.bottom() - px(1.));
        Some(position.clamp(&min, &max))
    }

    pub(crate) fn focus_path(&self) -> SmallVec<[FocusId; 8]> {
        self.focus
            .map(|focus_id| self.dispatch_tree.focus_path(focus_id))
            .unwrap_or_default()
    }

    pub(crate) fn finish(&mut self, prev_frame: &mut Self) {
        for element_state_key in &self.accessed_element_states {
            if let Some((element_state_key, element_state)) =
                prev_frame.element_states.remove_entry(element_state_key)
            {
                self.element_states.insert(element_state_key, element_state);
            }
        }

        self.scene.finish();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
enum InputModality {
    Mouse,
    Keyboard,
    Touch,
}

/// Holds the state for a specific window.
pub struct Window {
    pub(crate) handle: AnyWindowHandle,
    pub(crate) invalidator: WindowInvalidator,
    pub(crate) removed: bool,
    pub(crate) platform_window: Box<dyn PlatformWindow>,
    display_id: Option<DisplayId>,
    is_resizable: bool,
    is_minimizable: bool,
    sprite_atlas: Arc<dyn PlatformAtlas>,
    text_system: Arc<WindowTextSystem>,
    text_rendering_mode: Rc<Cell<TextRenderingMode>>,
    rem_size: Pixels,
    /// The stack of override values for the window's rem size.
    ///
    /// This is used by `with_rem_size` to allow rendering an element tree with
    /// a given rem size.
    rem_size_override_stack: SmallVec<[Pixels; 8]>,
    pub(crate) viewport_size: Size<Pixels>,
    layout_engine: Option<TaffyLayoutEngine>,
    pub(crate) root: Option<AnyView>,
    pub(crate) element_id_stack: SmallVec<[ElementId; 32]>,
    pub(crate) text_style_stack: Vec<TextStyleRefinement>,
    pub(crate) rendered_entity_stack: Vec<EntityId>,
    pub(crate) element_offset_stack: Vec<Point<Pixels>>,
    pub(crate) element_opacity: f32,
    pub(crate) glyph_swap_key: u32,
    pub(crate) image_swap_key: u32,
    pub(crate) content_mask_stack: Vec<ContentMask<Pixels>>,
    /// Riseonly 0099: the masks of `content_mask_stack` without what lies outside the clip boundary
    /// of the nearest cached view around them, the clips its primitives keep.
    pub(crate) paint_mask_stack: Vec<ContentMask<Pixels>>,
    /// Riseonly 0099: the hole a spliced frame paints now sits inside the clip boundary of its
    /// view, already open in the stacks it was drawn under.
    pub(crate) hole_clip_boundary: bool,
    pub(crate) surface_corner_clip_stack: Vec<(Bounds<Pixels>, Corners<Pixels>)>,
    pub(crate) transformation_stack: Vec<(TransformationMatrix, ContentMask<Pixels>, ContentMask<Pixels>)>,
    pub(crate) requested_autoscroll: Option<Bounds<Pixels>>,
    pub(crate) image_cache_stack: Vec<AnyImageCache>,
    pub(crate) rendered_frame: Frame,
    pub(crate) next_frame: Frame,
    next_hitbox_id: HitboxId,
    hitbox_layer: usize,
    pub(crate) next_tooltip_id: TooltipId,
    pub(crate) tooltip_bounds: Option<TooltipBounds>,
    next_frame_callbacks: Rc<RefCell<Vec<FrameCallback>>>,
    pub(crate) dirty_views: FxHashSet<EntityId>,
    /// Riseonly 0088: the views notified or reading a notified entity, without the ancestors
    /// `dirty_views` adds; the entities the last frame drawn by the root pass read.
    direct_dirty: FxHashSet<EntityId>,
    last_accessed: FxHashSet<EntityId>,
    /// Riseonly 0088: what the window read outside every cached view in the last full draw (the
    /// root and other plain views, which a spliced frame does not render), and whether one of
    /// these entities was notified since the last frame.
    root_own: FxHashSet<EntityId>,
    root_dirty: bool,
    spliced: bool,
    focus_listeners: SubscriberSet<(), AnyWindowFocusListener>,
    pub(crate) focus_lost_listeners: SubscriberSet<(), AnyObserver>,
    default_prevented: bool,
    mouse_position: Point<Pixels>,
    mouse_hit_test: HitTest,
    modifiers: Modifiers,
    capslock: Capslock,
    scale_factor: f32,
    zoom: f32,
    pub(crate) bounds_observers: SubscriberSet<(), AnyObserver>,
    appearance: WindowAppearance,
    pub(crate) appearance_observers: SubscriberSet<(), AnyObserver>,
    pub(crate) button_layout_observers: SubscriberSet<(), AnyObserver>,
    active: Rc<Cell<bool>>,
    hovered: Rc<Cell<bool>>,
    pub(crate) needs_present: Rc<Cell<bool>>,
    /// Tracks recent input event timestamps to determine if input is arriving at a high rate.
    /// Used to selectively enable VRR optimization only when input rate exceeds 60fps.
    pub(crate) input_rate_tracker: Rc<RefCell<InputRateTracker>>,
    #[cfg(feature = "input-latency-histogram")]
    input_latency_tracker: InputLatencyTracker,
    last_input_modality: InputModality,
    /// Riseonly 0069: views that read the input modality while drawing; a modality change
    /// notifies them instead of refreshing the window.
    modality_readers: RefCell<FxHashSet<EntityId>>,
    /// Riseonly 0111: views that drew a hitbox under the mouse, with that hitbox. A modality
    /// change flips `is_hovered` only for a hitbox under the mouse at that moment; the others
    /// read hover the same way under either modality and are dropped.
    hover_modality_readers: RefCell<FxHashSet<(EntityId, HitboxId)>>,
    /// Riseonly 0126: whether the mouse was inside each cached view when it last rendered, so a
    /// view replayed moved under a still mouse renders again when that changed.
    pub(crate) mouse_inside_at_render: FxHashMap<EntityId, bool>,
    /// Riseonly 0087: views that read the window active status while drawing; an activation
    /// change notifies them instead of refreshing the window.
    activation_readers: RefCell<FxHashSet<EntityId>>,
    pub(crate) refreshing: bool,
    /// Number of the frame being drawn, raised at the start of every draw. A cached view replays
    /// its recorded ranges only when they were recorded in the frame just before this one.
    pub(crate) draw_count: u64,
    /// Riseonly 0036: fingerprint of the environment values the elements being prepainted sit in.
    /// A cached view keeps it in its cache key, so a changed value above it renders it fresh.
    pub(crate) env_fingerprint: u64,
    /// Riseonly 0086: the environment keys around the element being prepainted, outermost first,
    /// each with the fingerprint of its own value, for the frame trace of a miss on env.
    pub(crate) env_trace: Vec<(&'static str, u64)>,
    /// Number of `Visibility::Hidden` divs around the element being prepainted (Riseonly 0034).
    pub(crate) hidden_depth: usize,
    /// Deferred draws dropped so far because a hidden div held them (Riseonly 0034).
    pub(crate) hidden_dropped_deferred: u64,
    /// Images dropped from the sprite atlas so far; a parked paint may point at their tiles.
    pub(crate) image_drops: u64,
    /// Atlas keys of dropped images, removed when the next frame starts: the frame on screen may
    /// still draw their tiles and is presented again without a draw while input runs fast (Riseonly 0128).
    pending_image_drops: Vec<AtlasKey>,
    pub(crate) activation_observers: SubscriberSet<(), AnyObserver>,
    pub(crate) focus: Option<FocusId>,
    focus_enabled: bool,
    /// Incremented every time focus moves. Used to invalidate a
    /// pending keyboard activation state when focus changes.
    pub(crate) focus_generation: u64,
    pending_input: Option<PendingInput>,
    pending_modifier: ModifierState,
    pub(crate) pending_input_observers: SubscriberSet<(), AnyObserver>,
    prompt: Option<RenderablePromptHandle>,
    pub(crate) client_inset: Option<Pixels>,
    /// The hitbox that has captured the pointer, if any.
    /// While captured, mouse events route to this hitbox regardless of hit testing.
    captured_hitbox: Option<HitboxId>,
    #[cfg(any(feature = "inspector", debug_assertions))]
    inspector: Option<Entity<Inspector>>,
    pub(crate) a11y: A11y,
}

#[derive(Clone, Debug, Default)]
struct ModifierState {
    modifiers: Modifiers,
    saw_other_input: bool,
}

/// Tracks input event timestamps to determine if input is arriving at a high rate.
/// Used for selective VRR (Variable Refresh Rate) optimization.
#[derive(Clone, Debug)]
pub(crate) struct InputRateTracker {
    timestamps: Vec<Instant>,
    window: Duration,
    inputs_per_second: u32,
    sustain_until: Instant,
    sustain_duration: Duration,
}

impl Default for InputRateTracker {
    fn default() -> Self {
        Self {
            timestamps: Vec::new(),
            window: Duration::from_millis(100),
            inputs_per_second: 60,
            sustain_until: Instant::now(),
            sustain_duration: Duration::from_secs(1),
        }
    }
}

impl InputRateTracker {
    pub fn record_input(&mut self) {
        let now = Instant::now();
        self.timestamps.push(now);
        self.prune_old_timestamps(now);

        let min_events = self.inputs_per_second as u128 * self.window.as_millis() / 1000;
        if self.timestamps.len() as u128 >= min_events {
            self.sustain_until = now + self.sustain_duration;
        }
    }

    pub fn is_high_rate(&self) -> bool {
        Instant::now() < self.sustain_until
    }

    fn prune_old_timestamps(&mut self, now: Instant) {
        self.timestamps
            .retain(|&t| now.duration_since(t) <= self.window);
    }
}

/// A point-in-time snapshot of the input-latency histograms for a window,
/// suitable for external formatting.
#[cfg(feature = "input-latency-histogram")]
#[derive(Clone)]
pub struct InputLatencySnapshot {
    /// Histogram of input-to-frame latency samples, in nanoseconds.
    pub latency_histogram: Histogram<u64>,
    /// Histogram of input events coalesced per rendered frame.
    pub events_per_frame_histogram: Histogram<u64>,
    /// Count of input events that arrived mid-draw and were excluded from
    /// latency recording.
    pub mid_draw_events_dropped: u64,
}

/// Records the time between when the first input event in a frame is dispatched
/// and when the resulting frame is presented, capturing worst-case latency when
/// multiple events are coalesced into a single frame.
#[cfg(feature = "input-latency-histogram")]
struct InputLatencyTracker {
    /// Timestamp of the first unrendered input event in the current frame;
    /// cleared when a frame is presented.
    first_input_at: Option<Instant>,
    /// Count of input events received since the last frame was presented.
    pending_input_count: u64,
    /// Histogram of input-to-frame latency samples, in nanoseconds.
    latency_histogram: Histogram<u64>,
    /// Histogram of input events coalesced per rendered frame.
    events_per_frame_histogram: Histogram<u64>,
    /// Count of input events that arrived mid-draw and were excluded from
    /// latency recording because their effects won't appear until the next frame.
    mid_draw_events_dropped: u64,
}

#[cfg(feature = "input-latency-histogram")]
impl InputLatencyTracker {
    fn new() -> Result<Self> {
        Ok(Self {
            first_input_at: None,
            pending_input_count: 0,
            latency_histogram: Histogram::new(3)
                .map_err(|e| anyhow!("Failed to create input latency histogram: {e}"))?,
            events_per_frame_histogram: Histogram::new(3)
                .map_err(|e| anyhow!("Failed to create events per frame histogram: {e}"))?,
            mid_draw_events_dropped: 0,
        })
    }

    /// Record that an input event was dispatched at the given time.
    /// Only the first event's timestamp per frame is retained (worst-case latency).
    fn record_input(&mut self, dispatch_time: Instant) {
        self.first_input_at.get_or_insert(dispatch_time);
        self.pending_input_count += 1;
    }

    /// Record that an input event arrived during a draw phase and was excluded
    /// from latency tracking.
    fn record_mid_draw_input(&mut self) {
        self.mid_draw_events_dropped += 1;
    }

    /// Record that a frame was presented, flushing pending latency and coalescing samples.
    fn record_frame_presented(&mut self) {
        if let Some(first_input_at) = self.first_input_at.take() {
            let latency_nanos = first_input_at.elapsed().as_nanos() as u64;
            self.latency_histogram.record(latency_nanos).ok();
        }
        if self.pending_input_count > 0 {
            self.events_per_frame_histogram
                .record(self.pending_input_count)
                .ok();
            self.pending_input_count = 0;
        }
    }

    fn snapshot(&self) -> InputLatencySnapshot {
        InputLatencySnapshot {
            latency_histogram: self.latency_histogram.clone(),
            events_per_frame_histogram: self.events_per_frame_histogram.clone(),
            mid_draw_events_dropped: self.mid_draw_events_dropped,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DrawPhase {
    None,
    Prepaint,
    Paint,
    Focus,
}

#[derive(Default, Debug)]
struct PendingInput {
    keystrokes: SmallVec<[Keystroke; 1]>,
    focus: Option<FocusId>,
    timer: Option<Task<()>>,
    needs_timeout: bool,
}

pub(crate) struct ElementStateBox {
    pub(crate) inner: Box<dyn Any>,
    #[cfg(debug_assertions)]
    pub(crate) type_name: &'static str,
}

fn default_bounds(display_id: Option<DisplayId>, cx: &mut App) -> WindowBounds {
    // TODO, BUG: if you open a window with the currently active window
    // on the stack, this will erroneously fallback to `None`
    //
    // TODO these should be the initial window bounds not considering maximized/fullscreen
    let active_window_bounds = cx
        .active_window()
        .and_then(|w| w.update(cx, |_, window, _| window.window_bounds()).ok());

    const CASCADE_OFFSET: f32 = 25.0;

    let display = display_id
        .map(|id| cx.find_display(id))
        .unwrap_or_else(|| cx.primary_display());

    let default_placement = || Bounds::new(point(px(0.), px(0.)), DEFAULT_WINDOW_SIZE);

    // Use visible_bounds to exclude taskbar/dock areas
    let display_bounds = display
        .as_ref()
        .map(|d| d.visible_bounds())
        .unwrap_or_else(default_placement);

    let (
        Bounds {
            origin: base_origin,
            size: base_size,
        },
        window_bounds_ctor,
    ): (_, fn(Bounds<Pixels>) -> WindowBounds) = match active_window_bounds {
        Some(bounds) => match bounds {
            WindowBounds::Windowed(bounds) => (bounds, WindowBounds::Windowed),
            WindowBounds::Maximized(bounds) => (bounds, WindowBounds::Maximized),
            WindowBounds::Fullscreen(bounds) => (bounds, WindowBounds::Fullscreen),
        },
        None => (
            display
                .as_ref()
                .map(|d| d.default_bounds())
                .unwrap_or_else(default_placement),
            WindowBounds::Windowed,
        ),
    };

    let cascade_offset = point(px(CASCADE_OFFSET), px(CASCADE_OFFSET));
    let proposed_origin = base_origin + cascade_offset;
    let proposed_bounds = Bounds::new(proposed_origin, base_size);

    let display_right = display_bounds.origin.x + display_bounds.size.width;
    let display_bottom = display_bounds.origin.y + display_bounds.size.height;
    let window_right = proposed_bounds.origin.x + proposed_bounds.size.width;
    let window_bottom = proposed_bounds.origin.y + proposed_bounds.size.height;

    let fits_horizontally = window_right <= display_right;
    let fits_vertically = window_bottom <= display_bottom;

    let final_origin = match (fits_horizontally, fits_vertically) {
        (true, true) => proposed_origin,
        (false, true) => point(display_bounds.origin.x, base_origin.y),
        (true, false) => point(base_origin.x, display_bounds.origin.y),
        (false, false) => display_bounds.origin,
    };
    window_bounds_ctor(Bounds::new(final_origin, base_size))
}

impl Window {
    pub(crate) fn new(
        handle: AnyWindowHandle,
        options: WindowOptions,
        cx: &mut App,
    ) -> Result<Self> {
        let WindowOptions {
            window_bounds,
            titlebar,
            focus,
            show,
            kind,
            is_movable,
            app_owns_titlebar_drag,
            is_resizable,
            is_minimizable,
            display_id,
            window_background,
            app_id,
            window_min_size,
            window_decorations,
            #[cfg_attr(
                not(any(target_os = "linux", target_os = "freebsd")),
                allow(unused_variables)
            )]
            icon,
            #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
            tabbing_identifier,
        } = options;

        let initial_window_title = titlebar
            .as_ref()
            .and_then(|titlebar| titlebar.title.clone());

        let window_bounds = window_bounds.unwrap_or_else(|| default_bounds(display_id, cx));
        let mut platform_window = cx.platform.open_window(
            handle,
            WindowParams {
                bounds: window_bounds.get_bounds(),
                titlebar,
                kind,
                is_movable,
                app_owns_titlebar_drag,
                is_resizable,
                is_minimizable,
                focus,
                show,
                display_id,
                window_min_size,
                app_id: app_id.clone(),
                icon,
                #[cfg(target_os = "macos")]
                tabbing_identifier,
            },
        )?;

        let tab_bar_visible = platform_window.tab_bar_visible();
        SystemWindowTabController::init_visible(cx, tab_bar_visible);
        if let Some(tabs) = platform_window.tabbed_windows() {
            SystemWindowTabController::add_tab(cx, handle.window_id(), tabs);
        }

        let display_id = platform_window.display().map(|display| display.id());
        let sprite_atlas = platform_window.sprite_atlas();
        let zoom = cx.window_zoom;
        let mouse_position = unzoom_point(platform_window.mouse_position(), zoom);
        let modifiers = platform_window.modifiers();
        let capslock = platform_window.capslock();
        let content_size = unzoom_size(platform_window.content_size(), zoom);
        let scale_factor = platform_window.scale_factor() * zoom;
        let appearance = platform_window.appearance();
        let text_system = Arc::new(WindowTextSystem::new(cx.text_system().clone()));
        let invalidator = WindowInvalidator::new();
        let active = Rc::new(Cell::new(platform_window.is_active()));
        let hovered = Rc::new(Cell::new(platform_window.is_hovered()));
        let needs_present = Rc::new(Cell::new(false));
        let next_frame_callbacks: Rc<RefCell<Vec<FrameCallback>>> = Default::default();
        let input_rate_tracker = Rc::new(RefCell::new(InputRateTracker::default()));
        let last_frame_time = Rc::new(Cell::new(None));

        platform_window
            .request_decorations(window_decorations.unwrap_or(WindowDecorations::Server));
        platform_window.set_background_appearance(window_background);

        match window_bounds {
            WindowBounds::Fullscreen(_) => platform_window.toggle_fullscreen(),
            WindowBounds::Maximized(_) => platform_window.zoom(),
            WindowBounds::Windowed(_) => {}
        }

        let accessibility_force_disabled = cx.accessibility_force_disabled;
        let a11y_active_flag = Arc::new(AtomicBool::new(false));

        #[cfg(not(target_family = "wasm"))]
        if !accessibility_force_disabled {
            let mut initial_root_node = accesskit::Node::new(accesskit::Role::Window);
            if let Some(title) = &initial_window_title {
                initial_root_node.set_label(title.to_string());
            }
            let initial_tree = accesskit::TreeUpdate {
                nodes: vec![(ROOT_NODE_ID, initial_root_node)],
                tree: Some(accesskit::Tree::new(ROOT_NODE_ID)),
                tree_id: accesskit::TreeId::ROOT,
                focus: ROOT_NODE_ID,
            };
            let (activation_sender, activation_receiver) = async_channel::unbounded::<()>();
            let (deactivation_sender, deactivation_receiver) = async_channel::unbounded::<()>();
            let (action_sender, action_receiver) =
                async_channel::unbounded::<accesskit::ActionRequest>();

            platform_window.a11y_init(crate::A11yCallbacks {
                activation: {
                    let active_flag = a11y_active_flag.clone();
                    Box::new(move || {
                        log::info!("Accessibility activated");
                        active_flag.store(true, SeqCst);
                        activation_sender.send_blocking(()).log_err();
                        Some(initial_tree.clone())
                    })
                },
                action: Box::new(move |request| {
                    action_sender.send_blocking(request).log_err();
                }),
                deactivation: {
                    let active_flag = a11y_active_flag.clone();
                    Box::new(move || {
                        log::info!("Accessibility deactivated");
                        active_flag.store(false, SeqCst);
                        deactivation_sender.send_blocking(()).log_err();
                    })
                },
            });

            // A11y can be activated at any time, and so we cannot compute a
            // correct `TreeUpdate` on-demand. When this happens, we return a
            // default empty `TreeUpdate`.
            //
            // So we force a new frame, which will then send a correct `TreeUpdate`.
            let mut async_cx = cx.to_async();
            cx.foreground_executor()
                .spawn(async move {
                    while activation_receiver.recv().await.is_ok() {
                        handle
                            .update(&mut async_cx, |_, window, _| window.refresh())
                            .log_err();
                    }
                })
                .detach();

            let mut async_cx = cx.to_async();
            cx.foreground_executor()
                .spawn(async move {
                    while deactivation_receiver.recv().await.is_ok() {
                        handle
                            .update(&mut async_cx, |_, window, _| window.refresh())
                            .log_err();
                    }
                })
                .detach();

            let mut async_cx = cx.to_async();
            cx.foreground_executor()
                .spawn(async move {
                    while let Ok(request) = action_receiver.recv().await {
                        handle
                            .update(&mut async_cx, |_, window, cx| {
                                window.handle_a11y_action(request, cx);
                            })
                            .log_err();
                    }
                })
                .detach();
        }

        platform_window.on_close(Box::new({
            let window_id = handle.window_id();
            let mut cx = cx.to_async();
            move || {
                let _ = handle.update(&mut cx, |_, window, _| window.remove_window());
                let _ = cx.update(|cx| {
                    SystemWindowTabController::remove_tab(cx, window_id);
                });
            }
        }));
        platform_window.on_request_frame(Box::new({
            let mut cx = cx.to_async();
            let invalidator = invalidator.clone();
            let active = active.clone();
            let needs_present = needs_present.clone();
            let next_frame_callbacks = next_frame_callbacks.clone();
            let input_rate_tracker = input_rate_tracker.clone();
            let mut deferred_force_render = false;
            move |request_frame_options| {
                // This must be checked before anything else: if this request
                // arrived re-entrantly while a draw is on this thread's stack
                // (e.g. via a nested message pump in the Windows window
                // procedure), drawing would nest draws, and even touching the
                // App would panic on its already-mutable borrow. Skip instead;
                // the platform leaves the window invalidated (or re-invalidates
                // it), so a fresh request arrives once the in-progress draw
                // unwinds. Remember force_render so the deferred frame still
                // bypasses the view cache.
                //
                // Returning here skips `complete_frame`, which on Wayland would
                // stall the window's frame callbacks (no `surface.commit()`) —
                // but calling it would hit the App borrow panic above, and this
                // branch is unreachable there in practice: only Windows pumps
                // platform events (and thus requests frames) mid-draw.
                if draw_in_progress() {
                    log::debug!("deferring re-entrant window draw request");
                    deferred_force_render |= request_frame_options.force_render;
                    return;
                }
                // Take the deferred flag first: `||` short-circuits, and leaving
                // the flag set when this request already forces a render would
                // force a second, redundant render on the next frame.
                let force_render =
                    mem::take(&mut deferred_force_render) || request_frame_options.force_render;

                let thermal_state = handle
                    .update(&mut cx, |_, _, cx| cx.thermal_state())
                    .log_err();

                // Throttle frame rate based on conditions:
                // - Thermal pressure (Serious/Critical): cap to ~60fps
                // - Inactive window (not focused): cap to ~30fps to save energy
                let min_frame_interval = if request_frame_options.require_presentation
                    || (!request_frame_options.force_render
                        && next_frame_callbacks.borrow().is_empty())
                {
                    None
                } else if !active.get() && !input_rate_tracker.borrow_mut().is_high_rate() {
                    Some(Duration::from_micros(33333))
                } else if let Some(ThermalState::Critical | ThermalState::Serious) = thermal_state {
                    Some(Duration::from_micros(16667))
                } else {
                    None
                };

                let now = Instant::now();
                if let Some(min_interval) = min_frame_interval {
                    if let Some(last_frame) = last_frame_time.get()
                        && now.duration_since(last_frame) < min_interval
                    {
                        // Don't lose a pending forced render to throttling.
                        deferred_force_render |= force_render;
                        // Must still complete the frame on platforms that require it.
                        // On Wayland, `surface.frame()` was already called to request the
                        // next frame callback, so we must call `surface.commit()` (via
                        // `complete_frame`) or the compositor won't send another callback.
                        handle
                            .update(&mut cx, |_, window, _| window.complete_frame())
                            .log_err();
                        return;
                    }
                }
                last_frame_time.set(Some(now));

                let next_frame_callbacks = next_frame_callbacks.take();
                if !next_frame_callbacks.is_empty() {
                    handle
                        .update(&mut cx, |_, window, cx| {
                            for callback in next_frame_callbacks {
                                callback(window, cx);
                            }
                        })
                        .log_err();
                }

                // Keep presenting if input was recently arriving at a high rate (>= 60fps).
                // Once high-rate input is detected, we sustain presentation for 1 second
                // to prevent display underclocking during active input.
                let needs_present = request_frame_options.require_presentation
                    || needs_present.get()
                    || input_rate_tracker.borrow_mut().is_high_rate();

                if invalidator.is_dirty() || force_render {
                    measure("frame duration", || {
                        handle
                            .update(&mut cx, |_, window, cx| {
                                if force_render {
                                    // Bypass cached view reuse so we don't replay stale
                                    // atlas tile references after a GPU device recovery.
                                    window.refresh();
                                }
                                let arena_clear_needed = window.draw(cx);
                                window.present();
                                arena_clear_needed.clear(cx);
                            })
                            .log_err();
                    })
                } else if needs_present {
                    handle
                        .update(&mut cx, |_, window, _| window.present())
                        .log_err();
                }

                handle
                    .update(&mut cx, |_, window, _| {
                        window.complete_frame();
                    })
                    .log_err();
            }
        }));
        platform_window.on_resize(Box::new({
            let mut cx = cx.to_async();
            move |_, _| {
                handle
                    .update(&mut cx, |_, window, cx| window.bounds_changed(cx))
                    .log_err();
            }
        }));
        platform_window.on_moved(Box::new({
            let mut cx = cx.to_async();
            move || {
                handle
                    .update(&mut cx, |_, window, cx| window.bounds_changed(cx))
                    .log_err();
            }
        }));
        platform_window.on_appearance_changed(Box::new({
            let cx = cx.to_async();
            let foreground_executor = cx.foreground_executor().clone();
            move || {
                let mut cx = cx.clone();
                // Defer the update because changing the AppKit appearance may
                // synchronously invoke this callback while App is already borrowed.
                foreground_executor
                    .spawn(async move {
                        handle
                            .update(&mut cx, |_, window, cx| window.appearance_changed(cx))
                            .log_err();
                    })
                    .detach();
            }
        }));
        platform_window.on_button_layout_changed(Box::new({
            let mut cx = cx.to_async();
            move || {
                handle
                    .update(&mut cx, |_, window, cx| window.button_layout_changed(cx))
                    .log_err();
            }
        }));
        platform_window.on_active_status_change(Box::new({
            let mut cx = cx.to_async();
            move |active| {
                handle
                    .update(&mut cx, |_, window, cx| {
                        window.active.set(active);
                        window.modifiers = window.platform_window.modifiers();
                        window.capslock = window.platform_window.capslock();
                        window
                            .activation_observers
                            .clone()
                            .retain(&(), |callback| callback(window, cx));

                        window.bounds_changed(cx);
                        // Riseonly 0087: an activation change notifies only the views that read
                        // the active status while drawing (animated images ask for frames only
                        // while the window is active, focus rings, typing dots). The full refresh
                        // of 0066 drew every cached view whenever the main window came back to
                        // front, e.g. after a call was accepted in the incoming call window
                        // (X1848: 196 views, layout 57 ms). One plain draw still runs so focus
                        // listeners see the new active status.
                        let readers = std::mem::take(&mut *window.activation_readers.borrow_mut());
                        for view in readers {
                            cx.notify(view);
                        }
                        if active && window.invalidator.not_drawing() {
                            window.invalidator.set_dirty(true);
                        }

                        SystemWindowTabController::update_last_active(cx, window.handle.id);
                    })
                    .log_err();
            }
        }));
        platform_window.on_hover_status_change(Box::new({
            let mut cx = cx.to_async();
            move |active| {
                handle
                    .update(&mut cx, |_, window, _| {
                        window.hovered.set(active);
                        window.refresh();
                    })
                    .log_err();
            }
        }));
        platform_window.on_input({
            let mut cx = cx.to_async();
            Box::new(move |event| {
                handle
                    .update(&mut cx, |_, window, cx| window.dispatch_event(event, cx))
                    .log_err()
                    .unwrap_or(DispatchEventResult::default())
            })
        });
        platform_window.on_hit_test_window_control({
            let mut cx = cx.to_async();
            Box::new(move || {
                handle
                    .update(&mut cx, |_, window, _cx| {
                        for (area, hitbox) in &window.rendered_frame.window_control_hitboxes {
                            if window.mouse_hit_test.ids.contains(&hitbox.id) {
                                return Some(*area);
                            }
                        }
                        None
                    })
                    .log_err()
                    .unwrap_or(None)
            })
        });
        platform_window.on_move_tab_to_new_window({
            let mut cx = cx.to_async();
            Box::new(move || {
                handle
                    .update(&mut cx, |_, _window, cx| {
                        SystemWindowTabController::move_tab_to_new_window(cx, handle.window_id());
                    })
                    .log_err();
            })
        });
        platform_window.on_merge_all_windows({
            let mut cx = cx.to_async();
            Box::new(move || {
                handle
                    .update(&mut cx, |_, _window, cx| {
                        SystemWindowTabController::merge_all_windows(cx, handle.window_id());
                    })
                    .log_err();
            })
        });
        platform_window.on_select_next_tab({
            let mut cx = cx.to_async();
            Box::new(move || {
                handle
                    .update(&mut cx, |_, _window, cx| {
                        SystemWindowTabController::select_next_tab(cx, handle.window_id());
                    })
                    .log_err();
            })
        });
        platform_window.on_select_previous_tab({
            let mut cx = cx.to_async();
            Box::new(move || {
                handle
                    .update(&mut cx, |_, _window, cx| {
                        SystemWindowTabController::select_previous_tab(cx, handle.window_id())
                    })
                    .log_err();
            })
        });
        platform_window.on_toggle_tab_bar({
            let mut cx = cx.to_async();
            Box::new(move || {
                handle
                    .update(&mut cx, |_, window, cx| {
                        let tab_bar_visible = window.platform_window.tab_bar_visible();
                        SystemWindowTabController::set_visible(cx, tab_bar_visible);
                    })
                    .log_err();
            })
        });

        if let Some(app_id) = app_id {
            platform_window.set_app_id(&app_id);
        }

        platform_window.map_window().unwrap();

        Ok(Window {
            handle,
            invalidator,
            removed: false,
            platform_window,
            display_id,
            is_resizable,
            is_minimizable,
            sprite_atlas,
            text_system,
            text_rendering_mode: cx.text_rendering_mode.clone(),
            rem_size: px(16.),
            rem_size_override_stack: SmallVec::new(),
            viewport_size: content_size,
            layout_engine: Some(TaffyLayoutEngine::new()),
            root: None,
            element_id_stack: SmallVec::default(),
            text_style_stack: Vec::new(),
            rendered_entity_stack: Vec::new(),
            element_offset_stack: Vec::new(),
            content_mask_stack: Vec::new(),
            paint_mask_stack: Vec::new(),
            hole_clip_boundary: false,
            surface_corner_clip_stack: Vec::new(),
            transformation_stack: Vec::new(),
            element_opacity: 1.0,
            glyph_swap_key: 0,
            image_swap_key: 0,
            requested_autoscroll: None,
            rendered_frame: Frame::new(DispatchTree::new(cx.keymap.clone(), cx.actions.clone())),
            next_frame: Frame::new(DispatchTree::new(cx.keymap.clone(), cx.actions.clone())),
            next_frame_callbacks,
            next_hitbox_id: HitboxId(0),
            hitbox_layer: 0,
            next_tooltip_id: TooltipId::default(),
            tooltip_bounds: None,
            dirty_views: FxHashSet::default(),
            direct_dirty: FxHashSet::default(),
            last_accessed: FxHashSet::default(),
            root_own: FxHashSet::default(),
            root_dirty: false,
            spliced: false,
            focus_listeners: SubscriberSet::new(),
            focus_lost_listeners: SubscriberSet::new(),
            default_prevented: true,
            mouse_position,
            mouse_hit_test: HitTest::default(),
            modifiers,
            capslock,
            scale_factor,
            zoom,
            bounds_observers: SubscriberSet::new(),
            appearance,
            appearance_observers: SubscriberSet::new(),
            button_layout_observers: SubscriberSet::new(),
            active,
            hovered,
            needs_present,
            input_rate_tracker,
            #[cfg(feature = "input-latency-histogram")]
            input_latency_tracker: InputLatencyTracker::new()?,
            last_input_modality: InputModality::Mouse,
            modality_readers: RefCell::new(FxHashSet::default()),
            hover_modality_readers: RefCell::new(FxHashSet::default()),
            mouse_inside_at_render: FxHashMap::default(),
            activation_readers: RefCell::new(FxHashSet::default()),
            refreshing: false,
            draw_count: 0,
            env_fingerprint: 0,
            env_trace: Vec::new(),
            hidden_depth: 0,
            hidden_dropped_deferred: 0,
            image_drops: 0,
            pending_image_drops: Vec::new(),
            activation_observers: SubscriberSet::new(),
            focus: None,
            focus_enabled: true,
            focus_generation: 0,
            pending_input: None,
            pending_modifier: ModifierState::default(),
            pending_input_observers: SubscriberSet::new(),
            prompt: None,
            client_inset: None,
            image_cache_stack: Vec::new(),
            captured_hitbox: None,
            #[cfg(any(feature = "inspector", debug_assertions))]
            inspector: None,
            a11y: A11y::new(
                a11y_active_flag,
                accessibility_force_disabled,
                initial_window_title,
            ),
        })
    }

    pub(crate) fn new_focus_listener(
        &self,
        value: AnyWindowFocusListener,
    ) -> (Subscription, impl FnOnce() + use<>) {
        self.focus_listeners.insert((), value)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(missing_docs)]
pub struct DispatchEventResult {
    pub propagate: bool,
    pub default_prevented: bool,
}

/// Indicates which region of the window is visible. Content falling outside of this mask will not be
/// rendered. Currently, only rectangular content masks are supported, but we give the mask its own type
/// to leave room to support more complex shapes in the future.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct ContentMask<P: Clone + Debug + Default + PartialEq> {
    /// The bounds
    pub bounds: Bounds<P>,
}

impl ContentMask<Pixels> {
    /// Scale the content mask's pixel units by the given scaling factor.
    pub fn scale(&self, factor: f32) -> ContentMask<ScaledPixels> {
        ContentMask {
            bounds: self.bounds.scale(factor),
        }
    }

    /// Intersect the content mask with the given content mask.
    pub fn intersect(&self, other: &Self) -> Self {
        let bounds = self.bounds.intersect(&other.bounds);
        ContentMask { bounds }
    }
}

impl Window {
    fn mark_view_dirty(&mut self, view_id: EntityId) {
        // Mark ancestor views as dirty. If already in the `dirty_views` set, then all its ancestors
        // should already be dirty.
        for view_id in self
            .rendered_frame
            .dispatch_tree
            .view_path_reversed(view_id)
        {
            if !self.dirty_views.insert(view_id) {
                break;
            }
        }
    }

    /// Registers a callback to be invoked when the window appearance changes.
    pub fn observe_window_appearance(
        &self,
        mut callback: impl FnMut(&mut Window, &mut App) + 'static,
    ) -> Subscription {
        let (subscription, activate) = self.appearance_observers.insert(
            (),
            Box::new(move |window, cx| {
                callback(window, cx);
                true
            }),
        );
        activate();
        subscription
    }

    /// Registers a callback to be invoked when the window button layout changes.
    pub fn observe_button_layout_changed(
        &self,
        mut callback: impl FnMut(&mut Window, &mut App) + 'static,
    ) -> Subscription {
        let (subscription, activate) = self.button_layout_observers.insert(
            (),
            Box::new(move |window, cx| {
                callback(window, cx);
                true
            }),
        );
        activate();
        subscription
    }

    /// Replaces the root entity of the window with a new one.
    pub fn replace_root<E>(
        &mut self,
        cx: &mut App,
        build_view: impl FnOnce(&mut Window, &mut Context<E>) -> E,
    ) -> Entity<E>
    where
        E: 'static + Render,
    {
        let view = cx.new(|cx| build_view(self, cx));
        self.root = Some(view.clone().into());
        self.refresh();
        view
    }

    /// Returns the root entity of the window, if it has one.
    pub fn root<E>(&self) -> Option<Option<Entity<E>>>
    where
        E: 'static + Render,
    {
        self.root
            .as_ref()
            .map(|view| view.clone().downcast::<E>().ok())
    }

    /// Obtain a handle to the window that belongs to this context.
    pub fn window_handle(&self) -> AnyWindowHandle {
        self.handle
    }

    /// Mark the window as dirty, scheduling it to be redrawn on the next frame.
    #[track_caller]
    pub fn refresh(&mut self) {
        if self.invalidator.not_drawing() {
            crate::profiler::note_dirty_source(std::panic::Location::caller());
            self.refreshing = true;
            self.invalidator.set_dirty(true);
        }
    }

    /// Riseonly 0095: whether anything invalidated the window since its last draw, the question
    /// the platform frame callback asks before it draws. A driver that draws a concealed window by
    /// hand asks it too, so it draws no frame that nothing asked for.
    pub fn needs_draw(&self) -> bool {
        self.invalidator.is_dirty()
    }

    /// Close this window.
    pub fn remove_window(&mut self) {
        self.removed = true;
    }

    /// Obtain the currently focused [`FocusHandle`]. If no elements are focused, returns `None`.
    pub fn focused(&self, cx: &App) -> Option<FocusHandle> {
        self.focus
            .and_then(|id| FocusHandle::for_id(id, &cx.focus_handles))
    }

    /// Move focus to the element associated with the given [`FocusHandle`].
    pub fn focus(&mut self, handle: &FocusHandle, cx: &mut App) {
        if !self.focus_enabled || self.focus == Some(handle.id) {
            return;
        }

        let previous = self.focus.replace(handle.id);
        self.focus_generation = self.focus_generation.wrapping_add(1);
        self.clear_pending_keystrokes();

        // Avoid re-entrant entity updates by deferring observer notifications to the end of the
        // current effect cycle, and only for this window.
        let window_handle = self.handle;
        cx.defer(move |cx| {
            window_handle
                .update(cx, |_, window, cx| {
                    window.pending_input_changed(cx);
                })
                .ok();
        });

        self.invalidate_focus_change(previous, Some(handle.id));
    }

    /// Riseonly 0040: a focus move dirties only the views on the old and the new focus path (the
    /// focused element, the views around it, their ancestors) instead of refreshing the window.
    /// Every cached view re-rendered on each focus move before: opening the media viewer over a
    /// chat drew the chat list, the chat and the hidden feed twice (X1278). A view that styles by
    /// a handle outside its own subtree must observe that handle's focus events itself.
    fn invalidate_focus_change(&mut self, previous: Option<FocusId>, next: Option<FocusId>) {
        if !self.invalidator.not_drawing() {
            return;
        }
        let tree = &self.rendered_frame.dispatch_tree;
        let mut views: SmallVec<[EntityId; 32]> = SmallVec::new();
        for focus_id in [previous, next].into_iter().flatten() {
            views.extend(tree.focus_view_path(focus_id));
        }
        self.invalidator.mark_views_dirty(views);
    }

    /// Remove focus from all elements within this context's window.
    pub fn blur(&mut self) {
        if !self.focus_enabled {
            return;
        }

        if self.focus.is_some() {
            self.focus_generation = self.focus_generation.wrapping_add(1);
        }
        let previous = self.focus.take();
        self.invalidate_focus_change(previous, None);
    }

    /// Blur the window and don't allow anything in it to be focused again.
    pub fn disable_focus(&mut self) {
        self.blur();
        self.focus_enabled = false;
    }

    /// Move focus to next tab stop.
    pub fn focus_next(&mut self, cx: &mut App) {
        if !self.focus_enabled {
            return;
        }

        if let Some(handle) = self.rendered_frame.tab_stops.next(self.focus.as_ref()) {
            self.focus(&handle, cx)
        }
    }

    /// Move focus to previous tab stop.
    pub fn focus_prev(&mut self, cx: &mut App) {
        if !self.focus_enabled {
            return;
        }

        if let Some(handle) = self.rendered_frame.tab_stops.prev(self.focus.as_ref()) {
            self.focus(&handle, cx)
        }
    }

    /// Accessor for the text system.
    pub fn text_system(&self) -> &Arc<WindowTextSystem> {
        &self.text_system
    }

    /// Riseonly 0036: fingerprint of the environment values around the element being prepainted.
    pub fn env_fingerprint(&self) -> u64 {
        self.env_fingerprint
    }

    /// Riseonly 0036: sets the environment fingerprint for the children prepainted next and
    /// returns the previous one, which the caller restores after them.
    pub fn set_env_fingerprint(&mut self, fingerprint: u64) -> u64 {
        std::mem::replace(&mut self.env_fingerprint, fingerprint)
    }

    /// Riseonly 0086: names the environment key the children prepainted next sit in, with the
    /// fingerprint of its own value, so a cached view that misses on env names the key that
    /// changed in the frame trace. Pair every push with [`Self::pop_env_trace`] after the children.
    pub fn push_env_trace(&mut self, key: &'static str, fingerprint: u64) {
        self.env_trace.push((key, fingerprint));
    }

    /// Riseonly 0086: leaves the scope [`Self::push_env_trace`] entered.
    pub fn pop_env_trace(&mut self) {
        self.env_trace.pop();
    }

    /// The current text style. Which is composed of all the style refinements provided to `with_text_style`.
    pub fn text_style(&self) -> TextStyle {
        let mut style = TextStyle::default();
        for refinement in &self.text_style_stack {
            style.refine(refinement);
        }
        style
    }

    /// Check if the platform window is maximized.
    ///
    /// On some platforms (namely Windows) this is different than the bounds being the size of the display
    pub fn is_maximized(&self) -> bool {
        self.platform_window.is_maximized()
    }

    /// request a certain window decoration (Wayland)
    pub fn request_decorations(&self, decorations: WindowDecorations) {
        self.platform_window.request_decorations(decorations);
    }

    /// Set the exclusive zone for a layer-shell surface: how much screen space it
    /// reserves so other surfaces avoid occluding it (e.g. a panel reserving space).
    /// Positive values reserve that distance from the anchored edge, 0 lets the
    /// surface be moved out of others' exclusive zones, and -1 ignores reserved
    /// space and may extend under other surfaces. (Wayland layer-shell windows only)
    pub fn set_exclusive_zone(&self, zone: Pixels) {
        self.platform_window.set_exclusive_zone(zone);
    }

    /// Set which anchored edge a layer-shell surface's exclusive zone applies to.
    /// This is only needed to disambiguate a corner-anchored surface; otherwise the
    /// edge is deduced from the anchor. The edge must be a single edge the surface
    /// is anchored to, or it is ignored. (Wayland layer-shell windows only)
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    pub fn set_exclusive_edge(&self, edge: crate::layer_shell::Anchor) {
        self.platform_window.set_exclusive_edge(edge);
    }

    /// Start an interactive window resize operation if this window is resizable.
    pub fn start_window_resize(&self, edge: ResizeEdge) {
        if self.is_resizable {
            self.platform_window.start_window_resize(edge);
        }
    }

    /// Linux (wayland) only: Set the window's input region, the area that receives pointer
    /// and touch input. Events outside it pass through to whatever is below the window.
    ///
    /// - `Some(rects)` restricts input to the union of `rects`, in window coordinates.
    /// - `Some(&[])` is an empty region, so the window receives no pointer or touch input.
    /// - `None` resets the region to the default, so the whole window receives input again.
    pub fn set_input_region(&self, region: Option<&[Bounds<Pixels>]>) {
        self.platform_window.set_input_region(region);
    }

    /// Return the `WindowBounds` to indicate that how a window should be opened
    /// after it has been closed
    pub fn window_bounds(&self) -> WindowBounds {
        self.platform_window.window_bounds()
    }

    /// Return the `WindowBounds` excluding insets (Wayland and X11)
    pub fn inner_window_bounds(&self) -> WindowBounds {
        self.platform_window.inner_window_bounds()
    }

    /// Dispatch the given action on the currently focused element.
    pub fn dispatch_action(&mut self, action: Box<dyn Action>, cx: &mut App) {
        let focus_id = self.focused(cx).map(|handle| handle.id);

        let window = self.handle;
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    let node_id = window.focus_node_id_in_rendered_frame(focus_id);
                    window.dispatch_action_on_node(node_id, action.as_ref(), cx);
                })
                .log_err();
        })
    }

    pub(crate) fn dispatch_keystroke_observers(
        &mut self,
        event: &dyn Any,
        action: Option<Box<dyn Action>>,
        context_stack: Vec<KeyContext>,
        cx: &mut App,
    ) {
        let Some(key_down_event) = event.downcast_ref::<KeyDownEvent>() else {
            return;
        };

        cx.keystroke_observers.clone().retain(&(), move |callback| {
            (callback)(
                &KeystrokeEvent {
                    keystroke: key_down_event.keystroke.clone(),
                    action: action.as_ref().map(|action| action.boxed_clone()),
                    context_stack: context_stack.clone(),
                },
                self,
                cx,
            )
        });
    }

    pub(crate) fn dispatch_keystroke_interceptors(
        &mut self,
        event: &dyn Any,
        context_stack: Vec<KeyContext>,
        cx: &mut App,
    ) {
        let Some(key_down_event) = event.downcast_ref::<KeyDownEvent>() else {
            return;
        };

        cx.keystroke_interceptors
            .clone()
            .retain(&(), move |callback| {
                (callback)(
                    &KeystrokeEvent {
                        keystroke: key_down_event.keystroke.clone(),
                        action: None,
                        context_stack: context_stack.clone(),
                    },
                    self,
                    cx,
                )
            });
    }

    /// Schedules the given function to be run at the end of the current effect cycle, allowing entities
    /// that are currently on the stack to be returned to the app.
    pub fn defer(&self, cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
        let handle = self.handle;
        cx.defer(move |cx| {
            handle.update(cx, |_, window, cx| f(window, cx)).ok();
        });
    }

    /// Subscribe to events emitted by a entity.
    /// The entity to which you're subscribing must implement the [`EventEmitter`] trait.
    /// The callback will be invoked a handle to the emitting entity, the event, and a window context for the current window.
    pub fn observe<T: 'static>(
        &mut self,
        observed: &Entity<T>,
        cx: &mut App,
        mut on_notify: impl FnMut(Entity<T>, &mut Window, &mut App) + 'static,
    ) -> Subscription {
        let entity_id = observed.entity_id();
        let observed = observed.downgrade();
        let window_handle = self.handle;
        cx.new_observer(
            entity_id,
            Box::new(move |cx| {
                window_handle
                    .update(cx, |_, window, cx| {
                        if let Some(handle) = observed.upgrade() {
                            on_notify(handle, window, cx);
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false)
            }),
        )
    }

    /// Subscribe to events emitted by a entity.
    /// The entity to which you're subscribing must implement the [`EventEmitter`] trait.
    /// The callback will be invoked a handle to the emitting entity, the event, and a window context for the current window.
    pub fn subscribe<Emitter, Evt>(
        &mut self,
        entity: &Entity<Emitter>,
        cx: &mut App,
        mut on_event: impl FnMut(Entity<Emitter>, &Evt, &mut Window, &mut App) + 'static,
    ) -> Subscription
    where
        Emitter: EventEmitter<Evt>,
        Evt: 'static,
    {
        let entity_id = entity.entity_id();
        let handle = entity.downgrade();
        let window_handle = self.handle;
        cx.new_subscription(
            entity_id,
            (
                TypeId::of::<Evt>(),
                Box::new(move |event, cx| {
                    window_handle
                        .update(cx, |_, window, cx| {
                            if let Some(entity) = handle.upgrade() {
                                let event = event.downcast_ref().expect("invalid event type");
                                on_event(entity, event, window, cx);
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false)
                }),
            ),
        )
    }

    /// Register a callback to be invoked when the given `Entity` is released.
    pub fn observe_release<T>(
        &self,
        entity: &Entity<T>,
        cx: &mut App,
        mut on_release: impl FnOnce(&mut T, &mut Window, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let entity_id = entity.entity_id();
        let window_handle = self.handle;
        let (subscription, activate) = cx.release_listeners.insert(
            entity_id,
            Box::new(move |entity, cx| {
                let entity = entity.downcast_mut().expect("invalid entity type");
                let _ = window_handle.update(cx, |_, window, cx| on_release(entity, window, cx));
            }),
        );
        activate();
        subscription
    }

    /// Creates an [`AsyncWindowContext`], which has a static lifetime and can be held across
    /// await points in async code.
    pub fn to_async(&self, cx: &App) -> AsyncWindowContext {
        AsyncWindowContext::new_context(cx.to_async(), self.handle)
    }

    /// Schedule the given closure to be run directly after the current frame is rendered.
    pub fn on_next_frame(&self, callback: impl FnOnce(&mut Window, &mut App) + 'static) {
        RefCell::borrow_mut(&self.next_frame_callbacks).push(Box::new(callback));
    }

    /// Schedule a frame to be drawn on the next animation frame.
    ///
    /// This is useful for elements that need to animate continuously, such as a video player or an animated GIF.
    /// It will cause the window to redraw on the next frame, even if no other changes have occurred.
    ///
    /// If called from within a view, it will notify that view on the next frame. Otherwise, it will refresh the entire window.
    ///
    /// Callers driving purely decorative animations (spinners, pulses, and the
    /// like) should prefer [`AnimationExt::with_animation`](crate::AnimationExt::with_animation),
    /// which automatically respects [`App::reduce_motion`]. When using this
    /// method directly for decorative motion, check [`App::reduce_motion`]
    /// and skip the frame request when it is set.
    #[track_caller]
    pub fn request_animation_frame(&self) {
        // Riseonly 0051: the delivered notify is counted once, under the requester's call site,
        // instead of under this file.
        let origin = std::panic::Location::caller();
        let entity = self.current_view();
        self.on_next_frame(move |_, cx| crate::profiler::with_dirty_origin(origin, || cx.notify(entity)));
    }

    /// Riseonly 0071: the number of the frame being drawn, raised at the start of every draw. An
    /// element prepainted under a layer that is drawn later in the same frame compares it with the
    /// frame the layer last prepainted in, to know whether it sits under or inside that layer.
    pub fn draw_count(&self) -> u64 {
        self.draw_count
    }

    /// Runs all callbacks scheduled via [`Self::on_next_frame`], returning how many ran.
    ///
    /// Tests have no platform frame loop, so this simulates the delivery of the
    /// next frame.
    #[cfg(any(test, feature = "test-support"))]
    pub fn simulate_next_frame(&mut self, cx: &mut App) -> usize {
        let callbacks = self.next_frame_callbacks.take();
        let count = callbacks.len();
        for callback in callbacks {
            callback(self, cx);
        }
        count
    }

    /// Spawn the future returned by the given closure on the application thread pool.
    /// The closure is provided a handle to the current window and an `AsyncWindowContext` for
    /// use within your future.
    #[track_caller]
    pub fn spawn<AsyncFn, R>(&self, cx: &App, f: AsyncFn) -> Task<R>
    where
        R: 'static,
        AsyncFn: AsyncFnOnce(&mut AsyncWindowContext) -> R + 'static,
    {
        let handle = self.handle;
        cx.spawn(async move |app| {
            let mut async_window_cx = AsyncWindowContext::new_context(app.clone(), handle);
            f(&mut async_window_cx).await
        })
    }

    /// Spawn the future returned by the given closure on the application thread
    /// pool, with the given priority. The closure is provided a handle to the
    /// current window and an `AsyncWindowContext` for use within your future.
    #[track_caller]
    pub fn spawn_with_priority<AsyncFn, R>(
        &self,
        priority: Priority,
        cx: &App,
        f: AsyncFn,
    ) -> Task<R>
    where
        R: 'static,
        AsyncFn: AsyncFnOnce(&mut AsyncWindowContext) -> R + 'static,
    {
        let handle = self.handle;
        cx.spawn_with_priority(priority, async move |app| {
            let mut async_window_cx = AsyncWindowContext::new_context(app.clone(), handle);
            f(&mut async_window_cx).await
        })
    }

    /// Notify the window that its bounds have changed.
    ///
    /// This updates internal state like `viewport_size` and `scale_factor` from
    /// the platform window, then notifies observers. Normally called automatically
    /// by the platform's resize callback, but exposed publicly for test infrastructure.
    pub fn bounds_changed(&mut self, cx: &mut App) {
        let scale_factor = self.platform_window.scale_factor() * self.zoom;
        let viewport_size = unzoom_size(self.platform_window.content_size(), self.zoom);
        let display_id = self.platform_window.display().map(|display| display.id());
        // Riseonly 0066: a move that keeps the size, the scale and the display draws nothing new,
        // yet the refresh rendered every cached view again on each move event (X1553). Bounds
        // observers still run.
        let reshaped = scale_factor != self.scale_factor
            || viewport_size != self.viewport_size
            || display_id != self.display_id;
        self.scale_factor = scale_factor;
        self.viewport_size = viewport_size;
        self.display_id = display_id;
        self.mouse_position = unzoom_point(self.platform_window.mouse_position(), self.zoom);

        if reshaped {
            self.refresh();
        }

        self.bounds_observers
            .clone()
            .retain(&(), |callback| callback(self, cx));
    }

    /// Returns the bounds of the current window in the global coordinate space, which could span across multiple displays.
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.platform_window.bounds()
    }

    /// Renders the current frame's scene to a texture and returns the pixel data as an RGBA image.
    /// This does not present the frame to screen - useful for visual testing where we want
    /// to capture what would be rendered without displaying it or requiring the window to be visible.
    #[cfg(any(test, feature = "test-support"))]
    pub fn render_to_image(&self) -> anyhow::Result<image::RgbaImage> {
        self.platform_window
            .render_to_image(&self.rendered_frame.scene)
    }

    /// Set the content size of the window.
    pub fn resize(&mut self, size: Size<Pixels>) {
        self.platform_window.resize(size);
    }

    /// Returns whether or not the window is currently fullscreen
    pub fn is_fullscreen(&self) -> bool {
        self.platform_window.is_fullscreen()
    }

    pub(crate) fn appearance_changed(&mut self, cx: &mut App) {
        self.appearance = self.platform_window.appearance();

        self.appearance_observers
            .clone()
            .retain(&(), |callback| callback(self, cx));
    }

    pub(crate) fn button_layout_changed(&mut self, cx: &mut App) {
        self.button_layout_observers
            .clone()
            .retain(&(), |callback| callback(self, cx));
    }

    /// Returns the appearance of the current window.
    pub fn appearance(&self) -> WindowAppearance {
        self.appearance
    }

    /// Returns the size of the drawable area within the window.
    pub fn viewport_size(&self) -> Size<Pixels> {
        self.viewport_size
    }

    /// Returns whether this window is focused by the operating system (receiving key events).
    pub fn is_window_active(&self) -> bool {
        // Riseonly 0087: a view that draws by the active status is notified when it changes.
        if let Some(view) = self.rendered_entity_stack.last() {
            self.activation_readers.borrow_mut().insert(*view);
        }
        self.active.get()
    }

    /// Returns whether this window is considered to be the window
    /// that currently owns the mouse cursor.
    /// On mac, this is equivalent to `is_window_active`.
    pub fn is_window_hovered(&self) -> bool {
        if cfg!(any(
            target_os = "windows",
            target_os = "linux",
            target_os = "freebsd"
        )) {
            self.hovered.get()
        } else {
            self.is_window_active()
        }
    }

    /// Toggle zoom on the window.
    pub fn zoom_window(&self) {
        self.platform_window.zoom();
    }

    /// Opens the native title bar context menu, useful when implementing client side decorations (Wayland and X11)
    pub fn show_window_menu(&self, position: Point<Pixels>) {
        self.platform_window
            .show_window_menu(point(position.x * self.zoom, position.y * self.zoom))
    }

    /// Handle window movement for Linux and macOS.
    /// Tells the compositor to take control of window movement (Wayland and X11)
    ///
    /// Events may not be received during a move operation.
    pub fn start_window_move(&self) {
        self.platform_window.start_window_move()
    }

    /// When using client side decorations, set this to the width of the invisible decorations (Wayland and X11)
    pub fn set_client_inset(&mut self, inset: Pixels) {
        self.client_inset = Some(inset);
        self.platform_window.set_client_inset(inset * self.zoom);
    }

    /// Returns the client_inset value by [`Self::set_client_inset`].
    pub fn client_inset(&self) -> Option<Pixels> {
        self.client_inset
    }

    /// Returns whether the title bar window controls need to be rendered by the application (Wayland and X11)
    pub fn window_decorations(&self) -> Decorations {
        self.platform_window.window_decorations()
    }

    /// Returns whether this window is resizable.
    pub fn is_resizable(&self) -> bool {
        self.is_resizable
    }

    /// Returns whether this window is minimizable.
    pub fn is_minimizable(&self) -> bool {
        self.is_minimizable
    }

    /// Returns the controls supported by the platform.
    pub fn window_controls(&self) -> WindowControls {
        self.platform_window.window_controls()
    }

    /// Updates the window's title at the platform level.
    pub fn set_window_title(&mut self, title: &str) {
        self.platform_window.set_title(title);
        self.a11y.set_window_title(title.to_string());
    }

    /// Sets the position of the macOS traffic light buttons.
    #[cfg(target_os = "macos")]
    pub fn set_traffic_light_position(&self, position: Point<Pixels>) {
        self.platform_window.set_traffic_light_position(position);
    }

    /// Sets the application identifier.
    pub fn set_app_id(&mut self, app_id: &str) {
        self.platform_window.set_app_id(app_id);
    }

    /// Sets the window background appearance.
    pub fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance) {
        self.platform_window
            .set_background_appearance(background_appearance);
    }

    /// Mark the window as dirty at the platform level.
    pub fn set_window_edited(&mut self, edited: bool) {
        self.platform_window.set_edited(edited);
    }

    /// Set the path of the file this window represents.
    /// On macOS, this sets the window's accessibility document property (AXDocument).
    pub fn set_document_path(&self, path: Option<&std::path::Path>) {
        self.platform_window.set_document_path(path);
    }

    /// Determine the display on which the window is visible.
    pub fn display(&self, cx: &App) -> Option<Rc<dyn PlatformDisplay>> {
        cx.platform
            .displays()
            .into_iter()
            .find(|display| Some(display.id()) == self.display_id)
    }

    /// Show the platform character palette.
    pub fn show_character_palette(&self) {
        self.platform_window.show_character_palette();
    }

    /// The scale factor of the display associated with the window. For example, it could
    /// return 2.0 for a "retina" display, indicating that each logical pixel should actually
    /// be rendered as two pixels on screen.
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Riseonly 0124: logical pixels of this window per platform point, see [`App::set_window_zoom`].
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub(crate) fn set_zoom(&mut self, zoom: f32, cx: &mut App) {
        if self.zoom == zoom {
            return;
        }
        self.zoom = zoom;
        self.bounds_changed(cx);
    }

    /// Window content bounds in platform points, what the platform wants back for an ime panel.
    pub fn zoomed_bounds(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        Bounds {
            origin: point(bounds.origin.x * self.zoom, bounds.origin.y * self.zoom),
            size: size(bounds.size.width * self.zoom, bounds.size.height * self.zoom),
        }
    }

    /// A platform point inside the window in this window's logical pixels.
    pub fn unzoomed_point(&self, position: Point<Pixels>) -> Point<Pixels> {
        unzoom_point(position, self.zoom)
    }

    /// Overrides the display scale factor for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor;
        self.refresh();
    }

    /// The size of an em for the base font of the application. Adjusting this value allows the
    /// UI to scale, just like zooming a web page.
    pub fn rem_size(&self) -> Pixels {
        self.rem_size_override_stack
            .last()
            .copied()
            .unwrap_or(self.rem_size)
    }

    /// Sets the size of an em for the base font of the application. Adjusting this value allows the
    /// UI to scale, just like zooming a web page.
    pub fn set_rem_size(&mut self, rem_size: impl Into<Pixels>) {
        self.rem_size = rem_size.into();
    }

    /// Acquire a globally unique identifier for the given ElementId.
    /// Only valid for the duration of the provided closure.
    pub fn with_global_id<R>(
        &mut self,
        element_id: ElementId,
        f: impl FnOnce(&GlobalElementId, &mut Self) -> R,
    ) -> R {
        self.with_id(element_id, |this| {
            let global_id = GlobalElementId(Arc::from(&*this.element_id_stack));

            f(&global_id, this)
        })
    }

    /// Calls the provided closure with the element ID pushed on the stack.
    #[inline]
    pub fn with_id<R>(
        &mut self,
        element_id: impl Into<ElementId>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.element_id_stack.push(element_id.into());
        let result = f(self);
        self.element_id_stack.pop();
        result
    }

    /// Executes the provided function with the specified rem size.
    ///
    /// This method must only be called as part of element drawing.
    // This function is called in a highly recursive manner in editor
    // prepainting, make sure its inlined to reduce the stack burden
    #[inline]
    pub fn with_rem_size<F, R>(&mut self, rem_size: Option<impl Into<Pixels>>, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.invalidator.debug_assert_paint_or_prepaint();

        if let Some(rem_size) = rem_size {
            self.rem_size_override_stack.push(rem_size.into());
            let result = f(self);
            self.rem_size_override_stack.pop();
            result
        } else {
            f(self)
        }
    }

    /// The line height associated with the current text style.
    pub fn line_height(&self) -> Pixels {
        self.text_style().line_height_in_pixels(self.rem_size())
    }

    /// Rounds a logical value to the nearest device pixel.
    #[inline]
    pub fn pixel_snap(&self, value: Pixels) -> Pixels {
        px(round_to_device_pixel(value.0, self.scale_factor()) / self.scale_factor())
    }

    /// f64 variant of [`Self::pixel_snap`].
    #[inline]
    pub fn pixel_snap_f64(&self, value: f64) -> f64 {
        let scale_factor = f64::from(self.scale_factor());
        round_half_toward_zero_f64(value * scale_factor) / scale_factor
    }

    /// Snaps a bounds' origin and size to the nearest device pixel.
    #[inline]
    pub fn pixel_snap_bounds(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        bounds.map(|c| self.pixel_snap(c))
    }

    /// Snaps a point's coordinates to the nearest device pixel.
    #[inline]
    pub fn pixel_snap_point(&self, position: Point<Pixels>) -> Point<Pixels> {
        position.map(|c| self.pixel_snap(c))
    }

    #[inline]
    fn snap_bounds(&self, bounds: Bounds<Pixels>) -> Bounds<ScaledPixels> {
        let scale_factor = self.scale_factor();
        let left = round_to_device_pixel(bounds.left().0, scale_factor);
        let top = round_to_device_pixel(bounds.top().0, scale_factor);
        let right = round_to_device_pixel(bounds.right().0, scale_factor).max(left);
        let bottom = round_to_device_pixel(bounds.bottom().0, scale_factor).max(top);
        Bounds::from_corners(
            point(ScaledPixels(left), ScaledPixels(top)),
            point(ScaledPixels(right), ScaledPixels(bottom)),
        )
    }

    /// Rounds half-to-zero but clamps any non-zero input up to 1 dp so thin strokes do not disappear.
    #[inline]
    fn snap_stroke(&self, value: Pixels) -> ScaledPixels {
        ScaledPixels(round_stroke_to_device_pixel(value.0, self.scale_factor()))
    }

    #[inline]
    fn snap_border_widths(&self, edges: Edges<Pixels>) -> Edges<ScaledPixels> {
        edges.map(|e| self.snap_stroke(*e))
    }

    /// Floors the near edge and ceils the far edge, producing a strict superset of the raw region.
    #[inline]
    fn cover_bounds(&self, bounds: Bounds<Pixels>) -> Bounds<ScaledPixels> {
        let scale_factor = self.scale_factor();
        let left = floor_to_device_pixel(bounds.left().0, scale_factor);
        let top = floor_to_device_pixel(bounds.top().0, scale_factor);
        let right = ceil_to_device_pixel(bounds.right().0, scale_factor).max(left);
        let bottom = ceil_to_device_pixel(bounds.bottom().0, scale_factor).max(top);
        Bounds::from_corners(
            point(ScaledPixels(left), ScaledPixels(top)),
            point(ScaledPixels(right), ScaledPixels(bottom)),
        )
    }

    #[inline]
    fn snapped_content_mask(&self) -> ContentMask<ScaledPixels> {
        ContentMask {
            bounds: self.cover_bounds(self.paint_mask().bounds),
        }
    }

    /// Surfaces painted by `f` are cut to `bounds` with `corner_radii`, the rounded box a
    /// clipping container draws its own background with. The content mask stays rectangular for
    /// every other primitive; a surface has no other way to follow a rounded clip.
    pub fn with_surface_corner_clip<R>(
        &mut self,
        bounds: Bounds<Pixels>,
        corner_radii: Corners<Pixels>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.surface_corner_clip_stack.push((bounds, corner_radii));
        let result = f(self);
        self.surface_corner_clip_stack.pop();
        result
    }

    fn surface_corner_clip(&self) -> (Bounds<ScaledPixels>, Corners<ScaledPixels>) {
        match self.surface_corner_clip_stack.last() {
            Some((bounds, corner_radii)) => (
                self.snap_bounds(self.surface_bounds_under_transformation(*bounds)),
                corner_radii.scale(self.scale_factor()),
            ),
            None => (Bounds::default(), Corners::default()),
        }
    }

    /// Call to prevent the default action of an event. Currently only used to prevent
    /// parent elements from becoming focused on mouse down.
    pub fn prevent_default(&mut self) {
        self.default_prevented = true;
    }

    /// Obtain whether default has been prevented for the event currently being dispatched.
    pub fn default_prevented(&self) -> bool {
        self.default_prevented
    }

    /// Determine whether the given action is available along the dispatch path to the currently focused element.
    pub fn is_action_available(&self, action: &dyn Action, cx: &App) -> bool {
        let node_id =
            self.focus_node_id_in_rendered_frame(self.focused(cx).map(|handle| handle.id));
        self.rendered_frame
            .dispatch_tree
            .is_action_available(action, node_id)
    }

    /// Determine whether the given action is available along the dispatch path to the given focus_handle.
    pub fn is_action_available_in(&self, action: &dyn Action, focus_handle: &FocusHandle) -> bool {
        let node_id = self.focus_node_id_in_rendered_frame(Some(focus_handle.id));
        self.rendered_frame
            .dispatch_tree
            .is_action_available(action, node_id)
    }

    /// The position of the mouse relative to the window.
    pub fn mouse_position(&self) -> Point<Pixels> {
        self.mouse_position
    }

    /// Captures the pointer for the given hitbox. While captured, all mouse move and mouse up
    /// events will be routed to listeners that check this hitbox's `is_hovered` status,
    /// regardless of actual hit testing. This enables drag operations that continue
    /// even when the pointer moves outside the element's bounds.
    ///
    /// The capture is automatically released on mouse up.
    pub fn capture_pointer(&mut self, hitbox_id: HitboxId) {
        self.captured_hitbox = Some(hitbox_id);
    }

    /// Releases any active pointer capture.
    pub fn release_pointer(&mut self) {
        self.captured_hitbox = None;
    }

    /// Returns the hitbox that has captured the pointer, if any.
    pub fn captured_hitbox(&self) -> Option<HitboxId> {
        self.captured_hitbox
    }

    /// Riseonly 0055: the frontmost hitbox that blocks the mouse at `position` in the frame on
    /// screen, in the hit test's paint order, with its paint layer and how many hitboxes lie in
    /// front of it. A diagnostic for a column that stops answering (X1556): it names the size and
    /// place of the layer that swallows the pointer. Hidden hitboxes are skipped as in the hit test.
    pub fn mouse_blocker_at(
        &self,
        position: Point<Pixels>,
    ) -> Option<(Bounds<Pixels>, HitboxBehavior, usize, usize)> {
        let hitboxes = &self.rendered_frame.hitboxes;
        let mut order: Vec<usize> = (0..hitboxes.len()).collect();
        order.sort_by_key(|ix| hitboxes[*ix].layer);
        let mut in_front = 0;
        for ix in order.into_iter().rev() {
            let hitbox = &hitboxes[ix];
            if hitbox.hidden {
                continue;
            }
            let bounds = hitbox.bounds.intersect(&hitbox.content_mask.bounds);
            if !bounds.contains(&position) {
                continue;
            }
            if hitbox.behavior != HitboxBehavior::Normal {
                return Some((bounds, hitbox.behavior, hitbox.layer, in_front));
            }
            in_front += 1;
        }
        None
    }

    /// The current state of the keyboard's modifiers
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Returns true if the last input event was keyboard-based (key press, tab navigation, etc.)
    /// This is used for focus-visible styling to show focus indicators only for keyboard navigation.
    pub fn last_input_was_keyboard(&self) -> bool {
        if let Some(view) = self.rendered_entity_stack.last() {
            self.modality_readers.borrow_mut().insert(*view);
        }
        self.last_input_modality == InputModality::Keyboard
    }

    /// The current state of the keyboard's capslock
    pub fn capslock(&self) -> Capslock {
        self.capslock
    }

    fn complete_frame(&self) {
        self.platform_window.completed_frame();
    }

    /// Produces a new frame and assigns it to `rendered_frame`. To actually show
    /// the contents of the new [`Scene`], use [`Self::present`].
    #[profiling::function]
    pub fn draw(&mut self, cx: &mut App) -> ArenaClearNeeded {
        self.draw_count += 1;
        self.flush_image_drops();
        // Drain unconditionally so a stale first-invalidation timestamp can't
        // leak into a later frame across enable/disable of frame tracing.
        let frame_dirty = self.invalidator.take_frame_dirty();
        let draw_started_at = profiler::frame_trace_enabled().then(Instant::now);
        let draw_usage_start = draw_started_at.map(|_| profiler::ThreadUsage::now());
        // Drop marks a frame drawn while tracing was being switched on may have left.
        profiler::take_frame_phases();

        // Set up the per-App arena for element allocation during this draw.
        // This ensures that multiple test Apps have isolated arenas.
        let arena_scope = ElementArenaScope::enter(&cx.element_arena);

        self.invalidate_entities();
        cx.entities.clear_accessed();
        cx.top_nested_reads.clear();
        debug_assert!(self.rendered_entity_stack.is_empty());
        self.invalidator.set_dirty(false);
        self.requested_autoscroll = None;

        // Restore the previously-used input handler.
        // Place it back into a None slot (left by a previous .take()) so that
        // cached paint_range indices in reuse_paint find the handler at the
        // expected position.
        if let Some(input_handler) = self.platform_window.take_input_handler() {
            if let Some(slot) = self
                .rendered_frame
                .input_handlers
                .iter_mut()
                .rev()
                .find(|h| h.is_none())
            {
                *slot = Some(input_handler);
            } else {
                self.rendered_frame.input_handlers.push(Some(input_handler));
            }
        }
        if !cx.mode.skip_drawing() {
            self.draw_roots(cx);
        }
        self.dirty_views.clear();
        self.direct_dirty.clear();
        self.root_dirty = false;
        profiler::clear_root_dirty();
        self.next_frame.window_active = self.active.get();

        // Register requested input handler with the platform window.
        // Use .take() instead of .pop() to preserve Vec length, so that cached
        // paint_range indices remain valid for reuse_paint on the next frame.
        // Search backwards to find the last Some entry, since reuse_paint may
        // have copied None slots from the previous frame. (Fixes #50456)
        if let Some(input_handler) = self
            .next_frame
            .input_handlers
            .iter_mut()
            .rev()
            .find_map(|h| h.take())
        {
            self.platform_window.set_input_handler(input_handler);
        }

        self.layout_engine.as_mut().unwrap().clear();
        self.text_system().finish_frame();
        self.next_frame.finish(&mut self.rendered_frame);

        self.invalidator.set_phase(DrawPhase::Focus);
        let previous_focus_path = self.rendered_frame.focus_path();
        let previous_window_active = self.rendered_frame.window_active;
        mem::swap(&mut self.rendered_frame, &mut self.next_frame);
        self.next_frame.clear();
        self.retain_live_hover_modality_readers();
        self.recheck_moved_hover(cx);
        let current_focus_path = self.rendered_frame.focus_path();
        let current_window_active = self.rendered_frame.window_active;
        let mut focus_before_listeners = self.focus;

        if previous_focus_path != current_focus_path
            || previous_window_active != current_window_active
        {
            if !previous_focus_path.is_empty() && current_focus_path.is_empty() {
                self.focus_lost_listeners
                    .clone()
                    .retain(&(), |listener| listener(self, cx));
                // The focus-lost fallback (e.g. a workspace refocusing itself) may target
                // an element that isn't part of the element tree, in which case scheduling
                // a redraw below would dispatch focus-lost again, looping forever. Only
                // track focus movement caused by the focus listeners.
                focus_before_listeners = self.focus;
            }

            let event = WindowFocusEvent {
                previous_focus_path: if previous_window_active {
                    previous_focus_path
                } else {
                    Default::default()
                },
                current_focus_path: if current_window_active {
                    current_focus_path
                } else {
                    Default::default()
                },
            };
            self.focus_listeners
                .clone()
                .retain(&(), |listener| listener(&event, self, cx));
        }

        debug_assert!(self.rendered_entity_stack.is_empty());
        self.record_entities_accessed(cx);
        self.reset_cursor_style(cx);
        self.refreshing = false;
        self.invalidator.set_phase(DrawPhase::None);
        // Focus listeners may move focus (e.g. a dock forwarding focus to its active
        // panel). `Window::focus` suppresses `refresh` while a draw is in progress, so
        // schedule another frame here to render the new focus state and dispatch the
        // resulting focus events.
        if self.focus != focus_before_listeners {
            self.invalidate_focus_change(focus_before_listeners, self.focus);
        }
        self.needs_present.set(true);

        if let Some(draw_start) = draw_started_at {
            let phases = profiler::take_frame_phases();
            let (thread_cpu, page_faults) = match draw_usage_start {
                Some(start) => profiler::ThreadUsage::now().since(start),
                None => (None, None),
            };
            profiler::record_frame_timing(profiler::FrameTiming {
                window_id: self.handle.window_id(),
                dirty_at: frame_dirty.dirty_at,
                invalidations: frame_dirty.invalidations,
                draw_start,
                draw_end: Instant::now(),
                paint_start: phases.paint_start,
                paint_end: phases.paint_end,
                rendered_views: phases.rendered_views,
                shaped_lines: phases.shaped_lines,
                shape_time: phases.shape_time,
                resolved_fonts: phases.resolved_fonts,
                font_resolve_time: phases.font_resolve_time,
                splice: phases.splice.unwrap_or_default(),
                thread_cpu,
                page_faults,
            });
        }

        // Exit the scope to obtain the arena-clear token this draw owes; the
        // scope's teardown itself happens in `ElementArenaScope::drop`.
        arena_scope.exit(&cx.element_arena)
    }

    fn record_entities_accessed(&mut self, cx: &mut App) {
        let mut entities_ref = cx.entities.accessed_entities.get_mut();
        let direct = mem::take(entities_ref.deref_mut());
        // Riseonly 0088: the frame reads what it read itself and what its cached views read; a
        // spliced frame read only its holes, the copied rest still depends on what the frame it
        // was copied from read, the root included.
        let mut entities = direct.clone();
        entities.extend(cx.top_nested_reads.drain());
        if mem::take(&mut self.spliced) {
            entities.extend(self.last_accessed.iter().copied());
            self.root_own.extend(direct);
        } else {
            self.root_own = direct;
        }
        self.last_accessed.clone_from(&entities);
        let handle = self.handle;
        cx.record_entities_accessed(
            handle,
            // Try moving window invalidator into the Window
            self.invalidator.clone(),
            &entities,
        );
        let mut entities_ref = cx.entities.accessed_entities.get_mut();
        mem::swap(&mut entities, entities_ref.deref_mut());
    }

    fn invalidate_entities(&mut self) {
        let mut views = self.invalidator.take_views();
        let mut readers = Vec::new();
        for entity in views.drain() {
            // Riseonly 0088: the root reads it, so only a full draw shows it.
            if self.root_own.contains(&entity) {
                // why: Riseonly 0103, the splice column names the first root read entity
                // and its notify site ("root:<type>@file:line") instead of a bare "root".
                if !self.root_dirty {
                    profiler::note_root_dirty(entity);
                }
                self.root_dirty = true;
            }
            self.mark_view_dirty(entity);
            // Riseonly 0088: a notified store is no view; only views drawn last frame count.
            if self.rendered_frame.dispatch_tree.view_path_reversed(entity).next().is_some() {
                self.direct_dirty.insert(entity);
            }
            // A cached view replays until its own id is dirty, so an entity it only read (a
            // store, a keyed state) would never reach it: its readers go dirty with it.
            let start = readers.len();
            readers.extend(
                self.rendered_frame
                    .cached_views
                    .iter()
                    .filter(|(view_id, accessed)| *view_id != entity && accessed.contains(&entity))
                    .map(|(view_id, _)| (*view_id, entity)),
            );
            // Riseonly 0088: a reader that holds the entity only through a cached view inside it
            // that reads it too renders anyway (dirty_views), but a spliced frame draws only that
            // inner one, so it is no hole of its own.
            for index in start..readers.len() {
                let (view_id, _) = readers[index];
                let reads_itself = self
                    .rendered_frame
                    .own_reads
                    .get(&view_id)
                    .is_none_or(|own| own.contains(&entity));
                let inner_reader = !reads_itself
                    && readers[start..].iter().any(|(inner, _)| {
                        *inner != view_id
                            && self
                                .rendered_frame
                                .dispatch_tree
                                .view_path_reversed(*inner)
                                .skip(1)
                                .any(|ancestor| ancestor == view_id)
                    });
                if !inner_reader {
                    self.direct_dirty.insert(view_id);
                }
            }
        }
        for (view_id, _) in readers {
            self.mark_view_dirty(view_id);
        }
        self.invalidator.replace_views(views);
    }

    #[profiling::function]
    fn present(&mut self) {
        self.platform_window.draw(&self.rendered_frame.scene);
        #[cfg(feature = "input-latency-histogram")]
        self.input_latency_tracker.record_frame_presented();
        self.needs_present.set(false);
        profiling::finish_frame!();
    }

    /// Presents the most recently drawn frame if it hasn't been presented yet.
    ///
    /// Benchmarks drive drawing synchronously rather than through a platform
    /// frame-request loop, so they call this after each measured update to
    /// submit the frame like production presentation would.
    #[cfg(feature = "bench")]
    pub fn present_if_needed(&mut self) {
        if self.needs_present.get() {
            self.present();
        }
    }

    /// Returns a snapshot of the current input-latency histograms.
    #[cfg(feature = "input-latency-histogram")]
    pub fn input_latency_snapshot(&self) -> InputLatencySnapshot {
        self.input_latency_tracker.snapshot()
    }

    fn draw_roots(&mut self, cx: &mut App) {
        self.invalidator.set_phase(DrawPhase::Prepaint);
        self.tooltip_bounds.take();

        self.a11y.sync_active_flag();
        if self.a11y.is_active() {
            self.a11y.begin_frame();
        }

        let _inspector_width: Pixels = rems(30.0).to_pixels(self.rem_size());
        let root_size = {
            #[cfg(any(feature = "inspector", debug_assertions))]
            {
                if self.inspector.is_some() {
                    let mut size = self.viewport_size;
                    size.width = (size.width - _inspector_width).max(px(0.0));
                    size
                } else {
                    self.viewport_size
                }
            }
            #[cfg(not(any(feature = "inspector", debug_assertions)))]
            {
                self.viewport_size
            }
        };

        // Riseonly 0088: only cached views changed: copy the frame around them.
        if self.try_splice(cx) {
            return;
        }

        // Layout all root elements. Like the root element on the web, which
        // stretches to fill the viewport unless explicitly sized, window roots
        // fill the window when their size is `auto`.
        let scale_factor = self.scale_factor();
        let mut root_element = self.root.as_ref().unwrap().clone().into_any_element();
        let root_layout_id = root_element.request_layout(self, cx);
        self.layout_engine
            .as_mut()
            .unwrap()
            .stretch_auto_size_to_fill(root_layout_id, root_size, scale_factor);
        self.hitbox_layer = 0;
        self.next_frame.prepaint_start = self.prepaint_index();
        root_element.prepaint_as_root(Point::default(), root_size.into(), self, cx);
        self.next_frame.prepaint_end = self.prepaint_index();

        #[cfg(any(feature = "inspector", debug_assertions))]
        let inspector_element = self.prepaint_inspector(_inspector_width, cx);

        self.prepaint_deferred_draws(None, cx);
        self.hitbox_layer = usize::MAX;

        let mut prompt_element = None;
        let mut active_drag_element = None;
        let mut tooltip_element = None;
        if let Some(prompt) = self.prompt.take() {
            let mut element = prompt.view.any_view().into_any_element();
            let prompt_layout_id = element.request_layout(self, cx);
            self.layout_engine
                .as_mut()
                .unwrap()
                .stretch_auto_size_to_fill(prompt_layout_id, root_size, scale_factor);
            element.prepaint_as_root(Point::default(), root_size.into(), self, cx);
            prompt_element = Some(element);
            self.prompt = Some(prompt);
        } else if let Some(active_drag) = cx.active_drag.take() {
            let mut element = active_drag.view.clone().into_any_element();
            let offset = self.mouse_position() - active_drag.cursor_offset;
            element.prepaint_as_root(offset, AvailableSpace::min_size(), self, cx);
            active_drag_element = Some(element);
            cx.active_drag = Some(active_drag);
        } else {
            tooltip_element = self.prepaint_tooltip(cx);
        }

        self.mouse_hit_test = self.next_frame.hit_test(self.mouse_position);

        // Now actually paint the elements.
        self.invalidator.set_phase(DrawPhase::Paint);
        profiler::note_paint_start();
        self.next_frame.paint_start = self.paint_index();
        root_element.paint(self, cx);
        self.next_frame.paint_end = self.paint_index();
        // Riseonly 0088: a frame drawn by the root pass and its deferred draws (0098) alone may be
        // spliced by the next one.
        // why: Riseonly 0102 (X2007), a tooltip no longer refuses it: its requests lie in the
        // prepaint spans a splice copies, and the splice lays out and paints the tooltip again
        // after the deferred draws, as this pass does; a hovered button made every frame a root
        // draw before.
        self.next_frame.spliceable = prompt_element.is_none() && active_drag_element.is_none();
        self.next_frame.viewport = (self.viewport_size, self.scale_factor);

        #[cfg(any(feature = "inspector", debug_assertions))]
        self.paint_inspector(inspector_element, cx);

        self.paint_deferred_draws(None, cx);

        if let Some(mut prompt_element) = prompt_element {
            prompt_element.paint(self, cx);
        } else if let Some(mut drag_element) = active_drag_element {
            drag_element.paint(self, cx);
        } else if let Some(mut tooltip_element) = tooltip_element {
            tooltip_element.paint(self, cx);
        }
        profiler::note_paint_end();

        #[cfg(any(feature = "inspector", debug_assertions))]
        self.paint_inspector_hitbox(cx);

        // a11y may have been activated/deactivated halfway through the frame
        let a11y_active_start_of_frame = self.a11y.is_active();
        self.a11y.sync_active_flag();
        let a11y_active_end_of_frame = self.a11y.is_active();

        let should_send_a11y_update = a11y_active_start_of_frame && a11y_active_end_of_frame;

        if a11y_active_start_of_frame {
            // Harvest frame metadata for the debug dump while the live window
            // and frame are still in scope.
            let frame_info = crate::window::a11y::debug::FrameDebugInfo {
                viewport_size: self.viewport_size,
                scale_factor: self.scale_factor,
                tab_stop_count: self.next_frame.tab_stops.tab_stop_count(),
            };
            // clear the builder state regardless
            let tree_update = self.a11y.end_frame(frame_info);

            if should_send_a11y_update {
                log::debug!(
                    "Sending a11y tree update: {} nodes",
                    tree_update.nodes.len()
                );
                self.platform_window.a11y_tree_update(tree_update);
            }
        }
    }

    /// Riseonly 0088: draws a frame in which only cached views changed, each one shown at the place
    /// and size it had, by copying the frame just drawn around them and drawing each of them alone
    /// in its place: no root render and no window layout (X1835: a notify inside the chat rendered
    /// every view up to the root and laid out the window, 9-12 ms). False, with the frame being
    /// drawn untouched, when the frame cannot be spliced.
    fn try_splice(&mut self, cx: &mut App) -> bool {
        let spliced = self.splice_frame(cx);
        if spliced {
            profiler::note_splice("spliced");
        }
        spliced
    }

    /// Riseonly 0090: the first reason the frame cannot be spliced, before any hole is looked at.
    fn splice_refusal(&self, cx: &App) -> Option<&'static str> {
        let refusal = if self.refreshing {
            "refresh"
        } else if self.root_dirty {
            profiler::root_dirty_reason().unwrap_or("root")
        } else if self.direct_dirty.is_empty() {
            "no_notify"
        } else if !self.rendered_frame.spliceable {
            "unspliceable"
        } else if self.rendered_frame.viewport != (self.viewport_size, self.scale_factor) {
            "viewport"
        } else if self.a11y.is_active() {
            "a11y"
        } else if self.prompt.is_some() {
            "prompt"
        } else if cx.active_drag.is_some() {
            "drag"
        } else {
            #[cfg(any(feature = "inspector", debug_assertions))]
            if self.inspector.is_some() {
                return Some("inspector");
            }
            return None;
        };
        Some(refusal)
    }

    fn splice_frame(&mut self, cx: &mut App) -> bool {
        if let Some(refusal) = self.splice_refusal(cx) {
            profiler::note_splice(refusal);
            return false;
        }

        // The outermost notified views; one inside another is drawn with it.
        let mut holes: Vec<EntityId> = Vec::new();
        for view_id in self.direct_dirty.iter().copied() {
            if !self.rendered_frame.holes.contains_key(&view_id) {
                if profiler::frame_trace_enabled() {
                    profiler::note_splice(profiler::detailed_reason("no_hole", profiler::view_type_name(view_id)));
                }
                return false;
            }
            let nested = self
                .rendered_frame
                .dispatch_tree
                .view_path_reversed(view_id)
                .skip(1)
                .any(|ancestor| self.direct_dirty.contains(&ancestor));
            if !nested {
                holes.push(view_id);
            }
        }
        // Riseonly 0098: each hole lies in the root pass or in one deferred draw, and the frame is
        // copied around it there in the same way.
        let mut placed: Vec<(EntityId, HolePass)> = Vec::with_capacity(holes.len());
        for view_id in &holes {
            let Some(pass) = self.hole_pass(&self.rendered_frame.holes[view_id]) else {
                if profiler::frame_trace_enabled() {
                    profiler::note_splice(profiler::detailed_reason("pass", profiler::view_type_name(*view_id)));
                }
                return false;
            };
            placed.push((*view_id, pass));
        }
        placed.sort_by_key(|(view_id, pass)| {
            (pass.order(), self.rendered_frame.holes[view_id].prepaint_range.start.dispatch_tree_index)
        });

        // invariant: the holes of a pass lie inside it, in frame order, apart in every index, and a
        // deferred draw holding holes is copied into the spliced frame.
        let mut index = 0;
        while index < placed.len() {
            let pass = placed[index].1;
            let (prepaint_bounds, paint_bounds) = self.pass_ranges(pass);
            let mut prepaint_cursor = prepaint_bounds.start.clone();
            let mut paint_cursor = paint_bounds.start.clone();
            while index < placed.len() && placed[index].1 == pass {
                let record = &self.rendered_frame.holes[&placed[index].0];
                if !prepaint_cursor.le(&record.prepaint_range.start)
                    || !record.prepaint_range.start.le(&record.prepaint_range.end)
                    || !paint_cursor.le(&record.paint_range.start)
                    || !record.paint_range.start.le(&record.paint_range.end)
                {
                    profiler::note_splice("order");
                    return false;
                }
                prepaint_cursor = record.prepaint_range.end.clone();
                paint_cursor = record.paint_range.end.clone();
                index += 1;
            }
            if !prepaint_cursor.le(&prepaint_bounds.end) || !paint_cursor.le(&paint_bounds.end) {
                profiler::note_splice("order");
                return false;
            }
            if let HolePass::Deferred(draw) = pass
                && !self.deferred_copied(draw, &placed)
            {
                profiler::note_splice("order");
                return false;
            }
        }

        // Each hole is laid out first, under the state it was drawn in: one whose style no longer
        // gives its old size refuses the splice before anything is copied.
        let mut splice = SpliceDraw {
            holes: Vec::with_capacity(placed.len()),
            map: SpliceMap::default(),
            nodes: SpliceNodes::default(),
        };
        for (view_id, pass) in &placed {
            let record = &self.rendered_frame.holes[view_id];
            let view = record.view.clone();
            let style = record.style.clone();
            let replays_moved = record.replays_moved;
            let bounds = record.bounds;
            let stacks = (*record.prepaint).clone();
            let paint_stacks = record.paint.clone();
            let prepaint_range = record.prepaint_range.clone();
            let paint_range = record.paint_range.clone();
            let outer = self.swap_hole_stacks(stacks, cx);
            let mut element = view.cached(style);
            if replays_moved {
                element = element.replay_when_moved();
            }
            let mut element = element.into_any_element();
            let size = element.layout_as_root(bounds.size.into(), self, cx);
            let inner = self.swap_hole_stacks(outer, cx);
            // why: a cached view lays out as a styled leaf without rendering, so a refusal here
            // left nothing in the frame being drawn (no cached views, reads or states twice).
            if size != bounds.size {
                if profiler::frame_trace_enabled() {
                    profiler::note_splice(profiler::detailed_reason("size", profiler::view_type_name(*view_id)));
                }
                return false;
            }
            splice.holes.push(SpliceHole {
                view_id: *view_id,
                pass: *pass,
                prepaint_range,
                paint_range,
                paint_stacks,
                element,
                stacks: inner,
                bounds,
                drawn: false,
            });
        }

        self.spliced = true;

        // Prepaint: the spans between the holes copied, the holes drawn fresh; deferred draws
        // copied with the spans replay whole or are spliced the same way, the ones a fresh hole
        // registered draw fresh, over the frame as the root pass does.
        self.hitbox_layer = 0;
        // Riseonly 0100: the root layout is not run again, so the states it read before the root
        // prepaint (keyed states, change slots and every other state of the views rendered outside
        // a cached one) stay as they are; unlisted, the frame swap dropped them and the next root
        // render built all of them anew (X2004: appear and change handlers notified, the root
        // rendered again and the next splice dropped them again, 150 frames a second at idle).
        let layout_states = self.rendered_frame.prepaint_start.accessed_element_states_index;
        self.next_frame.accessed_element_states.extend(
            self.rendered_frame.accessed_element_states[..layout_states]
                .iter()
                .map(|(id, type_id)| (id.clone(), *type_id)),
        );
        self.next_frame.prepaint_start = self.prepaint_index();
        let root_prepaint = self.rendered_frame.prepaint_start.clone()..self.rendered_frame.prepaint_end.clone();
        self.splice_prepaint_pass(HolePass::Root, root_prepaint, None, &mut splice, cx);
        self.next_frame.prepaint_end = self.prepaint_index();
        self.prepaint_deferred_draws(Some(&mut splice), cx);
        self.hitbox_layer = usize::MAX;
        let mut tooltip_element = self.prepaint_tooltip(cx);
        self.mouse_hit_test = self.next_frame.hit_test(self.mouse_position);

        // Paint, the same way.
        self.invalidator.set_phase(DrawPhase::Paint);
        profiler::note_paint_start();
        self.next_frame.paint_start = self.paint_index();
        let root_paint = self.rendered_frame.paint_start.clone()..self.rendered_frame.paint_end.clone();
        self.splice_paint_pass(HolePass::Root, root_paint, &mut splice, cx);
        self.next_frame.paint_end = self.paint_index();
        // invariant: the same test as the root pass (0102): no prompt or drag reaches a splice, and
        // the tooltip is drawn again below, outside every copied range.
        self.next_frame.spliceable = true;
        self.paint_deferred_draws(Some(&mut splice), cx);
        if let Some(tooltip_element) = tooltip_element.as_mut() {
            tooltip_element.paint(self, cx);
        }
        profiler::note_paint_end();
        // why: a hole whose deferred draw the frame did not copy was laid out but not drawn; its
        // view renders in the next frame instead of staying stale.
        for hole in &splice.holes {
            if !hole.drawn {
                log::warn!("gpui 0098 spliced hole not drawn");
                cx.notify(hole.view_id);
            }
        }

        // What the copied spans hold outside the indexed lists.
        let hole_views = splice.holes.iter().map(|hole| hole.view_id).collect::<Vec<_>>();
        let under_hole = |window: &Self, view_id: EntityId| {
            window
                .rendered_frame
                .dispatch_tree
                .view_path_reversed(view_id)
                .any(|ancestor| hole_views.contains(&ancestor))
        };
        let map = splice.map;
        let control_hitboxes = self.rendered_frame.window_control_hitboxes.clone();
        self.next_frame.window_control_hitboxes.extend(control_hitboxes);
        let cached_views = self
            .rendered_frame
            .cached_views
            .iter()
            .filter(|(view_id, _)| !under_hole(self, *view_id))
            .cloned()
            .collect::<Vec<_>>();
        self.next_frame.cached_views.extend(cached_views);
        let own_reads = self
            .rendered_frame
            .own_reads
            .iter()
            .filter(|(view_id, _)| !self.next_frame.own_reads.contains_key(*view_id) && !under_hole(self, **view_id))
            .map(|(view_id, own)| (*view_id, own.clone()))
            .collect::<Vec<_>>();
        self.next_frame.own_reads.extend(own_reads);
        let moved_views = self
            .rendered_frame
            .moved_views
            .iter()
            .filter(|(view_id, _)| !under_hole(self, *view_id))
            .copied()
            .collect::<Vec<_>>();
        self.next_frame.moved_views.extend(moved_views);
        let old_holes = mem::take(&mut self.rendered_frame.holes);
        for (view_id, mut record) in old_holes {
            if self.next_frame.holes.contains_key(&view_id) || under_hole(self, view_id) {
                continue;
            }
            if let (Some(prepaint_range), Some(paint_range)) =
                (map.remap_prepaint(&record.prepaint_range), map.remap_paint(&record.paint_range))
            {
                record.prepaint_range = prepaint_range;
                record.paint_range = paint_range;
                self.next_frame.holes.insert(view_id, record);
            }
        }
        crate::view::rebase_spliced_views(&map, self);
        self.next_frame.viewport = (self.viewport_size, self.scale_factor);
        true
    }

    /// Riseonly 0098: the pass of the frame just drawn whose ranges hold the hole `record`.
    fn hole_pass(&self, record: &HoleRecord) -> Option<HolePass> {
        let inside = |prepaint: &Range<PrepaintStateIndex>, paint: &Range<PaintIndex>| {
            prepaint.start.le(&record.prepaint_range.start)
                && record.prepaint_range.end.le(&prepaint.end)
                && paint.start.le(&record.paint_range.start)
                && record.paint_range.end.le(&paint.end)
        };
        let (root_prepaint, root_paint) = self.pass_ranges(HolePass::Root);
        if inside(&root_prepaint, &root_paint) {
            return Some(HolePass::Root);
        }
        self.rendered_frame
            .deferred_draws
            .iter()
            .position(|draw| inside(&draw.prepaint_range, &draw.paint_range))
            .map(HolePass::Deferred)
    }

    /// Riseonly 0098: the prepaint and paint ranges of a pass of the frame just drawn.
    fn pass_ranges(&self, pass: HolePass) -> (Range<PrepaintStateIndex>, Range<PaintIndex>) {
        let frame = &self.rendered_frame;
        match pass {
            HolePass::Deferred(index) if index < frame.deferred_draws.len() => {
                let draw = &frame.deferred_draws[index];
                (draw.prepaint_range.clone(), draw.paint_range.clone())
            }
            _ => (
                frame.prepaint_start.clone()..frame.prepaint_end.clone(),
                frame.paint_start.clone()..frame.paint_end.clone(),
            ),
        }
    }

    /// Riseonly 0098: whether the deferred draw `draw` of the frame just drawn goes into the spliced
    /// frame: registered in a copied span (not inside a hole) of a pass that is copied itself.
    fn deferred_copied(&self, draw: usize, placed: &[(EntityId, HolePass)]) -> bool {
        let frame = &self.rendered_frame;
        let registers = |range: &Range<PrepaintStateIndex>, draw: usize| {
            range.start.deferred_draws_index <= draw && draw < range.end.deferred_draws_index
        };
        let mut draw = draw;
        // why: deferred draws nest at most ten rounds deep (prepaint_deferred_draws).
        for _ in 0..12 {
            if placed
                .iter()
                .any(|(view_id, _)| registers(&frame.holes[view_id].prepaint_range, draw))
            {
                return false;
            }
            if registers(&(frame.prepaint_start.clone()..frame.prepaint_end.clone()), draw) {
                return true;
            }
            let Some(outer) = frame
                .deferred_draws
                .iter()
                .position(|outer| registers(&outer.prepaint_range, draw))
            else {
                return false;
            };
            draw = outer;
        }
        false
    }

    /// Riseonly 0098: copies the prepaint range `old` of one pass of the frame just drawn in spans
    /// around the holes it holds and prepaints each hole fresh in its place, under the node of its
    /// view copied just before it. `base` is the node of the frame just drawn the pass hangs under (the parent of a
    /// deferred draw), None for the root pass.
    fn splice_prepaint_pass(
        &mut self,
        pass: HolePass,
        old: Range<PrepaintStateIndex>,
        base: Option<DispatchNodeId>,
        splice: &mut SpliceDraw,
        cx: &mut App,
    ) {
        let mut spans = SpliceSpans::with_base(base);
        let mut segments = Vec::new();
        let mut hole_ranges = Vec::new();
        let mut cursor = old.start;
        for hole in splice.holes.iter_mut().filter(|hole| hole.pass == pass) {
            let new_start = self.prepaint_index();
            self.reuse_prepaint_span(cursor.clone()..hole.prepaint_range.start.clone(), &mut spans, &mut splice.nodes);
            segments.push((cursor..hole.prepaint_range.start.clone(), new_start));
            let outer = self.swap_hole_stacks(hole.stacks.clone(), cx);
            hole.element.prepaint_at(hole.bounds.origin, self, cx);
            hole.stacks = self.swap_hole_stacks(outer, cx);
            hole_ranges.push(hole.prepaint_range.clone());
            cursor = hole.prepaint_range.end.clone();
        }
        let new_start = self.prepaint_index();
        self.reuse_prepaint_span(cursor.clone()..old.end.clone(), &mut spans, &mut splice.nodes);
        segments.push((cursor..old.end, new_start));
        self.next_frame.dispatch_tree.finish_spans(&mut spans);
        if spans.contains_focus {
            self.next_frame.focus = self.focus;
        }
        splice.map.prepaint.push(SplicePass {
            segments,
            holes: hole_ranges,
        });
    }

    /// Riseonly 0098: [`Self::splice_prepaint_pass`] for the paint range `old` of the pass.
    fn splice_paint_pass(&mut self, pass: HolePass, old: Range<PaintIndex>, splice: &mut SpliceDraw, cx: &mut App) {
        let mut segments = Vec::new();
        let mut hole_ranges = Vec::new();
        let mut cursor = old.start;
        for hole in splice.holes.iter_mut().filter(|hole| hole.pass == pass) {
            let new_start = self.paint_index();
            self.reuse_paint(cursor.clone()..hole.paint_range.start.clone());
            segments.push((cursor..hole.paint_range.start.clone(), new_start));
            let outer = self.swap_hole_stacks((*hole.paint_stacks).clone(), cx);
            self.hole_clip_boundary = hole.paint_stacks.own_clip_boundary;
            hole.element.paint(self, cx);
            self.hole_clip_boundary = false;
            hole.stacks = self.swap_hole_stacks(outer, cx);
            hole.drawn = true;
            hole_ranges.push(hole.paint_range.clone());
            cursor = hole.paint_range.end.clone();
        }
        let new_start = self.paint_index();
        self.reuse_paint(cursor.clone()..old.end.clone());
        segments.push((cursor..old.end, new_start));
        splice.map.paint.push(SplicePass {
            segments,
            holes: hole_ranges,
        });
    }

    /// Riseonly 0088: a range whose element state keys are every key the spliced frame recorded,
    /// in its prepaint and its paint.
    pub(crate) fn spliced_state_keys(&self) -> Range<PrepaintStateIndex> {
        let start = self.next_frame.prepaint_start.clone();
        let mut end = self.next_frame.prepaint_end.clone();
        end.accessed_element_states_index = self.next_frame.accessed_element_states.len();
        start..end
    }

    /// Riseonly 0088: [`Self::reuse_prepaint`] for a span of a spliced frame: the root pass only
    /// (no deferred draw), hitboxes kept as recorded, the dispatch path left open for the hole
    /// drawn after it.
    fn reuse_prepaint_span(
        &mut self,
        range: Range<PrepaintStateIndex>,
        spans: &mut SpliceSpans,
        nodes: &mut SpliceNodes,
    ) {
        self.next_frame.move_listeners.extend(
            self.rendered_frame.move_listeners
                [range.start.move_listeners_index..range.end.move_listeners_index]
                .iter()
                .cloned(),
        );
        self.next_frame.hitboxes.extend(
            self.rendered_frame.hitboxes[range.start.hitboxes_index..range.end.hitboxes_index]
                .iter()
                .cloned(),
        );
        self.next_frame.tooltip_requests.extend(
            self.rendered_frame.tooltip_requests
                [range.start.tooltips_index..range.end.tooltips_index]
                .iter_mut()
                .map(|request| request.take()),
        );
        self.next_frame.accessed_element_states.extend(
            self.rendered_frame.accessed_element_states[range.start.accessed_element_states_index
                ..range.end.accessed_element_states_index]
                .iter()
                .map(|(id, type_id)| (id.clone(), *type_id)),
        );
        self.text_system
            .reuse_layouts(range.start.line_layout_index..range.end.line_layout_index);
        self.next_frame.dispatch_tree.reuse_span(
            range.start.dispatch_tree_index..range.end.dispatch_tree_index,
            &mut self.rendered_frame.dispatch_tree,
            self.focus,
            spans,
            nodes,
        );
        // Riseonly 0098: the deferred draws the span registered, replayed or spliced after the
        // root pass under the node their parent was copied to.
        let fallback = self.next_frame.dispatch_tree.active_node_id();
        let deferred_from = range.start.deferred_draws_index;
        let copied = self.rendered_frame.deferred_draws
            [range.start.deferred_draws_index..range.end.deferred_draws_index]
            .iter()
            .enumerate()
            .filter_map(|(offset, deferred_draw)| {
                let parent_node = nodes.refresh(deferred_draw.parent_node).or(fallback)?;
                Some(DeferredDraw {
                    current_view: deferred_draw.current_view,
                    parent_node,
                    element_id_stack: deferred_draw.element_id_stack.clone(),
                    text_style_stack: deferred_draw.text_style_stack.clone(),
                    content_mask: deferred_draw.content_mask,
                    rem_size: deferred_draw.rem_size,
                    priority: deferred_draw.priority,
                    element: None,
                    absolute_offset: deferred_draw.absolute_offset,
                    prepaint_range: deferred_draw.prepaint_range.clone(),
                    paint_range: deferred_draw.paint_range.clone(),
                    copied_from: Some(deferred_from + offset),
                    reads: deferred_draw.reads.clone(),
                })
            })
            .collect::<Vec<_>>();
        self.next_frame.deferred_draws.extend(copied);
    }

    /// Riseonly 0088: the state around the cached view `entity_id` being drawn, without its own
    /// element id. None when the id stack does not end with it or the environment cannot be
    /// captured.
    pub(crate) fn capture_hole_stacks(&self, entity_id: EntityId, cx: &mut App) -> Option<HoleStacks> {
        if self.element_id_stack.last() != Some(&ElementId::View(entity_id)) {
            return None;
        }
        let mut element_id_stack = self.element_id_stack.clone();
        element_id_stack.pop();
        let env = match env_snapshot_hooks() {
            Some(hooks) => Some((hooks.capture)(cx)),
            None if self.env_fingerprint == 0 => None,
            None => return None,
        };
        Some(HoleStacks {
            element_id_stack,
            text_style_stack: self.text_style_stack.clone(),
            rendered_entity_stack: self.rendered_entity_stack.clone(),
            element_offset_stack: self.element_offset_stack.clone(),
            content_mask_stack: self.content_mask_stack.clone(),
            paint_mask_stack: self.paint_mask_stack.clone(),
            scene_clip_stack: self.next_frame.scene.clip_stack.clone(),
            own_clip_boundary: false,
            surface_corner_clip_stack: self.surface_corner_clip_stack.clone(),
            transformation_stack: self.transformation_stack.clone(),
            image_cache_stack: self.image_cache_stack.clone(),
            rem_size_override_stack: self.rem_size_override_stack.clone(),
            element_opacity: self.element_opacity,
            glyph_swap_key: self.glyph_swap_key,
            image_swap_key: self.image_swap_key,
            hitbox_layer: self.hitbox_layer,
            env_fingerprint: self.env_fingerprint,
            env_trace: self.env_trace.clone(),
            env,
        })
    }

    /// Riseonly 0088: puts `stacks` in place and returns the state they replaced.
    fn swap_hole_stacks(&mut self, stacks: HoleStacks, cx: &mut App) -> HoleStacks {
        let env = match (env_snapshot_hooks(), stacks.env) {
            (Some(hooks), Some(env)) => Some((hooks.swap)(cx, env)),
            _ => None,
        };
        HoleStacks {
            element_id_stack: mem::replace(&mut self.element_id_stack, stacks.element_id_stack),
            text_style_stack: mem::replace(&mut self.text_style_stack, stacks.text_style_stack),
            rendered_entity_stack: mem::replace(&mut self.rendered_entity_stack, stacks.rendered_entity_stack),
            element_offset_stack: mem::replace(&mut self.element_offset_stack, stacks.element_offset_stack),
            content_mask_stack: mem::replace(&mut self.content_mask_stack, stacks.content_mask_stack),
            paint_mask_stack: mem::replace(&mut self.paint_mask_stack, stacks.paint_mask_stack),
            scene_clip_stack: mem::replace(&mut self.next_frame.scene.clip_stack, stacks.scene_clip_stack),
            own_clip_boundary: stacks.own_clip_boundary,
            surface_corner_clip_stack: mem::replace(
                &mut self.surface_corner_clip_stack,
                stacks.surface_corner_clip_stack,
            ),
            transformation_stack: mem::replace(&mut self.transformation_stack, stacks.transformation_stack),
            image_cache_stack: mem::replace(&mut self.image_cache_stack, stacks.image_cache_stack),
            rem_size_override_stack: mem::replace(
                &mut self.rem_size_override_stack,
                stacks.rem_size_override_stack,
            ),
            element_opacity: mem::replace(&mut self.element_opacity, stacks.element_opacity),
            glyph_swap_key: mem::replace(&mut self.glyph_swap_key, stacks.glyph_swap_key),
            image_swap_key: mem::replace(&mut self.image_swap_key, stacks.image_swap_key),
            hitbox_layer: mem::replace(&mut self.hitbox_layer, stacks.hitbox_layer),
            env_fingerprint: mem::replace(&mut self.env_fingerprint, stacks.env_fingerprint),
            env_trace: mem::replace(&mut self.env_trace, stacks.env_trace),
            env,
        }
    }

    fn prepaint_tooltip(&mut self, cx: &mut App) -> Option<AnyElement> {
        // Use indexing instead of iteration to avoid borrowing self for the duration of the loop.
        for tooltip_request_index in (0..self.next_frame.tooltip_requests.len()).rev() {
            let Some(Some(tooltip_request)) = self
                .next_frame
                .tooltip_requests
                .get(tooltip_request_index)
                .cloned()
            else {
                log::error!("Unexpectedly absent TooltipRequest");
                continue;
            };
            let mut element = tooltip_request.tooltip.view.clone().into_any_element();
            let mouse_position = tooltip_request.tooltip.mouse_position;
            let tooltip_size = element.layout_as_root(AvailableSpace::min_size(), self, cx);

            let mut tooltip_bounds =
                Bounds::new(mouse_position + point(px(1.), px(1.)), tooltip_size);
            let window_bounds = Bounds {
                origin: Point::default(),
                size: self.viewport_size(),
            };

            if tooltip_bounds.right() > window_bounds.right() {
                let new_x = mouse_position.x - tooltip_bounds.size.width - px(1.);
                if new_x >= Pixels::ZERO {
                    tooltip_bounds.origin.x = new_x;
                } else {
                    tooltip_bounds.origin.x = cmp::max(
                        Pixels::ZERO,
                        tooltip_bounds.origin.x - tooltip_bounds.right() - window_bounds.right(),
                    );
                }
            }

            if tooltip_bounds.bottom() > window_bounds.bottom() {
                let new_y = mouse_position.y - tooltip_bounds.size.height - px(1.);
                if new_y >= Pixels::ZERO {
                    tooltip_bounds.origin.y = new_y;
                } else {
                    tooltip_bounds.origin.y = cmp::max(
                        Pixels::ZERO,
                        tooltip_bounds.origin.y - tooltip_bounds.bottom() - window_bounds.bottom(),
                    );
                }
            }

            // It's possible for an element to have an active tooltip while not being painted (e.g.
            // via the `visible_on_hover` method). Since mouse listeners are not active in this
            // case, instead update the tooltip's visibility here.
            let is_visible =
                (tooltip_request.tooltip.check_visible_and_update)(tooltip_bounds, self, cx);
            if !is_visible {
                continue;
            }

            self.with_absolute_element_offset(tooltip_bounds.origin, |window| {
                element.prepaint(window, cx)
            });

            self.tooltip_bounds = Some(TooltipBounds {
                id: tooltip_request.id,
                bounds: tooltip_bounds,
            });
            return Some(element);
        }
        None
    }

    fn prepaint_deferred_draws(&mut self, mut splice: Option<&mut SpliceDraw>, cx: &mut App) {
        assert_eq!(self.element_id_stack.len(), 0);

        // Process deferred draws in multiple rounds to support nesting.
        // Each round processes all current deferred draws, which may push new ones.
        //
        // The draws are processed in place rather than being moved out of
        // `next_frame.deferred_draws`: `prepaint_index` snapshots that vector's
        // length, so any prepaint range recorded during a round (view caches,
        // nested deferred draws) must index the same vector `reuse_prepaint`
        // slices on the next frame. Moving the draws out and re-appending them
        // shifts the indices of nested draws, causing reused subtrees to graft
        // the wrong deferred draws and panic in the dispatch tree.
        let mut round_start = 0;
        let mut depth = 0;
        loop {
            let round_end = self.next_frame.deferred_draws.len();
            if round_start == round_end {
                break;
            }
            // Limit maximum nesting depth to prevent infinite loops.
            assert!(depth < 10, "Exceeded maximum (10) deferred depth");
            depth += 1;

            // Sort this round by priority.
            let mut traversal_order = (round_start..round_end).collect::<SmallVec<[usize; 8]>>();
            traversal_order.sort_by_key(|ix| self.next_frame.deferred_draws[*ix].priority);

            for deferred_draw_ix in traversal_order {
                let (element, parent_node, current_view, rem_size, absolute_offset, prepaint_range, copied_from) = {
                    let deferred_draw = &mut self.next_frame.deferred_draws[deferred_draw_ix];
                    self.element_id_stack
                        .clone_from(&deferred_draw.element_id_stack);
                    self.text_style_stack
                        .clone_from(&deferred_draw.text_style_stack);
                    (
                        deferred_draw.element.take(),
                        deferred_draw.parent_node,
                        deferred_draw.current_view,
                        deferred_draw.rem_size,
                        deferred_draw.absolute_offset,
                        deferred_draw.prepaint_range.clone(),
                        deferred_draw.copied_from,
                    )
                };
                // Riseonly 0098: a copied draw holding holes of a spliced frame.
                let spliced_pass = copied_from
                    .map(HolePass::Deferred)
                    .filter(|pass| splice.as_ref().is_some_and(|splice| splice.holds(*pass)));
                self.hitbox_layer = self.next_frame.deferred_draws[deferred_draw_ix]
                    .priority
                    .saturating_add(1);
                self.next_frame.dispatch_tree.set_active_node(parent_node);

                let prepaint_start = self.prepaint_index();
                let fresh = element.is_some();
                if let Some(mut element) = element {
                    // Riseonly 0108: the element lays out here, after the view that deferred it
                    // was drawn, so its reads (lazy list rows, their stores) land outside that
                    // view's capture; as direct reads of the frame they made each notify of them
                    // dirty the root. They are the deferring view's reads instead.
                    let ((), own, full) = cx.detect_own_accessed_entities(|cx| {
                        self.with_rendered_view(current_view, |window| {
                            window.with_rem_size(Some(rem_size), |window| {
                                window.with_absolute_element_offset(absolute_offset, |window| {
                                    element.prepaint(window, cx);
                                });
                            });
                        });
                    });
                    let draw = &mut self.next_frame.deferred_draws[deferred_draw_ix];
                    draw.element = Some(element);
                    draw.reads = Some(DeferredReads { own: Rc::new(own), full: Rc::new(full) });
                } else if let (Some(pass), Some(splice)) = (spliced_pass, splice.as_deref_mut()) {
                    let base = copied_from
                        .and_then(|from| self.rendered_frame.deferred_draws.get(from))
                        .map(|draw| draw.parent_node);
                    self.splice_prepaint_pass(pass, prepaint_range.clone(), base, splice, cx);
                } else {
                    self.reuse_prepaint(prepaint_range.clone());
                    // Riseonly 0080: the cached views inside the replayed draw replayed with it.
                    let replayed = prepaint_start.clone()..self.prepaint_index();
                    crate::view::rebase_nested_views(&prepaint_range, &replayed, self);
                    // Riseonly 0098: holes of the frame just drawn inside it move with it.
                    if let Some(splice) = splice.as_deref_mut() {
                        splice.map.prepaint.push(SplicePass::whole(prepaint_range.clone(), prepaint_start.clone()));
                    }
                }
                let reads = self.next_frame.deferred_draws[deferred_draw_ix].reads.clone();
                if let Some(reads) = reads {
                    self.attribute_deferred_reads(current_view, &reads, fresh, splice.is_some(), cx);
                }
                let prepaint_end = self.prepaint_index();
                self.next_frame.deferred_draws[deferred_draw_ix].prepaint_range =
                    prepaint_start..prepaint_end;
            }

            self.element_id_stack.clear();
            self.text_style_stack.clear();
            round_start = round_end;
        }
    }

    /// Riseonly 0108: the reads of a deferred draw go to the cached view that deferred it, as if
    /// its render read them; a draw deferred by the root (no cached view) keeps them as the
    /// frame's own reads.
    fn attribute_deferred_reads(
        &mut self,
        view_id: EntityId,
        reads: &DeferredReads,
        fresh: bool,
        spliced: bool,
        cx: &mut App,
    ) {
        if !fresh {
            // why: a replayed draw ran no capture; the frame still depends on what it read.
            cx.extend_nested_accessed(&reads.full);
        }
        let base = self
            .next_frame
            .own_reads
            .get(&view_id)
            .or_else(|| spliced.then(|| self.rendered_frame.own_reads.get(&view_id)).flatten())
            .cloned();
        let Some(base) = base else {
            cx.entities.accessed_entities.get_mut().extend(reads.own.iter().copied());
            return;
        };
        if !reads.own.is_subset(&base) {
            let mut own = (*base).clone();
            own.extend(reads.own.iter().copied());
            self.next_frame.own_reads.insert(view_id, Rc::new(own));
        } else if !self.next_frame.own_reads.contains_key(&view_id) {
            self.next_frame.own_reads.insert(view_id, base);
        }
        // invariant: a spliced frame copies the readers of the frame before it for every draw it
        // did not draw again; listing a replayed draw here too would grow the list each frame.
        if fresh || !spliced {
            self.next_frame.cached_views.push((view_id, reads.full.clone()));
        }
    }

    fn paint_deferred_draws(&mut self, mut splice: Option<&mut SpliceDraw>, cx: &mut App) {
        assert_eq!(self.element_id_stack.len(), 0);

        // Paint all deferred draws in priority order.
        // Since prepaint has already processed nested deferreds, we just paint them all.
        if self.next_frame.deferred_draws.len() == 0 {
            return;
        }

        let traversal_order = self.deferred_draw_traversal_order();
        let mut deferred_draws = mem::take(&mut self.next_frame.deferred_draws);
        for deferred_draw_ix in traversal_order {
            let mut deferred_draw = &mut deferred_draws[deferred_draw_ix];
            self.element_id_stack
                .clone_from(&deferred_draw.element_id_stack);
            self.next_frame
                .dispatch_tree
                .set_active_node(deferred_draw.parent_node);

            let paint_start = self.paint_index();
            let content_mask = deferred_draw.content_mask;
            // Riseonly 0098: a copied draw holding holes of a spliced frame.
            let spliced_pass = deferred_draw
                .copied_from
                .map(HolePass::Deferred)
                .filter(|pass| splice.as_ref().is_some_and(|splice| splice.holds(*pass)));
            if let Some(element) = deferred_draw.element.as_mut() {
                self.with_rendered_view(deferred_draw.current_view, |window| {
                    window.with_content_mask(content_mask, |window| {
                        window.with_rem_size(Some(deferred_draw.rem_size), |window| {
                            element.paint(window, cx);
                        });
                    })
                })
            } else if let (Some(pass), Some(splice)) = (spliced_pass, splice.as_deref_mut()) {
                let range = deferred_draw.paint_range.clone();
                self.splice_paint_pass(pass, range, splice, cx);
            } else if self.paint_range_in_frame(&deferred_draw.paint_range) {
                let range = deferred_draw.paint_range.clone();
                self.reuse_paint(range.clone());
                // Riseonly 0080: the cached views inside the replayed draw replayed with it.
                let replayed = paint_start.clone()..self.paint_index();
                // Riseonly 0113: a deferred draw keeps no holes of its nested views.
                crate::view::rebase_nested_paints(&range, &replayed, false, self);
                // Riseonly 0098: holes of the frame just drawn inside it move with it.
                if let Some(splice) = splice.as_deref_mut() {
                    splice.map.paint.push(SplicePass::whole(range, paint_start.clone()));
                }
            } else {
                // Riseonly 0096: a range of another frame cannot replay; the draw shows nothing
                // for this frame and its view renders it fresh in the next one.
                self.note_stale_paint_range("deferred", deferred_draw.current_view, &deferred_draw.paint_range);
                cx.notify(deferred_draw.current_view);
            }
            let paint_end = self.paint_index();
            deferred_draw.paint_range = paint_start..paint_end;
        }
        self.next_frame.deferred_draws = deferred_draws;
        self.element_id_stack.clear();
    }

    fn deferred_draw_traversal_order(&mut self) -> SmallVec<[usize; 8]> {
        let deferred_count = self.next_frame.deferred_draws.len();
        let mut sorted_indices = (0..deferred_count).collect::<SmallVec<[_; 8]>>();
        sorted_indices.sort_by_key(|ix| self.next_frame.deferred_draws[*ix].priority);
        sorted_indices
    }

    pub(crate) fn prepaint_index(&self) -> PrepaintStateIndex {
        PrepaintStateIndex {
            hitboxes_index: self.next_frame.hitboxes.len(),
            tooltips_index: self.next_frame.tooltip_requests.len(),
            deferred_draws_index: self.next_frame.deferred_draws.len(),
            dispatch_tree_index: self.next_frame.dispatch_tree.len(),
            accessed_element_states_index: self.next_frame.accessed_element_states.len(),
            line_layout_index: self.text_system.layout_index(),
            move_listeners_index: self.next_frame.move_listeners.len(),
            moved_views_index: self.next_frame.moved_views.len(),
        }
    }

    /// Riseonly 0111: hitbox ids never repeat, so a pair whose hitbox left the frame can never be
    /// under the mouse again. Dropped once per frame, the set stays bounded by the live hitboxes
    /// that were drawn under the mouse, even when the modality never changes (mouse-only use).
    /// A cached view replays its hitboxes with the same ids, so its pairs survive reuse.
    /// Riseonly 0126: a scroll under a still mouse moves cached rows without a mouse event; a row
    /// replayed with the hover it rendered under the old position renders again.
    fn recheck_moved_hover(&mut self, cx: &mut App) {
        let mouse = self.mouse_position;
        let mut stale: SmallVec<[EntityId; 4]> = SmallVec::new();
        for (entity_id, bounds) in &self.rendered_frame.moved_views {
            let inside = bounds.contains(&mouse);
            if self.mouse_inside_at_render.get(entity_id).copied() != Some(inside) {
                self.mouse_inside_at_render.insert(*entity_id, inside);
                stale.push(*entity_id);
            }
        }
        if self.mouse_inside_at_render.len() > 4 * self.rendered_frame.cached_views.len() + 64 {
            let live: FxHashSet<EntityId> =
                self.rendered_frame.cached_views.iter().map(|(entity_id, _)| *entity_id).collect();
            self.mouse_inside_at_render.retain(|entity_id, _| live.contains(entity_id));
        }
        if !stale.is_empty() {
            cx.defer(move |cx| {
                for entity_id in stale {
                    cx.notify(entity_id);
                }
            });
        }
    }

    fn retain_live_hover_modality_readers(&mut self) {
        let mut readers = self.hover_modality_readers.borrow_mut();
        if readers.is_empty() {
            return;
        }
        let hitboxes = &self.rendered_frame.hitboxes;
        // The set is a handful of pairs under the mouse; a scan beats hashing every hitbox.
        if readers.len() <= 16 {
            readers.retain(|(_, id)| hitboxes.iter().any(|hitbox| hitbox.id == *id));
        } else {
            let live: FxHashSet<HitboxId> = hitboxes.iter().map(|hitbox| hitbox.id).collect();
            readers.retain(|(_, id)| live.contains(id));
        }
    }

    pub(crate) fn reuse_prepaint(&mut self, range: Range<PrepaintStateIndex>) {
        self.next_frame.move_listeners.extend(
            self.rendered_frame.move_listeners
                [range.start.move_listeners_index..range.end.move_listeners_index]
                .iter()
                .cloned(),
        );
        let hidden = self.hidden_depth > 0;
        // Riseonly 0058: a replayed hitbox takes the layer of the pass replaying it. A range holds
        // only hitboxes of one pass (deferred draws prepaint in their own ranges), so the layer it
        // was recorded on is stale once a cached view first drawn in an overlay replays in the
        // root pass: it would stay above every root hitbox and swallow the mouse there.
        let layer = self.hitbox_layer;
        self.next_frame.hitboxes.extend(
            self.rendered_frame.hitboxes[range.start.hitboxes_index..range.end.hitboxes_index]
                .iter()
                .cloned()
                .map(|mut hitbox| {
                    hitbox.hidden = hidden;
                    hitbox.layer = layer;
                    hitbox
                }),
        );
        self.next_frame.tooltip_requests.extend(
            self.rendered_frame.tooltip_requests
                [range.start.tooltips_index..range.end.tooltips_index]
                .iter_mut()
                .map(|request| request.take()),
        );
        self.next_frame.accessed_element_states.extend(
            self.rendered_frame.accessed_element_states[range.start.accessed_element_states_index
                ..range.end.accessed_element_states_index]
                .iter()
                .map(|(id, type_id)| (id.clone(), *type_id)),
        );
        self.text_system
            .reuse_layouts(range.start.line_layout_index..range.end.line_layout_index);

        let reused_subtree = self.next_frame.dispatch_tree.reuse_subtree(
            range.start.dispatch_tree_index..range.end.dispatch_tree_index,
            &mut self.rendered_frame.dispatch_tree,
            self.focus,
        );

        if reused_subtree.contains_focus() {
            self.next_frame.focus = self.focus;
        }

        let deferred_from = range.start.deferred_draws_index;
        self.next_frame.deferred_draws.extend(
            self.rendered_frame.deferred_draws
                [range.start.deferred_draws_index..range.end.deferred_draws_index]
                .iter()
                .enumerate()
                .map(|(offset, deferred_draw)| DeferredDraw {
                    copied_from: Some(deferred_from + offset),
                    reads: deferred_draw.reads.clone(),
                    current_view: deferred_draw.current_view,
                    parent_node: reused_subtree.refresh_node_id(deferred_draw.parent_node),
                    element_id_stack: deferred_draw.element_id_stack.clone(),
                    text_style_stack: deferred_draw.text_style_stack.clone(),
                    content_mask: deferred_draw.content_mask,
                    rem_size: deferred_draw.rem_size,
                    priority: deferred_draw.priority,
                    element: None,
                    absolute_offset: deferred_draw.absolute_offset,
                    prepaint_range: deferred_draw.prepaint_range.clone(),
                    paint_range: deferred_draw.paint_range.clone(),
                }),
        );
    }

    /// Replays a prepaint range of the frame just drawn moved by `offset` and returns the move
    /// listeners it holds, for the cached view to call (Riseonly 0041). Element states keep the
    /// positions they recorded: a moved replay only runs for a view without input handlers,
    /// tooltips or deferred draws, and with mouse listeners only while the mouse is away (0044).
    pub(crate) fn reuse_moved_prepaint(
        &mut self,
        range: Range<PrepaintStateIndex>,
        offset: Point<Pixels>,
        mask_offset: Point<Pixels>,
        reclip: Option<(Bounds<Pixels>, Bounds<Pixels>)>,
    ) -> Vec<MoveListener> {
        let hitboxes_start = self.next_frame.hitboxes.len();
        let move_listeners_start = self.next_frame.move_listeners.len();
        self.reuse_prepaint(range);
        for hitbox in &mut self.next_frame.hitboxes[hitboxes_start..] {
            hitbox.bounds.origin = hitbox.bounds.origin + offset;
            hitbox.content_mask.bounds = match reclip {
                // Riseonly 0099: a view under its own clip boundary keeps the mask around it
                // where the frame has it: what the old one cut opens and the new one cuts.
                Some((old_mask, new_mask)) => {
                    reopened_mask(hitbox.content_mask.bounds, old_mask, offset).intersect(&new_mask)
                }
                None => Bounds {
                    origin: hitbox.content_mask.bounds.origin + offset,
                    size: hitbox.content_mask.bounds.size,
                },
            };
        }
        for listener in &mut self.next_frame.move_listeners[move_listeners_start..] {
            listener.offset = listener.offset + offset;
            listener.mask_offset = listener.mask_offset + mask_offset;
        }
        self.next_frame.move_listeners[move_listeners_start..].to_vec()
    }

    /// Whether the prepaint and paint ranges of the frame just drawn may replay at another origin
    /// (Riseonly 0041): no tooltip or deferred draw placed by the old origin, no input handler
    /// holding it. Mouse listeners may (Riseonly 0044, see [`Self::paint_range_listens`]).
    pub(crate) fn ranges_replay_moved(
        &self,
        prepaint: &Range<PrepaintStateIndex>,
        paint: &Range<PaintIndex>,
    ) -> bool {
        prepaint.start.tooltips_index == prepaint.end.tooltips_index
            && prepaint.start.deferred_draws_index == prepaint.end.deferred_draws_index
            && paint.start.input_handlers_index == paint.end.input_handlers_index
    }

    /// Whether a paint range registered mouse listeners (Riseonly 0044): replayed moved, they keep
    /// the old origin in the hitbox copies and element states they read, so the view renders
    /// whenever the mouse is over it.
    pub(crate) fn paint_range_listens(&self, paint: &Range<PaintIndex>) -> bool {
        paint.start.mouse_listeners_index != paint.end.mouse_listeners_index
    }

    /// Whether a cached view drew and hit-tests something in the frame just drawn, all of it cut away
    /// whole by the clip it recorded (Riseonly 0074). Such a view keeps the outer mask in those clips,
    /// so a replay that shows it again (moved, or under a grown mask) would still show nothing.
    pub(crate) fn ranges_wholly_hidden(
        &self,
        prepaint: &Range<PrepaintStateIndex>,
        paint: &Range<PaintIndex>,
    ) -> bool {
        let hitboxes = &self.rendered_frame.hitboxes[prepaint.start.hitboxes_index..prepaint.end.hitboxes_index];
        let mut any = !hitboxes.is_empty();
        if !hitboxes.iter().all(|hitbox| !hitbox.bounds.intersects(&hitbox.content_mask.bounds)) {
            return false;
        }
        for operation in &self.rendered_frame.scene.paint_operations[paint.start.scene_index..paint.end.scene_index] {
            if let crate::scene::PaintOperation::Primitive(primitive) = operation {
                if primitive.painted_bounds().intersects(&primitive.content_mask().bounds) {
                    return false;
                }
                any = true;
            }
        }
        any
    }

    /// Whether a cached view showed nothing in the frame just drawn and shows nothing once moved by
    /// `offset` under `mask` (Riseonly 0067): every primitive and hitbox was cut away whole by the
    /// clip it recorded, and moved it lies wholly outside `mask`. A pager neighbour off its clip
    /// while the page slides in is one. Its replay draws the same nothing at any offset, whole
    /// device pixels or not, and hit-tests nothing.
    pub(crate) fn ranges_hidden_by_mask(
        &self,
        prepaint: &Range<PrepaintStateIndex>,
        paint: &Range<PaintIndex>,
        mask: Bounds<Pixels>,
        offset: Point<Pixels>,
    ) -> bool {
        let hitboxes_hidden = self.rendered_frame.hitboxes
            [prepaint.start.hitboxes_index..prepaint.end.hitboxes_index]
            .iter()
            .all(|hitbox| {
                let moved = Bounds { origin: hitbox.bounds.origin + offset, size: hitbox.bounds.size };
                !hitbox.bounds.intersects(&hitbox.content_mask.bounds) && !moved.intersects(&mask)
            });
        if !hitboxes_hidden {
            return false;
        }
        let scale = self.scale_factor();
        let scaled_mask = mask.scale(scale);
        let scaled_offset = offset.scale(scale);
        self.rendered_frame.scene.paint_operations[paint.start.scene_index..paint.end.scene_index]
            .iter()
            .all(|operation| match operation {
                crate::scene::PaintOperation::Primitive(primitive) => {
                    let painted = primitive.painted_bounds();
                    let moved = Bounds { origin: painted.origin + scaled_offset, size: painted.size };
                    !painted.intersects(&primitive.content_mask().bounds) && !moved.intersects(&scaled_mask)
                }
                _ => true,
            })
    }

    /// Whether nothing a cached view drew or hit-tests in the frame just drawn was cut by `mask`,
    /// and nothing would be once moved by `offset` (Riseonly 0041). Then the clips its primitives
    /// and hitboxes recorded may move along although `mask` stays where it is.
    pub(crate) fn ranges_clear_of_mask(
        &self,
        prepaint: &Range<PrepaintStateIndex>,
        paint: &Range<PaintIndex>,
        mask: Bounds<Pixels>,
        offset: Point<Pixels>,
    ) -> bool {
        // Strictly inside before: a shape touching the edge may have been cut there.
        fn clear(shown: [f32; 4], mask: [f32; 4], dx: f32, dy: f32) -> bool {
            let [left, top, right, bottom] = shown;
            let [mask_left, mask_top, mask_right, mask_bottom] = mask;
            left > mask_left
                && top > mask_top
                && right < mask_right
                && bottom < mask_bottom
                && left + dx >= mask_left
                && top + dy >= mask_top
                && right + dx <= mask_right
                && bottom + dy <= mask_bottom
        }
        let (dx, dy) = (f32::from(offset.x), f32::from(offset.y));
        let edges = |bounds: &Bounds<Pixels>| {
            [
                f32::from(bounds.left()),
                f32::from(bounds.top()),
                f32::from(bounds.right()),
                f32::from(bounds.bottom()),
            ]
        };
        let pixels_mask = edges(&mask);
        let hitboxes_clear = self.rendered_frame.hitboxes
            [prepaint.start.hitboxes_index..prepaint.end.hitboxes_index]
            .iter()
            .all(|hitbox| {
                let shown = hitbox.bounds.intersect(&hitbox.content_mask.bounds);
                shown.is_empty() || clear(edges(&shown), pixels_mask, dx, dy)
            });
        if !hitboxes_clear {
            return false;
        }
        let scale = self.scale_factor();
        let scaled_mask = pixels_mask.map(|edge| edge * scale);
        self.rendered_frame.scene.paint_operations[paint.start.scene_index..paint.end.scene_index]
            .iter()
            .all(|operation| match operation {
                crate::scene::PaintOperation::Primitive(primitive) => {
                    let shown = primitive
                        .painted_bounds()
                        .intersect(&primitive.content_mask().bounds);
                    let edges = [
                        shown.left().0,
                        shown.top().0,
                        shown.right().0,
                        shown.bottom().0,
                    ];
                    shown.is_empty() || clear(edges, scaled_mask, dx * scale, dy * scale)
                }
                _ => true,
            })
    }

    /// Whether a prepaint range of the frame just drawn holds deferred draws (Riseonly 0034).
    pub(crate) fn prepaint_range_defers(&self, range: &Range<PrepaintStateIndex>) -> bool {
        range.start.deferred_draws_index != range.end.deferred_draws_index
    }

    /// Takes the paint a cached view recorded in the frame just drawn out of that frame, for a frame
    /// in which a hidden div skips the view's paint (Riseonly 0034). Input handlers stay behind: a
    /// hidden view holds no focus, and its field takes the handler back when focused and drawn.
    pub(crate) fn park_paint(&mut self, range: Range<PaintIndex>) -> ParkedPaint {
        let image_drops = self.image_drops;
        let frame = &mut self.rendered_frame;
        ParkedPaint {
            scene: frame.scene.paint_operations[range.start.scene_index..range.end.scene_index]
                .to_vec(),
            mouse_listeners: frame.mouse_listeners
                [range.start.mouse_listeners_index..range.end.mouse_listeners_index]
                .iter_mut()
                .map(|listener| listener.take())
                .collect(),
            cursor_styles: frame.cursor_styles
                [range.start.cursor_styles_index..range.end.cursor_styles_index]
                .to_vec(),
            accessed_element_states: frame.accessed_element_states
                [range.start.accessed_element_states_index..range.end.accessed_element_states_index]
                .to_vec(),
            tab_stops: frame.tab_stops.insertion_history
                [range.start.tab_handle_index..range.end.tab_handle_index]
                .to_vec(),
            image_drops,
        }
    }

    /// Whether a parked paint still draws what it recorded: no image left the sprite atlas since.
    pub(crate) fn parked_paint_valid(&self, parked: &ParkedPaint) -> bool {
        parked.image_drops == self.image_drops
    }

    /// Hands every element state of type `type_id` whose key the replayed prepaint range `replayed`
    /// copied to `update`, with this window (Riseonly 0045): the cached views nested in a view that
    /// replayed learn where their own ranges went, instead of holding ranges of an older frame.
    /// Riseonly 0109: the key goes along, so a nested view can name itself as a reader.
    pub(crate) fn update_replayed_states(
        &mut self,
        replayed: &Range<PrepaintStateIndex>,
        type_id: TypeId,
        mut update: impl FnMut(&GlobalElementId, &mut dyn Any, &mut Self),
    ) {
        let keys = self.next_frame.accessed_element_states[replayed
            .start
            .accessed_element_states_index
            ..replayed.end.accessed_element_states_index]
            .iter()
            .filter(|(_, state_type)| *state_type == type_id)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(mut state) = self.rendered_frame.element_states.remove(&key) {
                update(&key.0, &mut *state.inner, self);
                self.rendered_frame.element_states.insert(key, state);
            }
        }
    }

    /// Hands every element state of type `type_id` whose key the replayed paint range `replayed`
    /// copied to `update`, with this window (Riseonly 0063): the cached views nested in a view that
    /// replayed its paint learn where their paint went, as 0045 does for the prepaint.
    pub(crate) fn update_replayed_paint_states(
        &mut self,
        replayed: &Range<PaintIndex>,
        type_id: TypeId,
        mut update: impl FnMut(&GlobalElementId, &mut dyn Any, &mut Self),
    ) {
        let keys = self.next_frame.accessed_element_states[replayed
            .start
            .accessed_element_states_index
            ..replayed.end.accessed_element_states_index]
            .iter()
            .filter(|(_, state_type)| *state_type == type_id)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(mut state) = self.rendered_frame.element_states.remove(&key) {
                update(&key.0, &mut *state.inner, self);
                self.rendered_frame.element_states.insert(key, state);
            }
        }
    }

    /// Keeps the element states a parked paint reads alive through a frame that skips it.
    pub(crate) fn keep_parked_states(&mut self, parked: &ParkedPaint) {
        self.next_frame
            .accessed_element_states
            .extend(parked.accessed_element_states.iter().cloned());
    }

    /// Replays a parked paint into the frame being drawn (Riseonly 0034).
    pub(crate) fn replay_parked_paint(&mut self, parked: ParkedPaint) {
        self.next_frame.cursor_styles.extend(parked.cursor_styles);
        self.next_frame.mouse_listeners.extend(parked.mouse_listeners);
        self.next_frame
            .accessed_element_states
            .extend(parked.accessed_element_states);
        self.next_frame.tab_stops.replay(&parked.tab_stops);
        self.next_frame.scene.replay_operations(&parked.scene);
    }

    /// Riseonly 0096: whether `range` still indexes the paint of the frame just drawn, every list
    /// it slices included. A cached view a frame behind replays only such a range: one recorded
    /// against another frame (X1986: the gift shop sheet closing) aborted the app on the slice.
    pub(crate) fn paint_range_in_frame(&self, range: &Range<PaintIndex>) -> bool {
        let frame = &self.rendered_frame;
        range.start.le(&range.end)
            && range.end.scene_index <= frame.scene.len()
            && range.end.mouse_listeners_index <= frame.mouse_listeners.len()
            && range.end.input_handlers_index <= frame.input_handlers.len()
            && range.end.cursor_styles_index <= frame.cursor_styles.len()
            && range.end.accessed_element_states_index <= frame.accessed_element_states.len()
            && range.end.tab_handle_index <= frame.tab_stops.insertion_history.len()
    }

    /// Riseonly 0096: names a paint range that does not index the frame just drawn, once per
    /// frame and source, so the log tells which replay kept a range of another frame.
    pub(crate) fn note_stale_paint_range(&self, source: &str, view_id: EntityId, range: &Range<PaintIndex>) {
        log::warn!(
            "gpui 0096 stale paint range source={source} view={} scene={}..{} of {} listeners={} of {} frame={}",
            crate::profiler::view_type_name(view_id),
            range.start.scene_index,
            range.end.scene_index,
            self.rendered_frame.scene.len(),
            range.end.mouse_listeners_index,
            self.rendered_frame.mouse_listeners.len(),
            self.draw_count,
        );
    }

    pub(crate) fn paint_index(&self) -> PaintIndex {
        PaintIndex {
            scene_index: self.next_frame.scene.len(),
            mouse_listeners_index: self.next_frame.mouse_listeners.len(),
            input_handlers_index: self.next_frame.input_handlers.len(),
            cursor_styles_index: self.next_frame.cursor_styles.len(),
            accessed_element_states_index: self.next_frame.accessed_element_states.len(),
            tab_handle_index: self.next_frame.tab_stops.paint_index(),
            line_layout_index: self.text_system.layout_index(),
        }
    }

    pub(crate) fn reuse_paint(&mut self, range: Range<PaintIndex>) {
        self.reuse_paint_at(range, PaintReplay::Same);
    }

    /// Replays a paint range of the frame just drawn in place, painted under the opacity of
    /// `painted`, as the paint under the opacity of `now` (Riseonly 0065): a cached view inside a
    /// fade keeps its frame and takes the fade. False, replaying nothing, when either opacity is
    /// zero: a paint at zero opacity dropped its transparent primitives and cannot fade back in.
    pub(crate) fn reuse_faded_paint(
        &mut self,
        range: Range<PaintIndex>,
        painted: &PaintContext,
        now: &PaintContext,
    ) -> bool {
        if painted.opacity == now.opacity {
            self.reuse_paint_at(range, PaintReplay::Same);
            true
        } else if painted.opacity > 0.0 && now.opacity > 0.0 {
            self.reuse_paint_at(range, PaintReplay::Faded(now.opacity / painted.opacity));
            true
        } else {
            false
        }
    }

    /// Whether a paint under `painted` shows at the opacity a paint under `now` would (Riseonly 0065).
    pub(crate) fn same_opacity(painted: &PaintContext, now: &PaintContext) -> bool {
        painted.opacity == now.opacity
    }

    /// What the paint running now is painted under (Riseonly 0053).
    pub(crate) fn paint_context(&self) -> PaintContext {
        PaintContext {
            transformation: self.transformation(),
            transformation_mask: self.transformation_stack.last().map(|(_, mask, _)| mask.clone()),
            opacity: self.element_opacity,
        }
    }

    /// Replays a paint range of the frame just drawn, painted under `painted`, as the paint under
    /// `now` (Riseonly 0053): a cached view under a zoom flight keeps its frame while the matrix
    /// and the clip around it move. False, replaying nothing, when the replay would not be what a
    /// paint draws: a turn or a mirror, a paint outside any transformation, an opacity that was or
    /// is zero, or a clip that now shows what the old one cut away (the scene dropped those
    /// primitives). Another nonzero opacity fades the replayed colours (Riseonly 0059).
    pub(crate) fn reuse_transformed_paint(
        &mut self,
        range: Range<PaintIndex>,
        painted: &PaintContext,
        now: &PaintContext,
    ) -> bool {
        match self.transformed_replay(painted, now) {
            Some(replay) => {
                self.reuse_paint_at(range, PaintReplay::Transformed(replay));
                true
            }
            None => false,
        }
    }

    fn transformed_replay(&self, painted: &PaintContext, now: &PaintContext) -> Option<TransformedReplay> {
        fn scales_only(matrix: &TransformationMatrix) -> bool {
            let [[scale_x, shear_x], [shear_y, scale_y]] = matrix.rotation_scale;
            shear_x == 0.0 && shear_y == 0.0 && scale_x > 0.0 && scale_y > 0.0
        }
        // Riseonly 0059: a modal appears scaling and fading at once; the replay fades its colours
        // by the ratio instead of rendering every cached view under it in every frame (X1275).
        // A paint at zero opacity dropped its transparent primitives, so it cannot fade back in.
        let fade = if painted.opacity == now.opacity {
            1.0
        } else if painted.opacity > 0.0 && now.opacity > 0.0 {
            now.opacity / painted.opacity
        } else {
            return None;
        };
        if !scales_only(&painted.transformation)
            || !scales_only(&now.transformation)
        {
            return None;
        }
        let (Some(old_mask), Some(new_mask)) = (&painted.transformation_mask, &now.transformation_mask)
        else {
            return None;
        };
        let (old, new) = (&painted.transformation, &now.transformation);
        let scale_x = new.rotation_scale[0][0] / old.rotation_scale[0][0];
        let scale_y = new.rotation_scale[1][1] / old.rotation_scale[1][1];
        let move_x = new.translation[0] - scale_x * old.translation[0];
        let move_y = new.translation[1] - scale_y * old.translation[1];
        let delta = TransformationMatrix {
            rotation_scale: [[scale_x, 0.0], [0.0, scale_y]],
            translation: [move_x, move_y],
        };
        let scale = self.scale_factor();
        let edges = |bounds: &Bounds<Pixels>| {
            [
                bounds.left().0 * scale,
                bounds.top().0 * scale,
                bounds.right().0 * scale,
                bounds.bottom().0 * scale,
            ]
        };
        let within = |inner: [f32; 4], outer: [f32; 4]| {
            const SLACK: f32 = 0.01;
            inner[0] >= outer[0] - SLACK
                && inner[1] >= outer[1] - SLACK
                && inner[2] <= outer[2] + SLACK
                && inner[3] <= outer[3] + SLACK
        };
        let (old_edges, new_edges) = (edges(&old_mask.bounds), edges(&new_mask.bounds));
        // why: the scene drops a primitive its clip hides, so the new clip may only show less:
        // on screen for what stays untransformed, and through the delta for what follows it.
        let new_in_old_space = [
            (new_edges[0] - move_x) / scale_x,
            (new_edges[1] - move_y) / scale_y,
            (new_edges[2] - move_x) / scale_x,
            (new_edges[3] - move_y) / scale_y,
        ];
        if !within(new_edges, old_edges) || !within(new_in_old_space, old_edges) {
            return None;
        }
        Some(TransformedReplay {
            delta,
            fade,
            path_mask: new_mask.scale(scale),
            snapped_mask: ContentMask {
                bounds: self.cover_bounds(new_mask.bounds),
            },
        })
    }

    /// Replays a paint range of the frame just drawn moved by `offset` (Riseonly 0041).
    pub(crate) fn reuse_moved_paint(&mut self, range: Range<PaintIndex>, offset: Point<Pixels>) {
        let offset = offset.scale(self.scale_factor());
        self.reuse_paint_at(range, PaintReplay::Moved(offset));
    }

    /// Replays a paint range of the frame just drawn moved by `offset`, painted under the opacity
    /// of `painted`, as the paint under the opacity of `now` (Riseonly 0085). False, replaying
    /// nothing, when either opacity is zero, as [`Self::reuse_faded_paint`].
    pub(crate) fn reuse_moved_faded_paint(
        &mut self,
        range: Range<PaintIndex>,
        offset: Point<Pixels>,
        painted: &PaintContext,
        now: &PaintContext,
    ) -> bool {
        if painted.opacity <= 0.0 || now.opacity <= 0.0 {
            return false;
        }
        let offset = offset.scale(self.scale_factor());
        let factor = now.opacity / painted.opacity;
        self.reuse_paint_at(range, PaintReplay::MovedFaded(offset, factor));
        true
    }

    fn reuse_paint_at(&mut self, range: Range<PaintIndex>, replay: PaintReplay) {
        self.next_frame.cursor_styles.extend(
            self.rendered_frame.cursor_styles
                [range.start.cursor_styles_index..range.end.cursor_styles_index]
                .iter()
                .cloned(),
        );
        self.next_frame.input_handlers.extend(
            self.rendered_frame.input_handlers
                [range.start.input_handlers_index..range.end.input_handlers_index]
                .iter_mut()
                .map(|handler| handler.take()),
        );
        self.next_frame.mouse_listeners.extend(
            self.rendered_frame.mouse_listeners
                [range.start.mouse_listeners_index..range.end.mouse_listeners_index]
                .iter_mut()
                .map(|listener| listener.take()),
        );
        self.next_frame.accessed_element_states.extend(
            self.rendered_frame.accessed_element_states[range.start.accessed_element_states_index
                ..range.end.accessed_element_states_index]
                .iter()
                .map(|(id, type_id)| (id.clone(), *type_id)),
        );
        self.next_frame.tab_stops.replay(
            &self.rendered_frame.tab_stops.insertion_history
                [range.start.tab_handle_index..range.end.tab_handle_index],
        );

        self.text_system
            .reuse_layouts(range.start.line_layout_index..range.end.line_layout_index);
        let scene_range = range.start.scene_index..range.end.scene_index;
        match replay {
            PaintReplay::Moved(offset) => {
                self.next_frame
                    .scene
                    .replay_moved(scene_range, &self.rendered_frame.scene, offset)
            }
            PaintReplay::Transformed(replay) => {
                self.next_frame
                    .scene
                    .replay_transformed(scene_range, &self.rendered_frame.scene, &replay)
            }
            PaintReplay::Faded(factor) => {
                self.next_frame
                    .scene
                    .replay_faded(scene_range, &self.rendered_frame.scene, factor)
            }
            PaintReplay::MovedFaded(offset, factor) => self.next_frame.scene.replay_moved_faded(
                scene_range,
                &self.rendered_frame.scene,
                offset,
                factor,
            ),
            PaintReplay::Same => self
                .next_frame
                .scene
                .replay(scene_range, &self.rendered_frame.scene),
        }
    }

    /// Registers a callback for the element being prepainted that reads window positions (a frame
    /// reader, a visibility probe). A cached view opted in with `replay_when_moved` that only
    /// changed its origin replays its last frame instead of rendering, and calls the callback with
    /// the offset from the bounds this prepaint saw and the offset by which the content mask around
    /// it moved, zero when the mask stayed (Riseonly 0041, 0043).
    ///
    /// This method should only be called as part of the prepaint phase of element drawing.
    pub fn on_replayed_move(
        &mut self,
        callback: impl Fn(Point<Pixels>, Point<Pixels>, &mut Window, &mut App) + 'static,
    ) {
        self.invalidator.debug_assert_prepaint();
        self.next_frame.move_listeners.push(MoveListener {
            callback: Rc::new(callback),
            offset: Point::default(),
            mask_offset: Point::default(),
        });
    }

    /// Push a text style onto the stack, and call a function with that style active.
    /// Use [`Window::text_style`] to get the current, combined text style. This method
    /// should only be called as part of element drawing.
    pub fn with_text_style<F, R>(&mut self, style: Option<TextStyleRefinement>, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.invalidator.debug_assert_paint_or_prepaint();
        if let Some(style) = style {
            self.text_style_stack.push(style);
            let result = f(self);
            self.text_style_stack.pop();
            result
        } else {
            f(self)
        }
    }

    /// Updates the cursor style at the platform level. This method should only be called
    /// during the paint phase of element drawing.
    pub fn set_cursor_style(&mut self, style: CursorStyle, hitbox: &Hitbox) {
        self.invalidator.debug_assert_paint();
        self.next_frame.cursor_styles.push(CursorStyleRequest {
            hitbox_id: Some(hitbox.id),
            style,
        });
    }

    /// Updates the cursor style for the entire window at the platform level. A cursor
    /// style using this method will have precedence over any cursor style set using
    /// `set_cursor_style`. This method should only be called during the paint
    /// phase of element drawing.
    pub fn set_window_cursor_style(&mut self, style: CursorStyle) {
        self.invalidator.debug_assert_paint();
        self.next_frame.cursor_styles.push(CursorStyleRequest {
            hitbox_id: None,
            style,
        })
    }

    /// Sets a tooltip to be rendered for the upcoming frame. This method should only be called
    /// during the paint phase of element drawing.
    pub fn set_tooltip(&mut self, tooltip: AnyTooltip) -> TooltipId {
        self.invalidator.debug_assert_prepaint();
        let id = TooltipId(post_inc(&mut self.next_tooltip_id.0));
        self.next_frame
            .tooltip_requests
            .push(Some(TooltipRequest { id, tooltip }));
        id
    }

    /// Invoke the given function with the given content mask after intersecting it
    /// with the current mask. This method should only be called during element drawing.
    // This function is called in a highly recursive manner in editor
    // prepainting, make sure its inlined to reduce the stack burden
    #[inline]
    pub fn with_content_mask<R>(
        &mut self,
        mask: Option<ContentMask<Pixels>>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.invalidator.debug_assert_paint_or_prepaint();
        if let Some(mask) = mask {
            let paint_mask = mask.intersect(&self.paint_mask());
            // Riseonly 0104: under a transformation the content mask stays the one around it, so a
            // scroll clip or a sheet edge inside a scaling modal cut nothing; the scene cuts by it.
            let scene_clip = self.transformed_mask_clip(&mask);
            let mask = mask.intersect(&self.content_mask());
            self.content_mask_stack.push(mask);
            self.paint_mask_stack.push(paint_mask);
            if let Some(clip) = scene_clip {
                self.next_frame.scene.push_clip(clip);
            }
            let result = f(self);
            if scene_clip.is_some() {
                self.next_frame.scene.pop_clip();
            }
            self.content_mask_stack.pop();
            self.paint_mask_stack.pop();
            result
        } else {
            f(self)
        }
    }

    /// Whether the paint running now is under [`Self::with_transformation`] (Riseonly 0104).
    pub(crate) fn under_transformation(&self) -> bool {
        !self.transformation_stack.is_empty()
    }

    /// The mask pushed now as the scene clips it on screen under the active transformation
    /// (Riseonly 0104), snapped out to device pixels. None outside paint, outside any
    /// transformation, or under one that turns or mirrors, where a box does not map to a box.
    fn transformed_mask_clip(&self, mask: &ContentMask<Pixels>) -> Option<Bounds<ScaledPixels>> {
        let (matrix, _, _) = self.transformation_stack.last()?;
        if !matches!(self.invalidator.inner.borrow().draw_phase, DrawPhase::Paint) {
            return None;
        }
        let [[scale_x, shear_x], [shear_y, scale_y]] = matrix.rotation_scale;
        if shear_x != 0.0 || shear_y != 0.0 || scale_x <= 0.0 || scale_y <= 0.0 {
            return None;
        }
        let [move_x, move_y] = matrix.translation;
        let scale = self.scale_factor();
        let left = (mask.bounds.left().0 * scale * scale_x + move_x).floor();
        let top = (mask.bounds.top().0 * scale * scale_y + move_y).floor();
        let right = (mask.bounds.right().0 * scale * scale_x + move_x).ceil().max(left);
        let bottom = (mask.bounds.bottom().0 * scale * scale_y + move_y).ceil().max(top);
        Some(Bounds::from_corners(
            point(ScaledPixels(left), ScaledPixels(top)),
            point(ScaledPixels(right), ScaledPixels(bottom)),
        ))
    }

    /// Updates the global element offset relative to the current offset. This is used to implement
    /// scrolling. This method should only be called during the prepaint phase of element drawing.
    pub fn with_element_offset<R>(
        &mut self,
        offset: Point<Pixels>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.invalidator.debug_assert_prepaint();

        if offset.is_zero() {
            return f(self);
        };

        let abs_offset = self.element_offset() + offset;
        self.with_absolute_element_offset(abs_offset, f)
    }

    /// Updates the global element offset based on the given offset. This is used to implement
    /// drag handles and other manual painting of elements. This method should only be called during
    /// the prepaint phase of element drawing.
    pub fn with_absolute_element_offset<R>(
        &mut self,
        offset: Point<Pixels>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.invalidator.debug_assert_prepaint();
        self.element_offset_stack.push(offset);
        let result = f(self);
        self.element_offset_stack.pop();
        result
    }

    /// Paints what `f` paints through `transformation`, given in scaled pixels of the window and
    /// composed with the transformation already active. Layout and hit testing stay
    /// untransformed. Quads and paths become transformed paths; glyphs, svgs, images and emoji
    /// carry the matrix on their sprite; a surface takes its mapped box when the matrix only
    /// scales and moves; shadows and underlines stay untransformed. The
    /// content mask active here clips everything `f` paints; a mask pushed inside `f` clips
    /// through the matrix as a scene clip when it only scales and moves (Riseonly 0104), and is
    /// ignored under a turn or a mirror.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn with_transformation<R>(
        &mut self,
        transformation: TransformationMatrix,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.invalidator.debug_assert_paint();
        let mask = self.content_mask();
        let paint_mask = self.paint_mask();
        let composed = self.transformation().compose(transformation);
        self.transformation_stack.push((composed, mask, paint_mask));
        let result = f(self);
        self.transformation_stack.pop();
        result
    }

    /// The transformation [`Window::with_transformation`] applies to what is painted now, in
    /// scaled pixels of the window.
    pub fn transformation(&self) -> TransformationMatrix {
        self.transformation_stack
            .last()
            .map(|(transformation, _, _)| *transformation)
            .unwrap_or_default()
    }

    /// Multiplies the opacity of everything `f` paints by `opacity`, on top of the opacity
    /// already active. `None` leaves it as it is.
    ///
    /// This method should only be called as part of the prepaint or paint phase of element drawing.
    pub fn with_element_opacity<R>(
        &mut self,
        opacity: Option<f32>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.invalidator.debug_assert_paint_or_prepaint();

        let Some(opacity) = opacity else {
            return f(self);
        };

        let previous_opacity = self.element_opacity;
        self.element_opacity = previous_opacity * opacity;
        let result = f(self);
        self.element_opacity = previous_opacity;
        result
    }

    /// Marks every glyph `f` paints with `key` (Riseonly 0056), so [`Self::swap_glyph_colours`]
    /// can recolour them in the frame on screen. Zero names nothing.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn with_glyph_swap_key<R>(&mut self, key: u32, f: impl FnOnce(&mut Self) -> R) -> R {
        self.invalidator.debug_assert_paint();
        let previous = self.glyph_swap_key;
        self.glyph_swap_key = key;
        let result = f(self);
        self.glyph_swap_key = previous;
        result
    }

    /// Gives every glyph of the frame on screen painted under `key` the colour `colour` returns
    /// for its place across the span of those glyphs (0 at the left edge, 1 at the right), keeping
    /// the alpha it was painted with, and presents (Riseonly 0056): a sweep over a label moves
    /// with a present, without a render, layout or paint of the views around it. False when no
    /// glyph of the frame carries the key; the caller then repaints through its view.
    pub fn swap_glyph_colours(&mut self, key: u32, colour: &dyn Fn(f32) -> Hsla) -> bool {
        let found = self.rendered_frame.scene.swap_glyph_colours(key, colour);
        if found {
            self.needs_present.set(true);
        }
        found
    }

    /// Shows the glyphs painted under the `shown` keys and hides those under the `hidden` keys in
    /// the frame on screen, and presents (Riseonly 0078): a clock painted as stacked digits ticks
    /// without a render, layout or paint of the views around it. False when a key left the frame;
    /// the caller then repaints through its view.
    pub fn swap_glyph_visibility(&mut self, shown: &[u32], hidden: &[u32]) -> bool {
        let found = self.rendered_frame.scene.swap_glyph_visibility(shown, hidden);
        if found {
            self.needs_present.set(true);
        }
        found
    }

    /// Moves the glyphs (emoji included) painted under the sliding key `key` (a glyph swap key
    /// carrying [`crate::GLYPH_SHIFT_KEY_BITS`], given to [`Self::with_glyph_swap_key`]) by `dx`
    /// inside their clip in the frame on screen, and presents when a nonzero move touches a glyph
    /// that shows (Riseonly 0079): a marquee title slides without a render, layout or paint of the
    /// views around it. Pass whole device pixels so every glyph keeps its rasterised subpixel
    /// phase; a zero move only probes. None when the key left the frame; the caller then repaints
    /// through its view. Some(false): clipped out before and after the move.
    pub fn shift_glyphs(&mut self, key: u32, dx: Pixels) -> Option<bool> {
        let shows = self
            .rendered_frame
            .scene
            .shift_glyphs(key, dx.scale(self.scale_factor()))?;
        if shows && dx != Pixels::ZERO {
            self.needs_present.set(true);
        }
        Some(shows)
    }

    /// Names everything `f` paints with the shift key `key` (Riseonly 0084), so
    /// [`Self::shift_group`] and [`Self::move_group`] can move it in the frame on screen. Zero names
    /// nothing. A cached view replayed whole in place around the call keeps the group where it was
    /// moved (Riseonly 0112); one replayed at another origin or under another opacity keeps none,
    /// and the shift then refuses.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn with_shift_group<R>(&mut self, key: u64, f: impl FnOnce(&mut Self) -> R) -> R {
        self.invalidator.debug_assert_paint();
        let start = self.next_frame.scene.paint_operations.len();
        let result = f(self);
        self.next_frame.scene.close_shift_group(key, start);
        result
    }

    /// Moves everything painted under the shift key `key` by `offset` in the frame on screen, clip
    /// included, and presents (Riseonly 0084): a map pin follows the camera without a render, layout
    /// or paint of the views around it. Hitboxes stay where they were painted; the caller renders
    /// once the motion ends. Pass whole device pixels. False when the key left the frame or a
    /// primitive of it was not found; the caller then repaints through its view.
    pub fn shift_group(&mut self, key: u64, offset: Point<Pixels>) -> bool {
        let shifted = self
            .rendered_frame
            .scene
            .shift_group(key, offset.scale(self.scale_factor()));
        if shifted && offset != Point::default() {
            self.needs_present.set(true);
        }
        shifted
    }

    /// Shows everything painted under the shift key `key` scaled by `scale`, moved by
    /// `translation` and faded by `fade` against its paint, and presents (Riseonly 0112): a point
    /// painted at `p` lands at `p * scale + translation`. Each call starts from the paint, not from
    /// the last call, so a fade to zero comes back. The clips stay where they were painted, as a
    /// paint offset inside a fixed mask draws. A chat row folding into the forum rail rides
    /// the open without a render, layout or paint of its view. Scaling takes what a paint under a
    /// transformation records (paths and transformed sprites); hitboxes stay where they were
    /// painted, the caller renders once the motion ends. False when the key left the frame or the
    /// motion cannot show a primitive of it; the caller then repaints through its view.
    pub fn move_group(&mut self, key: u64, scale: f32, translation: Point<Pixels>, fade: f32) -> bool {
        let motion = crate::scene::GroupMotion {
            scale,
            translation: translation.scale(self.scale_factor()),
            fade,
            clip_stays: true,
        };
        let moved = self.rendered_frame.scene.move_group(key, motion);
        if moved {
            self.needs_present.set(true);
        }
        moved
    }

    /// Marks every image `f` paints with `key` (Riseonly 0057), so [`Self::swap_image`] can give
    /// it another picture in the frame on screen. Zero names nothing.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn with_image_swap_key<R>(&mut self, key: u32, f: impl FnOnce(&mut Self) -> R) -> R {
        self.invalidator.debug_assert_paint();
        let previous = self.image_swap_key;
        self.image_swap_key = key;
        let result = f(self);
        self.image_swap_key = previous;
        result
    }

    /// Gives every image of the frame on screen painted under `key` with frame `frame_index` of
    /// `previous` frame `frame_index` of `next` in the same place, and presents (Riseonly 0057): an
    /// animated sticker or GIF plays without rendering, laying out or painting any view. Call it
    /// outside a draw, before `previous` leaves the atlas. False when no image of the frame carries
    /// the key, one shows another picture than `previous`, or the pictures differ in size; the
    /// caller then repaints through its view.
    pub fn swap_image(
        &mut self,
        key: u32,
        previous: &Arc<RenderImage>,
        next: &Arc<RenderImage>,
        frame_index: usize,
    ) -> bool {
        if key == 0 || previous.size(frame_index) != next.size(frame_index) {
            return false;
        }
        let (Some(previous_tile), Some(next_tile)) = (
            self.image_tile(previous, frame_index),
            self.image_tile(next, frame_index),
        ) else {
            return false;
        };
        let found = self
            .rendered_frame
            .scene
            .swap_image_tile(key, previous_tile, next_tile);
        if found {
            self.needs_present.set(true);
        }
        found
    }

    /// Scales the opacity of every image of the frame on screen painted under `key` (see
    /// [`Self::with_image_swap_key`]) by `factor` and presents (Riseonly 0072): a picture fading in
    /// or out costs a present, not a render of its view and of every view above it. Call it
    /// outside a draw. False when no image of the frame carries the key; the caller then repaints
    /// through its view.
    pub fn fade_swapped_image(&mut self, key: u32, factor: f32) -> bool {
        let found = self.rendered_frame.scene.fade_image_sprites(key, factor);
        if found {
            self.needs_present.set(true);
        }
        found
    }

    /// Scales the alpha of every glyph and SVG icon of the frame on screen painted under `key` by
    /// `factor` and presents (Riseonly 0097). SVG icons carry the key only when it is an SVG fade
    /// key (see [`crate::SVG_FADE_KEY_BIT`] and [`Self::with_glyph_swap_key`]): a pulsing icon
    /// costs a present, not a render of its view and of every view above it. Call it outside a
    /// draw. False when no sprite of the frame carries the key; the caller then repaints through
    /// its view. A fade to zero must stay above zero, or the ratio cannot bring it back.
    pub fn fade_swapped_glyphs(&mut self, key: u32, factor: f32) -> bool {
        let found = self.rendered_frame.scene.fade_glyph_sprites(key, factor);
        if found {
            self.needs_present.set(true);
        }
        found
    }

    fn image_tile(&mut self, data: &Arc<RenderImage>, frame_index: usize) -> Option<AtlasTile> {
        let bytes = data.as_bytes(frame_index)?;
        let params = RenderImageParams {
            image_id: data.id,
            frame_index,
        };
        let size = data.size(frame_index);
        self.sprite_atlas
            .get_or_insert_with(&params.into(), &mut || Ok(Some((size, Cow::Borrowed(bytes)))))
            .ok()
            .flatten()
    }

    /// Perform prepaint on child elements in a "retryable" manner, so that any side effects
    /// of prepaints can be discarded before prepainting again. This is used to support autoscroll
    /// where we need to prepaint children to detect the autoscroll bounds, then adjust the
    /// element offset and prepaint again. See [`crate::List`] for an example. This method should only be
    /// called during the prepaint phase of element drawing.
    pub fn transact<T, U>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, U>) -> Result<T, U> {
        self.invalidator.debug_assert_prepaint();
        let index = self.prepaint_index();
        let result = f(self);
        if result.is_err() {
            // A cached view prepainted inside the rolled back pass stored ranges into the tail
            // truncated below; replaying them in a later frame slices past the end (Riseonly 0031).
            let view_state_type = crate::view::view_element_state_type_id();
            let frame = &mut self.next_frame;
            for key in &frame.accessed_element_states[index.accessed_element_states_index..] {
                if key.1 == view_state_type {
                    frame.element_states.remove(key);
                }
            }
            self.next_frame.hitboxes.truncate(index.hitboxes_index);
            self.next_frame
                .tooltip_requests
                .truncate(index.tooltips_index);
            self.next_frame
                .deferred_draws
                .truncate(index.deferred_draws_index);
            self.next_frame
                .dispatch_tree
                .truncate(index.dispatch_tree_index);
            self.next_frame
                .accessed_element_states
                .truncate(index.accessed_element_states_index);
            self.text_system.truncate_layouts(index.line_layout_index);
            self.next_frame
                .move_listeners
                .truncate(index.move_listeners_index);
            self.next_frame
                .moved_views
                .truncate(index.moved_views_index);
        }
        result
    }

    /// When you call this method during [`Element::prepaint`], containing elements will attempt to
    /// scroll to cause the specified bounds to become visible. When they decide to autoscroll, they will call
    /// [`Element::prepaint`] again with a new set of bounds. See [`crate::List`] for an example of an element
    /// that supports this method being called on the elements it contains. This method should only be
    /// called during the prepaint phase of element drawing.
    pub fn request_autoscroll(&mut self, bounds: Bounds<Pixels>) {
        self.invalidator.debug_assert_prepaint();
        self.requested_autoscroll = Some(bounds);
    }

    /// This method can be called from a containing element such as [`crate::List`] to support the autoscroll behavior
    /// described in [`Self::request_autoscroll`].
    pub fn take_autoscroll(&mut self) -> Option<Bounds<Pixels>> {
        self.invalidator.debug_assert_prepaint();
        self.requested_autoscroll.take()
    }

    /// Asynchronously load an asset, if the asset hasn't finished loading this will return None.
    /// Your view will be re-drawn once the asset has finished loading.
    ///
    /// Note that the multiple calls to this method will only result in one `Asset::load` call at a
    /// time.
    pub fn use_asset<A: Asset>(&mut self, source: &A::Source, cx: &mut App) -> Option<A::Output> {
        let (task, is_first) = cx.fetch_asset::<A>(source);
        task.clone().now_or_never().or_else(|| {
            if is_first {
                let entity_id = self.current_view();
                self.spawn(cx, {
                    let task = task.clone();
                    async move |cx| {
                        task.await;

                        cx.on_next_frame(move |_, cx| {
                            cx.notify(entity_id);
                        });
                    }
                })
                .detach();
            }

            None
        })
    }

    /// Asynchronously load an asset, if the asset hasn't finished loading or doesn't exist this will return None.
    /// Your view will not be re-drawn once the asset has finished loading.
    ///
    /// Note that the multiple calls to this method will only result in one `Asset::load` call at a
    /// time.
    pub fn get_asset<A: Asset>(&mut self, source: &A::Source, cx: &mut App) -> Option<A::Output> {
        let (task, _) = cx.fetch_asset::<A>(source);
        task.now_or_never()
    }
    /// Obtain the current element offset. This method should only be called during the
    /// prepaint phase of element drawing.
    pub fn element_offset(&self) -> Point<Pixels> {
        self.invalidator.debug_assert_prepaint();
        self.element_offset_stack
            .last()
            .copied()
            .unwrap_or_default()
    }

    /// Obtain the current element opacity. This method should only be called during the
    /// prepaint phase of element drawing.
    #[inline]
    pub(crate) fn element_opacity(&self) -> f32 {
        self.invalidator.debug_assert_paint_or_prepaint();
        self.element_opacity
    }

    /// The mask the primitives painted now keep (Riseonly 0099): [`Self::content_mask`] without
    /// what lies outside the clip boundary of the cached view painting them, which the scene cuts
    /// in the frame and a replay cuts again where the view lands.
    pub(crate) fn paint_mask(&self) -> ContentMask<Pixels> {
        if let Some((_, _, mask)) = self.transformation_stack.last() {
            return mask.clone();
        }
        match self.paint_mask_stack.last() {
            Some(mask) => mask.clone(),
            None => self.content_mask(),
        }
    }

    /// Opens the clip boundary of a cached view that replays at another origin (Riseonly 0099): the
    /// scene cuts what it paints by the mask around it, and its primitives keep only the clips
    /// inside it. A row the scroll clip cuts, or one built wholly outside it, then replays moved
    /// and the scene cuts it where it lands, instead of rendering. Not under a transformation,
    /// whose masks stay those of the window. False when not opened.
    pub(crate) fn begin_clip_boundary(&mut self) -> bool {
        if !self.transformation_stack.is_empty() {
            return false;
        }
        let clip = self.cover_bounds(self.paint_mask().bounds);
        self.next_frame.scene.push_clip(clip);
        self.paint_mask_stack.push(unbounded_paint_mask());
        true
    }

    /// Closes the boundary [`Self::begin_clip_boundary`] opened (Riseonly 0099).
    pub(crate) fn end_clip_boundary(&mut self) {
        self.paint_mask_stack.pop();
        self.next_frame.scene.pop_clip();
    }

    /// Opens a scene clip around the replay of a paint recorded inside a clip boundary when no
    /// boundary opens now (Riseonly 0101): under a transformation the masks are those of
    /// the window, so the mask around the view cuts the relative primitives. Nothing is painted
    /// fresh inside, so the paint mask stays. False when not opened.
    pub(crate) fn begin_replay_clip(&mut self) -> bool {
        let clip = self.cover_bounds(self.content_mask().bounds);
        self.next_frame.scene.push_clip(clip);
        true
    }

    /// Closes the clip [`Self::begin_replay_clip`] opened (Riseonly 0101).
    pub(crate) fn end_replay_clip(&mut self) {
        self.next_frame.scene.pop_clip();
    }

    /// Obtain the current content mask. This method should only be called during element drawing.
    pub fn content_mask(&self) -> ContentMask<Pixels> {
        self.invalidator.debug_assert_paint_or_prepaint();
        // Masks pushed under a transformation are in untransformed coordinates while the scene
        // clips in window coordinates, so the mask active when it was pushed clips all of it.
        if let Some((_, mask, _)) = self.transformation_stack.last() {
            return mask.clone();
        }
        self.content_mask_stack
            .last()
            .cloned()
            .unwrap_or_else(|| ContentMask {
                bounds: Bounds {
                    origin: Point::default(),
                    size: self.viewport_size,
                },
            })
    }

    /// Provide elements in the called function with a new namespace in which their identifiers must be unique.
    /// This can be used within a custom element to distinguish multiple sets of child elements.
    pub fn with_element_namespace<R>(
        &mut self,
        element_id: impl Into<ElementId>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.element_id_stack.push(element_id.into());
        let result = f(self);
        self.element_id_stack.pop();
        result
    }

    /// Use a piece of state that exists as long this element is being rendered in consecutive frames.
    #[track_caller]
    pub fn use_keyed_state<S: 'static>(
        &mut self,
        key: impl Into<ElementId>,
        cx: &mut App,
        init: impl FnOnce(&mut Self, &mut Context<S>) -> S,
    ) -> Entity<S> {
        let current_view = self.current_view();
        // Riseonly 0051: the host notify relayed from the state is counted under the
        // use_keyed_state call site, so a frame trace names which keyed state dirties its host.
        let origin = core::panic::Location::caller();
        self.with_global_id(key.into(), |global_id, window| {
            window.with_element_state(global_id, |state: Option<Entity<S>>, window| {
                if let Some(state) = state {
                    (state.clone(), state)
                } else {
                    let new_state = cx.new(|cx| init(window, cx));
                    cx.observe(&new_state, move |_, cx| {
                        crate::profiler::with_dirty_origin(origin, || cx.notify(current_view));
                    })
                    .detach();
                    (new_state.clone(), new_state)
                }
            })
        })
    }

    /// Use a piece of state that exists as long this element is being rendered in consecutive frames, without needing to specify a key
    ///
    /// NOTE: This method uses the location of the caller to generate an ID for this state.
    ///       If this is not sufficient to identify your state (e.g. you're rendering a list item),
    ///       you can provide a custom ElementID using the `use_keyed_state` method.
    #[track_caller]
    pub fn use_state<S: 'static>(
        &mut self,
        cx: &mut App,
        init: impl FnOnce(&mut Self, &mut Context<S>) -> S,
    ) -> Entity<S> {
        self.use_keyed_state(
            ElementId::CodeLocation(*core::panic::Location::caller()),
            cx,
            init,
        )
    }

    /// Updates or initializes state for an element with the given id that lives across multiple
    /// frames. If an element with this ID existed in the rendered frame, its state will be passed
    /// to the given closure. The state returned by the closure will be stored so it can be referenced
    /// when drawing the next frame. This method should only be called as part of element drawing.
    pub fn with_element_state<S, R>(
        &mut self,
        global_id: &GlobalElementId,
        f: impl FnOnce(Option<S>, &mut Self) -> (R, S),
    ) -> R
    where
        S: 'static,
    {
        self.invalidator.debug_assert_paint_or_prepaint();

        let key = (global_id.clone(), TypeId::of::<S>());
        self.next_frame.accessed_element_states.push(key.clone());

        if let Some(any) = self
            .next_frame
            .element_states
            .remove(&key)
            .or_else(|| self.rendered_frame.element_states.remove(&key))
        {
            let ElementStateBox {
                inner,
                #[cfg(debug_assertions)]
                type_name,
            } = any;
            // Using the extra inner option to avoid needing to reallocate a new box.
            let mut state_box = inner
                .downcast::<Option<S>>()
                .map_err(|_| {
                    #[cfg(debug_assertions)]
                    {
                        anyhow::anyhow!(
                            "invalid element state type for id, requested {:?}, actual: {:?}",
                            std::any::type_name::<S>(),
                            type_name
                        )
                    }

                    #[cfg(not(debug_assertions))]
                    {
                        anyhow::anyhow!(
                            "invalid element state type for id, requested {:?}",
                            std::any::type_name::<S>(),
                        )
                    }
                })
                .unwrap();

            let state = state_box.take().expect(
                "reentrant call to with_element_state for the same state type and element id",
            );
            let (result, state) = f(Some(state), self);
            state_box.replace(state);
            self.next_frame.element_states.insert(
                key,
                ElementStateBox {
                    inner: state_box,
                    #[cfg(debug_assertions)]
                    type_name,
                },
            );
            result
        } else {
            let (result, state) = f(None, self);
            self.next_frame.element_states.insert(
                key,
                ElementStateBox {
                    inner: Box::new(Some(state)),
                    #[cfg(debug_assertions)]
                    type_name: std::any::type_name::<S>(),
                },
            );
            result
        }
    }

    /// A variant of `with_element_state` that allows the element's id to be optional. This is a convenience
    /// method for elements where the element id may or may not be assigned. Prefer using `with_element_state`
    /// when the element is guaranteed to have an id.
    ///
    /// The first option means 'no ID provided'
    /// The second option means 'not yet initialized'
    pub fn with_optional_element_state<S, R>(
        &mut self,
        global_id: Option<&GlobalElementId>,
        f: impl FnOnce(Option<Option<S>>, &mut Self) -> (R, Option<S>),
    ) -> R
    where
        S: 'static,
    {
        self.invalidator.debug_assert_paint_or_prepaint();

        if let Some(global_id) = global_id {
            self.with_element_state(global_id, |state, cx| {
                let (result, state) = f(Some(state), cx);
                let state =
                    state.expect("you must return some state when you pass some element id");
                (result, state)
            })
        } else {
            let (result, state) = f(None, self);
            debug_assert!(
                state.is_none(),
                "you must not return an element state when passing None for the global id"
            );
            result
        }
    }

    /// Executes the given closure within the context of a tab group.
    #[inline]
    pub fn with_tab_group<R>(&mut self, index: Option<isize>, f: impl FnOnce(&mut Self) -> R) -> R {
        if let Some(index) = index {
            self.next_frame.tab_stops.begin_group(index);
            let result = f(self);
            self.next_frame.tab_stops.end_group();
            result
        } else {
            f(self)
        }
    }

    /// Defers the drawing of the given element, scheduling it to be painted on top of the currently-drawn tree
    /// at a later time. The `priority` parameter determines the drawing order relative to other deferred elements,
    /// with higher values being drawn on top.
    ///
    /// When `content_mask` is provided, the deferred element will be clipped to that region during
    /// both prepaint and paint. When `None`, no additional clipping is applied.
    ///
    /// This method should only be called as part of the prepaint phase of element drawing.
    pub fn defer_draw(
        &mut self,
        element: AnyElement,
        absolute_offset: Point<Pixels>,
        priority: usize,
        content_mask: Option<ContentMask<Pixels>>,
    ) {
        self.invalidator.debug_assert_prepaint();
        // Riseonly 0034: a popover of a hidden subtree would draw over what is shown, and a
        // deferred draw paints at the window level whatever hides its parent.
        if self.hidden_depth > 0 {
            self.hidden_dropped_deferred += 1;
            return;
        }
        let parent_node = self.next_frame.dispatch_tree.active_node_id().unwrap();
        self.next_frame.deferred_draws.push(DeferredDraw {
            current_view: self.current_view(),
            parent_node,
            element_id_stack: self.element_id_stack.clone(),
            text_style_stack: self.text_style_stack.clone(),
            content_mask,
            rem_size: self.rem_size(),
            priority,
            element: Some(element),
            absolute_offset,
            prepaint_range: PrepaintStateIndex::default()..PrepaintStateIndex::default(),
            paint_range: PaintIndex::default()..PaintIndex::default(),
            copied_from: None,
            reads: None,
        });
    }

    /// Creates a new painting layer for the specified bounds. A "layer" is a batch
    /// of geometry that are non-overlapping and have the same draw order. This is typically used
    /// for performance reasons.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_layer<R>(&mut self, bounds: Bounds<Pixels>, f: impl FnOnce(&mut Self) -> R) -> R {
        self.invalidator.debug_assert_paint();

        let content_mask = self.paint_mask();
        let clipped_bounds = bounds.intersect(&content_mask.bounds);
        if !clipped_bounds.is_empty() {
            self.next_frame
                .scene
                .push_layer(self.cover_bounds(clipped_bounds));
        }

        let result = f(self);

        if !clipped_bounds.is_empty() {
            self.next_frame.scene.pop_layer();
        }

        result
    }

    /// Paint the drop (non-inset) shadows from `shadows` into the scene at the current
    /// z-index. Inset shadows are skipped; paint those with [`Self::paint_inset_shadows`]
    /// after the element's background so they layer on top of the fill.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_drop_shadows(
        &mut self,
        bounds: Bounds<Pixels>,
        corner_radii: Corners<Pixels>,
        shadows: &[BoxShadow],
    ) {
        self.invalidator.debug_assert_paint();

        let scale_factor = self.scale_factor();
        let content_mask = self.snapped_content_mask();
        let opacity = self.element_opacity();
        let element_bounds = self.cover_bounds(bounds);
        let element_corner_radii = corner_radii.scale(scale_factor);
        for shadow in shadows {
            if shadow.inset {
                continue;
            }
            let shadow_bounds = (bounds + shadow.offset).dilate(shadow.spread_radius);
            self.next_frame.scene.insert_primitive(Shadow {
                order: 0,
                blur_radius: shadow.blur_radius.scale(scale_factor),
                bounds: self.cover_bounds(shadow_bounds),
                content_mask,
                corner_radii: corner_radii.scale(scale_factor),
                color: shadow.color.opacity(opacity),
                element_bounds,
                element_corner_radii,
                inset: 0,
                pad: 0,
            });
        }
    }

    /// Paint the inset shadows from `shadows` into the scene at the current z-index. Should
    /// be called after the element's background so the shadow layers on top of the fill.
    /// Drop shadows are skipped; paint those with [`Self::paint_drop_shadows`] before the background.
    pub fn paint_inset_shadows(
        &mut self,
        bounds: Bounds<Pixels>,
        corner_radii: Corners<Pixels>,
        shadows: &[BoxShadow],
    ) {
        self.invalidator.debug_assert_paint();

        let scale_factor = self.scale_factor();
        let content_mask = self.snapped_content_mask();
        let opacity = self.element_opacity();
        let element_bounds = self.cover_bounds(bounds);
        let element_corner_radii = corner_radii.scale(scale_factor);
        for shadow in shadows {
            if !shadow.inset {
                continue;
            }
            let hole = (bounds + shadow.offset).dilate(-shadow.spread_radius);
            // Clamp at zero so a large spread can't produce negative radii, which would
            // break the SDF in the shader.
            let zero = Pixels::ZERO;
            let hole_corner_radii = Corners {
                top_left: (corner_radii.top_left - shadow.spread_radius).max(zero),
                top_right: (corner_radii.top_right - shadow.spread_radius).max(zero),
                bottom_right: (corner_radii.bottom_right - shadow.spread_radius).max(zero),
                bottom_left: (corner_radii.bottom_left - shadow.spread_radius).max(zero),
            };
            self.next_frame.scene.insert_primitive(Shadow {
                order: 0,
                blur_radius: shadow.blur_radius.scale(scale_factor),
                bounds: self.cover_bounds(hole),
                content_mask,
                corner_radii: hole_corner_radii.scale(scale_factor),
                color: shadow.color.opacity(opacity),
                element_bounds,
                element_corner_radii,
                inset: 1,
                pad: 0,
            });
        }
    }

    fn largest_border_interior(quad: &Quad) -> Bounds<ScaledPixels> {
        let radii = &quad.corner_radii;
        let widths = &quad.border_widths;
        let edge_radii = Edges {
            top: radii.top_left.max(radii.top_right),
            right: radii.top_right.max(radii.bottom_right),
            bottom: radii.bottom_left.max(radii.bottom_right),
            left: radii.top_left.max(radii.bottom_left),
        };

        let antialias_inset = point(ScaledPixels(1.0), ScaledPixels(1.0));
        let inset_bounds = |top_left_inset, bottom_right_inset| {
            Bounds::from_corners(
                quad.bounds.origin + top_left_inset + antialias_inset,
                quad.bounds.bottom_right() - bottom_right_inset - antialias_inset,
            )
        };

        // Rounded corners need only be excluded on one axis. Either candidate
        // is empty of border pixels, so use the larger interior.
        let horizontal_band = inset_bounds(
            point(widths.left, widths.top.max(edge_radii.top)),
            point(widths.right, widths.bottom.max(edge_radii.bottom)),
        );
        let vertical_band = inset_bounds(
            point(widths.left.max(edge_radii.left), widths.top),
            point(widths.right.max(edge_radii.right), widths.bottom),
        );

        let area = |bounds: &Bounds<ScaledPixels>| {
            bounds.size.width.0.max(0.) * bounds.size.height.0.max(0.)
        };
        if area(&horizontal_band) >= area(&vertical_band) {
            horizontal_band
        } else {
            vertical_band
        }
    }

    /// Paint one or more quads into the scene for the next frame at the current stacking context.
    /// Quads are colored rectangular regions with an optional background, border, and corner radius.
    /// see [`fill`], [`outline`], and [`quad`] to construct this type.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    ///
    /// Note that the `quad.corner_radii` are allowed to exceed the bounds, creating sharp corners
    /// where the circular arcs meet. This will not display well when combined with dashed borders.
    /// Use `Corners::clamp_radii_for_quad_size` if the radii should fit within the bounds.
    /// Blurs what is painted under `bounds` so far, with a gaussian of `sigma`, thinning out to
    /// none over the bottom `fade` (Riseonly 0110). Paint the tint after it. Draws nothing where
    /// `BACKDROP_BLUR_SUPPORTED` is false.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_backdrop_blur(
        &mut self,
        bounds: Bounds<Pixels>,
        corner_radii: impl Into<Corners<Pixels>>,
        sigma: Pixels,
        fade: Pixels,
    ) {
        if !crate::BACKDROP_BLUR_SUPPORTED || sigma <= px(0.) || bounds.is_empty() {
            return;
        }
        self.paint_quad(PaintQuad {
            bounds,
            corner_radii: corner_radii.into(),
            background: crate::transparent_black().into(),
            border_widths: Edges {
                top: sigma,
                right: px(0.),
                bottom: fade.max(px(0.)),
                left: px(0.),
            },
            border_color: crate::transparent_black(),
            border_style: BorderStyle::BackdropBlur,
        });
    }

    /// Blurs what is painted under `bounds` so far with a radius that changes along the height
    /// (Riseonly 0114): `sigma` at the edge (`from_bottom` picks the bottom one, else the top),
    /// held over the first `hold` from that edge, then easing down to no blur at the opposite
    /// edge, so the content leaves the edge gradually. Same two passes as `paint_backdrop_blur`.
    /// Paint the tint after it. Draws nothing where `BACKDROP_BLUR_SUPPORTED` is false.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_progressive_backdrop_blur(
        &mut self,
        bounds: Bounds<Pixels>,
        corner_radii: impl Into<Corners<Pixels>>,
        sigma: Pixels,
        hold: Pixels,
        from_bottom: bool,
    ) {
        if !crate::BACKDROP_BLUR_SUPPORTED || sigma <= px(0.) || bounds.is_empty() {
            return;
        }
        let (left, right) = if from_bottom { (px(0.), px(1.)) } else { (px(1.), px(0.)) };
        self.paint_quad(PaintQuad {
            bounds,
            corner_radii: corner_radii.into(),
            background: crate::transparent_black().into(),
            border_widths: Edges {
                top: sigma,
                right,
                bottom: hold.max(px(0.)),
                left,
            },
            border_color: crate::transparent_black(),
            border_style: BorderStyle::BackdropBlur,
        });
    }

    pub fn paint_quad(&mut self, quad: PaintQuad) {
        self.invalidator.debug_assert_paint();

        let transformation = self.transformation();
        if transformation != TransformationMatrix::unit() {
            self.paint_transformed_quad(quad, transformation);
            return;
        }

        let opacity = self.element_opacity();
        let snapped_bounds = self.snap_bounds(quad.bounds);
        let snapped_border_widths = self.snap_border_widths(quad.border_widths);
        let quad = Quad {
            order: 0,
            bounds: snapped_bounds,
            content_mask: self.snapped_content_mask(),
            background: quad.background.opacity(opacity),
            border_color: quad.border_color.opacity(opacity),
            corner_radii: quad.corner_radii.scale(self.scale_factor()),
            border_widths: snapped_border_widths,
            border_style: quad.border_style,
        };

        // Riseonly 0110: a backdrop blur with no tint is no border-only quad to split.
        if !quad.background.is_transparent() || quad.border_style == BorderStyle::BackdropBlur {
            self.next_frame.scene.insert_primitive(quad);
            return;
        }

        // Splitting a border-only quad around its empty interior avoids shading
        // every transparent pixel inside large outlines.
        let outer_bounds = quad.bounds;
        let inner_bounds = Self::largest_border_interior(&quad);

        if inner_bounds.is_empty() {
            self.next_frame.scene.insert_primitive(quad);
            return;
        }

        let strips = [
            // Top
            Bounds::from_corners(
                outer_bounds.origin,
                point(outer_bounds.right(), inner_bounds.top()),
            ),
            // Bottom
            Bounds::from_corners(
                point(outer_bounds.left(), inner_bounds.bottom()),
                outer_bounds.bottom_right(),
            ),
            // Left
            Bounds::from_corners(
                point(outer_bounds.left(), inner_bounds.top()),
                inner_bounds.bottom_left(),
            ),
            // Right
            Bounds::from_corners(
                inner_bounds.top_right(),
                point(outer_bounds.right(), inner_bounds.bottom()),
            ),
        ];

        for strip in strips {
            let content_mask_bounds = quad.content_mask.bounds.intersect(&strip);
            if !content_mask_bounds.is_empty() {
                self.next_frame.scene.insert_primitive(Quad {
                    content_mask: ContentMask {
                        bounds: content_mask_bounds,
                    },
                    ..quad
                });
            }
        }
    }

    /// Paint the given `Path` into the scene for the next frame at the current z-index.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_path(&mut self, path: Path<Pixels>, color: impl Into<Background>) {
        self.paint_swappable_path(path, color, 0, Point::default());
    }

    /// [`Self::paint_path`] whose geometry [`Self::swap_path`] can replace until the next draw
    /// (Riseonly 0047); later geometry is laid out from `anchor`. `swap_key` zero names nothing,
    /// and a path painted under a transformation takes no swaps.
    pub fn paint_swappable_path(
        &mut self,
        mut path: Path<Pixels>,
        color: impl Into<Background>,
        swap_key: u64,
        anchor: Point<Pixels>,
    ) {
        self.invalidator.debug_assert_paint();

        let scale_factor = self.scale_factor();
        let content_mask = self.paint_mask();
        let opacity = self.element_opacity();
        path.content_mask = content_mask;
        let color: Background = color.into();
        path.color = color.opacity(opacity);
        let transformation = self.transformation();
        if transformation != TransformationMatrix::unit() {
            transform_path(&mut path, &transformation, scale_factor);
        } else {
            path.swap_key = swap_key;
            path.swap_anchor = anchor;
            path.swap_opacity = opacity;
        }
        self.next_frame
            .scene
            .insert_primitive(path.scale(scale_factor));
    }

    fn paint_transformed_quad(&mut self, quad: PaintQuad, transformation: TransformationMatrix) {
        let opacity = self.element_opacity();
        let scale_factor = self.scale_factor();
        let bounds = quad.bounds;
        let radii = quad.corner_radii.clamp_radii_for_quad_size(bounds.size);
        let outer = rounded_rect_outline(bounds, radii);

        let background = quad.background.opacity(opacity);
        if !background.is_transparent() {
            let center = bounds.center();
            let triangles = outer
                .iter()
                .zip(outer.iter().cycle().skip(1))
                .map(|(from, to)| [center, *from, *to]);
            self.insert_transformed_triangles(
                triangles,
                rotated_background(background, &transformation),
                &transformation,
                scale_factor,
            );
        }

        let widths = quad.border_widths;
        let border_color = quad.border_color.opacity(opacity);
        let has_border = widths.top > Pixels::ZERO
            || widths.right > Pixels::ZERO
            || widths.bottom > Pixels::ZERO
            || widths.left > Pixels::ZERO;
        if has_border && !border_color.is_transparent() {
            let inner_bounds = Bounds {
                origin: point(bounds.origin.x + widths.left, bounds.origin.y + widths.top),
                size: size(
                    (bounds.size.width - widths.left - widths.right).max(Pixels::ZERO),
                    (bounds.size.height - widths.top - widths.bottom).max(Pixels::ZERO),
                ),
            };
            let inner_radii = Corners {
                top_left: (radii.top_left - widths.top.max(widths.left)).max(Pixels::ZERO),
                top_right: (radii.top_right - widths.top.max(widths.right)).max(Pixels::ZERO),
                bottom_right: (radii.bottom_right - widths.bottom.max(widths.right))
                    .max(Pixels::ZERO),
                bottom_left: (radii.bottom_left - widths.bottom.max(widths.left))
                    .max(Pixels::ZERO),
            }
            .clamp_radii_for_quad_size(inner_bounds.size);
            let inner = rounded_rect_outline(inner_bounds, inner_radii);
            let count = outer.len();
            let triangles = (0..count).flat_map(|index| {
                let next = (index + 1) % count;
                [
                    [outer[index], outer[next], inner[index]],
                    [inner[index], outer[next], inner[next]],
                ]
            });
            self.insert_transformed_triangles(
                triangles,
                Background::from(border_color),
                &transformation,
                scale_factor,
            );
        }
    }

    fn insert_transformed_triangles(
        &mut self,
        triangles: impl IntoIterator<Item = [Point<Pixels>; 3]>,
        color: Background,
        transformation: &TransformationMatrix,
        scale_factor: f32,
    ) {
        let mut path: Option<Path<Pixels>> = None;
        for triangle in triangles {
            let [a, b, c] =
                triangle.map(|vertex| transform_point(transformation, vertex, scale_factor));
            let path = path.get_or_insert_with(|| Path::new(a));
            path.push_triangle(
                (a, b, c),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }
        let Some(mut path) = path else {
            return;
        };
        path.content_mask = self.paint_mask();
        path.color = color;
        self.next_frame
            .scene
            .insert_primitive(path.scale(scale_factor));
    }

    // A surface primitive has no matrix in any renderer, but a box is enough without a turn or a
    // mirror; under those the surface keeps its untransformed box.
    fn surface_bounds_under_transformation(&self, bounds: Bounds<Pixels>) -> Bounds<Pixels> {
        let transformation = self.transformation();
        let [[scale_x, shear_x], [shear_y, scale_y]] = transformation.rotation_scale;
        if shear_x != 0.0 || shear_y != 0.0 || scale_x <= 0.0 || scale_y <= 0.0 {
            return bounds;
        }
        let scale_factor = self.scale_factor();
        Bounds::from_corners(
            transform_point(&transformation, bounds.origin, scale_factor),
            transform_point(&transformation, bounds.bottom_right(), scale_factor),
        )
    }

    /// Paint an underline into the scene for the next frame at the current z-index.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_underline(
        &mut self,
        origin: Point<Pixels>,
        width: Pixels,
        style: &UnderlineStyle,
    ) {
        self.invalidator.debug_assert_paint();

        let scale_factor = self.scale_factor();
        let thickness = self.snap_stroke(style.thickness);
        let height = if style.wavy {
            ScaledPixels(thickness.0 * 3.)
        } else {
            thickness
        };
        let bounds = Bounds {
            origin: origin.map(|c| ScaledPixels(round_to_device_pixel(c.0, scale_factor))),
            size: size(self.snap_stroke(width), height),
        };
        let element_opacity = self.element_opacity();

        self.next_frame.scene.insert_primitive(Underline {
            order: 0,
            pad: 0,
            bounds,
            content_mask: self.snapped_content_mask(),
            color: style.color.unwrap_or_default().opacity(element_opacity),
            thickness,
            wavy: style.wavy.into(),
        });
    }

    /// Paint a strikethrough into the scene for the next frame at the current z-index.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_strikethrough(
        &mut self,
        origin: Point<Pixels>,
        width: Pixels,
        style: &StrikethroughStyle,
    ) {
        self.invalidator.debug_assert_paint();

        let scale_factor = self.scale_factor();
        let height = style.thickness;
        let bounds = Bounds {
            origin: origin.map(|c| ScaledPixels(round_to_device_pixel(c.0, scale_factor))),
            size: size(self.snap_stroke(width), self.snap_stroke(height)),
        };
        let opacity = self.element_opacity();

        self.next_frame.scene.insert_primitive(Underline {
            order: 0,
            pad: 0,
            bounds,
            content_mask: self.snapped_content_mask(),
            thickness: self.snap_stroke(style.thickness),
            color: style.color.unwrap_or_default().opacity(opacity),
            wavy: false.into(),
        });
    }

    /// Paints a monochrome (non-emoji) glyph into the scene for the next frame at the current z-index.
    ///
    /// The y component of the origin is the baseline of the glyph.
    /// You should generally prefer to use the [`ShapedLine::paint`](crate::ShapedLine::paint) or
    /// [`WrappedLine::paint`](crate::WrappedLine::paint) methods in the [`TextSystem`](crate::TextSystem).
    /// This method is only useful if you need to paint a single glyph that has already been shaped.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_glyph(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
        color: Hsla,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let element_opacity = self.element_opacity();
        let scale_factor = self.scale_factor();
        let glyph_origin = origin.scale(scale_factor);

        let quantized_origin = Point::new(
            round_half_toward_zero(glyph_origin.x.0 * SUBPIXEL_VARIANTS_X as f32)
                / SUBPIXEL_VARIANTS_X as f32,
            round_half_toward_zero(glyph_origin.y.0 * SUBPIXEL_VARIANTS_Y as f32)
                / SUBPIXEL_VARIANTS_Y as f32,
        );
        let subpixel_variant = Point::new(
            (quantized_origin.x.fract() * SUBPIXEL_VARIANTS_X as f32) as u8,
            (quantized_origin.y.fract() * SUBPIXEL_VARIANTS_Y as f32) as u8,
        );
        let integer_origin = quantized_origin.map(|c| ScaledPixels(c.trunc()));
        let subpixel_rendering = self.should_use_subpixel_rendering(font_id, font_size);
        let transformation = self.transformation();
        let dilation = self.text_system().glyph_dilation_for_color(color);
        let params = RenderGlyphParams {
            font_id,
            glyph_id,
            font_size,
            subpixel_variant,
            scale_factor,
            is_emoji: false,
            subpixel_rendering,
            dilation,
        };

        let raster_bounds = self.text_system().raster_bounds(&params)?;
        if !raster_bounds.is_zero() {
            let tile = self
                .sprite_atlas
                .get_or_insert_with(&params.clone().into(), &mut || {
                    let (size, bytes) = self.text_system().rasterize_glyph(&params)?;
                    Ok(Some((size, Cow::Owned(bytes))))
                })?
                .expect("Callback above only errors or returns Some");
            let bounds = Bounds {
                origin: integer_origin + raster_bounds.origin.map(Into::into),
                size: tile.bounds.size.map(Into::into),
            };
            let content_mask = self.snapped_content_mask();

            if subpixel_rendering {
                self.next_frame.scene.insert_primitive(SubpixelSprite {
                    order: 0,
                    pad: self.glyph_swap_key,
                    bounds,
                    content_mask,
                    color: color.opacity(element_opacity),
                    tile,
                    transformation,
                });
            } else {
                self.next_frame.scene.insert_primitive(MonochromeSprite {
                    order: 0,
                    pad: self.glyph_swap_key,
                    bounds,
                    content_mask,
                    color: color.opacity(element_opacity),
                    tile,
                    transformation,
                });
            }
        }
        Ok(())
    }

    fn should_use_subpixel_rendering(&self, font_id: FontId, font_size: Pixels) -> bool {
        if self.platform_window.background_appearance() != WindowBackgroundAppearance::Opaque {
            return false;
        }

        if !self.platform_window.is_subpixel_rendering_supported() {
            return false;
        }

        let mode = match self.text_rendering_mode.get() {
            TextRenderingMode::PlatformDefault => self
                .text_system()
                .recommended_rendering_mode(font_id, font_size),
            mode => mode,
        };

        mode == TextRenderingMode::Subpixel
    }

    /// Rasterizes the emoji of `text` on the background executor and stores their tiles in this
    /// window's sprite atlas (Riseonly 0060), so the first paint of those glyphs reads the atlas
    /// instead of rasterizing inside the frame: a colour emoji costs about a millisecond on the
    /// main thread, and a wheel step through an emoji grid revealed close to a hundred of them.
    /// Nothing is notified, the tiles only wait for a paint. Dropping the task stops the insert.
    pub fn prewarm_emoji_tiles(
        &self,
        text: SharedString,
        font: crate::Font,
        font_size: Pixels,
        cx: &App,
    ) -> Task<()> {
        let text_system = cx.text_system().clone();
        let atlas = self.sprite_atlas.clone();
        let scale_factor = self.scale_factor();
        let tiles = cx.background_executor().spawn(async move {
            text_system.rasterize_emoji_text(&text, &font, font_size, scale_factor)
        });
        cx.foreground_executor().spawn(async move {
            for (params, size, bytes) in tiles.await {
                let mut bytes = Some(bytes);
                atlas
                    .get_or_insert_with(&params.into(), &mut || {
                        Ok(bytes.take().map(|bytes| (size, Cow::Owned(bytes))))
                    })
                    .log_err();
            }
        })
    }

    /// Shapes each of `texts` as one run of `font` at `font_size` on the background executor and
    /// keeps the layouts in this window's line cache across frames (Riseonly 0068), so the frame
    /// that first lays out one of them reads it instead of shaping on the main thread: a wheel
    /// step through an emoji grid shaped close to a hundred single-emoji lines in its layout.
    /// Nothing is notified. Dropping the task stops the shaping not yet done.
    pub fn prewarm_line_layouts(
        &self,
        texts: Vec<SharedString>,
        font: crate::Font,
        font_size: Pixels,
        cx: &App,
    ) -> Task<()> {
        let font_id = self.text_system.resolve_font(&font);
        let retainer = self.text_system.line_layout_retainer();
        cx.background_executor().spawn(async move {
            for text in texts {
                retainer.retain(text, font_size, font_id);
            }
        })
    }

    /// Paints an emoji glyph into the scene for the next frame at the current z-index.
    ///
    /// The y component of the origin is the baseline of the glyph.
    /// You should generally prefer to use the [`ShapedLine::paint`](crate::ShapedLine::paint) or
    /// [`WrappedLine::paint`](crate::WrappedLine::paint) methods in the [`TextSystem`](crate::TextSystem).
    /// This method is only useful if you need to paint a single emoji that has already been shaped.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_emoji(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let scale_factor = self.scale_factor();
        let glyph_origin = origin.scale(scale_factor);
        let integer_origin = glyph_origin.map(|c| ScaledPixels(round_half_toward_zero(c.0)));
        let params = RenderGlyphParams {
            font_id,
            glyph_id,
            font_size,
            subpixel_variant: Default::default(),
            scale_factor,
            is_emoji: true,
            subpixel_rendering: false,
            dilation: 0,
        };

        let raster_bounds = self.text_system().raster_bounds(&params)?;
        if !raster_bounds.is_zero() {
            let tile = self
                .sprite_atlas
                .get_or_insert_with(&params.clone().into(), &mut || {
                    let (size, bytes) = self.text_system().rasterize_glyph(&params)?;
                    Ok(Some((size, Cow::Owned(bytes))))
                })?
                .expect("Callback above only errors or returns Some");

            let bounds = Bounds {
                origin: integer_origin + raster_bounds.origin.map(Into::into),
                size: tile.bounds.size.map(Into::into),
            };
            let content_mask = self.snapped_content_mask();
            let opacity = self.element_opacity();
            let transformation = self.transformation();

            // An emoji of a sliding title slides with its glyphs (0079).
            let pad = if crate::is_glyph_shift_key(self.glyph_swap_key) { self.glyph_swap_key } else { 0 };
            self.next_frame.scene.insert_primitive(PolychromeSprite {
                order: 0,
                pad,
                grayscale: false.into(),
                bounds,
                corner_radii: Default::default(),
                content_mask,
                tile,
                opacity,
                transformation,
            });
        }
        Ok(())
    }

    /// Paint a monochrome SVG into the scene for the next frame at the current stacking context.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_svg(
        &mut self,
        bounds: Bounds<Pixels>,
        path: SharedString,
        mut data: Option<&[u8]>,
        transformation: TransformationMatrix,
        color: Hsla,
        cx: &App,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let element_opacity = self.element_opacity();
        let transformation = self.transformation().compose(transformation);
        let bounds = self.snap_bounds(bounds);

        let params = RenderSvgParams {
            path,
            size: bounds.size.map(|pixels| {
                DevicePixels::from((pixels.0 * SMOOTH_SVG_SCALE_FACTOR).ceil() as i32)
            }),
        };

        let Some(tile) =
            self.sprite_atlas
                .get_or_insert_with(&params.clone().into(), &mut || {
                    let Some((size, bytes)) = cx.svg_renderer.render_alpha_mask(&params, data)?
                    else {
                        return Ok(None);
                    };
                    Ok(Some((size, Cow::Owned(bytes))))
                })?
        else {
            return Ok(());
        };
        let content_mask = self.snapped_content_mask();
        let svg_bounds = Bounds {
            origin: bounds.center()
                - Point::new(
                    ScaledPixels(tile.bounds.size.width.0 as f32 / SMOOTH_SVG_SCALE_FACTOR / 2.),
                    ScaledPixels(tile.bounds.size.height.0 as f32 / SMOOTH_SVG_SCALE_FACTOR / 2.),
                ),
            size: tile
                .bounds
                .size
                .map(|value| ScaledPixels(value.0 as f32 / SMOOTH_SVG_SCALE_FACTOR)),
        };
        let final_bounds = svg_bounds
            .map_origin(|value| ScaledPixels(round_half_toward_zero(value.0)))
            .map_size(|size| size.ceil());

        self.next_frame.scene.insert_primitive(MonochromeSprite {
            order: 0,
            pad: if crate::is_svg_fade_key(self.glyph_swap_key) { self.glyph_swap_key } else { 0 },
            bounds: final_bounds,
            content_mask,
            color: color.opacity(element_opacity),
            tile,
            transformation,
        });

        Ok(())
    }

    /// Paint an image into the scene for the next frame at the current z-index.
    /// This method will panic if the frame_index is not valid
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    /// Paint an image into `bounds`, positioning and scaling it according to `image_bounds`.
    ///
    /// The visible region rendered is `bounds.intersect(&image_bounds)`, with `corner_radii`
    /// applied to `bounds`.
    pub fn paint_image(
        &mut self,
        bounds: Bounds<Pixels>,
        image_bounds: Bounds<Pixels>,
        corner_radii: Corners<Pixels>,
        data: Arc<RenderImage>,
        frame_index: usize,
        grayscale: bool,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let visible_bounds = bounds.intersect(&image_bounds);
        if visible_bounds.size.width <= Pixels::ZERO || visible_bounds.size.height <= Pixels::ZERO {
            return Ok(());
        }
        if image_bounds.size.width <= Pixels::ZERO || image_bounds.size.height <= Pixels::ZERO {
            return Ok(());
        }

        let params = RenderImageParams {
            image_id: data.id,
            frame_index,
        };

        let tile = self
            .sprite_atlas
            .get_or_insert_with(&params.into(), &mut || {
                Ok(Some((
                    data.size(frame_index),
                    Cow::Borrowed(
                        data.as_bytes(frame_index)
                            .expect("It's the caller's job to pass a valid frame index"),
                    ),
                )))
            })?
            .expect("Callback above only returns Some");

        let visible_bounds_snapped = self.snap_bounds(visible_bounds);

        let sub_tile = if visible_bounds == image_bounds {
            tile
        } else {
            let x_offset_ratio =
                (visible_bounds.origin.x - image_bounds.origin.x) / image_bounds.size.width;
            let y_offset_ratio =
                (visible_bounds.origin.y - image_bounds.origin.y) / image_bounds.size.height;
            let width_ratio = visible_bounds.size.width / image_bounds.size.width;
            let height_ratio = visible_bounds.size.height / image_bounds.size.height;

            let tile_origin_x = tile.bounds.origin.x.0;
            let tile_origin_y = tile.bounds.origin.y.0;
            let tile_width = tile.bounds.size.width.0;
            let tile_height = tile.bounds.size.height.0;

            let sub_origin_x = tile_origin_x + (x_offset_ratio * tile_width as f32).round() as i32;
            let sub_origin_y = tile_origin_y + (y_offset_ratio * tile_height as f32).round() as i32;
            let sub_width = (width_ratio * tile_width as f32).round() as i32;
            let sub_height = (height_ratio * tile_height as f32).round() as i32;

            let max_x = tile_origin_x + tile_width;
            let max_y = tile_origin_y + tile_height;

            let clamped_origin_x = sub_origin_x.clamp(tile_origin_x, max_x);
            let clamped_origin_y = sub_origin_y.clamp(tile_origin_y, max_y);
            let clamped_width = sub_width.min(max_x - clamped_origin_x).max(0);
            let clamped_height = sub_height.min(max_y - clamped_origin_y).max(0);

            AtlasTile {
                bounds: Bounds {
                    origin: point(
                        DevicePixels(clamped_origin_x),
                        DevicePixels(clamped_origin_y),
                    ),
                    size: size(DevicePixels(clamped_width), DevicePixels(clamped_height)),
                },
                ..tile
            }
        };

        let content_mask = self.snapped_content_mask();
        let corner_radii = corner_radii
            .clamp_radii_for_quad_size(visible_bounds.size)
            .scale(self.scale_factor());
        let opacity = self.element_opacity();
        let transformation = self.transformation();

        self.next_frame.scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: self.image_swap_key,
            grayscale: grayscale.into(),
            bounds: visible_bounds_snapped,
            content_mask,
            corner_radii,
            tile: sub_tile,
            opacity,
            transformation,
        });
        Ok(())
    }

    /// Paint a surface into the scene for the next frame at the current z-index.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    #[cfg(target_os = "macos")]
    pub fn paint_surface(&mut self, bounds: Bounds<Pixels>, image_buffer: CVPixelBuffer) {
        self.paint_swappable_surface(bounds, image_buffer, 0);
    }

    /// [`Self::paint_surface`] whose picture [`Self::swap_surface`] can replace until the next
    /// draw (Riseonly 0046); `swap_key` zero names nothing.
    #[cfg(target_os = "macos")]
    pub fn paint_swappable_surface(&mut self, bounds: Bounds<Pixels>, image_buffer: CVPixelBuffer, swap_key: u64) {
        use crate::PaintSurface;

        self.invalidator.debug_assert_paint();

        let bounds = self.snap_bounds(self.surface_bounds_under_transformation(bounds));
        let content_mask = self.snapped_content_mask();
        let (corner_clip_bounds, corner_radii) = self.surface_corner_clip();
        self.next_frame.scene.insert_primitive(PaintSurface {
            order: 0,
            bounds,
            content_mask,
            corner_clip_bounds,
            corner_radii,
            image_buffer,
            swap_key,
        });
    }

    /// Paint a decoded video frame into the scene for the next frame at the current z-index.
    ///
    /// Off macOS only; renderers that own their surface textures draw it, others skip it.
    /// This method should only be called as part of the paint phase of element drawing.
    #[cfg(not(target_os = "macos"))]
    pub fn paint_owned_surface(&mut self, bounds: Bounds<Pixels>, frame: crate::OwnedSurfaceFrame) {
        self.paint_swappable_owned_surface(bounds, frame, 0);
    }

    /// [`Self::paint_owned_surface`] whose frame [`Self::swap_surface`] can replace until the next
    /// draw (Riseonly 0046); `swap_key` zero names nothing.
    #[cfg(not(target_os = "macos"))]
    pub fn paint_swappable_owned_surface(
        &mut self,
        bounds: Bounds<Pixels>,
        frame: crate::OwnedSurfaceFrame,
        swap_key: u64,
    ) {
        use crate::PaintSurface;

        self.invalidator.debug_assert_paint();

        let bounds = self.snap_bounds(self.surface_bounds_under_transformation(bounds));
        let content_mask = self.snapped_content_mask();
        let (corner_clip_bounds, corner_radii) = self.surface_corner_clip();
        self.next_frame.scene.insert_primitive(PaintSurface {
            order: 0,
            bounds,
            content_mask,
            corner_clip_bounds,
            corner_radii,
            frame,
            swap_key,
        });
    }

    /// Puts a new picture into every surface of the frame on screen painted with `swap_key`
    /// (Riseonly 0046) and asks for a present of that frame: a playing video shows its next picture
    /// without rendering, laying out or painting any view. Call it outside a draw, from an
    /// [`Self::on_next_frame`] callback. False when the frame on screen holds no surface with the
    /// key (hidden, clipped away or gone): the caller stops swapping and the next render of its
    /// element paints the picture again.
    pub fn swap_surface(&mut self, swap_key: u64, source: crate::SurfaceSource) -> bool {
        let found = self.rendered_frame.scene.swap_surface(swap_key, |surface| match &source {
            #[cfg(target_os = "macos")]
            crate::SurfaceSource::Surface(buffer) => surface.image_buffer = buffer.clone(),
            #[cfg(not(target_os = "macos"))]
            crate::SurfaceSource::Owned(frame) => surface.frame = frame.clone(),
        });
        if found {
            self.needs_present.set(true);
        }
        found
    }

    /// Gives every path of the frame on screen painted with `swap_key` the geometry of `local`,
    /// laid out from the anchor it was painted with (Riseonly 0047), and asks for a present of that
    /// frame: a spinner turns without rendering, laying out or painting any view. Call it outside a
    /// draw, from an [`Self::on_next_frame`] callback. False when the frame on screen holds no path
    /// with the key: the caller stops swapping and repaints through its view.
    pub fn swap_path(&mut self, swap_key: u64, local: Path<Pixels>) -> bool {
        let local = local.scale(self.scale_factor());
        let found = self.rendered_frame.scene.swap_path(swap_key, &local);
        if found {
            self.needs_present.set(true);
        }
        found
    }

    /// [`Self::swap_path`] for several keys in one pass over the frame on screen, each path also
    /// taking the colour of its swap (Riseonly 0049), and a present when every key was found. A
    /// false return asks the caller to stop swapping and repaint through its view.
    pub fn swap_paths(&mut self, swaps: Vec<(u64, Path<Pixels>, Background)>) -> bool {
        let scale_factor = self.scale_factor();
        let swaps: Vec<(u64, Path<ScaledPixels>, Background)> = swaps
            .into_iter()
            .map(|(swap_key, local, color)| (swap_key, local.scale(scale_factor), color))
            .collect();
        let found = self.rendered_frame.scene.swap_paths(&swaps);
        if found {
            self.needs_present.set(true);
        }
        found
    }

    /// The off thread tile writer of this window's sprite atlas, none where tiles are written in paint (Riseonly 0122).
    pub fn image_uploader(&self) -> Option<Arc<dyn PlatformImageUploader>> {
        self.sprite_atlas.image_uploader()
    }

    /// Removes an image from the sprite atlas when the next frame starts (Riseonly 0128).
    pub fn drop_image(&mut self, data: Arc<RenderImage>) -> Result<()> {
        self.image_drops += 1;
        for frame_index in 0..data.frame_count() {
            let params = RenderImageParams {
                image_id: data.id,
                frame_index,
            };

            self.pending_image_drops.push(params.into());
        }
        if self.pending_image_drops.len() > MAX_PENDING_IMAGE_DROPS {
            self.flush_image_drops();
        }

        Ok(())
    }

    fn flush_image_drops(&mut self) {
        for key in self.pending_image_drops.drain(..) {
            self.sprite_atlas.remove(&key);
        }
    }

    /// Returns whether every frame of an image is present in the sprite atlas.
    #[cfg(any(test, feature = "test-support"))]
    pub fn has_image_atlas_entry(&self, data: &RenderImage) -> bool {
        data.frame_count() > 0
            && (0..data.frame_count()).all(|frame_index| {
                self.sprite_atlas.contains(
                    &RenderImageParams {
                        image_id: data.id,
                        frame_index,
                    }
                    .into(),
                )
            })
    }

    /// Add a node to the layout tree for the current frame. Takes the `Style` of the element for which
    /// layout is being requested, along with the layout ids of any children. This method is called during
    /// calls to the [`Element::request_layout`] trait method and enables any element to participate in layout.
    ///
    /// This method should only be called as part of the request_layout or prepaint phase of element drawing.
    #[must_use]
    pub fn request_layout(
        &mut self,
        style: Style,
        children: impl IntoIterator<Item = LayoutId>,
        cx: &mut App,
    ) -> LayoutId {
        self.invalidator.debug_assert_prepaint();

        cx.layout_id_buffer.clear();
        cx.layout_id_buffer.extend(children);
        let rem_size = self.rem_size();
        let scale_factor = self.scale_factor();

        self.layout_engine.as_mut().unwrap().request_layout(
            style,
            rem_size,
            scale_factor,
            &cx.layout_id_buffer,
        )
    }

    /// Add a node to the layout tree for the current frame. Instead of taking a `Style` and children,
    /// this variant takes a function that is invoked during layout so you can use arbitrary logic to
    /// determine the element's size. One place this is used internally is when measuring text.
    ///
    /// The given closure is invoked at layout time with the known dimensions and available space and
    /// returns a `Size`.
    ///
    /// This method should only be called as part of the request_layout or prepaint phase of element drawing.
    pub fn request_measured_layout<F>(&mut self, style: Style, measure: F) -> LayoutId
    where
        F: Fn(Size<Option<Pixels>>, Size<AvailableSpace>, &mut Window, &mut App) -> Size<Pixels>
            + 'static,
    {
        self.invalidator.debug_assert_prepaint();

        let rem_size = self.rem_size();
        let scale_factor = self.scale_factor();
        self.layout_engine
            .as_mut()
            .unwrap()
            .request_measured_layout(style, rem_size, scale_factor, measure)
    }

    /// Compute the layout for the given id within the given available space.
    /// This method is called for its side effect, typically by the framework prior to painting.
    /// After calling it, you can request the bounds of the given layout node id or any descendant.
    ///
    /// This method should only be called as part of the prepaint phase of element drawing.
    pub fn compute_layout(
        &mut self,
        layout_id: LayoutId,
        available_space: Size<AvailableSpace>,
        cx: &mut App,
    ) {
        self.invalidator.debug_assert_prepaint();

        let mut layout_engine = self.layout_engine.take().unwrap();
        layout_engine.compute_layout(layout_id, available_space, self, cx);
        self.layout_engine = Some(layout_engine);
    }

    /// Obtain the bounds computed for the given LayoutId relative to the window. This method will usually be invoked by
    /// GPUI itself automatically in order to pass your element its `Bounds` automatically.
    ///
    /// This method should only be called as part of element drawing.
    pub fn layout_bounds(&mut self, layout_id: LayoutId) -> Bounds<Pixels> {
        self.invalidator.debug_assert_prepaint();

        let scale_factor = self.scale_factor();
        let mut bounds = self
            .layout_engine
            .as_mut()
            .unwrap()
            .layout_bounds(layout_id, scale_factor)
            .map(Into::into);
        let snapped_offset = self.pixel_snap_point(self.element_offset());
        bounds.origin += snapped_offset;
        bounds
    }

    /// This method should be called during `prepaint`. You can use
    /// the returned [Hitbox] during `paint` or in an event handler
    /// to determine whether the inserted hitbox was the topmost.
    ///
    /// This method should only be called as part of the prepaint phase of element drawing.
    pub fn insert_hitbox(&mut self, bounds: Bounds<Pixels>, behavior: HitboxBehavior) -> Hitbox {
        self.invalidator.debug_assert_prepaint();

        let content_mask = self.content_mask();
        let mut id = self.next_hitbox_id;
        self.next_hitbox_id = self.next_hitbox_id.next();
        let hitbox = Hitbox {
            id,
            bounds,
            content_mask,
            behavior,
            layer: self.hitbox_layer,
            hidden: self.hidden_depth > 0,
            wheel_area: None,
        };
        self.next_frame.hitboxes.push(hitbox.clone());
        hitbox
    }

    /// Riseonly 0129: a wheel over `hitbox` outside `area` (window coordinates) scrolls what lies
    /// at the nearest point of `area`, as if the pointer were there.
    pub(crate) fn set_hitbox_wheel_area(&mut self, hitbox: &Hitbox, area: Bounds<Pixels>) {
        if let Some(entry) = self
            .next_frame
            .hitboxes
            .iter_mut()
            .rev()
            .find(|entry| entry.id == hitbox.id)
        {
            entry.wheel_area = Some(Bounds {
                origin: area.origin - hitbox.bounds.origin,
                size: area.size,
            });
        }
    }

    /// Set a hitbox which will act as a control area of the platform window.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn insert_window_control_hitbox(&mut self, area: WindowControlArea, hitbox: Hitbox) {
        self.invalidator.debug_assert_paint();
        self.next_frame.window_control_hitboxes.push((area, hitbox));
    }

    /// Sets the key context for the current element. This context will be used to translate
    /// keybindings into actions.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn set_key_context(&mut self, context: KeyContext) {
        self.invalidator.debug_assert_paint();
        self.next_frame.dispatch_tree.set_key_context(context);
    }

    /// Sets the focus handle for the current element. This handle will be used to manage focus state
    /// and keyboard event dispatch for the element.
    ///
    /// This method should only be called as part of the prepaint phase of element drawing.
    pub fn set_focus_handle(&mut self, focus_handle: &FocusHandle, _: &App) {
        self.invalidator.debug_assert_prepaint();
        if focus_handle.is_focused(self) {
            self.next_frame.focus = Some(focus_handle.id);
        }
        self.next_frame.dispatch_tree.set_focus_id(focus_handle.id);
    }

    /// Sets the view id for the current element, which will be used to manage view caching.
    ///
    /// This method should only be called as part of element prepaint. We plan on removing this
    /// method eventually when we solve some issues that require us to construct editor elements
    /// directly instead of always using editors via views.
    pub fn set_view_id(&mut self, view_id: EntityId) {
        self.invalidator.debug_assert_prepaint();
        self.next_frame.dispatch_tree.set_view_id(view_id);
    }

    /// Get the entity ID for the currently rendering view
    pub fn current_view(&self) -> EntityId {
        self.invalidator.debug_assert_paint_or_prepaint();
        self.rendered_entity_stack.last().copied().unwrap()
    }

    #[inline]
    pub(crate) fn with_rendered_view<R>(
        &mut self,
        id: EntityId,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.rendered_entity_stack.push(id);
        let previous = RENDERING_VIEW.with(|view| view.replace(Some(id)));
        let result = f(self);
        RENDERING_VIEW.with(|view| view.set(previous));
        self.rendered_entity_stack.pop();
        result
    }

    /// Executes the provided function with the specified image cache.
    pub fn with_image_cache<F, R>(&mut self, image_cache: Option<AnyImageCache>, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        if let Some(image_cache) = image_cache {
            self.image_cache_stack.push(image_cache);
            let result = f(self);
            self.image_cache_stack.pop();
            result
        } else {
            f(self)
        }
    }

    /// Sets an input handler, such as [`ElementInputHandler`][element_input_handler], which interfaces with the
    /// platform to receive textual input with proper integration with concerns such
    /// as IME interactions. This handler will be active for the upcoming frame until the following frame is
    /// rendered.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    ///
    /// [element_input_handler]: crate::ElementInputHandler
    pub fn handle_input(
        &mut self,
        focus_handle: &FocusHandle,
        input_handler: impl InputHandler,
        cx: &App,
    ) {
        self.invalidator.debug_assert_paint();

        if focus_handle.is_focused(self) {
            let cx = self.to_async(cx);
            self.next_frame
                .input_handlers
                .push(Some(PlatformInputHandler::new(cx, Box::new(input_handler))));
        }
    }

    /// Register a mouse event listener on the window for the next frame. The type of event
    /// is determined by the first parameter of the given listener. When the next frame is rendered
    /// the listener will be cleared.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn on_mouse_event<Event: MouseEvent>(
        &mut self,
        mut listener: impl FnMut(&Event, DispatchPhase, &mut Window, &mut App) + 'static,
    ) {
        self.invalidator.debug_assert_paint();

        self.next_frame.mouse_listeners.push(Some(Box::new(
            move |event: &dyn Any, phase: DispatchPhase, window: &mut Window, cx: &mut App| {
                if let Some(event) = event.downcast_ref() {
                    listener(event, phase, window, cx)
                }
            },
        )));
    }

    /// Register a key event listener on this node for the next frame. The type of event
    /// is determined by the first parameter of the given listener. When the next frame is rendered
    /// the listener will be cleared.
    ///
    /// This is a fairly low-level method, so prefer using event handlers on elements unless you have
    /// a specific need to register a listener yourself.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn on_key_event<Event: KeyEvent>(
        &mut self,
        listener: impl Fn(&Event, DispatchPhase, &mut Window, &mut App) + 'static,
    ) {
        self.invalidator.debug_assert_paint();

        self.next_frame.dispatch_tree.on_key_event(Rc::new(
            move |event: &dyn Any, phase, window: &mut Window, cx: &mut App| {
                if let Some(event) = event.downcast_ref::<Event>() {
                    listener(event, phase, window, cx)
                }
            },
        ));
    }

    /// Register a modifiers changed event listener on the window for the next frame.
    ///
    /// This is a fairly low-level method, so prefer using event handlers on elements unless you have
    /// a specific need to register a global listener.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn on_modifiers_changed(
        &mut self,
        listener: impl Fn(&ModifiersChangedEvent, &mut Window, &mut App) + 'static,
    ) {
        self.invalidator.debug_assert_paint();

        self.next_frame.dispatch_tree.on_modifiers_changed(Rc::new(
            move |event: &ModifiersChangedEvent, window: &mut Window, cx: &mut App| {
                listener(event, window, cx)
            },
        ));
    }

    /// Register a listener to be called when the given focus handle or one of its descendants receives focus.
    /// This does not fire if the given focus handle - or one of its descendants - was previously focused.
    /// Returns a subscription and persists until the subscription is dropped.
    pub fn on_focus_in(
        &mut self,
        handle: &FocusHandle,
        cx: &mut App,
        mut listener: impl FnMut(&mut Window, &mut App) + 'static,
    ) -> Subscription {
        let focus_id = handle.id;
        let (subscription, activate) =
            self.new_focus_listener(Box::new(move |event, window, cx| {
                if event.is_focus_in(focus_id) {
                    listener(window, cx);
                }
                true
            }));
        cx.defer(move |_| activate());
        subscription
    }

    /// Register a listener to be called when the given focus handle or one of its descendants loses focus.
    /// Returns a subscription and persists until the subscription is dropped.
    pub fn on_focus_out(
        &mut self,
        handle: &FocusHandle,
        cx: &mut App,
        mut listener: impl FnMut(FocusOutEvent, &mut Window, &mut App) + 'static,
    ) -> Subscription {
        let focus_id = handle.id;
        let (subscription, activate) =
            self.new_focus_listener(Box::new(move |event, window, cx| {
                if let Some(blurred_id) = event.previous_focus_path.last().copied()
                    && event.is_focus_out(focus_id)
                {
                    let event = FocusOutEvent {
                        blurred: WeakFocusHandle {
                            id: blurred_id,
                            handles: Arc::downgrade(&cx.focus_handles),
                        },
                    };
                    listener(event, window, cx)
                }
                true
            }));
        cx.defer(move |_| activate());
        subscription
    }

    fn reset_cursor_style(&self, cx: &mut App) {
        // Set the cursor only if we're the active window.
        if self.is_window_hovered() {
            let style = self
                .rendered_frame
                .cursor_style(self)
                .unwrap_or(CursorStyle::Arrow);
            cx.platform.set_cursor_style(style);
        }
    }

    /// Dispatch a given keystroke as though the user had typed it.
    /// You can create a keystroke with Keystroke::parse("").
    pub fn dispatch_keystroke(&mut self, keystroke: Keystroke, cx: &mut App) -> bool {
        let keystroke = keystroke.with_simulated_ime();
        let result = self.dispatch_event(
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }),
            cx,
        );
        if !result.propagate {
            return true;
        }

        if let Some(input) = keystroke.key_char
            && let Some(mut input_handler) = self.platform_window.take_input_handler()
        {
            input_handler.dispatch_input(&input, self, cx);
            self.platform_window.set_input_handler(input_handler);
            return true;
        }

        false
    }

    /// Return a key binding string for an action, to display in the UI. Uses the highest precedence
    /// binding for the action (last binding added to the keymap).
    pub fn keystroke_text_for(&self, action: &dyn Action) -> String {
        self.highest_precedence_binding_for_action(action)
            .map(|binding| {
                binding
                    .keystrokes()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| action.name().to_string())
    }

    /// Dispatch a mouse or keyboard event on the window.
    #[profiling::function]
    pub fn dispatch_event(&mut self, event: PlatformInput, cx: &mut App) -> DispatchEventResult {
        #[cfg(feature = "input-latency-histogram")]
        let dispatch_time = Instant::now();
        let update_count_before = self.invalidator.update_count();
        // Track input modality for focus-visible styling and hover suppression.
        // Hover is suppressed during keyboard modality so that keyboard navigation
        // doesn't show hover highlights on the item under the mouse cursor.
        let old_modality = self.last_input_modality;
        self.last_input_modality = match &event {
            PlatformInput::KeyDown(_) => InputModality::Keyboard,
            PlatformInput::MouseMove(_) | PlatformInput::MouseDown(_) => InputModality::Mouse,
            PlatformInput::Touch(_) => InputModality::Touch,
            _ => self.last_input_modality,
        };
        // Riseonly 0069: the switch refreshed the window, so every cached view drew again on the
        // first mouse move after a key (X1388: 79 views, 10 ms, after Esc closed a panel). Only
        // the views that read the modality while drawing (a hitbox under the mouse, focus-visible
        // styles, focus rings, tooltip checks) are notified.
        if self.last_input_modality != old_modality {
            let mut readers = std::mem::take(&mut *self.modality_readers.borrow_mut());
            // Riseonly 0111: a view that once drew a hitbox under the mouse stayed a reader after
            // the mouse left it, so the first press after Esc drew the whole chats list again
            // beside the chat (X2331: 20 rows, 133 views).
            let hover_readers = std::mem::take(&mut *self.hover_modality_readers.borrow_mut());
            for (view, hitbox) in hover_readers {
                if hitbox.hit_test(self) {
                    readers.insert(view);
                }
            }
            for view in readers {
                cx.notify(view);
            }
        }

        // Handlers may set this to false by calling `stop_propagation`.
        cx.propagate_event = true;
        // Handlers may set this to true by calling `prevent_default`.
        self.default_prevented = false;

        let event = if self.zoom == 1.0 { event } else { unzoom_input(event, self.zoom) };
        let event = match event {
            // Track the mouse position with our own state, since accessing the platform
            // API for the mouse position can only occur on the main thread.
            PlatformInput::MouseMove(mouse_move) => {
                self.mouse_position = mouse_move.position;
                self.modifiers = mouse_move.modifiers;
                PlatformInput::MouseMove(mouse_move)
            }
            PlatformInput::MouseDown(mouse_down) => {
                self.mouse_position = mouse_down.position;
                self.modifiers = mouse_down.modifiers;
                PlatformInput::MouseDown(mouse_down)
            }
            PlatformInput::MouseUp(mouse_up) => {
                self.mouse_position = mouse_up.position;
                self.modifiers = mouse_up.modifiers;
                PlatformInput::MouseUp(mouse_up)
            }
            PlatformInput::MousePressure(mouse_pressure) => {
                PlatformInput::MousePressure(mouse_pressure)
            }
            PlatformInput::MouseExited(mouse_exited) => {
                self.modifiers = mouse_exited.modifiers;
                PlatformInput::MouseExited(mouse_exited)
            }
            PlatformInput::ModifiersChanged(modifiers_changed) => {
                self.modifiers = modifiers_changed.modifiers;
                self.capslock = modifiers_changed.capslock;
                PlatformInput::ModifiersChanged(modifiers_changed)
            }
            PlatformInput::ScrollWheel(scroll_wheel) => {
                self.mouse_position = scroll_wheel.position;
                self.modifiers = scroll_wheel.modifiers;
                PlatformInput::ScrollWheel(scroll_wheel)
            }
            PlatformInput::Pinch(pinch) => {
                self.mouse_position = pinch.position;
                self.modifiers = pinch.modifiers;
                PlatformInput::Pinch(pinch)
            }
            // Translate dragging and dropping of external files from the operating system
            // to internal drag and drop events.
            PlatformInput::FileDrop(file_drop) => match file_drop {
                FileDropEvent::Entered { position, paths } => {
                    self.mouse_position = position;
                    let source_window = self.handle.window_id();
                    if !cx.restore_platform_drag(source_window) && cx.active_drag.is_none() {
                        cx.active_drag = Some(AnyDrag {
                            value: Arc::new(paths.clone()),
                            view: cx.new(|_| paths).into(),
                            cursor_offset: position,
                            cursor_style: None,
                            external_payload_source: None,
                        });
                    }
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::default(),
                    })
                }
                FileDropEvent::Pending { position } => {
                    self.mouse_position = position;
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::default(),
                    })
                }
                FileDropEvent::Submit { position } => {
                    cx.activate(true);
                    self.mouse_position = position;
                    PlatformInput::MouseUp(MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    })
                }
                FileDropEvent::Exited => {
                    if !cx.hand_restored_drag_to_platform(self.handle.window_id()) {
                        cx.active_drag.take();
                    }
                    self.refresh();
                    PlatformInput::FileDrop(FileDropEvent::Exited)
                }
                FileDropEvent::Ended => {
                    cx.end_platform_drag(self.handle.window_id());
                    self.refresh();
                    PlatformInput::FileDrop(FileDropEvent::Ended)
                }
            },
            PlatformInput::Touch(touch) => PlatformInput::Touch(touch),
            PlatformInput::KeyDown(_) | PlatformInput::KeyUp(_) => event,
        };

        if let Some(any_mouse_event) = event.mouse_event() {
            self.dispatch_mouse_event(any_mouse_event, cx);
        } else if let Some(any_key_event) = event.keyboard_event() {
            self.dispatch_key_event(any_key_event, cx);
        }

        // Must run after the move is dispatched: the platform owns the gesture afterwards, so this
        // is the last chance for drag listeners to see the pointer leave and reset their state.
        self.promote_external_drag_to_platform(&event, cx);

        if self.invalidator.update_count() > update_count_before {
            self.input_rate_tracker.borrow_mut().record_input();
            #[cfg(feature = "input-latency-histogram")]
            if self.invalidator.not_drawing() {
                self.input_latency_tracker.record_input(dispatch_time);
            } else {
                self.input_latency_tracker.record_mid_draw_input();
            }
        }

        DispatchEventResult {
            propagate: cx.propagate_event,
            default_prevented: self.default_prevented,
        }
    }

    fn promote_external_drag_to_platform(&mut self, event: &PlatformInput, cx: &mut App) {
        let PlatformInput::MouseMove(mouse_move) = event else {
            return;
        };
        if mouse_move.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if Bounds::new(Point::default(), self.viewport_size).contains(&mouse_move.position) {
            return;
        }
        if !self.platform_window.can_start_external_drag() {
            return;
        }
        let Some(payload_source) = cx
            .active_drag
            .as_mut()
            .and_then(|drag| drag.external_payload_source.take())
        else {
            return;
        };
        let Some(payload) = payload_source(self, cx) else {
            return;
        };
        if self.platform_window.start_external_drag(&payload)
            && cx.hand_active_drag_to_platform(self.handle.window_id())
        {
            self.refresh();
        }
    }

    fn dispatch_mouse_event(&mut self, event: &dyn Any, cx: &mut App) {
        let hit_test = self.rendered_frame.hit_test(self.mouse_position());
        if hit_test != self.mouse_hit_test {
            self.mouse_hit_test = hit_test;
            self.reset_cursor_style(cx);
        }

        // Riseonly 0044: a view replayed moved keeps listeners of its old origin; the mouse
        // reaching it makes it render, so what the mouse does next reads fresh positions.
        let mouse_position = self.mouse_position;
        let reached: SmallVec<[EntityId; 2]> = self
            .rendered_frame
            .moved_views
            .iter()
            .filter(|(_, bounds)| bounds.contains(&mouse_position))
            .map(|(entity_id, _)| *entity_id)
            .collect();
        for entity_id in reached {
            cx.notify(entity_id);
        }

        #[cfg(any(feature = "inspector", debug_assertions))]
        if self.is_inspector_picking(cx) {
            self.handle_inspector_mouse_event(event, cx);
            // When inspector is picking, all other mouse handling is skipped.
            return;
        }

        // Riseonly 0129: a wheel over the margin of a width capped column (an element with
        // `wheel_into_children`) is dispatched at the nearest point of the column, with the hit
        // test of that point, so the column scrolls from anywhere in its box. Hover and the
        // pointer position are given back right after.
        let mut redirected_wheel = None;
        let mut held_pointer = None;
        if let Some(wheel) = event.downcast_ref::<crate::ScrollWheelEvent>() {
            let mut position = wheel.position;
            let mut redirected_hit_test: Option<HitTest> = None;
            for _ in 0..4 {
                let current = redirected_hit_test.as_ref().unwrap_or(&self.mouse_hit_test);
                let Some(next) = self.rendered_frame.wheel_redirect(position, current) else {
                    break;
                };
                position = next;
                redirected_hit_test = Some(self.rendered_frame.hit_test(position));
            }
            if let Some(hit_test) = redirected_hit_test {
                held_pointer = Some((
                    mem::replace(&mut self.mouse_hit_test, hit_test),
                    mem::replace(&mut self.mouse_position, position),
                ));
                redirected_wheel = Some(crate::ScrollWheelEvent {
                    position,
                    ..wheel.clone()
                });
            }
        }
        let event: &dyn Any = match redirected_wheel.as_ref() {
            Some(wheel) => wheel,
            None => event,
        };

        let mut mouse_listeners = mem::take(&mut self.rendered_frame.mouse_listeners);

        // Capture phase, events bubble from back to front. Handlers for this phase are used for
        // special purposes, such as detecting events outside of a given Bounds.
        for listener in &mut mouse_listeners {
            let listener = listener.as_mut().unwrap();
            listener(event, DispatchPhase::Capture, self, cx);
            if !cx.propagate_event {
                break;
            }
        }

        // Bubble phase, where most normal handlers do their work.
        if cx.propagate_event {
            for listener in mouse_listeners.iter_mut().rev() {
                let listener = listener.as_mut().unwrap();
                listener(event, DispatchPhase::Bubble, self, cx);
                if !cx.propagate_event {
                    break;
                }
            }
        }

        self.rendered_frame.mouse_listeners = mouse_listeners;

        if let Some((hit_test, position)) = held_pointer {
            self.mouse_hit_test = hit_test;
            self.mouse_position = position;
        }

        if cx.has_active_drag() {
            if event.is::<MouseMoveEvent>() {
                // If this was a mouse move event, redraw the window so that the
                // active drag can follow the mouse cursor.
                self.refresh();
            } else if event.is::<MouseUpEvent>() {
                // If this was a mouse up event, cancel the active drag and redraw
                // the window.
                cx.active_drag = None;
                self.refresh();
            }
        }

        // Auto-release pointer capture on mouse up
        if event.is::<MouseUpEvent>() && self.captured_hitbox.is_some() {
            self.captured_hitbox = None;
        }
    }

    fn dispatch_key_event(&mut self, event: &dyn Any, cx: &mut App) {
        if self.invalidator.is_dirty() {
            self.draw(cx).clear(cx);
        }

        let node_id = self.focus_node_id_in_rendered_frame(self.focus);
        let dispatch_path = self.rendered_frame.dispatch_tree.dispatch_path(node_id);

        let mut keystroke: Option<Keystroke> = None;

        if let Some(event) = event.downcast_ref::<ModifiersChangedEvent>() {
            if event.modifiers.number_of_modifiers() == 0
                && self.pending_modifier.modifiers.number_of_modifiers() == 1
                && !self.pending_modifier.saw_other_input
            {
                let key = match self.pending_modifier.modifiers {
                    modifiers if modifiers.shift => Some("shift"),
                    modifiers if modifiers.control => Some("control"),
                    modifiers if modifiers.alt => Some("alt"),
                    modifiers if modifiers.platform => Some("platform"),
                    modifiers if modifiers.function => Some("function"),
                    _ => None,
                };
                if let Some(key) = key {
                    keystroke = Some(Keystroke {
                        key: key.to_string(),
                        key_char: None,
                        modifiers: Modifiers::default(),
                    });
                }
            }

            if self.pending_modifier.modifiers.number_of_modifiers() == 0
                && event.modifiers.number_of_modifiers() == 1
            {
                self.pending_modifier.saw_other_input = false
            } else if event.modifiers.number_of_modifiers() > 1 {
                self.pending_modifier.saw_other_input = true
            }
            self.pending_modifier.modifiers = event.modifiers
        } else if let Some(key_down_event) = event.downcast_ref::<KeyDownEvent>() {
            self.pending_modifier.saw_other_input = true;
            keystroke = Some(key_down_event.keystroke.clone());
            if key_down_event.keystroke.key_char.is_some()
                && matches!(
                    cx.cursor_hide_mode,
                    CursorHideMode::OnTyping | CursorHideMode::OnTypingAndAction
                )
            {
                cx.platform.hide_cursor_until_mouse_moves();
            }
        }

        let Some(keystroke) = keystroke else {
            self.finish_dispatch_key_event(event, dispatch_path, self.context_stack(), cx);
            return;
        };

        cx.propagate_event = true;
        self.dispatch_keystroke_interceptors(event, self.context_stack(), cx);
        if !cx.propagate_event {
            self.finish_dispatch_key_event(event, dispatch_path, self.context_stack(), cx);
            return;
        }

        let mut currently_pending = self.pending_input.take().unwrap_or_default();
        if currently_pending.focus.is_some() && currently_pending.focus != self.focus {
            currently_pending = PendingInput::default();
        }

        let match_result = self.rendered_frame.dispatch_tree.dispatch_key(
            currently_pending.keystrokes,
            keystroke,
            &dispatch_path,
        );

        if !match_result.to_replay.is_empty() {
            self.replay_pending_input(match_result.to_replay, cx);
            cx.propagate_event = true;
        }

        if !match_result.pending.is_empty() {
            currently_pending.timer.take();
            currently_pending.keystrokes = match_result.pending;
            currently_pending.focus = self.focus;

            let text_input_requires_timeout = event
                .downcast_ref::<KeyDownEvent>()
                .filter(|key_down| key_down.keystroke.key_char.is_some())
                .and_then(|_| self.platform_window.take_input_handler())
                .map_or(false, |mut input_handler| {
                    let accepts = input_handler.accepts_text_input(self, cx);
                    self.platform_window.set_input_handler(input_handler);
                    accepts
                });

            currently_pending.needs_timeout |=
                match_result.pending_has_binding || text_input_requires_timeout;

            if currently_pending.needs_timeout {
                currently_pending.timer = Some(self.spawn(cx, async move |cx| {
                    cx.background_executor.timer(Duration::from_secs(1)).await;
                    cx.update(move |window, cx| {
                        let Some(currently_pending) = window
                            .pending_input
                            .take()
                            .filter(|pending| pending.focus == window.focus)
                        else {
                            return;
                        };

                        let node_id = window.focus_node_id_in_rendered_frame(window.focus);
                        let dispatch_path =
                            window.rendered_frame.dispatch_tree.dispatch_path(node_id);

                        let to_replay = window
                            .rendered_frame
                            .dispatch_tree
                            .flush_dispatch(currently_pending.keystrokes, &dispatch_path);

                        window.pending_input_changed(cx);
                        window.replay_pending_input(to_replay, cx)
                    })
                    .log_err();
                }));
            } else {
                currently_pending.timer = None;
            }
            self.pending_input = Some(currently_pending);
            self.pending_input_changed(cx);
            cx.propagate_event = false;
            return;
        }

        let skip_bindings = event
            .downcast_ref::<KeyDownEvent>()
            .filter(|key_down_event| key_down_event.prefer_character_input)
            .map(|_| {
                self.platform_window
                    .take_input_handler()
                    .map_or(false, |mut input_handler| {
                        let accepts = input_handler.accepts_text_input(self, cx);
                        self.platform_window.set_input_handler(input_handler);
                        // If modifiers are not excessive (e.g. AltGr), and the input handler is accepting text input,
                        // we prefer the text input over bindings.
                        accepts
                    })
            })
            .unwrap_or(false);

        if !skip_bindings {
            for binding in match_result.bindings {
                self.dispatch_action_on_node(node_id, binding.action.as_ref(), cx);
                if !cx.propagate_event {
                    self.dispatch_keystroke_observers(
                        event,
                        Some(binding.action),
                        match_result.context_stack,
                        cx,
                    );
                    self.pending_input_changed(cx);
                    return;
                }
            }
        }

        self.finish_dispatch_key_event(event, dispatch_path, match_result.context_stack, cx);
        self.pending_input_changed(cx);
    }

    fn finish_dispatch_key_event(
        &mut self,
        event: &dyn Any,
        dispatch_path: SmallVec<[DispatchNodeId; 32]>,
        context_stack: Vec<KeyContext>,
        cx: &mut App,
    ) {
        self.dispatch_key_down_up_event(event, &dispatch_path, cx);
        if !cx.propagate_event {
            return;
        }

        self.dispatch_modifiers_changed_event(event, &dispatch_path, cx);
        if !cx.propagate_event {
            return;
        }

        self.dispatch_keystroke_observers(event, None, context_stack, cx);
    }

    pub(crate) fn pending_input_changed(&mut self, cx: &mut App) {
        self.pending_input_observers
            .clone()
            .retain(&(), |callback| callback(self, cx));
    }

    fn dispatch_key_down_up_event(
        &mut self,
        event: &dyn Any,
        dispatch_path: &SmallVec<[DispatchNodeId; 32]>,
        cx: &mut App,
    ) {
        // Capture phase
        for node_id in dispatch_path {
            let node = self.rendered_frame.dispatch_tree.node(*node_id);

            for key_listener in node.key_listeners.clone() {
                key_listener(event, DispatchPhase::Capture, self, cx);
                if !cx.propagate_event {
                    return;
                }
            }
        }

        // Bubble phase
        for node_id in dispatch_path.iter().rev() {
            // Handle low level key events
            let node = self.rendered_frame.dispatch_tree.node(*node_id);
            for key_listener in node.key_listeners.clone() {
                key_listener(event, DispatchPhase::Bubble, self, cx);
                if !cx.propagate_event {
                    return;
                }
            }
        }
    }

    fn dispatch_modifiers_changed_event(
        &mut self,
        event: &dyn Any,
        dispatch_path: &SmallVec<[DispatchNodeId; 32]>,
        cx: &mut App,
    ) {
        let Some(event) = event.downcast_ref::<ModifiersChangedEvent>() else {
            return;
        };
        for node_id in dispatch_path.iter().rev() {
            let node = self.rendered_frame.dispatch_tree.node(*node_id);
            for listener in node.modifiers_changed_listeners.clone() {
                listener(event, self, cx);
                if !cx.propagate_event {
                    return;
                }
            }
        }
    }

    /// Pending input that can still complete a binding. Input left over from a previous focus can
    /// never complete one.
    fn active_pending_input(&self) -> Option<&PendingInput> {
        self.pending_input
            .as_ref()
            .filter(|pending_input| pending_input.focus == self.focus)
    }

    /// Determine whether a potential multi-stroke key binding is in progress on this window.
    pub fn has_pending_keystrokes(&self) -> bool {
        self.active_pending_input().is_some()
    }

    pub(crate) fn clear_pending_keystrokes(&mut self) {
        self.pending_input.take();
    }

    /// Returns the currently pending input keystrokes that might result in a multi-stroke key binding.
    pub fn pending_input_keystrokes(&self) -> Option<&[Keystroke]> {
        self.active_pending_input()
            .map(|pending_input| pending_input.keystrokes.as_slice())
    }

    fn replay_pending_input(&mut self, replays: SmallVec<[Replay; 1]>, cx: &mut App) {
        let node_id = self.focus_node_id_in_rendered_frame(self.focus);
        let dispatch_path = self.rendered_frame.dispatch_tree.dispatch_path(node_id);

        'replay: for replay in replays {
            let event = KeyDownEvent {
                keystroke: replay.keystroke.clone(),
                is_held: false,
                prefer_character_input: true,
            };

            cx.propagate_event = true;
            for binding in replay.bindings {
                self.dispatch_action_on_node(node_id, binding.action.as_ref(), cx);
                if !cx.propagate_event {
                    self.dispatch_keystroke_observers(
                        &event,
                        Some(binding.action),
                        Vec::default(),
                        cx,
                    );
                    continue 'replay;
                }
            }

            self.dispatch_key_down_up_event(&event, &dispatch_path, cx);
            if !cx.propagate_event {
                continue 'replay;
            }
            if let Some(input) = replay.keystroke.key_char.as_ref().cloned()
                && let Some(mut input_handler) = self.platform_window.take_input_handler()
            {
                input_handler.dispatch_input(&input, self, cx);
                self.platform_window.set_input_handler(input_handler)
            }
        }
    }

    fn focus_node_id_in_rendered_frame(&self, focus_id: Option<FocusId>) -> DispatchNodeId {
        focus_id
            .and_then(|focus_id| {
                self.rendered_frame
                    .dispatch_tree
                    .focusable_node_id(focus_id)
            })
            .unwrap_or_else(|| self.rendered_frame.dispatch_tree.root_node_id())
    }

    fn dispatch_action_on_node(
        &mut self,
        node_id: DispatchNodeId,
        action: &dyn Action,
        cx: &mut App,
    ) {
        self.dispatch_action_on_node_inner(node_id, action, cx);

        if !cx.propagate_event
            && cx.cursor_hide_mode == CursorHideMode::OnTypingAndAction
            && self.last_input_modality == InputModality::Keyboard
        {
            cx.platform.hide_cursor_until_mouse_moves();
        }
    }

    fn dispatch_action_on_node_inner(
        &mut self,
        node_id: DispatchNodeId,
        action: &dyn Action,
        cx: &mut App,
    ) {
        let dispatch_path = self.rendered_frame.dispatch_tree.dispatch_path(node_id);

        // Capture phase for global actions.
        cx.propagate_event = true;
        if let Some(mut global_listeners) = cx
            .global_action_listeners
            .remove(&action.as_any().type_id())
        {
            for listener in &global_listeners {
                profiler::update_running_action(action, cx);
                listener(action.as_any(), DispatchPhase::Capture, cx);
                profiler::save_action_timing();
                if !cx.propagate_event {
                    break;
                }
            }

            global_listeners.extend(
                cx.global_action_listeners
                    .remove(&action.as_any().type_id())
                    .unwrap_or_default(),
            );

            cx.global_action_listeners
                .insert(action.as_any().type_id(), global_listeners);
        }

        if !cx.propagate_event {
            return;
        }

        // Capture phase for window actions.
        for node_id in &dispatch_path {
            let node = self.rendered_frame.dispatch_tree.node(*node_id);
            for DispatchActionListener {
                action_type,
                listener,
            } in node.action_listeners.clone()
            {
                let any_action = action.as_any();
                if action_type == any_action.type_id() {
                    profiler::update_running_action(action, cx);
                    listener(any_action, DispatchPhase::Capture, self, cx);
                    profiler::save_action_timing();

                    if !cx.propagate_event {
                        return;
                    }
                }
            }
        }

        // Bubble phase for window actions.
        for node_id in dispatch_path.iter().rev() {
            let node = self.rendered_frame.dispatch_tree.node(*node_id);
            for DispatchActionListener {
                action_type,
                listener,
            } in node.action_listeners.clone()
            {
                let any_action = action.as_any();
                if action_type == any_action.type_id() {
                    cx.propagate_event = false; // Actions stop propagation by default during the bubble phase
                    profiler::update_running_action(action, cx);
                    listener(any_action, DispatchPhase::Bubble, self, cx);
                    profiler::save_action_timing();

                    if !cx.propagate_event {
                        return;
                    }
                }
            }
        }

        // Bubble phase for global actions.
        if let Some(mut global_listeners) = cx
            .global_action_listeners
            .remove(&action.as_any().type_id())
        {
            for listener in global_listeners.iter().rev() {
                cx.propagate_event = false; // Actions stop propagation by default during the bubble phase

                profiler::update_running_action(action, cx);
                listener(action.as_any(), DispatchPhase::Bubble, cx);
                profiler::save_action_timing();
                if !cx.propagate_event {
                    break;
                }
            }

            global_listeners.extend(
                cx.global_action_listeners
                    .remove(&action.as_any().type_id())
                    .unwrap_or_default(),
            );

            cx.global_action_listeners
                .insert(action.as_any().type_id(), global_listeners);
        }
    }

    /// Register the given handler to be invoked whenever the global of the given type
    /// is updated.
    pub fn observe_global<G: Global>(
        &mut self,
        cx: &mut App,
        f: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Subscription {
        let window_handle = self.handle;
        let (subscription, activate) = cx.global_observers.insert(
            TypeId::of::<G>(),
            Box::new(move |cx| {
                window_handle
                    .update(cx, |_, window, cx| f(window, cx))
                    .is_ok()
            }),
        );
        cx.defer(move |_| activate());
        subscription
    }

    /// Focus the current window and bring it to the foreground at the platform level.
    pub fn activate_window(&self) {
        self.platform_window.activate();
    }

    /// Requests that the operating system draw attention to this window.
    pub fn request_attention(&self) {
        self.platform_window.request_attention();
    }

    /// Minimize the current window at the platform level.
    pub fn minimize_window(&self) {
        self.platform_window.minimize();
    }

    /// Toggle full screen status on the current window at the platform level.
    pub fn toggle_fullscreen(&self) {
        self.platform_window.toggle_fullscreen();
    }

    /// Updates the IME panel position suggestions for languages like japanese, chinese.
    pub fn invalidate_character_coordinates(&self) {
        self.on_next_frame(|window, cx| {
            if let Some(mut input_handler) = window.platform_window.take_input_handler() {
                if let Some(bounds) = input_handler.selected_bounds(window, cx) {
                    window.platform_window.update_ime_position(bounds);
                }
                window.platform_window.set_input_handler(input_handler);
            }
        });
    }

    /// Present a platform dialog.
    /// The provided message will be presented, along with buttons for each answer.
    /// When a button is clicked, the returned Receiver will receive the index of the clicked button.
    pub fn prompt<T>(
        &mut self,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[T],
        cx: &mut App,
    ) -> oneshot::Receiver<usize>
    where
        T: Clone + Into<PromptButton>,
    {
        let prompt_builder = cx.prompt_builder.take();
        let Some(prompt_builder) = prompt_builder else {
            unreachable!("Re-entrant window prompting is not supported by GPUI");
        };

        let answers = answers
            .iter()
            .map(|answer| answer.clone().into())
            .collect::<Vec<_>>();

        let receiver = match &prompt_builder {
            PromptBuilder::Default => self
                .platform_window
                .prompt(level, message, detail, &answers)
                .unwrap_or_else(|| {
                    self.build_custom_prompt(&prompt_builder, level, message, detail, &answers, cx)
                }),
            PromptBuilder::Custom(_) => {
                self.build_custom_prompt(&prompt_builder, level, message, detail, &answers, cx)
            }
        };

        cx.prompt_builder = Some(prompt_builder);

        receiver
    }

    fn build_custom_prompt(
        &mut self,
        prompt_builder: &PromptBuilder,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
        cx: &mut App,
    ) -> oneshot::Receiver<usize> {
        let (sender, receiver) = oneshot::channel();
        let handle = PromptHandle::new(sender);
        let handle = (prompt_builder)(level, message, detail, answers, handle, self, cx);
        self.prompt = Some(handle);
        receiver
    }

    /// Returns whether a prompt rendered by GPUI is currently active in this window.
    ///
    /// This is only true for prompts rendered in the window (see
    /// [`App::set_prompt_builder`]), not for platform-native prompt dialogs.
    pub fn has_active_prompt(&self) -> bool {
        self.prompt.is_some()
    }

    /// Returns the current context stack.
    pub fn context_stack(&self) -> Vec<KeyContext> {
        let node_id = self.focus_node_id_in_rendered_frame(self.focus);
        let dispatch_tree = &self.rendered_frame.dispatch_tree;
        dispatch_tree
            .dispatch_path(node_id)
            .iter()
            .filter_map(move |&node_id| dispatch_tree.node(node_id).context.clone())
            .collect()
    }

    /// Returns all available actions for the focused element.
    pub fn available_actions(&self, cx: &App) -> Vec<Box<dyn Action>> {
        let node_id = self.focus_node_id_in_rendered_frame(self.focus);
        let mut actions = self.rendered_frame.dispatch_tree.available_actions(node_id);
        for action_type in cx.global_action_listeners.keys() {
            if let Err(ix) = actions.binary_search_by_key(action_type, |a| a.as_any().type_id()) {
                let action = cx.actions.build_action_type(action_type).ok();
                if let Some(action) = action {
                    actions.insert(ix, action);
                }
            }
        }
        actions
    }

    /// Returns key bindings that invoke an action on the currently focused element. Bindings are
    /// returned in the order they were added. For display, the last binding should take precedence.
    pub fn bindings_for_action(&self, action: &dyn Action) -> Vec<KeyBinding> {
        self.rendered_frame
            .dispatch_tree
            .bindings_for_action(action, &self.rendered_frame.dispatch_tree.context_stack)
    }

    /// Returns the highest precedence key binding that invokes an action on the currently focused
    /// element. This is more efficient than getting the last result of `bindings_for_action`.
    pub fn highest_precedence_binding_for_action(&self, action: &dyn Action) -> Option<KeyBinding> {
        self.rendered_frame
            .dispatch_tree
            .highest_precedence_binding_for_action(
                action,
                &self.rendered_frame.dispatch_tree.context_stack,
            )
    }

    /// Returns the key bindings for an action in a context.
    pub fn bindings_for_action_in_context(
        &self,
        action: &dyn Action,
        context: KeyContext,
    ) -> Vec<KeyBinding> {
        let dispatch_tree = &self.rendered_frame.dispatch_tree;
        dispatch_tree.bindings_for_action(action, &[context])
    }

    /// Returns the highest precedence key binding for an action in a context. This is more
    /// efficient than getting the last result of `bindings_for_action_in_context`.
    pub fn highest_precedence_binding_for_action_in_context(
        &self,
        action: &dyn Action,
        context: KeyContext,
    ) -> Option<KeyBinding> {
        let dispatch_tree = &self.rendered_frame.dispatch_tree;
        dispatch_tree.highest_precedence_binding_for_action(action, &[context])
    }

    /// Returns any bindings that would invoke an action on the given focus handle if it were
    /// focused. Bindings are returned in the order they were added. For display, the last binding
    /// should take precedence.
    pub fn bindings_for_action_in(
        &self,
        action: &dyn Action,
        focus_handle: &FocusHandle,
    ) -> Vec<KeyBinding> {
        let dispatch_tree = &self.rendered_frame.dispatch_tree;
        let Some(context_stack) = self.context_stack_for_focus_handle(focus_handle) else {
            return vec![];
        };
        dispatch_tree.bindings_for_action(action, &context_stack)
    }

    /// Returns the highest precedence key binding that would invoke an action on the given focus
    /// handle if it were focused. This is more efficient than getting the last result of
    /// `bindings_for_action_in`.
    pub fn highest_precedence_binding_for_action_in(
        &self,
        action: &dyn Action,
        focus_handle: &FocusHandle,
    ) -> Option<KeyBinding> {
        let dispatch_tree = &self.rendered_frame.dispatch_tree;
        let context_stack = self.context_stack_for_focus_handle(focus_handle)?;
        dispatch_tree.highest_precedence_binding_for_action(action, &context_stack)
    }

    /// Find the bindings that can follow the current input sequence for the current context stack.
    pub fn possible_bindings_for_input(&self, input: &[Keystroke]) -> Vec<KeyBinding> {
        self.rendered_frame
            .dispatch_tree
            .possible_next_bindings_for_input(input, &self.context_stack())
    }

    fn context_stack_for_focus_handle(
        &self,
        focus_handle: &FocusHandle,
    ) -> Option<Vec<KeyContext>> {
        let dispatch_tree = &self.rendered_frame.dispatch_tree;
        let node_id = dispatch_tree.focusable_node_id(focus_handle.id)?;
        let context_stack: Vec<_> = dispatch_tree
            .dispatch_path(node_id)
            .into_iter()
            .filter_map(|node_id| dispatch_tree.node(node_id).context.clone())
            .collect();
        Some(context_stack)
    }

    /// Returns a generic event listener that invokes the given listener with the view and context associated with the given view handle.
    pub fn listener_for<T: 'static, E>(
        &self,
        view: &Entity<T>,
        f: impl Fn(&mut T, &E, &mut Window, &mut Context<T>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let view = view.downgrade();
        move |e: &E, window: &mut Window, cx: &mut App| {
            view.update(cx, |view, cx| f(view, e, window, cx)).ok();
        }
    }

    /// Returns a generic handler that invokes the given handler with the view and context associated with the given view handle.
    pub fn handler_for<E: 'static, Callback: Fn(&mut E, &mut Window, &mut Context<E>) + 'static>(
        &self,
        entity: &Entity<E>,
        f: Callback,
    ) -> impl Fn(&mut Window, &mut App) + 'static {
        let entity = entity.downgrade();
        move |window: &mut Window, cx: &mut App| {
            entity.update(cx, |entity, cx| f(entity, window, cx)).ok();
        }
    }

    /// Register a callback that can interrupt the closing of the current window based the returned boolean.
    /// If the callback returns false, the window won't be closed.
    pub fn on_window_should_close(
        &self,
        cx: &App,
        f: impl Fn(&mut Window, &mut App) -> bool + 'static,
    ) {
        let mut cx = self.to_async(cx);
        self.platform_window.on_should_close(Box::new(move || {
            cx.update(|window, cx| f(window, cx)).unwrap_or(true)
        }))
    }

    /// Register an action listener on this node for the next frame. The type of action
    /// is determined by the first parameter of the given listener. When the next frame is rendered
    /// the listener will be cleared.
    ///
    /// This is a fairly low-level method, so prefer using action handlers on elements unless you have
    /// a specific need to register a listener yourself.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn on_action(
        &mut self,
        action_type: TypeId,
        listener: impl Fn(&dyn Any, DispatchPhase, &mut Window, &mut App) + 'static,
    ) {
        self.invalidator.debug_assert_paint();

        self.next_frame
            .dispatch_tree
            .on_action(action_type, Rc::new(listener));
    }

    /// Register a capturing action listener on this node for the next frame if the condition is true.
    /// The type of action is determined by the first parameter of the given listener. When the next
    /// frame is rendered the listener will be cleared.
    ///
    /// This is a fairly low-level method, so prefer using action handlers on elements unless you have
    /// a specific need to register a listener yourself.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn on_action_when(
        &mut self,
        condition: bool,
        action_type: TypeId,
        listener: impl Fn(&dyn Any, DispatchPhase, &mut Window, &mut App) + 'static,
    ) {
        self.invalidator.debug_assert_paint();

        if condition {
            self.next_frame
                .dispatch_tree
                .on_action(action_type, Rc::new(listener));
        }
    }

    /// Read information about the GPU backing this window.
    /// Currently returns None on Mac and Windows.
    pub fn gpu_specs(&self) -> Option<GpuSpecs> {
        self.platform_window.gpu_specs()
    }

    /// Perform titlebar double-click action.
    /// This is macOS specific.
    pub fn titlebar_double_click(&self) {
        self.platform_window
            .titlebar_double_click(self.is_resizable, self.is_minimizable);
    }

    /// Gets the window's title at the platform level.
    /// This is macOS specific.
    pub fn window_title(&self) -> String {
        self.platform_window.get_title()
    }

    /// Returns a list of all tabbed windows and their titles.
    /// This is macOS specific.
    pub fn tabbed_windows(&self) -> Option<Vec<SystemWindowTab>> {
        self.platform_window.tabbed_windows()
    }

    /// Returns the tab bar visibility.
    /// This is macOS specific.
    pub fn tab_bar_visible(&self) -> bool {
        self.platform_window.tab_bar_visible()
    }

    /// Merges all open windows into a single tabbed window.
    /// This is macOS specific.
    pub fn merge_all_windows(&self) {
        self.platform_window.merge_all_windows()
    }

    /// Moves the tab to a new containing window.
    /// This is macOS specific.
    pub fn move_tab_to_new_window(&self) {
        self.platform_window.move_tab_to_new_window()
    }

    /// Shows or hides the window tab overview.
    /// This is macOS specific.
    pub fn toggle_window_tab_overview(&self) {
        self.platform_window.toggle_window_tab_overview()
    }

    /// Sets the tabbing identifier for the window.
    /// This is macOS specific.
    pub fn set_tabbing_identifier(&self, tabbing_identifier: Option<String>) {
        self.platform_window
            .set_tabbing_identifier(tabbing_identifier)
    }

    /// Request the OS to play an alert sound. On some platforms this is associated
    /// with the window, for others it's just a simple global function call.
    pub fn play_system_bell(&self) {
        self.platform_window.play_system_bell()
    }

    /// Returns whether accessibility features are active for this frame,
    /// i.e. whether assistive technology (such as a screen reader) is
    /// connected and an accessibility tree is being built.
    ///
    /// Use this to skip computing data during rendering that is only
    /// observable through the accessibility tree. When accessibility is
    /// activated, a redraw is forced, so gated work is recomputed before the
    /// next tree update is sent to the platform.
    ///
    /// See the [accessibility guide](crate::_accessibility) for an overview.
    pub fn is_a11y_active(&self) -> bool {
        self.a11y.is_active()
    }

    /// Debug representation of the last frame's accessibility information.
    pub fn debug_a11y_tree_json(&self) -> Option<String> {
        self.a11y.debug_tree_json()
    }

    /// Register a listener for an accessibility action on a specific node.
    /// The listener will be called when a screen reader requests the given
    /// action on the node identified by `node_id`.
    ///
    /// See the [accessibility guide](crate::_accessibility) for an overview.
    pub fn on_a11y_action(
        &mut self,
        node_id: accesskit::NodeId,
        action: accesskit::Action,
        listener: impl FnMut(Option<&accesskit::ActionData>, &mut Window, &mut App) + 'static,
    ) {
        self.a11y
            .action_listeners
            .entry(node_id)
            .or_default()
            .push((action, Box::new(listener)));
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn handle_a11y_action(&mut self, request: accesskit::ActionRequest, cx: &mut App) {
        // Take listeners out temporarily so the closures can borrow Window
        // mutably, then restore them afterward.
        if let Some(mut listeners) = self.a11y.action_listeners.remove(&request.target_node) {
            let extra_data = request.data.as_ref();
            let mut matched = false;
            for (action, listener) in &mut listeners {
                if *action == request.action {
                    listener(extra_data, self, cx);
                    matched = true;
                }
            }
            self.a11y
                .action_listeners
                .insert(request.target_node, listeners);
            if matched {
                return;
            }
        }

        // Fall back to built-in action handling.
        match request.action {
            accesskit::Action::Click => {
                if let Some(bounds) = self.a11y.node_bounds.get(&request.target_node).copied() {
                    let center = bounds.center();
                    let mouse_down = PlatformInput::MouseDown(crate::MouseDownEvent {
                        button: MouseButton::Left,
                        position: center,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    });
                    let mouse_up = PlatformInput::MouseUp(MouseUpEvent {
                        button: MouseButton::Left,
                        position: center,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    });
                    self.dispatch_event(mouse_down, cx);
                    self.dispatch_event(mouse_up, cx);
                }
            }
            accesskit::Action::Focus => {
                if let Some(focus_id) = self.a11y.focus_ids.get(&request.target_node).copied()
                    && let Some(handle) = FocusHandle::for_id(focus_id, &cx.focus_handles)
                {
                    self.focus(&handle, cx);
                }
            }
            accesskit::Action::Blur => {
                self.blur();
            }
            _ => {
                log::debug!(
                    "Unhandled a11y action: {:?} on {:?}",
                    request.action,
                    request.target_node
                );
            }
        }
    }

    /// Toggles the inspector mode on this window.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub fn toggle_inspector(&mut self, cx: &mut App) {
        self.inspector = match self.inspector {
            None => Some(cx.new(|_| Inspector::new())),
            Some(_) => None,
        };
        self.refresh();
    }

    /// Returns true if the window is in inspector mode.
    pub fn is_inspector_picking(&self, _cx: &App) -> bool {
        #[cfg(any(feature = "inspector", debug_assertions))]
        {
            if let Some(inspector) = &self.inspector {
                return inspector.read(_cx).is_picking();
            }
        }
        false
    }

    /// Executes the provided function with mutable access to an inspector state.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub fn with_inspector_state<T: 'static, R>(
        &mut self,
        _inspector_id: Option<&crate::InspectorElementId>,
        cx: &mut App,
        f: impl FnOnce(&mut Option<T>, &mut Self) -> R,
    ) -> R {
        if let Some(inspector_id) = _inspector_id
            && let Some(inspector) = &self.inspector
        {
            let inspector = inspector.clone();
            let active_element_id = inspector.read(cx).active_element_id();
            if Some(inspector_id) == active_element_id {
                return inspector.update(cx, |inspector, _cx| {
                    inspector.with_active_element_state(self, f)
                });
            }
        }
        f(&mut None, self)
    }

    #[cfg(any(feature = "inspector", debug_assertions))]
    pub(crate) fn build_inspector_element_id(
        &mut self,
        path: crate::InspectorElementPath,
    ) -> crate::InspectorElementId {
        self.invalidator.debug_assert_paint_or_prepaint();
        let path = Rc::new(path);
        let next_instance_id = self
            .next_frame
            .next_inspector_instance_ids
            .entry(path.clone())
            .or_insert(0);
        let instance_id = *next_instance_id;
        *next_instance_id += 1;
        crate::InspectorElementId { path, instance_id }
    }

    #[cfg(any(feature = "inspector", debug_assertions))]
    fn prepaint_inspector(&mut self, inspector_width: Pixels, cx: &mut App) -> Option<AnyElement> {
        if let Some(inspector) = self.inspector.take() {
            let mut inspector_element = AnyView::from(inspector.clone()).into_any_element();
            inspector_element.prepaint_as_root(
                point(self.viewport_size.width - inspector_width, px(0.0)),
                size(inspector_width, self.viewport_size.height).into(),
                self,
                cx,
            );
            self.inspector = Some(inspector);
            Some(inspector_element)
        } else {
            None
        }
    }

    #[cfg(any(feature = "inspector", debug_assertions))]
    fn paint_inspector(&mut self, mut inspector_element: Option<AnyElement>, cx: &mut App) {
        if let Some(mut inspector_element) = inspector_element {
            inspector_element.paint(self, cx);
        };
    }

    /// Registers a hitbox that can be used for inspector picking mode, allowing users to select and
    /// inspect UI elements by clicking on them.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub fn insert_inspector_hitbox(
        &mut self,
        hitbox_id: HitboxId,
        inspector_id: Option<&crate::InspectorElementId>,
        cx: &App,
    ) {
        self.invalidator.debug_assert_paint_or_prepaint();
        if !self.is_inspector_picking(cx) {
            return;
        }
        if let Some(inspector_id) = inspector_id {
            self.next_frame
                .inspector_hitboxes
                .insert(hitbox_id, inspector_id.clone());
        }
    }

    #[cfg(any(feature = "inspector", debug_assertions))]
    fn paint_inspector_hitbox(&mut self, cx: &App) {
        if let Some(inspector) = self.inspector.as_ref() {
            let inspector = inspector.read(cx);
            if let Some((hitbox_id, _)) = self.hovered_inspector_hitbox(inspector, &self.next_frame)
                && let Some(hitbox) = self
                    .next_frame
                    .hitboxes
                    .iter()
                    .find(|hitbox| hitbox.id == hitbox_id)
            {
                self.paint_quad(crate::fill(hitbox.bounds, crate::rgba(0x61afef4d)));
            }
        }
    }

    #[cfg(any(feature = "inspector", debug_assertions))]
    fn handle_inspector_mouse_event(&mut self, event: &dyn Any, cx: &mut App) {
        let Some(inspector) = self.inspector.clone() else {
            return;
        };
        if event.downcast_ref::<MouseMoveEvent>().is_some() {
            inspector.update(cx, |inspector, _cx| {
                if let Some((_, inspector_id)) =
                    self.hovered_inspector_hitbox(inspector, &self.rendered_frame)
                {
                    inspector.hover(inspector_id, self);
                }
            });
        } else if event.downcast_ref::<crate::MouseDownEvent>().is_some() {
            inspector.update(cx, |inspector, _cx| {
                if let Some((_, inspector_id)) =
                    self.hovered_inspector_hitbox(inspector, &self.rendered_frame)
                {
                    inspector.select(inspector_id, self);
                }
            });
        } else if let Some(event) = event.downcast_ref::<crate::ScrollWheelEvent>() {
            // This should be kept in sync with SCROLL_LINES in x11 platform.
            const SCROLL_LINES: f32 = 3.0;
            const SCROLL_PIXELS_PER_LAYER: f32 = 36.0;
            let delta_y = event
                .delta
                .pixel_delta(px(SCROLL_PIXELS_PER_LAYER / SCROLL_LINES))
                .y;
            if let Some(inspector) = self.inspector.clone() {
                inspector.update(cx, |inspector, _cx| {
                    if let Some(depth) = inspector.pick_depth.as_mut() {
                        *depth += f32::from(delta_y) / SCROLL_PIXELS_PER_LAYER;
                        let max_depth = self.mouse_hit_test.ids.len() as f32 - 0.5;
                        if *depth < 0.0 {
                            *depth = 0.0;
                        } else if *depth > max_depth {
                            *depth = max_depth;
                        }
                        if let Some((_, inspector_id)) =
                            self.hovered_inspector_hitbox(inspector, &self.rendered_frame)
                        {
                            inspector.set_active_element_id(inspector_id, self);
                        }
                    }
                });
            }
        }
    }

    #[cfg(any(feature = "inspector", debug_assertions))]
    fn hovered_inspector_hitbox(
        &self,
        inspector: &Inspector,
        frame: &Frame,
    ) -> Option<(HitboxId, crate::InspectorElementId)> {
        if let Some(pick_depth) = inspector.pick_depth {
            let depth = (pick_depth as i64).try_into().unwrap_or(0);
            let max_skipped = self.mouse_hit_test.ids.len().saturating_sub(1);
            let skip_count = (depth as usize).min(max_skipped);
            for hitbox_id in self.mouse_hit_test.ids.iter().skip(skip_count) {
                if let Some(inspector_id) = frame.inspector_hitboxes.get(hitbox_id) {
                    return Some((*hitbox_id, inspector_id.clone()));
                }
            }
        }
        None
    }

    /// For testing: set the current modifier keys state.
    /// This does not generate any events.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    /// For testing: simulate a mouse move event to the given position.
    /// This dispatches the event through the normal event handling path,
    /// which will trigger hover states and tooltips.
    #[cfg(any(test, feature = "test-support"))]
    pub fn simulate_mouse_move(&mut self, position: Point<Pixels>, cx: &mut App) {
        let event = PlatformInput::MouseMove(MouseMoveEvent {
            position,
            modifiers: self.modifiers,
            pressed_button: None,
        });
        let _ = self.dispatch_event(event, cx);
    }
}

// #[derive(Clone, Copy, Eq, PartialEq, Hash)]
slotmap::new_key_type! {
    /// A unique identifier for a window.
    pub struct WindowId;
}

impl WindowId {
    /// Converts this window ID to a `u64`.
    pub fn as_u64(&self) -> u64 {
        self.0.as_ffi()
    }
}

impl From<u64> for WindowId {
    fn from(value: u64) -> Self {
        WindowId(slotmap::KeyData::from_ffi(value))
    }
}

/// A handle to a window with a specific root view type.
/// Note that this does not keep the window alive on its own.
#[derive(Deref, DerefMut)]
pub struct WindowHandle<V> {
    #[deref]
    #[deref_mut]
    pub(crate) any_handle: AnyWindowHandle,
    state_type: PhantomData<fn(V) -> V>,
}

impl<V> Debug for WindowHandle<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowHandle")
            .field("any_handle", &self.any_handle.id.as_u64())
            .finish()
    }
}

impl<V: 'static + Render> WindowHandle<V> {
    /// Creates a new handle from a window ID.
    /// This does not check if the root type of the window is `V`.
    pub fn new(id: WindowId) -> Self {
        WindowHandle {
            any_handle: AnyWindowHandle {
                id,
                state_type: TypeId::of::<V>(),
            },
            state_type: PhantomData,
        }
    }

    /// Get the root view out of this window.
    ///
    /// This will fail if the window is closed or if the root view's type does not match `V`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn root<C>(&self, cx: &mut C) -> Result<Entity<V>>
    where
        C: AppContext,
    {
        cx.update_window(self.any_handle, |root_view, _, _| {
            root_view
                .downcast::<V>()
                .map_err(|_| anyhow!("the type of the window's root view has changed"))
        })?
    }

    /// Updates the root view of this window.
    ///
    /// This will fail if the window has been closed or if the root view's type does not match
    pub fn update<C, R>(
        &self,
        cx: &mut C,
        update: impl FnOnce(&mut V, &mut Window, &mut Context<V>) -> R,
    ) -> Result<R>
    where
        C: AppContext,
    {
        cx.update_window(self.any_handle, |root_view, window, cx| {
            let view = root_view
                .downcast::<V>()
                .map_err(|_| anyhow!("the type of the window's root view has changed"))?;

            Ok(view.update(cx, |view, cx| update(view, window, cx)))
        })?
    }

    /// Read the root view out of this window.
    ///
    /// This will fail if the window is closed or if the root view's type does not match `V`.
    pub fn read<'a>(&self, cx: &'a App) -> Result<&'a V> {
        let x = cx
            .windows
            .get(self.id)
            .and_then(|window| {
                window
                    .as_deref()
                    .and_then(|window| window.root.clone())
                    .map(|root_view| root_view.downcast::<V>())
            })
            .context("window not found")?
            .map_err(|_| anyhow!("the type of the window's root view has changed"))?;

        Ok(x.read(cx))
    }

    /// Read the root view out of this window, with a callback
    ///
    /// This will fail if the window is closed or if the root view's type does not match `V`.
    pub fn read_with<C, R>(&self, cx: &C, read_with: impl FnOnce(&V, &App) -> R) -> Result<R>
    where
        C: AppContext,
    {
        cx.read_window(self, |root_view, cx| read_with(root_view.read(cx), cx))
    }

    /// Read the root view pointer off of this window.
    ///
    /// This will fail if the window is closed or if the root view's type does not match `V`.
    pub fn entity<C>(&self, cx: &C) -> Result<Entity<V>>
    where
        C: AppContext,
    {
        cx.read_window(self, |root_view, _cx| root_view)
    }

    /// Check if this window is 'active'.
    ///
    /// Will return `None` if the window is closed or currently
    /// borrowed.
    pub fn is_active(&self, cx: &mut App) -> Option<bool> {
        cx.update_window(self.any_handle, |_, window, _| window.is_window_active())
            .ok()
    }
}

impl<V> Copy for WindowHandle<V> {}

impl<V> Clone for WindowHandle<V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> PartialEq for WindowHandle<V> {
    fn eq(&self, other: &Self) -> bool {
        self.any_handle == other.any_handle
    }
}

impl<V> Eq for WindowHandle<V> {}

impl<V> Hash for WindowHandle<V> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.any_handle.hash(state);
    }
}

impl<V: 'static> From<WindowHandle<V>> for AnyWindowHandle {
    fn from(val: WindowHandle<V>) -> Self {
        val.any_handle
    }
}

/// A handle to a window with any root view type, which can be downcast to a window with a specific root view type.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct AnyWindowHandle {
    pub(crate) id: WindowId,
    state_type: TypeId,
}

impl AnyWindowHandle {
    /// Get the ID of this window.
    pub fn window_id(&self) -> WindowId {
        self.id
    }

    /// Attempt to convert this handle to a window handle with a specific root view type.
    /// If the types do not match, this will return `None`.
    pub fn downcast<T: 'static>(&self) -> Option<WindowHandle<T>> {
        if TypeId::of::<T>() == self.state_type {
            Some(WindowHandle {
                any_handle: *self,
                state_type: PhantomData,
            })
        } else {
            None
        }
    }

    /// Updates the state of the root view of this window.
    ///
    /// This will fail if the window has been closed.
    pub fn update<C, R>(
        self,
        cx: &mut C,
        update: impl FnOnce(AnyView, &mut Window, &mut App) -> R,
    ) -> Result<R>
    where
        C: AppContext,
    {
        cx.update_window(self, update)
    }

    /// Read the state of the root view of this window.
    ///
    /// This will fail if the window has been closed.
    pub fn read<T, C, R>(self, cx: &C, read: impl FnOnce(Entity<T>, &App) -> R) -> Result<R>
    where
        C: AppContext,
        T: 'static,
    {
        let view = self
            .downcast::<T>()
            .context("the type of the window's root view has changed")?;

        cx.read_window(&view, read)
    }
}

impl HasWindowHandle for Window {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, HandleError> {
        self.platform_window.window_handle()
    }
}

impl HasDisplayHandle for Window {
    fn display_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::DisplayHandle<'_>, HandleError> {
        self.platform_window.display_handle()
    }
}

/// An identifier for an [`Element`].
///
/// Can be constructed with a string, a number, or both, as well
/// as other internal representations.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ElementId {
    /// The ID of a View element
    View(EntityId),
    /// An integer ID.
    Integer(u64),
    /// A string based ID.
    Name(SharedString),
    /// A UUID.
    Uuid(Uuid),
    /// An ID that's equated with a focus handle.
    FocusHandle(FocusId),
    /// A combination of a name and an integer.
    NamedInteger(SharedString, u64),
    /// A path.
    Path(Arc<std::path::Path>),
    /// A code location.
    CodeLocation(core::panic::Location<'static>),
    /// A labeled child of an element.
    NamedChild(Arc<ElementId>, SharedString),
    /// A byte array ID (used for text-anchors)
    OpaqueId([u8; 20]),
}

impl ElementId {
    /// Constructs an `ElementId::NamedInteger` from a name and `usize`.
    pub fn named_usize(name: impl Into<SharedString>, integer: usize) -> ElementId {
        Self::NamedInteger(name.into(), integer as u64)
    }
}

impl Display for ElementId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElementId::View(entity_id) => write!(f, "view-{}", entity_id)?,
            ElementId::Integer(ix) => write!(f, "{}", ix)?,
            ElementId::Name(name) => write!(f, "{}", name)?,
            ElementId::FocusHandle(_) => write!(f, "FocusHandle")?,
            ElementId::NamedInteger(s, i) => write!(f, "{}-{}", s, i)?,
            ElementId::Uuid(uuid) => write!(f, "{}", uuid)?,
            ElementId::Path(path) => write!(f, "{}", path.display())?,
            ElementId::CodeLocation(location) => write!(f, "{}", location)?,
            ElementId::NamedChild(id, name) => write!(f, "{}-{}", id, name)?,
            ElementId::OpaqueId(opaque_id) => write!(f, "{:x?}", opaque_id)?,
        }

        Ok(())
    }
}

impl TryInto<SharedString> for ElementId {
    type Error = anyhow::Error;

    fn try_into(self) -> anyhow::Result<SharedString> {
        if let ElementId::Name(name) = self {
            Ok(name)
        } else {
            anyhow::bail!("element id is not string")
        }
    }
}

impl From<usize> for ElementId {
    fn from(id: usize) -> Self {
        ElementId::Integer(id as u64)
    }
}

impl From<i32> for ElementId {
    fn from(id: i32) -> Self {
        Self::Integer(id as u64)
    }
}

impl From<SharedString> for ElementId {
    fn from(name: SharedString) -> Self {
        ElementId::Name(name)
    }
}

impl From<String> for ElementId {
    fn from(name: String) -> Self {
        ElementId::Name(name.into())
    }
}

impl From<Arc<str>> for ElementId {
    fn from(name: Arc<str>) -> Self {
        ElementId::Name(name.into())
    }
}

impl From<Arc<std::path::Path>> for ElementId {
    fn from(path: Arc<std::path::Path>) -> Self {
        ElementId::Path(path)
    }
}

impl From<&'static str> for ElementId {
    fn from(name: &'static str) -> Self {
        ElementId::Name(SharedString::new_static(name))
    }
}

impl<'a> From<&'a FocusHandle> for ElementId {
    fn from(handle: &'a FocusHandle) -> Self {
        ElementId::FocusHandle(handle.id)
    }
}

impl From<(&'static str, EntityId)> for ElementId {
    fn from((name, id): (&'static str, EntityId)) -> Self {
        ElementId::NamedInteger(SharedString::new_static(name), id.as_u64())
    }
}

impl From<(&'static str, usize)> for ElementId {
    fn from((name, id): (&'static str, usize)) -> Self {
        ElementId::NamedInteger(SharedString::new_static(name), id as u64)
    }
}

impl From<(SharedString, usize)> for ElementId {
    fn from((name, id): (SharedString, usize)) -> Self {
        ElementId::NamedInteger(name, id as u64)
    }
}

impl From<(&'static str, u64)> for ElementId {
    fn from((name, id): (&'static str, u64)) -> Self {
        ElementId::NamedInteger(SharedString::new_static(name), id)
    }
}

impl From<Uuid> for ElementId {
    fn from(value: Uuid) -> Self {
        Self::Uuid(value)
    }
}

impl From<(&'static str, u32)> for ElementId {
    fn from((name, id): (&'static str, u32)) -> Self {
        ElementId::NamedInteger(SharedString::new_static(name), u64::from(id))
    }
}

impl<T: Into<SharedString>> From<(ElementId, T)> for ElementId {
    fn from((id, name): (ElementId, T)) -> Self {
        ElementId::NamedChild(Arc::new(id), name.into())
    }
}

impl From<&'static core::panic::Location<'static>> for ElementId {
    fn from(location: &'static core::panic::Location<'static>) -> Self {
        ElementId::CodeLocation(*location)
    }
}

impl From<[u8; 20]> for ElementId {
    fn from(opaque_id: [u8; 20]) -> Self {
        ElementId::OpaqueId(opaque_id)
    }
}

/// A rectangle to be rendered in the window at the given position and size.
/// Passed as an argument [`Window::paint_quad`].
#[derive(Clone)]
pub struct PaintQuad {
    /// The bounds of the quad within the window.
    pub bounds: Bounds<Pixels>,
    /// The radii of the quad's corners.
    pub corner_radii: Corners<Pixels>,
    /// The background color of the quad.
    pub background: Background,
    /// The widths of the quad's borders.
    pub border_widths: Edges<Pixels>,
    /// The color of the quad's borders.
    pub border_color: Hsla,
    /// The style of the quad's borders.
    pub border_style: BorderStyle,
}

impl PaintQuad {
    /// Sets the corner radii of the quad.
    pub fn corner_radii(self, corner_radii: impl Into<Corners<Pixels>>) -> Self {
        PaintQuad {
            corner_radii: corner_radii.into(),
            ..self
        }
    }

    /// Sets the border widths of the quad.
    pub fn border_widths(self, border_widths: impl Into<Edges<Pixels>>) -> Self {
        PaintQuad {
            border_widths: border_widths.into(),
            ..self
        }
    }

    /// Sets the border color of the quad.
    pub fn border_color(self, border_color: impl Into<Hsla>) -> Self {
        PaintQuad {
            border_color: border_color.into(),
            ..self
        }
    }

    /// Sets the background color of the quad.
    pub fn background(self, background: impl Into<Background>) -> Self {
        PaintQuad {
            background: background.into(),
            ..self
        }
    }
}

/// Creates a quad with the given parameters.
pub fn quad(
    bounds: Bounds<Pixels>,
    corner_radii: impl Into<Corners<Pixels>>,
    background: impl Into<Background>,
    border_widths: impl Into<Edges<Pixels>>,
    border_color: impl Into<Hsla>,
    border_style: BorderStyle,
) -> PaintQuad {
    PaintQuad {
        bounds,
        corner_radii: corner_radii.into(),
        background: background.into(),
        border_widths: border_widths.into(),
        border_color: border_color.into(),
        border_style,
    }
}

/// Creates a filled quad with the given bounds and background color.
pub fn fill(bounds: impl Into<Bounds<Pixels>>, background: impl Into<Background>) -> PaintQuad {
    PaintQuad {
        bounds: bounds.into(),
        corner_radii: (0.).into(),
        background: background.into(),
        border_widths: (0.).into(),
        border_color: transparent_black(),
        border_style: BorderStyle::default(),
    }
}

/// Creates a rectangle outline with the given bounds, border color, and a 1px border width
pub fn outline(
    bounds: impl Into<Bounds<Pixels>>,
    border_color: impl Into<Hsla>,
    border_style: BorderStyle,
) -> PaintQuad {
    PaintQuad {
        bounds: bounds.into(),
        corner_radii: (0.).into(),
        background: transparent_black().into(),
        border_widths: (1.).into(),
        border_color: border_color.into(),
        border_style,
    }
}

/// Dropped images a window that draws no frame holds before it frees them anyway (Riseonly 0128).
const MAX_PENDING_IMAGE_DROPS: usize = 4096;

/// Segments each rounded corner of a transformed quad is tessellated into.
const TRANSFORMED_CORNER_SEGMENTS: usize = 8;

/// Maps a point in window pixels through a transformation given in scaled pixels.
fn transform_point(
    transformation: &TransformationMatrix,
    point: Point<Pixels>,
    scale_factor: f32,
) -> Point<Pixels> {
    let scaled = transformation.apply(Point::new(
        Pixels(point.x.0 * scale_factor),
        Pixels(point.y.0 * scale_factor),
    ));
    Point::new(
        Pixels(scaled.x.0 / scale_factor),
        Pixels(scaled.y.0 / scale_factor),
    )
}

fn transform_path(path: &mut Path<Pixels>, transformation: &TransformationMatrix, scale_factor: f32) {
    let mut bounds: Option<Bounds<Pixels>> = None;
    for vertex in &mut path.vertices {
        vertex.xy_position = transform_point(transformation, vertex.xy_position, scale_factor);
        let corner = Bounds {
            origin: vertex.xy_position,
            size: Size::default(),
        };
        bounds = Some(match bounds {
            Some(bounds) => bounds.union(&corner),
            None => corner,
        });
    }
    if let Some(bounds) = bounds {
        path.bounds = bounds;
    }
}

/// The outline of a rounded rectangle, clockwise from the top left corner, with the same number
/// of points for any radii so two outlines can be stitched into a ring.
fn rounded_rect_outline(bounds: Bounds<Pixels>, radii: Corners<Pixels>) -> Vec<Point<Pixels>> {
    let corners = [
        (
            radii.top_left,
            point(bounds.left() + radii.top_left, bounds.top() + radii.top_left),
            std::f32::consts::PI,
        ),
        (
            radii.top_right,
            point(bounds.right() - radii.top_right, bounds.top() + radii.top_right),
            std::f32::consts::PI * 1.5,
        ),
        (
            radii.bottom_right,
            point(
                bounds.right() - radii.bottom_right,
                bounds.bottom() - radii.bottom_right,
            ),
            0.,
        ),
        (
            radii.bottom_left,
            point(bounds.left() + radii.bottom_left, bounds.bottom() - radii.bottom_left),
            std::f32::consts::FRAC_PI_2,
        ),
    ];
    let mut outline = Vec::with_capacity(corners.len() * (TRANSFORMED_CORNER_SEGMENTS + 1));
    for (radius, center, start) in corners {
        for step in 0..=TRANSFORMED_CORNER_SEGMENTS {
            let angle = start
                + std::f32::consts::FRAC_PI_2 * step as f32 / TRANSFORMED_CORNER_SEGMENTS as f32;
            outline.push(point(
                center.x + Pixels(radius.0 * angle.cos()),
                center.y + Pixels(radius.0 * angle.sin()),
            ));
        }
    }
    outline
}

/// A linear gradient turns with the content it fills; the other backgrounds have no direction.
fn rotated_background(background: Background, transformation: &TransformationMatrix) -> Background {
    if background.tag != crate::color::BackgroundTag::LinearGradient {
        return background;
    }
    let degrees = transformation.rotation_scale[1][0]
        .atan2(transformation.rotation_scale[0][0])
        .to_degrees();
    let mut rotated = background;
    rotated.gradient_angle_or_pattern_height += degrees;
    rotated
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        path::PathBuf,
        rc::Rc,
    };

    use crate::{
        AnyWindowHandle, AppContext as _, Bounds, Context, DragMoveEvent, Empty,
        ExternalDragPayload, ExternalPaths, FileDragPaths, FileDropEvent, FocusHandle,
        InputEvent as _, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
        MouseMoveEvent, ParentElement, Pixels, Point, Render, StatefulInteractiveElement as _,
        Styled, TestAppContext, Window, WindowAppearance, WindowOptions, canvas, div, point, px,
        size,
    };

    struct EmptyView;

    impl Render for EmptyView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    struct OpensWindowOnPaint {
        opened: Rc<Cell<bool>>,
    }

    impl Render for OpensWindowOnPaint {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let opened = self.opened.clone();
            div()
                .size_full()
                .child(canvas(
                    |_, _, _| {},
                    move |_, _, _window, cx| {
                        if !opened.replace(true) {
                            cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| EmptyView))
                                .unwrap();
                        }
                    },
                ))
                // Siblings painted after the canvas: their elements were
                // allocated in the arena before the nested draw, so they detect
                // a mid-draw arena clear when painted afterwards.
                .child(div().child("after"))
        }
    }

    /// Opening a window synchronously draws it and requests an element arena
    /// clear. When that happens from within another window's draw (here: from
    /// an element's paint), the clear must be deferred until the outer draw
    /// finishes, or the outer draw's arena-allocated elements would be freed
    /// out from under it.
    #[test]
    fn test_window_opened_during_draw_defers_arena_clear() {
        let mut cx = TestAppContext::single();

        let opened = Rc::new(Cell::new(false));
        // add_window draws once, which runs the nested open_window mid-draw.
        let window = cx.add_window({
            let opened = opened.clone();
            move |_, _| OpensWindowOnPaint { opened }
        });

        assert!(opened.get());
        assert_eq!(cx.windows().len(), 2);

        // The deferred clear must actually run once the outer draw unwinds:
        // subsequent draws of both windows work against a fresh arena.
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
    }

    #[gpui::test]
    fn test_appearance_change_runs_after_app_update(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, _| EmptyView);
        let observed_appearance = Rc::new(Cell::new(None));
        let _subscription = window
            .update(cx, {
                let observed_appearance = observed_appearance.clone();
                move |_, window, _| {
                    window.observe_window_appearance(move |window, _| {
                        observed_appearance.set(Some(window.appearance()));
                    })
                }
            })
            .unwrap();
        let test_window = cx.test_window(window.into());

        cx.update(|_| {
            test_window.simulate_appearance_change(WindowAppearance::Dark);
            assert_eq!(observed_appearance.get(), None);
        });
        cx.run_until_parked();

        assert_eq!(observed_appearance.get(), Some(WindowAppearance::Dark));
    }

    struct RootView {
        explicit_size: bool,
        child_bounds: Rc<Cell<Bounds<Pixels>>>,
    }

    impl Render for RootView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let child_bounds = self.child_bounds.clone();
            let root = div().flex().flex_col().child(
                canvas(
                    move |bounds, _, _| child_bounds.set(bounds),
                    |_, _, _, _| {},
                )
                .size_full(),
            );
            if self.explicit_size {
                root.w(px(300.)).h(px(200.))
            } else {
                root
            }
        }
    }

    #[test]
    fn auto_sized_window_root_fills_the_window() {
        let mut cx = TestAppContext::single();
        let child_bounds = Rc::new(Cell::new(Bounds::default()));
        let window = cx.add_window({
            let child_bounds = child_bounds.clone();
            move |_, _| RootView {
                explicit_size: false,
                child_bounds,
            }
        });

        let viewport_size = cx
            .update_window(window.into(), |_, window, cx| {
                window.draw(cx).clear(cx);
                window.viewport_size()
            })
            .unwrap();

        assert_eq!(child_bounds.get().size, viewport_size);
    }

    #[test]
    fn explicitly_sized_window_root_keeps_its_size() {
        let mut cx = TestAppContext::single();
        let child_bounds = Rc::new(Cell::new(Bounds::default()));
        let window = cx.add_window({
            let child_bounds = child_bounds.clone();
            move |_, _| RootView {
                explicit_size: true,
                child_bounds,
            }
        });

        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();

        assert_eq!(child_bounds.get().size, size(px(300.), px(200.)));
    }

    struct FileDragView {
        path: PathBuf,
        observed_drag_moves: Rc<RefCell<Vec<Point<Pixels>>>>,
        observed_drops: Rc<RefCell<Vec<PathBuf>>>,
    }

    impl Render for FileDragView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("file-drag")
                .size_full()
                .on_drag(self.path.clone(), |_, _, _, cx| cx.new(|_| Empty))
                .external_drag_payload(|path: &PathBuf, _, _| {
                    Some(ExternalDragPayload::Files(FileDragPaths::new([(
                        path.clone(),
                        true,
                    )])))
                })
                .on_drag_move({
                    let observed_drag_moves = self.observed_drag_moves.clone();
                    move |event: &DragMoveEvent<PathBuf>, _, _| {
                        observed_drag_moves.borrow_mut().push(event.event.position);
                    }
                })
                .on_drop({
                    let observed_drops = self.observed_drops.clone();
                    move |path: &PathBuf, _, _| observed_drops.borrow_mut().push(path.clone())
                })
        }
    }

    #[gpui::test]
    fn file_drag_is_promoted_once_and_restored_in_source_window(cx: &mut TestAppContext) {
        struct Drag {
            window: AnyWindowHandle,
            observed_drag_moves: Rc<RefCell<Vec<Point<Pixels>>>>,
            observed_drops: Rc<RefCell<Vec<PathBuf>>>,
        }

        fn start_drag(cx: &mut TestAppContext, path: PathBuf, platform_result: bool) -> Drag {
            let observed_drag_moves = Rc::new(RefCell::new(Vec::new()));
            let observed_drops = Rc::new(RefCell::new(Vec::new()));
            let window: AnyWindowHandle = cx
                .add_window({
                    let observed_drag_moves = observed_drag_moves.clone();
                    let observed_drops = observed_drops.clone();
                    move |_, _| FileDragView {
                        path,
                        observed_drag_moves,
                        observed_drops,
                    }
                })
                .into();
            cx.test_window(window)
                .set_start_external_drag_result(platform_result);

            let update_result = cx.update_window(window, |_, window, cx| {
                window.draw(cx).clear(cx);
                window.dispatch_event(
                    MouseDownEvent {
                        position: point(px(10.), px(10.)),
                        button: MouseButton::Left,
                        modifiers: Default::default(),
                        click_count: 1,
                        first_mouse: false,
                    }
                    .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    MouseMoveEvent {
                        position: point(px(20.), px(20.)),
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                assert!(cx.active_drag.is_some());
            });
            assert!(
                update_result.is_ok(),
                "failed to start drag: {update_result:?}"
            );

            assert!(cx.test_window(window).external_drag_files().is_empty());
            Drag {
                window,
                observed_drag_moves,
                observed_drops,
            }
        }

        let successful_path = PathBuf::from("/tmp/successful-drag");
        let successful = start_drag(cx, successful_path.clone(), true);
        let outside_position = point(px(-1.), px(20.));
        let update_result = cx.update_window(successful.window, |_, window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position: outside_position,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            assert!(cx.active_drag.is_none());
        });
        assert!(
            update_result.is_ok(),
            "failed to promote drag: {update_result:?}"
        );
        assert_eq!(
            cx.test_window(successful.window).external_drag_files(),
            [(successful_path.clone(), true)]
        );
        // Views must still see the move that leaves the window, otherwise they never learn to tear
        // down the drag state they built up while the pointer was inside.
        assert_eq!(
            successful.observed_drag_moves.borrow().last(),
            Some(&outside_position)
        );

        let destination: AnyWindowHandle = cx.add_window(|_, _| EmptyView).into();
        let reentry_position = point(px(30.), px(30.));
        let external_paths = || ExternalPaths([successful_path.clone()].into_iter().collect());
        let update_result = cx.update_window(destination, |_, window, cx| {
            window.dispatch_event(
                FileDropEvent::Entered {
                    position: reentry_position,
                    paths: external_paths(),
                }
                .to_platform_input(),
                cx,
            );
            assert!(
                cx.active_drag
                    .as_ref()
                    .is_some_and(|drag| drag.value.downcast_ref::<ExternalPaths>().is_some())
            );
            window.dispatch_event(FileDropEvent::Exited.to_platform_input(), cx);
            assert!(cx.active_drag.is_none());
        });
        assert!(
            update_result.is_ok(),
            "failed to handle drag in destination window: {update_result:?}"
        );

        let update_result = cx.update_window(successful.window, |_, window, cx| {
            window.dispatch_event(
                FileDropEvent::Entered {
                    position: reentry_position,
                    paths: external_paths(),
                }
                .to_platform_input(),
                cx,
            );
            assert!(
                cx.active_drag
                    .as_ref()
                    .is_some_and(|drag| drag.value.downcast_ref::<PathBuf>().is_some())
            );
            assert_eq!(
                successful.observed_drag_moves.borrow().last(),
                Some(&reentry_position)
            );

            window.dispatch_event(FileDropEvent::Exited.to_platform_input(), cx);
            assert!(cx.active_drag.is_none());

            window.dispatch_event(
                FileDropEvent::Entered {
                    position: reentry_position,
                    paths: external_paths(),
                }
                .to_platform_input(),
                cx,
            );
            assert!(
                cx.active_drag
                    .as_ref()
                    .is_some_and(|drag| drag.value.downcast_ref::<PathBuf>().is_some())
            );

            window.dispatch_event(
                FileDropEvent::Submit {
                    position: reentry_position,
                }
                .to_platform_input(),
                cx,
            );
            assert_eq!(
                successful.observed_drops.borrow().as_slice(),
                std::slice::from_ref(&successful_path)
            );
            assert!(cx.active_drag.is_none());

            window.dispatch_event(FileDropEvent::Exited.to_platform_input(), cx);
            assert!(cx.active_drag.is_none());
            window.dispatch_event(FileDropEvent::Ended.to_platform_input(), cx);
            assert!(cx.active_drag.is_none());

            window.dispatch_event(
                FileDropEvent::Entered {
                    position: reentry_position,
                    paths: external_paths(),
                }
                .to_platform_input(),
                cx,
            );
            assert!(
                cx.active_drag
                    .as_ref()
                    .is_some_and(|drag| drag.value.downcast_ref::<ExternalPaths>().is_some())
            );
            window.dispatch_event(FileDropEvent::Exited.to_platform_input(), cx);
        });
        assert!(
            update_result.is_ok(),
            "failed to restore drag in source window: {update_result:?}"
        );

        let cancelled_path = PathBuf::from("/tmp/cancelled-drag");
        let cancelled = start_drag(cx, cancelled_path.clone(), true);
        let update_result = cx.update_window(cancelled.window, |_, window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position: outside_position,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            assert!(cx.active_drag.is_none());

            window.dispatch_event(
                FileDropEvent::Entered {
                    position: reentry_position,
                    paths: ExternalPaths([cancelled_path].into_iter().collect()),
                }
                .to_platform_input(),
                cx,
            );
            assert!(
                cx.active_drag
                    .as_ref()
                    .is_some_and(|drag| drag.value.downcast_ref::<PathBuf>().is_some())
            );
            assert!(cx.stop_active_drag(window));
            assert!(cx.active_drag.is_none());
        });
        assert!(
            update_result.is_ok(),
            "failed to cancel restored drag: {update_result:?}"
        );
        assert!(!cx.update(|cx| cx.end_platform_drag(cancelled.window.window_id())));

        let removed_path = PathBuf::from("/tmp/removed-window-drag");
        let removed = start_drag(cx, removed_path, true);
        let removed_window_id = removed.window.window_id();
        let update_result = cx.update_window(removed.window, |_, window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position: outside_position,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            assert!(cx.active_drag.is_none());
            window.remove_window();
        });
        assert!(
            update_result.is_ok(),
            "failed to remove drag source window: {update_result:?}"
        );
        assert!(!cx.update(|cx| cx.end_platform_drag(removed_window_id)));

        let failed_path = PathBuf::from("/tmp/failed-drag");
        let failed = start_drag(cx, failed_path.clone(), false);
        let update_result = cx.update_window(failed.window, |_, window, cx| {
            for x_position in [-1., -2.] {
                window.dispatch_event(
                    MouseMoveEvent {
                        position: point(px(x_position), px(20.)),
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
            }
            assert!(cx.active_drag.is_some());
        });
        assert!(
            update_result.is_ok(),
            "failed to retain drag after platform failure: {update_result:?}"
        );
        assert_eq!(
            cx.test_window(failed.window).external_drag_files(),
            [(failed_path, true)]
        );
    }

    struct FocusForwarder {
        a: FocusHandle,
        b: FocusHandle,
    }

    impl Render for FocusForwarder {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(div().w(px(50.)).h(px(50.)).track_focus(&self.a))
                .child(div().w(px(50.)).h(px(50.)).track_focus(&self.b))
        }
    }

    /// When a focus listener moves focus again (e.g. a dock forwarding focus to its
    /// active panel), the resulting focus events must be dispatched without waiting
    /// for an unrelated redraw of the window.
    #[gpui::test]
    fn test_focus_moved_by_focus_listener_is_dispatched(cx: &mut TestAppContext) {
        let b_focus_count = Rc::new(Cell::new(0));
        let window = cx.add_window({
            let b_focus_count = b_focus_count.clone();
            move |window, cx| {
                let a = cx.focus_handle();
                let b = cx.focus_handle();
                cx.on_focus(&a, window, |this: &mut FocusForwarder, window, cx| {
                    let b = this.b.clone();
                    window.focus(&b, cx);
                })
                .detach();
                cx.on_focus(&b, window, move |_, _, _| {
                    b_focus_count.set(b_focus_count.get() + 1);
                })
                .detach();
                FocusForwarder { a, b }
            }
        });

        window
            .update(cx, |_, window, _| window.activate_window())
            .unwrap();
        cx.executor().run_until_parked();

        window
            .update(cx, |this, window, cx| {
                let a = this.a.clone();
                window.focus(&a, cx);
            })
            .unwrap();
        cx.executor().run_until_parked();

        window
            .update(cx, |this, window, _| {
                assert!(this.b.is_focused(window));
            })
            .unwrap();
        assert_eq!(b_focus_count.get(), 1);
    }
}

fn unzoom_point(position: Point<Pixels>, zoom: f32) -> Point<Pixels> {
    point(position.x / zoom, position.y / zoom)
}

fn unzoom_size(content: Size<Pixels>, zoom: f32) -> Size<Pixels> {
    size(content.width / zoom, content.height / zoom)
}

/// Riseonly 0124: platform positions and pixel scroll deltas in this window's logical pixels.
fn unzoom_input(event: PlatformInput, zoom: f32) -> PlatformInput {
    let at = |position: Point<Pixels>| unzoom_point(position, zoom);
    match event {
        PlatformInput::MouseDown(event) => PlatformInput::MouseDown(crate::MouseDownEvent {
            position: at(event.position),
            ..event
        }),
        PlatformInput::MouseUp(event) => PlatformInput::MouseUp(crate::MouseUpEvent {
            position: at(event.position),
            ..event
        }),
        PlatformInput::MousePressure(event) => PlatformInput::MousePressure(crate::MousePressureEvent {
            position: at(event.position),
            ..event
        }),
        PlatformInput::MouseMove(event) => PlatformInput::MouseMove(crate::MouseMoveEvent {
            position: at(event.position),
            ..event
        }),
        PlatformInput::MouseExited(event) => PlatformInput::MouseExited(crate::MouseExitEvent {
            position: at(event.position),
            ..event
        }),
        PlatformInput::ScrollWheel(event) => PlatformInput::ScrollWheel(crate::ScrollWheelEvent {
            position: at(event.position),
            delta: match event.delta {
                crate::ScrollDelta::Pixels(delta) => crate::ScrollDelta::Pixels(at(delta)),
                lines => lines,
            },
            ..event
        }),
        PlatformInput::Pinch(event) => PlatformInput::Pinch(crate::PinchEvent {
            position: at(event.position),
            ..event
        }),
        PlatformInput::Touch(event) => PlatformInput::Touch(crate::TouchEvent {
            position: at(event.position),
            ..event
        }),
        PlatformInput::FileDrop(drop) => PlatformInput::FileDrop(match drop {
            FileDropEvent::Entered { position, paths } => FileDropEvent::Entered {
                position: at(position),
                paths,
            },
            FileDropEvent::Pending { position } => FileDropEvent::Pending {
                position: at(position),
            },
            FileDropEvent::Submit { position } => FileDropEvent::Submit {
                position: at(position),
            },
            other => other,
        }),
        other => other,
    }
}
