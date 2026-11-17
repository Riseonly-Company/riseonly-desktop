use gpui::{Bounds, FillOptions, PathBuilder, PathStyle, Pixels, Point, Window, point, px, size};
use rise_canon::RiseGuideSpotlightShape;
use rise_core::foundation::foundation_date::RiseClock;

use crate::guide_overlay_ui::guide_overlay_theme::GuideOverlayTheme;
use crate::swiftui::animated::Animated;
use crate::swiftui::motion::{MotionSpec, duration_from_seconds};

/// the hole in the scrim, anchor rect in window coordinates plus the shape canon asked for
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideSpotlightCutout {
    pub anchor: Bounds<Pixels>,
    pub shape: RiseGuideSpotlightShape,
}

impl GuideSpotlightCutout {
    const CIRCLE_ASPECT_LIMIT: f32 = 1.5;

    const ROUND_SHORT_SIDE_LIMIT: f32 = 120.0;

    pub fn new(anchor: Bounds<Pixels>, shape: RiseGuideSpotlightShape) -> Self {
        Self { anchor, shape }
    }

    /// shape actually drawn, not always the requested one
    /// circle comes from the anchor short side, one too flat becomes a capsule
    /// a large area gets a rounded rect
    pub fn effective_shape(&self) -> RiseGuideSpotlightShape {
        if self.shape == RiseGuideSpotlightShape::RoundedRect {
            return self.shape;
        }
        let width = self.anchor.size.width.as_f32();
        let height = self.anchor.size.height.as_f32();
        let short = width.min(height);
        let long = width.max(height);
        if short <= 0.0 {
            return self.shape;
        }
        if short > Self::ROUND_SHORT_SIDE_LIMIT {
            return RiseGuideSpotlightShape::RoundedRect;
        }
        if self.shape != RiseGuideSpotlightShape::Circle {
            return self.shape;
        }
        if long / short <= Self::CIRCLE_ASPECT_LIMIT {
            return RiseGuideSpotlightShape::Circle;
        }
        RiseGuideSpotlightShape::Capsule
    }

    /// rect the hole takes, padded away from the element
    /// every shape must contain the element so circles and capsules clipping tall rows are avoided
    pub fn resolved_rect(&self) -> Bounds<Pixels> {
        let padding = GuideOverlayTheme::CUTOUT_PADDING;
        let x = self.anchor.origin.x.as_f32();
        let y = self.anchor.origin.y.as_f32();
        let width = self.anchor.size.width.as_f32();
        let height = self.anchor.size.height.as_f32();
        let padded = rect(x - padding, y - padding, width + padding * 2.0, height + padding * 2.0);
        match self.effective_shape() {
            RiseGuideSpotlightShape::RoundedRect => padded,
            RiseGuideSpotlightShape::Capsule => {
                let half = width.min(height) / 2.0;
                let radius = half + padding;
                let reach = (radius * radius - half * half).sqrt();
                let extra = (radius - padding - reach).ceil().max(0.0);
                if height > width {
                    rect(
                        x - padding,
                        y - padding - extra,
                        width + padding * 2.0,
                        height + (padding + extra) * 2.0,
                    )
                } else {
                    rect(
                        x - padding - extra,
                        y - padding,
                        width + (padding + extra) * 2.0,
                        height + padding * 2.0,
                    )
                }
            }
            RiseGuideSpotlightShape::Circle => {
                let diameter = width.max(height) + padding * 2.0;
                let mid_x = x + width / 2.0;
                let mid_y = y + height / 2.0;
                rect(mid_x - diameter / 2.0, mid_y - diameter / 2.0, diameter, diameter)
            }
        }
    }

    pub fn corner_radius(&self) -> f32 {
        GuideOverlayTheme::cutout_corner_radius(self.effective_shape(), self.resolved_rect())
    }

