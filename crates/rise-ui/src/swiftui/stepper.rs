use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, ElementId, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, RenderOnce,
    SharedString, Styled, Window, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::IconUi;
use crate::swiftui::binding::Binding;
use crate::swiftui::button_style::plain;
use crate::swiftui::stacks::hstack;

const SEGMENT_WIDTH: f32 = 47.0;
const CONTROL_HEIGHT: f32 = 32.0;
const CORNER_RADIUS: f32 = 8.0;
const SYMBOL_SIDE: f32 = 13.0;
const DIVIDER_HEIGHT: f32 = 18.0;
const UP_KEY: &str = "up";
const DOWN_KEY: &str = "down";
const TYPED_VALUE_WINDOW: Duration = Duration::from_millis(1500);

/// value one press leaves, none when the range ends there and the segment is disabled
pub fn stepped_value(current: i64, delta: i64, range: &RangeInclusive<i64>) -> Option<i64> {
    let next = current.saturating_add(delta).clamp(*range.start(), *range.end());
    (next != current).then_some(next)
}

/// value the typed digits stand for, none on overflow
pub fn typed_value(typed: &str, range: &RangeInclusive<i64>) -> Option<i64> {
    typed.parse::<i64>().ok().map(|value| value.clamp(*range.start(), *range.end()))
}

struct StepperKeyboard {
    focus_handle: FocusHandle,
    typed: String,
    typed_at: Duration,
}

impl StepperKeyboard {
    fn type_digit(&mut self, digit: char, now: Duration) -> &str {
        if now.saturating_sub(self.typed_at) > TYPED_VALUE_WINDOW {
            self.typed.clear();
        }
        self.typed.push(digit);
        self.typed_at = now;
        &self.typed
    }
}

#[derive(IntoElement)]
pub struct Stepper {
    id: ElementId,
    value: Binding<i64>,
    range: RangeInclusive<i64>,
    step: i64,
    disabled: bool,
}

impl Stepper {
    pub fn new(id: impl Into<ElementId>, value: Binding<i64>, range: RangeInclusive<i64>) -> Self {
        Self {
            id: id.into(),
            value,
            range,
            step: 1,
            disabled: false,
        }
    }

    /// step size, default is 1
    pub fn step(mut self, step: i64) -> Self {
        self.step = step;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn segment(&self, symbol: &'static str, delta: i64, cx: &App) -> impl IntoElement {
        let theme = crate::theme(cx);
        let current = self.value.get(cx);
        let enabled = !self.disabled && stepped_value(current, delta, &self.range).is_some();
        let tint = if enabled { theme.text.primary } else { theme.text.secondary };
        let value = self.value.clone();
        let range = self.range.clone();
        plain(SharedString::from(format!("{}-{symbol}", self.id)))
            .disabled(!enabled)
            .on_press(move |window, cx| {
                if let Some(next) = stepped_value(value.get(cx), delta, &range) {
                    value.set(next, window, cx);
                }
            })
            .w(px(SEGMENT_WIDTH))
            .h(px(CONTROL_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized(symbol, px(SYMBOL_SIDE), tint, false))
    }
}

impl RenderOnce for Stepper {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (surface, focused_surface, divider) = {
            let theme = crate::theme(cx);
            (theme.bg._100, theme.bg._300, theme.border._100)
        };
        let keyboard_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("keyboard"));
        let keyboard = window.use_keyed_state(keyboard_id, cx, |_, cx| StepperKeyboard {
            focus_handle: cx.focus_handle(),
            typed: String::new(),
            typed_at: Duration::ZERO,
        });
        let focus_handle = keyboard.read(cx).focus_handle.clone().tab_stop(!self.disabled);
        let is_focused = focus_handle.is_focused(window);
        let disabled = self.disabled;
        let step = self.step;
        let value = self.value.clone();
        let range = self.range.clone();
        hstack(0.0)
            .id(self.id.clone())
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                if disabled || keystroke.modifiers.modified() {
                    return;
                }
                let key = keystroke.key.as_str();
                let digit = key.chars().next().filter(|first| first.is_ascii_digit() && key.len() == 1);
                let next = if key == UP_KEY {
                    stepped_value(value.get(cx), step, &range)
                } else if key == DOWN_KEY {
                    stepped_value(value.get(cx), -step, &range)
                } else if let Some(digit) = digit {
                    let now = RiseClock::monotonic();
                    keyboard
                        .update(cx, |keyboard, _| typed_value(keyboard.type_digit(digit, now), &range))
                        .filter(|typed| *typed != value.get(cx))
                } else {
                    return;
                };
                cx.stop_propagation();
                if let Some(next) = next {
                    value.set(next, window, cx);
                }
            })
            .rounded(px(CORNER_RADIUS))
            .bg(surface)
            .when(is_focused, |stepper| stepper.bg(focused_surface))
            .child(self.segment("minus", -step, cx))
            .child(div().w(px(1.0)).h(px(DIVIDER_HEIGHT)).bg(divider))
            .child(self.segment("plus", step, cx))
    }
}
