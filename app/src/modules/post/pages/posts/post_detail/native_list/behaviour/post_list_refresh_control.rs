use std::f32::consts::{PI, TAU};
use std::rc::Rc;
use std::time::Duration;

use futures::future::LocalBoxFuture;
use gpui::{
    AppContext, AsyncApp, Bounds, Context, Entity, Hsla, Pixels, Render, Task, canvas, fill, point, px, size,
};
use gpui::{App, PathBuilder, Window};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_ui::swiftui::prelude::*;

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListScrollMetrics;

const RING_LINE_WIDTH: f32 = 2.5;
const RING_INSET: f32 = 3.0;
const RING_SEGMENTS: f32 = 64.0;

pub type PostListRefreshControlOnRefresh = Rc<dyn Fn(AsyncApp) -> LocalBoxFuture<'static, ()>>;
pub type PostListRefreshControlOnTopInsetChange = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct PostListRefreshControl {
    pub indicator_view: Entity<PostListRefreshIndicatorView>,
    pub on_refresh: Option<PostListRefreshControlOnRefresh>,
    pub on_top_inset_change: Option<PostListRefreshControlOnTopInsetChange>,
    pub on_refresh_end_at_top: Option<PostListRefreshControlOnTopInsetChange>,
    base_top_inset: f32,
    is_enabled: bool,
    is_refreshing: bool,
    is_armed: bool,
    began_at_top: bool,
    wheel_push: f32,
    wheel_push_at: Option<Duration>,
    offset_from_top: f32,
    last_width: f32,
    generation: u64,
    task: Option<Task<()>>,
    is_stopped: bool,
    feedback: ImpactFeedbackGenerator,
}

