use std::any::Any;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, Bounds, Context, CursorStyle, DispatchPhase, Entity, FocusHandle, FontWeight, Hitbox, HitboxBehavior, Hsla,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PinchEvent, Pixels, Point, ScrollWheelEvent,
    Task, TouchPhase, WeakEntity, Window, black, bounds, canvas, point, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::motion::{MotionSpec, ease_in, ease_in_out, ease_out, spring};
use rise_ui::swiftui::prelude::*;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::modules::media_picker::editor::editor_crop_auto_fit::{
    EDITOR_CROP_AUTO_FIT, EDITOR_CROP_AUTO_FIT_CANCEL, EDITOR_CROP_AUTO_FIT_DID_FINISH,
    EDITOR_CROP_AUTO_FIT_PRESENTATION_DID_CHANGE, EDITOR_CROP_FINE_ROTATION_DID_FINISH,
    EDITOR_CROP_FINE_ROTATION_INTERACTION, EDITOR_CROP_FRAME_IN_STAGE, EDITOR_CROP_GEOMETRY_DID_CHANGE,
    EDITOR_CROP_GEOMETRY_REQUEST, EDITOR_CROP_IMAGE_TRANSFORM_DID_CHANGE, EDITOR_CROP_INTERACTION_CHANGED,
    EDITOR_CROP_ORIENTATION_CHANGE, EDITOR_CROP_ORIENTATION_DID_FINISH, EDITOR_CROP_PINCH, EDITOR_CROP_RESET,
    EditorCropAutoFit, EditorCropAutoFitCancel, EditorCropAutoFitCompletion, EditorCropAutoFitPlanner,
    EditorCropAutoFitPresentationUpdate, EditorCropAutoFitPresentationUpdatePhase, EditorCropAutoFitRequest,
    EditorCropFineRotationCompletion, EditorCropFineRotationInteractionUpdate, EditorCropFrameUpdate,
    EditorCropGeometryRequest, EditorCropGeometryUpdate, EditorCropInteractionUpdate, EditorCropOrientationCompletion,
    EditorCropOrientationRequest, EditorCropPinchUpdate, EditorCropPinchUpdatePhase, EditorCropResetRequest,
};
use crate::modules::media_picker::editor::editor_crop_auto_fit_log::EditorCropAutoFitLog;
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use crate::modules::media_picker::editor::editor_image_layout::{EditorImageLayout, editor_crop_chrome_height_reader};
use crate::modules::media_picker::editor::editor_image_rect::EditorImageRect;
use crate::modules::media_picker::editor::editor_rotation_dial::{self, EditorRotationDial};
use crate::modules::media_picker::editor::editor_state::{
    EditorCropAspect, EditorImageState, EditorOverlayCoordinateSpace,
};

pub type EditorCropAction = Rc<dyn Fn(&mut Window, &mut App)>;
pub type EditorCropRectCallback = Rc<dyn Fn(Bounds<f32>, &mut Window, &mut App)>;
pub type EditorCropChromeHeightCallback = Rc<dyn Fn(f32, &mut Window, &mut App)>;

const FINE_ROTATION_WHEEL_IDLE_SECONDS: f64 = 0.25;
const RESET_WATCHDOG_SECONDS: f64 = 1.2;
const ROTATION_DIAL_MIN_WIDTH: f32 = 150.0;
const ROTATION_DIAL_MAX_WIDTH: f32 = 480.0;
const ROTATION_DIAL_STAGE_INSET: f32 = 28.0;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<f32> {
    bounds(point(x, y), size(width, height))
}

fn min_x(r: &Bounds<f32>) -> f32 {
    r.origin.x
}

fn max_x(r: &Bounds<f32>) -> f32 {
    r.origin.x + r.size.width
}

fn min_y(r: &Bounds<f32>) -> f32 {
    r.origin.y
}

fn max_y(r: &Bounds<f32>) -> f32 {
    r.origin.y + r.size.height
}

fn mid_x(r: &Bounds<f32>) -> f32 {
    r.origin.x + r.size.width / 2.0
}

fn mid_y(r: &Bounds<f32>) -> f32 {
    r.origin.y + r.size.height / 2.0
}

fn is_empty(r: &Bounds<f32>) -> bool {
    r.size.width <= 0.0 || r.size.height <= 0.0
}

/// intersection of two rects
/// no overlap gives a zero size which every caller rejects as null
fn intersection(a: &Bounds<f32>, b: &Bounds<f32>) -> Bounds<f32> {
    let left = min_x(a).max(min_x(b));
    let top = min_y(a).max(min_y(b));
    let right = max_x(a).min(max_x(b)).max(left);
    let bottom = max_y(a).min(max_y(b)).max(top);
    rect(left, top, right - left, bottom - top)
}

fn contains(r: &Bounds<f32>, p: Point<f32>) -> bool {
    p.x >= min_x(r) && p.x < max_x(r) && p.y >= min_y(r) && p.y < max_y(r)
}

fn to_pixels(r: Bounds<f32>) -> Bounds<Pixels> {
    r.map(px)
}

fn from_pixels(r: Bounds<Pixels>) -> Bounds<f32> {
    r.map(f32::from)
}

/// one notification the panel receives, carried from the posting thread to the panel's window
#[derive(Clone, Copy)]
enum EditorCropPanelNotification {
    GeometryDidChange(EditorCropGeometryUpdate),
    AutoFitDidFinish(EditorCropAutoFitCompletion),
    AutoFitPresentationDidChange(EditorCropAutoFitPresentationUpdate),
    ImageTransformDidChange(Uuid),
    InteractionChanged(EditorCropInteractionUpdate),
    OrientationDidFinish(EditorCropOrientationCompletion),
    FineRotationDidFinish(EditorCropFineRotationCompletion),
}

fn observe_into<P: Any + Send + Sync + Copy>(
    name: RiseNotificationName,
    sender: mpsc::UnboundedSender<EditorCropPanelNotification>,
    wrap: fn(P) -> EditorCropPanelNotification,
) -> RiseSubscription {
    RiseAppEvents::default_center().observe(name, move |notification| {
        if let Some(payload) = notification.payload::<P>() {
            let _ = sender.unbounded_send(wrap(*payload));
        }
    })
}

pub struct EditorCropPanel {
    pub crop_coordination_id: Uuid,
    pub state: Binding<EditorImageState>,
    pub on_reset: EditorCropAction,
    pub on_close: EditorCropAction,
    pub on_confirm: EditorCropAction,
    on_chrome_height_change: EditorCropChromeHeightCallback,

    local_crop_rect: Bounds<f32>,
    stage_image_layout: Option<EditorImageRect>,
    auto_fit_task: Option<Task<()>>,
    is_adjusting: bool,
    is_crop_auto_fitting: bool,
    auto_fit_id: Option<Uuid>,
    rotation_steps_animating: Option<i64>,
    is_aspect_menu_presented: bool,
    state_at_entry: Option<EditorImageState>,
    orientation_change_id: Option<Uuid>,
    fine_rotation_completion_id: Option<Uuid>,
    reset_id: Option<Uuid>,
    is_image_transforming: bool,
    is_fine_rotation_adjusting: bool,
    dial_anchor: Option<Bounds<f32>>,
    is_dial_anchor_held: bool,
    fine_rotation_wheel_raw: f64,
    is_resetting_crop: bool,
    auto_fit_presentation_rect: Option<Bounds<f32>>,
    auto_fit_presentation_latest: Arc<Mutex<Option<EditorCropAutoFitPresentationUpdate>>>,

    animated_crop_rect: Animated<Bounds<Pixels>>,
    overlay_opacity: Animated<f32>,
    overlay_is_visible: bool,
    dial_rotation_opacity: Animated<f32>,
    fine_rotation_wheel_task: Option<Task<()>>,
    reset_watchdog_task: Option<Task<()>>,
    focus_handle: FocusHandle,
    _observers: Vec<RiseSubscription>,
    _notification_pump: Task<()>,
}

