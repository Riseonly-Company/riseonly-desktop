use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, AppContext, Bounds, Context, DispatchPhase, Edges, Entity, Global,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Render, RenderOnce,
    StyleRefinement, Styled, Subscription, Task, WeakEntity, Window, WindowId, anchored, canvas, deferred, div, point,
    px,
};
use futures::StreamExt;
use futures::channel::mpsc;
use rise_canon::{RiseGuideAnchor, RiseGuideGesture, RiseGuideNarrator, RiseGuidePlacement, RiseGuidePose, RiseGuideSpotlightShape, RiseGuideStage};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};

use crate::guide_overlay_ui::guide_attention_layer::GuideAttentionView;
use crate::guide_overlay_ui::guide_gesture_layer::GuideGestureView;
use crate::guide_overlay_ui::guide_narrator_view::GuideNarratorView;
use crate::guide_overlay_ui::guide_overlay_layout::{GuideOverlayFrames, GuideOverlayLayout, GuideOverlaySolveInput};
use crate::guide_overlay_ui::guide_overlay_theme::{GuideOverlayMotion, GuideOverlayTheme};
use crate::guide_overlay_ui::guide_path_layer::GuidePathView;
use crate::guide_overlay_ui::guide_spotlight_layer::{GuideSpotlightCutout, GuideSpotlightView};
use crate::guide_overlay_ui::guide_surface_views::{
    GuideBubbleTailSide, GuideCaptionContent, GuideOverlayAction, GuideOverlaySurfaceContent, GuideSurfaceView,
};
use crate::bottom_sheet_ui::modal_sheet_surface::ModalSheetStack;
use crate::guide_anchor_ui::guide_anchor_registry::GuideAnchorRegistry;
use crate::hold_context_menu_ui::hold_context_menu_frame_keys::HoldContextMenuHitTargets;
use crate::main_text::MainText;
use crate::simple_button_ui::SimpleButtonUi;
use crate::swiftui::animated::Animated;
use crate::swiftui::environment::{LayoutDirection, layout_direction, reduce_motion};
use crate::swiftui::measure::measure;
use crate::swiftui::motion::MotionSpec;
use crate::swiftui::stacks::{HorizontalAlignment, VerticalAlignment};

/// identity of the step on screen, built by the driver from guide plus step position
/// the kit only compares it
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GuideOverlayStepID {
    pub value: String,
}

impl GuideOverlayStepID {
    pub fn new(value: impl Into<String>) -> Self {
        Self { value: value.into() }
    }
}

/// everything the overlay needs to draw one step
/// rects are in window coordinates and a probe window frame lands here unchanged
#[derive(Clone)]
pub struct GuideOverlayStep {
    pub id: GuideOverlayStepID,
    pub stage: RiseGuideStage,
    pub anchor: Option<Bounds<Pixels>>,
    pub origin: Option<Bounds<Pixels>>,
    pub previous_cutout: Option<GuideSpotlightCutout>,
    pub shape: RiseGuideSpotlightShape,
    pub narrator: RiseGuideNarrator,
    pub pose: Option<RiseGuidePose>,
    pub placement: RiseGuidePlacement,
    pub surface: GuideOverlaySurfaceContent,
    pub keyboard_height: Pixels,
    pub exit: Option<GuideOverlayAction>,
    gesture: Option<RiseGuideGesture>,
    pub compact_body: Option<String>,
}

impl GuideOverlayStep {
    pub fn new(id: GuideOverlayStepID, stage: RiseGuideStage) -> Self {
        Self {
            id,
            stage,
            anchor: None,
            origin: None,
            previous_cutout: None,
            shape: RiseGuideSpotlightShape::RoundedRect,
            narrator: RiseGuideNarrator::None,
            pose: None,
            placement: RiseGuidePlacement::Auto,
            surface: GuideOverlaySurfaceContent::None,
            keyboard_height: px(0.0),
            exit: None,
            gesture: None,
            compact_body: None,
        }
    }

    pub fn anchor(mut self, anchor: Option<Bounds<Pixels>>) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn previous_cutout(mut self, previous_cutout: Option<GuideSpotlightCutout>) -> Self {
        self.previous_cutout = previous_cutout;
        self
    }

    pub fn shape(mut self, shape: RiseGuideSpotlightShape) -> Self {
        self.shape = shape;
        self
    }

    pub fn narrator(mut self, narrator: RiseGuideNarrator) -> Self {
        self.narrator = narrator;
        self
    }

    pub fn pose(mut self, pose: Option<RiseGuidePose>) -> Self {
        self.pose = pose;
        self
    }

