use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyElement, AnyView, App, Bounds, Context, ElementId, Entity, IntoElement, Pixels, Point, Render,
    RenderOnce, StyleRefinement, WeakEntity, Window, canvas, point, px, size,
};

use crate::bottom_sheet_ui::bottom_sheet_attached_motion_geometry::BottomSheetAttachedMotionGeometry;
use crate::bottom_sheet_ui::bottom_sheet_content_pan_gate::{
    BottomSheetContentPanGate, BottomSheetNestedScrollKey, BottomSheetNestedScrollRegistry,
};
use crate::bottom_sheet_ui::bottom_sheet_interaction_policy::BottomSheetSnapAnimationCurve;
use crate::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use crate::swiftui::environment::env_scope;
use crate::swiftui::motion::CubicBezier;
use crate::swiftui::prelude::*;

type HostContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
type LiveFrameObserver = Rc<dyn Fn(Option<Bounds<Pixels>>, &mut App)>;
type AttachedLayoutObserver = Rc<dyn Fn(Bounds<Pixels>, &mut App)>;
type BackdropTap = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;
type ContentPanBegan = Rc<dyn Fn(&mut Window, &mut App)>;
type ContentPanEnded = Rc<dyn Fn(Pixels, Pixels, &mut Window, &mut App)>;
type MotionCompletion = Box<dyn FnOnce(&mut App)>;
type TranslationHook = Rc<dyn Fn(Pixels)>;
type PhaseHook = Rc<dyn Fn(&str)>;

/// content that can be laid out taller than the surface while it moves
/// a detent change reveals existing rows and doesnt lay the tree out every frame
pub trait BottomSheetContentHeightHolding {
    /// extra is how far above its current box the content should be laid out, None returns it to the box
    fn hold_content_height(&self, extra: Option<Pixels>, cx: &mut App);
}

#[derive(Default)]
struct MotionControllerState {
    host: Option<WeakEntity<BottomSheetMotionHostingController>>,
    pending_presentation: Option<(Pixels, f64)>,
    content_height_holders: HashMap<usize, WeakContentHeightHolder>,
}

struct WeakContentHeightHolder {
    holder: Weak<dyn BottomSheetContentHeightHolding>,
}

/// a shared handle, the sheet and its hosting controller hold the same controller
#[derive(Clone, Default)]
pub struct BottomSheetMotionController {
    state: Rc<RefCell<MotionControllerState>>,
}

fn holder_key(holder: &Rc<dyn BottomSheetContentHeightHolding>) -> usize {
    Rc::as_ptr(holder).cast::<()>() as usize
}

impl BottomSheetMotionController {
    pub fn new() -> Self {
        Self::default()
    }

    fn host(&self) -> Option<Entity<BottomSheetMotionHostingController>> {
        self.state.borrow().host.as_ref().and_then(WeakEntity::upgrade)
    }

    pub fn register_content_height_holder(&self, holder: &Rc<dyn BottomSheetContentHeightHolding>) {
        self.state
            .borrow_mut()
            .content_height_holders
            .insert(holder_key(holder), WeakContentHeightHolder { holder: Rc::downgrade(holder) });
    }

    pub fn unregister_content_height_holder(&self, holder: &Rc<dyn BottomSheetContentHeightHolding>) {
        self.state.borrow_mut().content_height_holders.remove(&holder_key(holder));
    }

    pub fn hold_content_height(&self, extra: Option<Pixels>, cx: &mut App) {
        let holders: Vec<Rc<dyn BottomSheetContentHeightHolding>> = {
            let mut state = self.state.borrow_mut();
            state.content_height_holders.retain(|_, entry| entry.holder.strong_count() > 0);
            state
                .content_height_holders
                .values()
                .filter_map(|entry| entry.holder.upgrade())
                .collect()
        };
        for holder in holders {
            holder.hold_content_height(extra, cx);
        }
    }

    pub fn attach(&self, host: &Entity<BottomSheetMotionHostingController>, cx: &mut App) {
        let pending = {
            let mut state = self.state.borrow_mut();
            state.host = Some(host.downgrade());
            state.pending_presentation.take()
        };
        if let Some((offset, duration)) = pending {
            host.update(cx, |host, cx| host.animate_presentation(offset, duration, cx));
        }
    }

    pub fn detach(&self, host: &Entity<BottomSheetMotionHostingController>) {
        let mut state = self.state.borrow_mut();
        let is_attached = state
            .host
            .as_ref()
            .is_some_and(|attached| attached.entity_id() == host.entity_id());
        if is_attached {
            state.host = None;
        }
    }

    pub fn prepare_presentation(&self, offset: Pixels, duration: f64, cx: &mut App) {
        self.state.borrow_mut().pending_presentation = Some((offset, duration));

        let Some(host) = self.host() else {
            return;
        };
        self.state.borrow_mut().pending_presentation = None;
        host.update(cx, |host, cx| host.animate_presentation(offset, duration, cx));
    }

    pub fn begin_interaction(
        &self,
        resting_height: Pixels,
        minimum_height: Pixels,
        maximum_height: Pixels,
        cx: &mut App,
    ) -> Pixels {
        let Some(host) = self.host() else {
            return resting_height;
        };
        host.update(cx, |host, cx| {
            host.begin_interaction(resting_height, minimum_height, maximum_height, cx)
        })
    }