impl EditorCropPanel {
    /// lives as long as the panel is on screen
    /// the owner drops the entity when crop mode ends, that is the disappear hook
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        crop_coordination_id: Uuid,
        state: Binding<EditorImageState>,
        on_reset: EditorCropAction,
        on_close: EditorCropAction,
        on_confirm: EditorCropAction,
        on_chrome_height_change: EditorCropChromeHeightCallback,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<EditorCropPanelNotification>();
        let mut observers = vec![
            observe_into(
                EDITOR_CROP_GEOMETRY_DID_CHANGE,
                sender.clone(),
                EditorCropPanelNotification::GeometryDidChange,
            ),
            observe_into(
                EDITOR_CROP_AUTO_FIT_DID_FINISH,
                sender.clone(),
                EditorCropPanelNotification::AutoFitDidFinish,
            ),
            observe_into(
                EDITOR_CROP_AUTO_FIT_PRESENTATION_DID_CHANGE,
                sender.clone(),
                EditorCropPanelNotification::AutoFitPresentationDidChange,
            ),
            observe_into(
                EDITOR_CROP_IMAGE_TRANSFORM_DID_CHANGE,
                sender.clone(),
                EditorCropPanelNotification::ImageTransformDidChange,
            ),
            observe_into(
                EDITOR_CROP_INTERACTION_CHANGED,
                sender.clone(),
                EditorCropPanelNotification::InteractionChanged,
            ),
            observe_into(
                EDITOR_CROP_ORIENTATION_DID_FINISH,
                sender.clone(),
                EditorCropPanelNotification::OrientationDidFinish,
            ),
            observe_into(
                EDITOR_CROP_FINE_ROTATION_DID_FINISH,
                sender,
                EditorCropPanelNotification::FineRotationDidFinish,
            ),
        ];
        let auto_fit_presentation_latest = Arc::new(Mutex::new(None));
        let latest_sink = auto_fit_presentation_latest.clone();
        observers.push(RiseAppEvents::default_center().observe(
            EDITOR_CROP_AUTO_FIT_PRESENTATION_DID_CHANGE,
            move |notification| {
                if let Some(update) = notification.payload::<EditorCropAutoFitPresentationUpdate>()
                    && let Ok(mut latest) = latest_sink.lock()
                {
                    *latest = Some(*update);
                }
            },
        ));
        let notification_pump = cx.spawn_in(window, async move |this, cx| {
            while let Some(notification) = inbox.next().await {
                let delivered =
                    this.update_in(cx, |this, window, cx| this.handle_notification(notification, window, cx));
                if delivered.is_err() {
                    break;
                }
            }
        });

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let state_at_entry = state.get(cx);
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_GEOMETRY_REQUEST,
            EditorCropGeometryRequest { scope_id: crop_coordination_id },
        );

        Self {
            crop_coordination_id,
            state,
            on_reset,
            on_close,
            on_confirm,
            on_chrome_height_change,
            local_crop_rect: Bounds::default(),
            stage_image_layout: None,
            auto_fit_task: None,
            is_adjusting: false,
            is_crop_auto_fitting: false,
            auto_fit_id: None,
            rotation_steps_animating: None,
            is_aspect_menu_presented: false,
            state_at_entry: Some(state_at_entry),
            orientation_change_id: None,
            fine_rotation_completion_id: None,
            reset_id: None,
            is_image_transforming: false,
            is_fine_rotation_adjusting: false,
            dial_anchor: None,
            is_dial_anchor_held: false,
            fine_rotation_wheel_raw: 0.0,
            is_resetting_crop: false,
            auto_fit_presentation_rect: None,
            auto_fit_presentation_latest,
            animated_crop_rect: Animated::new(Bounds::default()),
            overlay_opacity: Animated::new(0.0),
            overlay_is_visible: false,
            dial_rotation_opacity: Animated::new(1.0),
            fine_rotation_wheel_task: None,
            reset_watchdog_task: None,
            focus_handle,
            _observers: observers,
            _notification_pump: notification_pump,
        }
    }

    fn crop_viewport_rect(&self) -> Bounds<f32> {
        self.stage_image_layout.map(|layout| layout.viewport_rect).unwrap_or_default()
    }

    fn visible_image_rect(&self) -> Bounds<f32> {
        self.stage_image_layout.map(|layout| layout.rect).unwrap_or_default()
    }

    fn crop_horizontal_slot(&self) -> Bounds<f32> {
        self.stage_image_layout.map(|layout| layout.available_rect).unwrap_or_default()
    }

    fn presented_crop_rect(&self, now: Duration) -> Bounds<f32> {
        if let Some(frame) = self.posted_auto_fit_frame() {
            return frame;
        }
        self.auto_fit_presentation_rect.unwrap_or_else(|| from_pixels(self.animated_crop_rect.value(now)))
    }

    fn posted_auto_fit_frame(&self) -> Option<Bounds<f32>> {
        let auto_fit_id = self.auto_fit_id?;
        let latest = self.auto_fit_presentation_latest.lock().ok()?;
        (*latest)
            .filter(|update| update.id == auto_fit_id && update.scope_id == self.crop_coordination_id)
            .map(|update| update.crop_frame)
    }

    fn update_state(&self, window: &mut Window, cx: &mut App, update: impl FnOnce(&mut EditorImageState)) {
        let mut state = self.state.get(cx);
        update(&mut state);
        self.state.set(state, window, cx);
    }

    fn set_local_crop_rect(&mut self, new_rect: Bounds<f32>, animation: Option<MotionSpec>, cx: &mut Context<Self>) {
        self.local_crop_rect = new_rect;
        match animation {
            Some(spec) => self.animated_crop_rect.set_in(to_pixels(new_rect), spec, RiseClock::monotonic(), cx),
            None => self.animated_crop_rect.jump(to_pixels(new_rect)),
        }
        cx.notify();
    }

    fn overlay_allows_hit_testing(&self) -> bool {
        self.rotation_steps_animating.is_none() && !self.is_resetting_crop && self.stage_image_layout.is_some()
    }

    fn sync_overlay_visibility(&mut self, animation: Option<MotionSpec>, cx: &mut Context<Self>) {
        let is_visible = self.overlay_allows_hit_testing();
        if is_visible == self.overlay_is_visible {
            return;
        }
        self.overlay_is_visible = is_visible;
        let target = if is_visible { 1.0 } else { 0.0 };
        match animation {
            Some(spec) => self.overlay_opacity.set_in(target, spec, RiseClock::monotonic(), cx),
            None => self.overlay_opacity.jump(target),
        }
    }

    fn sync_dial_rotation_opacity(&mut self, cx: &mut Context<Self>) {
        let target = if self.rotation_steps_animating.is_some() { 0.0 } else { 1.0 };
        self.dial_rotation_opacity.set_in(target, ease_in_out(0.18), RiseClock::monotonic(), cx);
    }

    fn handle_notification(
        &mut self,
        notification: EditorCropPanelNotification,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match notification {
            EditorCropPanelNotification::GeometryDidChange(update) => {
                if update.scope_id != self.crop_coordination_id {
                    return;
                }
                if update.completed_reset_id.is_none()
                    && self.stage_image_layout == Some(update.image_layout)
                    && self.local_crop_rect == update.crop_frame
                {
                    return;
                }
                self.stage_image_layout = Some(update.image_layout);
                self.set_local_crop_rect(update.crop_frame, None, cx);
                if update.completed_reset_id.is_some() && update.completed_reset_id == self.reset_id {
                    self.reset_id = None;
                    self.is_resetting_crop = false;
                }
                self.sync_overlay_visibility(None, cx);
            }
            EditorCropPanelNotification::AutoFitDidFinish(completion) => {
                if completion.scope_id != self.crop_coordination_id || Some(completion.id) != self.auto_fit_id {
                    return;
                }
                self.is_crop_auto_fitting = false;
                self.auto_fit_id = None;
                self.auto_fit_presentation_rect = None;
            }
            EditorCropPanelNotification::AutoFitPresentationDidChange(update) => {
                if update.scope_id != self.crop_coordination_id
                    || Some(update.id) != self.auto_fit_id
                    || update.phase == EditorCropAutoFitPresentationUpdatePhase::Ended
                {
                    return;
                }
                self.auto_fit_presentation_rect = Some(update.crop_frame);
            }
            EditorCropPanelNotification::ImageTransformDidChange(scope_id) => {
                if scope_id != self.crop_coordination_id
                    || self.stage_image_layout.is_none()
                    || self.is_adjusting
                    || self.is_crop_auto_fitting
                    || self.is_resetting_crop
                    || self.is_image_transforming
                {
                    return;
                }
                self.sync_crop_relative_from_visible(None, window, cx);
            }
            EditorCropPanelNotification::InteractionChanged(update) => {
                if update.scope_id != self.crop_coordination_id {
                    return;
                }
                self.is_image_transforming = update.is_active;
            }
            EditorCropPanelNotification::OrientationDidFinish(completion) => {
                if completion.scope_id != self.crop_coordination_id || Some(completion.id) != self.orientation_change_id
                {
                    return;
                }
                self.orientation_change_id = None;
                self.rotation_steps_animating = None;
                self.sync_overlay_visibility(Some(ease_in(0.16)), cx);
                self.sync_dial_rotation_opacity(cx);
            }
            EditorCropPanelNotification::FineRotationDidFinish(completion) => {
                if completion.scope_id != self.crop_coordination_id
                    || Some(completion.id) != self.fine_rotation_completion_id
                {
                    return;
                }
                self.fine_rotation_completion_id = None;
                self.sync_crop_relative_from_visible(None, window, cx);
                self.schedule_auto_zoom(window, cx);
            }
        }
        cx.notify();
    }

    fn restore_crop_from_relative(&mut self, animated: bool, cx: &mut Context<Self>) {
        let state = self.state.get(cx);
        let is_full_frame = EditorCropAutoFitPlanner::is_effectively_full(state.crop_rect_relative);
        let viewport = self.crop_viewport_rect();
        let base_viewport_is_valid = viewport.size.width > 1.0 && viewport.size.height > 1.0;
        let bounds = if is_full_frame && base_viewport_is_valid { viewport } else { self.visible_image_rect() };
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            return;
        }
        let rel = state.crop_rect_relative.unwrap_or_else(|| rect(0.0, 0.0, 1.0, 1.0));
        let absolute = rect(
            min_x(&bounds) + rel.origin.x * bounds.size.width,
            min_y(&bounds) + rel.origin.y * bounds.size.height,
            rel.size.width * bounds.size.width,
            rel.size.height * bounds.size.height,
        );
        let clamped = self.clamp_crop_to_image_bounds(absolute);
        self.set_local_crop_rect(clamped, animated.then(|| spring(0.38, 0.84)), cx);
        self.publish_crop_frame_in_stage(None);
    }

    fn crop_grid_mode(&self) -> EditorCropGridMode {
        if self.is_fine_rotation_adjusting {
            return EditorCropGridMode::Minor;
        }
        if self.is_adjusting || self.is_image_transforming {
            return EditorCropGridMode::Major;
        }
        EditorCropGridMode::None
    }

    fn publish_crop_frame_in_stage(&self, explicit_rect: Option<Bounds<f32>>) {
        let rect = explicit_rect.unwrap_or(self.local_crop_rect);
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            return;
        }
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_FRAME_IN_STAGE,
            EditorCropFrameUpdate { scope_id: self.crop_coordination_id, rect },
        );
    }

    fn clamp_crop_to_image_bounds(&self, rect: Bounds<f32>) -> Bounds<f32> {
        let bounds = self.crop_clamp_bounds();
        if bounds.size.width <= EditorCropAutoFit::MIN_CROP_EXTENT
            || bounds.size.height <= EditorCropAutoFit::MIN_CROP_EXTENT
        {
            return rect;
        }
        EditorCropOverlayUIView::clamp(rect, bounds, EditorCropOverlayUIView::DEFAULT_CLAMP_MIN_SIDE)
    }

    fn crop_clamp_bounds(&self) -> Bounds<f32> {
        EditorCropOverlayUIView::crop_clamp_bounds(
            self.crop_viewport_rect(),
            self.visible_image_rect(),
            self.crop_horizontal_slot(),
        )
    }

    fn locked_aspect_ratio(&self, state: &EditorImageState) -> Option<f32> {
        let mut source = state.source_pixel_size;
        if state.rotation_steps % 2 != 0 {
            std::mem::swap(&mut source.width, &mut source.height);
        }
        let visible = self.visible_image_rect();
        let original_ratio = if source.width > 0.0 && source.height > 0.0 {
            source.width / source.height
        } else if visible.size.width > 0.0 && visible.size.height > 0.0 {
            visible.size.width / visible.size.height
        } else {
            return None;
        };
        state.aspect.ratio(original_ratio)
    }

    fn sync_crop_relative_from_visible(
        &self,
        crop_screen: Option<Bounds<f32>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let crop = self.clamp_crop_to_image_bounds(crop_screen.unwrap_or(self.local_crop_rect));
        let visible = self.visible_image_rect();
        if visible.size.width <= 1.0 || visible.size.height <= 1.0 {
            return;
        }
        let mut state = self.state.get(cx);
        let relative = Self::clamp_relative(Self::relative(crop, visible), &state);
        if state.crop_rect_relative.is_some_and(|current| Self::nearly_same_relative(current, relative)) {
            return;
        }
        state.crop_rect_relative = Some(relative);
        self.state.set(state, window, cx);
    }

    fn nearly_same_relative(a: Bounds<f32>, b: Bounds<f32>) -> bool {
        const EPSILON: f32 = 1e-4;
        (a.origin.x - b.origin.x).abs() <= EPSILON
            && (a.origin.y - b.origin.y).abs() <= EPSILON
            && (a.size.width - b.size.width).abs() <= EPSILON
            && (a.size.height - b.size.height).abs() <= EPSILON
    }

    fn clamp_relative(relative: Bounds<f32>, state: &EditorImageState) -> Bounds<f32> {
        let mut r = relative;
        let oriented_pixels = EditorImageLayout::oriented_display_size(
            state.source_pixel_size,
            state.rotation_steps,
            state.free_rotation_degrees,
        );
        let minimum_width = 1.0 / oriented_pixels.width.max(1.0);
        let minimum_height = 1.0 / oriented_pixels.height.max(1.0);
        r.origin.x = r.origin.x.min(1.0 - minimum_width).max(0.0);
        r.origin.y = r.origin.y.min(1.0 - minimum_height).max(0.0);
        r.size.width = r.size.width.min(1.0 - r.origin.x).max(minimum_width);
        r.size.height = r.size.height.min(1.0 - r.origin.y).max(minimum_height);
        r
    }

    fn is_rotation_in_flight(&self) -> bool {
        self.is_fine_rotation_adjusting
            || self.fine_rotation_wheel_task.is_some()
            || self.fine_rotation_completion_id.is_some()
            || self.rotation_steps_animating.is_some()
            || self.orientation_change_id.is_some()
    }

    fn drawn_crop_rect(&self, presented_crop_rect: Bounds<f32>) -> Bounds<f32> {
        if self.posted_auto_fit_frame().is_some() || self.auto_fit_presentation_rect.is_some() {
            return presented_crop_rect;
        }
        let bounds = if self.is_crop_auto_fitting { self.crop_horizontal_slot() } else { self.crop_clamp_bounds() };
        EditorCropOverlayUIView::resolve_crop_rect(presented_crop_rect, bounds)
    }

    /// invariant: the dial hangs under the crop frame it saw before the turn began and is not moved by any turn
    fn refresh_dial_anchor(&mut self, presented_crop_rect: Bounds<f32>) -> Bounds<f32> {
        if self.is_dial_anchor_held && !self.is_rotation_in_flight() {
            self.is_dial_anchor_held = false;
        }
        let presented_crop_rect = self.drawn_crop_rect(presented_crop_rect);
        if !is_empty(&presented_crop_rect) && (self.dial_anchor.is_none() || !self.is_dial_anchor_held) {
            self.dial_anchor = Some(presented_crop_rect);
        }
        self.dial_anchor.unwrap_or(presented_crop_rect)
    }

    /// dial as wide as the crop frame it hangs under, so it shrinks and grows with the window
    fn rotation_dial_width(&self, frame_width: f32) -> f32 {
        let wanted = frame_width.clamp(ROTATION_DIAL_MIN_WIDTH, ROTATION_DIAL_MAX_WIDTH);
        let stage_width = self.stage_image_layout.map(|layout| layout.stage_rect.size.width).unwrap_or(0.0);
        if stage_width > ROTATION_DIAL_STAGE_INSET {
            wanted.min(stage_width - ROTATION_DIAL_STAGE_INSET)
        } else {
            wanted
        }
    }

    fn dial_value_changed(&mut self, new: f64, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_auto_fit(cx);
        self.update_state(window, cx, |state| {
            state.free_rotation_degrees = new;
            state.was_edited = true;
        });
    }

    fn dial_interaction_changed(&mut self, active: bool, cx: &mut Context<Self>) {
        let degrees = self.state.get(cx).free_rotation_degrees;
        if active {
            self.is_dial_anchor_held = true;
            self.is_fine_rotation_adjusting = true;
            self.is_adjusting = true;
            self.invalidate_auto_fit(cx);
            RiseAppEvents::default_center().post_with(
                EDITOR_CROP_FINE_ROTATION_INTERACTION,
                EditorCropFineRotationInteractionUpdate {
                    scope_id: self.crop_coordination_id,
                    is_active: true,
                    degrees,
                    completion_id: None,
                },
            );
        } else {
            self.is_fine_rotation_adjusting = false;
            self.is_adjusting = false;
            let completion_id = Uuid::new_v4();
            self.fine_rotation_completion_id = Some(completion_id);
            RiseAppEvents::default_center().post_with(
                EDITOR_CROP_FINE_ROTATION_INTERACTION,
                EditorCropFineRotationInteractionUpdate {
                    scope_id: self.crop_coordination_id,
                    is_active: false,
                    degrees,
                    completion_id: Some(completion_id),
                },
            );
        }
        cx.notify();
    }

    fn handle_fine_rotation_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !event.modifiers.shift || self.rotation_steps_animating.is_some() || self.is_resetting_crop {
            return;
        }
        cx.stop_propagation();

        let frame_width = self.dial_anchor.map(|anchor| anchor.size.width).unwrap_or(0.0);
        let pixels_per_degree = EditorRotationDial::pixels_per_degree(self.rotation_dial_width(frame_width));
        let delta = event.delta.pixel_delta(window.line_height());
        let travel = if f32::from(delta.y).abs() >= f32::from(delta.x).abs() { delta.y } else { delta.x };

        if self.fine_rotation_wheel_task.is_none() {
            self.fine_rotation_wheel_raw = self.state.get(cx).free_rotation_degrees;
            self.dial_interaction_changed(true, cx);
        }
        let was_in_detent = EditorRotationDial::detent(self.fine_rotation_wheel_raw).is_some();
        self.fine_rotation_wheel_raw -= f64::from(f32::from(travel) / pixels_per_degree);
        let detent = EditorRotationDial::detent(self.fine_rotation_wheel_raw);
        if detent.is_some() && !was_in_detent {
            let _ = selection_changed();
        }
        let new = EditorRotationDial::wrapped(detent.unwrap_or(self.fine_rotation_wheel_raw));
        self.dial_value_changed(new, window, cx);

        let executor = cx.background_executor().clone();
        self.fine_rotation_wheel_task = Some(cx.spawn_in(window, async move |this, cx| {
            executor.timer(Duration::from_secs_f64(FINE_ROTATION_WHEEL_IDLE_SECONDS)).await;
            let _ = this.update_in(cx, |this, _window, cx| {
                this.fine_rotation_wheel_task = None;
                this.dial_interaction_changed(false, cx);
            });
        }));
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let is_escape = event.keystroke.key == "escape";
        if !self.overlay_allows_hit_testing() && !is_escape {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" if !event.keystroke.modifiers.modified() && self.focus_handle.is_focused(window) => {
                self.confirm_crop(window, cx);
            }
            "escape" => self.cancel_crop(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn cancel_crop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settle_auto_fit_for_exit(window, cx);
        if let Some(state_at_entry) = self.state_at_entry.clone() {
            self.state.set(state_at_entry, window, cx);
        }
        (self.on_close.clone())(window, cx);
    }

    fn confirm_crop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settle_auto_fit_for_exit(window, cx);
        self.sync_crop_relative_from_visible(None, window, cx);
        self.update_state(window, cx, |state| state.was_edited = true);
        (self.on_confirm.clone())(window, cx);
    }

    fn toggle_mirror(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_auto_fit(cx);
        self.update_state(window, cx, |state| {
            state.is_mirrored = !state.is_mirrored;
            state.was_edited = true;
        });
    }

    fn select_aspect(&mut self, aspect: EditorCropAspect, window: &mut Window, cx: &mut Context<Self>) {
        if !self.overlay_allows_hit_testing() {
            return;
        }
        self.invalidate_auto_fit(cx);
        self.update_state(window, cx, |state| {
            state.aspect = aspect;
            state.was_edited = true;
        });
        self.apply_aspect(aspect, window, cx);
        self.schedule_auto_zoom(window, cx);
    }

    fn reset_crop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_resetting_crop {
            return;
        }
        self.invalidate_auto_fit(cx);
        self.is_resetting_crop = true;
        self.is_dial_anchor_held = false;
        self.is_fine_rotation_adjusting = false;
        self.is_adjusting = false;
        self.is_image_transforming = false;
        self.orientation_change_id = None;
        self.fine_rotation_completion_id = None;

        let request_id = Uuid::new_v4();
        self.reset_id = Some(request_id);
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_RESET,
            EditorCropResetRequest { scope_id: self.crop_coordination_id, id: request_id },
        );
        let state = self.state.get(cx);
        let mut reset_state = EditorImageBake::remap_overlay_coordinates(
            &state,
            EditorOverlayCoordinateSpace {
                crop_rect_relative: rect(0.0, 0.0, 1.0, 1.0),
                rotation_steps: 0,
                free_rotation_degrees: 0.0,
                is_mirrored: false,
            },
        );
        reset_state.free_rotation_degrees = 0.0;
        reset_state.crop_rect_relative = None;
        reset_state.crop_baked_into_canvas = false;
        reset_state.aspect = EditorCropAspect::Freeform;
        reset_state.rotation_steps = 0;
        reset_state.is_mirrored = false;
        self.state.set(reset_state, window, cx);
        self.sync_overlay_visibility(None, cx);
        cx.notify();
        (self.on_reset.clone())(window, cx);
        self.reset_watchdog_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(RESET_WATCHDOG_SECONDS)).await;
            let _ = this.update_in(cx, |this, window, cx| this.release_stalled_reset(request_id, window, cx));
        }));
    }

    fn release_stalled_reset(&mut self, request_id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_watchdog_task = None;
        if !self.is_resetting_crop || self.reset_id != Some(request_id) {
            return;
        }
        self.reset_id = None;
        self.is_resetting_crop = false;
        self.sync_overlay_visibility(None, cx);
        self.restore_crop_from_relative(false, cx);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn trigger_step_rotation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rotation_steps_animating.is_some() || self.is_resetting_crop {
            return;
        }
        self.invalidate_auto_fit(cx);
        self.is_fine_rotation_adjusting = false;
        self.is_adjusting = false;
        self.sync_crop_relative_from_visible(None, window, cx);
        let state = self.state.get(cx);
        let target_steps = (state.rotation_steps + 3) % 4;
        let target_crop = Self::crop_rect_after_quarter_turn(&state, target_steps);
        let request_id = Uuid::new_v4();
        self.orientation_change_id = Some(request_id);
        self.rotation_steps_animating = Some(target_steps);
        self.is_dial_anchor_held = true;
        self.sync_overlay_visibility(Some(ease_out(0.12)), cx);
        self.sync_dial_rotation_opacity(cx);
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_ORIENTATION_CHANGE,
            EditorCropOrientationRequest {
                scope_id: self.crop_coordination_id,
                id: request_id,
                rotation_steps: target_steps,
            },
        );
        self.update_state(window, cx, |state| {
            state.rotation_steps = target_steps;
            state.crop_rect_relative = target_crop;
            state.was_edited = true;
        });
        cx.notify();
    }

    fn crop_rect_after_quarter_turn(state: &EditorImageState, target_steps: i64) -> Option<Bounds<f32>> {
        let crop = state.crop_rect_relative?;
        let source_space = EditorOverlayCoordinateSpace {
            crop_rect_relative: crop,
            rotation_steps: state.rotation_steps,
            free_rotation_degrees: state.free_rotation_degrees,
            is_mirrored: state.is_mirrored,
        };
        let destination_space = EditorOverlayCoordinateSpace {
            crop_rect_relative: crop,
            rotation_steps: target_steps,
            free_rotation_degrees: state.free_rotation_degrees,
            is_mirrored: state.is_mirrored,
        };
        let corners = [
            point(min_x(&crop), min_y(&crop)),
            point(max_x(&crop), min_y(&crop)),
            point(min_x(&crop), max_y(&crop)),
            point(max_x(&crop), max_y(&crop)),
        ]
        .map(|corner| {
            let source_point =
                EditorImageBake::map_from_oriented_unit(corner, state.source_pixel_size, &source_space);
            EditorImageBake::map_to_oriented_unit(source_point, state.source_pixel_size, &destination_space)
        });
        let left = corners.iter().map(|corner| corner.x).fold(f32::INFINITY, f32::min);
        let right = corners.iter().map(|corner| corner.x).fold(f32::NEG_INFINITY, f32::max);
        let top = corners.iter().map(|corner| corner.y).fold(f32::INFINITY, f32::min);
        let bottom = corners.iter().map(|corner| corner.y).fold(f32::NEG_INFINITY, f32::max);
        let mapped = intersection(&rect(left, top, right - left, bottom - top), &rect(0.0, 0.0, 1.0, 1.0));
        if mapped.size.width <= 0.0 || mapped.size.height <= 0.0 {
            return None;
        }
        if EditorCropAutoFitPlanner::is_effectively_full(Some(mapped)) { None } else { Some(mapped) }
    }

    fn schedule_auto_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.auto_fit_task = None;
        if self.state.get(cx).free_rotation_degrees.abs() > EditorImageLayout::GEOMETRY_EPSILON_DEGREES {
            return;
        }
        let crop_snapshot = self.local_crop_rect;
        EditorCropAutoFitLog::log(
            "crop.debounceScheduled",
            &[("delaySec", &EditorCropAutoFit::DEBOUNCE_SECONDS), ("crop", &crop_snapshot)],
        );
        let executor = cx.background_executor().clone();
        self.auto_fit_task = Some(cx.spawn_in(window, async move |this, cx| {
            executor.timer(Duration::from_secs_f64(EditorCropAutoFit::DEBOUNCE_SECONDS)).await;
            let _ = this.update_in(cx, |this, window, cx| this.fire_auto_zoom(crop_snapshot, window, cx));
        }));
    }

    fn fire_auto_zoom(&mut self, crop_snapshot: Bounds<f32>, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_adjusting {
            EditorCropAutoFitLog::log(
                "crop.debounceCancelled",
                &[("cancelled", &false), ("isAdjusting", &self.is_adjusting)],
            );
            return;
        }
        if self.stage_image_layout.is_none() || self.is_resetting_crop || self.is_image_transforming {
            return;
        }
        if crop_snapshot.size.width < EditorCropAutoFit::MIN_CROP_EXTENT
            || crop_snapshot.size.height < EditorCropAutoFit::MIN_CROP_EXTENT
        {
            return;
        }
        let Some(target_rect) = self.auto_fit_target(crop_snapshot) else {
            return;
        };
        self.sync_crop_relative_from_visible(Some(crop_snapshot), window, cx);
        EditorCropAutoFitLog::log("crop.debounceFired", &[("crop", &crop_snapshot)]);
        let request_id = Uuid::new_v4();
        self.is_crop_auto_fitting = true;
        self.auto_fit_id = Some(request_id);
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_AUTO_FIT,
            EditorCropAutoFitRequest {
                scope_id: self.crop_coordination_id,
                id: request_id,
                source_rect: crop_snapshot,
                target_rect,
            },
        );
        cx.notify();
    }

    fn auto_fit_target(&self, crop: Bounds<f32>) -> Option<Bounds<f32>> {
        let zoom = self.stage_image_layout.map(|layout| layout.zoom).unwrap_or(1.0);
        EditorCropAutoFitPlanner::target_rect(
            crop,
            self.crop_horizontal_slot(),
            EditorCropAutoFit::CROP_MAXIMUM_ZOOM_SCALE / zoom.max(0.001),
        )
    }

    fn post_auto_fit_cancel(&self) {
        if let Some(id) = self.auto_fit_id {
            RiseAppEvents::default_center().post_with(
                EDITOR_CROP_AUTO_FIT_CANCEL,
                EditorCropAutoFitCancel { scope_id: self.crop_coordination_id, id },
            );
        }
    }

    fn cancel_active_auto_fit(&mut self, restore: bool, cx: &mut Context<Self>) {
        if self.auto_fit_id.is_none() {
            return;
        }
        self.post_auto_fit_cancel();
        self.is_crop_auto_fitting = false;
        self.auto_fit_id = None;
        self.auto_fit_presentation_rect = None;
        if restore {
            let this = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = this.update(cx, |this, cx| this.restore_crop_from_relative(false, cx));
            });
        }
    }

    fn invalidate_auto_fit(&mut self, cx: &mut Context<Self>) {
        self.auto_fit_task = None;
        self.cancel_active_auto_fit(false, cx);
    }

    fn settle_auto_fit_for_exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_auto_fit(cx);
        self.sync_crop_relative_from_visible(None, window, cx);
    }

    fn apply_aspect(&mut self, aspect: EditorCropAspect, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = self.crop_clamp_bounds();
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            return;
        }
        let state = self.state.get(cx);
        let original_ratio =
            self.locked_aspect_ratio(&state).unwrap_or_else(|| bounds.size.width / bounds.size.height);
        let Some(r) = aspect.ratio(original_ratio) else {
            return;
        };
        let center_x = mid_x(&self.local_crop_rect);
        let center_y = mid_y(&self.local_crop_rect);
        let max_w = bounds.size.width;
        let max_h = bounds.size.height;
        let mut new_w = max_w;
        let mut new_h = new_w / r;
        if new_h > max_h {
            new_h = max_h;
            new_w = new_h * r;
        }
        let new_rect = EditorCropOverlayUIView::clamp(
            rect(center_x - new_w / 2.0, center_y - new_h / 2.0, new_w, new_h),
            bounds,
            EditorCropOverlayUIView::DEFAULT_CLAMP_MIN_SIDE,
        );
        self.set_local_crop_rect(new_rect, Some(spring(0.4, 0.85)), cx);
        self.publish_crop_frame_in_stage(Some(new_rect));
        self.sync_crop_relative_from_visible(Some(new_rect), window, cx);
    }

    fn relative(crop_screen: Bounds<f32>, visible_image: Bounds<f32>) -> Bounds<f32> {
        EditorCropAutoFitPlanner::relative_rect(crop_screen, visible_image).unwrap_or_else(|| rect(0.0, 0.0, 1.0, 1.0))
    }

    fn on_overlay_interaction_start(&mut self, cx: &mut Context<Self>) {
        self.is_adjusting = true;
        self.invalidate_auto_fit(cx);
        cx.notify();
    }

    fn on_overlay_interaction_end(&mut self, new_rect: Bounds<f32>, window: &mut Window, cx: &mut Context<Self>) {
        let clamped = self.clamp_crop_to_image_bounds(new_rect);
        if clamped != self.local_crop_rect {
            self.set_local_crop_rect(clamped, None, cx);
        }
        self.sync_crop_relative_from_visible(Some(clamped), window, cx);
        self.update_state(window, cx, |state| state.was_edited = true);
        self.is_adjusting = false;
        let relative = format!("{:?}", self.state.get(cx).crop_rect_relative);
        EditorCropAutoFitLog::log(
            "crop.interactionEnd",
            &[
                ("crop", &new_rect),
                ("relative", &relative),
                ("viewport", &self.crop_viewport_rect()),
                ("visible", &self.visible_image_rect()),
            ],
        );
        self.schedule_auto_zoom(window, cx);
        cx.notify();
    }

    fn crop_overlay_layer(&self, state: &EditorImageState, cx: &mut Context<Self>) -> EditorCropOverlayRepresentable {
        let frame_panel = cx.weak_entity();
        let start_panel = cx.weak_entity();
        let end_panel = cx.weak_entity();
        EditorCropOverlayRepresentable {
            crop_coordination_id: self.crop_coordination_id,
            crop_rect: Binding::entity(
                cx.entity(),
                |panel: &Self| panel.local_crop_rect,
                |panel, new_rect, cx| panel.set_local_crop_rect(new_rect, None, cx),
            ),
            crop_viewport_rect: self.crop_viewport_rect(),
            visible_image_rect: self.visible_image_rect(),
            horizontal_slot_rect: self.crop_horizontal_slot(),
            aspect_ratio: self.locked_aspect_ratio(state),
            is_auto_fitting: self.is_crop_auto_fitting,
            grid_mode: self.crop_grid_mode(),
            allows_hit_testing: self.overlay_allows_hit_testing(),
            on_frame_change: Rc::new(move |new_rect, _window, cx| {
                let _ = frame_panel.update(cx, |panel, _| panel.publish_crop_frame_in_stage(Some(new_rect)));
            }),
            on_interaction_start: Rc::new(move |_window, cx| {
                let _ = start_panel.update(cx, |panel, cx| panel.on_overlay_interaction_start(cx));
            }),
            on_interaction_end: Rc::new(move |new_rect, window, cx| {
                let _ = end_panel.update(cx, |panel, cx| panel.on_overlay_interaction_end(new_rect, window, cx));
            }),
        }
    }

    fn dial_overlay(&self, anchor: Bounds<f32>, now: Duration, cx: &mut Context<Self>) -> Option<AnyElement> {
        if is_empty(&anchor) {
            return None;
        }
        let dial_width = self.rotation_dial_width(anchor.size.width);
        let hidden = self.is_resetting_crop || self.stage_image_layout.is_none();
        let opacity = if hidden { 0.0 } else { self.dial_rotation_opacity.value(now) };
        let blocks_input = self.rotation_steps_animating.is_some() || self.is_resetting_crop;

        let state = self.state.clone();
        let degrees_state = self.state.clone();
        let degrees = Binding::new(
            move |cx: &App| state.get(cx).free_rotation_degrees,
            move |new, window, cx| {
                let mut state = degrees_state.get(cx);
                state.free_rotation_degrees = new;
                degrees_state.set(state, window, cx);
            },
        );
        let change_panel = cx.weak_entity();
        let interaction_panel = cx.weak_entity();
        let dial = EditorRotationDial::new("editor-crop-rotation-dial", degrees, move |new, window, cx| {
            let _ = change_panel.update(cx, |panel, cx| panel.dial_value_changed(new, window, cx));
        })
        .on_interaction_change(move |active, _window, cx| {
            let _ = interaction_panel.update(cx, |panel, cx| panel.dial_interaction_changed(active, cx));
        });

        Some(
            div()
                .absolute()
                .left(px(mid_x(&anchor) - dial_width / 2.0))
                .top(px(max_y(&anchor)))
                .w(px(dial_width))
                .h(px(EditorRotationDial::height(dial_width)))
                .opacity(opacity)
                .occlude()
                .child(dial)
                .when(blocks_input, |dial| dial.child(div().absolute().inset_0().occlude()))
                .into_any_element(),
        )
    }

    /// action runs only while hit testing of the tools would let the press through
    fn tool_action(
        this: WeakEntity<Self>,
        action: fn(&mut Self, &mut Window, &mut Context<Self>),
    ) -> impl Fn(&mut Window, &mut App) + 'static {
        move |window, cx| {
            let _ = this.update(cx, |panel, cx| {
                if panel.overlay_allows_hit_testing() {
                    action(panel, window, cx);
                }
            });
        }
    }

    fn tool_glyph(symbol: &str) -> Div {
        div()
            .w(px(46.0))
            .h(px(48.0))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(symbol, px(16.0), white(), false))
    }

    fn bottom_tools(&self, state: &EditorImageState, cx: &mut Context<Self>) -> Div {
        let this = cx.weak_entity();
        let is_aspect_menu_presented = Binding::entity(
            cx.entity(),
            |panel: &Self| panel.is_aspect_menu_presented,
            |panel, value, cx| {
                panel.is_aspect_menu_presented = value;
                cx.notify();
            },
        );
        let current_aspect = state.aspect;
        let aspect_actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| {
            EditorCropAspect::ALL_CASES
                .iter()
                .map(|aspect| {
                    HoldContextMenuAction::new(aspect.raw_value(), aspect.localized_title())
                        .checked(*aspect == current_aspect)
                })
                .collect()
        });
        let on_aspect_action: HoldContextMenuPresentationOnAction = {
            let this = this.clone();
            Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
                let Some(aspect) = EditorCropAspect::from_raw_value(&id) else {
                    return;
                };
                let _ = this.update(cx, |panel, cx| panel.select_aspect(aspect, window, cx));
            })
        };
        let aspect_source: HoldContextMenuTapSource =
            Rc::new(|_: &mut Window, _: &mut App| Self::tool_glyph("aspectratio").into_any_element());
        let open_aspect_menu = is_aspect_menu_presented.clone();
        let on_chrome_height_change = self.on_chrome_height_change.clone();

        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .pb(px(22.0))
            .child(div().h(px(editor_rotation_dial::DIAL_HEIGHT)))
            .child(
                div().flex().flex_row().justify_center().child(
                    div().occlude().child(
                        LiquidGlassButton::new("editor-crop-reset", LiquidGlassButtonShape::Capsule)
                            .horizontal_padding(16.0)
                            .vertical_padding(9.0)
                            .on_press(Self::tool_action(this.clone(), Self::reset_crop))
                            .child(
                                MainText::new(tr("editor_crop_reset"))
                                    .px(14.)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(white()),
                            ),
                    ),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(px(12.0))
                    .px(px(14.0))
                    .child(
                        div().occlude().child(
                            LiquidGlassButton::new("editor-crop-close", LiquidGlassButtonShape::Circle)
                                .size(48.0)
                                .on_press(Self::tool_action(this.clone(), Self::cancel_crop))
                                .children(IconUi::sized("xmark", px(16.0), white(), false)),
                        ),
                    )
                    .child(
                        div().occlude().child(
                            div()
                                .flex()
                                .flex_row()
                                .px(px(3.0))
                                .child(
                                    plain("editor-crop-rotate")
                                        .on_press(Self::tool_action(this.clone(), Self::trigger_step_rotation))
                                        .child(Self::tool_glyph("rotate.left")),
                                )
                                .child(
                                    plain("editor-crop-mirror")
                                        .on_press(Self::tool_action(this.clone(), Self::toggle_mirror))
                                        .child(Self::tool_glyph(
                                            "arrow.left.and.right.righttriangle.left.righttriangle.right",
                                        )),
                                )
                                .child(
                                    plain("editor-crop-aspect")
                                        .on_press(move |window, cx| open_aspect_menu.set(true, window, cx))
                                        .child(Self::tool_glyph("aspectratio"))
                                        .tap_context_menu(
                                            "editor-crop-aspect-menu",
                                            is_aspect_menu_presented,
                                            HoldContextMenuAlignment::Trailing,
                                            true,
                                            false,
                                            false,
                                            None,
                                            aspect_actions,
                                            on_aspect_action,
                                            aspect_source,
                                        ),
                                )
                                .liquid_chrome_capsule_with(0.08, true, None, false),
                        ),
                    )
                    .child(
                        div().occlude().child(
                            LiquidGlassButton::new("editor-crop-confirm", LiquidGlassButtonShape::Circle)
                                .size(48.0)
                                .on_press(Self::tool_action(this, Self::confirm_crop))
                                .children(IconUi::sized("checkmark", px(16.0), white(), false)),
                        ),
                    ),
            )
            .child(editor_crop_chrome_height_reader("editor-crop-chrome-height", move |height, window, cx| {
                on_chrome_height_change(height, window, cx);
            }))
    }
}