    pub fn placement(mut self, placement: RiseGuidePlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn surface(mut self, surface: GuideOverlaySurfaceContent) -> Self {
        self.surface = surface;
        self
    }

    pub fn keyboard_height(mut self, keyboard_height: Pixels) -> Self {
        self.keyboard_height = keyboard_height;
        self
    }

    pub fn exit(mut self, exit: Option<GuideOverlayAction>) -> Self {
        self.exit = exit;
        self
    }

    pub fn gesture(mut self, gesture: Option<RiseGuideGesture>) -> Self {
        self.gesture = if self.stage.requires_anchor() { gesture } else { None };
        self
    }

    pub fn gesture_hint(&self) -> Option<RiseGuideGesture> {
        self.gesture
    }

    pub fn cutout(&self) -> Option<GuideSpotlightCutout> {
        if !self.stage.requires_anchor() {
            return None;
        }
        self.anchor.map(|anchor| GuideSpotlightCutout::new(anchor, self.shape))
    }

    pub fn shows_narrator(&self) -> bool {
        self.narrator == RiseGuideNarrator::Panda
    }

    /// same step folded to one caption line without the narrator
    /// a full bubble over a tall menu hides rows the reader needs
    pub fn folded(&self) -> GuideOverlayStep {
        let body = match &self.surface {
            GuideOverlaySurfaceContent::Bubble(content) => {
                self.compact_body.clone().unwrap_or_else(|| content.body.clone())
            }
            GuideOverlaySurfaceContent::Caption(content) => {
                self.compact_body.clone().unwrap_or_else(|| content.body.clone())
            }
            GuideOverlaySurfaceContent::None
            | GuideOverlaySurfaceContent::Card(_)
            | GuideOverlaySurfaceContent::Video(_) => return self.clone(),
        };
        let mut step = GuideOverlayStep::new(self.id.clone(), self.stage)
            .anchor(self.anchor)
            .previous_cutout(self.previous_cutout)
            .shape(self.shape)
            .narrator(RiseGuideNarrator::None)
            .pose(self.pose)
            .placement(self.placement)
            .surface(GuideOverlaySurfaceContent::Caption(GuideCaptionContent::new(body)))
            .keyboard_height(self.keyboard_height)
            .exit(self.exit.clone())
            .gesture(self.gesture);
        step.origin = self.origin;
        step.compact_body = self.compact_body.clone();
        step
    }

    /// true when the step has an exit, no surface holds it and the guide is on screen
    pub fn draws_corner_exit(&self, is_aside: bool) -> bool {
        self.exit.is_some() && !is_aside && !self.surface.has_action_row()
    }

    pub fn narrator_pose(&self) -> RiseGuidePose {
        self.pose.unwrap_or(RiseGuidePose::Idle)
    }
}

/// what the overlay may claim from a press, its own chrome and nothing else
/// everything outside chrome reaches the app even under the dim scrim
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuideOverlayHitPolicy {
    pub is_active: bool,
    pub blocks_application: bool,
    pub cutout: Option<GuideSpotlightCutout>,
    pub chrome_rects: Vec<Bounds<Pixels>>,
    pub chrome_over_cutout: bool,
}

impl GuideOverlayHitPolicy {
    pub fn inactive() -> Self {
        Self::default()
    }

    pub fn chrome_contains(&self, point: Point<Pixels>) -> bool {
        self.chrome_rects.iter().any(|rect| rect.contains(&point))
    }

    /// hit test of the overlay window, cutout passes presses through
    /// surface and exit answer inside their frames, the rest passes through
    pub fn claims(&self, point: Point<Pixels>) -> bool {
        if !self.is_active {
            return false;
        }
        if self.chrome_over_cutout && self.chrome_contains(point) {
            return true;
        }
        if self.cutout.is_some_and(|cutout| cutout.contains(point)) {
            return false;
        }
        self.chrome_contains(point)
    }
}

/// overlay layer in one gpui window, mounted once as the last child
/// draws only in the window the guide was presented in and claims nothing without a step
#[derive(IntoElement, Default)]
pub struct GuideOverlayWindow;

impl GuideOverlayWindow {
    pub fn new() -> Self {
        Self
    }

    pub fn hit_test(point: Point<Pixels>, window: AnyWindowHandle, cx: &App) -> bool {
        let host = GuideOverlayHost::shared(cx);
        if host.window != Some(window) {
            return false;
        }
        host.controller
            .as_ref()
            .is_some_and(|controller| controller.read(cx).hit_policy().claims(point))
    }
}

impl RenderOnce for GuideOverlayWindow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = GuideOverlayHost::shared(cx);
        let handle = window.window_handle();
        HOSTING_WINDOWS.with(|hosts| {
            let mut hosts = hosts.borrow_mut();
            if !hosts.contains(&handle) {
                hosts.push(handle);
            }
        });
        match (host.window, host.controller.clone()) {
            (Some(host_window), Some(controller)) if host_window == handle => {
                let is_idle = controller.read(cx).step().is_none();
                let lit_menu_row = controller
                    .read(cx)
                    .step()
                    .and_then(GuideOverlayStep::cutout)
                    .and_then(|cutout| HoldContextMenuHitTargets::lifted_actions_frame(cutout.resolved_rect()))
                    .is_some();
                let level = if lit_menu_row { GuideOverlayHost::ABOVE_HOLD_MENU_LEVEL } else { host.level };
                let viewport = window.viewport_size();
                let overlay = if is_idle {
                    AnyView::from(controller)
                        .cached(StyleRefinement::default().w(viewport.width).h(viewport.height))
                        .into_any_element()
                } else {
                    controller.into_any_element()
                };
                deferred(anchored().position(point(px(0.0), px(0.0))).child(overlay))
                    .with_priority(level)
                    .into_any_element()
            }
            _ => div().into_any_element(),
        }
    }
}

const SURFACE_FRAME_KEY: &str = "surface";
const EXIT_FRAME_KEY: &str = "exit";
const MENU_BESIDE_GAP: f32 = 24.0;
const COMPOSER_BESIDE_GAP: f32 = 96.0;

/// root of the overlay, scrim ring hint line and chrome
/// scrim and line are painted layers so a moving spotlight doesnt re-solve placement per frame
pub struct GuideOverlayRootController {
    spotlight_view: GuideSpotlightView,
    attention_view: GuideAttentionView,
    gesture_view: GuideGestureView,
    path_view: GuidePathView,
    chrome: GuideOverlayChrome,
    chrome_fade: Animated<f32>,
    is_chrome_suppressed: bool,
    is_interaction_enabled: bool,

    step: Option<GuideOverlayStep>,
    pinged_step: Option<GuideOverlayStepID>,
    chrome_frames: HashMap<&'static str, Bounds<Pixels>>,
    chrome_rects: Vec<Bounds<Pixels>>,
    is_chrome_cramped: bool,
    bounds: Bounds<Pixels>,
    rendered_container: Bounds<Pixels>,
    rendered_safe_area: Edges<Pixels>,

    is_aside: bool,
    is_narrating_aside: bool,
    gesture_animates: bool,
    seen_menu_panel: Option<Bounds<Pixels>>,
    seen_dialog: Option<Bounds<Pixels>>,
    _subscriptions: Vec<Subscription>,
}

