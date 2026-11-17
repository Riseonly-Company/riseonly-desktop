use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Point, RenderOnce, ScrollDelta, ScrollWheelEvent,
    SharedString, Size, StatefulInteractiveElement, Styled, Task, TouchPhase, Window, div, point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::host_os::HostOs;
use rise_platform::wheel_gesture::{
    LineBurst, LineBurstVerdict, WHEEL_SETTLE_DELAY, WheelGestureTracker, WheelSettleToken, WheelStep, line_burst_step,
    line_burst_turn_lines,
};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::motion::ease_out;

use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// vertical pager of the two riff screens,
/// mounts the page on screen and neighbours and keeps the shown post under the
/// viewport when the list changes
pub struct ShortsVerticalPagerController {
    offset: Animated<f32>,
    ids: Vec<String>,
    extra_pages: usize,
    page_height: f32,
    anchor_id: Option<String>,
    pub(crate) shown_page: usize,
    wheel: WheelGestureTracker,
    line_burst: Option<LineBurst>,
    settle_task: Option<Task<()>>,
    pending_settle: Option<(WheelSettleToken, Duration)>,
    settle_armed: bool,
    locked_until: Duration,
    turn_locked: bool,
    claims_swipe: Option<bool>,
    swipe_base: f32,
    wheel_turn: WheelTurn,
}

/// a pixel scroll gesture turns exactly one page, in the sign of its first travel
/// settling by travel at lift snapped short flicks back
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WheelTurn {
    Open,
    Spent,
    Pulling,
    Sideways,
}

impl ShortsVerticalPagerController {
    const DRAG_MINIMUM_DISTANCE: f32 = 10.0;
    const SETTLE_DURATION: f64 = 0.24;
    const PAGE_SWIPE_THRESHOLD: f32 = 0.2;
    const ALIGNMENT_TOLERANCE: f32 = 8.0;
    const RUBBER_BAND_COEFFICIENT: f32 = 0.55;
    const WHEEL_TURN_TRAVEL: f32 = 24.0;
    const WHEEL_END_TURN_TRAVEL: f32 = 8.0;

    pub fn new(anchor_id: Option<String>) -> Self {
        Self {
            offset: Animated::new(0.0),
            ids: Vec::new(),
            extra_pages: 0,
            page_height: 1.0,
            anchor_id,
            shown_page: 0,
            wheel: WheelGestureTracker::new(),
            line_burst: None,
            settle_task: None,
            pending_settle: None,
            settle_armed: false,
            locked_until: Duration::ZERO,
            turn_locked: false,
            claims_swipe: None,
            swipe_base: 0.0,
            wheel_turn: WheelTurn::Open,
        }
    }

    /// written by the frame that reads it so it doesnt notify
    pub fn update_pages(&mut self, posts: &[Rc<GetPostFeedResponse>], extra_pages: usize, page_height: f32) {
        self.page_height = page_height.max(1.0);
        self.extra_pages = extra_pages;
        if !posts.iter().map(|post| post.id.as_str()).eq(self.ids.iter().map(String::as_str)) {
            self.ids = posts.iter().map(|post| post.id.clone()).collect();
        }
        let found = self.anchor_id.as_ref().and_then(|anchor| self.ids.iter().position(|id| id == anchor));
        match found {
            Some(index) => {
                if index != self.shown_page {
                    let delta = index as f32 - self.shown_page as f32;
                    self.shown_page = index;
                    self.swipe_base += delta;
                    self.offset.jump(*self.offset.target() + delta);
                }
            }
            None => {
                let last = self.page_count().saturating_sub(1);
                if self.shown_page > last {
                    self.shown_page = last;
                    self.offset.jump(last as f32);
                }
                self.anchor_id = self.ids.get(self.shown_page).cloned();
            }
        }
    }

    /// scrolls to the page with no animation so it lands at once
    pub fn scroll_to(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(index) = self.ids.iter().position(|candidate| candidate == id) else {
            return;
        };
        self.shown_page = index;
        self.anchor_id = Some(id.to_owned());
        self.swipe_base = index as f32;
        self.offset.jump(index as f32);
        cx.notify();
    }

    pub fn scroll_to_animated(&mut self, id: &str, duration: f64, cx: &mut Context<Self>) {
        let Some(index) = self.ids.iter().position(|candidate| candidate == id) else {
            return;
        };
        self.shown_page = index;
        self.anchor_id = Some(id.to_owned());
        self.swipe_base = index as f32;
        self.offset.set_in(index as f32, ease_out(duration), RiseClock::monotonic(), cx);
        cx.notify();
    }

