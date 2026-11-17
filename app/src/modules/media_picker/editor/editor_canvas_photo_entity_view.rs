use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, DispatchPhase, Hitbox, HitboxBehavior, ImageSource, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ObjectFit, ParentElement, Pixels, Point, Render, RenderImage, SharedString, Size, Styled, StyledImage, Window,
    canvas, div, img, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle, SelectionFeedbackGenerator};
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::rotation_effect::RotationEffectModifiers;

use crate::modules::media_picker::editor::editor_entity_snapping::{EditorEntitySnap, EditorEntitySnapFeedback};
use crate::modules::media_picker::editor::editor_link_entity::paint_dashed_selection;
use crate::modules::media_picker::editor::editor_two_finger_transform::EditorTwoFingerTransformable;
use crate::modules::media_picker::editor::story_canvas_model::{
    EditorCanvasPhotoEntity, StoryCanvasGeometry, StoryCanvasPhotoStore,
};

struct EditorCanvasPhotoEntityViewMetrics;

impl EditorCanvasPhotoEntityViewMetrics {
    const SELECTION_OUTSET: f32 = 4.0;
    const SHAPE_ANIMATION: Duration = Duration::from_millis(180);
    const PAN_SLOP: f32 = 4.0;
}

#[derive(Clone, Copy)]
struct EditorCanvasPhotoPress {
    start: Point<Pixels>,
    is_panning: bool,
}

/// implicit layer animation of a shape change, box and corner radius from their old values
#[derive(Clone, Copy)]
struct EditorCanvasPhotoShapeAnimation {
    started: Duration,
    from_box: Size<f32>,
    from_radius: f32,
}

/// a photo on the story canvas
/// a drag moves it and the canvas scales and turns it
/// a click focuses it and a click on the focused one cycles the shape square then rounded then circle
pub struct EditorCanvasPhotoEntityView {
    pub entity_id: Uuid,
    image: Option<Arc<RenderImage>>,
    current: Option<EditorCanvasPhotoEntity>,
    shown_key: Option<String>,
    canvas_size: Size<f32>,
    is_selected: bool,
    allows_body_drag: bool,
    move_start_center: Point<f32>,
    press: Option<EditorCanvasPhotoPress>,
    two_finger_start_scale: f32,
    two_finger_start_rotation: f32,
    shape_animation: Option<EditorCanvasPhotoShapeAnimation>,

    pub allows_delete: bool,
    pub on_update: Option<Rc<dyn Fn(EditorCanvasPhotoEntity, &mut App)>>,
    pub on_select: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_request_delete: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_snap_guides_changed: Option<Rc<dyn Fn(bool, bool, &mut App)>>,

    snap_feedback: EditorEntitySnapFeedback,
}

impl EditorCanvasPhotoEntityView {
    pub fn new(entity_id: Uuid, _cx: &mut Context<Self>) -> Self {
        Self {
            entity_id,
            image: None,
            current: None,
            shown_key: None,
            canvas_size: size(0.0, 0.0),
            is_selected: false,
            allows_body_drag: true,
            move_start_center: point(0.0, 0.0),
            press: None,
            two_finger_start_scale: 1.0,
            two_finger_start_rotation: 0.0,
            shape_animation: None,
            allows_delete: true,
            on_update: None,
            on_select: None,
            on_request_delete: None,
            on_snap_guides_changed: None,
            snap_feedback: EditorEntitySnapFeedback::new(),
        }
    }

    pub fn set_interaction_state(&mut self, allows_body_drag: bool, cx: &mut Context<Self>) {
        self.allows_body_drag = allows_body_drag;
        if !allows_body_drag {
            self.press = None;
        }
        cx.notify();
    }

    pub fn apply(
        &mut self,
        entity: &EditorCanvasPhotoEntity,
        canvas_size: Size<f32>,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) {
        self.canvas_size = canvas_size;
        self.current = Some(entity.clone());
        self.is_selected = is_selected;
        self.show_image_if_needed(entity, cx);
        cx.notify();
    }