impl GuideOverlayRootController {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let handle = window.window_handle();
        let activation = cx.observe_window_activation(window, move |_, window, cx| {
            if !window.is_window_active() {
                cx.defer(move |cx| GuideOverlayHost::scene_will_deactivate(handle, cx));
            }
        });
        let resize = cx.observe_window_bounds(window, |_, _, cx| cx.notify());
        let bounds = Bounds::new(point(px(0.0), px(0.0)), window.viewport_size());
        let weak = cx.weak_entity();
        let mut spotlight_view = GuideSpotlightView::new();
        spotlight_view.layout_subviews(bounds);
        Self {
            spotlight_view,
            attention_view: GuideAttentionView::new(),
            gesture_view: GuideGestureView::new(),
            path_view: GuidePathView::new(),
            chrome: GuideOverlayChrome::empty(weak, bounds),
            chrome_fade: Animated::new(1.0),
            is_chrome_suppressed: false,
            is_interaction_enabled: true,
            step: None,
            pinged_step: None,
            chrome_frames: HashMap::new(),
            chrome_rects: Vec::new(),
            is_chrome_cramped: false,
            bounds,
            rendered_container: bounds,
            rendered_safe_area: Edges::default(),
            is_aside: false,
            is_narrating_aside: false,
            gesture_animates: true,
            seen_menu_panel: None,
            seen_dialog: None,
            _subscriptions: vec![activation, resize],
        }
    }

    pub fn step(&self) -> Option<&GuideOverlayStep> {
        self.step.as_ref()
    }

    pub fn is_aside(&self) -> bool {
        self.is_aside
    }

    pub fn hit_policy(&self) -> GuideOverlayHitPolicy {
        let Some(step) = &self.step else {
            return GuideOverlayHitPolicy::inactive();
        };
        GuideOverlayHitPolicy {
            is_active: true,
            blocks_application: step.stage.dims() && !self.is_aside,
            cutout: if self.is_aside { None } else { step.cutout() },
            chrome_rects: if self.is_chrome_suppressed { Vec::new() } else { self.chrome_rects.clone() },
            chrome_over_cutout: self.is_chrome_cramped && !self.is_chrome_suppressed,
        }
    }

    fn set_interaction_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.is_interaction_enabled != enabled {
            self.is_interaction_enabled = enabled;
            cx.notify();
        }
    }

    fn fade_spec(motion: GuideOverlayMotion) -> Option<MotionSpec> {
        GuideOverlayTheme::fade_animation(motion)
    }

    /// installs a step, or none to take the guide off screen
    pub fn apply(&mut self, step: Option<GuideOverlayStep>, animated: bool, cx: &mut Context<Self>) {
        if self.pinged_step != step.as_ref().map(|step| step.id.clone()) {
            self.pinged_step = None;
        }
        let had_group = self.chrome.draws_group();
        self.step = step;
        self.is_aside = false;
        self.is_narrating_aside = false;
        let motion = GuideOverlayMotion::current(cx);
        let animates = animated && motion.animates_transitions();
        self.gesture_animates = motion.animates_transitions();
        if self.step.is_none() {
            self.gesture_view.gesture_layer.stop();
        }

        self.update_scene(motion, animates, cx);

        self.rendered_container = self.bounds;
        self.rendered_safe_area = Edges::default();
        self.chrome = self.make_chrome(cx);
        match Self::fade_spec(motion).filter(|_| animates && !had_group && self.chrome.draws_group()) {
            Some(spec) => {
                self.chrome_fade.jump(0.0);
                self.chrome_fade.set(1.0, spec, RiseClock::monotonic(), false);
            }
            None => self.chrome_fade.jump(1.0),
        }
        if self.step.is_none() {
            self.chrome_frames.clear();
            self.chrome_rects.clear();
            self.is_chrome_cramped = false;
        }
        self.refresh_chrome_visibility();
        cx.notify();
    }

    /// follows the moved element, the hole follows every sample unanimated
    /// narrator and bubble re-solve placement only once the anchor settles
    pub fn move_anchor(
        &mut self,
        frame: Bounds<Pixels>,
        resolves_chrome: bool,
        origin: Option<Bounds<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        if !self.step.as_ref().is_some_and(|step| step.stage.requires_anchor()) {
            return;
        }
        let had_anchor = self.step.as_ref().is_some_and(|step| step.anchor.is_some());
        if let Some(step) = self.step.as_mut() {
            step.anchor = Some(frame);
            step.origin = origin;
        }
        self.set_aside(false, cx);
        let cutout = self.step.as_ref().and_then(GuideOverlayStep::cutout);
        self.spotlight_view.spotlight_layer.apply(cutout, false);
        self.refresh_attention(false, cx);
        self.refresh_chrome_visibility();
        cx.notify();
        if !resolves_chrome && had_anchor {
            return;
        }
        self.chrome = self.make_chrome(cx);
    }

    pub fn update_keyboard(&mut self, height: Pixels, cx: &mut Context<Self>) {
        let Some(step) = self.step.as_mut() else {
            return;
        };
        if step.keyboard_height == height {
            return;
        }
        step.keyboard_height = height;
        self.chrome = self.make_chrome(cx);
        cx.notify();
    }

    /// element is gone but the step stays
    /// unlike set_aside, an absent anchor drops only the light and the narrator keeps talking
    pub fn clear_anchor(&mut self, cx: &mut Context<Self>) {
        let has_anchor = self
            .step
            .as_ref()
            .is_some_and(|step| step.stage.requires_anchor() && step.anchor.is_some());
        if !has_anchor {
            return;
        }
        self.set_aside(false, cx);
        if let Some(step) = self.step.as_mut() {
            step.anchor = None;
        }
        self.spotlight_view.spotlight_layer.apply(None, true);
        self.refresh_attention(true, cx);
        self.refresh_chrome_visibility();
        self.chrome = self.make_chrome(cx);
        cx.notify();
    }

    pub fn narrate_aside(&mut self, cx: &mut Context<Self>) {
        if self.step.is_none() || self.is_narrating_aside {
            return;
        }
        self.set_aside(true, cx);
        self.is_narrating_aside = true;
        if let Some(step) = self.step.as_mut() {
            step.anchor = None;
        }
        self.refresh_chrome_visibility();
        self.chrome = self.make_chrome(cx);
        cx.notify();
    }

    /// steps out of the way or comes back
    pub fn set_aside(&mut self, aside: bool, cx: &mut Context<Self>) {
        tracing::warn!(target: "riseonly::X1738", aside, was_aside = self.is_aside, has_step = self.step.is_some(), "overlay set_aside");
        if self.is_narrating_aside {
            self.is_narrating_aside = false;
            if self.is_aside == aside {
                self.chrome = self.make_chrome(cx);
                cx.notify();
                return;
            }
        }
        if self.is_aside == aside || self.step.is_none() {
            return;
        }
        self.is_aside = aside;
        let animated = !aside && GuideOverlayMotion::current(cx).animates_transitions();
        let dims = self.step.as_ref().is_some_and(|step| step.stage.dims());
        self.spotlight_view.spotlight_layer.set_visible(!aside && dims, animated);
        if aside {
            self.path_view.path_layer.clear(animated);
        }
        self.refresh_attention(animated, cx);
        self.chrome = self.make_chrome(cx);
        cx.notify();
    }

    fn update_scene(&mut self, motion: GuideOverlayMotion, animated: bool, cx: &mut Context<Self>) {
        let Some(step) = self.step.clone() else {
            self.spotlight_view.spotlight_layer.set_visible(false, animated);
            self.attention_view.attention_layer.follow(None, animated);
            self.gesture_view.gesture_layer.show(None, None, false, animated);
            self.path_view.path_layer.clear(animated);
            return;
        };

        let cutout = step.cutout();
        self.spotlight_view.spotlight_layer.apply(cutout, animated);
        self.spotlight_view.spotlight_layer.set_visible(step.stage.dims(), animated);

        let (attention_color, path_color) = {
            let theme = crate::theme(cx);
            (GuideOverlayTheme::attention_color(theme), GuideOverlayTheme::path_color(theme))
        };
        self.attention_view.attention_layer.set_color(attention_color);
        self.refresh_attention(animated, cx);

        match (step.stage, cutout, step.previous_cutout) {
            (RiseGuideStage::SpotlightPath, Some(target), Some(source)) => {
                self.path_view.path_layer.draw(
                    source.resolved_rect(),
                    target.resolved_rect(),
                    path_color,
                    motion.draws_path_progressively(),
                );
            }
            _ => self.path_view.path_layer.clear(false),
        }
    }

    /// keeps the ring on the hole and fires the one ping the step is owed
    /// called from everywhere the hole changes so it stays cheap
    fn refresh_attention(&mut self, animated: bool, cx: &mut Context<Self>) {
        let cutout = if self.is_aside { None } else { self.step.as_ref().and_then(GuideOverlayStep::cutout) };
        self.attention_view.attention_layer.follow(cutout, animated);
        self.refresh_gesture(cutout, animated);

        if cutout.is_none() {
            return;
        }
        let Some(id) = self.step.as_ref().map(|step| step.id.clone()) else {
            return;
        };
        if self.pinged_step.as_ref() == Some(&id) {
            return;
        }
        self.pinged_step = Some(id);
        let pings_progressively = GuideOverlayMotion::current(cx).draws_path_progressively();
        self.attention_view.attention_layer.ping(pings_progressively);
    }

    fn refresh_gesture(&mut self, cutout: Option<GuideSpotlightCutout>, fades: bool) {
        let gesture = self.step.as_ref().and_then(GuideOverlayStep::gesture_hint);
        self.gesture_view.gesture_layer.show(gesture, cutout, self.gesture_animates, fades);
    }

    fn view_did_layout_subviews(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.bounds != bounds {
            self.bounds = bounds;
            self.spotlight_view.layout_subviews(bounds);
        }
        self.refresh_chrome_if_needed(cx);
    }

    fn refresh_chrome_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.step.is_none() {
            return;
        }
        let safe_area = Edges::default();
        if self.rendered_container == self.bounds && self.rendered_safe_area == safe_area {
            return;
        }
        self.rendered_container = self.bounds;
        self.rendered_safe_area = safe_area;
        self.chrome = self.make_chrome(cx);
    }

    fn make_chrome(&self, cx: &mut Context<Self>) -> GuideOverlayChrome {
        GuideOverlayChrome {
            step: self.step.clone(),
            is_aside: self.is_aside && !self.is_narrating_aside,
            container: self.bounds,
            safe_area: Edges::default(),
            is_right_to_left: layout_direction(cx) == LayoutDirection::RightToLeft,
            is_interactive: true,
            controller: cx.weak_entity(),
        }
    }

    fn did_report_chrome_frame(&mut self, key: &'static str, frame: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.chrome_frames.insert(key, frame);
        let draws_group = self.chrome.draws_group();
        let draws_exit = self.chrome.draws_exit();
        self.chrome_frames.retain(|key, _| match *key {
            SURFACE_FRAME_KEY => draws_group,
            EXIT_FRAME_KEY => draws_exit,
            _ => false,
        });
        let is_cramped = self.chrome.is_cramped(self.chrome_frames.get(SURFACE_FRAME_KEY).copied());
        self.chrome_rects = self.chrome_frames.values().copied().collect();
        self.is_chrome_cramped = is_cramped;
        self.refresh_chrome_visibility();
        cx.notify();
    }

    /// hole follows every frame, chrome waits for the anchor to settle
    /// so hide only the chrome until next layout clears the hole
    /// hidden not removed, it would stop reporting
    fn refresh_chrome_visibility(&mut self) {
        let overlaps = !self.is_chrome_cramped
            && self.step.as_ref().and_then(GuideOverlayStep::cutout).is_some_and(|cutout| {
                let hole = cutout.resolved_rect();
                self.chrome_rects.iter().any(|rect| rect.intersects(&hole))
            });
        if self.is_chrome_suppressed == overlaps {
            return;
        }
        self.is_chrome_suppressed = overlaps;
    }

    fn escape_action(&self) -> Option<GuideOverlayAction> {
        let step = self.step.as_ref()?;
        if (self.is_aside && !self.is_narrating_aside) || !self.is_interaction_enabled {
            return None;
        }
        step.exit.clone()
    }
}

