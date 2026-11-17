use gpui::{
    App, BoxShadow, Context, ElementId, Entity, FocusHandle, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, RenderOnce, SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Task, Toggled,
    Window, black, div, point, px, white,
};
use std::time::Duration;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_theme::ui_color_system::UIColor;

use crate::main_text::MainText;
use crate::swiftui::animated::{Animated, Lerp};
use crate::swiftui::binding::Binding;
use crate::swiftui::button_style::{focus_visible_ring, keyboard_focus_ring};
use crate::swiftui::motion::{MotionSpec, snappy};
use crate::swiftui::shapes::SwiftUiShapes;
use crate::swiftui::stacks::spacer;

pub const SWITCH_WIDTH: f32 = 51.0;
pub const SWITCH_HEIGHT: f32 = 31.0;
pub const SWITCH_KNOB_INSET: f32 = 2.0;

pub fn switch_toggle_motion() -> MotionSpec {
    snappy(0.22)
}

const SWITCH_SPARE_TICK: Duration = Duration::from_millis(33);

/// switch value is spoken in the system language so read it from the app locale
fn switch_state_label(is_on: bool) -> String {
    tr(if is_on { "accessibility_state_on" } else { "accessibility_state_off" })
}

struct SwitchTapStyle;

impl SwitchTapStyle {
    fn make_body(label: gpui::Div) -> gpui::Div {
        label
    }
}

/// toggles the value inside the motion, the knob follows via SwitchUiState on the next render
fn toggle(value: &Binding<bool>, window: &mut Window, cx: &mut App) {
    value.toggle(window, cx);
}

/// knob travel from 0 off to 1 on, animated toward the bound value
/// also the focus that takes space, its own cached view so a frame of travel renders only the track
pub struct SwitchUiState {
    progress: Animated<f32>,
    focus: FocusHandle,
    tint: Hsla,
    off_track: Hsla,
    drawn_at: Duration,
    rest_drawn: bool,
    spare_ticks: Task<()>,
}

impl SwitchUiState {
    fn new(is_on: bool, tint: Hsla, off_track: Hsla, focus: FocusHandle) -> Self {
        Self {
            progress: Animated::new(if is_on { 1.0 } else { 0.0 }),
            focus,
            tint,
            off_track,
            drawn_at: Duration::ZERO,
            rest_drawn: true,
            spare_ticks: Task::ready(()),
        }
    }

