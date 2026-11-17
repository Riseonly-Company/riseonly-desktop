use std::ops::RangeInclusive;
use std::rc::Rc;

use gpui::AppContext;
use gpui::{
    App, BoxShadow, Bounds, ElementId, Empty, Entity, FocusHandle, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement, Pixels,
    Render, RenderOnce, Size, StatefulInteractiveElement, Styled, Window, black, container_query,
    div, point, px, white,
};

use crate::swiftui::binding::Binding;
use crate::swiftui::measure::measure;
use crate::swiftui::shapes::SwiftUiShapes;

pub type SliderEditingChanged = Rc<dyn Fn(bool, &mut Window, &mut App)>;

pub const TRACK_HEIGHT: f32 = 4.0;
pub const KNOB_SIZE: f32 = 28.0;
pub const SLIDER_HEIGHT: f32 = 31.0;
pub const KEYBOARD_FRACTION: f64 = 0.01;
const EPSILON: f64 = 1e-9;

/// value rules of the slider apart from drawing, what a pointer position or key resolves to
#[derive(Clone, Debug, PartialEq)]
pub struct SliderScale {
    pub range: RangeInclusive<f64>,
    pub step: Option<f64>,
}

impl SliderScale {
    pub fn new(range: RangeInclusive<f64>, step: Option<f64>) -> Self {
        Self { range, step }
    }

    /// width of the range, never zero so a fraction is always defined
    pub fn span(&self) -> f64 {
        EPSILON.max(self.range.end() - self.range.start())
    }

    pub fn clamp(&self, raw: f64) -> f64 {
        self.range.end().min(self.range.start().max(raw))
    }

    /// snaps the value to lower bound plus n steps, inside the range
    pub fn quantize(&self, raw: f64) -> f64 {
        match self.step {
            Some(step) if step > EPSILON => {
                let steps = ((raw - self.range.start()) / step).round();
                self.clamp(self.range.start() + steps * step)
            }
            _ => self.clamp(raw),
        }
    }

    /// where the value sits along the range, 0 to 1
    pub fn fraction(&self, value: f64) -> f64 {
        let fraction = (self.clamp(value) - self.range.start()) / self.span();
        1f64.min(0f64.max(fraction))
    }

    /// value under location_x measured from the left edge of a slider this wide
    /// knob center travels from half a knob in from the left to half a knob in from the right
    pub fn value_at(&self, location_x: f32, width: f32) -> f64 {
        let usable = width - KNOB_SIZE;
        if usable <= 0.0 {
            return self.quantize(*self.range.start());
        }
        let raw_fraction = f64::from((location_x - KNOB_SIZE / 2.0) / usable);
        let fraction = 1f64.min(0f64.max(raw_fraction));
        self.quantize(self.range.start() + fraction * self.span())
    }

    /// distance one arrow key press moves the value
    pub fn keyboard_step(&self) -> f64 {
        match self.step {
            Some(step) if step > EPSILON => step,
            _ => self.span() * KEYBOARD_FRACTION,
        }
    }

    /// one arrow key press toward direction (-1 or 1), none at the end of the range
    pub fn stepped(&self, previous: f64, direction: f64) -> Option<f64> {
        let next = self.quantize(self.clamp(previous) + direction * self.keyboard_step());
        ((next - previous).abs() > EPSILON).then_some(next)
    }
}

pub fn slider(id: impl Into<ElementId>, value: Binding<f64>, range: RangeInclusive<f64>) -> Slider {
    Slider {
        id: id.into(),
        value,
        scale: SliderScale::new(range, None),
        tint: None,
        on_editing_changed: None,
    }
}

#[derive(IntoElement)]
pub struct Slider {
    id: ElementId,
    value: Binding<f64>,
    scale: SliderScale,
    tint: Option<Hsla>,
    on_editing_changed: Option<SliderEditingChanged>,
}

impl Slider {
    /// tint of the filled part of the track
    pub fn tint(mut self, color: Hsla) -> Self {
        self.tint = Some(color);
        self
    }

    pub fn step(mut self, step: f64) -> Self {
        self.scale.step = Some(step);
        self
    }

    /// called with true when editing starts and false when it ends
    pub fn on_editing_changed(mut self, action: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_editing_changed = Some(Rc::new(action));
        self
    }
}

/// whether a drag is running, laid out bounds for the pointer and focus for arrow keys
pub struct SliderState {
    is_editing: bool,
    bounds: Option<Bounds<Pixels>>,
    focus: FocusHandle,
}

impl SliderState {
    fn new(focus: FocusHandle) -> Self {
        Self { is_editing: false, bounds: None, focus }
    }
}

/// type of the gpui drag that carries the knob across the window
#[derive(Clone, Copy)]
struct SliderDrag;

struct SliderDragGhost;

impl Render for SliderDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
struct SliderHandlers {
    state: Entity<SliderState>,
    value: Binding<f64>,
    scale: Rc<SliderScale>,
    on_editing_changed: Option<SliderEditingChanged>,
}

impl SliderHandlers {
    fn current(&self, cx: &App) -> f64 {
        self.scale.clamp(self.value.get(cx))
    }

    fn editing_changed(&self, is_editing: bool, window: &mut Window, cx: &mut App) {
        if let Some(action) = &self.on_editing_changed {
            action(is_editing, window, cx);
        }
    }

    /// writes next through the binding if it differs from the shown value
    fn write(&self, next: f64, window: &mut Window, cx: &mut App) {
        if (next - self.current(cx)).abs() > EPSILON {
            self.value.set(next, window, cx);
        }
    }

