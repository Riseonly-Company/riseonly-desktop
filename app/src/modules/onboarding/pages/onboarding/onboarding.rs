use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, DispatchPhase, ElementId, FocusHandle, FontWeight, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, RenderOnce,
    ScrollDelta, ScrollWheelEvent, SharedString, Styled, Task, TouchPhase, Window, canvas,
    container_query, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::host_os::HostOs;
use rise_platform::wheel_gesture::{
    LineBurst, LineBurstVerdict, WHEEL_SETTLE_DELAY, line_burst_step, line_burst_turn_lines,
};
use rise_theme::ThemeStore;
use rise_ui::bg_wrapper_ui::BgWrapperUi;
use rise_ui::button_ui::ButtonUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::pressable_ui::PressableUi;
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{ease_in_out, spring};
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::modules::onboarding::pages::onboarding::onboarding_progress::OnboardingProgress;
use crate::modules::onboarding::pages::onboarding::slides::onboarding_slide1::OnboardingSlide1;
use crate::modules::onboarding::pages::onboarding::slides::onboarding_slide2::OnboardingSlide2;
use crate::modules::onboarding::pages::onboarding::slides::onboarding_slide3::OnboardingSlide3;
use crate::modules::onboarding::pages::onboarding::slides::onboarding_slide4::OnboardingSlide4;

const SLIDES_DATA: [(u8, fn() -> AnyElement); 4] = [
    (1, || OnboardingSlide1.into_any_element()),
    (2, || OnboardingSlide2.into_any_element()),
    (3, || OnboardingSlide3.into_any_element()),
    (4, || OnboardingSlide4.into_any_element()),
];

const PAGE_TURN_FRACTION: f32 = 0.25;

pub(crate) const GESTURE_PAUSE: Duration = WHEEL_SETTLE_DELAY;

/// one two finger trackpad gesture over the pager, start to end
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct OnboardingSwipe {
    translation_x: f32,
    translation_y: f32,
    horizontal: Option<bool>,
    ends_on_pause: bool,
}

/// whether a Moved pixel event with no live swipe starts one
/// momentum after a macos swipe follows closely and is ignored
/// a wayland gesture with no Started phase opens after a quiet spell
pub(crate) fn phaseless_move_starts_swipe(inside: bool, last_pixels_at: Option<Duration>, now: Duration) -> bool {
    inside && last_pixels_at.is_none_or(|last| now.saturating_sub(last) >= GESTURE_PAUSE)
}

pub struct Onboarding {
    on_go_to_sign_up: Rc<dyn Fn(&mut Window, &mut App)>,
    current_slide: usize,
    page_position: Animated<f32>,
    swipe: Option<OnboardingSwipe>,
    line_swipe: Option<LineBurst>,
    last_pixels_at: Option<Duration>,
    swipe_pause_timer: Option<Task<()>>,
    page_width: Rc<Cell<f32>>,
    focus_handle: FocusHandle,
}