impl Drop for EditorCropPanel {
    fn drop(&mut self) {
        self.post_auto_fit_cancel();
    }
}

impl Render for EditorCropPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let state = self.state.get(cx);
        let presented_crop_rect = self.presented_crop_rect(now);
        if self.animated_crop_rect.is_running(now)
            || self.overlay_opacity.is_running(now)
            || self.dial_rotation_opacity.is_running(now)
        {
            window.request_animation_frame();
        }

        let overlay = self.crop_overlay_layer(&state, cx);
        let dial_anchor = self.refresh_dial_anchor(presented_crop_rect);
        let dial = self.dial_overlay(dial_anchor, now, cx);
        let tools = self.bottom_tools(&state, cx);

        div()
            .id("editor-crop-panel")
            .absolute()
            .inset_0()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_scroll_wheel(cx.listener(Self::handle_fine_rotation_wheel))
            .child(div().absolute().inset_0().opacity(self.overlay_opacity.value(now)).child(overlay))
            .children(dial)
            .child(div().absolute().left_0().right_0().bottom_0().child(tools))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorCropGridMode {
    None,
    Major,
    Minor,
}

#[derive(IntoElement)]
pub struct EditorCropOverlayRepresentable {
    pub crop_coordination_id: Uuid,
    pub crop_rect: Binding<Bounds<f32>>,
    pub crop_viewport_rect: Bounds<f32>,
    pub visible_image_rect: Bounds<f32>,
    pub horizontal_slot_rect: Bounds<f32>,
    pub aspect_ratio: Option<f32>,
    pub is_auto_fitting: bool,
    pub grid_mode: EditorCropGridMode,
    pub allows_hit_testing: bool,
    pub on_frame_change: EditorCropRectCallback,
    pub on_interaction_start: EditorCropAction,
    pub on_interaction_end: EditorCropRectCallback,
}