impl Render for GuideOverlayRootController {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), window.viewport_size());
        self.view_did_layout_subviews(bounds, cx);

        let fade = self.chrome_fade.frame_value(RiseClock::monotonic(), window);
        let suppressed = self.is_chrome_suppressed;
        let mut chrome = self.chrome.clone();
        chrome.is_interactive = self.is_interaction_enabled && !suppressed;
        let escape = self.escape_action();
        let layers = cx.weak_entity();

        div()
            .relative()
            .w(bounds.size.width)
            .h(bounds.size.height)
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, cx| {
                        if let Some(entity) = layers.upgrade() {
                            let controller = entity.read(cx);
                            controller.spotlight_view.spotlight_layer.paint(window);
                            controller.attention_view.attention_layer.paint(bounds, window);
                            controller.gesture_view.gesture_layer.paint(bounds, window);
                            controller.path_view.path_layer.paint(window);
                            let panel = HoldContextMenuHitTargets::live_actions_panel();
                            let dialog = controller
                                .step
                                .as_ref()
                                .and_then(GuideOverlayStep::cutout)
                                .and_then(|cutout| ModalSheetStack::surface_containing(cutout.resolved_rect()));
                            if controller.seen_menu_panel != panel || controller.seen_dialog != dialog {
                                let entity = entity.clone();
                                window.defer(cx, move |_, cx| {
                                    entity.update(cx, |controller, cx| {
                                        if controller.seen_menu_panel != panel || controller.seen_dialog != dialog {
                                            controller.seen_menu_panel = panel;
                                            controller.seen_dialog = dialog;
                                            cx.notify();
                                        }
                                    });
                                });
                            }
                        }
                        if let Some(exit) = escape {
                            window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
                                if phase == DispatchPhase::Capture && event.keystroke.key == "escape" {
                                    cx.stop_propagation();
                                    (exit.handler)(window, cx);
                                }
                            });
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(fade)
                    .when(suppressed, |layer| layer.invisible())
                    .child(chrome),
            )
    }
}