    /// none while no host is attached, so callers fall back to the value their own update carried
    pub fn expansion_room(&self, cx: &App) -> Option<Pixels> {
        self.host().map(|host| host.read(cx).expansion_room())
    }

    pub fn interaction_expansion_room(&self, cx: &App) -> Option<Pixels> {
        self.host().map(|host| host.read(cx).interaction_expansion_room())
    }

    pub fn refuses_interaction(&self, cx: &App) -> bool {
        self.host().is_some_and(|host| host.read(cx).refuses_interaction())
    }

    pub fn update_interactive_translation(&self, translation: Pixels, cx: &mut App) {
        if let Some(host) = self.host() {
            host.update(cx, |host, cx| host.update_interactive_translation(translation, cx));
        }
    }

    pub fn finish_interactive_updates(&self, cx: &mut App) {
        if let Some(host) = self.host() {
            host.update(cx, |host, cx| host.finish_interactive_updates(cx));
        }
    }

    pub fn animate_snap(
        &self,
        height: Pixels,
        duration: f64,
        curve: BottomSheetSnapAnimationCurve,
        completion: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) {
        let Some(host) = self.host() else {
            completion(cx);
            return;
        };
        host.update(cx, |host, cx| {
            host.animate_snap(height, duration, curve, true, Box::new(completion), cx);
        });
    }

    pub fn animate_dismiss(
        &self,
        safe_bottom: Pixels,
        duration: f64,
        completion: impl FnOnce(&mut App) + 'static,
        cx: &mut App,
    ) {
        let Some(host) = self.host() else {
            completion(cx);
            return;
        };
        host.update(cx, |host, cx| {
            host.animate_dismiss(safe_bottom, duration, Box::new(completion), cx);
        });
    }
}

/// creates the hosting entity once per identity and feeds it the newest config on every render
#[derive(IntoElement)]
pub struct BottomSheetMotionHost {
    id: ElementId,
    motion_controller: BottomSheetMotionController,
    pub resting_height: Pixels,
    pub maximum_height: Pixels,
    pub snap_animation_duration: f64,
    pub snap_animation_curve: BottomSheetSnapAnimationCurve,
    pub allows_dismissal_preview: bool,
    pub surface_corner_radius: Pixels,
    pub rounds_bottom_corners: bool,
    live_frame_observer: Option<LiveFrameObserver>,
    on_backdrop_tap: Option<BackdropTap>,
    pub owns_content_pan: bool,
    pub content_pan_locked: bool,
    on_content_pan_began: ContentPanBegan,
    on_content_pan_ended: ContentPanEnded,
    content: HostContent,
}

impl BottomSheetMotionHost {
    pub fn new<E: IntoElement>(
        id: impl Into<ElementId>,
        motion_controller: BottomSheetMotionController,
        content: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            motion_controller,
            resting_height: px(1.0),
            maximum_height: px(1.0),
            snap_animation_duration: BottomSheetMetrics::SNAP_DURATION,
            snap_animation_curve: BottomSheetSnapAnimationCurve::EaseOut,
            allows_dismissal_preview: false,
            surface_corner_radius: px(0.0),
            rounds_bottom_corners: false,
            live_frame_observer: None,
            on_backdrop_tap: None,
            owns_content_pan: false,
            content_pan_locked: false,
            on_content_pan_began: Rc::new(|_, _| {}),
            on_content_pan_ended: Rc::new(|_, _, _, _| {}),
            content: Rc::new(move |window, cx| content(window, cx).into_any_element()),
        }
    }

    pub fn resting_height(mut self, height: Pixels) -> Self {
        self.resting_height = height;
        self
    }

    pub fn maximum_height(mut self, height: Pixels) -> Self {
        self.maximum_height = height;
        self
    }

    pub fn snap_animation_duration(mut self, duration: f64) -> Self {
        self.snap_animation_duration = duration;
        self
    }

    pub fn snap_animation_curve(mut self, curve: BottomSheetSnapAnimationCurve) -> Self {
        self.snap_animation_curve = curve;
        self
    }

    pub fn allows_dismissal_preview(mut self, allows: bool) -> Self {
        self.allows_dismissal_preview = allows;
        self
    }

    pub fn surface_corner_radius(mut self, radius: Pixels) -> Self {
        self.surface_corner_radius = radius;
        self
    }

    pub fn rounds_bottom_corners(mut self, rounds: bool) -> Self {
        self.rounds_bottom_corners = rounds;
        self
    }

    /// left unset for nil
    pub fn live_frame_observer(
        mut self,
        observer: impl Fn(Option<Bounds<Pixels>>, &mut App) + 'static,
    ) -> Self {
        self.live_frame_observer = Some(Rc::new(observer));
        self
    }

    /// left unset while the dim takes no clicks
    pub fn on_backdrop_tap(
        mut self,
        handler: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_backdrop_tap = Some(Rc::new(handler));
        self
    }

    pub fn owns_content_pan(mut self, owns: bool) -> Self {
        self.owns_content_pan = owns;
        self
    }

    pub fn content_pan_locked(mut self, locked: bool) -> Self {
        self.content_pan_locked = locked;
        self
    }

    pub fn on_content_pan_began(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_content_pan_began = Rc::new(action);
        self
    }

    pub fn on_content_pan_ended(
        mut self,
        action: impl Fn(Pixels, Pixels, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_content_pan_ended = Rc::new(action);
        self
    }
}