impl EditorCropOverlayRepresentable {
    pub fn make_ui_view(window: &mut Window, cx: &mut App) -> Entity<EditorCropOverlayHostView> {
        window.use_keyed_state("editor-crop-overlay-host", cx, |_, cx| EditorCropOverlayHostView::new(cx))
    }

    pub fn update_ui_view(&self, ui_view: &Entity<EditorCropOverlayHostView>, cx: &mut App) {
        let crop_rect = self.crop_rect.get(cx);
        let overlay = ui_view.read(cx).overlay.clone();
        let on_frame_change = self.on_frame_change.clone();
        let binding = self.crop_rect.clone();
        overlay.update(cx, |overlay, cx| {
            overlay.on_change = Some(Rc::new(move |new_rect, window, cx| {
                on_frame_change(new_rect, window, cx);
                let binding = binding.clone();
                window.defer(cx, move |window, cx| binding.set(new_rect, window, cx));
            }));
            overlay.on_interaction_start = Some(self.on_interaction_start.clone());
            overlay.on_interaction_end = Some(self.on_interaction_end.clone());
            overlay.crop_coordination_id = self.crop_coordination_id;
            overlay.allows_hit_testing = self.allows_hit_testing;
            overlay.set(
                self.crop_viewport_rect,
                self.visible_image_rect,
                self.horizontal_slot_rect,
                crop_rect,
                self.aspect_ratio,
                self.is_auto_fitting,
                self.grid_mode,
                cx,
            );
        });
    }
}