/// narrator, surface and exit all values and no store reads
/// group is anchored to the band edge nearest the cutout so the view never measures itself
#[derive(IntoElement, Clone)]
pub struct GuideOverlayChrome {
    step: Option<GuideOverlayStep>,
    is_aside: bool,
    container: Bounds<Pixels>,
    safe_area: Edges<Pixels>,
    is_right_to_left: bool,
    is_interactive: bool,
    controller: WeakEntity<GuideOverlayRootController>,
}

impl GuideOverlayChrome {
    fn empty(controller: WeakEntity<GuideOverlayRootController>, container: Bounds<Pixels>) -> Self {
        Self {
            step: None,
            is_aside: false,
            container,
            safe_area: Edges::default(),
            is_right_to_left: false,
            is_interactive: true,
            controller,
        }
    }

    fn presented_step(&self) -> Option<GuideOverlayStep> {
        self.step.as_ref().map(|step| self.presented(step))
    }

    fn draws_group(&self) -> bool {
        self.step.is_some() && !self.is_aside
    }

    fn draws_exit(&self) -> bool {
        self.presented_step().is_some_and(|step| step.draws_corner_exit(self.is_aside))
    }

    /// only a step lit from a menu counts as overlap
    /// elsewhere the bubble steps aside and leaves the field its presses
    fn is_cramped(&self, surface: Option<Bounds<Pixels>>) -> bool {
        if self.is_aside {
            return false;
        }
        let Some(step) = self.presented_step() else {
            return false;
        };
        let solved = self.solve(&step);
        (step.origin.is_some() && solved.is_cramped) || Self::overflows_band(surface, solved.band)
    }

    /// a step lit from a menu folds to one line when the full group finds no band beside it
    /// else drawn as authored
    fn presented(&self, step: &GuideOverlayStep) -> GuideOverlayStep {
        if self.is_aside || step.origin.is_none() || !self.solve(step).is_cramped {
            return step.clone();
        }
        step.folded()
    }

    fn solver_insets(&self, step: &GuideOverlayStep) -> Edges<Pixels> {
        let mut insets = self.safe_area;
        if step.draws_corner_exit(self.is_aside) {
            insets.bottom = px(insets.bottom.as_f32()
                + GuideOverlayTheme::EXIT_HEIGHT
                + GuideOverlayTheme::LAYOUT.screen_margin.as_f32());
        }
        insets
    }

    fn group_column(&self, step: &GuideOverlayStep, band: Bounds<Pixels>) -> Bounds<Pixels> {
        let region = GuideOverlayLayout::content_region(
            self.container,
            self.solver_insets(step),
            step.keyboard_height,
            GuideOverlayTheme::LAYOUT,
        );
        let top = region.top().as_f32().min(band.top().as_f32());
        let bottom = region.bottom().as_f32().max(band.bottom().as_f32());
        Bounds::new(
            point(band.left(), px(top)),
            gpui::size(band.size.width, px(bottom - top)),
        )
    }

    fn overflows_band(surface: Option<Bounds<Pixels>>, band: Bounds<Pixels>) -> bool {
        let Some(surface) = surface else {
            return false;
        };
        if surface.size.width <= px(0.0) || surface.size.height <= px(0.0) {
            return false;
        }
        surface.top().as_f32() < band.top().as_f32() - 0.5 || surface.bottom().as_f32() > band.bottom().as_f32() + 0.5
    }

    fn solve(&self, step: &GuideOverlayStep) -> GuideOverlayFrames {
        let cutout = step.cutout();
        let menu = cutout.and_then(|cutout| {
            let rect = cutout.resolved_rect();
            HoldContextMenuHitTargets::lifted_actions_frame(rect)
                .or_else(|| HoldContextMenuHitTargets::actions_frame_beside(rect, px(MENU_BESIDE_GAP)))
        });
        let dialog = cutout.and_then(|cutout| ModalSheetStack::surface_containing(cutout.resolved_rect()));
        let composer = cutout.and_then(|cutout| Self::composer_beside(cutout.resolved_rect()));
        let kept = match (menu, composer) {
            (Some(menu), Some(composer)) => Some(menu.union(&composer)),
            (menu, composer) => menu.or(composer),
        };
        if let Some(dialog) = dialog {
            let frames = self.solve_around(step, Some(kept.map_or(dialog, |kept| kept.union(&dialog))));
            if !frames.is_cramped {
                return frames;
            }
        }
        if composer.is_some() {
            let frames = self.solve_around(step, kept);
            if !frames.is_cramped {
                return frames;
            }
        }
        self.solve_around(step, menu)
    }

