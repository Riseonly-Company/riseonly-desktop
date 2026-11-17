use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, ElementId, Entity, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, hsla, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::selection_changed;
use smallvec::SmallVec;

use crate::liquid_chrome::LiquidChromeModifiers;
use crate::page_header_toolbar_button::PageHeaderMetrics;
use crate::page_header_ui::PageCoveredByArrivingPageKey;
use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::button_style::{ButtonAction, focus_visible_ring};
use crate::swiftui::environment::{EnvKey, env};
use crate::swiftui::environment_values::is_enabled;
use crate::swiftui::measure::measure;
use crate::swiftui::modifiers::{ModifierExt, ViewModifier};
use crate::swiftui::motion::{DEFAULT_INTERACTIVE_BLEND_DURATION, interactive_spring, spring};
use crate::swiftui::shapes::{Shape, centred_stroke};
use crate::swiftui::tooltip_label::TooltipLabelModifiers;
use crate::swiftui::transition::TransitionModifiers;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiquidGlassButtonShape {
    Circle,
    Capsule,
}

pub enum LiquidGlassPressMetrics {}

impl LiquidGlassPressMetrics {
    pub const PRESS_SCALE: f32 = 1.06;
    pub const WHITE_TINT: f64 = 0.18;
    pub const FOLLOW_FACTOR: f32 = 0.16;
    pub const MAX_OFFSET: f32 = 6.;
    pub const PRESS_IN_RESPONSE: f64 = 0.18;
    pub const PRESS_IN_DAMPING: f64 = 0.72;
    pub const RELEASE_RESPONSE: f64 = 0.34;
    pub const RELEASE_DAMPING: f64 = 0.6;
    pub const RELEASE_SLOP: f32 = 18.;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LiquidPressShape {
    Circle,
    Capsule,
    RoundedRect(f32),
}

impl LiquidPressShape {
    pub fn any_shape(self) -> Shape {
        match self {
            Self::Circle => Shape::Circle,
            Self::Capsule => Shape::Capsule,
            Self::RoundedRect(radius) => Shape::continuous(radius),
        }
    }

    /// same outline for hit testing so the corners of a capsule frame still belong to what is behind
    /// location is relative to the frame origin
    pub(crate) fn contains(self, location: Point<Pixels>, size: gpui::Size<Pixels>) -> bool {
        let (x, y) = (f32::from(location.x), f32::from(location.y));
        let (width, height) = (f32::from(size.width), f32::from(size.height));
        if x < 0. || y < 0. || x > width || y > height {
            return false;
        }
        let radius = match self {
            Self::Circle => {
                let (dx, dy) = (x - width / 2., y - height / 2.);
                let (rx, ry) = (width / 2., height / 2.);
                if rx <= 0. || ry <= 0. {
                    return false;
                }
                return (dx * dx) / (rx * rx) + (dy * dy) / (ry * ry) <= 1.;
            }
            Self::Capsule => width.min(height) / 2.,
            Self::RoundedRect(radius) => radius.min(width.min(height) / 2.).max(0.),
        };
        let cx = x.clamp(radius, width - radius);
        let cy = y.clamp(radius, height - radius);
        let (dx, dy) = (x - cx, y - cy);
        dx * dx + dy * dy <= radius * radius
    }
}

/// one pointer as the mouse handlers report it to the press state
enum LiquidPressPhase {
    Began { location: Point<Pixels>, size: gpui::Size<Pixels> },
    Moved { location: Point<Pixels>, size: gpui::Size<Pixels> },
    Ended { inside: bool },
}

/// state of a press modifier, lives in element state while the content is drawn
pub struct LiquidPressState {
    is_pressed: bool,
    pressed: Animated<f32>,
    press_offset: Animated<Point<Pixels>>,
    offset_target: Point<Pixels>,
    bounds: Bounds<Pixels>,
    hitbox_press: bool,
    down_position: Option<Point<Pixels>>,
}

impl Default for LiquidPressState {
    fn default() -> Self {
        Self::new()
    }
}

impl LiquidPressState {
    pub fn new() -> Self {
        Self {
            is_pressed: false,
            pressed: Animated::new(0.),
            press_offset: Animated::new(Point::default()),
            offset_target: Point::default(),
            bounds: Bounds::default(),
            hitbox_press: false,
            down_position: None,
        }
    }