    fn show_image_if_needed(&mut self, entity: &EditorCanvasPhotoEntity, cx: &mut Context<Self>) {
        if self.shown_key.as_deref() == Some(entity.photo_key.as_str()) {
            return;
        }
        let key = entity.photo_key.clone();
        self.shown_key = Some(key.clone());
        if let Some(hit) = StoryCanvasPhotoStore::shared().preview(&key) {
            self.image = Some(hit);
            return;
        }
        let view = cx.weak_entity();
        StoryCanvasPhotoStore::shared().load_preview(&entity.photo_key, cx, move |image, cx| {
            let Some(image) = image else {
                return;
            };
            cx.defer(move |cx| {
                let _ = view.update(cx, |this, cx| {
                    if this.shown_key.as_deref() == Some(key.as_str()) {
                        this.image = Some(image);
                        cx.notify();
                    }
                });
            });
        });
    }

    fn fire_select(&self, cx: &mut Context<Self>) {
        if let Some(on_select) = self.on_select.clone() {
            let id = self.entity_id;
            App::defer(cx, move |cx| on_select(id, cx));
        }
    }

    fn fire_update(&self, entity: EditorCanvasPhotoEntity, cx: &mut Context<Self>) {
        if let Some(on_update) = self.on_update.clone() {
            App::defer(cx, move |cx| on_update(entity, cx));
        }
    }

    fn fire_snap_guides(&self, vertical: bool, horizontal: bool, cx: &mut Context<Self>) {
        if let Some(on_snap_guides_changed) = self.on_snap_guides_changed.clone() {
            App::defer(cx, move |cx| on_snap_guides_changed(vertical, horizontal, cx));
        }
    }

    fn center(&self) -> Point<f32> {
        self.current.as_ref().map_or(point(0.0, 0.0), |entity| {
            point(entity.center.x * self.canvas_size.width, entity.center.y * self.canvas_size.height)
        })
    }

    /// true if the location hits the shaped box, turned and scaled with the view
    /// location is relative to the canvas
    pub fn point(&self, location: Point<f32>) -> bool {
        let Some(entity) = self.current.as_ref() else {
            return false;
        };
        let box_size = StoryCanvasGeometry::shaped_box(entity, self.canvas_size);
        let center = self.center();
        let (sin, cos) = entity.rotation.sin_cos();
        let scale = entity.scale.max(f32::EPSILON);
        let dx = location.x - center.x;
        let dy = location.y - center.y;
        let local_x = (dx * cos + dy * sin) / scale;
        let local_y = (-dx * sin + dy * cos) / scale;
        local_x.abs() <= box_size.width / 2.0 && local_y.abs() <= box_size.height / 2.0
    }

    fn handle_tap(&mut self, cx: &mut Context<Self>) {
        if !self.is_selected {
            self.fire_select(cx);
            return;
        }
        let Some(mut entity) = self.current.clone() else {
            return;
        };
        let from_box = StoryCanvasGeometry::shaped_box(&entity, self.canvas_size);
        let from_radius = StoryCanvasGeometry::corner_radius(&entity, from_box);
        entity.shape = entity.next_shape().wire().to_owned();
        SelectionFeedbackGenerator::new().selection_changed();
        self.shape_animation = Some(EditorCanvasPhotoShapeAnimation { started: RiseClock::monotonic(), from_box, from_radius });
        let (canvas_size, is_selected) = (self.canvas_size, self.is_selected);
        self.apply(&entity, canvas_size, is_selected, cx);
        self.fire_update(entity, cx);
    }

    fn handle_hold(&mut self, cx: &mut Context<Self>) {
        if !self.allows_delete {
            return;
        }
        self.fire_select(cx);
        ImpactFeedbackGenerator::new(ImpactStyle::Medium).impact_occurred();
    }

    fn edit_menu_interaction(&self, cx: &mut Context<Self>) -> Vec<ContextMenuItem> {
        let view = cx.weak_entity();
        vec![ContextMenuItem::new(1, tr("ctx_delete"), None, "ctx_delete", move |_, cx| {
            let Ok((id, callback)) = view.read_with(cx, |view, _| (view.entity_id, view.on_request_delete.clone())) else {
                return;
            };
            if let Some(callback) = callback {
                callback(id, cx);
            }
        })
        .destructive(true)]
    }