impl RenderOnce for EditorCropOverlayRepresentable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = Self::make_ui_view(window, cx);
        self.update_ui_view(&host, cx);
        host
    }
}

pub struct EditorCropOverlayHostView {
    pub overlay: Entity<EditorCropOverlayUIView>,
}

impl EditorCropOverlayHostView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self { overlay: cx.new(EditorCropOverlayUIView::new) }
    }
}

impl Render for EditorCropOverlayHostView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().absolute().inset_0().child(self.overlay.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorCropOverlayUIViewResizeEdge {
    None,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Top,
    Bottom,
    Left,
    Right,
    Inside,
}

pub struct EditorCropOverlayUIView {
    image_rect: Bounds<f32>,
    aspect_ratio: Option<f32>,
    grid_mode: EditorCropGridMode,
    grid_opacity: Animated<f32>,
    last_input_crop_rect: Bounds<f32>,
    active_auto_fit_presentation_id: Option<Uuid>,
    auto_fit_presentation_inbox: Arc<Mutex<Vec<EditorCropAutoFitPresentationUpdate>>>,
    _auto_fit_presentation_observer: RiseSubscription,
    _auto_fit_presentation_pump: Task<()>,
    pub crop_rect: Bounds<f32>,
    pub on_change: Option<EditorCropRectCallback>,
    pub on_interaction_start: Option<EditorCropAction>,
    pub on_interaction_end: Option<EditorCropRectCallback>,
    pub crop_coordination_id: Uuid,
    pub is_capturing_handle_touch: bool,
    allows_hit_testing: bool,

    drag_start_crop: Bounds<f32>,
    current_edge: EditorCropOverlayUIViewResizeEdge,
    pan_press_location: Option<Point<f32>>,
    is_pan_active: bool,
    is_pinch_active: bool,
    pinch_scale: f32,
    hover_edge: EditorCropOverlayUIViewResizeEdge,
}

impl EditorCropOverlayUIView {
    pub const DEFAULT_HANDLE_SIZE: f32 = 44.0;
    pub const DEFAULT_HIT_OUTSET: f32 = 16.0;
    pub const DEFAULT_CLAMP_MIN_SIDE: f32 = 1.0;
    const CORNER_THICKNESS: f32 = 3.0;
    const EDGE_THICKNESS: f32 = 2.0;
    const MIN_CROP_SIDE: f32 = 1.0;
    const CORNER_ARM_LENGTH: f32 = 20.0;
    const SIDE_BAR_LENGTH: f32 = 24.0;

    pub fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut wake) = mpsc::unbounded::<()>();
        let auto_fit_presentation_inbox = Arc::new(Mutex::new(Vec::new()));
        let inbox_sink = auto_fit_presentation_inbox.clone();
        let observer =
            RiseAppEvents::default_center().observe(EDITOR_CROP_AUTO_FIT_PRESENTATION_DID_CHANGE, move |notification| {
                if let Some(update) = notification.payload::<EditorCropAutoFitPresentationUpdate>()
                    && let Ok(mut pending) = inbox_sink.lock()
                {
                    pending.push(*update);
                    let _ = sender.unbounded_send(());
                }
            });
        let pump = cx.spawn(async move |this, cx| {
            while wake.next().await.is_some() {
                let delivered = this.update(cx, |this, cx| {
                    if this.drain_auto_fit_presentations() {
                        cx.notify();
                    }
                });
                if delivered.is_err() {
                    break;
                }
            }
        });

        Self {
            image_rect: Bounds::default(),
            aspect_ratio: None,
            grid_mode: EditorCropGridMode::None,
            grid_opacity: Animated::new(0.0),
            last_input_crop_rect: Bounds::default(),
            active_auto_fit_presentation_id: None,
            auto_fit_presentation_inbox,
            _auto_fit_presentation_observer: observer,
            _auto_fit_presentation_pump: pump,
            crop_rect: Bounds::default(),
            on_change: None,
            on_interaction_start: None,
            on_interaction_end: None,
            crop_coordination_id: Uuid::new_v4(),
            is_capturing_handle_touch: false,
            allows_hit_testing: true,
            drag_start_crop: Bounds::default(),
            current_edge: EditorCropOverlayUIViewResizeEdge::None,
            pan_press_location: None,
            is_pan_active: false,
            is_pinch_active: false,
            pinch_scale: 1.0,
            hover_edge: EditorCropOverlayUIViewResizeEdge::None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set(
        &mut self,
        crop_viewport_rect: Bounds<f32>,
        visible_image_rect: Bounds<f32>,
        horizontal_slot_rect: Bounds<f32>,
        crop_rect: Bounds<f32>,
        aspect_ratio: Option<f32>,
        is_auto_fitting: bool,
        grid_mode: EditorCropGridMode,
        cx: &App,
    ) {
        let bounds = if is_auto_fitting {
            horizontal_slot_rect
        } else {
            Self::crop_clamp_bounds(crop_viewport_rect, visible_image_rect, horizontal_slot_rect)
        };
        self.image_rect = bounds;
        self.aspect_ratio = aspect_ratio;
        let input_changed = !Self::rect_approximately_equal(crop_rect, self.last_input_crop_rect);
        self.last_input_crop_rect = crop_rect;
        if self.active_auto_fit_presentation_id.is_none() && (self.crop_rect == Bounds::default() || input_changed) {
            self.crop_rect = Self::resolve_crop_rect(crop_rect, bounds);
        }
        self.set_grid_mode(grid_mode, cx);
    }

    fn rect_approximately_equal(lhs: Bounds<f32>, rhs: Bounds<f32>) -> bool {
        (lhs.origin.x - rhs.origin.x).abs() < 0.25
            && (lhs.origin.y - rhs.origin.y).abs() < 0.25
            && (lhs.size.width - rhs.size.width).abs() < 0.25
            && (lhs.size.height - rhs.size.height).abs() < 0.25
    }

    fn set_grid_mode(&mut self, mode: EditorCropGridMode, cx: &App) {
        if self.grid_mode == mode {
            return;
        }
        self.grid_mode = mode;
        let target_opacity = if mode == EditorCropGridMode::None { 0.0 } else { 1.0 };
        self.grid_opacity.set_in(target_opacity, ease_in_out(0.18), RiseClock::monotonic(), cx);
    }

    pub fn crop_clamp_bounds(viewport: Bounds<f32>, visible: Bounds<f32>, horizontal_slot: Bounds<f32>) -> Bounds<f32> {
        if visible.size.width < EditorCropAutoFit::MIN_CROP_EXTENT
            || visible.size.height < EditorCropAutoFit::MIN_CROP_EXTENT
        {
            return viewport;
        }
        let on_screen = intersection(&visible, &horizontal_slot);
        if on_screen.size.width >= EditorCropAutoFit::MIN_CROP_EXTENT
            && on_screen.size.height >= EditorCropAutoFit::MIN_CROP_EXTENT
        {
            return on_screen;
        }
        visible
    }

    /// callers without a minimum pass DEFAULT_CLAMP_MIN_SIDE, the default of min_side
    pub fn clamp(rect_to_clamp: Bounds<f32>, bounds: Bounds<f32>, min_side: f32) -> Bounds<f32> {
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            return rect_to_clamp;
        }
        let width = bounds.size.width.min(min_side.min(bounds.size.width).max(rect_to_clamp.size.width));
        let height = bounds.size.height.min(min_side.min(bounds.size.height).max(rect_to_clamp.size.height));
        let x = rect_to_clamp.origin.x.max(min_x(&bounds)).min(max_x(&bounds) - width);
        let y = rect_to_clamp.origin.y.max(min_y(&bounds)).min(max_y(&bounds) - height);
        rect(x, y, width, height)
    }

    fn resolve_crop_rect(crop_rect: Bounds<f32>, image_rect: Bounds<f32>) -> Bounds<f32> {
        if image_rect.size.width <= 0.0 || image_rect.size.height <= 0.0 {
            return crop_rect;
        }
        if crop_rect.size.width < EditorCropAutoFit::MIN_CROP_EXTENT
            || crop_rect.size.height < EditorCropAutoFit::MIN_CROP_EXTENT
        {
            return crop_rect;
        }
        Self::clamp(crop_rect, image_rect, Self::DEFAULT_CLAMP_MIN_SIDE)
    }

    /// applies every update posted since the last drain in posting order
    /// true when one landed
    fn drain_auto_fit_presentations(&mut self) -> bool {
        let pending = match self.auto_fit_presentation_inbox.lock() {
            Ok(mut pending) => std::mem::take(&mut *pending),
            Err(_) => return false,
        };
        let mut applied = false;
        for update in pending {
            if update.scope_id == self.crop_coordination_id {
                applied |= self.apply_auto_fit_presentation(update);
            }
        }
        applied
    }

    fn apply_auto_fit_presentation(&mut self, update: EditorCropAutoFitPresentationUpdate) -> bool {
        match update.phase {
            EditorCropAutoFitPresentationUpdatePhase::Began => {
                self.active_auto_fit_presentation_id = Some(update.id);
            }
            EditorCropAutoFitPresentationUpdatePhase::Changed | EditorCropAutoFitPresentationUpdatePhase::Ended => {
                if self.active_auto_fit_presentation_id != Some(update.id) {
                    return false;
                }
            }
        }
        self.crop_rect = update.crop_frame;
        if update.phase == EditorCropAutoFitPresentationUpdatePhase::Ended {
            self.active_auto_fit_presentation_id = None;
        }
        true
    }

    pub fn hit_test(&self, point: Point<f32>) -> bool {
        self.allows_hit_testing && self.hit_target(point)
    }

    fn hit_target(&self, point: Point<f32>) -> bool {
        !matches!(
            self.detect_edge(point),
            EditorCropOverlayUIViewResizeEdge::None | EditorCropOverlayUIViewResizeEdge::Inside
        )
    }

    fn handle_mouse_down(&mut self, location: Point<f32>) -> bool {
        if !self.hit_test(location) {
            return false;
        }
        self.is_capturing_handle_touch = true;
        self.pan_press_location = Some(location);
        true
    }

    fn handle_mouse_drag(&mut self, location: Point<f32>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = self.pan_press_location else {
            return;
        };
        if !self.is_pan_active {
            self.is_pan_active = true;
            self.drag_start_crop = self.crop_rect;
            self.current_edge = self.detect_edge(press);
            if let Some(on_interaction_start) = self.on_interaction_start.clone() {
                on_interaction_start(window, cx);
            }
        }
        self.apply_drag(point(location.x - press.x, location.y - press.y), window, cx);
    }

    fn handle_mouse_released(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pan_press_location.take().is_none() {
            return;
        }
        self.is_capturing_handle_touch = false;
        if !self.is_pan_active {
            return;
        }
        self.is_pan_active = false;
        self.current_edge = EditorCropOverlayUIViewResizeEdge::None;
        let crop_rect = self.crop_rect;
        if let Some(on_change) = self.on_change.clone() {
            on_change(crop_rect, window, cx);
        }
        if let Some(on_interaction_end) = self.on_interaction_end.clone() {
            on_interaction_end(crop_rect, window, cx);
        }
        cx.notify();
    }

    fn handle_hover(&mut self, location: Point<f32>, cx: &mut Context<Self>) {
        let edge = if self.allows_hit_testing { self.detect_edge(location) } else { EditorCropOverlayUIViewResizeEdge::None };
        if edge != self.hover_edge {
            self.hover_edge = edge;
            cx.notify();
        }
    }

    fn handle_pinch(
        &mut self,
        location: Point<f32>,
        delta: f32,
        phase: TouchPhase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_pinch_active && !(self.is_capturing_handle_touch || self.hit_test(location)) {
            return false;
        }
        let update_phase = match phase {
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if !self.is_pinch_active {
                    return true;
                }
                self.is_pinch_active = false;
                if self.pan_press_location.is_none() {
                    self.is_capturing_handle_touch = false;
                }
                EditorCropPinchUpdatePhase::Ended
            }
            TouchPhase::Started | TouchPhase::Moved if !self.is_pinch_active => {
                self.is_pinch_active = true;
                self.pinch_scale = 1.0 + delta;
                if let Some(on_interaction_start) = self.on_interaction_start.clone() {
                    on_interaction_start(window, cx);
                }
                EditorCropPinchUpdatePhase::Began
            }
            TouchPhase::Started | TouchPhase::Moved => {
                self.pinch_scale *= 1.0 + delta;
                EditorCropPinchUpdatePhase::Changed
            }
        };
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_PINCH,
            EditorCropPinchUpdate {
                scope_id: self.crop_coordination_id,
                phase: update_phase,
                scale: self.pinch_scale,
                center: location,
            },
        );
        if update_phase == EditorCropPinchUpdatePhase::Ended {
            let crop_rect = self.crop_rect;
            if let Some(on_interaction_end) = self.on_interaction_end.clone() {
                on_interaction_end(crop_rect, window, cx);
            }
        }
        true
    }

    fn detect_edge(&self, point: Point<f32>) -> EditorCropOverlayUIViewResizeEdge {
        Self::resize_edge(point, self.crop_rect, Self::DEFAULT_HANDLE_SIZE, Self::DEFAULT_HIT_OUTSET)
    }

    /// callers without their own metrics pass DEFAULT_HANDLE_SIZE and DEFAULT_HIT_OUTSET
    pub fn resize_edge(
        at: Point<f32>,
        crop_rect: Bounds<f32>,
        handle_size: f32,
        hit_outset: f32,
    ) -> EditorCropOverlayUIViewResizeEdge {
        use EditorCropOverlayUIViewResizeEdge as Edge;

        let hit_radius = handle_size / 2.0 + hit_outset;
        let outer = rect(
            min_x(&crop_rect) - hit_radius,
            min_y(&crop_rect) - hit_radius,
            crop_rect.size.width + hit_radius * 2.0,
            crop_rect.size.height + hit_radius * 2.0,
        );
        if !contains(&outer, at) {
            return Edge::None;
        }

        let corners = [
            (point(min_x(&crop_rect), min_y(&crop_rect)), Edge::TopLeft),
            (point(max_x(&crop_rect), min_y(&crop_rect)), Edge::TopRight),
            (point(min_x(&crop_rect), max_y(&crop_rect)), Edge::BottomLeft),
            (point(max_x(&crop_rect), max_y(&crop_rect)), Edge::BottomRight),
        ];
        let distance_squared = |corner: &Point<f32>| {
            let x = at.x - corner.x;
            let y = at.y - corner.y;
            x * x + y * y
        };
        let corner = corners
            .iter()
            .filter(|(corner, _)| (at.x - corner.x).abs() <= hit_radius && (at.y - corner.y).abs() <= hit_radius)
            .min_by(|lhs, rhs| distance_squared(&lhs.0).total_cmp(&distance_squared(&rhs.0)));
        if let Some((_, edge)) = corner {
            return *edge;
        }

        let left_distance = (at.x - min_x(&crop_rect)).abs();
        let right_distance = (at.x - max_x(&crop_rect)).abs();
        let top_distance = (at.y - min_y(&crop_rect)).abs();
        let bottom_distance = (at.y - max_y(&crop_rect)).abs();
        let horizontal_edge = if at.x <= mid_x(&crop_rect) { Edge::Left } else { Edge::Right };
        let vertical_edge = if at.y <= mid_y(&crop_rect) { Edge::Top } else { Edge::Bottom };
        let horizontal_distance = left_distance.min(right_distance);
        let vertical_distance = top_distance.min(bottom_distance);

        let near_horizontal_boundary = horizontal_distance <= hit_radius
            && at.y >= min_y(&crop_rect) - hit_radius
            && at.y <= max_y(&crop_rect) + hit_radius;
        let near_vertical_boundary = vertical_distance <= hit_radius
            && at.x >= min_x(&crop_rect) - hit_radius
            && at.x <= max_x(&crop_rect) + hit_radius;

        if near_horizontal_boundary && near_vertical_boundary {
            return if horizontal_distance < vertical_distance { horizontal_edge } else { vertical_edge };
        }
        if near_horizontal_boundary {
            return horizontal_edge;
        }
        if near_vertical_boundary {
            return vertical_edge;
        }
        Edge::Inside
    }

    fn apply_drag(&mut self, translation: Point<f32>, window: &mut Window, cx: &mut Context<Self>) {
        use EditorCropOverlayUIViewResizeEdge as Edge;

        if let Some(ratio) =
            self.aspect_ratio.filter(|ratio| *ratio > 0.0 && self.current_edge != Edge::Inside)
        {
            let new_rect = self.aspect_locked_rect(translation, ratio);
            self.commit_drag(new_rect, window, cx);
            return;
        }

        let start = self.drag_start_crop;
        let mut left = min_x(&start);
        let mut right = max_x(&start);
        let mut top = min_y(&start);
        let mut bottom = max_y(&start);

        match self.current_edge {
            Edge::TopLeft => {
                left += translation.x;
                top += translation.y;
            }
            Edge::TopRight => {
                right += translation.x;
                top += translation.y;
            }
            Edge::BottomLeft => {
                left += translation.x;
                bottom += translation.y;
            }
            Edge::BottomRight => {
                right += translation.x;
                bottom += translation.y;
            }
            Edge::Top => top += translation.y,
            Edge::Bottom => bottom += translation.y,
            Edge::Left => left += translation.x,
            Edge::Right => right += translation.x,
            Edge::Inside => {
                left += translation.x;
                right += translation.x;
                top += translation.y;
                bottom += translation.y;
            }
            Edge::None => return,
        }

        Self::enforce_min_size(self.current_edge, &mut left, &mut right, &mut top, &mut bottom);

        let new_rect = Self::clamp(
            rect(left, top, right - left, bottom - top),
            self.image_rect,
            Self::DEFAULT_CLAMP_MIN_SIDE,
        );
        self.commit_drag(new_rect, window, cx);
    }

    fn commit_drag(&mut self, new_rect: Bounds<f32>, window: &mut Window, cx: &mut Context<Self>) {
        self.crop_rect = new_rect;
        cx.notify();
        if let Some(on_change) = self.on_change.clone() {
            on_change(new_rect, window, cx);
        }
    }

    fn aspect_locked_rect(&self, translation: Point<f32>, ratio: f32) -> Bounds<f32> {
        use EditorCropOverlayUIViewResizeEdge as Edge;

        let start = self.drag_start_crop;
        let bounds = self.image_rect;
        let minimum_width = Self::MIN_CROP_SIDE.max(Self::MIN_CROP_SIDE * ratio);
        let minimum_height = Self::MIN_CROP_SIDE.max(Self::MIN_CROP_SIDE / ratio);

        match self.current_edge {
            Edge::Left | Edge::Right => {
                let is_left = self.current_edge == Edge::Left;
                let anchor_x = if is_left { max_x(&start) } else { min_x(&start) };
                let moving_x = (if is_left { min_x(&start) } else { max_x(&start) }) + translation.x;
                let proposed_width = if is_left { anchor_x - moving_x } else { moving_x - anchor_x };
                let max_width = if is_left { anchor_x - min_x(&bounds) } else { max_x(&bounds) - anchor_x };
                let width = minimum_width.max(proposed_width).min(max_width).min(bounds.size.height * ratio);
                let height = width / ratio;
                let x = if is_left { anchor_x - width } else { anchor_x };
                let y = (mid_y(&start) - height / 2.0).max(min_y(&bounds)).min(max_y(&bounds) - height);
                rect(x, y, width, height)
            }
            Edge::Top | Edge::Bottom => {
                let is_top = self.current_edge == Edge::Top;
                let anchor_y = if is_top { max_y(&start) } else { min_y(&start) };
                let moving_y = (if is_top { min_y(&start) } else { max_y(&start) }) + translation.y;
                let proposed_height = if is_top { anchor_y - moving_y } else { moving_y - anchor_y };
                let max_height = if is_top { anchor_y - min_y(&bounds) } else { max_y(&bounds) - anchor_y };
                let height = minimum_height.max(proposed_height).min(max_height).min(bounds.size.width / ratio);
                let width = height * ratio;
                let x = (mid_x(&start) - width / 2.0).max(min_x(&bounds)).min(max_x(&bounds) - width);
                let y = if is_top { anchor_y - height } else { anchor_y };
                rect(x, y, width, height)
            }
            Edge::TopLeft | Edge::TopRight | Edge::BottomLeft | Edge::BottomRight => {
                let is_left = matches!(self.current_edge, Edge::TopLeft | Edge::BottomLeft);
                let is_top = matches!(self.current_edge, Edge::TopLeft | Edge::TopRight);
                let anchor = point(
                    if is_left { max_x(&start) } else { min_x(&start) },
                    if is_top { max_y(&start) } else { min_y(&start) },
                );
                let moving = point(
                    (if is_left { min_x(&start) } else { max_x(&start) }) + translation.x,
                    (if is_top { min_y(&start) } else { max_y(&start) }) + translation.y,
                );
                let proposed_width = if is_left { anchor.x - moving.x } else { moving.x - anchor.x };
                let proposed_height = if is_top { anchor.y - moving.y } else { moving.y - anchor.y };
                let max_width = if is_left { anchor.x - min_x(&bounds) } else { max_x(&bounds) - anchor.x };
                let max_height = if is_top { anchor.y - min_y(&bounds) } else { max_y(&bounds) - anchor.y };
                let width = minimum_width
                    .max((proposed_width + proposed_height * ratio) / 2.0)
                    .min(max_width)
                    .min(max_height * ratio);
                let height = width / ratio;
                rect(
                    if is_left { anchor.x - width } else { anchor.x },
                    if is_top { anchor.y - height } else { anchor.y },
                    width,
                    height,
                )
            }
            Edge::None | Edge::Inside => start,
        }
    }

    fn enforce_min_size(
        edge: EditorCropOverlayUIViewResizeEdge,
        left: &mut f32,
        right: &mut f32,
        top: &mut f32,
        bottom: &mut f32,
    ) {
        use EditorCropOverlayUIViewResizeEdge as Edge;

        if *right - *left < Self::MIN_CROP_SIDE {
            match edge {
                Edge::TopLeft | Edge::BottomLeft | Edge::Left => *left = *right - Self::MIN_CROP_SIDE,
                Edge::TopRight | Edge::BottomRight | Edge::Right => *right = *left + Self::MIN_CROP_SIDE,
                _ => {
                    let mid = (*left + *right) / 2.0;
                    *left = mid - Self::MIN_CROP_SIDE / 2.0;
                    *right = mid + Self::MIN_CROP_SIDE / 2.0;
                }
            }
        }
        if *bottom - *top < Self::MIN_CROP_SIDE {
            match edge {
                Edge::TopLeft | Edge::TopRight | Edge::Top => *top = *bottom - Self::MIN_CROP_SIDE,
                Edge::BottomLeft | Edge::BottomRight | Edge::Bottom => *bottom = *top + Self::MIN_CROP_SIDE,
                _ => {
                    let mid = (*top + *bottom) / 2.0;
                    *top = mid - Self::MIN_CROP_SIDE / 2.0;
                    *bottom = mid + Self::MIN_CROP_SIDE / 2.0;
                }
            }
        }
    }

    fn cursor_style(&self) -> Option<CursorStyle> {
        use EditorCropOverlayUIViewResizeEdge as Edge;

        let edge = if self.is_pan_active { self.current_edge } else { self.hover_edge };
        match edge {
            Edge::TopLeft | Edge::BottomRight => Some(CursorStyle::ResizeUpLeftDownRight),
            Edge::TopRight | Edge::BottomLeft => Some(CursorStyle::ResizeUpRightDownLeft),
            Edge::Top | Edge::Bottom => Some(CursorStyle::ResizeUpDown),
            Edge::Left | Edge::Right => Some(CursorStyle::ResizeLeftRight),
            Edge::None | Edge::Inside => None,
        }
    }

    fn filled_rect(x: f32, y: f32, width: f32, height: f32, color: Hsla) -> Div {
        div().absolute().left(px(x)).top(px(y)).w(px(width.max(0.0))).h(px(height.max(0.0))).bg(color)
    }

    fn dim_bands(crop: &Bounds<f32>, view: &Bounds<f32>) -> [Div; 4] {
        let color = black().alpha(0.55);
        [
            Self::filled_rect(min_x(view), min_y(view), view.size.width, min_y(crop) - min_y(view), color),
            Self::filled_rect(min_x(view), max_y(crop), view.size.width, max_y(view) - max_y(crop), color),
            Self::filled_rect(min_x(view), min_y(crop), min_x(crop) - min_x(view), crop.size.height, color),
            Self::filled_rect(max_x(crop), min_y(crop), max_x(view) - max_x(crop), crop.size.height, color),
        ]
    }

    fn frame_lines(crop: &Bounds<f32>) -> [Div; 4] {
        let color = white();
        let half = 0.5;
        [
            Self::filled_rect(min_x(crop) - half, min_y(crop) - half, crop.size.width + 1.0, 1.0, color),
            Self::filled_rect(min_x(crop) - half, max_y(crop) - half, crop.size.width + 1.0, 1.0, color),
            Self::filled_rect(min_x(crop) - half, min_y(crop) - half, 1.0, crop.size.height + 1.0, color),
            Self::filled_rect(max_x(crop) - half, min_y(crop) - half, 1.0, crop.size.height + 1.0, color),
        ]
    }

    fn grid_lines(&self, crop: &Bounds<f32>) -> Vec<Div> {
        if self.grid_mode == EditorCropGridMode::None {
            return Vec::new();
        }
        let divisions: usize = if self.grid_mode == EditorCropGridMode::Minor { 9 } else { 3 };
        let color = white().alpha(0.45);
        let column_width = crop.size.width / divisions as f32;
        let row_height = crop.size.height / divisions as f32;
        let mut lines = Vec::with_capacity((divisions - 1) * 2);
        for index in 1..divisions {
            let offset = index as f32;
            lines.push(Self::filled_rect(
                min_x(crop) + column_width * offset - 0.25,
                min_y(crop),
                0.5,
                crop.size.height,
                color,
            ));
            lines.push(Self::filled_rect(
                min_x(crop),
                min_y(crop) + row_height * offset - 0.25,
                crop.size.width,
                0.5,
                color,
            ));
        }
        lines
    }

    fn handle_visuals(crop: &Bounds<f32>) -> Vec<Div> {
        let color = white();
        let arm = Self::CORNER_ARM_LENGTH;
        let thick = Self::CORNER_THICKNESS;
        let (left, right, top, bottom) = (min_x(crop), max_x(crop), min_y(crop), max_y(crop));
        let edge = Self::EDGE_THICKNESS;
        let bar = Self::SIDE_BAR_LENGTH;
        vec![
            Self::filled_rect(left, top, arm, thick, color),
            Self::filled_rect(left, top, thick, arm, color),
            Self::filled_rect(right - arm, top, arm, thick, color),
            Self::filled_rect(right - thick, top, thick, arm, color),
            Self::filled_rect(left, bottom - thick, arm, thick, color),
            Self::filled_rect(left, bottom - arm, thick, arm, color),
            Self::filled_rect(right - arm, bottom - thick, arm, thick, color),
            Self::filled_rect(right - thick, bottom - arm, thick, arm, color),
            Self::filled_rect(mid_x(crop) - bar / 2.0, top - edge / 2.0, bar, edge, color),
            Self::filled_rect(mid_x(crop) - bar / 2.0, bottom - edge / 2.0, bar, edge, color),
            Self::filled_rect(left - edge / 2.0, mid_y(crop) - bar / 2.0, edge, bar, color),
            Self::filled_rect(right - edge / 2.0, mid_y(crop) - bar / 2.0, edge, bar, color),
        ]
    }
}

