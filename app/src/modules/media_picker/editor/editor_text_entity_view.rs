use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, Bounds, BoxShadow, Context, DispatchPhase, ElementId, Entity, FontWeight, Hitbox,
    HitboxBehavior, Hsla, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, PathBuilder, Pixels, Point, Render, RenderOnce, Rgba, SharedString, Size, Styled,
    TextRun, Window, canvas, div, hsla, point, px, size,
};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT, SwiftUiFont};
use rise_ui::swiftui::rotation_effect::RotationEffectModifiers;
use rise_ui::swiftui::shapes::SwiftUiShapes;

use crate::modules::media_picker::editor::editor_entity_snapping::{
    EditorEntitySideHandle, EditorEntitySnap, EditorEntitySnapFeedback,
};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBakeTextPlate;
use crate::modules::media_picker::editor::editor_state::{
    EditorTextEntity, EditorTextEntityStyleMode, EditorTextEntityTextAlignment,
};
use crate::modules::media_picker::editor::editor_two_finger_transform::EditorTwoFingerTransformable;
use crate::modules::media_picker::editor::story_canvas_model::StoryCanvasGeometry;

pub enum EditorTextEntityViewMetrics {}

impl EditorTextEntityViewMetrics {
    pub const SELECTION_PADDING: f32 = 8.0;
    pub const HANDLE_VISUAL_SIZE: f32 = 12.0;
    pub const HANDLE_HIT_OUTSET: f32 = 22.0;
}

const PAN_MINIMUM_DISTANCE: f32 = 4.0;

pub type EditorTextEntityUpdate = Rc<dyn Fn(EditorTextEntity, &mut App)>;
pub type EditorTextEntityAction = Rc<dyn Fn(Uuid, &mut App)>;
pub type EditorTextEntitySnapGuides = Rc<dyn Fn(bool, bool, &mut App)>;

/// one left button press from down to up
#[derive(Clone, Copy)]
enum EditorTextEntityPress {
    Body { press: Point<Pixels>, began_at: Option<Point<Pixels>>, can_pan: bool },
    SideHandle { sign: f32, press: Point<Pixels>, began_at: Option<Point<Pixels>> },
}

pub struct EditorTextEntityView {
    pub entity_id: Uuid,

    current: Option<EditorTextEntity>,
    canvas_size: Size<f32>,
    is_selected: bool,
    handles_visible: bool,
    allows_body_drag: bool,
    bounds: Size<f32>,

    move_start_center: Point<f32>,
    side_start_scale: f32,
    side_start_rotation: f32,
    side_start_handle_offset: Point<f32>,
    is_side_handle_dragging: bool,
    two_finger_start_scale: f32,
    two_finger_start_rotation: f32,
    display_zoom: f32,

    pub on_update: Option<EditorTextEntityUpdate>,
    pub on_select: Option<EditorTextEntityAction>,
    pub on_request_edit_menu: Option<EditorTextEntityAction>,
    pub on_request_delete: Option<EditorTextEntityAction>,
    pub on_snap_guides_changed: Option<EditorTextEntitySnapGuides>,

    snap_feedback: EditorEntitySnapFeedback,
    press: Option<EditorTextEntityPress>,
    canvas_origin: Rc<Cell<Point<Pixels>>>,
}

impl EditorTextEntityView {
    pub fn new(entity_id: Uuid, _cx: &mut Context<Self>) -> Self {
        Self {
            entity_id,
            current: None,
            canvas_size: Size::default(),
            is_selected: false,
            handles_visible: false,
            allows_body_drag: true,
            bounds: Size::default(),
            move_start_center: Point::default(),
            side_start_scale: 1.0,
            side_start_rotation: 0.0,
            side_start_handle_offset: Point::default(),
            is_side_handle_dragging: false,
            two_finger_start_scale: 1.0,
            two_finger_start_rotation: 0.0,
            display_zoom: 1.0,
            on_update: None,
            on_select: None,
            on_request_edit_menu: None,
            on_request_delete: None,
            on_snap_guides_changed: None,
            snap_feedback: EditorEntitySnapFeedback::new(),
            press: None,
            canvas_origin: Rc::new(Cell::new(Point::default())),
        }
    }

