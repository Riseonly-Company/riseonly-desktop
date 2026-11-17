use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, ElementId, Entity, Hsla, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseUpEvent, ParentElement, RenderOnce, Styled, Window, div, transparent_black,
    white,
};
use rise_core::foundation::foundation_date::RiseClock;
use smallvec::SmallVec;

use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::button_style::{ButtonAction, DISABLED_OPACITY, button_chrome};
use crate::swiftui::frame::SwiftUiAlignedFrame;
use crate::swiftui::motion::{MotionSpec, ease_out};
use crate::swiftui::shapes::{Shape, SwiftUiShapes};
use crate::swiftui::stacks::Alignment;

pub const PRESSED_WHITEN_OPACITY: f32 = 0.09;

pub fn press_highlight_motion() -> MotionSpec {
    ease_out(0.12)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SimpleButtonVariant {
    #[default]
    Default,
    GroupedBtn,
}

impl SimpleButtonVariant {
    /// grouped variant always sits on the 200 background, the default one on its own color or nothing
    pub fn resolved_bg(self, bg_color: Option<Hsla>, theme: &rise_theme::AppTheme) -> Hsla {
        match self {
            Self::Default => bg_color.unwrap_or_else(transparent_black),
            Self::GroupedBtn => theme.bg._200,
        }
    }

    /// press style, whitens on press only for the grouped variant
    pub fn press_style(self) -> SimpleButtonPressStyle {
        SimpleButtonPressStyle { whiten_on_press: self == Self::GroupedBtn }
    }
}

/// button style, the label as it is and lightened while pressed when whiten_on_press
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimpleButtonPressStyle {
    pub whiten_on_press: bool,
}

impl SimpleButtonPressStyle {
    pub fn make_body(self, label: Div, whiten: f32) -> Div {
        if self.whiten_on_press && whiten > 0.0 {
            label.child(div().absolute().inset_0().capsule().bg(white().opacity(whiten)))
        } else {
            label
        }
    }
}

/// pressed state with its animation, lives in element state while drawn
pub struct SimpleButtonPressState {
    whiten: Animated<f32>,
}

impl Default for SimpleButtonPressState {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleButtonPressState {
    pub fn new() -> Self {
        Self { whiten: Animated::new(0.0) }
    }

    pub fn set_pressed(&mut self, is_pressed: bool, now: MotionTime, reduce_motion: bool) {
        let target = if is_pressed { PRESSED_WHITEN_OPACITY } else { 0.0 };
        self.whiten.set(target, press_highlight_motion(), now, reduce_motion);
    }

    pub fn whiten_opacity(&self, now: MotionTime) -> f32 {
        self.whiten.value(now)
    }

    pub fn is_running(&self, now: MotionTime) -> bool {
        self.whiten.is_running(now)
    }
}

#[derive(IntoElement)]
pub struct SimpleButtonUi {
    id: ElementId,
    variant: SimpleButtonVariant,
    disabled: bool,
    bg_color: Option<Hsla>,
    width: Option<f32>,
    height: Option<f32>,
    on_press: Option<ButtonAction>,
    children: SmallVec<[AnyElement; 2]>,
}

impl SimpleButtonUi {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            variant: SimpleButtonVariant::Default,
            disabled: false,
            bg_color: None,
            width: None,
            height: None,
            on_press: None,
            children: SmallVec::new(),
        }
    }

    pub fn variant(mut self, variant: SimpleButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn bg_color(mut self, color: Hsla) -> Self {
        self.bg_color = Some(color);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    pub fn on_press(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_press = Some(Rc::new(action));
        self
    }
}

impl ParentElement for SimpleButtonUi {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

fn set_pressed(state: &Entity<SimpleButtonPressState>, is_pressed: bool, cx: &mut App) {
    let reduce_motion = cx.reduce_motion();
    state.update(cx, |press, cx| {
        press.set_pressed(is_pressed, RiseClock::monotonic(), reduce_motion);
        cx.notify();
    });
}

impl RenderOnce for SimpleButtonUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let background = self.variant.resolved_bg(self.bg_color, crate::theme(cx));
        let press_style = self.variant.press_style();
        let fills_row = self.variant == SimpleButtonVariant::GroupedBtn;

        let frame = div().children(self.children).frame_size_aligned(self.width, self.height, Alignment::Center);
        let frame = if fills_row { frame.w_full() } else { frame };
        let label = div().relative().child(frame);
        let label = if fills_row { label.w_full() } else { label };
        let label = label
            .bg(background)
            .clip_shape(Shape::Capsule)
            .opacity(if self.disabled { DISABLED_OPACITY } else { 1.0 });

        let press = press_style.whiten_on_press.then(|| {
            window.use_keyed_state(self.id.clone(), cx, |_, _| SimpleButtonPressState::new())
        });
        let whiten = press.as_ref().map_or(0.0, |entity| {
            let now = RiseClock::monotonic();
            let state = entity.read(cx);
            if state.is_running(now) {
                window.request_animation_frame();
            }
            state.whiten_opacity(now)
        });
        let label = press_style.make_body(label, whiten);

        let mut button = div().id(self.id);
        if fills_row {
            button = button.w_full();
        }
        if let Some(up) = press.filter(|_| !self.disabled) {
            let down = up.clone();
            let up_out = up.clone();
            button = button
                .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, cx| set_pressed(&down, true, cx))
                .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, _, cx| set_pressed(&up, false, cx))
                .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, _, cx| set_pressed(&up_out, false, cx));
        }
        button_chrome(button, self.disabled, self.on_press).child(label)
    }
}