    fn composer_beside(rect: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        let composer = GuideAnchorRegistry::frame(RiseGuideAnchor::ChatComposerInput)?;
        if composer.size.width <= px(0.0) || composer.size.height <= px(0.0) {
            return None;
        }
        let centre = rect.center();
        let in_row = centre.y >= composer.top() && centre.y <= composer.bottom() + px(0.5);
        let gap = px(COMPOSER_BESIDE_GAP);
        let beside = rect.left() <= composer.right() + gap && rect.right() >= composer.left() - gap;
        (in_row && beside).then_some(composer)
    }

    fn solve_around(&self, step: &GuideOverlayStep, keep_clear: Option<Bounds<Pixels>>) -> GuideOverlayFrames {
        let cutout = step.cutout();
        GuideOverlayLayout::solve(GuideOverlaySolveInput {
            container: self.container,
            safe_area: self.solver_insets(step),
            keyboard_height: step.keyboard_height,
            cutout: cutout.map(|cutout| cutout.resolved_rect()),
            origin: step.origin,
            keep_clear,
            placement: step.placement,
            prefers_centred: step.surface.is_major(),
            narrator_side: px(if step.shows_narrator() { GuideOverlayTheme::NARRATOR_SIDE } else { 0.0 }),
            surface_width_limit: px(step.surface.width_limit()),
            is_right_to_left: self.is_right_to_left,
            metrics: GuideOverlayTheme::LAYOUT,
        })
    }

    fn frame_probe(&self, key: &'static str) -> impl IntoElement {
        let controller = self.controller.clone();
        measure(key, move |frame, _, cx| {
            controller
                .update(cx, |controller, cx| controller.did_report_chrome_frame(key, frame, cx))
                .ok();
        })
    }

    fn group(&self, step: &GuideOverlayStep, frames: &GuideOverlayFrames, motion: GuideOverlayMotion) -> AnyElement {
        let narrator_side = if step.shows_narrator() { GuideOverlayTheme::NARRATOR_SIDE } else { 0.0 };
        let tail = if self.is_right_to_left { GuideBubbleTailSide::Right } else { GuideBubbleTailSide::Left };
        let narrator = div()
            .flex_shrink_0()
            .w(px(narrator_side))
            .h(px(narrator_side))
            .child(GuideNarratorView::new(step.narrator_pose(), step.shows_narrator(), motion));
        let surface = div()
            .relative()
            .when(self.is_interactive, |surface| surface.occlude())
            .child(GuideSurfaceView::new(
                step.surface.clone(),
                tail,
                self.is_right_to_left,
                frames.surface_width.as_f32(),
            ))
            .child(self.frame_probe(SURFACE_FRAME_KEY));
        div()
            .flex()
            .items_end()
            .gap(px(GuideOverlayTheme::GROUP_SPACING))
            .when(self.is_right_to_left, |row| row.flex_row_reverse())
            .when(!self.is_right_to_left, |row| row.flex_row())
            .child(narrator)
            .child(surface)
            .into_any_element()
    }

    fn group_column_element(
        &self,
        step: &GuideOverlayStep,
        frames: &GuideOverlayFrames,
        column: Bounds<Pixels>,
        motion: GuideOverlayMotion,
    ) -> AnyElement {
        let alignment = frames.group_alignment();
        let band_top = (frames.band.top().as_f32() - column.top().as_f32()).max(0.0);
        let band_height = frames.band.size.height.as_f32();
        let below_band = (column.size.height.as_f32() - band_top - band_height).max(0.0);
        let horizontal = match (alignment.horizontal(), self.is_right_to_left) {
            (HorizontalAlignment::Leading, false) | (HorizontalAlignment::Trailing, true) => HorizontalAlignment::Leading,
            (HorizontalAlignment::Trailing, false) | (HorizontalAlignment::Leading, true) => HorizontalAlignment::Trailing,
            (HorizontalAlignment::Center, _) => HorizontalAlignment::Center,
        };
        let group = div()
            .flex()
            .flex_row()
            .flex_shrink_0()
            .max_h_full()
            .w(frames.band.size.width)
            .map(|row| match horizontal {
                HorizontalAlignment::Leading => row.justify_start(),
                HorizontalAlignment::Center => row.justify_center(),
                HorizontalAlignment::Trailing => row.justify_end(),
            })
            .child(self.group(step, frames, motion));
        let spacer = |height: f32| div().flex_shrink_1().h(px(height));

        let column_element = div().flex().flex_col().w(column.size.width).h(column.size.height);
        let column_element = match alignment.vertical() {
            VerticalAlignment::Top => column_element
                .child(spacer(band_top))
                .child(group)
                .child(div().flex_grow_1()),
            VerticalAlignment::Bottom => column_element
                .child(div().flex_grow_1())
                .child(group)
                .child(spacer(below_band)),
            VerticalAlignment::Center | VerticalAlignment::FirstTextBaseline => column_element
                .child(spacer(band_top))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .min_h(px(band_height))
                        .flex_shrink_0()
                        .child(group),
                )
                .child(div().flex_grow_1()),
        };
        column_element.into_any_element()
    }

    fn exit_control(&self, exit: &GuideOverlayAction, step: &GuideOverlayStep, cx: &App) -> AnyElement {
        let theme = crate::theme(cx);
        let margin = GuideOverlayTheme::LAYOUT.screen_margin.as_f32();
        let bottom = self.safe_area.bottom.as_f32().max(step.keyboard_height.as_f32()) + margin;
        let leading = if self.is_right_to_left { self.safe_area.right } else { self.safe_area.left }.as_f32() + margin;
        let handler = exit.handler.clone();
        let button = SimpleButtonUi::new("guide-overlay-exit")
            .height(GuideOverlayTheme::EXIT_HEIGHT)
            .on_press(move |window, cx| handler(window, cx))
            .child(
                div().px(px(GuideOverlayTheme::EXIT_HORIZONTAL_PADDING)).child(
                    MainText::new(exit.title.clone())
                        .px(GuideOverlayTheme::EXIT_TITLE_SIZE)
                        .color(GuideOverlayTheme::exit_title_color(theme)),
                ),
            );
        div()
            .absolute()
            .bottom(px(bottom))
            .when(self.is_right_to_left, |corner| corner.right(px(leading)))
            .when(!self.is_right_to_left, |corner| corner.left(px(leading)))
            .when(self.is_interactive, |corner| corner.occlude())
            .child(button)
            .child(self.frame_probe(EXIT_FRAME_KEY))
            .into_any_element()
    }
}

