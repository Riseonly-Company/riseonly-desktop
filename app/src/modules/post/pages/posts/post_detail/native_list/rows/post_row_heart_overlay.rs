use std::time::Duration;

use gpui::{
    BoxShadow, Context, Hsla, IntoElement, ParentElement, Pixels, Point, Render, Styled, Task, Window, black, div,
    point, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::shapes::pass_through;

use crate::modules::post::pages::posts::post_detail::native_list::rows::post_native_row_view::PostRowChromeSpring;

const HIDDEN_SCALE: f32 = 0.4;
const HEART_POINT_SIZE: f32 = 84.0;
const IN_RESPONSE: f64 = 0.28;
const IN_DAMPING_FRACTION: f64 = 0.55;
const HOLD_SECONDS: f64 = 0.55;
const OUT_SECONDS: f64 = 0.22;
const SHADOW_DISC_RATIO: f32 = 0.72;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostRowHeartOverlaySurface {
    Media,
    Copy,
}

pub struct PostRowHeartOverlay {
    surface: Option<PostRowHeartOverlaySurface>,
    generation: u64,
    is_shown: bool,
    is_hidden: bool,
    tint: Hsla,
    center: Point<Pixels>,
    scale: Animated<f32>,
    opacity: Animated<f32>,
    pending: Option<Task<()>>,
}

impl PostRowHeartOverlay {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            surface: None,
            generation: 0,
            is_shown: false,
            is_hidden: true,
            tint: gpui::transparent_black(),
            center: point(px(0.0), px(0.0)),
            scale: Animated::new(HIDDEN_SCALE),
            opacity: Animated::new(0.0),
            pending: None,
        }
    }

    pub fn surface(&self) -> Option<PostRowHeartOverlaySurface> {
        self.surface
    }

    /// rect is in the superview coordinate space
    pub fn place(&mut self, centered_in: gpui::Bounds<Pixels>, cx: &mut Context<Self>) {
        let center = centered_in.center();
        if self.center != center {
            self.center = center;
            cx.notify();
        }
    }

    pub fn show(&mut self, surface: PostRowHeartOverlaySurface, tint: Hsla, cx: &mut Context<Self>) {
        self.tint = tint;
        self.generation = self.generation.wrapping_add(1);
        let token = self.generation;
        let restarts = !self.is_shown || self.surface != Some(surface);
        self.surface = Some(surface);
        self.is_hidden = false;
        if restarts {
            self.is_shown = true;
            PostRowChromeSpring::reset(&mut self.scale, HIDDEN_SCALE);
            PostRowChromeSpring::reset(&mut self.opacity, 0.0);
            PostRowChromeSpring::animate(&mut self.scale, 1.0, IN_RESPONSE, IN_DAMPING_FRACTION, cx);
            PostRowChromeSpring::animate(&mut self.opacity, 1.0, IN_RESPONSE, IN_DAMPING_FRACTION, cx);
        }
        self.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(HOLD_SECONDS)).await;
            let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                if this.generation == token {
                    this.hide(token, cx);
                }
            });
        }));
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.is_shown = false;
        self.surface = None;
        PostRowChromeSpring::reset(&mut self.scale, HIDDEN_SCALE);
        PostRowChromeSpring::reset(&mut self.opacity, 0.0);
        self.is_hidden = true;
        self.pending = None;
        cx.notify();
    }

    fn hide(&mut self, token: u64, cx: &mut Context<Self>) {
        self.is_shown = false;
        PostRowChromeSpring::animate_ease_out(&mut self.scale, HIDDEN_SCALE, OUT_SECONDS, cx);
        PostRowChromeSpring::animate_ease_out(&mut self.opacity, 0.0, OUT_SECONDS, cx);
        if let Some(task) = self.pending.take() {
            task.detach();
        }
        self.pending = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(OUT_SECONDS)).await;
            let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                if this.generation != token {
                    return;
                }
                this.surface = None;
                this.is_hidden = true;
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for PostRowHeartOverlay {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_hidden {
            return div().into_any_element();
        }
        let now = RiseClock::monotonic();
        let scale = self.scale.frame_value(now, window);
        let opacity = self.opacity.frame_value(now, window);
        let side = (HEART_POINT_SIZE * scale).max(0.0);
        let disc = side * SHADOW_DISC_RATIO;
        pass_through()
            .absolute()
            .left(self.center.x - px(side / 2.0))
            .top(self.center.y - px(side / 2.0))
            .size(px(side))
            .opacity(opacity.clamp(0.0, 1.0))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .left(px((side - disc) / 2.0))
                    .top(px((side - disc) / 2.0))
                    .size(px(disc))
                    .rounded_full()
                    .shadow(vec![BoxShadow {
                        color: black().opacity(0.35),
                        offset: point(px(0.0), px(6.0)),
                        blur_radius: px(14.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }]),
            )
            .children(IconUi::sized("heart.fill", px(side), self.tint, true))
            .into_any_element()
    }
}