    pub fn is_pressed(&self) -> bool {
        self.is_pressed
    }

    /// 0 at rest and 1 pressed, the spring both scale and white tint ride on
    pub fn pressed_progress(&self, now: MotionTime) -> f32 {
        self.pressed.value(now)
    }

    pub fn press_offset(&self, now: MotionTime) -> Point<Pixels> {
        self.press_offset.value(now)
    }

    pub fn is_animating(&self, now: MotionTime) -> bool {
        self.pressed.is_running(now) || self.press_offset.is_running(now)
    }

    /// ends the running springs at their targets
    fn settle(&mut self) {
        self.pressed.jump(if self.is_pressed { 1. } else { 0. });
        self.press_offset.jump(self.offset_target);
    }

    /// returns whether the action runs
    fn handle(&mut self, phase: LiquidPressPhase, interactive: bool, haptic_on_press: bool, now: MotionTime, reduce_motion: bool) -> bool {
        match phase {
            LiquidPressPhase::Began { location, size } => {
                if !interactive {
                    return false;
                }
                if haptic_on_press {
                    let _ = selection_changed();
                }
                self.is_pressed = true;
                self.pressed.set(
                    1.,
                    spring(LiquidGlassPressMetrics::PRESS_IN_RESPONSE, LiquidGlassPressMetrics::PRESS_IN_DAMPING),
                    now,
                    reduce_motion,
                );
                self.follow(location, size, now, reduce_motion);
                false
            }
            LiquidPressPhase::Moved { location, size } => {
                if self.is_pressed {
                    self.follow(location, size, now, reduce_motion);
                }
                false
            }
            LiquidPressPhase::Ended { inside } => {
                self.release(now, reduce_motion);
                inside && interactive
            }
        }
    }

    fn follow(&mut self, location: Point<Pixels>, size: gpui::Size<Pixels>, now: MotionTime, reduce_motion: bool) {
        let (width, height) = (f32::from(size.width), f32::from(size.height));
        if width <= 0. || height <= 0. {
            return;
        }
        let m = LiquidGlassPressMetrics::MAX_OFFSET;
        let dx = (f32::from(location.x) - width / 2.) * LiquidGlassPressMetrics::FOLLOW_FACTOR;
        let dy = (f32::from(location.y) - height / 2.) * LiquidGlassPressMetrics::FOLLOW_FACTOR;
        let target = point(px(dx.clamp(-m, m)), px(dy.clamp(-m, m)));
        self.offset_target = target;
        self.press_offset.set(target, interactive_spring(0.2, 0.75, DEFAULT_INTERACTIVE_BLEND_DURATION), now, reduce_motion);
    }

    fn release(&mut self, now: MotionTime, reduce_motion: bool) {
        if !self.is_pressed && self.offset_target == Point::default() {
            return;
        }
        let release = spring(LiquidGlassPressMetrics::RELEASE_RESPONSE, LiquidGlassPressMetrics::RELEASE_DAMPING);
        self.is_pressed = false;
        self.offset_target = Point::default();
        self.pressed.set(0., release, now, reduce_motion);
        self.press_offset.set(Point::default(), release, now, reduce_motion);
    }

    fn ends_near_down(&self, position: Point<Pixels>) -> bool {
        let slop = px(LiquidGlassPressMetrics::RELEASE_SLOP);
        self.down_position.is_some_and(|down| (position.x - down.x).abs() <= slop && (position.y - down.y).abs() <= slop)
    }

    fn ends_inside(&self, position: Point<Pixels>) -> bool {
        let slop = px(LiquidGlassPressMetrics::RELEASE_SLOP);
        let origin = self.bounds.origin;
        let size = self.bounds.size;
        position.x >= origin.x - slop
            && position.y >= origin.y - slop
            && position.x <= origin.x + size.width + slop
            && position.y <= origin.y + size.height + slop
    }
}

fn white(opacity: f64) -> Hsla {
    hsla(0., 0., 1., opacity as f32)
}

/// for a press whose action removes its surface (eg a call window closing into its widget)
/// the spring would redraw the screen every frame until it is gone so the press jumps to the target
pub struct LiquidPressSettlesAtOnceKey;

impl EnvKey for LiquidPressSettlesAtOnceKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }

    fn fingerprint(_value: &Self::Value) -> Option<u64> {
        None
    }
}