    pub fn set_display_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        if self.display_zoom != zoom {
            self.display_zoom = zoom;
            cx.notify();
        }
    }

    pub fn set_interaction_state(
        &mut self,
        handles_visible: bool,
        allows_body_drag: bool,
        cx: &mut Context<Self>,
    ) {
        self.handles_visible = handles_visible;
        self.allows_body_drag = allows_body_drag;
        if !handles_visible {
            self.is_side_handle_dragging = false;
        }
        cx.notify();
    }

    pub fn apply(
        &mut self,
        entity: &EditorTextEntity,
        canvas_size: Size<f32>,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) {
        self.canvas_size = canvas_size;
        self.current = Some(entity.clone());
        self.is_selected = is_selected;
        cx.notify();
    }

    /// entity center in canvas points
    fn center(&self) -> Point<f32> {
        self.current.as_ref().map_or(Point::default(), |entity| {
            point(entity.center.x * self.canvas_size.width, entity.center.y * self.canvas_size.height)
        })
    }

    /// selection rect in the views own points
    fn selection_rect(&self) -> Bounds<f32> {
        let pad = EditorTextEntityViewMetrics::SELECTION_PADDING * self.display_zoom;
        Bounds::new(point(-pad, -pad), size(self.bounds.width + pad * 2.0, self.bounds.height + pad * 2.0))
    }

    /// frame of the left or right handle
    fn handle_frame(&self, sign: f32) -> Bounds<f32> {
        let selection = self.selection_rect();
        let dot = EditorTextEntityViewMetrics::HANDLE_VISUAL_SIZE * self.display_zoom;
        let x = if sign > 0.0 { selection.origin.x + selection.size.width } else { selection.origin.x };
        Bounds::new(
            point(x - dot / 2.0, selection.origin.y + selection.size.height / 2.0 - dot / 2.0),
            size(dot, dot),
        )
    }

    fn handle_hit_rect(&self, sign: f32) -> Bounds<f32> {
        let frame = self.handle_frame(sign);
        let outset = EditorTextEntityViewMetrics::HANDLE_HIT_OUTSET * self.display_zoom;
        Bounds::new(
            point(frame.origin.x - outset, frame.origin.y - outset),
            size(frame.size.width + outset * 2.0, frame.size.height + outset * 2.0),
        )
    }

    /// hit test, point is in the views own points
    pub fn point_inside(&self, point: Point<f32>) -> bool {
        if self.handles_visible
            && (rect_contains(self.handle_hit_rect(-1.0), point)
                || rect_contains(self.handle_hit_rect(1.0), point))
        {
            return true;
        }
        rect_contains(self.selection_rect(), point)
    }

    pub fn contains_canvas_point(&self, canvas_point: Point<f32>) -> bool {
        self.point_inside(self.local_point(canvas_point))
    }

    /// undoes the view transform, scale and rotation around its center
    fn local_point(&self, canvas_point: Point<f32>) -> Point<f32> {
        let center = self.center();
        let (scale, rotation) = self
            .current
            .as_ref()
            .map_or((1.0, 0.0), |entity| (entity.scale, entity.rotation));
        let dx = canvas_point.x - center.x;
        let dy = canvas_point.y - center.y;
        let (sin, cos) = rotation.sin_cos();
        let scale = scale.max(f32::EPSILON);
        point(
            (dx * cos + dy * sin) / scale + self.bounds.width / 2.0,
            (-dx * sin + dy * cos) / scale + self.bounds.height / 2.0,
        )
    }

    /// converts a local point to canvas coordinates
    fn canvas_point(&self, local: Point<f32>) -> Point<f32> {
        let center = self.center();
        let (scale, rotation) = self
            .current
            .as_ref()
            .map_or((1.0, 0.0), |entity| (entity.scale, entity.rotation));
        let dx = (local.x - self.bounds.width / 2.0) * scale;
        let dy = (local.y - self.bounds.height / 2.0) * scale;
        let (sin, cos) = rotation.sin_cos();
        point(center.x + dx * cos - dy * sin, center.y + dx * sin + dy * cos)
    }

    fn window_to_canvas(&self, position: Point<Pixels>) -> Point<f32> {
        let origin = self.canvas_origin.get();
        point(f32::from(position.x - origin.x), f32::from(position.y - origin.y))
    }

    fn configure_label(entity: &EditorTextEntity, canvas_size: Size<f32>, display_zoom: f32) -> PaddedLabel {
        let text: SharedString =
            if entity.text.is_empty() { " ".into() } else { entity.text.clone().into() };
        let canvas_short_side = canvas_size.width.min(canvas_size.height).max(1.0);
        let pixel_size = (entity.font_size * canvas_short_side).max(8.0 * display_zoom);
        let plate_corner = pixel_size * EditorImageBakeTextPlate::CORNER_RADIUS;
        let plate_insets = size(
            pixel_size * EditorImageBakeTextPlate::PLATE_INSET_X,
            pixel_size * EditorImageBakeTextPlate::PLATE_INSET_Y,
        );
        let outline_insets = size(
            pixel_size * EditorImageBakeTextPlate::OUTLINE_INSET_X,
            pixel_size * EditorImageBakeTextPlate::OUTLINE_INSET_Y,
        );
        let (background, text_color, corner_radius, content_insets, casts_shadow) =
            match entity.style_mode {
                EditorTextEntityStyleMode::Background => (
                    Some(entity.color),
                    Self::best_foreground_color(entity.color),
                    plate_corner,
                    plate_insets,
                    false,
                ),
                EditorTextEntityStyleMode::TranslucentBg => (
                    Some(hsla(0.0, 0.0, 1.0, 0.5)),
                    entity.color,
                    plate_corner,
                    plate_insets,
                    false,
                ),
                EditorTextEntityStyleMode::TextOnly => {
                    (None, entity.color, 0.0, outline_insets, true)
                }
            };
        PaddedLabel {
            text,
            pixel_size,
            layer_scale: entity.scale,
            display_zoom,
            soft_wraps: true,
            alignment: entity.alignment,
            background,
            text_color,
            corner_radius,
            content_insets,
            casts_shadow,
        }
    }

    pub(crate) fn best_foreground_color(color: Hsla) -> Hsla {
        let rgba: Rgba = color.into();
        let luminance = 0.2126 * rgba.r + 0.7152 * rgba.g + 0.0722 * rgba.b;
        if luminance > 0.55 { hsla(0.0, 0.0, 0.08, 1.0) } else { hsla(0.0, 0.0, 1.0, 1.0) }
    }

    fn handle_tap(this: &Entity<Self>, cx: &mut App) {
        let (entity_id, on_select) = {
            let view = this.read(cx);
            (view.entity_id, view.on_select.clone())
        };
        if let Some(on_select) = on_select {
            on_select(entity_id, cx);
        }
    }

    fn handle_double_tap(this: &Entity<Self>, cx: &mut App) {
        Self::handle_tap(this, cx);
    }

    fn edit_menu_items(&self) -> Vec<ContextMenuItem> {
        let entity_id = self.entity_id;
        let on_request_edit_menu = self.on_request_edit_menu.clone();
        let on_request_delete = self.on_request_delete.clone();
        vec![
            ContextMenuItem::new(0, tr("editor_text_edit"), None, "edit", move |_, cx| {
                if let Some(on_request_edit_menu) = &on_request_edit_menu {
                    on_request_edit_menu(entity_id, cx);
                }
            }),
            ContextMenuItem::new(1, tr("editor_text_delete"), None, "delete", move |_, cx| {
                if let Some(on_request_delete) = &on_request_delete {
                    on_request_delete(entity_id, cx);
                }
            })
            .destructive(true),
        ]
    }

    fn handle_pan_began(this: &Entity<Self>, cx: &mut App) {
        let began = this.update(cx, |view, _| {
            if !view.allows_body_drag || view.is_side_handle_dragging || view.current.is_none() {
                return false;
            }
            view.move_start_center = view.center();
            view.snap_feedback.begin();
            true
        });
        if began {
            Self::handle_tap(this, cx);
        }
    }

    fn handle_pan_changed(
        this: &Entity<Self>,
        translation: Point<f32>,
        cx: &mut App,
    ) {
        let outcome = this.update(cx, |view, cx| {
            if !view.allows_body_drag || view.is_side_handle_dragging {
                return None;
            }
            let mut entity = view.current.clone()?;
            let proposed = point(
                view.move_start_center.x + translation.x,
                view.move_start_center.y + translation.y,
            );
            let (snapped, snapped_x, snapped_y) =
                EditorEntitySnap::snap_center(proposed, view.canvas_size);
            view.snap_feedback.update_center(snapped_x, snapped_y);
            let updated = (view.canvas_size.width > 0.0 && view.canvas_size.height > 0.0).then(|| {
                entity.center =
                    point(snapped.x / view.canvas_size.width, snapped.y / view.canvas_size.height);
                view.current = Some(entity.clone());
                cx.notify();
                entity
            });
            Some((
                view.on_snap_guides_changed.clone(),
                snapped_x,
                snapped_y,
                view.on_update.clone(),
                updated,
            ))
        });
        let Some((on_snap_guides_changed, snapped_x, snapped_y, on_update, updated)) = outcome else {
            return;
        };
        if let Some(on_snap_guides_changed) = on_snap_guides_changed {
            on_snap_guides_changed(snapped_x, snapped_y, cx);
        }
        if let (Some(on_update), Some(updated)) = (on_update, updated) {
            on_update(updated, cx);
        }
    }

    fn handle_pan_ended(this: &Entity<Self>, cx: &mut App) {
        let view = this.read(cx);
        if !view.allows_body_drag || view.is_side_handle_dragging {
            return;
        }
        if let Some(on_snap_guides_changed) = view.on_snap_guides_changed.clone() {
            on_snap_guides_changed(false, false, cx);
        }
    }

    fn apply_side_handle_began(this: &Entity<Self>, sign: f32, cx: &mut App) {
        let began = this.update(cx, |view, _| {
            let Some((scale, rotation)) =
                view.current.as_ref().map(|entity| (entity.scale, entity.rotation))
            else {
                return false;
            };
            view.is_side_handle_dragging = true;
            view.side_start_scale = scale;
            view.side_start_rotation = rotation;
            let center = view.center();
            let handle = view.handle_frame(sign);
            let handle_center = view.canvas_point(point(
                handle.origin.x + handle.size.width / 2.0,
                handle.origin.y + handle.size.height / 2.0,
            ));
            view.side_start_handle_offset =
                point(handle_center.x - center.x, handle_center.y - center.y);
            view.snap_feedback.begin();
            true
        });
        if began {
            Self::handle_tap(this, cx);
        }
    }

    fn apply_side_handle_changed(
        this: &Entity<Self>,
        translation: Point<f32>,
        cx: &mut App,
    ) {
        let outcome = this.update(cx, |view, cx| {
            let mut entity = view.current.clone()?;
            let (scale, rotation) = EditorEntitySideHandle::scale_and_rotation(
                view.side_start_handle_offset,
                translation,
                view.side_start_scale,
                view.side_start_rotation,
            );
            let (angle, snapped) = EditorEntitySnap::snap_rotation(rotation);
            view.snap_feedback.update_rotation(snapped);
            entity.rotation = angle;
            entity.scale = scale;
            view.current = Some(entity.clone());
            cx.notify();
            Some((view.on_update.clone(), entity))
        });
        if let Some((Some(on_update), updated)) = outcome {
            on_update(updated, cx);
        }
    }

    fn handle_mouse_down(
        this: &Entity<Self>,
        event: &MouseDownEvent,
        cx: &mut App,
    ) {
        let view = this.read(cx);
        let local = view.local_point(view.window_to_canvas(event.position));
        if !view.point_inside(local) {
            return;
        }
        let handle_sign = view.handles_visible.then(|| {
            [-1.0_f32, 1.0].into_iter().find(|sign| rect_contains(view.handle_frame(*sign), local))
        });
        let can_pan = !rect_contains(view.handle_hit_rect(-1.0), local)
            && !rect_contains(view.handle_hit_rect(1.0), local)
            && !view.is_side_handle_dragging;
        cx.stop_propagation();

        if event.click_count >= 2 {
            Self::handle_double_tap(this, cx);
            return;
        }
        let press = match handle_sign.flatten() {
            Some(sign) => {
                EditorTextEntityPress::SideHandle { sign, press: event.position, began_at: None }
            }
            None => EditorTextEntityPress::Body { press: event.position, began_at: None, can_pan },
        };
        this.update(cx, |view, _| view.press = Some(press));
    }

    fn handle_mouse_move(
        this: &Entity<Self>,
        event: &MouseMoveEvent,
        cx: &mut App,
    ) {
        let Some(press) = this.read(cx).press else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            Self::finish_press(this, press, cx);
            return;
        }
        match press {
            EditorTextEntityPress::Body { press, began_at, can_pan } => {
                if !can_pan {
                    return;
                }
                let began_at = match began_at {
                    Some(began_at) => began_at,
                    None => {
                        if !past_pan_distance(press, event.position) {
                            return;
                        }
                        let began_at = event.position;
                        this.update(cx, |view, _| {
                            view.press = Some(EditorTextEntityPress::Body {
                                press,
                                began_at: Some(began_at),
                                can_pan,
                            });
                        });
                        Self::handle_pan_began(this, cx);
                        began_at
                    }
                };
                Self::handle_pan_changed(this, translation_of(began_at, event.position), cx);
            }
            EditorTextEntityPress::SideHandle { sign, press, began_at } => {
                let began_at = match began_at {
                    Some(began_at) => began_at,
                    None => {
                        if !past_pan_distance(press, event.position) {
                            return;
                        }
                        let began_at = event.position;
                        this.update(cx, |view, _| {
                            view.press = Some(EditorTextEntityPress::SideHandle {
                                sign,
                                press,
                                began_at: Some(began_at),
                            });
                        });
                        Self::apply_side_handle_began(this, sign, cx);
                        began_at
                    }
                };
                Self::apply_side_handle_changed(
                    this,
                    translation_of(began_at, event.position),
                    cx,
                );
            }
        }
    }

    fn finish_press(
        this: &Entity<Self>,
        press: EditorTextEntityPress,
        cx: &mut App,
    ) {
        this.update(cx, |view, _| view.press = None);
        match press {
            EditorTextEntityPress::Body { began_at: Some(_), .. } => {
                Self::handle_pan_ended(this, cx);
            }
            EditorTextEntityPress::SideHandle { began_at: Some(_), .. } => {
                this.update(cx, |view, _| view.is_side_handle_dragging = false);
            }
            EditorTextEntityPress::Body { began_at: None, .. }
            | EditorTextEntityPress::SideHandle { began_at: None, .. } => {
                Self::handle_tap(this, cx);
            }
        }
    }

    /// takes the pointer on the turned shape of the view, not on the box gpui hit tests
    fn gesture_layer(
        this: Entity<Self>,
        hit_bounds: Bounds<Pixels>,
        canvas_origin: Rc<Cell<Point<Pixels>>>,
        canvas_origin_in_hit: Point<Pixels>,
    ) -> impl IntoElement {
        canvas(
            move |bounds: Bounds<Pixels>, window: &mut Window, _: &mut App| {
                canvas_origin.set(bounds.origin - canvas_origin_in_hit);
                window.insert_hitbox(bounds, HitboxBehavior::Normal)
            },
            move |_: Bounds<Pixels>, hitbox: Hitbox, window: &mut Window, _: &mut App| {
                let down = this.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                        return;
                    }
                    match event.button {
                        MouseButton::Left => Self::handle_mouse_down(&down, event, cx),
                        MouseButton::Right => {
                            let view = down.read(cx);
                            let local = view.local_point(view.window_to_canvas(event.position));
                            if view.point_inside(local) {
                                Self::handle_tap(&down, cx);
                            } else {
                                cx.stop_propagation();
                            }
                        }
                        _ => {}
                    }
                });
                let moved = this.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        Self::handle_mouse_move(&moved, event, cx);
                    }
                });
                let up = this.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                        return;
                    }
                    if let Some(press) = up.read(cx).press {
                        Self::finish_press(&up, press, cx);
                    }
                });
            },
        )
        .absolute()
        .left_0()
        .top_0()
        .w(hit_bounds.size.width)
        .h(hit_bounds.size.height)
    }
}

