use std::ops::RangeInclusive;
use std::rc::Rc;

use gpui::{
    App, BoxShadow, Bounds, DispatchPhase, ElementId, Entity, FocusHandle, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, RenderOnce,
    Size, StatefulInteractiveElement, Styled, Window, black, canvas, container_query, div, point, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::selection_changed;

use crate::swiftui::animated::Animated;
use crate::swiftui::measure::measure;
use crate::swiftui::motion::{MotionSpec, spring};
use crate::swiftui::shapes::SwiftUiShapes;
use crate::swiftui::transition::TransitionModifiers;

pub type SteppedSliderAction = Rc<dyn Fn(f64, &mut Window, &mut App)>;

const TRACK_HEIGHT: f32 = 5.0;
pub const KNOB_SIZE: f32 = 20.0;
const TICK_SIZE: f32 = 3.0;
const EPSILON: f64 = 0.0001;
const SLIDER_HEIGHT: f32 = KNOB_SIZE + 12.0;
const MAXIMUM_TICK_COUNT: usize = 64;

pub fn knob_drag_motion() -> MotionSpec {
    spring(0.22, 0.85)
}

/// value rules of the slider apart from drawing, what a pointer position or key resolves to
#[derive(Clone, Debug, PartialEq)]
pub struct SteppedSliderScale {
    pub range: RangeInclusive<f64>,
    pub step: f64,
    pub breakpoints: Vec<f64>,
}

impl SteppedSliderScale {
    pub fn span(&self) -> f64 {
        EPSILON.max(self.range.end() - self.range.start())
    }

    pub fn snap_tolerance(&self) -> f64 {
        (self.step * 0.6).max(self.span() * 0.012)
    }

    pub fn tick_count(&self) -> usize {
        if self.step <= EPSILON {
            return 0;
        }
        let count = (self.span() / self.step).round() as usize + 1;
        if count <= MAXIMUM_TICK_COUNT { count } else { 0 }
    }

    pub fn clamp(&self, raw: f64) -> f64 {
        self.range.end().min(self.range.start().max(raw))
    }

    pub fn clamp_fraction(raw: f64) -> f64 {
        1f64.min(0f64.max(raw))
    }

    pub fn quantize(&self, raw: f64) -> f64 {
        if self.step <= EPSILON {
            return self.clamp(raw);
        }
        let steps = ((raw - self.range.start()) / self.step).round();
        self.clamp(self.range.start() + steps * self.step)
    }

    pub fn snap(&self, candidate: f64) -> f64 {
        let nearest = self
            .breakpoints
            .iter()
            .copied()
            .min_by(|a, b| (a - candidate).abs().total_cmp(&(b - candidate).abs()));
        match nearest {
            Some(nearest) if (nearest - candidate).abs() <= self.snap_tolerance() => self.clamp(nearest),
            _ => candidate,
        }
    }

    pub fn fraction(&self, value: f64) -> f64 {
        (value - self.range.start()) / self.span()
    }

    /// stop under location_x or none when it is the stop the knob already shows, no side effects
    pub fn next_value(&self, location_x: f32, usable: f32, previous: f64) -> Option<f64> {
        if usable <= 0.0 {
            return None;
        }
        let raw_fraction = f64::from((location_x - KNOB_SIZE / 2.0) / usable);
        let raw = self.range.start() + Self::clamp_fraction(raw_fraction) * self.span();
        let next = self.snap(self.quantize(raw));
        ((next - previous).abs() > EPSILON).then_some(next)
    }

    /// one arrow key press, exactly one step toward direction (-1 or 1), breakpoints pull only a drag
    pub fn stepped(&self, previous: f64, direction: f64) -> Option<f64> {
        let step = if self.step > EPSILON { self.step } else { self.span() / 100.0 };
        let next = self.quantize(previous + direction * step);
        ((next - previous).abs() > EPSILON).then_some(next)
    }
}

/// drag value and drag flag of the slider, knob press scale
/// plus laid out bounds and focus for arrow keys
pub struct SteppedSliderUiState {
    drag_value: Option<f64>,
    is_dragging: bool,
    knob_scale: Animated<f32>,
    bounds: Option<Bounds<Pixels>>,
    focus: FocusHandle,
}

impl SteppedSliderUiState {
    fn new(focus: FocusHandle) -> Self {
        Self { drag_value: None, is_dragging: false, knob_scale: Animated::new(1.0), bounds: None, focus }
    }

    fn set_dragging(&mut self, is_dragging: bool, cx: &App) {
        if self.is_dragging == is_dragging {
            return;
        }
        self.is_dragging = is_dragging;
        let target = if is_dragging { 1.12 } else { 1.0 };
        self.knob_scale.set_in(target, knob_drag_motion(), RiseClock::monotonic(), cx);
    }
}

#[derive(IntoElement)]
pub struct SteppedSliderUi {
    id: ElementId,
    value: f64,
    scale: SteppedSliderScale,
    shows_step_ticks: bool,
    tint: Option<Hsla>,
    track_color: Option<Hsla>,
    is_enabled: bool,
    on_change: SteppedSliderAction,
    on_commit: SteppedSliderAction,
}

impl SteppedSliderUi {
    pub fn new(
        id: impl Into<ElementId>,
        value: f64,
        range: RangeInclusive<f64>,
        step: f64,
        on_change: impl Fn(f64, &mut Window, &mut App) + 'static,
        on_commit: impl Fn(f64, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            value,
            scale: SteppedSliderScale { range, step, breakpoints: Vec::new() },
            shows_step_ticks: true,
            tint: None,
            track_color: None,
            is_enabled: true,
            on_change: Rc::new(on_change),
            on_commit: Rc::new(on_commit),
        }
    }

    /// values the knob snaps to, ticks are still drawn for every step
    pub fn breakpoints(mut self, breakpoints: Vec<f64>) -> Self {
        self.scale.breakpoints = breakpoints;
        self
    }

    pub fn shows_step_ticks(mut self, shows_step_ticks: bool) -> Self {
        self.shows_step_ticks = shows_step_ticks;
        self
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }

    pub fn track_color(mut self, track_color: Hsla) -> Self {
        self.track_color = Some(track_color);
        self
    }

    pub fn is_enabled(mut self, is_enabled: bool) -> Self {
        self.is_enabled = is_enabled;
        self
    }
}

#[derive(Clone)]
struct SteppedSliderHandlers {
    state: Entity<SteppedSliderUiState>,
    value: f64,
    scale: Rc<SteppedSliderScale>,
    on_change: SteppedSliderAction,
    on_commit: SteppedSliderAction,
}

impl SteppedSliderHandlers {
    fn resolved_value(&self, cx: &App) -> f64 {
        self.scale.clamp(self.state.read(cx).drag_value.unwrap_or(self.value))
    }

    /// moves the slider to a pointer x in window coords, mapped through bounds
    fn apply(&self, position_x: Pixels, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let width = KNOB_SIZE.max(f32::from(bounds.size.width));
        let usable = width - KNOB_SIZE;
        let location_x = f32::from(position_x - bounds.origin.x);
        let previous = self.resolved_value(cx);
        let Some(next) = self.scale.next_value(location_x, usable, previous) else { return };
        self.state.update(cx, |state, cx| {
            state.drag_value = Some(next);
            cx.notify();
        });
        let _ = selection_changed();
        (self.on_change)(next, window, cx);
    }

    fn begin(&self, position_x: Pixels, window: &mut Window, cx: &mut App) {
        let bounds = self.state.read(cx).bounds;
        self.state.update(cx, |state, cx| {
            state.set_dragging(true, cx);
            cx.notify();
        });
        if let Some(bounds) = bounds {
            self.apply(position_x, bounds, window, cx);
        }
    }

    fn end(&self, position_x: Option<Pixels>, window: &mut Window, cx: &mut App) {
        let (is_dragging, bounds) = {
            let state = self.state.read(cx);
            (state.is_dragging, state.bounds)
        };
        if !is_dragging {
            return;
        }
        if let (Some(position_x), Some(bounds)) = (position_x, bounds) {
            self.apply(position_x, bounds, window, cx);
        }
        let committed = self.resolved_value(cx);
        self.state.update(cx, |state, cx| {
            state.set_dragging(false, cx);
            state.drag_value = None;
            cx.notify();
        });
        (self.on_commit)(committed, window, cx);
    }

    fn key(&self, direction: f64, window: &mut Window, cx: &mut App) {
        let previous = self.resolved_value(cx);
        let Some(next) = self.scale.stepped(previous, direction) else { return };
        let _ = selection_changed();
        (self.on_change)(next, window, cx);
        (self.on_commit)(next, window, cx);
    }
}

#[allow(clippy::too_many_arguments)]
fn slider_body(
    size: Size<Pixels>,
    scale: &SteppedSliderScale,
    resolved: f64,
    knob_scale: f32,
    shows_step_ticks: bool,
    accent: Hsla,
    rail: Hsla,
    passed_tick: Hsla,
    ahead_tick: Hsla,
) -> gpui::Div {
    let width = KNOB_SIZE.max(f32::from(size.width));
    let height = f32::from(size.height);
    let usable = width - KNOB_SIZE;
    let fraction = scale.fraction(resolved);
    let knob_x = KNOB_SIZE / 2.0 + usable * fraction as f32;
    let centred = |side: f32| px((height - side) / 2.0);

    let mut body = div()
        .relative()
        .size_full()
        .child(div().absolute().left_0().top(centred(TRACK_HEIGHT)).w(px(width)).h(px(TRACK_HEIGHT)).capsule().bg(rail))
        .child(
            div()
                .absolute()
                .left_0()
                .top(centred(TRACK_HEIGHT))
                .w(px(TRACK_HEIGHT.max(knob_x)))
                .h(px(TRACK_HEIGHT))
                .capsule()
                .bg(accent),
        );

    let tick_count = scale.tick_count();
    if shows_step_ticks && tick_count > 1 {
        for index in 0..tick_count {
            let point_value = scale.clamp(scale.range.start() + index as f64 * scale.step);
            let point_fraction = scale.fraction(point_value);
            let fill = if point_fraction <= fraction { passed_tick } else { ahead_tick };
            body = body.child(
                div()
                    .absolute()
                    .left(px(KNOB_SIZE / 2.0 + usable * point_fraction as f32 - TICK_SIZE / 2.0))
                    .top(centred(TICK_SIZE))
                    .size(px(TICK_SIZE))
                    .circle()
                    .bg(fill),
            );
        }
    }

    body.child(
        div()
            .absolute()
            .left(px(knob_x - KNOB_SIZE / 2.0))
            .top(centred(KNOB_SIZE))
            .child(
                div()
                    .size(px(KNOB_SIZE))
                    .circle()
                    .bg(white())
                    .shadow(vec![BoxShadow {
                        color: black().opacity(0.22),
                        offset: point(px(0.0), px(1.0)),
                        blur_radius: px(3.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .scale_effect(knob_scale),
            ),
    )
}

impl RenderOnce for SteppedSliderUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| SteppedSliderUiState::new(cx.focus_handle()));
        let theme = crate::theme(cx);
        let accent = self.tint.unwrap_or(theme.primary._100);
        let rail = self.track_color.unwrap_or(theme.bg._400);
        let passed_tick = theme.bg._100.opacity(0.55);
        let ahead_tick = theme.text.secondary.opacity(0.45);

        let now = RiseClock::monotonic();
        let (drag_value, knob_scale, focus) = {
            let state = state.read(cx);
            if state.knob_scale.is_running(now) {
                window.request_animation_frame();
            }
            (state.drag_value, state.knob_scale.value(now), state.focus.clone())
        };
        let resolved = self.scale.clamp(drag_value.unwrap_or(self.value));
        let handlers = SteppedSliderHandlers {
            state: state.clone(),
            value: self.value,
            scale: Rc::new(self.scale.clone()),
            on_change: self.on_change,
            on_commit: self.on_commit,
        };

        let scale = self.scale;
        let shows_step_ticks = self.shows_step_ticks;
        let body = container_query(move |size, _, _| {
            slider_body(size, &scale, resolved, knob_scale, shows_step_ticks, accent, rail, passed_tick, ahead_tick)
        });

        let measured = state.clone();
        let mut slider = div()
            .id(self.id)
            .relative()
            .w_full()
            .h(px(SLIDER_HEIGHT))
            .opacity(if self.is_enabled { 1.0 } else { 0.4 })
            .role(gpui::Role::Slider)
            .aria_numeric_value(resolved)
            .aria_min_numeric_value(*handlers.scale.range.start())
            .aria_max_numeric_value(*handlers.scale.range.end())
            .child(body)
            .child(measure("stepped-slider-bounds", move |bounds, _, cx| {
                measured.update(cx, |state, _| state.bounds = Some(bounds));
            }));

        if self.is_enabled {
            let down = handlers.clone();
            let moved = handlers.clone();
            let up = handlers.clone();
            let up_out = handlers.clone();
            let keys = handlers;
            slider = slider
                .track_focus(&focus)
                .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                    down.begin(event.position.x, window, cx);
                })
                .child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, _| {
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                                if phase != DispatchPhase::Bubble {
                                    return;
                                }
                                let (is_dragging, bounds) = {
                                    let state = moved.state.read(cx);
                                    (state.is_dragging, state.bounds)
                                };
                                if !is_dragging {
                                    return;
                                }
                                if event.pressed_button != Some(MouseButton::Left) {
                                    moved.end(None, window, cx);
                                    return;
                                }
                                if let Some(bounds) = bounds {
                                    moved.apply(event.position.x, bounds, window, cx);
                                }
                            });
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
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
                });
        }
        slider
    }
}