    fn start_spare_ticks(&mut self, cx: &mut Context<Self>) {
        self.rest_drawn = false;
        self.spare_ticks = cx.spawn(async move |state, cx| {
            loop {
                cx.background_executor().timer(SWITCH_SPARE_TICK).await;
                let keep = state.update(cx, |state, cx| {
                    let now = RiseClock::monotonic();
                    let running = state.progress.is_running(now);
                    if !running && state.rest_drawn {
                        return false;
                    }
                    if now.saturating_sub(state.drawn_at) >= SWITCH_SPARE_TICK {
                        cx.notify();
                    }
                    true
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        });
    }

    /// moves the knob toward is_on and takes the theme colors, renders again only on a change
    fn sync(&mut self, is_on: bool, tint: Hsla, off_track: Hsla, cx: &mut Context<Self>) {
        let target = if is_on { 1.0 } else { 0.0 };
        let mut changed = false;
        if *self.progress.target() != target {
            self.progress.set_in(target, switch_toggle_motion(), RiseClock::monotonic(), cx);
            self.start_spare_ticks(cx);
            changed = true;
        }
        if self.tint != tint || self.off_track != off_track {
            self.tint = tint;
            self.off_track = off_track;
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }
}

impl Render for SwitchUiState {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        self.drawn_at = now;
        if self.progress.is_running(now) {
            window.request_animation_frame();
        }
        let progress = self.progress.value(now).clamp(0.0, 1.0);
        switch_track(progress, self.tint, self.off_track)
    }
}

fn switch_state(id: &ElementId, is_on: bool, window: &mut Window, cx: &mut App) -> Entity<SwitchUiState> {
    let (tint, off_track) = switch_colors(cx);
    window.use_keyed_state(id.clone(), cx, move |_, cx| SwitchUiState::new(is_on, tint, off_track, cx.focus_handle().tab_stop(true)))
}

fn switch_colors(cx: &App) -> (Hsla, Hsla) {
    let theme = crate::theme(cx);
    (theme.primary._100, UIColor::secondary_system_fill().resolved_color(theme.appearance))
}

/// drawn switch at progress, 0 off and 1 on
/// takes no clicks
fn switch_track(progress: f32, tint: gpui::Hsla, off_track: gpui::Hsla) -> gpui::Div {
    let knob = SWITCH_HEIGHT - 2.0 * SWITCH_KNOB_INSET;
    let travel = SWITCH_WIDTH - knob - 2.0 * SWITCH_KNOB_INSET;
    let track = off_track.lerp(&tint, progress);
    div()
        .relative()
        .flex_none()
        .w(px(SWITCH_WIDTH))
        .h(px(SWITCH_HEIGHT))
        .capsule()
        .bg(track)
        .child(
            div()
                .absolute()
                .top(px(SWITCH_KNOB_INSET))
                .left(px(SWITCH_KNOB_INSET + travel * progress))
                .size(px(knob))
                .circle()
                .bg(white())
                .shadow(vec![BoxShadow {
                    color: black().opacity(0.15),
                    offset: point(px(0.0), px(3.0)),
                    blur_radius: px(8.0),
                    spread_radius: px(0.0),
                    inset: false,
                }]),
        )
}

/// draws the switch, the knob view keeps animating while it travels
fn render_switch(state: &Entity<SwitchUiState>, is_on: bool, _window: &mut Window, cx: &mut App) -> gpui::Div {
    let (tint, off_track) = switch_colors(cx);
    state.update(cx, |state, cx| state.sync(is_on, tint, off_track, cx));
    let now = RiseClock::monotonic();
    let resting = state.update(cx, |state, _| {
        state.drawn_at = now;
        let resting = !state.progress.is_running(now);
        if resting {
            state.rest_drawn = true;
        }
        resting.then(|| (state.progress.value(now).clamp(0.0, 1.0), state.tint, state.off_track))
    });
    if let Some((progress, tint, off_track)) = resting {
        return div().flex_none().w(px(SWITCH_WIDTH)).h(px(SWITCH_HEIGHT)).child(switch_track(progress, tint, off_track));
    }
    div().flex_none().w(px(SWITCH_WIDTH)).h(px(SWITCH_HEIGHT)).child(
        state
            .clone()
            .cached(StyleRefinement::default().w(px(SWITCH_WIDTH)).h(px(SWITCH_HEIGHT))),
    )
}

#[derive(IntoElement)]
pub struct SwitchUi {
    id: ElementId,
    value: Binding<bool>,
}

impl SwitchUi {
    pub fn new(id: impl Into<ElementId>, value: Binding<bool>) -> Self {
        Self { id: id.into(), value }
    }
}

impl RenderOnce for SwitchUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_on = self.value.get(cx);
        let state = switch_state(&self.id, is_on, window, cx);
        let focus = state.read(cx).focus.clone();
        let drawn = render_switch(&state, is_on, window, cx);
        let focus_ring =
            keyboard_focus_ring(&focus, crate::theme(cx).primary._100, px(SWITCH_HEIGHT / 2.0), window);
        let click = self.value.clone();
        let key = self.value;

        div()
            .id(self.id)
            .flex_none()
            .track_focus(&focus)
            .cursor_pointer()
            .role(gpui::Role::Switch)
            .aria_toggled(if is_on { Toggled::True } else { Toggled::False })
            .aria_value(SharedString::from(switch_state_label(is_on)))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                toggle(&click, window, cx);
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "space" && !event.is_held {
                    cx.stop_propagation();
                    toggle(&key, window, cx);
                }
            })
            .child(SwitchTapStyle::make_body(drawn))
            .children(focus_ring)
    }
}

/// settings row where a click anywhere flips the switch
/// the switch takes no clicks itself so one click is exactly one change
#[derive(IntoElement)]
pub struct SwitchRowUi {
    id: ElementId,
    title: SharedString,
    value: Binding<bool>,
}

impl SwitchRowUi {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>, value: Binding<bool>) -> Self {
        Self { id: id.into(), title: title.into(), value }
    }
}

impl RenderOnce for SwitchRowUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_on = self.value.get(cx);
        let state = switch_state(&self.id, is_on, window, cx);
        let focus = state.read(cx).focus.clone();
        let drawn = render_switch(&state, is_on, window, cx);
        let click = self.value.clone();
        let key = self.value;

        focus_visible_ring(div().id(self.id).track_focus(&focus))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .px(px(16.0))
            .py(px(9.0))
            .cursor_pointer()
            .role(gpui::Role::Switch)
            .aria_label(self.title.clone())
            .aria_toggled(if is_on { Toggled::True } else { Toggled::False })
            .aria_value(SharedString::from(switch_state_label(is_on)))
            .on_click(move |_, window, cx| toggle(&click, window, cx))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "space" && !event.is_held {
                    cx.stop_propagation();
                    toggle(&key, window, cx);
                }
            })
            .child(div().flex_shrink(1.0).min_w_0().child(MainText::new(self.title).px(15.0)))
            .child(spacer())
            .child(drawn.flex_none())
    }
}
