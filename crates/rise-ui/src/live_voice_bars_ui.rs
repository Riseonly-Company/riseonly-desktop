use std::time::Duration;

use gpui::{
    App, Bounds, ElementId, Hsla, IntoElement, Pixels, RenderOnce, Styled, Window, canvas, fill, point, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::frame_request::request_frame_if_on_screen;
use crate::swiftui::motion::CubicBezier;

pub const BREATH_PERIOD: Duration = Duration::from_millis(1900);

pub const SCALE_VALUES: [f32; 5] = [0.38, 1.0, 0.55, 0.9, 0.38];
pub const KEY_TIMES: [f32; 5] = [0.0, 0.28, 0.52, 0.76, 1.0];

pub const PHASES: [f64; 3] = [0.0, 0.42, 0.84];

/// vertical scale of bar at elapsed after the animations were installed
/// before its begin time a bar shows its model value unscaled
pub fn bar_scale(bar: usize, elapsed: Duration) -> f32 {
    let begin = PHASES[bar % PHASES.len()];
    let local = elapsed.as_secs_f64() - begin;
    if local < 0.0 {
        return 1.0;
    }
    let period = BREATH_PERIOD.as_secs_f64();
    let fraction = ((local % period) / period) as f32;
    keyframe_value(fraction)
}

/// keyframe value with ease in ease out on every segment
pub fn keyframe_value(fraction: f32) -> f32 {
    let fraction = fraction.clamp(0.0, 1.0);
    for segment in 0..SCALE_VALUES.len() - 1 {
        let (start, end) = (KEY_TIMES[segment], KEY_TIMES[segment + 1]);
        if fraction <= end {
            let local = if end > start { (fraction - start) / (end - start) } else { 1.0 };
            let eased = CubicBezier::EASE_IN_OUT.progress(f64::from(local)) as f32;
            let (from, to) = (SCALE_VALUES[segment], SCALE_VALUES[segment + 1]);
            return from + (to - from) * eased;
        }
    }
    SCALE_VALUES[SCALE_VALUES.len() - 1]
}

/// frame of each bar in a badge of bounds, before the vertical scale
pub fn bar_frames(bounds: Bounds<Pixels>) -> [Bounds<Pixels>; 3] {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let bar_width = (width / 5.0).max(1.0);
    let spacing = bar_width;
    let total_width = bar_width * 3.0 + spacing * 2.0;
    let origin_x = (width - total_width) * 0.5;
    std::array::from_fn(|index| {
        let bar_height = height;
        Bounds::new(
            point(
                bounds.origin.x + px(origin_x + index as f32 * (bar_width + spacing)),
                bounds.origin.y + px((height - bar_height) * 0.5),
            ),
            size(px(bar_width), px(bar_height)),
        )
    })
}

struct LiveVoiceBarsInstall {
    base_time: Duration,
}

#[derive(IntoElement)]
pub struct LiveVoiceBarsUi {
    id: ElementId,
    pub color: Hsla,
    pub width: f32,
    pub height: f32,
}

impl LiveVoiceBarsUi {
    /// new loader, default size 11 by 10
    pub fn new(id: impl Into<ElementId>, color: Hsla) -> Self {
        Self { id: id.into(), color, width: 11.0, height: 10.0 }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
}

impl RenderOnce for LiveVoiceBarsUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let install = window.use_keyed_state(self.id, cx, |_, _| LiveVoiceBarsInstall { base_time: RiseClock::monotonic() });
        let elapsed = RiseClock::monotonic().saturating_sub(install.read(cx).base_time);
        let animates = self.height > 0.0;
        let color = self.color;
        canvas(
            move |bounds, window, _| {
                if animates {
                    request_frame_if_on_screen(bounds, window);
                }
            },
            move |bounds, (), window, _| {
                for (index, frame) in bar_frames(bounds).into_iter().enumerate() {
                    let scale = bar_scale(index, elapsed);
                    let height = f32::from(frame.size.height) * scale;
                    let width = f32::from(frame.size.width);
                    let scaled = Bounds::new(
                        point(frame.origin.x, frame.center().y - px(height / 2.0)),
                        size(frame.size.width, px(height)),
                    );
                    let radius = (width * 0.5).min(height * 0.5).max(0.0);
                    window.paint_quad(fill(scaled, color).corner_radii(px(radius)));
                }
            },
        )
        .w(px(self.width))
        .h(px(self.height))
        .flex_none()
    }
}
