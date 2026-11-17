use std::f32::consts::PI;
use std::rc::Rc;

use gpui::{
    App, Bounds, BorderStyle, ContentMask, Context, DispatchPhase, ElementId, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, Pixels, Point, RenderOnce, SharedString,
    Styled, TextAlign, TextRun, Window, canvas, div, point, px, quad, size, transparent_black, white,
};
use rise_platform::haptics::selection_changed;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::font::Font;

pub const DIAL_HEIGHT: f32 = 120.0;
const FULL_TURN: f64 = 360.0;
const DETENT_STEP: f64 = 90.0;
const DETENT_RANGE: f64 = 1.5;
const LABEL_FONT_SIZE: f32 = 9.0;
const LABEL_HALF_GLYPH: f32 = 5.0;
const LABEL_EDGE_FADE: f32 = 6.0;
const ARC_DEPTH_PER_WIDTH: f32 = 0.3;
const TRIANGLE_ROOM: f32 = 18.0;
const LABEL_MIN_GAP: f32 = 22.0;
const DOT_MIN_GAP: f32 = 2.0;

/// dial state
/// where the press began and how wide the dial was painted
struct EditorRotationDialState {
    drag_start_degrees: f64,
    is_dragging: bool,
    is_in_detent: bool,
    press_x: Pixels,
    press_width: f32,
    width: f32,
}

/// dial that turns the crop, ticks on a wheel arc below a fixed triangle
#[derive(IntoElement)]
pub struct EditorRotationDial {
    id: ElementId,
    degrees: Binding<f64>,
    on_change: Rc<dyn Fn(f64, &mut Window, &mut App)>,
    on_interaction_change: Rc<dyn Fn(bool, &mut Window, &mut App)>,
}

impl EditorRotationDial {
    pub fn new(
        id: impl Into<ElementId>,
        degrees: Binding<f64>,
        on_change: impl Fn(f64, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            degrees,
            on_change: Rc::new(on_change),
            on_interaction_change: Rc::new(|_, _, _| {}),
        }
    }

    pub fn on_interaction_change(mut self, action: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_interaction_change = Rc::new(action);
        self
    }

    pub fn height(width: f32) -> f32 {
        (width * ARC_DEPTH_PER_WIDTH + TRIANGLE_ROOM).min(DIAL_HEIGHT)
    }

    pub fn pixels_per_degree(width: f32) -> f32 {
        (width / 90.0).max(1.0)
    }

    /// angle folded into -180 up to 180, the dial turns freely past a full turn
    pub fn wrapped(degrees: f64) -> f64 {
        degrees - FULL_TURN * ((degrees + FULL_TURN / 2.0) / FULL_TURN).floor()
    }

    /// the right angle the dial clicks into when the angle is close to it
    pub fn detent(degrees: f64) -> Option<f64> {
        let nearest = (degrees / DETENT_STEP).round() * DETENT_STEP;
        ((degrees - nearest).abs() <= DETENT_RANGE).then_some(nearest)
    }

    fn label_text(tick: i64) -> SharedString {
        let folded = tick.rem_euclid(360);
        let value = if folded > 180 { folded - 360 } else { folded };
        SharedString::from(value.to_string())
    }

    fn paint_ticks(degrees: f64, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            Self::paint_wheel(degrees, bounds, window, cx)
        });
    }

    fn paint_wheel(degrees: f64, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let center_x = width / 2.0;
        let arc_bottom = height - TRIANGLE_ROOM;
        let half_span = (width / 2.0 - 6.0).max(1.0);
        let arc_depth = arc_bottom.max(1.0);
        let radius = ((half_span * half_span + arc_depth * arc_depth) / (2.0 * arc_depth)).max(half_span);
        let half_angle = (half_span / radius).min(1.0).asin();
        let pixels_per_degree = Self::pixels_per_degree(width);
        let radians_per_degree = pixels_per_degree / radius;
        let wheel_center = Point { x: center_x, y: arc_bottom - radius };
        let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));

        let mut label_marks = PathBuilder::stroke(px(1.2));
        let mut major_marks = PathBuilder::stroke(px(1.0));
        let label_mark_color: Hsla = white().opacity(0.9);
        let major_mark_color: Hsla = white().opacity(0.58);
        let dot_color: Hsla = white().opacity(0.55);
        let label_color: Hsla = white().opacity(0.86);
        let font = Font::system_weight(LABEL_FONT_SIZE, gpui::FontWeight::MEDIUM);

        let label_step = if pixels_per_degree * 10.0 >= LABEL_MIN_GAP { 10 } else { 20 };
        let shows_dots = pixels_per_degree >= DOT_MIN_GAP;
        let visible_degrees = f64::from(half_angle / radians_per_degree.max(f32::EPSILON)) + 1.0;
        let first_tick = (degrees - visible_degrees).floor() as i64;
        let last_tick = (degrees + visible_degrees).ceil() as i64;
        for tick in first_tick..=last_tick {
            let relative = tick as f64 - degrees;
            let turn = relative as f32 * radians_per_degree;
            if turn.abs() > half_angle {
                continue;
            }
            let angle = PI / 2.0 - turn;
            let is_label = tick % label_step == 0;
            let is_major = tick % 5 == 0;
            let outer_x = wheel_center.x + angle.cos() * radius;
            let outer_y = wheel_center.y + angle.sin() * radius;
            if is_major {
                let length = if is_label { 8.0 } else { 5.0 };
                let inner_radius = radius - length;
                let inner = at(wheel_center.x + angle.cos() * inner_radius, wheel_center.y + angle.sin() * inner_radius);
                let marks = if is_label { &mut label_marks } else { &mut major_marks };
                marks.move_to(inner);
                marks.line_to(at(outer_x, outer_y));
            } else if shows_dots {
                window.paint_quad(quad(
                    Bounds { origin: at(outer_x - 1.0, outer_y - 1.0), size: size(px(2.0), px(2.0)) },
                    px(1.0),
                    dot_color,
                    px(0.0),
                    transparent_black(),
                    BorderStyle::Solid,
                ));
            }

            let label_radius = radius - 21.0;
            let label_center_y = wheel_center.y + angle.sin() * label_radius;
            let label_fade = ((label_center_y - LABEL_HALF_GLYPH) / LABEL_EDGE_FADE).clamp(0.0, 1.0);
            if is_label && label_fade > 0.0 {
                let text = Self::label_text(tick);
                let run = TextRun {
                    len: text.len(),
                    font: gpui::Font {
                        family: font.family(),
                        features: font.features(),
                        fallbacks: None,
                        weight: font.weight,
                        style: font.style(),
                    },
                    color: label_color.opacity(label_fade),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(text, px(LABEL_FONT_SIZE), &[run], None);
                let line_height = window.line_height();
                let origin = point(
                    bounds.origin.x + px(wheel_center.x + angle.cos() * label_radius) - line.width / 2.0,
                    bounds.origin.y + px(label_center_y) - line_height / 2.0,
                );
                let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
            }
        }
        if let Ok(path) = label_marks.build() {
            window.paint_path(path, label_mark_color);
        }
        if let Ok(path) = major_marks.build() {
            window.paint_path(path, major_mark_color);
        }

        let mut triangle = PathBuilder::fill();
        triangle.add_polygon(&Triangle::path(center_x, arc_bottom + 8.0, &at), true);
        if let Ok(path) = triangle.build() {
            window.paint_path(path, white());
        }
    }
}

