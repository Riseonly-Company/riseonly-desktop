use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{
    AnyElement, AnyWindowHandle, App, Bounds, ElementId, HitboxBehavior, HitboxId, IntoElement, ParentElement,
    Pixels, Point, RenderOnce, Styled, Window, canvas, div, point,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::swiftui::environment::{EnvKey, env, env_scope};
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenCoordinates};

/// action a registered target runs on tap
pub type BubbleTapAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct BubbleTapRowHostView {
    id: ElementId,
    child: AnyElement,
}

impl BubbleTapRowHostView {
    pub fn new(id: impl Into<ElementId>, child: impl IntoElement) -> Self {
        Self { id: id.into(), child: child.into_any_element() }
    }
}

impl RenderOnce for BubbleTapRowHostView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        env_scope::<BubbleTapRowHostKey>(Some(self.id)).child(self.child)
    }
}

pub struct BubbleTapRowHostKey;

impl EnvKey for BubbleTapRowHostKey {
    type Value = Option<ElementId>;

    fn default_value() -> Option<ElementId> {
        None
    }
}

#[derive(Default)]
struct BubbleTapViewFrame {
    bounds: Option<Bounds<Pixels>>,
    window: Option<AnyWindowHandle>,
    hitbox: Option<HitboxId>,
    row: Option<ElementId>,
}

/// desktop stand in for the platform view a target binds to
/// holds last frame bounds, window and row. targets keep it weakly
#[derive(Clone, Default)]
pub struct BubbleTapView(Rc<RefCell<BubbleTapViewFrame>>);

impl BubbleTapView {
    pub fn new() -> Self {
        Self::default()
    }

    /// called in prepaint, stores the frame and puts a pass through hitbox on it
    /// so later sheets and menus count as covering it
    pub fn report_frame(&self, bounds: Bounds<Pixels>, row: Option<ElementId>, window: &mut Window) {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        {
            let mut frame = self.0.borrow_mut();
            frame.bounds = Some(bounds);
            frame.window = Some(window.window_handle());
            frame.hitbox = Some(hitbox.id);
            frame.row = row;
        }
        let moved = self.clone();
        window.on_replayed_move(move |offset, _, _, _| moved.shift_frame(bounds, offset));
    }

    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.0.borrow().bounds
    }

    pub fn shift_frame(&self, prepainted: Bounds<Pixels>, offset: Point<Pixels>) {
        self.0.borrow_mut().bounds = Some(Bounds { origin: prepainted.origin + offset, size: prepainted.size });
    }

    fn downgrade(&self) -> Weak<RefCell<BubbleTapViewFrame>> {
        Rc::downgrade(&self.0)
    }

    fn upgrade(weak: &Weak<RefCell<BubbleTapViewFrame>>) -> Option<Self> {
        weak.upgrade().map(Self)
    }
}

#[derive(Clone)]
pub struct BubbleTapRouterTarget {
    pub id: Uuid,
    pub frame: Bounds<Pixels>,
    pub on_tap: BubbleTapAction,
    coordinate_view: Option<Weak<RefCell<BubbleTapViewFrame>>>,
    pub uses_local_coordinates: bool,
    pub tracks_view_bounds: bool,
}

#[derive(Clone)]
pub struct BubbleTapRouterMediaTarget {
    pub id: Uuid,
    pub registry_id: String,
    pub on_tap: BubbleTapAction,
    view: Option<Weak<RefCell<BubbleTapViewFrame>>>,
}

#[derive(Default)]
pub struct BubbleTapRouter {
    targets: HashMap<Uuid, BubbleTapRouterTarget>,
    target_order: Vec<Uuid>,
    media_targets: HashMap<Uuid, BubbleTapRouterMediaTarget>,
    media_target_order: Vec<Uuid>,
    priority_targets: HashMap<Uuid, BubbleTapRouterTarget>,
    priority_target_order: Vec<Uuid>,
    ambient_source: Option<Weak<RefCell<BubbleTapViewFrame>>>,
}

struct BubbleTapAmbientSourceRestore(Option<Option<Weak<RefCell<BubbleTapViewFrame>>>>);