    /// exact containment for all three shapes since all are rounded rects
    /// called on every press so it stays plain arithmetic
    pub fn contains(&self, point: Point<Pixels>) -> bool {
        let rect = self.resolved_rect();
        let (min_x, min_y) = (rect.origin.x.as_f32(), rect.origin.y.as_f32());
        let (width, height) = (rect.size.width.as_f32(), rect.size.height.as_f32());
        let (max_x, max_y) = (min_x + width, min_y + height);
        let (px_, py_) = (point.x.as_f32(), point.y.as_f32());
        if !(px_ >= min_x && px_ < max_x && py_ >= min_y && py_ < max_y) {
            return false;
        }
        let radius = self.corner_radius().min(width.min(height) / 2.0);
        if radius <= 0.0 {
            return true;
        }
        let (inner_min_x, inner_max_x) = (min_x + radius, max_x - radius);
        let (inner_min_y, inner_max_y) = (min_y + radius, max_y - radius);
        if px_ >= inner_min_x && px_ <= inner_max_x {
            return true;
        }
        if py_ >= inner_min_y && py_ <= inner_max_y {
            return true;
        }
        let corner_x = if px_ < inner_min_x { inner_min_x } else { inner_max_x };
        let corner_y = if py_ < inner_min_y { inner_min_y } else { inner_max_y };
        (px_ - corner_x).hypot(py_ - corner_y) <= radius
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

/// adds a rounded rect as a closed subpath, quarter circles are cubic arcs with the usual 0.5523 handle
pub fn append_rounded_rect(builder: &mut PathBuilder, rect: Bounds<Pixels>, corner_radius: f32) {
    const KAPPA: f32 = 0.552_284_8;
    let (min_x, min_y) = (rect.origin.x.as_f32(), rect.origin.y.as_f32());
    let (width, height) = (rect.size.width.as_f32(), rect.size.height.as_f32());
    let (max_x, max_y) = (min_x + width, min_y + height);
    let radius = corner_radius.min(width.min(height) / 2.0).max(0.0);
    let handle = radius * (1.0 - KAPPA);
    let at = |x: f32, y: f32| point(px(x), px(y));

    builder.move_to(at(min_x + radius, min_y));
    builder.line_to(at(max_x - radius, min_y));
    builder.cubic_bezier_to(at(max_x, min_y + radius), at(max_x - handle, min_y), at(max_x, min_y + handle));
    builder.line_to(at(max_x, max_y - radius));
    builder.cubic_bezier_to(at(max_x - radius, max_y), at(max_x, max_y - handle), at(max_x - handle, max_y));
    builder.line_to(at(min_x + radius, max_y));
    builder.cubic_bezier_to(at(min_x, max_y - radius), at(min_x + handle, max_y), at(min_x, max_y - handle));
    builder.line_to(at(min_x, min_y + radius));
    builder.cubic_bezier_to(at(min_x + radius, min_y), at(min_x, min_y + handle), at(min_x + handle, min_y));
    builder.close();
}

pub struct GuideSpotlightLayer {
    cutout: Option<GuideSpotlightCutout>,
    bounds: Bounds<Pixels>,
    hole: Option<(Animated<Bounds<Pixels>>, Animated<f32>)>,
    opacity: f32,
    presented_opacity: Animated<f32>,
}

impl Default for GuideSpotlightLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl GuideSpotlightLayer {
    pub fn new() -> Self {
        Self {
            cutout: None,
            bounds: Bounds::default(),
            hole: None,
            opacity: 0.0,
            presented_opacity: Animated::new(0.0),
        }
    }

    pub fn cutout(&self) -> Option<GuideSpotlightCutout> {
        self.cutout
    }

    fn motion(duration: f64) -> MotionSpec {
        MotionSpec::curve(GuideOverlayTheme::core_animation_timing(), duration_from_seconds(duration))
    }

    /// installs a cutout, a move between live cutouts animates over the spotlight duration
    /// appear and disappear use the opacity fade
    pub fn apply(&mut self, cutout: Option<GuideSpotlightCutout>, animated: bool) {
        let previous = self.cutout;
        self.cutout = cutout;

        let Some(next) = cutout else {
            self.hole = None;
            return;
        };
        let target_rect = next.resolved_rect();
        let target_radius = next.corner_radius();

        let travels = animated
            && self.has_area()
            && previous.is_some()
            && previous != cutout
            && self.opacity > 0.0
            && self.hole.is_some();
        if travels && let Some((rect_motion, radius_motion)) = self.hole.as_mut() {
            let now = RiseClock::monotonic();
            let spec = Self::motion(GuideOverlayTheme::SPOTLIGHT_MOVE);
            rect_motion.set(target_rect, spec, now, false);
            radius_motion.set(target_radius, spec, now, false);
            return;
        }
        self.hole = Some((Animated::new(target_rect), Animated::new(target_radius)));
    }

    pub fn set_visible(&mut self, visible: bool, animated: bool) {
        let target = if visible { 1.0 } else { 0.0 };
        if self.opacity == target {
            return;
        }
        self.opacity = target;
        if !animated {
            self.presented_opacity.jump(target);
            return;
        }
        let spec = Self::motion(GuideOverlayTheme::SCRIM_FADE);
        self.presented_opacity.set(target, spec, RiseClock::monotonic(), false);
    }

    pub fn refresh_path(&mut self, bounds: Bounds<Pixels>) {
        self.bounds = bounds;
        if let Some(cutout) = self.cutout {
            self.hole = Some((Animated::new(cutout.resolved_rect()), Animated::new(cutout.corner_radius())));
        }
    }

    fn has_area(&self) -> bool {
        self.bounds.size.width > px(0.0) && self.bounds.size.height > px(0.0)
    }

    /// paints the scrim for this frame, asks for the next one only while something moves
    pub fn paint(&self, window: &mut Window) {
        if !self.has_area() {
            return;
        }
        let now = RiseClock::monotonic();
        let opacity = self.presented_opacity.frame_value(now, window);
        if opacity <= 0.0 {
            return;
        }

        let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::even_odd()));
        append_rounded_rect(&mut builder, self.bounds, 0.0);
        if let Some((rect_motion, radius_motion)) = &self.hole {
            let hole = rect_motion.frame_value(now, window);
            let radius = radius_motion.frame_value(now, window);
            if hole.size.width > px(0.0) && hole.size.height > px(0.0) {
                append_rounded_rect(&mut builder, hole, radius);
            }
        }
        let Ok(path) = builder.build() else {
            return;
        };
        let mut color = GuideOverlayTheme::scrim_color();
        color.a *= opacity;
        window.paint_path(path, color);
    }
}

/// host view for the spotlight layer, never interactive and claims no press of its own
#[derive(Default)]
pub struct GuideSpotlightView {
    pub spotlight_layer: GuideSpotlightLayer,
}

impl GuideSpotlightView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn layout_subviews(&mut self, bounds: Bounds<Pixels>) {
        if self.spotlight_layer.bounds != bounds {
            self.spotlight_layer.refresh_path(bounds);
        }
    }
}