impl Onboarding {
    pub fn new(
        on_go_to_sign_up: impl Fn(&mut Window, &mut App) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            on_go_to_sign_up: Rc::new(on_go_to_sign_up),
            current_slide: 0,
            page_position: Animated::new(0.0),
            swipe: None,
            line_swipe: None,
            last_pixels_at: None,
            swipe_pause_timer: None,
            page_width: Rc::new(Cell::new(0.0)),
            focus_handle,
        }
    }

    pub fn current_slide(&self) -> usize {
        self.current_slide
    }

    pub fn is_last_slide(&self) -> bool {
        self.current_slide == SLIDES_DATA.len() - 1
    }

    /// selection binding of the pager
    /// swipe or arrow turns the page without recording progress
    fn select_slide(&mut self, index: usize, cx: &mut Context<Self>) {
        let index = index.min(SLIDES_DATA.len() - 1);
        self.current_slide = index;
        self.page_position
            .set_in(index as f32, ease_in_out(0.35), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn handle_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_slide < SLIDES_DATA.len() - 1 {
            self.select_slide(self.current_slide + 1, cx);
            OnboardingProgress::record(self.current_slide);
        } else {
            self.handle_get_started(window, cx);
        }
    }

    fn handle_skip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        OnboardingProgress::record(self.current_slide);
        self.go_to_sign_up(window, cx);
    }

    fn handle_get_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        OnboardingProgress::record(SLIDES_DATA.len() - 1);
        self.go_to_sign_up(window, cx);
    }

    fn go_to_sign_up(&self, window: &mut Window, cx: &mut Context<Self>) {
        let action = self.on_go_to_sign_up.clone();
        window.defer(cx, move |window, cx| action(window, cx));
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "left" => {
                if self.current_slide > 0 {
                    self.select_slide(self.current_slide - 1, cx);
                }
            }
            "right" => {
                if !self.is_last_slide() {
                    self.select_slide(self.current_slide + 1, cx);
                }
            }
            "enter" if self.focus_handle.is_focused(window) => self.handle_next(window, cx),
            "escape" => {
                if !self.is_last_slide() {
                    self.handle_skip(window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn handle_swipe(&mut self, event: &ScrollWheelEvent, inside: bool, cx: &mut Context<Self>) -> bool {
        let now = RiseClock::monotonic();
        let delta = match event.delta {
            ScrollDelta::Lines(lines) => {
                if let Some(swipe) = self.swipe {
                    let recent = self
                        .last_pixels_at
                        .is_some_and(|last| now.saturating_sub(last) < GESTURE_PAUSE);
                    if swipe.horizontal == Some(true) || recent {
                        return false;
                    }
                    self.swipe = None;
                }
                let turn_lines = line_burst_turn_lines(HostOs::current());
                let (line_swipe, verdict) =
                    line_burst_step(self.line_swipe, lines.x, lines.y, inside, turn_lines, now);
                self.line_swipe = line_swipe;
                match verdict {
                    LineBurstVerdict::NotMine => return false,
                    LineBurstVerdict::Claimed => {}
                    LineBurstVerdict::TurnBack => {
                        if self.current_slide > 0 {
                            self.select_slide(self.current_slide - 1, cx);
                        }
                    }
                    LineBurstVerdict::TurnForward => {
                        if !self.is_last_slide() {
                            self.select_slide(self.current_slide + 1, cx);
                        }
                    }
                }
                return true;
            }
            ScrollDelta::Pixels(delta) => delta,
        };
        let (delta_x, delta_y) = (f32::from(delta.x), f32::from(delta.y));
        let last_pixels_at = self.last_pixels_at.replace(now);

        match event.touch_phase {
            TouchPhase::Started => {
                if !inside {
                    return false;
                }
                self.swipe = Some(OnboardingSwipe {
                    translation_x: delta_x,
                    translation_y: delta_y,
                    horizontal: None,
                    ends_on_pause: false,
                });
                false
            }
            TouchPhase::Moved => {
                if self.swipe.is_none() {
                    if !phaseless_move_starts_swipe(inside, last_pixels_at, now) {
                        return false;
                    }
                    self.swipe = Some(OnboardingSwipe {
                        ends_on_pause: true,
                        ..OnboardingSwipe::default()
                    });
                }
                let Some(swipe) = self.swipe.as_mut() else {
                    return false;
                };
                let ends_on_pause = swipe.ends_on_pause;
                swipe.translation_x += delta_x;
                swipe.translation_y += delta_y;
                if swipe.horizontal.is_none() && (swipe.translation_x != 0.0 || swipe.translation_y != 0.0) {
                    swipe.horizontal = Some(swipe.translation_x.abs() > swipe.translation_y.abs());
                }
                let claimed = swipe.horizontal == Some(true);
                if ends_on_pause {
                    self.arm_swipe_pause(cx);
                }
                if claimed {
                    cx.notify();
                }
                claimed
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.finish_swipe(event.touch_phase == TouchPhase::Ended, cx)
            }
        }
    }

    /// settles the live swipe
    /// turns the page if it went far enough and ended normally, else springs back
    /// answers whether it was a page gesture
    fn finish_swipe(&mut self, ended: bool, cx: &mut Context<Self>) -> bool {
        let Some(swipe) = self.swipe.take() else {
            return false;
        };
        if swipe.horizontal != Some(true) {
            return false;
        }
        let on_screen = self.position_on_screen(Some(swipe), RiseClock::monotonic());
        let width = self.page_width.get();
        let turned = ended && width > 0.0 && swipe.translation_x.abs() > width * PAGE_TURN_FRACTION;
        let target = match (turned, swipe.translation_x < 0.0) {
            (true, true) if !self.is_last_slide() => self.current_slide + 1,
            (true, false) if self.current_slide > 0 => self.current_slide - 1,
            _ => self.current_slide,
        };
        self.page_position.jump(on_screen);
        self.select_slide(target, cx);
        true
    }

    fn arm_swipe_pause(&mut self, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(GESTURE_PAUSE);
        self.swipe_pause_timer = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |view, cx| {
                if view.swipe.is_some_and(|swipe| swipe.ends_on_pause) {
                    view.finish_swipe(true, cx);
                }
            })
            .ok();
        }));
    }

    /// strip position the pager draws, the animated selection moved by the live swipe
    fn position_on_screen(&self, swipe: Option<OnboardingSwipe>, now: std::time::Duration) -> f32 {
        let width = self.page_width.get();
        let drag = match swipe {
            Some(swipe) if swipe.horizontal == Some(true) && width > 0.0 => swipe.translation_x / width,
            _ => 0.0,
        };
        (self.page_position.value(now) - drag).clamp(0.0, (SLIDES_DATA.len() - 1) as f32)
    }

    fn skip_row(&self, is_last_slide: bool, text_secondary: Hsla, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        div()
            .w_full()
            .h(px(44.0))
            .mt(px(8.0))
            .flex()
            .flex_row()
            .items_center()
            .child(div().flex_1())
            .child(
                div().opacity(if is_last_slide { 0.0 } else { 1.0 }).child(
                    PressableUi::new("onboarding.skip")
                        .disabled(is_last_slide)
                        .on_press(move |window, cx| {
                            view.update(cx, |view, cx| view.handle_skip(window, cx)).ok();
                        })
                        .child(
                            div().px(px(20.0)).py(px(12.0)).child(
                                MainText::new(tr("skip"))
                                    .px(16.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(text_secondary),
                            ),
                        ),
                ),
            )
    }

    fn pager(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        if self.page_position.is_running(now) {
            window.request_animation_frame();
        }
        let position = self.position_on_screen(self.swipe, now);
        let page_width = self.page_width.clone();
        let view = cx.entity().downgrade();

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .child(
                canvas(
                    |bounds, _, _| bounds,
                    move |_, bounds, window, _| {
                        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, cx| {
                            if phase != DispatchPhase::Capture {
                                return;
                            }
                            let inside = bounds.contains(&event.position);
                            let claimed = view
                                .update(cx, |view, cx| view.handle_swipe(event, inside, cx))
                                .unwrap_or(false);
                            if claimed {
                                cx.stop_propagation();
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(container_query(move |size, _, _| {
                let width = f32::from(size.width);
                page_width.set(width);
                div()
                    .relative()
                    .size_full()
                    .children(SLIDES_DATA.iter().enumerate().map(|(index, (_, slide))| {
                        div()
                            .absolute()
                            .top_0()
                            .h_full()
                            .w(px(width))
                            .left(px((index as f32 - position) * width))
                            .child(slide())
                    }))
            }))
    }
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text_secondary = {
            let store = ThemeStore::shared(cx);
            store.read(cx).current_theme().text.secondary
        };
        let is_last_slide = self.is_last_slide();
        let view = cx.entity().downgrade();

        BgWrapperUi::new().child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .track_focus(&self.focus_handle)
                .on_key_down(cx.listener(Self::handle_key_down))
                .child(self.skip_row(is_last_slide, text_secondary, cx))
                .child(self.pager(window, cx))
                .child(
                    div()
                        .w_full()
                        .py(px(20.0))
                        .child(OnboardingPagination::new(SLIDES_DATA.len(), self.current_slide)),
                )
                .child(
                    div().w_full().flex().justify_center().pb(px(40.0)).child(
                        div().w_full().max_w(px(500.0)).px(px(22.0)).child(
                            ButtonUi::new("onboarding.next")
                                .height(45.0)
                                .on_press(move |window, cx| {
                                    view.update(cx, |view, cx| view.handle_next(window, cx)).ok();
                                })
                                .child(
                                    MainText::new(if is_last_slide { tr("get_started") } else { tr("next") })
                                        .px(18.0)
                                        .font_weight(FontWeight::SEMIBOLD),
                                ),
                        ),
                    ),
                ),
        )
    }
}

#[derive(IntoElement)]
struct OnboardingPagination {
    count: usize,
    current_index: usize,
}

impl OnboardingPagination {
    fn new(count: usize, current_index: usize) -> Self {
        Self { count, current_index }
    }
}

struct OnboardingPaginationState {
    current_index: usize,
    activity: Vec<Animated<f32>>,
}

impl RenderOnce for OnboardingPagination {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let primary = {
            let store = ThemeStore::shared(cx);
            store.read(cx).current_theme().primary._100
        };
        let Self { count, current_index } = self;
        let state = window.use_keyed_state("onboarding.pagination", cx, move |_, _| OnboardingPaginationState {
            current_index,
            activity: (0..count)
                .map(|index| Animated::new(if index == current_index { 1.0 } else { 0.0 }))
                .collect(),
        });
        let now = RiseClock::monotonic();
        let reduce_motion = cx.reduce_motion();
        state.update(cx, |pagination, _| {
            if pagination.current_index == current_index {
                return;
            }
            pagination.current_index = current_index;
            for (index, activity) in pagination.activity.iter_mut().enumerate() {
                let target = if index == current_index { 1.0 } else { 0.0 };
                activity.set(target, spring(0.35, 0.7), now, reduce_motion);
            }
        });
        let activity: Vec<f32> = state
            .read(cx)
            .activity
            .iter()
            .map(|activity| activity.frame_value(now, window))
            .collect();

        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(8.0))
            .children(activity.into_iter().map(move |active| {
                div()
                    .h(px(8.0))
                    .w(px((8.0 + 16.0 * active).max(0.0)))
                    .rounded_full()
                    .bg(primary)
                    .opacity((0.3 + 0.7 * active).clamp(0.0, 1.0))
            }))
    }
}

/// a slide with illustration over title and description
/// texts are measured first
/// on short windows the illustration shrinks and the slide scrolls
#[derive(IntoElement)]
pub struct OnboardingSlideLayout {
    id: &'static str,
    pub title: SharedString,
    pub description: SharedString,
    pub max_illustration_size: f32,
    illustration: Box<dyn Fn(f32) -> AnyElement>,
}

impl OnboardingSlideLayout {
    /// id keys the measured text height
    pub fn new<I: IntoElement>(
        id: &'static str,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        max_illustration_size: f32,
        illustration: impl Fn(f32) -> I + 'static,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            description: description.into(),
            max_illustration_size,
            illustration: Box::new(move |size| illustration(size).into_any_element()),
        }
    }
}