impl Drop for BubbleTapAmbientSourceRestore {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            BubbleTapRouter::shared_if_alive(|router| router.ambient_source = previous);
        }
    }
}

thread_local! {
    static SHARED: RefCell<BubbleTapRouter> = RefCell::new(BubbleTapRouter::default());
}

impl BubbleTapRouter {
    /// runs f on the one router of the main thread
    pub fn shared<R>(f: impl FnOnce(&mut BubbleTapRouter) -> R) -> R {
        SHARED.with(|router| f(&mut router.borrow_mut()))
    }

    /// for element state drops, at exit or during a dispatch there is no router to unregister from
    pub fn shared_if_alive(f: impl FnOnce(&mut BubbleTapRouter)) {
        let _ = SHARED.try_with(|router| {
            if let Ok(mut router) = router.try_borrow_mut() {
                f(&mut router);
            }
        });
    }

    pub fn register(&mut self, frame: Bounds<Pixels>, on_tap: BubbleTapAction) -> Uuid {
        let id = Uuid::new_v4();
        self.targets.insert(
            id,
            BubbleTapRouterTarget {
                id,
                frame,
                on_tap,
                coordinate_view: None,
                uses_local_coordinates: false,
                tracks_view_bounds: false,
            },
        );
        self.target_order.push(id);
        id
    }

    pub fn register_priority(&mut self, frame: Bounds<Pixels>, on_tap: BubbleTapAction) -> Uuid {
        let id = Uuid::new_v4();
        self.priority_targets.insert(
            id,
            BubbleTapRouterTarget {
                id,
                frame,
                on_tap,
                coordinate_view: None,
                uses_local_coordinates: false,
                tracks_view_bounds: false,
            },
        );
        self.priority_target_order.push(id);
        id
    }

    pub fn register_local(
        &mut self,
        view: &BubbleTapView,
        frame: Bounds<Pixels>,
        priority: bool,
        on_tap: BubbleTapAction,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let target = BubbleTapRouterTarget {
            id,
            frame,
            on_tap,
            coordinate_view: Some(view.downgrade()),
            uses_local_coordinates: true,
            tracks_view_bounds: false,
        };
        self.store(target, priority);
        id
    }

    pub fn register_view(&mut self, view: &BubbleTapView, priority: bool, on_tap: BubbleTapAction) -> Uuid {
        let id = Uuid::new_v4();
        let target = BubbleTapRouterTarget {
            id,
            frame: Bounds::default(),
            on_tap,
            coordinate_view: Some(view.downgrade()),
            uses_local_coordinates: true,
            tracks_view_bounds: true,
        };
        self.store(target, priority);
        id
    }

    fn store(&mut self, target: BubbleTapRouterTarget, priority: bool) {
        let id = target.id;
        if priority {
            self.priority_targets.insert(id, target);
            self.priority_target_order.push(id);
        } else {
            self.targets.insert(id, target);
            self.target_order.push(id);
        }
    }

    pub fn register_media(&mut self, registry_id: String, view: Option<&BubbleTapView>, on_tap: BubbleTapAction) -> Uuid {
        let id = Uuid::new_v4();
        self.media_targets.insert(
            id,
            BubbleTapRouterMediaTarget { id, registry_id, on_tap, view: view.map(BubbleTapView::downgrade) },
        );
        self.media_target_order.push(id);
        id
    }

    pub fn update(&mut self, id: Uuid, frame: Bounds<Pixels>) {
        if let Some(target) = self.targets.get_mut(&id) {
            target.frame = frame;
        }
        if let Some(target) = self.priority_targets.get_mut(&id) {
            target.frame = frame;
        }
    }

    pub fn unregister(&mut self, id: Uuid) {
        if self.targets.remove(&id).is_some() {
            self.target_order.retain(|entry| *entry != id);
        }
        if self.priority_targets.remove(&id).is_some() {
            self.priority_target_order.retain(|entry| *entry != id);
        }
        if self.media_targets.remove(&id).is_some() {
            self.media_target_order.retain(|entry| *entry != id);
        }
    }