    fn pan_began(&mut self, cx: &mut Context<Self>) {
        if !self.allows_body_drag || self.current.is_none() {
            return;
        }
        self.move_start_center = self.center();
        self.snap_feedback.begin();
        self.fire_select(cx);
    }

    fn pan_changed(&mut self, translation: Point<f32>, cx: &mut Context<Self>) {
        if !self.allows_body_drag || self.current.is_none() {
            return;
        }
        let proposed = point(self.move_start_center.x + translation.x, self.move_start_center.y + translation.y);
        let (snapped, snapped_x, snapped_y) = EditorEntitySnap::snap_center(proposed, self.canvas_size);
        self.snap_feedback.update_center(snapped_x, snapped_y);
        self.fire_snap_guides(snapped_x, snapped_y, cx);
        if self.canvas_size.width <= 0.0 || self.canvas_size.height <= 0.0 {
            return;
        }
        let Some(entity) = self.current.as_mut() else {
            return;
        };
        entity.center = point(snapped.x / self.canvas_size.width, snapped.y / self.canvas_size.height);
        let entity = entity.clone();
        cx.notify();
        self.fire_update(entity, cx);
    }

    fn pointer_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(mut press) = self.press else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.pointer_released(cx);
            return;
        }
        let translation = point(
            f32::from(event.position.x - press.start.x),
            f32::from(event.position.y - press.start.y),
        );
        if !press.is_panning {
            if translation.x.hypot(translation.y) < EditorCanvasPhotoEntityViewMetrics::PAN_SLOP {
                return;
            }
            press.is_panning = true;
            self.press = Some(press);
            self.pan_began(cx);
        }
        self.pan_changed(translation, cx);
    }

    fn pointer_released(&mut self, cx: &mut Context<Self>) {
        let Some(press) = self.press.take() else {
            return;
        };
        if press.is_panning {
            if self.allows_body_drag && self.current.is_some() {
                self.fire_snap_guides(false, false, cx);
            }
        } else {
            self.handle_tap(cx);
        }
    }
}

impl EditorTwoFingerTransformable for EditorCanvasPhotoEntityView {
    fn entity_id(&self) -> Uuid {
        self.entity_id
    }

    fn begin_two_finger_transform(&mut self, cx: &mut Context<Self>) {
        let Some(entity) = self.current.as_ref() else {
            return;
        };
        self.two_finger_start_scale = entity.scale;
        self.two_finger_start_rotation = entity.rotation;
        self.snap_feedback.begin();
        self.fire_select(cx);
    }

    fn apply_two_finger_transform(&mut self, scale: f32, rotation: f32, cx: &mut Context<Self>) {
        let range = StoryCanvasGeometry::PHOTO_SCALE_RANGE;
        let (angle, snapped) = EditorEntitySnap::snap_rotation(self.two_finger_start_rotation + rotation);
        self.snap_feedback.update_rotation(snapped);
        let start_scale = self.two_finger_start_scale;
        let Some(entity) = self.current.as_mut() else {
            return;
        };
        entity.scale = (start_scale * scale).max(*range.start()).min(*range.end());
        entity.rotation = angle;
        let entity = entity.clone();
        cx.notify();
        self.fire_update(entity, cx);
    }

    fn end_two_finger_transform(&mut self, _cx: &mut Context<Self>) {}
}