    /// page top against the viewport top in points, positive while the first page is pulled down
    pub fn page_top(&self, id: &str) -> Option<f32> {
        let index = self.ids.iter().position(|candidate| candidate == id)?;
        Some((index as f32 - self.offset.value(RiseClock::monotonic())) * self.page_height)
    }

    /// bounces past the first page, the pull refresh of the riffs feed reads it
    fn rubber_banded(&self, raw: f32, last: f32) -> f32 {
        if raw >= 0.0 {
            return raw.min(last);
        }
        -(1.0 - 1.0 / (-raw * Self::RUBBER_BAND_COEFFICIENT + 1.0))
    }

    pub fn is_moving(&self) -> bool {
        self.claims_swipe == Some(true) || self.wheel.is_tracking() || self.offset.is_running(RiseClock::monotonic())
    }

    /// post whose page is near the viewport top, the trailing loader page has none
    pub fn aligned_id(&self) -> Option<String> {
        let offset = self.offset.value(RiseClock::monotonic());
        let nearest = offset.round();
        if ((offset - nearest) * self.page_height).abs() >= Self::ALIGNMENT_TOLERANCE {
            return None;
        }
        self.ids.get(nearest.max(0.0) as usize).cloned()
    }

    fn page_count(&self) -> usize {
        self.ids.len() + self.extra_pages
    }

    fn mount_window(&self, offset: f32) -> Option<(usize, usize)> {
        let last = self.page_count().checked_sub(1)?;
        let low = (offset.floor() as i64 - 1).max(0) as usize;
        let high = ((offset.ceil() as i64 + 1).max(0) as usize).min(last);
        Some((low.min(high), high))
    }

    fn should_begin(&self, translation: Point<Pixels>) -> bool {
        self.page_count() > 0 && f32::from(translation.y).abs() > f32::from(translation.x).abs()
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        if let ScrollDelta::Lines(lines) = event.delta {
            let turn_lines = line_burst_turn_lines(HostOs::current());
            if self.handle_line_scroll(lines, turn_lines, now, cx) {
                cx.stop_propagation();
            }
            return;
        }
        let was_tracking = self.wheel.is_tracking();
        let feed = self.wheel.feed(event, window.line_height());
        let starts = event.touch_phase == TouchPhase::Started || (!was_tracking && self.wheel.is_tracking());
        if self.apply_wheel_step(feed.step, starts, now, cx) {
            cx.stop_propagation();
        }
        if let Some(token) = feed.settle {
            self.pending_settle = Some((token, now + WHEEL_SETTLE_DELAY));
            if !self.settle_armed {
                self.settle_armed = true;
                self.settle_task = Some(cx.spawn(async move |this, cx| {
                    let mut wait = WHEEL_SETTLE_DELAY;
                    loop {
                        cx.background_executor().timer(wait).await;
                        match this.update(cx, |this: &mut Self, cx| this.settle_when_quiet(cx)) {
                            Ok(Some(next)) => wait = next,
                            _ => break,
                        }
                    }
                }));
            }
        }
    }

    fn settle_when_quiet(&mut self, cx: &mut Context<Self>) -> Option<Duration> {
        let now = RiseClock::monotonic();
        let Some((token, due)) = self.pending_settle else {
            self.settle_armed = false;
            return None;
        };
        if now < due {
            return Some(due - now);
        }
        self.pending_settle = None;
        self.settle_armed = false;
        if let Some(step) = self.wheel.settle(token) {
            self.apply_wheel_step(step, false, now, cx);
        }
        None
    }

    /// one pixel scroll gesture turns one page,
    /// starts opens a new gesture,
    /// true when the event belongs to this pager
    /// a gesture opened while the page still settles turns nothing
    pub(crate) fn apply_wheel_step(
        &mut self,
        step: WheelStep,
        starts: bool,
        now: Duration,
        cx: &mut Context<Self>,
    ) -> bool {
        if starts {
            self.wheel_turn = WheelTurn::Open;
            self.turn_locked = now < self.locked_until;
        }
        match step {
            WheelStep::Ignored => matches!(self.wheel_turn, WheelTurn::Spent | WheelTurn::Pulling),
            WheelStep::Moved(translation) => self.wheel_moved(translation, false, now, cx),
            WheelStep::Ended(translation) => {
                let claimed = self.wheel_moved(translation, true, now, cx);
                if self.wheel_turn == WheelTurn::Pulling {
                    self.wheel_turn = WheelTurn::Spent;
                    self.settle(0.0, now, cx);
                }
                claimed
            }
            WheelStep::Cancelled => {
                let claimed = matches!(self.wheel_turn, WheelTurn::Spent | WheelTurn::Pulling);
                if self.wheel_turn == WheelTurn::Pulling {
                    self.settle(0.0, now, cx);
                }
                self.wheel_turn = WheelTurn::Open;
                claimed
            }
        }
    }