impl RenderOnce for BottomSheetMotionHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let content = self.content.clone();
        let made = Rc::new(Cell::new(false));
        let made_flag = made.clone();
        let controller = window.use_keyed_state(self.id.clone(), cx, move |_, cx| {
            made_flag.set(true);
            BottomSheetMotionHostingController::new(content, cx)
        });
        if made.get() {
            self.make_ui_view_controller(&controller, cx);
        } else {
            self.update_ui_view_controller(&controller, cx);
        }

        let motion_controller = self.motion_controller.clone();
        let dismantled = controller.clone();
        let presence_id = ElementId::NamedChild(Arc::new(self.id), "motion-host-presence".into());
        controller.on_disappear(presence_id, move |_, cx| {
            Self::dismantle_ui_view_controller(&dismantled, &motion_controller, cx);
        })
    }
}

impl BottomSheetMotionHost {
    fn make_ui_view_controller(
        &self,
        controller: &Entity<BottomSheetMotionHostingController>,
        cx: &mut App,
    ) {
        controller.update(cx, |host, cx| {
            host.update_configuration(
                self.resting_height,
                self.maximum_height,
                self.snap_animation_duration,
                self.snap_animation_curve,
                self.allows_dismissal_preview,
                false,
                cx,
            );
            host.update_surface_corner_radius(self.surface_corner_radius, self.rounds_bottom_corners, cx);
            host.live_frame_observer = self.live_frame_observer.clone();
            host.update_backdrop_tap(self.on_backdrop_tap.clone());
            self.apply_content_pan(host, cx);
            host.motion_controller = Some(self.motion_controller.clone());
        });
        self.motion_controller.attach(controller, cx);
    }

    fn update_ui_view_controller(
        &self,
        controller: &Entity<BottomSheetMotionHostingController>,
        cx: &mut App,
    ) {
        controller.update(cx, |host, cx| {
            host.motion_controller = Some(self.motion_controller.clone());
            host.update_root_view(self.content.clone());
            host.update_configuration(
                self.resting_height,
                self.maximum_height,
                self.snap_animation_duration,
                self.snap_animation_curve,
                self.allows_dismissal_preview,
                true,
                cx,
            );
            host.update_surface_corner_radius(self.surface_corner_radius, self.rounds_bottom_corners, cx);
            host.live_frame_observer = self.live_frame_observer.clone();
            host.update_backdrop_tap(self.on_backdrop_tap.clone());
            self.apply_content_pan(host, cx);
        });
        self.motion_controller.attach(controller, cx);
    }

    fn apply_content_pan(
        &self,
        host: &mut BottomSheetMotionHostingController,
        cx: &mut Context<BottomSheetMotionHostingController>,
    ) {
        host.configure_content_pan(
            self.owns_content_pan,
            self.content_pan_locked,
            self.on_content_pan_began.clone(),
            self.on_content_pan_ended.clone(),
            cx,
        );
    }