/// the press owns the pointer, a button under it never fires so the tap goes in action
pub struct LiquidPressModifier {
    pub id: ElementId,
    pub shape: LiquidPressShape,
    pub haptic_on_press: bool,
    pub disabled: bool,
    pub simultaneous: bool,
    pub action: Option<ButtonAction>,
    pub accessibility_label: Option<SharedString>,
}

impl ViewModifier for LiquidPressModifier {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        let state: Entity<LiquidPressState> = window.use_keyed_state(self.id.clone(), cx, |_, _| LiquidPressState::new());
        let interactive = !self.disabled && is_enabled(cx);
        let now = RiseClock::monotonic();
        if (env::<PageCoveredByArrivingPageKey>(cx) || env::<LiquidPressSettlesAtOnceKey>(cx))
            && state.read(cx).is_animating(now)
        {
            state.update(cx, |press, _| press.settle());
        }
        let (progress, offset) = {
            let press = state.read(cx);
            if press.is_animating(now) {
                window.request_animation_frame();
            }
            (press.pressed_progress(now), press.press_offset(now))
        };
        let shape = self.shape;
        let scale = 1. + (LiquidGlassPressMetrics::PRESS_SCALE - 1.) * progress;
        let tint = shape
            .any_shape()
            .round(div().absolute().inset_0())
            .bg(white(LiquidGlassPressMetrics::WHITE_TINT * f64::from(progress.clamp(0., 1.))));

        let bounds_state = state.clone();
        let bounds_probe = measure(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("liquid-press-bounds")),
            move |bounds, _window, cx| bounds_state.update(cx, |press, _| press.bounds = bounds),
        )
        .absolute()
        .inset_0();

        let pressed_content = div().relative().child(content).child(tint).child(bounds_probe).scale_effect(scale);
        let mut press = div()
            .id(self.id)
            .relative()
            .left(offset.x)
            .top(offset.y)
            .opacity(if self.disabled { 0.5 } else { 1. })
            .child(pressed_content);
        if let Some(label) = self.accessibility_label {
            press = press.aria_label(label.clone()).tooltip_label(label);
        }

        if self.disabled {
            return press
                .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| cx.stop_propagation())
                .on_click(|_: &ClickEvent, _, cx| cx.stop_propagation())
                .into_any_element();
        }

        let haptic_on_press = self.haptic_on_press;
        let simultaneous = self.simultaneous;
        let action = self.action;
        let down_state = state.clone();
        let move_state = state.clone();
        let up_state = state.clone();
        let up_out_state = state.clone();
        let up_action = action.clone();
        let up_out_action = action.clone();

        if interactive {
            press = focus_visible_ring(press.cursor_pointer().tab_index(0));
        }
        press
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                let bounds = down_state.read(cx).bounds;
                let location = event.position - bounds.origin;
                let in_frame = location.x >= px(0.)
                    && location.y >= px(0.)
                    && location.x <= bounds.size.width
                    && location.y <= bounds.size.height;
                if in_frame && !shape.contains(location, bounds.size) {
                    return;
                }
                if !simultaneous {
                    cx.stop_propagation();
                }
                window.prevent_default();
                let reduce_motion = cx.reduce_motion();
                down_state.update(cx, |press, cx| {
                    press.hitbox_press = !in_frame;
                    press.down_position = Some(event.position);
                    press.handle(
                        LiquidPressPhase::Began { location, size: bounds.size },
                        interactive,
                        haptic_on_press,
                        RiseClock::monotonic(),
                        reduce_motion,
                    );
                    cx.notify();
                });
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) || !move_state.read(cx).is_pressed() {
                    return;
                }
                let reduce_motion = cx.reduce_motion();
                move_state.update(cx, |press, cx| {
                    let bounds = press.bounds;
                    press.handle(
                        LiquidPressPhase::Moved { location: event.position - bounds.origin, size: bounds.size },
                        interactive,
                        haptic_on_press,
                        RiseClock::monotonic(),
                        reduce_motion,
                    );
                    cx.notify();
                });
            })
            .on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, window, cx| {
                end_press(&up_state, event.position, true, interactive, haptic_on_press, up_action.as_ref(), window, cx);
            })
            .on_mouse_up_out(MouseButton::Left, move |event: &MouseUpEvent, window, cx| {
                end_press(&up_out_state, event.position, false, interactive, haptic_on_press, up_out_action.as_ref(), window, cx);
            })
            .on_click(move |event: &ClickEvent, window, cx| {
                if event.is_keyboard()
                    && interactive
                    && let Some(action) = &action
                {
                    action(window, cx);
                }
            })
            .into_any_element()
    }
}