impl EditorTwoFingerTransformable for EditorTextEntityView {
    fn entity_id(&self) -> Uuid {
        self.entity_id
    }

    fn begin_two_finger_transform(&mut self, cx: &mut Context<Self>) {
        let Some((scale, rotation)) = self.current.as_ref().map(|entity| (entity.scale, entity.rotation)) else {
            return;
        };
        if self.is_side_handle_dragging {
            return;
        }
        self.two_finger_start_scale = scale;
        self.two_finger_start_rotation = rotation;
        self.snap_feedback.begin();
        if let Some(on_select) = self.on_select.clone() {
            on_select(self.entity_id, cx);
        }
    }

    fn apply_two_finger_transform(&mut self, scale: f32, rotation: f32, cx: &mut Context<Self>) {
        if self.is_side_handle_dragging {
            return;
        }
        let Some(mut entity) = self.current.clone() else {
            return;
        };
        let range = StoryCanvasGeometry::ENTITY_SCALE_RANGE;
        entity.scale = (self.two_finger_start_scale * scale).max(*range.start()).min(*range.end());
        let (angle, snapped) = EditorEntitySnap::snap_rotation(self.two_finger_start_rotation + rotation);
        self.snap_feedback.update_rotation(snapped);
        entity.rotation = angle;
        self.current = Some(entity.clone());
        cx.notify();
        if let Some(on_update) = self.on_update.clone() {
            on_update(entity, cx);
        }
    }