    pub fn unregister_media(&mut self, id: Uuid) {
        if self.media_targets.remove(&id).is_none() {
            return;
        }
        self.media_target_order.retain(|entry| *entry != id);
    }

    pub fn with_tap_source(view: Option<&BubbleTapView>, body: impl FnOnce()) {
        let next = view.map(|view| Rc::downgrade(&view.0));
        let previous = Self::shared(|router| std::mem::replace(&mut router.ambient_source, next));
        let _restore = BubbleTapAmbientSourceRestore(Some(previous));
        body();
    }

    /// source is the view whose click took the tap, a target in another row never answers
    /// runs the first target hit and says if one was
    pub fn dispatch(at: Point<Pixels>, from: Option<&BubbleTapView>, window: &mut Window, cx: &mut App) -> bool {
        let ambient_source =
            Self::shared(|router| router.ambient_source.as_ref().and_then(Weak::upgrade)).map(BubbleTapView);
        let source = from.or(ambient_source.as_ref());
        let action = Self::shared(|router| router.target_at(at, source, window));
        let Some(action) = action else {
            return false;
        };
        action(window, cx);
        true
    }

    fn target_at(&self, window_point: Point<Pixels>, source: Option<&BubbleTapView>, window: &Window) -> Option<BubbleTapAction> {
        let point = WindowScreenCoordinates::screen_point(window_point);
        let source_row = source.and_then(|source| source.0.borrow().row.clone());
        let is_covered = |view: &BubbleTapView| -> bool {
            let frame = view.0.borrow();
            if frame.window != Some(window.window_handle()) {
                return false;
            }
            let hidden_under_another = frame.hitbox.is_some_and(|hitbox| !hitbox.is_hovered(window));
            let in_another_row = matches!((&frame.row, &source_row), (Some(own), Some(hit)) if own != hit);
            hidden_under_another || in_another_row
        };
        let hits = |target: &BubbleTapRouterTarget| -> bool {
            if !Self::hits(target, window_point, point, window) {
                return false;
            }
            let Some(view) = target.coordinate_view.as_ref().and_then(BubbleTapView::upgrade) else {
                return true;
            };
            !is_covered(&view)
        };

        for id in self.priority_target_order.iter().rev() {
            let Some(target) = self.priority_targets.get(id) else { continue };
            if hits(target) {
                Self::log_hit("priority", window_point, target.coordinate_view.as_ref(), None, window);
                return Some(target.on_tap.clone());
            }
        }

        for id in self.media_target_order.iter().rev() {
            let Some(target) = self.media_targets.get(id) else { continue };
            let Some(frame) = Self::media_frame(target, window) else { continue };
            if !frame.contains(&point) {
                continue;
            }
            if let Some(view) = target.view.as_ref().and_then(BubbleTapView::upgrade)
                && is_covered(&view)
            {
                continue;
            }
            Self::log_hit("media", window_point, target.view.as_ref(), Some(&target.registry_id), window);
            return Some(target.on_tap.clone());
        }

        for id in self.target_order.iter().rev() {
            let Some(target) = self.targets.get(id) else { continue };
            if hits(target) {
                Self::log_hit("plain", window_point, target.coordinate_view.as_ref(), None, window);
                return Some(target.on_tap.clone());
            }
        }
        tracing::info!(target: "riseonly::X1594", point = ?window_point, "routed tap missed");
        None
    }

    fn log_hit(
        lane: &str,
        window_point: Point<Pixels>,
        view: Option<&Weak<RefCell<BubbleTapViewFrame>>>,
        registry_id: Option<&str>,
        window: &Window,
    ) {
        let view = view.and_then(BubbleTapView::upgrade);
        let (bounds, hitbox, hovered, row) = match view.as_ref() {
            Some(view) => {
                let frame = view.0.borrow();
                (frame.bounds, frame.hitbox, frame.hitbox.map(|hitbox| hitbox.is_hovered(window)), frame.row.clone())
            }
            None => (None, None, None, None),
        };
        tracing::info!(
            target: "riseonly::X1594",
            lane,
            point = ?window_point,
            mouse = ?window.mouse_position(),
            frame = ?bounds,
            hitbox = ?hitbox,
            hovered = ?hovered,
            row = ?row,
            registry_id = registry_id.unwrap_or(""),
            "routed tap hit"
        );
    }