    fn wheel_moved(&mut self, translation: Point<Pixels>, ended: bool, now: Duration, cx: &mut Context<Self>) -> bool {
        let along = f32::from(translation.y);
        let across = f32::from(translation.x);
        let Some(last) = self.page_count().checked_sub(1) else {
            return false;
        };
        let last = last as f32;
        match self.wheel_turn {
            WheelTurn::Sideways => false,
            WheelTurn::Spent => true,
            WheelTurn::Pulling => {
                self.offset.jump(self.rubber_banded(-along / self.page_height, last));
                cx.notify();
                true
            }
            WheelTurn::Open => {
                let threshold = if ended { Self::WHEEL_END_TURN_TRAVEL } else { Self::WHEEL_TURN_TRAVEL };
                if along.abs().max(across.abs()) < threshold {
                    return along.abs() >= across.abs();
                }
                if across.abs() >= along.abs() {
                    self.wheel_turn = WheelTurn::Sideways;
                    return false;
                }
                if self.turn_locked {
                    self.wheel_turn = WheelTurn::Spent;
                    return true;
                }
                let forward = along < 0.0;
                let base = self.offset.target().round().clamp(0.0, last);
                if !forward && base <= 0.0 {
                    self.wheel_turn = WheelTurn::Pulling;
                    self.offset.jump(self.rubber_banded(-along / self.page_height, last));
                    cx.notify();
                    return true;
                }
                self.wheel_turn = WheelTurn::Spent;
                self.settle(base + if forward { 1.0 } else { -1.0 }, now, cx);
                true
            }
        }
    }

    /// arrow keys,
    /// page up and down and space turn one page,
    /// true when the page moved
    pub(crate) fn step_page(&mut self, step: f32, cx: &mut Context<Self>) -> bool {
        let Some(last) = self.page_count().checked_sub(1) else {
            return false;
        };
        let base = self.offset.target().round().clamp(0.0, last as f32);
        let target = (base + step).clamp(0.0, last as f32);
        if target == base {
            return false;
        }
        self.settle(target, RiseClock::monotonic(), cx);
        true
    }

    /// line deltas are far below the page threshold and have no end, so one burst is one page
    /// true when the event belongs to this pager
    pub(crate) fn handle_line_scroll(
        &mut self,
        lines: Point<f32>,
        turn_lines: f32,
        now: Duration,
        cx: &mut Context<Self>,
    ) -> bool {
        let (burst, verdict) = line_burst_step(self.line_burst, lines.y, lines.x, true, turn_lines, now);
        self.line_burst = burst;
        let step = match verdict {
            LineBurstVerdict::NotMine => return false,
            LineBurstVerdict::Claimed => return true,
            LineBurstVerdict::TurnBack => -1.0,
            LineBurstVerdict::TurnForward => 1.0,
        };
        if now < self.locked_until {
            return true;
        }
        self.settle(*self.offset.target() + step, now, cx);
        true
    }

    /// a mouse drag follows the pointer and settles by predicted travel, true when it belongs to this pager
    fn apply_step(&mut self, step: WheelStep, cx: &mut Context<Self>) -> bool {
        match step {
            WheelStep::Ignored => false,
            WheelStep::Moved(translation) => {
                if translation == Point::default() {
                    return false;
                }
                let claims = match self.claims_swipe {
                    Some(claims) => claims,
                    None => {
                        let claims = self.should_begin(translation);
                        self.claims_swipe = Some(claims);
                        self.swipe_base = *self.offset.target();
                        claims
                    }
                };
                if claims {
                    let last = self.page_count().saturating_sub(1) as f32;
                    let moved = f32::from(translation.y) / self.page_height;
                    self.offset.jump(self.rubber_banded(self.swipe_base - moved, last));
                    cx.notify();
                }
                claims
            }
            WheelStep::Ended(translation) => {
                let claimed = self.claims_swipe.take() == Some(true);
                if claimed {
                    let moved = -f32::from(translation.y) / self.page_height;
                    let target = if moved > Self::PAGE_SWIPE_THRESHOLD {
                        self.swipe_base + 1.0
                    } else if moved < -Self::PAGE_SWIPE_THRESHOLD {
                        self.swipe_base - 1.0
                    } else {
                        self.swipe_base
                    };
                    self.settle(target, RiseClock::monotonic(), cx);
                }
                claimed
            }
            WheelStep::Cancelled => {
                let claimed = self.claims_swipe.take() == Some(true);
                if claimed {
                    self.offset.set_in(self.swipe_base, ease_out(Self::SETTLE_DURATION), RiseClock::monotonic(), cx);
                    cx.notify();
                }
                claimed
            }
        }
    }

