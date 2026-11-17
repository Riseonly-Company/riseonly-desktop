use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Corners, DispatchPhase, Edges, ElementId, Entity, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    PinchEvent, Pixels, Point, Radians, Render, RenderImage, RenderOnce, ScrollWheelEvent, Size, Styled, Task,
    TouchPhase, TransformationMatrix, Window, canvas, div, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out, spring};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription};
use crate::modules::media_picker::editor::editor_crop_auto_fit::{
    EDITOR_CROP_AUTO_FIT, EDITOR_CROP_AUTO_FIT_CANCEL, EDITOR_CROP_AUTO_FIT_DID_FINISH,
    EDITOR_CROP_AUTO_FIT_PRESENTATION_DID_CHANGE, EDITOR_CROP_FINE_ROTATION_DID_FINISH,
    EDITOR_CROP_FINE_ROTATION_INTERACTION, EDITOR_CROP_FRAME_IN_STAGE, EDITOR_CROP_GEOMETRY_DID_CHANGE,
    EDITOR_CROP_GEOMETRY_REQUEST, EDITOR_CROP_IMAGE_TRANSFORM_DID_CHANGE, EDITOR_CROP_INTERACTION_CHANGED,
    EDITOR_CROP_ORIENTATION_CHANGE, EDITOR_CROP_ORIENTATION_DID_FINISH, EDITOR_CROP_PINCH, EDITOR_CROP_RESET,
    EditorCropAutoFit, EditorCropAutoFitCancel, EditorCropAutoFitCompletion, EditorCropAutoFitCompletionStatus,
    EditorCropAutoFitPlanner, EditorCropAutoFitPresentationUpdate, EditorCropAutoFitPresentationUpdatePhase,
    EditorCropAutoFitRequest, EditorCropAutoFitTransition, EditorCropAutoFitTransitionSample,
    EditorCropFineRotationCompletion, EditorCropFineRotationInteractionUpdate, EditorCropFrameUpdate,
    EditorCropGeometryRequest, EditorCropGeometryUpdate, EditorCropInteractionUpdate, EditorCropOrientationCompletion,
    EditorCropOrientationRequest, EditorCropPinchUpdate, EditorCropPinchUpdatePhase, EditorCropResetRequest,
};
use crate::modules::media_picker::editor::editor_crop_auto_fit_log::EditorCropAutoFitLog;
use crate::modules::media_picker::editor::editor_crop_overlay::EditorCropOverlayUIView;
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::media_picker::editor::editor_image_rect::EditorImageRect;
use crate::modules::media_picker::editor::editor_mode::EditorMode;
use crate::modules::media_picker::editor::editor_overlay_canvas::{
    EditorOverlayCanvas, EditorOverlayCanvasImageOrientation, EditorOverlayCanvasInteractionMode,
};
use crate::modules::media_picker::editor::editor_state::{EditorDrawStrokeTool, EditorImageState};

pub type EditorImageStageImageRectCallback = Rc<dyn Fn(EditorImageRect, &mut App)>;
pub type EditorImageStageStateCallback = Rc<dyn Fn(EditorImageState, &mut App)>;
pub type EditorImageStageEntityCallback = Rc<dyn Fn(Uuid, &mut App)>;
pub type EditorImageStagePointCallback = Rc<dyn Fn(Point<f32>, &mut App)>;
/// callback with the focused entity id or none
pub type EditorImageStageFocusCallback = Rc<dyn Fn(Option<Uuid>, &mut App)>;
pub type EditorImageStageDragChanged = Rc<dyn Fn(f32, &mut App)>;
/// callback with the translation and the velocity
pub type EditorImageStageDragEnded = Rc<dyn Fn(f32, f32, &mut App)>;
pub type EditorImageStageSingleTap = Rc<dyn Fn(&mut App)>;

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<f32> {
    Bounds { origin: point(x, y), size: size(width, height) }
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

fn pixels_point(position: Point<Pixels>) -> Point<f32> {
    point(f32::from(position.x), f32::from(position.y))
}

fn image_size(image: &RenderImage) -> Size<f32> {
    let pixels = image.size(0);
    size(pixels.width.0 as f32, pixels.height.0 as f32)
}

#[derive(IntoElement)]
pub struct EditorImageStage {
    pub id: ElementId,
    pub crop_coordination_id: Uuid,
    pub rendered_image: Arc<RenderImage>,
    pub state: Binding<EditorImageState>,
    pub mode: EditorMode,
    pub brush_tool: EditorDrawStrokeTool,
    pub brush_color: Hsla,
    pub brush_width_fraction: f32,
    pub allows_pinch_zoom: bool,
    pub side_insets: Edges<f32>,
    pub on_image_rect: EditorImageStageImageRectCallback,
    pub on_request_edit_text: EditorImageStageEntityCallback,
    pub on_request_create_text: EditorImageStagePointCallback,
    pub on_request_create_tag: EditorImageStagePointCallback,
    pub on_text_focus_changed: EditorImageStageFocusCallback,
    pub allows_vertical_dismiss_drag: bool,
    pub on_vertical_dismiss_drag_changed: Option<EditorImageStageDragChanged>,
    pub on_vertical_dismiss_drag_ended: Option<EditorImageStageDragEnded>,
    pub allows_horizontal_page_drag: bool,
    pub on_horizontal_page_drag_changed: Option<EditorImageStageDragChanged>,
    pub on_horizontal_page_drag_ended: Option<EditorImageStageDragEnded>,
    pub on_single_tap: Option<EditorImageStageSingleTap>,
}

impl EditorImageStage {
    /// the parameters without a default
    /// the rest start at theirs and have setters below
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        crop_coordination_id: Uuid,
        rendered_image: Arc<RenderImage>,
        state: Binding<EditorImageState>,
        mode: EditorMode,
        brush_tool: EditorDrawStrokeTool,
        brush_color: Hsla,
        brush_width_fraction: f32,
        allows_pinch_zoom: bool,
        side_insets: Edges<f32>,
        on_image_rect: EditorImageStageImageRectCallback,
        on_request_edit_text: EditorImageStageEntityCallback,
        on_request_create_text: EditorImageStagePointCallback,
    ) -> Self {
        Self {
            id: id.into(),
            crop_coordination_id,
            rendered_image,
            state,
            mode,
            brush_tool,
            brush_color,
            brush_width_fraction,
            allows_pinch_zoom,
            side_insets,
            on_image_rect,
            on_request_edit_text,
            on_request_create_text,
            on_request_create_tag: Rc::new(|_: Point<f32>, _: &mut App| {}),
            on_text_focus_changed: Rc::new(|_: Option<Uuid>, _: &mut App| {}),
            allows_vertical_dismiss_drag: false,
            on_vertical_dismiss_drag_changed: None,
            on_vertical_dismiss_drag_ended: None,
            allows_horizontal_page_drag: false,
            on_horizontal_page_drag_changed: None,
            on_horizontal_page_drag_ended: None,
            on_single_tap: None,
        }
    }

    pub fn on_request_create_tag(mut self, on_request_create_tag: EditorImageStagePointCallback) -> Self {
        self.on_request_create_tag = on_request_create_tag;
        self
    }

    pub fn on_text_focus_changed(mut self, on_text_focus_changed: EditorImageStageFocusCallback) -> Self {
        self.on_text_focus_changed = on_text_focus_changed;
        self
    }

    pub fn allows_vertical_dismiss_drag(mut self, allows_vertical_dismiss_drag: bool) -> Self {
        self.allows_vertical_dismiss_drag = allows_vertical_dismiss_drag;
        self
    }

    pub fn on_vertical_dismiss_drag_changed(mut self, callback: Option<EditorImageStageDragChanged>) -> Self {
        self.on_vertical_dismiss_drag_changed = callback;
        self
    }

    pub fn on_vertical_dismiss_drag_ended(mut self, callback: Option<EditorImageStageDragEnded>) -> Self {
        self.on_vertical_dismiss_drag_ended = callback;
        self
    }

    pub fn allows_horizontal_page_drag(mut self, allows_horizontal_page_drag: bool) -> Self {
        self.allows_horizontal_page_drag = allows_horizontal_page_drag;
        self
    }

    pub fn on_horizontal_page_drag_changed(mut self, callback: Option<EditorImageStageDragChanged>) -> Self {
        self.on_horizontal_page_drag_changed = callback;
        self
    }

    pub fn on_horizontal_page_drag_ended(mut self, callback: Option<EditorImageStageDragEnded>) -> Self {
        self.on_horizontal_page_drag_ended = callback;
        self
    }

    pub fn on_single_tap(mut self, on_single_tap: Option<EditorImageStageSingleTap>) -> Self {
        self.on_single_tap = on_single_tap;
        self
    }
}

impl RenderOnce for EditorImageStage {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        EditorImageStageRepresentable {
            id: self.id,
            crop_coordination_id: self.crop_coordination_id,
            rendered_image: self.rendered_image,
            state: self.state,
            mode: self.mode,
            brush_tool: self.brush_tool,
            brush_color: self.brush_color,
            brush_width_fraction: self.brush_width_fraction,
            allows_pinch_zoom: self.allows_pinch_zoom,
            side_insets: self.side_insets,
            allows_vertical_dismiss_drag: self.allows_vertical_dismiss_drag,
            on_vertical_dismiss_drag_changed: self.on_vertical_dismiss_drag_changed,
            on_vertical_dismiss_drag_ended: self.on_vertical_dismiss_drag_ended,
            allows_horizontal_page_drag: self.allows_horizontal_page_drag,
            on_horizontal_page_drag_changed: self.on_horizontal_page_drag_changed,
            on_horizontal_page_drag_ended: self.on_horizontal_page_drag_ended,
            on_single_tap: self.on_single_tap,
            on_image_rect: self.on_image_rect,
            on_request_edit_text: self.on_request_edit_text,
            on_request_create_text: self.on_request_create_text,
            on_request_create_tag: self.on_request_create_tag,
            on_text_focus_changed: self.on_text_focus_changed,
        }
    }
}

#[derive(IntoElement)]
pub struct EditorImageStageRepresentable {
    pub id: ElementId,
    pub crop_coordination_id: Uuid,
    pub rendered_image: Arc<RenderImage>,
    pub state: Binding<EditorImageState>,
    pub mode: EditorMode,
    pub brush_tool: EditorDrawStrokeTool,
    pub brush_color: Hsla,
    pub brush_width_fraction: f32,
    pub allows_pinch_zoom: bool,
    pub side_insets: Edges<f32>,
    pub allows_vertical_dismiss_drag: bool,
    pub on_vertical_dismiss_drag_changed: Option<EditorImageStageDragChanged>,
    pub on_vertical_dismiss_drag_ended: Option<EditorImageStageDragEnded>,
    pub allows_horizontal_page_drag: bool,
    pub on_horizontal_page_drag_changed: Option<EditorImageStageDragChanged>,
    pub on_horizontal_page_drag_ended: Option<EditorImageStageDragEnded>,
    pub on_single_tap: Option<EditorImageStageSingleTap>,
    pub on_image_rect: EditorImageStageImageRectCallback,
    pub on_request_edit_text: EditorImageStageEntityCallback,
    pub on_request_create_text: EditorImageStagePointCallback,
    pub on_request_create_tag: EditorImageStagePointCallback,
    pub on_text_focus_changed: EditorImageStageFocusCallback,
}

impl EditorImageStageRepresentable {
    fn make_ui_view(&self, window: &mut Window, cx: &mut App) -> Entity<EditorImageStageUIView> {
        window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            let mut view = EditorImageStageUIView::new(cx);
            view.crop_coordination_id = self.crop_coordination_id;
            view.on_image_rect_changed = Some(Rc::clone(&self.on_image_rect));
            view.on_request_edit_text = Some(Rc::clone(&self.on_request_edit_text));
            view.on_request_create_text = Some(Rc::clone(&self.on_request_create_text));
            view.on_request_create_tag = Some(Rc::clone(&self.on_request_create_tag));
            view.on_text_focus_changed = Some(Rc::clone(&self.on_text_focus_changed));
            let state = self.state.clone();
            let handle: AnyWindowHandle = window.window_handle();
            view.on_state_changed = Some(Rc::new(move |new_value: EditorImageState, cx: &mut App| {
                let state = state.clone();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| state.set(new_value, window, cx));
                });
            }));
            view.set_brush_tool(self.brush_tool, cx);
            view.set_brush_color(self.brush_color, cx);
            view.set_brush_width_fraction(self.brush_width_fraction, cx);
            view.set_allows_pinch_zoom(self.allows_pinch_zoom, cx);
            view.set_allows_vertical_dismiss_drag(self.allows_vertical_dismiss_drag, cx);
            view.on_vertical_dismiss_drag_changed = self.on_vertical_dismiss_drag_changed.clone();
            view.on_vertical_dismiss_drag_ended = self.on_vertical_dismiss_drag_ended.clone();
            view.set_allows_horizontal_page_drag(self.allows_horizontal_page_drag, cx);
            view.on_horizontal_page_drag_changed = self.on_horizontal_page_drag_changed.clone();
            view.on_horizontal_page_drag_ended = self.on_horizontal_page_drag_ended.clone();
            view.on_single_tap = self.on_single_tap.clone();
            view.update_side_insets(self.side_insets, false, cx);
            view.set(Arc::clone(&self.rendered_image), self.state.get(cx), self.mode, cx);
            view
        })
    }

    fn update_ui_view(&self, ui_view: &Entity<EditorImageStageUIView>, cx: &mut App) {
        let state = self.state.get(cx);
        ui_view.update(cx, |ui_view, cx| {
            ui_view.crop_coordination_id = self.crop_coordination_id;
            ui_view.on_image_rect_changed = Some(Rc::clone(&self.on_image_rect));
            ui_view.on_request_edit_text = Some(Rc::clone(&self.on_request_edit_text));
            ui_view.on_request_create_text = Some(Rc::clone(&self.on_request_create_text));
            ui_view.on_request_create_tag = Some(Rc::clone(&self.on_request_create_tag));
            ui_view.on_text_focus_changed = Some(Rc::clone(&self.on_text_focus_changed));
            ui_view.set_brush_tool(self.brush_tool, cx);
            ui_view.set_brush_color(self.brush_color, cx);
            ui_view.set_brush_width_fraction(self.brush_width_fraction, cx);
            ui_view.set_allows_pinch_zoom(self.allows_pinch_zoom, cx);
            ui_view.set_allows_vertical_dismiss_drag(self.allows_vertical_dismiss_drag, cx);
            ui_view.on_vertical_dismiss_drag_changed = self.on_vertical_dismiss_drag_changed.clone();
            ui_view.on_vertical_dismiss_drag_ended = self.on_vertical_dismiss_drag_ended.clone();
            ui_view.set_allows_horizontal_page_drag(self.allows_horizontal_page_drag, cx);
            ui_view.on_horizontal_page_drag_changed = self.on_horizontal_page_drag_changed.clone();
            ui_view.on_horizontal_page_drag_ended = self.on_horizontal_page_drag_ended.clone();
            ui_view.on_single_tap = self.on_single_tap.clone();
            let insets_changed = ui_view.side_insets != self.side_insets;
            ui_view.update_side_insets(self.side_insets, insets_changed && self.mode != EditorMode::Crop, cx);
            ui_view.set(Arc::clone(&self.rendered_image), state, self.mode, cx);
        });
    }
}