    fn media_frame(target: &BubbleTapRouterMediaTarget, window: &Window) -> Option<Bounds<Pixels>> {
        let Some(weak) = target.view.as_ref() else {
            return MediaCellScreenFrameRegistry::attached_frame(&target.registry_id);
        };
        let view = BubbleTapView::upgrade(weak)?;
        if !Self::is_presented_on_screen(&view, window) {
            return None;
        }
        let bounds = view.bounds()?;
        Some(Bounds { origin: WindowScreenCoordinates::screen_point(bounds.origin), size: bounds.size })
    }

    /// tap point is in the window that got the click
    /// view targets are compared in their own window so other windows never answer
    fn hits(target: &BubbleTapRouterTarget, window_point: Point<Pixels>, screen_point: Point<Pixels>, window: &Window) -> bool {
        if !target.uses_local_coordinates {
            return target.frame.contains(&screen_point);
        }
        let Some(view) = target.coordinate_view.as_ref().and_then(BubbleTapView::upgrade) else {
            return false;
        };
        if !Self::is_presented_on_screen(&view, window) {
            return false;
        }
        let Some(view_bounds) = view.bounds() else {
            return false;
        };
        let local_frame = if target.tracks_view_bounds {
            Bounds { origin: point(Pixels::ZERO, Pixels::ZERO), size: view_bounds.size }
        } else {
            target.frame
        };
        if local_frame.size.width <= Pixels::ZERO || local_frame.size.height <= Pixels::ZERO {
            return false;
        }
        let in_window = Bounds { origin: view_bounds.origin + local_frame.origin, size: local_frame.size };
        in_window.contains(&window_point)
    }

    /// a control in a row thats no longer drawn or in another window must not keep answering taps
    /// view leaves the router when its element state drops
    fn is_presented_on_screen(view: &BubbleTapView, window: &Window) -> bool {
        let frame = view.0.borrow();
        frame.bounds.is_some() && frame.window == Some(window.window_handle())
    }
}

/// content own frame answers routed taps
#[derive(IntoElement)]
pub struct BubbleTapTarget {
    id: ElementId,
    content: AnyElement,
    action: BubbleTapAction,
    priority: bool,
}

impl BubbleTapTarget {
    pub fn new(id: impl Into<ElementId>, content: impl IntoElement, action: BubbleTapAction, priority: bool) -> Self {
        Self { id: id.into(), content: content.into_any_element(), action, priority }
    }
}

impl RenderOnce for BubbleTapTarget {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div().relative().child(self.content).child(ScreenSpaceTapReporter {
            id: self.id,
            action: self.action,
            priority: self.priority,
        })
    }
}

#[derive(IntoElement)]
struct ScreenSpaceTapReporter {
    id: ElementId,
    action: BubbleTapAction,
    priority: bool,
}

impl RenderOnce for ScreenSpaceTapReporter {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let row = env::<BubbleTapRowHostKey>(cx);
        let reporter = window.use_keyed_state(self.id, cx, |_, _| ReporterView::new());
        let view = reporter.update(cx, |reporter, _| {
            *reporter.action.borrow_mut() = self.action.clone();
            reporter.priority = self.priority;
            reporter.sync_registration();
            reporter.view.clone()
        });
        canvas(move |bounds, window, _| view.report_frame(bounds, row, window), |_, _, _, _| {})
            .absolute()
            .inset_0()
    }
}

struct ReporterView {
    action: Rc<RefCell<BubbleTapAction>>,
    priority: bool,
    view: BubbleTapView,
    registration_id: Option<Uuid>,
    registered_priority: Option<bool>,
}

impl ReporterView {
    fn new() -> Self {
        Self {
            action: Rc::new(RefCell::new(Self::noop())),
            priority: false,
            view: BubbleTapView::new(),
            registration_id: None,
            registered_priority: None,
        }
    }

    fn noop() -> BubbleTapAction {
        Rc::new(|_: &mut Window, _: &mut App| {})
    }