fn end_press(
    state: &Entity<LiquidPressState>,
    position: Point<Pixels>,
    over_hitbox: bool,
    interactive: bool,
    haptic_on_press: bool,
    action: Option<&ButtonAction>,
    window: &mut Window,
    cx: &mut App,
) {
    if !state.read(cx).is_pressed() {
        return;
    }
    let reduce_motion = cx.reduce_motion();
    let fires = state.update(cx, |press, cx| {
        let inside = press.ends_inside(position) || (press.hitbox_press && over_hitbox) || press.ends_near_down(position);
        press.hitbox_press = false;
        press.down_position = None;
        let fires = press.handle(LiquidPressPhase::Ended { inside }, interactive, haptic_on_press, RiseClock::monotonic(), reduce_motion);
        cx.notify();
        fires
    });
    if fires && let Some(action) = action {
        action(window, cx);
    }
}

/// press modifier defaults haptic_on_press true, disabled false simultaneous false no action
/// the id keys the press state
pub trait LiquidGlassButtonModifiers: IntoElement + Sized {
    fn liquid_press(
        self,
        id: impl Into<ElementId>,
        shape: LiquidPressShape,
        haptic_on_press: bool,
        disabled: bool,
        simultaneous: bool,
        action: Option<ButtonAction>,
    ) -> AnyElement {
        self.modifier(LiquidPressModifier {
            id: id.into(),
            shape,
            haptic_on_press,
            disabled,
            simultaneous,
            action,
            accessibility_label: None,
        })
            .into_any_element()
    }
}

impl<E: IntoElement> LiquidGlassButtonModifiers for E {}

#[derive(IntoElement)]
pub struct LiquidGlassButton {
    id: ElementId,
    pub shape: LiquidGlassButtonShape,
    pub size: f32,
    pub horizontal_padding: f32,
    pub vertical_padding: f32,
    pub blur_intensity: Option<f32>,
    pub tint: Option<Hsla>,
    pub haptic_on_press: bool,
    pub disabled: bool,
    pub flattened: bool,
    pub accessibility_label: Option<String>,
    action: Option<ButtonAction>,
    label: SmallVec<[AnyElement; 1]>,
}

impl LiquidGlassButton {
    pub fn new(id: impl Into<ElementId>, shape: LiquidGlassButtonShape) -> Self {
        Self {
            id: id.into(),
            shape,
            size: PageHeaderMetrics::TOOLBAR_BUTTON_SIZE,
            horizontal_padding: 12.,
            vertical_padding: 6.,
            blur_intensity: None,
            tint: None,
            haptic_on_press: true,
            disabled: false,
            flattened: false,
            accessibility_label: None,
            action: None,
            label: SmallVec::new(),
        }
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn horizontal_padding(mut self, padding: f32) -> Self {
        self.horizontal_padding = padding;
        self
    }

    pub fn vertical_padding(mut self, padding: f32) -> Self {
        self.vertical_padding = padding;
        self
    }

    pub fn blur_intensity(mut self, intensity: Option<f32>) -> Self {
        self.blur_intensity = intensity;
        self
    }

    pub fn tint(mut self, tint: Option<Hsla>) -> Self {
        self.tint = tint;
        self
    }

    pub fn haptic_on_press(mut self, haptic: bool) -> Self {
        self.haptic_on_press = haptic;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn flattened(mut self, flattened: bool) -> Self {
        self.flattened = flattened;
        self
    }

    pub fn accessibility_label(mut self, label: Option<String>) -> Self {
        self.accessibility_label = label;
        self
    }

    pub fn on_press(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.action = Some(std::rc::Rc::new(action));
        self
    }

    fn press_shape(&self) -> LiquidPressShape {
        match self.shape {
            LiquidGlassButtonShape::Circle => LiquidPressShape::Circle,
            LiquidGlassButtonShape::Capsule => LiquidPressShape::Capsule,
        }
    }
}

impl ParentElement for LiquidGlassButton {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.label.extend(elements);
    }
}

impl RenderOnce for LiquidGlassButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let press_shape = self.press_shape();
        let sized_label = match self.shape {
            LiquidGlassButtonShape::Circle => div()
                .size(px(self.size))
                .flex()
                .items_center()
                .justify_center()
                .children(self.label),
            LiquidGlassButtonShape::Capsule => div()
                .flex()
                .items_center()
                .px(px(self.horizontal_padding))
                .py(px(self.vertical_padding))
                .children(self.label),
        };
        let background = LiquidGlassBackgroundModifier {
            shape: self.shape,
            blur_intensity: self.blur_intensity,
            tint: self.tint,
            flattened: self.flattened,
        };
        let accessibility_label =
            self.accessibility_label.filter(|label| !label.is_empty()).map(SharedString::from);
        sized_label.modifier(background).modifier(LiquidPressModifier {
            id: self.id,
            shape: press_shape,
            haptic_on_press: self.haptic_on_press,
            disabled: self.disabled,
            simultaneous: false,
            action: self.action,
            accessibility_label,
        })
    }
}