impl RenderOnce for GuideOverlayChrome {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let motion = GuideOverlayMotion::conserving_energy(reduce_motion(cx), cx);
        let mut root = div()
            .relative()
            .w(self.container.size.width)
            .h(self.container.size.height);

        let Some(step) = self.presented_step() else {
            return root;
        };

        if !self.is_aside {
            let frames = self.solve(&step);
            let target = self.group_column(&step, frames.band);
            let spec = if step.keyboard_height > px(0.0) { None } else { GuideOverlayTheme::move_animation(motion) };
            let column_motion =
                window.use_keyed_state("guide-overlay-column", cx, |_, _| Animated::new(target));
            let now = RiseClock::monotonic();
            column_motion.update(cx, |column, _| column.set(target, spec, now, false));
            let column = column_motion.read(cx).frame_value(now, window);

            root = root.child(
                div()
                    .absolute()
                    .left(column.left())
                    .top(column.top())
                    .child(self.group_column_element(&step, &frames, column, motion)),
            );
        }

        if let Some(exit) = step.exit.as_ref().filter(|_| step.draws_corner_exit(self.is_aside)) {
            root = root.child(self.exit_control(exit, &step, cx));
        }
        root
    }
}

/// owns the overlay, creation teardown and layer level follow the hold menu host
/// a guide survives losing focus for background_abort_ms, a memory warning drops the overlay
pub struct GuideOverlayHost {
    on_interrupted: Option<Rc<dyn Fn(&mut App)>>,
    window: Option<AnyWindowHandle>,
    controller: Option<Entity<GuideOverlayRootController>>,
    presents_step: bool,
    level: usize,
    _observers: Vec<Subscription>,
    _memory_warning_observer: Option<RiseSubscription>,
    _memory_warning_task: Option<Task<()>>,
}

impl Global for GuideOverlayHost {}

thread_local! {
    static HOSTING_WINDOWS: RefCell<Vec<AnyWindowHandle>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    static IDLE_HOST: &'static GuideOverlayHost = Box::leak(Box::new(GuideOverlayHost::idle(Vec::new())));
}

impl GuideOverlayHost {
    pub const BASE_LEVEL: usize = 2008;

    pub const ABOVE_HOLD_MENU_LEVEL: usize = 2011;

    fn idle(observers: Vec<Subscription>) -> Self {
        Self {
            on_interrupted: None,
            window: None,
            controller: None,
            presents_step: false,
            level: Self::BASE_LEVEL,
            _observers: observers,
            _memory_warning_observer: None,
            _memory_warning_task: None,
        }
    }

    pub fn front_hosting_window(cx: &App) -> Option<AnyWindowHandle> {
        let open = cx.windows();
        let hosts = HOSTING_WINDOWS.with(|hosts| {
            let mut hosts = hosts.borrow_mut();
            hosts.retain(|host| open.contains(host));
            hosts.clone()
        });
        let order = cx
            .window_stack()
            .unwrap_or_else(|| cx.active_window().into_iter().chain(open.iter().copied()).collect());
        order.into_iter().find(|window| hosts.contains(window))
    }

    pub fn shared(cx: &App) -> &GuideOverlayHost {
        match cx.try_global::<GuideOverlayHost>() {
            Some(host) => host,
            None => IDLE_HOST.with(|host| *host),
        }
    }