impl PostListRefreshControl {
    pub const TRIGGER_DISTANCE: f32 = 72.0;
    pub const WHEEL_PUSH_WINDOW: TimeInterval = 0.35;
    pub const HELD_INSET: f32 = 52.0;
    pub const INDICATOR_SIZE: f32 = 28.0;
    pub const MINIMUM_VISIBLE_DURATION: TimeInterval = 0.45;

    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            indicator_view: cx.new(|_| PostListRefreshIndicatorView::new()),
            on_refresh: None,
            on_top_inset_change: None,
            on_refresh_end_at_top: None,
            base_top_inset: 0.0,
            is_enabled: true,
            is_refreshing: false,
            is_armed: false,
            began_at_top: false,
            wheel_push: 0.0,
            wheel_push_at: None,
            offset_from_top: 0.0,
            last_width: 0.0,
            generation: 0,
            task: None,
            is_stopped: false,
            feedback: ImpactFeedbackGenerator::new(ImpactStyle::Light),
        }
    }

    /// lists top inset without the held part
    /// the floating header plus padding
    pub fn base_top_inset(&self) -> f32 {
        self.base_top_inset
    }

    pub fn set_base_top_inset(&mut self, base_top_inset: f32, cx: &mut Context<Self>) {
        if self.base_top_inset == base_top_inset {
            return;
        }
        self.base_top_inset = base_top_inset;
        self.layout_indicator(self.last_width, cx);
    }

    pub fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    pub fn set_is_enabled(&mut self, is_enabled: bool, cx: &mut Context<Self>) {
        let old_value = self.is_enabled;
        self.is_enabled = is_enabled;
        if old_value == is_enabled || is_enabled {
            return;
        }
        self.is_armed = false;
        if !self.is_refreshing {
            self.indicator_view.update(cx, |indicator, cx| indicator.set_alpha(0.0, cx));
        }
    }

    pub fn begin_refresh_from_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_enabled || self.is_stopped {
            return;
        }
        self.begin_refresh(window, cx);
    }

    pub fn is_refreshing(&self) -> bool {
        self.is_refreshing
    }

    /// inset held above the first row while a refresh runs
    pub fn held_inset(&self) -> f32 {
        if self.is_refreshing { Self::HELD_INSET } else { 0.0 }
    }

    /// every metrics change
    /// follows the pull and starts a refresh on a release past the threshold
    pub fn observe(&mut self, metrics: &RiseManualListScrollMetrics, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        self.offset_from_top = (metrics.content_offset_y - metrics.minimum_content_offset_y).as_f32();
        if self.is_refreshing {
            tracing::warn!(
                target: "riseonly::X1993",
                "observe refreshing offset_from_top={} offset={} min={} tracking={}",
                self.offset_from_top,
                metrics.content_offset_y.as_f32(),
                metrics.minimum_content_offset_y.as_f32(),
                metrics.is_tracking
            );
        }
        if self.is_enabled && !self.is_refreshing && self.on_refresh.is_some() {
            let progress = ((-self.offset_from_top).max(0.0) / Self::TRIGGER_DISTANCE).min(1.0);
            if metrics.is_tracking {
                if progress >= 1.0 && !self.is_armed {
                    self.is_armed = true;
                    let _ = self.feedback.impact_occurred();
                } else if progress < 1.0 && self.is_armed {
                    self.is_armed = false;
                }
            } else if self.is_armed {
                self.is_armed = false;
                self.begin_refresh(window, cx);
            }
            if !self.is_refreshing {
                self.indicator_view.update(cx, |indicator, cx| {
                    indicator.set_progress(progress, cx);
                    indicator.set_alpha(progress, cx);
                });
            }
        }
        self.layout_indicator(self.last_width, cx);
    }

    pub fn layout_indicator(&mut self, width: f32, cx: &mut Context<Self>) {
        if width <= 1.0 {
            return;
        }
        self.last_width = width;
        let side = Self::INDICATOR_SIZE;
        let center_y = if self.is_refreshing {
            let center_y = self.base_top_inset + Self::HELD_INSET / 2.0 - self.offset_from_top;
            let top = center_y - side / 2.0;
            let alpha = ((top - self.base_top_inset + side) / side).clamp(0.0, 1.0);
            self.indicator_view.update(cx, |indicator, cx| indicator.set_alpha(alpha, cx));
            center_y
        } else {
            self.base_top_inset + (-self.offset_from_top).max(0.0) / 2.0
        };
        let frame = Bounds::new(
            point(px(((width - side) / 2.0).round()), px(center_y - side / 2.0)),
            size(px(side), px(side)),
        );
        if self.is_refreshing {
            tracing::warn!(
                target: "riseonly::X1993",
                "indicator refreshing center_y={} offset_from_top={} base={} width={}",
                center_y,
                self.offset_from_top,
                self.base_top_inset,
                width
            );
        }
        self.indicator_view.update(cx, |indicator, cx| indicator.set_frame(frame, cx));
    }

    pub fn wheel_push_past_top(&mut self, push: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_stopped || !self.is_enabled || self.is_refreshing || self.on_refresh.is_none() {
            self.wheel_push = 0.0;
            return;
        }
        let now = RiseClock::monotonic();
        let is_fresh = self
            .wheel_push_at
            .is_some_and(|at| now.saturating_sub(at) <= Duration::from_secs_f64(Self::WHEEL_PUSH_WINDOW));
        self.wheel_push = if is_fresh { self.wheel_push + push } else { push };
        self.wheel_push_at = Some(now);
        if self.wheel_push < Self::TRIGGER_DISTANCE {
            return;
        }
        self.wheel_push = 0.0;
        self.wheel_push_at = None;
        let _ = self.feedback.impact_occurred();
        self.begin_refresh(window, cx);
    }

    /// a key or the button past the pull
    /// same refresh for a mouse
    pub fn request_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_stopped || !self.is_enabled {
            return;
        }
        self.begin_refresh(window, cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        self.is_stopped = true;
        self.generation = self.generation.wrapping_add(1);
        self.task = None;
        self.is_refreshing = false;
        self.on_refresh = None;
        self.on_top_inset_change = None;
        self.on_refresh_end_at_top = None;
        self.indicator_view.update(cx, |indicator, cx| indicator.stop_spinning(cx));
    }

    fn begin_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(on_refresh) = self.on_refresh.clone() else { return };
        if self.is_refreshing {
            return;
        }
        self.is_refreshing = true;
        self.began_at_top = self.offset_from_top <= 1.0;
        tracing::warn!(
            target: "riseonly::X1993",
            "begin refresh offset_from_top={} base={} width={}",
            self.offset_from_top,
            self.base_top_inset,
            self.last_width
        );
        self.generation = self.generation.wrapping_add(1);
        let current = self.generation;
        self.indicator_view.update(cx, |indicator, cx| {
            indicator.set_alpha(1.0, cx);
            indicator.start_spinning(cx);
        });
        if let Some(on_top_inset_change) = self.on_top_inset_change.clone() {
            window.defer(cx, move |window, cx| on_top_inset_change(window, cx));
        }
        self.layout_indicator(self.last_width, cx);
        let started_at = RiseClock::monotonic();
        let minimum = Duration::from_secs_f64(Self::MINIMUM_VISIBLE_DURATION);
        let window_handle = window.window_handle();
        self.task = Some(cx.spawn(async move |this, cx| {
            on_refresh(cx.clone()).await;
            let remaining = minimum.saturating_sub(RiseClock::monotonic().saturating_sub(started_at));
            if !remaining.is_zero() {
                cx.background_executor().timer(remaining).await;
            }
            let (on_top_inset_change, on_refresh_end_at_top) =
                this.update(cx, |this, cx| this.end_refresh(current, cx)).unwrap_or((None, None));
            let _ = window_handle.update(cx, |_, window, cx| {
                if let Some(on_top_inset_change) = on_top_inset_change {
                    on_top_inset_change(window, cx);
                }
                if let Some(on_refresh_end_at_top) = on_refresh_end_at_top {
                    window.defer(cx, move |window, cx| on_refresh_end_at_top(window, cx));
                }
            });
        }));
    }

    fn end_refresh(
        &mut self,
        current: u64,
        cx: &mut Context<Self>,
    ) -> (Option<PostListRefreshControlOnTopInsetChange>, Option<PostListRefreshControlOnTopInsetChange>) {
        if self.is_stopped || !self.is_refreshing || self.generation != current {
            return (None, None);
        }
        self.is_refreshing = false;
        self.task = None;
        tracing::warn!(
            target: "riseonly::X1993",
            "end refresh offset_from_top={} began_at_top={}",
            self.offset_from_top,
            self.began_at_top
        );
        self.indicator_view.update(cx, |indicator, cx| {
            indicator.stop_spinning(cx);
            indicator.set_alpha(0.0, cx);
        });
        self.layout_indicator(self.last_width, cx);
        let on_refresh_end_at_top = if self.began_at_top { self.on_refresh_end_at_top.clone() } else { None };
        self.began_at_top = false;
        (self.on_top_inset_change.clone(), on_refresh_end_at_top)
    }
}