/// height of the illustration frame for a slide of this height whose texts take texts_height
pub fn onboarding_illustration_height(height: f32, texts_height: f32) -> f32 {
    340.0_f32.min(height * 0.5).min(height - texts_height).max(0.0)
}

/// side of the illustration inside its frame
pub fn onboarding_illustration_size(max_illustration_size: f32, illustration_height: f32, width: f32) -> f32 {
    max_illustration_size.min((illustration_height - 40.0).max(0.0).min((width - 40.0).max(0.0)))
}

impl RenderOnce for OnboardingSlideLayout {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let texts_height = window.use_keyed_state(self.id, cx, |_, _| 0.0_f32);
        let measured_texts_height = *texts_height.read(cx);
        let Self { id, title, description, max_illustration_size, illustration } = self;

        container_query(move |size, _, _| {
            let height = f32::from(size.height);
            let width = f32::from(size.width);
            let illustration_height = onboarding_illustration_height(height, measured_texts_height);
            let illustration_size = onboarding_illustration_size(max_illustration_size, illustration_height, width);

            scroll_view(ElementId::Name(format!("{id}.scroll").into()))
                .shows_indicators(false)
                .child(
                    div()
                        .w_full()
                        .min_h(px(height))
                        .px(px(20.0))
                        .flex()
                        .flex_col()
                        .justify_center()
                        .child(
                            div()
                                .w_full()
                                .h(px(illustration_height))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(illustration(illustration_size)),
                        )
                        .child(
                            div()
                                .relative()
                                .w_full()
                                .flex()
                                .flex_col()
                                .child(
                                    MainText::new(title)
                                        .px(32.0)
                                        .tac(TextAlignT::Center)
                                        .font_weight(FontWeight::BOLD)
                                        .margin_top(40.0),
                                )
                                .child(
                                    SecondaryText::new(description)
                                        .px(16.0)
                                        .tac(TextAlignT::Center)
                                        .line_height(24.0)
                                        .mt(16.0),
                                )
                                .child(measure(
                                    ElementId::Name(format!("{id}.texts").into()),
                                    move |bounds, _, cx| {
                                        let measured = f32::from(bounds.size.height);
                                        texts_height.update(cx, |height, cx| {
                                            if *height != measured {
                                                *height = measured;
                                                cx.notify();
                                            }
                                        });
                                    },
                                )),
                        ),
                )
        })
    }
}
