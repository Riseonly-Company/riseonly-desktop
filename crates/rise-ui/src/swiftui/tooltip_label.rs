use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Global, IntoElement, MouseDownEvent, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::bottom_sheet_ui::modal_sheet_surface::ModalSheetMetrics;
use crate::keyboard_dismiss::ScreenFocus;
use crate::top_presenter::TopPresenter;

struct TooltipLabel {
    label: SharedString,
    hovered_at: Duration,
    pressed_in_place: bool,
}

const POINTER_CLEARANCE: f32 = 20.0;

const TOOLTIP_SHOW_DELAY: Duration = Duration::from_millis(500);

const PRESS_POSITION_SLOP: f32 = 4.0;

/// monotonic time and position of the last press on a control with a hover label
struct TooltipLabelLastPress(Duration, Point<Pixels>);

impl Global for TooltipLabelLastPress {}

/// monotonic time when the latest page push or pop finishes moving
struct TooltipLabelPageMotionEnd(Duration);

impl Global for TooltipLabelPageMotionEnd {}

/// navigation stack tells hover labels that a page slides over or off the controls under the pointer
pub struct TooltipLabelPageMotion;

impl TooltipLabelPageMotion {
    /// a page starts its push or pop motion now and settles after settle
    pub fn note(settle: Duration, cx: &mut App) {
        cx.set_global(TooltipLabelPageMotionEnd(RiseClock::monotonic() + settle));
    }

    /// time left until the latest page push or pop settles, zero once it has
    pub fn remaining(cx: &App) -> Duration {
        cx.try_global::<TooltipLabelPageMotionEnd>()
            .map_or(Duration::ZERO, |end| end.0.saturating_sub(RiseClock::monotonic()))
    }
}

impl TooltipLabel {
    fn is_under_top_sheet(&self, window: &Window, cx: &Context<Self>) -> bool {
        let settle = Duration::from_secs_f64(ModalSheetMetrics::PRESENT_DURATION);
        let presented_at = ScreenFocus::top_sheet_presented_at(window, cx).max(TopPresenter::top_presented_at(window, cx));
        presented_at.is_some_and(|presented_at| self.hovered_at < presented_at + settle)
    }

    fn is_before_page_motion_end(&self, cx: &Context<Self>) -> bool {
        cx.try_global::<TooltipLabelPageMotionEnd>().is_some_and(|motion| self.hovered_at < motion.0)
    }

    fn is_pressed_since_hover(&self, cx: &Context<Self>) -> bool {
        self.pressed_in_place || cx.try_global::<TooltipLabelLastPress>().is_some_and(|press| press.0 >= self.hovered_at)
    }

    /// true if the pointer rests where the last press landed
    fn pressed_in_place(window: &Window, cx: &App) -> bool {
        let pointer = window.mouse_position();
        cx.try_global::<TooltipLabelLastPress>().is_some_and(|press| {
            (press.1.x - pointer.x).abs() <= px(PRESS_POSITION_SLOP) && (press.1.y - pointer.y).abs() <= px(PRESS_POSITION_SLOP)
        })
    }
}

impl Render for TooltipLabel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme(cx);
        let hidden =
            self.is_under_top_sheet(window, cx) || self.is_pressed_since_hover(cx) || self.is_before_page_motion_end(cx);
        div().when(!hidden, |band| {
            band.py(px(POINTER_CLEARANCE)).child(
                div()
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .bg(theme.bg._200)
                    .text_size(px(12.0))
                    .text_color(theme.text.primary)
                    .child(self.label.clone()),
            )
        })
    }
}

pub trait TooltipLabelModifiers: StatefulInteractiveElement + Sized {
    fn tooltip_label(self, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        self.capture_any_mouse_down(|event: &MouseDownEvent, _, cx| {
            cx.set_global(TooltipLabelLastPress(RiseClock::monotonic(), event.position))
        })
        .tooltip(move |window, cx| {
            let label = label.clone();
            let hovered_at = RiseClock::monotonic().saturating_sub(TOOLTIP_SHOW_DELAY);
            let pressed_in_place = TooltipLabel::pressed_in_place(window, cx);
            cx.new(|_| TooltipLabel { label, hovered_at, pressed_in_place }).into()
        })
    }
}

impl<E: StatefulInteractiveElement + Sized> TooltipLabelModifiers for E {}