    fn end_two_finger_transform(&mut self, _cx: &mut Context<Self>) {}
}

impl Render for EditorTextEntityView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(entity) = self.current.clone() else {
            return div().into_any_element();
        };
        let zoom = self.display_zoom;
        let mut label = Self::configure_label(&entity, self.canvas_size, zoom);
        let measured = label.fitting_size(self.canvas_size.width, window);
        label.soft_wraps = !label.fits_unwrapped(self.canvas_size.width, window);
        self.bounds = size(measured.width.max(48.0 * zoom), measured.height.max(28.0 * zoom));

        let scale = entity.scale;
        let center = self.center();
        let frame = size(self.bounds.width * scale, self.bounds.height * scale);
        let (sin, cos) = entity.rotation.sin_cos();
        let selection = self.selection_rect();
        let reach = {
            let half_w = selection.size.width * scale / 2.0
                + (EditorTextEntityViewMetrics::HANDLE_HIT_OUTSET
                    + EditorTextEntityViewMetrics::HANDLE_VISUAL_SIZE / 2.0)
                    * zoom
                    * scale;
            let half_h = selection.size.height * scale / 2.0
                + (EditorTextEntityViewMetrics::HANDLE_HIT_OUTSET
                    + EditorTextEntityViewMetrics::HANDLE_VISUAL_SIZE / 2.0)
                    * zoom
                    * scale;
            size(
                half_w * cos.abs() + half_h * sin.abs(),
                half_w * sin.abs() + half_h * cos.abs(),
            )
        };
        let hit_origin = point(center.x - reach.width, center.y - reach.height);
        let hit_size = size(reach.width * 2.0, reach.height * 2.0);
        let hit_bounds = Bounds::new(
            point(px(hit_origin.x), px(hit_origin.y)),
            size(px(hit_size.width), px(hit_size.height)),
        );
        let canvas_origin_in_hit = hit_bounds.origin;

        let chrome_visible = self.is_selected && self.handles_visible;
        let mut turned = div().relative().size_full().child(label);
        if chrome_visible {
            turned = turned.child(selection_frame_layer(scale * zoom));
        }
        if self.handles_visible {
            turned = turned
                .child(HandleDotView { frame: self.handle_frame(-1.0), scale, zoom })
                .child(HandleDotView { frame: self.handle_frame(1.0), scale, zoom });
        }

        let positioned = div()
            .absolute()
            .left(px(center.x - frame.width / 2.0 - hit_origin.x))
            .top(px(center.y - frame.height / 2.0 - hit_origin.y))
            .w(px(frame.width))
            .h(px(frame.height))
            .child(turned.rotation_effect(entity.rotation.to_degrees()));

        let content = div()
            .relative()
            .w(px(hit_size.width))
            .h(px(hit_size.height))
            .child(positioned)
            .child(Self::gesture_layer(
                cx.entity(),
                hit_bounds,
                self.canvas_origin.clone(),
                canvas_origin_in_hit,
            ));
        let menu_id = ElementId::Name(SharedString::from(format!(
            "editor-text-entity-menu-{:?}",
            self.entity_id
        )));
        div()
            .absolute()
            .left(px(hit_origin.x))
            .top(px(hit_origin.y))
            .child(content.context_menu(menu_id, self.edit_menu_items()).opens_on_double_click())
            .into_any_element()
    }
}

