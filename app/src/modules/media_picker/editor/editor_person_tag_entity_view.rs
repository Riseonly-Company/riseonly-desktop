use std::rc::Rc;

use gpui::{
    App, BoxShadow, Context, DispatchPhase, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, Pixels, Point, Render, SharedString, Size, Styled, TextRun,
    Window, black, canvas, div, hsla, point, px, white,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics::SelectionFeedbackGenerator;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT};
use rise_ui::swiftui::shapes::SwiftUiShapes;

use crate::modules::media_picker::editor::editor_entity_snapping::{EditorEntitySnap, EditorEntitySnapFeedback};
use crate::modules::media_picker::editor::editor_state::EditorPersonTagEntity;

struct Metrics;

impl Metrics {
    const POINTER_HEIGHT: f32 = 6.0;
    const POINTER_WIDTH: f32 = 13.0;
    const HORIZONTAL_INSET: f32 = 11.0;
    const VERTICAL_INSET: f32 = 6.0;
    const CORNER_RADIUS: f32 = 9.0;
    const MINIMUM_WIDTH: f32 = 44.0;
    const FONT_SIZE: f32 = 13.0;
    const HIT_OUTSET: f32 = 8.0;
    const EDGE_INSET: f32 = 6.0;
    const SELECTION_OUTSET: f32 = 4.0;
    const PAN_SLOP: f32 = 4.0;
}

/// left button held on the bubble
#[derive(Clone, Copy)]
struct Press {
    start: Point<Pixels>,
    is_panning: bool,
}

/// person tag bubble on the canvas
/// drags and snaps to center lines like text but no scale or rotation since the tag is a single point
pub struct EditorPersonTagEntityView {
    pub entity_id: Uuid,
    current: Option<EditorPersonTagEntity>,
    canvas_size: Size<f32>,
    is_selected: bool,
    allows_body_drag: bool,
    move_start_anchor: Point<f32>,
    press: Option<Press>,
    bounds_size: Size<f32>,
    measured_title: Option<(SharedString, f32)>,
    display_zoom: f32,

    pub on_update: Option<Rc<dyn Fn(EditorPersonTagEntity, &mut App)>>,
    pub on_select: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_request_delete: Option<Rc<dyn Fn(Uuid, &mut App)>>,
    pub on_snap_guides_changed: Option<Rc<dyn Fn(bool, bool, &mut App)>>,

    snap_feedback: EditorEntitySnapFeedback,
}

impl EditorPersonTagEntityView {
    pub fn new(entity_id: Uuid, _cx: &mut Context<Self>) -> Self {
        Self {
            entity_id,
            current: None,
            canvas_size: Size { width: 0.0, height: 0.0 },
            is_selected: false,
            allows_body_drag: true,
            move_start_anchor: Point { x: 0.0, y: 0.0 },
            press: None,
            bounds_size: Size { width: Metrics::MINIMUM_WIDTH, height: Metrics::POINTER_HEIGHT },
            measured_title: None,
            display_zoom: 1.0,
            on_update: None,
            on_select: None,
            on_request_delete: None,
            on_snap_guides_changed: None,
            snap_feedback: EditorEntitySnapFeedback::new(),
        }
    }