/// ring that fills with the pull then a spinner while the refresh runs
pub struct PostListRefreshIndicatorView {
    frame: Bounds<Pixels>,
    alpha: f32,
    progress: f32,
    is_spinning: bool,
    tint: Option<Hsla>,
}

impl Default for PostListRefreshIndicatorView {
    fn default() -> Self {
        Self::new()
    }
}

impl PostListRefreshIndicatorView {
    pub fn new() -> Self {
        Self { frame: Bounds::default(), alpha: 0.0, progress: 0.0, is_spinning: false, tint: None }
    }

    pub fn set_tint(&mut self, color: Hsla, cx: &mut Context<Self>) {
        if self.tint != Some(color) {
            self.tint = Some(color);
            cx.notify();
        }
    }

    pub fn set_progress(&mut self, progress: f32, cx: &mut Context<Self>) {
        let progress = progress.clamp(0.0, 1.0);
        if self.progress != progress {
            self.progress = progress;
            cx.notify();
        }
    }

    pub fn start_spinning(&mut self, cx: &mut Context<Self>) {
        self.is_spinning = true;
        cx.notify();
    }

    pub fn stop_spinning(&mut self, cx: &mut Context<Self>) {
        self.is_spinning = false;
        self.progress = 0.0;
        cx.notify();
    }

    pub fn set_alpha(&mut self, alpha: f32, cx: &mut Context<Self>) {
        if self.alpha != alpha {
            self.alpha = alpha;
            cx.notify();
        }
    }

    pub fn set_frame(&mut self, frame: Bounds<Pixels>, cx: &mut Context<Self>) {
        if self.frame != frame {
            self.frame = frame;
            cx.notify();
        }
    }

    fn paint_ring(bounds: Bounds<Pixels>, progress: f32, tint: Hsla, window: &mut Window) {
        if progress <= 0.0 {
            return;
        }
        let center = bounds.center();
        let radius = (bounds.size.width.min(bounds.size.height).as_f32() / 2.0 - RING_INSET).max(0.0);
        let at = |angle: f32| {
            let angle = angle - PI / 2.0;
            point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
        };
        let sweep = progress * TAU;
        let steps = (progress * RING_SEGMENTS).ceil().max(1.0);
        let mut builder = PathBuilder::stroke(px(RING_LINE_WIDTH));
        builder.move_to(at(0.0));
        let mut step = 1.0;
        while step <= steps {
            builder.line_to(at(sweep * step / steps));
            step += 1.0;
        }
        if let Ok(path) = builder.build() {
            window.paint_path(path, tint);
        }
        let cap = size(px(RING_LINE_WIDTH), px(RING_LINE_WIDTH));
        for end in [at(0.0), at(sweep)] {
            window.paint_quad(fill(Bounds::centered_at(end, cap), tint).corner_radii(px(RING_LINE_WIDTH / 2.0)));
        }
    }
}

impl Render for PostListRefreshIndicatorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tint = self.tint.unwrap_or_else(|| rise_ui::theme(cx).text.secondary);
        let progress = self.progress;
        let root = div()
            .absolute()
            .left(self.frame.origin.x)
            .top(self.frame.origin.y)
            .w(self.frame.size.width)
            .h(self.frame.size.height)
            .opacity(self.alpha);
        if self.alpha <= 0.0 {
            return root.into_any_element();
        }
        if self.is_spinning {
            return root
                .flex()
                .items_center()
                .justify_center()
                .child(progress_view().tint(tint))
                .into_any_element();
        }
        root.child(
            canvas(|_, _, _| {}, move |bounds, _, window, _| Self::paint_ring(bounds, progress, tint, window)).size_full(),
        )
        .into_any_element()
    }
}