fn rect_contains(rect: Bounds<f32>, point: Point<f32>) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y <= rect.origin.y + rect.size.height
}

fn past_pan_distance(press: Point<Pixels>, position: Point<Pixels>) -> bool {
    let delta = translation_of(press, position);
    delta.x.hypot(delta.y) >= PAN_MINIMUM_DISTANCE
}

fn translation_of(from: Point<Pixels>, to: Point<Pixels>) -> Point<f32> {
    point(f32::from(to.x - from.x), f32::from(to.y - from.y))
}

fn selection_frame_layer(scale: f32) -> impl IntoElement {
    let pad = EditorTextEntityViewMetrics::SELECTION_PADDING * scale;
    canvas(
        |_: Bounds<Pixels>, _: &mut Window, _: &mut App| {},
        move |bounds: Bounds<Pixels>, (), window: &mut Window, _: &mut App| {
            let corner = px(10.0 * scale);
            let radii = point(corner, corner);
            let (left, top, right, bottom) =
                (bounds.left(), bounds.top(), bounds.right(), bounds.bottom());
            let mut builder = PathBuilder::stroke(px(scale))
                .dash_array(&[px(5.0 * scale), px(4.0 * scale)]);
            builder.move_to(point(left + corner, top));
            builder.line_to(point(right - corner, top));
            builder.arc_to(radii, px(0.0), false, true, point(right, top + corner));
            builder.line_to(point(right, bottom - corner));
            builder.arc_to(radii, px(0.0), false, true, point(right - corner, bottom));
            builder.line_to(point(left + corner, bottom));
            builder.arc_to(radii, px(0.0), false, true, point(left, bottom - corner));
            builder.line_to(point(left, top + corner));
            builder.arc_to(radii, px(0.0), false, true, point(left + corner, top));
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, hsla(0.0, 0.0, 1.0, 0.95));
            }
        },
    )
    .absolute()
    .left(px(-pad))
    .top(px(-pad))
    .right(px(-pad))
    .bottom(px(-pad))
}