    pub fn set_display_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        if self.display_zoom != zoom {
            self.display_zoom = zoom;
            cx.notify();
        }
    }

    pub fn set_interaction_state(&mut self, allows_body_drag: bool) {
        self.allows_body_drag = allows_body_drag;
    }

    pub fn apply(
        &mut self,
        entity: &EditorPersonTagEntity,
        canvas_size: Size<f32>,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) {
        self.canvas_size = canvas_size;
        self.current = Some(entity.clone());
        self.is_selected = is_selected;
        cx.notify();
    }

    fn measure_bounds(&mut self, title: &SharedString, window: &Window) {
        let zoom = self.display_zoom;
        if self.measured_title.as_ref().is_some_and(|(measured, at_zoom)| measured == title && *at_zoom == zoom) {
            return;
        }
        let font = Self::label_font();
        let run = TextRun {
            len: title.len(),
            font: gpui::Font {
                family: font.family(),
                features: font.features(),
                fallbacks: None,
                weight: font.weight,
                style: font.style(),
            },
            color: white(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(title.clone(), px(Metrics::FONT_SIZE * zoom), &[run], None);
        let width = (Metrics::MINIMUM_WIDTH * zoom).max(f32::from(line.width).ceil() + Metrics::HORIZONTAL_INSET * 2.0 * zoom);
        let pill_height = (Metrics::FONT_SIZE * zoom * NATURAL_LINE_HEIGHT).ceil() + Metrics::VERTICAL_INSET * 2.0 * zoom;
        self.bounds_size = Size { width, height: pill_height + Metrics::POINTER_HEIGHT * zoom };
        self.measured_title = Some((title.clone(), zoom));
    }

    fn label_font() -> Font {
        Font::system_weight(Metrics::FONT_SIZE, gpui::FontWeight::SEMIBOLD)
    }

    fn anchor(&self, entity: &EditorPersonTagEntity) -> Point<f32> {
        Point {
            x: entity.center.x * self.canvas_size.width,
            y: entity.center.y * self.canvas_size.height,
        }
    }

    /// same placement as the published post
    /// pointer tip on the tagged point, bubble kept inside the frame
    /// returns the top left corner
    fn place(&self, anchor: Point<f32>) -> Point<f32> {
        let size = self.bounds_size;
        let edge_inset = Metrics::EDGE_INSET * self.display_zoom;
        let max_left = edge_inset.max(self.canvas_size.width - size.width - edge_inset);
        let max_top = edge_inset.max(self.canvas_size.height - size.height - edge_inset);
        Point {
            x: edge_inset.max(anchor.x - size.width / 2.0).min(max_left),
            y: edge_inset.max(anchor.y).min(max_top),
        }
    }

    pub fn bubble_title(entity: &EditorPersonTagEntity) -> String {
        let tag = trimming_whitespaces_and_newlines(&entity.tag);
        if !tag.is_empty() {
            return if tag.starts_with('@') { tag.to_owned() } else { format!("@{tag}") };
        }
        let name = trimming_whitespaces_and_newlines(&entity.name);
        if name.is_empty() { "@".to_owned() } else { name.to_owned() }
    }

    pub fn contains_canvas_point(&self, location: Point<f32>) -> bool {
        let Some(entity) = self.current.as_ref() else {
            return false;
        };
        let origin = self.place(self.anchor(entity));
        let outset = Metrics::HIT_OUTSET * self.display_zoom;
        location.x >= origin.x - outset
            && location.x <= origin.x + self.bounds_size.width + outset
            && location.y >= origin.y - outset
            && location.y <= origin.y + self.bounds_size.height + outset
    }

    fn fire_select(&self, cx: &mut Context<Self>) {
        if let Some(on_select) = self.on_select.clone() {
            let id = self.entity_id;
            App::defer(cx, move |cx| on_select(id, cx));
        }
    }

    fn fire_update(&self, entity: EditorPersonTagEntity, cx: &mut Context<Self>) {
        if let Some(on_update) = self.on_update.clone() {
            App::defer(cx, move |cx| on_update(entity, cx));
        }
    }

    fn fire_snap_guides(&self, vertical: bool, horizontal: bool, cx: &mut Context<Self>) {
        if let Some(on_snap_guides_changed) = self.on_snap_guides_changed.clone() {
            App::defer(cx, move |cx| on_snap_guides_changed(vertical, horizontal, cx));
        }
    }

    fn handle_tap(&mut self, cx: &mut Context<Self>) {
        self.fire_select(cx);
        let Some(entity) = self.current.as_mut() else {
            return;
        };
        entity.is_dark = Some(!entity.uses_dark_style());
        let entity = entity.clone();
        SelectionFeedbackGenerator::new().selection_changed();
        cx.notify();
        self.fire_update(entity, cx);
    }

    fn pan_began(&mut self, cx: &mut Context<Self>) {
        let Some(entity) = self.current.as_ref().filter(|_| self.allows_body_drag) else {
            return;
        };
        self.move_start_anchor = self.anchor(entity);
        self.snap_feedback.begin();
        self.fire_select(cx);
    }

    fn pan_changed(&mut self, translation: Point<f32>, cx: &mut Context<Self>) {
        if !self.allows_body_drag || self.current.is_none() {
            return;
        }
        let proposed = Point {
            x: self.move_start_anchor.x + translation.x,
            y: self.move_start_anchor.y + translation.y,
        };
        let (snapped_point, snapped_x, snapped_y) = EditorEntitySnap::snap_center(proposed, self.canvas_size);
        self.snap_feedback.update_center(snapped_x, snapped_y);
        self.fire_snap_guides(snapped_x, snapped_y, cx);
        if self.canvas_size.width <= 0.0 || self.canvas_size.height <= 0.0 {
            return;
        }
        let Some(entity) = self.current.as_mut() else {
            return;
        };
        entity.center = Point {
            x: (snapped_point.x / self.canvas_size.width).max(0.0).min(1.0),
            y: (snapped_point.y / self.canvas_size.height).max(0.0).min(1.0),
        };
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
        let translation = Point {
            x: f32::from(event.position.x - press.start.x),
            y: f32::from(event.position.y - press.start.y),
        };
        if !press.is_panning {
            if translation.x.hypot(translation.y) < Metrics::PAN_SLOP {
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

    fn remove_menu(&self, cx: &mut Context<Self>) -> Vec<ContextMenuItem> {
        let view = cx.weak_entity();
        vec![ContextMenuItem::new(
            1,
            tr("media_editor_tag_remove"),
            None,
            "media_editor_tag_remove",
            move |_, cx| {
                let Ok((id, on_request_delete)) =
                    view.read_with(cx, |view, _| (view.entity_id, view.on_request_delete.clone()))
                else {
                    return;
                };
                if let Some(on_request_delete) = on_request_delete {
                    on_request_delete(id, cx);
                }
            },
        )
        .destructive(true)]
    }

    fn paint_selection_frame(pill_origin: Point<Pixels>, pill_size: Size<f32>, zoom: f32, window: &mut Window) {
        let outset = px(Metrics::SELECTION_OUTSET * zoom);
        let radius = px((Metrics::CORNER_RADIUS + Metrics::SELECTION_OUTSET) * zoom);
        let left = pill_origin.x - outset;
        let top = pill_origin.y - outset;
        let right = pill_origin.x + px(pill_size.width) + outset;
        let bottom = pill_origin.y + px(pill_size.height) + outset;
        let radii = point(radius, radius);
        let mut path = PathBuilder::stroke(px(zoom)).dash_array(&[px(5.0 * zoom), px(4.0 * zoom)]);
        path.move_to(point(left + radius, top));
        path.line_to(point(right - radius, top));
        path.arc_to(radii, px(0.0), false, true, point(right, top + radius));
        path.line_to(point(right, bottom - radius));
        path.arc_to(radii, px(0.0), false, true, point(right - radius, bottom));
        path.line_to(point(left + radius, bottom));
        path.arc_to(radii, px(0.0), false, true, point(left, bottom - radius));
        path.line_to(point(left, top + radius));
        path.arc_to(radii, px(0.0), false, true, point(left + radius, top));
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, white().opacity(0.95));
        }
    }
}

impl Render for EditorPersonTagEntityView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(entity) = self.current.clone() else {
            return div().into_any_element();
        };
        let title = SharedString::from(Self::bubble_title(&entity));
        self.measure_bounds(&title, window);

        let dark = entity.uses_dark_style();
        let pill_color: Hsla = if dark { hsla(0.0, 0.0, 0.08, 0.92) } else { white() };
        let label_color: Hsla = if dark { white() } else { hsla(0.0, 0.0, 0.06, 1.0) };

        let zoom = self.display_zoom;
        let hit_outset = Metrics::HIT_OUTSET * zoom;
        let pointer_height = Metrics::POINTER_HEIGHT * zoom;
        let size = self.bounds_size;
        let origin = self.place(self.anchor(&entity));
        let pill_size = Size { width: size.width, height: (size.height - pointer_height).max(0.0) };
        let is_selected = self.is_selected;
        let view = cx.weak_entity();
        let up_view = view.clone();

        let surface = canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                let left = bounds.origin.x + px(hit_outset);
                let top = bounds.origin.y + px(hit_outset);
                let mid = left + px(size.width / 2.0);
                let half_pointer = px(Metrics::POINTER_WIDTH * zoom / 2.0);
                let pointer_bottom = top + px(pointer_height);
                let mut pointer = PathBuilder::fill();
                pointer.add_polygon(
                    &[point(mid - half_pointer, pointer_bottom), point(mid, top), point(mid + half_pointer, pointer_bottom)],
                    true,
                );
                if let Ok(path) = pointer.build() {
                    window.paint_path(path, pill_color);
                }
                if is_selected {
                    Self::paint_selection_frame(point(left, pointer_bottom), pill_size, zoom, window);
                }

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
                let release_view = up_view.clone();
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

        let pill = div()
            .absolute()
            .left(px(hit_outset))
            .top(px(hit_outset + pointer_height))
            .w(px(pill_size.width))
            .h(px(pill_size.height))
            .bg(pill_color)
            .continuous_corners(Metrics::CORNER_RADIUS * zoom)
            .shadow(vec![BoxShadow {
                color: black().opacity(0.28),
                offset: point(px(0.0), px(2.0 * zoom)),
                blur_radius: px(6.0 * zoom),
                spread_radius: px(0.0),
                inset: false,
            }])
            .flex()
            .items_center()
            .justify_center()
            .px(px(Metrics::HORIZONTAL_INSET * zoom))
            .py(px(Metrics::VERTICAL_INSET * zoom))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(Metrics::FONT_SIZE * zoom))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(label_color)
            .child(title);

        let menu_id = SharedString::from(format!("editor-person-tag-menu-{}", self.entity_id));
        let items = self.remove_menu(cx);
        let body = div()
            .relative()
            .w(px(size.width + hit_outset * 2.0))
            .h(px(size.height + hit_outset * 2.0))
            .child(pill)
            .child(surface)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.fire_select(cx)),
            )
            .context_menu(menu_id, items);

        div()
            .absolute()
            .left(px(origin.x - hit_outset))
            .top(px(origin.y - hit_outset))
            .child(body)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.press = Some(Press { start: event.position, is_panning: false });
                    cx.notify();
                }),
            )
            .into_any_element()
    }
}