    fn ensure(cx: &mut App) {
        if cx.has_global::<GuideOverlayHost>() {
            return;
        }
        let closed = cx.on_window_closed(|cx, window_id| Self::scene_did_disconnect(window_id, cx));
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let observer = RiseAppEvents::default_center()
            .observe(UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(());
            });
        let task = cx.spawn(async move |cx| {
            while inbox.next().await.is_some() {
                cx.update(Self::release_for_memory_pressure);
            }
        });
        let mut host = Self::idle(vec![closed]);
        host._memory_warning_observer = Some(observer);
        host._memory_warning_task = Some(task);
        cx.set_global(host);
    }

    fn host_mut(cx: &mut App) -> &mut GuideOverlayHost {
        Self::ensure(cx);
        cx.global_mut::<GuideOverlayHost>()
    }

    pub fn set_on_interrupted(on_interrupted: impl Fn(&mut App) + 'static, cx: &mut App) {
        Self::host_mut(cx).on_interrupted = Some(Rc::new(on_interrupted));
    }

    pub fn is_presenting(&self) -> bool {
        self.presents_step
    }

    /// window a native dialog goes over while a step runs, modal to that window
    /// above dim and narrator, none when presenting would be refused
    pub fn alert_host(&self) -> Option<AnyWindowHandle> {
        self.controller.as_ref()?;
        self.window
    }

    fn controller(cx: &App) -> Option<Entity<GuideOverlayRootController>> {
        Self::shared(cx).controller.clone()
    }

    /// puts a step on screen or moves the running guide onto it
    /// window comes from the anchored element, never a key window scan
    /// above_window_level is the window holding the step anchor
    pub fn present(
        step: GuideOverlayStep,
        scene: Option<AnyWindowHandle>,
        above_window_level: Option<AnyWindowHandle>,
        cx: &mut App,
    ) {
        let Some(scene) = above_window_level.or(scene) else {
            return;
        };
        Self::show_window(scene, cx);
        Self::apply_level(above_window_level, cx);
        let Some(controller) = Self::controller(cx) else {
            return;
        };
        controller.update(cx, |controller, cx| {
            controller.set_interaction_enabled(true, cx);
            controller.apply(Some(step), true, cx);
        });
        Self::host_mut(cx).presents_step = true;
    }

    /// re applies the level mid step for an anchor that showed up late
    pub fn raise(above_anchor_level: Option<AnyWindowHandle>, cx: &mut App) {
        Self::apply_level(above_anchor_level, cx);
    }

    fn apply_level(above_anchor_level: Option<AnyWindowHandle>, cx: &mut App) {
        let host = Self::shared(cx);
        let Some(current) = host.window else {
            return;
        };
        if let Some(target) = above_anchor_level
            && target != current
        {
            let presents = host.presents_step;
            let step = host.controller.as_ref().and_then(|controller| controller.read(cx).step().cloned());
            Self::show_window(target, cx);
            if let (Some(controller), Some(step)) = (Self::controller(cx), step) {
                controller.update(cx, |controller, cx| {
                    controller.set_interaction_enabled(true, cx);
                    controller.apply(Some(step), false, cx);
                });
            }
            Self::host_mut(cx).presents_step = presents;
        }
        Self::host_mut(cx).level = Self::BASE_LEVEL;
    }

    /// follows the element the running step points at, see GuideAnchorTracker
    pub fn move_anchor(frame: Bounds<Pixels>, resolves_chrome: bool, origin: Option<Bounds<Pixels>>, cx: &mut App) {
        if let Some(controller) = Self::controller(cx) {
            controller.update(cx, |controller, cx| controller.move_anchor(frame, resolves_chrome, origin, cx));
        }
    }

    /// takes the light off screen without ending the step and puts it back
    pub fn set_aside(aside: bool, cx: &mut App) {
        if let Some(controller) = Self::controller(cx) {
            controller.update(cx, |controller, cx| controller.set_aside(aside, cx));
        }
    }

    pub fn update_keyboard(height: Pixels, cx: &mut App) {
        if let Some(controller) = Self::controller(cx) {
            controller.update(cx, |controller, cx| controller.update_keyboard(height, cx));
        }
    }

    pub fn narrate_aside(cx: &mut App) {
        if let Some(controller) = Self::controller(cx) {
            controller.update(cx, |controller, cx| controller.narrate_aside(cx));
        }
    }

    /// keeps the step and narrator but drops the hole
    pub fn clear_anchor(cx: &mut App) {
        if let Some(controller) = Self::controller(cx) {
            controller.update(cx, |controller, cx| controller.clear_anchor(cx));
        }
    }

    pub fn dismiss(cx: &mut App) {
        if let Some(controller) = Self::controller(cx) {
            controller.update(cx, |controller, cx| controller.apply(None, true, cx));
        }
        Self::host_mut(cx).presents_step = false;
        Self::hide(cx);
    }

    fn show_window(scene: AnyWindowHandle, cx: &mut App) {
        let host = Self::shared(cx);
        if host.window == Some(scene) && host.controller.is_some() {
            if let Some(controller) = host.controller.clone() {
                controller.update(cx, |controller, cx| controller.set_interaction_enabled(true, cx));
            }
            return;
        }

        Self::release_window(cx);
        let created = scene.update(cx, |_, window, cx| {
            let controller = cx.new(|cx| GuideOverlayRootController::new(window, cx));
            window.refresh();
            controller
        });
        let Ok(controller) = created else {
            return;
        };
        let host = Self::host_mut(cx);
        host.level = Self::BASE_LEVEL;
        host.window = Some(scene);
        host.controller = Some(controller);
    }

    /// hides the overlay but keeps its state attached
    /// with no step nothing claims a press so presses pass through
    fn hide(cx: &mut App) {
        if let Some(controller) = Self::controller(cx) {
            controller.update(cx, |controller, cx| controller.set_interaction_enabled(false, cx));
        }
    }

    fn scene_will_deactivate(scene: AnyWindowHandle, cx: &mut App) {
        let host = Self::shared(cx);
        if host.window != Some(scene) || host.is_presenting() {
            return;
        }
        Self::hide(cx);
    }

    fn scene_did_disconnect(window_id: WindowId, cx: &mut App) {
        let host = Self::shared(cx);
        if host.window.map(|window| window.window_id()) != Some(window_id) {
            return;
        }
        let was_presenting = host.is_presenting();
        let step = host.controller.as_ref().and_then(|controller| controller.read(cx).step().cloned());
        if let Some(controller) = Self::controller(cx) {
            controller.update(cx, |controller, cx| controller.apply(None, false, cx));
        }
        Self::host_mut(cx).presents_step = false;
        Self::release_window(cx);
        if !was_presenting {
            return;
        }
        if let Some(step) = step
            && let Some(scene) = Self::front_hosting_window_except(window_id, cx)
        {
            Self::show_window(scene, cx);
            if let Some(controller) = Self::controller(cx) {
                let had_cutout = step.cutout().is_some();
                controller.update(cx, |controller, cx| {
                    controller.set_interaction_enabled(true, cx);
                    controller.apply(Some(step), false, cx);
                    if had_cutout {
                        controller.clear_anchor(cx);
                    }
                });
                Self::host_mut(cx).presents_step = true;
                return;
            }
        }
        if let Some(on_interrupted) = Self::shared(cx).on_interrupted.clone() {
            on_interrupted(cx);
        }
    }

    fn front_hosting_window_except(closing: WindowId, cx: &App) -> Option<AnyWindowHandle> {
        let open: Vec<AnyWindowHandle> =
            cx.windows().into_iter().filter(|window| window.window_id() != closing).collect();
        let hosts: Vec<AnyWindowHandle> = HOSTING_WINDOWS
            .with(|hosts| hosts.borrow().iter().copied().filter(|host| open.contains(host)).collect());
        let order = cx.window_stack().unwrap_or_else(|| open.clone());
        order
            .into_iter()
            .find(|window| hosts.contains(window))
            .or_else(|| hosts.first().copied())
    }

    fn release_for_memory_pressure(cx: &mut App) {
        if Self::shared(cx).is_presenting() {
            return;
        }
        Self::release_window(cx);
    }

    fn release_window(cx: &mut App) {
        let host = Self::host_mut(cx);
        let window = host.window.take();
        host.controller = None;
        host.presents_step = false;
        if let Some(window) = window {
            let _ = window.update(cx, |_, window, _| window.refresh());
        }
    }
}