/// padded label, frame in the views own points
/// drawn through layer_scale since gpui has no layer to scale a laid out label with
#[derive(IntoElement)]
struct PaddedLabel {
    text: SharedString,
    pixel_size: f32,
    layer_scale: f32,
    display_zoom: f32,
    soft_wraps: bool,
    alignment: EditorTextEntityTextAlignment,
    background: Option<Hsla>,
    text_color: Hsla,
    corner_radius: f32,
    content_insets: Size<f32>,
    casts_shadow: bool,
}

impl PaddedLabel {
    fn fitting_size(&self, max_text_width: f32, window: &Window) -> Size<f32> {
        let scale = self.layer_scale.max(f32::EPSILON);
        let text = editor_semibold_text_size(
            self.text.clone(),
            self.pixel_size * scale,
            (max_text_width > 0.0).then_some(max_text_width * scale),
            window,
        );
        size(
            (text.width / scale).ceil() + self.content_insets.width * 2.0,
            (text.height / scale).ceil() + self.content_insets.height * 2.0,
        )
    }

    fn fits_unwrapped(&self, max_text_width: f32, window: &Window) -> bool {
        if max_text_width <= 0.0 {
            return true;
        }
        let scale = self.layer_scale.max(f32::EPSILON);
        let unwrapped = editor_semibold_text_size(self.text.clone(), self.pixel_size * scale, None, window);
        unwrapped.width <= max_text_width * scale
    }