/// static glass chrome for controls whose interaction something else owns (a menu label)
/// same look as LiquidGlassButton including flattened
#[derive(IntoElement)]
pub struct LiquidGlassMenuLabel {
    pub shape: LiquidGlassButtonShape,
    pub size: f32,
    pub flattened: bool,
    content: SmallVec<[AnyElement; 1]>,
}

impl Default for LiquidGlassMenuLabel {
    fn default() -> Self {
        Self::new()
    }
}

impl LiquidGlassMenuLabel {
    pub fn new() -> Self {
        Self {
            shape: LiquidGlassButtonShape::Circle,
            size: PageHeaderMetrics::TOOLBAR_BUTTON_SIZE,
            flattened: false,
            content: SmallVec::new(),
        }
    }

    pub fn shape(mut self, shape: LiquidGlassButtonShape) -> Self {
        self.shape = shape;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn flattened(mut self, flattened: bool) -> Self {
        self.flattened = flattened;
        self
    }
}

impl ParentElement for LiquidGlassMenuLabel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.content.extend(elements);
    }
}

impl RenderOnce for LiquidGlassMenuLabel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let framed = div().size(px(self.size)).flex().items_center().justify_center().children(self.content);
        framed.modifier(LiquidGlassBackgroundModifier {
            shape: self.shape,
            blur_intensity: None,
            tint: None,
            flattened: self.flattened,
        })
    }
}

/// the glass behind a button
/// flattened means a plain fill instead of the material, so swapping it under the finger costs nothing
struct LiquidGlassBackgroundModifier {
    shape: LiquidGlassButtonShape,
    blur_intensity: Option<f32>,
    tint: Option<Hsla>,
    flattened: bool,
}

impl ViewModifier for LiquidGlassBackgroundModifier {
    fn body(self, content: AnyElement, _window: &mut Window, _cx: &mut App) -> AnyElement {
        let shape = match self.shape {
            LiquidGlassButtonShape::Circle => Shape::Circle,
            LiquidGlassButtonShape::Capsule => Shape::Capsule,
        };
        if self.flattened {
            return Self::flat_background(content, shape, self.tint);
        }
        let tint_fill = shape.round(div().absolute().inset_0()).bg(self.tint.unwrap_or_else(|| white(0.)));
        let tinted = div().relative().child(tint_fill).child(content);
        match self.shape {
            LiquidGlassButtonShape::Circle => tinted.liquid_chrome_circle(self.blur_intensity),
            LiquidGlassButtonShape::Capsule => tinted.liquid_chrome_capsule_with(0.12, true, self.blur_intensity, false),
        }
    }
}

impl LiquidGlassBackgroundModifier {
    fn flat_background(content: AnyElement, shape: Shape, tint: Option<Hsla>) -> AnyElement {
        let fill = shape
            .round(div().absolute().inset_0())
            .bg(tint.unwrap_or_else(|| hsla(0., 0., 0., 0.42)));
        div()
            .relative()
            .child(fill)
            .child(content)
            .child(centred_stroke(white(0.14), 1., shape))
            .into_any_element()
    }
}