struct Triangle;

impl Triangle {
    /// path for a 12 by 8 frame centered on x and y
    fn path(x: f32, y: f32, at: &impl Fn(f32, f32) -> Point<Pixels>) -> [Point<Pixels>; 3] {
        [at(x, y - 4.0), at(x + 6.0, y + 4.0), at(x - 6.0, y + 4.0)]
    }
}

impl RenderOnce for EditorRotationDial {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _: &mut Context<EditorRotationDialState>| {
            EditorRotationDialState {
                drag_start_degrees: 0.0,
                is_dragging: false,
                is_in_detent: false,
                press_x: px(0.0),
                press_width: 0.0,
                width: 0.0,
            }
        });
        let current = self.degrees.get(cx);

        let paint_state = state.clone();
        let degrees = self.degrees.clone();
        let on_change = self.on_change.clone();
        let on_interaction_change = self.on_interaction_change.clone();
        let surface = canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                paint_state.update(cx, |state, _| state.width = f32::from(bounds.size.width));
                Self::paint_ticks(current, bounds, window, cx);

                let state = paint_state.clone();
                let degrees_on_move = degrees.clone();
                let on_interaction_on_move = on_interaction_change.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || !state.read(cx).is_dragging {
                        return;
                    }
                    if event.pressed_button != Some(MouseButton::Left) {
                        state.update(cx, |state, _| state.is_dragging = false);
                        on_interaction_on_move(false, window, cx);
                        return;
                    }
                    let (start, press_x, width) = {
                        let state = state.read(cx);
                        (state.drag_start_degrees, state.press_x, state.press_width)
                    };
                    let delta = -f32::from(event.position.x - press_x) / Self::pixels_per_degree(width);
                    let raw = start + f64::from(delta);
                    let detent = Self::detent(raw);
                    let entered_detent = state.update(cx, |state, _| {
                        let entered = detent.is_some() && !state.is_in_detent;
                        state.is_in_detent = detent.is_some();
                        entered
                    });
                    if entered_detent {
                        let _ = selection_changed();
                    }
                    let new = Self::wrapped(detent.unwrap_or(raw));
                    if (new - degrees_on_move.get(cx)).abs() > 0.05 {
                        degrees_on_move.set(new, window, cx);
                        on_change(new, window, cx);
                    }
                });

                let state = paint_state.clone();
                let degrees_on_up = degrees.clone();
                let on_interaction_on_up = on_interaction_change.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Left
                        || !state.read(cx).is_dragging
                    {
                        return;
                    }
                    let end = degrees_on_up.get(cx);
                    state.update(cx, |state, _| {
                        state.drag_start_degrees = end;
                        state.is_dragging = false;
                    });
                    on_interaction_on_up(false, window, cx);
                });
            },
        )
        .absolute()
        .inset_0();

        let press_state = state;
        let press_degrees = self.degrees;
        let press_interaction = self.on_interaction_change;
        div()
            .id(self.id)
            .relative()
            .size_full()
            .child(surface)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                let start = press_degrees.get(cx);
                press_state.update(cx, |state, _| {
                    state.drag_start_degrees = start;
                    state.is_dragging = true;
                    state.is_in_detent = Self::detent(start).is_some();
                    state.press_x = event.position.x;
                    state.press_width = state.width;
                });
                press_interaction(true, window, cx);
            })
    }
}
