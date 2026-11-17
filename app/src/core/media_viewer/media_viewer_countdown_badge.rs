use std::f32::consts::PI;

use gpui::{
    App, Bounds, FontWeight, IntoElement, ParentElement, Pixels, RenderOnce, Styled, Window, black, canvas, div,
    point, px, white,
};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::loader_ui::{arc_points, round_cap, stroke_polyline};
use rise_ui::main_text::MainText;
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;

/// a timed view limited file on screen, the window end
/// and the whole length so the ring shows the share left
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaViewerCountdown {
    pub presentation_id: Uuid,
    pub deadline: Date,
    pub total: TimeInterval,
}

/// time left of a timed file, a small ring in the top corner with seconds inside
/// stays when the bars hide because the file closes itself at zero
#[derive(IntoElement)]
pub struct MediaViewerCountdownBadge {
    pub countdown: MediaViewerCountdown,
    pub safe_top: f32,
}

impl MediaViewerCountdownBadge {
    pub const SIZE: f32 = 36.0;
    const STROKE: f32 = 2.5;

    pub fn new(countdown: MediaViewerCountdown, safe_top: f32) -> Self {
        Self { countdown, safe_top }
    }

    fn paint_ring(bounds: Bounds<Pixels>, fraction: f32, window: &mut Window) {
        let center = bounds.center();
        let radius = (f32::from(bounds.size.width) - Self::STROKE) / 2.0;
        let track = arc_points(center, radius, 0.0, 2.0 * PI);
        stroke_polyline(&track, Self::STROKE, true, white().opacity(0.25), window);
        if fraction <= 0.0 {
            return;
        }
        let start = -PI / 2.0;
        let sweep = 2.0 * PI * fraction;
        let progress = arc_points(center, radius, start, sweep);
        stroke_polyline(&progress, Self::STROKE, false, white(), window);
        let cap_at = |angle: f32| point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()));
        round_cap(cap_at(start), Self::STROKE, white(), window);
        round_cap(cap_at(start + sweep), Self::STROKE, white(), window);
    }
}

impl RenderOnce for MediaViewerCountdownBadge {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .pt(px(self.safe_top + 8.0 + PageHeaderMetrics::TOOLBAR_BUTTON_SIZE + 12.0))
            .pr(px(16.0))
            .child(MediaViewerCountdownRing::new(self.countdown.deadline, self.countdown.total))
    }
}

/// same ring for a tile, the peer countdown on a timed photo
/// redrawn every frame until the deadline
#[derive(IntoElement)]
pub struct MediaViewerCountdownRing {
    deadline: Date,
    total: TimeInterval,
}

impl MediaViewerCountdownRing {
    pub const SIDE: f32 = MediaViewerCountdownBadge::SIZE;

    /// runs the ring down to the deadline, total is the whole timer so the arc shows the share left
    pub fn new(deadline: Date, total: TimeInterval) -> Self {
        Self { deadline, total }
    }
}

impl RenderOnce for MediaViewerCountdownRing {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let left = f64::max(0.0, self.deadline.time_interval_since(&RiseClock::now()));
        let fraction = if self.total > 0.0 { f64::min(1.0, left / self.total) } else { 0.0 };
        if left > 0.0 {
            window.request_animation_frame();
        }
        let fraction = fraction as f32;
        div()
            .relative()
            .size(px(Self::SIDE))
            .flex()
            .items_center()
            .justify_center()
            .child(div().absolute().inset_0().rounded_full().bg(black().opacity(0.45)))
            .child(
                canvas(|_, _, _| {}, move |bounds, _, window, _| {
                    MediaViewerCountdownBadge::paint_ring(bounds, fraction, window)
                })
                .absolute()
                .inset_0(),
            )
            .child(
                MainText::new((left.ceil() as i64).to_string())
                    .px(13.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .monospaced_digits(true)
                    .color(white())
                    .line_limit(1),
            )
    }
}