    fn text_layer(&self, color: Hsla) -> gpui::Div {
        let font_size = self.pixel_size * self.layer_scale;
        let mut layer = div()
            .w_full()
            .swiftui_font(Font::system_weight(font_size, FontWeight::SEMIBOLD))
            .text_color(color)
            .child(self.text.clone());
        if !self.soft_wraps {
            layer = layer.whitespace_nowrap();
        }
        match self.alignment {
            EditorTextEntityTextAlignment::Leading => layer.text_left(),
            EditorTextEntityTextAlignment::Center => layer.text_center(),
            EditorTextEntityTextAlignment::Trailing => layer.text_right(),
        }
    }
}

impl RenderOnce for PaddedLabel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let scale = self.layer_scale;
        let mut text = div().relative().w_full();
        if self.casts_shadow {
            text = text.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(scale * self.display_zoom))
                    .child(self.text_layer(hsla(0.0, 0.0, 0.0, 0.5 * 0.55))),
            );
        }
        let text = text.child(self.text_layer(self.text_color));
        let mut plate = div()
            .size_full()
            .flex()
            .items_center()
            .px(px(self.content_insets.width * scale))
            .py(px(self.content_insets.height * scale))
            .continuous_corners(self.corner_radius * scale)
            .child(text);
        if let Some(background) = self.background {
            plate = plate.bg(background);
        }
        plate
    }
}