    fn settle(&mut self, target: f32, now: Duration, cx: &mut Context<Self>) {
        let Some(last) = self.page_count().checked_sub(1) else {
            return;
        };
        let target = target.round().clamp(0.0, last as f32);
        if target != *self.offset.target() {
            self.locked_until = now + Duration::from_secs_f64(Self::SETTLE_DURATION);
        }
        self.offset.set_in(target, ease_out(Self::SETTLE_DURATION), now, cx);
        self.shown_page = target as usize;
        self.anchor_id = self.ids.get(self.shown_page).cloned();
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct ShortsVerticalPager {
    id: ElementId,
    controller: Entity<ShortsVerticalPagerController>,
    size: Size<Pixels>,
    page_content: Box<dyn Fn(usize, &mut Window, &mut App) -> AnyElement>,
    takes_keys: bool,
}

impl ShortsVerticalPager {
    pub fn new(
        id: impl Into<ElementId>,
        controller: Entity<ShortsVerticalPagerController>,
        size: Size<Pixels>,
        page_content: impl Fn(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self { id: id.into(), controller, size, page_content: Box::new(page_content), takes_keys: true }
    }

    /// true when the pager is on screen and arrow keys work, a hidden kept tab passes false
    pub fn takes_keys(mut self, takes_keys: bool) -> Self {
        self.takes_keys = takes_keys;
        self
    }
}

impl RenderOnce for ShortsVerticalPager {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (offset, mounted) = {
            let controller = self.controller.read(cx);
            let offset = controller.offset.frame_value(RiseClock::monotonic(), window);
            (offset, controller.mount_window(offset))
        };
        let height = f32::from(self.size.height);
        let mut pager = div().relative().w(self.size.width).h(self.size.height).overflow_hidden();
        if let Some((low, high)) = mounted {
            for index in low..=high {
                pager = pager.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px((index as f32 - offset) * height))
                        .w(self.size.width)
                        .h(self.size.height)
                        .child((self.page_content)(index, window, cx)),
                );
            }
        }

        let focus_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("focus"));
        let focus = window.use_keyed_state::<FocusHandle>(focus_id, cx, |_, cx| cx.focus_handle()).read(cx).clone();
        let took_keys_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("took-keys"));
        let took_keys = window.use_keyed_state(took_keys_id, cx, |_, _| false);
        let takes_keys = self.takes_keys;
        if *took_keys.read(cx) != takes_keys {
            took_keys.update(cx, |took, _| *took = takes_keys);
            if takes_keys && ScreenFocus::top_sheet(window, cx).is_none() {
                ScreenFocus::claim(&focus, window, cx);
            }
        }

        let key_controller = self.controller.clone();
        let press_focus = focus.clone();
        let hover_focus = focus.clone();
        let wheel_controller = self.controller.clone();
        let changed_controller = self.controller.clone();
        let ended_controller = self.controller;
        pager
            .id(self.id.clone())
            .track_focus(&focus)
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                if takes_keys && !press_focus.contains_focused(window, cx) {
                    ScreenFocus::claim(&press_focus, window, cx);
                }
            })
            .on_hover(move |hovered, window, cx| {
                if *hovered && takes_keys && ScreenFocus::is_idle(window, cx) {
                    ScreenFocus::claim(&hover_focus, window, cx);
                }
            })
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let modifiers = &event.keystroke.modifiers;
                if !takes_keys || event.is_held || modifiers.control || modifiers.alt || modifiers.platform {
                    return;
                }
                let step = match (event.keystroke.key.as_str(), modifiers.shift) {
                    ("down" | "pagedown" | "space", false) => 1.0,
                    ("up" | "pageup", false) | ("space", true) => -1.0,
                    _ => return,
                };
                if key_controller.update(cx, |controller, cx| controller.step_page(step, cx)) {
                    cx.stop_propagation();
                }
            })
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                wheel_controller.update(cx, |controller, cx| controller.handle_scroll_wheel(event, window, cx));
            })
            .gesture(
                ElementId::NamedChild(Arc::new(self.id), SharedString::new_static("drag")),
                DragGesture::new(ShortsVerticalPagerController::DRAG_MINIMUM_DISTANCE)
                    .on_changed(move |value: &DragGestureValue, _, cx| {
                        changed_controller.update(cx, |controller, cx| {
                            controller.apply_step(
                                WheelStep::Moved(point(value.translation.width, value.translation.height)),
                                cx,
                            );
                        });
                    })
                    .on_ended(move |value: &DragGestureValue, _, cx| {
                        ended_controller.update(cx, |controller, cx| {
                            controller.apply_step(
                                WheelStep::Ended(point(
                                    value.predicted_end_translation.width,
                                    value.predicted_end_translation.height,
                                )),
                                cx,
                            );
                        });
                    }),
            )
    }
}