impl RenderOnce for EditorImageStageRepresentable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let ui_view = self.make_ui_view(window, cx);
        self.update_ui_view(&ui_view, cx);
        div().size_full().child(ui_view)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EditorImageStageUIViewPendingCropViewportReset {
    request_id: Option<Uuid>,
    target_zoom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EditorImageStageUIViewPendingCropOrientationChange {
    id: Uuid,
    rotation_steps: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EditorImageStageUIViewPendingFineRotationCompletion {
    id: Uuid,
    degrees: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorImageStageUIViewPanAxis {
    Vertical,
    Horizontal,
}

/// state of the dismiss pan gesture
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorImageStagePanState {
    Began,
    Changed,
    Ended,
    Cancelled,
}

/// what a press turned into once it left the tap slop
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorImageStagePressGesture {
    Undecided,
    DismissPan,
    ScrollPan,
    Unclaimed,
}

/// one left button press, the twin of a touch for the tap and dismiss and scroll recognizers
struct EditorImageStagePress {
    start: Point<Pixels>,
    last: Point<Pixels>,
    click_count: usize,
    gesture: EditorImageStagePressGesture,
    samples: Vec<(MotionTime, Point<f32>)>,
}

struct EditorImageStageZoomAnimation {
    from_zoom: f32,
    to_zoom: f32,
    from_offset: Point<f32>,
    to_offset: Point<f32>,
    progress: Animated<f32>,
}

/// offset moving back inside its limits after an overscroll
struct EditorImageStageBounceBack {
    from_offset: Point<f32>,
    to_offset: Point<f32>,
    progress: Animated<f32>,
}

struct EditorImageStageDeceleration {
    velocity: Point<f32>,
    last_tick: MotionTime,
}

/// scroll view state the stage reads and writes
/// motion lives on the stage view because each step reports to the delegate methods there
struct EditorImageStageScrollView {
    frame_size: Size<f32>,
    content_size: Size<f32>,
    content_offset: Point<f32>,
    content_inset: Edges<f32>,
    zoom_scale: f32,
    minimum_zoom_scale: f32,
    maximum_zoom_scale: f32,
    bounces_zoom: bool,
    bounces: bool,
    always_bounce_vertical: bool,
    always_bounce_horizontal: bool,
    is_scroll_enabled: bool,
    is_user_interaction_enabled: bool,
    is_pan_enabled: bool,
    is_pinch_enabled: bool,
    is_zooming: bool,
}

impl EditorImageStageScrollView {
    fn new() -> Self {
        Self {
            frame_size: Size::default(),
            content_size: Size::default(),
            content_offset: Point::default(),
            content_inset: Edges::default(),
            zoom_scale: 1.0,
            minimum_zoom_scale: 1.0,
            maximum_zoom_scale: 4.0,
            bounces_zoom: true,
            bounces: true,
            always_bounce_vertical: false,
            always_bounce_horizontal: false,
            is_scroll_enabled: true,
            is_user_interaction_enabled: true,
            is_pan_enabled: true,
            is_pinch_enabled: true,
            is_zooming: false,
        }
    }

    fn minimum_offset(&self) -> Point<f32> {
        point(-self.content_inset.left, -self.content_inset.top)
    }

    fn maximum_offset(&self) -> Point<f32> {
        let minimum = self.minimum_offset();
        point(
            minimum.x.max(self.content_size.width - self.frame_size.width + self.content_inset.right),
            minimum.y.max(self.content_size.height - self.frame_size.height + self.content_inset.bottom),
        )
    }

    fn clamped_offset(&self, offset: Point<f32>) -> Point<f32> {
        let minimum = self.minimum_offset();
        let maximum = self.maximum_offset();
        point(offset.x.max(minimum.x).min(maximum.x), offset.y.max(minimum.y).min(maximum.y))
    }

    fn scrolls_horizontally(&self) -> bool {
        self.always_bounce_horizontal
            || self.content_size.width + self.content_inset.left + self.content_inset.right
                > self.frame_size.width + 0.5
    }

    fn scrolls_vertically(&self) -> bool {
        self.always_bounce_vertical
            || self.content_size.height + self.content_inset.top + self.content_inset.bottom
                > self.frame_size.height + 0.5
    }

    fn clamped_zoom(&self, zoom: f32) -> f32 {
        zoom.max(self.minimum_zoom_scale).min(self.maximum_zoom_scale.max(self.minimum_zoom_scale))
    }
}

/// everything the orientation animation does when it lands
#[derive(Clone, Copy)]
struct EditorImageStageOrientationCompletion {
    should_start_viewport_reset: bool,
    entering_crop: bool,
    orientation_change: Option<EditorImageStageUIViewPendingCropOrientationChange>,
}

/// one crop notification on its way to the main thread
enum EditorImageStageNotification {
    AutoFit(EditorCropAutoFitRequest),
    AutoFitCancel(EditorCropAutoFitCancel),
    Frame(EditorCropFrameUpdate),
    Pinch(EditorCropPinchUpdate),
    Reset(EditorCropResetRequest),
    GeometryRequest(EditorCropGeometryRequest),
    Orientation(EditorCropOrientationRequest),
    FineRotation(EditorCropFineRotationInteractionUpdate),
}

pub struct EditorImageStageUIView {
    scroll_view: EditorImageStageScrollView,
    image_container_size: Size<f32>,
    image_view_size: Size<f32>,
    image_view_transform: [f32; 4],
    image_view_display_rotation: Animated<f32>,
    image_view_display_mirror: Animated<f32>,
    image_container_display_frame: Animated<Bounds<Pixels>>,
    pending_layout_animation: Option<MotionSpec>,
    needs_layout: bool,
    bounds_size: Size<f32>,
    stage_origin_in_window: Point<Pixels>,

    overlay: Entity<EditorOverlayCanvas>,
    rect_report_token: u64,
    last_reported_image_rect: Option<EditorImageRect>,
    last_published_crop_geometry: Option<(EditorImageRect, Bounds<f32>)>,
    pub crop_coordination_id: Uuid,

    pub on_image_rect_changed: Option<EditorImageStageImageRectCallback>,
    pub on_state_changed: Option<EditorImageStageStateCallback>,
    pub on_request_edit_text: Option<EditorImageStageEntityCallback>,
    pub on_request_create_text: Option<EditorImageStagePointCallback>,
    pub on_request_create_tag: Option<EditorImageStagePointCallback>,
    pub on_text_focus_changed: Option<EditorImageStageFocusCallback>,

    pub allows_pinch_zoom: bool,

    #[allow(dead_code)]
    crop_observers: Vec<RiseSubscription>,
    #[allow(dead_code)]
    crop_notification_delivery: Task<()>,
    is_crop_auto_fit_animating: bool,
    is_stopping_crop_auto_fit: bool,
    is_applying_crop_auto_fit_tick: bool,
    pending_auto_fit_id: Option<Uuid>,
    pending_auto_fit_content_center: Option<Point<f32>>,
    pending_auto_fit_relative_crop: Option<Bounds<f32>>,
    pending_auto_fit_transition: Option<EditorCropAutoFitTransition>,
    pending_auto_fit_started_at: Option<MotionTime>,
    crop_auto_fit_display_link: bool,
    display_link_scheduled: bool,
    crop_frame_in_stage: Bounds<f32>,
    external_pinch_start_zoom: Option<f32>,
    is_enforcing_crop_coverage: bool,
    is_clamping_scroll_offset: bool,
    is_user_dragging_crop: bool,
    is_user_zooming_crop: bool,
    is_crop_reset_requested: bool,
    crop_reset_request_id: Option<Uuid>,
    pending_crop_viewport_reset: Option<EditorImageStageUIViewPendingCropViewportReset>,
    is_crop_reset_transition_running: bool,
    is_crop_viewport_ready: bool,
    pending_crop_orientation_change: Option<EditorImageStageUIViewPendingCropOrientationChange>,
    is_fine_rotation_interaction_active: bool,
    pending_fine_rotation_completion: Option<EditorImageStageUIViewPendingFineRotationCompletion>,
    should_reproject_crop_frame_after_layout: bool,
    should_reset_preview_viewport: bool,
    pub side_insets: Edges<f32>,

    pub brush_tool: EditorDrawStrokeTool,
    pub brush_color: Hsla,
    pub brush_width_fraction: f32,

    pub allows_vertical_dismiss_drag: bool,
    pub on_vertical_dismiss_drag_changed: Option<EditorImageStageDragChanged>,
    pub on_vertical_dismiss_drag_ended: Option<EditorImageStageDragEnded>,

    pub allows_horizontal_page_drag: bool,
    pub on_horizontal_page_drag_changed: Option<EditorImageStageDragChanged>,
    pub on_horizontal_page_drag_ended: Option<EditorImageStageDragEnded>,

    pub on_single_tap: Option<EditorImageStageSingleTap>,

    single_tap_started_on_focus: bool,
    single_tap_started_in_initial: bool,
    #[allow(dead_code)]
    pending_single_tap: Option<Task<()>>,

    dismiss_pan_is_enabled: bool,
    pan_axis: Option<EditorImageStageUIViewPanAxis>,
    horizontal_activation_x: f32,

    press: Option<EditorImageStagePress>,
    is_scroll_dragging: bool,
    zoom_animation: Option<EditorImageStageZoomAnimation>,
    bounce_back: Option<EditorImageStageBounceBack>,
    deceleration: Option<EditorImageStageDeceleration>,
    zoom_gesture_raw_scale: Option<f32>,
    #[allow(dead_code)]
    zoom_gesture_end: Option<Task<()>>,
    #[allow(dead_code)]
    wheel_scroll_end: Option<Task<()>>,
    orientation_animation: Option<(Task<()>, EditorImageStageOrientationCompletion)>,
    crop_reset_fallback_task: Option<Task<()>>,

    image: Option<Arc<RenderImage>>,
    current_mode: EditorMode,
    current_state: EditorImageState,

    last_applied_steps: i64,
    last_applied_mirror: bool,
}

impl EditorImageStageUIView {
    const SINGLE_TAP_FAILURE_DELAY: Duration = Duration::from_millis(350);
    const TAP_SLOP: f32 = 4.0;
    const WHEEL_ZOOM_PIXELS_PER_E: f32 = 240.0;
    const WHEEL_LINE_HEIGHT: Pixels = px(20.0);
    const WHEEL_GESTURE_END_DELAY: Duration = Duration::from_millis(140);
    const LAYOUT_SPRING_RESPONSE: f64 = 0.38;
    const INSETS_SPRING_RESPONSE: f64 = 0.34;
    const LAYOUT_SPRING_DAMPING: f64 = 0.84;
    const ZOOM_ANIMATION_DURATION: f64 = 0.3;
    const DECELERATION_RATE: f32 = 0.998;
    const DECELERATION_START_SPEED: f32 = 50.0;
    const DECELERATION_STOP_SPEED: f32 = 5.0;
    const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
    const OVERSCROLL_RESISTANCE: f32 = 0.55;
    const ZOOM_BOUNCE_RESISTANCE: f32 = 0.35;

    pub fn new(cx: &mut Context<Self>) -> Self {
        let overlay = cx.new(EditorOverlayCanvas::new);
        {
            let this = cx.weak_entity();
            let on_state_changed: EditorImageStageStateCallback = Rc::new(move |new_state: EditorImageState, cx: &mut App| {
                let this = this.clone();
                cx.defer(move |cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.current_state = new_state.clone();
                        if let Some(on_state_changed) = this.on_state_changed.clone() {
                            on_state_changed(new_state, cx);
                        }
                    });
                });
            });
            let this = cx.weak_entity();
            let on_request_edit_text: EditorImageStageEntityCallback = Rc::new(move |id: Uuid, cx: &mut App| {
                let this = this.clone();
                cx.defer(move |cx| {
                    let callback = this.upgrade().and_then(|this| this.read(cx).on_request_edit_text.clone());
                    if let Some(callback) = callback {
                        callback(id, cx);
                    }
                });
            });
            let this = cx.weak_entity();
            let on_request_create_text: EditorImageStagePointCallback = Rc::new(move |location: Point<f32>, cx: &mut App| {
                let this = this.clone();
                cx.defer(move |cx| {
                    let callback = this.upgrade().and_then(|this| this.read(cx).on_request_create_text.clone());
                    if let Some(callback) = callback {
                        callback(location, cx);
                    }
                });
            });
            let this = cx.weak_entity();
            let on_request_create_tag: EditorImageStagePointCallback = Rc::new(move |location: Point<f32>, cx: &mut App| {
                let this = this.clone();
                cx.defer(move |cx| {
                    let callback = this.upgrade().and_then(|this| this.read(cx).on_request_create_tag.clone());
                    if let Some(callback) = callback {
                        callback(location, cx);
                    }
                });
            });
            let this = cx.weak_entity();
            let on_text_focus_changed: EditorImageStageFocusCallback = Rc::new(move |id: Option<Uuid>, cx: &mut App| {
                let this = this.clone();
                cx.defer(move |cx| {
                    let callback = this.upgrade().and_then(|this| this.read(cx).on_text_focus_changed.clone());
                    if let Some(callback) = callback {
                        callback(id, cx);
                    }
                });
            });
            overlay.update(cx, |overlay, _| {
                overlay.on_state_changed = Some(on_state_changed);
                overlay.on_request_edit_text = Some(on_request_edit_text);
                overlay.on_request_create_text = Some(on_request_create_text);
                overlay.on_request_create_tag = Some(on_request_create_tag);
                overlay.on_text_focus_changed = Some(on_text_focus_changed);
            });
        }

        let (crop_observers, crop_notification_delivery) = Self::observe_crop_notifications(cx);

        let mut view = Self {
            scroll_view: EditorImageStageScrollView::new(),
            image_container_size: Size::default(),
            image_view_size: Size::default(),
            image_view_transform: [1.0, 0.0, 0.0, 1.0],
            image_view_display_rotation: Animated::new(0.0),
            image_view_display_mirror: Animated::new(1.0),
            image_container_display_frame: Animated::new(Bounds::default()),
            pending_layout_animation: None,
            needs_layout: true,
            bounds_size: Size::default(),
            stage_origin_in_window: Point::default(),
            overlay,
            rect_report_token: 0,
            last_reported_image_rect: None,
            last_published_crop_geometry: None,
            crop_coordination_id: Uuid::new_v4(),
            on_image_rect_changed: None,
            on_state_changed: None,
            on_request_edit_text: None,
            on_request_create_text: None,
            on_request_create_tag: None,
            on_text_focus_changed: None,
            allows_pinch_zoom: true,
            crop_observers,
            crop_notification_delivery,
            is_crop_auto_fit_animating: false,
            is_stopping_crop_auto_fit: false,
            is_applying_crop_auto_fit_tick: false,
            pending_auto_fit_id: None,
            pending_auto_fit_content_center: None,
            pending_auto_fit_relative_crop: None,
            pending_auto_fit_transition: None,
            pending_auto_fit_started_at: None,
            crop_auto_fit_display_link: false,
            display_link_scheduled: false,
            crop_frame_in_stage: Bounds::default(),
            external_pinch_start_zoom: None,
            is_enforcing_crop_coverage: false,
            is_clamping_scroll_offset: false,
            is_user_dragging_crop: false,
            is_user_zooming_crop: false,
            is_crop_reset_requested: false,
            crop_reset_request_id: None,
            pending_crop_viewport_reset: None,
            is_crop_reset_transition_running: false,
            is_crop_viewport_ready: false,
            pending_crop_orientation_change: None,
            is_fine_rotation_interaction_active: false,
            pending_fine_rotation_completion: None,
            should_reproject_crop_frame_after_layout: false,
            should_reset_preview_viewport: false,
            side_insets: Edges::default(),
            brush_tool: EditorDrawStrokeTool::Pen,
            brush_color: gpui::white(),
            brush_width_fraction: 0.05,
            allows_vertical_dismiss_drag: false,
            on_vertical_dismiss_drag_changed: None,
            on_vertical_dismiss_drag_ended: None,
            allows_horizontal_page_drag: false,
            on_horizontal_page_drag_changed: None,
            on_horizontal_page_drag_ended: None,
            on_single_tap: None,
            single_tap_started_on_focus: false,
            single_tap_started_in_initial: false,
            pending_single_tap: None,
            dismiss_pan_is_enabled: false,
            pan_axis: None,
            horizontal_activation_x: 0.0,
            press: None,
            is_scroll_dragging: false,
            zoom_animation: None,
            bounce_back: None,
            deceleration: None,
            zoom_gesture_raw_scale: None,
            zoom_gesture_end: None,
            wheel_scroll_end: None,
            orientation_animation: None,
            crop_reset_fallback_task: None,
            image: None,
            current_mode: EditorMode::Initial,
            current_state: EditorImageState::empty(None, Size::default()),
            last_applied_steps: 0,
            last_applied_mirror: false,
        };
        view.apply_scroll_interaction_policy(cx);
        view
    }

    pub fn overlay(&self) -> &Entity<EditorOverlayCanvas> {
        &self.overlay
    }

    pub(super) fn zoom_scale(&self) -> f32 {
        self.scroll_view.zoom_scale
    }

    pub(super) fn image_container_size(&self) -> Size<f32> {
        self.image_container_size
    }

    fn observe_crop_notification<P: Clone + Send + Sync + 'static>(
        name: RiseNotificationName,
        sender: &mpsc::UnboundedSender<EditorImageStageNotification>,
        wrap: fn(P) -> EditorImageStageNotification,
    ) -> RiseSubscription {
        let sender = sender.clone();
        RiseAppEvents::default_center().observe(name, move |note| {
            if let Some(payload) = note.payload::<P>() {
                let _ = sender.unbounded_send(wrap(payload.clone()));
            }
        })
    }

    fn observe_crop_notifications(cx: &mut Context<Self>) -> (Vec<RiseSubscription>, Task<()>) {
        let (sender, mut inbox) = mpsc::unbounded::<EditorImageStageNotification>();
        let observers = vec![
            Self::observe_crop_notification(EDITOR_CROP_AUTO_FIT, &sender, EditorImageStageNotification::AutoFit),
            Self::observe_crop_notification(
                EDITOR_CROP_AUTO_FIT_CANCEL,
                &sender,
                EditorImageStageNotification::AutoFitCancel,
            ),
            Self::observe_crop_notification(EDITOR_CROP_FRAME_IN_STAGE, &sender, EditorImageStageNotification::Frame),
            Self::observe_crop_notification(EDITOR_CROP_PINCH, &sender, EditorImageStageNotification::Pinch),
            Self::observe_crop_notification(EDITOR_CROP_RESET, &sender, EditorImageStageNotification::Reset),
            Self::observe_crop_notification(
                EDITOR_CROP_GEOMETRY_REQUEST,
                &sender,
                EditorImageStageNotification::GeometryRequest,
            ),
            Self::observe_crop_notification(
                EDITOR_CROP_ORIENTATION_CHANGE,
                &sender,
                EditorImageStageNotification::Orientation,
            ),
            Self::observe_crop_notification(
                EDITOR_CROP_FINE_ROTATION_INTERACTION,
                &sender,
                EditorImageStageNotification::FineRotation,
            ),
        ];
        let delivery = cx.spawn(async move |this, cx| {
            while let Some(notification) = inbox.next().await {
                if this.update(cx, |view, cx| view.handle_crop_notification(notification, cx)).is_err() {
                    break;
                }
            }
        });
        (observers, delivery)
    }

    fn handle_crop_notification(&mut self, notification: EditorImageStageNotification, cx: &mut Context<Self>) {
        match notification {
            EditorImageStageNotification::AutoFit(request) => {
                if self.current_mode != EditorMode::Crop {
                    return;
                }
                if request.scope_id == self.crop_coordination_id {
                    self.animate_auto_fit_for_crop(request.id, request.source_rect, Some(request.target_rect), cx);
                }
            }
            EditorImageStageNotification::AutoFitCancel(cancel) => {
                if cancel.scope_id != self.crop_coordination_id || Some(cancel.id) != self.pending_auto_fit_id {
                    return;
                }
                self.cancel_crop_auto_fit_animation(cx);
            }
            EditorImageStageNotification::Frame(update) => {
                if update.scope_id != self.crop_coordination_id {
                    return;
                }
                self.crop_frame_in_stage = update.rect;
                if self.current_mode == EditorMode::Crop {
                    self.enforce_crop_coverage(false, cx);
                }
            }
            EditorImageStageNotification::Pinch(update) => {
                if update.scope_id != self.crop_coordination_id || self.current_mode != EditorMode::Crop {
                    return;
                }
                self.handle_external_crop_pinch(update, cx);
            }
            EditorImageStageNotification::Reset(request) => {
                if request.scope_id != self.crop_coordination_id || self.current_mode != EditorMode::Crop {
                    return;
                }
                self.cancel_crop_auto_fit_animation(cx);
                self.is_crop_reset_requested = true;
                self.crop_reset_request_id = Some(request.id);
                self.is_crop_viewport_ready = false;
                self.pending_crop_orientation_change = None;
                self.pending_fine_rotation_completion = None;
                self.is_fine_rotation_interaction_active = false;
                self.scroll_view.is_user_interaction_enabled = false;
                self.start_requested_crop_reset_if_ready(cx);
                cx.notify();
            }
            EditorImageStageNotification::GeometryRequest(request) => {
                if request.scope_id != self.crop_coordination_id
                    || self.current_mode != EditorMode::Crop
                    || !self.is_crop_viewport_ready
                {
                    return;
                }
                self.last_published_crop_geometry = None;
                self.publish_crop_geometry(None, None);
            }
            EditorImageStageNotification::Orientation(request) => {
                if request.scope_id != self.crop_coordination_id || self.current_mode != EditorMode::Crop {
                    return;
                }
                self.pending_crop_orientation_change = Some(EditorImageStageUIViewPendingCropOrientationChange {
                    id: request.id,
                    rotation_steps: request.rotation_steps,
                });
            }
            EditorImageStageNotification::FineRotation(update) => {
                if update.scope_id != self.crop_coordination_id || self.current_mode != EditorMode::Crop {
                    return;
                }
                self.is_fine_rotation_interaction_active = update.is_active;
                if update.is_active {
                    self.pending_fine_rotation_completion = None;
                } else if let Some(completion_id) = update.completion_id {
                    self.pending_fine_rotation_completion = Some(EditorImageStageUIViewPendingFineRotationCompletion {
                        id: completion_id,
                        degrees: update.degrees,
                    });
                    self.complete_fine_rotation_if_ready(cx);
                }
            }
        }
    }

    pub fn set_allows_pinch_zoom(&mut self, allows_pinch_zoom: bool, cx: &mut Context<Self>) {
        self.allows_pinch_zoom = allows_pinch_zoom;
        self.apply_scroll_interaction_policy(cx);
    }

    pub fn set_brush_tool(&mut self, brush_tool: EditorDrawStrokeTool, cx: &mut Context<Self>) {
        self.brush_tool = brush_tool;
        self.overlay.update(cx, |overlay, cx| overlay.set_brush_tool(brush_tool, cx));
    }

    pub fn set_brush_color(&mut self, brush_color: Hsla, cx: &mut Context<Self>) {
        self.brush_color = brush_color;
        self.overlay.update(cx, |overlay, _| overlay.brush_color = brush_color);
    }

    pub fn set_brush_width_fraction(&mut self, brush_width_fraction: f32, cx: &mut Context<Self>) {
        self.brush_width_fraction = brush_width_fraction;
        self.overlay.update(cx, |overlay, _| overlay.brush_width_fraction = brush_width_fraction);
    }

    pub fn set_allows_vertical_dismiss_drag(&mut self, allows_vertical_dismiss_drag: bool, cx: &mut Context<Self>) {
        self.allows_vertical_dismiss_drag = allows_vertical_dismiss_drag;
        self.apply_scroll_interaction_policy(cx);
    }

    pub fn set_allows_horizontal_page_drag(&mut self, allows_horizontal_page_drag: bool, cx: &mut Context<Self>) {
        self.allows_horizontal_page_drag = allows_horizontal_page_drag;
        self.apply_scroll_interaction_policy(cx);
    }

    pub fn update_side_insets(&mut self, new: Edges<f32>, animated: bool, cx: &mut Context<Self>) {
        if new == self.side_insets {
            return;
        }
        if self.is_crop_auto_fit_animating {
            self.cancel_crop_auto_fit_animation(cx);
        }
        if self.current_mode == EditorMode::Crop && self.is_crop_viewport_ready {
            self.should_reproject_crop_frame_after_layout = true;
        }
        self.side_insets = new;
        if animated {
            self.pending_layout_animation = Some(spring(Self::INSETS_SPRING_RESPONSE, Self::LAYOUT_SPRING_DAMPING));
            self.set_needs_layout(cx);
            self.layout_if_needed(cx);
        } else {
            self.set_needs_layout(cx);
        }
    }

    pub(super) fn set_needs_layout(&mut self, cx: &mut Context<Self>) {
        self.needs_layout = true;
        cx.notify();
    }

    pub(super) fn layout_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.needs_layout {
            return;
        }
        self.needs_layout = false;
        self.layout_subviews(cx);
    }

    fn defer_on_self(cx: &mut Context<Self>, action: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = this.update(cx, action);
        });
    }

    fn bounds(&self) -> Bounds<f32> {
        rect(0.0, 0.0, self.bounds_size.width, self.bounds_size.height)
    }

    pub(super) fn set_stage_bounds(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.stage_origin_in_window = bounds.origin;
        let new_size = size(f32::from(bounds.size.width), f32::from(bounds.size.height));
        let size_changed = (new_size.width - self.bounds_size.width).abs() > 0.5
            || (new_size.height - self.bounds_size.height).abs() > 0.5;
        if size_changed {
            self.bounds_size = new_size;
            self.set_needs_layout(cx);
            if self.current_mode == EditorMode::Crop && new_size.width > 0.0 && new_size.height > 0.0 {
                Self::defer_on_self(cx, |view, cx| {
                    if view.current_mode == EditorMode::Crop && view.pending_crop_viewport_reset.is_none() {
                        view.start_crop_viewport_reset(None, false, cx);
                    }
                });
            }
        }
    }

    fn local_point(&self, position: Point<Pixels>) -> Point<f32> {
        point(
            f32::from(position.x - self.stage_origin_in_window.x),
            f32::from(position.y - self.stage_origin_in_window.y),
        )
    }

    /// converts a stage point to image container coords
    fn image_container_point(&self, stage_point: Point<f32>) -> Point<f32> {
        let zoom = self.scroll_view.zoom_scale.max(0.001);
        point(
            (stage_point.x + self.scroll_view.content_offset.x) / zoom,
            (stage_point.y + self.scroll_view.content_offset.y) / zoom,
        )
    }

    /// stage point in overlay coords
    /// canvas is laid out at the zoomed size of the container
    /// gpui has no layer scale so its points are the zoomed ones
    fn overlay_point(&self, stage_point: Point<f32>) -> Point<f32> {
        point(
            stage_point.x + self.scroll_view.content_offset.x,
            stage_point.y + self.scroll_view.content_offset.y,
        )
    }

    /// center of the image view in stage coords
    /// container is laid out from the content origin and the image sits at its middle
    fn image_view_center_in_stage(&self) -> Point<f32> {
        let zoom = self.scroll_view.zoom_scale;
        point(
            self.image_container_size.width * zoom / 2.0 - self.scroll_view.content_offset.x,
            self.image_container_size.height * zoom / 2.0 - self.scroll_view.content_offset.y,
        )
    }

    fn image_view_point_to_stage(&self, image_point: Point<f32>) -> Point<f32> {
        let zoom = self.scroll_view.zoom_scale;
        let [a, b, c, d] = self.image_view_transform;
        let center = self.image_view_center_in_stage();
        let x = image_point.x - self.image_view_size.width / 2.0;
        let y = image_point.y - self.image_view_size.height / 2.0;
        point(center.x + zoom * (a * x + c * y), center.y + zoom * (b * x + d * y))
    }

    fn image_view_point_from_stage(&self, stage_point: Point<f32>) -> Point<f32> {
        let zoom = self.scroll_view.zoom_scale.max(0.001);
        let [a, b, c, d] = self.image_view_transform;
        let center = self.image_view_center_in_stage();
        let x = (stage_point.x - center.x) / zoom;
        let y = (stage_point.y - center.y) / zoom;
        let determinant = a * d - b * c;
        if determinant.abs() <= f32::EPSILON {
            return point(self.image_view_size.width / 2.0, self.image_view_size.height / 2.0);
        }
        point(
            (d * x - c * y) / determinant + self.image_view_size.width / 2.0,
            (a * y - b * x) / determinant + self.image_view_size.height / 2.0,
        )
    }

    pub fn set(&mut self, image: Arc<RenderImage>, state: EditorImageState, mode: EditorMode, cx: &mut Context<Self>) {
        let image_changed = !self.image.as_ref().is_some_and(|current| Arc::ptr_eq(current, &image));
        let image_geometry_changed = Self::display_geometry_differs(self.image.as_deref(), &image);
        let mode_changed = self.current_mode != mode;
        let prev_steps = Self::effective_rotation_steps(&self.current_state, self.current_mode);
        let prev_mirrored = Self::effective_mirror(&self.current_state, self.current_mode);
        let prev_free_rotation = Self::effective_free_rotation(&self.current_state, self.current_mode);
        let next_steps = Self::effective_rotation_steps(&state, mode);
        let next_mirrored = Self::effective_mirror(&state, mode);
        let next_free_rotation = Self::effective_free_rotation(&state, mode);

        let step_changed = prev_steps != next_steps;
        let mirror_changed = prev_mirrored != next_mirrored;
        let fine_rotation_changed =
            (prev_free_rotation - next_free_rotation).abs() > EditorImageLayout::GEOMETRY_EPSILON_DEGREES;
        let orientation_transform_changed = step_changed || mirror_changed || fine_rotation_changed;
        let crop_anchor: Option<(Point<f32>, Point<f32>)> = if !step_changed
            && fine_rotation_changed
            && self.current_mode == EditorMode::Crop
            && self.crop_frame_in_stage.size.width > 1.0
            && self.crop_frame_in_stage.size.height > 1.0
        {
            let screen_point = point(mid_x(&self.crop_frame_in_stage), mid_y(&self.crop_frame_in_stage));
            Some((self.image_view_point_from_stage(screen_point), screen_point))
        } else {
            None
        };

        if self.is_crop_auto_fit_animating && (image_geometry_changed || mode_changed || orientation_transform_changed)
        {
            self.cancel_crop_auto_fit_animation(cx);
        }

        if self.is_crop_auto_fit_animating {
            EditorCropAutoFitLog::log(
                "set.skipped-layout",
                &[("reason", &"autoFitAnimating" as &dyn std::fmt::Debug), ("zoom", &self.scroll_view.zoom_scale)],
            );
            self.current_mode = mode;
            self.overlay.update(cx, |overlay, cx| {
                if image_changed {
                    overlay.set_blur_source_image(Some(Arc::clone(&image)), cx);
                }
                overlay.apply(state.clone(), cx);
            });
            self.current_state = state;
            if image_changed {
                self.image = Some(image);
            }
            cx.notify();
            return;
        }

        self.image = Some(Arc::clone(&image));
        let previous_mode = self.current_mode;
        let previous_crop = self.current_state.crop_rect_relative;
        self.current_mode = mode;
        self.current_state = state.clone();
        if mode != EditorMode::Crop && previous_mode == EditorMode::Crop {
            self.cancel_pending_crop_reset(cx);
            self.pending_crop_orientation_change = None;
            self.pending_fine_rotation_completion = None;
            self.is_fine_rotation_interaction_active = false;
        }
        let entering_crop = mode_changed && mode == EditorMode::Crop;
        let crop_was_reset = mode == EditorMode::Crop
            && !EditorCropAutoFitPlanner::is_effectively_full(previous_crop)
            && EditorCropAutoFitPlanner::is_effectively_full(state.crop_rect_relative);

        if entering_crop || crop_was_reset {
            self.crop_frame_in_stage = Bounds::default();
            self.is_crop_viewport_ready = false;
            self.scroll_view.minimum_zoom_scale = EditorCropAutoFit::CROP_MINIMUM_ZOOM_SCALE;
        }

        if mode_changed && previous_mode == EditorMode::Crop {
            self.should_reset_preview_viewport = true;
            self.crop_frame_in_stage = Bounds::default();
            self.is_crop_viewport_ready = false;
            self.should_reproject_crop_frame_after_layout = false;
        }
        if step_changed && mode == EditorMode::Crop && !mode_changed {
            self.crop_frame_in_stage = Bounds::default();
        }
        self.apply_scroll_zoom_limits(mode, cx);

        if image_geometry_changed {
            self.set_zoom_scale(1.0, false, cx);
        }
        self.overlay.update(cx, |overlay, cx| {
            if image_changed {
                overlay.set_blur_source_image(Some(Arc::clone(&image)), cx);
            }
            overlay.apply(state.clone(), cx);
            overlay.set_is_hidden(mode == EditorMode::Crop, cx);
            match mode {
                EditorMode::Draw => overlay.set_interaction_mode(EditorOverlayCanvasInteractionMode::Draw, cx),
                EditorMode::Text { .. } => {
                    overlay.set_interaction_mode(EditorOverlayCanvasInteractionMode::TextPlacement, cx);
                    if mode_changed || overlay.selected_entity_id().is_some() {
                        overlay.select_entity(None, cx);
                    }
                }
                EditorMode::TagPeople => {
                    overlay.set_interaction_mode(EditorOverlayCanvasInteractionMode::TagPlacement, cx)
                }
                _ => overlay.set_interaction_mode(EditorOverlayCanvasInteractionMode::EntityEdit, cx),
            }
            let editing_text_entity_id = match mode {
                EditorMode::Text { entity_id } => entity_id,
                _ => None,
            };
            overlay.set_editing_text_entity_id(editing_text_entity_id, cx);
        });

        if mode_changed {
            if matches!(mode, EditorMode::Crop | EditorMode::Adjust) {
                self.overlay.update(cx, |overlay, cx| overlay.select_entity(None, cx));
            }
            EditorCropAutoFitLog::log(
                "set.modeChanged",
                &[("from", &previous_mode as &dyn std::fmt::Debug), ("to", &mode)],
            );
            if previous_mode == EditorMode::Crop {
                self.set_zoom_scale(1.0, false, cx);
            }
        }

        let reset_state_is_ready = mode == EditorMode::Crop
            && next_steps == 0
            && !next_mirrored
            && next_free_rotation.abs() <= EditorImageLayout::GEOMETRY_EPSILON_DEGREES
            && EditorCropAutoFitPlanner::is_effectively_full(state.crop_rect_relative);
        let requested_reset_can_start = self.is_crop_reset_requested && reset_state_is_ready;
        let should_start_viewport_reset =
            !self.is_crop_reset_transition_running && (entering_crop || crop_was_reset || requested_reset_can_start);
        if should_start_viewport_reset {
            self.is_crop_reset_transition_running = true;
            self.is_crop_viewport_ready = false;
        }

        let orientation_change =
            self.pending_crop_orientation_change.filter(|pending| pending.rotation_steps == next_steps);
        let should_animate_orientation = orientation_transform_changed
            && !self.is_fine_rotation_interaction_active
            && self.pending_fine_rotation_completion.is_none();
        let orientation_animation_completed = EditorImageStageOrientationCompletion {
            should_start_viewport_reset,
            entering_crop,
            orientation_change,
        };
        let orientation_animation_started =
            self.apply_orientation_transform(should_animate_orientation, orientation_animation_completed, cx);

        let needs_layout = image_geometry_changed || mode_changed || step_changed;
        if needs_layout {
            EditorCropAutoFitLog::log(
                "set.layout",
                &[
                    ("imageChanged", &image_geometry_changed as &dyn std::fmt::Debug),
                    ("modeChanged", &mode_changed),
                    ("orientationChanged", &orientation_transform_changed),
                    ("zoom", &self.scroll_view.zoom_scale),
                ],
            );
            if step_changed && self.scroll_view.zoom_scale != 1.0 {
                self.set_zoom_scale(1.0, false, cx);
            }
            if step_changed {
                self.pending_layout_animation = Some(spring(Self::LAYOUT_SPRING_RESPONSE, Self::LAYOUT_SPRING_DAMPING));
                self.set_needs_layout(cx);
                self.layout_if_needed(cx);
            } else {
                self.set_needs_layout(cx);
            }
        }

        if !orientation_animation_started {
            if should_start_viewport_reset {
                Self::defer_on_self(cx, move |view, cx| {
                    view.orientation_animation_did_complete(orientation_animation_completed, cx);
                });
            } else if orientation_transform_changed {
                self.orientation_animation_did_complete(orientation_animation_completed, cx);
            }
        }

        if !should_start_viewport_reset && (fine_rotation_changed || mirror_changed) {
            self.enforce_crop_coverage(false, cx);
            if let Some(crop_anchor) = crop_anchor {
                self.preserve_crop_anchor(crop_anchor, cx);
                self.clamp_scroll_offset_to_cover_crop_frame(cx);
            }
            self.notify_image_rect(None, cx);
        }
        self.complete_fine_rotation_if_ready(cx);
        cx.notify();
    }

    fn orientation_animation_did_complete(
        &mut self,
        completion: EditorImageStageOrientationCompletion,
        cx: &mut Context<Self>,
    ) {
        if completion.should_start_viewport_reset {
            let request_id = if completion.entering_crop { None } else { self.crop_reset_request_id };
            self.start_crop_viewport_reset(request_id, true, cx);
        } else if let Some(orientation_change) = completion.orientation_change {
            self.finish_crop_orientation_change(orientation_change, cx);
        }
    }

    fn effective_rotation_steps(state: &EditorImageState, mode: EditorMode) -> i64 {
        if mode == EditorMode::Crop { state.rotation_steps.rem_euclid(4) } else { 0 }
    }

    fn effective_mirror(state: &EditorImageState, mode: EditorMode) -> bool {
        mode == EditorMode::Crop && state.is_mirrored
    }

    fn effective_free_rotation(state: &EditorImageState, mode: EditorMode) -> f64 {
        if mode == EditorMode::Crop { state.free_rotation_degrees } else { 0.0 }
    }

    fn apply_orientation_transform(
        &mut self,
        animate: bool,
        completion: EditorImageStageOrientationCompletion,
        cx: &mut Context<Self>,
    ) -> bool {
        let steps = Self::effective_rotation_steps(&self.current_state, self.current_mode);
        let mirrored = Self::effective_mirror(&self.current_state, self.current_mode);
        let free_rotation = Self::effective_free_rotation(&self.current_state, self.current_mode);
        let step_angle = steps as f32 * std::f32::consts::FRAC_PI_2;
        let free_angle = free_rotation as f32 * std::f32::consts::PI / 180.0;
        let cs = step_angle.cos();
        let ss = step_angle.sin();
        let cf = free_angle.cos();
        let sf = free_angle.sin();
        let transform = if mirrored {
            [-cf * cs - sf * ss, -sf * cs + cf * ss, cf * ss - sf * cs, sf * ss + cf * cs]
        } else {
            let angle = step_angle + free_angle;
            [angle.cos(), angle.sin(), -angle.sin(), angle.cos()]
        };
        let display_angle = if mirrored { free_angle - step_angle } else { step_angle + free_angle };
        let display_mirror = if mirrored { -1.0 } else { 1.0 };
        let current_angle = *self.image_view_display_rotation.target();
        let two_pi = std::f32::consts::TAU;
        let display_angle =
            current_angle + (display_angle - current_angle + std::f32::consts::PI).rem_euclid(two_pi) - std::f32::consts::PI;

        let steps_or_mirror_changed = steps != self.last_applied_steps || mirrored != self.last_applied_mirror;
        self.last_applied_steps = steps;
        self.last_applied_mirror = mirrored;
        self.image_view_transform = transform;

        if animate && steps_or_mirror_changed {
            let now = RiseClock::monotonic();
            let reduce = reduce_motion(cx);
            let motion = spring(Self::LAYOUT_SPRING_RESPONSE, Self::LAYOUT_SPRING_DAMPING);
            self.image_view_display_rotation.set(display_angle, motion, now, reduce);
            self.image_view_display_mirror.set(display_mirror, motion, now, reduce);
            if let Some((_, replaced)) = self.orientation_animation.take() {
                Self::defer_on_self(cx, move |view, cx| view.orientation_animation_did_complete(replaced, cx));
            }
            let delay = Duration::from_secs_f64(Self::LAYOUT_SPRING_RESPONSE);
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |view, cx| {
                    if let Some((_, completion)) = view.orientation_animation.take() {
                        view.orientation_animation_did_complete(completion, cx);
                    }
                });
            });
            self.orientation_animation = Some((task, completion));
            cx.notify();
            true
        } else {
            self.image_view_display_rotation.jump(display_angle);
            self.image_view_display_mirror.jump(display_mirror);
            cx.notify();
            false
        }
    }

    fn layout_subviews(&mut self, cx: &mut Context<Self>) {
        self.scroll_view.frame_size = self.bounds_size;
        let Some(image) = self.image.clone() else {
            return;
        };
        if self.bounds_size.width <= 0.0 || self.bounds_size.height <= 0.0 {
            return;
        }
        let source_size = image_size(&image);

        let avail = size(
            (self.bounds_size.width - self.side_insets.left - self.side_insets.right).max(0.0),
            (self.bounds_size.height - self.side_insets.top - self.side_insets.bottom).max(0.0),
        );
        let layout_size = EditorImageLayout::oriented_display_size(
            source_size,
            Self::effective_rotation_steps(&self.current_state, self.current_mode),
            0.0,
        );
        let img_size = Self::aspect_fit_size(layout_size, avail);
        let image_scale = if layout_size.width > 0.0 { img_size.width / layout_size.width } else { 1.0 };
        let natural_image_size = size(source_size.width * image_scale, source_size.height * image_scale);

        let zoom = self.scroll_view.zoom_scale.max(EditorCropAutoFit::CROP_MINIMUM_ZOOM_SCALE);
        let base_geometry_changed = (self.image_container_size.width - img_size.width).abs() > 0.5
            || (self.image_container_size.height - img_size.height).abs() > 0.5;
        let container_is_empty = self.image_container_size.width <= 0.0 || self.image_container_size.height <= 0.0;
        let can_update_base_geometry = container_is_empty
            || (!self.scroll_view.is_zooming
                && !self.is_crop_auto_fit_animating
                && self.pending_crop_viewport_reset.is_none());
        if base_geometry_changed && can_update_base_geometry {
            self.image_container_size = img_size;
        }
        self.image_view_size = natural_image_size;
        let orientation = EditorOverlayCanvasImageOrientation {
            rotation_steps: Self::effective_rotation_steps(&self.current_state, self.current_mode),
            mirrored: Self::effective_mirror(&self.current_state, self.current_mode),
            free_rotation_degrees: Self::effective_free_rotation(&self.current_state, self.current_mode) as f32,
        };
        self.overlay.update(cx, |overlay, _| overlay.image_orientation = orientation);

        let expected_content =
            size(self.image_container_size.width * zoom, self.image_container_size.height * zoom);
        let content_size_changed = (self.scroll_view.content_size.width - expected_content.width).abs() > 0.5
            || (self.scroll_view.content_size.height - expected_content.height).abs() > 0.5;
        if content_size_changed {
            EditorCropAutoFitLog::log(
                "layoutSubviews.contentSize",
                &[
                    ("from", &self.scroll_view.content_size as &dyn std::fmt::Debug),
                    ("to", &expected_content),
                    ("zoom", &zoom),
                    ("base", &img_size),
                ],
            );
        }
        if content_size_changed
            && !self.scroll_view.is_zooming
            && !self.is_crop_auto_fit_animating
            && !self.is_crop_reset_requested
            && !self.is_crop_reset_transition_running
            && self.pending_crop_viewport_reset.is_none()
        {
            self.scroll_view.content_size = expected_content;
        }

        self.update_content_insets();
        if self.current_mode != EditorMode::Crop
            && !self.scroll_view.is_zooming
            && !self.is_scroll_dragging
            && self.bounce_back.is_none()
            && self.deceleration.is_none()
            && self.zoom_animation.is_none()
            && !self.is_crop_auto_fit_animating
            && self.pending_crop_viewport_reset.is_none()
        {
            let clamped = self.scroll_view.clamped_offset(self.scroll_view.content_offset);
            self.set_content_offset(clamped, cx);
        }
        if self.should_reset_preview_viewport {
            self.should_reset_preview_viewport = false;
            let offset = point(-self.scroll_view.content_inset.left, -self.scroll_view.content_inset.top);
            self.set_content_offset(offset, cx);
        }
        if self.current_mode == EditorMode::Crop && !self.is_crop_auto_fit_animating {
            if self.should_reproject_crop_frame_after_layout && self.is_crop_viewport_ready {
                self.should_reproject_crop_frame_after_layout = false;
                self.crop_frame_in_stage = self.crop_frame_for_current_state(None);
            }
            self.enforce_crop_coverage(false, cx);
        }
        self.notify_image_rect(None, cx);
    }

    fn update_content_insets(&mut self) {
        self.scroll_view.content_inset = self.content_insets_at_zoom(self.scroll_view.zoom_scale);
    }

    fn content_insets_at_zoom(&self, zoom: f32) -> Edges<f32> {
        let zoom = zoom.max(EditorCropAutoFit::CROP_MINIMUM_ZOOM_SCALE);
        let zoomed = size(self.image_container_size.width * zoom, self.image_container_size.height * zoom);
        let avail_w = (self.bounds_size.width - self.side_insets.left - self.side_insets.right).max(0.0);
        let avail_h = (self.bounds_size.height - self.side_insets.top - self.side_insets.bottom).max(0.0);
        let extra_h = ((avail_w - zoomed.width) / 2.0).max(0.0);
        let extra_v = ((avail_h - zoomed.height) / 2.0).max(0.0);
        let centered = Edges {
            top: self.side_insets.top + extra_v,
            left: self.side_insets.left + extra_h,
            bottom: self.side_insets.bottom + extra_v,
            right: self.side_insets.right + extra_h,
        };
        self.crop_reach_insets(centered)
    }

    fn crop_reach_insets(&self, centered: Edges<f32>) -> Edges<f32> {
        let crop = self.crop_frame_in_stage;
        let usable = self.current_mode == EditorMode::Crop
            && crop.size.width > EditorCropAutoFit::MIN_CROP_EXTENT
            && crop.size.height > EditorCropAutoFit::MIN_CROP_EXTENT
            && self.bounds_size.width > 1.0
            && self.bounds_size.height > 1.0;
        if !usable {
            return centered;
        }
        Edges {
            top: centered.top.max(min_y(&crop)),
            left: centered.left.max(min_x(&crop)),
            bottom: centered.bottom.max(self.bounds_size.height - max_y(&crop)),
            right: centered.right.max(self.bounds_size.width - max_x(&crop)),
        }
    }

    fn aspect_fit_size(image: Size<f32>, fitting: Size<f32>) -> Size<f32> {
        if image.width <= 0.0 || image.height <= 0.0 || fitting.width <= 0.0 || fitting.height <= 0.0 {
            return Size::default();
        }
        let scale = (fitting.width / image.width).min(fitting.height / image.height);
        size(image.width * scale, image.height * scale)
    }

    fn display_geometry_differs(previous: Option<&RenderImage>, next: &RenderImage) -> bool {
        let Some(previous) = previous else {
            return true;
        };
        let lhs = image_size(previous);
        let rhs = image_size(next);
        if lhs.width <= 0.0 || lhs.height <= 0.0 || rhs.width <= 0.0 || rhs.height <= 0.0 {
            return lhs != rhs;
        }
        let lhs_aspect = lhs.width / lhs.height;
        let rhs_aspect = rhs.width / rhs.height;
        (lhs_aspect - rhs_aspect).abs() / lhs_aspect.max(rhs_aspect) > 0.002
    }

    fn visible_image_rect_in_stage(&self) -> Bounds<f32> {
        let width = self.image_view_size.width;
        let height = self.image_view_size.height;
        let corners = [
            self.image_view_point_to_stage(point(0.0, 0.0)),
            self.image_view_point_to_stage(point(width, 0.0)),
            self.image_view_point_to_stage(point(0.0, height)),
            self.image_view_point_to_stage(point(width, height)),
        ];
        let left = corners.iter().map(|corner| corner.x).fold(f32::MAX, f32::min);
        let right = corners.iter().map(|corner| corner.x).fold(f32::MIN, f32::max);
        let top = corners.iter().map(|corner| corner.y).fold(f32::MAX, f32::min);
        let bottom = corners.iter().map(|corner| corner.y).fold(f32::MIN, f32::max);
        rect(left, top, right - left, bottom - top)
    }

    fn base_image_viewport_in_stage(&self) -> Bounds<f32> {
        let Some(image) = self.image.as_ref() else {
            return Bounds::default();
        };
        if self.bounds_size.width <= 0.0 || self.bounds_size.height <= 0.0 {
            return Bounds::default();
        }
        let avail = size(
            (self.bounds_size.width - self.side_insets.left - self.side_insets.right).max(0.0),
            (self.bounds_size.height - self.side_insets.top - self.side_insets.bottom).max(0.0),
        );
        let layout_size = EditorImageLayout::oriented_display_size(
            image_size(image),
            Self::effective_rotation_steps(&self.current_state, self.current_mode),
            0.0,
        );
        let img_size = Self::aspect_fit_size(layout_size, avail);
        let origin_x = self.side_insets.left + ((avail.width - img_size.width) / 2.0).max(0.0);
        let origin_y = self.side_insets.top + ((avail.height - img_size.height) / 2.0).max(0.0);
        rect(origin_x, origin_y, img_size.width, img_size.height)
    }

    fn image_layout_snapshot(&self) -> EditorImageRect {
        EditorImageRect {
            rect: self.visible_image_rect_in_stage(),
            viewport_rect: self.base_image_viewport_in_stage(),
            stage_rect: self.bounds(),
            available_rect: rect(
                self.side_insets.left,
                self.side_insets.top,
                (self.bounds_size.width - self.side_insets.left - self.side_insets.right).max(0.0),
                (self.bounds_size.height - self.side_insets.top - self.side_insets.bottom).max(0.0),
            ),
            zoom: self.scroll_view.zoom_scale,
        }
    }

    fn notify_image_rect(&mut self, completed_reset_id: Option<Uuid>, cx: &mut Context<Self>) {
        let snapshot = self.image_layout_snapshot();
        if self.current_mode == EditorMode::Crop && self.is_crop_viewport_ready && !self.is_crop_auto_fit_animating {
            self.publish_crop_geometry(Some(snapshot), completed_reset_id);
        }
        if completed_reset_id.is_none() && self.last_reported_image_rect == Some(snapshot) {
            return;
        }
        self.last_reported_image_rect = Some(snapshot);
        self.rect_report_token = self.rect_report_token.wrapping_add(1);
        let token = self.rect_report_token;
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let Some(this) = this.upgrade() else {
                return;
            };
            let view = this.read(cx);
            if view.rect_report_token != token {
                return;
            }
            if let Some(on_image_rect_changed) = view.on_image_rect_changed.clone() {
                on_image_rect_changed(snapshot, cx);
            }
        });
    }

    fn publish_crop_geometry(&mut self, image_layout: Option<EditorImageRect>, completed_reset_id: Option<Uuid>) {
        if self.current_mode != EditorMode::Crop || !self.is_crop_viewport_ready {
            return;
        }
        let layout = image_layout.unwrap_or_else(|| self.image_layout_snapshot());
        if self.crop_frame_in_stage.size.width <= EditorCropAutoFit::MIN_CROP_EXTENT
            || self.crop_frame_in_stage.size.height <= EditorCropAutoFit::MIN_CROP_EXTENT
        {
            self.crop_frame_in_stage = self.crop_frame_for_current_state(Some(layout));
        }
        if self.crop_frame_in_stage.size.width <= EditorCropAutoFit::MIN_CROP_EXTENT
            || self.crop_frame_in_stage.size.height <= EditorCropAutoFit::MIN_CROP_EXTENT
        {
            return;
        }
        let published = (layout, self.crop_frame_in_stage);
        if completed_reset_id.is_none() && self.last_published_crop_geometry == Some(published) {
            return;
        }
        self.last_published_crop_geometry = Some(published);
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_GEOMETRY_DID_CHANGE,
            EditorCropGeometryUpdate {
                scope_id: self.crop_coordination_id,
                image_layout: layout,
                crop_frame: self.crop_frame_in_stage,
                completed_reset_id,
            },
        );
    }

    fn crop_frame_for_current_state(&self, image_layout: Option<EditorImageRect>) -> Bounds<f32> {
        let layout = image_layout.unwrap_or_else(|| self.image_layout_snapshot());
        let is_full_frame = EditorCropAutoFitPlanner::is_effectively_full(self.current_state.crop_rect_relative);
        let image_bounds = if is_full_frame { layout.viewport_rect } else { layout.rect };
        if image_bounds.size.width <= 0.0 || image_bounds.size.height <= 0.0 {
            return Bounds::default();
        }
        let relative = self.current_state.crop_rect_relative.unwrap_or_else(|| rect(0.0, 0.0, 1.0, 1.0));
        let projected = EditorCropAutoFitPlanner::project(relative, image_bounds);
        let clamp_bounds =
            EditorCropOverlayUIView::crop_clamp_bounds(layout.viewport_rect, layout.rect, layout.available_rect);
        EditorCropOverlayUIView::clamp(projected, clamp_bounds, 1.0)
    }

    fn is_crop_at_unit_zoom(&self) -> bool {
        let z = self.scroll_view.zoom_scale;
        (0.999..=1.001).contains(&z)
    }

    fn apply_crop_unit_zoom_scroll_lock(&mut self, cx: &mut Context<Self>) {
        if self.current_mode != EditorMode::Crop {
            return;
        }
        let at_unit = self.is_crop_at_unit_zoom();
        self.scroll_view.bounces = !at_unit;
        self.scroll_view.always_bounce_vertical = !at_unit && self.scroll_view.zoom_scale > 1.002;
        self.scroll_view.always_bounce_horizontal = self.scroll_view.always_bounce_vertical;
        if at_unit {
            let target = self.centered_crop_content_offset(self.scroll_view.zoom_scale);
            if (self.scroll_view.content_offset.x - target.x).abs() > 0.5
                || (self.scroll_view.content_offset.y - target.y).abs() > 0.5
            {
                self.set_content_offset(target, cx);
            }
        }
        self.apply_scroll_interaction_policy(cx);
    }

    fn centered_crop_content_offset(&self, zoom: f32) -> Point<f32> {
        let content_size = size(self.image_container_size.width * zoom, self.image_container_size.height * zoom);
        let available_width = (self.bounds_size.width - self.side_insets.left - self.side_insets.right).max(0.0);
        let available_height = (self.bounds_size.height - self.side_insets.top - self.side_insets.bottom).max(0.0);
        let horizontal_inset = self.side_insets.left + ((available_width - content_size.width) / 2.0).max(0.0);
        let vertical_inset = self.side_insets.top + ((available_height - content_size.height) / 2.0).max(0.0);
        point(-horizontal_inset, -vertical_inset)
    }

    fn start_crop_viewport_reset(&mut self, request_id: Option<Uuid>, animated: bool, cx: &mut Context<Self>) {
        if self.current_mode != EditorMode::Crop {
            return;
        }
        if self.is_crop_auto_fit_animating {
            self.cancel_crop_auto_fit_animation(cx);
        }
        self.is_crop_viewport_ready = false;
        self.crop_frame_in_stage = Bounds::default();
        self.scroll_view.minimum_zoom_scale = EditorCropAutoFit::CROP_MINIMUM_ZOOM_SCALE;
        self.scroll_view.maximum_zoom_scale = EditorCropAutoFit::CROP_MAXIMUM_ZOOM_SCALE;

        let target_zoom = EditorCropAutoFit::CROP_MINIMUM_ZOOM_SCALE;
        self.pending_crop_viewport_reset =
            Some(EditorImageStageUIViewPendingCropViewportReset { request_id, target_zoom });
        if animated && (self.scroll_view.zoom_scale - target_zoom).abs() > 0.001 {
            self.set_zoom_scale(target_zoom, true, cx);
            let delay = Duration::from_secs_f64(Self::ZOOM_ANIMATION_DURATION + 0.25);
            self.crop_reset_fallback_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(delay).await;
                let _ = this.update(cx, |view, cx| {
                    view.crop_reset_fallback_task = None;
                    if view.pending_crop_viewport_reset.is_some_and(|pending| pending.request_id == request_id) {
                        view.finish_crop_viewport_reset(cx);
                    }
                });
            }));
        } else {
            self.set_zoom_scale(target_zoom, false, cx);
            self.finish_crop_viewport_reset(cx);
        }
    }

    fn start_requested_crop_reset_if_ready(&mut self, cx: &mut Context<Self>) {
        let mode = self.current_mode;
        let state = &self.current_state;
        let reset_state_is_ready = mode == EditorMode::Crop
            && Self::effective_rotation_steps(state, mode) == 0
            && !Self::effective_mirror(state, mode)
            && Self::effective_free_rotation(state, mode).abs() <= EditorImageLayout::GEOMETRY_EPSILON_DEGREES
            && EditorCropAutoFitPlanner::is_effectively_full(state.crop_rect_relative);
        if !reset_state_is_ready || self.is_crop_reset_transition_running {
            return;
        }
        self.is_crop_reset_transition_running = true;
        let completion = EditorImageStageOrientationCompletion {
            should_start_viewport_reset: true,
            entering_crop: false,
            orientation_change: None,
        };
        Self::defer_on_self(cx, move |view, cx| view.orientation_animation_did_complete(completion, cx));
    }

    fn finish_crop_viewport_reset(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_crop_viewport_reset.take() else {
            return;
        };
        if (self.scroll_view.zoom_scale - pending.target_zoom).abs() > 0.001 {
            self.set_zoom_scale(pending.target_zoom, false, cx);
        }
        self.is_crop_reset_requested = false;
        self.crop_reset_request_id = None;
        self.is_crop_reset_transition_running = false;
        self.set_needs_layout(cx);
        self.layout_if_needed(cx);
        self.update_content_insets();
        let offset = self.centered_crop_content_offset(pending.target_zoom);
        self.set_content_offset(offset, cx);
        self.crop_frame_in_stage = self.crop_frame_for_current_state(None);
        self.enforce_crop_coverage(false, cx);
        self.is_crop_viewport_ready = true;
        self.last_published_crop_geometry = None;
        self.scroll_view.is_user_interaction_enabled = true;
        self.apply_scroll_interaction_policy(cx);
        self.notify_image_rect(pending.request_id, cx);
    }

    fn preserve_crop_anchor(&mut self, anchor: (Point<f32>, Point<f32>), cx: &mut Context<Self>) {
        let (image_point, screen_point) = anchor;
        let projected = self.image_view_point_to_stage(image_point);
        let mut offset = self.scroll_view.content_offset;
        offset.x += projected.x - screen_point.x;
        offset.y += projected.y - screen_point.y;
        self.set_content_offset(offset, cx);
    }

    fn cancel_pending_crop_reset(&mut self, cx: &mut Context<Self>) {
        self.pending_crop_viewport_reset = None;
        self.is_crop_reset_requested = false;
        self.crop_reset_request_id = None;
        self.is_crop_reset_transition_running = false;
        self.is_crop_viewport_ready = false;
        self.scroll_view.is_user_interaction_enabled = true;
        self.apply_scroll_interaction_policy(cx);
    }

    fn finish_crop_orientation_change(
        &mut self,
        pending: EditorImageStageUIViewPendingCropOrientationChange,
        cx: &mut Context<Self>,
    ) {
        if self.current_mode != EditorMode::Crop
            || self.pending_crop_orientation_change.map(|current| current.id) != Some(pending.id)
            || Self::effective_rotation_steps(&self.current_state, self.current_mode) != pending.rotation_steps
        {
            return;
        }
        self.pending_crop_orientation_change = None;
        self.set_needs_layout(cx);
        self.layout_if_needed(cx);
        self.crop_frame_in_stage = self.crop_frame_for_current_state(None);
        self.enforce_crop_coverage(false, cx);
        self.notify_image_rect(None, cx);
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_ORIENTATION_DID_FINISH,
            EditorCropOrientationCompletion { scope_id: self.crop_coordination_id, id: pending.id },
        );
    }

    fn complete_fine_rotation_if_ready(&mut self, cx: &mut Context<Self>) {
        if self.current_mode != EditorMode::Crop || self.is_fine_rotation_interaction_active {
            return;
        }
        let Some(pending) = self.pending_fine_rotation_completion else {
            return;
        };
        let free_rotation = Self::effective_free_rotation(&self.current_state, self.current_mode);
        if (free_rotation - pending.degrees).abs() > EditorImageLayout::GEOMETRY_EPSILON_DEGREES {
            return;
        }
        self.pending_fine_rotation_completion = None;
        self.set_needs_layout(cx);
        self.layout_if_needed(cx);
        self.enforce_crop_coverage(false, cx);
        self.notify_image_rect(None, cx);
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_FINE_ROTATION_DID_FINISH,
            EditorCropFineRotationCompletion { scope_id: self.crop_coordination_id, id: pending.id },
        );
    }

    fn orientation_angle(&self) -> f32 {
        let step_angle = Self::effective_rotation_steps(&self.current_state, self.current_mode) as f32
            * std::f32::consts::FRAC_PI_2;
        let free_angle = Self::effective_free_rotation(&self.current_state, self.current_mode) as f32
            * std::f32::consts::PI
            / 180.0;
        step_angle + free_angle
    }

    fn minimum_zoom_covering_crop_frame(&self) -> f32 {
        let crop = self.crop_frame_in_stage;
        let base_image_size = self.image_view_size;
        if crop.size.width <= 0.0
            || crop.size.height <= 0.0
            || base_image_size.width <= 0.0
            || base_image_size.height <= 0.0
        {
            return EditorCropAutoFit::CROP_MINIMUM_ZOOM_SCALE;
        }
        let coverage = EditorCropAutoFitPlanner::minimum_zoom_covering(
            crop.size,
            base_image_size,
            self.orientation_angle(),
            0.0,
        );
        let free_rotation = Self::effective_free_rotation(&self.current_state, self.current_mode);
        let safety = if free_rotation.abs() > EditorImageLayout::GEOMETRY_EPSILON_DEGREES { 1.002 } else { 1.0 };
        EditorCropAutoFit::CROP_MINIMUM_ZOOM_SCALE.max(coverage * safety)
    }

    fn enforce_crop_coverage(&mut self, animated: bool, cx: &mut Context<Self>) {
        if self.current_mode != EditorMode::Crop
            || self.is_crop_auto_fit_animating
            || self.is_enforcing_crop_coverage
        {
            return;
        }
        if self.crop_frame_in_stage.size.width <= EditorCropAutoFit::MIN_CROP_EXTENT
            || self.crop_frame_in_stage.size.height <= EditorCropAutoFit::MIN_CROP_EXTENT
        {
            return;
        }
        self.is_enforcing_crop_coverage = true;

        let min_zoom = self.minimum_zoom_covering_crop_frame();
        self.scroll_view.minimum_zoom_scale = min_zoom;

        if self.scroll_view.zoom_scale < min_zoom - 0.001 {
            self.set_zoom_scale(min_zoom, animated, cx);
        }

        for _ in 0..4 {
            let before = self.scroll_view.content_offset;
            self.clamp_scroll_offset_to_cover_crop_frame(cx);
            let after = self.scroll_view.content_offset;
            if (before.x - after.x).abs() < 0.5 && (before.y - after.y).abs() < 0.5 {
                break;
            }
        }

        self.apply_crop_unit_zoom_scroll_lock(cx);
        self.is_enforcing_crop_coverage = false;
    }

    fn clamp_scroll_offset_to_cover_crop_frame(&mut self, cx: &mut Context<Self>) {
        if self.is_clamping_scroll_offset {
            return;
        }
        let crop = self.crop_frame_in_stage;
        if crop.size.width <= 0.0 || crop.size.height <= 0.0 {
            return;
        }

        let zoom = self.scroll_view.zoom_scale.max(0.001);
        let angle = self.orientation_angle();
        let u = point(angle.cos(), angle.sin());
        let v = point(-angle.sin(), angle.cos());
        let corners = [
            point(min_x(&crop), min_y(&crop)),
            point(max_x(&crop), min_y(&crop)),
            point(min_x(&crop), max_y(&crop)),
            point(max_x(&crop), max_y(&crop)),
        ];
        let dot = |p: Point<f32>, axis: Point<f32>| p.x * axis.x + p.y * axis.y;
        let min_u = corners.iter().map(|corner| dot(*corner, u)).fold(f32::MAX, f32::min);
        let max_u = corners.iter().map(|corner| dot(*corner, u)).fold(f32::MIN, f32::max);
        let min_v = corners.iter().map(|corner| dot(*corner, v)).fold(f32::MAX, f32::min);
        let max_v = corners.iter().map(|corner| dot(*corner, v)).fold(f32::MIN, f32::max);
        let center = self.image_view_center_in_stage();
        let half_width = self.image_view_size.width * zoom / 2.0;
        let half_height = self.image_view_size.height * zoom / 2.0;
        let min_center_u = max_u - half_width;
        let max_center_u = min_u + half_width;
        let min_center_v = max_v - half_height;
        let max_center_v = min_v + half_height;
        if min_center_u > max_center_u || min_center_v > max_center_v {
            return;
        }
        let center_u = dot(center, u).max(min_center_u).min(max_center_u);
        let center_v = dot(center, v).max(min_center_v).min(max_center_v);
        let target_center = point(u.x * center_u + v.x * center_v, u.y * center_u + v.y * center_v);
        let image_translation = point(target_center.x - center.x, target_center.y - center.y);
        let mut offset = self.scroll_view.content_offset;
        offset.x -= image_translation.x;
        offset.y -= image_translation.y;

        let offset = self.scroll_view.clamped_offset(offset);
        self.is_clamping_scroll_offset = true;
        self.set_content_offset(offset, cx);
        self.is_clamping_scroll_offset = false;
    }

    fn apply_scroll_zoom_limits(&mut self, mode: EditorMode, cx: &mut Context<Self>) {
        if mode == EditorMode::Crop {
            self.scroll_view.minimum_zoom_scale = self.minimum_zoom_covering_crop_frame();
            self.scroll_view.maximum_zoom_scale = EditorCropAutoFit::CROP_MAXIMUM_ZOOM_SCALE;
        } else {
            self.scroll_view.minimum_zoom_scale = 1.0;
            self.scroll_view.maximum_zoom_scale = 4.0;
        }
        self.apply_scroll_interaction_policy(cx);
    }

    fn apply_scroll_interaction_policy(&mut self, cx: &mut Context<Self>) {
        let crop_mode = self.current_mode == EditorMode::Crop;
        let zoom_enabled = self.allows_pinch_zoom;
        let at_unit_zoom = self.scroll_view.zoom_scale <= 1.002;
        let dismiss_pan_active =
            (self.allows_vertical_dismiss_drag || self.allows_horizontal_page_drag) && !crop_mode && at_unit_zoom;

        self.dismiss_pan_is_enabled = dismiss_pan_active;
        self.scroll_view.is_pinch_enabled = zoom_enabled;
        self.scroll_view.bounces_zoom = zoom_enabled && !crop_mode;
        if crop_mode && zoom_enabled {
            self.scroll_view.is_scroll_enabled = true;
            self.scroll_view.bounces = false;
            self.scroll_view.always_bounce_vertical = false;
            self.scroll_view.always_bounce_horizontal = false;
            self.scroll_view.is_pan_enabled = true;
        } else if dismiss_pan_active {
            self.scroll_view.is_scroll_enabled = false;
            self.scroll_view.bounces = false;
            self.scroll_view.always_bounce_vertical = false;
            self.scroll_view.always_bounce_horizontal = false;
            self.scroll_view.is_pan_enabled = false;
        } else {
            self.scroll_view.is_scroll_enabled = zoom_enabled;
            self.scroll_view.bounces = zoom_enabled;
            self.scroll_view.always_bounce_vertical = false;
            self.scroll_view.always_bounce_horizontal = false;
            self.scroll_view.is_pan_enabled = zoom_enabled;
        }
        if !dismiss_pan_active
            && let Some(press) = self.press.as_mut()
            && press.gesture == EditorImageStagePressGesture::DismissPan
        {
            press.gesture = EditorImageStagePressGesture::Unclaimed;
            let translation = pixels_point(press.last - press.start);
            self.handle_dismiss_pan(EditorImageStagePanState::Cancelled, translation, Point::default(), cx);
        }
    }

    /// one recognizer owns both axes, down closes the editor and sideways turns the page
    /// axis locks once and stays for the whole gesture
    fn handle_dismiss_pan(
        &mut self,
        state: EditorImageStagePanState,
        translation: Point<f32>,
        velocity: Point<f32>,
        cx: &mut Context<Self>,
    ) {
        match state {
            EditorImageStagePanState::Began => {
                self.pan_axis = None;
            }
            EditorImageStagePanState::Changed => {
                if self.pan_axis.is_none() {
                    if self.allows_vertical_dismiss_drag
                        && translation.y > 12.0
                        && translation.y > translation.x.abs() * 0.72
                    {
                        self.pan_axis = Some(EditorImageStageUIViewPanAxis::Vertical);
                    } else if self.allows_horizontal_page_drag
                        && translation.x.abs() > 12.0
                        && translation.x.abs() > translation.y.abs() * 1.2
                    {
                        self.pan_axis = Some(EditorImageStageUIViewPanAxis::Horizontal);
                        self.horizontal_activation_x = if translation.x > 0.0 { 12.0 } else { -12.0 };
                    } else {
                        return;
                    }
                }
                match self.pan_axis {
                    Some(EditorImageStageUIViewPanAxis::Vertical) => {
                        if let Some(callback) = self.on_vertical_dismiss_drag_changed.clone() {
                            callback(translation.y.max(0.0), cx);
                        }
                    }
                    Some(EditorImageStageUIViewPanAxis::Horizontal) => {
                        if let Some(callback) = self.on_horizontal_page_drag_changed.clone() {
                            callback(translation.x - self.horizontal_activation_x, cx);
                        }
                    }
                    None => {}
                }
            }
            EditorImageStagePanState::Ended | EditorImageStagePanState::Cancelled => {
                let axis = self.pan_axis.take();
                match axis {
                    Some(EditorImageStageUIViewPanAxis::Vertical) => {
                        if let Some(callback) = self.on_vertical_dismiss_drag_ended.clone() {
                            callback(translation.y.max(0.0), velocity.y, cx);
                        }
                    }
                    Some(EditorImageStageUIViewPanAxis::Horizontal) => {
                        let cancelled = state == EditorImageStagePanState::Cancelled;
                        let dx = if cancelled { 0.0 } else { translation.x - self.horizontal_activation_x };
                        if let Some(callback) = self.on_horizontal_page_drag_ended.clone() {
                            callback(dx, if cancelled { 0.0 } else { velocity.x }, cx);
                        }
                    }
                    None => {}
                }
            }
        }
    }

    fn handle_single_tap(&mut self, cx: &mut Context<Self>) {
        if self.current_mode != EditorMode::Initial {
            return;
        }
        if self.single_tap_started_on_focus {
            if self.overlay.read(cx).selected_entity_id().is_some() {
                self.overlay.update(cx, |overlay, cx| overlay.select_entity(None, cx));
            }
            return;
        }
        if let Some(on_single_tap) = self.on_single_tap.clone() {
            on_single_tap(cx);
        }
    }

    fn single_tap_should_begin(&self, stage_point: Point<f32>, cx: &App) -> bool {
        if (self.on_single_tap.is_none() && !self.single_tap_started_on_focus)
            || !self.single_tap_started_in_initial
            || self.current_mode != EditorMode::Initial
        {
            return false;
        }
        !self.overlay.read(cx).contains_entity(self.overlay_point(stage_point), cx)
    }

    fn dismiss_pan_should_begin(&self, stage_point: Point<f32>, cx: &App) -> bool {
        if !self.dismiss_pan_is_enabled
            || !(self.allows_vertical_dismiss_drag || self.allows_horizontal_page_drag)
            || self.current_mode != EditorMode::Initial
        {
            return false;
        }
        if self.overlay.read(cx).contains_entity(self.overlay_point(stage_point), cx) {
            return false;
        }
        self.scroll_view.zoom_scale <= 1.002
    }

    fn single_tap_should_receive(&mut self, cx: &App) {
        self.single_tap_started_on_focus = self.overlay.read(cx).selected_entity_id().is_some();
        self.single_tap_started_in_initial = self.current_mode == EditorMode::Initial;
    }

    pub fn animate_auto_fit_for_crop(
        &mut self,
        request_id: Uuid,
        crop_rect_in_stage: Bounds<f32>,
        target_rect_in_stage: Option<Bounds<f32>>,
        cx: &mut Context<Self>,
    ) {
        if self.is_crop_auto_fit_animating {
            self.cancel_crop_auto_fit_animation(cx);
        }
        if self.current_mode != EditorMode::Crop
            || !self.is_crop_viewport_ready
            || self.is_crop_reset_requested
            || self.pending_crop_viewport_reset.is_some()
        {
            EditorCropAutoFitLog::log(
                "autoFit.aborted",
                &[("animating", &self.is_crop_auto_fit_animating as &dyn std::fmt::Debug), ("mode", &self.current_mode)],
            );
            RiseAppEvents::default_center().post_with(
                EDITOR_CROP_AUTO_FIT_DID_FINISH,
                EditorCropAutoFitCompletion {
                    scope_id: self.crop_coordination_id,
                    id: request_id,
                    status: EditorCropAutoFitCompletionStatus::Cancelled,
                    projected_rect: None,
                },
            );
            return;
        }

        self.pending_auto_fit_id = Some(request_id);
        self.crop_frame_in_stage = crop_rect_in_stage;
        let image_rect = self.visible_image_rect_in_stage();
        if image_rect.size.width <= 0.0 || image_rect.size.height <= 0.0 {
            self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Cancelled, cx);
            return;
        }
        let Some(rel) = EditorCropAutoFitPlanner::relative_rect(crop_rect_in_stage, image_rect) else {
            self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Cancelled, cx);
            return;
        };
        self.apply_scroll_zoom_limits(EditorMode::Crop, cx);

        let avail_w = self.bounds_size.width - self.side_insets.left - self.side_insets.right;
        if crop_rect_in_stage.size.width > 0.5 && avail_w > 1.0 {
            let needed_zoom = self.scroll_view.zoom_scale * (avail_w / crop_rect_in_stage.size.width);
            if needed_zoom > self.scroll_view.maximum_zoom_scale {
                self.scroll_view.maximum_zoom_scale = EditorCropAutoFit::CROP_MAXIMUM_ZOOM_SCALE.min(needed_zoom * 1.05);
            }
        }

        let Some(target_frame) = target_rect_in_stage else {
            self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Cancelled, cx);
            return;
        };
        if crop_rect_in_stage.size.width <= 0.0 || crop_rect_in_stage.size.height <= 0.0 {
            self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Cancelled, cx);
            return;
        }
        let source_zoom = self.scroll_view.zoom_scale;
        let target_zoom = self
            .scroll_view
            .maximum_zoom_scale
            .min(source_zoom * target_frame.size.width / crop_rect_in_stage.size.width);
        if target_zoom < source_zoom * EditorCropAutoFit::MIN_ZOOM_GAIN {
            EditorCropAutoFitLog::log(
                "autoFit.planRejected",
                &[
                    ("crop", &crop_rect_in_stage as &dyn std::fmt::Debug),
                    ("image", &image_rect),
                    ("zoom", &self.scroll_view.zoom_scale),
                ],
            );
            self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Cancelled, cx);
            return;
        }

        let container_bounds = rect(0.0, 0.0, self.image_container_size.width, self.image_container_size.height);
        let crop_in_content = EditorCropAutoFitPlanner::rect_in_image_container(rel, container_bounds);
        if crop_in_content.size.width < 0.01 || crop_in_content.size.height < 0.01 {
            EditorCropAutoFitLog::log(
                "autoFit.badCropInContent",
                &[("cropInContent", &crop_in_content as &dyn std::fmt::Debug)],
            );
            self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Cancelled, cx);
            return;
        }

        EditorCropAutoFitLog::log(
            "autoFit.start",
            &[
                ("currentZoom", &self.scroll_view.zoom_scale as &dyn std::fmt::Debug),
                ("targetZoom", &target_zoom),
                ("crop", &crop_rect_in_stage),
                ("relative", &rel),
                ("cropInContent", &crop_in_content),
            ],
        );

        self.pending_auto_fit_content_center = Some(point(mid_x(&crop_in_content), mid_y(&crop_in_content)));
        self.pending_auto_fit_relative_crop = Some(rel);
        let transition = EditorCropAutoFitTransition {
            source_frame: crop_rect_in_stage,
            target_frame,
            source_zoom,
            target_zoom,
        };
        self.pending_auto_fit_transition = Some(transition);
        self.is_crop_auto_fit_animating = true;
        self.post_crop_auto_fit_presentation(EditorCropAutoFitPresentationUpdatePhase::Began, crop_rect_in_stage);

        if (target_zoom - source_zoom).abs() < 0.001 {
            self.apply_crop_auto_fit_sample(transition.sample(1.0), cx);
            self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Completed, cx);
            return;
        }

        self.pending_auto_fit_started_at = Some(RiseClock::monotonic());
        self.crop_auto_fit_display_link = true;
        cx.notify();
    }

    fn step_crop_auto_fit_animation(&mut self, now: MotionTime, cx: &mut Context<Self>) {
        let (Some(transition), Some(started_at)) = (self.pending_auto_fit_transition, self.pending_auto_fit_started_at)
        else {
            self.crop_auto_fit_display_link = false;
            return;
        };
        let elapsed = now.saturating_sub(started_at).as_secs_f64();
        let raw_progress = (elapsed / EditorCropAutoFit::ANIMATION_DURATION).min(1.0) as f32;
        self.apply_crop_auto_fit_sample(transition.sample(raw_progress), cx);
        if raw_progress >= 1.0 {
            self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Completed, cx);
        }
    }

    fn apply_crop_auto_fit_sample(&mut self, sample: EditorCropAutoFitTransitionSample, cx: &mut Context<Self>) {
        let (Some(center), Some(relative_crop)) =
            (self.pending_auto_fit_content_center, self.pending_auto_fit_relative_crop)
        else {
            return;
        };

        self.is_applying_crop_auto_fit_tick = true;
        self.crop_frame_in_stage = sample.crop_frame;
        self.set_zoom_scale(sample.zoom, false, cx);
        self.update_content_insets();
        self.center_scroll(
            center,
            self.scroll_view.zoom_scale,
            point(mid_x(&sample.crop_frame), mid_y(&sample.crop_frame)),
            cx,
        );
        self.apply_crop_unit_zoom_scroll_lock(cx);

        let anchored_frame = EditorCropAutoFitPlanner::project(relative_crop, self.visible_image_rect_in_stage());
        if anchored_frame.size.width > EditorCropAutoFit::MIN_CROP_EXTENT
            && anchored_frame.size.height > EditorCropAutoFit::MIN_CROP_EXTENT
        {
            self.crop_frame_in_stage = anchored_frame;
        }
        self.is_applying_crop_auto_fit_tick = false;

        self.post_crop_auto_fit_presentation(EditorCropAutoFitPresentationUpdatePhase::Changed, self.crop_frame_in_stage);
    }

    fn post_crop_auto_fit_presentation(&self, phase: EditorCropAutoFitPresentationUpdatePhase, crop_frame: Bounds<f32>) {
        let Some(request_id) = self.pending_auto_fit_id else {
            return;
        };
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_AUTO_FIT_PRESENTATION_DID_CHANGE,
            EditorCropAutoFitPresentationUpdate {
                scope_id: self.crop_coordination_id,
                id: request_id,
                phase,
                crop_frame,
            },
        );
    }

    fn finish_crop_auto_fit(&mut self, status: EditorCropAutoFitCompletionStatus, cx: &mut Context<Self>) {
        let Some(request_id) = self.pending_auto_fit_id else {
            return;
        };
        self.crop_auto_fit_display_link = false;
        EditorCropAutoFitLog::log(
            "autoFit.finish",
            &[
                ("zoom", &self.scroll_view.zoom_scale as &dyn std::fmt::Debug),
                ("contentSize", &self.scroll_view.content_size),
                ("offset", &self.scroll_view.content_offset),
            ],
        );
        let final_crop = self.crop_frame_in_stage;
        self.pending_auto_fit_id = None;
        self.pending_auto_fit_content_center = None;
        self.pending_auto_fit_relative_crop = None;
        self.pending_auto_fit_transition = None;
        self.pending_auto_fit_started_at = None;
        self.is_crop_auto_fit_animating = false;
        self.notify_image_rect(None, cx);
        let center = RiseAppEvents::default_center();
        center.post_with(
            EDITOR_CROP_AUTO_FIT_DID_FINISH,
            EditorCropAutoFitCompletion {
                scope_id: self.crop_coordination_id,
                id: request_id,
                status,
                projected_rect: Some(final_crop),
            },
        );
        center.post_with(
            EDITOR_CROP_AUTO_FIT_PRESENTATION_DID_CHANGE,
            EditorCropAutoFitPresentationUpdate {
                scope_id: self.crop_coordination_id,
                id: request_id,
                phase: EditorCropAutoFitPresentationUpdatePhase::Ended,
                crop_frame: final_crop,
            },
        );
    }

    pub fn cancel_crop_auto_fit_animation(&mut self, cx: &mut Context<Self>) {
        if !self.is_crop_auto_fit_animating {
            return;
        }
        EditorCropAutoFitLog::log("autoFit.cancelled", &[("zoom", &self.scroll_view.zoom_scale as &dyn std::fmt::Debug)]);
        self.is_stopping_crop_auto_fit = true;
        self.crop_auto_fit_display_link = false;
        self.stop_scrolling_and_zooming();
        self.update_content_insets();
        self.post_crop_auto_fit_presentation(EditorCropAutoFitPresentationUpdatePhase::Changed, self.crop_frame_in_stage);
        self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Cancelled, cx);
        Self::defer_on_self(cx, |view, _| view.is_stopping_crop_auto_fit = false);
    }

    fn center_scroll(
        &mut self,
        point_in_content: Point<f32>,
        zoom: f32,
        viewport_center: Point<f32>,
        cx: &mut Context<Self>,
    ) {
        let content_w = self.image_container_size.width * zoom;
        let content_h = self.image_container_size.height * zoom;
        if self.scroll_view.content_size.width < content_w - 0.5
            || self.scroll_view.content_size.height < content_h - 0.5
        {
            self.scroll_view.content_size = size(content_w, content_h);
        }

        let offset_x = point_in_content.x * zoom - viewport_center.x;
        let offset_y = point_in_content.y * zoom - viewport_center.y;
        let min_offset = self.scroll_view.minimum_offset();
        let max_offset = point(
            min_offset.x.max(content_w - self.scroll_view.frame_size.width + self.scroll_view.content_inset.right),
            min_offset.y.max(content_h - self.scroll_view.frame_size.height + self.scroll_view.content_inset.bottom),
        );
        let offset = point(
            offset_x.max(min_offset.x).min(max_offset.x),
            offset_y.max(min_offset.y).min(max_offset.y),
        );
        self.set_content_offset(offset, cx);
    }

    fn handle_external_crop_pinch(&mut self, update: EditorCropPinchUpdate, cx: &mut Context<Self>) {
        match update.phase {
            EditorCropPinchUpdatePhase::Began => {
                self.cancel_crop_auto_fit_animation(cx);
                self.external_pinch_start_zoom = Some(self.scroll_view.zoom_scale);
            }
            EditorCropPinchUpdatePhase::Changed => {
                let Some(start_zoom) = self.external_pinch_start_zoom else {
                    return;
                };
                let target_zoom = self
                    .scroll_view
                    .maximum_zoom_scale
                    .min(self.scroll_view.minimum_zoom_scale.max(start_zoom * update.scale));
                let location = self.image_container_point(update.center);
                let width = self.scroll_view.frame_size.width / target_zoom.max(0.001);
                let height = self.scroll_view.frame_size.height / target_zoom.max(0.001);
                self.zoom_to_rect(
                    rect(location.x - width / 2.0, location.y - height / 2.0, width, height),
                    false,
                    cx,
                );
            }
            EditorCropPinchUpdatePhase::Ended => {
                self.external_pinch_start_zoom = None;
                self.enforce_crop_coverage(true, cx);
                self.notify_image_rect(None, cx);
                self.post_crop_image_transform_if_needed();
            }
        }
    }

    /// sets the content offset and reports a scroll when it moved
    fn set_content_offset(&mut self, offset: Point<f32>, cx: &mut Context<Self>) {
        if !Self::content_offset_moved(self.scroll_view.content_offset, offset) {
            return;
        }
        self.scroll_view.content_offset = offset;
        self.scroll_view_did_scroll(cx);
        cx.notify();
    }

    /// true if the offset really moved
    /// f32 rotation math lands a hair away on every pass so a move below the jitter floor is no move
    pub(crate) fn content_offset_moved(from: Point<f32>, to: Point<f32>) -> bool {
        const CONTENT_OFFSET_JITTER: f32 = 0.01;
        (from.x - to.x).abs() >= CONTENT_OFFSET_JITTER || (from.y - to.y).abs() >= CONTENT_OFFSET_JITTER
    }

    fn offset_keeping_center(&self, zoom: f32) -> Point<f32> {
        let current = self.scroll_view.zoom_scale.max(0.001);
        let half_w = self.scroll_view.frame_size.width / 2.0;
        let half_h = self.scroll_view.frame_size.height / 2.0;
        let center = point(
            (self.scroll_view.content_offset.x + half_w) / current,
            (self.scroll_view.content_offset.y + half_h) / current,
        );
        self.offset_in_limits_at_zoom(point(center.x * zoom - half_w, center.y * zoom - half_h), zoom)
    }

    fn offset_in_limits_at_zoom(&self, offset: Point<f32>, zoom: f32) -> Point<f32> {
        let inset = self.content_insets_at_zoom(zoom);
        let content_w = self.image_container_size.width * zoom;
        let content_h = self.image_container_size.height * zoom;
        let min_x = -inset.left;
        let min_y = -inset.top;
        let max_x = min_x.max(content_w - self.scroll_view.frame_size.width + inset.right);
        let max_y = min_y.max(content_h - self.scroll_view.frame_size.height + inset.bottom);
        point(offset.x.max(min_x).min(max_x), offset.y.max(min_y).min(max_y))
    }

    #[cfg(test)]
    pub(super) fn is_zooming(&self) -> bool {
        self.scroll_view.is_zooming
    }

    /// sets the zoom scale, optionally animated
    pub(super) fn set_zoom_scale(&mut self, scale: f32, animated: bool, cx: &mut Context<Self>) {
        let clamped = self.scroll_view.clamped_zoom(scale);
        let offset = self.offset_keeping_center(clamped);
        if animated {
            self.start_zoom_animation(clamped, offset, cx);
        } else {
            self.cancel_zoom_animation();
            self.apply_zoom(clamped, offset, cx);
        }
    }

    /// zooms to a rect in the container's own coordinates, ends centered
    fn zoom_to_rect(&mut self, target: Bounds<f32>, animated: bool, cx: &mut Context<Self>) {
        let frame = self.scroll_view.frame_size;
        if target.size.width <= 0.0 || target.size.height <= 0.0 {
            return;
        }
        let scale = self.scroll_view.clamped_zoom((frame.width / target.size.width).min(frame.height / target.size.height));
        let offset = self.offset_in_limits_at_zoom(
            point(mid_x(&target) * scale - frame.width / 2.0, mid_y(&target) * scale - frame.height / 2.0),
            scale,
        );
        if animated {
            self.start_zoom_animation(scale, offset, cx);
        } else {
            self.cancel_zoom_animation();
            self.apply_zoom(scale, offset, cx);
        }
    }

    /// a dropped animation must not leave is_zooming set
    /// it would freeze the base geometry in layout
    /// a live pinch owns the flag and clears it itself
    fn cancel_zoom_animation(&mut self) {
        if self.zoom_animation.take().is_some() && self.zoom_gesture_raw_scale.is_none() {
            self.scroll_view.is_zooming = false;
        }
    }

    fn start_zoom_animation(&mut self, to_zoom: f32, to_offset: Point<f32>, cx: &mut Context<Self>) {
        let mut progress = Animated::new(0.0);
        progress.set(1.0, ease_in_out(Self::ZOOM_ANIMATION_DURATION), RiseClock::monotonic(), reduce_motion(cx));
        self.deceleration = None;
        self.bounce_back = None;
        self.zoom_animation = Some(EditorImageStageZoomAnimation {
            from_zoom: self.scroll_view.zoom_scale,
            to_zoom,
            from_offset: self.scroll_view.content_offset,
            to_offset,
            progress,
        });
        self.scroll_view.is_zooming = true;
        cx.notify();
    }

    /// one zoom step, the delegate sees the zoom first then the offset
    fn apply_zoom(&mut self, zoom: f32, offset: Point<f32>, cx: &mut Context<Self>) {
        let zoom_changed = zoom != self.scroll_view.zoom_scale;
        self.scroll_view.zoom_scale = zoom;
        self.scroll_view.content_size =
            size(self.image_container_size.width * zoom, self.image_container_size.height * zoom);
        if zoom_changed {
            self.scroll_view_did_zoom(cx);
        }
        self.set_content_offset(offset, cx);
        cx.notify();
    }

    fn stop_scrolling_and_zooming(&mut self) {
        self.zoom_animation = None;
        self.bounce_back = None;
        self.deceleration = None;
        self.scroll_view.is_zooming = false;
    }

    fn needs_display_link(&self) -> bool {
        self.crop_auto_fit_display_link
            || self.zoom_animation.is_some()
            || self.bounce_back.is_some()
            || self.deceleration.is_some()
    }

    fn display_link_did_fire(&mut self, cx: &mut Context<Self>) {
        self.display_link_scheduled = false;
        let now = RiseClock::monotonic();
        if self.crop_auto_fit_display_link {
            self.step_crop_auto_fit_animation(now, cx);
        }
        self.step_zoom_animation(now, cx);
        self.step_bounce_back(now, cx);
        self.step_deceleration(now, cx);
        cx.notify();
    }

    fn step_zoom_animation(&mut self, now: MotionTime, cx: &mut Context<Self>) {
        let Some(animation) = self.zoom_animation.as_ref() else {
            return;
        };
        let t = animation.progress.value(now);
        let is_running = animation.progress.is_running(now);
        let zoom = animation.from_zoom + (animation.to_zoom - animation.from_zoom) * t;
        let offset = point(
            animation.from_offset.x + (animation.to_offset.x - animation.from_offset.x) * t,
            animation.from_offset.y + (animation.to_offset.y - animation.from_offset.y) * t,
        );
        let to_zoom = animation.to_zoom;
        let to_offset = animation.to_offset;
        if is_running {
            self.apply_zoom(zoom, offset, cx);
            return;
        }
        self.zoom_animation = None;
        self.apply_zoom(to_zoom, to_offset, cx);
        self.scroll_view.is_zooming = false;
        self.scroll_view_did_end_zooming(to_zoom, cx);
    }

    fn step_bounce_back(&mut self, now: MotionTime, cx: &mut Context<Self>) {
        let Some(bounce) = self.bounce_back.as_ref() else {
            return;
        };
        let t = bounce.progress.value(now);
        let is_running = bounce.progress.is_running(now);
        let offset = point(
            bounce.from_offset.x + (bounce.to_offset.x - bounce.from_offset.x) * t,
            bounce.from_offset.y + (bounce.to_offset.y - bounce.from_offset.y) * t,
        );
        let to_offset = bounce.to_offset;
        if is_running {
            self.set_content_offset(offset, cx);
            return;
        }
        self.bounce_back = None;
        self.set_content_offset(to_offset, cx);
        self.scroll_view_did_end_decelerating(cx);
    }

    fn step_deceleration(&mut self, now: MotionTime, cx: &mut Context<Self>) {
        let Some(deceleration) = self.deceleration.as_mut() else {
            return;
        };
        let elapsed = now.saturating_sub(deceleration.last_tick);
        deceleration.last_tick = now;
        let decay = Self::DECELERATION_RATE.powf(elapsed.as_secs_f32() * 1000.0);
        deceleration.velocity.x *= decay;
        deceleration.velocity.y *= decay;
        let velocity = deceleration.velocity;
        let current = self.scroll_view.content_offset;
        let wanted = point(
            current.x + velocity.x * elapsed.as_secs_f32(),
            current.y + velocity.y * elapsed.as_secs_f32(),
        );
        let next = self.scroll_view.clamped_offset(wanted);
        self.set_content_offset(next, cx);
        let reached = self.scroll_view.content_offset;
        let Some(deceleration) = self.deceleration.as_mut() else {
            return;
        };
        if (reached.x - wanted.x).abs() > 0.5 {
            deceleration.velocity.x = 0.0;
        }
        if (reached.y - wanted.y).abs() > 0.5 {
            deceleration.velocity.y = 0.0;
        }
        let speed = deceleration.velocity.x.hypot(deceleration.velocity.y);
        if speed < Self::DECELERATION_STOP_SPEED {
            self.deceleration = None;
                self.scroll_view_did_end_decelerating(cx);
        }
    }

    fn scroll_view_will_begin_zooming(&mut self, pinch_is_active: bool, cx: &mut Context<Self>) {
        if self.current_mode != EditorMode::Crop || !pinch_is_active {
            return;
        }
        if !self.is_crop_viewport_ready
            || self.pending_crop_viewport_reset.is_some()
            || self.is_crop_reset_requested
        {
            return;
        }
        self.is_user_zooming_crop = true;
        self.post_crop_interaction_changed();
        self.cancel_crop_auto_fit_animation(cx);
    }

    fn scroll_view_did_zoom(&mut self, cx: &mut Context<Self>) {
        if self.is_stopping_crop_auto_fit || self.is_applying_crop_auto_fit_tick {
            return;
        }
        self.apply_scroll_interaction_policy(cx);
        self.update_content_insets();
        if self.pending_crop_viewport_reset.is_some() {
            let offset = self.centered_crop_content_offset(self.scroll_view.zoom_scale);
            self.set_content_offset(offset, cx);
            self.notify_image_rect(None, cx);
            return;
        }
        if self.is_crop_auto_fit_animating {
            return;
        }
        if self.current_mode == EditorMode::Crop {
            self.enforce_crop_coverage(false, cx);
        }
        self.apply_crop_unit_zoom_scroll_lock(cx);
        self.notify_image_rect(None, cx);
    }

    fn scroll_view_did_scroll(&mut self, cx: &mut Context<Self>) {
        if self.is_stopping_crop_auto_fit || self.is_applying_crop_auto_fit_tick {
            return;
        }
        if self.pending_crop_viewport_reset.is_some() {
            self.notify_image_rect(None, cx);
            return;
        }
        if self.current_mode == EditorMode::Crop {
            if self.is_crop_at_unit_zoom() {
                self.apply_crop_unit_zoom_scroll_lock(cx);
            } else if !self.is_crop_auto_fit_animating {
                self.clamp_scroll_offset_to_cover_crop_frame(cx);
            }
        }
        self.notify_image_rect(None, cx);
    }

    fn scroll_view_will_begin_dragging(&mut self, cx: &mut Context<Self>) {
        if self.current_mode == EditorMode::Crop {
            if !self.is_crop_viewport_ready
                || self.pending_crop_viewport_reset.is_some()
                || self.is_crop_reset_requested
            {
                return;
            }
            self.is_user_dragging_crop = true;
            self.post_crop_interaction_changed();
        }
        if self.is_crop_auto_fit_animating {
            self.cancel_crop_auto_fit_animation(cx);
        }
    }

    fn scroll_view_did_end_dragging(&mut self, will_decelerate: bool, cx: &mut Context<Self>) {
        if self.current_mode != EditorMode::Crop {
            return;
        }
        if !will_decelerate {
            self.is_user_dragging_crop = false;
            self.post_crop_interaction_changed();
            self.enforce_crop_coverage(true, cx);
            self.post_crop_image_transform_if_needed();
        }
    }

    fn scroll_view_did_end_decelerating(&mut self, cx: &mut Context<Self>) {
        if self.current_mode != EditorMode::Crop {
            return;
        }
        self.is_user_dragging_crop = false;
        self.post_crop_interaction_changed();
        self.enforce_crop_coverage(true, cx);
        self.post_crop_image_transform_if_needed();
    }

    fn post_crop_image_transform_if_needed(&self) {
        if self.current_mode != EditorMode::Crop
            || !self.is_crop_viewport_ready
            || self.is_crop_auto_fit_animating
            || self.is_crop_reset_requested
            || self.pending_crop_viewport_reset.is_some()
        {
            return;
        }
        RiseAppEvents::default_center().post_with(EDITOR_CROP_IMAGE_TRANSFORM_DID_CHANGE, self.crop_coordination_id);
    }

    fn post_crop_interaction_changed(&self) {
        if self.current_mode != EditorMode::Crop {
            return;
        }
        RiseAppEvents::default_center().post_with(
            EDITOR_CROP_INTERACTION_CHANGED,
            EditorCropInteractionUpdate {
                scope_id: self.crop_coordination_id,
                is_active: self.is_user_dragging_crop || self.is_user_zooming_crop,
            },
        );
    }

    fn scroll_view_did_end_zooming(&mut self, _scale: f32, cx: &mut Context<Self>) {
        if self.is_stopping_crop_auto_fit || self.is_applying_crop_auto_fit_tick || self.crop_auto_fit_display_link {
            return;
        }
        if self.pending_crop_viewport_reset.is_some() {
            self.finish_crop_viewport_reset(cx);
            return;
        }
        if self.current_mode == EditorMode::Crop {
            self.is_user_zooming_crop = false;
            self.post_crop_interaction_changed();
        }
        if self.is_crop_auto_fit_animating {
            self.apply_crop_unit_zoom_scroll_lock(cx);
            self.update_content_insets();
            self.finish_crop_auto_fit(EditorCropAutoFitCompletionStatus::Completed, cx);
            return;
        }
        self.enforce_crop_coverage(true, cx);
        self.apply_crop_unit_zoom_scroll_lock(cx);
        self.notify_image_rect(None, cx);
        self.post_crop_image_transform_if_needed();
    }

    fn handle_double_tap(&mut self, stage_point: Point<f32>, cx: &mut Context<Self>) {
        if !self.allows_pinch_zoom {
            return;
        }
        if self.is_crop_auto_fit_animating {
            self.cancel_crop_auto_fit_animation(cx);
        }
        let is_crop = self.current_mode == EditorMode::Crop;
        let reset_scale = if is_crop { self.minimum_zoom_covering_crop_frame() } else { 1.0 };
        let zoom_in_scale =
            if is_crop { (self.scroll_view.zoom_scale * 2.2).min(self.scroll_view.maximum_zoom_scale) } else { 2.4 };
        let target = if self.scroll_view.zoom_scale > reset_scale + 0.05 { reset_scale } else { zoom_in_scale };
        let location = self.image_container_point(stage_point);
        let frame = self.scroll_view.frame_size;
        let w = frame.width / target;
        let h = frame.height / target;
        self.zoom_to_rect(rect(location.x - w / 2.0, location.y - h / 2.0, w, h), true, cx);
    }

    /// a pinch or ctrl or cmd wheel step around anchor, by factor of the zoom so far
    fn zoom_gesture_changed(&mut self, factor: f32, anchor: Point<f32>, cx: &mut Context<Self>) {
        if self.zoom_gesture_raw_scale.is_none() {
            self.zoom_animation = None;
            self.bounce_back = None;
            self.deceleration = None;
                self.scroll_view.is_zooming = true;
            self.zoom_gesture_raw_scale = Some(self.scroll_view.zoom_scale);
            self.scroll_view_will_begin_zooming(true, cx);
        }
        let raw = self.zoom_gesture_raw_scale.unwrap_or(self.scroll_view.zoom_scale) * factor;
        self.zoom_gesture_raw_scale = Some(raw);
        let zoom = self.rubber_banded_zoom(raw);
        let current = self.scroll_view.zoom_scale.max(0.001);
        let offset = point(
            (anchor.x + self.scroll_view.content_offset.x) / current * zoom - anchor.x,
            (anchor.y + self.scroll_view.content_offset.y) / current * zoom - anchor.y,
        );
        let offset = if self.scroll_view.bounces_zoom { offset } else { self.offset_in_limits_at_zoom(offset, zoom) };
        self.apply_zoom(zoom, offset, cx);
    }

    fn rubber_banded_zoom(&self, raw: f32) -> f32 {
        let minimum = self.scroll_view.minimum_zoom_scale.max(0.001);
        let maximum = self.scroll_view.maximum_zoom_scale.max(minimum);
        if !self.scroll_view.bounces_zoom {
            return raw.max(minimum).min(maximum);
        }
        if raw > maximum {
            maximum * (raw / maximum).powf(Self::ZOOM_BOUNCE_RESISTANCE)
        } else if raw < minimum {
            minimum * (raw.max(0.001) / minimum).powf(Self::ZOOM_BOUNCE_RESISTANCE)
        } else {
            raw
        }
    }

    fn zoom_gesture_ended(&mut self, cx: &mut Context<Self>) {
        self.zoom_gesture_end = None;
        if self.zoom_gesture_raw_scale.take().is_none() {
            return;
        }
        let zoom = self.scroll_view.zoom_scale;
        let clamped = self.scroll_view.clamped_zoom(zoom);
        if (zoom - clamped).abs() > 0.0005 {
            self.set_zoom_scale(clamped, true, cx);
            return;
        }
        self.scroll_view.is_zooming = false;
        self.scroll_view_did_end_zooming(zoom, cx);
    }

    fn schedule_zoom_gesture_end(&mut self, cx: &mut Context<Self>) {
        self.zoom_gesture_end = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Self::WHEEL_GESTURE_END_DELAY).await;
            let _ = this.update(cx, |view, cx| view.zoom_gesture_ended(cx));
        }));
    }

    fn handle_pinch(&mut self, event: &PinchEvent, cx: &mut Context<Self>) {
        if !self.scroll_view.is_pinch_enabled || !self.scroll_view.is_user_interaction_enabled {
            return;
        }
        let anchor = self.local_point(event.position);
        self.zoom_gesture_changed(1.0 + event.delta, anchor, cx);
        if matches!(event.phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.zoom_gesture_ended(cx);
        } else {
            self.schedule_zoom_gesture_end(cx);
        }
        cx.stop_propagation();
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        if !self.scroll_view.is_user_interaction_enabled {
            return;
        }
        let delta = event.delta.pixel_delta(Self::WHEEL_LINE_HEIGHT);
        if event.modifiers.control || event.modifiers.platform {
            if !self.scroll_view.is_pinch_enabled {
                return;
            }
            let factor = (f32::from(delta.y) / Self::WHEEL_ZOOM_PIXELS_PER_E).exp();
            let anchor = self.local_point(event.position);
            self.zoom_gesture_changed(factor, anchor, cx);
            self.schedule_zoom_gesture_end(cx);
            cx.stop_propagation();
            return;
        }
        if !self.scroll_view.is_scroll_enabled || !self.scroll_view.is_pan_enabled {
            return;
        }
        let scrolls_x = self.scroll_view.scrolls_horizontally();
        let scrolls_y = self.scroll_view.scrolls_vertically();
        if !scrolls_x && !scrolls_y {
            return;
        }
        if !self.is_scroll_dragging {
            self.begin_scroll_drag(cx);
        }
        let current = self.scroll_view.content_offset;
        let wanted = point(
            if scrolls_x { current.x - f32::from(delta.x) } else { current.x },
            if scrolls_y { current.y - f32::from(delta.y) } else { current.y },
        );
        let offset = self.scroll_view.clamped_offset(wanted);
        self.set_content_offset(offset, cx);
        self.wheel_scroll_end = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Self::WHEEL_GESTURE_END_DELAY).await;
            let _ = this.update(cx, |view, cx| {
                view.wheel_scroll_end = None;
                if view.is_scroll_dragging {
                    view.is_scroll_dragging = false;
                    view.scroll_view_did_end_dragging(false, cx);
                }
            });
        }));
        cx.stop_propagation();
    }

    fn begin_scroll_drag(&mut self, cx: &mut Context<Self>) {
        self.bounce_back = None;
        self.deceleration = None;
        self.is_scroll_dragging = true;
        self.scroll_view_will_begin_dragging(cx);
    }

    fn scroll_pan_changed(&mut self, delta: Point<f32>, cx: &mut Context<Self>) {
        let minimum = self.scroll_view.minimum_offset();
        let maximum = self.scroll_view.maximum_offset();
        let bounces = self.scroll_view.bounces;
        let axis = |enabled: bool, value: f32, change: f32, low: f32, high: f32| {
            if !enabled {
                return value;
            }
            let next = value - change;
            if next >= low && next <= high {
                next
            } else if bounces {
                value - change * Self::OVERSCROLL_RESISTANCE
            } else {
                next.max(low).min(high)
            }
        };
        let current = self.scroll_view.content_offset;
        let offset = point(
            axis(self.scroll_view.scrolls_horizontally(), current.x, delta.x, minimum.x, maximum.x),
            axis(self.scroll_view.scrolls_vertically(), current.y, delta.y, minimum.y, maximum.y),
        );
        self.set_content_offset(offset, cx);
    }

    fn scroll_pan_ended(&mut self, velocity: Point<f32>, cx: &mut Context<Self>) {
        self.is_scroll_dragging = false;
        let offset = self.scroll_view.content_offset;
        let settled = self.scroll_view.clamped_offset(offset);
        if settled != offset {
            let mut progress = Animated::new(0.0);
            progress.set(1.0, ease_in_out(Self::ZOOM_ANIMATION_DURATION), RiseClock::monotonic(), reduce_motion(cx));
            self.bounce_back =
                Some(EditorImageStageBounceBack { from_offset: offset, to_offset: settled, progress });
            self.scroll_view_did_end_dragging(true, cx);
            cx.notify();
            return;
        }
        let speed = velocity.x.hypot(velocity.y);
        if speed > Self::DECELERATION_START_SPEED {
            self.deceleration = Some(EditorImageStageDeceleration {
                velocity: point(
                    if self.scroll_view.scrolls_horizontally() { -velocity.x } else { 0.0 },
                    if self.scroll_view.scrolls_vertically() { -velocity.y } else { 0.0 },
                ),
                last_tick: RiseClock::monotonic(),
            });
            self.scroll_view_did_end_dragging(true, cx);
            cx.notify();
            return;
        }
        self.scroll_view_did_end_dragging(false, cx);
    }

    fn handle_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.single_tap_should_receive(cx);
        self.press = Some(EditorImageStagePress {
            start: event.position,
            last: event.position,
            click_count: event.click_count,
            gesture: EditorImageStagePressGesture::Undecided,
            samples: vec![(RiseClock::monotonic(), pixels_point(event.position))],
        });
    }

    fn handle_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.press.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.press_ended(event.position, cx);
            return;
        }
        let stage_point = self.local_point(event.position);
        let Some(press) = self.press.as_mut() else {
            return;
        };
        let now = RiseClock::monotonic();
        press.samples.push((now, pixels_point(event.position)));
        press.samples.retain(|(time, _)| now.saturating_sub(*time) <= Self::VELOCITY_WINDOW);
        let delta = pixels_point(event.position - press.last);
        let translation = pixels_point(event.position - press.start);
        press.last = event.position;

        if press.gesture == EditorImageStagePressGesture::Undecided {
            if translation.x.abs() <= Self::TAP_SLOP && translation.y.abs() <= Self::TAP_SLOP {
                return;
            }
            let gesture = if self.dismiss_pan_should_begin(stage_point, cx) {
                EditorImageStagePressGesture::DismissPan
            } else if self.scroll_view.is_user_interaction_enabled
                && self.scroll_view.is_scroll_enabled
                && self.scroll_view.is_pan_enabled
                && (self.scroll_view.scrolls_horizontally() || self.scroll_view.scrolls_vertically())
            {
                EditorImageStagePressGesture::ScrollPan
            } else {
                EditorImageStagePressGesture::Unclaimed
            };
            if let Some(press) = self.press.as_mut() {
                press.gesture = gesture;
            }
            match gesture {
                EditorImageStagePressGesture::DismissPan => {
                    self.handle_dismiss_pan(EditorImageStagePanState::Began, translation, Point::default(), cx);
                }
                EditorImageStagePressGesture::ScrollPan => self.begin_scroll_drag(cx),
                _ => {}
            }
        }

        let gesture = self.press.as_ref().map(|press| press.gesture);
        match gesture {
            Some(EditorImageStagePressGesture::DismissPan) => {
                let velocity = self.press_velocity();
                self.handle_dismiss_pan(EditorImageStagePanState::Changed, translation, velocity, cx);
            }
            Some(EditorImageStagePressGesture::ScrollPan) => self.scroll_pan_changed(delta, cx),
            _ => {}
        }
    }

    fn handle_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if event.button == MouseButton::Left {
            self.press_ended(event.position, cx);
        }
    }

    fn press_velocity(&self) -> Point<f32> {
        let Some(press) = self.press.as_ref() else {
            return Point::default();
        };
        let (Some(first), Some(last)) = (press.samples.first(), press.samples.last()) else {
            return Point::default();
        };
        let elapsed = last.0.saturating_sub(first.0).as_secs_f32();
        if elapsed <= 0.0 {
            return Point::default();
        }
        point((last.1.x - first.1.x) / elapsed, (last.1.y - first.1.y) / elapsed)
    }

    fn press_ended(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let velocity = self.press_velocity();
        let Some(press) = self.press.take() else {
            return;
        };
        let translation = pixels_point(position - press.start);
        match press.gesture {
            EditorImageStagePressGesture::Undecided => {
                let stage_point = self.local_point(position);
                self.press_was_tap(stage_point, press.click_count, cx);
            }
            EditorImageStagePressGesture::DismissPan => {
                self.handle_dismiss_pan(EditorImageStagePanState::Ended, translation, velocity, cx);
            }
            EditorImageStagePressGesture::ScrollPan => self.scroll_pan_ended(velocity, cx),
            EditorImageStagePressGesture::Unclaimed => {}
        }
    }

    fn press_was_tap(&mut self, stage_point: Point<f32>, click_count: usize, cx: &mut Context<Self>) {
        if click_count >= 2 {
            self.pending_single_tap = None;
            if self.scroll_view.is_user_interaction_enabled {
                self.handle_double_tap(stage_point, cx);
            }
            return;
        }
        self.pending_single_tap = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Self::SINGLE_TAP_FAILURE_DELAY).await;
            let _ = this.update(cx, |view, cx| {
                view.pending_single_tap = None;
                if view.single_tap_should_begin(stage_point, cx) {
                    view.handle_single_tap(cx);
                }
            });
        }));
    }

    fn image_container_frame(&self) -> Bounds<Pixels> {
        let zoom = self.scroll_view.zoom_scale;
        Bounds {
            origin: point(px(-self.scroll_view.content_offset.x), px(-self.scroll_view.content_offset.y)),
            size: size(
                px(self.image_container_size.width * zoom),
                px(self.image_container_size.height * zoom),
            ),
        }
    }
}