    fn sync_registration(&mut self) {
        if self.registered_priority.is_some_and(|registered| registered != self.priority) {
            self.unregister();
        }
        if self.registration_id.is_some() {
            return;
        }
        let weak_action = Rc::downgrade(&self.action);
        let priority = self.priority;
        let view = self.view.clone();
        self.registration_id = Some(BubbleTapRouter::shared(|router| {
            router.register_view(
                &view,
                priority,
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let current = weak_action.upgrade().map(|action| action.borrow().clone());
                    if let Some(action) = current {
                        action(window, cx);
                    }
                }),
            )
        }));
        self.registered_priority = Some(priority);
    }

    fn unregister(&mut self) {
        self.registered_priority = None;
        if let Some(id) = self.registration_id.take() {
            BubbleTapRouter::shared_if_alive(|router| router.unregister(id));
        }
    }
}

impl Drop for ReporterView {
    fn drop(&mut self) {
        self.unregister();
    }
}

#[derive(IntoElement)]
pub struct MediaBubbleTapTarget {
    id: ElementId,
    registry_id: String,
    action: BubbleTapAction,
}

impl MediaBubbleTapTarget {
    pub fn new(id: impl Into<ElementId>, registry_id: String, action: BubbleTapAction) -> Self {
        Self { id: id.into(), registry_id, action }
    }
}

struct MediaBubbleTapTargetState {
    action: Rc<RefCell<BubbleTapAction>>,
    view: BubbleTapView,
    registration_id: Option<Uuid>,
    registered_registry_id: Option<String>,
}

impl MediaBubbleTapTargetState {
    fn new() -> Self {
        Self {
            action: Rc::new(RefCell::new(ReporterView::noop())),
            view: BubbleTapView::new(),
            registration_id: None,
            registered_registry_id: None,
        }
    }

    fn register(&mut self, registry_id: &str) {
        self.unregister();
        let weak_action = Rc::downgrade(&self.action);
        let view = self.view.clone();
        self.registration_id = Some(BubbleTapRouter::shared(|router| {
            router.register_media(
                registry_id.to_owned(),
                Some(&view),
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let current = weak_action.upgrade().map(|action| action.borrow().clone());
                    if let Some(action) = current {
                        action(window, cx);
                    }
                }),
            )
        }));
        self.registered_registry_id = Some(registry_id.to_owned());
    }

    fn unregister(&mut self) {
        let Some(id) = self.registration_id.take() else {
            return;
        };
        BubbleTapRouter::shared_if_alive(|router| router.unregister_media(id));
    }
}

impl Drop for MediaBubbleTapTargetState {
    fn drop(&mut self) {
        self.unregister();
    }
}

impl RenderOnce for MediaBubbleTapTarget {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let row = env::<BubbleTapRowHostKey>(cx);
        let state = window.use_keyed_state(self.id, cx, |_, _| MediaBubbleTapTargetState::new());
        let view = state.update(cx, |state, _| {
            *state.action.borrow_mut() = self.action.clone();
            if state.registered_registry_id.as_deref() != Some(self.registry_id.as_str()) {
                state.register(&self.registry_id);
            }
            state.view.clone()
        });
        canvas(
            move |bounds, window, _| {
                view.report_frame(bounds, row, window);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}

/// modifiers to make an element a bubble tap target, id keys the registration across frames
pub trait BubbleTapRouterModifiers: IntoElement + Sized {
    fn bubble_tap_target(self, id: impl Into<ElementId>, action: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
        BubbleTapTarget::new(id, self, Rc::new(action), false)
    }

    fn bubble_tap_target_priority(
        self,
        id: impl Into<ElementId>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        BubbleTapTarget::new(id, self, Rc::new(action), true)
    }

    fn media_bubble_tap_target(
        self,
        id: impl Into<ElementId>,
        registry_id: impl Into<String>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement
    where
        Self: ParentElement,
    {
        self.child(MediaBubbleTapTarget::new(id, registry_id.into(), Rc::new(action)))
    }
}

impl<E: IntoElement> BubbleTapRouterModifiers for E {}