    fn dismantle_ui_view_controller(
        controller: &Entity<BottomSheetMotionHostingController>,
        motion_controller: &BottomSheetMotionController,
        cx: &mut App,
    ) {
        let attached = controller.read(cx).motion_controller.clone();
        attached.unwrap_or_else(|| motion_controller.clone()).detach(controller);
        controller.update(cx, |host, cx| host.teardown_motion(cx));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MotionPhase {
    Presentation,
    Snap,
    Dismissal,
}

struct SnapAnimation {
    start_height: Pixels,
    target_height: Pixels,
    start_translation_y: Pixels,
    start_time: Instant,
    duration: f64,
    curve: BottomSheetSnapAnimationCurve,
    completion: MotionCompletion,
}

enum AnimatorCompletion {
    Presentation,
    Dismissal(MotionCompletion),
}

/// ease out animator on the surface transform
/// stopping it leaves translation_y where the last frame put it
struct PropertyAnimator {
    from_translation_y: Pixels,
    to_translation_y: Pixels,
    start_time: Instant,
    duration: f64,
    completion: AnimatorCompletion,
}

pub struct BottomSheetMotionHostingController {
    pub motion_controller: Option<BottomSheetMotionController>,

    root_view: HostContent,
    content_view: Option<Entity<BottomSheetMotionContentView>>,
    content_is_stale: bool,
    height: Pixels,
    translation_y: Pixels,
    maximum_height: Pixels,
    resting_height: Pixels,
    interaction_base_height: Pixels,
    interaction_minimum_height: Pixels,
    snap_animation_duration: f64,
    snap_animation_curve: BottomSheetSnapAnimationCurve,
    allows_dismissal_preview: bool,
    pending_translation: Option<Pixels>,
    pending_root_view: Option<HostContent>,
    pending_configuration_height: Option<Pixels>,
    pending_configuration_animated: bool,
    display_link: bool,
    frame_scheduled: bool,
    frame_scheduled_at: Option<Instant>,
    snap_animation: Option<SnapAnimation>,
    animator: Option<PropertyAnimator>,
    motion_phase: Option<MotionPhase>,
    defers_root_view_during_snap: bool,
    is_interacting: bool,
    pub attached_layout_observer: Option<AttachedLayoutObserver>,
    pub live_frame_observer: Option<LiveFrameObserver>,

    content_pan_gate: Option<Rc<RefCell<BottomSheetContentPanGate>>>,
    content_pan_is_locked: bool,
    on_content_pan_began: ContentPanBegan,
    on_content_pan_ended: ContentPanEnded,

    surface_corner_radius: Pixels,
    rounds_bottom_corners: bool,
    on_backdrop_tap: Option<BackdropTap>,
    container_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    nested_scrolls: BottomSheetNestedScrollRegistry,
}

impl BottomSheetMotionHostingController {
    pub fn new(root_view: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>, _cx: &mut Context<Self>) -> Self {
        Self {
            motion_controller: None,
            root_view,
            content_view: None,
            content_is_stale: true,
            height: px(1.0),
            translation_y: px(0.0),
            maximum_height: px(1.0),
            resting_height: px(1.0),
            interaction_base_height: px(1.0),
            interaction_minimum_height: px(1.0),
            snap_animation_duration: BottomSheetMetrics::SNAP_DURATION,
            snap_animation_curve: BottomSheetSnapAnimationCurve::EaseOut,
            allows_dismissal_preview: false,
            pending_translation: None,
            pending_root_view: None,
            pending_configuration_height: None,
            pending_configuration_animated: true,
            display_link: false,
            frame_scheduled: false,
            frame_scheduled_at: None,
            snap_animation: None,
            animator: None,
            motion_phase: None,
            defers_root_view_during_snap: false,
            is_interacting: false,
            attached_layout_observer: None,
            live_frame_observer: None,
            content_pan_gate: None,
            content_pan_is_locked: false,
            on_content_pan_began: Rc::new(|_, _| {}),
            on_content_pan_ended: Rc::new(|_, _, _, _| {}),
            surface_corner_radius: px(0.0),
            rounds_bottom_corners: false,
            on_backdrop_tap: None,
            container_bounds: Rc::new(Cell::new(None)),
            nested_scrolls: BottomSheetNestedScrollRegistry::default(),
        }
    }

    pub fn is_interacting(&self) -> bool {
        self.is_interacting
    }

    pub fn content_pan_is_locked(&self) -> bool {
        self.content_pan_is_locked
    }

    pub fn expansion_room(&self) -> Pixels {
        px(0.0).max(self.maximum_height - self.resting_height)
    }

    pub fn interaction_expansion_room(&self) -> Pixels {
        px(0.0).max(self.maximum_height - self.interaction_base_height)
    }

    pub fn refuses_interaction(&self) -> bool {
        self.content_pan_is_locked || self.motion_phase == Some(MotionPhase::Dismissal)
    }

    pub fn is_motion_active(&self) -> bool {
        self.is_interacting
            || self.animator.is_some()
            || self.display_link
            || self.snap_animation.is_some()
    }

    pub(crate) fn nested_scrolls(&self) -> &BottomSheetNestedScrollRegistry {
        &self.nested_scrolls
    }

    fn defers_root_view_updates(&self) -> bool {
        self.is_interacting
            || self.display_link
            || self.motion_phase == Some(MotionPhase::Presentation)
            || (self.motion_phase == Some(MotionPhase::Snap) && self.defers_root_view_during_snap)
            || self.motion_phase == Some(MotionPhase::Dismissal)
    }

    fn is_in_window(&self) -> bool {
        self.container_bounds.get().is_some()
    }

    pub fn update_root_view(&mut self, root_view: HostContent) {
        if self.defers_root_view_updates() {
            self.pending_root_view = Some(root_view);
            return;
        }
        self.pending_root_view = None;
        self.root_view = root_view;
        self.content_is_stale = true;
    }

    /// rounds the surface on its own box, not through a clip over the content
    /// a mask costs a compositor pass on every drag frame
    pub fn update_surface_corner_radius(
        &mut self,
        radius: Pixels,
        rounds_bottom_corners: bool,
        cx: &mut Context<Self>,
    ) {
        if (self.surface_corner_radius - radius).abs() <= px(0.01)
            && self.rounds_bottom_corners == rounds_bottom_corners
        {
            return;
        }
        self.surface_corner_radius = radius;
        self.rounds_bottom_corners = rounds_bottom_corners;
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_configuration(
        &mut self,
        resting_height: Pixels,
        maximum_height: Pixels,
        snap_animation_duration: f64,
        snap_animation_curve: BottomSheetSnapAnimationCurve,
        allows_dismissal_preview: bool,
        animated: bool,
        cx: &mut Context<Self>,
    ) {
        let maximum_height = px(1.0).max(maximum_height);
        if self.maximum_height != maximum_height {
            cx.notify();
        }
        self.maximum_height = maximum_height;
        self.snap_animation_duration = snap_animation_duration;
        self.snap_animation_curve = snap_animation_curve;
        self.allows_dismissal_preview = allows_dismissal_preview;
        let next_height = self.maximum_height.min(px(1.0).max(resting_height));
        self.resting_height = next_height;

        if self.motion_phase == Some(MotionPhase::Presentation) {
            self.pending_configuration_height = None;
            self.set_height(next_height, cx);
            return;
        }
        if self.is_motion_active() {
            self.pending_configuration_height = Some(next_height);
            self.pending_configuration_animated = animated;
            return;
        }

        self.pending_configuration_height = None;
        if (self.height - next_height).abs() <= px(0.5) {
            return;
        }

        if animated && self.is_in_window() {
            let duration = self.snap_animation_duration;
            let curve = self.snap_animation_curve;
            self.animate_snap(next_height, duration, curve, false, Box::new(|_: &mut App| {}), cx);
        } else {
            self.set_height(next_height, cx);
        }
    }

    fn set_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        if self.height == height {
            return;
        }
        self.height = height;
        cx.notify();
    }

    pub fn animate_presentation(&mut self, offset: Pixels, duration: f64, cx: &mut Context<Self>) {
        self.cancel_snap_animation();
        self.stop_animator();
        self.stop_display_link();
        self.is_interacting = false;
        self.pending_translation = None;
        self.hold_content_height(None, cx);
        self.translation_y = px(0.0).max(offset);

        self.motion_phase = Some(MotionPhase::Presentation);
        self.animator = Some(PropertyAnimator {
            from_translation_y: self.translation_y,
            to_translation_y: px(0.0),
            start_time: cx.background_executor().now(),
            duration,
            completion: AnimatorCompletion::Presentation,
        });
        if self.live_frame_observer.is_some() {
            self.start_display_link_if_needed();
        }
        cx.notify();
    }

    pub fn begin_interaction(
        &mut self,
        resting_height: Pixels,
        minimum_height: Pixels,
        maximum_height: Pixels,
        cx: &mut Context<Self>,
    ) -> Pixels {
        if self.motion_phase == Some(MotionPhase::Dismissal) {
            return resting_height;
        }
        let interrupted = self.is_motion_active();
        self.cancel_snap_animation();
        self.stop_animator();
        self.stop_display_link();
        let rendered_height = self.rendered_sheet_height();
        self.apply_pending_root_view();
        self.pending_configuration_height = None;

        let ceiling = if interrupted { maximum_height.max(rendered_height) } else { maximum_height };
        self.maximum_height = px(1.0).max(px(1.0).max(resting_height).max(ceiling));
        let base = self
            .maximum_height
            .min(px(1.0).max(if interrupted { rendered_height } else { resting_height }));
        self.resting_height = base;
        self.interaction_base_height = base;
        self.interaction_minimum_height = self.interaction_base_height.min(px(1.0).max(minimum_height));
        self.height = self.interaction_base_height;
        self.translation_y = px(0.0);
        self.pending_translation = Some(px(0.0));
        self.is_interacting = true;
        BottomSheetMotionDiagnostics::report_interaction_phase("begin");
        let extra = px(0.0).max(self.maximum_height - base);
        self.hold_content_height(Some(extra), cx);
        cx.notify();
        base
    }

    fn rendered_sheet_height(&self) -> Pixels {
        self.height - self.translation_y
    }

    pub fn update_interactive_translation(&mut self, translation: Pixels, cx: &mut Context<Self>) {
        if !self.is_interacting {
            return;
        }
        self.pending_translation = Some(translation);
        BottomSheetMotionDiagnostics::report_interactive_translation(translation);
        self.start_display_link_if_needed();
        cx.notify();
    }

    pub fn finish_interactive_updates(&mut self, cx: &mut Context<Self>) {
        self.apply_pending_translation(cx);
        self.stop_display_link();
        BottomSheetMotionDiagnostics::report_interaction_phase("end");
    }

    pub fn animate_snap(
        &mut self,
        height: Pixels,
        duration: f64,
        curve: BottomSheetSnapAnimationCurve,
        defers_root_view_updates: bool,
        completion: MotionCompletion,
        cx: &mut Context<Self>,
    ) {
        self.finish_interactive_updates(cx);
        self.cancel_snap_animation();
        self.stop_animator();
        self.is_interacting = false;
        self.pending_configuration_height = None;

        let target_height = self.maximum_height.min(px(1.0).max(height));
        self.resting_height = target_height;
        self.defers_root_view_during_snap = defers_root_view_updates;
        self.motion_phase = Some(MotionPhase::Snap);

        let start_height = self.height;
        let start_translation_y = self.translation_y;
        if (start_height - target_height).abs() <= px(0.5) && start_translation_y.abs() <= px(0.5) {
            self.apply_attached_height(target_height, cx);
            self.finish_snap_animation(completion, cx);
            return;
        }

        self.hold_content_height(Some(px(0.0).max(target_height - start_height)), cx);

        self.snap_animation = Some(SnapAnimation {
            start_height,
            target_height,
            start_translation_y,
            start_time: cx.background_executor().now(),
            duration: duration.max(0.01),
            curve,
            completion,
        });
        self.start_display_link_if_needed();
        cx.notify();
    }

    pub fn animate_dismiss(
        &mut self,
        safe_bottom: Pixels,
        duration: f64,
        completion: MotionCompletion,
        cx: &mut Context<Self>,
    ) {
        self.finish_interactive_updates(cx);
        self.cancel_snap_animation();
        self.stop_animator();
        self.is_interacting = false;
        self.pending_configuration_height = None;

        let travel = self.height + px(0.0).max(safe_bottom) + px(40.0);
        self.motion_phase = Some(MotionPhase::Dismissal);
        self.animator = Some(PropertyAnimator {
            from_translation_y: self.translation_y,
            to_translation_y: travel,
            start_time: cx.background_executor().now(),
            duration: duration.max(0.01),
            completion: AnimatorCompletion::Dismissal(completion),
        });
        if self.live_frame_observer.is_some() {
            self.start_display_link_if_needed();
        }
        cx.notify();
    }

    pub fn teardown_motion(&mut self, cx: &mut Context<Self>) {
        self.cancel_snap_animation();
        self.stop_display_link();
        self.stop_animator();
        self.hold_content_height(None, cx);
        self.notify_live_frame(None, cx);
        self.pending_translation = None;
        self.pending_root_view = None;
        self.pending_configuration_height = None;
        self.is_interacting = false;
        self.configure_content_pan(
            false,
            true,
            Rc::new(|_, _| {}),
            Rc::new(|_, _, _, _| {}),
            cx,
        );
    }

    fn update_backdrop_tap(&mut self, handler: Option<BackdropTap>) {
        self.on_backdrop_tap = handler;
    }

    fn configure_content_pan(
        &mut self,
        enabled: bool,
        locked: bool,
        on_began: ContentPanBegan,
        on_ended: ContentPanEnded,
        cx: &mut Context<Self>,
    ) {
        self.content_pan_is_locked = locked;
        self.on_content_pan_began = on_began;
        self.on_content_pan_ended = on_ended;

        if !enabled {
            if self.content_pan_gate.take().is_some() {
                cx.notify();
            }
            return;
        }

        if self.content_pan_gate.is_some() {
            return;
        }
        let gate = BottomSheetContentPanGate::new(cx.weak_entity());
        self.content_pan_gate = Some(Rc::new(RefCell::new(gate)));
        cx.notify();
    }

    pub(crate) fn content_pan_did_begin(host: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let on_began = host.read(cx).on_content_pan_began.clone();
        on_began(window, cx);
    }

    pub(crate) fn content_pan_did_end(
        host: &Entity<Self>,
        translation_y: Pixels,
        velocity_y: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        let on_ended = host.read(cx).on_content_pan_ended.clone();
        on_ended(translation_y, velocity_y, window, cx);
    }

    fn hold_content_height(&self, extra: Option<Pixels>, cx: &mut App) {
        if let Some(motion_controller) = self.motion_controller.as_ref() {
            motion_controller.hold_content_height(extra, cx);
        }
    }

    fn start_display_link_if_needed(&mut self) {
        self.display_link = true;
    }

    fn stop_display_link(&mut self) {
        self.display_link = false;
    }

    const FRAME_CALLBACK_STALE_MS: u128 = 100;

    fn sync_animator_to_clock(&mut self, now: Instant) {
        let Some(animator) = self.animator.as_ref() else {
            return;
        };
        let elapsed = now.saturating_duration_since(animator.start_time).as_secs_f64();
        let linear_progress = (elapsed / animator.duration.max(0.01)).min(1.0);
        let eased = CubicBezier::EASE_OUT.progress(linear_progress) as f32;
        self.translation_y = animator.from_translation_y
            + (animator.to_translation_y - animator.from_translation_y) * eased;
    }

    fn needs_frames(&self) -> bool {
        self.display_link || self.animator.is_some()
    }

    fn schedule_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let stale = self.frame_scheduled_at.is_none_or(|at| {
            now.saturating_duration_since(at).as_millis() >= Self::FRAME_CALLBACK_STALE_MS
        });
        if self.frame_scheduled && !stale {
            return;
        }
        self.frame_scheduled = true;
        self.frame_scheduled_at = Some(now);
        let host = cx.weak_entity();
        window.on_next_frame(move |_, cx| {
            let _ = host.update(cx, |host, cx| {
                host.frame_scheduled = false;
                host.frame_scheduled_at = None;
                host.frame_tick(cx);
            });
        });
        window.request_animation_frame();
    }

    pub fn frame_tick(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        self.advance_animator(now, cx);
        if self.display_link {
            self.display_link_tick(now, cx);
        }
        cx.notify();
    }

    fn display_link_tick(&mut self, now: Instant, cx: &mut Context<Self>) {
        if self.snap_animation.is_some() {
            self.advance_snap_animation(now, cx);
        } else if matches!(
            self.motion_phase,
            Some(MotionPhase::Presentation) | Some(MotionPhase::Dismissal)
        ) {
            self.report_live_frame(cx);
        } else {
            self.apply_pending_translation(cx);
        }
    }

    fn advance_animator(&mut self, now: Instant, cx: &mut Context<Self>) {
        let Some(animator) = self.animator.as_ref() else {
            return;
        };
        let elapsed = now.saturating_duration_since(animator.start_time).as_secs_f64();
        let linear_progress = (elapsed / animator.duration.max(0.01)).min(1.0);
        let eased = CubicBezier::EASE_OUT.progress(linear_progress) as f32;
        self.translation_y = animator.from_translation_y
            + (animator.to_translation_y - animator.from_translation_y) * eased;
        if linear_progress < 1.0 {
            return;
        }
        let Some(animator) = self.animator.take() else {
            return;
        };
        self.motion_phase = None;
        match animator.completion {
            AnimatorCompletion::Presentation => {
                self.stop_display_link();
                self.report_live_frame(cx);
                self.reconcile_pending_updates(cx);
            }
            AnimatorCompletion::Dismissal(completion) => {
                self.pending_translation = None;
                self.stop_display_link();
                self.hold_content_height(None, cx);
                self.notify_live_frame(None, cx);
                Self::run_completion(completion, cx);
            }
        }
    }

    /// deferred to the end of this update
    /// a completion usually reaches back into the sheet through the motion controller
    /// and this entity is leased until the update returns
    fn run_completion(completion: MotionCompletion, cx: &mut Context<Self>) {
        cx.defer(move |cx| completion(cx));
    }

    /// vertical translation of the surface
    pub fn translation_y(&self) -> Pixels {
        self.translation_y
    }

    pub fn surface_frame(&self) -> Option<Bounds<Pixels>> {
        let container = self.container_bounds.get()?;
        Some(Bounds {
            origin: point(
                container.left(),
                container.bottom() - self.height + self.translation_y,
            ),
            size: size(container.size.width, self.height),
        })
    }

    fn report_live_frame(&self, cx: &mut App) {
        if self.live_frame_observer.is_none() {
            return;
        }
        let Some(frame) = self.surface_frame() else {
            return;
        };
        self.notify_live_frame(Some(frame), cx);
    }

    fn notify_live_frame(&self, frame: Option<Bounds<Pixels>>, cx: &mut App) {
        if let Some(observer) = self.live_frame_observer.clone() {
            observer(frame, cx);
        }
    }

    fn apply_pending_translation(&mut self, cx: &mut Context<Self>) {
        let Some(translation) = self.pending_translation.take() else {
            return;
        };
        let container = self.container_bounds.get().unwrap_or_default();
        let geometry = BottomSheetAttachedMotionGeometry::resolve_detent_interaction(
            container,
            self.interaction_base_height,
            self.interaction_minimum_height,
            self.maximum_height,
            translation,
            self.allows_dismissal_preview,
        );
        self.translation_y = geometry.translation_y;
        if (self.height - geometry.sheet_frame.size.height).abs() > px(0.5) {
            self.height = geometry.sheet_frame.size.height;
        }
        cx.notify();
        self.report_live_frame(cx);
    }

    fn advance_snap_animation(&mut self, now: Instant, cx: &mut Context<Self>) {
        let Some(snap) = self.snap_animation.as_ref() else {
            return;
        };

        let elapsed = now.saturating_duration_since(snap.start_time).as_secs_f64();
        let linear_progress = (elapsed / snap.duration).min(1.0) as f32;
        let eased_progress = snap.curve.solve(linear_progress, snap.duration);
        let height = snap.start_height + (snap.target_height - snap.start_height) * eased_progress;
        let translation_y = snap.start_translation_y * (1.0 - eased_progress);
        let target_height = snap.target_height;

        self.apply_attached_geometry(height, translation_y, cx);

        if linear_progress < 1.0 {
            return;
        }
        self.apply_attached_height(target_height, cx);
        if let Some(snap) = self.snap_animation.take() {
            self.finish_snap_animation(snap.completion, cx);
        }
    }

    fn apply_attached_height(&mut self, height: Pixels, cx: &mut Context<Self>) {
        self.apply_attached_geometry(height, px(0.0), cx);
    }

    fn apply_attached_geometry(&mut self, height: Pixels, translation_y: Pixels, cx: &mut Context<Self>) {
        self.translation_y = translation_y;
        self.height = self.maximum_height.min(px(1.0).max(height));
        if let (Some(observer), Some(frame)) =
            (self.attached_layout_observer.clone(), self.surface_frame())
        {
            observer(frame, cx);
        }
        cx.notify();
        self.report_live_frame(cx);
    }

    fn finish_snap_animation(&mut self, completion: MotionCompletion, cx: &mut Context<Self>) {
        self.snap_animation = None;
        self.stop_display_link();
        self.motion_phase = None;
        self.defers_root_view_during_snap = false;
        self.pending_translation = None;
        self.hold_content_height(None, cx);
        Self::run_completion(completion, cx);
        self.reconcile_pending_updates(cx);
    }

    fn cancel_snap_animation(&mut self) {
        if self.snap_animation.is_none() && self.motion_phase != Some(MotionPhase::Snap) {
            return;
        }
        self.snap_animation = None;
        self.stop_display_link();
        self.motion_phase = None;
        self.defers_root_view_during_snap = false;
    }

    /// stops the animation, the transform stays where the last frame left it and the completion never runs
    fn stop_animator(&mut self) {
        self.animator = None;
        self.motion_phase = None;
        self.defers_root_view_during_snap = false;
    }

    fn apply_pending_root_view(&mut self) {
        if let Some(pending_root_view) = self.pending_root_view.take() {
            self.root_view = pending_root_view;
            self.content_is_stale = true;
        }
    }

    fn reconcile_pending_updates(&mut self, cx: &mut Context<Self>) {
        if self.is_motion_active() {
            return;
        }
        self.apply_pending_root_view();
        self.reconcile_pending_configuration(cx);
        cx.notify();
    }

    fn reconcile_pending_configuration(&mut self, cx: &mut Context<Self>) {
        if self.is_motion_active() {
            return;
        }
        let Some(pending_height) = self.pending_configuration_height.take() else {
            return;
        };
        let animated = self.pending_configuration_animated;
        if (self.height - pending_height).abs() <= px(0.5) {
            return;
        }

        if animated && self.is_in_window() {
            let duration = self.snap_animation_duration;
            let curve = self.snap_animation_curve;
            self.animate_snap(pending_height, duration, curve, false, Box::new(|_: &mut App| {}), cx);
        } else {
            self.set_height(pending_height, cx);
        }
    }
}

impl Render for BottomSheetMotionHostingController {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.animator.is_some() {
            self.sync_animator_to_clock(cx.background_executor().now());
        }
        if self.needs_frames() {
            self.schedule_frame(window, cx);
        }
        let content = self.content_element(cx);