fn local_point(position: Point<Pixels>, origin: Point<Pixels>) -> Point<f32> {
    point(f32::from(position.x - origin.x), f32::from(position.y - origin.y))
}

impl Render for EditorCropOverlayUIView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drain_auto_fit_presentations();
        if self.crop_rect == Bounds::default() {
            self.crop_rect = self.image_rect;
        }
        let now = RiseClock::monotonic();
        if self.grid_opacity.is_running(now) {
            window.request_animation_frame();
        }
        let crop = self.crop_rect;
        let view_size = window.viewport_size();
        let view = rect(
            min_x(&crop).min(0.0),
            min_y(&crop).min(0.0),
            f32::from(view_size.width) + min_x(&crop).min(0.0).abs(),
            f32::from(view_size.height) + min_y(&crop).min(0.0).abs(),
        );
        let overlay = cx.entity();
        let cursor = self.cursor_style();

        div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .children(Self::dim_bands(&crop, &view))
            .children(Self::frame_lines(&crop))
            .child(div().absolute().inset_0().opacity(self.grid_opacity.value(now)).children(self.grid_lines(&crop)))
            .children(Self::handle_visuals(&crop))
            .child(
                canvas(
                    |bounds, window, _| (bounds, window.insert_hitbox(bounds, HitboxBehavior::Normal)),
                    move |_, (bounds, hitbox): (Bounds<Pixels>, Hitbox), window, _| {
                        if let Some(cursor) = cursor {
                            window.set_cursor_style(cursor, &hitbox);
                        }
                        let origin = bounds.origin;

                        let down = overlay.clone();
                        let down_hitbox = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble
                                || event.button != MouseButton::Left
                                || !down_hitbox.is_hovered(window)
                            {
                                return;
                            }
                            let location = local_point(event.position, origin);
                            if down.update(cx, |this, _| this.handle_mouse_down(location)) {
                                cx.stop_propagation();
                            }
                        });

                        let moved = overlay.clone();
                        let moved_hitbox = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            let location = local_point(event.position, origin);
                            let is_tracking = moved.read(cx).pan_press_location.is_some();
                            if is_tracking && phase == DispatchPhase::Capture {
                                moved.update(cx, |this, cx| {
                                    if event.pressed_button == Some(MouseButton::Left) {
                                        this.handle_mouse_drag(location, window, cx);
                                    } else {
                                        this.handle_mouse_released(window, cx);
                                    }
                                });
                                cx.stop_propagation();
                            } else if !is_tracking && phase == DispatchPhase::Bubble {
                                let is_hovered = moved_hitbox.is_hovered(window);
                                moved.update(cx, |this, cx| {
                                    if is_hovered {
                                        this.handle_hover(location, cx);
                                    } else if this.hover_edge != EditorCropOverlayUIViewResizeEdge::None {
                                        this.hover_edge = EditorCropOverlayUIViewResizeEdge::None;
                                        cx.notify();
                                    }
                                });
                            }
                        });

                        let up = overlay.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                            if phase != DispatchPhase::Capture
                                || event.button != MouseButton::Left
                                || up.read(cx).pan_press_location.is_none()
                            {
                                return;
                            }
                            up.update(cx, |this, cx| this.handle_mouse_released(window, cx));
                            cx.stop_propagation();
                        });

                        let pinched = overlay.clone();
                        window.on_mouse_event(move |event: &PinchEvent, phase, window, cx| {
                            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                                return;
                            }
                            let location = local_point(event.position, origin);
                            let handled = pinched.update(cx, |this, cx| {
                                this.handle_pinch(location, event.delta, event.phase, window, cx)
                            });
                            if handled {
                                cx.stop_propagation();
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}