    /// position_x is in window coords, mapped through bounds
    fn apply(&self, position_x: Pixels, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let width = f32::from(bounds.size.width);
        let location_x = f32::from(position_x - bounds.origin.x);
        let next = self.scale.value_at(location_x, width);
        self.write(next, window, cx);
    }

    fn begin(&self, position_x: Pixels, window: &mut Window, cx: &mut App) {
        let (was_editing, bounds) = {
            let state = self.state.read(cx);
            (state.is_editing, state.bounds)
        };
        if !was_editing {
            self.state.update(cx, |state, cx| {
                state.is_editing = true;
                cx.notify();
            });
            self.editing_changed(true, window, cx);
        }
        if let Some(bounds) = bounds {
            self.apply(position_x, bounds, window, cx);
        }
    }

    fn end(&self, position_x: Option<Pixels>, window: &mut Window, cx: &mut App) {
        let (is_editing, bounds) = {
            let state = self.state.read(cx);
            (state.is_editing, state.bounds)
        };
        if !is_editing {
            return;
        }
        if let (Some(position_x), Some(bounds)) = (position_x, bounds) {
            self.apply(position_x, bounds, window, cx);
        }
        self.state.update(cx, |state, cx| {
            state.is_editing = false;
            cx.notify();
        });
        self.editing_changed(false, window, cx);
    }

    fn key(&self, direction: f64, window: &mut Window, cx: &mut App) {
        let previous = self.current(cx);
        let Some(next) = self.scale.stepped(previous, direction) else { return };
        self.editing_changed(true, window, cx);
        self.value.set(next, window, cx);
        self.editing_changed(false, window, cx);
    }
}

fn slider_body(size: Size<Pixels>, fraction: f64, tint: Hsla, track: Hsla) -> gpui::Div {
    let width = KNOB_SIZE.max(f32::from(size.width));
    let height = f32::from(size.height);
    let usable = width - KNOB_SIZE;
    let knob_x = KNOB_SIZE / 2.0 + usable * fraction as f32;
    let centred = |side: f32| px((height - side) / 2.0);

    div()
        .relative()
        .size_full()
        .child(div().absolute().left_0().top(centred(TRACK_HEIGHT)).w(px(width)).h(px(TRACK_HEIGHT)).capsule().bg(track))
        .child(
            div()
                .absolute()
                .left_0()
                .top(centred(TRACK_HEIGHT))
                .w(px(TRACK_HEIGHT.max(knob_x)))
                .h(px(TRACK_HEIGHT))
                .capsule()
                .bg(tint),
        )
        .child(
            div().absolute().left(px(knob_x - KNOB_SIZE / 2.0)).top(centred(KNOB_SIZE)).child(
                div().size(px(KNOB_SIZE)).circle().bg(white()).shadow(vec![
                    BoxShadow {
                        color: black().opacity(0.15),
                        offset: point(px(0.0), px(3.0)),
                        blur_radius: px(8.0),
                        spread_radius: px(0.0),
                        inset: false,
                    },
                    BoxShadow {
                        color: black().opacity(0.16),
                        offset: point(px(0.0), px(0.5)),
                        blur_radius: px(1.0),
                        spread_radius: px(0.0),
                        inset: false,
                    },
                ]),
            ),
        )
}

impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| SliderState::new(cx.focus_handle()));
        let theme = crate::theme(cx);
        let tint = self.tint.unwrap_or(theme.primary._100);
        let track = theme.bg._400;

        let focus = state.read(cx).focus.clone();
        let resolved = self.scale.clamp(self.value.get(cx));
        let fraction = self.scale.fraction(resolved);
        let handlers = SliderHandlers {
            state: state.clone(),
            value: self.value,
            scale: Rc::new(self.scale),
            on_editing_changed: self.on_editing_changed,
        };

        let body = container_query(move |size, _, _| slider_body(size, fraction, tint, track));

        let measured = state.clone();
        let down = handlers.clone();
        let moved = handlers.clone();
        let up = handlers.clone();
        let up_out = handlers.clone();
        let keys = handlers.clone();
        div()
            .id(self.id)
            .relative()
            .w_full()
            .h(px(SLIDER_HEIGHT))
            .role(gpui::Role::Slider)
            .aria_numeric_value(resolved)
            .aria_min_numeric_value(*handlers.scale.range.start())
            .aria_max_numeric_value(*handlers.scale.range.end())
            .child(body)
            .child(measure("slider-bounds", move |bounds, _, cx| {
                measured.update(cx, |state, _| state.bounds = Some(bounds));
            }))
            .track_focus(&focus)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                down.begin(event.position.x, window, cx);
            })
            .on_drag(SliderDrag, |_, _, _, cx| cx.new(|_| SliderDragGhost))
            .on_drag_move::<SliderDrag>(move |event, window, cx| {
                if moved.state.read(cx).is_editing {
                    moved.apply(event.event.position.x, event.bounds, window, cx);
                }
            })
            .on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, window, cx| {
                up.end(Some(event.position.x), window, cx);
            })
            .on_mouse_up_out(MouseButton::Left, move |event: &MouseUpEvent, window, cx| {
                up_out.end(Some(event.position.x), window, cx);
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let direction = match event.keystroke.key.as_str() {
                    "left" | "down" => -1.0,
                    "right" | "up" => 1.0,
                    _ => return,
                };
                cx.stop_propagation();
                keys.key(direction, window, cx);
            })
    }
}