        let radius = self.surface_corner_radius;
        let mut surface = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(-self.translation_y)
            .h(self.height)
            .child(env_scope::<BottomSheetNestedScrollKey>(Some(self.nested_scrolls.clone())).child(content));
        if radius > px(0.0) {
            surface = surface.overflow_hidden().rounded_tl(radius).rounded_tr(radius);
            if self.rounds_bottom_corners {
                surface = surface.rounded_bl(radius).rounded_br(radius);
            }
        }
        if let Some(gate) = self.content_pan_gate.as_ref() {
            surface = surface.child(BottomSheetContentPanGate::recognizer_overlay(gate));
        }

        self.load_view(surface)
    }
}

impl BottomSheetMotionHostingController {
    fn replays_content(&self) -> bool {
        !self.content_is_stale
            && !self.is_interacting
            && matches!(self.motion_phase, Some(MotionPhase::Presentation) | Some(MotionPhase::Dismissal))
    }

    fn content_element(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let content_view = match self.content_view.clone() {
            Some(content_view) => content_view,
            None => {
                let root_view = self.root_view.clone();
                let content_view = cx.new(|_| BottomSheetMotionContentView { root_view });
                self.content_view = Some(content_view.clone());
                content_view
            }
        };
        if self.replays_content() {
            return content_view
                .cached(StyleRefinement::default().size_full())
                .replay_when_moved()
                .into_any_element();
        }
        self.nested_scrolls.clear();
        self.content_is_stale = false;
        let root_view = self.root_view.clone();
        content_view.update(cx, |content_view, _| content_view.root_view = root_view);
        div().size_full().child(AnyView::from(content_view)).into_any_element()
    }

