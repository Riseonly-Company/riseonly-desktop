use gpui::{
    Background, BorderStyle, Bounds, Corners, Edges, Hsla, IntoElement, Pixels, Styled, Window, canvas, point, px,
    quad, transparent_black,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::guide_overlay_ui::guide_overlay_theme::GuideOverlayTheme;
use crate::guide_overlay_ui::guide_spotlight_layer::GuideSpotlightCutout;
use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::motion::{CubicBezier, MotionSpec, duration_from_seconds};

/// primary colored edge of the hole plus the one ping a step gets when it finds its element
/// helps on light lists where the lit patch is near white anyway
#[derive(Clone, Debug)]
pub struct GuideAttentionLayer {
    cutout: Option<GuideSpotlightCutout>,
    drawn_cutout: Option<GuideSpotlightCutout>,
    color: Hsla,
    line_width: f32,
    opacity: Animated<f32>,
    echo_start: Option<MotionTime>,
    bounds: Bounds<Pixels>,
}

/// ring outline, the rect it strokes and its corner radius
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideRingPath {
    pub rect: Bounds<Pixels>,
    pub corner_radius: f32,
}

impl Default for GuideAttentionLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl GuideAttentionLayer {
    pub fn new() -> Self {
        Self {
            cutout: None,
            drawn_cutout: None,
            color: transparent_black(),
            line_width: GuideOverlayTheme::ATTENTION_LINE_WIDTH,
            opacity: Animated::new(0.0),
            echo_start: None,
            bounds: Bounds::default(),
        }
    }

    pub fn cutout(&self) -> Option<GuideSpotlightCutout> {
        self.cutout
    }

    pub fn refresh_bounds(&mut self, bounds: Bounds<Pixels>) {
        self.bounds = bounds;
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    /// set when a step is installed, never while its element moves
    pub fn set_color(&mut self, color: Hsla) {
        self.color = color;
    }

    /// puts the ring on the hole or removes it when there is no hole
    pub fn follow(&mut self, cutout: Option<GuideSpotlightCutout>, animated: bool) {
        let now = RiseClock::monotonic();
        let previous = self.cutout;
        self.cutout = cutout;

        let Some(cutout) = cutout else {
            self.echo_start = None;
            self.set_visible(false, animated, now);
            return;
        };

        if previous.is_some_and(|previous| previous != cutout) {
            self.echo_start = None;
        }
        self.drawn_cutout = Some(cutout);

        self.set_visible(true, animated, now);
    }

    pub fn ping(&mut self, animated: bool) {
        let now = RiseClock::monotonic();
        if !animated || self.cutout.is_none() || *self.opacity.target() <= 0.0 {
            return;
        }
        self.echo_start = Some(now);
    }

    /// true while the ring or its echo still needs frames at now
    pub fn is_animating(&self, now: MotionTime) -> bool {
        self.opacity.is_running(now) || self.echo_progress(now).is_some()
    }

    fn set_visible(&mut self, visible: bool, animated: bool, now: MotionTime) {
        let target = if visible { 1.0 } else { 0.0 };
        if *self.opacity.target() == target {
            return;
        }
        if !animated {
            self.opacity.jump(target);
            return;
        }
        let spec = MotionSpec::curve(
            GuideOverlayTheme::core_animation_timing(),
            duration_from_seconds(GuideOverlayTheme::SCRIM_FADE),
        );
        self.opacity.set(target, spec, now, false);
    }

    /// eased progress of the running echo at now, none once both echoes ended
    fn echo_progress(&self, now: MotionTime) -> Option<f32> {
        let start = self.echo_start?;
        let pulse = GuideOverlayTheme::ATTENTION_PULSE;
        if pulse <= 0.0 {
            return None;
        }
        let elapsed = now.saturating_sub(start).as_secs_f64();
        let cycles = f64::from(GuideOverlayTheme::ATTENTION_ECHO_COUNT);
        if elapsed >= pulse * cycles {
            return None;
        }
        let fraction = (elapsed % pulse) / pulse;
        Some(CubicBezier::EASE_OUT.progress(fraction) as f32)
    }

    /// outline of the hole, optionally grown by spread
    /// growing radius with the rect keeps the echo the same shape at any spread
    pub fn ring_path(cutout: GuideSpotlightCutout, spread: f32) -> GuideRingPath {
        let rect = cutout.resolved_rect().dilate(px(spread));
        let short = f32::from(rect.size.width.min(rect.size.height));
        let radius = GuideOverlayTheme::MINIMUM_CORNER_RADIUS.max((cutout.corner_radius() + spread).min(short * 0.5));
        GuideRingPath {
            rect,
            corner_radius: radius,
        }
    }

    /// draws ring and echo in host coordinates, asks for the next frame while either moves
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let now = RiseClock::monotonic();
        if self.is_animating(now) {
            window.request_animation_frame();
        }
        let Some(cutout) = self.drawn_cutout else {
            return;
        };
        let opacity = self.opacity.value(now);
        if opacity <= 0.0 {
            return;
        }
        let origin = bounds.origin;
        paint_stroked_ring(
            Self::ring_path(cutout, 0.0),
            self.line_width,
            self.color.opacity(opacity),
            origin,
            window,
        );
        if let Some(progress) = self.echo_progress(now) {
            let echo = Self::ring_path(cutout, GuideOverlayTheme::ATTENTION_ECHO_SPREAD * progress);
            paint_stroked_ring(echo, self.line_width, self.color.opacity(opacity * (1.0 - progress)), origin, window);
        }
    }
}

/// strokes the ring centered on its outline, half the line outside the rect and half inside
pub(crate) fn paint_stroked_ring(
    ring: GuideRingPath,
    line_width: f32,
    color: Hsla,
    origin: gpui::Point<Pixels>,
    window: &mut Window,
) {
    let half = line_width * 0.5;
    let rect = Bounds::new(
        point(ring.rect.origin.x + origin.x, ring.rect.origin.y + origin.y),
        ring.rect.size,
    )
    .dilate(px(half));
    window.paint_quad(quad(
        rect,
        Corners::all(px(ring.corner_radius + half)),
        Background::from(transparent_black()),
        Edges::all(px(line_width)),
        color,
        BorderStyle::Solid,
    ));
}

/// host view for the ring, never interactive
/// it registers no hitbox so clicks pass through the cutout to the app
#[derive(Clone, Debug, Default)]
pub struct GuideAttentionView {
    pub attention_layer: GuideAttentionLayer,
}

impl GuideAttentionView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attention_layer(&self) -> &GuideAttentionLayer {
        &self.attention_layer
    }

    pub fn attention_layer_mut(&mut self) -> &mut GuideAttentionLayer {
        &mut self.attention_layer
    }

    /// element that draws the ring over the whole overlay for this frame
    pub fn element(&self) -> impl IntoElement {
        let mut layer = self.attention_layer.clone();
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                layer.refresh_bounds(bounds);
                layer.paint(bounds, window);
            },
        )
        .absolute()
        .size_full()
    }
}