impl Render for EditorImageStageUIView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.layout_if_needed(cx);

        let now = RiseClock::monotonic();
        let model_frame = self.image_container_frame();
        if *self.image_container_display_frame.target() != model_frame {
            match self.pending_layout_animation.take() {
                Some(motion) => self.image_container_display_frame.set(model_frame, motion, now, reduce_motion(cx)),
                None => self.image_container_display_frame.jump(model_frame),
            }
        } else {
            self.pending_layout_animation = None;
        }
        let frame = self.image_container_display_frame.frame_value(now, window);
        let rotation = self.image_view_display_rotation.frame_value(now, window);
        let mirror = self.image_view_display_mirror.frame_value(now, window);

        if self.needs_display_link() {
            window.request_animation_frame();
            if !self.display_link_scheduled {
                self.display_link_scheduled = true;
                cx.on_next_frame(window, |view, _, cx| view.display_link_did_fire(cx));
            }
        }

        let base_width = self.image_container_size.width;
        let scale = if base_width > 0.0 { f32::from(frame.size.width) / base_width } else { 1.0 };
        let overlay_is_hidden = self.overlay.read(cx).is_hidden();
        self.overlay.update(cx, |overlay, cx| overlay.set_display_zoom(scale, cx));

        let image = self.image.clone();
        let image_view_size = self.image_view_size;
        let this = cx.weak_entity();
        let moves = this.clone();
        let ups = this.clone();
        let image_layer = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let _ = moves.update(cx, |view, cx| view.handle_mouse_move(event, cx));
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let _ = ups.update(cx, |view, cx| view.handle_mouse_up(event, cx));
                });
                let Some(image) = image else {
                    return;
                };
                let center = point(
                    bounds.origin.x + frame.origin.x + frame.size.width / 2.0,
                    bounds.origin.y + frame.origin.y + frame.size.height / 2.0,
                );
                let width = px(image_view_size.width * scale);
                let height = px(image_view_size.height * scale);
                let image_bounds = Bounds {
                    origin: point(center.x - width / 2.0, center.y - height / 2.0),
                    size: size(width, height),
                };
                let scale_factor = window.scale_factor();
                let matrix = TransformationMatrix::unit()
                    .translate(center.scale(scale_factor))
                    .rotate(Radians(rotation))
                    .scale(size(mirror, 1.0))
                    .translate(center.scale(-scale_factor));
                window.with_transformation(matrix, |window| {
                    let _ = window.paint_image(image_bounds, image_bounds, Corners::default(), image, 0, false);
                });
            },
        )
        .absolute()
        .size_full();

        div()
            .id("editor-image-stage")
            .relative()
            .size_full()
            .overflow_hidden()
            .child(image_layer)
            .when(!overlay_is_hidden, |stage| {
                stage.child(
                    div()
                        .absolute()
                        .left(frame.origin.x)
                        .top(frame.origin.y)
                        .w(frame.size.width)
                        .h(frame.size.height)
                        .overflow_hidden()
                        .child(self.overlay.clone()),
                )
            })
            .child(measure("editor-image-stage-bounds", move |bounds, _, cx| {
                let _ = this.update(cx, |view, cx| view.set_stage_bounds(bounds, cx));
            }))
            .on_pinch(cx.listener(|view, event: &PinchEvent, _, cx| view.handle_pinch(event, cx)))
            .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _, cx| view.handle_scroll_wheel(event, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, _, cx| view.handle_mouse_down(event, cx)),
            )
    }
}