    fn load_view(&self, surface: Div) -> AnyElement {
        let container_bounds = self.container_bounds.clone();
        let root = div()
            .relative()
            .size_full()
            .child(
                canvas(move |bounds, _, _| container_bounds.set(Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(surface);
        BottomSheetMotionRootView {
            sheet_view: self.surface_frame(),
            on_backdrop_tap: self.on_backdrop_tap.clone(),
        }
        .install(root)
    }
}

/// the sheet root view as a view of its own
/// renders whatever root the hosting controller installed last
struct BottomSheetMotionContentView {
    root_view: HostContent,
}

impl Render for BottomSheetMotionContentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root_view = self.root_view.clone();
        root_view(window, cx)
    }
}

struct BottomSheetMotionRootView {
    sheet_view: Option<Bounds<Pixels>>,
    on_backdrop_tap: Option<BackdropTap>,
}

impl BottomSheetMotionRootView {
    const TAKES_BACKDROP_TAPS: bool = false;

    fn point(&self, point: Point<Pixels>) -> bool {
        let Some(sheet_view) = self.sheet_view else {
            return false;
        };
        if sheet_view.contains(&point) {
            return true;
        }
        Self::TAKES_BACKDROP_TAPS && self.on_backdrop_tap.is_some()
    }

    fn gesture_recognizer(&self, location: Point<Pixels>) -> bool {
        if self.on_backdrop_tap.is_none() {
            return false;
        }
        let Some(sheet_view) = self.sheet_view else {
            return true;
        };
        !sheet_view.contains(&location)
    }