impl Render for EditorCanvasPhotoEntityView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(entity) = self.current.clone() else {
            return div().into_any_element();
        };
        let target_box = StoryCanvasGeometry::shaped_box(&entity, self.canvas_size);
        let target_radius = StoryCanvasGeometry::corner_radius(&entity, target_box);
        let (box_size, radius) = match self.shape_animation {
            Some(animation) => {
                let total = EditorCanvasPhotoEntityViewMetrics::SHAPE_ANIMATION.as_secs_f32();
                let t = (RiseClock::monotonic().saturating_sub(animation.started).as_secs_f32() / total).clamp(0.0, 1.0);
                if t >= 1.0 {
                    self.shape_animation = None;
                } else {
                    window.request_animation_frame();
                }
                let eased = t * t * (3.0 - 2.0 * t);
                let lerp = |from: f32, to: f32| from + (to - from) * eased;
                (
                    size(
                        lerp(animation.from_box.width, target_box.width),
                        lerp(animation.from_box.height, target_box.height),
                    ),
                    lerp(animation.from_radius, target_radius),
                )
            }
            None => (target_box, target_radius),
        };

        let scale = entity.scale;
        let center = self.center();
        let frame = size(box_size.width * scale, box_size.height * scale);
        let outset = EditorCanvasPhotoEntityViewMetrics::SELECTION_OUTSET * scale;
        let (sin, cos) = entity.rotation.sin_cos();
        let half_w = frame.width / 2.0 + outset;
        let half_h = frame.height / 2.0 + outset;
        let reach = size(half_w * cos.abs() + half_h * sin.abs(), half_w * sin.abs() + half_h * cos.abs());
        let hit_origin = point(center.x - reach.width, center.y - reach.height);

        let mut turned = div().relative().w(px(frame.width)).h(px(frame.height));
        if let Some(image) = self.image.clone() {
            turned = turned.child(
                img(ImageSource::Render(image))
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded(px(radius * scale)),
            );
        }
        if self.is_selected {
            let selection = size(frame.width + outset * 2.0, frame.height + outset * 2.0);
            let selection_radius = (radius + EditorCanvasPhotoEntityViewMetrics::SELECTION_OUTSET) * scale;
            turned = turned.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, _| {
                        paint_dashed_selection(bounds.origin, selection, selection_radius, scale, window);
                    },
                )
                .absolute()
                .left(px(-outset))
                .top(px(-outset))
                .w(px(selection.width))
                .h(px(selection.height)),
            );
        }

        let positioned = div()
            .absolute()
            .left(px(center.x - frame.width / 2.0 - hit_origin.x))
            .top(px(center.y - frame.height / 2.0 - hit_origin.y))
            .child(turned.rotation_effect(entity.rotation.to_degrees()));

        let mut body = div()
            .absolute()
            .left(px(hit_origin.x))
            .top(px(hit_origin.y))
            .w(px(reach.width * 2.0))
            .h(px(reach.height * 2.0))
            .child(positioned);

        if self.allows_body_drag {
            let view = cx.weak_entity();
            let gestures = canvas(
                |bounds: Bounds<Pixels>, window: &mut Window, _: &mut App| {
                    window.insert_hitbox(bounds, HitboxBehavior::Normal)
                },
                move |bounds: Bounds<Pixels>, hitbox: Hitbox, window: &mut Window, _: &mut App| {
                    let down_view = view.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble
                            || event.button != MouseButton::Left
                            || !hitbox.is_hovered(window)
                        {
                            return;
                        }
                        if !bounds.contains(&event.position) {
                            return;
                        }
                        let local = point(
                            f32::from(event.position.x - bounds.origin.x) + hit_origin.x,
                            f32::from(event.position.y - bounds.origin.y) + hit_origin.y,
                        );
                        let inside = down_view.read_with(cx, |view, _| view.point(local)).unwrap_or(false);
                        if !inside {
                            return;
                        }
                        cx.stop_propagation();
                        let _ = down_view.update(cx, |view, cx| {
                            view.press = Some(EditorCanvasPhotoPress { start: event.position, is_panning: false });
                            cx.notify();
                        });
                    });
                    let move_view = view.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let is_pressed = move_view.read_with(cx, |view, _| view.press.is_some()).unwrap_or(false);
                        if is_pressed {
                            let _ = move_view.update(cx, |view, cx| view.pointer_moved(event, cx));
                        }
                    });
                    let release_view = view.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                            return;
                        }
                        let is_pressed = release_view.read_with(cx, |view, _| view.press.is_some()).unwrap_or(false);
                        if is_pressed {
                            let _ = release_view.update(cx, |view, cx| view.pointer_released(cx));
                        }
                    });
                },
            )
            .absolute()
            .inset_0();
            body = body.child(gestures);
            if self.allows_delete {
                let menu_id = SharedString::from(format!("editor-canvas-photo-menu-{}", self.entity_id));
                let items = self.edit_menu_interaction(cx);
                return body
                    .on_mouse_down(MouseButton::Right, cx.listener(|this, _: &MouseDownEvent, _, cx| this.handle_hold(cx)))
                    .context_menu(menu_id, items)
                    .into_any_element();
            }
        }
        body.into_any_element()
    }
}