/// size of semibold system text, widest line and all lines
/// wraps at wrap_width when given
pub(crate) fn editor_semibold_text_size(
    text: SharedString,
    pixel_size: f32,
    wrap_width: Option<f32>,
    window: &Window,
) -> Size<f32> {
    if text.is_empty() {
        return size(0.0, pixel_size * NATURAL_LINE_HEIGHT);
    }
    let spec = Font::system_weight(pixel_size, FontWeight::SEMIBOLD);
    let font = gpui::Font {
        family: spec.family(),
        features: spec.features(),
        fallbacks: None,
        weight: FontWeight::SEMIBOLD,
        style: spec.style(),
    };
    let run = TextRun {
        len: text.len(),
        font,
        color: hsla(0.0, 0.0, 0.0, 1.0),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line_height = px(pixel_size * NATURAL_LINE_HEIGHT);
    let Ok(lines) = window.text_system().shape_text(
        text,
        px(pixel_size),
        &[run],
        wrap_width.map(px),
        None,
    ) else {
        return size(0.0, pixel_size * NATURAL_LINE_HEIGHT);
    };
    lines.iter().fold(Size::default(), |total: Size<f32>, line| {
        let line_size = line.size(line_height);
        size(
            total.width.max(f32::from(line_size.width)),
            total.height + f32::from(line_size.height),
        )
    })
}

/// handle dot view, frame in the views own points
#[derive(IntoElement)]
struct HandleDotView {
    frame: Bounds<f32>,
    scale: f32,
    zoom: f32,
}

impl RenderOnce for HandleDotView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let scale = self.scale;
        let chrome = scale * self.zoom;
        let fill: Hsla = Rgba { r: 0.31, g: 0.55, b: 1.0, a: 1.0 }.into();
        div()
            .absolute()
            .left(px(self.frame.origin.x * scale))
            .top(px(self.frame.origin.y * scale))
            .w(px(self.frame.size.width * scale))
            .h(px(self.frame.size.height * scale))
            .circle()
            .bg(fill)
            .stroke_border(hsla(0.0, 0.0, 1.0, 1.0), 0.5 * chrome)
            .shadow(vec![BoxShadow {
                color: hsla(0.0, 0.0, 0.0, 0.55),
                offset: point(px(0.0), px(chrome)),
                blur_radius: px(2.0 * chrome),
                spread_radius: px(0.0),
                inset: false,
            }])
    }
}