    fn install(self, root: Div) -> AnyElement {
        if !Self::TAKES_BACKDROP_TAPS {
            return root.into_any_element();
        }
        let Some(on_backdrop_tap) = self.on_backdrop_tap.clone() else {
            return root.into_any_element();
        };
        root.id("bottom-sheet-motion-backdrop")
            .on_click(move |event, window, cx| {
                let position = event.position();
                if self.point(position) && self.gesture_recognizer(position) {
                    on_backdrop_tap(position, window, cx);
                }
            })
            .into_any_element()
    }
}

thread_local! {
    static ON_INTERACTIVE_TRANSLATION: RefCell<Option<TranslationHook>> = const { RefCell::new(None) };
    static ON_INTERACTION_PHASE: RefCell<Option<PhaseHook>> = const { RefCell::new(None) };
}

/// diagnostic hooks for the sheet interactive motion
/// none in a normal run (one check per drag frame), set only while a perf recorder is active
/// so a trace can tell gesture frames from idle ones
pub enum BottomSheetMotionDiagnostics {}

impl BottomSheetMotionDiagnostics {
    pub fn set_on_interactive_translation(hook: Option<impl Fn(Pixels) + 'static>) {
        let hook = hook.map(|hook| Rc::new(hook) as TranslationHook);
        ON_INTERACTIVE_TRANSLATION.with(|slot| *slot.borrow_mut() = hook);
    }

    pub fn set_on_interaction_phase(hook: Option<impl Fn(&str) + 'static>) {
        let hook = hook.map(|hook| Rc::new(hook) as PhaseHook);
        ON_INTERACTION_PHASE.with(|slot| *slot.borrow_mut() = hook);
    }

    fn report_interactive_translation(translation: Pixels) {
        let hook = ON_INTERACTIVE_TRANSLATION.with(|slot| slot.borrow().clone());
        if let Some(hook) = hook {
            hook(translation);
        }
    }

    fn report_interaction_phase(phase: &str) {
        let hook = ON_INTERACTION_PHASE.with(|slot| slot.borrow().clone());
        if let Some(hook) = hook {
            hook(phase);
        }
    }
}
